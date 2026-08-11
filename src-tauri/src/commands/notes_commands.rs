// Notes commands — CRUD + tag management + trash
use crate::database;
use serde_json::{json, Value};

const NOTE_META_COLS: &str = "n.note_id, n.title, substr(n.content,1,300) AS excerpt, n.is_daily, n.notebook_id, n.updated_at, n.created_at, n.deleted_at";

#[tauri::command]
pub async fn notes_list_all() -> Result<Value, String> {
    let notes = database::all(&format!(
        "SELECT {}, GROUP_CONCAT(npm.project_id) as project_ids FROM notes n LEFT JOIN note_project_mapping npm ON n.note_id=npm.note_id WHERE n.deleted_at IS NULL GROUP BY n.note_id ORDER BY n.updated_at DESC",
        NOTE_META_COLS
    ),&[]).map_err(|e|e.to_string())?;
    Ok(json!({"success":true,"notes":notes}))
}
#[tauri::command] pub async fn notes_list_daily() -> Result<Value, String> {
    let notes = database::all(&format!(
        "SELECT {} FROM notes n WHERE n.is_daily=1 AND n.deleted_at IS NULL ORDER BY n.updated_at DESC",
        NOTE_META_COLS
    ),&[]).map_err(|e|e.to_string())?;
    Ok(json!({"success":true,"notes":notes}))
}
#[tauri::command] pub async fn notes_list_by_project(project_id: String) -> Result<Value, String> {
    let notes = database::all(&format!(
        "SELECT {} FROM notes n INNER JOIN note_project_mapping npm ON n.note_id=npm.note_id WHERE npm.project_id=? AND n.deleted_at IS NULL ORDER BY n.updated_at DESC",
        NOTE_META_COLS
    ),&[json!(project_id)]).map_err(|e|e.to_string())?;
    Ok(json!({"success":true,"notes":notes}))
}
#[tauri::command] pub async fn notes_list_orphan() -> Result<Value, String> {
    let notes = database::all(&format!(
        "SELECT {} FROM notes n LEFT JOIN note_project_mapping npm ON n.note_id=npm.note_id WHERE npm.project_id IS NULL AND n.is_daily=0 AND n.deleted_at IS NULL ORDER BY n.updated_at DESC",
        NOTE_META_COLS
    ),&[]).map_err(|e|e.to_string())?;
    Ok(json!({"success":true,"notes":notes}))
}
#[tauri::command] pub async fn notes_list_trash() -> Result<Value, String> {
    let notes = database::all(&format!(
        "SELECT {} FROM notes n WHERE n.deleted_at IS NOT NULL ORDER BY n.deleted_at DESC",
        NOTE_META_COLS
    ),&[]).map_err(|e|e.to_string())?;
    Ok(json!({"success":true,"notes":notes}))
}
#[tauri::command] pub async fn notes_list_by_notebook(notebook_id: String) -> Result<Value, String> {
    let notes = database::all(&format!(
        "SELECT {} FROM notes n WHERE n.notebook_id=? AND n.is_daily=0 AND n.deleted_at IS NULL ORDER BY n.updated_at DESC",
        NOTE_META_COLS
    ),&[json!(notebook_id)]).map_err(|e|e.to_string())?;
    Ok(json!({"success":true,"notes":notes}))
}
#[tauri::command] pub async fn notes_get(note_id: String) -> Result<Value, String> {
    let note = database::get(
        "SELECT note_id, title, content, is_daily, notebook_id, updated_at, created_at FROM notes WHERE note_id=?",
        &[json!(note_id)],
    ).map_err(|e|e.to_string())?;
    Ok(json!({"success":true,"note":note}))
}
#[tauri::command] pub async fn notes_get_for_trash(note_id: String) -> Result<Value, String> {
    let note = database::get(
        "SELECT note_id, title, substr(content,1,300) AS excerpt, is_daily, deleted_at FROM notes WHERE note_id=? AND deleted_at IS NOT NULL",
        &[json!(note_id)],
    ).map_err(|e|e.to_string())?;
    Ok(json!({"success":true,"note":note}))
}
#[tauri::command]
pub async fn notes_create(data: Value) -> Result<Value, String> {
    let note_id = uuid::Uuid::new_v4().to_string();
    let title = data["title"].as_str().unwrap_or("");
    let content = data["content"].as_str().unwrap_or("");
    let is_daily = data["isDaily"].as_bool().or_else(||data["isDaily"].as_i64().map(|n|n!=0)).unwrap_or(false);
    let notebook_id = data["notebookId"].as_str().map(|s|json!(s));
    let ts = crate::time_utils::now_db();
    let project_ids: Vec<String> = data["projectIds"].as_array().map(|a|a.iter().filter_map(|v|v.as_str().map(String::from)).collect()).unwrap_or_default();
    database::with_db(|conn| {
        conn.execute("BEGIN",[])?;
        let r = conn.execute("INSERT INTO notes(note_id,title,content,is_daily,notebook_id,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7)", rusqlite::params![note_id,title,content,if is_daily{1i64}else{0i64},notebook_id.as_ref().map(|v|v.as_str().unwrap_or("")),ts,ts]);
        if let Err(e) = r { let _ = conn.execute("ROLLBACK",[]); return Err(e); }
        for pid in &project_ids { conn.execute("INSERT OR IGNORE INTO note_project_mapping(note_id,project_id) VALUES(?1,?2)", rusqlite::params![note_id,pid]).ok(); }
        conn.execute("COMMIT",[]).map(|_|())?;
        Ok(())
    }).map_err(|e|e.to_string())?;
    // business_relations::upsert must be called outside with_db to avoid mutex deadlock
    // 新建笔记：所有项目关联都是新增的，统一刷新项目活跃时间（touch 也必须在 with_db 外调用）
    for pid in &project_ids {
        crate::services::business_relations::upsert("NOTE", &note_id, "PROJECT", pid, "authored_by", "{}");
        crate::commands::project_commands::touch_project(pid);
    }
    crate::services::knowledge_lifecycle::notify_changed("NOTE", &note_id);
    crate::services::object_changed::object_changed(crate::services::object_changed::ObjectType::Note, &note_id, crate::services::object_changed::Action::Created);
    Ok(json!({"success":true,"noteId":note_id}))
}
#[tauri::command]
pub async fn notes_update(data: Value) -> Result<Value, String> {
    let note_id = data["noteId"].as_str().ok_or("missing noteId")?;
    let ts = crate::time_utils::now_db();
    let mut fields: Vec<String> = Vec::new();
    let mut values: Vec<Value> = Vec::new();
    if let Some(v) = data["title"].as_str() { fields.push("title=?".into()); values.push(json!(v)); }
    if let Some(v) = data["content"].as_str() { fields.push("content=?".into()); values.push(json!(v)); }
    if let Some(v) = data["isDaily"].as_bool().or_else(||data["isDaily"].as_i64().map(|n|n!=0)) { fields.push("is_daily=?".into()); values.push(json!(if v{1}else{0})); }
    if let Some(v) = data["notebookId"].as_str() { fields.push("notebook_id=?".into()); values.push(json!(v)); }
    if !fields.is_empty() { fields.push("updated_at=?".into()); values.push(json!(ts)); values.push(json!(note_id)); let sql = format!("UPDATE notes SET {} WHERE note_id=?", fields.join(",")); database::run(&sql, &values).map_err(|e|e.to_string())?; }
    if let Some(pids) = data["projectIds"].as_array() {
        // 增量比对：笔记自动保存会反复走"先删后插"，只为"新出现的项目关联"刷新活跃时间
        let old_pids: std::collections::HashSet<String> = database::all(
            "SELECT project_id FROM note_project_mapping WHERE note_id=?", &[json!(note_id)]
        ).unwrap_or_default().iter().filter_map(|r| r["project_id"].as_str().map(String::from)).collect();
        database::run("DELETE FROM note_project_mapping WHERE note_id=?",&[json!(note_id)]).ok();
        crate::services::business_relations::delete_by_source("NOTE", &note_id);
        for p in pids {
            if let Some(pid) = p.as_str() {
                database::run("INSERT OR IGNORE INTO note_project_mapping(note_id,project_id) VALUES(?,?)",&[json!(note_id),p.clone()]).ok();
                crate::services::business_relations::upsert("NOTE", &note_id, "PROJECT", pid, "authored_by", "{}");
                if !old_pids.contains(pid) {
                    crate::commands::project_commands::touch_project(pid);
                }
            }
        }
    }
    database::save().map_err(|e|e.to_string())?;
    crate::services::knowledge_lifecycle::notify_changed("NOTE", &note_id);
    crate::services::object_changed::object_changed(crate::services::object_changed::ObjectType::Note, &note_id, crate::services::object_changed::Action::Updated);
    Ok(json!({"success":true}))
}
#[tauri::command]
pub async fn notes_delete(note_id: String) -> Result<Value, String> {
    database::run("UPDATE notes SET deleted_at=datetime('now', 'localtime'), notebook_id=NULL WHERE note_id=?",&[json!(&note_id)]).map_err(|e|e.to_string())?;
    database::run("DELETE FROM note_project_mapping WHERE note_id=?",&[json!(&note_id)]).ok();
    // 软删除即彻底清理：business_relations + 知识管线（恢复只入笔记本，不需要这些关联）
    crate::services::business_relations::delete_by_source("NOTE", &note_id);
    crate::services::knowledge_lifecycle::notify_deleted("NOTE", &note_id);
    crate::services::object_changed::object_changed(crate::services::object_changed::ObjectType::Note, &note_id, crate::services::object_changed::Action::Deleted);
    database::save().map_err(|e|e.to_string())?;
    Ok(json!({"success":true}))
}
#[tauri::command] pub async fn notes_restore(note_id: String, notebook_id: String) -> Result<Value, String> {
    let actual_nb = if let Some(proj_id) = notebook_id.strip_prefix("project:") {
        database::run("INSERT OR IGNORE INTO note_project_mapping(note_id,project_id) VALUES(?,?)",&[json!(&note_id),json!(proj_id)]).ok();
        None::<String>
    } else { Some(notebook_id) };
    database::run("UPDATE notes SET deleted_at=NULL, notebook_id=?, updated_at=datetime('now', 'localtime') WHERE note_id=?",&[json!(actual_nb),json!(&note_id)]).map_err(|e|e.to_string())?;
    // 恢复笔记时通知知识库重新物化（软删除时未清理知识管线，恢复后需更新内容）
    crate::services::knowledge_lifecycle::notify_changed("NOTE", &note_id);
    database::save().map_err(|e|e.to_string())?;
    crate::services::object_changed::object_changed(crate::services::object_changed::ObjectType::Note, &note_id, crate::services::object_changed::Action::Updated);
    Ok(json!({"success":true}))
}
#[tauri::command] pub async fn notes_list_archive() -> Result<Value, String> {
    let notes = database::all(&format!(
        "SELECT {} FROM notes n LEFT JOIN note_project_mapping npm ON n.note_id=npm.note_id WHERE npm.project_id IS NULL AND n.is_daily=0 AND n.notebook_id IS NULL AND n.deleted_at IS NULL ORDER BY n.updated_at DESC",
        NOTE_META_COLS
    ),&[]).map_err(|e|e.to_string())?;
    Ok(json!({"success":true,"notes":notes}))
}
#[tauri::command] pub async fn notes_permanent_delete(note_id: String) -> Result<Value, String> {
    crate::services::knowledge_lifecycle::notify_deleted("NOTE", &note_id);
    database::run("DELETE FROM note_project_mapping WHERE note_id=?",&[json!(&note_id)]).ok();
    database::run("DELETE FROM file_relations WHERE entity_type='note' AND entity_id=?",&[json!(&note_id)]).ok();
    crate::services::business_relations::delete_by_source("NOTE", &note_id);
    // 清理以该笔记为目标的边（真实附件的 FILE→NOTE has_file 边，由 add_relation 写入）
    database::run("DELETE FROM business_relations WHERE target_type='NOTE' AND target_id=?", &[json!(&note_id)]).ok();
    database::run("DELETE FROM notes WHERE note_id=?",&[json!(&note_id)]).map_err(|e|e.to_string())?;
    crate::services::object_changed::object_changed(crate::services::object_changed::ObjectType::Note, &note_id, crate::services::object_changed::Action::Deleted);
    database::save().map_err(|e|e.to_string())?;
    Ok(json!({"success":true}))
}
#[tauri::command] pub async fn notes_count_by_notebook(notebook_id: String) -> Result<Value, String> { let count: i64 = database::with_db(|c| c.query_row("SELECT COUNT(*) FROM notes WHERE notebook_id=? AND deleted_at IS NULL", rusqlite::params![&notebook_id],|r|r.get(0))).unwrap_or(0); Ok(json!({"success":true,"count":count})) }
// Notebooks
#[tauri::command] pub async fn notebook_list() -> Result<Value, String> { let notebooks = database::all("SELECT * FROM notebooks ORDER BY sort_order ASC,name ASC",&[]).map_err(|e|e.to_string())?; Ok(json!({"success":true,"notebooks":notebooks})) }
#[tauri::command] pub async fn notebook_create(data: Value) -> Result<Value, String> { let id = data["id"].as_str().map(|s|s.to_string()).unwrap_or_else(|| uuid::Uuid::new_v4().to_string()); let name = data["name"].as_str().unwrap_or("新笔记本"); let color = data["color"].as_str().unwrap_or("#64748b"); let parent_id = data["parentId"].as_str().map(|s|json!(s)); database::run("INSERT INTO notebooks(id,name,color,parent_id) VALUES(?,?,?,?)",&[json!(id),json!(name),json!(color),parent_id.clone().unwrap_or(json!(serde_json::Value::Null))]).map_err(|e|e.to_string())?; database::save().map_err(|e|e.to_string())?; Ok(json!({"success":true,"notebook":{"id":id,"name":name,"color":color,"parentId":parent_id}})) }
#[tauri::command] pub async fn notebook_update(data: Value) -> Result<Value, String> { let id = data["id"].as_str().ok_or("missing id")?; if let Some(n) = data["name"].as_str() { database::run("UPDATE notebooks SET name=? WHERE id=?",&[json!(n),json!(id)]).ok(); } if let Some(c) = data["color"].as_str() { database::run("UPDATE notebooks SET color=? WHERE id=?",&[json!(c),json!(id)]).ok(); } if let Some(p) = data["parentId"].as_str() { database::run("UPDATE notebooks SET parent_id=? WHERE id=?",&[json!(p),json!(id)]).ok(); } else if data.get("parentId").map(|v|v.is_null()).unwrap_or(false) { database::run("UPDATE notebooks SET parent_id=NULL WHERE id=?",&[json!(id)]).ok(); } if let Some(s) = data["sortOrder"].as_i64() { database::run("UPDATE notebooks SET sort_order=? WHERE id=?",&[json!(s),json!(id)]).ok(); } database::save().map_err(|e|e.to_string())?; Ok(json!({"success":true})) }
#[tauri::command] pub async fn notebook_delete(notebook_id: String) -> Result<Value, String> { database::run("UPDATE notes SET deleted_at=datetime('now', 'localtime'), notebook_id=NULL WHERE notebook_id=?",&[json!(&notebook_id)]).map_err(|e|e.to_string())?; database::run("DELETE FROM notebooks WHERE id=?",&[json!(&notebook_id)]).map_err(|e|e.to_string())?; database::save().map_err(|e|e.to_string())?; Ok(json!({"success":true})) }
