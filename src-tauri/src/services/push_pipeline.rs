//! 业务层推送管线
//!
//! 业务层通过 enqueue_push 将变更入队到 knowledge_push_queue 表。
//! 数据中台（file_ingestion.rs）负责从队列获取任务并处理。
//!
//! 设计目标：即使知识库暂时可用，变更也不会丢失，
//! 数据中台会持续重试直到成功。

use serde_json::json;

/// 业务层调用：将变更入队等待推送（同一实体只保留一条 pending，5秒防抖窗口）
pub fn enqueue_push(source_type: &str, source_id: &str, operation: &str) {
    // 先删除同实体的 pending 条目，避免重复推送
    let _ = crate::database::run(
        "DELETE FROM knowledge_push_queue WHERE source_type=?1 AND source_id=?2 AND status='pending'",
        &[json!(source_type), json!(source_id)],
    );
    // file_upload 操作不需要防抖窗口，立即处理
    // 其他操作设置 5 秒防抖窗口
    let debounce = if operation == "file_upload" {
        "NULL"
    } else {
        "datetime('now','localtime','+5 seconds')"
    };
    let _ = crate::database::run(
        &format!(
            "INSERT INTO knowledge_push_queue(source_type, source_id, operation, status, debounce_until) \
             VALUES(?1, ?2, ?3, 'pending', {})",
            debounce
        ),
        &[json!(source_type), json!(source_id), json!(operation)],
    );
    log::info!("[推送管线] 入队: {}/{} ({})", source_type, source_id, operation);
}
