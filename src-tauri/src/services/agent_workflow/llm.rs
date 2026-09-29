//! LLM 流式调用（SSE）与无工具总结调用

use serde_json::{json, Value};
use crate::services::agent_workflow::*;

/// 严格流结束校验：未收到 [DONE] 且未收到 finish_reason → 判定上游截断（不再把半截内容伪装成功）。
/// 若所用端点既不发 finish_reason 也不发 [DONE]（不合规端点，正常回复全部报中断），改为 false 保守放行。
const STRICT_STREAM_END: bool = true;

/// 从 buffer 头部取出一个完整 SSE 行并识别 data 载荷。
/// 返回 None = 无完整行（等待更多数据）；Some(None) = 非 data 行或空载荷；Some(Some(payload)) = 完整 data 载荷。
/// 关键：`data:` 前缀只认行首，JSON 字符串值内出现 "data:"（如 base64 URI）不再破坏边界。
pub(crate) fn extract_sse_data(buf: &mut String) -> Option<Option<String>> {
    let nl = buf.find('\n')?;
    let line = buf[..nl].to_string();
    *buf = buf[nl + 1..].to_string();
    let line = line.trim_end_matches('\r');
    match line.strip_prefix("data:") {
        Some(rest) => {
            let p = rest.trim();
            if p.is_empty() { Some(None) } else { Some(Some(p.to_string())) }
        }
        None => Some(None),
    }
}

/// 从 chunk 中提取服务端错误消息（`{"error":{...}}` 或 `{"error":"..."}`）。
/// OpenAI 兼容端常在配额/策略/上游超时中断流前推送 error 事件——必须显式处理，否则被静默忽略。
pub(crate) fn chunk_error_message(chunk: &Value) -> Option<String> {
    let err = chunk.get("error")?;
    if err.is_null() { return None; }
    if let Some(m) = err.as_str() { return if m.is_empty() { None } else { Some(m.to_string()) }; }
    if let Some(m) = err.get("message").and_then(|v| v.as_str()) { if !m.is_empty() { return Some(m.to_string()); } }
    let s = err.to_string();
    if s.is_empty() || s == "{}" { None } else { Some(s) }
}

/// 流结束完整性判定（纯函数，可单测）：
/// - 收到服务端 error 事件 → 明确错误
/// - 收到 [DONE] 或 finish_reason → 正常结束
/// - 两者皆无 → 上游静默截断（如网关空闲掐断连接），返回错误而非伪装成功
pub(crate) fn stream_end_error(got_done: bool, got_finish: bool, server_err: Option<&str>) -> Option<String> {
    if let Some(e) = server_err { return Some(format!("服务端错误: {}", e)); }
    if STRICT_STREAM_END && !got_done && !got_finish {
        return Some("AI 响应在传输中意外中断（上游连接关闭，未收到完成标记），请重试".to_string());
    }
    None
}

/// 续写请求的追加消息（纯函数，可单测）：assistant 半截内容 + 续写指令。
/// 用于流中断后从中断处继续（已推送给前端的半截不重复输出）。
pub(crate) fn build_resume_messages(base: &[ChatMessage], partial: &str) -> Vec<ChatMessage> {
    let mut msgs = base.to_vec();
    msgs.push(ChatMessage {
        role: MessageRole::Assistant,
        content: Some(partial.to_string()),
        tool_calls: None,
        tool_call_id: None,
        name: None,
        reasoning_content: None,
    });
    msgs.push(ChatMessage {
        role: MessageRole::User,
        content: Some("你的上一条回复在传输中意外中断，请从中断处原样继续输出剩余内容：不要重复已输出的任何文字，直接从中断句的断点接着写。".into()),
        tool_calls: None,
        tool_call_id: None,
        name: None,
        reasoning_content: None,
    });
    msgs
}

/// 单次流式调用的失败信息：携带已产出的部分内容，供上层决定续写或原样重试
struct StreamFail {
    message: String,
    /// 失败时已累积的正文（其中 token_buf 积压部分已 flush 给前端）
    partial: String,
    /// 是否已出现工具调用（API tool_calls 或文本抑制标记）——此时不宜续写，应原样重试
    has_tool_calls: bool,
    cancelled: bool,
}

/// SSE 解析状态
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
    done: bool,               // 已收到 [DONE]（或 error 事件），终止流读取
    got_finish: bool,         // 已收到 finish_reason（stop/tool_calls/length 等）
    server_err: Option<String>, // 服务端推送的 error 事件
}

/// flush 积压未发的 token（失败路径也要 flush：保证前端已有内容与 partial 一致，续写无缝衔接）
fn flush_pending_tokens(st: &mut SseState, silent: bool, app: &Option<tauri::AppHandle>) {
    if !st.suppress_tokens && !st.token_buf.is_empty() {
        if !silent {
            if let Some(ref h) = app {
                let _ = h.emit("agent:token", json!({"token": &st.token_buf}));
            }
        }
        st.token_buf.clear();
        st.token_buf_start = st.full.len();
    }
}

/// 请求级模型/温度覆盖（per-agent 配置）：Default = 不覆盖，跟随全局 ChatConfig
#[derive(Default, Clone, Debug)]
pub struct LlmOverrides {
    pub model: Option<String>,
    pub temperature: Option<f64>,
}

/// 公开接口：流式调用（silent=false）
pub async fn call_llm_streaming(messages: &[ChatMessage], tools: Option<&Value>, app_handle: &Option<tauri::AppHandle>) -> Result<(String, Option<String>, Vec<Value>, Option<Value>), String> {
    call_llm_streaming_inner(messages, tools, app_handle, false, true, &LlmOverrides::default()).await
}

/// 内部实现：silent=true 时不发任何聊天生命周期事件（子Agent/后台运行体使用）
/// cancel_aware=false 时完全不读取/响应全局 AGENT_CANCEL（独立后台运行体与聊天"停止"互不误伤）
///
/// 流中断自动恢复（上限 2 次重试）：
/// - 已产出正文且无工具调用 → 续写模式（assistant 半截 + 续写指令），新 token 接在半截之后推送
/// - 空内容或含工具调用 → 原样重试
/// - 用户取消/已重试耗尽 → 直接上抛
pub async fn call_llm_streaming_inner(messages: &[ChatMessage], tools: Option<&Value>, app_handle: &Option<tauri::AppHandle>, silent: bool, cancel_aware: bool, overrides: &LlmOverrides) -> Result<(String, Option<String>, Vec<Value>, Option<Value>), String> {
    const MAX_RETRIES: u32 = 2;
    // 已产出且已推送给前端的正文（续写时作为 assistant 前缀拼回）
    let mut accumulated = String::new();
    let mut attempt = 0u32;
    loop {
        attempt += 1;
        // 重试间隙响应取消信号（流内检查只在收到 chunk 时触发）
        if cancel_aware && AGENT_CANCEL.load(Ordering::Acquire) {
            return Err("cancelled".into());
        }
        let attempt_msgs: Vec<ChatMessage> = if accumulated.is_empty() {
            messages.to_vec()
        } else {
            build_resume_messages(messages, &accumulated)
        };
        match stream_once(&attempt_msgs, tools, app_handle, silent, cancel_aware, overrides).await {
            Ok((content, reasoning, tcs, usage)) => {
                if accumulated.is_empty() {
                    return Ok((content, reasoning, tcs, usage));
                }
                let full = format!("{}{}", accumulated, content);
                log::info!("[LLM] 流中断续写成功: 前段{}字符 + 续段{}字符", accumulated.len(), content.len());
                return Ok((full, reasoning, tcs, usage));
            }
            Err(f) if f.cancelled => return Err("cancelled".into()),
            Err(f) => {
                log::warn!("[LLM] 流式调用失败(第{}次): {}", attempt, f.message);
                if attempt > MAX_RETRIES {
                    log::error!("[LLM] 流式调用重试{}次后仍失败: {}", MAX_RETRIES, f.message);
                    return Err(f.message);
                }
                if !f.partial.trim().is_empty() && !f.has_tool_calls {
                    if accumulated.is_empty() {
                        accumulated = f.partial;
                    } else {
                        accumulated.push_str(&f.partial);
                    }
                    log::warn!("[LLM] 第{}次重试改为续写模式，已产出 {} 字符", attempt, accumulated.len());
                } else {
                    log::warn!("[LLM] 第{}次重试为原样重发", attempt);
                }
            }
        }
    }
}

/// 单次流式请求（无重试）：失败时经 StreamFail 携带已产出内容与失败原因
async fn stream_once(
    messages: &[ChatMessage],
    tools: Option<&Value>,
    app_handle: &Option<tauri::AppHandle>,
    silent: bool,
    cancel_aware: bool,
    overrides: &LlmOverrides,
) -> Result<(String, Option<String>, Vec<Value>, Option<Value>), StreamFail> {
    let (url, api_key, model) = get_api_endpoint();
    // ★ per-agent 覆盖：未配置时跟随全局模型/默认温度
    let model = overrides.model.as_deref().filter(|m| !m.is_empty()).unwrap_or(&model);
    let temperature = overrides.temperature.unwrap_or(0.3);
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
    let mut body = json!({"model":model,"messages":msgs,"temperature":temperature,"stream":true,"stream_options":{"include_usage":true}});
    if let Some(t) = tools { body["tools"] = t.clone(); body["tool_choice"] = json!("auto"); }
    let mut req = AGENT_HTTP_CLIENT.post(&format!("{}/chat/completions", url)).json(&body);
    if !api_key.is_empty() { req = req.header("Authorization", format!("Bearer {}", api_key)); }
    req = req.header("Accept", "text/event-stream");
    req = req.timeout(std::time::Duration::from_secs(300)); // 覆盖客户端 30s 默认：reqwest 的 timeout 覆盖整个流式周期
    let t0 = std::time::Instant::now();
    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => { log::error!("LLM 请求发送失败: {e}"); return Err(StreamFail { message: format!("LLM: {e}"), partial: String::new(), has_tool_calls: false, cancelled: false }); }
    };
    log::info!("[SSE] headers +{}ms status={}", t0.elapsed().as_millis(), resp.status());
    if !resp.status().is_success() {
        let s = resp.status();
        let b = resp.text().await.unwrap_or_default();
        return Err(StreamFail { message: format!("LLM {s}: {b}"), partial: String::new(), has_tool_calls: false, cancelled: false });
    }

    let mut st = SseState {
        buf: String::new(), full: String::new(), reasoning: String::new(), tca: Vec::new(),
        usage: None, count: 0, token_buf: String::new(), token_buf_start: 0,
        suppress_tokens: false, done: false, got_finish: false, server_err: None,
    };
    let mut stream = resp.bytes_stream();

    // 逐行解析 SSE：data: 前缀仅认行首，一行到达即完整事件（内容含 "data:" 不再破坏边界）
    let try_process_one = |st: &mut SseState, app: &Option<tauri::AppHandle>| -> bool {
        let mut processed_any = false;
        loop {
            if st.done { return false; }
            let payload = match extract_sse_data(&mut st.buf) {
                Some(Some(p)) => p,
                Some(None) => continue,
                None => return processed_any,
            };
            processed_any = true;
            if payload == "[DONE]" { st.done = true; return true; }
            match serde_json::from_str::<Value>(&payload) {
            Ok(chunk) => {
                // ★ 服务端 error 事件：立即记录并终止流（此前被静默忽略 → 半截内容被伪装成功）
                if let Some(msg) = chunk_error_message(&chunk) {
                    log::error!("[SSE] 服务端错误事件: {}", msg);
                    st.server_err = Some(msg);
                    st.done = true;
                    return true;
                }
                if chunk["usage"].is_object() { st.usage = Some(chunk["usage"].clone()); }
                // finish_reason（choices[]. 或 chunk 级）：流完整性的正标记
                if chunk["finish_reason"].as_str().map_or(false, |f| !f.is_empty()) { st.got_finish = true; }
                if let Some(choices) = chunk["choices"].as_array() {
                    for c in choices {
                        if c["finish_reason"].as_str().map_or(false, |f| !f.is_empty()) { st.got_finish = true; }
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
                            if !silent {
                                if let Some(ref h) = app {
                                    let _ = h.emit("agent:thought", json!({"text": rtok}));
                                }
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
            }
            Err(e) => { log::warn!("[SSE] 事件载荷解析失败，跳过: {}", e); }
            }
        }
    };

    // 失败退出（流错误/取消）：flush 积压 token 保证前端与 partial 一致
    macro_rules! fail {
        ($msg:expr, $cancelled:expr) => {{
            flush_pending_tokens(&mut st, silent, &app_handle);
            let has_tool_calls = !st.tca.is_empty() || st.suppress_tokens;
            let partial = st.full.clone();
            if $cancelled {
                log::info!("SSE stream cancelled +{}ms tokens={}", t0.elapsed().as_millis(), st.count);
            } else {
                log::error!("SSE stream error +{}ms: {}", t0.elapsed().as_millis(), $msg);
            }
            return Err(StreamFail { message: $msg, partial, has_tool_calls, cancelled: $cancelled });
        }};
    }

    while let Some(chunk) = stream.next().await {
        let bytes = match chunk {
            Ok(b) => b,
            Err(e) => { fail!(format!("stream: {e}"), false); }
        };
        st.buf.push_str(&String::from_utf8_lossy(&bytes));
        st.buf = st.buf.replace("\r\n", "\n");
        while try_process_one(&mut st, &app_handle) {}
        if st.done { break; }
        if cancel_aware && AGENT_CANCEL.load(Ordering::Acquire) {
            fail!("cancelled".to_string(), true);
        }
    }
    // 流结束后处理剩余数据（最后一行可能无换行结尾，补一个再解析）
    if !st.buf.is_empty() { st.buf.push('\n'); }
    while try_process_one(&mut st, &app_handle) {}
    // 流结束后 flush 残余 buffer
    flush_pending_tokens(&mut st, silent, &app_handle);
    log::info!("SSE stream end +{}ms tokens={} len={} tool_calls={} done={} finish={} err={}",
        t0.elapsed().as_millis(), st.count, st.full.len(), st.tca.len(), st.done, st.got_finish, st.server_err.is_some());

    // ★ 完整性判定：服务端 error 事件 / 未收到完成标记 → 视为中断，不把半截内容伪装成功
    if let Some(e) = stream_end_error(st.done, st.got_finish, st.server_err.as_deref()) {
        let has_tool_calls = !st.tca.is_empty() || st.suppress_tokens;
        log::error!("SSE stream incomplete +{}ms: {}", t0.elapsed().as_millis(), e);
        return Err(StreamFail { message: e, partial: st.full.clone(), has_tool_calls, cancelled: false });
    }

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
/// silent/cancel_aware 由调用方工作流透传：后台运行体不得向聊天泄漏 token 事件、不得响应聊天取消信号
pub(crate) async fn call_llm_without_tools(messages: &[ChatMessage], app_handle: &Option<tauri::AppHandle>, silent: bool, cancel_aware: bool, overrides: &LlmOverrides) -> Result<String, String> {
    const MAX_ATTEMPTS: u32 = 3;
    let mut attempt = 0u32;
    let mut retry_msgs: Vec<ChatMessage> = messages.to_vec();
    loop {
        attempt += 1;
        log::info!("[call_llm_without_tools] 被调用（第{}次），messages 长度: {}", attempt, retry_msgs.len());
        let (content, _, _, _) = call_llm_streaming_inner(&retry_msgs, None, app_handle, silent, cancel_aware, overrides).await?;
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

#[cfg(test)]
mod tests {
    use super::{extract_sse_data, stream_end_error, chunk_error_message, build_resume_messages};
    use crate::services::agent_workflow::{ChatMessage, MessageRole};
    use serde_json::json;

    #[test]
    fn data_prefix_inside_json_value_does_not_break_boundary() {
        // 回归：回答含 base64 data URI 时，旧实现把内容里的 "data:" 当事件边界导致整流卡死
        let mut buf = String::from(r#"data: {"choices":[{"delta":{"content":"img data:image/png;base64,AAAA"}}]}"#);
        buf.push('\n');
        buf.push('\n');
        let first = extract_sse_data(&mut buf).expect("line").expect("payload");
        assert_eq!(first, r#"{"choices":[{"delta":{"content":"img data:image/png;base64,AAAA"}}]}"#);
        // 后续空行为 Some(None)，再之后无完整行 → None
        assert!(matches!(extract_sse_data(&mut buf), Some(None)));
        assert!(extract_sse_data(&mut buf).is_none());
    }

    #[test]
    fn incomplete_line_waits_for_more() {
        let mut buf = String::from("data: {\"choi");
        assert!(extract_sse_data(&mut buf).is_none());
        buf.push_str("ces\":[{\"delta\":{\"content\":\"hi\"}}]}\n");
        let p = extract_sse_data(&mut buf).unwrap().unwrap();
        assert_eq!(p, r#"{"choices":[{"delta":{"content":"hi"}}]}"#);
    }

    #[test]
    fn crlf_and_done_and_non_data_lines() {
        let mut buf = String::from("event: message\r\ndata: [DONE]\r\n\r\n");
        assert!(matches!(extract_sse_data(&mut buf), Some(None))); // event: 行
        assert_eq!(extract_sse_data(&mut buf).unwrap().unwrap(), "[DONE]");
        assert!(matches!(extract_sse_data(&mut buf), Some(None))); // 空行
    }

    #[test]
    fn stream_end_error_classification() {
        // 服务端 error 事件 → 明确错误（最高优先级）
        assert!(stream_end_error(true, true, Some("quota exceeded")).unwrap().contains("quota exceeded"));
        // 未收到任何完成标记 → 上游截断（本 bug 的核心场景：历史里存了半截）
        let e = stream_end_error(false, false, None).unwrap();
        assert!(e.contains("意外中断"));
        // 收到 [DONE] → 正常
        assert!(stream_end_error(true, false, None).is_none());
        // 收到 finish_reason → 正常（端点不发 [DONE] 但合规）
        assert!(stream_end_error(false, true, None).is_none());
    }

    #[test]
    fn chunk_error_message_shapes() {
        // 结构化错误
        assert_eq!(chunk_error_message(&json!({"error": {"message": "rate limited", "type": "x"}})).unwrap(), "rate limited");
        // 字符串错误
        assert_eq!(chunk_error_message(&json!({"error": "boom"})).unwrap(), "boom");
        // 无 error / null → None（正常流式块不误判）
        assert!(chunk_error_message(&json!({"choices": []})).is_none());
        assert!(chunk_error_message(&json!({"error": null})).is_none());
    }

    #[test]
    fn build_resume_messages_appends_partial_and_instruction() {
        let base = vec![ChatMessage {
            role: MessageRole::System,
            content: Some("sys".into()),
            tool_calls: None, tool_call_id: None, name: None, reasoning_content: None,
        }];
        let msgs = build_resume_messages(&base, "前半段内容");
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].role, MessageRole::System);
        assert_eq!(msgs[1].role, MessageRole::Assistant);
        assert_eq!(msgs[1].content.as_deref(), Some("前半段内容"));
        assert_eq!(msgs[2].role, MessageRole::User);
        assert!(msgs[2].content.as_deref().unwrap().contains("继续"));
        // base 未被修改
        assert_eq!(base.len(), 1);
    }
}
