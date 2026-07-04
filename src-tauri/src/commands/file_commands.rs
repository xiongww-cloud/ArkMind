use crate::database;
use serde_json::{json, Value};
use sha2::{Sha256, Digest};
use std::io::{Read, BufReader};
use std::fs::File;

/// Unified knowledge base cleanup for any source type.
pub fn cleanup_kb(source_type: &str, source_id: &str) {
    crate::services::knowledge_lifecycle::delete_object(source_type, source_id);
}

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
    { let guard = STORAGE_BASE.lock().unwrap_or_else(|e| e.into_inner()); if let Some(ref s) = *guard { return s.clone(); } }
    let default = std::env::current_exe()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .parent().unwrap_or(std::path::Path::new("."))
        .join("files");
    std::fs::create_dir_all(&default).ok();
    let s = default.to_string_lossy().to_string();
    let mut guard = STORAGE_BASE.lock().unwrap_or_else(|e| e.into_inner()); *guard = Some(s.clone()); s
}

pub fn invalidate_storage_base() { *STORAGE_BASE.lock().unwrap_or_else(|e| e.into_inner()) = None; }

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
    let meta = serde_json::json!({"custom_name": custom_name, "creator": creator});
    crate::services::business_relations::upsert("FILE", file_id, &entity_type.to_uppercase(), entity_id, "has_file", &meta.to_string());
}

fn add_relation_with_desc(file_id: &str, entity_type: &str, entity_id: &str, custom_name: &str, creator: &str, description: &str) {
    database::run(
        "INSERT OR IGNORE INTO file_relations(id,file_id,entity_type,entity_id,custom_name,creator,description) VALUES(?1,?2,?3,?4,?5,?6,?7)",
        &[json!(uuid::Uuid::new_v4().to_string()), json!(file_id), json!(entity_type), json!(entity_id), json!(custom_name), json!(creator), json!(description)]
    ).ok();
    let meta = serde_json::json!({"custom_name": custom_name, "creator": creator, "description": description});
    crate::services::business_relations::upsert("FILE", file_id, &entity_type.to_uppercase(), entity_id, "has_file", &meta.to_string());
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
    let original_name = if custom_name.is_empty() {
        std::path::Path::new(file_path).file_name().and_then(|n| n.to_str()).unwrap_or("").to_string()
    } else {
        custom_name.to_string()
    };
    database::run(
        "INSERT INTO file_assets(id,file_ext,file_size,content_hash,original_name,parse_status) VALUES(?1,?2,?3,?4,?5,'PENDING')",
        &[json!(&file_id), json!(&ext), json!(file_size as i64), json!(&hash), json!(&original_name)]
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

    // 通知知识库：解析 → 清洗 → 写入 source_data → 触发同步
    crate::services::knowledge_lifecycle::notify_file_upload(&file_id);

    Ok((file_id.clone(), file_id))
}

#[tauri::command]
pub async fn file_upload(data: Value) -> Result<Value, String> {
    let file_path = data["filePath"].as_str().unwrap_or("");
    if file_path.is_empty() { return Ok(json!({"success":false,"error":"缺少 filePath"})); }

    // 路径安全验证：禁止相对路径穿越
    let path = std::path::Path::new(file_path);
    if path.is_relative() {
        return Ok(json!({"success":false,"error":"不支持相对路径"}));
    }
    // 检查路径规范化后是否仍包含 ..
    if let Ok(canonical) = path.canonicalize() {
        let components: Vec<_> = canonical.components().collect();
        for comp in &components {
            if let std::path::Component::ParentDir = comp {
                return Ok(json!({"success":false,"error":"路径包含非法穿越"}));
            }
        }
    }

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
    crate::services::object_changed::object_changed(crate::services::object_changed::ObjectType::Note, &note_id, crate::services::object_changed::Action::Updated);
    Ok(json!({"success":true}))
}
#[tauri::command] pub async fn file_remove_note_mapping(file_id: String, note_id: String) -> Result<Value, String> {
    database::run("DELETE FROM file_relations WHERE entity_type='note' AND entity_id=? AND file_id=?", &[json!(&note_id), json!(&file_id)]).map_err(|e| e.to_string())?;
    crate::services::object_changed::object_changed(crate::services::object_changed::ObjectType::Note, &note_id, crate::services::object_changed::Action::Updated);
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
#[tauri::command] pub async fn file_list_global(query: Option<String>) -> Result<Value, String> {
    let files = if let Some(q) = query.filter(|q| !q.is_empty()) {
        let like = format!("%{}%", crate::database::escape_like(&q));
        database::all(
            "SELECT fr.id, fa.id as file_id, fa.file_ext, fa.file_size, fa.content_hash, fa.parse_status, fa.created_at, fa.updated_at, fr.custom_name AS lib_gn, fr.custom_name AS lib_cn, fr.description AS lib_desc, lfl.folder_id AS lib_folder_id, fr.entity_type, fr.entity_id FROM file_relations fr INNER JOIN file_assets fa ON fa.id = fr.file_id LEFT JOIN library_file_locations lfl ON lfl.file_id = fa.id WHERE fr.entity_type='library' AND (fr.custom_name LIKE ?1 OR fa.file_ext LIKE ?1) ORDER BY fr.created_at DESC LIMIT 50",
            &[json!(like)]
        ).map_err(|e| e.to_string())?
    } else {
        database::all(
            "SELECT fr.id, fa.id as file_id, fa.file_ext, fa.file_size, fa.content_hash, fa.parse_status, fa.created_at, fa.updated_at, fr.custom_name AS lib_gn, fr.custom_name AS lib_cn, fr.description AS lib_desc, lfl.folder_id AS lib_folder_id, fr.entity_type, fr.entity_id FROM file_relations fr INNER JOIN file_assets fa ON fa.id = fr.file_id LEFT JOIN library_file_locations lfl ON lfl.file_id = fa.id WHERE fr.entity_type='library' ORDER BY fr.created_at DESC LIMIT 200",
            &[]
        ).map_err(|e| e.to_string())?
    };
    Ok(json!({"files": files}))
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
    // 0. Look up entity info BEFORE deleting (for business_relations cleanup)
    let rel_info: Option<(String, String, String)> = database::get(
        "SELECT file_id, entity_type, entity_id FROM file_relations WHERE id=?", &[json!(&file_id)]
    ).ok().flatten().map(|r| (
        r["file_id"].as_str().unwrap_or("").to_string(),
        r["entity_type"].as_str().unwrap_or("").to_string(),
        r["entity_id"].as_str().unwrap_or("").to_string(),
    ));

    // 1. Look up the asset ID
    let asset_id: Option<String> = rel_info.as_ref().map(|(fid, _, _)| fid.clone());

    // 2. Delete the relation
    database::run("DELETE FROM file_relations WHERE id=?", &[json!(&file_id)]).ok();

    // 3. If no remaining references, clean up asset + KB + physical file
    if let Some(ref aid) = asset_id {
        let remaining: i64 = database::get("SELECT COUNT(*) as cnt FROM file_relations WHERE file_id=?", &[json!(aid)])
            .ok().flatten().and_then(|r| r["cnt"].as_i64()).unwrap_or(0);
        if remaining == 0 {
            crate::services::knowledge_lifecycle::notify_deleted("FILE", aid);
            if let Some(p) = get_physical_path(aid) { if std::path::Path::new(&p).exists() { std::fs::remove_file(&p).ok(); } }
            database::run("DELETE FROM library_file_locations WHERE file_id=?", &[json!(aid)]).ok();
            database::run("DELETE FROM file_assets WHERE id=?", &[json!(aid)]).ok();
        }
    }

    // 4. Sync business_relations
    if let Some((_, et, eid)) = &rel_info {
        crate::services::business_relations::delete("FILE", &asset_id.unwrap_or_default(), &et.to_uppercase(), eid, "has_file");
    }

    crate::services::object_changed::object_changed(crate::services::object_changed::ObjectType::File, &file_id, crate::services::object_changed::Action::Deleted);
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
            crate::services::knowledge_lifecycle::notify_deleted("FILE", aid);
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
    // file_id may be fr.id (file_relations PK) — resolve to fa.id (file_assets PK)
    let asset_id = resolve_file_asset_id(&file_id).unwrap_or_else(|| file_id.clone());
    database::run("INSERT OR REPLACE INTO library_file_locations(file_id, folder_id) VALUES(?1,?2)", &[json!(&asset_id), json!(folder_id)]).ok();
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
    if std::path::Path::new(&src).exists() {
        let ext = row["file_ext"].as_str().unwrap_or("").to_string();
        let name = row["custom_name"].as_str().unwrap_or("").to_string();
        return Some((src, name, ext));
    }
    // Fallback: search for the file by hash in known storage locations
    let exe_parent = std::env::current_exe().ok()
        .and_then(|p| p.parent().map(|pp| pp.to_path_buf()));
    if let Some(parent) = exe_parent {
        for subdir in &["files", "target/debug/files", "target/release/files"] {
            let candidate = parent.join(subdir).join(hash);
            if candidate.exists() {
                let ext = row["file_ext"].as_str().unwrap_or("").to_string();
                let name = row["custom_name"].as_str().unwrap_or("").to_string();
                return Some((candidate.to_string_lossy().to_string(), name, ext));
            }
        }
    }
    None
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
    // 路径安全验证：禁止相对路径穿越
    let src = std::path::Path::new(&file_path);
    let dst = std::path::Path::new(&save_path);
    if src.is_relative() || dst.is_relative() {
        return Ok(json!({"success":false,"error":"不支持相对路径"}));
    }
    // 检查路径规范化后是否仍包含 ..
    if let (Ok(canonical_src), Ok(canonical_dst)) = (src.canonicalize(), dst.canonicalize()) {
        for comp in canonical_src.components().chain(canonical_dst.components()) {
            if let std::path::Component::ParentDir = comp {
                return Ok(json!({"success":false,"error":"路径包含非法穿越"}));
            }
        }
    }
    std::fs::copy(&file_path, &save_path).map_err(|e| format!("拷贝失败: {}", e))?;
    Ok(json!({"success":true}))
}

#[tauri::command] pub async fn file_read_bytes(file_id: String) -> Result<Value, String> {
    use base64::Engine;
    match resolve_file_info(&file_id) {
        Some((src, _, _)) => {
            let buf = std::fs::read(&src).map_err(|e| format!("读取文件失败: {}", e))?;
            let b64 = base64::engine::general_purpose::STANDARD.encode(&buf);
            Ok(json!({"success":true,"data":b64}))
        }
        None => Ok(json!({"success":false,"error":"文件不存在"}))
    }
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
    let asset_id = resolve_file_asset_id(file_id).unwrap_or_else(|| file_id.to_string());
    database::run("INSERT OR REPLACE INTO library_file_locations(file_id, folder_id) VALUES(?1,?2)", &[json!(&asset_id), json!(folder_id)]).ok();
    Ok(json!({"success":true}))
}
#[tauri::command] pub async fn file_cleanup_if_orphan(_file_id: String) -> Result<Value, String> { Ok(json!({"success":true})) }
#[tauri::command] pub async fn file_open_product(file_id: String) -> Result<Value, String> { file_preview(file_id).await }
#[tauri::command] pub async fn file_download_product(data: Value) -> Result<Value, String> { file_download(data).await }
#[tauri::command] pub async fn file_unlink_from_product(file_id: String, _product_id: String) -> Result<Value, String> {
    file_delete(file_id).await
}
// ─── Resolve fr.id (file_relations PK) to fr.file_id (file_assets FK) ───
fn resolve_file_asset_id(relation_id: &str) -> Option<String> {
    database::get("SELECT file_id FROM file_relations WHERE id=?", &[json!(relation_id)])
        .ok().flatten()
        .and_then(|r| r["file_id"].as_str().map(|s| s.to_string()))
}

#[tauri::command] pub async fn file_get_product_associations(file_id: String) -> Result<Value, String> {
    // file_id may be file_relations.id (fr.id) or file_assets.id; resolve to file_assets.id
    let asset_id = resolve_file_asset_id(&file_id).unwrap_or_else(|| file_id.clone());
    let ids = database::all("SELECT entity_id FROM file_relations WHERE entity_type='product' AND file_id=?",&[json!(&asset_id)]).unwrap_or_default();
    Ok(json!({"success":true,"products":ids}))
}
#[tauri::command] pub async fn file_update_product_associations(file_id: String, data: Value) -> Result<Value, String> {
    // Resolve to file_assets.id
    let asset_id = resolve_file_asset_id(&file_id).unwrap_or_else(|| file_id.clone());

    // Preserve custom_name and description from existing records before deleting
    let old_info: Option<(String, String)> = database::get(
        "SELECT custom_name, description FROM file_relations WHERE entity_type='product' AND file_id=? LIMIT 1",
        &[json!(&asset_id)],
    ).ok().flatten().map(|r| {
        (r["custom_name"].as_str().unwrap_or("").to_string(), r["description"].as_str().unwrap_or("").to_string())
    });
    let (custom_name, description) = old_info.unwrap_or_default();

    database::run("DELETE FROM file_relations WHERE entity_type='product' AND file_id=?",&[json!(&asset_id)]).ok();
    let has_new = data["productIds"].as_array().map(|a| !a.is_empty()).unwrap_or(false);
    if has_new {
        if let Some(ids) = data["productIds"].as_array() {
            for pid in ids { if let Some(p) = pid.as_str() { add_relation_with_desc(&asset_id, "product", p, &custom_name, "", &description); } }
        }
    } else {
        let remaining: i64 = database::get("SELECT COUNT(*) as cnt FROM file_relations WHERE file_id=?",&[json!(&asset_id)])
            .ok().flatten().and_then(|r| r["cnt"].as_i64()).unwrap_or(0);
        if remaining == 0 {
            crate::services::knowledge_lifecycle::notify_deleted("FILE", &asset_id);
            if let Some(p) = get_physical_path(&asset_id) { if std::path::Path::new(&p).exists() { std::fs::remove_file(&p).ok(); } }
            database::run("DELETE FROM library_file_locations WHERE file_id=?",&[json!(&asset_id)]).ok();
            database::run("DELETE FROM file_assets WHERE id=?",&[json!(&asset_id)]).ok();
        }
    }
    Ok(json!({"success":true}))
}
#[tauri::command] pub async fn file_repair_() -> Result<Value, String> { Ok(json!({"success":true})) }
