use crate::database;
use serde_json::{json, Value};

pub mod kb_config_commands;
pub mod kb_search_commands;
pub mod kb_memory_commands;

pub use kb_config_commands::*;
pub use kb_search_commands::*;
pub use kb_memory_commands::*;

// Chat session persistence
#[tauri::command] pub async fn kb_save_sessions(data: Value) -> Result<Value, String> {
    let sessions = data["sessions"].as_array().ok_or("missing sessions array")?;
    for s in sessions.iter() {
        let id = s["id"].as_str().ok_or("missing session id")?;
        let _title = s["title"].as_str().or_else(|| s["name"].as_str()).unwrap_or("");
        let updated_at = s["updatedAt"].as_str().or_else(|| s["updated_at"].as_str())
            .unwrap_or(&chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f").to_string())
            .to_string();
        database::run("INSERT INTO chat_sessions(id,title,created_at,updated_at) VALUES(?1,?2,?3,?3) ON CONFLICT(id) DO UPDATE SET title=excluded.title, updated_at=excluded.updated_at",
            &[json!(id), json!(_title), json!(updated_at)]).map_err(|e| e.to_string())?;
        let msgs_raw = s["messages"].as_array().cloned().or_else(|| s["messages"].as_str().and_then(|s| serde_json::from_str::<Vec<Value>>(s).ok()));
        if let Some(ref msgs) = msgs_raw {
            // Use transaction to ensure atomicity: DELETE + INSERT as a single operation
            database::with_db(|conn| {
                let tx = conn.unchecked_transaction()?;
                tx.execute("DELETE FROM chat_messages WHERE session_id=?", rusqlite::params![id])?;
                for m in msgs {
                    let role = m["role"].as_str().unwrap_or("");
                    let content = m["content"].as_str().unwrap_or("");
                    if role != "user" && role != "assistant" { continue; }
                    if content.trim().is_empty() { continue; }
                    let metadata = serde_json::to_string(m).ok();
                    tx.execute("INSERT INTO chat_messages(session_id,role,content,metadata) VALUES(?1,?2,?3,?4)",
                        rusqlite::params![id, role, content, metadata])?;
                }
                tx.commit()?;
                Ok(())
            }).map_err(|e| e.to_string())?;
            // 对话记录由后端 agent_chat_workflow 以 MEMORY 类型（认知过程格式）自动创建
            // 前端不再创建 CHAT source_data，只保存 chat_messages 表（显示用）
        }
    }
    Ok(json!({"success":true}))
}
#[tauri::command] pub async fn kb_load_sessions() -> Result<Value, String> {
    let mut rows = database::all(
        "SELECT s.id, s.title, s.created_at, s.updated_at, \
         (SELECT json_group_array(json_object('role',m.role,'content',m.content,'metadata',m.metadata)) \
          FROM chat_messages m WHERE m.session_id=s.id ORDER BY m.id) as messages \
         FROM chat_sessions s WHERE s.id != 'fab-main-session' ORDER BY s.updated_at DESC LIMIT 100",
        &[],
    ).unwrap_or_default();
    for row in &mut rows {
        if let Some(s) = row["messages"].as_str() {
            if let Ok(arr) = serde_json::from_str::<Value>(s) {
                // Restore full message from metadata (toolCalls, workspace, etc.)
                let restored: Vec<Value> = arr.as_array().map(|msgs| {
                    msgs.iter().map(|m| {
                        if let Some(meta_str) = m.get("metadata").and_then(|v| v.as_str()) {
                            if let Ok(full_msg) = serde_json::from_str::<Value>(meta_str) {
                                return full_msg;
                            }
                        }
                        json!({"role": m["role"], "content": m["content"]})
                    }).collect()
                }).unwrap_or_default();
                let m = row.as_object_mut().unwrap();
                m.insert("messages".to_string(), Value::Array(restored));
            }
        }
    }
    Ok(json!({"success":true,"sessions":rows}))
}

#[tauri::command] pub async fn kb_delete_session(session_id: String) -> Result<Value, String> {
    database::run("DELETE FROM chat_sessions WHERE id=?", &[json!(&session_id)]).ok();
    Ok(json!({"success":true}))
}

// ═══════════════════════════════════════════════════════════════════
// FAB Session — 小精灵隐藏会话（永远存在，全量持久化）
// ═══════════════════════════════════════════════════════════════════
const FAB_SESSION_ID: &str = "fab-main-session";

/// 初始化 FAB 专属会话，确保存在
#[tauri::command]
pub async fn kb_fab_init_session() -> Result<Value, String> {
    database::run(
        "INSERT OR IGNORE INTO chat_sessions(id, title) VALUES(?1, '小精灵对话')",
        &[json!(FAB_SESSION_ID)]
    ).map_err(|e| e.to_string())?;
    Ok(json!({"success": true, "session_id": FAB_SESSION_ID}))
}

/// 加载 FAB 会话的历史消息
#[tauri::command]
pub async fn kb_fab_load_history(limit: Option<usize>) -> Result<Value, String> {
    let limit = limit.unwrap_or(50);
    let rows = database::all(
        "SELECT role, content, metadata FROM chat_messages WHERE session_id=?1 ORDER BY id DESC LIMIT ?2",
        &[json!(FAB_SESSION_ID), json!(limit)]
    ).unwrap_or_default();
    // 反转为正序（从旧到新）
    let messages: Vec<Value> = rows.into_iter().rev().map(|m| {
        if let Some(meta_str) = m.get("metadata").and_then(|v| v.as_str()) {
            if let Ok(full_msg) = serde_json::from_str::<Value>(meta_str) {
                return full_msg;
            }
        }
        json!({"role": m["role"], "content": m["content"]})
    }).collect();
    Ok(json!({"success": true, "messages": messages}))
}

/// 保存单条消息到 FAB 会话（增量保存）
#[tauri::command]
pub async fn kb_fab_save_message(data: Value) -> Result<Value, String> {
    let role = data["role"].as_str().unwrap_or("");
    let content = data["content"].as_str().unwrap_or("");
    let metadata = data.get("metadata")
        .filter(|m| !m.is_null())
        .and_then(|m| serde_json::to_string(m).ok());

    if role.is_empty() || content.trim().is_empty() {
        return Ok(json!({"success": true}));
    }

    database::run(
        "INSERT INTO chat_messages(session_id, role, content, metadata) VALUES(?1, ?2, ?3, ?4)",
        &[json!(FAB_SESSION_ID), json!(role), json!(content), json!(metadata)]
    ).map_err(|e| e.to_string())?;

    // 更新会话时间戳
    database::run(
        "UPDATE chat_sessions SET updated_at=datetime('now','localtime') WHERE id=?1",
        &[json!(FAB_SESSION_ID)]
    ).ok();

    Ok(json!({"success": true}))
}
