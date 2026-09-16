//! Arkmind - Tauri v2 backend
#![recursion_limit = "256"]

mod commands;
mod crypto;
mod database;
mod license;
mod services;
mod time_utils;

/// 获取应用数据目录
/// - macOS: ~/Arkmind
/// - Windows: %APPDATA%/com.arkmind.app
pub fn get_data_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join("Arkmind")
    }
    #[cfg(not(target_os = "macos"))]
    {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("com.arkmind.app")
    }
}

use database::db_path;
use std::path::PathBuf;
use tauri::{Manager, Emitter};
use tauri::tray::{TrayIconBuilder, MouseButton, MouseButtonState, TrayIconEvent};
use tauri::menu::{MenuBuilder, MenuItemBuilder};

fn app_data_dir(_app: &tauri::AppHandle) -> PathBuf {
    get_data_dir()
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
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // 二次启动：恢复并聚焦主窗口（主窗口可能被关闭隐藏）
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.show();
                let _ = win.set_focus();
            }
        }))
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // ★ 初始化防抖物化调度器（需 tokio runtime 就绪）
            once_cell::sync::Lazy::force(&services::debounced_sync::SCHEDULER);
            // Windows: titleBarStyle Overlay 仅 macOS 生效，需运行时移除原生标题栏（自定义控件在 Header.vue）
            #[cfg(not(target_os = "macos"))]
            {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.set_decorations(false);
                }
            }
            // 开发环境下窗口启动后自动获取焦点
            #[cfg(debug_assertions)]
            {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.set_focus();
                }
            }
            // ★ 初始化日志系统
            let log_state = services::log_collector::init_logger(1000)
                .unwrap_or_else(|e| {
                    eprintln!("Logger init failed: {}", e);
                    services::log_collector::LogState::new()
                });
            app.manage(log_state);

            let data_dir = app_data_dir(&app.handle());
            let db_file = db_path(&data_dir);
            if let Err(e) = database::init_database(&db_file) { eprintln!("DB init failed: {}", e); return Err(Box::new(e)); }
            log::info!("DB initialized: {:?}", db_file);

            // Initialize business_relations table
            // ★ 若检测并修复了 INTEGER id 坏表（旧 schema 导致所有关联写入静默失败），
            //    从现存业务映射表回填恢复全部关联 + 重建方案图谱边
            let br_rebuilt = services::business_relations::init();
            if br_rebuilt {
                log::info!("[业务关联] 检测到坏表修复，开始从业务数据重建关联...");
                services::business_relations::backfill();
                if let Ok(projects) = database::all("SELECT id FROM projects", &[]) {
                    for p in &projects {
                        if let Some(pid) = p["id"].as_str() {
                            let _ = commands::project_commands::sync_project_products(pid);
                            commands::project_commands::sync_solution_graph(pid);
                        }
                    }
                    log::info!("[业务关联] 方案图谱重建完成（{} 个项目）", projects.len());
                }
            }

            // 清理过期 compressed 消息：每 session 最多保留 50 条，清理空 session
            commands::kb::cleanup_chat_messages(50);

            // ★ 启动时清理过期许可证
            license::cleanup_expired_licenses();

            // ★ 启动时初始化试用期（首次启动记录开始时间）
            license::init_trial();

            // Window events
            {
                let handle = app.handle().clone();
                if let Some(win) = app.get_webview_window("main") {
                    let was = std::cell::Cell::new(win.is_maximized().unwrap_or(false));
                    win.on_window_event(move |ev| {
                        match ev {
                            tauri::WindowEvent::Resized(_) => {
                                if let Some(w) = handle.get_webview_window("main") {
                                    let is = w.is_maximized().unwrap_or(false);
                                    if is != was.get() { let _ = w.emit("window:maximizeChange", is); was.set(is); }
                                }
                            }
                            // 关闭窗口时隐藏而非退出（macOS 保留在 Dock）
                            tauri::WindowEvent::CloseRequested { api, .. } => {
                                if let Some(w) = handle.get_webview_window("main") {
                                    let _ = w.hide();
                                }
                                api.prevent_close();
                            }
                            _ => {}
                        }
                    });
                }
            }

            migrate_fix_is_daily_corruption();
            commands::file_commands::cleanup_preview_links();
            if database::check_license_premium() { upgrade_fts5_triggers_on_startup(); }

            // ★ 启动时根据开关显示精灵
            {
                let sprite_enabled = commands::get_sprite_enabled_sync().unwrap_or(false);
                if sprite_enabled {
                    if let Some(win) = app.get_webview_window("sprite") {
                        // 恢复位置
                        if let Ok(pos) = commands::get_sprite_position_sync() {
                            let x = pos["x"].as_f64().unwrap_or(800.0);
                            let y = pos["y"].as_f64().unwrap_or(400.0);
                            let _ = win.set_position(tauri::LogicalPosition::new(x, y));
                        }
                        let _ = win.set_always_on_top(true);
                        let _ = win.show();
                        let _ = win.set_focus();
                    } else {
                        // 配置窗口未创建时手动创建（macOS 兼容）
                        let app_handle = app.handle().clone();
                        tauri::async_runtime::spawn(async move {
                            let _ = commands::window_commands::create_sprite_window(app_handle).await;
                        });
                    }
                }
            }

            // ★ 系统托盘图标（macOS 不需要）
            #[cfg(not(target_os = "macos"))]
            {
            let sprite_item = MenuItemBuilder::with_id("sprite", "显示/隐藏精灵").build(app)?;
            let quit_item = MenuItemBuilder::with_id("quit", "退出").build(app)?;
            let menu = MenuBuilder::new(app).items(&[&sprite_item, &quit_item]).build()?;
            let _tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().cloned().unwrap())
                .menu(&menu)
                .show_menu_on_left_click(false)
                .tooltip("Arkmind")
                .on_menu_event(move |app, event| {
                    match event.id().as_ref() {
                        "sprite" => {
                            let app_handle = app.clone();
                            tauri::async_runtime::spawn(async move {
                                let current = commands::get_sprite_enabled_sync().unwrap_or(false);
                                let _ = commands::set_sprite_enabled(!current, app_handle).await;
                            });
                        }
                        "quit" => {
                            app.exit(0);
                        }
                        _ => {}
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                        let app = tray.app_handle();
                        if let Some(win) = app.get_webview_window("main") {
                            let _ = win.show();
                            let _ = win.set_focus();
                        }
                    }
                })
                .build(app)?;
            }

            // Start KnowledgePipeline
            std::thread::spawn(|| {
                std::thread::sleep(std::time::Duration::from_secs(2));
                services::knowledge_pipeline::start();
            });

            // Start independent Tag Pipeline (reads from source_data)
            std::thread::spawn(|| {
                std::thread::sleep(std::time::Duration::from_secs(3));
                // services::ai_tag::start();  // 已禁用
            });

            // Start Data Ingestion Mid-Platform (统一数据中台)
            // 从 knowledge_push_queue 获取任务，处理所有业务对象
            let ingestion_handle = app.handle().clone();
            std::thread::spawn(move || {
                // 启动时清理孤儿数据
                println!("[启动] 清理孤儿数据...");
                services::file_ingestion::cleanup_orphan_source_data();
                println!("[启动] 孤儿数据清理完成");
                
                std::thread::sleep(std::time::Duration::from_secs(1));
                services::file_ingestion::start(ingestion_handle);
            });

            // ★ 启动通知循环：App启动时执行周期后台LLM分析
            services::notification_loop::start_notification_loop(app.handle().clone());

            // ★ 启动邮件轮询：App启动时执行周期IMAP同步+AI分类
            services::email_polling::init_app_handle(app.handle().clone());
            services::email_polling::start_email_loop();

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
            // Files
            commands::file_list, commands::file_upload, commands::file_delete,
            commands::file_open, commands::file_download, commands::file_get_path, commands::file_has_oss_files, commands::file_migrate_files,
            commands::file_update_meta,
            commands::file_list_by_product, commands::file_open_product, commands::file_download_product,
            commands::file_preview, commands::file_export, commands::file_export_to_path, commands::file_read_bytes, commands::chat_save_temp_image, commands::file_remove_temp, commands::chat_read_clipboard_files,
            commands::file_copy_to_clipboard,
            commands::file_list_global,
            commands::file_move, commands::file_tree,
            commands::file_folder_create, commands::file_folder_rename, commands::file_folder_delete, commands::file_folder_move, commands::file_folder_reorder,
            commands::file_remove_note_mapping, commands::file_list_by_note,
            commands::file_add_todo_mapping, commands::file_remove_todo_mapping, commands::file_list_by_todo,
            commands::file_delete_from_domain,
            commands::file_list_all, commands::file_move_to_folder,
            commands::file_unlink_from_product, commands::file_get_product_associations, commands::file_update_product_associations,
            // Notes
            commands::notes_list_all, commands::notes_list_daily, commands::notes_list_by_project, commands::notes_list_orphan, commands::notes_list_trash, commands::notes_list_archive, commands::notes_list_by_notebook, commands::notes_get, commands::notes_get_for_trash,
            commands::notes_create, commands::notes_update, commands::notes_delete, commands::notes_restore, commands::notes_permanent_delete,
            commands::notes_count_by_notebook,
            // Notebooks
            commands::notebook_list, commands::notebook_create, commands::notebook_update, commands::notebook_delete,
            // Partners (unified)
            commands::partner_list, commands::partner_create, commands::partner_update, commands::partner_delete,
            commands::partner_link_project, commands::partner_unlink_project,
            commands::partner_contact_create, commands::partner_contact_update, commands::partner_contact_delete,
            commands::partner_update_remark,
            commands::contact_link_project, commands::contact_unlink_project,
            commands::contacts_list_all, commands::contacts_count,
            // Partners (legacy aliases)
            commands::channel_list, commands::channel_create, commands::channel_update, commands::channel_delete,
            commands::channel_contact_create, commands::channel_contact_update, commands::channel_contact_delete,
            commands::channel_link_project, commands::channel_unlink_project,
            commands::channel_update_remark,
            commands::vendor_list, commands::vendor_create, commands::vendor_update, commands::vendor_delete,
            commands::vendor_contact_create, commands::vendor_contact_update, commands::vendor_contact_delete,
            commands::vendor_link_project, commands::vendor_unlink_project,
            commands::vendor_update_remark,
            commands::customer_list, commands::customer_create, commands::customer_update, commands::customer_delete,
            commands::customer_contact_create, commands::customer_contact_update, commands::customer_contact_delete,
            commands::customer_link_project, commands::customer_unlink_project,
            commands::contacts_list_channels, commands::contacts_list_vendors,
            // Solutions
            commands::solution_list, commands::solution_create, commands::solution_rename, commands::solution_delete,
            commands::solution_version_list, commands::solution_version_create, commands::solution_version_update_meta,
            commands::solution_version_finalize, commands::solution_version_save_draft, commands::solution_version_delete,
            // Storage
            commands::storage_get_config, commands::storage_update_config, commands::oss_test_connection,
            commands::storage_migrate, commands::storage_migrate_progress, commands::storage_default_path,
            // KB
            commands::kb_get_config, commands::kb_update_config, commands::kb_import_file, commands::kb_save_note,
            commands::kb_search, commands::kb_delete, commands::kb_list_documents, commands::kb_chat, commands::kb_agent_chat, commands::agent_cancel, commands::kb_save_sessions, commands::kb_load_sessions, commands::kb_load_session_messages, commands::kb_delete_session, commands::kb_fab_init_session, commands::kb_fab_load_history, commands::kb_fab_save_message,
            commands::kb_diagnose, commands::kb_search_debug, commands::kb_data_browse, commands::kb_browse_parents, commands::kb_browse_vectors, commands::kb_inspect_fts5,
            commands::kb_fts5_inspector, commands::kb_get_table_chunks, commands::execute_production_rag,
            commands::kb_test_embedding, commands::kb_test_chat,
            commands::ext_tool_search_knowledge_base,
            commands::ext_tool_find_related_knowledge,
            commands::ext_tool_get_project_context,
            commands::kb_memory_objects, commands::search_business, commands::search_source_data, commands::lookup_object_names, commands::kb_project_note_ids, commands::kb_memory_parents,
            commands::kb_memory_heatmap, commands::kb_memory_dashboard_stats,
            commands::ext_tool_translate_filter, commands::ext_tool_search_filtered,
            // AI sync
            commands::ai_sync_status, commands::ai_sync_trigger, commands::ai_sync_diagnose,
            commands::cleanup_retry_failed, commands::cleanup_orphan_data,
            commands::knowledge_pipeline_status,
            commands::pipeline_debug_status, commands::pipeline_clear_knowledge, commands::pipeline_clear_vectors, commands::pipeline_clear_all, commands::pipeline_resync, commands::pipeline_browse_source, commands::pipeline_get_source_content, commands::pipeline_revectorize, commands::pipeline_retry_failed,
            // Assets
            commands::assets_search, commands::assets_search_fts5, commands::assets_list,
            commands::asset_sync_status, commands::asset_sync_trigger, commands::asset_diagnose,
            commands::asset_get_latest_version, commands::asset_get_parsed_text, commands::asset_rerun_migration,
            // Settings
            commands::settings_get, commands::settings_update, commands::get_ai_config, commands::update_ai_config, commands::ai_test_provider, commands::ai_test_embedding,
            commands::get_system_setting, commands::set_system_setting,
            commands::user_profile, commands::user_profile_update,
            // Sprite Position
            commands::save_sprite_position, commands::get_sprite_position,
            commands::get_sprite_enabled, commands::set_sprite_enabled,
            // Window
            commands::window_minimize, commands::window_toggle_maximize, commands::window_close,
            // Sprite Window
            commands::create_sprite_window, commands::toggle_sprite_window, commands::close_sprite_window, commands::set_sprite_position,
            // Chat Window
            commands::create_chat_window, commands::toggle_chat_window, commands::close_chat_window, commands::expand_chat_window, commands::set_chat_dialog_mode,
            // Export
            commands::export_data, commands::export_pick_save_path, commands::import_data, commands::backup_all, commands::restore_all, commands::export_write_buffer,
            commands::solution_download_excel,
            // Todos
            commands::todos_list, commands::todos_create, commands::todos_update, commands::todos_delete,
            commands::todos_toggle, commands::todos_get,
            // OSS
            commands::oss_raw_diagnose,
            // Logs
            commands::get_logs, commands::clear_logs,
            // Notifications
            commands::notification_list, commands::notification_create, commands::notification_mark_read,
            commands::notification_mark_all_read, commands::notification_delete, commands::notification_batch_delete, commands::notification_stats,
            commands::notification_config_get, commands::notification_config_update,
            commands::notification_loop_status,
            // 邮件
            commands::email_add_account, commands::email_list_accounts,
            commands::email_update_account, commands::email_delete_account,
            commands::email_list, commands::email_get_detail,
            commands::email_render_html, commands::email_inline_asset,
            commands::email_sync, commands::email_loop_status,
            commands::email_config_update, commands::email_config_get,
            commands::email_stats, commands::email_action_confirm, commands::email_action_ignore,
            commands::email_reprocess_knowledge, commands::email_reprocess_attachments,
            // Project ID Migration（canonical ID 收敛）
            commands::migration_submit_project_id_map, commands::migration_project_id_state,
            // Agent Profiles
            commands::agent_available_tools, commands::agent_profile_list, commands::agent_profile_get,
            commands::agent_profile_create, commands::agent_profile_update, commands::agent_profile_delete,
            commands::agent_tool_list, commands::agent_tool_assign, commands::agent_profile_reset,
            // Debug（仅 debug 构建注册）
            #[cfg(debug_assertions)]
            commands::save_agent_context,
            #[cfg(debug_assertions)]
            commands::get_agent_context,
            #[cfg(debug_assertions)]
            commands::debug_rehearse_id_migration,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Arkmind")
        .run(|app_handle, event| {
            #[cfg(target_os = "macos")]
            {
                // macOS：点击 Dock 图标重新打开主窗口
                if let tauri::RunEvent::Reopen { .. } = event {
                    if let Some(win) = app_handle.get_webview_window("main") {
                        let _ = win.show();
                        let _ = win.set_focus();
                    }
                }
            }
            #[cfg(not(target_os = "macos"))]
            {
                let _ = (app_handle, event);
            }
        });
}
