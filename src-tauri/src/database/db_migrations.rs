use rusqlite::{Connection, Result as SqliteResult};
use crate::database::db_core::with_db;

/// 检查表是否存在（遗留表迁移前的保护：全新安装的库没有老版本遗留表）
fn table_exists(conn: &Connection, name: &str) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
        [name], |r| r.get::<_, i64>(0),
    ).unwrap_or(0) > 0
}

pub fn check_license_premium() -> bool {
    // ★ 试用期内开放全部高级功能
    if crate::license::trial_is_active() {
        return true;
    }
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
        // 破坏性重建前强制快照：备份失败则放弃本次迁移（下次启动重试），绝不裸删
        if let Err(e) = backup_before_destructive(conn, "pre_files_rebuild") {
            log::error!("[数据库] 迁移前备份失败，跳过 file_assets 破坏性重建: {}", e);
            return;
        }
        // 不再 DROP source_data/global_search_fts：清洗文本语料（NOTE/EMAIL/FILE）保留，
        // source_data 的 id 类型转换由其后的 migrate_source_data_to_text_id 处理；
        // 指向已清空 file_assets 的孤儿行由启动孤儿清理收敛
        let _ = conn.execute_batch("
            DELETE FROM file_project_mapping WHERE file_id IN (SELECT id FROM file_assets);
            DELETE FROM file_product_mapping WHERE file_id IN (SELECT id FROM file_assets);
            DROP TABLE IF EXISTS file_assets;
            DROP TABLE IF EXISTS library_folders;
            DROP TABLE IF EXISTS file_library_mapping;
            DROP TABLE IF EXISTS file_product_mapping;
            DROP TABLE IF EXISTS file_project_mapping;
            CREATE TABLE IF NOT EXISTS source_data (
                id TEXT PRIMARY KEY,
                source_type TEXT NOT NULL, source_id TEXT NOT NULL,
                title TEXT DEFAULT '', content TEXT NOT NULL DEFAULT '',
                created_at TEXT DEFAULT (datetime('now', 'localtime')),
                updated_at TEXT DEFAULT (datetime('now', 'localtime')),
                UNIQUE(source_type, source_id)
            );
            CREATE TABLE file_assets (
                id TEXT PRIMARY KEY, file_ext TEXT DEFAULT '', file_size INTEGER DEFAULT 0,
                content_hash TEXT NOT NULL UNIQUE, parse_status TEXT DEFAULT 'PENDING',
                original_name TEXT DEFAULT '',
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
        ");
        log::info!("[数据库] 文件相关表已重建为新 schema（source_data 语料已保留）");
    }
}

/// 破坏性迁移前的库内快照：VACUUM INTO 到数据目录 backups/，失败上抛由调用方中止迁移
fn backup_before_destructive(conn: &Connection, prefix: &str) -> Result<(), String> {
    let db_file: String = conn.query_row(
        "SELECT file FROM pragma_database_list WHERE name='main'", [], |r| r.get(0),
    ).unwrap_or_default();
    let dir = std::path::Path::new(&db_file).parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| "无法确定数据目录".to_string())?;
    let backups = dir.join("backups");
    std::fs::create_dir_all(&backups).map_err(|e| format!("创建备份目录失败: {}", e))?;
    let ts = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let dst = backups.join(format!("{}_{}.db", prefix, ts));
    conn.execute("VACUUM INTO ?1", rusqlite::params![dst.to_string_lossy().as_ref()])
        .map_err(|e| format!("VACUUM INTO 失败: {}", e))?;
    log::warn!("[数据库] 破坏性迁移前快照已生成: {:?}", dst);
    Ok(())
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
        // message status: active（正常，进上下文）/ compressed（已压缩，不进上下文，前端可展示）
        "ALTER TABLE chat_messages ADD COLUMN status TEXT DEFAULT 'active'",
        // push_pipeline debounce
        "ALTER TABLE knowledge_push_queue ADD COLUMN debounce_until TEXT DEFAULT NULL",
        // 预计下单时间
        "ALTER TABLE projects ADD COLUMN expected_order_date TEXT DEFAULT ''",
        // 中标时间（updated_at 改为"最后活跃时间"语义后，中标时间需要专用列承载）
        "ALTER TABLE projects ADD COLUMN won_at TEXT DEFAULT ''",
        // session state: SessionMemory JSON 缓存到 chat_sessions
        "ALTER TABLE chat_sessions ADD COLUMN session_state TEXT DEFAULT NULL",
        // context metadata: 压缩摘要存储在 session 级别
        "ALTER TABLE chat_sessions ADD COLUMN context_metadata TEXT DEFAULT NULL",
        // tool metadata: 最新轮次的 tool_calls/tool_results 存储在 session 级别（替代 chat_messages.metadata）
        "ALTER TABLE chat_sessions ADD COLUMN tool_metadata TEXT DEFAULT NULL",
        // 清理旧的 session_cache 表（如果存在）
        "DROP TABLE IF EXISTS session_cache",
        // 精灵窗口位置
        "ALTER TABLE system_config ADD COLUMN sprite_position TEXT DEFAULT '{}'",
        // 精灵窗口开关
        "ALTER TABLE system_config ADD COLUMN sprite_enabled INTEGER DEFAULT 0",
        // 邮件附件关联文件资产
        "ALTER TABLE email_attachments ADD COLUMN file_asset_id TEXT DEFAULT NULL",
        // 邮件附件 Content-ID 和内嵌标识
        "ALTER TABLE email_attachments ADD COLUMN content_id TEXT DEFAULT NULL",
        "ALTER TABLE email_attachments ADD COLUMN is_inline INTEGER DEFAULT 0",
        "CREATE INDEX IF NOT EXISTS idx_email_attach_content_id ON email_attachments(content_id)",
        // 邮件抄送地址
        "ALTER TABLE emails ADD COLUMN cc_addr TEXT DEFAULT ''",
        // Agent 预设工具字段
        "ALTER TABLE agent_profiles ADD COLUMN preset_tools TEXT DEFAULT '[]'",
    ];
    for m in migrations {
        let _ = conn.execute(m, []);
    }

    // ★ 修复：将"未分类"目录下的产品 series_id 改为 NULL（使其正确显示在未分类中）
    if let Ok(uncat_id) = conn.query_row(
        "SELECT id FROM product_series WHERE name='未分类' AND parent_id IS NULL LIMIT 1",
        [], |r| r.get::<_, String>(0),
    ) {
        let fixed = conn.execute(
            "UPDATE products SET series_id=NULL WHERE series_id=?1",
            rusqlite::params![uncat_id],
        ).unwrap_or(0);
        if fixed > 0 {
            log::info!("[迁移] 修复 {} 个产品的 series_id（未分类目录 → NULL）", fixed);
        }
    }

    // 修复 knowledge_chunks 结构：source_data_id INTEGER → TEXT，并补齐 source_type/source_id 列。
    // SQLite 不支持 ALTER COLUMN → 重建表；对缺列的远古表做防御式拷贝（缺什么补默认值）。
    let kc_cols: Vec<String> = conn
        .prepare("SELECT name FROM pragma_table_info('knowledge_chunks')")
        .and_then(|mut s| Ok(s.query_map([], |r| r.get::<_, String>(0))?.filter_map(|x| x.ok()).collect()))
        .unwrap_or_default();
    let sd_id_type: String = conn.query_row(
        "SELECT type FROM pragma_table_info('knowledge_chunks') WHERE name='source_data_id'",
        [], |r| r.get(0),
    ).unwrap_or_default();
    let need_rebuild = !kc_cols.is_empty()
        && (sd_id_type == "INTEGER"
            || !kc_cols.iter().any(|c| c == "source_type")
            || !kc_cols.iter().any(|c| c == "source_id"));

    if need_rebuild {
        log::info!("[迁移] 重建 knowledge_chunks：source_data_id→TEXT + 补齐 source_type/source_id");

        // 1. 创建新表
        conn.execute_batch("
            CREATE TABLE IF NOT EXISTS knowledge_chunks_new (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                chunk_id TEXT NOT NULL DEFAULT '',
                chunk_type TEXT NOT NULL DEFAULT 'child',
                parent_id TEXT DEFAULT NULL,
                source_data_id TEXT DEFAULT NULL,
                source_type TEXT DEFAULT '',
                source_id TEXT DEFAULT '',
                content TEXT NOT NULL,
                doc_title TEXT DEFAULT '',
                chunk_index INTEGER DEFAULT 0,
                content_hash TEXT DEFAULT '',
                base_weight REAL DEFAULT 1.0,
                last_access_at INTEGER DEFAULT NULL,
                access_count INTEGER DEFAULT 0,
                created_at TEXT DEFAULT (datetime('now', 'localtime'))
            );
        ")?;

        // 2. 防御式复制：仅拷贝新旧表都存在的普通列；特殊列用表达式兜底
        let plain_cols = ["chunk_id", "chunk_type", "parent_id", "content", "doc_title",
            "chunk_index", "content_hash", "base_weight", "last_access_at", "access_count", "created_at"];
        let mut insert_cols: Vec<String> = vec!["id".into()];
        let mut select_cols: Vec<String> = vec!["id".into()];
        for c in plain_cols {
            if kc_cols.iter().any(|x| x == c) {
                insert_cols.push(c.into());
                select_cols.push(c.into());
            }
        }
        insert_cols.push("source_data_id".into());
        select_cols.push(if kc_cols.iter().any(|x| x == "source_data_id") {
            "CAST(source_data_id AS TEXT)".into()
        } else { "NULL".into() });
        for c in ["source_type", "source_id"] {
            insert_cols.push(c.into());
            select_cols.push(if kc_cols.iter().any(|x| x == c) { c.into() } else { "''".into() });
        }
        conn.execute_batch(&format!(
            "INSERT INTO knowledge_chunks_new ({}) SELECT {} FROM knowledge_chunks;",
            insert_cols.join(", "), select_cols.join(", ")
        ))?;

        // 3. 替换旧表
        conn.execute_batch("
            DROP TABLE IF EXISTS knowledge_chunks_old;
            ALTER TABLE knowledge_chunks RENAME TO knowledge_chunks_old;
            ALTER TABLE knowledge_chunks_new RENAME TO knowledge_chunks;
        ")?;

        // 4. 重建索引
        conn.execute_batch("
            CREATE INDEX IF NOT EXISTS idx_kc_parent ON knowledge_chunks(parent_id);
            CREATE INDEX IF NOT EXISTS idx_kc_source ON knowledge_chunks(source_type, source_id);
            CREATE INDEX IF NOT EXISTS idx_kc_source_data ON knowledge_chunks(source_data_id);
        ")?;

        // 5. 清理旧表
        conn.execute_batch("DROP TABLE IF EXISTS knowledge_chunks_old;")?;

        log::info!("[迁移] knowledge_chunks 结构修复完成");
    }

    // ★ 回填存量 Won 项目的 won_at：优先取状态变更日志的最早时间，找不到回退 updated_at
    if let Err(e) = conn.execute_batch("
        UPDATE projects SET won_at = (
            SELECT MIN(created_at) FROM project_dynamics
            WHERE project_id = projects.id AND type='system' AND content='项目状态更新为：Won'
        ) WHERE status='Won' AND (won_at IS NULL OR won_at='');
        UPDATE projects SET won_at = updated_at
        WHERE status='Won' AND (won_at IS NULL OR won_at='');
    ") {
        log::error!("[数据库] won_at 存量回填失败: {}", e);
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

    // ★ 更新 source_data 中产品的"完整规格"为直接内容（移除前缀）
    let updated = conn.execute(
        "UPDATE source_data SET content = REPLACE(content, '完整规格：', '') WHERE source_type = 'PRODUCT' AND content LIKE '%完整规格：%'", []
    ).unwrap_or(0);
    if updated > 0 {
        log::info!("[数据库] 更新了 {} 条产品的 source_data（移除完整规格前缀）", updated);
    }
    // ★ 清理之前误加的 "Params: " 前缀
    let updated2 = conn.execute(
        "UPDATE source_data SET content = REPLACE(content, 'Params: ', '') WHERE source_type = 'PRODUCT' AND content LIKE '%Params: %'", []
    ).unwrap_or(0);
    if updated2 > 0 {
        log::info!("[数据库] 清理了 {} 条产品的 source_data（移除 Params: 前缀）", updated2);
    }

    // ★ 输出规范已固化到系统提示词第9节，删除存量 default-output-rules 策略行（不再动态注入，避免重复）
    let _ = conn.execute("DELETE FROM policies WHERE id='default-output-rules'", []);

    // ★ 清理 policies 表遗留列（旧schema的 agent_types/scope，代码已无任何读写；新库无此列则跳过）
    for legacy_col in ["agent_types", "scope"] {
        let has_col: bool = conn
            .prepare("PRAGMA table_info(policies)")
            .and_then(|mut s| {
                let cols = s.query_map([], |r| r.get::<_, String>(1))?;
                Ok(cols.filter_map(|x| x.ok()).any(|c| c == legacy_col))
            })
            .unwrap_or(false);
        if has_col {
            let _ = conn.execute(&format!("ALTER TABLE policies DROP COLUMN {}", legacy_col), []);
            log::info!("[数据库] 清理 policies 表遗留列 {}", legacy_col);
        }
    }

    // ★ 预置业务助手 Agent
    let has_business_agent: bool = conn.query_row(
        "SELECT 1 FROM agent_profiles WHERE id='business'",
        [], |_| Ok(true),
    ).unwrap_or(false);
    if !has_business_agent {
        let business_prompt = "你是 ArkMind 的业务管理助手。你的职责是执行业务对象的增删改查操作。\n\n## 业务对象模型\nPROJECT → SOLUTION → SOLUTION_VERSION → SOLUTION_ITEM → PRODUCT\nPARTNER（CHANNEL/VENDOR/CUSTOMER）→ CONTACT\nTODO\n\n## 原则\n- 通过工具操作数据，不直接猜测\n- 重要修改前确认用户意图\n- 联系人必须归属于合作伙伴；创建联系人时先确认 partner_id\n- 联系人关联项目时，所属合作伙伴必须同时关联到同一项目\n- 设备完整参数快照：ext_tool_object(type=\"SOLUTION_ITEM\") 返回的 params 字段（该版本保存时的快照，与产品中心参数无关）";
        let preset_tools = "[\"ext_tool_object\",\"ext_tool_relations\",\"ext_tool_manage_project\",\"ext_tool_manage_todo\",\"ext_tool_manage_partner\",\"ext_tool_manage_contact\",\"ext_tool_manage_product\"]";
        conn.execute(
            "INSERT INTO agent_profiles(id, name, description, system_prompt, max_iterations, can_delegate, output_format, status, is_builtin, preset_tools, created_at, updated_at) VALUES('business', '业务助手', '项目、待办、合作伙伴、联系人、产品的增删改查操作', ?1, 20, 0, 'natural', 'active', 1, ?2, datetime('now','localtime'), datetime('now','localtime'))",
            rusqlite::params![business_prompt, preset_tools],
        ).ok();
        // 分配工具
        for tool_name in &["ext_tool_object", "ext_tool_relations", "ext_tool_manage_project", "ext_tool_manage_todo", "ext_tool_manage_partner", "ext_tool_manage_contact", "ext_tool_manage_product"] {
            conn.execute(
                "INSERT OR IGNORE INTO agent_tool_assignments(agent_id, tool_name, created_at) VALUES('business', ?1, datetime('now','localtime'))",
                rusqlite::params![tool_name],
            ).ok();
        }
        log::info!("[数据库] 预置业务助手 Agent");
    } else {
        // 更新现有业务助手的 preset_tools 和 system_prompt（移除工具使用部分）
        let business_prompt = "你是 ArkMind 的业务管理助手。你的职责是执行业务对象的增删改查操作。\n\n## 业务对象模型\nPROJECT → SOLUTION → SOLUTION_VERSION → SOLUTION_ITEM → PRODUCT\nPARTNER（CHANNEL/VENDOR/CUSTOMER）→ CONTACT\nTODO\n\n## 原则\n- 通过工具操作数据，不直接猜测\n- 重要修改前确认用户意图\n- 联系人必须归属于合作伙伴；创建联系人时先确认 partner_id\n- 联系人关联项目时，所属合作伙伴必须同时关联到同一项目\n- 设备完整参数快照：ext_tool_object(type=\"SOLUTION_ITEM\") 返回的 params 字段（该版本保存时的快照，与产品中心参数无关）";
        let preset_tools = "[\"ext_tool_object\",\"ext_tool_relations\",\"ext_tool_manage_project\",\"ext_tool_manage_todo\",\"ext_tool_manage_partner\",\"ext_tool_manage_contact\",\"ext_tool_manage_product\"]";
        conn.execute(
            "UPDATE agent_profiles SET system_prompt=?1, preset_tools=?2 WHERE id='business'",
            rusqlite::params![business_prompt, preset_tools],
        ).ok();
        // ★ 确保已有业务助手也有工具分配（补插）
        for tool_name in &["ext_tool_object", "ext_tool_relations", "ext_tool_manage_project", "ext_tool_manage_todo", "ext_tool_manage_partner", "ext_tool_manage_contact", "ext_tool_manage_product"] {
            conn.execute(
                "INSERT OR IGNORE INTO agent_tool_assignments(agent_id, tool_name, created_at) VALUES('business', ?1, datetime('now','localtime'))",
                rusqlite::params![tool_name],
            ).ok();
        }
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

    // ★ 回填 project_customer_mapping → business_relations（遗留表，全新安装不存在则跳过）
    if table_exists(conn, "project_customer_mapping") {
        if let Err(e) = conn.execute_batch(
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
        ) {
            log::error!("[迁移] project_customer_mapping → business_relations 回填失败: {}", e);
        }
    }

    // ★ 迁移旧表数据到统一合作伙伴表（遗留表，全新安装不存在则跳过）
    if table_exists(conn, "channel_partners") {
        let _ = conn.execute_batch(
            "INSERT OR IGNORE INTO partners (id, type, name, phone, company, remark, created_at)
             SELECT id, 'CHANNEL', name, phone, company, remark, created_at FROM channel_partners;"
        );
    }
    if table_exists(conn, "vendor_partners") {
        let _ = conn.execute_batch(
            "INSERT OR IGNORE INTO partners (id, type, name, phone, company, remark, created_at)
             SELECT id, 'VENDOR', name, phone, company, remark, created_at FROM vendor_partners;"
        );
    }
    if table_exists(conn, "customers") {
        let _ = conn.execute_batch(
            "INSERT OR IGNORE INTO partners (id, type, name, phone, company, remark, contact_person, contact_phone, created_at, updated_at)
             SELECT id, 'CUSTOMER', name, '', '', remark, contact_person, contact_phone, created_at, updated_at FROM customers;"
        );
    }

    // ★ 删除旧表（不再需要）
    let _ = conn.execute_batch("DROP TABLE IF EXISTS channel_contacts;");
    let _ = conn.execute_batch("DROP TABLE IF EXISTS vendor_contacts;");
    let _ = conn.execute_batch("DROP TABLE IF EXISTS customer_contacts;");

    // ★ 迁移项目-合作伙伴映射到 business_relations（仅首次执行，后续跳过）
    // 注意：system_config 没有 key 列（PK 是 tenant_id），旧代码查错表导致标记恒为 false、每次启动全量重跑
    let migration_done: bool = conn.query_row(
        "SELECT COUNT(*) FROM system_settings WHERE key='migration_br_mapping_done'",
        [], |r| r.get::<_,i64>(0),
    ).unwrap_or(0) > 0;

    if !migration_done {
        let mut all_ok = true;
        // 迁移渠道映射（表在当前 schema 中，但做存在性检查以防万一）
        if table_exists(conn, "project_channel_mapping") {
            if let Err(e) = conn.execute_batch("
                INSERT OR IGNORE INTO business_relations(id, source_type, source_id, target_type, target_id, relation_type, metadata, created_at, updated_at)
                SELECT hex(randomblob(16)), 'CHANNEL', channel_id, 'PROJECT', project_id, 'belongs_to',
                       json_object('selected_contact_id', COALESCE(selected_contact_id, ''), 'remark', ''),
                       created_at, datetime('now','localtime')
                FROM project_channel_mapping
                WHERE channel_id IS NOT NULL AND project_id IS NOT NULL;
            ") {
                log::error!("[迁移] 渠道映射 → business_relations 失败: {}", e);
                all_ok = false;
            }
        }
        // 迁移厂商映射
        if table_exists(conn, "project_vendor_mapping") {
            if let Err(e) = conn.execute_batch("
                INSERT OR IGNORE INTO business_relations(id, source_type, source_id, target_type, target_id, relation_type, metadata, created_at, updated_at)
                SELECT hex(randomblob(16)), 'VENDOR', vendor_id, 'PROJECT', project_id, 'belongs_to',
                       json_object('selected_contact_id', COALESCE(selected_contact_id, ''), 'remark', ''),
                       created_at, datetime('now','localtime')
                FROM project_vendor_mapping
                WHERE vendor_id IS NOT NULL AND project_id IS NOT NULL;
            ") {
                log::error!("[迁移] 厂商映射 → business_relations 失败: {}", e);
                all_ok = false;
            }
        }
        // 迁移客户映射（遗留表，全新安装不存在则跳过）
        if table_exists(conn, "project_customer_mapping") {
            if let Err(e) = conn.execute_batch("
                INSERT OR IGNORE INTO business_relations(id, source_type, source_id, target_type, target_id, relation_type, metadata, created_at, updated_at)
                SELECT hex(randomblob(16)), 'CUSTOMER', customer_id, 'PROJECT', project_id, 'belongs_to',
                       json_object('remark', ''),
                       created_at, datetime('now','localtime')
                FROM project_customer_mapping
                WHERE customer_id IS NOT NULL AND project_id IS NOT NULL;
            ") {
                log::error!("[迁移] 客户映射 → business_relations 失败: {}", e);
                all_ok = false;
            }
        }
        // 全部成功才标记迁移完成；失败则下次启动重试（INSERT OR IGNORE 幂等，重试安全）
        if all_ok {
            conn.execute(
                "INSERT OR REPLACE INTO system_settings (key, value) VALUES ('migration_br_mapping_done', '1')",
                [],
            ).ok();
            log::info!("[迁移] business_relations 映射迁移完成");
        } else {
            log::warn!("[迁移] business_relations 映射迁移存在失败项，将在下次启动重试");
        }
    }

    // ★ 处理 partner_contacts 表 schema 变更（移除 partner_id 列）
    let has_partner_id: bool = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('partner_contacts') WHERE name='partner_id'",
        [], |r| r.get::<_,i64>(0),
    ).unwrap_or(0) > 0;
    if has_partner_id {
        // 旧表有 partner_id 列，需要迁移数据并重建表
        // 先无条件删除残留备份：若上次在 DROP 与 RENAME 之间崩溃，IF NOT EXISTS 会跳过重建，
        // 导致 DROP 正表后 RENAME 陈旧备份 = 静默丢新数据
        let _ = conn.execute_batch("
            DROP TABLE IF EXISTS partner_contacts_backup;
            CREATE TABLE partner_contacts_backup AS SELECT id, name, phone, email, position, remark, created_at FROM partner_contacts;
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
                source_data_id TEXT DEFAULT NULL,
                source_type TEXT DEFAULT '',
                source_id TEXT DEFAULT '',
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

    // ★ 热点谓词索引化（审计P2）：先建新名索引（消除 idx_kc_source 一名两义），再删旧名
    let _ = conn.execute_batch("
        CREATE INDEX IF NOT EXISTS idx_kc_sd_id ON knowledge_chunks(source_data_id);
        CREATE INDEX IF NOT EXISTS idx_kc_st_sid ON knowledge_chunks(source_type, source_id);
        CREATE UNIQUE INDEX IF NOT EXISTS idx_kc_chunk_id ON knowledge_chunks(chunk_id) WHERE chunk_id <> '';
        DROP INDEX IF EXISTS idx_kc_source;
        CREATE INDEX IF NOT EXISTS idx_sd_updated_at ON source_data(updated_at);
        CREATE INDEX IF NOT EXISTS idx_sd_source_id ON source_data(source_id);
        CREATE INDEX IF NOT EXISTS idx_pushq_status_created ON knowledge_push_queue(status, created_at);
        CREATE INDEX IF NOT EXISTS idx_pushq_st_sid ON knowledge_push_queue(source_type, source_id);
        DELETE FROM knowledge_push_queue WHERE status='pending' AND id NOT IN (SELECT MIN(id) FROM knowledge_push_queue WHERE status='pending' GROUP BY source_type, source_id);
        CREATE UNIQUE INDEX IF NOT EXISTS idx_pushq_pending_uniq ON knowledge_push_queue(source_type, source_id) WHERE status='pending';
        CREATE INDEX IF NOT EXISTS idx_emails_acct_folder_recv ON emails(account_id, folder, received_at);
    ");

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

    // ★ notifications 增加 meta 列（结构化载荷预留）
    let _ = conn.execute_batch("ALTER TABLE notifications ADD COLUMN meta TEXT DEFAULT '{}'");

    // ★ notifications 增加 actions 列（操作按钮 JSON 数组）
    let _ = conn.execute_batch("ALTER TABLE notifications ADD COLUMN actions TEXT DEFAULT '[]'");

    // ★ 归一化 notes 时间戳格式：存量 RFC3339（含 'T'）→ SQLite "YYYY-MM-DD HH:MM:SS" 本地格式。
    //    排序按字符串字典序，混合格式会导致 space(0x20) < 'T'(0x54) 使同一天排序错乱。
    //    datetime() 解析 RFC3339 带时区偏移 → 归一 UTC → 'localtime' 转回本地，等于原本地时刻。
    //    幂等：转换后不含 'T'，下次启动不再命中。
    for (col, idx_name) in [
        ("updated_at", "idx_notes_updated_at"),
        ("created_at", "idx_notes_created_at"),
        ("deleted_at", "idx_notes_deleted"),
    ] {
        let migrated: i64 = conn
            .execute(
                &format!(
                    "UPDATE notes SET {col}=datetime({col}, 'localtime') WHERE {col} LIKE '%T%' AND datetime({col}, 'localtime') IS NOT NULL"
                ),
                [],
            )
            .map(|n| n as i64)
            .unwrap_or(0);
        if migrated > 0 {
            log::info!("[数据库] notes.{col} 时间戳格式归一化: {} 条 (RFC3339 → SQLite)", migrated);
        }
        let _ = conn.execute_batch(&format!("CREATE INDEX IF NOT EXISTS {idx_name} ON notes({col});"));
    }

    // ★ 存量自愈：Outlook CID 垃圾后缀（image.jpg@hex.hash 之类）导致 file_ext 为十六进制串、
    //   解析任务被误标 unsupported。按磁盘文件内容魔数修正 file_ext，并重置对应 unsupported 任务重新解析。
    //   幂等：仅"嗅探出真实类型且与现值不同"才修正+重入队；魔数未命中的（WMF等）保持 unsupported。
    {
        let suspects: Vec<(String, String, String)> = conn
            .prepare("SELECT a.id, a.file_ext, a.content_hash FROM file_assets a \
                      JOIN knowledge_push_queue q ON q.source_id=a.id AND q.source_type='FILE' AND q.status='unsupported'")
            .and_then(|mut s| Ok(s.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?
                .filter_map(|x| x.ok()).collect()))
            .unwrap_or_default();
        let mut fixed = 0usize;
        for (id, cur_ext, hash) in suspects {
            if cur_ext == "bin" || cur_ext.is_empty() { continue; }
            let path = crate::commands::file_commands::flat_disk_path(&hash);
            let true_ext = crate::services::file_parser::sniff_file_ext(&path, "");
            if !true_ext.is_empty() && true_ext != cur_ext {
                let _ = conn.execute("UPDATE file_assets SET file_ext=?1 WHERE id=?2", rusqlite::params![&true_ext, &id]);
                let _ = conn.execute(
                    "UPDATE knowledge_push_queue SET status='pending', last_error='', retry_count=0 WHERE source_type='FILE' AND source_id=?1 AND status='unsupported'",
                    rusqlite::params![&id]);
                fixed += 1;
            }
        }
        if fixed > 0 {
            log::info!("[数据库] file_ext 内容嗅探自愈: 修正 {} 个文件并重新入队解析", fixed);
        }
    }

    // ★ 项目 canonical ID 迁移：阶段 A —— 仅准备状态字段与审计表。
    //    真正的数据迁移由前端启动后提交 identity map 触发（migration_submit_project_id_map），
    //    因为 run_migrations 执行时前端尚未启动，无法获得 localId→backendId 映射。
    crate::database::project_id_migration::ensure_migration_infrastructure(conn);

    // ★ 存量 emails.received_at 归一化：原始 RFC2822 字符串（如 "Mon, 15 Sep 2026 10:30:00 +0800"）
    //    无法字典序排序，统一转为本地时间 'YYYY-MM-DD HH:MM:SS'。
    //    幂等：转换后匹配 GLOB 模式，下次启动不再命中。解析失败的行回退 created_at。
    let bad_rows: Vec<(String, String, String)> = conn
        .prepare(
            "SELECT id, received_at, created_at FROM emails \
             WHERE received_at != '' AND received_at NOT GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9] [0-9][0-9]:[0-9][0-9]:[0-9][0-9]'",
        )
        .and_then(|mut s| s.query_map([], |r| Ok((
            r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2).unwrap_or_default(),
        ))).map(|rows| rows.filter_map(|x| x.ok()).collect()))
        .unwrap_or_default();
    let mut fixed = 0i64;
    for (id, raw, created) in &bad_rows {
        let normalized = chrono::DateTime::parse_from_rfc2822(raw)
            .or_else(|_| chrono::DateTime::parse_from_rfc3339(raw))
            .ok()
            .map(|dt| dt.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S").to_string())
            .or_else(|| if created.is_empty() { None } else { Some(created.clone()) });
        if let Some(n) = normalized {
            let _ = conn.execute("UPDATE emails SET received_at=?1 WHERE id=?2", rusqlite::params![n, id]);
            fixed += 1;
        }
    }
    if fixed > 0 {
        log::info!("[数据库] emails.received_at 时间戳归一化: {} 条 (RFC2822 → 本地时间)", fixed);
    }

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
        CREATE INDEX IF NOT EXISTS idx_kc_st_sid ON knowledge_chunks(source_type, source_id);
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

#[cfg(test)]
mod tests {
    use super::run_migrations;
    use crate::database::db_schema;

    fn schema_dump(conn: &rusqlite::Connection) -> String {
        let mut stmt = conn.prepare(
            "SELECT type, name, sql FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY type, name"
        ).unwrap();
        let rows = stmt.query_map([], |r| {
            Ok(format!("{}|{}|{}", r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?.unwrap_or_default()))
        }).unwrap();
        rows.flatten().collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn test_run_migrations_idempotent() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        db_schema::create_schema(&conn).unwrap();
        run_migrations(&conn).unwrap();
        let schema1 = schema_dump(&conn);

        run_migrations(&conn).unwrap();
        let schema2 = schema_dump(&conn);

        assert_eq!(schema1, schema2, "run_migrations 二次执行不得改变 schema");
    }

    #[test]
    fn test_knowledge_chunks_legacy_rebuild_is_defensive() {
        // 模拟远古库：source_data_id 为 INTEGER 且缺 source_type/source_id/base_weight 等列，
        // 迁移须防御式重建（缺列补默认值）并保留存量数据，而不是报错。
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        db_schema::create_schema(&conn).unwrap();
        run_migrations(&conn).unwrap();

        conn.execute_batch("
            DROP INDEX IF EXISTS idx_kc_source;
            DROP TABLE knowledge_chunks;
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
                created_at TEXT DEFAULT (datetime('now','localtime'))
            );
            INSERT INTO knowledge_chunks(id, chunk_id, chunk_type, source_data_id, content)
                VALUES (5, 'abc123', 'parent', 42, 'hello');
            INSERT OR IGNORE INTO source_data(id, source_type, source_id, title, content)
                VALUES ('42', 'NOTE', 'sid-42', 't', 'c');
        ").unwrap();

        run_migrations(&conn).unwrap();

        let sd_type: String = conn.query_row(
            "SELECT type FROM pragma_table_info('knowledge_chunks') WHERE name='source_data_id'",
            [], |r| r.get(0)).unwrap();
        assert_eq!(sd_type, "TEXT", "source_data_id 应已迁移为 TEXT");
        let has_cols: i64 = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('knowledge_chunks') WHERE name IN ('source_type','source_id')",
            [], |r| r.get(0)).unwrap();
        assert_eq!(has_cols, 2, "source_type/source_id 列应已补齐");
        let (sid, content, base_w): (String, String, f64) = conn.query_row(
            "SELECT source_data_id, content, base_weight FROM knowledge_chunks WHERE id=5",
            [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap();
        assert_eq!(sid, "42");
        assert_eq!(content, "hello");
        assert_eq!(base_w, 1.0, "缺列应补默认值");
    }
}
