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
         (SELECT json_group_array(json_object('id', m.id, 'role', m.role, 'content', m.content, 'status', m.status, 'created_at', m.created_at, 'metadata', m.metadata)) \
          FROM chat_messages m \
          WHERE m.id IN (SELECT id FROM chat_messages WHERE session_id=s.id ORDER BY id DESC LIMIT 10) \
          ORDER BY m.id ASC) as messages \
         FROM chat_sessions s WHERE s.id NOT LIKE 'fab-%' ORDER BY s.updated_at DESC LIMIT 100",
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
            "SELECT id, role, content, status, metadata, created_at FROM chat_messages WHERE session_id=?1 AND id<?2 ORDER BY id DESC LIMIT ?3",
            &[json!(session_id), json!(bid), json!(limit)]
        ).unwrap_or_default(),
        None => database::all(
            "SELECT id, role, content, status, metadata, created_at FROM chat_messages WHERE session_id=?1 ORDER BY id DESC LIMIT ?2",
            &[json!(session_id), json!(limit)]
        ).unwrap_or_default(),
    };
    // 反转为正序（从旧到新）
    let messages: Vec<Value> = rows.into_iter().rev().map(|m| {
        json!({"id": m["id"], "role": m["role"], "content": m["content"], "status": m["status"], "metadata": m["metadata"], "created_at": m["created_at"]})
    }).collect();
    Ok(json!({"success": true, "messages": messages}))
}

#[tauri::command] pub async fn kb_delete_session(session_id: String) -> Result<Value, String> {
    // ★ 删除前收集本会话 AI 生成文档的 file_id（存于消息 metadata，会话删除后即失联）
    let doc_file_ids = collect_session_document_ids(&session_id);
    // 级联删除会话与消息
    database::run("DELETE FROM chat_sessions WHERE id=?", &[json!(&session_id)]).ok();
    // ★ 联动删除生成的文档（本会话消息已删，此时剩余的 metadata 引用均来自其他会话）
    for fid in &doc_file_ids {
        let other_refs: i64 = database::get(
            "SELECT COUNT(*) as cnt FROM chat_messages WHERE metadata IS NOT NULL AND metadata LIKE ?1",
            &[json!(format!("%{}%", fid))],
        ).ok().flatten().and_then(|r| r["cnt"].as_i64()).unwrap_or(0);
        if other_refs > 0 {
            // 同字节内容被其他会话复用（CAS 同 hash）→ 保留，避免他处文档卡片失效
            log::info!("[删除会话] 文档 {} 仍被其他会话引用，保留", fid);
            continue;
        }
        crate::services::doc_writer::delete_generated_doc(fid);
    }
    if !doc_file_ids.is_empty() {
        log::info!("[删除会话] {} 联动处理了 {} 个生成文档", session_id, doc_file_ids.len());
    }
    Ok(json!({"success":true}))
}

/// 从会话消息的 metadata 中提取 AI 生成文档的 file_id 列表（去重）
fn collect_session_document_ids(session_id: &str) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    let rows = database::all(
        "SELECT metadata FROM chat_messages WHERE session_id=?1 AND metadata IS NOT NULL",
        &[json!(session_id)],
    ).unwrap_or_default();
    for r in &rows {
        let Some(meta) = r["metadata"].as_str() else { continue };
        let Ok(v) = serde_json::from_str::<Value>(meta) else { continue };
        if let Some(arr) = v["documents"].as_array() {
            for d in arr {
                if let Some(fid) = d["file_id"].as_str().filter(|s| !s.is_empty()) {
                    ids.push(fid.to_string());
                }
            }
        }
    }
    ids.sort();
    ids.dedup();
    ids
}

// ═══════════════════════════════════════════════════════════════════
// FAB Session — 快呼浮窗会话：30 分钟活跃窗口轮转（Spotlight 语义）
//   · 距最后活跃 ≤30min：维持会话（追问/连续对话不断线）
//   · 有未答复追问卡片：轮转冻结至多 24h，保 ask_user 闭环
//   · 其余：新建 fab-<uuid> 会话，浮窗回到干净面板
//   状态存 system_settings KV（key=fab_session_state，零迁移）；历史列表按
//   'fab-%' 前缀排除；被换下的旧会话 7 天后 GC。
// ═══════════════════════════════════════════════════════════════════
const FAB_SESSION_PREFIX: &str = "fab-";
const FAB_LEGACY_SESSION: &str = "fab-main-session";
const FAB_SESSION_WINDOW_SECS: i64 = 30 * 60;
const FAB_CLARIFICATION_GRACE_SECS: i64 = 24 * 3600;
const FAB_GC_DAYS: i64 = 7;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum FabAction { Keep, Rotate }

/// 轮转判定（纯函数，可单测）：now/last 为 epoch 秒
pub(crate) fn decide_fab_rotation(now: i64, last: i64, has_pending_clarification: bool) -> FabAction {
    let elapsed = now.saturating_sub(last);
    if elapsed <= FAB_SESSION_WINDOW_SECS { return FabAction::Keep; }
    if has_pending_clarification && elapsed <= FAB_CLARIFICATION_GRACE_SECS { return FabAction::Keep; }
    FabAction::Rotate
}

fn read_fab_state() -> (Option<String>, Option<i64>) {
    let row = database::get("SELECT value FROM system_settings WHERE key='fab_session_state'", &[])
        .ok().flatten();
    let Some(v) = row.and_then(|r| r["value"].as_str().map(String::from)) else { return (None, None) };
    let Ok(parsed) = serde_json::from_str::<Value>(&v) else { return (None, None) };
    let id = parsed["id"].as_str().map(String::from);
    let last = parsed["last"].as_i64();
    (id, last)
}

fn write_fab_state(id: &str, last: i64) {
    database::run(
        "INSERT OR REPLACE INTO system_settings(key, value, updated_at) VALUES('fab_session_state', ?1, datetime('now','localtime'))",
        &[json!({"id": id, "last": last})],
    ).ok();
}

/// 遗留固定会话是否仍有消息（升级接管判据）
fn legacy_fab_session_exists() -> bool {
    database::get(
        "SELECT id FROM chat_messages WHERE session_id=?1 LIMIT 1",
        &[json!(FAB_LEGACY_SESSION)],
    ).ok().flatten().is_some()
}

/// 会话最后一条消息是否为"未答复的追问"（assistant 且 metadata.clarification 存在）
fn fab_has_pending_clarification(session_id: &str) -> bool {
    database::get(
        "SELECT role, metadata FROM chat_messages WHERE session_id=?1 ORDER BY id DESC LIMIT 1",
        &[json!(session_id)],
    ).ok().flatten().map_or(false, |r| {
        if r["role"].as_str() != Some("assistant") { return false; }
        let Some(meta) = r["metadata"].as_str() else { return false };
        serde_json::from_str::<Value>(meta).ok()
            .and_then(|m| m.get("clarification").filter(|c| c.is_object()).map(|_| true))
            .unwrap_or(false)
    })
}

/// 新建浮窗会话（标题"快速唤起"，主窗列表按前缀排除）
fn create_fab_session() -> String {
    let id = format!("{}{}", FAB_SESSION_PREFIX, uuid::Uuid::new_v4());
    database::run(
        "INSERT INTO chat_sessions(id, title, created_at, updated_at) VALUES(?1, '快速唤起', datetime('now','localtime'), datetime('now','localtime'))",
        &[json!(&id)],
    ).ok();
    id
}

/// 轮换时顺路 GC：删除 7 天未活跃的 fab 会话及其消息
fn gc_fab_sessions() {
    database::run(
        "DELETE FROM chat_messages WHERE session_id IN (
            SELECT id FROM chat_sessions WHERE id LIKE 'fab-%'
              AND updated_at < datetime('now','localtime', ?1)
        )",
        &[json!(format!("-{} days", FAB_GC_DAYS))],
    ).ok();
    database::run(
        "DELETE FROM chat_sessions WHERE id LIKE 'fab-%'
           AND updated_at < datetime('now','localtime', ?1)",
        &[json!(format!("-{} days", FAB_GC_DAYS))],
    ).ok();
}

/// 解析当前浮窗会话：判定维持/轮转，刷新活跃时间。
/// 前端在挂载、每次发送前、窗口获得焦点（唤起）时调用——获得焦点即视为活跃
#[tauri::command]
pub async fn fab_resolve_session() -> Result<Value, String> {
    let now = chrono::Local::now().timestamp();
    let (mut cur_id, mut last) = read_fab_state();
    // 升级迁移：无 KV 状态但存在遗留固定会话 → 接管（视为刚活跃，历史不丢）
    if cur_id.is_none() && legacy_fab_session_exists() {
        database::run(
            "INSERT OR IGNORE INTO chat_sessions(id, title) VALUES(?1, '快速唤起')",
            &[json!(FAB_LEGACY_SESSION)],
        ).ok();
        cur_id = Some(FAB_LEGACY_SESSION.to_string());
        last = None;
    }
    let pending = cur_id.as_deref().map_or(false, |id| fab_has_pending_clarification(id));
    let action = match (cur_id.as_ref(), last) {
        (Some(_), Some(l)) => decide_fab_rotation(now, l, pending),
        (Some(_), None) => FabAction::Keep, // 刚接管/时间戳缺失：视为窗口内
        (None, _) => FabAction::Rotate,
    };
    let (id, rotated) = match action {
        FabAction::Keep => (cur_id.unwrap(), false),
        FabAction::Rotate => {
            gc_fab_sessions();
            (create_fab_session(), true)
        }
    };
    write_fab_state(&id, now);
    Ok(json!({"success": true, "sessionId": id, "rotated": rotated}))
}

/// 当前浮窗会话 id（不判定、不 touch，历史回放兜底用）
fn current_fab_session_id() -> Option<String> {
    read_fab_state().0
}

/// 清空全部快呼会话（含轮换前的旧会话与遗留 fab-main-session）
#[tauri::command]
pub async fn kb_clear_fab_sessions() -> Result<Value, String> {
    database::run("DELETE FROM chat_messages WHERE session_id LIKE 'fab-%'", &[]).map_err(|e| e.to_string())?;
    database::run("DELETE FROM chat_sessions WHERE id LIKE 'fab-%'", &[]).map_err(|e| e.to_string())?;
    database::run("DELETE FROM system_settings WHERE key='fab_session_state'", &[]).map_err(|e| e.to_string())?;
    Ok(json!({"success": true}))
}

/// 加载 FAB 会话的历史消息（支持 before_id 游标分页）；缺省用当前浮窗会话
#[tauri::command]
pub async fn kb_fab_load_history(session_id: Option<String>, limit: Option<usize>, before_id: Option<i64>) -> Result<Value, String> {
    let Some(sid) = session_id.or_else(current_fab_session_id) else {
        return Ok(json!({"success": true, "messages": []}));
    };
    let limit = limit.unwrap_or(50);
    let rows = match before_id {
        Some(bid) => database::all(
            "SELECT id, role, content, status, metadata, created_at FROM chat_messages WHERE session_id=?1 AND id<?2 ORDER BY id DESC LIMIT ?3",
            &[json!(sid), json!(bid), json!(limit)]
        ).unwrap_or_default(),
        None => database::all(
            "SELECT id, role, content, status, metadata, created_at FROM chat_messages WHERE session_id=?1 ORDER BY id DESC LIMIT ?2",
            &[json!(sid), json!(limit)]
        ).unwrap_or_default(),
    };

    // 反转为正序（从旧到新）
    let messages: Vec<Value> = rows.into_iter().rev().map(|m| {
        json!({"id": m["id"], "role": m["role"], "content": m["content"], "metadata": m["metadata"], "created_at": m["created_at"]})
    }).collect();
    Ok(json!({"success": true, "messages": messages}))
}

/// 保存单条消息到 FAB 会话（增量保存）；缺省用当前浮窗会话
#[tauri::command]
pub async fn kb_fab_save_message(data: Value) -> Result<Value, String> {
    let role = data["role"].as_str().unwrap_or("");
    let content = data["content"].as_str().unwrap_or("");
    let sid = data["session_id"].as_str().map(String::from).or_else(current_fab_session_id);

    if role.is_empty() || content.trim().is_empty() {
        return Ok(json!({"success": true}));
    }
    let Some(sid) = sid else { return Ok(json!({"success": true})); };

    database::run(
        "INSERT INTO chat_messages(session_id, role, content) VALUES(?1, ?2, ?3)",
        &[json!(&sid), json!(role), json!(content)]
    ).map_err(|e| e.to_string())?;

    // 更新会话时间戳
    database::run(
        "UPDATE chat_sessions SET updated_at=datetime('now','localtime') WHERE id=?1",
        &[json!(&sid)]
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
    // 清理没有任何消息的空 session（fab 空会话由 resolve 的 7 天 GC 负责）
    database::run(
        "DELETE FROM chat_sessions WHERE id NOT IN (SELECT DISTINCT session_id FROM chat_messages) AND id NOT LIKE 'fab-%'",
        &[]
    ).ok();
}

#[cfg(test)]
mod fab_rotation_tests {
    use super::{decide_fab_rotation, FabAction};

    const MIN: i64 = 60;

    #[test]
    fn keeps_within_window() {
        assert_eq!(decide_fab_rotation(1000, 1000 - 29 * MIN, false), FabAction::Keep);
        // 恰好 30 分钟边界：维持
        assert_eq!(decide_fab_rotation(1000, 1000 - 30 * MIN, false), FabAction::Keep);
    }

    #[test]
    fn rotates_after_window_without_pending() {
        assert_eq!(decide_fab_rotation(1000, 1000 - 31 * MIN, false), FabAction::Rotate);
        assert_eq!(decide_fab_rotation(1000, 1000 - 10 * 60 * MIN, false), FabAction::Rotate);
    }

    #[test]
    fn clarification_freezes_rotation_up_to_24h() {
        assert_eq!(decide_fab_rotation(1000, 1000 - 45 * MIN, true), FabAction::Keep);
        // 恰好 24h 边界：仍冻结
        assert_eq!(decide_fab_rotation(1000, 1000 - 24 * 60 * MIN, true), FabAction::Keep);
        // 超时视为放弃作答 → 轮转
        assert_eq!(decide_fab_rotation(1000, 1000 - 25 * 60 * MIN, true), FabAction::Rotate);
    }

    #[test]
    fn clock_skew_last_in_future_keeps() {
        assert_eq!(decide_fab_rotation(1000, 1200, false), FabAction::Keep);
    }
}
