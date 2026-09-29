use crate::database;
use serde_json::{json, Value};
use sha2::{Sha256, Digest};
use std::io::{Read, BufReader};
use std::fs::File;

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

pub fn storage_base() -> String {
    { let guard = STORAGE_BASE.lock().unwrap_or_else(|e| e.into_inner()); if let Some(ref s) = *guard { return s.clone(); } }
    
    // 使用 AppData 路径作为稳定的文件存储位置
    let default = dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("com.arkmind.app")
        .join("files")
        .to_string_lossy()
        .to_string();
    
    std::fs::create_dir_all(&default).ok();
    let mut guard = STORAGE_BASE.lock().unwrap_or_else(|e| e.into_inner());
    *guard = Some(default.clone());
    default
}

pub fn invalidate_storage_base() { *STORAGE_BASE.lock().unwrap_or_else(|e| e.into_inner()) = None; }

pub fn flat_disk_path(hash: &str) -> String {
    // 强校验：sha256 hex 恰为 64 位十六进制，其余一律拒绝（黑名单可被短名越出扁平命名空间）
    if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return String::new();
    }
    std::path::Path::new(storage_base().trim_end_matches(&['\\','/'][..]))
        .join(hash)
        .to_string_lossy()
        .to_string()
}

fn temp_dir() -> String {
    let base = storage_base();
    let parent = std::path::Path::new(&base).parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| base.clone());
    std::path::Path::new(parent.trim_end_matches(&['\\','/'][..]))
        .join("temp")
        .to_string_lossy()
        .to_string()
}

/// 清空应用 temp 目录（预览临时文件 ARK-*、剪贴板暂存等一次性文件）。
/// 该目录只存放可抛弃的临时物，启动时整目录清空。
pub fn cleanup_preview_links() {
    let td = temp_dir();
    let _ = std::fs::create_dir_all(&td);
    if let Ok(entries) = std::fs::read_dir(&td) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let _ = std::fs::remove_dir_all(&path);
            } else {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
}

pub(crate) fn add_relation(file_id: &str, entity_type: &str, entity_id: &str, custom_name: &str, creator: &str, folder_id: Option<&str>) {
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
        "SELECT fr.id, fa.id as file_id, fa.file_ext, fa.file_size, fa.content_hash, fa.created_at, fa.updated_at, fr.custom_name, fr.remark, fr.folder_id, fr.entity_type, fr.entity_id FROM file_relations fr INNER JOIN file_assets fa ON fa.id = fr.file_id ORDER BY fr.created_at DESC"
    } else {
        "SELECT fr.id, fa.id as file_id, fa.file_ext, fa.file_size, fa.content_hash, fa.created_at, fa.updated_at, fr.custom_name as name, fr.custom_name as displayName, fr.custom_name as custom_name, fr.remark, fr.folder_id, fr.entity_type, fr.entity_id FROM file_relations fr INNER JOIN file_assets fa ON fa.id = fr.file_id WHERE fr.entity_type='project' AND fr.entity_id=? ORDER BY fr.created_at DESC"
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
    // ★ 内容权威：魔数嗅探优先决定 file_ext，文件名（含 Outlook CID 垃圾后缀）仅作回退
    let fp0 = file_path.to_string();
    let name0 = std::path::Path::new(file_path)
        .file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
    let ext = tauri::async_runtime::spawn_blocking(move || crate::services::file_parser::sniff_file_ext(&fp0, &name0))
        .await.map_err(|e| format!("嗅探失败: {}", e))?;

    let fp = file_path.to_string();
    let (hash, file_size) = tauri::async_runtime::spawn_blocking(move || compute_file_hash(&fp))
        .await.map_err(|e| format!("计算 hash 失败: {}", e))?
        .map_err(|e| e.to_string())?;

    // CAS 复用：DB 有同 hash 行即复用（原实现要求磁盘也在，否则落到新分支 INSERT 撞 content_hash UNIQUE 永久失败）
    let db_existing_id: Option<String> = database::get("SELECT id FROM file_assets WHERE content_hash=? LIMIT 1",&[json!(&hash)])
        .ok().flatten().and_then(|r| r["id"].as_str().map(|s| s.to_string()));
    if let Some(existing_id) = db_existing_id {
        // 磁盘文件可能丢失（换机/清理/漂移）→ 用刚上传的临时文件补回，再复用
        let disk_path = flat_disk_path(&hash);
        if !disk_path.is_empty() && !std::path::Path::new(&disk_path).exists() {
            if let Some(p) = std::path::Path::new(&disk_path).parent() { let _ = std::fs::create_dir_all(p); }
            let dp = disk_path.clone();
            let fp2 = file_path.to_string();
            let _ = tauri::async_runtime::spawn_blocking(move || std::fs::copy(&fp2, &dp)).await;
        }
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
        "INSERT INTO file_assets(id,file_ext,file_size,content_hash,original_name,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,datetime('now','localtime'),datetime('now','localtime'))",
        &[json!(&file_id), json!(&ext), json!(file_size as i64), json!(&hash), json!(&original_name)]
    ).map_err(|e| e.to_string())?;

    for &eid in entity_ids {
        add_relation_with_desc(&file_id, entity_type, eid, custom_name, creator, note);
    }
    if entity_ids.is_empty() {
        add_relation_with_desc(&file_id, entity_type, "", custom_name, creator, note);
    }

    // 上传文件到项目 → 刷新项目"最后活跃时间"
    if entity_type == "project" {
        for &eid in entity_ids {
            crate::commands::project_commands::touch_project(eid);
        }
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

#[tauri::command] pub async fn file_remove_note_mapping(file_id: String, note_id: String) -> Result<Value, String> {
    database::run("DELETE FROM file_relations WHERE entity_type='note' AND entity_id=? AND file_id=?", &[json!(&note_id), json!(&file_id)]).map_err(|e| e.to_string())?;
    release_asset_if_orphan(&file_id);
    Ok(json!({"success":true}))
}
#[tauri::command] pub async fn file_list_by_note(note_id: String) -> Result<Vec<Value>, String> {
    database::all(
        "SELECT fr.id, fa.file_ext, fa.file_size, fa.content_hash, fa.created_at, fa.updated_at, fr.custom_name, fr.remark, fr.folder_id, fr.entity_type, fr.entity_id FROM file_relations fr INNER JOIN file_assets fa ON fa.id = fr.file_id WHERE fr.entity_type='note' AND fr.entity_id=? ORDER BY fr.created_at DESC",
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
    release_asset_if_orphan(&file_id);
    Ok(json!({"success":true}))
}
#[tauri::command] pub async fn file_list_by_todo(todo_id: String) -> Result<Vec<Value>, String> {
    database::all(
        "SELECT fr.id, fa.file_ext, fa.file_size, fa.content_hash, fa.created_at, fa.updated_at, fr.custom_name, fr.remark, fr.folder_id, fr.entity_type, fr.entity_id FROM file_relations fr INNER JOIN file_assets fa ON fa.id = fr.file_id WHERE fr.entity_type='todo' AND fr.entity_id=? ORDER BY fr.created_at DESC",
        &[json!(todo_id)]
    ).map_err(|e| e.to_string())
}
#[tauri::command] pub async fn file_list_global(query: Option<String>) -> Result<Value, String> {
    let files = if let Some(q) = query.filter(|q| !q.is_empty()) {
        let like = format!("%{}%", crate::database::escape_like(&q));
        database::all(
            "SELECT fr.id, fa.id as file_id, fa.file_ext, fa.file_size, fa.content_hash, fa.created_at, fa.updated_at, fr.custom_name AS lib_gn, fr.custom_name AS lib_cn, fr.description AS lib_desc, fr.remark AS lib_remark, lfl.folder_id AS lib_folder, fr.entity_type, fr.entity_id FROM file_relations fr INNER JOIN file_assets fa ON fa.id = fr.file_id LEFT JOIN library_file_locations lfl ON lfl.file_id = fa.id WHERE fr.entity_type='library' AND (fr.custom_name LIKE ?1 ESCAPE '\\' OR fa.file_ext LIKE ?1 ESCAPE '\\') ORDER BY fr.created_at DESC LIMIT 50",
            &[json!(like)]
        ).map_err(|e| e.to_string())?
    } else {
        database::all(
            "SELECT fr.id, fa.id as file_id, fa.file_ext, fa.file_size, fa.content_hash, fa.created_at, fa.updated_at, fr.custom_name AS lib_gn, fr.custom_name AS lib_cn, fr.description AS lib_desc, fr.remark AS lib_remark, lfl.folder_id AS lib_folder, fr.entity_type, fr.entity_id FROM file_relations fr INNER JOIN file_assets fa ON fa.id = fr.file_id LEFT JOIN library_file_locations lfl ON lfl.file_id = fa.id WHERE fr.entity_type='library' ORDER BY fr.created_at DESC LIMIT 200",
            &[]
        ).map_err(|e| e.to_string())?
    };
    Ok(json!({"files": files}))
}
#[tauri::command] pub async fn file_list_all() -> Result<Vec<Value>, String> {
    database::all(
        "SELECT fa.id, fa.file_ext, fa.file_size, fa.content_hash, fa.created_at, fa.updated_at FROM file_relations fr INNER JOIN file_assets fa ON fa.id = fr.file_id ORDER BY fr.created_at DESC",
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
            "SELECT fr.folder_id, fr.file_id, fa.file_ext, fa.file_size, fa.created_at, fr.custom_name AS name, fr.description AS remark, fr.entity_type FROM file_relations fr INNER JOIN file_assets fa ON fa.id = fr.file_id WHERE fr.entity_type='project' AND fr.entity_id=? ORDER BY fr.custom_name",
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

/// 资产是否仍被引用：file_relations 或 email_attachments（邮件附件可引用库内资产，漏算会导致附件碎裂）
fn asset_still_referenced(asset_id: &str) -> bool {
    let rels: i64 = database::get("SELECT COUNT(*) as cnt FROM file_relations WHERE file_id=?", &[json!(asset_id)])
        .ok().flatten().and_then(|r| r["cnt"].as_i64()).unwrap_or(0);
    if rels > 0 { return true; }
    let emails: i64 = database::get("SELECT COUNT(*) as cnt FROM email_attachments WHERE file_asset_id=?", &[json!(asset_id)])
        .ok().flatten().and_then(|r| r["cnt"].as_i64()).unwrap_or(0);
    emails > 0
}

/// 零引用释放：资产不再被任何 file_relations / email_attachments 引用时，
/// 回收知识库、FILE 关联边、物理文件、目录位置行与资产行。空 id / 仍被引用时为 no-op。
pub(crate) fn release_asset_if_orphan(asset_id: &str) {
    if asset_id.is_empty() { return; }
    if asset_still_referenced(asset_id) { return; }
    crate::services::knowledge_lifecycle::notify_deleted("FILE", asset_id);
    if let Some(p) = get_physical_path(asset_id) { if std::path::Path::new(&p).exists() { std::fs::remove_file(&p).ok(); } }
    database::run("DELETE FROM library_file_locations WHERE file_id=?", &[json!(asset_id)]).ok();
    database::run("DELETE FROM business_relations WHERE (source_type='FILE' AND source_id=?) OR (target_type='FILE' AND target_id=?)", &[json!(asset_id), json!(asset_id)]).ok();
    database::run("DELETE FROM file_assets WHERE id=?", &[json!(asset_id)]).ok();
    log::info!("[资产回收] 零引用资产已清理: {}", asset_id);
}

/// 在删除引用行**之前**收集某组实体关联的资产 id（file_relations + 邮件域的 email_attachments）
pub(crate) fn collect_entity_asset_ids(entity_type: &str, entity_ids: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if entity_ids.is_empty() { return out; }
    let ph = entity_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let mut vals: Vec<Value> = vec![json!(entity_type)];
    vals.extend(entity_ids.iter().map(|s| json!(s)));
    if let Ok(rows) = database::all(
        &format!("SELECT DISTINCT file_id FROM file_relations WHERE entity_type=? AND entity_id IN ({})", ph), &vals) {
        out.extend(rows.iter().filter_map(|r| r["file_id"].as_str().map(String::from)));
    }
    if entity_type == "email" {
        if let Ok(rows) = database::all(
            &format!("SELECT DISTINCT file_asset_id FROM email_attachments WHERE email_id IN ({})", ph), &vals[1..]) {
            out.extend(rows.iter().filter_map(|r| r["file_asset_id"].as_str().filter(|s| !s.is_empty()).map(String::from)));
        }
    }
    out.sort(); out.dedup();
    out
}

/// 自动/手动共用的孤儿扫描宽限期：上传时 file_assets 插入与 file_relations 插入是两条语句，
/// 存在"资产刚建、引用未写"的瞬时窗口；没有宽限的周期扫描会误删刚上传的附件（backfill 2 分钟宽限同一先例）
const SWEEP_GRACE_MINUTES: i64 = 10;

/// 全局资产回收核心（幂等三段，命令与数据中台周期兜底共用）：
/// ① 悬垂关系行（实体已不存在的 file_relations；entity_type='library' 为资料库根/目录文件，恒合法）
/// ② 零引用资产（无关系行、无邮件附件引用 → release：物理文件+知识库+边+资产行；受宽限期保护）
/// ③ 磁盘游离文件（files/ 下 64 位 hex 命名但无任何 file_assets 记录；受宽限期保护）
/// 返回 (悬垂关系数, 回收资产数, 清理磁盘文件数)
pub(crate) fn sweep_orphans() -> (usize, usize, usize) {
    // ① 悬垂关系行
    let dangling: Vec<(String, String)> = database::all(
        "SELECT fr.id, fr.file_id FROM file_relations fr \
         WHERE fr.entity_type != 'library' AND CASE fr.entity_type \
             WHEN 'project' THEN EXISTS(SELECT 1 FROM projects p WHERE p.id=fr.entity_id) \
             WHEN 'note'    THEN EXISTS(SELECT 1 FROM notes n WHERE n.note_id=fr.entity_id) \
             WHEN 'todo'    THEN EXISTS(SELECT 1 FROM todos t WHERE t.id=fr.entity_id) \
             WHEN 'email'   THEN EXISTS(SELECT 1 FROM emails e WHERE e.id=fr.entity_id) \
             WHEN 'product' THEN EXISTS(SELECT 1 FROM products p WHERE p.id=fr.entity_id) \
             WHEN 'contact' THEN EXISTS(SELECT 1 FROM partner_contacts c WHERE c.id=fr.entity_id) \
             WHEN 'partner' THEN EXISTS(SELECT 1 FROM partners pa WHERE pa.id=fr.entity_id) \
             WHEN 'channel' THEN EXISTS(SELECT 1 FROM partners pa WHERE pa.id=fr.entity_id) \
             WHEN 'vendor'  THEN EXISTS(SELECT 1 FROM partners pa WHERE pa.id=fr.entity_id) \
             WHEN 'customer' THEN EXISTS(SELECT 1 FROM partners pa WHERE pa.id=fr.entity_id) \
             ELSE 1 END = 0", &[])
        .unwrap_or_default().iter()
        .filter_map(|r| Some((r["id"].as_str()?.to_string(), r["file_id"].as_str()?.to_string())))
        .collect();
    let affected_assets: Vec<String> = {
        let mut v: Vec<String> = dangling.iter().map(|(_, f)| f.clone()).collect();
        v.sort(); v.dedup();
        v
    };
    for (rel_id, _) in &dangling {
        database::run("DELETE FROM file_relations WHERE id=?", &[json!(rel_id)]).ok();
    }

    // ② 零引用资产（悬垂关系涉及资产 + 天然孤儿；新资产在宽限期内不回收）
    let mut candidates = affected_assets;
    candidates.extend(database::all(
        &format!("SELECT fa.id FROM file_assets fa \
         WHERE NOT EXISTS(SELECT 1 FROM file_relations fr WHERE fr.file_id=fa.id) \
           AND NOT EXISTS(SELECT 1 FROM email_attachments ea WHERE ea.file_asset_id=fa.id) \
           AND fa.created_at < datetime('now','localtime','-{} minutes')", SWEEP_GRACE_MINUTES), &[])
        .unwrap_or_default().iter().filter_map(|r| r["id"].as_str().map(String::from)));
    candidates.sort(); candidates.dedup();
    let mut assets_removed = 0usize;
    for aid in &candidates {
        let before: i64 = database::get("SELECT COUNT(*) c FROM file_assets WHERE id=?", &[json!(aid)])
            .ok().flatten().and_then(|r| r["c"].as_i64()).unwrap_or(0);
        release_asset_if_orphan(aid);
        let after: i64 = database::get("SELECT COUNT(*) c FROM file_assets WHERE id=?", &[json!(aid)])
            .ok().flatten().and_then(|r| r["c"].as_i64()).unwrap_or(0);
        if before > 0 && after == 0 { assets_removed += 1; }
    }

    // ③ 磁盘游离的 hash 文件（我们只写过 64-hex 命名；其余文件一律不碰；刚落盘的文件在宽限期内不删）
    let known: std::collections::HashSet<String> = database::all("SELECT content_hash FROM file_assets", &[])
        .unwrap_or_default().iter()
        .filter_map(|r| r["content_hash"].as_str().map(|s| s.to_lowercase()))
        .collect();
    let mut disk_removed = 0usize;
    if let Ok(rd) = std::fs::read_dir(storage_base()) {
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if name.len() != 64 || !name.bytes().all(|b| b.is_ascii_hexdigit()) { continue; }
            if known.contains(&name) { continue; }
            let fresh = entry.metadata().and_then(|m| m.modified())
                .map(|t| t.elapsed().map(|d| d.as_secs() < (SWEEP_GRACE_MINUTES * 60) as u64).unwrap_or(false))
                .unwrap_or(false);
            if fresh { continue; }
            if entry.path().is_file() && std::fs::remove_file(entry.path()).is_ok() { disk_removed += 1; }
        }
    }

    database::save().ok();
    (dangling.len(), assets_removed, disk_removed)
}

#[tauri::command]
pub async fn assets_cleanup_orphans() -> Result<Value, String> {
    let (d, a, f) = sweep_orphans();
    log::info!("[资产回收] 悬垂关系 {} 行 | 回收资产 {} 个 | 磁盘游离文件 {} 个", d, a, f);
    Ok(json!({ "success": true, "danglingRelations": d, "assetsRemoved": a, "diskFilesRemoved": f }))
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
        release_asset_if_orphan(aid);
    }

    // 4. Sync business_relations
    if let Some((_, et, eid)) = &rel_info {
        crate::services::business_relations::delete("FILE", &asset_id.unwrap_or_default(), &et.to_uppercase(), eid, "has_file");
    }
    Ok(json!({"success":true}))
}

#[tauri::command] pub async fn file_delete_from_domain(file_id: String, _entity_type: String, _entity_id: Option<String>) -> Result<Value, String> {
    // file_id here is file_relations.id (the relation record ID)
    // 1. Look up the file_assets.id from this relation
    let asset_id: Option<String> = database::get(
        "SELECT file_id FROM file_relations WHERE id=?", &[json!(&file_id)]
    ).ok().flatten().and_then(|r| r["file_id"].as_str().map(|s| s.to_string()));

    // 2. Get entity info BEFORE deleting (for business_relations cleanup)
    let rel_info: Option<(String, String)> = database::get(
        "SELECT entity_type, entity_id FROM file_relations WHERE id=?", &[json!(&file_id)]
    ).ok().flatten().map(|r| (
        r["entity_type"].as_str().unwrap_or("").to_uppercase(),
        r["entity_id"].as_str().unwrap_or("").to_string(),
    ));

    // 3. Delete the specific file_relations record
    database::run("DELETE FROM file_relations WHERE id=?", &[json!(&file_id)]).map_err(|e| e.to_string())?;

    // 4. If file_assets has no remaining relations, clean up everything
    if let Some(ref aid) = asset_id {
        release_asset_if_orphan(aid);
    }

    // 5. Clean up business_relations
    if let Some((entity_type, entity_id)) = &rel_info {
        if !entity_type.is_empty() && !entity_id.is_empty() {
            crate::services::business_relations::delete("FILE", &asset_id.unwrap_or_default(), entity_type, entity_id, "has_file");
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
    let files: i64 = database::with_db(|c| c.query_row("SELECT COUNT(*) FROM library_file_locations WHERE folder_id=?", rusqlite::params![&id], |r| r.get(0))).unwrap_or(0);
    if subs > 0 || files > 0 { return Ok(json!({"success":false,"error":"文件夹不为空，请先清空内容"})); }
    database::run("DELETE FROM library_folders WHERE id=?",&[json!(&id)]).map_err(|e| e.to_string())?;
    Ok(json!({"success":true}))
}

#[tauri::command] pub async fn file_list_by_product(product_id: String) -> Result<Vec<Value>, String> {
    database::all(
        "SELECT fr.id, fa.file_ext, fa.file_size, fa.content_hash, fr.created_at, fa.updated_at, fr.custom_name AS global_name, fr.description AS mapping_type \
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
            let base: String = if name.is_empty() { "ARK-unknown".to_string() } else {
                // 只取路径末段，防 custom_name 含 ../ 时硬链接逃逸临时目录
                std::path::Path::new(&name).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| "ARK-unknown".to_string())
            };
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

/// 粘贴文件落临时文件（供 Chat 附件粘贴场景：图片、文档等）
/// ⚠️ 大文件请改用 chat_save_temp_chunk 分片上传——bytes 走 IPC 会 JSON 序列化成数字数组，主线程开销 O(size)
#[tauri::command]
pub async fn chat_save_temp_image(bytes: Vec<u8>, ext: String, orig_name: Option<String>) -> Result<Value, String> {
    if bytes.is_empty() { return Err("文件数据为空".into()); }
    if bytes.len() > PASTE_MAX_BYTES { return Err(format!("文件超过 {}MB 上限", PASTE_MAX_BYTES / (1024 * 1024))); }
    let ext_clean = ext.trim().trim_start_matches('.').to_lowercase();
    // 只允许字母数字扩展名，防止路径注入
    let safe_ext: String = ext_clean.chars().take(10).filter(|c| c.is_ascii_alphanumeric()).collect();
    if safe_ext.is_empty() { return Err("无效的文件扩展名".into()); }
    let dir = std::env::temp_dir().join("arkmind_attach");
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建临时目录失败: {}", e))?;
    // display_name 只取路径末段（防 .. 或绝对路径经 join 逃逸临时目录）
    let uuid_name = format!("{}.{}", uuid::Uuid::new_v4(), safe_ext);
    let display_name = orig_name
        .filter(|n| !n.is_empty())
        .and_then(|n| std::path::Path::new(&n).file_name().map(|f| f.to_string_lossy().to_string()))
        .unwrap_or(uuid_name);
    let path = dir.join(&display_name);
    std::fs::write(&path, &bytes).map_err(|e| format!("写入临时文件失败: {}", e))?;
    Ok(json!({"success": true, "path": path.to_string_lossy(), "name": display_name}))
}

/// 粘贴上传大小上限（前端 pasteUpload.ts 的 MAX_PASTE_BYTES 与此保持一致）
const PASTE_MAX_BYTES: usize = 20 * 1024 * 1024;

/// 粘贴文件分片落临时文件——替代整文件 bytes IPC，单片 base64 传输让主线程不冻结，天然产生进度粒度
/// 无状态：路径由 upload_id 派生；index=0 截断创建，其余追加；末片返回 {path, name}
#[tauri::command]
pub async fn chat_save_temp_chunk(
    upload_id: String,
    data_b64: String,
    ext: String,
    orig_name: Option<String>,
    index: u32,
    total: u32,
) -> Result<Value, String> {
    if total == 0 || index >= total { return Err("分片序号无效".into()); }
    // upload_id 仅允许字母数字与连字符（uuid），防路径注入
    let id_ok = !upload_id.is_empty()
        && upload_id.len() <= 64
        && upload_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !id_ok { return Err("无效的 upload_id".into()); }
    let ext_clean = ext.trim().trim_start_matches('.').to_lowercase();
    let safe_ext: String = ext_clean.chars().take(10).filter(|c| c.is_ascii_alphanumeric()).collect();
    if safe_ext.is_empty() { return Err("无效的文件扩展名".into()); }

    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data_b64.as_bytes())
        .map_err(|e| format!("分片解码失败: {}", e))?;

    let dir = std::env::temp_dir().join("arkmind_attach");
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建临时目录失败: {}", e))?;
    let path = dir.join(format!("{}.{}", upload_id, safe_ext));

    use std::io::Write;
    if index == 0 {
        // 首片：截断创建（显示名在末片统一返回）
        let mut f = std::fs::File::create(&path).map_err(|e| format!("创建临时文件失败: {}", e))?;
        f.write_all(&bytes).map_err(|e| format!("写入分片失败: {}", e))?;
    } else {
        // 追加前检查累计大小（写入前 len = 已写字节）
        let existing = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        if existing + bytes.len() as u64 > PASTE_MAX_BYTES as u64 {
            let _ = std::fs::remove_file(&path);
            return Err(format!("文件超过 {}MB 上限", PASTE_MAX_BYTES / (1024 * 1024)));
        }
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .map_err(|e| format!("打开分片文件失败（首片未落盘？）: {}", e))?;
        f.write_all(&bytes).map_err(|e| format!("写入分片失败: {}", e))?;
    }

    if index + 1 == total {
        // 末片：返回最终路径与显示名
        let display_name = orig_name
            .filter(|n| !n.is_empty())
            .and_then(|n| std::path::Path::new(&n).file_name().map(|f| f.to_string_lossy().to_string()))
            .unwrap_or_else(|| format!("{}.{}", upload_id, safe_ext));
        // 末片同样校验总大小
        let final_size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        if final_size > PASTE_MAX_BYTES as u64 {
            let _ = std::fs::remove_file(&path);
            return Err(format!("文件超过 {}MB 上限", PASTE_MAX_BYTES / (1024 * 1024)));
        }
        Ok(json!({"success": true, "path": path.to_string_lossy(), "name": display_name}))
    } else {
        Ok(json!({"success": true, "done": false}))
    }
}

/// 中断分片上传：删除已落盘的分片临时文件（按 upload_id 派生路径，仅限 arkmind_attach 目录）
#[tauri::command]
pub async fn chat_save_temp_abort(upload_id: String, ext: String) -> Result<Value, String> {
    let id_ok = !upload_id.is_empty()
        && upload_id.len() <= 64
        && upload_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !id_ok { return Ok(json!({"success": false})); }
    let ext_clean = ext.trim().trim_start_matches('.').to_lowercase();
    let safe_ext: String = ext_clean.chars().take(10).filter(|c| c.is_ascii_alphanumeric()).collect();
    if safe_ext.is_empty() { return Ok(json!({"success": false})); }
    let dir = std::env::temp_dir().join("arkmind_attach");
    let path = dir.join(format!("{}.{}", upload_id, safe_ext));
    if let Ok(p) = path.canonicalize() {
        if p.starts_with(dir.canonicalize().unwrap_or(dir)) {
            let _ = std::fs::remove_file(p);
        }
    }
    Ok(json!({"success": true}))
}

/// 删除粘贴上传产生的临时文件（仅允许删除 arkmind_attach 目录内的文件，防路径穿越）
#[tauri::command]
pub fn file_remove_temp(path: String) -> Result<Value, String> {
    let attach_dir = std::env::temp_dir().join("arkmind_attach");
    let canonical_attach = attach_dir.canonicalize().unwrap_or(attach_dir);
    let p = std::path::Path::new(&path);
    let canonical_p = p.canonicalize().map_err(|_| "路径无效".to_string())?;
    if !canonical_p.starts_with(&canonical_attach) {
        return Ok(json!({"success":false,"error":"非临时目录文件，拒绝删除"}));
    }
    let _ = std::fs::remove_file(&canonical_p);
    Ok(json!({"success":true}))
}

/// 把源文件拷贝到临时目录（剪贴板文件粘贴用），返回 {path, name}
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn copy_to_attach_temp(src_path: &str) -> Option<serde_json::Value> {
    let bytes = std::fs::read(src_path).ok()?;
    if bytes.is_empty() { return None; }
    let name = std::path::Path::new(src_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("paste.bin")
        .to_string();
    let ext = std::path::Path::new(src_path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("bin")
        .to_string();
    let ext_clean: String = ext.chars().take(10).filter(|c| c.is_ascii_alphanumeric()).collect();
    let safe_ext = if ext_clean.is_empty() { "bin".to_string() } else { ext_clean };
    let dir = std::env::temp_dir().join("arkmind_attach");
    std::fs::create_dir_all(&dir).ok()?;
    // 防止与已有临时文件重名
    let mut display_name = name.clone();
    let mut path = dir.join(&display_name);
    let mut i = 1;
    while path.exists() {
        display_name = format!("{}_{}.{}", name.trim_end_matches(&format!(".{}", safe_ext)), i, safe_ext);
        path = dir.join(&display_name);
        i += 1;
    }
    std::fs::write(&path, &bytes).ok()?;
    Some(serde_json::json!({ "path": path.to_string_lossy(), "name": display_name, "kind": "doc" }))
}

/// 简单百分号解码（macOS file URL 路径可能含 %20 等）
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i+1..i+3]).ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok());
            if let Some(b) = hex {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

/// macOS：从 NSPasteboard 读取 Finder 复制的文件，拷贝到临时目录返回附件列表
/// 必须在主线程读取剪贴板（AppKit 限制），故通过 run_on_main_thread 同步执行
#[cfg(target_os = "macos")]
fn read_clipboard_files_impl(app: &tauri::AppHandle) -> Vec<serde_json::Value> {
    use objc2_app_kit::{NSPasteboard, NSPasteboardTypeFileURL};
    use std::sync::mpsc;

    let (tx, rx) = mpsc::channel::<Vec<serde_json::Value>>();
    let app = app.clone();
    // 主线程执行读取
    let _ = app.run_on_main_thread(move || {
        let files = objc2::rc::autoreleasepool(|_| {
            let pb = NSPasteboard::generalPasteboard();
            let mut files = Vec::new();
            if let Some(items) = pb.pasteboardItems() {
                for item in items.iter() {
                    // NSPasteboardTypeFileURL 是 extern static，需 unsafe 访问
                    let file_url_type = unsafe { NSPasteboardTypeFileURL };
                    if let Some(url) = item.stringForType(file_url_type) {
                        let s = url.to_string();
                        // file:///Users/... → /Users/...
                        let path = s.strip_prefix("file://").map(|p| p.to_string()).unwrap_or(s);
                        let decoded = percent_decode(&path);
                        if let Some(v) = copy_to_attach_temp(&decoded) {
                            files.push(v);
                        }
                    }
                }
            }
            files
        });
        let _ = tx.send(files);
    });
    rx.recv().unwrap_or_default()
}

/// Windows：从 CF_HDROP 读取资源管理器复制的文件，拷贝到临时目录返回附件列表
#[cfg(target_os = "windows")]
fn read_clipboard_files_impl(_app: &tauri::AppHandle) -> Vec<serde_json::Value> {
    use clipboard_win::{formats, get_clipboard};
    match get_clipboard::<Vec<String>, _>(formats::FileList) {
        Ok(paths) => paths.iter().filter_map(|p| copy_to_attach_temp(p)).collect(),
        Err(e) => {
            // 剪贴板不是文件列表（如纯文本/图片）时属正常，返回空即可
            log::debug!("[剪贴板] Windows 读取文件列表失败（可能非文件内容）: {}", e);
            vec![]
        }
    }
}

/// 其他平台：无系统剪贴板文件读取，返回空
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn read_clipboard_files_impl(_app: &tauri::AppHandle) -> Vec<serde_json::Value> {
    vec![]
}

/// 读取系统剪贴板中的文件（资源管理器/Finder 复制 → 粘贴），返回附件列表
#[tauri::command]
pub async fn chat_read_clipboard_files(app: tauri::AppHandle) -> Result<Value, String> {
    let files = read_clipboard_files_impl(&app);
    log::info!("[剪贴板] 读取到 {} 个文件", files.len());
    Ok(json!({ "success": true, "files": files }))
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
    // 目标必须是用户刚在保存对话框确认过的路径；源必须位于资料库 storage 目录内
    super::export_guard::ensure_registered(&save_path)?;
    let src = std::path::Path::new(&file_path);
    if src.is_relative() {
        return Ok(json!({"success":false,"error":"不支持相对路径"}));
    }
    let canonical_src = src.canonicalize().map_err(|_| "源文件不存在".to_string())?;
    let base = std::path::Path::new(&storage_base()).canonicalize().unwrap_or_else(|_| std::path::PathBuf::from(storage_base()));
    if !canonical_src.starts_with(&base) {
        return Ok(json!({"success":false,"error":"仅允许导出资料库内文件"}));
    }
    std::fs::copy(&canonical_src, &save_path).map_err(|e| format!("拷贝失败: {}", e))?;
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

/// 复制文件到系统剪贴板（资源管理器/Finder/微信等直接 Ctrl/Cmd+V 粘贴出真实文件）。
/// 实现：把 CAS 文件暂存到临时目录（还原原始文件名），再把暂存路径写入剪贴板。
#[tauri::command]
pub async fn file_copy_to_clipboard(file_id: String, app: tauri::AppHandle) -> Result<Value, String> {
    let Some((src, name, ext)) = resolve_file_info(&file_id) else {
        return Ok(json!({"success":false,"error":"文件不存在"}));
    };

    // ── 暂存：还原原始文件名（CAS 存储是哈希名，直接入剪贴板粘贴出来名字不可读） ──
    let staged = stage_for_clipboard(&src, &name, &ext)?;
    let staged_path = staged.clone();

    // ── 平台分派：macOS 需主线程；Windows 任意线程 ──
    #[cfg(target_os = "macos")]
    {
        use std::sync::mpsc;
        let (tx, rx) = mpsc::channel::<Result<(), String>>();
        let app2 = app.clone();
        let _ = app2.run_on_main_thread(move || {
            let r = crate::services::file_clipboard::copy_files_macos(&[staged_path]);
            let _ = tx.send(r);
        });
        match rx.recv().unwrap_or_else(|_| Err("剪贴板线程失败".to_string())) {
            Ok(_) => Ok(json!({"success":true,"fileName":filename_for_clipboard(&name, &ext)})),
            Err(e) => Err(e),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = &app;
        let r = crate::services::file_clipboard::set_clipboard_files(&[staged_path]);
        match r {
            Ok(_) => Ok(json!({"success":true,"fileName":filename_for_clipboard(&name, &ext)})),
            Err(e) => Err(e),
        }
    }
}

/// 复制前暂存文件到 temp 目录，文件名 = 软件显示的文件名（原始名+扩展名）。
/// 方案 B：复制前先删除旧的暂存文件，避免覆盖被其他进程占用（如剪贴板引用、
/// 杀软扫描）的文件导致复制失败。暂存文件生命周期短（下次复制或启动即替换/清空）。
fn stage_for_clipboard(src: &str, name: &str, ext: &str) -> Result<String, String> {
    let display_name = filename_for_clipboard(name, ext);

    let td = temp_dir();
    std::fs::create_dir_all(&td).map_err(|e| format!("创建暂存目录失败: {}", e))?;
    let target = std::path::Path::new(&td).join(display_name);

    // 复制前先删除旧的暂存文件（文件不存在时忽略——首次复制/已被清空）
    let mut rm_attempt = 0u32;
    loop {
        match std::fs::remove_file(&target) {
            Ok(_) => break,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
            Err(_) => {
                rm_attempt += 1;
                if rm_attempt >= 5 {
                    break; // 删除失败则继续尝试复制，复制带重试兜底
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }

    // 复制带重试（等待句柄释放）
    let mut copy_err: Option<std::io::Error> = None;
    for _ in 0..5 {
        match std::fs::copy(src, &target) {
            Ok(_) => {
                copy_err = None;
                break;
            }
            Err(e) => {
                copy_err = Some(e);
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }
    match copy_err {
        Some(e) => Err(format!("暂存文件失败: {}", e)),
        None => Ok(target.to_string_lossy().to_string()),
    }
}

fn filename_for_clipboard(name: &str, ext: &str) -> String {
    if name.is_empty() {
        return "download".to_string();
    }
    if ext.is_empty() || name.to_lowercase().ends_with(&format!(".{}", ext.to_lowercase())) {
        name.to_string()
    } else {
        format!("{}.{}", name, ext)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filename_for_clipboard_keeps_extension_when_missing() {
        assert_eq!(filename_for_clipboard("方案", "docx"), "方案.docx");
        assert_eq!(filename_for_clipboard("a", "pdf"), "a.pdf");
    }

    #[test]
    fn test_filename_for_clipboard_preserves_existing_extension() {
        assert_eq!(filename_for_clipboard("方案.docx", "docx"), "方案.docx");
        assert_eq!(filename_for_clipboard("report.PDF", "pdf"), "report.PDF");
    }

    #[test]
    fn test_filename_for_clipboard_empty_fallback() {
        assert_eq!(filename_for_clipboard("", "xlsx"), "download");
        assert_eq!(filename_for_clipboard("", ""), "download");
        assert_eq!(filename_for_clipboard("名字", ""), "名字");
    }

    #[test]
    fn test_stage_for_clipboard_stages_with_display_name() {
        let dir = tempfile::tempdir().unwrap();
        // 用临时文件模拟 CAS 源文件
        let src = dir.path().join("abcdef1234567890");
        std::fs::write(&src, b"content").unwrap();
        let staged = stage_for_clipboard(&src.to_string_lossy(), "测试.docx", "docx").unwrap();
        let path = std::path::Path::new(&staged);
        assert!(path.exists(), "暂存文件必须存在");
        let fname = path.file_name().unwrap().to_string_lossy().to_string();
        assert_eq!(fname, "测试.docx", "暂存文件名必须等于软件显示的文件名（无前缀）: {}", fname);
        assert_eq!(std::fs::read(&staged).unwrap(), b"content", "暂存内容必须一致");
        std::fs::remove_file(&staged).ok();
    }

    #[test]
    fn test_stage_for_clipboard_overwrites_same_name() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("abcdef");
        std::fs::write(&src, b"v1").unwrap();
        let staged = stage_for_clipboard(&src.to_string_lossy(), "同名.docx", "docx").unwrap();
        // 同名字覆盖：写入不同内容，再次暂存应覆盖
        std::fs::write(&src, b"v2").unwrap();
        let staged2 = stage_for_clipboard(&src.to_string_lossy(), "同名.docx", "docx").unwrap();
        assert_eq!(staged, staged2, "同名文件应覆盖到同一路径");
        assert_eq!(std::fs::read(&staged2).unwrap(), b"v2", "覆盖后内容应为最新");
        std::fs::remove_file(&staged2).ok();
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
#[tauri::command] pub async fn file_move(data: Value) -> Result<Value, String> {
    let file_id = data["fileId"].as_str().unwrap_or("");
    let folder_id: Option<String> = data.get("toFolderId").and_then(|v| v.as_str().map(|s| s.to_string()));
    if file_id.is_empty() { return Ok(json!({"success":false})); }
    let asset_id = resolve_file_asset_id(file_id).unwrap_or_else(|| file_id.to_string());
    database::run("INSERT OR REPLACE INTO library_file_locations(file_id, folder_id) VALUES(?1,?2)", &[json!(&asset_id), json!(folder_id)]).ok();
    Ok(json!({"success":true}))
}
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
