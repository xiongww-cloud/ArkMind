//! Context Compressor — 上下文压缩器
//! 复用会话上下文进行压缩，LLM 返回 JSON 格式摘要
//! 摘要保存到 chat_sessions.context_metadata（session 级别）
//! 旧消息的 metadata 清空，content 保留供前端展示

/// 压缩 prompt：要求 LLM 返回 JSON
const COMPRESS_PROMPT: &str = r#"请将以上对话历史压缩为 JSON 格式的摘要。严格按以下格式输出，不要输出其他内容：

{
  "summary": "详细的对话摘要（3000-5000字）",
  "key_points": ["关键发现1", "关键发现2", "..."],
  "decisions": ["决策1", "决策2", "..."],
  "unfinished": "未完成的事项（没有则为空字符串）"
}

要求：
- summary 必须是 3000-5000 字的详细摘要，包含：
  * 用户的主要问题和目标
  * AI 的回答和分析过程
  * 工具调用的关键结果和数据
  * 讨论中涉及的具体数字、日期、名称等细节
  * 对话的演进脉络和逻辑链条
- key_points 列出 5-10 个最重要的发现或结论
- decisions 列出对话中已做出的所有决策
- unfinished 记录所有待跟进事项
- 只输出 JSON，不要输出其他内容"#;

/// 压缩结果
pub struct CompressResult {
    pub summary: String,
    pub key_points: Vec<String>,
    pub decisions: Vec<String>,
    pub unfinished: String,
}

/// 从 LLM 返回的文本中提取 JSON（兼容 markdown code block 包裹的情况）
fn extract_json_from_response(text: &str) -> &str {
    let trimmed = text.trim();
    if let Some(start) = trimmed.find('{') {
        if let Some(end) = trimmed.rfind('}') {
            return &trimmed[start..=end];
        }
    }
    trimmed
}

/// 复用会话上下文进行压缩，返回压缩结果
/// messages: 当前会话的完整 messages 数组（会被临时修改，调用后恢复）
pub async fn compress_with_context(
    messages: &mut Vec<crate::services::agent_workflow::ChatMessage>,
    app_handle: &Option<tauri::AppHandle>,
) -> Result<CompressResult, String> {
    use crate::services::agent_workflow::{ChatMessage, MessageRole};

    if messages.len() <= 2 {
        return Err("消息太少，无法压缩".to_string());
    }

    // 1. 保存最后一条用户消息（当前查询）
    let original_last = messages.pop().unwrap_or_else(|| ChatMessage {
        role: MessageRole::User,
        content: Some(String::new()),
        tool_calls: None,
        tool_call_id: None,
        name: None,
        reasoning_content: None,
    });

    // 2. 添加压缩 prompt 作为最后一条用户消息
    messages.push(ChatMessage {
        role: MessageRole::User,
        content: Some(COMPRESS_PROMPT.to_string()),
        tool_calls: None,
        tool_call_id: None,
        name: None,
        reasoning_content: None,
    });

    // 3. 调用 LLM（复用会话上下文，命中缓存）
    log::info!("[ContextCompressor] 开始压缩，复用 {} 条消息的上下文", messages.len());
    let (ac, _reasoning, _tcs, _usage) = crate::services::agent_workflow::call_llm_streaming(
        messages, None, app_handle,
    ).await.map_err(|e| format!("LLM 调用失败: {}", e))?;

    // 4. 恢复最后一条用户消息
    messages.pop(); // 移除压缩 prompt
    messages.push(original_last); // 恢复原始查询

    // 5. 解析 JSON
    let json_str = extract_json_from_response(&ac);
    let parsed: serde_json::Value = serde_json::from_str(json_str)
        .map_err(|e| format!("JSON 解析失败: {}，原始响应: {}", e, &ac[..ac.len().min(200)]))?;

    let result = CompressResult {
        summary: parsed["summary"].as_str().unwrap_or("").to_string(),
        key_points: parsed["key_points"].as_array()
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
            .unwrap_or_default(),
        decisions: parsed["decisions"].as_array()
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
            .unwrap_or_default(),
        unfinished: parsed["unfinished"].as_str().unwrap_or("").to_string(),
    };

    log::info!("[ContextCompressor] 压缩完成: summary={}字, key_points={}, decisions={}",
        result.summary.len(), result.key_points.len(), result.decisions.len());

    Ok(result)
}

/// 保存压缩结果到 DB：
/// 1. 更新 chat_sessions.context_metadata（session 级别存储压缩摘要）
/// 2. 清空旧消息的 metadata（保留 content 供前端展示）
///
/// 参数:
/// - session_id: 会话 ID
/// - result: 压缩结果
/// - total_message_count: 当前会话的总消息数量（保留参数兼容，不再使用）
pub fn save_compressed_to_db(
    session_id: &str,
    result: &CompressResult,
    _total_message_count: usize,
) -> Result<(), String> {
    // 1. 标记当前所有 active 消息为 compressed（不删除，前端仍可展示）
    crate::database::run(
        "UPDATE chat_messages SET status='compressed' WHERE session_id=?1 AND status='active'",
        &[serde_json::json!(session_id)],
    ).map_err(|e| format!("标记消息为 compressed 失败: {}", e))?;

    // 2. 构建 session 级别的 context_metadata（只存摘要，不含 original_count）
    let context_metadata = serde_json::json!({
        "compressed": true,
        "summary": result.summary,
        "key_points": result.key_points,
        "decisions": result.decisions,
        "unfinished": result.unfinished
    }).to_string();

    // 3. 写入 chat_sessions.context_metadata
    crate::database::run(
        "UPDATE chat_sessions SET context_metadata=?1 WHERE id=?2",
        &[serde_json::json!(context_metadata), serde_json::json!(session_id)],
    ).map_err(|e| format!("更新 session context_metadata 失败: {}", e))?;

    log::info!("[ContextCompressor] 压缩保存完成: session={}, summary={}字",
        session_id, result.summary.len());

    Ok(())
}
