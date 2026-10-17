//! 邮件轮询服务 — 后台 IMAP 轮询 + AI 分类执行

use crate::database;
use crate::services::email_client::{self, EmailAccount, ParsedEmail};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

static LOOP_RUNNING: AtomicBool = AtomicBool::new(false);

/// 缓存 AppHandle，供后台通知推送（notification:new）使用
static APP_HANDLE: OnceLock<tauri::AppHandle> = OnceLock::new();

/// 在应用启动时注入 AppHandle（幂等）
pub fn init_app_handle(handle: tauri::AppHandle) {
    let _ = APP_HANDLE.set(handle);
}

/// 从 MIME 文件名安全提取扩展名：仅允许 1-10 位字母数字，否则回退 bin（防路径穿越）
pub fn safe_attachment_ext(filename: &str) -> String {
    let raw = filename.rsplit('.').next().unwrap_or("").to_lowercase();
    if !raw.is_empty() && raw.len() <= 10 && raw.chars().all(|c| c.is_ascii_alphanumeric()) {
        raw
    } else {
        "bin".to_string()
    }
}

/// 自动化发件人判定（收件箱黑名单）：noreply 类机器邮件不入知识库（ai_category='ignore' 终态）。
/// 只匹配 @ 前的本地部分前缀，误伤概率低；已发送邮件不受此限制。
pub fn is_automated_sender(from_addr: &str) -> bool {
    const PREFIXES: [&str; 11] = [
        "noreply", "no-reply", "no_reply", "donotreply", "do-not-reply",
        "auto-reply", "autoreply", "postmaster", "mailer-daemon", "bounce", "returns",
    ];
    let local = from_addr.split('@').next().unwrap_or("").to_ascii_lowercase();
    let local = local.trim();
    PREFIXES.iter().any(|p| local.starts_with(p))
}



/// 启动邮件轮询服务（幂等）
pub fn start_email_loop() {
    if LOOP_RUNNING.swap(true, Ordering::SeqCst) {
        return; // 已在运行
    }

    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("邮件轮询 tokio runtime 创建失败");

        rt.block_on(async {
            log::info!("[邮件轮询] 服务启动");
            // 启动后等待 20 秒，让应用初始化完成
            tokio::time::sleep(Duration::from_secs(20)).await;

            // 保留期清理 24h 节流：首轮由 lib.rs 启动清扫负责，这里对齐启动时刻避免双跑
            let mut last_retention_sweep = std::time::Instant::now();

            loop {
                if last_retention_sweep.elapsed() >= Duration::from_secs(24 * 3600) {
                    last_retention_sweep = std::time::Instant::now();
                    let n = crate::commands::email_commands::cleanup_expired_emails();
                    if n > 0 { log::info!("[邮件轮询] 周期保留期清理：物理删除 {} 封", n); }
                }
                match tick().await {
                    Ok(wait_secs) => {
                        tokio::time::sleep(Duration::from_secs(wait_secs)).await;
                    }
                    Err(e) => {
                        log::error!("[邮件轮询] tick 异常: {}，30秒后重试", e);
                        tokio::time::sleep(Duration::from_secs(30)).await;
                    }
                }
            }
        });
    });
}

/// 单次轮询（节奏固定：每 1 分钟一次，不提供可配置间隔）
async fn tick() -> Result<u64, String> {
    let enabled = get_config("enabled").unwrap_or_else(|| "1".to_string());
    if !crate::database::is_truthy_flag(&enabled) {
        return Ok(60);
    }

    let last_check = get_config("last_check_at").unwrap_or_default();
    if !last_check.is_empty() {
        if let Ok(last_time) =
            chrono::NaiveDateTime::parse_from_str(&last_check, "%Y-%m-%d %H:%M")
        {
            let now = chrono::Local::now().naive_local();
            let diff = now.signed_duration_since(last_time).num_minutes();
            if diff < 1 {
                // 封顶 60s：时钟回拨时 (1-diff) 无上界，曾导致一次性 sleep 数天（同 notification_loop 已修问题）
                return Ok(((1 - diff).clamp(0, 1) as u64) * 60);
            }
        }
    }

    // 执行同步
    if let Err(e) = run_sync().await {
        log::error!("[邮件轮询] 同步失败: {}", e);
    }

    set_config(
        "last_check_at",
        &chrono::Local::now().format("%Y-%m-%d %H:%M").to_string(),
    );

    Ok(60)
}

/// 执行邮件同步 + AI 分类（每账户每轮一次登录：同会话顺序完成收件箱/已发送，
/// 登录频率从 2-3 次/分钟降到 1 次/分钟，直击企业邮箱 IMAP 登录限频；
/// 登录失败按账户指数退避，避免被限频后每分钟继续踩踏）
async fn run_sync() -> Result<(), String> {
    let accounts = load_accounts()?;
    if accounts.is_empty() {
        return Ok(());
    }

    for account in &accounts {
        if !account.sync_enabled {
            continue;
        }
        let now_ts = chrono::Local::now().timestamp();
        if backoff_should_skip(&account.id, now_ts) {
            continue;
        }
        let mut session = match email_client::open_session(account) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("[邮件轮询] 登录失败 ({}): {}（指数退避后重试）", account.name, e);
                backoff_fail(&account.id, now_ts);
                continue;
            }
        };

        // ── 收件箱同步 ──
        if account.last_sync_inbox_uid == 0 {
            // 新账户：只记录当前最大 UID，不获取历史邮件
            match email_client::get_current_max_uid_with(&mut session, "INBOX") {
                Ok(max_uid) => {
                    update_last_uid(&account.id, "inbox", max_uid);
                    log::info!("[邮件轮询] 新账户首次同步，收件箱当前最大 UID={}, 跳过历史邮件", max_uid);
                }
                Err(e) => {
                    log::warn!("[邮件轮询] 获取收件箱最大 UID 失败 ({}): {}", account.name, e);
                }
            }
        } else {
            match email_client::fetch_new_emails_with(&mut session, account, "INBOX", account.last_sync_inbox_uid) {
                Ok(mut emails) => {
                    if !emails.is_empty() {
                        let max_uid = emails.iter().map(|e| e.uid).max().unwrap_or(0);
                        for email in emails.iter_mut() {
                            store_email(account, "INBOX", email).await;
                        }
                        update_last_uid(&account.id, "inbox", max_uid);
                        log::info!("[邮件轮询] 收件箱同步完成: {} 封新邮件，已写入知识库", emails.len());
                    }
                }
                Err(e) => {
                    log::warn!("[邮件轮询] 收件箱同步失败 ({}): {}", account.name, e);
                }
            }
        }

        // ── 已发送同步 ──
        let sent_folder = get_or_detect_sent_folder(account, &mut session);
        if account.last_sync_sent_uid == 0 {
            // 新账户：只记录当前最大 UID，不获取历史邮件
            match email_client::get_current_max_uid_with(&mut session, &sent_folder) {
                Ok(max_uid) => {
                    update_last_uid(&account.id, "sent", max_uid);
                    log::info!("[邮件轮询] 新账户首次同步，已发送当前最大 UID={}, 跳过历史邮件", max_uid);
                }
                Err(e) => {
                    log::warn!("[邮件轮询] 获取已发送最大 UID 失败 ({}): {}", account.name, e);
                }
            }
        } else {
            match email_client::fetch_new_emails_with(&mut session, account, &sent_folder, account.last_sync_sent_uid) {
                Ok(mut emails) => {
                    if !emails.is_empty() {
                        let max_uid = emails.iter().map(|e| e.uid).max().unwrap_or(0);
                        for email in emails.iter_mut() {
                            store_email(account, "Sent", email).await;
                        }
                        update_last_uid(&account.id, "sent", max_uid);
                        log::info!("[邮件轮询] 已发送同步完成: {} 封新邮件", emails.len());
                    }
                }
                Err(e) => {
                    log::warn!("[邮件轮询] 已发送同步失败 ({}): {}", account.name, e);
                }
            }
        }

        let _ = session.logout();
        backoff_ok(&account.id);
    }

    Ok(())
}

// ── 登录失败退避（仅登录级失败；文件夹级错误不退避——会话已建，无踩踏风险）──
static SYNC_BACKOFF: std::sync::Mutex<Option<std::collections::HashMap<String, (u32, i64)>>> =
    std::sync::Mutex::new(None);

/// 失败 1→2→4→8→16 分钟，30 分钟封顶（纯函数，可单测）
fn next_backoff_secs(fail_count: u32) -> i64 {
    let exp = fail_count.saturating_sub(1).min(5);
    (60i64 << exp).min(30 * 60)
}

fn backoff_should_skip(id: &str, now: i64) -> bool {
    if let Ok(g) = SYNC_BACKOFF.lock() {
        return g.as_ref().and_then(|m| m.get(id)).map(|(_, next)| now < *next).unwrap_or(false);
    }
    false
}

fn backoff_fail(id: &str, now: i64) {
    let mut g = SYNC_BACKOFF.lock().unwrap();
    let m = g.get_or_insert_with(std::collections::HashMap::new);
    let e = m.entry(id.to_string()).or_insert((0, 0));
    e.0 += 1;
    let secs = next_backoff_secs(e.0);
    e.1 = now + secs;
    log::warn!("[邮件轮询] {} 连续登录失败第 {} 次，{} 秒后重试", id, e.0, secs);
}

fn backoff_ok(id: &str) {
    if let Ok(mut g) = SYNC_BACKOFF.lock() {
        if let Some(m) = g.as_mut() {
            if m.remove(id).is_some() {
                log::info!("[邮件轮询] {} 登录恢复正常", id);
            }
        }
    }
}

/// 知识化收尾：黑名单外的收件/任何发件 → knowledge 并入队；自动化发件人 → ignore 终态不入队。
/// 幂等可重放（notify_changed 队列侧同实体合并），用于统一手动/自动两条入库路径。
fn finalize_email_knowledge(email_id: &str, from_addr: &str, folder: &str) {
    let automated = folder == "INBOX" && is_automated_sender(from_addr);
    let cat = if automated { "ignore" } else { "knowledge" };
    database::run(
        "UPDATE emails SET ai_category=?, ai_summary='' WHERE id=?",
        &[json!(cat), json!(email_id)],
    ).ok();
    if !automated {
        crate::services::knowledge_lifecycle::notify_changed("EMAIL", email_id);
    } else {
        log::info!("[邮件轮询] 自动化发件人命中黑名单，跳过知识库: {} ({})", from_addr, crate::preview(email_id, 8));
    }
}

/// 存储邮件到数据库（唯一入库口径：轮询与手动同步共用；异步，含附件落盘入库）
/// 顺序：查重 → 邮件行落库成功 → 才处理附件（防冲突忽略后附件挂到不存在的 email_id 成幽灵行）
pub(crate) async fn store_email(account: &EmailAccount, folder: &str, email: &mut ParsedEmail) -> String {
    // data:URI 内联 base64 图统一为内嵌附件 + src 改写 cid 引用（base64 不进文本层）
    let n_datauri = email_client::extract_data_uri_images(email);
    if n_datauri > 0 {
        log::info!("[邮件轮询] data:URI 内联图转附件: {} 张 ({})", n_datauri, crate::preview(&email.subject, 20));
    }

    // 同封邮件（account+folder+uid 唯一）已存在 → 复用现有 id，跳过重复落附件
    let existing: Option<String> = database::get(
        "SELECT id FROM emails WHERE account_id=?1 AND folder=?2 AND uid=?3",
        &[json!(&account.id), json!(folder), json!(email.uid)],
    ).ok().flatten().and_then(|r| r["id"].as_str().map(String::from));
    if let Some(eid) = existing {
        finalize_email_knowledge(&eid, &email.from_addr, folder);
        return eid;
    }

    let id = uuid::Uuid::new_v4().to_string();

    // 先存邮件（保留原始 body_html，内嵌图片由 email_render_html 在渲染时以 data URL 内联解析）
    let inserted = database::run(
        "INSERT OR IGNORE INTO emails(id, account_id, folder, uid, message_id, from_addr, from_name, to_addr, cc_addr, subject, body_text, body_html, received_at, has_attachments) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
        &[
            json!(id), json!(account.id), json!(folder), json!(email.uid),
            json!(email.message_id), json!(email.from_addr), json!(email.from_name),
            json!(email.to_addr), json!(email.cc_addr), json!(email.subject), json!(email.body_text),
            json!(email.body_html), json!(email.received_at), json!(email.has_attachments),
        ],
    );
    match inserted {
        Ok(r) if r.changes == 0 => {
            // 并发窗口内已被其他任务插入 → 回查真实 id
            let real = database::get(
                "SELECT id FROM emails WHERE account_id=?1 AND folder=?2 AND uid=?3",
                &[json!(&account.id), json!(folder), json!(email.uid)],
            ).ok().flatten().and_then(|r| r["id"].as_str().map(String::from));
            let eid = real.unwrap_or(id);
            finalize_email_knowledge(&eid, &email.from_addr, folder);
            return eid;
        }
        Ok(_) => {}
        Err(e) => {
            log::error!("[邮件轮询] 邮件行写入失败，跳过附件存储: {}", e);
            return id;
        }
    }

    // 邮件行已确认存在，再存附件
    for att in &email.attachments {
        let att_id = uuid::Uuid::new_v4().to_string();

        // 将附件原始字节写入临时文件，然后存入资料库；
        // 内嵌图（is_inline）只存渲染用资产、不触发 FILE 知识摄入——其 OCR 文字在 extract_email 并入邮件正文，避免双份来源
        let file_asset_id = if !att.raw_bytes.is_empty() {
            save_attachment_to_library(&att.raw_bytes, &att.filename, &att.mime_type, &id, att.is_inline).await
        } else {
            None
        };

        if let Err(e) = database::run(
            "INSERT INTO email_attachments(id, email_id, filename, mime_type, size, parsed_text, file_asset_id, content_id, is_inline) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            &[
                json!(att_id), json!(id), json!(att.filename), json!(att.mime_type),
                json!(att.size), json!(att.parsed_text), json!(file_asset_id),
                json!(att.content_id), json!(att.is_inline as i32),
            ],
        ) { log::warn!("[邮件轮询] 附件行写入失败 ({}): {}", att.filename, e); }
    }

    // 内嵌判定修正（渲染端 email_cid_map 已支持按 filename 回退解析 cid: 引用）：
    // 先选出将被改判为内嵌、且已产生 FILE 知识源的附件 → 修正后逐出知识（保持"内嵌图不进 FILE 源"不变式）
    let to_revoke: Vec<String> = database::all(
        "SELECT file_asset_id FROM email_attachments ea WHERE email_id=?1 AND is_inline=0 AND file_asset_id IS NOT NULL AND ( \
           (COALESCE(filename,'')!='' AND EXISTS (SELECT 1 FROM emails e WHERE e.id=?1 AND instr(lower(COALESCE(e.body_html,'')), 'cid:'||lower(ea.filename))>0)) \
           OR (mime_type LIKE 'image/%' AND EXISTS (SELECT 1 FROM email_attachments b WHERE b.email_id=?1 AND b.is_inline=1 AND b.file_asset_id=ea.file_asset_id)) \
         )",
        &[json!(id)],
    ).unwrap_or_default().iter()
     .filter_map(|r| r["file_asset_id"].as_str().map(String::from))
     .collect();

    // 1) 正文引用了 cid:<filename> 的附件 part（发件端未带 Content-ID 头）→ 按内嵌处理，不再显示为附件
    let n_inline = database::run(
        "UPDATE email_attachments SET is_inline=1 WHERE email_id=?1 AND is_inline=0 AND COALESCE(filename,'')!='' \
         AND EXISTS (SELECT 1 FROM emails e WHERE e.id=?1 AND instr(lower(COALESCE(e.body_html,'')), 'cid:'||lower(email_attachments.filename))>0)",
        &[json!(id)],
    ).map(|r| r.changes).unwrap_or(0);
    // 2) Outlook 双份 part：无 cid 图片附件与某内嵌附件同资产（内容哈希去重同 file_asset_id）→ 隐藏副本
    let n_dup = database::run(
        "UPDATE email_attachments SET is_inline=1 WHERE email_id=?1 AND is_inline=0 AND mime_type LIKE 'image/%' AND file_asset_id IS NOT NULL \
         AND EXISTS (SELECT 1 FROM email_attachments b WHERE b.email_id=?1 AND b.is_inline=1 AND b.file_asset_id=email_attachments.file_asset_id)",
        &[json!(id)],
    ).map(|r| r.changes).unwrap_or(0);
    if n_inline + n_dup > 0 {
        log::info!("[邮件轮询] 内嵌判定修正: cid引用文件名匹配 {} 行, 同资产副本 {} 行 (邮件 {})", n_inline, n_dup, crate::preview(&id, 8));
    }
    for aid in &to_revoke {
        crate::services::knowledge_lifecycle::notify_deleted("FILE", aid);
    }

    finalize_email_knowledge(&id, &email.from_addr, folder);
    id
}

/// 将邮件附件保存到资料库，返回 file_asset_id
/// inline=true（内嵌图）只存渲染用资产、不触发知识解析摄入（其 OCR 文字由 extract_email 并入邮件正文）
async fn save_attachment_to_library(
    raw_bytes: &[u8],
    filename: &str,
    _mime_type: &str,
    email_id: &str,
    is_inline: bool,
) -> Option<String> {
    let ext = safe_attachment_ext(filename);

    let tmp_path = std::env::temp_dir().join(format!(
        "email_att_{}.{}",
        uuid::Uuid::new_v4(),
        ext
    ));
    if let Err(e) = std::fs::write(&tmp_path, raw_bytes) {
        log::warn!("[邮件附件] 写入临时文件失败: {} - {}", filename, e);
        return None;
    }

    match crate::commands::file_commands::upload_file_core(
        &tmp_path.to_string_lossy(),
        "email",
        &[email_id],
        filename,
        "",
        "",
        None,
        !is_inline,
    )
    .await
    {
        Ok((file_id, _, _)) => {
            let _ = std::fs::remove_file(&tmp_path);
            log::info!("[邮件附件] 已存入资料库: {} → {}", filename, &file_id[..8.min(file_id.len())]);
            Some(file_id)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp_path);
            log::warn!("[邮件附件] 存入资料库失败: {} - {}", filename, e);
            None
        }
    }
}





/// 加载所有启用的账户
fn load_accounts() -> Result<Vec<EmailAccount>, String> {
    let rows = database::all(
        "SELECT id, name, email_address, imap_host, imap_port, username, password_encrypted, sync_enabled, last_sync_inbox_uid, last_sync_sent_uid FROM email_accounts WHERE sync_enabled=1",
        &[],
    )
    .map_err(|e| format!("加载邮件账户失败: {}", e))?;

    Ok(rows
        .iter()
        .map(|r| EmailAccount {
            id: r["id"].as_str().unwrap_or("").to_string(),
            name: r["name"].as_str().unwrap_or("").to_string(),
            email_address: r["email_address"].as_str().unwrap_or("").to_string(),
            imap_host: r["imap_host"].as_str().unwrap_or("").to_string(),
            imap_port: r["imap_port"].as_u64().unwrap_or(993) as u16,
            username: r["username"].as_str().unwrap_or("").to_string(),
            password_encrypted: r["password_encrypted"].as_str().unwrap_or("").to_string(),
            sync_enabled: r["sync_enabled"].as_i64().unwrap_or(1) != 0,
            last_sync_inbox_uid: r["last_sync_inbox_uid"].as_u64().unwrap_or(0) as u32,
            last_sync_sent_uid: r["last_sync_sent_uid"].as_u64().unwrap_or(0) as u32,
        })
        .collect())
}

/// 获取或探测 Sent 文件夹名称
fn get_or_detect_sent_folder(account: &EmailAccount, session: &mut email_client::ImapSession) -> String {
    let key = format!("sent_folder_{}", account.id);
    if let Some(folder) = get_config(&key) {
        return folder;
    }
    let folder = email_client::detect_sent_folder_in(session);
    set_config(&key, &folder);
    folder
}

/// 更新最后同步 UID
fn update_last_uid(account_id: &str, folder: &str, uid: u32) {
    let col = if folder == "inbox" {
        "last_sync_inbox_uid"
    } else {
        "last_sync_sent_uid"
    };
    database::run(
        &format!(
            "UPDATE email_accounts SET {}=?, updated_at=datetime('now','localtime') WHERE id=?",
            col
        ),
        &[json!(uid), json!(account_id)],
    )
    .ok();
}

fn get_config(key: &str) -> Option<String> {
    database::get(
        "SELECT value FROM email_config WHERE key=?1",
        &[json!(key)],
    )
    .ok()
    .flatten()
    .and_then(|r| r["value"].as_str().map(|s| s.to_string()))
}

fn set_config(key: &str, value: &str) {
    database::run(
        "INSERT OR REPLACE INTO email_config(key, value) VALUES(?1, ?2)",
        &[json!(key), json!(value)],
    )
    .ok();
}



/// 获取轮询状态
pub fn get_loop_status() -> Value {
    let running = LOOP_RUNNING.load(Ordering::SeqCst);
    let last_check = get_config("last_check_at").unwrap_or_default();

    let next_check = if !last_check.is_empty() {
        if let Ok(last_time) =
            chrono::NaiveDateTime::parse_from_str(&last_check, "%Y-%m-%d %H:%M")
        {
            let next = last_time + chrono::Duration::minutes(1);
            next.format("%Y-%m-%d %H:%M").to_string()
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    json!({
        "running": running,
        "last_check_at": last_check,
        "next_check_at": next_check,
        "interval_minutes": 1,
    })
}

#[cfg(test)]
mod tests {
    use super::is_automated_sender;

    #[test]
    fn automated_sender_prefixes() {
        assert!(is_automated_sender("noreply@acme.com"));
        assert!(is_automated_sender("No-Reply@x.cn"));
        assert!(is_automated_sender(" MAILER-DAEMON@mx "));
        assert!(is_automated_sender("do-not-reply@mail.io"));
        assert!(is_automated_sender("postmaster@localhost"));
    }

    #[test]
    fn human_senders_not_flagged() {
        assert!(!is_automated_sender("zhang@acme.com"));
        assert!(!is_automated_sender("sales@acme.com"));
        assert!(!is_automated_sender("notify@acme.com"));
        assert!(is_automated_sender("noreply-not-usual@x")); // 前缀语义：noreply 开头视为自动化（保守可接受）
        assert!(!is_automated_sender(""));
    }

    /// 登录失败指数退避：1→2→4→8→16 分钟，30 分钟封顶（防限频踩踏）
    #[test]
    fn backoff_grows_exponentially_and_caps() {
        assert_eq!(super::next_backoff_secs(1), 60);
        assert_eq!(super::next_backoff_secs(2), 120);
        assert_eq!(super::next_backoff_secs(3), 240);
        assert_eq!(super::next_backoff_secs(5), 960, "16 分钟");
        assert_eq!(super::next_backoff_secs(6), 1800, "封顶 30 分钟");
        assert_eq!(super::next_backoff_secs(20), 1800, "再大也封顶");
    }
}
