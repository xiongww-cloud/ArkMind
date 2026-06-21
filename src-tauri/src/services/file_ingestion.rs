//! 数据摄入中台 (Data Ingestion Mid-Platform)
//!
//! 统一摄入所有业务实体到 source_data 表，轮转顺序：
//!   ① file   → PENDING文件：解析→清洗→落盘→广播
//!   ② note   → 未摄入笔记：去HTML→清洗→落盘
//!   ③ product→ 未摄入产品：名称+规格+描述→清洗→落盘
//!   ④ project→ 未摄入项目：名称+描述→清洗→落盘
//!
//! 后续物化/打标/向量化由知识管线和标签管线各自独立调度。
//!
//! 日志前缀统一为 "[数据摄入中台]"。

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::Emitter;

static INGESTION_RUNNING: AtomicBool = AtomicBool::new(false);

pub fn start(app_handle: tauri::AppHandle) {
    if INGESTION_RUNNING.swap(true, Ordering::AcqRel) {
        println!("[数据摄入中台] 已在运行中，忽略重复启动");
        return;
    }
    println!("╔══════════════════════════════════════════════╗");
    println!("║        数据摄入中台 v2 启动完成              ║");
    println!("║  智能路由: 魔数嗅探 + 后缀 + 轮询兜底        ║");
    println!("║  因果时序: 解析→清洗→落盘→分发→广播         ║");
    println!("╚══════════════════════════════════════════════╝");

    std::thread::spawn(move || {
        ingestion_loop(app_handle);
    });
}

pub fn is_running() -> bool {
    INGESTION_RUNNING.load(Ordering::Acquire)
}

fn ingestion_loop(app: tauri::AppHandle) {
    let mut loop_tick: u64 = 0;

    loop {
        loop_tick = loop_tick.wrapping_add(1);
        if !INGESTION_RUNNING.load(Ordering::Acquire) {
            println!("[数据摄入中台] 收到停止信号，退出轮询");
            break;
        }

        // 每轮只处理一类实体，轮转顺序：file → note → product → project
        match loop_tick % 4 {
            0 => ingest_pending_files(&app),
            1 => ingest_pending_notes(),
            2 => ingest_pending_products(),
            3 => ingest_pending_projects(),
            _ => {}
        }

        // 空闲时每秒检查一次
        std::thread::sleep(Duration::from_secs(1));
    }
}

// ═══════════════════════════════════════════════════════════════
// Shared helpers
// ═══════════════════════════════════════════════════════════════

/// 从 product_series 自引用树递归解析目录路径（如 "全闪存存储 / Dorado 系列"）
fn resolve_series_path(series_id: &str) -> String {
    if series_id.is_empty() { return String::new(); }
    let mut parts: Vec<String> = Vec::new();
    let mut cur: Option<String> = Some(series_id.to_string());
    while let Some(ref sid) = cur.clone() {
        if let Ok(Some(row)) = crate::database::get(
            "SELECT name, parent_id FROM product_series WHERE id=?1",
            &[serde_json::json!(sid)],
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
// File ingestion
// ═══════════════════════════════════════════════════════════════

fn ingest_pending_files(app: &tauri::AppHandle) {
    // 先恢复上次崩溃残留在 PROCESSING 状态的僵尸文件
    let _ = crate::database::run(
        "UPDATE file_assets SET parse_status='PENDING' WHERE parse_status='PROCESSING'",
        &[],
    );

    let rows = match crate::database::all(
        "SELECT fa.id, fa.file_ext, fa.file_size, fa.content_hash \
         FROM file_assets fa \
         WHERE fa.parse_status = 'PENDING' \
         ORDER BY fa.created_at ASC LIMIT 5",
        &[],
    ) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[数据摄入中台] ⚠ 查询 PENDING 队列失败: {}", e);
            return;
        }
    };

    if rows.is_empty() { return; }

    let active_files: Vec<String> = rows.iter()
        .filter_map(|r| r["id"].as_str().map(|s| s.to_string()))
        .collect();
    println!("[数据摄入中台] 📥 本次调度 {} 个待处理文件: {:?}", rows.len(), active_files);

    for row in &rows {
        let file_id = row["id"].as_str().unwrap_or("").to_string();
        if file_id.is_empty() { continue; }

        // ── 异常兜底：单个文件处理 panic 不影响其他文件 ──
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            process_single_file(app, row)
        }));
        match result {
            Ok(_) => {}
            Err(e) => {
                let msg = if let Some(s) = e.downcast_ref::<&str>() { s.to_string() }
                          else if let Some(s) = e.downcast_ref::<String>() { s.clone() }
                          else { "未知错误".to_string() };
                eprintln!("[数据摄入中台] ✗ {} 处理崩溃: {}", file_id, msg);
                let _ = crate::database::run(
                    "UPDATE file_assets SET parse_status='FAILED' WHERE id=?",
                    &[serde_json::json!(&file_id)],
                );
                let _ = app.emit("file:parse-status", serde_json::json!({"fileId": &file_id, "status": "failed", "error": msg}));
            }
        }
    }
}

fn process_single_file(app: &tauri::AppHandle, row: &serde_json::Value) {
    let file_id = row["id"].as_str().unwrap_or("").to_string();
    if file_id.is_empty() { return; }

    let file_size = row["file_size"].as_i64().unwrap_or(0);
    let hash = row["content_hash"].as_str().unwrap_or("");
    let path = if hash.is_empty() { String::new() } else { crate::commands::file_commands::flat_disk_path(hash) };
    let ext = row["file_ext"].as_str().unwrap_or("").to_string();
    let size_fmt = if file_size >= 1048576 { format!("{:.1}MB", file_size as f64 / 1048576.0) } else { format!("{}KB", file_size / 1024) };

    println!("[数据摄入中台] ═══ 开始处理 {} ({}, {}) ═══", file_id, ext, size_fmt);

    // ── 设置 PROCESSING ──
    let _ = crate::database::run(
        "UPDATE file_assets SET parse_status='PROCESSING' WHERE id=?",
        &[serde_json::json!(&file_id)],
    );
    let _ = app.emit("file:parse-status", serde_json::json!({"fileId": &file_id, "status": "processing"}));
    println!("[数据摄入中台] ① 状态 → PROCESSING");

    // ── 文件可用性检查 ──
    if path.is_empty() || !std::path::Path::new(&path).exists() {
        println!("[数据摄入中台] ✗ {} 文件缺失: {}", file_id, path);
        let _ = crate::database::run(
            "UPDATE file_assets SET parse_status='FAILED' WHERE id=?",
            &[serde_json::json!(&file_id)],
        );
        let _ = app.emit("file:parse-status", serde_json::json!({"fileId": &file_id, "status": "failed", "error": "文件缺失"}));
        return;
    }

    // ════════════════════════════════════════
    // ① 智能解析
    // ════════════════════════════════════════
    let parse_start = std::time::Instant::now();
    let parse_result = crate::services::file_parser::parse_file_smart(&path, &ext);
    let parse_elapsed = parse_start.elapsed().as_millis();

    if !parse_result.success || parse_result.text.is_empty() {
        println!("[数据摄入中台] ✗ {} 智能解析失败 (耗时 {}ms, 后缀={})", file_id, parse_elapsed, ext);
        let _ = crate::database::run(
            "UPDATE file_assets SET parse_status='FAILED' WHERE id=?",
            &[serde_json::json!(&file_id)],
        );
        let _ = app.emit("file:parse-status", serde_json::json!({"fileId": &file_id, "status": "failed", "error": "解析失败"}));
        return;
    }
    println!("[数据摄入中台] ① 智能解析完成 → raw_text={} 字符 (耗时 {}ms)", parse_result.text.len(), parse_elapsed);

    // ── 解析后中间状态验证：文件可能在解析过程中被删除 ──
    let file_still_exists: bool = crate::database::get("SELECT 1 FROM file_assets WHERE id=?1", &[serde_json::json!(&file_id)]).ok().flatten().is_some();
    if !file_still_exists {
        println!("[数据摄入中台] ✗ {} 解析完成但文件已被删除，跳过摄入", file_id);
        return;
    }

    // ════════════════════════════════════════
    // ② 文本清洗
    // ════════════════════════════════════════
    let clean_start = std::time::Instant::now();
    let clean_text = crate::services::file_parser::clean_extracted_text_with_hint(&parse_result.text, &ext);
    let clean_elapsed = clean_start.elapsed().as_micros();
    let reduction = if parse_result.text.len() > 0 {
        100.0 - (clean_text.len() as f64 / parse_result.text.len() as f64 * 100.0)
    } else { 0.0 };

    if clean_text.trim().is_empty() {
        println!("[数据摄入中台] ✗ {} 清洗后无有效文本 (原始 {} 字符)", file_id, parse_result.text.len());
        let _ = crate::database::run(
            "UPDATE file_assets SET parse_status='FAILED' WHERE id=?",
            &[serde_json::json!(&file_id)],
        );
        let _ = app.emit("file:parse-status", serde_json::json!({"fileId": &file_id, "status": "failed", "error": "清洗后无文本"}));
        return;
    }
    println!("[数据摄入中台] ② 文本清洗完成 → clean_text={} 字符 (压缩 {:.1}%, 耗时 {}μs)", clean_text.len(), reduction, clean_elapsed);

    // ════════════════════════════════════════
    // ③ 中台落盘：状态标记 + 写入 source_data
    // ════════════════════════════════════════
    let save_result = crate::database::run(
        "UPDATE file_assets SET parse_status='COMPLETED', updated_at=datetime('now', 'localtime') WHERE id=?",
        &[serde_json::json!(&file_id)],
    );
    if save_result.is_err() {
        eprintln!("[数据摄入中台] ✗ {} 状态写入失败: {:?}", file_id, save_result.err());
        let _ = app.emit("file:parse-status", serde_json::json!({"fileId": &file_id, "status": "failed", "error": "数据库写入失败"}));
        return;
    }
    // 写入统一源数据表
    crate::database::file_sync_to_source_data(&file_id, &clean_text);
    // 验证写入是否成功
    let check = crate::database::get(
        "SELECT 1 FROM source_data WHERE source_type='FILE' AND source_id=?",
        &[serde_json::json!(&file_id)],
    );
    if check.is_ok() && check.unwrap().is_some() {
        println!("[数据摄入中台] ③ 中台落盘完成 → parse_status='COMPLETED', source_data 已同步 ({} 字符)", clean_text.len());
    } else {
        eprintln!("[数据摄入中台] ⚠ source_data 写入后验证失败!");
    }

    // ════════════════════════════════════════
    // ④ 状态广播
    // ════════════════════════════════════════
    let _ = app.emit("file:parse-status", serde_json::json!({"fileId": &file_id, "status": "completed"}));
    println!("[数据摄入中台] ④ 状态广播 → completed");
    println!("[数据摄入中台] ═══ {} 处理完成 >>> 清洗后 {} 字符 ═══", file_id, clean_text.len());
}

// ═══════════════════════════════════════════════════════════════
// Note ingestion
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
        Err(e) => { eprintln!("[数据摄入中台] ⚠ 查询笔记失败: {}", e); return; }
    };
    if rows.is_empty() { return; }

    for row in &rows {
        let note_id = row["note_id"].as_str().unwrap_or("").to_string();
        if note_id.is_empty() { continue; }
        let title = row["title"].as_str().unwrap_or("").to_string();
        let raw_content = row["content"].as_str().unwrap_or("").to_string();
        // 将 TipTap HTML 转换为 Markdown（保留标题/列表/表格等格式）
        let markdown = crate::services::file_parser::html_to_markdown(&raw_content);
        let clean = crate::services::file_parser::sanitize_chars(&markdown);
        if clean.trim().is_empty() { continue; }

        const LONG_NOTE_THRESHOLD: usize = 500;

        if clean.chars().count() > LONG_NOTE_THRESHOLD {
            // 长笔记：保留原文以支持 chunk，入 ai_sync_queue
            let _ = crate::database::run(
                "INSERT OR REPLACE INTO source_data(source_type,source_id,title,content,updated_at) VALUES('NOTE',?1,?2,?3,datetime('now','localtime'))",
                &[serde_json::json!(&note_id), serde_json::json!(&title), serde_json::json!(&clean)],
            );
            crate::services::ai_sync::trigger_sync("notes", &note_id);
            println!("[数据摄入中台] 📝 长笔记 {} 已摄入 ({} 字符)，已入队物化", &note_id[..note_id.len().min(8)], clean.len());
        } else {
            // 短笔记：渲染键值对，不入 ai_sync_queue
            let mut lines = Vec::new();
            lines.push(format!("title: {}", title));
            // 取正文前 200 字作为摘要
            let preview: String = clean.chars().take(200).collect();
            if !preview.is_empty() { lines.push(format!("content: {}", preview)); }
            let kv = lines.join("\n");
            let _ = crate::database::run(
                "INSERT OR REPLACE INTO source_data(source_type,source_id,title,content,updated_at) VALUES('NOTE',?1,?2,?3,datetime('now','localtime'))",
                &[serde_json::json!(&note_id), serde_json::json!(&title), serde_json::json!(&kv)],
            );
            println!("[数据摄入中台] 📝 短笔记 {} 已摄入 ({} 字符)", &note_id[..note_id.len().min(8)], kv.len());
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// Product ingestion
// ═══════════════════════════════════════════════════════════════

/// 摄入产品：将尚未同步到 source_data 的产品写入（名称+指标+规格+描述）
fn ingest_pending_products() {
    let rows = match crate::database::all(
        "SELECT p.id, p.name, p.standard_specs, p.full_specs, p.description, p.series_id FROM products p \
         WHERE p.id NOT IN (SELECT source_id FROM source_data WHERE source_type='PRODUCT') \
         LIMIT 20",
        &[],
    ) {
        Ok(r) => r,
        Err(e) => { eprintln!("[数据摄入中台] ⚠ 查询产品失败: {}", e); return; }
    };
    if rows.is_empty() { return; }

    for row in &rows {
        let prod_id = row["id"].as_str().unwrap_or("").to_string();
        if prod_id.is_empty() { continue; }
        let name = row["name"].as_str().unwrap_or("").to_string();
        let specs = row["standard_specs"].as_str().unwrap_or("").to_string();
        let full = row["full_specs"].as_str().unwrap_or("").to_string();
        let desc = row["description"].as_str().unwrap_or("").to_string();
        let series_id = row["series_id"].as_str().unwrap_or("").to_string();
        let path = resolve_series_path(&series_id);

        // 渲染键值对
        let mut lines = Vec::new();
        lines.push(format!("name: {}", name));
        if !desc.is_empty() { lines.push(format!("description: {}", desc)); }
        if !specs.is_empty() { lines.push(format!("specs: {}", specs)); }
        if !full.is_empty() { lines.push(format!("full_specs: {}", full)); }
        if !path.is_empty() { lines.push(format!("path: {}", path)); }
        let content = lines.join("\n");

        let clean = crate::services::file_parser::sanitize_chars(&content);
        if clean.trim().is_empty() { continue; }
        let _ = crate::database::run(
            "INSERT OR REPLACE INTO source_data(source_type,source_id,title,content,updated_at) VALUES('PRODUCT',?1,?2,?3,datetime('now','localtime'))",
            &[serde_json::json!(&prod_id), serde_json::json!(&name), serde_json::json!(&clean)],
        );
        // ★ 不入 ai_sync_queue，结构化数据不走 chunk
        println!("[数据摄入中台] 📦 产品 {} 已摄入 ({} 字符)", &prod_id[..prod_id.len().min(8)], clean.len());
    }
}

// ═══════════════════════════════════════════════════════════════
// Project ingestion
// ═══════════════════════════════════════════════════════════════

/// 摄入项目：将尚未同步到 source_data 的项目写入（仅名称+描述）
fn ingest_pending_projects() {
    let rows = match crate::database::all(
        "SELECT p.id, p.name, COALESCE(p.description,'') as description FROM projects p \
         WHERE p.id NOT IN (SELECT source_id FROM source_data WHERE source_type='PROJECT') \
         LIMIT 20",
        &[],
    ) {
        Ok(r) => r,
        Err(e) => { eprintln!("[数据摄入中台] ⚠ 查询项目失败: {}", e); return; }
    };
    if rows.is_empty() { return; }

    for row in &rows {
        let proj_id = row["id"].as_str().unwrap_or("").to_string();
        if proj_id.is_empty() { continue; }
        let name = row["name"].as_str().unwrap_or("").to_string();
        let desc = row["description"].as_str().unwrap_or("").to_string();

        // 渲染键值对
        let mut lines = Vec::new();
        lines.push(format!("name: {}", name));
        if !desc.is_empty() { lines.push(format!("description: {}", desc)); }
        // ★ 同时附上方案产品名称
        if let Ok(rows2) = crate::database::all(
            "SELECT DISTINCT si.product_name FROM solution_items si
             JOIN solution_versions sv ON sv.id=si.solution_version_id AND sv.status='final'
             JOIN solutions s ON s.id=sv.solution_id WHERE s.project_id=?1 LIMIT 20",
            &[serde_json::json!(&proj_id)],
        ) {
            let names: Vec<String> = rows2.iter().filter_map(|r| r["product_name"].as_str().map(String::from)).collect();
            if !names.is_empty() { lines.push(format!("solution_products: {}", names.join(", "))); }
        }
        let content = lines.join("\n");

        let clean = crate::services::file_parser::sanitize_chars(&content);
        if clean.trim().is_empty() { continue; }
        let _ = crate::database::run(
            "INSERT OR REPLACE INTO source_data(source_type,source_id,title,content,updated_at) VALUES('PROJECT',?1,?2,?3,datetime('now','localtime'))",
            &[serde_json::json!(&proj_id), serde_json::json!(&name), serde_json::json!(&clean)],
        );
        // ★ 不入 ai_sync_queue，结构化数据不走 chunk
        println!("[数据摄入中台] 📁 项目 {} 已摄入 ({} 字符)", &proj_id[..proj_id.len().min(8)], clean.len());
    }
}
