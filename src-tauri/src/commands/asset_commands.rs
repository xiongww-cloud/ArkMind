use serde_json::{json, Value}; use crate::database;
#[tauri::command] pub async fn assets_search(data: Value) -> Result<Value, String> {
    let query = data["query"].as_str().unwrap_or("");
    let limit = data["limit"].as_i64().unwrap_or(10) as usize;
    let results = database::fts5_search_chunks_slim(query, limit).unwrap_or_default();
    Ok(json!({"success":true,"results":results}))
}
#[tauri::command] pub async fn assets_search_fts5(data: Value) -> Result<Value, String> {
    let query = data["query"].as_str().unwrap_or("");
    let limit = data["limit"].as_i64().unwrap_or(10);
    let results = database::fts5_search_chunks_slim(query, limit as usize).unwrap_or_default();
    Ok(json!({"success":true,"results":results}))
}
#[tauri::command] pub async fn assets_list() -> Result<Value, String> { let assets = database::all("SELECT fa.id, fa.file_ext, fa.file_size, fa.content_hash, sd.content as parsed_text, fa.created_at, fa.updated_at, fr.custom_name as lib_gn, fr.custom_name as lib_cn, fa.id as latest_version_id, fa.file_size as latest_version_size FROM file_assets fa LEFT JOIN source_data sd ON sd.source_type='FILE' AND sd.source_id=fa.id LEFT JOIN file_relations fr ON fr.file_id = fa.id AND fr.entity_type='library' ORDER BY fa.updated_at DESC",&[]).map_err(|e| e.to_string())?; Ok(json!({"success":true,"assets":assets})) }
#[tauri::command] pub async fn asset_sync_status() -> Result<Value, String> {
    let running = crate::services::knowledge_pipeline::is_running();
    let pending: i64 = database::with_db(|c| c.query_row("SELECT COUNT(*) FROM knowledge_push_queue WHERE status='pending'",[],|r|r.get(0))).unwrap_or(0);
    let done: i64 = database::with_db(|c| c.query_row("SELECT COUNT(*) FROM knowledge_push_queue WHERE status='done'",[],|r|r.get(0))).unwrap_or(0);
    Ok(json!({"running":running,"pending":pending,"done":done}))
}
#[tauri::command] pub async fn asset_sync_trigger(file_id: String) -> Result<Value, String> {
    crate::services::knowledge_lifecycle::notify_changed("FILE", &file_id);
    Ok(json!({"success":true}))
}
#[tauri::command] pub async fn asset_diagnose() -> Result<Value, String> {
    let fa: i64 = database::with_db(|c| c.query_row("SELECT COUNT(*) FROM file_assets",[],|r|r.get(0))).unwrap_or(0);
    let fts: i64 = database::with_db(|c| c.query_row("SELECT COUNT(*) FROM global_search_fts",[],|r|r.get(0))).unwrap_or(0);
    let vch: i64 = database::with_db(|c| c.query_row("SELECT COUNT(*) FROM vector_cache",[],|r|r.get(0))).unwrap_or(0);
    Ok(json!({"fileAssets":fa,"ftsIndexed":fts,"vectorCached":vch}))
}
#[tauri::command] pub async fn asset_get_latest_version(file_id: String) -> Result<Value, String> {
    let ver = database::get("SELECT id FROM file_assets WHERE id=?",&[json!(file_id)]).map_err(|e|e.to_string())?;
    match ver {
        Some(v) => {
            let fp = crate::commands::file_commands::get_physical_path(&file_id);
            Ok(json!({"success":true,"versionId":v["id"],"filePath":fp.unwrap_or_default()}))
        }
        None => Ok(json!({"success":false,"error":"无版本记录"}))
    }
}
#[tauri::command] pub async fn asset_get_parsed_text(file_id: String) -> Result<Value, String> {
    let row = database::get("SELECT sd.content,fa.file_ext FROM source_data sd JOIN file_assets fa ON fa.id=sd.source_id WHERE sd.source_type='FILE' AND sd.source_id=?",&[json!(&file_id)]).map_err(|e|e.to_string())?;
    match row {
        Some(r) => Ok(json!({"success":true,"text":r["content"].as_str().unwrap_or(""),"file_ext":r["file_ext"].as_str().unwrap_or("")})),
        None => Ok(json!({"success":false,"error":"文件不存在"}))
    }
}
#[tauri::command] pub async fn asset_rerun_migration() -> Result<Value, String> {
    let files = database::all("SELECT fa.id,fa.file_ext,fa.content_hash FROM file_assets fa LEFT JOIN source_data sd ON sd.source_type='FILE' AND sd.source_id=fa.id WHERE sd.source_id IS NULL LIMIT 200",&[]).map_err(|e|e.to_string())?;
    let mut migrated = 0usize;
    for f in &files {
        let fid = f["id"].as_str().unwrap_or("");
        let hash = f["content_hash"].as_str().unwrap_or("");
        if fid.is_empty() || hash.is_empty() { continue; }
        let path = crate::commands::file_commands::flat_disk_path(hash);
        if !std::path::Path::new(&path).exists() { continue; }
        let ext = f["file_ext"].as_str().unwrap_or("");
        let result = crate::services::file_parser::parse_file_with_hint(&path, ext);
        if result.success && !result.text.is_empty() {
            // 写入 source_data
            crate::database::file_sync_to_source_data(fid, &result.text);
            migrated += 1;
        }
    }
    Ok(json!({"success":true,"sourceCount":files.len(),"migrated":migrated}))
}
