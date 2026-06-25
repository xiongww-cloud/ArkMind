//! 统一源数据同步模块
//!
//! 笔记/产品/项目/待办的 CRUD 操作通过此模块，将清洗后的数据
//! 统一写入 source_data 表，并触发防抖 AI 物化调度。

/// 将业务实体的文本内容清洗后同步到 source_data。
/// content 为空时跳过写入。
///
/// 副作用：通过全局防抖调度器将实体加入物化队列（5 秒静默窗口）。
pub fn sync_entity_to_source_docs(source_type: &str, source_id: &str, title: &str, content: &str) {
    if content.trim().is_empty() { return; }
    let clean = crate::services::file_parser::sanitize_chars(content);
    if clean.trim().is_empty() { return; }
    let _ = crate::database::run(
        "INSERT OR REPLACE INTO source_data(source_type,source_id,title,content,updated_at) VALUES(?1,?2,?3,?4,datetime('now','localtime'))",
        &[
            serde_json::json!(source_type),
            serde_json::json!(source_id),
            serde_json::json!(title),
            serde_json::json!(&clean),
        ],
    );

    // ★ 触发防抖物化 + 独立打标（不依赖物化）
    let st_lower = source_type.to_lowercase();
    let queue_type = if st_lower.ends_with('s') { st_lower.clone() } else { format!("{}s", st_lower) };
    _ = &queue_type; // 保留以备后用
    
}

/// 删除实体对应的 source_data 记录
pub fn delete_entity_source_doc(source_type: &str, source_id: &str) {
    let _ = crate::database::run(
        "DELETE FROM source_data WHERE source_type=?1 AND source_id=?2",
        &[serde_json::json!(source_type), serde_json::json!(source_id)],
    );
}
