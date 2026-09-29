//! ext_tool_manage_partner：合作伙伴增删改与项目关联执行器

use crate::database;
use serde_json::{json, Value};
use crate::services::agent_workflow::*;

pub(crate) fn execute(a: &Value) -> String {
    let a = a.clone();
                            let action = a["action"].as_str().unwrap_or("");
                            let res = match action {
                                "create" => {
                                    let ptype = a["type"].as_str().unwrap_or("");
                                    let name = a["name"].as_str().unwrap_or("");
                                    if ptype.is_empty() || name.is_empty() {
                                        let envelope = ToolResultEnvelope {
                                            result_type: "object_create_result".to_string(),
                                            data: json!({"success": false, "error": "创建合作伙伴需要提供 type 和 name"}),
                                            summary: Some("创建合作伙伴需要提供 type 和 name。".to_string()),
                                        };
                                        envelope.to_json()
                                    }
                                    else {
                                        // ★ 同名复用：同 type+name 精确命中直接复用（AI 重复建档/并发写入防护）
                                        if let Ok(Some(existing)) = database::get(
                                            "SELECT id, name FROM partners WHERE type=?1 AND name=?2 LIMIT 1",
                                            &[json!(ptype.to_uppercase()), json!(name)],
                                        ) {
                                            let eid = existing["id"].as_str().unwrap_or("");
                                            if !eid.is_empty() {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "object_create_result".to_string(),
                                                    data: json!({"success": true, "id": eid, "type": ptype.to_uppercase(), "name": name, "duplicate": true}),
                                                    summary: Some(format!("已存在{}「{}」，ID: {}，已复用，未重复创建。", ptype.to_uppercase(), name, eid)),
                                                };
                                                return envelope.to_json();
                                            }
                                        }
                                        let data = json!({
                                            "type": ptype.to_uppercase(),
                                            "name": name,
                                            "phone": a["phone"].as_str().unwrap_or(""),
                                            "address": a["address"].as_str().unwrap_or(""),
                                            "remark": a["remark"].as_str().unwrap_or(""),
                                            "contact_person": a["contact_person"].as_str().unwrap_or(""),
                                            "contact_phone": a["contact_phone"].as_str().unwrap_or(""),
                                            "projectId": a["project_id"].as_str().unwrap_or("")
                                        });
                                        // 使用 block_on 调用异步函数
                                        match block_on_async(crate::commands::partner_commands::partner_create(data)) {
                                            Ok(result) => {
                                                let id = result["id"].as_str().unwrap_or("");
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "object_create_result".to_string(),
                                                    data: json!({"success": true, "id": id, "type": ptype.to_uppercase(), "name": name}),
                                                    summary: Some(format!("已创建{}「{}」，ID: {}", ptype.to_uppercase(), name, id)),
                                                };
                                                envelope.to_json()
                                            },
                                            Err(e) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "object_create_result".to_string(),
                                                    data: json!({"success": false, "error": e.to_string()}),
                                                    summary: Some(format!("创建失败: {}", e)),
                                                };
                                                envelope.to_json()
                                            },
                                        }
                                    }
                                }
                                "update" => {
                                    let raw_pid = a["partner_id"].as_str().unwrap_or("");
                                    let partner_id = raw_pid.strip_prefix("CHANNEL-").or_else(|| raw_pid.strip_prefix("VENDOR-")).or_else(|| raw_pid.strip_prefix("CUSTOMER-")).unwrap_or(raw_pid);
                                    if partner_id.is_empty() {
                                        let envelope = ToolResultEnvelope {
                                            result_type: "object_update_result".to_string(),
                                            data: json!({"success": false, "error": "更新需要提供 partner_id"}),
                                            summary: Some("更新需要提供 partner_id。".to_string()),
                                        };
                                        envelope.to_json()
                                    }
                                    else {
                                        let mut data = json!({"id": partner_id});
                                        if let Some(v) = a["name"].as_str().filter(|s| !s.is_empty()) { data["name"] = json!(v); }
                                        if let Some(v) = a["phone"].as_str().filter(|s| !s.is_empty()) { data["phone"] = json!(v); }
                                        if let Some(v) = a["address"].as_str().filter(|s| !s.is_empty()) { data["address"] = json!(v); }
                                        if let Some(v) = a["remark"].as_str().filter(|s| !s.is_empty()) { data["remark"] = json!(v); }
                                        if data.as_object().map_or(true, |m| m.len() <= 1) {
                                            let envelope = ToolResultEnvelope {
                                                result_type: "object_update_result".to_string(),
                                                data: json!({"success": false, "id": partner_id, "error": "未提供任何更新字段"}),
                                                summary: Some("未提供任何更新字段。".to_string()),
                                            };
                                            envelope.to_json()
                                        } else {
                                            // 使用 block_on 调用异步函数
                                        match block_on_async(crate::commands::partner_commands::partner_update(data)) {
                                            Ok(_) => {
                                                let partner_name = database::get(
                                                    "SELECT name FROM partners WHERE id=?1",
                                                    &[json!(partner_id)],
                                                ).ok().flatten().and_then(|r| r["name"].as_str().map(String::from)).unwrap_or_default();
                                                let display_name = if partner_name.is_empty() { partner_id.to_string() } else { partner_name };
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "object_update_result".to_string(),
                                                    data: json!({"success": true, "id": partner_id}),
                                                    summary: Some(format!("已更新合作伙伴「{}」", display_name)),
                                                };
                                                envelope.to_json()
                                            }
                                            Err(e) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "object_update_result".to_string(),
                                                    data: json!({"success": false, "id": partner_id, "error": e.to_string()}),
                                                    summary: Some(format!("更新失败: {}", e)),
                                                };
                                                envelope.to_json()
                                            },
                                        }
                                        }
                                    }
                                }
                                "delete" => {
                                    let partner_id = a["partner_id"].as_str().unwrap_or("");
                                    if partner_id.is_empty() {
                                        let envelope = ToolResultEnvelope {
                                            result_type: "object_delete_result".to_string(),
                                            data: json!({"success": false, "error": "删除需要提供 partner_id"}),
                                            summary: Some("删除需要提供 partner_id。".to_string()),
                                        };
                                        envelope.to_json()
                                    }
                                    else {
                                        // 使用 block_on 调用异步函数
                                        match block_on_async(crate::commands::partner_commands::partner_delete(partner_id.to_string())) {
                                            Ok(_) => {
                                                let partner_name = database::get(
                                                    "SELECT name FROM partners WHERE id=?1",
                                                    &[json!(partner_id)],
                                                ).ok().flatten().and_then(|r| r["name"].as_str().map(String::from)).unwrap_or_default();
                                                let display_name = if partner_name.is_empty() { partner_id.to_string() } else { partner_name };
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "object_delete_result".to_string(),
                                                    data: json!({"success": true, "id": partner_id}),
                                                    summary: Some(format!("已删除合作伙伴「{}」", display_name)),
                                                };
                                                envelope.to_json()
                                            }
                                            Err(e) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "object_delete_result".to_string(),
                                                    data: json!({"success": false, "id": partner_id, "error": e.to_string()}),
                                                    summary: Some(format!("删除失败: {}", e)),
                                                };
                                                envelope.to_json()
                                            },
                                        }
                                    }
                                }
                                "link_project" => {
                                    let partner_id = a["partner_id"].as_str().unwrap_or("");
                                    let project_id = a["project_id"].as_str().unwrap_or("");
                                    if partner_id.is_empty() || project_id.is_empty() {
                                        let envelope = ToolResultEnvelope {
                                            result_type: "partner_link_result".to_string(),
                                            data: json!({"success": false}),
                                            summary: Some("关联需要提供 partner_id 和 project_id。".to_string()),
                                        };
                                        envelope.to_json()
                                    }
                                    else {
                                        let contact_id = a["contact_id"].as_str().map(|s| s.to_string());
                                        let remark = a["remark"].as_str().map(|s| s.to_string());
                                        // 使用 block_on 调用异步函数
                                        match block_on_async(crate::commands::partner_commands::partner_link_project(
                                            partner_id.to_string(),
                                            project_id.to_string(),
                                            contact_id,
                                            remark
                                        )) {
                                            Ok(_) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "partner_link_result".to_string(),
                                                    data: json!({"partner_id": partner_id, "project_id": project_id, "success": true}),
                                                    summary: Some("已将合作伙伴关联到项目".to_string()),
                                                };
                                                envelope.to_json()
                                            }
                                            Err(e) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "partner_link_result".to_string(),
                                                    data: json!({"success": false}),
                                                    summary: Some(format!("关联失败: {}", e)),
                                                };
                                                envelope.to_json()
                                            },
                                        }
                                    }
                                }
                                "unlink_project" => {
                                    let partner_id = a["partner_id"].as_str().unwrap_or("");
                                    let project_id = a["project_id"].as_str().unwrap_or("");
                                    if partner_id.is_empty() || project_id.is_empty() {
                                        let envelope = ToolResultEnvelope {
                                            result_type: "partner_unlink_result".to_string(),
                                            data: json!({"success": false}),
                                            summary: Some("取消关联需要提供 partner_id 和 project_id。".to_string()),
                                        };
                                        envelope.to_json()
                                    }
                                    else {
                                        // 使用 block_on 调用异步函数
                                        match block_on_async(crate::commands::partner_commands::partner_unlink_project(
                                            partner_id.to_string(),
                                            project_id.to_string()
                                        )) {
                                            Ok(_) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "partner_unlink_result".to_string(),
                                                    data: json!({"partner_id": partner_id, "project_id": project_id, "success": true}),
                                                    summary: Some("已取消合作伙伴与项目的关联".to_string()),
                                                };
                                                envelope.to_json()
                                            }
                                            Err(e) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "partner_unlink_result".to_string(),
                                                    data: json!({"success": false}),
                                                    summary: Some(format!("取消关联失败: {}", e)),
                                                };
                                                envelope.to_json()
                                            },
                                        }
                                    }
                                }
                                _ => {
                                    let envelope = ToolResultEnvelope {
                                        result_type: "object_error".to_string(),
                                        data: json!({"success": false, "error": format!("未知操作「{}」。支持：create, update, delete, link_project, unlink_project。", action)}),
                                        summary: Some(format!("未知操作「{}」。支持：create, update, delete, link_project, unlink_project。", action)),
                                    };
                                    envelope.to_json()
                                },
                            };
    res
}