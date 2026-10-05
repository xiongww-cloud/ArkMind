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

/// 交付型协议：回答即交付文档 + 禁 MISSING + NEED_USER 确认通道
const DELIVERABLE_PROTOCOL: &str = "## 执行协议（交付型产出）\n- 你的回答就是最终交付文档，将原样、完整地呈现给用户（主 Agent 不复述、你也没有向用户提问的工具），请按角色提示词的结构、篇幅与风格直接成文；本协议优先于角色提示词中的 MISSING/追问类规则\n- 不使用 MISSING: 行（会破坏交付直出）；信息缺口在相应处内联标注（待补：xxx）或文末列「【待补/需确认】」\n- 确需用户确认/拍板时（典型：规划遍的框架确认），在回答最末单起一行输出 NEED_USER: <问题> | <选项A> / <选项B>（选项可省略）——该行会被系统剥出并交主 Agent 转成追问卡片，不会进入文档正文；成文遍原则上不用（按合理假设+★待校核继续），仅出现必须用户拍板的分支决策时使用\n- 同一任务的多个独立工具调用必须在一次回复中并发发起；查询结果全程复用，需要补充信息时换角度查询，不要原样重查\n- 若工具返回\"未授权\"或\"工具未找到\"，跳过该操作，用已有信息继续完成任务\n- 按照 system_prompt 中的「回答规范」输出，不要输出内部 ID";

const NATURAL_PROTOCOL: &str = "## 执行协议\n- 同一任务的多个独立操作，必须在一次回复中并发调用多个工具，不要拆成多轮\n- 查询结果在同一任务内全程复用：需要补充信息时换角度查询，不要原样重查\n- 创建任何业务对象前，必须先用 ext_tool_object 按名称检索同类对象：完全同名由工具层自动复用；存在相似项且不确定是否同一目标时，不要猜测创建，输出 MISSING 列出候选请确认\n- 拿不准的判断点不要中途停摆、不要编造：独立操作照常完成，在回复的最后一行输出 MISSING 向主 Agent 提问（它会作答或转问用户）；有可枚举候选时附带：MISSING: <问题> | <选项A> / <选项B>\n- 若工具返回\"未授权\"或\"工具未找到\"，跳过该操作，用已有的信息继续完成任务，不要输出 MISSING\n- 按照 system_prompt 中的「回答规范」向用户汇报结果，不要输出内部 ID";

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
    /// 结果终裁：返回 (正文, status, 生效 missing)。
    /// 交付型：NEED_USER→partial（问题交主转卡片，杂散 MISSING 回贴）；杂散 MISSING→回贴保持 completed；
    /// 执行型：维持 missing→partial 三态。全失败无产出恒 failed（NEED_USER 不豁免）。
    pub fn finalize(self, mut text: String, missing: &[String], questions: &[String], any_failed: bool, has_artifacts: bool) -> (String, &'static str, Vec<String>) {
        let append_stray = |text: &mut String, stray: &[String]| {
            let block = stray.iter().map(|m| format!("- {}", m)).collect::<Vec<_>>().join("\n");
            text.push_str(&format!("\n\n【待补/需确认】\n{}", block));
        };
        match self {
            OutputClass::Deliverable if !questions.is_empty() => {
                if any_failed && !has_artifacts { return (text, "failed", vec![]); }
                if !missing.is_empty() { append_stray(&mut text, missing); }
                (text, "partial", questions.to_vec())
            }
            OutputClass::Deliverable if !missing.is_empty() => {
                if any_failed && !has_artifacts { return (text, "failed", vec![]); }
                append_stray(&mut text, missing);
                (text, "completed", vec![])
            }
            _ => {
                let status = if !missing.is_empty() || !questions.is_empty() { "partial" }
                    else if any_failed && !has_artifacts { "failed" }
                    else { "completed" };
                let mut eff: Vec<String> = missing.to_vec();
                eff.extend(questions.iter().cloned());
                (text, status, eff)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::OutputClass;

    #[test]
    fn deliverable_needs_user_goes_partial() {
        let q = vec!["确认框架？| 确认 / 调整".to_string()];
        let (t, s, eff) = OutputClass::Deliverable.finalize("框架".into(), &[], &q, false, false);
        assert_eq!((s, eff.as_slice()), ("partial", q.as_slice()));
        assert_eq!(t, "框架");
    }
    #[test]
    fn deliverable_stray_missing_keeps_completed() {
        let miss = vec!["缺预算".into()];
        let (t, s, eff) = OutputClass::Deliverable.finalize("正文".into(), &miss, &[], false, false);
        assert_eq!(s, "completed");
        assert!(eff.is_empty() && t.contains("【待补/需确认】") && t.contains("- 缺预算"));
    }
    #[test]
    fn natural_semantics_unchanged_and_failures_wins() {
        let miss = vec!["缺X".into()];
        let (_, s, eff) = OutputClass::Natural.finalize("文".into(), &miss, &[], false, true);
        assert_eq!(s, "partial");
        assert_eq!(eff, miss);
        let q = vec!["?".into()];
        assert_eq!(OutputClass::Deliverable.finalize("x".into(), &miss, &q, true, false).1, "failed");
        assert_eq!(OutputClass::Natural.finalize("x".into(), &[], &[], true, false).1, "failed");
        assert_eq!(OutputClass::from_format("weird"), OutputClass::Natural);
    }
}
