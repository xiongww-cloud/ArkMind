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
    /// 已累积的工具调用参数总字数（大参数流中断时上层据此决定不盲重发）
    tool_args_chars: usize,
}

/// 增量 UTF-8 解码：chunk 可能在多字节字符（如汉字 3 字节）中间断包，
/// 逐 chunk 独立 from_utf8_lossy 会把半个字符替换成 U+FFFD（Word 中显示为 "?" 乱码）。
/// 有效前缀入 buf；"某完整字符的前缀但被截断"（error_len=None）残字节续传到下轮；
/// 真坏字节（error_len=Some）与 lossy 语义等价写一个 U+FFFD 并跳过，保证不卡死。
fn push_utf8_incremental(buf: &mut String, pending: &mut Vec<u8>, bytes: &[u8]) {
    pending.extend_from_slice(bytes);
    loop {
        match std::str::from_utf8(pending) {
            Ok(s) => {
                buf.push_str(s);
                pending.clear();
                return;
            }
            Err(e) => {
                let up = e.valid_up_to();
                if up > 0 {
                    if let Ok(s) = std::str::from_utf8(&pending[..up]) {
                        buf.push_str(s);
                    }
                    pending.drain(..up);
                }
                match e.error_len() {
                    Some(n) => {
                        buf.push('\u{FFFD}');
                        pending.drain(..n.min(pending.len()));
                    }
                    // None = 截断前缀：保留在 pending，等待后续 chunk 补全
                    None => return,
                }
            }
        }
    }
}

/// 流终止时冲刷 pending 残字节（≤3 字节的不完整前缀，与旧 lossy 行为对齐）
fn flush_pending_utf8(buf: &mut String, pending: &mut Vec<u8>) {
    if pending.is_empty() {
        return;
    }
    buf.push_str(&String::from_utf8_lossy(pending));
    pending.clear();
}

/// SSE 解析状态
struct SseState {
    buf: String,
    /// 未完成的 UTF-8 序列字节（chunk 断包续传，见 push_utf8_incremental）
    pending: Vec<u8>,
    full: String,
    reasoning: String,
    tca: Vec<(String, String, String)>, // (id, name, arguments)
    usage: Option<Value>,
    count: u64,
    token_buf: String,
    token_buf_start: usize, // token_buf 在 full 中的起始位置
    thought_buf: String,    // 思考流批量缓冲（与 token 同窗口节流，前端渲染风暴防线）
    suppress_tokens: bool,
    done: bool,               // 已收到 [DONE]（或 error 事件），终止流读取
    got_finish: bool,         // 已收到 finish_reason（stop/tool_calls/length 等）
    finish: Option<String>,   // 具体 finish_reason（length = 输出被模型长度上限截断）
    server_err: Option<String>, // 服务端推送的 error 事件
    prog_last_chars: usize,   // 生成心跳：上次已上报的产出字符数
    prog_last_at: std::time::Instant, // 生成心跳：上次上报时刻
}

/// flush 积压未发的 token（失败路径也要 flush：保证前端已有内容与 partial 一致，续写无缝衔接）
/// to_ui=DELIVERY 通道授权（OutputClass 裁决）；thought_to_ui=思考流通道（!silent：主链+全部子链直出，后台运行体安静）
/// owner=token 归属（"main" 或子 Agent id）
fn flush_pending_tokens(st: &mut SseState, to_ui: bool, thought_to_ui: bool, owner: &str, sid: &str, app: &Option<tauri::AppHandle>) {
    if !st.thought_buf.is_empty() {
        if thought_to_ui {
            if let Some(ref h) = app {
                let _ = h.emit("agent:thought", json!({"text": &st.thought_buf, "owner": owner, "sid": sid}));
            }
        }
        st.thought_buf.clear();
    }
    if !st.suppress_tokens && !st.token_buf.is_empty() {
        if to_ui {
            if let Some(ref h) = app {
                let _ = h.emit("agent:token", json!({"token": &st.token_buf, "owner": owner, "sid": sid}));
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
    /// 委托路径注入：当前子 Agent 的 id，随 writing 心跳下发供前端按 Agent 分桶显示（主流程为 None）
    pub agent_id: Option<String>,
    /// 取消归属会话（per-session 停止信号；workflow 注入，委托子链共享父会话）
    pub cancel_sid: Option<String>,
    /// 委托路径注入：产出类别 key（natural/deliverable），心跳据此让前端选择字数/步数展示口径
    pub agent_class: Option<String>,
}

/// 公开接口：流式调用（silent=false）
pub async fn call_llm_streaming(messages: &[ChatMessage], tools: Option<&Value>, app_handle: &Option<tauri::AppHandle>) -> Result<(String, Option<String>, Vec<Value>, Option<Value>), String> {
    call_llm_streaming_inner(messages, tools, app_handle, false, true, &LlmOverrides::default()).await
}

/// 内部实现：silent=true 时不发任何聊天生命周期事件（子Agent/后台运行体使用）
/// cancel_aware=false 时完全不读取/响应会话取消信号（独立后台运行体与聊天"停止"互不误伤）
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
        if cancel_aware && overrides.cancel_sid.as_deref().map_or(false, crate::services::agent_workflow::session_cancelled) {
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
                    log::error!("[LLM] 流式调用重试{}次后仍失败: {}", attempt, f.message);
                    return Err(f.message);
                }
                // ★ 大型工具流快失败：工具参数已深度累积（长文档生成中）后断流，原样重发=整篇重生成，
                //   大概率同样死且再耗数分钟静默——立即失败，交调用方（主 Agent 的重试语义）决策
                if f.tool_args_chars > 2000 {
                    log::error!("[LLM] 工具参数已累积 {} 字后中断，不原样重发直接失败", f.tool_args_chars);
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
    let (g_url, g_key, g_model) = get_api_endpoint();
    // ★ per-agent 覆盖：未配置时跟随全局模型/默认温度
    let ovr_model = overrides.model.as_deref().filter(|m| !m.is_empty());
    let model = ovr_model.unwrap_or(&g_model);
    // ★ 跨服务商路由：专属模型属于其他已配置 provider 时同时换接口与密钥；
    //   active 模型/同服务商自定义模型名 → None，保持全局路径原语义
    let (url, api_key) = match ovr_model.and_then(|m| crate::services::chat_api::resolve_provider_for_model(m)) {
        Some((u, k)) => (u, k),
        None => (g_url, g_key),
    };
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
    req = req.timeout(std::time::Duration::from_secs(1800)); // 总上限放宽到 30 分钟（长文档生成场景）；掐断死流的责任交给下面的 120s 空闲超时
    let t0 = std::time::Instant::now();
    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => { log::error!("LLM 请求发送失败: {e}"); return Err(StreamFail { message: format!("LLM: {e}"), partial: String::new(), has_tool_calls: false, cancelled: false, tool_args_chars: 0 }); }
    };
    log::info!("[SSE] headers +{}ms status={}", t0.elapsed().as_millis(), resp.status());
    if !resp.status().is_success() {
        let s = resp.status();
        let b = resp.text().await.unwrap_or_default();
        return Err(StreamFail { message: format!("LLM {s}: {b}"), partial: String::new(), has_tool_calls: false, cancelled: false, tool_args_chars: 0 });
    }

    let mut st = SseState {
        buf: String::new(), pending: Vec::new(), full: String::new(), reasoning: String::new(), tca: Vec::new(),
        usage: None, count: 0, token_buf: String::new(), token_buf_start: 0, thought_buf: String::new(),
        suppress_tokens: false, done: false, got_finish: false, finish: None,
        server_err: None, prog_last_chars: 0, prog_last_at: std::time::Instant::now(),
    };
    let mut stream = resp.bytes_stream();

    // ★ 通道框架：DELIVERY 直出授权由 OutputClass 单点裁决——执行型子 Agent 的正文/思考是
    //   交主 Agent 的 HANDOFF 内部汇报，不直连用户 UI（防内部问句泄漏、与主 Agent 转述/卡片双重呈现）；
    //   主 Agent（class=None）与交付型（deliverable）保持流式直出。owner 显式声明每段 token 归属。
    let streams_user = match overrides.agent_class.as_deref() {
        None => true, // 主 Agent 自身恒直出
        Some(c) => crate::services::agent_workflow::output_class::OutputClass::from_format(c).streams_to_user(),
    };
    let to_ui = !silent && streams_user;
    let owner = overrides.agent_id.clone().unwrap_or_else(|| "main".to_string());
    let sid_str = overrides.cancel_sid.clone().unwrap_or_default();

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
                if chunk["finish_reason"].as_str().filter(|f| !f.is_empty()).map(|f| f.to_string()).is_some() { st.got_finish = true; st.finish = chunk["finish_reason"].as_str().map(|f| f.to_string()); }
                if let Some(choices) = chunk["choices"].as_array() {
                    for c in choices {
                        if c["finish_reason"].as_str().filter(|f| !f.is_empty()).is_some() { st.got_finish = true; st.finish = c["finish_reason"].as_str().map(|f| f.to_string()); }
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
                                        if to_ui { if let Some(ref h) = app {
                                            let _ = h.emit("agent:token", json!({"token": &st.token_buf[..marker - st.token_buf_start], "owner": owner, "sid": sid_str.clone()}));
                                        } }
                                    }
                                    st.suppress_tokens = true;
                                    st.token_buf.clear();
                                    st.token_buf_start = st.full.len();
                                } else if st.token_buf.len() >= 16 || tok.contains('\n') {
                                    if to_ui { if let Some(ref h) = app {
                                        let _ = h.emit("agent:token", json!({"token": &st.token_buf, "owner": owner, "sid": sid_str.clone()}));
                                    } }
                                    st.token_buf.clear();
                                    st.token_buf_start = st.full.len();
                                }
                            }
                        }
        // reasoning_content → 累积并推送到前端显示思考过程（批量窗：首段立发，其后 ≥50字/换行 一批）
        if let Some(rtok) = delta["reasoning_content"].as_str() {
            let first = st.reasoning.is_empty();
            st.reasoning.push_str(rtok);
            st.thought_buf.push_str(rtok);
            if !silent && (first || st.thought_buf.len() >= 50 || rtok.contains('\n')) {
                if let Some(ref h) = app {
                    let _ = h.emit("agent:thought", json!({"text": &st.thought_buf, "owner": owner, "sid": sid_str.clone()}));
                }
                st.thought_buf.clear();
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
            flush_pending_tokens(&mut st, to_ui, !silent, &owner, &sid_str, &app_handle);
            let has_tool_calls = !st.tca.is_empty() || st.suppress_tokens;
            let tool_args_chars = st.tca.iter().map(|(_, _, a)| a.len()).sum();
            let partial = st.full.clone();
            if $cancelled {
                log::info!("SSE stream cancelled +{}ms tokens={}", t0.elapsed().as_millis(), st.count);
            } else {
                log::error!("SSE stream error +{}ms: {}", t0.elapsed().as_millis(), $msg);
            }
            return Err(StreamFail { message: $msg, partial, has_tool_calls, cancelled: $cancelled, tool_args_chars });
        }};
    }

    loop {
        // ★ 空闲超时：120s 收不到任何 chunk 判连接已死（生成中的流每几秒必有增量，不会误杀大文档）
        let next = match tokio::time::timeout(std::time::Duration::from_secs(120), stream.next()).await {
            Ok(Some(chunk)) => chunk,
            Ok(None) => break,
            Err(_) => { fail!("流空闲超时：120 秒未收到数据，连接疑似已断开".to_string(), false); }
        };
        let bytes = match next {
            Ok(b) => b,
            Err(e) => { fail!(format!("stream: {e}"), false); }
        };
        push_utf8_incremental(&mut st.buf, &mut st.pending, &bytes);
        st.buf = st.buf.replace("\r\n", "\n");
        while try_process_one(&mut st, &app_handle) {}
        // ★ 生成心跳：每 1s 且产出有增量即上报已产字符数（含工具参数——文档助手长文生成期的"活着"证据；
        //   有增量才发 → 静默等待期零开销，流式期上限每秒一条）
        // 在途未成行字节估算：部分 provider 把 tool_calls 参数攒成单个巨型 JSON 事件最后才吐，
        // 期间 produced 恒定 → 心跳冻结显示"固定字数"；把 buf 滞留量按 0.8 系数计入实时进度。
        // 增量下发型 provider 每行极短，此值恒≈0，不影响原口径。
        let inflight = st.buf.len().saturating_sub(160) * 8 / 10;
        let produced = st.full.len() + st.reasoning.len() + st.tca.iter().map(|(_, _, a)| a.len()).sum::<usize>() + inflight;
        if !silent && produced > st.prog_last_chars && st.prog_last_at.elapsed().as_millis() >= 1000 {
            if let Some(ref h) = app_handle {
                let tool_steps = messages.iter().filter(|m| matches!(m.role, MessageRole::Tool)).count();
                // 当前正在流式写参数的工具调用（长文落盘期判据）：前端据此显示"正在生成内容 · N 字"
                let targs: usize = st.tca.iter().map(|(_, _, a)| a.len()).sum();
                let stool = st.tca.iter().rev()
                    .find(|(_, n, a)| !a.is_empty() && !n.is_empty())
                    .map(|(_, n, _)| n.clone())
                    .unwrap_or_default();
                let _ = h.emit("agent:status", json!({"stage":"writing","agent_id":overrides.agent_id.clone().unwrap_or_default(),"class":overrides.agent_class.clone().unwrap_or_default(),"chars":produced,"tools":tool_steps,"stool":stool,"targs":targs,"pending":inflight,"sid":sid_str.clone()}));
            }
            st.prog_last_chars = produced;
            st.prog_last_at = std::time::Instant::now();
        }
        if st.done { break; }
        if cancel_aware && overrides.cancel_sid.as_deref().map_or(false, crate::services::agent_workflow::session_cancelled) {
            fail!("cancelled".to_string(), true);
        }
    }
    // 流结束后处理剩余数据（续传缓冲冲刷 + 最后一行可能无换行结尾，补一个再解析）
    flush_pending_utf8(&mut st.buf, &mut st.pending);
    if !st.buf.is_empty() { st.buf.push('\n'); }
    while try_process_one(&mut st, &app_handle) {}
    // 流结束后 flush 残余 buffer
    flush_pending_tokens(&mut st, to_ui, !silent, &owner, &sid_str, &app_handle);
    log::info!("SSE stream end +{}ms tokens={} len={} tool_calls={} done={} finish={} err={}",
        t0.elapsed().as_millis(), st.count, st.full.len(), st.tca.len(), st.done, st.got_finish, st.server_err.is_some());

    // ★ 输出达模型长度上限被截断：明确失败（此前被当正常结束 → 半截文档/工具参数）
    if st.finish.as_deref() == Some("length") {
        let has_tool_calls = !st.tca.is_empty() || st.suppress_tokens;
        let tool_args_chars = st.tca.iter().map(|(_, _, a)| a.len()).sum();
        log::error!("SSE 输出达模型长度上限被截断 (finish_reason=length) content={} tool_args={}", st.full.len(), tool_args_chars);
        return Err(StreamFail { message: "模型输出达到长度上限被截断（finish_reason=length）：请拆分为多次生成，或改用输出上限更大的模型".to_string(), partial: st.full.clone(), has_tool_calls, cancelled: false, tool_args_chars });
    }

    // ★ 完整性判定：服务端 error 事件 / 未收到完成标记 → 视为中断，不把半截内容伪装成功
    if let Some(e) = stream_end_error(st.done, st.got_finish, st.server_err.as_deref()) {
        let has_tool_calls = !st.tca.is_empty() || st.suppress_tokens;
        log::error!("SSE stream incomplete +{}ms: {}", t0.elapsed().as_millis(), e);
        return Err(StreamFail { message: e, partial: st.full.clone(), has_tool_calls, cancelled: false, tool_args_chars: st.tca.iter().map(|(_, _, a)| a.len()).sum() });
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
        // ★ 清洗为空但原文有正文且不含任何标记字面量 → 非工具残留（清洗误伤兜底）：返回原文，不进重试烧轮次
        if !content.trim().is_empty() && !crate::services::agent_workflow::schemas::has_tool_markers_literal(&content) {
            return Ok(content.trim().to_string());
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
    use super::{extract_sse_data, stream_end_error, chunk_error_message, build_resume_messages, push_utf8_incremental, flush_pending_utf8};
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

    /// 三字节汉字按 1+2 字节断包：修复前每半各产 U+FFFD，修复后拼回无损
    #[test]
    fn push_utf8_char_split_across_chunks_roundtrip() {
        let mut buf = String::new();
        let mut pending = Vec::new();
        let bytes = "文".as_bytes(); // E6 96 87
        assert_eq!(bytes.len(), 3);
        push_utf8_incremental(&mut buf, &mut pending, &bytes[..1]);
        push_utf8_incremental(&mut buf, &mut pending, &bytes[1..]);
        assert_eq!(buf, "文", "断包续传不得产生替换字符");
        assert!(pending.is_empty());
    }

    /// 混合内容按任意小块切分（含 ascii/中文/emoji 边界）：整体 round-trip
    #[test]
    fn push_utf8_mixed_small_chunks_roundtrip() {
        let mut buf = String::new();
        let mut pending = Vec::new();
        for part in ["数据清单 📦 表格：网络方案😀\n", "second 行 文本 文档", "🚀end"] {
            let raw = part.as_bytes();
            let mut off = 0;
            while off < raw.len() {
                let n = 7.min(raw.len() - off); // 质数步长制造各种断包位置
                push_utf8_incremental(&mut buf, &mut pending, &raw[off..off + n]);
                off += n;
            }
        }
        flush_pending_utf8(&mut buf, &mut pending);
        assert_eq!(buf, "数据清单 📦 表格：网络方案😀\nsecond 行 文本 文档🚀end");
    }

    /// 真坏字节（不可能是任何字符前缀）：与 from_utf8_lossy 语义等价（各产一个 U+FFFD）
    #[test]
    fn push_utf8_genuinely_invalid_bytes_matches_lossy() {
        let mut buf = String::new();
        let mut pending = Vec::new();
        push_utf8_incremental(&mut buf, &mut pending, b"ok\xFF\xFEgood");
        flush_pending_utf8(&mut buf, &mut pending);
        assert_eq!(buf, String::from_utf8_lossy(b"ok\xFF\xFEgood"), "坏字节处理必须与 lossy 对齐");
    }

    /// 截断流尾：pending 残留的半字符按 lossy 冲刷（与旧行为对齐，不静默吞掉）
    #[test]
    fn flush_pending_utf8_tail_is_lossy() {
        let mut buf = String::new();
        let mut pending = Vec::new();
        push_utf8_incremental(&mut buf, &mut pending, "好".as_bytes());
        push_utf8_incremental(&mut buf, &mut pending, &[0xE6]); // 断流的半字符
        flush_pending_utf8(&mut buf, &mut pending);
        assert!(buf.starts_with('好'));
        assert!(buf.contains('\u{FFFD}'));
        assert!(pending.is_empty());
    }
}
