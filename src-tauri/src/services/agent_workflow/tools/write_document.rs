//! ext_tool_write_document：AI 生成 Word(.docx)/Excel(.xlsx) 文档执行器
//!
//! - create：按 AI 设计的 content 结构生成文档 → CAS 入库 → 返回 file_id（供预览/下载/复制）
//! - read：回读已有生成文档的原始 content（小文档多轮修改：读结构后整体重新生成）
//! - append：向已有 docx 追加章节块（长文档分批成文：file_id 原地不变）

use crate::services::agent_workflow::ToolResultEnvelope;
use crate::services::doc_writer;
use serde_json::{json, Value};

pub(crate) fn execute(a: &Value) -> String {
    let file_type = a["file_type"].as_str().unwrap_or("").trim().to_lowercase();
    let action = a["action"].as_str().unwrap_or("create").trim().to_lowercase();

    // ── read：回读原文档结构（多轮修改第一步）──
    if action == "read" {
        let source_id = a["source_file_id"].as_str().unwrap_or("");
        return match doc_writer::read_content(source_id) {
            Ok(data) => ToolResultEnvelope {
                result_type: "document_read_result".to_string(),
                data,
                summary: Some(
                    "已返回该文档的原始 content 结构。小文档修改用 action=create 传完整新内容重新生成；长文档只改尾部时优先 action=append 仅传新增块。"
                        .to_string(),
                ),
            }
            .to_json(),
            Err(e) => ToolResultEnvelope {
                result_type: "document_read_result".to_string(),
                data: json!({"success": false, "error": e}),
                summary: Some(format!("读取文档失败: {}", e)),
            }
            .to_json(),
        };
    }

    // ── append：向已有 docx 追加章节块（长文分批成文的组装路径，file_id 不变）──
    if action == "append" {
        let source_id = a["source_file_id"].as_str().unwrap_or("");
        if source_id.is_empty() {
            return err("append 需要 source_file_id（create 返回的 file_id）。".to_string());
        }
        let content = &a["content"];
        if content.is_null() {
            return err("append 需要 content：{children:[新章节块]}，只传本次新增内容。".to_string());
        }
        return match doc_writer::append_docx(source_id, content) {
            Ok(doc) => ToolResultEnvelope {
                result_type: "document_write_result".to_string(),
                data: json!({
                    "success": true,
                    "appended": true,
                    "file_id": doc.file_id,
                    "file_name": doc.file_name,
                    "file_type": doc.file_type,
                    "file_size": doc.file_size,
                }),
                summary: Some(format!(
                    "已向文档「{}」追加章节（file_id={} 不变，当前 {} KB）。继续 append 后续章节；全部章节完成后一句话汇报。",
                    doc.file_name, doc.file_id, (doc.file_size as f64 / 1024.0).ceil() as i64
                )),
            }
            .to_json(),
            Err(e) => err(e),
        };
    }

    if action != "create" {
        return err(format!(
            "未知 action「{}」。支持：create（生成文档）、read（读取已有文档结构）、append（向 docx 追加章节块）。",
            action
        ));
    }

    if file_type != "docx" && file_type != "xlsx" {
        return err(format!(
            "不支持的 file_type「{}」。支持：docx（Word）、xlsx（Excel）。",
            file_type
        ));
    }
    let filename = a["filename"].as_str().unwrap_or("").trim();
    if filename.is_empty() {
        return err("创建文档需要提供 filename（文件名，不含扩展名）。".to_string());
    }
    let content = &a["content"];
    if !content.is_object() || content.as_object().map_or(true, |m| m.is_empty()) {
        return err(
            "创建文档需要提供 content（文档内容结构，由你根据需求设计）。".to_string(),
        );
    }

    let result = if file_type == "docx" {
        doc_writer::write_docx(filename, content, a.get("metadata"))
    } else {
        doc_writer::write_xlsx(filename, content)
    };

    match result {
        Ok(doc) => {
            let type_label = if doc.file_type == "docx" { "Word" } else { "Excel" };
            ToolResultEnvelope {
                result_type: "document_write_result".to_string(),
                data: json!({
                    "success": true,
                    "file_id": doc.file_id,
                    "file_name": doc.file_name,
                    "file_type": doc.file_type,
                    "file_size": doc.file_size,
                }),
                summary: Some(format!(
                    "已创建{}文档「{}」（{}），用户可在对话中预览/下载/复制。修改：小文档 action=read 读原结构后 action=create 传完整新内容；长文档追加章节用 action=append + source_file_id={} 只传新增块。",
                    type_label,
                    doc.file_name,
                    human_size(doc.file_size),
                    doc.file_id
                )),
            }
            .to_json()
        }
        Err(e) => ToolResultEnvelope {
            result_type: "document_write_result".to_string(),
            data: json!({"success": false, "error": e}),
            summary: Some(format!("文档创建失败: {}", e)),
        }
        .to_json(),
    }
}

fn err(msg: String) -> String {
    ToolResultEnvelope {
        result_type: "document_write_result".to_string(),
        data: json!({"success": false, "error": msg}),
        summary: Some(msg),
    }
    .to_json()
}

fn human_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{} B", bytes)
    }
}

/// 从 tool_log 提取本轮成功生成的文档对象列表（供 assistant 消息 metadata 持久化，
/// 重启后前端据此恢复聊天记录中的文档卡片）。
/// 两条路径：
/// 1. 直调 ext_tool_write_document（保留兼容）
/// 2. 委托 ext_tool_delegate_agent → delegate envelope 的 data.artifacts 中 type=document_created
///    （文档助手子 Agent 生成时，主 Agent tool_log 只有 delegate 条目）
pub(crate) fn extract_created_documents(tool_log: Option<&Value>) -> Vec<Value> {
    let mut docs: Vec<Value> = Vec::new();
    let Some(rounds) = tool_log.and_then(|t| t.as_array()) else {
        return docs;
    };
    for round in rounds {
        let Some(tools) = round["tools"].as_array() else { continue };
        for t in tools {
            let Ok(env) = serde_json::from_str::<Value>(t["result"].as_str().unwrap_or("")) else {
                continue;
            };
            match t["tool"].as_str() {
                // 路径1：直调 write_document
                Some("ext_tool_write_document") => {
                    if env["result_type"] == "document_write_result"
                        && env["data"]["success"] == true
                        && env["data"]["file_id"].as_str().is_some()
                    {
                        docs.push(json!({
                            "file_id": env["data"]["file_id"],
                            "file_name": env["data"]["file_name"],
                            "file_type": env["data"]["file_type"],
                            "file_size": env["data"]["file_size"],
                        }));
                    }
                }
                // 路径2：委托文档助手 → artifacts 中的文档产物
                Some("ext_tool_delegate_agent") => {
                    if env["result_type"] != "delegate_agent_result" {
                        continue;
                    }
                    let Some(artifacts) = env["data"]["artifacts"].as_array() else { continue };
                    for a in artifacts {
                        if a["type"] != "document_created" || a["file_id"].as_str().is_none() {
                            continue;
                        }
                        docs.push(json!({
                            "file_id": a["file_id"],
                            "file_name": a["file_name"],
                            "file_type": a["file_type"],
                            "file_size": a["file_size"],
                        }));
                    }
                }
                _ => {}
            }
        }
    }
    // 按 file_id 去重（保序）：同 id 保留**末次**结果的 size/name——长文分批成文时
    // create 快照只有骨架大小，必须以最后一次 append 的完整大小为准
    let mut idx: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut out: Vec<Value> = Vec::new();
    for d in docs {
        let fid = d["file_id"].as_str().unwrap_or("").to_string();
        match idx.get(&fid) {
            Some(&i) => out[i] = d,
            None => {
                idx.insert(fid, out.len());
                out.push(d);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 双路径提取：直调 + 委托嵌套（artifacts）+ file_id 去重
    #[test]
    fn test_extract_created_documents_both_paths() {
        let tool_log = json!([
            {
                "round": 1,
                "tools": [{
                    "tool": "ext_tool_write_document",
                    "result": r#"{"result_type":"document_write_result","data":{"success":true,"file_id":"fid-direct","file_name":"直调.docx","file_type":"docx","file_size":100}}"#
                }]
            },
            {
                "round": 2,
                "tools": [{
                    "tool": "ext_tool_delegate_agent",
                    "result": r#"{"result_type":"delegate_agent_result","data":{"status":"completed","artifacts":[{"type":"document_created","file_id":"fid-delegated","file_name":"委托.xlsx","file_type":"xlsx","file_size":200},{"type":"object_created","id":"x","name":"y"}]}}"#
                }]
            },
            {
                "round": 3,
                "tools": [{
                    "tool": "ext_tool_delegate_agent",
                    "result": r#"{"result_type":"delegate_agent_result","data":{"artifacts":[{"type":"document_created","file_id":"fid-direct","file_name":"重复.docx","file_type":"docx","file_size":100}]}}"#
                }]
            }
        ]);
        let docs = extract_created_documents(Some(&tool_log));
        assert_eq!(docs.len(), 2, "应提取2个文档（按 file_id 去重）: {:?}", docs);
        assert_eq!(docs[0]["file_id"].as_str(), Some("fid-direct"));
        assert_eq!(docs[1]["file_id"].as_str(), Some("fid-delegated"));
        assert_eq!(docs[1]["file_name"].as_str(), Some("委托.xlsx"));
    }

    /// 分批成文：同 file_id 的 create(骨架小尺寸)+多次 append(增长) → 去重保**末次**尺寸
    #[test]
    fn test_extract_created_documents_last_wins_size() {
        let tool_log = json!([
            {"round": 1, "tools": [{
                "tool": "ext_tool_delegate_agent",
                "result": r#"{"result_type":"delegate_agent_result","data":{"artifacts":[{"type":"document_created","file_id":"F1","file_name":"方案.docx","file_type":"docx","file_size":30}]}}"#
            }]},
            {"round": 2, "tools": [{
                "tool": "ext_tool_delegate_agent",
                "result": r#"{"result_type":"delegate_agent_result","data":{"artifacts":[{"type":"document_created","file_id":"F1","file_name":"方案.docx","file_type":"docx","file_size":190}]}}"#
            }]},
            {"round": 3, "tools": [{
                "tool": "ext_tool_write_document",
                "result": r#"{"result_type":"document_write_result","data":{"success":true,"appended":true,"file_id":"F1","file_name":"方案.docx","file_type":"docx","file_size":420}}"#
            }]}
        ]);
        let docs = extract_created_documents(Some(&tool_log));
        assert_eq!(docs.len(), 1, "同 file_id 应合并为一条");
        assert_eq!(docs[0]["file_size"].as_i64(), Some(420), "尺寸必须取末次完整大小");
    }

    /// 空输入与非文档条目不产出
    #[test]
    fn test_extract_created_documents_empty_and_noise() {
        assert!(extract_created_documents(None).is_empty());
        assert!(extract_created_documents(Some(&json!([]))).is_empty());
        // 只有失败的生成与非文档 artifacts → 空
        let noise = json!([{
            "round": 1,
            "tools": [
                {"tool": "ext_tool_write_document",
                 "result": r#"{"result_type":"document_write_result","data":{"success":false,"error":"boom"}}"#},
                {"tool": "ext_tool_delegate_agent",
                 "result": r#"{"result_type":"delegate_agent_result","data":{"artifacts":[{"type":"object_created","id":"a","name":"b"}]}}"#},
                {"tool": "ext_tool_search", "result": "not json"},
            ]
        }]);
        assert!(extract_created_documents(Some(&noise)).is_empty());
    }
}
