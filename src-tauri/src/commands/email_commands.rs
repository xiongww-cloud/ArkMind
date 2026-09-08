//! 邮件 Tauri 命令 — 账户管理、邮件列表、手动同步、AI 建议确认

use crate::database;
use crate::services::email_client;
use crate::services::email_polling;
use serde_json::{json, Value};
use std::collections::HashMap;

/// 添加 IMAP 账户
#[tauri::command]
pub async fn email_add_account(data: Value) -> Result<Value, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let name = data["name"].as_str().unwrap_or("").to_string();
    let email_address = data["emailAddress"].as_str().unwrap_or("").to_string();
    let imap_host = data["imapHost"].as_str().unwrap_or("").to_string();
    let imap_port = data["imapPort"].as_u64().unwrap_or(993) as i64;
    let username = data["username"].as_str().unwrap_or("").to_string();
    let password = data["password"].as_str().unwrap_or("").to_string();

    if name.is_empty() || email_address.is_empty() || imap_host.is_empty() || password.is_empty() {
        return Err("请填写完整的账户信息。".to_string());
    }

    // 加密密码
    let machine = crate::license::get_machine_hash();
    let password_encrypted = crate::crypto::encrypt_api_key(&password, &machine);

    database::run(
        "INSERT INTO email_accounts(id, name, email_address, imap_host, imap_port, username, password_encrypted) VALUES(?1,?2,?3,?4,?5,?6,?7)",
        &[
            json!(id), json!(name), json!(email_address), json!(imap_host),
            json!(imap_port), json!(username), json!(password_encrypted),
        ],
    ).map_err(|e| e.to_string())?;

    Ok(json!({"success": true, "id": id}))
}

/// 列出所有账户
#[tauri::command]
pub async fn email_list_accounts() -> Result<Value, String> {
    let rows = database::all(
        "SELECT id, name, email_address, imap_host, imap_port, username, sync_enabled, last_sync_at FROM email_accounts ORDER BY created_at DESC",
        &[],
    ).map_err(|e| e.to_string())?;

    Ok(json!({"success": true, "accounts": rows}))
}

/// 更新账户
#[tauri::command]
pub async fn email_update_account(id: String, data: Value) -> Result<Value, String> {
    let mut fields = Vec::new();
    let mut values = Vec::new();

    if let Some(v) = data["name"].as_str() {
        fields.push("name=?".to_string());
        values.push(json!(v));
    }
    if let Some(v) = data["emailAddress"].as_str() {
        fields.push("email_address=?".to_string());
        values.push(json!(v));
    }
    if let Some(v) = data["imapHost"].as_str() {
        fields.push("imap_host=?".to_string());
        values.push(json!(v));
    }
    if let Some(v) = data["imapPort"].as_u64() {
        fields.push("imap_port=?".to_string());
        values.push(json!(v));
    }
    if let Some(v) = data["username"].as_str() {
        fields.push("username=?".to_string());
        values.push(json!(v));
    }
    if let Some(v) = data["password"].as_str() {
        if !v.is_empty() {
            let machine = crate::license::get_machine_hash();
            let encrypted = crate::crypto::encrypt_api_key(v, &machine);
            fields.push("password_encrypted=?".to_string());
            values.push(json!(encrypted));
        }
    }
    if let Some(v) = data["syncEnabled"].as_bool() {
        fields.push("sync_enabled=?".to_string());
        values.push(json!(if v { 1 } else { 0 }));
    }

    if fields.is_empty() {
        return Ok(json!({"success": true}));
    }

    fields.push("updated_at=datetime('now','localtime')".to_string());
    values.push(json!(id));

    database::run(
        &format!("UPDATE email_accounts SET {} WHERE id=?", fields.join(",")),
        &values,
    )
    .map_err(|e| e.to_string())?;

    Ok(json!({"success": true}))
}

/// 删除账户
#[tauri::command]
pub async fn email_delete_account(id: String) -> Result<Value, String> {
    database::run("DELETE FROM email_attachments WHERE email_id IN (SELECT id FROM emails WHERE account_id=?)", &[json!(&id)]).ok();
    database::run("DELETE FROM emails WHERE account_id=?", &[json!(&id)]).ok();
    database::run("DELETE FROM email_accounts WHERE id=?", &[json!(&id)]).map_err(|e| e.to_string())?;
    Ok(json!({"success": true}))
}

/// 列出邮件
#[tauri::command]
pub async fn email_list(
    account_id: Option<String>,
    folder: Option<String>,
    category: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Value, String> {
    let mut conditions: Vec<String> = Vec::new();
    let mut values: Vec<Value> = Vec::new();

    if let Some(aid) = &account_id {
        conditions.push("account_id=?".to_string());
        values.push(json!(aid));
    }
    if let Some(f) = &folder {
        conditions.push("folder=?".to_string());
        values.push(json!(f));
    }
    if let Some(c) = &category {
        conditions.push("ai_category=?".to_string());
        values.push(json!(c));
    }

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    };
    let limit = limit.unwrap_or(50).min(100);
    let offset = offset.unwrap_or(0);

    let sql = format!(
        "SELECT id, account_id, folder, from_addr, from_name, to_addr, subject, received_at, has_attachments, ai_summary, ai_category, created_at FROM emails {} ORDER BY created_at DESC LIMIT {} OFFSET {}",
        where_clause, limit, offset
    );

    let rows = database::all(&sql, &values).map_err(|e| e.to_string())?;

    // 获取总数
    let count_sql = format!("SELECT COUNT(*) as cnt FROM emails {}", where_clause);
    let total = database::get(&count_sql, &values)
        .ok()
        .flatten()
        .and_then(|r| r["cnt"].as_i64())
        .unwrap_or(0);

    Ok(json!({"success": true, "emails": rows, "total": total}))
}

/// 获取邮件详情
#[tauri::command]
pub async fn email_get_detail(id: String) -> Result<Value, String> {
    let email = database::get(
        "SELECT * FROM emails WHERE id=?",
        &[json!(id)],
    )
    .map_err(|e| e.to_string())?
    .ok_or("未找到邮件")?;

    let attachments = database::all(
        "SELECT id, filename, mime_type, size, file_asset_id, is_inline FROM email_attachments WHERE email_id=?",
        &[json!(id)],
    )
    .unwrap_or_default();

    Ok(json!({"success": true, "email": email, "attachments": attachments}))
}

/// 手动同步
#[tauri::command]
pub async fn email_sync(account_id: Option<String>) -> Result<Value, String> {
    let accounts = if let Some(aid) = account_id {
        let row = database::get(
            "SELECT id, name, email_address, imap_host, imap_port, username, password_encrypted, sync_enabled, last_sync_inbox_uid, last_sync_sent_uid FROM email_accounts WHERE id=?",
            &[json!(aid)],
        ).map_err(|e| e.to_string())?.ok_or("未找到账户")?;
        vec![row]
    } else {
        database::all(
            "SELECT id, name, email_address, imap_host, imap_port, username, password_encrypted, sync_enabled, last_sync_inbox_uid, last_sync_sent_uid FROM email_accounts WHERE sync_enabled=1",
            &[],
        ).map_err(|e| e.to_string())?
    };

    let mut total_synced = 0;
    for row in &accounts {
        let account = email_client::EmailAccount {
            id: row["id"].as_str().unwrap_or("").to_string(),
            name: row["name"].as_str().unwrap_or("").to_string(),
            email_address: row["email_address"].as_str().unwrap_or("").to_string(),
            imap_host: row["imap_host"].as_str().unwrap_or("").to_string(),
            imap_port: row["imap_port"].as_u64().unwrap_or(993) as u16,
            username: row["username"].as_str().unwrap_or("").to_string(),
            password_encrypted: row["password_encrypted"].as_str().unwrap_or("").to_string(),
            sync_enabled: true,
            last_sync_inbox_uid: row["last_sync_inbox_uid"].as_u64().unwrap_or(0) as u32,
            last_sync_sent_uid: row["last_sync_sent_uid"].as_u64().unwrap_or(0) as u32,
        };

        // 同步收件箱（新账户跳过历史邮件）
        if account.last_sync_inbox_uid == 0 {
            if let Ok(max_uid) = email_client::get_current_max_uid(&account, "INBOX") {
                database::run(
                    "UPDATE email_accounts SET last_sync_inbox_uid=?, updated_at=datetime('now','localtime') WHERE id=?",
                    &[json!(max_uid), json!(account.id)],
                ).ok();
            }
        } else if let Ok(emails) = email_client::fetch_new_emails(&account, "INBOX", account.last_sync_inbox_uid) {
            let count = emails.len();
            if count > 0 {
                let max_uid = emails.iter().map(|e| e.uid).max().unwrap_or(0);
                for email in &emails {
                    store_email_sync(&account.id, "INBOX", email).await;
                }
                database::run(
                    "UPDATE email_accounts SET last_sync_inbox_uid=?, updated_at=datetime('now','localtime') WHERE id=?",
                    &[json!(max_uid), json!(account.id)],
                ).ok();
                total_synced += count;
            }
        }
    }

    Ok(json!({"success": true, "synced": total_synced}))
}

/// 获取轮询状态
#[tauri::command]
pub async fn email_loop_status() -> Result<Value, String> {
    Ok(email_polling::get_loop_status())
}

/// 更新轮询配置
#[tauri::command]
pub async fn email_config_update(data: Value) -> Result<Value, String> {
    if let Some(obj) = data.as_object() {
        for (key, val) in obj {
            let v_str = val.as_str().unwrap_or(&val.to_string()).to_string();
            database::run(
                "INSERT OR REPLACE INTO email_config(key, value) VALUES(?1, ?2)",
                &[json!(key), json!(v_str)],
            ).ok();
        }
    }
    Ok(json!({"success": true}))
}

/// 获取轮询配置
#[tauri::command]
pub async fn email_config_get() -> Result<Value, String> {
    let rows = database::all("SELECT key, value FROM email_config", &[])
        .map_err(|e| e.to_string())?;
    let mut config = serde_json::Map::new();
    for row in &rows {
        let key = row["key"].as_str().unwrap_or("");
        let value = row["value"].as_str().unwrap_or("");
        config.insert(key.to_string(), json!(value));
    }
    Ok(json!(config))
}

/// 获取邮件统计
#[tauri::command]
pub async fn email_stats() -> Result<Value, String> {
    let total = database::get("SELECT COUNT(*) as cnt FROM emails", &[])
        .ok().flatten().and_then(|r| r["cnt"].as_i64()).unwrap_or(0);
    let knowledge = database::get("SELECT COUNT(*) as cnt FROM emails WHERE ai_category='knowledge'", &[])
        .ok().flatten().and_then(|r| r["cnt"].as_i64()).unwrap_or(0);

    Ok(json!({
        "success": true,
        "total": total,
        "knowledge": knowledge,
    }))
}

/// 存储邮件（异步版本，供手动同步使用）
async fn store_email_sync(account_id: &str, folder: &str, email: &email_client::ParsedEmail) {
    let id = uuid::Uuid::new_v4().to_string();
    database::run(
        "INSERT OR IGNORE INTO emails(id, account_id, folder, uid, message_id, from_addr, from_name, to_addr, subject, body_text, body_html, received_at, has_attachments) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
        &[
            json!(id), json!(account_id), json!(folder), json!(email.uid),
            json!(email.message_id), json!(email.from_addr), json!(email.from_name),
            json!(email.to_addr), json!(email.subject), json!(email.body_text),
            json!(email.body_html), json!(email.received_at), json!(email.has_attachments),
        ],
    ).ok();

    for att in &email.attachments {
        let att_id = uuid::Uuid::new_v4().to_string();

        // 将附件原始字节写入临时文件，然后存入资料库
        let file_asset_id = if !att.raw_bytes.is_empty() {
            save_attachment_to_library(&att.raw_bytes, &att.filename, &att.mime_type, &id).await
        } else {
            None
        };

        database::run(
            "INSERT INTO email_attachments(id, email_id, filename, mime_type, size, parsed_text, file_asset_id) VALUES(?1,?2,?3,?4,?5,?6,?7)",
            &[json!(att_id), json!(id), json!(att.filename), json!(att.mime_type), json!(att.size), json!(att.parsed_text), json!(file_asset_id)],
        ).ok();
    }
}

/// 将邮件附件保存到资料库，返回 file_asset_id
async fn save_attachment_to_library(
    raw_bytes: &[u8],
    filename: &str,
    _mime_type: &str,
    email_id: &str,
) -> Option<String> {
    // 从 mime_type 或文件名推断扩展名
    let ext = filename
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_lowercase();

    // 写入临时文件
    let tmp_path = std::env::temp_dir().join(format!(
        "email_att_{}.{}",
        uuid::Uuid::new_v4(),
        ext
    ));
    if let Err(e) = std::fs::write(&tmp_path, raw_bytes) {
        log::warn!("[邮件附件] 写入临时文件失败: {} - {}", filename, e);
        return None;
    }

    // 调用 upload_file_core 存入资料库
    match crate::commands::file_commands::upload_file_core(
        &tmp_path.to_string_lossy(),
        "email",
        &[email_id],
        filename,
        "",
        "",
        None,
    )
    .await
    {
        Ok((file_id, _)) => {
            // 清理临时文件
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

// ═══════════════════════════════════════════════════════════════════
// AI 建议 → 用户确认后才创建业务对象
// ═══════════════════════════════════════════════════════════════════





fn normalize_priority(p: &str) -> String {
    let up = p.trim().to_uppercase();
    match up.as_str() {
        "P1" | "高" | "紧急" | "HIGH" => "P1".to_string(),
        "P2" | "中" | "重要" | "MEDIUM" => "P2".to_string(),
        _ => "P3".to_string(),
    }
}

fn resolve_project_id(ref_name: &str) -> Option<String> {
    if ref_name.is_empty() {
        return None;
    }
    // 优先按名称精确匹配
    if let Some(r) = database::get("SELECT id FROM projects WHERE name=?1 LIMIT 1", &[json!(ref_name)])
        .ok()
        .flatten()
    {
        if let Some(pid) = r["id"].as_str() {
            return Some(pid.to_string());
        }
    }
    // 回退：若填的是 ID 则直接命中
    database::get("SELECT id FROM projects WHERE id=?1 LIMIT 1", &[json!(ref_name)])
        .ok()
        .flatten()
        .and_then(|r| r["id"].as_str().map(String::from))
}

/// 把该邮件的附件关联到新创建的对象（todo/project 等支持文件关联的实体）
fn link_email_attachments(email_id: &str, entity_type: &str, entity_id: &str) -> usize {
    let rows = database::all(
        "SELECT file_asset_id, filename FROM email_attachments \
         WHERE email_id=?1 AND file_asset_id IS NOT NULL AND file_asset_id != ''",
        &[json!(email_id)],
    )
    .unwrap_or_default();
    let mut linked = 0;
    for r in &rows {
        if let (Some(fid), Some(fname)) = (r["file_asset_id"].as_str(), r["filename"].as_str()) {
            crate::commands::file_commands::add_relation(fid, entity_type, entity_id, fname, "", None);
            linked += 1;
        }
    }
    linked
}

fn persist_suggestion_meta(notification_id: &str, items: &[Value]) {
    let all_resolved = items
        .iter()
        .all(|s| s["status"].as_str().map(|x| x != "pending").unwrap_or(true));
    database::run(
        "UPDATE notifications SET meta=?1, read=?2 WHERE id=?3",
        &[
            json!(serde_json::to_string(items).unwrap_or_else(|_| "[]".to_string())),
            json!(if all_resolved { 1 } else { 0 }),
            json!(notification_id),
        ],
    )
    .ok();
}

/// 根据建议条目确定式创建业务对象，返回创建结果
async fn apply_suggestion(email_id: &str, s: &Value) -> Result<Value, String> {
    let kind = s["kind"].as_str().unwrap_or("");
    match kind {
        "todo" => {
            let mut create = json!({});
            create["title"] = json!(s["title"].as_str().unwrap_or("未命名待办"));
            if let Some(c) = s["content"].as_str().filter(|x| !x.is_empty()) {
                create["content"] = json!(c);
            }
            if let Some(d) = s["due_date"].as_str().filter(|x| !x.is_empty()) {
                create["dueDate"] = json!(d);
            }
            if let Some(p) = s["priority"].as_str().filter(|x| !x.is_empty()) {
                create["priority"] = json!(normalize_priority(p));
            }
            let mut note = String::new();
            if let Some(ref_name) = s["project_ref"].as_str().filter(|x| !x.is_empty()) {
                if let Some(pid) = resolve_project_id(ref_name) {
                    create["projectId"] = json!(pid);
                } else {
                    note = format!("（未找到关联项目“{}”，已作为独立待办创建）", ref_name);
                }
            }
            let res = crate::commands::todo_commands::todos_create(create)
                .await
                .map_err(|e| e.to_string())?;
            let oid = res["todo"]["id"].as_str().unwrap_or("").to_string();
            let oname = res["todo"]["title"].as_str().unwrap_or("").to_string();
            let linked = link_email_attachments(email_id, "todo", &oid);
            Ok(json!({
                "object_type": "todo", "object_id": oid, "object_name": oname,
                "note": note, "linked_files": linked
            }))
        }
        "project" => {
            let mut create = json!({});
            create["name"] = json!(s["name"].as_str().unwrap_or("新项目"));
            if let Some(d) = s["description"].as_str().filter(|x| !x.is_empty()) {
                create["description"] = json!(d);
            }
            if let Some(b) = s["budget"].as_f64() {
                create["budget"] = json!(b);
            } else if let Some(bs) = s["budget"].as_str().and_then(|v| v.parse::<f64>().ok()) {
                create["budget"] = json!(bs);
            }
            if let Some(bt) = s["bid_time"].as_str().filter(|x| !x.is_empty()) {
                create["status"] = json!("Bidding");
                create["bidTime"] = json!(bt);
            } else {
                create["status"] = json!("Lead");
            }
            let res = crate::commands::project_commands::project_create(create)
                .await
                .map_err(|e| e.to_string())?;
            let oid = res["id"].as_str().unwrap_or("").to_string();
            let oname = res["name"].as_str().unwrap_or("").to_string();
            let linked = link_email_attachments(email_id, "project", &oid);
            Ok(json!({
                "object_type": "project", "object_id": oid, "object_name": oname,
                "note": "", "linked_files": linked
            }))
        }
        "partner" => {
            let mut create = json!({});
            create["type"] = json!(s["type"].as_str().unwrap_or("CUSTOMER"));
            create["name"] = json!(s["name"].as_str().unwrap_or(""));
            if let Some(p) = s["phone"].as_str().filter(|x| !x.is_empty()) {
                create["phone"] = json!(p);
            }
            if let Some(c) = s["company"].as_str().filter(|x| !x.is_empty()) {
                create["company"] = json!(c);
            }
            if let Some(cp) = s["contact_person"].as_str().filter(|x| !x.is_empty()) {
                create["contact_person"] = json!(cp);
            }
            if let Some(cph) = s["contact_phone"].as_str().filter(|x| !x.is_empty()) {
                create["contact_phone"] = json!(cph);
            }
            if let Some(r) = s["remark"].as_str().filter(|x| !x.is_empty()) {
                create["remark"] = json!(r);
            }
            let res = crate::commands::partner_commands::partner_create(create)
                .await
                .map_err(|e| e.to_string())?;
            let oid = res["id"].as_str().unwrap_or("").to_string();
            let oname = res["name"].as_str().unwrap_or("").to_string();
            Ok(json!({
                "object_type": "partner", "object_id": oid, "object_name": oname,
                "note": "", "linked_files": 0
            }))
        }
        _ => Err(format!("不支持的建议类型: {}", kind)),
    }
}

/// 用户确认某条建议 → 创建对应业务对象并关联附件
#[tauri::command]
pub async fn email_action_confirm(notification_id: String, suggestion_id: String) -> Result<Value, String> {
    let row = database::get(
        "SELECT * FROM notifications WHERE id=?1",
        &[json!(&notification_id)],
    )
    .map_err(|e| e.to_string())?
    .ok_or_else(|| "通知不存在".to_string())?;
    let email_id = row["entity_id"].as_str().unwrap_or("").to_string();
    let meta_raw = row["meta"].as_str().unwrap_or("[]");
    let mut items: Vec<Value> = serde_json::from_str(meta_raw).unwrap_or_default();
    let pos = items
        .iter()
        .position(|s| s["id"].as_str() == Some(suggestion_id.as_str()))
        .ok_or_else(|| "建议不存在".to_string())?;

    if items[pos]["status"].as_str().map(|x| x != "pending").unwrap_or(true) {
        let result = items[pos]["result"].clone();
        return Ok(json!({
            "success": true, "items": items,
            "object": result,
            "note": "该建议已处理"
        }));
    }

    let suggestion = items[pos].clone();
    let created = apply_suggestion(&email_id, &suggestion).await?;

    items[pos]["status"] = json!("done");
    items[pos]["result"] = json!({
        "type": created["object_type"],
        "id": created["object_id"],
        "name": created["object_name"]
    });
    if let Some(note) = created["note"].as_str().filter(|n| !n.is_empty()) {
        items[pos]["note"] = json!(note);
    }
    persist_suggestion_meta(&notification_id, &items);

    // 记录到邮件 ai_actions
    let existing_actions = database::get("SELECT ai_actions FROM emails WHERE id=?1", &[json!(&email_id)])
        .ok().flatten().and_then(|r| r["ai_actions"].as_str().map(String::from)).unwrap_or_default();
    let mut actions: Value = serde_json::from_str(&existing_actions).unwrap_or(json!({}));
    let mut created_list = actions["created"].as_array().cloned().unwrap_or_default();
    created_list.push(json!({
        "kind": created["object_type"],
        "id": created["object_id"],
        "name": created["object_name"]
    }));
    actions["created"] = json!(created_list);
    database::run(
        "UPDATE emails SET ai_actions=?1 WHERE id=?2",
        &[json!(actions.to_string()), json!(&email_id)],
    ).ok();

    Ok(json!({
        "success": true,
        "items": items,
        "object": json!({
            "type": created["object_type"],
            "id": created["object_id"],
            "name": created["object_name"]
        }),
        "note": created["note"],
        "linked_files": created["linked_files"]
    }))
}

/// 用户忽略某条建议
#[tauri::command]
pub async fn email_action_ignore(notification_id: String, suggestion_id: String) -> Result<Value, String> {
    let row = database::get(
        "SELECT * FROM notifications WHERE id=?1",
        &[json!(&notification_id)],
    )
    .map_err(|e| e.to_string())?
    .ok_or_else(|| "通知不存在".to_string())?;
    let meta_raw = row["meta"].as_str().unwrap_or("[]");
    let mut items: Vec<Value> = serde_json::from_str(meta_raw).unwrap_or_default();
    let pos = items
        .iter()
        .position(|s| s["id"].as_str() == Some(suggestion_id.as_str()))
        .ok_or_else(|| "建议不存在".to_string())?;
    if items[pos]["status"].as_str().map(|x| x != "pending").unwrap_or(true) {
        return Ok(json!({"success": true, "items": items, "note": "该建议已处理"}));
    }
    items[pos]["status"] = json!("ignored");
    persist_suggestion_meta(&notification_id, &items);
    Ok(json!({"success": true, "items": items}))
}

/// 重新处理历史邮件的知识库数据
/// 将所有已分类为 knowledge 的邮件重新提取内容并写入 source_data 表
#[tauri::command]
pub async fn email_reprocess_knowledge() -> Result<Value, String> {
    // 查询所有需要重新处理的邮件
    let rows = database::all(
        "SELECT e.id FROM emails e WHERE e.ai_category='knowledge'",
        &[],
    ).map_err(|e| format!("查询邮件失败: {}", e))?;

    let total = rows.len();
    let mut processed = 0;
    let mut errors = 0;

    for row in &rows {
        let id = row["id"].as_str().unwrap_or("").to_string();
        if id.is_empty() {
            continue;
        }

        // 使用新的提取逻辑（优先使用 HTML 转 Markdown）
        let (title, content) = crate::services::source_doc_sync::extract_email(&id);
        if content.is_empty() {
            continue;
        }

        // 更新或插入 source_data
        let result = database::run(
            "INSERT INTO source_data(source_type, source_id, title, content, updated_at) \
             VALUES('EMAIL', ?1, ?2, ?3, datetime('now','localtime')) \
             ON CONFLICT(source_type, source_id) \
             DO UPDATE SET title=excluded.title, content=excluded.content, updated_at=excluded.updated_at",
            &[json!(&id), json!(&title), json!(&content)],
        );

        match result {
            Ok(_) => {
                processed += 1;
                log::info!("[邮件迁移] 已重新处理邮件 {}", &id[..id.len().min(8)]);
            }
            Err(e) => {
                errors += 1;
                log::warn!("[邮件迁移] 处理邮件 {} 失败: {}", &id[..id.len().min(8)], e);
            }
        }
    }

    // 触发知识管线重新同步
    crate::services::knowledge_lifecycle::reset_pipeline();

    Ok(json!({
        "success": true,
        "total": total,
        "processed": processed,
        "errors": errors,
        "message": format!("已重新处理 {}/{} 封邮件，{} 个错误", processed, total, errors)
    }))
}

/// 重新处理历史邮件的附件和 cid: 引用
/// 将所有邮件的内嵌图片 Content-ID 替换为 API URL
#[tauri::command]
pub async fn email_reprocess_attachments() -> Result<Value, String> {
    // 查询所有邮件
    let emails = database::all(
        "SELECT id, body_html FROM emails",
        &[],
    ).map_err(|e| format!("查询邮件失败: {}", e))?;

    let total = emails.len();
    let mut processed = 0;
    let mut errors = 0;

    for email in &emails {
        let email_id = email["id"].as_str().unwrap_or("");
        let body_html = email["body_html"].as_str().unwrap_or("");

        // 查询该邮件的所有附件
        let attachments = database::all(
            "SELECT id, file_asset_id, content_id FROM email_attachments WHERE email_id=?1",
            &[json!(email_id)],
        ).unwrap_or_default();

        // 构建 cid: → file_asset_id 映射
        let mut cid_to_asset: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        for att in &attachments {
            if let (Some(cid), Some(asset_id)) = (
                att["content_id"].as_str(),
                att["file_asset_id"].as_str(),
            ) {
                if !cid.is_empty() && !asset_id.is_empty() {
                    cid_to_asset.insert(cid.to_string(), asset_id.to_string());
                }
            }
        }

        // 替换 HTML 中的 cid: 引用
        if !cid_to_asset.is_empty() {
            let mut new_html = body_html.to_string();
            for (cid, asset_id) in &cid_to_asset {
                // 查询文件的实际路径
                let file_path = database::get("SELECT content_hash FROM file_assets WHERE id=?", &[serde_json::json!(asset_id)])
                    .ok().flatten()
                    .and_then(|r| r["content_hash"].as_str().map(|h| crate::commands::file_commands::flat_disk_path(h)));
                
                if let Some(path) = file_path {
                    let api_url = format!("https://asset.localhost/{}", path.replace("\\", "/"));
                    let cid_clean = cid.trim_matches(|c| c == '<' || c == '>');
                    let cid_ref1 = format!("cid:{}", cid_clean);
                    let cid_ref2 = format!("cid:<{}>", cid_clean);
                    new_html = new_html.replace(&cid_ref1, &api_url);
                    new_html = new_html.replace(&cid_ref2, &api_url);
                }
            }

            // 更新数据库
            match database::run(
                "UPDATE emails SET body_html=?1 WHERE id=?2",
                &[json!(new_html), json!(email_id)],
            ) {
                Ok(_) => {
                    processed += 1;
                    log::info!("[邮件迁移] 已重新处理邮件 {}", &email_id[..email_id.len().min(8)]);
                }
                Err(e) => {
                    errors += 1;
                    log::warn!("[邮件迁移] 处理邮件 {} 失败: {}", &email_id[..email_id.len().min(8)], e);
                }
            }
        }
    }

    // 触发知识管线重新同步
    crate::services::knowledge_lifecycle::reset_pipeline();

    Ok(json!({
        "success": true,
        "total": total,
        "processed": processed,
        "errors": errors,
        "message": format!("已重新处理 {}/{} 封邮件的附件引用，{} 个错误", processed, total, errors)
    }))
}

// ═══════════════════════════════════════════════════════════════════
// 邮件正文渲染：把内嵌本地图片以 data URL 内联，规避自定义协议问题
// ═══════════════════════════════════════════════════════════════════

fn email_asset_path_by_uuid(uuid: &str) -> Option<String> {
    let hash = database::get(
        "SELECT content_hash FROM file_assets WHERE id=?1",
        &[json!(uuid)],
    )
    .ok()
    .flatten()
    .and_then(|r| r["content_hash"].as_str().map(String::from))
    .filter(|h| !h.is_empty())?;
    let p = crate::commands::file_commands::flat_disk_path(&hash);
    if p.is_empty() || !std::path::Path::new(&p).exists() { None } else { Some(p) }
}

fn email_asset_path_by_hash(hash: &str) -> Option<String> {
    if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let p = crate::commands::file_commands::flat_disk_path(hash);
    if p.is_empty() || !std::path::Path::new(&p).exists() { None } else { Some(p) }
}

fn email_is_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36
        && b[8] == b'-' && b[13] == b'-' && b[18] == b'-' && b[23] == b'-'
        && b.iter().enumerate().all(|(i, &c)| {
            [8usize, 13, 18, 23].contains(&i) || c.is_ascii_hexdigit()
        })
}

fn email_build_data_url(path: &str) -> Option<String> {
    let buf = std::fs::read(path).ok()?;
    if buf.is_empty() || buf.len() > 25 * 1024 * 1024 {
        return None;
    }
    let mime = infer::get(&buf)
        .map(|t| t.mime_type().to_string())
        .unwrap_or_else(|| "application/octet-stream".to_string());
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&buf);
    Some(format!("data:{};base64,{}", mime, b64))
}

/// 内嵌图片总字节 ≤ 该阈值时一次性内联（快速路径），超过则改为懒加载占位
const EMAIL_EAGER_THRESHOLD: i64 = 256 * 1024;

/// 附件 content_id(归一) → file_asset_id(uuid)
fn email_cid_map(email_id: &str) -> HashMap<String, String> {
    let mut m: HashMap<String, String> = HashMap::new();
    if let Ok(atts) = database::all(
        "SELECT content_id, file_asset_id FROM email_attachments WHERE email_id=?1",
        &[json!(email_id)],
    ) {
        for a in &atts {
            let cid = a["content_id"].as_str().map(|s| s.trim_matches(|c: char| c == '<' || c == '>' || c.is_whitespace()).to_string()).unwrap_or_default();
            let uuid = a["file_asset_id"].as_str().unwrap_or("").to_string();
            if !cid.is_empty() && !uuid.is_empty() {
                m.entry(cid).or_insert(uuid);
            }
        }
    }
    m
}

/// 把本地内嵌图片 src 归一到规范 token（file 的 uuid 或 64 位 content_hash）。
/// 无法识别 / 外部 http(s) / data: 返回 None。
fn email_src_token(src: &str, cid_to_uuid: &HashMap<String, String>) -> Option<String> {
    let s = src.trim();
    if s.is_empty() || s.starts_with("data:") {
        return None;
    }
    let lower = s.to_ascii_lowercase();
    let external_http = (lower.starts_with("http://") || lower.starts_with("https://")) && !lower.contains("asset.localhost");
    if external_http {
        return None;
    }
    if lower.starts_with("cid:") {
        let tok = s[4..].trim_matches(|c: char| c == '<' || c == '>' || c.is_whitespace()).to_string();
        return cid_to_uuid.get(&tok).cloned();
    }
    if lower.starts_with("pms://file/asset/") {
        let tok = s["pms://file/asset/".len()..].trim_end_matches("/content").to_string();
        return if email_is_uuid(&tok) { Some(tok) } else { None };
    }
    if lower.starts_with("asset://") || s.contains("asset.localhost/") {
        let tail = s.split('?').next().unwrap_or(s).rsplit('/').next().unwrap_or("").to_string();
        if email_is_uuid(&tail) {
            return Some(tail);
        }
        if tail.len() == 64 && tail.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Some(tail);
        }
        return None;
    }
    None
}

/// 解析 src → 直接内联 data URL
fn email_src_data_url(src: &str, cid_to_uuid: &HashMap<String, String>) -> Option<String> {
    let token = email_src_token(src, cid_to_uuid)?;
    if email_is_uuid(&token) {
        let p = email_asset_path_by_uuid(&token)?;
        return email_build_data_url(&p);
    }
    let p = email_asset_path_by_hash(&token)?;
    email_build_data_url(&p)
}

/// 重写 HTML 中的 <img src>：
/// - eager=true：本地图片替换为 data URL；
/// - eager=false（懒加载）：src 置空并追加 data-emimg="<token>"，保留其余属性。
fn rewrite_email_imgs(html: &str, cid_to_uuid: &HashMap<String, String>, eager: bool) -> String {
    fn pass(html: &str, re: &regex::Regex, cid_to_uuid: &HashMap<String, String>, eager: bool) -> String {
        let mut out = String::with_capacity(html.len() + 1024);
        let mut last = 0usize;
        for cap in re.captures_iter(html) {
            let m0 = cap.get(0).unwrap();
            out.push_str(&html[last..m0.start()]);
            let val = cap.get(2).unwrap().as_str();
            if eager {
                if let Some(data) = email_src_data_url(val, cid_to_uuid) {
                    out.push_str(&cap[1]);
                    out.push_str(&data);
                    out.push_str(&cap[3]);
                } else {
                    out.push_str(&cap[0]);
                }
            } else if let Some(token) = email_src_token(val, cid_to_uuid) {
                out.push_str(&cap[1]);
                out.push_str("");
                out.push_str(&cap[3]);
                out.push_str(&format!(" data-emimg=\"{}\"", token));
            } else {
                out.push_str(&cap[0]);
            }
            last = m0.end();
        }
        out.push_str(&html[last..]);
        out
    }

    let re_dq = regex::Regex::new(r#"(?i)(\bsrc\s*=\s*")([^"]*)(")"#).unwrap();
    let re_sq = regex::Regex::new(r#"(?i)(\bsrc\s*=\s*')([^']*)(')"#).unwrap();
    let out = pass(html, &re_dq, cid_to_uuid, eager);
    let out = pass(&out, &re_sq, cid_to_uuid, eager);
    out
}

/// 渲染邮件正文。小邮件（内嵌图片 ≤256KB）直接内联 data URL；
/// 大邮件改为懒加载占位（lazy=true），由前端按需调用 email_inline_asset 逐图拉取。
/// 外部 http(s) 图片原样保留。
#[tauri::command]
pub async fn email_render_html(id: String) -> Result<Value, String> {
    let email = database::get(
        "SELECT body_html FROM emails WHERE id=?1",
        &[json!(id)],
    )
    .map_err(|e| e.to_string())?
    .ok_or("未找到邮件")?;
    let body_html = email["body_html"].as_str().unwrap_or("").to_string();

    let cid_to_uuid = email_cid_map(&id);

    let total_inline: i64 = database::all(
        "SELECT COALESCE(SUM(size),0) AS total FROM email_attachments WHERE email_id=?1 AND is_inline=1",
        &[json!(&id)],
    )
    .ok()
    .and_then(|rows| rows.first().and_then(|r| {
        r["total"].as_i64()
            .or_else(|| r["total"].as_u64().map(|v| v as i64))
            .or_else(|| r["total"].as_str().and_then(|s| s.parse().ok()))
    }))
    .unwrap_or(0);

    let lazy = total_inline > EMAIL_EAGER_THRESHOLD;
    let out = rewrite_email_imgs(&body_html, &cid_to_uuid, !lazy);

    Ok(json!({"success": true, "html": out, "lazy": lazy}))
}

/// 懒加载：按 token（file 的 uuid / content_hash / cid）解析单张内嵌图片并返回 data URL
#[tauri::command]
pub async fn email_inline_asset(email_id: String, token: String) -> Result<Value, String> {
    let cid_to_uuid = email_cid_map(&email_id);
    let t = token.trim();
    let data_url: Option<String> = if email_is_uuid(t) {
        email_asset_path_by_uuid(t).and_then(|p| email_build_data_url(&p))
    } else if t.len() == 64 && t.bytes().all(|b| b.is_ascii_hexdigit()) {
        email_asset_path_by_hash(t).and_then(|p| email_build_data_url(&p))
    } else {
        let key = t.trim_matches(|c: char| c == '<' || c == '>' || c.is_whitespace());
        cid_to_uuid.get(key).and_then(|u| email_asset_path_by_uuid(u)).and_then(|p| email_build_data_url(&p))
    };
    match data_url {
        Some(u) => Ok(json!({"success": true, "dataUrl": u})),
        None => Ok(json!({"success": false, "error": "资源不存在"})),
    }
}
