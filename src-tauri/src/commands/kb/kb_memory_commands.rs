use crate::database;
use serde_json::{json, Value};

/// 搜索所有对象（source_data），类似 Object 工具但返回 JSON
#[tauri::command]
pub async fn kb_memory_objects(query: String) -> Result<Value, String> {
    let q = query.trim().to_string();
    if q.is_empty() { return Ok(json!({"success":true,"objects":[]})); }
    let like = format!("%{}%", crate::database::escape_like(&q));
    // FTS5 try first, fallback to LIKE
    let fts_q = q.split_whitespace().map(|k| format!("\"{}\"", k.replace('"',""))).collect::<Vec<_>>().join(" OR ");
    let mut objects: Vec<Value> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    if let Ok(rows) = database::all(
        "SELECT rowid, source_type, source_id, title FROM source_data_fts WHERE source_data_fts MATCH ?1 LIMIT 30",
        &[json!(&fts_q)],
    ) {
        for r in &rows {
            let st = r["source_type"].as_str().unwrap_or("").to_string();
            let sid = r["source_id"].as_str().unwrap_or("").to_string();
            let key = format!("{}-{}", st, sid);
            if !seen.insert(key) { continue; }
            objects.push(json!({"source_type": st, "source_id": sid, "title": r["title"].as_str().unwrap_or("")}));
        }
    }
    if objects.is_empty() {
        if let Ok(rows) = database::all(
            "SELECT source_type, source_id, title FROM source_data WHERE title LIKE ?1 OR content LIKE ?1 LIMIT 20",
            &[json!(&like)],
        ) {
            for r in &rows {
                let st = r["source_type"].as_str().unwrap_or("").to_string();
                let sid = r["source_id"].as_str().unwrap_or("").to_string();
                let key = format!("{}-{}", st, sid);
                if !seen.insert(key) { continue; }
                objects.push(json!({"source_type": st, "source_id": sid, "title": r["title"].as_str().unwrap_or("")}));
            }
        }
    }
    // ★ 补全文件标题：FILE 类型的 title 在 source_data 中为空，从 file_relations 获取
    for obj in &mut objects {
        if obj["source_type"].as_str() == Some("FILE") {
            let sid = obj["source_id"].as_str().unwrap_or("");
            let name = obj["title"].as_str().unwrap_or("");
            if name.is_empty() {
                if let Ok(Some(row)) = database::get(
                    "SELECT COALESCE(fr.custom_name, fa.file_ext, fa.id) as name \
                     FROM file_assets fa LEFT JOIN file_relations fr ON fr.file_id=fa.id \
                     WHERE fa.id=? ORDER BY COALESCE(fr.custom_name,'') DESC LIMIT 1",
                    &[json!(sid)],
                ) {
                    obj["title"] = row["name"].clone();
                }
            }
        }
    }
    // ★ 补全 NOTE/TODO 的项目关联信息
    for obj in &mut objects {
        let st = obj["source_type"].as_str().unwrap_or("");
        let sid = obj["source_id"].as_str().unwrap_or("");
        let (table, id_col) = match st {
            "NOTE" => ("note_project_mapping", "note_id"),
            "TODO" => ("todo_project_mapping", "todo_id"),
            _ => continue,
        };
        if let Ok(Some(row)) = database::get(
            &format!("SELECT p.id as project_id, p.name as project_name FROM {} npm JOIN projects p ON p.id=npm.project_id WHERE npm.{} = ? LIMIT 1", table, id_col),
            &[json!(sid)],
        ) {
            obj["project_id"] = row["project_id"].clone();
            obj["project_name"] = row["project_name"].clone();
        }
    }
    // ★ 过滤已完成的待办（不在全局搜索中出现）
    objects.retain(|obj| {
        if obj["source_type"].as_str() != Some("TODO") { return true }
        let sid = obj["source_id"].as_str().unwrap_or("");
        let done: bool = database::get("SELECT done FROM todos WHERE id=?", &[json!(sid)])
            .ok().flatten()
            .and_then(|r| r["done"].as_i64())
            .map(|d| d != 0)
            .unwrap_or(false);
        !done
    });
    // ★ 过滤已关联项目的笔记/待办（应通过项目搜索定位）
    objects.retain(|obj| {
        let st = obj["source_type"].as_str().unwrap_or("");
        if st != "NOTE" && st != "TODO" { return true }
        obj["project_id"].is_null() || obj["project_id"].as_str().map_or(true, |s| s.is_empty())
    });
    Ok(json!({"success":true,"objects":objects}))
}

/// 业务表检索：按名称模糊匹配 projects/products/notes/file_relations
#[tauri::command]
pub async fn search_business(query: String) -> Result<Value, String> {
    let q = query.trim().to_string();
    if q.is_empty() { return Ok(json!({"success":true,"objects":[]})); }
    let like = format!("%{}%", crate::database::escape_like(&q));
    let mut objects: Vec<Value> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    // projects
    if let Ok(rows) = database::all("SELECT id, name FROM projects WHERE name LIKE ?1 LIMIT 10", &[json!(&like)]) {
        for r in &rows {
            let id = r["id"].as_str().unwrap_or("").to_string();
            let name = r["name"].as_str().unwrap_or("").to_string();
            if seen.insert(format!("PROJECT-{}", id)) {
                objects.push(json!({"source_type":"PROJECT","source_id":id,"title":name}));
            }
        }
    }
    // products
    if let Ok(rows) = database::all("SELECT id, name FROM products WHERE name LIKE ?1 LIMIT 10", &[json!(&like)]) {
        for r in &rows {
            let id = r["id"].as_str().unwrap_or("").to_string();
            let name = r["name"].as_str().unwrap_or("").to_string();
            if seen.insert(format!("PRODUCT-{}", id)) {
                objects.push(json!({"source_type":"PRODUCT","source_id":id,"title":name}));
            }
        }
    }
    // notes
    if let Ok(rows) = database::all("SELECT note_id, title FROM notes WHERE title LIKE ?1 AND deleted_at IS NULL LIMIT 10", &[json!(&like)]) {
        for r in &rows {
            let id = r["note_id"].as_str().unwrap_or("").to_string();
            let name = r["title"].as_str().unwrap_or("").to_string();
            if seen.insert(format!("NOTE-{}", id)) {
                objects.push(json!({"source_type":"NOTE","source_id":id,"title":name}));
            }
        }
    }
    // file_relations
    if let Ok(rows) = database::all("SELECT file_id, custom_name FROM file_relations WHERE custom_name LIKE ?1 LIMIT 10", &[json!(&like)]) {
        for r in &rows {
            let id = r["file_id"].as_str().unwrap_or("").to_string();
            let name = r["custom_name"].as_str().unwrap_or("").to_string();
            if seen.insert(format!("FILE-{}", id)) {
                objects.push(json!({"source_type":"FILE","source_id":id,"title":name}));
            }
        }
    }
    // todos (未完成)
    if let Ok(rows) = database::all("SELECT id, title FROM todos WHERE title LIKE ?1 AND done=0 LIMIT 10", &[json!(&like)]) {
        for r in &rows {
            let id = r["id"].as_str().unwrap_or("").to_string();
            let name = r["title"].as_str().unwrap_or("").to_string();
            if seen.insert(format!("TODO-{}", id)) {
                objects.push(json!({"source_type":"TODO","source_id":id,"title":name}));
            }
        }
    }
    // partners (渠道/厂商/客户)
    if let Ok(rows) = database::all("SELECT id, type, name FROM partners WHERE name LIKE ?1 LIMIT 10", &[json!(&like)]) {
        for r in &rows {
            let id = r["id"].as_str().unwrap_or("").to_string();
            let name = r["name"].as_str().unwrap_or("").to_string();
            let st = r["type"].as_str().unwrap_or("PARTNER");
            if seen.insert(format!("PARTNER-{}", id)) {
                objects.push(json!({"source_type":"PARTNER","source_id":id,"title":name,"partner_type":st}));
            }
        }
    }
    // partner_contacts (联系人)
    if let Ok(rows) = database::all("SELECT id, name FROM partner_contacts WHERE name LIKE ?1 LIMIT 10", &[json!(&like)]) {
        for r in &rows {
            let id = r["id"].as_str().unwrap_or("").to_string();
            let name = r["name"].as_str().unwrap_or("").to_string();
            if seen.insert(format!("CONTACT-{}", id)) {
                objects.push(json!({"source_type":"CONTACT","source_id":id,"title":name}));
            }
        }
    }
    Ok(json!({"success":true,"objects":objects}))
}

/// source_data FTS5 检索
#[tauri::command]
pub async fn search_source_data(query: String) -> Result<Value, String> {
    let q = query.trim().to_string();
    if q.is_empty() { return Ok(json!({"success":true,"objects":[]})); }
    let fts_q = q.split_whitespace().map(|k| format!("\"{}\"", k.replace('"',""))).collect::<Vec<_>>().join(" OR ");
    let mut objects: Vec<Value> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    // MEMORY 已迁移到独立表，global_search_fts 中不再有 MEMORY
    if let Ok(rows) = database::all(
        "SELECT source_type, source_id FROM global_search_fts WHERE global_search_fts MATCH ?1 ORDER BY bm25(global_search_fts, 1.0, 1.0) LIMIT 30",
        &[json!(&fts_q)],
    ) {
        for r in &rows {
            let st = r["source_type"].as_str().unwrap_or("").to_string();
            let sid = r["source_id"].as_str().unwrap_or("").to_string();
            let key = format!("{}-{}", st, sid);
            if seen.insert(key) {
                objects.push(json!({"source_type": st, "source_id": sid}));
            }
        }
    }
    Ok(json!({"success":true,"objects":objects}))
}

/// 根据 type+id 批量查询业务表名称
#[tauri::command]
pub async fn lookup_object_names(ids: Vec<Value>) -> Result<Value, String> {
    // ids: [{"source_type":"PROJECT","source_id":"xxx"}, ...]
    let mut result: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for item in &ids {
        let st = item["source_type"].as_str().unwrap_or("");
        let sid = item["source_id"].as_str().unwrap_or("");
        if st.is_empty() || sid.is_empty() { continue; }
        let key = format!("{}-{}", st, sid);
        let name = match st {
            "PROJECT" => database::get("SELECT name FROM projects WHERE id=?1", &[json!(sid)])
                .ok().flatten().and_then(|r| r["name"].as_str().map(String::from)).unwrap_or_default(),
            "PRODUCT" => database::get("SELECT name FROM products WHERE id=?1", &[json!(sid)])
                .ok().flatten().and_then(|r| r["name"].as_str().map(String::from)).unwrap_or_default(),
            "NOTE" => database::get("SELECT title FROM notes WHERE note_id=?1 AND deleted_at IS NULL", &[json!(sid)])
                .ok().flatten().and_then(|r| r["title"].as_str().map(String::from)).unwrap_or_default(),
            "FILE" => database::get("SELECT custom_name FROM file_relations WHERE file_id=?1 AND custom_name != '' LIMIT 1", &[json!(sid)])
                .ok().flatten().and_then(|r| r["custom_name"].as_str().map(String::from))
                .or_else(|| database::get("SELECT original_name FROM file_assets WHERE id=?1", &[json!(sid)])
                    .ok().flatten().and_then(|r| r["original_name"].as_str().map(String::from)))
                .unwrap_or_default(),
            "TODO" => database::get("SELECT title FROM todos WHERE id=?1", &[json!(sid)])
                .ok().flatten().and_then(|r| r["title"].as_str().map(String::from)).unwrap_or_default(),
            _ => String::new(),
        };
        if !name.is_empty() {
            result.insert(key, name);
        }
    }
    Ok(json!({"names": result}))
}

/// 获取所有关联了项目的笔记 ID 集合
#[tauri::command]
pub async fn kb_project_note_ids() -> Result<Value, String> {
    let rows = database::all("SELECT note_id FROM note_project_mapping", &[]).unwrap_or_default();
    let ids: Vec<String> = rows.iter().filter_map(|r| r["note_id"].as_str().map(|s| s.to_string())).collect();
    Ok(json!({"ids": ids}))
}

/// 获取指定对象的父块列表（含突触权重、实体、标签）
#[tauri::command]
pub async fn kb_memory_parents(source_type: String, source_id: String) -> Result<Value, String> {
    let _sid = format!("{}-{}", source_type, source_id);
    let parents = database::all(
        "SELECT kc.rowid, kc.chunk_id, kc.content, kc.doc_title, kc.chunk_index, kc.created_at, kc.base_weight, kc.last_access_at, kc.access_count, strftime('%s', kc.created_at) as created_ts \
         FROM knowledge_chunks kc WHERE kc.chunk_type='parent' \
         AND kc.source_data_id = (SELECT id FROM source_data WHERE source_type=?1 AND source_id=?2) \
         ORDER BY kc.chunk_index ASC",
        &[json!(source_type), json!(source_id)],
    ).unwrap_or_default();

    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
    const GAMMA: f64 = 0.69314718056 / (30.0 * 86400.0);

    let mut result: Vec<Value> = Vec::new();
    let mut seen_parent_ids = std::collections::HashSet::new();
    for p in &parents {
        let pid = p["chunk_id"].as_str().unwrap_or("").to_string();
        if pid.is_empty() || !seen_parent_ids.insert(pid.clone()) { continue; }
        let base = p["base_weight"].as_f64().unwrap_or(1.0);
        let lat = p["last_access_at"].as_i64().unwrap_or(0);
        let access_count = p["access_count"].as_i64().unwrap_or(0);
        let effective = if lat > 0 { base * (-GAMMA * ((now as i64 - lat) as f64)).exp() } else { let created = p["created_ts"].as_i64().unwrap_or(now as i64); base * (-GAMMA * ((now as i64 - created) as f64)).exp() };
        let content = p["content"].as_str().unwrap_or("").chars().take(300).collect::<String>();
        let doc_title = p["doc_title"].as_str().unwrap_or("").to_string();

        // Tags
        let tags: Vec<String> = database::all(
            "SELECT DISTINCT t.name FROM tags t JOIN tag_mappings tm ON tm.tag_id=t.tag_id WHERE tm.entity_type=?1 AND tm.entity_id=?2 LIMIT 10",
            &[json!(source_type.to_lowercase() + "s"), json!(&source_id)],
        ).unwrap_or_default().iter().filter_map(|r| r["name"].as_str().map(String::from)).collect();

        // Child count
        let child_count: i64 = database::with_db(|c| c.query_row(
            "SELECT COUNT(*) FROM knowledge_chunks WHERE chunk_type='child' AND parent_id=?1",
            [&pid], |r| r.get(0),
        )).unwrap_or(0);

        let stars = if effective >= 4.0 { 5 } else if effective >= 3.0 { 4 } else if effective >= 2.0 { 3 } else if effective >= 1.0 { 2 } else { 1 };

        result.push(json!({
            "parent_id": pid,
            "chunk_index": p["chunk_index"].as_i64().unwrap_or(0),
            "doc_title": doc_title,
            "content_preview": content,
            "base_weight": base,
            "effective_weight": (effective * 100.0).round() / 100.0,
            "access_count": access_count,
            "child_count": child_count,
            "stars": stars,
            "tags": tags,
            "created_at": p["created_at"].as_str().unwrap_or(""),
        }));
    }
    Ok(json!({"success":true,"parents":result}))
}

/// 热点/冷块统计
#[tauri::command]
pub async fn kb_memory_heatmap() -> Result<Value, String> {
    let total: i64 = database::with_db(|c| c.query_row("SELECT COUNT(*) FROM knowledge_chunks WHERE chunk_type='child'",[],|r|r.get(0))).unwrap_or(0);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0) as i64;
    const GAMMA: f64 = 0.69314718056 / (30.0 * 86400.0);

    // Hot parents: top 10 by effective_weight
    let mut hot: Vec<Value> = Vec::new();
    if let Ok(rows) = database::all(
        "SELECT kc.rowid, kc.chunk_id, kc.doc_title, kc.base_weight, kc.last_access_at, kc.access_count, kc.source_data_id, sd.source_type, sd.source_id, sd.title as source_title, strftime('%s', kc.created_at) as created_ts \
         FROM knowledge_chunks kc LEFT JOIN source_data sd ON sd.id=kc.source_data_id \
         WHERE kc.chunk_type='child' AND kc.base_weight > 0 ORDER BY kc.base_weight DESC LIMIT 10",
        &[],
    ) {
        for r in &rows {
            let base = r["base_weight"].as_f64().unwrap_or(1.0);
            let lat = r["last_access_at"].as_i64().unwrap_or(0);
            let effective = if lat > 0 { base * (-GAMMA * (now - lat) as f64).exp() } else { let created = r["created_ts"].as_i64().unwrap_or(now); base * (-GAMMA * (now - created) as f64).exp() };
            if effective < 0.5 { continue; }
            hot.push(json!({
                "parent_id": r["chunk_id"].as_str().unwrap_or("").to_string(),
                "title": r["doc_title"].as_str().filter(|s|!s.is_empty()).or_else(|| r["source_title"].as_str()).unwrap_or(""),
                "effective_weight": (effective * 100.0).round() / 100.0,
                "access_count": r["access_count"].as_i64().unwrap_or(0),
                "source_type": r["source_type"].as_str().unwrap_or(""),
                "source_id": r["source_id"].as_str().unwrap_or(""),
            }));
        }
    }

    // Cold parents: top 10 by effective_weight (lowest)
    let mut cold: Vec<Value> = Vec::new();
    let threshold_ts = now - 90 * 86400; // 90 days ago
    if let Ok(rows) = database::all(
        "SELECT kc.rowid, kc.chunk_id, kc.doc_title, kc.base_weight, kc.last_access_at, kc.access_count, kc.source_data_id, sd.source_type, sd.source_id, sd.title as source_title, kc.created_at \
         FROM knowledge_chunks kc LEFT JOIN source_data sd ON sd.id=kc.source_data_id \
         WHERE kc.chunk_type='child' AND COALESCE(kc.last_access_at, 0) < ?1 \
         ORDER BY COALESCE(kc.last_access_at, 0) ASC LIMIT 10",
        &[json!(threshold_ts)],
    ) {
        for r in &rows {
            cold.push(json!({
                "parent_id": r["chunk_id"].as_str().unwrap_or("").to_string(),
                "title": r["doc_title"].as_str().filter(|s|!s.is_empty()).or_else(|| r["source_title"].as_str()).unwrap_or(""),
                "last_access": r["last_access_at"].as_i64().unwrap_or(0),
                "access_count": r["access_count"].as_i64().unwrap_or(0),
                "source_type": r["source_type"].as_str().unwrap_or(""),
                "source_id": r["source_id"].as_str().unwrap_or(""),
            }));
        }
    }

    Ok(json!({"success":true,"total_chunks":total,"hot":hot,"cold":cold}))
}

#[tauri::command]
pub async fn kb_memory_dashboard_stats() -> Result<Value, String> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0) as i64;
    const GAMMA: f64 = 0.69314718056 / (30.0 * 86400.0);
    const MAX_WEIGHT: f64 = 5.0;

    // 1. By source type (child chunks)
    let by_type = database::all(
        "SELECT COALESCE(sd.source_type,'未知') as label, COUNT(*) as cnt \
         FROM knowledge_chunks kc LEFT JOIN source_data sd ON sd.id=kc.source_data_id \
         WHERE kc.chunk_type='child' GROUP BY sd.source_type ORDER BY cnt DESC", &[],
    ).unwrap_or_default();

    // 2. Weight distribution (child chunks, effective_weight → percentage buckets 0-100%, step 5%)
    let mut buckets = vec![0i64; 21]; // 0%, 5%, 10%, ..., 100%
    let mut child_total: i64 = 0;
    let mut effective_sum: f64 = 0.0;
    let mut hot_count: i64 = 0;
    let mut cold_count: i64 = 0;
    if let Ok(rows) = database::all(
        "SELECT base_weight, last_access_at, strftime('%s', created_at) as created_ts FROM knowledge_chunks WHERE chunk_type='child'", &[],
    ) {
        for r in &rows {
            let base = r["base_weight"].as_f64().unwrap_or(1.0);
            let lat = r["last_access_at"].as_i64().unwrap_or(0);
            let effective = if lat > 0 { base * (-GAMMA * (now - lat) as f64).exp() } else { let created = r["created_ts"].as_i64().unwrap_or(now); base * (-GAMMA * (now - created) as f64).exp() };
            let pct = (effective / MAX_WEIGHT * 100.0).clamp(0.0, 100.0);
            let idx = (pct / 5.0).round() as usize;
            let idx = idx.min(20);
            buckets[idx] += 1;
            child_total += 1;
            effective_sum += effective;
            if effective >= 2.0 { hot_count += 1; }
            if lat > 0 && lat < now - 90 * 86400 { cold_count += 1; }
        }
    }
    let weight_dist: Vec<Value> = (0..=20).map(|i| {
        json!({"label": format!("{}%", i * 5), "cnt": buckets[i]})
    }).collect();
    let avg_weight = if child_total > 0 { (effective_sum / child_total as f64 * 100.0).round() / 100.0 } else { 0.0 };

    // 3. Daily growth (last 365 days, child chunks — frontend filters by selected range)
    let daily_growth = database::all(
        "SELECT date(created_at) as label, COUNT(*) as cnt \
         FROM knowledge_chunks WHERE chunk_type='child' \
         AND created_at >= date('now', '-365 days', 'localtime') \
         GROUP BY date(created_at) ORDER BY label", &[],
    ).unwrap_or_default();

    // 4. Object counts from source_data
    let object_total: i64 = database::with_db(|c| c.query_row(
        "SELECT COUNT(*) FROM source_data", [], |r| r.get(0)
    )).unwrap_or(0);
    let object_by_type = database::all(
        "SELECT COALESCE(source_type,'未知') as label, COUNT(*) as cnt \
         FROM source_data GROUP BY source_type ORDER BY cnt DESC", &[],
    ).unwrap_or_default();

    // 5. Tag count
    let tag_count: i64 = database::with_db(|c| c.query_row(
        "SELECT COUNT(*) FROM tags WHERE type != 'ai_shadow'", [], |r| r.get(0)
    )).unwrap_or(0);

    // 6. Activity heatmap (last 30 days: updates + accesses per day)
    let activity_heatmap = database::all(
        "SELECT date, SUM(cnt) as cnt FROM ( \
            SELECT date(updated_at) as date, COUNT(*) as cnt FROM source_data \
            WHERE updated_at >= date('now', '-30 days', 'localtime') \
            GROUP BY date(updated_at) \
            UNION ALL \
            SELECT date(last_access_at) as date, COUNT(*) as cnt FROM knowledge_chunks \
            WHERE last_access_at >= date('now', '-30 days', 'localtime') \
            GROUP BY date(last_access_at) \
        ) GROUP BY date ORDER BY date", &[],
    ).unwrap_or_default();

    Ok(json!({
        "success": true,
        "by_type": by_type,
        "weight_dist": weight_dist,
        "daily_growth": daily_growth,
        "activity_heatmap": activity_heatmap,
        "object_counts": { "total": object_total, "by_type": object_by_type },
        "health": {
            "total": child_total,
            "hot_count": hot_count,
            "cold_count": cold_count,
            "avg_weight": avg_weight,
            "tag_count": tag_count,
            "hot_ratio": if child_total > 0 { (hot_count as f64 / child_total as f64 * 100.0).round() } else { 0.0 },
            "cold_ratio": if child_total > 0 { (cold_count as f64 / child_total as f64 * 100.0).round() } else { 0.0 },
        }
    }))
}
