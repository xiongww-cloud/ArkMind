//! Embedding API client — OpenAI-compatible
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::RwLock;
use std::time::Instant;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingConfig { pub base_url: String, pub api_key: String, pub model: String, pub dimensions: usize, pub timeout_secs: u64 }
impl Default for EmbeddingConfig { fn default() -> Self { Self { base_url: "https://api.openai.com".into(), api_key: String::new(), model: "text-embedding-3-small".into(), dimensions: 1024, timeout_secs: 30 } } }

struct ConfigCache { config: EmbeddingConfig, fetched_at: Instant }
static CONFIG_CACHE: Lazy<RwLock<ConfigCache>> = Lazy::new(|| RwLock::new(ConfigCache { config: EmbeddingConfig::default(), fetched_at: Instant::now() - std::time::Duration::from_secs(60) }));
static SHARED_CLIENT: Lazy<reqwest::Client> = Lazy::new(|| reqwest::Client::builder().timeout(std::time::Duration::from_secs(30)).build().expect("embed client"));

// Embedding 结果 LRU 缓存：最近 N 条文本的向量
const EMBED_CACHE_MAX: usize = 64;
static EMBED_CACHE: Lazy<RwLock<EmbedCache>> = Lazy::new(|| RwLock::new(EmbedCache { map: std::collections::HashMap::new(), order: VecDeque::new() }));
struct EmbedCache {
    map: std::collections::HashMap<String, Vec<f32>>,
    order: VecDeque<String>,
}

pub fn get_embedding_config() -> EmbeddingConfig {
    { let cache = CONFIG_CACHE.read().unwrap_or_else(|e| e.into_inner()); if cache.fetched_at.elapsed().as_secs() < 30 { return cache.config.clone(); } }
    let mut config = EmbeddingConfig::default();
    let mut db_ok = false;

    // ★ 优先从 system_config.ai_config.embedding 读取（新版设置页面路径）
    if let Ok(Some(row)) = crate::database::get("SELECT ai_config FROM system_config WHERE tenant_id='local-user'", &[]) {
        if let Some(s) = row["ai_config"].as_str() {
            if let Ok(v) = serde_json::from_str::<Value>(s) {
                let machine = crate::license::get_machine_hash();
                let v = crate::crypto::decrypt_config_api_keys(&v, &machine);
                if let Some(e) = v.get("embedding") {
                    if let Some(u) = e.get("baseUrl").and_then(|v|v.as_str()) { config.base_url = u.to_string(); }
                    if let Some(k) = e.get("apiKey").and_then(|v|v.as_str()) { config.api_key = k.to_string(); }
                    if let Some(m) = e.get("modelName").and_then(|v|v.as_str()) { config.model = m.to_string(); }
                    db_ok = true;
                }
            }
        }
    }

    // ★ 回退：从 system_config.kb_config 读取（旧路径）
    if config.api_key.is_empty() {
        let row = crate::database::get("SELECT kb_config FROM system_config WHERE tenant_id='local-user'", &[]).ok().flatten();
        if let Some(r) = row { if let Some(s) = r["kb_config"].as_str() { if let Ok(v) = serde_json::from_str::<Value>(s) { if let Some(u) = v.get("embeddingApiUrl").and_then(|v|v.as_str()) { config.base_url = u.to_string(); } if let Some(k) = v.get("embeddingApiKey").and_then(|v|v.as_str()) { config.api_key = k.to_string(); } if let Some(m) = v.get("embeddingModelName").and_then(|v|v.as_str()) { config.model = m.to_string(); } db_ok = true; } } }
    }

    // ★ DB 读取失败时保留上次有效配置，不回退到 OpenAI 默认值
    if !db_ok {
        let cache = CONFIG_CACHE.read().unwrap_or_else(|e| e.into_inner());
        if !cache.config.api_key.is_empty() { return cache.config.clone(); }
    }

    let mut cache = CONFIG_CACHE.write().unwrap_or_else(|e| e.into_inner()); cache.config = config.clone(); cache.fetched_at = Instant::now();
    config
}

pub fn invalidate_embedding_config_cache() { if let Ok(mut c) = CONFIG_CACHE.write() { c.fetched_at = Instant::now() - std::time::Duration::from_secs(60); } }

pub fn get_embedding_cached(text: &str) -> Option<Vec<f32>> {
    let cache = EMBED_CACHE.read().ok()?;
    cache.map.get(text).cloned()
}

fn set_embedding_cache(text: &str, vec: Vec<f32>) {
    if let Ok(mut cache) = EMBED_CACHE.write() {
        if !cache.map.contains_key(text) {
            cache.map.insert(text.to_string(), vec);
            cache.order.push_back(text.to_string());
            while cache.order.len() > EMBED_CACHE_MAX {
                if let Some(old) = cache.order.pop_front() {
                    cache.map.remove(&old);
                }
            }
        }
    }
}

pub async fn get_embedding_with_caller(text: &str, caller: &str) -> Result<Vec<f32>, String> {
    if let Some(cached) = get_embedding_cached(text) { return Ok(cached); }
    let v = embed_texts_with_caller(&[text.to_string()], caller).await.map(|mut v| v.pop().unwrap_or_default());
    if let Ok(ref vec) = v { set_embedding_cache(text, vec.clone()); }
    v
}

pub async fn embed_texts_with_caller(texts: &[String], caller: &str) -> Result<Vec<Vec<f32>>, String> {
    if texts.is_empty() { return Ok(Vec::new()); }
    let config = get_embedding_config();
    let base = config.base_url.trim_end_matches('/');
    // 智能拼接：如果 URL 已经包含 /v1/ 或 /embeddings，直接使用；否则根据情况拼接
    let url = if base.contains("/v1/") || base.ends_with("/embeddings") { 
        base.to_string()
    } else if base.ends_with("/v1") { 
        format!("{}/embeddings", base) 
    } else { 
        format!("{}/v1/embeddings", base) 
    };
    eprintln!("[嵌入] POST {} 模型={} 数量={} 调用方={}", url, config.model, texts.len(), caller);
    let mut req = SHARED_CLIENT.post(&url).json(&json!({"model": config.model, "input": texts}));
    if !config.api_key.is_empty() { req = req.header("Authorization", format!("Bearer {}", config.api_key)); }
    let resp = req.send().await.map_err(|e| format!("embed error: {}", e))?;
    let st = resp.status();
    if !st.is_success() { let b = resp.text().await.unwrap_or_default(); return Err(format!("embed HTTP {}: {}", st, b)); }
    let body: Value = resp.json().await.map_err(|e| format!("embed json: {}", e))?;
    let data = body["data"].as_array().ok_or("no data")?;
    let vecs: Vec<Vec<f32>> = data.iter().filter_map(|d| d["embedding"].as_array().map(|a| a.iter().filter_map(|v| v.as_f64().map(|f| f as f32)).collect())).collect();
    Ok(vecs)
}
