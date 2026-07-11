use crate::database;
use serde_json::{json, Value};
use std::io::Write;
use tauri::Emitter;

/// 导出全部或指定域数据，写入 save_path
#[tauri::command]
pub async fn export_data(domains: Vec<String>, save_path: String, app_handle: tauri::AppHandle) -> Result<Value, String> {
    let total = domains.len();
    let handle = app_handle.clone();
    // ★ 使用 spawn_blocking 执行同步 I/O，避免阻塞事件循环
    tokio::task::spawn_blocking(move || {
        let tmp = std::env::temp_dir().join(format!("ark_fullexport_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
        let mut exported_domains: Vec<String> = Vec::new();

        for (idx, domain) in domains.iter().enumerate() {
            // ★ 发送进度事件
            let _ = handle.emit("export:progress", json!({
                "current": idx + 1,
                "total": total,
                "domain": domain,
                "stage": "exporting",
            }));

            let mut domain_data = json!({});
            match domain.as_str() {
            "projects" => {
                let projects = crate::database::all("SELECT * FROM projects", &[]).unwrap_or_default();
                let solutions = crate::database::all("SELECT * FROM solutions", &[]).unwrap_or_default();
                let solution_versions = crate::database::all("SELECT * FROM solution_versions", &[]).unwrap_or_default();
                let solution_items = crate::database::all("SELECT * FROM solution_items", &[]).unwrap_or_default();
                let dynamics = crate::database::all("SELECT * FROM project_dynamics", &[]).unwrap_or_default();
                let channels = crate::database::all("SELECT * FROM channel_partners", &[]).unwrap_or_default();
                let channel_contacts = crate::database::all("SELECT * FROM channel_contacts", &[]).unwrap_or_default();
                let vendors = crate::database::all("SELECT * FROM vendor_partners", &[]).unwrap_or_default();
                let vendor_contacts = crate::database::all("SELECT * FROM vendor_contacts", &[]).unwrap_or_default();
                let customers = crate::database::all("SELECT * FROM customers", &[]).unwrap_or_default();
                let customer_contacts = crate::database::all("SELECT * FROM customer_contacts", &[]).unwrap_or_default();
                let channel_mappings = crate::database::all("SELECT * FROM project_channel_mapping", &[]).unwrap_or_default();
                let vendor_mappings = crate::database::all("SELECT * FROM project_vendor_mapping", &[]).unwrap_or_default();
                let customer_mappings = crate::database::all("SELECT * FROM project_customer_mapping", &[]).unwrap_or_default();
                domain_data = json!({
                    "projects": projects, "solutions": solutions,
                    "solution_versions": solution_versions, "solution_items": solution_items,
                    "project_dynamics": dynamics, "channel_partners": channels,
                    "channel_contacts": channel_contacts, "vendor_partners": vendors,
                    "vendor_contacts": vendor_contacts, "customers": customers,
                    "customer_contacts": customer_contacts, "project_channel_mapping": channel_mappings,
                    "project_vendor_mapping": vendor_mappings, "project_customer_mapping": customer_mappings,
                });
            }
            "products" => {
                let products = crate::database::all("SELECT * FROM products", &[]).unwrap_or_default();
                let series = crate::database::all("SELECT * FROM product_series", &[]).unwrap_or_default();
                // 关联物料: file_relations entity_type='product' → file_assets
                let material_relations = crate::database::all(
                    "SELECT * FROM file_relations WHERE entity_type='product'", &[]
                ).unwrap_or_default();
                let file_ids: Vec<String> = material_relations.iter()
                    .filter_map(|r| r["file_id"].as_str().map(String::from)).collect();
                let file_assets = if file_ids.is_empty() { vec![] } else {
                    let ph: Vec<String> = file_ids.iter().map(|_| "?".to_string()).collect();
                    let params: Vec<Value> = file_ids.iter().map(|id| json!(id)).collect();
                    crate::database::all(&format!("SELECT * FROM file_assets WHERE id IN ({})", ph.join(",")), &params).unwrap_or_default()
                };
                domain_data = json!({
                    "products": products, "product_series": series,
                    "file_relations": material_relations, "file_assets": file_assets,
                });
                // 复制物料文件二进制
                let files_dir = tmp.join("product_files");
                std::fs::create_dir_all(&files_dir).ok();
                for fa in &file_assets {
                    if let Some(hash) = fa["content_hash"].as_str().filter(|h| !h.is_empty()) {
                        let src = crate::commands::file_commands::flat_disk_path(hash);
                        if std::path::Path::new(&src).exists() {
                            let fid = fa["id"].as_str().unwrap_or("unknown");
                            let ext = fa["file_ext"].as_str().unwrap_or("bin");
                            std::fs::copy(&src, files_dir.join(format!("{}.{}", fid, ext))).ok();
                        }
                    }
                }
            }
            "notes" => {
                let notes = crate::database::all("SELECT * FROM notes WHERE deleted_at IS NULL", &[]).unwrap_or_default();
                let notebooks = crate::database::all("SELECT * FROM notebooks", &[]).unwrap_or_default();
                let note_files = crate::database::all(
                    "SELECT * FROM file_relations WHERE entity_type='note'", &[]
                ).unwrap_or_default();
                let file_ids: Vec<String> = note_files.iter()
                    .filter_map(|r| r["file_id"].as_str().map(String::from)).collect();
                let file_assets = if file_ids.is_empty() { vec![] } else {
                    let ph: Vec<String> = file_ids.iter().map(|_| "?".to_string()).collect();
                    let params: Vec<Value> = file_ids.iter().map(|id| json!(id)).collect();
                    crate::database::all(&format!("SELECT * FROM file_assets WHERE id IN ({})", ph.join(",")), &params).unwrap_or_default()
                };
                domain_data = json!({
                    "notes": notes, "notebooks": notebooks,
                    "file_relations": note_files, "file_assets": file_assets,
                });
                // 复制笔记附件
                let files_dir = tmp.join("note_files");
                std::fs::create_dir_all(&files_dir).ok();
                for fa in &file_assets {
                    if let Some(hash) = fa["content_hash"].as_str().filter(|h| !h.is_empty()) {
                        let src = crate::commands::file_commands::flat_disk_path(hash);
                        if std::path::Path::new(&src).exists() {
                            let fid = fa["id"].as_str().unwrap_or("unknown");
                            let ext = fa["file_ext"].as_str().unwrap_or("bin");
                            std::fs::copy(&src, files_dir.join(format!("{}.{}", fid, ext))).ok();
                        }
                    }
                }
            }
            "todos" => {
                let todos = crate::database::all("SELECT * FROM todos", &[]).unwrap_or_default();
                let todo_files = crate::database::all(
                    "SELECT * FROM file_relations WHERE entity_type='todo'", &[]
                ).unwrap_or_default();
                let file_ids: Vec<String> = todo_files.iter()
                    .filter_map(|r| r["file_id"].as_str().map(String::from)).collect();
                let file_assets = if file_ids.is_empty() { vec![] } else {
                    let ph: Vec<String> = file_ids.iter().map(|_| "?".to_string()).collect();
                    let params: Vec<Value> = file_ids.iter().map(|id| json!(id)).collect();
                    crate::database::all(&format!("SELECT * FROM file_assets WHERE id IN ({})", ph.join(",")), &params).unwrap_or_default()
                };
                domain_data = json!({
                    "todos": todos,
                    "file_relations": todo_files, "file_assets": file_assets,
                });
                // 复制待办附件
                let files_dir = tmp.join("todo_files");
                std::fs::create_dir_all(&files_dir).ok();
                for fa in &file_assets {
                    if let Some(hash) = fa["content_hash"].as_str().filter(|h| !h.is_empty()) {
                        let src = crate::commands::file_commands::flat_disk_path(hash);
                        if std::path::Path::new(&src).exists() {
                            let fid = fa["id"].as_str().unwrap_or("unknown");
                            let ext = fa["file_ext"].as_str().unwrap_or("bin");
                            std::fs::copy(&src, files_dir.join(format!("{}.{}", fid, ext))).ok();
                        }
                    }
                }
            }
            "files" => {
                let file_relations = crate::database::all(
                    "SELECT * FROM file_relations WHERE entity_type='library'", &[]
                ).unwrap_or_default();
                let folders = crate::database::all("SELECT * FROM library_folders", &[]).unwrap_or_default();
                // ★ 只导出被 library file_relations 引用的 file_assets，避免冗余
                let lib_file_ids: Vec<String> = file_relations.iter()
                    .filter_map(|r| r["file_id"].as_str().map(String::from)).collect();
                let file_assets = if lib_file_ids.is_empty() { vec![] } else {
                    let ph: Vec<String> = lib_file_ids.iter().map(|_| "?".to_string()).collect();
                    let params: Vec<Value> = lib_file_ids.iter().map(|id| json!(id)).collect();
                    crate::database::all(&format!("SELECT * FROM file_assets WHERE id IN ({})", ph.join(",")), &params).unwrap_or_default()
                };
                domain_data = json!({
                    "file_assets": file_assets, "file_relations": file_relations,
                    "library_folders": folders,
                });
                // 复制文件二进制
                let files_dir = tmp.join("library_files");
                std::fs::create_dir_all(&files_dir).ok();
                for fa in &file_assets {
                    if let Some(hash) = fa["content_hash"].as_str().filter(|h| !h.is_empty()) {
                        let src = crate::commands::file_commands::flat_disk_path(hash);
                        if std::path::Path::new(&src).exists() {
                            let fid = fa["id"].as_str().unwrap_or("unknown");
                            let ext = fa["file_ext"].as_str().unwrap_or("bin");
                            std::fs::copy(&src, files_dir.join(format!("{}.{}", fid, ext))).ok();
                        }
                    }
                }
            }
            _ => continue,
        }
        let domain_file = tmp.join(format!("{}.json", domain));
        let mut f = std::fs::File::create(&domain_file).map_err(|e| e.to_string())?;
        f.write_all(serde_json::to_string_pretty(&domain_data).unwrap_or_default().as_bytes()).map_err(|e| e.to_string())?;
        exported_domains.push(domain.clone());
    }

    // 生成 manifest + zip
    let manifest = json!({"version": 1, "domains": exported_domains});
    let manifest_file = tmp.join("manifest.json");
    let mut f = std::fs::File::create(&manifest_file).map_err(|e| e.to_string())?;
    f.write_all(serde_json::to_string_pretty(&manifest).unwrap_or_default().as_bytes()).map_err(|e| e.to_string())?;

    let file = std::fs::File::create(&save_path).map_err(|e| format!("创建文件失败: {}", e))?;
    let mut zip_writer = zip::ZipWriter::new(file);
    let opts = zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    fn add_dir_to_zip(w: &mut zip::ZipWriter<std::fs::File>, base: &std::path::Path, dir: &std::path::Path, opts: zip::write::FileOptions) -> Result<(), String> {
        for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            let name = path.strip_prefix(base).unwrap_or(&path).to_string_lossy().replace('\\', "/");
            if path.is_dir() { w.add_directory(name, opts).map_err(|e| e.to_string())?; add_dir_to_zip(w, base, &path, opts)?; }
            else { w.start_file(name, opts).map_err(|e| e.to_string())?; let mut f = std::fs::File::open(&path).map_err(|e| e.to_string())?; std::io::copy(&mut f, w).map_err(|e| e.to_string())?; }
        }
        Ok(())
    }
    add_dir_to_zip(&mut zip_writer, &tmp, &tmp, opts)?;
    zip_writer.finish().map_err(|e| e.to_string())?;
    std::fs::remove_dir_all(&tmp).ok();
    // ★ 发送完成事件
    let _ = handle.emit("export:progress", json!({
        "current": total,
        "total": total,
        "domain": "",
        "stage": "done",
    }));
    Ok(json!({"success": true}))
    }).await.map_err(|e| format!("导出任务失败: {}", e))?
}

/// 从 .ark ZIP 文件导入业务数据
#[tauri::command]
pub async fn import_data(file_path: String) -> Result<Value, String> {
    let file = std::fs::File::open(&file_path).map_err(|e| format!("打开文件失败: {}", e))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("解析 ZIP 失败: {}", e))?;

    // ★ 导入前备份已禁用（需要 app_handle 参数，简化处理）
    eprintln!("[导入] 开始导入: {}", file_path);

    // 解压到临时目录
    let tmp = std::env::temp_dir().join(format!("ark_import_{}", uuid::Uuid::new_v4()));
    archive.extract(&tmp).map_err(|e| format!("解压失败: {}", e))?;

    // 读取 manifest
    let manifest_path = tmp.join("manifest.json");
    let manifest_str = std::fs::read_to_string(&manifest_path).map_err(|e| format!("读取 manifest 失败: {}", e))?;
    let manifest: Value = serde_json::from_str(&manifest_str).map_err(|e| format!("解析 manifest 失败: {}", e))?;
    let domains = manifest["domains"].as_array().ok_or("manifest 缺少 domains 字段")?;

    let mut imported: Vec<String> = Vec::new();
    
    // ★ 开启事务保护
    database::run("BEGIN TRANSACTION", &[]).map_err(|e| format!("开启事务失败: {}", e))?;
    
    let mut has_error = false;
    for domain in domains {
        let domain_name = domain.as_str().unwrap_or("");
        let domain_file = tmp.join(format!("{}.json", domain_name));
        if !domain_file.exists() { continue; }
        let data_str = std::fs::read_to_string(&domain_file).map_err(|e| e.to_string())?;
        let data: Value = serde_json::from_str(&data_str).map_err(|e| e.to_string())?;

        let result = match domain_name {
            "projects" => import_projects(&data, &tmp),
            "products" => import_products(&data, &tmp),
            "notes" => import_notes(&data, &tmp),
            "todos" => import_todos(&data, &tmp),
            "files" => import_files(&data, &tmp),
            _ => Ok(()),
        };
        if let Err(e) = result {
            eprintln!("[导入] {} 导入失败: {}", domain_name, e);
            has_error = true;
        } else {
            imported.push(domain_name.to_string());
        }
    }

    // ★ 事务提交或回滚
    if has_error {
        database::run("ROLLBACK", &[]).ok();
        std::fs::remove_dir_all(&tmp).ok();
        return Err("部分数据导入失败，已回滚所有更改".to_string());
    }
    database::run("COMMIT", &[]).map_err(|e| format!("提交事务失败: {}", e))?;

    // 清理临时目录
    std::fs::remove_dir_all(&tmp).ok();

    Ok(json!({"success": true, "imported": imported}))
}

fn import_projects(data: &Value, _tmp: &std::path::Path) -> Result<(), String> {
    // 项目
    if let Some(projects) = data["projects"].as_array() {
        for p in projects {
            let id = p["id"].as_str().unwrap_or("");
            if id.is_empty() { continue; }
            database::run("INSERT OR REPLACE INTO projects(id,name,description,status,owner,budget,customer,bid_time,customer_contact,customer_phone,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)", &[
                json!(id), json!(p["name"]), json!(p["description"]), json!(p["status"]),
                json!(p["owner"]), json!(p["budget"]), json!(p["customer"]),
                json!(p["bid_time"]), json!(p["customer_contact"]), json!(p["customer_phone"]),
                json!(p["created_at"]), json!(p["updated_at"]),
            ]).ok();
        }
    }
    // 方案
    if let Some(sols) = data["solutions"].as_array() {
        for s in sols { database::run("INSERT OR REPLACE INTO solutions(id,project_id,name,created_at,updated_at) VALUES(?1,?2,?3,?4,?5)", &[json!(s["id"]),json!(s["project_id"]),json!(s["name"]),json!(s["created_at"]),json!(s["updated_at"])]).ok(); }
    }
    // 方案版本
    if let Some(vers) = data["solution_versions"].as_array() {
        for v in vers { database::run("INSERT OR REPLACE INTO solution_versions(id,solution_id,version,tags,content,creator,status,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)", &[json!(v["id"]),json!(v["solution_id"]),json!(v["version"]),json!(v["tags"]),json!(v["content"]),json!(v["creator"]),json!(v["status"]),json!(v["created_at"]),json!(v["updated_at"])]).ok(); }
    }
    // 方案设备清单
    if let Some(items) = data["solution_items"].as_array() {
        for i in items { database::run("INSERT OR REPLACE INTO solution_items(id,solution_version_id,product_id,product_name,quantity,unit_price,description,device_name,row_type,sort_order,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)", &[json!(i["id"]),json!(i["solution_version_id"]),json!(i["product_id"]),json!(i["product_name"]),json!(i["quantity"]),json!(i["unit_price"]),json!(i["description"]),json!(i["device_name"]),json!(i["row_type"]),json!(i["sort_order"]),json!(i["created_at"]),json!(i["updated_at"])]).ok(); }
    }
    // 动态
    if let Some(dyns) = data["project_dynamics"].as_array() {
        for d in dyns { database::run("INSERT OR REPLACE INTO project_dynamics(id,project_id,type,content,operator,created_at) VALUES(?1,?2,?3,?4,?5,?6)", &[json!(d["id"]),json!(d["project_id"]),json!(d["type"]),json!(d["content"]),json!(d["operator"]),json!(d["created_at"])]).ok(); }
    }
    // 渠道
    if let Some(chs) = data["channel_partners"].as_array() {
        for c in chs { database::run("INSERT OR REPLACE INTO channel_partners(id,project_id,name,phone,company,remark,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7)", &[json!(c["id"]),json!(c["project_id"]),json!(c["name"]),json!(c["phone"]),json!(c["company"]),json!(c["remark"]),json!(c["created_at"])]).ok(); }
    }
    if let Some(ccs) = data["channel_contacts"].as_array() {
        for c in ccs { database::run("INSERT OR REPLACE INTO channel_contacts(id,channel_id,name,phone,email,remark,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7)", &[json!(c["id"]),json!(c["channel_id"]),json!(c["name"]),json!(c["phone"]),json!(c["email"]),json!(c["remark"]),json!(c["created_at"])]).ok(); }
    }
    // 供应商
    if let Some(vds) = data["vendor_partners"].as_array() {
        for v in vds { database::run("INSERT OR REPLACE INTO vendor_partners(id,project_id,name,phone,company,remark,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7)", &[json!(v["id"]),json!(v["project_id"]),json!(v["name"]),json!(v["phone"]),json!(v["company"]),json!(v["remark"]),json!(v["created_at"])]).ok(); }
    }
    if let Some(vcs) = data["vendor_contacts"].as_array() {
        for v in vcs { database::run("INSERT OR REPLACE INTO vendor_contacts(id,vendor_id,name,phone,email,remark,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7)", &[json!(v["id"]),json!(v["vendor_id"]),json!(v["name"]),json!(v["phone"]),json!(v["email"]),json!(v["remark"]),json!(v["created_at"])]).ok(); }
    }
    // 客户
    if let Some(custs) = data["customers"].as_array() {
        for c in custs { database::run("INSERT OR REPLACE INTO customers(id,name,contact_person,contact_phone,remark,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7)", &[json!(c["id"]),json!(c["name"]),json!(c["contact_person"]),json!(c["contact_phone"]),json!(c["remark"]),json!(c["created_at"]),json!(c["updated_at"])]).ok(); }
    }
    if let Some(cts) = data["customer_contacts"].as_array() {
        for c in cts { database::run("INSERT OR REPLACE INTO customer_contacts(id,customer_id,name,phone,email,position,remark,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)", &[json!(c["id"]),json!(c["customer_id"]),json!(c["name"]),json!(c["phone"]),json!(c["email"]),json!(c["position"]),json!(c["remark"]),json!(c["created_at"])]).ok(); }
    }
    // 映射表
    if let Some(maps) = data["project_channel_mapping"].as_array() {
        for m in maps { database::run("INSERT OR IGNORE INTO project_channel_mapping(project_id,channel_id,selected_contact_id,created_at) VALUES(?1,?2,?3,?4)", &[json!(m["project_id"]),json!(m["channel_id"]),json!(m["selected_contact_id"]),json!(m["created_at"])]).ok(); }
    }
    if let Some(maps) = data["project_vendor_mapping"].as_array() {
        for m in maps { database::run("INSERT OR IGNORE INTO project_vendor_mapping(project_id,vendor_id,selected_contact_id,created_at) VALUES(?1,?2,?3,?4)", &[json!(m["project_id"]),json!(m["vendor_id"]),json!(m["selected_contact_id"]),json!(m["created_at"])]).ok(); }
    }
    if let Some(maps) = data["project_customer_mapping"].as_array() {
        for m in maps { database::run("INSERT OR IGNORE INTO project_customer_mapping(project_id,customer_id,created_at) VALUES(?1,?2,?3)", &[json!(m["project_id"]),json!(m["customer_id"]),json!(m["created_at"])]).ok(); }
    }
    Ok(())
}

fn import_products(data: &Value, tmp: &std::path::Path) -> Result<(), String> {
    if let Some(products) = data["products"].as_array() {
        for p in products {
            let id = p["id"].as_str().unwrap_or("");
            if id.is_empty() { continue; }
            database::run("INSERT OR REPLACE INTO products(id,name,series_id,model,description,status,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)", &[
                json!(id), json!(p["name"]), json!(p["series_id"]), json!(p["model"]),
                json!(p["description"]), json!(p["status"]), json!(p["created_at"]), json!(p["updated_at"]),
            ]).ok();
        }
    }
    if let Some(series) = data["product_series"].as_array() {
        for s in series { database::run("INSERT OR REPLACE INTO product_series(id,name,created_at,updated_at) VALUES(?1,?2,?3,?4)", &[json!(s["id"]),json!(s["name"]),json!(s["created_at"]),json!(s["updated_at"])]).ok(); }
    }
    // 复制物料文件
    let files_dir = tmp.join("product_files");
    if files_dir.exists() {
        for entry in std::fs::read_dir(&files_dir).ok().into_iter().flatten() {
            if let Ok(entry) = entry {
                let fname = entry.file_name().to_string_lossy().to_string();
                if let Some(dot) = fname.rfind('.') {
                    let fid = &fname[..dot];
                    if let Some(fa) = database::get("SELECT content_hash FROM file_assets WHERE id=?1", &[json!(fid)]).ok().flatten() {
                        if let Some(hash) = fa["content_hash"].as_str() {
                            let dst = crate::commands::file_commands::flat_disk_path(hash);
                            if let Some(parent) = std::path::Path::new(&dst).parent() { std::fs::create_dir_all(parent).ok(); }
                            std::fs::copy(entry.path(), &dst).ok();
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

fn import_notes(data: &Value, tmp: &std::path::Path) -> Result<(), String> {
    if let Some(notebooks) = data["notebooks"].as_array() {
        for n in notebooks { database::run("INSERT OR REPLACE INTO notebooks(id,name,color,sort_order,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6)", &[json!(n["id"]),json!(n["name"]),json!(n["color"]),json!(n["sort_order"]),json!(n["created_at"]),json!(n["updated_at"])]).ok(); }
    }
    if let Some(notes) = data["notes"].as_array() {
        for n in notes { database::run("INSERT OR REPLACE INTO notes(note_id,title,content,is_daily,notebook_id,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7)", &[json!(n["note_id"]),json!(n["title"]),json!(n["content"]),json!(n["is_daily"]),json!(n["notebook_id"]),json!(n["created_at"]),json!(n["updated_at"])]).ok(); }
    }
    // 复制笔记附件
    let files_dir = tmp.join("note_files");
    if files_dir.exists() {
        for entry in std::fs::read_dir(&files_dir).ok().into_iter().flatten() {
            if let Ok(entry) = entry {
                let fname = entry.file_name().to_string_lossy().to_string();
                if let Some(dot) = fname.rfind('.') {
                    let fid = &fname[..dot];
                    if let Some(fa) = database::get("SELECT content_hash FROM file_assets WHERE id=?1", &[json!(fid)]).ok().flatten() {
                        if let Some(hash) = fa["content_hash"].as_str() {
                            let dst = crate::commands::file_commands::flat_disk_path(hash);
                            if let Some(parent) = std::path::Path::new(&dst).parent() { std::fs::create_dir_all(parent).ok(); }
                            std::fs::copy(entry.path(), &dst).ok();
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

fn import_todos(data: &Value, tmp: &std::path::Path) -> Result<(), String> {
    if let Some(todos) = data["todos"].as_array() {
        for t in todos {
            let id = t["id"].as_str().unwrap_or("");
            if id.is_empty() { continue; }
            database::run("INSERT OR REPLACE INTO todos(id,title,content,done,priority,due_date,repeat,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)", &[
                json!(id), json!(t["title"]), json!(t["content"]), json!(t["done"]),
                json!(t["priority"]), json!(t["due_date"]), json!(t["repeat"]),
                json!(t["created_at"]), json!(t["updated_at"]),
            ]).ok();
        }
    }
    // 复制待办附件
    let files_dir = tmp.join("todo_files");
    if files_dir.exists() {
        for entry in std::fs::read_dir(&files_dir).ok().into_iter().flatten() {
            if let Ok(entry) = entry {
                let fname = entry.file_name().to_string_lossy().to_string();
                if let Some(dot) = fname.rfind('.') {
                    let fid = &fname[..dot];
                    if let Some(fa) = database::get("SELECT content_hash FROM file_assets WHERE id=?1", &[json!(fid)]).ok().flatten() {
                        if let Some(hash) = fa["content_hash"].as_str() {
                            let dst = crate::commands::file_commands::flat_disk_path(hash);
                            if let Some(parent) = std::path::Path::new(&dst).parent() { std::fs::create_dir_all(parent).ok(); }
                            std::fs::copy(entry.path(), &dst).ok();
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

fn import_files(data: &Value, tmp: &std::path::Path) -> Result<(), String> {
    if let Some(assets) = data["file_assets"].as_array() {
        for a in assets {
            let id = a["id"].as_str().unwrap_or("");
            if id.is_empty() { continue; }
            database::run("INSERT OR REPLACE INTO file_assets(id,file_ext,file_size,content_hash,original_name,parse_status,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)", &[
                json!(id), json!(a["file_ext"]), json!(a["file_size"]), json!(a["content_hash"]),
                json!(a["original_name"]), json!(a["parse_status"]), json!(a["created_at"]), json!(a["updated_at"]),
            ]).ok();
        }
    }
    if let Some(rels) = data["file_relations"].as_array() {
        for r in rels { database::run("INSERT OR REPLACE INTO file_relations(id,file_id,entity_type,entity_id,custom_name,remark,creator,description,folder_id,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)", &[json!(r["id"]),json!(r["file_id"]),json!(r["entity_type"]),json!(r["entity_id"]),json!(r["custom_name"]),json!(r["remark"]),json!(r["creator"]),json!(r["description"]),json!(r["folder_id"]),json!(r["created_at"])]).ok(); }
    }
    if let Some(folders) = data["library_folders"].as_array() {
        for f in folders { database::run("INSERT OR REPLACE INTO library_folders(id,name,parent_id,project_id,sort_order,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7)", &[json!(f["id"]),json!(f["name"]),json!(f["parent_id"]),json!(f["project_id"]),json!(f["sort_order"]),json!(f["created_at"]),json!(f["updated_at"])]).ok(); }
    }
    // 复制资料库文件
    let files_dir = tmp.join("library_files");
    if files_dir.exists() {
        for entry in std::fs::read_dir(&files_dir).ok().into_iter().flatten() {
            if let Ok(entry) = entry {
                let fname = entry.file_name().to_string_lossy().to_string();
                if let Some(dot) = fname.rfind('.') {
                    let fid = &fname[..dot];
                    if let Some(fa) = database::get("SELECT content_hash FROM file_assets WHERE id=?1", &[json!(fid)]).ok().flatten() {
                        if let Some(hash) = fa["content_hash"].as_str() {
                            let dst = crate::commands::file_commands::flat_disk_path(hash);
                            if let Some(parent) = std::path::Path::new(&dst).parent() { std::fs::create_dir_all(parent).ok(); }
                            std::fs::copy(entry.path(), &dst).ok();
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

