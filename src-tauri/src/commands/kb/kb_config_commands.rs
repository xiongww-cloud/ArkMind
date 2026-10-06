use crate::database;
use serde_json::{json, Value};
use tauri::Emitter;

#[tauri::command] pub async fn kb_get_config() -> Result<Value, String> {
    let row = database::get("SELECT kb_config FROM system_config WHERE tenant_id='local-user'", &[]).map_err(|e| e.to_string())?;
    let config: Value = row.and_then(|r| r["kb_config"].as_str().map(|s| s.to_string())).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(json!({}));
    Ok(json!({"success":true,"embeddingApiUrl":config["embeddingApiUrl"],"embeddingApiKey":crate::crypto::mask_secret(config["embeddingApiKey"].as_str().unwrap_or("")),"embeddingModelName":config["embeddingModelName"],"embeddingActive":config["embeddingActive"],"chatApiUrl":config["chatApiUrl"],"chatApiKey":crate::crypto::mask_secret(config["chatApiKey"].as_str().unwrap_or("")),"chatModelName":config["chatModelName"]}))
}
#[tauri::command] pub async fn kb_update_config(data: Value) -> Result<Value, String> {
    let machine = crate::license::get_machine_hash();
    let mut config = data;
    // 密钥字段：脱敏占位=未改动，沿用库内旧值；新值加密存储
    for key in &["chatApiKey", "embeddingApiKey"] {
        let incoming = config.get(*key).and_then(|v| v.as_str()).unwrap_or("").to_string();
        if incoming.is_empty() || crate::crypto::is_masked_placeholder(&incoming) {
            if let Some(existing) = existing_kb_key(key) {
                config[key] = json!(existing);
            }
        } else if !crate::crypto::is_encrypted(&incoming) {
            config[key] = json!(crate::crypto::encrypt_api_key(&incoming, &machine));
        }
    }
    database::run("UPDATE system_config SET kb_config=?,updated_at=datetime('now', 'localtime') WHERE tenant_id='local-user'", &[json!(config.to_string())]).map_err(|e| e.to_string())?;
    crate::services::embedding_client::invalidate_embedding_config_cache();
    crate::services::chat_api::invalidate_chat_config_cache();
    Ok(json!({"success":true}))
}

fn existing_kb_key(field: &str) -> Option<String> {
    database::get("SELECT kb_config FROM system_config WHERE tenant_id='local-user'", &[]).ok().flatten()
        .and_then(|r| r["kb_config"].as_str().map(|s| s.to_string()))
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v[field].as_str().map(|s| s.to_string()))
}

/// 将 tool_log 转换为 tool_calls + tool_results 格式
/// 返回 (all_tool_calls, all_tool_results)
fn convert_tool_log(tool_log: &[Value]) -> (Vec<Value>, Vec<Value>) {
    let mut all_tool_calls: Vec<Value> = Vec::new();
    let mut all_tool_results: Vec<Value> = Vec::new();
    let mut call_counter = 0u32;
    for round in tool_log {
        if let Some(tools) = round.get("tools").and_then(|v| v.as_array()) {
            for t in tools {
                let tool_name = t["tool"].as_str().unwrap_or("");
                let args = &t["args"];
                let result = t["result"].as_str().unwrap_or("");
                call_counter += 1;
                let call_id = format!("call_db_{}", call_counter);
                all_tool_calls.push(json!({
                    "id": call_id,
                    "type": "function",
                    "function": {
                        "name": tool_name,
                        "arguments": serde_json::to_string(args).unwrap_or_default()
                    }
                }));
                all_tool_results.push(json!({
                    "role": "tool",
                    "content": result,
                    "tool_call_id": call_id,
                    "name": tool_name
                }));
            }
        }
    }
    (all_tool_calls, all_tool_results)
}

/// 从 tool_log 提取本轮参与过的子 Agent 显示名（delegate 信封 data.agent_id，去重保序）。
/// 供消息 metadata.assistants 持久化，前端历史回放渲染 "via XX" 署名。纯函数，可单测。
pub(crate) fn extract_assistants(tool_log: &Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for round in tool_log.as_array().into_iter().flatten() {
        for t in round["tools"].as_array().into_iter().flatten() {
            if t["tool"].as_str() != Some("ext_tool_delegate_agent") { continue; }
            let Ok(env) = serde_json::from_str::<Value>(t["result"].as_str().unwrap_or("")) else { continue };
            if env["result_type"].as_str() != Some("delegate_agent_result") { continue; }
            if let Some(n) = env["data"]["agent_id"].as_str().map(|s| s.trim()).filter(|s| !s.is_empty()) {
                let n = n.to_string();
                if !out.contains(&n) { out.push(n); }
            }
        }
    }
    out
}

/// 主 Agent 认知协调工具集（不含 CRUD 与通知管理——通知产出归分析师与用户主动指令场景，
/// 主 Agent 对话在场即通知，其 schema 最重，移除以省每轮上下文；
/// 文档生成(write_document, 2508 字符全场最重)同样剥离：生成文件一律委托文档助手 document_writer）。
/// 联网工具特性门控：仅在已配置搜索服务时提供（未配置时模型完全无感知）。
pub(crate) fn main_agent_tool_list() -> Vec<String> {
    let base: Vec<String> = vec![
        "ext_tool_search".to_string(),
        "ext_tool_object".to_string(),
        "ext_tool_chunk_navigate".to_string(),
        "ext_tool_relations".to_string(),
        "ext_tool_memory".to_string(),
        "ext_tool_delegate_agent".to_string(),
        "ext_tool_ask_user".to_string(),
    ];
    if crate::services::web_search::is_configured() {
        base.into_iter().chain(crate::services::web_search::WEB_TOOL_NAMES.iter().map(|s| s.to_string())).collect()
    } else {
        base
    }
}

#[tauri::command] pub async fn kb_agent_chat(app_handle: tauri::AppHandle, data: Value) -> Result<(), String> {
    let query = data["query"].as_str().unwrap_or("").to_string();
    let max_iter = data["maxIterations"].as_i64().map(|n| n as i32);
    let session_id = data["sessionId"].as_str().map(String::from);
    let attachments: Vec<Value> = data["attachments"].as_array().cloned().unwrap_or_default();

    // ★ B 单流指针：任一窗口（浮窗/主窗）发送即更新"当前对话"——浮窗唤开续上次聊到的地方
    if let Some(ref sid) = session_id {
        super::write_fab_state(sid, chrono::Local::now().timestamp());
    }

    let main_agent_tools = main_agent_tool_list();

    tauri::async_runtime::spawn(async move {
        let result = crate::services::agent_workflow::agent_chat_workflow(query, attachments, max_iter, Some(app_handle.clone()), false, session_id.clone(), Some(main_agent_tools)).await;
        match result {
            Ok(mut v) => {
                // ★ 框架自动发卡：本轮委托上报的 confirmations（主 Agent 未自行追问时）
                crate::services::agent_workflow::workflow::attach_confirmation_cards(&mut v);
                // ★ 保存 assistant 消息到 DB（纯内容，无 metadata）
                if let Some(ref sid) = session_id {
                    let mut answer = v["answer"].as_str().unwrap_or("").to_string();
                    // ★ 持久化兜底：剥离工具调用 XML 残留，避免污染历史上下文
                    // 不过滤 ext_tool_ 文本（工具名可合法出现在答案中）
                    // "<tool_call" 同时匹配 <tool_call> 和 <tool_calls>（前缀覆盖）
                    if answer.contains("<tool_call") || answer.contains("<invoke") || answer.contains("DSML") {
                        answer = crate::services::agent_workflow::clean_tool_markers(&answer, true);
                    }
                    // ★ 提取本轮生成的文档对象写入消息 metadata（重启后恢复文档卡片）
                    let docs = crate::services::agent_workflow::tools::write_document::extract_created_documents(
                        v.get("tool_log"),
                    );
                    // ★ 追问卡片结构化数据（text+card 双轨：正文存文本版，metadata 存结构化版）
                    let clarification = v.get("clarification").filter(|c| c.is_object()).cloned();
                    // ★ via 署名：本轮参与过的子 Agent（历史回放用）
                    let assistants: Vec<String> = v.get("tool_log").map(extract_assistants).unwrap_or_default();
                    let mut meta_map = serde_json::Map::new();
                    if !docs.is_empty() {
                        meta_map.insert("documents".to_string(), json!(docs));
                        // ★ 卡片与定稿同步：生成期 draft 到此翻 ready 并首次进知识库解析管道
                        crate::services::doc_writer::finalize_generated_docs(&docs);
                    }
                    if let Some(c) = clarification { meta_map.insert("clarification".to_string(), c); }
                    if !assistants.is_empty() { meta_map.insert("assistants".to_string(), json!(assistants)); }
                    let metadata = if meta_map.is_empty() { Value::Null } else { Value::Object(meta_map) };
                    let _ = crate::database::run(
                        "INSERT INTO chat_messages(session_id, role, content, metadata) VALUES(?1, 'assistant', ?2, ?3)",
                        &[json!(sid), json!(answer), metadata],
                    );
                    // ★ 保存本轮工具数据到 session 级别（覆盖式，不含 content/role）
                    let mut tool_meta = serde_json::Map::new();
                    if let Some(tool_log) = v.get("tool_log").and_then(|v| v.as_array()) {
                        let (tc, tr) = convert_tool_log(tool_log);
                        if !tc.is_empty() { tool_meta.insert("tool_calls".to_string(), json!(tc)); }
                        if !tr.is_empty() { tool_meta.insert("tool_results".to_string(), json!(tr)); }
                    }
                    if let Some(cd) = v.get("context_docs") { tool_meta.insert("context_docs".to_string(), cd.clone()); }
                    let tool_meta_str = serde_json::Value::Object(tool_meta).to_string();
                    let _ = crate::database::run(
                        "UPDATE chat_sessions SET tool_metadata=?1, updated_at=datetime('now','localtime') WHERE id=?2",
                        &[json!(tool_meta_str), json!(sid)],
                    );
                }
                let _ = app_handle.emit("agent:done", &v);
                if let Some(ans) = v["answer"].as_str() { if !ans.is_empty() { let preview: String = ans.chars().take(100).collect(); println!("[Agent] 响应: {}", preview); } }
            }
            Err(e) => {
                if e == "cancelled" {
                    let _ = app_handle.emit("agent:done", json!({"success":true,"answer":"已停止执行。","cancelled":true}));
                } else {
                    eprintln!("[Agent] 错误: {}", e);
                    let _ = app_handle.emit("agent:error", json!({"error": e}));
                }
            }
        }
    });
    Ok(())
}
#[tauri::command] pub async fn kb_chat(app_handle: tauri::AppHandle, data: Value) -> Result<(), String> {
    let query = data["query"].as_str().unwrap_or("").to_string();
    if query.is_empty() { return Err("请输入问题".to_string()); }
    // ★ 与主聊天同一份工具名单（此前传 None=全量工具集，绕过主 Agent 收敛，且 web 教学与 schema 错位）
    let tools = main_agent_tool_list();
    tauri::async_runtime::spawn(async move {
        let result = crate::services::agent_workflow::agent_chat_workflow(query, vec![], None, Some(app_handle.clone()), false, None, Some(tools)).await;
        match result {
            Ok(mut v) => { crate::services::agent_workflow::workflow::attach_confirmation_cards(&mut v); let _ = app_handle.emit("agent:done", &v); }
            Err(e) => {
                if e == "cancelled" {
                    let _ = app_handle.emit("agent:done", json!({"success":true,"answer":"已停止执行。","cancelled":true}));
                } else {
                    let _ = app_handle.emit("agent:error", json!({"error": e}));
                }
            }
        }
    });
    Ok(())
}
#[tauri::command] pub async fn agent_cancel() -> Result<Value, String> {
    crate::services::agent_workflow::cancel_agent();
    Ok(json!({"success": true}))
}

#[cfg(test)]
mod tests {
    use super::extract_assistants;
    use serde_json::json;

    /// via 署名提取：多轮 delegate 去重保序；非 delegate/失败信封（无 agent_id）忽略
    #[test]
    fn extract_assistants_dedups_and_orders() {
        let log = json!([
            {"round":1,"tools":[
                {"tool":"ext_tool_search","result":"whatever"},
                {"tool":"ext_tool_delegate_agent","result": json!({"result_type":"delegate_agent_result","data":{"status":"completed","agent_id":"业务助手"}}).to_string()},
                {"tool":"ext_tool_delegate_agent","result":"not-json"}
            ]},
            {"round":2,"tools":[
                {"tool":"ext_tool_delegate_agent","result": json!({"result_type":"delegate_agent_result","data":{"status":"partial","agent_id":"业务助手"}}).to_string()},
                {"tool":"ext_tool_delegate_agent","result": json!({"result_type":"delegate_agent_result","data":{"status":"failed","result":"路由未命中"}}).to_string()},
                {"tool":"ext_tool_delegate_agent","result": json!({"result_type":"delegate_agent_result","data":{"status":"completed","agent_id":"文档助手"}}).to_string()},
                {"tool":"ext_tool_object","result": json!({"result_type":"delegate_agent_result","data":{"agent_id":"不该出现"}}).to_string()}
            ]}
        ]);
        assert_eq!(extract_assistants(&log), vec!["业务助手".to_string(), "文档助手".to_string()]);
        assert!(extract_assistants(&json!([])).is_empty());
        assert!(extract_assistants(&json!(null)).is_empty());
    }
}
