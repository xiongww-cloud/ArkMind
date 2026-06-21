use serde_json::{json, Value};
use std::io::Write;

/// 导出全部或指定域数据，写入 save_path
#[tauri::command]
pub async fn export_data(domains: Vec<String>, save_path: String) -> Result<Value, String> {
    let tmp = std::env::temp_dir().join(format!("ark_fullexport_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let mut exported_domains: Vec<String> = Vec::new();

    for domain in &domains {
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
                domain_data = json!({
                    "projects": projects, "solutions": solutions,
                    "solution_versions": solution_versions, "solution_items": solution_items,
                    "project_dynamics": dynamics, "channel_partners": channels,
                    "channel_contacts": channel_contacts, "vendor_partners": vendors,
                    "vendor_contacts": vendor_contacts,
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
                let file_assets = crate::database::all("SELECT * FROM file_assets", &[]).unwrap_or_default();
                let file_relations = crate::database::all(
                    "SELECT * FROM file_relations WHERE entity_type='library'", &[]
                ).unwrap_or_default();
                let folders = crate::database::all("SELECT * FROM library_folders", &[]).unwrap_or_default();
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
    Ok(json!({"success": true}))
}

#[tauri::command] pub async fn import_data(_data: Value) -> Result<Value, String> { Ok(json!({"success":true})) }
#[tauri::command] pub async fn db_export_raw() -> Result<Value, String> { Ok(json!({"success":true})) }
#[tauri::command] pub async fn products_export() -> Result<Value, String> { Ok(json!({"success":true})) }
#[tauri::command] pub async fn products_export_file() -> Result<Value, String> { Ok(json!({"success":true})) }
#[tauri::command] pub async fn products_import(_data: Value) -> Result<Value, String> { Ok(json!({"success":true})) }
#[tauri::command] pub async fn products_import_file(_file_path: String) -> Result<Value, String> { Ok(json!({"success":true})) }
