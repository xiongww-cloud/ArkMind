//! 执行计划状态机：`ext_tool_plan` 的后端唯一事实源。
//! 快照持久化于 `SessionMemory.plan`（chat_sessions.session_state），跨 run 恢复——
//! 追问续答 / 预算熔断续聊不断链；撤回断代置 session_state=NULL 时计划同灭（生命周期同会话记忆）。
//! 前端经 `agent:plan` 快照事件纯渲染（Dock 增删改即刷新）；模型经工具返回值回显自省。

use serde_json::Value;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PlanStep {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum StepState {
    #[serde(rename = "pending")]
    Pending,
    #[serde(rename = "active")]
    Active,
    #[serde(rename = "done")]
    Done,
    #[serde(rename = "failed")]
    Failed,
}

impl StepState {
    fn as_str(self) -> &'static str {
        match self {
            StepState::Pending => "pending",
            StepState::Active => "active",
            StepState::Done => "done",
            StepState::Failed => "failed",
        }
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PlanSnapshot {
    pub title: String,
    pub steps: Vec<PlanStep>,
    pub states: Vec<StepState>,
}

/// 每步工具轮软上限（达到即在动态上下文注入收敛指引）
pub const STEP_TOOL_SOFT_CAP: i32 = 10;
/// create 时按步数放大的预算额度（几何扣费保留，总额随计划规模上调）
pub const PLAN_BUDGET_PER_STEP: i32 = 45;

impl PlanSnapshot {
    pub fn done_count(&self) -> usize {
        self.states.iter().filter(|s| **s == StepState::Done).count()
    }

    pub fn all_done(&self) -> bool {
        !self.states.is_empty() && self.states.iter().all(|s| *s == StepState::Done)
    }

    /// 工具轮且无在途步 → 点亮最早 pending（检索型步骤同样有"进行中"态，进度提示不失真）。
    /// 已有 active 时幂等 no-op。返回是否点亮。
    pub fn begin_next_pending(&mut self) -> bool {
        if self.states.contains(&StepState::Active) { return false }
        if let Some(i) = self.states.iter().position(|s| *s == StepState::Pending) {
            self.states[i] = StepState::Active;
            return true;
        }
        false
    }

    /// 委托开始（保险丝）：hint（Agent 名互含）命中的最早 pending 步 → active；
    /// 无命中 → 最早 pending。模型显式 update 为主，此为兜底。
    pub fn advance_on_delegate(&mut self, agent_name: &str) {
        let n = agent_name.trim().to_lowercase();
        let match_hint = |i: usize| -> bool {
            let a = match &self.steps[i].agent {
                Some(a) => a.trim().to_lowercase(),
                None => return false,
            };
            if a.is_empty() { return false }
            n.contains(&a) || a.contains(&n)
        };
        let mut idx = self.steps.iter().enumerate()
            .find(|(i, _)| self.states[*i] == StepState::Pending && match_hint(*i))
            .map(|(i, _)| i);
        if idx.is_none() {
            idx = self.states.iter().position(|s| *s == StepState::Pending);
        }
        if let Some(i) = idx {
            self.states[i] = StepState::Active;
        }
    }

    /// 委托成功返回（保险丝）：当前 active 步落定 done。返回是否落定。
    pub fn complete_active(&mut self) -> bool {
        if let Some(i) = self.states.iter().position(|s| *s == StepState::Active) {
            self.states[i] = StepState::Done;
            return true;
        }
        false
    }

    pub fn to_payload(&self) -> Value {
        let items: Vec<Value> = self.steps.iter().enumerate().map(|(i, st)| {
            let mut m = serde_json::Map::new();
            m.insert("title".into(), Value::String(st.title.clone()));
            m.insert("agent".into(), Value::String(st.agent.clone().unwrap_or_default()));
            m.insert("state".into(), Value::String(self.states[i].as_str().to_string()));
            Value::Object(m)
        }).collect();
        let mut root = serde_json::Map::new();
        root.insert("title".into(), Value::String(self.title.clone()));
        root.insert("steps".into(), Value::Array(items));
        Value::Object(root)
    }

    /// 工具返回值回显文本（模型自省当前清单）
    pub fn render_list(&self) -> String {
        let mut s = format!("「{}」共 {} 步：", self.title, self.steps.len());
        for (i, st) in self.steps.iter().enumerate() {
            let mark = match self.states[i] {
                StepState::Pending => "…",
                StepState::Active => "▶",
                StepState::Done => "✓",
                StepState::Failed => "✗",
            };
            let agent = st.agent.as_deref().filter(|a| !a.is_empty()).map(|a| format!("（{}）", a)).unwrap_or_default();
            s.push_str(&format!("\n{}. [{}] {}{}", i + 1, mark, st.title, agent));
        }
        s
    }
}

/// create 参数校验与构建：title 必填、steps 1..=10、每步 title 必填（≤120 字）、agent 可选（≤20 字）
pub fn create_from_args(args: &Value) -> Result<PlanSnapshot, String> {
    let title = args["title"].as_str().map(|t| t.trim()).filter(|t| !t.is_empty())
        .ok_or("create 需要 title（计划标题）")?;
    if title.chars().count() > 120 { return Err("title 过长（≤120 字）".into()); }
    let arr = args["steps"].as_array().ok_or("create 需要 steps 数组（1-10 步）")?;
    if arr.is_empty() || arr.len() > 10 { return Err("steps 须为 1-10 步".into()); }
    let mut steps = Vec::new();
    for s in arr {
        let t = s["title"].as_str().map(|x| x.trim()).filter(|x| !x.is_empty())
            .ok_or("每步需要 title（动词开头，说清做什么）")?;
        if t.chars().count() > 120 { return Err("步骤 title 过长".into()); }
        let agent = s["agent"].as_str().map(|a| a.trim().chars().take(20).collect::<String>())
            .filter(|a| !a.is_empty());
        steps.push(PlanStep { title: t.chars().take(120).collect(), agent });
    }
    Ok(PlanSnapshot { title: title.chars().take(120).collect(), states: vec![StepState::Pending; steps.len()], steps })
}

/// update 应用：step 1-based；state ∈ active/done/failed（不允许直接改回 pending——进度只向前）
pub fn apply_update(snap: &mut PlanSnapshot, step: i64, state: &str) -> Result<(), String> {
    let n = snap.steps.len() as i64;
    if step < 1 || step > n { return Err(format!("step 序号越界：{}（当前共 {} 步）", step, n)); }
    let st = match state {
        "active" => StepState::Active,
        "done" => StepState::Done,
        "failed" => StepState::Failed,
        _ => return Err("state 须为 active | done | failed".into()),
    };
    let i = (step - 1) as usize;
    if snap.states[i] == StepState::Done { return Err(format!("第 {} 步已完成，不可再变更", step)); }
    snap.states[i] = st;
    Ok(())
}

/// 有 plan 时的总轮次硬上限：每步均摊 4 轮 + 收尾 4，钳位 [20,40]
pub fn max_iters_for(plan_steps: Option<usize>, current: i32) -> i32 {
    match plan_steps {
        Some(n) => current.max(((n as i32) * 4 + 4).clamp(20, 40)),
        None => current,
    }
}

/// 工具执行结果：信封 + create 信号（驱动预算放大与轮上限重算）
pub struct ToolOutcome {
    pub envelope: String,
    pub created_steps: Option<usize>,
}

fn envelope(action: &str, summary: String, data: Value) -> String {
    let mut m = serde_json::Map::new();
    m.insert("result_type".into(), Value::String("plan_result".into()));
    m.insert("data".into(), {
        let mut d = serde_json::Map::new();
        d.insert("action".into(), Value::String(action.into()));
        d.insert("success".into(), Value::Bool(true));
        for (k, v) in data.as_object().into_iter().flat_map(|o| o.iter()) {
            d.insert(k.clone(), v.clone());
        }
        Value::Object(d)
    });
    m.insert("summary".into(), Value::String(summary));
    Value::Object(m).to_string()
}

fn envelope_err(action: &str, msg: &str) -> String {
    let mut m = serde_json::Map::new();
    m.insert("result_type".into(), Value::String("plan_result".into()));
    m.insert("data".into(), serde_json::json!({"action": action, "success": false, "error": msg}));
    m.insert("summary".into(), Value::String(format!("计划操作失败：{}", msg)));
    Value::Object(m).to_string()
}

/// `ext_tool_plan` 执行：create / update / delete / list。
/// 状态机唯一写入口（模型显式操作）；委托/工具轮事件兜底在 workflow 侧。
pub fn execute_tool(args: &Value, session: &mut crate::services::agent_workflow::session::SessionMemory) -> ToolOutcome {
    let action = args["action"].as_str().unwrap_or("").trim().to_string();
    match action.as_str() {
        "create" => match create_from_args(args) {
            Ok(snap) => {
                let summary = format!("计划已建立：{}", snap.render_list());
                let n = snap.steps.len();
                let payload = snap.to_payload();
                session.plan = Some(snap);
                ToolOutcome { envelope: envelope("create", summary, serde_json::json!({"plan": payload})), created_steps: Some(n) }
            }
            Err(e) => ToolOutcome { envelope: envelope_err("create", &e), created_steps: None },
        },
        "update" => {
            let step = args["step"].as_i64().unwrap_or(0);
            let state = args["state"].as_str().unwrap_or("");
            let note = args["note"].as_str().unwrap_or("").trim();
            let Some(snap) = session.plan.as_mut() else {
                return ToolOutcome { envelope: envelope_err("update", "当前没有计划——请先 create"), created_steps: None };
            };
            match apply_update(snap, step, state) {
                Ok(()) => {
                    let note_suffix = if note.is_empty() { String::new() } else { format!("（{}）", note) };
                    let summary = format!("第 {} 步 → {}{}。当前：{}", step, state, note_suffix, snap.render_list());
                    let payload = snap.to_payload();
                    ToolOutcome { envelope: envelope("update", summary, serde_json::json!({"plan": payload})), created_steps: None }
                }
                Err(e) => ToolOutcome { envelope: envelope_err("update", &e), created_steps: None },
            }
        }
        "delete" => {
            if session.plan.is_none() {
                return ToolOutcome { envelope: envelope_err("delete", "当前没有计划"), created_steps: None };
            }
            session.plan = None;
            ToolOutcome { envelope: envelope("delete", "计划已清空。".to_string(), serde_json::json!({"plan": null})), created_steps: None }
        }
        "list" => {
            match session.plan.as_ref() {
                Some(snap) => ToolOutcome { envelope: envelope("list", snap.render_list(), serde_json::json!({"plan": snap.to_payload()})), created_steps: None },
                None => ToolOutcome { envelope: envelope("list", "当前没有计划。".to_string(), serde_json::json!({"plan": null})), created_steps: None },
            }
        }
        _ => ToolOutcome { envelope: envelope_err("", "action 须为 create | update | delete | list"), created_steps: None },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap2() -> PlanSnapshot {
        let v: Value = serde_json::from_str(
            r#"{"title":"投标准备","steps":[{"title":"梳理参数缺口","agent":"投标助手"},{"title":"编制方案正文"},{"title":"生成对照表","agent":"演示与材料"}]}"#,
        ).unwrap();
        create_from_args(&v).unwrap()
    }

    #[test]
    fn create_validates_bounds() {
        let ok: Value = serde_json::from_str(r#"{"title":"t","steps":[{"title":"a"}]}"#).unwrap();
        assert!(create_from_args(&ok).is_ok());
        let empty: Value = serde_json::from_str(r#"{"title":"t","steps":[]}"#).unwrap();
        assert!(create_from_args(&empty).is_err());
        let no_title: Value = serde_json::from_str(r#"{"steps":[{"title":"a"}]}"#).unwrap();
        assert!(create_from_args(&no_title).is_err());
        let no_step_title: Value = serde_json::from_str(r#"{"title":"t","steps":[{}]}"#).unwrap();
        assert!(create_from_args(&no_step_title).is_err());
        let too_many: Value = serde_json::from_str(
            r#"{"title":"t","steps":[{"title":"a"},{"title":"b"},{"title":"c"},{"title":"d"},{"title":"e"},{"title":"f"},{"title":"g"},{"title":"h"},{"title":"i"},{"title":"j"},{"title":"k"}]}"#,
        ).unwrap();
        assert!(create_from_args(&too_many).is_err());
    }

    #[test]
    fn update_matrix() {
        let mut p = snap2();
        assert!(apply_update(&mut p, 1, "active").is_ok());
        assert_eq!(p.states[0], StepState::Active);
        assert!(apply_update(&mut p, 1, "done").is_ok());
        // 已 done 不可变更
        assert!(apply_update(&mut p, 1, "failed").is_err());
        // 越界
        assert!(apply_update(&mut p, 0, "done").is_err());
        assert!(apply_update(&mut p, 4, "done").is_err());
        // 非法 state
        assert!(apply_update(&mut p, 2, "pending").is_err());
        // failed 合法
        assert!(apply_update(&mut p, 2, "failed").is_ok());
        assert_eq!(p.states[1], StepState::Failed);
    }

    #[test]
    fn hint_match_wins_over_earliest_pending() {
        let mut p = snap2();
        p.advance_on_delegate("演示与材料");
        assert_eq!(p.states[2], StepState::Active);
        p.advance_on_delegate("投标助手");
        assert_eq!(p.states[0], StepState::Active);
    }

    #[test]
    fn no_hint_falls_to_earliest_and_complete() {
        let mut p = snap2();
        p.advance_on_delegate("未声明 Agent");
        assert_eq!(p.states[0], StepState::Active);
        assert!(p.complete_active());
        assert_eq!(p.states[0], StepState::Done);
        assert!(!p.complete_active());
        p.advance_on_delegate("任意");
        assert_eq!(p.states[1], StepState::Active);
    }

    #[test]
    fn same_agent_multi_steps_advance_in_order() {
        // 投标助手负责第 1、3 步：done 后再委托同名 → 必须推进第 3 步（历史 guard 版会卡死）
        let v: Value = serde_json::from_str(
            r#"{"title":"t","steps":[{"title":"a","agent":"投标助手"},{"title":"b","agent":"方案"},{"title":"c","agent":"投标助手"}]}"#,
        ).unwrap();
        let mut p = create_from_args(&v).unwrap();
        p.advance_on_delegate("投标助手");
        assert_eq!(p.states[0], StepState::Active);
        assert!(p.complete_active());
        p.advance_on_delegate("方案");
        assert_eq!(p.states[1], StepState::Active);
        assert!(p.complete_active());
        p.advance_on_delegate("投标助手");
        assert_eq!(p.states[2], StepState::Active); // 同名已有 done 不拦后续 pending
    }

    #[test]
    fn search_only_plan_advances_via_begin_and_complete() {
        let mut p = snap2();
        assert!(p.begin_next_pending());
        assert!(!p.begin_next_pending()); // 幂等
        assert!(p.complete_active());
        assert!(p.begin_next_pending());
        assert!(p.complete_active());
        assert!(p.begin_next_pending());
        assert!(p.complete_active());
        assert!(p.all_done());
    }

    #[test]
    fn max_iters_scales_with_plan() {
        assert_eq!(max_iters_for(None, 20), 20);
        assert_eq!(max_iters_for(Some(6), 20), 28);
        assert_eq!(max_iters_for(Some(10), 20), 40);
    }

    #[test]
    fn payload_and_render_shape() {
        let mut p = snap2();
        p.advance_on_delegate("投标助手");
        let v = p.to_payload();
        assert_eq!(v["steps"][0]["state"].as_str(), Some("active"));
        assert_eq!(v["steps"][1]["state"].as_str(), Some("pending"));
        let text = p.render_list();
        assert!(text.contains("「投标准备」共 3 步"));
        assert!(text.contains("1. [▶] 梳理参数缺口（投标助手）"));
    }

    #[test]
    fn snapshot_serde_roundtrip() {
        let mut p = snap2();
        p.states[0] = StepState::Done;
        p.states[1] = StepState::Failed;
        let json = serde_json::to_string(&p).unwrap();
        let back: PlanSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
        assert_eq!(back.states[1], StepState::Failed);
    }

    #[test]
    fn tool_execute_lifecycle() {
        let mut session = crate::services::agent_workflow::session::SessionMemory::new();
        // list 空
        let o = execute_tool(&serde_json::json!({"action":"list"}), &mut session);
        assert!(o.envelope.contains("没有计划"));
        // create
        let o = execute_tool(&serde_json::json!({"action":"create","title":"t","steps":[{"title":"a"},{"title":"b"}]}), &mut session);
        assert_eq!(o.created_steps, Some(2));
        assert!(session.plan.is_some());
        // update
        let o = execute_tool(&serde_json::json!({"action":"update","step":1,"state":"done"}), &mut session);
        assert!(o.envelope.contains("done"));
        assert_eq!(session.plan.as_ref().unwrap().states[0], StepState::Done);
        // update 越界报错
        let o = execute_tool(&serde_json::json!({"action":"update","step":9,"state":"done"}), &mut session);
        assert!(o.envelope.contains("越界"));
        // delete
        let o = execute_tool(&serde_json::json!({"action":"delete"}), &mut session);
        assert!(session.plan.is_none());
        // delete 再删报错
        let o = execute_tool(&serde_json::json!({"action":"delete"}), &mut session);
        assert!(o.envelope.contains("没有计划"));
    }
}
