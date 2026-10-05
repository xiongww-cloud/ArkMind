//! delegate_agent — 子 Agent 委托执行模块
//! 共享上下文模式：子 Agent 直接复用主会话 messages 快照（截尾防御 + 尾部指令注入），
//! 头部 system 与主 Agent 字节级一致以保前缀缓存；工具集硬白名单隔离

use serde_json::{json, Value};
use crate::database;
use crate::services::agent_workflow::{ChatMessage, MessageRole, SessionMemory};
use tauri::Emitter;

/// 安全截断字符串（尊重 UTF-8 字符边界）
fn truncate_str(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
}

/// 快照截尾（协议 400 防御）：剥离尾部未闭合的 Assistant(tool_calls)。
/// 委托发生在工具执行期间——对应的 Tool 响应尚未写入历史，直接把这种"开口"序列
/// 发给 OpenAI 兼容端点会触发 400（tool_calls 后必须紧跟同 id 的 tool 消息）。
/// 纯函数，可单测。
pub(crate) fn truncate_dangling_tool_calls(mut msgs: Vec<ChatMessage>) -> Vec<ChatMessage> {
    while let Some(last) = msgs.last() {
        let dangling = last.role == MessageRole::Assistant
            && last.tool_calls.as_ref().is_some_and(|v| !v.is_empty());
        if dangling {
            msgs.pop();
        } else {
            break;
        }
    }
    msgs
}

/// Hard Masking：子 Agent 工具白名单硬剔除判定（纯函数，可单测）。
/// delegate（递归委托死循环风险）与 MAIN_AGENT_ONLY（delegate/memory 主 Agent 专属）
/// 对子 Agent 物理不可见——即使 agent_tool_assignments 出现脏数据分配也会被过滤。
pub(crate) fn is_sub_agent_forbidden_tool(name: &str) -> bool {
    name == "ext_tool_delegate_agent"
        || crate::services::agent_workflow::tools::MAIN_AGENT_ONLY.contains(&name)
}

/// 执行 delegate_agent 工具
/// 从数据库加载 Agent Profile，以共享上下文模式调用 agent_chat_workflow 执行任务，返回结果
/// delegate_depth: 委托深度（0=主Agent直接委托，≥1=已嵌套，拒绝再委托）
/// history: 主会话 messages 快照（调用点 clone，入口截尾防御后作为子 Agent 初始上下文）
/// main_session: 主会话工作记忆——子 Agent 写穿共享（visited/tool_history/token 累计直接回写），
///   顺序 await 无竞态，返回后主会话天然持有子 Agent 的全部状态增量
pub async fn execute_delegate_agent(
    agent_id: &str,
    task: &str,
    context: Option<&Value>,
    expected_output: &str,
    app_handle: Option<tauri::AppHandle>,
    delegate_depth: u8,
    history: Vec<ChatMessage>,
    main_session: &mut SessionMemory,
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

    // ★ 一次 JOIN 查询加载 Agent Profile + 工具分配（含 name 用于状态展示；model/temperature 用于 per-agent LLM 覆盖）
    let rows = match database::all(
        "SELECT p.name, p.system_prompt, p.max_iterations, p.model, p.temperature, COALESCE(p.output_format,'natural') AS output_format, a.tool_name FROM agent_profiles p LEFT JOIN agent_tool_assignments a ON a.agent_id=p.id WHERE p.id=?1 AND p.status='active'",
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
    // ★ Hard Masking：子 Agent 工具白名单硬剔除——delegate（防递归死循环）与 MAIN_AGENT_ONLY
    //   （delegate/memory 主 Agent 专属）物理不可见，即使 agent_tool_assignments 有脏数据分配也被过滤
    if rows.iter().any(|r| r["tool_name"].as_str() == Some("ext_tool_delegate_agent")) {
        log::warn!("[DelegateAgent] 检测到为子 Agent 分配 delegate 工具（已硬剔除）: agent_id={}", agent_id);
    }
    // ★ 主 Agent 专属工具清单：以 is_sub_agent_forbidden_tool 为唯一数据源（MAIN_AGENT_ONLY + delegate）
    let allowed_tools: Vec<String> = rows.iter()
        .filter_map(|r| r["tool_name"].as_str().map(|s| s.to_string()))
        .filter(|t| !is_sub_agent_forbidden_tool(t))
        .collect();

    // system_prompt 直接用 Agent 配置的提示词（工具说明已由 API tools schema 提供，不再重复注入）
    let system_prompt = first["system_prompt"].as_str().unwrap_or("").to_string();
    let max_iterations = first["max_iterations"].as_i64().unwrap_or(20) as i32;
    let agent_name = first["name"].as_str().unwrap_or(agent_id).to_string();
    // ★ 产出类别（单一落点 OutputClass）：协议注入/终裁语义/直出口径全部跟随
    let out_class = crate::services::agent_workflow::output_class::OutputClass::from_format(
        first["output_format"].as_str().unwrap_or("natural"),
    );
    // ★ per-agent 模型/温度：空串/NULL = 跟随全局配置
    let llm = crate::services::agent_workflow::LlmOverrides {
        model: first["model"].as_str().filter(|m| !m.is_empty()).map(String::from),
        temperature: first["temperature"].as_f64(),
        agent_id: Some(agent_id.to_string()),
    };

    // 构建共享上下文快照：截尾防御（剥未闭合 tool_calls）+ 尾部注入执行体指令
    let mut sub_messages = truncate_dangling_tool_calls(history);
    let history_len = sub_messages.len();
    let directive = build_directive(agent_id, &agent_name, &system_prompt, task, context, expected_output, &allowed_tools, out_class);
    sub_messages.push(ChatMessage {
        role: MessageRole::User,
        content: Some(directive),
        tool_calls: None,
        tool_call_id: None,
        name: None,
        reasoning_content: None,
    });

    log::info!(
        "[DelegateAgent] 委托 Agent「{}」，任务: {}，工具数: {}，共享历史 {} 条（截尾后，尾部注入执行体指令），model={:?}, temperature={:?}",
        agent_name,
        truncate_str(task, 50),
        allowed_tools.len(),
        history_len,
        llm.model,
        llm.temperature
    );

    // ★ 委托可见性：status 携带结构化身份（agent_id/agent_name），前端状态条显示当前干活的子 Agent
    let emit_handle = app_handle.clone();
    if let Some(ref h) = emit_handle {
        let _ = h.emit("agent:status", json!({"stage":"delegating","agent_id":agent_id,"agent_name":agent_name,"msg":format!("委托「{}」处理中...", agent_name)}));
        let _ = h.emit("agent:progress", json!({"msg":format!("已路由到「{}」，处理中...", agent_name)}));
    }

    // 使用 Box::pin 解决递归 async 调用（session 写穿共享：无独立 load/save，状态直接回写主会话）
    let result = Box::pin(super::agent_workflow::agent_chat_workflow_internal(
        task.to_string(),
        vec![],
        Some(max_iterations),
        app_handle,
        false, // ★ silent=false：子 Agent 的 token/进度直连前端流式输出（回复不再只在刷新后可见）
        None, // ★ 无独立 session：load/save 有守卫，避免4次无用DB操作
        Some(allowed_tools),
        None,            // ★ system_prompt=None：头部保主 Agent 完整 system（前缀缓存），职责经尾部指令注入
        false,            // ★ is_root=false：不重置/不清除 AGENT_CANCEL
        delegate_depth + 1, // ★ 深度+1
        true,             // ★ cancel_aware=true：委托链响应聊天"停止"，取消向上传播
        false,            // ★ 子Agent保留完整动态上下文（工作记忆/预算对其有价值）
        Some(sub_messages), // ★ 共享历史模式：截尾快照 + 尾部执行体指令
        true,             // ★ canonical tools：与主 Agent同一份全量清单（tools 计入前缀缓存，字节不一致则共享历史段全灭）
        Some(super::agent_workflow::DelegateRun {
            session: main_session,
            agent_id: agent_id.to_string(),
            agent_name: agent_name.clone(),
            llm,
        }),
    ))
    .await;

    // ★ 委托结束：广播 delegate-done（前端据此清除"当前子 Agent"状态条；
    //   精确的 completed/partial/failed 语义在 make_success 信封里，此处仅取粗状态供 UI）
    let rough_status = match &result {
        Ok(v) if v["cancelled"].as_bool() == Some(true) => "cancelled",
        Ok(v) if !v["answer"].as_str().unwrap_or("").is_empty() => "completed",
        Ok(_) => "failed",
        Err(_) => "failed",
    };
    if let Some(ref h) = emit_handle {
        let _ = h.emit("agent:status", json!({"stage":"delegate-done","agent_id":agent_id,"agent_name":agent_name,"delegate_status":rough_status}));
    }

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
                return make_cancelled(&agent_name, answer, iterations);
            }

            make_success(&agent_name, answer, iterations, tool_log, out_class)
        }
        Err(e) => {
            log::error!("[DelegateAgent] Agent「{}」执行失败: {}", agent_name, e);
            make_error(&format!("Agent「{}」执行失败: {}", agent_name, e))
        }
    }
}

/// 构建尾部执行体指令（角色覆盖声明 + 职责 + 任务 + 协议，合成一条 User 消息追加在历史尾部）。
/// 头部 system 保持主 Agent 原样（前缀缓存），子 Agent 的身份与职责在此声明并覆盖冲突部分。
fn build_directive(
    agent_id: &str,
    agent_name: &str,
    system_prompt: &str,
    task: &str,
    context: Option<&Value>,
    expected_output: &str,
    allowed_tools: &[String],
    out_class: crate::services::agent_workflow::output_class::OutputClass,
) -> String {
    let mut parts = Vec::new();

    // 1. 角色覆盖声明（历史已全量共享：无需再摘要 recent_messages/user_query）
    parts.push(format!(
        "[系统指令：专精执行体模式 - 角色覆盖声明]\n\
        你当前处于专精执行体（Sub-Agent: {}（{}））模式接管此任务。\n\
        【上下文政策】前序对话历史均为客观事实背景与已定稿内容，不得推翻或违背；历史中被后续修订的中间讨论以最终定稿为准。\n\
        【职责与边界】{}\n\
        【覆盖约束】system 提示词中与上述职责冲突的部分（委托其他 Agent、任务规划、输出规范等）一律以本指令为准；你的工具以可用清单为准，没有委托能力。",
        agent_name, agent_id, system_prompt
    ));

    // 2. context 结构化补充（主 Agent 显式给出的关键引用——历史可见的内容无需重复）
    if let Some(ctx) = context {
        if !ctx.is_null() {
            if let Some(obj) = ctx.as_object() {
                let filtered: Vec<(&String, &Value)> = obj.iter()
                    .filter(|(k, _)| *k != "user_query" && *k != "recent_messages")
                    .collect();
                if !filtered.is_empty() {
                    let mut ctx_obj = serde_json::Map::new();
                    for (k, v) in filtered { ctx_obj.insert(k.clone(), v.clone()); }
                    parts.push(format!("【补充上下文】{}", Value::Object(ctx_obj)));
                }
            }
        }
    }

    // 3. 可用工具清单（canonical tools 下 schema 为全量清单，此处收敛本 Agent 实际可调用子集）
    if allowed_tools.is_empty() {
        parts.push("注意：你当前没有分配任何工具，无法执行任何操作。不要编造信息，直接回复最后一行：MISSING: 当前 Agent 未配置任何工具".to_string());
    } else {
        parts.push(format!(
            "【可用工具清单】你只能调用：{}。tools schema 中出现的其他工具对你不可用（调用会返回\"未授权\"），跳过即可，不要重试。",
            allowed_tools.join("、")
        ));
    }

    // 4. 任务本体
    parts.push(format!("【执行任务】{}", task));

    // 5. 期望输出格式
    if !expected_output.is_empty() {
        parts.push(format!("【期望输出格式】{}", expected_output));
    }

    // 6. 执行协议（运行时统一注入；文本按 OutputClass 取用）
    parts.push(out_class.directive_protocol().to_string());

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


fn make_success(agent_id: &str, answer: &str, iterations: i64, tool_log: &[Value], out_class: crate::services::agent_workflow::output_class::OutputClass) -> String {
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

    // ★ MISSING / NEED_USER 解析：信息缺口与确认请求从正文剥离（容忍列表符/加粗/全角冒号等常见变体）
    let mut missing: Vec<String> = Vec::new();
    let mut user_questions: Vec<String> = Vec::new();
    let result_text = {
        let kept: Vec<&str> = answer.lines().filter(|line| {
            let t = line.trim()
                .trim_start_matches(['-', '*', '#', '>', ' '])
                .trim_start_matches('*')
                .trim_start();
            if let Some(rest) = t.strip_prefix("NEED_USER") {
                let q = rest.trim_start_matches([':', '：', '*', ' ']).trim().to_string();
                if !q.is_empty() { user_questions.push(q); }
                return false;
            }
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

    // ★ 三态判定：OutputClass 策略收口（交付型缺口回贴保持直出；NEED_USER 交回主 Agent 转卡片）
    let (result_text, status, effective_missing) =
        out_class.finalize(result_text, &missing, &user_questions, any_failed, !artifacts.is_empty());

    let preview: String = result_text.chars().take(200).collect();
    let status_label = match status { "partial" => "部分完成", "failed" => "执行失败", _ => "已完成任务" };
    let envelope = super::agent_workflow::ToolResultEnvelope {
        result_type: "delegate_agent_result".to_string(),
        data: json!({
            "status": status,
            "result": result_text,
            "artifacts": artifacts,
            "missing": effective_missing,
            "agent_id": agent_id,
            "iterations": iterations,
            "warnings": warnings
        }),
        summary: Some(format!("Agent「{}」{}。{}", agent_id, status_label, preview)),
    };
    envelope.to_json()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::agent_workflow::{FunctionCall, ToolCall};

    fn user_msg(s: &str) -> ChatMessage {
        ChatMessage { role: MessageRole::User, content: Some(s.into()), tool_calls: None, tool_call_id: None, name: None, reasoning_content: None }
    }
    fn tool_msg(id: &str) -> ChatMessage {
        ChatMessage { role: MessageRole::Tool, content: Some("ok".into()), tool_calls: None, tool_call_id: Some(id.into()), name: None, reasoning_content: None }
    }
    fn assistant_with_tcs(id: &str) -> ChatMessage {
        ChatMessage {
            role: MessageRole::Assistant,
            content: None,
            tool_calls: Some(vec![ToolCall {
                id: id.into(),
                call_type: "function".into(),
                function: FunctionCall { name: "ext_tool_search".into(), arguments: "{}".into() },
            }]),
            tool_call_id: None,
            name: None,
            reasoning_content: None,
        }
    }
    fn assistant_plain(s: &str) -> ChatMessage {
        ChatMessage { role: MessageRole::Assistant, content: Some(s.into()), tool_calls: None, tool_call_id: None, name: None, reasoning_content: None }
    }

    /// 验收1：尾部未闭合 Assistant(tool_calls) 必须被精准剥离，最后一条消息闭合
    #[test]
    fn truncates_dangling_assistant_tool_calls_at_tail() {
        let msgs = vec![
            user_msg("问题"),
            assistant_with_tcs("call_1"),
            tool_msg("call_1"),
            assistant_with_tcs("call_2"), // 委托发生时：本轮 tool_calls 已入史、Tool 结果未入史
        ];
        let out = truncate_dangling_tool_calls(msgs);
        assert_eq!(out.len(), 3);
        assert!(out.last().unwrap().role == MessageRole::Tool);
    }

    #[test]
    fn truncates_consecutive_dangling_assistant_messages() {
        let msgs = vec![
            user_msg("问题"),
            assistant_with_tcs("call_a"),
            assistant_with_tcs("call_b"),
        ];
        let out = truncate_dangling_tool_calls(msgs);
        assert_eq!(out.len(), 1);
        assert!(out.last().unwrap().role == MessageRole::User);
    }

    /// 已闭合/普通尾部不得误删
    #[test]
    fn keeps_closed_and_plain_tails() {
        // 尾部 User → 保留
        let out = truncate_dangling_tool_calls(vec![assistant_with_tcs("c"), user_msg("q")]);
        assert_eq!(out.len(), 2);
        // 尾部 Tool（闭合）→ 保留
        let out = truncate_dangling_tool_calls(vec![assistant_with_tcs("c"), tool_msg("c")]);
        assert_eq!(out.len(), 2);
        // 尾部 assistant 无 tool_calls → 保留
        let out = truncate_dangling_tool_calls(vec![user_msg("q"), assistant_plain("答案")]);
        assert_eq!(out.len(), 2);
        // 空数组 → 空
        assert!(truncate_dangling_tool_calls(vec![]).is_empty());
    }

    /// tool_calls 为空数组的 assistant 不算 dangling（防御可选语义）
    #[test]
    fn empty_tool_calls_vector_is_not_dangling() {
        let mut m = assistant_plain("x");
        m.tool_calls = Some(vec![]);
        let out = truncate_dangling_tool_calls(vec![user_msg("q"), m]);
        assert_eq!(out.len(), 2);
    }

    /// build_directive：必须包含角色覆盖、职责、任务、协议、MISSING 约定
    #[test]
    fn directive_contains_required_sections() {
        let ctx = json!({"product_ids": ["P-1"]});
        let allowed = vec!["ext_tool_write_document".to_string()];
        let d = build_directive("document_writer", "文档助手", "你是转换器", "把定稿转为 Word", Some(&ctx), "review", &allowed, crate::services::agent_workflow::output_class::OutputClass::Natural);
        assert!(d.contains("专精执行体"));
        assert!(d.contains("【职责与边界】你是转换器"));
        assert!(d.contains("【执行任务】把定稿转为 Word"));
        assert!(d.contains("【补充上下文】"));
        assert!(d.contains("【期望输出格式】review"));
        assert!(d.contains("MISSING"));
        assert!(d.contains("没有委托能力"));
        // MISSING=向主 Agent 提问 语义 + 选项格式 + 相似项工具层复用（禁止"先向用户确认"死路表述）
        assert!(d.contains("向主 Agent 提问"));
        assert!(d.contains("<问题> | <选项A> / <选项B>"));
        assert!(d.contains("完全同名由工具层自动复用"));
        assert!(!d.contains("先向用户确认"), "子 Agent 无提问用户通道，不应再出现该指令");
        // 无工具提示路径
        let d2 = build_directive("x", "y", "p", "t", None, "", &[], crate::services::agent_workflow::output_class::OutputClass::Natural);
        assert!(d2.contains("未配置任何工具"));
    }

    /// 交付型指令：回答即交付文档、禁 MISSING 通道、协议优先于角色提示词
    #[test]
    fn directive_deliverable_protocol() {
        let allowed = vec!["ext_tool_object".to_string()];
        let d = build_directive("solution", "解决方案助手", "p", "t", None, "", &allowed, crate::services::agent_workflow::output_class::OutputClass::Deliverable);
        assert!(d.contains("执行协议（交付型产出）"));
        assert!(d.contains("你的回答就是最终交付文档"));
        assert!(d.contains("不使用 MISSING: 行"));
        assert!(d.contains("NEED_USER"), "交付型须有 NEED_USER 确认卡片通道");
        assert!(d.contains("本协议优先于角色提示词"));
        assert!(!d.contains("在回复的最后一行输出 MISSING"), "交付型不得注入执行型 MISSING 协议");
    }


    /// canonical tools 下 schema 为全量清单，指令必须显式收敛本 Agent 实际可调用子集
    #[test]
    fn directive_lists_allowed_tools_explicitly() {
        let allowed = vec!["ext_tool_object".to_string(), "ext_tool_manage_todo".to_string()];
        let d = build_directive("business", "业务助手", "p", "t", None, "", &allowed, crate::services::agent_workflow::output_class::OutputClass::Natural);
        assert!(d.contains("【可用工具清单】"));
        assert!(d.contains("ext_tool_object、ext_tool_manage_todo"));
        assert!(!d.contains("ext_tool_delegate_agent"));
        assert!(!d.contains("ext_tool_write_document"));
    }

    /// Hard Masking：delegate 与 MAIN_AGENT_ONLY 对子 Agent 物理不可见
    #[test]
    fn hard_masking_forbids_delegate_and_main_only_tools() {
        assert!(is_sub_agent_forbidden_tool("ext_tool_delegate_agent"));
        assert!(is_sub_agent_forbidden_tool("ext_tool_memory"));
        assert!(is_sub_agent_forbidden_tool("ext_tool_ask_user"));
        assert!(!is_sub_agent_forbidden_tool("ext_tool_write_document"));
        assert!(!is_sub_agent_forbidden_tool("ext_tool_search"));
    }
}
