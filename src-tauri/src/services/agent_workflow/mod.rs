//! Native Tool Calling Agent — ReAct loop + symmetric retrieval + structured context lookup

use crate::database;
use crate::services::search_assets;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, RwLock};
use std::time::Instant;

pub(crate) mod prompt;
pub(crate) mod schemas;
pub(crate) mod session;
pub(crate) mod llm;
pub(crate) mod tools;

pub(crate) use prompt::*;
pub(crate) use schemas::*;
pub(crate) use session::*;
pub(crate) use llm::*;
pub(crate) use tools::*;
pub(crate) mod workflow;
pub(crate) use workflow::*;

/// 在已有的 Tokio runtime 中阻塞执行异步函数，避免创建新 runtime 导致 panic
pub(crate) fn block_on_async<F: std::future::Future>(f: F) -> F::Output {
    let handle = tokio::runtime::Handle::current();
    tokio::task::block_in_place(|| handle.block_on(f))
}
use tauri::Emitter;
use futures::StreamExt;

pub(crate) const API_TIMEOUT_SECS: u64 = 30;

/// 工具返回的统一信封结构
#[derive(Serialize, Deserialize, Clone)]
pub struct ToolResultEnvelope {
    /// 工具结果类型，如 "knowledge_search_result"、"object_detail" 等
    pub result_type: String,
    /// 结构化数据
    pub data: Value,
    /// 可选的 LLM 可读摘要
    pub summary: Option<String>,
}

impl ToolResultEnvelope {
    /// 序列化为 JSON 字符串
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| format!(r#"{{"result_type":"{}","data":{{}},"summary":"序列化失败"}}"#, self.result_type))
    }
}

/// LLM 最终回答的类型
#[allow(dead_code)]
#[derive(Serialize, Deserialize, Clone)]
pub enum ResponseType {
    /// 普通回答（Markdown）
    #[serde(rename = "answer")]
    Answer,
    /// 对象引用（包含对象ID）
    #[serde(rename = "object_reference")]
    ObjectReference,
}

// ★ Agent 停止信号
pub(crate) static AGENT_CANCEL: AtomicBool = AtomicBool::new(false);

/// 取消当前 Agent 循环
pub fn cancel_agent() {
    AGENT_CANCEL.store(true, Ordering::Release);
}

// Data structures
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)] #[serde(rename_all = "lowercase")]
pub enum MessageRole { System, User, Assistant, Tool }
#[derive(Debug, Clone, Serialize, Deserialize)] pub struct ToolCall { pub id: String, #[serde(rename="type")] pub call_type: String, pub function: FunctionCall }
#[derive(Debug, Clone, Serialize, Deserialize)] pub struct FunctionCall { pub name: String, pub arguments: String }
#[derive(Debug, Clone, Serialize, Deserialize)] pub struct ChatMessage { pub role: MessageRole, #[serde(skip_serializing_if="Option::is_none")] pub content: Option<String>, #[serde(skip_serializing_if="Option::is_none")] pub tool_calls: Option<Vec<ToolCall>>, #[serde(skip_serializing_if="Option::is_none")] pub tool_call_id: Option<String>, #[serde(skip_serializing_if="Option::is_none")] pub name: Option<String>, #[serde(skip_serializing_if="Option::is_none")] pub reasoning_content: Option<String> }

static API_CACHE: Lazy<RwLock<(String,String,String,Instant)>> = Lazy::new(|| RwLock::new((String::new(),String::new(),String::new(),Instant::now()-std::time::Duration::from_secs(60))));
static AGENT_HTTP_CLIENT: Lazy<reqwest::Client> = Lazy::new(|| reqwest::Client::builder().timeout(std::time::Duration::from_secs(API_TIMEOUT_SECS)).build().expect("agent http"));

pub(crate) fn get_api_endpoint() -> (String, String, String) {
    { let (url,key,model,at) = &*API_CACHE.read().unwrap_or_else(|e| e.into_inner()); if at.elapsed().as_secs()<30 && !url.is_empty() { return (url.clone(),key.clone(),model.clone()); } }
    let config = crate::services::chat_api::get_chat_config();
    let url = config.base_url.trim_end_matches('/').to_string();
    let (url, key, model) = (url, config.api_key, config.model);
    let mut c = API_CACHE.write().unwrap_or_else(|e| e.into_inner()); *c = (url.clone(), key.clone(), model.clone(), Instant::now());
    (url, key, model)
}

// ═══════════════════════════════════════════════════════════════════

/// 中文日期时间格式："YYYY-MM-DD 周X HH:MM"（注入 LLM 的所有时间戳统一用此格式，保证 prompt cache 前缀一致）
pub(crate) fn cn_datetime_str(nd: chrono::NaiveDateTime) -> String {
    const WEEKDAYS: [&str; 7] = ["周一", "周二", "周三", "周四", "周五", "周六", "周日"];
    let wd = chrono::Datelike::weekday(&nd).num_days_from_monday() as usize;
    format!("{} {} {}", nd.format("%Y-%m-%d"), WEEKDAYS[wd], nd.format("%H:%M"))
}

/// 当前本地时间的中文格式（同上）
pub(crate) fn cn_now_str() -> String {
    cn_datetime_str(chrono::Local::now().naive_local())
}

pub(crate) fn relative_time(created_at: &str) -> String {
    if created_at.is_empty() { return String::new(); }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let ts = chrono::NaiveDateTime::parse_from_str(created_at, "%Y-%m-%d %H:%M:%S").map(|dt| dt.and_utc().timestamp()).unwrap_or(0);
    let delta = (now - ts).max(0);
    let rel = if delta < 60 { "刚刚".to_string() } else if delta < 3600 { format!("{}分钟前", delta / 60) } else if delta < 86400 { format!("{}小时前", delta / 3600) } else if delta < 2592000 { format!("{}天前", delta / 86400) } else if delta < 77760000 { format!("{}个月前", delta / 2592000) } else { format!("{}年前", delta / 31104000) };
    format!("{} ({})", rel, &created_at[..created_at.len().min(16)])
}

// ═══════════════════════════════════════════════════════════════════
// Main Agent Loop
// ═══════════════════════════════════════════════════════════════════

