//! ext_tool_manage_policy：用户策略增删改查执行器

use crate::database;
use serde_json::{json, Value};
use crate::services::agent_workflow::*;

pub(crate) fn execute(a: &Value) -> String {
    let a = a.clone();
                            let action = a["action"].as_str().unwrap_or("");
                            let id = a["id"].as_str().map(|s| s.to_string());
                            let content = a["content"].as_str().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
                            let res = match action {
                                "update" => {
                                    if id.is_none() || content.is_none() {
                                        let e = ToolResultEnvelope { result_type: "policy_update_result".to_string(), data: json!({"success": false}), summary: Some("更新需要提供 id 和 content。".to_string()) };
                                        e.to_json()
                                    } else {
                                        let pid = id.as_ref().unwrap();
                                        let c = content.as_ref().unwrap();
                                        let real_id = pid.strip_prefix("policy-").unwrap_or(pid);
                                        let r = database::run("UPDATE policies SET content=?1, updated_at=datetime('now','localtime') WHERE id=?2", &[json!(c), json!(real_id)]);
                                        match r {
                                            Ok(r) if r.changes > 0 => {
                                                let envelope = ToolResultEnvelope { result_type: "policy_update_result".to_string(), data: json!({"id": real_id, "success": true}), summary: Some(format!("已更新策略：{}", c)) };
                                                envelope.to_json()
                                            }
                                            Ok(_) => {
                                                let envelope = ToolResultEnvelope { result_type: "policy_update_result".to_string(), data: json!({"id": real_id, "success": false}), summary: Some(format!("未找到策略 ID：{}", real_id)) };
                                                envelope.to_json()
                                            }
                                            Err(e) => {
                                                let envelope = ToolResultEnvelope { result_type: "policy_update_result".to_string(), data: json!({"success": false}), summary: Some(format!("更新失败：{}", e)) };
                                                envelope.to_json()
                                            }
                                        }
                                    }
                                }
                                "delete" => {
                                    if id.is_none() {
                                        let e = ToolResultEnvelope { result_type: "policy_delete_result".to_string(), data: json!({"success": false}), summary: Some("删除需要提供 id。".to_string()) };
                                        e.to_json()
                                    } else {
                                        let pid = id.as_ref().unwrap();
                                        let real_id = pid.strip_prefix("policy-").unwrap_or(pid);
                                        let r = database::run("DELETE FROM policies WHERE id=?1", &[json!(real_id)]);
                                        match r {
                                            Ok(r) if r.changes > 0 => {
                                                let envelope = ToolResultEnvelope { result_type: "policy_delete_result".to_string(), data: json!({"id": real_id, "success": true}), summary: Some("已删除策略".to_string()) };
                                                envelope.to_json()
                                            }
                                            Ok(_) => {
                                                let envelope = ToolResultEnvelope { result_type: "policy_delete_result".to_string(), data: json!({"id": real_id, "success": false}), summary: Some(format!("未找到策略 ID：{}", real_id)) };
                                                envelope.to_json()
                                            }
                                            Err(e) => {
                                                let envelope = ToolResultEnvelope { result_type: "policy_delete_result".to_string(), data: json!({"success": false}), summary: Some(format!("删除失败：{}", e)) };
                                                envelope.to_json()
                                            }
                                        }
                                    }
                                }
                                _ => { // create
                                    if content.is_none() {
                                        let e = ToolResultEnvelope { result_type: "policy_create_result".to_string(), data: json!({"success": false}), summary: Some("创建需要提供 content。".to_string()) };
                                        e.to_json()
                                    } else {
                                        let c = content.as_ref().unwrap();
                                        let new_id = uuid::Uuid::new_v4().to_string();
                                        let r = database::run("INSERT INTO policies(id,content,updated_at) VALUES(?1,?2,datetime('now','localtime'))", &[json!(&new_id), json!(c)]);
                                    match r {
                                        Ok(_) => {
                                            let envelope = ToolResultEnvelope {
                                                result_type: "policy_create_result".to_string(),
                                                data: json!({"id": new_id, "success": true}),
                                                 summary: Some(format!("已创建策略：{}", content.as_ref().unwrap())),
                                            };
                                            envelope.to_json()
                                        }
                                        Err(e) => {
                                            let envelope = ToolResultEnvelope {
                                                result_type: "policy_create_result".to_string(),
                                                data: json!({"success": false}),
                                                summary: Some(format!("创建失败：{}", e)),
                                            };
                                            envelope.to_json()
                                        }
                                    }
                                    }
                                }
                            };
    res
}