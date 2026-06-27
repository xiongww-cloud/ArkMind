//! 统一记忆库生命周期接口
//!
//! 业务层通过 notify_deleted / notify_changed 与记忆库交互，
//! 不直接操作 source_data / knowledge_chunks / LanceDB。
//!
//! 知识管线通过 get_changed_since 接口获取需要同步的对象，
//! 不直接查询 source_data 表结构。

use serde_json::json;

/// 业务对象删除时调用，通知记忆库清理相关数据
pub fn notify_deleted(source_type: &str, source_id: &str) {
    eprintln!("[记忆库] notify_deleted: {}/{}", source_type, source_id);
    crate::services::cleanup_pipeline::enqueue_cleanup(source_type, source_id);
    crate::services::cleanup_pipeline::wake_cleanup();
}

/// 业务对象创建/更新时调用，通知记忆库同步数据
pub fn notify_changed(source_type: &str, source_id: &str) {
    eprintln!("[记忆库] notify_changed: {}/{}", source_type, source_id);
    crate::services::source_doc_sync::sync_entity(source_type, source_id);
}

/// 获取指定时间之后变更的对象列表（供知识管线调用）
/// 返回 (source_type, source_id, content, updated_at)
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
pub fn get_source_data_id(source_type: &str, source_id: &str) -> Option<i64> {
    crate::database::get(
        "SELECT id FROM source_data WHERE source_type=?1 AND source_id=?2",
        &[json!(source_type), json!(source_id)],
    )
    .ok()
    .flatten()
    .and_then(|r| r["id"].as_i64())
}
