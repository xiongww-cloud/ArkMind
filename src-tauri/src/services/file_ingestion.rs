//! 统一数据中台 (Unified Data Ingestion Mid-Platform)
//!
//! 统一从 knowledge_push_queue 表获取待处理任务，处理所有业务对象：
//!   - operation="change"      → 从业务表读取数据，清洗后写入 source_data
//!   - operation="file_upload" → 解析文件，清洗后写入 source_data
//!   - operation="delete"      → 级联删除 source_data → knowledge_chunks → vector_cache → LanceDB
//!
//! 支持并发处理，最大并发数 5。
//!
//! 日志前缀统一为 "[数据中台]"。

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;
use serde_json::json;

static INGESTION_RUNNING: AtomicBool = AtomicBool::new(false);
static ACTIVE_TASKS: AtomicUsize = AtomicUsize::new(0);
const MAX_CONCURRENT: usize = 5;

/// 任务错误前缀：带此前缀的错误不参与任务级重试，直接标记为 failed（如视觉模型解析失败）
pub const NON_RETRYABLE_PREFIX: &str = "__NO_RETRY__";

/// 任务错误前缀：不支持的文件格式，标记为 unsupported 终态（不重试、冷启动不重置）
pub const UNSUPPORTED_PREFIX: &str = "__UNSUPPORTED__";

/// 有真实解析器的文件后缀白名单（与 file_parser::parse_file_smart 的路由一致）
const SUPPORTED_PARSE_EXTS: &[&str] = &[
    "txt", "md", "csv", "json",
    "doc", "docx", "xlsx", "xls", "pptx", "ppt",
    "pdf",
    "png", "jpg", "jpeg", "webp", "bmp",
];

fn is_supported_parse_ext(ext: &str) -> bool {
    let e = ext.trim_start_matches('.').to_lowercase();
    SUPPORTED_PARSE_EXTS.contains(&e.as_str())
}

// ═══════════════════════════════════════════════════════════════
// 启动与主循环
// ═══════════════════════════════════════════════════════════════

pub fn start(_app_handle: tauri::AppHandle) {
    if INGESTION_RUNNING.swap(true, Ordering::AcqRel) {
        log::info!("[数据中台] 已在运行中，忽略重复启动");
        return;
    }
    log::info!("[数据中台] v4 启动完成 | 统一入口: knowledge_push_queue | 并发: {} | 支持: change/file_upload/delete", MAX_CONCURRENT);

    std::thread::spawn(move || {
        ingestion_loop();
    });
}

fn ingestion_loop() {
    // 冷启动：将崩溃残留的 processing 状态重置为 pending
    let reset_count = crate::database::run(
        "UPDATE knowledge_push_queue SET status='pending', retry_count=0, last_error='' WHERE status='processing'",
        &[],
    ).map(|r| r.changes).unwrap_or(0);
    if reset_count > 0 {
        log::info!("[数据中台] 冷启动：已重置 {} 个崩溃残留任务", reset_count);
    }

    // 冷启动：将上次 failed 的任务重置为 pending（重启后自动重试）
    let retry_failed = crate::database::run(
        "UPDATE knowledge_push_queue SET status='pending', retry_count=0, last_error='', debounce_until=NULL WHERE status='failed'",
        &[],
    ).map(|r| r.changes).unwrap_or(0);
    if retry_failed > 0 {
        log::info!("[数据中台] 冷启动：已重置 {} 个失败任务为待重试", retry_failed);
    }

    // 清理孤立的 file_parse_cache 数据（没有对应任务的缓存）
    let cache_cleaned = crate::database::run(
        "DELETE FROM file_parse_cache WHERE file_id NOT IN (
            SELECT source_id FROM knowledge_push_queue WHERE source_type='FILE'
        )",
        &[],
    ).map(|r| r.changes).unwrap_or(0);
    if cache_cleaned > 0 {
        log::info!("[数据中台] 冷启动：已清理 {} 条孤立缓存", cache_cleaned);
    }

    loop {
        if !INGESTION_RUNNING.load(Ordering::Acquire) {
            log::info!("[数据中台] 收到停止信号，退出轮询");
            break;
        }

        // 1. 优先处理 knowledge_push_queue 中的待处理任务（非阻塞）
        process_push_queue_non_blocking();

        // 2. 轮询式补漏（处理历史遗留数据）
        ingest_pending_notes();
        ingest_pending_products();
        ingest_pending_projects();
        ingest_pending_todos();
        ingest_pending_emails();

        // 空闲时每100ms检查一次（更快响应新任务）
        std::thread::sleep(Duration::from_millis(100));
    }
}

// ═══════════════════════════════════════════════════════════════
// 核心调度：从 knowledge_push_queue 获取任务（并发处理）
// ═══════════════════════════════════════════════════════════════

/// 从 knowledge_push_queue 表获取待处理任务并并发执行（非阻塞）
fn process_push_queue_non_blocking() {
    // 自愈：重置卡在 processing 超过 5 分钟的条目为 pending（崩溃/静默失败残留自动恢复）
    let stale_reset = crate::database::run(
        "UPDATE knowledge_push_queue SET status='pending', retry_count=0, updated_at=datetime('now','localtime') \
         WHERE status='processing' AND updated_at < datetime('now','localtime','-5 minutes')",
        &[],
    ).map(|r| r.changes).unwrap_or(0);
    if stale_reset > 0 {
        log::warn!("[数据中台] 自愈：重置 {} 个过期 processing 条目为 pending", stale_reset);
    }

    // 检查是否还有并发槽位
    let current_active = ACTIVE_TASKS.load(Ordering::Relaxed);
    if current_active >= MAX_CONCURRENT {
        return; // 没有可用槽位，跳过本轮
    }

    let available_slots = MAX_CONCURRENT - current_active;
    let tasks = match crate::database::all(
        &format!(
            "SELECT id, source_type, source_id, operation, retry_count \
             FROM knowledge_push_queue \
             WHERE status='pending' AND (debounce_until IS NULL OR debounce_until <= datetime('now','localtime')) \
             ORDER BY created_at ASC LIMIT {}",
            available_slots
        ),
        &[],
    ) {
        Ok(r) => r,
        Err(e) => {
            log::info!("[数据中台] ⚠ 查询 knowledge_push_queue 失败: {}", e);
            return;
        }
    };

    if tasks.is_empty() {
        return;
    }

    let task_count = tasks.len();
    log::info!("[数据中台] 本次处理 {} 条待推送变更（当前活跃 {}/{})", task_count, current_active, MAX_CONCURRENT);

    // 创建线程处理所有任务（不等待完成）
    for (idx, task) in tasks.into_iter().enumerate() {
        // 检查并发槽位
        if ACTIVE_TASKS.load(Ordering::Relaxed) >= MAX_CONCURRENT {
            log::info!("[数据中台] 并发槽位已满，剩余任务等待下一轮");
            break;
        }

        ACTIVE_TASKS.fetch_add(1, Ordering::SeqCst);

        let source_type = task["source_type"].as_str().unwrap_or("").to_string();
        let source_id = task["source_id"].as_str().unwrap_or("").to_string();
        let operation = task["operation"].as_str().unwrap_or("").to_string();

        log::info!("[数据中台] → 启动任务 {}/{} ({}) [线程 {}/{}]", source_type, source_id, operation, idx + 1, task_count);

        std::thread::spawn(move || {
            // H12: RAII guard 确保 panic 时 ACTIVE_TASKS 也能递减，防止管线永久停摆
            struct TaskGuard;
            impl Drop for TaskGuard {
                fn drop(&mut self) { ACTIVE_TASKS.fetch_sub(1, Ordering::SeqCst); }
            }
            let _guard = TaskGuard;
            let start = std::time::Instant::now();
            process_task_with_retry(task);
            let elapsed = start.elapsed().as_millis();
            log::info!("[数据中台] ← 完成任务 {}/{} (耗时 {}ms) [剩余活跃 {}]", source_type, source_id, elapsed, ACTIVE_TASKS.load(Ordering::Relaxed));
        });
    }
}

/// 检查任务对应的业务对象是否仍存在（未知类型保守返回 true 不拦截）
fn task_target_exists(source_type: &str, source_id: &str) -> bool {
    let sql = match source_type {
        "FILE" => "SELECT 1 FROM file_assets WHERE id=?1",
        "NOTE" => "SELECT 1 FROM notes WHERE note_id=?1",
        "PRODUCT" => "SELECT 1 FROM products WHERE id=?1",
        "PROJECT" => "SELECT 1 FROM projects WHERE id=?1",
        "TODO" => "SELECT 1 FROM todos WHERE id=?1",
        "CONTACT" => "SELECT 1 FROM partner_contacts WHERE id=?1",
        "CHANNEL" => "SELECT 1 FROM partners WHERE id=?1 AND type='CHANNEL'",
        "VENDOR" => "SELECT 1 FROM partners WHERE id=?1 AND type='VENDOR'",
        "CUSTOMER" => "SELECT 1 FROM partners WHERE id=?1 AND type='CUSTOMER'",
        "CHAT" => "SELECT 1 FROM chat_sessions WHERE id=?1",
        _ => return true,
    };
    crate::database::get(sql, &[json!(source_id)]).ok().flatten().is_some()
}

/// 处理单个任务（带重试逻辑）
fn process_task_with_retry(task: serde_json::Value) {
    let id = task["id"].as_i64().unwrap_or(0);
    let source_type = task["source_type"].as_str().unwrap_or("").to_string();
    let source_id = task["source_id"].as_str().unwrap_or("").to_string();
    let operation = task["operation"].as_str().unwrap_or("").to_string();
    let retry_count = task["retry_count"].as_i64().unwrap_or(0);

    // 标记 processing
    let _ = crate::database::run(
        "UPDATE knowledge_push_queue SET status='processing', updated_at=datetime('now','localtime') WHERE id=?1",
        &[json!(id)],
    );

    // 执行对应操作
    let result = match operation.as_str() {
        "change" => process_change(&source_type, &source_id),
        "file_upload" => process_file_upload(&source_id, &source_type, &source_id),
        "delete" => process_cleanup(&source_type, &source_id),
        _ => Err(format!("未知操作: {}", operation)),
    };

    // 处理结果
    match result {
        Ok(()) => {
            // 成功：删除任务（必须确认删除成功，否则条目卡 processing 导致进度条永久不变）
            match crate::database::run(
                "DELETE FROM knowledge_push_queue WHERE id=?1",
                &[json!(id)],
            ) {
                Ok(_) => log::info!("[数据中台] ✓ {}/{} ({})", source_type, source_id, operation),
                Err(e) => log::error!("[数据中台] 删除队列条目失败 id={}: {}", id, e),
            }
        }
        Err(e) => {
            // 失败重试前检查对象是否存在；对象已删除则任务无意义，直接删除任务
            if operation != "delete" && !task_target_exists(&source_type, &source_id) {
                let _ = crate::database::run(
                    "DELETE FROM knowledge_push_queue WHERE id=?1",
                    &[json!(id)],
                );
                log::info!("[数据中台] ✗ {}/{} 对象已删除，删除无效任务", source_type, source_id);
                return;
            }
            // 不支持的文件格式：标记 unsupported 终态，不参与重试，冷启动也不重置
            if e.starts_with(UNSUPPORTED_PREFIX) {
                let _ = crate::database::run(
                    "UPDATE knowledge_push_queue SET status='unsupported', retry_count=?1, last_error=?2, updated_at=datetime('now','localtime') WHERE id=?3",
                    &[json!(retry_count + 1), json!(&e), json!(id)],
                );
                log::info!("[数据中台] ✗ {}/{} {}", source_type, source_id, e.trim_start_matches(UNSUPPORTED_PREFIX));
                return;
            }
            // 不可重试错误（如视觉模型解析失败）：直接标记 failed，不做任务级重试
            if e.starts_with(NON_RETRYABLE_PREFIX) {
                let _ = crate::database::run(
                    "UPDATE knowledge_push_queue SET status='failed', retry_count=?1, last_error=?2, updated_at=datetime('now','localtime') WHERE id=?3",
                    &[json!(retry_count + 1), json!(&e), json!(id)],
                );
                log::info!("[数据中台] ✗ {}/{} 不可重试失败: {}", source_type, source_id, e);
                return;
            }
            // 失败：退回 pending 或标记 failed
            if retry_count + 1 >= 3 {
                let _ = crate::database::run(
                    "UPDATE knowledge_push_queue SET status='failed', retry_count=?1, last_error=?2, updated_at=datetime('now','localtime') WHERE id=?3",
                    &[json!(retry_count + 1), json!(&e), json!(id)],
                );
                log::info!("[数据中台] ✗ {}/{} 超过最大重试次数: {}", source_type, source_id, e);
            } else {
                let _ = crate::database::run(
                    "UPDATE knowledge_push_queue SET status='pending', debounce_until=datetime('now','localtime','+5 seconds'), retry_count=?1, last_error=?2, updated_at=datetime('now','localtime') WHERE id=?3",
                    &[json!(retry_count + 1), json!(&e), json!(id)],
                );
                log::info!("[数据中台] ⚠ {}/{} 重试 {}/3: {}", source_type, source_id, retry_count + 1, e);
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// 处理 change 操作：从业务表读取数据，清洗后写入 source_data
// ═══════════════════════════════════════════════════════════════

fn process_change(source_type: &str, source_id: &str) -> Result<(), String> {
    crate::services::source_doc_sync::sync_entity(source_type, source_id)
}

// ═══════════════════════════════════════════════════════════════
// 处理 file_upload 操作：解析文件，清洗后写入 source_data
// ═══════════════════════════════════════════════════════════════

fn process_file_upload(file_id: &str, source_type: &str, source_id: &str) -> Result<(), String> {
    // 从 file_assets 获取文件元数据（不依赖 parse_status）
    let row = match crate::database::get(
        "SELECT id, file_ext, file_size, content_hash, original_name FROM file_assets WHERE id=?1",
        &[json!(file_id)],
    ) {
        Ok(Some(r)) => r,
        Ok(None) => return Err(format!("文件不存在: {}", file_id)),
        Err(e) => return Err(format!("查询文件失败: {}", e)),
    };

    // 检查是否是PDF文件，如果是则使用流式处理
    let ext = row["file_ext"].as_str().unwrap_or("");
    if ext.eq_ignore_ascii_case("pdf") {
        return process_pdf_streaming(&row, source_type, source_id);
    }

    process_single_file(&row, source_type, source_id)
}

// ═══════════════════════════════════════════════════════════════
// PDF 流式解析：逐页渲染 + OCR，支持断点续传
// ═══════════════════════════════════════════════════════════════

fn process_pdf_streaming(row: &serde_json::Value, source_type: &str, source_id: &str) -> Result<(), String> {
    use crate::services::file_parser::{get_pdf_page_count, pdfium_render_page, ocr_image_sync_result, has_real_text, OcrOutcome};

    let file_id = row["id"].as_str().unwrap_or("").to_string();
    if file_id.is_empty() {
        return Err("文件ID为空".to_string());
    }

    // 日志标识：{source_type}/{source_id前8位}
    let log_tag = format!("{}/{}", source_type, &source_id[..source_id.len().min(8)]);

    let hash = row["content_hash"].as_str().unwrap_or("");
    let path = if hash.is_empty() {
        String::new()
    } else {
        crate::commands::file_commands::flat_disk_path(hash)
    };

    // 文件可用性检查
    if path.is_empty() || !std::path::Path::new(&path).exists() {
        return Err(format!("文件缺失: {}", path));
    }

    // 获取PDF总页数
    let total_pages = get_pdf_page_count(&path)
        .ok_or("无法获取PDF页数")?;

    log::info!("[数据中台] {} PDF 流式解析 ({} 页)", log_tag, total_pages);

    // 更新任务进度总数
    let _ = crate::database::run(
        "UPDATE knowledge_push_queue SET progress_total=?1, progress_page=0 WHERE source_id=?2 AND source_type=?3",
        &[json!(total_pages), json!(source_id), json!(source_type)],
    );

    // 检查视觉模型配置
    let has_vision = crate::services::file_parser::get_vision_config().is_some();
    if !has_vision {
        log::info!("[数据中台] {} 未配置视觉模型，使用 pdfium 文本提取", log_tag);
        return process_pdf_text_only(&file_id, &path, total_pages, source_type, source_id);
    }

    // 按页号有序收集
    let mut full_text_parts: Vec<Option<String>> = vec![None; total_pages as usize];

    // 逐页流式处理（单文件内串行；多文件间的并行由调度层负责，避免视觉模型过载）
    let mut page_num: u16 = 0;
    while page_num < total_pages {
        let p = page_num;
        page_num += 1;

        // 每页检查文件是否仍然存在
        if !std::path::Path::new(&path).exists() {
            log::info!("[数据中台] {} 文件已被删除，终止解析（已处理 {} / {} 页）", log_tag, page_num, total_pages);
            break;
        }

        // 跳过已有缓存结果的页
        let cached_text = crate::database::get(
            "SELECT ocr_text FROM file_parse_cache WHERE file_id=?1 AND page_num=?2",
            &[json!(&file_id), json!(p)],
        ).ok().flatten().and_then(|r| r["ocr_text"].as_str().map(|s| s.to_string()));
        if let Some(text) = cached_text {
            if has_real_text(&text) {
                full_text_parts[p as usize] = Some(text);
            }
            log::info!("[数据中台] {} 第 {}/{} 页: 使用缓存", log_tag, p + 1, total_pages);
            continue;
        }

        let t_page = std::time::Instant::now();
        let image_uri = match pdfium_render_page(&path, p) {
            Some(uri) => uri,
            None => {
                log::info!("[数据中台] {} 第 {}/{} 页 OCR 失败: 渲染失败", log_tag, p + 1, total_pages);
                return Err(format!("{}{} 第 {}/{} 页渲染失败", NON_RETRYABLE_PREFIX, log_tag, p + 1, total_pages));
            }
        };
        let outcome = ocr_image_sync_result(&image_uri);

        match outcome {
            OcrOutcome::Failed(e) => {
                // 视觉模型解析失败（已重试 3 次）→ 整文件失败，不写入 source_data
                log::info!("[数据中台] {} 第 {}/{} 页 OCR 失败: {}", log_tag, p + 1, total_pages, e);
                return Err(format!("{}{} 第 {}/{} 页视觉模型解析失败: {}", NON_RETRYABLE_PREFIX, log_tag, p + 1, total_pages, e));
            }
            OcrOutcome::Blank => {
                // 空白页：跳过，不缓存、不判定失败
                log::info!("[数据中台] {} 第 {}/{} 页: 空白页，跳过", log_tag, p + 1, total_pages);
            }
            OcrOutcome::Ok(ocr_text) => {
                // 仅当有真实文本时写入缓存（修复缓存投毒）
                if has_real_text(&ocr_text) {
                    let _ = crate::database::run(
                        "INSERT OR REPLACE INTO file_parse_cache(file_id, page_num, ocr_text) VALUES(?1, ?2, ?3)",
                        &[json!(&file_id), json!(p), json!(ocr_text)],
                    );
                    full_text_parts[p as usize] = Some(ocr_text.clone());
                }
            }
        }
        // 更新进度（心跳：刷新 updated_at，避免长时间 OCR 被自愈误判为卡死而重复启动）
        let _ = crate::database::run(
            "UPDATE knowledge_push_queue SET progress_page=?1, updated_at=datetime('now','localtime') WHERE source_id=?2 AND source_type=?3",
            &[json!(p + 1), json!(source_id), json!(source_type)],
        );
        log::info!("[数据中台] {} 第 {}/{} 页: OCR 完成 (整页耗时 {}ms)", log_tag, p + 1, total_pages, t_page.elapsed().as_millis());
    }

    // Tier 1 修复：文件在解析过程中被删除 → 丢弃已解析部分，不写入 source_data
    // （返回 Err 后，process_task_with_retry 会检查 task_target_exists 并删除无效任务）
    if !std::path::Path::new(&path).exists() {
        log::info!("[数据中台] {} 文件已被删除，丢弃已解析部分", log_tag);
        return Err("文件已被删除".into());
    }

    // 按页号组装 full_text
    let mut full_text = String::new();
    for (i, part) in full_text_parts.iter().enumerate() {
        if let Some(text) = part {
            if !full_text.is_empty() {
                full_text.push_str(&format!("\n\n--- 第 {} 页 ---\n\n", i + 1));
            }
            full_text.push_str(text);
        }
    }

    if full_text.is_empty() {
        return Err("PDF 解析结果为空".to_string());
    }

    // 文本清洗
    let clean_text = crate::services::file_parser::clean_extracted_text_with_hint(&full_text, "pdf");
    if clean_text.trim().is_empty() {
        return Err("清洗后无有效文本".to_string());
    }

    // 写入 source_data
    crate::database::file_sync_to_source_data(&file_id, &clean_text);

    // 清理临时缓存
    let _ = crate::database::run(
        "DELETE FROM file_parse_cache WHERE file_id=?1",
        &[json!(&file_id)],
    );

    log::info!("[数据中台] {} PDF 流式解析完成 ({} 字符)", log_tag, clean_text.len());
    Ok(())
}

/// PDF 纯文本提取（无视觉模型时的回退方案）
fn process_pdf_text_only(file_id: &str, path: &str, _total_pages: u16, source_type: &str, source_id: &str) -> Result<(), String> {
    use crate::services::file_parser::try_pdfium_text_per_page;

    // 日志标识：{source_type}/{source_id前8位}
    let log_tag = format!("{}/{}", source_type, &source_id[..source_id.len().min(8)]);

    let pdfium_pages = try_pdfium_text_per_page(path)
        .ok_or("pdfium 文本提取失败")?;

    let mut full_text = String::new();
    for (i, text) in pdfium_pages.iter().enumerate() {
        if !text.is_empty() {
            if !full_text.is_empty() {
                full_text.push_str(&format!("\n\n--- 第 {} 页 ---\n\n", i + 1));
            }
            full_text.push_str(text);
        }

        // 更新进度
        let _ = crate::database::run(
            "UPDATE knowledge_push_queue SET progress_page=?1 WHERE source_id=?2 AND source_type=?3",
            &[json!(i + 1), json!(source_id), json!(source_type)],
        );
    }

    if full_text.is_empty() {
        return Err("PDF 文本提取结果为空".to_string());
    }

    // 文本清洗
    let clean_text = crate::services::file_parser::clean_extracted_text_with_hint(&full_text, "pdf");
    if clean_text.trim().is_empty() {
        return Err("清洗后无有效文本".to_string());
    }

    // 写入 source_data
    crate::database::file_sync_to_source_data(file_id, &clean_text);

    // 清理临时缓存
    let _ = crate::database::run(
        "DELETE FROM file_parse_cache WHERE file_id=?1",
        &[json!(file_id)],
    );

    log::info!("[数据中台] {} PDF 文本提取完成 ({} 字符)", log_tag, clean_text.len());
    Ok(())
}

// ═══════════════════════════════════════════════════════════════
// 处理 delete 操作：级联删除 source_data → knowledge_chunks → vector_cache → LanceDB
// ═══════════════════════════════════════════════════════════════

fn process_cleanup(source_type: &str, source_id: &str) -> Result<(), String> {
    log::info!("[数据中台] 开始清理: {}/{}", source_type, source_id);

    // 1. 获取关联的 chunk_ids（用于清理 vector_cache 和 LanceDB）
    let chunk_ids: Vec<i64> = crate::database::all(
        "SELECT id FROM knowledge_chunks WHERE source_type=?1 AND source_id=?2",
        &[json!(source_type), json!(source_id)],
    )
    .unwrap_or_default()
    .iter()
    .filter_map(|r| r["id"].as_i64())
    .collect();

    // 2. 清理 knowledge_chunks
    let _ = crate::database::run(
        "DELETE FROM knowledge_chunks WHERE source_type=?1 AND source_id=?2",
        &[json!(source_type), json!(source_id)],
    );

    // 3. 清理 vector_cache
    if !chunk_ids.is_empty() {
        let ph: Vec<String> = chunk_ids.iter().map(|_| "?".to_string()).collect();
        let params: Vec<serde_json::Value> = chunk_ids.iter().map(|id| json!(id)).collect();
        let _ = crate::database::run(
            &format!("DELETE FROM vector_cache WHERE chunk_id IN ({})", ph.join(",")),
            &params,
        );
    }

    // 4. 清理 LanceDB
    if !chunk_ids.is_empty() {
        if let Ok(g) = crate::services::lancedb_client::get_lancedb() {
            if let Some(ref c) = *g {
                if let Err(e) = c.delete_chunks_by_rowids(&chunk_ids) {
                    log::info!("[数据中台] LanceDB 清理失败: {}", e);
                }
            }
        }
    }

    // 5. 清理 source_data
    let _ = crate::database::run(
        "DELETE FROM source_data WHERE source_type=?1 AND source_id=?2",
        &[json!(source_type), json!(source_id)],
    );

    // 6. 清理 FTS5 索引
    let _ = crate::database::run(
        "DELETE FROM global_search_fts WHERE source_type=?1 AND source_id=?2",
        &[json!(source_type), json!(source_id)],
    );

    log::info!("[数据中台] 清理完成: {}/{}", source_type, source_id);
    Ok(())
}

// ═══════════════════════════════════════════════════════════════
// 文件解析核心逻辑
// ═══════════════════════════════════════════════════════════════

fn process_single_file(row: &serde_json::Value, source_type: &str, source_id: &str) -> Result<(), String> {
    let file_id = row["id"].as_str().unwrap_or("").to_string();
    if file_id.is_empty() {
        return Err("文件ID为空".to_string());
    }

    // 日志标识：{source_type}/{source_id前8位}
    let log_tag = format!("{}/{}", source_type, &source_id[..source_id.len().min(8)]);

    let file_size = row["file_size"].as_i64().unwrap_or(0);
    let hash = row["content_hash"].as_str().unwrap_or("");
    let path = if hash.is_empty() {
        String::new()
    } else {
        crate::commands::file_commands::flat_disk_path(hash)
    };
    let ext = row["file_ext"].as_str().unwrap_or("").to_string();
    let size_fmt = if file_size >= 1048576 {
        format!("{:.1}MB", file_size as f64 / 1048576.0)
    } else {
        format!("{}KB", file_size / 1024)
    };

    log::info!("[数据摄入中台] {} 开始处理 ({}, {})", log_tag, ext, size_fmt);

    // ── 文件可用性检查 ──
    if path.is_empty() || !std::path::Path::new(&path).exists() {
        return Err(format!("文件缺失: {}", path));
    }

    // ════════════════════════════════════════
    // ① 智能解析
    // ════════════════════════════════════════
    let parse_start = std::time::Instant::now();
    let parse_result = crate::services::file_parser::parse_file_smart(&path, &ext);
    let parse_elapsed = parse_start.elapsed().as_millis();

    if !parse_result.success || parse_result.text.is_empty() {
        // 后缀不在解析器白名单内且暴力兜底也失败 → 不支持的格式，终态不再重试
        if !is_supported_parse_ext(&ext) {
            return Err(format!("{}不支持的文件格式: .{}", UNSUPPORTED_PREFIX, ext));
        }
        return Err(if !parse_result.error.is_empty() {
            parse_result.error
        } else {
            format!("智能解析失败 (耗时 {}ms, 后缀={})", parse_elapsed, ext)
        });
    }
    log::info!(
        "[数据摄入中台] {} 智能解析完成 → raw_text={} 字符 (耗时 {}ms)",
        log_tag,
        parse_result.text.len(),
        parse_elapsed
    );

    // ── 解析后中间状态验证：文件可能在解析过程中被删除 ──
    let file_still_exists: bool = crate::database::get(
        "SELECT 1 FROM file_assets WHERE id=?1",
        &[json!(&file_id)],
    )
    .ok()
    .flatten()
    .is_some();
    if !file_still_exists {
        log::info!("[数据摄入中台] {} 解析完成但文件已被删除，跳过摄入", log_tag);
        return Ok(());
    }

    // ════════════════════════════════════════
    // ② 文本清洗
    // ════════════════════════════════════════
    let clean_start = std::time::Instant::now();
    let clean_text = crate::services::file_parser::clean_extracted_text_with_hint(&parse_result.text, &ext);
    let clean_elapsed = clean_start.elapsed().as_micros();
    let reduction = if parse_result.text.len() > 0 {
        100.0 - (clean_text.len() as f64 / parse_result.text.len() as f64 * 100.0)
    } else {
        0.0
    };

    if clean_text.trim().is_empty() {
        // 非白名单后缀清洗后无有效内容 → 视为不支持，终态不再重试
        if !is_supported_parse_ext(&ext) {
            return Err(format!("{}不支持的文件格式: .{}（清洗后无有效文本）", UNSUPPORTED_PREFIX, ext));
        }
        return Err(format!(
            "清洗后无有效文本 (原始 {} 字符)",
            parse_result.text.len()
        ));
    }
    log::info!(
        "[数据摄入中台] {} 文本清洗完成 → clean_text={} 字符 (压缩 {:.1}%, 耗时 {}μs)",
        log_tag,
        clean_text.len(),
        reduction,
        clean_elapsed
    );

    // ════════════════════════════════════════
    // ③ 写入 source_data
    // ════════════════════════════════════════
    crate::database::file_sync_to_source_data(&file_id, &clean_text);

    // 验证写入是否成功
    let check = crate::database::get(
        "SELECT 1 FROM source_data WHERE source_type='FILE' AND source_id=?",
        &[json!(&file_id)],
    );
    if check.is_ok() && check.unwrap().is_some() {
        log::info!(
            "[数据摄入中台] {} 写入 source_data 完成 ({} 字符)",
            log_tag,
            clean_text.len()
        );
    } else {
        log::info!("[数据摄入中台] {} source_data 写入后验证失败!", log_tag);
    }

    log::info!(
        "[数据摄入中台] {} 处理完成 >>> 清洗后 {} 字符",
        log_tag,
        clean_text.len()
    );
    Ok(())
}

// ═══════════════════════════════════════════════════════════════
// 共享辅助函数
// ═══════════════════════════════════════════════════════════════

/// 从 product_series 自引用树递归解析目录路径（如 "全闪存存储 / Dorado 系列"）
fn resolve_series_path(series_id: &str) -> String {
    if series_id.is_empty() {
        return String::new();
    }
    let mut parts: Vec<String> = Vec::new();
    let mut cur: Option<String> = Some(series_id.to_string());
    while let Some(ref sid) = cur.clone() {
        if let Ok(Some(row)) = crate::database::get(
            "SELECT name, parent_id FROM product_series WHERE id=?1",
            &[json!(sid)],
        ) {
            if let Some(name) = row["name"].as_str().filter(|n| !n.is_empty()) {
                parts.push(name.to_string());
                cur = row["parent_id"].as_str().map(|s| s.to_string());
            } else {
                break;
            }
        } else {
            break;
        }
    }
    parts.reverse();
    parts.join(" / ")
}

// ═══════════════════════════════════════════════════════════════
// 轮询式补漏：处理历史遗留数据
// ═══════════════════════════════════════════════════════════════

/// 摄入笔记：将尚未同步到 source_data 的笔记写入
fn ingest_pending_notes() {
    let rows = match crate::database::all(
        "SELECT n.note_id, n.title, n.content FROM notes n \
         WHERE n.note_id NOT IN (SELECT source_id FROM source_data WHERE source_type='NOTE') \
           AND n.deleted_at IS NULL \
         LIMIT 20",
        &[],
    ) {
        Ok(r) => r,
        Err(e) => {
            log::info!("[数据中台] ⚠ 查询笔记失败: {}", e);
            return;
        }
    };
    if rows.is_empty() {
        return;
    }

    for row in &rows {
        let note_id = row["note_id"].as_str().unwrap_or("").to_string();
        if note_id.is_empty() {
            continue;
        }
        let title = row["title"].as_str().unwrap_or("").to_string();
        let raw_content = row["content"].as_str().unwrap_or("").to_string();
        // 将 TipTap HTML 转换为 Markdown（保留标题/列表/表格等格式）
        let markdown = crate::services::file_parser::html_to_markdown(&raw_content);
        let clean = crate::services::file_parser::sanitize_chars(&markdown);
        if clean.trim().is_empty() {
            continue;
        }

        // 构建Markdown格式
        let mut parts = Vec::new();
        if !title.is_empty() {
            parts.push(format!("# {}", title));
        }
        parts.push(clean.clone());
        let md_content = parts.join("\n\n");

        let _ = crate::database::run(
            "INSERT INTO source_data(id,source_type,source_id,title,content,updated_at) SELECT ?1,'NOTE',?1,?2,?3,datetime('now','localtime') WHERE NOT EXISTS (SELECT 1 FROM source_data WHERE source_type='NOTE' AND source_id=?1)",
            &[json!(&note_id), json!(&title), json!(&md_content)],
        );
        log::info!(
            "[数据中台] 📝 笔记 {} 已摄入 ({} 字符)",
            &note_id[..note_id.len().min(8)],
            md_content.len()
        );
    }
}

/// 摄入产品：将尚未同步到 source_data 的产品写入（名称+指标+规格+描述）
fn ingest_pending_products() {
    let rows = match crate::database::all(
        "SELECT p.id, p.name, p.full_specs, p.description, p.series_id FROM products p \
         WHERE p.id NOT IN (SELECT source_id FROM source_data WHERE source_type='PRODUCT') \
         LIMIT 20",
        &[],
    ) {
        Ok(r) => r,
        Err(e) => {
            log::info!("[数据中台] ⚠ 查询产品失败: {}", e);
            return;
        }
    };
    if rows.is_empty() {
        return;
    }

    for row in &rows {
        let prod_id = row["id"].as_str().unwrap_or("").to_string();
        if prod_id.is_empty() {
            continue;
        }
        let name = row["name"].as_str().unwrap_or("").to_string();
        let full = row["full_specs"].as_str().unwrap_or("").to_string();
        let desc = row["description"].as_str().unwrap_or("").to_string();
        let series_id = row["series_id"].as_str().unwrap_or("").to_string();
        let path = resolve_series_path(&series_id);

        // 构建Markdown格式
        let mut parts = Vec::new();
        if !name.is_empty() {
            parts.push(format!("# {}", name));
        }
        if !desc.is_empty() {
            parts.push(desc);
        }
        let mut specs_lines = Vec::new();
        if !path.is_empty() {
            specs_lines.push(format!("系列：{}", path));
        }
        if !full.is_empty() {
            specs_lines.push(full.to_string());
        }
        if !specs_lines.is_empty() {
            parts.push(format!("## 技术参数\n{}", specs_lines.join("\n")));
        }
        let content = parts.join("\n\n");

        let clean = crate::services::file_parser::sanitize_chars(&content);
        if clean.trim().is_empty() {
            continue;
        }
        let _ = crate::database::run(
            "INSERT INTO source_data(id,source_type,source_id,title,content,updated_at) SELECT ?1,'PRODUCT',?1,?2,?3,datetime('now','localtime') WHERE NOT EXISTS (SELECT 1 FROM source_data WHERE source_type='PRODUCT' AND source_id=?1)",
            &[json!(&prod_id), json!(&name), json!(&clean)],
        );
        log::info!(
            "[数据中台] 📦 产品 {} 已摄入 ({} 字符)",
            &prod_id[..prod_id.len().min(8)],
            clean.len()
        );
    }
}

/// 摄入项目：将尚未同步到 source_data 的项目写入
fn ingest_pending_projects() {
    let rows = match crate::database::all(
        "SELECT p.id, p.name, COALESCE(p.description,'') as description, \
         p.status, p.owner, p.budget, p.bid_time, p.expected_order_date \
         FROM projects p \
         WHERE p.id NOT IN (SELECT source_id FROM source_data WHERE source_type='PROJECT') \
         LIMIT 20",
        &[],
    ) {
        Ok(r) => r,
        Err(e) => {
            log::info!("[数据中台] ⚠ 查询项目失败: {}", e);
            return;
        }
    };
    if rows.is_empty() {
        return;
    }

    for row in &rows {
        let proj_id = row["id"].as_str().unwrap_or("").to_string();
        if proj_id.is_empty() {
            continue;
        }
        let name = row["name"].as_str().unwrap_or("").to_string();
        let desc = row["description"].as_str().unwrap_or("").to_string();

        // 构建Markdown格式
        let mut parts = Vec::new();
        if !name.is_empty() {
            parts.push(format!("# {}", name));
        }
        // 结构化输出业务字段
        let mut meta = Vec::new();
        if let Some(s) = row["status"].as_str().filter(|s| !s.is_empty()) { meta.push(format!("阶段：{}", s)); }
        if let Some(o) = row["owner"].as_str().filter(|o| !o.is_empty()) { meta.push(format!("负责人：{}", o)); }
        if let Some(b) = row["budget"].as_f64().filter(|b| *b > 0.0) { meta.push(format!("金额：¥{}万", b)); }
        if let Some(bt) = row["bid_time"].as_str().filter(|s| !s.is_empty()) { meta.push(format!("投标时间：{}", bt)); }
        if let Some(eo) = row["expected_order_date"].as_str().filter(|s| !s.is_empty()) { meta.push(format!("预计下单：{}", eo)); }
        if !meta.is_empty() { parts.push(meta.join(" | ")); }
        if !desc.is_empty() {
            parts.push(desc);
        }
        // 项目动态：倒序 + 限制 20 条
        if let Ok(dynamics) = crate::database::all(
            "SELECT content, created_at FROM project_dynamics WHERE project_id=?1 AND content != '' ORDER BY created_at DESC LIMIT 20",
            &[json!(&proj_id)],
        ) {
            let dyn_str: Vec<String> = dynamics.iter().filter_map(|d| {
                let c = d["content"].as_str().unwrap_or("");
                let dt = d["created_at"].as_str().unwrap_or("");
                if !c.is_empty() { Some(format!("- {} ({})", c, dt)) } else { None }
            }).collect();
            if !dyn_str.is_empty() {
                parts.push(format!("## 项目动态\n{}", dyn_str.join("\n")));
            }
        }
        let content = parts.join("\n\n");

        let clean = crate::services::file_parser::sanitize_chars(&content);
        if clean.trim().is_empty() {
            continue;
        }
        let _ = crate::database::run(
            "INSERT INTO source_data(id,source_type,source_id,title,content,updated_at) SELECT ?1,'PROJECT',?1,?2,?3,datetime('now','localtime') WHERE NOT EXISTS (SELECT 1 FROM source_data WHERE source_type='PROJECT' AND source_id=?1)",
            &[json!(&proj_id), json!(&name), json!(&clean)],
        );
        log::info!(
            "[数据中台] 📁 项目 {} 已摄入 ({} 字符)",
            &proj_id[..proj_id.len().min(8)],
            clean.len()
        );
    }
}

fn ingest_pending_todos() {
    let rows = match crate::database::all(
        "SELECT t.id, t.title, t.content FROM todos t WHERE t.id NOT IN (SELECT source_id FROM source_data WHERE source_type='TODO') AND t.done=0 LIMIT 20",
        &[],
    ) {
        Ok(r) => r,
        Err(e) => {
            log::info!("[数据中台] ⚠ 查询待办失败: {}", e);
            return;
        }
    };
    if rows.is_empty() {
        return;
    }
    for row in &rows {
        let id = row["id"].as_str().unwrap_or("").to_string();
        if id.is_empty() {
            continue;
        }
        let title = row["title"].as_str().unwrap_or("");
        let raw = row["content"].as_str().unwrap_or("");
        let markdown = crate::services::file_parser::html_to_markdown(&raw);
        let clean = crate::services::file_parser::sanitize_chars(&markdown);
        if clean.trim().is_empty() {
            continue;
        }

        // 构建Markdown格式
        let mut parts = Vec::new();
        if !title.is_empty() {
            parts.push(format!("# {}", title));
        }
        parts.push(clean.clone());
        let md_content = parts.join("\n\n");

        let _ = crate::database::run(
            "INSERT INTO source_data(id,source_type,source_id,title,content,updated_at) SELECT ?1,'TODO',?1,?2,?3,datetime('now','localtime') WHERE NOT EXISTS (SELECT 1 FROM source_data WHERE source_type='TODO' AND source_id=?1)",
            &[json!(&id), json!(title), json!(&md_content)],
        );
        log::info!(
            "[数据中台] 📋 待办 {} 已摄入 ({} 字符)",
            &id[..id.len().min(8)],
            md_content.len()
        );
    }
}

fn ingest_pending_emails() {
    let rows = match crate::database::all(
        "SELECT e.id FROM emails e WHERE e.ai_category='knowledge' AND e.id NOT IN (SELECT source_id FROM source_data WHERE source_type='EMAIL') LIMIT 20",
        &[],
    ) {
        Ok(r) => r,
        Err(e) => {
            log::info!("[数据中台] ⚠ 查询邮件失败: {}", e);
            return;
        }
    };
    if rows.is_empty() {
        return;
    }
    for row in &rows {
        let id = row["id"].as_str().unwrap_or("").to_string();
        if id.is_empty() {
            continue;
        }
        
        // 统一调用 source_doc_sync::extract_email() 获取邮件内容
        let (title, content) = crate::services::source_doc_sync::extract_email(&id);
        if content.is_empty() {
            continue;
        }
        
        // 检查是否已存在
        if crate::database::get(
            "SELECT 1 FROM source_data WHERE source_type='EMAIL' AND source_id=?1",
            &[serde_json::json!(&id)],
        )
        .ok()
        .flatten()
        .is_some()
        {
            continue;
        }
        
        // 写入 source_data（id = source_id）
        crate::database::run(
            "INSERT INTO source_data(id, source_type, source_id, title, content, updated_at) VALUES(?1, 'EMAIL', ?1, ?2, ?3, datetime('now','localtime'))",
            &[serde_json::json!(&id), serde_json::json!(&title), serde_json::json!(&content)],
        ).ok();
        log::info!(
            "[数据中台] 📧 邮件 {} 已摄入 ({} 字符)",
            &id[..id.len().min(8)],
            content.len()
        );
    }
}

// ═══════════════════════════════════════════════════════════════
// 孤儿数据清理（从 cleanup_pipeline.rs 迁移）
// ═══════════════════════════════════════════════════════════════

/// 兜底扫描：清理所有孤儿数据
/// 1. knowledge_chunks 中 source_data 不存在的
/// 2. vector_cache 中 knowledge_chunks 不存在的
/// 3. source_data 中业务对象已不存在的
pub fn cleanup_orphan_source_data() {
    // 调试：检查数据量
    let kc_count: i64 = crate::database::get("SELECT COUNT(*) as cnt FROM knowledge_chunks", &[])
        .ok()
        .flatten()
        .and_then(|r| r["cnt"].as_i64())
        .unwrap_or(0);
    let sd_count: i64 = crate::database::get("SELECT COUNT(*) as cnt FROM source_data", &[])
        .ok()
        .flatten()
        .and_then(|r| r["cnt"].as_i64())
        .unwrap_or(0);
    let vc_count: i64 = crate::database::get("SELECT COUNT(*) as cnt FROM vector_cache", &[])
        .ok()
        .flatten()
        .and_then(|r| r["cnt"].as_i64())
        .unwrap_or(0);
    log::info!(
        "[数据中台] 孤儿扫描前: knowledge_chunks={}, source_data={}, vector_cache={}",
        kc_count, sd_count, vc_count
    );

    // 1. 清理 knowledge_chunks 孤儿（source_data 不存在）
    let orphan_count: i64 = crate::database::get(
        "SELECT COUNT(*) as cnt FROM knowledge_chunks kc WHERE NOT EXISTS (SELECT 1 FROM source_data sd WHERE sd.source_type=kc.source_type AND sd.source_id=kc.source_id)",
        &[],
    )
    .ok()
    .flatten()
    .and_then(|r| r["cnt"].as_i64())
    .unwrap_or(0);
    log::info!(
        "[数据中台] 发现 {} 条孤儿 knowledge_chunks",
        orphan_count
    );

    if orphan_count > 0 {
        // 先清理 FTS5 索引（避免删除时触发器报错）
        let _ = crate::database::run(
            "DELETE FROM kk_chunks_fts WHERE rowid IN (SELECT id FROM knowledge_chunks WHERE NOT EXISTS (SELECT 1 FROM source_data sd WHERE sd.source_type=knowledge_chunks.source_type AND sd.source_id=knowledge_chunks.source_id) AND chunk_type='child')",
            &[],
        );

        // 删除孤儿 knowledge_chunks
        match crate::database::run(
            "DELETE FROM knowledge_chunks WHERE NOT EXISTS (SELECT 1 FROM source_data sd WHERE sd.source_type=knowledge_chunks.source_type AND sd.source_id=knowledge_chunks.source_id)",
            &[],
        ) {
            Ok(r) => log::info!(
                "[数据中台] 已清理 knowledge_chunks: {} 行",
                r.changes
            ),
            Err(e) => log::info!("[数据中台] knowledge_chunks 清理失败: {}", e),
        }
    }

    // 2. 清理 vector_cache 孤儿（knowledge_chunks 不存在）
    match crate::database::run(
        "DELETE FROM vector_cache WHERE chunk_id NOT IN (SELECT id FROM knowledge_chunks)",
        &[],
    ) {
        Ok(r) if r.changes > 0 => log::info!(
            "[数据中台] 孤儿 vector_cache 清理: {} 行",
            r.changes
        ),
        _ => {}
    }

    // 3. 清理 source_data 孤儿（业务对象已不存在）
    for (source_type, table, id_col) in &[
        ("NOTE", "notes", "note_id"),
        ("PRODUCT", "products", "id"),
        ("PROJECT", "projects", "id"),
        ("TODO", "todos", "id"),
        ("FILE", "file_assets", "id"),
    ] {
        // 先清理 FTS5 索引
        let fts_sql = format!(
            "INSERT INTO global_search_fts(global_search_fts, rowid, source_type, source_id, title, content) \
             SELECT 'delete', rowid, source_type, source_id, title, content FROM source_data \
             WHERE source_type='{}' AND source_id NOT IN (SELECT {} FROM {})",
            source_type, id_col, table
        );
        crate::database::run(&fts_sql, &[]).ok();
        // 清理 source_data
        let sql = format!(
            "DELETE FROM source_data WHERE source_type='{}' AND source_id NOT IN (SELECT {} FROM {})",
            source_type, id_col, table
        );
        match crate::database::run(&sql, &[]) {
            Ok(r) if r.changes > 0 => log::info!(
                "[数据中台] 孤儿 source_data 清理 ({}): {} 行",
                source_type, r.changes
            ),
            Err(e) => log::info!(
                "[数据中台] 孤儿 source_data 清理失败 ({}): {}",
                source_type, e
            ),
            _ => {}
        }
    }

    // 4. 清理 business_relations 孤儿（引用的实体已不存在）
    let br_orphan_count: i64 = crate::database::get(
        "SELECT COUNT(*) as cnt FROM business_relations br WHERE \
         (br.source_type='NOTE' AND br.source_id NOT IN (SELECT note_id FROM notes)) OR \
         (br.source_type='PRODUCT' AND br.source_id NOT IN (SELECT id FROM products)) OR \
         (br.source_type='PROJECT' AND br.source_id NOT IN (SELECT id FROM projects)) OR \
         (br.source_type='TODO' AND br.source_id NOT IN (SELECT id FROM todos)) OR \
         (br.source_type='FILE' AND br.source_id NOT IN (SELECT id FROM file_assets)) OR \
         (br.source_type IN ('CHANNEL','VENDOR','CUSTOMER') AND br.source_id NOT IN (SELECT id FROM partners)) OR \
         (br.source_type='CONTACT' AND br.source_id NOT IN (SELECT id FROM partner_contacts)) OR \
         (br.target_type='PROJECT' AND br.target_id NOT IN (SELECT id FROM projects))",
        &[],
    )
    .ok()
    .flatten()
    .and_then(|r| r["cnt"].as_i64())
    .unwrap_or(0);

    if br_orphan_count > 0 {
        log::info!("[数据中台] 发现 {} 条孤儿 business_relations", br_orphan_count);
        // 删除孤儿 business_relations
        match crate::database::run(
            "DELETE FROM business_relations WHERE \
             (source_type='NOTE' AND source_id NOT IN (SELECT note_id FROM notes)) OR \
             (source_type='PRODUCT' AND source_id NOT IN (SELECT id FROM products)) OR \
             (source_type='PROJECT' AND source_id NOT IN (SELECT id FROM projects)) OR \
             (source_type='TODO' AND source_id NOT IN (SELECT id FROM todos)) OR \
             (source_type='FILE' AND source_id NOT IN (SELECT id FROM file_assets)) OR \
             (source_type IN ('CHANNEL','VENDOR','CUSTOMER') AND source_id NOT IN (SELECT id FROM partners)) OR \
             (source_type='CONTACT' AND source_id NOT IN (SELECT id FROM partner_contacts)) OR \
             (target_type='PROJECT' AND target_id NOT IN (SELECT id FROM projects))",
            &[],
        ) {
            Ok(r) => log::info!("[数据中台] 已清理 business_relations 孤儿: {} 行", r.changes),
            Err(e) => log::info!("[数据中台] business_relations 孤儿清理失败: {}", e),
        }
    }
}
