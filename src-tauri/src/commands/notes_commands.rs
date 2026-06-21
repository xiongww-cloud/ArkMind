// Notes commands — CRUD + tag management + trash
use crate::database;
use serde_json::{json, Value};

#[tauri::command]
pub async fn notes_list_all() -> Result<Value, String> {
    let notes = database::all("SELECT n.*, GROUP_CONCAT(npm.project_id) as project_ids FROM notes n LEFT JOIN note_project_mapping npm ON n.note_id=npm.note_id WHERE n.deleted_at IS NULL GROUP BY n.note_id ORDER BY n.updated_at DESC",&[]).map_err(|e|e.to_string())?;
    Ok(json!({"success":true,"notes":notes}))
}
#[tauri::command] pub async fn notes_list_daily() -> Result<Value, String> {
    let notes = database::all("SELECT * FROM notes WHERE is_daily=1 AND deleted_at IS NULL ORDER BY updated_at DESC",&[]).map_err(|e|e.to_string())?;
    Ok(json!({"success":true,"notes":notes}))
}
#[tauri::command] pub async fn notes_list_by_project(project_id: String) -> Result<Value, String> {
    let notes = database::all("SELECT n.* FROM notes n INNER JOIN note_project_mapping npm ON n.note_id=npm.note_id WHERE npm.project_id=? AND n.deleted_at IS NULL ORDER BY n.updated_at DESC",&[json!(project_id)]).map_err(|e|e.to_string())?;
    Ok(json!({"success":true,"notes":notes}))
}
#[tauri::command] pub async fn notes_list_orphan() -> Result<Value, String> {
    let notes = database::all("SELECT n.* FROM notes n LEFT JOIN note_project_mapping npm ON n.note_id=npm.note_id WHERE npm.project_id IS NULL AND n.is_daily=0 AND n.deleted_at IS NULL ORDER BY n.updated_at DESC",&[]).map_err(|e|e.to_string())?;
    Ok(json!({"success":true,"notes":notes}))
}
#[tauri::command] pub async fn notes_list_trash() -> Result<Value, String> {
    let notes = database::all("SELECT n.* FROM notes n WHERE n.deleted_at IS NOT NULL ORDER BY n.deleted_at DESC",&[]).map_err(|e|e.to_string())?;
    Ok(json!({"success":true,"notes":notes}))
}
#[tauri::command]
pub async fn notes_create(data: Value) -> Result<Value, String> {
    let note_id = uuid::Uuid::new_v4().to_string();
    let title = data["title"].as_str().unwrap_or("");
    let content = data["content"].as_str().unwrap_or("");
    let is_daily = data["isDaily"].as_bool().or_else(||data["isDaily"].as_i64().map(|n|n!=0)).unwrap_or(false);
    let notebook_id = data["notebookId"].as_str().map(|s|json!(s));
    let ts = crate::time_utils::now_utc_rfc3339();
    let project_ids: Vec<String> = data["projectIds"].as_array().map(|a|a.iter().filter_map(|v|v.as_str().map(String::from)).collect()).unwrap_or_default();
    let tag_ids: Vec<String> = data["tagIds"].as_array().map(|a|a.iter().filter_map(|v|v.as_str().map(String::from)).collect()).unwrap_or_default();
    database::with_db(|conn| {
        conn.execute("BEGIN",[])?;
        let r = conn.execute("INSERT INTO notes(note_id,title,content,is_daily,notebook_id,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7)", rusqlite::params![note_id,title,content,if is_daily{1i64}else{0i64},notebook_id.as_ref().map(|v|v.as_str().unwrap_or("")),ts,ts]);
        if let Err(e) = r { let _ = conn.execute("ROLLBACK",[]); return Err(e); }
        for pid in &project_ids { conn.execute("INSERT OR IGNORE INTO note_project_mapping(note_id,project_id) VALUES(?1,?2)", rusqlite::params![note_id,pid]).ok(); }
        for tid in &tag_ids { conn.execute("INSERT OR IGNORE INTO tag_mappings(entity_type,entity_id,tag_id) VALUES('notes',?1,?2)", rusqlite::params![note_id,tid]).ok(); }
        conn.execute("COMMIT",[]).map(|_|())?;
        Ok(())
    }).map_err(|e|e.to_string())?;
    crate::services::source_doc_sync::sync_entity_to_source_docs("NOTE", &note_id, title, content);
    Ok(json!({"success":true,"noteId":note_id}))
}
#[tauri::command]
pub async fn notes_update(data: Value) -> Result<Value, String> {
    let note_id = data["noteId"].as_str().ok_or("missing noteId")?;
    let ts = crate::time_utils::now_utc_rfc3339();
    let mut fields: Vec<String> = Vec::new();
    let mut values: Vec<Value> = Vec::new();
    if let Some(v) = data["title"].as_str() { fields.push("title=?".into()); values.push(json!(v)); }
    if let Some(v) = data["content"].as_str() { fields.push("content=?".into()); values.push(json!(v)); }
    if let Some(v) = data["isDaily"].as_bool().or_else(||data["isDaily"].as_i64().map(|n|n!=0)) { fields.push("is_daily=?".into()); values.push(json!(if v{1}else{0})); }
    if let Some(v) = data["notebookId"].as_str() { fields.push("notebook_id=?".into()); values.push(json!(v)); }
    if !fields.is_empty() { fields.push("updated_at=?".into()); values.push(json!(ts)); values.push(json!(note_id)); let sql = format!("UPDATE notes SET {} WHERE note_id=?", fields.join(",")); database::run(&sql, &values).map_err(|e|e.to_string())?; }
    if let Some(pids) = data["projectIds"].as_array() { database::run("DELETE FROM note_project_mapping WHERE note_id=?",&[json!(note_id)]).ok(); for p in pids { database::run("INSERT OR IGNORE INTO note_project_mapping(note_id,project_id) VALUES(?,?)",&[json!(note_id),p.clone()]).ok(); } }
    if let Some(tids) = data["tagIds"].as_array() { database::run("DELETE FROM tag_mappings WHERE entity_type='notes' AND entity_id=?",&[json!(note_id)]).ok(); for t in tids { database::run("INSERT OR IGNORE INTO tag_mappings(entity_type,entity_id,tag_id) VALUES('notes',?,?)",&[json!(note_id),t.clone()]).ok(); } }
    database::save().map_err(|e|e.to_string())?;
    // 同步到 source_data（通过清洗管道）
    if let Ok(Some(row)) = database::get("SELECT title,content FROM notes WHERE note_id=? AND deleted_at IS NULL",&[json!(&note_id)]) {
        let title = row["title"].as_str().unwrap_or("");
        let content = row["content"].as_str().unwrap_or("");
        crate::services::source_doc_sync::sync_entity_to_source_docs("NOTE", &note_id, title, content);
    }
    Ok(json!({"success":true}))
}
#[tauri::command]
pub async fn notes_delete(note_id: String) -> Result<Value, String> {
    database::run("UPDATE notes SET deleted_at=datetime('now', 'localtime'), notebook_id=NULL WHERE note_id=?",&[json!(&note_id)]).map_err(|e|e.to_string())?;
    database::run("DELETE FROM note_project_mapping WHERE note_id=?",&[json!(&note_id)]).ok();
    database::run("DELETE FROM tag_mappings WHERE entity_type='notes' AND entity_id=?",&[json!(&note_id)]).ok();
    database::save().map_err(|e|e.to_string())?;
    Ok(json!({"success":true}))
}
#[tauri::command] pub async fn notes_restore(note_id: String, notebook_id: String) -> Result<Value, String> {
    let actual_nb = if let Some(proj_id) = notebook_id.strip_prefix("project:") {
        database::run("INSERT OR IGNORE INTO note_project_mapping(note_id,project_id) VALUES(?,?)",&[json!(&note_id),json!(proj_id)]).ok();
        None::<String>
    } else { Some(notebook_id) };
    database::run("UPDATE notes SET deleted_at=NULL, notebook_id=?, updated_at=datetime('now', 'localtime') WHERE note_id=?",&[json!(actual_nb),json!(&note_id)]).map_err(|e|e.to_string())?;
    database::save().map_err(|e|e.to_string())?;
    Ok(json!({"success":true}))
}
#[tauri::command] pub async fn notes_list_archive() -> Result<Value, String> {
    let notes = database::all("SELECT n.* FROM notes n LEFT JOIN note_project_mapping npm ON n.note_id=npm.note_id WHERE npm.project_id IS NULL AND n.is_daily=0 AND n.notebook_id IS NULL AND n.deleted_at IS NULL ORDER BY n.updated_at DESC",&[]).map_err(|e|e.to_string())?;
    Ok(json!({"success":true,"notes":notes}))
}
#[tauri::command] pub async fn notes_permanent_delete(note_id: String) -> Result<Value, String> {
    // ★ 清理笔记的知识库（source_data + knowledge_chunks + vector_cache + LanceDB）
    crate::services::cleanup_pipeline::enqueue_cleanup("NOTE", &note_id);

    database::run("DELETE FROM note_project_mapping WHERE note_id=?",&[json!(&note_id)]).ok();
    database::run("DELETE FROM tag_mappings WHERE entity_type='notes' AND entity_id=?",&[json!(&note_id)]).ok();
    database::run("DELETE FROM file_relations WHERE entity_type='note' AND entity_id=?",&[json!(&note_id)]).ok();
    database::run("DELETE FROM notes WHERE note_id=?",&[json!(&note_id)]).map_err(|e|e.to_string())?;

    database::save().map_err(|e|e.to_string())?;
    Ok(json!({"success":true}))
}
// Tag management
#[tauri::command] pub async fn notes_list_tags() -> Result<Value, String> { let tags = database::all("SELECT * FROM tags ORDER BY name ASC",&[]).map_err(|e|e.to_string())?; Ok(json!({"success":true,"tags":tags})) }
#[tauri::command] pub async fn notes_create_tag(data: Value) -> Result<Value, String> { let name = data["name"].as_str().ok_or("missing name")?; let color = data["color"].as_str().unwrap_or("#6366f1"); let tag_type = data["type"].as_str().unwrap_or("general"); let id = uuid::Uuid::new_v4().to_string(); database::run("INSERT INTO tags(tag_id,name,color,type) VALUES(?,?,?,?)",&[json!(id),json!(name),json!(color),json!(tag_type)]).map_err(|e|e.to_string())?; Ok(json!({"success":true,"tag":{"tag_id":id,"name":name,"color":color,"type":tag_type}})) }
#[tauri::command] pub async fn notes_search_tags(query: String) -> Result<Value, String> { let tags = database::all("SELECT * FROM tags WHERE name LIKE ? ORDER BY name ASC",&[json!(format!("%{}%",query))]).map_err(|e|e.to_string())?; Ok(json!({"success":true,"tags":tags})) }
#[tauri::command] pub async fn notes_list_tags_by_type(tag_type: String) -> Result<Value, String> { let tags = database::all("SELECT * FROM tags WHERE type=? ORDER BY name ASC",&[json!(tag_type)]).map_err(|e|e.to_string())?; Ok(json!({"success":true,"tags":tags})) }
#[tauri::command] pub async fn notes_generate_ai_tags(note_id: String) -> Result<Value, String> { crate::services::ai_tag::trigger_tagging("notes",&note_id); Ok(json!({"success":true})) }
#[tauri::command] pub async fn notes_rebuild_fts() -> Result<Value, String> { Ok(json!({"success":true})) }
#[tauri::command] pub async fn notes_upgrade_fts_triggers() -> Result<Value, String> { Ok(json!({"success":true})) }
#[tauri::command] pub async fn notes_get_inherited_tags(note_id: String) -> Result<Value, String> { let tags = database::all("SELECT DISTINCT t.* FROM tags t INNER JOIN tag_mappings tm ON tm.tag_id=t.tag_id INNER JOIN note_project_mapping npm ON npm.note_id=tm.entity_id AND tm.entity_type='notes' WHERE npm.project_id IN (SELECT project_id FROM note_project_mapping WHERE note_id=?) AND npm.note_id!=?", &[json!(&note_id),json!(&note_id)]).unwrap_or_default(); Ok(json!({"success":true,"tags":tags})) }
#[tauri::command] pub async fn notes_get_project_knowledge_graph(_project_id: String) -> Result<Value, String> { Ok(json!({"success":true,"nodes":[],"edges":[]})) }
#[tauri::command] pub async fn notes_semantic_recall(_data: Value) -> Result<Value, String> { Ok(json!({"success":true,"results":[]})) }
#[tauri::command] pub async fn notes_list_tag_mappings() -> Result<Value, String> { let mappings = database::all("SELECT tm.entity_id AS note_id, tm.tag_id, t.name, t.color, t.type FROM tag_mappings tm JOIN tags t ON t.tag_id=tm.tag_id WHERE tm.entity_type='notes'",&[]).map_err(|e|e.to_string())?; Ok(json!({"success":true,"mappings":mappings})) }
#[tauri::command] pub async fn notes_add_tag_mapping(note_id: String, tag_id: String) -> Result<Value, String> { database::run("INSERT OR IGNORE INTO tag_mappings(entity_type,entity_id,tag_id) VALUES('notes',?1,?2)",&[json!(&note_id),json!(&tag_id)]).map_err(|e|e.to_string())?; Ok(json!({"success":true})) }
#[tauri::command] pub async fn notes_remove_tag_mapping(note_id: String, tag_id: String) -> Result<(), String> { database::run("DELETE FROM tag_mappings WHERE entity_type='notes' AND entity_id=? AND tag_id=?",&[json!(note_id),json!(tag_id)]).map_err(|e|e.to_string())?; database::save().map_err(|e|e.to_string()) }
#[tauri::command] pub async fn notes_count_by_notebook(notebook_id: String) -> Result<Value, String> { let count: i64 = database::with_db(|c| c.query_row("SELECT COUNT(*) FROM notes WHERE notebook_id=? AND deleted_at IS NULL", rusqlite::params![&notebook_id],|r|r.get(0))).unwrap_or(0); Ok(json!({"success":true,"count":count})) }
// Notebooks
#[tauri::command] pub async fn notebook_list() -> Result<Value, String> { let notebooks = database::all("SELECT * FROM notebooks ORDER BY sort_order ASC,name ASC",&[]).map_err(|e|e.to_string())?; Ok(json!({"success":true,"notebooks":notebooks})) }
#[tauri::command] pub async fn notebook_create(data: Value) -> Result<Value, String> { let id = data["id"].as_str().map(|s|s.to_string()).unwrap_or_else(|| uuid::Uuid::new_v4().to_string()); let name = data["name"].as_str().unwrap_or("新笔记本"); let color = data["color"].as_str().unwrap_or("#64748b"); let parent_id = data["parentId"].as_str().map(|s|json!(s)); database::run("INSERT INTO notebooks(id,name,color,parent_id) VALUES(?,?,?,?)",&[json!(id),json!(name),json!(color),parent_id.clone().unwrap_or(json!(serde_json::Value::Null))]).map_err(|e|e.to_string())?; database::save().map_err(|e|e.to_string())?; Ok(json!({"success":true,"notebook":{"id":id,"name":name,"color":color,"parentId":parent_id}})) }
#[tauri::command] pub async fn notebook_update(data: Value) -> Result<Value, String> { let id = data["id"].as_str().ok_or("missing id")?; if let Some(n) = data["name"].as_str() { database::run("UPDATE notebooks SET name=? WHERE id=?",&[json!(n),json!(id)]).ok(); } if let Some(c) = data["color"].as_str() { database::run("UPDATE notebooks SET color=? WHERE id=?",&[json!(c),json!(id)]).ok(); } if let Some(p) = data["parentId"].as_str() { database::run("UPDATE notebooks SET parent_id=? WHERE id=?",&[json!(p),json!(id)]).ok(); } else if data.get("parentId").map(|v|v.is_null()).unwrap_or(false) { database::run("UPDATE notebooks SET parent_id=NULL WHERE id=?",&[json!(id)]).ok(); } if let Some(s) = data["sortOrder"].as_i64() { database::run("UPDATE notebooks SET sort_order=? WHERE id=?",&[json!(s),json!(id)]).ok(); } database::save().map_err(|e|e.to_string())?; Ok(json!({"success":true})) }
#[tauri::command] pub async fn notebook_delete(notebook_id: String) -> Result<Value, String> {
    let count: i64 = database::with_db(|c| c.query_row("SELECT COUNT(*) FROM notes WHERE notebook_id=? AND deleted_at IS NULL", rusqlite::params![&notebook_id], |r| r.get(0))).unwrap_or(0);
    if count > 0 { return Ok(json!({"success":false,"error":"笔记本不为空，请先移动所有笔记"})); }
    database::run("DELETE FROM notebooks WHERE id=?",&[json!(&notebook_id)]).map_err(|e|e.to_string())?;
    database::save().map_err(|e|e.to_string())?;
    Ok(json!({"success":true}))
}
// Tag management (global)
#[tauri::command] pub async fn tag_force_resync() -> Result<Value, String> {
    database::run("DELETE FROM ai_tag_queue",&[]).ok();
    database::run("INSERT OR IGNORE INTO ai_tag_queue(source_type,source_id) SELECT 'notes',note_id FROM notes",&[]).ok();
    database::run("INSERT OR IGNORE INTO ai_tag_queue(source_type,source_id) SELECT 'files',id FROM file_assets",&[]).ok();
    database::run("INSERT OR IGNORE INTO ai_tag_queue(source_type,source_id) SELECT 'products',id FROM products",&[]).ok();
    database::run("INSERT OR IGNORE INTO ai_tag_queue(source_type,source_id) SELECT 'projects',id FROM projects",&[]).ok();
    let count: i64 = database::with_db(|c| c.query_row("SELECT COUNT(*) FROM ai_tag_queue",[],|r|r.get(0))).unwrap_or(0);
    Ok(json!({"success":true,"queueCount":count}))
}
#[tauri::command] pub async fn tag_tag_single(data: Value) -> Result<Value, String> {
    let source_table = data["sourceTable"].as_str().unwrap_or("notes");
    let source_id = data["sourceId"].as_str().ok_or("missing sourceId")?;
    crate::services::ai_tag::force_retag(source_table, source_id);
    Ok(json!({"success":true}))
}
