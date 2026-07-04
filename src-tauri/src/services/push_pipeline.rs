//! 业务层推送管线
//!
//! 业务层通过 enqueue_push 将变更入队，push_worker 负责
//! 将变更最终送达知识库（写入 source_data）。
//!
//! 设计目标：即使知识库暂时不可用，变更也不会丢失，
//! push_worker 会持续重试直到成功。

use std::sync::atomic::{AtomicBool, Ordering};
use serde_json::json;

static PUSH_RUNNING: AtomicBool = AtomicBool::new(false);
static PUSH_NOTIFY: AtomicBool = AtomicBool::new(false);

/// 业务层调用：将变更入队等待推送（同一实体只保留一条 pending，5秒防抖窗口）
pub fn enqueue_push(source_type: &str, source_id: &str, operation: &str) {
    // 先删除同实体的 pending 条目，避免重复推送
    let _ = crate::database::run(
        "DELETE FROM knowledge_push_queue WHERE source_type=?1 AND source_id=?2 AND status='pending'",
        &[json!(source_type), json!(source_id)],
    );
    // 插入新条目，设置 5 秒防抖窗口
    let _ = crate::database::run(
        "INSERT INTO knowledge_push_queue(source_type, source_id, operation, status, debounce_until) \
         VALUES(?1, ?2, ?3, 'pending', datetime('now','localtime','+5 seconds'))",
        &[json!(source_type), json!(source_id), json!(operation)],
    );
    wake_push();
}

/// 唤醒 push_worker 立即处理
pub fn wake_push() {
    PUSH_NOTIFY.store(true, Ordering::Release);
}

/// 启动 push worker 后台线程
pub fn start() {
    if PUSH_RUNNING.swap(true, Ordering::AcqRel) {
        println!("[推送管线] 已在运行中，忽略重复启动");
        return;
    }
    println!("[推送管线] 启动");
    std::thread::spawn(|| push_loop());
}

/// 停止 push worker
pub fn stop() {
    PUSH_RUNNING.store(false, Ordering::Release);
}

fn push_loop() {
    loop {
        if !PUSH_RUNNING.load(Ordering::Acquire) {
            println!("[推送管线] 收到停止信号，退出");
            break;
        }

        // 处理 pending 条目（仅处理防抖窗口已过的条目）
        let pending = crate::database::all(
            "SELECT id, source_type, source_id, operation, retry_count \
             FROM knowledge_push_queue WHERE status='pending' \
             AND (debounce_until IS NULL OR debounce_until <= datetime('now','localtime')) \
             ORDER BY created_at ASC LIMIT 10",
            &[],
        )
        .unwrap_or_default();

        if !pending.is_empty() {
            println!("[推送管线] 本次处理 {} 条待推送变更", pending.len());
        }

        for row in &pending {
            let id = row["id"].as_i64().unwrap_or(0);
            let source_type = row["source_type"].as_str().unwrap_or("");
            let source_id = row["source_id"].as_str().unwrap_or("");
            let operation = row["operation"].as_str().unwrap_or("");
            let retry_count = row["retry_count"].as_i64().unwrap_or(0);

            // 标记 processing
            let _ = crate::database::run(
                "UPDATE knowledge_push_queue SET status='processing', \
                 updated_at=datetime('now','localtime') WHERE id=?1",
                &[json!(id)],
            );

            let result = process_push_item(source_type, source_id, operation);

            match result {
                Ok(()) => {
                    let _ = crate::database::run(
                        "DELETE FROM knowledge_push_queue WHERE id=?1",
                        &[json!(id)],
                    );
                    eprintln!("[推送管线] ✓ {}/{} ({})", source_type, source_id, operation);
                }
                Err(e) => {
                    let new_retry = retry_count + 1;
                    if new_retry >= 10 {
                        // 超过最大重试次数，标记 failed
                        let _ = crate::database::run(
                            "UPDATE knowledge_push_queue SET status='failed', \
                             retry_count=?1, last_error=?2, \
                             updated_at=datetime('now','localtime') WHERE id=?3",
                            &[json!(new_retry), json!(&e), json!(id)],
                        );
                        eprintln!("[推送管线] ✗ {}/{} 超过最大重试次数: {}", source_type, source_id, e);
                    } else {
                        // 退回 pending，重置防抖窗口，等待下次重试
                        let _ = crate::database::run(
                            "UPDATE knowledge_push_queue SET status='pending', \
                             debounce_until=datetime('now','localtime','+5 seconds'), \
                             retry_count=?1, last_error=?2, \
                             updated_at=datetime('now','localtime') WHERE id=?3",
                            &[json!(new_retry), json!(&e), json!(id)],
                        );
                        eprintln!("[推送管线] ⚠ {}/{} 重试 {}/10: {}", source_type, source_id, new_retry, e);
                    }
                }
            }
        }

        // 空闲时等待唤醒或超时
        if pending.is_empty() {
            let mut waited = 0u32;
            while waited < 30 {
                if PUSH_NOTIFY.swap(false, Ordering::Acquire) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
                waited += 1;
            }
        }
    }
}

/// 处理单条推送任务
fn process_push_item(source_type: &str, source_id: &str, operation: &str) -> Result<(), String> {
    match operation {
        "change" => {
            crate::services::source_doc_sync::sync_entity(source_type, source_id);
            Ok(())
        }
        "delete" => {
            crate::services::cleanup_pipeline::enqueue_cleanup(source_type, source_id);
            crate::services::cleanup_pipeline::wake_cleanup();
            Ok(())
        }
        "file_upload" => {
            // 文件需要解析，直接调用 notify_file_upload 完成解析+写入
            crate::services::knowledge_lifecycle::notify_file_upload_inner(source_id);
            Ok(())
        }
        _ => Err(format!("未知操作: {}", operation)),
    }
}
