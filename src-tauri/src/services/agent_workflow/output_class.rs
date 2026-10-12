//! 子 Agent 产出协议与终裁（2026-10 类型合并：执行型/交付型二分退役）。
//! 合并依据：合格的干净单委托本来就不分类直出（pick_direct_out 无类别）；二分只剩四轴差异，
//! 其中流通道已改由「本轮委托数」裁决（llm.rs suppress_body_stream），终裁语义统一如下——
//!   正文一律面向用户成文；缺口(gaps)→partial+missing 交回主 Agent 补齐续派/转问；
//!   全失败无产出→failed；干净→completed（单委托直出终稿）。
//! output_format 列/seed 保留为休眠元数据，不再驱动任何行为。

use serde_json::Value;

/// 统一执行协议：交付型协议为底，并入执行型独有的「创建前查重」条款；
/// 绝对断言改中性表述（多委托综合场景不存在"一定原样呈现"）。
pub const DELEGATE_PROTOCOL: &str = "## 执行协议（委托产出）\n- 你在为最终用户成文：合格的单委托正文将由系统原样直出交付；多委托综合时主 Agent 仅添加总述与结论、不改写你的正文。请按角色提示词的结构、篇幅与风格直接成文；本协议优先于角色提示词中的追问/上报类规则\n- 不要在正文写 MISSING:/NEED_USER: 等状态文字标记（系统不解析）；数据缺口通过「缺口与确认上报」通道登记——缺口会使结果被交回主 Agent 补齐续派或转问用户，成文所需关键数据缺失必须如实登记，禁止编造\n- 确需用户确认/拍板时（典型：规划遍的框架确认），通过「缺口与确认上报」通道登记确认事项（每条一句问题、可配 2-6 个预设选项）——系统会自动转为交互卡片，不会混入文档正文，主 Agent 也无需转述\n- 正文只含交付内容：一切对话控制（确认请求、继续引导、进度播报、完成提示）一律经「缺口与确认上报」的确认通道表达，由系统自动转为交互卡片——禁止在正文写【请确认】【回复「继续」】【进度】类对话句，否则会与卡片重复呈现\n- 创建任何业务对象前，必须先用「数据查询」按名称检索同类对象：完全同名由工具层自动复用；存在相似项且不确定是否同一目标时，不要猜测创建，通过「缺口与确认上报」登记确认事项（附候选）请用户确认\n- 任何输出给用户可见的文字只用中文业务称谓，禁止出现工具函数名（ext_tool_ 开头的标识符）与英文参数名\n- 同一任务的多个独立工具调用必须在一次回复中并发发起；查询结果全程复用，需要补充信息时换角度查询，不要原样重查\n- 若工具返回\"未授权\"或\"工具未找到\"，跳过该操作，用已有信息继续完成任务\n- 按照 system_prompt 中的「回答规范」输出，不要输出内部 ID";

/// 结果终裁（唯一分支）：返回 (正文, status, 生效 missing, confirmations)。
/// 输入 = ext_tool_report 结构化上报（文字标记解析层已删除）。
/// 全失败无产出→failed；有缺口→partial+missing 交回主 Agent（续派闭环）；干净→completed。
/// confirmations 恒原样带出，由框架自动转交互卡片，不影响 status。
pub fn finalize_delegate_output(
    mut text: String,
    rep_gaps: &[String],
    rep_confs: &[Value],
    any_failed: bool,
    has_artifacts: bool,
) -> (String, &'static str, Vec<String>, Vec<Value>) {
    let mut gaps: Vec<String> = Vec::new();
    for s in rep_gaps {
        if !s.is_empty() && !gaps.iter().any(|e| e == s) { gaps.push(s.clone()); }
    }
    if any_failed && !has_artifacts {
        return (text, "failed", vec![], vec![]);
    }
    if !gaps.is_empty() {
        return (text, "partial", gaps, rep_confs.to_vec());
    }
    (text, "completed", vec![], rep_confs.to_vec())
}

/// 委托返回信封分流（纯函数，可单测）：取消 > 硬失败（子链 LLM 调用失败信封）> 正常。
/// 工作流层 LLM 失败返回 Ok({"success":false,"answer":"LLM 调用失败:…"})——
/// 曾被子链当正常产出 make_success 包装成 completed，错误文本会以成品直出给用户。
pub fn classify_delegate_output(v: &Value) -> &'static str {
    if v["cancelled"].as_bool() == Some(true) {
        "cancelled"
    } else if v["success"].as_bool() == Some(false) {
        "failed"
    } else {
        "ok"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn finalize_three_way() {
        // 干净 → completed
        let (t, s, m, _) = finalize_delegate_output("正文".into(), &[], &[], false, true);
        assert_eq!((s, m.as_slice()), ("completed", &[] as &[String]));
        assert_eq!(t, "正文");
        // 有缺口（即使有产出）→ partial+missing 交回主 Agent
        let (_, s, m, _) = finalize_delegate_output("文".into(), &["缺X".to_string(), "缺X".to_string(), "".to_string()], &[], false, true);
        assert_eq!(s, "partial");
        assert_eq!(m, vec!["缺X".to_string()], "缺口去重且剔空");
        // 全失败无产出 → failed
        let (_, s, _, _) = finalize_delegate_output("x".into(), &[], &[], true, false);
        assert_eq!(s, "failed");
        // 部分工具失败但有产出 → 不算 failed（artifacts 豁免）
        let (_, s, _, _) = finalize_delegate_output("x".into(), &[], &[], true, true);
        assert_eq!(s, "completed");
    }

    #[test]
    fn finalize_confirmations_never_change_status() {
        let confs = vec![json!({"question": "路线?", "options": ["A", "B"]})];
        let (_, s, m, out) = finalize_delegate_output("文".into(), &[], &confs, false, true);
        assert_eq!(s, "completed");
        assert!(m.is_empty());
        assert_eq!(out, confs, "确认事项原样带出转卡片");
    }

    #[test]
    fn classify_cancel_beats_failure_flag() {
        assert_eq!(classify_delegate_output(&json!({"success": false, "cancelled": true})), "cancelled");
        assert_eq!(classify_delegate_output(&json!({"success": false, "answer": "LLM 调用失败: x"})), "failed");
        assert_eq!(classify_delegate_output(&json!({"success": true, "answer": "正常"})), "ok");
        assert_eq!(classify_delegate_output(&json!({})), "ok", "无 success 字段视为正常（旧信封兼容）");
    }

    #[test]
    fn protocol_covers_both_legacy_audiences() {
        assert!(DELEGATE_PROTOCOL.contains("正文只含交付内容"), "对话控制分离条款");
        assert!(DELEGATE_PROTOCOL.contains("禁止在正文写【请确认】"));
        assert!(DELEGATE_PROTOCOL.contains("完全同名由工具层自动复用"), "执行型查重条款须升入统一协议");
        assert!(DELEGATE_PROTOCOL.contains("不要在正文写 MISSING"), "禁文字标记");
        // 绝对断言已中性化（多委托综合场景存在）
        assert!(!DELEGATE_PROTOCOL.contains("将原样、完整地呈现给用户"), "不得再对正文做无条件直出断言");
    }
}
