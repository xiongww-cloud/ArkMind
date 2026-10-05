//! ext_tool_report：子 Agent 结构化上行报告（无副作用 ack）
//! - gaps：数据/信息缺口 → 主 Agent 自查补齐后续派，或随交付稿回贴「待补」
//! - confirmations：必须用户拍板/确认的问题 → 主 Agent 转 ext_tool_ask_user 卡片
//! 本 handler 只做校验归一与回执；消费与聚合在 delegate_agent::collect_reports + OutputClass::finalize。

use crate::services::agent_workflow::ToolResultEnvelope;
use serde_json::{json, Value};

pub(crate) const REPORT_TOOL: &str = "ext_tool_report";
const MAX_GAPS: usize = 5;
const MAX_CONFS: usize = 3;
const MAX_OPTS: usize = 6;
const MAX_LEN: usize = 120;

fn cap(s: &str) -> String {
    let t = s.trim();
    if t.chars().count() <= MAX_LEN { return t.to_string(); }
    t.chars().take(MAX_LEN).collect()
}

pub(crate) fn execute(a: &Value) -> String {
    let mut gaps: Vec<String> = a["gaps"].as_array()
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).map(cap).filter(|s| !s.is_empty()).collect())
        .unwrap_or_default();
    gaps.truncate(MAX_GAPS);
    let mut confirmations: Vec<Value> = Vec::new();
    if let Some(arr) = a["confirmations"].as_array() {
        for c in arr {
            let q = cap(c["question"].as_str().unwrap_or(""));
            if q.is_empty() { continue; }
            let options: Vec<String> = c["options"].as_array()
                .map(|o| o.iter().filter_map(|v| v.as_str()).map(cap).filter(|s| !s.is_empty()).take(MAX_OPTS).collect())
                .unwrap_or_default();
            confirmations.push(json!({"question": q, "options": options}));
            if confirmations.len() >= MAX_CONFS { break; }
        }
    }
    if gaps.is_empty() && confirmations.is_empty() {
        return ToolResultEnvelope {
            result_type: "agent_report_result".to_string(),
            data: json!({"success": false, "error": "上报为空：gaps 或 confirmations 至少给一条"}),
            summary: Some("无效上报：未提供任何 gaps 或 confirmations。".to_string()),
        }.to_json();
    }
    ToolResultEnvelope {
        result_type: "agent_report_result".to_string(),
        data: json!({"success": true, "recorded": {"gaps": gaps, "confirmations": confirmations}}),
        summary: Some(format!(
            "已记录 {} 条缺口、{} 条确认请求，任务结束统一转达——无需再用文字标记重复上报这些事项。",
            gaps.len(), confirmations.len()
        )),
    }.to_json()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_caps_and_rejects_empty() {
        let ok: Value = serde_json::from_str(&execute(&json!({
            "gaps": ["  缺预算  ", "x".repeat(300), ""],
            "confirmations": [{"question": "架构路线？", "options": ["X86", "ARM", ""]}, {"question": " "}],
        }))).unwrap();
        assert_eq!(ok["result_type"], "agent_report_result");
        assert_eq!(ok["data"]["recorded"]["gaps"].as_array().unwrap().len(), 2);
        assert_eq!(ok["data"]["recorded"]["gaps"][0], "缺预算");
        assert_eq!(ok["data"]["recorded"]["gaps"][1].as_str().unwrap().chars().count(), 120);
        let confs = ok["data"]["recorded"]["confirmations"].as_array().unwrap();
        assert_eq!(confs.len(), 1);
        assert_eq!(confs[0]["options"].as_array().unwrap().len(), 2);
        // 全空 → 无效回执
        let bad: Value = serde_json::from_str(&execute(&json!({"gaps": []}))).unwrap();
        assert_eq!(bad["data"]["success"], false);
    }
}
