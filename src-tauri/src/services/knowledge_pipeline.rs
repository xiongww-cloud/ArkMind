//! KnowledgePipeline — 3 independent polling loops
//!
//! Schedule:
//!   sync_loop(10s)  → 通过 get_changed_since 接口获取变更 → materialize → knowledge_chunks → FTS5
//!   embed_loop(60s) → vectorize new chunks → LanceDB
//!   tag_loop(600s)  → 标签管线（已禁用）

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

static PIPELINE_RUNNING: AtomicBool = AtomicBool::new(false);
static LAST_SYNC_TIME: Mutex<String> = Mutex::new(String::new());

// ─── Entry point ──────────────────────────────────────────────────

pub fn start() {
    if PIPELINE_RUNNING.swap(true, Ordering::AcqRel) {
        println!("[知识管线] 已在运行中");
        return;
    }
    println!("[知识管线] Arkmind 知识管线启动 (v12.0 - 时间戳驱动)");

    // ── Step 1: 清理旧版触发器 ──
    cleanup_legacy_triggers();

    // ── Step 2: 初始化同步时间戳 ──
    init_sync_timestamp();

    // ── Step 3: Vector cache consistency check ──
    clean_vector_cache();

    // ── Step 4: Trash auto-cleanup ──
    auto_clean_trash();

    // ── Step 5: Persist WAL ──
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

// ─── Startup helpers ──────────────────────────────────────────────

fn cleanup_legacy_triggers() {
    let _ = crate::database::run(
        "DROP TRIGGER IF EXISTS ai_sync_products_insert;
         DROP TRIGGER IF EXISTS ai_sync_products_update;
         DROP TRIGGER IF EXISTS sd_reset_sync_on_update;
         DROP TRIGGER IF EXISTS sd_reset_sync_on_insert;",
        &[],
    );
}

fn init_sync_timestamp() {
    // 从 source_data 获取最新时间戳作为初始值
    let latest = crate::database::get(
        "SELECT MAX(updated_at) as ts FROM source_data", &[],
    ).ok().flatten().and_then(|r| r["ts"].as_str().map(String::from));
    
    let mut last_sync = LAST_SYNC_TIME.lock().unwrap();
    *last_sync = latest.unwrap_or_else(|| "1970-01-01 00:00:00".to_string());
    println!("[知识管线] 初始同步时间戳: {}", *last_sync);
}

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

/// 同步循环：每 10s 通过 get_changed_since 接口获取变更，创建/更新 knowledge_chunks + FTS5
async fn sync_loop() {
    loop {
        if !PIPELINE_RUNNING.load(Ordering::Acquire) { break; }

        let last_sync = LAST_SYNC_TIME.lock().unwrap().clone();
        let changed = crate::services::knowledge_lifecycle::get_changed_since(&last_sync, 20);
        
        if !changed.is_empty() {
            println!("[知识管线] 发现 {} 个变更对象（since {}）", changed.len(), last_sync);
            let mut max_updated = last_sync.clone();
            
            for (source_type, source_id, content, updated_at) in &changed {
                if updated_at > &max_updated {
                    max_updated = updated_at.clone();
                }
                // 转换 source_type 为 source_table 格式
                let source_table = match source_type.as_str() {
                    "NOTE" => "notes",
                    "PRODUCT" => "products",
                    "PROJECT" => "projects",
                    "TODO" => "todos",
                    "FILE" => "files",
                    "CHAT" => "chat_sessions",
                    _ => continue,
                };
                
                // 获取 source_data_id
                if let Some(sd_id) = crate::services::knowledge_lifecycle::get_source_data_id(source_type, source_id) {
                    // 调用现有的物化逻辑
                    match crate::services::ai_sync::materialize_and_store(source_table, source_id, sd_id, content).await {
                        Ok(_) => {},
                        Err(e) => eprintln!("[知识管线] 物化失败 {}/{}: {}", source_type, source_id, e),
                    }
                }
            }
            
            // 更新同步时间戳
            *LAST_SYNC_TIME.lock().unwrap() = max_updated;
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

/// 标签循环：每 10 分钟处理一次
async fn tag_loop() {
    loop {
        if !PIPELINE_RUNNING.load(Ordering::Acquire) { break; }
        tokio::time::sleep(Duration::from_secs(600)).await;
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
