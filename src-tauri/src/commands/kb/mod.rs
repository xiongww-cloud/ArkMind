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
    }
    Ok(json!({"success":true}))
}
#[tauri::command] pub async fn kb_load_sessions() -> Result<Value, String> {
    let mut rows = database::all(
        "SELECT s.id, s.title, s.created_at, s.updated_at, \
         (SELECT json_group_array(json_object('id', m.id, 'role', m.role, 'content', m.content, 'status', m.status, 'created_at', m.created_at)) \
          FROM chat_messages m \
          WHERE m.id IN (SELECT id FROM chat_messages WHERE session_id=s.id ORDER BY id DESC LIMIT 10) \
          ORDER BY m.id ASC) as messages \
         FROM chat_sessions s WHERE s.id != 'fab-main-session' ORDER BY s.updated_at DESC LIMIT 100",
        &[],
    ).unwrap_or_default();
    for row in &mut rows {
        if let Some(s) = row["messages"].as_str() {
            if let Ok(arr) = serde_json::from_str::<Value>(s) {
                let m = row.as_object_mut().unwrap();
                m.insert("messages".to_string(), arr);
            }
        }
    }
    Ok(json!({"success":true,"sessions":rows}))
}

/// 按会话分页加载消息（游标：before_id，向前翻更早的消息）
#[tauri::command] pub async fn kb_load_session_messages(session_id: String, before_id: Option<i64>, limit: Option<usize>) -> Result<Value, String> {
    let limit = limit.unwrap_or(10);
    let rows = match before_id {
        Some(bid) => database::all(
            "SELECT id, role, content, status, created_at FROM chat_messages WHERE session_id=?1 AND id<?2 ORDER BY id DESC LIMIT ?3",
            &[json!(session_id), json!(bid), json!(limit)]
        ).unwrap_or_default(),
        None => database::all(
            "SELECT id, role, content, status, created_at FROM chat_messages WHERE session_id=?1 ORDER BY id DESC LIMIT ?2",
            &[json!(session_id), json!(limit)]
        ).unwrap_or_default(),
    };
    // 反转为正序（从旧到新）
    let messages: Vec<Value> = rows.into_iter().rev().map(|m| {
        json!({"id": m["id"], "role": m["role"], "content": m["content"], "status": m["status"], "created_at": m["created_at"]})
    }).collect();
    Ok(json!({"success": true, "messages": messages}))
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

/// 加载 FAB 会话的历史消息（支持 before_id 游标分页）
#[tauri::command]
pub async fn kb_fab_load_history(limit: Option<usize>, before_id: Option<i64>) -> Result<Value, String> {
    let limit = limit.unwrap_or(50);
    let rows = match before_id {
        Some(bid) => database::all(
            "SELECT id, role, content, status, created_at FROM chat_messages WHERE session_id=?1 AND id<?2 ORDER BY id DESC LIMIT ?3",
            &[json!(FAB_SESSION_ID), json!(bid), json!(limit)]
        ).unwrap_or_default(),
        None => database::all(
            "SELECT id, role, content, status, created_at FROM chat_messages WHERE session_id=?1 ORDER BY id DESC LIMIT ?2",
            &[json!(FAB_SESSION_ID), json!(limit)]
        ).unwrap_or_default(),
    };

    // 反转为正序（从旧到新）
    let messages: Vec<Value> = rows.into_iter().rev().map(|m| {
        json!({"id": m["id"], "role": m["role"], "content": m["content"], "created_at": m["created_at"]})
    }).collect();
    Ok(json!({"success": true, "messages": messages}))
}

/// 保存单条消息到 FAB 会话（增量保存）
#[tauri::command]
pub async fn kb_fab_save_message(data: Value) -> Result<Value, String> {
    let role = data["role"].as_str().unwrap_or("");
    let content = data["content"].as_str().unwrap_or("");

    if role.is_empty() || content.trim().is_empty() {
        return Ok(json!({"success": true}));
    }

    database::run(
        "INSERT INTO chat_messages(session_id, role, content) VALUES(?1, ?2, ?3)",
        &[json!(FAB_SESSION_ID), json!(role), json!(content)]
    ).map_err(|e| e.to_string())?;

    // 更新会话时间戳
    database::run(
        "UPDATE chat_sessions SET updated_at=datetime('now','localtime') WHERE id=?1",
        &[json!(FAB_SESSION_ID)]
    ).ok();

    Ok(json!({"success": true}))
}

/// 启动时清理：每个 session 最多保留 max_compressed 条 compressed 消息，清理空 session
pub fn cleanup_chat_messages(max_compressed_per_session: i64) {
    // 每个 session 最多保留 max_compressed_per_session 条 compressed 消息（删最老的）
    database::run(
        "DELETE FROM chat_messages WHERE id IN (
            SELECT id FROM (
                SELECT id, ROW_NUMBER() OVER (PARTITION BY session_id ORDER BY id DESC) as rn
                FROM chat_messages WHERE status='compressed'
            ) WHERE rn > ?1
        )",
        &[json!(max_compressed_per_session)],
    ).ok();
    // 清理没有任何消息的空 session
    database::run(
        "DELETE FROM chat_sessions WHERE id NOT IN (SELECT DISTINCT session_id FROM chat_messages) AND id != 'fab-main-session'",
        &[]
    ).ok();
}
