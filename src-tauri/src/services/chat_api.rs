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
    { let (cfg, at) = &*CONFIG_CACHE.read().unwrap(); if at.elapsed().as_secs() < 30 { return cfg.clone(); } }
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

    let mut c = CONFIG_CACHE.write().unwrap(); *c = (config.clone(), Instant::now()); config
}

pub fn invalidate_chat_config_cache() { if let Ok(mut c) = CONFIG_CACHE.write() { c.1 = Instant::now() - std::time::Duration::from_secs(60); } }

pub async fn chat_completion(system: &str, user: &str, cfg: Option<ChatConfig>) -> Result<String, String> {
    let config = cfg.unwrap_or_else(get_chat_config);
    let url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));
    let mut req = HTTP_CLIENT.post(&url).json(&json!({"model": config.model, "max_tokens": config.max_tokens, "temperature": config.temperature, "messages": [{"role":"system","content":system}, {"role":"user","content":user}]}));
    if !config.api_key.is_empty() { req = req.header("Authorization", format!("Bearer {}", config.api_key)); }
    let resp = req.send().await.map_err(|e| format!("chat error: {}", e))?;
    let st = resp.status();
    if !st.is_success() { let b = resp.text().await.unwrap_or_default(); return Err(format!("chat HTTP {}: {}", st, b)); }
    let body: Value = resp.json().await.map_err(|e| format!("chat json: {}", e))?;
    Ok(body["choices"][0]["message"]["content"].as_str().unwrap_or("").to_string())
}

pub async fn chat_completion_stream(messages: &[Value], on_token: &(dyn Fn(&str) + Send + Sync)) -> Result<String, String> {
    let config = get_chat_config();
    let url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));
    let mut req = HTTP_CLIENT.post(&url).json(&json!({"model": config.model, "max_tokens": config.max_tokens, "temperature": 0.3, "stream": true, "messages": messages}));
    if !config.api_key.is_empty() { req = req.header("Authorization", format!("Bearer {}", config.api_key)); }
    let resp = req.send().await.map_err(|e| format!("stream chat error: {}", e))?;
    let st = resp.status();
    if !st.is_success() { let b = resp.text().await.unwrap_or_default(); return Err(format!("stream chat HTTP {}: {}", st, b)); }
    let mut full = String::new();
    let mut stream = resp.bytes_stream();
    use futures::StreamExt;
    let mut buffer = String::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("stream read: {}", e))?;
        let text = String::from_utf8_lossy(&chunk);
        buffer.push_str(&text);
        // 按行处理 SSE
        loop {
            let line_end = match buffer.find('\n') { Some(n) => n, None => { break } };
            let line = buffer[..line_end].trim().to_string();
            buffer = buffer[line_end + 1..].to_string();
            if line.is_empty() || line.starts_with(':') { continue; }
            if line.starts_with("data: ") {
                let data = &line[6..];
                if data == "[DONE]" { break; }
                if let Ok(v) = serde_json::from_str::<Value>(data) {
                    if let Some(delta) = v["choices"][0]["delta"]["content"].as_str() {
                        full.push_str(delta);
                        on_token(delta);
                    }
                }
            }
        }
    }
    Ok(full)
}
