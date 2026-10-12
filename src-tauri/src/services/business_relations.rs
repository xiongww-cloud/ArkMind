use crate::database;
use serde_json::json;

/// Initialize the business_relations table
/// 返回 true 表示检测并修复了 INTEGER id 坏表（调用方应触发数据回填恢复）
pub fn init() -> bool {
    let mut rebuilt_from_integer = false;
    // 检查表是否存在，如果存在则检查是否有 UNIQUE 约束
    let has_table: bool = database::get(
        "SELECT COUNT(*) as cnt FROM sqlite_master WHERE type='table' AND name='business_relations'",
        &[]
    ).ok().flatten().and_then(|r| r["cnt"].as_i64()).unwrap_or(0) > 0;

    if has_table {
        // ★ 修复：db_schema.rs 曾用 INTEGER PRIMARY KEY 创建本表，
        // 导致所有 TEXT id 的 INSERT 静默失败（datatype mismatch）。
        // 检测 id 列类型，若为 INTEGER 则重建为 TEXT。
        let id_is_integer: bool = database::get(
            "SELECT COUNT(*) as cnt FROM pragma_table_info('business_relations') WHERE name='id' AND UPPER(type)='INTEGER'",
            &[]
        ).ok().flatten().and_then(|r| r["cnt"].as_i64()).unwrap_or(0) > 0;

        if id_is_integer {
            log::warn!("[业务关联] 检测到 id 列为 INTEGER（旧 schema），正在重建为 TEXT...");
            let r = crate::database::with_db(|conn| {
                conn.execute_batch("
                    CREATE TABLE business_relations_fix (
                        id TEXT PRIMARY KEY,
                        source_type TEXT NOT NULL,
                        source_id TEXT NOT NULL,
                        target_type TEXT NOT NULL,
                        target_id TEXT NOT NULL,
                        relation_type TEXT NOT NULL DEFAULT 'belongs_to',
                        metadata TEXT DEFAULT '{}',
                        created_at TEXT DEFAULT (datetime('now', 'localtime')),
                        updated_at TEXT DEFAULT (datetime('now', 'localtime')),
                        UNIQUE(source_type, source_id, target_type, target_id, relation_type)
                    );
                    INSERT OR IGNORE INTO business_relations_fix
                        SELECT CAST(id AS TEXT), source_type, source_id, target_type, target_id,
                               relation_type, metadata, created_at, updated_at
                        FROM business_relations;
                    DROP TABLE business_relations;
                    ALTER TABLE business_relations_fix RENAME TO business_relations;
                    CREATE UNIQUE INDEX IF NOT EXISTS idx_br_unique ON business_relations(source_type, source_id, target_type, target_id, relation_type);
                    CREATE INDEX IF NOT EXISTS idx_br_source ON business_relations(source_type, source_id);
                    CREATE INDEX IF NOT EXISTS idx_br_target ON business_relations(target_type, target_id);
                    CREATE INDEX IF NOT EXISTS idx_br_type ON business_relations(relation_type);
                    CREATE INDEX IF NOT EXISTS idx_br_updated ON business_relations(updated_at);
                ")
            });
            match r {
                Ok(_) => { log::info!("[业务关联] id 列已重建为 TEXT"); rebuilt_from_integer = true; }
                Err(e) => log::error!("[业务关联] id 列重建失败: {}", e),
            }
        }

        // 检查是否有 UNIQUE 约束
        let has_unique: bool = database::get(
            "SELECT COUNT(*) as cnt FROM sqlite_master WHERE type='index' AND name='idx_br_unique'",
            &[]
        ).ok().flatten().and_then(|r| r["cnt"].as_i64()).unwrap_or(0) > 0;

        if !has_unique {
            log::info!("[业务关联] 检测到表缺少 UNIQUE 约束，正在重建...");
            // 清理重复数据
            cleanup_duplicates();
            // 重建表（保留数据）
            let _ = crate::database::with_db(|conn| {
                conn.execute_batch("
                    CREATE TABLE business_relations_new AS
                    SELECT MIN(id) as id, source_type, source_id, target_type, target_id, relation_type,
                           metadata, created_at, updated_at
                    FROM business_relations
                    GROUP BY source_type, source_id, target_type, target_id, relation_type;
                    DROP TABLE business_relations;
                    ALTER TABLE business_relations_new RENAME TO business_relations;
                    CREATE UNIQUE INDEX idx_br_unique ON business_relations(source_type, source_id, target_type, target_id, relation_type);
                ")
            });
            log::info!("[业务关联] 表已重建，UNIQUE 约束已添加");
        }
    } else {
        // 创建新表（带 UNIQUE 约束）
        let _ = database::run(
            "CREATE TABLE IF NOT EXISTS business_relations (
                id TEXT PRIMARY KEY,
                source_type TEXT NOT NULL,
                source_id TEXT NOT NULL,
                target_type TEXT NOT NULL,
                target_id TEXT NOT NULL,
                relation_type TEXT NOT NULL,
                metadata TEXT DEFAULT '{}',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                UNIQUE(source_type, source_id, target_type, target_id, relation_type)
            )", &[]
        );
    }

    // 确保索引存在
    let _ = database::run("CREATE INDEX IF NOT EXISTS idx_br_source ON business_relations(source_type, source_id)", &[]);
    let _ = database::run("CREATE INDEX IF NOT EXISTS idx_br_target ON business_relations(target_type, target_id)", &[]);
    let _ = database::run("CREATE INDEX IF NOT EXISTS idx_br_type ON business_relations(relation_type)", &[]);
    let _ = database::run("CREATE INDEX IF NOT EXISTS idx_br_updated ON business_relations(updated_at)", &[]);

    // 启动时清扫解决方案图谱孤儿边（修复历史遗留）
    cleanup_solution_graph_orphans();

    // ★ 方案 B：本表是映射表的派生视图，启动必做一次全量对账（幂等），
    // 任何同步路径漏写的边最迟下次启动自动收敛
    rebuild_all_derived_edges();

    rebuilt_from_integer
}

/// 清理解决方案图谱的孤儿边（边的源或目标节点已不存在于业务表）
/// 场景：方案/版本删除时业务行已先删，按 project_id 反查不到被删节点，导致边残留
pub fn cleanup_solution_graph_orphans() {
    let _ = database::run(
        "DELETE FROM business_relations WHERE relation_type IN ('has_equipment_list','has_version','uses','instance_of') AND (\
         (source_type='PROJECT' AND source_id NOT IN (SELECT id FROM projects)) OR \
         (source_type='EQUIPMENT_LIST' AND source_id NOT IN (SELECT id FROM solutions)) OR \
         (source_type='EQUIPMENT_VERSION' AND source_id NOT IN (SELECT id FROM solution_versions)) OR \
         (source_type='EQUIPMENT_ITEM' AND source_id NOT IN (SELECT id FROM solution_items)) OR \
         (target_type='EQUIPMENT_LIST' AND target_id NOT IN (SELECT id FROM solutions)) OR \
         (target_type='EQUIPMENT_VERSION' AND target_id NOT IN (SELECT id FROM solution_versions)) OR \
         (target_type='EQUIPMENT_ITEM' AND target_id NOT IN (SELECT id FROM solution_items)) OR \
         (target_type='PRODUCT' AND target_id NOT IN (SELECT id FROM products)))",
        &[]
    );
}

/// 清理重复数据（保留 id 最小的记录）
fn cleanup_duplicates() {
    let before: i64 = database::get("SELECT COUNT(*) as cnt FROM business_relations", &[])
        .ok().flatten().and_then(|r| r["cnt"].as_i64()).unwrap_or(0);

    let _ = database::run(
        "DELETE FROM business_relations WHERE id NOT IN (
            SELECT MIN(id) FROM business_relations
            GROUP BY source_type, source_id, target_type, target_id, relation_type
        )",
        &[]
    );

    let after: i64 = database::get("SELECT COUNT(*) as cnt FROM business_relations", &[])
        .ok().flatten().and_then(|r| r["cnt"].as_i64()).unwrap_or(0);

    if before > after {
        log::info!("[业务关联] 已清理 {} 条重复数据", before - after);
    }
}

/// 派生边全量对账（方案 B：业务映射表是唯一真相源，本表是可重建的派生视图）。
/// 幂等，每次启动必跑。对每个派生边族执行两步：
///   1) 删除"映射源里已不存在"的过期边（修历史双写漏网的 stale 边）
///   2) 从映射源 INSERT..ON CONFLICT UPDATE（补缺失边 + 用权威列刷新 metadata，
///      彻底消除 upsert 覆盖丢 remark/selected_contact_id 的陷阱——metadata 永远来自源列）
pub fn rebuild_all_derived_edges() {
    // 0. 方向规范化：旧版 backfill 曾按 PRODUCT→PROJECT 写入（运行时真相是 PROJECT→belongs_to→PRODUCT，
    //    由 sync_project_products 从 solution_items 派生），反向遗留边直接清除
    let _ = database::run(
        "DELETE FROM business_relations WHERE source_type='PRODUCT' AND target_type='PROJECT' AND relation_type='belongs_to'", &[]);

    // 1. FILE has_file（源：file_relations）
    let _ = database::run(
        "DELETE FROM business_relations WHERE relation_type='has_file' AND source_type='FILE' AND NOT EXISTS (\
           SELECT 1 FROM file_relations fr WHERE fr.file_id=business_relations.source_id \
             AND upper(fr.entity_type)=business_relations.target_type AND fr.entity_id=business_relations.target_id)", &[]);
    let _ = database::with_db(|conn| {
        conn.execute(
            "INSERT INTO business_relations(id,source_type,source_id,target_type,target_id,relation_type,metadata,created_at,updated_at) \
             SELECT lower(hex(randomblob(16))),'FILE',fr.file_id,upper(fr.entity_type),fr.entity_id,'has_file', \
               CASE WHEN COALESCE(fr.description,'')!='' THEN json_object('custom_name',fr.custom_name,'creator',fr.creator,'description',fr.description) \
                    ELSE json_object('custom_name',fr.custom_name,'creator',fr.creator) END, \
               datetime('now','localtime'),datetime('now','localtime') \
             FROM file_relations fr WHERE COALESCE(fr.entity_id,'')!='' \
             ON CONFLICT(source_type,source_id,target_type,target_id,relation_type) DO UPDATE SET metadata=excluded.metadata, updated_at=datetime('now','localtime')", [])
    });

    // 2. NOTE authored_by PROJECT（源：note_project_mapping，排除回收站笔记）
    let _ = database::run(
        "DELETE FROM business_relations WHERE source_type='NOTE' AND target_type='PROJECT' AND relation_type='authored_by' AND NOT EXISTS (\
           SELECT 1 FROM note_project_mapping npm JOIN notes n ON n.note_id=npm.note_id AND n.deleted_at IS NULL \
           WHERE npm.note_id=business_relations.source_id AND npm.project_id=business_relations.target_id)", &[]);
    let _ = database::with_db(|conn| {
        conn.execute(
            "INSERT INTO business_relations(id,source_type,source_id,target_type,target_id,relation_type,metadata,created_at,updated_at) \
             SELECT lower(hex(randomblob(16))),'NOTE',npm.note_id,'PROJECT',npm.project_id,'authored_by','{}', \
               datetime('now','localtime'),datetime('now','localtime') \
             FROM note_project_mapping npm JOIN notes n ON n.note_id=npm.note_id AND n.deleted_at IS NULL \
             JOIN projects p ON p.id=npm.project_id \
             ON CONFLICT(source_type,source_id,target_type,target_id,relation_type) DO UPDATE SET updated_at=datetime('now','localtime')", [])
    });

    // 3. TODO belongs_to PROJECT（源：todo_project_mapping）
    let _ = database::run(
        "DELETE FROM business_relations WHERE source_type='TODO' AND target_type='PROJECT' AND relation_type='belongs_to' AND NOT EXISTS (\
           SELECT 1 FROM todo_project_mapping tpm JOIN todos td ON td.id=tpm.todo_id JOIN projects p ON p.id=tpm.project_id \
           WHERE tpm.todo_id=business_relations.source_id AND tpm.project_id=business_relations.target_id)", &[]);
    let _ = database::with_db(|conn| {
        conn.execute(
            "INSERT INTO business_relations(id,source_type,source_id,target_type,target_id,relation_type,metadata,created_at,updated_at) \
             SELECT lower(hex(randomblob(16))),'TODO',tpm.todo_id,'PROJECT',tpm.project_id,'belongs_to','{}', \
               datetime('now','localtime'),datetime('now','localtime') \
             FROM todo_project_mapping tpm JOIN todos td ON td.id=tpm.todo_id JOIN projects p ON p.id=tpm.project_id \
             ON CONFLICT(source_type,source_id,target_type,target_id,relation_type) DO UPDATE SET updated_at=datetime('now','localtime')", [])
    });

    // 4. PROJECT belongs_to PRODUCT（方向与 sync_project_products 一致：项目→产品；源：solution_items，非 mapping 表）
    let _ = database::run(
        "DELETE FROM business_relations WHERE source_type='PROJECT' AND target_type='PRODUCT' AND relation_type='belongs_to' AND NOT EXISTS (\
           SELECT 1 FROM solution_items si JOIN solution_versions sv ON sv.id=si.solution_version_id JOIN solutions s ON s.id=sv.solution_id JOIN products pr ON pr.id=si.product_id \
           WHERE s.project_id=business_relations.source_id AND si.product_id=business_relations.target_id)", &[]);
    let _ = database::with_db(|conn| {
        conn.execute(
            "INSERT INTO business_relations(id,source_type,source_id,target_type,target_id,relation_type,metadata,created_at,updated_at) \
             SELECT DISTINCT lower(hex(randomblob(16))),s.project_id,'PROJECT','PRODUCT',si.product_id,'belongs_to', \
               json_object('product_name', COALESCE(si.product_name,'')), datetime('now','localtime'),datetime('now','localtime') \
             FROM solution_items si JOIN solution_versions sv ON sv.id=si.solution_version_id JOIN solutions s ON s.id=sv.solution_id \
             JOIN products pr ON pr.id=si.product_id WHERE COALESCE(si.product_id,'')!='' \
             ON CONFLICT(source_type,source_id,target_type,target_id,relation_type) DO UPDATE SET metadata=excluded.metadata, updated_at=datetime('now','localtime')", [])
    });

    // 5. CHANNEL/VENDOR/CUSTOMER belongs_to PROJECT（源：partner_project_mapping；type 以 partners 表为权威）
    let _ = database::run(
        "DELETE FROM business_relations WHERE source_type IN ('CHANNEL','VENDOR','CUSTOMER') AND target_type='PROJECT' AND relation_type='belongs_to' AND NOT EXISTS (\
           SELECT 1 FROM partner_project_mapping ppm JOIN partners pa ON pa.id=ppm.partner_id AND upper(pa.type)=business_relations.source_type JOIN projects pr ON pr.id=ppm.project_id \
           WHERE ppm.partner_id=business_relations.source_id AND ppm.project_id=business_relations.target_id)", &[]);
    let _ = database::with_db(|conn| {
        conn.execute(
            "INSERT INTO business_relations(id,source_type,source_id,target_type,target_id,relation_type,metadata,created_at,updated_at) \
             SELECT lower(hex(randomblob(16))),upper(pa.type),ppm.partner_id,'PROJECT',ppm.project_id,'belongs_to', \
               json_object('remark',COALESCE(ppm.remark,''),'selected_contact_id',COALESCE(ppm.selected_contact_id,'')), \
               datetime('now','localtime'),datetime('now','localtime') \
             FROM partner_project_mapping ppm JOIN partners pa ON pa.id=ppm.partner_id JOIN projects pr ON pr.id=ppm.project_id \
             ON CONFLICT(source_type,source_id,target_type,target_id,relation_type) DO UPDATE SET metadata=excluded.metadata, updated_at=datetime('now','localtime')", [])
    });

    // 6. CONTACT employed_by PARTNER（源：partner_contact_mapping）
    let _ = database::run(
        "DELETE FROM business_relations WHERE relation_type='employed_by' AND source_type='CONTACT' AND NOT EXISTS (\
           SELECT 1 FROM partner_contact_mapping pcm JOIN partners pa ON pa.id=pcm.partner_id AND upper(pa.type)=business_relations.target_type JOIN partner_contacts c ON c.id=pcm.contact_id \
           WHERE pcm.contact_id=business_relations.source_id AND pcm.partner_id=business_relations.target_id)", &[]);
    let _ = database::with_db(|conn| {
        conn.execute(
            "INSERT INTO business_relations(id,source_type,source_id,target_type,target_id,relation_type,metadata,created_at,updated_at) \
             SELECT lower(hex(randomblob(16))),'CONTACT',pcm.contact_id,upper(pa.type),pcm.partner_id,'employed_by','{}', \
               datetime('now','localtime'),datetime('now','localtime') \
             FROM partner_contact_mapping pcm JOIN partners pa ON pa.id=pcm.partner_id JOIN partner_contacts c ON c.id=pcm.contact_id \
             ON CONFLICT(source_type,source_id,target_type,target_id,relation_type) DO UPDATE SET updated_at=datetime('now','localtime')", [])
    });

    // 7. CONTACT belongs_to PROJECT（源：contact_project_mapping，remark 进 metadata）
    let _ = database::run(
        "DELETE FROM business_relations WHERE source_type='CONTACT' AND target_type='PROJECT' AND relation_type='belongs_to' AND NOT EXISTS (\
           SELECT 1 FROM contact_project_mapping cpm JOIN partner_contacts c ON c.id=cpm.contact_id JOIN projects pr ON pr.id=cpm.project_id \
           WHERE cpm.contact_id=business_relations.source_id AND cpm.project_id=business_relations.target_id)", &[]);
    let _ = database::with_db(|conn| {
        conn.execute(
            "INSERT INTO business_relations(id,source_type,source_id,target_type,target_id,relation_type,metadata,created_at,updated_at) \
             SELECT lower(hex(randomblob(16))),'CONTACT',cpm.contact_id,'PROJECT',cpm.project_id,'belongs_to', \
               json_object('remark',COALESCE(cpm.remark,'')), datetime('now','localtime'),datetime('now','localtime') \
             FROM contact_project_mapping cpm JOIN partner_contacts c ON c.id=cpm.contact_id JOIN projects pr ON pr.id=cpm.project_id \
             ON CONFLICT(source_type,source_id,target_type,target_id,relation_type) DO UPDATE SET metadata=excluded.metadata, updated_at=datetime('now','localtime')", [])
    });

    // 8. 设备清单图谱（源：solutions/solution_versions/solution_items 业务表；写入方是
    //    project_commands::sync_solution_graph）。导入路径（project_import_inner/import_projects）
    //    直插业务表不走 CRUD 命令，曾长期漏边——检测"有清单但缺 has_equipment_list 边"的项目
    //    并逐个全量重建，任何漏写最迟下次启动收敛。
    {
        let missing: Vec<String> = database::all(
            "SELECT DISTINCT s.project_id FROM solutions s \
             WHERE COALESCE(s.project_id,'')!='' AND NOT EXISTS (\
               SELECT 1 FROM business_relations br WHERE br.source_type='PROJECT' \
                 AND br.source_id=s.project_id AND br.target_type='EQUIPMENT_LIST' \
                 AND br.target_id=s.id AND br.relation_type='has_equipment_list')",
            &[],
        ).unwrap_or_default().iter().filter_map(|r| r["project_id"].as_str().map(String::from)).collect();
        if !missing.is_empty() {
            log::info!("[业务关联] 检测到 {} 个项目缺设备清单图谱边，全量重建中...", missing.len());
            for pid in &missing {
                let _ = crate::commands::project_commands::sync_project_products(pid);
                crate::commands::project_commands::sync_solution_graph(pid);
            }
        }
    }

    let edges: i64 = database::get("SELECT COUNT(*) c FROM business_relations", &[])
        .ok().flatten().and_then(|r| r["c"].as_i64()).unwrap_or(0);
    log::info!("[业务关联] 派生边对账完成，当前 {} 条", edges);
}

/// 单实体重建：某联系人的全部派生边（employed_by + belongs_to PROJECT）
pub fn resync_contact_edges(contact_id: &str) {
    if contact_id.is_empty() { return; }
    let _ = database::run(
        "DELETE FROM business_relations WHERE source_type='CONTACT' AND source_id=?", &[json!(contact_id)]);
    if let Ok(rows) = database::all(
        "SELECT pcm.partner_id, upper(pa.type) ptype FROM partner_contact_mapping pcm JOIN partners pa ON pa.id=pcm.partner_id WHERE pcm.contact_id=?",
        &[json!(contact_id)]) {
        for r in &rows {
            upsert("CONTACT", contact_id, r["ptype"].as_str().unwrap_or("CHANNEL"), r["partner_id"].as_str().unwrap_or(""), "employed_by", "{}");
        }
    }
    if let Ok(rows) = database::all(
        "SELECT project_id, COALESCE(remark,'') remark FROM contact_project_mapping WHERE contact_id=?",
        &[json!(contact_id)]) {
        for r in &rows {
            let meta = serde_json::json!({"remark": r["remark"].as_str().unwrap_or("")}).to_string();
            upsert("CONTACT", contact_id, "PROJECT", r["project_id"].as_str().unwrap_or(""), "belongs_to", &meta);
        }
    }
}

/// 单实体重建：某合作伙伴→项目的 belongs_to 边（源：partner_project_mapping）
pub fn resync_partner_edges(partner_id: &str) {
    if partner_id.is_empty() { return; }
    let _ = database::run(
        "DELETE FROM business_relations WHERE source_id=? AND relation_type='belongs_to' AND target_type='PROJECT'", &[json!(partner_id)]);
    let ptype = database::get("SELECT type FROM partners WHERE id=?", &[json!(partner_id)])
        .ok().flatten().and_then(|r| r["type"].as_str().map(|s| s.to_uppercase()))
        .unwrap_or_else(|| "CHANNEL".into());
    if let Ok(rows) = database::all(
        "SELECT project_id, COALESCE(remark,'') remark, COALESCE(selected_contact_id,'') sc FROM partner_project_mapping WHERE partner_id=?",
        &[json!(partner_id)]) {
        for r in &rows {
            let meta = serde_json::json!({"remark": r["remark"].as_str().unwrap_or(""), "selected_contact_id": r["sc"].as_str().unwrap_or("")}).to_string();
            upsert(&ptype, partner_id, "PROJECT", r["project_id"].as_str().unwrap_or(""), "belongs_to", &meta);
        }
    }
}

/// 单实体重建：某待办→项目的 belongs_to 边（源：todo_project_mapping）
pub fn resync_todo_edges(todo_id: &str) {
    if todo_id.is_empty() { return; }
    let _ = database::run(
        "DELETE FROM business_relations WHERE source_type='TODO' AND source_id=?", &[json!(todo_id)]);
    if let Ok(rows) = database::all(
        "SELECT project_id FROM todo_project_mapping WHERE todo_id=?", &[json!(todo_id)]) {
        for r in &rows {
            upsert("TODO", todo_id, "PROJECT", r["project_id"].as_str().unwrap_or(""), "belongs_to", "{}");
        }
    }
}

/// 单实体重建：某笔记→项目的 authored_by 边（源：note_project_mapping）
pub fn resync_note_edges(note_id: &str) {
    if note_id.is_empty() { return; }
    let _ = database::run(
        "DELETE FROM business_relations WHERE source_type='NOTE' AND source_id=?", &[json!(note_id)]);
    if let Ok(rows) = database::all(
        "SELECT npm.project_id FROM note_project_mapping npm JOIN notes n ON n.note_id=npm.note_id AND n.deleted_at IS NULL WHERE npm.note_id=?",
        &[json!(note_id)]) {
        for r in &rows {
            upsert("NOTE", note_id, "PROJECT", r["project_id"].as_str().unwrap_or(""), "authored_by", "{}");
        }
    }
}

/// Insert or update a business relation (dedup by source+target+type)
pub fn upsert(
    source_type: &str, source_id: &str,
    target_type: &str, target_id: &str,
    relation_type: &str,
    metadata: &str,
) {
    let r = database::with_db(|conn| {
        upsert_on(conn, source_type, source_id, target_type, target_id, relation_type, metadata)
            .map_err(|e| rusqlite::Error::InvalidParameterName(e))
    });
    match r {
        Ok(_) => log::info!("[业务关联] upsert: {}[{}] → {}[{}] ({})", source_type, source_id, target_type, target_id, relation_type),
        Err(e) => log::error!("[业务关联] upsert 失败: {}[{}] → {}[{}] ({}): {}", source_type, source_id, target_type, target_id, relation_type, e),
    }
}

pub(crate) fn upsert_on(
    conn: &rusqlite::Connection,
    source_type: &str, source_id: &str,
    target_type: &str, target_id: &str,
    relation_type: &str,
    metadata: &str,
) -> Result<(), String> {
    if source_id.is_empty() || target_id.is_empty() { return Ok(()); }
    // 单语句原子 upsert（idx_br_unique 已覆盖五元组）：原 UPDATE→INSERT 两步在并发下 INSERT 撞唯一约束
    let id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO business_relations(id, source_type, source_id, target_type, target_id, relation_type, metadata, created_at, updated_at) \
         VALUES(?1,?2,?3,?4,?5,?6,?7,datetime('now','localtime'),datetime('now','localtime')) \
         ON CONFLICT(source_type, source_id, target_type, target_id, relation_type) \
         DO UPDATE SET metadata=excluded.metadata, updated_at=datetime('now','localtime')",
        rusqlite::params![id, source_type, source_id, target_type, target_id, relation_type, metadata],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

/// Delete a specific business relation
pub fn delete(
    source_type: &str, source_id: &str,
    target_type: &str, target_id: &str,
    relation_type: &str,
) {
    let _ = database::with_db(|conn| {
        delete_on(conn, source_type, source_id, target_type, target_id, relation_type)
            .map_err(|e| rusqlite::Error::InvalidParameterName(e))
    });
    log::info!("[业务关联] 删除: {}[{}] → {}[{}] ({})", source_type, source_id, target_type, target_id, relation_type);
}

pub(crate) fn delete_on(
    conn: &rusqlite::Connection,
    source_type: &str, source_id: &str,
    target_type: &str, target_id: &str,
    relation_type: &str,
) -> Result<(), String> {
    conn.execute(
        "DELETE FROM business_relations WHERE source_type=? AND source_id=? AND target_type=? AND target_id=? AND relation_type=?",
        rusqlite::params![source_type, source_id, target_type, target_id, relation_type],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

/// Delete all relations where an entity is the source
pub fn delete_by_source(source_type: &str, source_id: &str) {
    let _ = database::with_db(|conn| {
        delete_by_source_on(conn, source_type, source_id)
            .map_err(|e| rusqlite::Error::InvalidParameterName(e))
    });
    log::info!("[业务关联] 删除所有源关联: {}[{}]", source_type, source_id);
}

pub(crate) fn delete_by_source_on(conn: &rusqlite::Connection, source_type: &str, source_id: &str) -> Result<(), String> {
    conn.execute(
        "DELETE FROM business_relations WHERE source_type=? AND source_id=?",
        rusqlite::params![source_type, source_id],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

/// Delete all relations where an entity is the target
pub fn delete_by_target(target_type: &str, target_id: &str) {
    if target_id.is_empty() { return; }
    let _ = database::run(
        "DELETE FROM business_relations WHERE target_type=? AND target_id=?",
        &[json!(target_type), json!(target_id)]);
    log::info!("[业务关联] 删除所有目标关联: {}[{}]", target_type, target_id);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("
            CREATE TABLE business_relations (
                id TEXT PRIMARY KEY, source_type TEXT NOT NULL, source_id TEXT NOT NULL,
                target_type TEXT NOT NULL, target_id TEXT NOT NULL,
                relation_type TEXT NOT NULL DEFAULT 'belongs_to', metadata TEXT DEFAULT '{}',
                created_at TEXT DEFAULT '', updated_at TEXT DEFAULT ''
            );
            CREATE UNIQUE INDEX idx_br_unique ON business_relations(source_type, source_id, target_type, target_id, relation_type);
        ").unwrap();
        conn
    }

    fn count(conn: &rusqlite::Connection, sql: &str) -> i64 {
        conn.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn test_upsert_idempotent() {
        let conn = fixture();
        upsert_on(&conn, "NOTE", "n1", "PROJECT", "p1", "authored_by", "{}").unwrap();
        upsert_on(&conn, "NOTE", "n1", "PROJECT", "p1", "authored_by", "{\"x\":1}").unwrap();
        upsert_on(&conn, "NOTE", "n1", "PROJECT", "p1", "authored_by", "{}").unwrap();
        // 幂等：重复 upsert 不产生重复行
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM business_relations"), 1);
        // 重复 upsert 走 UPDATE 路径（metadata 被更新）
        upsert_on(&conn, "NOTE", "n1", "PROJECT", "p1", "authored_by", "{\"x\":2}").unwrap();
        let meta: String = conn.query_row(
            "SELECT metadata FROM business_relations WHERE source_id='n1'", [], |r| r.get(0)
        ).unwrap();
        assert_eq!(meta, "{\"x\":2}");
    }

    #[test]
    fn test_upsert_empty_ids_noop() {
        let conn = fixture();
        upsert_on(&conn, "NOTE", "", "PROJECT", "p1", "authored_by", "{}").unwrap();
        upsert_on(&conn, "NOTE", "n1", "PROJECT", "", "authored_by", "{}").unwrap();
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM business_relations"), 0);
    }

    #[test]
    fn test_delete_specific() {
        let conn = fixture();
        upsert_on(&conn, "NOTE", "n1", "PROJECT", "p1", "authored_by", "{}").unwrap();
        upsert_on(&conn, "NOTE", "n1", "PROJECT", "p2", "authored_by", "{}").unwrap();
        delete_on(&conn, "NOTE", "n1", "PROJECT", "p1", "authored_by").unwrap();
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM business_relations"), 1);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM business_relations WHERE target_id='p2'"), 1);
    }

    #[test]
    fn test_delete_by_source_cascade() {
        let conn = fixture();
        upsert_on(&conn, "TODO", "t1", "PROJECT", "p1", "belongs_to", "{}").unwrap();
        upsert_on(&conn, "TODO", "t1", "PROJECT", "p2", "belongs_to", "{}").unwrap();
        upsert_on(&conn, "TODO", "t2", "PROJECT", "p1", "belongs_to", "{}").unwrap();
        delete_by_source_on(&conn, "TODO", "t1").unwrap();
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM business_relations"), 1);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM business_relations WHERE source_id='t2'"), 1);
    }

    /// 导入漏边自愈：solutions 存在但缺 has_equipment_list 边 → rebuild_all_derived_edges 检测并重建四族边（幂等）
    #[test]
    fn rebuild_backfills_missing_equipment_graph_edges() {
        let (_dir, _g) = crate::database::db_core::test_util::setup_test_db();
        database::with_db(|c| c.execute_batch(
            "CREATE TABLE business_relations(id TEXT PRIMARY KEY, source_type TEXT NOT NULL, source_id TEXT NOT NULL, target_type TEXT NOT NULL, target_id TEXT NOT NULL, relation_type TEXT NOT NULL, metadata TEXT DEFAULT '{}', created_at TEXT NOT NULL DEFAULT '', updated_at TEXT NOT NULL DEFAULT '', UNIQUE(source_type, source_id, target_type, target_id, relation_type));
             CREATE TABLE solutions(id TEXT PRIMARY KEY, project_id TEXT, name TEXT, created_at TEXT DEFAULT '', updated_at TEXT DEFAULT '');
             CREATE TABLE solution_versions(id TEXT PRIMARY KEY, solution_id TEXT, version TEXT, status TEXT, creator TEXT, created_at TEXT DEFAULT '', updated_at TEXT DEFAULT '');
             CREATE TABLE solution_items(id TEXT PRIMARY KEY, solution_version_id TEXT, product_id TEXT, product_name TEXT, device_name TEXT, quantity INTEGER, unit_price REAL, description TEXT, row_type TEXT, sort_order INTEGER);
             CREATE TABLE products(id TEXT PRIMARY KEY);
             INSERT INTO solutions(id, project_id, name) VALUES('s1','p1','清单A');
             INSERT INTO solution_versions(id, solution_id, version, status, creator, created_at) VALUES('v1','s1','V1','finalized','张','2026-10-01');
             INSERT INTO solution_items(id, solution_version_id, product_id, product_name, device_name, quantity, unit_price, description, row_type, sort_order) VALUES('i1','v1','prd1','AP','核心交换机',2,100.0,'48口','device',1);"
        )).unwrap();

        let cnt = |sql: &str| -> i64 {
            database::get(sql, &[]).ok().flatten().and_then(|r| r["c"].as_i64()).unwrap_or(-1)
        };
        // 重建前：无任何设备清单图谱边（模拟导入直插漏写）
        assert_eq!(cnt("SELECT COUNT(*) c FROM business_relations WHERE relation_type='has_equipment_list'"), 0);

        rebuild_all_derived_edges();

        assert_eq!(cnt("SELECT COUNT(*) c FROM business_relations WHERE relation_type='has_equipment_list'"), 1, "PROJECT→EQUIPMENT_LIST 补建");
        assert_eq!(cnt("SELECT COUNT(*) c FROM business_relations WHERE relation_type='has_version'"), 1, "EQUIPMENT_LIST→VERSION 补建");
        assert_eq!(cnt("SELECT COUNT(*) c FROM business_relations WHERE relation_type='uses'"), 1, "VERSION→ITEM 补建");

        // 幂等：重复对账不翻倍
        rebuild_all_derived_edges();
        assert_eq!(cnt("SELECT COUNT(*) c FROM business_relations WHERE relation_type='has_equipment_list'"), 1);
    }
}
