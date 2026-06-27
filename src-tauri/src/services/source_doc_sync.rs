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

/// 根据 source_type 和 source_id 从业务表提取数据，同步到 source_data
/// 替代 SQLite 触发器逻辑
pub fn sync_entity(source_type: &str, source_id: &str) {
    let (title, content) = match source_type {
        "NOTE" => extract_note(source_id),
        "PRODUCT" => extract_product(source_id),
        "PROJECT" => extract_project(source_id),
        "TODO" => extract_todo(source_id),
        "FILE" => extract_file(source_id),
        "CHAT" => extract_chat(source_id),
        _ => {
            eprintln!("[source_doc_sync] 未知 source_type: {}", source_type);
            return;
        }
    };
    if content.trim().is_empty() {
        eprintln!("[source_doc_sync] 内容为空，跳过: {}/{}", source_type, source_id);
        return;
    }
    sync_entity_to_source_docs(source_type, source_id, &title, &content);
}

fn extract_note(note_id: &str) -> (String, String) {
    let row = match crate::database::get(
        "SELECT n.title, n.content, nb.name as nb_name FROM notes n LEFT JOIN notebooks nb ON nb.id=n.notebook_id WHERE n.note_id=?1",
        &[serde_json::json!(note_id)],
    ) {
        Ok(Some(r)) => r,
        _ => return (String::new(), String::new()),
    };
    let title = row["title"].as_str().unwrap_or("").to_string();
    let mut parts = Vec::new();
    if !title.is_empty() { parts.push(format!("title: {}", title)); }
    if let Some(c) = row["content"].as_str().filter(|c| !c.is_empty()) { parts.push(format!("content: {}", c)); }
    if let Some(nb) = row["nb_name"].as_str().filter(|n| !n.is_empty()) { parts.push(format!("notebook: {}", nb)); }
    (title, parts.join("\n"))
}

fn extract_product(product_id: &str) -> (String, String) {
    let row = match crate::database::get(
        "SELECT name, description, series_name, price, standard_specs, full_specs FROM products WHERE id=?1",
        &[serde_json::json!(product_id)],
    ) {
        Ok(Some(r)) => r,
        _ => return (String::new(), String::new()),
    };
    let title = row["name"].as_str().unwrap_or("").to_string();
    let mut parts = Vec::new();
    parts.push(format!("name: {}", title));
    if let Some(d) = row["description"].as_str().filter(|d| !d.is_empty()) { parts.push(format!("description: {}", d)); }
    if let Some(s) = row["series_name"].as_str().filter(|s| !s.is_empty()) { parts.push(format!("series_name: {}", s)); }
    if let Some(p) = row["standard_specs"].as_str().filter(|p| !p.is_empty()) { parts.push(format!("standard_specs: {}", p)); }
    if let Some(f) = row["full_specs"].as_str().filter(|f| !f.is_empty()) { parts.push(format!("full_specs: {}", f)); }
    (title, parts.join("\n"))
}

fn extract_project(project_id: &str) -> (String, String) {
    let row = match crate::database::get(
        "SELECT name, customer, description, status FROM projects WHERE id=?1",
        &[serde_json::json!(project_id)],
    ) {
        Ok(Some(r)) => r,
        _ => return (String::new(), String::new()),
    };
    let title = row["name"].as_str().unwrap_or("").to_string();
    let mut parts = Vec::new();
    parts.push(format!("name: {}", title));
    if let Some(c) = row["customer"].as_str().filter(|c| !c.is_empty()) { parts.push(format!("customer: {}", c)); }
    if let Some(d) = row["description"].as_str().filter(|d| !d.is_empty()) { parts.push(format!("description: {}", d)); }
    if let Some(s) = row["status"].as_str().filter(|s| !s.is_empty()) { parts.push(format!("status: {}", s)); }
    (title, parts.join("\n"))
}

fn extract_todo(todo_id: &str) -> (String, String) {
    let row = match crate::database::get(
        "SELECT title, content, priority, due_date FROM todos WHERE id=?1",
        &[serde_json::json!(todo_id)],
    ) {
        Ok(Some(r)) => r,
        _ => return (String::new(), String::new()),
    };
    let title = row["title"].as_str().unwrap_or("").to_string();
    let mut parts = Vec::new();
    parts.push(format!("title: {}", title));
    if let Some(c) = row["content"].as_str().filter(|c| !c.is_empty()) { parts.push(format!("content: {}", c)); }
    if let Some(p) = row["priority"].as_str().filter(|p| !p.is_empty()) { parts.push(format!("priority: {}", p)); }
    if let Some(d) = row["due_date"].as_str().filter(|d| !d.is_empty()) { parts.push(format!("due_date: {}", d)); }
    (title, parts.join("\n"))
}

fn extract_file(file_id: &str) -> (String, String) {
    // 文件没有单一 title，从 file_relations 获取所有 custom_name 拼接
    let rows = match crate::database::all(
        "SELECT custom_name FROM file_relations WHERE file_id=?1",
        &[serde_json::json!(file_id)],
    ) {
        Ok(r) => r,
        _ => return (String::new(), String::new()),
    };
    let names: Vec<String> = rows.iter()
        .filter_map(|r| r["custom_name"].as_str().filter(|n| !n.is_empty()).map(String::from))
        .collect();
    if names.is_empty() { return (String::new(), String::new()); }
    (String::new(), names.join("\n"))
}

fn extract_chat(session_id: &str) -> (String, String) {
    let row = match crate::database::get(
        "SELECT title FROM chat_sessions WHERE id=?1",
        &[serde_json::json!(session_id)],
    ) {
        Ok(Some(r)) => r,
        _ => return (String::new(), String::new()),
    };
    let title = row["title"].as_str().unwrap_or("").to_string();
    let msgs = crate::database::all(
        "SELECT role, content FROM chat_messages WHERE session_id=?1 ORDER BY id",
        &[serde_json::json!(session_id)],
    ).unwrap_or_default();
    let chat_text: String = msgs.iter()
        .filter_map(|m| {
            let role = m["role"].as_str()?;
            let content = m["content"].as_str()?;
            if content.trim().is_empty() { return None; }
            Some(format!("[{}]: {}", role, content))
        })
        .collect::<Vec<_>>()
        .join("\n");
    (title, chat_text)
}
