//! SQLite database — rusqlite powered, WAL mode, FTS5 full-text search
//!
//! 35+ tables: core business + AI pipeline + knowledge base + FTS5 virtual tables

use rusqlite::{Connection, Result as SqliteResult};
use once_cell::sync::Lazy;
use std::sync::Mutex;
use base64::Engine;
use serde_json::json;

static DB: Lazy<Mutex<Option<Connection>>> = Lazy::new(|| Mutex::new(None));

pub fn db_path(data_dir: &std::path::Path) -> std::path::PathBuf {
    data_dir.join("arkmind.db")
}

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

    create_schema(&conn)?;

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

    run_migrations(&conn)?;

    // ★ 文件索引：file_relations.file_id 索引
    let _ = conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_file_rel_file ON file_relations(file_id);"
    );
    // Drop old unique constraint (was based on file_id/asset_id, now using id/relation_id)
    let _ = conn.execute_batch("DROP INDEX IF EXISTS idx_file_rel_unique;");

    repair_empty_file_ext(&conn);
    migrate_files_to_file_assets(&conn);

    // ★ 将旧映射表数据迁入 file_relations（若无 object_id 则直接使用 file_id）
    migrate_to_file_relations(&conn);
    // ★ 旧版 file_objects/file_versions → 填充 file_relations.file_id，然后清理
    migrate_file_relations_add_file_id(&conn);
    dedupe_file_assets_hash(&conn);
    migrate_folder_to_library_locations(&conn);
    fix_assets_fts_schema(&conn);

    // ★ 迁移 user_policies → source_data (POLICY)
    migrate_user_policies_to_source_data(&conn);

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
    init_kb_fts_triggers(&conn)?;
    seed_kb_fts(&conn);

    // ★ FTS5 自动镜像（已移除写入触发器，改为业务命令调用 notify_changed）
    // init_struct_source_data_triggers(&conn)?;

    // ★ 迁移 source_data.id 为 TEXT（使用 source_id）
    migrate_source_data_to_text_id(&conn)?;

    // ★ FTS5 触发器在迁移后创建，确保引用正确的表名
    eprintln!("[数据库] 创建 FTS5 触发器...");
    init_source_data_fts_triggers(&conn)?;
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

    // ★ 初始化 AI 触发器（已禁用，改用时间戳驱动）
    // init_ai_queue_triggers(&conn)?;

    // ★ ★ CHAT 去重：清理重复 + 唯一索引（P0 修复）
    let _ = conn.execute_batch("DELETE FROM source_data WHERE rowid NOT IN (SELECT MIN(rowid) FROM source_data GROUP BY source_type, source_id)");
    let _ = conn.execute_batch("CREATE UNIQUE INDEX IF NOT EXISTS idx_sd_unique ON source_data(source_type, source_id)");
    println!("[数据库] FTS5 + AI 触发器已初始化");

    // Store in global AFTER all init is done
    {
        let mut db = DB.lock().unwrap_or_else(|e| e.into_inner());
        *db = Some(conn);
    }

    purge_stale_queue_entries();
    if let Err(e) = with_db(|c| c.pragma_update(None, "wal_checkpoint", "TRUNCATE")) {
        eprintln!("[数据库] WAL 检查点警告: {}", e);
    }
    Ok(())
}

pub fn with_db<F, T>(f: F) -> SqliteResult<T>
where F: FnOnce(&Connection) -> SqliteResult<T> {
    let db = DB.lock().unwrap_or_else(|e| e.into_inner());
    let conn = db.as_ref().expect("Database not initialized");
    f(conn)
}

// ═══════════════════════════════════════════════════════════════════
// Schema
// ═══════════════════════════════════════════════════════════════════

fn create_schema(conn: &Connection) -> SqliteResult<()> {
    conn.execute_batch("
        CREATE TABLE IF NOT EXISTS users (
            id TEXT PRIMARY KEY, name TEXT DEFAULT '本地管理员',
            username TEXT DEFAULT 'local-admin', role TEXT DEFAULT 'OWNER',
            phone TEXT DEFAULT '', email TEXT DEFAULT '', avatar TEXT DEFAULT '',
            ai_config TEXT DEFAULT '{}',
            created_at TEXT DEFAULT (datetime('now', 'localtime')),
            updated_at TEXT DEFAULT (datetime('now', 'localtime'))
        );
        CREATE TABLE IF NOT EXISTS license (
            id INTEGER PRIMARY KEY AUTOINCREMENT, key_hash TEXT,
            license_key TEXT, status TEXT DEFAULT 'inactive',
            product TEXT DEFAULT '', activated_at TEXT, expires_at TEXT,
            machine_code TEXT, created_at TEXT DEFAULT (datetime('now', 'localtime'))
        );
        CREATE TABLE IF NOT EXISTS projects (
            id TEXT PRIMARY KEY, name TEXT NOT NULL, description TEXT DEFAULT '',
            status TEXT DEFAULT 'Lead', owner TEXT DEFAULT '', user_id TEXT DEFAULT '',
            budget REAL DEFAULT 0, customer TEXT DEFAULT '',
            bid_time TEXT DEFAULT '', customer_contact TEXT DEFAULT '', customer_phone TEXT DEFAULT '',
            created_at TEXT DEFAULT (datetime('now', 'localtime')), updated_at TEXT DEFAULT (datetime('now', 'localtime'))
        );
        CREATE TABLE IF NOT EXISTS project_dynamics (
            id TEXT PRIMARY KEY, project_id TEXT NOT NULL, type TEXT DEFAULT 'system',
            content TEXT NOT NULL DEFAULT '', operator TEXT DEFAULT '',
            created_at TEXT DEFAULT (datetime('now', 'localtime'))
        );
        CREATE TABLE IF NOT EXISTS product_series (
            id TEXT PRIMARY KEY, name TEXT NOT NULL, parent_id TEXT,
            sort_order INTEGER DEFAULT 0,
            created_at TEXT DEFAULT (datetime('now', 'localtime')), updated_at TEXT DEFAULT (datetime('now', 'localtime'))
        );
        CREATE TABLE IF NOT EXISTS products (
            id TEXT PRIMARY KEY, name TEXT NOT NULL,
            series_id TEXT, series_name TEXT DEFAULT '', description TEXT DEFAULT '',
            price REAL DEFAULT 0, standard_specs TEXT DEFAULT '', full_specs TEXT DEFAULT '',
            sort_order INTEGER DEFAULT 0,
            created_at TEXT DEFAULT (datetime('now', 'localtime')), updated_at TEXT DEFAULT (datetime('now', 'localtime'))
        );
        CREATE TABLE IF NOT EXISTS solutions (
            id TEXT PRIMARY KEY, project_id TEXT NOT NULL,
            name TEXT NOT NULL DEFAULT '新建方案',
            created_at TEXT DEFAULT (datetime('now', 'localtime')), updated_at TEXT DEFAULT (datetime('now', 'localtime'))
        );
        CREATE TABLE IF NOT EXISTS solution_versions (
            id TEXT PRIMARY KEY, solution_id TEXT NOT NULL,
            version TEXT DEFAULT 'V1.0', tags TEXT DEFAULT '',
            content TEXT DEFAULT '', creator TEXT DEFAULT '', status TEXT DEFAULT 'draft',
            created_at TEXT DEFAULT (datetime('now', 'localtime')), updated_at TEXT DEFAULT (datetime('now', 'localtime'))
        );
        CREATE TABLE IF NOT EXISTS solution_items (
            id TEXT PRIMARY KEY, solution_version_id TEXT NOT NULL,
            product_id TEXT, product_name TEXT DEFAULT '', quantity INTEGER DEFAULT 1,
            unit_price REAL DEFAULT 0, description TEXT DEFAULT '',
            device_name TEXT DEFAULT '', row_type TEXT DEFAULT 'device',
            sort_order INTEGER DEFAULT 0,
            created_at TEXT DEFAULT (datetime('now', 'localtime')), updated_at TEXT DEFAULT (datetime('now', 'localtime'))
        );
        CREATE TABLE IF NOT EXISTS system_config (
            tenant_id TEXT PRIMARY KEY DEFAULT 'local-user',
            active_storage TEXT DEFAULT 'LOCAL', local_path TEXT DEFAULT '',
            oss_region TEXT DEFAULT '', oss_bucket TEXT DEFAULT '',
            oss_ak TEXT DEFAULT '', oss_sk TEXT DEFAULT '', oss_encrypt_key TEXT DEFAULT '',
            encrypt_password_salt TEXT DEFAULT '',
            ai_config TEXT DEFAULT '{}', kb_config TEXT DEFAULT '{}',
            storage_cards TEXT DEFAULT '[]', active_card_id TEXT DEFAULT '',
            created_at TEXT DEFAULT (datetime('now', 'localtime')), updated_at TEXT DEFAULT (datetime('now', 'localtime'))
        );
        CREATE TABLE IF NOT EXISTS notebooks (
            id TEXT PRIMARY KEY, name TEXT NOT NULL, color TEXT DEFAULT '#64748b',
            sort_order INTEGER DEFAULT 0,
            created_at TEXT DEFAULT (datetime('now', 'localtime')), updated_at TEXT DEFAULT (datetime('now', 'localtime'))
        );
        CREATE TABLE IF NOT EXISTS notes (
            note_id TEXT PRIMARY KEY, title TEXT NOT NULL DEFAULT '',
            content TEXT NOT NULL DEFAULT '', is_daily INTEGER NOT NULL DEFAULT 1,
            notebook_id TEXT DEFAULT NULL, deleted_at TEXT DEFAULT NULL,
            hit_count INTEGER DEFAULT 0, last_hit_at TEXT, neural_energy REAL DEFAULT 100.0,
            created_at TEXT DEFAULT (datetime('now', 'localtime')), updated_at TEXT DEFAULT (datetime('now', 'localtime'))
        );
        CREATE TABLE IF NOT EXISTS note_project_mapping (
            note_id TEXT NOT NULL, project_id TEXT NOT NULL,
            created_at TEXT DEFAULT (datetime('now', 'localtime')),
            PRIMARY KEY (note_id, project_id)
        );
        CREATE TABLE IF NOT EXISTS tags (
            tag_id TEXT PRIMARY KEY, name TEXT NOT NULL, color TEXT DEFAULT '#6366f1', type TEXT DEFAULT 'general'
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_tags_name ON tags(name);
        CREATE TABLE IF NOT EXISTS file_assets (
            id TEXT PRIMARY KEY, file_ext TEXT DEFAULT '', file_size INTEGER DEFAULT 0,
            content_hash TEXT NOT NULL UNIQUE, parse_status TEXT DEFAULT 'PENDING',
            original_name TEXT DEFAULT '',
            created_at TEXT DEFAULT (datetime('now', 'localtime')), updated_at TEXT DEFAULT (datetime('now', 'localtime'))
        );
        CREATE TABLE IF NOT EXISTS library_folders (
            id TEXT PRIMARY KEY, name TEXT NOT NULL DEFAULT '',
            parent_id TEXT DEFAULT NULL,
            project_id TEXT DEFAULT NULL,
            sort_order INTEGER DEFAULT 0,
            created_at TEXT DEFAULT (datetime('now', 'localtime')), updated_at TEXT DEFAULT (datetime('now', 'localtime')),
            FOREIGN KEY (parent_id) REFERENCES library_folders(id) ON DELETE CASCADE
        );
        CREATE TABLE IF NOT EXISTS library_file_locations (
            file_id TEXT NOT NULL,
            folder_id TEXT DEFAULT NULL,
            PRIMARY KEY (file_id),
            FOREIGN KEY (folder_id) REFERENCES library_folders(id) ON DELETE SET NULL
        );
        -- ★ Polymorphic file relation: file_id references file_assets.id directly
        CREATE TABLE IF NOT EXISTS file_relations (
            id TEXT PRIMARY KEY,
            file_id TEXT DEFAULT NULL,
            entity_type TEXT NOT NULL,
            entity_id TEXT NOT NULL DEFAULT '',
            custom_name TEXT DEFAULT '',
            remark TEXT DEFAULT '',
            creator TEXT DEFAULT '',
            description TEXT DEFAULT '',
            folder_id TEXT DEFAULT NULL,
            created_at TEXT DEFAULT (datetime('now', 'localtime'))
        );
        CREATE INDEX IF NOT EXISTS idx_file_rel_entity ON file_relations(entity_type, entity_id);
        -- idx_file_rel_object / idx_file_rel_unique 在 run_migrations(ALTER TABLE ADD object_id)之后创建

        CREATE TABLE IF NOT EXISTS channel_partners (id TEXT PRIMARY KEY, project_id TEXT, name TEXT, phone TEXT, company TEXT, created_at TEXT DEFAULT (datetime('now', 'localtime')));
        CREATE TABLE IF NOT EXISTS channel_contacts (id TEXT PRIMARY KEY, channel_id TEXT, name TEXT, phone TEXT, email TEXT, created_at TEXT DEFAULT (datetime('now', 'localtime')));
        CREATE TABLE IF NOT EXISTS vendor_partners (id TEXT PRIMARY KEY, project_id TEXT, name TEXT, phone TEXT, company TEXT, created_at TEXT DEFAULT (datetime('now', 'localtime')));
        CREATE TABLE IF NOT EXISTS vendor_contacts (id TEXT PRIMARY KEY, vendor_id TEXT, name TEXT, phone TEXT, email TEXT, created_at TEXT DEFAULT (datetime('now', 'localtime')));
        -- ★ Project ↔ Product mapping
        CREATE TABLE IF NOT EXISTS project_product_mapping (
            project_id TEXT NOT NULL, product_id TEXT NOT NULL, product_name TEXT DEFAULT '',
            created_at TEXT DEFAULT (datetime('now', 'localtime')),
            PRIMARY KEY (project_id, product_id)
        );

        -- ★ Todos: task management
        CREATE TABLE IF NOT EXISTS todos (
            id TEXT PRIMARY KEY, title TEXT NOT NULL DEFAULT '', content TEXT NOT NULL DEFAULT '',
            done INTEGER NOT NULL DEFAULT 0, priority TEXT DEFAULT NULL, due_date TEXT DEFAULT NULL,
            repeat TEXT DEFAULT NULL,
            created_at TEXT DEFAULT (datetime('now', 'localtime')), updated_at TEXT DEFAULT (datetime('now', 'localtime'))
        );
        CREATE TABLE IF NOT EXISTS todo_project_mapping (
            todo_id TEXT NOT NULL, project_id TEXT NOT NULL,
            created_at TEXT DEFAULT (datetime('now', 'localtime')),
            PRIMARY KEY (todo_id, project_id)
        );
        CREATE INDEX IF NOT EXISTS idx_todos_done ON todos(done);
        CREATE INDEX IF NOT EXISTS idx_todos_due ON todos(due_date);

        -- AI pipeline tables
        CREATE TABLE IF NOT EXISTS ai_sync_queue (id INTEGER PRIMARY KEY AUTOINCREMENT, source_table TEXT NOT NULL, source_id TEXT NOT NULL, is_embedded INTEGER DEFAULT 0, retry_count INTEGER DEFAULT 0, last_dedup_key TEXT DEFAULT '', last_error TEXT DEFAULT '', created_at TEXT DEFAULT (datetime('now', 'localtime')), updated_at TEXT DEFAULT (datetime('now', 'localtime')), UNIQUE(source_table, source_id));
        CREATE TABLE IF NOT EXISTS ai_tag_queue (id INTEGER PRIMARY KEY AUTOINCREMENT, source_type TEXT NOT NULL, source_id TEXT NOT NULL, status INTEGER DEFAULT 0, tag_type TEXT DEFAULT 'ai_shadow', retry_count INTEGER DEFAULT 0, last_error TEXT DEFAULT '', created_at TEXT DEFAULT (datetime('now', 'localtime')), updated_at TEXT DEFAULT (datetime('now', 'localtime')), UNIQUE(source_type, source_id));
        -- ★ Event Bus: 对象变更事件溯源
        CREATE TABLE IF NOT EXISTS event_store (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            event_type TEXT NOT NULL,
            object_type TEXT NOT NULL,
            object_id TEXT NOT NULL,
            payload TEXT DEFAULT '{}',
            source TEXT NOT NULL DEFAULT 'user',
            trace_id TEXT,
            created_at TEXT DEFAULT (datetime('now', 'localtime'))
        );
        CREATE INDEX IF NOT EXISTS idx_event_type ON event_store(event_type);
        CREATE INDEX IF NOT EXISTS idx_event_object ON event_store(object_type, object_id);
        CREATE INDEX IF NOT EXISTS idx_event_time ON event_store(created_at);

        -- ★ Unified tag mappings (replaces note/file/product/project_tags_mapping)
        CREATE TABLE IF NOT EXISTS tag_mappings (
            entity_type TEXT NOT NULL,
            entity_id TEXT NOT NULL,
            tag_id TEXT NOT NULL,
            PRIMARY KEY (entity_type, entity_id, tag_id)
        );

        CREATE TABLE IF NOT EXISTS chat_sessions (
            id TEXT PRIMARY KEY, title TEXT DEFAULT '',
            created_at TEXT DEFAULT (datetime('now','localtime')),
            updated_at TEXT DEFAULT (datetime('now','localtime'))
        );
        CREATE TABLE IF NOT EXISTS chat_messages (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id TEXT NOT NULL REFERENCES chat_sessions(id) ON DELETE CASCADE,
            role TEXT NOT NULL, content TEXT DEFAULT '',
            created_at TEXT DEFAULT (datetime('now','localtime'))
        );
        CREATE INDEX IF NOT EXISTS idx_chat_msg_session ON chat_messages(session_id);

        -- ═══════════════════════════════════════════════════════════════════
        -- ★ Knowledge Base: Content-Label Separation / Dual-Layer Storage
        -- knowledge_chunks: 150-280 char fine-grained slices
        CREATE TABLE IF NOT EXISTS knowledge_chunks (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            chunk_id TEXT NOT NULL DEFAULT '',
            chunk_type TEXT NOT NULL DEFAULT 'child',    -- 'parent' | 'child'
            parent_id TEXT DEFAULT NULL,                  -- 子块 → 父块.chunk_id；父块=NULL
            source_data_id INTEGER DEFAULT NULL,          -- 父块 → source_data.id；子块=NULL
            content TEXT NOT NULL,
            doc_title TEXT DEFAULT '',                     -- 业务名称（文件为空）
            chunk_index INTEGER DEFAULT 0,                -- 父块：源文档中的顺序；子块：父块中的顺序
            content_hash TEXT DEFAULT '',
            base_weight REAL DEFAULT 1.0,                  -- 演化：初始语义位能
            last_access_at INTEGER DEFAULT NULL,           -- 演化：上次被访问的 Unix 时间戳
            access_count INTEGER DEFAULT 0,                -- 演化：历史放电次数
            created_at TEXT DEFAULT (datetime('now', 'localtime'))
        );
        CREATE INDEX IF NOT EXISTS idx_kc_parent ON knowledge_chunks(parent_id);
        -- idx_kc_type / idx_kc_source created in run_migrations (avoid breaking old schema)

        -- idx_dp_document_id deferred to run_migrations (column may not exist yet on old DBs)
        CREATE TABLE IF NOT EXISTS vector_cache (chunk_id INTEGER PRIMARY KEY, created_at TEXT DEFAULT (datetime('now', 'localtime')));

        -- ★ 统一源数据表：所有实体的清洗后全文
        CREATE TABLE IF NOT EXISTS source_data (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            source_type TEXT NOT NULL, source_id TEXT NOT NULL,
            title TEXT DEFAULT '', content TEXT NOT NULL DEFAULT '',
            created_at TEXT DEFAULT (datetime('now', 'localtime')),
            updated_at TEXT DEFAULT (datetime('now', 'localtime')),
            UNIQUE(source_type, source_id)
        );

        -- ★ FTS5: kk_chunks_fts — manual content-sync via triggers (child-only)
        CREATE VIRTUAL TABLE IF NOT EXISTS kk_chunks_fts USING fts5(
            doc_title, content,
            tokenize='unicode61'
        );

        -- ★ 大一统全局搜索引擎 (external content on source_data)
        CREATE VIRTUAL TABLE IF NOT EXISTS global_search_fts USING fts5(source_type UNINDEXED, source_id UNINDEXED, title, content, content='source_data', content_rowid='rowid');
        CREATE TABLE IF NOT EXISTS system_notifications (id INTEGER PRIMARY KEY AUTOINCREMENT, notification_type TEXT DEFAULT 'general', project_id TEXT, payload TEXT DEFAULT '{}', is_read INTEGER DEFAULT 0, created_at TEXT DEFAULT (datetime('now', 'localtime')));
        CREATE TABLE IF NOT EXISTS system_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at TEXT DEFAULT (datetime('now', 'localtime')));
        CREATE TABLE IF NOT EXISTS user_policies (id INTEGER PRIMARY KEY AUTOINCREMENT, content TEXT NOT NULL, created_at TEXT DEFAULT (datetime('now', 'localtime')), is_active INTEGER DEFAULT 1);
    ")?;
    Ok(())
}

// Type conversion
fn json_to_sqlite(val: &serde_json::Value) -> rusqlite::types::Value {
    match val { serde_json::Value::Null => rusqlite::types::Value::Null, serde_json::Value::Bool(b) => rusqlite::types::Value::Integer(if *b{1}else{0}), serde_json::Value::Number(n) => { if let Some(i)=n.as_i64(){rusqlite::types::Value::Integer(i)}else if let Some(f)=n.as_f64(){rusqlite::types::Value::Real(f)}else{rusqlite::types::Value::Text(n.to_string())} }, serde_json::Value::String(s) => rusqlite::types::Value::Text(s.clone()), _ => rusqlite::types::Value::Text(val.to_string()) }
}
fn sqlite_to_json(val: rusqlite::types::Value) -> serde_json::Value {
    match val { rusqlite::types::Value::Null=>serde_json::Value::Null, rusqlite::types::Value::Integer(i)=>serde_json::Value::Number(i.into()), rusqlite::types::Value::Real(f)=>serde_json::json!(f), rusqlite::types::Value::Text(s)=>serde_json::Value::String(s), rusqlite::types::Value::Blob(b)=>serde_json::Value::String(base64::engine::general_purpose::STANDARD.encode(&b)) }
}

#[derive(Debug,Clone,serde::Serialize)]
pub struct RunResult { pub changes: u32, pub last_insert_rowid: i64 }

pub(crate) fn all(sql: &str, params: &[serde_json::Value]) -> rusqlite::Result<Vec<serde_json::Value>> {
    with_db(|conn| {
        let mut stmt = conn.prepare(sql)?;
        let cols: Vec<String> = stmt.column_names().iter().map(|c|c.to_string()).collect();
        let p: Vec<rusqlite::types::Value> = params.iter().map(json_to_sqlite).collect();
        let rows = stmt.query_map(rusqlite::params_from_iter(p.iter()), |row| {
            let mut map = serde_json::Map::new();
            for (i,name) in cols.iter().enumerate() { map.insert(name.clone(), sqlite_to_json(row.get_unwrap(i))); }
            Ok(serde_json::Value::Object(map))
        })?;
        Ok(rows.filter_map(|r|r.ok()).collect())
    })
}

/// ★ Event Bus: 对象变更事件
pub(crate) fn emit_event(
    event_type: &str, object_type: &str, object_id: &str,
    payload: serde_json::Value, source: &str, trace_id: &str,
) {
    run(
        "INSERT INTO event_store(event_type,object_type,object_id,payload,source,trace_id) VALUES(?1,?2,?3,?4,?5,?6)",
        &[json!(event_type), json!(object_type), json!(object_id), payload, json!(source), json!(trace_id)],
    ).ok();
}

pub(crate) fn get(sql: &str, params: &[serde_json::Value]) -> rusqlite::Result<Option<serde_json::Value>> {
    all(sql, params).map(|rows| rows.into_iter().next())
}

pub(crate) fn run(sql: &str, params: &[serde_json::Value]) -> rusqlite::Result<RunResult> {
    with_db(|conn| {
        let p: Vec<rusqlite::types::Value> = params.iter().map(json_to_sqlite).collect();
        let mut stmt = conn.prepare(sql)?;
        let changes = stmt.execute(rusqlite::params_from_iter(p.iter()))?;
        Ok(RunResult { changes: changes as u32, last_insert_rowid: conn.last_insert_rowid() })
    })
}

pub(crate) fn save() -> rusqlite::Result<()> {
    with_db(|conn| conn.pragma_update(None, "wal_checkpoint", "TRUNCATE"))
}

// FTS5 triggers init — external content on source_data
/// 无条件创建结构化数据的 source_data 同步触发器（不论 Premium 许可）
/// PRODUCT / PROJECT / TODO 等结构化对象的 CRUD 需要实时同步到 source_data
fn init_struct_source_data_triggers(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    // ── source_data triggers: notes ──
    let _ = conn.execute_batch(
        "DROP TRIGGER IF EXISTS sd_notes_insert;
         DROP TRIGGER IF EXISTS sd_notes_update;
         DROP TRIGGER IF EXISTS sd_notes_delete;
         CREATE TRIGGER sd_notes_insert AFTER INSERT ON notes BEGIN
           INSERT INTO source_data(source_type,source_id,title,content,updated_at)
           VALUES('NOTE',new.note_id,new.title,
             'title: '||new.title||
             CASE WHEN COALESCE(new.content,'')!='' THEN CHAR(10)||'content: '||new.content ELSE '' END||
             CASE WHEN COALESCE(new.notebook_id,'')!=''
               THEN CHAR(10)||'notebook: '||COALESCE((SELECT name FROM notebooks WHERE id=new.notebook_id), new.notebook_id)
               ELSE '' END||
             CHAR(10)||'created: '||COALESCE(new.created_at, datetime('now','localtime'))||
             CHAR(10)||'updated: '||COALESCE(new.updated_at, datetime('now','localtime'))||
             CASE WHEN (SELECT COUNT(*) FROM note_project_mapping npm JOIN projects p ON p.id=npm.project_id WHERE npm.note_id=new.note_id)>0
               THEN CHAR(10)||'projects: '||(SELECT GROUP_CONCAT(p.name, ', ') FROM note_project_mapping npm JOIN projects p ON p.id=npm.project_id WHERE npm.note_id=new.note_id)
             ELSE '' END,
             datetime('now', 'localtime'))
           ON CONFLICT(source_type,source_id) DO UPDATE SET title=excluded.title,content=excluded.content,updated_at=excluded.updated_at;
         END;
         CREATE TRIGGER sd_notes_update AFTER UPDATE ON notes BEGIN
           INSERT INTO source_data(source_type,source_id,title,content,updated_at)
           VALUES('NOTE',new.note_id,new.title,
             'title: '||new.title||
             CASE WHEN COALESCE(new.content,'')!='' THEN CHAR(10)||'content: '||new.content ELSE '' END||
             CASE WHEN COALESCE(new.notebook_id,'')!=''
               THEN CHAR(10)||'notebook: '||COALESCE((SELECT name FROM notebooks WHERE id=new.notebook_id), new.notebook_id)
               ELSE '' END||
             CHAR(10)||'created: '||COALESCE(new.created_at, datetime('now','localtime'))||
             CHAR(10)||'updated: '||COALESCE(new.updated_at, datetime('now','localtime'))||
             CASE WHEN (SELECT COUNT(*) FROM note_project_mapping npm JOIN projects p ON p.id=npm.project_id WHERE npm.note_id=new.note_id)>0
               THEN CHAR(10)||'projects: '||(SELECT GROUP_CONCAT(p.name, ', ') FROM note_project_mapping npm JOIN projects p ON p.id=npm.project_id WHERE npm.note_id=new.note_id)
             ELSE '' END,
             datetime('now', 'localtime'))
           ON CONFLICT(source_type,source_id) DO UPDATE SET title=excluded.title,content=excluded.content,updated_at=excluded.updated_at;
         END;
         -- sd_notes_delete: handled by Rust knowledge_delete()"
    );
    // ── source_data triggers: products ──
    let _ = conn.execute_batch(
        "DROP TRIGGER IF EXISTS sd_products_insert;
         DROP TRIGGER IF EXISTS sd_products_update;
         DROP TRIGGER IF EXISTS sd_products_delete;
         CREATE TRIGGER sd_products_insert AFTER INSERT ON products BEGIN
           INSERT INTO source_data(source_type,source_id,title,content,updated_at)
           VALUES('PRODUCT',new.id,new.name,
             'name: '||new.name||
             CASE WHEN COALESCE(new.description,'')!='' THEN CHAR(10)||'description: '||new.description ELSE '' END||
             CASE WHEN COALESCE(new.series_name,'')!='' THEN CHAR(10)||'series_name: '||new.series_name ELSE '' END||
             CASE WHEN new.price>0 THEN CHAR(10)||'price: '||printf('%.2f',new.price) ELSE '' END||
             CASE WHEN COALESCE(new.standard_specs,'')!='' THEN CHAR(10)||'standard_specs: '||new.standard_specs ELSE '' END||
             CASE WHEN COALESCE(new.full_specs,'')!='' THEN CHAR(10)||'full_specs: '||new.full_specs ELSE '' END||
             CASE WHEN new.series_id IS NOT NULL AND new.series_id!=''
               THEN CHAR(10)||'path: '||IFNULL(
                 (WITH RECURSIVE sp AS (SELECT name,parent_id,0 AS d FROM product_series WHERE id=new.series_id
                   UNION ALL SELECT ps.name,ps.parent_id,sp.d+1 FROM product_series ps JOIN sp ON ps.id=sp.parent_id)
                  SELECT GROUP_CONCAT(name,' / ' ORDER BY d DESC) FROM sp),'')
               ELSE '' END,
             datetime('now', 'localtime'))
           ON CONFLICT(source_type,source_id) DO UPDATE SET title=excluded.title,content=excluded.content,updated_at=excluded.updated_at;
         END;
         CREATE TRIGGER sd_products_update AFTER UPDATE ON products BEGIN
           INSERT INTO source_data(source_type,source_id,title,content,updated_at)
           VALUES('PRODUCT',new.id,new.name,
             'name: '||new.name||
             CASE WHEN COALESCE(new.description,'')!='' THEN CHAR(10)||'description: '||new.description ELSE '' END||
             CASE WHEN COALESCE(new.series_name,'')!='' THEN CHAR(10)||'series_name: '||new.series_name ELSE '' END||
             CASE WHEN new.price>0 THEN CHAR(10)||'price: '||printf('%.2f',new.price) ELSE '' END||
             CASE WHEN COALESCE(new.standard_specs,'')!='' THEN CHAR(10)||'standard_specs: '||new.standard_specs ELSE '' END||
             CASE WHEN COALESCE(new.full_specs,'')!='' THEN CHAR(10)||'full_specs: '||new.full_specs ELSE '' END||
             CASE WHEN new.series_id IS NOT NULL AND new.series_id!=''
               THEN CHAR(10)||'path: '||IFNULL(
                 (WITH RECURSIVE sp AS (SELECT name,parent_id,0 AS d FROM product_series WHERE id=new.series_id
                   UNION ALL SELECT ps.name,ps.parent_id,sp.d+1 FROM product_series ps JOIN sp ON ps.id=sp.parent_id)
                  SELECT GROUP_CONCAT(name,' / ' ORDER BY d DESC) FROM sp),'')
               ELSE '' END,
             datetime('now', 'localtime'))
           ON CONFLICT(source_type,source_id) DO UPDATE SET title=excluded.title,content=excluded.content,updated_at=excluded.updated_at;
         END;
         -- sd_products_delete: handled by Rust knowledge_delete()"
    );
    // ── source_data triggers: projects（键值对格式）──
    let _ = conn.execute_batch(
        "DROP TRIGGER IF EXISTS sd_projects_insert;
         DROP TRIGGER IF EXISTS sd_projects_update;
         DROP TRIGGER IF EXISTS sd_projects_delete;
         CREATE TRIGGER sd_projects_insert AFTER INSERT ON projects BEGIN
           INSERT INTO source_data(source_type,source_id,title,content,updated_at)
           VALUES('PROJECT',new.id,new.name,
             'name: '||new.name||
             CASE WHEN COALESCE(new.status,'')!='' THEN CHAR(10)||'status: '||new.status ELSE '' END||
             CASE WHEN COALESCE(new.owner,'')!='' THEN CHAR(10)||'owner: '||new.owner ELSE '' END||
             CASE WHEN COALESCE(new.customer,'')!='' THEN CHAR(10)||'customer: '||new.customer ELSE '' END||
             CASE WHEN new.budget>0 THEN CHAR(10)||'budget: '||printf('%.2f',new.budget) ELSE '' END||
             CASE WHEN COALESCE(new.description,'')!='' THEN CHAR(10)||'description: '||new.description ELSE '' END||
             CASE WHEN COALESCE(new.bid_time,'')!='' THEN CHAR(10)||'bid_time: '||new.bid_time ELSE '' END||
             CASE WHEN COALESCE(new.customer_contact,'')!='' THEN CHAR(10)||'customer_contact: '||new.customer_contact ELSE '' END||
             CASE WHEN COALESCE(new.customer_phone,'')!='' THEN CHAR(10)||'customer_phone: '||new.customer_phone ELSE '' END||
             CHAR(10)||'solution_products: '||COALESCE((SELECT GROUP_CONCAT(DISTINCT si.product_name) FROM solution_items si JOIN solution_versions sv ON sv.id=si.solution_version_id AND sv.status='final' JOIN solutions s ON s.id=sv.solution_id WHERE s.project_id=new.id AND COALESCE(si.product_name,'')!=''), '')||
             CHAR(10)||'dynamics: '||COALESCE((SELECT GROUP_CONCAT(pd.type||': '||pd.content||' ('||pd.created_at||')', ' | ') FROM project_dynamics pd WHERE pd.project_id=new.id ORDER BY pd.created_at), '')||
             CHAR(10)||'channel_partners: '||COALESCE((SELECT GROUP_CONCAT(COALESCE(cp.company, cp.name)||'（'||cp.name||COALESCE('-'||cp.phone,'')||'）'||COALESCE(' ['||(SELECT GROUP_CONCAT(cc.name||COALESCE('-'||cc.phone,'')||COALESCE('('||cc.remark||')',''), '; ') FROM channel_contacts cc WHERE cc.channel_id=cp.id AND COALESCE(cc.name,'')!='')||']', ''), ', ') FROM channel_partners cp WHERE cp.project_id=new.id AND COALESCE(cp.name,'')!=''), '')||
             CHAR(10)||'vendor_partners: '||COALESCE((SELECT GROUP_CONCAT(COALESCE(vp.company, vp.name)||'（'||vp.name||COALESCE('-'||vp.phone,'')||'）'||COALESCE(' ['||(SELECT GROUP_CONCAT(vc.name||COALESCE('-'||vc.phone,'')||COALESCE('('||vc.remark||')',''), '; ') FROM vendor_contacts vc WHERE vc.vendor_id=vp.id AND COALESCE(vc.name,'')!='')||']', ''), ', ') FROM vendor_partners vp WHERE vp.project_id=new.id AND COALESCE(vp.name,'')!=''), ''),
             datetime('now', 'localtime'))
           ON CONFLICT(source_type,source_id) DO UPDATE SET title=excluded.title,content=excluded.content,updated_at=excluded.updated_at;
         END;
         CREATE TRIGGER sd_projects_update AFTER UPDATE ON projects BEGIN
           INSERT INTO source_data(source_type,source_id,title,content,updated_at)
           VALUES('PROJECT',new.id,new.name,
             'name: '||new.name||
             CASE WHEN COALESCE(new.status,'')!='' THEN CHAR(10)||'status: '||new.status ELSE '' END||
             CASE WHEN COALESCE(new.owner,'')!='' THEN CHAR(10)||'owner: '||new.owner ELSE '' END||
             CASE WHEN COALESCE(new.customer,'')!='' THEN CHAR(10)||'customer: '||new.customer ELSE '' END||
             CASE WHEN new.budget>0 THEN CHAR(10)||'budget: '||printf('%.2f',new.budget) ELSE '' END||
             CASE WHEN COALESCE(new.description,'')!='' THEN CHAR(10)||'description: '||new.description ELSE '' END||
             CASE WHEN COALESCE(new.bid_time,'')!='' THEN CHAR(10)||'bid_time: '||new.bid_time ELSE '' END||
             CASE WHEN COALESCE(new.customer_contact,'')!='' THEN CHAR(10)||'customer_contact: '||new.customer_contact ELSE '' END||
             CASE WHEN COALESCE(new.customer_phone,'')!='' THEN CHAR(10)||'customer_phone: '||new.customer_phone ELSE '' END||
             CHAR(10)||'solution_products: '||COALESCE((SELECT GROUP_CONCAT(DISTINCT si.product_name) FROM solution_items si JOIN solution_versions sv ON sv.id=si.solution_version_id AND sv.status='final' JOIN solutions s ON s.id=sv.solution_id WHERE s.project_id=new.id AND COALESCE(si.product_name,'')!=''), '')||
             CHAR(10)||'dynamics: '||COALESCE((SELECT GROUP_CONCAT(pd.type||': '||pd.content||' ('||pd.created_at||')', ' | ') FROM project_dynamics pd WHERE pd.project_id=new.id ORDER BY pd.created_at), '')||
             CHAR(10)||'channel_partners: '||COALESCE((SELECT GROUP_CONCAT(COALESCE(cp.company, cp.name)||'（'||cp.name||COALESCE('-'||cp.phone,'')||'）'||COALESCE(' ['||(SELECT GROUP_CONCAT(cc.name||COALESCE('-'||cc.phone,'')||COALESCE('('||cc.remark||')',''), '; ') FROM channel_contacts cc WHERE cc.channel_id=cp.id AND COALESCE(cc.name,'')!='')||']', ''), ', ') FROM channel_partners cp WHERE cp.project_id=new.id AND COALESCE(cp.name,'')!=''), '')||
             CHAR(10)||'vendor_partners: '||COALESCE((SELECT GROUP_CONCAT(COALESCE(vp.company, vp.name)||'（'||vp.name||COALESCE('-'||vp.phone,'')||'）'||COALESCE(' ['||(SELECT GROUP_CONCAT(vc.name||COALESCE('-'||vc.phone,'')||COALESCE('('||vc.remark||')',''), '; ') FROM vendor_contacts vc WHERE vc.vendor_id=vp.id AND COALESCE(vc.name,'')!='')||']', ''), ', ') FROM vendor_partners vp WHERE vp.project_id=new.id AND COALESCE(vp.name,'')!=''), ''),
             datetime('now', 'localtime'))
           ON CONFLICT(source_type,source_id) DO UPDATE SET title=excluded.title,content=excluded.content,updated_at=excluded.updated_at;
         END;
         -- sd_projects_delete: handled by Rust knowledge_delete()
         -- solution_items 变更时同步更新对应项目的 source_data（含方案产品 + 动态）
         DROP TRIGGER IF EXISTS sd_project_solution_insert;
         CREATE TRIGGER sd_project_solution_insert AFTER INSERT ON solution_items
         BEGIN
           INSERT INTO source_data(source_type,source_id,title,content,updated_at)
           SELECT 'PROJECT',p.id,p.name,
             'name: '||p.name||
             CASE WHEN COALESCE(p.status,'')!='' THEN CHAR(10)||'status: '||p.status ELSE '' END||
             CASE WHEN COALESCE(p.owner,'')!='' THEN CHAR(10)||'owner: '||p.owner ELSE '' END||
             CASE WHEN COALESCE(p.customer,'')!='' THEN CHAR(10)||'customer: '||p.customer ELSE '' END||
             CASE WHEN p.budget>0 THEN CHAR(10)||'budget: '||printf('%.2f',p.budget) ELSE '' END||
             CASE WHEN COALESCE(p.description,'')!='' THEN CHAR(10)||'description: '||p.description ELSE '' END||
             CASE WHEN COALESCE(p.bid_time,'')!='' THEN CHAR(10)||'bid_time: '||p.bid_time ELSE '' END||
             CASE WHEN COALESCE(p.customer_contact,'')!='' THEN CHAR(10)||'customer_contact: '||p.customer_contact ELSE '' END||
             CASE WHEN COALESCE(p.customer_phone,'')!='' THEN CHAR(10)||'customer_phone: '||p.customer_phone ELSE '' END||
             CHAR(10)||'solution_products: '||
             COALESCE((SELECT GROUP_CONCAT(DISTINCT si2.product_name)
               FROM solution_items si2
               JOIN solution_versions sv2 ON sv2.id=si2.solution_version_id AND sv2.status='final'
               JOIN solutions s2 ON s2.id=sv2.solution_id WHERE s2.project_id=p.id AND COALESCE(si2.product_name,'')!=''), '')||
             CHAR(10)||'dynamics: '||
             COALESCE((SELECT GROUP_CONCAT(pd.type||': '||pd.content||' ('||pd.created_at||')', ' | ')
               FROM project_dynamics pd
               WHERE pd.project_id=p.id
               ORDER BY pd.created_at), '')||
             CHAR(10)||'channel_partners: '||
             COALESCE((SELECT GROUP_CONCAT(COALESCE(cp.company, cp.name)||'（'||cp.name||COALESCE('-'||cp.phone,'')||'）'||COALESCE(' ['||(SELECT GROUP_CONCAT(cc.name||COALESCE('-'||cc.phone,'')||COALESCE('('||cc.remark||')',''), '; ') FROM channel_contacts cc WHERE cc.channel_id=cp.id AND COALESCE(cc.name,'')!='')||']', ''), ', ') FROM channel_partners cp WHERE cp.project_id=p.id AND COALESCE(cp.name,'')!=''), '')||
             CHAR(10)||'vendor_partners: '||
             COALESCE((SELECT GROUP_CONCAT(COALESCE(vp.company, vp.name)||'（'||vp.name||COALESCE('-'||vp.phone,'')||'）'||COALESCE(' ['||(SELECT GROUP_CONCAT(vc.name||COALESCE('-'||vc.phone,'')||COALESCE('('||vc.remark||')',''), '; ') FROM vendor_contacts vc WHERE vc.vendor_id=vp.id AND COALESCE(vc.name,'')!='')||']', ''), ', ') FROM vendor_partners vp WHERE vp.project_id=p.id AND COALESCE(vp.name,'')!=''), ''),
             datetime('now', 'localtime')
           FROM projects p
           WHERE p.id=(SELECT s.project_id FROM solutions s JOIN solution_versions sv ON sv.solution_id=s.id WHERE sv.id=new.solution_version_id)
           ON CONFLICT(source_type,source_id) DO UPDATE SET content=excluded.content,updated_at=excluded.updated_at;
         END;
         -- solution_items UPDATE → 刷新关联项目的 source_data
         DROP TRIGGER IF EXISTS sd_project_solution_update;
         CREATE TRIGGER sd_project_solution_update AFTER UPDATE ON solution_items
         BEGIN
           INSERT INTO source_data(source_type,source_id,title,content,updated_at)
           SELECT 'PROJECT',p.id,p.name,
             'name: '||p.name||
             CASE WHEN COALESCE(p.status,'')!='' THEN CHAR(10)||'status: '||p.status ELSE '' END||
             CASE WHEN COALESCE(p.owner,'')!='' THEN CHAR(10)||'owner: '||p.owner ELSE '' END||
             CASE WHEN COALESCE(p.customer,'')!='' THEN CHAR(10)||'customer: '||p.customer ELSE '' END||
             CASE WHEN p.budget>0 THEN CHAR(10)||'budget: '||printf('%.2f',p.budget) ELSE '' END||
             CASE WHEN COALESCE(p.description,'')!='' THEN CHAR(10)||'description: '||p.description ELSE '' END||
             CASE WHEN COALESCE(p.bid_time,'')!='' THEN CHAR(10)||'bid_time: '||p.bid_time ELSE '' END||
             CASE WHEN COALESCE(p.customer_contact,'')!='' THEN CHAR(10)||'customer_contact: '||p.customer_contact ELSE '' END||
             CASE WHEN COALESCE(p.customer_phone,'')!='' THEN CHAR(10)||'customer_phone: '||p.customer_phone ELSE '' END||
             CHAR(10)||'solution_products: '||
             COALESCE((SELECT GROUP_CONCAT(DISTINCT si.product_name)
               FROM solution_items si
               JOIN solution_versions sv ON sv.id=si.solution_version_id AND sv.status='final'
               JOIN solutions s ON s.id=sv.solution_id WHERE s.project_id=p.id AND COALESCE(si.product_name,'')!=''), '')||
             CHAR(10)||'dynamics: '||
             COALESCE((SELECT GROUP_CONCAT(pd.type||': '||pd.content||' ('||pd.created_at||')', ' | ')
               FROM project_dynamics pd
               WHERE pd.project_id=p.id
               ORDER BY pd.created_at), '')||
             CHAR(10)||'channel_partners: '||
             COALESCE((SELECT GROUP_CONCAT(COALESCE(cp.company, cp.name)||'（'||cp.name||COALESCE('-'||cp.phone,'')||'）'||COALESCE(' ['||(SELECT GROUP_CONCAT(cc.name||COALESCE('-'||cc.phone,'')||COALESCE('('||cc.remark||')',''), '; ') FROM channel_contacts cc WHERE cc.channel_id=cp.id AND COALESCE(cc.name,'')!='')||']', ''), ', ') FROM channel_partners cp WHERE cp.project_id=p.id AND COALESCE(cp.name,'')!=''), '')||
             CHAR(10)||'vendor_partners: '||
             COALESCE((SELECT GROUP_CONCAT(COALESCE(vp.company, vp.name)||'（'||vp.name||COALESCE('-'||vp.phone,'')||'）'||COALESCE(' ['||(SELECT GROUP_CONCAT(vc.name||COALESCE('-'||vc.phone,'')||COALESCE('('||vc.remark||')',''), '; ') FROM vendor_contacts vc WHERE vc.vendor_id=vp.id AND COALESCE(vc.name,'')!='')||']', ''), ', ') FROM vendor_partners vp WHERE vp.project_id=p.id AND COALESCE(vp.name,'')!=''), ''),
             datetime('now', 'localtime')
           FROM projects p
           WHERE p.id=(SELECT s.project_id FROM solutions s JOIN solution_versions sv ON sv.solution_id=s.id WHERE sv.id=old.solution_version_id)
           ON CONFLICT(source_type,source_id) DO UPDATE SET content=excluded.content,updated_at=excluded.updated_at;
         END;
         -- solution_items DELETE → 刷新关联项目的 source_data
         DROP TRIGGER IF EXISTS sd_project_solution_delete;
         CREATE TRIGGER sd_project_solution_delete AFTER DELETE ON solution_items
         BEGIN
           INSERT INTO source_data(source_type,source_id,title,content,updated_at)
           SELECT 'PROJECT',p.id,p.name,
             'name: '||p.name||
             CASE WHEN COALESCE(p.status,'')!='' THEN CHAR(10)||'status: '||p.status ELSE '' END||
             CASE WHEN COALESCE(p.owner,'')!='' THEN CHAR(10)||'owner: '||p.owner ELSE '' END||
             CASE WHEN COALESCE(p.customer,'')!='' THEN CHAR(10)||'customer: '||p.customer ELSE '' END||
             CASE WHEN p.budget>0 THEN CHAR(10)||'budget: '||printf('%.2f',p.budget) ELSE '' END||
             CASE WHEN COALESCE(p.description,'')!='' THEN CHAR(10)||'description: '||p.description ELSE '' END||
             CASE WHEN COALESCE(p.bid_time,'')!='' THEN CHAR(10)||'bid_time: '||p.bid_time ELSE '' END||
             CASE WHEN COALESCE(p.customer_contact,'')!='' THEN CHAR(10)||'customer_contact: '||p.customer_contact ELSE '' END||
             CASE WHEN COALESCE(p.customer_phone,'')!='' THEN CHAR(10)||'customer_phone: '||p.customer_phone ELSE '' END||
             CHAR(10)||'solution_products: '||
             COALESCE((SELECT GROUP_CONCAT(DISTINCT si.product_name)
               FROM solution_items si
               JOIN solution_versions sv ON sv.id=si.solution_version_id AND sv.status='final'
               JOIN solutions s ON s.id=sv.solution_id WHERE s.project_id=p.id AND COALESCE(si.product_name,'')!=''), '')||
             CHAR(10)||'dynamics: '||
             COALESCE((SELECT GROUP_CONCAT(pd.type||': '||pd.content||' ('||pd.created_at||')', ' | ')
               FROM project_dynamics pd
               WHERE pd.project_id=p.id
               ORDER BY pd.created_at), '')||
             CHAR(10)||'channel_partners: '||
             COALESCE((SELECT GROUP_CONCAT(COALESCE(cp.company, cp.name)||'（'||cp.name||COALESCE('-'||cp.phone,'')||'）'||COALESCE(' ['||(SELECT GROUP_CONCAT(cc.name||COALESCE('-'||cc.phone,'')||COALESCE('('||cc.remark||')',''), '; ') FROM channel_contacts cc WHERE cc.channel_id=cp.id AND COALESCE(cc.name,'')!='')||']', ''), ', ') FROM channel_partners cp WHERE cp.project_id=p.id AND COALESCE(cp.name,'')!=''), '')||
             CHAR(10)||'vendor_partners: '||
             COALESCE((SELECT GROUP_CONCAT(COALESCE(vp.company, vp.name)||'（'||vp.name||COALESCE('-'||vp.phone,'')||'）'||COALESCE(' ['||(SELECT GROUP_CONCAT(vc.name||COALESCE('-'||vc.phone,'')||COALESCE('('||vc.remark||')',''), '; ') FROM vendor_contacts vc WHERE vc.vendor_id=vp.id AND COALESCE(vc.name,'')!='')||']', ''), ', ') FROM vendor_partners vp WHERE vp.project_id=p.id AND COALESCE(vp.name,'')!=''), ''),
             datetime('now', 'localtime')
           FROM projects p
           WHERE p.id=(SELECT s.project_id FROM solutions s JOIN solution_versions sv ON sv.solution_id=s.id WHERE sv.id=old.solution_version_id)
           ON CONFLICT(source_type,source_id) DO UPDATE SET content=excluded.content,updated_at=excluded.updated_at;
         END;"
    );
    // ── source_data triggers: todos ──
    let _ = conn.execute_batch(
        "DROP TRIGGER IF EXISTS sd_todos_insert;
         DROP TRIGGER IF EXISTS sd_todos_update;
         DROP TRIGGER IF EXISTS sd_todos_delete;
         CREATE TRIGGER sd_todos_insert AFTER INSERT ON todos BEGIN
           INSERT INTO source_data(source_type,source_id,title,content,updated_at)
           VALUES('TODO',new.id,new.title,
             'title: '||new.title||
             CASE WHEN new.done=1 THEN CHAR(10)||'done: yes' ELSE '' END||
             CASE WHEN COALESCE(new.priority,'')!='' THEN CHAR(10)||'priority: '||new.priority ELSE '' END||
             CASE WHEN COALESCE(new.due_date,'')!='' THEN CHAR(10)||'due_date: '||new.due_date ELSE '' END||
             CASE WHEN COALESCE(new.repeat,'')!='' THEN CHAR(10)||'repeat: '||new.repeat ELSE '' END||
             CASE WHEN COALESCE(new.content,'')!='' THEN CHAR(10)||'content: '||new.content ELSE '' END,
             datetime('now', 'localtime'))
           ON CONFLICT(source_type,source_id) DO UPDATE SET title=excluded.title,content=excluded.content,updated_at=excluded.updated_at;
         END;
         CREATE TRIGGER sd_todos_update AFTER UPDATE ON todos BEGIN
           INSERT INTO source_data(source_type,source_id,title,content,updated_at)
           VALUES('TODO',new.id,new.title,
             'title: '||new.title||
             CASE WHEN new.done=1 THEN CHAR(10)||'done: yes' ELSE '' END||
             CASE WHEN COALESCE(new.priority,'')!='' THEN CHAR(10)||'priority: '||new.priority ELSE '' END||
             CASE WHEN COALESCE(new.due_date,'')!='' THEN CHAR(10)||'due_date: '||new.due_date ELSE '' END||
             CASE WHEN COALESCE(new.repeat,'')!='' THEN CHAR(10)||'repeat: '||new.repeat ELSE '' END||
             CASE WHEN COALESCE(new.content,'')!='' THEN CHAR(10)||'content: '||new.content ELSE '' END,
             datetime('now', 'localtime'))
           ON CONFLICT(source_type,source_id) DO UPDATE SET title=excluded.title,content=excluded.content,updated_at=excluded.updated_at;
         END;
         -- sd_todos_delete: handled by Rust knowledge_delete()"
    );
    // ── source_data triggers: INSERT/UPDATE by ingestion worker only ──
    // DELETE is handled explicitly in file_delete / notes_permanent_delete
    // (the old sd_files_delete trigger was removed — it deleted source_data
    //  prematurely when file_assets was shared by multiple file_versions)
    let _ = conn.execute_batch(
        "DROP TRIGGER IF EXISTS sd_files_delete;
         DROP TRIGGER IF EXISTS sd_dynamics_delete;
         DROP TRIGGER IF EXISTS sd_project_dynamics_insert;
         -- project_dynamics 新增时，刷新对应 PROJECT 的 source_data，追加 dynamics: 行
         CREATE TRIGGER sd_project_dynamics_insert AFTER INSERT ON project_dynamics
         BEGIN
           INSERT INTO source_data(source_type,source_id,title,content,updated_at)
           SELECT 'PROJECT',p.id,p.name,
             'name: '||p.name||
             CASE WHEN COALESCE(p.status,'')!='' THEN CHAR(10)||'status: '||p.status ELSE '' END||
             CASE WHEN COALESCE(p.owner,'')!='' THEN CHAR(10)||'owner: '||p.owner ELSE '' END||
             CASE WHEN COALESCE(p.customer,'')!='' THEN CHAR(10)||'customer: '||p.customer ELSE '' END||
             CASE WHEN p.budget>0 THEN CHAR(10)||'budget: '||printf('%.2f',p.budget) ELSE '' END||
             CASE WHEN COALESCE(p.description,'')!='' THEN CHAR(10)||'description: '||p.description ELSE '' END||
             CASE WHEN COALESCE(p.bid_time,'')!='' THEN CHAR(10)||'bid_time: '||p.bid_time ELSE '' END||
             CASE WHEN COALESCE(p.customer_contact,'')!='' THEN CHAR(10)||'customer_contact: '||p.customer_contact ELSE '' END||
             CASE WHEN COALESCE(p.customer_phone,'')!='' THEN CHAR(10)||'customer_phone: '||p.customer_phone ELSE '' END||
             CHAR(10)||'solution_products: '||
             COALESCE((SELECT GROUP_CONCAT(DISTINCT si.product_name)
               FROM solution_items si
               JOIN solution_versions sv ON sv.id=si.solution_version_id AND sv.status='final'
               JOIN solutions s ON s.id=sv.solution_id WHERE s.project_id=p.id AND COALESCE(si.product_name,'')!=''), '')||
             CHAR(10)||'dynamics: '||
             COALESCE((SELECT GROUP_CONCAT(pd.type||': '||pd.content||' ('||pd.created_at||')', ' | ')
               FROM project_dynamics pd
               WHERE pd.project_id=p.id
               ORDER BY pd.created_at), '')||
             CHAR(10)||'channel_partners: '||
             COALESCE((SELECT GROUP_CONCAT(COALESCE(cp.company, cp.name)||'（'||cp.name||COALESCE('-'||cp.phone,'')||'）'||COALESCE(' ['||(SELECT GROUP_CONCAT(cc.name||COALESCE('-'||cc.phone,'')||COALESCE('('||cc.remark||')',''), '; ') FROM channel_contacts cc WHERE cc.channel_id=cp.id AND COALESCE(cc.name,'')!='')||']', ''), ', ') FROM channel_partners cp WHERE cp.project_id=p.id AND COALESCE(cp.name,'')!=''), '')||
             CHAR(10)||'vendor_partners: '||
             COALESCE((SELECT GROUP_CONCAT(COALESCE(vp.company, vp.name)||'（'||vp.name||COALESCE('-'||vp.phone,'')||'）'||COALESCE(' ['||(SELECT GROUP_CONCAT(vc.name||COALESCE('-'||vc.phone,'')||COALESCE('('||vc.remark||')',''), '; ') FROM vendor_contacts vc WHERE vc.vendor_id=vp.id AND COALESCE(vc.name,'')!='')||']', ''), ', ') FROM vendor_partners vp WHERE vp.project_id=p.id AND COALESCE(vp.name,'')!=''), ''),
             datetime('now', 'localtime')
           FROM projects p
           WHERE p.id=new.project_id
           ON CONFLICT(source_type,source_id) DO UPDATE SET content=excluded.content,updated_at=excluded.updated_at;
         END;
         -- project_dynamics UPDATE → 刷新关联项目的 source_data
         DROP TRIGGER IF EXISTS sd_project_dynamics_update;
         CREATE TRIGGER sd_project_dynamics_update AFTER UPDATE ON project_dynamics
         BEGIN
           INSERT INTO source_data(source_type,source_id,title,content,updated_at)
           SELECT 'PROJECT',p.id,p.name,
             'name: '||p.name||
             CASE WHEN COALESCE(p.status,'')!='' THEN CHAR(10)||'status: '||p.status ELSE '' END||
             CASE WHEN COALESCE(p.owner,'')!='' THEN CHAR(10)||'owner: '||p.owner ELSE '' END||
             CASE WHEN COALESCE(p.customer,'')!='' THEN CHAR(10)||'customer: '||p.customer ELSE '' END||
             CASE WHEN p.budget>0 THEN CHAR(10)||'budget: '||printf('%.2f',p.budget) ELSE '' END||
             CASE WHEN COALESCE(p.description,'')!='' THEN CHAR(10)||'description: '||p.description ELSE '' END||
             CASE WHEN COALESCE(p.bid_time,'')!='' THEN CHAR(10)||'bid_time: '||p.bid_time ELSE '' END||
             CASE WHEN COALESCE(p.customer_contact,'')!='' THEN CHAR(10)||'customer_contact: '||p.customer_contact ELSE '' END||
             CASE WHEN COALESCE(p.customer_phone,'')!='' THEN CHAR(10)||'customer_phone: '||p.customer_phone ELSE '' END||
             CHAR(10)||'solution_products: '||
             COALESCE((SELECT GROUP_CONCAT(DISTINCT si.product_name) FROM solution_items si JOIN solution_versions sv ON sv.id=si.solution_version_id AND sv.status='final' JOIN solutions s ON s.id=sv.solution_id WHERE s.project_id=p.id AND COALESCE(si.product_name,'')!=''), '')||
             CHAR(10)||'dynamics: '||
             COALESCE((SELECT GROUP_CONCAT(pd.type||': '||pd.content||' ('||pd.created_at||')', ' | ') FROM project_dynamics pd WHERE pd.project_id=p.id ORDER BY pd.created_at), '')||
             CHAR(10)||'channel_partners: '||
             COALESCE((SELECT GROUP_CONCAT(COALESCE(cp.company, cp.name)||'（'||cp.name||COALESCE('-'||cp.phone,'')||'）'||COALESCE(' ['||(SELECT GROUP_CONCAT(cc.name||COALESCE('-'||cc.phone,'')||COALESCE('('||cc.remark||')',''), '; ') FROM channel_contacts cc WHERE cc.channel_id=cp.id AND COALESCE(cc.name,'')!='')||']', ''), ', ') FROM channel_partners cp WHERE cp.project_id=p.id AND COALESCE(cp.name,'')!=''), '')||
             CHAR(10)||'vendor_partners: '||
             COALESCE((SELECT GROUP_CONCAT(COALESCE(vp.company, vp.name)||'（'||vp.name||COALESCE('-'||vp.phone,'')||'）'||COALESCE(' ['||(SELECT GROUP_CONCAT(vc.name||COALESCE('-'||vc.phone,'')||COALESCE('('||vc.remark||')',''), '; ') FROM vendor_contacts vc WHERE vc.vendor_id=vp.id AND COALESCE(vc.name,'')!='')||']', ''), ', ') FROM vendor_partners vp WHERE vp.project_id=p.id AND COALESCE(vp.name,'')!=''), ''),
             datetime('now', 'localtime')
           FROM projects p
           WHERE p.id=new.project_id
           ON CONFLICT(source_type,source_id) DO UPDATE SET content=excluded.content,updated_at=excluded.updated_at;
         END;
         -- project_dynamics DELETE → 刷新关联项目的 source_data
         DROP TRIGGER IF EXISTS sd_project_dynamics_delete;
         CREATE TRIGGER sd_project_dynamics_delete AFTER DELETE ON project_dynamics
         BEGIN
           INSERT INTO source_data(source_type,source_id,title,content,updated_at)
           SELECT 'PROJECT',p.id,p.name,
             'name: '||p.name||
             CASE WHEN COALESCE(p.status,'')!='' THEN CHAR(10)||'status: '||p.status ELSE '' END||
             CASE WHEN COALESCE(p.owner,'')!='' THEN CHAR(10)||'owner: '||p.owner ELSE '' END||
             CASE WHEN COALESCE(p.customer,'')!='' THEN CHAR(10)||'customer: '||p.customer ELSE '' END||
             CASE WHEN p.budget>0 THEN CHAR(10)||'budget: '||printf('%.2f',p.budget) ELSE '' END||
             CASE WHEN COALESCE(p.description,'')!='' THEN CHAR(10)||'description: '||p.description ELSE '' END||
             CASE WHEN COALESCE(p.bid_time,'')!='' THEN CHAR(10)||'bid_time: '||p.bid_time ELSE '' END||
             CASE WHEN COALESCE(p.customer_contact,'')!='' THEN CHAR(10)||'customer_contact: '||p.customer_contact ELSE '' END||
             CASE WHEN COALESCE(p.customer_phone,'')!='' THEN CHAR(10)||'customer_phone: '||p.customer_phone ELSE '' END||
             CHAR(10)||'solution_products: '||
             COALESCE((SELECT GROUP_CONCAT(DISTINCT si.product_name) FROM solution_items si JOIN solution_versions sv ON sv.id=si.solution_version_id AND sv.status='final' JOIN solutions s ON s.id=sv.solution_id WHERE s.project_id=p.id AND COALESCE(si.product_name,'')!=''), '')||
             CHAR(10)||'dynamics: '||
             COALESCE((SELECT GROUP_CONCAT(pd.type||': '||pd.content||' ('||pd.created_at||')', ' | ') FROM project_dynamics pd WHERE pd.project_id=p.id ORDER BY pd.created_at), '')||
             CHAR(10)||'channel_partners: '||
             COALESCE((SELECT GROUP_CONCAT(COALESCE(cp.company, cp.name)||'（'||cp.name||COALESCE('-'||cp.phone,'')||'）'||COALESCE(' ['||(SELECT GROUP_CONCAT(cc.name||COALESCE('-'||cc.phone,'')||COALESCE('('||cc.remark||')',''), '; ') FROM channel_contacts cc WHERE cc.channel_id=cp.id AND COALESCE(cc.name,'')!='')||']', ''), ', ') FROM channel_partners cp WHERE cp.project_id=p.id AND COALESCE(cp.name,'')!=''), '')||
             CHAR(10)||'vendor_partners: '||
             COALESCE((SELECT GROUP_CONCAT(COALESCE(vp.company, vp.name)||'（'||vp.name||COALESCE('-'||vp.phone,'')||'）'||COALESCE(' ['||(SELECT GROUP_CONCAT(vc.name||COALESCE('-'||vc.phone,'')||COALESCE('('||vc.remark||')',''), '; ') FROM vendor_contacts vc WHERE vc.vendor_id=vp.id AND COALESCE(vc.name,'')!='')||']', ''), ', ') FROM vendor_partners vp WHERE vp.project_id=p.id AND COALESCE(vp.name,'')!=''), ''),
             datetime('now', 'localtime')
           FROM projects p
           WHERE p.id=old.project_id
           ON CONFLICT(source_type,source_id) DO UPDATE SET content=excluded.content,updated_at=excluded.updated_at;
         END;"
    );
    let _ = conn.execute_batch(
        "DROP TRIGGER IF EXISTS sd_channel_partners_insert;
         DROP TRIGGER IF EXISTS sd_channel_partners_update;
         DROP TRIGGER IF EXISTS sd_channel_partners_delete;
         DROP TRIGGER IF EXISTS sd_vendor_partners_insert;
         DROP TRIGGER IF EXISTS sd_vendor_partners_update;
         DROP TRIGGER IF EXISTS sd_vendor_partners_delete;
         -- channel_partners INSERT → 刷新关联项目的 source_data
         CREATE TRIGGER sd_channel_partners_insert AFTER INSERT ON channel_partners
         BEGIN
           INSERT INTO source_data(source_type,source_id,title,content,updated_at)
           SELECT 'PROJECT',p.id,p.name,
             'name: '||p.name||
             CASE WHEN COALESCE(p.status,'')!='' THEN CHAR(10)||'status: '||p.status ELSE '' END||
             CASE WHEN COALESCE(p.owner,'')!='' THEN CHAR(10)||'owner: '||p.owner ELSE '' END||
             CASE WHEN COALESCE(p.customer,'')!='' THEN CHAR(10)||'customer: '||p.customer ELSE '' END||
             CASE WHEN p.budget>0 THEN CHAR(10)||'budget: '||printf('%.2f',p.budget) ELSE '' END||
             CASE WHEN COALESCE(p.description,'')!='' THEN CHAR(10)||'description: '||p.description ELSE '' END||
             CASE WHEN COALESCE(p.bid_time,'')!='' THEN CHAR(10)||'bid_time: '||p.bid_time ELSE '' END||
             CASE WHEN COALESCE(p.customer_contact,'')!='' THEN CHAR(10)||'customer_contact: '||p.customer_contact ELSE '' END||
             CASE WHEN COALESCE(p.customer_phone,'')!='' THEN CHAR(10)||'customer_phone: '||p.customer_phone ELSE '' END||
             CHAR(10)||'solution_products: '||
             COALESCE((SELECT GROUP_CONCAT(DISTINCT si.product_name) FROM solution_items si JOIN solution_versions sv ON sv.id=si.solution_version_id AND sv.status='final' JOIN solutions s ON s.id=sv.solution_id WHERE s.project_id=p.id AND COALESCE(si.product_name,'')!=''), '')||
             CHAR(10)||'dynamics: '||
             COALESCE((SELECT GROUP_CONCAT(pd.type||': '||pd.content||' ('||pd.created_at||')', ' | ') FROM project_dynamics pd WHERE pd.project_id=p.id ORDER BY pd.created_at), '')||
             CHAR(10)||'channel_partners: '||
             COALESCE((SELECT GROUP_CONCAT(COALESCE(cp.company, cp.name)||'（'||cp.name||COALESCE('-'||cp.phone,'')||'）'||COALESCE(' ['||(SELECT GROUP_CONCAT(cc.name||COALESCE('-'||cc.phone,'')||COALESCE('('||cc.remark||')',''), '; ') FROM channel_contacts cc WHERE cc.channel_id=cp.id AND COALESCE(cc.name,'')!='')||']', ''), ', ') FROM channel_partners cp WHERE cp.project_id=p.id AND COALESCE(cp.name,'')!=''), '')||
             CHAR(10)||'vendor_partners: '||
             COALESCE((SELECT GROUP_CONCAT(COALESCE(vp.company, vp.name)||'（'||vp.name||COALESCE('-'||vp.phone,'')||'）'||COALESCE(' ['||(SELECT GROUP_CONCAT(vc.name||COALESCE('-'||vc.phone,'')||COALESCE('('||vc.remark||')',''), '; ') FROM vendor_contacts vc WHERE vc.vendor_id=vp.id AND COALESCE(vc.name,'')!='')||']', ''), ', ') FROM vendor_partners vp WHERE vp.project_id=p.id AND COALESCE(vp.name,'')!=''), ''),
             datetime('now', 'localtime')
           FROM projects p
           WHERE p.id=new.project_id
           ON CONFLICT(source_type,source_id) DO UPDATE SET content=excluded.content,updated_at=excluded.updated_at;
         END;
         -- channel_partners UPDATE → 刷新关联项目的 source_data
         CREATE TRIGGER sd_channel_partners_update AFTER UPDATE ON channel_partners
         BEGIN
           INSERT INTO source_data(source_type,source_id,title,content,updated_at)
           SELECT 'PROJECT',p.id,p.name,
             'name: '||p.name||
             CASE WHEN COALESCE(p.status,'')!='' THEN CHAR(10)||'status: '||p.status ELSE '' END||
             CASE WHEN COALESCE(p.owner,'')!='' THEN CHAR(10)||'owner: '||p.owner ELSE '' END||
             CASE WHEN COALESCE(p.customer,'')!='' THEN CHAR(10)||'customer: '||p.customer ELSE '' END||
             CASE WHEN p.budget>0 THEN CHAR(10)||'budget: '||printf('%.2f',p.budget) ELSE '' END||
             CASE WHEN COALESCE(p.description,'')!='' THEN CHAR(10)||'description: '||p.description ELSE '' END||
             CASE WHEN COALESCE(p.bid_time,'')!='' THEN CHAR(10)||'bid_time: '||p.bid_time ELSE '' END||
             CASE WHEN COALESCE(p.customer_contact,'')!='' THEN CHAR(10)||'customer_contact: '||p.customer_contact ELSE '' END||
             CASE WHEN COALESCE(p.customer_phone,'')!='' THEN CHAR(10)||'customer_phone: '||p.customer_phone ELSE '' END||
             CHAR(10)||'solution_products: '||
             COALESCE((SELECT GROUP_CONCAT(DISTINCT si.product_name) FROM solution_items si JOIN solution_versions sv ON sv.id=si.solution_version_id AND sv.status='final' JOIN solutions s ON s.id=sv.solution_id WHERE s.project_id=p.id AND COALESCE(si.product_name,'')!=''), '')||
             CHAR(10)||'dynamics: '||
             COALESCE((SELECT GROUP_CONCAT(pd.type||': '||pd.content||' ('||pd.created_at||')', ' | ') FROM project_dynamics pd WHERE pd.project_id=p.id ORDER BY pd.created_at), '')||
             CHAR(10)||'channel_partners: '||
             COALESCE((SELECT GROUP_CONCAT(COALESCE(cp.company, cp.name)||'（'||cp.name||COALESCE('-'||cp.phone,'')||'）'||COALESCE(' ['||(SELECT GROUP_CONCAT(cc.name||COALESCE('-'||cc.phone,'')||COALESCE('('||cc.remark||')',''), '; ') FROM channel_contacts cc WHERE cc.channel_id=cp.id AND COALESCE(cc.name,'')!='')||']', ''), ', ') FROM channel_partners cp WHERE cp.project_id=p.id AND COALESCE(cp.name,'')!=''), '')||
             CHAR(10)||'vendor_partners: '||
             COALESCE((SELECT GROUP_CONCAT(COALESCE(vp.company, vp.name)||'（'||vp.name||COALESCE('-'||vp.phone,'')||'）'||COALESCE(' ['||(SELECT GROUP_CONCAT(vc.name||COALESCE('-'||vc.phone,'')||COALESCE('('||vc.remark||')',''), '; ') FROM vendor_contacts vc WHERE vc.vendor_id=vp.id AND COALESCE(vc.name,'')!='')||']', ''), ', ') FROM vendor_partners vp WHERE vp.project_id=p.id AND COALESCE(vp.name,'')!=''), ''),
             datetime('now', 'localtime')
           FROM projects p
           WHERE p.id=new.project_id
           ON CONFLICT(source_type,source_id) DO UPDATE SET content=excluded.content,updated_at=excluded.updated_at;
         END;
         -- channel_partners DELETE → 刷新关联项目的 source_data
         CREATE TRIGGER sd_channel_partners_delete AFTER DELETE ON channel_partners
         BEGIN
           INSERT INTO source_data(source_type,source_id,title,content,updated_at)
           SELECT 'PROJECT',p.id,p.name,
             'name: '||p.name||
             CASE WHEN COALESCE(p.status,'')!='' THEN CHAR(10)||'status: '||p.status ELSE '' END||
             CASE WHEN COALESCE(p.owner,'')!='' THEN CHAR(10)||'owner: '||p.owner ELSE '' END||
             CASE WHEN COALESCE(p.customer,'')!='' THEN CHAR(10)||'customer: '||p.customer ELSE '' END||
             CASE WHEN p.budget>0 THEN CHAR(10)||'budget: '||printf('%.2f',p.budget) ELSE '' END||
             CASE WHEN COALESCE(p.description,'')!='' THEN CHAR(10)||'description: '||p.description ELSE '' END||
             CASE WHEN COALESCE(p.bid_time,'')!='' THEN CHAR(10)||'bid_time: '||p.bid_time ELSE '' END||
             CASE WHEN COALESCE(p.customer_contact,'')!='' THEN CHAR(10)||'customer_contact: '||p.customer_contact ELSE '' END||
             CASE WHEN COALESCE(p.customer_phone,'')!='' THEN CHAR(10)||'customer_phone: '||p.customer_phone ELSE '' END||
             CHAR(10)||'solution_products: '||
             COALESCE((SELECT GROUP_CONCAT(DISTINCT si.product_name) FROM solution_items si JOIN solution_versions sv ON sv.id=si.solution_version_id AND sv.status='final' JOIN solutions s ON s.id=sv.solution_id WHERE s.project_id=p.id AND COALESCE(si.product_name,'')!=''), '')||
             CHAR(10)||'dynamics: '||
             COALESCE((SELECT GROUP_CONCAT(pd.type||': '||pd.content||' ('||pd.created_at||')', ' | ') FROM project_dynamics pd WHERE pd.project_id=p.id ORDER BY pd.created_at), '')||
             CHAR(10)||'channel_partners: '||
             COALESCE((SELECT GROUP_CONCAT(COALESCE(cp.company, cp.name)||'（'||cp.name||COALESCE('-'||cp.phone,'')||'）'||COALESCE(' ['||(SELECT GROUP_CONCAT(cc.name||COALESCE('-'||cc.phone,'')||COALESCE('('||cc.remark||')',''), '; ') FROM channel_contacts cc WHERE cc.channel_id=cp.id AND COALESCE(cc.name,'')!='')||']', ''), ', ') FROM channel_partners cp WHERE cp.project_id=p.id AND COALESCE(cp.name,'')!=''), '')||
             CHAR(10)||'vendor_partners: '||
             COALESCE((SELECT GROUP_CONCAT(COALESCE(vp.company, vp.name)||'（'||vp.name||COALESCE('-'||vp.phone,'')||'）'||COALESCE(' ['||(SELECT GROUP_CONCAT(vc.name||COALESCE('-'||vc.phone,'')||COALESCE('('||vc.remark||')',''), '; ') FROM vendor_contacts vc WHERE vc.vendor_id=vp.id AND COALESCE(vc.name,'')!='')||']', ''), ', ') FROM vendor_partners vp WHERE vp.project_id=p.id AND COALESCE(vp.name,'')!=''), ''),
             datetime('now', 'localtime')
           FROM projects p
           WHERE p.id=old.project_id
           ON CONFLICT(source_type,source_id) DO UPDATE SET content=excluded.content,updated_at=excluded.updated_at;
         END;
         -- vendor_partners INSERT → 刷新关联项目的 source_data
         CREATE TRIGGER sd_vendor_partners_insert AFTER INSERT ON vendor_partners
         BEGIN
           INSERT INTO source_data(source_type,source_id,title,content,updated_at)
           SELECT 'PROJECT',p.id,p.name,
             'name: '||p.name||
             CASE WHEN COALESCE(p.status,'')!='' THEN CHAR(10)||'status: '||p.status ELSE '' END||
             CASE WHEN COALESCE(p.owner,'')!='' THEN CHAR(10)||'owner: '||p.owner ELSE '' END||
             CASE WHEN COALESCE(p.customer,'')!='' THEN CHAR(10)||'customer: '||p.customer ELSE '' END||
             CASE WHEN p.budget>0 THEN CHAR(10)||'budget: '||printf('%.2f',p.budget) ELSE '' END||
             CASE WHEN COALESCE(p.description,'')!='' THEN CHAR(10)||'description: '||p.description ELSE '' END||
             CASE WHEN COALESCE(p.bid_time,'')!='' THEN CHAR(10)||'bid_time: '||p.bid_time ELSE '' END||
             CASE WHEN COALESCE(p.customer_contact,'')!='' THEN CHAR(10)||'customer_contact: '||p.customer_contact ELSE '' END||
             CASE WHEN COALESCE(p.customer_phone,'')!='' THEN CHAR(10)||'customer_phone: '||p.customer_phone ELSE '' END||
             CHAR(10)||'solution_products: '||
             COALESCE((SELECT GROUP_CONCAT(DISTINCT si.product_name) FROM solution_items si JOIN solution_versions sv ON sv.id=si.solution_version_id AND sv.status='final' JOIN solutions s ON s.id=sv.solution_id WHERE s.project_id=p.id AND COALESCE(si.product_name,'')!=''), '')||
             CHAR(10)||'dynamics: '||
             COALESCE((SELECT GROUP_CONCAT(pd.type||': '||pd.content||' ('||pd.created_at||')', ' | ') FROM project_dynamics pd WHERE pd.project_id=p.id ORDER BY pd.created_at), '')||
             CHAR(10)||'channel_partners: '||
             COALESCE((SELECT GROUP_CONCAT(COALESCE(cp.company, cp.name)||'（'||cp.name||COALESCE('-'||cp.phone,'')||'）'||COALESCE(' ['||(SELECT GROUP_CONCAT(cc.name||COALESCE('-'||cc.phone,'')||COALESCE('('||cc.remark||')',''), '; ') FROM channel_contacts cc WHERE cc.channel_id=cp.id AND COALESCE(cc.name,'')!='')||']', ''), ', ') FROM channel_partners cp WHERE cp.project_id=p.id AND COALESCE(cp.name,'')!=''), '')||
             CHAR(10)||'vendor_partners: '||
             COALESCE((SELECT GROUP_CONCAT(COALESCE(vp.company, vp.name)||'（'||vp.name||COALESCE('-'||vp.phone,'')||'）'||COALESCE(' ['||(SELECT GROUP_CONCAT(vc.name||COALESCE('-'||vc.phone,'')||COALESCE('('||vc.remark||')',''), '; ') FROM vendor_contacts vc WHERE vc.vendor_id=vp.id AND COALESCE(vc.name,'')!='')||']', ''), ', ') FROM vendor_partners vp WHERE vp.project_id=p.id AND COALESCE(vp.name,'')!=''), ''),
             datetime('now', 'localtime')
           FROM projects p
           WHERE p.id=new.project_id
           ON CONFLICT(source_type,source_id) DO UPDATE SET content=excluded.content,updated_at=excluded.updated_at;
         END;
         -- vendor_partners UPDATE → 刷新关联项目的 source_data
         CREATE TRIGGER sd_vendor_partners_update AFTER UPDATE ON vendor_partners
         BEGIN
           INSERT INTO source_data(source_type,source_id,title,content,updated_at)
           SELECT 'PROJECT',p.id,p.name,
             'name: '||p.name||
             CASE WHEN COALESCE(p.status,'')!='' THEN CHAR(10)||'status: '||p.status ELSE '' END||
             CASE WHEN COALESCE(p.owner,'')!='' THEN CHAR(10)||'owner: '||p.owner ELSE '' END||
             CASE WHEN COALESCE(p.customer,'')!='' THEN CHAR(10)||'customer: '||p.customer ELSE '' END||
             CASE WHEN p.budget>0 THEN CHAR(10)||'budget: '||printf('%.2f',p.budget) ELSE '' END||
             CASE WHEN COALESCE(p.description,'')!='' THEN CHAR(10)||'description: '||p.description ELSE '' END||
             CASE WHEN COALESCE(p.bid_time,'')!='' THEN CHAR(10)||'bid_time: '||p.bid_time ELSE '' END||
             CASE WHEN COALESCE(p.customer_contact,'')!='' THEN CHAR(10)||'customer_contact: '||p.customer_contact ELSE '' END||
             CASE WHEN COALESCE(p.customer_phone,'')!='' THEN CHAR(10)||'customer_phone: '||p.customer_phone ELSE '' END||
             CHAR(10)||'solution_products: '||
             COALESCE((SELECT GROUP_CONCAT(DISTINCT si.product_name) FROM solution_items si JOIN solution_versions sv ON sv.id=si.solution_version_id AND sv.status='final' JOIN solutions s ON s.id=sv.solution_id WHERE s.project_id=p.id AND COALESCE(si.product_name,'')!=''), '')||
             CHAR(10)||'dynamics: '||
             COALESCE((SELECT GROUP_CONCAT(pd.type||': '||pd.content||' ('||pd.created_at||')', ' | ') FROM project_dynamics pd WHERE pd.project_id=p.id ORDER BY pd.created_at), '')||
             CHAR(10)||'channel_partners: '||
             COALESCE((SELECT GROUP_CONCAT(COALESCE(cp.company, cp.name)||'（'||cp.name||COALESCE('-'||cp.phone,'')||'）'||COALESCE(' ['||(SELECT GROUP_CONCAT(cc.name||COALESCE('-'||cc.phone,'')||COALESCE('('||cc.remark||')',''), '; ') FROM channel_contacts cc WHERE cc.channel_id=cp.id AND COALESCE(cc.name,'')!='')||']', ''), ', ') FROM channel_partners cp WHERE cp.project_id=p.id AND COALESCE(cp.name,'')!=''), '')||
             CHAR(10)||'vendor_partners: '||
             COALESCE((SELECT GROUP_CONCAT(COALESCE(vp.company, vp.name)||'（'||vp.name||COALESCE('-'||vp.phone,'')||'）'||COALESCE(' ['||(SELECT GROUP_CONCAT(vc.name||COALESCE('-'||vc.phone,'')||COALESCE('('||vc.remark||')',''), '; ') FROM vendor_contacts vc WHERE vc.vendor_id=vp.id AND COALESCE(vc.name,'')!='')||']', ''), ', ') FROM vendor_partners vp WHERE vp.project_id=p.id AND COALESCE(vp.name,'')!=''), ''),
             datetime('now', 'localtime')
           FROM projects p
           WHERE p.id=new.project_id
           ON CONFLICT(source_type,source_id) DO UPDATE SET content=excluded.content,updated_at=excluded.updated_at;
         END;
         -- vendor_partners DELETE → 刷新关联项目的 source_data
         CREATE TRIGGER sd_vendor_partners_delete AFTER DELETE ON vendor_partners
         BEGIN
           INSERT INTO source_data(source_type,source_id,title,content,updated_at)
           SELECT 'PROJECT',p.id,p.name,
             'name: '||p.name||
             CASE WHEN COALESCE(p.status,'')!='' THEN CHAR(10)||'status: '||p.status ELSE '' END||
             CASE WHEN COALESCE(p.owner,'')!='' THEN CHAR(10)||'owner: '||p.owner ELSE '' END||
             CASE WHEN COALESCE(p.customer,'')!='' THEN CHAR(10)||'customer: '||p.customer ELSE '' END||
             CASE WHEN p.budget>0 THEN CHAR(10)||'budget: '||printf('%.2f',p.budget) ELSE '' END||
             CASE WHEN COALESCE(p.description,'')!='' THEN CHAR(10)||'description: '||p.description ELSE '' END||
             CASE WHEN COALESCE(p.bid_time,'')!='' THEN CHAR(10)||'bid_time: '||p.bid_time ELSE '' END||
             CASE WHEN COALESCE(p.customer_contact,'')!='' THEN CHAR(10)||'customer_contact: '||p.customer_contact ELSE '' END||
             CASE WHEN COALESCE(p.customer_phone,'')!='' THEN CHAR(10)||'customer_phone: '||p.customer_phone ELSE '' END||
             CHAR(10)||'solution_products: '||
             COALESCE((SELECT GROUP_CONCAT(DISTINCT si.product_name) FROM solution_items si JOIN solution_versions sv ON sv.id=si.solution_version_id AND sv.status='final' JOIN solutions s ON s.id=sv.solution_id WHERE s.project_id=p.id AND COALESCE(si.product_name,'')!=''), '')||
             CHAR(10)||'dynamics: '||
             COALESCE((SELECT GROUP_CONCAT(pd.type||': '||pd.content||' ('||pd.created_at||')', ' | ') FROM project_dynamics pd WHERE pd.project_id=p.id ORDER BY pd.created_at), '')||
             CHAR(10)||'channel_partners: '||
             COALESCE((SELECT GROUP_CONCAT(COALESCE(cp.company, cp.name)||'（'||cp.name||COALESCE('-'||cp.phone,'')||'）'||COALESCE(' ['||(SELECT GROUP_CONCAT(cc.name||COALESCE('-'||cc.phone,'')||COALESCE('('||cc.remark||')',''), '; ') FROM channel_contacts cc WHERE cc.channel_id=cp.id AND COALESCE(cc.name,'')!='')||']', ''), ', ') FROM channel_partners cp WHERE cp.project_id=p.id AND COALESCE(cp.name,'')!=''), '')||
             CHAR(10)||'vendor_partners: '||
             COALESCE((SELECT GROUP_CONCAT(COALESCE(vp.company, vp.name)||'（'||vp.name||COALESCE('-'||vp.phone,'')||'）'||COALESCE(' ['||(SELECT GROUP_CONCAT(vc.name||COALESCE('-'||vc.phone,'')||COALESCE('('||vc.remark||')',''), '; ') FROM vendor_contacts vc WHERE vc.vendor_id=vp.id AND COALESCE(vc.name,'')!='')||']', ''), ', ') FROM vendor_partners vp WHERE vp.project_id=p.id AND COALESCE(vp.name,'')!=''), ''),
             datetime('now', 'localtime')
           FROM projects p
           WHERE p.id=old.project_id
           ON CONFLICT(source_type,source_id) DO UPDATE SET content=excluded.content,updated_at=excluded.updated_at;
         END;"
    );

    // ── kk_chunks_fts: external content-sync triggers ──
    init_kb_fts_triggers(conn)?;

    // Seed source_data from existing data
    let before: i64 = conn.query_row("SELECT COUNT(*) FROM source_data", [], |r| r.get(0)).unwrap_or(0);
    let _ = conn.execute_batch(
        "INSERT OR IGNORE INTO source_data(source_type,source_id,title,content) SELECT 'NOTE',n.note_id,n.title,
           'title: '||n.title||
           CASE WHEN COALESCE(n.content,'')!='' THEN CHAR(10)||'content: '||n.content ELSE '' END||
           CASE WHEN COALESCE(n.notebook_id,'')!=''
             THEN CHAR(10)||'notebook: '||COALESCE((SELECT name FROM notebooks WHERE id=n.notebook_id), n.notebook_id)
             ELSE '' END||
           CHAR(10)||'created: '||COALESCE(n.created_at, datetime('now','localtime'))||
           CHAR(10)||'updated: '||COALESCE(n.updated_at, datetime('now','localtime'))||
           CASE WHEN (SELECT COUNT(*) FROM note_project_mapping npm JOIN projects p ON p.id=npm.project_id WHERE npm.note_id=n.note_id)>0
             THEN CHAR(10)||'projects: '||(SELECT GROUP_CONCAT(p.name, ', ') FROM note_project_mapping npm JOIN projects p ON p.id=npm.project_id WHERE npm.note_id=n.note_id)
           ELSE '' END
         FROM notes n WHERE n.deleted_at IS NULL;
         INSERT OR IGNORE INTO source_data(source_type,source_id,title,content) SELECT 'PRODUCT',p.id,p.name,
           'name: '||p.name||
           CASE WHEN COALESCE(p.description,'')!='' THEN CHAR(10)||'description: '||p.description ELSE '' END||
           CASE WHEN COALESCE(p.series_name,'')!='' THEN CHAR(10)||'series_name: '||p.series_name ELSE '' END||
           CASE WHEN p.price>0 THEN CHAR(10)||'price: '||printf('%.2f',p.price) ELSE '' END||
           CASE WHEN COALESCE(p.standard_specs,'')!='' THEN CHAR(10)||'standard_specs: '||p.standard_specs ELSE '' END||
           CASE WHEN COALESCE(p.full_specs,'')!='' THEN CHAR(10)||'full_specs: '||p.full_specs ELSE '' END||
           CASE WHEN p.series_id IS NOT NULL AND p.series_id!='' THEN CHAR(10)||'path: '||IFNULL(
             (WITH RECURSIVE sp AS (SELECT name,parent_id,0 AS d FROM product_series WHERE id=p.series_id
               UNION ALL SELECT ps.name,ps.parent_id,sp.d+1 FROM product_series ps JOIN sp ON ps.id=sp.parent_id)
              SELECT GROUP_CONCAT(name,' / ' ORDER BY d DESC) FROM sp),'')
           ELSE '' END
         FROM products p;
         INSERT OR IGNORE INTO source_data(source_type,source_id,title,content) SELECT 'PROJECT',p.id,p.name,
           'name: '||p.name||
           CASE WHEN COALESCE(p.status,'')!='' THEN CHAR(10)||'status: '||p.status ELSE '' END||
           CASE WHEN COALESCE(p.owner,'')!='' THEN CHAR(10)||'owner: '||p.owner ELSE '' END||
           CASE WHEN COALESCE(p.customer,'')!='' THEN CHAR(10)||'customer: '||p.customer ELSE '' END||
           CASE WHEN p.budget>0 THEN CHAR(10)||'budget: '||printf('%.2f',p.budget) ELSE '' END||
           CASE WHEN COALESCE(p.description,'')!='' THEN CHAR(10)||'description: '||p.description ELSE '' END||
           CASE WHEN COALESCE(p.bid_time,'')!='' THEN CHAR(10)||'bid_time: '||p.bid_time ELSE '' END||
           CASE WHEN COALESCE(p.customer_contact,'')!='' THEN CHAR(10)||'customer_contact: '||p.customer_contact ELSE '' END||
           CASE WHEN COALESCE(p.customer_phone,'')!='' THEN CHAR(10)||'customer_phone: '||p.customer_phone ELSE '' END||
           CASE WHEN (SELECT COUNT(*) FROM solution_items si JOIN solution_versions sv ON sv.id=si.solution_version_id AND sv.status='final' JOIN solutions s ON s.id=sv.solution_id WHERE s.project_id=p.id AND COALESCE(si.product_name,'')!='')>0
             THEN CHAR(10)||'solution_products: '||(SELECT GROUP_CONCAT(DISTINCT si.product_name) FROM solution_items si JOIN solution_versions sv ON sv.id=si.solution_version_id AND sv.status='final' JOIN solutions s ON s.id=sv.solution_id WHERE s.project_id=p.id AND COALESCE(si.product_name,'')!='')
           ELSE '' END||
           CASE WHEN (SELECT COUNT(*) FROM project_dynamics pd WHERE pd.project_id=p.id)>0
             THEN CHAR(10)||'dynamics: '||(SELECT GROUP_CONCAT(pd.type||': '||pd.content||' ('||pd.created_at||')', ' | ') FROM project_dynamics pd WHERE pd.project_id=p.id ORDER BY pd.created_at)
           ELSE '' END||
           CASE WHEN (SELECT COUNT(*) FROM channel_partners cp WHERE cp.project_id=p.id AND COALESCE(cp.name,'')!='')>0
             THEN CHAR(10)||'channel_partners: '||(SELECT GROUP_CONCAT(COALESCE(cp.company, cp.name)||'（'||cp.name||COALESCE('-'||cp.phone,'')||'）'||COALESCE(' ['||(SELECT GROUP_CONCAT(cc.name||COALESCE('-'||cc.phone,'')||COALESCE('('||cc.remark||')',''), '; ') FROM channel_contacts cc WHERE cc.channel_id=cp.id AND COALESCE(cc.name,'')!='')||']', ''), ', ') FROM channel_partners cp WHERE cp.project_id=p.id AND COALESCE(cp.name,'')!='')
           ELSE '' END||
           CASE WHEN (SELECT COUNT(*) FROM vendor_partners vp WHERE vp.project_id=p.id AND COALESCE(vp.name,'')!='')>0
             THEN CHAR(10)||'vendor_partners: '||(SELECT GROUP_CONCAT(COALESCE(vp.company, vp.name)||'（'||vp.name||COALESCE('-'||vp.phone,'')||'）'||COALESCE(' ['||(SELECT GROUP_CONCAT(vc.name||COALESCE('-'||vc.phone,'')||COALESCE('('||vc.remark||')',''), '; ') FROM vendor_contacts vc WHERE vc.vendor_id=vp.id AND COALESCE(vc.name,'')!='')||']', ''), ', ') FROM vendor_partners vp WHERE vp.project_id=p.id AND COALESCE(vp.name,'')!='')
           ELSE '' END
         FROM projects p;
         INSERT OR IGNORE INTO source_data(source_type,source_id,title,content) SELECT 'TODO',t.id,t.title,
           'title: '||t.title||
           CASE WHEN t.done=1 THEN CHAR(10)||'done: yes' ELSE '' END||
           CASE WHEN COALESCE(t.priority,'')!='' THEN CHAR(10)||'priority: '||t.priority ELSE '' END||
           CASE WHEN COALESCE(t.due_date,'')!='' THEN CHAR(10)||'due_date: '||t.due_date ELSE '' END||
           CASE WHEN COALESCE(t.repeat,'')!='' THEN CHAR(10)||'repeat: '||t.repeat ELSE '' END||
           CASE WHEN COALESCE(t.content,'')!='' THEN CHAR(10)||'content: '||t.content ELSE '' END
         FROM todos t;"
    );
    let after: i64 = conn.query_row("SELECT COUNT(*) FROM source_data", [], |r| r.get(0)).unwrap_or(0);
    if after > before {
        println!("[数据库] source_data 播种: {} 行 (笔记+产品+项目+待办)", after - before);
    }

    // ★ 刷新已有 PROJECT 记录，补全所有字段（单独执行以便错误报告）
    let refresh_sql = "UPDATE source_data SET content=
      'name: '||COALESCE((SELECT name FROM projects WHERE id=source_data.source_id),'')||
      CASE WHEN COALESCE((SELECT status FROM projects WHERE id=source_data.source_id),'')!='' THEN CHAR(10)||'status: '||(SELECT status FROM projects WHERE id=source_data.source_id) ELSE '' END||
      CASE WHEN COALESCE((SELECT owner FROM projects WHERE id=source_data.source_id),'')!='' THEN CHAR(10)||'owner: '||(SELECT owner FROM projects WHERE id=source_data.source_id) ELSE '' END||
      CASE WHEN COALESCE((SELECT customer FROM projects WHERE id=source_data.source_id),'')!='' THEN CHAR(10)||'customer: '||(SELECT customer FROM projects WHERE id=source_data.source_id) ELSE '' END||
      CASE WHEN (SELECT COALESCE(budget,0) FROM projects WHERE id=source_data.source_id)>0 THEN CHAR(10)||'budget: '||printf('%.2f',(SELECT budget FROM projects WHERE id=source_data.source_id)) ELSE '' END||
      CASE WHEN COALESCE((SELECT description FROM projects WHERE id=source_data.source_id),'')!='' THEN CHAR(10)||'description: '||(SELECT description FROM projects WHERE id=source_data.source_id) ELSE '' END||
      CASE WHEN COALESCE((SELECT bid_time FROM projects WHERE id=source_data.source_id),'')!='' THEN CHAR(10)||'bid_time: '||(SELECT bid_time FROM projects WHERE id=source_data.source_id) ELSE '' END||
      CASE WHEN COALESCE((SELECT customer_contact FROM projects WHERE id=source_data.source_id),'')!='' THEN CHAR(10)||'customer_contact: '||(SELECT customer_contact FROM projects WHERE id=source_data.source_id) ELSE '' END||
      CASE WHEN COALESCE((SELECT customer_phone FROM projects WHERE id=source_data.source_id),'')!='' THEN CHAR(10)||'customer_phone: '||(SELECT customer_phone FROM projects WHERE id=source_data.source_id) ELSE '' END||
      CASE WHEN (SELECT COUNT(*) FROM solution_items si JOIN solution_versions sv ON sv.id=si.solution_version_id AND sv.status='final' JOIN solutions s ON s.id=sv.solution_id WHERE s.project_id=source_data.source_id AND COALESCE(si.product_name,'')!='')>0 THEN CHAR(10)||'solution_products: '||(SELECT GROUP_CONCAT(DISTINCT si.product_name) FROM solution_items si JOIN solution_versions sv ON sv.id=si.solution_version_id AND sv.status='final' JOIN solutions s ON s.id=sv.solution_id WHERE s.project_id=source_data.source_id AND COALESCE(si.product_name,'')!='') ELSE '' END||
      CASE WHEN (SELECT COUNT(*) FROM project_dynamics pd WHERE pd.project_id=source_data.source_id)>0
        THEN CHAR(10)||'dynamics: '||(SELECT GROUP_CONCAT(pd.type||': '||pd.content||' ('||pd.created_at||')', ' | ') FROM project_dynamics pd WHERE pd.project_id=source_data.source_id ORDER BY pd.created_at)
      ELSE '' END||
      CASE WHEN (SELECT COUNT(*) FROM channel_partners cp WHERE cp.project_id=source_data.source_id AND COALESCE(cp.name,'')!='')>0
        THEN CHAR(10)||'channel_partners: '||(SELECT GROUP_CONCAT(COALESCE(cp.company, cp.name)||'（'||cp.name||COALESCE('-'||cp.phone,'')||'）'||COALESCE(' ['||(SELECT GROUP_CONCAT(cc.name||COALESCE('-'||cc.phone,'')||COALESCE('('||cc.remark||')',''), '; ') FROM channel_contacts cc WHERE cc.channel_id=cp.id AND COALESCE(cc.name,'')!='')||']', ''), ', ') FROM channel_partners cp WHERE cp.project_id=source_data.source_id AND COALESCE(cp.name,'')!='')
      ELSE '' END||
      CASE WHEN (SELECT COUNT(*) FROM vendor_partners vp WHERE vp.project_id=source_data.source_id AND COALESCE(vp.name,'')!='')>0
        THEN CHAR(10)||'vendor_partners: '||(SELECT GROUP_CONCAT(COALESCE(vp.company, vp.name)||'（'||vp.name||COALESCE('-'||vp.phone,'')||'）'||COALESCE(' ['||(SELECT GROUP_CONCAT(vc.name||COALESCE('-'||vc.phone,'')||COALESCE('('||vc.remark||')',''), '; ') FROM vendor_contacts vc WHERE vc.vendor_id=vp.id AND COALESCE(vc.name,'')!='')||']', ''), ', ') FROM vendor_partners vp WHERE vp.project_id=source_data.source_id AND COALESCE(vp.name,'')!='')
      ELSE '' END,
      updated_at=datetime('now','localtime')
    WHERE source_type='PROJECT'";
    match conn.execute_batch(refresh_sql) {
        Ok(_) => {
            let updated = conn.changes();
            if updated > 0 {
                println!("[数据库] 刷新 PROJECT source_data: {} 行", updated);
            }
        }
        Err(e) => eprintln!("[数据库] 刷新 PROJECT source_data 失败: {}", e),
    }

    // ★★★ FTS5 自动镜像: source_data → global_search_fts ★★★
    // 延迟到 migrate_source_data_to_text_id 之后创建，避免触发器引用被重命名的表
    // init_source_data_fts_triggers(conn)?;

    // 全量重建 FTS5 索引（幂等操作，确保存量数据完整索引）
    rebuild_global_search_fts(conn);

    seed_kb_fts(conn);
    Ok(())
}

/// ★ 初始化 source_data → global_search_fts 的 3 个自动镜像触发器。
///
/// 触发时机与覆盖范围：
///   AFTER INSERT  — 新建实体 / REPLACE 的插入半程
///   AFTER UPDATE  — 直接用 UPDATE 修改 source_data（罕见，源数据模块偏好 REPLACE）
///   AFTER DELETE  — 删除实体 / REPLACE 的删除半程 / 手动清理
///
/// FTS5 external content 工作原理：
///   查询时 FTS5 通过 content_rowid='rowid' 从 source_data 读取实际文本；
///   但倒排索引（token → rowid）不会自动更新，必须由触发器显式维护。
fn init_source_data_fts_triggers(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    let result = conn.execute_batch(
        "DROP TRIGGER IF EXISTS sd_fts_insert;
         DROP TRIGGER IF EXISTS sd_fts_update;
         DROP TRIGGER IF EXISTS sd_fts_delete;
         DROP TRIGGER IF EXISTS sd_chunks_delete;
         DROP TRIGGER IF EXISTS sd_tag_trigger;

         -- ★ AFTER INSERT: 将新行加入 FTS5 倒排索引。
         -- SQLite 根据 rowid 从 source_data 读取 title/content 并分词。
         CREATE TRIGGER IF NOT EXISTS sd_fts_insert AFTER INSERT ON source_data
         BEGIN
             INSERT INTO global_search_fts(rowid, source_type, source_id, title, content)
             VALUES (new.rowid, new.source_type, new.source_id, new.title, new.content);
         END;

         -- sd_tag_trigger: handled by Rust pipeline

         

         -- ★ AFTER UPDATE: 先删旧、再插新，保证索引与 source_data 一致。
         -- 注意：本次使用 INSERT OR REPLACE 写 source_data 时，SQLite 将其分解为
         -- DELETE + INSERT，分别触发 sd_fts_delete 和 sd_fts_insert，本触发器不参与。
         -- 此触发器仅覆盖对 source_data 执行显式 UPDATE 的边缘场景。
         CREATE TRIGGER IF NOT EXISTS sd_fts_update AFTER UPDATE ON source_data
         BEGIN
             INSERT INTO global_search_fts(global_search_fts, rowid, source_type, source_id, title, content)
             VALUES ('delete', old.rowid, old.source_type, old.source_id, old.title, old.content);
             INSERT INTO global_search_fts(rowid, source_type, source_id, title, content)
             VALUES (new.rowid, new.source_type, new.source_id, new.title, new.content);
         END;"
    );
    match result {
        Ok(()) => println!("[数据库] source_data → global_search_fts 镜像触发器已就绪"),
        Err(ref e) => eprintln!("[数据库] FTS5 镜像触发器初始化失败: {e}"),
    }
    Ok(())
}

/// ★ 全量重建 global_search_fts 索引（幂等）。
///
/// FTS5 'rebuild' 命令会：
///   1. 清空当前倒排索引
///   2. 遍历 source_data 每一行，通过 content_rowid 重新分词建索引
/// 这个过程是原子化的本地操作，不需要网络调用，耗时与 source_data 行数成正比。
fn rebuild_global_search_fts(conn: &rusqlite::Connection) {
    let before: i64 = conn
        .query_row("SELECT COUNT(*) FROM global_search_fts", [], |r| r.get(0))
        .unwrap_or(-1);
    let sd_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM source_data", [], |r| r.get(0))
        .unwrap_or(0);

    // 仅在 FTS 索引落后于源数据时才重建（减少不必要的重建开销）
    if before < sd_count || before < 0 {
        match conn.execute_batch(
            "INSERT INTO global_search_fts(global_search_fts) VALUES('rebuild');"
        ) {
            Ok(()) => {
                let after: i64 = conn
                    .query_row("SELECT COUNT(*) FROM global_search_fts", [], |r| r.get(0))
                    .unwrap_or(0);
                println!(
                    "[数据库] global_search_fts 索引重建: {} → {} 条 (源数据 {})",
                    before, after, sd_count
                );
            }
            Err(e) => eprintln!("[数据库] global_search_fts 重建失败: {e}"),
        }
    } else {
        println!(
            "[数据库] global_search_fts 索引已同步 ({} 条 = 源数据 {})",
            before, sd_count
        );
    }
}

/// Initialize AFTER INSERT/UPDATE/DELETE triggers for kk_chunks_fts (content-sync)
fn init_kb_fts_triggers(conn: &rusqlite::Connection) -> SqliteResult<()> {
    let result = conn.execute_batch(
        "DROP TRIGGER IF EXISTS kk_chunks_fts_insert;
         DROP TRIGGER IF EXISTS kk_chunks_fts_delete;
         DROP TRIGGER IF EXISTS kk_chunks_fts_update;

         -- ★ chunk_type='child' 确保仅子块入 FTS5 索引
         CREATE TRIGGER IF NOT EXISTS kk_chunks_fts_insert AFTER INSERT ON knowledge_chunks
         WHEN new.chunk_type = 'child'
         BEGIN
             INSERT INTO kk_chunks_fts(rowid, doc_title, content) VALUES (new.rowid, new.doc_title, new.content);
         END;



         CREATE TRIGGER IF NOT EXISTS kk_chunks_fts_update AFTER UPDATE ON knowledge_chunks
         WHEN old.chunk_type = 'child' OR new.chunk_type = 'child'
         BEGIN
             DELETE FROM kk_chunks_fts WHERE rowid = old.rowid;
             INSERT INTO kk_chunks_fts(rowid, doc_title, content)
               SELECT new.rowid, new.doc_title, new.content
               WHERE new.chunk_type = 'child';
         END;"
    );
    match result {
        Ok(()) => println!("[数据库] kk_chunks_fts 内容同步触发器已初始化 (仅限子块)"),
        Err(e) => eprintln!("[数据库] 警告: kk_chunks_fts 触发器初始化失败 (可能来自旧版本 schema): {e}"),
    }
    Ok(())
}

/// Seed kk_chunks_fts from existing knowledge_chunks rows (idempotent)
fn seed_kb_fts(conn: &rusqlite::Connection) {
    let before: i64 = conn.query_row("SELECT COUNT(*) FROM kk_chunks_fts", [], |r| r.get(0)).unwrap_or(0);
    if let Err(e) = conn.execute_batch(
        "-- Child-only seed: chunk_type='child'
         INSERT OR IGNORE INTO kk_chunks_fts(rowid, doc_title, content)
         SELECT rowid, doc_title, content FROM knowledge_chunks WHERE chunk_type = 'child';"
    ) {
        eprintln!("[数据库] kk_chunks_fts 播种批量失败: {}", e);
    }
    let after: i64 = conn.query_row("SELECT COUNT(*) FROM kk_chunks_fts", [], |r| r.get(0)).unwrap_or(0);
    if after > before {
        println!("[数据库] kk_chunks_fts 播种 (仅限子块): {} 行", after - before);
    } else {
        println!("[数据库] kk_chunks_fts 索引已是最新");
    }
}

fn init_ai_queue_triggers(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    // ── AI sync queue triggers (AFTER INSERT/UPDATE on business tables) ──
    // ★ 先清除旧的结构化数据 trigger（products/todos），避免已存在的数据库残留
    if let Err(e) = conn.execute_batch(
        "DROP TRIGGER IF EXISTS ai_sync_products_insert;
         DROP TRIGGER IF EXISTS ai_sync_products_update;"
    ) { eprintln!("[数据库] 清除旧 products trigger 失败: {}", e); }
    // Notes: 仅原始 HTML content > 1000 字符的笔记入队（短笔记由摄入中台渲染键值对，不走 chunk）
    for (table, id_col, source_type, length_filter) in &[
        ("file_assets","id","files", ""),
        ("notes","note_id","notes", ""),
        ("products","id","products", ""),
        ("projects","id","projects", ""),
    ] {
        let batch = format!(
            "DROP TRIGGER IF EXISTS ai_sync_{table}_insert;
             DROP TRIGGER IF EXISTS ai_sync_{table}_update;
             CREATE TRIGGER ai_sync_{table}_insert AFTER INSERT ON {table} BEGIN
               INSERT OR IGNORE INTO ai_sync_queue(source_table,source_id)
               SELECT '{source_type}',new.{id_col}{length_filter};
             END;
             CREATE TRIGGER ai_sync_{table}_update AFTER UPDATE ON {table} BEGIN
               INSERT OR IGNORE INTO ai_sync_queue(source_table,source_id)
               SELECT '{source_type}',new.{id_col}{length_filter};
               UPDATE ai_sync_queue
               SET is_embedded=0,retry_count=0,last_error='',updated_at=datetime('now', 'localtime')
               WHERE source_table='{source_type}' AND source_id=new.{id_col}
                 AND (updated_at IS NULL OR datetime(updated_at,'+5 minutes')<datetime('now', 'localtime'));
             END;"
        );
        if let Err(e) = conn.execute_batch(&batch) {
            eprintln!("[数据库] ai_sync 触发器 '{}' 失败: {}", table, e);
        }
    }
    // ★ todos: 仅未完成待办入队（已完成不进知识管线）
    if let Err(e) = conn.execute_batch(
        "DROP TRIGGER IF EXISTS ai_sync_todos_insert;
         DROP TRIGGER IF EXISTS ai_sync_todos_update;
         CREATE TRIGGER ai_sync_todos_insert AFTER INSERT ON todos BEGIN
           INSERT OR IGNORE INTO ai_sync_queue(source_table,source_id)
           VALUES('todos',new.id);
         END;
         CREATE TRIGGER ai_sync_todos_update AFTER UPDATE ON todos BEGIN
           INSERT OR IGNORE INTO ai_sync_queue(source_table,source_id)
           VALUES('todos',new.id);
           UPDATE ai_sync_queue
           SET is_embedded=0,retry_count=0,last_error='',updated_at=datetime('now', 'localtime')
           WHERE source_table='todos' AND source_id=new.id
             AND (updated_at IS NULL OR datetime(updated_at,'+5 minutes')<datetime('now', 'localtime'));
         END;"
    ) { eprintln!("[数据库] ai_sync todos 触发器失败: {}", e); }
    // ── AI tag queue triggers ──
    for (st, table, id_col) in &[
        ("notes","notes","note_id"),
        ("files","file_assets","id"),
        ("products","products","id"),
        ("projects","projects","id"),
        ("todos","todos","id"),
    ] {
        let batch = format!(
            "DROP TRIGGER IF EXISTS ai_tag_{st}_insert;
             DROP TRIGGER IF EXISTS ai_tag_{st}_update;
             CREATE TRIGGER ai_tag_{st}_insert AFTER INSERT ON {table} BEGIN
               INSERT OR IGNORE INTO ai_tag_queue(source_type,source_id)
               SELECT '{st}',new.{id_col}
               WHERE NOT EXISTS (
                 SELECT 1 FROM tags t
                 INNER JOIN tag_mappings tm ON tm.tag_id=t.tag_id
                 WHERE tm.entity_type='{st}' AND tm.entity_id=new.{id_col} AND t.type='ai_shadow'
               );
             END;
             CREATE TRIGGER ai_tag_{st}_update AFTER UPDATE ON {table} BEGIN
               UPDATE ai_tag_queue
               SET status=0,retry_count=0,last_error='',updated_at=datetime('now', 'localtime')
               WHERE source_type='{st}' AND source_id=old.{id_col} AND status!=1
                 AND NOT EXISTS (
                   SELECT 1 FROM tags t
                   INNER JOIN tag_mappings tm ON tm.tag_id=t.tag_id
                   WHERE tm.entity_type='{st}' AND tm.entity_id=new.{id_col} AND t.type='ai_shadow'
                 );
             END;"
        );
        if let Err(e) = conn.execute_batch(&batch) {
            eprintln!("[数据库] ai_tag 触发器 '{}' 失败: {}", st, e);
        }
    }
    // Seed queues (idempotent)
    if let Err(e) = conn.execute_batch(
        "INSERT OR IGNORE INTO ai_sync_queue(source_table,source_id) SELECT 'files',id FROM file_assets;
         INSERT OR IGNORE INTO ai_sync_queue(source_table,source_id) SELECT 'products',id FROM products;
         INSERT OR IGNORE INTO ai_sync_queue(source_table,source_id) SELECT 'projects',id FROM projects;
         INSERT OR IGNORE INTO ai_sync_queue(source_table,source_id) SELECT 'todos',id FROM todos WHERE done=0;"
    ) { eprintln!("[数据库] ai_sync 队列播种失败: {}", e); }
    if let Err(e) = conn.execute_batch(
        "INSERT OR IGNORE INTO ai_tag_queue(source_type,source_id) SELECT 'notes',note_id FROM notes;
         INSERT OR IGNORE INTO ai_tag_queue(source_type,source_id) SELECT 'files',id FROM file_assets;
         INSERT OR IGNORE INTO ai_tag_queue(source_type,source_id) SELECT 'products',id FROM products;
         INSERT OR IGNORE INTO ai_tag_queue(source_type,source_id) SELECT 'projects',id FROM projects;"
    ) { eprintln!("[数据库] ai_tag 队列播种失败: {}", e); }
    Ok(())
}

pub fn check_license_premium() -> bool {
    match with_db(|conn| {
        conn.query_row(
            "SELECT product FROM license WHERE status='active' ORDER BY id DESC LIMIT 1",
            [],
            |r| r.get::<_, String>(0),
        )
        .map(|p| p == "advanced" || p == "enhanced")
    }) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[数据库] license 检查失败: {}", e);
            false
        }
    }
}

// Placeholder repair functions
fn repair_file_product_mapping_fk(_conn: &rusqlite::Connection) {}
fn repair_empty_file_ext(_conn: &rusqlite::Connection) {}
fn migrate_files_to_file_assets(conn: &rusqlite::Connection) {
    let has_old = conn.prepare("SELECT global_name FROM file_assets LIMIT 0").is_ok();
    if has_old {
        println!("[数据库] 检测到旧版 file_assets，正在重建...");
        let _ = conn.execute_batch("
            DELETE FROM file_project_mapping WHERE file_id IN (SELECT id FROM file_assets);
            DELETE FROM file_product_mapping WHERE file_id IN (SELECT id FROM file_assets);
            DROP TABLE IF EXISTS file_assets;
            DROP TABLE IF EXISTS library_folders;
            DROP TABLE IF EXISTS file_library_mapping;
            DROP TABLE IF EXISTS file_product_mapping;
            DROP TABLE IF EXISTS file_project_mapping;
            DROP TABLE IF EXISTS source_data;
            DROP TABLE IF EXISTS global_search_fts;
            CREATE TABLE source_data (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                source_type TEXT NOT NULL, source_id TEXT NOT NULL,
                title TEXT DEFAULT '', content TEXT NOT NULL DEFAULT '',
                created_at TEXT DEFAULT (datetime('now', 'localtime')),
                updated_at TEXT DEFAULT (datetime('now', 'localtime')),
                UNIQUE(source_type, source_id)
            );
            CREATE TABLE file_assets (
                id TEXT PRIMARY KEY, file_ext TEXT DEFAULT '', file_size INTEGER DEFAULT 0,
                content_hash TEXT NOT NULL UNIQUE, parse_status TEXT DEFAULT 'PENDING',
                created_at TEXT DEFAULT (datetime('now', 'localtime')), updated_at TEXT DEFAULT (datetime('now', 'localtime'))
            );
            CREATE TABLE library_folders (
                id TEXT PRIMARY KEY, name TEXT NOT NULL DEFAULT '',
                parent_id TEXT DEFAULT NULL,
                created_at TEXT DEFAULT (datetime('now', 'localtime')), updated_at TEXT DEFAULT (datetime('now', 'localtime')),
                FOREIGN KEY (parent_id) REFERENCES library_folders(id) ON DELETE CASCADE
            );
            CREATE TABLE file_library_mapping (
                file_id TEXT NOT NULL PRIMARY KEY,
                folder_id TEXT DEFAULT NULL,
                global_name TEXT DEFAULT '', custom_name TEXT DEFAULT '',
                creator TEXT DEFAULT '', description TEXT DEFAULT '',
                created_at TEXT DEFAULT (datetime('now', 'localtime')),
                CONSTRAINT fk_lfm_folder FOREIGN KEY (folder_id) REFERENCES library_folders(id) ON DELETE SET NULL
            );
            CREATE TABLE file_product_mapping (
                file_id TEXT NOT NULL, product_id TEXT NOT NULL,
                type TEXT DEFAULT 'OTHER',
                global_name TEXT DEFAULT '', custom_name TEXT DEFAULT '',
                created_at TEXT DEFAULT (datetime('now', 'localtime')),
                PRIMARY KEY (file_id, product_id)
            );
            CREATE TABLE file_project_mapping (
                file_id TEXT NOT NULL, project_id TEXT NOT NULL,
                custom_name TEXT DEFAULT '', global_name TEXT DEFAULT '',
                PRIMARY KEY (file_id, project_id)
            );
            CREATE VIRTUAL TABLE IF NOT EXISTS global_search_fts USING fts5(source_type UNINDEXED, source_id UNINDEXED, title, content, content='source_data', content_rowid='rowid');
        ");
        println!("[数据库] 文件相关表已重建为新 schema");
    }
}

/// ★ 将旧的五张域映射表迁移到 file_relations（新版直接使用 file_id）
fn migrate_to_file_relations(_conn: &rusqlite::Connection) {}

/// ★ 从旧版 file_objects/file_versions 反填 file_relations.file_id，然后清理旧表
fn migrate_file_relations_add_file_id(conn: &rusqlite::Connection) {
    // 先确保 file_relations 有 file_id 列
    let has_file_id = conn.prepare("SELECT file_id FROM file_relations LIMIT 0").is_ok();
    if !has_file_id {
        conn.execute_batch("ALTER TABLE file_relations ADD COLUMN file_id TEXT DEFAULT NULL").ok();
    }
    // 从 file_versions 反填 file_relations.file_id（旧版通过 object_id 关联）
    let need: i64 = conn.query_row("SELECT COUNT(*) FROM file_relations WHERE file_id IS NULL AND object_id IS NOT NULL",[],|r|r.get(0)).unwrap_or(0);
    if need > 0 {
        println!("[迁移] 回填 {} 条 file_relations.file_id", need);
        conn.execute("UPDATE file_relations SET file_id=(SELECT fv.file_id FROM file_versions fv WHERE fv.object_id=file_relations.object_id LIMIT 1) WHERE file_id IS NULL AND object_id IS NOT NULL",[]).ok();
    }
    // 清理旧表
    conn.execute("DROP TABLE IF EXISTS file_versions",[]).ok();
    conn.execute("DROP TABLE IF EXISTS file_objects",[]).ok();
}

/// 将 file_relations 中 library 类型的 folder_id 迁移到 library_file_locations
fn migrate_folder_to_library_locations(conn: &rusqlite::Connection) {
    conn.execute(
        "INSERT OR IGNORE INTO library_file_locations(file_id, folder_id)
         SELECT file_id, folder_id FROM file_relations WHERE entity_type='library' AND file_id IS NOT NULL AND folder_id IS NOT NULL",
        [],
    ).ok();
    conn.execute(
        "INSERT OR IGNORE INTO library_file_locations(file_id, folder_id)
         SELECT file_id, NULL FROM file_relations WHERE entity_type='library' AND file_id IS NOT NULL AND (folder_id IS NULL OR folder_id='')",
        [],
    ).ok();
}

fn dedupe_file_assets_hash(conn: &rusqlite::Connection) {
    let dupes: Vec<String> = {
        let mut s = conn.prepare(
            "SELECT content_hash FROM file_assets WHERE content_hash != '' GROUP BY content_hash HAVING COUNT(*) > 1"
        ).unwrap();
        s.query_map([], |r| r.get::<_, String>(0)).unwrap().filter_map(|r| r.ok()).collect()
    };
    if dupes.is_empty() { return; }
    println!("[数据库] 发现 {} 个重复 content_hash，开始归并...", dupes.len());

    for hash in &dupes {
        let rows: Vec<(String,)> = {
            let mut s = conn.prepare("SELECT id FROM file_assets WHERE content_hash=?1 ORDER BY created_at ASC").unwrap();
            s.query_map(rusqlite::params![hash], |r| Ok((r.get::<_, String>(0)?,))).unwrap().filter_map(|r| r.ok()).collect()
        };
        if rows.len() < 2 { continue; }
        let keeper = &rows[0].0;
        for (dup_id,) in &rows[1..] {
            // file_relations 指向 dup_id 的全部改为指向 keeper
            conn.execute("UPDATE file_relations SET file_id=?1 WHERE file_id=?2", rusqlite::params![keeper, dup_id]).ok();
            // source_data 同理
            conn.execute("UPDATE source_data SET source_id=?1 WHERE source_type='FILE' AND source_id=?2", rusqlite::params![keeper, dup_id]).ok();
            // 删除重复的 file_assets 行
            conn.execute("DELETE FROM file_assets WHERE id=?1", rusqlite::params![dup_id]).ok();
        }
    }
    println!("[数据库] content_hash 去重完成");
}

fn fix_assets_fts_schema(_conn: &rusqlite::Connection) {}



fn run_migrations(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    // Idempotent column migrations (IGNORE errors if column already exists)
    let migrations: &[&str] = &[
        "ALTER TABLE notes ADD COLUMN deleted_at TEXT DEFAULT NULL",
        "ALTER TABLE notes ADD COLUMN hit_count INTEGER DEFAULT 0",
        "ALTER TABLE notes ADD COLUMN last_hit_at TEXT",
        "ALTER TABLE notes ADD COLUMN neural_energy REAL DEFAULT 100.0",
        "ALTER TABLE knowledge_chunks ADD COLUMN file_id TEXT DEFAULT NULL",
        "ALTER TABLE documents_parent ADD COLUMN document_id TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE documents_parent ADD COLUMN doc_title TEXT DEFAULT ''",
        "ALTER TABLE documents_parent ADD COLUMN tags TEXT DEFAULT ''",
        // notebook hierarchy
        "ALTER TABLE notebooks ADD COLUMN parent_id TEXT DEFAULT NULL",
        "ALTER TABLE todos ADD COLUMN priority TEXT DEFAULT NULL",
        "ALTER TABLE todos ADD COLUMN repeat TEXT DEFAULT NULL",
        // file remark
        "ALTER TABLE file_relations ADD COLUMN remark TEXT DEFAULT ''",
        // project-scoped folders
        "ALTER TABLE library_folders ADD COLUMN project_id TEXT DEFAULT NULL",
        "ALTER TABLE file_assets ADD COLUMN parse_status TEXT DEFAULT 'PENDING'",
        "ALTER TABLE library_folders ADD COLUMN sort_order INTEGER DEFAULT 0",
        "ALTER TABLE file_relations ADD COLUMN file_id TEXT DEFAULT NULL",
        // mapping table timestamps
        "ALTER TABLE note_project_mapping ADD COLUMN created_at TEXT DEFAULT (datetime('now', 'localtime'))",
        "ALTER TABLE todo_project_mapping ADD COLUMN created_at TEXT DEFAULT (datetime('now', 'localtime'))",
        // contact remark columns (used by ProjectDetail 渠道/厂商 联系人)
        "ALTER TABLE channel_partners ADD COLUMN remark TEXT DEFAULT ''",
        "ALTER TABLE channel_contacts ADD COLUMN remark TEXT DEFAULT ''",
        "ALTER TABLE vendor_partners ADD COLUMN remark TEXT DEFAULT ''",
        "ALTER TABLE vendor_contacts ADD COLUMN remark TEXT DEFAULT ''",
        // 移除废弃的 products.category 列（存量数据库升级）
        "ALTER TABLE products DROP COLUMN category",
        // cleanup_queue snapshot columns
        "ALTER TABLE cleanup_queue ADD COLUMN source_data_id INTEGER DEFAULT NULL",
        "ALTER TABLE cleanup_queue ADD COLUMN chunk_ids TEXT DEFAULT '[]'",
        "ALTER TABLE ai_sync_queue ADD COLUMN last_content_hash TEXT DEFAULT ''",
        // Object Links
        "CREATE TABLE IF NOT EXISTS object_links(source_id TEXT NOT NULL, target_id TEXT NOT NULL, weight REAL DEFAULT 1.0, last_seen TEXT NOT NULL, PRIMARY KEY(source_id, target_id))",
        // Synapse evolution fields — lazy decay on child chunks
        "ALTER TABLE knowledge_chunks ADD COLUMN base_weight REAL DEFAULT 1.0",
        "ALTER TABLE knowledge_chunks ADD COLUMN last_access_at INTEGER DEFAULT NULL",
        "ALTER TABLE knowledge_chunks ADD COLUMN access_count INTEGER DEFAULT 0",
        "CREATE TABLE IF NOT EXISTS chat_sessions (id TEXT PRIMARY KEY, title TEXT DEFAULT '', created_at TEXT DEFAULT (datetime('now','localtime')), updated_at TEXT DEFAULT (datetime('now','localtime')))",
        "CREATE TABLE IF NOT EXISTS chat_messages (id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL REFERENCES chat_sessions(id) ON DELETE CASCADE, role TEXT NOT NULL, content TEXT DEFAULT '', created_at TEXT DEFAULT (datetime('now','localtime')))",
        "CREATE INDEX IF NOT EXISTS idx_chat_msg_session ON chat_messages(session_id)",
    ];
    for m in migrations {
        let _ = conn.execute(m, []);
    }
    // ★ 迁移 library_file_locations.object_id → file_id
    let has_old_col: bool = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('library_file_locations') WHERE name='object_id'",
        [], |r| r.get::<_,i64>(0),
    ).unwrap_or(0) > 0;
    if has_old_col {
        conn.execute_batch("ALTER TABLE library_file_locations RENAME COLUMN object_id TO file_id").ok();
        println!("[数据库] library_file_locations.object_id → file_id 迁移完成");
    }

    // ★ 修复异常 base_weight：超过上限的 child chunk 重置为 5.0，NULL 或 0 重置为 1.0
    conn.execute("UPDATE knowledge_chunks SET base_weight=5.0 WHERE chunk_type='child' AND base_weight>5.0", []).ok();
    conn.execute("UPDATE knowledge_chunks SET base_weight=1.0 WHERE chunk_type='child' AND (base_weight IS NULL OR base_weight<=0)", []).ok();
    conn.execute("UPDATE knowledge_chunks SET base_weight=1.0 WHERE chunk_type='parent' AND (base_weight IS NULL OR base_weight<=0)", []).ok();
    // ★ 删除所有废弃表
    conn.execute_batch("DROP TABLE IF EXISTS entity_mentions; DROP TABLE IF EXISTS entities; DROP TABLE IF EXISTS documents_parent; DROP TABLE IF EXISTS file_notes_mapping; DROP TABLE IF EXISTS file_library_mapping; DROP TABLE IF EXISTS file_project_mapping; DROP TABLE IF EXISTS file_product_mapping; DROP TABLE IF EXISTS file_todos_mapping; DROP TABLE IF EXISTS file_objects; DROP TABLE IF EXISTS file_versions").ok();
    let _ = conn.execute_batch("CREATE INDEX IF NOT EXISTS idx_notes_deleted ON notes(deleted_at);");

    // ★ knowledge_chunks 重构：改为 chunk_type 显式区分父子，
    //    新增 source_data_id 指向 source_data，删除 source_doc_id / file_id。
    let has_chunk_type: bool = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('knowledge_chunks') WHERE name='chunk_type'",
        [], |r| r.get::<_,i64>(0),
    ).unwrap_or(0) > 0;
    if !has_chunk_type {
        let _ = conn.execute_batch("
            DROP TRIGGER IF EXISTS kk_chunks_fts_insert;
            DROP TRIGGER IF EXISTS kk_chunks_fts_delete;
            DROP TRIGGER IF EXISTS kk_chunks_fts_update;
            DROP TABLE IF EXISTS kk_chunks_fts;
            DROP TABLE IF EXISTS knowledge_chunks;
            CREATE VIRTUAL TABLE IF NOT EXISTS kk_chunks_fts USING fts5(doc_title, content, tokenize='unicode61');
            CREATE TABLE knowledge_chunks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                chunk_id TEXT NOT NULL DEFAULT '',
                chunk_type TEXT NOT NULL DEFAULT 'child',
                parent_id TEXT DEFAULT NULL,
                source_data_id INTEGER DEFAULT NULL,
                content TEXT NOT NULL,
                doc_title TEXT DEFAULT '',
                chunk_index INTEGER DEFAULT 0,
                content_hash TEXT DEFAULT '',
                base_weight REAL DEFAULT 1.0,
                last_access_at INTEGER DEFAULT NULL,
                access_count INTEGER DEFAULT 0,
                created_at TEXT DEFAULT (datetime('now', 'localtime'))
            );
            CREATE INDEX IF NOT EXISTS idx_kc_type ON knowledge_chunks(chunk_type);
            CREATE INDEX IF NOT EXISTS idx_kc_parent ON knowledge_chunks(parent_id);
            CREATE INDEX IF NOT EXISTS idx_kc_source ON knowledge_chunks(source_data_id);
        ").ok();
        println!("[数据库] knowledge_chunks 已重构: chunk_type + source_data_id");
    } else {
        println!("[数据库] knowledge_chunks 结构已正确 (chunk_type 存在)");
        // 确保新索引存在（兼容新库直接走 create_schema 的场景）
        conn.execute_batch("
            CREATE INDEX IF NOT EXISTS idx_kc_type ON knowledge_chunks(chunk_type);
            CREATE INDEX IF NOT EXISTS idx_kc_source ON knowledge_chunks(source_data_id);
        ").ok();
    }

    // ★ 重置失败的同步任务（已完成的不动）
    let _ = conn.execute("UPDATE ai_sync_queue SET is_embedded=0,retry_count=0,last_error='' WHERE is_embedded=-1", []);
    if conn.changes() > 0 {
        println!("[数据库] 已重置 {} 个同步任务", conn.changes());
    }

    // ★ Migrate todos.project_id → todo_project_mapping, then drop redundant columns
    migrate_todo_project_mapping(conn);

    // ★ FTS5: ensure kk_chunks_fts has doc_title column (newly added)
    //     FTS5 doesn't support ALTER TABLE ADD COLUMN, so we drop+recreate if needed.
    let has_fts_doc_title: bool = conn
        .prepare("SELECT doc_title FROM kk_chunks_fts LIMIT 0")
        .is_ok();
    if !has_fts_doc_title {
        println!("[数据库] kk_chunks_fts 缺少 doc_title 列，正在重建...");
        let _ = conn.execute_batch(
            "DROP TRIGGER IF EXISTS kk_chunks_fts_insert;
             DROP TRIGGER IF EXISTS kk_chunks_fts_delete;
             DROP TRIGGER IF EXISTS kk_chunks_fts_update;
             DROP TABLE IF EXISTS kk_chunks_fts;
             CREATE VIRTUAL TABLE IF NOT EXISTS kk_chunks_fts USING fts5(doc_title, content, tokenize='unicode61');
             INSERT OR IGNORE INTO kk_chunks_fts(rowid, doc_title, content)
               SELECT rowid, doc_title, content FROM knowledge_chunks WHERE chunk_type = 'child';"
        );
        // Recreate triggers
        let _ = conn.execute_batch(
            "CREATE TRIGGER IF NOT EXISTS kk_chunks_fts_insert AFTER INSERT ON knowledge_chunks
             WHEN new.chunk_type = 'child'
             BEGIN
                 INSERT INTO kk_chunks_fts(rowid, doc_title, content) VALUES (new.rowid, new.doc_title, new.content);
             END;
             CREATE TRIGGER IF NOT EXISTS kk_chunks_fts_delete AFTER DELETE ON knowledge_chunks
             WHEN old.chunk_type = 'child'
             BEGIN
                 INSERT INTO kk_chunks_fts(kk_chunks_fts, rowid) VALUES ('delete', old.rowid);
             END;
             CREATE TRIGGER IF NOT EXISTS kk_chunks_fts_update AFTER UPDATE ON knowledge_chunks
             WHEN old.chunk_type = 'child' OR new.chunk_type = 'child'
             BEGIN
                 DELETE FROM kk_chunks_fts WHERE rowid = old.rowid;
                 INSERT INTO kk_chunks_fts(rowid, doc_title, content)
                   SELECT new.rowid, new.doc_title, new.content
                   WHERE new.chunk_type = 'child';
             END;"
        );
        println!("[数据库] kk_chunks_fts 重建完成（含 doc_title）");
    }

    // ── Products: deduplicate by name, then add unique index ──
    let has_idx: bool = conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type='index' AND name='idx_products_name'",
        [],
        |_| Ok(()),
    ).is_ok();
    if !has_idx {
        // 删除同名产品中保留 id 最小的那一条
        let deduped = conn.execute(
            "DELETE FROM products WHERE rowid NOT IN (SELECT MIN(rowid) FROM products GROUP BY name)",
            [],
        ).unwrap_or(0);
        if deduped > 0 {
            println!("[数据库] 产品去重: 删除了 {} 条同名记录", deduped);
        }
        conn.execute_batch("CREATE UNIQUE INDEX IF NOT EXISTS idx_products_name ON products(name);")?;
        println!("[数据库] 产品名称唯一索引已创建");
    }

    // ★ 清理存量孤儿 chunk
    conn.execute("DELETE FROM knowledge_chunks WHERE source_data_id NOT IN (SELECT id FROM source_data)", []).ok();

    // ★ 统一文件标签的 entity_id 为 file_assets.id（修复历史数据中混入 file_relations.id 的问题）
    conn.execute(
        "UPDATE tag_mappings SET entity_id = (SELECT fr.file_id FROM file_relations fr WHERE fr.id = tag_mappings.entity_id) \
         WHERE entity_type='files' AND entity_id IN (SELECT id FROM file_relations)",
        [],
    ).ok();

    Ok(())
}

/// Migrate legacy todos.project_id into todo_project_mapping, then drop project_id / project_name columns
fn migrate_todo_project_mapping(conn: &rusqlite::Connection) {
    // Check if the old column still exists
    let has_project_id: bool = conn
        .prepare("SELECT project_id FROM todos LIMIT 0")
        .is_ok();
    if !has_project_id {
        return; // Already migrated
    }

    // Copy existing project associations into mapping table
    let moved: i64 = conn
        .execute(
            "INSERT OR IGNORE INTO todo_project_mapping(todo_id, project_id)
             SELECT id, project_id FROM todos WHERE project_id IS NOT NULL AND project_id != ''",
            [],
        )
        .map(|n| n as i64)
        .unwrap_or(0);
    if moved > 0 {
        println!("[数据库] 待办→项目映射迁移: {} 条记录", moved);
    }

    // Attempt to drop legacy columns (SQLite 3.35+, ignore errors on older versions)
    let _ = conn.execute("ALTER TABLE todos DROP COLUMN project_id", []);
    let _ = conn.execute("ALTER TABLE todos DROP COLUMN project_name", []);

    // Drop the now-stale index
    let _ = conn.execute("DROP INDEX IF EXISTS idx_todos_project", []);
}

pub fn purge_stale_queue_entries() {
    if let Err(e) = with_db(|conn| {
        for sql in &[
            "DELETE FROM ai_sync_queue WHERE source_table='projects' AND source_id NOT IN (SELECT id FROM projects)",
            "DELETE FROM ai_sync_queue WHERE source_table='products' AND source_id NOT IN (SELECT id FROM products)",
            "DELETE FROM ai_sync_queue WHERE source_table='files' AND source_id NOT IN (SELECT id FROM file_assets)",
            "DELETE FROM ai_sync_queue WHERE source_table='notes' AND source_id NOT IN (SELECT note_id FROM notes)",
            "DELETE FROM ai_sync_queue WHERE source_table='todos' AND source_id NOT IN (SELECT id FROM todos)",
        ] {
            if let Err(inner) = conn.execute(sql, []) {
                eprintln!("[数据库] 清理过期队列出错: {}", inner);
            }
        }
        // ★ 清理超过 7 天的已完成待办及其管线数据
        let cutoff = "datetime('now', '-7 days', 'localtime')";
        if let Ok(n) = conn.execute(
            &format!("DELETE FROM ai_sync_queue WHERE source_table='todos' AND source_id IN (SELECT id FROM todos WHERE done=1 AND updated_at < {cutoff})"), []
        ) { if n > 0 { println!("[数据库] 清理已完成待办队列: {} 条", n); } }
        if let Ok(n) = conn.execute(
            &format!("DELETE FROM source_data WHERE source_type='TODO' AND source_id IN (SELECT id FROM todos WHERE done=1 AND updated_at < {cutoff})"), []
        ) { if n > 0 { println!("[数据库] 清理已完成待办 source_data: {} 条", n); } }
        if let Ok(n) = conn.execute(
            &format!("DELETE FROM todos WHERE done=1 AND updated_at < {cutoff}"), []
        ) { if n > 0 { println!("[数据库] 清理超过 7 天的已完成待办: {} 条", n); } }
        Ok(())
    }) {
        eprintln!("[数据库] 清理过期队列失败: {}", e);
    }
}

pub fn seed_tag_queue() {
    if let Err(e) = with_db(|conn| {
        conn.execute_batch(
            "INSERT OR IGNORE INTO ai_tag_queue(source_type,source_id) SELECT 'notes',note_id FROM notes;
             INSERT OR IGNORE INTO ai_tag_queue(source_type,source_id) SELECT 'files',id FROM file_assets;
             INSERT OR IGNORE INTO ai_tag_queue(source_type,source_id) SELECT 'products',id FROM products;
             INSERT OR IGNORE INTO ai_tag_queue(source_type,source_id) SELECT 'projects',id FROM projects;
             INSERT OR IGNORE INTO ai_tag_queue(source_type,source_id) SELECT 'todos',id FROM todos;"
        )
    }) {
        eprintln!("[数据库] 播种标签队列失败: {}", e);
    }
}

// ═══════════════════════════════════════════════════════════════════
// FTS5 Search — Child-only symmetric retrieval, BM25(1.0, 0.3)
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug,Clone,serde::Serialize)]
pub struct Fts5Result { pub rowid: i64, pub entity_type: String, pub entity_id: String, pub title: String, pub snippet: String, pub rank: f64, pub source: serde_json::Value }

/// Lightweight FTS5 hit: (chunk_rowid, bm25_score)
#[derive(Debug,Clone,serde::Serialize)]
pub struct Fts5Hit { pub chunk_id: i64, pub score: f64 }

/// Search kk_chunks_fts
pub fn fts5_search_chunks_slim(query: &str, limit: usize) -> SqliteResult<Vec<Fts5Hit>> {
    with_db(|conn| {
        let sql = "SELECT rowid, bm25(kk_chunks_fts, 5.0, 1.0) as rank FROM kk_chunks_fts WHERE kk_chunks_fts MATCH ? ORDER BY rank LIMIT ?";
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map(rusqlite::params![query, limit as i64], |row| {
            Ok(Fts5Hit { chunk_id: row.get(0)?, score: row.get::<_, f64>(1).unwrap_or(0.0) })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    })
}

/// Search kk_chunks_fts with snippets (for debug)
pub fn fts5_search_chunks(query: &str, limit: i64) -> SqliteResult<Vec<Fts5Result>> {
    with_db(|conn| {
        let sql = "SELECT rowid, snippet(kk_chunks_fts,0,'<mark>','</mark>','…',64) as snippet, bm25(kk_chunks_fts, 5.0, 1.0) as rank FROM kk_chunks_fts WHERE kk_chunks_fts MATCH ? ORDER BY rank LIMIT ?";
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map(rusqlite::params![query, limit], |row| Ok(Fts5Result{rowid:row.get(0)?,entity_type:String::new(),entity_id:String::new(),title:String::new(),snippet:row.get(1)?,rank:row.get(2)?,source:serde_json::Value::Null}))?;
        Ok(rows.filter_map(|r|r.ok()).collect())
    })
}

/// 大一统全局搜索 — global_search_fts
pub fn fts5_search_content(query: &str, limit: i64) -> SqliteResult<Vec<Fts5Result>> {
    with_db(|conn| {
        let sql = "SELECT rowid,source_type,source_id,title,snippet(global_search_fts,3,'<mark>','</mark>','…',64),bm25(global_search_fts,1.0,1.0) as rank FROM global_search_fts WHERE global_search_fts MATCH ? ORDER BY rank LIMIT ?";
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map(rusqlite::params![query, limit], |row| Ok(Fts5Result{rowid:row.get(0)?,entity_type:row.get::<_,String>(1).unwrap_or_default(),entity_id:row.get::<_,String>(2).unwrap_or_default(),title:row.get::<_,String>(3).unwrap_or_default(),snippet:row.get::<_,String>(4).unwrap_or_default(),rank:row.get::<_,f64>(5).unwrap_or(0.0),source:serde_json::Value::Null}))?;
        Ok(rows.filter_map(|r|r.ok()).collect())
    })
}

/// 文件解析完成后写入 source_data（FTS5 external content 自动同步索引）
pub fn file_sync_to_source_data(file_id: &str, parsed_text: &str) {
    if parsed_text.is_empty() { return; }
    // 从 file_assets 获取原始文件名作为 title
    let title = with_db(|c| {
        c.query_row(
            "SELECT original_name FROM file_assets WHERE id=?1",
            rusqlite::params![file_id],
            |row| row.get::<_, String>(0),
        )
    }).unwrap_or_default();
    let result = with_db(|c| {
        c.execute(
            "INSERT INTO source_data(id,source_type,source_id,title,content,updated_at) VALUES(?1,'FILE',?1,?2,?3,datetime('now', 'localtime')) ON CONFLICT(source_type,source_id) DO UPDATE SET content=excluded.content,updated_at=excluded.updated_at",
            rusqlite::params![file_id, title, parsed_text],
        ).map(|_|())
    });
    if let Err(e) = result {
        eprintln!("[数据库] 写入 source_data 失败 (file={}): {}", file_id, e);
    } else {
        // source_data 写入成功后触发同步入队和打标
        crate::services::ai_sync::trigger_sync("files", file_id);
        crate::services::ai_tag::enqueue_tag_task("files", file_id);
    }
}

/// Batch-fetch parent blocks by chunk IDs — returns (parent_id, parent_content) tuples
pub fn batch_get_parents_by_chunks(chunk_ids: &[i64]) -> SqliteResult<Vec<(i64, String)>> {
    if chunk_ids.is_empty() { return Ok(Vec::new()); }
    let placeholders: Vec<String> = chunk_ids.iter().map(|_| "?".to_string()).collect();
    let sql = format!(
        "SELECT dp.id, dp.content FROM knowledge_chunks kc \
         INNER JOIN documents_parent dp ON dp.id = kc.parent_id \
         WHERE kc.id IN ({})",
        placeholders.join(",")
    );
    with_db(|conn| {
        let params: Vec<rusqlite::types::Value> = chunk_ids.iter().map(|&id| rusqlite::types::Value::Integer(id)).collect();
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    })
}

/// Fetch full parent blocks by a list of parent IDs
pub fn batch_get_parent_blocks(parent_ids: &[i64]) -> SqliteResult<Vec<(i64, String)>> {
    if parent_ids.is_empty() { return Ok(Vec::new()); }
    let placeholders: Vec<String> = parent_ids.iter().map(|_| "?".to_string()).collect();
    let sql = format!("SELECT id, content FROM documents_parent WHERE id IN ({})", placeholders.join(","));
    with_db(|conn| {
        let params: Vec<rusqlite::types::Value> = parent_ids.iter().map(|&id| rusqlite::types::Value::Integer(id)).collect();
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    })
}

/// 迁移：将 source_data.id 从 INTEGER 改为 TEXT（使用 source_id）
/// 同时将 knowledge_chunks.source_data_id 改为 TEXT
pub fn migrate_source_data_to_text_id(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    // 检查是否已迁移（source_data.id 是否为 TEXT）
    let id_type: String = conn.query_row(
        "SELECT type FROM pragma_table_info('source_data') WHERE name='id'",
        [], |r| r.get(0),
    ).unwrap_or_default();
    
    eprintln!("[迁移] source_data.id 类型: {}", id_type);
    
    if id_type == "TEXT" {
        eprintln!("[迁移] 已迁移，跳过");
        return Ok(()); // 已迁移
    }
    
    eprintln!("[迁移] 开始迁移 source_data.id 为 TEXT...");
    
    eprintln!("[迁移] 创建 source_data_new 表...");
    // 1. 创建新表
    conn.execute_batch("
        CREATE TABLE IF NOT EXISTS source_data_new (
            id TEXT PRIMARY KEY,
            source_type TEXT NOT NULL,
            source_id TEXT NOT NULL,
            title TEXT DEFAULT '',
            content TEXT NOT NULL DEFAULT '',
            created_at TEXT DEFAULT (datetime('now', 'localtime')),
            updated_at TEXT DEFAULT (datetime('now', 'localtime')),
            UNIQUE(source_type, source_id)
        );
    ")?;
    
    eprintln!("[迁移] 复制数据到 source_data_new...");
    // 2. 迁移数据（id = source_id）
    conn.execute_batch("
        INSERT OR REPLACE INTO source_data_new (id, source_type, source_id, title, content, created_at, updated_at)
        SELECT source_id, source_type, source_id, title, content, created_at, updated_at
        FROM source_data;
    ")?;
    
    eprintln!("[迁移] 重命名表...");
    // 3. 备份旧表，重命名新表
    conn.execute_batch("
        DROP TABLE IF EXISTS source_data_old;
        ALTER TABLE source_data RENAME TO source_data_old;
        ALTER TABLE source_data_new RENAME TO source_data;
    ")?;
    
    eprintln!("[迁移] 重建 FTS5 索引...");
    // 4. 重建 FTS5 索引
    conn.execute_batch("
        DROP TABLE IF EXISTS global_search_fts;
        CREATE VIRTUAL TABLE IF NOT EXISTS global_search_fts USING fts5(
            source_type UNINDEXED, source_id UNINDEXED, title, content, 
            content='source_data', 
            content_rowid='rowid'
        );
        INSERT INTO global_search_fts(global_search_fts) VALUES('rebuild');
    ")?;
    
    eprintln!("[迁移] 检查 knowledge_chunks 是否需要更新...");
    // 5. 迁移 knowledge_chunks.source_data_id 为 TEXT
    let has_sd_id_text: bool = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('knowledge_chunks') WHERE name='source_type'",
        [], |r| r.get::<_, i64>(0),
    ).unwrap_or(0) > 0;
    
    eprintln!("[迁移] knowledge_chunks 已有 source_type 字段: {}", has_sd_id_text);
    
    if !has_sd_id_text {
        eprintln!("[迁移] 添加 source_type 和 source_id 字段...");
        // 添加 source_type 和 source_id 字段
        conn.execute_batch("ALTER TABLE knowledge_chunks ADD COLUMN source_type TEXT DEFAULT '';")?;
        conn.execute_batch("ALTER TABLE knowledge_chunks ADD COLUMN source_id TEXT DEFAULT '';")?;
        
        // 检查 source_data_old 是否存在
        let has_old_table: bool = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='source_data_old'",
            [], |r| r.get::<_, i64>(0),
        ).unwrap_or(0) > 0;
        
        eprintln!("[迁移] source_data_old 存在: {}", has_old_table);
        
        // 从 source_data 填充数据
        let source_table = if has_old_table { "source_data_old" } else { "source_data" };
        eprintln!("[迁移] 从 {} 填充 knowledge_chunks...", source_table);
        conn.execute_batch(&format!(
            "UPDATE knowledge_chunks SET 
                source_type = (SELECT source_type FROM {} WHERE id = knowledge_chunks.source_data_id),
                source_id = (SELECT source_id FROM {} WHERE id = knowledge_chunks.source_data_id)
            WHERE source_data_id IS NOT NULL;",
            source_table, source_table
        ))?;
        eprintln!("[迁移] knowledge_chunks 更新完成");
    }
    
    eprintln!("[迁移] 清理 source_data_old...");
    // 6. 清理旧表
    conn.execute_batch("DROP TABLE IF EXISTS source_data_old;")?;
    
    eprintln!("[迁移] 迁移完成");
    
    // 7. 重建索引
    conn.execute_batch("
        CREATE INDEX IF NOT EXISTS idx_kc_source ON knowledge_chunks(source_type, source_id);
        CREATE INDEX IF NOT EXISTS idx_sd_type_id ON source_data(source_type, source_id);
    ")?;
    
    println!("[数据库] source_data.id 迁移完成");
    Ok(())
}

/// 迁移 user_policies 表数据到 source_data (POLICY 类型)
fn migrate_user_policies_to_source_data(conn: &rusqlite::Connection) {
    // 检查 user_policies 表是否存在
    let table_exists: bool = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='user_policies'")
        .ok()
        .and_then(|mut s| s.query_row([], |_| Ok(true)).ok())
        .unwrap_or(false);
    if !table_exists { return; }

    // 检查是否有活跃数据
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM user_policies WHERE is_active=1", [], |r| r.get(0))
        .unwrap_or(0);
    if count == 0 { return; }

    eprintln!("[迁移] 迁移 {} 条 user_policies → source_data(POLICY)...", count);
    let _ = conn.execute_batch(
        "INSERT OR IGNORE INTO source_data(id,source_type,source_id,title,content,updated_at)
         SELECT 'policy-' || id, 'POLICY', 'policy-' || id, substr(content,1,50), content, created_at
         FROM user_policies WHERE is_active=1"
    );
    println!("[数据库] user_policies 迁移完成");
}

