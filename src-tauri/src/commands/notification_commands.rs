use crate::database;
use serde_json::{json, Value};

#[tauri::command]
pub async fn notification_list(unread_only: Option<bool>) -> Result<Value, String> {
    let sql = if unread_only.unwrap_or(false) {
        "SELECT * FROM notifications WHERE read=0 ORDER BY created_at DESC"
    } else {
        "SELECT * FROM notifications ORDER BY created_at DESC"
    };
    let rows = database::all(sql, &[]).map_err(|e| e.to_string())?;
    Ok(json!({ "success": true, "notifications": rows }))
}

#[tauri::command]
pub async fn notification_create(data: Value) -> Result<Value, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let ntype = data["type"].as_str().unwrap_or("llm_insight");
    let title = data["title"].as_str().unwrap_or("");
    let content = data["content"].as_str().unwrap_or("");
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
    Ok(json!({ "success": true, "config": config }))
}

#[tauri::command]
pub async fn notification_config_update(data: Value) -> Result<(), String> {
    if let Some(obj) = data.as_object() {
        for (k, v) in obj {
            let val = v.as_str().unwrap_or(&v.to_string()).to_string();
            database::run(
                "INSERT OR REPLACE INTO notification_config(key, value) VALUES(?1, ?2)",
                &[json!(k), json!(val)],
            ).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
