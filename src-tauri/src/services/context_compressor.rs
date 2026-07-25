//! Context Compressor — 上下文压缩器
//! 在 agent_chat_workflow 入口处检查历史长度，超过阈值时压缩旧消息为摘要
//! 摘要直接替换 chat_messages 中的旧消息，保证 prompt cache 前缀稳定

const MAX_CONTEXT_TOKENS: usize = 200_000;
const COMPRESS_TRIGGER_RATIO: f64 = 0.8;
const KEEP_RECENT_MESSAGES: usize = 10;

/// 估算文本的 token 数量
pub fn estimate_tokens(text: &str) -> usize {
    let chinese_chars = text.chars()
        .filter(|c| *c >= '\u{4e00}' && *c <= '\u{9fff}')
        .count();
    let other_chars = text.len().saturating_sub(chinese_chars);
    chinese_chars * 2 + other_chars / 4
}

/// 估算历史消息的总 token 数
pub fn estimate_history_tokens(history: &[serde_json::Value]) -> usize {
    history.iter()
        .map(|m| {
            let content = m["content"].as_str().unwrap_or("");
            estimate_tokens(content)
        })
        .sum()
}

/// 检查是否需要压缩历史
pub fn should_compress_history(history: &[serde_json::Value]) -> bool {
    let total_tokens = estimate_history_tokens(history);
    let threshold = (MAX_CONTEXT_TOKENS as f64 * COMPRESS_TRIGGER_RATIO) as usize;
    total_tokens > threshold
}

/// 压缩历史并持久化到 DB，返回压缩后的历史
/// 1. 从 DB 加载所有消息
/// 2. 分离旧消息和保留的最近消息
/// 3. 调用 LLM 生成摘要
/// 4. 删除旧消息，插入摘要
/// 5. 返回 [摘要消息] + 保留消息
pub async fn compress_and_persist(session_id: &str) -> Result<Vec<serde_json::Value>, String> {
    // 1. 从 DB 加载所有消息（带 id）
    let rows = crate::database::all(
        "SELECT id, role, content FROM chat_messages WHERE session_id=?1 ORDER BY id",
        &[serde_json::json!(session_id)],
    ).map_err(|e| format!("加载消息失败: {}", e))?;

    if rows.len() <= KEEP_RECENT_MESSAGES {
        return Ok(rows.iter().map(|r| {
            serde_json::json!({"role": r["role"], "content": r["content"]})
        }).collect());
    }

    // 2. 分离：旧消息（需压缩）和保留的最近消息
    let split_at = rows.len() - KEEP_RECENT_MESSAGES;
    let old_rows = &rows[..split_at];
    let recent_rows = &rows[split_at..];

    // 3. 生成摘要
    let old_messages: Vec<serde_json::Value> = old_rows.iter().map(|r| {
        serde_json::json!({"role": r["role"], "content": r["content"]})
    }).collect();

    log::info!("[ContextCompressor] 开始压缩 {} 条旧消息（session: {}）", old_rows.len(), session_id);
    let summary = generate_summary(&old_messages).await?;
    if summary.is_empty() {
        log::warn!("[ContextCompressor] 摘要为空，跳过压缩");
        return Ok(rows.iter().map(|r| {
            serde_json::json!({"role": r["role"], "content": r["content"]})
        }).collect());
    }

    // 4. 删除旧消息
    let old_ids: Vec<i64> = old_rows.iter()
        .filter_map(|r| r["id"].as_i64())
        .collect();
    if !old_ids.is_empty() {
        let placeholders: Vec<String> = old_ids.iter().enumerate()
            .map(|(i, _)| format!("?{}", i + 1))
            .collect();
        let sql = format!(
            "DELETE FROM chat_messages WHERE session_id=? AND id IN ({})",
            placeholders.join(",")
        );
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(session_id.to_string())];
        for id in &old_ids {
            params.push(Box::new(*id));
        }
        let params_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        crate::database::with_db(|conn| {
            conn.execute(&sql, rusqlite::params_from_iter(params_refs.iter()))
        }).map_err(|e| format!("删除旧消息失败: {}", e))?;
    }

    // 5. 插入摘要消息
    let summary_content = format!("[HISTORY_SUMMARY]\n{}", summary);
    crate::database::run(
        "INSERT INTO chat_messages(session_id, role, content) VALUES(?1, 'assistant', ?2)",
        &[serde_json::json!(session_id), serde_json::json!(summary_content)],
    ).map_err(|e| format!("插入摘要失败: {}", e))?;

    log::info!("[ContextCompressor] 压缩完成: {} 条旧消息 → 1 条摘要", old_rows.len());

    // 6. 返回新历史 = [摘要] + 保留消息
    let mut result = vec![serde_json::json!({"role": "assistant", "content": summary_content})];
    for r in recent_rows {
        result.push(serde_json::json!({"role": r["role"], "content": r["content"]}));
    }
    Ok(result)
}

/// 调用 LLM 生成历史摘要
async fn generate_summary(messages: &[serde_json::Value]) -> Result<String, String> {
    if messages.is_empty() {
        return Ok(String::new());
    }

    // 检查是否有已有的摘要（可能包含 [HISTORY_SUMMARY] 前缀）
    let has_existing_summary = messages.iter().any(|m| {
        m["content"].as_str().unwrap_or("").starts_with("[HISTORY_SUMMARY]")
    });

    let history_text: Vec<String> = messages.iter()
        .filter_map(|m| {
            let role = m["role"].as_str()?;
            let content = m["content"].as_str()?;
            let truncated = if content.len() > 500 {
                format!("{}...", &content[..500])
            } else {
                content.to_string()
            };
            Some(format!("{}: {}", role, truncated))
        })
        .collect();

    let summary_hint = if has_existing_summary {
        "注意：对话历史中可能包含之前的摘要（以 [HISTORY_SUMMARY] 开头），请将其内容也纳入本次压缩。\n"
    } else {
        ""
    };

    let prompt = format!(
        r#"请将以下对话历史压缩为简洁的摘要，保留关键信息：
{hint}1. 用户的主要问题和需求
2. AI 的重要回答和结论
3. 提到的关键数据（项目名、产品名、数字等）
4. 未完成的任务或待跟进事项

注意：
- 摘要应简洁，不超过 2000 字
- 保留具体的数字、名称、日期等关键信息
- 用中文输出

对话历史：
{history}

请输出压缩后的摘要："#,
        hint = summary_hint,
        history = history_text.join("\n")
    );

    let config = crate::services::chat_api::get_chat_config();
    if config.api_key.is_empty() {
        return Ok(String::new());
    }

    let url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

    let mut req = client.post(&url).json(&serde_json::json!({
        "model": config.model,
        "max_tokens": 2048,
        "temperature": 0.0,
        "messages": [
            {"role": "system", "content": "你是一个对话历史压缩助手。请将对话历史压缩为简洁的摘要，保留关键信息。"},
            {"role": "user", "content": prompt}
        ]
    }));

    if !config.api_key.is_empty() {
        req = req.header("Authorization", format!("Bearer {}", config.api_key));
    }

    let resp = req.send().await.map_err(|e| format!("LLM 请求失败: {}", e))?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("LLM API 错误 {}: {}", status, body));
    }

    let body: serde_json::Value = resp.json().await.map_err(|e| format!("解析响应失败: {}", e))?;
    let content = body["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .to_string();

    Ok(content)
}
