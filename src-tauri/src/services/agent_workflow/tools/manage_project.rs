//! ext_tool_manage_project：项目增删改与动态执行器

use crate::database;
use serde_json::{json, Value};
use crate::services::agent_workflow::*;

/// 项目阶段归一：中文别名/大小写 → 9 值规范域（与前端 useProjectStatus.STATUS_OPTIONS 对齐）。
/// 无法识别的值原样保留（update 语义下不阻断其余字段写入）。
pub(crate) fn norm_status(s: &str) -> String {
    let t = s.trim();
    if t.is_empty() { return String::new(); }
    let low = t.to_lowercase();
    const PAIRS: [(&str, &[&str]); 9] = [
        ("Lead", &["lead", "线索"]),
        ("SolutionDesign", &["solutiondesign", "方案", "方案报价", "报价"]),
        ("PoC", &["poc", "测试"]),
        ("Bidding", &["bidding", "投标", "投标中"]),
        ("Won", &["won", "中标"]),
        ("Ordered", &["ordered", "下单", "已下单"]),
        ("Lost", &["lost", "丢单"]),
        ("OnHold", &["onhold", "搁置", "暂停"]),
        ("Archived", &["archived", "归档"]),
    ];
    for (canon, alts) in PAIRS {
        if alts.iter().any(|a| *a == low || *a == t) {
            return canon.to_string();
        }
    }
    t.to_string()
}

pub(crate) fn execute(a: &Value) -> String {
    let a = a.clone();
                            let action = a["action"].as_str().unwrap_or("");
                            let res = match action {
                                "create" => {
                                    let name = a["name"].as_str().unwrap_or("");
                                    if name.is_empty() {
                                        let envelope = ToolResultEnvelope {
                                            result_type: "object_create_result".to_string(),
                                            data: json!({"success": false, "error": "创建项目需要提供 name"}),
                                            summary: Some("创建项目需要提供 name。".to_string()),
                                        };
                                        envelope.to_json()
                                    }
                                    else {
                                        // ★ 同名复用：精确同名项目直接复用（AI 重复建档/并发写入防护）
                                        if let Ok(Some(existing)) = crate::database::get(
                                            "SELECT id, name FROM projects WHERE name=?1 ORDER BY created_at DESC LIMIT 1",
                                            &[json!(name)],
                                        ) {
                                            let eid = existing["id"].as_str().unwrap_or("");
                                            if !eid.is_empty() {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "object_create_result".to_string(),
                                                    data: json!({"success": true, "id": eid, "type": "PROJECT", "name": name, "duplicate": true}),
                                                    summary: Some(format!("已存在同名项目「{}」，ID: {}，已复用，未重复创建。", name, eid)),
                                                };
                                                return envelope.to_json();
                                            }
                                        }
                                        let budget = a["budget"].as_f64().unwrap_or(0.0);
                                        let description = a["description"].as_str().unwrap_or("");
                                        let status = norm_status(a["status"].as_str().unwrap_or("Lead"));

                                        let bid_in = a["bid_time"].as_str().unwrap_or("");
                                        let bid_time = if status == "Bidding" { bid_in } else { "" };
                                        let bid_dropped = !bid_in.trim().is_empty() && status != "Bidding";
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
                                                    data: json!({"success": true, "id": pid, "type": "PROJECT", "name": name}),
                                                    summary: Some(if bid_dropped {
                                                        format!("已创建项目「{}」，ID: {}。注意：bid_time 已忽略——投标时间仅在 status=Bidding（投标）时生效", name, pid)
                                                    } else {
                                                        format!("已创建项目「{}」，ID: {}", name, pid)
                                                    }),
                                                };
                                                envelope.to_json()
                                            },
                                            Err(e) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "object_create_result".to_string(),
                                                    data: json!({"success": false, "error": e.to_string()}),
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
                                            data: json!({"success": false, "error": "更新项目需要提供 project_id"}),
                                            summary: Some("更新项目需要提供 project_id。".to_string()),
                                        };
                                        envelope.to_json()
                                    }
                                    else {
                                        let mut data = json!({});
                                        if let Some(v) = a["name"].as_str().filter(|s| !s.is_empty()) { data["name"] = json!(v); }
                                        if let Some(v) = a["description"].as_str().filter(|s| !s.is_empty()) { data["description"] = json!(v); }
                                        if let Some(v) = a["status"].as_str().filter(|s| !s.is_empty()) { data["status"] = json!(norm_status(v)); }
                                        if let Some(v) = a["bid_time"].as_str().filter(|s| !s.is_empty()) { data["bidTime"] = json!(v); }
                                        if let Some(v) = a["budget"].as_f64().filter(|v| *v > 0.0) { data["budget"] = json!(v); }
                                        if data.as_object().map_or(true, |m| m.is_empty()) {
                                            let envelope = ToolResultEnvelope {
                                                result_type: "object_update_result".to_string(),
                                                data: json!({"success": false, "id": project_id, "error": "未提供任何更新字段"}),
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
                                                        data: json!({"success": true, "id": project_id}),
                                                        summary: Some(format!("已更新项目「{}」", display_name)),
                                                    };
                                                    envelope.to_json()
                                                }
                                                Err(e) => {
                                                    let envelope = ToolResultEnvelope {
                                                        result_type: "object_update_result".to_string(),
                                                        data: json!({"success": false, "id": project_id, "error": e.to_string()}),
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
                                            data: json!({"success": false, "error": "删除项目需要提供 project_id"}),
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
                                                    data: json!({"success": true, "id": project_id}),
                                                    summary: Some(format!("已删除项目「{}」", display_name)),
                                                };
                                                envelope.to_json()
                                            }
                                            Err(e) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "object_delete_result".to_string(),
                                                    data: json!({"success": false, "id": project_id, "error": e.to_string()}),
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
                                            data: json!({"success": false, "error": "add_dynamic 需要提供 project_id 和 content"}),
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
                                                    data: json!({"success": true, "project_id": project_id, "type": dynamic_type}),
                                                    summary: Some(format!("已为项目追加动态记录")),
                                                };
                                                envelope.to_json()
                                            }
                                            Err(e) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "object_create_result".to_string(),
                                                    data: json!({"success": false, "error": e.to_string()}),
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
                                            data: json!({"success": false, "error": "delete_dynamic 需要提供 dynamic_id"}),
                                            summary: Some("delete_dynamic 需要提供 dynamic_id。".to_string()),
                                        };
                                        envelope.to_json()
                                    } else {
                                        match block_on_async(crate::commands::project_commands::project_dynamics_delete(dynamic_id.to_string())) {
                                            Ok(_) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "object_delete_result".to_string(),
                                                    data: json!({"success": true, "dynamic_id": dynamic_id, "deleted": true}),
                                                    summary: Some("已删除动态记录。".to_string()),
                                                };
                                                envelope.to_json()
                                            }
                                            Err(e) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "object_delete_result".to_string(),
                                                    data: json!({"success": false, "error": e.to_string()}),
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
                                        data: json!({"success": false, "error": format!("未知操作「{}」。支持：create, update, delete, add_dynamic, delete_dynamic。", action)}),
                                        summary: Some(format!("未知操作「{}」。支持：create, update, delete, add_dynamic, delete_dynamic。", action)),
                                    };
                                    envelope.to_json()
                                },
                            };
    res
}

#[cfg(test)]
mod tests {
    use super::norm_status;

    #[test]
    fn status_aliases_normalize() {
        for v in ["bidding", "Bidding", "BIDDING", "投标", "投标中"] {
            assert_eq!(norm_status(v), "Bidding", "{}", v);
        }
        assert_eq!(norm_status("线索"), "Lead");
        assert_eq!(norm_status(" poc "), "PoC");
        assert_eq!(norm_status("已下单"), "Ordered");
        assert_eq!(norm_status("自定义阶段"), "自定义阶段");
        assert_eq!(norm_status("  "), "");
    }
}
