//! Native Tool Calling Agent — ReAct loop + symmetric retrieval + structured context lookup

use crate::database;
use crate::services::search_assets;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, RwLock};
use std::time::Instant;
use tauri::Emitter;
use futures::StreamExt;

const API_TIMEOUT_SECS: u64 = 30;

// ★ Agent 停止信号
static AGENT_CANCEL: AtomicBool = AtomicBool::new(false);

/// 取消当前 Agent 循环
pub fn cancel_agent() {
    AGENT_CANCEL.store(true, Ordering::Release);
}
const AGENT_SYSTEM_PROMPT: &str = r#"
## 检索与记忆（必须严格遵守）

⚠️ 当用户提问时，按以下流程处理：

1. **先检索记忆**：调用 ext_tool_memory(action=search)
   - 记忆里有用户的历史讨论、偏好、决策、经验
   - 当记忆中有足够的信息回答时，直接使用，不用再检索知识库。
   - 如果用户明确要求检索知识库（如"帮我查一下知识库里..."、"搜索知识库"），跳过此步

2. **再检索知识库**：调用 ext_tool_search
   - 知识库是原始文档、文件、笔记等

## ⚠️ 预算管理（必须遵守）

每次工具调用都会消耗预算：
- 初始消耗：8分
- 每次调用后消耗增长20%
- 预算耗尽时将强制停止检索并回答

**必须遵守的规则**：
1. 优先使用记忆（ext_tool_memory），减少知识库检索
2. 避免重复检索相同内容
3. 信息足够时立即停止检索
4. 预算不足时直接总结已获取的信息
5. 单次回答不超过3次工具调用

## ⚠️ 回答前强制检查（必须执行）

在组织最终回答之前，必须先执行以下检查：

**本次对话中是否有新事实需要记录？**

新事实包括：
- 用户主动告知的信息（需求、偏好、决策、确认）
- 从知识库检索到的、之前未记录过的关键事实
- 本次对话中确认的信息

有 → 调用 ext_tool_memory(action=create) 记录后再回答
无 → 直接回答

格式：[时间] YYYY-MM-DD / [问题] 用户问了什么 / [查阅] 检索了哪些来源 / [结论] 要点总结

涉及价格、状态、截止日期等易变信息时，提醒用户核实。
使用记忆回答时，注明来源（如"根据之前的讨论"）。

未执行此步骤直接输出回答，会导致用户的信息未被保存，下次对话无法回忆。

## 当前日期
{{today}}

## 用户策略
{{user_policies}}

## 当前会话工作记忆
{{session_scratchpad}}

## 你的身份
你是用户的长期思维伙伴。
记忆是你提炼的事实，知识库是原始文档。
你的职责不是查找信息，而是帮助用户理解问题、形成判断、推进决策。

## 冲突裁决
记忆与知识库冲突时，以时间更近的来源为准。

## 回答目标
帮助用户解决问题，而不是复述记忆。
最终回答应围绕用户的问题组织，而不是围绕查到的内容组织。

## 工具
- ext_tool_object: 搜索知识库中的对象。支持按关键词过滤、时间范围查看、按ID精确获取。时间回忆类问题优先使用此工具。
- ext_tool_search: 从知识库中搜索文档内容、笔记、技术资料等，支持时间范围过滤。
- ext_tool_traverse: 接收父块ID（chunk_id），用于翻页查看一个对象中的不同父块。chunk_id 是8位短ID（如 47beae9a），来自 search() 结果中的「ChunkID」字段。
- ext_tool_query: 结构化查询业务数据，支持按字段过滤、排序、分页。
- ext_tool_memory: 管理记忆。action=create(创建)，action=delete(删除)，action=search(搜索)。
- ext_tool_manage_todo: 管理待办。
- ext_tool_manage_project: 管理项目。
- ext_tool_manage_partner: 管理合作伙伴。
- ext_tool_manage_contact: 管理联系人。
- ext_tool_manage_policy: 管理用户规则。
- ext_tool_manage_reminder: 管理提醒。

⚠️ 如果 ext_tool_search 已经返回了某对象的内容，禁止再用其他工具重复获取。

## 何时停止检索
当信息足够回答时，立即停止检索。禁止在信息充足时继续搜索。

## 回答约束
- 思考过程禁止出现在最终回答中
- 禁止提及工具名称
- 来自记忆：标注「根据记忆」
- 来自知识库：标注「根据资料」
- 记忆与知识冲突：告知用户存在冲突，请用户确认

## 工具调用格式
使用标准的 tool_calls 格式调用工具。禁止在 content 中输出工具调用。

## 引用
- 日期范围用「至」代替「~」
- 回答中使用口语化表达
- 最终答案末尾用 <refs> 标签列出引用对象ID
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
    #[allow(dead_code)]
    intent: String,
    open_questions: Vec<String>,
    focus_objects: Vec<String>,  // 当前焦点对象 ID（由 Goal 决定，不是 score）
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
    completeness: f64,  // 信息充分度 (0.0 ~ 1.0)
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
            current_goal: String::new(),
            access_counter: 0,
            search_keywords: Vec::new(),
            tool_history: Vec::new(),
            completeness: 0.0,
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
                        "TODO" => "todo",
                        "CONTACT" => "contact",
                        _ => parts[0],
                    };
                    let name = tl.split("| source_id: ").next()
                        .and_then(|s| s.splitn(2, ']').nth(1))
                        .map(|s| s.trim().trim_end_matches('.'))
                        .unwrap_or("");
                    self.update(obj_type, parts[1], name);
                }
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

    /// 从用户目标中提取需要的信息类型
    fn extract_required_types(goal: &str) -> Vec<String> {
        let mut types = Vec::new();
        let g = goal.to_lowercase();
        if g.contains("客户") || g.contains("customer") { types.push("CUSTOMER".to_string()); }
        if g.contains("渠道") || g.contains("channel") { types.push("CHANNEL".to_string()); }
        if g.contains("厂商") || g.contains("供应商") || g.contains("vendor") { types.push("VENDOR".to_string()); }
        if g.contains("联系人") || g.contains("contact") { types.push("CONTACT".to_string()); }
        if g.contains("产品") || g.contains("product") { types.push("PRODUCT".to_string()); }
        if g.contains("文件") || g.contains("file") { types.push("FILE".to_string()); }
        if g.contains("笔记") || g.contains("note") { types.push("NOTE".to_string()); }
        if g.contains("待办") || g.contains("todo") { types.push("TODO".to_string()); }
        if types.is_empty() { types.push("PROJECT".to_string()); }
        types
    }

    /// 计算信息充分度 (0.0 ~ 1.0)
    fn compute_completeness(&self) -> f64 {
        if self.current_goal.is_empty() { return 0.0; }
        let required_types = Self::extract_required_types(&self.current_goal);
        let covered_types: std::collections::HashSet<&str> = self.active_objects.iter()
            .map(|o| o.obj_type.as_str()).collect();
        let type_coverage = required_types.iter()
            .filter(|t| covered_types.contains(t.as_str()))
            .count() as f64 / required_types.len().max(1) as f64;
        let observation_score = if self.observations.is_empty() { 0.0 } else {
            (self.observations.len() as f64).min(5.0) / 5.0
        };
        (type_coverage * 0.6 + observation_score * 0.4).clamp(0.0, 1.0)
    }

    /// 生成注入 Prompt 的工作记忆文本，受 Token 预算约束
    fn generate_scratchpad(&mut self) -> String {
        // ★ P1: 下一轮扩散 — 每轮开始时从 object_links 注入关联对象
        let mut sorted: Vec<&ActiveObject> = self.active_objects.iter().collect();
        sorted.sort_by(|a,b|b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        let top_oids: Vec<String> = sorted.iter().take(5).map(|o| o.obj_id.to_string()).collect();
        let neighbors = crate::services::synapse::expand_object_links(&top_oids, 15);
        if neighbors.len() > 0 { log::info!("[LLM检索] expand(next_round): {} neighbors", neighbors.len()); }
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

        // 1. Goal（当前目标）
        if !self.current_goal.is_empty() {
            lines.push(String::new());
            lines.push("## Goal（当前目标）".to_string());
            lines.push(self.current_goal.clone());
        }

        // 2. Attention（注意力排序，按 score 降序）
        if !self.active_objects.is_empty() {
            lines.push(String::new());
            lines.push("## Attention（注意力排序）".to_string());
            let mut sorted: Vec<&ActiveObject> = self.active_objects.iter().collect();
            sorted.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
            for (_i, o) in sorted.iter().take(10).enumerate() {
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

        // 4. Working Memory（推理状态，不是数据库字段）
        let has_wm = !self.observations.is_empty() || !self.hypotheses.is_empty() || !self.focus.open_questions.is_empty();
        if has_wm {
            lines.push(String::new());
            lines.push("## Working Memory（推理状态）".to_string());
            // 已确认的事实（来自 observations）
            if !self.observations.is_empty() {
                lines.push("已确认：".to_string());
                for obs in self.observations.iter().take(5) {
                    let preview: String = obs.chars().take(150).collect();
                    lines.push(format!("  - {}", preview));
                }
            }
            // 假设（来自 hypotheses）
            if !self.hypotheses.is_empty() {
                lines.push("假设：".to_string());
                for h in self.hypotheses.iter().take(3) {
                    let preview: String = h.chars().take(100).collect();
                    lines.push(format!("  - {}", preview));
                }
            }
            // 未解决的问题
            if !self.focus.open_questions.is_empty() {
                lines.push("仍待确认：".to_string());
                for q in self.focus.open_questions.iter() {
                    lines.push(format!("  □ {}", q));
                }
            }
        }

        // 5. Completeness（信息充分度）
        if !self.current_goal.is_empty() {
            self.completeness = self.compute_completeness();
            let required_types = Self::extract_required_types(&self.current_goal);
            let covered_types: std::collections::HashSet<&str> = self.active_objects.iter()
                .map(|o| o.obj_type.as_str()).collect();
            let covered_str: Vec<String> = required_types.iter()
                .filter(|t| covered_types.contains(t.as_str()))
                .map(|t| format!("{} ✓", t))
                .collect();
            let uncovered_str: Vec<String> = required_types.iter()
                .filter(|t| !covered_types.contains(t.as_str()))
                .map(|t| format!("{} ✗", t))
                .collect();
            let status = if self.completeness >= 0.8 { "✅ 充足" } 
                else if self.completeness >= 0.5 { "⚠️ 部分" } 
                else { "❌ 不足" };
            lines.push(String::new());
            lines.push("## Completeness（信息充分度）".to_string());
            lines.push(format!("目标：{}", self.current_goal));
            if !covered_str.is_empty() { lines.push(format!("已覆盖：{}", covered_str.join(", "))); }
            if !uncovered_str.is_empty() { lines.push(format!("未覆盖：{}", uncovered_str.join(", "))); }
            lines.push(format!("充分度：{:.0}% {}", self.completeness * 100.0, status));
        }

        // 6. Execution Log（从工具调用历史生成）
        if !self.tool_history.is_empty() {
            lines.push(String::new());
            lines.push("## Execution Log".to_string());
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

        // 6. Knowledge Progress（source 覆盖率统计）
        if !self.visited_sources.is_empty() {
            lines.push(String::new());
            lines.push("## Knowledge Progress".to_string());
            for src_key in self.visited_sources.iter() {
                let parts: Vec<&str> = src_key.splitn(2, '-').collect();
                if parts.len() != 2 { continue; }
                let (st, sid) = (parts[0], parts[1]);
                let name = crate::database::get(
                    "SELECT title FROM source_data WHERE source_type=?1 AND source_id=?2",
                    &[json!(st), json!(sid)],
                ).ok().flatten().and_then(|r| r["title"].as_str().map(String::from)).unwrap_or_default();
                if name.is_empty() { continue; }
                let total: i64 = crate::database::get(
                    "SELECT COUNT(*) as cnt FROM knowledge_chunks kc JOIN source_data sd ON sd.id=kc.source_data_id WHERE sd.source_type=?1 AND sd.source_id=?2 AND kc.chunk_type='child'",
                    &[json!(st), json!(sid)],
                ).ok().flatten().and_then(|r| r["cnt"].as_i64()).unwrap_or(0);
                if total == 0 { continue; }
                let visited_count: i64 = crate::database::all(
                    "SELECT kc.chunk_id FROM knowledge_chunks kc JOIN source_data sd ON sd.id=kc.source_data_id WHERE sd.source_type=?1 AND sd.source_id=?2 AND kc.chunk_type='child'",
                    &[json!(st), json!(sid)],
                ).unwrap_or_default().iter().filter(|r| {
                    let cid = r["chunk_id"].as_str().unwrap_or("");
                    let short: String = cid.chars().take(8).collect();
                    self.visited_chunks.contains(&short)
                }).count() as i64;
                let pct = (visited_count as f64 / total as f64 * 100.0) as i64;
                let status = if visited_count >= total { "✔" } else { "…​" };
                lines.push(format!("  {} {}：{}/{} 块 ({}%)", status, name, visited_count, total, pct));
            }
            // 显示已搜索关键词
            if !self.search_keywords.is_empty() {
                let all_kw: Vec<String> = self.search_keywords.iter().flat_map(|v| v.iter().cloned()).collect();
                let unique_kw: Vec<String> = { let s: std::collections::HashSet<String> = all_kw.into_iter().collect(); s.into_iter().collect() };
                lines.push(format!("  🔍 已搜索：{}", unique_kw.join("、")));
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
    #[allow(dead_code)]
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
                    // Residual: 旧分衰减 + 新分叠加
                    existing.score = existing.score * 0.95 + score_delta;
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
            "file" => Some("FILE"), "note" => Some("NOTE"), "todo" => Some("TODO"),
            "contact" => Some("CONTACT"), "chat" => Some("CHAT"), _ => None }
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
    if scope_ids.is_empty() { log::info!("[Scope] scope_ids 为空，scope=None"); return None; }
    let s = resolve_scope_to_rowids(&scope_ids);
    log::info!("[Scope] scope_ids={:?}, resolved_rowids={}", scope_ids, s.len());
    if s.is_empty() { log::info!("[Scope] scope_rowids 解析为空，scope=None"); None } else { Some(s) }
}

fn execute_structured_query(params: &Value) -> String {
    let entity = params["entity"].as_str().unwrap_or("");
    let (table, fields): (&str, &[&str]) = match entity {
        "project" => ("projects", &["id","name","description","status","owner","budget","bid_time","created_at","updated_at"]),
        "product" => ("products", &["id","name","series_name","description","price","standard_specs","full_specs","sort_order","created_at","updated_at"]),
        "partner" => ("partners", &["id","type","name","phone","address","remark","contact_person","contact_phone","created_at","updated_at"]),
        "contact" => ("partner_contacts", &["id","name","phone","email","position","remark","created_at"]),
        "todo" => ("todos", &["id","title","content","done","priority","due_date","repeat","created_at","updated_at"]),
        "note" => ("notes", &["note_id","title","content","is_daily","notebook_id","deleted_at","created_at","updated_at"]),
        _ => return format!("Unsupported entity: {}. Use: project, product, partner, contact, todo, note", entity),
    };
    let mut conditions: Vec<String> = Vec::new();
    let mut values: Vec<serde_json::Value> = Vec::new();
    if let Some(filters) = params["filters"].as_array() {
        for filter in filters {
            let field = filter["field"].as_str().unwrap_or("");
            let op = filter["op"].as_str().unwrap_or("=");
            let value = &filter["value"];
            if !fields.contains(&field) { continue; }
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
                "LIKE" => { conditions.push(format!("{} LIKE ?", field)); values.push(json!(value.as_str().unwrap_or(""))); }
                "=" | "!=" | ">" | "<" | ">=" | "<=" => { conditions.push(format!("{} {} ?", field, op)); values.push(value.clone()); }
                _ => {}
            }
        }
    }
    if entity == "note" && !conditions.iter().any(|c| c.contains("deleted_at")) {
        conditions.push("deleted_at IS NULL".to_string());
    }
    let where_clause = if conditions.is_empty() { String::new() } else { format!(" WHERE {}", conditions.join(" AND ")) };
    let order = if let Some(sort) = params["sort"].as_object() {
        let field = sort["field"].as_str().unwrap_or("created_at");
        let dir = if sort["order"].as_str().unwrap_or("desc") == "asc" { "ASC" } else { "DESC" };
        if fields.contains(&field) { format!(" ORDER BY {} {}", field, dir) } else { String::new() }
    } else { " ORDER BY created_at DESC".to_string() };
    let limit = params["limit"].as_u64().unwrap_or(20).min(100);
    let offset = params["offset"].as_u64().unwrap_or(0);
    let sql = format!("SELECT * FROM {}{}{} LIMIT {} OFFSET {}", table, where_clause, order, limit, offset);
    match crate::database::all(&sql, &values) {
        Ok(rows) => {
            if rows.is_empty() { format!("No results. Entity: {}, filters: {:?}", entity, params["filters"]) }
            else { format!("Found {} records:\n{}", rows.len(), serde_json::to_string_pretty(&rows).unwrap_or_default()) }
        }
        Err(e) => format!("Query failed: {}", e)
    }
}

fn build_tools_definitions() -> Value {
    json!([
        {"type":"function","function":{"name":"ext_tool_search","description":"在知识库中检索证据。支持时间范围过滤。","parameters":{"type":"object","properties":{"keywords":{"type":"array","items":{"type":"string"},"description":"1~4 个核心关键词"},"start_time":{"type":"string","description":"开始时间，格式 2026-06-25 或 昨天"},"end_time":{"type":"string","description":"结束时间，格式 2026-06-25 或 昨天"}},"required":["keywords"]}}},
        {"type":"function","function":{"name":"ext_tool_object","description":"搜索知识库中的对象。支持按ID精确获取、按关键词搜索、按时间范围浏览。IDs格式为TYPE-UUID（如PROJECT-xxx），也支持裸UUID。","parameters":{"type":"object","properties":{"id":{"type":"string","description":"单个对象ID（如PROJECT-abc123），也支持裸UUID"},"ids":{"type":"array","items":{"type":"string"},"description":"批量对象ID列表（如[\"PROJECT-xxx\",\"PROJECT-yyy\"]），也支持裸UUID，最多30个"},"query":{"type":"string","description":"关键词过滤"},"type":{"type":"string","description":"CHAT/NOTE/PRODUCT/PROJECT/FILE/TODO/CHANNEL/VENDOR/CONTACT"},"start_time":{"type":"string","description":"开始时间"},"end_time":{"type":"string","description":"结束时间"},"limit":{"type":"integer","description":"每页条数，默认50，最大100"},"offset":{"type":"integer","description":"分页偏移，默认0"}},"required":[]}}},
        {"type":"function","function":{"name":"ext_tool_traverse","description":"沿关联图谱漫游。⚠️ chunk_id 是8位短ID（如 47beae9a），来自 search() 结果中的「ChunkID」字段。绝对不是 source_id（如 FILE-xxx 或 PRODUCT-xxx）。","parameters":{"type":"object","properties":{"chunk_id":{"type":"string","description":"8位短ID（如 47beae9a），必须来自 search() 结果的 ChunkID 字段，不是 source_id"},"edge_type":{"type":"string","enum":["next","prev","related","references"],"description":"next=向后翻页；prev=向前翻页；related=同项目/同产品/同实体；references=引用实体"},"target_name":{"type":"string","description":"目标名称（related/references 时用）"},"radius":{"type":"integer","description":"步长，默认 10，最大 20"}},"required":["chunk_id","edge_type"]}}},
        {"type":"function","function":{"name":"ext_tool_manage_todo","description":"管理待办。action=create(title必填,priority从用户原话提取,due_date从用户原话提取,repeat可选daily/weekly/monthly,project_id关联项目), action=update, action=delete","parameters":{"type":"object","properties":{"action":{"type":"string","enum":["create","update","delete"]},"title":{"type":"string","description":"待办标题"},"content":{"type":"string","description":"待办详情"},"priority":{"type":"string","description":"优先级: P1/P2/P3 或 高/中/低"},"due_date":{"type":"string","description":"截止日期: YYYY-MM-DD"},"repeat":{"type":"string","description":"循环: daily/weekly/monthly"},"project_id":{"type":"string","description":"关联项目ID"},"done":{"type":"boolean","description":"是否完成(update用)"},"todo_id":{"type":"string","description":"待办ID(update/delete)"}},"required":["action"]}}},
        {"type":"function","function":{"name":"ext_tool_manage_project","description":"管理项目。action: create(创建), update(更新字段), delete(删除及关联数据)","parameters":{"type":"object","properties":{"action":{"type":"string","enum":["create","update","delete"]},"project_id":{"type":"string","description":"项目ID(update/delete用)"},"name":{"type":"string","description":"项目名称(create用)"},"budget":{"type":"number","description":"预算金额，单位：万元"},"description":{"type":"string","description":"项目描述"},"bid_time":{"type":"string","description":"投标时间 YYYY-MM-DD"},"status":{"type":"string","description":"项目状态: Lead/SolutionDesign/Ordered/Won/Lost/Archived"}},"required":["action"]}}},
        {"type":"function","function":{"name":"ext_tool_manage_partner","description":"管理合作伙伴（渠道/厂商/客户）。action: create(创建), update(更新), delete(删除), link_project(关联到项目), unlink_project(取消关联)","parameters":{"type":"object","properties":{"action":{"type":"string","enum":["create","update","delete","link_project","unlink_project"]},"partner_id":{"type":"string","description":"合作伙伴ID(update/delete/link_project/unlink_project用)"},"type":{"type":"string","description":"类型: CHANNEL/VENDOR/CUSTOMER(create用)"},"name":{"type":"string","description":"名称(create/update用)"},"phone":{"type":"string","description":"电话"},"address":{"type":"string","description":"地址"},"remark":{"type":"string","description":"备注"},"project_id":{"type":"string","description":"项目ID(link_project/unlink_project用)"}},"required":["action"]}}},
        {"type":"function","function":{"name":"ext_tool_manage_contact","description":"管理联系人(通讯录). action: create(创建), update(更新), delete(删除), list(查看通讯录), link_project(关联到项目), unlink_project(取消关联)","parameters":{"type":"object","properties":{"action":{"type":"string","enum":["create","update","delete","list","link_project","unlink_project"]},"contact_id":{"type":"string","description":"联系人ID(update/delete/link_project/unlink_project用)"},"name":{"type":"string","description":"姓名(create/update用)"},"phone":{"type":"string","description":"电话"},"email":{"type":"string","description":"邮箱"},"position":{"type":"string","description":"职位"},"remark":{"type":"string","description":"备注"},"parent_id":{"type":"string","description":"关联的合作伙伴ID(渠道/厂商/客户), 可选"},"project_id":{"type":"string","description":"项目ID(link_project/unlink_project用)"},"type":{"type":"string","description":"类型过滤: CHANNEL/VENDOR/CUSTOMER(list用)"}},"required":["action"]}}},
        {"type":"function","function":{"name":"ext_tool_query","description":"Structured query for business data with field filtering, sorting, pagination.","parameters":{"type":"object","properties":{"entity":{"type":"string","enum":["project","product","partner","contact","todo","note"],"description":"Entity type to query"},"filters":{"type":"array","items":{"type":"object","properties":{"field":{"type":"string","description":"Field name"},"op":{"type":"string","enum":["=","!=",">","<",">=","<=","LIKE","IN","IS_NULL","IS_NOT_NULL"],"description":"Operator"},"value":{"type":["string","number","boolean","null"],"description":"Filter value"}},"required":["field","op","value"]},"description":"Filter conditions (AND)"},"sort":{"type":"object","properties":{"field":{"type":"string","description":"Sort field"},"order":{"type":"string","enum":["asc","desc"],"description":"Sort direction"}},"description":"Sort"},"limit":{"type":"integer","description":"Max results, default 20, max 100"},"offset":{"type":"integer","description":"Offset, default 0"}},"required":["entity"]}}},
        {"type":"function","function":{"name":"ext_tool_manage_reminder","description":"Manage reminders. action=ack(acknowledge, hide from future reminders), action=unack(cancel acknowledgement, show again).","parameters":{"type":"object","properties":{"action":{"type":"string","enum":["ack","unack"],"description":"ack=confirm reminder, unack=cancel confirmation"},"reminder_type":{"type":"string","enum":["todo_due","todo_today","project_stale","bid_deadline"],"description":"Type of reminder"},"entity_id":{"type":"string","description":"ID of the todo or project"}},"required":["action","reminder_type","entity_id"]}}},
        {"type":"function","function":{"name":"ext_tool_memory","description":"管理记忆. action=create(创建), action=delete(删除), action=search(搜索记忆)","parameters":{"type":"object","properties":{"action":{"type":"string","enum":["create","delete","search"],"description":"create=创建记忆, delete=删除记忆, search=搜索记忆"},"content":{"type":"string","description":"单条记忆内容（与 items 二选一）"},"items":{"type":"array","items":{"type":"string"},"description":"批量创建：多条事实内容数组，一次调用创建多条记忆"},"memory_id":{"type":"string","description":"删除时用"},"replace_ids":{"type":"string","description":"创建时逗号分隔的旧记忆id"},"keywords":{"type":"array","items":{"type":"string"},"description":"搜索关键词列表（如 [\"S12700E\",\"交换容量\"]），多个关键词都会匹配"},"query":{"type":"string","description":"搜索关键词（单个）"},"start_time":{"type":"string","description":"开始时间"},"end_time":{"type":"string","description":"结束时间"}},"required":["action"]}}},
        {"type":"function","function":{"name":"ext_tool_manage_policy","description":"管理用户策略（宪法）。id+content=更新，id+无content=删除，无id+content=新增。查询不需要，系统自动注入prompt。","parameters":{"type":"object","properties":{"id":{"type":"string","description":"策略ID（更新/删除用）"},"content":{"type":"string","description":"策略内容（新增/更新用）"}},"required":[]}}},
    ])
}


/// Fallback: parse XML tool calls from content when model does not use function calling format
fn parse_xml_tool_calls(content: &str) -> Option<Vec<Value>> {
    // 匹配所有 DSML 变体（全角/半角/单双字符/混合/Unicode/闭合标签）
    // 支持: <｜｜DSML｜｜, <|DSML|, <‖DSML‖, <DSML>, ＜DSML＞, </｜｜DSML｜｜, 等
    let re = regex::Regex::new(r"(?i)[^a-zA-Z0-9]*DSML[^a-zA-Z0-9]*").ok()?;
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
/// ★ 单请求流式调用：一次请求同时完成
///   - content 逐 token 流式输出到前端（agent:token / agent:start-answer）
///   - tool_calls delta 按 index 累积（工具调用决策）
///   - usage 统计（stream_options.include_usage）
/// 返回 (content, reasoning_content, tool_calls, usage)
async fn call_llm_streaming(messages: &[ChatMessage], tools: Option<&Value>, app_handle: &Option<tauri::AppHandle>) -> Result<(String, Option<String>, Vec<Value>, Option<Value>), String> {
    let (url, api_key, model) = get_api_endpoint();
    let msgs: Vec<Value> = messages.iter().map(|m| {
        let mut v = serde_json::to_value(m).unwrap_or_default();
        if let Some(obj) = v.as_object_mut() {
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
        first: bool,
        count: u64,
    }
    let mut st = SseState { buf: String::new(), full: String::new(), reasoning: String::new(), tca: Vec::new(), usage: None, first: true, count: 0 };
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
        if json_str.starts_with("[DONE]") { st.buf = st.buf[after_prefix + json_end..].to_string(); return true; }
        match serde_json::from_str::<Value>(json_str) {
            Ok(chunk) => {
                st.buf = st.buf[after_prefix + json_end..].to_string();
                if chunk["usage"].is_object() { st.usage = Some(chunk["usage"].clone()); }
                if let Some(choices) = chunk["choices"].as_array() {
                    for c in choices {
                        let delta = &c["delta"];
                        // content → 逐 token 流式输出到前端
                        if let Some(tok) = delta["content"].as_str() {
                            st.full.push_str(tok);
                            st.count += 1;
                            if let Some(ref h) = app {
                                if st.first {
                                    let _ = h.emit("agent:start-answer", json!({}));
                                    st.first = false;
                                }
                                let _ = h.emit("agent:token", json!({"token": tok}));
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
        if AGENT_CANCEL.load(Ordering::Acquire) {
            log::info!("SSE stream cancelled +{}ms tokens={}", t0.elapsed().as_millis(), st.count);
            return Err("cancelled".into());
        }
    }
    // 流结束后处理剩余数据
    while try_process_one(&mut st, &app_handle) {}
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
    Ok((st.full, reasoning, tool_calls, st.usage))
}

/// 流式补生成（无工具）：用于强制总结等路径；答案逐 token 输出到前端
async fn call_llm_without_tools(messages: &[ChatMessage], app_handle: &Option<tauri::AppHandle>) -> Result<String, String> {
    let (content, _, _, _) = call_llm_streaming(messages, None, app_handle).await?;
    // 过滤 DSML 残留
    let re_dsml = regex::Regex::new(r"(?si)[^a-zA-Z0-9]*DSML[^a-zA-Z0-9]*[^\n]*").unwrap();
    Ok(re_dsml.replace_all(&content, "").to_string())
}

// ═══════════════════════════════════════════════════════════════════
// Chunk metadata enrichment
// ═══════════════════════════════════════════════════════════════════

/// 从 source_type / source_id 查询所有关联元数据，返回格式化信息块
/// 数据来源：business_relations（关联关系）+ source_data.title（名称）
pub fn build_chunk_metadata(source_type: &str, source_id: &str, with_relations: bool) -> String {
    if source_type.is_empty() || source_id.is_empty() { return String::new(); }

    // ── 来源标题（从 source_data.title）──
    let title = database::get(
        "SELECT title FROM source_data WHERE source_type=?1 AND source_id=?2",
        &[json!(source_type), json!(source_id)],
    ).ok().flatten().and_then(|r| r["title"].as_str().map(String::from)).unwrap_or_default();

    let mut lines: Vec<String> = Vec::new();
    let type_label = match source_type { "FILE" => "文件", "PRODUCT" => "产品", "PROJECT" => "项目", "NOTE" => "笔记", "TODO" => "待办", "CHANNEL" => "渠道", "VENDOR" => "供应商", "CONTACT" => "联系人", "CUSTOMER" => "客户", _ => source_type };
    let title_display = if title.is_empty() { source_id } else { title.as_str() };
    lines.push(format!("来源：{} · {} | source_id: {}-{}", type_label, title_display, source_type.to_uppercase(), source_id));

    if !with_relations { return lines.join("\n"); }

    // ── 关联查询（ext_tool_object 按需调用时才执行）──
    let rels = database::all(
        "SELECT target_type, target_id, relation_type, metadata FROM business_relations WHERE source_type=?1 AND source_id=?2",
        &[json!(source_type), json!(source_id)],
    ).unwrap_or_default();
    let reverse_rels = database::all(
        "SELECT source_type as target_type, source_id as target_id, relation_type, metadata FROM business_relations WHERE target_type=?1 AND target_id=?2",
        &[json!(source_type), json!(source_id)],
    ).unwrap_or_default();

    let mut all_rels: Vec<(String, String, String, String)> = rels.into_iter()
        .chain(reverse_rels.into_iter())
        .filter_map(|r| {
            Some((r["target_type"].as_str()?.to_string(), r["target_id"].as_str()?.to_string(), r["relation_type"].as_str().unwrap_or("").to_string(), r["metadata"].as_str().unwrap_or("{}").to_string()))
        }).collect();
    all_rels.sort_by(|a,b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    all_rels.dedup_by(|a,b| a.0 == b.0 && a.1 == b.1);

    if all_rels.is_empty() { return lines.join("\n"); }

    // 批量查名称
    let mut name_cache: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut targets_by_type: std::collections::HashMap<String, Vec<(String, String, String)>> = std::collections::HashMap::new();
    for (tt, tid, rt, _meta) in &all_rels {
        targets_by_type.entry(tt.clone()).or_default().push((tid.clone(), rt.clone(), _meta.clone()));
    }
    for (tt, targets) in &targets_by_type {
        let ids: Vec<String> = targets.iter().map(|(id,_,_)| id.clone()).collect();
        if ids.is_empty() { continue; }
        let ph: Vec<String> = ids.iter().map(|_| "?".to_string()).collect();
        let params: Vec<serde_json::Value> = ids.iter().map(|id| json!(id)).collect();
        let type_upper = tt.to_uppercase();
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

    let relation_label = |st: &str, tt: &str, rt: &str| -> String {
        match (st, tt, rt) {
            ("PROJECT", "PRODUCT", _) => "项目使用".to_string(),
            ("PROJECT", "FILE", _) => "项目文件".to_string(),
            ("PROJECT", "CHANNEL", _) => "项目渠道".to_string(),
            ("PROJECT", "VENDOR", _) => "项目厂商".to_string(),
            ("PROJECT", "CONTACT", _) => "项目联系人".to_string(),
            ("PROJECT", "CUSTOMER", _) => "项目客户".to_string(),
            ("PROJECT", "NOTE", _) => "项目笔记".to_string(),
            ("PROJECT", "TODO", _) => "项目待办".to_string(),
            ("PRODUCT", "PROJECT", _) => "用于项目".to_string(),
            ("FILE", "PROJECT", _) => "项目文件".to_string(),
            ("CHANNEL", "PROJECT", _) => "渠道归属".to_string(),
            ("VENDOR", "PROJECT", _) => "厂商归属".to_string(),
            ("CONTACT", "PROJECT", _) => "项目联系人".to_string(),
            ("CUSTOMER", "PROJECT", _) => "客户归属".to_string(),
            ("NOTE", "PROJECT", _) => "项目笔记".to_string(),
            ("TODO", "PROJECT", _) => "项目待办".to_string(),
            ("FILE", "PRODUCT", _) | ("PRODUCT", "FILE", _) => "产品资料".to_string(),
            (_, _, "belongs_to") => "归属于".to_string(),
            (_, _, "related") => "相关".to_string(),
            (_, _, "authored_by") => "创建".to_string(),
            _ => rt.to_string(),
        }
    };

    let format_targets = |tt: &str, targets: &[(String, String, String)]| -> Vec<String> {
        targets.iter().filter_map(|(tid, rt, _meta)| {
            let name = name_cache.get(&format!("{}-{}", tt, tid)).map(|s| s.as_str()).unwrap_or("");
            let type_label = match tt { "FILE" => "文件", "PRODUCT" => "产品", "PROJECT" => "项目", "NOTE" => "笔记", "TODO" => "待办", "CHANNEL" => "渠道", "VENDOR" => "供应商", "CONTACT" => "联系人", "CUSTOMER" => "客户", _ => tt };
            let display_name = format!("[{}] {}", type_label, if name.is_empty() { tid.as_str() } else { name });
            Some(format!("· {} | 关系：{} | source_id: {}-{}", display_name, relation_label(source_type, tt, rt), tt, tid))
        }).collect()
    };

    let mut rel_items: Vec<String> = Vec::new();
    for (_, type_key) in &[("关联项目", "PROJECT"), ("关联产品", "PRODUCT"), ("关联笔记", "NOTE"), ("关联待办", "TODO"), ("关联文件", "FILE"), ("关联渠道", "CHANNEL"), ("关联供应商", "VENDOR"), ("关联联系人", "CONTACT"), ("关联客户", "CUSTOMER")] {
        if type_key == &source_type { continue; }
        if let Some(targets) = targets_by_type.get(*type_key) {
            rel_items.extend(format_targets(type_key, targets));
        }
    }
    if !rel_items.is_empty() {
        lines.push(format!("关联对象：\n{}", rel_items.join("\n")));
    }

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
    let like = format!("%{}%", crate::database::escape_like(&keywords.join(" ")));

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
async fn execute_search_knowledge(keywords: &[String], scope_rowids: Option<std::collections::HashSet<i64>>, _visited_sources: &std::collections::HashSet<String>, visited_chunks: &std::collections::HashSet<String>, time_range: Option<(String, String)>, shown_memory_ids: &std::collections::HashSet<String>) -> (String, Vec<String>) {
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
        if let Ok(rows) = database::all("SELECT id,name,status FROM projects WHERE name LIKE ?1 ORDER BY LENGTH(name) ASC LIMIT 3",&[json!(&like)]) {
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
            let mut meta = build_chunk_metadata(st, sid, false);
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
            let preview: String = content.chars().take(1300).collect();
            if meta.is_empty() { search_out.push_str(&format!("\n\n--- [父块{}|RRF={}]{} [来源：知识库] ---\n{}", idx + 1, rrf_tag, boost_tag, preview)); }
            else { search_out.push_str(&format!("\n\n--- [父块{}|RRF={}]{} [来源：知识库] ---\n{}\n--------\n{}", idx + 1, rrf_tag, boost_tag, meta, preview)); }

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
    let out = search_out;
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
        // 不再输出"未访问热点"提示，避免给 LLM 造成无关噪声
        (format!("{}{}{}", obj_head, out.trim(), horizontal).trim().to_string(), found_sources)
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
            let meta = build_chunk_metadata(&st.to_lowercase(), sid, false);
            if !meta.is_empty() { out.push_str(&format!("\n\n当前块导航卡：\n{}",meta)); }
            out
        }
        "Related" => {
            let mut out = String::new();
            for &(ety, name_cn) in &[("PRODUCT","产品"), ("PROJECT","项目")] {
                let ids: Vec<String> = if target_name.trim().is_empty() {
                    // 从 business_relations 双向查询关联实体
                    let mut ids: Vec<String> = database::all("SELECT target_id FROM business_relations WHERE source_type='FILE' AND source_id=?1 AND target_type=?2 LIMIT 3",&[json!(&row_sid),json!(ety)]).unwrap_or_default().iter().filter_map(|r2|r2["target_id"].as_str().map(String::from)).collect();
                    let reverse_ids: Vec<String> = database::all("SELECT source_id FROM business_relations WHERE target_type='FILE' AND target_id=?1 AND source_type=?2 LIMIT 3",&[json!(&row_sid),json!(ety)]).unwrap_or_default().iter().filter_map(|r2|r2["source_id"].as_str().map(String::from)).collect();
                    ids.extend(reverse_ids);
                    ids.dedup();
                    ids
                } else {
                    let tbl = if ety == "PRODUCT" { "products" } else { "projects" };
                    // 从 business_relations 双向查询，再按名称过滤
                    let mut ids: Vec<String> = database::all(&format!("SELECT target_id FROM business_relations WHERE source_type='FILE' AND source_id=?1 AND target_type=?2 AND target_id IN (SELECT id FROM {} WHERE name LIKE ?3) LIMIT 3",tbl),&[json!(&row_sid),json!(ety),json!(format!("%{}%",target_name.trim()))]).unwrap_or_default().iter().filter_map(|r2|r2["target_id"].as_str().map(String::from)).collect();
                    let reverse_ids: Vec<String> = database::all(&format!("SELECT source_id FROM business_relations WHERE target_type='FILE' AND target_id=?1 AND source_type=?2 AND source_id IN (SELECT id FROM {} WHERE name LIKE ?3) LIMIT 3",tbl),&[json!(&row_sid),json!(ety),json!(format!("%{}%",target_name.trim()))]).unwrap_or_default().iter().filter_map(|r2|r2["source_id"].as_str().map(String::from)).collect();
                    ids.extend(reverse_ids);
                    ids.dedup();
                    ids
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

/// 批量 Recall：并发获取多个对象的完整信息
async fn execute_batch_recall(ids: &[String], visited_chunks: &std::collections::HashSet<String>, visited_sources: &std::collections::HashSet<String>) -> String {
    if ids.is_empty() { return "请提供至少一个对象 ID。".to_string(); }
    let limit = ids.len().min(30);
    let ids = &ids[..limit];
    log::info!("[batch_recall] 收到 {} 个ID: {:?}", ids.len(), ids.iter().take(3).collect::<Vec<_>>());
    
    // 并发获取每个对象的完整信息
    let mut handles = Vec::new();
    for id in ids.iter() {
        let id = id.clone();
        let vc = visited_chunks.clone();
        let vs = visited_sources.clone();
        handles.push(tokio::spawn(async move {
            (id.clone(), fetch_parent_chunks(&id, &[], &vc, &vs))
        }));
    }
    
    // 收集结果
    let mut results: Vec<(String, String)> = Vec::new();
    for h in handles {
        if let Ok((id, result)) = h.await {
            results.push((id, result));
        }
    }
    
    // 格式化输出
    let mut out = String::from("## 批量 Recall 结果\n\n");
    for (i, (id, result)) in results.iter().enumerate() {
        out.push_str(&format!("### {} ({}/{})\n", id, i + 1, results.len()));
        out.push_str(result);
        out.push_str("\n\n");
    }
    
    // 截断保护
    let total_len = out.len();
    if total_len > 50000 {
        let truncated: String = out.chars().take(50000).collect();
        out = format!("{}…\n\n⚠️ 输出已截断（共{}字），建议分批查询", truncated, total_len);
    }
    
    out
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
        "SELECT id FROM projects WHERE name LIKE ?1 ORDER BY LENGTH(name) ASC LIMIT 1",
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
            database::get("SELECT id,title FROM todos WHERE id=?1", &[json!(tid.trim())])
                .ok().flatten().map(|r| (r["id"].as_str().unwrap_or("").to_string(), r["title"].as_str().unwrap_or("").to_string()))
        } else if !t.trim().is_empty() {
            let like = format!("%{}%", crate::database::escape_like(t.trim()));
            database::get("SELECT id,title FROM todos WHERE title LIKE ?1 ORDER BY created_at DESC LIMIT 1", &[json!(&like)])
                .ok().flatten().map(|r| (r["id"].as_str().unwrap_or("").to_string(), r["title"].as_str().unwrap_or("").to_string()))
        } else { None }
    };
    match action {
        "create" => {
            if title.trim().is_empty() { return "创建待办需要提供 title。".into(); }
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
            let rt = tokio::runtime::Runtime::new().unwrap();
            match rt.block_on(crate::commands::todo_commands::todos_create(data)) {
                Ok(result) => {
                    let todo_id = result["todo"]["id"].as_str().unwrap_or("");
                    let todo_title = result["todo"]["title"].as_str().unwrap_or(title.trim());
                    if !todo_id.is_empty() && pid.is_some() {
                        log::info!("[待办] 关联项目: todo_id={}, project_id={:?}", todo_id, pid);
                    }
                    format!("已创建待办：{}", todo_title)
                },
                Err(e) => format!("创建待办失败: {}", e),
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
                        return "更新待办需要提供要修改的字段。".into();
                    }
                    // 使用 block_on 调用异步函数
                    let rt = tokio::runtime::Runtime::new().unwrap();
                    match rt.block_on(crate::commands::todo_commands::todos_update(fid.clone(), data)) {
                        Ok(_) => format!("已更新待办「{}」", ftitle),
                        Err(e) => format!("更新待办失败: {}", e),
                    }
                }
                None => "未找到匹配的待办。".into(),
            }
        }
        "delete" => {
            let found = find_todo(todo_id, title);
            match found {
                Some((fid, ftitle)) => {
                    // 使用 block_on 调用异步函数
                    let rt = tokio::runtime::Runtime::new().unwrap();
                    match rt.block_on(crate::commands::todo_commands::todos_delete(fid)) {
                        Ok(_) => format!("已删除待办「{}」", ftitle),
                        Err(e) => format!("删除待办失败: {}", e),
                    }
                }
                None => "未找到匹配的待办。".into(),
            }
        }
        _ => format!("未知操作「{}」。支持：create, update, delete。", action),
    }
}

fn ext_tool_memory(action: &str, content: &str, items_json: &str, memory_id: &str, _replace_ids: &str, query: &str, keywords_json: &str, start_time: &str, end_time: &str) -> String {
    match action {
        "create" => {
            // 优先取 items 数组，否则取 content
            let contents: Vec<String> = if !items_json.trim().is_empty() {
                serde_json::from_str::<Vec<String>>(items_json)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|s| !s.trim().is_empty())
                    .collect()
            } else {
                let s = content.trim().to_string();
                if s.is_empty() { return "创建记忆需要提供 content 或 items。".into(); }
                vec![s]
            };
            
            let mut created = 0;
            for statement in &contents {
                let id = uuid::Uuid::new_v4().to_string();
                if let Ok(r) = database::run(
                    "INSERT INTO memories(id,content,updated_at) VALUES(?1,?2,datetime('now','localtime'))",
                    &[json!(&id), json!(statement)],
                ) {
                    if r.changes > 0 { created += 1; }
                }
            }
            format!("已记住 {} 条事实", created)
        }
        "delete" => {
            if !memory_id.trim().is_empty() {
                let sid = memory_id.trim().strip_prefix("MEMORY-").unwrap_or(memory_id.trim()).to_string();
                match database::run("DELETE FROM memories WHERE id=?1", &[json!(sid)]) {
                    Ok(r) if r.changes > 0 => format!("已删除记忆: {}", sid),
                    Err(e) => format!("删除失败: {}", e),
                    _ => format!("未找到记忆: {}", sid),
                }
            } else {
                "删除需要提供 memory_id。".into()
            }
        }
        "search" => {
            let q = query.trim();
            let start = start_time.trim();
            let end = end_time.trim();
            
            // 优先取 keywords 数组，否则按空格拆分 query
            let search_words: Vec<String> = if !keywords_json.is_empty() {
                serde_json::from_str::<Vec<String>>(keywords_json).unwrap_or_default()
            } else if !q.is_empty() {
                q.split_whitespace().map(String::from).collect()
            } else {
                vec![]
            };
            
            let mut sql = String::from("SELECT id, content, created_at FROM memories");
            let mut params: Vec<serde_json::Value> = Vec::new();
            let mut param_idx = 1;
            let mut where_clauses: Vec<String> = Vec::new();
            
            if !search_words.is_empty() {
                // FTS5 搜索：拼接所有关键词
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
                // LIKE 兜底：每个词都要匹配
                if where_clauses.is_empty() {
                    for w in &search_words {
                        let like = format!("%{}%", crate::database::escape_like(w));
                        where_clauses.push(format!("content LIKE ?{}", param_idx));
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
            sql.push_str(" ORDER BY created_at DESC LIMIT 10");
            
            match database::all(&sql, &params) {
                Ok(rows) => {
                    if rows.is_empty() {
                        return if q.is_empty() { "暂无记忆。".to_string() }
                               else { format!("未找到匹配「{}」的记忆。", q) };
                    }
                    let mut out = format!("## 记忆列表（共 {} 条）\n\n", rows.len());
                    for (i, r) in rows.iter().enumerate() {
                        let sid = r["id"].as_str().unwrap_or("");
                        let content = r["content"].as_str().unwrap_or("");
                        let created = r["created_at"].as_str().unwrap_or("");
                        let rel_time = relative_time(created);
                        out.push_str(&format!("[{}][来源：记忆] {}\nsource_id: MEMORY-{}\n{}\n\n", i + 1, rel_time, sid, content));
                    }
                    out
                }
                Err(e) => format!("搜索记忆失败: {}", e),
            }
        }
        _ => format!("未知操作「{}」。支持：create, delete, search。", action),
    }
}

/// 被动提醒：检查待办到期、项目停滞、投标截止，生成提醒文本注入 LLM 上下文
fn check_proactive_reminders() -> String {
    let mut reminders = Vec::new();

    // 1. 24h 内到期的待办（排除已确认的）
    if let Ok(rows) = database::all(
        "SELECT t.title, t.due_date, t.id FROM todos t WHERE t.done=0 AND t.due_date IS NOT NULL AND t.due_date <= date('now','+1 day') AND t.due_date >= date('now') AND NOT EXISTS (SELECT 1 FROM reminder_ack WHERE reminder_type='todo_due' AND entity_id=t.id) ORDER BY t.due_date ASC LIMIT 5",
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

    // 2. 今天到期的待办（排除已确认的）
    if let Ok(rows) = database::all(
        "SELECT t.title, t.id FROM todos t WHERE t.done=0 AND t.due_date=date('now') AND NOT EXISTS (SELECT 1 FROM reminder_ack WHERE reminder_type='todo_today' AND entity_id=t.id) ORDER BY t.due_date ASC LIMIT 5",
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

    // 3. 7 天未更新的项目（排除已确认的）
    if let Ok(rows) = database::all(
        "SELECT p.name, p.updated_at, p.id FROM projects p WHERE p.updated_at < date('now','-7 days') AND NOT EXISTS (SELECT 1 FROM reminder_ack WHERE reminder_type='project_stale' AND entity_id=p.id) ORDER BY p.updated_at ASC LIMIT 3",
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

    // 4. 3 天内投标截止（排除已确认的）
    if let Ok(rows) = database::all(
        "SELECT p.name, p.bid_time, p.id FROM projects p WHERE p.bid_time IS NOT NULL AND p.bid_time != '' AND p.bid_time <= date('now','+3 days') AND p.bid_time >= date('now') AND NOT EXISTS (SELECT 1 FROM reminder_ack WHERE reminder_type='bid_deadline' AND entity_id=p.id) ORDER BY p.bid_time ASC LIMIT 3",
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

fn fetch_parent_chunks(source_id_str: &str, fields: &[String], visited_chunks: &std::collections::HashSet<String>, visited_sources: &std::collections::HashSet<String>) -> String {
    let known_types = ["PROJECT", "PRODUCT", "FILE", "NOTE", "TODO", "CHANNEL", "VENDOR", "CONTACT", "CUSTOMER"];
    let (ot, oi) = if let Some(dash) = source_id_str.find('-') {
        let prefix = &source_id_str[..dash];
        if known_types.contains(&prefix.to_uppercase().as_str()) {
            (prefix.to_uppercase(), source_id_str[dash+1..].to_string())
        } else {
            ("__raw__".to_string(), source_id_str.to_string())
        }
    } else {
        ("__raw__".to_string(), source_id_str.to_string())
    };
    log::info!("[fetch_parent_chunks] 查询: source_type={}, source_id={}", ot, oi);
    
    let (sdid, resolved_type) = if ot == "__raw__" {
        // 裸 UUID：搜索所有类型的 source_data
        let q = "SELECT id, source_type FROM source_data WHERE source_id=?1 LIMIT 1";
        log::info!("[fetch_parent_chunks] 裸UUID查询: {} | params: {:?}", q, oi);
        match database::get(q, &[json!(&oi)]) {
            Ok(Some(r)) => {
                let id = r["id"].as_str().unwrap_or("").to_string();
                let st = r["source_type"].as_str().unwrap_or("").to_string();
                log::info!("[fetch_parent_chunks] 找到: id={}, type={}", id, st);
                (id, st)
            }
            Ok(None) => {
                log::info!("[fetch_parent_chunks] 查询返回None");
                (String::new(), String::new())
            }
            Err(e) => {
                log::info!("[fetch_parent_chunks] 查询出错: {}", e);
                (String::new(), String::new())
            }
        }
    } else {
        let q = "SELECT id FROM source_data WHERE source_type=?1 AND source_id=?2";
        log::info!("[fetch_parent_chunks] 带前缀查询: {} | params: {:?}, {:?}", q, ot, oi);
        match database::get(q, &[json!(&ot), json!(&oi)]) {
            Ok(Some(r)) => {
                let id = r["id"].as_str().unwrap_or("").to_string();
                log::info!("[fetch_parent_chunks] 找到: id={}", id);
                (id, ot.clone())
            }
            Ok(None) => {
                log::info!("[fetch_parent_chunks] 查询返回None");
                (String::new(), ot.clone())
            }
            Err(e) => {
                log::info!("[fetch_parent_chunks] 查询出错: {}", e);
                (String::new(), ot.clone())
            }
        }
    };

    log::info!("[fetch_parent_chunks] sdid={}, type={}", sdid, resolved_type);
    if sdid.is_empty() { return format!("未找到对象: {source_id_str}"); }

    // 检查是否已在前文展示过
    let source_full_id = format!("{}-{}", resolved_type, oi);
    let rows = database::all("SELECT chunk_id, content, chunk_index FROM knowledge_chunks WHERE source_data_id=?1 AND parent_id IS NULL ORDER BY chunk_index LIMIT 3", &[json!(sdid)]).unwrap_or_default();
    let chunk_ids: Vec<String> = rows.iter().map(|r| r["chunk_id"].as_str().unwrap_or("").chars().take(8).collect()).collect();
    if !chunk_ids.is_empty() && chunk_ids.iter().all(|id| visited_chunks.contains(id)) && visited_sources.contains(&source_full_id) {
        return format!("对象 {} 的内容已在前文展示。", source_id_str);
    }

    let chunks: Vec<String> = rows.iter().map(|r| {
        let content = r["content"].as_str().unwrap_or("");
        let chunk_id = r["chunk_id"].as_str().unwrap_or("");
        // 安全截断：使用 char_indices 确保不在 UTF-8 字符中间截断
        let content_truncated = if content.len() > 1800 {
            let mut end = 0;
            for (i, _) in content.char_indices() {
                if i > 1800 { break; }
                end = i;
            }
            &content[..end]
        } else {
            content
        };
        format!("[OBJECT-{} 父块{:?}] ChunkID: {}\n{}\n", resolved_type, r["chunk_index"].as_i64().unwrap_or(0)+1, &chunk_id[..chunk_id.len().min(8)], content_truncated)
    }).collect();
    if chunks.is_empty() { return format!("对象 {source_id_str} 无可用内容。"); }
    let mut full_content = chunks.join("\n");

    // 拼接关联对象元数据（从 business_relations 查询）
    let meta = build_chunk_metadata(&resolved_type, &oi, true);
    if !meta.is_empty() {
        full_content.push_str(&format!("\n\n{}", meta));
    }

    // 如果没有指定 fields，返回完整内容
    if fields.is_empty() { return full_content; }

    // 按 fields 过滤，提取关键行
    let mut result: Vec<String> = Vec::new();
    result.push(format!("[OBJECT-{} Recall by fields: {:?}]", resolved_type, fields));
    for field in fields {
        let field_lower = field.to_lowercase();
        // 在内容中查找包含该字段关键词的行
        let matching_lines: Vec<&str> = full_content.lines().filter(|line| {
            let line_lower = line.to_lowercase();
            match field_lower.as_str() {
                "vendor" => line_lower.contains("厂商") || line_lower.contains("供应商") || line_lower.contains("vendor"),
                "channel" => line_lower.contains("渠道") || line_lower.contains("channel"),
                "contact" => line_lower.contains("联系人") || line_lower.contains("contact"),
                "dynamics" => line_lower.contains("dynamics") || line_lower.contains("操作记录") || line_lower.contains("动态"),
                "budget" => line_lower.contains("预算") || line_lower.contains("budget") || line_lower.contains("金额"),
                "status" => line_lower.contains("状态") || line_lower.contains("status"),
                "description" => line_lower.contains("描述") || line_lower.contains("description"),
                "bid_time" => line_lower.contains("投标") || line_lower.contains("bid"),
                "products" => line_lower.contains("产品") || line_lower.contains("product"),
                "files" => line_lower.contains("文件") || line_lower.contains("file"),
                "notes" => line_lower.contains("笔记") || line_lower.contains("note"),
                "todos" => line_lower.contains("待办") || line_lower.contains("todo"),
                _ => line_lower.contains(&field_lower),
            }
        }).collect();
        if !matching_lines.is_empty() {
            result.push(format!("## {}:", field));
            for line in matching_lines.iter().take(5) {
                result.push(line.to_string());
            }
        }
    }
    if result.len() <= 1 {
        // 没有找到匹配的字段，返回完整内容
        full_content
    } else {
        result.join("\n")
    }
}

fn relative_time(created_at: &str) -> String {
    if created_at.is_empty() { return String::new(); }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let ts = chrono::NaiveDateTime::parse_from_str(created_at, "%Y-%m-%d %H:%M:%S").map(|dt| dt.and_utc().timestamp()).unwrap_or(0);
    let delta = (now - ts).max(0);
    let rel = if delta < 60 { "刚刚".to_string() } else if delta < 3600 { format!("{}分钟前", delta / 60) } else if delta < 86400 { format!("{}小时前", delta / 3600) } else if delta < 2592000 { format!("{}天前", delta / 86400) } else if delta < 77760000 { format!("{}个月前", delta / 2592000) } else { format!("{}年前", delta / 31104000) };
    format!("🕐 {} ({})", rel, &created_at[..created_at.len().min(16)])
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


#[tauri::command]
pub async fn agent_chat_workflow(user_query: String, max_iterations: Option<i32>, history: Vec<Value>, app_handle: Option<tauri::AppHandle>) -> Result<Value, String> {
    AGENT_CANCEL.store(false, Ordering::Release);
    log::info!("workflow start, app_some={}", app_handle.is_some());
    let query = user_query.trim().to_string();
    let max_iters = max_iterations.unwrap_or(8).clamp(2, 20);
    if query.is_empty() { return Ok(json!({"success":true,"answer":"请输入问题。","iterations":0})); }
    crate::services::chat_api::invalidate_chat_config_cache();
    { let mut c = API_CACHE.write().unwrap_or_else(|e| e.into_inner()); c.3 = Instant::now() - std::time::Duration::from_secs(60); }
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
                // ★ 过滤空数组，避免 LLM API 报错
                let tcs: Option<Vec<ToolCall>> = h["tool_calls"].as_array()
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
                let tcs = tcs.filter(|v| !v.is_empty()); // 确保空 Vec 变为 None
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
        log::info!("[主动提醒] 注入提醒: {}", &reminders[..reminders.len().min(100)]);
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
    let mut prev_pt: i64 = 0;
    let mut budget = 150i32;  // 增加到150分
    let mut search_cost = 8i32;  // 降低初始消耗到8分
    let mut traverse_source_counter: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    // ★ 振荡检测：防止 FOCUS ↔ EXPAND 无限循环
    let mut oscillation_count: usize = 0;
    let mut last_search_count: usize = 0;
    
    // 加载用户策略（policy）— 从 policies 表读取
    let user_policies = {
        let rows = crate::database::all(
            "SELECT id, content, updated_at FROM policies ORDER BY updated_at DESC",
            &[],
        ).unwrap_or_default();
        if rows.is_empty() {
            "无特殊策略要求。".to_string()
        } else {
            let mut lines: Vec<String> = Vec::new();
            for r in rows.iter() {
                if let (Some(sid), Some(content)) = (r["id"].as_str(), r["content"].as_str()) {
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
    let mut tool_text_retry_count = 0u32; // 工具调用文本重试计数器

    loop {
        // ★ 检查停止信号
        if AGENT_CANCEL.load(Ordering::Acquire) {
            AGENT_CANCEL.store(false, Ordering::Release);
            log::info!("收到停止信号，中断执行");
            let ws = session.generate_scratchpad();
            return Ok(json!({"success":true,"answer":"已停止执行。","workspace":ws,"iterations":iters,"cancelled":true,"promptTokens":pt,"completionTokens":ct,"tool_log":tool_log}));
        }
        
        log::info!("预算: {}/150 cost={}", budget, search_cost);
        if let Some(ref h) = app_handle { let _ = h.emit("agent:status", json!({"stage":"thinking"})); }
        // ★ 每轮注意力衰减（Residual 方式：new_score = old_score * 0.95 + gain）
        // gain 在 SessionEffect::UpdateObjectScore 中通过 EMA 完成
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

        // ★ 注入 scratchpad + budget_info 到 system prompt（供当前轮 LLM 读取）
        let scratchpad = session.generate_scratchpad();
        let new_sys = AGENT_SYSTEM_PROMPT
            .replace("{{today}}", &chrono::Local::now().format("%Y年%m月%d日").to_string())
            .replace("{{user_policies}}", &user_policies)
            .replace("{{session_scratchpad}}", &scratchpad);
        let budget_info = if iters == 1 {
            format!("\n\n## 🧠 思维预算\n- 剩余预算: {}/150\n- 本次消耗: {} 分\n- 证据足够就立刻回答，不要浪费预算。", budget, search_cost)
        } else {
            let round_pt = pt - prev_pt;
            format!("\n\n## 🧠 思维预算\n- 剩余预算: {}/150 | 本次消耗: {} 分\n- 上下文: {} tokens（本轮 +{}）| 已生成: {} tokens\n- 上下文越大响应越慢且越贵。证据足够就立刻回答，不要浪费预算。", budget, search_cost, pt, round_pt, ct)
        };
        let new_sys = format!("{}{}", new_sys, budget_info);
        if let Some(ref mut content) = messages[0].content {
            *content = new_sys;
        }

        // ★ 单请求流式：content 逐 token 输出到前端，同时累积 tool_calls delta
        let (ac_owned, reasoning, tcs_streamed, usage) = call_llm_streaming(&messages, Some(&tools), &app_handle).await?;
        if let Some(u) = usage { prev_pt = pt; pt += u["prompt_tokens"].as_i64().unwrap_or(0); ct += u["completion_tokens"].as_i64().unwrap_or(0); }
        let ac = ac_owned.as_str();
        let mut tcs: Option<Vec<Value>> = if tcs_streamed.is_empty() { None } else { Some(tcs_streamed) };
        let mut cleaned_ac: Option<String> = None; // 清理后的 content（当 XML 解析失败时）

        // ★ 兜底：如果 tool_calls 为空但 content 包含 XML 格式工具调用，尝试解析
        if tcs.is_none() || tcs.as_ref().map_or(false, |v| v.is_empty()) {
            if let Some(parsed) = parse_xml_tool_calls(ac) {
                log::info!("[LLM检索] 从 content 解析到 XML 工具调用: {:?}", parsed);
                tcs = Some(parsed);
            } else if ac.contains("tool_calls") || ac.contains("invoke") {
                // 解析失败：用正则移除 tool_calls 块，剩余内容作为最终答案
                log::info!("[LLM检索] XML工具调用解析失败，尝试清理原始文本");
                // 匹配所有 DSML 变体的 tool_calls 块（Unicode 感知，含闭合标签）
                if let Ok(re) = regex::Regex::new(r"(?si)[^a-zA-Z0-9]*DSML[^a-zA-Z0-9]*tool_calls[^a-zA-Z0-9]*>.*?[^a-zA-Z0-9]*DSML[^a-zA-Z0-9]*tool_calls[^a-zA-Z0-9]*>") {
                    let cleaned = re.replace_all(ac, "").trim().to_string();
                    if !cleaned.is_empty() {
                        log::info!("[LLM检索] 清理后内容长度: {}", cleaned.len());
                        cleaned_ac = Some(cleaned);
                    }
                }
            }
        }

        let effective_ac = cleaned_ac.as_deref().unwrap_or(ac);

        match tcs {
            None => {
                // ★ 检测：LLM 是否将工具调用以 JSON、XML 或代码块格式输出（而非通过 tool_calls 字段）
                let has_tool_pattern = (effective_ac.trim().starts_with('{') && effective_ac.contains("tool_calls"))
                    || effective_ac.starts_with("<invoke")
                    || (effective_ac.contains("ext_tool_") && effective_ac.contains("```"));
                if has_tool_pattern && tool_text_retry_count < 2 {
                    tool_text_retry_count += 1;
                    log::warn!("[Agent] 检测到 content 中包含工具调用文本 (第{}次)，追加提示重试", tool_text_retry_count);
                    // 将 LLM 的文本输出作为 assistant 消息
                    messages.push(ChatMessage{role:MessageRole::Assistant, content: Some(effective_ac.to_string()), tool_calls: None, tool_call_id: None, name: None, reasoning_content: None});
                    // 追加系统提示要求使用标准格式
                    messages.push(ChatMessage{role:MessageRole::User, content: Some("你的回复包含了工具调用文本（如 tool_calls 或 ext_tool_），但没有通过 tool_calls 字段返回。这会导致工具无法被执行。请使用标准的 function calling 格式重新调用工具，不要在 content 中输出工具调用。".into()), tool_calls: None, tool_call_id: None, name: None, reasoning_content: None});
                    continue; // 重新进入循环
                }

                // ★ 单请求流式：答案已在流式中逐 token 输出，无需再次生成
                let mut answer = effective_ac.to_string();
                if answer.trim().is_empty() {
                    log::info!("[Agent] 流式内容为空，回退补生成");
                    answer = call_llm_without_tools(&messages, &app_handle).await.unwrap_or_else(|e| { log::info!("[Agent] 流式补生成失败: {e}"); format!("未能生成有效回答: {e}") });
                }
                // 清理 DSML 残留（流式原文可能含未解析的工具调用标记）
                let re_dsml = regex::Regex::new(r"(?si)[^a-zA-Z0-9]*DSML[^a-zA-Z0-9]*[^\n]*").unwrap();
                answer = re_dsml.replace_all(&answer, "").to_string();
                if let Some(ref h) = app_handle { let _ = h.emit("agent:status", json!({"stage":"answer"})); let _ = h.emit("agent:final-answer", json!({"answer": &answer})); }
                if !answer.is_empty() { session.record_observation(&answer); let cited = extract_citations(&answer); log::info!("[LLM检索] cited_ids(from final): {:?}", cited); if cited.len() >= 2 { crate::services::synapse::update_object_links_weighted(&cited, 1.0); } }
                let ws = session.generate_scratchpad(); return Ok(json!({"success":true,"answer":answer,"workspace":ws,"iterations":iters,"promptTokens":pt,"completionTokens":ct,"tool_log":tool_log}));
            }
            Some(ref calls) if !calls.is_empty() => {
                // ★ 工具轮：本轮已流式输出的 interim 内容通知前端归置到思考区
                if !effective_ac.trim().is_empty() { if let Some(ref h) = app_handle { let _ = h.emit("agent:round-thought", json!({"text": effective_ac})); } }
                // ★ 预算检查：不足时强制回答（在工具调用之前检查）
                if budget < search_cost {
                    if let Some(ref h) = app_handle {
                        let kw = calls.first().and_then(|tc| tc["function"]["arguments"].as_str().map(|s| s.to_string())).unwrap_or_default();
                        let _ = h.emit("agent:budget-low", json!({"remaining":budget,"cost":search_cost,"next_query":kw}));
                    }
                    messages.push(ChatMessage{role:MessageRole::User,content:Some("预算不足，请直接总结已获取的信息。".into()),tool_calls:None,tool_call_id:None,name:None,reasoning_content:None});
                    let ans = call_llm_without_tools(&messages, &app_handle).await.unwrap_or_else(|e| { log::info!("[Agent] 流式总结失败: {e}"); format!("总结失败: {e}") });
                    let ws = session.generate_scratchpad();
                    return Ok(json!({"success":true,"answer":ans,"workspace":ws,"iterations":iters,"budget_exhausted":true,"promptTokens":pt,"completionTokens":ct,"tool_log":tool_log}));
                }
                // ★ 振荡检测：如果本轮搜索次数与上轮相同，可能在振荡
                let current_search_count = session.search_keywords.len();
                if current_search_count == last_search_count && current_search_count > 0 {
                    oscillation_count += 1;
                } else {
                    oscillation_count = 0;
                }
                last_search_count = current_search_count;
                if oscillation_count >= 3 {
                    log::info!("[Agent] 检测到状态振荡 ({} 轮无新搜索)，强制终止", oscillation_count);
                    messages.push(ChatMessage{role:MessageRole::User,content:Some("已检测到重复检索，信息可能已足够，请直接总结。".into()),tool_calls:None,tool_call_id:None,name:None,reasoning_content:None});
                    let ans = call_llm_without_tools(&messages, &app_handle).await.unwrap_or_else(|e| { log::info!("[Agent] 流式总结失败: {e}"); format!("总结失败: {e}") });
                    let ws = session.generate_scratchpad();
                    return Ok(json!({"success":true,"answer":ans,"workspace":ws,"iterations":iters,"oscillation_break":true,"promptTokens":pt,"completionTokens":ct,"tool_log":tool_log}));
                }
                if iters >= max_iters {
                    messages.push(ChatMessage{role:MessageRole::User,content:Some("已达到工具调用上限，请直接总结。".into()),tool_calls:None,tool_call_id:None,name:None,reasoning_content:None});
                    let ans = call_llm_without_tools(&messages, &app_handle).await.unwrap_or_else(|e| { log::info!("[Agent] 流式总结失败: {e}"); format!("总结失败: {e}") });
                    let ws = session.generate_scratchpad();
                    if let Some(ref h) = app_handle { let _ = h.emit("agent:status", json!({"stage":"answer"})); let _ = h.emit("agent:final-answer", json!({"answer": ans})); }
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
        "SELECT source_type, source_id, title, content as preview, updated_at FROM source_data
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
            log::info!("[Boundary] Δt={}m timeB={:.2} spaceB={:.2} → score={:.2}", d_time as i64, time_b, space_b, boundary);
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
        // object_links 仅用于后台边界评分，不输出给 LLM
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
        let like = format!("%{}%", crate::database::escape_like(&q));
        if let Ok(rows) = database::all(
            "SELECT source_type, source_id, title, content as preview, updated_at FROM source_data WHERE (title LIKE ?1 OR content LIKE ?1) AND (source_type=?2 OR ?2='') ORDER BY updated_at DESC",
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
        let preview_full = r["preview"].as_str().unwrap_or("");
        let preview = {
            let char_count = preview_full.chars().count();
            if char_count <= 200 {
                preview_full.to_string()
            } else {
                // 动态截断：优先在换行符处截断，保持信息完整
                let mut last_newline = 0;
                let mut char_idx = 0;
                for ch in preview_full.chars() {
                    if ch == '\n' && char_idx <= 200 {
                        last_newline = char_idx;
                    }
                    char_idx += 1;
                    if char_idx > 250 { break; } // 超过 250 字符停止扫描
                }
                let cut_at = if last_newline > 100 { last_newline } else { 200 };
                let truncated: String = preview_full.chars().take(cut_at).collect();
                format!("{}…(已截断，共{}字)", truncated.trim_end(), char_count)
            }
        };
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
            out.push_str(&format!("[{}] {} | source_id: {}-{}\n  {} | {}\n\n", st, name, st, sid, rel, preview));
        }
        // object_links 仅用于后台注意力机制，不输出给 LLM
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
                            results_mutex.lock().unwrap_or_else(|e| e.into_inner()).push(result);
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
                            let res = if let Some(ids_arr) = a["ids"].as_array().filter(|arr| !arr.is_empty()) {
                                // 批量模式：传入 ids 数组
                                let ids: Vec<String> = ids_arr.iter().filter_map(|v| v.as_str().map(String::from)).collect();
                                execute_batch_recall(&ids, &visited_chunks_snapshot, &visited_sources_snapshot
                                ).await
                            } else if a.get("id").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).is_some() {
                                // 单个模式：传入单个 id
                                fetch_parent_chunks(a["id"].as_str().unwrap_or(""), &[], &visited_chunks_snapshot, &visited_sources_snapshot)
                            } else {
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
                                a["replace_ids"].as_str().unwrap_or(""),
                                a["query"].as_str().unwrap_or(""),
                                &keywords_str,
                                a["start_time"].as_str().unwrap_or(""),
                                a["end_time"].as_str().unwrap_or(""),
                            );
                            (res, vec![])
                        }
                        "ext_tool_manage_policy" => {
                            let id = a["id"].as_str().map(|s| s.to_string());
                            let content = a["content"].as_str().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
                            let res = match (id, content) {
                                (Some(ref pid), Some(ref c)) => {
                                    let real_id = pid.strip_prefix("policy-").unwrap_or(pid);
                                    let r = database::run("UPDATE policies SET content=?1, updated_at=datetime('now','localtime') WHERE id=?2", &[json!(c), json!(real_id)]);
                                    match r {
                                        Ok(r) if r.changes > 0 => format!("已更新策略：{}", c),
                                        Ok(_) => format!("未找到策略 ID：{}", real_id),
                                        Err(e) => format!("更新失败：{}", e),
                                    }
                                }
                                (Some(ref pid), None) => {
                                    let real_id = pid.strip_prefix("policy-").unwrap_or(pid);
                                    let r = database::run("DELETE FROM policies WHERE id=?1", &[json!(real_id)]);
                                    match r {
                                        Ok(r) if r.changes > 0 => "已删除策略".to_string(),
                                        Ok(_) => format!("未找到策略 ID：{}", real_id),
                                        Err(e) => format!("删除失败：{}", e),
                                    }
                                }
                                (None, Some(ref c)) => {
                                    let new_id = uuid::Uuid::new_v4().to_string();
                                    let r = database::run("INSERT INTO policies(id,content,updated_at) VALUES(?1,?2,datetime('now','localtime'))", &[json!(&new_id), json!(c)]);
                                    match r { Ok(_) => format!("已记入宪法：{}", c), Err(e) => format!("记录失败：{}", e) }
                                }
                                _ => "参数不完整：需要content或id".to_string(),
                            };
                            (res, vec![])
                        }
                        "ext_tool_manage_project" => {
                            let action = a["action"].as_str().unwrap_or("");
                            let res = match action {
                                "create" => {
                                    let name = a["name"].as_str().unwrap_or("");
                                    if name.is_empty() { "创建项目需要提供 name。".to_string() }
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
                                        let rt = tokio::runtime::Runtime::new().unwrap();
                                        match rt.block_on(crate::commands::project_commands::project_create(data)) {
                                            Ok(project) => {
                                                let pid = project["id"].as_str().unwrap_or("");
                                                format!("已创建项目「{}」(ID: {})", name, pid)
                                            },
                                            Err(e) => format!("创建项目失败: {}", e),
                                        }
                                    }
                                }
                                "update" => {
                                    let raw_id = a["project_id"].as_str().unwrap_or("");
                                    let project_id = raw_id.strip_prefix("PROJECT-").unwrap_or(raw_id);
                                    if project_id.is_empty() { "更新项目需要提供 project_id。".to_string() }
                                    else {
                                        let mut data = json!({});
                                        if let Some(v) = a["name"].as_str().filter(|s| !s.is_empty()) { data["name"] = json!(v); }
                                        if let Some(v) = a["description"].as_str().filter(|s| !s.is_empty()) { data["description"] = json!(v); }
                                        if let Some(v) = a["status"].as_str().filter(|s| !s.is_empty()) { data["status"] = json!(v); }
                                        if let Some(v) = a["bid_time"].as_str().filter(|s| !s.is_empty()) { data["bidTime"] = json!(v); }
                                        if let Some(v) = a["budget"].as_f64().filter(|v| *v > 0.0) { data["budget"] = json!(v); }
                                        if data.as_object().map_or(true, |m| m.is_empty()) {
                                            "未提供任何更新字段。".to_string()
                                        } else {
                                            // 使用 block_on 调用异步函数
                                            let rt = tokio::runtime::Runtime::new().unwrap();
                                            match rt.block_on(crate::commands::project_commands::project_update(project_id.to_string(), data)) {
                                                Ok(_) => format!("已更新项目 {}", project_id),
                                                Err(e) => format!("更新项目失败: {}", e),
                                            }
                                        }
                                    }
                                }
                                "delete" => {
                                    let raw_id = a["project_id"].as_str().unwrap_or("");
                                    let project_id = raw_id.strip_prefix("PROJECT-").unwrap_or(raw_id);
                                    if project_id.is_empty() { "删除项目需要提供 project_id。".to_string() }
                                    else {
                                        // 使用 block_on 调用异步函数
                                        let rt = tokio::runtime::Runtime::new().unwrap();
                                        match rt.block_on(crate::commands::project_commands::project_delete(project_id.to_string())) {
                                            Ok(_) => format!("已删除项目 {} 及其所有关联数据", project_id),
                                            Err(e) => format!("删除项目失败: {}", e),
                                        }
                                    }
                                }
                                _ => format!("未知操作「{}」。支持：create, update, delete。", action),
                            };
                            (res, vec![])
                        }
                        "ext_tool_manage_partner" => {
                            let action = a["action"].as_str().unwrap_or("");
                            let res = match action {
                                "create" => {
                                    let ptype = a["type"].as_str().unwrap_or("");
                                    let name = a["name"].as_str().unwrap_or("");
                                    if ptype.is_empty() || name.is_empty() { "创建合作伙伴需要提供 type 和 name。".to_string() }
                                    else {
                                        let data = json!({
                                            "type": ptype.to_uppercase(),
                                            "name": name,
                                            "phone": a["phone"].as_str().unwrap_or(""),
                                            "company": a["address"].as_str().unwrap_or(""),
                                            "remark": a["remark"].as_str().unwrap_or("")
                                        });
                                        // 使用 block_on 调用异步函数
                                        let rt = tokio::runtime::Runtime::new().unwrap();
                                        match rt.block_on(crate::commands::partner_commands::partner_create(data)) {
                                            Ok(result) => {
                                                let id = result["id"].as_str().unwrap_or("");
                                                format!("已创建{}「{}」(ID: {})", ptype, name, id)
                                            },
                                            Err(e) => format!("创建失败: {}", e),
                                        }
                                    }
                                }
                                "update" => {
                                    let partner_id = a["partner_id"].as_str().unwrap_or("");
                                    if partner_id.is_empty() { "更新需要提供 partner_id。".to_string() }
                                    else {
                                        let mut data = json!({"id": partner_id});
                                        if let Some(v) = a["name"].as_str().filter(|s| !s.is_empty()) { data["name"] = json!(v); }
                                        if let Some(v) = a["phone"].as_str().filter(|s| !s.is_empty()) { data["phone"] = json!(v); }
                                        if let Some(v) = a["address"].as_str().filter(|s| !s.is_empty()) { data["company"] = json!(v); }
                                        if let Some(v) = a["remark"].as_str().filter(|s| !s.is_empty()) { data["remark"] = json!(v); }
                                        if data.as_object().map_or(true, |m| m.len() <= 1) {
                                            "未提供任何更新字段。".to_string()
                                        } else {
                                            // 使用 block_on 调用异步函数
                                            let rt = tokio::runtime::Runtime::new().unwrap();
                                            match rt.block_on(crate::commands::partner_commands::partner_update(data)) {
                                                Ok(_) => format!("已更新合作伙伴 {}", partner_id),
                                                Err(e) => format!("更新失败: {}", e),
                                            }
                                        }
                                    }
                                }
                                "delete" => {
                                    let partner_id = a["partner_id"].as_str().unwrap_or("");
                                    if partner_id.is_empty() { "删除需要提供 partner_id。".to_string() }
                                    else {
                                        // 使用 block_on 调用异步函数
                                        let rt = tokio::runtime::Runtime::new().unwrap();
                                        match rt.block_on(crate::commands::partner_commands::partner_delete(partner_id.to_string())) {
                                            Ok(_) => format!("已删除合作伙伴 {}", partner_id),
                                            Err(e) => format!("删除失败: {}", e),
                                        }
                                    }
                                }
                                "link_project" => {
                                    let partner_id = a["partner_id"].as_str().unwrap_or("");
                                    let project_id = a["project_id"].as_str().unwrap_or("");
                                    if partner_id.is_empty() || project_id.is_empty() { "关联需要提供 partner_id 和 project_id。".to_string() }
                                    else {
                                        let contact_id = a["contact_id"].as_str().map(|s| s.to_string());
                                        let remark = a["remark"].as_str().map(|s| s.to_string());
                                        // 使用 block_on 调用异步函数
                                        let rt = tokio::runtime::Runtime::new().unwrap();
                                        match rt.block_on(crate::commands::partner_commands::partner_link_project(
                                            partner_id.to_string(),
                                            project_id.to_string(),
                                            contact_id,
                                            remark
                                        )) {
                                            Ok(_) => format!("已将合作伙伴关联到项目"),
                                            Err(e) => format!("关联失败: {}", e),
                                        }
                                    }
                                }
                                "unlink_project" => {
                                    let partner_id = a["partner_id"].as_str().unwrap_or("");
                                    let project_id = a["project_id"].as_str().unwrap_or("");
                                    if partner_id.is_empty() || project_id.is_empty() { "取消关联需要提供 partner_id 和 project_id。".to_string() }
                                    else {
                                        // 使用 block_on 调用异步函数
                                        let rt = tokio::runtime::Runtime::new().unwrap();
                                        match rt.block_on(crate::commands::partner_commands::partner_unlink_project(
                                            partner_id.to_string(),
                                            project_id.to_string()
                                        )) {
                                            Ok(_) => format!("已取消合作伙伴与项目的关联"),
                                            Err(e) => format!("取消关联失败: {}", e),
                                        }
                                    }
                                }
                                _ => format!("未知操作「{}」。支持：create, update, delete, link_project, unlink_project。", action),
                            };
                            (res, vec![])
                        }
                        "ext_tool_manage_contact" => {
                            let action = a["action"].as_str().unwrap_or("");
                            let res = match action {
                                "create" => {
                                    let name = a["name"].as_str().unwrap_or("");
                                    if name.is_empty() { "创建联系人需要提供 name。".to_string() }
                                    else {
                                        let phone = a["phone"].as_str().unwrap_or("");
                                        let email = a["email"].as_str().unwrap_or("");
                                        let position = a["position"].as_str().unwrap_or("");
                                        let remark = a["remark"].as_str().unwrap_or("");
                                        let parent_id = a["parent_id"].as_str().unwrap_or("");
                                        let raw_proj_id = a["project_id"].as_str().unwrap_or("");
                                        let project_id = raw_proj_id.strip_prefix("PROJECT-").unwrap_or(raw_proj_id);
                                        let data = json!({
                                            "name": name,
                                            "phone": phone,
                                            "email": email,
                                            "position": position,
                                            "remark": remark,
                                            "partnerId": parent_id
                                        });
                                        // 使用 block_on 调用异步函数
                                        let rt = tokio::runtime::Runtime::new().unwrap();
                                        match rt.block_on(crate::commands::partner_commands::partner_contact_create(data)) {
                                            Ok(result) => {
                                                let id = result["id"].as_str().unwrap_or("");
                                                // 如果有项目ID，手动关联到项目
                                                if !project_id.is_empty() && !id.is_empty() {
                                                    let _ = rt.block_on(crate::commands::partner_commands::contact_link_project(
                                                        id.to_string(),
                                                        project_id.to_string()
                                                    ));
                                                }
                                                format!("已创建联系人「{}」(ID: {})", name, id)
                                            },
                                            Err(e) => format!("创建失败: {}", e),
                                        }
                                    }
                                }
                                "update" => {
                                    let contact_id = a["contact_id"].as_str().unwrap_or("");
                                    if contact_id.is_empty() { "更新需要提供 contact_id。".to_string() }
                                    else {
                                        let mut data = json!({"id": contact_id});
                                        if let Some(v) = a["name"].as_str().filter(|s| !s.is_empty()) { data["name"] = json!(v); }
                                        if let Some(v) = a["phone"].as_str().filter(|s| !s.is_empty()) { data["phone"] = json!(v); }
                                        if let Some(v) = a["email"].as_str().filter(|s| !s.is_empty()) { data["email"] = json!(v); }
                                        if let Some(v) = a["position"].as_str().filter(|s| !s.is_empty()) { data["position"] = json!(v); }
                                        if let Some(v) = a["remark"].as_str().filter(|s| !s.is_empty()) { data["remark"] = json!(v); }
                                        if data.as_object().map_or(true, |m| m.len() <= 1) {
                                            "未提供任何更新字段。".to_string()
                                        } else {
                                            // 使用 block_on 调用异步函数
                                            let rt = tokio::runtime::Runtime::new().unwrap();
                                            match rt.block_on(crate::commands::partner_commands::partner_contact_update(data)) {
                                                Ok(_) => format!("已更新联系人 {}", contact_id),
                                                Err(e) => format!("更新失败: {}", e),
                                            }
                                        }
                                    }
                                }
                                "delete" => {
                                    let contact_id = a["contact_id"].as_str().unwrap_or("");
                                    if contact_id.is_empty() { "删除需要提供 contact_id。".to_string() }
                                    else {
                                        // 使用 block_on 调用异步函数
                                        let rt = tokio::runtime::Runtime::new().unwrap();
                                        match rt.block_on(crate::commands::partner_commands::partner_contact_delete(contact_id.to_string())) {
                                            Ok(_) => format!("已删除联系人 {}", contact_id),
                                            Err(e) => format!("删除失败: {}", e),
                                        }
                                    }
                                }
                                "link_project" => {
                                    let contact_id = a["contact_id"].as_str().unwrap_or("");
                                    let project_id = a["project_id"].as_str().unwrap_or("");
                                    if contact_id.is_empty() || project_id.is_empty() { "关联需要提供 contact_id 和 project_id。".to_string() }
                                    else {
                                        // 使用 block_on 调用异步函数
                                        let rt = tokio::runtime::Runtime::new().unwrap();
                                        match rt.block_on(crate::commands::partner_commands::contact_link_project(
                                            contact_id.to_string(),
                                            project_id.to_string()
                                        )) {
                                            Ok(_) => "已将联系人关联到项目".to_string(),
                                            Err(e) => format!("关联失败: {}", e),
                                        }
                                    }
                                }
                                "unlink_project" => {
                                    let contact_id = a["contact_id"].as_str().unwrap_or("");
                                    let project_id = a["project_id"].as_str().unwrap_or("");
                                    if contact_id.is_empty() || project_id.is_empty() { "取消关联需要提供 contact_id 和 project_id。".to_string() }
                                    else {
                                        // 使用 block_on 调用异步函数
                                        let rt = tokio::runtime::Runtime::new().unwrap();
                                        match rt.block_on(crate::commands::partner_commands::contact_unlink_project(
                                            contact_id.to_string(),
                                            project_id.to_string()
                                        )) {
                                            Ok(_) => "已取消联系人与项目的关联".to_string(),
                                            Err(e) => format!("取消关联失败: {}", e),
                                        }
                                    }
                                }
                                "list" => {
                                    let type_filter = a["type"].as_str().unwrap_or("");
                                    let sql = if type_filter.is_empty() {
                                        "SELECT pc.id, pc.name, pc.phone, pc.email, pc.position, pc.remark, pcm.partner_id, p.type as partner_type, p.name as partner_name, GROUP_CONCAT(DISTINCT proj.name) as project_names FROM partner_contacts pc LEFT JOIN partner_contact_mapping pcm ON pc.id=pcm.contact_id LEFT JOIN partners p ON pcm.partner_id=p.id LEFT JOIN business_relations br ON br.source_type='CONTACT' AND br.source_id=pc.id AND br.target_type='PROJECT' AND br.relation_type='belongs_to' LEFT JOIN projects proj ON proj.id=br.target_id GROUP BY pc.id ORDER BY pc.name"
                                    } else {
                                        "SELECT pc.id, pc.name, pc.phone, pc.email, pc.position, pc.remark, pcm.partner_id, p.type as partner_type, p.name as partner_name, GROUP_CONCAT(DISTINCT proj.name) as project_names FROM partner_contacts pc LEFT JOIN partner_contact_mapping pcm ON pc.id=pcm.contact_id LEFT JOIN partners p ON pcm.partner_id=p.id LEFT JOIN business_relations br ON br.source_type='CONTACT' AND br.source_id=pc.id AND br.target_type='PROJECT' AND br.relation_type='belongs_to' LEFT JOIN projects proj ON proj.id=br.target_id WHERE p.type=?1 GROUP BY pc.id ORDER BY pc.name"
                                    };
                                    let params = if type_filter.is_empty() { vec![] } else { vec![json!(type_filter.to_uppercase())] };
                                    match database::all(sql, &params) {
                                        Ok(rows) => {
                                            if rows.is_empty() { "通讯录为空。".to_string() }
                                            else {
                                                let mut out = format!("## 通讯录（共 {} 条）\n\n", rows.len());
                                                for r in &rows {
                                                    let name = r["name"].as_str().unwrap_or("");
                                                    let phone = r["phone"].as_str().unwrap_or("");
                                                    let email = r["email"].as_str().unwrap_or("");
                                                    let position = r["position"].as_str().unwrap_or("");
                                                    let partner_name = r["partner_name"].as_str().unwrap_or("");
                                                    let partner_type = r["partner_type"].as_str().unwrap_or("");
                                                    let project_names = r["project_names"].as_str().unwrap_or("");
                                                    let mut line = format!("• {} | {} | {} | {} | 归属: {}({})", name, phone, email, position, partner_type, partner_name);
                                                    if !project_names.is_empty() { line.push_str(&format!(" | 项目: {}", project_names)); }
                                                    out.push_str(&format!("{}\n", line));
                                                }
                                                out
                                            }
                                        }
                                        Err(e) => format!("查询通讯录失败: {}", e),
                                    }
                                }
                                _ => format!("未知操作「{}」。支持：create, update, delete, link_project, unlink_project, list。", action),
                            };
                            (res, vec![])
                        }
                        "ext_tool_query" => {
                            let res = execute_structured_query(&a);
                            (res, vec![])
                        }
                        "ext_tool_manage_reminder" => {
                            let action = a["action"].as_str().unwrap_or("");
                            let rtype = a["reminder_type"].as_str().unwrap_or("");
                            let eid = a["entity_id"].as_str().unwrap_or("");
                            let res = match action {
                                "ack" => {
                                    let _ = database::run("INSERT OR IGNORE INTO reminder_ack(reminder_type, entity_id) VALUES(?1, ?2)", &[json!(rtype), json!(eid)]);
                                    format!("已确认提醒: {} ({})", rtype, eid)
                                }
                                "unack" => {
                                    let _ = database::run("DELETE FROM reminder_ack WHERE reminder_type=?1 AND entity_id=?2", &[json!(rtype), json!(eid)]);
                                    format!("已取消确认: {} ({})", rtype, eid)
                                }
                                _ => "未知操作。支持: ack, unack".to_string(),
                            };
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
                    let ans = call_llm_without_tools(&messages, &app_handle).await.unwrap_or_else(|e| { log::info!("[Agent] 流式总结失败: {e}"); format!("总结失败: {e}") });
                    let ws = session.generate_scratchpad();
                    if let Some(ref h) = app_handle { let _ = h.emit("agent:status", json!({"stage":"answer"})); let _ = h.emit("agent:final-answer", json!({"answer": ans})); }
                    return Ok(json!({"success":true,"answer":ans,"workspace":ws,"iterations":iters,"circuit_breaker":true,"promptTokens":pt,"completionTokens":ct,"tool_log":tool_log}));
                }

                // ★ 扣减预算（工具执行后，下一轮生效）
                budget -= search_cost;
                search_cost = ((search_cost as f64 * 1.2) as i32).max(1);
            }
            _ => { let ws = session.generate_scratchpad(); return Ok(json!({"success":true,"answer":format!("根据当前信息：\n\n{}",effective_ac),"workspace":ws,"iterations":iters,"promptTokens":pt,"completionTokens":ct,"tool_log":tool_log})); }
        }
    }
}
