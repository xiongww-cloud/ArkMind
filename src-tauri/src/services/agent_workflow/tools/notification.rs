//! ext_tool_manage_notification：通知增删改查执行器

use crate::database;
use serde_json::{json, Value};
use crate::services::agent_workflow::*;

/// 校验通知 action 按钮数组：view_* 按钮必须带可解析 ID 且指向真实存在的对象，
/// 否则（缺参/死链）移除该按钮。返回 (清理后的 JSON, 被移除按钮的标签列表)。
/// 被移除标签会回显进工具结果，让创建方 Agent 当场感知并修正。
pub(crate) fn sanitize_actions_json(actions_raw: &str) -> (String, Vec<String>) {
    let parsed: Vec<Value> = match serde_json::from_str(actions_raw) { Ok(v) => v, Err(_) => return (actions_raw.to_string(), vec![]) };
    let mut kept: Vec<Value> = Vec::new();
    let mut dropped: Vec<String> = Vec::new();
    for mut item in parsed {
        let atype = item["type"].as_str().unwrap_or("").to_string();
        // ★ update_object 创建侧校验：类型支持 + 目标存在 + 白名单内至少一个有效字段，
        //   并把 updates 重写为过滤后的安全子集（防死按钮与越权字段）
        if atype == "update_object" {
            use crate::commands::notification_commands::{filter_update_fields, link_target_table};
            let et = item["params"]["entity_type"].as_str().unwrap_or("").trim().to_uppercase();
            let eid = item["params"]["entity_id"].as_str().unwrap_or("").trim().to_string();
            let eid = match eid.find('-') {
                Some(i) if i > 0 && eid[..i].chars().all(|c| c.is_ascii_uppercase()) => eid[i + 1..].to_string(),
                _ => eid.clone(),
            };
            let target_ok = link_target_table(&et).map(|(t, c)| {
                !eid.is_empty() && database::get(&format!("SELECT 1 FROM {} WHERE {}=?1 LIMIT 1", t, c), &[json!(&eid)])
                    .ok().flatten().is_some()
            }).unwrap_or(false);
            let clean = filter_update_fields(&et, &item["params"]["updates"]);
            if target_ok && !clean.is_empty() {
                if let Some(o) = item.as_object_mut() {
                    if let Some(pm) = o.get_mut("params").and_then(|p| p.as_object_mut()) {
                        pm.insert("entity_id".into(), json!(eid));
                        pm.insert("entity_type".into(), json!(et));
                        pm.insert("updates".into(), Value::Object(clean));
                    }
                }
                kept.push(item);
            } else {
                dropped.push(item["label"].as_str().filter(|s| !s.is_empty()).unwrap_or(&atype).to_string());
            }
            continue;
        }
        // ★ link_file 创建侧校验：单目标(entity_type/entity_id)或批量(links[])，
        //   逐条验存在性、剔除坏条（全坏才删按钮）——死按钮不留到用户点击（与 view_* 同策略）
        if atype == "link_file" {
            use crate::commands::notification_commands::{link_target_table, strip_type_prefix};
            let exists = |t: &str, id: &str| -> bool {
                link_target_table(t).map(|(tab, col)| {
                    !id.is_empty() && database::get(
                        &format!("SELECT 1 FROM {} WHERE {}=?1 LIMIT 1", tab, col), &[json!(id)]
                    ).ok().flatten().is_some()
                }).unwrap_or(false)
            };
            let strip = |s: &str| strip_type_prefix(s.trim()).to_string();
            let fid = strip(item["params"]["file_id"].as_str().unwrap_or(""));
            let file_ok = !fid.is_empty() && database::get(
                "SELECT 1 FROM file_assets WHERE id=?1 LIMIT 1", &[json!(&fid)]
            ).ok().flatten().is_some();
            if !file_ok { dropped.push(item["label"].as_str().filter(|s| !s.is_empty()).unwrap_or(&atype).to_string()); continue; }
            if let Some(arr) = item["params"]["links"].as_array().cloned() {
                let good: Vec<Value> = arr.into_iter().filter_map(|mut l| {
                    let et = l["entity_type"].as_str().unwrap_or("").trim().to_uppercase();
                    let eid = strip(l["entity_id"].as_str().unwrap_or(""));
                    if eid.is_empty() { return None; }
                    if !exists(&et, &eid) { return None; }
                    if let Some(o) = l.as_object_mut() { o.insert("entity_id".into(), json!(eid)); o.insert("entity_type".into(), json!(et)); }
                    Some(l)
                }).collect();
                if good.is_empty() { dropped.push(item["label"].as_str().filter(|s| !s.is_empty()).unwrap_or(&atype).to_string()); }
                else {
                    if let Some(o) = item.as_object_mut() {
                        if let Some(p) = o.get_mut("params").and_then(|p| p.as_object_mut()) { p.insert("links".into(), Value::Array(good)); }
                    }
                    kept.push(item);
                }
                continue;
            }
            let et = item["params"]["entity_type"].as_str().unwrap_or("").trim().to_uppercase();
            let eid = strip(item["params"]["entity_id"].as_str().unwrap_or(""));
            if exists(&et, &eid) { kept.push(item); }
            else { dropped.push(item["label"].as_str().filter(|s| !s.is_empty()).unwrap_or(&atype).to_string()); }
            continue;
        }
        let target = match atype.strip_prefix("view_") {
            Some(t) => t,
            None => { kept.push(item); continue; }
        };
        let (table, id_col, id_param) = match target {
            "email" => ("emails", "id", "email_id"),
            "project" => ("projects", "id", "project_id"),
            "todo" => ("todos", "id", "todo_id"),
            "note" => ("notes", "note_id", "note_id"),
            "product" => ("products", "id", "product_id"),
            "contact" => ("partner_contacts", "id", "contact_id"),
            "partner" | "channel" | "vendor" | "customer" => ("partners", "id", "partner_id"),
            _ => { kept.push(item); continue; }
        };
        let raw = item["params"][id_param].as_str().unwrap_or("").trim().to_string();
        // 剥已知 TYPE- 前缀（entity_ref 单一入口，非注册前缀不误剥）
        let bare = crate::services::agent_workflow::tools::entity_ref::bare_id(&raw);
        let exists = !bare.is_empty() && database::get(
            &format!("SELECT 1 FROM {} WHERE {}=?1 LIMIT 1", table, id_col), &[json!(&bare)]
        ).ok().flatten().is_some();
        if exists { kept.push(item); }
        else { dropped.push(item["label"].as_str().filter(|s| !s.is_empty()).unwrap_or(&atype).to_string()); }
    }
    (serde_json::to_string(&kept).unwrap_or_else(|_| "[]".to_string()), dropped)
}

/// 启动清扫：移除存量通知里指向不存在对象的 view_* 死按钮
pub fn sweep_dead_notification_actions() {
    let rows = match database::all("SELECT id, actions FROM notifications WHERE actions IS NOT NULL AND actions NOT IN ('', '[]')", &[]) { Ok(r) => r, Err(_) => return };
    let mut fixed = 0usize;
    for r in &rows {
        let id = r["id"].as_str().unwrap_or("");
        let raw = r["actions"].as_str().unwrap_or("[]");
        let (clean, dropped) = sanitize_actions_json(raw);
        if !dropped.is_empty() && clean != raw {
            database::run("UPDATE notifications SET actions=?1 WHERE id=?2", &[json!(clean), json!(id)]).ok();
            fixed += 1;
        }
    }
    if fixed > 0 { log::info!("[通知] 已清理 {} 条通知中指向不存在对象的失效跳转按钮", fixed); }
}

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
                                        let display_name = {
                                            let t = database::get(
                                                "SELECT title FROM notifications WHERE id=?1",
                                                &[json!(nid)],
                                            ).ok().flatten().and_then(|r| r["title"].as_str().map(String::from)).unwrap_or_default();
                                            if t.is_empty() { nid.to_string() } else { t }
                                        };
                                        match database::run("DELETE FROM notifications WHERE id=?1", &[json!(nid)]) {
                                            Ok(r) if r.changes == 0 => ToolResultEnvelope {
                                                result_type: "notification_delete_result".to_string(),
                                                data: json!({"id": nid, "success": false}),
                                                summary: Some(format!("通知不存在，未删除：{}", nid)),
                                            }.to_json(),
                                            Err(e) => ToolResultEnvelope {
                                                result_type: "notification_delete_result".to_string(),
                                                data: json!({"id": nid, "success": false}),
                                                summary: Some(format!("删除通知失败: {}", e)),
                                            }.to_json(),
                                            Ok(_) => {
                                                if let Some(ref h) = app_handle { let _ = h.emit("notification:deleted", &nid); }
                                                ToolResultEnvelope {
                                                    result_type: "notification_delete_result".to_string(),
                                                    data: json!({"id": nid, "success": true}),
                                                    summary: Some(format!("已删除通知「{}」", display_name)),
                                                }.to_json()
                                            }
                                        }
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
                                        let mut dropped_actions: Vec<String> = Vec::new();
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
                                            let raw = serde_json::to_string(actions_arr).unwrap_or_else(|_| "[]".to_string());
                                            // ★ 更新侧同样校验，防死按钮经 update 混入
                                            let (clean, dropped) = sanitize_actions_json(&raw);
                                            dropped_actions = dropped;
                                            sets.push("actions=?"); params.push(json!(clean));
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
                                            match database::run(&sql, &params) {
                                                Ok(r) if r.changes == 0 => ToolResultEnvelope {
                                                    result_type: "notification_update_result".to_string(),
                                                    data: json!({"id": nid, "success": false}),
                                                    summary: Some(format!("通知不存在，未更新：{}", nid)),
                                                }.to_json(),
                                                Err(e) => ToolResultEnvelope {
                                                    result_type: "notification_update_result".to_string(),
                                                    data: json!({"id": nid, "success": false}),
                                                    summary: Some(format!("更新通知失败: {}", e)),
                                                }.to_json(),
                                                Ok(_) => {
                                                    let notification_title = database::get(
                                                        "SELECT title FROM notifications WHERE id=?1",
                                                        &[json!(nid)],
                                                    ).ok().flatten().and_then(|r| r["title"].as_str().map(String::from)).unwrap_or_default();
                                                    let display_name = if notification_title.is_empty() { nid.to_string() } else { notification_title };
                                                    let summary = if dropped_actions.is_empty() {
                                                        format!("已更新通知「{}」", display_name)
                                                    } else {
                                                        format!("已更新通知「{}」；注意：{} 个跳转按钮因引用对象不存在已被移除（{}）", display_name, dropped_actions.len(), dropped_actions.join("、"))
                                                    };
                                                    ToolResultEnvelope {
                                                        result_type: "notification_update_result".to_string(),
                                                        data: json!({"id": nid, "success": true}),
                                                        summary: Some(summary),
                                                    }.to_json()
                                                }
                                            }
                                        }
                                    }
                                }
                                "create" => {
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
                                    // ★ 创建侧校验：剔除指向不存在对象的 view_* 死按钮
                                    let (actions_str, dropped_actions) = sanitize_actions_json(&actions_str);
                                    // ★ 去重安全网（7 天窗口，仅防完全相同 title+entity_id 重复）：
                                    //   语义级去重由分析师自行调 list 查近 7 天通知判断，这里只兜底防机器级重复
                                    let is_dup = (!entity_id.is_empty() && database::get(
                                        "SELECT id FROM notifications WHERE type=?1 AND entity_id=?2 AND created_at > datetime('now','-7 days') LIMIT 1",
                                        &[json!(ntype), json!(entity_id)]
                                    ).ok().flatten().is_some()) || {
                                        let title_trim = title.trim();
                                        !title_trim.is_empty() && !entity_id.is_empty() && database::get(
                                            "SELECT id FROM notifications WHERE title=?1 AND entity_id=?2 AND created_at > datetime('now','-7 days') LIMIT 1",
                                            &[json!(title_trim), json!(entity_id)]
                                        ).ok().flatten().is_some()
                                    };
                                    if is_dup {
                                        let envelope = ToolResultEnvelope {
                                            result_type: "notification_create_result".to_string(),
                                            data: json!({"success": false}),
                                            summary: Some("通知已存在，跳过创建".to_string()),
                                        };
                                        envelope.to_json()
                                    } else {
                                        let id = uuid::Uuid::new_v4().to_string();
                                        match database::run(
                                            "INSERT INTO notifications(id,type,title,content,entity_type,entity_id,priority,source,follow_up_of,actions) VALUES(?1,?2,?3,?4,?5,?6,?7,'llm',?8,?9)",
                                            &[json!(&id), json!(ntype), json!(title), json!(content), json!(entity_type), json!(entity_id), json!(priority), json!(follow_up_of), json!(actions_str)],
                                        ) {
                                            Err(e) => ToolResultEnvelope {
                                                result_type: "notification_create_result".to_string(),
                                                data: json!({"success": false}),
                                                summary: Some(format!("创建通知失败: {}", e)),
                                            }.to_json(),
                                            Ok(_) => {
                                                if let Some(ref h) = app_handle { let _ = h.emit("notification:new", &id); }
                                                let summary = if dropped_actions.is_empty() {
                                                    format!("已创建通知「{}」", title)
                                                } else {
                                                    format!("已创建通知「{}」；注意：{} 个跳转按钮因引用对象不存在已被移除（{}）；view_* 按钮的 ID 必须来自本轮工具查询确认存在的对象", title, dropped_actions.len(), dropped_actions.join("、"))
                                                };
                                                ToolResultEnvelope {
                                                    result_type: "notification_create_result".to_string(),
                                                    data: json!({"id": id, "title": title, "success": true}),
                                                    summary: Some(summary),
                                                }.to_json()
                                            }
                                        }
                                    }
                                }
                                _ => {
                                    ToolResultEnvelope {
                                        result_type: "notification_create_result".to_string(),
                                        data: json!({"success": false}),
                                        summary: Some(format!("未知操作类型: {}。支持 create/update/delete/list/get。", a["action"].as_str().unwrap_or(""))),
                                    }.to_json()
                                }
                            };
    res
}