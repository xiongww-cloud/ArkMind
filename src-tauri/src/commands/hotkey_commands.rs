//! 浮窗快呼全局快捷键：规范化 + 运行时注册/回滚 + system_settings KV 持久化。
//! 注册与重注册全在 Rust 侧（JS 不引插件包，capabilities 无需新增权限）。

use once_cell::sync::Lazy;
use serde_json::{json, Value};
use std::sync::Mutex;
use tauri::AppHandle;
use tauri_plugin_global_shortcut::GlobalShortcutExt;
use crate::database;

/// 当前已注册快捷键（None=未注册/注册失败）；set 命令回滚也依赖它
static REGISTERED: Lazy<Mutex<Option<String>>> = Lazy::new(|| Mutex::new(None));

/// 平台默认键位：macOS ⌥Space；Windows/Linux Ctrl+Alt+K（Alt+Space 是系统窗口菜单键，避开）
pub fn default_chat_hotkey() -> &'static str {
    if cfg!(target_os = "macos") { "Alt+Space" } else { "Ctrl+Alt+K" }
}

/// 键位规范化（纯函数，可单测）：`"⌥ space"`/`"alt+space"`/`"ALT+SPACE"` → `"Alt+Space"`。
/// 规则：`修饰键(≥1)...+实体键`；实体键支持 字母A-Z / 数字0-9 / Space / Enter / Escape / Tab / F1-F12。
pub(crate) fn normalize_chat_hotkey(raw: &str) -> Result<String, String> {
    let parts: Vec<String> = raw
        .split('+')
        .map(|p| p.trim().to_uppercase())
        .filter(|p| !p.is_empty())
        .collect();
    if parts.len() < 2 {
        return Err("快捷键需至少包含一个修饰键（Ctrl/Alt/Shift/Cmd）和一个按键".to_string());
    }
    let (key_part, mod_parts) = parts.split_last().unwrap();
    let mut mods: Vec<&str> = Vec::new();
    for m in mod_parts {
        let norm = match m.as_str() {
            "CTRL" | "CONTROL" | "⌃" => "Ctrl",
            "ALT" | "OPTION" | "⌥" => "Alt",
            "SHIFT" | "⇧" => "Shift",
            "CMD" | "COMMAND" | "META" | "SUPER" | "⌘" => "Cmd",
            _ => return Err("含未知修饰键，仅支持 Ctrl/Alt/Shift/Cmd".to_string()),
        };
        if !mods.contains(&norm) { mods.push(norm); }
    }
    let k = match key_part.as_str() {
        "SPACE" => "Space".to_string(),
        "ENTER" | "RETURN" => "Enter".to_string(),
        "ESCAPE" | "ESC" => "Escape".to_string(),
        "TAB" => "Tab".to_string(),
        s if s.chars().count() == 1 && s.chars().next().map_or(false, |c| c.is_ascii_alphabetic()) => s.to_string(),
        s if s.chars().count() == 1 && s.chars().next().map_or(false, |c| c.is_ascii_digit()) => s.to_string(),
        s if s.is_ascii() && s.starts_with('F') && s.len() <= 3 && s[1..].parse::<u8>().map_or(false, |n| (1..=12).contains(&n)) => s.to_string(),
        _ => return Err("按键仅支持 字母/数字/Space/Enter/Escape/Tab/F1-F12".to_string()),
    };
    Ok(format!("{}+{}", mods.join("+"), k))
}

fn stored_chat_hotkey() -> Option<String> {
    database::get("SELECT value FROM system_settings WHERE key='chat_hotkey'", &[])
        .ok()
        .flatten()
        .and_then(|r| r["value"].as_str().map(|s| s.to_string()))
        .filter(|s| !s.trim().is_empty())
}

fn persist_chat_hotkey(key: &str) -> Result<(), String> {
    database::run(
        "INSERT OR REPLACE INTO system_settings(key, value, updated_at) VALUES('chat_hotkey', ?1, datetime('now','localtime'))",
        &[json!(key)],
    )
    .map(|_| ()).map_err(|e| e.to_string())
}

fn clear_chat_hotkey() {
    database::run("DELETE FROM system_settings WHERE key='chat_hotkey'", &[]).ok();
}

fn registered_key() -> Option<String> {
    REGISTERED.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// 注册并记录；失败清注册态。返回是否成功
fn do_register(app: &AppHandle, key: &str) -> bool {
    match app.global_shortcut().register(key) {
        Ok(()) => { *REGISTERED.lock().unwrap_or_else(|e| e.into_inner()) = Some(key.to_string()); log::info!("[Hotkey] 已注册浮窗快捷键: {}", key); true }
        Err(e) => { *REGISTERED.lock().unwrap_or_else(|e| e.into_inner()) = None; log::warn!("[Hotkey] 注册失败 {}: {}", key, e); false }
    }
}

fn do_unregister(app: &AppHandle, key: &str) {
    let _ = app.global_shortcut().unregister(key);
}

/// 启动注册（setup 内、DB 初始化后调用）：存储值优先，否则平台默认；失败保持未注册（仍可用精灵）
pub fn init_chat_hotkey(app: &AppHandle) {
    let key = match stored_chat_hotkey().as_deref().map(normalize_chat_hotkey) {
        Some(Ok(k)) => k,
        Some(Err(e)) => { log::warn!("[Hotkey] 存储值非法，回退默认: {e}"); default_chat_hotkey().to_string() }
        None => default_chat_hotkey().to_string(),
    };
    do_register(app, &key);
}

/// 当前有效键位 + 注册状态（设置页展示）
#[tauri::command]
pub async fn get_chat_hotkey() -> Result<Value, String> {
    let stored = stored_chat_hotkey().and_then(|s| normalize_chat_hotkey(&s).ok());
    Ok(json!({
        "key": registered_key().or(stored).unwrap_or_else(|| default_chat_hotkey().to_string()),
        "defaultKey": default_chat_hotkey(),
        "registered": registered_key().is_some(),
        "usingDefault": stored_chat_hotkey().is_none(),
    }))
}

/// 设置快捷键： unregister旧 → register新 → 成功才落库；失败回滚旧绑定
#[tauri::command]
pub async fn set_chat_hotkey(app: AppHandle, key: Option<String>) -> Result<Value, String> {
    // key=None → 清除自定义，回落平台默认
    let nk = match key.as_deref() {
        Some(raw) => normalize_chat_hotkey(raw)?,
        None => default_chat_hotkey().to_string(),
    };
    let old = registered_key();
    if let Some(o) = &old { do_unregister(&app, o); }
    if do_register(&app, &nk) {
        if key.is_some() { persist_chat_hotkey(&nk)?; } else { clear_chat_hotkey(); }
        Ok(json!({ "key": nk, "registered": true }))
    } else {
        // 回滚旧绑定
        let rollback_ok = match &old {
            Some(o) => do_register(&app, o),
            None => false,
        };
        if !rollback_ok {
            // 旧键也被占（极端）：恢复启动配置重注册默认键
            do_register(&app, default_chat_hotkey());
        }
        Err("该快捷键注册失败（可能被其他应用占用），已保持原绑定".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_chat_hotkey;

    #[test]
    fn normalizes_case_and_aliases() {
        assert_eq!(normalize_chat_hotkey(" alt + space ").unwrap(), "Alt+Space");
        assert_eq!(normalize_chat_hotkey("CONTROL+ALT+k").unwrap(), "Ctrl+Alt+K");
        assert_eq!(normalize_chat_hotkey("cmd+shift+esc").unwrap(), "Cmd+Shift+Escape");
        assert_eq!(normalize_chat_hotkey("ctrl+ENTER").unwrap(), "Ctrl+Enter");
        assert_eq!(normalize_chat_hotkey("ctrl+f5").unwrap(), "Ctrl+F5");
        assert_eq!(normalize_chat_hotkey("ctrl+alt+alt+m").unwrap(), "Ctrl+Alt+M"); // 修饰键去重
    }

    #[test]
    fn rejects_invalid() {
        assert!(normalize_chat_hotkey("space").is_err()); // 无修饰键
        assert!(normalize_chat_hotkey("ctrl+").is_err()); // 无按键
        assert!(normalize_chat_hotkey("ctrl+alt").is_err()); // 末段仍是修饰键 → 非法按键
        assert!(normalize_chat_hotkey("ctrl+§").is_err());
        assert!(normalize_chat_hotkey("ctrl+alt+f13").is_err());
        assert!(normalize_chat_hotkey("hyper+x").is_err());
    }

    #[test]
    fn accepts_symbol_modifiers() {
        assert_eq!(normalize_chat_hotkey("⌥+space").unwrap(), "Alt+Space");
        assert_eq!(normalize_chat_hotkey("⌃+⌥+K").unwrap(), "Ctrl+Alt+K");
        assert_eq!(normalize_chat_hotkey("⇧+⌘+escape").unwrap(), "Shift+Cmd+Escape");
    }

    #[test]
    fn accepts_single_letters_digits_tab() {
        assert_eq!(normalize_chat_hotkey("ALT+j").unwrap(), "Alt+J");
        assert_eq!(normalize_chat_hotkey("CTRL+1").unwrap(), "Ctrl+1");
        assert_eq!(normalize_chat_hotkey("CTRL+SHIFT+tab").unwrap(), "Ctrl+Shift+Tab");
    }
}
