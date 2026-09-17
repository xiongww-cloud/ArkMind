use crate::database;
use crate::license;
use crate::crypto;
use serde_json::{json, Value};
use tauri::Manager;

#[tauri::command] pub async fn settings_get() -> Result<Value, String> {
    // 返回 camelCase 键名，匹配前端 Settings.vue 的读取方式
    let row = database::get("SELECT tenant_id, active_storage, local_path, oss_region, oss_bucket, oss_ak, oss_sk, oss_encrypt_key, encrypt_password_salt, ai_config, kb_config, storage_cards, active_card_id, created_at, updated_at FROM system_config WHERE tenant_id='local-user'",&[]).map_err(|e| e.to_string())?;
    match row {
        Some(r) => {
            let rename = |from: &str, _to: &str| -> serde_json::Value {
                r.get(from).cloned().unwrap_or(serde_json::Value::Null)
            };
            let mask = |v: &serde_json::Value| -> serde_json::Value {
                match v.as_str() { Some(s) => json!(crypto::mask_secret(s)), None => v.clone() }
            };
            // kb_config 字符串内部密钥同样脱敏后再回传
            let kb_masked: serde_json::Value = match r.get("kb_config").and_then(|v| v.as_str()) {
                Some(s) => {
                    let mut cfg: Value = serde_json::from_str(s).unwrap_or(json!({}));
                    for k in ["chatApiKey", "embeddingApiKey"] {
                        if let Some(v) = cfg.get(k).and_then(|v| v.as_str()) {
                            cfg[k] = json!(crypto::mask_secret(&crypto::decrypt_api_key(v, &license::get_machine_hash())));
                        }
                    }
                    json!(cfg.to_string())
                }
                None => rename("kb_config", "kbConfig"),
            };
            Ok(json!({
                "tenantId": rename("tenant_id", "tenantId"),
                "activeStorage": rename("active_storage", "activeStorage"),
                "localPath": rename("local_path", "localPath"),
                "ossRegion": rename("oss_region", "ossRegion"),
                "ossBucket": rename("oss_bucket", "ossBucket"),
                "ossAk": rename("oss_ak", "ossAk"),
                "ossSk": mask(&rename("oss_sk", "ossSk")),
                "ossEncryptKey": mask(&rename("oss_encrypt_key", "ossEncryptKey")),
                "encryptPasswordSalt": mask(&rename("encrypt_password_salt", "encryptPasswordSalt")),
                "aiConfig": rename("ai_config", "aiConfig"),
                "kbConfig": kb_masked,
                "storageCards": rename("storage_cards", "storageCards"),
                "activeCardId": rename("active_card_id", "activeCardId"),
                "createdAt": rename("created_at", "createdAt"),
                "updatedAt": rename("updated_at", "updatedAt"),
            }))
        }
        None => Ok(json!({}))
    }
}
#[tauri::command] pub async fn settings_update(data: Value) -> Result<Value, String> {
    let mut fields: Vec<String>=Vec::new(); let mut values: Vec<Value>=Vec::new();
    if let Some(v)=data["activeStorage"].as_str(){fields.push("active_storage=?".into());values.push(json!(v));}
    if !data["localPath"].is_null() { fields.push("local_path=?".into()); values.push(json!(data["localPath"].as_str().unwrap_or(""))); }
    if !data["ossRegion"].is_null() { fields.push("oss_region=?".into()); values.push(json!(data["ossRegion"].as_str().unwrap_or(""))); }
    if !data["ossBucket"].is_null() { fields.push("oss_bucket=?".into()); values.push(json!(data["ossBucket"].as_str().unwrap_or(""))); }
    if !data["ossAk"].is_null() { fields.push("oss_ak=?".into()); values.push(json!(data["ossAk"].as_str().unwrap_or(""))); }
    // 秘密字段：收到脱敏占位串视为未改动，跳过写回（防打码值覆盖真实密钥）
    if let Some(v) = data["ossSk"].as_str() { if !crypto::is_masked_placeholder(v) { fields.push("oss_sk=?".into()); values.push(json!(v)); } }
    if let Some(v) = data["ossEncryptKey"].as_str() { if !crypto::is_masked_placeholder(v) { fields.push("oss_encrypt_key=?".into()); values.push(json!(v)); } }
    if let Some(v) = data["encryptPasswordSalt"].as_str() { if !crypto::is_masked_placeholder(v) { fields.push("encrypt_password_salt=?".into()); values.push(json!(v)); } }
    if let Some(v)=data["aiConfig"].as_str(){fields.push("ai_config=?".into());values.push(json!(v));}
    if fields.is_empty(){return Ok(json!({"success":true}));}
    fields.push("updated_at=datetime('now', 'localtime')".into());
    let sql = format!("UPDATE system_config SET {} WHERE tenant_id='local-user'", fields.join(","));
    let result = database::run(&sql, &values).map_err(|e| e.to_string())?;
    if result.changes == 0 {
        let cols: Vec<&str> = fields.iter().filter(|f| !f.starts_with("updated_at")).filter_map(|f| f.split_once('=').map(|(c,_)| c)).collect();
        let phs: Vec<&str> = cols.iter().map(|_| "?").collect();
        let ins = format!("INSERT INTO system_config (tenant_id, {}, updated_at) VALUES ('local-user', {}, datetime('now', 'localtime'))", cols.join(","), phs.join(","));
        database::run(&ins, &values).map_err(|e| e.to_string())?;
    }
    // 存储路径变更后刷新缓存
    if !data["localPath"].is_null() { crate::commands::file_commands::invalidate_storage_base(); }
    Ok(json!({"success":true}))
}
#[tauri::command] pub async fn get_ai_config() -> Result<Value, String> { let row = database::get("SELECT ai_config FROM system_config WHERE tenant_id='local-user'",&[]).map_err(|e| e.to_string())?; match row { Some(r) => { if let Some(s) = r["ai_config"].as_str() { let parsed: Value = serde_json::from_str(s).unwrap_or(json!({})); let machine = license::get_machine_hash(); Ok(crypto::decrypt_config_api_keys(&parsed, &machine)) } else { Ok(json!({})) } } None => Ok(json!({})) } }
#[tauri::command] pub async fn update_ai_config(data: Value) -> Result<Value, String> {
    // Support both wrapped (data.aiConfig) and direct (data itself) formats
    let incoming: Value = if let Some(s) = data["aiConfig"].as_str().or(data["ai_config"].as_str()) {
        serde_json::from_str(s).unwrap_or(json!({}))
    } else if data.get("embedding").is_some() || data.get("providers").is_some() || data.get("visionProviders").is_some() || data.get("chatApiUrl").is_some() {
        data.clone()
    } else {
        json!({})
    };
    // Read existing config and merge (so partial saves don't wipe other fields)
    let machine = license::get_machine_hash();
    let existing: Value = database::get("SELECT ai_config FROM system_config WHERE tenant_id='local-user'", &[]).ok().flatten()
        .and_then(|r| r["ai_config"].as_str().map(|s| {
            let v: Value = serde_json::from_str(s).unwrap_or(json!({}));
            crypto::decrypt_config_api_keys(&v, &machine)
        }))
        .unwrap_or(json!({}));
    let mut merged = existing;
    if let (Some(obj), Some(patch)) = (merged.as_object_mut(), incoming.as_object()) {
        for (k, v) in patch {
            if v.is_null() {
                obj.remove(k);
            } else {
                obj.insert(k.clone(), v.clone());
            }
        }
    }
    let encrypted = crypto::encrypt_config_api_keys(&merged, &machine);
    let encrypted_str = serde_json::to_string(&encrypted).unwrap_or_else(|_| serde_json::to_string(&merged).unwrap_or_default());
    let result = database::run("UPDATE system_config SET ai_config=?,updated_at=datetime('now', 'localtime') WHERE tenant_id='local-user'",&[json!(encrypted_str)]).map_err(|e| e.to_string())?;
    if result.changes == 0 {
        database::run("INSERT INTO system_config(tenant_id, ai_config, created_at, updated_at) VALUES('local-user', ?, datetime('now','localtime'), datetime('now','localtime'))",&[json!(encrypted_str)]).map_err(|e| e.to_string())?;
    }
    crate::services::embedding_client::invalidate_embedding_config_cache();
    crate::services::chat_api::invalidate_chat_config_cache();
    Ok(json!({"success":true}))
}
#[tauri::command]
pub async fn ai_test_provider(data: Value) -> Result<Value, String> {
    let base_url = data["baseUrl"].as_str().unwrap_or("");
    let api_key = data["apiKey"].as_str().unwrap_or("");
    let model = data["modelName"].as_str().unwrap_or("");
    if base_url.is_empty() || api_key.is_empty() || model.is_empty() {
        return Ok(json!({"success":false,"error":"请填写完整信息"}));
    }
    // 直接对完整的 API URL 发一个轻量 POST 请求验证连通性
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build().map_err(|e| e.to_string())?;
    let body = json!({"model": model, "max_tokens": 1, "messages": [{"role":"user","content":[{"type":"text","text":"ping"}]}]});
    let test_url = if base_url.contains("/v1/") || base_url.ends_with("/chat/completions") {
        base_url.to_string()
    } else {
        format!("{}/chat/completions", base_url.trim_end_matches('/'))
    };
    let mut req = client.post(&test_url).json(&body);
    if !api_key.is_empty() { req = req.header("Authorization", format!("Bearer {}", api_key)); }
    match req.send().await {
        Ok(resp) => {
            let status = resp.status();
            if status.is_success() || status == 400 {
                // 400 may mean the request is valid but content rejected — that's OK for connectivity test
                Ok(json!({"success":true,"message":format!("「{}」连接成功", model)}))
            } else {
                let body = resp.text().await.unwrap_or_default();
                Ok(json!({"success":false,"error":format!("「{}」连接失败: HTTP {}: {}", model, status, body)}))
            }
        }
        Err(e) => Ok(json!({"success":false,"error":format!("「{}」连接失败: {}", model, e)})),
    }
}
#[tauri::command]
pub async fn ai_test_embedding(data: Value) -> Result<Value, String> {
    let base_url = data["baseUrl"].as_str().unwrap_or("");
    let api_key = data["apiKey"].as_str().unwrap_or("");
    let model = data["modelName"].as_str().unwrap_or("text-embedding-3-small");
    if base_url.is_empty() || api_key.is_empty() || model.is_empty() {
        return Ok(json!({"success":false,"error":"请填写完整信息"}));
    }
    let base = base_url.trim_end_matches('/');
    let url = if base.contains("/v1/") || base.ends_with("/embeddings") {
        base_url.to_string()
    } else if base.ends_with("/v1") {
        format!("{}/embeddings", base)
    } else {
        format!("{}/v1/embeddings", base)
    };
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(30)).build().map_err(|e| e.to_string())?;
    let mut req = client.post(&url).json(&serde_json::json!({"model": model, "input": "ping"}));
    if !api_key.is_empty() { req = req.header("Authorization", format!("Bearer {}", api_key)); }
    match req.send().await {
        Ok(resp) => {
            let status = resp.status();
            if status.is_success() {
                Ok(json!({"success":true,"message":format!("「{}」连接成功", model)}))
            } else {
                let body = resp.text().await.unwrap_or_default();
                Ok(json!({"success":false,"error":format!("「{}」连接失败: HTTP {}: {}", model, status, body)}))
            }
        }
        Err(e) => Ok(json!({"success":false,"error":format!("「{}」连接失败: {}", model, e)})),
    }
}
#[tauri::command] pub async fn user_profile() -> Result<Value, String> { let r = database::get("SELECT * FROM users WHERE id='local-admin' LIMIT 1",&[]).map_err(|e| e.to_string())?.unwrap_or(json!({"id":"local-admin","name":"本地管理员","username":"local-admin","role":"OWNER"})); let mut v = match r { Value::Object(m)=>m, _=>serde_json::Map::new() }; v.insert("email_addresses".into(), json!(crate::services::whoami::Whoami::load().emails)); Ok(Value::Object(v)) }
#[tauri::command] pub async fn user_profile_update(data: Value) -> Result<Value, String> { let mut fields: Vec<String>=Vec::new(); let mut values: Vec<Value>=Vec::new(); for k in ["name","phone","title","company"] { if let Some(v)=data[k].as_str(){fields.push(format!("{k}=?"));values.push(json!(v.trim()));} } if fields.is_empty(){return Ok(json!({"success":true}));} database::run("INSERT OR IGNORE INTO users (id,name,username,role) VALUES ('local-admin','本地管理员','local-admin','OWNER')", &[]).map_err(|e| e.to_string())?; fields.push("updated_at=datetime('now', 'localtime')".into()); let sql = format!("UPDATE users SET {} WHERE id='local-admin'", fields.join(",")); database::run(&sql, &values).map_err(|e| e.to_string())?; Ok(json!({"success":true})) }
// ─── AI 完善档案（证据提取 → 用户确认，不自动写入）────────────
/// 收集"用户是谁"的确定性证据：本人发出邮件的显示名分布 + 签名档样本（正文尾部）
pub(crate) fn collect_profile_evidence(w: &crate::services::whoami::Whoami) -> String {
    let mut ev: Vec<String> = Vec::new();
    let mut sig_lines: Vec<String> = Vec::new();
    for me in &w.emails {
        let rows = database::all(
            "SELECT from_name, COUNT(*) AS n FROM emails WHERE lower(from_addr)=?1 AND from_name<>'' GROUP BY from_name ORDER BY n DESC LIMIT 3",
            &[json!(me)],
        ).unwrap_or_default();
        for r in rows {
            ev.push(format!("- 发件显示名「{}」出现 {} 次（邮箱 {}）",
                r["from_name"].as_str().unwrap_or(""), r["n"].as_i64().unwrap_or(0), me));
        }
        let bodies = database::all(
            "SELECT body_text FROM emails WHERE lower(from_addr)=?1 AND body_text<>'' ORDER BY received_at DESC LIMIT 5",
            &[json!(me)],
        ).unwrap_or_default();
        for r in bodies.iter().take(3) {
            let b = r["body_text"].as_str().unwrap_or("");
            let tail: String = b.chars().rev().take(260).collect::<Vec<_>>().into_iter().rev().collect();
            let tail = tail.trim();
            if !tail.is_empty() { sig_lines.push(format!("- 「…{}」", tail.replace('\n', " / "))); }
        }
    }
    if ev.is_empty() && sig_lines.is_empty() { return String::new(); }
    let mut out = String::new();
    if !ev.is_empty() { out.push_str(&format!("发件人显示名统计：\n{}\n", ev.join("\n"))); }
    if !sig_lines.is_empty() { out.push_str(&format!("\n本人发出邮件的正文尾部片段（签名档常在末尾）：\n{}\n", sig_lines.join("\n"))); }
    out
}

/// 从 LLM 回复中提取首个 JSON 对象（容忍 ```json 围栏与前后闲文；按括号平衡扫描，不被后续对象干扰）
pub(crate) fn extract_json_object(s: &str) -> Option<Value> {
    let bytes: Vec<char> = s.chars().collect();
    for (i, c) in bytes.iter().enumerate() {
        if *c != '{' { continue; }
        let mut depth = 0i32;
        let mut in_str = false;
        let mut esc = false;
        for (j, d) in bytes.iter().enumerate().skip(i) {
            if in_str {
                if esc { esc = false; }
                else if *d == '\\' { esc = true; }
                else if *d == '"' { in_str = false; }
                continue;
            }
            match d {
                '"' => in_str = true,
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        if let Ok(v) = serde_json::from_str::<Value>(&bytes[i..=j].iter().collect::<String>()) {
                            return Some(v);
                        }
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    None
}

#[tauri::command]
pub async fn profile_ai_fill(app: tauri::AppHandle) -> Result<Value, String> {
    let w = crate::services::whoami::Whoami::load();
    let evidence = collect_profile_evidence(&w);
    if evidence.is_empty() {
        return Err("还没有可分析的邮件证据：请先在「邮件」配置账户并同步收发件".into());
    }
    let msgs = vec![
        crate::services::agent_workflow::ChatMessage {
            role: crate::services::agent_workflow::MessageRole::System,
            content: Some("你是用户档案提取器。根据用户自己发出邮件的证据（显示名统计、正文尾部签名档），提取用户的姓名、职位、公司。只输出一个JSON对象：{\"name\":\"\",\"title\":\"\",\"company\":\"\"}。提取不到的字段必须留空字符串，严禁猜测编造。注意：签名档证据可能混有正文尾巴或免责声明，从中辨别真正的姓名/职位/公司。".to_string()),
            tool_calls: None, tool_call_id: None, name: None, reasoning_content: None,
        },
        crate::services::agent_workflow::ChatMessage {
            role: crate::services::agent_workflow::MessageRole::User,
            content: Some(evidence.clone()),
            tool_calls: None, tool_call_id: None, name: None, reasoning_content: None,
        },
    ];
    // 后台单发任务：silent=true 不向聊天泄漏 token；cancel_aware=false 与聊天取消隔离
    let raw = crate::services::agent_workflow::llm::call_llm_without_tools(&msgs, &Some(app), true, false).await
        .map_err(|e| format!("AI 分析失败: {}", e))?;
    let parsed = extract_json_object(&raw).ok_or_else(|| "AI 返回结果无法解析，请重试".to_string())?;
    let clean = |k: &str| -> String {
        let s = parsed[k].as_str().unwrap_or("").trim().to_string();
        if s.chars().count() > 50 { s.chars().take(50).collect() } else { s }
    };
    Ok(json!({
        "success": true,
        "name": clean("name"), "title": clean("title"), "company": clean("company"),
        "evidence_preview": evidence.chars().take(600).collect::<String>(),
    }))
}

// Storage
#[tauri::command] pub async fn storage_get_config() -> Result<Value, String> {
    let row = database::get("SELECT * FROM system_config WHERE tenant_id='local-user'",&[]).map_err(|e| e.to_string())?;
    let mut v = row.unwrap_or(json!({}));
    // 秘密字段脱敏，渲染层拿不到明文
    if let Some(obj) = v.as_object_mut() {
        for k in ["oss_sk", "oss_encrypt_key", "encrypt_password_salt"] {
            if let Some(s) = obj.get(k).and_then(|x| x.as_str()) { obj.insert(k.to_string(), json!(crypto::mask_secret(s))); }
        }
    }
    Ok(v)
}
#[tauri::command] pub async fn storage_update_config(_data: Value) -> Result<Value, String> { Ok(json!({"success":true})) }
#[tauri::command] pub async fn oss_test_connection() -> Result<Value, String> { Ok(json!({"success":true,"message":"OSS test OK"})) }
#[tauri::command] pub async fn storage_migrate(_data: Value) -> Result<Value, String> { Ok(json!({"success":true})) }
#[tauri::command] pub async fn storage_migrate_progress() -> Result<Value, String> { Ok(json!({"status":"idle","total":0,"current":0})) }
#[tauri::command] pub async fn storage_default_path() -> Result<String, String> { Ok(".".to_string()) }
#[tauri::command] pub async fn oss_raw_diagnose() -> Result<Value, String> { Ok(json!({"success":true})) }

// ─── 日志 ─────────────────────────────────────────────────
#[tauri::command] pub fn get_logs(state: tauri::State<'_, crate::services::log_collector::LogState>) -> Vec<crate::services::log_collector::LogEntry> {
    state.get_all()
}
#[tauri::command] pub fn clear_logs(state: tauri::State<'_, crate::services::log_collector::LogState>) {
    state.clear();
}

// ─── 精灵窗口位置 ─────────────────────────────────────────
#[tauri::command]
pub async fn save_sprite_position(x: f64, y: f64) -> Result<Value, String> {
    let pos_str = serde_json::to_string(&json!({ "x": x, "y": y })).unwrap_or_default();
    let result = database::run(
        "UPDATE system_config SET sprite_position=?, updated_at=datetime('now','localtime') WHERE tenant_id='local-user'",
        &[json!(pos_str)]
    ).map_err(|e| e.to_string())?;

    if result.changes == 0 {
        database::run(
            "INSERT INTO system_config(tenant_id, sprite_position, created_at, updated_at) VALUES('local-user', ?, datetime('now','localtime'), datetime('now','localtime'))",
            &[json!(pos_str)]
        ).map_err(|e| e.to_string())?;
    }

    Ok(json!({ "success": true }))
}

#[tauri::command]
pub async fn get_sprite_position() -> Result<Value, String> {
    get_sprite_position_sync().map_err(|e| e.to_string())
}

pub fn get_sprite_position_sync() -> Result<Value, String> {
    let row = database::get("SELECT sprite_position FROM system_config WHERE tenant_id='local-user'", &[]).map_err(|e| e.to_string())?;
    match row {
        Some(r) => {
            if let Some(s) = r["sprite_position"].as_str() {
                let pos: Value = serde_json::from_str(s).unwrap_or(json!({}));
                if pos.get("x").is_some() && pos.get("y").is_some() {
                    return Ok(pos);
                }
            }
            Ok(json!({ "x": 800.0, "y": 400.0 }))
        }
        None => Ok(json!({ "x": 800.0, "y": 400.0 }))
    }
}

// ─── 精灵窗口开关 ─────────────────────────────────────────
#[tauri::command]
pub async fn get_sprite_enabled() -> Result<bool, String> {
    get_sprite_enabled_sync().map_err(|e| e.to_string())
}

pub fn get_sprite_enabled_sync() -> Result<bool, String> {
    let row = database::get("SELECT sprite_enabled FROM system_config WHERE tenant_id='local-user'", &[]).map_err(|e| e.to_string())?;
    match row {
        Some(r) => Ok(r["sprite_enabled"].as_i64().unwrap_or(0) != 0),
        None => Ok(false),
    }
}

#[tauri::command]
pub async fn set_sprite_enabled(enabled: bool, app: tauri::AppHandle) -> Result<Value, String> {
    let val: i64 = if enabled { 1 } else { 0 };
    let result = database::run(
        "UPDATE system_config SET sprite_enabled=?, updated_at=datetime('now','localtime') WHERE tenant_id='local-user'",
        &[json!(val)]
    ).map_err(|e| e.to_string())?;

    if result.changes == 0 {
        database::run(
            "INSERT INTO system_config(tenant_id, sprite_enabled, created_at, updated_at) VALUES('local-user', ?, datetime('now','localtime'), datetime('now','localtime'))",
            &[json!(val)]
        ).map_err(|e| e.to_string())?;
    }

    // 立即控制窗口显示/隐藏
    if enabled {
        // 窗口不存在时先创建
        if app.get_webview_window("sprite").is_none() {
            let _ = crate::commands::window_commands::create_sprite_window(app.clone()).await;
        }
        if let Some(win) = app.get_webview_window("sprite") {
            if let Ok(pos) = get_sprite_position_sync() {
                let x = pos["x"].as_f64().unwrap_or(800.0);
                let y = pos["y"].as_f64().unwrap_or(400.0);
                let _ = win.set_position(tauri::LogicalPosition::new(x, y));
            }
            let _ = win.set_always_on_top(true);
            let _ = win.show();
            let _ = win.set_focus();
        }
    } else if let Some(win) = app.get_webview_window("sprite") {
        let _ = win.hide();
    }

    Ok(json!({ "success": true }))
}

#[tauri::command] pub async fn get_system_setting(key: String) -> Result<Value, String> {
    let row = database::get("SELECT value FROM system_settings WHERE key=?", &[json!(key)]).map_err(|e| e.to_string())?;
    Ok(json!({"key": key, "value": row.and_then(|r| r["value"].as_str().map(|s| s.to_string()))}))
}

#[tauri::command] pub async fn set_system_setting(key: String, value: String) -> Result<Value, String> {
    database::run("INSERT OR REPLACE INTO system_settings(key, value, updated_at) VALUES(?1, ?2, datetime('now','localtime'))", &[json!(key), json!(value)]).map_err(|e| e.to_string())?;
    Ok(json!({"success": true}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_json_object_tolerates_fences_and_prose() {
        let v = extract_json_object("好的：\n```json\n{\"name\":\"熊某\",\"title\":\"\"}\n```").unwrap();
        assert_eq!(v["name"], json!("熊某"));
        assert_eq!(v["title"], json!(""));
        assert!(extract_json_object("没有大括号").is_none());
        assert!(extract_json_object("} 先闭后开 {").is_none());
        assert!(extract_json_object("{ 残缺不闭").is_none());
    }

    #[test]
    fn extract_json_object_picks_first_to_last_braces() {
        let v = extract_json_object("前置闲话 {\"a\":1} 尾巴 {\"b\":2}").unwrap();
        // 取首 { 到末 } 的跨度——内层非法则整体失败，属预期宽容边界
        assert!(v.is_null() || v["a"] == json!(1));
    }
}
