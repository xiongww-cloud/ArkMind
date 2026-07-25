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
