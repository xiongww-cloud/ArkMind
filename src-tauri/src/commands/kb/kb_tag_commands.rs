use crate::database;
use serde_json::{json, Value};

#[tauri::command] pub async fn tag_list(category: Option<String>) -> Result<Vec<Value>, String> {
    if let Some(cat) = category.filter(|s| !s.is_empty()) {
        database::all("SELECT t.tag_id, t.name, t.color, t.type FROM tags t WHERE t.type=?1 ORDER BY t.name",&[json!(cat)]).map_err(|e| e.to_string())
    } else {
        database::all("SELECT t.tag_id, t.name, t.color, t.type FROM tags t ORDER BY t.type, t.name",&[]).map_err(|e| e.to_string())
    }
}

#[tauri::command] pub async fn tag_create(name: String, color: Option<String>, category: String) -> Result<Value, String> {
    let tag_id = uuid::Uuid::new_v4().to_string();
    let palette = ["#6366f1","#06b6d4","#10b981","#f59e0b","#ef4444","#8b5cf6","#ec4899","#14b8a6","#f97316","#84cc16"];
    let actual_color = color.filter(|c| !c.is_empty()).unwrap_or_else(|| {
        use rand::Rng;
        let idx = rand::thread_rng().gen_range(0..palette.len());
        palette[idx].to_string()
    });
    database::run("INSERT INTO tags(tag_id,name,color,type) VALUES(?1,?2,?3,?4)",&[json!(&tag_id), json!(&name), json!(&actual_color), json!(&category)]).map_err(|e| e.to_string())?;
    Ok(json!({"success":true,"tag_id":tag_id,"color":actual_color}))
}

#[tauri::command] pub async fn tag_update(tag_id: String, name: Option<String>, color: Option<String>, category: Option<String>) -> Result<Value, String> {
    let mut fields: Vec<String> = Vec::new(); let mut params: Vec<Value> = Vec::new();
    if let Some(n) = name { fields.push("name=?".into()); params.push(json!(n)); }
    if let Some(c) = color { fields.push("color=?".into()); params.push(json!(c)); }
    if let Some(c) = category { fields.push("type=?".into()); params.push(json!(c)); }
    if fields.is_empty() { return Ok(json!({"success":false,"error":"无更新字段"})); }
    params.push(json!(&tag_id));
    database::run(&format!("UPDATE tags SET {} WHERE tag_id=?", fields.join(",")), &params).map_err(|e| e.to_string())?;
    Ok(json!({"success":true}))
}

#[tauri::command] pub async fn tag_delete(tag_id: String) -> Result<Value, String> {
    database::run("DELETE FROM tag_mappings WHERE tag_id=?",&[json!(&tag_id)]).ok();
    database::run("DELETE FROM tags WHERE tag_id=?",&[json!(&tag_id)]).map_err(|e| e.to_string())?;
    Ok(json!({"success":true}))
}

#[tauri::command] pub async fn tag_list_categories() -> Result<Vec<Value>, String> {
    database::all("SELECT DISTINCT type FROM tags ORDER BY type",&[]).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn tag_stats() -> Result<Value, String> {
    // All tags with usage count
    let tags = database::all(
        "SELECT t.tag_id, t.name, t.color, t.type, \
           COALESCE((SELECT COUNT(*) FROM tag_mappings tm WHERE tm.tag_id=t.tag_id), 0) as usage_count \
         FROM tags t WHERE t.type != 'ai_shadow' \
         ORDER BY usage_count DESC, t.name", &[],
    ).unwrap_or_default();

    let total_tags = tags.len() as i64;
    let used_tags = tags.iter().filter(|t| t["usage_count"].as_i64().unwrap_or(0) > 0).count() as i64;
    let unused_tags = total_tags - used_tags;

    // By category
    let mut by_category: std::collections::HashMap<String, (i64, i64)> = std::collections::HashMap::new();
    for t in &tags {
        let cat = t["type"].as_str().unwrap_or("general").to_string();
        let entry = by_category.entry(cat).or_insert((0, 0));
        entry.0 += 1;
        entry.1 += t["usage_count"].as_i64().unwrap_or(0);
    }
    let category_stats: Vec<Value> = by_category.iter().map(|(k, v)| {
        json!({"label": k, "count": v.0, "total_usage": v.1})
    }).collect();

    // Top 10 by usage
    let top_tags: Vec<Value> = tags.iter().take(10).map(|t| {
        json!({
            "tag_id": t["tag_id"],
            "name": t["name"],
            "color": t["color"],
            "type": t["type"],
            "usage_count": t["usage_count"],
        })
    }).collect();

    // Coverage: objects with at least one tag / total objects
    let tagged_objects: i64 = database::with_db(|c| c.query_row(
        "SELECT COUNT(DISTINCT entity_type||':'||entity_id) FROM tag_mappings \
         WHERE tag_id IN (SELECT tag_id FROM tags WHERE type != 'ai_shadow')", [], |r| r.get(0)
    )).unwrap_or(0);
    let total_objects: i64 = database::with_db(|c| c.query_row(
        "SELECT COUNT(*) FROM source_data", [], |r| r.get(0)
    )).unwrap_or(0);

    Ok(json!({
        "success": true,
        "tags": tags,
        "total_tags": total_tags,
        "used_tags": used_tags,
        "unused_tags": unused_tags,
        "coverage": if total_objects > 0 { (tagged_objects as f64 / total_objects as f64 * 100.0).round() as i64 } else { 0 },
        "by_category": category_stats,
        "top_tags": top_tags,
    }))
}

// ═══════════════════════════════════════════════════════════════════
// Universal entity tag API — works with any entity_type via tag_mappings
// ═══════════════════════════════════════════════════════════════════

#[tauri::command]
pub async fn entity_get_tags(entity_type: String, entity_id: String) -> Result<Value, String> {
    let tags = database::all(
        "SELECT t.* FROM tags t INNER JOIN tag_mappings tm ON tm.tag_id=t.tag_id WHERE tm.entity_type=?1 AND tm.entity_id=?2 ORDER BY t.name",
        &[json!(entity_type), json!(entity_id)],
    ).map_err(|e| e.to_string())?;
    Ok(json!({"success":true,"tags":tags}))
}

#[tauri::command]
pub async fn entity_add_tag(entity_type: String, entity_id: String, tag_name: String) -> Result<Value, String> {
    let tag_id = match database::get("SELECT tag_id FROM tags WHERE name=?1", &[json!(&tag_name)]).ok().flatten() {
        Some(r) => r["tag_id"].as_str().unwrap_or("").to_string(),
        None => {
            let nid = uuid::Uuid::new_v4().to_string();
            database::run("INSERT INTO tags(tag_id,name,color,type) VALUES(?1,?2,?3,'general')", &[json!(&nid), json!(&tag_name), json!("#6366f1")]).map_err(|e| e.to_string())?;
            nid
        }
    };
    if tag_id.is_empty() { return Ok(json!({"success":false,"error":"标签ID无效"})); }
    database::run("INSERT OR IGNORE INTO tag_mappings(entity_type,entity_id,tag_id) VALUES(?1,?2,?3)", &[json!(&entity_type), json!(&entity_id), json!(&tag_id)]).map_err(|e| e.to_string())?;
    Ok(json!({"success":true}))
}

#[tauri::command]
pub async fn entity_remove_tag(entity_type: String, entity_id: String, tag_id: String) -> Result<Value, String> {
    database::run("DELETE FROM tag_mappings WHERE entity_type=?1 AND entity_id=?2 AND tag_id=?3", &[json!(&entity_type), json!(&entity_id), json!(&tag_id)]).map_err(|e| e.to_string())?;
    Ok(json!({"success":true}))
}

/// 调试：返回 knowledge_push_queue 的原始数据
#[tauri::command]
pub async fn kb_pipeline_debug() -> Result<Value, String> {
    let push_queue = database::all("SELECT * FROM knowledge_push_queue ORDER BY id DESC LIMIT 30", &[]).map_err(|e| e.to_string())?;
    let push_pending = database::get("SELECT COUNT(*) as cnt FROM knowledge_push_queue WHERE status='pending'", &[]).map_err(|e| e.to_string())?.and_then(|r| r["cnt"].as_i64()).unwrap_or(0);
    let push_processing = database::get("SELECT COUNT(*) as cnt FROM knowledge_push_queue WHERE status='processing'", &[]).map_err(|e| e.to_string())?.and_then(|r| r["cnt"].as_i64()).unwrap_or(0);
    let push_failed = database::get("SELECT COUNT(*) as cnt FROM knowledge_push_queue WHERE status='failed'", &[]).map_err(|e| e.to_string())?.and_then(|r| r["cnt"].as_i64()).unwrap_or(0);
    Ok(json!({
        "success":true,
        "stats": {"push_pending":push_pending,"push_processing":push_processing,"push_failed":push_failed},
        "push_queue": push_queue
    }))
}
