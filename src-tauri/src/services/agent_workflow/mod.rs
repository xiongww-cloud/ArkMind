//! Native Tool Calling Agent — ReAct loop + symmetric retrieval + structured context lookup

use crate::database;
use crate::services::search_assets;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use std::sync::{Mutex, RwLock};
use std::time::Instant;

pub(crate) mod prompt;
pub(crate) mod schemas;
pub(crate) mod session;
pub(crate) mod plan;
pub(crate) mod llm;
pub(crate) mod output_class;
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

// ★ Agent 停止信号：按会话记账（并行安全）——停止一个会话不影响其他会话在跑的 run。
// 空串作"无会话归属"运行体的兜底 key。
pub(crate) static AGENT_CANCELLED: Lazy<std::sync::Mutex<std::collections::HashSet<String>>> =
    Lazy::new(|| std::sync::Mutex::new(std::collections::HashSet::new()));

pub fn cancel_session(sid: &str) {
    if let Ok(mut s) = AGENT_CANCELLED.lock() { s.insert(sid.to_string()); }
}
pub fn session_cancelled(sid: &str) -> bool {
    AGENT_CANCELLED.lock().map(|s| s.contains(sid)).unwrap_or(false)
}
pub fn clear_session_cancel(sid: &str) {
    if let Ok(mut s) = AGENT_CANCELLED.lock() { s.remove(sid); }
}

/// 取消当前 Agent 循环（旧接口兼容：按会话取消）
pub fn cancel_agent(session_id: &str) {
    cancel_session(if session_id.is_empty() { "" } else { session_id });
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

// ═══════════════════════════════════════════════════════════════════
// Main Agent Loop
// ═══════════════════════════════════════════════════════════════════

