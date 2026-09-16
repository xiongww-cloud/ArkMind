//! ext_tool_manage_notification：通知增删改查执行器

use crate::database;
use serde_json::{json, Value};
use crate::services::agent_workflow::*;

pub(crate) fn execute(a: &Value, app_handle: &Option<tauri::AppHandle>) -> String {
    let a = a.clone();
                            let action = a["action"].as_str().unwrap_or("create");
                            let res = match action {
                                "get" => {
                                    let nid = a["notification_id"].as_str().unwrap_or("");
                                    if nid.is_empty() {
                                        let envelope = ToolResultEnvelope {
                                            result_type: "notification_get_result".to_string(),
                                            data: json!({"success": false}),
                                            summary: Some("请提供 notification_id。".to_string()),
                                        };
                                        envelope.to_json()
                                    } else {
                                        match crate::database::get(
                                            "SELECT id, type, title, content, entity_type, entity_id, priority, source, follow_up_of, actions, created_at FROM notifications WHERE id=?1",
                                            &[json!(nid)],
                                        ) {
                                            Ok(Some(r)) => {
                                                let ntype = r["type"].as_str().unwrap_or("");
                                                let title = r["title"].as_str().unwrap_or("");
                                                let content = r["content"].as_str().unwrap_or("");
                                                let entity_type = r["entity_type"].as_str().unwrap_or("");
                                                let entity_id = r["entity_id"].as_str().unwrap_or("");
                                                let priority = r["priority"].as_str().unwrap_or("medium");
                                                let follow_up_of = r["follow_up_of"].as_str().unwrap_or("");
                                                let actions_raw = r["actions"].as_str().unwrap_or("[]");
                                                let actions: Value = serde_json::from_str(actions_raw).unwrap_or(json!([]));
                                                let created = r["created_at"].as_str().unwrap_or("");
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "notification_get_result".to_string(),
                                                    data: json!({
                                                        "id": nid,
                                                        "type": ntype,
                                                        "title": title,
                                                        "content": content,
                                                        "entity_type": entity_type,
                                                        "entity_id": entity_id,
                                                        "priority": priority,
                                                        "follow_up_of": follow_up_of,
                                                        "actions": actions,
                                                        "created_at": created
                                                    }),
                                                    summary: Some(format!("通知「{}」", title)),
                                                };
                                                envelope.to_json()
                                            }
                                            Ok(None) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "notification_get_result".to_string(),
                                                    data: json!({"success": false}),
                                                    summary: Some(format!("未找到通知: {}", nid)),
                                                };
                                                envelope.to_json()
                                            }
                                            Err(e) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "notification_get_result".to_string(),
                                                    data: json!({"success": false}),
                                                    summary: Some(format!("查询失败: {}", e)),
                                                };
                                                envelope.to_json()
                                            },
                                        }
                                    }
                                }
                                "list" => {
                                    let limit = a["limit"].as_u64().unwrap_or(20) as usize;
                                    let offset = a["offset"].as_u64().unwrap_or(0) as usize;
                                    let sql = "SELECT id, type, title, content, entity_type, entity_id, priority, source, follow_up_of, actions, created_at FROM notifications ORDER BY created_at DESC LIMIT ?1 OFFSET ?2";
                                    match crate::database::all(sql, &[json!(limit), json!(offset)]) {
                                        Ok(rows) => {
                                            let items: Vec<Value> = rows.iter().map(|r| {
                                                let actions_raw = r["actions"].as_str().unwrap_or("[]");
                                                let actions: Value = serde_json::from_str(actions_raw).unwrap_or(json!([]));
                                                json!({
                                                    "id": r["id"].as_str().unwrap_or(""),
                                                    "type": r["type"].as_str().unwrap_or(""),
                                                    "title": r["title"].as_str().unwrap_or(""),
                                                    "priority": r["priority"].as_str().unwrap_or("medium"),
                                                    "actions": actions,
                                                    "created_at": r["created_at"].as_str().unwrap_or("")
                                                })
                                            }).collect();
                                            let envelope = ToolResultEnvelope {
                                                result_type: "notification_list_result".to_string(),
                                                data: json!({
                                                    "total": items.len(),
                                                    "offset": offset,
                                                    "items": items
                                                }),
                                                summary: Some(if items.is_empty() { "暂无通知。".to_string() } else { format!("共 {} 条通知。", items.len()) }),
                                            };
                                            envelope.to_json()
                                        }
                                        Err(e) => {
                                            let envelope = ToolResultEnvelope {
                                                result_type: "notification_list_result".to_string(),
                                                data: json!({"success": false}),
                                                summary: Some(format!("查询失败: {}", e)),
                                            };
                                            envelope.to_json()
                                        },
                                    }
                                }
                                "delete" => {
                                    let nid = a["notification_id"].as_str().unwrap_or("");
                                    if nid.is_empty() {
                                        let envelope = ToolResultEnvelope {
                                            result_type: "notification_delete_result".to_string(),
                                            data: json!({"success": false}),
                                            summary: Some("请提供 notification_id。".to_string()),
                                        };
                                        envelope.to_json()
                                    } else {
                                        let notification_title = database::get(
                                            "SELECT title FROM notifications WHERE id=?1",
                                            &[json!(nid)],
                                        ).ok().flatten().and_then(|r| r["title"].as_str().map(String::from)).unwrap_or_default();
                                        let _ = database::run("DELETE FROM notifications WHERE id=?1", &[json!(nid)]);
                                        if let Some(ref h) = app_handle { let _ = h.emit("notification:deleted", &nid); }
                                        let display_name = if notification_title.is_empty() { nid.to_string() } else { notification_title };
                                        let envelope = ToolResultEnvelope {
                                            result_type: "notification_delete_result".to_string(),
                                            data: json!({"id": nid, "success": true}),
                                            summary: Some(format!("已删除通知「{}」", display_name)),
                                        };
                                        envelope.to_json()
                                    }
                                }
                                "update" => {
                                    let nid = a["notification_id"].as_str().unwrap_or("");
                                    if nid.is_empty() {
                                        let envelope = ToolResultEnvelope {
                                            result_type: "notification_update_result".to_string(),
                                            data: json!({"success": false}),
                                            summary: Some("请提供 notification_id。".to_string()),
                                        };
                                        envelope.to_json()
                                    } else {
                                        let mut sets = Vec::new();
                                        let mut params: Vec<Value> = Vec::new();
                                        if let Some(t) = a.get("type").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
                                            sets.push("type=?"); params.push(json!(t));
                                        }
                                        if let Some(t) = a.get("title").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
                                            sets.push("title=?"); params.push(json!(t));
                                        }
                                        if let Some(c) = a.get("content").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
                                            sets.push("content=?"); params.push(json!(c));
                                        }
                                        if let Some(p) = a.get("priority").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
                                            sets.push("priority=?"); params.push(json!(p));
                                        }
                                        if let Some(et) = a.get("entity_type").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
                                            sets.push("entity_type=?"); params.push(json!(et));
                                        }
                                        if let Some(ei) = a.get("entity_id").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
                                            sets.push("entity_id=?"); params.push(json!(ei));
                                        }
                                        if let Some(actions_arr) = a.get("actions") {
                                            let actions_str = serde_json::to_string(actions_arr).unwrap_or_else(|_| "[]".to_string());
                                            sets.push("actions=?"); params.push(json!(actions_str));
                                        }
                                        if sets.is_empty() {
                                            let envelope = ToolResultEnvelope {
                                                result_type: "notification_update_result".to_string(),
                                                data: json!({"id": nid, "success": false}),
                                                summary: Some("没有要更新的字段。".to_string()),
                                            };
                                            envelope.to_json()
                                        } else {
                                            let sql = format!("UPDATE notifications SET {} WHERE id=?", sets.join(", "));
                                            params.push(json!(nid));
                                            let _ = database::run(&sql, &params);
                                            let notification_title = database::get(
                                                "SELECT title FROM notifications WHERE id=?1",
                                                &[json!(nid)],
                                            ).ok().flatten().and_then(|r| r["title"].as_str().map(String::from)).unwrap_or_default();
                                            let display_name = if notification_title.is_empty() { nid.to_string() } else { notification_title };
                                            let envelope = ToolResultEnvelope {
                                                result_type: "notification_update_result".to_string(),
                                                data: json!({"id": nid, "success": true}),
                                                summary: Some(format!("已更新通知「{}」", display_name)),
                                            };
                                            envelope.to_json()
                                        }
                                    }
                                }
                                _ => { // create
                                    let ntype = a["type"].as_str().unwrap_or("llm_insight");
                                    let title = crate::commands::notification_commands::sanitize_notification_text(a["title"].as_str().unwrap_or(""));
                                    let content = crate::commands::notification_commands::sanitize_notification_text(a["content"].as_str().unwrap_or(""));
                                    let entity_type = a["entity_type"].as_str().unwrap_or("");
                                    let entity_id = a["entity_id"].as_str().unwrap_or("");
                                    let priority = a["priority"].as_str().unwrap_or("medium");
                                    let follow_up_of = a["follow_up_of"].as_str().unwrap_or("");
                                    let actions_str = if let Some(actions_arr) = a.get("actions") {
                                        actions_arr.as_str().map(String::from).unwrap_or_else(|| {
                                            serde_json::to_string(actions_arr).unwrap_or_else(|_| "[]".to_string())
                                        })
                                    } else {
                                        "[]".to_string()
                                    };
                                    let is_dup = if !entity_id.is_empty() {
                                        database::get(
                                            "SELECT id FROM notifications WHERE type=?1 AND entity_id=?2 AND created_at > datetime('now','-1 day')",
                                            &[json!(ntype), json!(entity_id)]
                                        ).ok().flatten().is_some()
                                    } else { false };
                                    if is_dup {
                                        let envelope = ToolResultEnvelope {
                                            result_type: "notification_create_result".to_string(),
                                            data: json!({"success": false}),
                                            summary: Some("通知已存在，跳过创建".to_string()),
                                        };
                                        envelope.to_json()
                                    } else {
                                        let id = uuid::Uuid::new_v4().to_string();
                                        let _ = database::run(
                                            "INSERT INTO notifications(id,type,title,content,entity_type,entity_id,priority,source,follow_up_of,actions) VALUES(?1,?2,?3,?4,?5,?6,?7,'llm',?8,?9)",
                                            &[json!(&id), json!(ntype), json!(title), json!(content), json!(entity_type), json!(entity_id), json!(priority), json!(follow_up_of), json!(actions_str)],
                                        );
                                        if let Some(ref h) = app_handle { let _ = h.emit("notification:new", &id); }
                                        let envelope = ToolResultEnvelope {
                                            result_type: "notification_create_result".to_string(),
                                            data: json!({"id": id, "title": title, "success": true}),
                                            summary: Some(format!("已创建通知「{}」", title)),
                                        };
                                        envelope.to_json()
                                    }
                                }
                            };
    res
}