//! ext_tool_manage_partner：合作伙伴增删改与项目关联执行器

use crate::database;
use serde_json::{json, Value};
use crate::services::agent_workflow::*;
use super::common::{resolve_project_id, normalize_ref};

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
                                        // ★ project_id 归一：模型世界以 PROJECT-uuid 引用形态传递（entity_ref 规范），
                                        //   映射表存裸 uuid——解析失败直接报错，绝不静默写垃圾映射（垃圾行=前端按裸 uuid 查不到→关联单向失明）
                                        let raw_pid = a["project_id"].as_str().unwrap_or("").trim().to_string();
                                        let pid_resolved: Option<String> = if raw_pid.is_empty() {
                                            None
                                        } else {
                                            match resolve_project_id(&raw_pid) {
                                                Some(p) => Some(p),
                                                None => {
                                                    let envelope = ToolResultEnvelope {
                                                        result_type: "object_create_result".to_string(),
                                                        data: json!({"success": false, "error": format!("未找到项目「{}」", raw_pid)}),
                                                        summary: Some(format!("未找到项目「{}」，创建已中止：请先用「数据查询」确认项目获取 ID 后再关联。", raw_pid)),
                                                    };
                                                    return envelope.to_json();
                                                }
                                            }
                                        };
                                        // ★ 同名复用：同 type+name 精确命中直接复用（AI 重复建档/并发写入防护）
                                        if let Ok(Some(existing)) = database::get(
                                            "SELECT id, name FROM partners WHERE type=?1 AND name=?2 LIMIT 1",
                                            &[json!(ptype.to_uppercase()), json!(name)],
                                        ) {
                                            let eid = existing["id"].as_str().unwrap_or("");
                                            if !eid.is_empty() {
                                                // ★ 复用路径此前静默丢弃 project_id（关联悄悄丢失）——同样落关联
                                                let mut linked = String::new();
                                                if let Some(ref pid) = pid_resolved {
                                                    database::run(
                                                        "INSERT OR IGNORE INTO partner_project_mapping(partner_id, project_id, partner_type, remark) VALUES(?, ?, ?, '')",
                                                        &[json!(eid), json!(pid), json!(ptype.to_uppercase())],
                                                    ).ok();
                                                    crate::services::business_relations::resync_partner_edges(eid);
                                                    linked = format!("，并已关联到项目 {}", pid);
                                                }
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "object_create_result".to_string(),
                                                    data: json!({"success": true, "id": eid, "type": ptype.to_uppercase(), "name": name, "duplicate": true}),
                                                    summary: Some(format!("已存在{}「{}」，ID: {}，已复用，未重复创建。{}", ptype.to_uppercase(), name, eid, linked)),
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
                                            "projectId": pid_resolved.clone().unwrap_or_default()
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
                                    let raw_pid = a["partner_id"].as_str().unwrap_or("").trim();
                                    let partner_id = match normalize_ref(raw_pid, &["CHANNEL", "VENDOR", "CUSTOMER"]) {
                                        Ok(p) => p,
                                        Err(e) => {
                                            let envelope = ToolResultEnvelope {
                                                result_type: "object_update_result".to_string(),
                                                data: json!({"success": false, "error": e.clone()}),
                                                summary: Some(format!("{}。", e)),
                                            };
                                            return envelope.to_json();
                                        }
                                    };
                                    if partner_id.is_empty() {
                                        let envelope = ToolResultEnvelope {
                                            result_type: "object_update_result".to_string(),
                                            data: json!({"success": false, "error": "更新需要提供 partner_id"}),
                                            summary: Some("更新需要提供 partner_id。".to_string()),
                                        };
                                        envelope.to_json()
                                    }
                                    else {
                                        let mut data = json!({"id": &partner_id});
                                        if let Some(v) = a["name"].as_str().filter(|s| !s.is_empty()) { data["name"] = json!(v); }
                                        if let Some(v) = a["phone"].as_str().filter(|s| !s.is_empty()) { data["phone"] = json!(v); }
                                        if let Some(v) = a["address"].as_str().filter(|s| !s.is_empty()) { data["address"] = json!(v); }
                                        if let Some(v) = a["remark"].as_str().filter(|s| !s.is_empty()) { data["remark"] = json!(v); }
                                        if data.as_object().map_or(true, |m| m.len() <= 1) {
                                            let envelope = ToolResultEnvelope {
                                                result_type: "object_update_result".to_string(),
                                                data: json!({"success": false, "id": &partner_id, "error": "未提供任何更新字段"}),
                                                summary: Some("未提供任何更新字段。".to_string()),
                                            };
                                            envelope.to_json()
                                        } else {
                                            // 使用 block_on 调用异步函数
                                        match block_on_async(crate::commands::partner_commands::partner_update(data)) {
                                            Ok(_) => {
                                                let partner_name = database::get(
                                                    "SELECT name FROM partners WHERE id=?1",
                                                    &[json!(&partner_id)],
                                                ).ok().flatten().and_then(|r| r["name"].as_str().map(String::from)).unwrap_or_default();
                                                let display_name = if partner_name.is_empty() { partner_id.clone() } else { partner_name };
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "object_update_result".to_string(),
                                                    data: json!({"success": true, "id": &partner_id}),
                                                    summary: Some(format!("已更新合作伙伴「{}」", display_name)),
                                                };
                                                envelope.to_json()
                                            }
                                            Err(e) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "object_update_result".to_string(),
                                                    data: json!({"success": false, "id": &partner_id, "error": e.to_string()}),
                                                    summary: Some(format!("更新失败: {}", e)),
                                                };
                                                envelope.to_json()
                                            },
                                        }
                                        }
                                    }
                                }
                                "delete" => {
                                    let raw_pid = a["partner_id"].as_str().unwrap_or("").trim();
                                    let partner_id = match normalize_ref(raw_pid, &["CHANNEL", "VENDOR", "CUSTOMER"]) {
                                        Ok(p) => p,
                                        Err(e) => {
                                            let envelope = ToolResultEnvelope {
                                                result_type: "object_delete_result".to_string(),
                                                data: json!({"success": false, "error": e.clone()}),
                                                summary: Some(format!("{}。", e)),
                                            };
                                            return envelope.to_json();
                                        }
                                    };
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
                                        match block_on_async(crate::commands::partner_commands::partner_delete(partner_id.clone())) {
                                            Ok(_) => {
                                                let partner_name = database::get(
                                                    "SELECT name FROM partners WHERE id=?1",
                                                    &[json!(&partner_id)],
                                                ).ok().flatten().and_then(|r| r["name"].as_str().map(String::from)).unwrap_or_default();
                                                let display_name = if partner_name.is_empty() { partner_id.clone() } else { partner_name };
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "object_delete_result".to_string(),
                                                    data: json!({"success": true, "id": &partner_id}),
                                                    summary: Some(format!("已删除合作伙伴「{}」", display_name)),
                                                };
                                                envelope.to_json()
                                            }
                                            Err(e) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "object_delete_result".to_string(),
                                                    data: json!({"success": false, "id": &partner_id, "error": e.to_string()}),
                                                    summary: Some(format!("删除失败: {}", e)),
                                                };
                                                envelope.to_json()
                                            },
                                        }
                                    }
                                }
                                "link_project" => {
                                    let raw_partner = a["partner_id"].as_str().unwrap_or("").trim();
                                    let raw_project = a["project_id"].as_str().unwrap_or("").trim();
                                    if raw_partner.is_empty() || raw_project.is_empty() {
                                        let envelope = ToolResultEnvelope {
                                            result_type: "partner_link_result".to_string(),
                                            data: json!({"success": false}),
                                            summary: Some("关联需要提供 partner_id 和 project_id。".to_string()),
                                        };
                                        envelope.to_json()
                                    }
                                    else {
                                        // ★ id 归一：模型传 VENDOR-xxx/PROJECT-xxx 引用形态是常态（entity_ref 规范），
                                        //   映射表只认裸 uuid——类型不符/项目解析失败一律报错，绝不静默写垃圾映射
                                        let partner_id = match normalize_ref(raw_partner, &["CHANNEL", "VENDOR", "CUSTOMER"]) {
                                            Ok(p) => p,
                                            Err(e) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "partner_link_result".to_string(),
                                                    data: json!({"success": false}),
                                                    summary: Some(format!("{}。", e)),
                                                };
                                                return envelope.to_json();
                                            }
                                        };
                                        let project_id = match resolve_project_id(raw_project) {
                                            Some(p) => p,
                                            None => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "partner_link_result".to_string(),
                                                    data: json!({"success": false}),
                                                    summary: Some(format!("未找到项目「{}」，请先用「数据查询」确认项目获取 ID 后再关联。", raw_project)),
                                                };
                                                return envelope.to_json();
                                            }
                                        };
                                        let contact_id = match a["contact_id"].as_str().map(|s| s.trim()).filter(|s| !s.is_empty()) {
                                            Some(raw_c) => match normalize_ref(raw_c, &["CONTACT"]) {
                                                Ok(c) => Some(c),
                                                Err(e) => {
                                                    let envelope = ToolResultEnvelope {
                                                        result_type: "partner_link_result".to_string(),
                                                        data: json!({"success": false}),
                                                        summary: Some(format!("{}。", e)),
                                                    };
                                                    return envelope.to_json();
                                                }
                                            },
                                            None => None,
                                        };
                                        let remark = a["remark"].as_str().map(|s| s.to_string());
                                        // 使用 block_on 调用异步函数
                                        match block_on_async(crate::commands::partner_commands::partner_link_project(
                                            partner_id.clone(),
                                            project_id.clone(),
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
                                    let raw_partner = a["partner_id"].as_str().unwrap_or("").trim();
                                    let raw_project = a["project_id"].as_str().unwrap_or("").trim();
                                    if raw_partner.is_empty() || raw_project.is_empty() {
                                        let envelope = ToolResultEnvelope {
                                            result_type: "partner_unlink_result".to_string(),
                                            data: json!({"success": false}),
                                            summary: Some("取消关联需要提供 partner_id 和 project_id。".to_string()),
                                        };
                                        envelope.to_json()
                                    }
                                    else {
                                        // ★ 严格归一：仅剥 entity_ref 前缀精确匹配，不走名称模糊回退（防按名误删别家关联）
                                        let partner_id = match normalize_ref(raw_partner, &["CHANNEL", "VENDOR", "CUSTOMER"]) {
                                            Ok(p) => p,
                                            Err(e) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "partner_unlink_result".to_string(),
                                                    data: json!({"success": false}),
                                                    summary: Some(format!("{}。", e)),
                                                };
                                                return envelope.to_json();
                                            }
                                        };
                                        let project_id = match normalize_ref(raw_project, &["PROJECT"]) {
                                            Ok(p) => p,
                                            Err(e) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "partner_unlink_result".to_string(),
                                                    data: json!({"success": false}),
                                                    summary: Some(format!("{}。", e)),
                                                };
                                                return envelope.to_json();
                                            }
                                        };
                                        let existed = database::get(
                                            "SELECT 1 FROM partner_project_mapping WHERE partner_id=?1 AND project_id=?2",
                                            &[json!(&partner_id), json!(&project_id)],
                                        ).ok().flatten().is_some();
                                        // 使用 block_on 调用异步函数
                                        match block_on_async(crate::commands::partner_commands::partner_unlink_project(
                                            partner_id.clone(),
                                            project_id.clone()
                                        )) {
                                            Ok(_) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "partner_unlink_result".to_string(),
                                                    data: json!({"partner_id": partner_id, "project_id": project_id, "success": true}),
                                                    summary: Some(if existed { "已取消合作伙伴与项目的关联".to_string() } else { "未找到对应关联记录（可能已不存在，或与建立时的项目标识不一致）。".to_string() }),
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