//! 子 Agent 产出类别（output_format 列的语义化枚举）——输出行为策略的单一落点。
//! 新增类别 = 加一个变体并补齐三个方法，delegate/workflow/UI 零散点自动收敛。

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OutputClass {
    /// 执行型：结果交回主 Agent 综合后答复；gaps→partial 走自答续派闭环
    Natural,
    /// 交付型：回答即终稿，合格时系统直出给用户；confirmations 由框架自动转交互卡片
    Deliverable,
}

pub const ALL_OUTPUT_CLASSES: [OutputClass; 2] = [OutputClass::Natural, OutputClass::Deliverable];

/// 交付型协议：回答即交付文档 + ext_tool_report 结构化上报（禁文字标记暗号）
const DELIVERABLE_PROTOCOL: &str = "## 执行协议（交付型产出）\n- 你的回答就是最终交付文档，将原样、完整地呈现给用户（主 Agent 不复述、你也没有向用户提问的工具），请按角色提示词的结构、篇幅与风格直接成文；本协议优先于角色提示词中的追问/上报类规则\n- 不要在正文写 MISSING:/NEED_USER: 等状态文字标记（系统不解析，且会破坏交付直出）；信息缺口通过「缺口与确认上报」通道登记数据缺口——系统会在文末回贴「【待补/需确认】」清单，不阻塞你成文\n- 确需用户确认/拍板时（典型：规划遍的框架确认），通过「缺口与确认上报」通道登记确认事项（每条一句问题、可配 2-6 个预设选项）——系统会自动转为交互卡片，不会混入文档正文，主 Agent 也无需转述；成文遍原则上不用（按合理假设+★待校核继续），仅出现必须用户拍板的分支决策时使用\n- 正文只含交付内容：一切对话控制（确认请求、继续引导、进度播报、完成提示）一律经「缺口与确认上报」的确认通道表达，由系统自动转为交互卡片——禁止在正文写【请确认】【回复「继续」】【进度】类对话句，否则会与卡片重复呈现\n- 任何输出给用户可见的文字只用中文业务称谓，禁止出现工具函数名（ext_tool_ 开头的标识符）与英文参数名\n- 同一任务的多个独立工具调用必须在一次回复中并发发起；查询结果全程复用，需要补充信息时换角度查询，不要原样重查\n- 若工具返回\"未授权\"或\"工具未找到\"，跳过该操作，用已有信息继续完成任务\n- 按照 system_prompt 中的「回答规范」输出，不要输出内部 ID";

/// 执行型协议：结果交主 Agent 综合；缺口/确认走 ext_tool_report 结构化上报
const NATURAL_PROTOCOL: &str = "## 执行协议\n- 同一任务的多个独立操作，必须在一次回复中并发调用多个工具，不要拆成多轮\n- 查询结果在同一任务内全程复用：需要补充信息时换角度查询，不要原样重查\n- 创建任何业务对象前，必须先用「数据查询」按名称检索同类对象：完全同名由工具层自动复用；存在相似项且不确定是否同一目标时，不要猜测创建，通过「缺口与确认上报」登记确认事项（附候选）请用户确认\n- 拿不准的判断点不要中途停摆、不要编造：独立操作照常完成；数据缺口通过「缺口与确认上报」登记数据缺口（主 Agent 会自查补齐并续派，或转问用户），必须用户拍板的登记为确认事项；不要在正文里写 MISSING:/NEED_USER: 等文字标记\n- 你的汇报只交主 Agent 处理、不直接展示给用户：面向用户的提问与引导一律经「缺口与确认上报」的确认通道表达，由系统自动转卡片（主 Agent 无权代答），正文勿直接向用户发问\n- 任何输出中禁止出现工具函数名（ext_tool_ 开头的标识符）与英文参数名，一律用中文业务称谓\n- 若工具返回\"未授权\"或\"工具未找到\"，跳过该操作，用已有的信息继续完成任务\n- 按照 system_prompt 中的「回答规范」向用户汇报结果，不要输出内部 ID";

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
    /// ★ 通道框架（DELIVERY 直出授权唯一裁决）：本轮 LLM 正文/思考流是否直连用户 UI。
    /// deliverable=成品流直出；natural=内部汇报仅交主 Agent（UI 走 TRACE 事件与主 Agent 复述）。
    /// 主 Agent 自身（无子 Agent 身份）恒为 true（调用侧以 class=None 处理）。
    /// 新增产出类别只需在此补齐一行语义，delegate/llm/前端零散点自动收敛。
    pub fn streams_to_user(self) -> bool {
        match self { OutputClass::Natural => false, OutputClass::Deliverable => true }
    }
    /// 委托尾部指令的执行协议段
    pub fn directive_protocol(self) -> &'static str {
        match self { OutputClass::Natural => NATURAL_PROTOCOL, OutputClass::Deliverable => DELIVERABLE_PROTOCOL }
    }
    /// 结果终裁：返回 (正文, status, 生效 missing, confirmations)。
    /// 输入 = ext_tool_report 结构化上报（唯一通道——旧 MISSING:/NEED_USER: 文字标记解析层已删除）。
    /// 交付型：缺口回贴「待补」清单不阻塞，任何上报都不改 status（正文即终稿，直出恒通过）；
    ///   confirmations 原样带出，由框架自动转交互卡片（直出轮或主 Agent 回合末）。
    /// 执行型：gaps→partial（主 Agent 自答续派闭环）；confirmations 仅带出，不影响 status；
    ///   全失败无产出→failed（上报不豁免）。
    pub fn finalize(
        self,
        mut text: String,
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
        fn append_block(text: &mut String, items: &[String]) {
            let block = items.iter().map(|m| format!("- {}", m)).collect::<Vec<_>>().join("\n");
            text.push_str(&format!("\n\n【待补/需确认】\n{}", block));
        }
        let gaps = uniq(rep_gaps.iter().cloned());
        let failed_first = any_failed && !has_artifacts;
        match self {
            OutputClass::Deliverable => {
                if failed_first { return (text, "failed", vec![], vec![]); }
                if !gaps.is_empty() { append_block(&mut text, &gaps); }
                (text, "completed", vec![], rep_confs.to_vec())
            }
            OutputClass::Natural => {
                let status = if !gaps.is_empty() { "partial" }
                    else if failed_first { "failed" }
                    else { "completed" };
                (text, status, gaps, rep_confs.to_vec())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::OutputClass;
    use serde_json::json;

    /// 通道框架：DELIVERY 直出授权表驱动——natural 不直出、deliverable 直出
    #[test]
    fn streams_to_user_channel_routing() {
        assert!(!OutputClass::Natural.streams_to_user(), "执行型正文是 HANDOFF，不直连 UI");
        assert!(OutputClass::Deliverable.streams_to_user(), "交付型正文是 DELIVERY，直出气泡");
    }

    /// 框架协议含控制面/内容面分离条款（对话控制走 report，正文禁对话句）
    #[test]
    fn protocols_enforce_control_plane_separation() {
        use super::{DELIVERABLE_PROTOCOL, NATURAL_PROTOCOL};
        assert!(DELIVERABLE_PROTOCOL.contains("正文只含交付内容"), "交付协议须含对话控制分离条款");
        assert!(DELIVERABLE_PROTOCOL.contains("禁止在正文写【请确认】"));
        assert!(NATURAL_PROTOCOL.contains("不直接展示给用户"), "执行协议须声明 HANDOFF 定位");
    }

    #[test]
    fn deliverable_confirmation_goes_completed_with_confs() {
        let confs = vec![json!({"question": "确认框架？", "options": ["确认", "调整"]})];
        let (t, s, eff, out_confs) = OutputClass::Deliverable.finalize("框架".into(), &[], &confs, false, false);
        assert_eq!(s, "completed", "确认事项不改交付直出——框架自动发卡");
        assert!(eff.is_empty());
        assert_eq!(out_confs, confs, "confs 原样带出，由框架转卡片");
        assert_eq!(t, "框架");
    }
    #[test]
    fn deliverable_gaps_only_appends_and_stays_completed() {
        let (t, s, eff, _) = OutputClass::Deliverable.finalize(
            "正文".into(), &["缺预算".into()], &[], false, false);
        assert_eq!(s, "completed");
        assert!(eff.is_empty());
        assert!(t.contains("【待补/需确认】") && t.contains("- 缺预算"));
    }
    #[test]
    fn natural_status_gaps_only_and_failure_wins() {
        let confs = vec![json!({"question": "路线?", "options": []})];
        let (t, s, eff, out_confs) = OutputClass::Natural.finalize("文".into(), &["缺X".to_string(), "缺Y".to_string()], &confs, false, true);
        assert_eq!(s, "partial");
        assert_eq!(eff, vec!["缺X".to_string(), "缺Y".to_string()]);
        assert_eq!(out_confs.len(), 1, "natural 的 confs 原样带出转卡片，不再串化进 missing");
        assert_eq!(t, "文");
        // 纯 confirmations 无 gaps：不标 partial（无续派事项），只随回合转卡片
        let (_, s2, eff2, c2) = OutputClass::Natural.finalize("文".into(), &[], &confs, false, true);
        assert_eq!(s2, "completed");
        assert!(eff2.is_empty());
        assert_eq!(c2, confs);
        assert_eq!(OutputClass::Deliverable.finalize("x".into(), &[], &[], true, false).1, "failed");
        assert_eq!(OutputClass::Natural.finalize("x".into(), &[], &[], true, false).1, "failed");
        assert_eq!(OutputClass::from_format("weird"), OutputClass::Natural);
    }
    #[test]
    fn gaps_dedup_in_finalize() {
        let (_, _, eff, _) = OutputClass::Natural.finalize("x".into(), &["同一条".to_string(), "同一条".to_string(), "".to_string()], &[], false, true);
        assert_eq!(eff, vec!["同一条".to_string()]);
    }
}
