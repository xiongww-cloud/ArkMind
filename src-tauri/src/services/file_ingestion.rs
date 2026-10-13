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
/// 时间预算耗尽的"续跑"信号：退回 pending 且不消耗重试预算（断点续进，直至完成）
pub const PROGRESS_PREFIX: &str = "__PROGRESS__";

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

/// 内容权威判定（双向）：魔数命中→按**真实类型**裁决（zip/rar/7z 等已知容器无解析器=unsupported，
/// 不再因"类型已知"误入真解析失败×3 重试）；魔数未命中→沿用后缀白名单裁决。
/// 返回 Some(展示后缀) = 判 unsupported 终态；None = 真解析失败可重试
fn classify_parse_failure_type(inferred_ext: &Option<String>, ext: &str) -> Option<String> {
    match inferred_ext {
        Some(m) if !is_supported_parse_ext(m) => Some(m.clone()),
        None if !is_supported_parse_ext(ext) => Some(ext.to_string()),
        _ => None,
    }
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

    let mut idle_rounds: u32 = 0;
    let mut last_backfill = std::time::Instant::now();
    // 孤儿资产兜底：初始值回拨一个周期 → 启动后首轮空闲即清历史欠账，之后每 24h 一次
    let mut last_orphan_sweep = std::time::Instant::now() - Duration::from_secs(24 * 3600);

    loop {
        if !INGESTION_RUNNING.load(Ordering::Acquire) {
            log::info!("[数据中台] 收到停止信号，退出轮询");
            break;
        }

        // 1. 优先处理 knowledge_push_queue 中的待处理任务（非阻塞）
        let claimed = process_push_queue_non_blocking();

        // 2. 轮询式补漏（处理历史遗留数据）
        ingest_pending_notes();
        ingest_pending_products();
        ingest_pending_projects();
        ingest_pending_todos();

        // 3. FILE/EMAIL 兜底扫描（每30s）：上传后入队前崩溃 → 有资产无队列记录，永久漏解析的缺口在此收敛。
        //    邮件补漏必须同闸：曾因其每轮（100ms）执行 + 只查 source_data 不管队列在途，
        //    与卡住的 OCR 任务叠加成每 200ms 重入队 12 封的日志风暴与管线空转。
        if last_backfill.elapsed() >= Duration::from_secs(30) {
            last_backfill = std::time::Instant::now();
            backfill_missing_file_sources();
            ingest_pending_emails();
        }

        // 4. 孤儿资产兜底回收（每 24h）：任何同步删除路径的漏网在宽限期后被此处收敛
        if last_orphan_sweep.elapsed() >= Duration::from_secs(24 * 3600) {
            last_orphan_sweep = std::time::Instant::now();
            let (d, a, f) = crate::commands::file_commands::sweep_orphans();
            if d + a + f > 0 {
                log::info!("[数据中台] 周期资产回收：悬垂关系 {} | 孤儿资产 {} | 磁盘游离 {}", d, a, f);
            }
        }

        // 空闲指数退避：无任务时 100ms→200→400→…封顶3s，消除常驻忙轮询（有任务/在途立即恢复 100ms）
        idle_rounds = if claimed == 0 && ACTIVE_TASKS.load(Ordering::Relaxed) == 0 { idle_rounds + 1 } else { 0 };
        let sleep_ms: u64 = if idle_rounds == 0 { 100 } else { (100u64 << idle_rounds.min(5)).min(3000) };

        // 空闲时退避轮询
        std::thread::sleep(Duration::from_millis(sleep_ms));
    }
}

/// FILE 补漏：file_assets 存在、但 source_data 无文本且队列无任何在途/终态记录 → 重新入队解析
/// （含 AI 生成文档——生成物同样入知识库，检索侧由 search 工具按需过滤；存量生成文件由本兜底自动补解析）
/// operation 必须是 'file_upload'（走 process_file_upload 解析磁盘文件正文）——
/// 历史 bug：曾入队 'change'（走 extract_file 只取文件名当 content），写入假 source_data
/// 后反而永久堵死正确解析路径（项目导入的文件因此不进知识库）
fn backfill_missing_file_sources() {
    let missing = match crate::database::all(
        "SELECT f.id FROM file_assets f \
         WHERE f.created_at < datetime('now','localtime','-2 minutes') \
           AND NOT EXISTS (SELECT 1 FROM source_data sd WHERE sd.source_type='FILE' AND sd.source_id=f.id) \
           AND NOT EXISTS (SELECT 1 FROM knowledge_push_queue q \
               WHERE q.source_type='FILE' AND q.source_id=f.id \
                 AND q.status IN ('pending','processing','failed','unsupported')) \
         LIMIT 20",
        &[],
    ) { Ok(r) => r, Err(_) => return };
    if missing.is_empty() { return; }
    for r in &missing {
        let fid = match r["id"].as_str() { Some(s) => s, None => continue };
        let _ = crate::database::run(
            "DELETE FROM knowledge_push_queue WHERE source_type='FILE' AND source_id=?1 AND status='pending'",
            &[json!(fid)],
        );
        let _ = crate::database::run(
            "INSERT INTO knowledge_push_queue(source_type, source_id, operation, status) VALUES('FILE', ?1, 'file_upload', 'pending')",
            &[json!(fid)],
        );
    }
    log::info!("[数据中台] FILE 兜底扫描：补入队 {} 个漏解析文件", missing.len());
}

// ═══════════════════════════════════════════════════════════════
// 核心调度：从 knowledge_push_queue 获取任务（并发处理）
// ═══════════════════════════════════════════════════════════════

/// 从 knowledge_push_queue 表原子领取任务并并发执行（非阻塞）；返回本轮领取数
fn process_push_queue_non_blocking() -> usize {
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
        // 满载不再静默：限频 warn（60s 一条），停摆可被第一时间看见
        use std::sync::atomic::AtomicU64;
        static LAST_FULL_WARN: AtomicU64 = AtomicU64::new(0);
        let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
        let last = LAST_FULL_WARN.load(Ordering::Relaxed);
        if now_ms.saturating_sub(last) >= 60_000 && LAST_FULL_WARN.compare_exchange(last, now_ms, Ordering::Relaxed, Ordering::Relaxed).is_ok() {
            log::warn!("[数据中台] 并发槽位满载({}/ {})，本轮暂停领取；若长期出现请检查在途任务是否卡住", current_active, MAX_CONCURRENT);
        }
        return 0; // 没有可用槽位，跳过本轮
    }

    let available_slots = MAX_CONCURRENT - current_active;
    // 单语句原子领取（UPDATE…RETURNING）：SELECT 与标记分离时，并发入队/重启窗口会让同一行被领取两次
    let tasks = match crate::database::all(
        &format!(
            "UPDATE knowledge_push_queue SET status='processing', updated_at=datetime('now','localtime') \
             WHERE id IN (SELECT id FROM knowledge_push_queue \
                 WHERE status='pending' AND (debounce_until IS NULL OR debounce_until <= datetime('now','localtime')) \
                 ORDER BY created_at ASC LIMIT {}) \
               AND status='pending' \
             RETURNING id, source_type, source_id, operation, retry_count",
            available_slots
        ),
        &[],
    ) {
        Ok(r) => r,
        Err(e) => {
            log::info!("[数据中台] ⚠ 领取 knowledge_push_queue 任务失败: {}", e);
            return 0;
        }
    };

    if tasks.is_empty() {
        return 0;
    }

    let task_count = tasks.len();
    log::info!("[数据中台] 本次领取 {} 条待推送变更（当前活跃 {}/{})", task_count, current_active, MAX_CONCURRENT);

    // 创建线程处理所有任务（不等待完成）
    for (idx, task) in tasks.into_iter().enumerate() {
        // 检查并发槽位（槽位耗尽则把剩余已领取任务退回 pending）
        if ACTIVE_TASKS.load(Ordering::Relaxed) >= MAX_CONCURRENT {
            let id = task["id"].clone();
            let _ = crate::database::run(
                "UPDATE knowledge_push_queue SET status='pending', updated_at=datetime('now','localtime') WHERE id=?1 AND status='processing'",
                &[id],
            );
            log::info!("[数据中台] 并发槽位已满，剩余任务退回 pending");
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
    task_count
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
            match decide_failure_action(&e, retry_count) {
                // 不支持的文件格式：标记 unsupported 终态，不参与重试，冷启动也不重置
                FailureAction::Unsupported => {
                    let _ = crate::database::run(
                        "UPDATE knowledge_push_queue SET status='unsupported', retry_count=?1, last_error=?2, updated_at=datetime('now','localtime') WHERE id=?3",
                        &[json!(retry_count + 1), json!(&e), json!(id)],
                    );
                    log::info!("[数据中台] ✗ {}/{} {}", source_type, source_id, e.trim_start_matches(UNSUPPORTED_PREFIX));
                }
                // 不可重试错误（如视觉模型解析失败）：直接标记 failed，不做任务级重试
                FailureAction::NoRetry => {
                    let _ = crate::database::run(
                        "UPDATE knowledge_push_queue SET status='failed', retry_count=?1, last_error=?2, updated_at=datetime('now','localtime') WHERE id=?3",
                        &[json!(retry_count + 1), json!(&e), json!(id)],
                    );
                    log::info!("[数据中台] ✗ {}/{} 不可重试失败: {}", source_type, source_id, e);
                }
                // 预算耗尽续跑：退回 pending、retry 清零、30s debounce（OCR 结果有缓存，每轮有净进展）
                FailureAction::Progress => {
                    let _ = crate::database::run(
                        "UPDATE knowledge_push_queue SET status='pending', retry_count=0, debounce_until=datetime('now','localtime','+30 seconds'), last_error='', updated_at=datetime('now','localtime') WHERE id=?1",
                        &[json!(id)],
                    );
                    log::info!("[数据中台] ⏳ {}/{} 续跑: {}", source_type, source_id, e.trim_start_matches(PROGRESS_PREFIX));
                }
                // 超过最大重试：标记 failed
                FailureAction::GiveUp => {
                    let _ = crate::database::run(
                        "UPDATE knowledge_push_queue SET status='failed', retry_count=?1, last_error=?2, updated_at=datetime('now','localtime') WHERE id=?3",
                        &[json!(retry_count + 1), json!(&e), json!(id)],
                    );
                    log::info!("[数据中台] ✗ {}/{} 超过最大重试次数: {}", source_type, source_id, e);
                }
                // 普通失败：退回 pending，5s 防抖，消耗一次重试预算
                FailureAction::Retry => {
                    let _ = crate::database::run(
                        "UPDATE knowledge_push_queue SET status='pending', debounce_until=datetime('now','localtime','+5 seconds'), retry_count=?1, last_error=?2, updated_at=datetime('now','localtime') WHERE id=?3",
                        &[json!(retry_count + 1), json!(&e), json!(id)],
                    );
                    log::info!("[数据中台] ⚠ {}/{} 重试 {}/3: {}", source_type, source_id, retry_count + 1, e);
                }
            }
        }
    }
}

/// 失败处置动作（纯决策，便于测试）。前缀优先级高于重试计数：
/// __PROGRESS__ 无论已重试几次都续跑且不消耗重试预算（它不是错误，是断点）。
enum FailureAction { Unsupported, NoRetry, Progress, GiveUp, Retry }

fn decide_failure_action(err: &str, retry_count: i64) -> FailureAction {
    if err.starts_with(UNSUPPORTED_PREFIX) { FailureAction::Unsupported }
    else if err.starts_with(NON_RETRYABLE_PREFIX) { FailureAction::NoRetry }
    else if err.starts_with(PROGRESS_PREFIX) { FailureAction::Progress }
    else if retry_count + 1 >= 3 { FailureAction::GiveUp }
    else { FailureAction::Retry }
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
// PDF 流式解析：一次 load 批量渲染 + 文件内 3 路并发 OCR，支持断点续传
// 全局视觉在途请求由 file_parser::VISION_SEMAPHORE 跨文件封顶（防 429）
// ═══════════════════════════════════════════════════════════════

/// 单文件内 OCR 并发路数（视觉 API 等待为主，渲染已前置为一次 load 批量）
const PDF_PAGE_OCR_CONCURRENCY: usize = 3;

/// 乱序完成的页结果按页号组装为全文（纯函数，便于测试）
pub(crate) fn assemble_pdf_pages(parts: &mut [Option<String>]) -> String {
    let mut out = String::new();
    for (i, part) in parts.iter().enumerate() {
        if let Some(text) = part {
            if !out.is_empty() {
                out.push_str(&format!("\n\n--- 第 {} 页 ---\n\n", i + 1));
            }
            out.push_str(text);
        }
    }
    out
}

fn process_pdf_streaming(row: &serde_json::Value, source_type: &str, source_id: &str) -> Result<(), String> {
    use crate::services::file_parser::{get_pdf_page_count, render_pdf_pages, ocr_image_async, has_real_text, OcrOutcome, PageRender};
    use futures::StreamExt;
    use std::sync::{Arc, Mutex};

    let file_id = row["id"].as_str().unwrap_or("").to_string();
    if file_id.is_empty() {
        return Err("文件ID为空".to_string());
    }

    // 日志标识：{source_type}/{source_id前8位}
    let log_tag = format!("{}/{}", source_type, crate::preview(source_id, 8));

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

    // ── 阶段 1：缓存命中页直接进结果（跳过渲染与 API，断点续传）──
    let mut full_text_parts: Vec<Option<String>> = vec![None; total_pages as usize];
    let mut targets: Vec<u16> = Vec::new();
    let mut done_count: usize = 0;
    for p in 0..total_pages {
        let cached_text = crate::database::get(
            "SELECT ocr_text FROM file_parse_cache WHERE file_id=?1 AND page_num=?2",
            &[json!(&file_id), json!(p)],
        ).ok().flatten().and_then(|r| r["ocr_text"].as_str().map(|s| s.to_string()));
        if let Some(text) = cached_text {
            if has_real_text(&text) {
                full_text_parts[p as usize] = Some(text);
            }
            done_count += 1;
            log::info!("[数据中台] {} 第 {}/{} 页: 使用缓存", log_tag, p + 1, total_pages);
            continue;
        }
        targets.push(p);
    }

    if !std::path::Path::new(&path).exists() {
        log::info!("[数据中台] {} 文件已被删除，丢弃已解析部分", log_tag);
        return Err("文件已被删除".into());
    }

    // ── 阶段 2：批量渲染（文档一次 load）+ 近空白页本地预检（不发 API）──
    let rendered = match render_pdf_pages(&path, &targets) {
        Some(v) => v,
        None => {
            log::info!("[数据中台] {} PDF 文档加载失败", log_tag);
            return Err(format!("{}{} PDF 文档加载失败", NON_RETRYABLE_PREFIX, log_tag));
        }
    };
    let mut ocr_jobs: Vec<(u16, String)> = Vec::new();
    for (p, r) in rendered {
        match r {
            PageRender::Failed => {
                log::info!("[数据中台] {} 第 {}/{} 页 OCR 失败: 渲染失败", log_tag, p + 1, total_pages);
                return Err(format!("{}{} 第 {}/{} 页渲染失败", NON_RETRYABLE_PREFIX, log_tag, p + 1, total_pages));
            }
            PageRender::Blank => {
                done_count += 1;
                log::info!("[数据中台] {} 第 {}/{} 页: 空白页预检命中，跳过", log_tag, p + 1, total_pages);
            }
            PageRender::Image(uri) => ocr_jobs.push((p, uri)),
        }
    }

    // ── 阶段 3：OCR 并发池（文件内 3 路；全局闸 10 在 ocr_with_vision_api 内部）──
    if !ocr_jobs.is_empty() {
        let parts = Arc::new(Mutex::new(full_text_parts));
        let done = Arc::new(AtomicUsize::new(done_count));
        let abort = Arc::new(AtomicBool::new(false));
        let first_error = Arc::new(Mutex::new(None::<String>));

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("tokio 运行时创建失败: {}", e))?;

        let parts_pool = parts.clone();
        let done_pool = done.clone();
        let abort_pool = abort.clone();
        let fe_pool = first_error.clone();
        let file_id_pool = file_id.clone();
        let path_pool = path.clone();
        let tag_pool = log_tag.clone();
        let sid_pool = source_id.to_string();
        let styp_pool = source_type.to_string();

        rt.block_on(async move {
            futures::stream::iter(ocr_jobs)
                .map(|(p, uri)| {
                    let (parts, done, abort, fe, fid, fpath, tag, sid, styp) = (
                        parts_pool.clone(), done_pool.clone(), abort_pool.clone(), fe_pool.clone(),
                        file_id_pool.clone(), path_pool.clone(), tag_pool.clone(), sid_pool.clone(), styp_pool.clone(),
                    );
                    async move {
                        let t_page = std::time::Instant::now();
                        // 已有页失败/文件被删 → 未派发页直接跳过（在途页继续跑完落缓存）
                        if abort.load(Ordering::Relaxed) {
                            return;
                        }
                        if !std::path::Path::new(&fpath).exists() {
                            abort.store(true, Ordering::Relaxed);
                            let mut e = fe.lock().unwrap();
                            if e.is_none() { *e = Some("文件已被删除".to_string()); }
                            return;
                        }
                        match ocr_image_async(&uri).await {
                            OcrOutcome::Failed(err_msg) => {
                                // 视觉模型解析失败（已重试 3 次）→ 中止剩余页；已落缓存保留（重试时断点续传）
                                log::info!("[数据中台] {} 第 {}/{} 页 OCR 失败: {}", tag, p + 1, total_pages, err_msg);
                                abort.store(true, Ordering::Relaxed);
                                let mut e = fe.lock().unwrap();
                                if e.is_none() {
                                    *e = Some(format!("{}{} 第 {}/{} 页视觉模型解析失败: {}", NON_RETRYABLE_PREFIX, tag, p + 1, total_pages, err_msg));
                                }
                            }
                            OcrOutcome::Blank => {
                                log::info!("[数据中台] {} 第 {}/{} 页: 空白页，跳过", tag, p + 1, total_pages);
                                done.fetch_add(1, Ordering::SeqCst);
                            }
                            OcrOutcome::Ok(ocr_text) => {
                                // 仅当有真实文本时写入缓存（修复缓存投毒）
                                if has_real_text(&ocr_text) {
                                    let _ = crate::database::run(
                                        "INSERT OR REPLACE INTO file_parse_cache(file_id, page_num, ocr_text) VALUES(?1, ?2, ?3)",
                                        &[json!(&fid), json!(p), json!(ocr_text.clone())],
                                    );
                                    parts.lock().unwrap()[p as usize] = Some(ocr_text);
                                }
                                let n = done.fetch_add(1, Ordering::SeqCst) + 1;
                                // 更新进度（已完成页数；心跳刷新 updated_at 防自愈误判卡死）
                                let _ = crate::database::run(
                                    "UPDATE knowledge_push_queue SET progress_page=?1, updated_at=datetime('now','localtime') WHERE source_id=?2 AND source_type=?3",
                                    &[json!(n), json!(&sid), json!(&styp)],
                                );
                                log::info!("[数据中台] {} 第 {}/{} 页: OCR 完成 (整页耗时 {}ms, 进度 {}/{})", tag, p + 1, total_pages, t_page.elapsed().as_millis(), n, total_pages);
                            }
                        }
                    }
                })
                .buffer_unordered(PDF_PAGE_OCR_CONCURRENCY)
                .for_each(|_| async {})
                .await;
        });

        full_text_parts = match Arc::try_unwrap(parts) {
            Ok(v) => v.into_inner().unwrap_or_default(),
            Err(arc) => arc.lock().unwrap().clone(),
        };

        // 文件在解析过程中被删除 → 丢弃已解析部分，不写入 source_data
        // （返回 Err 后，process_task_with_retry 会检查 task_target_exists 并删除无效任务）
        if !std::path::Path::new(&path).exists() {
            log::info!("[数据中台] {} 文件已被删除，丢弃已解析部分", log_tag);
            return Err("文件已被删除".into());
        }

        // 存在失败页 → 在途页已全部收尾，整文件标记不可重试失败
        if abort.load(Ordering::Relaxed) {
            let err = first_error.lock().unwrap().take()
                .unwrap_or_else(|| format!("{}{} 视觉模型解析失败", NON_RETRYABLE_PREFIX, log_tag));
            return Err(err);
        }
    }

    // 按页号组装 full_text（并发乱序完成 → 此处恢复页序）
    let full_text = assemble_pdf_pages(&mut full_text_parts);

    if full_text.is_empty() {
        return Err("PDF 解析结果为空".to_string());
    }

    // 文本清洗
    let clean_text = crate::services::file_parser::clean_extracted_text_with_hint(&full_text, "pdf");
    if clean_text.trim().is_empty() {
        return Err("清洗后无有效文本".to_string());
    }

    // 写入 source_data（失败则任务重试，不吞错）
    crate::database::file_sync_to_source_data(&file_id, &clean_text)?;

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
    let log_tag = format!("{}/{}", source_type, crate::preview(source_id, 8));

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

    // 写入 source_data（失败则任务重试，不吞错）
    crate::database::file_sync_to_source_data(file_id, &clean_text)?;

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
    let log_tag = format!("{}/{}", source_type, crate::preview(source_id, 8));

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
        // ★ 内容权威判定：魔数命中按真实类型裁决（zip/rar 等容器=unsupported 终态）；
        //   "已知可解析类型仍报错"才是真解析错误（含NON_RETRYABLE前缀），可重试
        if let Some(shown) = classify_parse_failure_type(&parse_result.inferred_ext, &ext) {
            return Err(format!("{}不支持的文件格式: .{}", UNSUPPORTED_PREFIX, shown));
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
    // ★ 清洗提示优先用魔数识别的真实类型
    let effective_ext = parse_result.inferred_ext.clone().unwrap_or_else(|| ext.clone());
    let clean_text = crate::services::file_parser::clean_extracted_text_with_hint(&parse_result.text, &effective_ext);
    let clean_elapsed = clean_start.elapsed().as_micros();
    let reduction = if parse_result.text.len() > 0 {
        100.0 - (clean_text.len() as f64 / parse_result.text.len() as f64 * 100.0)
    } else {
        0.0
    };

    if clean_text.trim().is_empty() {
        // 魔数/后缀判定为无解析器格式（同 parse 出口口径），清洗后无有效文本 → unsupported 终态
        if let Some(shown) = classify_parse_failure_type(&parse_result.inferred_ext, &ext) {
            return Err(format!("{}不支持的文件格式: .{}（清洗后无有效文本）", UNSUPPORTED_PREFIX, shown));
        }
        return Err(format!(
            "清洗后无有效文本 (原始 {} 字符)",
            parse_result.text.len()
        ));
    }
    // ★ 魔数识别出的真实类型与库中后缀不符 → 自愈 file_assets.file_ext（内容权威）
    // ★ 例外：泛型 zip 不得覆盖 OOXML 特异后缀——docx/xlsx/pptx 本质是 zip 容器（PK 魔数），
    //   降级会让预览打开资源管理器（ARK-xxx.docx.zip）、下载变 .zip（refine 漏网时的双保险）
    if let Some(ie) = &parse_result.inferred_ext {
        let ooxml_demote = *ie == "zip" && matches!(ext.as_str(), "docx" | "xlsx" | "pptx");
        if !ie.is_empty() && *ie != ext && !ooxml_demote {
            let _ = crate::database::run(
                "UPDATE file_assets SET file_ext=?1 WHERE id=?2",
                &[json!(ie), json!(&file_id)],
            );
            log::info!("[数据摄入中台] {} file_ext 自愈: .{} → .{}", log_tag, ext, ie);
        }
    }
    log::info!(
        "[数据摄入中台] {} 文本清洗完成 → clean_text={} 字符 (压缩 {:.1}%, 耗时 {}μs)",
        log_tag,
        clean_text.len(),
        reduction,
        clean_elapsed
    );

    // ════════════════════════════════════════
    // ③ 写入 source_data（失败则任务重试，不吞错）
    // ════════════════════════════════════════
    crate::database::file_sync_to_source_data(&file_id, &clean_text)?;

    // 验证写入是否成功（失败上抛进重试）
    let check = crate::database::get(
        "SELECT 1 FROM source_data WHERE source_type='FILE' AND source_id=?",
        &[json!(&file_id)],
    );
    match check {
        Ok(Some(_)) => {
            log::info!(
                "[数据摄入中台] {} 写入 source_data 完成 ({} 字符)",
                log_tag,
                clean_text.len()
            );
        }
        Ok(None) => {
            log::warn!("[数据摄入中台] {} source_data 写入后验证失败，任务重试!", log_tag);
            return Err("source_data 写入后验证失败".to_string());
        }
        Err(e) => {
            log::warn!("[数据摄入中台] {} source_data 验证查询失败: {}", log_tag, e);
            return Err(format!("source_data 验证查询失败: {}", e));
        }
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
            crate::preview(note_id, 8),
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
            crate::preview(prod_id, 8),
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
            crate::preview(proj_id, 8),
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
            crate::preview(id, 8),
            md_content.len()
        );
    }
}

/// 兜底补漏：知识化但从未入过 source 的邮件（notify 丢失/崩溃窗口）→ 重新入队走统一队列路径。
/// 不再直接 extract+INSERT：避免绕过队列的清洗/幂等 upsert，且内嵌图 OCR（extract_email 现含）
/// 属重 IO，绝不能跑在数据中台主循环里。
/// 邮件补漏查询：知识化、从未入过 source、且队列无任何在途/终态记录。
/// 在途/终态排除（对齐 FILE 补漏范式）是关键：曾因"只查 source_data"叠加卡住的
/// OCR 任务，每 200ms 把同一批 12 封邮件反复入队成风暴（5042 行日志/17747 次入队）。
const EMAIL_BACKFILL_QUERY: &str = "SELECT e.id FROM emails e \
    WHERE e.ai_category='knowledge' AND e.is_deleted=0 \
    AND NOT EXISTS (SELECT 1 FROM source_data sd WHERE sd.source_type='EMAIL' AND sd.source_id=e.id) \
    AND NOT EXISTS (SELECT 1 FROM knowledge_push_queue q WHERE q.source_type='EMAIL' AND q.source_id=e.id \
                    AND q.status IN ('pending','processing','failed','unsupported')) \
    LIMIT 20";

fn ingest_pending_emails() {
    let rows = match crate::database::all(EMAIL_BACKFILL_QUERY, &[]) {
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
        let id = row["id"].as_str().unwrap_or("");
        if id.is_empty() {
            continue;
        }
        crate::services::knowledge_lifecycle::notify_changed("EMAIL", id);
    }
    log::info!("[数据中台] 📧 邮件补漏重新入队 {} 封（原直插路径已废弃，统一走队列）", rows.len());
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

    // 5. 清理 materialize_retry 孤儿（source_data 已不存在的债务条目）
    match crate::database::run(
        "DELETE FROM materialize_retry WHERE NOT EXISTS (SELECT 1 FROM source_data sd WHERE sd.source_type=materialize_retry.source_type AND sd.source_id=materialize_retry.source_id)",
        &[],
    ) {
        Ok(r) if r.changes > 0 => log::info!(
            "[数据中台] 孤儿 materialize_retry 清理: {} 行",
            r.changes
        ),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 双向内容权威裁决：魔数命中按真实类型判（zip/rar 容器=unsupported；docx/pdf 已知类型仍失败=真解析错误）；
    /// 魔数未命中沿用后缀白名单。堵住"改名压缩包→误判 failed×3 重试"与"纯 zip 判失败"两类错位
    #[test]
    fn classify_parse_failure_prefers_magic_over_suffix() {
        assert_eq!(classify_parse_failure_type(&Some("zip".into()), "docx").as_deref(), Some("zip"));
        assert_eq!(classify_parse_failure_type(&Some("rar".into()), "pdf").as_deref(), Some("rar"), "rar 伪装 pdf 后缀仍按容器判 unsupported");
        assert_eq!(classify_parse_failure_type(&Some("docx".into()), "zip"), None, "坏 docx=真解析失败可重试");
        assert_eq!(classify_parse_failure_type(&Some("pdf".into()), "bin"), None);
        assert_eq!(classify_parse_failure_type(&None, "docx"), None);
        assert_eq!(classify_parse_failure_type(&None, "exe").as_deref(), Some("exe"));
    }

    #[test]
    fn assemble_keeps_page_order_regardless_of_completion_order() {
        // 并发池乱序写入 → 组装必须按页号恢复顺序，空页（None）跳过且分隔行用真实页号
        let mut parts: Vec<Option<String>> = vec![None; 4];
        parts[3] = Some("第四页内容".to_string());
        parts[1] = Some("第二页内容".to_string());
        let out = assemble_pdf_pages(&mut parts);
        assert_eq!(out, "第二页内容\n\n--- 第 4 页 ---\n\n第四页内容");
    }

    #[test]
    fn assemble_empty_all_returns_empty() {
        let mut parts: Vec<Option<String>> = vec![None; 3];
        assert_eq!(assemble_pdf_pages(&mut parts), "");
    }

    #[test]
    fn failure_action_prefers_progress_over_retry_cap() {
        // 续跑不是错误：即便重试计数已顶格也必须走 Progress（且队列侧清零 retry）
        assert!(matches!(decide_failure_action("__PROGRESS__OCR 未完成", 7), FailureAction::Progress));
        assert!(matches!(decide_failure_action("__UNSUPPORTED__格式", 0), FailureAction::Unsupported));
        assert!(matches!(decide_failure_action("__NO_RETRY__视觉失败", 0), FailureAction::NoRetry));
        assert!(matches!(decide_failure_action("网络抖动", 1), FailureAction::Retry));
        assert!(matches!(decide_failure_action("网络抖动", 2), FailureAction::GiveUp));
    }

    #[test]
    fn email_backfill_excludes_inflight_and_terminal() {
        let (_dir, _g) = crate::database::db_core::test_util::setup_test_db();
        crate::database::with_db(|c| c.execute_batch(
            "CREATE TABLE emails(id TEXT PRIMARY KEY, ai_category TEXT, is_deleted INTEGER DEFAULT 0);
             CREATE TABLE source_data(source_type TEXT, source_id TEXT);
             CREATE TABLE knowledge_push_queue(source_type TEXT, source_id TEXT, status TEXT);
             INSERT INTO emails VALUES('e1','knowledge',0),('e2','knowledge',0),('e3','knowledge',0),('e4','knowledge',0),('e5','knowledge',0),('e6','ignore',0);
             INSERT INTO knowledge_push_queue VALUES('EMAIL','e2','pending'),('EMAIL','e3','failed'),('EMAIL','e5','unsupported'),('EMAIL','e6','pending');
             INSERT INTO source_data VALUES('EMAIL','e4');"
        )).unwrap();
        let rows = crate::database::all(EMAIL_BACKFILL_QUERY, &[]).unwrap();
        let ids: Vec<&str> = rows.iter().map(|r| r["id"].as_str().unwrap_or("")).collect();
        // 只有"知识化+无 source+队列无任何记录"的 e1 会被补；在途 e2/终态 e3·e5/已入库 e4/非知识 e6 全排除
        assert_eq!(ids, vec!["e1"]);
    }
}
