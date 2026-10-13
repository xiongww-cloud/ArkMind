use crate::database;
use serde_json::{json, Value};

/// 剥除通知标题/内容中暴露的对象 UUID，避免泄露内部标识符；标题强制单行化（合并换行为空格）
pub(crate) fn sanitize_notification_text(text: &str) -> String {
    let re = regex::Regex::new(r"(?i)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}").unwrap();
    let cleaned = re.replace_all(text, "").to_string();
    cleaned.replace('\r', "").replace('\n', " ").split_whitespace().collect::<Vec<_>>().join(" ")
}

#[tauri::command]
pub async fn notification_list(unread_only: Option<bool>, state_filter: Option<String>) -> Result<Value, String> {
    // state_filter: None/"pending"=待处理流（未处置，含已读）；"done"/"dismissed"=细分已处置流；"handled"=已处置合集；"all"=不过滤
    let state_cond = match state_filter.as_deref() {
        Some("done") => "user_state='done'",
        Some("dismissed") => "user_state='dismissed'",
        Some("handled") => "user_state!=''",
        Some("all") => "1=1",
        _ => "user_state=''",
    };
    // 未处置（待办语义）通知置顶：全部/已处置流混合展示时，"还需要你处理"的永远排在前
    let order = "CASE WHEN user_state='' THEN 0 ELSE 1 END, read ASC, created_at DESC";
    let sql = if unread_only.unwrap_or(false) {
        &format!("SELECT * FROM notifications WHERE read=0 AND {} ORDER BY {}", state_cond, order)
    } else {
        &format!("SELECT * FROM notifications WHERE {} ORDER BY {}", state_cond, order)
    };
    let mut rows = database::all(sql, &[]).map_err(|e| e.to_string())?;
    // ★ actions 从 JSON 字符串解析为数组
    for row in rows.iter_mut() {
        if let Some(actions_str) = row.get("actions").and_then(|v| v.as_str()) {
            if let Ok(parsed) = serde_json::from_str::<Value>(actions_str) {
                row["actions"] = parsed;
            }
        }
    }
    Ok(json!({ "success": true, "notifications": rows }))
}

/// 用户处置通知：done=现实里已办结（勿再报未办，同对象新变化照常跟踪）
/// dismissed=不再跟进（该追踪撤销，无新输入不得再提）；''=恢复回待处理流。
/// 处置写 state_changed_at 时刻（时间窗注入：本轮被处置的旧通知也要让分析师立即看见）；
/// 观察池联动不在这里——通知创建时条目即出队（tools/notification.rs），处置只裁决通知本身
#[tauri::command]
pub async fn notification_set_state(id: String, state: String) -> Result<(), String> {
    const ALLOWED: [&str; 3] = ["", "done", "dismissed"];
    if !ALLOWED.contains(&state.as_str()) { return Err(format!("非法处置状态: {}", state)); }
    if state.is_empty() {
        database::run("UPDATE notifications SET user_state='' WHERE id=?", &[json!(id)])
            .map_err(|e| e.to_string())?;
    } else {
        database::run("UPDATE notifications SET user_state=?1, state_changed_at=datetime('now','localtime') WHERE id=?2",
            &[json!(state), json!(id)]).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 继续跟进：用通知标题构造观察条目再入用户区（通知是追踪事项的最新状态）。
/// 池不复活旧条目、新建条目（新 id 新起算）。
/// ★ 处置联动：续盯=用户对"本轮通知"的终局裁决（追踪意愿已另行入池表达），同置 user_state='done'
/// 并标已读——一点即办结离开待处理流，免二次点击；已在池中（重复点击）同样落 done。
#[tauri::command]
pub async fn notification_watch_keep(id: String) -> Result<Value, String> {
    let row = database::get("SELECT title, entity_type, entity_id, user_state FROM notifications WHERE id=?", &[json!(id)])
        .map_err(|e| e.to_string())?
        .ok_or("通知不存在")?;
    let title = row["title"].as_str().unwrap_or("").to_string();
    let ent = match (row["entity_type"].as_str().unwrap_or(""), row["entity_id"].as_str().unwrap_or("")) {
        (t, i) if !t.is_empty() && !i.is_empty() => crate::services::agent_workflow::tools::entity_ref::fmt(t, i),
        _ => String::new(),
    };
    let new_id = match crate::services::notification_loop::watch_reinject(&title, &ent) {
        Ok(id) => Some(id),
        // 已在池中完全相同的观察（重复点击续盯）：幂等成功，非错误
        Err(e) if e.contains("完全相同") => None,
        Err(e) => return Err(e),
    };
    database::run("UPDATE notifications SET read=1, user_state='done', state_changed_at=datetime('now','localtime') WHERE id=?", &[json!(id)]).map_err(|e| e.to_string())?;
    match new_id {
        Some(wid) => Ok(json!({ "success": true, "watch_id": wid })),
        None => Ok(json!({ "success": true, "already": true })),
    }
}

/// 观察池结构化内容（Settings 双分区只读展示与主 Agent 工具共用）
#[tauri::command]
pub async fn watch_pool_get() -> Result<Value, String> {
    Ok(json!({ "success": true, "entries": crate::services::notification_loop::watch_pool_structured() }))
}

/// Settings 直接删除观察条目（无勾选无静默——单纯删除追踪；话题再出现由分析师按新输入判断）
#[tauri::command]
pub async fn watch_entry_delete(watch_id: u64) -> Result<(), String> {
    crate::services::notification_loop::watch_remove(watch_id)
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
    // 角标计数=未处置通知（待处理流全集）：弹窗已无手动已读入口，read 不再参与角标语义，
    // 一键处理/已读过的未办结通知必须继续计入，直到办结/忽略才减数
    let unread: i64 = database::with_db(|c| c.query_row(
        "SELECT COUNT(*) FROM notifications WHERE user_state=''", [], |r| r.get(0)
    )).unwrap_or(0);
    // 处置计数：done=洞察被采纳的信号，dismissed=噪音信号——供后续通知质量校准
    let done_count: i64 = database::with_db(|c| c.query_row(
        "SELECT COUNT(*) FROM notifications WHERE user_state='done'", [], |r| r.get(0)
    )).unwrap_or(0);
    let dismissed_count: i64 = database::with_db(|c| c.query_row(
        "SELECT COUNT(*) FROM notifications WHERE user_state='dismissed'", [], |r| r.get(0)
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
        "done_count": done_count,
        "dismissed_count": dismissed_count,
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

/// 用户从通知的 create_project_log 按钮确认记录业务事件到项目动态
#[tauri::command]
pub async fn notification_create_project_log(project_id: String, content: String, notification_id: Option<String>) -> Result<Value, String> {
    let pid = project_id.trim().strip_prefix("PROJECT-").unwrap_or(project_id.trim()).to_string();
    if pid.is_empty() { return Err("project_id 不能为空".into()); }
    let c = content.trim().to_string();
    if c.is_empty() { return Err("日志内容不能为空".into()); }
    // 调用已有的 project_dynamics_add
    let event = json!({"type": "ai", "content": c, "operator": "AI"});
    let result = crate::commands::project_commands::project_dynamics_add(pid.clone(), event).await.map_err(|e| e.to_string())?;
    // 标记通知已处理（如有 notification_id）
    if let Some(nid) = notification_id {
        database::run("UPDATE notifications SET read=1 WHERE id=?1", &[json!(nid)]).ok();
    }
    Ok(json!({"success": true, "dynamic": result}))
}

/// link_file 目标类型白名单→(表, ID 列)（按钮创建侧 sanitize 与点击落库校验共用）
pub(crate) fn link_target_table(et_upper: &str) -> Option<(&'static str, &'static str)> {
    let et_norm = crate::services::agent_workflow::tools::entity_ref::normalize_type(et_upper);
    match et_norm.as_str() {
        "PROJECT" => Some(("projects", "id")),
        "PRODUCT" => Some(("products", "id")),
        "EQUIPMENT_LIST" => Some(("solutions", "id")),
        "CHANNEL" | "VENDOR" | "CUSTOMER" => Some(("partners", "id")),
        "CONTACT" => Some(("partner_contacts", "id")),
        "NOTE" => Some(("notes", "note_id")),
        "TODO" => Some(("todos", "id")),
        _ => None,
    }
}

/// 单条 文件↔对象 关联的落库核心（单条与批量命令共用）。
/// Ok(true)=新写边，Ok(false)=已存在（幂等跳过），Err=校验拒绝（不落库）
fn apply_file_link(fid: &str, fname: &str, et_upper: &str, eid: &str) -> Result<bool, String> {
    let (table, id_col) = link_target_table(et_upper).ok_or_else(|| format!("不支持的关联目标类型：{}", et_upper))?;
    let obj_ok = database::get(
        &format!("SELECT 1 FROM {} WHERE {}=?1 LIMIT 1", table, id_col), &[json!(eid)]
    ).ok().flatten().is_some();
    if !obj_ok { return Err("关联目标对象不存在".into()); }
    let dup = database::get(
        "SELECT 1 FROM file_relations WHERE file_id=?1 AND upper(entity_type)=?2 AND entity_id=?3 LIMIT 1",
        &[json!(fid), json!(et_upper), json!(eid)]
    ).ok().flatten().is_some();
    if dup { return Ok(false); }
    crate::commands::file_commands::add_relation(fid, &et_upper.to_lowercase(), eid, fname, "AI", None);
    crate::services::knowledge_lifecycle::notify_changed(et_upper, eid); // 关联变化尽力通知下游（失败无害）
    Ok(true)
}

fn file_name_if_exists(fid: &str) -> Option<String> {
    database::get("SELECT original_name FROM file_assets WHERE id=?1", &[json!(fid)])
        .ok().flatten()
        .and_then(|r| r["original_name"].as_str().filter(|s| !s.is_empty()).map(String::from))
}

/// 用户从通知的 link_file 按钮确认：把文件关联到业务对象（分析师建议、用户点击才写边）。
/// 幂等：已存在的关联返回 already=true 不重复写；文件/对象任一不存在拒绝（防死按钮）。
#[tauri::command]
pub async fn notification_link_file(file_id: String, entity_type: String, entity_id: String, notification_id: Option<String>) -> Result<Value, String> {
    let fid = strip_type_prefix(file_id.trim());
    let et = entity_type.trim().to_uppercase();
    let eid = strip_type_prefix(entity_id.trim());
    if fid.is_empty() || eid.is_empty() { return Err("关联参数不完整".into()); }
    let fname = file_name_if_exists(&fid).ok_or_else(|| "来源文件不存在".to_string())?;
    let created = apply_file_link(&fid, &fname, &et, &eid)?;
    if let Some(nid) = notification_id.as_deref() {
        database::run("UPDATE notifications SET read=1 WHERE id=?1", &[json!(nid)]).ok();
    }
    Ok(json!({"success": true, "already": !created}))
}

/// 批量版：一个按钮一次确认写多条边（link_file params.links 数组形态）。
/// 逐条独立校验幂等，坏条不连累好条；至少落一条即标记通知已读。
#[tauri::command]
pub async fn notification_link_files(file_id: String, links: Vec<Value>, notification_id: Option<String>) -> Result<Value, String> {
    let fid = strip_type_prefix(file_id.trim());
    if fid.is_empty() || links.is_empty() { return Err("关联参数不完整".into()); }
    let fname = file_name_if_exists(&fid).ok_or_else(|| "来源文件不存在".to_string())?;
    let (mut linked, mut already) = (0usize, 0usize);
    let mut failed: Vec<Value> = Vec::new();
    for l in links.iter().take(10) {
        let et = l["entity_type"].as_str().unwrap_or("").trim().to_uppercase();
        let eid = strip_type_prefix(l["entity_id"].as_str().unwrap_or("").trim());
        if eid.is_empty() { continue; }
        match apply_file_link(&fid, &fname, &et, &eid) {
            Ok(true) => linked += 1,
            Ok(false) => already += 1,
            Err(e) => failed.push(json!({"entity_type": et, "entity_id": eid, "error": e})),
        }
    }
    if linked + already > 0 {
        if let Some(nid) = notification_id.as_deref() {
            database::run("UPDATE notifications SET read=1 WHERE id=?1", &[json!(nid)]).ok();
        }
    }
    let all_failed = linked == 0 && already == 0 && !failed.is_empty();
    Ok(json!({"success": !all_failed, "linked": linked, "already": already, "failed": failed}))
}

/// 剥已知 TYPE- 前缀取裸 id（entity_ref 单一入口的薄包装，保持既有调用与测试稳定）
pub(crate) fn strip_type_prefix(s: &str) -> String {
    crate::services::agent_workflow::tools::entity_ref::bare_id(s)
}

/// update_object 按钮：各实体类型允许 AI 建议更新的字段白名单（键名即 updates 入参键）
pub(crate) fn update_object_field_whitelist(et_upper: &str) -> &'static [&'static str] {
    match et_upper {
        "PROJECT" => &["name", "description", "budget", "bid_time", "expected_order_date", "owner", "status"],
        "PRODUCT" => &["name", "description", "full_specs"],
        "CHANNEL" | "VENDOR" | "CUSTOMER" => &["name", "phone", "address", "remark", "contact_person", "contact_phone"],
        _ => &[],
    }
}

/// 按白名单过滤 updates（纯函数）：剔除白名单外键、空串/null；evidence 透传供动态记录
pub(crate) fn filter_update_fields(et_upper: &str, updates: &Value) -> serde_json::Map<String, Value> {
    let mut out = serde_json::Map::new();
    for key in update_object_field_whitelist(et_upper) {
        if let Some(v) = updates.get(*key) {
            if v.is_null() { continue; }
            if v.is_string() && v.as_str().unwrap_or("").trim().is_empty() { continue; }
            out.insert((*key).to_string(), v.clone());
        }
    }
    out
}

/// 用户从通知的 update_object 按钮确认：按 AI 洞察建议更新对象信息（受信任落库，白名单字段）。
/// PROJECT 的 status 走阶段状态机命令；项目类变更自动记一条项目动态（附证据）。
#[tauri::command]
pub async fn notification_update_object(entity_type: String, entity_id: String, updates: Value, notification_id: Option<String>) -> Result<Value, String> {
    let et = entity_type.trim().to_uppercase();
    let eid = strip_type_prefix(entity_id.trim());
    if eid.is_empty() || !updates.is_object() { return Err("更新参数不完整".into()); }
    let clean = filter_update_fields(&et, &updates);
    if clean.is_empty() { return Err("没有可更新的有效字段".into()); }
    let (table, id_col) = link_target_table(&et).ok_or_else(|| format!("该类型不支持一键更新：{}", et))?;
    let exists = database::get(
        &format!("SELECT 1 FROM {} WHERE {}=?1 LIMIT 1", table, id_col), &[json!(&eid)]
    ).ok().flatten().is_some();
    if !exists { return Err("目标对象不存在或已被删除".into()); }
    let evidence = updates["evidence"].as_str().unwrap_or("").trim().to_string();
    let summary: Vec<String> = clean.iter()
        .map(|(k, v)| format!("{}→{}", k, v.as_str().unwrap_or("")))
        .collect();
    match et.as_str() {
        "PROJECT" => {
            let mut fields = clean.clone();
            if let Some(st) = fields.remove("status") {
                let s = st.as_str().unwrap_or("").trim().to_string();
                if !s.is_empty() {
                    crate::commands::project_commands::project_update_status(eid.clone(), s, "AI".into()).await?;
                }
            }
            if !fields.is_empty() {
                crate::commands::project_commands::project_update(eid.clone(), Value::Object(fields)).await?;
            }
            let content = if evidence.is_empty() {
                format!("AI 洞察更新：{}", summary.join("；"))
            } else {
                format!("AI 洞察更新：{}（依据：{}）", summary.join("；"), evidence)
            };
            crate::commands::project_commands::project_dynamics_add(eid.clone(), json!({"type": "ai", "content": content, "operator": "AI"})).await.ok();
        }
        "PRODUCT" => {
            let mut data = serde_json::Map::new();
            if let Some(v) = clean.get("name") { data.insert("name".into(), v.clone()); }
            if let Some(v) = clean.get("description") { data.insert("description".into(), v.clone()); }
            if let Some(v) = clean.get("full_specs") { data.insert("fullSpecs".into(), v.clone()); }
            crate::commands::product_commands::product_update(eid.clone(), Value::Object(data)).await?;
        }
        _ => {
            let mut data = Value::Object(clean.clone());
            data["id"] = json!(&eid);
            crate::commands::partner_commands::partner_update(data).await.map(|_| ())?;
        }
    }
    if let Some(nid) = notification_id.as_deref() {
        database::run("UPDATE notifications SET read=1 WHERE id=?1", &[json!(nid)]).ok();
    }
    log::info!("[通知] update_object 已应用: {} {} → {:?}", et, eid, summary);
    Ok(json!({"success": true}))
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

#[cfg(test)]
mod tests {
    use super::{link_target_table, notification_watch_keep, strip_type_prefix};

    #[test]
    fn strip_type_prefix_only_for_uppercase_head() {
        assert_eq!(strip_type_prefix("FILE-abc"), "abc");
        assert_eq!(strip_type_prefix("PROJECT-abc"), "abc");
        assert_eq!(strip_type_prefix("abc-def"), "abc-def");
        assert_eq!(strip_type_prefix("abc"), "abc");
        assert_eq!(strip_type_prefix("-abc"), "-abc");
    }

    /// 续盯=一点即办结：入池成功同时落 user_state='done'+已读（免得用户再点第二次办结）；重复点击幂等
    #[tokio::test]
    async fn watch_keep_also_finalizes_notification_as_done() {
        let (_d, _g) = crate::database::db_core::test_util::setup_test_db();
        crate::database::with_db(|c| crate::database::db_schema::create_schema(c)).unwrap();
        crate::database::run(
            "INSERT INTO notifications(id,type,title,user_state,read) VALUES('n1','insight','盯一下咸宁回款','',0)", &[]
        ).unwrap();
        let r = notification_watch_keep("n1".into()).await.unwrap();
        assert!(r["success"].as_bool().unwrap());
        let row = crate::database::get("SELECT user_state, read FROM notifications WHERE id='n1'", &[]).unwrap().unwrap();
        assert_eq!(row["user_state"].as_str(), Some("done"), "续盯应自动办结");
        assert_eq!(row["read"].as_i64(), Some(1));
        // 重复点击：池去重走 already 分支，处置仍幂等成功
        let r2 = notification_watch_keep("n1".into()).await.unwrap();
        assert!(r2["success"].as_bool().unwrap(), "already 分支同样成功");
    }

    #[test]
    fn link_target_whitelist() {
        assert_eq!(link_target_table("PROJECT"), Some(("projects", "id")));
        assert_eq!(link_target_table("NOTE"), Some(("notes", "note_id")));
        assert_eq!(link_target_table("VENDOR"), Some(("partners", "id")));
        assert_eq!(link_target_table("EMAIL"), None, "邮件不走 link_file");
        assert_eq!(link_target_table("file"), None);
    }

    /// update_object 白名单过滤：越权字段剔除、空值剔除、evidence 不进 updates
    #[test]
    fn filter_update_fields_whitelists() {
        use super::filter_update_fields;
        let u = serde_json::json!({
            "bid_time": "2026-10-20 09:30:00", "status": "Won", "id": "hacked", "created_at": "x",
            "evidence": "邮件原文", "remark": "", "budget": null
        });
        let out = filter_update_fields("PROJECT", &u);
        assert_eq!(out.len(), 2);
        assert_eq!(out["bid_time"], "2026-10-20 09:30:00");
        assert_eq!(out["status"], "Won");
        assert!(!out.contains_key("id") && !out.contains_key("evidence") && !out.contains_key("remark"));
        assert!(filter_update_fields("CONTACT", &u).is_empty(), "未登记类型无白名单");
        assert!(filter_update_fields("PRODUCT", &serde_json::json!({"phone": "1"})).is_empty(), "字段与类型不匹配剔除");
    }
}
