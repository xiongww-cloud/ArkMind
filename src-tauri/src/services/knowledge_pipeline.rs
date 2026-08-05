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
static LAST_EMBED_ERROR: Mutex<Option<String>> = Mutex::new(None);

/// 最近一次向量化错误（供设置页管线状态展示）
pub fn get_last_embed_error() -> Option<String> {
    LAST_EMBED_ERROR.lock().unwrap_or_else(|e| e.into_inner()).clone()
}
fn set_last_embed_error(err: Option<String>) {
    *LAST_EMBED_ERROR.lock().unwrap_or_else(|e| e.into_inner()) = err;
}

/// 复合游标格式："updated_at|rowid"（兼容旧格式纯时间戳 → rowid=0）
fn parse_cursor(s: &str) -> (String, i64) {
    match s.split_once('|') {
        Some((ts, rid)) => (ts.to_string(), rid.parse::<i64>().unwrap_or(0)),
        None => (if s.is_empty() { "1970-01-01 00:00:00".to_string() } else { s.to_string() }, 0),
    }
}
fn format_cursor(ts: &str, rowid: i64) -> String { format!("{}|{}", ts, rowid) }

// ─── Entry point ──────────────────────────────────────────────────

/// 启动时清理 LanceDB 孤儿向量（rowid 不在 knowledge_chunks 中）。
/// LanceDB delete 是全表重写（单次 ~6s），故只在启动时批量执行一次，
/// 物化热路径不再逐个删除孤儿向量。
pub fn cleanup_orphan_vectors() {
    if let Ok(g) = crate::services::lancedb_client::get_lancedb() {
        if let Some(ref c) = *g {
            let Ok(all_rowids) = c.scan_all_rowids() else { return; };
            if all_rowids.is_empty() { return; }
            let chunk_ids: std::collections::HashSet<i64> = crate::database::all(
                "SELECT id FROM knowledge_chunks WHERE chunk_type='child'", &[]
            ).unwrap_or_default().iter()
                .filter_map(|r| r["id"].as_i64())
                .collect();
            let orphans: Vec<i64> = all_rowids.into_iter()
                .filter(|id| !chunk_ids.contains(id))
                .collect();
            if orphans.is_empty() { return; }
            match c.delete_chunks_by_rowids(&orphans) {
                Ok(_) => log::info!("[知识管线] 启动清理 LanceDB 孤儿向量: {} 条", orphans.len()),
                Err(e) => log::warn!("[知识管线] LanceDB 孤儿向量清理失败: {}", e),
            }
        }
    }
}

pub fn start() {
    if PIPELINE_RUNNING.swap(true, Ordering::AcqRel) {
        log::info!("[知识管线] 已在运行中");
        return;
    }
    log::info!("[知识管线] Arkmind 知识管线启动 (v12.0 - 时间戳驱动)");

    // ── Step 1: 清理旧版触发器 ──
    cleanup_legacy_triggers();

    // ── Step 1.5: 清理 FTS5 孤儿条目 ──
    crate::services::knowledge_lifecycle::cleanup_orphaned_fts5();

    // ── Step 2: 初始化同步时间戳 ──
    init_sync_timestamp();

    // ── Step 3: Vector cache consistency check ──
    clean_vector_cache();

    // ── Step 3.5: 启动时批量清理 LanceDB 孤儿向量（物化热路径已移除逐条删除） ──
    cleanup_orphan_vectors();

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
            tokio::join!(sync_loop(), embed_loop());
        });
    });
}

pub fn is_running() -> bool {
    PIPELINE_RUNNING.load(Ordering::Acquire)
}

/// 重置同步游标，让管线重新处理所有对象
pub fn reset_sync_time() {
    crate::database::run(
        "DELETE FROM system_settings WHERE key='last_sync_time'",
        &[],
    ).ok();
    *LAST_SYNC_TIME.lock().unwrap_or_else(|e| e.into_inner()) = format_cursor("1970-01-01 00:00:00", 0);
    log::info!("[知识管线] 已重置同步游标，将重新物化所有对象");
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
            log::info!("[知识管线] 删除旧触发器: {}", name);
        }
        if !to_drop.is_empty() {
            log::info!("[知识管线] 共删除 {} 个旧触发器", to_drop.len());
        }
    }
}

fn init_sync_timestamp() {
    // 从数据库加载上次同步游标（兼容旧格式纯时间戳）
    let saved = crate::database::get(
        "SELECT value FROM system_settings WHERE key='last_sync_time'",
        &[],
    ).ok().flatten().and_then(|r| r["value"].as_str().map(String::from));

    let mut last_sync = LAST_SYNC_TIME.lock().unwrap_or_else(|e| e.into_inner());
    *last_sync = saved.unwrap_or_else(|| format_cursor("1970-01-01 00:00:00", 0));
    log::info!("[知识管线] 初始同步游标: {}", *last_sync);
}

fn save_sync_cursor(cursor: &str) {
    let _ = crate::database::run(
        "INSERT OR REPLACE INTO system_settings(key, value, updated_at) VALUES('last_sync_time', ?1, datetime('now','localtime'))",
        &[json!(cursor)],
    );
}

fn clean_vector_cache() {
    match crate::database::run(
        "DELETE FROM vector_cache WHERE chunk_id NOT IN (SELECT id FROM knowledge_chunks)", &[],
    ) { Ok(r) if r.changes > 0 => log::info!("[知识管线] 清理 {} 个过期向量缓存", r.changes), _ => {} }
    let vc_cnt: i64 = crate::database::with_db(|c|
        c.query_row("SELECT COUNT(*) FROM vector_cache", [], |r| r.get(0))).unwrap_or(0);
    let child_cnt: i64 = crate::database::with_db(|c|
        c.query_row("SELECT COUNT(*) FROM knowledge_chunks WHERE chunk_type='child'", [], |r| r.get(0))).unwrap_or(0);
    if vc_cnt > child_cnt { let _ = crate::database::run("DELETE FROM vector_cache", &[]); }
}

fn auto_clean_trash() {
    match crate::database::run(
        "DELETE FROM notes WHERE deleted_at IS NOT NULL AND datetime(deleted_at,'+30 days')<datetime('now', 'localtime')", &[],
    ) { Ok(r) if r.changes > 0 => log::info!("[知识管线] 回收站清理: {} 条笔记已永久删除", r.changes), _ => {} }
}

// ─── 3 independent polling loops ──────────────────────────────────

/// 同步循环：每 10s 扫描 source_data 变更并入队到防抖物化调度器（事件驱动物化）
async fn sync_loop() {
    loop {
        if !PIPELINE_RUNNING.load(Ordering::Acquire) { break; }

        let cursor = LAST_SYNC_TIME.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let (since_ts, since_rowid) = parse_cursor(&cursor);
        let changed = crate::services::knowledge_lifecycle::get_changed_since(&since_ts, since_rowid, 100);

        if !changed.is_empty() {
            log::info!("[知识管线] 发现 {} 个变更对象（since {}|{}）", changed.len(), since_ts, since_rowid);
            for (_, source_type, source_id, updated_at) in &changed {
                log::info!("[知识管线]   - {}/{} updated={}", source_type, source_id, updated_at);
            }
            let mut last_ts = since_ts.clone();
            let mut last_rowid = since_rowid;

            for (rowid, source_type, source_id, updated_at) in &changed {
                // 事件驱动：入队到防抖物化调度器（source_data 已写入，调度器负责物化 + 失败重试）
                // is_tracked 检查避免重置已入队实体的防抖窗口
                if !crate::services::debounced_sync::SCHEDULER.is_tracked(source_type, source_id) {
                    crate::services::debounced_sync::SCHEDULER.enqueue(source_type, source_id);
                }
                // 行按 (updated_at, rowid) 升序，最后处理的即最大游标；enqueue 为 fire-and-forget，游标正常推进
                last_ts = updated_at.clone();
                last_rowid = *rowid;
            }

            let new_cursor = format_cursor(&last_ts, last_rowid);
            *LAST_SYNC_TIME.lock().unwrap_or_else(|e| e.into_inner()) = new_cursor.clone();
            save_sync_cursor(&new_cursor);
        }

        tokio::time::sleep(Duration::from_secs(10)).await;
    }
}

/// 向量化循环：持续处理直到没有待向量化的块
/// O7: 批大小 20；错误指数退避（1s→60s），避免打爆失败 API
async fn embed_loop() {
    let mut backoff = 1u64;
    loop {
        if !PIPELINE_RUNNING.load(Ordering::Acquire) { break; }

        // 持续处理直到没有待向量化的块
        let mut had_work = false;
        let mut had_error = false;
        for _ in 0..10 {  // 每轮最多处理 10 批（200 个块）
            let batch_size = 20;
            match crate::services::hybrid_knowledge::vectorize_new_chunks(batch_size).await {
                Ok(n) => {
                    if n > 0 {
                        log::info!("[知识管线] 已向量化: {n} 个块");
                        set_last_embed_error(None);
                        had_work = true;
                    } else {
                        break; // 没有待处理的块，退出循环
                    }
                }
                Err(e) => {
                    log::warn!("[知识管线] 向量化错误: {e}");
                    set_last_embed_error(Some(format!("{}（{}）", e, chrono::Local::now().format("%H:%M:%S"))));
                    had_error = true;
                    break;
                }
            }
            tokio::task::yield_now().await;  // 让出 CPU，避免阻塞
        }

        // 有工作 → 快速继续；有错误 → 指数退避；空闲 → 常规轮询
        if had_error {
            tokio::time::sleep(Duration::from_secs(backoff)).await;
            backoff = (backoff * 2).min(60);
        } else if had_work {
            backoff = 1;
            tokio::time::sleep(Duration::from_secs(1)).await;
        } else {
            backoff = 1;
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_pipeline_not_running_by_default() { assert!(!is_running()); }
}
