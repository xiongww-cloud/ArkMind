use crate::database;
use serde_json::{json, Value};

#[tauri::command] pub async fn project_list() -> Result<Vec<Value>, String> { database::all("SELECT * FROM projects ORDER BY updated_at DESC", &[]).map_err(|e| e.to_string()) }
#[tauri::command] pub async fn project_get(id: String) -> Result<Value, String> { database::get("SELECT * FROM projects WHERE id=?", &[json!(id)]).map_err(|e| e.to_string()).map(|r| r.unwrap_or(Value::Null)) }
#[tauri::command]
pub async fn project_create(data: Value) -> Result<Value, String> {
    let id = data["displayId"].as_str().filter(|s| !s.is_empty()).or_else(|| data["id"].as_str().filter(|s| !s.is_empty())).unwrap_or("").to_string();
    let id = if id.is_empty() { uuid::Uuid::new_v4().to_string() } else { id };
    let name = data["name"].as_str().unwrap_or("新项目").to_string();
    let desc = data["description"].as_str().unwrap_or("").to_string();
    let status = data["status"].as_str().unwrap_or("Lead");
    let budget_val = data["budget"].as_f64()
        .or_else(|| data["budget"].as_str().and_then(|s| s.parse::<f64>().ok()))
        .unwrap_or(0.0);
    database::run("INSERT INTO projects(id,name,status,description,budget,customer,owner,user_id,bid_time,customer_contact,customer_phone) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
    &[json!(&id),json!(&name),json!(status),json!(&desc),json!(budget_val),json!(data["customer"].as_str().unwrap_or("")),json!(data["owner"].as_str().unwrap_or("")),json!(data["userId"].as_str().unwrap_or("")),json!(data["bidTime"].as_str().unwrap_or("")),json!(data["customerContact"].as_str().unwrap_or("")),json!(data["customerPhone"].as_str().unwrap_or(""))]).map_err(|e| e.to_string())?;
    crate::services::knowledge_lifecycle::notify_changed("PROJECT", &id);
    crate::services::object_changed::object_changed(crate::services::object_changed::ObjectType::Project, &id, crate::services::object_changed::Action::Created);
    database::get("SELECT * FROM projects WHERE id=?", &[json!(id)]).map_err(|e| e.to_string()).map(|r| r.unwrap_or(Value::Null))
}
#[tauri::command] pub async fn project_update(id: String, data: Value) -> Result<(), String> {
    let mut fields: Vec<String> = Vec::new(); let mut values: Vec<Value> = Vec::new();
    for (k, col) in &[
        ("name","name"),("status","status"),("owner","owner"),("customer","customer"),
        ("budget","budget"),("description","description"),
        ("bid_time","bid_time"),("bidTime","bid_time"),
        ("customer_contact","customer_contact"),("customerContact","customer_contact"),
        ("customer_phone","customer_phone"),("customerPhone","customer_phone"),
    ] { if let Some(s) = data[*k].as_str() { fields.push(format!("{}=?",col)); values.push(json!(s)); } }
    if let Some(_v) = data["budget"].as_f64() { /* already handled via str in loop above */ }
    if fields.is_empty() { return Ok(()); }
    fields.push("updated_at=datetime('now', 'localtime')".into()); values.push(json!(id));
    database::run(&format!("UPDATE projects SET {} WHERE id=?", fields.join(",")), &values).map_err(|e| e.to_string())?;
    crate::services::knowledge_lifecycle::notify_changed("PROJECT", &id);
    crate::services::object_changed::object_changed(crate::services::object_changed::ObjectType::Project, &id, crate::services::object_changed::Action::Updated);
    Ok(())
}
#[tauri::command] pub async fn project_delete(id: String) -> Result<Value, String> {
    crate::services::knowledge_lifecycle::notify_deleted("PROJECT", &id);
    database::run("DELETE FROM projects WHERE id=?", &[json!(&id)]).map_err(|e| e.to_string())?;
    crate::services::object_changed::object_changed(crate::services::object_changed::ObjectType::Project, &id, crate::services::object_changed::Action::Deleted);
    database::run("DELETE FROM note_project_mapping WHERE project_id=?", &[json!(&id)]).ok();
    database::run("DELETE FROM tag_mappings WHERE entity_type='projects' AND entity_id=?", &[json!(&id)]).ok();
    database::run("DELETE FROM project_dynamics WHERE project_id=?", &[json!(&id)]).ok();
    database::run("DELETE FROM solution_items WHERE solution_version_id IN (SELECT id FROM solution_versions WHERE solution_id IN (SELECT id FROM solutions WHERE project_id=?))", &[json!(&id)]).ok();
    database::run("DELETE FROM solution_versions WHERE solution_id IN (SELECT id FROM solutions WHERE project_id=?)", &[json!(&id)]).ok();
    database::run("DELETE FROM solutions WHERE project_id=?", &[json!(&id)]).ok();
    database::save().map_err(|e| e.to_string())?;
    Ok(json!({"success":true}))
}
#[tauri::command] pub async fn project_dynamics_add(project_id: String, event: Value) -> Result<Value, String> {
    let id = uuid::Uuid::new_v4().to_string(); let ts = crate::time_utils::now_db();
    let content = event["content"].as_str().unwrap_or("").to_string();
    database::run("INSERT INTO project_dynamics(id,project_id,type,content,operator,created_at) VALUES(?1,?2,?3,?4,?5,?6)", &[json!(id),json!(project_id),json!(event["type"].as_str().unwrap_or("note")),json!(&content),json!(event["operator"].as_str().unwrap_or("")),json!(ts)]).map_err(|e| e.to_string())?;
    crate::services::object_changed::object_changed(crate::services::object_changed::ObjectType::Project, &project_id, crate::services::object_changed::Action::Updated);
    database::get("SELECT * FROM project_dynamics WHERE id=?", &[json!(id)]).map_err(|e| e.to_string()).map(|r| r.unwrap_or(Value::Null))
}
#[tauri::command] pub async fn project_dynamics_list(project_id: String) -> Result<Vec<Value>, String> { database::all("SELECT * FROM project_dynamics WHERE project_id=? ORDER BY created_at DESC", &[json!(project_id)]).map_err(|e| e.to_string()) }
#[tauri::command] pub async fn project_dynamics_list_all() -> Result<Vec<Value>, String> { database::all("SELECT pd.id, pd.project_id, pd.type, pd.content, pd.operator, pd.created_at, p.name as project_name FROM project_dynamics pd JOIN projects p ON p.id=pd.project_id ORDER BY pd.created_at DESC LIMIT 30", &[]).map_err(|e| e.to_string()) }
#[tauri::command] pub async fn project_dynamics_delete(dynamic_id: String) -> Result<(), String> {
    if let Ok(Some(r)) = database::get("SELECT project_id FROM project_dynamics WHERE id=?1",&[json!(&dynamic_id)]) {
        database::run("DELETE FROM project_dynamics WHERE id=? AND type='note'", &[json!(dynamic_id)]).map_err(|e| e.to_string())?;
        if let Some(pid) = r["project_id"].as_str() {
            crate::services::object_changed::object_changed(crate::services::object_changed::ObjectType::Project, pid, crate::services::object_changed::Action::Updated);
        }
    }
    Ok(())
}
#[tauri::command] pub async fn project_dynamics_edit(dynamic_id: String, content: String) -> Result<(), String> {
    if let Ok(Some(r)) = database::get("SELECT project_id FROM project_dynamics WHERE id=?1",&[json!(&dynamic_id)]) {
        database::run("UPDATE project_dynamics SET content=? WHERE id=? AND type='note'", &[json!(content), json!(dynamic_id)]).map_err(|e| e.to_string())?;
        if let Some(pid) = r["project_id"].as_str() {
            crate::services::object_changed::object_changed(crate::services::object_changed::ObjectType::Project, pid, crate::services::object_changed::Action::Updated);
        }
    }
    Ok(())
}
#[tauri::command] pub async fn project_update_status(project_id: String, new_status: String, operator: String) -> Result<(), String> { database::run("UPDATE projects SET status=?,updated_at=datetime('now', 'localtime') WHERE id=?", &[json!(new_status),json!(project_id)]).map_err(|e| e.to_string())?; let id = uuid::Uuid::new_v4().to_string(); database::run("INSERT INTO project_dynamics(id,project_id,type,content,operator) VALUES(?1,?2,'system',?3,?4)", &[json!(id),json!(project_id),json!(format!("项目状态更新为：{}",new_status)),json!(operator)]).ok(); crate::services::knowledge_lifecycle::notify_changed("PROJECT", &project_id); crate::services::object_changed::object_changed(crate::services::object_changed::ObjectType::Project, &project_id, crate::services::object_changed::Action::Updated); Ok(()) }
#[tauri::command] pub async fn project_archive(project_id: String, _operator: String) -> Result<(), String> { database::run("UPDATE projects SET status='Archived',updated_at=datetime('now', 'localtime') WHERE id=?", &[json!(project_id)]).map_err(|e| e.to_string())?; crate::services::knowledge_lifecycle::notify_changed("PROJECT", &project_id); crate::services::object_changed::object_changed(crate::services::object_changed::ObjectType::Project, &project_id, crate::services::object_changed::Action::Updated); Ok(()) }

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
    let channels = {
        let mut result = Vec::new();
        for ch in database::all("SELECT * FROM channel_partners WHERE project_id=?",&[json!(&project_id)]).unwrap_or_default() {
            let cid = ch["id"].as_str().unwrap_or("");
            let contacts = database::all("SELECT * FROM channel_contacts WHERE channel_id=?",&[json!(cid)]).unwrap_or_default();
            let mut c = ch.clone();
            c.as_object_mut().map(|m| { m.insert("contacts".into(), json!(contacts)); });
            result.push(c);
        }
        result
    };
    let vendors = {
        let mut result = Vec::new();
        for vp in database::all("SELECT * FROM vendor_partners WHERE project_id=?",&[json!(&project_id)]).unwrap_or_default() {
            let vid = vp["id"].as_str().unwrap_or("");
            let contacts = database::all("SELECT * FROM vendor_contacts WHERE vendor_id=?",&[json!(vid)]).unwrap_or_default();
            let mut v = vp.clone();
            v.as_object_mut().map(|m| { m.insert("contacts".into(), json!(contacts)); });
            result.push(v);
        }
        result
    };
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
        "version":1,"name":name,"project":p,"solutions":solutions,
        "channel_partners":channels,"vendor_partners":vendors,"dynamics":dynamics,
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
        let cols = ["id","name","description","status","owner","budget","customer","bid_time","customer_contact","customer_phone","created_at","updated_at"];
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
        // Import channels
        if let Some(arr) = manifest["channel_partners"].as_array() {
            for ch in arr {
                database::run("INSERT OR REPLACE INTO channel_partners(id,project_id,name,phone,company,remark,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",&[
                    ch["id"].clone(),json!(&pid),ch["name"].clone(),ch["phone"].clone(),ch["company"].clone(),ch["remark"].clone(),ch["created_at"].clone()
                ]).ok();
                if let Some(contacts) = ch["contacts"].as_array() {
                    for ct in contacts {
                        database::run("INSERT OR REPLACE INTO channel_contacts(id,channel_id,name,phone,email,remark,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",&[
                            ct["id"].clone(),ch["id"].clone(),ct["name"].clone(),ct["phone"].clone(),ct["email"].clone(),ct["remark"].clone(),ct["created_at"].clone()
                        ]).ok();
                    }
                }
            }
        }
        // Import vendors
        if let Some(arr) = manifest["vendor_partners"].as_array() {
            for vp in arr {
                database::run("INSERT OR REPLACE INTO vendor_partners(id,project_id,name,phone,company,remark,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",&[
                    vp["id"].clone(),json!(&pid),vp["name"].clone(),vp["phone"].clone(),vp["company"].clone(),vp["remark"].clone(),vp["created_at"].clone()
                ]).ok();
                if let Some(contacts) = vp["contacts"].as_array() {
                    for ct in contacts {
                        database::run("INSERT OR REPLACE INTO vendor_contacts(id,vendor_id,name,phone,email,remark,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",&[
                            ct["id"].clone(),vp["id"].clone(),ct["name"].clone(),ct["phone"].clone(),ct["email"].clone(),ct["remark"].clone(),ct["created_at"].clone()
                        ]).ok();
                    }
                }
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
                database::run("INSERT OR REPLACE INTO file_assets(id,file_ext,file_size,content_hash,parse_status,created_at,updated_at) VALUES(?1,?2,?3,?4,'COMPLETED',?5,?6)",&[
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

        // Insert project
        let cols = ["id","name","description","status","owner","budget","customer","bid_time","customer_contact","customer_phone","created_at","updated_at"];
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
        // Channels
        if let Some(arr) = manifest["channel_partners"].as_array() {
            for ch in arr {
                database::run("INSERT OR REPLACE INTO channel_partners(id,project_id,name,phone,company,remark,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",&[
                    ch["id"].clone(),json!(&pid),ch["name"].clone(),ch["phone"].clone(),ch["company"].clone(),ch["remark"].clone(),ch["created_at"].clone()
                ]).ok();
                if let Some(contacts) = ch["contacts"].as_array() {
                    for ct in contacts {
                        database::run("INSERT OR REPLACE INTO channel_contacts(id,channel_id,name,phone,email,remark,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",&[
                            ct["id"].clone(),ch["id"].clone(),ct["name"].clone(),ct["phone"].clone(),ct["email"].clone(),ct["remark"].clone(),ct["created_at"].clone()
                        ]).ok();
                    }
                }
            }
        }
        // Vendors
        if let Some(arr) = manifest["vendor_partners"].as_array() {
            for vp in arr {
                database::run("INSERT OR REPLACE INTO vendor_partners(id,project_id,name,phone,company,remark,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",&[
                    vp["id"].clone(),json!(&pid),vp["name"].clone(),vp["phone"].clone(),vp["company"].clone(),vp["remark"].clone(),vp["created_at"].clone()
                ]).ok();
                if let Some(contacts) = vp["contacts"].as_array() {
                    for ct in contacts {
                        database::run("INSERT OR REPLACE INTO vendor_contacts(id,vendor_id,name,phone,email,remark,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",&[
                            ct["id"].clone(),vp["id"].clone(),ct["name"].clone(),ct["phone"].clone(),ct["email"].clone(),ct["remark"].clone(),ct["created_at"].clone()
                        ]).ok();
                    }
                }
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
                database::run("INSERT OR REPLACE INTO file_assets(id,file_ext,file_size,content_hash,parse_status,created_at,updated_at) VALUES(?1,?2,?3,?4,'COMPLETED',?5,?6)",&[
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
#[tauri::command] pub async fn solution_create(project_id: String, name: String) -> Result<Value, String> { let id = uuid::Uuid::new_v4().to_string(); database::run("INSERT INTO solutions(id,project_id,name) VALUES(?,?,?)",&[json!(id),json!(project_id),json!(name)]).map_err(|e| e.to_string())?; crate::services::object_changed::object_changed(crate::services::object_changed::ObjectType::Project, &project_id, crate::services::object_changed::Action::Updated); database::get("SELECT * FROM solutions WHERE id=?",&[json!(id)]).map_err(|e| e.to_string()).map(|r| r.unwrap_or(Value::Null)) }
#[tauri::command] pub async fn solution_rename(solution_id: String, name: String) -> Result<(), String> { database::run("UPDATE solutions SET name=?,updated_at=datetime('now', 'localtime') WHERE id=?",&[json!(name),json!(solution_id)]).map_err(|e| e.to_string())?; if let Ok(Some(r)) = database::get("SELECT project_id FROM solutions WHERE id=?1",&[json!(&solution_id)]) { if let Some(pid) = r["project_id"].as_str() { crate::services::object_changed::object_changed(crate::services::object_changed::ObjectType::Project, pid, crate::services::object_changed::Action::Updated); } } Ok(()) }
#[tauri::command] pub async fn solution_delete(solution_id: String) -> Result<(), String> { let pid = database::get("SELECT project_id FROM solutions WHERE id=?1",&[json!(&solution_id)]).ok().flatten().and_then(|r| r["project_id"].as_str().map(|s| s.to_string())); database::run("DELETE FROM solution_items WHERE solution_version_id IN (SELECT id FROM solution_versions WHERE solution_id=?)",&[json!(&solution_id)]).ok(); database::run("DELETE FROM solution_versions WHERE solution_id=?",&[json!(&solution_id)]).ok(); database::run("DELETE FROM solutions WHERE id=?",&[json!(solution_id)]).map_err(|e| e.to_string())?; if let Some(pid) = pid { crate::services::object_changed::object_changed(crate::services::object_changed::ObjectType::Project, &pid, crate::services::object_changed::Action::Updated); } Ok(()) }
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
    database::run(
        "INSERT INTO solution_versions(id,solution_id,version,tags,content,creator,status) VALUES(?1,?2,?3,?4,?5,?6,'final')",
        &[json!(id), json!(solution_id), json!(version_num), json!(tags), json!(content), json!(creator)]
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
    if let Ok(Some(sol)) = database::get("SELECT project_id FROM solutions WHERE id=?1",&[json!(&solution_id)]) {
        if let Some(pid) = sol["project_id"].as_str().filter(|s| !s.is_empty()) {
            crate::services::object_changed::object_changed(crate::services::object_changed::ObjectType::Project, pid, crate::services::object_changed::Action::Updated);
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
    sync_project_products(&solution_id).ok();

    Ok(version)
}

/// 收集方案中使用的所有产品，写入 project_product_mapping
fn sync_project_products(solution_id: &str) -> Result<(), String> {
    let project_id = match database::get("SELECT project_id FROM solutions WHERE id=?", &[json!(solution_id)])
        .ok().flatten().and_then(|r| r["project_id"].as_str().map(|s| s.to_string())) {
        Some(pid) => pid,
        None => return Ok(()),
    };
    let products = database::all(
        "SELECT DISTINCT si.product_id, si.product_name FROM solution_items si \
         JOIN solution_versions sv ON sv.id = si.solution_version_id \
         WHERE sv.solution_id=? AND si.product_id IS NOT NULL AND si.product_id != ''",
        &[json!(solution_id)]
    ).unwrap_or_default();
    for p in &products {
        let product_id = match p["product_id"].as_str() { Some(pid) => pid, None => continue };
        let product_name = p["product_name"].as_str().unwrap_or("");
        database::run(
            "INSERT OR IGNORE INTO project_product_mapping(project_id,product_id,product_name) VALUES(?1,?2,?3)",
            &[json!(&project_id), json!(product_id), json!(product_name)]
        ).ok();
    }
    Ok(())
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
    Ok(())
}
#[tauri::command] pub async fn solution_version_finalize(version_id: String, _data: Value) -> Result<Value, String> { database::run("UPDATE solution_versions SET status='final',updated_at=datetime('now', 'localtime') WHERE id=?",&[json!(version_id)]).map_err(|e| e.to_string())?; emit_for_version(&version_id); database::save().map_err(|e| e.to_string())?; database::get("SELECT * FROM solution_versions WHERE id=?",&[json!(version_id)]).map_err(|e| e.to_string()).map(|r| r.unwrap_or(Value::Null)) }
#[tauri::command] pub async fn solution_version_save_draft(version_id: String, _data: Value) -> Result<Value, String> { database::run("UPDATE solution_versions SET updated_at=datetime('now', 'localtime') WHERE id=?",&[json!(version_id)]).map_err(|e| e.to_string())?; database::get("SELECT * FROM solution_versions WHERE id=?",&[json!(version_id)]).map_err(|e| e.to_string()).map(|r| r.unwrap_or(Value::Null)) }
#[tauri::command] pub async fn solution_version_delete(version_id: String) -> Result<(), String> { emit_for_version(&version_id); database::run("DELETE FROM solution_items WHERE solution_version_id=?",&[json!(&version_id)]).ok(); database::run("DELETE FROM solution_versions WHERE id=?",&[json!(&version_id)]).map_err(|e| e.to_string())?; Ok(()) }

/// 从方案版本 ID 反查项目 ID，触发项目重新物化
fn emit_for_version(version_id: &str) {
    if let Ok(Some(r)) = database::get("SELECT s.project_id FROM solution_versions sv JOIN solutions s ON s.id=sv.solution_id WHERE sv.id=?1", &[json!(version_id)]) {
        if let Some(pid) = r["project_id"].as_str() {
            crate::services::object_changed::object_changed(crate::services::object_changed::ObjectType::Project, pid, crate::services::object_changed::Action::Updated);
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
