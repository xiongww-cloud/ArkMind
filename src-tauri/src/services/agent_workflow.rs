//! Native Tool Calling Agent — ReAct loop + symmetric retrieval + structured context lookup

use crate::database;
use crate::services::search_assets;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, RwLock};
use std::time::Instant;

/// 在已有的 Tokio runtime 中阻塞执行异步函数，避免创建新 runtime 导致 panic
fn block_on_async<F: std::future::Future>(f: F) -> F::Output {
    let handle = tokio::runtime::Handle::current();
    tokio::task::block_in_place(|| handle.block_on(f))
}
use tauri::Emitter;
use futures::StreamExt;

const API_TIMEOUT_SECS: u64 = 30;

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
static AGENT_CANCEL: AtomicBool = AtomicBool::new(false);

/// 取消当前 Agent 循环
pub fn cancel_agent() {
    AGENT_CANCEL.store(true, Ordering::Release);
}
const AGENT_SYSTEM_PROMPT: &str = r#"
## 1. 核心身份
你是用户的长期思维伙伴，是决策辅助Agent，不是搜索机器人。
你的职责不是查找信息，而是帮助用户理解问题、形成判断、推进决策。
记忆是你提炼的经验，知识库是原始事实。

## 2. 核心原则（事实优先级）
- Memory 提供背景和历史经验，不是事实来源
- 涉及产品参数、金额、规格、项目状态等事实时，必须以知识库或业务对象数据为准
- 记忆与知识库冲突时，以时间更近的来源为准

## 3. 上下文使用
当用户提问时：
1. 先回忆记忆，获取背景和经验
2. 再查知识库，获取事实依据
3. 信息足够时立即停止检索
⚠️ 每次工具调用都会消耗预算。已获取的对象内容禁止重复获取。

## 4. 工具调用协议（必须遵守）
- 所有工具调用必须通过 tool_calls 字段返回，不得在 content 中输出
- content 字段只能包含思考过程或最终回答
- 禁止在 content 中出现以下内容：
  - XML 标签：<tool_call>、<tool_calls>、<invoke> 等
  - 函数调用：ext_tool_search(...)、ext_tool_object(...) 等
  - JSON 格式：{"name": "ext_tool_xxx", ...}
  - 工具名称、tool_call_id 等工具调用元数据

## 5. 工具选择规则
- 搜索文档内容 → ext_tool_search
- 查看对象详情（已知 ID）→ ext_tool_object(mode="detail", id=[...])
- 关键词搜索对象目录 → ext_tool_object(mode="search", query="...")
- 条件筛选记录 → ext_tool_object(mode="filter", type=..., field_filters=[...])
- 统计数量 → ext_tool_object(mode="count", type=..., field_filters=[...])
- 分组统计 → ext_tool_object(mode="group", type=..., group_by="...")
- 聚合计算 → ext_tool_object(mode="aggregate", type=..., aggregate="SUM(...)")
- 查看对象关联 → ext_tool_relations
- 已知 ChunkID 翻页 → ext_tool_chunk_navigate
- 创建/修改/删除数据 → ext_tool_manage_*
- 记录记忆 → ext_tool_memory(action="create")
- 管理策略 → ext_tool_manage_policy
- 查看项目动态/日志 → ext_tool_object(type="PROJECT_DYNAMIC", field_filters=[{field:"project_id",op:"=",value:"..."}])，可加 created_at >= / <= 做时间范围过滤
- 追加项目动态 → ext_tool_manage_project(action="add_dynamic", project_id="...", content="...")
- 删除项目动态 → ext_tool_manage_project(action="delete_dynamic", dynamic_id="...")

## 6. 业务对象模型
项目结构图谱：
PROJECT
 └─ SOLUTION
     └─ SOLUTION_VERSION（边 metadata 含金额/状态）
         └─ SOLUTION_ITEM（边 metadata 含型号/数量/单价）
             └─ PRODUCT

- 探索方案结构：用 ext_tool_relations 沿图谱逐层导航
- 设备完整参数快照：ext_tool_object(type="SOLUTION_ITEM") 返回的 params 字段（该版本保存时的快照，与产品中心参数无关）
- 联系人必须归属于合作伙伴；创建联系人时先确认 partner_id；联系人关联项目时，所属合作伙伴必须同时关联到同一项目

## 7. 策略系统（Policy）
策略是用户设定的做事方法和行为约束，具有指令性和普适性——对所有对话生效（如"报价前必须先确认库存"）。
规则只能来自用户的输入，禁止在分析数据、回答问题时自行创建或修改策略。

## 8. 记忆系统（Memory）
回答前检查上下文中已检索到的记忆，如果有相关的记忆且内容一致，则跳过记录；仅在发现新信息时调用工具记录。格式为：
[问题] 我遇到什么问题或情况 / [动作] 查看了什么资料、执行了什么操作 / [结论] 结果和关键数据

- 一条记录只写一个记忆，可独立检索
- 记忆中不得出现"主人""老板"等称呼，一律用"用户"指代
- 新旧记忆冲突时，以时间更近的为准
- 已有记忆有更新时，更新旧记忆
"#;

// Data structures
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)] #[serde(rename_all = "lowercase")]
pub enum MessageRole { System, User, Assistant, Tool }
#[derive(Debug, Clone, Serialize, Deserialize)] pub struct ToolCall { pub id: String, #[serde(rename="type")] pub call_type: String, pub function: FunctionCall }
#[derive(Debug, Clone, Serialize, Deserialize)] pub struct FunctionCall { pub name: String, pub arguments: String }
#[derive(Debug, Clone, Serialize, Deserialize)] pub struct ChatMessage { pub role: MessageRole, #[serde(skip_serializing_if="Option::is_none")] pub content: Option<String>, #[serde(skip_serializing_if="Option::is_none")] pub tool_calls: Option<Vec<ToolCall>>, #[serde(skip_serializing_if="Option::is_none")] pub tool_call_id: Option<String>, #[serde(skip_serializing_if="Option::is_none")] pub name: Option<String>, #[serde(skip_serializing_if="Option::is_none")] pub reasoning_content: Option<String> }

static API_CACHE: Lazy<RwLock<(String,String,String,Instant)>> = Lazy::new(|| RwLock::new((String::new(),String::new(),String::new(),Instant::now()-std::time::Duration::from_secs(60))));
static AGENT_HTTP_CLIENT: Lazy<reqwest::Client> = Lazy::new(|| reqwest::Client::builder().timeout(std::time::Duration::from_secs(API_TIMEOUT_SECS)).build().expect("agent http"));

fn get_api_endpoint() -> (String, String, String) {
    { let (url,key,model,at) = &*API_CACHE.read().unwrap_or_else(|e| e.into_inner()); if at.elapsed().as_secs()<30 && !url.is_empty() { return (url.clone(),key.clone(),model.clone()); } }
    let config = crate::services::chat_api::get_chat_config();
    let url = config.base_url.trim_end_matches('/').to_string();
    let (url, key, model) = (url, config.api_key, config.model);
    let mut c = API_CACHE.write().unwrap_or_else(|e| e.into_inner()); *c = (url.clone(), key.clone(), model.clone(), Instant::now());
    (url, key, model)
}

// ═══════════════════════════════════════════════════════════════════
// Session Memory — 会话工作记忆层
// ═══════════════════════════════════════════════════════════════════
const ACCESS_STALE_THRESHOLD: usize = 20;   // 超过 20 次访问未命中即淘汰
const SCRATCHPAD_MAX_CHARS: usize = 4000;     // scratchpad 最长字符数

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ActiveObject {
    obj_type: String,
    obj_id: String,
    name: String,
    access_order: usize,
    hits: usize,
    score: f64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct Focus {
    entities: Vec<String>,
    #[allow(dead_code)]
    intent: String,
    open_questions: Vec<String>,
    focus_objects: Vec<String>,  // 当前焦点对象 ID（由 Goal 决定，不是 score）
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionMemory {
    active_objects: Vec<ActiveObject>,
    visited_chunks: std::collections::HashSet<String>,
    visited_sources: std::collections::HashSet<String>,
    shown_memory_ids: std::collections::HashSet<String>,  // MEMORY 跨轮次去重
    focus: Focus,
    observations: Vec<String>,
    hypotheses: Vec<String>,
    access_counter: usize,
    search_keywords: Vec<Vec<String>>,
    tool_history: Vec<String>,  // 工具调用历史（用于生成检索计划）
    prompt_tokens: i64,  // 累计 prompt tokens
    completion_tokens: i64,  // 累计 cached tokens
    cached_tokens: i64,  // 累计 cached tokens
    #[serde(skip)]
    prev_visited_chunks: std::collections::HashSet<String>,  // 本次提问开始时的快照（不持久化）
    #[serde(skip)]
    prev_visited_sources: std::collections::HashSet<String>,  // 同上
}

// ★ Session 状态缓存：从 chat_sessions.session_state 加载/保存 SessionMemory
pub fn load_session_state(session_id: &str) -> Option<SessionMemory> {
    let json_str = crate::database::get(
        "SELECT session_state FROM chat_sessions WHERE id=?1 AND session_state IS NOT NULL",
        &[json!(session_id)],
    ).ok().flatten()?.get("session_state")?.as_str()?.to_string();
    serde_json::from_str(&json_str).ok()
}

fn save_session_state(session_id: &str, session: &SessionMemory) {
    let mut to_save = session.clone();
    // 只保存本次提问新增的 visited，不累积历史（上下文中只有上一次的工具结果）
    for k in &session.prev_visited_chunks { to_save.visited_chunks.remove(k); }
    for k in &session.prev_visited_sources { to_save.visited_sources.remove(k); }
    // 快照字段不参与序列化（#[serde(skip)]），但 clone 后清空以防万一
    to_save.prev_visited_chunks.clear();
    to_save.prev_visited_sources.clear();
    let json_str = serde_json::to_string(&to_save).unwrap_or_default();
    let _ = crate::database::run(
        "UPDATE chat_sessions SET session_state=?1 WHERE id=?2",
        &[json!(json_str), json!(session_id)],
    );
}

impl SessionMemory {
    fn new() -> Self {
        Self {
            active_objects: Vec::new(),
            visited_chunks: std::collections::HashSet::new(),
            visited_sources: std::collections::HashSet::new(),
            shown_memory_ids: std::collections::HashSet::new(),
            focus: Focus { entities: Vec::new(), intent: String::new(), open_questions: Vec::new(), focus_objects: Vec::new() },
            observations: Vec::new(),
            hypotheses: Vec::new(),
            access_counter: 0,
            search_keywords: Vec::new(),
            tool_history: Vec::new(),
            prompt_tokens: 0,
            completion_tokens: 0,
            cached_tokens: 0,
            prev_visited_chunks: std::collections::HashSet::new(),
            prev_visited_sources: std::collections::HashSet::new(),
        }
    }

    /// 记录一个对象被访问/命中，并根据 score_delta 更新注意力分数，然后淘汰过期的
    fn update(&mut self, obj_type: &str, obj_id: &str, name: &str, score_delta: f64) {
        self.access_counter += 1;
        if let Some(existing) = self.active_objects.iter_mut().find(|o| o.obj_type == obj_type && o.obj_id == obj_id) {
            existing.name = name.to_string();
            existing.access_order = self.access_counter;
            existing.hits += 1;
            existing.score = (existing.score * 0.95 + score_delta).clamp(0.0, 1.0);
        } else {
            self.active_objects.push(ActiveObject {
                obj_type: obj_type.to_string(),
                obj_id: obj_id.to_string(),
                name: name.to_string(),
                access_order: self.access_counter,
                hits: 1,
                score: (0.3_f64 * 0.95 + score_delta).clamp(0.0, 1.0),
            });
        }
        self.evict_stale();
    }

    /// 从包含 source_id 的行解析并记录
    fn update_from_line(&mut self, _tool_name: &str, line: &str) {
        let tl = line.trim();
        if let Some(cid) = tl.strip_prefix("ChunkID：").map(|s| s.trim()) {
            self.mark_chunk_visited(cid);
        }
    }

    /// 根据工具名返回注意力分数增量（EMA 公式：new_score = old_score * 0.95 + delta）
    fn score_delta_for_tool(tool_name: &str) -> f64 {
        match tool_name {
            "ext_tool_search" | "ext_tool_batch_search" => 0.9,
            "ext_tool_object" => 0.8,
            "ext_tool_relations" => 0.6,
            "ext_tool_chunk_navigate" => 0.6,
            "ext_tool_manage_todo" | "ext_tool_manage_project" | "ext_tool_manage_partner" |
            "ext_tool_manage_contact" | "ext_tool_manage_notification" | "ext_tool_manage_policy" => 0.5,
            "ext_tool_memory" => 0.0,
            _ => 0.3,
        }
    }

    /// 从 JSON 信封格式更新会话状态
    fn update_from_envelope(&mut self, tool_name: &str, envelope: &ToolResultEnvelope) {
        let score_delta = Self::score_delta_for_tool(tool_name);
        match envelope.result_type.as_str() {
            "knowledge_search_result" => {
                // 提取 chunks 中的 chunk_id 和 source
                if let Some(chunks) = envelope.data["chunks"].as_array() {
                    for chunk in chunks {
                        if let Some(cid) = chunk["chunk_id"].as_str() {
                            self.mark_chunk_visited(cid);
                        }
                        if let Some(stype) = chunk["source"]["type"].as_str() {
                            if let Some(sid) = chunk["source"]["id"].as_str() {
                                let name = chunk["source"]["name"].as_str().unwrap_or("");
                                self.update(stype, sid, name, score_delta);
                            }
                        }
                    }
                }
                // 提取 related_objects
                if let Some(objects) = envelope.data["related_objects"].as_array() {
                    for obj in objects {
                        if let Some(stype) = obj["type"].as_str() {
                            if let Some(sid) = obj["id"].as_str() {
                                let name = obj["name"].as_str().unwrap_or("");
                                self.update(stype, sid, name, score_delta);
                            }
                        }
                    }
                }
            }
            "object_query_result" => {
                if let Some(items) = envelope.data["items"].as_array() {
                    for item in items {
                        if let Some(stype) = item["source_type"].as_str() {
                            if let Some(sid) = item["id"].as_str() {
                                let name = item["name"].as_str().unwrap_or("");
                                self.update(stype, sid, name, score_delta);
                            }
                        }
                    }
                }
            }
            "memory_search_result" => {
                if let Some(memories) = envelope.data["memories"].as_array() {
                    for mem in memories {
                        if let Some(mid) = mem["memory_id"].as_str() {
                            self.shown_memory_ids.insert(mid.to_string());
                        }
                    }
                }
            }
            "relations_result" => {
                if let Some(relations) = envelope.data["relations"].as_array() {
                    for rel in relations {
                        if let Some(stype) = rel["type"].as_str() {
                            if let Some(sid) = rel["id"].as_str() {
                                let name = rel["name"].as_str().unwrap_or("");
                                self.update(stype, sid, name, score_delta);
                            }
                        }
                    }
                }
            }
            "chunk_navigate_result" => {
                if let Some(cid) = envelope.data["chunk"]["chunk_id"].as_str() {
                    self.mark_chunk_visited(cid);
                }
            }
            "batch_search_result" => {
                if let Some(results) = envelope.data["results"].as_array() {
                    for r in results {
                        if let Some(cid) = r["chunk_id"].as_str() {
                            self.mark_chunk_visited(cid);
                        }
                        if let Some(stype) = r["source_type"].as_str() {
                            if let Some(sid) = r["id"].as_str() {
                                self.update(stype, sid, "", score_delta);
                            }
                        }
                    }
                }
            }
            "object_catalog_result" => {
                let entity = envelope.data["entity"].as_str().unwrap_or("");
                if let Some(items) = envelope.data["items"].as_array() {
                    for item in items {
                        let id = item["id"].as_str().unwrap_or("");
                        let name = item["name"].as_str()
                            .or_else(|| item["title"].as_str())
                            .unwrap_or("");
                        if !entity.is_empty() && !id.is_empty() {
                            self.update(&entity.to_lowercase(), id, name, score_delta);
                        }
                    }
                }
            }
            "object_detail" => {
                if let (Some(st), Some(sid)) = (
                    envelope.data["type"].as_str(),
                    envelope.data["id"].as_str(),
                ) {
                    let name = envelope.data["name"].as_str().unwrap_or("");
                    self.update(st, sid, name, score_delta);
                }
                if let Some(cid) = envelope.data["chunk_id"].as_str() {
                    self.mark_chunk_visited(cid);
                }
            }
            t if t.ends_with("_create_result") || t.ends_with("_update_result") || t.ends_with("_delete_result") => {
                if let Some(id) = envelope.data["id"].as_str() {
                    let name = envelope.data["name"].as_str()
                        .or_else(|| envelope.data["title"].as_str())
                        .unwrap_or("");
                    let entity = t.replace("_create_result", "")
                        .replace("_update_result", "")
                        .replace("_delete_result", "");
                    self.update(&entity, id, name, score_delta);
                }
            }
            _ => {}
        }
    }

    /// 从工具结果更新会话状态（支持 JSON 信封和旧格式）
    pub fn update_from_tool_result(&mut self, tool_name: &str, result: &str) {
        // 尝试解析 JSON 信封
        if let Ok(envelope) = serde_json::from_str::<ToolResultEnvelope>(result) {
            self.update_from_envelope(tool_name, &envelope);
            return;
        }
        // 回退到旧格式解析
        for line in result.lines() {
            self.update_from_line(tool_name, line);
        }
    }

    /// 淘汰长期不活跃的对象

    /// 淘汰长期不活跃的对象
    fn evict_stale(&mut self) {
        let threshold = if self.access_counter > ACCESS_STALE_THRESHOLD { self.access_counter - ACCESS_STALE_THRESHOLD } else { 0 };
        self.active_objects.retain(|o| o.access_order >= threshold);
    }

    fn mark_chunk_visited(&mut self, chunk_id: &str) { self.visited_chunks.insert(chunk_id.to_string()); }
    fn mark_source_visited(&mut self, source_key: &str) { self.visited_sources.insert(source_key.to_string()); }
    fn record_search(&mut self, keywords: &[String]) { if !keywords.is_empty() { self.search_keywords.push(keywords.to_vec()); } }
    pub fn record_observation(&mut self, text: &str) {
        let t = text.trim();
        if !t.is_empty() && t.chars().count() > 10 {
            self.observations.insert(0, t.chars().take(200).collect());
            self.observations.truncate(5);
        }
    }
    fn set_focus_from_user_query(&mut self, query: &str, top_entities: &[String]) {
        if query.is_empty() { return; }
        self.focus.entities = top_entities.to_vec();
        // 追问检测：查询包含 top3 实体名称，或查询较短且有之前的观察
        let is_followup = top_entities.iter().any(|e| query.contains(e.as_str()))
            || (!self.observations.is_empty() && query.chars().count() < 40);
        if is_followup && !self.observations.is_empty() {
            self.focus.open_questions.push(query.to_string());
            self.focus.open_questions.truncate(3);
        } else {
            self.focus.open_questions.clear();
            self.focus.open_questions.push(query.to_string());
        }
    }

    /// 生成注入 Prompt 的工作记忆文本，受 Token 预算约束
    pub fn generate_scratchpad(&mut self) -> String {
        let mut lines: Vec<String> = Vec::new();

        // 1. Attention（注意力排序，按 score 降序，只保留 top 5）
        if !self.active_objects.is_empty() {
            lines.push(String::new());
            lines.push("## Attention（注意力排序）".to_string());
            let mut sorted: Vec<&ActiveObject> = self.active_objects.iter().filter(|o| o.score > 0.0).collect();
            sorted.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
            for (_i, o) in sorted.iter().take(5).enumerate() {
                let label = match o.obj_type.as_str() {
                    "product" => format!("PRODUCT: {}", o.name),
                    "project" => format!("PROJECT: {}", o.name),
                    "file" => format!("FILE: {}", o.name),
                    "note" => format!("NOTE: {}", o.name),
                    t => format!("{}: {}", t, o.name),
                };
                lines.push(format!("  ① {}  score: {:.2}", label, o.score));
            }
        }

        // 3. Focus（当前焦点，由 Goal 决定）
        if !self.focus.focus_objects.is_empty() {
            lines.push(String::new());
            lines.push("## Focus（当前焦点，由 Goal 决定）".to_string());
            for fid in self.focus.focus_objects.iter() {
                // 从 active_objects 中查找名称
                let name = self.active_objects.iter()
                    .find(|o| o.obj_id == *fid || format!("{}-{}", o.obj_type.to_uppercase(), o.obj_id) == *fid)
                    .map(|o| format!("{}: {}", o.obj_type.to_uppercase(), o.name))
                    .unwrap_or_else(|| fid.clone());
                lines.push(format!("  → {}", name));
            }
        }

        // ✅ 已删除：Working Memory（已确认/仍待确认）
        // ✅ 已删除：Execution Log
        // ✅ 已删除：Knowledge Progress
        // ✅ 已删除：Completeness（信息充分度）— 改为由 LLM 自行判断

        let mut text = lines.join("\n");
        if text.chars().count() > SCRATCHPAD_MAX_CHARS {
            let trunc: String = text.chars().take(SCRATCHPAD_MAX_CHARS - 3).collect();
            text = format!("{}…", trunc);
        }
        format!("\n\n{}\n", text)
    }
}

// ═══════════════════════════════════════════════════════════════════
// SessionEffect — 并行工具执行的延迟会话更新
// ═══════════════════════════════════════════════════════════════════
#[derive(Debug, Clone)]
enum SessionEffect {
    RecordSearch(Vec<String>),
    MarkSourceVisited(String),
    UpdateObject { obj_type: String, obj_id: String, name: String, score_delta: f64 },
    #[allow(dead_code)]
    MarkChunkVisited(String),
}

impl SessionMemory {
    fn apply_effect(&mut self, effect: &SessionEffect) {
        match effect {
            SessionEffect::RecordSearch(kw) => self.record_search(kw),
            SessionEffect::MarkSourceVisited(s) => self.mark_source_visited(s),
            SessionEffect::UpdateObject { obj_type, obj_id, name, score_delta } => {
                self.update(obj_type, obj_id, name, *score_delta);
            }
            SessionEffect::MarkChunkVisited(cid) => self.mark_chunk_visited(cid),
        }
    }
}

/// 从 session 的 active_objects 计算 scope（用于限定搜索范围）
/// 取 top 5 高分对象 + 所有新加来源（score < 1 的新进入注意力的对象）
fn compute_scope_from_session(session: &SessionMemory) -> Option<std::collections::HashSet<i64>> {
    let mut scope_ids: Vec<String> = Vec::new();
    let type_prefix = |t: &str| -> Option<&str> {
        match t.to_lowercase().as_str() {
            "product" => Some("PRODUCT"), "project" => Some("PROJECT"),
            "file" => Some("FILE"), "note" => Some("NOTE"), "todo" => Some("TODO"),
            "contact" => Some("CONTACT"), "chat" => Some("CHAT"),
            "channel" => Some("CHANNEL"), "vendor" => Some("VENDOR"), "customer" => Some("CUSTOMER"),
            _ => None
        }
    };
    // 1. top 5 高分对象
    let mut sorted: Vec<&ActiveObject> = session.active_objects.iter().collect();
    sorted.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    for o in sorted.iter().take(5) {
        if let Some(p) = type_prefix(&o.obj_type) {
            scope_ids.push(format!("{}-{}", p, o.obj_id));
        }
    }
    // 2. 新加入的来源（score < 1 的，来自 MEMORY 引用等）
    for o in session.active_objects.iter() {
        if o.score < 1.0 {
            if let Some(p) = type_prefix(&o.obj_type) {
                let key = format!("{}-{}", p, o.obj_id);
                if !scope_ids.contains(&key) { scope_ids.push(key); }
            }
        }
    }
    if scope_ids.is_empty() { log::info!("[Scope] scope_ids 为空，scope=None"); return None; }
    let s = resolve_scope_to_rowids(&scope_ids);
    log::info!("[Scope] scope_ids={:?}, resolved_rowids={}", scope_ids, s.len());
    if s.is_empty() { log::info!("[Scope] scope_rowids 解析为空，scope=None"); None } else { Some(s) }
}

fn execute_object_catalog(entity: &str, field_filters: &[Value], sort: Option<&Value>, args: &Value) -> String {
    let (table, allowed_fields): (&str, &[&str]) = match entity.to_uppercase().as_str() {
        "PROJECT" => ("projects", &["id","name","description","status","owner","budget","bid_time","created_at","updated_at"]),
        "PRODUCT" => ("products", &["id","name","series_name","description","price","standard_specs","full_specs AS params","sort_order","created_at","updated_at"]),
        "CHANNEL" | "VENDOR" | "CUSTOMER" => ("partners", &["id","type","name","phone","address","remark","contact_person","contact_phone","created_at","updated_at"]),
        "CONTACT" => ("partner_contacts", &["id","name","phone","email","position","remark","created_at"]),
        "TODO" => ("todos", &["id","title","content","done","priority","due_date","repeat","created_at","updated_at"]),
        "NOTE" => ("notes", &["note_id","title","content","is_daily","notebook_id","deleted_at","created_at","updated_at"]),
        "SOLUTION" => ("solutions", &["id","project_id","name","created_at","updated_at"]),
        "SOLUTION_VERSION" => ("solution_versions", &["id","solution_id","version","tags","content","creator","status","created_at","updated_at"]),
        "SOLUTION_ITEM" => ("(SELECT si.id, si.solution_version_id, si.product_id, si.product_name, si.device_name, si.quantity, si.unit_price, si.description AS params, si.row_type, si.sort_order, si.created_at, si.updated_at, sv.version, sv.status AS version_status, s.name AS solution_name, s.project_id, p.name AS project_name FROM solution_items si JOIN solution_versions sv ON sv.id=si.solution_version_id JOIN solutions s ON s.id=sv.solution_id JOIN projects p ON p.id=s.project_id)", &["id","solution_version_id","product_id","product_name","device_name","quantity","unit_price","params","row_type","sort_order","created_at","updated_at","version","version_status","solution_name","project_id","project_name"]),
        "PROJECT_DYNAMIC" => ("project_dynamics", &["id","project_id","type","content","operator","created_at"]),
        "CHAT" | "FILE" => return format!("{} 不支持结构化筛选，请使用 query 关键词搜索。", entity),
        _ => return format!("不支持的实体类型: {}。可用: PROJECT/PRODUCT/CHANNEL/VENDOR/CUSTOMER/CONTACT/TODO/NOTE/SOLUTION/SOLUTION_VERSION/SOLUTION_ITEM/PROJECT_DYNAMIC", entity),
    };
    let mut conditions: Vec<String> = Vec::new();
    let mut values: Vec<Value> = Vec::new();
    let upper = entity.to_uppercase();
    if upper == "CHANNEL" || upper == "VENDOR" || upper == "CUSTOMER" {
        conditions.push("type = ?".to_string());
        values.push(json!(upper));
    }
    for filter in field_filters {
        let field = filter["field"].as_str().unwrap_or("");
        let op = filter["op"].as_str().unwrap_or("=");
        let value = &filter["value"];
        if !allowed_fields.contains(&field) { continue; }
        match op {
            "IS_NULL" => { conditions.push(format!("{} IS NULL", field)); }
            "IS_NOT_NULL" => { conditions.push(format!("{} IS NOT NULL", field)); }
            "IN" => {
                if let Some(arr) = value.as_array() {
                    let placeholders: Vec<String> = arr.iter().map(|_| "?".to_string()).collect();
                    conditions.push(format!("{} IN ({})", field, placeholders.join(",")));
                    for v in arr { values.push(v.clone()); }
                }
            }
            "LIKE" => {
                let v = value.as_str().unwrap_or("");
                let like_value = if v.contains('%') { v.to_string() } else { format!("%{}%", v) };
                conditions.push(format!("{} LIKE ?", field)); 
                values.push(json!(like_value));
            }
            "=" | "!=" | ">" | "<" | ">=" | "<=" => { conditions.push(format!("{} {} ?", field, op)); values.push(value.clone()); }
            _ => {}
        }
    }
    if upper == "NOTE" && !conditions.iter().any(|c| c.contains("deleted_at")) {
        conditions.push("deleted_at IS NULL".to_string());
    }
    let where_clause = if conditions.is_empty() { String::new() } else { format!(" WHERE {}", conditions.join(" AND ")) };

    // ★ 聚合模式：count_only / group_by / aggregate
    if args["count_only"].as_bool().unwrap_or(false) {
        let sql = format!("SELECT COUNT(*) as cnt FROM {}{}", table, where_clause);
        return match crate::database::get(&sql, &values) {
            Ok(Some(row)) => {
                let count = row["cnt"].as_i64().unwrap_or(0);
                let envelope = ToolResultEnvelope {
                    result_type: "object_catalog_result".to_string(),
                    data: json!({"entity": entity, "count": count}),
                    summary: Some(format!("{} 共 {} 条匹配记录", entity, count)),
                };
                envelope.to_json()
            }
            Ok(None) => {
                let envelope = ToolResultEnvelope {
                    result_type: "object_catalog_result".to_string(),
                    data: json!({"entity": entity, "count": 0}),
                    summary: Some(format!("{} 共 0 条匹配记录", entity)),
                };
                envelope.to_json()
            }
            Err(e) => {
                let envelope = ToolResultEnvelope {
                    result_type: "object_catalog_result".to_string(),
                    data: json!({"success": false}),
                    summary: Some(format!("查询失败: {}", e)),
                };
                envelope.to_json()
            }
        };
    }
    if let Some(group) = args["group_by"].as_str().filter(|s| !s.is_empty()) {
        if allowed_fields.contains(&group) {
            let sql = format!("SELECT {}, COUNT(*) as cnt FROM {}{} GROUP BY {} ORDER BY cnt DESC", group, table, where_clause, group);
            return match crate::database::all(&sql, &values) {
                Ok(rows) => {
                    if rows.is_empty() {
                        let envelope = ToolResultEnvelope {
                            result_type: "object_catalog_result".to_string(),
                            data: json!({"entity": entity, "group_by": group, "groups": []}),
                            summary: Some(format!("{} 无匹配记录。", entity)),
                        };
                        return envelope.to_json();
                    }
                    let groups: Vec<Value> = rows.iter().map(|row| {
                        let val = row[group].as_str().unwrap_or("");
                        let display = if val.is_empty() { row[group].to_string() } else { val.to_string() };
                        json!({"value": display, "count": row["cnt"].as_i64().unwrap_or(0)})
                    }).collect();
                    let envelope = ToolResultEnvelope {
                        result_type: "object_catalog_result".to_string(),
                        data: json!({"entity": entity, "group_by": group, "groups": groups}),
                        summary: Some(format!("{} 按 {} 分组。", entity, group)),
                    };
                    envelope.to_json()
                }
                Err(e) => {
                    let envelope = ToolResultEnvelope {
                        result_type: "object_catalog_result".to_string(),
                        data: json!({"error": format!("查询失败: {}", e)}),
                        summary: Some(format!("查询失败: {}", e)),
                    };
                    envelope.to_json()
                }
            };
        } else {
            let envelope = ToolResultEnvelope {
                result_type: "object_catalog_result".to_string(),
                data: json!({"error": format!("字段 '{}' 不支持分组，可用字段：{}", group, allowed_fields.join(", "))}),
                summary: Some(format!("不支持按 '{}' 分组", group)),
            };
            return envelope.to_json();
        }
    }
    if let Some(agg) = args["aggregate"].as_str().filter(|s| !s.is_empty()) {
        // 解析 SUM(budget) / AVG(price) / MAX(budget) / MIN(budget)
        if let Some(paren_start) = agg.find('(') {
            if let Some(paren_end) = agg.rfind(')') {
                let func = agg[..paren_start].to_uppercase();
                let field = &agg[paren_start+1..paren_end].trim();
                if allowed_fields.contains(field) && ["SUM","AVG","MAX","MIN"].contains(&func.as_str()) {
                    let sql = format!("SELECT COALESCE({}({}), 0) as result FROM {}{}", func, field, table, where_clause);
                    return match crate::database::get(&sql, &values) {
                        Ok(Some(row)) => {
                            let result = if let Some(n) = row["result"].as_f64() {
                                if n == (n as i64) as f64 { n as i64 } else { n as i64 }
                            } else { 0 };
                            let envelope = ToolResultEnvelope {
                                result_type: "object_catalog_result".to_string(),
                                data: json!({"entity": entity, "function": func, "field": field, "result": result}),
                                summary: Some(format!("{} {}({}): {}", entity, func, field, result)),
                            };
                            envelope.to_json()
                        }
                        Ok(None) => {
                            let envelope = ToolResultEnvelope {
                                result_type: "object_catalog_result".to_string(),
                                data: json!({"entity": entity, "function": func, "field": field, "result": 0}),
                                summary: Some(format!("{} 无匹配记录。", entity)),
                            };
                            envelope.to_json()
                        }
                        Err(e) => {
                            let envelope = ToolResultEnvelope {
                                result_type: "object_catalog_result".to_string(),
                                data: json!({"success": false}),
                                summary: Some(format!("查询失败: {}", e)),
                            };
                            envelope.to_json()
                        }
                    };
                }
            }
        }
        let envelope = ToolResultEnvelope {
            result_type: "object_catalog_result".to_string(),
            data: json!({"success": false}),
            summary: Some("聚合格式错误，请使用 SUM(字段)/AVG(字段)/MAX(字段)/MIN(字段)，字段必须是允许的字段名。".to_string()),
        };
        return envelope.to_json();
    }

    let order = if let Some(sort) = sort.and_then(|s| s.as_object()) {
        let field = sort["field"].as_str().unwrap_or("created_at");
        let dir = if sort["order"].as_str().unwrap_or("desc") == "asc" { "ASC" } else { "DESC" };
        if allowed_fields.contains(&field) {
            format!(" ORDER BY {} {}", field, dir)
        } else {
            let envelope = ToolResultEnvelope {
                result_type: "object_catalog_result".to_string(),
                data: json!({"error": format!("字段 '{}' 不支持排序，可用字段：{}", field, allowed_fields.join(", "))}),
                summary: Some(format!("不支持按 '{}' 排序", field)),
            };
            return envelope.to_json();
        }
    } else { " ORDER BY created_at DESC".to_string() };
    let limit = args["limit"].as_u64().unwrap_or(20).min(100);
    let offset = args["offset"].as_u64().unwrap_or(0);
    let sql = format!("SELECT {} FROM {}{}{} LIMIT {} OFFSET {}", allowed_fields.join(","), table, where_clause, order, limit, offset);
    match crate::database::all(&sql, &values) {
        Ok(rows) => {
            if rows.is_empty() {
                let envelope = ToolResultEnvelope {
                    result_type: "object_catalog_result".to_string(),
                    data: json!({"entity": entity, "items": [], "total": 0, "offset": offset}),
                    summary: Some(format!("未找到匹配的{}对象。", entity)),
                };
                return envelope.to_json();
            }
        let total = rows.len();
            let items: Vec<Value> = rows.iter().map(|row| {
                json!(row)
            }).collect();
            let envelope = ToolResultEnvelope {
                result_type: "object_catalog_result".to_string(),
                data: json!({
                    "entity": entity,
                    "items": items,
                    "total": total,
                    "offset": offset,
                    "limit": limit
                }),
                summary: Some(format!("显示 {}/{} 条{}对象。", offset as usize + total, total, entity)),
            };
            envelope.to_json()
        }
        Err(e) => {
            let envelope = ToolResultEnvelope {
                result_type: "object_catalog_result".to_string(),
                data: json!({"success": false}),
                summary: Some(format!("查询失败: {}", e)),
            };
            envelope.to_json()
        }
    }
}

#[allow(dead_code)]
fn format_catalog_row(entity: &str, row: &Value) -> String {
    let upper = entity.to_uppercase();
    match upper.as_str() {
        "PROJECT" => {
            let status = row["status"].as_str().unwrap_or("");
            let budget = row["budget"].as_f64().unwrap_or(0.0);
            let bid = row["bid_time"].as_str().unwrap_or("");
            let mut s = format!("[PROJECT] {} | PROJECT-{}", row["name"].as_str().unwrap_or(""), row["id"].as_str().unwrap_or(""));
            if !status.is_empty() { s.push_str(&format!(" | 状态: {}", status)); }
            if budget > 0.0 { s.push_str(&format!(" | 预算: {}万", budget)); }
            if !bid.is_empty() { s.push_str(&format!(" | 投标: {}", bid)); }
            s
        }
        "TODO" => {
            let done = row["done"].as_i64().unwrap_or(0);
            let pri = row["priority"].as_str().unwrap_or("");
            let due = row["due_date"].as_str().unwrap_or("");
            let status = if done != 0 { "已完成" } else { "未完成" };
            let mut s = format!("[TODO] {} | TODO-{}", row["title"].as_str().unwrap_or(""), row["id"].as_str().unwrap_or(""));
            if !pri.is_empty() { s.push_str(&format!(" | 优先级: {}", pri)); }
            if !due.is_empty() { s.push_str(&format!(" | 截止: {}", due)); }
            s.push_str(&format!(" | {}", status));
            s
        }
        "PRODUCT" => {
            let mut s = format!("[PRODUCT] {} | PRODUCT-{}", row["name"].as_str().unwrap_or(""), row["id"].as_str().unwrap_or(""));
            if let Some(series) = row["series_name"].as_str().filter(|s| !s.is_empty()) { s.push_str(&format!(" | 系列: {}", series)); }
            if let Some(price) = row["price"].as_f64().filter(|p| *p > 0.0) { s.push_str(&format!(" | 价格: {}元", price)); }
            s
        }
        "CONTACT" => {
            let mut s = format!("[CONTACT] {} | CONTACT-{}", row["name"].as_str().unwrap_or(""), row["id"].as_str().unwrap_or(""));
            if let Some(phone) = row["phone"].as_str().filter(|s| !s.is_empty()) { s.push_str(&format!(" | 电话: {}", phone)); }
            if let Some(pos) = row["position"].as_str().filter(|s| !s.is_empty()) { s.push_str(&format!(" | 职位: {}", pos)); }
            s
        }
        "CHANNEL" | "VENDOR" | "CUSTOMER" => {
            let mut s = format!("[{}] {} | {}", upper, row["name"].as_str().unwrap_or(""), row["id"].as_str().unwrap_or(""));
            if let Some(phone) = row["phone"].as_str().filter(|s| !s.is_empty()) { s.push_str(&format!(" | 电话: {}", phone)); }
            if let Some(cp) = row["contact_person"].as_str().filter(|s| !s.is_empty()) { s.push_str(&format!(" | 联系人: {}", cp)); }
            s
        }
        "NOTE" => {
            let id = row["note_id"].as_str().unwrap_or(row["id"].as_str().unwrap_or(""));
            let mut s = format!("[NOTE] {} | NOTE-{}", row["title"].as_str().unwrap_or(""), id);
            if let Some(nb) = row["notebook_id"].as_str().filter(|s| !s.is_empty()) { s.push_str(&format!(" | 笔记本: {}", nb)); }
            s
        }
        _ => format!("[{}] {}", upper, row["id"].as_str().unwrap_or("")),
    }
}

fn execute_relations(entity_type: &str, entity_id: &str, target_type: Option<&str>, limit: usize, offset: usize) -> String {
    if entity_type.is_empty() || entity_id.is_empty() {
        let envelope = ToolResultEnvelope {
            result_type: "relations_result".to_string(),
            data: json!({"source": {"type": entity_type, "id": entity_id}, "total": 0, "relations": []}),
            summary: Some("请提供 entity_type 和 entity_id。".to_string()),
        };
        return envelope.to_json();
    }
    let upper = entity_type.to_uppercase();
    // 去掉 TYPE- 前缀，business_relations 存储裸 UUID
    let bare_id = entity_id.strip_prefix(&format!("{}-", upper)).unwrap_or(entity_id);

    // 正向查询 + 反向查询 business_relations（含 metadata 供图谱导航透传）
    let forward_sql = "SELECT br.target_type, br.target_id, br.metadata, sd.title
        FROM business_relations br
        LEFT JOIN source_data sd ON sd.source_type = br.target_type AND sd.source_id = br.target_id
        WHERE br.source_type = ?1 AND br.source_id = ?2";
    let reverse_sql = "SELECT br.source_type as target_type, br.source_id as target_id, br.metadata, sd.title
        FROM business_relations br
        LEFT JOIN source_data sd ON sd.source_type = br.source_type AND sd.source_id = br.source_id
        WHERE br.target_type = ?1 AND br.target_id = ?2";

    let params = vec![json!(upper), json!(bare_id)];
    let mut all_rels: Vec<(String, String, String, String)> = Vec::new();

    if let Ok(rows) = crate::database::all(forward_sql, &params) {
        for r in &rows {
            if let (Some(tt), Some(tid)) = (r["target_type"].as_str(), r["target_id"].as_str()) {
                if !tt.is_empty() && !tid.is_empty() {
                    all_rels.push((tt.to_uppercase(), tid.to_string(), r["title"].as_str().unwrap_or("").to_string(), r["metadata"].as_str().unwrap_or("").to_string()));
                }
            }
        }
    }
    if let Ok(rows) = crate::database::all(reverse_sql, &params) {
        for r in &rows {
            if let (Some(tt), Some(tid)) = (r["target_type"].as_str(), r["target_id"].as_str()) {
                if !tt.is_empty() && !tid.is_empty() {
                    all_rels.push((tt.to_uppercase(), tid.to_string(), r["title"].as_str().unwrap_or("").to_string(), r["metadata"].as_str().unwrap_or("").to_string()));
                }
            }
        }
    }

    // 去重 + 过滤自身 + 按 target_type 过滤
    all_rels.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    all_rels.dedup_by(|a, b| a.0 == b.0 && a.1 == b.1);
    all_rels.retain(|(t, id, _, _)| !(t == &upper && id == entity_id));
    if let Some(tt) = target_type {
        let tt_upper = tt.to_uppercase();
        all_rels.retain(|(t, _, _, _)| t == &tt_upper);
    }

    let total = all_rels.len();

    // 获取源实体名称
    let source_name = crate::database::get(
        "SELECT title FROM source_data WHERE source_type=?1 AND source_id=?2",
        &[json!(&upper), json!(bare_id)],
    ).ok().flatten().and_then(|r| r["title"].as_str().map(String::from)).unwrap_or_default();

    // 分页
    let start = offset.min(total);
    let end = (offset + limit).min(total);

    let relations: Vec<Value> = all_rels[start..end].iter().map(|(tt, tid, name, metadata)| {
        // 图谱节点（方案/版本/设备实例）不在 source_data 中，直接从业务表解析名称
        let resolved = if name.is_empty() { resolve_graph_node_name(tt, tid) } else { name.clone() };
        let mut relation = json!({
            "type": tt,
            "name": if resolved.is_empty() { tid.as_str() } else { resolved.as_str() },
            "id": tid
        });
        // 透传边 metadata（如版本金额/状态、设备数量/单价）
        if !metadata.is_empty() {
            if let Ok(meta) = serde_json::from_str::<Value>(metadata) {
                if meta.as_object().map(|o| !o.is_empty()).unwrap_or(false) {
                    relation["metadata"] = meta;
                }
            }
        }
        // CONTACT 类型额外返回 phone、email、position
        if tt == "CONTACT" {
            if let Ok(Some(contact)) = database::get(
                "SELECT phone, email, position FROM partner_contacts WHERE id=?1",
                &[json!(tid)]
            ) {
                if let Some(phone) = contact["phone"].as_str() {
                    if !phone.is_empty() { relation["phone"] = json!(phone); }
                }
                if let Some(email) = contact["email"].as_str() {
                    if !email.is_empty() { relation["email"] = json!(email); }
                }
                if let Some(position) = contact["position"].as_str() {
                    if !position.is_empty() { relation["position"] = json!(position); }
                }
            }
        }
        relation
    }).collect();

    let summary = if total == 0 {
        format!("{} 没有关联对象。", source_name)
    } else {
        format!("{}关联 {} 个对象。", source_name, relations.len())
    };

    let envelope = ToolResultEnvelope {
        result_type: "relations_result".to_string(),
        data: json!({
            "source": {
                "type": upper,
                "id": entity_id,
                "name": source_name
            },
            "total": total,
            "offset": offset,
            "limit": limit,
            "relations": relations
        }),
        summary: Some(summary),
    };
    envelope.to_json()
}

/// 解析图谱节点名称（SOLUTION/SOLUTION_VERSION/SOLUTION_ITEM 不进 source_data，直接查业务表）
fn resolve_graph_node_name(node_type: &str, id: &str) -> String {
    let sql = match node_type {
        "SOLUTION" => "SELECT name FROM solutions WHERE id=?1",
        "SOLUTION_VERSION" => "SELECT s.name || ' · ' || sv.version AS name FROM solution_versions sv JOIN solutions s ON s.id=sv.solution_id WHERE sv.id=?1",
        "SOLUTION_ITEM" => "SELECT CASE WHEN si.device_name IS NOT NULL AND si.device_name != '' THEN si.device_name || ' · ' || si.product_name ELSE si.product_name END AS name FROM solution_items si WHERE si.id=?1",
        _ => return String::new(),
    };
    database::get(sql, &[json!(id)]).ok().flatten()
        .and_then(|r| r["name"].as_str().map(String::from))
        .unwrap_or_default()
}

/// 工具模式：控制 agent_chat_workflow 加载哪些工具
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ToolMode {
    /// 通用模式：12 个基础工具（知识库/对象/项目/待办等）
    General,
    /// 演示材料模式：12 个基础工具 + 3 个演示材料工具
    Presentation,
}

fn build_presentation_tool_definitions() -> Vec<Value> {
    vec![
        json!({"type":"function","function":{
            "name": "pres_tool_generate_page",
            "description": "生成一页演示材料的 HTML。逐页生成，每页调用一次。输出 <section class='slide'> 元素。使用全局 CSS 变量确保风格一致。",
            "parameters": {"type":"object","properties":{
                "page": {"type":"integer", "description": "页码，从 1 开始"},
                "instruction": {"type":"string", "description": "该页的设计要求（布局、内容、风格）"},
                "totalPages": {"type":"integer", "description": "总页数（可选，帮助 AI 理解整体结构）"}
            }, "required": ["page", "instruction"]}
        }}),
        json!({"type":"function","function":{
            "name": "pres_tool_regenerate_slide",
            "description": "重新设计某一页的 HTML。保留核心信息，重新设计布局和样式。需要传入当前页的 HTML。",
            "parameters": {"type":"object","properties":{
                "page": {"type":"integer", "description": "页码，从 1 开始"},
                "instruction": {"type":"string", "description": "修改要求"},
                "currentHtml": {"type":"string", "description": "当前页的完整 HTML（从之前的工具结果中获取）"}
            }, "required": ["page", "instruction", "currentHtml"]}
        }}),
        json!({"type":"function","function":{
            "name": "pres_tool_modify_slide",
            "description": "修改某一页中的特定元素。用于小调整（改颜色、改字号、改文字等）。需要传入当前页的 HTML。",
            "parameters": {"type":"object","properties":{
                "page": {"type":"integer", "description": "页码，从 1 开始"},
                "instruction": {"type":"string", "description": "修改要求"},
                "currentHtml": {"type":"string", "description": "当前页的完整 HTML"},
                "elementSelector": {"type":"string", "description": "CSS 选择器（可选，指定要修改的元素）"}
            }, "required": ["page", "instruction", "currentHtml"]}
        }}),
    ]
}

fn build_tools_definitions(tool_mode: ToolMode) -> Value {
    let mut tools = json!([
        {"type":"function","function":{"name":"ext_tool_search","description":"在知识库中搜索文档内容（全文检索+语义检索）。适用于：找到提到某个关键词的文档、笔记、技术资料。支持时间范围过滤。返回匹配的文档片段及 ChunkID。","parameters":{"type":"object","properties":{"keywords":{"type":"array","items":{"type":"string"},"description":"1~4 个核心关键词"},"start_time":{"type":"string","description":"开始时间，格式 2026-06-25 或 昨天"},"end_time":{"type":"string","description":"结束时间，格式 2026-06-25 或 昨天"}},"required":["keywords"]}}},
        {"type":"function","function":{"name":"ext_tool_object","description":"查询或统计业务对象。必须指定 mode 决定操作类型。如需搜索文档正文内容（笔记、文件、资料），请用 ext_tool_search。","parameters":{"type":"object","properties":{"mode":{"type":"string","enum":["detail","search","filter","count","group","aggregate"],"description":"操作模式：detail=按ID查看详情，search=关键词搜索对象目录（名称/标题），filter=按字段筛选记录，count=统计数量，group=按字段分组统计，aggregate=聚合计算(SUM/AVG/MAX/MIN)"},"id":{"type":"array","items":{"type":"string"},"description":"[detail模式-必填] 对象ID列表，格式 TYPE-UUID 或裸UUID"},"query":{"type":"string","description":"[search模式-必填] 关键词，搜索对象的名称/标题等元数据"},"type":{"type":"string","description":"[filter/count/group/aggregate模式-必填] 对象类型：PROJECT/PRODUCT/CHANNEL/VENDOR/CUSTOMER/CONTACT/TODO/NOTE/FILE/SOLUTION/SOLUTION_VERSION(仅filter模式)/SOLUTION_ITEM(仅filter模式)/PROJECT_DYNAMIC(仅filter模式)"},"field_filters":{"type":"array","items":{"type":"object","properties":{"field":{"type":"string"},"op":{"type":"string","enum":["=","!=",">","<",">=","<=","LIKE","IN","IS_NULL","IS_NOT_NULL"]},"value":{"type":["string","number","boolean","null"]}},"required":["field","op","value"]},"description":"[filter/count模式] 结构化字段过滤"},"group_by":{"type":"string","description":"[group模式-必填] 分组字段名"},"aggregate":{"type":"string","description":"[aggregate模式-必填] 聚合表达式：SUM(字段)/AVG(字段)/MAX(字段)/MIN(字段)"},"sort":{"type":"object","properties":{"field":{"type":"string"},"order":{"type":"string","enum":["asc","desc"]}},"description":"排序配置"},"start_time":{"type":"string","description":"开始时间"},"end_time":{"type":"string","description":"结束时间"},"limit":{"type":"integer","description":"每页条数，默认50"},"offset":{"type":"integer","description":"分页偏移，默认0"}},"required":["mode"]}}},
        {"type":"function","function":{"name":"ext_tool_chunk_navigate","description":"在知识块间翻页。next=向后翻页，prev=向前翻页。用于阅读长文档的前后内容。⚠️ chunk_id 是8位短ID，来自 search() 结果的 ChunkID 字段。","parameters":{"type":"object","properties":{"chunk_id":{"type":"string","description":"8位短ID，必须来自 search() 结果的 ChunkID 字段"},"edge_type":{"type":"string","enum":["next","prev"],"description":"next=向后翻页；prev=向前翻页"},"radius":{"type":"integer","description":"返回块数量，默认10，最大20"}},"required":["chunk_id","edge_type"]}}},
        {"type":"function","function":{"name":"ext_tool_relations","description":"查询实体的关联对象（双向）。如：项目关联的方案(SOLUTION)/产品/合作伙伴/文件，方案关联的版本(SOLUTION_VERSION，metadata含金额（元）/设备数/状态)，版本关联的设备实例(SOLUTION_ITEM，metadata含型号/数量/单价（元）)，设备实例关联的产品(PRODUCT)。返回关联对象的名称、类型、ID及边metadata。联系人类型额外返回 phone/email/position。","parameters":{"type":"object","properties":{"entity_type":{"type":"string","description":"源实体类型：PROJECT/PRODUCT/SOLUTION/SOLUTION_VERSION/SOLUTION_ITEM/CHANNEL/VENDOR/CUSTOMER/CONTACT/TODO/NOTE/FILE"},"entity_id":{"type":"string","description":"源实体ID"},"target_type":{"type":"string","description":"目标类型过滤：SOLUTION/SOLUTION_VERSION/SOLUTION_ITEM/PRODUCT/FILE/NOTE/TODO/CHANNEL/VENDOR/CONTACT/CUSTOMER。不传返回所有关联"},"limit":{"type":"integer","description":"每页条数，默认20"},"offset":{"type":"integer","description":"分页偏移，默认0"}},"required":["entity_type","entity_id"]}}},
        {"type":"function","function":{"name":"ext_tool_manage_todo","description":"管理待办。查看待办请用 ext_tool_object(type=\"TODO\")。action=create/update/delete。","parameters":{"type":"object","properties":{"action":{"type":"string","enum":["create","update","delete"]},"title":{"type":"string","description":"待标题(create必填)"},"content":{"type":"string","description":"待办详情"},"priority":{"type":"string","description":"优先级: P1/P2/P3 或 高/中/低"},"due_date":{"type":"string","description":"截止日期: YYYY-MM-DD"},"repeat":{"type":"string","description":"循环: daily/weekly/monthly"},"project_id":{"type":"string","description":"关联项目ID"},"done":{"type":"boolean","description":"是否完成(update用)"},"todo_id":{"type":"string","description":"待办ID(update/delete)"}},"required":["action"]}}},
        {"type":"function","function":{"name":"ext_tool_manage_project","description":"管理项目。查看项目请用 ext_tool_object(type=\"PROJECT\")。action=create/update/delete/add_dynamic/delete_dynamic。add_dynamic=追加项目动态（需project_id+content）。delete_dynamic=删除动态（需dynamic_id）。","parameters":{"type":"object","properties":{"action":{"type":"string","enum":["create","update","delete","add_dynamic","delete_dynamic"]},"project_id":{"type":"string","description":"项目ID(update/delete/add_dynamic用)"},"name":{"type":"string","description":"项目名称(create用)"},"budget":{"type":"number","description":"预算金额，单位：万元"},"description":{"type":"string","description":"项目描述"},"bid_time":{"type":"string","description":"投标时间 YYYY-MM-DD"},"status":{"type":"string","description":"项目状态: Lead/SolutionDesign/Ordered/Won/Lost/Archived"},"content":{"type":"string","description":"动态内容(add_dynamic必填)"},"dynamic_type":{"type":"string","description":"动态类型(add_dynamic用,默认ai)：ai=AI记录/note=用户记录/system=系统变更"},"dynamic_id":{"type":"string","description":"动态ID(delete_dynamic用)"}},"required":["action"]}}},
        {"type":"function","function":{"name":"ext_tool_manage_partner","description":"管理合作伙伴（渠道/厂商/客户）。查看请用 ext_tool_object(type=\"CHANNEL/VENDOR/CUSTOMER\")。action=create/update/delete/link_project/unlink_project。","parameters":{"type":"object","properties":{"action":{"type":"string","enum":["create","update","delete","link_project","unlink_project"]},"partner_id":{"type":"string","description":"合作伙伴ID(update/delete/link_project/unlink_project用)"},"type":{"type":"string","description":"类型: CHANNEL/VENDOR/CUSTOMER(create用)"},"name":{"type":"string","description":"名称(create/update用)"},"phone":{"type":"string","description":"电话"},"address":{"type":"string","description":"地址"},"remark":{"type":"string","description":"备注"},"project_id":{"type":"string","description":"项目ID(link_project/unlink_project用)"}},"required":["action"]}}},
        {"type":"function","function":{"name":"ext_tool_manage_contact","description":"管理联系人（归属于合作伙伴下的人员）。查看联系人请用 ext_tool_object(type=\"CONTACT\")。action=create/update/delete/link_project/unlink_project。创建联系人时必须提供 partner_id 指定所属合作伙伴。","parameters":{"type":"object","properties":{"action":{"type":"string","enum":["create","update","delete","link_project","unlink_project"]},"contact_id":{"type":"string","description":"联系人ID(update/delete/link_project/unlink_project用)"},"name":{"type":"string","description":"姓名(create/update用)"},"phone":{"type":"string","description":"电话"},"email":{"type":"string","description":"邮箱"},"position":{"type":"string","description":"职位"},"remark":{"type":"string","description":"备注"},"partner_id":{"type":"string","description":"所属合作伙伴ID(create必填)。联系人是合作伙伴下的人员，必须指定归属。"},"project_id":{"type":"string","description":"项目ID(link_project/unlink_project用)"}},"required":["action"]}}},
        {"type":"function","function":{"name":"ext_tool_memory","description":"记录和检索长期记忆。action=create(创建记忆), action=update(更新记忆内容), action=delete(删除记忆), action=search(搜索记忆，不传query或传*可列出所有记忆)。","parameters":{"type":"object","properties":{"action":{"type":"string","enum":["create","update","delete","search"],"description":"必填。search=搜索记忆，create=创建记忆，update=更新记忆，delete=删除记忆"},"content":{"type":"string","description":"记忆内容（create/update 必填）"},"items":{"type":"array","items":{"type":"string"},"description":"批量创建：多条记忆内容数组"},"memory_id":{"type":"string","description":"记忆ID（update/delete 必填）"},"query":{"type":"string","description":"搜索关键词（search 用）。不传或传 * 可列出所有记忆"},"start_time":{"type":"string","description":"开始时间"},"end_time":{"type":"string","description":"结束时间"}},"required":["action"]}}},
        {"type":"function","function":{"name":"ext_tool_manage_policy","description":"管理用户策略。action=create(新增策略), action=update(更新策略), action=delete(删除策略)。策略会自动注入 prompt，无需手动查询。","parameters":{"type":"object","properties":{"action":{"type":"string","enum":["create","update","delete"],"description":"必填。create=新增策略，update=更新策略，delete=删除策略"},"id":{"type":"string","description":"策略ID（update/delete 用）"},"content":{"type":"string","description":"策略内容（create/update 用）"}},"required":["action"]}}},
        {"type":"function","function":{"name":"ext_tool_manage_notification","description":"管理通知。action=create(创建通知)、update(更新通知)、delete(删除通知)、list(查看通知列表)、get(查看通知详情)。只有在发现真正需要用户关注的洞察时才调用，不要滥用。","parameters":{"type":"object","properties":{"action":{"type":"string","enum":["create","update","delete","list","get"],"description":"必填。create=创建通知，update=更新通知，delete=删除通知，list=查看通知，get=查看通知详情"},"notification_id":{"type":"string","description":"通知ID（get/update/delete用）"},"type":{"type":"string","description":"通知类型: todo_due/todo_today/project_stale/bid_deadline/llm_insight"},"title":{"type":"string","description":"通知标题，简短精炼"},"content":{"type":"string","description":"通知详情，使用 Markdown 格式：支持 **加粗**、- 列表、`代码`、> 引用等，让内容结构清晰易读"},"entity_type":{"type":"string","description":"关联实体类型: todo/project/null"},"entity_id":{"type":"string","description":"关联实体ID"},"priority":{"type":"string","enum":["high","medium","low"],"description":"优先级"},"follow_up_of":{"type":"string","description":"跟进的通知ID"},"limit":{"type":"integer","description":"每页条数，默认20"},"offset":{"type":"integer","description":"分页偏移，默认0"}},"required":["action"]}}},
    ]);
    // 演示材料模式：追加 3 个演示工具
    if tool_mode == ToolMode::Presentation {
        if let Some(arr) = tools.as_array_mut() {
            for t in build_presentation_tool_definitions() {
                arr.push(t);
            }
        }
    }
    tools
}


/// Fallback: parse XML tool calls from content when model does not use function calling format
fn parse_xml_tool_calls(content: &str) -> Option<Vec<Value>> {
    // 先清理 DSML 标记（支持全角/半角/Unicode 变体），只移除标记本身，保留标签名和闭合斜杠
    // 匹配: ｜｜DSML｜｜, |DSML|, ‖DSML‖, DSML（管道符号+DSML）
    // 不匹配: <, >, / 等括号和斜杠
    let re = regex::Regex::new(r"(?i)[|｜‖]*DSML[|｜‖]*").ok()?;
    let cleaned = re.replace_all(content, "").to_string();
    
    // 查找 tool_calls 块
    let tag_start = "tool_calls>";
    let tag_end = "/tool_calls>";
    if !cleaned.contains(tag_start) || !cleaned.contains(tag_end) { return None; }
    let start = cleaned.find(tag_start).map(|i| i + tag_start.len())?;
    let end = cleaned.find(tag_end)?;
    let xml_body = &cleaned[start..end];
    
    let mut tool_calls = Vec::new();
    let mut remaining = xml_body;
    while let Some(inv_start) = remaining.find("invoke name=\"") {
        let ns = inv_start + "invoke name=\"".len();
        let ne = remaining[ns..].find('"').map(|i| ns + i)?;
        let func_name = &remaining[ns..ne];
        let ps = remaining.find('>').map(|i| i + 1)?;
        let ib = &remaining[ps..];
        let ie = ib.find("/invoke>").unwrap_or(ib.len());
        let pb = &ib[..ie];
        let mut params = serde_json::Map::new();
        let mut pr = pb;
        while let Some(pst) = pr.find("parameter name=\"") {
            let pns = pst + "parameter name=\"".len();
            let pne = pr[pns..].find('"').map(|i| pns + i)?;
            let pn = pr[pns..pne].to_string();
            let pvs = pr.find('>').map(|i| i + 1)?;
            let pbody = &pr[pvs..];
            let pe = pbody.find("/parameter>").unwrap_or(pbody.len());
            let pv = pbody[..pe].trim().to_string();
            let jv = serde_json::from_str::<Value>(&pv).unwrap_or_else(|_| json!(pv));
            params.insert(pn, jv);
            pr = &pr[pvs + pe + "/parameter>".len()..];
        }
        tool_calls.push(json!({
            "id": format!("call_{}", &uuid::Uuid::new_v4().to_string()[..8]),
            "type": "function",
            "function": {"name": func_name, "arguments": serde_json::to_string(&Value::Object(params)).unwrap_or_default()}
        }));
        remaining = &remaining[ps + ie + "/invoke>".len()..];
    }
    if tool_calls.is_empty() { None } else { Some(tool_calls) }
}

/// Fallback: parse HTML-style XML tool calls from content
/// Format: tool_call, function=name, parameter=name value
fn parse_html_tool_calls(content: &str) -> Option<Vec<Value>> {
    let tc_open = "\u{3c}tool_call\u{3e}";
    let tc_close = "\u{3c}/tool_call\u{3e}";
    let fn_open = "\u{3c}function=";
    let param_open = "\u{3c}parameter=";
    let param_close = "\u{3c}/parameter\u{3e}";

    let mut tool_calls = Vec::new();
    let mut remaining = content;
    while let Some(tc_start) = remaining.find(tc_open) {
        let after_tag = tc_start + tc_open.len();
        let tc_end = remaining[after_tag..].find(tc_close).unwrap_or(remaining[after_tag..].len());
        let block = &remaining[after_tag..after_tag + tc_end];
        remaining = &remaining[after_tag + tc_end + tc_close.len()..];

        let func_name = if let Some(fn_start) = block.find(fn_open) {
            let ns = fn_start + fn_open.len();
            let ne = block[ns..].find('>').map(|i| ns + i).unwrap_or(ns);
            block[ns..ne].trim().to_string()
        } else {
            continue;
        };
        if func_name.is_empty() { continue; }

        let mut params = serde_json::Map::new();
        let mut inner = block;
        while let Some(p_start) = inner.find(param_open) {
            let pn_start = p_start + param_open.len();
            let pn_end = inner[pn_start..].find('>').map(|i| pn_start + i).unwrap_or(pn_start);
            let pname = inner[pn_start..pn_end].trim().to_string();
            let after_p = pn_end + 1;
            let p_end = inner[after_p..].find(param_close).unwrap_or(inner[after_p..].len());
            let pval = inner[after_p..after_p + p_end].trim().to_string();
            let jv = serde_json::from_str::<Value>(&pval).unwrap_or_else(|_| json!(pval));
            params.insert(pname, jv);
            inner = &inner[after_p + p_end + param_close.len()..];
        }
        tool_calls.push(json!({
            "id": format!("call_{}", &uuid::Uuid::new_v4().to_string()[..8]),
            "type": "function",
            "function": {"name": func_name, "arguments": serde_json::to_string(&Value::Object(params)).unwrap_or_default()}
        }));
    }
    if tool_calls.is_empty() { None } else { Some(tool_calls) }
}

/// 清理消息内容中的非法字符，防止 LLM API 解析失败
fn sanitize_content(content: &str) -> String {
    content.chars().filter(|c| {
        // 保留正常字符，过滤 null 字符和控制字符（保留换行和制表符）
        !(*c == '\0') && (!c.is_control() || *c == '\n' || *c == '\r' || *c == '\t')
    }).collect()
}

/// 从 content 中移除工具调用标记，返回干净的文本
/// - DSML 标记（全角/半角/Unicode 变体）
/// - <tool_calls>...</tool_calls> 块
/// - <tool_call>...</tool_call> 块
/// - 可选：ext_tool_xxx(...) 函数调用（strip_ext_tool=true 时）
/// - 压缩连续空行
pub fn clean_tool_markers(text: &str, strip_ext_tool: bool) -> String {
    let mut out = text.to_string();
    let re_dsml = regex::Regex::new(r"(?i)[|｜‖]*DSML[|｜‖]*").unwrap();
    out = re_dsml.replace_all(&out, "").to_string();
    let re_xml = regex::Regex::new(r"(?s)<tool_calls>.*?</tool_calls>").unwrap();
    out = re_xml.replace_all(&out, "").to_string();
    let re_tool_call = regex::Regex::new(r"(?s)<tool_call>.*?</tool_call>").unwrap();
    out = re_tool_call.replace_all(&out, "").to_string();
    if strip_ext_tool {
        let re_tool = regex::Regex::new(r"(?s)ext_tool_\w+\s*\([^)]*\)").unwrap();
        out = re_tool.replace_all(&out, "").to_string();
    }
    while out.contains("\n\n\n") {
        out = out.replace("\n\n\n", "\n\n");
    }
    out.trim().to_string()
}

/// ★ 单请求流式调用：一次请求同时完成
///   - content 逐 token 流式输出到前端（agent:token / agent:start-answer）
///   - tool_calls delta 按 index 累积（工具调用决策）
///   - usage 统计（stream_options.include_usage）
/// 返回 (content, reasoning_content, tool_calls, usage)
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
async fn call_llm_without_tools(messages: &[ChatMessage], app_handle: &Option<tauri::AppHandle>) -> Result<String, String> {
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
// ═══════════════════════════════════════════════════════════════════

/// 从 source_id 字符串解析出 source_type 和 source_id
/// 支持格式：PROJECT-uuid 或裸 UUID
fn parse_source_id(source_id_str: &str) -> (String, String) {
    let known_types = ["PROJECT", "PRODUCT", "FILE", "NOTE", "TODO", "CHANNEL", "VENDOR", "CONTACT", "CUSTOMER"];
    if let Some(dash) = source_id_str.find('-') {
        let prefix = &source_id_str[..dash];
        if known_types.contains(&prefix.to_uppercase().as_str()) {
            return (prefix.to_uppercase(), source_id_str[dash+1..].to_string());
        }
    }
    ("__raw__".to_string(), source_id_str.to_string())
}

/// 查询 source_data 表获取来源信息，返回 JSON 对象
/// 返回结构：{"found": true, "id": "uuid", "type": "PROJECT", "name": "名称", "source_data_id": "123"}
pub fn build_chunk_metadata(source_id_str: &str) -> serde_json::Value {
    if source_id_str.is_empty() { return json!({"found": false}); }

    let (source_type, source_id) = parse_source_id(source_id_str);
    
    let result = if source_type == "__raw__" {
        // 裸 UUID：搜索所有类型的 source_data
        database::get(
            "SELECT id, source_type, source_id, title FROM source_data WHERE source_id=?1 LIMIT 1",
            &[json!(&source_id)],
        )
    } else {
        // 带前缀：精确查询
        database::get(
            "SELECT id, source_type, source_id, title FROM source_data WHERE source_type=?1 AND source_id=?2",
            &[json!(&source_type), json!(&source_id)],
        )
    };

    match result {
        Ok(Some(r)) => {
            let sdid = r["id"].as_str().unwrap_or("").to_string();
            let st = r["source_type"].as_str().unwrap_or("").to_string();
            let sid = r["source_id"].as_str().unwrap_or("").to_string();
            let title = r["title"].as_str().unwrap_or("").to_string();
            json!({
                "found": true,
                "id": sid,
                "type": st,
                "name": title,
                "source_data_id": sdid
            })
        }
        _ => json!({"found": false})
    }
}

// ═══════════════════════════════════════════════════════════════════
// Tag-based pre-filtering (已废弃，tags 表已移除)
// ═══════════════════════════════════════════════════════════════════


/// 关键词 → 横向检索：纯数据库外键级联（标签查询已移除）。
fn execute_horizontal_search(keywords: &[String]) -> String {
    if keywords.is_empty() { return String::new(); }
    let _like = format!("%{}%", crate::database::escape_like(&keywords.join(" ")));

    if true { return String::new(); }
    String::new()
}

// ═══════════════════════════════════════════════════════════════════
// Tool executors
// ═══════════════════════════════════════════════════════════════════

/// 将 "TYPE-id" 格式的 scope 列表解析为 child rowid 集合（用于限定搜索范围）
/// 优化版：直接通过 child chunk 的 source_data_id 查询，跳过 parent 中转
fn resolve_scope_to_rowids(scope: &[String]) -> std::collections::HashSet<i64> {
    let mut all_rowids = std::collections::HashSet::new();
    for s in scope {
        let parts: Vec<&str> = s.splitn(2, '-').collect();
        if parts.len() != 2 { log::info!("[Scope] 无法解析 scope key: {}", s); continue; }
        let st = parts[0].to_uppercase();
        let sid = parts[1];
        // source_data.id 是 TEXT 类型（= source_id），不能用 as_i64()
        match database::get("SELECT id FROM source_data WHERE source_type=?1 AND source_id=?2",&[json!(st), json!(sid)]) {
            Ok(Some(row)) => {
                let sdid_str = row["id"].as_str().unwrap_or("");
                if sdid_str.is_empty() { log::info!("[Scope] ✗ {} source_data.id 为空", s); continue; }
                match database::all("SELECT id FROM knowledge_chunks WHERE chunk_type='child' AND source_data_id=?1",&[json!(sdid_str)]) {
                    Ok(rows) => {
                        let count = rows.len();
                        for cr in &rows { if let Some(cid) = cr["id"].as_i64() { all_rowids.insert(cid); } }
                        log::info!("[Scope] ✓ {} → source_data_id={}, child_chunks={}", s, sdid_str, count);
                    }
                    Err(e) => log::info!("[Scope] ✗ {} 查询 child_chunks 失败: {}", s, e),
                }
            }
            Ok(None) => log::info!("[Scope] ✗ {} 在 source_data 中未找到 (type={}, sid={})", s, st, sid),
            Err(e) => log::info!("[Scope] ✗ {} 查询 source_data 失败: {}", s, e),
        }
    }
    all_rowids
}

/// 执行 search 并同时返回发现的来源列表
async fn execute_search_knowledge(keywords: &[String], scope_rowids: Option<std::collections::HashSet<i64>>, _visited_sources: &std::collections::HashSet<String>, visited_chunks: &std::collections::HashSet<String>, time_range: Option<(String, String)>, _shown_memory_ids: &std::collections::HashSet<String>) -> (String, Vec<String>) {
    if keywords.is_empty() || keywords.join(" ").trim().is_empty() { return ("no keywords".into(), vec![]); }

    // ★ 停用词过滤：避免"项目""产品"等类型词污染 FTS5/Vector 搜索（对象名精确匹配也使用过滤后关键词）
    let filtered_keywords: Vec<String> = {
        let stopwords: std::collections::HashSet<&str> = [
            "项目", "产品", "文件", "客户", "渠道", "厂商", "供应商", "联系人", "笔记", "待办", "任务",
            "project", "product", "file", "customer", "channel", "vendor", "contact", "note", "todo",
            "介绍", "详情", "什么", "查询", "检索", "搜索", "一下", "查一下", "的", "是", "和", "或",
        ].iter().cloned().collect();
        let f: Vec<String> = keywords.iter().filter(|k| !stopwords.contains(k.as_str().to_lowercase().as_str())).cloned().collect();
        if f.is_empty() { keywords.to_vec() } else { f }
    };
    let keywords = filtered_keywords.as_slice();
    let query = keywords.join(" ");

    let tag_rowids: std::collections::HashSet<i64> = std::collections::HashSet::new();
    let mut obj_head = String::new();
    let mut matched_sources: std::collections::HashSet<(String,String)> = std::collections::HashSet::new();
    if !keywords.is_empty() {
        let like = format!("%{}%", crate::database::escape_like(&keywords.join(" ")));
        if let Ok(rows) = database::all("SELECT id,name,status FROM projects WHERE name LIKE ?1 ESCAPE '\\' ORDER BY LENGTH(name) ASC LIMIT 3",&[json!(&like)]) {
            for r in &rows { let id = r["id"].as_str().unwrap_or("").to_string(); matched_sources.insert(("PROJECT".into(), id.clone())); obj_head.push_str(&format!("[Object] Project: {} | Status: {} | source_id: PROJECT-{}\n",r["name"].as_str().unwrap_or(""),r["status"].as_str().unwrap_or(""),id)); }
        }
        if let Ok(rows) = database::all("SELECT id,name FROM products WHERE name LIKE ?1 ORDER BY LENGTH(name) ASC LIMIT 3",&[json!(&like)]) {
            for r in &rows { let id = r["id"].as_str().unwrap_or("").to_string(); matched_sources.insert(("PRODUCT".into(), id.clone())); obj_head.push_str(&format!("[Object] Product: {} | source_id: PRODUCT-{}\n",r["name"].as_str().unwrap_or(""),id)); }
        }
        if let Ok(rows) = database::all("SELECT source_id, title FROM source_data WHERE source_type='FILE' AND title LIKE ?1 ORDER BY LENGTH(title) ASC LIMIT 3",&[json!(&like)]) {
            for r in &rows { let id = r["source_id"].as_str().unwrap_or("").to_string(); matched_sources.insert(("FILE".into(), id.clone())); obj_head.push_str(&format!("[Object] File: {} | source_id: FILE-{}\n",r["title"].as_str().unwrap_or(""),id)); }
        }
        if let Ok(rows) = database::all("SELECT note_id,title FROM notes WHERE title LIKE ?1 ORDER BY LENGTH(title) ASC LIMIT 3",&[json!(&like)]) {
            for r in &rows { let id = r["note_id"].as_str().unwrap_or("").to_string(); obj_head.push_str(&format!("[Object] Note: {} | source_id: NOTE-{}\n",r["title"].as_str().unwrap_or(""),id)); }
        }
        if let Ok(rows) = database::all("SELECT id,title,priority,due_date FROM todos WHERE title LIKE ?1 ORDER BY LENGTH(title) ASC LIMIT 3",&[json!(&like)]) {
            for r in &rows { let id = r["id"].as_str().unwrap_or("").to_string(); let pri = r["priority"].as_str().unwrap_or(""); let due = r["due_date"].as_str().unwrap_or(""); let extra = if !pri.is_empty() { format!(" | 优先级: {}", pri) } else { String::new() } + &if !due.is_empty() { format!(" | 截止: {}", due) } else { String::new() }; obj_head.push_str(&format!("[Object] Todo: {}{} | source_id: TODO-{}\n",r["title"].as_str().unwrap_or(""),extra,id)); }
        }
        if let Ok(rows) = database::all("SELECT id,name,phone,position FROM partner_contacts WHERE name LIKE ?1 ORDER BY LENGTH(name) ASC LIMIT 3",&[json!(&like)]) {
            for r in &rows { let id = r["id"].as_str().unwrap_or("").to_string(); let pos = r["position"].as_str().unwrap_or(""); let extra = if !pos.is_empty() { format!(" | {}", pos) } else { String::new() }; obj_head.push_str(&format!("[Object] Contact: {}{} | source_id: CONTACT-{}\n",r["name"].as_str().unwrap_or(""),extra,id)); }
        }
    }
    let _ = if !obj_head.is_empty() { format!("--- Related Objects ---\n{}--- Chunks ---\n", obj_head) } else { String::new() };

    // ★ 始终走双路（FTS5 + Vector），不再因 scope 而跳过向量检索
    // ★ 始终走双路（FTS5 + Vector），不再因 scope 而跳过向量检索

    let vec_rowids: Vec<(String,f64)> = {
        match crate::services::embedding_client::get_embedding_with_caller(&query, "agent_workflow").await {
            Ok(qv) => {
                match crate::services::lancedb_client::get_lancedb() {
                    Ok(g) => match g.as_ref() {
                        Some(c) => {
                            let raw = c.search_vectors_with_parent(&qv, 30).unwrap_or_default();
                            raw.into_iter().map(|(r,s,_,_,_)|(r.to_string(),s)).collect()
                        }
                        None => vec![]
                    }
                    Err(_) => vec![]
                }
            }
            Err(_e) => vec![]
        }
    };
    let fts_limit = 30;
    let fts_q = keywords.iter().map(|k| format!("\"{}\"", k.replace('"',""))).collect::<Vec<_>>().join(" OR ");
    let fts_rowids: Vec<(String,f64)> = database::fts5_search_chunks(&fts_q, fts_limit).unwrap_or_default().iter()
        .map(|fr| (fr.rowid.to_string(), fr.rank)).collect();
    log::info!("Search keywords={:?}, scope={}", keywords, scope_rowids.as_ref().map(|s| format!("Some({} rowids)", s.len())).unwrap_or_else(|| "None".to_string()));
    log::info!("Search FTS5 hits={}, Vector hits={}", fts_rowids.len(), vec_rowids.len());

    let fuse = |rows: Vec<(String,f64)>| -> Vec<(String,f64)> {
        if tag_rowids.is_empty() { return rows; }
        rows.into_iter().filter(|(id,_)| id.parse::<i64>().map(|r| tag_rowids.contains(&r)).unwrap_or(false)).collect()
    };

    let fts_filtered = fuse(fts_rowids.clone());
    let vec_filtered = fuse(vec_rowids.clone());
    let fused = search_assets::rrf_fusion(&fts_filtered, &vec_filtered, 60.0);

    if fused.is_empty() {
        let kw_display = keywords.join("、");
        let diag = format!("未找到关键词「{}」的匹配切片。\n\n💡 已在全库范围搜索，无匹配结果。建议：\n1. 使用更简短或不同的关键词重试（如去掉型号只留品牌）\n2. 使用 object 工具先查看知识库中的文件与产品范围\n3. 确认相关文件已导入知识库", kw_display);
        return (diag, vec![]);
    }
    let mut seen_hashes = std::collections::HashSet::new();
    let mut disc_sources: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut found_sources: Vec<String> = Vec::new();
    let mut candidates: Vec<(f64, i64, String, String, String, String, String, i64, i64, bool, bool)> = Vec::new();

    for (rowid_str, rrf) in fused.iter() {
        let Ok(cr) = rowid_str.parse::<i64>() else { continue; };
        if let Ok(Some(r)) = database::get("SELECT p.content,p.content_hash,p.doc_title,p.chunk_index,p.chunk_id,p.source_data_id as parent_sdid,sd.source_type,sd.source_id,sd.id as source_data_id FROM knowledge_chunks c JOIN knowledge_chunks p ON p.chunk_id=c.parent_id LEFT JOIN source_data sd ON sd.id=p.source_data_id WHERE c.rowid=?1",&[json!(cr)]) {
            let st = r["source_type"].as_str().unwrap_or("").to_string();
            let sid = r["source_id"].as_str().unwrap_or("").to_string();
            let dedup = format!("{}-{}", st, sid);
            let h = r["content_hash"].as_str().unwrap_or("").to_string();
            let content = r["content"].as_str().unwrap_or("").to_string();
            if content.trim().chars().count() < 60 {
                continue;
            }
            let chunk_id = r["chunk_id"].as_str().unwrap_or("").to_string();
            let chunk_index = r["chunk_index"].as_i64().unwrap_or(0);
            let sdid = r["source_data_id"].as_str().unwrap_or("").to_string();
            let is_in_scope = scope_rowids.as_ref().map(|sids| sids.contains(&cr)).unwrap_or(false);

            let base_rrf = if matched_sources.contains(&(st.clone(), sid.clone())) { rrf * 1.5 } else { *rrf };
            let syn_mult = crate::services::synapse::rrf_weight_multiplier(cr);
            let effective_rrf = base_rrf * syn_mult;
            let (score, is_scope_path) = if is_in_scope {
                (effective_rrf, true)
            } else {
                // discovery path: 每个 source 每轮最多展示 2 个 chunk
                let count = disc_sources.entry(dedup.clone()).or_insert(0);
                *count += 1;
                if *count > 2 {
                    continue;
                }
                (effective_rrf * 0.3, false)
            };
            if !h.is_empty() && !seen_hashes.insert(h.clone()) {
                continue;
            }
            // ★ 时间过滤
            if let Some((ref start, ref end)) = time_range {
                let created_at: String = database::get("SELECT created_at FROM knowledge_chunks WHERE id=?1",&[json!(cr)]).ok().flatten().and_then(|r| r["created_at"].as_str().map(String::from)).unwrap_or_default();
                if !created_at.is_empty() {
                    let date = &created_at[..10.min(created_at.len())];  // "2026-06-25"
                    if date < start.as_str() || date > end.as_str() { continue; }
                }
            }
            // ★ 已在前轮展示过的 chunk 直接丢弃，让排名更靠后的新 chunk 补位
            let short_chunk: String = chunk_id.chars().take(8).collect();
            if !short_chunk.is_empty() && visited_chunks.contains(&short_chunk) { continue; }

            let total_parents: i64 = database::get("SELECT COUNT(*) as cnt FROM knowledge_chunks WHERE source_data_id=?1 AND chunk_type='parent'",&[json!(sdid)]).ok().flatten().and_then(|r| r["cnt"].as_i64()).unwrap_or(0);
            candidates.push((score, cr, st.clone(), sid.clone(), chunk_id, content, h, chunk_index, total_parents, is_scope_path, matched_sources.contains(&(st, sid))));
        }
    }

    candidates.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    log::info!("[Search] RRF fusion={}, candidates={}, scope_in={}, discovery={}", fused.len(), candidates.len(),
        candidates.iter().filter(|c| c.9).count(), candidates.iter().filter(|c| !c.9).count());

    // ★ 双路结果：非MEMORY top 3
    let non_memory: Vec<_> = candidates.iter().filter(|(_, _, st, _, _, _, _, _, _, _, _)| st != "MEMORY").take(3).collect();

    // ── 知识检索区 ──
    let mut search_out = String::new();
    if !non_memory.is_empty() {
        search_out.push_str("## 📄 知识检索");
        for (idx, (score, cr, st, sid, chunk_id, content, _h, chunk_index, total_parents, is_scope_path, is_obj_match)) in non_memory.iter().enumerate() {
            let rrf_tag = &format!("{:.3}", score);
            let boost_tag = if *is_obj_match { "🔥" } else { "" };
            let source_id_str = format!("{}-{}", st, sid);
            let meta_json = build_chunk_metadata(&source_id_str);
            let mut meta = if meta_json["found"].as_bool().unwrap_or(false) {
                format!("来源：{} · {}", 
                    match st.as_str() { "FILE" => "文件", "PRODUCT" => "产品", "PROJECT" => "项目", "NOTE" => "笔记", "TODO" => "待办", _ => st },
                    meta_json["name"].as_str().unwrap_or(sid)
                )
            } else {
                String::new()
            };
            if *total_parents > 1 {
                if !meta.is_empty(){meta.push('\n');}
                meta.push_str(&format!("父块：{}/{}", chunk_index + 1, total_parents));
            }
            if !chunk_id.is_empty() {
                let short: String = chunk_id.chars().take(8).collect();
                if !meta.is_empty(){meta.push('\n');}
                meta.push_str(&format!("ChunkID：{}", short));
            }
            if *is_scope_path {
                meta.push_str("\n📍 该段落属于当前工作区");
            } else {
                meta.push_str("\n💡 该段落来自全库发现（不属于当前工作区）");
            }
            let _source_label = match st.as_str() {
                "FILE" => "FILE",
                "PRODUCT" => "PRODUCT",
                "PROJECT" => "PROJECT",
                "NOTE" => "NOTE",
                "TODO" => "TODO",
                _ => st,
            };
            let created_at: String = database::get(
                "SELECT created_at FROM knowledge_chunks WHERE id=?1", &[json!(cr)],
            ).ok().flatten().and_then(|r| r["created_at"].as_str().map(String::from)).unwrap_or_default();
            let time_info = relative_time(&created_at);
            if !time_info.is_empty() {
                if !meta.is_empty() { meta.push('\n'); }
                meta.push_str(&time_info);
            }
            let preview = content.clone();
            if meta.is_empty() { search_out.push_str(&format!("\n\n--- [父块{}|RRF={}]{} ---\n{}", idx + 1, rrf_tag, boost_tag, preview)); }
            else { search_out.push_str(&format!("\n\n--- [父块{}|RRF={}]{} ---\n{}\n--------\n{}", idx + 1, rrf_tag, boost_tag, meta, preview)); }

            found_sources.push(format!("{}-{}", st, sid));

            // ★ 关联对象自动进入 workspace（统一从 business_relations 查询）
            match st.as_str() {
                "FILE" => {
                    // 文件关联的实体：双向查询
                    for rel_type in &["PROJECT", "PRODUCT", "TODO", "NOTE"] {
                        // 正向：FILE 是 source
                        if let Ok(rows) = database::all(
                            "SELECT target_id FROM business_relations WHERE source_type='FILE' AND source_id=?1 AND target_type=?2",
                            &[json!(sid.as_str()), json!(rel_type)],
                        ) {
                            for r in &rows {
                                if let Some(eid) = r["target_id"].as_str().filter(|s| !s.is_empty()) {
                                    let dedup = format!("{}-{}", rel_type, eid);
                                    if !found_sources.iter().any(|s| s == &dedup) { found_sources.push(dedup); }
                                }
                            }
                        }
                        // 反向：FILE 是 target
                        if let Ok(rows) = database::all(
                            "SELECT source_id FROM business_relations WHERE target_type='FILE' AND target_id=?1 AND source_type=?2",
                            &[json!(sid.as_str()), json!(rel_type)],
                        ) {
                            for r in &rows {
                                if let Some(eid) = r["source_id"].as_str().filter(|s| !s.is_empty()) {
                                    let dedup = format!("{}-{}", rel_type, eid);
                                    if !found_sources.iter().any(|s| s == &dedup) { found_sources.push(dedup); }
                                }
                            }
                        }
                    }
                }
                "PROJECT" | "PRODUCT" | "NOTE" | "TODO" => {
                    // 实体关联的文件：双向查询
                    // 正向：实体是 source，FILE 是 target
                    if let Ok(rows) = database::all(
                        "SELECT target_id FROM business_relations WHERE source_type=?1 AND source_id=?2 AND target_type='FILE'",
                        &[json!(st), json!(sid.as_str())],
                    ) {
                        for r in &rows {
                            if let Some(fid) = r["target_id"].as_str().filter(|s| !s.is_empty()) {
                                let dedup = format!("FILE-{}", fid);
                                if !found_sources.iter().any(|s| s == &dedup) { found_sources.push(dedup); }
                            }
                        }
                    }
                    // 反向：实体是 target，FILE 是 source
                    if let Ok(rows) = database::all(
                        "SELECT source_id FROM business_relations WHERE target_type=?1 AND target_id=?2 AND source_type='FILE'",
                        &[json!(st), json!(sid.as_str())],
                    ) {
                        for r in &rows {
                            if let Some(fid) = r["source_id"].as_str().filter(|s| !s.is_empty()) {
                                let dedup = format!("FILE-{}", fid);
                                if !found_sources.iter().any(|s| s == &dedup) { found_sources.push(dedup); }
                            }
                        }
                    }
                }
                _ => {}
            }

            // 父块被采纳 → 激活其下所有子块
            if !chunk_id.is_empty() {
                if let Ok(child_rows) = database::all("SELECT id FROM knowledge_chunks WHERE chunk_type='child' AND parent_id=?1",&[json!(chunk_id)]) {
                    for cr in &child_rows { if let Some(cid) = cr["id"].as_i64() { crate::services::synapse::activate_child_chunk(cid); } }
                }
            }
        }
    }

    // 合并知识检索结果
    let _horizontal = execute_horizontal_search(keywords);
    
    // 构建 chunks 数组
    let chunks: Vec<Value> = non_memory.iter().map(|(score, cr, st, sid, chunk_id, content, _h, chunk_index, total_parents, is_scope_path, is_obj_match)| {
        let short_chunk_id: String = chunk_id.chars().take(8).collect();
        let preview = content.clone();
        // 查询时间信息
        let created_at: String = database::get(
            "SELECT created_at FROM knowledge_chunks WHERE id=?1",
            &[json!(cr)],
        ).ok().flatten().and_then(|r| r["created_at"].as_str().map(String::from)).unwrap_or_default();
        json!({
            "chunk_id": short_chunk_id,
            "rrf_score": score,
            "source": {
                "type": st,
                "id": sid
            },
            "chunk_index": chunk_index,
            "total_parents": total_parents,
            "is_in_workspace": is_scope_path,
            "is_boosted": is_obj_match,
            "content": preview,
            "created_at": created_at
        })
    }).collect();
    
    // 构建 related_objects 数组（带名称）
    let related_objects: Vec<Value> = matched_sources.iter().map(|(st, sid)| {
        let name = database::get(
            "SELECT title FROM source_data WHERE source_type=?1 AND source_id=?2",
            &[json!(st), json!(sid)],
        ).ok().flatten().and_then(|r| r["title"].as_str().map(String::from)).unwrap_or_default();
        json!({
            "type": st,
            "id": sid,
            "name": name
        })
    }).collect();
    
    let total_hits = chunks.len();
    let summary = if total_hits == 0 {
        let kw_display = keywords.join("、");
        format!("未找到关键词「{}」的匹配切片。", kw_display)
    } else {
        format!("找到 {} 条相关知识片段。", total_hits)
    };
    
    let envelope = ToolResultEnvelope {
        result_type: "knowledge_search_result".to_string(),
        data: json!({
            "query": query,
            "total_hits": total_hits,
            "related_objects": related_objects,
            "chunks": chunks
        }),
        summary: Some(summary),
    };
    
    (envelope.to_json(), found_sources)
}


fn execute_traverse(chunk_id: &str, edge_type: &str, radius: usize, visited: &std::collections::HashSet<String>, _target_name: &str, traverse_source_counter: &mut std::collections::HashMap<String, usize>) -> String {
    if chunk_id.trim().is_empty() {
        let envelope = ToolResultEnvelope {
            result_type: "chunk_navigate_result".to_string(),
            data: json!({"current_index": 0, "total_chunks": 0, "coverage": 0.0, "chunks": [], "warnings": ["请提供 ChunkID。"]}),
            summary: Some("请提供 ChunkID。".to_string()),
        };
        return envelope.to_json();
    }
    let cid = chunk_id.trim();
    let row = match database::get("SELECT p.chunk_id, p.content, p.chunk_index, p.source_data_id, sd.source_type, sd.source_id FROM knowledge_chunks p LEFT JOIN source_data sd ON sd.id=p.source_data_id WHERE p.chunk_type='parent' AND p.chunk_id LIKE ?1",&[json!(format!("{}%", cid))]) { Ok(Some(r)) => r, _ => {
        let envelope = ToolResultEnvelope {
            result_type: "chunk_navigate_result".to_string(),
            data: json!({"current_index": 0, "total_chunks": 0, "coverage": 0.0, "chunks": [], "warnings": [format!("未找到 ChunkID「{}」对应的块。", cid)]}),
            summary: Some(format!("未找到 ChunkID「{}」对应的块。", cid)),
        };
        return envelope.to_json();
    }};
    let current_index = row["chunk_index"].as_i64().unwrap_or(0);
    let sdid = match row["source_data_id"].as_str() { Some(id) => id.to_string(), None => {
        let envelope = ToolResultEnvelope {
            result_type: "chunk_navigate_result".to_string(),
            data: json!({"current_index": 0, "total_chunks": 0, "coverage": 0.0, "chunks": [], "warnings": ["该块无关联源数据。"]}),
            summary: Some("该块无关联源数据。".to_string()),
        };
        return envelope.to_json();
    }};
    let total: i64 = database::get("SELECT COUNT(*) as cnt FROM knowledge_chunks WHERE source_data_id=?1 AND chunk_type='parent'",&[json!(sdid)]).ok().flatten().and_then(|r| r["cnt"].as_i64()).unwrap_or(0);
    let r = radius.max(1).min(20);
    // 将认知边缘映射到物理边缘
    let edge = match edge_type {
        "next" | "Next" => "Next",
        "prev" | "Prev" => "Prev",
        _ => edge_type,
    };
    match edge {
        "Next" | "Prev" => {
            let remaining = total - (current_index + 1);
            let _coverage_pct = ((current_index + 1) as f64 / total as f64 * 100.0) as i64;
            let est_steps = if remaining > 0 { (remaining + r as i64 - 1) / r as i64 } else { 0 };
            let get_chunk = |offset: i64| -> Option<(String, String, i64)> {
                let target = current_index + offset;
                if target < 0 || target >= total { return None; }
                let r2 = database::get("SELECT chunk_id, content, chunk_index FROM knowledge_chunks WHERE source_data_id=?1 AND chunk_type='parent' AND chunk_index=?2",&[json!(sdid),json!(target)]).ok().flatten()?;
                let cid_out = r2["chunk_id"].as_str().unwrap_or("").to_string();
                let short = cid_out.chars().take(8).collect::<String>();
                let preview = r2["content"].as_str().unwrap_or("").to_string();
                Some((short, preview, target))
            };
            let mut chunks = Vec::new();
            let mut warnings = Vec::new();
            let mut visited_indices: Vec<i64> = Vec::new();
            // ★ 遍历 radius 个步长，跳过已访问的 chunk，直到集齐 radius 个新块或到边界
            if edge == "Prev" {
                let mut got = 0usize;
                for step in 1..=r.max(r*2) {
                    if got >= r as usize { break; }
                    if let Some((short, preview, target)) = get_chunk(-(step as i64)) {
                        if visited.contains(&short) { visited_indices.push(target); continue; }
                        got += 1;
                        chunks.push(json!({
                            "chunk_id": short,
                            "index": target + 1,
                            "total": total,
                            "content": preview,
                            "direction": "prev",
                            "step": step
                        }));
                    }
                }
            }
            if edge == "Next" {
                let mut got = 0usize;
                for step in 1..=r.max(r*2) {
                    if got >= r as usize { break; }
                    if let Some((short, preview, target)) = get_chunk(step as i64) {
                        if visited.contains(&short) { visited_indices.push(target); continue; }
                        got += 1;
                        chunks.push(json!({
                            "chunk_id": short,
                            "index": target + 1,
                            "total": total,
                            "content": preview,
                            "direction": "next",
                            "step": step
                        }));
                    } else { break; }
                }
            }
            // ★ 标记已跳过哪些块
            if !visited_indices.is_empty() {
                let skipped: Vec<String> = visited_indices.iter().map(|i| format!("{}", i+1)).collect();
                warnings.push(format!("跳过 {}（已在前面展示过）", skipped.join(", ")));
            }
            // ★ 连续遍历同一文件的阻断信号
            if edge == "Next" || edge == "Prev" {
                // 新 source → 重置计数器；同一 source → 递增
                let consecutive = if let Some(c) = traverse_source_counter.get_mut(&sdid) {
                    *c += 1; *c
                } else {
                    traverse_source_counter.clear();
                    traverse_source_counter.insert(sdid, 1);
                    1
                };
                if consecutive > 5 {
                    warnings.push(format!("已在该文件中连续遍历 {} 次（剩余约 {} 块，预计还需约 {} 步）。建议使用 search 工具定位目标关键词，而非继续翻页。", consecutive, remaining, est_steps));
                }
            }
            // 边界阻断信号：无后续/前置块时明确提示 LLM
            if chunks.is_empty() {
                warnings.push(match edge {
                    "Next" => "已到达当前文档的物理【最末尾】，下方已无后续内容。请立即停止向后漫游，总结已有信息或切换到其他文档。".to_string(),
                    "Prev" => "已到达当前文档的物理【最开头】，上方已无前文内容。请立即停止向前漫游。".to_string(),
                    _ => String::new(),
                });
            }
            let coverage = (current_index + 1) as f64 / total as f64;
            let envelope = ToolResultEnvelope {
                result_type: "chunk_navigate_result".to_string(),
                data: json!({
                    "current_index": current_index + 1,
                    "total_chunks": total,
                    "coverage": coverage,
                    "radius": r,
                    "direction": edge_type,
                    "chunks": chunks,
                    "warnings": warnings
                }),
                summary: Some(format!("已导航到 {}/{} 块，覆盖率 {:.0}%。", current_index + 1, total, coverage * 100.0)),
            };
            envelope.to_json()
        }
        _ => {
            let envelope = ToolResultEnvelope {
                result_type: "chunk_navigate_result".to_string(),
                data: json!({"current_index": 0, "total_chunks": 0, "coverage": 0.0, "chunks": [], "warnings": [format!("不支持的边类型「{}」。支持：next, prev。", edge)]}),
                summary: Some(format!("不支持的边类型「{}」。支持：next, prev。", edge)),
            };
            envelope.to_json()
        }
    }
}

/// ★ 批量并发检索：同时检索多个独立子查询，每个子查询返回最多 2 个结果
async fn execute_batch_search(queries: &[Vec<String>], scope_rowids: Option<std::collections::HashSet<i64>>, visited_sources: &std::collections::HashSet<String>) -> String {
    if queries.len() < 3 {
        return format!("只有 {} 个子查询。batch_search 需要至少 3 个子查询。请使用 ext_tool_search 获取更精准的结果。", queries.len());
    }
    // 并发执行每个子查询的 FTS5 + 向量双路检索
    let mut handles = Vec::new();
    for kw in queries.iter().take(10) {
        let kw = kw.clone();
        let _sids = scope_rowids.clone();
        let _visited = visited_sources.clone();
        handles.push(tokio::spawn(async move {
            let query = kw.join(" ");
            // FTS5 路径
            let fts_q = kw.iter().map(|k| format!("\"{}\"", k.replace('"',""))).collect::<Vec<_>>().join(" OR ");
            let fts_rowids: Vec<(String,f64)> = database::fts5_search_chunks(&fts_q, 30).unwrap_or_default().iter()
                .map(|fr| (fr.rowid.to_string(), fr.rank)).collect();
            // 向量路径
            let vec_rowids: Vec<(String,f64)> = match crate::services::embedding_client::get_embedding_with_caller(&query, "batch_search").await {
                Ok(qv) => match crate::services::lancedb_client::get_lancedb() {
                    Ok(g) => match g.as_ref() {
                        Some(c) => c.search_vectors_with_parent(&qv, 30).unwrap_or_default().into_iter()
                            .map(|(r,s,_,_,_)|(r.to_string(),s)).collect(),
                        None => vec![],
                    },
                    Err(_) => vec![],
                },
                Err(_) => vec![],
            };
            // RRF 融合，每个子查询取 top 2
            let fuse = |rows: Vec<(String,f64)>| rows;
            let fused = crate::services::search_assets::rrf_fusion(&fuse(fts_rowids), &fuse(vec_rowids), 60.0);
            let mut items: Vec<(String, f64, String, String, String)> = Vec::new(); // (chunk_id, rrf, source_type, source_id, source_id_display)
            let mut seen_local = std::collections::HashSet::new();
            for (rowid_str, rrf) in fused.iter().take(2) {
                if let Ok(cr) = rowid_str.parse::<i64>() {
                    if let Ok(Some(r)) = database::get("SELECT p.content,p.chunk_id,sd.source_type,sd.source_id FROM knowledge_chunks c JOIN knowledge_chunks p ON p.chunk_id=c.parent_id LEFT JOIN source_data sd ON sd.id=p.source_data_id WHERE c.rowid=?1",&[json!(cr)]) {
                        let chunk_id = r["chunk_id"].as_str().unwrap_or("").to_string();
                        if chunk_id.is_empty() || !seen_local.insert(chunk_id.clone()) { continue; }
                        let st = r["source_type"].as_str().unwrap_or("").to_string();
                        let sid = r["source_id"].as_str().unwrap_or("").to_string();
                        items.push((chunk_id, *rrf, st.clone(), sid.clone(), format!("{}-{}", st, sid)));
                    }
                }
            }
            (kw, items)
        }));
    }
    // 收集结果
    let mut all_results: Vec<(Vec<String>, Vec<(String, f64, String, String, String)>)> = Vec::new();
    for h in handles {
        if let Ok((kw, items)) = h.await { all_results.push((kw, items)); }
    }

    // ★ 跨子查询仅按 chunk_id 去重（不按 source 去重，同一个文件不同 chunk 各自保留）
    let mut seen_chunks = std::collections::HashSet::new();
    let mut out = String::from("## 批量检索结果\n\n");
    let mut total_hits = 0usize;
    for (kw, items) in &all_results {
        let kw_str = kw.join("、");
        let mut hits = 0usize;
        for (chunk_id, rrf, _st, _sid, source_display) in items {
            if !seen_chunks.insert(chunk_id.clone()) { continue; }
            hits += 1;
            total_hits += 1;
            if let Ok(Some(r)) = database::get("SELECT p.content FROM knowledge_chunks c JOIN knowledge_chunks p ON p.chunk_id=c.parent_id WHERE c.chunk_id=?1",&[json!(chunk_id)]) {
                if let Some(content) = r["content"].as_str() {
                    out.push_str(&format!("**[{}] RRF={:.3} | source_id: {}**\n> {}\n\n", kw_str, rrf, source_display, content.replace('\n', "\n> ")));
                }
            }
        }
        if hits == 0 {
            out.push_str(&format!("**[{}]** ❌ 未找到匹配内容\n\n", kw_str));
        }
    }
    out.push_str(&format!("\n---\n共 {} 个检索结果（跨子查询按 chunk_id 去重）", total_hits));
    out
}

/// 批量 Recall：并发获取多个对象的完整信息
async fn execute_batch_recall(ids: &[String], visited_chunks: &std::collections::HashSet<String>, visited_sources: &std::collections::HashSet<String>) -> String {
    if ids.is_empty() { return json!({"result_type": "object_detail_batch", "data": {"objects": []}, "summary": "请提供至少一个对象 ID。"}).to_string(); }

    // ★ 单个 ID：直接返回详情
    if ids.len() == 1 {
        let metadata = build_chunk_metadata(&ids[0]);
        if !metadata["found"].as_bool().unwrap_or(false) {
            return json!({
                "result_type": "object_detail",
                "data": {"id": ids[0], "found": false}
            }).to_string();
        }
        let source_data_id = metadata["source_data_id"].as_str().unwrap_or("");
        let source_type = metadata["type"].as_str().unwrap_or("");
        let source_id = metadata["id"].as_str().unwrap_or("");
        let chunks = fetch_parent_chunks(source_data_id, source_type, source_id, visited_chunks, visited_sources);
        return json!({
            "result_type": "object_detail",
            "data": {
                "id": metadata["id"],
                "type": metadata["type"],
                "name": metadata["name"],
                "found": true,
                "total_chunks": chunks["total_chunks"],
                "chunks": chunks["chunks"]
            }
        }).to_string();
    }

    let limit = ids.len().min(30);
    let ids = &ids[..limit];
    log::info!("[batch_recall] 收到 {} 个ID: {:?}", ids.len(), ids.iter().take(3).collect::<Vec<_>>());
    
    // 并发获取每个对象的来源信息和 chunks
    let mut handles = Vec::new();
    for id in ids.iter() {
        let id = id.clone();
        let vc = visited_chunks.clone();
        let vs = visited_sources.clone();
        handles.push(tokio::spawn(async move {
            let metadata = build_chunk_metadata(&id);
            if !metadata["found"].as_bool().unwrap_or(false) {
                return json!({"id": id, "found": false});
            }
            let source_data_id = metadata["source_data_id"].as_str().unwrap_or("");
            let source_type = metadata["type"].as_str().unwrap_or("");
            let source_id = metadata["id"].as_str().unwrap_or("");
            let chunks = fetch_parent_chunks(source_data_id, source_type, source_id, &vc, &vs);
            json!({
                "id": metadata["id"],
                "type": metadata["type"],
                "name": metadata["name"],
                "found": true,
                "total_chunks": chunks["total_chunks"],
                "chunks": chunks["chunks"]
            })
        }));
    }
    
    // 收集结果
    let mut objects: Vec<serde_json::Value> = Vec::new();
    for h in handles {
        if let Ok(obj) = h.await {
            objects.push(obj);
        }
    }
    
    // 返回纯 JSON
    json!({
        "result_type": "object_detail_batch",
        "data": {"objects": objects},
        "summary": format!("批量查询 {} 个对象", objects.len())
    }).to_string()
}

/// 解析项目 ID：支持按 ID 或名称查找项目
fn resolve_project_id(project_id: &str) -> Option<String> {
    let raw_pid = project_id.trim();
    let clean_pid = raw_pid.strip_prefix("PROJECT-").unwrap_or(raw_pid);
    if clean_pid.is_empty() {
        return None;
    }
    // 先按 ID 查找
    if let Ok(Some(_)) = database::get("SELECT 1 FROM projects WHERE id=?1", &[json!(clean_pid)]) {
        return Some(clean_pid.to_string());
    }
    // 再按名称查找
    database::get(
        "SELECT id FROM projects WHERE name LIKE ?1 ESCAPE '\\' ORDER BY LENGTH(name) ASC LIMIT 1",
        &[json!(format!("%{}%", crate::database::escape_like(clean_pid)))],
    )
    .ok()
    .flatten()
    .and_then(|r| r["id"].as_str().map(String::from))
}

fn ext_tool_manage_todo(action: &str, title: &str, content: &str, priority: &str, due_date: &str, repeat: &str, project_id: &str, done: bool, todo_id: &str) -> String {
    let normalize_pri = |p: &str| -> Option<String> {
        if p.trim().is_empty() { return None; }
        Some(match p.trim() {
            "最高"|"高"|"high"|"urgent"|"紧急"|"P1"|"p1" => "P1".to_string(),
            "中"|"medium"|"normal"|"一般"|"P2"|"p2" => "P2".to_string(),
            "低"|"low"|"P3"|"p3" => "P3".to_string(),
            other => other.to_string(),
        })
    };
    // 通过 todo_id 或 title 模糊匹配查找待办
    let find_todo = |tid: &str, t: &str| -> Option<(String, String)> {
        if !tid.trim().is_empty() {
            let clean_id = tid.trim().strip_prefix("TODO-").unwrap_or(tid.trim());
            database::get("SELECT id,title FROM todos WHERE id=?1", &[json!(clean_id)])
                .ok().flatten().map(|r| (r["id"].as_str().unwrap_or("").to_string(), r["title"].as_str().unwrap_or("").to_string()))
        } else if !t.trim().is_empty() {
            let like = format!("%{}%", crate::database::escape_like(t.trim()));
            database::get("SELECT id,title FROM todos WHERE title LIKE ?1 ESCAPE '\\' ORDER BY created_at DESC LIMIT 1", &[json!(&like)])
                .ok().flatten().map(|r| (r["id"].as_str().unwrap_or("").to_string(), r["title"].as_str().unwrap_or("").to_string()))
        } else { None }
    };
    match action {
        "create" => {
            if title.trim().is_empty() {
                let envelope = ToolResultEnvelope {
                    result_type: "todo_create_result".to_string(),
                    data: json!({"success": false}),
                    summary: Some("创建待办需要提供 title。".to_string()),
                };
                return envelope.to_json();
            }
            let p = normalize_pri(priority);
            let d = if due_date.trim().is_empty() { None } else { Some(due_date.trim()) };
            let r = if repeat.trim().is_empty() { None } else { Some(repeat.trim()) };
            let pid = resolve_project_id(project_id);
            let data = json!({
                "title": title.trim(),
                "content": content.trim(),
                "priority": p,
                "dueDate": d,
                "repeat": r,
                "projectId": pid
            });
            // 使用 block_on 调用异步函数
            match block_on_async(crate::commands::todo_commands::todos_create(data)) {
                Ok(result) => {
                    let todo_id = result["todo"]["id"].as_str().unwrap_or("");
                    let todo_title = result["todo"]["title"].as_str().unwrap_or(title.trim());
                    // ★ 双写 business_relations
                    if let Some(ref pid) = pid {
                        if !todo_id.is_empty() {
                            crate::services::business_relations::upsert("TODO", todo_id, "PROJECT", pid, "belongs_to", "{}");
                        }
                    }
                    if !todo_id.is_empty() && pid.is_some() {
                        log::info!("[待办] 关联项目: todo_id={}, project_id={:?}", todo_id, pid);
                    }
                    let envelope = ToolResultEnvelope {
                        result_type: "object_create_result".to_string(),
                        data: json!({"id": todo_id, "type": "TODO", "name": todo_title}),
                        summary: Some(format!("已创建待办「{}」", todo_title)),
                    };
                    envelope.to_json()
                },
                Err(e) => {
                    let envelope = ToolResultEnvelope {
                        result_type: "object_create_result".to_string(),
                        data: json!({"error": e.to_string()}),
                        summary: Some(format!("创建待办失败: {}", e)),
                    };
                    envelope.to_json()
                },
            }
        }
        "update" => {
            let found = find_todo(todo_id, title);
            match found {
                Some((fid, ftitle)) => {
                    let mut data = json!({});
                    if !content.trim().is_empty() { data["content"] = json!(content.trim()); }
                    if !priority.trim().is_empty() { data["priority"] = json!(normalize_pri(priority).unwrap_or_default()); }
                    if !due_date.trim().is_empty() { data["dueDate"] = json!(due_date.trim()); }
                    if !repeat.trim().is_empty() { data["repeat"] = json!(repeat.trim()); }
                    if done { data["done"] = json!(true); }
                    // 处理项目关联
                    if !project_id.trim().is_empty() {
                        data["projectId"] = json!(resolve_project_id(project_id));
                    }
                    if data.as_object().map_or(true, |m| m.is_empty()) {
                        let envelope = ToolResultEnvelope {
                            result_type: "object_update_result".to_string(),
                            data: json!({"id": fid, "error": "更新待办需要提供要修改的字段"}),
                            summary: Some("更新待办需要提供要修改的字段。".to_string()),
                        };
                        return envelope.to_json();
                    }
                    // 使用 block_on 调用异步函数
                    match block_on_async(crate::commands::todo_commands::todos_update(fid.clone(), data)) {
                        Ok(_) => {
                            // ★ 双写 business_relations（如果有关联项目）
                            if !project_id.trim().is_empty() {
                                if let Some(ref pid) = resolve_project_id(project_id) {
                                    crate::services::business_relations::upsert("TODO", &fid, "PROJECT", pid, "belongs_to", "{}");
                                }
                            }
                            let envelope = ToolResultEnvelope {
                                result_type: "object_update_result".to_string(),
                                data: json!({"id": fid, "type": "TODO", "name": ftitle}),
                                summary: Some(format!("已更新待办「{}」", ftitle)),
                            };
                            envelope.to_json()
                        }
                        Err(e) => {
                            let envelope = ToolResultEnvelope {
                                result_type: "object_update_result".to_string(),
                                data: json!({"id": fid, "error": e.to_string()}),
                                summary: Some(format!("更新待办失败: {}", e)),
                            };
                            envelope.to_json()
                        },
                    }
                }
                None => {
                    let envelope = ToolResultEnvelope {
                        result_type: "object_update_result".to_string(),
                        data: json!({"error": "未找到匹配的待办"}),
                        summary: Some("未找到匹配的待办。".to_string()),
                    };
                    envelope.to_json()
                },
            }
        }
        "delete" => {
            let found = find_todo(todo_id, title);
            match found {
                                Some((fid, ftitle)) => {
                                    // 使用 block_on 调用异步函数
                                    match block_on_async(crate::commands::todo_commands::todos_delete(fid.clone())) {
                        Ok(_) => {
                            let envelope = ToolResultEnvelope {
                                result_type: "object_delete_result".to_string(),
                                data: json!({"id": fid, "type": "TODO", "name": ftitle}),
                                summary: Some(format!("已删除待办「{}」", ftitle)),
                            };
                            envelope.to_json()
                        }
                        Err(e) => {
                            let envelope = ToolResultEnvelope {
                                result_type: "object_delete_result".to_string(),
                                data: json!({"id": fid, "error": e.to_string()}),
                                summary: Some(format!("删除待办失败: {}", e)),
                            };
                            envelope.to_json()
                        },
                    }
                }
                None => {
                    let envelope = ToolResultEnvelope {
                        result_type: "object_delete_result".to_string(),
                        data: json!({"error": "未找到匹配的待办"}),
                        summary: Some("未找到匹配的待办。".to_string()),
                    };
                    envelope.to_json()
                },
            }
        }
        _ => {
            let envelope = ToolResultEnvelope {
                result_type: "object_error".to_string(),
                data: json!({"error": format!("未知操作「{}」。支持：create, update, delete。", action)}),
                summary: Some(format!("未知操作「{}」。支持：create, update, delete。", action)),
            };
            envelope.to_json()
        },
    }
}

fn ext_tool_memory(action: &str, content: &str, items_json: &str, memory_id: &str, query: &str, keywords_json: &str, start_time: &str, end_time: &str) -> String {
    match action {
        "create" => {
            let contents: Vec<String> = if !items_json.trim().is_empty() {
                serde_json::from_str::<Vec<String>>(items_json)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|s| !s.trim().is_empty())
                    .collect()
            } else {
                let s = content.trim().to_string();
                if s.is_empty() {
                    let envelope = ToolResultEnvelope {
                        result_type: "memory_create_result".to_string(),
                        data: json!({"created": 0, "ids": []}),
                        summary: Some("创建记忆需要提供 content 或 items。".to_string()),
                    };
                    return envelope.to_json();
                }
                vec![s]
            };
            let mut created = 0;
            let mut ids: Vec<String> = Vec::new();
            for statement in &contents {
                let id = uuid::Uuid::new_v4().to_string();
                if let Ok(r) = database::run(
                    "INSERT INTO memories(id,content,updated_at) VALUES(?1,?2,datetime('now','localtime'))",
                    &[json!(&id), json!(statement)],
                ) {
                    if r.changes > 0 {
                        created += 1;
                        ids.push(id);
                    }
                }
            }
            let summary = if created == 1 {
                let short_id = &ids[0];
                format!("已记住 1 条事实，ID: {}", short_id)
            } else {
                let short_ids: Vec<&str> = ids.iter().map(|id| id.as_str()).collect();
                format!("已记住 {} 条事实，ID: {}", created, short_ids.join(", "))
            };
            let envelope = ToolResultEnvelope {
                result_type: "memory_create_result".to_string(),
                data: json!({"created": created, "ids": ids}),
                summary: Some(summary),
            };
            envelope.to_json()
        }
        "update" => {
            let mid = memory_id.trim();
            if mid.is_empty() {
                let envelope = ToolResultEnvelope {
                    result_type: "memory_update_result".to_string(),
                    data: json!({"success": false, "memory_id": null}),
                    summary: Some("更新需要提供 memory_id。".to_string()),
                };
                return envelope.to_json();
            }
            let c = content.trim();
            if c.is_empty() {
                let envelope = ToolResultEnvelope {
                    result_type: "memory_update_result".to_string(),
                    data: json!({"success": false, "memory_id": mid}),
                    summary: Some("更新需要提供 content。".to_string()),
                };
                return envelope.to_json();
            }
            let sid = mid;
            let (success, summary) = match database::run("UPDATE memories SET content=?1, updated_at=datetime('now','localtime') WHERE id=?2", &[json!(c), json!(sid)]) {
                Ok(r) if r.changes > 0 => (true, format!("已更新记忆: {}", sid)),
                Err(e) => (false, format!("更新失败: {}", e)),
                _ => (false, format!("未找到记忆: {}", sid)),
            };
            let envelope = ToolResultEnvelope {
                result_type: "memory_update_result".to_string(),
                data: json!({"success": success, "memory_id": mid}),
                summary: Some(summary),
            };
            envelope.to_json()
        }
        "delete" => {
            if !memory_id.trim().is_empty() {
                let sid = memory_id.trim().to_string();
                let (success, summary) = match database::run("DELETE FROM memories WHERE id=?1", &[json!(sid)]) {
                    Ok(r) if r.changes > 0 => (true, format!("已删除记忆: {}", sid)),
                    Err(e) => (false, format!("删除失败: {}", e)),
                    _ => (false, format!("未找到记忆: {}", sid)),
                };
                let envelope = ToolResultEnvelope {
                    result_type: "memory_delete_result".to_string(),
                    data: json!({"success": success, "memory_id": memory_id}),
                    summary: Some(summary),
                };
                envelope.to_json()
            } else {
                let envelope = ToolResultEnvelope {
                    result_type: "memory_delete_result".to_string(),
                    data: json!({"success": false, "memory_id": null}),
                    summary: Some("删除需要提供 memory_id。".to_string()),
                };
                envelope.to_json()
            }
        }
        "search" => {
            let q = query.trim();
            let start = start_time.trim();
            let end = end_time.trim();
            let search_words: Vec<String> = if !keywords_json.is_empty() {
                serde_json::from_str::<Vec<String>>(keywords_json).unwrap_or_default()
            } else if !q.is_empty() {
                q.split_whitespace().map(String::from).collect()
            } else {
                vec![]
            };
            
            // ★ 检测是否为"列出所有"模式
            let fetch_all = search_words.is_empty() 
                || search_words.iter().all(|w| w == "*" || w == "all" || w == "全部");
            
            let mut sql = String::from("SELECT id, content, created_at FROM memories");
            let mut params: Vec<serde_json::Value> = Vec::new();
            let mut param_idx = 1;
            let mut where_clauses: Vec<String> = Vec::new();
            
            if !fetch_all && !search_words.is_empty() {
                let fts_query = search_words.iter().map(|w| format!("content:{}", w)).collect::<Vec<_>>().join(" ");
                let fts_sql = "SELECT rowid FROM memories_fts WHERE memories_fts MATCH ?1 ORDER BY bm25(memories_fts) LIMIT 10";
                if let Ok(fts_rows) = database::all(fts_sql, &[json!(&fts_query)]) {
                    let rowids: Vec<i64> = fts_rows.iter().filter_map(|r| r["rowid"].as_i64()).collect();
                    if !rowids.is_empty() {
                        let placeholders: Vec<String> = rowids.iter().enumerate().map(|(i, _)| format!("?{}", param_idx + i)).collect();
                        where_clauses.push(format!("rowid IN ({})", placeholders.join(",")));
                        for rid in rowids { params.push(json!(rid)); }
                        param_idx += placeholders.len();
                    }
                }
                if where_clauses.is_empty() {
                    for w in &search_words {
                        let like = format!("%{}%", crate::database::escape_like(w));
                        where_clauses.push(format!("content LIKE ?{} ESCAPE '\\'", param_idx));
                        params.push(json!(like));
                        param_idx += 1;
                    }
                }
            }
            if !start.is_empty() {
                where_clauses.push(format!("date(created_at) >= ?{}", param_idx));
                params.push(json!(start));
                param_idx += 1;
            }
            if !end.is_empty() {
                where_clauses.push(format!("date(created_at) <= ?{}", param_idx));
                params.push(json!(end));
            }
            if !where_clauses.is_empty() {
                sql.push_str(" WHERE ");
                sql.push_str(&where_clauses.join(" AND "));
            }
            sql.push_str(" ORDER BY created_at DESC LIMIT 20");
            match database::all(&sql, &params) {
                Ok(rows) => {
                    let memories: Vec<Value> = rows.iter().map(|r| {
                        let sid = r["id"].as_str().unwrap_or("");
                        let content = r["content"].as_str().unwrap_or("");
                        json!({
                            "memory_id": sid.to_string(),
                            "content": content
                        })
                    }).collect();
                    
                    let summary = if memories.is_empty() {
                        if q.is_empty() { "暂无记忆。".to_string() }
                        else { format!("未找到匹配「{}」的记忆。", q) }
                    } else {
                        format!("找到 {} 条记忆。", memories.len())
                    };
                    
                    let envelope = ToolResultEnvelope {
                        result_type: "memory_search_result".to_string(),
                        data: json!({
                            "action": "search",
                            "total": memories.len(),
                            "memories": memories
                        }),
                        summary: Some(summary),
                    };
                    envelope.to_json()
                }
                Err(e) => {
                    let envelope = ToolResultEnvelope {
                        result_type: "memory_search_result".to_string(),
                        data: json!({"action": "search", "total": 0, "memories": []}),
                        summary: Some(format!("搜索记忆失败: {}", e)),
                    };
                    envelope.to_json()
                }
            }
        }
        _ => {
            let envelope = ToolResultEnvelope {
                result_type: "memory_unknown".to_string(),
                data: json!({}),
                summary: Some(format!("未知操作「{}」。支持：create, update, delete, search。", action)),
            };
            envelope.to_json()
        }
    }
}

fn resolve_time(s: &str) -> String {
    let s = s.trim();
    if s.is_empty() { return String::new(); }
    if s.len() >= 10 && s.contains('-') { return s[..10].to_string(); }
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    // Handle relative expressions via simple matching
    if s.contains("昨天") || s.contains("昨天") { let d = chrono::Local::now() - chrono::Duration::days(1); return d.format("%Y-%m-%d").to_string(); }
    if s.contains("前天") { let d = chrono::Local::now() - chrono::Duration::days(2); return d.format("%Y-%m-%d").to_string(); }
    if s.contains("今天") || s.is_empty() { return today; }
    // fallback: return as-is
    s.to_string()
}

fn fetch_parent_chunks(source_data_id: &str, source_type: &str, source_id: &str, visited_chunks: &std::collections::HashSet<String>, visited_sources: &std::collections::HashSet<String>) -> serde_json::Value {
    log::info!("[fetch_parent_chunks] 查询: source_data_id={}, type={}, id={}", source_data_id, source_type, source_id);

    // 检查是否已在前文展示过
    let source_full_id = format!("{}-{}", source_type, source_id);
    let rows = database::all("SELECT chunk_id, content, chunk_index FROM knowledge_chunks WHERE source_data_id=?1 AND parent_id IS NULL ORDER BY chunk_index LIMIT 3", &[json!(source_data_id)]).unwrap_or_default();
    let chunk_ids: Vec<String> = rows.iter().map(|r| r["chunk_id"].as_str().unwrap_or("").chars().take(8).collect()).collect();
    if !chunk_ids.is_empty() && chunk_ids.iter().all(|id| visited_chunks.contains(id)) && visited_sources.contains(&source_full_id) {
        return json!({
            "found": true,
            "already_shown": true,
            "total_chunks": 0,
            "chunks": []
        });
    }

    // 查询 chunk 总数
    let total_chunks: i64 = database::get(
        "SELECT COUNT(*) as cnt FROM knowledge_chunks WHERE source_data_id=?1 AND parent_id IS NULL",
        &[json!(source_data_id)]
    ).ok().flatten().and_then(|r| r["cnt"].as_i64()).unwrap_or(0);

    let chunks: Vec<serde_json::Value> = rows.iter().enumerate().map(|(_i, r)| {
        let content = r["content"].as_str().unwrap_or("");
        let chunk_id = r["chunk_id"].as_str().unwrap_or("");
        let content_truncated = content;
        json!({
            "index": r["chunk_index"].as_i64().unwrap_or(0) + 1,
            "chunk_id": chunk_id.chars().take(8).collect::<String>(),
            "content": content_truncated
        })
    }).collect();

    json!({
        "found": true,
        "already_shown": false,
        "total_chunks": total_chunks,
        "chunks": chunks
    })
}

fn relative_time(created_at: &str) -> String {
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

/// 大附件切片提炼阈值：单附件超过该字符数则不再截断，改为 LLM 提炼关键信息
const ATTACH_SUMMARIZE_THRESHOLD: usize = 100000;
/// 提炼切片大小（字符）
const ATTACH_SUMMARIZE_CHUNK: usize = 6000;
/// 提炼片数上限（防病态超大文件打爆 API）
const ATTACH_SUMMARIZE_MAX_CHUNKS: usize = 30;
/// 提炼提示词：通用关键信息提炼，不限定领域
const ATTACH_SUMMARIZE_PROMPT: &str = "你是信息提炼助手。请从以下文件片段中提炼并保留关键信息，删除冗余表述，用简洁的中文输出精炼摘要。";

/// 大附件切片提炼：split → 每片 LLM 提炼 → 按序汇总
/// 超过 ATTACH_SUMMARIZE_THRESHOLD 字符的附件不再截断，而是切片后由 LLM 提炼关键信息。
/// 最终以「片段N摘要」块拼接输出，体积远小于原文且不丢失关键内容。
async fn summarize_large_attachment(
    text: &str,
    app_handle: &Option<tauri::AppHandle>,
) -> String {
    let chunks = crate::services::smart_chunker::split_into_parents(text, ATTACH_SUMMARIZE_CHUNK);
    let total = chunks.len().min(ATTACH_SUMMARIZE_MAX_CHUNKS);

    // 预收集为 owned 数据，避免 futures::stream::iter 的借用生命周期冲突
    let chunk_data: Vec<(usize, String)> = chunks.iter().take(total).enumerate()
        .map(|(i, s)| (i, s.clone()))
        .collect();

    let results: Vec<String> = futures::stream::iter(chunk_data)
        .map(|(i, chunk)| async move {
            if let Some(ref h) = app_handle {
                let _ = h.emit("agent:progress", serde_json::json!({"msg": format!("正在提炼附件… {}/{}", i + 1, total)}));
            }
            let msgs = vec![
                ChatMessage { role: MessageRole::System, content: Some(ATTACH_SUMMARIZE_PROMPT.to_string()), tool_calls: None, tool_call_id: None, name: None, reasoning_content: None },
                ChatMessage { role: MessageRole::User, content: Some(chunk.clone()), tool_calls: None, tool_call_id: None, name: None, reasoning_content: None },
            ];
            match call_llm_streaming(&msgs, None, app_handle).await {
                Ok((content, ..)) => content.trim().to_string(),
                Err(e) => {
                    log::warn!("[大附件提炼] 片段 {} 提炼失败: {}", i + 1, e);
                    let cut = crate::services::file_parser::truncate_at_boundary(&chunk, 2000);
                    format!("（片段提炼失败: {}）\n{}", e, cut)
                }
            }
        })
        .buffered(3)
        .collect::<Vec<_>>()
        .await;

    let mut summary = results.into_iter().enumerate()
        .map(|(i, s)| format!("【片段{}】\n{}", i + 1, s))
        .collect::<Vec<_>>()
        .join("\n\n");

    // 超出 30 片的部分：结构截断追加（带提示），避免丢失
    if chunks.len() > ATTACH_SUMMARIZE_MAX_CHUNKS {
        let rest = chunks[ATTACH_SUMMARIZE_MAX_CHUNKS..].join("\n");
        let cut = crate::services::file_parser::truncate_at_boundary(&rest, 4000);
        summary.push_str(&format!("\n\n（其余 {} 片内容较长已略过）\n{}", chunks.len() - ATTACH_SUMMARIZE_MAX_CHUNKS, cut));
    }

    if summary.trim().is_empty() { "（大附件提炼结果为空）".to_string() } else { summary }
}

#[tauri::command]
pub async fn agent_chat_workflow(user_query: String, attachments: Vec<Value>, max_iterations: Option<i32>, app_handle: Option<tauri::AppHandle>, silent: bool, session_id: Option<String>, persona_suffix: Option<String>, tool_mode: ToolMode) -> Result<Value, String> {
    AGENT_CANCEL.store(false, Ordering::Release);
    let system_prompt = match persona_suffix.as_deref() {
        Some(s) if !s.trim().is_empty() => format!("{}\n\n{}", AGENT_SYSTEM_PROMPT, s),
        _ => AGENT_SYSTEM_PROMPT.to_string(),
    };
    log::info!("workflow start, app_some={}", app_handle.is_some());
    let query = user_query.trim().to_string();
    let max_iters = max_iterations.unwrap_or(8).clamp(2, 20);
    if query.is_empty() { return Ok(json!({"success":true,"result_type":"answer","answer":"请输入问题。","iterations":0})); }
    crate::services::chat_api::invalidate_chat_config_cache();
    { let mut c = API_CACHE.write().unwrap_or_else(|e| e.into_inner()); c.3 = Instant::now() - std::time::Duration::from_secs(60); }
    let (_, api_key, _model) = get_api_endpoint();
    if api_key.is_empty() { return Ok(json!({"success":true,"result_type":"answer","answer":"请先在系统设置中配置 Chat API 密钥。"})); }

    let tools = build_tools_definitions(tool_mode);
    // ★ Session 缓存：从 chat_sessions.session_state 恢复（始终遍历所有历史）
    let mut session = if let Some(ref sid) = session_id {
        load_session_state(sid).unwrap_or_else(|| {
            log::info!("[Session] 无缓存，从头创建");
            SessionMemory::new()
        })
    } else {
        SessionMemory::new()
    };
    // ★ 记录加载时的快照，save 时只保存本次提问新增的 visited（差集）
    session.prev_visited_chunks = session.visited_chunks.clone();
    session.prev_visited_sources = session.visited_sources.clone();

    // ★ 从 DB 加载历史消息
    // 读取 session 的压缩摘要（如果有）
    let compressed_summary: Option<String> = if let Some(ref sid) = session_id {
        let session_row = crate::database::get(
            "SELECT context_metadata FROM chat_sessions WHERE id=?1",
            &[json!(sid)],
        ).unwrap_or_default();
        if let Some(meta_str) = session_row.and_then(|r| r["context_metadata"].as_str().map(|s| s.to_string())) {
            if let Ok(meta) = serde_json::from_str::<Value>(&meta_str) {
                meta["summary"].as_str().map(|s| s.to_string())
            } else { None }
        } else { None }
    } else { None };

    let history: Vec<Value> = if let Some(ref sid) = session_id {
        let rows = crate::database::all(
            "SELECT role, content, created_at FROM chat_messages WHERE session_id=?1 AND status='active' ORDER BY id",
            &[json!(sid)],
        ).unwrap_or_default();

        log::info!("[AgentWorkflow] 从 DB 加载 {} 条 active 消息, 压缩摘要: {}",
            rows.len(), compressed_summary.is_some());

        rows.into_iter().filter_map(|r| {

            let role = r["role"].as_str().unwrap_or("");
            let db_content = r["content"].as_str().unwrap_or("");
            let created_at = r["created_at"].as_str().unwrap_or("");

            if db_content.is_empty() { return None; }

            // 为 user 和 assistant 消息添加时间戳（格式与发送时一致，保证 prompt cache 命中）
            let time_str = created_at.get(11..19).unwrap_or("");
            let content_with_timestamp = if role == "user" && !time_str.is_empty() {
                format!("时间：{}\n提问：{}", time_str, db_content)
            } else if role == "assistant" && !time_str.is_empty() {
                format!("时间：{}\n回答：{}", time_str, db_content)
            } else {
                db_content.to_string()
            };

            // 历史消息纯内容，不含 metadata（tool_metadata 在 session 级别）
            let msg = json!({"role": role, "content": content_with_timestamp});
            Some(msg)
        }).collect()
    } else {
        vec![]
    };

    let mut messages: Vec<ChatMessage> = vec![
        ChatMessage{role:MessageRole::System,content:Some(system_prompt.clone()),tool_calls:None,tool_call_id:None,name:None,reasoning_content:None},
    ];

    // ★ 注入压缩摘要（如果有）
    if let Some(ref summary) = compressed_summary {
        let summary_text = format!("【历史摘要】\n{}", summary);
        messages.push(ChatMessage{role:MessageRole::Assistant,content:Some(summary_text),tool_calls:None,tool_call_id:None,name:None,reasoning_content:None});
        log::info!("[AgentWorkflow] 注入压缩摘要: {}字", summary.len());
    }
    // ★ 恢复历史消息
    for h in &history {
        let role = h["role"].as_str().unwrap_or("");
        match role {
            "assistant" => {
                let content = h["content"].as_str().map(|s| s.to_string());
                if let Some(ref text) = content { session.record_observation(text); }
                messages.push(ChatMessage{role:MessageRole::Assistant,content,tool_calls:None,tool_call_id:None,name:None,reasoning_content:None});
            }
            _ => {
                let content = h["content"].as_str().map(|s| s.to_string());
                messages.push(ChatMessage{role:MessageRole::User,content,tool_calls:None,tool_call_id:None,name:None,reasoning_content:None});
            }
        }
    }

    // ★ 从 session.tool_metadata 注入最新轮次的工具调用和结果
    if let Some(ref sid) = session_id {
        if let Ok(Some(row)) = crate::database::get("SELECT tool_metadata FROM chat_sessions WHERE id=?1", &[json!(sid)]) {
            if let Some(meta_str) = row["tool_metadata"].as_str().filter(|s| !s.is_empty()) {
                if let Ok(meta) = serde_json::from_str::<Value>(meta_str) {
                    // 注入 tool_calls 为 assistant 消息
                    let tcs: Option<Vec<ToolCall>> = meta.get("tool_calls")
                        .and_then(|v| v.as_array())
                        .filter(|arr| !arr.is_empty())
                        .map(|arr| arr.iter().filter_map(|tc| {
                            let id = tc["id"].as_str()?;
                            let name = tc["function"]["name"].as_str()?;
                            let args_str = tc["function"]["arguments"].as_str().unwrap_or("");
                            let args = if args_str.is_empty() {
                                serde_json::to_string(&tc["function"]["arguments"]).unwrap_or_default()
                            } else { args_str.to_string() };
                            Some(ToolCall { id: id.to_string(), call_type: "function".into(), function: FunctionCall { name: name.to_string(), arguments: args } })
                        }).collect());
                    let has_tcs = tcs.as_ref().map(|v| !v.is_empty()).unwrap_or(false);
                    let tcs = tcs.filter(|v| !v.is_empty());
                    if has_tcs {
                        messages.push(ChatMessage{role:MessageRole::Assistant,content:None,tool_calls:tcs,tool_call_id:None,name:None,reasoning_content:None});
                    }
                    // 注入 tool_results 为 tool 消息
                    if has_tcs {
                        if let Some(results) = meta.get("tool_results").and_then(|v| v.as_array()) {
                            let mut grouped: std::collections::HashMap<String, (String, String)> = std::collections::HashMap::new();
                            for tr in results {
                                let tr_id = tr["tool_call_id"].as_str().unwrap_or("").to_string();
                                let tr_name = tr["name"].as_str().unwrap_or("").to_string();
                                let tr_content = tr["content"].as_str().unwrap_or("");
                                let entry = grouped.entry(tr_id).or_insert((String::new(), tr_name));
                                if !entry.0.is_empty() { entry.0.push_str("\n\n---\n\n"); }
                                entry.0.push_str(tr_content);
                            }
                            for (tr_id, (tr_content, tr_name)) in grouped {
                                messages.push(ChatMessage{role:MessageRole::Tool,content:Some(tr_content.clone()),tool_calls:None,tool_call_id:Some(tr_id),reasoning_content:None,name:Some(tr_name.clone())});
                                session.update_from_tool_result(&tr_name, &tr_content);
                            }
                        }
                    }
                }
            }
        }
    }
    // ★ 保存 user 消息到 DB（历史加载完成后，避免重复）
    // ★ 解析附件并构建增强消息
    let augmented_query = if !attachments.is_empty() {
        if let Some(ref h) = app_handle { let _ = h.emit("agent:progress", serde_json::json!({"msg":"正在解析附件..."})); }
        let mut blocks = Vec::new();
        let mut total_bytes = 0usize;
        const TOTAL_CAP: usize = 100000;
        for att in &attachments {
            let path = att["path"].as_str().unwrap_or("");
            let name = att["name"].as_str().unwrap_or("未命名文件");
            if path.is_empty() { continue; }
            let ext = std::path::Path::new(path).extension()
                .and_then(|e| e.to_str()).unwrap_or("").to_string();
            match crate::services::file_parser::parse_chat_attachment(path, &ext).await {
                Ok((_kind, text)) if !text.is_empty() => {
                    if text.len() > ATTACH_SUMMARIZE_THRESHOLD {
                        // ★ 大附件：切片提炼，不截断
                        if let Some(ref h) = app_handle {
                            let _ = h.emit("agent:progress", serde_json::json!({"msg": format!("检测到超大附件「{}」，正在切片提炼…", name)}));
                        }
                        let distilled = summarize_large_attachment(&text, &app_handle).await;
                        total_bytes += distilled.len();
                        blocks.push(format!("【附件：{}（已提炼）】\n{}", name, distilled));
                    } else if total_bytes + text.len() > TOTAL_CAP {
                        // 超出总预算：结构感知截断（保留表头/段落边界），而非整段跳过
                        let remaining = TOTAL_CAP.saturating_sub(total_bytes);
                        let cut = crate::services::file_parser::truncate_at_boundary(&text, remaining);
                        total_bytes += cut.len();
                        blocks.push(format!("【附件：{}】\n{}", name, cut));
                    } else {
                        total_bytes += text.len();
                        blocks.push(format!("【附件：{}】\n{}", name, text));
                    }
                }
                Ok(_) => { blocks.push(format!("【附件：{}】\n（文件解析为空）", name)); }
                Err(e) => { blocks.push(format!("【附件：{}】\n（解析失败: {}）", name, e)); }
            }
        }
        if blocks.is_empty() {
            query.clone()
        } else {
            format!("{}\n\n<<<ATTACHMENTS>>>\n{}", query, blocks.join("\n\n"))
        }
    } else {
        query.clone()
    };
    if let Some(ref sid) = session_id {
        let _ = crate::database::run(
            "INSERT INTO chat_messages(session_id, role, content) VALUES(?1, 'user', ?2)",
            &[json!(sid), json!(&augmented_query)],
        );
    }

    // ★ 追加本轮新问题（带时间戳前缀，与历史加载格式一致，保证 LLM prompt cache 命中）
    let now = chrono::Local::now().format("%H:%M:%S").to_string();
    let timestamped = format!("时间：{}\n提问：{}", now, augmented_query);
    messages.push(ChatMessage{role:MessageRole::User,content:Some(timestamped),tool_calls:None,tool_call_id:None,name:None,reasoning_content:None});

    // ★ 上下文压缩：复用会话上下文，LLM 返回 JSON 摘要
    // 单个会话上下文长度 256k，自动压缩阈值 80%（204,800 tokens）
    if let Some(ref sid) = session_id {
        let total_tokens: i64 = messages.iter().map(|m| {
            let content_len = m.content.as_ref().map(|s| s.len()).unwrap_or(0);
            let tc_len = m.tool_calls.as_ref().map(|tc| tc.len() * 100).unwrap_or(0);
            ((content_len + tc_len) as i64) / 3
        }).sum();
        if total_tokens > 204_800 {
            log::info!("[ContextCompressor] 当前上下文 token {} 超过阈值 204800 (256k×80%)，开始压缩...", total_tokens);
            match crate::services::context_compressor::compress_with_context(&mut messages, &app_handle).await {
                Ok(result) => {
                    log::info!("[ContextCompressor] 压缩成功: summary={}字", result.summary.len());
                    // 计算当前会话的总消息数量（保留调用兼容，参数已不使用）
                    let msg_count = crate::database::get(
                        "SELECT COUNT(*) as cnt FROM chat_messages WHERE session_id=?1",
                        &[json!(sid)],
                    ).ok().flatten().and_then(|r| r["cnt"].as_i64()).unwrap_or(0) as usize;
                    if let Err(e) = crate::services::context_compressor::save_compressed_to_db(sid, &result, msg_count) {
                        log::warn!("[ContextCompressor] 保存压缩结果失败: {}", e);
                    }
                    // 压缩后重新加载历史（从 DB 读取压缩后的状态）
                    messages.clear();
                    messages.push(ChatMessage{role:MessageRole::System,content:Some(system_prompt.clone()),tool_calls:None,tool_call_id:None,name:None,reasoning_content:None});

                    // 读取新的压缩摘要
                    let new_session_row = crate::database::get(
                        "SELECT context_metadata FROM chat_sessions WHERE id=?1",
                        &[json!(sid)],
                    ).unwrap_or_default();
                    if let Some(meta_str) = new_session_row.as_ref().and_then(|r| r["context_metadata"].as_str().map(|s| s.to_string())) {
                        if let Ok(meta) = serde_json::from_str::<Value>(&meta_str) {
                            if let Some(summary) = meta["summary"].as_str() {
                                let summary_text = format!("【历史摘要】\n{}", summary);
                                messages.push(ChatMessage{role:MessageRole::Assistant,content:Some(summary_text),tool_calls:None,tool_call_id:None,name:None,reasoning_content:None});
                                log::info!("[ContextCompressor] 注入压缩摘要: {}字", summary.len());
                            }
                        }
                    }

                    // 从 DB 重新加载近期消息（只加载 active 状态）
                    let recent_history: Vec<Value> = {
                        let rows = crate::database::all(
                            "SELECT role, content, created_at FROM chat_messages WHERE session_id=?1 AND status='active' ORDER BY id",
                            &[json!(sid)],
                        ).unwrap_or_default();
                        log::info!("[ContextCompressor] 从 DB 加载 {} 条 active 消息", rows.len());

                        rows.into_iter().filter_map(|r| {
                            let role = r["role"].as_str().unwrap_or("");
                            let db_content = r["content"].as_str().unwrap_or("");
                            let created_at = r["created_at"].as_str().unwrap_or("");

                            if db_content.is_empty() { return None; }

                            // 为 user 和 assistant 消息添加时间戳（格式与发送时一致）
                            let time_str = created_at.get(11..19).unwrap_or("");
                            let content_with_timestamp = if role == "user" && !time_str.is_empty() {
                                format!("时间：{}\n提问：{}", time_str, db_content)
                            } else if role == "assistant" && !time_str.is_empty() {
                                format!("时间：{}\n回答：{}", time_str, db_content)
                            } else {
                                db_content.to_string()
                            };

                            let msg = json!({"role": role, "content": content_with_timestamp});
                            Some(msg)
                        }).collect()
                    };
                    // ★ 恢复压缩后的近期消息
                    for h in &recent_history {
                        let role = h["role"].as_str().unwrap_or("");
                        match role {
                            "assistant" => {
                                let content = h["content"].as_str().map(|s| s.to_string());
                                messages.push(ChatMessage{role:MessageRole::Assistant,content,tool_calls:None,tool_call_id:None,name:None,reasoning_content:None});
                            }
                            _ => {
                                let content = h["content"].as_str().map(|s| s.to_string());
                                messages.push(ChatMessage{role:MessageRole::User,content,tool_calls:None,tool_call_id:None,name:None,reasoning_content:None});
                            }
                        }
                    }
                    // ★ 压缩后从 session.tool_metadata 注入工具调用
                    if let Ok(Some(tm_row)) = crate::database::get("SELECT tool_metadata FROM chat_sessions WHERE id=?1", &[json!(sid)]) {
                        if let Some(meta_str) = tm_row["tool_metadata"].as_str().filter(|s| !s.is_empty()) {
                            if let Ok(meta) = serde_json::from_str::<Value>(meta_str) {
                                let tcs: Option<Vec<ToolCall>> = meta.get("tool_calls")
                                    .and_then(|v| v.as_array())
                                    .filter(|arr| !arr.is_empty())
                                    .map(|arr| arr.iter().filter_map(|tc| {
                                        let id = tc["id"].as_str()?;
                                        let name = tc["function"]["name"].as_str()?;
                                        let args_str = tc["function"]["arguments"].as_str().unwrap_or("");
                                        let args = if args_str.is_empty() {
                                            serde_json::to_string(&tc["function"]["arguments"]).unwrap_or_default()
                                        } else { args_str.to_string() };
                                        Some(ToolCall { id: id.to_string(), call_type: "function".into(), function: FunctionCall { name: name.to_string(), arguments: args } })
                                    }).collect());
                                let has_tcs = tcs.as_ref().map(|v| !v.is_empty()).unwrap_or(false);
                                let tcs = tcs.filter(|v| !v.is_empty());
                                if has_tcs {
                                    messages.push(ChatMessage{role:MessageRole::Assistant,content:None,tool_calls:tcs,tool_call_id:None,name:None,reasoning_content:None});
                                }
                                if has_tcs {
                                    if let Some(results) = meta.get("tool_results").and_then(|v| v.as_array()) {
                                        let mut grouped: std::collections::HashMap<String, (String, String)> = std::collections::HashMap::new();
                                        for tr in results {
                                            let tr_id = tr["tool_call_id"].as_str().unwrap_or("").to_string();
                                            let tr_name = tr["name"].as_str().unwrap_or("").to_string();
                                            let tr_content = tr["content"].as_str().unwrap_or("");
                                            let entry = grouped.entry(tr_id).or_insert((String::new(), tr_name));
                                            if !entry.0.is_empty() { entry.0.push_str("\n\n---\n\n"); }
                                            entry.0.push_str(tr_content);
                                        }
                                        for (tr_id, (tr_content, tr_name)) in grouped {
                                            messages.push(ChatMessage{role:MessageRole::Tool,content:Some(tr_content),tool_calls:None,tool_call_id:Some(tr_id),reasoning_content:None,name:Some(tr_name)});
                                        }
                                    }
                                }
                            }
                        }
                    }
                    // 追加当前查询
                    messages.push(ChatMessage{role:MessageRole::User,content:Some(query.clone()),tool_calls:None,tool_call_id:None,name:None,reasoning_content:None});
                }
                Err(e) => {
                    log::warn!("[ContextCompressor] 压缩失败: {}", e);
                    return Err(format!("上下文压缩失败: {}", e));
                }
            }
        }
    }

    // ★ 从活跃对象和本轮 query 设置 Focus
    let top_entities: Vec<String> = {
        let mut sorted: Vec<&ActiveObject> = session.active_objects.iter().collect();
        sorted.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        sorted.iter().take(3).map(|o| o.name.clone()).collect()
    };
    session.set_focus_from_user_query(&query, &top_entities);

    let mut tool_log: Vec<Value> = Vec::new();
    let (mut iters, mut pt, mut _ct) = (0i32, 0i64, 0i64);
    let mut _cached_total: i64 = 0;
    let mut _prev_pt: i64 = 0;
    let mut budget = 150i32;  // 增加到150分
    let mut search_cost = 8i32;  // 降低初始消耗到8分
    let mut traverse_source_counter: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    
    // 加载用户策略（policy）— 从 policies 表读取
    let user_policies = {
        // 1. 加载默认输出规则
        let default_rules = crate::database::get(
            "SELECT content FROM policies WHERE id='default-output-rules'",
            &[],
        ).ok().flatten().and_then(|r| r["content"].as_str().map(|s| s.to_string()))
        .unwrap_or_default();

        // 2. 加载其他用户策略
        let other_policies = crate::database::all(
            "SELECT id, content, updated_at FROM policies WHERE id != 'default-output-rules' ORDER BY updated_at DESC",
            &[],
        ).unwrap_or_default();

        // 3. 拼接
        let mut parts = Vec::new();

        if !default_rules.is_empty() {
            parts.push(format!("【输出规范（必须遵守）】\n{}", default_rules));
        }

        if !other_policies.is_empty() {
            let lines: Vec<String> = other_policies.iter().filter_map(|r| {
                let sid = r["id"].as_str()?;
                let content = r["content"].as_str()?;
                let date = r["updated_at"].as_str().unwrap_or("").get(..10).unwrap_or("");
                Some(format!("[ID:{}] {} {}", sid, date, content))
            }).collect();
            if !lines.is_empty() {
                parts.push(format!("【用户策略】\n{}", lines.join("\n")));
            }
        }

        if parts.is_empty() {
            "无特殊策略要求。".to_string()
        } else {
            parts.join("\n\n")
        }
    };
    
    // System Prompt 已是纯静态（today + user_policies 移至动态上下文消息）
    messages[0].content = Some(system_prompt.clone());
    let mut tool_text_retry_count = 0u32; // 工具调用文本重试计数器
    let mut retry_count = 0u32; // ToolCall 清理后重试计数器

    loop {
        // ★ 检查停止信号
        if AGENT_CANCEL.load(Ordering::Acquire) {
            AGENT_CANCEL.store(false, Ordering::Release);
            log::info!("收到停止信号，中断执行");
            let ws = session.generate_scratchpad();
            return Ok(json!({"success":true,"result_type":"answer","answer":"已停止执行。","workspace":ws,"iterations":iters,"cancelled":true,"promptTokens":session.prompt_tokens,"completionTokens":session.completion_tokens,"cachedTokens":session.cached_tokens,"tool_log":tool_log}));
        }
        
        log::info!("预算: {}/150 cost={}", budget, search_cost);
        if !silent { if let Some(ref h) = app_handle { let _ = h.emit("agent:status", json!({"stage":"thinking","msg":"思考中..."})); let _ = h.emit("agent:progress", json!({"msg":"思考中..."})); } }
        // ★ 每轮注意力衰减（Residual 方式：new_score = old_score * 0.95 + gain）
        // gain 在 update_from_tool_result / UpdateObject effect 中通过 EMA 完成
        // 这里只做衰减，不做加分
        let prev_count = session.active_objects.len();
        for o in &mut session.active_objects {
            o.score *= 0.95;
        }
        // 淘汰：score < 0.1 且 最近 10 轮未访问
        session.active_objects.retain(|o| {
            o.score >= 0.1 || o.access_order >= session.access_counter.saturating_sub(10)
        });
        let removed = prev_count - session.active_objects.len();
        if removed > 0 { log::info!("[注意力] 衰减后移除 {} 个未命中对象，剩余 {}", removed, session.active_objects.len()); }
        iters += 1;


        // ★ 首轮/无工具轮：在 LLM 调用前注入动态上下文（后续轮次由循环末尾注入）
        {
            let scratchpad = session.generate_scratchpad();
            let status = if budget >= 100 { "预算充足" } else if budget >= 50 { "预算偏低" } else { "预算紧张，请优先总结已有信息" };
            // ★ 计算当前上下文 token 数
            let context_tokens: i64 = messages.iter().map(|m| {
                let text_len = m.content.as_ref().map_or(0, |s| s.len() as i64);
                let tc_len = m.tool_calls.as_ref().map_or(0, |tcs| tcs.iter()
                    .map(|tc| tc.function.arguments.len() as i64)
                    .sum::<i64>());
                (text_len + tc_len) / 3
            }).sum();
            let budget_info = format!(
                "当前会话工作记忆\n{}\n\n当前时间: {}\n\n搜索预算系统（单次对话）:\n- 总预算: 150 分\n- 计算方式: 每轮扣减，首次8分，逐轮×1.2递增（如8→9→11→13→...）\n- 当前轮次: 第{}轮，本轮将消耗: {}分\n- 剩余预算: {}/150 — {}\n- 当前上下文: {} tokens\n- 建议: 信息足够时主动回答，不要等到预算耗尽",
                scratchpad, chrono::Local::now().format("%Y-%m-%d %A %H:%M:%S").to_string(), iters, search_cost, budget, status, context_tokens
            );
            // 移除上一轮的动态上下文消息
            if let Some(pos) = messages.iter().rposition(|m| m.content.as_deref().map_or(false, |c| c.starts_with("[DYNAMIC_CONTEXT]"))) {
                messages.remove(pos);
            }
            messages.push(ChatMessage{role:MessageRole::User, content: Some(format!("[DYNAMIC_CONTEXT]\n{}\n\n{}", user_policies, budget_info)), tool_calls: None, tool_call_id: None, name: None, reasoning_content: None});
        }

        // ★ 调试：保存 context 到文件（供 debug_server.py 读取）—— 仅 debug 构建
        #[cfg(debug_assertions)]
        {
            let context_json: Vec<Value> = messages.iter().filter_map(|m| {
                serde_json::to_value(m).ok()
            }).collect();
            let _ = crate::commands::save_agent_context(context_json);
        }

        // ★ 单请求流式：content 逐 token 输出到前端，同时累积 tool_calls delta
        log::info!("[Agent] 调用 call_llm_streaming，messages 长度: {}", messages.len());
        let (ac_owned, reasoning, tcs_streamed, usage) = match call_llm_streaming(&messages, Some(&tools), &app_handle).await {
            Ok(r) => {
                log::info!("[Agent] call_llm_streaming 成功: content_len={}, tool_calls={}", r.0.len(), r.2.len());
                r
            }
            Err(e) => {
                log::error!("[Agent] call_llm_streaming 失败: {}", e);
                if let Some(ref h) = app_handle {
                    let _ = h.emit("agent:error", json!({"error": e}));
                }
                return Ok(json!({"success":false,"answer":format!("LLM 调用失败: {}", e),"iterations":iters}));
            }
        };
        if let Some(u) = usage {
            _prev_pt = pt;
            let round_pt = u["prompt_tokens"].as_i64().unwrap_or(0);
            let round_ct = u["completion_tokens"].as_i64().unwrap_or(0);
            let cached = u["prompt_tokens_details"]["cached_tokens"].as_i64().unwrap_or(0);
            pt += round_pt;
            _ct += round_ct;
            _cached_total += cached;
            session.prompt_tokens += round_pt;
            session.completion_tokens += round_ct;
            session.cached_tokens += cached;
            let hit_rate = if round_pt > 0 { cached as f64 / round_pt as f64 * 100.0 } else { 0.0 };
            // ★ 当前上下文 token 数
            let context_tokens: i64 = messages.iter().map(|m| {
                let text_len = m.content.as_ref().map_or(0, |s| s.len() as i64);
                let tc_len = m.tool_calls.as_ref().map_or(0, |tcs| tcs.iter()
                    .map(|tc| tc.function.arguments.len() as i64)
                    .sum::<i64>());
                (text_len + tc_len) / 3
            }).sum();
            log::info!("[Token] 当前上下文={} | 本轮 prompt={} cached={:.0}% completion={} | 累计 prompt={} | session p={} c={}",
                context_tokens, round_pt, hit_rate, round_ct, pt, session.prompt_tokens, session.completion_tokens);
        }
        let ac = ac_owned.as_str();
        let mut tcs: Option<Vec<Value>> = if tcs_streamed.is_empty() { None } else { Some(tcs_streamed) };
        // ★ 始终计算清理后的 content（剥离工具调用标记），后续所有展示/保存/判定都使用干净文本
        let cleaned_ac = clean_tool_markers(ac, true);
        log::info!("[Agent] 解析完成: ac_len={}, cleaned_len={}, tcs={}, has_xml_tool={}", 
            ac.len(), cleaned_ac.len(),
            tcs.as_ref().map(|v| v.len()).unwrap_or(0),
            ac.contains("tool_calls") || ac.contains("<tool_call>")
        );

        // ★ 兜底：如果 tool_calls 为空但 content 包含 XML 格式工具调用，尝试解析
        if tcs.is_none() || tcs.as_ref().map_or(false, |v| v.is_empty()) {
            if let Some(parsed) = parse_xml_tool_calls(ac) {
                log::info!("[LLM检索] 从 content 解析到 XML 工具调用: {:?}", parsed);
                tcs = Some(parsed);
            } else if let Some(parsed) = parse_html_tool_calls(ac) {
                log::info!("[LLM检索] 从 content 解析到 HTML 工具调用: {:?}", parsed);
                tcs = Some(parsed);
            }
        }

        let effective_ac = cleaned_ac.as_str();
        // ★ 工具调用文本检测基于原始 ac（effective_ac 已清理，无法再检测到残留标记）
        let has_tool_pattern = (ac.trim().starts_with('{') && ac.contains("tool_calls"))
            || ac.contains("<tool_call")
            || ac.starts_with("<invoke")
            || (ac.contains("ext_tool_") && ac.contains("```"))
            ;
        log::info!("[Agent] LLM 响应: len={}, tool_calls={}, has_tool_pattern={}", 
            effective_ac.len(), 
            tcs.as_ref().map(|v| v.len()).unwrap_or(0),
            has_tool_pattern
        );

        match tcs {
            None => {
                if has_tool_pattern && tool_text_retry_count < 2 {
                    tool_text_retry_count += 1;
                    log::warn!("[Agent] 检测到 content 中包含工具调用文本 (第{}次)，追加提示重试", tool_text_retry_count);
                    // 不存坏内容到历史，只标记上一轮失败
                    messages.push(ChatMessage{role:MessageRole::Assistant, content: None, tool_calls: None, tool_call_id: None, name: None, reasoning_content: None});
                    // 追加系统提示要求使用标准格式
                    messages.push(ChatMessage{role:MessageRole::User, content: Some("你的回复包含了工具调用文本，但没有通过 tool_calls 字段返回，工具无法被执行。请必须使用标准的 function calling 格式调用工具，禁止在 content 中输出任何工具调用格式。".into()), tool_calls: None, tool_call_id: None, name: None, reasoning_content: None});
                    continue; // 重新进入循环
                }

                // ★ 单请求流式：答案已在流式中逐 token 输出，无需再次生成
                let mut answer = effective_ac.to_string();
                log::info!("[Agent] 处理答案前: len={}, preview={}", answer.len(), answer.chars().take(100).collect::<String>());
                if answer.trim().is_empty() {
                    log::info!("[Agent] 流式内容为空，回退补生成");
                    answer = call_llm_without_tools(&messages, &app_handle).await.unwrap_or_else(|e| { log::warn!("[Agent] 流式补生成失败，降级为纯文本回答: {e}"); format!("未能生成有效回答: {e}") });
                }
                answer = answer.trim().to_string();

                // ★ 如果 cleaned 后为空（内容全部是工具调用标记），跳回主循环重新执行（允许工具调用）
                if answer.is_empty() && !query.is_empty() {
                    if retry_count < 2 {
                        retry_count += 1;
                        log::warn!("[Agent] 清理后答案为空，跳回主循环重新执行... (retry={})", retry_count);
                        // 清理最后一条 user 消息（重试提示），避免上下文污染
                        messages.pop();
                        continue;
                    } else {
                        log::warn!("[Agent] 重试次数耗尽，返回空答案");
                    }
                }

                log::info!("[Agent] 最终答案: len={}, preview={}", answer.len(), answer.chars().take(50).collect::<String>());
                if !silent { if let Some(ref h) = app_handle { let _ = h.emit("agent:status", json!({"stage":"answer"})); let _ = h.emit("agent:final-answer", json!({"answer": &answer})); } }
                if !answer.is_empty() { session.record_observation(&answer); }
                if let Some(ref sid) = session_id { save_session_state(sid, &session); }
                let ws = session.generate_scratchpad(); return Ok(json!({"success":true,"result_type":"answer","answer":answer,"workspace":ws,"iterations":iters,"promptTokens":session.prompt_tokens,"completionTokens":session.completion_tokens,"cachedTokens":session.cached_tokens,"tool_log":tool_log}));
            }
            Some(ref calls) if !calls.is_empty() => {
                // ★ 工具轮：不再向前端推送 interim 内容，检索期间保持预设文案
                // ★ 预算检查：不足时强制回答（在工具调用之前检查）
                if budget < search_cost {
                    if let Some(ref h) = app_handle {
                        let kw = calls.first().and_then(|tc| tc["function"]["arguments"].as_str().map(|s| s.to_string())).unwrap_or_default();
                        let _ = h.emit("agent:budget-low", json!({"remaining":budget,"cost":search_cost,"next_query":kw}));
                    }
                    messages.push(ChatMessage{role:MessageRole::User,content:Some("预算不足，请直接总结已获取的信息。".into()),tool_calls:None,tool_call_id:None,name:None,reasoning_content:None});
                    let ans = call_llm_without_tools(&messages, &app_handle).await.unwrap_or_else(|e| { log::warn!("[Agent] 预算不足总结失败: {e}"); format!("总结失败: {e}") });
                    if let Some(ref sid) = session_id { save_session_state(sid, &session); }
                    let ws = session.generate_scratchpad();
                    if !silent { if let Some(ref h) = app_handle { let _ = h.emit("agent:status", json!({"stage":"answer"})); let _ = h.emit("agent:final-answer", json!({"answer": &ans})); } }
                    return Ok(json!({"success":true,"result_type":"answer","answer":ans,"workspace":ws,"iterations":iters,"budget_exhausted":true,"promptTokens":session.prompt_tokens,"completionTokens":session.completion_tokens,"cachedTokens":session.cached_tokens,"tool_log":tool_log}));
                }
                if iters >= max_iters {
                    messages.push(ChatMessage{role:MessageRole::User,content:Some("已达到工具调用上限，请直接总结。".into()),tool_calls:None,tool_call_id:None,name:None,reasoning_content:None});
                    let ans = call_llm_without_tools(&messages, &app_handle).await.unwrap_or_else(|e| { log::warn!("[Agent] 迭代上限总结失败: {e}"); format!("总结失败: {e}") });
                    let ws = session.generate_scratchpad();
                    if !silent { if let Some(ref h) = app_handle { let _ = h.emit("agent:status", json!({"stage":"answer"})); let _ = h.emit("agent:final-answer", json!({"answer": ans})); } }
                    return Ok(json!({"success":true,"answer":ans,"workspace":ws,"iterations":iters,"circuit_breaker":true,"promptTokens":session.prompt_tokens,"completionTokens":session.completion_tokens,"cachedTokens":session.cached_tokens,"tool_log":tool_log}));
                }
            // ── Object Retrieval (dual-mode: timeline + keyword) ─────────────────

fn timeline_mode(obj_type: &str, start: &str, end: &str, limit: usize, offset: usize) -> String {
    // 先获取总数
    let total: i64 = database::get(
        "SELECT COUNT(*) as cnt FROM source_data WHERE (source_type=?1 OR ?1='') AND date(updated_at)>=?2 AND date(updated_at)<=?3",
        &[json!(obj_type), json!(start), json!(end)],
    ).ok().flatten().and_then(|r| r["cnt"].as_i64()).unwrap_or(0);
    
    let rows = database::all(
        "SELECT source_type, source_id, title, content as preview, updated_at FROM source_data
         WHERE (source_type=?1 OR ?1='')
           AND date(updated_at) >= ?2 AND date(updated_at) <= ?3
         ORDER BY updated_at DESC LIMIT ?4 OFFSET ?5",
        &[json!(obj_type), json!(start), json!(end), json!(limit), json!(offset)],
    ).unwrap_or_default();
    
    if rows.is_empty() {
        let summary = if offset > 0 { format!("已无更多对象（共 {} 条）。", total) } else { format!("时间范围内无{}对象。", if obj_type.is_empty() { "任何" } else { obj_type }) };
        let envelope = ToolResultEnvelope {
            result_type: "object_query_result".to_string(),
            data: json!({"mode": "timeline", "total": total, "offset": offset, "limit": limit, "items": []}),
            summary: Some(summary),
        };
        return envelope.to_json();
    }
    
    let mut items = Vec::new();
    let mut prev_ts: Option<String> = None;
    let mut boundary_count = 0u64;
    let mut max_boundary = 0.0f64;
    let mut total_boundary = 0.0f64;
    
    for r in &rows {
        let st = r["source_type"].as_str().unwrap_or("");
        let sid = r["source_id"].as_str().unwrap_or("");
        let name = r["title"].as_str().unwrap_or("");
        let preview_full = r["preview"].as_str().unwrap_or("");
        let preview = {
            let char_count = preview_full.chars().count();
            if char_count <= 200 {
                preview_full.to_string()
            } else {
                let mut last_newline = 0;
                let mut char_idx = 0;
                for ch in preview_full.chars() {
                    if ch == '\n' && char_idx <= 200 {
                        last_newline = char_idx;
                    }
                    char_idx += 1;
                    if char_idx > 250 { break; }
                }
                let cut_at = if last_newline > 100 { last_newline } else { 200 };
                let truncated: String = preview_full.chars().take(cut_at).collect();
                format!("{}…(已截断，共{}字)", truncated.trim_end(), char_count)
            }
        };
        let ts = r["updated_at"].as_str().unwrap_or("");
        
        // ★ Boundary scoring (time-based only)
        if let Some(ref p_ts) = prev_ts {
            let d_time = (chrono::NaiveDateTime::parse_from_str(ts, "%Y-%m-%d %H:%M:%S").ok()
                .zip(chrono::NaiveDateTime::parse_from_str(p_ts, "%Y-%m-%d %H:%M:%S").ok())
                .map(|(t,p)| (t-p).num_minutes().max(0) as f64)).unwrap_or(999.0);
            let boundary = 1.0 - (-0.03 * d_time).exp();
            if boundary > 0.45 {
                boundary_count += 1;
                max_boundary = max_boundary.max(boundary);
                total_boundary += boundary;
            }
        }
        
        // TODO 类型：额外显示优先级、截止日期、完成状态、todo_id
        let mut extra = None;
        if st == "TODO" {
            if let Ok(Some(todo)) = database::get("SELECT t.done, t.priority, t.due_date, t.repeat, p.name as project_name FROM todos t LEFT JOIN todo_project_mapping tpm ON t.id=tpm.todo_id LEFT JOIN projects p ON p.id=tpm.project_id WHERE t.id=?1", &[json!(sid)]) {
                let done = todo["done"].as_i64().unwrap_or(0);
                let pri = todo["priority"].as_str().unwrap_or("");
                let due = todo["due_date"].as_str().unwrap_or("");
                let repeat = todo["repeat"].as_str().unwrap_or("");
                let proj = todo["project_name"].as_str().unwrap_or("");
                let done_bool = done != 0;
                let status_str = if done_bool { "已完成" } else { "未完成" };
                let mut extra_obj = json!({"status": status_str, "done": done_bool, "todo_id": sid});
                if !pri.is_empty() { extra_obj["priority"] = json!(pri); }
                if !due.is_empty() { extra_obj["due_date"] = json!(due); }
                if !repeat.is_empty() { extra_obj["repeat"] = json!(repeat); }
                if !proj.is_empty() { extra_obj["project_name"] = json!(proj); }
                extra = Some(extra_obj);
            }
        }
        
        let mut item = json!({
            "source_type": st,
            "id": sid,
            "name": name,
            "preview": preview,
            "updated_at": ts
        });
        if let Some(extra_obj) = extra {
            item["extra"] = extra_obj;
        }
        items.push(item);
        
        prev_ts = Some(ts.to_string());
    }
    
    if boundary_count > 0 {
        let avg_boundary = total_boundary / boundary_count as f64;
        log::info!("[Boundary] 检测到 {} 处突变，最高 {:.2}，平均 {:.2}", boundary_count, max_boundary, avg_boundary);
    } else {
        log::info!("[Boundary] 无突变检测");
    }
    
    let end_pos = (offset + rows.len()).min(total as usize);
    let summary = format!("显示 {}/{} 条对象。", end_pos, total);
    
    let envelope = ToolResultEnvelope {
        result_type: "object_query_result".to_string(),
        data: json!({
            "mode": "timeline",
            "total": total,
            "offset": offset,
            "limit": limit,
            "items": items
        }),
        summary: Some(summary),
    };
    envelope.to_json()
}

fn keyword_object_mode(query: &str, obj_type: &str, start: &str, end: &str, limit: usize, offset: usize) -> String {
    let q = query.trim(); if q.is_empty() { return timeline_mode(obj_type, start, end, limit, offset); }
    let words: Vec<String> = q.split_whitespace().map(|k| k.replace('"',"")).collect();
    let fts_q = if words.len() > 1 { format!("NEAR(({}), 30)", words.join(" ")) } else { words.first().cloned().unwrap_or_default() };
    let results = match database::fts5_search_content(&fts_q, 30) { Ok(r) => r, Err(_) => Vec::new() };
    let mut seen = std::collections::HashSet::new();
    let mut matched: Vec<Value> = Vec::new();
    // 关键词搜索：不按日期过滤，直接按 FTS5 结果从 source_data 取数据
    for r in &results {
        let st = r.entity_type.to_uppercase();
        if !obj_type.is_empty() && st != obj_type.to_uppercase() { continue; }
        let sid = r.entity_id.trim().to_string();
        if !seen.insert(format!("{}-{}", st, sid)) { continue; }
        if let Ok(Some(row)) = database::get("SELECT title, content, updated_at FROM source_data WHERE source_type=?1 AND source_id=?2", &[json!(st), json!(sid)]) {
            matched.push(json!({"st": st, "sid": sid, "name": row["title"], "preview": row["content"], "ts": row["updated_at"]}));
        }
    }
    // LIKE fallback：同时搜索 title 和 content
    if matched.is_empty() {
        let like = format!("%{}%", crate::database::escape_like(&q));
        if let Ok(rows) = database::all(
            "SELECT source_type, source_id, title, content as preview, updated_at FROM source_data WHERE (title LIKE ?1 ESCAPE '\\' OR content LIKE ?1 ESCAPE '\\') AND (source_type=?2 OR ?2='') ORDER BY updated_at DESC",
            &[json!(&like), json!(obj_type)],
        ) {
            for r in &rows {
                let st = r["source_type"].as_str().unwrap_or("").to_uppercase();
                let sid = r["source_id"].as_str().unwrap_or("").to_string();
                if !seen.insert(format!("{}-{}", st, sid)) { continue; }
                matched.push(json!({"st": st, "sid": sid, "name": r["title"], "preview": r["preview"], "ts": r["updated_at"]}));
            }
        }
    }
    
    if matched.is_empty() {
        let envelope = ToolResultEnvelope {
            result_type: "object_query_result".to_string(),
            data: json!({"mode": "keyword", "query": q, "total": 0, "offset": offset, "limit": limit, "items": []}),
            summary: Some(format!("未找到匹配「{}」的对象。", q)),
        };
        return envelope.to_json();
    }
    
    let total = matched.len();
    let start_pos = offset.min(total);
    let end_pos = (offset + limit).min(total);
    let page: Vec<&Value> = matched.iter().skip(start_pos).take(limit).collect();
    
    let mut items = Vec::new();
    for r in &page {
        let st = r["st"].as_str().unwrap_or("");
        let sid = r["sid"].as_str().unwrap_or("");
        let name = r["name"].as_str().unwrap_or("");
        let preview_full = r["preview"].as_str().unwrap_or("");
        let preview = {
            let char_count = preview_full.chars().count();
            if char_count <= 200 {
                preview_full.to_string()
            } else {
                let mut last_newline = 0;
                let mut char_idx = 0;
                for ch in preview_full.chars() {
                    if ch == '\n' && char_idx <= 200 {
                        last_newline = char_idx;
                    }
                    char_idx += 1;
                    if char_idx > 250 { break; }
                }
                let cut_at = if last_newline > 100 { last_newline } else { 200 };
                let truncated: String = preview_full.chars().take(cut_at).collect();
                format!("{}…(已截断，共{}字)", truncated.trim_end(), char_count)
            }
        };
        let ts = r["ts"].as_str().unwrap_or("");
        
        // TODO 类型：额外显示优先级、截止日期、完成状态、todo_id
        let mut extra = None;
        if st == "TODO" {
            if let Ok(Some(todo)) = database::get("SELECT t.done, t.priority, t.due_date, t.repeat, p.name as project_name FROM todos t LEFT JOIN todo_project_mapping tpm ON t.id=tpm.todo_id LEFT JOIN projects p ON p.id=tpm.project_id WHERE t.id=?1", &[json!(sid)]) {
                let done = todo["done"].as_i64().unwrap_or(0);
                let pri = todo["priority"].as_str().unwrap_or("");
                let due = todo["due_date"].as_str().unwrap_or("");
                let repeat = todo["repeat"].as_str().unwrap_or("");
                let proj = todo["project_name"].as_str().unwrap_or("");
                let done_bool = done != 0;
                let status_str = if done_bool { "已完成" } else { "未完成" };
                let mut extra_obj = json!({"status": status_str, "done": done_bool, "todo_id": sid});
                if !pri.is_empty() { extra_obj["priority"] = json!(pri); }
                if !due.is_empty() { extra_obj["due_date"] = json!(due); }
                if !repeat.is_empty() { extra_obj["repeat"] = json!(repeat); }
                if !proj.is_empty() { extra_obj["project_name"] = json!(proj); }
                extra = Some(extra_obj);
            }
        }
        
        let mut item = json!({
            "source_type": st,
            "id": sid,
            "name": name,
            "preview": preview,
            "updated_at": ts
        });
        if let Some(extra_obj) = extra {
            item["extra"] = extra_obj;
        }
        items.push(item);
    }
    
    let summary = format!("显示 {}/{} 条对象。", end_pos, total);
    
    let envelope = ToolResultEnvelope {
        result_type: "object_query_result".to_string(),
        data: json!({
            "mode": "keyword",
            "query": q,
            "total": total,
            "offset": offset,
            "limit": limit,
            "items": items
        }),
        summary: Some(summary),
    };
    envelope.to_json()
}

/// Recall 模式：按天分组展示所有对象，不去重、不排序，完整回放
fn recall_mode(obj_type: &str, start: &str, end: &str, limit: usize, offset: usize) -> String {
    let rows = database::all(
        "SELECT source_type, source_id, title, updated_at FROM source_data
         WHERE (source_type=?1 OR ?1='')
           AND date(updated_at) >= ?2 AND date(updated_at) <= ?3
         ORDER BY updated_at ASC",
        &[json!(obj_type), json!(start), json!(end)],
    ).unwrap_or_default();
    
    if rows.is_empty() {
        return format!("时间范围内无{}对象。", if obj_type.is_empty() { "任何" } else { obj_type });
    }
    
    // 按日期分组
    let mut day_groups: std::collections::BTreeMap<String, Vec<&serde_json::Value>> = std::collections::BTreeMap::new();
    for r in &rows {
        let date = r["updated_at"].as_str().unwrap_or("").get(..10).unwrap_or("未知").to_string();
        day_groups.entry(date).or_default().push(r);
    }
    
    let total_days = day_groups.len();
    let days: Vec<_> = day_groups.iter().collect();
    let start_day = offset.min(total_days);
    let end_day = (offset + limit).min(total_days);
    
    if start_day >= total_days {
        return format!("已无更多数据（共 {} 天）。", total_days);
    }
    
    let mut out = format!("┌─ 共 {} 天，显示第 {}-{} 天 ─┐\n\n", total_days, start_day + 1, end_day);
    let mut total_chars = out.len();
    
    for (date, items) in days.iter().skip(start_day).take(limit) {
        // 统计新增和修改数量
        let mut new_count = 0;
        let mut update_count = 0;
        for r in items.iter() {
            let created = r["created_at"].as_str().unwrap_or("").get(..10).unwrap_or("");
            let updated = r["updated_at"].as_str().unwrap_or("").get(..10).unwrap_or("");
            if created == *date {
                new_count += 1;
            } else if updated == *date {
                update_count += 1;
            }
        }
        let day_header = format!("==== {} (新增: {}, 修改: {}) ====\n", date, new_count, update_count);
        out.push_str(&day_header);
        
        for r in items.iter() {
            let st = r["source_type"].as_str().unwrap_or("");
            let sid = r["source_id"].as_str().unwrap_or("");
            let mut name = r["title"].as_str().unwrap_or("").to_string();
            let created = r["created_at"].as_str().unwrap_or("").get(..10).unwrap_or("");
            let updated = r["updated_at"].as_str().unwrap_or("").get(..10).unwrap_or("");
            
            // 判断标记：+新增 *修改 -其他
            let marker = if created == *date { "+" } else if updated == *date { "*" } else { "-" };
            
            // FILE 类型：从 source_data 获取文件名
            if st == "FILE" && name.is_empty() {
                if let Ok(Some(sd)) = database::get(
                    "SELECT title FROM source_data WHERE source_type='FILE' AND source_id=?1",
                    &[json!(sid)],
                ) {
                    if let Some(t) = sd["title"].as_str().filter(|s| !s.is_empty()) {
                        name = t.to_string();
                    }
                }
            }
            
            // 输出主对象
            out.push_str(&format!("{} [{}] {} | source_id: {}-{}\n", marker, st, name, st, sid));
        }
        out.push('\n');
        
        total_chars = out.len();
        // 如果超过约 200K 字符（约 50K token），截断并提示分页
        if total_chars > 200_000 {
            let remaining_days = total_days - (start_day + (out.matches("====").count() - start_day));
            out.push_str(&format!("└─ 输出已截断（{} 字符），还有约 {} 天的数据，传 offset={} 继续 ─┘", total_chars, remaining_days, end_day));
            break;
        }
    }
    
    if end_day < total_days && total_chars <= 200_000 {
        out.push_str(&format!("└─ 还有 {} 天未显示，传 offset={} 继续查看 ─┘", total_days - end_day, end_day));
    }
    
    out
}

fn execute_object_get(query: &str, obj_type: &str, time_range: Option<(String, String)>, limit: usize, offset: usize, mode: &str) -> String {
    let limit = if limit == 0 { 50 } else { limit.min(100) };
    let (start, end) = match time_range {
        Some((s, e)) => (s, e),
        None => ("0001-01-01".to_string(), "2099-12-31".to_string()),
    };
    // 如果有关键词，优先使用关键词搜索模式
    if !query.trim().is_empty() {
        return keyword_object_mode(query, obj_type, &start, &end, limit, offset);
    }
    if mode == "recall" {
        recall_mode(obj_type, &start, &end, limit, offset)
    } else {
        timeline_mode(obj_type, &start, &end, limit, offset)
    }
}

    let mut current_round = json!({"round":iters,"thought":ac,"tools":[]});
                let mut atc = Vec::new();
                for tc in calls {
                    let id = tc["id"].as_str().unwrap_or("").to_string();
                    let name = tc["function"]["name"].as_str().unwrap_or("");
                    let args = tc["function"]["arguments"].as_str().unwrap_or("").to_string();
                    atc.push(ToolCall{id:id.clone(),call_type:"function".into(),function:FunctionCall{name:name.into(),arguments:args.clone()}});
                }
                messages.push(ChatMessage{role:MessageRole::Assistant,content:None,tool_calls:Some(atc.clone()),tool_call_id:None,name:None,reasoning_content:reasoning.clone()});
                // ★ 向前端广播工具调用状态
                if let Some(ref h) = app_handle {
                    for tc in atc.iter() {
                        let args: Value = serde_json::from_str(&tc.function.arguments).unwrap_or(json!({}));
                        let target = match tc.function.name.as_str() {
                            "ext_tool_search" => args.get("keywords").and_then(|k| k.as_array()).map(|a| a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join("、")),
                            "ext_tool_object" => {
                                if let Some(ids) = args.get("id").and_then(|v| v.as_array()).filter(|a| !a.is_empty()) {
                                    Some(ids.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join(", "))
                                } else if let Some(ff) = args.get("field_filters").and_then(|v| v.as_array()).filter(|a| !a.is_empty()) {
                                    let entity = args.get("type").and_then(|v| v.as_str()).unwrap_or("?");
                                    Some(format!("{} ({}条过滤)", entity, ff.len()))
                                } else {
                                    args.get("query").and_then(|q| q.as_str()).map(String::from)
                                }
                            },
                            "ext_tool_chunk_navigate" => args.get("chunk_id").and_then(|c| c.as_str()).map(|c| format!("chunk {}", &c[..c.len().min(8)])),
                            _ => None,
                        };
                        let _ = h.emit("agent:status", json!({"stage":"tool_start","tool":tc.function.name,"target":target.unwrap_or_default()}));
                        let _ = h.emit("agent:progress", json!({"msg": format!("正在{}...", if tc.function.name.starts_with("ext_tool_search") { "检索知识库" } else if tc.function.name.starts_with("ext_tool_object") { "查询数据" } else { "处理中" })}));
                    }
                }
                // ═══ 并行工具调度：async 工具并行执行，sync 工具顺序执行 ═══
                let mut round_tools = Vec::new();
                let mut dedup_cache: std::collections::HashMap<(String,String),String> = std::collections::HashMap::new();
                let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
                let mut extract_chunk_ids = |tools: &[Value]| { for t in tools { let s = t["result"].as_str().unwrap_or(""); if s.contains("ChunkID") { for line in s.lines() { let tl = line.trim(); if tl.starts_with("ChunkID") { if let Some(id) = tl.split('：').nth(1).map(|x| x.trim().to_string()) { visited.insert(id); } } } } } };
                for r in &tool_log { if let Some(tools) = r["tools"].as_array() { extract_chunk_ids(tools); } }
                extract_chunk_ids(&round_tools);
                session.visited_chunks.extend(visited.iter().cloned());

                // 快照并行工具需要的只读状态
                let scope_snapshot = compute_scope_from_session(&session);
                let visited_sources_snapshot = session.visited_sources.clone();
                let visited_chunks_snapshot = session.visited_chunks.clone();

                // 预去重 + 分组：async 工具 vs sync 工具
                let mut async_tasks: Vec<(usize, String, String)> = Vec::new(); // (index, name, args)
                let mut sync_tasks: Vec<(usize, String, String)> = Vec::new();
                let mut cached_results_map: std::collections::HashMap<usize, String> = std::collections::HashMap::new();

                for (i, tc) in atc.iter().enumerate() {
                    let key = (tc.function.name.clone(), tc.function.arguments.clone());
                    if let Some(cached) = dedup_cache.get(&key) {
                        if let Some(ref h) = app_handle {
                            let _ = h.emit("agent:tool-result", json!({"round": iters, "tool": tc.function.name, "cached": true, "result_len": cached.len()}));
                        }
                        cached_results_map.insert(i, cached.clone());
                    } else {
                        match tc.function.name.as_str() {
                            "ext_tool_search" | "ext_tool_search_knowledge_base" | "ext_tool_batch_search" => {
                                async_tasks.push((i, tc.function.name.clone(), tc.function.arguments.clone()));
                            }
                            _ => {
                                sync_tasks.push((i, tc.function.name.clone(), tc.function.arguments.clone()));
                            }
                        }
                    }
                }

                // 并行执行 async 工具
                let results_mutex: Mutex<Vec<(usize, String, Vec<SessionEffect>)>> = Mutex::new(Vec::new());
                if !async_tasks.is_empty() {
                    let mut handles = Vec::new();
                    for (orig_idx, name, args_str) in &async_tasks {
                        let name = name.clone();
                        let args_str = args_str.clone();
                        let orig_idx = *orig_idx;
                        let scope = scope_snapshot.clone();
                        let vs = visited_sources_snapshot.clone();
                        let vc = visited_chunks_snapshot.clone();
                        let shown_mem = session.shown_memory_ids.clone();
                        handles.push(tokio::spawn(async move {
                            let a: Value = serde_json::from_str(&args_str).unwrap_or(json!({}));
                            let (res, effects) = match name.as_str() {
                                "ext_tool_search" | "ext_tool_search_knowledge_base" => {
                                    let kw: Vec<String> = a["keywords"].as_array()
                                        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                                        .or_else(|| a["keywords"].as_str().map(|s| s.split_whitespace().map(String::from).collect()))
                                        .unwrap_or_default();
                                    let time_range = a["start_time"].as_str()
                                        .and_then(|s| a["end_time"].as_str().map(|e| (resolve_time(s), resolve_time(e))));
                                    let (mut res, new_sources) = execute_search_knowledge(
                                        &kw, scope.clone(), &vs, &vc, time_range.clone(), &shown_mem
                                    ).await;
                                    if new_sources.len() < 2 && scope.is_some() {
                                        let fb = execute_search_knowledge(&kw, None, &vs, &vc, time_range, &shown_mem).await;
                                        res = fb.0;
                                        let mut effects = vec![SessionEffect::RecordSearch(kw)];
                                        for s in &fb.1 {
                                            effects.push(SessionEffect::MarkSourceVisited(s.clone()));
                                            if let Some(dash) = s.find('-') {
                                                let (ot, oi) = s.split_at(dash);
                                                let oi = &oi[1..];
                                                let name = database::get("SELECT title FROM source_data WHERE source_type=?1 AND source_id=?2", &[json!(ot.to_uppercase()), json!(oi)]).ok().flatten().and_then(|r| r["title"].as_str().map(String::from)).unwrap_or_default();
                                                if !ot.is_empty() && !oi.is_empty() && !name.is_empty() {
                                                    effects.push(SessionEffect::UpdateObject { obj_type: ot.to_lowercase(), obj_id: oi.to_string(), name, score_delta: 0.0 });
                                                }
                                            }
                                        }
                                        (res, effects)
                                    } else {
                                        let mut effects = vec![SessionEffect::RecordSearch(kw)];
                                        for s in &new_sources {
                                            effects.push(SessionEffect::MarkSourceVisited(s.clone()));
                                            if let Some(dash) = s.find('-') {
                                                let (ot, oi) = s.split_at(dash);
                                                let oi = &oi[1..];
                                                let name = database::get("SELECT title FROM source_data WHERE source_type=?1 AND source_id=?2", &[json!(ot.to_uppercase()), json!(oi)]).ok().flatten().and_then(|r| r["title"].as_str().map(String::from)).unwrap_or_default();
                                                if !ot.is_empty() && !oi.is_empty() && !name.is_empty() {
                                                    effects.push(SessionEffect::UpdateObject { obj_type: ot.to_lowercase(), obj_id: oi.to_string(), name, score_delta: 0.0 });
                                                }
                                            }
                                        }
                                        (res, effects)
                                    }
                                }
                                "ext_tool_batch_search" => {
                                    let queries_raw = a["queries"].as_array();
                                    let mut sub_queries: Vec<Vec<String>> = Vec::new();
                                    if let Some(arr) = queries_raw {
                                        for q in arr.iter().take(10) {
                                            if let Some(kw) = q["keywords"].as_array() {
                                                let k: Vec<String> = kw.iter().filter_map(|v| v.as_str().map(String::from)).collect();
                                                if !k.is_empty() { sub_queries.push(k); }
                                            }
                                        }
                                    }
                                    let res = execute_batch_search(&sub_queries, scope, &vs).await;
                                    (res, vec![])
                                }
                                _ => unreachable!(),
                            };
                            (orig_idx, res, effects)
                        }));
                    }
                    for handle in handles {
                        match handle.await {
                            Ok(result) => {
                                results_mutex.lock().unwrap_or_else(|e| e.into_inner()).push(result);
                            }
                            Err(e) => {
                                log::error!("[Agent] 异步工具任务 JoinHandle 失败: {}", e);
                            }
                        }
                    }
                }
                let async_results: Vec<(usize, String, Vec<SessionEffect>)> = {
                    let guard = results_mutex.lock().unwrap_or_else(|e| e.into_inner());
                    guard.clone()
                };

                // 顺序执行 sync 工具
                let mut sync_results: Vec<(usize, String, Vec<SessionEffect>)> = Vec::new();
                for (idx, name, args_str) in &sync_tasks {
                    let a: Value = serde_json::from_str(args_str).unwrap_or(json!({}));
                    let (result, effects) = match name.as_str() {
                        "ext_tool_object" => {
                            let mode = a["mode"].as_str().unwrap_or("");
                            let res = match mode {
                                "detail" => {
                                    // ★ 按ID查看详情
                                    let ids_arr = a["id"].as_array().filter(|arr| !arr.is_empty());
                                    let ids: Vec<String> = ids_arr.map(|arr| {
                                        arr.iter().filter_map(|v| v.as_str().map(String::from)).collect()
                                    }).unwrap_or_default();
                                    if ids.is_empty() {
                                        "错误：detail模式需要传入id参数".to_string()
                                    } else {
                                        execute_batch_recall(&ids, &visited_chunks_snapshot, &visited_sources_snapshot).await
                                    }
                                },
                                "search" => {
                                    // ★ 关键词搜索
                                    let q = a["query"].as_str().unwrap_or("");
                                    let t = a["type"].as_str().unwrap_or("");
                                    let s = a["start_time"].as_str().map(resolve_time);
                                    let e = a["end_time"].as_str().map(resolve_time);
                                    let tr = match (s, e) {
                                        (Some(s), Some(e)) => Some((s, e)),
                                        (Some(s), None) => Some((s, "2099-12-31".to_string())),
                                        (None, Some(e)) => Some(("2000-01-01".to_string(), e)),
                                        (None, None) => None,
                                    };
                                    let limit = a["limit"].as_u64().unwrap_or(50) as usize;
                                    let offset = a["offset"].as_u64().unwrap_or(0) as usize;
                                    execute_object_get(q, t, tr, limit, offset, "search")
                                },
                                "filter" => {
                                    // ★ 结构化筛选
                                    let entity = a["type"].as_str().unwrap_or("");
                                    let ff = a["field_filters"].as_array().map(|a| a.as_slice()).unwrap_or(&[]);
                                    execute_object_catalog(entity, ff, a.get("sort"), &a)
                                },
                                "count" => {
                                    // ★ 统计数量
                                    let entity = a["type"].as_str().unwrap_or("");
                                    let ff = a["field_filters"].as_array().map(|a| a.as_slice()).unwrap_or(&[]);
                                    execute_object_catalog(entity, ff, a.get("sort"), &a)
                                },
                                "group" => {
                                    // ★ 分组统计
                                    let entity = a["type"].as_str().unwrap_or("");
                                    let ff = a["field_filters"].as_array().map(|a| a.as_slice()).unwrap_or(&[]);
                                    execute_object_catalog(entity, ff, a.get("sort"), &a)
                                },
                                "aggregate" => {
                                    // ★ 聚合计算
                                    let entity = a["type"].as_str().unwrap_or("");
                                    let ff = a["field_filters"].as_array().map(|a| a.as_slice()).unwrap_or(&[]);
                                    execute_object_catalog(entity, ff, a.get("sort"), &a)
                                },
                                _ => {
                                    // 兼容旧格式：没有 mode 时，按参数推断
                                    if let Some(ids_arr) = a["id"].as_array().filter(|arr| !arr.is_empty()) {
                                        let ids: Vec<String> = ids_arr.iter().filter_map(|v| v.as_str().map(String::from)).collect();
                                        execute_batch_recall(&ids, &visited_chunks_snapshot, &visited_sources_snapshot).await
                                    } else if let Some(ff) = a["field_filters"].as_array().filter(|arr| !arr.is_empty()) {
                                        let entity = a["type"].as_str().unwrap_or("");
                                        execute_object_catalog(entity, ff, a.get("sort"), &a)
                                    } else {
                                        let q = a["query"].as_str().unwrap_or("");
                                        let t = a["type"].as_str().unwrap_or("");
                                        execute_object_get(q, t, None, 50, 0, "search")
                                    }
                                }
                            };
                            (res, vec![])
                        }
                        "ext_tool_chunk_navigate" => {
                            let res = execute_traverse(
                                a["chunk_id"].as_str().unwrap_or(""),
                                a["edge_type"].as_str().unwrap_or("Next"),
                                a["radius"].as_i64().unwrap_or(10) as usize,
                                &visited,
                                a["target_name"].as_str().unwrap_or(""),
                                &mut traverse_source_counter,
                            );
                            (res, vec![])
                        }
                        "ext_tool_relations" => {
                            let entity_type = a["entity_type"].as_str().unwrap_or("");
                            let entity_id = a["entity_id"].as_str().unwrap_or("");
                            let target_type = a["target_type"].as_str().filter(|s| !s.is_empty());
                            let limit = a["limit"].as_u64().unwrap_or(20) as usize;
                            let offset = a["offset"].as_u64().unwrap_or(0) as usize;
                            let res = execute_relations(entity_type, entity_id, target_type, limit, offset);
                            (res, vec![])
                        }
                        "ext_tool_manage_todo" => {
                            let res = ext_tool_manage_todo(
                                a["action"].as_str().unwrap_or(""),
                                a["title"].as_str().unwrap_or(""),
                                a["content"].as_str().unwrap_or(""),
                                a["priority"].as_str().unwrap_or(""),
                                a["due_date"].as_str().unwrap_or(""),
                                a["repeat"].as_str().unwrap_or(""),
                                a["project_id"].as_str().unwrap_or(""),
                                a["done"].as_bool().unwrap_or(false),
                                a["todo_id"].as_str().unwrap_or(""),
                            );
                            (res, vec![])
                        }
                        "ext_tool_memory" => {
                            let items_str = if let Some(arr) = a["items"].as_array() {
                                serde_json::to_string(arr).unwrap_or_default()
                            } else {
                                String::new()
                            };
                            let keywords_str = if let Some(arr) = a["keywords"].as_array() {
                                serde_json::to_string(arr).unwrap_or_default()
                            } else {
                                String::new()
                            };
                            let res = ext_tool_memory(
                                a["action"].as_str().unwrap_or(""),
                                a["content"].as_str().unwrap_or(""),
                                &items_str,
                                a["memory_id"].as_str().unwrap_or(""),
                                a["query"].as_str().unwrap_or(""),
                                &keywords_str,
                                a["start_time"].as_str().unwrap_or(""),
                                a["end_time"].as_str().unwrap_or(""),
                            );
                            (res, vec![])
                        }
                        "ext_tool_manage_policy" => {
                            let action = a["action"].as_str().unwrap_or("");
                            let id = a["id"].as_str().map(|s| s.to_string());
                            let content = a["content"].as_str().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
                            let res = match action {
                                "update" => {
                                    if id.is_none() || content.is_none() {
                                        let e = ToolResultEnvelope { result_type: "policy_update_result".to_string(), data: json!({"success": false}), summary: Some("更新需要提供 id 和 content。".to_string()) };
                                        e.to_json()
                                    } else {
                                        let pid = id.as_ref().unwrap();
                                        let c = content.as_ref().unwrap();
                                        let real_id = pid.strip_prefix("policy-").unwrap_or(pid);
                                        let r = database::run("UPDATE policies SET content=?1, updated_at=datetime('now','localtime') WHERE id=?2", &[json!(c), json!(real_id)]);
                                        match r {
                                            Ok(r) if r.changes > 0 => {
                                                let envelope = ToolResultEnvelope { result_type: "policy_update_result".to_string(), data: json!({"id": real_id, "success": true}), summary: Some(format!("已更新策略：{}", c)) };
                                                envelope.to_json()
                                            }
                                            Ok(_) => {
                                                let envelope = ToolResultEnvelope { result_type: "policy_update_result".to_string(), data: json!({"id": real_id, "success": false}), summary: Some(format!("未找到策略 ID：{}", real_id)) };
                                                envelope.to_json()
                                            }
                                            Err(e) => {
                                                let envelope = ToolResultEnvelope { result_type: "policy_update_result".to_string(), data: json!({"success": false}), summary: Some(format!("更新失败：{}", e)) };
                                                envelope.to_json()
                                            }
                                        }
                                    }
                                }
                                "delete" => {
                                    if id.is_none() {
                                        let e = ToolResultEnvelope { result_type: "policy_delete_result".to_string(), data: json!({"success": false}), summary: Some("删除需要提供 id。".to_string()) };
                                        e.to_json()
                                    } else {
                                        let pid = id.as_ref().unwrap();
                                        let real_id = pid.strip_prefix("policy-").unwrap_or(pid);
                                        let r = database::run("DELETE FROM policies WHERE id=?1", &[json!(real_id)]);
                                        match r {
                                            Ok(r) if r.changes > 0 => {
                                                let envelope = ToolResultEnvelope { result_type: "policy_delete_result".to_string(), data: json!({"id": real_id, "success": true}), summary: Some("已删除策略".to_string()) };
                                                envelope.to_json()
                                            }
                                            Ok(_) => {
                                                let envelope = ToolResultEnvelope { result_type: "policy_delete_result".to_string(), data: json!({"id": real_id, "success": false}), summary: Some(format!("未找到策略 ID：{}", real_id)) };
                                                envelope.to_json()
                                            }
                                            Err(e) => {
                                                let envelope = ToolResultEnvelope { result_type: "policy_delete_result".to_string(), data: json!({"success": false}), summary: Some(format!("删除失败：{}", e)) };
                                                envelope.to_json()
                                            }
                                        }
                                    }
                                }
                                _ => { // create
                                    if content.is_none() {
                                        let e = ToolResultEnvelope { result_type: "policy_create_result".to_string(), data: json!({"success": false}), summary: Some("创建需要提供 content。".to_string()) };
                                        e.to_json()
                                    } else {
                                        let c = content.as_ref().unwrap();
                                        let new_id = uuid::Uuid::new_v4().to_string();
                                        let r = database::run("INSERT INTO policies(id,content,updated_at) VALUES(?1,?2,datetime('now','localtime'))", &[json!(&new_id), json!(c)]);
                                    match r {
                                        Ok(_) => {
                                            let envelope = ToolResultEnvelope {
                                                result_type: "policy_create_result".to_string(),
                                                data: json!({"id": new_id, "success": true}),
                                                 summary: Some(format!("已创建策略：{}", content.as_ref().unwrap())),
                                            };
                                            envelope.to_json()
                                        }
                                        Err(e) => {
                                            let envelope = ToolResultEnvelope {
                                                result_type: "policy_create_result".to_string(),
                                                data: json!({"success": false}),
                                                summary: Some(format!("创建失败：{}", e)),
                                            };
                                            envelope.to_json()
                                        }
                                    }
                                    }
                                }
                            };
                            (res, vec![])
                        }
                        "ext_tool_manage_project" => {
                            let action = a["action"].as_str().unwrap_or("");
                            let res = match action {
                                "create" => {
                                    let name = a["name"].as_str().unwrap_or("");
                                    if name.is_empty() {
                                        let envelope = ToolResultEnvelope {
                                            result_type: "object_create_result".to_string(),
                                            data: json!({"error": "创建项目需要提供 name"}),
                                            summary: Some("创建项目需要提供 name。".to_string()),
                                        };
                                        envelope.to_json()
                                    }
                                    else {
                                        let budget = a["budget"].as_f64().unwrap_or(0.0);
                                        let description = a["description"].as_str().unwrap_or("");
                                        let status = a["status"].as_str().unwrap_or("Lead");
                                        let bid_time = if status == "Bidding" { a["bid_time"].as_str().unwrap_or("") } else { "" };
                                        let data = json!({
                                            "name": name,
                                            "budget": budget,
                                            "description": description,
                                            "status": status,
                                            "bidTime": bid_time
                                        });
                                        // 使用 block_on 调用异步函数
                                        match block_on_async(crate::commands::project_commands::project_create(data)) {
                                            Ok(project) => {
                                                let pid = project["id"].as_str().unwrap_or("");
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "object_create_result".to_string(),
                                                    data: json!({"id": pid, "type": "PROJECT", "name": name}),
                                                    summary: Some(format!("已创建项目「{}」", name)),
                                                };
                                                envelope.to_json()
                                            },
                                            Err(e) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "object_create_result".to_string(),
                                                    data: json!({"error": e.to_string()}),
                                                    summary: Some(format!("创建项目失败: {}", e)),
                                                };
                                                envelope.to_json()
                                            },
                                        }
                                    }
                                }
                                "update" => {
                                    let raw_id = a["project_id"].as_str().unwrap_or("");
                                    let project_id = raw_id.strip_prefix("PROJECT-").unwrap_or(raw_id);
                                    if project_id.is_empty() {
                                        let envelope = ToolResultEnvelope {
                                            result_type: "object_update_result".to_string(),
                                            data: json!({"error": "更新项目需要提供 project_id"}),
                                            summary: Some("更新项目需要提供 project_id。".to_string()),
                                        };
                                        envelope.to_json()
                                    }
                                    else {
                                        let mut data = json!({});
                                        if let Some(v) = a["name"].as_str().filter(|s| !s.is_empty()) { data["name"] = json!(v); }
                                        if let Some(v) = a["description"].as_str().filter(|s| !s.is_empty()) { data["description"] = json!(v); }
                                        if let Some(v) = a["status"].as_str().filter(|s| !s.is_empty()) { data["status"] = json!(v); }
                                        if let Some(v) = a["bid_time"].as_str().filter(|s| !s.is_empty()) { data["bidTime"] = json!(v); }
                                        if let Some(v) = a["budget"].as_f64().filter(|v| *v > 0.0) { data["budget"] = json!(v); }
                                        if data.as_object().map_or(true, |m| m.is_empty()) {
                                            let envelope = ToolResultEnvelope {
                                                result_type: "object_update_result".to_string(),
                                                data: json!({"id": project_id, "error": "未提供任何更新字段"}),
                                                summary: Some("未提供任何更新字段。".to_string()),
                                            };
                                            envelope.to_json()
                                        } else {
                                            // 使用 block_on 调用异步函数
                                            match block_on_async(crate::commands::project_commands::project_update(project_id.to_string(), data)) {
                                                Ok(_) => {
                                                    let project_name = database::get(
                                                        "SELECT name FROM projects WHERE id=?1",
                                                        &[json!(project_id)],
                                                    ).ok().flatten().and_then(|r| r["name"].as_str().map(String::from)).unwrap_or_default();
                                                    let display_name = if project_name.is_empty() { project_id.to_string() } else { project_name };
                                                    let envelope = ToolResultEnvelope {
                                                        result_type: "object_update_result".to_string(),
                                                        data: json!({"id": project_id}),
                                                        summary: Some(format!("已更新项目「{}」", display_name)),
                                                    };
                                                    envelope.to_json()
                                                }
                                                Err(e) => {
                                                    let envelope = ToolResultEnvelope {
                                                        result_type: "object_update_result".to_string(),
                                                        data: json!({"id": project_id, "error": e.to_string()}),
                                                        summary: Some(format!("更新项目失败: {}", e)),
                                                    };
                                                    envelope.to_json()
                                                },
                                            }
                                        }
                                    }
                                }
                                "delete" => {
                                    let raw_id = a["project_id"].as_str().unwrap_or("");
                                    let project_id = raw_id.strip_prefix("PROJECT-").unwrap_or(raw_id);
                                    if project_id.is_empty() {
                                        let envelope = ToolResultEnvelope {
                                            result_type: "object_delete_result".to_string(),
                                            data: json!({"error": "删除项目需要提供 project_id"}),
                                            summary: Some("删除项目需要提供 project_id。".to_string()),
                                        };
                                        envelope.to_json()
                                    }
                                    else {
                                        // 使用 block_on 调用异步函数
                                        match block_on_async(crate::commands::project_commands::project_delete(project_id.to_string())) {
                                            Ok(_) => {
                                                let project_name = database::get(
                                                    "SELECT name FROM projects WHERE id=?1",
                                                    &[json!(project_id)],
                                                ).ok().flatten().and_then(|r| r["name"].as_str().map(String::from)).unwrap_or_default();
                                                let display_name = if project_name.is_empty() { project_id.to_string() } else { project_name };
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "object_delete_result".to_string(),
                                                    data: json!({"id": project_id}),
                                                    summary: Some(format!("已删除项目「{}」", display_name)),
                                                };
                                                envelope.to_json()
                                            }
                                            Err(e) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "object_delete_result".to_string(),
                                                    data: json!({"id": project_id, "error": e.to_string()}),
                                                    summary: Some(format!("删除项目失败: {}", e)),
                                                };
                                                envelope.to_json()
                                            },
                                        }
                                    }
                                }
                                "add_dynamic" => {
                                    let raw_id = a["project_id"].as_str().unwrap_or("");
                                    let project_id = raw_id.strip_prefix("PROJECT-").unwrap_or(raw_id);
                                    let content = a["content"].as_str().unwrap_or("");
                                    if project_id.is_empty() || content.is_empty() {
                                        let envelope = ToolResultEnvelope {
                                            result_type: "object_create_result".to_string(),
                                            data: json!({"error": "add_dynamic 需要提供 project_id 和 content"}),
                                            summary: Some("add_dynamic 需要提供 project_id 和 content。".to_string()),
                                        };
                                        envelope.to_json()
                                    } else {
                                        let dynamic_type = a["dynamic_type"].as_str().unwrap_or("ai");
                                        let event = json!({"type": dynamic_type, "content": content, "operator": "AI"});
                                        match block_on_async(crate::commands::project_commands::project_dynamics_add(project_id.to_string(), event)) {
                                            Ok(_) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "object_create_result".to_string(),
                                                    data: json!({"project_id": project_id, "type": dynamic_type}),
                                                    summary: Some(format!("已为项目追加动态记录")),
                                                };
                                                envelope.to_json()
                                            }
                                            Err(e) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "object_create_result".to_string(),
                                                    data: json!({"error": e.to_string()}),
                                                    summary: Some(format!("追加动态失败: {}", e)),
                                                };
                                                envelope.to_json()
                                            }
                                        }
                                    }
                                }
                                "delete_dynamic" => {
                                    let dynamic_id = a["dynamic_id"].as_str().unwrap_or("");
                                    if dynamic_id.is_empty() {
                                        let envelope = ToolResultEnvelope {
                                            result_type: "object_delete_result".to_string(),
                                            data: json!({"error": "delete_dynamic 需要提供 dynamic_id"}),
                                            summary: Some("delete_dynamic 需要提供 dynamic_id。".to_string()),
                                        };
                                        envelope.to_json()
                                    } else {
                                        match block_on_async(crate::commands::project_commands::project_dynamics_delete(dynamic_id.to_string())) {
                                            Ok(_) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "object_delete_result".to_string(),
                                                    data: json!({"dynamic_id": dynamic_id, "deleted": true}),
                                                    summary: Some("已删除动态记录。".to_string()),
                                                };
                                                envelope.to_json()
                                            }
                                            Err(e) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "object_delete_result".to_string(),
                                                    data: json!({"error": e.to_string()}),
                                                    summary: Some(format!("删除动态失败: {}", e)),
                                                };
                                                envelope.to_json()
                                            }
                                        }
                                    }
                                }
                                _ => {
                                    let envelope = ToolResultEnvelope {
                                        result_type: "object_error".to_string(),
                                        data: json!({"error": format!("未知操作「{}」。支持：create, update, delete, add_dynamic, delete_dynamic。", action)}),
                                        summary: Some(format!("未知操作「{}」。支持：create, update, delete, add_dynamic, delete_dynamic。", action)),
                                    };
                                    envelope.to_json()
                                },
                            };
                            (res, vec![])
                        }
                        "ext_tool_manage_partner" => {
                            let action = a["action"].as_str().unwrap_or("");
                            let res = match action {
                                "create" => {
                                    let ptype = a["type"].as_str().unwrap_or("");
                                    let name = a["name"].as_str().unwrap_or("");
                                    if ptype.is_empty() || name.is_empty() {
                                        let envelope = ToolResultEnvelope {
                                            result_type: "object_create_result".to_string(),
                                            data: json!({"error": "创建合作伙伴需要提供 type 和 name"}),
                                            summary: Some("创建合作伙伴需要提供 type 和 name。".to_string()),
                                        };
                                        envelope.to_json()
                                    }
                                    else {
                                        let data = json!({
                                            "type": ptype.to_uppercase(),
                                            "name": name,
                                            "phone": a["phone"].as_str().unwrap_or(""),
                                            "company": a["address"].as_str().unwrap_or(""),
                                            "remark": a["remark"].as_str().unwrap_or("")
                                        });
                                        // 使用 block_on 调用异步函数
                                        match block_on_async(crate::commands::partner_commands::partner_create(data)) {
                                            Ok(result) => {
                                                let id = result["id"].as_str().unwrap_or("");
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "object_create_result".to_string(),
                                                    data: json!({"id": id, "type": ptype.to_uppercase(), "name": name}),
                                                    summary: Some(format!("已创建{}「{}」", ptype.to_uppercase(), name)),
                                                };
                                                envelope.to_json()
                                            },
                                            Err(e) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "object_create_result".to_string(),
                                                    data: json!({"error": e.to_string()}),
                                                    summary: Some(format!("创建失败: {}", e)),
                                                };
                                                envelope.to_json()
                                            },
                                        }
                                    }
                                }
                                "update" => {
                                    let raw_pid = a["partner_id"].as_str().unwrap_or("");
                                    let partner_id = raw_pid.strip_prefix("CHANNEL-").or_else(|| raw_pid.strip_prefix("VENDOR-")).or_else(|| raw_pid.strip_prefix("CUSTOMER-")).unwrap_or(raw_pid);
                                    if partner_id.is_empty() {
                                        let envelope = ToolResultEnvelope {
                                            result_type: "object_update_result".to_string(),
                                            data: json!({"error": "更新需要提供 partner_id"}),
                                            summary: Some("更新需要提供 partner_id。".to_string()),
                                        };
                                        envelope.to_json()
                                    }
                                    else {
                                        let mut data = json!({"id": partner_id});
                                        if let Some(v) = a["name"].as_str().filter(|s| !s.is_empty()) { data["name"] = json!(v); }
                                        if let Some(v) = a["phone"].as_str().filter(|s| !s.is_empty()) { data["phone"] = json!(v); }
                                        if let Some(v) = a["address"].as_str().filter(|s| !s.is_empty()) { data["company"] = json!(v); }
                                        if let Some(v) = a["remark"].as_str().filter(|s| !s.is_empty()) { data["remark"] = json!(v); }
                                        if data.as_object().map_or(true, |m| m.len() <= 1) {
                                            let envelope = ToolResultEnvelope {
                                                result_type: "object_update_result".to_string(),
                                                data: json!({"id": partner_id, "error": "未提供任何更新字段"}),
                                                summary: Some("未提供任何更新字段。".to_string()),
                                            };
                                            envelope.to_json()
                                        } else {
                                            // 使用 block_on 调用异步函数
                                        match block_on_async(crate::commands::partner_commands::partner_update(data)) {
                                            Ok(_) => {
                                                let partner_name = database::get(
                                                    "SELECT name FROM partners WHERE id=?1",
                                                    &[json!(partner_id)],
                                                ).ok().flatten().and_then(|r| r["name"].as_str().map(String::from)).unwrap_or_default();
                                                let display_name = if partner_name.is_empty() { partner_id.to_string() } else { partner_name };
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "object_update_result".to_string(),
                                                    data: json!({"id": partner_id}),
                                                    summary: Some(format!("已更新合作伙伴「{}」", display_name)),
                                                };
                                                envelope.to_json()
                                            }
                                            Err(e) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "object_update_result".to_string(),
                                                    data: json!({"id": partner_id, "error": e.to_string()}),
                                                    summary: Some(format!("更新失败: {}", e)),
                                                };
                                                envelope.to_json()
                                            },
                                        }
                                        }
                                    }
                                }
                                "delete" => {
                                    let partner_id = a["partner_id"].as_str().unwrap_or("");
                                    if partner_id.is_empty() {
                                        let envelope = ToolResultEnvelope {
                                            result_type: "object_delete_result".to_string(),
                                            data: json!({"error": "删除需要提供 partner_id"}),
                                            summary: Some("删除需要提供 partner_id。".to_string()),
                                        };
                                        envelope.to_json()
                                    }
                                    else {
                                        // 使用 block_on 调用异步函数
                                        match block_on_async(crate::commands::partner_commands::partner_delete(partner_id.to_string())) {
                                            Ok(_) => {
                                                let partner_name = database::get(
                                                    "SELECT name FROM partners WHERE id=?1",
                                                    &[json!(partner_id)],
                                                ).ok().flatten().and_then(|r| r["name"].as_str().map(String::from)).unwrap_or_default();
                                                let display_name = if partner_name.is_empty() { partner_id.to_string() } else { partner_name };
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "object_delete_result".to_string(),
                                                    data: json!({"id": partner_id}),
                                                    summary: Some(format!("已删除合作伙伴「{}」", display_name)),
                                                };
                                                envelope.to_json()
                                            }
                                            Err(e) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "object_delete_result".to_string(),
                                                    data: json!({"id": partner_id, "error": e.to_string()}),
                                                    summary: Some(format!("删除失败: {}", e)),
                                                };
                                                envelope.to_json()
                                            },
                                        }
                                    }
                                }
                                "link_project" => {
                                    let partner_id = a["partner_id"].as_str().unwrap_or("");
                                    let project_id = a["project_id"].as_str().unwrap_or("");
                                    if partner_id.is_empty() || project_id.is_empty() {
                                        let envelope = ToolResultEnvelope {
                                            result_type: "partner_link_result".to_string(),
                                            data: json!({"success": false}),
                                            summary: Some("关联需要提供 partner_id 和 project_id。".to_string()),
                                        };
                                        envelope.to_json()
                                    }
                                    else {
                                        let contact_id = a["contact_id"].as_str().map(|s| s.to_string());
                                        let remark = a["remark"].as_str().map(|s| s.to_string());
                                        // 使用 block_on 调用异步函数
                                        match block_on_async(crate::commands::partner_commands::partner_link_project(
                                            partner_id.to_string(),
                                            project_id.to_string(),
                                            contact_id,
                                            remark
                                        )) {
                                            Ok(_) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "partner_link_result".to_string(),
                                                    data: json!({"partner_id": partner_id, "project_id": project_id, "success": true}),
                                                    summary: Some("已将合作伙伴关联到项目".to_string()),
                                                };
                                                envelope.to_json()
                                            }
                                            Err(e) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "partner_link_result".to_string(),
                                                    data: json!({"success": false}),
                                                    summary: Some(format!("关联失败: {}", e)),
                                                };
                                                envelope.to_json()
                                            },
                                        }
                                    }
                                }
                                "unlink_project" => {
                                    let partner_id = a["partner_id"].as_str().unwrap_or("");
                                    let project_id = a["project_id"].as_str().unwrap_or("");
                                    if partner_id.is_empty() || project_id.is_empty() {
                                        let envelope = ToolResultEnvelope {
                                            result_type: "partner_unlink_result".to_string(),
                                            data: json!({"success": false}),
                                            summary: Some("取消关联需要提供 partner_id 和 project_id。".to_string()),
                                        };
                                        envelope.to_json()
                                    }
                                    else {
                                        // 使用 block_on 调用异步函数
                                        match block_on_async(crate::commands::partner_commands::partner_unlink_project(
                                            partner_id.to_string(),
                                            project_id.to_string()
                                        )) {
                                            Ok(_) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "partner_unlink_result".to_string(),
                                                    data: json!({"partner_id": partner_id, "project_id": project_id, "success": true}),
                                                    summary: Some("已取消合作伙伴与项目的关联".to_string()),
                                                };
                                                envelope.to_json()
                                            }
                                            Err(e) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "partner_unlink_result".to_string(),
                                                    data: json!({"success": false}),
                                                    summary: Some(format!("取消关联失败: {}", e)),
                                                };
                                                envelope.to_json()
                                            },
                                        }
                                    }
                                }
                                _ => {
                                    let envelope = ToolResultEnvelope {
                                        result_type: "object_error".to_string(),
                                        data: json!({"error": format!("未知操作「{}」。支持：create, update, delete, link_project, unlink_project。", action)}),
                                        summary: Some(format!("未知操作「{}」。支持：create, update, delete, link_project, unlink_project。", action)),
                                    };
                                    envelope.to_json()
                                },
                            };
                            (res, vec![])
                        }
                        "ext_tool_manage_contact" => {
                            let action = a["action"].as_str().unwrap_or("");
                            let res = match action {
                                "create" => {
                                    let name = a["name"].as_str().unwrap_or("");
                                    if name.is_empty() {
                                        let envelope = ToolResultEnvelope {
                                            result_type: "object_create_result".to_string(),
                                            data: json!({"error": "创建联系人需要提供 name"}),
                                            summary: Some("创建联系人需要提供 name。".to_string()),
                                        };
                                        envelope.to_json()
                                    }
                                    else {
                                        let phone = a["phone"].as_str().unwrap_or("");
                                        let email = a["email"].as_str().unwrap_or("");
                                        let position = a["position"].as_str().unwrap_or("");
                                        let remark = a["remark"].as_str().unwrap_or("");
                                        let parent_id = a["partner_id"].as_str()
                                            .or_else(|| a["parent_id"].as_str())
                                            .unwrap_or("");
                                        let raw_proj_id = a["project_id"].as_str().unwrap_or("");
                                        let project_id = raw_proj_id.strip_prefix("PROJECT-").unwrap_or(raw_proj_id);
                                        // ★ 去重检查：同名联系人（同一合作伙伴下）
                                        let mut is_dup = false;
                                        let mut dup_msg = String::new();
                                        if !parent_id.is_empty() {
                                            if let Ok(Some(existing)) = database::get(
                                                "SELECT pc.id, pc.name FROM partner_contacts pc JOIN partner_contact_mapping pcm ON pcm.contact_id=pc.id WHERE pc.name=?1 AND pcm.partner_id=?2 LIMIT 1",
                                                &[json!(name), json!(parent_id)],
                                            ) {
                                                let eid = existing["id"].as_str().unwrap_or("");
                                                is_dup = true;
                                                dup_msg = format!("已存在同名联系人「{}」(ID: {})，未重复创建。如需新建同名联系人请先修改名称。", name, eid);
                                            }
                                        }
                                        if is_dup {
                                            let envelope = ToolResultEnvelope {
                                                result_type: "object_create_result".to_string(),
                                                data: json!({"error": dup_msg.clone()}),
                                                summary: Some(dup_msg),
                                            };
                                            envelope.to_json()
                                        } else {
                                        let data = json!({
                                            "name": name,
                                            "phone": phone,
                                            "email": email,
                                            "position": position,
                                            "remark": remark,
                                            "partnerId": parent_id
                                        });
                                        // 使用 block_on 调用异步函数
                                        match block_on_async(crate::commands::partner_commands::partner_contact_create(data)) {
                                            Ok(result) => {
                                                let id = result["id"].as_str().unwrap_or("");
                                                // 如果有项目ID，手动关联到项目
                                                if !project_id.is_empty() && !id.is_empty() {
                                                    let _ = block_on_async(crate::commands::partner_commands::contact_link_project(
                                                        id.to_string(),
                                                        project_id.to_string()
                                                    ));
                                                }
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "object_create_result".to_string(),
                                                    data: json!({"id": id, "type": "CONTACT", "name": name}),
                                                    summary: Some(format!("已创建联系人「{}」", name)),
                                                };
                                                envelope.to_json()
                                            },
                                            Err(e) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "object_create_result".to_string(),
                                                    data: json!({"error": e.to_string()}),
                                                    summary: Some(format!("创建失败: {}", e)),
                                                };
                                                envelope.to_json()
                                            },
                                        }
                                        } // close if is_dup else
                                    }
                                }
                                "update" => {
                                    let raw_cid = a["contact_id"].as_str().unwrap_or("");
                                    let contact_id = raw_cid.strip_prefix("CONTACT-").unwrap_or(raw_cid);
                                    if contact_id.is_empty() {
                                        let envelope = ToolResultEnvelope {
                                            result_type: "object_update_result".to_string(),
                                            data: json!({"error": "更新需要提供 contact_id"}),
                                            summary: Some("更新需要提供 contact_id。".to_string()),
                                        };
                                        envelope.to_json()
                                    }
                                    else {
                                        let mut data = json!({"id": contact_id});
                                        if let Some(v) = a["name"].as_str().filter(|s| !s.is_empty()) { data["name"] = json!(v); }
                                        if let Some(v) = a["phone"].as_str().filter(|s| !s.is_empty()) { data["phone"] = json!(v); }
                                        if let Some(v) = a["email"].as_str().filter(|s| !s.is_empty()) { data["email"] = json!(v); }
                                        if let Some(v) = a["position"].as_str().filter(|s| !s.is_empty()) { data["position"] = json!(v); }
                                        if let Some(v) = a["remark"].as_str().filter(|s| !s.is_empty()) { data["remark"] = json!(v); }
                                        if data.as_object().map_or(true, |m| m.len() <= 1) {
                                            let envelope = ToolResultEnvelope {
                                                result_type: "object_update_result".to_string(),
                                                data: json!({"id": contact_id, "error": "未提供任何更新字段"}),
                                                summary: Some("未提供任何更新字段。".to_string()),
                                            };
                                            envelope.to_json()
                                        } else {
                                            // 使用 block_on 调用异步函数
                                            match block_on_async(crate::commands::partner_commands::partner_contact_update(data)) {
                                                Ok(_) => {
                                                    let contact_name = database::get(
                                                        "SELECT name FROM partner_contacts WHERE id=?1",
                                                        &[json!(contact_id)],
                                                    ).ok().flatten().and_then(|r| r["name"].as_str().map(String::from)).unwrap_or_default();
                                                    let display_name = if contact_name.is_empty() { contact_id.to_string() } else { contact_name };
                                                    let envelope = ToolResultEnvelope {
                                                        result_type: "object_update_result".to_string(),
                                                        data: json!({"id": contact_id}),
                                                        summary: Some(format!("已更新联系人「{}」", display_name)),
                                                    };
                                                    envelope.to_json()
                                                }
                                                Err(e) => {
                                                    let envelope = ToolResultEnvelope {
                                                        result_type: "object_update_result".to_string(),
                                                        data: json!({"id": contact_id, "error": e.to_string()}),
                                                        summary: Some(format!("更新失败: {}", e)),
                                                    };
                                                    envelope.to_json()
                                                },
                                            }
                                        }
                                    }
                                }
                                "delete" => {
                                    let contact_id = a["contact_id"].as_str().unwrap_or("");
                                    if contact_id.is_empty() {
                                        let envelope = ToolResultEnvelope {
                                            result_type: "object_delete_result".to_string(),
                                            data: json!({"error": "删除需要提供 contact_id"}),
                                            summary: Some("删除需要提供 contact_id。".to_string()),
                                        };
                                        envelope.to_json()
                                    }
                                    else {
                                        // 使用 block_on 调用异步函数
                                        match block_on_async(crate::commands::partner_commands::partner_contact_delete(contact_id.to_string())) {
                                            Ok(_) => {
                                                let contact_name = database::get(
                                                    "SELECT name FROM partner_contacts WHERE id=?1",
                                                    &[json!(contact_id)],
                                                ).ok().flatten().and_then(|r| r["name"].as_str().map(String::from)).unwrap_or_default();
                                                let display_name = if contact_name.is_empty() { contact_id.to_string() } else { contact_name };
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "object_delete_result".to_string(),
                                                    data: json!({"id": contact_id}),
                                                    summary: Some(format!("已删除联系人「{}」", display_name)),
                                                };
                                                envelope.to_json()
                                            }
                                            Err(e) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "object_delete_result".to_string(),
                                                    data: json!({"id": contact_id, "error": e.to_string()}),
                                                    summary: Some(format!("删除失败: {}", e)),
                                                };
                                                envelope.to_json()
                                            },
                                        }
                                    }
                                }
                                "link_project" => {
                                    let contact_id = a["contact_id"].as_str().unwrap_or("");
                                    let project_id = a["project_id"].as_str().unwrap_or("");
                                    if contact_id.is_empty() || project_id.is_empty() {
                                        let envelope = ToolResultEnvelope {
                                            result_type: "contact_link_result".to_string(),
                                            data: json!({"success": false}),
                                            summary: Some("关联需要提供 contact_id 和 project_id。".to_string()),
                                        };
                                        envelope.to_json()
                                    }
                                    else {
                                        // 使用 block_on 调用异步函数
                                        match block_on_async(crate::commands::partner_commands::contact_link_project(
                                            contact_id.to_string(),
                                            project_id.to_string()
                                        )) {
                                            Ok(_) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "contact_link_result".to_string(),
                                                    data: json!({"contact_id": contact_id, "project_id": project_id, "success": true}),
                                                    summary: Some("已将联系人关联到项目".to_string()),
                                                };
                                                envelope.to_json()
                                            }
                                            Err(e) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "contact_link_result".to_string(),
                                                    data: json!({"success": false}),
                                                    summary: Some(format!("关联失败: {}", e)),
                                                };
                                                envelope.to_json()
                                            },
                                        }
                                    }
                                }
                                "unlink_project" => {
                                    let contact_id = a["contact_id"].as_str().unwrap_or("");
                                    let project_id = a["project_id"].as_str().unwrap_or("");
                                    if contact_id.is_empty() || project_id.is_empty() {
                                        let envelope = ToolResultEnvelope {
                                            result_type: "contact_unlink_result".to_string(),
                                            data: json!({"success": false}),
                                            summary: Some("取消关联需要提供 contact_id 和 project_id。".to_string()),
                                        };
                                        envelope.to_json()
                                    }
                                    else {
                                        // 使用 block_on 调用异步函数
                                        match block_on_async(crate::commands::partner_commands::contact_unlink_project(
                                            contact_id.to_string(),
                                            project_id.to_string()
                                        )) {
                                            Ok(_) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "contact_unlink_result".to_string(),
                                                    data: json!({"contact_id": contact_id, "project_id": project_id, "success": true}),
                                                    summary: Some("已取消联系人与项目的关联".to_string()),
                                                };
                                                envelope.to_json()
                                            }
                                            Err(e) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "contact_unlink_result".to_string(),
                                                    data: json!({"success": false}),
                                                    summary: Some(format!("取消关联失败: {}", e)),
                                                };
                                                envelope.to_json()
                                            },
                                        }
                                    }
                                }
                                _ => {
                                    let envelope = ToolResultEnvelope {
                                        result_type: "object_error".to_string(),
                                        data: json!({"error": format!("未知操作「{}」。支持：create, update, delete, link_project, unlink_project。", action)}),
                                        summary: Some(format!("未知操作「{}」。支持：create, update, delete, link_project, unlink_project。", action)),
                                    };
                                    envelope.to_json()
                                },
                            };
                            (res, vec![])
                        }
                        "ext_tool_manage_notification" => {
                            let action = a["action"].as_str().unwrap_or("create");
                            let res = match action {
                                "get" => {
                                    let nid = a["notification_id"].as_str().unwrap_or("");
                                    if nid.is_empty() {
                                        let envelope = ToolResultEnvelope {
                                            result_type: "notification_get_result".to_string(),
                                            data: json!({"success": false}),
                                            summary: Some("请提供 notification_id。".to_string()),
                                        };
                                        envelope.to_json()
                                    } else {
                                        match crate::database::get(
                                            "SELECT id, type, title, content, entity_type, entity_id, priority, source, follow_up_of, created_at FROM notifications WHERE id=?1",
                                            &[json!(nid)],
                                        ) {
                                            Ok(Some(r)) => {
                                                let ntype = r["type"].as_str().unwrap_or("");
                                                let title = r["title"].as_str().unwrap_or("");
                                                let content = r["content"].as_str().unwrap_or("");
                                                let entity_type = r["entity_type"].as_str().unwrap_or("");
                                                let entity_id = r["entity_id"].as_str().unwrap_or("");
                                                let priority = r["priority"].as_str().unwrap_or("medium");
                                                let follow_up_of = r["follow_up_of"].as_str().unwrap_or("");
                                                let created = r["created_at"].as_str().unwrap_or("");
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "notification_get_result".to_string(),
                                                    data: json!({
                                                        "id": nid,
                                                        "type": ntype,
                                                        "title": title,
                                                        "content": content,
                                                        "entity_type": entity_type,
                                                        "entity_id": entity_id,
                                                        "priority": priority,
                                                        "follow_up_of": follow_up_of,
                                                        "created_at": created
                                                    }),
                                                    summary: Some(format!("通知「{}」", title)),
                                                };
                                                envelope.to_json()
                                            }
                                            Ok(None) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "notification_get_result".to_string(),
                                                    data: json!({"success": false}),
                                                    summary: Some(format!("未找到通知: {}", nid)),
                                                };
                                                envelope.to_json()
                                            }
                                            Err(e) => {
                                                let envelope = ToolResultEnvelope {
                                                    result_type: "notification_get_result".to_string(),
                                                    data: json!({"success": false}),
                                                    summary: Some(format!("查询失败: {}", e)),
                                                };
                                                envelope.to_json()
                                            },
                                        }
                                    }
                                }
                                "list" => {
                                    let limit = a["limit"].as_u64().unwrap_or(20) as usize;
                                    let offset = a["offset"].as_u64().unwrap_or(0) as usize;
                                    let sql = "SELECT id, type, title, content, entity_type, entity_id, priority, source, follow_up_of, created_at FROM notifications ORDER BY created_at DESC LIMIT ?1 OFFSET ?2";
                                    match crate::database::all(sql, &[json!(limit), json!(offset)]) {
                                        Ok(rows) => {
                                            let items: Vec<Value> = rows.iter().map(|r| {
                                                json!({
                                                    "id": r["id"].as_str().unwrap_or(""),
                                                    "type": r["type"].as_str().unwrap_or(""),
                                                    "title": r["title"].as_str().unwrap_or(""),
                                                    "priority": r["priority"].as_str().unwrap_or("medium"),
                                                    "created_at": r["created_at"].as_str().unwrap_or("")
                                                })
                                            }).collect();
                                            let envelope = ToolResultEnvelope {
                                                result_type: "notification_list_result".to_string(),
                                                data: json!({
                                                    "total": items.len(),
                                                    "offset": offset,
                                                    "items": items
                                                }),
                                                summary: Some(if items.is_empty() { "暂无通知。".to_string() } else { format!("共 {} 条通知。", items.len()) }),
                                            };
                                            envelope.to_json()
                                        }
                                        Err(e) => {
                                            let envelope = ToolResultEnvelope {
                                                result_type: "notification_list_result".to_string(),
                                                data: json!({"success": false}),
                                                summary: Some(format!("查询失败: {}", e)),
                                            };
                                            envelope.to_json()
                                        },
                                    }
                                }
                                "delete" => {
                                    let nid = a["notification_id"].as_str().unwrap_or("");
                                    if nid.is_empty() {
                                        let envelope = ToolResultEnvelope {
                                            result_type: "notification_delete_result".to_string(),
                                            data: json!({"success": false}),
                                            summary: Some("请提供 notification_id。".to_string()),
                                        };
                                        envelope.to_json()
                                    } else {
                                        let notification_title = database::get(
                                            "SELECT title FROM notifications WHERE id=?1",
                                            &[json!(nid)],
                                        ).ok().flatten().and_then(|r| r["title"].as_str().map(String::from)).unwrap_or_default();
                                        let _ = database::run("DELETE FROM notifications WHERE id=?1", &[json!(nid)]);
                                        if let Some(ref h) = app_handle { let _ = h.emit("notification:deleted", &nid); }
                                        let display_name = if notification_title.is_empty() { nid.to_string() } else { notification_title };
                                        let envelope = ToolResultEnvelope {
                                            result_type: "notification_delete_result".to_string(),
                                            data: json!({"id": nid, "success": true}),
                                            summary: Some(format!("已删除通知「{}」", display_name)),
                                        };
                                        envelope.to_json()
                                    }
                                }
                                "update" => {
                                    let nid = a["notification_id"].as_str().unwrap_or("");
                                    if nid.is_empty() {
                                        let envelope = ToolResultEnvelope {
                                            result_type: "notification_update_result".to_string(),
                                            data: json!({"success": false}),
                                            summary: Some("请提供 notification_id。".to_string()),
                                        };
                                        envelope.to_json()
                                    } else {
                                        let mut sets = Vec::new();
                                        let mut params: Vec<Value> = Vec::new();
                                        if let Some(t) = a.get("type").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
                                            sets.push("type=?"); params.push(json!(t));
                                        }
                                        if let Some(t) = a.get("title").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
                                            sets.push("title=?"); params.push(json!(t));
                                        }
                                        if let Some(c) = a.get("content").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
                                            sets.push("content=?"); params.push(json!(c));
                                        }
                                        if let Some(p) = a.get("priority").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
                                            sets.push("priority=?"); params.push(json!(p));
                                        }
                                        if let Some(et) = a.get("entity_type").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
                                            sets.push("entity_type=?"); params.push(json!(et));
                                        }
                                        if let Some(ei) = a.get("entity_id").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
                                            sets.push("entity_id=?"); params.push(json!(ei));
                                        }
                                        if sets.is_empty() {
                                            let envelope = ToolResultEnvelope {
                                                result_type: "notification_update_result".to_string(),
                                                data: json!({"id": nid, "success": false}),
                                                summary: Some("没有要更新的字段。".to_string()),
                                            };
                                            envelope.to_json()
                                        } else {
                                            let sql = format!("UPDATE notifications SET {} WHERE id=?", sets.join(", "));
                                            params.push(json!(nid));
                                            let _ = database::run(&sql, &params);
                                            let notification_title = database::get(
                                                "SELECT title FROM notifications WHERE id=?1",
                                                &[json!(nid)],
                                            ).ok().flatten().and_then(|r| r["title"].as_str().map(String::from)).unwrap_or_default();
                                            let display_name = if notification_title.is_empty() { nid.to_string() } else { notification_title };
                                            let envelope = ToolResultEnvelope {
                                                result_type: "notification_update_result".to_string(),
                                                data: json!({"id": nid, "success": true}),
                                                summary: Some(format!("已更新通知「{}」", display_name)),
                                            };
                                            envelope.to_json()
                                        }
                                    }
                                }
                                _ => { // create
                                    let ntype = a["type"].as_str().unwrap_or("llm_insight");
                                    let title = crate::commands::notification_commands::sanitize_notification_text(a["title"].as_str().unwrap_or(""));
                                    let content = crate::commands::notification_commands::sanitize_notification_text(a["content"].as_str().unwrap_or(""));
                                    let entity_type = a["entity_type"].as_str().unwrap_or("");
                                    let entity_id = a["entity_id"].as_str().unwrap_or("");
                                    let priority = a["priority"].as_str().unwrap_or("medium");
                                    let follow_up_of = a["follow_up_of"].as_str().unwrap_or("");
                                    let is_dup = if !entity_id.is_empty() {
                                        database::get(
                                            "SELECT id FROM notifications WHERE type=?1 AND entity_id=?2 AND created_at > datetime('now','-1 day')",
                                            &[json!(ntype), json!(entity_id)]
                                        ).ok().flatten().is_some()
                                    } else { false };
                                    if is_dup {
                                        let envelope = ToolResultEnvelope {
                                            result_type: "notification_create_result".to_string(),
                                            data: json!({"success": false}),
                                            summary: Some("通知已存在，跳过创建".to_string()),
                                        };
                                        envelope.to_json()
                                    } else {
                                        let id = uuid::Uuid::new_v4().to_string();
                                        let _ = database::run(
                                            "INSERT INTO notifications(id,type,title,content,entity_type,entity_id,priority,source,follow_up_of) VALUES(?1,?2,?3,?4,?5,?6,?7,'llm',?8)",
                                            &[json!(&id), json!(ntype), json!(title), json!(content), json!(entity_type), json!(entity_id), json!(priority), json!(follow_up_of)],
                                        );
                                        if let Some(ref h) = app_handle { let _ = h.emit("notification:new", &id); }
                                        let envelope = ToolResultEnvelope {
                                            result_type: "notification_create_result".to_string(),
                                            data: json!({"id": id, "title": title, "success": true}),
                                            summary: Some(format!("已创建通知「{}」", title)),
                                        };
                                        envelope.to_json()
                                    }
                                }
                            };
                            (res, vec![])
                        }
                        // ── 演示材料工具 ─────────────────────────
                        "pres_tool_generate_page" => {
                            let page = a["page"].as_i64().unwrap_or(1) as usize;
                            let instruction = a["instruction"].as_str().unwrap_or("");
                            let total = a["totalPages"].as_i64().unwrap_or(10) as usize;
                            log::info!("[PresentationTool] generate_page page={}, instruction_len={}", page, instruction.len());
                            let result = match crate::services::presentation_ai::generate_page_html(page, total, instruction, &app_handle).await {
                                Ok(html) => {
                                    log::info!("[PresentationTool] generate_page success, html_len={}", html.len());
                                    if let Some(ref h) = app_handle { let _ = h.emit("presentation:page-generated", json!({"page": page, "html": html})); }
                                    format!("第{}页已生成 ({} 字符)", page, html.len())
                                }
                                Err(e) => {
                                    log::error!("[PresentationTool] generate_page failed: {}", e);
                                    format!("生成失败: {}", e)
                                }
                            };
                            (result, vec![])
                        }
                        "pres_tool_regenerate_slide" => {
                            let page = a["page"].as_i64().unwrap_or(1) as usize;
                            let instruction = a["instruction"].as_str().unwrap_or("");
                            let current_html = a["currentHtml"].as_str().unwrap_or("");
                            log::info!("[PresentationTool] regenerate_slide page={}, instruction_len={}", page, instruction.len());
                            let result = match crate::services::presentation_ai::regenerate_slide_html(page, instruction, current_html, &app_handle).await {
                                Ok(html) => {
                                    log::info!("[PresentationTool] regenerate_slide success, html_len={}", html.len());
                                    if let Some(ref h) = app_handle { let _ = h.emit("presentation:page-updated", json!({"page": page, "html": html})); }
                                    format!("第{}页已重新设计 ({} 字符)", page, html.len())
                                }
                                Err(e) => {
                                    log::error!("[PresentationTool] regenerate_slide failed: {}", e);
                                    format!("重新设计失败: {}", e)
                                }
                            };
                            (result, vec![])
                        }
                        "pres_tool_modify_slide" => {
                            let page = a["page"].as_i64().unwrap_or(1) as usize;
                            let instruction = a["instruction"].as_str().unwrap_or("");
                            let current_html = a["currentHtml"].as_str().unwrap_or("");
                            let selector = a["elementSelector"].as_str().unwrap_or("");
                            log::info!("[PresentationTool] modify_slide page={}, instruction_len={}, selector={}", page, instruction.len(), selector);
                            let result = match crate::services::presentation_ai::modify_slide_html(page, instruction, current_html, selector, &app_handle).await {
                                Ok(html) => {
                                    log::info!("[PresentationTool] modify_slide success, html_len={}", html.len());
                                    if let Some(ref h) = app_handle { let _ = h.emit("presentation:page-updated", json!({"page": page, "html": html})); }
                                    format!("第{}页已修改 ({} 字符)", page, html.len())
                                }
                                Err(e) => {
                                    log::error!("[PresentationTool] modify_slide failed: {}", e);
                                    format!("修改失败: {}", e)
                                }
                            };
                            (result, vec![])
                        }
                        _ => (format!("未知工具: {}", name), vec![]),
                    };
                    sync_results.push((*idx, result, effects));
                }

                // 合并所有结果并按原始顺序排列
                let mut all_results: Vec<(usize, String, Vec<SessionEffect>)> = Vec::new();
                all_results.extend(async_results);
                all_results.extend(sync_results);
                all_results.sort_by_key(|(idx, _, _)| *idx);

                // 合并缓存结果
                for (idx, cached) in &cached_results_map {
                    all_results.push((*idx, cached.clone(), vec![]));
                }
                all_results.sort_by_key(|(idx, _, _)| *idx);

                // 顺序应用 SessionEffects + 构建 round_tools + messages
                for (idx, result, effects) in &all_results {
                    for effect in effects {
                        session.apply_effect(effect);
                    }

                    let tc = &atc[*idx];
                    let key = (tc.function.name.clone(), tc.function.arguments.clone());
                    dedup_cache.insert(key, result.clone());
                    if let Some(ref h) = app_handle {
                        let _ = h.emit("agent:tool-result", json!({"round": iters, "tool": tc.function.name, "result_len": result.chars().count()}));
                    }
                    let parsed_args: Value = serde_json::from_str(&tc.function.arguments).unwrap_or(json!({}));
                    round_tools.push(json!({"tool":tc.function.name,"args":parsed_args,"result":result,"result_len":result.chars().count()}));
                    messages.push(ChatMessage{role:MessageRole::Tool,content:Some(result.clone()),tool_calls:None,tool_call_id:Some(tc.id.clone()),reasoning_content:None,name:Some(tc.function.name.clone())});
                }
                current_round["tools"] = json!(round_tools);
                tool_log.push(current_round);
                // ★ 同时记录工具调用历史到检索计划
                for t in &round_tools {
                    let result_str = t["result"].as_str().unwrap_or("");
                    let mut step_desc = t["tool"].as_str().unwrap_or("").to_string();
                    if let Some(args) = t["args"].as_object() {
                        if step_desc == "ext_tool_search" {
                            if let Some(kw) = args.get("keywords").and_then(|v| v.as_array()) {
                                step_desc = format!("search({})", kw.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join(", "));
                            }
                        } else if step_desc == "ext_tool_object" {
                            if let Some(q) = args.get("query").and_then(|v| v.as_str()) {
                                step_desc = format!("object(\"{}\")", q);
                            }
                        } else if step_desc == "ext_tool_chunk_navigate" {
                            if let Some(cid) = args.get("chunk_id").and_then(|v| v.as_str()) {
                                step_desc = format!("traverse({}…)", &cid[..cid.len().min(8)]);
                            }
                        }
                    }
                    session.tool_history.push(step_desc);
                    let tool_name = t["tool"].as_str().unwrap_or("");
                    session.update_from_tool_result(tool_name, &result_str);
                }

                // ★ 状态驱动熔断检查
                let mut circuit_break_reason = String::new();

                // 条件②：连续两轮没有新 chunk 或活跃对象（用它们作为新事实的代理指标）
                if circuit_break_reason.is_empty() && iters >= 4 {
                    // 只在第 4 轮后检查，给初始搜索留出空间
                    let stale = tool_log.iter().rev().take(2).all(|r| {
                        r["tools"].as_array().map(|tools| tools.iter().all(|t| {
                            let result = t["result"].as_str().unwrap_or("");
                            result.len() < 50 || result.contains("未找到")
                        })).unwrap_or(true)
                    });
                    if stale { circuit_break_reason = "当前范围未发现新内容，尝试 search 新关键词".to_string(); }
                }

                // 条件④：Search 关键词重复率过高（与最近一次搜索比较）
                if let Some(last_search) = round_tools.iter().find(|t| {
                    matches!(t["tool"].as_str(), Some("ext_tool_search"))
                }) {
                    if let Some(args) = last_search["args"].as_object() {
                        if let Some(kws) = args.get("keywords").and_then(|v| v.as_array()) {
                            let cur: std::collections::HashSet<String> = kws.iter().filter_map(|v| v.as_str().map(String::from)).collect();
                            if !cur.is_empty() {
                                for prev in session.search_keywords.iter().rev().skip(1).take(3) {
                                    let prev_set: std::collections::HashSet<String> = prev.iter().cloned().collect();
                                    let intersection = cur.intersection(&prev_set).count();
                                    let union = cur.union(&prev_set).count();
                                    if union > 0 && (intersection as f64 / union as f64) > 0.6 {
                                        circuit_break_reason = "Search 关键词重复，陷入循环".to_string();
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }

                // ★ 条件⑤：本轮 search 未产生新 chunk（来源已全部访问过）
                if circuit_break_reason.is_empty() {
                    let no_new_chunks: bool = round_tools.iter().any(|t| {
                        if t["tool"].as_str() == Some("ext_tool_search") {
                            let result = t["result"].as_str().unwrap_or("");
                            // 没有 ChunkID 行 = 没有新父块
                            !result.contains("ChunkID：")
                        } else { false }
                    });
                    if no_new_chunks && !session.observations.is_empty() {
                        circuit_break_reason = "当前搜索范围已无新内容".to_string();
                    }
                }

                if !circuit_break_reason.is_empty() {
                    messages.push(ChatMessage{role:MessageRole::User,content:Some(format!("【系统提示】{circuit_break_reason}。请根据已有 Known Facts 直接总结回答。")),tool_calls:None,tool_call_id:None,name:None,reasoning_content:None});
                    let ans = call_llm_without_tools(&messages, &app_handle).await.unwrap_or_else(|e| { log::warn!("[Agent] 熔断总结失败: {e}"); format!("总结失败: {e}") });
                    let ws = session.generate_scratchpad();
                    if !silent { if let Some(ref h) = app_handle { let _ = h.emit("agent:status", json!({"stage":"answer"})); let _ = h.emit("agent:final-answer", json!({"answer": ans})); } }
                    if let Some(ref sid) = session_id { save_session_state(sid, &session); }
                    return Ok(json!({"success":true,"result_type":"answer","answer":ans,"workspace":ws,"iterations":iters,"circuit_breaker":true,"promptTokens":session.prompt_tokens,"completionTokens":session.completion_tokens,"cachedTokens":session.cached_tokens,"tool_log":tool_log}));
                }

                // ★ 扣减预算（工具执行后，下一轮生效）
                budget -= search_cost;
                search_cost = ((search_cost as f64 * 1.2) as i32).max(1);

                // ★ 动态上下文注入到消息末尾（工具执行后），确保下一轮 LLM 调用时
                // 前缀为 [System Prompt] + [History] + [Tool Calls/Results...] + [Dynamic Context]
                // 工具结果成为稳定前缀，可被 prompt cache 命中
                {
                    let scratchpad = session.generate_scratchpad();
                    let status = if budget >= 100 { "预算充足" } else if budget >= 50 { "预算偏低" } else { "预算紧张，请优先总结已有信息" };
                    // ★ 计算当前上下文 token 数
                    let context_tokens: i64 = messages.iter().map(|m| {
                        let text_len = m.content.as_ref().map_or(0, |s| s.len() as i64);
                        let tc_len = m.tool_calls.as_ref().map_or(0, |tcs| tcs.iter()
                            .map(|tc| tc.function.arguments.len() as i64)
                            .sum::<i64>());
                        (text_len + tc_len) / 3
                    }).sum();
                     let budget_info = format!(
                        "当前会话工作记忆\n{}\n\n当前时间: {}\n\n搜索预算系统（单次对话）:\n- 总预算: 150 分\n- 计算方式: 每轮扣减，首次8分，逐轮×1.2递增（如8→9→11→13→...）\n- 当前轮次: 第{}轮，本轮将消耗: {}分\n- 剩余预算: {}/150 — {}\n- 当前上下文: {} tokens\n- 建议: 信息足够时主动回答，不要等到预算耗尽",
                        scratchpad, chrono::Local::now().format("%Y-%m-%d %A %H:%M:%S").to_string(), iters, search_cost, budget, status, context_tokens
                    );
                    // 移除上一轮的动态上下文消息
                    if let Some(pos) = messages.iter().rposition(|m| m.content.as_deref().map_or(false, |c| c.starts_with("[DYNAMIC_CONTEXT]"))) {
                        messages.remove(pos);
                    }
            messages.push(ChatMessage{role:MessageRole::User, content: Some(format!("[DYNAMIC_CONTEXT]\n{}\n\n{}", user_policies, budget_info)), tool_calls: None, tool_call_id: None, name: None, reasoning_content: None});
                }
                // ★ 调试：每轮工具调用后保存完整上下文 —— 仅 debug 构建
                #[cfg(debug_assertions)]
                {
                    let context_json: Vec<Value> = messages.iter().filter_map(|m| {
                        serde_json::to_value(m).ok()
                    }).collect();
                    let _ = crate::commands::save_agent_context(context_json);
                }
                // ★ 保存 session 缓存
                if let Some(ref sid) = session_id {
                    save_session_state(sid, &session);
                }
            }
            _ => { if let Some(ref sid) = session_id { save_session_state(sid, &session); } let ws = session.generate_scratchpad(); return Ok(json!({"success":true,"result_type":"answer","answer":format!("根据当前信息：\n\n{}",effective_ac),"workspace":ws,"iterations":iters,"promptTokens":session.prompt_tokens,"completionTokens":session.completion_tokens,"cachedTokens":session.cached_tokens,"tool_log":tool_log})); }
        }
    }
}
