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
        // ★ 前端未传 updatedAt 时不写时间戳（重命名等元数据操作不得污染会话时间）；
        //   仅新会话 INSERT 路径用当前时间兜底 created_at/updated_at
        let updated_at = s["updatedAt"].as_str().or_else(|| s["updated_at"].as_str()).unwrap_or("").to_string();
        database::run("INSERT INTO chat_sessions(id,title,created_at,updated_at) \
             VALUES(?1,?2,COALESCE(NULLIF(?3,''),datetime('now','localtime')),COALESCE(NULLIF(?3,''),datetime('now','localtime'))) \
             ON CONFLICT(id) DO UPDATE SET title=excluded.title, \
             updated_at=COALESCE(NULLIF(excluded.updated_at,''), chat_sessions.updated_at)",
            &[json!(id), json!(_title), json!(updated_at)]).map_err(|e| e.to_string())?;
    }
    Ok(json!({"success":true}))
}
#[tauri::command] pub async fn kb_load_sessions() -> Result<Value, String> {
    // ★ 会话时间权威口径 = 最后一条消息时间（last_msg_at）：updated_at 可被历史操作污染，
    //   展示与排序都用消息时间；无消息会话回退 updated_at/created_at
    let mut rows = database::all(
        "SELECT s.id, s.title, s.created_at, s.updated_at, lm.last_msg AS last_msg_at, \
         (SELECT json_group_array(json_object('id', m.id, 'role', m.role, 'content', m.content, 'status', m.status, 'created_at', m.created_at, 'metadata', m.metadata)) \
          FROM chat_messages m \
         WHERE m.id IN (SELECT id FROM chat_messages WHERE session_id=s.id AND status!='withdrawn' ORDER BY id DESC LIMIT 10) \
         ORDER BY m.id ASC) as messages \
         FROM chat_sessions s \
         LEFT JOIN (SELECT session_id, MAX(created_at) AS last_msg FROM chat_messages WHERE status!='withdrawn' GROUP BY session_id) lm ON lm.session_id = s.id \
         ORDER BY COALESCE(lm.last_msg, s.updated_at, s.created_at) DESC LIMIT 100",
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
            "SELECT id, role, content, status, metadata, created_at FROM chat_messages WHERE session_id=?1 AND status!='withdrawn' AND id<?2 ORDER BY id DESC LIMIT ?3",
            &[json!(session_id), json!(bid), json!(limit as i64)]
        ).unwrap_or_default(),
        None => database::all(
            "SELECT id, role, content, status, metadata, created_at FROM chat_messages WHERE session_id=?1 AND status!='withdrawn' ORDER BY id DESC LIMIT ?2",
            &[json!(session_id), json!(limit as i64)]
        ).unwrap_or_default(),
    };
    // 反转为正序（从旧到新）
    let messages: Vec<Value> = rows.into_iter().rev().map(|m| {
        json!({"id": m["id"], "role": m["role"], "content": m["content"], "status": m["status"], "metadata": m["metadata"], "created_at": m["created_at"]})
    }).collect();
    Ok(json!({"success": true, "messages": messages}))
}

#[tauri::command] pub async fn kb_delete_session(session_id: String) -> Result<Value, String> {
    // ★ 生成文档联动：与项目/笔记等域同构——删除前按归属收集（file_relations entity_id=会话id），
    //   会话删除（CASCADE 消息）后释放。跨会话同内容复用/转存资料库等引用由 release 计数天然保护。
    let doc_file_ids = crate::services::doc_writer::collect_generated_file_ids(&[session_id.clone()]);
    database::run("DELETE FROM chat_sessions WHERE id=?", &[json!(&session_id)]).ok();
    crate::services::doc_writer::release_generated_for_sessions(&[session_id.clone()], &doc_file_ids);
    if !doc_file_ids.is_empty() {
        log::info!("[删除会话] {} 联动处理了 {} 个生成文档", session_id, doc_file_ids.len());
    }
    Ok(json!({"success":true}))
}

// ═══════════════════════════════════════════════════════════════════
// 快呼浮窗会话 — 30 分钟提问窗口轮转 + Gemini 式统一当前对话
//   · 全局指针 KV（fab_session_state={id,last}）单一写入者 = kb_agent_chat
//     每次发送（任一窗口）→ 浮窗唤开即续"上次聊到的地方"
//   · 轮转仅在浮窗发送时判定：距上次提问 ≤30min 续；追问卡片冻结 ≤24h；
//     当前会话已被列表删除 / 首次无状态 → 新建 fab-<uuid>（决定不写状态）
//   · 打开面板永不轮转：始终淡显当前会话上一轮，提问才切换
//   · 会话在智能问答列表全量可见（无前缀排除、不 GC），标题=首问 30 字
// ═══════════════════════════════════════════════════════════════════
const FAB_SESSION_PREFIX: &str = "fab-";
const FAB_LEGACY_SESSION: &str = "fab-main-session";
const FAB_SESSION_WINDOW_SECS: i64 = 30 * 60;
const FAB_CLARIFICATION_GRACE_SECS: i64 = 24 * 3600;
const FAB_TITLE_PLACEHOLDER: &str = "快速唤起";

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum FabAction { Keep, Rotate }

/// 轮转判定（纯函数，可单测）：now/last 为 epoch 秒
pub(crate) fn decide_fab_rotation(now: i64, last: i64, has_pending_clarification: bool) -> FabAction {
    let elapsed = now.saturating_sub(last);
    if elapsed <= FAB_SESSION_WINDOW_SECS { return FabAction::Keep; }
    if has_pending_clarification && elapsed <= FAB_CLARIFICATION_GRACE_SECS { return FabAction::Keep; }
    FabAction::Rotate
}

/// 读浮窗指针：(会话 id, 最后提问时刻, force_new——用户已按"新会话"、下次发送强制轮换)
fn read_fab_state() -> (Option<String>, Option<i64>, bool) {
    let row = database::get("SELECT value FROM system_settings WHERE key='fab_session_state'", &[])
        .ok().flatten();
    let Some(v) = row.and_then(|r| r["value"].as_str().map(String::from)) else { return (None, None, false) };
    let Ok(parsed) = serde_json::from_str::<Value>(&v) else { return (None, None, false) };
    let id = parsed["id"].as_str().filter(|s| !s.is_empty()).map(String::from);
    let last = parsed["last"].as_i64();
    (id, last, parsed["force"].as_bool().unwrap_or(false))
}

fn write_fab_state(id: &str, last: i64) {
    database::run(
        "INSERT OR REPLACE INTO system_settings(key, value, updated_at) VALUES('fab_session_state', ?1, datetime('now','localtime'))",
        &[json!({"id": id, "last": last})],
    ).ok();
}

/// 会话行是否存在（列表删除防御）
fn session_row_exists(id: &str) -> bool {
    database::get("SELECT id FROM chat_sessions WHERE id=?1", &[json!(id)]).ok().flatten().is_some()
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

/// 新建快呼会话（占位标题，首条消息落库时由 workflow 改为首问 30 字）
fn create_fab_session() -> String {
    let id = format!("{}{}", FAB_SESSION_PREFIX, uuid::Uuid::new_v4());
    database::run(
        "INSERT INTO chat_sessions(id, title, created_at, updated_at) VALUES(?1, ?2, datetime('now','localtime'), datetime('now','localtime'))",
        &[json!(&id), json!(FAB_TITLE_PLACEHOLDER)],
    ).ok();
    id
}

/// 会话标题：首问正文截 30 字（剥离附件标记块、压空白），对齐 AgentChat 的首问命名惯例
pub(crate) fn fab_title_from_content(content: &str) -> String {
    let body = match content.find("<<<ATTACHMENTS>>>") {
        Some(i) => &content[..i],
        None => content,
    };
    let truncated: String = body.trim().chars().take(30).collect();
    truncated.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 幂等回填：占位标题"快速唤起"的快呼会话 → 首条用户消息前 30 字
fn backfill_fab_titles() {
    let rows = database::all(
        "SELECT s.id, (SELECT m.content FROM chat_messages m WHERE m.session_id=s.id AND m.role='user' ORDER BY m.id LIMIT 1) as first_q \
         FROM chat_sessions s WHERE s.id LIKE 'fab-%' AND s.title=?1",
        &[json!(FAB_TITLE_PLACEHOLDER)],
    ).unwrap_or_default();
    for r in &rows {
        let (Some(id), Some(q)) = (r["id"].as_str(), r["first_q"].as_str()) else { continue };
        let title = fab_title_from_content(q);
        if !title.is_empty() {
            database::run("UPDATE chat_sessions SET title=?1 WHERE id=?2", &[json!(title), json!(id)]).ok();
        }
    }
}

/// 会话状态综合判定（遗留接管 + 已删防御 + 窗口/冻结 decide）
/// 返回 (当前会话 id, 动作)——Rotate 时 id 为既有会话（调用方决定是否新建）
fn fab_evaluate(now: i64) -> (Option<String>, FabAction) {
    let (mut cur_id, mut last, force) = read_fab_state();
    // 显式新会话（懒实体化）：直接判轮换，跳过遗留接管与窗口计算；
    // 新会话行只在用户真正发出下一条消息时由 fab_resolve_session 的 Rotate 分支创建
    if force {
        return (cur_id.filter(|id| session_row_exists(id)), FabAction::Rotate);
    }
    // 升级迁移：无 KV 状态但存在遗留固定会话 → 接管（历史不丢）
    if cur_id.is_none() && legacy_fab_session_exists() {
        database::run(
            "INSERT OR IGNORE INTO chat_sessions(id, title) VALUES(?1, '小精灵对话')",
            &[json!(FAB_LEGACY_SESSION)],
        ).ok();
        cur_id = Some(FAB_LEGACY_SESSION.to_string());
        last = None;
    }
    // 当前会话已被智能问答页删除 → 视同无状态（防御）
    if let Some(id) = cur_id.as_deref() {
        if !session_row_exists(id) {
            cur_id = None;
            last = None;
        }
    }
    let pending = cur_id.as_deref().map_or(false, |id| fab_has_pending_clarification(id));
    let action = match (cur_id.as_ref(), last) {
        (Some(_), Some(l)) => decide_fab_rotation(now, l, pending),
        (Some(_), None) => FabAction::Keep, // 刚接管/时间戳缺失：视为窗口内
        (None, _) => FabAction::Rotate,
    };
    (cur_id, action)
}

/// 浮窗发送前的轮转判定（唯一调用点 = ChatWindow.send）。
/// 只读决定不写状态——KV 由 kb_agent_chat 每次发送落（单写者 = B 单流指针）
#[tauri::command]
pub async fn fab_resolve_session() -> Result<Value, String> {
    let now = chrono::Local::now().timestamp();
    let (cur_id, action) = fab_evaluate(now);
    let (id, rotated) = match action {
        FabAction::Keep => (cur_id.unwrap(), false),
        FabAction::Rotate => (create_fab_session(), true),
    };
    Ok(json!({"success": true, "sessionId": id, "rotated": rotated}))
}

/// 浮窗显式开新会话（"新会话"按钮）：**懒实体化**——只给指针打 force 标记，不建会话行；
/// 下一条消息发送时由 fab_resolve_session 的 Rotate 分支建行（点而不发不留空白会话），
/// 打开面板时经 load_history 的 Rotate 门槛显示为空；kb_agent_chat 落指针时自然清除标记
#[tauri::command]
pub async fn fab_new_session() -> Result<Value, String> {
    let (cur, _, _) = read_fab_state();
    database::run(
        "INSERT OR REPLACE INTO system_settings(key, value, updated_at) VALUES('fab_session_state', ?1, datetime('now','localtime'))",
        &[json!({"id": cur.unwrap_or_default(), "last": chrono::Local::now().timestamp(), "force": true})],
    )
    .map_err(|e| e.to_string())?;
    Ok(json!({"success": true}))
}

/// 当前对话 id（只读；历史回放兜底——无状态时先看遗留固定会话）
fn current_fab_session_id() -> Option<String> {
    match read_fab_state().0 {
        Some(id) => Some(id),
        None if legacy_fab_session_exists() => Some(FAB_LEGACY_SESSION.to_string()),
        None => None,
    }
}

/// 清空全部快呼会话（含轮换前的旧会话与遗留 fab-main-session）
#[tauri::command]
pub async fn kb_clear_fab_sessions() -> Result<Value, String> {
    // ★ 生成文档联动：与 kb_delete_session 同构（此前直接删消息导致 fab 生成的文档成孤儿泄漏）
    let fab_sids: Vec<String> = database::all(
        "SELECT id FROM chat_sessions WHERE id LIKE 'fab-%'", &[],
    )
    .unwrap_or_default().iter()
    .filter_map(|r| r["id"].as_str().map(String::from))
    .collect();
    let doc_file_ids = crate::services::doc_writer::collect_generated_file_ids(&fab_sids);
    database::run("DELETE FROM chat_messages WHERE session_id LIKE 'fab-%'", &[]).map_err(|e| e.to_string())?;
    database::run("DELETE FROM chat_sessions WHERE id LIKE 'fab-%'", &[]).map_err(|e| e.to_string())?;
    database::run("DELETE FROM system_settings WHERE key='fab_session_state'", &[]).map_err(|e| e.to_string())?;
    crate::services::doc_writer::release_generated_for_sessions(&fab_sids, &doc_file_ids);
    Ok(json!({"success": true}))
}

/// 加载当前对话的历史消息（支持 before_id 游标分页）；session_id 缺省走全局指针
/// 全量加载（before_id=None，召唤/挂载路径）设显示门槛：此刻发送会轮转的会话不展示
/// （超 30min 打开不淡显上一轮）；面板内翻页（before_id=Some）不设门槛，阅读不中断
#[tauri::command]
pub async fn kb_fab_load_history(session_id: Option<String>, limit: Option<usize>, before_id: Option<i64>) -> Result<Value, String> {
    let sid = session_id.or_else(current_fab_session_id);
    if before_id.is_none() {
        let (_, action) = fab_evaluate(chrono::Local::now().timestamp());
        if action == FabAction::Rotate {
            return Ok(json!({"success": true, "sessionId": sid, "messages": [], "stale": true}));
        }
    }
    let Some(sid) = sid else {
        return Ok(json!({"success": true, "sessionId": Value::Null, "messages": []}));
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
    Ok(json!({"success": true, "sessionId": sid, "messages": messages}))
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
    // 清理没有任何消息的空 session（含快呼会话：召唤未提问产生的空条目）
    database::run(
        "DELETE FROM chat_sessions WHERE id NOT IN (SELECT DISTINCT session_id FROM chat_messages)",
        &[]
    ).ok();
    // 存量快呼会话标题回填（占位 → 首问 30 字，幂等；会话已进列表须可辨识）
    backfill_fab_titles();
}

/// 消息撤回：锚定用户消息（原文匹配，附件消息按前缀）及其后全部逻辑删除（status='withdrawn'），
/// 并对跨轮元数据全面断代——tool_metadata（上下文工具注入源）/ session_state（工作记忆）/
/// 压缩摘要 一并清除、compressed 消息还原，撤回后的世界在每条上下文通道上都成立。
/// delete_files=true 时顺带删除范围内 AI 生成文件（走会话释放链，引用计数保护与手动删除同规则）。
#[tauri::command]
pub async fn kb_withdraw_messages(session_id: String, anchor_text: String, delete_files: Option<bool>) -> Result<Value, String> {
    let anchor = anchor_text.trim();
    if anchor.is_empty() { return Err("撤回锚点为空".into()); }
    let pattern = format!("{}\n\n<<<ATTACHMENTS>>>%", crate::database::escape_like(anchor));
    let row = database::get(
        "SELECT id FROM chat_messages WHERE session_id=?1 AND role='user' AND (content=?2 OR content LIKE ?3 ESCAPE '\\') ORDER BY id DESC LIMIT 1",
        &[json!(&session_id), json!(anchor), json!(&pattern)],
    ).map_err(|e| e.to_string())?.ok_or("未找到要撤回的消息")?;
    let from_id = row["id"].as_i64().ok_or("消息 id 异常")?;

    // 范围内生成文件（撤回标记前后均可，metadata 不受 status 影响）
    let mut files: Vec<Value> = Vec::new();
    let meta_rows = database::all(
        "SELECT metadata FROM chat_messages WHERE session_id=?1 AND id>=?2 AND role='assistant' AND metadata IS NOT NULL AND metadata != ''",
        &[json!(&session_id), json!(from_id)],
    ).unwrap_or_default();
    for mr in &meta_rows {
        let Ok(m) = serde_json::from_str::<Value>(mr["metadata"].as_str().unwrap_or("")) else { continue };
        if let Some(docs) = m["documents"].as_array() {
            for d in docs {
                if let Some(fid) = d["file_id"].as_str() {
                    files.push(json!({"id": fid, "name": d["file_name"].as_str().unwrap_or("")}));
                }
            }
        }
    }
    let mut files_deleted = 0usize;
    if delete_files == Some(true) && !files.is_empty() {
        let ids: Vec<String> = files.iter().filter_map(|f| f["id"].as_str().map(String::from)).collect();
        files_deleted = ids.len();
        crate::services::doc_writer::release_generated_for_sessions(&[session_id.clone()], &ids);
    }

    let withdrawn = database::run(
        "UPDATE chat_messages SET status='withdrawn' WHERE session_id=?1 AND id>=?2 AND status='active'",
        &[json!(&session_id), json!(from_id)],
    ).map(|r| r.changes).unwrap_or(0);
    let _ = database::run(
        "UPDATE chat_sessions SET tool_metadata='{}', session_state=NULL, context_metadata=NULL, updated_at=datetime('now','localtime') WHERE id=?1",
        &[json!(&session_id)],
    );
    let _ = database::run(
        "UPDATE chat_messages SET status='active' WHERE session_id=?1 AND status='compressed'",
        &[json!(&session_id)],
    );
    Ok(json!({ "success": true, "withdrawn_count": withdrawn, "files": files, "files_deleted": files_deleted }))
}

/// 会话生成文件清单（交付抽屉数据源）：与会话消息存活无关，附"来源是否仍在会话中"标记
#[tauri::command]
pub async fn kb_session_files(session_id: String) -> Result<Value, String> {
    let rows = database::all(
        "SELECT fa.id AS id, fa.original_name AS name, fa.file_ext AS ext, fa.file_size AS size, fr.created_at AS created_at, \
         EXISTS(SELECT 1 FROM chat_messages m WHERE m.session_id=?1 AND m.status='active' AND instr(COALESCE(m.metadata,''), fa.id)>0) AS live_src \
         FROM file_relations fr JOIN file_assets fa ON fa.id=fr.file_id \
         WHERE fr.entity_type='generated' AND fr.entity_id=?1 ORDER BY fr.created_at DESC",
        &[json!(&session_id)],
    ).map_err(|e| e.to_string())?;
    Ok(json!({ "success": true, "files": rows }))
}

/// 单文件存在性（消息内文档卡片删除后灰壳判定）
#[tauri::command]
pub async fn kb_file_exists(file_id: String) -> Result<Value, String> {
    let c: i64 = database::get("SELECT COUNT(*) AS c FROM file_assets WHERE id=?1", &[json!(&file_id)])
        .ok().flatten().and_then(|r| r["c"].as_i64()).unwrap_or(0);
    Ok(json!({ "success": true, "exists": c > 0 }))
}

/// 单条释放会话生成文件（消息卡/抽屉删除按钮，asset 语义）：清该资产全部 generated 关系；
/// 无 generated 残留时清内容快照并按剩余引用（资料库/项目/邮件等）决定物理回收——与手动删除同规则
#[tauri::command]
pub async fn kb_release_generated_file(file_id: String) -> Result<Value, String> {
    let _ = database::run(
        "DELETE FROM file_relations WHERE file_id=?1 AND entity_type='generated'", &[json!(&file_id)]);
    let remains: i64 = database::get(
        "SELECT COUNT(*) AS c FROM file_relations WHERE file_id=?1 AND entity_type='generated'", &[json!(&file_id)],
    ).ok().flatten().and_then(|r| r["c"].as_i64()).unwrap_or(0);
    if remains == 0 {
        let _ = database::run("DELETE FROM generated_docs WHERE file_id=?1", &[json!(&file_id)]);
        crate::commands::file_commands::release_asset_if_orphan(&file_id);
    }
    Ok(json!({ "success": true }))
}

#[cfg(test)]
mod fab_rotation_tests {
    use super::{decide_fab_rotation, fab_title_from_content, FabAction};

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

    #[test]
    fn title_strips_attachments_marker() {
        assert_eq!(fab_title_from_content("帮我看下这份合同\n<<<ATTACHMENTS>>>\n[附件：a.pdf]"), "帮我看下这份合同");
    }

    #[test]
    fn title_truncates_to_30_chars() {
        let long = "问".repeat(40);
        let t = fab_title_from_content(&long);
        assert_eq!(t.chars().count(), 30);
    }

    #[test]
    fn title_collapses_whitespace() {
        assert_eq!(fab_title_from_content("  a\n\n b\tc  "), "a b c");
    }

    #[test]
    fn title_blank_is_empty() {
        assert_eq!(fab_title_from_content("   \n  "), "");
        assert_eq!(fab_title_from_content("\n<<<ATTACHMENTS>>>\nx"), "");
    }
}
