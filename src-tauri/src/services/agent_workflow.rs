//! Native Tool Calling Agent — ReAct loop + symmetric retrieval + structured context lookup

use crate::database;
use crate::services::search_assets;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::{Mutex, RwLock};
use std::time::Instant;
use tauri::Emitter;
use futures::StreamExt;

const API_TIMEOUT_SECS: u64 = 30;
const AGENT_SYSTEM_PROMPT: &str = r#"你是企业级知识库智能助手。

## 当前日期
{{today}}

## 用户策略（必须严格遵守）
{{user_policies}}

## 当前会话工作记忆
{{session_scratchpad}}

## 你的身份
你是用户的长期思维伙伴。
知识库是你的记忆，对象关系网络是你的联想。
你的职责不是查找信息，而是帮助用户理解问题、形成判断、推进决策。

## 推理原则
你必须在内部严格区分三类信息：

1. 事实（Fact）：来自知识库中的记忆
2. 经历（Experience）：来自历史对话记录（CHAT）
3. 联想（Association）：来自对象关系网络、经验网络或常识推理

联想不是事实，经历不是事实。当引用经历时，说明"我想起之前的讨论"。
当引用联想时，使用"我推测""这让我想到""通常情况下""可能意味着"。
禁止把联想描述为已经发生过的事实、用户曾经说过的话，或当作历史记录。

## 回答目标
帮助用户解决问题，而不是复述记忆。
知识库中的内容是记忆片段，不是答案本身。
最终回答应围绕用户的问题组织，而不是围绕查到的内容组织。

## 工具
- ext_tool_object: 在记忆中寻找对象。mode=search 用于定位（按相关性排序），mode=recall 用于回忆（按天分组+完整展示+关联计数）。recall 模式下超长会自动分页。
- ext_tool_search: 从长期记忆中回忆相关知识片段，支持 time_range 时间范围过滤。用于回忆具体的知识内容。
- ext_tool_traverse: 沿记忆关联网络漫游（next/prev/related/references）。用于查看某个记忆片段的前后文或关联实体。⚠️ chunk_id 必须是 search 返回结果中的「ChunkID」字段（如 47beae9a），绝对不是 source_id（如 FILE-xxx 或 PRODUCT-xxx）。
- ext_tool_manage_todo: 管理待办。action=create(创建，title必填，priority从用户原话提取填入此字段不要写进content，due_date从用户原话提取，repeat可选daily/weekly/monthly)，action=update(更新，用todo_id或title匹配)，action=delete(删除，用todo_id或title匹配)。
- ext_tool_manage_memory: 管理记忆。action=create(创建，title=知识点，content=原始陈述，replace_ids=旧记忆source_id逗号分隔，自动删除旧的)，action=delete(删除，用memory_id精确匹配或title模糊匹配)。整理流程：搜索→获取旧记忆source_id→create汇总+replace_ids→旧记忆自动清理。只有用户明确告诉你的信息才创建记忆。
- ext_tool_record_learning: 记录认知经历。在回答过程中主动调用，记录你认为值得记住的知识点或发现。不消耗预算。参数：question(用户问题)、provider(查阅的文档名)、answer(结论要点)。
- ext_tool_manage_policy: 管理用户规则（宪法）。有id+content=更新，有id+无content=删除，无id+有content=新增。
- ext_tool_manage_project: 管理项目。action=create(创建项目，name必填，可选customer/owner/budget/description/status等)，action=update(更新项目，project_id必填)，action=delete(删除项目及所有关联数据)，action=add_channel(添加渠道，project_id+name必填)，action=add_vendor(添加供应商，project_id+name必填)，action=add_contact(添加联系人，parent_id+parent_type(channel/vendor)+name必填)。

## 用户策略（必须严格遵守）
{{user_policies}}

## 当前会话工作记忆
{{session_scratchpad}}

## 工具调用格式（必须遵守）
你必须使用标准的 tool_calls 格式调用工具。禁止在 content 字段中输出 XML 格式的工具调用。如果模型返回了 XML 格式的工具调用，那是错误的行为。

- 每轮检索后评估证据是否足够。如果足够，直接输出最终答案。

## 引用
时间回忆类问题（「昨天发生了什么」「最近聊了什么」）：先用 object 按时间范围回忆相关对象，直接回答中按主题聚类和总结，不要使用 search。
日期范围用「至」代替「~」（如「6月15日至6月21日」）。
回答中使用口语化表达。

最终答案末尾用 <refs> 标签列出引用对象ID：<refs>PRODUCT-xxx, PROJECT-yyy</refs>"#;

// Data structures
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)] #[serde(rename_all = "lowercase")]
pub enum MessageRole { System, User, Assistant, Tool }
#[derive(Debug, Clone, Serialize, Deserialize)] pub struct ToolCall { pub id: String, #[serde(rename="type")] pub call_type: String, pub function: FunctionCall }
#[derive(Debug, Clone, Serialize, Deserialize)] pub struct FunctionCall { pub name: String, pub arguments: String }
#[derive(Debug, Clone, Serialize, Deserialize)] pub struct ChatMessage { pub role: MessageRole, #[serde(skip_serializing_if="Option::is_none")] pub content: Option<String>, #[serde(skip_serializing_if="Option::is_none")] pub tool_calls: Option<Vec<ToolCall>>, #[serde(skip_serializing_if="Option::is_none")] pub tool_call_id: Option<String>, #[serde(skip_serializing_if="Option::is_none")] pub name: Option<String>, #[serde(skip_serializing_if="Option::is_none")] pub reasoning_content: Option<String> }

static API_CACHE: Lazy<RwLock<(String,String,String,Instant)>> = Lazy::new(|| RwLock::new((String::new(),String::new(),String::new(),Instant::now()-std::time::Duration::from_secs(60))));
static AGENT_HTTP_CLIENT: Lazy<reqwest::Client> = Lazy::new(|| reqwest::Client::builder().timeout(std::time::Duration::from_secs(API_TIMEOUT_SECS)).build().expect("agent http"));

fn get_api_endpoint() -> (String, String, String) {
    { let (url,key,model,at) = &*API_CACHE.read().unwrap(); if at.elapsed().as_secs()<30 && !url.is_empty() { return (url.clone(),key.clone(),model.clone()); } }
    let config = crate::services::chat_api::get_chat_config();
    let url = config.base_url.trim_end_matches('/').to_string();
    let (url, key, model) = (url, config.api_key, config.model);
    let mut c = API_CACHE.write().unwrap(); *c = (url.clone(), key.clone(), model.clone(), Instant::now());
    (url, key, model)
}

// ═══════════════════════════════════════════════════════════════════
// Session Memory — 会话工作记忆层
// ═══════════════════════════════════════════════════════════════════
const ACCESS_STALE_THRESHOLD: usize = 20;   // 超过 20 次访问未命中即淘汰
const SCRATCHPAD_MAX_CHARS: usize = 4000;     // scratchpad 最长字符数

#[derive(Debug, Clone)]
struct ActiveObject {
    obj_type: String,
    obj_id: String,
    name: String,
    access_order: usize,
    hits: usize,
    score: f64,
}

#[derive(Debug, Clone)]
struct Focus {
    entities: Vec<String>,
    intent: String,
    open_questions: Vec<String>,
}

#[derive(Debug, Clone)]
struct SessionMemory {
    active_objects: Vec<ActiveObject>,
    visited_chunks: std::collections::HashSet<String>,
    visited_sources: std::collections::HashSet<String>,
    shown_memory_ids: std::collections::HashSet<String>,  // MEMORY 跨轮次去重
    focus: Focus,
    observations: Vec<String>,
    hypotheses: Vec<String>,
    current_goal: String,
    access_counter: usize,
    search_keywords: Vec<Vec<String>>,
    tool_history: Vec<String>,  // 工具调用历史（用于生成检索计划）
}

impl SessionMemory {
    fn new() -> Self {
        Self {
            active_objects: Vec::new(),
            visited_chunks: std::collections::HashSet::new(),
            visited_sources: std::collections::HashSet::new(),
            shown_memory_ids: std::collections::HashSet::new(),
            focus: Focus { entities: Vec::new(), intent: String::new(), open_questions: Vec::new() },
            observations: Vec::new(),
            hypotheses: Vec::new(),
            current_goal: String::new(),
            access_counter: 0,
            search_keywords: Vec::new(),
            tool_history: Vec::new(),
        }
    }

    /// 记录一个对象被访问/命中，然后淘汰过期的
    fn update(&mut self, obj_type: &str, obj_id: &str, name: &str) {
        self.access_counter += 1;
        if let Some(existing) = self.active_objects.iter_mut().find(|o| o.obj_type == obj_type && o.obj_id == obj_id) {
            existing.name = name.to_string();
            existing.access_order = self.access_counter;
            existing.hits += 1;
        } else {
            self.active_objects.push(ActiveObject {
                obj_type: obj_type.to_string(),
                obj_id: obj_id.to_string(),
                name: name.to_string(),
                access_order: self.access_counter,
                hits: 1,
                score: 0.0,
            });
        }
        self.evict_stale();
    }

    /// 从包含 source_id 的行解析并记录
    fn update_from_line(&mut self, line: &str) {
        let tl = line.trim();
        // 检测 source_id 格式的行：[TYPE] Name | source_id: TYPE-id
        let has_source_id = tl.contains("| source_id: ");
        if has_source_id {
            if let Some(sid) = tl.split("| source_id: ").nth(1).map(|s| s.trim()) {
                let parts: Vec<&str> = sid.splitn(2, '-').collect();
                if parts.len() == 2 {
                    let obj_type = match parts[0] {
                        "FILE" => "file",
                        "PRODUCT" => "product",
                        "PROJECT" => "project",
                        "NOTE" => "note",
                        _ => parts[0],
                    };
                    let name = tl.split("| source_id: ").next()
                        .and_then(|s| s.splitn(2, ']').nth(1))
                        .map(|s| s.trim().trim_end_matches('.'))
                        .unwrap_or("");
                    self.update(obj_type, sid, name);
                }
            }
        } else {
            // 旧格式兼容（不含 source_id 的纯 [Object] 行）
            if let Some(name) = tl.strip_prefix("[Object] Product:").map(|s| s.trim().split('|').next().unwrap_or("").trim()) {
                self.update("product", name, name);
            }
            if let Some(name) = tl.strip_prefix("[Object] Project:").map(|s| s.trim().split('|').next().unwrap_or("").trim()) {
                self.update("project", name, name);
            }
            if let Some(name) = tl.strip_prefix("[Object] File:").map(|s| s.trim().split('|').next().unwrap_or("").trim()) {
                self.update("file", name, name);
            }
        }
        if let Some(cid) = tl.strip_prefix("ChunkID：").map(|s| s.trim()) {
            self.mark_chunk_visited(cid);
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
    fn record_observation(&mut self, text: &str) {
        let t = text.trim();
        if !t.is_empty() && t.chars().count() > 10 {
            self.observations.insert(0, t.chars().take(200).collect());
            self.observations.truncate(5);
        }
    }
    fn set_focus_from_user_query(&mut self, query: &str, top_entities: &[String]) {
        if query.is_empty() { return; }
        self.focus.entities = top_entities.to_vec();
        let is_followup = top_entities.iter().any(|e| query.contains(e.as_str()))
            || (!self.observations.is_empty() && query.chars().count() < 40);
        if self.current_goal.is_empty() || !is_followup {
            self.current_goal = query.to_string();
            self.hypotheses.clear();
        }
        if is_followup && !self.observations.is_empty() {
            self.focus.open_questions.push(query.to_string());
            self.focus.open_questions.truncate(3);
        } else {
            self.focus.open_questions.clear();
            self.focus.open_questions.push(query.to_string());
        }
    }

    /// 生成注入 Prompt 的工作记忆文本，受 Token 预算约束
    fn generate_scratchpad(&mut self) -> String {
        // ★ P1: 下一轮扩散 — 每轮开始时从 object_links 注入关联对象
        let mut sorted: Vec<&ActiveObject> = self.active_objects.iter().collect();
        sorted.sort_by(|a,b|b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        let top_oids: Vec<String> = sorted.iter().take(5).map(|o| o.obj_id.to_string()).collect();
        let neighbors = crate::services::synapse::expand_object_links(&top_oids, 15);
        if neighbors.len() > 0 { eprintln!("[LLM检索] expand(next_round): {} neighbors", neighbors.len()); }
        for (nid, _w) in neighbors.iter().take(5) {
            if let Some(dash) = nid.find("-") {
                let (ot, oi) = nid.split_at(dash);
                let oi = &oi[1..];
                let obj_type = ot.to_lowercase();
                if let Ok(Some(row)) = database::get("SELECT title FROM source_data WHERE source_type=?1 AND source_id=?2", &[serde_json::json!(ot), serde_json::json!(oi)]) {
                    let name = row["title"].as_str().unwrap_or("");
                    if !obj_type.is_empty() && !oi.is_empty() && !name.is_empty() {
                        self.update(&obj_type, oi, name);
                    }
                }
            }
        }
        let mut lines: Vec<String> = Vec::new();
        lines.push("## 当前会话工作记忆".to_string());

        // 0. Execution Log（从工具调用历史生成，同类调用自动合并，避免正向强化贪心遍历）
        if !self.tool_history.is_empty() {
            lines.push(String::new());
            lines.push("## Execution Log".to_string());
            // 合并连续相同类型的工具调用，加速日志摘要
            let mut groups: Vec<(String, usize)> = Vec::new();
            for step in self.tool_history.iter() {
                let group_key = step.split(&['(', ' '][..]).next().unwrap_or(step).to_string();
                if let Some(last) = groups.last_mut() {
                    if last.0 == group_key {
                        last.1 += 1;
                        continue;
                    }
                }
                groups.push((group_key, 1));
            }
            for (i, (action, count)) in groups.iter().enumerate() {
                let is_last = i == groups.len() - 1;
                let status = if is_last { "⏳" } else { "✅" };
                let count_label = if *count > 1 { format!(" (已完成 {} 次)", count) } else { String::new() };
                let label = if is_last { " (当前)" } else { "" };
                lines.push(format!("  {} {}{}{}", status, action, count_label, label));
            }
        }

        // 1. Current Goal
        if !self.current_goal.is_empty() {
            lines.push(String::new());
            lines.push("## Current Goal".to_string());
            lines.push(self.current_goal.clone());
        }

        // 2. Active Objects（带焦点标记 + Top3 标签）
        if !self.active_objects.is_empty() {
            lines.push(String::new());
            lines.push("## Active Objects".to_string());
            let mut sorted: Vec<&ActiveObject> = self.active_objects.iter().collect();
            sorted.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
            for (i, o) in sorted.iter().take(10).enumerate() {
                let label = match o.obj_type.as_str() {
                    "product" => format!("PRODUCT: {}", o.name),
                    "project" => format!("PROJECT: {}", o.name),
                    "file" => format!("FILE: {}", o.name),
                    "note" => format!("NOTE: {}", o.name),
                    t => format!("{}: {}", t, o.name),
                };
                if i == 0 {
                    lines.push(format!("  [{}] 🌟 当前焦点", label));
                } else {
                    lines.push(format!("  [{}] 💤", label));
                }
            }
        }

        // 3. Open Questions
        if !self.focus.open_questions.is_empty() {
            lines.push(String::new());
            lines.push("## Unknown".to_string());
            for q in self.focus.open_questions.iter() {
                lines.push(format!("  □ {}", q));
            }
        }

        // 4. Collected Evidence（按来源分组的 observations）
        if !self.observations.is_empty() {
            lines.push(String::new());
            lines.push("## Collected Evidence".to_string());
            for obs in self.observations.iter().take(5) {
                let preview: String = obs.chars().take(120).collect();
                lines.push(format!("  ✓ {}", preview));
            }
            if !self.visited_chunks.is_empty() || !self.visited_sources.is_empty() {
                let parts: Vec<String> = std::iter::once(())
                    .filter_map(|_| if !self.visited_chunks.is_empty() { Some(format!("浏览 {} 个段落", self.visited_chunks.len())) } else { None })
                    .chain(std::iter::once(()).filter_map(|_| if !self.visited_sources.is_empty() { Some(format!("来源 {} 个", self.visited_sources.len())) } else { None }))
                    .collect();
                if !parts.is_empty() {
                    lines.push(format!("  📎 {}", parts.join(" · ")));
                }
            }
        }

        // 5. Evidence Path（从 observations 推导探索轨迹）
        if !self.observations.is_empty() {
            lines.push(String::new());
            lines.push("## Evidence Path".to_string());
            for obs in self.observations.iter().take(3).rev() {
                let preview: String = obs.chars().take(80).collect();
                lines.push(format!("  → {}", preview));
            }
        }

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
    UpdateObject { obj_type: String, obj_id: String, name: String },
    UpdateObjectScore { obj_type: String, obj_id: String, score_delta: f64 },
    MarkChunkVisited(String),
}

impl SessionMemory {
    fn apply_effect(&mut self, effect: &SessionEffect) {
        match effect {
            SessionEffect::RecordSearch(kw) => self.record_search(kw),
            SessionEffect::MarkSourceVisited(s) => self.mark_source_visited(s),
            SessionEffect::UpdateObject { obj_type, obj_id, name } => self.update(obj_type, obj_id, name),
            SessionEffect::UpdateObjectScore { obj_type, obj_id, score_delta } => {
                if let Some(existing) = self.active_objects.iter_mut().find(|o| o.obj_type == *obj_type && o.obj_id == *obj_id) {
                    existing.score = existing.score * 0.5 + score_delta;
                }
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
        match t { "product" => Some("PRODUCT"), "project" => Some("PROJECT"),
            "file" => Some("FILE"), "note" => Some("NOTE"), "chat" => Some("CHAT"), _ => None }
    };
    // 1. top 5 高分对象
    let mut sorted: Vec<&ActiveObject> = session.active_objects.iter().collect();
    sorted.sort_by(|a, b| b.hits.cmp(&a.hits).then_with(|| b.access_order.cmp(&a.access_order)));
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
    if scope_ids.is_empty() { return None; }
    let s = resolve_scope_to_rowids(&scope_ids);
    if s.is_empty() { None } else { Some(s) }
}

fn build_tools_definitions() -> Value {
    json!([
        {"type":"function","function":{"name":"ext_tool_search","description":"在知识库中检索证据。支持时间范围过滤。","parameters":{"type":"object","properties":{"keywords":{"type":"array","items":{"type":"string"},"description":"1~4 个核心关键词"},"start_time":{"type":"string","description":"开始时间，格式 2026-06-25 或 昨天"},"end_time":{"type":"string","description":"结束时间，格式 2026-06-25 或 昨天"}},"required":["keywords"]}}},
        {"type":"function","function":{"name":"ext_tool_object","description":"搜索和浏览记忆中的对象。mode=search用于定位（FTS+排序+TopK），mode=recall用于回忆（按天分组+完整展示+关联计数）。recall模式超长时自动分页。","parameters":{"type":"object","properties":{"query":{"type":"string","description":"对象名称关键词（search模式用）"},"type":{"type":"string","description":"CHAT/NOTE/PRODUCT/PROJECT/FILE，留空全部类型"},"start_time":{"type":"string","description":"开始时间 如 昨天/2026-06-25"},"end_time":{"type":"string","description":"结束时间"},"mode":{"type":"string","enum":["search","recall"],"description":"search=检索定位，recall=时间线回忆，默认search"},"limit":{"type":"integer","description":"每页数量，默认50，最大100"},"offset":{"type":"integer","description":"分页偏移，默认0"}},"required":[]}}},
        {"type":"function","function":{"name":"ext_tool_traverse","description":"沿关联图谱漫游。⚠️ chunk_id 是8位短ID（如 47beae9a），来自 search() 结果中的「ChunkID」字段。绝对不是 source_id（如 FILE-xxx 或 PRODUCT-xxx）。","parameters":{"type":"object","properties":{"chunk_id":{"type":"string","description":"8位短ID（如 47beae9a），必须来自 search() 结果的 ChunkID 字段，不是 source_id"},"edge_type":{"type":"string","enum":["next","prev","related","references"],"description":"next=向后翻页；prev=向前翻页；related=同项目/同产品/同实体；references=引用实体"},"target_name":{"type":"string","description":"目标名称（related/references 时用）"},"radius":{"type":"integer","description":"步长，默认 10，最大 20"}},"required":["chunk_id","edge_type"]}}},
        {"type":"function","function":{"name":"ext_tool_manage_todo","description":"管理待办。action=create(title必填,priority从用户原话提取,due_date从用户原话提取,repeat可选daily/weekly/monthly,project_id关联项目), action=update, action=delete","parameters":{"type":"object","properties":{"action":{"type":"string","enum":["create","update","delete"]},"title":{"type":"string","description":"待办标题"},"content":{"type":"string","description":"待办详情"},"priority":{"type":"string","description":"优先级: P1/P2/P3 或 高/中/低"},"due_date":{"type":"string","description":"截止日期: YYYY-MM-DD"},"repeat":{"type":"string","description":"循环: daily/weekly/monthly"},"project_id":{"type":"string","description":"关联项目ID"},"done":{"type":"boolean","description":"是否完成(update用)"},"todo_id":{"type":"string","description":"待办ID(update/delete)"}},"required":["action"]}}},
        {"type":"function","function":{"name":"ext_tool_act","description":"执行项目操作。action: update_project_status=更新项目状态(project_id+new_status)","parameters":{"type":"object","properties":{"action":{"type":"string","enum":["update_project_status"]},"project_id":{"type":"string","description":"项目ID"},"new_status":{"type":"string","description":"新状态"}},"required":["action"]}}},
        {"type":"function","function":{"name":"ext_tool_manage_memory","description":"管理记忆。action=create(创建，title=知识点，content=原始陈述，replace_ids=旧记忆source_id逗号分隔，自动删除旧的)，action=delete(删除，用memory_id或title匹配)。整理流程：搜索→获取旧记忆source_id→create汇总+replace_ids→旧记忆自动清理。","parameters":{"type":"object","properties":{"action":{"type":"string","enum":["create","delete"]},"title":{"type":"string","description":"知识标题或记忆标题"},"content":{"type":"string","description":"记忆内容(create用)"},"memory_id":{"type":"string","description":"记忆source_id(delete精确匹配)"},"replace_ids":{"type":"string","description":"逗号分隔的旧记忆source_id(create时自动删除)"}},"required":["action"]}}},
        {"type":"function","function":{"name":"ext_tool_batch_search","description":"批量并发检索多个独立指标。仅当用户在单轮中询问≥3个独立指标时使用。每个指标返回2个最相关结果。简单问题和单指标请用 ext_tool_search。","parameters":{"type":"object","properties":{"queries":{"type":"array","items":{"type":"object","properties":{"keywords":{"type":"array","items":{"type":"string"},"description":"1~3 个核心关键词"}},"required":["keywords"]},"description":"3~10 个独立子查询"}},"required":["queries"]}}},
        {"type":"function","function":{"name":"ext_tool_manage_policy","description":"管理用户的规则/策略（宪法）。有id+content=更新规则，有id+无content=删除规则，无id+有content=新增规则。当用户表达「以后...」「记住...」「不要...」「总是...」「删掉...」等规则性指令时使用。","parameters":{"type":"object","properties":{"id":{"type":"string","description":"策略source_id（更新或删除时必填）"},"content":{"type":"string","description":"规则内容（新增或更新时必填）"}},"required":[]}}},
        {"type":"function","function":{"name":"ext_tool_record_learning","description":"记录认知经历。在回答过程中主动调用，记录你认为值得记住的知识点或发现。不消耗预算。","parameters":{"type":"object","properties":{"question":{"type":"string","description":"用户的问题或主题"},"provider":{"type":"string","description":"查阅的文档或来源"},"answer":{"type":"string","description":"结论要点"}},"required":["question","provider","answer"]}}},
        {"type":"function","function":{"name":"ext_tool_manage_project","description":"管理项目。action: create(创建项目), update(更新项目字段), delete(删除项目及关联数据), add_channel(添加渠道伙伴), add_vendor(添加供应商), add_contact(添加联系人), update_channel(更新渠道), update_vendor(更新供应商), delete_channel(删除渠道), delete_vendor(删除供应商), delete_contact(删除联系人)","parameters":{"type":"object","properties":{"action":{"type":"string","enum":["create","update","delete","add_channel","add_vendor","add_contact","update_channel","update_vendor","delete_channel","delete_vendor","delete_contact"]},"project_id":{"type":"string","description":"项目ID(update/delete/add_channel/add_vendor用)"},"name":{"type":"string","description":"项目名称(create)或渠道/供应商/联系人名称"},"customer":{"type":"string","description":"客户名称"},"customer_contact":{"type":"string","description":"客户联系人"},"customer_phone":{"type":"string","description":"客户联系电话"},"owner":{"type":"string","description":"项目负责人"},"budget":{"type":"number","description":"预算金额，单位：万元"},"description":{"type":"string","description":"项目描述"},"bid_time":{"type":"string","description":"投标时间 YYYY-MM-DD"},"status":{"type":"string","description":"项目状态: Lead/Won/Lost/Archived"},"channel_id":{"type":"string","description":"渠道ID(update_channel/delete_channel用)"},"vendor_id":{"type":"string","description":"供应商ID(update_vendor/delete_vendor用)"},"contact_id":{"type":"string","description":"联系人ID(delete_contact用)"},"parent_id":{"type":"string","description":"父级ID(add_contact时为channel_id或vendor_id)"},"parent_type":{"type":"string","description":"父级类型: channel/vendor(add_contact/delete_contact用)"},"phone":{"type":"string","description":"联系电话"},"company":{"type":"string","description":"公司名称"},"email":{"type":"string","description":"邮箱"},"remark":{"type":"string","description":"备注"}},"required":["action"]}}}
    ])
}


/// Fallback: parse XML tool calls from content when model does not use function calling format
fn parse_xml_tool_calls(content: &str) -> Option<Vec<Value>> {
    // 清理非标准前缀（如 DeepSeek 的 ｜｜DSML｜｜）
    let cleaned = content.replace("｜｜DSML｜｜", "").replace("‖DSML‖", "").replace("<<DSML>>", "");
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
async fn call_llm_with_tools(messages: &[ChatMessage], tools: &Value, _app_handle: &Option<tauri::AppHandle>) -> Result<Value, String> {
    let (url, api_key, model) = get_api_endpoint();
    let msgs: Vec<Value> = messages.iter().map(|m| serde_json::to_value(m).unwrap_or_default()).collect();
    let body = json!({"model":model,"messages":msgs,"tools":tools,"tool_choice":"auto","temperature":0.3});
    let mut req = AGENT_HTTP_CLIENT.post(&format!("{}/chat/completions", url)).json(&body);
    if !api_key.is_empty() { req = req.header("Authorization", format!("Bearer {}", api_key)); }
    let resp = req.send().await.map_err(|e| format!("LLM: {e}"))?;
    if !resp.status().is_success() { let s = resp.status(); let b = resp.text().await.unwrap_or_default(); return Err(format!("LLM {s}: {b}")); }
    resp.json().await.map_err(|e| format!("JSON: {e}"))
}

async fn call_llm_without_tools(messages: &[ChatMessage], app_handle: &Option<tauri::AppHandle>) -> Result<String, String> {
    let (url, api_key, model) = get_api_endpoint();
    let msgs: Vec<Value> = messages.iter().map(|m| serde_json::to_value(m).unwrap_or_default()).collect();
    let body = json!({"model":model,"messages":msgs,"temperature":0.3,"stream":true});
    let mut req = AGENT_HTTP_CLIENT.post(&format!("{}/chat/completions", url)).json(&body);
    if !api_key.is_empty() { req = req.header("Authorization", format!("Bearer {}", api_key)); }
    let resp = req.send().await.map_err(|e| format!("LLM: {e}"))?;
    if !resp.status().is_success() { let s = resp.status(); let b = resp.text().await.unwrap_or_default(); return Err(format!("LLM {s}: {b}")); }
    let byte_stream = resp.bytes_stream();
    let mut buffer = String::new();
    let mut full = String::new();
    let mut stream = byte_stream;
    while let Some(chunk) = stream.next().await {
        let bytes = chunk.map_err(|e| format!("stream: {e}"))?;
        buffer.push_str(&String::from_utf8_lossy(&bytes));
        while let Some(pos) = buffer.find('\n') {
            let line = buffer[..pos].to_string();
            buffer = buffer[pos+1..].to_string();
            if line.is_empty() || line == "data: [DONE]" { continue; }
            if !line.starts_with("data: ") { continue; }
            if let Ok(chunk) = serde_json::from_str::<Value>(&line[6..]) {
                if let Some(choices) = chunk["choices"].as_array() {
                    for c in choices {
                        if let Some(tok) = c["delta"]["content"].as_str() {
                            full.push_str(tok);
                            if let Some(ref h) = app_handle { let _ = h.emit("agent:token", json!({"token": tok})); }
                        }
                    }
                }
            }
        }
    }
    Ok(full)
}

// ═══════════════════════════════════════════════════════════════════
// Chunk metadata enrichment
// ═══════════════════════════════════════════════════════════════════

/// 从 source_type / source_id 查询所有关联元数据，返回格式化信息块
/// 数据来源：business_relations（关联关系）+ source_data.title（名称）
pub fn build_chunk_metadata(source_type: &str, source_id: &str) -> String {
    if source_type.is_empty() || source_id.is_empty() { return String::new(); }

    // ── 1. 来源标题（从 source_data.title）──
    let title = database::get(
        "SELECT title FROM source_data WHERE source_type=?1 AND source_id=?2",
        &[json!(source_type), json!(source_id)],
    ).ok().flatten().and_then(|r| r["title"].as_str().map(String::from)).unwrap_or_default();

    let mut lines: Vec<String> = Vec::new();
    if !title.is_empty() {
        let type_label = match source_type { "FILE" => "文件", "PRODUCT" => "产品", "PROJECT" => "项目", "NOTE" => "笔记", "TODO" => "待办", _ => source_type };
        lines.push(format!("来源：{} · {}", type_label, title));
    }

    // ── 2. 从 business_relations 查所有关联 ──
    let rels = database::all(
        "SELECT target_type, target_id, relation_type, metadata FROM business_relations WHERE source_type=?1 AND source_id=?2",
        &[json!(source_type), json!(source_id)],
    ).unwrap_or_default();
    let reverse_rels = database::all(
        "SELECT source_type as target_type, source_id as target_id, relation_type, metadata FROM business_relations WHERE target_type=?1 AND target_id=?2",
        &[json!(source_type), json!(source_id)],
    ).unwrap_or_default();

    // 合并双向关联，去重
    let mut all_rels: Vec<(String, String, String, String)> = rels.into_iter()
        .chain(reverse_rels.into_iter())
        .filter_map(|r| {
            Some((r["target_type"].as_str()?.to_string(), r["target_id"].as_str()?.to_string(), r["relation_type"].as_str().unwrap_or("").to_string(), r["metadata"].as_str().unwrap_or("{}").to_string()))
        }).collect();
    all_rels.sort_by(|a,b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    all_rels.dedup_by(|a,b| a.0 == b.0 && a.1 == b.1);

    if all_rels.is_empty() {
        // 无关联时输出占位行
        let is_self = |t: &str| t.to_uppercase() == source_type;
        for label in &["项目", "产品", "笔记", "待办", "文件"] {
            let type_key = match *label { "项目" => "PROJECT", "产品" => "PRODUCT", "笔记" => "NOTE", "待办" => "TODO", "文件" => "FILE", _ => "" };
            if !is_self(type_key) { lines.push(format!("{}：无关联{}", label, label)); }
        }
        if lines.is_empty() { return String::new(); }
        return lines.join("\n");
    }

    // ── 3. 批量查名称 ──
    let mut name_cache: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut targets_by_type: std::collections::HashMap<String, Vec<(String, String)>> = std::collections::HashMap::new();
    for (tt, tid, _rt, _meta) in &all_rels {
        targets_by_type.entry(tt.clone()).or_default().push((tid.clone(), _meta.clone()));
    }
    for (tt, targets) in &targets_by_type {
        let ids: Vec<String> = targets.iter().map(|(id,_)| id.clone()).collect();
        if ids.is_empty() { continue; }
        let ph: Vec<String> = ids.iter().map(|_| "?".to_string()).collect();
        let params: Vec<serde_json::Value> = ids.iter().map(|id| json!(id)).collect();
        let type_upper = tt.to_uppercase();
        // FILE 类型：从 source_data.title 取名称（已存原始文件名）
        let rows = database::all(
            &format!("SELECT source_id, title FROM source_data WHERE source_type=?1 AND source_id IN ({})", ph.join(",")),
            &[json!(&type_upper)].iter().chain(params.iter()).cloned().collect::<Vec<_>>().as_slice(),
        ).unwrap_or_default();
        for r in &rows {
            if let (Some(id), Some(t)) = (r["source_id"].as_str(), r["title"].as_str()) {
                if !t.is_empty() { name_cache.insert(format!("{}-{}", type_upper, id), t.to_string()); }
            }
        }
    }

    // ── 4. 按类型格式化 ──
    let format_targets = |tt: &str, targets: &[(String, String)]| -> Vec<String> {
        targets.iter().filter_map(|(tid, meta)| {
            let name = name_cache.get(&format!("{}-{}", tt, tid)).map(|s| s.as_str()).unwrap_or("");
            if name.is_empty() { return None; }
            let meta_val: Value = serde_json::from_str(meta).unwrap_or(Value::Null);
            let custom = meta_val.get("custom_name").and_then(|v| v.as_str()).filter(|s| !s.is_empty());
            let mut extra = String::new();
            if let Some(cn) = custom { extra.push_str(&format!(" — 作为「{}」", cn)); }
            Some(format!("· {}{}", name, extra))
        }).collect()
    };

    for (label, type_key) in &[("项目", "PROJECT"), ("产品", "PRODUCT"), ("笔记", "NOTE"), ("待办", "TODO"), ("文件", "FILE")] {
        if type_key == &source_type { continue; } // 跳过自身类型
        if let Some(targets) = targets_by_type.get(*type_key) {
            let items = format_targets(type_key, targets);
            if !items.is_empty() { lines.push(format!("{}：\n{}", label, items.join("\n"))); }
            else { lines.push(format!("{}：无关联{}", label, label)); }
        } else {
            lines.push(format!("{}：无关联{}", label, label));
        }
    }

    if lines.len() <= 1 { return String::new(); } // 只有标题没有关联
    lines.join("\n")
}

// ═══════════════════════════════════════════════════════════════════
// Tag-based pre-filtering
// ═══════════════════════════════════════════════════════════════════

/// 从 tag_mappings 查出匹配标签的实体 → source_data → 父块 → 子块 rowid 列表
pub fn get_child_rowids_by_tag_names(tag_names: &[String]) -> Vec<i64> {
    if tag_names.is_empty() { return vec![]; }

    // 1. tag_mappings → (entity_type, entity_id)
    let placeholders = tag_names.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "SELECT DISTINCT tm.entity_type, tm.entity_id FROM tag_mappings tm \
         JOIN tags t ON t.tag_id = tm.tag_id \
         WHERE t.name IN ({})", placeholders
    );
    let params: Vec<serde_json::Value> = tag_names.iter().map(|n| json!(n)).collect();
    let rows = database::all(&sql, &params).unwrap_or_default();
    if rows.is_empty() { return vec![]; }

    // 2. 将 entity_type 映射为 source_type（小写复数 → 大写单数）
    let mut source_pairs: Vec<(String, String)> = Vec::new();
    for row in &rows {
        let et = row["entity_type"].as_str().unwrap_or("");
        let eid = row["entity_id"].as_str().unwrap_or("");
        if et.is_empty() || eid.is_empty() { continue; }
        let st = match et {
            "notes" => "NOTE", "files" => "FILE",
            "products" => "PRODUCT", "projects" => "PROJECT",
            _ => continue,
        };
        source_pairs.push((st.to_string(), eid.to_string()));
    }
    if source_pairs.is_empty() { return vec![]; }

    // 3. source_data.id → 子块 rowid
    let mut ids = Vec::new();
    for (st, sid) in &source_pairs {
        if let Ok(Some(row)) = database::get(
            "SELECT id FROM source_data WHERE source_type=?1 AND source_id=?2",
            &[json!(st), json!(sid)],
        ) {
            if let Some(sdid) = row["id"].as_i64() {
                ids.push(sdid);
            }
        }
    }
    if ids.is_empty() { return vec![]; }

    let ph = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    database::all(
        &format!("SELECT id FROM knowledge_chunks WHERE chunk_type='child' AND parent_id IN (SELECT chunk_id FROM knowledge_chunks WHERE chunk_type='parent' AND source_data_id IN ({}))", ph),
        &ids.iter().map(|id| json!(id)).collect::<Vec<_>>(),
    ).unwrap_or_default().iter().filter_map(|r| r["id"].as_i64()).collect()
}

/// 关键词 + 标签 → 横向检索：从 tags 表硬查询匹配对象，返回对象卡片。
/// 输出格式与 Object 工具一致（[TYPE] Name | source_id: TYPE-id），自动反哺 Workspace。
/// 不依赖语义匹配，纯数据库外键级联。
fn execute_horizontal_search(keywords: &[String]) -> String {
    if keywords.is_empty() { return String::new(); }
    let like = format!("%{}%", keywords.join(" "));

    let mut out = String::new();
    let mut seen = std::collections::HashSet::new();

    // 1. 标签命中：tags.name LIKE → tag_mappings → source_data
    if let Ok(rows) = database::all(
        "SELECT DISTINCT tm.entity_type, tm.entity_id, t.name as tag_name FROM tag_mappings tm \
         JOIN tags t ON t.tag_id = tm.tag_id WHERE t.name LIKE ?1 LIMIT 15",
        &[json!(&like)],
    ) {
        for r in &rows {
            let et = r["entity_type"].as_str().unwrap_or("");
            let eid = r["entity_id"].as_str().unwrap_or("");
            let tag_name = r["tag_name"].as_str().unwrap_or("");
            if et.is_empty() || eid.is_empty() { continue; }
            let st = match et {
                "notes" => "NOTE", "files" => "FILE",
                "products" => "PRODUCT", "projects" => "PROJECT",
                "todos" => "TODO", _ => continue,
            };
            let key = format!("{}-{}", st, eid);
            if !seen.insert(key) { continue; }
            if let Ok(Some(sd)) = database::get("SELECT title FROM source_data WHERE source_type=?1 AND source_id=?2", &[json!(st), json!(eid)]) {
                let n = sd["title"].as_str().unwrap_or("");
                out.push_str(&format!("  [{}] {} | source_id: {}-{}\n", st, n, st, eid));
                out.push_str(&format!("    ↳ 关联原因: 标签「{}」匹配\n", tag_name));
            }
        }
    }

    if out.is_empty() { return String::new(); }
    format!("\n💡 【横向关联对象】\n{}\n", out.trim())
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
        if parts.len() != 2 { continue; }
        let st = parts[0].to_uppercase();
        let sid = parts[1];
        if let Ok(Some(row)) = database::get("SELECT id FROM source_data WHERE source_type=?1 AND source_id=?2",&[json!(st), json!(sid)]) {
            if let Some(sdid) = row["id"].as_i64() {
                if let Ok(rows) = database::all("SELECT id FROM knowledge_chunks WHERE chunk_type='child' AND source_data_id=?1",&[json!(sdid)]) {
                    for cr in &rows { if let Some(cid) = cr["id"].as_i64() { all_rowids.insert(cid); } }
                }
            }
        }
    }
    all_rowids
}

/// 执行 search 并同时返回发现的来源列表
async fn execute_search_knowledge(keywords: &[String], scope_rowids: Option<std::collections::HashSet<i64>>, visited_sources: &std::collections::HashSet<String>, visited_chunks: &std::collections::HashSet<String>, time_range: Option<(String, String)>, shown_memory_ids: &std::collections::HashSet<String>) -> (String, Vec<String>) {
    let query = keywords.join(" "); if query.trim().is_empty() { return ("no keywords".into(), vec![]); }
    let tag_rowids: std::collections::HashSet<i64> = std::collections::HashSet::new();
    let mut obj_head = String::new();
    let mut matched_sources: std::collections::HashSet<(String,String)> = std::collections::HashSet::new();
    if !keywords.is_empty() {
        let like = format!("%{}%", keywords.join(" "));
        if let Ok(rows) = database::all("SELECT id,name,status,customer FROM projects WHERE name LIKE ?1 ORDER BY LENGTH(name) ASC LIMIT 3",&[json!(&like)]) {
            for r in &rows { let id = r["id"].as_str().unwrap_or("").to_string(); matched_sources.insert(("PROJECT".into(), id.clone())); obj_head.push_str(&format!("[Object] Project: {} | Status: {} | Customer: {}\n",r["name"].as_str().unwrap_or(""),r["status"].as_str().unwrap_or(""),r["customer"].as_str().unwrap_or(""))); }
        }
        if let Ok(rows) = database::all("SELECT id,name FROM products WHERE name LIKE ?1 ORDER BY LENGTH(name) ASC LIMIT 3",&[json!(&like)]) {
            for r in &rows { let id = r["id"].as_str().unwrap_or("").to_string(); matched_sources.insert(("PRODUCT".into(), id.clone())); obj_head.push_str(&format!("[Object] Product: {}\n",r["name"].as_str().unwrap_or(""))); }
        }
        if let Ok(rows) = database::all("SELECT fr.custom_name,fa.id,fa.file_ext FROM file_relations fr JOIN file_assets fa ON fa.id=fr.file_id WHERE fr.custom_name LIKE ?1 ORDER BY LENGTH(fr.custom_name) ASC LIMIT 3",&[json!(&like)]) {
            for r in &rows { let id = r["id"].as_str().unwrap_or("").to_string(); matched_sources.insert(("FILE".into(), id.clone())); obj_head.push_str(&format!("[Object] File: {}{}\n",r["custom_name"].as_str().unwrap_or(""),r["file_ext"].as_str().unwrap_or(""))); }
        }
        if let Ok(rows) = database::all("SELECT title FROM notes WHERE title LIKE ?1 ORDER BY LENGTH(title) ASC LIMIT 3",&[json!(&like)]) {
            for r in &rows { obj_head.push_str(&format!("[Object] Note: {}\n",r["title"].as_str().unwrap_or(""))); }
        }
    }
    if !obj_head.is_empty() { obj_head = format!("--- Related Objects ---\n{}--- Chunks ---\n", obj_head); }

    // ★ 始终走双路（FTS5 + Vector），不再因 scope 而跳过向量检索
    // ★ 始终走双路（FTS5 + Vector），不再因 scope 而跳过向量检索

    let vec_rowids: Vec<(String,f64)> = {
        match crate::services::embedding_client::get_embedding_with_caller(&query, "agent_workflow").await {
            Ok(qv) => {
                match crate::services::lancedb_client::get_lancedb() {
                    Ok(g) => match g.as_ref() {
                        Some(c) => {
                            let raw = c.search_vectors_with_parent(&qv, if scope_rowids.is_some() { 120 } else { 30 }).unwrap_or_default();
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
    let fts_limit = if scope_rowids.is_some() { 120 } else { 30 };
    let fts_q = keywords.iter().map(|k| format!("\"{}\"", k.replace('"',""))).collect::<Vec<_>>().join(" OR ");
    let fts_rowids: Vec<(String,f64)> = database::fts5_search_chunks(&fts_q, fts_limit).unwrap_or_default().iter()
        .map(|fr| (fr.rowid.to_string(), fr.rank)).collect();

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
    // ★ 双路结果：非MEMORY top 3 + MEMORY top 2（跨轮次去重）
    let non_memory: Vec<_> = candidates.iter().filter(|(_, _, st, _, _, _, _, _, _, _, _)| st != "MEMORY").take(3).collect();
    let memory: Vec<_> = candidates.iter().filter(|(_, _, st, sid, _, _, _, _, _, _, _)| {
        st == "MEMORY" && !shown_memory_ids.contains(&format!("MEMORY-{}", sid))
    }).take(2).collect();
    let selected: Vec<_> = non_memory.into_iter().chain(memory.into_iter()).collect();
    let mut out = String::new();
    for (idx, (score, cr, st, sid, chunk_id, content, _h, chunk_index, total_parents, is_scope_path, is_obj_match)) in selected.iter().enumerate() {
        let rrf_tag = &format!("{:.3}", score);
        let boost_tag = if *is_obj_match { "🔥" } else { "" };
        let mut meta = build_chunk_metadata(st, sid);
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
        // ★ 添加 source_id 行，让 update_from_line 可以将其加入工作区
        let source_label = match st.as_str() {
            "FILE" => "FILE",
            "PRODUCT" => "PRODUCT",
            "PROJECT" => "PROJECT",
            "NOTE" => "NOTE",
            "TODO" => "TODO",
            _ => st,
        };
        if let Ok(Some(title_row)) = database::get("SELECT title FROM source_data WHERE source_type=?1 AND source_id=?2",&[json!(source_label), json!(sid)]) {
            let name = title_row["title"].as_str().filter(|t| !t.is_empty()).map(|s| s.to_string()).or_else(|| {
                // FILE: custom_name 可能存储在 title 之外，尝试从 file_relations 获取
                if source_label == "FILE" {
                    database::get("SELECT custom_name FROM file_relations WHERE file_id=?1 AND custom_name != '' LIMIT 1",&[json!(sid)])
                        .ok().flatten().and_then(|r| r["custom_name"].as_str().map(|s| s.to_string()))
                } else { None }
            });
            if let Some(name) = name {
                meta.push_str(&format!("\n[{}] {} | source_id: {}-{}", source_label, name, source_label, sid));
            }
        }
        // ★ 时间标注
        let created_at: String = database::get(
            "SELECT created_at FROM knowledge_chunks WHERE id=?1", &[json!(cr)],
        ).ok().flatten().and_then(|r| r["created_at"].as_str().map(String::from)).unwrap_or_default();
        let time_info = relative_time(&created_at);
        if !time_info.is_empty() {
            if !meta.is_empty() { meta.push('\n'); }
            meta.push_str(&time_info);
        }
        let preview: String = content.chars().take(1300).collect();
        if meta.is_empty() { out.push_str(&format!("\n\n--- [父块{}|RRF={}]{} ---\n{}", idx + 1, rrf_tag, boost_tag, preview)); }
        else { out.push_str(&format!("\n\n--- [父块{}|RRF={}]{} ---\n{}\n--------\n{}", idx + 1, rrf_tag, boost_tag, meta, preview)); }

        // ★ MEMORY 类型：自动附带引用来源的摘要，并将来源加入 found_sources（进入注意力范围）
        if st == "MEMORY" {
            if let Some(refs_start) = content.find("[查阅]") {
                if let Some(refs_end) = content[refs_start..].find("\n") {
                    let refs_line = &content[refs_start+4..refs_start+refs_end];
                    let ref_ids: Vec<&str> = refs_line.split(|c| c == '、' || c == ',').map(|s| s.trim()).filter(|s| !s.is_empty() && *s != "无特定来源").collect();
                    if !ref_ids.is_empty() {
                        out.push_str("\n📎 引用来源：");
                        for ref_id in &ref_ids {
                            if let Some(dash_pos) = ref_id.find('-') {
                                let ref_type = &ref_id[..dash_pos];
                                let ref_sid = &ref_id[dash_pos+1..];
                                // 将引用来源加入 found_sources，使其进入注意力范围
                                found_sources.push(ref_id.to_string());
                                if let Ok(Some(ref_row)) = database::get(
                                    "SELECT title, substr(content,1,100) as preview FROM source_data WHERE source_type=?1 AND source_id=?2",
                                    &[json!(ref_type), json!(ref_sid)],
                                ) {
                                    let ref_title = ref_row["title"].as_str().unwrap_or("");
                                    let ref_preview = ref_row["preview"].as_str().unwrap_or("");
                                    out.push_str(&format!("\n  [{}] {} — {}", ref_type, ref_title, ref_preview));
                                }
                            }
                        }
                    }
                }
            }
        }

        found_sources.push(format!("{}-{}", st, sid));

        // ★ 关联对象自动进入 workspace
        match st.as_str() {
            "FILE" => {
                for rel_type in &["project", "product", "todo", "note"] {
                    if let Ok(rows) = database::all(
                        "SELECT entity_id FROM file_relations WHERE file_id=?1 AND entity_type=?2 AND entity_id!=''",
                        &[json!(sid.as_str()), json!(rel_type)],
                    ) {
                        for r in &rows {
                            if let Some(eid) = r["entity_id"].as_str().filter(|s| !s.is_empty()) {
                                let dedup = format!("{}-{}", rel_type.to_uppercase(), eid);
                                if !found_sources.iter().any(|s| s == &dedup) { found_sources.push(dedup); }
                            }
                        }
                    }
                }
            }
            "PROJECT" | "PRODUCT" | "NOTE" | "TODO" => {
                if let Ok(rows) = database::all(
                    "SELECT DISTINCT file_id FROM file_relations WHERE entity_id=?1 AND file_id IS NOT NULL AND file_id!=''",
                    &[json!(sid.as_str())],
                ) {
                    for r in &rows {
                        if let Some(fid) = r["file_id"].as_str().filter(|s| !s.is_empty()) {
                            let dedup = format!("FILE-{}", fid);
                            if !found_sources.iter().any(|s| s == &dedup) { found_sources.push(dedup); }
                        }
                    }
                }
            }
            _ => {}
        }

        // ★ 父块被采纳 → 激活其下所有子块
        if !chunk_id.is_empty() {
            if let Ok(child_rows) = database::all("SELECT id FROM knowledge_chunks WHERE chunk_type='child' AND parent_id=?1",&[json!(chunk_id)]) {
                for cr in &child_rows { if let Some(cid) = cr["id"].as_i64() { crate::services::synapse::activate_child_chunk(cid); } }
            }
        }
    }
    let horizontal = execute_horizontal_search(keywords);
    if out.trim().is_empty() {
        if obj_head.is_empty() {
            if !horizontal.trim().is_empty() { (horizontal.trim().to_string(), found_sources) } else {
                let kw_display = keywords.join("、");
                (format!("未找到关键词「{}」的任何匹配内容。\n\n💡 建议：\n1. 使用更简短或不同的关键词（如「混闪」代替「混合闪存」）\n2. 使用 object 先发现相关对象\n3. 确认知识库中已导入相关文件", kw_display), found_sources)
            }
        } else {
            (format!("{}{}", obj_head.trim(), horizontal).trim().to_string(), found_sources)
        }
    } else {
        // ★ Coverage-aware: 检测已命中 source 的未访问热点
        let mut hotspot_hint = String::new();
        if !found_sources.is_empty() {
            let mut hotspots: Vec<(String, i64)> = Vec::new();
            let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
            for src in &found_sources {
                if !seen.insert(src.clone()) { continue; }
                let parts: Vec<&str> = src.splitn(2, '-').collect();
                if parts.len() != 2 { continue; }
                let (st, sid) = (parts[0], parts[1]);
                if let Ok(Some(row)) = database::get(
                    "SELECT COUNT(*) as cnt FROM knowledge_chunks kc JOIN source_data sd ON sd.id=kc.source_data_id WHERE sd.source_type=?1 AND sd.source_id=?2 AND kc.chunk_type='parent'",
                    &[json!(st), json!(sid)]
                ) {
                    let total = row["cnt"].as_i64().unwrap_or(0);
                    if total > 3 {
                        // 估算已访问数（从 visited_chunks 中匹配该 source 的）
                        let visited_count = visited_sources.iter().filter(|v| v.starts_with(&format!("{}-{}", st, sid))).count() as i64;
                        let remaining = total - visited_count;
                        if remaining > 3 {
                            let label = format!("{}-{}", st, sid);
                            hotspots.push((label, remaining));
                        }
                    }
                }
            }
            if !hotspots.is_empty() {
                hotspots.sort_by(|a, b| b.1.cmp(&a.1));
                let hint: Vec<String> = hotspots.iter().take(4).map(|(name, cnt)| format!("{}({}块)", name, cnt)).collect();
                hotspot_hint = format!("\n\n🔍 未访问热点: {}", hint.join(" | "));
            }
        }
        (format!("{}{}{}{}", obj_head, out.trim(), horizontal, hotspot_hint).trim().to_string(), found_sources)
    }
}


fn execute_traverse(chunk_id: &str, edge_type: &str, radius: usize, visited: &std::collections::HashSet<String>, target_name: &str, traverse_source_counter: &mut std::collections::HashMap<String, usize>) -> String {
    if chunk_id.trim().is_empty() { return "请提供 ChunkID。".into(); }
    let cid = chunk_id.trim();
    let row = match database::get("SELECT p.chunk_id, p.content, p.chunk_index, p.source_data_id, sd.source_type, sd.source_id FROM knowledge_chunks p LEFT JOIN source_data sd ON sd.id=p.source_data_id WHERE p.chunk_type='parent' AND p.chunk_id LIKE ?1",&[json!(format!("{}%", cid))]) { Ok(Some(r)) => r, _ => return format!("未找到 ChunkID「{}」对应的块。", cid) };
    let current_index = row["chunk_index"].as_i64().unwrap_or(0);
    let sdid = match row["source_data_id"].as_str() { Some(id) => id.to_string(), None => return "该块无关联源数据。".into() };
    let total: i64 = database::get("SELECT COUNT(*) as cnt FROM knowledge_chunks WHERE source_data_id=?1 AND chunk_type='parent'",&[json!(sdid)]).ok().flatten().and_then(|r| r["cnt"].as_i64()).unwrap_or(0);
    let r = radius.max(1).min(20);
    let _row_st = row["source_type"].as_str().unwrap_or("").to_string();
    let row_sid = row["source_id"].as_str().unwrap_or("").to_string();
    // 将认知边缘映射到物理边缘
    let edge = match edge_type {
        "next" | "Next" => "Next",
        "prev" | "Prev" => "Prev",
        "related" | "Related" | "SameProduct" | "SameProject" => "Related",
        "references" | "References" | "SameEntity" => "References",
        _ => edge_type,
    };
    match edge {
        "Next" | "Prev" => {
            let remaining = total - (current_index + 1);
            let coverage_pct = ((current_index + 1) as f64 / total as f64 * 100.0) as i64;
            let est_steps = if remaining > 0 { (remaining + r as i64 - 1) / r as i64 } else { 0 };
            let mut out = format!("当前位置：{}/{} | 覆盖率 {}% | 半径={}，剩余 {} 块约需 {} 步", current_index + 1, total, coverage_pct, r, remaining, est_steps);
            let get_chunk = |offset: i64| -> Option<(String, String, i64)> {
                let target = current_index + offset;
                if target < 0 || target >= total { return None; }
                let r2 = database::get("SELECT chunk_id, content, chunk_index FROM knowledge_chunks WHERE source_data_id=?1 AND chunk_type='parent' AND chunk_index=?2",&[json!(sdid),json!(target)]).ok().flatten()?;
                let cid_out = r2["chunk_id"].as_str().unwrap_or("").to_string();
                let short = cid_out.chars().take(8).collect::<String>();
                let content = r2["content"].as_str().unwrap_or(""); let preview = content.chars().take(600).collect::<String>();
                Some((short, preview, target))
            };
            let mut found_any = false;
            let mut visited_indices: Vec<i64> = Vec::new();
            // ★ 遍历 radius 个步长，跳过已访问的 chunk，直到集齐 radius 个新块或到边界
            if edge == "Prev" {
                let mut got = 0usize;
                for step in 1..=r.max(r*2) {
                    if got >= r as usize { break; }
                    if let Some((short, preview, target)) = get_chunk(-(step as i64)) {
                        if visited.contains(&short) { visited_indices.push(target); continue; }
                        got += 1; found_any = true;
                        out.push_str(&format!("\n\n⬆ 前{}块：\n[{}] {}/{}\n{}\nChunkID：{}", step, short, target+1, total, preview, short));
                    }
                }
            }
            if edge == "Next" {
                let mut got = 0usize;
                for step in 1..=r.max(r*2) {
                    if got >= r as usize { break; }
                    if let Some((short, preview, target)) = get_chunk(step as i64) {
                        if visited.contains(&short) { visited_indices.push(target); continue; }
                        got += 1; found_any = true;
                        out.push_str(&format!("\n\n⬇ 后{}块：\n[{}] {}/{}\n{}\nChunkID：{}", step, short, target+1, total, preview, short));
                    } else { break; }
                }
            }
            // ★ 标记已跳过哪些块
            if !visited_indices.is_empty() {
                let skipped: Vec<String> = visited_indices.iter().map(|i| format!("{}", i+1)).collect();
                out.push_str(&format!("\n⏭ 跳过 {}/{}（已在前面展示过）", skipped.join(", "), total));
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
                    out.push_str(&format!("\n\n⚠️ 已在该文件中连续遍历 {} 次（剩余约 {} 块，预计还需约 {} 步）。建议使用 search 工具定位目标关键词，而非继续翻页。", consecutive, remaining, est_steps));
                }
            }
            // 边界阻断信号：无后续/前置块时明确提示 LLM
            if !found_any {
                out.push_str(match edge {
                    "Next" => "\n\n⚠️ 系统提示：已到达当前文档的物理【最末尾】，下方已无后续内容。请立即停止向后漫游，总结已有信息或切换到其他文档。",
                    "Prev" => "\n\n⚠️ 系统提示：已到达当前文档的物理【最开头】，上方已无前文内容。请立即停止向前漫游。",
                    _ => "",
                });
            }
            let st = row["source_type"].as_str().unwrap_or(""); let sid = row["source_id"].as_str().unwrap_or(""); let _full_cid = row["chunk_id"].as_str().unwrap_or("");
            let meta = build_chunk_metadata(&st.to_lowercase(),sid);
            if !meta.is_empty() { out.push_str(&format!("\n\n当前块导航卡：\n{}",meta)); }
            out
        }
        "Related" => {
            let mut out = String::new();
            for &(ety, name_cn) in &[("product","产品"), ("project","项目")] {
                let ids: Vec<String> = if target_name.trim().is_empty() {
                    database::all("SELECT fr.entity_id FROM file_relations fr JOIN file_assets fa ON fa.id=fr.file_id WHERE fr.entity_type=?1 AND fa.id=?2 LIMIT 3",&[json!(ety),json!(&row_sid)]).unwrap_or_default().iter().filter_map(|r2|r2["entity_id"].as_str().map(String::from)).collect()
                } else {
                    let tbl = if ety == "product" { "products" } else { "projects" };
                    database::all(&format!("SELECT id FROM {} WHERE id IN (SELECT fr.entity_id FROM file_relations fr JOIN file_assets fa ON fa.id=fr.file_id WHERE fr.entity_type=?1 AND fa.id=?2) AND name LIKE ?3 LIMIT 3",tbl),&[json!(ety),json!(&row_sid),json!(format!("%{}%",target_name.trim()))]).unwrap_or_default().iter().filter_map(|r2|r2["id"].as_str().map(String::from)).collect()
                };
                if ids.is_empty() { continue; }
                if !out.is_empty() { out.push_str("\n"); }
                out.push_str(&format!("同{}关联块：\n",name_cn));
                let child_rowids = get_child_rowids_by_tag_names(&ids);
                if child_rowids.is_empty() { continue; }
                let pi = child_rowids.len(); let ph = child_rowids.iter().map(|_|"?").collect::<Vec<_>>().join(",");
                let sql = format!("SELECT DISTINCT p.content,p.chunk_id,p.chunk_index FROM knowledge_chunks c JOIN knowledge_chunks p ON p.chunk_id=c.parent_id WHERE c.id IN ({}) AND p.content IS NOT NULL AND NOT (p.chunk_id LIKE ?{}) ORDER BY p.chunk_index LIMIT 5",ph,pi+1);
                let mut params: Vec<Value> = child_rowids.iter().map(|id|json!(id)).collect(); params.push(json!(format!("{}%",row["chunk_id"].as_str().unwrap_or(""))));
                if let Ok(rows) = database::all(&sql,&params) { for r2 in &rows { let pc=r2["chunk_id"].as_str().unwrap_or(""); if visited.contains(pc){continue}; let content=r2["content"].as_str().unwrap_or(""); let preview=content.chars().take(200).collect::<String>(); let short=pc.chars().take(8).collect::<String>(); out.push_str(&format!("\n· {}\nChunkID：{}\n",preview.replace('\n'," "),short)); } }
            }
            if out.is_empty() { if target_name.trim().is_empty() { return "未找到关联知识块。".into(); } else { return format!("未找到「{}」的关联知识块。",target_name.trim()); } }
            out
        }
        "References" => {
            let tn = target_name.trim();
            if tn.is_empty() { return "查找引用需要指定 target_name。".into(); }
            let tags: Vec<String> = database::all("SELECT DISTINCT t.name FROM tags t WHERE t.name LIKE ?1 LIMIT 5",&[json!(format!("%{}%",tn))]).unwrap_or_default().iter().filter_map(|r|r["name"].as_str().map(String::from)).collect();
            if tags.is_empty() { return format!("未找到标签「{}」。",tn); }
            let mut out = format!("引用标签「{}」的其他文档：\n",tags.join("、"));
            let child_rowids = get_child_rowids_by_tag_names(&tags);
            if child_rowids.is_empty() { out.push_str("未找到引用。"); return out; }
            let pi = child_rowids.len(); let ph = child_rowids.iter().map(|_|"?").collect::<Vec<_>>().join(",");
            let sql = format!("SELECT DISTINCT p.content,p.chunk_id,p.chunk_index FROM knowledge_chunks c JOIN knowledge_chunks p ON p.chunk_id=c.parent_id WHERE c.id IN ({}) AND p.content IS NOT NULL AND NOT (p.chunk_id LIKE ?{}) ORDER BY p.chunk_index LIMIT 5",ph,pi+1);
            let mut params: Vec<Value> = child_rowids.iter().map(|id|json!(id)).collect(); params.push(json!(format!("{}%",row["chunk_id"].as_str().unwrap_or(""))));
            if let Ok(rows) = database::all(&sql,&params) { for r2 in &rows { let pc=r2["chunk_id"].as_str().unwrap_or(""); if visited.contains(pc){continue}; let content=r2["content"].as_str().unwrap_or(""); let preview=content.chars().take(200).collect::<String>(); let short=pc.chars().take(8).collect::<String>(); out.push_str(&format!("\n· {}\nChunkID：{}\n",preview.replace('\n'," "),short)); } }
            out
        }
        _ => format!("不支持的边类型「{}」。支持：continue, related, references。",edge),
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
                    let preview: String = content.chars().take(500).collect();
                    out.push_str(&format!("**[{}] RRF={:.3} | source_id: {}**\n> {}\n\n", kw_str, rrf, source_display, preview.replace('\n', "\n> ")));
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
            database::get("SELECT id,title FROM todos WHERE id=?1", &[json!(tid.trim())])
                .ok().flatten().map(|r| (r["id"].as_str().unwrap_or("").to_string(), r["title"].as_str().unwrap_or("").to_string()))
        } else if !t.trim().is_empty() {
            let like = format!("%{}%", t.trim());
            database::get("SELECT id,title FROM todos WHERE title LIKE ?1 ORDER BY created_at DESC LIMIT 1", &[json!(&like)])
                .ok().flatten().map(|r| (r["id"].as_str().unwrap_or("").to_string(), r["title"].as_str().unwrap_or("").to_string()))
        } else { None }
    };
    match action {
        "create" => {
            if title.trim().is_empty() { return "创建待办需要提供 title。".into(); }
            let id = uuid::Uuid::new_v4().to_string();
            let p = normalize_pri(priority);
            let d = if due_date.trim().is_empty() { None } else { Some(due_date.trim()) };
            let r = if repeat.trim().is_empty() { None } else { Some(repeat.trim()) };
            match database::run("INSERT INTO todos(id,title,content,done,priority,due_date,repeat,created_at,updated_at) VALUES(?1,?2,?3,0,?4,?5,?6,datetime('now','localtime'),datetime('now','localtime'))",&[json!(&id),json!(title.trim()),json!(content.trim()),json!(p),json!(d),json!(r)]) {
                Ok(r) if r.changes > 0 => {
                    // 关联项目：支持 project_id 或项目名称（去掉 PROJECT- 前缀）
                    let raw_pid = project_id.trim();
                    let clean_pid = raw_pid.strip_prefix("PROJECT-").unwrap_or(raw_pid);
                    if !clean_pid.is_empty() {
                        eprintln!("[待办] 尝试关联项目: raw='{}', clean='{}'", raw_pid, clean_pid);
                        let pid = if let Ok(Some(_)) = database::get("SELECT 1 FROM projects WHERE id=?1", &[json!(clean_pid)]) {
                            eprintln!("[待办] 按ID匹配成功: {}", clean_pid);
                            clean_pid.to_string()
                        } else {
                            // 按名称查找项目
                            let found = database::get("SELECT id FROM projects WHERE name LIKE ?1 ORDER BY LENGTH(name) ASC LIMIT 1", &[json!(format!("%{}%", clean_pid))])
                                .ok().flatten().and_then(|r| r["id"].as_str().map(String::from)).unwrap_or_default();
                            eprintln!("[待办] 按名称查找: '{}' → found='{}'", clean_pid, found);
                            found
                        };
                        if !pid.is_empty() {
                            let r = database::run("INSERT OR IGNORE INTO todo_project_mapping(todo_id,project_id) VALUES(?1,?2)", &[json!(&id), json!(&pid)]);
                            eprintln!("[待办] 关联结果: todo_id={}, project_id={}, result={:?}", &id, &pid, r.map(|r| r.changes));
                            crate::services::knowledge_lifecycle::notify_changed("TODO", &id);
                        } else {
                            eprintln!("[待办] 未找到匹配项目: '{}'", project_id.trim());
                        }
                    }
                    format!("已创建待办：{}", title.trim())
                },
                Err(e) => format!("创建待办失败: {}", e),
                _ => "创建待办失败".into(),
            }
        }
        "update" => {
            let found = find_todo(todo_id, title);
            match found {
                Some((fid, ftitle)) => {
                    let mut sets = vec!["updated_at=datetime('now','localtime')".to_string()];
                    let mut params: Vec<serde_json::Value> = Vec::new();
                    if !content.trim().is_empty() { sets.push(format!("content=?{}", params.len()+1)); params.push(json!(content.trim())); }
                    if !priority.trim().is_empty() { sets.push(format!("priority=?{}", params.len()+1)); params.push(json!(normalize_pri(priority).unwrap_or_default())); }
                    if !due_date.trim().is_empty() { sets.push(format!("due_date=?{}", params.len()+1)); params.push(json!(due_date.trim())); }
                    if !repeat.trim().is_empty() { sets.push(format!("repeat=?{}", params.len()+1)); params.push(json!(repeat.trim())); }
                    if done { sets.push(format!("done=?{}", params.len()+1)); params.push(json!(1)); }
                    if params.is_empty() { return "更新待办需要提供要修改的字段。".into(); }
                    params.push(json!(&fid));
                    let sql = format!("UPDATE todos SET {} WHERE id=?", sets.join(", "));
                    match database::run(&sql, &params) {
                        Ok(r) if r.changes > 0 => format!("已更新待办「{}」", ftitle),
                        Err(e) => format!("更新待办失败: {}", e),
                        _ => format!("待办「{}」无变更。", ftitle),
                    }
                }
                None => "未找到匹配的待办。".into(),
            }
        }
        "delete" => {
            let found = find_todo(todo_id, title);
            match found {
                Some((fid, ftitle)) => {
                    match database::run("DELETE FROM todos WHERE id=?1", &[json!(&fid)]) {
                        Ok(r) if r.changes > 0 => format!("已删除待办「{}」", ftitle),
                        Err(e) => format!("删除待办失败: {}", e),
                        _ => format!("待办「{}」删除失败。", ftitle),
                    }
                }
                None => "未找到匹配的待办。".into(),
            }
        }
        _ => format!("未知操作「{}」。支持：create, update, delete。", action),
    }
}

fn ext_tool_manage_memory(action: &str, title: &str, content: &str, memory_id: &str, replace_ids: &str) -> String {
    match action {
        "create" => {
            let title = title.trim();
            let statement = content.trim();
            if title.is_empty() { return "创建记忆需要提供 title（完整知识点）。".into(); }
            let id = uuid::Uuid::new_v4().to_string();
            let actual_content = if statement.is_empty() { title } else { statement };
            match database::run(
                "INSERT INTO source_data(id,source_type,source_id,title,content,updated_at) VALUES(?1,'MEMORY',?1,?2,?3,datetime('now','localtime'))",
                &[json!(&id), json!(title), json!(&actual_content)],
            ) {
                Ok(r) if r.changes > 0 => {
                    // 自动删除被替换的旧记忆
                    let mut deleted_count = 0;
                    if !replace_ids.trim().is_empty() {
                        for rid in replace_ids.split(',') {
                            let rid = rid.trim();
                            if !rid.is_empty() && rid != id {
                                if let Ok(r) = database::run("DELETE FROM source_data WHERE source_type='MEMORY' AND source_id=?1", &[json!(rid)]) {
                                    if r.changes > 0 { deleted_count += 1; }
                                }
                            }
                        }
                    }
                    if deleted_count > 0 {
                        format!("已记住：{}（同时清理了 {} 条旧记忆）", title, deleted_count)
                    } else {
                        format!("已记住：{}", title)
                    }
                },
                Err(e) => format!("记忆失败: {}", e),
                _ => "记忆失败".into(),
            }
        }
        "delete" => {
            if !memory_id.trim().is_empty() {
                let sid = memory_id.trim();
                match database::run("DELETE FROM source_data WHERE source_type='MEMORY' AND source_id=?1", &[json!(sid)]) {
                    Ok(r) if r.changes > 0 => format!("已删除记忆: {}", sid),
                    Err(e) => format!("删除失败: {}", e),
                    _ => format!("未找到记忆: {}", sid),
                }
            } else if !title.trim().is_empty() {
                let like = format!("%{}%", title.trim());
                let found = database::get("SELECT source_id, title FROM source_data WHERE source_type='MEMORY' AND title LIKE ?1 ORDER BY created_at DESC LIMIT 1", &[json!(&like)]).ok().flatten();
                match found {
                    Some(row) => {
                        let sid = row["source_id"].as_str().unwrap_or("");
                        let name = row["title"].as_str().unwrap_or("");
                        match database::run("DELETE FROM source_data WHERE source_type='MEMORY' AND source_id=?1", &[json!(sid)]) {
                            Ok(r) if r.changes > 0 => format!("已删除记忆: {}", name),
                            Err(e) => format!("删除失败: {}", e),
                            _ => "删除失败。".into(),
                        }
                    }
                    None => format!("未找到匹配「{}」的记忆。", title.trim()),
                }
            } else {
                "删除需要提供 memory_id 或 title。".into()
            }
        }
        _ => format!("未知操作「{}」。支持：create, delete。", action),
    }
}

fn ext_tool_manage_project(
    action: &str,
    project_id: &str,
    name: &str,
    customer: &str,
    customer_contact: &str,
    customer_phone: &str,
    owner: &str,
    budget: f64,
    description: &str,
    bid_time: &str,
    status: &str,
    channel_id: &str,
    vendor_id: &str,
    contact_id: &str,
    parent_id: &str,
    parent_type: &str,
    phone: &str,
    company: &str,
    email: &str,
    remark: &str,
) -> String {
    // 统一处理 project_id：去掉 PROJECT- 前缀
    let raw_pid = project_id.trim();
    let clean_pid = raw_pid.strip_prefix("PROJECT-").unwrap_or(raw_pid);

    // 查找项目的辅助函数
    let find_project = |pid: &str| -> Option<(String, String)> {
        database::get("SELECT id, name FROM projects WHERE id=?1", &[json!(pid)])
            .ok().flatten()
            .map(|r| (r["id"].as_str().unwrap_or("").to_string(), r["name"].as_str().unwrap_or("").to_string()))
    };

    // 查找渠道/供应商的辅助函数
    let find_channel = |cid: &str| -> Option<(String, String, String)> {
        database::get("SELECT id, project_id, name FROM channel_partners WHERE id=?1", &[json!(cid)])
            .ok().flatten()
            .map(|r| (r["id"].as_str().unwrap_or("").to_string(), r["project_id"].as_str().unwrap_or("").to_string(), r["name"].as_str().unwrap_or("").to_string()))
    };
    let find_vendor = |vid: &str| -> Option<(String, String, String)> {
        database::get("SELECT id, project_id, name FROM vendor_partners WHERE id=?1", &[json!(vid)])
            .ok().flatten()
            .map(|r| (r["id"].as_str().unwrap_or("").to_string(), r["project_id"].as_str().unwrap_or("").to_string(), r["name"].as_str().unwrap_or("").to_string()))
    };

    match action {
        // ═══ 项目 CRUD ═══
        "create" => {
            if name.trim().is_empty() { return "创建项目需要提供 name。".into(); }
            let id = uuid::Uuid::new_v4().to_string();
            let bid = if bid_time.trim().is_empty() { None } else { Some(bid_time.trim()) };
            let st = if status.trim().is_empty() { "Lead" } else { status.trim() };
            let b = if budget > 0.0 { budget } else { 0.0 };
            match database::run(
                "INSERT INTO projects(id,name,description,status,owner,budget,customer,bid_time,customer_contact,customer_phone,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,datetime('now','localtime'),datetime('now','localtime'))",
                &[json!(&id), json!(name.trim()), json!(description.trim()), json!(st), json!(owner.trim()), json!(b), json!(customer.trim()), json!(bid), json!(customer_contact.trim()), json!(customer_phone.trim())],
            ) {
                Ok(r) if r.changes > 0 => {
                    // 记录动态
                    let did = uuid::Uuid::new_v4().to_string();
                    database::run("INSERT INTO project_dynamics(id,project_id,type,content,operator) VALUES(?1,?2,'system',?3,'AI Agent')", &[json!(&did), json!(&id), json!(format!("创建项目：{}", name.trim()))]).ok();
                    // 通知知识库
                    crate::services::knowledge_lifecycle::notify_changed("PROJECT", &id);
                    format!("已创建项目「{}」(ID: {})", name.trim(), id)
                },
                Err(e) => format!("创建项目失败: {}", e),
                _ => "创建项目失败".into(),
            }
        }
        "update" => {
            if clean_pid.is_empty() { return "更新项目需要提供 project_id。".into(); }
            let found = find_project(clean_pid);
            match found {
                Some((fid, fname)) => {
                    let mut sets = vec!["updated_at=datetime('now','localtime')".to_string()];
                    let mut params: Vec<serde_json::Value> = Vec::new();
                    if !name.trim().is_empty() { sets.push(format!("name=?{}", params.len()+1)); params.push(json!(name.trim())); }
                    if !customer.trim().is_empty() { sets.push(format!("customer=?{}", params.len()+1)); params.push(json!(customer.trim())); }
                    if !customer_contact.trim().is_empty() { sets.push(format!("customer_contact=?{}", params.len()+1)); params.push(json!(customer_contact.trim())); }
                    if !customer_phone.trim().is_empty() { sets.push(format!("customer_phone=?{}", params.len()+1)); params.push(json!(customer_phone.trim())); }
                    if !owner.trim().is_empty() { sets.push(format!("owner=?{}", params.len()+1)); params.push(json!(owner.trim())); }
                    if budget > 0.0 { sets.push(format!("budget=?{}", params.len()+1)); params.push(json!(budget)); }
                    if !description.trim().is_empty() { sets.push(format!("description=?{}", params.len()+1)); params.push(json!(description.trim())); }
                    if !bid_time.trim().is_empty() { sets.push(format!("bid_time=?{}", params.len()+1)); params.push(json!(bid_time.trim())); }
                    if !status.trim().is_empty() { sets.push(format!("status=?{}", params.len()+1)); params.push(json!(status.trim())); }
                    if params.is_empty() { return "更新项目需要提供要修改的字段。".into(); }
                    params.push(json!(&fid));
                    let sql = format!("UPDATE projects SET {} WHERE id=?", sets.join(", "));
                    match database::run(&sql, &params) {
                        Ok(r) if r.changes > 0 => {
                            crate::services::knowledge_lifecycle::notify_changed("PROJECT", &fid);
                            format!("已更新项目「{}」", fname)
                        },
                        Err(e) => format!("更新项目失败: {}", e),
                        _ => format!("项目「{}」无变更。", fname),
                    }
                }
                None => format!("未找到项目：{}", clean_pid),
            }
        }
        "delete" => {
            if clean_pid.is_empty() { return "删除项目需要提供 project_id。".into(); }
            let found = find_project(clean_pid);
            match found {
                Some((fid, fname)) => {
                    // 级联删除关联数据
                    database::run("DELETE FROM project_dynamics WHERE project_id=?1", &[json!(&fid)]).ok();
                    database::run("DELETE FROM note_project_mapping WHERE project_id=?1", &[json!(&fid)]).ok();
                    database::run("DELETE FROM todo_project_mapping WHERE project_id=?1", &[json!(&fid)]).ok();
                    database::run("DELETE FROM tag_mappings WHERE entity_type='projects' AND entity_id=?1", &[json!(&fid)]).ok();
                    // 删除方案及子项
                    if let Ok(solutions) = database::all("SELECT id FROM solutions WHERE project_id=?1", &[json!(&fid)]) {
                        for sol in &solutions {
                            if let Some(sid) = sol["id"].as_str() {
                                if let Ok(versions) = database::all("SELECT id FROM solution_versions WHERE solution_id=?1", &[json!(sid)]) {
                                    for ver in &versions {
                                        if let Some(vid) = ver["id"].as_str() {
                                            database::run("DELETE FROM solution_items WHERE solution_version_id=?1", &[json!(vid)]).ok();
                                        }
                                    }
                                    database::run("DELETE FROM solution_versions WHERE solution_id=?1", &[json!(sid)]).ok();
                                }
                            }
                        }
                        database::run("DELETE FROM solutions WHERE project_id=?1", &[json!(&fid)]).ok();
                    }
                    // 删除渠道及联系人
                    if let Ok(channels) = database::all("SELECT id FROM channel_partners WHERE project_id=?1", &[json!(&fid)]) {
                        for ch in &channels {
                            if let Some(cid) = ch["id"].as_str() {
                                database::run("DELETE FROM channel_contacts WHERE channel_id=?1", &[json!(cid)]).ok();
                            }
                        }
                        database::run("DELETE FROM channel_partners WHERE project_id=?1", &[json!(&fid)]).ok();
                    }
                    // 删除供应商及联系人
                    if let Ok(vendors) = database::all("SELECT id FROM vendor_partners WHERE project_id=?1", &[json!(&fid)]) {
                        for v in &vendors {
                            if let Some(vid) = v["id"].as_str() {
                                database::run("DELETE FROM vendor_contacts WHERE vendor_id=?1", &[json!(vid)]).ok();
                            }
                        }
                        database::run("DELETE FROM vendor_partners WHERE project_id=?1", &[json!(&fid)]).ok();
                    }
                    // 删除项目
                    match database::run("DELETE FROM projects WHERE id=?1", &[json!(&fid)]) {
                        Ok(r) if r.changes > 0 => {
                            crate::services::knowledge_lifecycle::notify_deleted("PROJECT", &fid);
                            format!("已删除项目「{}」及所有关联数据", fname)
                        },
                        Err(e) => format!("删除项目失败: {}", e),
                        _ => format!("项目「{}」删除失败。", fname),
                    }
                }
                None => format!("未找到项目：{}", clean_pid),
            }
        }

        // ═══ 渠道管理 ═══
        "add_channel" => {
            if clean_pid.is_empty() || name.trim().is_empty() { return "添加渠道需要 project_id 和 name。".into(); }
            if find_project(clean_pid).is_none() { return format!("未找到项目：{}", clean_pid); }
            let id = uuid::Uuid::new_v4().to_string();
            match database::run(
                "INSERT INTO channel_partners(id,project_id,name,phone,company,remark,created_at) VALUES(?1,?2,?3,?4,?5,?6,datetime('now','localtime'))",
                &[json!(&id), json!(clean_pid), json!(name.trim()), json!(phone.trim()), json!(company.trim()), json!(remark.trim())],
            ) {
                Ok(r) if r.changes > 0 => {
                    let did = uuid::Uuid::new_v4().to_string();
                    database::run("INSERT INTO project_dynamics(id,project_id,type,content,operator) VALUES(?1,?2,'system',?3,'AI Agent')", &[json!(&did), json!(clean_pid), json!(format!("添加渠道伙伴：{}", name.trim()))]).ok();
                    format!("已添加渠道「{}」(channel_id: {})", name.trim(), id)
                },
                Err(e) => format!("添加渠道失败: {}", e),
                _ => "添加渠道失败".into(),
            }
        }
        "update_channel" => {
            if channel_id.trim().is_empty() { return "更新渠道需要 channel_id。".into(); }
            let found = find_channel(channel_id.trim());
            match found {
                Some((fid, _pid, fname)) => {
                    let mut sets = Vec::new();
                    let mut params: Vec<serde_json::Value> = Vec::new();
                    if !name.trim().is_empty() { sets.push(format!("name=?{}", params.len()+1)); params.push(json!(name.trim())); }
                    if !phone.trim().is_empty() { sets.push(format!("phone=?{}", params.len()+1)); params.push(json!(phone.trim())); }
                    if !company.trim().is_empty() { sets.push(format!("company=?{}", params.len()+1)); params.push(json!(company.trim())); }
                    if !remark.trim().is_empty() { sets.push(format!("remark=?{}", params.len()+1)); params.push(json!(remark.trim())); }
                    if params.is_empty() { return "更新渠道需要提供要修改的字段。".into(); }
                    params.push(json!(&fid));
                    let sql = format!("UPDATE channel_partners SET {} WHERE id=?", sets.join(", "));
                    match database::run(&sql, &params) {
                        Ok(r) if r.changes > 0 => format!("已更新渠道「{}」", fname),
                        Err(e) => format!("更新渠道失败: {}", e),
                        _ => format!("渠道「{}」无变更。", fname),
                    }
                }
                None => format!("未找到渠道：{}", channel_id.trim()),
            }
        }
        "delete_channel" => {
            if channel_id.trim().is_empty() { return "删除渠道需要 channel_id。".into(); }
            let found = find_channel(channel_id.trim());
            match found {
                Some((fid, _pid, fname)) => {
                    database::run("DELETE FROM channel_contacts WHERE channel_id=?1", &[json!(&fid)]).ok();
                    match database::run("DELETE FROM channel_partners WHERE id=?1", &[json!(&fid)]) {
                        Ok(r) if r.changes > 0 => format!("已删除渠道「{}」及联系人", fname),
                        Err(e) => format!("删除渠道失败: {}", e),
                        _ => format!("渠道「{}」删除失败。", fname),
                    }
                }
                None => format!("未找到渠道：{}", channel_id.trim()),
            }
        }

        // ═══ 供应商管理 ═══
        "add_vendor" => {
            if clean_pid.is_empty() || name.trim().is_empty() { return "添加供应商需要 project_id 和 name。".into(); }
            if find_project(clean_pid).is_none() { return format!("未找到项目：{}", clean_pid); }
            let id = uuid::Uuid::new_v4().to_string();
            match database::run(
                "INSERT INTO vendor_partners(id,project_id,name,phone,company,remark,created_at) VALUES(?1,?2,?3,?4,?5,?6,datetime('now','localtime'))",
                &[json!(&id), json!(clean_pid), json!(name.trim()), json!(phone.trim()), json!(company.trim()), json!(remark.trim())],
            ) {
                Ok(r) if r.changes > 0 => {
                    let did = uuid::Uuid::new_v4().to_string();
                    database::run("INSERT INTO project_dynamics(id,project_id,type,content,operator) VALUES(?1,?2,'system',?3,'AI Agent')", &[json!(&did), json!(clean_pid), json!(format!("添加供应商伙伴：{}", name.trim()))]).ok();
                    format!("已添加供应商「{}」(vendor_id: {})", name.trim(), id)
                },
                Err(e) => format!("添加供应商失败: {}", e),
                _ => "添加供应商失败".into(),
            }
        }
        "update_vendor" => {
            if vendor_id.trim().is_empty() { return "更新供应商需要 vendor_id。".into(); }
            let found = find_vendor(vendor_id.trim());
            match found {
                Some((fid, _pid, fname)) => {
                    let mut sets = Vec::new();
                    let mut params: Vec<serde_json::Value> = Vec::new();
                    if !name.trim().is_empty() { sets.push(format!("name=?{}", params.len()+1)); params.push(json!(name.trim())); }
                    if !phone.trim().is_empty() { sets.push(format!("phone=?{}", params.len()+1)); params.push(json!(phone.trim())); }
                    if !company.trim().is_empty() { sets.push(format!("company=?{}", params.len()+1)); params.push(json!(company.trim())); }
                    if !remark.trim().is_empty() { sets.push(format!("remark=?{}", params.len()+1)); params.push(json!(remark.trim())); }
                    if params.is_empty() { return "更新供应商需要提供要修改的字段。".into(); }
                    params.push(json!(&fid));
                    let sql = format!("UPDATE vendor_partners SET {} WHERE id=?", sets.join(", "));
                    match database::run(&sql, &params) {
                        Ok(r) if r.changes > 0 => format!("已更新供应商「{}」", fname),
                        Err(e) => format!("更新供应商失败: {}", e),
                        _ => format!("供应商「{}」无变更。", fname),
                    }
                }
                None => format!("未找到供应商：{}", vendor_id.trim()),
            }
        }
        "delete_vendor" => {
            if vendor_id.trim().is_empty() { return "删除供应商需要 vendor_id。".into(); }
            let found = find_vendor(vendor_id.trim());
            match found {
                Some((fid, _pid, fname)) => {
                    database::run("DELETE FROM vendor_contacts WHERE vendor_id=?1", &[json!(&fid)]).ok();
                    match database::run("DELETE FROM vendor_partners WHERE id=?1", &[json!(&fid)]) {
                        Ok(r) if r.changes > 0 => format!("已删除供应商「{}」及联系人", fname),
                        Err(e) => format!("删除供应商失败: {}", e),
                        _ => format!("供应商「{}」删除失败。", fname),
                    }
                }
                None => format!("未找到供应商：{}", vendor_id.trim()),
            }
        }

        // ═══ 联系人管理 ═══
        "add_contact" => {
            if parent_id.trim().is_empty() || name.trim().is_empty() { return "添加联系人需要 parent_id 和 name。".into(); }
            let id = uuid::Uuid::new_v4().to_string();
            let result = match parent_type.trim() {
                "channel" => {
                    if find_channel(parent_id.trim()).is_none() { return format!("未找到渠道：{}", parent_id.trim()); }
                    database::run(
                        "INSERT INTO channel_contacts(id,channel_id,name,phone,email,remark,created_at) VALUES(?1,?2,?3,?4,?5,?6,datetime('now','localtime'))",
                        &[json!(&id), json!(parent_id.trim()), json!(name.trim()), json!(phone.trim()), json!(email.trim()), json!(remark.trim())],
                    )
                }
                "vendor" => {
                    if find_vendor(parent_id.trim()).is_none() { return format!("未找到供应商：{}", parent_id.trim()); }
                    database::run(
                        "INSERT INTO vendor_contacts(id,vendor_id,name,phone,email,remark,created_at) VALUES(?1,?2,?3,?4,?5,?6,datetime('now','localtime'))",
                        &[json!(&id), json!(parent_id.trim()), json!(name.trim()), json!(phone.trim()), json!(email.trim()), json!(remark.trim())],
                    )
                }
                _ => return "parent_type 必须是 channel 或 vendor。".into(),
            };
            match result {
                Ok(r) if r.changes > 0 => format!("已添加联系人「{}」", name.trim()),
                Err(e) => format!("添加联系人失败: {}", e),
                _ => "添加联系人失败".into(),
            }
        }
        "delete_contact" => {
            if contact_id.trim().is_empty() { return "删除联系人需要 contact_id。".into(); }
            let result = match parent_type.trim() {
                "channel" => database::run("DELETE FROM channel_contacts WHERE id=?1", &[json!(contact_id.trim())]),
                "vendor" => database::run("DELETE FROM vendor_contacts WHERE id=?1", &[json!(contact_id.trim())]),
                _ => return "parent_type 必须是 channel 或 vendor。".into(),
            };
            match result {
                Ok(r) if r.changes > 0 => "已删除联系人".to_string(),
                Err(e) => format!("删除联系人失败: {}", e),
                _ => "未找到联系人或已删除。".to_string(),
            }
        }

        _ => format!("未知操作「{}」。支持：create, update, delete, add_channel, add_vendor, add_contact, update_channel, update_vendor, delete_channel, delete_vendor, delete_contact。", action),
    }
}

/// 被动提醒：检查待办到期、项目停滞、投标截止，生成提醒文本注入 LLM 上下文
fn check_proactive_reminders() -> String {
    let mut reminders = Vec::new();

    // 1. 24h 内到期的待办
    if let Ok(rows) = database::all(
        "SELECT title, due_date FROM todos WHERE done=0 AND due_date IS NOT NULL AND due_date <= date('now','+1 day') AND due_date >= date('now') ORDER BY due_date ASC LIMIT 5",
        &[],
    ) {
        if !rows.is_empty() {
            let items: Vec<String> = rows.iter().map(|r| {
                let title = r["title"].as_str().unwrap_or("");
                let due = r["due_date"].as_str().unwrap_or("");
                format!("• {} (截止: {})", title, due)
            }).collect();
            reminders.push(format!("⏰ 待办即将到期：\n{}", items.join("\n")));
        }
    }

    // 2. 今天到期的待办（最高优先级）
    if let Ok(rows) = database::all(
        "SELECT title, due_date FROM todos WHERE done=0 AND due_date=date('now') ORDER BY due_date ASC LIMIT 5",
        &[],
    ) {
        if !rows.is_empty() {
            let items: Vec<String> = rows.iter().map(|r| {
                let title = r["title"].as_str().unwrap_or("");
                format!("• {}", title)
            }).collect();
            reminders.push(format!("🔴 今天到期：\n{}", items.join("\n")));
        }
    }

    // 3. 7 天未更新的项目
    if let Ok(rows) = database::all(
        "SELECT name, updated_at FROM projects WHERE updated_at < date('now','-7 days') ORDER BY updated_at ASC LIMIT 3",
        &[],
    ) {
        if !rows.is_empty() {
            let items: Vec<String> = rows.iter().map(|r| {
                let name = r["name"].as_str().unwrap_or("");
                let updated = r["updated_at"].as_str().unwrap_or("");
                format!("• {} (最后更新: {})", name, updated)
            }).collect();
            reminders.push(format!("📌 项目长期未更新：\n{}", items.join("\n")));
        }
    }

    // 4. 3 天内投标截止
    if let Ok(rows) = database::all(
        "SELECT name, bid_time FROM projects WHERE bid_time IS NOT NULL AND bid_time != '' AND bid_time <= date('now','+3 days') AND bid_time >= date('now') ORDER BY bid_time ASC LIMIT 3",
        &[],
    ) {
        if !rows.is_empty() {
            let items: Vec<String> = rows.iter().map(|r| {
                let name = r["name"].as_str().unwrap_or("");
                let bid = r["bid_time"].as_str().unwrap_or("");
                format!("• {} (投标: {})", name, bid)
            }).collect();
            reminders.push(format!("🎯 投标截止临近：\n{}", items.join("\n")));
        }
    }

    if reminders.is_empty() {
        String::new()
    } else {
        format!("【系统提醒】\n{}", reminders.join("\n\n"))
    }
}

fn execute_act(action: &str, project_id: &str, new_status: &str) -> String {
    match action {
        "update_project_status" => {
            if project_id.trim().is_empty()||new_status.trim().is_empty() { return "更新项目状态需要 project_id 和 new_status。".into(); }
            match database::run("UPDATE projects SET status=?1, updated_at=datetime('now','localtime') WHERE id=?2",&[json!(new_status.trim()),json!(project_id.trim())]) { Ok(r) if r.changes>0 => { let did = uuid::Uuid::new_v4().to_string(); database::run("INSERT INTO project_dynamics(id,project_id,type,content,operator) VALUES(?1,?2,'system',?3,'AI Agent')",&[json!(&did),json!(project_id),json!(format!("项目状态更新为：{}",new_status.trim()))]).ok(); format!("已更新项目 {} 状态为「{}」",project_id,new_status.trim()) } Err(e) => format!("更新项目失败: {}",e), _ => format!("项目 {} 未找到或无变更。",project_id) }
        }
        _ => format!("未知操作「{}」。支持：update_project_status。", action),
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

fn fetch_parent_chunks(source_id_str: &str) -> String {
    let (ot, oi) = if let Some(dash) = source_id_str.find('-') { source_id_str.split_at(dash) } else { return "".to_string() };
    let oi = &oi[1..];
    let sdid = database::get("SELECT id FROM source_data WHERE source_type=?1 AND source_id=?2", &[json!(ot), json!(oi)]).ok().flatten().and_then(|r| r["id"].as_i64()).unwrap_or(0);
    if sdid == 0 { return format!("未找到对象: {source_id_str}"); }
    let chunks: Vec<String> = database::all("SELECT chunk_id, content, chunk_index FROM knowledge_chunks WHERE source_data_id=?1 AND parent_id IS NULL ORDER BY chunk_index LIMIT 3", &[json!(sdid)]).unwrap_or_default().iter().map(|r| {
        let content = r["content"].as_str().unwrap_or("");
        let chunk_id = r["chunk_id"].as_str().unwrap_or("");
        format!("[OBJECT-{} 父块{:?}] ChunkID: {}\n{}\n", ot, r["chunk_index"].as_i64().unwrap_or(0)+1, &chunk_id[..chunk_id.len().min(8)], &content[..content.len().min(1800)])
    }).collect();
    if chunks.is_empty() { format!("对象 {source_id_str} 无可用内容。") } else { chunks.join("\n") }
}

fn relative_time(created_at: &str) -> String {
    if created_at.is_empty() { return String::new(); }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let ts = chrono::NaiveDateTime::parse_from_str(created_at, "%Y-%m-%d %H:%M:%S").map(|dt| dt.and_utc().timestamp()).unwrap_or(0);
    let delta = (now - ts).max(0);
    let rel = if delta < 60 { "刚刚".to_string() } else if delta < 3600 { format!("{}分钟前", delta / 60) } else if delta < 86400 { format!("{}小时前", delta / 3600) } else if delta < 2592000 { format!("{}天前", delta / 86400) } else if delta < 77760000 { format!("{}个月前", delta / 2592000) } else { format!("{}年前", delta / 31104000) };
    format!("🕐 {} ({})", rel, &created_at[..created_at.len().min(16)])
}

fn execute_object_get(query: &str) -> String {
    let q = query.trim();
    if q.is_empty() { return "请提供搜索关键词。".into(); }

    // ★ FTS5 使用 NEAR 弹性距离查询替代 OR，容忍实体名中的间隔字
    let words: Vec<String> = q.split_whitespace().map(|k| k.replace('"',"")).collect();
    let fts_q = if words.len() > 1 {
        format!("NEAR(({}), 30)", words.join(" "))
    } else {
        words.first().cloned().unwrap_or_default()
    };
    if fts_q.is_empty() { return format!("关键词「{}」未匹配到任何对象。", q); }

    // ★ 先读 source_data_fts
    let results = match database::fts5_search_content(&fts_q, 30) {
        Ok(r) => r,
        Err(_) => Vec::new(),
    };

    let mut seen = std::collections::HashSet::new();
    let mut ranked: Vec<(String, String, f64)> = Vec::new();

    // ★ FTS5 结果按 source_type 分组取 top 3，确保每种类型都有代表
    {
        use std::collections::HashMap;
        let mut type_indices: HashMap<String, Vec<(usize, f64)>> = HashMap::new();
        for (i, r) in results.iter().enumerate() {
            let st = r.entity_type.to_uppercase();
            type_indices.entry(st).or_default().push((i, r.rank));
        }
        for (_st, indices) in &type_indices {
            let mut sorted = indices.clone();
            sorted.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
            for (idx, _rank) in sorted.iter().take(3) {
                let r = &results[*idx];
                let sid = r.entity_id.trim().to_string();
                if seen.insert(format!("{}-{}", r.entity_type.to_uppercase(), sid)) {
                    ranked.push((r.entity_type.to_uppercase(), sid, r.rank));
                }
            }
        }
    }

    // ★ LIKE 查询 source_data 表（补充 FTS5 未命中的数据）
    {
        let like = format!("%{}%", q.split_whitespace().next().unwrap_or(q));
        if let Ok(rows) = database::all(
            "SELECT source_type, source_id, title FROM source_data WHERE title LIKE ?1 OR content LIKE ?1 LIMIT 20",
            &[json!(&like)],
        ) {
            for r in &rows {
                let st = r["source_type"].as_str().unwrap_or("").to_string();
                let sid = r["source_id"].as_str().unwrap_or("").to_string();
                if seen.insert(format!("{}-{}", st, sid)) {
                    ranked.push((st, sid, 0.0));
                }
            }
        }
    }

    // ★ LIKE 查询 file_relations.custom_name
    let words: Vec<&str> = q.split_whitespace().collect();
    for word in words.iter().take(3) {
        let like = format!("%{}%", word);
        if let Ok(rows) = database::all(
            "SELECT DISTINCT file_id, custom_name FROM file_relations WHERE custom_name LIKE ?1 AND custom_name != '' LIMIT 10",
            &[json!(&like)],
        ) {
            for r in &rows {
                let fid = r["file_id"].as_str().unwrap_or("").to_string();
                if fid.is_empty() { continue; }
                if seen.insert(format!("FILE-{}", fid)) {
                    ranked.push(("FILE".to_string(), fid, 0.0));
                }
            }
            if words.len() == 1 { break; }
        }
    }

    // ★ 标签/实体兜底检索：FTS5 和 LIKE 都未命中时，通过标签名匹配发现对象
    if ranked.is_empty() {
        for word in q.split_whitespace().take(3) {
            let like = format!("%{}%", word);
            if let Ok(rows) = database::all(
                "SELECT tm.entity_type, tm.entity_id FROM tag_mappings tm JOIN tags t ON t.tag_id=tm.tag_id WHERE t.name LIKE ?1 LIMIT 10",
                &[json!(&like)],
            ) {
                for r in &rows {
                    let et = r["entity_type"].as_str().unwrap_or("").to_uppercase();
                    let eid = r["entity_id"].as_str().unwrap_or("").to_string();
                    if seen.insert(format!("{}-{}", et, eid)) {
                        ranked.push((et, eid, 0.0));
                    }
                }
                if !ranked.is_empty() { break; }
            }
        }
    }

    if ranked.is_empty() { return format!("关键词「{}」未匹配到任何对象。", q); }

    // ★ 排序：FTS5 结果按 rank 降序（高相关优先），file_relations 补集（rank=0.0）排在末尾
    ranked.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));

    let mut out = String::new();

    for (st, sid, _rank) in ranked.iter().take(10) {
        match st.as_str() {
            "PRODUCT" | "PROJECT" | "NOTE" | "TODO" => {
                let stype = st.as_str();
                if let Ok(Some(row)) = database::get("SELECT title FROM source_data WHERE source_type=?1 AND source_id=?2",&[json!(stype), json!(sid)]) {
                    let n = row["title"].as_str().unwrap_or("");
                    out.push_str(&format!("  [{}] {} | source_id: {}-{}\n", stype.to_uppercase(), n, stype.to_uppercase(), sid));
                }
            }
            "FILE" => {
                if let Ok(Some(row)) = database::get("SELECT fa.id,fr.custom_name FROM file_assets fa LEFT JOIN file_relations fr ON fr.file_id=fa.id WHERE fa.id=?1 LIMIT 1",&[json!(sid)]) {
                    let name = row["custom_name"].as_str().filter(|x|!x.is_empty()).or_else(|| row["id"].as_str()).unwrap_or("");
                    out.push_str(&format!("  [FILE] {} | source_id: FILE-{}\n", name, sid));
                }
            }
            _ => {}
        }
    }

    let result = out.trim().to_string();
    if !result.is_empty() {
        format!("{}\n\n💡 对象的详细参数已物化为知识碎片。使用 search 工具检索所需具体内容。", result)
    } else {
        result
    }
}

// ═══════════════════════════════════════════════════════════════════
// Main Agent Loop
// ═══════════════════════════════════════════════════════════════════

/// 从 LLM 最终回答中提取 <refs> 格式的引用对象 ID
fn extract_citations(text: &str) -> Vec<String> {
    if let Some(s) = text.find("<refs>") {
        if let Some(e) = text[s..].find("</refs>") {
            return text[s+6..s+e].split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect();
        }
    }
    vec![]
}

/// 记录认知经历：用户问了什么、查了什么、用了什么工具、完成了什么
/// 写入 MEMORY 类型（认知过程格式）
fn record_experience(query: &str, session: &SessionMemory, _tool_log: &[Value], answer: &str) {
    if query.trim().is_empty() { return; }

    let sources: Vec<String> = session.active_objects.iter().take(5).map(|o| {
        format!("{} ({})", o.name, o.obj_type.to_uppercase())
    }).collect();
    let sources_str = if sources.is_empty() { "无特定来源".to_string() } else { sources.join("、") };

    let now = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
    let answer_summary: String = answer.chars().take(200).collect();

    let id = uuid::Uuid::new_v4().to_string();
    let content = format!(
        "[时间] {}\n[用户问] {}\n[查阅] {}\n[结论] {}",
        now, query, sources_str, answer_summary
    );
    let _ = database::run(
        "INSERT INTO source_data(id,source_type,source_id,title,content,updated_at) VALUES(?1,'MEMORY',?1,?2,?3,datetime('now','localtime'))",
        &[json!(&id), json!(query), json!(&content)],
    );
}

fn ext_tool_record_learning(question: &str, provider: &str, answer: &str) -> String {
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
    let id = uuid::Uuid::new_v4().to_string();
    let content = format!(
        "[时间] {}\n[用户问] {}\n[查阅] {}\n[结论] {}",
        now, question, provider, answer
    );
    let _ = database::run(
        "INSERT INTO source_data(id,source_type,source_id,title,content,updated_at) VALUES(?1,'MEMORY',?1,?2,?3,datetime('now','localtime'))",
        &[json!(&id), json!(question), json!(&content)],
    );
    format!("已记录认知经历: {}", question)
}


#[tauri::command]
pub async fn agent_chat_workflow(user_query: String, max_iterations: Option<i32>, history: Vec<Value>, app_handle: Option<tauri::AppHandle>) -> Result<Value, String> {
    let query = user_query.trim().to_string();
    let max_iters = max_iterations.unwrap_or(8).clamp(2, 20);
    if query.is_empty() { return Ok(json!({"success":true,"answer":"请输入问题。","iterations":0})); }
    crate::services::chat_api::invalidate_chat_config_cache();
    { let mut c = API_CACHE.write().unwrap(); c.3 = Instant::now() - std::time::Duration::from_secs(60); }
    let (_, api_key, _model) = get_api_endpoint();
    if api_key.is_empty() { return Ok(json!({"success":true,"answer":"请先在系统设置中配置 Chat API 密钥。"})); }

    let tools = build_tools_definitions();
    let mut session = SessionMemory::new();
    let mut messages: Vec<ChatMessage> = vec![
        ChatMessage{role:MessageRole::System,content:Some(AGENT_SYSTEM_PROMPT.to_string()),tool_calls:None,tool_call_id:None,name:None,reasoning_content:None},
    ];
    // ★ 恢复历史消息中的 assistant/tool 对话 + 重建 SessionMemory
    for h in &history {
        let role = h["role"].as_str().unwrap_or("");
        match role {
            "assistant" => {
                let content = h["content"].as_str().map(|s| s.to_string());
                let tcs: Option<Vec<ToolCall>> = h["tool_calls"].as_array().map(|arr| arr.iter().filter_map(|tc| {
                    let id = tc["id"].as_str()?;
                    let name = tc["function"]["name"].as_str()?;
                    let args_str = tc["function"]["arguments"].as_str().unwrap_or("");
                    let args = if args_str.is_empty() {
                        serde_json::to_string(&tc["function"]["arguments"]).unwrap_or_default()
                    } else { args_str.to_string() };
                    Some(ToolCall { id: id.to_string(), call_type: "function".into(), function: FunctionCall { name: name.to_string(), arguments: args } })
                }).collect());
                messages.push(ChatMessage{role:MessageRole::Assistant,content:content.clone(),tool_calls:tcs,tool_call_id:None,name:None,reasoning_content:h["reasoning_content"].as_str().map(String::from)});
                // 没有 tool_calls 的 content 是最终回答，记录为 observation
                if let Some(ref text) = content {
                    if h["tool_calls"].as_array().map(|a| a.is_empty()).unwrap_or(true) {
                        session.record_observation(text);
                    }
                }
            }
            "tool" => {
                let content_str = h["content"].as_str().unwrap_or("").to_string();
                let id = h["tool_call_id"].as_str().unwrap_or("").to_string();
                let name = h["name"].as_str().unwrap_or("").to_string();
                messages.push(ChatMessage{role:MessageRole::Tool,content:Some(content_str.clone()),tool_calls:None,tool_call_id:Some(id),reasoning_content:None,name:Some(name)});
                // 从 tool 结果中重建 SessionMemory
                for line in content_str.lines() {
                    session.update_from_line(line);
                }
            }
            _ => {
                // user 或其他角色
                let content = h["content"].as_str().map(|s| s.to_string());
                messages.push(ChatMessage{role:MessageRole::User,content,tool_calls:None,tool_call_id:None,name:None,reasoning_content:None});
            }
        }
    }
    // ★ 追加本轮新问题
    messages.push(ChatMessage{role:MessageRole::User,content:Some(query.clone()),tool_calls:None,tool_call_id:None,name:None,reasoning_content:None});

    // ★ 被动提醒：注入待办到期/项目停滞/投标截止等提醒
    let reminders = check_proactive_reminders();
    if !reminders.is_empty() {
        eprintln!("[主动提醒] 注入提醒: {}", &reminders[..reminders.len().min(100)]);
        messages.push(ChatMessage{role:MessageRole::System,content:Some(format!("{}（你应该在回答中自然地提到这些提醒，但不要让用户觉得你在念清单。如果用户的问题与某个提醒相关，优先关联回答。）", reminders)),tool_calls:None,tool_call_id:None,name:None,reasoning_content:None});
    }

    // ★ 从活跃对象和本轮 query 设置 Focus
    let top_entities: Vec<String> = {
        let mut sorted: Vec<&ActiveObject> = session.active_objects.iter().collect();
        sorted.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        sorted.iter().take(3).map(|o| o.name.clone()).collect()
    };
    session.set_focus_from_user_query(&query, &top_entities);

    let mut tool_log: Vec<Value> = Vec::new();
    let (mut iters, mut pt, mut ct) = (0i32, 0i64, 0i64);
    let mut budget = 100i32;
    let mut search_cost = 10i32;
    let mut traverse_source_counter: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    
    // 加载用户策略（policy）— 从 source_data 读取
    let user_policies = {
        let rows = crate::database::all(
            "SELECT source_id, title, content, updated_at FROM source_data WHERE source_type='POLICY' ORDER BY updated_at DESC",
            &[],
        ).unwrap_or_default();
        if rows.is_empty() {
            "无特殊策略要求。".to_string()
        } else {
            let mut lines: Vec<String> = Vec::new();
            for r in rows.iter() {
                if let (Some(sid), Some(content)) = (r["source_id"].as_str(), r["content"].as_str()) {
                    let date = r["updated_at"].as_str().unwrap_or("").get(..10).unwrap_or("");
                    lines.push(format!("[ID:{}] {} {}", sid, date, content));
                }
            }
            lines.join("\n")
        }
    };
    
    // 注入初始 scratchpad（空），后续每轮由循环内代码更新
    let empty_sys = AGENT_SYSTEM_PROMPT
        .replace("{{today}}", &chrono::Local::now().format("%Y年%m月%d日").to_string())
        .replace("{{user_policies}}", &user_policies)
        .replace("{{session_scratchpad}}", "");
    messages[0].content = Some(empty_sys);

    loop {
        eprintln!("[LLM检索] 预算: {}/{} cost={}", budget, 100i32, search_cost);
        if let Some(ref h) = app_handle { let _ = h.emit("agent:status", json!({"stage":"thinking"})); }
        // ★ 每轮注意力衰减
        let prev_count = session.active_objects.len();
        for o in &mut session.active_objects { o.score *= 0.5; }
        // 移除从未被搜索命中的对象（score=0 说明从未被检索命中）
        session.active_objects.retain(|o| o.score > 0.0 || o.access_order >= session.access_counter.saturating_sub(1));
        let removed = prev_count - session.active_objects.len();
        if removed > 0 { eprintln!("[注意力] 衰减后移除 {} 个未命中对象，剩余 {}", removed, session.active_objects.len()); }
        iters += 1;

        // ★ 注入 scratchpad + budget_info 到 system prompt（供当前轮 LLM 读取）
        let scratchpad = session.generate_scratchpad();
        let new_sys = AGENT_SYSTEM_PROMPT
            .replace("{{today}}", &chrono::Local::now().format("%Y年%m月%d日").to_string())
            .replace("{{user_policies}}", &user_policies)
            .replace("{{session_scratchpad}}", &scratchpad);
        let budget_info = format!("\n\n## 🧠 思维预算\n- 剩余: {} | 本轮成本: {}\n- 预算有限，不要全部搜索轮次都用完。证据足够就立刻回答。", budget, search_cost);
        let new_sys = format!("{}{}", new_sys, budget_info);
        if let Some(ref mut content) = messages[0].content {
            *content = new_sys;
        }

        let ar = call_llm_with_tools(&messages, &tools, &app_handle).await?;
        if let Some(u) = ar.get("usage") { pt += u["prompt_tokens"].as_i64().unwrap_or(0); ct += u["completion_tokens"].as_i64().unwrap_or(0); }
        let choice = ar["choices"].as_array().and_then(|a|a.first()).ok_or("no choices")?;
        let msg = &choice["message"];
        let ac = msg["content"].as_str().unwrap_or("");
        let reasoning = msg["reasoning_content"].as_str().map(String::from);
        let mut tcs: Option<Vec<Value>> = msg["tool_calls"].as_array().map(|a|a.to_vec());

        // ★ 兜底：如果 tool_calls 为空但 content 包含 XML 格式工具调用，尝试解析
        if tcs.is_none() || tcs.as_ref().map_or(false, |v| v.is_empty()) {
            if let Some(parsed) = parse_xml_tool_calls(ac) {
                eprintln!("[LLM检索] 从 content 解析到 XML 工具调用: {:?}", parsed);
                tcs = Some(parsed);
            }
        }

        if !ac.is_empty() { if let Some(ref h) = app_handle { let _ = h.emit("agent:thought", json!({"text": ac})); } }

        match tcs {
            None => {
                let answer = if ac.is_empty() { "未能生成有效回答。" } else { ac };
                if let Some(ref h) = app_handle { let _ = h.emit("agent:status", json!({"stage":"answer"})); let _ = h.emit("agent:final-answer", json!({"answer": answer})); }
                if !ac.is_empty() { session.record_observation(ac); let cited = extract_citations(&ac); eprintln!("[LLM检索] cited_ids(from final): {:?}", cited); if cited.len() >= 2 { crate::services::synapse::update_object_links_weighted(&cited, 1.0); } }
                record_experience(&query, &session, &tool_log, answer);
                let ws = session.generate_scratchpad(); return Ok(json!({"success":true,"answer":answer,"workspace":ws,"iterations":iters,"promptTokens":pt,"completionTokens":ct,"tool_log":tool_log}));
            }
            Some(ref calls) if !calls.is_empty() => {
                // ★ 预算检查：耗尽时强制回答
                if budget < 0 {
                    if let Some(ref h) = app_handle {
                        let kw = calls.first().and_then(|tc| tc["function"]["arguments"].as_str().map(|s| s.to_string())).unwrap_or_default();
                        let _ = h.emit("agent:budget-low", json!({"remaining":budget,"cost":search_cost,"next_query":kw}));
                    }
                    let ans = call_llm_without_tools(&messages, &app_handle).await.unwrap_or_else(|e| format!("总结失败: {e}"));
                    let ws = session.generate_scratchpad();
                    record_experience(&query, &session, &tool_log, &ans);
                    return Ok(json!({"success":true,"answer":ans,"workspace":ws,"iterations":iters,"budget_exhausted":true,"promptTokens":pt,"completionTokens":ct,"tool_log":tool_log}));
                }
                if iters >= max_iters {
                    messages.push(ChatMessage{role:MessageRole::User,content:Some("已达到工具调用上限，请直接总结。".into()),tool_calls:None,tool_call_id:None,name:None,reasoning_content:None});
                    let ans = call_llm_without_tools(&messages, &app_handle).await.unwrap_or_else(|e| format!("总结失败: {e}"));
                    let ws = session.generate_scratchpad();
                    if let Some(ref h) = app_handle { let _ = h.emit("agent:status", json!({"stage":"answer"})); let _ = h.emit("agent:final-answer", json!({"answer": ans})); }
                    record_experience(&query, &session, &tool_log, &ans);
                    return Ok(json!({"success":true,"answer":ans,"workspace":ws,"iterations":iters,"circuit_breaker":true,"promptTokens":pt,"completionTokens":ct,"tool_log":tool_log}));
                }
            // ── Object Retrieval (dual-mode: timeline + keyword) ─────────────────

fn timeline_mode(obj_type: &str, start: &str, end: &str, limit: usize, offset: usize) -> String {
    // 先获取总数
    let total: i64 = database::get(
        "SELECT COUNT(*) as cnt FROM source_data WHERE (source_type=?1 OR ?1='') AND date(updated_at)>=?2 AND date(updated_at)<=?3",
        &[json!(obj_type), json!(start), json!(end)],
    ).ok().flatten().and_then(|r| r["cnt"].as_i64()).unwrap_or(0);
    
    let rows = database::all(
        "SELECT source_type, source_id, title, substr(content,1,100) as preview, updated_at FROM source_data
         WHERE (source_type=?1 OR ?1='')
           AND date(updated_at) >= ?2 AND date(updated_at) <= ?3
         ORDER BY updated_at DESC LIMIT ?4 OFFSET ?5",
        &[json!(obj_type), json!(start), json!(end), json!(limit), json!(offset)],
    ).unwrap_or_default();
    if rows.is_empty() { return if offset > 0 { format!("已无更多对象（共 {} 条）。", total) } else { format!("时间范围内无{}对象。", if obj_type.is_empty() { "任何" } else { obj_type }) }; }
    let mut out = String::new();
    // 添加分页信息头
    let end_pos = (offset + rows.len()).min(total as usize);
    out.push_str(&format!("┌─ 共 {} 条，显示 {}-{} ─┐\n\n", total, offset + 1, end_pos));
    let expand = |sid: &str| -> std::collections::HashSet<String> {
        let n = crate::services::synapse::expand_object_links(&[sid.to_string()], 10);
        n.into_iter().filter(|(_,w)| *w > 0.0).map(|(id,_)| id).collect()
    };
    let mut prev_neighbors: Option<std::collections::HashSet<String>> = None;
    let mut prev_ts: Option<String> = None;
    for r in &rows {
        let st = r["source_type"].as_str().unwrap_or("");
        let sid = r["source_id"].as_str().unwrap_or("");
        let name = r["title"].as_str().unwrap_or("");
        let preview = r["preview"].as_str().unwrap_or("");
        let ts = r["updated_at"].as_str().unwrap_or("");
        let seed = format!("{}-{}", st, sid);
        let current_neighbors = expand(&seed);
        
        // ★ Boundary scoring vs previous row
        if let (Some(ref prev_n), Some(ref p_ts)) = (prev_neighbors, prev_ts) {
            let d_time = (chrono::NaiveDateTime::parse_from_str(ts, "%Y-%m-%d %H:%M:%S").ok()
                .zip(chrono::NaiveDateTime::parse_from_str(p_ts, "%Y-%m-%d %H:%M:%S").ok())
                .map(|(t,p)| (t-p).num_minutes().max(0) as f64)).unwrap_or(999.0);
            let time_b = 1.0 - (-0.03 * d_time).exp();
            let shared = prev_n.intersection(&current_neighbors).count() as f64;
            let union = prev_n.union(&current_neighbors).count().max(1) as f64;
            let jaccard = shared / union;
            let space_b = 1.0 - jaccard;
            let boundary = 0.5 * time_b + 0.5 * space_b;
            eprintln!("[Boundary] Δt={}m timeB={:.2} spaceB={:.2} → score={:.2}", d_time as i64, time_b, space_b, boundary);
            if boundary > 0.45 {
                out.push_str(&format!("━━━━━ 💥 边界突变 (Boundary: {:.2}) ── Δt={}m ──\n\n", boundary, d_time as i64));
            }
        }
        
        let rel = relative_time(ts);
        // TODO 类型：额外显示优先级、截止日期、完成状态、todo_id
        if st == "TODO" {
            if let Ok(Some(todo)) = database::get("SELECT t.done, t.priority, t.due_date, t.repeat, p.name as project_name FROM todos t LEFT JOIN todo_project_mapping tpm ON t.id=tpm.todo_id LEFT JOIN projects p ON p.id=tpm.project_id WHERE t.id=?1", &[json!(sid)]) {
                let done = todo["done"].as_i64().unwrap_or(0);
                let pri = todo["priority"].as_str().unwrap_or("");
                let due = todo["due_date"].as_str().unwrap_or("");
                let repeat = todo["repeat"].as_str().unwrap_or("");
                let proj = todo["project_name"].as_str().unwrap_or("");
                let status_str = if done != 0 { "✅已完成" } else { "⬜未完成" };
                let mut extra = format!("  状态: {} | todo_id: {}", status_str, sid);
                if !pri.is_empty() { extra.push_str(&format!(" | 优先级: {}", pri)); }
                if !due.is_empty() { extra.push_str(&format!(" | 截止: {}", due)); }
                if !repeat.is_empty() { extra.push_str(&format!(" | 循环: {}", repeat)); }
                if !proj.is_empty() { extra.push_str(&format!(" | 项目: {}", proj)); }
                out.push_str(&format!("[{}] {} | source_id: {}-{}\n  {} | {}\n{}\n\n", st, name, st, sid, rel, preview, extra));
            } else {
                out.push_str(&format!("[{}] {} | source_id: {}-{}\n  {} | {}\n\n", st, name, st, sid, rel, preview));
            }
        } else {
            out.push_str(&format!("[{}] {} | source_id: {}-{}\n  {} | {}", st, name, st, sid, rel, preview));
        }
        if !current_neighbors.is_empty() {
            let mut links_by_type: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
            for nid in &current_neighbors {
                if let Some(dash) = nid.find('-') {
                    let nt = &nid[..dash]; let ni = &nid[dash+1..];
                    links_by_type.entry(nt.to_string()).or_default().push(ni[..ni.len().min(8)].to_string());
                }
            }
            let link_str: Vec<String> = links_by_type.iter().map(|(t, ids)| format!("{}: {}", t, ids.join(", "))).collect();
            if !link_str.is_empty() { out.push_str(&format!("\n  🔗 {}", link_str.join(" | "))); }
        }
        out.push_str("\n\n");
        prev_neighbors = Some(current_neighbors);
        prev_ts = Some(ts.to_string());
    }
    // 添加分页提示
    if end_pos < total as usize {
        out.push_str(&format!("└─ 还有 {} 条未显示，传 offset={} 继续查看 ─┘", total as usize - end_pos, end_pos));
    }
    out
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
        let like = format!("%{}%", q);
        if let Ok(rows) = database::all(
            "SELECT source_type, source_id, title, substr(content,1,100) as preview, updated_at FROM source_data WHERE (title LIKE ?1 OR content LIKE ?1) AND (source_type=?2 OR ?2='') ORDER BY updated_at DESC",
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
    if matched.is_empty() { return format!("未找到匹配「{}」的对象。", q); }
    let total = matched.len();
    let start_pos = offset.min(total);
    let end_pos = (offset + limit).min(total);
    let page: Vec<&Value> = matched.iter().skip(start_pos).take(limit).collect();
    let mut out = format!("┌─ 共 {} 条，显示 {}-{} ─┐\n\n", total, start_pos + 1, end_pos);
    for r in &page {
        let st = r["st"].as_str().unwrap_or("");
        let sid = r["sid"].as_str().unwrap_or("");
        let name = r["name"].as_str().unwrap_or("");
        let preview = r["preview"].as_str().unwrap_or("").chars().take(100).collect::<String>();
        let ts = r["ts"].as_str().unwrap_or("");
        let rel = relative_time(ts);
        // TODO 类型：额外显示优先级、截止日期、完成状态、todo_id
        if st == "TODO" {
            if let Ok(Some(todo)) = database::get("SELECT t.done, t.priority, t.due_date, t.repeat, p.name as project_name FROM todos t LEFT JOIN todo_project_mapping tpm ON t.id=tpm.todo_id LEFT JOIN projects p ON p.id=tpm.project_id WHERE t.id=?1", &[json!(sid)]) {
                let done = todo["done"].as_i64().unwrap_or(0);
                let pri = todo["priority"].as_str().unwrap_or("");
                let due = todo["due_date"].as_str().unwrap_or("");
                let repeat = todo["repeat"].as_str().unwrap_or("");
                let proj = todo["project_name"].as_str().unwrap_or("");
                let status_str = if done != 0 { "✅已完成" } else { "⬜未完成" };
                let mut extra = format!("  状态: {} | todo_id: {}", status_str, sid);
                if !pri.is_empty() { extra.push_str(&format!(" | 优先级: {}", pri)); }
                if !due.is_empty() { extra.push_str(&format!(" | 截止: {}", due)); }
                if !repeat.is_empty() { extra.push_str(&format!(" | 循环: {}", repeat)); }
                if !proj.is_empty() { extra.push_str(&format!(" | 项目: {}", proj)); }
                out.push_str(&format!("[{}] {} | source_id: {}-{}\n  {} | {}\n{}\n\n", st, name, st, sid, rel, preview, extra));
            } else {
                out.push_str(&format!("[{}] {} | source_id: {}-{}\n  {} | {}\n\n", st, name, st, sid, rel, preview));
            }
        } else {
            out.push_str(&format!("[{}] {} | source_id: {}-{}\n  {} | {}", st, name, st, sid, rel, preview));
        }
    }
    if end_pos < total {
        out.push_str(&format!("└─ 还有 {} 条未显示，传 offset={} 继续查看 ─┘", total - end_pos, end_pos));
    }
    out
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
            
            // FILE 类型：从 file_relations 获取 custom_name
            if st == "FILE" && name.is_empty() {
                if let Ok(Some(fr)) = database::get(
                    "SELECT custom_name FROM file_relations WHERE file_id=?1 ORDER BY id LIMIT 1",
                    &[json!(sid)],
                ) {
                    if let Some(cn) = fr["custom_name"].as_str().filter(|s| !s.is_empty()) {
                        name = cn.to_string();
                    }
                }
            }
            
            // 查询关联对象
            let seed = format!("{}-{}", st, sid);
            let neighbors = crate::services::synapse::expand_object_links(&[seed], 5);
            
            // 输出主对象
            out.push_str(&format!("{} [{}] {}\n", marker, st, name));
            
            // 输出关联对象
            for (nid, _) in &neighbors {
                if let Some(dash) = nid.find('-') {
                    let nst = &nid[..dash];
                    let nsid = &nid[dash+1..];
                    // 查询关联对象的名称
                    let n_name = if nst == "FILE" {
                        database::get(
                            "SELECT custom_name FROM file_relations WHERE file_id=?1 ORDER BY id LIMIT 1",
                            &[json!(nsid)],
                        ).ok().flatten().and_then(|r| r["custom_name"].as_str().filter(|s| !s.is_empty()).map(String::from))
                        .unwrap_or_else(|| "未命名文件".to_string())
                    } else {
                        database::get(
                            "SELECT title FROM source_data WHERE source_type=?1 AND source_id=?2",
                            &[json!(nst.to_uppercase()), json!(nsid)],
                        ).ok().flatten().and_then(|r| r["title"].as_str().filter(|s| !s.is_empty()).map(String::from))
                        .unwrap_or_else(|| "未知".to_string())
                    };
                    out.push_str(&format!("  └ [{}] {}\n", nst.to_uppercase(), n_name));
                }
            }
            out.push('\n');
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
        None => (chrono::Local::now().format("%Y-%m-%d").to_string(), "2099-12-31".to_string()),
    };
    if mode == "recall" {
        recall_mode(obj_type, &start, &end, limit, offset)
    } else if query.trim().is_empty() {
        timeline_mode(obj_type, &start, &end, limit, offset)
    } else {
        keyword_object_mode(query, obj_type, &start, &end, limit, offset)
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
                            "ext_tool_object" => args.get("query").and_then(|q| q.as_str()).map(String::from),
                            "ext_tool_traverse" => args.get("chunk_id").and_then(|c| c.as_str()).map(|c| format!("chunk {}", &c[..c.len().min(8)])),
                            _ => None,
                        };
                        let _ = h.emit("agent:status", json!({"stage":"tool_start","tool":tc.function.name,"target":target.unwrap_or_default()}));
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
                                                    effects.push(SessionEffect::UpdateObject { obj_type: ot.to_lowercase(), obj_id: oi.to_string(), name });
                                                    effects.push(SessionEffect::UpdateObjectScore { obj_type: ot.to_lowercase(), obj_id: oi.to_string(), score_delta: 0.9 });
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
                                                    effects.push(SessionEffect::UpdateObject { obj_type: ot.to_lowercase(), obj_id: oi.to_string(), name });
                                                    effects.push(SessionEffect::UpdateObjectScore { obj_type: ot.to_lowercase(), obj_id: oi.to_string(), score_delta: 0.9 });
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
                        if let Ok(result) = handle.await {
                            results_mutex.lock().unwrap().push(result);
                        }
                    }
                }
                let async_results: Vec<(usize, String, Vec<SessionEffect>)> = {
                    let guard = results_mutex.lock().unwrap();
                    guard.clone()
                };

                // 顺序执行 sync 工具
                let mut sync_results: Vec<(usize, String, Vec<SessionEffect>)> = Vec::new();
                for (idx, name, args_str) in &sync_tasks {
                    let a: Value = serde_json::from_str(args_str).unwrap_or(json!({}));
                    let (result, effects) = match name.as_str() {
                        "ext_tool_object" => {
                            let res = if a.get("id").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).is_some() {
                                fetch_parent_chunks(a["id"].as_str().unwrap_or(""))
                            } else {
                                let q = a["query"].as_str().unwrap_or("");
                                let t = a["type"].as_str().unwrap_or("");
                                let s = a["start_time"].as_str().map(resolve_time);
                                let e = a["end_time"].as_str().map(resolve_time);
                                let tr = s.and_then(|s| e.map(|e| (s, e)));
                                let limit = a["limit"].as_u64().unwrap_or(50) as usize;
                                let offset = a["offset"].as_u64().unwrap_or(0) as usize;
                                let mode = a["mode"].as_str().unwrap_or("search");
                                execute_object_get(q, t, tr, limit, offset, mode)
                            };
                            let mut effects = Vec::new();
                            for line in res.lines() {
                                if let Some(si) = line.split("source_id: ").nth(1) {
                                    let sid = si.trim();
                                    if sid.is_empty() { continue; }
                                    if let Some(dash) = sid.find('-') {
                                        let (ot, oi) = sid.split_at(dash);
                                        let oi = &oi[1..];
                                        let name = database::get("SELECT title FROM source_data WHERE source_type=?1 AND source_id=?2", &[json!(ot), json!(oi)]).ok().flatten().and_then(|r| r["title"].as_str().map(String::from)).unwrap_or_default();
                                        if !ot.is_empty() && !oi.is_empty() && !name.is_empty() {
                                            effects.push(SessionEffect::UpdateObject { obj_type: ot.to_lowercase(), obj_id: oi.to_string(), name });
                                        }
                                    }
                                }
                            }
                            (res, effects)
                        }
                        "ext_tool_traverse" => {
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
                        "ext_tool_manage_memory" => {
                            let res = ext_tool_manage_memory(
                                a["action"].as_str().unwrap_or(""),
                                a["title"].as_str().unwrap_or(""),
                                a["content"].as_str().unwrap_or(""),
                                a["memory_id"].as_str().unwrap_or(""),
                                a["replace_ids"].as_str().unwrap_or(""),
                            );
                            (res, vec![])
                        }
                        "ext_tool_record_learning" => {
                            let res = ext_tool_record_learning(
                                a["question"].as_str().unwrap_or(""),
                                a["provider"].as_str().unwrap_or(""),
                                a["answer"].as_str().unwrap_or(""),
                            );
                            (res, vec![])
                        }
                        "ext_tool_act" => {
                            let res = execute_act(
                                a["action"].as_str().unwrap_or(""),
                                a["project_id"].as_str().unwrap_or(""),
                                a["new_status"].as_str().unwrap_or(""),
                            );
                            (res, vec![])
                        }
                        "ext_tool_manage_policy" => {
                            let id = a["id"].as_str().map(|s| s.to_string());
                            let content = a["content"].as_str().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
                            let res = match (id, content) {
                                (Some(ref pid), Some(ref c)) => {
                                    let r = database::run("UPDATE source_data SET content=?1, updated_at=datetime('now','localtime') WHERE source_type='POLICY' AND source_id=?2", &[json!(c), json!(pid)]);
                                    match r { Ok(_) => format!("已更新策略：{}", c), Err(e) => format!("更新失败：{}", e) }
                                }
                                (Some(ref pid), None) => {
                                    let r = database::run("DELETE FROM source_data WHERE source_type='POLICY' AND source_id=?1", &[json!(pid)]);
                                    match r { Ok(_) => "已删除策略".to_string(), Err(e) => format!("删除失败：{}", e) }
                                }
                                (None, Some(ref c)) => {
                                    let new_id = format!("policy-{}", uuid::Uuid::new_v4());
                                    let title: String = c.chars().take(50).collect();
                                    let r = database::run("INSERT INTO source_data(id,source_type,source_id,title,content,updated_at) VALUES(?1,'POLICY',?1,?2,?3,datetime('now','localtime'))", &[json!(&new_id), json!(&title), json!(c)]);
                                    match r { Ok(_) => format!("已记入宪法：{}", c), Err(e) => format!("记录失败：{}", e) }
                                }
                                _ => "参数不完整：需要content或id".to_string(),
                            };
                            (res, vec![])
                        }
                        "ext_tool_manage_project" => {
                            let res = ext_tool_manage_project(
                                a["action"].as_str().unwrap_or(""),
                                a["project_id"].as_str().unwrap_or(""),
                                a["name"].as_str().unwrap_or(""),
                                a["customer"].as_str().unwrap_or(""),
                                a["customer_contact"].as_str().unwrap_or(""),
                                a["customer_phone"].as_str().unwrap_or(""),
                                a["owner"].as_str().unwrap_or(""),
                                a["budget"].as_f64().unwrap_or(0.0),
                                a["description"].as_str().unwrap_or(""),
                                a["bid_time"].as_str().unwrap_or(""),
                                a["status"].as_str().unwrap_or(""),
                                a["channel_id"].as_str().unwrap_or(""),
                                a["vendor_id"].as_str().unwrap_or(""),
                                a["contact_id"].as_str().unwrap_or(""),
                                a["parent_id"].as_str().unwrap_or(""),
                                a["parent_type"].as_str().unwrap_or(""),
                                a["phone"].as_str().unwrap_or(""),
                                a["company"].as_str().unwrap_or(""),
                                a["email"].as_str().unwrap_or(""),
                                a["remark"].as_str().unwrap_or(""),
                            );
                            (res, vec![])
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
                    // ★ 记录本轮展示的 MEMORY source_id（跨轮次去重）
                    for line in result.lines() {
                        if let Some(sid) = line.split("source_id: MEMORY-").nth(1) {
                            let sid = sid.trim().split(|c: char| c == '|' || c.is_whitespace()).next().unwrap_or("");
                            if !sid.is_empty() {
                                session.shown_memory_ids.insert(format!("MEMORY-{}", sid));
                            }
                        }
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
                        } else if step_desc == "ext_tool_traverse" {
                            if let Some(cid) = args.get("chunk_id").and_then(|v| v.as_str()) {
                                step_desc = format!("traverse({}…)", &cid[..cid.len().min(8)]);
                            }
                        } else if step_desc == "ext_tool_act" {
                            if let Some(a) = args.get("action").and_then(|v| v.as_str()) {
                                step_desc = format!("act({})", a);
                            }
                        }
                    }
                    session.tool_history.push(step_desc);
                    for line in result_str.lines() {
                        session.update_from_line(line);
                    }
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
                    let ans = call_llm_without_tools(&messages, &app_handle).await.unwrap_or_else(|e| format!("总结失败: {e}"));
                    let ws = session.generate_scratchpad();
                    if let Some(ref h) = app_handle { let _ = h.emit("agent:status", json!({"stage":"answer"})); let _ = h.emit("agent:final-answer", json!({"answer": ans})); }
                    record_experience(&query, &session, &tool_log, &ans);
                    return Ok(json!({"success":true,"answer":ans,"workspace":ws,"iterations":iters,"circuit_breaker":true,"promptTokens":pt,"completionTokens":ct,"tool_log":tool_log}));
                }

                // ★ 扣减预算（工具执行后，下一轮生效）
                budget -= search_cost;
                search_cost = ((search_cost as f64 * 1.2) as i32).max(1);
            }
            _ => { let ws = session.generate_scratchpad(); return Ok(json!({"success":true,"answer":format!("根据当前信息：\n\n{}",ac),"workspace":ws,"iterations":iters,"promptTokens":pt,"completionTokens":ct,"tool_log":tool_log})); }
        }
    }
}
