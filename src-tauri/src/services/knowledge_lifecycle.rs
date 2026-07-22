//! 统一知识库生命周期接口
//!
//! 业务层通过 lifecycle 接口与知识库交互，
//! 不直接操作 source_data / knowledge_chunks / LanceDB。
//!
//! 知识管线通过 get_changed_since 接口获取需要同步的对象，
//! 不直接查询 source_data 表结构。

use serde_json::json;

/// 业务对象删除时调用 → 入队等待推送
pub fn notify_deleted(source_type: &str, source_id: &str) {
    log::info!("[知识库] notify_deleted: {}/{}", source_type, source_id);
    crate::services::push_pipeline::enqueue_push(source_type, source_id, "delete");
}

/// 业务对象创建/更新时调用 → 入队等待推送
pub fn notify_changed(source_type: &str, source_id: &str) {
    log::info!("[知识库] notify_changed: {}/{}", source_type, source_id);
    crate::services::push_pipeline::enqueue_push(source_type, source_id, "change");
}

/// 文件上传后调用：入队等待推送
/// 业务层只需传入 file_id，push_worker 完成剩余工作
pub fn notify_file_upload(file_id: &str) {
    log::info!("[知识库] notify_file_upload: {}", file_id);
    crate::services::push_pipeline::enqueue_push("FILE", file_id, "file_upload");
}

/// 重置知识管线：清空 chunks + 向量 + LanceDB，重新物化
pub fn reset_pipeline() {
    log::info!("[知识库] reset_pipeline: 重置知识管线");
    // 1. 清空向量
    crate::database::run("DELETE FROM vector_cache", &[]).ok();
    if let Ok(g) = crate::services::lancedb_client::get_lancedb() {
        if let Some(ref c) = *g { let _ = c.clear_all_vectors(); }
    }
    // 2. 删除 LanceDB 数据目录
    let lancedb_dir = crate::get_data_dir().join("lancedb");
    let _ = std::fs::remove_dir_all(&lancedb_dir);
    crate::services::lancedb_client::invalidate_lancedb_cache();

    // 3. 禁用触发器，清空 chunks + FTS5
    let triggers = ["kk_chunks_fts_insert", "kk_chunks_fts_delete", "kk_chunks_fts_update"];
    for t in &triggers { crate::database::run(&format!("DROP TRIGGER IF EXISTS {}", t), &[]).ok(); }
    crate::database::run("DELETE FROM knowledge_chunks", &[]).ok();
    crate::database::run("DELETE FROM kk_chunks_fts", &[]).ok();

    // 4. 重建触发器
    restore_fts_triggers();

    // 5. 重置同步时间戳并启动管线
    crate::services::knowledge_pipeline::reset_sync_time();
    crate::services::knowledge_pipeline::start();
}

/// 清理 kk_chunks_fts 中的孤儿条目（rowid 不存在于 knowledge_chunks）
/// 在知识管线启动时调用
pub fn cleanup_orphaned_fts5() {
    let before: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM kk_chunks_fts", [], |r| r.get(0))).unwrap_or(0);
    // 删除 FTS5 中 rowid 不在 knowledge_chunks 中的条目
    crate::database::run(
        "DELETE FROM kk_chunks_fts WHERE rowid NOT IN (SELECT rowid FROM knowledge_chunks WHERE chunk_type='child')",
        &[],
    ).ok();
    // 补插 knowledge_chunks 中有但 kk_chunks_fts 中没有的条目
    crate::database::run(
        "INSERT OR IGNORE INTO kk_chunks_fts(rowid, doc_title, content) SELECT rowid, doc_title, content FROM knowledge_chunks WHERE chunk_type='child' AND rowid NOT IN (SELECT rowid FROM kk_chunks_fts)",
        &[],
    ).ok();
    let after: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM kk_chunks_fts", [], |r| r.get(0))).unwrap_or(0);
    if before != after {
        log::info!("[知识库] FTS5 清理: {} → {} 条 (孤儿删除+缺失补插)", before, after);
    } else {
        log::info!("[知识库] FTS5 无需清理 ({} 条)", after);
    }
}

/// 获取指定时间之后变更的对象列表（供知识管线调用）
pub fn get_changed_since(since: &str, limit: i64) -> Vec<(String, String, String, String)> {
    crate::database::all(
        "SELECT source_type, source_id, content, updated_at FROM source_data WHERE updated_at > ?1 ORDER BY updated_at ASC LIMIT ?2",
        &[json!(since), json!(limit)],
    )
    .unwrap_or_default()
    .iter()
    .filter_map(|r| {
        let st = r["source_type"].as_str()?.to_string();
        let sid = r["source_id"].as_str()?.to_string();
        let content = r["content"].as_str().unwrap_or("").to_string();
        let updated_at = r["updated_at"].as_str().unwrap_or("").to_string();
        Some((st, sid, content, updated_at))
    })
    .collect()
}

/// 获取指定 source_data 的 id（供知识管线调用）
pub fn get_source_data_id(_source_type: &str, source_id: &str) -> Option<String> {
    Some(source_id.to_string())
}

/// 重建 FTS5 触发器
fn restore_fts_triggers() {
    crate::database::run("CREATE TRIGGER IF NOT EXISTS sd_chunks_delete AFTER DELETE ON source_data BEGIN DELETE FROM knowledge_chunks WHERE source_type = old.source_type AND source_id = old.source_id; END", &[]).ok();
    // global_search_fts 是 external content 表，触发器只维护 rowid 倒排索引
    crate::database::run("CREATE TRIGGER IF NOT EXISTS sd_fts_insert AFTER INSERT ON source_data BEGIN INSERT INTO global_search_fts(rowid) VALUES(new.rowid); END", &[]).ok();
    crate::database::run("CREATE TRIGGER IF NOT EXISTS sd_fts_delete AFTER DELETE ON source_data BEGIN INSERT INTO global_search_fts(global_search_fts, rowid) VALUES('delete', old.rowid); END", &[]).ok();
    crate::database::run("CREATE TRIGGER IF NOT EXISTS sd_fts_update AFTER UPDATE ON source_data BEGIN INSERT INTO global_search_fts(global_search_fts, rowid) VALUES('delete', old.rowid); INSERT INTO global_search_fts(rowid) VALUES(new.rowid); END", &[]).ok();
    // kk_chunks_fts 触发器：使用 DROP + CREATE 确保定义始终最新
    // 注意：不创建 DELETE 触发器，FTS5 清理在应用代码中处理（更可靠）
    let _ = crate::database::run("DROP TRIGGER IF EXISTS kk_chunks_fts_delete", &[]);
    let _ = crate::database::run("DROP TRIGGER IF EXISTS kk_chunks_fts_insert", &[]);
    let _ = crate::database::run("DROP TRIGGER IF EXISTS kk_chunks_fts_update", &[]);
    let _ = crate::database::run("CREATE TRIGGER kk_chunks_fts_insert AFTER INSERT ON knowledge_chunks WHEN new.chunk_type='child' BEGIN INSERT INTO kk_chunks_fts(rowid, doc_title, content) VALUES (new.rowid, new.doc_title, new.content); END", &[]);
    let _ = crate::database::run("CREATE TRIGGER kk_chunks_fts_update AFTER UPDATE ON knowledge_chunks WHEN old.chunk_type='child' OR new.chunk_type='child' BEGIN DELETE FROM kk_chunks_fts WHERE rowid = old.rowid; INSERT INTO kk_chunks_fts(rowid, doc_title, content) SELECT new.rowid, new.doc_title, new.content WHERE new.chunk_type='child'; END", &[]);
    let _ = crate::database::with_db(|c| { c.pragma_update(None, "foreign_keys", "ON").ok(); Ok(()) });
}
