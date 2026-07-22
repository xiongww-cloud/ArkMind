use crate::database;
use serde_json::{json, Value};
use tauri::Emitter;

#[tauri::command] pub async fn kb_get_config() -> Result<Value, String> {
    let row = database::get("SELECT kb_config FROM system_config WHERE tenant_id='local-user'", &[]).map_err(|e| e.to_string())?;
    let config: Value = row.and_then(|r| r["kb_config"].as_str().map(|s| s.to_string())).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(json!({}));
    Ok(json!({"success":true,"embeddingApiUrl":config["embeddingApiUrl"],"embeddingApiKey":config["embeddingApiKey"],"embeddingModelName":config["embeddingModelName"],"embeddingActive":config["embeddingActive"],"chatApiUrl":config["chatApiUrl"],"chatApiKey":config["chatApiKey"],"chatModelName":config["chatModelName"]}))
}
#[tauri::command] pub async fn kb_update_config(data: Value) -> Result<Value, String> {
    database::run("UPDATE system_config SET kb_config=?,updated_at=datetime('now', 'localtime') WHERE tenant_id='local-user'", &[json!(data.to_string())]).map_err(|e| e.to_string())?;
    crate::services::embedding_client::invalidate_embedding_config_cache();
    Ok(json!({"success":true}))
}
#[tauri::command] pub async fn kb_agent_chat(app_handle: tauri::AppHandle, data: Value) -> Result<(), String> {
    let query = data["query"].as_str().unwrap_or("").to_string();
    let max_iter = data["maxIterations"].as_i64().map(|n| n as i32);
    let history: Vec<Value> = data["history"].as_array().map(|a| a.to_vec()).unwrap_or_default();
    tauri::async_runtime::spawn(async move {
        let result = crate::services::agent_workflow::agent_chat_workflow(query, max_iter, history, Some(app_handle.clone()), false).await;
        match result {
            Ok(v) => { let _ = app_handle.emit("agent:done", &v); if let Some(ans) = v["answer"].as_str() { if !ans.is_empty() { let preview: String = ans.chars().take(100).collect(); println!("[Agent] 响应: {}", preview); } } }
            Err(e) => {
                if e == "cancelled" {
                    let _ = app_handle.emit("agent:done", json!({"success":true,"answer":"已停止执行。","cancelled":true}));
                } else {
                    eprintln!("[Agent] 错误: {}", e);
                    let _ = app_handle.emit("agent:error", json!({"error": e}));
                }
            }
        }
    });
    Ok(())
}
#[tauri::command] pub async fn kb_chat(app_handle: tauri::AppHandle, data: Value) -> Result<(), String> {
    let query = data["query"].as_str().unwrap_or("").to_string();
    if query.is_empty() { return Err("请输入问题".to_string()); }
    tauri::async_runtime::spawn(async move {
        let result = crate::services::agent_workflow::agent_chat_workflow(query, None, vec![], Some(app_handle.clone()), false).await;
        match result {
            Ok(v) => { let _ = app_handle.emit("agent:done", &v); }
            Err(e) => {
                if e == "cancelled" {
                    let _ = app_handle.emit("agent:done", json!({"success":true,"answer":"已停止执行。","cancelled":true}));
                } else {
                    let _ = app_handle.emit("agent:error", json!({"error": e}));
                }
            }
        }
    });
    Ok(())
}
#[tauri::command] pub async fn agent_cancel() -> Result<Value, String> {
    crate::services::agent_workflow::cancel_agent();
    Ok(json!({"success": true}))
}
