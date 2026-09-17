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

#[tauri::command]
pub async fn agent_chat_workflow(user_query: String, attachments: Vec<Value>, max_iterations: Option<i32>, app_handle: Option<tauri::AppHandle>, silent: bool, session_id: Option<String>, allowed_tools: Option<Vec<String>>) -> Result<Value, String> {
    agent_chat_workflow_internal(user_query, attachments, max_iterations, app_handle, silent, session_id, allowed_tools, None, true, 0, true, false).await
}

/// 内部工作流函数，支持自定义 system_prompt
/// is_root: true=顶层工作流（控制 AGENT_CANCEL 生命周期），false=子Agent（保留取消信号向上传播）
/// delegate_depth: 当前委托深度（0=主Agent，≥1=子Agent）
/// respond_cancel: false=独立后台运行体，完全无视全局 AGENT_CANCEL（与聊天"停止"互不干扰）
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
) -> Result<Value, String> {
    let effective_prompt = system_prompt.unwrap_or(AGENT_SYSTEM_PROMPT);
    // ★ 加载可用专业 Agent 列表，注入系统提示词（仅当有委托能力时）
    let has_delegate = allowed_tools.as_ref().map_or(true, |t| t.iter().any(|x| x == "ext_tool_delegate_agent"));
    let agent_section = if has_delegate {
        let agents: Vec<Value> = database::all(
            "SELECT id, name, description FROM agent_profiles WHERE status='active'",
            &[],
        ).unwrap_or_default();
        if agents.is_empty() {
            String::new()
        } else {
            let lines: Vec<String> = agents.iter().filter_map(|a| {
                let id = a["id"].as_str()?;
                let name = a["name"].as_str()?;
                let desc = a["description"].as_str().unwrap_or("无描述");
                Some(format!("- {}: {} (agent_id: \"{}\")", name, desc, id))
            }).collect();
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

    let tools = build_tools_definitions(allowed_tools.as_deref());
    // ★ Session 缓存：从 chat_sessions.session_state 恢复（始终遍历所有历史）
    let mut session = if let Some(ref sid) = session_id {
        load_session_state(sid).unwrap_or_else(|| {
            log::info!("[Session] 无缓存，从头创建");
            SessionMemory::new()
        })
    } else {
        SessionMemory::new()
    };
    // ★ 记录加载时的快照，save 时只保存本次提问新增的 visited（差集）
    session.prev_visited_chunks = session.visited_chunks.clone();
    session.prev_visited_sources = session.visited_sources.clone();

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
            "SELECT role, content, created_at FROM chat_messages WHERE session_id=?1 AND status='active' ORDER BY id",
            &[json!(sid)],
        ).unwrap_or_default();

        log::info!("[AgentWorkflow] 从 DB 加载 {} 条 active 消息, 压缩摘要: {}",
            rows.len(), compressed_summary.is_some());

        rows.into_iter().filter_map(|r| {

            let role = r["role"].as_str().unwrap_or("");
            let db_content = r["content"].as_str().unwrap_or("");
            let created_at = r["created_at"].as_str().unwrap_or("");

            if db_content.is_empty() { return None; }

            // 为 user 和 assistant 消息添加时间戳（格式与发送时一致，保证 prompt cache 命中）
            let time_str = chrono::NaiveDateTime::parse_from_str(created_at, "%Y-%m-%d %H:%M:%S").map(cn_datetime_str).unwrap_or_default();
            let content_with_timestamp = if role == "user" && !time_str.is_empty() {
                format!("时间：{}\n提问：{}", time_str, db_content)
            } else if role == "assistant" && !time_str.is_empty() {
                format!("时间：{}\n回答：{}", time_str, db_content)
            } else {
                db_content.to_string()
            };

            // 历史消息纯内容，不含 metadata（tool_metadata 在 session 级别）
            let msg = json!({"role": role, "content": content_with_timestamp});
            Some(msg)
        }).collect()
    } else {
        vec![]
    };

    let mut messages: Vec<ChatMessage> = vec![
        ChatMessage{role:MessageRole::System,content:Some(effective_prompt.to_string()),tool_calls:None,tool_call_id:None,name:None,reasoning_content:None},
    ];

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
                let content = h["content"].as_str().map(|s| s.to_string());
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
    }

    // ★ 追加本轮新问题（带时间戳前缀，与历史加载格式一致，保证 LLM prompt cache 命中）
    let now = cn_now_str();
    let timestamped = format!("时间：{}\n提问：{}", now, augmented_query);
    messages.push(ChatMessage{role:MessageRole::User,content:Some(timestamped),tool_calls:None,tool_call_id:None,name:None,reasoning_content:None});

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
                            "SELECT role, content, created_at FROM chat_messages WHERE session_id=?1 AND status='active' ORDER BY id",
                            &[json!(sid)],
                        ).unwrap_or_default();
                        log::info!("[ContextCompressor] 从 DB 加载 {} 条 active 消息", rows.len());

                        rows.into_iter().filter_map(|r| {
                            let role = r["role"].as_str().unwrap_or("");
                            let db_content = r["content"].as_str().unwrap_or("");
                            let created_at = r["created_at"].as_str().unwrap_or("");

                            if db_content.is_empty() { return None; }

                            // 为 user 和 assistant 消息添加时间戳（格式与发送时一致）
                            let time_str = chrono::NaiveDateTime::parse_from_str(created_at, "%Y-%m-%d %H:%M:%S").map(cn_datetime_str).unwrap_or_default();
                            let content_with_timestamp = if role == "user" && !time_str.is_empty() {
                                format!("时间：{}\n提问：{}", time_str, db_content)
                            } else if role == "assistant" && !time_str.is_empty() {
                                format!("时间：{}\n回答：{}", time_str, db_content)
                            } else {
                                db_content.to_string()
                            };

                            let msg = json!({"role": role, "content": content_with_timestamp});
                            Some(msg)
                        }).collect()
                    };
                    // ★ 恢复压缩后的近期消息
                    for h in &recent_history {
                        let role = h["role"].as_str().unwrap_or("");
                        match role {
                            "assistant" => {
                                let content = h["content"].as_str().map(|s| s.to_string());
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
    
    // 加载用户策略（policy）— 仅智能问答主Agent注入（自定义 system_prompt 的运行体如分析师/子Agent不注入）；输出规范已固化在系统提示词第9节，不再动态注入
    let user_policies = if is_root && system_prompt.is_none() {
        let policies = crate::database::all(
            "SELECT id, content, updated_at FROM policies ORDER BY updated_at DESC",
            &[],
        ).unwrap_or_default();
        let lines: Vec<String> = policies.iter().filter_map(|r| {
            let sid = r["id"].as_str()?;
            let content = r["content"].as_str()?;
            let date = r["updated_at"].as_str().unwrap_or("").get(..10).unwrap_or("");
            Some(format!("[ID:{}] {} {}", sid, date, content))
        }).collect();
        if lines.is_empty() {
            "无特殊策略要求。".to_string()
        } else {
            format!("【用户策略】\n{}", lines.join("\n"))
        }
    } else {
        String::new()
    };
    
    // System Prompt 注入可用 Agent 列表；【关于我】仅智能问答主Agent注入（与用户策略同口径）
    let whoami_section = if is_root && system_prompt.is_none() {
        let block = crate::services::whoami::Whoami::load().prompt_block();
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
    messages[0].content = Some(format!("{}{}{}{}", effective_prompt, agent_section, whoami_section, web_section));
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
        
        log::info!("预算: {}/150 cost={}", budget, search_cost);
        if !silent { if let Some(ref h) = app_handle { let _ = h.emit("agent:status", json!({"stage":"thinking","msg":"思考中..."})); let _ = h.emit("agent:progress", json!({"msg":"思考中..."})); } }
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
            messages.push(ChatMessage{role:MessageRole::User, content: Some(if time_only_context { format!("[DYNAMIC_CONTEXT]\n当前时间: {}", cn_now_str()) } else if user_policies.is_empty() { format!("[DYNAMIC_CONTEXT]\n{}", budget_info) } else { format!("[DYNAMIC_CONTEXT]\n{}\n\n{}", user_policies, budget_info) }), tool_calls: None, tool_call_id: None, name: None, reasoning_content: None});
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
        let (ac_owned, reasoning, tcs_streamed, usage) = match call_llm_streaming_inner(&messages, Some(&tools), &app_handle, silent, respond_cancel).await {
            Ok(r) => {
                log::info!("[Agent] call_llm_streaming 成功: content_len={}, tool_calls={}", r.0.len(), r.2.len());
                r
            }
            Err(e) => {
                log::error!("[Agent] call_llm_streaming 失败: {}", e);
                if !silent { if let Some(ref h) = app_handle {
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
                    answer = call_llm_without_tools(&messages, &app_handle, silent, respond_cancel).await.unwrap_or_else(|e| { log::warn!("[Agent] 流式补生成失败，降级为纯文本回答: {e}"); format!("未能生成有效回答: {e}") });
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
                    let ans = call_llm_without_tools(&messages, &app_handle, silent, respond_cancel).await.unwrap_or_else(|e| { log::warn!("[Agent] 预算不足总结失败: {e}"); format!("总结失败: {e}") });
                    if let Some(ref sid) = session_id { save_session_state(sid, &session); }
                    let ws = session.generate_scratchpad();
                    if !silent { if let Some(ref h) = app_handle { let _ = h.emit("agent:status", json!({"stage":"answer"})); let _ = h.emit("agent:final-answer", json!({"answer": &ans})); } }
                    return Ok(json!({"success":true,"result_type":"answer","answer":ans,"workspace":ws,"iterations":iters,"budget_exhausted":true,"promptTokens":session.prompt_tokens,"completionTokens":session.completion_tokens,"cachedTokens":session.cached_tokens,"tool_log":tool_log}));
                }
                if iters >= max_iters {
                    messages.push(ChatMessage{role:MessageRole::User,content:Some("已达到工具调用上限，请直接总结。".into()),tool_calls:None,tool_call_id:None,name:None,reasoning_content:None});
                    let ans = call_llm_without_tools(&messages, &app_handle, silent, respond_cancel).await.unwrap_or_else(|e| { log::warn!("[Agent] 迭代上限总结失败: {e}"); format!("总结失败: {e}") });
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
                            "ext_tool_chunk_navigate" => args.get("chunk_id").and_then(|c| c.as_str()).map(|c| format!("chunk {}", &c[..c.len().min(8)])),
                            _ => None,
                        };
                        let _ = h.emit("agent:status", json!({"stage":"tool_start","tool":tc.function.name,"target":target.unwrap_or_default()}));
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
                    let mut handles = Vec::new();
                    for (orig_idx, name, args_str) in &async_tasks {
                        let name = name.clone();
                        let args_str = args_str.clone();
                        let orig_idx = *orig_idx;
                        let scope = scope_snapshot.clone();
                        let vs = visited_sources_snapshot.clone();
                        let vc = visited_chunks_snapshot.clone();
                        let shown_mem = session.shown_memory_ids.clone();
                        handles.push(tokio::spawn(async move {
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
                        }));
                    }
                    for handle in handles {
                        match handle.await {
                            Ok(result) => {
                                results_mutex.lock().unwrap_or_else(|e| e.into_inner()).push(result);
                            }
                            Err(e) => {
                                log::error!("[Agent] 异步工具任务 JoinHandle 失败: {}", e);
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
                                                Ok(Some(r)) => results.push(json!({"id": bid, "type": tp, "found": true, "data": r})),
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
                                    // ★ 统计数量
                                    let entity = a["type"].as_str().unwrap_or("");
                                    let ff = a["field_filters"].as_array().map(|a| a.as_slice()).unwrap_or(&[]);
                                    execute_object_catalog(entity, ff, a.get("sort"), &a)
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
                                a["done"].as_bool().unwrap_or(false),
                                a["todo_id"].as_str().unwrap_or(""),
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
                            );
                            (res, vec![])
                        }
                        "ext_tool_manage_policy" => {
                            (tools::policy::execute(&a), vec![])
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
                        "ext_tool_manage_product" => {
                            let res = ext_tool_manage_product(&a);
                            (res, vec![])
                        }
                        "ext_tool_manage_notification" => {
                            (tools::notification::execute(&a, &app_handle), vec![])
                        }
                        "ext_tool_delegate_agent" => {
                            let agent_id = a["agent_id"].as_str().unwrap_or("");
                            let task = a["task"].as_str().unwrap_or("");
                            let context = a.get("context");
                            let expected_output = a["expected_output"].as_str().unwrap_or("");
                            let res = crate::services::delegate_agent::execute_delegate_agent(
                                agent_id, task, context, expected_output, app_handle.clone(), delegate_depth,
                            ).await;
                            // ★ 取消透传：子Agent返回cancelled时，传播取消信号
                            if let Ok(v) = serde_json::from_str::<Value>(&res) {
                                if v["data"]["cancelled"].as_bool() == Some(true) {
                                    AGENT_CANCEL.store(true, Ordering::Release);
                                }
                            }
                            // ★ 委托扣费：主Agent感知委托成本
                            if let Ok(v) = serde_json::from_str::<Value>(&res) {
                                let iters = v["data"]["iterations"].as_i64().unwrap_or(0) as i32;
                                budget -= 40 + iters * 2;
                                search_cost += 30;
                            }
                            (res, vec![])
                        }
                        _ => (format!("未知工具: {}", name), vec![]),
                    };
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
                // ★ 同时记录工具调用历史到检索计划
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
                                step_desc = format!("traverse({}…)", &cid[..cid.len().min(8)]);
                            }
                        }
                    }
                    session.tool_history.push(step_desc);
                    let tool_name = t["tool"].as_str().unwrap_or("");
                    session.update_from_tool_result(tool_name, &result_str);
                }

                // ★ 子Agent单轮执行跳过总结：首个工具轮全部为成功的执行类调用
                //   → 跳过后续LLM总结轮，直接返回工具摘要（链式任务天然需要多轮，不受影响）
                if !is_root && tool_log.len() == 1 && !round_tools.is_empty() {
                    let all_exec_success = round_tools.iter().all(|t| {
                        t["tool"].as_str().unwrap_or("").starts_with("ext_tool_manage_")
                            && serde_json::from_str::<Value>(t["result"].as_str().unwrap_or(""))
                                .ok()
                                .map(|v| v["data"]["success"].as_bool() == Some(true))
                                .unwrap_or(false)
                    });
                    if all_exec_success {
                        let summaries: Vec<String> = round_tools.iter().filter_map(|t| {
                            serde_json::from_str::<Value>(t["result"].as_str().unwrap_or(""))
                                .ok()
                                .and_then(|v| v["summary"].as_str().map(|s| s.to_string()))
                        }).collect();
                        let answer = if summaries.is_empty() { "所有操作已完成。".to_string() } else { summaries.join("\n") };
                        log::info!("[子Agent跳过总结] 单轮 {} 次执行类调用全部成功，省去总结轮", round_tools.len());
                        let ws = session.generate_scratchpad();
                        return Ok(json!({"success":true,"result_type":"answer","answer":answer,"workspace":ws,"iterations":iters,"promptTokens":session.prompt_tokens,"completionTokens":session.completion_tokens,"cachedTokens":session.cached_tokens,"tool_log":tool_log}));
                    }
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
                    let ans = call_llm_without_tools(&messages, &app_handle, silent, respond_cancel).await.unwrap_or_else(|e| { log::warn!("[Agent] 熔断总结失败: {e}"); format!("总结失败: {e}") });
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
            messages.push(ChatMessage{role:MessageRole::User, content: Some(if time_only_context { format!("[DYNAMIC_CONTEXT]\n当前时间: {}", cn_now_str()) } else if user_policies.is_empty() { format!("[DYNAMIC_CONTEXT]\n{}", budget_info) } else { format!("[DYNAMIC_CONTEXT]\n{}\n\n{}", user_policies, budget_info) }), tool_calls: None, tool_call_id: None, name: None, reasoning_content: None});
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
