//! ext_tool_manage_product：产品与目录增删改执行器

use crate::database;
use serde_json::{json, Value};
use crate::services::agent_workflow::*;

pub(crate) fn ext_tool_manage_product(a: &Value) -> String {
    let action = a["action"].as_str().unwrap_or("");

    // 辅助：按 UUID 或名称查找产品
    let find_product = |pid: &str, pname: &str| -> Option<(String, String)> {
        if !pid.trim().is_empty() {
            let clean_id = pid.trim().strip_prefix("PRODUCT-").unwrap_or(pid.trim());
            database::get("SELECT id,name FROM products WHERE id=?1", &[json!(clean_id)])
                .ok().flatten().map(|r| (r["id"].as_str().unwrap_or("").to_string(), r["name"].as_str().unwrap_or("").to_string()))
        } else if !pname.trim().is_empty() {
            let like = format!("%{}%", crate::database::escape_like(pname.trim()));
            database::get("SELECT id,name FROM products WHERE name LIKE ?1 ESCAPE '\\' ORDER BY created_at DESC LIMIT 1", &[json!(&like)])
                .ok().flatten().map(|r| (r["id"].as_str().unwrap_or("").to_string(), r["name"].as_str().unwrap_or("").to_string()))
        } else { None }
    };

    // 辅助：按 UUID 或名称查找目录
    let find_series = |sid: &str, sname: &str| -> Option<(String, String)> {
        if !sid.trim().is_empty() {
            let clean_id = sid.trim().strip_prefix("PRODUCT-").unwrap_or(sid.trim());
            database::get("SELECT id,name FROM product_series WHERE id=?1", &[json!(clean_id)])
                .ok().flatten().map(|r| (r["id"].as_str().unwrap_or("").to_string(), r["name"].as_str().unwrap_or("").to_string()))
        } else if !sname.trim().is_empty() {
            database::get("SELECT id,name FROM product_series WHERE name=?1", &[json!(sname.trim())])
                .ok().flatten().map(|r| (r["id"].as_str().unwrap_or("").to_string(), r["name"].as_str().unwrap_or("").to_string()))
        } else { None }
    };

    match action {
        "create" => {
            let name = a["name"].as_str().unwrap_or("");
            if name.is_empty() {
                let envelope = ToolResultEnvelope {
                    result_type: "object_create_result".to_string(),
                    data: json!({"success": false}),
                    summary: Some("创建产品需要提供 name。".to_string()),
                };
                return envelope.to_json();
            }
            let series_id_val = if let Some((sid, _)) = find_series(
                a["series_id"].as_str().unwrap_or(""),
                a["series_name"].as_str().unwrap_or("")
            ) { Some(sid) } else { None };
            let mut data = json!({
                "name": name,
                "description": a["description"].as_str().unwrap_or(""),
                "price": a["price"].as_f64().unwrap_or(0.0),
                "fullSpecs": a["full_specs"].as_str().unwrap_or("")
            });
            if let Some(sid) = series_id_val { data["seriesId"] = json!(sid); }
            match block_on_async(crate::commands::product_commands::product_create(data)) {
                Ok(created) => {
                    let pid = created["id"].as_str().unwrap_or("");
                    let pname = created["name"].as_str().unwrap_or("");
                    let envelope = ToolResultEnvelope {
                        result_type: "object_create_result".to_string(),
                        data: json!({"success": true, "id": pid, "name": pname}),
                        summary: Some(format!("已创建产品「{}」，ID: {}", pname, pid)),
                    };
                    envelope.to_json()
                }
                Err(e) => {
                    let envelope = ToolResultEnvelope {
                        result_type: "object_create_result".to_string(),
                        data: json!({"success": false}),
                        summary: Some(format!("创建产品失败: {}", e)),
                    };
                    envelope.to_json()
                }
            }
        }
        "update" => {
            let product_id = a["product_id"].as_str().unwrap_or("");
            let product_name = a["name"].as_str().unwrap_or("");
            let target_name = if product_id.is_empty() && !product_name.is_empty() { product_name } else { "" };
            match find_product(product_id, target_name) {
                None => {
                    let envelope = ToolResultEnvelope {
                        result_type: "object_update_result".to_string(),
                        data: json!({"success": false}),
                        summary: Some("未找到指定产品，请提供 product_id 或准确的产品名称。".to_string()),
                    };
                    envelope.to_json()
                }
                Some((fid, _)) => {
                    let mut data = json!({});
                    if let Some(v) = a["name"].as_str().filter(|s| !s.is_empty()) { data["name"] = json!(v); }
                    if let Some(v) = a["description"].as_str().filter(|s| !s.is_empty()) { data["description"] = json!(v); }
                    if a["price"].as_f64().is_some() { data["price"] = json!(a["price"].as_f64().unwrap()); }
                    if let Some(v) = a["full_specs"].as_str().filter(|s| !s.is_empty()) { data["fullSpecs"] = json!(v); }
                    if let Some((sid, _)) = find_series(a["series_id"].as_str().unwrap_or(""), a["series_name"].as_str().unwrap_or("")) {
                        data["seriesId"] = json!(sid);
                    }
                    match block_on_async(crate::commands::product_commands::product_update(fid.clone(), data)) {
                        Ok(_) => {
                            let envelope = ToolResultEnvelope {
                                result_type: "object_update_result".to_string(),
                                data: json!({"success": true, "id": fid}),
                                summary: Some("产品已更新。".to_string()),
                            };
                            envelope.to_json()
                        }
                        Err(e) => {
                            let envelope = ToolResultEnvelope {
                                result_type: "object_update_result".to_string(),
                                data: json!({"success": false}),
                                summary: Some(format!("更新产品失败: {}", e)),
                            };
                            envelope.to_json()
                        }
                    }
                }
            }
        }
        "delete" => {
            let product_id = a["product_id"].as_str().unwrap_or("");
            let product_name = a["name"].as_str().unwrap_or("");
            let target_name = if product_id.is_empty() && !product_name.is_empty() { product_name } else { "" };
            match find_product(product_id, target_name) {
                None => {
                    let envelope = ToolResultEnvelope {
                        result_type: "object_delete_result".to_string(),
                        data: json!({"success": false}),
                        summary: Some("未找到指定产品。".to_string()),
                    };
                    envelope.to_json()
                }
                Some((fid, fname)) => {
                    match block_on_async(crate::commands::product_commands::product_delete(fid)) {
                        Ok(_) => {
                            let envelope = ToolResultEnvelope {
                                result_type: "object_delete_result".to_string(),
                                data: json!({"success": true}),
                                summary: Some(format!("已删除产品「{}」。", fname)),
                            };
                            envelope.to_json()
                        }
                        Err(e) => {
                            let envelope = ToolResultEnvelope {
                                result_type: "object_delete_result".to_string(),
                                data: json!({"success": false}),
                                summary: Some(format!("删除产品失败: {}", e)),
                            };
                            envelope.to_json()
                        }
                    }
                }
            }
        }
        "move" => {
            let product_id = a["product_id"].as_str().unwrap_or("");
            let product_name = a["name"].as_str().unwrap_or("");
            let target_name = if product_id.is_empty() && !product_name.is_empty() { product_name } else { "" };
            match find_product(product_id, target_name) {
                None => {
                    let envelope = ToolResultEnvelope {
                        result_type: "object_update_result".to_string(),
                        data: json!({"success": false}),
                        summary: Some("未找到指定产品。".to_string()),
                    };
                    envelope.to_json()
                }
                Some((fid, fname)) => {
                    let series_id_val = if let Some((sid, _)) = find_series(
                        a["series_id"].as_str().unwrap_or(""),
                        a["series_name"].as_str().unwrap_or("")
                    ) { sid } else {
                        let envelope = ToolResultEnvelope {
                            result_type: "object_update_result".to_string(),
                            data: json!({"success": false}),
                            summary: Some("未找到目标目录，请提供 series_id 或 series_name。".to_string()),
                        };
                        return envelope.to_json();
                    };
                    let data = json!({"seriesId": series_id_val});
                    match block_on_async(crate::commands::product_commands::product_update(fid, data)) {
                        Ok(_) => {
                            let envelope = ToolResultEnvelope {
                                result_type: "object_update_result".to_string(),
                                data: json!({"success": true}),
                                summary: Some(format!("已将产品「{}」移动到目标目录。", fname)),
                            };
                            envelope.to_json()
                        }
                        Err(e) => {
                            let envelope = ToolResultEnvelope {
                                result_type: "object_update_result".to_string(),
                                data: json!({"success": false}),
                                summary: Some(format!("移动产品失败: {}", e)),
                            };
                            envelope.to_json()
                        }
                    }
                }
            }
        }
        "create_series" => {
            let name = a["name"].as_str().unwrap_or("");
            if name.is_empty() {
                let envelope = ToolResultEnvelope {
                    result_type: "object_create_result".to_string(),
                    data: json!({"success": false}),
                    summary: Some("创建目录需要提供 name。".to_string()),
                };
                return envelope.to_json();
            }
            let mut data = json!({"name": name});
            if let Some(pid) = a["parent_id"].as_str().filter(|s| !s.is_empty()) {
                data["parentId"] = json!(pid);
            }
            match block_on_async(crate::commands::product_commands::product_series_create(data)) {
                Ok(created) => {
                    let sid = created["id"].as_str().unwrap_or("");
                    let sname = created["name"].as_str().unwrap_or("");
                    let envelope = ToolResultEnvelope {
                        result_type: "object_create_result".to_string(),
                        data: json!({"success": true, "id": sid, "name": sname}),
                        summary: Some(format!("已创建目录「{}」，ID: {}", sname, sid)),
                    };
                    envelope.to_json()
                }
                Err(e) => {
                    let envelope = ToolResultEnvelope {
                        result_type: "object_create_result".to_string(),
                        data: json!({"success": false}),
                        summary: Some(format!("创建目录失败: {}", e)),
                    };
                    envelope.to_json()
                }
            }
        }
        "update_series" => {
            let series_id = a["series"].as_str()
                .or_else(|| a["series_id"].as_str())
                .unwrap_or("");
            let series_name = a["series_name"].as_str().unwrap_or("");
            match find_series(series_id, series_name) {
                None => {
                    let envelope = ToolResultEnvelope {
                        result_type: "object_update_result".to_string(),
                        data: json!({"success": false}),
                        summary: Some("未找到指定目录。".to_string()),
                    };
                    envelope.to_json()
                }
                Some((sid, _)) => {
                    let mut data = json!({});
                    if let Some(v) = a["name"].as_str().filter(|s| !s.is_empty()) { data["name"] = json!(v); }
                    match block_on_async(crate::commands::product_commands::product_series_update(sid, data)) {
                        Ok(_) => {
                            let envelope = ToolResultEnvelope {
                                result_type: "object_update_result".to_string(),
                                data: json!({"success": true}),
                                summary: Some("目录已更新。".to_string()),
                            };
                            envelope.to_json()
                        }
                        Err(e) => {
                            let envelope = ToolResultEnvelope {
                                result_type: "object_update_result".to_string(),
                                data: json!({"success": false}),
                                summary: Some(format!("更新目录失败: {}", e)),
                            };
                            envelope.to_json()
                        }
                    }
                }
            }
        }
        "delete_series" => {
            let series_id = a["series"].as_str()
                .or_else(|| a["series_id"].as_str())
                .unwrap_or("");
            let series_name = a["series_name"].as_str().unwrap_or("");
            match find_series(series_id, series_name) {
                None => {
                    let envelope = ToolResultEnvelope {
                        result_type: "object_delete_result".to_string(),
                        data: json!({"success": false}),
                        summary: Some("未找到指定目录。".to_string()),
                    };
                    envelope.to_json()
                }
                Some((sid, sname)) => {
                    match block_on_async(crate::commands::product_commands::product_series_delete(sid)) {
                        Ok(result) => {
                            let success = result["success"].as_bool().unwrap_or(false);
                            if success {
                                let envelope = ToolResultEnvelope {
                                    result_type: "object_delete_result".to_string(),
                                    data: json!({"success": true}),
                                    summary: Some(format!("已删除目录「{}」。", sname)),
                                };
                                envelope.to_json()
                            } else {
                                let err = result["error"].as_str().unwrap_or("删除失败");
                                let envelope = ToolResultEnvelope {
                                    result_type: "object_delete_result".to_string(),
                                    data: json!({"success": false}),
                                    summary: Some(format!("删除目录失败: {}", err)),
                                };
                                envelope.to_json()
                            }
                        }
                        Err(e) => {
                            let envelope = ToolResultEnvelope {
                                result_type: "object_delete_result".to_string(),
                                data: json!({"success": false}),
                                summary: Some(format!("删除目录失败: {}", e)),
                            };
                            envelope.to_json()
                        }
                    }
                }
            }
        }
        _ => {
            let envelope = ToolResultEnvelope {
                result_type: "object_error".to_string(),
                data: json!({"error": format!("未知操作「{}」。支持：create, update, delete, move, create_series, update_series, delete_series。", action)}),
                summary: Some(format!("未知操作「{}」。支持：create, update, delete, move, create_series, update_series, delete_series。", action)),
            };
            envelope.to_json()
        }
    }
}

