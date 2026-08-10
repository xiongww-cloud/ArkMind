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

    rebuilt_from_integer
}

/// 清理解决方案图谱的孤儿边（边的源或目标节点已不存在于业务表）
/// 场景：方案/版本删除时业务行已先删，按 project_id 反查不到被删节点，导致边残留
pub fn cleanup_solution_graph_orphans() {
    let _ = database::run(
        "DELETE FROM business_relations WHERE relation_type IN ('has_solution','has_version','uses','instance_of') AND (\
         (source_type='PROJECT' AND source_id NOT IN (SELECT id FROM projects)) OR \
         (source_type='SOLUTION' AND source_id NOT IN (SELECT id FROM solutions)) OR \
         (source_type='SOLUTION_VERSION' AND source_id NOT IN (SELECT id FROM solution_versions)) OR \
         (source_type='SOLUTION_ITEM' AND source_id NOT IN (SELECT id FROM solution_items)) OR \
         (target_type='SOLUTION' AND target_id NOT IN (SELECT id FROM solutions)) OR \
         (target_type='SOLUTION_VERSION' AND target_id NOT IN (SELECT id FROM solution_versions)) OR \
         (target_type='SOLUTION_ITEM' AND target_id NOT IN (SELECT id FROM solution_items)) OR \
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

/// Incremental sync: sync all business layer relations into business_relations table
/// upsert handles dedup, so full sync is safe to run every time
pub fn backfill() {
    log::info!("[业务关联] 开始同步业务关联...");
    let mut total = 0i64;

    // 1. file_relations → has_file (FILE → PRODUCT/NOTE/TODO/PROJECT/etc.)
    if let Ok(rows) = database::all(
        "SELECT fr.file_id, fr.entity_type, fr.entity_id FROM file_relations fr WHERE fr.entity_id IS NOT NULL AND fr.entity_id != ''",
        &[]
    ) {
        for r in &rows {
            let fid = r["file_id"].as_str().unwrap_or("");
            let etype = r["entity_type"].as_str().unwrap_or("").to_uppercase();
            let eid = r["entity_id"].as_str().unwrap_or("");
            if fid.is_empty() || eid.is_empty() { continue; }
            upsert("FILE", fid, &etype, eid, "has_file", "{}");
            total += 1;
        }
    }

    // 2. note_project_mapping → authored_by (NOTE → PROJECT)
    if let Ok(rows) = database::all(
        "SELECT note_id, project_id FROM note_project_mapping WHERE project_id IS NOT NULL AND project_id != ''",
        &[]
    ) {
        for r in &rows {
            let nid = r["note_id"].as_str().unwrap_or("");
            let pid = r["project_id"].as_str().unwrap_or("");
            if nid.is_empty() || pid.is_empty() { continue; }
            upsert("NOTE", nid, "PROJECT", pid, "authored_by", "{}");
            total += 1;
        }
    }

    // 3. todo_project_mapping → belongs_to (TODO → PROJECT)
    if let Ok(rows) = database::all(
        "SELECT todo_id, project_id FROM todo_project_mapping WHERE project_id IS NOT NULL AND project_id != ''",
        &[]
    ) {
        for r in &rows {
            let tid = r["todo_id"].as_str().unwrap_or("");
            let pid = r["project_id"].as_str().unwrap_or("");
            if tid.is_empty() || pid.is_empty() { continue; }
            upsert("TODO", tid, "PROJECT", pid, "belongs_to", "{}");
            total += 1;
        }
    }

    // 4. project_product_mapping → belongs_to (PRODUCT → PROJECT)
    if let Ok(rows) = database::all(
        "SELECT project_id, product_id FROM project_product_mapping WHERE product_id IS NOT NULL AND product_id != ''",
        &[]
    ) {
        for r in &rows {
            let pid = r["project_id"].as_str().unwrap_or("");
            let prid = r["product_id"].as_str().unwrap_or("");
            if pid.is_empty() || prid.is_empty() { continue; }
            upsert("PRODUCT", prid, "PROJECT", pid, "belongs_to", "{}");
            total += 1;
        }
    }

    // 5. partner_project_mapping → belongs_to (PARTNER → PROJECT)
    if let Ok(rows_full) = database::all(
        "SELECT partner_id, partner_type, project_id FROM partner_project_mapping WHERE project_id IS NOT NULL AND project_id != ''",
        &[]
    ) {
        for r in &rows_full {
                let partner_id = r["partner_id"].as_str().unwrap_or("");
                let ptype = r["partner_type"].as_str().unwrap_or("").to_uppercase();
                let project_id = r["project_id"].as_str().unwrap_or("");
                if partner_id.is_empty() || project_id.is_empty() || ptype.is_empty() { continue; }
                upsert(&ptype, partner_id, "PROJECT", project_id, "belongs_to", "{}");
                total += 1;
            }
    }

    // 6. contact_project_mapping → belongs_to (CONTACT → PROJECT)
    if let Ok(rows) = database::all(
        "SELECT contact_id, project_id FROM contact_project_mapping WHERE project_id IS NOT NULL AND project_id != ''",
        &[]
    ) {
        for r in &rows {
            let cid = r["contact_id"].as_str().unwrap_or("");
            let pid = r["project_id"].as_str().unwrap_or("");
            if cid.is_empty() || pid.is_empty() { continue; }
            upsert("CONTACT", cid, "PROJECT", pid, "belongs_to", "{}");
            total += 1;
        }
    }

    log::info!("[业务关联] Backfill 完成，共同步 {} 条关联", total);
}

/// Insert or update a business relation (dedup by source+target+type)
pub fn upsert(
    source_type: &str, source_id: &str,
    target_type: &str, target_id: &str,
    relation_type: &str,
    metadata: &str,
) {
    if source_id.is_empty() || target_id.is_empty() { return; }
    // Try update first
    let changes = database::run(
        "UPDATE business_relations SET metadata=?, updated_at=datetime('now','localtime') WHERE source_type=? AND source_id=? AND target_type=? AND target_id=? AND relation_type=?",
        &[json!(metadata), json!(source_type), json!(source_id), json!(target_type), json!(target_id), json!(relation_type)],
    );
    match changes {
        Ok(c) if c.changes > 0 => {
            log::info!("[业务关联] 更新: {}[{}] → {}[{}] ({})", source_type, source_id, target_type, target_id, relation_type);
            return;
        }
        Ok(_) => {}
        Err(e) => log::error!("[业务关联] 更新失败: {}[{}] → {}[{}] ({}): {}", source_type, source_id, target_type, target_id, relation_type, e),
    }
    // Insert new
    let id = uuid::Uuid::new_v4().to_string();
    match database::run(
        "INSERT INTO business_relations(id, source_type, source_id, target_type, target_id, relation_type, metadata, created_at, updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,datetime('now','localtime'),datetime('now','localtime'))",
        &[json!(id), json!(source_type), json!(source_id), json!(target_type), json!(target_id), json!(relation_type), json!(metadata)],
    ) {
        Ok(_) => log::info!("[业务关联] 新增: {}[{}] → {}[{}] ({})", source_type, source_id, target_type, target_id, relation_type),
        Err(e) => log::error!("[业务关联] 新增失败: {}[{}] → {}[{}] ({}): {}", source_type, source_id, target_type, target_id, relation_type, e),
    }
}

/// Delete a specific business relation
pub fn delete(
    source_type: &str, source_id: &str,
    target_type: &str, target_id: &str,
    relation_type: &str,
) {
    let _ = database::run(
        "DELETE FROM business_relations WHERE source_type=? AND source_id=? AND target_type=? AND target_id=? AND relation_type=?",
        &[json!(source_type), json!(source_id), json!(target_type), json!(target_id), json!(relation_type)],
    );
    log::info!("[业务关联] 删除: {}[{}] → {}[{}] ({})", source_type, source_id, target_type, target_id, relation_type);
}

/// Delete all relations where an entity is the source
pub fn delete_by_source(source_type: &str, source_id: &str) {
    let _ = database::run(
        "DELETE FROM business_relations WHERE source_type=? AND source_id=?",
        &[json!(source_type), json!(source_id)],
    );
    log::info!("[业务关联] 删除所有源关联: {}[{}]", source_type, source_id);
}
