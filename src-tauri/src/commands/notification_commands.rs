use crate::database;
use serde_json::{json, Value};

/// 剥除通知标题/内容中暴露的对象 UUID，避免泄露内部标识符；标题强制单行化（合并换行为空格）
pub(crate) fn sanitize_notification_text(text: &str) -> String {
    let re = regex::Regex::new(r"(?i)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}").unwrap();
    let cleaned = re.replace_all(text, "").to_string();
    cleaned.replace('\r', "").replace('\n', " ").split_whitespace().collect::<Vec<_>>().join(" ")
}

#[tauri::command]
pub async fn notification_list(unread_only: Option<bool>) -> Result<Value, String> {
    let sql = if unread_only.unwrap_or(false) {
        "SELECT * FROM notifications WHERE read=0 ORDER BY created_at DESC"
    } else {
        "SELECT * FROM notifications ORDER BY read ASC, created_at DESC"
    };
    let mut rows = database::all(sql, &[]).map_err(|e| e.to_string())?;
    // 将 actions 从 JSON 字符串解析为数组
    for row in rows.iter_mut() {
        if let Some(actions_str) = row.get("actions").and_then(|v| v.as_str()) {
            if let Ok(parsed) = serde_json::from_str::<Value>(actions_str) {
                row["actions"] = parsed;
            }
        }
    }
    Ok(json!({ "success": true, "notifications": rows }))
}

#[tauri::command]
pub async fn notification_create(data: Value) -> Result<Value, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let ntype = data["type"].as_str().unwrap_or("llm_insight");
    let title = sanitize_notification_text(data["title"].as_str().unwrap_or(""));
    let content = sanitize_notification_text(data["content"].as_str().unwrap_or(""));
    let entity_type = data["entity_type"].as_str().unwrap_or("");
    let entity_id = data["entity_id"].as_str().unwrap_or("");
    let priority = data["priority"].as_str().unwrap_or("medium");
    let source = data["source"].as_str().unwrap_or("system");
    let follow_up_of = data["follow_up_of"].as_str().unwrap_or("");
    database::run(
        "INSERT INTO notifications(id,type,title,content,entity_type,entity_id,priority,source,follow_up_of) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        &[json!(id), json!(ntype), json!(title), json!(content), json!(entity_type), json!(entity_id), json!(priority), json!(source), json!(follow_up_of)],
    ).map_err(|e| e.to_string())?;
    Ok(json!({ "success": true, "id": id }))
}

#[tauri::command]
pub async fn notification_mark_read(id: String) -> Result<(), String> {
    database::run("UPDATE notifications SET read=1 WHERE id=?", &[json!(id)]).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn notification_mark_all_read() -> Result<(), String> {
    database::run("UPDATE notifications SET read=1 WHERE read=0", &[]).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn notification_delete(id: String) -> Result<(), String> {
    database::run("DELETE FROM notifications WHERE id=?", &[json!(id)]).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn notification_batch_delete(ids: Vec<String>) -> Result<(), String> {
    if ids.is_empty() { return Ok(()); }
    let placeholders: Vec<String> = ids.iter().enumerate().map(|(i, _)| format!("?{}", i + 1)).collect();
    let sql = format!("DELETE FROM notifications WHERE id IN ({})", placeholders.join(","));
    let params: Vec<Value> = ids.into_iter().map(|id| json!(id)).collect();
    database::run(&sql, &params).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn notification_stats() -> Result<Value, String> {
    let unread: i64 = database::with_db(|c| c.query_row(
        "SELECT COUNT(*) FROM notifications WHERE read=0", [], |r| r.get(0)
    )).unwrap_or(0);
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let stored_date: String = database::with_db(|c| c.query_row(
        "SELECT value FROM notification_config WHERE key='today_date'", [], |r| r.get(0)
    )).unwrap_or_default();
    let today_calls: i64 = if stored_date == today {
        database::with_db(|c| c.query_row(
            "SELECT value FROM notification_config WHERE key='today_calls'", [], |r| r.get(0)
        )).unwrap_or(0)
    } else { 0 };
    let today_tokens: i64 = if stored_date == today {
        database::with_db(|c| c.query_row(
            "SELECT value FROM notification_config WHERE key='today_tokens'", [], |r| r.get(0)
        )).unwrap_or(0)
    } else { 0 };
    Ok(json!({
        "success": true,
        "unread_count": unread,
        "today_calls": today_calls,
        "today_tokens": today_tokens,
    }))
}

#[tauri::command]
pub async fn notification_config_get() -> Result<Value, String> {
    let rows = database::all("SELECT key, value FROM notification_config", &[]).map_err(|e| e.to_string())?;
    let mut config = serde_json::Map::new();
    for r in &rows {
        if let (Some(k), Some(v)) = (r["key"].as_str(), r["value"].as_str()) {
            config.insert(k.to_string(), json!(v));
        }
    }
    Ok(json!({ "success": true, "config": config,
        "analysis_prompt_default": crate::services::notification_loop::DEFAULT_ANALYSIS_PROMPT,
        "analyst_watch": crate::services::notification_loop::get_watch_pool() }))
}

#[tauri::command]
pub async fn notification_config_update(data: Value) -> Result<(), String> {
    if let Some(obj) = data.as_object() {
        for (k, v) in obj {
            let val = v.as_str().unwrap_or(&v.to_string()).to_string();
            // 附加分析要求（软层）：限 2000 字符；空串=删除（恢复无附加状态）
            if k == "analysis_focus" {
                if val.trim().is_empty() {
                    let _ = database::run("DELETE FROM notification_config WHERE key='analysis_focus'", &[]);
                    continue;
                }
                if val.chars().count() > 2000 {
                    return Err("附加分析要求不能超过 2000 字符".into());
                }
            }
            database::run(
                "INSERT OR REPLACE INTO notification_config(key, value) VALUES(?1, ?2)",
                &[json!(k), json!(val)],
            ).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn notification_loop_status() -> Result<Value, String> {
    let status = crate::services::notification_loop::get_loop_status();
    Ok(json!({
        "success": true,
        "running": status.running,
        "last_check_at": status.last_check_at,
        "next_check_at": status.next_check_at,
    }))
}

/// 用户点击确认按钮后的受信任落库路径（分析师本体无写权限）
fn create_confirmed_fact(content: &str, notification_id: Option<&str>, source: &str) -> Result<Value, String> {
    let c = content.trim().to_string();
    if c.is_empty() { return Err("内容不能为空".into()); }
    if c.chars().count() > 500 { return Err("内容不能超过 500 字符".into()); }
    // 去重：同源同内容已存在则直接复用（用户重复点击幂等）
    let existing: Option<String> = database::get(
        "SELECT id FROM memories WHERE content=?1 AND source=?2 ORDER BY created_at DESC LIMIT 1",
        &[json!(&c), json!(source)],
    ).ok().flatten().and_then(|r| r["id"].as_str().map(String::from));
    let id = match existing {
        Some(id) => id,
        None => {
            let id = uuid::Uuid::new_v4().to_string();
            database::run(
                "INSERT INTO memories(id, content, source, created_at, updated_at) VALUES(?1, ?2, ?3, datetime('now','localtime'), datetime('now','localtime'))",
                &[json!(&id), json!(&c), json!(source)],
            ).map_err(|e| e.to_string())?;
            id
        }
    };
    // 标记来源通知已处理（若有 notification_id）
    if let Some(nid) = notification_id.filter(|s| !s.is_empty()) {
        let _ = database::run("UPDATE notifications SET read=1 WHERE id=?1", &[json!(nid)]);
    }
    Ok(json!({ "success": true, "id": id }))
}

/// 用户从通知的 save_memory 按钮确认沉淀记忆（受信任路径，分析师本体无写权限）
#[tauri::command]
pub async fn memory_create_from_notification(content: String, notification_id: Option<String>) -> Result<Value, String> {
    create_confirmed_fact(&content, notification_id.as_deref(), "notification_confirmed")
}

/// 用户从通知的 save_profile 按钮确认沉淀用户画像事实
#[tauri::command]
pub async fn profile_fact_save(content: String, notification_id: Option<String>) -> Result<Value, String> {
    create_confirmed_fact(&content, notification_id.as_deref(), "profile")
}

/// 用户从通知的 merge_profile 按钮确认整合画像：删除零散旧条目，写入整合后的一条
#[tauri::command]
pub async fn profile_fact_merge(ids: Vec<String>, content: String, notification_id: Option<String>) -> Result<Value, String> {
    let c = content.trim().to_string();
    if c.is_empty() { return Err("整合后的画像陈述不能为空".into()); }
    if c.chars().count() > 500 { return Err("整合后的画像陈述不能超过 500 字符".into()); }
    let ids: Vec<String> = ids.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).map(String::from).collect();
    if !ids.is_empty() {
        let placeholders = vec!["?1"; ids.len()].join(",");
        let sql = format!("DELETE FROM memories WHERE source='profile' AND id IN ({})", placeholders);
        let params: Vec<Value> = ids.iter().map(|i| json!(i)).collect();
        database::run(&sql, &params).map_err(|e| e.to_string())?;
    }
    create_confirmed_fact(&c, notification_id.as_deref(), "profile")
}

/// 画像事实列表（设置页"我的档案"卡用）
#[tauri::command]
pub async fn profile_facts_list() -> Result<Value, String> {
    let rows = database::all(
        "SELECT id, content, updated_at FROM memories WHERE source='profile' ORDER BY updated_at DESC",
        &[],
    ).map_err(|e| e.to_string()).unwrap_or_default();
    Ok(json!({ "success": true, "facts": rows }))
}

/// 删除一条画像事实（仅限 source='profile'，防误删普通记忆）
#[tauri::command]
pub async fn profile_fact_delete(id: String) -> Result<Value, String> {
    database::run("DELETE FROM memories WHERE id=?1 AND source='profile'", &[json!(&id)])
        .map_err(|e| e.to_string())?;
    Ok(json!({ "success": true }))
}
