use crate::database;
use serde_json::{json, Value};

/// 搜索所有对象（source_data），类似 Object 工具但返回 JSON
#[tauri::command]
pub async fn kb_memory_objects(query: String) -> Result<Value, String> {
    let q = query.trim().to_string();
    if q.is_empty() { return Ok(json!({"success":true,"objects":[]})); }
    let like = format!("%{}%", crate::database::escape_like(&q));
    // FTS5 try first（表名修正：source_data_fts 从不存在，正确的镜像表是 global_search_fts）, fallback to LIKE
    let fts_q = q.split_whitespace().map(|k| format!("\"{}\"", k.replace('"',""))).collect::<Vec<_>>().join(" OR ");
    let mut objects: Vec<Value> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    if let Ok(rows) = database::all(
        "SELECT rowid, source_type, source_id, title FROM global_search_fts WHERE global_search_fts MATCH ?1 LIMIT 30",
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
            "SELECT source_type, source_id, title FROM source_data WHERE title LIKE ?1 ESCAPE '\\' OR content LIKE ?1 ESCAPE '\\' LIMIT 20",
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
    if let Ok(rows) = database::all("SELECT id, name FROM projects WHERE name LIKE ?1 ESCAPE '\\' LIMIT 10", &[json!(&like)]) {
        for r in &rows {
            let id = r["id"].as_str().unwrap_or("").to_string();
            let name = r["name"].as_str().unwrap_or("").to_string();
            if seen.insert(format!("PROJECT-{}", id)) {
                objects.push(json!({"source_type":"PROJECT","source_id":id,"title":name}));
            }
        }
    }
    // products
    if let Ok(rows) = database::all("SELECT id, name FROM products WHERE name LIKE ?1 ESCAPE '\\' LIMIT 10", &[json!(&like)]) {
        for r in &rows {
            let id = r["id"].as_str().unwrap_or("").to_string();
            let name = r["name"].as_str().unwrap_or("").to_string();
            if seen.insert(format!("PRODUCT-{}", id)) {
                objects.push(json!({"source_type":"PRODUCT","source_id":id,"title":name}));
            }
        }
    }
    // notes
    if let Ok(rows) = database::all("SELECT note_id, title FROM notes WHERE title LIKE ?1 ESCAPE '\\' AND deleted_at IS NULL LIMIT 10", &[json!(&like)]) {
        for r in &rows {
            let id = r["note_id"].as_str().unwrap_or("").to_string();
            let name = r["title"].as_str().unwrap_or("").to_string();
            if seen.insert(format!("NOTE-{}", id)) {
                objects.push(json!({"source_type":"NOTE","source_id":id,"title":name}));
            }
        }
    }
    // file_relations
    if let Ok(rows) = database::all("SELECT file_id, custom_name FROM file_relations WHERE custom_name LIKE ?1 ESCAPE '\\' LIMIT 10", &[json!(&like)]) {
        for r in &rows {
            let id = r["file_id"].as_str().unwrap_or("").to_string();
            let name = r["custom_name"].as_str().unwrap_or("").to_string();
            if seen.insert(format!("FILE-{}", id)) {
                objects.push(json!({"source_type":"FILE","source_id":id,"title":name}));
            }
        }
    }
    // todos (未完成)
    if let Ok(rows) = database::all("SELECT id, title FROM todos WHERE title LIKE ?1 ESCAPE '\\' AND done=0 LIMIT 10", &[json!(&like)]) {
        for r in &rows {
            let id = r["id"].as_str().unwrap_or("").to_string();
            let name = r["title"].as_str().unwrap_or("").to_string();
            if seen.insert(format!("TODO-{}", id)) {
                objects.push(json!({"source_type":"TODO","source_id":id,"title":name}));
            }
        }
    }
    // partners (渠道商/供应商/客户)
    if let Ok(rows) = database::all("SELECT id, name, type FROM partners WHERE name LIKE ?1 ESCAPE '\\' LIMIT 10", &[json!(&like)]) {
        for r in &rows {
            let id = r["id"].as_str().unwrap_or("").to_string();
            let name = r["name"].as_str().unwrap_or("").to_string();
            let ptype = r["type"].as_str().unwrap_or("").to_string();
            let source_type = match ptype.as_str() {
                "CHANNEL" => "CHANNEL",
                "VENDOR" => "VENDOR",
                "CUSTOMER" => "CUSTOMER",
                _ => continue,
            };
            if seen.insert(format!("{}-{}", source_type, id)) {
                objects.push(json!({"source_type":source_type,"source_id":id,"title":name}));
            }
        }
    }
    // partner_contacts (联系人)
    if let Ok(rows) = database::all("SELECT id, name FROM partner_contacts WHERE name LIKE ?1 ESCAPE '\\' LIMIT 10", &[json!(&like)]) {
        for r in &rows {
            let id = r["id"].as_str().unwrap_or("").to_string();
            let name = r["name"].as_str().unwrap_or("").to_string();
            if seen.insert(format!("CONTACT-{}", id)) {
                objects.push(json!({"source_type":"CONTACT","source_id":id,"title":name}));
            }
        }
    }
    // emails (邮件，搜索主题和正文)
    if let Ok(rows) = database::all("SELECT id, subject FROM emails WHERE (subject LIKE ?1 ESCAPE '\\' OR body_text LIKE ?1 ESCAPE '\\') LIMIT 10", &[json!(&like)]) {
        for r in &rows {
            let id = r["id"].as_str().unwrap_or("").to_string();
            let name = r["subject"].as_str().unwrap_or("").to_string();
            if seen.insert(format!("EMAIL-{}", id)) {
                objects.push(json!({"source_type":"EMAIL","source_id":id,"title":name}));
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
            "CHANNEL" => database::get("SELECT name FROM partners WHERE id=?1 AND type='CHANNEL'", &[json!(sid)])
                .ok().flatten().and_then(|r| r["name"].as_str().map(String::from)).unwrap_or_default(),
            "VENDOR" => database::get("SELECT name FROM partners WHERE id=?1 AND type='VENDOR'", &[json!(sid)])
                .ok().flatten().and_then(|r| r["name"].as_str().map(String::from)).unwrap_or_default(),
            "CUSTOMER" => database::get("SELECT name FROM partners WHERE id=?1 AND type='CUSTOMER'", &[json!(sid)])
                .ok().flatten().and_then(|r| r["name"].as_str().map(String::from)).unwrap_or_default(),
            "CONTACT" => database::get("SELECT name FROM partner_contacts WHERE id=?1", &[json!(sid)])
                .ok().flatten().and_then(|r| r["name"].as_str().map(String::from)).unwrap_or_default(),
            "EMAIL" => database::get("SELECT subject FROM emails WHERE id=?1", &[json!(sid)])
                .ok().flatten().and_then(|r| r["subject"].as_str().map(String::from)).unwrap_or_default(),
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
        let content = p["content"].as_str().unwrap_or("").to_string();
        let doc_title = p["doc_title"].as_str().unwrap_or("").to_string();

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
            let created = r["created_ts"].as_i64().unwrap_or(now);
            let effective = if lat > 0 { base * (-GAMMA * (now - lat) as f64).exp() } else { base * (-GAMMA * (now - created) as f64).exp() };
            let pct = (effective / MAX_WEIGHT * 100.0).clamp(0.0, 100.0);
            let idx = (pct / 5.0).round() as usize;
            let idx = idx.min(20);
            buckets[idx] += 1;
            child_total += 1;
            effective_sum += effective;
            if effective >= 2.0 { hot_count += 1; }
            // 冷块：被访问过=90 天未再访问；从未被访问=创建即冷（原口径漏计后者，新块永不算冷）
            if (lat > 0 && lat < now - 90 * 86400) || (lat <= 0 && created < now - 90 * 86400) { cold_count += 1; }
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

    // （已移除 activity_heatmap：last_access_at 为 INTEGER unix 秒，与 date() 文本比较恒 false，
    //   查询结果永远不完整且前端无消费方——活动数据可视化另见遗忘曲线/增长趋势）

    Ok(json!({
        "success": true,
        "by_type": by_type,
        "weight_dist": weight_dist,
        "daily_growth": daily_growth,
        "object_counts": { "total": object_total, "by_type": object_by_type },
        "health": {
            "total": child_total,
            "hot_count": hot_count,
            "cold_count": cold_count,
            "avg_weight": avg_weight,
            "hot_ratio": if child_total > 0 { (hot_count as f64 / child_total as f64 * 100.0).round() } else { 0.0 },
            "cold_ratio": if child_total > 0 { (cold_count as f64 / child_total as f64 * 100.0).round() } else { 0.0 },
        }
    }))
}

// ═══════════════════════════════════════════════════════════════════
// 对象关系图谱（知识观测台）
// ═══════════════════════════════════════════════════════════════════

/// 节点/边上限：canvas 力导图性能足够，上限主要守护可读性与前端载荷体积；按度数截断保枢纽优先
const GRAPH_MAX_NODES: usize = 800;
const GRAPH_MAX_EDGES: usize = 2000;

/// 图谱数据：节点=有关系的对象（孤立对象过滤——图谱只表达关联），边=business_relations。
/// 端点解析双键 (type,source_id)/(type,id)；解析不到的脏边行（如 source_type 存成 UUID）自然丢弃。
#[tauri::command]
pub async fn kb_graph_data() -> Result<Value, String> {
    let node_rows = database::all(
        "SELECT id, source_type AS type, source_id AS biz_id, COALESCE(title,'') AS title FROM source_data", &[],
    ).map_err(|e| e.to_string())?;
    let edge_rows = database::all(
        "SELECT source_type AS st, source_id AS si, target_type AS tt, target_id AS ti, COALESCE(relation_type,'') AS rel FROM business_relations", &[],
    ).map_err(|e| e.to_string())?;
    Ok(build_graph(node_rows, edge_rows, GRAPH_MAX_NODES, GRAPH_MAX_EDGES))
}

/// 建图纯函数（可单测）：解析边端点→度数累计→过滤孤立→按度截断→输出 nodes/links
pub(crate) fn build_graph(node_rows: Vec<Value>, edge_rows: Vec<Value>, cap_nodes: usize, cap_edges: usize) -> Value {
    use std::collections::{HashMap, HashSet};
    let mut key_to_id: HashMap<String, String> = HashMap::new();
    let mut meta: HashMap<String, &Value> = HashMap::new();
    for n in &node_rows {
        let id = n["id"].as_str().unwrap_or("").to_string();
        if id.is_empty() { continue; }
        let ty = n["type"].as_str().unwrap_or("");
        key_to_id.insert(format!("{}:{}", ty, id), id.clone());
        if let Some(biz) = n["biz_id"].as_str() {
            key_to_id.insert(format!("{}:{}", ty, biz), id.clone());
        }
        meta.insert(id, n);
    }
    let resolve = |ty: &str, key: &str| -> Option<String> {
        key_to_id.get(&format!("{}:{}", ty, key)).cloned()
    };

    // 边：两端可解析、非自环；无向去重（同对同关系只留一条）
    let mut seen_edge: HashSet<String> = HashSet::new();
    let mut edges: Vec<(String, String, String)> = Vec::new();
    let mut degree: HashMap<String, usize> = HashMap::new();
    for e in &edge_rows {
        let st = e["st"].as_str().unwrap_or("");
        let si = e["si"].as_str().unwrap_or("");
        let tt = e["tt"].as_str().unwrap_or("");
        let ti = e["ti"].as_str().unwrap_or("");
        let (Some(a), Some(b)) = (resolve(st, si), resolve(tt, ti)) else { continue };
        if a == b { continue; }
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        let rel = e["rel"].as_str().unwrap_or("").to_string();
        if !seen_edge.insert(format!("{}|{}|{}", lo, hi, rel)) { continue; }
        *degree.entry(lo.clone()).or_insert(0) += 1;
        *degree.entry(hi.clone()).or_insert(0) += 1;
        edges.push((lo, hi, rel));
    }

    let object_total = meta.len();
    let connected_total = degree.len();
    // 只保留连通节点，度数降序（同度按 id 保证确定性），超上限截断
    let mut connected: Vec<String> = degree.keys().cloned().collect();
    connected.sort_by(|a, b| degree[b].cmp(&degree[a]).then_with(|| a.cmp(b)));
    let nodes_truncated = connected.len() > cap_nodes;
    connected.truncate(cap_nodes);
    let kept: HashSet<String> = connected.iter().cloned().collect();

    let mut links_all: Vec<(String, String, String)> = edges.into_iter()
        .filter(|(a, b, _)| kept.contains(a.as_str()) && kept.contains(b.as_str()))
        .collect();
    // 边按两端度数和降序截断：枢纽间的主干边优先保留
    links_all.sort_by(|x, y| (degree[&y.0] + degree[&y.1]).cmp(&(degree[&x.0] + degree[&x.1])));
    let edges_truncated = links_all.len() > cap_edges;
    let links: Vec<Value> = links_all.into_iter().take(cap_edges)
        .map(|(a, b, rel)| json!({ "source": a, "target": b, "relation": rel }))
        .collect();

    let nodes: Vec<Value> = connected.iter().map(|id| {
        let m: &Value = meta.get(id).copied().unwrap_or(&Value::Null);
        let title = m["title"].as_str().unwrap_or("");
        json!({
            "id": id,
            "bizId": m["biz_id"].as_str().unwrap_or(id),
            "type": m["type"].as_str().unwrap_or(""),
            "label": if title.is_empty() { id.chars().take(8).collect::<String>() } else { title.to_string() },
            "degree": degree.get(id).copied().unwrap_or(0),
        })
    }).collect();

    json!({
        "success": true,
        "nodes": nodes,
        "links": links,
        "stats": {
            "objectTotal": object_total,
            "connectedTotal": connected_total,
            "orphanFiltered": object_total.saturating_sub(connected_total),
            "nodeCount": nodes.len(),
            "edgeCount": links.len(),
            "nodesTruncated": nodes_truncated,
            "edgesTruncated": edges_truncated,
        }
    })
}

#[cfg(test)]
mod graph_tests {
    use super::build_graph;
    use serde_json::{json, Value};

    fn node(id: &str, ty: &str, title: &str) -> Value {
        json!({ "id": id, "type": ty, "biz_id": id, "title": title })
    }
    fn edge(st: &str, si: &str, tt: &str, ti: &str, rel: &str) -> Value {
        json!({ "st": st, "si": si, "tt": tt, "ti": ti, "rel": rel })
    }

    #[test]
    fn connected_only_and_degree_counts() {
        let nodes = vec![node("a", "PROJECT", "项目A"), node("b", "FILE", "文件B"), node("c", "NOTE", "孤立笔记")];
        let edges = vec![edge("PROJECT", "a", "FILE", "b", "has_file")];
        let g = build_graph(nodes, edges, 800, 2000);
        assert_eq!(g["nodes"].as_array().unwrap().len(), 2, "孤立节点 c 必须被过滤");
        let na = g["nodes"].as_array().unwrap().iter().find(|n| n["id"].as_str() == Some("a")).unwrap();
        assert_eq!(na["degree"].as_i64().unwrap(), 1);
        assert_eq!(g["stats"]["orphanFiltered"].as_i64().unwrap(), 1);
    }

    #[test]
    fn dedup_selfloop_dirty_edge_dropped() {
        let nodes = vec![node("a", "PROJECT", "A"), node("b", "FILE", "B")];
        let edges = vec![
            edge("PROJECT", "a", "FILE", "b", "has_file"),
            edge("FILE", "b", "PROJECT", "a", "has_file"),
            edge("PROJECT", "a", "PROJECT", "a", "self"),
            edge("078fdcff-uuid-as-type", "x", "FILE", "b", "dirty"),
        ];
        let g = build_graph(nodes, edges, 800, 2000);
        assert_eq!(g["links"].as_array().unwrap().len(), 1, "反向重复/自环/解析失败边丢弃");
    }

    #[test]
    fn biz_id_dual_key_resolves() {
        let nodes = vec![
            json!({ "id": "sd-1", "type": "FILE", "biz_id": "asset-1", "title": "T" }),
            node("p", "PROJECT", "P"),
        ];
        let edges = vec![edge("FILE", "asset-1", "PROJECT", "p", "has_file")];
        let g = build_graph(nodes, edges, 800, 2000);
        assert_eq!(g["links"].as_array().unwrap().len(), 1);
        let n = g["nodes"].as_array().unwrap().iter().find(|x| x["id"].as_str() == Some("sd-1")).unwrap();
        assert_eq!(n["bizId"].as_str(), Some("asset-1"));
    }

    #[test]
    fn cap_keeps_hubs_and_drops_dangling_links() {
        let nodes = vec![
            json!({ "id": "h", "type": "PROJECT", "biz_id": "h", "title": "H" }),
            node("x1", "FILE", "X1"),
            node("x2", "FILE", "X2"),
        ];
        let edges = vec![
            edge("PROJECT", "h", "FILE", "x1", "has_file"),
            edge("PROJECT", "h", "FILE", "x2", "has_file"),
        ];
        let g = build_graph(nodes, edges, 1, 1000);
        assert_eq!(g["nodes"].as_array().unwrap().len(), 1, "cap=1 留度数最高的枢纽");
        assert_eq!(g["nodes"][0]["id"].as_str(), Some("h"));
        assert!(g["links"].as_array().unwrap().is_empty(), "端点被截断的边丢弃");
        assert_eq!(g["stats"]["nodesTruncated"].as_bool(), Some(true));
    }

    #[test]
    fn empty_edge_cap_zero_keeps_all_nodes_no_links() {
        let nodes = vec![node("a", "PROJECT", "A"), node("b", "FILE", "B")];
        let edges = vec![edge("PROJECT", "a", "FILE", "b", "has_file")];
        let g = build_graph(nodes, edges, 800, 0);
        assert!(g["links"].as_array().unwrap().is_empty());
        assert_eq!(g["nodes"].as_array().unwrap().len(), 2);
    }
}
