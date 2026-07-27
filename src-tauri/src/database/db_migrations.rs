use rusqlite::{Connection, Result as SqliteResult};
use crate::database::db_core::with_db;

pub fn check_license_premium() -> bool {
    match with_db(|conn| {
        let row = conn.query_row(
            "SELECT product, expires_at FROM license WHERE status='active' ORDER BY id DESC LIMIT 1",
            [],
            |r| Ok((r.get::<_, String>(0).unwrap_or_default(), r.get::<_, String>(1).unwrap_or_default())),
        );
        match row {
            Ok((product, expires_at)) => {
                if product != "advanced" && product != "enhanced" {
                    return Ok(false);
                }
                if !expires_at.is_empty() && expires_at != "2099-12-31" {
                    if let Ok(expiry) = chrono::NaiveDate::parse_from_str(&expires_at, "%Y-%m-%d") {
                        let today = chrono::Local::now().date_naive();
                        if expiry < today {
                            let _ = conn.execute(
                                "UPDATE license SET status='inactive' WHERE status='active' AND expires_at<?1",
                                rusqlite::params![expires_at],
                            );
                            return Ok(false);
                        }
                    }
                }
                Ok(true)
            }
            Err(_) => Ok(false),
        }
    }) {
        Ok(v) => v,
        Err(e) => {
            log::info!("[数据库] license 检查失败: {}", e);
            false
        }
    }
}

pub(crate) fn migrate_files_to_file_assets(conn: &Connection) {
    let has_old = conn.prepare("SELECT global_name FROM file_assets LIMIT 0").is_ok();
    if has_old {
        log::info!("[数据库] 检测到旧版 file_assets，正在重建...");
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
        log::info!("[数据库] 文件相关表已重建为新 schema");
    }
}

/// ★ 从旧版 file_objects/file_versions 反填 file_relations.file_id，然后清理旧表
pub(crate) fn migrate_file_relations_add_file_id(conn: &Connection) {
    // 先确保 file_relations 有 file_id 列
    let has_file_id = conn.prepare("SELECT file_id FROM file_relations LIMIT 0").is_ok();
    if !has_file_id {
        conn.execute_batch("ALTER TABLE file_relations ADD COLUMN file_id TEXT DEFAULT NULL").ok();
    }
    // 从 file_versions 反填 file_relations.file_id（旧版通过 object_id 关联）
    let need: i64 = conn.query_row("SELECT COUNT(*) FROM file_relations WHERE file_id IS NULL AND object_id IS NOT NULL",[],|r|r.get(0)).unwrap_or(0);
    if need > 0 {
        log::info!("[迁移] 回填 {} 条 file_relations.file_id", need);
        conn.execute("UPDATE file_relations SET file_id=(SELECT fv.file_id FROM file_versions fv WHERE fv.object_id=file_relations.object_id LIMIT 1) WHERE file_id IS NULL AND object_id IS NOT NULL",[]).ok();
    }
    // 清理旧表
    conn.execute("DROP TABLE IF EXISTS file_versions",[]).ok();
    conn.execute("DROP TABLE IF EXISTS file_objects",[]).ok();
}

/// 将 file_relations 中 library 类型的 folder_id 迁移到 library_file_locations
pub(crate) fn migrate_folder_to_library_locations(conn: &Connection) {
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

pub(crate) fn dedupe_file_assets_hash(conn: &Connection) {
    let dupes: Vec<String> = {
        let mut s = conn.prepare(
            "SELECT content_hash FROM file_assets WHERE content_hash != '' GROUP BY content_hash HAVING COUNT(*) > 1"
        ).unwrap();
        s.query_map([], |r| r.get::<_, String>(0)).unwrap().filter_map(|r| r.ok()).collect()
    };
    if dupes.is_empty() { return; }
    log::info!("[数据库] 发现 {} 个重复 content_hash，开始归并...", dupes.len());

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
    log::info!("[数据库] content_hash 去重完成");
}

pub(crate) fn run_migrations(conn: &Connection) -> SqliteResult<()> {
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
        "ALTER TABLE library_folders ADD COLUMN sort_order INTEGER DEFAULT 0",
        "ALTER TABLE file_relations ADD COLUMN file_id TEXT DEFAULT NULL",
        // mapping table timestamps
        "ALTER TABLE note_project_mapping ADD COLUMN created_at TEXT DEFAULT (datetime('now', 'localtime'))",
        "ALTER TABLE todo_project_mapping ADD COLUMN created_at TEXT DEFAULT (datetime('now', 'localtime'))",
        // contact remark columns (used by ProjectDetail 渠道/厂商 联系人)
        "ALTER TABLE channel_partners ADD COLUMN remark TEXT DEFAULT ''",
        "ALTER TABLE vendor_partners ADD COLUMN remark TEXT DEFAULT ''",
        // 移除废弃的 products.category 列（存量数据库升级）
        "ALTER TABLE products DROP COLUMN category",
        "ALTER TABLE ai_sync_queue ADD COLUMN last_content_hash TEXT DEFAULT ''",
        // Object Links (已废弃，见 docs/object_links.md)
        "DROP TABLE IF EXISTS object_links",
        // Synapse evolution fields — lazy decay on child chunks
        "ALTER TABLE knowledge_chunks ADD COLUMN base_weight REAL DEFAULT 1.0",
        "ALTER TABLE knowledge_chunks ADD COLUMN last_access_at INTEGER DEFAULT NULL",
        "ALTER TABLE knowledge_chunks ADD COLUMN access_count INTEGER DEFAULT 0",
        "CREATE TABLE IF NOT EXISTS chat_sessions (id TEXT PRIMARY KEY, title TEXT DEFAULT '', created_at TEXT DEFAULT (datetime('now','localtime')), updated_at TEXT DEFAULT (datetime('now','localtime')))",
        "CREATE TABLE IF NOT EXISTS chat_messages (id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL REFERENCES chat_sessions(id) ON DELETE CASCADE, role TEXT NOT NULL, content TEXT DEFAULT '', created_at TEXT DEFAULT (datetime('now','localtime')))",
        "CREATE INDEX IF NOT EXISTS idx_chat_msg_session ON chat_messages(session_id)",
        "ALTER TABLE chat_messages ADD COLUMN metadata TEXT DEFAULT NULL",
        // push_pipeline debounce
        "ALTER TABLE knowledge_push_queue ADD COLUMN debounce_until TEXT DEFAULT NULL",
        // 预计下单时间
        "ALTER TABLE projects ADD COLUMN expected_order_date TEXT DEFAULT ''",
        // session state: SessionMemory JSON 缓存到 chat_sessions
        "ALTER TABLE chat_sessions ADD COLUMN session_state TEXT DEFAULT NULL",
        // context metadata: 压缩摘要存储在 session 级别
        "ALTER TABLE chat_sessions ADD COLUMN context_metadata TEXT DEFAULT NULL",
        // 清理旧的 session_cache 表（如果存在）
        "DROP TABLE IF EXISTS session_cache",
        // 精灵窗口位置
        "ALTER TABLE system_config ADD COLUMN sprite_position TEXT DEFAULT '{}'",
        // 精灵窗口开关
        "ALTER TABLE system_config ADD COLUMN sprite_enabled INTEGER DEFAULT 1",
    ];
    for m in migrations {
        let _ = conn.execute(m, []);
    }

    // ★ 重命名 partners 表的 company 列为 address
    let has_company_col: bool = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('partners') WHERE name='company'",
        [], |r| r.get::<_,i64>(0),
    ).unwrap_or(0) > 0;
    if has_company_col {
        conn.execute_batch("ALTER TABLE partners RENAME COLUMN company TO address").ok();
        log::info!("[数据库] partners.company → address 迁移完成");
    }

    // ★ 迁移渠道/厂商数据到映射表（从 direct project_id 到 many-to-many mapping）
    let _ = conn.execute_batch("
        INSERT OR IGNORE INTO project_channel_mapping (project_id, channel_id)
        SELECT project_id, id FROM channel_partners WHERE project_id IS NOT NULL AND project_id != '';
        INSERT OR IGNORE INTO project_vendor_mapping (project_id, vendor_id)
        SELECT project_id, id FROM vendor_partners WHERE project_id IS NOT NULL AND project_id != '';
    ");

    // ★ 添加 selected_contact_id 列（如果不存在）
    let _ = conn.execute_batch("
        ALTER TABLE project_channel_mapping ADD COLUMN selected_contact_id TEXT DEFAULT NULL;
        ALTER TABLE project_vendor_mapping ADD COLUMN selected_contact_id TEXT DEFAULT NULL;
    ");

    // ★ 回填 project_customer_mapping → business_relations
    let _ = conn.execute_batch(
        "INSERT OR IGNORE INTO business_relations(id, source_type, source_id, target_type, target_id, relation_type, metadata, created_at, updated_at)
         SELECT hex(randomblob(16)), 'CUSTOMER', pcm.customer_id, 'PROJECT', pcm.project_id, 'belongs_to', '{}', datetime('now','localtime'), datetime('now','localtime')
         FROM project_customer_mapping pcm
         WHERE pcm.customer_id IS NOT NULL AND pcm.customer_id != ''
           AND pcm.project_id IS NOT NULL AND pcm.project_id != ''
           AND NOT EXISTS (
               SELECT 1 FROM business_relations br
               WHERE br.source_type='CUSTOMER' AND br.source_id=pcm.customer_id
               AND br.target_type='PROJECT' AND br.target_id=pcm.project_id
           )"
    );

    // ★ 迁移旧表数据到统一合作伙伴表
    let _ = conn.execute_batch("
        -- 迁移渠道
        INSERT OR IGNORE INTO partners (id, type, name, phone, company, remark, created_at)
        SELECT id, 'CHANNEL', name, phone, company, remark, created_at FROM channel_partners;
        -- 迁移厂商
        INSERT OR IGNORE INTO partners (id, type, name, phone, company, remark, created_at)
        SELECT id, 'VENDOR', name, phone, company, remark, created_at FROM vendor_partners;
        -- 迁移客户
        INSERT OR IGNORE INTO partners (id, type, name, phone, company, remark, contact_person, contact_phone, created_at, updated_at)
        SELECT id, 'CUSTOMER', name, '', '', remark, contact_person, contact_phone, created_at, updated_at FROM customers;
    ");

    // ★ 删除旧表（不再需要）
    let _ = conn.execute_batch("DROP TABLE IF EXISTS channel_contacts;");
    let _ = conn.execute_batch("DROP TABLE IF EXISTS vendor_contacts;");
    let _ = conn.execute_batch("DROP TABLE IF EXISTS customer_contacts;");

    // ★ 迁移项目-合作伙伴映射到 business_relations（metadata 包含 selected_contact_id 和 remark）
    let _ = conn.execute_batch("
        -- 迁移渠道映射
        INSERT OR IGNORE INTO business_relations(id, source_type, source_id, target_type, target_id, relation_type, metadata, created_at, updated_at)
        SELECT hex(randomblob(16)), 'CHANNEL', channel_id, 'PROJECT', project_id, 'belongs_to',
               json_object('selected_contact_id', COALESCE(selected_contact_id, ''), 'remark', ''),
               created_at, datetime('now','localtime')
        FROM project_channel_mapping
        WHERE channel_id IS NOT NULL AND project_id IS NOT NULL;
        -- 迁移厂商映射
        INSERT OR IGNORE INTO business_relations(id, source_type, source_id, target_type, target_id, relation_type, metadata, created_at, updated_at)
        SELECT hex(randomblob(16)), 'VENDOR', vendor_id, 'PROJECT', project_id, 'belongs_to',
               json_object('selected_contact_id', COALESCE(selected_contact_id, ''), 'remark', ''),
               created_at, datetime('now','localtime')
        FROM project_vendor_mapping
        WHERE vendor_id IS NOT NULL AND project_id IS NOT NULL;
        -- 迁移客户映射
        INSERT OR IGNORE INTO business_relations(id, source_type, source_id, target_type, target_id, relation_type, metadata, created_at, updated_at)
        SELECT hex(randomblob(16)), 'CUSTOMER', customer_id, 'PROJECT', project_id, 'belongs_to',
               json_object('remark', ''),
               created_at, datetime('now','localtime')
        FROM project_customer_mapping
        WHERE customer_id IS NOT NULL AND project_id IS NOT NULL;
    ");

    // ★ 处理 partner_contacts 表 schema 变更（移除 partner_id 列）
    let has_partner_id: bool = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('partner_contacts') WHERE name='partner_id'",
        [], |r| r.get::<_,i64>(0),
    ).unwrap_or(0) > 0;
    if has_partner_id {
        // 旧表有 partner_id 列，需要迁移数据并重建表
        let _ = conn.execute_batch("
            CREATE TABLE IF NOT EXISTS partner_contacts_backup AS SELECT id, name, phone, email, position, remark, created_at FROM partner_contacts;
            DROP TABLE IF EXISTS partner_contacts;
            ALTER TABLE partner_contacts_backup RENAME TO partner_contacts;
        ");
        log::info!("[数据库] partner_contacts 表已重建（移除 partner_id 列）");
    }

    // 确保 partner_contact_mapping 表存在
    let _ = conn.execute_batch("
        CREATE TABLE IF NOT EXISTS partner_contact_mapping (
            partner_id TEXT NOT NULL,
            contact_id TEXT NOT NULL,
            created_at TEXT DEFAULT (datetime('now', 'localtime')),
            PRIMARY KEY (partner_id, contact_id)
        );
    ");

    // ★ 迁移 library_file_locations.object_id → file_id
    let has_old_col: bool = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('library_file_locations') WHERE name='object_id'",
        [], |r| r.get::<_,i64>(0),
    ).unwrap_or(0) > 0;
    if has_old_col {
        conn.execute_batch("ALTER TABLE library_file_locations RENAME COLUMN object_id TO file_id").ok();
        log::info!("[数据库] library_file_locations.object_id → file_id 迁移完成");
    }

    // ★ 修复异常 base_weight：超过上限的 child chunk 重置为 5.0，NULL 或 0 重置为 1.0
    conn.execute("UPDATE knowledge_chunks SET base_weight=5.0 WHERE chunk_type='child' AND base_weight>5.0", []).ok();
    conn.execute("UPDATE knowledge_chunks SET base_weight=1.0 WHERE chunk_type='child' AND (base_weight IS NULL OR base_weight<=0)", []).ok();
    conn.execute("UPDATE knowledge_chunks SET base_weight=1.0 WHERE chunk_type='parent' AND (base_weight IS NULL OR base_weight<=0)", []).ok();
    // ★ 删除所有废弃表
    conn.execute_batch("DROP TABLE IF EXISTS entity_mentions; DROP TABLE IF EXISTS entities; DROP TABLE IF EXISTS documents_parent; DROP TABLE IF EXISTS file_notes_mapping; DROP TABLE IF EXISTS file_library_mapping; DROP TABLE IF EXISTS file_project_mapping; DROP TABLE IF EXISTS file_product_mapping; DROP TABLE IF EXISTS file_todos_mapping; DROP TABLE IF EXISTS file_objects; DROP TABLE IF EXISTS file_versions").ok();
    let _ = conn.execute_batch("CREATE INDEX IF NOT EXISTS idx_notes_deleted ON notes(deleted_at);");
    let _ = conn.execute_batch("CREATE INDEX IF NOT EXISTS idx_notes_is_daily ON notes(is_daily);");
    let _ = conn.execute_batch("CREATE INDEX IF NOT EXISTS idx_notes_notebook_id ON notes(notebook_id);");
    let _ = conn.execute_batch("CREATE INDEX IF NOT EXISTS idx_notes_updated_at ON notes(updated_at);");

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
        log::info!("[数据库] knowledge_chunks 已重构: chunk_type + source_data_id");
    } else {
        log::info!("[数据库] knowledge_chunks 结构已正确 (chunk_type 存在)");
        // 确保新索引存在（兼容新库直接走 create_schema 的场景）
        conn.execute_batch("
            CREATE INDEX IF NOT EXISTS idx_kc_type ON knowledge_chunks(chunk_type);
            CREATE INDEX IF NOT EXISTS idx_kc_source ON knowledge_chunks(source_data_id);
        ").ok();
    }

    // ★ 重置失败的同步任务（已完成的不动）
    let _ = conn.execute("UPDATE ai_sync_queue SET is_embedded=0,retry_count=0,last_error='' WHERE is_embedded=-1", []);
    if conn.changes() > 0 {
        log::info!("[数据库] 已重置 {} 个同步任务", conn.changes());
    }

    // ★ Migrate todos.project_id → todo_project_mapping, then drop redundant columns
    migrate_todo_project_mapping(conn);

    // ★ FTS5: ensure kk_chunks_fts has doc_title column (newly added)
    //     FTS5 doesn't support ALTER TABLE ADD COLUMN, so we drop+recreate if needed.
    let has_fts_doc_title: bool = conn
        .prepare("SELECT doc_title FROM kk_chunks_fts LIMIT 0")
        .is_ok();
    if !has_fts_doc_title {
        log::info!("[数据库] kk_chunks_fts 缺少 doc_title 列，正在重建...");
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
                 INSERT INTO kk_chunks_fts(kk_chunks_fts, rowid)
                 SELECT 'delete', old.rowid
                 WHERE EXISTS (SELECT 1 FROM kk_chunks_fts WHERE rowid = old.rowid);
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
        log::info!("[数据库] kk_chunks_fts 重建完成（含 doc_title）");
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
            log::info!("[数据库] 产品去重: 删除了 {} 条同名记录", deduped);
        }
        conn.execute_batch("CREATE UNIQUE INDEX IF NOT EXISTS idx_products_name ON products(name);")?;
        log::info!("[数据库] 产品名称唯一索引已创建");
    }

    // ★ 清理存量孤儿 chunk
    conn.execute("DELETE FROM knowledge_chunks WHERE source_data_id NOT IN (SELECT id FROM source_data)", []).ok();

    // ★ 迁移 projects 客户字段到 partners + project_customer_mapping，然后删除旧列
    let has_customer_col: bool = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('projects') WHERE name='customer'",
        [], |r| r.get::<_,i64>(0),
    ).unwrap_or(0) > 0;
    if has_customer_col {
        // 1. 将 projects.customer 迁移到 partners 表（创建 CUSTOMER 类型的合作伙伴）
        let _ = conn.execute_batch("
            INSERT OR IGNORE INTO partners (id, type, name, phone, company, remark, contact_person, contact_phone, created_at, updated_at)
            SELECT hex(randomblob(16)), 'CUSTOMER', customer, '', '', '', customer_contact, customer_phone, datetime('now','localtime'), datetime('now','localtime')
            FROM projects
            WHERE customer IS NOT NULL AND customer != '';
        ");
        // 2. 创建 project_customer_mapping 条目
        let _ = conn.execute_batch("
            INSERT OR IGNORE INTO project_customer_mapping (project_id, customer_id)
            SELECT p.id, pt.id
            FROM projects p
            JOIN partners pt ON pt.type='CUSTOMER' AND pt.name=p.customer
            WHERE p.customer IS NOT NULL AND p.customer != '';
        ");
        // 3. 删除旧列
        let _ = conn.execute("ALTER TABLE projects DROP COLUMN customer", []);
        let _ = conn.execute("ALTER TABLE projects DROP COLUMN customer_contact", []);
        let _ = conn.execute("ALTER TABLE projects DROP COLUMN customer_phone", []);
        log::info!("[数据库] projects 客户字段已迁移到 partners + project_customer_mapping");
    }

    // ★ 迁移 file_assets 表：移除 parse_status 字段
    let has_parse_status: bool = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('file_assets') WHERE name='parse_status'",
        [], |r| r.get::<_,i64>(0),
    ).unwrap_or(0) > 0;
    if has_parse_status {
        log::info!("[数据库] 检测到 file_assets.parse_status 字段，正在重建表...");
        let _ = conn.execute_batch("
            CREATE TABLE file_assets_new AS SELECT id, file_ext, file_size, content_hash, original_name, created_at, updated_at FROM file_assets;
            DROP TABLE file_assets;
            ALTER TABLE file_assets_new RENAME TO file_assets;
            CREATE UNIQUE INDEX IF NOT EXISTS idx_file_assets_content_hash ON file_assets(content_hash);
        ");
        log::info!("[数据库] file_assets 表已重建（移除 parse_status 字段）");
    }

    // ★ 删除 cleanup_queue 表（功能已合并到 knowledge_push_queue）
    let has_cleanup_queue: bool = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='cleanup_queue'",
        [], |r| r.get::<_,i64>(0),
    ).unwrap_or(0) > 0;
    if has_cleanup_queue {
        let _ = conn.execute_batch("DROP TABLE IF EXISTS cleanup_queue;");
        log::info!("[数据库] cleanup_queue 表已删除");
    }

    // ★ 创建 file_parse_cache 表（PDF断点续传临时缓存）
    let _ = conn.execute_batch("
        CREATE TABLE IF NOT EXISTS file_parse_cache (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            file_id TEXT NOT NULL,
            page_num INTEGER NOT NULL,
            ocr_text TEXT DEFAULT '',
            created_at TEXT DEFAULT (datetime('now', 'localtime')),
            UNIQUE(file_id, page_num)
        );
    ");

    // ★ 为 knowledge_push_queue 添加进度字段
    let has_progress_page: bool = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('knowledge_push_queue') WHERE name='progress_page'",
        [], |r| r.get::<_,i64>(0),
    ).unwrap_or(0) > 0;
    if !has_progress_page {
        let _ = conn.execute_batch("
            ALTER TABLE knowledge_push_queue ADD COLUMN progress_page INTEGER DEFAULT 0;
            ALTER TABLE knowledge_push_queue ADD COLUMN progress_total INTEGER DEFAULT 0;
        ");
        log::info!("[数据库] knowledge_push_queue 已添加进度字段");
    }

    // ★ 回填 partner_project_mapping（从旧 mapping 表 + business_relations）
    let ppm_count: i64 = conn.query_row("SELECT COUNT(*) FROM partner_project_mapping", [], |r| r.get(0)).unwrap_or(0);
    if ppm_count == 0 {
        // 从旧 project_customer_mapping 回填
        conn.execute_batch(
            "INSERT OR IGNORE INTO partner_project_mapping(partner_id, project_id, partner_type, selected_contact_id)
             SELECT customer_id, project_id, 'CUSTOMER', NULL FROM project_customer_mapping
             WHERE customer_id IS NOT NULL AND project_id IS NOT NULL"
        ).ok();
        // 从旧 project_channel_mapping 回填
        conn.execute_batch(
            "INSERT OR IGNORE INTO partner_project_mapping(partner_id, project_id, partner_type, selected_contact_id)
             SELECT channel_id, project_id, 'CHANNEL', selected_contact_id FROM project_channel_mapping
             WHERE channel_id IS NOT NULL AND project_id IS NOT NULL"
        ).ok();
        // 从旧 project_vendor_mapping 回填
        conn.execute_batch(
            "INSERT OR IGNORE INTO partner_project_mapping(partner_id, project_id, partner_type, selected_contact_id)
             SELECT vendor_id, project_id, 'VENDOR', selected_contact_id FROM project_vendor_mapping
             WHERE vendor_id IS NOT NULL AND project_id IS NOT NULL"
        ).ok();
        // 从 business_relations 回填（补充旧表中没有的数据）
        conn.execute_batch(
            "INSERT OR IGNORE INTO partner_project_mapping(partner_id, project_id, partner_type, selected_contact_id, remark)
             SELECT br.source_id, br.target_id, br.source_type,
                    COALESCE(json_extract(br.metadata, '$.selected_contact_id'), ''),
                    COALESCE(json_extract(br.metadata, '$.remark'), '')
             FROM business_relations br
             WHERE br.source_type IN ('CHANNEL','VENDOR','CUSTOMER')
               AND br.target_type = 'PROJECT' AND br.relation_type = 'belongs_to'
               AND br.source_id IS NOT NULL AND br.target_id IS NOT NULL"
        ).ok();
        let filled: i64 = conn.query_row("SELECT COUNT(*) FROM partner_project_mapping", [], |r| r.get(0)).unwrap_or(0);
        if filled > 0 {
            log::info!("[数据库] partner_project_mapping 回填完成: {} 条记录", filled);
        }
    }
    let _ = conn.execute_batch("CREATE INDEX IF NOT EXISTS idx_ppm_project ON partner_project_mapping(project_id);");
    let _ = conn.execute_batch("CREATE INDEX IF NOT EXISTS idx_ppm_partner ON partner_project_mapping(partner_id);");

    // ★ 回填 contact_project_mapping（从 business_relations 中的 CONTACT → PROJECT 关系）
    let cpm_count: i64 = conn.query_row("SELECT COUNT(*) FROM contact_project_mapping", [], |r| r.get(0)).unwrap_or(0);
    if cpm_count == 0 {
        conn.execute_batch(
            "INSERT OR IGNORE INTO contact_project_mapping(contact_id, project_id)
             SELECT br.source_id, br.target_id FROM business_relations br
             WHERE br.source_type = 'CONTACT' AND br.target_type = 'PROJECT'
               AND br.relation_type = 'belongs_to'
               AND br.source_id IS NOT NULL AND br.target_id IS NOT NULL"
        ).ok();
        let filled: i64 = conn.query_row("SELECT COUNT(*) FROM contact_project_mapping", [], |r| r.get(0)).unwrap_or(0);
        if filled > 0 {
            log::info!("[数据库] contact_project_mapping 回填完成: {} 条记录", filled);
        }
    }
    let _ = conn.execute_batch("CREATE INDEX IF NOT EXISTS idx_cpm_project ON contact_project_mapping(project_id);");
    let _ = conn.execute_batch("CREATE INDEX IF NOT EXISTS idx_cpm_contact ON contact_project_mapping(contact_id);");

    // ★ 迁移 MEMORY / POLICY 到独立表
    migrate_memory_policy_to_own_tables(conn);

    // ★ 创建 notifications 和 notification_config 表
    create_notifications_tables(conn);

    Ok(())
}

/// Migrate legacy todos.project_id into todo_project_mapping, then drop project_id / project_name columns
fn migrate_todo_project_mapping(conn: &Connection) {
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
        log::info!("[数据库] 待办→项目映射迁移: {} 条记录", moved);
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
                log::info!("[数据库] 清理过期队列出错: {}", inner);
            }
        }
        // ★ 清理超过 7 天的已完成待办及其管线数据
        let cutoff = "datetime('now', '-7 days', 'localtime')";
        if let Ok(n) = conn.execute(
            &format!("DELETE FROM ai_sync_queue WHERE source_table='todos' AND source_id IN (SELECT id FROM todos WHERE done=1 AND updated_at < {cutoff})"), []
        ) { if n > 0 { log::info!("[数据库] 清理已完成待办队列: {} 条", n); } }
        if let Ok(n) = conn.execute(
            &format!("DELETE FROM source_data WHERE source_type='TODO' AND source_id IN (SELECT id FROM todos WHERE done=1 AND updated_at < {cutoff})"), []
        ) { if n > 0 { log::info!("[数据库] 清理已完成待办 source_data: {} 条", n); } }
        if let Ok(n) = conn.execute(
            &format!("DELETE FROM todos WHERE done=1 AND updated_at < {cutoff}"), []
        ) { if n > 0 { log::info!("[数据库] 清理超过 7 天的已完成待办: {} 条", n); } }

        // ★ 清理已废弃的 ai_tag_queue 及其触发器
        for t in &["ai_tag_notes_insert", "ai_tag_notes_update", "ai_tag_products_insert", "ai_tag_products_update", "ai_tag_projects_insert", "ai_tag_projects_update", "ai_tag_todos_insert", "ai_tag_todos_update"] {
            let _ = conn.execute(&format!("DROP TRIGGER IF EXISTS {}", t), []);
        }
        let _ = conn.execute("DROP TABLE IF EXISTS ai_tag_queue", []);

        // ★ 清理已废弃的 tags 和 tag_mappings 表
        let _ = conn.execute("DROP TABLE IF EXISTS tag_mappings", []);
        let _ = conn.execute("DROP TABLE IF EXISTS tags", []);

        // ★ 清理已废弃的通讯录旧表（已迁移到 partners + partner_project_mapping）
        for t in &["channel_partners", "channel_contacts", "vendor_partners", "vendor_contacts", "customers", "customer_contacts", "project_customer_mapping", "user_policies", "system_notifications"] {
            let _ = conn.execute(&format!("DROP TABLE IF EXISTS {}", t), []);
        }

        Ok(())
    }) {
        log::info!("[数据库] 清理过期队列失败: {}", e);
    }
}

/// 迁移：将 source_data.id 从 INTEGER 改为 TEXT（使用 source_id）
/// 同时将 knowledge_chunks.source_data_id 改为 TEXT
pub fn migrate_source_data_to_text_id(conn: &Connection) -> SqliteResult<()> {
    // 检查是否已迁移（source_data.id 是否为 TEXT）
    let id_type: String = conn.query_row(
        "SELECT type FROM pragma_table_info('source_data') WHERE name='id'",
        [], |r| r.get(0),
    ).unwrap_or_default();
    
    log::info!("[迁移] source_data.id 类型: {}", id_type);
    
    if id_type == "TEXT" {
        log::info!("[迁移] 已迁移，跳过");
        return Ok(()); // 已迁移
    }
    
    log::info!("[迁移] 开始迁移 source_data.id 为 TEXT...");
    
    log::info!("[迁移] 创建 source_data_new 表...");
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
    
    log::info!("[迁移] 复制数据到 source_data_new...");
    // 2. 迁移数据（id = source_id）
    conn.execute_batch("
        INSERT OR REPLACE INTO source_data_new (id, source_type, source_id, title, content, created_at, updated_at)
        SELECT source_id, source_type, source_id, title, content, created_at, updated_at
        FROM source_data;
    ")?;
    
    log::info!("[迁移] 重命名表...");
    // 3. 备份旧表，重命名新表
    conn.execute_batch("
        DROP TABLE IF EXISTS source_data_old;
        ALTER TABLE source_data RENAME TO source_data_old;
        ALTER TABLE source_data_new RENAME TO source_data;
    ")?;
    
    log::info!("[迁移] 重建 FTS5 索引...");
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
    
    log::info!("[迁移] 检查 knowledge_chunks 是否需要更新...");
    // 5. 迁移 knowledge_chunks.source_data_id 为 TEXT
    let has_sd_id_text: bool = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('knowledge_chunks') WHERE name='source_type'",
        [], |r| r.get::<_, i64>(0),
    ).unwrap_or(0) > 0;
    
    log::info!("[迁移] knowledge_chunks 已有 source_type 字段: {}", has_sd_id_text);
    
    if !has_sd_id_text {
        log::info!("[迁移] 添加 source_type 和 source_id 字段...");
        // 添加 source_type 和 source_id 字段
        conn.execute_batch("ALTER TABLE knowledge_chunks ADD COLUMN source_type TEXT DEFAULT '';")?;
        conn.execute_batch("ALTER TABLE knowledge_chunks ADD COLUMN source_id TEXT DEFAULT '';")?;
        
        // 检查 source_data_old 是否存在
        let has_old_table: bool = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='source_data_old'",
            [], |r| r.get::<_, i64>(0),
        ).unwrap_or(0) > 0;
        
        log::info!("[迁移] source_data_old 存在: {}", has_old_table);
        
        // 从 source_data 填充数据
        let source_table = if has_old_table { "source_data_old" } else { "source_data" };
        log::info!("[迁移] 从 {} 填充 knowledge_chunks...", source_table);
        conn.execute_batch(&format!(
            "UPDATE knowledge_chunks SET 
                source_type = (SELECT source_type FROM {} WHERE id = knowledge_chunks.source_data_id),
                source_id = (SELECT source_id FROM {} WHERE id = knowledge_chunks.source_data_id)
            WHERE source_data_id IS NOT NULL;",
            source_table, source_table
        ))?;
        log::info!("[迁移] knowledge_chunks 更新完成");
    }
    
    log::info!("[迁移] 清理 source_data_old...");
    // 6. 清理旧表
    conn.execute_batch("DROP TABLE IF EXISTS source_data_old;")?;
    
    log::info!("[迁移] 迁移完成");
    
    // 7. 重建索引
    conn.execute_batch("
        CREATE INDEX IF NOT EXISTS idx_kc_source ON knowledge_chunks(source_type, source_id);
        CREATE INDEX IF NOT EXISTS idx_sd_type_id ON source_data(source_type, source_id);
    ")?;
    
    log::info!("[数据库] source_data.id 迁移完成");
    Ok(())
}

/// 迁移 user_policies 表数据到 source_data (POLICY 类型)
pub(crate) fn migrate_user_policies_to_source_data(conn: &Connection) {
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

    log::info!("[迁移] 迁移 {} 条 user_policies → source_data(POLICY)...", count);
    let _ = conn.execute_batch(
        "INSERT OR IGNORE INTO source_data(id,source_type,source_id,title,content,updated_at)
         SELECT 'policy-' || id, 'POLICY', 'policy-' || id, substr(content,1,50), content, created_at
         FROM user_policies WHERE is_active=1"
    );
    log::info!("[数据库] user_policies 迁移完成");
}

/// 迁移 source_data 中的 MEMORY / POLICY 数据到独立表
pub(crate) fn migrate_memory_policy_to_own_tables(conn: &Connection) {
    // 检查 source_data 中是否还有 MEMORY / POLICY 数据
    let has_memory: bool = conn
        .prepare("SELECT 1 FROM source_data WHERE source_type='MEMORY' LIMIT 1")
        .ok()
        .and_then(|mut s| s.query_row([], |_| Ok(true)).ok())
        .unwrap_or(false);
    let has_policy: bool = conn
        .prepare("SELECT 1 FROM source_data WHERE source_type='POLICY' LIMIT 1")
        .ok()
        .and_then(|mut s| s.query_row([], |_| Ok(true)).ok())
        .unwrap_or(false);
    if !has_memory && !has_policy { return; }

    log::info!("[迁移] 迁移 MEMORY / POLICY 到独立表...");

    // 1. 迁移 MEMORY
    if has_memory {
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM source_data WHERE source_type='MEMORY'", [], |r| r.get(0))
            .unwrap_or(0);
        log::info!("[迁移] 迁移 {} 条 MEMORY → memories", count);
        let _ = conn.execute_batch(
            "INSERT OR IGNORE INTO memories(id, content, created_at, updated_at)
             SELECT source_id, content, created_at, updated_at
             FROM source_data WHERE source_type='MEMORY'"
        );
        let _ = conn.execute("DELETE FROM source_data WHERE source_type='MEMORY'", []);
    }

    // 2. 迁移 POLICY
    if has_policy {
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM source_data WHERE source_type='POLICY'", [], |r| r.get(0))
            .unwrap_or(0);
        log::info!("[迁移] 迁移 {} 条 POLICY → policies", count);
        let _ = conn.execute_batch(
            "INSERT OR IGNORE INTO policies(id, content, created_at, updated_at)
             SELECT source_id, content, created_at, updated_at
             FROM source_data WHERE source_type='POLICY'"
        );
        let _ = conn.execute("DELETE FROM source_data WHERE source_type='POLICY'", []);
    }

    // 3. 强制重建 memories FTS5 虚拟表和触发器（确保索引与 memories 表同步）
    let _ = conn.execute("DROP TABLE IF EXISTS memories_fts", []);
    let _ = conn.execute_batch(
        "CREATE VIRTUAL TABLE memories_fts USING fts5(
            content,
            content='memories', content_rowid='rowid'
        )"
    );
    let _ = conn.execute_batch(
        "CREATE TRIGGER IF NOT EXISTS memories_ai AFTER INSERT ON memories BEGIN
            INSERT INTO memories_fts(rowid, content) VALUES (new.rowid, new.content);
         END;
         CREATE TRIGGER IF NOT EXISTS memories_ad AFTER DELETE ON memories BEGIN
            INSERT INTO memories_fts(memories_fts, rowid, content) VALUES ('delete', old.rowid, old.content);
         END;
         CREATE TRIGGER IF NOT EXISTS memories_au AFTER UPDATE ON memories BEGIN
            INSERT INTO memories_fts(memories_fts, rowid, content) VALUES ('delete', old.rowid, old.content);
            INSERT INTO memories_fts(rowid, content) VALUES (new.rowid, new.content);
         END;"
    );
    log::info!("[迁移] memories_fts 虚拟表和触发器已重建");

    // 4. 重建 FTS5 索引（外部内容 FTS5 表初始为空，需要 rebuild 才能索引已插入的数据）
    let _ = conn.execute("INSERT INTO memories_fts(memories_fts) VALUES('rebuild')", []);

    // 6. 清理旧 user_policies 表
    let _ = conn.execute("DROP TABLE IF EXISTS user_policies", []);

    log::info!("[迁移] MEMORY / POLICY 独立表迁移完成");
}

/// 创建 notifications 和 notification_config 表
pub(crate) fn create_notifications_tables(conn: &Connection) {
    let has_notifications: bool = conn
        .prepare("SELECT 1 FROM sqlite_master WHERE type='table' AND name='notifications' LIMIT 1")
        .ok()
        .and_then(|mut s| s.query_row([], |_| Ok(true)).ok())
        .unwrap_or(false);
    if has_notifications { return; }

    log::info!("[迁移] 创建 notifications 和 notification_config 表...");
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS notifications (
            id            TEXT PRIMARY KEY,
            type          TEXT NOT NULL,
            title         TEXT NOT NULL,
            content       TEXT DEFAULT '',
            entity_type   TEXT DEFAULT '',
            entity_id     TEXT DEFAULT '',
            priority      TEXT DEFAULT 'medium',
            read          INTEGER DEFAULT 0,
            source        TEXT DEFAULT 'system',
            follow_up_of  TEXT DEFAULT '',
            created_at    TEXT DEFAULT (datetime('now','localtime'))
        );
        CREATE INDEX IF NOT EXISTS idx_notifications_read ON notifications(read);
        CREATE INDEX IF NOT EXISTS idx_notifications_created ON notifications(created_at);

        CREATE TABLE IF NOT EXISTS notification_config (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        INSERT OR IGNORE INTO notification_config(key,value) VALUES('enabled','1');
        INSERT OR IGNORE INTO notification_config(key,value) VALUES('interval_minutes','60');
        INSERT OR IGNORE INTO notification_config(key,value) VALUES('last_check_at','');
        INSERT OR IGNORE INTO notification_config(key,value) VALUES('today_calls','0');
        INSERT OR IGNORE INTO notification_config(key,value) VALUES('today_tokens','0');
        INSERT OR IGNORE INTO notification_config(key,value) VALUES('today_date','');
        INSERT OR IGNORE INTO notification_config(key,value) VALUES('startup_only','0');"
    );
    log::info!("[迁移] notifications 表创建完成");
}
