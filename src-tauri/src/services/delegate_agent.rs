//! delegate_agent — 子 Agent 委托执行模块
//! 将任务委托给专业 Agent 处理，子 Agent 拥有独立会话和工具集

use serde_json::{json, Value};
use crate::database;

/// 安全截断字符串（尊重 UTF-8 字符边界）
fn truncate_str(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
}

/// 执行 delegate_agent 工具
/// 从数据库加载 Agent Profile，调用 agent_chat_workflow 执行任务，返回结果
pub async fn execute_delegate_agent(
    agent_id: &str,
    task: &str,
    context: Option<&Value>,
    expected_output: &str,
    app_handle: Option<tauri::AppHandle>,
) -> String {
    // 参数校验
    if agent_id.is_empty() {
        return make_error("请提供 agent_id。");
    }
    if task.is_empty() {
        return make_error("请提供 task 描述。");
    }

    // ★ 一次 JOIN 查询加载 Agent Profile + 工具分配（原来 2 次查询）
    let rows = match database::all(
        "SELECT p.system_prompt, p.max_iterations, a.tool_name FROM agent_profiles p LEFT JOIN agent_tool_assignments a ON a.agent_id=p.id WHERE p.id=?1 AND p.status='active'",
        &[json!(agent_id)],
    ) {
        Ok(r) => r,
        Err(e) => {
            return make_error(&format!("加载 Agent 失败: {}", e));
        }
    };
    if rows.is_empty() {
        return make_error(&format!("未找到 Agent「{}」或已禁用。", agent_id));
    }

    let first = &rows[0];
    let allowed_tools: Vec<String> = rows.iter()
        .filter_map(|r| r["tool_name"].as_str().map(|s| s.to_string()))
        .collect();

    // system_prompt 直接用 Agent 配置的提示词（工具说明已由 API tools schema 提供，不再重复注入）
    let system_prompt = first["system_prompt"].as_str().unwrap_or("").to_string();
    let max_iterations = first["max_iterations"].as_i64().unwrap_or(20) as i32;

    // 构建子 Agent 的任务描述
    let has_tools = !allowed_tools.is_empty();
    let sub_query = build_sub_query(task, context, expected_output, has_tools);

    // 调用 Agent 工作流（独立 Session）
    let sub_session_id = format!("delegate-{}", uuid::Uuid::new_v4());

    log::info!(
        "[DelegateAgent] 委托任务给 Agent「{}」，任务: {}，工具数: {}",
        agent_id,
        truncate_str(task, 50),
        allowed_tools.len()
    );

    // 使用 Box::pin 解决递归 async 调用
    let result = Box::pin(super::agent_workflow::agent_chat_workflow_internal(
        sub_query,
        vec![],
        Some(max_iterations),
        app_handle,
        true, // silent
        Some(sub_session_id.clone()),
        Some(allowed_tools),
        Some(system_prompt.as_str()),
    ))
    .await;

    // 清理子 Agent Session
    let _ = database::run(
        "DELETE FROM chat_sessions WHERE id=?1",
        &[json!(sub_session_id)],
    );
    let _ = database::run(
        "DELETE FROM chat_messages WHERE session_id=?1",
        &[json!(sub_session_id)],
    );

    match result {
        Ok(v) => {
            let answer = v["answer"].as_str().unwrap_or("");
            let iterations = v["iterations"].as_i64().unwrap_or(0);
            let tool_log = v["tool_log"].as_array().map(|a| a.as_slice()).unwrap_or(&[]);
            log::info!(
                "[DelegateAgent] Agent「{}」完成，迭代{}次，结果长度: {}",
                agent_id,
                iterations,
                answer.len()
            );
            make_success(agent_id, answer, iterations, tool_log)
        }
        Err(e) => {
            log::error!("[DelegateAgent] Agent「{}」执行失败: {}", agent_id, e);
            make_error(&format!("Agent「{}」执行失败: {}", agent_id, e))
        }
    }
}

/// 构建子 Agent 的任务描述
fn build_sub_query(task: &str, context: Option<&Value>, expected_output: &str, has_tools: bool) -> String {
    let mut parts = Vec::new();
    parts.push(format!("任务：{}", task));

    if !has_tools {
        parts.push("注意：你当前没有分配任何工具。如果你无法仅凭自身知识完成此任务，请直接回复：「无法完成：当前 Agent 没有配置相关工具，无法执行此任务。」不要编造信息。".to_string());
    }

    if let Some(ctx) = context {
        if !ctx.is_null() && ctx.as_object().map_or(false, |o| !o.is_empty()) {
            parts.push(format!("上下文：{}", ctx));
        }
    }

    if !expected_output.is_empty() {
        parts.push(format!("期望输出格式：{}", expected_output));
    }

    parts.join("\n\n")
}

fn make_error(msg: &str) -> String {
    let envelope = super::agent_workflow::ToolResultEnvelope {
        result_type: "delegate_agent_result".to_string(),
        data: json!({
            "status": "failed",
            "result": msg,
            "artifacts": [],
            "warnings": []
        }),
        summary: Some(msg.to_string()),
    };
    envelope.to_json()
}

/// 从子 Agent 的 tool_log 中提取产出物（artifacts）
fn extract_artifacts(tool_log: &[Value]) -> Vec<Value> {
    let mut artifacts = Vec::new();
    for round in tool_log {
        let Some(tools) = round["tools"].as_array() else { continue };
        for tool in tools {
            let tool_name = tool["tool"].as_str().unwrap_or("");
            let result_str = tool["result"].as_str().unwrap_or("");
            let Ok(envelope) = serde_json::from_str::<Value>(result_str) else { continue };
            let data = &envelope["data"];
            let result_type = envelope["result_type"].as_str().unwrap_or("");

            // CRUD 结果：提取 id, name
            if result_type.contains("_create_result") || result_type.contains("_update_result") || result_type.contains("_delete_result") {
                if data["success"].as_bool() == Some(true) {
                    if let Some(id) = data["id"].as_str() {
                        let name = data["name"].as_str()
                            .or_else(|| data["title"].as_str())
                            .unwrap_or("")
                            .to_string();
                        let action = if result_type.contains("_create") { "created" }
                            else if result_type.contains("_update") { "updated" }
                            else { "deleted" };
                        artifacts.push(json!({
                            "type": format!("object_{}", action),
                            "id": id,
                            "name": name,
                            "tool": tool_name
                        }));
                    }
                }
            }

            // 查询结果：提取 items
            if result_type == "object_query_result" {
                if let Some(items) = data["items"].as_array() {
                    for item in items {
                        if let (Some(id), Some(name)) = (item["id"].as_str(), item["name"].as_str()) {
                            artifacts.push(json!({
                                "type": "object_found",
                                "id": id,
                                "name": name,
                                "tool": tool_name
                            }));
                        }
                    }
                }
            }

            // 关联结果
            if result_type == "relations_result" {
                if let Some(relations) = data["relations"].as_array() {
                    for rel in relations {
                        if let (Some(id), Some(name)) = (rel["id"].as_str(), rel["name"].as_str()) {
                            artifacts.push(json!({
                                "type": "relation_found",
                                "id": id,
                                "name": name,
                                "tool": tool_name
                            }));
                        }
                    }
                }
            }
        }
    }
    artifacts
}

fn make_success(agent_id: &str, answer: &str, iterations: i64, tool_log: &[Value]) -> String {
    let artifacts = extract_artifacts(tool_log);
    let preview = truncate_str(answer, 200);
    let envelope = super::agent_workflow::ToolResultEnvelope {
        result_type: "delegate_agent_result".to_string(),
        data: json!({
            "status": "completed",
            "result": answer,
            "artifacts": artifacts,
            "agent_id": agent_id,
            "iterations": iterations,
            "warnings": []
        }),
        summary: Some(format!("Agent「{}」已完成任务。{}", agent_id, preview)),
    };
    envelope.to_json()
}
