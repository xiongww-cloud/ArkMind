//! KnowledgePipeline — 3 independent polling loops
//!
//! Schedule:
//!   sync_loop(10s)  → 通过 get_changed_since 接口获取变更 → materialize → knowledge_chunks → FTS5
//!   embed_loop(60s) → vectorize new chunks → LanceDB
//!   tag_loop(600s)  → 标签管线（已禁用）

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use serde_json::json;

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

    // ── Step 1.5: 清理 FTS5 孤儿条目 ──
    crate::services::knowledge_lifecycle::cleanup_orphaned_fts5();

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

/// 重置同步时间戳，让管线重新处理所有对象
pub fn reset_sync_time() {
    crate::database::run(
        "DELETE FROM system_settings WHERE key='last_sync_time'",
        &[],
    ).ok();
    *LAST_SYNC_TIME.lock().unwrap_or_else(|e| e.into_inner()) = "1970-01-01 00:00:00".to_string();
    println!("[知识管线] 已重置同步时间戳，将重新物化所有对象");
}

// ─── Startup helpers ──────────────────────────────────────────────

fn cleanup_legacy_triggers() {
    // 查询所有触发器名称，删除 sd_ 前缀的旧触发器（保留 sd_fts_* 和 sd_chunks_*）
    if let Ok(rows) = crate::database::all("SELECT name FROM sqlite_master WHERE type='trigger' AND name LIKE 'sd_%'", &[]) {
        let to_drop: Vec<String> = rows.iter()
            .filter_map(|r| r["name"].as_str())
            .filter(|n| !n.starts_with("sd_fts_") && !n.starts_with("sd_chunks_"))
            .map(|n| n.to_string())
            .collect();
        for name in &to_drop {
            let _ = crate::database::run(&format!("DROP TRIGGER IF EXISTS {}", name), &[]);
            println!("[知识管线] 删除旧触发器: {}", name);
        }
        if !to_drop.is_empty() {
            println!("[知识管线] 共删除 {} 个旧触发器", to_drop.len());
        }
    }
}

fn init_sync_timestamp() {
    // 从数据库加载上次同步时间戳
    let saved = crate::database::get(
        "SELECT value FROM system_settings WHERE key='last_sync_time'",
        &[],
    ).ok().flatten().and_then(|r| r["value"].as_str().map(String::from));
    
    let mut last_sync = LAST_SYNC_TIME.lock().unwrap_or_else(|e| e.into_inner());
    *last_sync = saved.unwrap_or_else(|| "1970-01-01 00:00:00".to_string());
    println!("[知识管线] 初始同步时间戳: {}", *last_sync);
}

fn save_sync_timestamp(ts: &str) {
    let _ = crate::database::run(
        "INSERT OR REPLACE INTO system_settings(key, value, updated_at) VALUES('last_sync_time', ?1, datetime('now','localtime'))",
        &[json!(ts)],
    );
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

        let last_sync = LAST_SYNC_TIME.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let changed = crate::services::knowledge_lifecycle::get_changed_since(&last_sync, 100);
        
        if !changed.is_empty() {
            println!("[知识管线] 发现 {} 个变更对象（since {}）", changed.len(), last_sync);
            for (source_type, source_id, _, updated_at) in &changed {
                println!("[知识管线]   - {}/{} updated={}", source_type, source_id, updated_at);
            }
            let mut max_updated = last_sync.clone();
            
            for (source_type, source_id, content, updated_at) in &changed {
                if updated_at > &max_updated {
                    max_updated = updated_at.clone();
                }
                // 获取 source_data_id
                if let Some(sd_id) = crate::services::knowledge_lifecycle::get_source_data_id(source_type, source_id) {
                    // 直接传入 source_type，由 materialize_and_store 内部处理类型转换
                    match crate::services::ai_sync::materialize_and_store(source_type, source_id, sd_id, content).await {
                        Ok(_) => {},
                        Err(e) => eprintln!("[知识管线] 物化失败 {}/{}: {}", source_type, source_id, e),
                    }
                }
            }
            
            // 更新同步时间戳
            *LAST_SYNC_TIME.lock().unwrap_or_else(|e| e.into_inner()) = max_updated.clone();
            save_sync_timestamp(&max_updated);
        }
        
        tokio::time::sleep(Duration::from_secs(10)).await;
    }
}

/// 向量化循环：持续处理直到没有待向量化的块
async fn embed_loop() {
    loop {
        if !PIPELINE_RUNNING.load(Ordering::Acquire) { break; }
        
        // 持续处理直到没有待向量化的块
        let mut had_work = false;
        for _ in 0..10 {  // 每轮最多处理 10 批（50 个块）
            let batch_size = 5;
            match crate::services::hybrid_knowledge::vectorize_new_chunks(batch_size).await {
                Ok(n) => { 
                    if n > 0 { 
                        println!("[知识管线] 已向量化: {n} 个块"); 
                        had_work = true;
                    } else {
                        break; // 没有待处理的块，退出循环
                    }
                }
                Err(e) => { eprintln!("[知识管线] 向量化错误: {e}"); break; }
            }
            tokio::task::yield_now().await;  // 让出 CPU，避免阻塞
        }
        
        // 如果有工作，快速继续；否则等待 10 秒
        if had_work {
            tokio::time::sleep(Duration::from_secs(1)).await;
        } else {
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
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
