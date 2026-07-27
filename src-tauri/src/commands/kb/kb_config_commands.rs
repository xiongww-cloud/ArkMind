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

/// 将 tool_log 转换为 tool_calls + tool_results 格式
/// 返回 (all_tool_calls, all_tool_results)
fn convert_tool_log(tool_log: &[Value]) -> (Vec<Value>, Vec<Value>) {
    let mut all_tool_calls: Vec<Value> = Vec::new();
    let mut all_tool_results: Vec<Value> = Vec::new();
    let mut call_counter = 0u32;
    for round in tool_log {
        if let Some(tools) = round.get("tools").and_then(|v| v.as_array()) {
            for t in tools {
                let tool_name = t["tool"].as_str().unwrap_or("");
                let args = &t["args"];
                let result = t["result"].as_str().unwrap_or("");
                call_counter += 1;
                let call_id = format!("call_db_{}", call_counter);
                all_tool_calls.push(json!({
                    "id": call_id,
                    "type": "function",
                    "function": {
                        "name": tool_name,
                        "arguments": serde_json::to_string(args).unwrap_or_default()
                    }
                }));
                all_tool_results.push(json!({
                    "role": "tool",
                    "content": result,
                    "tool_call_id": call_id,
                    "name": tool_name
                }));
            }
        }
    }
    (all_tool_calls, all_tool_results)
}

#[tauri::command] pub async fn kb_agent_chat(app_handle: tauri::AppHandle, data: Value) -> Result<(), String> {
    let query = data["query"].as_str().unwrap_or("").to_string();
    let max_iter = data["maxIterations"].as_i64().map(|n| n as i32);
    let session_id = data["sessionId"].as_str().map(String::from);

    // ★ 保存 user 消息到 DB（发送前）
    if let Some(ref sid) = session_id {
        let user_content = query.clone();
        let _ = crate::database::run(
            "INSERT INTO chat_messages(session_id, role, content, metadata) VALUES(?1, 'user', ?2, NULL)",
            &[json!(sid), json!(user_content)],
        );
    }

    tauri::async_runtime::spawn(async move {
        let result = crate::services::agent_workflow::agent_chat_workflow(query, max_iter, Some(app_handle.clone()), false, session_id.clone()).await;
        match result {
            Ok(v) => {
                // ★ 保存 assistant 消息到 DB：content=最终回答，metadata=tool_calls+tool_results+context_docs
                if let Some(ref sid) = session_id {
                    let answer = v["answer"].as_str().unwrap_or("");
                    let mut meta = serde_json::Map::new();
                    meta.insert("role".to_string(), json!("assistant"));
                    meta.insert("content".to_string(), json!(answer));
                    // 合并所有轮次的 tool_calls 和 tool_results
                    if let Some(tool_log) = v.get("tool_log").and_then(|v| v.as_array()) {
                        let (tc, tr) = convert_tool_log(tool_log);
                        if !tc.is_empty() { meta.insert("tool_calls".to_string(), json!(tc)); }
                        if !tr.is_empty() { meta.insert("tool_results".to_string(), json!(tr)); }
                    }
                    if let Some(cd) = v.get("context_docs") { meta.insert("context_docs".to_string(), cd.clone()); }
                    let assistant_metadata = serde_json::Value::Object(meta).to_string();
                    let _ = crate::database::run(
                        "INSERT INTO chat_messages(session_id, role, content, metadata) VALUES(?1, 'assistant', ?2, ?3)",
                        &[json!(sid), json!(answer), json!(assistant_metadata)],
                    );
                    // 更新会话时间戳
                    let _ = crate::database::run(
                        "UPDATE chat_sessions SET updated_at=datetime('now','localtime') WHERE id=?1",
                        &[json!(sid)],
                    );
                }
                let _ = app_handle.emit("agent:done", &v);
                if let Some(ans) = v["answer"].as_str() { if !ans.is_empty() { let preview: String = ans.chars().take(100).collect(); println!("[Agent] 响应: {}", preview); } }
            }
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
        let result = crate::services::agent_workflow::agent_chat_workflow(query, None, Some(app_handle.clone()), false, None).await;
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
