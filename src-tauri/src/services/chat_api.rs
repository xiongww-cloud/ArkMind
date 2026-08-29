//! Chat API client — OpenAI-compatible
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::RwLock;
use std::time::Instant;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatConfig { pub max_tokens: i32, pub temperature: f64, pub base_url: String, pub api_key: String, pub model: String }
impl Default for ChatConfig { fn default() -> Self { Self { max_tokens: 4096, temperature: 0.0, base_url: "https://api.openai.com/v1".into(), api_key: String::new(), model: "gpt-4o-mini".into() } } }

static CONFIG_CACHE: Lazy<RwLock<(ChatConfig, Instant)>> = Lazy::new(|| RwLock::new((ChatConfig::default(), Instant::now() - std::time::Duration::from_secs(60))));
static HTTP_CLIENT: Lazy<reqwest::Client> = Lazy::new(|| reqwest::Client::builder().timeout(std::time::Duration::from_secs(120)).build().expect("chat client"));

pub fn get_chat_config() -> ChatConfig {
    { let (cfg, at) = &*CONFIG_CACHE.read().unwrap_or_else(|e| e.into_inner()); if at.elapsed().as_secs() < 30 { return cfg.clone(); } }
    let mut config = ChatConfig::default();

    // ★ 优先从 system_config.ai_config 读取（新版设置页面）
    //    结构: { providers: [{id, apiKey, baseUrl, modelName, ...}], activeProviderId, embedding: {...}, vision: {...} }
    if let Ok(Some(row)) = crate::database::get("SELECT ai_config FROM system_config WHERE tenant_id='local-user'", &[]) {
        if let Some(s) = row["ai_config"].as_str() {
            if let Ok(v) = serde_json::from_str::<Value>(s) {
                let machine = crate::license::get_machine_hash();
                let v = crate::crypto::decrypt_config_api_keys(&v, &machine);
                // 根据 activeProviderId 找到当前激活的 provider
                let active_id = v.get("activeProviderId").and_then(|v|v.as_str());
                if let Some(aid) = active_id {
                    if let Some(providers) = v.get("providers").and_then(|v|v.as_array()) {
                        for p in providers {
                            if p.get("id").and_then(|v|v.as_str()) == Some(aid) {
                                if let Some(u) = p.get("baseUrl").and_then(|v|v.as_str()) { config.base_url = u.to_string(); }
                                if let Some(k) = p.get("apiKey").and_then(|v|v.as_str()) { config.api_key = k.to_string(); }
                                if let Some(m) = p.get("modelName").and_then(|v|v.as_str()) { config.model = m.to_string(); }
                                break;
                            }
                        }
                    }
                }
                // 如果没找到 active provider，fallback 到 embedding 的 baseUrl
                if config.api_key.is_empty() {
                    if let Some(e) = v.get("embedding").and_then(|v|v.as_object()) {
                        if let Some(u) = e.get("baseUrl").and_then(|v|v.as_str()) { config.base_url = u.to_string(); }
                        if let Some(k) = e.get("apiKey").and_then(|v|v.as_str()) { config.api_key = k.to_string(); }
                    }
                }
            }
        }
    }

    // ★ 回退：从 system_config.kb_config 读取（旧路径）
    if config.api_key.is_empty() {
        if let Ok(Some(row)) = crate::database::get("SELECT kb_config FROM system_config WHERE tenant_id='local-user'", &[]) {
            if let Some(s) = row["kb_config"].as_str() {
                if let Ok(v) = serde_json::from_str::<Value>(s) {
                    if let Some(u) = v.get("chatApiUrl").and_then(|v|v.as_str()) { config.base_url = u.to_string(); }
                    if let Some(k) = v.get("chatApiKey").and_then(|v|v.as_str()) { config.api_key = k.to_string(); }
                    if let Some(m) = v.get("chatModelName").and_then(|v|v.as_str()) { config.model = m.to_string(); }
                }
            }
        }
    }

    // ★ DB 读取失败时保留上次有效配置，不回退到 OpenAI 默认值
    if config.api_key.is_empty() {
        let (ref old, _) = *CONFIG_CACHE.read().unwrap_or_else(|e| e.into_inner());
        if !old.api_key.is_empty() { return old.clone(); }
    }

    let mut c = CONFIG_CACHE.write().unwrap_or_else(|e| e.into_inner()); *c = (config.clone(), Instant::now()); config
}

pub fn invalidate_chat_config_cache() { if let Ok(mut c) = CONFIG_CACHE.write() { c.1 = Instant::now() - std::time::Duration::from_secs(60); } }

/// 从推理模型的 reasoning_content 中提取 JSON（处理 deepseek-r1/v4 等推理模型）
fn extract_json_from_reasoning(reasoning: &str) -> String {
    // 找第一个 { 和最后一个 }
    if let Some(start) = reasoning.find('{') {
        if let Some(end) = reasoning.rfind('}') {
            if end > start {
                let candidate = &reasoning[start..=end];
                // 验证是合法 JSON 且包含 slides 数组
                if let Ok(v) = serde_json::from_str::<Value>(candidate) {
                    if v.get("slides").and_then(|s| s.as_array()).is_some() {
                        return candidate.to_string();
                    }
                }
            }
        }
    }
    // 降级：找 [ 开头的数组格式
    if let Some(start) = reasoning.find('[') {
        if let Some(end) = reasoning.rfind(']') {
            if end > start {
                let candidate = &reasoning[start..=end];
                if serde_json::from_str::<Value>(candidate).is_ok() {
                    return format!("{{\"slides\":{}}}", candidate);
                }
            }
        }
    }
    String::new()
}

/// 核心 LLM 调用（支持 max_tokens 覆盖 + 推理模型兜底）
async fn chat_completion_inner(system: &str, user: &str, cfg: Option<ChatConfig>, max_tokens_override: Option<i32>) -> Result<String, String> {
    let mut config = cfg.unwrap_or_else(get_chat_config);
    if let Some(mt) = max_tokens_override { config.max_tokens = mt; }
    let url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));
    let mut req = HTTP_CLIENT.post(&url).json(&json!({"model": config.model, "max_tokens": config.max_tokens, "temperature": config.temperature, "messages": [{"role":"system","content":system}, {"role":"user","content":user}]}));
    if !config.api_key.is_empty() { req = req.header("Authorization", format!("Bearer {}", config.api_key)); }
    let resp = req.send().await.map_err(|e| format!("chat error: {}", e))?;
    let st = resp.status();
    if !st.is_success() { let b = resp.text().await.unwrap_or_default(); return Err(format!("chat HTTP {}: {}", st, b)); }
    let body: Value = resp.json().await.map_err(|e| format!("chat json: {}", e))?;
    let content = body["choices"][0]["message"]["content"].as_str().unwrap_or("");
    let reasoning = body["choices"][0]["message"]["reasoning_content"].as_str().unwrap_or("");
    let finish_reason = body["choices"][0]["finish_reason"].as_str().unwrap_or("unknown");

    // 如果 content 为空但 reasoning 非空，从 reasoning 中提取 JSON（推理模型兜底）
    if content.is_empty() && !reasoning.is_empty() {
        let extracted = extract_json_from_reasoning(reasoning);
        if !extracted.is_empty() {
            log::info!("[chat_completion] 从 reasoning_content 提取 JSON ({} chars), finish_reason={}", extracted.len(), finish_reason);
            return Ok(extracted);
        }
        log::warn!("[chat_completion] reasoning_content 非空但无法提取 JSON, finish_reason={}, reasoning_preview={}", finish_reason, &reasoning[..reasoning.len().min(200)]);
    }

    Ok(content.to_string())
}

/// 公开接口：chat_completion（保持向后兼容）
pub async fn chat_completion(system: &str, user: &str, cfg: Option<ChatConfig>) -> Result<String, String> {
    chat_completion_inner(system, user, cfg, None).await
}


