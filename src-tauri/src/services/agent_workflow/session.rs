//! 会话记忆：SessionMemory / ActiveObject / Focus / 注意力衰减 / scope 计算

use serde_json::json;
use crate::services::agent_workflow::ToolResultEnvelope;
use crate::services::agent_workflow::tools::search::resolve_scope_to_rowids;

// Session Memory — 会话工作记忆层
// ═══════════════════════════════════════════════════════════════════
pub(crate) const ACCESS_STALE_THRESHOLD: usize = 20;   // 超过 20 次访问未命中即淘汰
pub(crate) const SCRATCHPAD_MAX_CHARS: usize = 4000;     // scratchpad 最长字符数

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct ActiveObject {
    pub(crate) obj_type: String,
    pub(crate) obj_id: String,
    pub(crate) name: String,
    pub(crate) access_order: usize,
    pub(crate) hits: usize,
    pub(crate) score: f64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct Focus {
    pub(crate) entities: Vec<String>,
    #[allow(dead_code)]
    pub(crate) intent: String,
    pub(crate) open_questions: Vec<String>,
    pub(crate) focus_objects: Vec<String>,  // 当前焦点对象 ID（由 Goal 决定，不是 score）
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionMemory {
    pub(crate) active_objects: Vec<ActiveObject>,
    pub(crate) visited_chunks: std::collections::HashSet<String>,
    pub(crate) visited_sources: std::collections::HashSet<String>,
    pub(crate) shown_memory_ids: std::collections::HashSet<String>,  // MEMORY 跨轮次去重
    pub(crate) focus: Focus,
    pub(crate) observations: Vec<String>,
    pub(crate) hypotheses: Vec<String>,
    pub(crate) access_counter: usize,
    pub(crate) search_keywords: Vec<Vec<String>>,
    pub(crate) tool_history: Vec<String>,  // 工具调用历史（用于生成检索计划）
    pub(crate) prompt_tokens: i64,  // 累计 prompt tokens
    pub(crate) completion_tokens: i64,  // 累计 cached tokens
    pub(crate) cached_tokens: i64,  // 累计 cached tokens
    #[serde(skip)]
    pub(crate) prev_visited_chunks: std::collections::HashSet<String>,  // 本次提问开始时的快照（不持久化）
    #[serde(skip)]
    pub(crate) prev_visited_sources: std::collections::HashSet<String>,  // 同上
}

// ★ Session 状态缓存：从 chat_sessions.session_state 加载/保存 SessionMemory
pub fn load_session_state(session_id: &str) -> Option<SessionMemory> {
    let json_str = crate::database::get(
        "SELECT session_state FROM chat_sessions WHERE id=?1 AND session_state IS NOT NULL",
        &[json!(session_id)],
    ).ok().flatten()?.get("session_state")?.as_str()?.to_string();
    serde_json::from_str(&json_str).ok()
}

pub(crate) fn save_session_state(session_id: &str, session: &SessionMemory) {
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
    pub(crate) fn new() -> Self {
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
    pub(crate) fn update(&mut self, obj_type: &str, obj_id: &str, name: &str, score_delta: f64) {
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
    pub(crate) fn update_from_line(&mut self, _tool_name: &str, line: &str) {
        let tl = line.trim();
        if let Some(cid) = tl.strip_prefix("ChunkID：").map(|s| s.trim()) {
            self.mark_chunk_visited(cid);
        }
    }

    /// 根据工具名返回注意力分数增量（EMA 公式：new_score = old_score * 0.95 + delta）
    pub(crate) fn score_delta_for_tool(tool_name: &str) -> f64 {
        match tool_name {
            "ext_tool_search" | "ext_tool_batch_search" => 0.9,
            "ext_tool_object" => 0.8,
            "ext_tool_relations" => 0.6,
            "ext_tool_chunk_navigate" => 0.6,
            "ext_tool_manage_todo" | "ext_tool_manage_project" | "ext_tool_manage_partner" |
            "ext_tool_manage_contact" | "ext_tool_manage_notification" | "ext_tool_manage_product" |
            "ext_tool_manage_note" => 0.5,
            "ext_tool_memory" => 0.0,
            _ => 0.3,
        }
    }

    /// 从 JSON 信封格式更新会话状态
    pub(crate) fn update_from_envelope(&mut self, tool_name: &str, envelope: &ToolResultEnvelope) {
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
    pub(crate) fn evict_stale(&mut self) {
        let threshold = if self.access_counter > ACCESS_STALE_THRESHOLD { self.access_counter - ACCESS_STALE_THRESHOLD } else { 0 };
        self.active_objects.retain(|o| o.access_order >= threshold);
    }

    pub(crate) fn mark_chunk_visited(&mut self, chunk_id: &str) { self.visited_chunks.insert(chunk_id.to_string()); }
    pub(crate) fn mark_source_visited(&mut self, source_key: &str) { self.visited_sources.insert(source_key.to_string()); }
    pub(crate) fn record_search(&mut self, keywords: &[String]) { if !keywords.is_empty() { self.search_keywords.push(keywords.to_vec()); } }
    pub fn record_observation(&mut self, text: &str) {
        let t = text.trim();
        if !t.is_empty() && t.chars().count() > 10 {
            self.observations.insert(0, t.chars().take(200).collect());
            self.observations.truncate(5);
        }
    }
    pub(crate) fn set_focus_from_user_query(&mut self, query: &str, top_entities: &[String]) {
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
pub(crate) enum SessionEffect {
    RecordSearch(Vec<String>),
    MarkSourceVisited(String),
    UpdateObject { obj_type: String, obj_id: String, name: String, score_delta: f64 },
    #[allow(dead_code)]
    MarkChunkVisited(String),
}

impl SessionMemory {
    pub(crate) fn apply_effect(&mut self, effect: &SessionEffect) {
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
pub(crate) fn compute_scope_from_session(session: &SessionMemory) -> Option<std::collections::HashSet<i64>> {
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

