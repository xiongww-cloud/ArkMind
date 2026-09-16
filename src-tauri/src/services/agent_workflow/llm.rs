//! LLM 流式调用（SSE）与无工具总结调用

use serde_json::{json, Value};
use crate::services::agent_workflow::*;

/// 单请求流式调用：一次请求同时完成
///   - content 的 token 流式输出到前端（agent:token / agent:start-answer）
///   - tool_calls delta 按 index 累积（工具调用决策）
/// 返回 (content, reasoning_content, tool_calls, usage)
/// 公开接口：流式调用（silent=false）
pub async fn call_llm_streaming(messages: &[ChatMessage], tools: Option<&Value>, app_handle: &Option<tauri::AppHandle>) -> Result<(String, Option<String>, Vec<Value>, Option<Value>), String> {
    call_llm_streaming_inner(messages, tools, app_handle, false).await
}

/// 内部实现：silent=true 时不发 agent:token 事件（用于工具内部的 LLM 调用）
pub async fn call_llm_streaming_inner(messages: &[ChatMessage], tools: Option<&Value>, app_handle: &Option<tauri::AppHandle>, silent: bool) -> Result<(String, Option<String>, Vec<Value>, Option<Value>), String> {
    let (url, api_key, model) = get_api_endpoint();
    // ★ 清理消息内容中的非法字符，防止 LLM API 解析失败
    let msgs: Vec<Value> = messages.iter().enumerate().map(|(i, m)| {
        let mut v = serde_json::to_value(m).unwrap_or_default();
        if let Some(obj) = v.as_object_mut() {
            // 清理 content 字段
            if let Some(content) = obj.get("content").and_then(|c| c.as_str()) {
                let sanitized = sanitize_content(content);
                if sanitized.len() != content.len() {
                    log::warn!("[LLM] 消息 {} 清理了 {} 个非法字符", i, content.len() - sanitized.len());
                }
                obj.insert("content".to_string(), json!(sanitized));
            }
            // 清理 tool_call_id 字段
            if let Some(id) = obj.get("tool_call_id").and_then(|c| c.as_str()) {
                obj.insert("tool_call_id".to_string(), json!(sanitize_content(id)));
            }
            // 移除空的 tool_calls
            if let Some(tc) = obj.get("tool_calls") {
                if tc.as_array().map_or(false, |a| a.is_empty()) { obj.remove("tool_calls"); }
            }
        }
        v
    }).collect();
    let mut body = json!({"model":model,"messages":msgs,"temperature":0.3,"stream":true,"stream_options":{"include_usage":true}});
    if let Some(t) = tools { body["tools"] = t.clone(); body["tool_choice"] = json!("auto"); }
    let mut req = AGENT_HTTP_CLIENT.post(&format!("{}/chat/completions", url)).json(&body);
    if !api_key.is_empty() { req = req.header("Authorization", format!("Bearer {}", api_key)); }
    req = req.header("Accept", "text/event-stream");
    req = req.timeout(std::time::Duration::from_secs(300)); // 覆盖客户端 30s 默认：reqwest 的 timeout 覆盖整个流式周期
    let t0 = std::time::Instant::now();
    let resp = req.send().await.map_err(|e| format!("LLM: {e}"))?;
    log::info!("[SSE] headers +{}ms status={}", t0.elapsed().as_millis(), resp.status());
    if !resp.status().is_success() { let s = resp.status(); let b = resp.text().await.unwrap_or_default(); return Err(format!("LLM {s}: {b}")); }

    // SSE 解析状态
    struct SseState {
        buf: String,
        full: String,
        reasoning: String,
        tca: Vec<(String, String, String)>, // (id, name, arguments)
        usage: Option<Value>,
        count: u64,
        token_buf: String,
        token_buf_start: usize, // token_buf 在 full 中的起始位置
        suppress_tokens: bool,
        done: bool, // 已收到 [DONE]，终止流读取
    }
    let mut st = SseState { buf: String::new(), full: String::new(), reasoning: String::new(), tca: Vec::new(), usage: None, count: 0, token_buf: String::new(), token_buf_start: 0, suppress_tokens: false, done: false };
    let mut stream = resp.bytes_stream();

    // 尝试从 buffer 头部提取一个完整的 SSE JSON 并处理；返回 false 表示数据不完整
    let try_process_one = |st: &mut SseState, app: &Option<tauri::AppHandle>| -> bool {
        let trimmed_start = st.buf.find("data:").unwrap_or(0);
        if trimmed_start > 0 { st.buf = st.buf[trimmed_start..].to_string(); }
        let after_prefix = if st.buf.starts_with("data: ") { 6 } else if st.buf.starts_with("data:") { 5 } else { return false; };
        let json_part = &st.buf[after_prefix..];
        let json_end = json_part.find("data:").unwrap_or(json_part.len());
        let json_str = json_part[..json_end].trim();
        if json_str.is_empty() { return false; }
        if json_str.starts_with("[DONE]") { st.done = true; st.buf = st.buf[after_prefix + json_end..].to_string(); return true; }
        match serde_json::from_str::<Value>(json_str) {
            Ok(chunk) => {
                st.buf = st.buf[after_prefix + json_end..].to_string();
                if chunk["usage"].is_object() { st.usage = Some(chunk["usage"].clone()); }
                if let Some(choices) = chunk["choices"].as_array() {
                    for c in choices {
                        let delta = &c["delta"];
                        // content → 缓存并检测工具调用标记（用 st.full 检测，不受换行/切分影响）
                        if let Some(tok) = delta["content"].as_str() {
                            st.full.push_str(tok);
                            st.count += 1;
                            st.token_buf.push_str(tok);
                            if !st.suppress_tokens {
                                // 检测工具调用标记："<tool_call" 同时匹配 "<tool_call>" 和 "<tool_calls>"（前缀覆盖）
                                if st.full.contains("<tool_call") || st.full.contains("<invoke") {
                                    // 发出标记前的剩余文本（如有），然后抑制后续全部内容
                                    let marker = st.full.find("<tool_call")
                                        .or_else(|| st.full.find("<invoke"))
                                        .unwrap_or(0);
                                    if marker > st.token_buf_start {
                                        if !silent { if let Some(ref h) = app {
                                            let _ = h.emit("agent:token", json!({"token": &st.token_buf[..marker - st.token_buf_start]}));
                                        } }
                                    }
                                    st.suppress_tokens = true;
                                    st.token_buf.clear();
                                    st.token_buf_start = st.full.len();
                                } else if st.token_buf.len() >= 50 || tok.contains('\n') {
                                    if !silent { if let Some(ref h) = app {
                                        let _ = h.emit("agent:token", json!({"token": &st.token_buf}));
                                    } }
                                    st.token_buf.clear();
                                    st.token_buf_start = st.full.len();
                                }
                            }
                        }
                        // reasoning_content → 累积并推送到前端显示思考过程
                        if let Some(rtok) = delta["reasoning_content"].as_str() {
                            st.reasoning.push_str(rtok);
                            if let Some(ref h) = app {
                                let _ = h.emit("agent:thought", json!({"text": rtok}));
                            }
                        }
                        // tool_calls delta → 按 index 累积
                        if let Some(tds) = delta["tool_calls"].as_array() {
                            for td in tds {
                                let idx = td["index"].as_u64().unwrap_or(0) as usize;
                                while st.tca.len() <= idx { st.tca.push((String::new(), String::new(), String::new())); }
                                if let Some(id) = td["id"].as_str() { st.tca[idx].0 = id.to_string(); }
                                if let Some(name) = td["function"]["name"].as_str() { st.tca[idx].1 = name.to_string(); }
                                if let Some(args) = td["function"]["arguments"].as_str() { st.tca[idx].2.push_str(args); }
                            }
                        }
                    }
                }
                true
            }
            Err(_) => false // JSON 不完整，等下一个 chunk
        }
    };

    while let Some(chunk) = stream.next().await {
        let bytes = match chunk {
            Ok(b) => b,
            Err(e) => {     log::error!("SSE stream error +{}ms: {e}", t0.elapsed().as_millis()); return Err(format!("stream: {e}")); }
        };
        st.buf.push_str(&String::from_utf8_lossy(&bytes));
        st.buf = st.buf.replace("\r\n", "\n");
        while try_process_one(&mut st, &app_handle) {}
        if st.done { break; }
        if AGENT_CANCEL.load(Ordering::Acquire) {
            log::info!("SSE stream cancelled +{}ms tokens={}", t0.elapsed().as_millis(), st.count);
            return Err("cancelled".into());
        }
    }
    // 流结束后处理剩余数据
    while try_process_one(&mut st, &app_handle) {}
    // 流结束后 flush 残余 buffer
    if !st.suppress_tokens && !st.token_buf.is_empty() {
        if !silent { if let Some(ref h) = app_handle {
            let _ = h.emit("agent:token", json!({"token": &st.token_buf}));
        } }
        st.token_buf.clear();
        st.token_buf_start = st.full.len();
    }
    log::info!("SSE stream end +{}ms tokens={} len={} tool_calls={}", t0.elapsed().as_millis(), st.count, st.full.len(), st.tca.len());

    let tool_calls: Vec<Value> = st.tca.into_iter()
        .filter(|(id, name, _)| !id.is_empty() || !name.is_empty())
        .enumerate()
        .map(|(i, (id, name, arguments))| json!({
            "id": if id.is_empty() { format!("call_{}", i) } else { id },
            "type": "function",
            "function": {"name": name, "arguments": arguments}
        }))
        .collect();
    let reasoning = if st.reasoning.is_empty() { None } else { Some(st.reasoning) };
    log::info!("[call_llm_streaming] 返回: content_len={}, tool_calls={}, usage={}", st.full.len(), tool_calls.len(), st.usage.is_some());
    Ok((st.full, reasoning, tool_calls, st.usage))
}

/// 流式补生成（无工具）：用于强制总结等路径；答案逐 token 输出到前端
/// 若 LLM 回复清理后为空（疑似工具调用残留），自动重试重新请求，避免检索/回答中断
pub(crate) async fn call_llm_without_tools(messages: &[ChatMessage], app_handle: &Option<tauri::AppHandle>) -> Result<String, String> {
    const MAX_ATTEMPTS: u32 = 3;
    let mut attempt = 0u32;
    let mut retry_msgs: Vec<ChatMessage> = messages.to_vec();
    loop {
        attempt += 1;
        log::info!("[call_llm_without_tools] 被调用（第{}次），messages 长度: {}", attempt, retry_msgs.len());
        let (content, _, _, _) = call_llm_streaming(&retry_msgs, None, app_handle).await?;
        log::info!("[call_llm_without_tools] 返回: content_len={}", content.len());
        // 过滤 DSML / 工具调用 XML 残留
        let cleaned = clean_tool_markers(&content, false);
        if !cleaned.trim().is_empty() {
            return Ok(cleaned);
        }
        if attempt >= MAX_ATTEMPTS {
            log::warn!("[call_llm_without_tools] 重试{}次后仍为空，返回兜底文案", attempt);
            return Ok("已根据已有信息完成总结。".to_string());
        }
        // ★ 重新发送请求：追加纯文本提示，强制模型输出普通文本
        log::warn!("[call_llm_without_tools] 第{}次回复为空（疑似工具调用残留），重新请求 LLM", attempt);
        retry_msgs.push(ChatMessage {
            role: MessageRole::User,
            content: Some("你的上一条回复只包含工具调用标记（tool_call/DSML/XML），没有任何可用文本。请用纯文本直接回答，严禁输出任何工具调用标记、XML 或 DSML 格式。".into()),
            tool_calls: None, tool_call_id: None, name: None, reasoning_content: None,
        });
    }
}

// ═══════════════════════════════════════════════════════════════════
// Chunk metadata enrichment
