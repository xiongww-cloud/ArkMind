//! 子 Agent 产出类别（output_format 列的语义化枚举）——输出行为策略的单一落点。
//! 新增类别 = 加一个变体并补齐三个方法，delegate/workflow/UI 零散点自动收敛。
use serde::Serialize;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OutputClass {
    /// 执行型：结果交回主 Agent 综合后答复；MISSING→partial 走自答/转问闭环
    Natural,
    /// 交付型：回答即终稿，合格时系统直出给用户；禁 MISSING、NEED_USER 转追问卡片
    Deliverable,
}

pub const ALL_OUTPUT_CLASSES: [OutputClass; 2] = [OutputClass::Natural, OutputClass::Deliverable];

/// 交付型协议：回答即交付文档 + ext_tool_report 结构化上报（禁文字标记暗号）
const DELIVERABLE_PROTOCOL: &str = "## 执行协议（交付型产出）\n- 你的回答就是最终交付文档，将原样、完整地呈现给用户（主 Agent 不复述、你也没有向用户提问的工具），请按角色提示词的结构、篇幅与风格直接成文；本协议优先于角色提示词中的 MISSING/追问类规则\n- 不要使用 MISSING:/NEED_USER: 等文字标记（会破坏交付直出）；信息缺口调用 ext_tool_report 上报 gaps——系统会在文末回贴「【待补/需确认】」清单，不阻塞你成文\n- 确需用户确认/拍板时（典型：规划遍的框架确认），调用 ext_tool_report 上报 confirmations（question+options）——结束后可转成追问卡片，不会混入文档正文；成文遍原则上不用（按合理假设+★待校核继续），仅出现必须用户拍板的分支决策时使用\n- 同一任务的多个独立工具调用必须在一次回复中并发发起；查询结果全程复用，需要补充信息时换角度查询，不要原样重查\n- 若工具返回\"未授权\"或\"工具未找到\"，跳过该操作，用已有信息继续完成任务\n- 按照 system_prompt 中的「回答规范」输出，不要输出内部 ID";

/// 执行型协议：结果交主 Agent 综合；缺口/确认走 ext_tool_report 结构化上报
const NATURAL_PROTOCOL: &str = "## 执行协议\n- 同一任务的多个独立操作，必须在一次回复中并发调用多个工具，不要拆成多轮\n- 查询结果在同一任务内全程复用：需要补充信息时换角度查询，不要原样重查\n- 创建任何业务对象前，必须先用 ext_tool_object 按名称检索同类对象：完全同名由工具层自动复用；存在相似项且不确定是否同一目标时，不要猜测创建，调用 ext_tool_report 上报 confirmations（附候选）请用户确认\n- 拿不准的判断点不要中途停摆、不要编造：独立操作照常完成；数据缺口调用 ext_tool_report 上报 gaps（主 Agent 会自查补齐并续派，或转问用户），必须用户拍板的放 confirmations；不要在正文里写 MISSING:/NEED_USER: 等文字标记\n- 若工具返回\"未授权\"或\"工具未找到\"，跳过该操作，用已有的信息继续完成任务\n- 按照 system_prompt 中的「回答规范」向用户汇报结果，不要输出内部 ID";

#[derive(Serialize)]
pub struct OutputClassInfo {
    pub key: &'static str,
    pub label: &'static str,
    pub desc: &'static str,
}

impl OutputClass {
    pub fn from_format(s: &str) -> Self {
        match s {
            "deliverable" => OutputClass::Deliverable,
            _ => OutputClass::Natural,
        }
    }
    pub fn key(self) -> &'static str {
        match self { OutputClass::Natural => "natural", OutputClass::Deliverable => "deliverable" }
    }
    pub fn label(self) -> &'static str {
        match self { OutputClass::Natural => "执行型（默认）", OutputClass::Deliverable => "交付型" }
    }
    pub fn desc(self) -> &'static str {
        match self {
            OutputClass::Natural => "结果交回主 Agent 综合后答复",
            OutputClass::Deliverable => "回答即终稿，直接完整呈现给用户（方案/报告类长文适用）",
        }
    }
    /// 注入主 Agent【可用专业 Agent】列表的行尾标记
    pub fn injection_tag(self) -> &'static str {
        match self {
            OutputClass::Natural => "",
            OutputClass::Deliverable => "\n  ⚑ 交付型：正文即终稿，直出交付用户，不得改写、不得代笔续写",
        }
    }
    /// 委托尾部指令的执行协议段
    pub fn directive_protocol(self) -> &'static str {
        match self { OutputClass::Natural => NATURAL_PROTOCOL, OutputClass::Deliverable => DELIVERABLE_PROTOCOL }
    }
    /// 结果终裁：返回 (正文, status, 生效 missing, confirmations)。
    /// 输入 = 旧文字标记（stray_missing/legacy_questions，LEGACY 过渡层）+ 结构化上报（rep_gaps/rep_confs）。
    /// 交付型：有确认请求→partial（直出被拒，交主转追问卡片）；缺口一律回贴「待补/需确认」清单不阻塞成文；
    /// 执行型：任何上报→partial，missing=并集（conf 串化为"问题 | 候选A / 候选B"）。
    /// 全失败无产出→failed（上报不豁免）。
    pub fn finalize(
        self,
        mut text: String,
        stray_missing: &[String],
        legacy_questions: &[String],
        rep_gaps: &[String],
        rep_confs: &[serde_json::Value],
        any_failed: bool,
        has_artifacts: bool,
    ) -> (String, &'static str, Vec<String>, Vec<serde_json::Value>) {
        fn uniq(it: impl IntoIterator<Item = String>) -> Vec<String> {
            let mut out: Vec<String> = Vec::new();
            for s in it { if !s.is_empty() && !out.iter().any(|e| e == &s) { out.push(s); } }
            out
        }
        fn conf_to_str(c: &serde_json::Value) -> String {
            let q = c["question"].as_str().unwrap_or("").to_string();
            let opts: Vec<String> = c["options"].as_array()
                .map(|a| a.iter().filter_map(|v| v.as_str()).map(String::from).collect())
                .unwrap_or_default();
            if opts.is_empty() { q } else { format!("{} | {}", q, opts.join(" / ")) }
        }
        fn append_block(text: &mut String, items: &[String]) {
            let block = items.iter().map(|m| format!("- {}", m)).collect::<Vec<_>>().join("\n");
            text.push_str(&format!("\n\n【待补/需确认】\n{}", block));
        }
        let gaps = uniq(stray_missing.iter().cloned().chain(rep_gaps.iter().cloned()));
        let questions = uniq(legacy_questions.iter().cloned().chain(rep_confs.iter().map(conf_to_str)));
        let failed_first = any_failed && !has_artifacts;
        match self {
            OutputClass::Deliverable => {
                if !questions.is_empty() {
                    if failed_first { return (text, "failed", vec![], vec![]); }
                    if !gaps.is_empty() { append_block(&mut text, &gaps); }
                    return (text, "partial", questions, rep_confs.to_vec());
                }
                if !gaps.is_empty() {
                    if failed_first { return (text, "failed", vec![], vec![]); }
                    append_block(&mut text, &gaps);
                    return (text, "completed", vec![], vec![]);
                }
                if failed_first { return (text, "failed", vec![], vec![]); }
                (text, "completed", vec![], vec![])
            }
            OutputClass::Natural => {
                let eff = uniq(stray_missing.iter().cloned()
                    .chain(legacy_questions.iter().cloned())
                    .chain(rep_gaps.iter().cloned())
                    .chain(rep_confs.iter().map(conf_to_str)));
                let status = if !eff.is_empty() { "partial" }
                    else if failed_first { "failed" }
                    else { "completed" };
                (text, status, eff, rep_confs.to_vec())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::OutputClass;
    use serde_json::json;

    #[test]
    fn deliverable_confirmation_report_goes_partial() {
        let confs = vec![json!({"question": "确认框架？", "options": ["确认", "调整"]})];
        let (t, s, eff, out_confs) = OutputClass::Deliverable.finalize("框架".into(), &[], &[], &[], &confs, false, false);
        assert_eq!(s, "partial");
        assert_eq!(eff, vec!["确认框架？ | 确认 / 调整".to_string()]);
        assert_eq!(out_confs, confs);
        assert_eq!(t, "框架");
    }
    #[test]
    fn deliverable_gaps_only_appends_and_stays_completed() {
        let (t, s, eff, _) = OutputClass::Deliverable.finalize(
            "正文".into(), &["旧杂散".into()], &[], &["缺预算".into()], &[], false, false);
        assert_eq!(s, "completed");
        assert!(eff.is_empty());
        assert!(t.contains("【待补/需确认】") && t.contains("- 旧杂散") && t.contains("- 缺预算"));
    }
    #[test]
    fn natural_unions_all_channels_and_failure_wins() {
        let confs = vec![json!({"question": "路线?", "options": []})];
        let (t, s, eff, _) = OutputClass::Natural.finalize("文".into(), &["缺X".into()], &["旧问".into()], &["缺Y".into()], &confs, false, true);
        assert_eq!(s, "partial");
        assert_eq!(eff, vec!["缺X".to_string(), "旧问".to_string(), "缺Y".to_string(), "路线?".to_string()]);
        assert_eq!(t, "文");
        assert_eq!(OutputClass::Deliverable.finalize("x".into(), &[], &["问".into()], &[], &[], true, false).1, "failed");
        assert_eq!(OutputClass::Natural.finalize("x".into(), &[], &[], &[], &[], true, false).1, "failed");
        assert_eq!(OutputClass::from_format("weird"), OutputClass::Natural);
    }
    #[test]
    fn dedup_across_legacy_and_report() {
        let confs = vec![json!({"question": "同一条", "options": []})];
        let (_, _, eff, _) = OutputClass::Natural.finalize("x".into(), &[], &["同一条".into()], &[], &confs, false, true);
        assert_eq!(eff.len(), 1);
    }
}
