use rusqlite::{Connection, Result as SqliteResult};

pub(crate) fn create_schema(conn: &Connection) -> SqliteResult<()> {
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
            budget REAL DEFAULT 0,
            bid_time TEXT DEFAULT '',
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
            content_hash TEXT NOT NULL UNIQUE,
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
        CREATE TABLE IF NOT EXISTS customers (id TEXT PRIMARY KEY, name TEXT DEFAULT '', contact_person TEXT DEFAULT '', contact_phone TEXT DEFAULT '', remark TEXT DEFAULT '', created_at TEXT DEFAULT (datetime('now', 'localtime')), updated_at TEXT DEFAULT (datetime('now', 'localtime')));
        CREATE TABLE IF NOT EXISTS customer_contacts (id TEXT PRIMARY KEY, customer_id TEXT, name TEXT, phone TEXT, email TEXT, position TEXT, remark TEXT DEFAULT '', created_at TEXT DEFAULT (datetime('now', 'localtime')));
        -- ★ 统一合作伙伴表（渠道/厂商/客户）
        CREATE TABLE IF NOT EXISTS partners (
            id TEXT PRIMARY KEY,
            type TEXT NOT NULL,  -- 'CHANNEL', 'VENDOR', 'CUSTOMER'
            name TEXT NOT NULL,
            phone TEXT DEFAULT '',
            company TEXT DEFAULT '',
            remark TEXT DEFAULT '',
            contact_person TEXT DEFAULT '',  -- 客户专用
            contact_phone TEXT DEFAULT '',   -- 客户专用
            created_at TEXT DEFAULT (datetime('now', 'localtime')),
            updated_at TEXT DEFAULT (datetime('now', 'localtime'))
        );
        -- ★ 统一联系人表（独立，不依赖 partner_id）
        CREATE TABLE IF NOT EXISTS partner_contacts (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            phone TEXT DEFAULT '',
            email TEXT DEFAULT '',
            position TEXT DEFAULT '',
            remark TEXT DEFAULT '',
            created_at TEXT DEFAULT (datetime('now', 'localtime'))
        );
        -- ★ Partner ↔ Contact 映射（多对多，替代 partner_contacts.partner_id）
        CREATE TABLE IF NOT EXISTS partner_contact_mapping (
            partner_id TEXT NOT NULL,
            contact_id TEXT NOT NULL,
            created_at TEXT DEFAULT (datetime('now', 'localtime')),
            PRIMARY KEY (partner_id, contact_id)
        );
        -- ★ Project ↔ Channel/Vendor mapping (many-to-many) - 旧表保留兼容
        CREATE TABLE IF NOT EXISTS project_channel_mapping (
            project_id TEXT NOT NULL, channel_id TEXT NOT NULL,
            selected_contact_id TEXT DEFAULT NULL,
            created_at TEXT DEFAULT (datetime('now', 'localtime')),
            PRIMARY KEY (project_id, channel_id)
        );
        CREATE TABLE IF NOT EXISTS project_vendor_mapping (
            project_id TEXT NOT NULL, vendor_id TEXT NOT NULL,
            selected_contact_id TEXT DEFAULT NULL,
            created_at TEXT DEFAULT (datetime('now', 'localtime')),
            PRIMARY KEY (project_id, vendor_id)
        );
        CREATE TABLE IF NOT EXISTS project_customer_mapping (
            project_id TEXT NOT NULL, customer_id TEXT NOT NULL,
            created_at TEXT DEFAULT (datetime('now', 'localtime')),
            PRIMARY KEY (project_id, customer_id)
        );
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
        CREATE TABLE IF NOT EXISTS reminder_ack (id INTEGER PRIMARY KEY AUTOINCREMENT, reminder_type TEXT NOT NULL, entity_id TEXT NOT NULL, acked_at TEXT DEFAULT (datetime('now', 'localtime')), UNIQUE(reminder_type, entity_id));

        -- Push pipeline queue: business layer -> knowledge base
        CREATE TABLE IF NOT EXISTS knowledge_push_queue (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            source_type TEXT NOT NULL,
            source_id TEXT NOT NULL,
            operation TEXT NOT NULL DEFAULT 'change',
            status TEXT NOT NULL DEFAULT 'pending',
            retry_count INTEGER DEFAULT 0,
            last_error TEXT DEFAULT '',
            debounce_until TEXT DEFAULT NULL,
            progress_page INTEGER DEFAULT 0,
            progress_total INTEGER DEFAULT 0,
            created_at TEXT DEFAULT (datetime('now', 'localtime')),
            updated_at TEXT DEFAULT (datetime('now', 'localtime'))
        );

        -- 临时解析缓存：存储PDF每页OCR结果，支持断点续传
        CREATE TABLE IF NOT EXISTS file_parse_cache (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            file_id TEXT NOT NULL,
            page_num INTEGER NOT NULL,
            ocr_text TEXT DEFAULT '',
            created_at TEXT DEFAULT (datetime('now', 'localtime')),
            UNIQUE(file_id, page_num)
        );

        -- ★ MEMORY 独立表（原 source_data.source_type='MEMORY'）
        CREATE TABLE IF NOT EXISTS memories (
            id         TEXT PRIMARY KEY,
            content    TEXT NOT NULL DEFAULT '',
            created_at TEXT DEFAULT (datetime('now','localtime')),
            updated_at TEXT DEFAULT (datetime('now','localtime'))
        );

        -- ★ POLICY 独立表（原 source_data.source_type='POLICY'）
        CREATE TABLE IF NOT EXISTS policies (
            id         TEXT PRIMARY KEY,
            content    TEXT NOT NULL DEFAULT '',
            created_at TEXT DEFAULT (datetime('now','localtime')),
            updated_at TEXT DEFAULT (datetime('now','localtime'))
        );


    ")?;
    Ok(())
}
