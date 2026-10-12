//! ext_tool_manage_todo：待办增删改执行器

use crate::database;
use serde_json::json;
use crate::services::agent_workflow::*;
use super::entity_ref::fmt;

pub(crate) fn ext_tool_manage_todo(action: &str, title: &str, content: &str, priority: &str, due_date: &str, repeat: &str, project_id: &str, done: Option<bool>, todo_id: &str) -> String {
    let normalize_pri = crate::commands::todo_commands::normalize_todo_priority;
    // 通过 todo_id 或 title 模糊匹配查找待办
    let find_todo = |tid: &str, t: &str| -> Option<(String, String)> {
        if !tid.trim().is_empty() {
            let clean_id = tid.trim().strip_prefix("TODO-").unwrap_or(tid.trim());
            database::get("SELECT id,title FROM todos WHERE id=?1", &[json!(clean_id)])
                .ok().flatten().map(|r| (r["id"].as_str().unwrap_or("").to_string(), r["title"].as_str().unwrap_or("").to_string()))
        } else if !t.trim().is_empty() {
            let like = format!("%{}%", crate::database::escape_like(t.trim()));
            database::get("SELECT id,title FROM todos WHERE title LIKE ?1 ESCAPE '\\' ORDER BY created_at DESC LIMIT 1", &[json!(&like)])
                .ok().flatten().map(|r| (r["id"].as_str().unwrap_or("").to_string(), r["title"].as_str().unwrap_or("").to_string()))
        } else { None }
    };
    match action {
        "create" => {
            if title.trim().is_empty() {
                let envelope = ToolResultEnvelope {
                    result_type: "todo_create_result".to_string(),
                    data: json!({"success": false}),
                    summary: Some("创建待办需要提供 title。".to_string()),
                };
                return envelope.to_json();
            }
            // ★ 同名复用：精确同名待办直接复用，不重复创建（AI 重复建档/并发写入防护）
            if let Ok(Some(existing)) = database::get(
                "SELECT id, title FROM todos WHERE title=?1 ORDER BY created_at DESC LIMIT 1",
                &[json!(title.trim())],
            ) {
                let eid = existing["id"].as_str().unwrap_or("").to_string();
                let etitle = existing["title"].as_str().unwrap_or(title.trim()).to_string();
                if !eid.is_empty() {
                    let envelope = ToolResultEnvelope {
                        result_type: "object_create_result".to_string(),
                        data: json!({"success": true, "id": fmt("TODO", &eid), "type": "TODO", "name": etitle, "duplicate": true}),
                        summary: Some(format!("已存在同名待办「{}」，ID: {}，已复用，未重复创建。", etitle, eid)),
                    };
                    return envelope.to_json();
                }
            }
            let p = normalize_pri(priority);
            let d = if due_date.trim().is_empty() { None } else { Some(due_date.trim()) };
            let r = if repeat.trim().is_empty() { None } else { Some(repeat.trim()) };
            let pid = resolve_project_id(project_id);
            let data = json!({
                "title": title.trim(),
                "content": content.trim(),
                "priority": p,
                "dueDate": d,
                "repeat": r,
                "projectId": pid
            });
            // 使用 block_on 调用异步函数
            match block_on_async(crate::commands::todo_commands::todos_create(data)) {
                Ok(result) => {
                    let todo_id = result["todo"]["id"].as_str().unwrap_or("");
                    let todo_title = result["todo"]["title"].as_str().unwrap_or(title.trim());
                    // 派生边由 todos_create 内部 resync（business_relations 为映射表派生视图）
                    if !todo_id.is_empty() && pid.is_some() {
                        log::info!("[待办] 关联项目: todo_id={}, project_id={:?}", todo_id, pid);
                    }
                    let envelope = ToolResultEnvelope {
                        result_type: "object_create_result".to_string(),
                        data: json!({"success": true, "id": fmt("TODO", todo_id), "type": "TODO", "name": todo_title}),
                        summary: Some(format!("已创建待办「{}」，ID: {}", todo_title, todo_id)),
                    };
                    envelope.to_json()
                },
                Err(e) => {
                    let envelope = ToolResultEnvelope {
                        result_type: "object_create_result".to_string(),
                        data: json!({"success": false, "error": e.to_string()}),
                        summary: Some(format!("创建待办失败: {}", e)),
                    };
                    envelope.to_json()
                },
            }
        }
        "update" => {
            let found = find_todo(todo_id, title);
            match found {
                Some((fid, ftitle)) => {
                    let mut data = json!({});
                    if !content.trim().is_empty() { data["content"] = json!(content.trim()); }
                    if !priority.trim().is_empty() { data["priority"] = json!(normalize_pri(priority).unwrap_or_default()); }
                    if !due_date.trim().is_empty() { data["dueDate"] = json!(due_date.trim()); }
                    if !repeat.trim().is_empty() { data["repeat"] = json!(repeat.trim()); }
                    if let Some(d) = done { data["done"] = json!(d); }
                    // 处理项目关联
                    if !project_id.trim().is_empty() {
                        data["projectId"] = json!(resolve_project_id(project_id));
                    }
                    if data.as_object().map_or(true, |m| m.is_empty()) {
                        let envelope = ToolResultEnvelope {
                            result_type: "object_update_result".to_string(),
                            data: json!({"success": false, "id": fmt("TODO", &fid), "error": "更新待办需要提供要修改的字段"}),
                            summary: Some("更新待办需要提供要修改的字段。".to_string()),
                        };
                        return envelope.to_json();
                    }
                    // 使用 block_on 调用异步函数
                    match block_on_async(crate::commands::todo_commands::todos_update(fid.clone(), data)) {
                        Ok(_) => {
                            // 派生边由 todos_update 内部 resync
                            let envelope = ToolResultEnvelope {
                                result_type: "object_update_result".to_string(),
                                data: json!({"success": true, "id": fmt("TODO", &fid), "type": "TODO", "name": ftitle}),
                                summary: Some(format!("已更新待办「{}」", ftitle)),
                            };
                            envelope.to_json()
                        }
                        Err(e) => {
                            let envelope = ToolResultEnvelope {
                                result_type: "object_update_result".to_string(),
                                data: json!({"success": false, "id": fmt("TODO", &fid), "error": e.to_string()}),
                                summary: Some(format!("更新待办失败: {}", e)),
                            };
                            envelope.to_json()
                        },
                    }
                }
                None => {
                    let envelope = ToolResultEnvelope {
                        result_type: "object_update_result".to_string(),
                        data: json!({"success": false, "error": "未找到匹配的待办"}),
                        summary: Some("未找到匹配的待办。".to_string()),
                    };
                    envelope.to_json()
                },
            }
        }
        "delete" => {
            let found = find_todo(todo_id, title);
            match found {
                                Some((fid, ftitle)) => {
                                    // 使用 block_on 调用异步函数
                                    match block_on_async(crate::commands::todo_commands::todos_delete(fid.clone())) {
                        Ok(_) => {
                            let envelope = ToolResultEnvelope {
                                result_type: "object_delete_result".to_string(),
                                data: json!({"success": true, "id": fmt("TODO", &fid), "type": "TODO", "name": ftitle}),
                                summary: Some(format!("已删除待办「{}」", ftitle)),
                            };
                            envelope.to_json()
                        }
                        Err(e) => {
                            let envelope = ToolResultEnvelope {
                                result_type: "object_delete_result".to_string(),
                                data: json!({"success": false, "id": fmt("TODO", &fid), "error": e.to_string()}),
                                summary: Some(format!("删除待办失败: {}", e)),
                            };
                            envelope.to_json()
                        },
                    }
                }
                None => {
                    let envelope = ToolResultEnvelope {
                        result_type: "object_delete_result".to_string(),
                        data: json!({"success": false, "error": "未找到匹配的待办"}),
                        summary: Some("未找到匹配的待办。".to_string()),
                    };
                    envelope.to_json()
                },
            }
        }
        _ => {
            let envelope = ToolResultEnvelope {
                result_type: "object_error".to_string(),
                data: json!({"success": false, "error": format!("未知操作「{}」。支持：create, update, delete。", action)}),
                summary: Some(format!("未知操作「{}」。支持：create, update, delete。", action)),
            };
            envelope.to_json()
        },
    }
}

