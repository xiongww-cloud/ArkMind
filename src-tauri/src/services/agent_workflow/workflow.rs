//! 工作流主体：附件提炼 + ReAct 主循环（agent_chat_workflow / _internal）

use serde_json::{json, Value};
use crate::services::agent_workflow::*;

/// 大附件切片提炼阈值：单附件超过该字符数则不再截断，改为 LLM 提炼关键信息
const ATTACH_SUMMARIZE_THRESHOLD: usize = 100000;
/// 提炼切片大小（字符）
const ATTACH_SUMMARIZE_CHUNK: usize = 6000;
/// 提炼片数上限（防病态超大文件打爆 API）
const ATTACH_SUMMARIZE_MAX_CHUNKS: usize = 30;
/// 提炼提示词：通用关键信息提炼，不限定领域
const ATTACH_SUMMARIZE_PROMPT: &str = "你是信息提炼助手。请从以下文件片段中提炼并保留关键信息，删除冗余表述，用简洁的中文输出精炼摘要。";

/// 大附件切片提炼：split → 每片 LLM 提炼 → 按序汇总
/// 超过 ATTACH_SUMMARIZE_THRESHOLD 字符的附件不再截断，而是切片后由 LLM 提炼关键信息。
/// 最终以「片段N摘要」块拼接输出，体积远小于原文且不丢失关键内容。
async fn summarize_large_attachment(
    text: &str,
    app_handle: &Option<tauri::AppHandle>,
) -> String {
    let chunks = crate::services::smart_chunker::split_into_parents(text, ATTACH_SUMMARIZE_CHUNK);
    let total = chunks.len().min(ATTACH_SUMMARIZE_MAX_CHUNKS);

    // 预收集为 owned 数据，避免 futures::stream::iter 的借用生命周期冲突
    let chunk_data: Vec<(usize, String)> = chunks.iter().take(total).enumerate()
        .map(|(i, s)| (i, s.clone()))
        .collect();

    let results: Vec<String> = futures::stream::iter(chunk_data)
        .map(|(i, chunk)| async move {
            if let Some(ref h) = app_handle {
                let _ = h.emit("agent:progress", serde_json::json!({"msg": format!("正在提炼附件… {}/{}", i + 1, total)}));
            }
            let msgs = vec![
                ChatMessage { role: MessageRole::System, content: Some(ATTACH_SUMMARIZE_PROMPT.to_string()), tool_calls: None, tool_call_id: None, name: None, reasoning_content: None },
                ChatMessage { role: MessageRole::User, content: Some(chunk.clone()), tool_calls: None, tool_call_id: None, name: None, reasoning_content: None },
            ];
            match call_llm_streaming(&msgs, None, app_handle).await {
                Ok((content, ..)) => content.trim().to_string(),
                Err(e) => {
                    log::warn!("[大附件提炼] 片段 {} 提炼失败: {}", i + 1, e);
                    let cut = crate::services::file_parser::truncate_at_boundary(&chunk, 2000);
                    format!("（片段提炼失败: {}）\n{}", e, cut)
                }
            }
        })
        .buffered(3)
        .collect::<Vec<_>>()
        .await;

    let mut summary = results.into_iter().enumerate()
        .map(|(i, s)| format!("【片段{}】\n{}", i + 1, s))
        .collect::<Vec<_>>()
        .join("\n\n");

    // 超出 30 片的部分：结构截断追加（带提示），避免丢失
    if chunks.len() > ATTACH_SUMMARIZE_MAX_CHUNKS {
        let rest = chunks[ATTACH_SUMMARIZE_MAX_CHUNKS..].join("\n");
        let cut = crate::services::file_parser::truncate_at_boundary(&rest, 4000);
        summary.push_str(&format!("\n\n（其余 {} 片内容较长已略过）\n{}", chunks.len() - ATTACH_SUMMARIZE_MAX_CHUNKS, cut));
    }

    if summary.trim().is_empty() { "（大附件提炼结果为空）".to_string() } else { summary }
}

/// 委托直出判定（纯函数，可单测）。
/// 输入：本轮全部可解析结果的 (result_type, status, answer, has_missing) 四元组。
/// 仅当其中恰好 1 个委托信封（delegate_agent_result）且回答合格时返回 Some：
/// - ≥2 个委托：多份回答必须交回主 LLM 综合，否则第二个及之后的回答文本被丢弃
/// - partial 且 missing 非空：MISSING 声明已从正文剥离，直出会把缺口藏起来——
///   交回主 LLM，由它决定补齐/追问（衔接 ext_tool_ask_user）
/// - 0 个委托 / failed / 空回答：None，走常规主 LLM 路径
pub(crate) fn pick_direct_out(results: &[(String, String, String, bool)]) -> Option<String> {
    let delegates: Vec<&(String, String, String, bool)> = results.iter()
        .filter(|(rt, _, _, _)| rt == "delegate_agent_result")
        .collect();
    if delegates.len() != 1 { return None; }
    let (_, status, answer, has_missing) = delegates[0];
    let qualified = (status == "completed" || (status == "partial" && !has_missing)) && !answer.is_empty();
    if qualified { Some(answer.clone()) } else { None }
}

/// ext_tool_ask_user 参数归一化（纯函数，可单测）：
/// - 问题截到 ≤5（超出丢弃并置 truncated）、每题选项截到 ≤6、剔除空 prompt/空选项
/// - type 非 single_choice/multiple_choice 一律降级为 text
/// - question_id 缺失时自动补 qN
/// - 无合格问题 → 失败信封（提示 LLM 正确用法）
pub(crate) fn normalize_ask_args(a: &Value) -> String {
    let reason = a["reason"].as_str().unwrap_or("").trim().to_string();
    let raw_qs = a["questions"].as_array().cloned().unwrap_or_default();
    let mut questions: Vec<Value> = Vec::new();
    let mut truncated = false;
    for q in raw_qs {
        let prompt = q["prompt"].as_str().unwrap_or("").trim().to_string();
        if prompt.is_empty() { continue; }
        if questions.len() >= 5 { truncated = true; break; }
        let qtype = match q["type"].as_str() {
            Some("single_choice") | Some("multiple_choice") => q["type"].as_str().unwrap(),
            _ => "text",
        };
        let options: Vec<String> = q["options"].as_array()
            .map(|arr| arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.trim().to_string()))
                .filter(|s| !s.is_empty())
                .take(6)
                .collect())
            .unwrap_or_default();
        let qid = q["question_id"].as_str()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("q{}", questions.len() + 1));
        questions.push(json!({"question_id": qid, "prompt": prompt, "type": qtype, "options": options}));
    }
    if questions.is_empty() {
        let envelope = ToolResultEnvelope {
            result_type: "ask_user_result".to_string(),
            data: json!({"status": "invalid", "error": "questions 中没有任何含 prompt 的有效问题"}),
            summary: Some("追问无效：至少需要一个包含 prompt 的问题。常规可默认的信息不要追问。".to_string()),
        };
        return envelope.to_json();
    }
    ToolResultEnvelope {
        result_type: "ask_user_result".to_string(),
        data: json!({"status": "pending", "reason": reason, "questions": questions, "truncated": truncated}),
        summary: Some(format!("已向用户发出 {} 个追问（本轮结束，等待用户答复）", questions.len())),
    }.to_json()
}

/// 追问文本版答案（纯函数，可单测）：写入历史/流式兜底，结构化卡片另走 clarification 字段
pub(crate) fn format_clarification_answer(reason: &str, questions: &[Value]) -> String {
    let mut s = String::from("【需要确认】");
    if !reason.is_empty() { s.push_str(&format!(" {}", reason)); }
    for (i, q) in questions.iter().enumerate() {
        s.push_str(&format!("\n{}. {}", i + 1, q["prompt"].as_str().unwrap_or("")));
        if let Some(opts) = q["options"].as_array() {
            let joined = opts.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join(" / ");
            if !joined.is_empty() { s.push_str(&format!("\n   {}", joined)); }
        }
    }
    s.push_str("\n\n请在上方卡片选择或直接回复补充信息，我将继续推进。");
    s
}

/// 追问早退判定（纯函数，可单测）：本轮存在合格 ask_user_result（status=pending 且含问题）
/// → 返回第一个的 data。追问语义是"本轮立即结束等待用户"，优先级高于委托直出。
pub(crate) fn pick_clarification_out(parsed: &[Value]) -> Option<Value> {
    parsed.iter().find_map(|v| {
        if v["result_type"].as_str()? != "ask_user_result" { return None; }
        let data = v.get("data")?;
        if data["status"].as_str()? != "pending" { return None; }
        data["questions"].as_array().filter(|a| !a.is_empty())?;
        Some(data.clone())
    })
}

/// 构建委托执行快照（纯函数，可单测）：在共享历史基础上让子 Agent 看到本轮已完成的工具结果。
/// 消息尾部必然是本轮 assistant(tool_calls)（结果尚未回写）。此处将其裁剪为
/// "仅含已完成调用的 assistant + 对应 tool 响应"——序列对端点合法（每个 tool_call 都有响应），
/// 后一个委托因此能看到前一个委托/检索的结果；无已完成结果时退化为仅截尾（原行为）。
pub(crate) fn build_delegate_snapshot(
    messages: &[ChatMessage],
    completed: &[(usize, String)],
) -> Vec<ChatMessage> {
    let mut snap: Vec<ChatMessage> = messages.to_vec();
    let tail_is_assistant = snap.last().map(|m| {
        m.role == MessageRole::Assistant && m.tool_calls.as_ref().map_or(false, |v| !v.is_empty())
    }).unwrap_or(false);
    if !tail_is_assistant { return snap; }
    let tail = snap.pop().expect("tail checked above");
    let all_tcs = tail.tool_calls.unwrap_or_default();
    if completed.is_empty() { return snap; }
    // 按原始调用顺序取已完成者
    let mut done: Vec<(usize, &String)> = completed.iter().map(|(i, r)| (*i, r)).collect();
    done.sort_by_key(|(i, _)| *i);
    let mut kept: Vec<ToolCall> = Vec::new();
    let mut replies: Vec<ChatMessage> = Vec::new();
    for (idx, result) in done {
        let Some(tc) = all_tcs.get(idx) else { continue };
        kept.push(tc.clone());
        replies.push(ChatMessage {
            role: MessageRole::Tool,
            content: Some(result.clone()),
            tool_calls: None,
            tool_call_id: Some(tc.id.clone()),
            name: Some(tc.function.name.clone()),
            reasoning_content: None,
        });
    }
    if kept.is_empty() { return snap; }
    snap.push(ChatMessage {
        role: MessageRole::Assistant,
        content: None,
        tool_calls: Some(kept),
        tool_call_id: None,
        name: None,
        reasoning_content: tail.reasoning_content,
    });
    snap.extend(replies);
    snap
}

#[tauri::command]
pub async fn agent_chat_workflow(user_query: String, attachments: Vec<Value>, max_iterations: Option<i32>, app_handle: Option<tauri::AppHandle>, silent: bool, session_id: Option<String>, allowed_tools: Option<Vec<String>>) -> Result<Value, String> {
    agent_chat_workflow_internal(user_query, attachments, max_iterations, app_handle, silent, session_id, allowed_tools, None, true, 0, true, false, None, true, None).await
}

/// 工具名 → 中文能力标签（单一真相源：agent 清单注入与进度事件共用）
pub(crate) fn tool_display_label(name: &str) -> &'static str {
    match name {
        "ext_tool_object" => "业务对象查询",
        "ext_tool_search" => "知识库检索",
        "ext_tool_chunk_navigate" => "文档翻页",
        "ext_tool_web_search" => "联网搜索",
        "ext_tool_manage_product" => "产品管理(读写)",
        "ext_tool_manage_project" => "项目管理(读写)",
        "ext_tool_manage_todo" => "待办管理(读写)",
        "ext_tool_manage_partner" => "合作伙伴管理(读写)",
        "ext_tool_manage_contact" => "联系人管理(读写)",
        "ext_tool_manage_note" => "笔记管理(读写)",
        "ext_tool_relations" => "关联查询",
        "ext_tool_memory" => "记忆管理",
        "ext_tool_report" => "上报缺口/确认",
        "ext_tool_write_document" => "文档生成",
        "ext_tool_delegate_agent" => "委托专业助手",
        "ext_tool_ask_user" => "向用户提问",
        _ => "其他",
    }
}

/// 委托运行上下文（共享历史模式专用）：
/// - session: 直接借用主会话（写穿共享——子 Agent 的 visited/tool_history/token 累计
///   无需合并即回写主会话；顺序 await 无并发竞态），工作记忆不再每次清零
/// - agent_id/agent_name: 结构化可观测性（agent:status 事件携带，前端展示当前干活的子 Agent）
/// - llm: per-agent 模型/温度覆盖（NULL/空 = 跟随全局配置）
pub struct DelegateRun<'a> {
    pub session: &'a mut SessionMemory,
    pub agent_id: String,
    pub agent_name: String,
    pub llm: LlmOverrides,
}

/// 历史消息 metadata.documents → 附加到 content 尾部。
/// 与前端文档卡片一致：用户在回答后看到的文件，AI 在上下文里也看到（文件名 + file_id，可 read 回读/告知下载）。
/// 不限数量；无 documents 或解析失败时 no-op。纯函数，可单测。
pub(crate) fn append_documents_note(content: &mut String, metadata: Option<&str>) {
    let Some(meta) = metadata.filter(|m| !m.is_empty()) else { return };
    let Ok(v) = serde_json::from_str::<Value>(meta) else { return };
    let Some(docs) = v["documents"].as_array().filter(|a| !a.is_empty()) else { return };
    let mut any = false;
    let mut note = String::from("\n\n[本条回复生成的文档]");
    for d in docs {
        let name = d["file_name"].as_str().unwrap_or("");
        let fid = d["file_id"].as_str().unwrap_or("");
        if name.is_empty() && fid.is_empty() { continue; }
        any = true;
        if fid.is_empty() {
            note.push_str(&format!("\n- {}", name));
        } else {
            note.push_str(&format!("\n- {}（file_id: {}）", name, fid));
        }
    }
    if any {
        content.push_str(&note);
    }
}

/// 历史消息 metadata.assistants（委托产出的子 Agent 署名）→ 给 LLM 上下文追加"直出交付"运行期标注。
/// 委托直出后，该正文以主 Agent 的 assistant 消息身份进入历史；无标注时主 LLM 会把交付稿
/// 当成自己写的，用户一句"继续/修改"就自行代笔续写（继任漂移）。标注只存在于重建的 LLM
/// 消息中，不污染 DB 与用户可见文本。纯函数，可单测。
pub(crate) fn append_delegated_note(content: &mut String, metadata: Option<&str>) {
    let Some(meta) = metadata.filter(|m| !m.is_empty()) else { return };
    let Ok(v) = serde_json::from_str::<Value>(meta) else { return };
    let names: Vec<&str> = match v["assistants"].as_array() {
        Some(arr) => arr.iter().filter_map(|x| x.as_str()).filter(|s| !s.is_empty()).collect(),
        None => return,
    };
    if names.is_empty() { return; }
    content.push_str(&format!(
        "\n\n（本条正文由「{}」直出交付，未经改写；用户对该交付稿的任何修改/续写/补充章节请求，必须再次委托该 Agent 完成，不得代笔续写）",
        names.join("、")
    ));
}

/// 内部工作流函数，支持自定义 system_prompt
/// is_root: true=顶层工作流（控制 AGENT_CANCEL 生命周期），false=子Agent（保留取消信号向上传播）
/// delegate_depth: 当前委托深度（0=主Agent，≥1=子Agent）
/// respond_cancel: false=独立后台运行体，完全无视全局 AGENT_CANCEL（与聊天"停止"互不干扰）
/// shared_history: Some=共享上下文模式（委托子Agent）：直接以主会话消息快照起步，
///   跳过新会话构造/本轮 query 追加/system 重写——messages[0] 保持主 Agent 完整 system，
///   与主会话形成字节级一致前缀，最大化端点 Prefix Caching 命中
/// canonical_tools: true=主聊天/共享历史子Agent——请求级 tools 用同一份全量清单
///   （端点把 tools 计入前缀缓存，字节不一致则共享历史段全灭；能力隔离由 allowed_tools
///   运行时守卫执行）；false=独立后台运行体，按 allowed_tools 收敛 tools 可见性
/// delegate_run: Some=委托子 Agent——session 写穿共享主会话 + agent 身份 + 模型/温度覆盖；None=常规路径
pub async fn agent_chat_workflow_internal(
    user_query: String,
    attachments: Vec<Value>,
    max_iterations: Option<i32>,
    app_handle: Option<tauri::AppHandle>,
    silent: bool,
    session_id: Option<String>,
    allowed_tools: Option<Vec<String>>,
    system_prompt: Option<&str>,
    is_root: bool,
    delegate_depth: u8,
    respond_cancel: bool,
    time_only_context: bool,
    shared_history: Option<Vec<ChatMessage>>,
    canonical_tools: bool,
    delegate_run: Option<DelegateRun<'_>>,
) -> Result<Value, String> {
    let effective_prompt = system_prompt.unwrap_or(AGENT_SYSTEM_PROMPT);
    // ★ 加载可用专业 Agent 列表，注入系统提示词（仅当有委托能力时）
    let has_delegate = allowed_tools.as_ref().map_or(true, |t| t.iter().any(|x| x == "ext_tool_delegate_agent"));
    let agent_section = if has_delegate {
        let agents: Vec<Value> = database::all(
            "SELECT id, name, description, COALESCE(routing_hint,'') AS routing_hint, COALESCE(output_format,'natural') AS output_format FROM agent_profiles WHERE status='active'",
            &[],
        ).unwrap_or_default();
        if agents.is_empty() {
            String::new()
        } else {
            let lines: Vec<String> = agents.iter().filter_map(|a| {
                let id = a["id"].as_str()?;
                let name = a["name"].as_str()?;
                let desc = a["description"].as_str().unwrap_or("无描述");
                let hint = a["routing_hint"].as_str().unwrap_or("").trim();
                // 查询该 Agent 的工具列表，映射为中文能力标签
                let tools: Vec<String> = database::all(
                    "SELECT tool_name FROM agent_tool_assignments WHERE agent_id=?",
                    &[json!(id)]
                ).unwrap_or_default().iter()
                    .filter_map(|r| r["tool_name"].as_str().map(|s| tool_display_label(s).to_string()))
                    .collect();
                let tools_str = if tools.is_empty() { String::new() } else { format!("\n  工具：{}", tools.join("、")) };
                // ★ 交付型标记：其回答即终稿、原样交付用户（协议见 prompt.rs §6 交付型保真规则）
                // ★ 产出类别标记：文案由 OutputClass 单点维护
                let deliver_tag = crate::services::agent_workflow::output_class::OutputClass::from_format(
                    a["output_format"].as_str().unwrap_or("natural"),
                ).injection_tag();
                if hint.is_empty() {
                    Some(format!("- {}: {} (agent_id: \"{}\"){}{}", name, desc, id, tools_str, deliver_tag))
                } else {
                    Some(format!("- {}: {} (agent_id: \"{}\")\n  → {}{}{}", name, desc, id, hint, tools_str, deliver_tag))
                }
            }).collect();
            // ★ 路由决策日志：记录本次注入主 Agent 的可用清单（路由未命中时的第一排查点）
            let names: Vec<&str> = agents.iter().filter_map(|a| a["name"].as_str()).collect();
            log::info!("[Route] 注入【可用专业 Agent】{} 个: {}", names.len(), names.join("、"));
            format!("\n\n【可用专业 Agent】\n{}", lines.join("\n"))
        }
    } else {
        String::new()
    };
    // ★ 仅顶层工作流重置取消信号；子Agent保留信号向上传播；独立后台运行体不触碰该全局信号
    if is_root && respond_cancel { AGENT_CANCEL.store(false, Ordering::Release); }
    log::info!("workflow start, app_some={}", app_handle.is_some());
    let query = user_query.trim().to_string();
    let max_iters = max_iterations.unwrap_or(20).clamp(2, 40);
    if query.is_empty() { return Ok(json!({"success":true,"result_type":"answer","answer":"请输入问题。","iterations":0})); }
    crate::services::chat_api::invalidate_chat_config_cache();
    { let mut c = API_CACHE.write().unwrap_or_else(|e| e.into_inner()); c.3 = Instant::now() - std::time::Duration::from_secs(60); }
    let (_, api_key, _model) = get_api_endpoint();
    if api_key.is_empty() { return Ok(json!({"success":true,"result_type":"answer","answer":"请先在系统设置中配置 Chat API 密钥。"})); }

    // ★ 请求级 tools：canonical 模式下与白名单解耦——主/子 Agent 同一份全量清单，
    //   保证 tools 字段字节级一致（前缀缓存把 tools 计入前缀，不一致则共享历史段全灭）；
    //   能力隔离由 allowed_tools 运行时守卫执行（见下方工具集守卫）
    let tools = request_tools_definitions(canonical_tools, allowed_tools.as_deref());
    // ★ 委托上下文解构：先取出身份/模型覆盖，再把 session 引用接出（之后不再触碰 delegate_run）
    let is_delegate_run = delegate_run.is_some();
    let delegate_label: Option<(String, String)> = delegate_run.as_ref().map(|d| (d.agent_id.clone(), d.agent_name.clone()));
    let llm_overrides: LlmOverrides = delegate_run.as_ref().map(|d| d.llm.clone()).unwrap_or_default();
    let delegate_session: Option<&mut SessionMemory> = delegate_run.map(|d| d.session);
    // ★ Session 来源：委托运行写穿共享主会话（visited/tool_history/token 无需合并即回写，
    //   顺序 await 无竞态）；否则按 session_id 加载/新建
    let mut owned_session: Option<SessionMemory> = None;
    let session: &mut SessionMemory = match delegate_session {
        Some(s) => s,
        None => {
            owned_session = Some(if let Some(ref sid) = session_id {
                load_session_state(sid).unwrap_or_else(|| {
                    log::info!("[Session] 无缓存，从头创建");
                    SessionMemory::new()
                })
            } else {
                SessionMemory::new()
            });
            owned_session.as_mut().expect("owned_session just set")
        }
    };
    // ★ prev 快照仅自有 session 时建立（委托运行时是主会话的问题中快照，覆盖会破坏 save 差集）
    if !is_delegate_run {
        session.prev_visited_chunks = session.visited_chunks.clone();
        session.prev_visited_sources = session.visited_sources.clone();
    }

    // ★ 从 DB 加载历史消息
    // 读取 session 的压缩摘要（如果有）
    let compressed_summary: Option<String> = if let Some(ref sid) = session_id {
        let session_row = crate::database::get(
            "SELECT context_metadata FROM chat_sessions WHERE id=?1",
            &[json!(sid)],
        ).unwrap_or_default();
        if let Some(meta_str) = session_row.and_then(|r| r["context_metadata"].as_str().map(|s| s.to_string())) {
            if let Ok(meta) = serde_json::from_str::<Value>(&meta_str) {
                meta["summary"].as_str().map(|s| s.to_string())
            } else { None }
        } else { None }
    } else { None };

    let history: Vec<Value> = if let Some(ref sid) = session_id {
        let rows = crate::database::all(
            "SELECT role, content, metadata, created_at FROM chat_messages WHERE session_id=?1 AND status='active' ORDER BY id",
            &[json!(sid)],
        ).unwrap_or_default();

        log::info!("[AgentWorkflow] 从 DB 加载 {} 条 active 消息, 压缩摘要: {}",
            rows.len(), compressed_summary.is_some());

        rows.into_iter().filter_map(|r| {

            let role = r["role"].as_str().unwrap_or("");
            let db_content = r["content"].as_str().unwrap_or("");

            if db_content.is_empty() { return None; }

            // 历史消息纯内容，不含 metadata（tool_metadata 在 session 级别）
            let msg = json!({"role": role, "content": db_content});
            Some(msg)
        }).collect()
    } else {
        vec![]
    };

    // ★ 共享上下文模式（委托子Agent）：直接以主会话快照起步，跳过新会话构造
    let has_shared_history = shared_history.is_some();
    let mut messages: Vec<ChatMessage> = match shared_history {
        Some(h) => {
            log::info!("[AgentWorkflow] 共享历史模式: {} 条消息（跳过新会话构造）", h.len());
            h
        }
        None => vec![
            ChatMessage{role:MessageRole::System,content:Some(effective_prompt.to_string()),tool_calls:None,tool_call_id:None,name:None,reasoning_content:None},
        ],
    };

    // ★ 注入压缩摘要（如果有）
    if let Some(ref summary) = compressed_summary {
        let summary_text = format!("【历史摘要】\n{}", summary);
        messages.push(ChatMessage{role:MessageRole::Assistant,content:Some(summary_text),tool_calls:None,tool_call_id:None,name:None,reasoning_content:None});
        log::info!("[AgentWorkflow] 注入压缩摘要: {}字", summary.len());
    }
    // ★ 恢复历史消息
    for h in &history {
        let role = h["role"].as_str().unwrap_or("");
        match role {
            "assistant" => {
                let mut content = h["content"].as_str().map(|s| s.to_string());
                // ★ 生成文档文件名随回答注入（与前端卡片一致：AI 看到的 = 用户看到的）
                // ★ 委托直出的交付稿加"直出交付"标注，防主 LLM 把子 Agent 的稿子当自己的代笔续写
                if let Some(ref mut c) = content {
                    append_documents_note(c, h["metadata"].as_str());
                    append_delegated_note(c, h["metadata"].as_str());
                }
                if let Some(ref text) = content { session.record_observation(text); }
                messages.push(ChatMessage{role:MessageRole::Assistant,content,tool_calls:None,tool_call_id:None,name:None,reasoning_content:None});
            }
            _ => {
                let content = h["content"].as_str().map(|s| s.to_string());
                messages.push(ChatMessage{role:MessageRole::User,content,tool_calls:None,tool_call_id:None,name:None,reasoning_content:None});
            }
        }
    }

    // ★ 从 session.tool_metadata 注入最新轮次的工具调用和结果
    if let Some(ref sid) = session_id {
        if let Ok(Some(row)) = crate::database::get("SELECT tool_metadata FROM chat_sessions WHERE id=?1", &[json!(sid)]) {
            if let Some(meta_str) = row["tool_metadata"].as_str().filter(|s| !s.is_empty()) {
                if let Ok(meta) = serde_json::from_str::<Value>(meta_str) {
                    // 注入 tool_calls 为 assistant 消息
                    let tcs: Option<Vec<ToolCall>> = meta.get("tool_calls")
                        .and_then(|v| v.as_array())
                        .filter(|arr| !arr.is_empty())
                        .map(|arr| arr.iter().filter_map(|tc| {
                            let id = tc["id"].as_str()?;
                            let name = tc["function"]["name"].as_str()?;
                            let args_str = tc["function"]["arguments"].as_str().unwrap_or("");
                            let args = if args_str.is_empty() {
                                serde_json::to_string(&tc["function"]["arguments"]).unwrap_or_default()
                            } else { args_str.to_string() };
                            Some(ToolCall { id: id.to_string(), call_type: "function".into(), function: FunctionCall { name: name.to_string(), arguments: args } })
                        }).collect());
                    let has_tcs = tcs.as_ref().map(|v| !v.is_empty()).unwrap_or(false);
                    let tcs = tcs.filter(|v| !v.is_empty());
                    if has_tcs {
                        messages.push(ChatMessage{role:MessageRole::Assistant,content:None,tool_calls:tcs,tool_call_id:None,name:None,reasoning_content:None});
                    }
                    // 注入 tool_results 为 tool 消息
                    if has_tcs {
                        if let Some(results) = meta.get("tool_results").and_then(|v| v.as_array()) {
                            let mut grouped: std::collections::HashMap<String, (String, String)> = std::collections::HashMap::new();
                            for tr in results {
                                let tr_id = tr["tool_call_id"].as_str().unwrap_or("").to_string();
                                let tr_name = tr["name"].as_str().unwrap_or("").to_string();
                                let tr_content = tr["content"].as_str().unwrap_or("");
                                let entry = grouped.entry(tr_id).or_insert((String::new(), tr_name));
                                if !entry.0.is_empty() { entry.0.push_str("\n\n---\n\n"); }
                                entry.0.push_str(tr_content);
                            }
                            for (tr_id, (tr_content, tr_name)) in grouped {
                                messages.push(ChatMessage{role:MessageRole::Tool,content:Some(tr_content.clone()),tool_calls:None,tool_call_id:Some(tr_id),reasoning_content:None,name:Some(tr_name.clone())});
                                session.update_from_tool_result(&tr_name, &tr_content);
                            }
                        }
                    }
                }
            }
        }
    }
    // ★ 保存 user 消息到 DB（历史加载完成后，避免重复）
    // ★ 解析附件并构建增强消息
    let augmented_query = if !attachments.is_empty() {
        if let Some(ref h) = app_handle { let _ = h.emit("agent:progress", serde_json::json!({"msg":"正在解析附件..."})); }
        let mut blocks = Vec::new();
        let mut total_bytes = 0usize;
        const TOTAL_CAP: usize = 100000;
        for att in &attachments {
            let path = att["path"].as_str().unwrap_or("");
            let name = att["name"].as_str().unwrap_or("未命名文件");
            if path.is_empty() { continue; }
            let ext = std::path::Path::new(path).extension()
                .and_then(|e| e.to_str()).unwrap_or("").to_string();
            match crate::services::file_parser::parse_chat_attachment(path, &ext).await {
                Ok((_kind, text)) if !text.is_empty() => {
                    if text.len() > ATTACH_SUMMARIZE_THRESHOLD {
                        // ★ 大附件：切片提炼，不截断
                        if let Some(ref h) = app_handle {
                            let _ = h.emit("agent:progress", serde_json::json!({"msg": format!("检测到超大附件「{}」，正在切片提炼…", name)}));
                        }
                        let distilled = summarize_large_attachment(&text, &app_handle).await;
                        total_bytes += distilled.len();
                        blocks.push(format!("【附件：{}（已提炼）】\n{}", name, distilled));
                    } else if total_bytes + text.len() > TOTAL_CAP {
                        // 超出总预算：结构感知截断（保留表头/段落边界），而非整段跳过
                        let remaining = TOTAL_CAP.saturating_sub(total_bytes);
                        let cut = crate::services::file_parser::truncate_at_boundary(&text, remaining);
                        total_bytes += cut.len();
                        blocks.push(format!("【附件：{}】\n{}", name, cut));
                    } else {
                        total_bytes += text.len();
                        blocks.push(format!("【附件：{}】\n{}", name, text));
                    }
                }
                Ok(_) => { blocks.push(format!("【附件：{}】\n（文件解析为空）", name)); }
                Err(e) => { blocks.push(format!("【附件：{}】\n（解析失败: {}）", name, e)); }
            }
        }
        if blocks.is_empty() {
            query.clone()
        } else {
            format!("{}\n\n<<<ATTACHMENTS>>>\n{}", query, blocks.join("\n\n"))
        }
    } else {
        query.clone()
    };
    if let Some(ref sid) = session_id {
        let _ = crate::database::run(
            "INSERT INTO chat_messages(session_id, role, content) VALUES(?1, 'user', ?2)",
            &[json!(sid), json!(&augmented_query)],
        );
        // ★ 快呼会话首条消息回填标题（占位"快速唤起" → 首问 30 字，列表可辨识）
        if sid.starts_with("fab-") {
            let title = crate::commands::fab_title_from_content(&augmented_query);
            if !title.is_empty() {
                let _ = crate::database::run(
                    "UPDATE chat_sessions SET title=?1 WHERE id=?2 AND title=?3",
                    &[json!(title), json!(sid), json!("快速唤起")],
                );
            }
        }
    }

    // ★ 追加本轮新问题（共享历史模式：任务已由委托方以尾部指令注入，不重复 push）
    if !has_shared_history {
        messages.push(ChatMessage{role:MessageRole::User,content:Some(augmented_query),tool_calls:None,tool_call_id:None,name:None,reasoning_content:None});
    }

    // ★ 上下文压缩：复用会话上下文，LLM 返回 JSON 摘要
    // 单个会话上下文长度 256k，自动压缩阈值 80%（204,800 tokens）
    if let Some(ref sid) = session_id {
        let total_tokens: i64 = messages.iter().map(|m| {
            let content_len = m.content.as_ref().map(|s| s.len()).unwrap_or(0);
            let tc_len = m.tool_calls.as_ref().map(|tc| tc.len() * 100).unwrap_or(0);
            ((content_len + tc_len) as i64) / 3
        }).sum();
        if total_tokens > 204_800 {
            log::info!("[ContextCompressor] 当前上下文 token {} 超过阈值 204800 (256k×80%)，开始压缩...", total_tokens);
            match crate::services::context_compressor::compress_with_context(&mut messages, &app_handle).await {
                Ok(result) => {
                    log::info!("[ContextCompressor] 压缩成功: summary={}字", result.summary.len());
                    // 计算当前会话的总消息数量（保留调用兼容，参数已不使用）
                    let msg_count = crate::database::get(
                        "SELECT COUNT(*) as cnt FROM chat_messages WHERE session_id=?1",
                        &[json!(sid)],
                    ).ok().flatten().and_then(|r| r["cnt"].as_i64()).unwrap_or(0) as usize;
                    if let Err(e) = crate::services::context_compressor::save_compressed_to_db(sid, &result, msg_count) {
                        log::warn!("[ContextCompressor] 保存压缩结果失败: {}", e);
                    }
                    // 压缩后重新加载历史（从 DB 读取压缩后的状态）
                    messages.clear();
                    messages.push(ChatMessage{role:MessageRole::System,content:Some(effective_prompt.to_string()),tool_calls:None,tool_call_id:None,name:None,reasoning_content:None});

                    // 读取新的压缩摘要
                    let new_session_row = crate::database::get(
                        "SELECT context_metadata FROM chat_sessions WHERE id=?1",
                        &[json!(sid)],
                    ).unwrap_or_default();
                    if let Some(meta_str) = new_session_row.as_ref().and_then(|r| r["context_metadata"].as_str().map(|s| s.to_string())) {
                        if let Ok(meta) = serde_json::from_str::<Value>(&meta_str) {
                            if let Some(summary) = meta["summary"].as_str() {
                                let summary_text = format!("【历史摘要】\n{}", summary);
                                messages.push(ChatMessage{role:MessageRole::Assistant,content:Some(summary_text),tool_calls:None,tool_call_id:None,name:None,reasoning_content:None});
                                log::info!("[ContextCompressor] 注入压缩摘要: {}字", summary.len());
                            }
                        }
                    }

                    // 从 DB 重新加载近期消息（只加载 active 状态）
                    let recent_history: Vec<Value> = {
                        let rows = crate::database::all(
                            "SELECT role, content, metadata, created_at FROM chat_messages WHERE session_id=?1 AND status='active' ORDER BY id",
                            &[json!(sid)],
                        ).unwrap_or_default();
                        log::info!("[ContextCompressor] 从 DB 加载 {} 条 active 消息", rows.len());

                        rows.into_iter().filter_map(|r| {
                            let role = r["role"].as_str().unwrap_or("");
                            let db_content = r["content"].as_str().unwrap_or("");

                            if db_content.is_empty() { return None; }

                            let msg = json!({"role": role, "content": db_content});
                            Some(msg)
                        }).collect()
                    };
                    // ★ 恢复压缩后的近期消息
                    for h in &recent_history {
                        let role = h["role"].as_str().unwrap_or("");
                        match role {
                            "assistant" => {
                                let mut content = h["content"].as_str().map(|s| s.to_string());
                                // ★ 生成文档文件名随回答注入（与前端卡片一致，压缩重建后同样可见）
                                if let Some(ref mut c) = content { append_documents_note(c, h["metadata"].as_str()); }
                                messages.push(ChatMessage{role:MessageRole::Assistant,content,tool_calls:None,tool_call_id:None,name:None,reasoning_content:None});
                            }
                            _ => {
                                let content = h["content"].as_str().map(|s| s.to_string());
                                messages.push(ChatMessage{role:MessageRole::User,content,tool_calls:None,tool_call_id:None,name:None,reasoning_content:None});
                            }
                        }
                    }
                    // ★ 压缩后从 session.tool_metadata 注入工具调用
                    if let Ok(Some(tm_row)) = crate::database::get("SELECT tool_metadata FROM chat_sessions WHERE id=?1", &[json!(sid)]) {
                        if let Some(meta_str) = tm_row["tool_metadata"].as_str().filter(|s| !s.is_empty()) {
                            if let Ok(meta) = serde_json::from_str::<Value>(meta_str) {
                                let tcs: Option<Vec<ToolCall>> = meta.get("tool_calls")
                                    .and_then(|v| v.as_array())
                                    .filter(|arr| !arr.is_empty())
                                    .map(|arr| arr.iter().filter_map(|tc| {
                                        let id = tc["id"].as_str()?;
                                        let name = tc["function"]["name"].as_str()?;
                                        let args_str = tc["function"]["arguments"].as_str().unwrap_or("");
                                        let args = if args_str.is_empty() {
                                            serde_json::to_string(&tc["function"]["arguments"]).unwrap_or_default()
                                        } else { args_str.to_string() };
                                        Some(ToolCall { id: id.to_string(), call_type: "function".into(), function: FunctionCall { name: name.to_string(), arguments: args } })
                                    }).collect());
                                let has_tcs = tcs.as_ref().map(|v| !v.is_empty()).unwrap_or(false);
                                let tcs = tcs.filter(|v| !v.is_empty());
                                if has_tcs {
                                    messages.push(ChatMessage{role:MessageRole::Assistant,content:None,tool_calls:tcs,tool_call_id:None,name:None,reasoning_content:None});
                                }
                                if has_tcs {
                                    if let Some(results) = meta.get("tool_results").and_then(|v| v.as_array()) {
                                        let mut grouped: std::collections::HashMap<String, (String, String)> = std::collections::HashMap::new();
                                        for tr in results {
                                            let tr_id = tr["tool_call_id"].as_str().unwrap_or("").to_string();
                                            let tr_name = tr["name"].as_str().unwrap_or("").to_string();
                                            let tr_content = tr["content"].as_str().unwrap_or("");
                                            let entry = grouped.entry(tr_id).or_insert((String::new(), tr_name));
                                            if !entry.0.is_empty() { entry.0.push_str("\n\n---\n\n"); }
                                            entry.0.push_str(tr_content);
                                        }
                                        for (tr_id, (tr_content, tr_name)) in grouped {
                                            messages.push(ChatMessage{role:MessageRole::Tool,content:Some(tr_content),tool_calls:None,tool_call_id:Some(tr_id),reasoning_content:None,name:Some(tr_name)});
                                        }
                                    }
                                }
                            }
                        }
                    }
                    // 追加当前查询
                    messages.push(ChatMessage{role:MessageRole::User,content:Some(query.clone()),tool_calls:None,tool_call_id:None,name:None,reasoning_content:None});
                }
                Err(e) => {
                    log::warn!("[ContextCompressor] 压缩失败: {}", e);
                    return Err(format!("上下文压缩失败: {}", e));
                }
            }
        }
    }

    // ★ 从活跃对象和本轮 query 设置 Focus
    let top_entities: Vec<String> = {
        let mut sorted: Vec<&ActiveObject> = session.active_objects.iter().collect();
        sorted.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        sorted.iter().take(3).map(|o| o.name.clone()).collect()
    };
    session.set_focus_from_user_query(&query, &top_entities);

    let mut tool_log: Vec<Value> = Vec::new();
    let (mut iters, mut pt, mut _ct) = (0i32, 0i64, 0i64);
    let mut _cached_total: i64 = 0;
    let mut _prev_pt: i64 = 0;
    let mut budget = 300i32;  // 增加到300分
    let mut search_cost = 8i32;  // 降低初始消耗到8分
    let mut traverse_source_counter: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    
    // System Prompt 注入可用 Agent 列表；【关于我】仅智能问答主Agent注入（分析师/子Agent不注入）
    let whoami_section = if is_root && system_prompt.is_none() {
        let block = crate::services::whoami::Whoami::load().prompt_block(true);
        if block.is_empty() { String::new() } else { format!("\n\n{}", block) }
    } else {
        String::new()
    };
    // 联网能力提示段：仅主Agent且已配置搜索服务时注入（未配置=模型无感知）
    let web_section = if is_root && system_prompt.is_none() && crate::services::web_search::is_configured() {
        crate::services::agent_workflow::prompt::WEB_PROMPT_SECTION.to_string()
    } else {
        String::new()
    };
    // ★ system 重写仅限新会话模式：共享历史模式的 messages[0] 是主 Agent 完整 system
    //   （含 agent/whoami/web 段），重写会改变前缀字节 → 分叉点提前 → 历史段缓存全灭
    if !has_shared_history {
        messages[0].content = Some(format!("{}{}{}{}", effective_prompt, agent_section, whoami_section, web_section));
    }
    let mut tool_text_retry_count = 0u32; // 工具调用文本重试计数器
    let mut retry_count = 0u32; // ToolCall 清理后重试计数器

    loop {
        // ★ 检查停止信号（仅顶层清除标志，子Agent保留信号向上传播；独立后台运行体跳过）
        if respond_cancel && AGENT_CANCEL.load(Ordering::Acquire) {
            if is_root { AGENT_CANCEL.store(false, Ordering::Release); }
            log::info!("收到停止信号，中断执行");
            let ws = session.generate_scratchpad();
            return Ok(json!({"success":true,"result_type":"answer","answer":"已停止执行。","workspace":ws,"iterations":iters,"cancelled":true,"promptTokens":session.prompt_tokens,"completionTokens":session.completion_tokens,"cachedTokens":session.cached_tokens,"tool_log":tool_log}));
        }
        
        log::info!("预算: {}/300 cost={}", budget, search_cost);
        // ★ 状态事件携带子 Agent 身份（delegate_label=Some 时），前端状态条据此显示"谁在干活"
        let thinking_status = match &delegate_label {
            Some((aid, aname)) => json!({"stage":"thinking","agent_id":aid,"agent_name":aname,"msg":format!("{} · 思考中...", aname)}),
            None => json!({"stage":"thinking","msg":"思考中..."}),
        };
        if !silent { if let Some(ref h) = app_handle { let _ = h.emit("agent:status", thinking_status); let _ = h.emit("agent:progress", json!({"msg":"思考中..."})); } }
        // ★ 每轮注意力衰减（Residual 方式：new_score = old_score * 0.95 + gain）
        // gain 在 update_from_tool_result / UpdateObject effect 中通过 EMA 完成
        // 这里只做衰减，不做加分
        let prev_count = session.active_objects.len();
        for o in &mut session.active_objects {
            o.score *= 0.95;
        }
        // 淘汰：score < 0.1 且 最近 10 轮未访问
        session.active_objects.retain(|o| {
            o.score >= 0.1 || o.access_order >= session.access_counter.saturating_sub(10)
        });
        let removed = prev_count - session.active_objects.len();
        if removed > 0 { log::info!("[注意力] 衰减后移除 {} 个未命中对象，剩余 {}", removed, session.active_objects.len()); }
        iters += 1;


        // ★ 首轮/无工具轮：在 LLM 调用前注入动态上下文（后续轮次由循环末尾注入）
        {
            let scratchpad = session.generate_scratchpad();
            let status = if budget >= 100 { "预算充足" } else if budget >= 50 { "预算偏低" } else { "预算紧张，请优先总结已有信息" };
            // ★ 计算当前上下文 token 数
            let context_tokens: i64 = messages.iter().map(|m| {
                let text_len = m.content.as_ref().map_or(0, |s| s.len() as i64);
                let tc_len = m.tool_calls.as_ref().map_or(0, |tcs| tcs.iter()
                    .map(|tc| tc.function.arguments.len() as i64)
                    .sum::<i64>());
                (text_len + tc_len) / 3
            }).sum();
            let budget_info = format!(
                "当前会话工作记忆\n{}\n\n当前时间: {}\n\n搜索预算系统（单次对话）:\n- 总预算: 300 分\n- 计算方式: 每轮扣减，首次8分，逐轮×1.2递增（如8→9→11→13→...）\n- 当前轮次: 第{}轮，本轮将消耗: {}分\n- 剩余预算: {}/300 — {}\n- 当前上下文: {} tokens\n- 建议: 信息足够时主动回答，不要等到预算耗尽",
                scratchpad, cn_now_str(), iters, search_cost, budget, status, context_tokens
            );
            // 移除上一轮的动态上下文消息
            if let Some(pos) = messages.iter().rposition(|m| m.content.as_deref().map_or(false, |c| c.starts_with("[DYNAMIC_CONTEXT]"))) {
                messages.remove(pos);
            }
            messages.push(ChatMessage{role:MessageRole::User, content: Some(if time_only_context { format!("[DYNAMIC_CONTEXT]\n当前时间: {}", cn_now_str()) } else { format!("[DYNAMIC_CONTEXT]\n{}", budget_info) }), tool_calls: None, tool_call_id: None, name: None, reasoning_content: None});
        }

        // ★ 调试：保存 context 到文件（供 debug_server.py 读取）—— 仅 debug 构建
        #[cfg(debug_assertions)]
        {
            let context_json: Vec<Value> = messages.iter().filter_map(|m| {
                serde_json::to_value(m).ok()
            }).collect();
            let _ = crate::commands::save_agent_context(context_json);
        }

        // ★ 单请求流式：content 逐 token 输出到前端，同时累积 tool_calls delta
        log::info!("[Agent] 调用 call_llm_streaming，messages 长度: {}", messages.len());
        let (ac_owned, reasoning, tcs_streamed, usage) = match call_llm_streaming_inner(&messages, Some(&tools), &app_handle, silent, respond_cancel, &llm_overrides).await {
            Ok(r) => {
                log::info!("[Agent] call_llm_streaming 成功: content_len={}, tool_calls={}", r.0.len(), r.2.len());
                r
            }
            Err(e) => {
                log::error!("[Agent] call_llm_streaming 失败: {}", e);
                // ★ 仅根工作流上报 agent:error：子 Agent（delegate_depth≥1）失败由委托信封回传，
                // 若直接发 error 会让前端提前拆掉 done/token 监听，导致父流程最终回复丢失（仅刷新可见）
                if !silent && delegate_depth == 0 { if let Some(ref h) = app_handle {
                    let _ = h.emit("agent:error", json!({"error": e}));
                } }
                return Ok(json!({"success":false,"answer":format!("LLM 调用失败: {}", e),"iterations":iters}));
            }
        };
        if let Some(u) = usage {
            _prev_pt = pt;
            let round_pt = u["prompt_tokens"].as_i64().unwrap_or(0);
            let round_ct = u["completion_tokens"].as_i64().unwrap_or(0);
            let cached = u["prompt_tokens_details"]["cached_tokens"].as_i64().unwrap_or(0);
            pt += round_pt;
            _ct += round_ct;
            _cached_total += cached;
            session.prompt_tokens += round_pt;
            session.completion_tokens += round_ct;
            session.cached_tokens += cached;
            let hit_rate = if round_pt > 0 { cached as f64 / round_pt as f64 * 100.0 } else { 0.0 };
            // ★ 当前上下文 token 数
            let context_tokens: i64 = messages.iter().map(|m| {
                let text_len = m.content.as_ref().map_or(0, |s| s.len() as i64);
                let tc_len = m.tool_calls.as_ref().map_or(0, |tcs| tcs.iter()
                    .map(|tc| tc.function.arguments.len() as i64)
                    .sum::<i64>());
                (text_len + tc_len) / 3
            }).sum();
            log::info!("[Token] 当前上下文={} | 本轮 prompt={} cached={:.0}% completion={} | 累计 prompt={} | session p={} c={}",
                context_tokens, round_pt, hit_rate, round_ct, pt, session.prompt_tokens, session.completion_tokens);
        }
        let ac = ac_owned.as_str();
        let mut tcs: Option<Vec<Value>> = if tcs_streamed.is_empty() { None } else { Some(tcs_streamed) };
        // ★ 始终计算清理后的 content（剥离工具调用标记），后续所有展示/保存/判定都使用干净文本
        let cleaned_ac = clean_tool_markers(ac, true);
        log::info!("[Agent] 解析完成: ac_len={}, cleaned_len={}, tcs={}, has_xml_tool={}", 
            ac.len(), cleaned_ac.len(),
            tcs.as_ref().map(|v| v.len()).unwrap_or(0),
            ac.contains("tool_calls") || ac.contains("<tool_call>")
        );

        // ★ 兜底：如果 tool_calls 为空但 content 包含 XML 格式工具调用，尝试解析
        if tcs.is_none() || tcs.as_ref().map_or(false, |v| v.is_empty()) {
            if let Some(parsed) = parse_xml_tool_calls(ac) {
                log::info!("[LLM检索] 从 content 解析到 XML 工具调用: {:?}", parsed);
                tcs = Some(parsed);
            } else if let Some(parsed) = parse_html_tool_calls(ac) {
                log::info!("[LLM检索] 从 content 解析到 HTML 工具调用: {:?}", parsed);
                tcs = Some(parsed);
            }
        }

        let effective_ac = cleaned_ac.as_str();
        // ★ 工具调用文本检测基于原始 ac（effective_ac 已清理，无法再检测到残留标记）
        let has_tool_pattern = (ac.trim().starts_with('{') && ac.contains("tool_calls"))
            || ac.contains("<tool_call")
            || ac.starts_with("<invoke")
            || (ac.contains("ext_tool_") && ac.contains("```"))
            ;
        log::info!("[Agent] LLM 响应: len={}, tool_calls={}, has_tool_pattern={}", 
            effective_ac.len(), 
            tcs.as_ref().map(|v| v.len()).unwrap_or(0),
            has_tool_pattern
        );

        match tcs {
            None => {
                if has_tool_pattern && tool_text_retry_count < 2 {
                    tool_text_retry_count += 1;
                    log::warn!("[Agent] 检测到 content 中包含工具调用文本 (第{}次)，追加提示重试", tool_text_retry_count);
                    // 不存坏内容到历史，只标记上一轮失败
                    messages.push(ChatMessage{role:MessageRole::Assistant, content: None, tool_calls: None, tool_call_id: None, name: None, reasoning_content: None});
                    // 追加系统提示要求使用标准格式
                    messages.push(ChatMessage{role:MessageRole::User, content: Some("你的回复包含了工具调用文本，但没有通过 tool_calls 字段返回，工具无法被执行。请必须使用标准的 function calling 格式调用工具，禁止在 content 中输出任何工具调用格式。".into()), tool_calls: None, tool_call_id: None, name: None, reasoning_content: None});
                    continue; // 重新进入循环
                }

                // ★ 单请求流式：答案已在流式中逐 token 输出，无需再次生成
                let mut answer = effective_ac.to_string();
                log::info!("[Agent] 处理答案前: len={}, preview={}", answer.len(), answer.chars().take(100).collect::<String>());
                if answer.trim().is_empty() {
                    log::info!("[Agent] 流式内容为空，回退补生成");
                    answer = call_llm_without_tools(&messages, &app_handle, silent, respond_cancel, &llm_overrides).await.unwrap_or_else(|e| { log::warn!("[Agent] 流式补生成失败，降级为纯文本回答: {e}"); format!("未能生成有效回答: {e}") });
                }
                answer = answer.trim().to_string();

                // ★ 如果 cleaned 后为空（内容全部是工具调用标记），跳回主循环重新执行（允许工具调用）
                if answer.is_empty() && !query.is_empty() {
                    if retry_count < 2 {
                        retry_count += 1;
                        log::warn!("[Agent] 清理后答案为空，跳回主循环重新执行... (retry={})", retry_count);
                        // 清理最后一条 user 消息（重试提示），避免上下文污染
                        messages.pop();
                        continue;
                    } else {
                        log::warn!("[Agent] 重试次数耗尽，返回空答案");
                    }
                }

                log::info!("[Agent] 最终答案: len={}, preview={}", answer.len(), answer.chars().take(50).collect::<String>());
                if !silent { if let Some(ref h) = app_handle { let _ = h.emit("agent:status", json!({"stage":"answer"})); let _ = h.emit("agent:final-answer", json!({"answer": &answer})); } }
                if !answer.is_empty() { session.record_observation(&answer); }
                if let Some(ref sid) = session_id { save_session_state(sid, &session); }
                let ws = session.generate_scratchpad(); return Ok(json!({"success":true,"result_type":"answer","answer":answer,"workspace":ws,"iterations":iters,"promptTokens":session.prompt_tokens,"completionTokens":session.completion_tokens,"cachedTokens":session.cached_tokens,"tool_log":tool_log}));
            }
            Some(ref calls) if !calls.is_empty() => {
                // ★ 工具轮：不再向前端推送 interim 内容，检索期间保持预设文案
                // ★ 预算检查：不足时强制回答（在工具调用之前检查）
                if budget < search_cost {
                    if !silent { if let Some(ref h) = app_handle {
                        let kw = calls.first().and_then(|tc| tc["function"]["arguments"].as_str().map(|s| s.to_string())).unwrap_or_default();
                        let _ = h.emit("agent:budget-low", json!({"remaining":budget,"cost":search_cost,"next_query":kw}));
                    } }
                    messages.push(ChatMessage{role:MessageRole::User,content:Some("预算不足，请直接总结已获取的信息。".into()),tool_calls:None,tool_call_id:None,name:None,reasoning_content:None});
                    let ans = call_llm_without_tools(&messages, &app_handle, silent, respond_cancel, &llm_overrides).await.unwrap_or_else(|e| { log::warn!("[Agent] 预算不足总结失败: {e}"); format!("总结失败: {e}") });
                    if let Some(ref sid) = session_id { save_session_state(sid, &session); }
                    let ws = session.generate_scratchpad();
                    if !silent { if let Some(ref h) = app_handle { let _ = h.emit("agent:status", json!({"stage":"answer"})); let _ = h.emit("agent:final-answer", json!({"answer": &ans})); } }
                    return Ok(json!({"success":true,"result_type":"answer","answer":ans,"workspace":ws,"iterations":iters,"budget_exhausted":true,"promptTokens":session.prompt_tokens,"completionTokens":session.completion_tokens,"cachedTokens":session.cached_tokens,"tool_log":tool_log}));
                }
                if iters >= max_iters {
                    messages.push(ChatMessage{role:MessageRole::User,content:Some("已达到工具调用上限，请直接总结。".into()),tool_calls:None,tool_call_id:None,name:None,reasoning_content:None});
                    let ans = call_llm_without_tools(&messages, &app_handle, silent, respond_cancel, &llm_overrides).await.unwrap_or_else(|e| { log::warn!("[Agent] 迭代上限总结失败: {e}"); format!("总结失败: {e}") });
                    let ws = session.generate_scratchpad();
                    if !silent { if let Some(ref h) = app_handle { let _ = h.emit("agent:status", json!({"stage":"answer"})); let _ = h.emit("agent:final-answer", json!({"answer": ans})); } }
                    return Ok(json!({"success":true,"answer":ans,"workspace":ws,"iterations":iters,"circuit_breaker":true,"promptTokens":session.prompt_tokens,"completionTokens":session.completion_tokens,"cachedTokens":session.cached_tokens,"tool_log":tool_log}));
                }

    let mut current_round = json!({"round":iters,"thought":ac,"tools":[]});
                let mut atc = Vec::new();
                for tc in calls {
                    let id = tc["id"].as_str().unwrap_or("").to_string();
                    let name = tc["function"]["name"].as_str().unwrap_or("");
                    let args = tc["function"]["arguments"].as_str().unwrap_or("").to_string();
                    atc.push(ToolCall{id:id.clone(),call_type:"function".into(),function:FunctionCall{name:name.into(),arguments:args.clone()}});
                }
                messages.push(ChatMessage{role:MessageRole::Assistant,content:None,tool_calls:Some(atc.clone()),tool_call_id:None,name:None,reasoning_content:reasoning.clone()});
                // ★ 向前端广播工具调用状态
                if !silent { if let Some(ref h) = app_handle {
                    for tc in atc.iter() {
                        let args: Value = serde_json::from_str(&tc.function.arguments).unwrap_or(json!({}));
                        let target = match tc.function.name.as_str() {
                            "ext_tool_search" => args.get("keywords").and_then(|k| k.as_array()).map(|a| a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join("、")),
                            "ext_tool_object" => {
                                if let Some(ids) = args.get("id").and_then(|v| v.as_array()).filter(|a| !a.is_empty()) {
                                    Some(ids.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join(", "))
                                } else if let Some(ff) = args.get("field_filters").and_then(|v| v.as_array()).filter(|a| !a.is_empty()) {
                                    let entity = args.get("type").and_then(|v| v.as_str()).unwrap_or("?");
                                    Some(format!("{} ({}条过滤)", entity, ff.len()))
                                } else {
                                    args.get("query").and_then(|q| q.as_str()).map(String::from)
                                }
                            },
                            "ext_tool_chunk_navigate" => args.get("chunk_id").and_then(|c| c.as_str()).map(|c| format!("chunk {}", crate::preview(c, 8))),
                            _ => None,
                        };
                        let (said, sanme) = match &delegate_label { Some((aid, aname)) => (aid.as_str(), aname.as_str()), None => ("", "") };
                        let _ = h.emit("agent:status", json!({"stage":"tool_start","tool":tc.function.name,"label":tool_display_label(&tc.function.name),"agent_id":said,"agent_name":sanme,"target":target.unwrap_or_default()}));
                        let _ = h.emit("agent:progress", json!({"msg": format!("正在{}...", if tc.function.name.starts_with("ext_tool_search") { "检索知识库" } else if tc.function.name.starts_with("ext_tool_object") { "查询数据" } else { "处理中" })}));
                    }
                } }
                // ═══ 并行工具调度：async 工具并行执行，sync 工具顺序执行 ═══
                let mut round_tools = Vec::new();
                let mut dedup_cache: std::collections::HashMap<(String,String),String> = std::collections::HashMap::new();
                let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
                let mut extract_chunk_ids = |tools: &[Value]| { for t in tools { let s = t["result"].as_str().unwrap_or(""); if s.contains("ChunkID") { for line in s.lines() { let tl = line.trim(); if tl.starts_with("ChunkID") { if let Some(id) = tl.split('：').nth(1).map(|x| x.trim().to_string()) { visited.insert(id); } } } } } };
                for r in &tool_log { if let Some(tools) = r["tools"].as_array() { extract_chunk_ids(tools); } }
                extract_chunk_ids(&round_tools);
                session.visited_chunks.extend(visited.iter().cloned());

                // 快照并行工具需要的只读状态
                let scope_snapshot = compute_scope_from_session(&session);
                let visited_sources_snapshot = session.visited_sources.clone();
                let visited_chunks_snapshot = session.visited_chunks.clone();

                // 预去重 + 分组：async 工具 vs sync 工具
                let mut async_tasks: Vec<(usize, String, String)> = Vec::new(); // (index, name, args)
                let mut sync_tasks: Vec<(usize, String, String)> = Vec::new();
                let mut cached_results_map: std::collections::HashMap<usize, String> = std::collections::HashMap::new();

                for (i, tc) in atc.iter().enumerate() {
                    let key = (tc.function.name.clone(), tc.function.arguments.clone());
                    if let Some(cached) = dedup_cache.get(&key) {
                        if !silent { if let Some(ref h) = app_handle {
                            let _ = h.emit("agent:tool-result", json!({"round": iters, "tool": tc.function.name, "cached": true, "result_len": cached.len()}));
                        } }
                        cached_results_map.insert(i, cached.clone());
                    } else {
                        // 工具集守卫：不在允许范围内的工具不执行（如周期分析只允许只读+通知）
                        if let Some(ref allowed) = allowed_tools {
                            if !allowed.iter().any(|a| a == &tc.function.name) {
                                let env = serde_json::json!({
                                    "result_type": "tool_forbidden",
                                    "data": {"name": tc.function.name},
                                    "summary": format!("工具「{}」在当前任务中未授权，已跳过执行。", tc.function.name)
                                });
                                if !silent { if let Some(ref h) = app_handle {
                                    let _ = h.emit("agent:tool-result", json!({"round": iters, "tool": tc.function.name, "forbidden": true}));
                                } }
                                cached_results_map.insert(i, env.to_string());
                                continue;
                            }
                        }
                         match tc.function.name.as_str() {
                            "ext_tool_search" | "ext_tool_search_knowledge_base" | "ext_tool_batch_search" => {
                                async_tasks.push((i, tc.function.name.clone(), tc.function.arguments.clone()));
                            }
                            "ext_tool_web_search" | "ext_tool_web_fetch" => {
                                async_tasks.push((i, tc.function.name.clone(), tc.function.arguments.clone()));
                            }
                            _ => {
                                sync_tasks.push((i, tc.function.name.clone(), tc.function.arguments.clone()));
                            }
                        }
                    }
                }

                // 并行执行 async 工具
                let results_mutex: Mutex<Vec<(usize, String, Vec<SessionEffect>)>> = Mutex::new(Vec::new());
                if !async_tasks.is_empty() {
                    let mut handles: Vec<(usize, tokio::task::JoinHandle<(usize, String, Vec<SessionEffect>)>)> = Vec::new();
                    for (orig_idx, name, args_str) in &async_tasks {
                        let name = name.clone();
                        let args_str = args_str.clone();
                        let orig_idx = *orig_idx;
                        let scope = scope_snapshot.clone();
                        let vs = visited_sources_snapshot.clone();
                        let vc = visited_chunks_snapshot.clone();
                        let shown_mem = session.shown_memory_ids.clone();
                        handles.push((orig_idx, tokio::spawn(async move {
                            let a: Value = serde_json::from_str(&args_str).unwrap_or(json!({}));
                            let (res, effects) = match name.as_str() {
                                "ext_tool_search" | "ext_tool_search_knowledge_base" => {
                                    let kw: Vec<String> = a["keywords"].as_array()
                                        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                                        .or_else(|| a["keywords"].as_str().map(|s| s.split_whitespace().map(String::from).collect()))
                                        .unwrap_or_default();
                                    let time_range = a["start_time"].as_str()
                                        .and_then(|s| a["end_time"].as_str().map(|e| (resolve_time(s), resolve_time(e))));
                                    let (mut res, new_sources) = execute_search_knowledge(
                                        &kw, scope.clone(), &vs, &vc, time_range.clone(), &shown_mem
                                    ).await;
                                    if new_sources.len() < 2 && scope.is_some() {
                                        let fb = execute_search_knowledge(&kw, None, &vs, &vc, time_range, &shown_mem).await;
                                        res = fb.0;
                                        let mut effects = vec![SessionEffect::RecordSearch(kw)];
                                        for s in &fb.1 {
                                            effects.push(SessionEffect::MarkSourceVisited(s.clone()));
                                            if let Some(dash) = s.find('-') {
                                                let (ot, oi) = s.split_at(dash);
                                                let oi = &oi[1..];
                                                let name = database::get("SELECT title FROM source_data WHERE source_type=?1 AND source_id=?2", &[json!(ot.to_uppercase()), json!(oi)]).ok().flatten().and_then(|r| r["title"].as_str().map(String::from)).unwrap_or_default();
                                                if !ot.is_empty() && !oi.is_empty() && !name.is_empty() {
                                                    effects.push(SessionEffect::UpdateObject { obj_type: ot.to_lowercase(), obj_id: oi.to_string(), name, score_delta: 0.0 });
                                                }
                                            }
                                        }
                                        (res, effects)
                                    } else {
                                        let mut effects = vec![SessionEffect::RecordSearch(kw)];
                                        for s in &new_sources {
                                            effects.push(SessionEffect::MarkSourceVisited(s.clone()));
                                            if let Some(dash) = s.find('-') {
                                                let (ot, oi) = s.split_at(dash);
                                                let oi = &oi[1..];
                                                let name = database::get("SELECT title FROM source_data WHERE source_type=?1 AND source_id=?2", &[json!(ot.to_uppercase()), json!(oi)]).ok().flatten().and_then(|r| r["title"].as_str().map(String::from)).unwrap_or_default();
                                                if !ot.is_empty() && !oi.is_empty() && !name.is_empty() {
                                                    effects.push(SessionEffect::UpdateObject { obj_type: ot.to_lowercase(), obj_id: oi.to_string(), name, score_delta: 0.0 });
                                                }
                                            }
                                        }
                                        (res, effects)
                                    }
                                }
                                "ext_tool_batch_search" => {
                                    let queries_raw = a["queries"].as_array();
                                    let mut sub_queries: Vec<Vec<String>> = Vec::new();
                                    if let Some(arr) = queries_raw {
                                        for q in arr.iter().take(10) {
                                            if let Some(kw) = q["keywords"].as_array() {
                                                let k: Vec<String> = kw.iter().filter_map(|v| v.as_str().map(String::from)).collect();
                                                if !k.is_empty() { sub_queries.push(k); }
                                            }
                                        }
                                    }
                                    let res = execute_batch_search(&sub_queries, scope, &vs).await;
                                    (res, vec![])
                                }
                                "ext_tool_web_search" => {
                                    let q = a["query"].as_str().unwrap_or("").to_string();
                                    let count = a["count"].as_i64().unwrap_or(5) as u32;
                                    (crate::services::web_search::web_search_text(&q, count).await, vec![])
                                }
                                "ext_tool_web_fetch" => {
                                    let u = a["url"].as_str().unwrap_or("").to_string();
                                    (crate::services::web_search::web_fetch_text(&u).await, vec![])
                                }
                                _ => unreachable!(),
                            };
                            (orig_idx, res, effects)
                        })));
                    }
                    for (idx, handle) in handles {
                        match handle.await {
                            Ok(result) => {
                                results_mutex.lock().unwrap_or_else(|e| e.into_inner()).push(result);
                            }
                            Err(e) => {
                                log::error!("[Agent] 异步工具任务 JoinHandle 失败: {}", e);
                                // ★ 必须补占位结果：assistant 消息带此 tool_call，缺对应 tool 回复会让
                                //   后续请求消息序列残缺，严格校验的 OpenAI 兼容端直接 400
                                let placeholder = ToolResultEnvelope {
                                    result_type: "tool_error".to_string(),
                                    data: json!({"success": false}),
                                    summary: Some(format!("工具执行异常（任务崩溃），请换方式完成该步骤: {}", e)),
                                };
                                results_mutex.lock().unwrap_or_else(|e| e.into_inner()).push((idx, placeholder.to_json(), vec![]));
                            }
                        }
                    }
                }
                let async_results: Vec<(usize, String, Vec<SessionEffect>)> = {
                    let guard = results_mutex.lock().unwrap_or_else(|e| e.into_inner());
                    guard.clone()
                };

                // 顺序执行 sync 工具
                let mut sync_results: Vec<(usize, String, Vec<SessionEffect>)> = Vec::new();
                // ★ 同轮已完成结果（缓存命中 + async 已返回 + 已执行完的 sync）：
                //   委托快照据此构建，后一个委托能看到前一个委托/检索的结果（一轮内依赖链）
                let mut completed_round: Vec<(usize, String)> = cached_results_map.iter()
                    .map(|(i, r)| (*i, r.clone())).collect();
                completed_round.extend(async_results.iter().map(|(i, r, _)| (*i, r.clone())));
                for (idx, name, args_str) in &sync_tasks {
                    let a: Value = serde_json::from_str(args_str).unwrap_or(json!({}));
                    let (result, effects) = match name.as_str() {
                        "ext_tool_object" => {
                            let mode = a["mode"].as_str().unwrap_or("");
                            let res = match mode {
                                "detail" => {
                                    // ★ 按ID查看详情：业务对象走数据库结构化查询，文档对象走chunk回忆
                                    let ids: Vec<String> = match a.get("id") {
                                        Some(v) if v.is_array() => v.as_array().unwrap().iter().filter_map(|x| x.as_str().map(String::from)).collect(),
                                        Some(v) if v.is_string() => vec![v.as_str().unwrap().to_string()],
                                        _ => vec![],
                                    };
                                    if ids.is_empty() {
                                        "错误：detail模式需要传入id参数".to_string()
                                    } else {
                                        let business_types = ["PROJECT", "PRODUCT", "CHANNEL", "VENDOR", "CUSTOMER", "CONTACT", "TODO", "SOLUTION"];
                                        let mut biz_ids = Vec::new();
                                        let mut doc_ids = Vec::new();
                                        for id in &ids {
                                            let (tp, _) = parse_source_id(id);
                                            if business_types.contains(&tp.as_str()) {
                                                biz_ids.push(id.clone());
                                            } else {
                                                doc_ids.push(id.clone());
                                            }
                                        }
                                        let mut results = Vec::new();
                                        // 业务对象：直接查数据库
                                        for bid in &biz_ids {
                                            let (tp, raw_id) = parse_source_id(bid);
                                            let row = match tp.as_str() {
                                                "PROJECT" => database::get("SELECT id, name, description, status, owner, budget, bid_time, created_at FROM projects WHERE id=?1", &[json!(raw_id)]),
                                                "PRODUCT" => database::get("SELECT p.id, p.name, COALESCE(ps.name,'') AS series_name, p.description, p.price, p.full_specs AS params, p.sort_order, p.created_at FROM products p LEFT JOIN product_series ps ON p.series_id=ps.id WHERE p.id=?1", &[json!(raw_id)]),
                                                "CHANNEL" | "VENDOR" | "CUSTOMER" => database::get("SELECT id, type, name, phone, address, remark, contact_person, contact_phone, created_at FROM partners WHERE id=?1 AND type=?2", &[json!(raw_id), json!(tp)]),
                                                "CONTACT" => database::get("SELECT id, name, phone, email, position, remark, created_at FROM partner_contacts WHERE id=?1", &[json!(raw_id)]),
                                                "TODO" => database::get("SELECT id, title, content, done, priority, due_date, repeat, created_at, updated_at FROM todos WHERE id=?1", &[json!(raw_id)]),
                                                "SOLUTION" => database::get("SELECT id, project_id, name, created_at FROM solutions WHERE id=?1", &[json!(raw_id)]),
                                                _ => Ok(None),
                                            };
                                             match row {
                                                 Ok(Some(mut r)) => {
                                                     // CONTACT 详情补结构化归属公司（含 id，供后续 link/unlink 直接引用）
                                                     if tp == "CONTACT" {
                                                         if let Ok(ps) = database::all(
                                                             "SELECT pcm.partner_id as id, p.name, p.type FROM partner_contact_mapping pcm JOIN partners p ON p.id=pcm.partner_id WHERE pcm.contact_id=?1",
                                                             &[json!(raw_id)]) {
                                                             r["partners"] = json!(ps);
                                                         }
                                                     }
                                                     results.push(json!({"id": bid, "type": tp, "found": true, "data": r}))
                                                 }
                                                 _ => results.push(json!({"id": bid, "type": tp, "found": false})),
                                             }
                                        }
                                        // 文档对象：走chunk回忆
                                        if !doc_ids.is_empty() {
                                            let doc_result = execute_batch_recall(&doc_ids, &visited_chunks_snapshot, &visited_sources_snapshot).await;
                                            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&doc_result) {
                                                if let Some(objects) = v["data"]["objects"].as_array() {
                                                    for obj in objects { results.push(obj.clone()); }
                                                } else {
                                                    results.push(v);
                                                }
                                            }
                                        }
                                        json!({"result_type": "object_detail_batch", "data": {"objects": results}, "summary": format!("查询 {} 个对象", results.len())}).to_string()
                                    }
                                },
                                "search" => {
                                    // ★ 关键词搜索
                                    let q = a["query"].as_str().unwrap_or("");
                                    let t = a["type"].as_str().unwrap_or("");
                                    let s = a["start_time"].as_str().map(resolve_time);
                                    let e = a["end_time"].as_str().map(resolve_time);
                                    let tr = match (s, e) {
                                        (Some(s), Some(e)) => Some((s, e)),
                                        (Some(s), None) => Some((s, "2099-12-31".to_string())),
                                        (None, Some(e)) => Some(("2000-01-01".to_string(), e)),
                                        (None, None) => None,
                                    };
                                    let limit = a["limit"].as_u64().unwrap_or(50) as usize;
                                    let offset = a["offset"].as_u64().unwrap_or(0) as usize;
                                    execute_object_get(q, t, tr, limit, offset, "search")
                                },
                                "filter" => {
                                    // ★ 结构化筛选
                                    let entity = a["type"].as_str().unwrap_or("");
                                    let ff = a["field_filters"].as_array().map(|a| a.as_slice()).unwrap_or(&[]);
                                    execute_object_catalog(entity, ff, a.get("sort"), &a)
                                },
                                "count" => {
                                    // ★ 统计数量（count_only 由 dispatch 注入：schema 以 mode 表达，实现层读 count_only）
                                    let entity = a["type"].as_str().unwrap_or("");
                                    let ff = a["field_filters"].as_array().map(|a| a.as_slice()).unwrap_or(&[]);
                                    let mut ca = a.clone();
                                    ca["count_only"] = json!(true);
                                    execute_object_catalog(entity, ff, a.get("sort"), &ca)
                                },
                                "group" => {
                                    // ★ 分组统计
                                    let entity = a["type"].as_str().unwrap_or("");
                                    let ff = a["field_filters"].as_array().map(|a| a.as_slice()).unwrap_or(&[]);
                                    execute_object_catalog(entity, ff, a.get("sort"), &a)
                                },
                                "aggregate" => {
                                    // ★ 聚合计算
                                    let entity = a["type"].as_str().unwrap_or("");
                                    let ff = a["field_filters"].as_array().map(|a| a.as_slice()).unwrap_or(&[]);
                                    execute_object_catalog(entity, ff, a.get("sort"), &a)
                                },
                                _ => {
                                    // 兼容旧格式：没有 mode 时，按参数推断
                                    if let Some(ids_arr) = a["id"].as_array().filter(|arr| !arr.is_empty()) {
                                        let ids: Vec<String> = ids_arr.iter().filter_map(|v| v.as_str().map(String::from)).collect();
                                        execute_batch_recall(&ids, &visited_chunks_snapshot, &visited_sources_snapshot).await
                                    } else if let Some(ff) = a["field_filters"].as_array().filter(|arr| !arr.is_empty()) {
                                        let entity = a["type"].as_str().unwrap_or("");
                                        execute_object_catalog(entity, ff, a.get("sort"), &a)
                                    } else {
                                        let q = a["query"].as_str().unwrap_or("");
                                        let t = a["type"].as_str().unwrap_or("");
                                        execute_object_get(q, t, None, 50, 0, "search")
                                    }
                                }
                            };
                            (res, vec![])
                        }
                        "ext_tool_chunk_navigate" => {
                            let res = execute_traverse(
                                a["chunk_id"].as_str().unwrap_or(""),
                                a["edge_type"].as_str().unwrap_or("Next"),
                                a["radius"].as_i64().unwrap_or(10) as usize,
                                &visited,
                                a["target_name"].as_str().unwrap_or(""),
                                &mut traverse_source_counter,
                            );
                            (res, vec![])
                        }
                        "ext_tool_relations" => {
                            let entity_type = a["entity_type"].as_str().unwrap_or("");
                            let entity_id = a["entity_id"].as_str().unwrap_or("");
                            let target_type = a["target_type"].as_str().filter(|s| !s.is_empty());
                            let limit = a["limit"].as_u64().unwrap_or(20) as usize;
                            let offset = a["offset"].as_u64().unwrap_or(0) as usize;
                            let res = execute_relations(entity_type, entity_id, target_type, limit, offset);
                            (res, vec![])
                        }
                        "ext_tool_manage_todo" => {
                            let res = ext_tool_manage_todo(
                                a["action"].as_str().unwrap_or(""),
                                a["title"].as_str().unwrap_or(""),
                                a["content"].as_str().unwrap_or(""),
                                a["priority"].as_str().unwrap_or(""),
                                a["due_date"].as_str().unwrap_or(""),
                                a["repeat"].as_str().unwrap_or(""),
                                a["project_id"].as_str().unwrap_or(""),
                                a["done"].as_bool(),
                                a["todo_id"].as_str().unwrap_or(""),
                            );
                            (res, vec![])
                        }
                        "ext_tool_manage_note" => {
                            let res = ext_tool_manage_note(
                                a["action"].as_str().unwrap_or(""),
                                a["title"].as_str().unwrap_or(""),
                                a["content"].as_str().unwrap_or(""),
                                a["note_id"].as_str().unwrap_or(""),
                                a["notebook"].as_str().unwrap_or(""),
                                a["project_id"].as_str().unwrap_or(""),
                            );
                            (res, vec![])
                        }
                        "ext_tool_memory" => {
                            let items_str = if let Some(arr) = a["items"].as_array() {
                                serde_json::to_string(arr).unwrap_or_default()
                            } else {
                                String::new()
                            };
                            let keywords_str = if let Some(arr) = a["keywords"].as_array() {
                                serde_json::to_string(arr).unwrap_or_default()
                            } else {
                                String::new()
                            };
                            let res = ext_tool_memory(
                                a["action"].as_str().unwrap_or(""),
                                a["content"].as_str().unwrap_or(""),
                                &items_str,
                                a["memory_id"].as_str().unwrap_or(""),
                                a["query"].as_str().unwrap_or(""),
                                &keywords_str,
                                a["start_time"].as_str().unwrap_or(""),
                                a["end_time"].as_str().unwrap_or(""),
                                a["profile"].as_bool().unwrap_or(false),
                            );
                            (res, vec![])
                        }
                        "ext_tool_memory_search" => {
                            // 只读派生工具：固定 action=search，供后台分析师等受限运行体使用（写权限仍在 ext_tool_memory）
                            let res = ext_tool_memory(
                                "search", "", "", "",
                                a["query"].as_str().unwrap_or(""),
                                "",
                                a["start_time"].as_str().unwrap_or(""),
                                a["end_time"].as_str().unwrap_or(""),
                                false,
                            );
                            (res, vec![])
                        }
                        "ext_tool_manage_project" => {
                            (tools::manage_project::execute(&a), vec![])
                        }
                        "ext_tool_manage_partner" => {
                            (tools::manage_partner::execute(&a), vec![])
                        }
                        "ext_tool_manage_contact" => {
                            (tools::manage_contact::execute(&a), vec![])
                        }
                        "ext_tool_report" => {
                            (tools::report::execute(&a), vec![])
                        }
                        "ext_tool_manage_product" => {
                            let res = ext_tool_manage_product(&a);
                            (res, vec![])
                        }
                        "ext_tool_manage_notification" => {
                            (tools::notification::execute(&a, &app_handle), vec![])
                        }
                        "ext_tool_write_document" => {
                            (tools::write_document::execute(&a), vec![])
                        }
                        "ext_tool_delegate_agent" => {
                            let agent_id = a["agent_id"].as_str().unwrap_or("");
                            let task = a["task"].as_str().unwrap_or("");
                            let expected_output = a["expected_output"].as_str().unwrap_or("");
                            // ★ 路由决策日志：记录 LLM 选中的目标（未命中时与注入清单对照）
                            log::info!("[Route] 选择委托 agent_id={} task={:.80}", agent_id, task);
                            // ★ 共享完整上下文：全量 messages 快照直传（废除 recent_messages 20条×200字摘要）；
                            //   context 仅保留主 Agent 显式补充的结构化引用
                            let ctx = a.get("context").cloned()
                                .filter(|v| v.is_object())
                                .unwrap_or(json!({}));
                            // ★ 同轮可见：快照含本轮已完成工具结果（前序委托/检索），依赖链一轮完成
                            let snapshot = build_delegate_snapshot(&messages, &completed_round);
                            let res = crate::services::delegate_agent::execute_delegate_agent(
                                agent_id, task, Some(&ctx), expected_output, app_handle.clone(), delegate_depth,
                                snapshot, session,
                            ).await;
                            // ★ 取消透传 + 委托扣费（合并解析；解析失败也扣基础额度，防绕过）
                            match serde_json::from_str::<Value>(&res) {
                                Ok(v) => {
                                    if v["data"]["cancelled"].as_bool() == Some(true) {
                                        AGENT_CANCEL.store(true, Ordering::Release);
                                    }
                                    let iters = v["data"]["iterations"].as_i64().unwrap_or(1) as i32;
                                    budget -= 30 + iters * 2;
                                }
                                Err(_) => { budget -= 45; }
                            }
                            (res, vec![])
                        }
                        "ext_tool_ask_user" => {
                            // ★ 结构化追问：归一化（≤3题/≤6选项/降级text）后由本轮末尾早退等用户答复
                            log::info!("[AskUser] 主 Agent 发起追问: {:.80}", a["reason"].as_str().unwrap_or(""));
                            (normalize_ask_args(&a), vec![])
                        }
                        _ => (format!("未知工具: {}", name), vec![]),
                    };
                    completed_round.push((*idx, result.clone()));
                    sync_results.push((*idx, result, effects));
                }

                // 合并所有结果并按原始顺序排列
                let mut all_results: Vec<(usize, String, Vec<SessionEffect>)> = Vec::new();
                all_results.extend(async_results);
                all_results.extend(sync_results);
                all_results.sort_by_key(|(idx, _, _)| *idx);

                // 合并缓存结果
                for (idx, cached) in &cached_results_map {
                    all_results.push((*idx, cached.clone(), vec![]));
                }
                all_results.sort_by_key(|(idx, _, _)| *idx);

                // 顺序应用 SessionEffects + 构建 round_tools + messages
                for (idx, result, effects) in &all_results {
                    for effect in effects {
                        session.apply_effect(effect);
                    }

                    let tc = &atc[*idx];
                    let key = (tc.function.name.clone(), tc.function.arguments.clone());
                    dedup_cache.insert(key, result.clone());
                    if !silent { if let Some(ref h) = app_handle {
                        let _ = h.emit("agent:tool-result", json!({"round": iters, "tool": tc.function.name, "result_len": result.chars().count()}));
                    } }
                    let parsed_args: Value = serde_json::from_str(&tc.function.arguments).unwrap_or(json!({}));
                    round_tools.push(json!({"tool":tc.function.name,"args":parsed_args,"result":result,"result_len":result.chars().count()}));
                    messages.push(ChatMessage{role:MessageRole::Tool,content:Some(result.clone()),tool_calls:None,tool_call_id:Some(tc.id.clone()),reasoning_content:None,name:Some(tc.function.name.clone())});
                }
                current_round["tools"] = json!(round_tools);
                tool_log.push(current_round);
                // ★ 同时记录工具调用历史到检索计划（必须先于直出判定：直出 return 路径同样要落账本轮 session 更新）
                for t in &round_tools {
                    let result_str = t["result"].as_str().unwrap_or("");
                    let mut step_desc = t["tool"].as_str().unwrap_or("").to_string();
                    if let Some(args) = t["args"].as_object() {
                        if step_desc == "ext_tool_search" {
                            if let Some(kw) = args.get("keywords").and_then(|v| v.as_array()) {
                                step_desc = format!("search({})", kw.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join(", "));
                            }
                        } else if step_desc == "ext_tool_object" {
                            if let Some(q) = args.get("query").and_then(|v| v.as_str()) {
                                step_desc = format!("object(\"{}\")", q);
                            }
                        } else if step_desc == "ext_tool_chunk_navigate" {
                            if let Some(cid) = args.get("chunk_id").and_then(|v| v.as_str()) {
                                step_desc = format!("traverse({}…)", crate::preview(cid, 8));
                            }
                        }
                    }
                    session.tool_history.push(step_desc);
                    let tool_name = t["tool"].as_str().unwrap_or("");
                    session.update_from_tool_result(tool_name, &result_str);
                }
                // ★ 本轮早退判定（信封一次性解析）：追问优先 > 委托直出
                let parsed_envelopes: Vec<Value> = all_results.iter()
                    .filter_map(|(_, result, _)| serde_json::from_str::<Value>(result).ok())
                    .collect();
                // ★ 追问早退：本轮存在 ask_user pending → 立即结束等用户答复。
                //   同轮若还有合格委托，追问优先（委托结果已入历史，下轮主 LLM 一并综合）
                if let Some(ask_data) = pick_clarification_out(&parsed_envelopes) {
                    let questions = ask_data["questions"].as_array().cloned().unwrap_or_default();
                    let answer = format_clarification_answer(ask_data["reason"].as_str().unwrap_or(""), &questions);
                    log::info!("[追问] 本轮挂起等待用户: {} 个问题", questions.len());
                    if !silent { if let Some(ref h) = app_handle {
                        let _ = h.emit("agent:status", json!({"stage":"answer"}));
                        let _ = h.emit("agent:final-answer", json!({"answer": answer}));
                    } }
                    if let Some(ref sid) = session_id { save_session_state(sid, &session); }
                    let ws = session.generate_scratchpad();
                    return Ok(json!({
                        "success": true, "result_type": "clarification", "answer": answer,
                        "clarification": ask_data,
                        "workspace": ws,
                        "iterations": iters,
                        "promptTokens": session.prompt_tokens,
                        "completionTokens": session.completion_tokens,
                        "cachedTokens": session.cached_tokens,
                        "tool_log": tool_log
                    }));
                }
                // ★ 委托直出：恰好一个合格委托才直出（多委托交回主 LLM 综合；partial+missing 同样交回）
                let direct_candidates: Vec<(String, String, String, bool)> = parsed_envelopes.iter()
                    .map(|v| (
                        v["result_type"].as_str().unwrap_or("").to_string(),
                        v["data"]["status"].as_str().unwrap_or("").to_string(),
                        v["data"]["answer"].as_str()
                            .or_else(|| v["data"]["result"].as_str())
                            .unwrap_or("").to_string(),
                        v["data"]["missing"].as_array().map_or(false, |m| !m.is_empty()),
                    ))
                    .collect();
                if let Some(answer) = pick_direct_out(&direct_candidates) {
                    log::info!("[委托直出] 命中：单委托直出, answer_len={}", answer.len());
                    // ★ 与正常回答路径对齐：委托直出同样向前端广播最终答案
                    if !silent { if let Some(ref h) = app_handle {
                        let _ = h.emit("agent:status", json!({"stage":"answer"}));
                        let _ = h.emit("agent:final-answer", json!({"answer": answer}));
                    } }
                    if let Some(ref sid) = session_id { save_session_state(sid, &session); }
                    let ws = session.generate_scratchpad();
                    return Ok(json!({
                        "success": true, "result_type": "answer", "answer": answer,
                        "delegated_completion": true, "workspace": ws,
                        "iterations": iters,
                        "promptTokens": session.prompt_tokens,
                        "completionTokens": session.completion_tokens,
                        "cachedTokens": session.cached_tokens,
                        "tool_log": tool_log
                    }));
                }

                // ★ 状态驱动熔断检查
                let mut circuit_break_reason = String::new();

                // 条件②：连续两轮没有新 chunk 或活跃对象（用它们作为新事实的代理指标）
                if circuit_break_reason.is_empty() && iters >= 4 {
                    // 只在第 4 轮后检查，给初始搜索留出空间
                    let stale = tool_log.iter().rev().take(2).all(|r| {
                        r["tools"].as_array().map(|tools| tools.iter().all(|t| {
                            let result = t["result"].as_str().unwrap_or("");
                            result.len() < 50 || result.contains("未找到")
                        })).unwrap_or(true)
                    });
                    if stale { circuit_break_reason = "当前范围未发现新内容，尝试 search 新关键词".to_string(); }
                }

                // 条件④：Search 关键词重复率过高（与最近一次搜索比较）
                if let Some(last_search) = round_tools.iter().find(|t| {
                    matches!(t["tool"].as_str(), Some("ext_tool_search"))
                }) {
                    if let Some(args) = last_search["args"].as_object() {
                        if let Some(kws) = args.get("keywords").and_then(|v| v.as_array()) {
                            let cur: std::collections::HashSet<String> = kws.iter().filter_map(|v| v.as_str().map(String::from)).collect();
                            if !cur.is_empty() {
                                for prev in session.search_keywords.iter().rev().skip(1).take(3) {
                                    let prev_set: std::collections::HashSet<String> = prev.iter().cloned().collect();
                                    let intersection = cur.intersection(&prev_set).count();
                                    let union = cur.union(&prev_set).count();
                                    if union > 0 && (intersection as f64 / union as f64) > 0.6 {
                                        circuit_break_reason = "Search 关键词重复，陷入循环".to_string();
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }

                // ★ 条件⑤：本轮 search 未产生新 chunk（来源已全部访问过）
                if circuit_break_reason.is_empty() {
                    let no_new_chunks: bool = round_tools.iter().any(|t| {
                        if t["tool"].as_str() == Some("ext_tool_search") {
                            let result = t["result"].as_str().unwrap_or("");
                            // 没有 ChunkID 行 = 没有新父块
                            !result.contains("ChunkID：")
                        } else { false }
                    });
                    if no_new_chunks && !session.observations.is_empty() {
                        circuit_break_reason = "当前搜索范围已无新内容".to_string();
                    }
                }

                if !circuit_break_reason.is_empty() {
                    messages.push(ChatMessage{role:MessageRole::User,content:Some(format!("【系统提示】{circuit_break_reason}。请根据已有 Known Facts 直接总结回答。")),tool_calls:None,tool_call_id:None,name:None,reasoning_content:None});
                    let ans = call_llm_without_tools(&messages, &app_handle, silent, respond_cancel, &llm_overrides).await.unwrap_or_else(|e| { log::warn!("[Agent] 熔断总结失败: {e}"); format!("总结失败: {e}") });
                    let ws = session.generate_scratchpad();
                    if !silent { if let Some(ref h) = app_handle { let _ = h.emit("agent:status", json!({"stage":"answer"})); let _ = h.emit("agent:final-answer", json!({"answer": ans})); } }
                    if let Some(ref sid) = session_id { save_session_state(sid, &session); }
                    return Ok(json!({"success":true,"result_type":"answer","answer":ans,"workspace":ws,"iterations":iters,"circuit_breaker":true,"promptTokens":session.prompt_tokens,"completionTokens":session.completion_tokens,"cachedTokens":session.cached_tokens,"tool_log":tool_log}));
                }

                // ★ 扣减预算（工具执行后，下一轮生效）
                budget -= search_cost;
                search_cost = ((search_cost as f64 * 1.2) as i32).max(1);

                // ★ 动态上下文注入到消息末尾（工具执行后），确保下一轮 LLM 调用时
                // 前缀为 [System Prompt] + [History] + [Tool Calls/Results...] + [Dynamic Context]
                // 工具结果成为稳定前缀，可被 prompt cache 命中
                {
                    let scratchpad = session.generate_scratchpad();
                    let status = if budget >= 100 { "预算充足" } else if budget >= 50 { "预算偏低" } else { "预算紧张，请优先总结已有信息" };
                    // ★ 计算当前上下文 token 数
                    let context_tokens: i64 = messages.iter().map(|m| {
                        let text_len = m.content.as_ref().map_or(0, |s| s.len() as i64);
                        let tc_len = m.tool_calls.as_ref().map_or(0, |tcs| tcs.iter()
                            .map(|tc| tc.function.arguments.len() as i64)
                            .sum::<i64>());
                        (text_len + tc_len) / 3
                    }).sum();
                     let budget_info = format!(
                        "当前会话工作记忆\n{}\n\n当前时间: {}\n\n搜索预算系统（单次对话）:\n- 总预算: 300 分\n- 计算方式: 每轮扣减，首次8分，逐轮×1.2递增（如8→9→11→13→...）\n- 当前轮次: 第{}轮，本轮将消耗: {}分\n- 剩余预算: {}/300 — {}\n- 当前上下文: {} tokens\n- 建议: 信息足够时主动回答，不要等到预算耗尽",
                        scratchpad, cn_now_str(), iters, search_cost, budget, status, context_tokens
                    );
                    // 移除上一轮的动态上下文消息
                    if let Some(pos) = messages.iter().rposition(|m| m.content.as_deref().map_or(false, |c| c.starts_with("[DYNAMIC_CONTEXT]"))) {
                        messages.remove(pos);
                    }
            messages.push(ChatMessage{role:MessageRole::User, content: Some(if time_only_context { format!("[DYNAMIC_CONTEXT]\n当前时间: {}", cn_now_str()) } else { format!("[DYNAMIC_CONTEXT]\n{}", budget_info) }), tool_calls: None, tool_call_id: None, name: None, reasoning_content: None});
                }
                // ★ 调试：每轮工具调用后保存完整上下文 —— 仅 debug 构建
                #[cfg(debug_assertions)]
                {
                    let context_json: Vec<Value> = messages.iter().filter_map(|m| {
                        serde_json::to_value(m).ok()
                    }).collect();
                    let _ = crate::commands::save_agent_context(context_json);
                }
                // ★ 保存 session 缓存
                if let Some(ref sid) = session_id {
                    save_session_state(sid, &session);
                }
            }
            _ => { if let Some(ref sid) = session_id { save_session_state(sid, &session); } let ws = session.generate_scratchpad(); return Ok(json!({"success":true,"result_type":"answer","answer":format!("根据当前信息：\n\n{}",effective_ac),"workspace":ws,"iterations":iters,"promptTokens":session.prompt_tokens,"completionTokens":session.completion_tokens,"cachedTokens":session.cached_tokens,"tool_log":tool_log})); }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::append_documents_note;
    use super::{pick_direct_out, build_delegate_snapshot, normalize_ask_args, format_clarification_answer, pick_clarification_out, append_delegated_note};
    use crate::services::agent_workflow::{ChatMessage, FunctionCall, MessageRole, ToolCall};
    use serde_json::json;

    fn env(rt: &str, status: &str, answer: &str) -> (String, String, String, bool) {
        (rt.to_string(), status.to_string(), answer.to_string(), false)
    }
    fn env_missing(rt: &str, status: &str, answer: &str, has_missing: bool) -> (String, String, String, bool) {
        (rt.to_string(), status.to_string(), answer.to_string(), has_missing)
    }

    /// P0 回归：单委托合格才直出
    #[test]
    fn direct_out_only_for_single_qualified_delegate() {
        let one = vec![env("delegate_agent_result", "completed", "答案A")];
        assert_eq!(pick_direct_out(&one).as_deref(), Some("答案A"));

        let partial = vec![env("delegate_agent_result", "partial", "部分答案")];
        assert_eq!(pick_direct_out(&partial).as_deref(), Some("部分答案"));
    }

    /// P0 回归：同轮多个委托禁止直出（否则第二份及之后的回答被丢弃，交回主 LLM 综合）
    #[test]
    fn direct_out_rejected_when_multiple_delegates() {
        let two = vec![
            env("delegate_agent_result", "completed", "答案A"),
            env("delegate_agent_result", "completed", "答案B"),
        ];
        assert_eq!(pick_direct_out(&two), None);

        let mixed = vec![
            env("object_create_result", "success", ""),
            env("delegate_agent_result", "completed", "答案A"),
            env("delegate_agent_result", "failed", ""),
        ];
        assert_eq!(pick_direct_out(&mixed), None);
    }

    /// 无委托 / 失败 / 空回答 → 不直出（走主 LLM）
    #[test]
    fn direct_out_rejected_for_no_delegate_or_bad_answer() {
        assert_eq!(pick_direct_out(&[]), None);
        let no_delegate = vec![env("object_create_result", "completed", "x")];
        assert_eq!(pick_direct_out(&no_delegate), None);
        let failed = vec![env("delegate_agent_result", "failed", "错误")];
        assert_eq!(pick_direct_out(&failed), None);
        let empty = vec![env("delegate_agent_result", "completed", "")];
        assert_eq!(pick_direct_out(&empty), None);
    }

    /// partial+missing 非空 → 不直出（MISSING 已从正文剥离，直出会藏缺口令）
    #[test]
    fn direct_out_rejected_for_partial_with_missing() {
        let with_missing = vec![env_missing("delegate_agent_result", "partial", "已完成部分", true)];
        assert_eq!(pick_direct_out(&with_missing), None);
        let without = vec![env_missing("delegate_agent_result", "partial", "已完成", false)];
        assert_eq!(pick_direct_out(&without).as_deref(), Some("已完成"));
    }

    /// 交付稿直出标注：metadata.assistants 非空才追加，正文原样在前
    #[test]
    fn delegated_note_marks_direct_output_provenance() {
        let mut c = "方案框架……".to_string();
        append_delegated_note(&mut c, Some(r#"{"assistants":["解决方案助手"]}"#));
        assert!(c.starts_with("方案框架……"));
        assert!(c.contains("直出交付") && c.contains("解决方案助手") && c.contains("不得代笔续写"));
        // 无 assistants / 非法 metadata / 空数组 → no-op
        let mut d = "正文".to_string();
        append_delegated_note(&mut d, Some(r#"{"documents":[]}"#));
        append_delegated_note(&mut d, None);
        append_delegated_note(&mut d, Some("not-json"));
        append_delegated_note(&mut d, Some(r#"{"assistants":[]}"#));
        assert_eq!(d, "正文");
    }

    /// 追问归一化：截断≤5题/≤6选项、type降级、question_id补全、全非法→invalid
    #[test]
    fn normalize_ask_args_truncates_and_sanitizes() {
        // 正常化
        let a = json!({
            "reason": "为了精准选型",
            "questions": [
                {"question_id": "q1", "prompt": " 架构路线？", "type": "single_choice", "options": (1..=9).map(|i| format!("选项{}", i)).collect::<Vec<_>>()},
                {"prompt": "", "type": "text"},
                {"question_id": "q3", "prompt": "预算范围？", "type": "weird_type"},
                {"question_id": "q4", "prompt": "交付形态？", "type": "multiple_choice", "options": []},
                {"question_id": "q5", "prompt": "第五题", "type": "text"},
                {"question_id": "q6", "prompt": "第六题", "type": "text"},
                {"question_id": "q7", "prompt": "第七题应被截断", "type": "text"}
            ]
        });
        let v: serde_json::Value = serde_json::from_str(&normalize_ask_args(&a)).unwrap();
        assert_eq!(v["result_type"], "ask_user_result");
        assert_eq!(v["data"]["status"], "pending");
        let qs = v["data"]["questions"].as_array().unwrap();
        // 空 prompt 剔除 + ≤5 截断（q1 + q3 + q4 + q5 + q6，q7 丢弃）
        assert_eq!(qs.len(), 5);
        assert_eq!(qs[0]["options"].as_array().unwrap().len(), 6); // ≤6 选项
        assert_eq!(qs[1]["type"], "text"); // 非法 type 降级
        assert_eq!(qs[2]["question_id"], "q4");
        assert_eq!(qs[4]["question_id"], "q6");
        assert_eq!(v["data"]["truncated"], true);

        // 无合格问题 → invalid
        let bad = json!({"reason": "r", "questions": [{"prompt": "  ", "type": "text"}]});
        let v2: serde_json::Value = serde_json::from_str(&normalize_ask_args(&bad)).unwrap();
        assert_eq!(v2["data"]["status"], "invalid");
    }

    /// 追问文本版答案：含 reason 与每题 prompt/options
    #[test]
    fn clarification_answer_formats_questions_as_text() {
        let qs = vec![
            json!({"question_id":"q1","prompt":"业务场景？","type":"single_choice","options":["HIS/PACS","通用办公"]}),
            json!({"question_id":"q2","prompt":"补充说明","type":"text","options":[]}),
        ];
        let text = format_clarification_answer("为了精准编制参数", &qs);
        assert!(text.starts_with("【需要确认】"));
        assert!(text.contains("为了精准编制参数"));
        assert!(text.contains("1. 业务场景？"));
        assert!(text.contains("HIS/PACS / 通用办公"));
        assert!(text.contains("2. 补充说明"));
        assert!(!text.contains("\n   \n"), "text 题无选项不应留空行尾巴");
    }

    /// 早退判定：pending 且有问题的 ask_user_result → Some(data)；invalid/普通工具 → None
    #[test]
    fn clarification_out_picks_pending_ask_envelope() {
        let parsed: Vec<serde_json::Value> = vec![
            json!({"result_type":"object_create_result","data":{"success":true}}),
            json!({"result_type":"ask_user_result","data":{"status":"invalid"}}),
            json!({"result_type":"ask_user_result","data":{"status":"pending","reason":"r","questions":[{"question_id":"q1","prompt":"p","type":"text","options":[]}]}}),
        ];
        let got = pick_clarification_out(&parsed).expect("should pick first pending ask");
        assert_eq!(got["reason"], "r");
        assert_eq!(got["questions"].as_array().unwrap().len(), 1);

        let none: Vec<serde_json::Value> = vec![
            json!({"result_type":"ask_user_result","data":{"status":"pending","questions":[]}}),
        ];
        assert!(pick_clarification_out(&none).is_none());
        assert!(pick_clarification_out(&[]).is_none());
    }

    fn tc(id: &str, name: &str) -> ToolCall {
        ToolCall { id: id.into(), call_type: "function".into(), function: FunctionCall { name: name.into(), arguments: "{}".into() } }
    }
    fn user_msg(s: &str) -> ChatMessage {
        ChatMessage { role: MessageRole::User, content: Some(s.into()), tool_calls: None, tool_call_id: None, name: None, reasoning_content: None }
    }
    fn assistant_with(calls: Vec<ToolCall>) -> ChatMessage {
        ChatMessage { role: MessageRole::Assistant, content: None, tool_calls: Some(calls), tool_call_id: None, name: None, reasoning_content: None }
    }

    /// 即时可见：快照必须是合法序列——assistant 只保留已完成子集，且每个都带 tool 响应
    #[test]
    fn snapshot_shows_completed_results_and_stays_valid() {
        let msgs = vec![
            user_msg("查参数并出表格"),
            assistant_with(vec![tc("c1", "ext_tool_delegate_agent"), tc("c2", "ext_tool_delegate_agent")]),
        ];
        let snap = build_delegate_snapshot(&msgs, &[(0, "委托A结果".into())]);
        // assistant 仅含已完成的 c1，紧跟其 tool 响应
        assert_eq!(snap.len(), 3);
        let a = &snap[1];
        assert_eq!(a.role, MessageRole::Assistant);
        let calls = a.tool_calls.as_ref().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "c1");
        let t = &snap[2];
        assert_eq!(t.role, MessageRole::Tool);
        assert_eq!(t.tool_call_id.as_deref(), Some("c1"));
        assert_eq!(t.content.as_deref(), Some("委托A结果"));
    }

    /// 无已完成结果 → 退化为仅截尾（原 truncate 行为，不携带 assistant）
    #[test]
    fn snapshot_without_completed_strips_dangling_assistant() {
        let msgs = vec![
            user_msg("q"),
            assistant_with(vec![tc("c1", "ext_tool_search")]),
        ];
        let snap = build_delegate_snapshot(&msgs, &[]);
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].role, MessageRole::User);
    }

    /// 多个已完成按原始调用顺序排列
    #[test]
    fn snapshot_orders_completed_by_original_index() {
        let msgs = vec![
            user_msg("q"),
            assistant_with(vec![tc("c1", "ext_tool_search"), tc("c2", "ext_tool_delegate_agent")]),
        ];
        // 故意乱序传入
        let snap = build_delegate_snapshot(&msgs, &[(1, "B".into()), (0, "A".into())]);
        assert_eq!(snap.len(), 4);
        let calls = snap[1].tool_calls.as_ref().unwrap();
        assert_eq!(calls[0].id, "c1");
        assert_eq!(calls[1].id, "c2");
        assert_eq!(snap[2].tool_call_id.as_deref(), Some("c1"));
        assert_eq!(snap[3].tool_call_id.as_deref(), Some("c2"));
    }

    /// 历史回答的 metadata.documents → 文件名注记（与前端文档卡片一致）
    #[test]
    fn documents_note_appended_for_assistant_history() {
        let mut content = "回答正文".to_string();
        let meta = r#"{"documents":[{"file_id":"F1","file_name":"设备清单_20260929_143052.docx","file_type":"docx","file_size":100}]}"#;
        append_documents_note(&mut content, Some(meta));
        assert!(content.starts_with("回答正文"));
        assert!(content.contains("[本条回复生成的文档]"));
        assert!(content.contains("设备清单_20260929_143052.docx"));
        assert!(content.contains("file_id: F1"));
    }

    /// 多文件全列出（不限数量）；无 documents 的各类形态 → no-op
    #[test]
    fn documents_note_multiple_and_noop() {
        let mut c = "x".to_string();
        let meta = r#"{"documents":[{"file_id":"F1","file_name":"a.docx"},{"file_id":"F2","file_name":"b.xlsx"}]}"#;
        append_documents_note(&mut c, Some(meta));
        assert!(c.contains("a.docx") && c.contains("b.xlsx"));

        for meta in [None, Some(""), Some("not-json"), Some("{}"), Some(r#"{"documents":[]}"#)] {
            let mut c2 = "原文".to_string();
            append_documents_note(&mut c2, meta);
            assert_eq!(c2, "原文", "meta={:?}", meta);
        }
    }
}
