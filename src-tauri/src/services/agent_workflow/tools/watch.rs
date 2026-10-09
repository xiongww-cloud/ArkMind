//! ext_tool_watch：观察池管理（分析师工作队列的用户侧入口）
//! - 池=工作队列：条目入池（分析师 WATCH / 用户指定 add / 续盯再入）→ 通知创建即出队 → 用户处置定终态
//! - add/update 直写（用户明示关注的执行）；remove 仅限用户本轮明确指令（教育门槛在 schema 与系统提示）
//! - 存储与分区规则在 notification_loop（zone 保护、上限、TTL 由池侧统一保证）

use crate::services::agent_workflow::ToolResultEnvelope;
use crate::services::notification_loop;
use serde_json::{json, Value};

pub(crate) fn execute(a: &Value) -> String {
    let action = a["action"].as_str().unwrap_or("");
    let fail = |e: String| ToolResultEnvelope {
        result_type: "watch_pool_result".to_string(),
        data: json!({"success": false, "error": e}),
        summary: Some("操作未完成".to_string()),
    }.to_json();
    let ok = |data: Value, summary: String| ToolResultEnvelope {
        result_type: "watch_pool_result".to_string(),
        data,
        summary: Some(summary),
    }.to_json();
    let entity = a["entity_ref"].as_str().unwrap_or("");
    match action {
        "list" => {
            let entries = notification_loop::watch_pool_structured();
            let lines: Vec<String> = entries.iter().map(|e| format!(
                "- [w{}|{}|{}] {}{}",
                e["id"].as_u64().unwrap_or(0),
                if e["zone"] == json!("user") { "用户指定" } else { "AI洞察" },
                e["age"].as_str().unwrap_or(""),
                e["text"].as_str().unwrap_or(""),
                if e["entity"].as_str().unwrap_or("").is_empty() { String::new() } else { format!("（锚点 {}）", e["entity"].as_str().unwrap_or("")) },
            )).collect();
            let body = if lines.is_empty() { "（观察池为空）".to_string() } else { lines.join("\n") };
            ok(json!({"success": true, "entries": entries}), format!("观察池 {} 条：\n{}", lines.len(), body))
        }
        "add" => {
            let text = a["text"].as_str().unwrap_or("");
            match notification_loop::watch_add(text, entity) {
                Ok(id) => ok(json!({"success": true, "watch_id": id}), format!("已加入用户指定观察 w{}：{}（分析师每轮将优先跟进；通知产出后自动出队，用户可经卡片续盯）", id, text.trim())),
                Err(e) => fail(e),
            }
        }
        "update" => {
            let raw = a["watch_id"].as_str().unwrap_or("");
            let Some(id) = notification_loop::parse_wid(raw) else { return fail(format!("无法识别观察条目 id：{}", raw)); };
            match notification_loop::watch_update(id, a["text"].as_str().unwrap_or(""), a["entity_ref"].as_str()) {
                Ok(()) => ok(json!({"success": true, "watch_id": id}), format!("已更新观察 w{}（保留原 id，引用不断链）", id)),
                Err(e) => fail(e),
            }
        }
        "remove" => {
            let raw = a["watch_id"].as_str().unwrap_or("");
            let Some(id) = notification_loop::parse_wid(raw) else { return fail(format!("无法识别观察条目 id：{}", raw)); };
            match notification_loop::watch_remove(id) {
                Ok(()) => ok(json!({"success": true}), format!("已删除观察 w{}", id)),
                Err(e) => fail(e),
            }
        }
        other => fail(format!("未知操作: {}（支持 list/add/update/remove）", other)),
    }
}
