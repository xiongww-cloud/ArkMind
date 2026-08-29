use serde_json::{json, Value};
use tauri::Emitter;

const PRESENTATION_PERSONA: &str = r#"
## 关键规则：直接调用工具
当用户要求制作/修改演示材料时，你必须直接调用工具执行。禁止在回复中描述"我来做"或输出操作步骤。

示例：
- 用户："帮我生成10页项目汇报" → 你：直接逐页调用 pres_tool_generate_page（不输出文字）
- 用户："第3页改成表格" → 你：直接调用 pres_tool_regenerate_slide
- 用户："标题改大一点" → 你：直接调用 pres_tool_modify_slide
- 不确定时可以先问用户，但一旦明确就直接执行
- 工具完成后，用简洁文字告知用户结果

### 演示材料工具
- **pres_tool_generate_page**：生成一页 HTML。逐页生成，每页调用一次。
- **pres_tool_regenerate_slide**：重新设计某页。需要传入当前页 HTML。
- **pres_tool_modify_slide**：修改某页局部。需要传入当前页 HTML。
- 其他工具（知识库检索、项目查询等）照常使用。

### 画布尺寸（极其重要）
演示材料的画布尺寸是固定的：**1920×1080px（Full HD 16:9）**。
- 所有内容必须在这个尺寸内完整显示，不能溢出
- 文字大小要适配这个尺寸（标题 48-72px，正文 24-32px，小字 16-20px）
- 布局要考虑内容密度，避免一页塞太多内容导致溢出
- 传给工具的 instruction 中要明确这个尺寸约束

### 其他规则
- 每页输出 <section class="slide"> 元素
- 使用 CSS 变量（var(--accent) 等）确保风格一致
- 使用绝对定位或 flexbox/grid 自由排版
"#;

/// 演示材料智能体对话（复用 agent_chat_workflow + 演示工具）。
/// data: { query: string, deckId: string, themeCss?: string, deckStatus?: string, attachments?: [{path,name}] }
#[tauri::command]
pub async fn presentation_agent_chat(app_handle: tauri::AppHandle, data: Value) -> Result<(), String> {
    let query = data["query"].as_str().unwrap_or("").to_string();
    let deck_id = data["deckId"].as_str().unwrap_or("default").to_string();
    let theme_css = data["themeCss"].as_str().unwrap_or("").to_string();
    let deck_status = data["deckStatus"].as_str().unwrap_or("").to_string();
    let attachments: Vec<Value> = data["attachments"].as_array().cloned().unwrap_or_default();

    let session_id = format!("pres-{}", deck_id);
    let persona = format!(
        "{}\n\n## 全局样式\n{}\n\n## 当前演示材料状态\n{}",
        PRESENTATION_PERSONA,
        if theme_css.is_empty() { "（使用默认主题）".to_string() } else { theme_css },
        if deck_status.is_empty() { "（尚未生成）".to_string() } else { deck_status }
    );

    tauri::async_runtime::spawn(async move {
        let result = crate::services::agent_workflow::agent_chat_workflow(
            query, attachments, Some(10), Some(app_handle.clone()), false,
            Some(session_id), Some(persona),
            crate::services::agent_workflow::ToolMode::Presentation,
        ).await;
        match result {
            Ok(v) => {
                let answer = v["answer"].as_str().unwrap_or("").to_string();
                let success = v["success"].as_bool().unwrap_or(true);
                let error = v["error"].as_str().unwrap_or("").to_string();
                if success {
                    let _ = app_handle.emit("agent:done", json!({"success": true, "answer": answer}));
                } else {
                    let _ = app_handle.emit("agent:done", json!({"success": false, "error": error, "answer": answer}));
                }
            }
            Err(e) => {
                let _ = app_handle.emit("agent:error", json!({"error": e}));
            }
        }
    });
    Ok(())
}
