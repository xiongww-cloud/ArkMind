//! 演示材料 AI —— 受约束的 Deck JSON 生成 / 单页精修
//!
//! 设计原则：AI 不拥有 CSS 权限。模型只能从「元素注册表」（layout + block + span + scale）
//! 中组合 Deck JSON，由渲染引擎负责视觉。注册表由前端从 TS elements.ts 序列化后注入，
//! 保证单一事实源。

use serde_json::{json, Value};

/// Deck 生成系统提示词。`{registry}` 占位符会被替换为前端传来的注册表摘要。
const DECK_GEN_SYSTEM_PROMPT: &str = r#"你是企业级演示材料策划与制作引擎。你的任务：根据用户的策划需求，产出一份符合指定 schema 的 Deck JSON。

## 硬性规则
1. 只能输出一个 JSON 对象，禁止输出 markdown 代码围栏、禁止任何解释文字。
2. 输出结构固定为：
{
  "slides": [
    { "layout": "<layoutId>", "heading": "本页标题文字", "blocks": [ { "type": "<blockType>", "data": {...}, "span": 6, "scale": 1 } ] }
  ]
}
3. layout 必须来自下方「元素注册表」，block.type 必须来自注册表且是该 layout 允许的类型。
4. 每页必须有一个标题：用顶层 "heading" 字段表示即可，blocks 中不要重复放 heading 区块。
5. data 字段严格按注册表中该 block 的字段说明填写；表格的 rows[0] 为表头行，columns 需与列数一致。
6. span 只能取注册表中的 span 集合；scale 只能取注册表中的 scale 集合。数值错误会整页被丢弃。
7. 产出页数：封面 1 页 + 目录 1 页 + 核心内容 5~10 页 + 结束页 1 页。
8. 内容必须基于【背景知识】中的事实，禁止编造参数、金额、型号；背景知识不足时给出方向性内容。
9. 中文字符正常输出（UTF-8），不要转义。
10. {capacity}

## 元素注册表
{registry}
"#;

/// 整篇修改系统提示词。输入当前完整 Deck + 修改要求，输出修改后的完整 Deck。
const DECK_REVISE_SYSTEM_PROMPT: &str = r#"你是演示材料整篇修改引擎。用户给出【当前完整 Deck JSON】和【修改要求】，请产出修改后的完整 Deck JSON。

## 硬性规则
1. 只能输出一个 JSON 对象（Deck），结构为 { "slides": [ { "layout": "...", "heading": "...", "blocks": [...] } ] }，禁止代码围栏、禁止解释文字。
2. 未在修改要求中提及的页面尽量保持原样（layout、heading、blocks 都不要动）；只改动用户要求涉及的页面。
3. 若要求涉及整篇（如换主题风格、全篇精简、整体重排），按要求调整所有页面。
4. 每个 slide 用顶层 "heading" 字段表示标题，blocks 中不要重复放 heading 区块。
5. layout / block.type / span / scale 必须来自下方「元素注册表」，且 block 属于该 layout 允许的类型；data 字段按注册表说明填写。
6. 基于【背景知识】补充事实，禁止编造。
7. 中文字符正常输出（UTF-8）。
8. {capacity}

## 元素注册表
{registry}
"#;

/// 单页精修系统提示词。
const SLIDE_REFINE_SYSTEM_PROMPT: &str = r#"你是演示材料精修引擎。用户给出【当前页面的 Deck JSON】和【修改要求】，请产出该页修改后的 Deck JSON（单个 slide 对象）。

## 硬性规则
1. 只能输出一个 JSON 对象（slide），禁止 markdown 代码围栏、禁止解释文字。
2. 结构固定为：
{ "layout": "<layoutId>", "heading": "...", "blocks": [ { "type": "<blockType>", "data": {...}, "span": 6, "scale": 1 } ] }
3. layout / block.type / span / scale 必须来自下方「元素注册表」，且 block 属于该 layout 允许的类型。
4. 保留该页原本的标题与核心信息，除非用户要求改；不要无故删掉原页中有价值的内容。
5. 用顶层 "heading" 字段表示标题，blocks 中不要重复放 heading 区块。
6. 若用户要求切换版式（如改表格、改三列），换成注册表中合适的 layout 并重组 blocks。
7. data 字段按注册表说明填写；表格 rows[0] 为表头行。
8. 基于【背景知识】补充事实，禁止编造。
9. {capacity}

## 元素注册表
{registry}
"#;

/// 版面容量约束（生成/整篇修改/单页精修共用）——防止元素溢出
const CAPACITY_RULES: &str = "版面容量约束：画布为 1280×720（16:9），内容必须一屏放下，宁可精简不可溢出。\n- 每页区块总数 ≤ 6 个（含标题以外的正文块）；封面/结束页 ≤ 4 个。\n- 要点/步骤列表 ≤ 6 条，每条 ≤ 40 字；段落/引用 ≤ 120 字。\n- 表格 ≤ 7 行（含表头）× 5 列，单元格内容 ≤ 20 字。\n- 每页最多 1 张图片；数据卡 ≤ 4 个；卡片网格 ≤ 3 个；左右分栏每栏条目 ≤ 4 条。\n- 文字过多时优先精简内容，而不是堆叠区块。";

/// 从 LLM 输出中提取 JSON（剥围栏、取首对花括号，失败返回 Err）
pub fn extract_json(text: &str) -> Result<Value, String> {
    let raw = text.trim();
    if raw.is_empty() {
        return Err("模型输出为空".to_string());
    }
    // 去掉可能的 ```json ... ``` 围栏
    let mut cleaned = raw.to_string();
    if cleaned.starts_with("```") {
        cleaned = cleaned
            .lines()
            .filter(|l| !l.trim_start().starts_with("```"))
            .collect::<Vec<_>>()
            .join("\n");
    }
    // 优先对象
    if let Some(start) = cleaned.find('{') {
        if let Some(end) = cleaned.rfind('}') {
            let slice = &cleaned[start..=end];
            if let Ok(v) = serde_json::from_str::<Value>(slice) {
                return Ok(v);
            }
        }
    }
    // 再试数组
    if let Some(start) = cleaned.find('[') {
        if let Some(end) = cleaned.rfind(']') {
            let slice = &cleaned[start..=end];
            if let Ok(v) = serde_json::from_str::<Value>(slice) {
                return Ok(v);
            }
        }
    }
    Err(format!("无法从模型输出中解析 JSON: {}", cleaned.chars().take(200).collect::<String>()))
}

fn check_key() -> Result<(), String> {
    let cfg = crate::services::chat_api::get_chat_config();
    if cfg.api_key.trim().is_empty() {
        return Err("请先在「系统设置 → AI 配置」中配置 Chat API 密钥。".to_string());
    }
    Ok(())
}

/// 注入注册表与版面容量约束到提示词占位符
fn fill_prompt(prompt: &str, registry: &str) -> String {
    let reg = if registry.is_empty() { "（未提供，请使用常见企业汇报版式）" } else { registry };
    prompt.replace("{registry}", reg).replace("{capacity}", CAPACITY_RULES)
}

/// 生成整份 Deck JSON。返回 `{ "slides": [...] }`（解析失败自动重试一次）。
pub async fn generate_deck(plan: &str, context: &str, registry: &str) -> Result<Value, String> {
    check_key()?;
    let system = fill_prompt(DECK_GEN_SYSTEM_PROMPT, registry);
    let user = format!(
        "【背景知识】\n{}\n\n【策划需求】\n{}\n\n请输出 Deck JSON。",
        if context.trim().is_empty() { "（暂无检索到的知识库内容）" } else { context },
        plan
    );

    let mut content = crate::services::chat_api::chat_completion_ex(&system, &user, None, 16384).await?;
    let mut parsed = extract_json(&content);
    if parsed.is_err() {
        let retry_user = format!("{}\n\n（请仅输出 JSON，不要包含任何解释或代码围栏）", user);
        content = crate::services::chat_api::chat_completion_ex(&system, &retry_user, None, 16384).await?;
        parsed = extract_json(&content);
    }
    let v = parsed?;

    if let Some(arr) = v.as_array() {
        return Ok(json!({ "slides": arr }));
    }
    if let Some(slides) = v.get("slides").and_then(|s| s.as_array()) {
        return Ok(json!({ "slides": slides }));
    }
    Err("模型输出缺少 slides 数组".to_string())
}

/// 整篇修改：输入当前完整 Deck JSON，返回修改后的 `{ "slides": [...] }`。
pub async fn revise_deck(
    deck_json: &str,
    instruction: &str,
    context: &str,
    registry: &str,
) -> Result<Value, String> {
    check_key()?;
    let system = fill_prompt(DECK_REVISE_SYSTEM_PROMPT, registry);
    let user = format!(
        "【当前完整 Deck JSON】\n{}\n\n【背景知识】\n{}\n\n【修改要求】\n{}\n\n请输出修改后的完整 Deck JSON。",
        deck_json,
        if context.trim().is_empty() { "（暂无）" } else { context },
        instruction
    );

    let mut content = crate::services::chat_api::chat_completion_ex(&system, &user, None, 12288).await?;
    let mut parsed = extract_json(&content);
    if parsed.is_err() {
        let retry_user = format!("{}\n\n（请仅输出 JSON，不要包含任何解释或代码围栏）", user);
        content = crate::services::chat_api::chat_completion_ex(&system, &retry_user, None, 12288).await?;
        parsed = extract_json(&content);
    }
    let v = parsed?;
    if let Some(arr) = v.as_array() {
        return Ok(json!({ "slides": arr }));
    }
    if let Some(slides) = v.get("slides").and_then(|s| s.as_array()) {
        return Ok(json!({ "slides": slides }));
    }
    Err("模型输出缺少 slides 数组".to_string())
}

/// 精修单页，返回修改后的 slide 对象。
pub async fn refine_slide(
    slide_json: &str,
    instruction: &str,
    context: &str,
    registry: &str,
) -> Result<Value, String> {
    check_key()?;
    let system = fill_prompt(SLIDE_REFINE_SYSTEM_PROMPT, registry);
    let user = format!(
        "【当前页面的 Deck JSON】\n{}\n\n【背景知识】\n{}\n\n【修改要求】\n{}\n\n请输出修改后的 slide JSON。",
        slide_json,
        if context.trim().is_empty() { "（暂无）" } else { context },
        instruction
    );

    let mut content = crate::services::chat_api::chat_completion_ex(&system, &user, None, 8192).await?;
    let mut parsed = extract_json(&content);
    if parsed.is_err() {
        let retry_user = format!("{}\n\n（请仅输出 JSON，不要包含任何解释或代码围栏）", user);
        content = crate::services::chat_api::chat_completion_ex(&system, &retry_user, None, 8192).await?;
        parsed = extract_json(&content);
    }
    let v = parsed?;
    if let Some(arr) = v.as_array() {
        if let Some(first) = arr.first() {
            return Ok(first.clone());
        }
        return Err("模型输出的 slide 数组为空".to_string());
    }
    Ok(v)
}

// ═══════════════════════════════════════════════════════════════════════════
// HTML 原生演示材料工具（新架构）
// ═══════════════════════════════════════════════════════════════════════════

const PAGE_GEN_SYSTEM_PROMPT: &str = r#"你是演示材料制作引擎。请根据要求生成一页演示材料的 HTML。

## 画布尺寸（最重要）
画布固定为 1920×1080px（Full HD 16:9）。所有内容必须在这个范围内完整显示，不能溢出。
- 文字大小适配此尺寸：标题 48-72px，正文 24-32px，小字 16-20px
- 内容密度适中，避免塞太多元素导致溢出
- 使用 flexbox/grid 布局时注意容器不会超出 1920×1080

## 输出规则
1. 只输出一个 <section class="slide"> 元素的完整 HTML，不要输出 <html>/<head>/<body> 标签。
2. 使用内联 style 定义所有样式。
3. 使用 CSS 变量：var(--accent)、var(--bg)、var(--text)、var(--muted)、var(--border)、var(--card)、var(--font)。
4. 使用绝对定位或 flexbox/grid 布局自由排版。
5. 确保 HTML 有效且标签闭合。
6. 中文内容正常输出（UTF-8），不要转义。
7. 禁止输出 markdown 代码围栏（```html）。
"#;

const SLIDE_REGEN_SYSTEM_PROMPT: &str = r#"你是演示材料修改引擎。用户要求重新设计某一页，你需要输出修改后的完整 HTML。

## 画布尺寸（最重要）
画布固定为 1920×1080px（Full HD 16:9）。所有内容必须在这个范围内完整显示，不能溢出。

## 输出规则
1. 只输出一个 <section class="slide"> 元素的完整 HTML。
2. 保留页面的核心信息（标题、数据、要点），重新设计布局和样式。
3. 使用 CSS 变量：var(--accent)、var(--bg)、var(--text)、var(--muted)、var(--border)、var(--card)、var(--font)。
4. 确保 HTML 有效且标签闭合。
5. 禁止输出 markdown 代码围栏。
"#;

const SLIDE_MODIFY_SYSTEM_PROMPT: &str = r#"你是演示材料修改引擎。用户要求修改某一页中的特定元素，你需要输出修改后的完整 HTML。

## 画布尺寸（最重要）
画布固定为 1920×1080px（Full HD 16:9）。所有内容必须在这个范围内完整显示，不能溢出。

## 输出规则
1. 只输出一个 <section class="slide"> 元素的完整 HTML。
2. 只修改用户要求的部分，其他内容保持不变。
3. 如果指定了 elementSelector，精确修改该元素。
4. 使用 CSS 变量：var(--accent)、var(--bg)、var(--text)、var(--muted)。
5. 确保 HTML 有效且标签闭合。
6. 禁止输出 markdown 代码围栏。
"#;

/// 从 LLM 输出中提取 HTML section（剥围栏、取 section 标签）
fn extract_html_section(text: &str) -> Result<String, String> {
    let raw = text.trim();
    if raw.is_empty() {
        return Err("模型输出为空".to_string());
    }
    // 去掉 ```html ... ``` 围栏
    let mut cleaned = raw.to_string();
    if cleaned.starts_with("```") {
        cleaned = cleaned
            .lines()
            .filter(|l| !l.trim_start().starts_with("```"))
            .collect::<Vec<_>>()
            .join("\n");
    }
    // 找 <section 和 </section>
    let start = cleaned.find("<section").ok_or("未找到 <section> 标签")?;
    let end_tag = "</section>";
    let end = cleaned.rfind(end_tag).ok_or("未找到 </section> 标签")?;
    let end = end + end_tag.len();
    Ok(cleaned[start..end].to_string())
}

/// 用流式 API 调用 LLM（避免推理模型 reasoning_content 问题）
/// silent=true 时不发 agent:token 事件（工具内部的 LLM 调用不应污染前端聊天流）
async fn llm_generate(
    system: &str,
    user: &str,
    app_handle: &Option<tauri::AppHandle>,
) -> Result<String, String> {
    use crate::services::agent_workflow::{call_llm_streaming_inner, ChatMessage, MessageRole};
    let messages = vec![
        ChatMessage { role: MessageRole::System, content: Some(system.to_string()), tool_calls: None, tool_call_id: None, name: None, reasoning_content: None },
        ChatMessage { role: MessageRole::User, content: Some(user.to_string()), tool_calls: None, tool_call_id: None, name: None, reasoning_content: None },
    ];
    let (content, reasoning, _tc, _usage) = call_llm_streaming_inner(&messages, None, app_handle, true).await?;
    // 流式 API 正常情况 content 非空；推理模型可能把输出放在 reasoning_content
    if !content.is_empty() {
        Ok(content)
    } else if let Some(r) = reasoning.filter(|r| !r.is_empty()) {
        log::info!("[llm_generate] content 为空，从 reasoning_content 提取 ({} chars)", r.len());
        Ok(r)
    } else {
        Err("模型输出为空".to_string())
    }
}

/// 生成一页演示材料的 HTML
pub async fn generate_page_html(
    page: usize,
    total_pages: usize,
    instruction: &str,
    app_handle: &Option<tauri::AppHandle>,
) -> Result<String, String> {
    check_key()?;
    let system = PAGE_GEN_SYSTEM_PROMPT.to_string();
    let user = format!(
        "请生成第{page}页（共{total_pages}页）的 HTML。\n\n设计要求：{instruction}\n\n请输出 <section class=\"slide\"> 的完整 HTML："
    );
    let mut content = llm_generate(&system, &user, app_handle).await?;
    let mut result = extract_html_section(&content);
    if result.is_err() {
        let retry_user = format!("{}\n\n请严格只输出 <section class=\"slide\"> 的 HTML，不要包含任何解释。", user);
        content = llm_generate(&system, &retry_user, app_handle).await?;
        result = extract_html_section(&content);
    }
    result
}

/// 重新设计某一页的 HTML
pub async fn regenerate_slide_html(
    page: usize,
    instruction: &str,
    current_html: &str,
    app_handle: &Option<tauri::AppHandle>,
) -> Result<String, String> {
    check_key()?;
    let system = SLIDE_REGEN_SYSTEM_PROMPT.to_string();
    let user = format!(
        "请重新设计第{page}页的 HTML。\n\n当前页面的 HTML：\n{current_html}\n\n修改要求：{instruction}\n\n请输出修改后的 <section class=\"slide\"> 的完整 HTML："
    );
    let mut content = llm_generate(&system, &user, app_handle).await?;
    let mut result = extract_html_section(&content);
    if result.is_err() {
        let retry_user = format!("{}\n\n请严格只输出 <section class=\"slide\"> 的 HTML。", user);
        content = llm_generate(&system, &retry_user, app_handle).await?;
        result = extract_html_section(&content);
    }
    result
}

/// 修改某页的局部元素
pub async fn modify_slide_html(
    page: usize,
    instruction: &str,
    current_html: &str,
    element_selector: &str,
    app_handle: &Option<tauri::AppHandle>,
) -> Result<String, String> {
    check_key()?;
    let system = SLIDE_MODIFY_SYSTEM_PROMPT.to_string();
    let selector_info = if element_selector.is_empty() {
        String::new()
    } else {
        format!("\n\n用户点击的元素选择器：{element_selector}")
    };
    let user = format!(
        "请修改第{page}页中的特定元素。\n\n当前页面的 HTML：\n{current_html}\n\n修改要求：{instruction}{selector_info}\n\n请输出修改后的 <section class=\"slide\"> 的完整 HTML："
    );
    let mut content = llm_generate(&system, &user, app_handle).await?;
    let mut result = extract_html_section(&content);
    if result.is_err() {
        let retry_user = format!("{}\n\n请严格只输出 <section class=\"slide\"> 的 HTML。", user);
        content = llm_generate(&system, &retry_user, app_handle).await?;
        result = extract_html_section(&content);
    }
    result
}
