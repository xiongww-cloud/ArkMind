use crate::database;
use serde_json::{json, Value};
use sha2::{Sha256, Digest};
use std::io::{Read, BufReader};
use std::fs::File;

/// Unified knowledge base cleanup for any source type.
/// Flow: collect chunk_ids → delete source_data (triggers knowledge_chunks + FTS5) → clean vector_cache + LanceDB
pub fn cleanup_kb(source_type: &str, source_id: &str) {
    let sd_row = database::get(
        "SELECT id FROM source_data WHERE source_type=?1 AND source_id=?2",
        &[json!(source_type), json!(source_id)],
    );
    let sd_id = match sd_row {
        Ok(Some(ref r)) => r["id"].as_i64(),
        Err(e) => { eprintln!("[KB清理] source_data 查询失败 {}/{}: {}", source_type, source_id, e); return; }
        _ => { eprintln!("[KB清理] 未找到 source_data: {}/{}", source_type, source_id); return; }
    };
    let sd_id = match sd_id {
        Some(id) => id,
        None => { eprintln!("[KB清理] source_data id 为 NULL: {}/{}", source_type, source_id); return; }
    };
    let chunk_ids: Vec<i64> = database::all(
        "SELECT id FROM knowledge_chunks WHERE source_data_id=?", &[json!(sd_id)]
    ).unwrap_or_default().iter().filter_map(|r| r["id"].as_i64()).collect();

    for cid in &chunk_ids {
        let _ = database::run("INSERT INTO kk_chunks_fts(kk_chunks_fts, rowid) VALUES('delete', ?)", &[json!(cid)]);
    }
    // ★ 先禁用触发器，避免 DELETE knowledge_chunks 时 kk_chunks_fts_delete 重复删 FTS5
    database::run("DROP TRIGGER IF EXISTS sd_chunks_delete", &[]).ok();
    database::run("DROP TRIGGER IF EXISTS sd_fts_delete", &[]).ok();
    database::run("DROP TRIGGER IF EXISTS sd_fts_update", &[]).ok();
    database::run("DROP TRIGGER IF EXISTS kk_chunks_fts_delete", &[]).ok();
    database::run("DROP TRIGGER IF EXISTS kk_chunks_fts_insert", &[]).ok();
    database::run("DROP TRIGGER IF EXISTS kk_chunks_fts_update", &[]).ok();

    if !chunk_ids.is_empty() {
        let ph2: Vec<String> = chunk_ids.iter().map(|_| "?".to_string()).collect();
        let params2: Vec<serde_json::Value> = chunk_ids.iter().map(|id| json!(id)).collect();
        match database::run(&format!("DELETE FROM knowledge_chunks WHERE id IN ({})", ph2.join(",")), &params2) {
            Ok(r) => eprintln!("[KB清理] 已删除 knowledge_chunks: {} 行", r.changes),
            Err(e) => eprintln!("[KB清理] knowledge_chunks 删除失败: {:#?}", e),
        }
    }
    match database::run("DELETE FROM source_data WHERE id=?", &[json!(sd_id)]) {
        Ok(r) => eprintln!("[KB清理] 已删除 source_data id={} ({}行)", sd_id, r.changes),
        Err(e) => eprintln!("[KB清理] 删除 source_data 失败 id={}: {:#?}", sd_id, e),
    }
    database::run("CREATE TRIGGER IF NOT EXISTS sd_chunks_delete AFTER DELETE ON source_data BEGIN DELETE FROM knowledge_chunks WHERE source_data_id = old.id; END", &[]).ok();
    database::run("CREATE TRIGGER IF NOT EXISTS sd_fts_delete AFTER DELETE ON source_data BEGIN INSERT INTO global_search_fts(global_search_fts, rowid, source_type, source_id, title, content) VALUES ('delete', old.rowid, old.source_type, old.source_id, old.title, old.content); END", &[]).ok();
    database::run("CREATE TRIGGER IF NOT EXISTS sd_fts_update AFTER UPDATE ON source_data BEGIN INSERT INTO global_search_fts(global_search_fts, rowid, source_type, source_id, title, content) VALUES ('delete', old.rowid, old.source_type, old.source_id, old.title, old.content); INSERT INTO global_search_fts(rowid, source_type, source_id, title, content) VALUES (new.rowid, new.source_type, new.source_id, new.title, new.content); END", &[]).ok();
    database::run("CREATE TRIGGER IF NOT EXISTS kk_chunks_fts_delete AFTER DELETE ON knowledge_chunks WHEN old.chunk_type='child' BEGIN INSERT INTO kk_chunks_fts(kk_chunks_fts, rowid) VALUES ('delete', old.rowid); END", &[]).ok();
    database::run("CREATE TRIGGER IF NOT EXISTS kk_chunks_fts_insert AFTER INSERT ON knowledge_chunks WHEN new.chunk_type='child' BEGIN INSERT INTO kk_chunks_fts(rowid, doc_title, content) VALUES (new.rowid, new.doc_title, new.content); END", &[]).ok();
    database::run("CREATE TRIGGER IF NOT EXISTS kk_chunks_fts_update AFTER UPDATE ON knowledge_chunks WHEN old.chunk_type='child' OR new.chunk_type='child' BEGIN DELETE FROM kk_chunks_fts WHERE rowid = old.rowid; INSERT INTO kk_chunks_fts(rowid, doc_title, content) SELECT new.rowid, new.doc_title, new.content WHERE new.chunk_type='child'; END", &[]).ok();
    let _ = database::with_db(|c| { c.pragma_update(None, "foreign_keys", "ON").ok(); Ok(()) });

    if !chunk_ids.is_empty() {
        let placeholders: Vec<String> = chunk_ids.iter().map(|_| "?".to_string()).collect();
        let params: Vec<serde_json::Value> = chunk_ids.iter().map(|id| json!(id)).collect();
        match database::run(&format!("DELETE FROM vector_cache WHERE chunk_id IN ({})", placeholders.join(",")), &params) {
            Ok(r) => eprintln!("[KB清理] 已清理 vector_cache: {} 行", r.changes),
            Err(e) => eprintln!("[KB清理] vector_cache 清理失败: {:#?}", e),
        }
        if let Ok(g) = crate::services::lancedb_client::get_lancedb() {
            if let Some(ref c) = *g {
                match c.delete_chunks_by_rowids(&chunk_ids) {
                    Ok(_) => eprintln!("[KB清理] 已清理 LanceDB"),
                    Err(e) => eprintln!("[KB清理] LanceDB 清理失败: {:#?}", e),
                }
            }
        }
    }
}

pub fn cleanup_kb_for_file(fid: &str) { crate::services::cleanup_pipeline::enqueue_cleanup("FILE", fid); }

/// 清理指定实体类型的所有关联文件（stub，由 cleanup_pipeline 统一处理）
pub fn cleanup_orphan_files_for_entity(_entity_type: &str, _entity_id: &str) {}

use std::sync::Mutex;
static STORAGE_BASE: Mutex<Option<String>> = Mutex::new(None);

fn compute_file_hash(path: &str) -> Result<(String, u64), String> {
    let f = File::open(path).map_err(|e| format!("打开文件失败: {}", e))?;
    let file_size = f.metadata().map(|m| m.len()).unwrap_or(0);
    let mut reader = BufReader::with_capacity(64 * 1024, f);
    let mut hasher = Sha256::new();
    let mut chunk = vec![0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut chunk).map_err(|e| format!("读取文件失败: {}", e))?;
        if n == 0 { break; }
        hasher.update(&chunk[..n]);
    }
    let hash = hex::encode(&hasher.finalize()[..32]);
    Ok((hash, file_size))
}

fn storage_base() -> String {
    { let guard = STORAGE_BASE.lock().unwrap(); if let Some(ref s) = *guard { return s.clone(); } }
    let default = std::env::current_exe()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .parent().unwrap_or(std::path::Path::new("."))
        .join("files");
    std::fs::create_dir_all(&default).ok();
    let s = default.to_string_lossy().to_string();
    let mut guard = STORAGE_BASE.lock().unwrap(); *guard = Some(s.clone()); s
}

pub fn invalidate_storage_base() { *STORAGE_BASE.lock().unwrap() = None; }

pub fn flat_disk_path(hash: &str) -> String { format!("{}\\{}", storage_base().trim_end_matches(&['\\','/'][..]), hash) }

fn temp_dir() -> String {
    let base = storage_base();
    let parent = std::path::Path::new(&base).parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| base.clone());
    format!("{}\\temp", parent.trim_end_matches(&['\\','/'][..]))
}

pub fn cleanup_preview_links() {
    let td = temp_dir();
    let _ = std::fs::create_dir_all(&td);
    if let Ok(entries) = std::fs::read_dir(&td) {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with("ARK-") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

fn find_existing_by_hash(hash: &str) -> Option<String> {
    database::get("SELECT id FROM file_assets WHERE content_hash=? LIMIT 1",&[json!(hash)])
        .ok().flatten()
        .and_then(|r| r["id"].as_str().map(|s| s.to_string()))
        .filter(|_| { let dp = flat_disk_path(hash); std::path::Path::new(&dp).exists() })
}

// ─── Helpers — file_relations (file_id → file_assets directly) ───

fn add_relation(file_id: &str, entity_type: &str, entity_id: &str, custom_name: &str, creator: &str, folder_id: Option<&str>) {
    database::run(
        "INSERT OR IGNORE INTO file_relations(id,file_id,entity_type,entity_id,custom_name,creator,folder_id) VALUES(?1,?2,?3,?4,?5,?6,?7)",
        &[json!(uuid::Uuid::new_v4().to_string()), json!(file_id), json!(entity_type), json!(entity_id), json!(custom_name), json!(creator), json!(folder_id)]
    ).ok();
}

fn add_relation_with_desc(file_id: &str, entity_type: &str, entity_id: &str, custom_name: &str, creator: &str, description: &str) {
    database::run(
        "INSERT OR IGNORE INTO file_relations(id,file_id,entity_type,entity_id,custom_name,creator,description) VALUES(?1,?2,?3,?4,?5,?6,?7)",
        &[json!(uuid::Uuid::new_v4().to_string()), json!(file_id), json!(entity_type), json!(entity_id), json!(custom_name), json!(creator), json!(description)]
    ).ok();
}

pub fn get_physical_path(file_id: &str) -> Option<String> {
    // file_id can be either file_assets.id or file_relations.id — try both
    let hash = database::get("SELECT content_hash FROM file_assets WHERE id=?",&[json!(file_id)]).ok().flatten()
        .and_then(|r| r["content_hash"].as_str().map(|s| s.to_string()))
        .or_else(|| {
            database::get("SELECT fa.content_hash FROM file_relations fr INNER JOIN file_assets fa ON fa.id = fr.file_id WHERE fr.id=?",&[json!(file_id)]).ok().flatten()
                .and_then(|r| r["content_hash"].as_str().map(|s| s.to_string()))
        });
    hash.filter(|h| !h.is_empty()).map(|h| flat_disk_path(&h)).filter(|p| std::path::Path::new(p).exists())
}

// ═══════════════════════════════════════════════════════════════════
// Queries
// ═══════════════════════════════════════════════════════════════════

#[tauri::command] pub async fn file_list(project_id: String) -> Result<Vec<Value>, String> {
    let sql = if project_id.is_empty() {
        "SELECT fr.id, fa.id as file_id, fa.file_ext, fa.file_size, fa.content_hash, fa.parse_status, fa.created_at, fa.updated_at, fr.custom_name, fr.remark, fr.folder_id, fr.entity_type, fr.entity_id FROM file_relations fr INNER JOIN file_assets fa ON fa.id = fr.file_id ORDER BY fr.created_at DESC"
    } else {
        "SELECT fr.id, fa.id as file_id, fa.file_ext, fa.file_size, fa.content_hash, fa.parse_status, fa.created_at, fa.updated_at, fr.custom_name as name, fr.custom_name as displayName, fr.custom_name as custom_name, fr.remark, fr.folder_id, fr.entity_type, fr.entity_id FROM file_relations fr INNER JOIN file_assets fa ON fa.id = fr.file_id WHERE fr.entity_type='project' AND fr.entity_id=? ORDER BY fr.created_at DESC"
    };
    if project_id.is_empty() { database::all(sql, &[]).map_err(|e| e.to_string()) } else { database::all(sql, &[json!(project_id)]).map_err(|e| e.to_string()) }
}

// ═══════════════════════════════════════════════════════════════════
// Upload
// ═══════════════════════════════════════════════════════════════════

pub(crate) async fn upload_file_core(
    file_path: &str,
    entity_type: &str,
    entity_ids: &[&str],
    custom_name: &str,
    creator: &str,
    note: &str,
    folder_id: Option<&str>,
) -> Result<(String, String), String> {
    let ext = std::path::Path::new(file_path)
        .extension().and_then(|e| e.to_str()).unwrap_or("").to_string();

    let fp = file_path.to_string();
    let (hash, file_size) = tauri::async_runtime::spawn_blocking(move || compute_file_hash(&fp))
        .await.map_err(|e| format!("计算 hash 失败: {}", e))?
        .map_err(|e| e.to_string())?;

    // CAS 复用
    if let Some(existing_id) = find_existing_by_hash(&hash) {
        for &eid in entity_ids {
            add_relation_with_desc(&existing_id, entity_type, eid, custom_name, creator, note);
        }
        if entity_ids.is_empty() {
            add_relation_with_desc(&existing_id, entity_type, "", custom_name, creator, note);
        }
        return Ok((existing_id.clone(), existing_id));
    }

    // 新文件
    let disk_path = flat_disk_path(&hash);
    if !std::path::Path::new(&disk_path).exists() {
        if let Some(p) = std::path::Path::new(&disk_path).parent() {
            std::fs::create_dir_all(p).map_err(|e| format!("创建目录失败: {}", e))?;
        }
        let dp = disk_path.clone();
        let fp2 = file_path.to_string();
        tauri::async_runtime::spawn_blocking(move || std::fs::copy(&fp2, &dp))
            .await.map_err(|e| format!("文件复制失败: {}", e))?
            .map_err(|e| format!("文件复制失败: {}", e))?;
    }

    let file_id = uuid::Uuid::new_v4().to_string();
    database::run(
        "INSERT INTO file_assets(id,file_ext,file_size,content_hash,parse_status) VALUES(?1,?2,?3,?4,'PENDING')",
        &[json!(&file_id), json!(&ext), json!(file_size as i64), json!(&hash)]
    ).map_err(|e| e.to_string())?;

    for &eid in entity_ids {
        add_relation_with_desc(&file_id, entity_type, eid, custom_name, creator, note);
    }
    if entity_ids.is_empty() {
        add_relation_with_desc(&file_id, entity_type, "", custom_name, creator, note);
    }

    if entity_type == "library" {
        database::run(
            "INSERT OR REPLACE INTO library_file_locations(file_id, folder_id) VALUES(?1,?2)",
            &[json!(&file_id), json!(folder_id)]
        ).ok();
    }

    // Set folder_id on file_relations for project files
    if entity_type != "library" {
        if let Some(fid) = folder_id.filter(|s| !s.is_empty()) {
            database::run(
                "UPDATE file_relations SET folder_id=? WHERE file_id=? AND entity_type=?",
                &[json!(fid), json!(&file_id), json!(entity_type)]
            ).ok();
        }
    }

    Ok((file_id.clone(), file_id))
}

#[tauri::command]
pub async fn file_upload(data: Value) -> Result<Value, String> {
    let file_path = data["filePath"].as_str().unwrap_or("");
    if file_path.is_empty() { return Ok(json!({"success":false,"error":"缺少 filePath"})); }
    let entity_type = data["entityType"].as_str().unwrap_or("library");
    let entity_ids: Vec<&str> = if let Some(arr) = data["entityIds"].as_array() {
        arr.iter().filter_map(|v| v.as_str()).collect()
    } else if let Some(single) = data["entityId"].as_str().filter(|s| !s.is_empty()) {
        vec![single]
    } else if let Some(pid) = data["projectId"].as_str().filter(|s| !s.is_empty()) {
        vec![pid]
    } else { vec![] };
    let custom_name = data["customName"].as_str().or(data["fileName"].as_str()).unwrap_or("");
    let creator = data["creator"].as_str().unwrap_or("");
    let note = data["note"].as_str().or(data["description"].as_str()).unwrap_or("");
    let folder_id = data["folderId"].as_str();
    let (file_id, _) = upload_file_core(file_path, entity_type, &entity_ids, custom_name, creator, note, folder_id).await?;
    Ok(json!({"success":true,"fileId":file_id,"objectId":file_id,"id":file_id}))
}

// ═══════════════════════════════════════════════════════════════════
// 关联操作
// ═══════════════════════════════════════════════════════════════════

#[tauri::command] pub async fn file_add_note_mapping(file_id: String, note_id: String) -> Result<Value, String> {
    if file_id.is_empty() || note_id.is_empty() { return Ok(json!({"success":false,"error":"参数缺失"})); }
    add_relation(&file_id, "note", &note_id, "", "", None);
    Ok(json!({"success":true}))
}
#[tauri::command] pub async fn file_remove_note_mapping(file_id: String, note_id: String) -> Result<Value, String> {
    database::run("DELETE FROM file_relations WHERE entity_type='note' AND entity_id=? AND file_id=?", &[json!(&note_id), json!(&file_id)]).map_err(|e| e.to_string())?;
    Ok(json!({"success":true}))
}
#[tauri::command] pub async fn file_list_by_note(note_id: String) -> Result<Vec<Value>, String> {
    database::all(
        "SELECT fr.id, fa.file_ext, fa.file_size, fa.content_hash, fa.parse_status, fa.created_at, fa.updated_at, fr.custom_name, fr.remark, fr.folder_id, fr.entity_type, fr.entity_id FROM file_relations fr INNER JOIN file_assets fa ON fa.id = fr.file_id WHERE fr.entity_type='note' AND fr.entity_id=? ORDER BY fr.created_at DESC",
        &[json!(note_id)]
    ).map_err(|e| e.to_string())
}
#[tauri::command] pub async fn file_add_todo_mapping(file_id: String, todo_id: String) -> Result<Value, String> {
    if file_id.is_empty() || todo_id.is_empty() { return Ok(json!({"success":false,"error":"参数缺失"})); }
    add_relation(&file_id, "todo", &todo_id, "", "", None);
    Ok(json!({"success":true}))
}
#[tauri::command] pub async fn file_remove_todo_mapping(file_id: String, todo_id: String) -> Result<Value, String> {
    database::run("DELETE FROM file_relations WHERE entity_type='todo' AND entity_id=? AND file_id=?", &[json!(&todo_id), json!(&file_id)]).map_err(|e| e.to_string())?;
    Ok(json!({"success":true}))
}
#[tauri::command] pub async fn file_list_by_todo(todo_id: String) -> Result<Vec<Value>, String> {
    database::all(
        "SELECT fr.id, fa.file_ext, fa.file_size, fa.content_hash, fa.parse_status, fa.created_at, fa.updated_at, fr.custom_name, fr.remark, fr.folder_id, fr.entity_type, fr.entity_id FROM file_relations fr INNER JOIN file_assets fa ON fa.id = fr.file_id WHERE fr.entity_type='todo' AND fr.entity_id=? ORDER BY fr.created_at DESC",
        &[json!(todo_id)]
    ).map_err(|e| e.to_string())
}
#[tauri::command] pub async fn file_list_global(query: Option<String>) -> Result<Vec<Value>, String> {
    if let Some(q) = query.filter(|q| !q.is_empty()) {
        let like = format!("%{}%", q);
        database::all(
            "SELECT fr.id, fa.id as file_id, fa.file_ext, fa.file_size, fa.content_hash, fa.parse_status, fa.created_at, fa.updated_at, fr.custom_name AS lib_gn, fr.custom_name AS lib_cn, fr.description AS lib_desc, lfl.folder_id AS lib_folder, fr.entity_type, fr.entity_id FROM file_relations fr INNER JOIN file_assets fa ON fa.id = fr.file_id LEFT JOIN library_file_locations lfl ON lfl.file_id = fa.id WHERE fr.entity_type='library' AND (fr.custom_name LIKE ?1 OR fa.file_ext LIKE ?1) ORDER BY fr.created_at DESC LIMIT 50",
            &[json!(like)]
        ).map_err(|e| e.to_string())
    } else {
        database::all(
            "SELECT fr.id, fa.id as file_id, fa.file_ext, fa.file_size, fa.content_hash, fa.parse_status, fa.created_at, fa.updated_at, fr.custom_name AS lib_gn, fr.custom_name AS lib_cn, fr.description AS lib_desc, lfl.folder_id AS lib_folder, fr.entity_type, fr.entity_id FROM file_relations fr INNER JOIN file_assets fa ON fa.id = fr.file_id LEFT JOIN library_file_locations lfl ON lfl.file_id = fa.id WHERE fr.entity_type='library' ORDER BY fr.created_at DESC LIMIT 200",
            &[]
        ).map_err(|e| e.to_string())
    }
}
#[tauri::command] pub async fn file_list_all() -> Result<Vec<Value>, String> {
    database::all(
        "SELECT fa.id, fa.file_ext, fa.file_size, fa.content_hash, fa.parse_status, fa.created_at, fa.updated_at FROM file_relations fr INNER JOIN file_assets fa ON fa.id = fr.file_id ORDER BY fr.created_at DESC",
        &[]
    ).map_err(|e| e.to_string())
}

#[tauri::command] pub async fn file_tree(project_id: Option<String>) -> Result<Value, String> {
    let pid = project_id.as_deref().filter(|s| !s.is_empty());
    let (folder_sql, folder_params): (String, Vec<Value>) = if let Some(p) = pid {
        ("SELECT id, name, parent_id, project_id, sort_order FROM library_folders WHERE project_id=? ORDER BY sort_order, name".to_string(), vec![json!(p)])
    } else {
        ("SELECT id, name, parent_id, project_id, sort_order FROM library_folders WHERE project_id IS NULL ORDER BY sort_order, name".to_string(), vec![])
    };
    let folders = database::all(&folder_sql, &folder_params).map_err(|e| e.to_string())?;
    let files = if let Some(p) = pid {
        database::all(
            "SELECT fr.folder_id, fr.file_id, fa.file_ext, fa.file_size, fa.created_at, fr.custom_name AS name, fr.entity_type FROM file_relations fr INNER JOIN file_assets fa ON fa.id = fr.file_id WHERE fr.entity_type='project' AND fr.entity_id=? ORDER BY fr.custom_name",
            &[json!(p)]
        ).map_err(|e| e.to_string())?
    } else {
        database::all(
            "SELECT lfl.folder_id, lfl.file_id, fa.file_ext, fa.file_size, fa.created_at, fr.custom_name AS name FROM library_file_locations lfl INNER JOIN file_assets fa ON fa.id = lfl.file_id LEFT JOIN file_relations fr ON fr.file_id = lfl.file_id AND fr.entity_type='library' ORDER BY fr.custom_name",
            &[]
        ).map_err(|e| e.to_string())?
    };
    Ok(json!({"folders":folders,"files":files}))
}

/// 简化删除：按 file_relations.id 删除关联，无剩余引用时清理 file_assets
#[tauri::command] pub async fn file_delete(file_id: String) -> Result<Value, String> {
    // file_id is file_relations.id
    // 1. Look up the asset ID
    let asset_id: Option<String> = database::get(
        "SELECT file_id FROM file_relations WHERE id=?", &[json!(&file_id)]
    ).ok().flatten().and_then(|r| r["file_id"].as_str().map(|s| s.to_string()));

    // 2. Delete the relation
    database::run("DELETE FROM file_relations WHERE id=?", &[json!(&file_id)]).ok();

    // 3. If no remaining references, clean up asset + KB + physical file
    if let Some(ref aid) = asset_id {
        let remaining: i64 = database::get("SELECT COUNT(*) as cnt FROM file_relations WHERE file_id=?", &[json!(aid)])
            .ok().flatten().and_then(|r| r["cnt"].as_i64()).unwrap_or(0);
        if remaining == 0 {
            cleanup_kb_for_file(aid);
            if let Some(p) = get_physical_path(aid) { if std::path::Path::new(&p).exists() { std::fs::remove_file(&p).ok(); } }
            database::run("DELETE FROM library_file_locations WHERE file_id=?", &[json!(aid)]).ok();
            database::run("DELETE FROM file_assets WHERE id=?", &[json!(aid)]).ok();
        }
    }
    Ok(json!({"success":true}))
}

#[tauri::command] pub async fn file_delete_from_domain(file_id: String, _entity_type: String, _entity_id: Option<String>) -> Result<Value, String> {
    // file_id here is file_relations.id (the relation record ID)
    // 1. Look up the file_assets.id from this relation
    let asset_id: Option<String> = database::get(
        "SELECT file_id FROM file_relations WHERE id=?", &[json!(&file_id)]
    ).ok().flatten().and_then(|r| r["file_id"].as_str().map(|s| s.to_string()));

    // 2. Delete the specific file_relations record
    database::run("DELETE FROM file_relations WHERE id=?", &[json!(&file_id)]).map_err(|e| e.to_string())?;

    // 3. If file_assets has no remaining relations, clean up everything
    if let Some(ref aid) = asset_id {
        let remaining: i64 = database::get("SELECT COUNT(*) as cnt FROM file_relations WHERE file_id=?", &[json!(aid)])
            .ok().flatten().and_then(|r| r["cnt"].as_i64()).unwrap_or(0);
        if remaining == 0 {
            cleanup_kb_for_file(aid);
            if let Some(p) = get_physical_path(aid) { if std::path::Path::new(&p).exists() { std::fs::remove_file(&p).ok(); } }
            database::run("DELETE FROM library_file_locations WHERE file_id=?", &[json!(aid)]).ok();
            database::run("DELETE FROM file_assets WHERE id=?", &[json!(aid)]).ok();
        }
    }
    Ok(json!({"success":true}))
}

// ═══════════════════════════════════════════════════════════════════
// File move, folder CRUD, product list, preview, export
// ═══════════════════════════════════════════════════════════════════

#[tauri::command] pub async fn file_move_to_folder(file_id: String, folder_id: Option<String>) -> Result<Value, String> {
    if file_id.is_empty() { return Ok(json!({"success":false,"error":"参数缺失"})); }
    database::run("INSERT OR REPLACE INTO library_file_locations(file_id, folder_id) VALUES(?1,?2)", &[json!(&file_id), json!(folder_id)]).ok();
    Ok(json!({"success":true}))
}
#[tauri::command] pub async fn file_folder_create(name: String, parent_id: Option<String>, project_id: Option<String>) -> Result<Value, String> {
    if name.trim().is_empty() { return Ok(json!({"success":false,"error":"名称不能为空"})); }
    let id = uuid::Uuid::new_v4().to_string();
    database::run("INSERT INTO library_folders(id,name,parent_id,project_id) VALUES(?1,?2,?3,?4)",&[json!(id),json!(name.trim()),json!(parent_id),json!(project_id)]).map_err(|e| e.to_string())?;
    Ok(json!({"success":true,"id":id}))
}
#[tauri::command] pub async fn file_folder_move(id: String, parent_id: Option<String>) -> Result<Value, String> {
    if id.is_empty() { return Ok(json!({"success":false,"error":"参数缺失"})); }
    database::run("UPDATE library_folders SET parent_id=?,updated_at=datetime('now','localtime') WHERE id=?",&[json!(parent_id),json!(&id)]).map_err(|e| e.to_string())?;
    Ok(json!({"success":true}))
}
#[tauri::command] pub async fn file_folder_reorder(id: String, sort_order: i64) -> Result<Value, String> {
    if id.is_empty() { return Ok(json!({"success":false,"error":"参数缺失"})); }
    database::run("UPDATE library_folders SET sort_order=?,updated_at=datetime('now','localtime') WHERE id=?",&[json!(sort_order),json!(&id)]).map_err(|e| e.to_string())?;
    Ok(json!({"success":true}))
}
#[tauri::command] pub async fn file_folder_rename(id: String, name: String) -> Result<Value, String> {
    if name.trim().is_empty() { return Ok(json!({"success":false,"error":"名称不能为空"})); }
    database::run("UPDATE library_folders SET name=?,updated_at=datetime('now','localtime') WHERE id=?",&[json!(name.trim()),json!(id)]).map_err(|e| e.to_string())?;
    Ok(json!({"success":true}))
}
#[tauri::command] pub async fn file_folder_delete(id: String) -> Result<Value, String> {
    if id.is_empty() { return Ok(json!({"success":false,"error":"参数缺失"})); }
    let subs: i64 = database::with_db(|c| c.query_row("SELECT COUNT(*) FROM library_folders WHERE parent_id=?", rusqlite::params![&id], |r| r.get(0))).unwrap_or(0);
    let files: i64 = database::with_db(|c| c.query_row("SELECT COUNT(*) FROM library_file_locations WHERE file_id IN (SELECT file_id FROM library_file_locations WHERE file_id=? AND folder_id=?)", rusqlite::params![&id], |r| r.get(0))).unwrap_or(0);
    if subs > 0 || files > 0 { return Ok(json!({"success":false,"error":"文件夹不为空，请先清空内容"})); }
    database::run("DELETE FROM library_folders WHERE id=?",&[json!(&id)]).map_err(|e| e.to_string())?;
    Ok(json!({"success":true}))
}

#[tauri::command] pub async fn file_list_by_product(product_id: String) -> Result<Vec<Value>, String> {
    database::all(
        "SELECT fr.id, fa.file_ext, fa.file_size, fa.content_hash, fa.parse_status, fr.created_at, fa.updated_at, fr.custom_name AS global_name, fr.description AS mapping_type \
         FROM file_relations fr INNER JOIN file_assets fa ON fa.id = fr.file_id \
         WHERE fr.entity_type='product' AND fr.entity_id=? ORDER BY fr.created_at DESC",
        &[json!(product_id)]
    ).map_err(|e| e.to_string())
}

fn resolve_file_info(file_id: &str) -> Option<(String, String, String)> {
    // Try as file_relations.id first, then as file_assets.id (for old data / legacy callers)
    let row = database::get(
        "SELECT fa.content_hash, fa.file_ext, fr.custom_name FROM file_relations fr INNER JOIN file_assets fa ON fa.id = fr.file_id WHERE fr.id=?",
        &[json!(file_id)],
    ).ok().flatten().or_else(|| {
        database::get(
            "SELECT fa.content_hash, fa.file_ext, fr.custom_name FROM file_relations fr INNER JOIN file_assets fa ON fa.id = fr.file_id WHERE fr.file_id=? LIMIT 1",
            &[json!(file_id)],
        ).ok().flatten()
    })?;
    let hash = row["content_hash"].as_str().filter(|h| !h.is_empty())?;
    let src = flat_disk_path(hash);
    if !std::path::Path::new(&src).exists() { return None; }
    let ext = row["file_ext"].as_str().unwrap_or("").to_string();
    let name = row["custom_name"].as_str().unwrap_or("").to_string();
    Some((src, name, ext))
}

#[tauri::command] pub async fn file_preview(file_id: String) -> Result<Value, String> {
    match resolve_file_info(&file_id) {
        Some((src, name, ext)) => {
            let base = if name.is_empty() { "ARK-unknown" } else { name.as_str() };
            let tmp_name = if ext.is_empty() || base.to_lowercase().ends_with(&format!(".{}", ext.to_lowercase())) {
                format!("ARK-{}", base)
            } else { format!("ARK-{}.{}", base, ext) };
            let td = temp_dir();
            std::fs::create_dir_all(&td).ok();
            if let Ok(entries) = std::fs::read_dir(&td) {
                for entry in entries.flatten() {
                    if entry.file_name().to_string_lossy().starts_with("ARK-") { let _ = std::fs::remove_file(entry.path()); }
                }
            }
            let tmp_path = std::path::Path::new(&td).join(&tmp_name);
            std::fs::hard_link(&src, &tmp_path).map_err(|e| format!("创建硬链接失败: {}", e))?;
            let mut perms = std::fs::metadata(&tmp_path).map_err(|e| format!("读取权限失败: {}", e))?.permissions();
            perms.set_readonly(true); std::fs::set_permissions(&tmp_path, perms).ok();
            opener::open(&tmp_path.to_string_lossy().to_string()).ok();
            Ok(json!({"success":true}))
        }
        None => Ok(json!({"success":false,"error":"文件不存在"}))
    }
}

#[tauri::command] pub async fn file_export(file_id: String) -> Result<Value, String> {
    match resolve_file_info(&file_id) {
        Some((src, name, ext)) => {
            let filename = if name.is_empty() { "download".to_string() } else {
                if ext.is_empty() || name.to_lowercase().ends_with(&format!(".{}", ext.to_lowercase())) { name }
                else { format!("{}.{}", name, ext) }
            };
            Ok(json!({"success":true,"filePath":src,"fileName":filename}))
        }
        None => Ok(json!({"success":false,"error":"文件不存在"}))
    }
}

#[tauri::command] pub async fn file_export_to_path(file_path: String, save_path: String) -> Result<Value, String> {
    std::fs::copy(&file_path, &save_path).map_err(|e| format!("拷贝失败: {}", e))?;
    Ok(json!({"success":true}))
}

// ── Stubs / legacy compat ──
#[tauri::command] pub async fn file_open(version_id: String) -> Result<Value, String> { file_preview(version_id).await }
#[tauri::command] pub async fn file_download(data: Value) -> Result<Value, String> { let fid = data["versionId"].as_str().or(data["fileId"].as_str()).unwrap_or(""); file_export(fid.to_string()).await }
#[tauri::command] pub async fn file_get_path(_version_id: String) -> Result<String, String> { Ok(String::new()) }
#[tauri::command] pub async fn file_has_oss_files() -> Result<Value, String> { Ok(json!({"success":true,"hasOss":false})) }
#[tauri::command] pub async fn file_migrate_files(_old: String, _new: String) -> Result<Value, String> { Ok(json!({"success":true})) }
#[tauri::command] pub async fn file_update_meta(file_id: String, data: Value) -> Result<Value, String> {
    if let Some(cn) = data["customName"].as_str().or(data["globalName"].as_str()) {
        database::run("UPDATE file_relations SET custom_name=? WHERE id=?", &[json!(cn), json!(&file_id)]).ok();
    }
    if let Some(rm) = data["remark"].as_str() {
        database::run("UPDATE file_relations SET remark=? WHERE id=?", &[json!(rm), json!(&file_id)]).ok();
    }
    if data.get("folderId").is_some() {
        let fid = data["folderId"].as_str().map(|s| json!(s)).unwrap_or(Value::Null);
        database::run("UPDATE file_relations SET folder_id=? WHERE id=?", &[fid, json!(&file_id)]).ok();
    }
    Ok(json!({"success":true}))
}
#[tauri::command] pub async fn file_get_tags(file_id: String) -> Result<Value, String> { let tags = database::all("SELECT t.* FROM tags t INNER JOIN tag_mappings tm ON tm.tag_id=t.tag_id WHERE tm.entity_type='files' AND tm.entity_id=? ORDER BY t.name",&[json!(file_id)]).map_err(|e| e.to_string())?; Ok(json!({"success":true,"tags":tags})) }
#[tauri::command] pub async fn file_add_tag_mapping(file_id: String, tag_name: String) -> Result<Value, String> { let tag_id = match database::get("SELECT tag_id FROM tags WHERE name=?",&[json!(&tag_name)]).ok().flatten() { Some(r) => r["tag_id"].as_str().unwrap_or("").to_string(), None => { let nid = uuid::Uuid::new_v4().to_string(); database::run("INSERT INTO tags(tag_id,name,color,type) VALUES(?,?,?,'general')",&[json!(&nid),json!(&tag_name),json!("#6366f1")]).ok(); nid } }; database::run("INSERT OR IGNORE INTO tag_mappings(entity_type,entity_id,tag_id) VALUES('files',?,?)",&[json!(&file_id),json!(&tag_id)]).map_err(|e| e.to_string())?; Ok(json!({"success":true})) }
#[tauri::command] pub async fn file_remove_tag_mapping(file_id: String, tag_id: String) -> Result<Value, String> { database::run("DELETE FROM tag_mappings WHERE entity_type='files' AND entity_id=? AND tag_id=?",&[json!(&file_id),json!(&tag_id)]).map_err(|e| e.to_string())?; Ok(json!({"success":true})) }
#[tauri::command] pub async fn file_move(data: Value) -> Result<Value, String> {
    let file_id = data["fileId"].as_str().unwrap_or("");
    let folder_id: Option<String> = data.get("toFolderId").and_then(|v| v.as_str().map(|s| s.to_string()));
    if file_id.is_empty() { return Ok(json!({"success":false})); }
    database::run("INSERT OR REPLACE INTO library_file_locations(file_id, folder_id) VALUES(?1,?2)", &[json!(&file_id), json!(folder_id)]).ok();
    Ok(json!({"success":true}))
}
#[tauri::command] pub async fn file_cleanup_if_orphan(_file_id: String) -> Result<Value, String> { Ok(json!({"success":true})) }
#[tauri::command] pub async fn file_open_product(file_id: String) -> Result<Value, String> { file_preview(file_id).await }
#[tauri::command] pub async fn file_download_product(data: Value) -> Result<Value, String> { file_download(data).await }
#[tauri::command] pub async fn file_unlink_from_product(file_id: String, product_id: String) -> Result<Value, String> {
    database::run("DELETE FROM file_relations WHERE entity_type='product' AND entity_id=? AND id=?", &[json!(&product_id), json!(&file_id)]).ok();
    Ok(json!({"success":true}))
}
#[tauri::command] pub async fn file_get_product_associations(file_id: String) -> Result<Value, String> {
    let ids = database::all("SELECT entity_id FROM file_relations WHERE entity_type='product' AND file_id=?",&[json!(&file_id)]).unwrap_or_default();
    Ok(json!({"success":true,"products":ids}))
}
#[tauri::command] pub async fn file_update_product_associations(file_id: String, data: Value) -> Result<Value, String> {
    database::run("DELETE FROM file_relations WHERE entity_type='product' AND file_id=?",&[json!(&file_id)]).ok();
    if let Some(ids) = data["productIds"].as_array() {
        for pid in ids { if let Some(p) = pid.as_str() { add_relation(&file_id, "product", p, "", "", None); } }
    }
    Ok(json!({"success":true}))
}
#[tauri::command] pub async fn file_repair_() -> Result<Value, String> { Ok(json!({"success":true})) }
