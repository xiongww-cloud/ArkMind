//! 工作流计划状态机：plan 围栏解析、委托驱动的步推进、按步预算/轮次上限计算。
//! 唯一事实源在后端；前端经 agent:plan 事件纯渲染（Dock），不再本地匹配推进。

use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub struct PlanStep {
    pub title: String,
    pub agent: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StepState {
    Pending,
    Active,
    Done,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WorkflowPlan {
    pub title: String,
    pub steps: Vec<PlanStep>,
    pub states: Vec<StepState>,
}

/// 每步工具轮软上限（达到即注入收敛指令，max_iters 为硬网）
pub const STEP_TOOL_SOFT_CAP: i32 = 10;

/// 从答复文本中探测 plan 围栏：行首 ```ui-block，次行单行 JSON（type=plan）。
/// 格式不合/校验不过 → None（当普通文本，不报错）。
pub fn plan_from_text(text: &str) -> Option<WorkflowPlan> {
    if !text.contains("```ui-block") {
        return None;
    }
    let lines: Vec<&str> = text.split('\n').collect();
    let mut i = 0usize;
    while i < lines.len() {
        if lines[i].trim_start().starts_with("```ui-block") {
            if i + 1 < lines.len() {
                let candidate = lines[i + 1].trim();
                if let Ok(v) = serde_json::from_str::<Value>(candidate) {
                    if v["type"].as_str() == Some("plan") {
                        if let Some(p) = parse_plan(&v) {
                            return Some(p);
                        }
                    }
                }
            }
            i += 1;
            continue;
        }
        i += 1;
    }
    None
}

fn parse_plan(v: &Value) -> Option<WorkflowPlan> {
    let title = v["title"].as_str().filter(|t| !t.trim().is_empty())?.trim().to_string();
    let arr = v["steps"].as_array()?;
    if arr.is_empty() || arr.len() > 10 {
        return None;
    }
    let mut steps = Vec::new();
    for s in arr {
        let t = s["title"].as_str().filter(|x| !x.trim().is_empty())?.trim().chars().take(120).collect::<String>();
        let agent = s["agent"].as_str().map(|a| a.trim().chars().take(20).collect::<String>())
            .filter(|a| !a.is_empty());
        steps.push(PlanStep { title: t, agent });
    }
    let states = vec![StepState::Pending; steps.len()];
    Some(WorkflowPlan { title, steps, states })
}

impl WorkflowPlan {
    pub fn done_count(&self) -> usize {
        self.states.iter().filter(|s| **s == StepState::Done).count()
    }

    pub fn all_done(&self) -> bool {
        !self.states.is_empty() && self.states.iter().all(|s| *s == StepState::Done)
    }

    /// 委托开始：hint（Agent 显示名互含）命中的最早 pending 步 → active；
    /// 无命中 → 最早 pending 步 → active；已无 pending 保持不变（一步多委托）。
    pub fn advance_on_delegate(&mut self, agent_name: &str) {
        let n = agent_name.trim().to_lowercase();
        let match_hint = |i: usize| -> bool {
            let a = match &self.steps[i].agent {
                Some(a) => a.trim().to_lowercase(),
                None => return false,
            };
            if a.is_empty() { return false }
            n.contains(&a) || a.contains(&n) || (n.is_empty() == false && a == n)
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

    /// 委托成功返回：当前 active 步落定 done
    pub fn complete_active(&mut self) -> bool {
        if let Some(i) = self.states.iter().position(|s| *s == StepState::Active) {
            self.states[i] = StepState::Done;
            return true;
        }
        false
    }

    pub fn to_payload(&self) -> Value {
        let items: Vec<Value> = self.steps.iter().enumerate().map(|(i, st)| {
            let state = match self.states[i] {
                StepState::Pending => "pending",
                StepState::Active => "active",
                StepState::Done => "done",
            };
            let mut m = serde_json::Map::new();
            m.insert("title".into(), Value::String(st.title.clone()));
            m.insert("agent".into(), Value::String(st.agent.clone().unwrap_or_default()));
            m.insert("state".into(), Value::String(state.to_string()));
            Value::Object(m)
        }).collect();
        let mut root = serde_json::Map::new();
        root.insert("title".into(), Value::String(self.title.clone()));
        root.insert("steps".into(), Value::Array(items));
        Value::Object(root)
    }
}

/// 有 plan 时的总轮次硬上限：每步均摊 4 轮 + 收尾 4，钳位 [20,40]
pub fn max_iters_for(plan_steps: Option<usize>, current: i32) -> i32 {
    match plan_steps {
        Some(n) => current.max(((n as i32) * 4 + 4).clamp(20, 40)),
        None => current,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FENCED: &str = "先梳理缺口。\n```ui-block\n{\"type\":\"plan\",\"title\":\"投标准备\",\"steps\":[{\"title\":\"梳理参数缺口\",\"agent\":\"投标助手\"},{\"title\":\"编制方案正文\"},{\"title\":\"生成对照表\",\"agent\":\"演示与材料\"}]}\n```\n开始执行。";

    #[test]
    fn parses_plan_fence() {
        let p = plan_from_text(FENCED).expect("parse");
        assert_eq!(p.title, "投标准备");
        assert_eq!(p.steps.len(), 3);
        assert_eq!(p.steps[0].agent.as_deref(), Some("投标助手"));
        assert_eq!(p.states, vec![StepState::Pending; 3]);
    }

    #[test]
    fn rejects_non_plan_and_malformed() {
        assert!(plan_from_text("```ui-block\n{\"type\":\"form\",\"title\":\"x\",\"fields\":[{\"name\":\"a\",\"label\":\"甲\",\"kind\":\"text\"}]}\n```").is_none());
        assert!(plan_from_text("```ui-block\n{bad json\n```").is_none());
        assert!(plan_from_text("```ui-block\n{\"type\":\"plan\",\"title\":\"t\",\"steps\":[]}\n```").is_none());
        assert!(plan_from_text("没有围栏").is_none());
    }

    #[test]
    fn hint_match_wins_over_earliest_pending() {
        let mut p = plan_from_text(FENCED).unwrap();
        p.advance_on_delegate("演示与材料");
        assert_eq!(p.states[2], StepState::Active);
        p.advance_on_delegate("投标助手");
        assert_eq!(p.states[0], StepState::Active);
        assert_eq!(p.done_count(), 0);
    }

    #[test]
    fn no_hint_falls_to_earliest_pending_and_complete() {
        let mut p = plan_from_text(FENCED).unwrap();
        p.advance_on_delegate("某个未声明的 Agent");
        assert_eq!(p.states[0], StepState::Active);
        assert!(p.complete_active());
        assert_eq!(p.states[0], StepState::Done);
        assert!(!p.complete_active()); // 无 active：no-op
        p.advance_on_delegate("任意"); // 无 pending hint 命中 → 最早 pending = idx1
        assert_eq!(p.states[1], StepState::Active);
    }

    #[test]
    fn redeclaration_resets() {
        let mut p = plan_from_text(FENCED).unwrap();
        p.advance_on_delegate("投标助手");
        p.complete_active();
        let fresh = plan_from_text(FENCED).unwrap();
        assert_eq!(fresh.states, vec![StepState::Pending; 3]);
        assert_eq!(p.done_count(), 1);
    }

    #[test]
    fn same_agent_multi_steps_advance_in_order() {
        // 投标助手负责第 1、3 步：done 后再委托同名 → 必须推进第 3 步（历史 guard 版会卡死）
        let fenced = "{\"type\":\"plan\",\"title\":\"t\",\"steps\":[{\"title\":\"a\",\"agent\":\"投标助手\"},{\"title\":\"b\",\"agent\":\"方案\"},{\"title\":\"c\",\"agent\":\"投标助手\"}]}";
        let v: serde_json::Value = serde_json::from_str(fenced).unwrap();
        let mut p = parse_plan(&v).unwrap();
        p.advance_on_delegate("投标助手");
        assert_eq!(p.states[0], StepState::Active);
        assert!(p.complete_active());
        p.advance_on_delegate("方案");
        assert_eq!(p.states[1], StepState::Active);
        assert!(p.complete_active());
        p.advance_on_delegate("投标助手");
        assert_eq!(p.states[2], StepState::Active); // 关键：同名已有一步 done 不拦后续
    }

    #[test]
    fn max_iters_scales_with_plan() {
        assert_eq!(max_iters_for(None, 20), 20);
        assert_eq!(max_iters_for(Some(2), 20), 20); // 4*2+4=12 → clamp 20
        assert_eq!(max_iters_for(Some(6), 20), 28);
        assert_eq!(max_iters_for(Some(10), 20), 40); // 44 → clamp 40
    }

    #[test]
    fn payload_shape() {
        let mut p = plan_from_text(FENCED).unwrap();
        p.advance_on_delegate("投标助手");
        let v = p.to_payload();
        assert_eq!(v["steps"][0]["state"].as_str(), Some("active"));
        assert_eq!(v["steps"][1]["state"].as_str(), Some("pending"));
    }
}
