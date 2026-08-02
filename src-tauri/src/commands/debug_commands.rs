#![cfg(debug_assertions)]
use serde_json::{json, Value};
use std::path::PathBuf;

/// 获取调试目录路径
fn get_debug_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".arkmind")
        .join("debug")
}

/// 保存 agent context 到文件（供调试页面使用）
#[tauri::command]
pub fn save_agent_context(messages: Vec<Value>) -> Result<Value, String> {
    let debug_dir = get_debug_dir();
    std::fs::create_dir_all(&debug_dir)
        .map_err(|e| format!("创建调试目录失败: {}", e))?;
    
    let path = debug_dir.join("context.json");
    let content = serde_json::to_string_pretty(&messages)
        .map_err(|e| format!("序列化 JSON 失败: {}", e))?;
    
    std::fs::write(&path, content)
        .map_err(|e| format!("写入文件失败: {}", e))?;
    
    log::info!("[Debug] 已保存 {} 条消息到 {}", messages.len(), path.display());
    Ok(json!({"success": true, "path": path.to_string_lossy()}))
}

/// 获取当前保存的 agent context
#[tauri::command]
pub fn get_agent_context() -> Result<Value, String> {
    let path = get_debug_dir().join("context.json");
    if !path.exists() {
        return Ok(json!({"messages": []}));
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("读取文件失败: {}", e))?;
    let messages: Vec<Value> = serde_json::from_str(&content)
        .map_err(|e| format!("解析 JSON 失败: {}", e))?;
    Ok(json!({"messages": messages}))
}
