use crate::database;
use serde_json::{json, Value};

#[tauri::command] pub async fn project_list() -> Result<Vec<Value>, String> { database::all("SELECT p.*, pr.name as customer_name FROM projects p LEFT JOIN (SELECT partner_id, project_id FROM partner_project_mapping WHERE partner_type = 'CUSTOMER' UNION SELECT pcm.partner_id, cpm.project_id FROM contact_project_mapping cpm INNER JOIN partner_contact_mapping pcm ON cpm.contact_id = pcm.contact_id INNER JOIN partners pt ON pcm.partner_id = pt.id AND pt.type = 'CUSTOMER') ppm ON ppm.project_id = p.id LEFT JOIN partners pr ON ppm.partner_id = pr.id ORDER BY p.updated_at DESC", &[]).map_err(|e| e.to_string()) }
#[tauri::command] pub async fn project_get(id: String) -> Result<Value, String> { database::get("SELECT p.*, pr.name as customer_name FROM projects p LEFT JOIN (SELECT partner_id, project_id FROM partner_project_mapping WHERE partner_type = 'CUSTOMER' UNION SELECT pcm.partner_id, cpm.project_id FROM contact_project_mapping cpm INNER JOIN partner_contact_mapping pcm ON cpm.contact_id = pcm.contact_id INNER JOIN partners pt ON pcm.partner_id = pt.id AND pt.type = 'CUSTOMER') ppm ON ppm.project_id = p.id LEFT JOIN partners pr ON ppm.partner_id = pr.id WHERE p.id=?", &[json!(id)]).map_err(|e| e.to_string()).map(|r| r.unwrap_or(Value::Null)) }
#[tauri::command]
pub async fn project_create(data: Value) -> Result<Value, String> {
    let id = data["displayId"].as_str().filter(|s| !s.is_empty()).or_else(|| data["id"].as_str().filter(|s| !s.is_empty())).unwrap_or("").to_string();
    let id = if id.is_empty() { uuid::Uuid::new_v4().to_string() } else { id };
    let name = data["name"].as_str().unwrap_or("新项目").to_string();
    let desc = data["description"].as_str().unwrap_or("").to_string();
    let status = data["status"].as_str().unwrap_or("Lead");
    let bid_time = if status == "Bidding" { data["bidTime"].as_str().unwrap_or("") } else { "" };
    let budget_val = data["budget"].as_f64()
        .or_else(|| data["budget"].as_str().and_then(|s| s.parse::<f64>().ok()))
        .unwrap_or(0.0);
    let expected_order_date = data["expectedOrderDate"].as_str().unwrap_or("");
    database::run("INSERT INTO projects(id,name,status,description,budget,owner,user_id,bid_time,expected_order_date) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
    &[json!(&id),json!(&name),json!(status),json!(&desc),json!(budget_val),json!(data["owner"].as_str().unwrap_or("")),json!(data["userId"].as_str().unwrap_or("")),json!(bid_time),json!(expected_order_date)]).map_err(|e| e.to_string())?;
    crate::services::knowledge_lifecycle::notify_changed("PROJECT", &id);
    database::get("SELECT * FROM projects WHERE id=?", &[json!(id)]).map_err(|e| e.to_string()).map(|r| r.unwrap_or(Value::Null))
}
#[tauri::command] pub async fn project_update(id: String, data: Value) -> Result<(), String> {
    let mut fields: Vec<String> = Vec::new(); let mut values: Vec<Value> = Vec::new();
    // 获取当前项目状态（优先用 data 中的，否则从数据库查）
    let current_status = data["status"].as_str()
        .map(String::from)
        .unwrap_or_else(|| {
            database::get("SELECT status FROM projects WHERE id=?", &[json!(&id)])
                .ok().flatten()
                .and_then(|r| r["status"].as_str().map(String::from))
                .unwrap_or_else(|| "Lead".to_string())
        });
    for (k, col) in &[
        ("name","name"),("status","status"),("owner","owner"),
        ("budget","budget"),("description","description"),
    ] { if let Some(s) = data[*k].as_str() { fields.push(format!("{}=?",col)); values.push(json!(s)); } }
    // bid_time 只在投标状态下有效
    let bid_time_val = if current_status == "Bidding" {
        data["bid_time"].as_str().or_else(|| data["bidTime"].as_str()).unwrap_or("").to_string()
    } else {
        String::new()
    };
    if !bid_time_val.is_empty() { fields.push("bid_time=?".into()); values.push(json!(bid_time_val)); }
    // expected_order_date 预计下单时间
    if let Some(v) = data["expectedOrderDate"].as_str() { fields.push("expected_order_date=?".into()); values.push(json!(v)); }
    if let Some(_v) = data["budget"].as_f64() { /* already handled via str in loop above */ }
    // 回退：如果 budget 以数字类型传入，补充处理
    if !fields.iter().any(|f| f.starts_with("budget")) {
        if let Some(v) = data["budget"].as_f64() {
            fields.push("budget=?".into()); values.push(json!(v));
        }
    }
    if fields.is_empty() { return Ok(()); }
    fields.push("updated_at=datetime('now', 'localtime')".into()); values.push(json!(id));
    let result = database::run(&format!("UPDATE projects SET {} WHERE id=?", fields.join(",")), &values).map_err(|e| e.to_string())?;
    if result.changes == 0 { return Err(format!("未找到项目: {}", id)); }
    crate::services::knowledge_lifecycle::notify_changed("PROJECT", &id);
    Ok(())
}
#[tauri::command] pub async fn project_delete(id: String) -> Result<Value, String> {
    crate::services::knowledge_lifecycle::notify_deleted("PROJECT", &id);
    let result = database::run("DELETE FROM projects WHERE id=?", &[json!(&id)]).map_err(|e| e.to_string())?;
    if result.changes == 0 { return Err(format!("未找到项目: {}", id)); }
    database::run("DELETE FROM note_project_mapping WHERE project_id=?", &[json!(&id)]).ok();
    database::run("DELETE FROM project_dynamics WHERE project_id=?", &[json!(&id)]).ok();
    database::run("DELETE FROM partner_project_mapping WHERE project_id=?", &[json!(&id)]).ok();
    database::run("DELETE FROM contact_project_mapping WHERE project_id=?", &[json!(&id)]).ok();
    // M1: 清理项目附件 file_relations（项目文件成为孤儿）
    database::run("DELETE FROM file_relations WHERE entity_type='project' AND entity_id=?", &[json!(&id)]).ok();
    // M1: 清理项目目录（library_folders 成为孤儿）
    database::run("DELETE FROM library_folders WHERE project_id=?", &[json!(&id)]).ok();
    // 清理解决方案图谱边（has_version/uses/instance_of 以方案/版本/设备为源，PROJECT 源边由下方统一清理）
    database::run("DELETE FROM business_relations WHERE relation_type IN ('has_version','uses','instance_of') AND ((source_type='SOLUTION' AND source_id IN (SELECT id FROM solutions WHERE project_id=?1)) OR (source_type='SOLUTION_VERSION' AND source_id IN (SELECT sv.id FROM solution_versions sv JOIN solutions s ON s.id=sv.solution_id WHERE s.project_id=?1)) OR (source_type='SOLUTION_ITEM' AND source_id IN (SELECT si.id FROM solution_items si JOIN solution_versions sv ON sv.id=si.solution_version_id JOIN solutions s ON s.id=sv.solution_id WHERE s.project_id=?1)))", &[json!(&id)]).ok();
    database::run("DELETE FROM solution_items WHERE solution_version_id IN (SELECT id FROM solution_versions WHERE solution_id IN (SELECT id FROM solutions WHERE project_id=?))", &[json!(&id)]).ok();
    database::run("DELETE FROM solution_versions WHERE solution_id IN (SELECT id FROM solutions WHERE project_id=?)", &[json!(&id)]).ok();
    database::run("DELETE FROM solutions WHERE project_id=?", &[json!(&id)]).ok();
    // 清理 business_relations 表中与该项目相关的关联
    database::run("DELETE FROM business_relations WHERE (source_type='PROJECT' AND source_id=?) OR (target_type='PROJECT' AND target_id=?)", &[json!(&id), json!(&id)]).ok();
    database::save().map_err(|e| e.to_string())?;
    Ok(json!({"success":true}))
}
/// 项目"最后活跃时间"收口：任何项目维度的活动（日志/文件/笔记/待办/方案/合作伙伴关联）都调用本函数
/// 只更新 projects.updated_at，不触碰知识库层（source_data）——知识库刷新由 notify_changed 独立负责
pub fn touch_project(project_id: &str) {
    if project_id.is_empty() { return; }
    database::run("UPDATE projects SET updated_at=datetime('now','localtime') WHERE id=?", &[json!(project_id)]).ok();
}

#[tauri::command] pub async fn project_dynamics_add(project_id: String, event: Value) -> Result<Value, String> {
    let id = uuid::Uuid::new_v4().to_string(); let ts = crate::time_utils::now_db();
    let content = event["content"].as_str().unwrap_or("").to_string();
    database::run("INSERT INTO project_dynamics(id,project_id,type,content,operator,created_at) VALUES(?1,?2,?3,?4,?5,?6)", &[json!(id),json!(project_id),json!(event["type"].as_str().unwrap_or("note")),json!(&content),json!(event["operator"].as_str().unwrap_or("")),json!(ts)]).map_err(|e| e.to_string())?;
    touch_project(&project_id);
    database::get("SELECT * FROM project_dynamics WHERE id=?", &[json!(id)]).map_err(|e| e.to_string()).map(|r| r.unwrap_or(Value::Null))
}
#[tauri::command] pub async fn project_dynamics_list(project_id: String) -> Result<Vec<Value>, String> { database::all("SELECT * FROM project_dynamics WHERE project_id=? ORDER BY created_at DESC", &[json!(project_id)]).map_err(|e| e.to_string()) }
#[tauri::command] pub async fn project_dynamics_list_all() -> Result<Vec<Value>, String> { database::all("SELECT pd.id, pd.project_id, pd.type, pd.content, pd.operator, pd.created_at, p.name as project_name FROM project_dynamics pd JOIN projects p ON p.id=pd.project_id ORDER BY pd.created_at DESC LIMIT 30", &[]).map_err(|e| e.to_string()) }
#[tauri::command] pub async fn project_dynamics_delete(dynamic_id: String) -> Result<(), String> {
    if let Ok(Some(r)) = database::get("SELECT project_id FROM project_dynamics WHERE id=?1",&[json!(&dynamic_id)]) {
        database::run("DELETE FROM project_dynamics WHERE id=? AND type IN ('note','ai')", &[json!(dynamic_id)]).map_err(|e| e.to_string())?;
        if let Some(pid) = r["project_id"].as_str() {
            let _ = pid;
        }
    }
    Ok(())
}
#[tauri::command] pub async fn project_dynamics_edit(dynamic_id: String, content: String) -> Result<(), String> {
    if let Ok(Some(r)) = database::get("SELECT project_id FROM project_dynamics WHERE id=?1",&[json!(&dynamic_id)]) {
        database::run("UPDATE project_dynamics SET content=? WHERE id=? AND type IN ('note','ai')", &[json!(content), json!(dynamic_id)]).map_err(|e| e.to_string())?;
        if let Some(pid) = r["project_id"].as_str() {
            let _ = pid;
        }
    }
    Ok(())
}
#[tauri::command] pub async fn project_update_status(project_id: String, new_status: String, operator: String) -> Result<(), String> {
    // won_at：进入 Won 记录中标时间，离开 Won 清空（updated_at 已改为"最后活跃时间"语义，不能当中标时间用）
    let won_clause = if new_status == "Won" { ",won_at=datetime('now','localtime')" } else { ",won_at=''" };
    database::run(&format!("UPDATE projects SET status=?,updated_at=datetime('now', 'localtime'){} WHERE id=?", won_clause), &[json!(new_status),json!(project_id)]).map_err(|e| e.to_string())?;
    let id = uuid::Uuid::new_v4().to_string();
    database::run("INSERT INTO project_dynamics(id,project_id,type,content,operator) VALUES(?1,?2,'system',?3,?4)", &[json!(id),json!(project_id),json!(format!("项目状态更新为：{}",new_status)),json!(operator)]).ok();
    crate::services::knowledge_lifecycle::notify_changed("PROJECT", &project_id);
    Ok(())
}
#[tauri::command] pub async fn project_archive(project_id: String, _operator: String) -> Result<(), String> { database::run("UPDATE projects SET status='Archived',updated_at=datetime('now', 'localtime') WHERE id=?", &[json!(project_id)]).map_err(|e| e.to_string())?; crate::services::knowledge_lifecycle::notify_changed("PROJECT", &project_id); Ok(()) }

async fn project_export_inner(project_id: &str) -> Result<Value, String> {
    use std::io::Write;
    let p = database::get("SELECT * FROM projects WHERE id=?",&[json!(&project_id)]).map_err(|e| e.to_string())?.ok_or("项目未找到")?;
    let name = p["name"].as_str().unwrap_or("unnamed");
    // Collect all related data
    let solutions: Vec<Value> = {
        let mut result = Vec::new();
        let sols = database::all("SELECT * FROM solutions WHERE project_id=? ORDER BY created_at DESC",&[json!(&project_id)]).unwrap_or_default();
        for s in &sols {
            let sid = s["id"].as_str().unwrap_or("");
            let svs = database::all("SELECT * FROM solution_versions WHERE solution_id=? ORDER BY created_at ASC",&[json!(sid)]).unwrap_or_default();
            let mut versions = Vec::new();
            for v in &svs {
                let vid = v["id"].as_str().unwrap_or("");
                let items = database::all("SELECT * FROM solution_items WHERE solution_version_id=?",&[json!(vid)]).unwrap_or_default();
                let mut vv = v.clone();
                vv.as_object_mut().map(|m| { m.insert("items".into(), json!(items)); });
                versions.push(vv);
            }
            let mut ss = s.clone();
            ss.as_object_mut().map(|m| { m.insert("versions".into(), json!(versions)); });
            result.push(ss);
        }
        result
    };
    let partners = database::all(
        "SELECT DISTINCT p.* FROM partners p \
         JOIN partner_project_mapping ppm ON p.id = ppm.partner_id \
         WHERE ppm.project_id = ?",
        &[json!(&project_id)]
    ).unwrap_or_default();
    let partner_project_mappings = database::all(
        "SELECT * FROM partner_project_mapping WHERE project_id = ?",
        &[json!(&project_id)]
    ).unwrap_or_default();
    let partner_contacts = database::all(
        "SELECT DISTINCT pc.* FROM partner_contacts pc \
         LEFT JOIN partner_contact_mapping pcm ON pc.id = pcm.contact_id \
         LEFT JOIN contact_project_mapping cpm ON pc.id = cpm.contact_id \
         WHERE pcm.partner_id IN (SELECT ppm.partner_id FROM partner_project_mapping ppm WHERE ppm.project_id = ?) \
            OR cpm.project_id = ?",
        &[json!(&project_id), json!(&project_id)]
    ).unwrap_or_default();
    let partner_contact_mappings = database::all(
        "SELECT DISTINCT pcm.* FROM partner_contact_mapping pcm \
         INNER JOIN partner_project_mapping ppm ON pcm.partner_id = ppm.partner_id \
         WHERE ppm.project_id = ?",
        &[json!(&project_id)]
    ).unwrap_or_default();
    let contact_project_mappings = database::all(
        "SELECT * FROM contact_project_mapping WHERE project_id = ?",
        &[json!(&project_id)]
    ).unwrap_or_default();
    let relations = database::all(
        "SELECT * FROM business_relations WHERE target_type = 'PROJECT' AND target_id = ?",
        &[json!(&project_id)]
    ).unwrap_or_default();
    let dynamics = database::all("SELECT * FROM project_dynamics WHERE project_id=? ORDER BY created_at ASC",&[json!(&project_id)]).unwrap_or_default();
    let todos_mappings = database::all("SELECT * FROM todo_project_mapping WHERE project_id=?",&[json!(&project_id)]).unwrap_or_default();
    let todo_ids: Vec<String> = todos_mappings.iter().filter_map(|r| r["todo_id"].as_str().map(String::from)).collect();
    let todos = if todo_ids.is_empty() { vec![] } else {
        let ph: Vec<String> = todo_ids.iter().map(|_| "?".to_string()).collect();
        let params: Vec<Value> = todo_ids.iter().map(|id| json!(id)).collect();
        database::all(&format!("SELECT * FROM todos WHERE id IN ({})", ph.join(",")), &params).unwrap_or_default()
    };
    let notes_mappings = database::all("SELECT * FROM note_project_mapping WHERE project_id=?",&[json!(&project_id)]).unwrap_or_default();
    let note_ids: Vec<String> = notes_mappings.iter().filter_map(|r| r["note_id"].as_str().map(String::from)).collect();
    let notes = if note_ids.is_empty() { vec![] } else {
        let ph: Vec<String> = note_ids.iter().map(|_| "?".to_string()).collect();
        let params: Vec<Value> = note_ids.iter().map(|id| json!(id)).collect();
        database::all(&format!("SELECT * FROM notes WHERE note_id IN ({})", ph.join(",")), &params).unwrap_or_default()
    };
    let file_relations = database::all("SELECT fr.*, fa.file_ext, fa.file_size, fa.content_hash FROM file_relations fr JOIN file_assets fa ON fa.id=fr.file_id WHERE fr.entity_type='project' AND fr.entity_id=?",&[json!(&project_id)]).unwrap_or_default();
    let folders = database::all("SELECT * FROM library_folders WHERE project_id=?",&[json!(&project_id)]).unwrap_or_default();
    let manifest = json!({
        "version":2,"name":name,"project":p,"solutions":solutions,
        "partners":partners,"partner_project_mappings":partner_project_mappings,
        "partner_contacts":partner_contacts,"partner_contact_mappings":partner_contact_mappings,
        "contact_project_mappings":contact_project_mappings,
        "business_relations":relations,
        "dynamics":dynamics,
        "todos":todos,"todo_mappings":todos_mappings,
        "notes":notes,"note_mappings":notes_mappings,
        "file_relations":file_relations,"library_folders":folders,
    });
    // Write to temp dir, create zip
    let tmp = std::env::temp_dir().join(format!("ark_export_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let files_dir = tmp.join("files"); std::fs::create_dir_all(&files_dir).map_err(|e| e.to_string())?;
    // Build folder lookup map
    let mut folder_map = std::collections::HashMap::new();
    for f in &folders {
        if let (Some(id), Some(name)) = (f["id"].as_str(), f["name"].as_str().filter(|n| !n.is_empty())) {
            folder_map.insert(id.to_string(), name.to_string());
        }
    }
    // Write manifest.json
    let manifest_path = tmp.join("manifest.json");
    let mut mf = std::fs::File::create(&manifest_path).map_err(|e| e.to_string())?;
    mf.write_all(serde_json::to_string_pretty(&manifest).unwrap_or_default().as_bytes()).map_err(|e| e.to_string())?;
    // Copy files
    let mut copied = 0usize; let mut skipped = 0usize;
    for fr in &file_relations {
        if let Some(hash) = fr["content_hash"].as_str().filter(|h| !h.is_empty()) {
            let src = crate::commands::file_commands::flat_disk_path(hash);
            if std::path::Path::new(&src).exists() {
                let ext = fr["file_ext"].as_str().unwrap_or("bin");
                let fid = fr["file_id"].as_str().unwrap_or("unknown");
                let dest = files_dir.join(format!("{}.{}", fid, ext));
                std::fs::copy(&src, &dest).ok();
                copied += 1;
            } else { skipped += 1; eprintln!("[export] 文件缺失: {}", src); }
        }
    }
    eprintln!("[export] 文件复制: {} 成功, {} 缺失", copied, skipped);
    // Create zip archive
    let ts = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
    let safe_name: String = name.chars().filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_').take(30).collect();
    let archive_name = format!("{}_{}.ark", safe_name, ts);
    let archive_path = std::env::temp_dir().join(&archive_name);
    let file = std::fs::File::create(&archive_path).map_err(|e| e.to_string())?;
    let mut zip_writer = zip::ZipWriter::new(file);
    let opts = zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    fn add_dir_to_zip(w: &mut zip::ZipWriter<std::fs::File>, base: &std::path::Path, dir: &std::path::Path, opts: zip::write::FileOptions) -> Result<(), String> {
        for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            let name = path.strip_prefix(base).unwrap_or(&path).to_string_lossy().replace('\\', "/");
            if path.is_dir() {
                w.add_directory(name, opts).map_err(|e| e.to_string())?;
                add_dir_to_zip(w, base, &path, opts)?;
            } else {
                w.start_file(name, opts).map_err(|e| e.to_string())?;
                let mut f = std::fs::File::open(&path).map_err(|e| e.to_string())?;
                std::io::copy(&mut f, w).map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }
    add_dir_to_zip(&mut zip_writer, &tmp, &tmp, opts)?;
    zip_writer.finish().map_err(|e| e.to_string())?;
    // Cleanup temp
    std::fs::remove_dir_all(&tmp).ok();
    Ok(json!({"success":true,"exportPath":archive_path.to_string_lossy().to_string(),"fileName":archive_name}))
}

#[tauri::command] pub async fn project_import(data: Value) -> Result<Value, String> {
    let file_path = data["filePath"].as_str().ok_or("缺少文件路径")?;
    eprintln!("[项目导入] zip导入: path={}, exists={}", file_path, std::path::Path::new(file_path).exists());
    let file = std::fs::File::open(file_path).map_err(|e| format!("无法打开文件: {} ({})", file_path, e))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let tmp = std::env::temp_dir().join(format!("ark_import_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    archive.extract(&tmp).map_err(|e| e.to_string())?;
    let manifest_path = tmp.join("manifest.json");
    if !manifest_path.exists() {
        // 批量导出 zip，遍历其中的 .ark 文件分别导入
        let mut imported = 0;
        for entry in std::fs::read_dir(&tmp).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            if path.extension().map(|e| e == "ark").unwrap_or(false) {
                if let Some(sub_path) = path.to_str().map(String::from) {
                    eprintln!("[项目导入] 批量导入子文件: {}", sub_path);
                    match project_import_inner(&sub_path) {
                        Ok(_) => { imported += 1; eprintln!("[项目导入] 子文件导入成功"); },
                        Err(e) => eprintln!("[项目导入] 子文件导入失败: {}", e),
                    }
                }
            }
        }
        eprintln!("[项目导入] 批量导入完成: {} 个项目", imported);
        return Ok(json!({"success":true}));
    }
    let manifest: Value = serde_json::from_reader(std::fs::File::open(&manifest_path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let sols = manifest["solutions"].as_array().map(|a| a.len()).unwrap_or(0);
    eprintln!("[项目导入] zip manifest 中有 {} 个方案", sols);
    // Import project
    if let Some(proj) = manifest["project"].as_object() {
        let proj = proj.clone();
        let pid = proj["id"].as_str().unwrap_or("").to_string();
        if pid.is_empty() { return Err("项目ID缺失".into()); }
        // Insert project
        let cols = ["id","name","description","status","owner","budget","bid_time","expected_order_date","won_at","created_at","updated_at"];
        let mut vals: Vec<Value> = Vec::new();
        let mut placeholders: Vec<String> = Vec::new();
        for c in &cols {
            vals.push(proj.get(*c).cloned().unwrap_or(json!("")));
            placeholders.push("?".to_string());
        }
        database::run(&format!("INSERT OR REPLACE INTO projects({}) VALUES({})",cols.join(","),placeholders.join(",")),&vals).map_err(|e| e.to_string())?;
        // Import solutions
        if let Some(arr) = manifest["solutions"].as_array() {
            for sol in arr {
                database::run("INSERT OR REPLACE INTO solutions(id,project_id,name,created_at,updated_at) VALUES(?1,?2,?3,?4,?5)",&[
                    sol["id"].clone(),json!(&pid),sol["name"].clone(),sol["created_at"].clone(),sol["updated_at"].clone()
                ]).ok();
                if let Some(vers) = sol["versions"].as_array() {
                    for v in vers {
                        if let Err(e) = database::run("INSERT OR REPLACE INTO solution_versions(id,solution_id,version,tags,content,creator,status,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",&[
                            v["id"].clone(),sol["id"].clone(),v["version"].clone(),v["tags"].clone(),v["content"].clone(),v["creator"].as_str().map(|s| json!(s)).unwrap_or(json!("")),v["status"].clone(),v["created_at"].clone(),v["updated_at"].clone()
                        ]) {
                            eprintln!("[导入] 版本插入失败: {}", e);
                        }
                        if let Some(items) = v["items"].as_array() {
                            for item in items {
                                if let Err(e) = database::run("INSERT OR REPLACE INTO solution_items(id,solution_version_id,product_id,product_name,device_name,quantity,unit_price,description,row_type,sort_order,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",&[
                                    item["id"].clone(),v["id"].clone(),item["product_id"].as_str().map(|s| json!(s)).unwrap_or(item["productId"].as_str().map(|s| json!(s)).unwrap_or(json!(""))),item["product_name"].as_str().map(|s| json!(s)).unwrap_or(json!("")),item["device_name"].as_str().map(|s| json!(s)).unwrap_or(json!("")),item["quantity"].clone(),item["unit_price"].as_f64().map(|v| json!(v)).unwrap_or(json!(0)),item["description"].as_str().map(|s| json!(s)).unwrap_or(json!("")),item["row_type"].as_str().map(|s| json!(s)).unwrap_or(json!("device")),item["sort_order"].as_i64().map(|v| json!(v)).unwrap_or(json!(0)),item["created_at"].clone()
                                ]) {
                                    eprintln!("[导入] 设备项插入失败: {}", e);
                                }
                            }
                        }
                    }
                }
            }
        }
        // Import partners (new or legacy format)
        if let Some(arr) = manifest["partners"].as_array() {
            for p in arr {
                database::run("INSERT OR IGNORE INTO partners(id,type,name,phone,company,remark,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",&[
                    p["id"].clone(),p["type"].clone(),p["name"].clone(),
                    p["phone"].as_str().map(|s| json!(s)).unwrap_or(json!("")),
                    p["company"].as_str().map(|s| json!(s)).unwrap_or(json!("")),
                    p["remark"].as_str().map(|s| json!(s)).unwrap_or(json!("")),
                    p["created_at"].clone(),p["updated_at"].as_str().map(|s| json!(s)).unwrap_or(p["created_at"].clone())
                ]).ok();
            }
        } else {
            // Legacy format: convert channel_partners → partners
            if let Some(arr) = manifest["channel_partners"].as_array() {
                for ch in arr {
                    database::run("INSERT OR IGNORE INTO partners(id,type,name,phone,company,remark,created_at,updated_at) VALUES(?1,'CHANNEL',?2,?3,?4,?5,?6,?6)",&[
                        ch["id"].clone(),ch["name"].clone(),
                        ch["phone"].as_str().map(|s| json!(s)).unwrap_or(json!("")),
                        ch["company"].as_str().map(|s| json!(s)).unwrap_or(json!("")),
                        ch["remark"].as_str().map(|s| json!(s)).unwrap_or(json!("")),
                        ch["created_at"].clone()
                    ]).ok();
                    crate::services::business_relations::upsert("CHANNEL", ch["id"].as_str().unwrap_or(""), "PROJECT", &pid, "belongs_to", "{}");
                    if let Some(contacts) = ch["contacts"].as_array() {
                        for ct in contacts {
                            let ct_id = ct["id"].as_str().unwrap_or("");
                            database::run("INSERT OR IGNORE INTO partner_contacts(id,name,phone,email,position,remark,created_at) VALUES(?1,?2,?3,?4,'',?5,?6)",&[
                                ct["id"].clone(),ct["name"].clone(),
                                ct["phone"].as_str().map(|s| json!(s)).unwrap_or(json!("")),
                                ct["email"].as_str().map(|s| json!(s)).unwrap_or(json!("")),
                                ct["remark"].as_str().map(|s| json!(s)).unwrap_or(json!("")),
                                ct["created_at"].clone()
                            ]).ok();
                            database::run("INSERT OR IGNORE INTO partner_contact_mapping(partner_id,contact_id) VALUES(?1,?2)",&[
                                ch["id"].clone(),json!(ct_id)
                            ]).ok();
                        }
                    }
                }
            }
            // Legacy format: convert vendor_partners → partners
            if let Some(arr) = manifest["vendor_partners"].as_array() {
                for vp in arr {
                    database::run("INSERT OR IGNORE INTO partners(id,type,name,phone,company,remark,created_at,updated_at) VALUES(?1,'VENDOR',?2,?3,?4,?5,?6,?6)",&[
                        vp["id"].clone(),vp["name"].clone(),
                        vp["phone"].as_str().map(|s| json!(s)).unwrap_or(json!("")),
                        vp["company"].as_str().map(|s| json!(s)).unwrap_or(json!("")),
                        vp["remark"].as_str().map(|s| json!(s)).unwrap_or(json!("")),
                        vp["created_at"].clone()
                    ]).ok();
                    crate::services::business_relations::upsert("VENDOR", vp["id"].as_str().unwrap_or(""), "PROJECT", &pid, "belongs_to", "{}");
                    if let Some(contacts) = vp["contacts"].as_array() {
                        for ct in contacts {
                            let ct_id = ct["id"].as_str().unwrap_or("");
                            database::run("INSERT OR IGNORE INTO partner_contacts(id,name,phone,email,position,remark,created_at) VALUES(?1,?2,?3,?4,'',?5,?6)",&[
                                ct["id"].clone(),ct["name"].clone(),
                                ct["phone"].as_str().map(|s| json!(s)).unwrap_or(json!("")),
                                ct["email"].as_str().map(|s| json!(s)).unwrap_or(json!("")),
                                ct["remark"].as_str().map(|s| json!(s)).unwrap_or(json!("")),
                                ct["created_at"].clone()
                            ]).ok();
                            database::run("INSERT OR IGNORE INTO partner_contact_mapping(partner_id,contact_id) VALUES(?1,?2)",&[
                                vp["id"].clone(),json!(ct_id)
                            ]).ok();
                        }
                    }
                }
            }
        }
        // Import partner contacts (new format)
        if let Some(arr) = manifest["partner_contacts"].as_array() {
            for pc in arr {
                database::run("INSERT OR IGNORE INTO partner_contacts(id,name,phone,email,position,remark,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",&[
                    pc["id"].clone(),pc["name"].clone(),
                    pc["phone"].as_str().map(|s| json!(s)).unwrap_or(json!("")),
                    pc["email"].as_str().map(|s| json!(s)).unwrap_or(json!("")),
                    pc["position"].as_str().map(|s| json!(s)).unwrap_or(json!("")),
                    pc["remark"].as_str().map(|s| json!(s)).unwrap_or(json!("")),
                    pc["created_at"].clone()
                ]).ok();
            }
        }
        if let Some(arr) = manifest["partner_contact_mapping"].as_array() {
            for m in arr {
                database::run("INSERT OR IGNORE INTO partner_contact_mapping(partner_id,contact_id) VALUES(?1,?2)",&[
                    m["partner_id"].clone(),m["contact_id"].clone()
                ]).ok();
            }
        }
        // Import business relations (new format)
        if let Some(arr) = manifest["business_relations"].as_array() {
            for br in arr {
                crate::services::business_relations::upsert(
                    br["source_type"].as_str().unwrap_or(""),
                    br["source_id"].as_str().unwrap_or(""),
                    br["target_type"].as_str().unwrap_or(""),
                    br["target_id"].as_str().unwrap_or(""),
                    br["relation_type"].as_str().unwrap_or(""),
                    br["metadata"].as_str().unwrap_or("{}"),
                );
            }
        }
        // Import dynamics
        if let Some(arr) = manifest["dynamics"].as_array() {
            for d in arr {
                database::run("INSERT OR REPLACE INTO project_dynamics(id,project_id,type,content,operator,created_at) VALUES(?1,?2,?3,?4,?5,?6)",&[
                    d["id"].clone(),json!(&pid),d["type"].clone(),d["content"].clone(),d["operator"].clone(),d["created_at"].clone()
                ]).ok();
            }
        }
        // Import todos
        if let Some(arr) = manifest["todos"].as_array() {
            for t in arr {
                database::run("INSERT OR REPLACE INTO todos(id,title,content,done,priority,due_date,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",&[
                    t["id"].clone(),t["title"].clone(),t["content"].clone(),t["done"].clone(),t["priority"].clone(),t["due_date"].clone(),t["created_at"].clone(),t["updated_at"].clone()
                ]).ok();
            }
        }
        if let Some(arr) = manifest["todo_mappings"].as_array() {
            for m in arr {
                database::run("INSERT OR REPLACE INTO todo_project_mapping(todo_id,project_id,created_at) VALUES(?1,?2,?3)",&[
                    m["todo_id"].clone(),json!(&pid),m["created_at"].clone()
                ]).ok();
            }
        }
        // Import notes
        if let Some(arr) = manifest["notes"].as_array() {
            for n in arr {
                database::run("INSERT OR REPLACE INTO notes(note_id,title,content,is_daily,notebook_id,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",&[
                    n["note_id"].clone(),n["title"].clone(),n["content"].clone(),n["is_daily"].clone(),n["notebook_id"].clone(),n["created_at"].clone(),n["updated_at"].clone()
                ]).ok();
            }
        }
        if let Some(arr) = manifest["note_mappings"].as_array() {
            for m in arr {
                database::run("INSERT OR REPLACE INTO note_project_mapping(note_id,project_id) VALUES(?1,?2)",&[
                    m["note_id"].clone(),json!(&pid)
                ]).ok();
            }
        }
        // Import file relations
        if let Some(arr) = manifest["file_relations"].as_array() {
            for fr in arr {
                let fid = fr["file_id"].as_str().unwrap_or("");
                // Copy file from archive to storage
                let ext = fr["file_ext"].as_str().unwrap_or("bin");
                let src = tmp.join("files").join(format!("{}.{}", fid, ext));
                let hash = fr["content_hash"].as_str().unwrap_or("");
                let dest = crate::commands::file_commands::flat_disk_path(hash);
                if src.exists() && !dest.is_empty() && !hash.is_empty() {
                    if let Some(parent) = std::path::Path::new(&dest).parent() { std::fs::create_dir_all(parent).ok(); }
                    std::fs::copy(&src, &dest).ok();
                }
                database::run("INSERT OR REPLACE INTO file_assets(id,file_ext,file_size,content_hash,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6)",&[
                    json!(fid),fr["file_ext"].clone(),fr["file_size"].clone(),json!(hash),fr["created_at"].clone(),fr["updated_at"].clone()
                ]).ok();
                database::run("INSERT OR REPLACE INTO file_relations(id,file_id,entity_type,entity_id,custom_name,creator,description,folder_id,created_at) VALUES(?1,?2,'project',?3,?4,?5,?6,?7,?8)",&[
                    fr["id"].clone(),json!(fid),json!(&pid),fr["custom_name"].clone(),fr["creator"].clone(),fr["description"].clone(),fr["folder_id"].clone(),fr["created_at"].clone()
                ]).ok();
            }
        }
        // Import folders
        if let Some(arr) = manifest["library_folders"].as_array() {
            for f in arr {
                database::run("INSERT OR REPLACE INTO library_folders(id,name,parent_id,project_id,created_at) VALUES(?1,?2,?3,?4,?5)",&[
                    f["id"].clone(),f["name"].clone(),f["parent_id"].clone(),json!(&pid),f["created_at"].clone()
                ]).ok();
            }
        }
    }
    std::fs::remove_dir_all(&tmp).ok();
    Ok(json!({"success":true}))
}

#[tauri::command] pub async fn project_import_file(file_path: String) -> Result<Value, String> {
    project_import_inner(&file_path)
}

fn project_import_inner(file_path: &str) -> Result<Value, String> {
    eprintln!("[项目导入] 开始导入: {}", file_path);
    let file = std::fs::File::open(file_path).map_err(|e| format!("无法打开文件: {}", e))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("无法解压: {}", e))?;
    let tmp = std::env::temp_dir().join(format!("ark_import_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    archive.extract(&tmp).map_err(|e| format!("解压失败: {}", e))?;
    let manifest_path = tmp.join("manifest.json");
    let manifest_str = std::fs::read_to_string(&manifest_path).map_err(|e| format!("读取manifest失败: {}", e))?;
    let manifest: Value = serde_json::from_str(&manifest_str).map_err(|e| format!("解析manifest失败: {}", e))?;
    let sols = manifest["solutions"].as_array().map(|a| a.len()).unwrap_or(0);
    eprintln!("[项目导入] manifest 中有 {} 个方案", sols);

    if let Some(proj) = manifest["project"].as_object() {
        let pid = proj["id"].as_str().unwrap_or("").to_string();
        if pid.is_empty() { return Err("项目ID缺失".into()); }

        // 去重映射：source_id → actual_id
        let mut partner_id_map: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        let mut contact_id_map: std::collections::HashMap<String, String> = std::collections::HashMap::new();

        // Insert project
        let cols = ["id","name","description","status","owner","budget","bid_time","expected_order_date","won_at","created_at","updated_at"];
        let mut vals: Vec<Value> = Vec::new();
        let mut placeholders: Vec<String> = Vec::new();
        for c in &cols {
            vals.push(proj.get(*c).cloned().unwrap_or(json!("")));
            placeholders.push("?".to_string());
        }
        database::run(&format!("INSERT OR REPLACE INTO projects({}) VALUES({})",cols.join(","),placeholders.join(",")),&vals).map_err(|e| e.to_string())?;

        // Solutions
        if let Some(arr) = manifest["solutions"].as_array() {
            for sol in arr {
                database::run("INSERT OR REPLACE INTO solutions(id,project_id,name,created_at,updated_at) VALUES(?1,?2,?3,?4,?5)",&[
                    sol["id"].clone(),json!(&pid),sol["name"].clone(),sol["created_at"].clone(),sol["updated_at"].clone()
                ]).ok();
                if let Some(vers) = sol["versions"].as_array() {
                    for v in vers {
                        if let Err(e) = database::run("INSERT OR REPLACE INTO solution_versions(id,solution_id,version,tags,content,creator,status,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",&[
                            v["id"].clone(),sol["id"].clone(),v["version"].clone(),v["tags"].clone(),v["content"].clone(),v["creator"].as_str().map(|s| json!(s)).unwrap_or(json!("")),v["status"].clone(),v["created_at"].clone(),v["updated_at"].clone()
                        ]) {
                            eprintln!("[导入] 版本插入失败: {}", e);
                        }
                        if let Some(items) = v["items"].as_array() {
                            for item in items {
                                if let Err(e) = database::run("INSERT OR REPLACE INTO solution_items(id,solution_version_id,product_id,product_name,device_name,quantity,unit_price,description,row_type,sort_order,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",&[
                                    item["id"].clone(),v["id"].clone(),item["product_id"].as_str().map(|s| json!(s)).unwrap_or(item["productId"].as_str().map(|s| json!(s)).unwrap_or(json!(""))),item["product_name"].as_str().map(|s| json!(s)).unwrap_or(json!("")),item["device_name"].as_str().map(|s| json!(s)).unwrap_or(json!("")),item["quantity"].clone(),item["unit_price"].as_f64().map(|v| json!(v)).unwrap_or(json!(0)),item["description"].as_str().map(|s| json!(s)).unwrap_or(json!("")),item["row_type"].as_str().map(|s| json!(s)).unwrap_or(json!("device")),item["sort_order"].as_i64().map(|v| json!(v)).unwrap_or(json!(0)),item["created_at"].clone()
                                ]) {
                                    eprintln!("[导入] 设备项插入失败: {}", e);
                                }
                            }
                        }
                    }
                }
            }
        }
        // Partners (new or legacy format) - with name deduplication
        if let Some(arr) = manifest["partners"].as_array() {
            for p in arr {
                let source_id = p["id"].as_str().unwrap_or("");
                let name = p["name"].as_str().unwrap_or("");
                let ptype = p["type"].as_str().unwrap_or("");
                // 查重：同名同类型的合作伙伴
                let actual_id = if let Ok(Some(row)) = database::get(
                    "SELECT id FROM partners WHERE name=? AND type=?",
                    &[json!(name), json!(ptype)]
                ) {
                    let existing_id = row["id"].as_str().unwrap_or(source_id).to_string();
                    eprintln!("[导入] 合作伙伴 '{}' 已存在，复用 ID: {}", name, existing_id);
                    existing_id
                } else {
                    database::run("INSERT OR IGNORE INTO partners(id,type,name,phone,company,remark,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",&[
                        p["id"].clone(),p["type"].clone(),p["name"].clone(),
                        p["phone"].as_str().map(|s| json!(s)).unwrap_or(json!("")),
                        p["company"].as_str().map(|s| json!(s)).unwrap_or(json!("")),
                        p["remark"].as_str().map(|s| json!(s)).unwrap_or(json!("")),
                        p["created_at"].clone(),p["updated_at"].as_str().map(|s| json!(s)).unwrap_or(p["created_at"].clone())
                    ]).ok();
                    source_id.to_string()
                };
                partner_id_map.insert(source_id.to_string(), actual_id);
            }
        } else {
            // Legacy format: convert channel_partners → partners
            if let Some(arr) = manifest["channel_partners"].as_array() {
                for ch in arr {
                    let source_id = ch["id"].as_str().unwrap_or("");
                    let name = ch["name"].as_str().unwrap_or("");
                    // 查重
                    let actual_id = if let Ok(Some(row)) = database::get(
                        "SELECT id FROM partners WHERE name=? AND type='CHANNEL'",
                        &[json!(name)]
                    ) {
                        row["id"].as_str().unwrap_or(source_id).to_string()
                    } else {
                        database::run("INSERT OR IGNORE INTO partners(id,type,name,phone,company,remark,created_at,updated_at) VALUES(?1,'CHANNEL',?2,?3,?4,?5,?6,?6)",&[
                            ch["id"].clone(),ch["name"].clone(),
                            ch["phone"].as_str().map(|s| json!(s)).unwrap_or(json!("")),
                            ch["company"].as_str().map(|s| json!(s)).unwrap_or(json!("")),
                            ch["remark"].as_str().map(|s| json!(s)).unwrap_or(json!("")),
                            ch["created_at"].clone()
                        ]).ok();
                        source_id.to_string()
                    };
                    partner_id_map.insert(source_id.to_string(), actual_id.clone());
                    crate::services::business_relations::upsert("CHANNEL", &actual_id, "PROJECT", &pid, "belongs_to", "{}");
                    if let Some(contacts) = ch["contacts"].as_array() {
                        for ct in contacts {
                            let ct_source_id = ct["id"].as_str().unwrap_or("");
                            let ct_name = ct["name"].as_str().unwrap_or("");
                            // 联系人查重
                            let ct_actual_id = if let Ok(Some(row)) = database::get(
                                "SELECT id FROM partner_contacts WHERE name=?",
                                &[json!(ct_name)]
                            ) {
                                row["id"].as_str().unwrap_or(ct_source_id).to_string()
                            } else {
                                database::run("INSERT OR IGNORE INTO partner_contacts(id,name,phone,email,position,remark,created_at) VALUES(?1,?2,?3,?4,'',?5,?6)",&[
                                    ct["id"].clone(),ct["name"].clone(),
                                    ct["phone"].as_str().map(|s| json!(s)).unwrap_or(json!("")),
                                    ct["email"].as_str().map(|s| json!(s)).unwrap_or(json!("")),
                                    ct["remark"].as_str().map(|s| json!(s)).unwrap_or(json!("")),
                                    ct["created_at"].clone()
                                ]).ok();
                                ct_source_id.to_string()
                            };
                            contact_id_map.insert(ct_source_id.to_string(), ct_actual_id.clone());
                            database::run("INSERT OR IGNORE INTO partner_contact_mapping(partner_id,contact_id) VALUES(?1,?2)",&[
                                json!(&actual_id),json!(&ct_actual_id)
                            ]).ok();
                        }
                    }
                }
            }
            // Legacy format: convert vendor_partners → partners
            if let Some(arr) = manifest["vendor_partners"].as_array() {
                for vp in arr {
                    let source_id = vp["id"].as_str().unwrap_or("");
                    let name = vp["name"].as_str().unwrap_or("");
                    // 查重
                    let actual_id = if let Ok(Some(row)) = database::get(
                        "SELECT id FROM partners WHERE name=? AND type='VENDOR'",
                        &[json!(name)]
                    ) {
                        row["id"].as_str().unwrap_or(source_id).to_string()
                    } else {
                        database::run("INSERT OR IGNORE INTO partners(id,type,name,phone,company,remark,created_at,updated_at) VALUES(?1,'VENDOR',?2,?3,?4,?5,?6,?6)",&[
                            vp["id"].clone(),vp["name"].clone(),
                            vp["phone"].as_str().map(|s| json!(s)).unwrap_or(json!("")),
                            vp["company"].as_str().map(|s| json!(s)).unwrap_or(json!("")),
                            vp["remark"].as_str().map(|s| json!(s)).unwrap_or(json!("")),
                            vp["created_at"].clone()
                        ]).ok();
                        source_id.to_string()
                    };
                    partner_id_map.insert(source_id.to_string(), actual_id.clone());
                    crate::services::business_relations::upsert("VENDOR", &actual_id, "PROJECT", &pid, "belongs_to", "{}");
                    if let Some(contacts) = vp["contacts"].as_array() {
                        for ct in contacts {
                            let ct_source_id = ct["id"].as_str().unwrap_or("");
                            let ct_name = ct["name"].as_str().unwrap_or("");
                            // 联系人查重
                            let ct_actual_id = if let Ok(Some(row)) = database::get(
                                "SELECT id FROM partner_contacts WHERE name=?",
                                &[json!(ct_name)]
                            ) {
                                row["id"].as_str().unwrap_or(ct_source_id).to_string()
                            } else {
                                database::run("INSERT OR IGNORE INTO partner_contacts(id,name,phone,email,position,remark,created_at) VALUES(?1,?2,?3,?4,'',?5,?6)",&[
                                    ct["id"].clone(),ct["name"].clone(),
                                    ct["phone"].as_str().map(|s| json!(s)).unwrap_or(json!("")),
                                    ct["email"].as_str().map(|s| json!(s)).unwrap_or(json!("")),
                                    ct["remark"].as_str().map(|s| json!(s)).unwrap_or(json!("")),
                                    ct["created_at"].clone()
                                ]).ok();
                                ct_source_id.to_string()
                            };
                            contact_id_map.insert(ct_source_id.to_string(), ct_actual_id.clone());
                            database::run("INSERT OR IGNORE INTO partner_contact_mapping(partner_id,contact_id) VALUES(?1,?2)",&[
                                json!(&actual_id),json!(&ct_actual_id)
                            ]).ok();
                        }
                    }
                }
            }
        }
        // Partner ↔ Project mappings
        if let Some(arr) = manifest["partner_project_mappings"].as_array() {
            for m in arr {
                let source_partner_id = m["partner_id"].as_str().unwrap_or("");
                let fallback = source_partner_id.to_string();
                let actual_partner_id = partner_id_map.get(source_partner_id).unwrap_or(&fallback);
                database::run("INSERT OR IGNORE INTO partner_project_mapping(partner_id,project_id,partner_type,selected_contact_id,remark) VALUES(?1,?2,?3,?4,?5)",&[
                    json!(actual_partner_id),json!(&pid),m["partner_type"].clone(),
                    m["selected_contact_id"].as_str().map(|s| json!(s)).unwrap_or(json!("")),
                    m["remark"].as_str().map(|s| json!(s)).unwrap_or(json!(""))
                ]).ok();
            }
        } else {
            // Legacy format: use business_relations to reconstruct partner_project_mapping
            if let Some(arr) = manifest["business_relations"].as_array() {
                for br in arr {
                    if br["relation_type"].as_str() == Some("belongs_to")
                        && br["target_type"].as_str() == Some("PROJECT")
                        && br["target_id"].as_str() == Some(&pid)
                    {
                        let st = br["source_type"].as_str().unwrap_or("");
                        let sid = br["source_id"].as_str().unwrap_or("");
                        if !sid.is_empty() && ["CHANNEL","VENDOR","CUSTOMER"].contains(&st) {
                            let fallback_sid = sid.to_string();
                            let actual_sid = partner_id_map.get(sid).unwrap_or(&fallback_sid);
                            database::run("INSERT OR IGNORE INTO partner_project_mapping(partner_id,project_id,partner_type,remark) VALUES(?1,?2,?3,'')",&[
                                json!(actual_sid),json!(&pid),json!(st)
                            ]).ok();
                        }
                    }
                }
            }
        }
        // Partner contacts (new format) - with name deduplication
        if let Some(arr) = manifest["partner_contacts"].as_array() {
            for pc in arr {
                let ct_source_id = pc["id"].as_str().unwrap_or("");
                let ct_name = pc["name"].as_str().unwrap_or("");
                // 查重：同名联系人
                let ct_actual_id = if let Ok(Some(row)) = database::get(
                    "SELECT id FROM partner_contacts WHERE name=?",
                    &[json!(ct_name)]
                ) {
                    let existing_id = row["id"].as_str().unwrap_or(ct_source_id).to_string();
                    eprintln!("[导入] 联系人 '{}' 已存在，复用 ID: {}", ct_name, existing_id);
                    existing_id
                } else {
                    database::run("INSERT OR IGNORE INTO partner_contacts(id,name,phone,email,position,remark,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",&[
                        pc["id"].clone(),pc["name"].clone(),
                        pc["phone"].as_str().map(|s| json!(s)).unwrap_or(json!("")),
                        pc["email"].as_str().map(|s| json!(s)).unwrap_or(json!("")),
                        pc["position"].as_str().map(|s| json!(s)).unwrap_or(json!("")),
                        pc["remark"].as_str().map(|s| json!(s)).unwrap_or(json!("")),
                        pc["created_at"].clone()
                    ]).ok();
                    ct_source_id.to_string()
                };
                contact_id_map.insert(ct_source_id.to_string(), ct_actual_id);
            }
        }
        if let Some(arr) = manifest["partner_contact_mapping"].as_array() {
            for m in arr {
                let source_partner_id = m["partner_id"].as_str().unwrap_or("");
                let source_contact_id = m["contact_id"].as_str().unwrap_or("");
                let fallback_p = source_partner_id.to_string();
                let fallback_c = source_contact_id.to_string();
                let actual_partner_id = partner_id_map.get(source_partner_id).unwrap_or(&fallback_p);
                let actual_contact_id = contact_id_map.get(source_contact_id).unwrap_or(&fallback_c);
                database::run("INSERT OR IGNORE INTO partner_contact_mapping(partner_id,contact_id) VALUES(?1,?2)",&[
                    json!(actual_partner_id),json!(actual_contact_id)
                ]).ok();
            }
        }
        // Contact ↔ Project mappings
        if let Some(arr) = manifest["contact_project_mappings"].as_array() {
            for m in arr {
                let source_contact_id = m["contact_id"].as_str().unwrap_or("");
                let fallback_c = source_contact_id.to_string();
                let actual_contact_id = contact_id_map.get(source_contact_id).unwrap_or(&fallback_c);
                database::run("INSERT OR IGNORE INTO contact_project_mapping(contact_id,project_id,remark) VALUES(?1,?2,?3)",&[
                    json!(actual_contact_id),json!(&pid),
                    m["remark"].as_str().map(|s| json!(s)).unwrap_or(json!(""))
                ]).ok();
            }
        } else {
            // Legacy format: reconstruct contact_project_mapping from partner mappings
            if let Some(ppm_arr) = manifest["partner_project_mappings"].as_array() {
                for ppm in ppm_arr {
                    let source_partner_id = ppm["partner_id"].as_str().unwrap_or("");
                    let fallback = source_partner_id.to_string();
                    let actual_partner_id = partner_id_map.get(source_partner_id).unwrap_or(&fallback);
                    if !actual_partner_id.is_empty() {
                        let contacts = database::all(
                            "SELECT contact_id FROM partner_contact_mapping WHERE partner_id=?",
                            &[json!(actual_partner_id)]
                        ).unwrap_or_default();
                        for c in &contacts {
                            if let Some(cid) = c["contact_id"].as_str() {
                                database::run("INSERT OR IGNORE INTO contact_project_mapping(contact_id,project_id,remark) VALUES(?1,?2,'')",&[
                                    json!(cid),json!(&pid)
                                ]).ok();
                            }
                        }
                    }
                }
            }
        }
        // Business relations (new format)
        if let Some(arr) = manifest["business_relations"].as_array() {
            for br in arr {
                crate::services::business_relations::upsert(
                    br["source_type"].as_str().unwrap_or(""),
                    br["source_id"].as_str().unwrap_or(""),
                    br["target_type"].as_str().unwrap_or(""),
                    br["target_id"].as_str().unwrap_or(""),
                    br["relation_type"].as_str().unwrap_or(""),
                    br["metadata"].as_str().unwrap_or("{}"),
                );
            }
        }
        // Dynamics
        if let Some(arr) = manifest["dynamics"].as_array() {
            for d in arr {
                database::run("INSERT OR REPLACE INTO project_dynamics(id,project_id,type,content,operator,created_at) VALUES(?1,?2,?3,?4,?5,?6)",&[
                    d["id"].clone(),json!(&pid),d["type"].clone(),d["content"].clone(),d["operator"].clone(),d["created_at"].clone()
                ]).ok();
            }
        }
        // Todos
        if let Some(arr) = manifest["todos"].as_array() {
            for t in arr {
                database::run("INSERT OR REPLACE INTO todos(id,title,content,done,priority,due_date,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",&[
                    t["id"].clone(),t["title"].clone(),t["content"].clone(),t["done"].clone(),t["priority"].clone(),t["due_date"].clone(),t["created_at"].clone(),t["updated_at"].clone()
                ]).ok();
            }
        }
        if let Some(arr) = manifest["todo_mappings"].as_array() {
            for m in arr {
                database::run("INSERT OR REPLACE INTO todo_project_mapping(todo_id,project_id,created_at) VALUES(?1,?2,?3)",&[
                    m["todo_id"].clone(),json!(&pid),m["created_at"].clone()
                ]).ok();
            }
        }
        // Notes
        if let Some(arr) = manifest["notes"].as_array() {
            for n in arr {
                database::run("INSERT OR REPLACE INTO notes(note_id,title,content,is_daily,notebook_id,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",&[
                    n["note_id"].clone(),n["title"].clone(),n["content"].clone(),n["is_daily"].clone(),n["notebook_id"].clone(),n["created_at"].clone(),n["updated_at"].clone()
                ]).ok();
            }
        }
        if let Some(arr) = manifest["note_mappings"].as_array() {
            for m in arr {
                database::run("INSERT OR REPLACE INTO note_project_mapping(note_id,project_id) VALUES(?1,?2)",&[
                    m["note_id"].clone(),json!(&pid)
                ]).ok();
            }
        }
        // Files
        if let Some(arr) = manifest["file_relations"].as_array() {
            for fr in arr {
                let fid = fr["file_id"].as_str().unwrap_or("");
                let ext = fr["file_ext"].as_str().unwrap_or("bin");
                let src = tmp.join("files").join(format!("{}.{}", fid, ext));
                let hash = fr["content_hash"].as_str().unwrap_or("");
                let dest = crate::commands::file_commands::flat_disk_path(hash);
                if src.exists() && !dest.is_empty() && !hash.is_empty() {
                    if let Some(parent) = std::path::Path::new(&dest).parent() { std::fs::create_dir_all(parent).ok(); }
                    std::fs::copy(&src, &dest).map_err(|e| format!("文件复制失败({}): {}", hash, e))?;
                }
                database::run("INSERT OR REPLACE INTO file_assets(id,file_ext,file_size,content_hash,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6)",&[
                    json!(fid),fr["file_ext"].clone(),fr["file_size"].clone(),json!(hash),fr["created_at"].clone(),fr["updated_at"].clone()
                ]).ok();
                database::run("INSERT OR REPLACE INTO file_relations(id,file_id,entity_type,entity_id,custom_name,creator,description,folder_id,created_at) VALUES(?1,?2,'project',?3,?4,?5,?6,?7,?8)",&[
                    fr["id"].clone(),json!(fid),json!(&pid),fr["custom_name"].clone(),fr["creator"].clone(),fr["description"].clone(),fr["folder_id"].clone(),fr["created_at"].clone()
                ]).ok();
            }
        }
        // Folders
        if let Some(arr) = manifest["library_folders"].as_array() {
            for f in arr {
                database::run("INSERT OR REPLACE INTO library_folders(id,name,parent_id,project_id,created_at) VALUES(?1,?2,?3,?4,?5)",&[
                    f["id"].clone(),f["name"].clone(),f["parent_id"].clone(),json!(&pid),f["created_at"].clone()
                ]).ok();
            }
        }
    }
    std::fs::remove_dir_all(&tmp).ok();
    eprintln!("[导入] 完成: {}", file_path);
    Ok(json!({"success":true}))
}
#[tauri::command] pub async fn project_export(project_id: String, save_path: Option<String>) -> Result<Value, String> {
    let mut result = project_export_inner(&project_id).await?;
    if let Some(sp) = save_path {
        let tmp_path = result["exportPath"].as_str().unwrap_or("");
        if !tmp_path.is_empty() {
            std::fs::copy(tmp_path, &sp).map_err(|e| format!("保存失败: {}", e))?;
            result["exportPath"] = json!(sp);
        }
    }
    Ok(result)
}
#[tauri::command] pub async fn project_export_files(_project_id: String) -> Result<Value, String> { Ok(json!({"success":true})) }
#[tauri::command] pub async fn projects_batch_export(project_ids: Vec<String>, file_path: String) -> Result<Value, String> {
    if project_ids.is_empty() { return Err("无项目".into()); }
    let file = std::fs::File::create(&file_path).map_err(|e| format!("创建文件失败: {}", e))?;
    let mut zip_writer = zip::ZipWriter::new(file);
    let opts = zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for pid in &project_ids {
        let result = project_export_inner(pid).await?;
        let tmp_path = result["exportPath"].as_str().unwrap_or("");
        if tmp_path.is_empty() { continue }
        let project_name = result["project"]["name"].as_str().unwrap_or("unnamed");
        let project_id = pid.chars().take(8).collect::<String>();
        let entry_name = format!("{}_{}.ark", project_name, project_id);
        zip_writer.start_file(entry_name, opts).map_err(|e| format!("创建zip条目失败: {}", e))?;
        let mut tmp_file = std::fs::File::open(tmp_path).map_err(|e| format!("读取临时文件失败: {}", e))?;
        std::io::copy(&mut tmp_file, &mut zip_writer).map_err(|e| format!("写入zip失败: {}", e))?;
    }
    zip_writer.finish().map_err(|e| format!("完成zip失败: {}", e))?;
    Ok(json!({"success":true}))
}
// Solutions
#[tauri::command] pub async fn solution_list(project_id: String) -> Result<Vec<Value>, String> {
    let solutions = database::all("SELECT * FROM solutions WHERE project_id=? ORDER BY created_at DESC", &[json!(project_id)]).map_err(|e| e.to_string())?;
    let mut result = Vec::new();
    for s in &solutions {
        let sol_id = s["id"].as_str().unwrap_or("");
        let versions = if !sol_id.is_empty() {
            let mut rows = database::all("SELECT sv.id, sv.version as versionNum, sv.tags, sv.content, sv.creator, sv.created_at as createdAt, sv.status, COALESCE((SELECT SUM(si2.quantity * si2.unit_price) FROM solution_items si2 WHERE si2.solution_version_id=sv.id), 0) as amount, COALESCE(si.items, '[]') as items FROM solution_versions sv LEFT JOIN (SELECT solution_version_id, json_group_array(json_object('id', si.id, 'productId', si.product_id, 'product_id', si.product_id, 'model', si.product_name, 'deviceName', si.device_name, 'count', si.quantity, 'price', si.unit_price, 'name', CASE WHEN si.row_type='subnet' THEN si.description ELSE '' END, 'parameters', si.description, 'rowType', si.row_type, 'sortOrder', si.sort_order)) as items FROM solution_items si GROUP BY solution_version_id) si ON si.solution_version_id = sv.id WHERE sv.solution_id=? ORDER BY sv.created_at DESC", &[json!(sol_id)]).unwrap_or_default();
            for row in &mut rows {
                if let Some(s) = row["items"].as_str() {
                    if let Ok(arr) = serde_json::from_str::<Value>(s) {
                        row["items"] = arr;
                    }
                }
            }
            rows
        } else { vec![] };
        let mut obj = s.clone();
        obj["versions"] = json!(versions);
        result.push(obj);
    }
    Ok(result)
}
#[tauri::command] pub async fn solution_create(project_id: String, name: String) -> Result<Value, String> { let id = uuid::Uuid::new_v4().to_string(); database::run("INSERT INTO solutions(id,project_id,name) VALUES(?,?,?)",&[json!(id),json!(project_id),json!(name)]).map_err(|e| e.to_string())?; touch_project(&project_id); sync_solution_graph(&project_id); database::get("SELECT * FROM solutions WHERE id=?",&[json!(id)]).map_err(|e| e.to_string()).map(|r| r.unwrap_or(Value::Null)) }
#[tauri::command] pub async fn solution_rename(solution_id: String, name: String) -> Result<(), String> { database::run("UPDATE solutions SET name=?,updated_at=datetime('now', 'localtime') WHERE id=?",&[json!(name),json!(solution_id)]).map_err(|e| e.to_string())?; if let Ok(Some(r)) = database::get("SELECT project_id FROM solutions WHERE id=?1",&[json!(&solution_id)]) { if let Some(pid) = r["project_id"].as_str() { touch_project(pid); sync_solution_graph(pid); } } Ok(()) }
#[tauri::command] pub async fn solution_delete(solution_id: String) -> Result<(), String> { let pid = database::get("SELECT project_id FROM solutions WHERE id=?1",&[json!(&solution_id)]).ok().flatten().and_then(|r| r["project_id"].as_str().map(|s| s.to_string())); database::run("DELETE FROM business_relations WHERE (source_type='SOLUTION' AND source_id=?1) OR (target_type='SOLUTION' AND target_id=?1) OR (source_type='SOLUTION_VERSION' AND source_id IN (SELECT id FROM solution_versions WHERE solution_id=?1)) OR (target_type='SOLUTION_VERSION' AND target_id IN (SELECT id FROM solution_versions WHERE solution_id=?1)) OR (source_type='SOLUTION_ITEM' AND source_id IN (SELECT si.id FROM solution_items si JOIN solution_versions sv ON sv.id=si.solution_version_id WHERE sv.solution_id=?1)) OR (target_type='SOLUTION_ITEM' AND target_id IN (SELECT si.id FROM solution_items si JOIN solution_versions sv ON sv.id=si.solution_version_id WHERE sv.solution_id=?1))",&[json!(&solution_id)]).ok(); database::run("DELETE FROM solution_items WHERE solution_version_id IN (SELECT id FROM solution_versions WHERE solution_id=?)",&[json!(&solution_id)]).ok(); database::run("DELETE FROM solution_versions WHERE solution_id=?",&[json!(&solution_id)]).ok(); database::run("DELETE FROM solutions WHERE id=?",&[json!(solution_id)]).map_err(|e| e.to_string())?; if let Some(pid) = pid { sync_project_products(&pid).ok(); sync_solution_graph(&pid); touch_project(&pid); } Ok(()) }
#[tauri::command] pub async fn solution_version_list(solution_id: String) -> Result<Vec<Value>, String> {
    let mut rows = database::all("SELECT sv.id, sv.version as versionNum, sv.tags, sv.content, sv.creator, sv.created_at as createdAt, sv.status, COALESCE((SELECT SUM(si2.quantity * si2.unit_price) FROM solution_items si2 WHERE si2.solution_version_id=sv.id), 0) as amount, COALESCE((SELECT json_group_array(json_object('id', si.id, 'productId', si.product_id, 'product_id', si.product_id, 'model', si.product_name, 'deviceName', si.device_name, 'count', si.quantity, 'price', si.unit_price, 'name', CASE WHEN si.row_type='subnet' THEN si.description ELSE '' END, 'parameters', si.description, 'rowType', si.row_type, 'sortOrder', si.sort_order)) FROM solution_items si WHERE si.solution_version_id=sv.id), '[]') as items FROM solution_versions sv WHERE sv.solution_id=? ORDER BY sv.created_at DESC",&[json!(solution_id)]).map_err(|e| e.to_string())?;
    for row in &mut rows {
        if let Some(s) = row["items"].as_str() {
            if let Ok(arr) = serde_json::from_str::<Value>(s) {
                row["items"] = arr;
            }
        }
    }
    Ok(rows)
}
#[tauri::command] pub async fn solution_version_create(solution_id: String, data: Value) -> Result<Value, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let version_num = data["versionNum"].as_str().unwrap_or("V1.0");
    let tags = data["tags"].as_array().map(|a| a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join(",")).unwrap_or_default();
    let content = data["content"].as_str().unwrap_or("");
    let creator = data["creator"].as_str().unwrap_or("");
    let status = data["status"].as_str().unwrap_or("final");
    database::run(
        "INSERT INTO solution_versions(id,solution_id,version,tags,content,creator,status) VALUES(?1,?2,?3,?4,?5,?6,?7)",
        &[json!(id), json!(solution_id), json!(version_num), json!(tags), json!(content), json!(creator), json!(status)]
    ).map_err(|e| e.to_string())?;

    // 保存设备清单
    if let Some(items) = data["deviceItems"].as_array() {
        for (i, item) in items.iter().enumerate() {
            let item_id = uuid::Uuid::new_v4().to_string();
            let row_type = item["rowType"].as_str().unwrap_or("device");
            let device_name = item["deviceName"].as_str().unwrap_or("");
            let model = item["model"].as_str().unwrap_or("");
            let product_id = item["productId"].as_str().unwrap_or("");
            // description: 子网存 name，设备存 parameters
            let parameters = if row_type == "subnet" { item["name"].as_str().unwrap_or("") } else { item["parameters"].as_str().unwrap_or("") };
            let count = item["count"].as_i64().unwrap_or(1);
            let price = item["price"].as_f64().unwrap_or(0.0);
            database::run(
                "INSERT INTO solution_items(id,solution_version_id,product_id,product_name,device_name,quantity,unit_price,description,row_type,sort_order) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                &[json!(item_id), json!(&id), json!(product_id), json!(model), json!(device_name), json!(count), json!(price), json!(parameters), json!(row_type), json!(i as i64)]
            ).map_err(|e| e.to_string())?;
        }
    }
    // ★ 解决方案中的产品变化 → 刷新关联项目的 source_data，触发知识管线重新物化
    let mut pid: Option<String> = None;
    if let Ok(Some(sol)) = database::get("SELECT project_id FROM solutions WHERE id=?1",&[json!(&solution_id)]) {
        if let Some(p) = sol["project_id"].as_str().filter(|s| !s.is_empty()) {
            pid = Some(p.to_string());
        }
    }

    // 返回完整的版本数据（含设备清单）
    let mut version = database::get("SELECT sv.id, sv.version as versionNum, sv.tags, sv.content, sv.creator, sv.created_at as createdAt, sv.status, COALESCE((SELECT SUM(si2.quantity * si2.unit_price) FROM solution_items si2 WHERE si2.solution_version_id=sv.id), 0) as amount, COALESCE((SELECT json_group_array(json_object('id', si.id, 'productId', si.product_id, 'product_id', si.product_id, 'model', si.product_name, 'deviceName', si.device_name, 'count', si.quantity, 'price', si.unit_price, 'name', CASE WHEN si.row_type='subnet' THEN si.description ELSE '' END, 'parameters', si.description, 'rowType', si.row_type, 'sortOrder', si.sort_order)) FROM solution_items si WHERE si.solution_version_id=sv.id), '[]') as items FROM solution_versions sv WHERE sv.id=?", &[json!(id)])
        .map_err(|e| e.to_string())?.unwrap_or(Value::Null);
    // ★ 解析 items 从 JSON 字符串到原生数组
    if let Some(s) = version["items"].as_str() {
        if let Ok(arr) = serde_json::from_str::<Value>(s) {
            version["items"] = arr;
        }
    }

    // 同步产品↔项目关联（轻量映射，供 LLM 检索用）
    if let Some(p) = &pid { touch_project(p); sync_project_products(p).ok(); sync_solution_graph(p); }

    Ok(version)
}

/// 收集项目所有方案中使用的所有产品，重写 project_product_mapping 与 business_relations
pub fn sync_project_products(project_id: &str) -> Result<(), String> {
    let products = database::all(
        "SELECT DISTINCT si.product_id, si.product_name FROM solution_items si \
         JOIN solution_versions sv ON sv.id = si.solution_version_id \
         JOIN solutions s ON s.id = sv.solution_id \
         WHERE s.project_id=? AND si.product_id IS NOT NULL AND si.product_id != ''",
        &[json!(project_id)]
    ).unwrap_or_default();
    // Clean old business_relations for this project
    match database::run(
        "DELETE FROM business_relations WHERE source_type='PROJECT' AND source_id=? AND target_type='PRODUCT' AND relation_type='belongs_to'",
        &[json!(project_id)]
    ) {
        Ok(r) => log::info!("[业务关联] 删除: PROJECT[{}] → PRODUCT[*] (belongs_to) {} 行", project_id, r.changes),
        Err(e) => log::info!("[业务关联] 删除失败: PROJECT[{}] → PRODUCT[*] (belongs_to): {}", project_id, e),
    }
    database::run("DELETE FROM project_product_mapping WHERE project_id=?", &[json!(project_id)]).ok();
    for p in &products {
        let product_id = match p["product_id"].as_str() { Some(pid) => pid, None => continue };
        let product_name = p["product_name"].as_str().unwrap_or("");
        database::run(
            "INSERT OR IGNORE INTO project_product_mapping(project_id,product_id,product_name) VALUES(?1,?2,?3)",
            &[json!(project_id), json!(product_id), json!(product_name)]
        ).ok();
        let meta = serde_json::json!({"product_name": product_name});
        crate::services::business_relations::upsert("PROJECT", project_id, "PRODUCT", product_id, "belongs_to", &meta.to_string());
    }
    Ok(())
}

/// 按版本 ID 反查项目并重建方案图谱
fn sync_graph_for_version(version_id: &str) {
    if let Ok(Some(r)) = database::get("SELECT s.project_id FROM solution_versions sv JOIN solutions s ON s.id=sv.solution_id WHERE sv.id=?1", &[json!(version_id)]) {
        if let Some(pid) = r["project_id"].as_str() { sync_solution_graph(pid); }
    }
}

/// 重建项目的解决方案图谱边（幂等：先清后建）
/// PROJECT -has_solution-> SOLUTION -has_version-> SOLUTION_VERSION -uses-> SOLUTION_ITEM -instance_of-> PRODUCT
pub fn sync_solution_graph(project_id: &str) {
    use crate::services::business_relations as br;
    // 1. 清理该项目旧的图谱边（只清图谱关系，不动 belongs_to 等其他边）
    database::run(
        "DELETE FROM business_relations WHERE relation_type IN ('has_solution','has_version','uses','instance_of') AND (\
         (source_type='PROJECT' AND source_id=?1) OR \
         (source_type='SOLUTION' AND source_id IN (SELECT id FROM solutions WHERE project_id=?1)) OR \
         (source_type='SOLUTION_VERSION' AND source_id IN (SELECT sv.id FROM solution_versions sv JOIN solutions s ON s.id=sv.solution_id WHERE s.project_id=?1)) OR \
         (source_type='SOLUTION_ITEM' AND source_id IN (SELECT si.id FROM solution_items si JOIN solution_versions sv ON sv.id=si.solution_version_id JOIN solutions s ON s.id=sv.solution_id WHERE s.project_id=?1)))",
        &[json!(project_id)]
    ).ok();
    // 2. PROJECT -has_solution-> SOLUTION
    let solutions = database::all("SELECT id, name FROM solutions WHERE project_id=?1", &[json!(project_id)]).unwrap_or_default();
    for s in &solutions {
        let sid = s["id"].as_str().unwrap_or("");
        if sid.is_empty() { continue; }
        let meta = json!({"solution_name": s["name"].as_str().unwrap_or("")});
        br::upsert("PROJECT", project_id, "SOLUTION", sid, "has_solution", &meta.to_string());
    }
    // 3. SOLUTION -has_version-> SOLUTION_VERSION（metadata 带金额/设备数）
    let versions = database::all(
        "SELECT sv.id, sv.solution_id, sv.version, sv.status, s.name AS solution_name, \
         COALESCE(SUM(si.quantity*si.unit_price),0) AS amount, COUNT(si.id) AS item_count \
         FROM solution_versions sv JOIN solutions s ON s.id=sv.solution_id \
         LEFT JOIN solution_items si ON si.solution_version_id=sv.id \
         WHERE s.project_id=?1 GROUP BY sv.id",
        &[json!(project_id)]
    ).unwrap_or_default();
    for v in &versions {
        let vid = v["id"].as_str().unwrap_or("");
        let sol_id = v["solution_id"].as_str().unwrap_or("");
        if vid.is_empty() || sol_id.is_empty() { continue; }
        let meta = json!({
            "version": v["version"].as_str().unwrap_or(""),
            "status": v["status"].as_str().unwrap_or(""),
            "solution_name": v["solution_name"].as_str().unwrap_or(""),
            "amount": v["amount"].as_f64().unwrap_or(0.0),
            "amount_unit": "元",
            "item_count": v["item_count"].as_i64().unwrap_or(0),
        });
        br::upsert("SOLUTION", sol_id, "SOLUTION_VERSION", vid, "has_version", &meta.to_string());
    }
    // 4. SOLUTION_VERSION -uses-> SOLUTION_ITEM；SOLUTION_ITEM -instance_of-> PRODUCT
    let items = database::all(
        "SELECT si.id, si.solution_version_id, si.product_id, si.product_name, si.device_name, si.quantity, si.unit_price \
         FROM solution_items si JOIN solution_versions sv ON sv.id=si.solution_version_id \
         JOIN solutions s ON s.id=sv.solution_id WHERE s.project_id=?1 AND si.row_type='device'",
        &[json!(project_id)]
    ).unwrap_or_default();
    for it in &items {
        let iid = it["id"].as_str().unwrap_or("");
        let vid = it["solution_version_id"].as_str().unwrap_or("");
        if iid.is_empty() || vid.is_empty() { continue; }
        let model = it["product_name"].as_str().unwrap_or("");
        let meta = json!({
            "model": model,
            "device_name": it["device_name"].as_str().unwrap_or(""),
            "quantity": it["quantity"].as_i64().unwrap_or(0),
            "unit_price": it["unit_price"].as_f64().unwrap_or(0.0),
            "price_unit": "元",
        });
        br::upsert("SOLUTION_VERSION", vid, "SOLUTION_ITEM", iid, "uses", &meta.to_string());
        let product_id = it["product_id"].as_str().unwrap_or("");
        if !product_id.is_empty() {
            br::upsert("SOLUTION_ITEM", iid, "PRODUCT", product_id, "instance_of", &json!({"model": model}).to_string());
        }
    }
}
#[tauri::command] pub async fn solution_version_update_meta(version_id: String, data: Value) -> Result<(), String> {
    let mut fields: Vec<String> = Vec::new();
    let mut values: Vec<Value> = Vec::new();
    if let Some(v) = data["version"].as_str() { fields.push("version=?".into()); values.push(json!(v)); }
    if let Some(v) = data["content"].as_str() { fields.push("content=?".into()); values.push(json!(v)); }
    if let Some(v) = data["tags"].as_str() { fields.push("tags=?".into()); values.push(json!(v)); }
    if fields.is_empty() { return Ok(()); }
    values.push(json!(version_id));
    database::run(&format!("UPDATE solution_versions SET {} WHERE id=?", fields.join(",")), &values).map_err(|e| e.to_string())?;
    emit_for_version(&version_id);
    sync_graph_for_version(&version_id);
    Ok(())
}
#[tauri::command] pub async fn solution_version_finalize(version_id: String, _data: Value) -> Result<Value, String> { database::run("UPDATE solution_versions SET status='final',updated_at=datetime('now', 'localtime') WHERE id=?",&[json!(version_id)]).map_err(|e| e.to_string())?; emit_for_version(&version_id); sync_graph_for_version(&version_id); database::save().map_err(|e| e.to_string())?; let mut version = database::get("SELECT sv.id, sv.version as versionNum, sv.tags, sv.content, sv.creator, sv.created_at as createdAt, sv.status, COALESCE((SELECT SUM(si2.quantity * si2.unit_price) FROM solution_items si2 WHERE si2.solution_version_id=sv.id), 0) as amount, COALESCE((SELECT json_group_array(json_object('id', si.id, 'productId', si.product_id, 'product_id', si.product_id, 'model', si.product_name, 'deviceName', si.device_name, 'count', si.quantity, 'price', si.unit_price, 'name', CASE WHEN si.row_type='subnet' THEN si.description ELSE '' END, 'parameters', si.description, 'rowType', si.row_type, 'sortOrder', si.sort_order)) FROM solution_items si WHERE si.solution_version_id=sv.id), '[]') as items FROM solution_versions sv WHERE sv.id=?", &[json!(version_id)]).map_err(|e| e.to_string())?.unwrap_or(Value::Null); if let Some(s) = version["items"].as_str() { if let Ok(arr) = serde_json::from_str::<Value>(s) { version["items"] = arr; } } Ok(version) }
#[tauri::command] pub async fn solution_version_save_draft(version_id: String, data: Value) -> Result<Value, String> {
    // 更新版本元信息
    if let Some(v) = data["versionNum"].as_str() { database::run("UPDATE solution_versions SET version=?1 WHERE id=?2",&[json!(v),json!(&version_id)]).map_err(|e| e.to_string())?; }
    if let Some(c) = data["content"].as_str() { database::run("UPDATE solution_versions SET content=?1 WHERE id=?2",&[json!(c),json!(&version_id)]).map_err(|e| e.to_string())?; }
    if let Some(t) = data["tags"].as_array().map(|a| a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join(",")) { database::run("UPDATE solution_versions SET tags=?1 WHERE id=?2",&[json!(t),json!(&version_id)]).map_err(|e| e.to_string())?; }
    // 覆盖式保存设备清单：删除旧 items 后插入
    database::run("DELETE FROM solution_items WHERE solution_version_id=?",&[json!(&version_id)]).map_err(|e| e.to_string())?;
    if let Some(items) = data["deviceItems"].as_array() {
        for (i, item) in items.iter().enumerate() {
            let item_id = uuid::Uuid::new_v4().to_string();
            let row_type = item["rowType"].as_str().unwrap_or("device");
            let device_name = item["deviceName"].as_str().unwrap_or("");
            let model = item["model"].as_str().unwrap_or("");
            let product_id = item["productId"].as_str().unwrap_or("");
            let parameters = if row_type == "subnet" { item["name"].as_str().unwrap_or("") } else { item["parameters"].as_str().unwrap_or("") };
            let count = item["count"].as_i64().unwrap_or(1);
            let price = item["price"].as_f64().unwrap_or(0.0);
            database::run(
                "INSERT INTO solution_items(id,solution_version_id,product_id,product_name,device_name,quantity,unit_price,description,row_type,sort_order) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                &[json!(item_id), json!(&version_id), json!(product_id), json!(model), json!(device_name), json!(count), json!(price), json!(parameters), json!(row_type), json!(i as i64)]
            ).map_err(|e| e.to_string())?;
        }
    }
    database::run("UPDATE solution_versions SET updated_at=datetime('now', 'localtime') WHERE id=?",&[json!(&version_id)]).map_err(|e| e.to_string())?;
    // 返回完整版本数据（含 items 解析）
    let mut version = database::get("SELECT sv.id, sv.version as versionNum, sv.tags, sv.content, sv.creator, sv.created_at as createdAt, sv.status, COALESCE((SELECT SUM(si2.quantity * si2.unit_price) FROM solution_items si2 WHERE si2.solution_version_id=sv.id), 0) as amount, COALESCE((SELECT json_group_array(json_object('id', si.id, 'productId', si.product_id, 'product_id', si.product_id, 'model', si.product_name, 'deviceName', si.device_name, 'count', si.quantity, 'price', si.unit_price, 'name', CASE WHEN si.row_type='subnet' THEN si.description ELSE '' END, 'parameters', si.description, 'rowType', si.row_type, 'sortOrder', si.sort_order)) FROM solution_items si WHERE si.solution_version_id=sv.id), '[]') as items FROM solution_versions sv WHERE sv.id=?", &[json!(&version_id)])
        .map_err(|e| e.to_string())?.unwrap_or(Value::Null);
    if let Some(s) = version["items"].as_str() {
        if let Ok(arr) = serde_json::from_str::<Value>(s) {
            version["items"] = arr;
        }
    }
    Ok(version)
}
#[tauri::command] pub async fn solution_version_delete(version_id: String) -> Result<(), String> { let pid = database::get("SELECT s.project_id FROM solution_versions sv JOIN solutions s ON s.id=sv.solution_id WHERE sv.id=?1", &[json!(&version_id)]).ok().flatten().and_then(|r| r["project_id"].as_str().map(|s| s.to_string())); emit_for_version(&version_id); database::run("DELETE FROM business_relations WHERE (source_type='SOLUTION_VERSION' AND source_id=?1) OR (target_type='SOLUTION_VERSION' AND target_id=?1) OR (source_type='SOLUTION_ITEM' AND source_id IN (SELECT id FROM solution_items WHERE solution_version_id=?1)) OR (target_type='SOLUTION_ITEM' AND target_id IN (SELECT id FROM solution_items WHERE solution_version_id=?1))",&[json!(&version_id)]).ok(); database::run("DELETE FROM solution_items WHERE solution_version_id=?",&[json!(&version_id)]).ok(); database::run("DELETE FROM solution_versions WHERE id=?",&[json!(&version_id)]).map_err(|e| e.to_string())?; if let Some(pid) = pid { sync_project_products(&pid).ok(); sync_solution_graph(&pid); } Ok(()) }

/// 从方案版本 ID 反查项目 ID，触发项目重新物化
fn emit_for_version(version_id: &str) {
    if let Ok(Some(r)) = database::get("SELECT s.project_id FROM solution_versions sv JOIN solutions s ON s.id=sv.solution_id WHERE sv.id=?1", &[json!(version_id)]) {
        if let Some(pid) = r["project_id"].as_str() {
            touch_project(pid);
        }
    }
}
#[tauri::command] pub async fn solution_download_excel(data: Value) -> Result<Value, String> {
    let save_path = data["savePath"].as_str().unwrap_or("");
    if save_path.is_empty() {
        return Ok(json!({"success":false,"error":"未指定保存路径"}));
    }
    let buffer = data["buffer"].as_array().map(|arr| {
        arr.iter().filter_map(|v| v.as_u64().map(|n| n as u8)).collect::<Vec<u8>>()
    }).unwrap_or_default();
    if buffer.is_empty() {
        return Ok(json!({"success":false,"error":"文件内容为空"}));
    }
    std::fs::write(save_path, &buffer).map_err(|e| format!("写入文件失败: {}", e))?;
    Ok(json!({"success":true,"savedPath":save_path}))
}
