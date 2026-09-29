//! ext_tool_manage_note：笔记增删改执行器（查看用 ext_tool_object(type="NOTE")，搜索正文用 ext_tool_search）

use crate::database;
use serde_json::json;
use crate::services::agent_workflow::*;
use super::common::resolve_project_id;

/// 笔记本匹配：空=未归类；按 ID 精确 → 名称精确 → 名称唯一模糊；查无/多匹配返回 Err（不自动建笔记本）
fn match_notebook(input: &str) -> Result<Option<(String, String)>, String> {
    let kw = input.trim();
    if kw.is_empty() {
        return Ok(None);
    }
    let by_id = database::get("SELECT id, name FROM notebooks WHERE id=?1", &[json!(kw)])
        .ok().flatten()
        .map(|r| (r["id"].as_str().unwrap_or("").to_string(), r["name"].as_str().unwrap_or("").to_string()));
    if let Some(v) = by_id {
        if !v.0.is_empty() { return Ok(Some(v)); }
    }
    let exact = database::get("SELECT id, name FROM notebooks WHERE name=?1", &[json!(kw)])
        .ok().flatten()
        .map(|r| (r["id"].as_str().unwrap_or("").to_string(), r["name"].as_str().unwrap_or("").to_string()));
    if let Some(v) = exact {
        if !v.0.is_empty() { return Ok(Some(v)); }
    }
    let like = format!("%{}%", crate::database::escape_like(kw));
    let rows = database::all(
        "SELECT id, name FROM notebooks WHERE name LIKE ?1 ESCAPE '\\' LIMIT 2",
        &[json!(&like)],
    ).unwrap_or_default();
    let cands: Vec<(String, String)> = rows.iter()
        .filter_map(|r| Some((r["id"].as_str()?.to_string(), r["name"].as_str()?.to_string())))
        .collect();
    match cands.len() {
        1 => Ok(Some(cands[0].clone())),
        0 => Err(format!("笔记本「{}」不存在。可用笔记本：{}", kw, notebook_names())),
        _ => Err(format!("笔记本「{}」匹配到多个，请指明其一：{}", kw, notebook_names())),
    }
}

fn notebook_names() -> String {
    database::all("SELECT name FROM notebooks ORDER BY sort_order, name", &[])
        .unwrap_or_default()
        .iter()
        .filter_map(|r| r["name"].as_str())
        .collect::<Vec<_>>()
        .join("、")
}

pub(crate) fn ext_tool_manage_note(action: &str, title: &str, content: &str, note_id: &str, notebook: &str, project_id: &str) -> String {
    // 通过 note_id 或标题模糊匹配查找笔记（仅限未删除）
    let find_note = |nid: &str, t: &str| -> Option<(String, String)> {
        if !nid.trim().is_empty() {
            let clean = nid.trim().strip_prefix("NOTE-").unwrap_or(nid.trim());
            database::get("SELECT note_id, title FROM notes WHERE note_id=?1 AND deleted_at IS NULL", &[json!(clean)])
                .ok().flatten().map(|r| (r["note_id"].as_str().unwrap_or("").to_string(), r["title"].as_str().unwrap_or("").to_string()))
        } else if !t.trim().is_empty() {
            let like = format!("%{}%", crate::database::escape_like(t.trim()));
            database::get("SELECT note_id, title FROM notes WHERE title LIKE ?1 ESCAPE '\\' AND deleted_at IS NULL ORDER BY updated_at DESC LIMIT 1", &[json!(&like)])
                .ok().flatten().map(|r| (r["note_id"].as_str().unwrap_or("").to_string(), r["title"].as_str().unwrap_or("").to_string()))
        } else { None }
    };
    let fail = |msg: String| -> String {
        ToolResultEnvelope {
            result_type: "object_create_result".to_string(),
            data: json!({"success": false}),
            summary: Some(msg),
        }.to_json()
    };

    match action {
        "create" => {
            if title.trim().is_empty() && content.trim().is_empty() {
                return fail("创建笔记需要提供 title（或至少 content）。".to_string());
            }
            // ★ 同名分流（AI 重复建档/并发写入防护）：
            //   同名+同内容 → 复用；同名+不同内容 → 拒绝并引导 update（防止新内容静默丢失）
            if !title.trim().is_empty() {
                if let Ok(Some(existing)) = crate::database::get(
                    "SELECT note_id, title, content FROM notes WHERE title=?1 AND deleted_at IS NULL ORDER BY created_at DESC LIMIT 1",
                    &[json!(title.trim())],
                ) {
                    let eid = existing["note_id"].as_str().unwrap_or("").to_string();
                    let econtent = existing["content"].as_str().unwrap_or("");
                    if !eid.is_empty() {
                        if econtent == content.trim() || content.trim().is_empty() {
                            let envelope = ToolResultEnvelope {
                                result_type: "object_create_result".to_string(),
                                data: json!({"success": true, "id": eid, "type": "NOTE", "name": title.trim(), "duplicate": true}),
                                summary: Some(format!("已存在同名笔记「{}」，ID: {}，内容一致已复用，未重复创建。", title.trim(), eid)),
                            };
                            return envelope.to_json();
                        } else {
                            let envelope = ToolResultEnvelope {
                                result_type: "object_create_result".to_string(),
                                data: json!({"success": false, "id": eid, "type": "NOTE", "duplicate_conflict": true, "error": "同名笔记已存在且内容不同"}),
                                summary: Some(format!("已存在同名笔记「{}」（ID: {}）但内容不同，未创建。如需修改已有笔记请用 update（传 note_id），如需新建请换一个标题。", title.trim(), eid)),
                            };
                            return envelope.to_json();
                        }
                    }
                }
            }
            let nb = match match_notebook(notebook) {
                Ok(v) => v,
                Err(msg) => return fail(msg),
            };
            let pid = if project_id.trim().is_empty() { None } else { resolve_project_id(project_id) };
            let mut data = json!({
                "title": if title.trim().is_empty() { "未命名笔记" } else { title.trim() },
                "content": content,
                "isDaily": false,
            });
            if let Some((ref id, _)) = nb { data["notebookId"] = json!(id); }
            if let Some(ref p) = pid { data["projectIds"] = json!([p]); }
            match block_on_async(crate::commands::notes_commands::notes_create(data)) {
                Ok(created) => {
                    let nid = created["noteId"].as_str().unwrap_or("");
                    let mut summary = format!("已创建笔记「{}」，ID: {}", title.trim(), nid);
                    if let Some((_, ref nbname)) = nb { summary.push_str(&format!("（笔记本：{}）", nbname)); }
                    else { summary.push_str("（未归类，可在全部笔记视图查看）"); }
                    ToolResultEnvelope {
                        result_type: "object_create_result".to_string(),
                        data: json!({"success": true, "id": nid, "type": "NOTE", "name": title.trim()}),
                        summary: Some(summary),
                    }.to_json()
                }
                Err(e) => fail(format!("创建笔记失败: {}", e)),
            }
        }
        "update" => {
            let found = find_note(note_id, title);
            match found {
                None => fail("未找到指定笔记，请提供 note_id 或准确的标题关键词。".to_string()),
                Some((fid, ftitle)) => {
                    let mut data = json!({ "noteId": fid });
                    let mut changed = false;
                    // title 非空但 note_id 为空时，title 只作为定位条件，不视为改名（改名需显式 note_id）
                    if !title.trim().is_empty() && !note_id.trim().is_empty() { data["title"] = json!(title.trim()); changed = true; }
                    if !content.trim().is_empty() { data["content"] = json!(content); changed = true; }
                    if !notebook.trim().is_empty() {
                        match match_notebook(notebook) {
                            Ok(Some((id, name))) => { data["notebookId"] = json!(id); data["_nbname"] = json!(name); changed = true; }
                            Ok(None) => {}
                            Err(msg) => return fail(msg),
                        }
                    }
                    if !changed {
                        return fail("更新笔记需要提供要修改的字段（title/content/notebook）。".to_string());
                    }
                    let nbname = data.as_object().and_then(|m| m.get("_nbname")).and_then(|v| v.as_str()).unwrap_or("").to_string();
                    data.as_object_mut().map(|m| m.remove("_nbname"));
                    match block_on_async(crate::commands::notes_commands::notes_update(data)) {
                        Ok(_) => {
                            let mut summary = format!("已更新笔记「{}」", ftitle);
                            if !nbname.is_empty() { summary.push_str(&format!("，已移动到笔记本「{}」", nbname)); }
                            ToolResultEnvelope {
                                result_type: "object_update_result".to_string(),
                                data: json!({"success": true, "id": fid, "type": "NOTE", "name": ftitle}),
                                summary: Some(summary),
                            }.to_json()
                        }
                        Err(e) => fail(format!("更新笔记失败: {}", e)),
                    }
                }
            }
        }
        "delete" => {
            let found = find_note(note_id, title);
            match found {
                None => fail("未找到指定笔记，请提供 note_id 或准确的标题关键词。".to_string()),
                Some((fid, ftitle)) => {
                    match block_on_async(crate::commands::notes_commands::notes_delete(fid)) {
                        Ok(_) => ToolResultEnvelope {
                            result_type: "object_delete_result".to_string(),
                            data: json!({"success": true, "name": ftitle}),
                            summary: Some(format!("笔记「{}」已移入回收站（可在笔记页回收站恢复，30 天后自动清空）", ftitle)),
                        }.to_json(),
                        Err(e) => fail(format!("删除笔记失败: {}", e)),
                    }
                }
            }
        }
        "list_notebooks" => {
            match block_on_async(crate::commands::notes_commands::notebook_list()) {
                Ok(result) => {
                    let notebooks = result["notebooks"].as_array().cloned().unwrap_or_default();
                    let items: Vec<Value> = notebooks.iter().map(|nb| {
                        json!({
                            "id": nb["id"].as_str().unwrap_or(""),
                            "name": nb["name"].as_str().unwrap_or("未命名"),
                            "color": nb["color"].as_str().unwrap_or("")
                        })
                    }).collect();
                    let summary_lines: Vec<String> = items.iter().map(|nb| {
                        nb["name"].as_str().unwrap_or("未命名").to_string()
                    }).collect();
                    ToolResultEnvelope {
                        result_type: "object_query_result".to_string(),
                        data: json!({"success": true, "items": items, "count": items.len()}),
                        summary: Some(format!("共 {} 个笔记本：{}", items.len(), summary_lines.join("、"))),
                    }.to_json()
                }
                Err(e) => fail(format!("查询笔记本列表失败: {}", e)),
            }
        }
        "create_notebook" => {
            if title.trim().is_empty() {
                return fail("创建笔记本需要提供 title。".to_string());
            }
            match block_on_async(crate::commands::notes_commands::notebook_create(json!({"name": title.trim()}))) {
                Ok(created) => {
                    let nb = &created["notebook"];
                    let nid = nb["id"].as_str().unwrap_or("");
                    let nname = nb["name"].as_str().unwrap_or(title.trim());
                    ToolResultEnvelope {
                        result_type: "object_create_result".to_string(),
                        data: json!({"success": true, "id": nid, "name": nname}),
                        summary: Some(format!("已创建笔记本「{}」，ID: {}", nname, nid)),
                    }.to_json()
                }
                Err(e) => fail(format!("创建笔记本失败: {}", e)),
            }
        }
        _ => fail(format!("未知操作类型: {}。支持 create/update/delete/list_notebooks/create_notebook。", action)),
    }
}
