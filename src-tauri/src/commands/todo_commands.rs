use crate::database;
use chrono::Datelike;
use serde_json::{json, Value};

/// 查询待办列表 — 自动重置到期的循环待办 + 通过 todo_project_mapping LEFT JOIN projects 获取项目名
#[tauri::command]
pub async fn todos_list() -> Result<Value, String> {
    // ★ 到期克隆：已完成的循环待办 due_date 已到 → 克隆新周期 + 删除原待办
    let expired: Vec<Value> = database::all(
        "SELECT * FROM todos WHERE done=1 AND repeat IS NOT NULL AND repeat!='' AND date(due_date) <= date('now','localtime')",
        &[],
    ).unwrap_or_default();
    for row in &expired {
        let old_id = match row["id"].as_str() { Some(id) => id.to_string(), None => continue };
        let title = row["title"].as_str().unwrap_or("");
        let content = row["content"].as_str().unwrap_or("");
        let priority = row["priority"].as_str();
        let repeat = row["repeat"].as_str().unwrap_or("");
        let due = match row["due_date"].as_str() { Some(d) => d, None => continue };
        if let Some(next) = next_due_date(due, repeat) {
            let new_id = uuid::Uuid::new_v4().to_string();
            database::with_db(|conn| {
                conn.execute(
                    "INSERT INTO todos(id,title,content,priority,due_date,repeat) VALUES(?1,?2,?3,?4,?5,?6)",
                    rusqlite::params![new_id, title, content, priority, next, repeat],
                )?;
                conn.execute(
                    "INSERT INTO todo_project_mapping(todo_id,project_id) SELECT ?1,project_id FROM todo_project_mapping WHERE todo_id=?2",
                    rusqlite::params![new_id, old_id],
                ).ok();
                // ★ 关键：更新旧待办的 due_date 以防止重复克隆
                conn.execute("UPDATE todos SET due_date=?1, updated_at=datetime('now','localtime') WHERE id=?2",
                    rusqlite::params![next, old_id],
                )?;
                Ok(())
            }).ok();
        }
    }

    let todos = database::all(
        "SELECT t.*, tpm.project_id, p.name as project_name FROM todos t \
         LEFT JOIN todo_project_mapping tpm ON t.id = tpm.todo_id \
         LEFT JOIN projects p ON p.id = tpm.project_id \
         GROUP BY t.id \
         ORDER BY t.done ASC, t.due_date ASC, t.priority ASC, t.created_at DESC",
        &[],
    )
    .map_err(|e| e.to_string())?;
    Ok(json!({ "success": true, "todos": todos }))
}

/// 待办优先级统一归一：中/英/大小写/P1-P3 诸写法 → 规范值域 P1/P2/P3；空视为未设置；无法识别的原样保留
pub(crate) fn normalize_todo_priority(p: &str) -> Option<String> {
    let t = p.trim();
    if t.is_empty() {
        return None;
    }
    let lower = t.to_lowercase();
    Some(match t {
        "最高" | "高" | "紧急" => "P1",
        "中" | "一般" => "P2",
        "低" => "P3",
        _ => match lower.as_str() {
            "high" | "urgent" | "p1" => "P1",
            "medium" | "normal" => "P2",
            "low" | "p3" => "P3",
            "p2" => "P2",
            other => other,
        },
    }.to_string())
}

#[tauri::command]
pub async fn todos_create(data: Value) -> Result<Value, String> {
    let title = data["title"].as_str().unwrap_or("新待办").to_string();
    let content = data["content"].as_str().unwrap_or("").to_string();
    let priority = data["priority"].as_str().and_then(normalize_todo_priority);
    let due_date = data["dueDate"].as_str().map(|s| s.to_string());
    let project_id = data["projectId"].as_str().map(|s| s.to_string());
    let repeat = data["repeat"].as_str().map(|s| s.to_string());

    let id = uuid::Uuid::new_v4().to_string();
    let mut linked_pid: Option<String> = None;
    database::with_db(|conn| {
        conn.execute("BEGIN", [])?;
        let r = conn.execute(
            "INSERT INTO todos(id,title,content,priority,due_date,repeat) VALUES(?1,?2,?3,?4,?5,?6)",
            rusqlite::params![id, title, content, priority, due_date, repeat],
        );
        if let Err(e) = r {
            let _ = conn.execute("ROLLBACK", []);
            return Err(e);
        }
        if let Some(ref pid) = project_id {
            if !pid.is_empty() {
                conn.execute(
                    "INSERT OR IGNORE INTO todo_project_mapping(todo_id, project_id) VALUES(?1, ?2)",
                    rusqlite::params![id, pid],
                )?;
                linked_pid = Some(pid.clone());
            }
        }
        conn.execute("COMMIT", [])
    })
    .map_err(|e| {
        let msg = format!("创建待办失败: {}", e);
        eprintln!("{}", msg);
        msg
    })?;

    // 新建待办关联项目 → 刷新项目活跃时间（必须在 with_db 闭包外调用，避免死锁）
    if let Some(ref pid) = linked_pid {
        crate::commands::project_commands::touch_project(pid);
    }

    let todo = database::get(
        "SELECT t.*, tpm.project_id, p.name as project_name FROM todos t \
         LEFT JOIN todo_project_mapping tpm ON t.id = tpm.todo_id \
         LEFT JOIN projects p ON p.id = tpm.project_id WHERE t.id=?",
        &[json!(&id)],
    )
    .map_err(|e| e.to_string())?
    .unwrap_or(json!({}));
    crate::services::knowledge_lifecycle::notify_changed("TODO", &id);
    Ok(json!({ "success": true, "todo": todo }))
}

#[tauri::command]
pub async fn todos_update(id: String, data: Value) -> Result<Value, String> {
    let mut fields: Vec<String> = Vec::new();
    let mut values: Vec<Value> = Vec::new();

    if let Some(v) = data["title"].as_str() { fields.push("title=?".into()); values.push(json!(v)); }
    if let Some(v) = data["content"].as_str() { fields.push("content=?".into()); values.push(json!(v)); }
    if let Some(v) = data["done"].as_bool() {
        fields.push("done=?".into()); values.push(json!(v as i32));
    } else if let Some(v) = data["done"].as_i64() {
        fields.push("done=?".into()); values.push(json!(if v != 0 { 1i32 } else { 0i32 }));
    }
    // ★ 待办完成检测：仅 false→true 翻转视为"完成"，用于刷新关联项目活跃时间
    let completing = {
        let new_done = data["done"].as_bool().or_else(|| data["done"].as_i64().map(|n| n != 0));
        if new_done == Some(true) {
            let old_done = database::get("SELECT done FROM todos WHERE id=?", &[json!(&id)])
                .ok().flatten().and_then(|r| r["done"].as_i64()).unwrap_or(0) != 0;
            !old_done
        } else { false }
    };
    if let Some(p) = data.get("priority") {
        if p.is_null() || p.as_str().map(|s| s.is_empty()).unwrap_or(true) {
            fields.push("priority=NULL".into());
        } else if let Some(v) = p.as_str() {
            if let Some(n) = normalize_todo_priority(v) {
                fields.push("priority=?".into()); values.push(json!(n));
            }
        }
    }
    if let Some(dd) = data.get("dueDate") {
        if dd.is_null() || dd.as_str().map(|s| s.is_empty()).unwrap_or(true) {
            fields.push("due_date=NULL".into());
        } else if let Some(v) = dd.as_str() {
            fields.push("due_date=?".into()); values.push(json!(v));
        }
    }
    if let Some(rp) = data.get("repeat") {
        if rp.is_null() || rp.as_str().map(|s| s.is_empty()).unwrap_or(true) {
            fields.push("repeat=NULL".into());
        } else if let Some(v) = rp.as_str() {
            fields.push("repeat=?".into()); values.push(json!(v));
        }
    }

    // ★ Sync todo_project_mapping when projectId changes
    let update_project = !data["projectId"].is_null();
    let mut pid_newly_linked: Option<String> = None;
    if update_project {
        let new_pid = data["projectId"].as_str().map(|s| s.to_string());
        database::with_db(|conn| {
            // 增量比对：仅当项目关联实际变化时才刷新项目活跃时间
            let old_pid: Option<String> = conn.query_row(
                "SELECT project_id FROM todo_project_mapping WHERE todo_id=?1 LIMIT 1",
                rusqlite::params![id], |r| r.get(0),
            ).ok();
            // Remove old mapping(s)
            conn.execute("DELETE FROM todo_project_mapping WHERE todo_id=?1", rusqlite::params![id])?;
            // Insert new mapping if provided
            if let Some(ref pid) = new_pid {
                if !pid.is_empty() {
                    conn.execute(
                        "INSERT OR IGNORE INTO todo_project_mapping(todo_id, project_id) VALUES(?1, ?2)",
                        rusqlite::params![id, pid],
                    )?;
                    if old_pid.as_deref() != Some(pid.as_str()) {
                        pid_newly_linked = Some(pid.clone());
                    }
                }
            }
            Ok(())
        }).map_err(|e| e.to_string())?;
    }
    // 项目关联变化 → 刷新项目活跃时间（with_db 闭包外调用，避免死锁）
    if let Some(ref pid) = pid_newly_linked {
        crate::commands::project_commands::touch_project(pid);
    }

    if !fields.is_empty() {
        fields.push("updated_at=datetime('now', 'localtime')".into());
        values.push(json!(&id));
        let result = database::run(
            &format!("UPDATE todos SET {} WHERE id=?", fields.join(",")),
            &values,
        ).map_err(|e| e.to_string())?;
        if result.changes == 0 { return Err(format!("未找到待办: {}", id)); }
    }

    // 待办完成（false→true）→ 刷新关联项目活跃时间
    if completing {
        if let Ok(Some(r)) = database::get("SELECT project_id FROM todo_project_mapping WHERE todo_id=?1 LIMIT 1", &[json!(&id)]) {
            if let Some(pid) = r["project_id"].as_str() {
                crate::commands::project_commands::touch_project(pid);
            }
        }
    }

    let todo = database::get(
        "SELECT t.*, tpm.project_id, p.name as project_name FROM todos t \
         LEFT JOIN todo_project_mapping tpm ON t.id = tpm.todo_id \
         LEFT JOIN projects p ON p.id = tpm.project_id WHERE t.id=?",
        &[json!(&id)],
    )
    .map_err(|e| e.to_string())?
    .unwrap_or(json!({}));
    crate::services::knowledge_lifecycle::notify_changed("TODO", &id);
    Ok(json!({ "success": true, "todo": todo }))
}

#[tauri::command]
pub async fn todos_delete(id: String) -> Result<Value, String> {
    database::run("DELETE FROM todo_project_mapping WHERE todo_id=?", &[json!(&id)]).ok();
    database::run("DELETE FROM file_relations WHERE entity_type='todo' AND entity_id=?", &[json!(&id)]).ok();
    database::run("DELETE FROM notifications WHERE entity_type='todo' AND entity_id=?", &[json!(&id)]).ok();
    crate::services::knowledge_lifecycle::notify_deleted("TODO", &id);
    crate::services::business_relations::delete_by_source("TODO", &id);
    let result = database::run("DELETE FROM todos WHERE id=?", &[json!(&id)]).map_err(|e| e.to_string())?;
    if result.changes == 0 { return Err(format!("未找到待办: {}", id)); }
    database::save().map_err(|e| e.to_string())?;
    Ok(json!({ "success": true }))
}

fn next_due_date(current: &str, repeat: &str) -> Option<String> {
    let time_suffix = if current.contains('T') {
        chrono::NaiveDateTime::parse_from_str(current, "%Y-%m-%dT%H:%M:%S").ok().map(|dt| format!("T{}", dt.format("%H:%M:%S")))
    } else { None };
    let date = chrono::NaiveDate::parse_from_str(current, "%Y-%m-%d")
        .or_else(|_| {
            chrono::NaiveDateTime::parse_from_str(current, "%Y-%m-%dT%H:%M:%S")
                .map(|dt| dt.date())
        }).ok()?;
    let next = match repeat {
        "daily" => date + chrono::Duration::days(1),
        "weekly" => date + chrono::Duration::days(7),
        "monthly" => {
            let (y, m) = if date.month() == 12 { (date.year() + 1, 1) } else { (date.year(), date.month() + 1) };
            let max_day = chrono::NaiveDate::from_ymd_opt(y, m, 1).unwrap_or(date).pred_opt().map(|d| d.day()).unwrap_or(28);
            let day = date.day().min(max_day);
            chrono::NaiveDate::from_ymd_opt(y, m, day).unwrap_or(date)
        }
        _ => date,
    };
    match time_suffix {
        Some(ts) => Some(format!("{}{}", next.format("%Y-%m-%d"), ts)),
        None => Some(next.format("%Y-%m-%d").to_string()),
    }
}

#[tauri::command]
pub async fn todos_toggle(id: String) -> Result<Value, String> {
    let before = database::get("SELECT * FROM todos WHERE id=?", &[json!(&id)])
        .map_err(|e| e.to_string())?.unwrap_or(json!({}));
    let was_done = before["done"].as_i64().unwrap_or(0) == 1;
    let repeat_val = before["repeat"].as_str().map(|s| s.to_string());
    let _due_val = before["due_date"].as_str().map(|s| s.to_string());

    // Recurring: mark done, keep original due_date for clone-on-due
    if let Some(ref rpt) = repeat_val {
        if !rpt.is_empty() && !was_done {
            database::run(
                "UPDATE todos SET done=1, updated_at=datetime('now', 'localtime') WHERE id=?",
                &[json!(&id)],
            ).map_err(|e| { let m = format!("[todos_toggle] repeat mark done fail: {}", e); eprintln!("{}", m); m })?;
            crate::services::knowledge_lifecycle::notify_changed("TODO", &id);
            let todo = database::get(
                "SELECT t.*, tpm.project_id, p.name as project_name FROM todos t \
                 LEFT JOIN todo_project_mapping tpm ON t.id = tpm.todo_id \
                 LEFT JOIN projects p ON p.id = tpm.project_id WHERE t.id=?",
                &[json!(&id)],
            ).map_err(|e| e.to_string())?.unwrap_or(json!({}));
            return Ok(json!({ "success": true, "todo": todo, "repeat_advanced": true }));
        }
    }

    // Normal toggle
    database::run(
        "UPDATE todos SET done = CASE WHEN done=0 THEN 1 ELSE 0 END, updated_at=datetime('now', 'localtime') WHERE id=?",
        &[json!(&id)],
    ).map_err(|e| e.to_string())?;
    crate::services::knowledge_lifecycle::notify_changed("TODO", &id);
    // 标记完成时入队清理 source_data + 向量
    let todo = database::get(
        "SELECT t.*, tpm.project_id, p.name as project_name FROM todos t \
         LEFT JOIN todo_project_mapping tpm ON t.id = tpm.todo_id \
         LEFT JOIN projects p ON p.id = tpm.project_id WHERE t.id=?",
        &[json!(&id)],
    ).map_err(|e| e.to_string())?.unwrap_or(json!({}));
    Ok(json!({ "success": true, "todo": todo }))
}

#[tauri::command]
pub async fn todos_get(id: String) -> Result<Value, String> {
    let todo = database::get(
        "SELECT t.*, tpm.project_id, p.name as project_name FROM todos t \
         LEFT JOIN todo_project_mapping tpm ON t.id = tpm.todo_id \
         LEFT JOIN projects p ON p.id = tpm.project_id WHERE t.id=?",
        &[json!(&id)],
    )
    .map_err(|e| e.to_string())?
    .unwrap_or(json!({}));
    Ok(json!({ "success": true, "todo": todo }))
}

#[cfg(test)]
mod tests {
    use super::normalize_todo_priority;

    #[test]
    fn priority_variants_all_map_to_canonical_range() {
        for v in ["high", "HIGH", "High", "urgent", "P1", "p1", "高", "最高", "紧急"] {
            assert_eq!(normalize_todo_priority(v).as_deref(), Some("P1"), "{}", v);
        }
        for v in ["medium", "Medium", "normal", "P2", "p2", "中", "一般"] {
            assert_eq!(normalize_todo_priority(v).as_deref(), Some("P2"), "{}", v);
        }
        for v in ["low", "LOW", "P3", "p3", "低"] {
            assert_eq!(normalize_todo_priority(v).as_deref(), Some("P3"), "{}", v);
        }
    }

    #[test]
    fn priority_empty_is_none_and_unknown_preserved() {
        assert_eq!(normalize_todo_priority("   "), None);
        assert_eq!(normalize_todo_priority("").as_deref(), None);
        assert_eq!(normalize_todo_priority("自定义级别").as_deref(), Some("自定义级别"));
        assert_eq!(normalize_todo_priority(" P1 ").as_deref(), Some("P1"));
    }
}
