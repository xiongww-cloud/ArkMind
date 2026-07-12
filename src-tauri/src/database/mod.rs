pub mod db_core;
pub mod db_schema;
pub mod db_fts;
pub mod db_migrations;

pub use db_core::*;
pub use db_fts::*;
pub use db_migrations::*;

use rusqlite::{Connection, Result as SqliteResult};

pub fn init_database(path: &std::path::Path) -> SqliteResult<()> {
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent).map_err(|e| {
                let msg = format!("[数据库] 创建目录失败 {:?}: {}", parent, e);
                eprintln!("{}", msg);
                rusqlite::Error::InvalidParameterName(msg)
            })?;
        }
    }
    eprintln!("[数据库] 正在打开数据库 {:?}", path);
    let conn = Connection::open(path).map_err(|e| {
        eprintln!("[数据库] 打开数据库失败: {}", e);
        e
    })?;

    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "busy_timeout", "5000")?;
    conn.pragma_update(None, "recursive_triggers", "ON")?;

    db_schema::create_schema(&conn)?;

    // ★ 列出所有残留触发器（调试用）
    {
        let mut stmt = conn.prepare("SELECT name, sql FROM sqlite_master WHERE type='trigger'").unwrap();
        let rows = stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))).unwrap();
        for row in rows.flatten() {
            let (name, sql) = row;
            if sql.contains("source_data_old") || name.starts_with("sd_") {
                eprintln!("[数据库] 残留触发器: {} => {}", name, &sql[..sql.len().min(300)]);
            }
        }
    }

    // ★ 清理所有旧的 source_data 触发器（避免引用不存在的 source_data_old 表）
    eprintln!("[数据库] 清理旧触发器...");
    conn.execute_batch("
        DROP TRIGGER IF EXISTS sd_notes_insert;
        DROP TRIGGER IF EXISTS sd_notes_update;
        DROP TRIGGER IF EXISTS sd_notes_delete;
        DROP TRIGGER IF EXISTS sd_products_insert;
        DROP TRIGGER IF EXISTS sd_products_update;
        DROP TRIGGER IF EXISTS sd_products_delete;
        DROP TRIGGER IF EXISTS sd_projects_insert;
        DROP TRIGGER IF EXISTS sd_projects_update;
        DROP TRIGGER IF EXISTS sd_projects_delete;
        DROP TRIGGER IF EXISTS sd_todos_insert;
        DROP TRIGGER IF EXISTS sd_todos_update;
        DROP TRIGGER IF EXISTS sd_todos_delete;
        DROP TRIGGER IF EXISTS sd_files_insert;
        DROP TRIGGER IF EXISTS sd_files_update;
        DROP TRIGGER IF EXISTS sd_files_delete;
        DROP TRIGGER IF EXISTS sd_chats_insert;
        DROP TRIGGER IF EXISTS sd_chats_update;
        DROP TRIGGER IF EXISTS sd_chats_delete;
        DROP TRIGGER IF EXISTS sd_project_solution_insert;
        DROP TRIGGER IF EXISTS sd_project_solution_update;
        DROP TRIGGER IF EXISTS sd_project_solution_delete;
        DROP TRIGGER IF EXISTS sd_project_dynamics_insert;
        DROP TRIGGER IF EXISTS sd_project_dynamics_update;
        DROP TRIGGER IF EXISTS sd_project_dynamics_delete;
        DROP TRIGGER IF EXISTS sd_channel_partners_insert;
        DROP TRIGGER IF EXISTS sd_channel_partners_update;
        DROP TRIGGER IF EXISTS sd_channel_partners_delete;
        DROP TRIGGER IF EXISTS sd_vendor_partners_insert;
        DROP TRIGGER IF EXISTS sd_vendor_partners_update;
        DROP TRIGGER IF EXISTS sd_vendor_partners_delete;
        DROP TRIGGER IF EXISTS sd_files_delete;
        DROP TRIGGER IF EXISTS sd_dynamics_delete;
        DROP TRIGGER IF EXISTS sd_reset_sync_on_update;
        DROP TRIGGER IF EXISTS sd_reset_sync_on_insert;
        DROP TRIGGER IF EXISTS sd_fts_insert;
        DROP TRIGGER IF EXISTS sd_fts_update;
        DROP TRIGGER IF EXISTS sd_fts_delete;
        DROP TRIGGER IF EXISTS sd_chunks_delete;
        DROP TRIGGER IF EXISTS sd_todos_done_cleanup;
    ").ok();
    eprintln!("[数据库] 旧触发器清理完成");

    // ★ 列出清理后所有触发器（调试用）
    {
        let mut stmt = conn.prepare("SELECT name, sql FROM sqlite_master WHERE type='trigger'").unwrap();
        let rows = stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))).unwrap();
        for row in rows.flatten() {
            let (name, sql) = row;
            if sql.contains("source_data_old") {
                eprintln!("[数据库] ⚠️ 仍引用 source_data_old: {} => {}", name, &sql[..sql.len().min(300)]);
            }
        }
    }

    db_migrations::run_migrations(&conn)?;

    // ★ 文件索引：file_relations.file_id 索引
    let _ = conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_file_rel_file ON file_relations(file_id);"
    );
    // Drop old unique constraint (was based on file_id/asset_id, now using id/relation_id)
    let _ = conn.execute_batch("DROP INDEX IF EXISTS idx_file_rel_unique;");

    db_migrations::migrate_files_to_file_assets(&conn);
    // ★ 旧版 file_objects/file_versions → 填充 file_relations.file_id，然后清理
    db_migrations::migrate_file_relations_add_file_id(&conn);
    db_migrations::dedupe_file_assets_hash(&conn);
    db_migrations::migrate_folder_to_library_locations(&conn);

    // ★ 迁移 user_policies → source_data (POLICY)
    db_migrations::migrate_user_policies_to_source_data(&conn);

    // ★ 添加缺失的索引
    let _ = conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_cp_project ON channel_partners(project_id);
         CREATE INDEX IF NOT EXISTS idx_cc_channel ON channel_contacts(channel_id);
         CREATE INDEX IF NOT EXISTS idx_vp_project ON vendor_partners(project_id);
         CREATE INDEX IF NOT EXISTS idx_vc_vendor ON vendor_contacts(vendor_id);
         CREATE INDEX IF NOT EXISTS idx_pd_project ON project_dynamics(project_id);
         CREATE INDEX IF NOT EXISTS idx_si_version ON solution_items(solution_version_id);"
    );

    // ★ 添加 file_assets.original_name 列
    let _ = conn.execute_batch("ALTER TABLE file_assets ADD COLUMN original_name TEXT DEFAULT ''");

    // ★ KB FTS5 triggers always initialized (core retrieval infrastructure)
    db_fts::init_kb_fts_triggers(&conn)?;
    db_fts::seed_kb_fts(&conn);

    // ★ KB FTS5 内容同步触发器（确保 knowledge_chunks 变更实时同步到 kk_chunks_fts）
    // 使用 DROP + CREATE 确保触发器定义始终最新（CREATE TRIGGER IF NOT EXISTS 不会更新已有触发器）
    // 注意：不创建 DELETE 触发器，FTS5 清理在应用代码中处理（更可靠）
    conn.execute_batch(
        "DROP TRIGGER IF EXISTS kk_chunks_fts_insert;
         DROP TRIGGER IF EXISTS kk_chunks_fts_delete;
         DROP TRIGGER IF EXISTS kk_chunks_fts_update;
         CREATE TRIGGER kk_chunks_fts_insert AFTER INSERT ON knowledge_chunks
         WHEN new.chunk_type = 'child'
         BEGIN
             INSERT INTO kk_chunks_fts(rowid, doc_title, content) VALUES (new.rowid, new.doc_title, new.content);
         END;
         CREATE TRIGGER kk_chunks_fts_update AFTER UPDATE ON knowledge_chunks
         WHEN old.chunk_type = 'child' OR new.chunk_type = 'child'
         BEGIN
             DELETE FROM kk_chunks_fts WHERE rowid = old.rowid;
             INSERT INTO kk_chunks_fts(rowid, doc_title, content)
               SELECT new.rowid, new.doc_title, new.content
               WHERE new.chunk_type = 'child';
         END;"
    )?;
    eprintln!("[数据库] kk_chunks_fts 触发器已创建（INSERT + UPDATE）");
    // ★ 回填已有数据到 kk_chunks_fts（触发器不覆盖已有行）
    let seeded: i64 = conn.query_row(
        "SELECT COUNT(*) FROM kk_chunks_fts", [], |r| r.get(0)
    ).unwrap_or(0);
    if seeded == 0 {
        let n = conn.execute(
            "INSERT OR IGNORE INTO kk_chunks_fts(rowid, doc_title, content) SELECT rowid, doc_title, content FROM knowledge_chunks WHERE chunk_type = 'child'",
            [],
        ).unwrap_or(0);
        eprintln!("[数据库] kk_chunks_fts 回填: {} 行", n);
    } else {
        eprintln!("[数据库] kk_chunks_fts 已有数据 ({} 行)，跳过回填", seeded);
    }

    // ★ FTS5 自动镜像（已移除写入触发器，改为业务命令调用 notify_changed）

    // ★ 迁移 source_data.id 为 TEXT（使用 source_id）
    db_migrations::migrate_source_data_to_text_id(&conn)?;

    // ★ FTS5 触发器在迁移后创建，确保引用正确的表名
    eprintln!("[数据库] 创建 FTS5 触发器...");
    db_fts::init_source_data_fts_triggers(&conn)?;
    eprintln!("[数据库] FTS5 触发器创建完成");

    // ★ 最终检查：列出所有触发器
    {
        let mut stmt = conn.prepare("SELECT name, sql FROM sqlite_master WHERE type='trigger'").unwrap();
        let rows = stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))).unwrap();
        for row in rows.flatten() {
            let (name, sql) = row;
            if sql.contains("source_data_old") {
                eprintln!("[数据库] ⚠️ 最终检查 - 仍引用 source_data_old: {} => {}", name, &sql[..sql.len().min(300)]);
            }
        }
    }

    // ★ ★ CHAT 去重：清理重复 + 唯一索引（P0 修复）
    let _ = conn.execute_batch("DELETE FROM source_data WHERE rowid NOT IN (SELECT MIN(rowid) FROM source_data GROUP BY source_type, source_id)");
    let _ = conn.execute_batch("CREATE UNIQUE INDEX IF NOT EXISTS idx_sd_unique ON source_data(source_type, source_id)");
    println!("[数据库] FTS5 + AI 触发器已初始化");

    // Store in global AFTER all init is done
    db_core::set_db(conn);

    db_migrations::purge_stale_queue_entries();
    if let Err(e) = with_db(|c| c.pragma_update(None, "wal_checkpoint", "TRUNCATE")) {
        eprintln!("[数据库] WAL 检查点警告: {}", e);
    }
    Ok(())
}
