//! ext_tool_manage_contact：联系人增删改与关联执行器

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
                                            data: json!({"success": false, "error": "创建联系人需要提供 name"}),
                                            summary: Some("创建联系人需要提供 name。".to_string()),
                                        };
                                        envelope.to_json()
                                    }
                                    else {
                                        let phone = a["phone"].as_str().unwrap_or("");
                                        let email = a["email"].as_str().unwrap_or("");
                                        let position = a["position"].as_str().unwrap_or("");
                                        let remark = a["remark"].as_str().unwrap_or("");
                                        let parent_id = a["partner_id"].as_str()
                                            .or_else(|| a["parent_id"].as_str())
                                            .unwrap_or("");
                                        let raw_proj_id = a["project_id"].as_str().unwrap_or("");
                                        let project_id = raw_proj_id.strip_prefix("PROJECT-").unwrap_or(raw_proj_id);
                                        // ★ 去重检查：同名联系人（同一合作伙伴下）
                                        let mut is_dup = false;
                                        let mut dup_msg = String::new();
                                        if !parent_id.is_empty() {
                                            if let Ok(Some(existing)) = database::get(
                                                "SELECT pc.id, pc.name FROM partner_contacts pc JOIN partner_contact_mapping pcm ON pcm.contact_id=pc.id WHERE pc.name=?1 AND pcm.partner_id=?2 LIMIT 1",
                                                &[json!(name), json!(parent_id)],
                                            ) {
                                                let eid = existing["id"].as_str().unwrap_or("");
                                                is_dup = true;
                                                dup_msg = format!("已存在同名联系人「{}」(ID: {})，未重复创建。如需新建同名联系人请先修改名称。", name, eid);
                                            }
                                        }
                                        if is_dup {
                                            let envelope = ToolResultEnvelope {
                                                result_type: "object_create_result".to_string(),
                                                data: json!({"success": false, "error": dup_msg.clone()}),
                                                summary: Some(dup_msg),
                                            };
                                            envelope.to_json()
                                        } else {
                                        let data = json!({
                                            "name": name,
                                            "phone": phone,
                                            "email": email,
                                            "position": position,
                                            "remark": remark,
                                            "partnerId": parent_id
                                        });
                                        // 使用 block_on 调用异步函数
                                        match block_on_async(crate::commands::partner_commands::partner_contact_create(data)) {
                                            Ok(result) => {
                                                let id = result["id"].as_str().unwrap_or("");
                                                // 如果有项目ID，手动关联到项目
                                                if !project_id.is_empty() && !id.is_empty() {
                                                    let _ = block_on_async(crate::commands::partner_commands::contact_link_project(
                                                        id.to_string(),
                                                        project_id.to_string()
                                                    ));
                                                }
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "object_create_result".to_string(),
                                                    data: json!({"success": true, "id": id, "type": "CONTACT", "name": name}),
                                                    summary: Some(format!("已创建联系人「{}」", name)),
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
                                        } // close if is_dup else
                                    }
                                }
                                "update" => {
                                    let raw_cid = a["contact_id"].as_str().unwrap_or("");
                                    let contact_id = raw_cid.strip_prefix("CONTACT-").unwrap_or(raw_cid);
                                    if contact_id.is_empty() {
                                        let envelope = ToolResultEnvelope {
                                            result_type: "object_update_result".to_string(),
                                            data: json!({"success": false, "error": "更新需要提供 contact_id"}),
                                            summary: Some("更新需要提供 contact_id。".to_string()),
                                        };
                                        envelope.to_json()
                                    }
                                    else {
                                        let mut data = json!({"id": contact_id});
                                        if let Some(v) = a["name"].as_str().filter(|s| !s.is_empty()) { data["name"] = json!(v); }
                                        if let Some(v) = a["phone"].as_str().filter(|s| !s.is_empty()) { data["phone"] = json!(v); }
                                        if let Some(v) = a["email"].as_str().filter(|s| !s.is_empty()) { data["email"] = json!(v); }
                                        if let Some(v) = a["position"].as_str().filter(|s| !s.is_empty()) { data["position"] = json!(v); }
                                        if let Some(v) = a["remark"].as_str().filter(|s| !s.is_empty()) { data["remark"] = json!(v); }
                                        if data.as_object().map_or(true, |m| m.len() <= 1) {
                                            let envelope = ToolResultEnvelope {
                                                result_type: "object_update_result".to_string(),
                                                data: json!({"success": false, "id": contact_id, "error": "未提供任何更新字段"}),
                                                summary: Some("未提供任何更新字段。".to_string()),
                                            };
                                            envelope.to_json()
                                        } else {
                                            // 使用 block_on 调用异步函数
                                            match block_on_async(crate::commands::partner_commands::partner_contact_update(data)) {
                                                Ok(_) => {
                                                    let contact_name = database::get(
                                                        "SELECT name FROM partner_contacts WHERE id=?1",
                                                        &[json!(contact_id)],
                                                    ).ok().flatten().and_then(|r| r["name"].as_str().map(String::from)).unwrap_or_default();
                                                    let display_name = if contact_name.is_empty() { contact_id.to_string() } else { contact_name };
                                                    let envelope = ToolResultEnvelope {
                                                        result_type: "object_update_result".to_string(),
                                                        data: json!({"success": true, "id": contact_id}),
                                                        summary: Some(format!("已更新联系人「{}」", display_name)),
                                                    };
                                                    envelope.to_json()
                                                }
                                                Err(e) => {
                                                    let envelope = ToolResultEnvelope {
                                                        result_type: "object_update_result".to_string(),
                                                        data: json!({"success": false, "id": contact_id, "error": e.to_string()}),
                                                        summary: Some(format!("更新失败: {}", e)),
                                                    };
                                                    envelope.to_json()
                                                },
                                            }
                                        }
                                    }
                                }
                                "delete" => {
                                    let contact_id = a["contact_id"].as_str().unwrap_or("");
                                    if contact_id.is_empty() {
                                        let envelope = ToolResultEnvelope {
                                            result_type: "object_delete_result".to_string(),
                                            data: json!({"success": false, "error": "删除需要提供 contact_id"}),
                                            summary: Some("删除需要提供 contact_id。".to_string()),
                                        };
                                        envelope.to_json()
                                    }
                                    else {
                                        // 使用 block_on 调用异步函数
                                        match block_on_async(crate::commands::partner_commands::partner_contact_delete(contact_id.to_string())) {
                                            Ok(_) => {
                                                let contact_name = database::get(
                                                    "SELECT name FROM partner_contacts WHERE id=?1",
                                                    &[json!(contact_id)],
                                                ).ok().flatten().and_then(|r| r["name"].as_str().map(String::from)).unwrap_or_default();
                                                let display_name = if contact_name.is_empty() { contact_id.to_string() } else { contact_name };
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "object_delete_result".to_string(),
                                                    data: json!({"success": true, "id": contact_id}),
                                                    summary: Some(format!("已删除联系人「{}」", display_name)),
                                                };
                                                envelope.to_json()
                                            }
                                            Err(e) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "object_delete_result".to_string(),
                                                    data: json!({"success": false, "id": contact_id, "error": e.to_string()}),
                                                    summary: Some(format!("删除失败: {}", e)),
                                                };
                                                envelope.to_json()
                                            },
                                        }
                                    }
                                }
                                "link_project" => {
                                    let contact_id = a["contact_id"].as_str().unwrap_or("");
                                    let project_id = a["project_id"].as_str().unwrap_or("");
                                    if contact_id.is_empty() || project_id.is_empty() {
                                        let envelope = ToolResultEnvelope {
                                            result_type: "contact_link_result".to_string(),
                                            data: json!({"success": false}),
                                            summary: Some("关联需要提供 contact_id 和 project_id。".to_string()),
                                        };
                                        envelope.to_json()
                                    }
                                    else {
                                        // 使用 block_on 调用异步函数
                                        match block_on_async(crate::commands::partner_commands::contact_link_project(
                                            contact_id.to_string(),
                                            project_id.to_string()
                                        )) {
                                            Ok(_) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "contact_link_result".to_string(),
                                                    data: json!({"contact_id": contact_id, "project_id": project_id, "success": true}),
                                                    summary: Some("已将联系人关联到项目".to_string()),
                                                };
                                                envelope.to_json()
                                            }
                                            Err(e) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "contact_link_result".to_string(),
                                                    data: json!({"success": false}),
                                                    summary: Some(format!("关联失败: {}", e)),
                                                };
                                                envelope.to_json()
                                            },
                                        }
                                    }
                                }
                                "unlink_project" => {
                                    let contact_id = a["contact_id"].as_str().unwrap_or("");
                                    let project_id = a["project_id"].as_str().unwrap_or("");
                                    if contact_id.is_empty() || project_id.is_empty() {
                                        let envelope = ToolResultEnvelope {
                                            result_type: "contact_unlink_result".to_string(),
                                            data: json!({"success": false}),
                                            summary: Some("取消关联需要提供 contact_id 和 project_id。".to_string()),
                                        };
                                        envelope.to_json()
                                    }
                                    else {
                                        // 使用 block_on 调用异步函数
                                        match block_on_async(crate::commands::partner_commands::contact_unlink_project(
                                            contact_id.to_string(),
                                            project_id.to_string()
                                        )) {
                                            Ok(_) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "contact_unlink_result".to_string(),
                                                    data: json!({"contact_id": contact_id, "project_id": project_id, "success": true}),
                                                    summary: Some("已取消联系人与项目的关联".to_string()),
                                                };
                                                envelope.to_json()
                                            }
                                            Err(e) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "contact_unlink_result".to_string(),
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