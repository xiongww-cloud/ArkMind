//! ext_tool_memory：长期记忆增删改查执行器

use crate::database;
use serde_json::{json, Value};
use crate::services::agent_workflow::*;

pub(crate) fn ext_tool_memory(action: &str, content: &str, items_json: &str, memory_id: &str, query: &str, keywords_json: &str, start_time: &str, end_time: &str, profile: bool) -> String {
    match action {
        "create" => {
            let contents: Vec<String> = if !items_json.trim().is_empty() {
                serde_json::from_str::<Vec<String>>(items_json)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|s| !s.trim().is_empty())
                    .collect()
            } else {
                let s = content.trim().to_string();
                if s.is_empty() {
                    let envelope = ToolResultEnvelope {
                        result_type: "memory_create_result".to_string(),
                        data: json!({"created": 0, "ids": []}),
                        summary: Some("创建记忆需要提供 content 或 items。".to_string()),
                    };
                    return envelope.to_json();
                }
                vec![s]
            };
            let source = if profile { "profile" } else { "user" };
            let mut created = 0;
            let mut ids: Vec<String> = Vec::new();
            for statement in &contents {
                let id = uuid::Uuid::new_v4().to_string();
                if let Ok(r) = database::run(
                    "INSERT INTO memories(id,content,source,updated_at) VALUES(?1,?2,?3,datetime('now','localtime'))",
                    &[json!(&id), json!(statement), json!(source)],
                ) {
                    if r.changes > 0 {
                        created += 1;
                        ids.push(id);
                    }
                }
            }
            let verb = if profile { "已记入你的画像" } else { "已记住" };
            let summary = if created == 1 {
                let short_id = &ids[0];
                format!("{} 1 条，ID: {}", verb, short_id)
            } else {
                let short_ids: Vec<&str> = ids.iter().map(|id| id.as_str()).collect();
                format!("{} {} 条，ID: {}", verb, created, short_ids.join(", "))
            };
            let envelope = ToolResultEnvelope {
                result_type: "memory_create_result".to_string(),
                data: json!({"created": created, "ids": ids}),
                summary: Some(summary),
            };
            envelope.to_json()
        }
        "update" => {
            let mid = memory_id.trim();
            if mid.is_empty() {
                let envelope = ToolResultEnvelope {
                    result_type: "memory_update_result".to_string(),
                    data: json!({"success": false, "memory_id": null}),
                    summary: Some("更新需要提供 memory_id。".to_string()),
                };
                return envelope.to_json();
            }
            let c = content.trim();
            if c.is_empty() {
                let envelope = ToolResultEnvelope {
                    result_type: "memory_update_result".to_string(),
                    data: json!({"success": false, "memory_id": mid}),
                    summary: Some("更新需要提供 content。".to_string()),
                };
                return envelope.to_json();
            }
            let sid = mid;
            let (success, summary) = match database::run("UPDATE memories SET content=?1, updated_at=datetime('now','localtime') WHERE id=?2", &[json!(c), json!(sid)]) {
                Ok(r) if r.changes > 0 => (true, format!("已更新记忆: {}", sid)),
                Err(e) => (false, format!("更新失败: {}", e)),
                _ => (false, format!("未找到记忆: {}", sid)),
            };
            let envelope = ToolResultEnvelope {
                result_type: "memory_update_result".to_string(),
                data: json!({"success": success, "memory_id": mid}),
                summary: Some(summary),
            };
            envelope.to_json()
        }
        "delete" => {
            if !memory_id.trim().is_empty() {
                let sid = memory_id.trim().to_string();
                let (success, summary) = match database::run("DELETE FROM memories WHERE id=?1", &[json!(sid)]) {
                    Ok(r) if r.changes > 0 => (true, format!("已删除记忆: {}", sid)),
                    Err(e) => (false, format!("删除失败: {}", e)),
                    _ => (false, format!("未找到记忆: {}", sid)),
                };
                let envelope = ToolResultEnvelope {
                    result_type: "memory_delete_result".to_string(),
                    data: json!({"success": success, "memory_id": memory_id}),
                    summary: Some(summary),
                };
                envelope.to_json()
            } else {
                let envelope = ToolResultEnvelope {
                    result_type: "memory_delete_result".to_string(),
                    data: json!({"success": false, "memory_id": null}),
                    summary: Some("删除需要提供 memory_id。".to_string()),
                };
                envelope.to_json()
            }
        }
        "search" => {
            let q = query.trim();
            let start = start_time.trim();
            let end = end_time.trim();
            let search_words: Vec<String> = if !keywords_json.is_empty() {
                serde_json::from_str::<Vec<String>>(keywords_json).unwrap_or_default()
            } else if !q.is_empty() {
                q.split_whitespace().map(String::from).collect()
            } else {
                vec![]
            };
            
            // ★ 检测是否为"列出所有"模式
            let fetch_all = search_words.is_empty() 
                || search_words.iter().all(|w| w == "*" || w == "all" || w == "全部");
            
            let mut sql = String::from("SELECT id, content, created_at FROM memories");
            let mut params: Vec<serde_json::Value> = Vec::new();
            let mut param_idx = 1;
            let mut where_clauses: Vec<String> = Vec::new();
            
            if !fetch_all && !search_words.is_empty() {
                let fts_query = search_words.iter().map(|w| format!("content:{}", w)).collect::<Vec<_>>().join(" ");
                let fts_sql = "SELECT rowid FROM memories_fts WHERE memories_fts MATCH ?1 ORDER BY bm25(memories_fts) LIMIT 10";
                if let Ok(fts_rows) = database::all(fts_sql, &[json!(&fts_query)]) {
                    let rowids: Vec<i64> = fts_rows.iter().filter_map(|r| r["rowid"].as_i64()).collect();
                    if !rowids.is_empty() {
                        let placeholders: Vec<String> = rowids.iter().enumerate().map(|(i, _)| format!("?{}", param_idx + i)).collect();
                        where_clauses.push(format!("rowid IN ({})", placeholders.join(",")));
                        for rid in rowids { params.push(json!(rid)); }
                        param_idx += placeholders.len();
                    }
                }
                if where_clauses.is_empty() {
                    for w in &search_words {
                        let like = format!("%{}%", crate::database::escape_like(w));
                        where_clauses.push(format!("content LIKE ?{} ESCAPE '\\'", param_idx));
                        params.push(json!(like));
                        param_idx += 1;
                    }
                }
            }
            if !start.is_empty() {
                where_clauses.push(format!("date(created_at) >= ?{}", param_idx));
                params.push(json!(start));
                param_idx += 1;
            }
            if !end.is_empty() {
                where_clauses.push(format!("date(created_at) <= ?{}", param_idx));
                params.push(json!(end));
            }
            if !where_clauses.is_empty() {
                sql.push_str(" WHERE ");
                sql.push_str(&where_clauses.join(" AND "));
            }
            sql.push_str(" ORDER BY created_at DESC LIMIT 20");
            match database::all(&sql, &params) {
                Ok(rows) => {
                    let memories: Vec<Value> = rows.iter().map(|r| {
                        let sid = r["id"].as_str().unwrap_or("");
                        let content = r["content"].as_str().unwrap_or("");
                        json!({
                            "memory_id": sid.to_string(),
                            "content": content
                        })
                    }).collect();
                    
                    let summary = if memories.is_empty() {
                        if q.is_empty() { "暂无记忆。".to_string() }
                        else { format!("未找到匹配「{}」的记忆。", q) }
                    } else {
                        format!("找到 {} 条记忆。", memories.len())
                    };
                    
                    let envelope = ToolResultEnvelope {
                        result_type: "memory_search_result".to_string(),
                        data: json!({
                            "action": "search",
                            "total": memories.len(),
                            "memories": memories
                        }),
                        summary: Some(summary),
                    };
                    envelope.to_json()
                }
                Err(e) => {
                    let envelope = ToolResultEnvelope {
                        result_type: "memory_search_result".to_string(),
                        data: json!({"action": "search", "total": 0, "memories": []}),
                        summary: Some(format!("搜索记忆失败: {}", e)),
                    };
                    envelope.to_json()
                }
            }
        }
        _ => {
            let envelope = ToolResultEnvelope {
                result_type: "memory_unknown".to_string(),
                data: json!({}),
                summary: Some(format!("未知操作「{}」。支持：create, update, delete, search。", action)),
            };
            envelope.to_json()
        }
    }
}

