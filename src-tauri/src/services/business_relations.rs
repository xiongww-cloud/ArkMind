use crate::database;
use serde_json::{json, Value};

/// Initialize the business_relations table
pub fn init() {
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
    let _ = database::run("CREATE INDEX IF NOT EXISTS idx_br_source ON business_relations(source_type, source_id)", &[]);
    let _ = database::run("CREATE INDEX IF NOT EXISTS idx_br_target ON business_relations(target_type, target_id)", &[]);
    let _ = database::run("CREATE INDEX IF NOT EXISTS idx_br_type ON business_relations(relation_type)", &[]);
    let _ = database::run("CREATE INDEX IF NOT EXISTS idx_br_updated ON business_relations(updated_at)", &[]);
    // Backfill existing business relations
    backfill();
}

/// One-time backfill: sync all existing business layer relations into business_relations table
fn backfill() {
    let count: i64 = database::get("SELECT COUNT(*) as cnt FROM business_relations", &[])
        .ok().flatten().and_then(|r| r["cnt"].as_i64()).unwrap_or(0);
    if count > 0 {
        eprintln!("[business_relations] 已有 {} 条记录，跳过 backfill", count);
        return;
    }
    eprintln!("[business_relations] 开始 backfill 业务关联...");
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

    // 5. channel_partners → belongs_to (CHANNEL → PROJECT)
    if let Ok(rows) = database::all(
        "SELECT id, project_id FROM channel_partners WHERE project_id IS NOT NULL AND project_id != ''",
        &[]
    ) {
        for r in &rows {
            let cid = r["id"].as_str().unwrap_or("");
            let pid = r["project_id"].as_str().unwrap_or("");
            if cid.is_empty() || pid.is_empty() { continue; }
            upsert("CHANNEL", cid, "PROJECT", pid, "belongs_to", "{}");
            total += 1;
        }
    }

    // 6. vendor_partners → belongs_to (VENDOR → PROJECT)
    if let Ok(rows) = database::all(
        "SELECT id, project_id FROM vendor_partners WHERE project_id IS NOT NULL AND project_id != ''",
        &[]
    ) {
        for r in &rows {
            let vid = r["id"].as_str().unwrap_or("");
            let pid = r["project_id"].as_str().unwrap_or("");
            if vid.is_empty() || pid.is_empty() { continue; }
            upsert("VENDOR", vid, "PROJECT", pid, "belongs_to", "{}");
            total += 1;
        }
    }

    // 7. channel_contacts → employed_by (CONTACT → CHANNEL)
    if let Ok(rows) = database::all(
        "SELECT id, channel_id FROM channel_contacts WHERE channel_id IS NOT NULL AND channel_id != ''",
        &[]
    ) {
        for r in &rows {
            let cid = r["id"].as_str().unwrap_or("");
            let chid = r["channel_id"].as_str().unwrap_or("");
            if cid.is_empty() || chid.is_empty() { continue; }
            upsert("CONTACT", cid, "CHANNEL", chid, "employed_by", "{}");
            total += 1;
        }
    }

    // 8. vendor_contacts → employed_by (CONTACT → VENDOR)
    if let Ok(rows) = database::all(
        "SELECT id, vendor_id FROM vendor_contacts WHERE vendor_id IS NOT NULL AND vendor_id != ''",
        &[]
    ) {
        for r in &rows {
            let cid = r["id"].as_str().unwrap_or("");
            let vid = r["vendor_id"].as_str().unwrap_or("");
            if cid.is_empty() || vid.is_empty() { continue; }
            upsert("CONTACT", cid, "VENDOR", vid, "employed_by", "{}");
            total += 1;
        }
    }

    // 9. customer_contacts → employed_by (CONTACT → CUSTOMER)
    if let Ok(rows) = database::all(
        "SELECT id, customer_id FROM customer_contacts WHERE customer_id IS NOT NULL AND customer_id != ''",
        &[]
    ) {
        for r in &rows {
            let cid = r["id"].as_str().unwrap_or("");
            let cuid = r["customer_id"].as_str().unwrap_or("");
            if cid.is_empty() || cuid.is_empty() { continue; }
            upsert("CONTACT", cid, "CUSTOMER", cuid, "employed_by", "{}");
            total += 1;
        }
    }

    eprintln!("[business_relations] Backfill 完成，共同步 {} 条关联", total);
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
    if let Ok(c) = changes {
        if c.changes > 0 {
            eprintln!("[business_relations] 更新: {}[{}] → {}[{}] ({})", source_type, source_id, target_type, target_id, relation_type);
            return;
        }
    }
    // Insert new
    let id = uuid::Uuid::new_v4().to_string();
    let _ = database::run(
        "INSERT INTO business_relations(id, source_type, source_id, target_type, target_id, relation_type, metadata, created_at, updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,datetime('now','localtime'),datetime('now','localtime'))",
        &[json!(id), json!(source_type), json!(source_id), json!(target_type), json!(target_id), json!(relation_type), json!(metadata)],
    );
    eprintln!("[business_relations] 新增: {}[{}] → {}[{}] ({})", source_type, source_id, target_type, target_id, relation_type);
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
    eprintln!("[business_relations] 删除: {}[{}] → {}[{}] ({})", source_type, source_id, target_type, target_id, relation_type);
}

/// Delete all relations where an entity is the source
pub fn delete_by_source(source_type: &str, source_id: &str) {
    let _ = database::run(
        "DELETE FROM business_relations WHERE source_type=? AND source_id=?",
        &[json!(source_type), json!(source_id)],
    );
    eprintln!("[business_relations] 删除所有源关联: {}[{}]", source_type, source_id);
}

/// Delete all relations where an entity is the target
pub fn delete_by_target(target_type: &str, target_id: &str) {
    let _ = database::run(
        "DELETE FROM business_relations WHERE target_type=? AND target_id=?",
        &[json!(target_type), json!(target_id)],
    );
}

/// Get all relations for an entity (as source or target)
pub fn get_relations_for_entity(entity_type: &str, entity_id: &str) -> Vec<Value> {
    let as_source = database::all(
        "SELECT target_type, target_id, relation_type, metadata, updated_at FROM business_relations WHERE source_type=? AND source_id=?",
        &[json!(entity_type), json!(entity_id)],
    ).unwrap_or_default();
    let as_target = database::all(
        "SELECT source_type as target_type, source_id as target_id, relation_type, metadata, updated_at FROM business_relations WHERE target_type=? AND target_id=?",
        &[json!(entity_type), json!(entity_id)],
    ).unwrap_or_default();
    let mut result = as_source;
    result.extend(as_target);
    result
}

/// Get relations changed since a timestamp (for knowledge pipeline)
pub fn get_changed_since(since: &str, limit: usize) -> Vec<Value> {
    database::all(
        "SELECT source_type, source_id, target_type, target_id, relation_type, metadata, updated_at FROM business_relations WHERE updated_at > ?1 ORDER BY updated_at ASC LIMIT ?2",
        &[json!(since), json!(limit as i64)],
    ).unwrap_or_default()
}

/// Format relations for an entity as a display string for LLM output.
/// Returns empty string if no relations.
pub fn format_relations(entity_type: &str, entity_id: &str) -> String {
    let rels = get_relations_for_entity(entity_type, entity_id);
    if rels.is_empty() { return String::new(); }
    let mut lines: Vec<String> = Vec::new();
    for r in &rels {
        let target_type = r["target_type"].as_str().unwrap_or("");
        let target_id = r["target_id"].as_str().unwrap_or("");
        let rel_type = r["relation_type"].as_str().unwrap_or("");
        let target_name = resolve_entity_name(target_type, target_id);
        // 返回 UUID 供 LLM 使用
        lines.push(format!("{} [{}] {} (ID: {}-{})", rel_type, target_type, target_name, target_type, target_id));
    }
    format!("  📎 关联: {}", lines.join(", "))
}

/// Resolve a human-readable name for an entity
fn resolve_entity_name(entity_type: &str, entity_id: &str) -> String {
    let sql = match entity_type {
        "PROJECT" => Some("SELECT name FROM projects WHERE id=?"),
        "PRODUCT" => Some("SELECT name FROM products WHERE id=?"),
        "NOTE" => Some("SELECT title FROM notes WHERE note_id=?"),
        "TODO" => Some("SELECT title FROM todos WHERE id=?"),
        "FILE" => Some("SELECT custom_name FROM file_relations WHERE file_id=?1 ORDER BY id LIMIT 1"),
        "CHANNEL" => Some("SELECT name FROM partners WHERE id=? AND type='CHANNEL'"),
        "VENDOR" => Some("SELECT name FROM partners WHERE id=? AND type='VENDOR'"),
        "CUSTOMER" => Some("SELECT name FROM partners WHERE id=? AND type='CUSTOMER'"),
        _ => None,
    };
    if let Some(s) = sql {
        if let Ok(Some(r)) = database::get(s, &[json!(entity_id)]) {
            for key in &["name", "title", "custom_name"] {
                if let Some(v) = r[*key].as_str().filter(|v| !v.is_empty()) {
                    return v.to_string();
                }
            }
        }
    }
    entity_id.to_string()
}
