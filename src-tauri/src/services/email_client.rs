//! Email client — IMAP 连接、邮件获取、MIME 解析、附件处理

use mailparse::ParsedMail;
use serde::{Deserialize, Serialize};

/// IMAP 账户配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailAccount {
    pub id: String,
    pub name: String,
    pub email_address: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub username: String,
    pub password_encrypted: String,
    pub sync_enabled: bool,
    pub last_sync_inbox_uid: u32,
    pub last_sync_sent_uid: u32,
}

/// 解析后的邮件
#[derive(Debug, Clone)]
pub struct ParsedEmail {
    pub uid: u32,
    pub message_id: String,
    pub from_addr: String,
    pub from_name: String,
    pub to_addr: String,
    pub cc_addr: String,
    pub subject: String,
    pub body_text: String,
    pub body_html: String,
    pub received_at: String,
    pub has_attachments: bool,
    pub attachments: Vec<EmailAttachment>,
}

/// 邮件附件
#[derive(Debug, Clone)]
pub struct EmailAttachment {
    pub filename: String,
    pub mime_type: String,
    pub size: usize,
    pub parsed_text: String,
    pub raw_bytes: Vec<u8>,
    pub content_id: Option<String>,
    pub is_inline: bool,
}

/// 带超时的 IMAP 连接（imap::connect 内部用无超时 TcpStream::connect，
/// 服务器黑洞时可无限挂起 tokio worker，拖垮整个 async 运行时）
fn imap_connect(host: &str, port: u16) -> Result<imap::Client<native_tls::TlsStream<std::net::TcpStream>>, String> {
    use std::net::{TcpStream, ToSocketAddrs};
    use std::time::Duration;
    let addrs: Vec<_> = (host, port).to_socket_addrs()
        .map_err(|e| format!("解析 IMAP 主机 {} 失败: {}", host, e))?
        .collect();
    if addrs.is_empty() {
        return Err(format!("IMAP 主机无法解析: {}", host));
    }
    let mut tcp = None;
    let mut last_err = String::new();
    for a in &addrs {
        match TcpStream::connect_timeout(a, Duration::from_secs(15)) {
            Ok(s) => { tcp = Some(s); break; }
            Err(e) => last_err = e.to_string(),
        }
    }
    let tcp = tcp.ok_or_else(|| format!("IMAP 连接超时({}): {}", host, last_err))?;
    tcp.set_read_timeout(Some(Duration::from_secs(120))).ok();
    tcp.set_write_timeout(Some(Duration::from_secs(30))).ok();
    let tls = native_tls::TlsConnector::builder()
        .build()
        .map_err(|e| format!("TLS 创建失败: {}", e))?;
    let tls_stream = tls.connect(host, tcp).map_err(|e| format!("TLS 握手失败: {}", e))?;
    let mut client = imap::Client::new(tls_stream);
    client.read_greeting().map_err(|e| format!("IMAP 握手失败: {}", e))?;
    Ok(client)}

/// 连接 IMAP 服务器并获取新邮件
pub fn fetch_new_emails(
    account: &EmailAccount,
    folder: &str,
    last_uid: u32,
) -> Result<Vec<ParsedEmail>, String> {
    let client = imap_connect(&account.imap_host, account.imap_port)?;

    // 解密密码
    let machine = crate::license::get_machine_hash();
    let password = crate::crypto::decrypt_api_key(&account.password_encrypted, &machine);

    let mut session = client
        .login(&account.username, &password)
        .map_err(|e| format!("IMAP 登录失败: {}", e.0))?;

    // 选择文件夹
    session
        .select(folder)
        .map_err(|e| format!("选择文件夹 {} 失败: {}", folder, e))?;

    // 搜索所有邮件，然后在 Rust 侧过滤大于 last_uid 的
    let uids = session
        .uid_search("ALL")
        .map_err(|e| format!("搜索邮件失败: {}", e))?;

    let mut new_uids: Vec<u32> = uids
        .into_iter()
        .filter(|uid| *uid > last_uid)
        .collect();
    new_uids.sort();

    if new_uids.is_empty() {
        let _ = session.logout();
        return Ok(vec![]);
    }

    // 逐封获取邮件（更稳健，避免批量获取导致解析失败）
    let mut all_emails = Vec::new();
    for uid in &new_uids {
        let uid_str = uid.to_string();
        match session.uid_fetch(&uid_str, "(BODY.PEEK[])") {
            Ok(messages) => {
                for msg in messages.iter() {
                    if let Some(body) = msg.body() {
                        let msg_uid = msg.uid.unwrap_or(*uid);
                        match parse_email(body) {
                            Ok(mut email) => {
                                email.uid = msg_uid;
                                all_emails.push(email);
                            }
                            Err(e) => {
                                log::warn!("[邮件] 解析邮件 UID={} 失败: {}", msg_uid, e);
                            }
                        }
                    }
                }
            }
            Err(e) => {
                log::warn!("[邮件] 获取邮件 UID={} 失败: {}", uid, e);
            }
        }
    }

    let _ = session.logout();
    Ok(all_emails)
}

/// 删除邮件（标记为已删除）


/// 获取指定文件夹当前最大 UID（用于新账户跳过历史邮件）
pub fn get_current_max_uid(account: &EmailAccount, folder: &str) -> Result<u32, String> {
    let client = imap_connect(&account.imap_host, account.imap_port)?;

    let machine = crate::license::get_machine_hash();
    let password = crate::crypto::decrypt_api_key(&account.password_encrypted, &machine);

    let mut session = client
        .login(&account.username, &password)
        .map_err(|e| format!("IMAP 登录失败: {}", e.0))?;

    session
        .select(folder)
        .map_err(|e| format!("选择文件夹 {} 失败: {}", folder, e))?;

    let uids = session
        .uid_search("ALL")
        .map_err(|e| format!("搜索邮件失败: {}", e))?;

    let max_uid = uids.iter().max().copied().unwrap_or(0);

    let _ = session.logout();
    Ok(max_uid)
}

/// 探测 Sent 文件夹名称
pub fn detect_sent_folder(account: &EmailAccount) -> String {
    let client = match imap_connect(&account.imap_host, account.imap_port) {
        Ok(c) => c,
        Err(_) => return "Sent".to_string(),
    };

    let machine = crate::license::get_machine_hash();
    let password = crate::crypto::decrypt_api_key(&account.password_encrypted, &machine);

    let mut session = match client.login(&account.username, &password) {
        Ok(s) => s,
        Err(_) => return "Sent".to_string(),
    };

    let folders = match session.list(None, Some("*")) {
        Ok(f) => f,
        Err(_) => {
            let _ = session.logout();
            return "Sent".to_string();
        }
    };
    let _ = session.logout();

    // 常见 Sent 文件夹名称
    let sent_names = [
        "Sent", "Sent Messages", "Sent Mail", "已发送",
        "[Gmail]/Sent Mail", "[Gmail]/已发邮件",
        "INBOX.Sent", "INBOX.Sent Messages",
    ];

    for name in &sent_names {
        for f in folders.iter() {
            if f.name().eq_ignore_ascii_case(name) {
                return f.name().to_string();
            }
        }
    }

    // 模糊匹配
    for f in folders.iter() {
        let lower = f.name().to_lowercase();
        if lower.contains("sent") || lower.contains("已发送") {
            return f.name().to_string();
        }
    }

    "Sent".to_string()
}

/// 将邮件 Date 头解析为标准格式 YYYY-MM-DD HH:MM:SS
fn normalize_email_date(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    }

    // 尝试 RFC 2822 格式: "Mon, 15 Sep 2026 10:30:00 +0800"（统一转为本地时间，保证跨时区可排序）
    if let Ok(dt) = chrono::DateTime::parse_from_rfc2822(trimmed) {
        return dt.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S").to_string();
    }

    // 尝试 RFC 3339/ISO 8601: "2026-09-15T10:30:00Z" / "2026-09-15T10:30:00+08:00"
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(trimmed) {
        return dt.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S").to_string();
    }

    // 尝试常见格式: "2026-09-15 10:30:00"
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%d %H:%M:%S") {
        return dt.format("%Y-%m-%d %H:%M:%S").to_string();
    }

    // 尝试: "2026-09-15T10:30:00"
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%dT%H:%M:%S") {
        return dt.format("%Y-%m-%d %H:%M:%S").to_string();
    }

    // 回退：使用当前时间
    log::warn!("[邮件] 无法解析日期: {}", trimmed);
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// 解析 MIME 邮件
fn parse_email(raw: &[u8]) -> Result<ParsedEmail, String> {
    let parsed = mailparse::parse_mail(raw).map_err(|e| format!("MIME 解析失败: {}", e))?;

    let headers = &parsed.headers;
    let message_id = get_header(headers, "Message-ID");
    let from_raw = get_header(headers, "From");
    let to_raw = get_header(headers, "To");
    let cc_raw = get_header(headers, "Cc");
    let subject = get_header(headers, "Subject");
    let date_raw = get_header(headers, "Date");
    let date = normalize_email_date(&date_raw);

    let (from_name, from_addr) = parse_from(&from_raw);

    let mut text_parts = Vec::new();
    let mut html_parts = Vec::new();
    let mut attachments = Vec::new();

    walk_mail(&parsed, &mut text_parts, &mut html_parts, &mut attachments);

    let body_text = text_parts.join("\n");
    let body_html = html_parts.join("\n");

    // body_text 为空但 body_html 不为空时，使用 HTML 转 Markdown（保留表格结构）
    let body_text = if body_text.trim().is_empty() && !body_html.trim().is_empty() {
        crate::services::file_parser::html_to_markdown(&body_html)
    } else {
        body_text
    };
    let has_attachments = !attachments.is_empty();

    Ok(ParsedEmail {
        uid: 0,
        message_id,
        from_addr,
        from_name,
        to_addr: to_raw,
        cc_addr: cc_raw,
        subject,
        body_text,
        body_html,
        received_at: date,
        has_attachments,
        attachments,
    })
}

/// 递归遍历 MIME 结构
fn walk_mail(
    mail: &ParsedMail,
    text_parts: &mut Vec<String>,
    html_parts: &mut Vec<String>,
    attachments: &mut Vec<EmailAttachment>,
) {
    let content_type = &mail.ctype.mimetype;
    let disposition = mail.get_content_disposition();

    // 读取 Content-ID
    let content_id = get_header(&mail.headers, "Content-ID");
    let content_id = if content_id.is_empty() { None } else { Some(content_id) };

    // 判断是否为附件
    let is_attachment = disposition.disposition == mailparse::DispositionType::Attachment;
    // 判断是否为内嵌内容（有 Content-ID 或图片类型）
    let has_content_id = content_id.is_some();
    let is_image = content_type.starts_with("image/");
    // 判断是否为非文本/HTML 类型（可能是附件）
    let is_non_text = !content_type.starts_with("text/") 
        && !content_type.starts_with("multipart/")
        && !content_type.starts_with("message/");

    // 判断是否应该作为附件处理
    let should_attach = is_attachment 
        || has_content_id 
        || (is_non_text && !content_type.is_empty());

    if should_attach {
        // 获取文件名
        let filename = if is_attachment {
            // 优先从 Content-Disposition 获取
            disposition.params.get("filename").cloned().unwrap_or_default()
        } else if let Some(cid) = &content_id {
            // 内嵌图片使用 Content-ID 作为文件名
            cid.trim_matches(|c| c == '<' || c == '>').to_string()
        } else {
            // 其他类型生成默认文件名
            let ext = content_type.split('/').last().unwrap_or("bin");
            format!("attachment_{}.{}", attachments.len(), ext)
        };

        // 如果文件名为空，生成默认文件名
        let filename = if filename.is_empty() {
            let ext = content_type.split('/').last().unwrap_or("bin");
            format!("attachment_{}.{}", attachments.len(), ext)
        } else {
            filename
        };

        if let Ok(body) = mail.get_body_raw() {
            let size = body.len();
            if size > 0 {
                // 图片类型默认视为内嵌（除非明确标记为附件）
                let is_inline = (has_content_id || is_image) && !is_attachment;
                attachments.push(EmailAttachment {
                    filename,
                    mime_type: content_type.clone(),
                    size,
                    parsed_text: String::new(),
                    raw_bytes: body,
                    content_id,
                    is_inline,
                });
            }
        }
    } else if content_type.starts_with("text/plain") {
        if let Ok(body) = mail.get_body() {
            text_parts.push(body);
        }
    } else if content_type.starts_with("text/html") {
        if let Ok(body) = mail.get_body() {
            html_parts.push(body);
        }
    }

    // 递归处理子部分
    for sub in &mail.subparts {
        walk_mail(sub, text_parts, html_parts, attachments);
    }
}



/// 获取邮件头
fn get_header(headers: &[mailparse::MailHeader], name: &str) -> String {
    headers
        .iter()
        .find(|h| h.get_key().eq_ignore_ascii_case(name))
        .map(|h| h.get_value())
        .unwrap_or_default()
}

/// 解析 From 头
fn parse_from(from: &str) -> (String, String) {
    // 格式: "Name <email>" 或 "email"
    if let Some(idx) = from.find('<') {
        let name = from[..idx].trim().trim_matches('"').to_string();
        let addr = from[idx + 1..]
            .find('>')
            .map(|end| from[idx + 1..idx + 1 + end].to_string())
            .unwrap_or_default();
        (name, addr)
    } else {
        (String::new(), from.trim().to_string())
    }
}


