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
            Ok(json!({
                "tenantId": rename("tenant_id", "tenantId"),
                "activeStorage": rename("active_storage", "activeStorage"),
                "localPath": rename("local_path", "localPath"),
                "ossRegion": rename("oss_region", "ossRegion"),
                "ossBucket": rename("oss_bucket", "ossBucket"),
                "ossAk": rename("oss_ak", "ossAk"),
                "ossSk": rename("oss_sk", "ossSk"),
                "ossEncryptKey": rename("oss_encrypt_key", "ossEncryptKey"),
                "encryptPasswordSalt": rename("encrypt_password_salt", "encryptPasswordSalt"),
                "aiConfig": rename("ai_config", "aiConfig"),
                "kbConfig": rename("kb_config", "kbConfig"),
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
    if !data["ossSk"].is_null() { fields.push("oss_sk=?".into()); values.push(json!(data["ossSk"].as_str().unwrap_or(""))); }
    if !data["ossEncryptKey"].is_null() { fields.push("oss_encrypt_key=?".into()); values.push(json!(data["ossEncryptKey"].as_str().unwrap_or(""))); }
    if !data["encryptPasswordSalt"].is_null() { fields.push("encrypt_password_salt=?".into()); values.push(json!(data["encryptPasswordSalt"].as_str().unwrap_or(""))); }
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
#[tauri::command] pub async fn user_profile() -> Result<Value, String> { database::get("SELECT * FROM users LIMIT 1",&[]).map_err(|e| e.to_string()).map(|r| r.unwrap_or(json!({"id":"local-admin","name":"本地管理员","username":"local-admin","role":"OWNER"}))) }
#[tauri::command] pub async fn user_profile_update(data: Value) -> Result<Value, String> { let mut fields: Vec<String>=Vec::new(); let mut values: Vec<Value>=Vec::new(); if let Some(v)=data["name"].as_str(){fields.push("name=?".into());values.push(json!(v));} if let Some(v)=data["phone"].as_str(){fields.push("phone=?".into());values.push(json!(v));} if fields.is_empty(){return Ok(json!({"success":true}));} fields.push("updated_at=datetime('now', 'localtime')".into()); let sql = format!("UPDATE users SET {} WHERE id='local-admin'", fields.join(",")); database::run(&sql, &values).map_err(|e| e.to_string())?; Ok(json!({"success":true})) }
// Storage
#[tauri::command] pub async fn storage_get_config() -> Result<Value, String> { database::get("SELECT * FROM system_config WHERE tenant_id='local-user'",&[]).map_err(|e| e.to_string()).map(|r| r.unwrap_or(json!({}))) }
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
