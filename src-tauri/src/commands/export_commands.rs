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
                let partners = crate::database::all("SELECT * FROM partners", &[]).unwrap_or_default();
                let partner_contacts = crate::database::all("SELECT * FROM partner_contacts", &[]).unwrap_or_default();
                let partner_contact_mapping = crate::database::all("SELECT * FROM partner_contact_mapping", &[]).unwrap_or_default();
                let business_relations = crate::database::all("SELECT * FROM business_relations WHERE source_type IN ('CHANNEL','VENDOR','CUSTOMER','CONTACT')", &[]).unwrap_or_default();
                domain_data = json!({
                    "projects": projects, "solutions": solutions,
                    "solution_versions": solution_versions, "solution_items": solution_items,
                    "project_dynamics": dynamics, "partners": partners,
                    "partner_contacts": partner_contacts,
                    "partner_contact_mapping": partner_contact_mapping,
                    "business_relations": business_relations,
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
            "contacts" => {
                let partners = crate::database::all("SELECT * FROM partners", &[]).unwrap_or_default();
                let partner_contacts = crate::database::all("SELECT * FROM partner_contacts", &[]).unwrap_or_default();
                let partner_contact_mapping = crate::database::all("SELECT * FROM partner_contact_mapping", &[]).unwrap_or_default();
                let business_relations = crate::database::all(
                    "SELECT * FROM business_relations WHERE source_type IN ('CHANNEL','VENDOR','CUSTOMER','CONTACT') OR target_type IN ('CHANNEL','VENDOR','CUSTOMER','CONTACT')", &[]
                ).unwrap_or_default();
                domain_data = json!({
                    "partners": partners,
                    "partner_contacts": partner_contacts,
                    "partner_contact_mapping": partner_contact_mapping,
                    "business_relations": business_relations,
                });
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
    add_dir_to_zip(&mut zip_writer, &tmp, &tmp, opts).map_err(|e| e)?;
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

/// 一键备份：打包所有数据到单个 ZIP 文件
#[tauri::command]
pub async fn backup_all(save_path: String, app_handle: tauri::AppHandle) -> Result<Value, String> {
    let handle = app_handle.clone();
    tokio::task::spawn_blocking(move || {
        eprintln!("[备份] 开始一键备份 -> {}", save_path);
        let tmp = std::env::temp_dir().join(format!("arkmind_backup_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;

        // 1. SQLite 数据库
        let _ = handle.emit("backup:progress", json!({"stage":"exporting","step":1,"total":4,"name":"数据库"}));
        eprintln!("[备份] 步骤 1/4: 复制数据库...");
        // ★ 先强制 WAL checkpoint，确保所有事务落盘，避免备份不一致
        let _ = crate::database::with_db(|conn| conn.pragma_update(None, "wal_checkpoint", "TRUNCATE"));
        let db_dir = tmp.join("database");
        std::fs::create_dir_all(&db_dir).map_err(|e| e.to_string())?;
        let db_path = crate::get_data_dir().join("arkmind.db");
        if db_path.exists() {
            let db_size = std::fs::metadata(&db_path).map(|m| m.len()).unwrap_or(0);
            std::fs::copy(&db_path, db_dir.join("arkmind.db")).map_err(|e| e.to_string())?;
            eprintln!("[备份] 数据库已复制 ({} KB)", db_size / 1024);
        } else {
            eprintln!("[备份] 数据库不存在，跳过");
        }
        let wal_path = db_path.with_extension("db-wal");
        let shm_path = db_path.with_extension("db-shm");
        if wal_path.exists() { std::fs::copy(&wal_path, db_dir.join("arkmind.db-wal")).ok(); }
        if shm_path.exists() { std::fs::copy(&shm_path, db_dir.join("arkmind.db-shm")).ok(); }

        // 2. 上传文件
        let _ = handle.emit("backup:progress", json!({"stage":"exporting","step":2,"total":4,"name":"文件"}));
        eprintln!("[备份] 步骤 2/4: 复制上传文件...");
        let files_dir = tmp.join("files");
        let files_path = crate::commands::file_commands::storage_base();
        let mut total_files = 0;
        
        // 检查 AppData 路径
        if std::path::Path::new(&files_path).exists() {
            let count = copy_dir_recursive(std::path::Path::new(&files_path), &files_dir).unwrap_or(0);
            if count > 0 {
                eprintln!("[备份] 从 AppData 复制了 {} 个文件", count);
                total_files += count;
            }
        }
        
        // 检查 exe 相对路径（旧版本存储位置）
        if total_files == 0 {
            if let Ok(exe_path) = std::env::current_exe() {
                if let Some(parent) = exe_path.parent() {
                    let old_path = parent.join("files");
                    if old_path.exists() {
                        let count = copy_dir_recursive(&old_path, &files_dir).unwrap_or(0);
                        if count > 0 {
                            eprintln!("[备份] 从旧路径复制了 {} 个文件: {}", count, old_path.display());
                            total_files += count;
                        }
                    }
                }
            }
        }
        
        // 检查 target/debug/files 和 target/release/files
        if total_files == 0 {
            let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
            for subdir in &["debug/files", "release/files"] {
                let target_path = manifest_dir.join("target").join(subdir);
                if target_path.exists() {
                    let count = copy_dir_recursive(&target_path, &files_dir).unwrap_or(0);
                    if count > 0 {
                        eprintln!("[备份] 从 target/{} 复制了 {} 个文件", subdir, count);
                        total_files += count;
                    }
                }
            }
        }
        
        if total_files == 0 {
            eprintln!("[备份] 未找到任何上传文件");
        }

        // 3. 向量数据库
        let _ = handle.emit("backup:progress", json!({"stage":"exporting","step":3,"total":4,"name":"向量库"}));
        eprintln!("[备份] 步骤 3/4: 复制向量数据库...");
        let vectordb_path = crate::get_data_dir().join("lancedb");
        let vectordb_dir = tmp.join("vectordb");
        if vectordb_path.exists() {
            let count = copy_dir_recursive(&vectordb_path, &vectordb_dir).unwrap_or(0);
            eprintln!("[备份] 向量数据库已复制 ({} 个文件)", count);
        } else {
            eprintln!("[备份] 向量数据库不存在，跳过");
        }

        // 4. manifest.json
        let manifest = json!({
            "version": "1.0",
            "app_version": env!("CARGO_PKG_VERSION"),
            "timestamp": chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            "contents": ["database", "files", "vectordb"]
        });
        std::fs::write(tmp.join("manifest.json"), serde_json::to_string_pretty(&manifest).unwrap()).ok();

        // 5. 打包为 ZIP
        let _ = handle.emit("backup:progress", json!({"stage":"exporting","step":4,"total":4,"name":"打包"}));
        eprintln!("[备份] 步骤 4/4: 打包 ZIP 文件...");
        let zip_path = std::path::PathBuf::from(&save_path);
        create_zip_archive(&tmp, &zip_path).map_err(|e| e.to_string())?;
        let zip_size = std::fs::metadata(&zip_path).map(|m| m.len()).unwrap_or(0);
        eprintln!("[备份] 完成! 备份文件: {} ({} KB)", save_path, zip_size / 1024);

        // 6. 清理临时目录
        std::fs::remove_dir_all(&tmp).ok();

        let _ = handle.emit("backup:progress", json!({"stage":"done"}));
        Ok(json!({"success": true, "path": save_path}))
    }).await.map_err(|e| e.to_string())?
}

/// 一键恢复：从 ZIP 文件恢复所有数据（完成后自动重启应用）
#[tauri::command]
pub async fn restore_all(backup_path: String, app_handle: tauri::AppHandle) -> Result<Value, String> {
    let handle = app_handle.clone();
    tokio::task::spawn_blocking(move || {
        eprintln!("[恢复] 开始一键恢复 <- {}", backup_path);
        let zip_path = std::path::PathBuf::from(&backup_path);
        if !zip_path.exists() { return Err("备份文件不存在".to_string()); }

        let tmp = std::env::temp_dir().join(format!("arkmind_restore_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;

        // 1. 解压 ZIP
        let _ = handle.emit("restore:progress", json!({"stage":"exporting","step":1,"total":4,"name":"解压"}));
        eprintln!("[恢复] 步骤 1/4: 解压备份文件...");
        let file = std::fs::File::open(&zip_path).map_err(|e| e.to_string())?;
        let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
        let file_count = archive.len();
        archive.extract(&tmp).map_err(|e| e.to_string())?;
        eprintln!("[恢复] 解压完成 ({} 个文件)", file_count);

        // 2. 验证 manifest
        let manifest_path = tmp.join("manifest.json");
        if !manifest_path.exists() { return Err("无效的备份文件：缺少 manifest.json".to_string()); }
        let manifest_str = std::fs::read_to_string(&manifest_path).map_err(|e| e.to_string())?;
        eprintln!("[恢复] 备份信息: {}", manifest_str.lines().next().unwrap_or(""));

        // 3. 恢复 SQLite 数据库
        let _ = handle.emit("restore:progress", json!({"stage":"exporting","step":2,"total":4,"name":"数据库"}));
        eprintln!("[恢复] 步骤 2/4: 恢复数据库...");
        let db_dir = tmp.join("database");
        if db_dir.exists() {
            let db_dest = crate::get_data_dir().join("arkmind.db");
            // ★ 先关闭数据库连接，释放文件句柄（Windows 下覆盖打开中的文件会失败）
            crate::database::close_db();
            // ★ 删除残留的 WAL/SHM，避免旧 WAL 重放到恢复后的数据库
            let wal_dest = db_dest.with_extension("db-wal");
            let shm_dest = db_dest.with_extension("db-shm");
            if wal_dest.exists() { std::fs::remove_file(&wal_dest).ok(); }
            if shm_dest.exists() { std::fs::remove_file(&shm_dest).ok(); }
            if let Some(parent) = db_dest.parent() { std::fs::create_dir_all(parent).ok(); }
            std::fs::copy(db_dir.join("arkmind.db"), &db_dest).map_err(|e| e.to_string())?;
            let wal_src = db_dir.join("arkmind.db-wal");
            let shm_src = db_dir.join("arkmind.db-shm");
            if wal_src.exists() { std::fs::copy(&wal_src, &wal_dest).ok(); }
            if shm_src.exists() { std::fs::copy(&shm_src, &shm_dest).ok(); }
            eprintln!("[恢复] 数据库已恢复到: {}", db_dest.display());
        } else {
            eprintln!("[恢复] 备份中无数据库，跳过");
        }

        // 4. 恢复上传文件
        let _ = handle.emit("restore:progress", json!({"stage":"exporting","step":3,"total":4,"name":"文件"}));
        eprintln!("[恢复] 步骤 3/4: 恢复上传文件...");
        let files_dir = tmp.join("files");
        if files_dir.exists() {
            let files_dest = crate::commands::file_commands::storage_base();
            let dest_path = std::path::PathBuf::from(&files_dest);
            if let Some(parent) = dest_path.parent() { std::fs::create_dir_all(parent).ok(); }
            let count = copy_dir_recursive(&files_dir, &dest_path).unwrap_or(0);
            eprintln!("[恢复] 上传文件已恢复 ({} 个文件)", count);
        } else {
            eprintln!("[恢复] 备份中无上传文件，跳过");
        }

        // 5. 恢复向量数据库
        let _ = handle.emit("restore:progress", json!({"stage":"exporting","step":4,"total":4,"name":"向量库"}));
        eprintln!("[恢复] 步骤 4/4: 恢复向量数据库...");
        let vectordb_dir = tmp.join("vectordb");
        if vectordb_dir.exists() {
            let vectordb_dest = crate::get_data_dir().join("lancedb");
            if let Some(parent) = vectordb_dest.parent() { std::fs::create_dir_all(parent).ok(); }
            let count = copy_dir_recursive(&vectordb_dir, &vectordb_dest).unwrap_or(0);
            eprintln!("[恢复] 向量数据库已恢复 ({} 个文件)", count);
        } else {
            eprintln!("[恢复] 备份中无向量数据库，跳过");
        }

        // 6. 清理临时目录
        std::fs::remove_dir_all(&tmp).ok();
        eprintln!("[恢复] 完成! 应用即将自动重启...");

        let _ = handle.emit("restore:progress", json!({"stage":"done"}));

        // ★ 延迟 1.5 秒后自动重启（让响应先送达前端）
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(1500));
            app_handle.restart();
        });

        Ok(json!({"success": true, "restart": true}))
    }).await.map_err(|e| e.to_string())?
}

/// 递归复制目录，返回复制的文件数量
fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> Result<usize, std::io::Error> {
    if !src.exists() { return Ok(0); }
    std::fs::create_dir_all(dst)?;
    let mut count = 0;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            count += copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            if let Some(parent) = dst_path.parent() { std::fs::create_dir_all(parent)?; }
            std::fs::copy(&src_path, &dst_path)?;
            count += 1;
        }
    }
    Ok(count)
}

/// 创建 ZIP 归档
fn create_zip_archive(src: &std::path::Path, dst: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    let file = std::fs::File::create(dst)?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    add_directory_to_zip(&mut zip, src, src, options)?;
    zip.finish()?;
    Ok(())
}

fn add_directory_to_zip(zip: &mut zip::ZipWriter<std::fs::File>, base: &std::path::Path, current: &std::path::Path, options: zip::write::FileOptions) -> Result<(), Box<dyn std::error::Error>> {
    for entry in std::fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let name = path.strip_prefix(base).map_err(|e| e.to_string())?;
        if path.is_dir() {
            zip.add_directory(name.to_string_lossy(), options)?;
            add_directory_to_zip(zip, base, &path, options)?;
        } else {
            zip.start_file(name.to_string_lossy(), options)?;
            let mut f = std::fs::File::open(&path)?;
            std::io::copy(&mut f, zip)?;
        }
    }
    Ok(())
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
            "contacts" => import_contacts(&data),
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
            database::run("INSERT OR IGNORE INTO projects(id,name,description,status,owner,budget,bid_time,expected_order_date,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)", &[
                json!(id), json!(p["name"]), json!(p["description"]), json!(p["status"]),
                json!(p["owner"]), json!(p["budget"]),
                json!(p["bid_time"]), json!(p["expected_order_date"]),
                json!(p["created_at"]), json!(p["updated_at"]),
            ]).ok();
        }
    }
    // 方案
    if let Some(sols) = data["solutions"].as_array() {
        for s in sols { database::run("INSERT OR IGNORE INTO solutions(id,project_id,name,created_at,updated_at) VALUES(?1,?2,?3,?4,?5)", &[json!(s["id"]),json!(s["project_id"]),json!(s["name"]),json!(s["created_at"]),json!(s["updated_at"])]).ok(); }
    }
    // 方案版本
    if let Some(vers) = data["solution_versions"].as_array() {
        for v in vers { database::run("INSERT OR IGNORE INTO solution_versions(id,solution_id,version,tags,content,creator,status,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)", &[json!(v["id"]),json!(v["solution_id"]),json!(v["version"]),json!(v["tags"]),json!(v["content"]),json!(v["creator"]),json!(v["status"]),json!(v["created_at"]),json!(v["updated_at"])]).ok(); }
    }
    // 方案设备清单
    if let Some(items) = data["solution_items"].as_array() {
        for i in items { database::run("INSERT OR IGNORE INTO solution_items(id,solution_version_id,product_id,product_name,quantity,unit_price,description,device_name,row_type,sort_order,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)", &[json!(i["id"]),json!(i["solution_version_id"]),json!(i["product_id"]),json!(i["product_name"]),json!(i["quantity"]),json!(i["unit_price"]),json!(i["description"]),json!(i["device_name"]),json!(i["row_type"]),json!(i["sort_order"]),json!(i["created_at"]),json!(i["updated_at"])]).ok(); }
    }
    // 动态
    if let Some(dyns) = data["project_dynamics"].as_array() {
        for d in dyns { database::run("INSERT OR IGNORE INTO project_dynamics(id,project_id,type,content,operator,created_at) VALUES(?1,?2,?3,?4,?5,?6)", &[json!(d["id"]),json!(d["project_id"]),json!(d["type"]),json!(d["content"]),json!(d["operator"]),json!(d["created_at"])]).ok(); }
    }
    // 合作伙伴（新格式或旧格式兼容）
    if let Some(arr) = data["partners"].as_array() {
        // 新格式：直接导入 partners 表（address 列，由 company 迁移而来）
        for p in arr {
            database::run("INSERT OR IGNORE INTO partners(id,type,name,phone,address,remark,contact_person,contact_phone,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)", &[
                json!(p["id"]),json!(p["type"]),json!(p["name"]),json!(p["phone"]),
                json!(p["address"]),json!(p["remark"]),json!(p["contact_person"]),json!(p["contact_phone"]),
                json!(p["created_at"]),json!(p["updated_at"])
            ]).ok();
        }
    } else {
        // 旧格式：转换 channel_partners → partners
        if let Some(chs) = data["channel_partners"].as_array() {
            for c in chs {
                database::run("INSERT OR IGNORE INTO partners(id,type,name,phone,address,remark,created_at,updated_at) VALUES(?1,'CHANNEL',?2,?3,?4,?5,?6,?6)", &[
                    json!(c["id"]),json!(c["name"]),json!(c["phone"]),json!(c["company"]),json!(c["remark"]),json!(c["created_at"])
                ]).ok();
            }
        }
        // 旧格式：转换 vendor_partners → partners
        if let Some(vds) = data["vendor_partners"].as_array() {
            for v in vds {
                database::run("INSERT OR IGNORE INTO partners(id,type,name,phone,address,remark,created_at,updated_at) VALUES(?1,'VENDOR',?2,?3,?4,?5,?6,?6)", &[
                    json!(v["id"]),json!(v["name"]),json!(v["phone"]),json!(v["company"]),json!(v["remark"]),json!(v["created_at"])
                ]).ok();
            }
        }
        // 旧格式：客户
        if let Some(custs) = data["customers"].as_array() {
            for c in custs {
                database::run("INSERT OR IGNORE INTO partners(id,type,name,phone,address,remark,contact_person,contact_phone,created_at,updated_at) VALUES(?1,'CUSTOMER',?2,'','',?3,?4,?5,?6,?7)", &[
                    json!(c["id"]),json!(c["name"]),json!(c["remark"]),json!(c["contact_person"]),json!(c["contact_phone"]),json!(c["created_at"]),json!(c["updated_at"])
                ]).ok();
            }
        }
    }
    // 联系人（新格式或旧格式兼容）
    if let Some(arr) = data["partner_contacts"].as_array() {
        for pc in arr {
            database::run("INSERT OR IGNORE INTO partner_contacts(id,name,phone,email,position,remark,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7)", &[
                json!(pc["id"]),json!(pc["name"]),json!(pc["phone"]),json!(pc["email"]),
                json!(pc["position"]),json!(pc["remark"]),json!(pc["created_at"])
            ]).ok();
        }
    } else {
        // 旧格式：转换 channel_contacts → partner_contacts
        if let Some(ccs) = data["channel_contacts"].as_array() {
            for c in ccs {
                database::run("INSERT OR IGNORE INTO partner_contacts(id,name,phone,email,position,remark,created_at) VALUES(?1,?2,?3,?4,'',?5,?6)", &[
                    json!(c["id"]),json!(c["name"]),json!(c["phone"]),json!(c["email"]),
                    json!(c["remark"]),json!(c["created_at"])
                ]).ok();
            }
        }
        // 旧格式：转换 vendor_contacts → partner_contacts
        if let Some(vcs) = data["vendor_contacts"].as_array() {
            for v in vcs {
                database::run("INSERT OR IGNORE INTO partner_contacts(id,name,phone,email,position,remark,created_at) VALUES(?1,?2,?3,?4,'',?5,?6)", &[
                    json!(v["id"]),json!(v["name"]),json!(v["phone"]),json!(v["email"]),
                    json!(v["remark"]),json!(v["created_at"])
                ]).ok();
            }
        }
    }
    // 联系人映射
    if let Some(arr) = data["partner_contact_mapping"].as_array() {
        for m in arr {
            database::run("INSERT OR IGNORE INTO partner_contact_mapping(partner_id,contact_id) VALUES(?1,?2)", &[
                json!(m["partner_id"]),json!(m["contact_id"])
            ]).ok();
        }
    }
    // 业务关联
    if let Some(arr) = data["business_relations"].as_array() {
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
    // 映射表（保留兼容）
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
            database::run("INSERT OR IGNORE INTO products(id,name,series_id,series_name,description,price,standard_specs,full_specs,sort_order,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)", &[
                json!(id), json!(p["name"]), json!(p["series_id"]), json!(p["series_name"]),
                json!(p["description"]), json!(p["price"]), json!(p["standard_specs"]),
                json!(p["full_specs"]), json!(p["sort_order"]),
                json!(p["created_at"]), json!(p["updated_at"]),
            ]).ok();
        }
    }
    if let Some(series) = data["product_series"].as_array() {
        for s in series { database::run("INSERT OR IGNORE INTO product_series(id,name,parent_id,sort_order,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6)", &[json!(s["id"]),json!(s["name"]),json!(s["parent_id"]),json!(s["sort_order"]),json!(s["created_at"]),json!(s["updated_at"])]).ok(); }
    }
    // 物料附件：先恢复 file_assets 行（二进制复制依赖 content_hash 查询）
    if let Some(assets) = data["file_assets"].as_array() {
        for a in assets {
            let id = a["id"].as_str().unwrap_or("");
            if id.is_empty() { continue; }
            database::run("INSERT OR IGNORE INTO file_assets(id,file_ext,file_size,content_hash,original_name,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7)", &[
                json!(id), json!(a["file_ext"]), json!(a["file_size"]), json!(a["content_hash"]),
                json!(a["original_name"]), json!(a["created_at"]), json!(a["updated_at"]),
            ]).ok();
        }
    }
    if let Some(rels) = data["file_relations"].as_array() {
        for r in rels { database::run("INSERT OR IGNORE INTO file_relations(id,file_id,entity_type,entity_id,custom_name,remark,creator,description,folder_id,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)", &[json!(r["id"]),json!(r["file_id"]),json!(r["entity_type"]),json!(r["entity_id"]),json!(r["custom_name"]),json!(r["remark"]),json!(r["creator"]),json!(r["description"]),json!(r["folder_id"]),json!(r["created_at"])]).ok(); }
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
        for n in notebooks { database::run("INSERT OR IGNORE INTO notebooks(id,name,color,sort_order,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6)", &[json!(n["id"]),json!(n["name"]),json!(n["color"]),json!(n["sort_order"]),json!(n["created_at"]),json!(n["updated_at"])]).ok(); }
    }
    if let Some(notes) = data["notes"].as_array() {
        for n in notes { database::run("INSERT OR IGNORE INTO notes(note_id,title,content,is_daily,notebook_id,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7)", &[json!(n["note_id"]),json!(n["title"]),json!(n["content"]),json!(n["is_daily"]),json!(n["notebook_id"]),json!(n["created_at"]),json!(n["updated_at"])]).ok(); }
    }
    // 笔记附件：先恢复 file_assets 行（二进制复制依赖 content_hash 查询）
    if let Some(assets) = data["file_assets"].as_array() {
        for a in assets {
            let id = a["id"].as_str().unwrap_or("");
            if id.is_empty() { continue; }
            database::run("INSERT OR IGNORE INTO file_assets(id,file_ext,file_size,content_hash,original_name,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7)", &[
                json!(id), json!(a["file_ext"]), json!(a["file_size"]), json!(a["content_hash"]),
                json!(a["original_name"]), json!(a["created_at"]), json!(a["updated_at"]),
            ]).ok();
        }
    }
    if let Some(rels) = data["file_relations"].as_array() {
        for r in rels { database::run("INSERT OR IGNORE INTO file_relations(id,file_id,entity_type,entity_id,custom_name,remark,creator,description,folder_id,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)", &[json!(r["id"]),json!(r["file_id"]),json!(r["entity_type"]),json!(r["entity_id"]),json!(r["custom_name"]),json!(r["remark"]),json!(r["creator"]),json!(r["description"]),json!(r["folder_id"]),json!(r["created_at"])]).ok(); }
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
            database::run("INSERT OR IGNORE INTO todos(id,title,content,done,priority,due_date,repeat,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)", &[
                json!(id), json!(t["title"]), json!(t["content"]), json!(t["done"]),
                json!(t["priority"]), json!(t["due_date"]), json!(t["repeat"]),
                json!(t["created_at"]), json!(t["updated_at"]),
            ]).ok();
        }
    }
    // 待办附件：先恢复 file_assets 行（二进制复制依赖 content_hash 查询）
    if let Some(assets) = data["file_assets"].as_array() {
        for a in assets {
            let id = a["id"].as_str().unwrap_or("");
            if id.is_empty() { continue; }
            database::run("INSERT OR IGNORE INTO file_assets(id,file_ext,file_size,content_hash,original_name,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7)", &[
                json!(id), json!(a["file_ext"]), json!(a["file_size"]), json!(a["content_hash"]),
                json!(a["original_name"]), json!(a["created_at"]), json!(a["updated_at"]),
            ]).ok();
        }
    }
    if let Some(rels) = data["file_relations"].as_array() {
        for r in rels { database::run("INSERT OR IGNORE INTO file_relations(id,file_id,entity_type,entity_id,custom_name,remark,creator,description,folder_id,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)", &[json!(r["id"]),json!(r["file_id"]),json!(r["entity_type"]),json!(r["entity_id"]),json!(r["custom_name"]),json!(r["remark"]),json!(r["creator"]),json!(r["description"]),json!(r["folder_id"]),json!(r["created_at"])]).ok(); }
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
            database::run("INSERT OR IGNORE INTO file_assets(id,file_ext,file_size,content_hash,original_name,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7)", &[
                json!(id), json!(a["file_ext"]), json!(a["file_size"]), json!(a["content_hash"]),
                json!(a["original_name"]), json!(a["created_at"]), json!(a["updated_at"]),
            ]).ok();
        }
    }
    if let Some(rels) = data["file_relations"].as_array() {
        for r in rels { database::run("INSERT OR IGNORE INTO file_relations(id,file_id,entity_type,entity_id,custom_name,remark,creator,description,folder_id,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)", &[json!(r["id"]),json!(r["file_id"]),json!(r["entity_type"]),json!(r["entity_id"]),json!(r["custom_name"]),json!(r["remark"]),json!(r["creator"]),json!(r["description"]),json!(r["folder_id"]),json!(r["created_at"])]).ok(); }
    }
    if let Some(folders) = data["library_folders"].as_array() {
        for f in folders { database::run("INSERT OR IGNORE INTO library_folders(id,name,parent_id,project_id,sort_order,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7)", &[json!(f["id"]),json!(f["name"]),json!(f["parent_id"]),json!(f["project_id"]),json!(f["sort_order"]),json!(f["created_at"]),json!(f["updated_at"])]).ok(); }
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


fn import_contacts(data: &Value) -> Result<(), String> {
    // Import partners
    if let Some(arr) = data["partners"].as_array() {
        for p in arr {
            database::run(
                "INSERT OR IGNORE INTO partners(id,type,name,phone,address,remark,contact_person,contact_phone,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                &[
                    p["id"].clone(), p["type"].clone(), p["name"].clone(),
                    p["phone"].as_str().map(|s| json!(s)).unwrap_or(json!("")),
                    p["address"].as_str().map(|s| json!(s)).unwrap_or(json!("")),
                    p["remark"].as_str().map(|s| json!(s)).unwrap_or(json!("")),
                    p["contact_person"].as_str().map(|s| json!(s)).unwrap_or(json!("")),
                    p["contact_phone"].as_str().map(|s| json!(s)).unwrap_or(json!("")),
                    p["created_at"].clone(), p["updated_at"].clone(),
                ],
            ).ok();
        }
    }
    // Import partner_contacts
    if let Some(arr) = data["partner_contacts"].as_array() {
        for pc in arr {
            database::run(
                "INSERT OR IGNORE INTO partner_contacts(id,name,phone,email,position,remark,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",
                &[
                    pc["id"].clone(), pc["name"].clone(),
                    pc["phone"].as_str().map(|s| json!(s)).unwrap_or(json!("")),
                    pc["email"].as_str().map(|s| json!(s)).unwrap_or(json!("")),
                    pc["position"].as_str().map(|s| json!(s)).unwrap_or(json!("")),
                    pc["remark"].as_str().map(|s| json!(s)).unwrap_or(json!("")),
                    pc["created_at"].clone(),
                ],
            ).ok();
        }
    }
    // Import partner_contact_mapping
    if let Some(arr) = data["partner_contact_mapping"].as_array() {
        for pcm in arr {
            database::run(
                "INSERT OR IGNORE INTO partner_contact_mapping(partner_id,contact_id,created_at) VALUES(?1,?2,?3)",
                &[pcm["partner_id"].clone(), pcm["contact_id"].clone(), pcm["created_at"].clone()],
            ).ok();
        }
    }
    // Import business_relations
    if let Some(arr) = data["business_relations"].as_array() {
        for br in arr {
            database::run(
                "INSERT OR IGNORE INTO business_relations(id,source_type,source_id,target_type,target_id,relation_type,metadata,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                &[
                    br["id"].clone(), br["source_type"].clone(), br["source_id"].clone(),
                    br["target_type"].clone(), br["target_id"].clone(), br["relation_type"].clone(),
                    br["metadata"].as_str().map(|s| json!(s)).unwrap_or(json!("{}")),
                    br["created_at"].clone(), br["updated_at"].clone(),
                ],
            ).ok();
        }
    }
    Ok(())
}
