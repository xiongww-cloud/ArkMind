//! 统一源数据同步模块
//!
//! 笔记/产品/项目/待办的 CRUD 操作通过此模块，将清洗后的数据
//! 统一写入 source_data 表，并触发防抖 AI 物化调度。

/// 将业务实体的文本内容清洗后同步到 source_data。
/// content 为空时跳过写入。
///
/// 副作用：通过全局防抖调度器将实体加入物化队列（5 秒静默窗口）。
pub fn sync_entity_to_source_docs(source_type: &str, source_id: &str, title: &str, content: &str) -> Result<(), String> {
    if content.trim().is_empty() { return Ok(()); }
    let clean = crate::services::file_parser::sanitize_chars(content);
    if clean.trim().is_empty() { return Ok(()); }
    crate::database::run(
        "INSERT INTO source_data(id,source_type,source_id,title,content,updated_at) VALUES(?1,?2,?3,?4,?5,datetime('now','localtime')) \
         ON CONFLICT(source_type,source_id) DO UPDATE SET title=excluded.title, content=excluded.content, updated_at=CASE WHEN content!=excluded.content THEN excluded.updated_at ELSE updated_at END",
        &[
            serde_json::json!(source_id),  // id = source_id
            serde_json::json!(source_type), // source_type
            serde_json::json!(source_id),  // source_id
            serde_json::json!(title),
            serde_json::json!(&clean),
        ],
    ).map_err(|e| format!("写入 source_data 失败: {}", e))?;

    // 纯轮询：不在此触发物化，sync_loop 每 10s 扫描 source_data 变更统一入队
    Ok(())
}

/// 删除实体对应的 source_data 记录
#[allow(dead_code)]
pub fn delete_entity_source_doc(source_type: &str, source_id: &str) {
    let _ = crate::database::run(
        "DELETE FROM source_data WHERE source_type=?1 AND source_id=?2",
        &[serde_json::json!(source_type), serde_json::json!(source_id)],
    );
}

/// 根据 source_type 和 source_id 从业务表提取数据，同步到 source_data
/// 替代 SQLite 触发器逻辑
pub fn sync_entity(source_type: &str, source_id: &str) -> Result<(), String> {
    let (title, content) = match source_type {
        "NOTE" => extract_note(source_id),
        "PRODUCT" => extract_product(source_id),
        "PROJECT" => extract_project(source_id),
        "TODO" => extract_todo(source_id),
        "FILE" => extract_file(source_id),
        "CHAT" => extract_chat(source_id),
        "EMAIL" => extract_email(source_id),
        "CONTACT" => extract_contact(source_id),
        "CHANNEL" => extract_channel(source_id),
        "VENDOR" => extract_vendor(source_id),
        "CUSTOMER" => extract_customer(source_id),
        _ => {
            log::info!("[源文档同步] 未知 source_type: {}", source_type);
            return Ok(());
        }
    };
    if content.trim().is_empty() {
        log::info!("[源文档同步] 内容为空，跳过: {}/{}", source_type, source_id);
        return Ok(());
    }
    log::info!("[源文档同步] 同步: {}/{} ({})", source_type, source_id, title);
    sync_entity_to_source_docs(source_type, source_id, &title, &content)
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
    let content = row["content"].as_str().unwrap_or("");
    let markdown = if content.contains('<') {
        crate::services::file_parser::html_to_markdown(content)
    } else {
        content.to_string()
    };
    let mut parts = Vec::new();
    if !title.is_empty() {
        parts.push(format!("# {}", title));
    }
    if !markdown.trim().is_empty() {
        parts.push(markdown);
    }
    (title, parts.join("\n\n"))
}

fn extract_product(product_id: &str) -> (String, String) {
    let row = match crate::database::get(
        "SELECT name, description, series_name, price, full_specs FROM products WHERE id=?1",
        &[serde_json::json!(product_id)],
    ) {
        Ok(Some(r)) => r,
        _ => return (String::new(), String::new()),
    };
    let title = row["name"].as_str().unwrap_or("").to_string();
    let mut parts = Vec::new();
    if !title.is_empty() {
        parts.push(format!("# {}", title));
    }
    if let Some(d) = row["description"].as_str().filter(|d| !d.is_empty()) {
        parts.push(d.to_string());
    }
    let mut specs = Vec::new();
    if let Some(s) = row["series_name"].as_str().filter(|s| !s.is_empty()) {
        specs.push(format!("系列：{}", s));
    }
    if let Some(f) = row["full_specs"].as_str().filter(|f| !f.is_empty()) {
        specs.push(f.to_string());
    }
    if !specs.is_empty() {
        parts.push(format!("## 技术参数\n{}", specs.join("\n")));
    }
    (title, parts.join("\n\n"))
}

fn extract_project(project_id: &str) -> (String, String) {
    let row = match crate::database::get(
        "SELECT name, description, status, owner, budget, bid_time, expected_order_date FROM projects WHERE id=?1",
        &[serde_json::json!(project_id)],
    ) {
        Ok(Some(r)) => r,
        _ => return (String::new(), String::new()),
    };
    let title = row["name"].as_str().unwrap_or("").to_string();
    let mut parts = Vec::new();
    if !title.is_empty() {
        parts.push(format!("# {}", title));
    }
    // 结构化输出业务字段
    let mut meta = Vec::new();
    if let Some(s) = row["status"].as_str().filter(|s| !s.is_empty()) { meta.push(format!("阶段：{}", s)); }
    if let Some(o) = row["owner"].as_str().filter(|o| !o.is_empty()) { meta.push(format!("负责人：{}", o)); }
    if let Some(b) = row["budget"].as_f64().filter(|b| *b > 0.0) { meta.push(format!("金额：¥{}万", b)); }
    if let Some(bt) = row["bid_time"].as_str().filter(|s| !s.is_empty()) { meta.push(format!("投标时间：{}", bt)); }
    if let Some(eo) = row["expected_order_date"].as_str().filter(|s| !s.is_empty()) { meta.push(format!("预计下单：{}", eo)); }
    if !meta.is_empty() { parts.push(meta.join(" | ")); }
    if let Some(d) = row["description"].as_str().filter(|d| !d.is_empty()) {
        parts.push(d.to_string());
    }
    (title, parts.join("\n\n"))
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
    if !title.is_empty() {
        parts.push(format!("# {}", title));
    }
    let mut details = Vec::new();
    if let Some(p) = row["phone"].as_str().filter(|p| !p.is_empty()) {
        details.push(format!("电话：{}", p));
    }
    if let Some(c) = row["address"].as_str().filter(|c| !c.is_empty()) {
        details.push(format!("地址：{}", c));
    }
    if let Some(r) = row["remark"].as_str().filter(|r| !r.is_empty()) {
        details.push(format!("备注：{}", r));
    }
    if !details.is_empty() {
        parts.push(details.join("\n"));
    }
    (title, parts.join("\n\n"))
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
    if !title.is_empty() {
        parts.push(format!("# {}", title));
    }
    let mut details = Vec::new();
    if let Some(p) = row["phone"].as_str().filter(|p| !p.is_empty()) {
        details.push(format!("电话：{}", p));
    }
    if let Some(c) = row["address"].as_str().filter(|c| !c.is_empty()) {
        details.push(format!("地址：{}", c));
    }
    if let Some(r) = row["remark"].as_str().filter(|r| !r.is_empty()) {
        details.push(format!("备注：{}", r));
    }
    if !details.is_empty() {
        parts.push(details.join("\n"));
    }
    (title, parts.join("\n\n"))
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
    if !title.is_empty() {
        parts.push(format!("# {}", title));
    }
    let mut details = Vec::new();
    if let Some(p) = row["phone"].as_str().filter(|p| !p.is_empty()) {
        details.push(format!("电话：{}", p));
    }
    if let Some(c) = row["address"].as_str().filter(|c| !c.is_empty()) {
        details.push(format!("地址：{}", c));
    }
    if let Some(r) = row["remark"].as_str().filter(|r| !r.is_empty()) {
        details.push(format!("备注：{}", r));
    }
    if !details.is_empty() {
        parts.push(details.join("\n"));
    }
    (title, parts.join("\n\n"))
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
    let content = row["content"].as_str().unwrap_or("");
    let markdown = if content.contains('<') {
        crate::services::file_parser::html_to_markdown(content)
    } else {
        content.to_string()
    };
    let mut parts = Vec::new();
    if !title.is_empty() {
        parts.push(format!("# {}", title));
    }
    if !markdown.trim().is_empty() {
        parts.push(markdown);
    }
    (title, parts.join("\n\n"))
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
    let title = names.first().cloned().unwrap_or_default();
    let content = names.join("\n");
    (title, content)
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
    let mut parts = Vec::new();
    if !title.is_empty() {
        parts.push(format!("# {}", title));
    }
    if !chat_text.is_empty() {
        parts.push(chat_text);
    }
    (title, parts.join("\n\n"))
}

pub fn extract_email(email_id: &str) -> (String, String) {
    let row = match crate::database::get(
        "SELECT e.subject, e.from_addr, e.from_name, e.to_addr, e.body_text, e.body_html, e.received_at, e.ai_category, e.folder FROM emails e WHERE e.id=?1",
        &[serde_json::json!(email_id)],
    ) {
        Ok(Some(r)) => r,
        _ => return (String::new(), String::new()),
    };

    let subject = row["subject"].as_str().unwrap_or("");
    let from_name = row["from_name"].as_str().unwrap_or("");
    let from_addr = row["from_addr"].as_str().unwrap_or("");
    let to_addr = row["to_addr"].as_str().unwrap_or("");
    let body_text = row["body_text"].as_str().unwrap_or("");
    let body_html = row["body_html"].as_str().unwrap_or("");
    let received_at = row["received_at"].as_str().unwrap_or("");
    let folder = row["folder"].as_str().unwrap_or("INBOX");

    let title = if subject.is_empty() {
        format!("邮件: {}", from_addr)
    } else {
        subject.to_string()
    };

    // 构建邮件头部信息（纯文本格式，用于搜索和摘要）
    let header = format!(
        "发件人：{} <{}>\n收件人：{}\n时间：{}\n文件夹：{}",
        from_name, from_addr, to_addr, received_at, folder
    );

    // 优先使用 HTML 转换为 Markdown（保留表格、列表、链接等结构）
    // 降级使用纯文本
    let body = if !body_html.is_empty() {
        crate::services::file_parser::html_to_markdown_v2(body_html)
    } else if !body_text.is_empty() {
        body_text.to_string()
    } else {
        String::new()
    };

    // 附加附件解析文本
    let mut attachment_parts = Vec::new();
    if let Ok(attachments) = crate::database::all(
        "SELECT filename, parsed_text FROM email_attachments WHERE email_id=?1 AND parsed_text != ''",
        &[serde_json::json!(email_id)],
    ) {
        for att in &attachments {
            let filename = att["filename"].as_str().unwrap_or("");
            let text = att["parsed_text"].as_str().unwrap_or("");
            if !text.is_empty() {
                attachment_parts.push(format!("附件: {}\n{}", filename, text));
            }
        }
    }

    // 组合最终内容：标题 + 头部 + 正文 + 附件
    let mut content = format!("{}\n{}", header, body);
    if !attachment_parts.is_empty() {
        content.push_str("\n\n");
        content.push_str(&attachment_parts.join("\n\n"));
    }

    (title, content)
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
    let mut parts = vec![format!("# {}", name)];
    let mut details = Vec::new();
    if let Some(v) = row["phone"].as_str().filter(|s| !s.is_empty()) {
        details.push(format!("电话：{}", v));
    }
    if let Some(v) = row["email"].as_str().filter(|s| !s.is_empty()) {
        details.push(format!("邮箱：{}", v));
    }
    if let Some(v) = row["position"].as_str().filter(|s| !s.is_empty()) {
        details.push(format!("职位：{}", v));
    }
    if let Some(v) = row["address"].as_str().filter(|s| !s.is_empty()) {
        details.push(format!("公司：{}", v));
    }
    if let Some(v) = row["role"].as_str().filter(|s| !s.is_empty()) {
        details.push(format!("角色：{}", v));
    }
    if !details.is_empty() {
        parts.push(details.join("\n"));
    }
    (name, parts.join("\n\n"))
}
