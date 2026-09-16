//! ext_tool_manage_project：项目增删改与动态执行器

use crate::database;
use serde_json::{json, Value};
use crate::services::agent_workflow::*;

pub(crate) fn execute(a: &Value) -> String {
    let a = a.clone();
                            let action = a["action"].as_str().unwrap_or("");
                            let res = match action {
                                "create" => {
                                    let name = a["name"].as_str().unwrap_or("");
                                    if name.is_empty() {
                                        let envelope = ToolResultEnvelope {
                                            result_type: "object_create_result".to_string(),
                                            data: json!({"error": "创建项目需要提供 name"}),
                                            summary: Some("创建项目需要提供 name。".to_string()),
                                        };
                                        envelope.to_json()
                                    }
                                    else {
                                        let budget = a["budget"].as_f64().unwrap_or(0.0);
                                        let description = a["description"].as_str().unwrap_or("");
                                        let status = a["status"].as_str().unwrap_or("Lead");
                                        let bid_time = if status == "Bidding" { a["bid_time"].as_str().unwrap_or("") } else { "" };
                                        let data = json!({
                                            "name": name,
                                            "budget": budget,
                                            "description": description,
                                            "status": status,
                                            "bidTime": bid_time
                                        });
                                        // 使用 block_on 调用异步函数
                                        match block_on_async(crate::commands::project_commands::project_create(data)) {
                                            Ok(project) => {
                                                let pid = project["id"].as_str().unwrap_or("");
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "object_create_result".to_string(),
                                                    data: json!({"id": pid, "type": "PROJECT", "name": name}),
                                                    summary: Some(format!("已创建项目「{}」", name)),
                                                };
                                                envelope.to_json()
                                            },
                                            Err(e) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "object_create_result".to_string(),
                                                    data: json!({"error": e.to_string()}),
                                                    summary: Some(format!("创建项目失败: {}", e)),
                                                };
                                                envelope.to_json()
                                            },
                                        }
                                    }
                                }
                                "update" => {
                                    let raw_id = a["project_id"].as_str().unwrap_or("");
                                    let project_id = raw_id.strip_prefix("PROJECT-").unwrap_or(raw_id);
                                    if project_id.is_empty() {
                                        let envelope = ToolResultEnvelope {
                                            result_type: "object_update_result".to_string(),
                                            data: json!({"error": "更新项目需要提供 project_id"}),
                                            summary: Some("更新项目需要提供 project_id。".to_string()),
                                        };
                                        envelope.to_json()
                                    }
                                    else {
                                        let mut data = json!({});
                                        if let Some(v) = a["name"].as_str().filter(|s| !s.is_empty()) { data["name"] = json!(v); }
                                        if let Some(v) = a["description"].as_str().filter(|s| !s.is_empty()) { data["description"] = json!(v); }
                                        if let Some(v) = a["status"].as_str().filter(|s| !s.is_empty()) { data["status"] = json!(v); }
                                        if let Some(v) = a["bid_time"].as_str().filter(|s| !s.is_empty()) { data["bidTime"] = json!(v); }
                                        if let Some(v) = a["budget"].as_f64().filter(|v| *v > 0.0) { data["budget"] = json!(v); }
                                        if data.as_object().map_or(true, |m| m.is_empty()) {
                                            let envelope = ToolResultEnvelope {
                                                result_type: "object_update_result".to_string(),
                                                data: json!({"id": project_id, "error": "未提供任何更新字段"}),
                                                summary: Some("未提供任何更新字段。".to_string()),
                                            };
                                            envelope.to_json()
                                        } else {
                                            // 使用 block_on 调用异步函数
                                            match block_on_async(crate::commands::project_commands::project_update(project_id.to_string(), data)) {
                                                Ok(_) => {
                                                    let project_name = database::get(
                                                        "SELECT name FROM projects WHERE id=?1",
                                                        &[json!(project_id)],
                                                    ).ok().flatten().and_then(|r| r["name"].as_str().map(String::from)).unwrap_or_default();
                                                    let display_name = if project_name.is_empty() { project_id.to_string() } else { project_name };
                                                    let envelope = ToolResultEnvelope {
                                                        result_type: "object_update_result".to_string(),
                                                        data: json!({"id": project_id}),
                                                        summary: Some(format!("已更新项目「{}」", display_name)),
                                                    };
                                                    envelope.to_json()
                                                }
                                                Err(e) => {
                                                    let envelope = ToolResultEnvelope {
                                                        result_type: "object_update_result".to_string(),
                                                        data: json!({"id": project_id, "error": e.to_string()}),
                                                        summary: Some(format!("更新项目失败: {}", e)),
                                                    };
                                                    envelope.to_json()
                                                },
                                            }
                                        }
                                    }
                                }
                                "delete" => {
                                    let raw_id = a["project_id"].as_str().unwrap_or("");
                                    let project_id = raw_id.strip_prefix("PROJECT-").unwrap_or(raw_id);
                                    if project_id.is_empty() {
                                        let envelope = ToolResultEnvelope {
                                            result_type: "object_delete_result".to_string(),
                                            data: json!({"error": "删除项目需要提供 project_id"}),
                                            summary: Some("删除项目需要提供 project_id。".to_string()),
                                        };
                                        envelope.to_json()
                                    }
                                    else {
                                        // 使用 block_on 调用异步函数
                                        match block_on_async(crate::commands::project_commands::project_delete(project_id.to_string())) {
                                            Ok(_) => {
                                                let project_name = database::get(
                                                    "SELECT name FROM projects WHERE id=?1",
                                                    &[json!(project_id)],
                                                ).ok().flatten().and_then(|r| r["name"].as_str().map(String::from)).unwrap_or_default();
                                                let display_name = if project_name.is_empty() { project_id.to_string() } else { project_name };
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "object_delete_result".to_string(),
                                                    data: json!({"id": project_id}),
                                                    summary: Some(format!("已删除项目「{}」", display_name)),
                                                };
                                                envelope.to_json()
                                            }
                                            Err(e) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "object_delete_result".to_string(),
                                                    data: json!({"id": project_id, "error": e.to_string()}),
                                                    summary: Some(format!("删除项目失败: {}", e)),
                                                };
                                                envelope.to_json()
                                            },
                                        }
                                    }
                                }
                                "add_dynamic" => {
                                    let raw_id = a["project_id"].as_str().unwrap_or("");
                                    let project_id = raw_id.strip_prefix("PROJECT-").unwrap_or(raw_id);
                                    let content = a["content"].as_str().unwrap_or("");
                                    if project_id.is_empty() || content.is_empty() {
                                        let envelope = ToolResultEnvelope {
                                            result_type: "object_create_result".to_string(),
                                            data: json!({"error": "add_dynamic 需要提供 project_id 和 content"}),
                                            summary: Some("add_dynamic 需要提供 project_id 和 content。".to_string()),
                                        };
                                        envelope.to_json()
                                    } else {
                                        let dynamic_type = a["dynamic_type"].as_str().unwrap_or("ai");
                                        let event = json!({"type": dynamic_type, "content": content, "operator": "AI"});
                                        match block_on_async(crate::commands::project_commands::project_dynamics_add(project_id.to_string(), event)) {
                                            Ok(_) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "object_create_result".to_string(),
                                                    data: json!({"project_id": project_id, "type": dynamic_type}),
                                                    summary: Some(format!("已为项目追加动态记录")),
                                                };
                                                envelope.to_json()
                                            }
                                            Err(e) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "object_create_result".to_string(),
                                                    data: json!({"error": e.to_string()}),
                                                    summary: Some(format!("追加动态失败: {}", e)),
                                                };
                                                envelope.to_json()
                                            }
                                        }
                                    }
                                }
                                "delete_dynamic" => {
                                    let dynamic_id = a["dynamic_id"].as_str().unwrap_or("");
                                    if dynamic_id.is_empty() {
                                        let envelope = ToolResultEnvelope {
                                            result_type: "object_delete_result".to_string(),
                                            data: json!({"error": "delete_dynamic 需要提供 dynamic_id"}),
                                            summary: Some("delete_dynamic 需要提供 dynamic_id。".to_string()),
                                        };
                                        envelope.to_json()
                                    } else {
                                        match block_on_async(crate::commands::project_commands::project_dynamics_delete(dynamic_id.to_string())) {
                                            Ok(_) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "object_delete_result".to_string(),
                                                    data: json!({"dynamic_id": dynamic_id, "deleted": true}),
                                                    summary: Some("已删除动态记录。".to_string()),
                                                };
                                                envelope.to_json()
                                            }
                                            Err(e) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "object_delete_result".to_string(),
                                                    data: json!({"error": e.to_string()}),
                                                    summary: Some(format!("删除动态失败: {}", e)),
                                                };
                                                envelope.to_json()
                                            }
                                        }
                                    }
                                }
                                _ => {
                                    let envelope = ToolResultEnvelope {
                                        result_type: "object_error".to_string(),
                                        data: json!({"error": format!("未知操作「{}」。支持：create, update, delete, add_dynamic, delete_dynamic。", action)}),
                                        summary: Some(format!("未知操作「{}」。支持：create, update, delete, add_dynamic, delete_dynamic。", action)),
                                    };
                                    envelope.to_json()
                                },
                            };
    res
}