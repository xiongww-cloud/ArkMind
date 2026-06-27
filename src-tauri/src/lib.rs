//! Arkmind - Tauri v2 backend
#![allow(dead_code)]

mod commands;
mod crypto;
mod database;
mod license;
mod oss;
mod services;
mod time_utils;

use database::db_path;
use std::path::PathBuf;
use tauri::{Manager, Emitter};

fn app_data_dir(app: &tauri::AppHandle) -> PathBuf {
    app.path().app_data_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn upgrade_fts5_triggers_on_startup() {
    let result = database::with_db(|conn| {
        let exists: bool = conn.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='content_fts'", [], |r| r.get::<_,i64>(0)).map(|c| c>0).unwrap_or(false);
        if !exists { return Ok(()); }
        conn.execute_batch("DROP TRIGGER IF EXISTS fts_notes_insert; DROP TRIGGER IF EXISTS fts_notes_update; DROP TRIGGER IF EXISTS fts_notes_delete; CREATE TRIGGER fts_notes_insert AFTER INSERT ON notes BEGIN INSERT INTO content_fts(doc_id,source_table,title,content) VALUES(new.note_id,'notes',new.title,new.content); END; CREATE TRIGGER fts_notes_update AFTER UPDATE ON notes BEGIN DELETE FROM content_fts WHERE doc_id=old.note_id AND source_table='notes'; INSERT INTO content_fts(doc_id,source_table,title,content) VALUES(new.note_id,'notes',new.title,new.content); END; CREATE TRIGGER fts_notes_delete AFTER DELETE ON notes BEGIN DELETE FROM content_fts WHERE doc_id=old.note_id AND source_table='notes'; END;").ok();
        Ok(())
    });
    match result { Ok(()) => log::info!("FTS5 triggers upgraded"), Err(e) => log::warn!("FTS5 trigger upgrade failed: {}", e) }
}

fn migrate_fix_is_daily_corruption() {
    let r = database::with_db(|conn| {
        let nb = conn.execute("UPDATE notes SET is_daily=0 WHERE is_daily=1 AND notebook_id IS NOT NULL", [])?;
        let proj = conn.execute("UPDATE notes SET is_daily=0 WHERE is_daily=1 AND note_id IN (SELECT DISTINCT note_id FROM note_project_mapping)", [])?;
        Ok((nb,proj))
    });
    if let Ok((nb,proj)) = r { if nb>0||proj>0 { log::info!("is_daily fix: notebook={} project={}", nb, proj); } }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_opener::init())
        .register_asynchronous_uri_scheme_protocol("pms", move |_ctx, request, responder| {
            let uri = request.uri().to_string();
            let path = uri.strip_prefix("pms://").unwrap_or(&uri);
            let (_host, id) = if let Some(p) = path.find('/') { (&path[..p], &path[p+1..]) } else { (path, "") };
            let file_path: Option<String> =
                database::get("SELECT content_hash FROM file_relations fr INNER JOIN file_assets fa ON fa.id = fr.file_id WHERE fr.id=?", &[serde_json::json!(id)]).ok().flatten()
                    .and_then(|r| r["content_hash"].as_str().filter(|h| !h.is_empty()).map(|h| crate::commands::file_commands::flat_disk_path(h)));
            let resp = match file_path {
                Some(p) if !p.is_empty() => {
                    let path = std::path::PathBuf::from(&p);
                    if !path.exists() { tauri::http::Response::builder().status(404).body("File Missing".as_bytes().to_vec()) }
                    else {
                        let buf = std::fs::read(&path).unwrap_or_default();
                        let mime = match path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase().as_str() {
                            "pdf"=>"application/pdf", "docx"=>"application/vnd.openxmlformats-officedocument.wordprocessingml.document", "xlsx"=>"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet", "png"=>"image/png", "jpg"|"jpeg"=>"image/jpeg", "gif"=>"image/gif", "webp"=>"image/webp", _=>"application/octet-stream"
                        };
                        tauri::http::Response::builder().status(200).header("Content-Type", mime).header("Access-Control-Allow-Origin","pms://localhost").body(buf)
                    }
                }
                _ => tauri::http::Response::builder().status(404).body("Not Found".as_bytes().to_vec())
            };
            let _ = match resp { Ok(r) => responder.respond(r), Err(_) => {} };
        })
        .setup(|app| {
            // 开发环境下窗口启动后自动获取焦点
            #[cfg(debug_assertions)]
            {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.set_focus();
                }
            }
            let data_dir = app_data_dir(&app.handle());
            let db_file = db_path(&data_dir);
            if let Err(e) = database::init_database(&db_file) { eprintln!("DB init failed: {}", e); return Err(Box::new(e)); }
            log::info!("DB initialized: {:?}", db_file);

            // Window maximize event
            {
                let handle = app.handle().clone();
                if let Some(win) = app.get_webview_window("main") {
                    let was = std::cell::Cell::new(win.is_maximized().unwrap_or(false));
                    win.on_window_event(move |ev| {
                        if let tauri::WindowEvent::Resized(_) = ev {
                            if let Some(w) = handle.get_webview_window("main") {
                                let is = w.is_maximized().unwrap_or(false);
                                if is != was.get() { let _ = w.emit("window:maximizeChange", is); was.set(is); }
                            }
                        }
                    });
                }
            }
            migrate_fix_is_daily_corruption();
            commands::file_commands::cleanup_preview_links();
            if database::check_license_premium() { upgrade_fts5_triggers_on_startup(); }

            // Start KnowledgePipeline
            std::thread::spawn(|| {
                std::thread::sleep(std::time::Duration::from_secs(2));
                services::knowledge_pipeline::start();
                crate::services::cleanup_pipeline::start();
            });

            // Start independent Tag Pipeline (reads from source_data)
            std::thread::spawn(|| {
                std::thread::sleep(std::time::Duration::from_secs(3));
                // services::ai_tag::start();  // 已禁用
            });

            // Start File Ingestion Worker (异步文件解析：智能路由 + 严格时序 + 状态广播)
            let ingestion_handle = app.handle().clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(4));
                services::file_ingestion::start(ingestion_handle);
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // License
            commands::license_status, commands::license_activate, commands::machine_code,
            // Projects
            commands::project_list, commands::project_get, commands::project_create, commands::project_update, commands::project_delete,
            commands::project_dynamics_add, commands::project_dynamics_list, commands::project_dynamics_list_all, commands::project_dynamics_delete, commands::project_dynamics_edit, commands::project_update_status, commands::project_archive,
            commands::project_import, commands::project_import_file, commands::project_export, commands::project_export_files, commands::projects_batch_export,
            // Products
            commands::product_series_list, commands::product_series_create, commands::product_series_update, commands::product_series_delete, commands::product_series_update_sort_order,
            commands::product_list, commands::product_create, commands::product_update, commands::product_update_sort_orders, commands::product_delete,
            commands::product_get_tags, commands::product_add_tag_mapping, commands::product_remove_tag_mapping,
            // Files
            commands::file_list, commands::file_upload, commands::file_delete,
            commands::file_open, commands::file_download, commands::file_get_path, commands::file_has_oss_files, commands::file_migrate_files,
            commands::file_update_meta, commands::file_cleanup_if_orphan,
            commands::file_get_tags, commands::file_add_tag_mapping, commands::file_remove_tag_mapping,
            commands::file_list_by_product, commands::file_open_product, commands::file_download_product,
            commands::file_preview, commands::file_export, commands::file_export_to_path, commands::file_read_bytes,
            commands::file_list_global,
            commands::file_move, commands::file_tree,
            commands::file_folder_create, commands::file_folder_rename, commands::file_folder_delete, commands::file_folder_move, commands::file_folder_reorder,
            commands::file_add_note_mapping, commands::file_remove_note_mapping, commands::file_list_by_note,
            commands::file_add_todo_mapping, commands::file_remove_todo_mapping, commands::file_list_by_todo,
            commands::file_delete_from_domain,
            commands::file_list_all, commands::file_move_to_folder,
            commands::file_unlink_from_product, commands::file_get_product_associations, commands::file_update_product_associations,
            // Notes
            commands::notes_list_all, commands::notes_list_daily, commands::notes_list_by_project, commands::notes_list_orphan, commands::notes_list_trash, commands::notes_list_archive,
            commands::notes_create, commands::notes_update, commands::notes_delete, commands::notes_restore, commands::notes_permanent_delete,
            commands::notes_list_tags, commands::notes_create_tag, commands::notes_search_tags, commands::notes_list_tags_by_type,
            commands::notes_generate_ai_tags, commands::notes_rebuild_fts, commands::notes_upgrade_fts_triggers,
            commands::notes_get_inherited_tags, commands::notes_get_project_knowledge_graph, commands::notes_semantic_recall,
            commands::notes_list_tag_mappings, commands::notes_add_tag_mapping, commands::notes_remove_tag_mapping, commands::notes_count_by_notebook,
            // Notebooks
            commands::notebook_list, commands::notebook_create, commands::notebook_update, commands::notebook_delete,
            // Partners
            commands::channel_list, commands::channel_create, commands::channel_update, commands::channel_delete,
            commands::channel_contact_create, commands::channel_contact_update, commands::channel_contact_delete,
            commands::vendor_list, commands::vendor_create, commands::vendor_update, commands::vendor_delete,
            commands::vendor_contact_create, commands::vendor_contact_update, commands::vendor_contact_delete,
            // Solutions
            commands::solution_list, commands::solution_create, commands::solution_rename, commands::solution_delete,
            commands::solution_version_list, commands::solution_version_create, commands::solution_version_update_meta,
            commands::solution_version_finalize, commands::solution_version_save_draft, commands::solution_version_delete,
            // Storage
            commands::storage_get_config, commands::storage_update_config, commands::oss_test_connection,
            commands::storage_migrate, commands::storage_migrate_progress, commands::storage_default_path,
            // KB
            commands::kb_get_config, commands::kb_update_config, commands::kb_import_file, commands::kb_save_note,
            commands::kb_search, commands::kb_delete, commands::kb_list_documents, commands::kb_chat, commands::kb_agent_chat, commands::kb_save_sessions, commands::kb_load_sessions, commands::kb_delete_session,
            commands::kb_diagnose, commands::kb_search_debug, commands::kb_data_browse, commands::kb_browse_parents, commands::kb_browse_vectors, commands::kb_inspect_fts5,
            commands::kb_fts5_inspector, commands::kb_get_table_chunks, commands::execute_production_rag,
            commands::kb_test_embedding, commands::kb_test_chat,
            commands::ext_tool_search_knowledge_base, commands::ext_tool_get_document_page,
            commands::ext_tool_find_related_knowledge, commands::ext_tool_search_by_tag,
            commands::ext_tool_get_project_context,
            commands::kb_memory_objects, commands::kb_project_note_ids, commands::kb_memory_parents,
            commands::kb_memory_heatmap, commands::kb_memory_dashboard_stats,
            commands::kb_pipeline_debug,
            commands::ext_tool_translate_filter, commands::ext_tool_search_filtered,
            // AI sync
            commands::ai_sync_status, commands::ai_sync_trigger, commands::ai_sync_diagnose, commands::ai_sync_reset,
            commands::ai_sync_retry_failed, commands::cleanup_retry_failed, commands::ai_sync_force_resync, commands::ai_sync_diagnose_failed_files,
            commands::knowledge_pipeline_status,
            commands::pipeline_debug_status, commands::pipeline_clear_knowledge, commands::pipeline_clear_vectors, commands::pipeline_clear_all, commands::pipeline_clear_tag_pipeline, commands::pipeline_resync, commands::pipeline_browse_source, commands::pipeline_get_source_content, commands::pipeline_retag_all,
            // Assets
            commands::assets_search, commands::assets_search_fts5, commands::assets_list,
            commands::asset_sync_status, commands::asset_sync_trigger, commands::asset_diagnose,
            commands::asset_get_latest_version, commands::asset_get_parsed_text, commands::asset_rerun_migration,
            // Tags
            commands::tag_get_state, commands::tag_list_registered, commands::tag_force_resync, commands::tag_tag_single,
            commands::tag_list, commands::tag_create, commands::tag_update, commands::tag_delete, commands::tag_list_categories, commands::tag_stats,
            commands::entity_get_tags, commands::entity_add_tag, commands::entity_remove_tag,
            // Settings
            commands::settings_get, commands::settings_update, commands::get_ai_config, commands::update_ai_config, commands::ai_test_provider, commands::ai_test_embedding,
            commands::user_profile, commands::user_profile_update,
            // Window
            commands::window_minimize, commands::window_toggle_maximize, commands::window_close,
            // Export
            commands::export_data, commands::import_data, commands::db_export_raw,
            commands::products_export, commands::products_export_file, commands::products_import, commands::products_import_file,
            commands::solution_download_excel,
            // Todos
            commands::todos_list, commands::todos_create, commands::todos_update, commands::todos_delete,
            commands::todos_toggle, commands::todos_get,
            // OSS
            commands::oss_raw_diagnose,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Arkmind");
}
