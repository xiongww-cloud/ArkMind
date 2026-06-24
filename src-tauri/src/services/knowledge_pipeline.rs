//! KnowledgePipeline — 3 independent polling loops
//!
//! Schedule:
//!   sync_loop(10s)  → materialize source_data → knowledge_chunks → FTS5
//!   embed_loop(60s) → vectorize new chunks → LanceDB
//!   tag_loop(10min) → AI tagging → tag_mappings

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;


static PIPELINE_RUNNING: AtomicBool = AtomicBool::new(false);

// ─── Entry point ──────────────────────────────────────────────────

pub fn start() {
    if PIPELINE_RUNNING.swap(true, Ordering::AcqRel) {
        println!("[知识管线] 已在运行中");
        return;
    }
    println!("[知识管线] Arkmind 知识管线启动 (v11.0)");

    // ── Step 1: Cleanup stale queue entries (zombie source_ids) ──
    crate::services::ai_sync::purge_stale_queue_entries();

    // ── Step 2: Cleanup stale SQL triggers from old schema versions ──
    let _ = crate::database::run(
        "DROP TRIGGER IF EXISTS ai_sync_products_insert;
         DROP TRIGGER IF EXISTS ai_sync_products_update;",
        &[],
    );

    // ── Step 3: Seed sync queue ──
    let queue_count: i64 = crate::database::with_db(|c| {
        c.query_row("SELECT COUNT(*) FROM ai_sync_queue", [], |r| r.get(0))
    }).unwrap_or(0);
    if queue_count == 0 {
        crate::services::ai_sync::cold_start_seed_sync_queue();
    } else {
        crate::services::ai_sync::seed_from_source_data();
    }

    // ── Step 4: Reset failed tasks with retry_count < 3 ──
    reset_retryable_failures();

    // ── Step 5: Heal ──
    heal_duplicate_tasks();

    // ── Step 6: Vector cache consistency check ──
    clean_vector_cache();

    // ── Step 7: Trash auto-cleanup ──
    auto_clean_trash();

    // ── Step 8: Persist WAL ──
    let _ = crate::database::save();

    // ── Spawn 3 independent loops ──
    std::thread::spawn(|| {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .build().expect("[知识管线] tokio 运行时创建失败");
        rt.block_on(async {
            tokio::join!(sync_loop(), embed_loop(), tag_loop());
        });
    });
}

pub fn is_running() -> bool {
    PIPELINE_RUNNING.load(Ordering::Acquire)
}

// ─── Startup helpers (unchanged) ──────────────────────────────────

fn reset_retryable_failures() {
    match crate::database::run(
        "UPDATE ai_sync_queue SET is_embedded=0, retry_count=0, last_error=''
         WHERE is_embedded=-1 AND retry_count<3", &[],
    ) { Ok(r) if r.changes > 0 => println!("[知识管线] 重置 {} 个可重试的同步失败", r.changes), _ => {} }
    match crate::database::run(
        "UPDATE ai_tag_queue SET status=0, retry_count=0
         WHERE status=-1 AND retry_count<3", &[],
    ) { Ok(r) if r.changes > 0 => println!("[知识管线] 重置 {} 个可重试的标签失败", r.changes), _ => {} }
}

fn heal_duplicate_tasks() {}

fn clean_vector_cache() {
    match crate::database::run(
        "DELETE FROM vector_cache WHERE chunk_id NOT IN (SELECT id FROM knowledge_chunks)", &[],
    ) { Ok(r) if r.changes > 0 => println!("[知识管线] 清理 {} 个过期向量缓存", r.changes), _ => {} }
    let vc_cnt: i64 = crate::database::with_db(|c|
        c.query_row("SELECT COUNT(*) FROM vector_cache", [], |r| r.get(0))).unwrap_or(0);
    let child_cnt: i64 = crate::database::with_db(|c|
        c.query_row("SELECT COUNT(*) FROM knowledge_chunks WHERE chunk_type='child'", [], |r| r.get(0))).unwrap_or(0);
    if vc_cnt > child_cnt { let _ = crate::database::run("DELETE FROM vector_cache", &[]); }
}

fn auto_clean_trash() {
    match crate::database::run(
        "DELETE FROM notes WHERE deleted_at IS NOT NULL AND datetime(deleted_at,'+30 days')<datetime('now', 'localtime')", &[],
    ) { Ok(r) if r.changes > 0 => println!("[知识管线] 回收站清理: {} 条笔记已永久删除", r.changes), _ => {} }
}

// ─── 3 independent polling loops ──────────────────────────────────

/// 同步循环：每 10s 扫描 ai_sync_queue，创建/更新 knowledge_chunks + FTS5
async fn sync_loop() {
    loop {
        if !PIPELINE_RUNNING.load(Ordering::Acquire) { break; }
        // 补种
        crate::services::ai_sync::seed_from_source_data();
        crate::services::ai_sync::seed_new_records();
        // 处理同步任务（每 tick 最多 20 个）
        for _ in 0..20 {
            if !run_timeout("sync", || crate::services::ai_sync::process_sync_task()).await { break; }
            tokio::task::yield_now().await;
        }
        tokio::time::sleep(Duration::from_secs(10)).await;
    }
}

/// 向量化循环：每 60s 调一次 embedding API，每批最多 20 个
async fn embed_loop() {
    loop {
        if !PIPELINE_RUNNING.load(Ordering::Acquire) { break; }
        run_timeout("vectorize", || async {
            match crate::services::hybrid_knowledge::vectorize_new_chunks(20).await {
                Ok(n) => { if n > 0 { println!("[知识管线] 已向量化: {n} 个块"); } n > 0 }
                Err(e) => { eprintln!("[知识管线] 向量化错误: {e}"); false }
            }
        }).await;
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}

/// 标签循环：每 10 分钟处理一次 ai_tag_queue
async fn tag_loop() {
    loop {
        if !PIPELINE_RUNNING.load(Ordering::Acquire) { break; }
        // 一次处理多个标签任务
        for _ in 0..20 {
            if !crate::services::ai_tag::process_tag_task().await { break; }
        }
        tokio::time::sleep(Duration::from_secs(600)).await; // 10min
    }
}

// ─── Helpers ──────────────────────────────────────────────────────

async fn run_timeout<F, Fut>(name: &str, f: F) -> bool
where F: FnOnce() -> Fut, Fut: std::future::Future<Output = bool>,
{
    match tokio::time::timeout(Duration::from_secs(30), f()).await {
        Ok(result) => result,
        Err(_) => { log::warn!("[知识管线] {} 超时 (>30秒)", name); false }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_pipeline_not_running_by_default() { assert!(!is_running()); }
}
