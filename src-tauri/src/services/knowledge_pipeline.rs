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
static FAILED_RETRY: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());
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
    FAILED_RETRY.lock().unwrap_or_else(|e| e.into_inner()).clear();
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

/// 同步循环：每 10s 通过 get_changed_since 接口获取变更，创建/更新 knowledge_chunks + FTS5
async fn sync_loop() {
    loop {
        if !PIPELINE_RUNNING.load(Ordering::Acquire) { break; }

        // ── 优先重试上轮物化失败的对象（失败不推进游标之外的兜底） ──
        let retry_list: Vec<(String, String)> = FAILED_RETRY.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if !retry_list.is_empty() {
            log::info!("[知识管线] 重试 {} 个物化失败对象", retry_list.len());
            for (source_type, source_id) in &retry_list {
                let row = crate::database::get(
                    "SELECT id, content FROM source_data WHERE source_type=?1 AND source_id=?2",
                    &[json!(source_type), json!(source_id)],
                ).ok().flatten();
                match row {
                    Some(r) => {
                        let sd_id = r["id"].as_str().unwrap_or("").to_string();
                        let content = r["content"].as_str().unwrap_or("").to_string();
                        match crate::services::ai_sync::materialize_and_store(source_type, source_id, sd_id, &content).await {
                            Ok(_) => {
                                FAILED_RETRY.lock().unwrap_or_else(|e| e.into_inner())
                                    .retain(|(t, i)| !(t == source_type && i == source_id));
                                log::info!("[知识管线] 重试成功: {}/{}", source_type, source_id);
                            },
                            Err(e) => log::warn!("[知识管线] 重试仍失败 {}/{}: {}", source_type, source_id, e),
                        }
                    }
                    None => {
                        // 源数据已删除，从重试列表移除
                        FAILED_RETRY.lock().unwrap_or_else(|e| e.into_inner())
                            .retain(|(t, i)| !(t == source_type && i == source_id));
                    }
                }
            }
        }

        let cursor = LAST_SYNC_TIME.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let (since_ts, since_rowid) = parse_cursor(&cursor);
        let changed = crate::services::knowledge_lifecycle::get_changed_since(&since_ts, since_rowid, 100);

        if !changed.is_empty() {
            log::info!("[知识管线] 发现 {} 个变更对象（since {}|{}）", changed.len(), since_ts, since_rowid);
            for (_, source_type, source_id, _, updated_at) in &changed {
                log::info!("[知识管线]   - {}/{} updated={}", source_type, source_id, updated_at);
            }
            let mut last_ts = since_ts.clone();
            let mut last_rowid = since_rowid;

            for (rowid, source_type, source_id, content, updated_at) in &changed {
                // 获取 source_data_id
                if let Some(sd_id) = crate::services::knowledge_lifecycle::get_source_data_id(source_type, source_id) {
                    // 直接传入 source_type，由 materialize_and_store 内部处理类型转换
                    match crate::services::ai_sync::materialize_and_store(source_type, source_id, sd_id, content).await {
                        Ok(_) => {},
                        Err(e) => {
                            log::warn!("[知识管线] 物化失败 {}/{}: {}（已加入重试队列）", source_type, source_id, e);
                            let mut failed = FAILED_RETRY.lock().unwrap_or_else(|e| e.into_inner());
                            if !failed.iter().any(|(t, i)| t == source_type && i == source_id) {
                                failed.push((source_type.clone(), source_id.clone()));
                            }
                        },
                    }
                }
                // 无论成败都推进游标（失败的由重试队列兜底，避免毒丸阻塞）
                // 行按 (updated_at, rowid) 升序，最后处理的即最大游标
                last_ts = updated_at.clone();
                last_rowid = *rowid;
            }

            // 更新同步游标（复合：时间戳 + rowid，防止同秒边界跳行）
            let new_cursor = format_cursor(&last_ts, last_rowid);
            *LAST_SYNC_TIME.lock().unwrap_or_else(|e| e.into_inner()) = new_cursor.clone();
            save_sync_cursor(&new_cursor);
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
                    break;
                }
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_pipeline_not_running_by_default() { assert!(!is_running()); }
}
