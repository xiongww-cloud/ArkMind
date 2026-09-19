//! ext_tool_object：目录筛选/统计/聚合 + 关键词/时间线/回看检索 + detail 批量回忆

use crate::database;
use serde_json::{json, Value};
use crate::services::agent_workflow::*;

pub(crate) fn execute_object_catalog(entity: &str, field_filters: &[Value], sort: Option<&Value>, args: &Value) -> String {
    let (table, allowed_fields): (&str, &[&str]) = match entity.to_uppercase().as_str() {
        "PROJECT" => ("projects", &["id","name","description","status","owner","budget","bid_time","created_at","updated_at"]),
        "PRODUCT" => ("(SELECT p.id, p.name, COALESCE(ps.name, '') AS series_name, p.description, p.price, p.full_specs AS params, p.sort_order, p.created_at, p.updated_at FROM products p LEFT JOIN product_series ps ON p.series_id = ps.id)", &["id","name","series_name","description","price","params","sort_order","created_at","updated_at"]),
        "CHANNEL" | "VENDOR" | "CUSTOMER" => ("partners", &["id","type","name","phone","address","remark","contact_person","contact_phone","created_at","updated_at"]),
        "CONTACT" => ("(SELECT pc.id, pc.name, pc.phone, pc.email, pc.position, pc.remark, pc.created_at, COALESCE(GROUP_CONCAT(DISTINCT p.name),'') AS partner_names FROM partner_contacts pc LEFT JOIN partner_contact_mapping pcm ON pcm.contact_id=pc.id LEFT JOIN partners p ON p.id=pcm.partner_id GROUP BY pc.id)", &["id","name","phone","email","position","remark","partner_names","created_at"]),
        "TODO" => ("todos", &["id","title","content","done","priority","due_date","repeat","created_at","updated_at"]),
        "NOTE" => ("(SELECT n.note_id, n.title, n.content, n.is_daily, n.notebook_id, nb.name AS notebook_name, n.deleted_at, n.created_at, n.updated_at FROM notes n LEFT JOIN notebooks nb ON n.notebook_id = nb.id)", &["note_id","title","content","is_daily","notebook_id","notebook_name","deleted_at","created_at","updated_at"]),
        "SOLUTION" => ("solutions", &["id","project_id","name","created_at","updated_at"]),
        "SOLUTION_VERSION" => ("solution_versions", &["id","solution_id","version","tags","content","creator","status","created_at","updated_at"]),
        "SOLUTION_ITEM" => ("(SELECT si.id, si.solution_version_id, si.product_id, si.product_name, si.device_name, si.quantity, si.unit_price, si.description AS params, si.row_type, si.sort_order, si.created_at, si.updated_at, sv.version, sv.status AS version_status, s.name AS solution_name, s.project_id, p.name AS project_name FROM solution_items si JOIN solution_versions sv ON sv.id=si.solution_version_id JOIN solutions s ON s.id=sv.solution_id JOIN projects p ON p.id=s.project_id)", &["id","solution_version_id","product_id","product_name","device_name","quantity","unit_price","params","row_type","sort_order","created_at","updated_at","version","version_status","solution_name","project_id","project_name"]),
        "PROJECT_DYNAMIC" => ("project_dynamics", &["id","project_id","type","content","operator","created_at"]),
        "EMAIL" => ("emails", &["id","account_id","folder","from_addr","from_name","to_addr","cc_addr","subject","received_at","has_attachments","ai_category","created_at"]),
        "CHAT" | "FILE" => return format!("{} 不支持结构化筛选，请使用 query 关键词搜索。", entity),
        _ => return format!("不支持的实体类型: {}。可用: PROJECT/PRODUCT/CHANNEL/VENDOR/CUSTOMER/CONTACT/TODO/NOTE/SOLUTION/SOLUTION_VERSION/SOLUTION_ITEM/PROJECT_DYNAMIC/EMAIL", entity),
    };
    let mut conditions: Vec<String> = Vec::new();
    let mut values: Vec<Value> = Vec::new();
    let upper = entity.to_uppercase();
    if upper == "CHANNEL" || upper == "VENDOR" || upper == "CUSTOMER" {
        conditions.push("type = ?".to_string());
        values.push(json!(upper));
    }
    for filter in field_filters {
        let field = filter["field"].as_str().unwrap_or("");
        let op = filter["op"].as_str().unwrap_or("=");
        let value = &filter["value"];
        if !allowed_fields.contains(&field) { continue; }
        match op {
            "IS_NULL" => { conditions.push(format!("{} IS NULL", field)); }
            "IS_NOT_NULL" => { conditions.push(format!("{} IS NOT NULL", field)); }
            "IN" => {
                if let Some(arr) = value.as_array() {
                    let placeholders: Vec<String> = arr.iter().map(|_| "?".to_string()).collect();
                    conditions.push(format!("{} IN ({})", field, placeholders.join(",")));
                    for v in arr { values.push(v.clone()); }
                }
            }
            "LIKE" => {
                let v = value.as_str().unwrap_or("");
                let like_value = if v.contains('%') { v.to_string() } else { format!("%{}%", v) };
                conditions.push(format!("{} LIKE ?", field)); 
                values.push(json!(like_value));
            }
            "=" | "!=" | ">" | "<" | ">=" | "<=" => { conditions.push(format!("{} {} ?", field, op)); values.push(value.clone()); }
            _ => {}
        }
    }
    if upper == "NOTE" && !conditions.iter().any(|c| c.contains("deleted_at")) {
        conditions.push("deleted_at IS NULL".to_string());
    }
    let where_clause = if conditions.is_empty() { String::new() } else { format!(" WHERE {}", conditions.join(" AND ")) };

    // ★ 聚合模式：count_only / group_by / aggregate
    if args["count_only"].as_bool().unwrap_or(false) {
        let sql = format!("SELECT COUNT(*) as cnt FROM {}{}", table, where_clause);
        return match crate::database::get(&sql, &values) {
            Ok(Some(row)) => {
                let count = row["cnt"].as_i64().unwrap_or(0);
                let envelope = ToolResultEnvelope {
                    result_type: "object_catalog_result".to_string(),
                    data: json!({"entity": entity, "count": count}),
                    summary: Some(format!("{} 共 {} 条匹配记录", entity, count)),
                };
                envelope.to_json()
            }
            Ok(None) => {
                let envelope = ToolResultEnvelope {
                    result_type: "object_catalog_result".to_string(),
                    data: json!({"entity": entity, "count": 0}),
                    summary: Some(format!("{} 共 0 条匹配记录", entity)),
                };
                envelope.to_json()
            }
            Err(e) => {
                let envelope = ToolResultEnvelope {
                    result_type: "object_catalog_result".to_string(),
                    data: json!({"success": false}),
                    summary: Some(format!("查询失败: {}", e)),
                };
                envelope.to_json()
            }
        };
    }
    if let Some(group) = args["group_by"].as_str().filter(|s| !s.is_empty()) {
        if allowed_fields.contains(&group) {
            let sql = format!("SELECT {}, COUNT(*) as cnt FROM {}{} GROUP BY {} ORDER BY cnt DESC", group, table, where_clause, group);
            return match crate::database::all(&sql, &values) {
                Ok(rows) => {
                    if rows.is_empty() {
                        let envelope = ToolResultEnvelope {
                            result_type: "object_catalog_result".to_string(),
                            data: json!({"entity": entity, "group_by": group, "groups": []}),
                            summary: Some(format!("{} 无匹配记录。", entity)),
                        };
                        return envelope.to_json();
                    }
                    let groups: Vec<Value> = rows.iter().map(|row| {
                        let val = row[group].as_str().unwrap_or("");
                        let display = if val.is_empty() { row[group].to_string() } else { val.to_string() };
                        json!({"value": display, "count": row["cnt"].as_i64().unwrap_or(0)})
                    }).collect();
                    let envelope = ToolResultEnvelope {
                        result_type: "object_catalog_result".to_string(),
                        data: json!({"entity": entity, "group_by": group, "groups": groups}),
                        summary: Some(format!("{} 按 {} 分组。", entity, group)),
                    };
                    envelope.to_json()
                }
                Err(e) => {
                    let envelope = ToolResultEnvelope {
                        result_type: "object_catalog_result".to_string(),
                        data: json!({"error": format!("查询失败: {}", e)}),
                        summary: Some(format!("查询失败: {}", e)),
                    };
                    envelope.to_json()
                }
            };
        } else {
            let envelope = ToolResultEnvelope {
                result_type: "object_catalog_result".to_string(),
                data: json!({"error": format!("字段 '{}' 不支持分组，可用字段：{}", group, allowed_fields.join(", "))}),
                summary: Some(format!("不支持按 '{}' 分组", group)),
            };
            return envelope.to_json();
        }
    }
    if let Some(agg) = args["aggregate"].as_str().filter(|s| !s.is_empty()) {
        // 解析 SUM(budget) / AVG(price) / MAX(budget) / MIN(budget)
        if let Some(paren_start) = agg.find('(') {
            if let Some(paren_end) = agg.rfind(')') {
                let func = agg[..paren_start].to_uppercase();
                let field = &agg[paren_start+1..paren_end].trim();
                if allowed_fields.contains(field) && ["SUM","AVG","MAX","MIN"].contains(&func.as_str()) {
                    let sql = format!("SELECT COALESCE({}({}), 0) as result FROM {}{}", func, field, table, where_clause);
                    return match crate::database::get(&sql, &values) {
                        Ok(Some(row)) => {
                            let result = if let Some(n) = row["result"].as_f64() {
                                if n == (n as i64) as f64 { n as i64 } else { n as i64 }
                            } else { 0 };
                            let envelope = ToolResultEnvelope {
                                result_type: "object_catalog_result".to_string(),
                                data: json!({"entity": entity, "function": func, "field": field, "result": result}),
                                summary: Some(format!("{} {}({}): {}", entity, func, field, result)),
                            };
                            envelope.to_json()
                        }
                        Ok(None) => {
                            let envelope = ToolResultEnvelope {
                                result_type: "object_catalog_result".to_string(),
                                data: json!({"entity": entity, "function": func, "field": field, "result": 0}),
                                summary: Some(format!("{} 无匹配记录。", entity)),
                            };
                            envelope.to_json()
                        }
                        Err(e) => {
                            let envelope = ToolResultEnvelope {
                                result_type: "object_catalog_result".to_string(),
                                data: json!({"success": false}),
                                summary: Some(format!("查询失败: {}", e)),
                            };
                            envelope.to_json()
                        }
                    };
                }
            }
        }
        let envelope = ToolResultEnvelope {
            result_type: "object_catalog_result".to_string(),
            data: json!({"success": false}),
            summary: Some("聚合格式错误，请使用 SUM(字段)/AVG(字段)/MAX(字段)/MIN(字段)，字段必须是允许的字段名。".to_string()),
        };
        return envelope.to_json();
    }

    let order = if let Some(sort) = sort.and_then(|s| s.as_object()) {
        let field = sort["field"].as_str().unwrap_or("created_at");
        let dir = if sort["order"].as_str().unwrap_or("desc") == "asc" { "ASC" } else { "DESC" };
        if allowed_fields.contains(&field) {
            format!(" ORDER BY {} {}", field, dir)
        } else {
            let envelope = ToolResultEnvelope {
                result_type: "object_catalog_result".to_string(),
                data: json!({"error": format!("字段 '{}' 不支持排序，可用字段：{}", field, allowed_fields.join(", "))}),
                summary: Some(format!("不支持按 '{}' 排序", field)),
            };
            return envelope.to_json();
        }
    } else { " ORDER BY created_at DESC".to_string() };
    let limit = args["limit"].as_u64().unwrap_or(50).min(100);
    let offset = args["offset"].as_u64().unwrap_or(0);
    let sql = format!("SELECT {} FROM {}{}{} LIMIT {} OFFSET {}", allowed_fields.join(","), table, where_clause, order, limit, offset);
    
    // 先查询总记录数
    let count_sql = format!("SELECT COUNT(*) as cnt FROM {}{}", table, where_clause);
    let total: i64 = crate::database::get(&count_sql, &values)
        .ok().flatten().and_then(|r| r["cnt"].as_i64()).unwrap_or(0);
    
    match crate::database::all(&sql, &values) {
        Ok(rows) => {
            if rows.is_empty() {
                let envelope = ToolResultEnvelope {
                    result_type: "object_catalog_result".to_string(),
                    data: json!({"entity": entity, "items": [], "total": total, "offset": offset}),
                    summary: Some(format!("未找到匹配的{}对象。", entity)),
                };
                return envelope.to_json();
            }
            let items: Vec<Value> = rows.iter().map(|row| {
                json!(row)
            }).collect();
            let has_more = (offset as i64 + rows.len() as i64) < total;
            let next_offset = if has_more { Some(offset + rows.len() as u64) } else { None };
            let envelope = ToolResultEnvelope {
                result_type: "object_catalog_result".to_string(),
                data: json!({
                    "entity": entity,
                    "items": items,
                    "total": total,
                    "offset": offset,
                    "limit": limit,
                    "has_more": has_more,
                    "next_offset": next_offset
                }),
                summary: Some(format!("显示 {}/{} 条{}对象。", offset as usize + rows.len(), total, entity)),
            };
            envelope.to_json()
        }
        Err(e) => {
            let envelope = ToolResultEnvelope {
                result_type: "object_catalog_result".to_string(),
                data: json!({"success": false}),
                summary: Some(format!("查询失败: {}", e)),
            };
            envelope.to_json()
        }
    }
}

#[allow(dead_code)]
pub(crate) fn format_catalog_row(entity: &str, row: &Value) -> String {
    let upper = entity.to_uppercase();
    match upper.as_str() {
        "PROJECT" => {
            let status = row["status"].as_str().unwrap_or("");
            let budget = row["budget"].as_f64().unwrap_or(0.0);
            let bid = row["bid_time"].as_str().unwrap_or("");
            let mut s = format!("[PROJECT] {} | PROJECT-{}", row["name"].as_str().unwrap_or(""), row["id"].as_str().unwrap_or(""));
            if !status.is_empty() { s.push_str(&format!(" | 状态: {}", status)); }
            if budget > 0.0 { s.push_str(&format!(" | 预算: {}万", budget)); }
            if !bid.is_empty() { s.push_str(&format!(" | 投标: {}", bid)); }
            s
        }
        "TODO" => {
            let done = row["done"].as_i64().unwrap_or(0);
            let pri = row["priority"].as_str().unwrap_or("");
            let due = row["due_date"].as_str().unwrap_or("");
            let status = if done != 0 { "已完成" } else { "未完成" };
            let mut s = format!("[TODO] {} | TODO-{}", row["title"].as_str().unwrap_or(""), row["id"].as_str().unwrap_or(""));
            if !pri.is_empty() { s.push_str(&format!(" | 优先级: {}", pri)); }
            if !due.is_empty() { s.push_str(&format!(" | 截止: {}", due)); }
            s.push_str(&format!(" | {}", status));
            s
        }
        "PRODUCT" => {
            let mut s = format!("[PRODUCT] {} | PRODUCT-{}", row["name"].as_str().unwrap_or(""), row["id"].as_str().unwrap_or(""));
            if let Some(series) = row["series_name"].as_str().filter(|s| !s.is_empty()) { s.push_str(&format!(" | 系列: {}", series)); }
            if let Some(price) = row["price"].as_f64().filter(|p| *p > 0.0) { s.push_str(&format!(" | 价格: {}元", price)); }
            s
        }
        "CONTACT" => {
            let mut s = format!("[CONTACT] {} | CONTACT-{}", row["name"].as_str().unwrap_or(""), row["id"].as_str().unwrap_or(""));
            if let Some(phone) = row["phone"].as_str().filter(|s| !s.is_empty()) { s.push_str(&format!(" | 电话: {}", phone)); }
            if let Some(pos) = row["position"].as_str().filter(|s| !s.is_empty()) { s.push_str(&format!(" | 职位: {}", pos)); }
            s
        }
        "CHANNEL" | "VENDOR" | "CUSTOMER" => {
            let mut s = format!("[{}] {} | {}", upper, row["name"].as_str().unwrap_or(""), row["id"].as_str().unwrap_or(""));
            if let Some(phone) = row["phone"].as_str().filter(|s| !s.is_empty()) { s.push_str(&format!(" | 电话: {}", phone)); }
            if let Some(cp) = row["contact_person"].as_str().filter(|s| !s.is_empty()) { s.push_str(&format!(" | 联系人: {}", cp)); }
            s
        }
        "NOTE" => {
            let id = row["note_id"].as_str().unwrap_or(row["id"].as_str().unwrap_or(""));
            let mut s = format!("[NOTE] {} | NOTE-{}", row["title"].as_str().unwrap_or(""), id);
            if let Some(nb) = row["notebook_name"].as_str().filter(|s| !s.is_empty()) { s.push_str(&format!(" | 笔记本: {}", nb)); }
            s
        }
        _ => format!("[{}] {}", upper, row["id"].as_str().unwrap_or("")),
    }
}


/// 批量 Recall：并发获取多个对象的完整信息
pub(crate) async fn execute_batch_recall(ids: &[String], visited_chunks: &std::collections::HashSet<String>, visited_sources: &std::collections::HashSet<String>) -> String {
    if ids.is_empty() { return json!({"result_type": "object_detail_batch", "data": {"objects": []}, "summary": "请提供至少一个对象 ID。"}).to_string(); }

    // ★ 单个 ID：直接返回详情
    if ids.len() == 1 {
        let metadata = build_chunk_metadata(&ids[0]);
        if !metadata["found"].as_bool().unwrap_or(false) {
            return json!({
                "result_type": "object_detail",
                "data": {"id": ids[0], "found": false}
            }).to_string();
        }
        let source_data_id = metadata["source_data_id"].as_str().unwrap_or("");
        let source_type = metadata["type"].as_str().unwrap_or("");
        let source_id = metadata["id"].as_str().unwrap_or("");
        let chunks = fetch_parent_chunks(source_data_id, source_type, source_id, visited_chunks, visited_sources);
        return json!({
            "result_type": "object_detail",
            "data": {
                "id": metadata["id"],
                "type": metadata["type"],
                "name": metadata["name"],
                "found": true,
                "total_chunks": chunks["total_chunks"],
                "chunks": chunks["chunks"]
            }
        }).to_string();
    }

    let limit = ids.len().min(30);
    let ids = &ids[..limit];
    log::info!("[batch_recall] 收到 {} 个ID: {:?}", ids.len(), ids.iter().take(3).collect::<Vec<_>>());
    
    // 并发获取每个对象的来源信息和 chunks
    let mut handles = Vec::new();
    for id in ids.iter() {
        let id = id.clone();
        let vc = visited_chunks.clone();
        let vs = visited_sources.clone();
        handles.push(tokio::spawn(async move {
            let metadata = build_chunk_metadata(&id);
            if !metadata["found"].as_bool().unwrap_or(false) {
                return json!({"id": id, "found": false});
            }
            let source_data_id = metadata["source_data_id"].as_str().unwrap_or("");
            let source_type = metadata["type"].as_str().unwrap_or("");
            let source_id = metadata["id"].as_str().unwrap_or("");
            let chunks = fetch_parent_chunks(source_data_id, source_type, source_id, &vc, &vs);
            json!({
                "id": metadata["id"],
                "type": metadata["type"],
                "name": metadata["name"],
                "found": true,
                "total_chunks": chunks["total_chunks"],
                "chunks": chunks["chunks"]
            })
        }));
    }
    
    // 收集结果
    let mut objects: Vec<serde_json::Value> = Vec::new();
    for h in handles {
        if let Ok(obj) = h.await {
            objects.push(obj);
        }
    }
    
    // 返回纯 JSON
    json!({
        "result_type": "object_detail_batch",
        "data": {"objects": objects},
        "summary": format!("批量查询 {} 个对象", objects.len())
    }).to_string()
}


            // ── Object Retrieval (dual-mode: timeline + keyword) ─────────────────

pub(crate) fn timeline_mode(obj_type: &str, start: &str, end: &str, limit: usize, offset: usize) -> String {
    // 先获取总数（date() 包裹列使 updated_at 索引失效，改字符串区间比较可走索引）
    let total: i64 = database::get(
        "SELECT COUNT(*) as cnt FROM source_data WHERE (source_type=?1 OR ?1='') AND updated_at>=?2 AND updated_at<date(?3,'+1 day')",
        &[json!(obj_type), json!(start), json!(end)],
    ).ok().flatten().and_then(|r| r["cnt"].as_i64()).unwrap_or(0);
    
    let rows = database::all(
        "SELECT source_type, source_id, title, substr(content,1,300) as preview, length(content) as content_len, updated_at FROM source_data
         WHERE (source_type=?1 OR ?1='')
           AND updated_at >= ?2 AND updated_at < date(?3,'+1 day')
         ORDER BY updated_at DESC LIMIT ?4 OFFSET ?5",
        &[json!(obj_type), json!(start), json!(end), json!(limit), json!(offset)],
    ).unwrap_or_default();
    
    if rows.is_empty() {
        let summary = if offset > 0 { format!("已无更多对象（共 {} 条）。", total) } else { format!("时间范围内无{}对象。", if obj_type.is_empty() { "任何" } else { obj_type }) };
        let envelope = ToolResultEnvelope {
            result_type: "object_query_result".to_string(),
            data: json!({"mode": "timeline", "total": total, "offset": offset, "limit": limit, "items": []}),
            summary: Some(summary),
        };
        return envelope.to_json();
    }
    
    let mut items = Vec::new();
    let mut prev_ts: Option<String> = None;
    let mut boundary_count = 0u64;
    let mut max_boundary = 0.0f64;
    let mut total_boundary = 0.0f64;
    
    for r in &rows {
        let st = r["source_type"].as_str().unwrap_or("");
        let sid = r["source_id"].as_str().unwrap_or("");
        let name = r["title"].as_str().unwrap_or("");
        let preview_full = r["preview"].as_str().unwrap_or("");
        let preview = {
            let char_count = r["content_len"].as_i64().map(|n| n as usize).unwrap_or_else(|| preview_full.chars().count());
            if char_count <= 200 {
                preview_full.to_string()
            } else {
                let mut last_newline = 0;
                let mut char_idx = 0;
                for ch in preview_full.chars() {
                    if ch == '\n' && char_idx <= 200 {
                        last_newline = char_idx;
                    }
                    char_idx += 1;
                    if char_idx > 250 { break; }
                }
                let cut_at = if last_newline > 100 { last_newline } else { 200 };
                let truncated: String = preview_full.chars().take(cut_at).collect();
                format!("{}…(已截断，共{}字)", truncated.trim_end(), char_count)
            }
        };
        let ts = r["updated_at"].as_str().unwrap_or("");
        
        // ★ Boundary scoring (time-based only)
        if let Some(ref p_ts) = prev_ts {
            let d_time = (chrono::NaiveDateTime::parse_from_str(ts, "%Y-%m-%d %H:%M:%S").ok()
                .zip(chrono::NaiveDateTime::parse_from_str(p_ts, "%Y-%m-%d %H:%M:%S").ok())
                .map(|(t,p)| (t-p).num_minutes().max(0) as f64)).unwrap_or(999.0);
            let boundary = 1.0 - (-0.03 * d_time).exp();
            if boundary > 0.45 {
                boundary_count += 1;
                max_boundary = max_boundary.max(boundary);
                total_boundary += boundary;
            }
        }
        
        // TODO 类型：额外显示优先级、截止日期、完成状态、todo_id
        let mut extra = None;
        if st == "TODO" {
            if let Ok(Some(todo)) = database::get("SELECT t.done, t.priority, t.due_date, t.repeat, p.name as project_name FROM todos t LEFT JOIN todo_project_mapping tpm ON t.id=tpm.todo_id LEFT JOIN projects p ON p.id=tpm.project_id WHERE t.id=?1", &[json!(sid)]) {
                let done = todo["done"].as_i64().unwrap_or(0);
                let pri = todo["priority"].as_str().unwrap_or("");
                let due = todo["due_date"].as_str().unwrap_or("");
                let repeat = todo["repeat"].as_str().unwrap_or("");
                let proj = todo["project_name"].as_str().unwrap_or("");
                let done_bool = done != 0;
                let status_str = if done_bool { "已完成" } else { "未完成" };
                let mut extra_obj = json!({"status": status_str, "done": done_bool, "todo_id": sid});
                if !pri.is_empty() { extra_obj["priority"] = json!(pri); }
                if !due.is_empty() { extra_obj["due_date"] = json!(due); }
                if !repeat.is_empty() { extra_obj["repeat"] = json!(repeat); }
                if !proj.is_empty() { extra_obj["project_name"] = json!(proj); }
                extra = Some(extra_obj);
            }
        }
        // CONTACT 类型：额外显示所属公司（空串=确认未关联，与"无数据"区分）
        if st == "CONTACT" {
            if let Ok(Some(c)) = database::get(
                "SELECT COALESCE(GROUP_CONCAT(DISTINCT p.name),'') AS partner_names FROM partner_contact_mapping pcm LEFT JOIN partners p ON p.id=pcm.partner_id WHERE pcm.contact_id=?1",
                &[json!(sid)]) {
                extra = Some(json!({"partner_names": c["partner_names"].as_str().unwrap_or("")}));
            }
        }
        
        let mut item = json!({
            "source_type": st,
            "id": sid,
            "name": name,
            "preview": preview,
            "updated_at": ts
        });
        if let Some(extra_obj) = extra {
            item["extra"] = extra_obj;
        }
        items.push(item);
        
        prev_ts = Some(ts.to_string());
    }
    
    if boundary_count > 0 {
        let avg_boundary = total_boundary / boundary_count as f64;
        log::info!("[Boundary] 检测到 {} 处突变，最高 {:.2}，平均 {:.2}", boundary_count, max_boundary, avg_boundary);
    } else {
        log::info!("[Boundary] 无突变检测");
    }
    
    let end_pos = (offset + rows.len()).min(total as usize);
    let has_more = (offset + rows.len()) < total as usize;
    let next_offset = if has_more { Some(offset + rows.len()) } else { None };
    let summary = format!("显示 {}/{} 条对象。", end_pos, total);
    
    let envelope = ToolResultEnvelope {
        result_type: "object_query_result".to_string(),
        data: json!({
            "mode": "timeline",
            "total": total,
            "offset": offset,
            "limit": limit,
            "has_more": has_more,
            "next_offset": next_offset,
            "items": items
        }),
        summary: Some(summary),
    };
    envelope.to_json()
}

pub(crate) fn keyword_object_mode(query: &str, obj_type: &str, start: &str, end: &str, limit: usize, offset: usize) -> String {
    let q = query.trim(); if q.is_empty() { return timeline_mode(obj_type, start, end, limit, offset); }
    let words: Vec<String> = q.split_whitespace().map(|k| k.replace('"',"")).collect();
    let fts_q = if words.len() > 1 { format!("NEAR(({}), 30)", words.join(" ")) } else { words.first().cloned().unwrap_or_default() };
    let results = match database::fts5_search_content(&fts_q, 30) { Ok(r) => r, Err(_) => Vec::new() };
    let mut seen = std::collections::HashSet::new();
    let mut matched: Vec<Value> = Vec::new();
    // 关键词搜索：不按日期过滤，直接按 FTS5 结果从 source_data 取数据
    for r in &results {
        let st = r.entity_type.to_uppercase();
        if !obj_type.is_empty() && st != obj_type.to_uppercase() { continue; }
        let sid = r.entity_id.trim().to_string();
        if !seen.insert(format!("{}-{}", st, sid)) { continue; }
        if let Ok(Some(row)) = database::get("SELECT title, content, updated_at FROM source_data WHERE source_type=?1 AND source_id=?2", &[json!(st), json!(sid)]) {
            matched.push(json!({"st": st, "sid": sid, "name": row["title"], "preview": row["content"], "ts": row["updated_at"]}));
        }
    }
    // LIKE fallback：同时搜索 title 和 content
    if matched.is_empty() {
        let like = format!("%{}%", crate::database::escape_like(&q));
        if let Ok(rows) = database::all(
            "SELECT source_type, source_id, title, substr(content,1,500) as preview, updated_at FROM source_data WHERE (title LIKE ?1 ESCAPE '\\' OR content LIKE ?1 ESCAPE '\\') AND (source_type=?2 OR ?2='') ORDER BY updated_at DESC LIMIT 200",
            &[json!(&like), json!(obj_type)],
        ) {
            for r in &rows {
                let st = r["source_type"].as_str().unwrap_or("").to_uppercase();
                let sid = r["source_id"].as_str().unwrap_or("").to_string();
                if !seen.insert(format!("{}-{}", st, sid)) { continue; }
                matched.push(json!({"st": st, "sid": sid, "name": r["title"], "preview": r["preview"], "ts": r["updated_at"]}));
            }
        }
    }
    
    if matched.is_empty() {
        let envelope = ToolResultEnvelope {
            result_type: "object_query_result".to_string(),
            data: json!({"mode": "keyword", "query": q, "total": 0, "offset": offset, "limit": limit, "items": []}),
            summary: Some(format!("未找到匹配「{}」的对象。", q)),
        };
        return envelope.to_json();
    }
    
    let total = matched.len();
    let start_pos = offset.min(total);
    let end_pos = (offset + limit).min(total);
    let page: Vec<&Value> = matched.iter().skip(start_pos).take(limit).collect();
    
    let mut items = Vec::new();
    for r in &page {
        let st = r["st"].as_str().unwrap_or("");
        let sid = r["sid"].as_str().unwrap_or("");
        let name = r["name"].as_str().unwrap_or("");
        let preview_full = r["preview"].as_str().unwrap_or("");
        let preview = {
            let char_count = preview_full.chars().count();
            if char_count <= 200 {
                preview_full.to_string()
            } else {
                let mut last_newline = 0;
                let mut char_idx = 0;
                for ch in preview_full.chars() {
                    if ch == '\n' && char_idx <= 200 {
                        last_newline = char_idx;
                    }
                    char_idx += 1;
                    if char_idx > 250 { break; }
                }
                let cut_at = if last_newline > 100 { last_newline } else { 200 };
                let truncated: String = preview_full.chars().take(cut_at).collect();
                format!("{}…(已截断，共{}字)", truncated.trim_end(), char_count)
            }
        };
        let ts = r["ts"].as_str().unwrap_or("");
        
        // TODO 类型：额外显示优先级、截止日期、完成状态、todo_id
        let mut extra = None;
        if st == "TODO" {
            if let Ok(Some(todo)) = database::get("SELECT t.done, t.priority, t.due_date, t.repeat, p.name as project_name FROM todos t LEFT JOIN todo_project_mapping tpm ON t.id=tpm.todo_id LEFT JOIN projects p ON p.id=tpm.project_id WHERE t.id=?1", &[json!(sid)]) {
                let done = todo["done"].as_i64().unwrap_or(0);
                let pri = todo["priority"].as_str().unwrap_or("");
                let due = todo["due_date"].as_str().unwrap_or("");
                let repeat = todo["repeat"].as_str().unwrap_or("");
                let proj = todo["project_name"].as_str().unwrap_or("");
                let done_bool = done != 0;
                let status_str = if done_bool { "已完成" } else { "未完成" };
                let mut extra_obj = json!({"status": status_str, "done": done_bool, "todo_id": sid});
                if !pri.is_empty() { extra_obj["priority"] = json!(pri); }
                if !due.is_empty() { extra_obj["due_date"] = json!(due); }
                if !repeat.is_empty() { extra_obj["repeat"] = json!(repeat); }
                if !proj.is_empty() { extra_obj["project_name"] = json!(proj); }
                extra = Some(extra_obj);
            }
        }
        // CONTACT 类型：额外显示所属公司（空串=确认未关联，与"无数据"区分）
        if st == "CONTACT" {
            if let Ok(Some(c)) = database::get(
                "SELECT COALESCE(GROUP_CONCAT(DISTINCT p.name),'') AS partner_names FROM partner_contact_mapping pcm LEFT JOIN partners p ON p.id=pcm.partner_id WHERE pcm.contact_id=?1",
                &[json!(sid)]) {
                extra = Some(json!({"partner_names": c["partner_names"].as_str().unwrap_or("")}));
            }
        }
        
        let mut item = json!({
            "source_type": st,
            "id": sid,
            "name": name,
            "preview": preview,
            "updated_at": ts
        });
        if let Some(extra_obj) = extra {
            item["extra"] = extra_obj;
        }
        items.push(item);
    }
    
    let summary = format!("显示 {}/{} 条对象。", end_pos, total);
    let has_more = end_pos < total;
    let next_offset = if has_more { Some(end_pos) } else { None };
    
    let envelope = ToolResultEnvelope {
        result_type: "object_query_result".to_string(),
        data: json!({
            "mode": "keyword",
            "query": q,
            "total": total,
            "offset": offset,
            "limit": limit,
            "has_more": has_more,
            "next_offset": next_offset,
            "items": items
        }),
        summary: Some(summary),
    };
    envelope.to_json()
}

/// Recall 模式：按天分组展示所有对象，不去重、不排序，完整回放
pub(crate) fn recall_mode(obj_type: &str, start: &str, end: &str, limit: usize, offset: usize) -> String {
    let rows = database::all(
        "SELECT source_type, source_id, title, updated_at FROM source_data
         WHERE (source_type=?1 OR ?1='')
           AND date(updated_at) >= ?2 AND date(updated_at) <= ?3
         ORDER BY updated_at ASC",
        &[json!(obj_type), json!(start), json!(end)],
    ).unwrap_or_default();
    
    if rows.is_empty() {
        return format!("时间范围内无{}对象。", if obj_type.is_empty() { "任何" } else { obj_type });
    }
    
    // 按日期分组
    let mut day_groups: std::collections::BTreeMap<String, Vec<&serde_json::Value>> = std::collections::BTreeMap::new();
    for r in &rows {
        let date = r["updated_at"].as_str().unwrap_or("").get(..10).unwrap_or("未知").to_string();
        day_groups.entry(date).or_default().push(r);
    }
    
    let total_days = day_groups.len();
    let days: Vec<_> = day_groups.iter().collect();
    let start_day = offset.min(total_days);
    let end_day = (offset + limit).min(total_days);
    
    if start_day >= total_days {
        return format!("已无更多数据（共 {} 天）。", total_days);
    }
    
    let mut out = format!("┌─ 共 {} 天，显示第 {}-{} 天 ─┐\n\n", total_days, start_day + 1, end_day);
    let mut total_chars = out.len();
    
    for (date, items) in days.iter().skip(start_day).take(limit) {
        // 统计新增和修改数量
        let mut new_count = 0;
        let mut update_count = 0;
        for r in items.iter() {
            let created = r["created_at"].as_str().unwrap_or("").get(..10).unwrap_or("");
            let updated = r["updated_at"].as_str().unwrap_or("").get(..10).unwrap_or("");
            if created == *date {
                new_count += 1;
            } else if updated == *date {
                update_count += 1;
            }
        }
        let day_header = format!("==== {} (新增: {}, 修改: {}) ====\n", date, new_count, update_count);
        out.push_str(&day_header);
        
        for r in items.iter() {
            let st = r["source_type"].as_str().unwrap_or("");
            let sid = r["source_id"].as_str().unwrap_or("");
            let mut name = r["title"].as_str().unwrap_or("").to_string();
            let created = r["created_at"].as_str().unwrap_or("").get(..10).unwrap_or("");
            let updated = r["updated_at"].as_str().unwrap_or("").get(..10).unwrap_or("");
            
            // 判断标记：+新增 *修改 -其他
            let marker = if created == *date { "+" } else if updated == *date { "*" } else { "-" };
            
            // FILE 类型：从 source_data 获取文件名
            if st == "FILE" && name.is_empty() {
                if let Ok(Some(sd)) = database::get(
                    "SELECT title FROM source_data WHERE source_type='FILE' AND source_id=?1",
                    &[json!(sid)],
                ) {
                    if let Some(t) = sd["title"].as_str().filter(|s| !s.is_empty()) {
                        name = t.to_string();
                    }
                }
            }
            
            // 输出主对象
            out.push_str(&format!("{} [{}] {} | source_id: {}-{}\n", marker, st, name, st, sid));
        }
        out.push('\n');
        
        total_chars = out.len();
        // 如果超过约 200K 字符（约 50K token），截断并提示分页
        if total_chars > 200_000 {
            let remaining_days = total_days - (start_day + (out.matches("====").count() - start_day));
            out.push_str(&format!("└─ 输出已截断（{} 字符），还有约 {} 天的数据，传 offset={} 继续 ─┘", total_chars, remaining_days, end_day));
            break;
        }
    }
    
    if end_day < total_days && total_chars <= 200_000 {
        out.push_str(&format!("└─ 还有 {} 天未显示，传 offset={} 继续查看 ─┘", total_days - end_day, end_day));
    }
    
    out
}

pub(crate) fn execute_object_get(query: &str, obj_type: &str, time_range: Option<(String, String)>, limit: usize, offset: usize, mode: &str) -> String {
    let limit = if limit == 0 { 50 } else { limit.min(100) };
    let (start, end) = match time_range {
        Some((s, e)) => (s, e),
        None => ("0001-01-01".to_string(), "2099-12-31".to_string()),
    };
    // 如果有关键词，优先使用关键词搜索模式
    if !query.trim().is_empty() {
        return keyword_object_mode(query, obj_type, &start, &end, limit, offset);
    }
    if mode == "recall" {
        recall_mode(obj_type, &start, &end, limit, offset)
    } else {
        timeline_mode(obj_type, &start, &end, limit, offset)
    }
}
