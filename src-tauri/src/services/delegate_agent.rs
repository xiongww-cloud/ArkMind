//! delegate_agent — 子 Agent 委托执行模块
//! 将任务委托给专业 Agent 处理，子 Agent 拥有独立会话和工具集

use serde_json::{json, Value};
use crate::database;
use tauri::Emitter;

/// 安全截断字符串（尊重 UTF-8 字符边界）
fn truncate_str(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
}

/// 执行 delegate_agent 工具
/// 从数据库加载 Agent Profile，调用 agent_chat_workflow 执行任务，返回结果
/// delegate_depth: 委托深度（0=主Agent直接委托，≥1=已嵌套，拒绝再委托）
pub async fn execute_delegate_agent(
    agent_id: &str,
    task: &str,
    context: Option<&Value>,
    expected_output: &str,
    app_handle: Option<tauri::AppHandle>,
    delegate_depth: u8,
) -> String {
    // 参数校验
    if agent_id.is_empty() {
        return make_error("请提供 agent_id。");
    }
    if task.is_empty() {
        return make_error("请提供 task 描述。");
    }
    // ★ 深度守卫：只允许主Agent→子Agent一层委托
    if delegate_depth >= 1 {
        return make_error("已达到最大委托深度（仅支持主 Agent 直接委托）。");
    }

    // ★ 一次 JOIN 查询加载 Agent Profile + 工具分配（含 name 用于状态展示）
    let rows = match database::all(
        "SELECT p.name, p.system_prompt, p.max_iterations, a.tool_name FROM agent_profiles p LEFT JOIN agent_tool_assignments a ON a.agent_id=p.id WHERE p.id=?1 AND p.status='active'",
        &[json!(agent_id)],
    ) {
        Ok(r) => r,
        Err(e) => {
            return make_error(&format!("加载 Agent 失败: {}", e));
        }
    };
    if rows.is_empty() {
        // ★ 路由未命中：日志 + 前端可见反馈（LLM 会收到错误信封自行修正，用户也应看到）
        log::warn!("[Route] 委托未命中: agent_id={}（不存在或已禁用）", agent_id);
        if let Some(ref h) = app_handle {
            let _ = h.emit("agent:progress", json!({"msg": format!("路由未命中：目标 Agent（{}）不存在或已禁用", agent_id)}));
        }
        return make_error(&format!("未找到 Agent「{}」或已禁用。", agent_id));
    }

    let first = &rows[0];
    // ★ 主 Agent 专属工具清单：agent_workflow::tools::MAIN_AGENT_ONLY（唯一数据源）
    let allowed_tools: Vec<String> = rows.iter()
        .filter_map(|r| r["tool_name"].as_str().map(|s| s.to_string()))
        .filter(|t| !crate::services::agent_workflow::tools::MAIN_AGENT_ONLY.contains(&t.as_str()))
        .collect();

    // system_prompt 直接用 Agent 配置的提示词（工具说明已由 API tools schema 提供，不再重复注入）
    let system_prompt = first["system_prompt"].as_str().unwrap_or("").to_string();
    let max_iterations = first["max_iterations"].as_i64().unwrap_or(20) as i32;
    let agent_name = first["name"].as_str().unwrap_or(agent_id);

    // 构建子 Agent 的任务描述
    let has_tools = !allowed_tools.is_empty();
    let sub_query = build_sub_query(task, context, expected_output, has_tools);

    log::info!(
        "[DelegateAgent] 委托任务给 Agent「{}」，任务: {}，工具数: {}",
        agent_name,
        truncate_str(task, 50),
        allowed_tools.len()
    );

    // ★ 委托可见性：通知前端委托开始（status + progress 双通道，聊天页只监听 progress）
    if let Some(ref h) = app_handle {
        let _ = h.emit("agent:status", json!({"stage":"delegating","msg":format!("委托「{}」处理中...", agent_name)}));
        let _ = h.emit("agent:progress", json!({"msg":format!("已路由到「{}」，处理中...", agent_name)}));
    }

    // 使用 Box::pin 解决递归 async 调用（无独立 session，避免无用DB操作）
    let result = Box::pin(super::agent_workflow::agent_chat_workflow_internal(
        sub_query,
        vec![],
        Some(max_iterations),
        app_handle,
        false, // ★ silent=false：子 Agent 的 token/进度直连前端流式输出（回复不再只在刷新后可见）
        None, // ★ 无独立 session：load/save 有守卫，避免4次无用DB操作
        Some(allowed_tools),
        Some(system_prompt.as_str()),
        false,            // ★ is_root=false：不重置/不清除 AGENT_CANCEL
        delegate_depth + 1, // ★ 深度+1
        true,             // ★ cancel_aware=true：委托链响应聊天"停止"，取消向上传播
        false,            // ★ 子Agent保留完整动态上下文（工作记忆/预算对其有价值）
    ))
    .await;

    match result {
        Ok(v) => {
            let answer = v["answer"].as_str().unwrap_or("");
            let iterations = v["iterations"].as_i64().unwrap_or(0);
            let tool_log = v["tool_log"].as_array().map(|a| a.as_slice()).unwrap_or(&[]);
            let cancelled = v["cancelled"].as_bool().unwrap_or(false);

            log::info!(
                "[DelegateAgent] Agent「{}」完成，迭代{}次，结果长度: {}，cancelled: {}",
                agent_name,
                iterations,
                answer.len(),
                cancelled
            );

            // ★ 取消透传：子Agent被取消时，向主Agent传播取消状态
            if cancelled {
                return make_cancelled(agent_name, answer, iterations);
            }

            make_success(agent_name, answer, iterations, tool_log)
        }
        Err(e) => {
            log::error!("[DelegateAgent] Agent「{}」执行失败: {}", agent_name, e);
            make_error(&format!("Agent「{}」执行失败: {}", agent_name, e))
        }
    }
}

/// 构建子 Agent 的任务描述
fn build_sub_query(task: &str, context: Option<&Value>, expected_output: &str, has_tools: bool) -> String {
    let mut parts = Vec::new();

    // 优先使用用户原始输入，主 Agent 的 task 作为补充
    if let Some(ctx) = context {
        if let Some(user_query) = ctx["user_query"].as_str().filter(|s| !s.is_empty()) {
            parts.push(format!("用户问题：{}", user_query));
        } else {
            parts.push(format!("任务：{}", task));
        }
    } else {
        parts.push(format!("任务：{}", task));
    }

    if !has_tools {
        parts.push("注意：你当前没有分配任何工具，无法执行任何操作。不要编造信息，直接回复最后一行：MISSING: 当前 Agent 未配置任何工具".to_string());
    }

    if let Some(ctx) = context {
        // 注入 context（去掉 user_query 避免重复）
        if !ctx.is_null() {
            if let Some(obj) = ctx.as_object() {
                let filtered: Vec<(&String, &Value)> = obj.iter()
                    .filter(|(k, _)| *k != "user_query" && *k != "recent_messages")
                    .collect();
                if !filtered.is_empty() {
                    let mut ctx_obj = serde_json::Map::new();
                    for (k, v) in filtered { ctx_obj.insert(k.clone(), v.clone()); }
                    parts.push(format!("上下文：{}", Value::Object(ctx_obj)));
                }
            }
        }
        // recent_messages 单独注入（更清晰）
        if let Some(recent) = ctx["recent_messages"].as_array().filter(|a| !a.is_empty()) {
            let mut lines = Vec::new();
            for msg in recent {
                let role = msg["role"].as_str().unwrap_or("");
                let content = msg["content"].as_str().unwrap_or("");
                if !content.is_empty() {
                    let label = if role == "user" { "用户" } else { "助手" };
                    let preview: String = content.chars().take(200).collect();
                    lines.push(format!("{}：{}", label, preview));
                }
            }
            if !lines.is_empty() {
                parts.push(format!("最近对话：\n{}", lines.join("\n")));
            }
        }
    }

    if !expected_output.is_empty() {
        parts.push(format!("期望输出格式：{}", expected_output));
    }

    // ★ 执行协议：委托运行时统一注入，对所有子Agent（内置/自定义）生效，与agent档案prompt无关
    parts.push("## 执行协议\n- 同一任务的多个独立操作，必须在一次回复中并发调用多个工具，不要拆成多轮\n- 查询结果在同一任务内全程复用：需要补充信息时换角度查询，不要原样重查\n- 创建任何业务对象前，必须先用 ext_tool_object 按名称检索同类对象，存在相似项则先向用户确认\n- 若工具返回\"未授权\"或\"工具未找到\"，跳过该操作，用已有的信息继续完成任务，不要输出 MISSING\n- 若信息不足以完成部分任务，不要编造，在回复的最后一行输出：MISSING: <缺少什么信息或哪个操作未执行>\n- 按照 system_prompt 中的「回答规范」向用户汇报结果，不要输出内部 ID".to_string());

    parts.join("\n\n")
}

fn make_error(msg: &str) -> String {
    let envelope = super::agent_workflow::ToolResultEnvelope {
        result_type: "delegate_agent_result".to_string(),
        data: json!({
            "status": "failed",
            "result": msg,
            "artifacts": [],
            "missing": [],
            "warnings": []
        }),
        summary: Some(msg.to_string()),
    };
    envelope.to_json()
}

fn make_cancelled(agent_id: &str, answer: &str, iterations: i64) -> String {
    let envelope = super::agent_workflow::ToolResultEnvelope {
        result_type: "delegate_agent_result".to_string(),
        data: json!({
            "status": "cancelled",
            "result": answer,
            "artifacts": [],
            "missing": [],
            "agent_id": agent_id,
            "iterations": iterations,
            "cancelled": true,
            "warnings": ["任务被用户取消"]
        }),
        summary: Some(format!("Agent「{}」任务已取消", agent_id)),
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

            // 文档生成结果：提取 file_id（主 Agent 的文档卡片与 metadata 持久化靠此回传）
            if result_type == "document_write_result" && data["success"].as_bool() == Some(true) {
                if let Some(file_id) = data["file_id"].as_str() {
                    artifacts.push(json!({
                        "type": "document_created",
                        "file_id": file_id,
                        "file_name": data["file_name"].as_str().unwrap_or(""),
                        "file_type": data["file_type"].as_str().unwrap_or(""),
                        "file_size": data["file_size"].as_i64().unwrap_or(0),
                        "tool": tool_name
                    }));
                }
            }
        }
    }
    artifacts
}

fn make_success(agent_id: &str, answer: &str, iterations: i64, tool_log: &[Value]) -> String {
    let artifacts = extract_artifacts(tool_log);

    // ★ warnings：收集失败的工具结果
    let mut warnings: Vec<Value> = Vec::new();
    let mut any_failed = false;
    for round in tool_log {
        let Some(tools) = round["tools"].as_array() else { continue };
        for tool in tools {
            if let Ok(envelope) = serde_json::from_str::<Value>(tool["result"].as_str().unwrap_or("")) {
                // 失败判定双保险：显式 success:false，或带 error 字段但未声明 success:true（旧式信封）
                let d = &envelope["data"];
                let failed = d["success"].as_bool() == Some(false)
                    || (!d["error"].is_null() && d["success"].as_bool() != Some(true));
                if failed {
                    any_failed = true;
                    warnings.push(json!({
                        "tool": tool["tool"].as_str().unwrap_or(""),
                        "error": envelope["summary"].as_str().unwrap_or("操作失败")
                    }));
                }
            }
        }
    }

    // ★ MISSING 解析：子Agent声明的信息缺口，从正文剥离（容忍列表符/加粗/全角冒号等常见变体）
    let mut missing: Vec<String> = Vec::new();
    let result_text = {
        let kept: Vec<&str> = answer.lines().filter(|line| {
            let t = line.trim()
                .trim_start_matches(['-', '*', '#', '>', ' '])
                .trim_start_matches('*')
                .trim_start();
            match t.strip_prefix("MISSING") {
                Some(rest) => {
                    let m = rest.trim_start_matches([':', '：', '*', ' ']).trim().to_string();
                    if !m.is_empty() { missing.push(m); }
                    false
                }
                None => true,
            }
        }).collect();
        kept.join("\n").trim().to_string()
    };

    // ★ 三态判定：partial（有MISSING声明）> failed（有失败且无任何成功写入）> completed
    let status = if !missing.is_empty() { "partial" }
        else if any_failed && artifacts.is_empty() { "failed" }
        else { "completed" };

    let preview: String = result_text.chars().take(200).collect();
    let status_label = match status { "partial" => "部分完成", "failed" => "执行失败", _ => "已完成任务" };
    let envelope = super::agent_workflow::ToolResultEnvelope {
        result_type: "delegate_agent_result".to_string(),
        data: json!({
            "status": status,
            "result": result_text,
            "artifacts": artifacts,
            "missing": missing,
            "agent_id": agent_id,
            "iterations": iterations,
            "warnings": warnings
        }),
        summary: Some(format!("Agent「{}」{}。{}", agent_id, status_label, preview)),
    };
    envelope.to_json()
}
