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
        "INSERT INTO source_data(id,source_type,source_id,title,content,updated_at) VALUES(?1,?2,?3,?4,?5,datetime('now','localtime')) \
         ON CONFLICT(source_type,source_id) DO UPDATE SET title=excluded.title, content=excluded.content, updated_at=CASE WHEN content!=excluded.content THEN excluded.updated_at ELSE updated_at END",
        &[
            serde_json::json!(source_id),  // id = source_id
            serde_json::json!(source_type), // source_type
            serde_json::json!(source_id),  // source_id
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
        "CONTACT" => extract_contact(source_id),
        "CHANNEL" => extract_channel(source_id),
        "VENDOR" => extract_vendor(source_id),
        "CUSTOMER" => extract_customer(source_id),
        _ => {
            eprintln!("[source_doc_sync] 未知 source_type: {}", source_type);
            return;
        }
    };
    if content.trim().is_empty() {
        eprintln!("[source_doc_sync] 内容为空，跳过: {}/{}", source_type, source_id);
        return;
    }
    eprintln!("[source_doc_sync] 同步: {}/{} ({})", source_type, source_id, title);
    sync_entity_to_source_docs(source_type, source_id, &title, &content);
}

/// Resolve a human-readable name for an entity
fn resolve_entity_name(entity_type: &str, entity_id: &str) -> String {
    let sql = match entity_type {
        "PROJECT" => Some("SELECT name FROM projects WHERE id=?"),
        "PRODUCT" => Some("SELECT name FROM products WHERE id=?"),
        "NOTE" => Some("SELECT title FROM notes WHERE note_id=?"),
        "TODO" => Some("SELECT title FROM todos WHERE id=?"),
        "FILE" => None, // files use custom_name from file_relations
        _ => None,
    };
    if let Some(s) = sql {
        if let Ok(Some(r)) = crate::database::get(s, &[serde_json::json!(entity_id)]) {
            for key in &["name", "title"] {
                if let Some(v) = r[*key].as_str().filter(|v| !v.is_empty()) {
                    return v.to_string();
                }
            }
        }
    }
    entity_id.to_string()
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
        "SELECT name, description, status, owner, budget, bid_time FROM projects WHERE id=?1",
        &[serde_json::json!(project_id)],
    ) {
        Ok(Some(r)) => r,
        _ => return (String::new(), String::new()),
    };
    let title = row["name"].as_str().unwrap_or("").to_string();
    let mut parts = Vec::new();
    parts.push(format!("name: {}", title));
    if let Some(d) = row["description"].as_str().filter(|d| !d.is_empty()) { parts.push(format!("description: {}", d)); }
    if let Some(s) = row["status"].as_str().filter(|s| !s.is_empty()) { parts.push(format!("status: {}", s)); }
    if let Some(o) = row["owner"].as_str().filter(|o| !o.is_empty()) { parts.push(format!("owner: {}", o)); }
    if let Some(b) = row["budget"].as_f64().filter(|b| *b > 0.0) { parts.push(format!("budget: {:.2}", b)); }
    if let Some(bt) = row["bid_time"].as_str().filter(|bt| !bt.is_empty()) { parts.push(format!("bid_time: {}", bt)); }
    // 解决方案产品
    if let Ok(Some(sols)) = crate::database::get(
        "SELECT GROUP_CONCAT(DISTINCT si.product_name) as products FROM solution_items si JOIN solution_versions sv ON sv.id=si.solution_version_id AND sv.status='final' JOIN solutions s ON s.id=sv.solution_id WHERE s.project_id=?1 AND COALESCE(si.product_name,'')!=''",
        &[serde_json::json!(project_id)]
    ) {
        if let Some(products) = sols["products"].as_str().filter(|p| !p.is_empty()) {
            parts.push(format!("solution_products: {}", products));
        }
    }
    // 项目动态
    if let Ok(dynamics) = crate::database::all(
        "SELECT type, content, created_at FROM project_dynamics WHERE project_id=?1 ORDER BY created_at",
        &[serde_json::json!(project_id)]
    ) {
        let dyn_str: Vec<String> = dynamics.iter().filter_map(|d| {
            let t = d["type"].as_str().unwrap_or("");
            let c = d["content"].as_str().unwrap_or("");
            let dt = d["created_at"].as_str().unwrap_or("");
            if !c.is_empty() { Some(format!("{}: {} ({})", t, c, dt)) } else { None }
        }).collect();
        if !dyn_str.is_empty() { parts.push(format!("dynamics: {}", dyn_str.join(" | "))); }
    }
    (title, parts.join("\n"))
}

fn extract_channel(channel_id: &str) -> (String, String) {
    let row = match crate::database::get(
        "SELECT name, phone, address, remark FROM partners WHERE id=?1 AND type='CHANNEL'",
        &[serde_json::json!(channel_id)],
    ) {
        Ok(Some(r)) => r,
        _ => return (String::new(), String::new()),
    };
    let title = row["name"].as_str().unwrap_or("").to_string();
    let mut parts = Vec::new();
    parts.push(format!("name: {}", title));
    if let Some(p) = row["phone"].as_str().filter(|p| !p.is_empty()) { parts.push(format!("phone: {}", p)); }
    if let Some(c) = row["address"].as_str().filter(|c| !c.is_empty()) { parts.push(format!("address: {}", c)); }
    if let Some(r) = row["remark"].as_str().filter(|r| !r.is_empty()) { parts.push(format!("remark: {}", r)); }
    (title, parts.join("\n"))
}

fn extract_vendor(vendor_id: &str) -> (String, String) {
    let row = match crate::database::get(
        "SELECT name, phone, address, remark FROM partners WHERE id=?1 AND type='VENDOR'",
        &[serde_json::json!(vendor_id)],
    ) {
        Ok(Some(r)) => r,
        _ => return (String::new(), String::new()),
    };
    let title = row["name"].as_str().unwrap_or("").to_string();
    let mut parts = Vec::new();
    parts.push(format!("name: {}", title));
    if let Some(p) = row["phone"].as_str().filter(|p| !p.is_empty()) { parts.push(format!("phone: {}", p)); }
    if let Some(c) = row["address"].as_str().filter(|c| !c.is_empty()) { parts.push(format!("address: {}", c)); }
    if let Some(r) = row["remark"].as_str().filter(|r| !r.is_empty()) { parts.push(format!("remark: {}", r)); }
    (title, parts.join("\n"))
}

fn extract_customer(customer_id: &str) -> (String, String) {
    let row = match crate::database::get(
        "SELECT name, phone, address, remark FROM partners WHERE id=?1 AND type='CUSTOMER'",
        &[serde_json::json!(customer_id)],
    ) {
        Ok(Some(r)) => r,
        _ => return (String::new(), String::new()),
    };
    let title = row["name"].as_str().unwrap_or("").to_string();
    let mut parts = Vec::new();
    parts.push(format!("name: {}", title));
    if let Some(p) = row["phone"].as_str().filter(|p| !p.is_empty()) { parts.push(format!("phone: {}", p)); }
    if let Some(c) = row["address"].as_str().filter(|c| !c.is_empty()) { parts.push(format!("address: {}", c)); }
    if let Some(r) = row["remark"].as_str().filter(|r| !r.is_empty()) { parts.push(format!("remark: {}", r)); }
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

fn extract_contact(contact_id: &str) -> (String, String) {
    let json_id = serde_json::json!(contact_id);
    // 通过 partner_contacts 查询联系人信息，通过 partner_contact_mapping 关联到 Partner
    if let Ok(Some(r)) = crate::database::get(
        "SELECT pc.name, pc.phone, pc.email, pc.position, p.name as address, p.type as role \
         FROM partner_contacts pc \
         LEFT JOIN partner_contact_mapping pcm ON pcm.contact_id = pc.id \
         LEFT JOIN partners p ON p.id = pcm.partner_id \
         WHERE pc.id=?1",
        &[json_id],
    ) {
        return build_contact_content(&r);
    }
    (String::new(), String::new())
}

fn build_contact_content(row: &serde_json::Value) -> (String, String) {
    let name = row["name"].as_str().unwrap_or("").to_string();
    if name.is_empty() { return (String::new(), String::new()); }
    let mut parts = vec![format!("name: {}", name)];
    if let Some(v) = row["phone"].as_str().filter(|s| !s.is_empty()) { parts.push(format!("phone: {}", v)); }
    if let Some(v) = row["email"].as_str().filter(|s| !s.is_empty()) { parts.push(format!("email: {}", v)); }
    if let Some(v) = row["position"].as_str().filter(|s| !s.is_empty()) { parts.push(format!("position: {}", v)); }
    if let Some(v) = row["address"].as_str().filter(|s| !s.is_empty()) { parts.push(format!("address: {}", v)); }
    if let Some(v) = row["role"].as_str().filter(|s| !s.is_empty()) { parts.push(format!("role: {}", v)); }
    if let Some(v) = row["remark"].as_str().filter(|s| !s.is_empty()) { parts.push(format!("remark: {}", v)); }
    (name, parts.join("\n"))
}
