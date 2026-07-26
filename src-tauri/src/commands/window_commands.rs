use tauri::{AppHandle, Emitter, Manager, WebviewWindow, WebviewWindowBuilder, WebviewUrl};

#[tauri::command]
pub fn window_minimize(w: WebviewWindow) {
    let _ = w.minimize();
}

#[tauri::command]
pub fn window_toggle_maximize(w: WebviewWindow) {
    if w.is_maximized().unwrap_or(false) {
        let _ = w.unmaximize();
    } else {
        let _ = w.maximize();
    }
}

#[tauri::command]
pub fn window_close(w: WebviewWindow) {
    let _ = w.close();
}

/// 创建精灵窗口
#[tauri::command]
pub async fn create_sprite_window(app: AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("sprite") {
        let _ = win.show();
        let _ = win.set_focus();
        return Ok(());
    }

    // 窗口属性已在 tauri.conf.json 中配置（transparent, decorations, shadow 等）
    // 这里只需要创建并显示
    WebviewWindowBuilder::new(&app, "sprite", WebviewUrl::App("sprite.html".into()))
        .build()
        .map_err(|e| e.to_string())?;

    let _ = app.get_webview_window("sprite").map(|w| w.show());
    Ok(())
}

/// 切换精灵窗口显示/隐藏
#[tauri::command]
pub async fn toggle_sprite_window(app: AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("sprite") {
        if win.is_visible().unwrap_or(false) {
            let _ = win.hide();
        } else {
            // 先恢复位置，再显示
            if let Ok(pos) = crate::commands::settings_commands::get_sprite_position().await {
                let x = pos["x"].as_f64().unwrap_or(800.0);
                let y = pos["y"].as_f64().unwrap_or(400.0);
                let _ = win.set_position(tauri::LogicalPosition::new(x, y));
            }
            let _ = win.show();
            let _ = win.set_focus();
        }
    } else {
        create_sprite_window(app).await?;
    }
    Ok(())
}

/// 关闭精灵窗口
#[tauri::command]
pub async fn close_sprite_window(app: AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("sprite") {
        let _ = win.close();
    }
    Ok(())
}

/// 设置精灵窗口位置
#[tauri::command]
pub async fn set_sprite_position(app: AppHandle, x: f64, y: f64) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("sprite") {
        let _ = win.set_position(tauri::LogicalPosition::new(x, y));
    }
    Ok(())
}

/// 创建聊天窗口
#[tauri::command]
pub async fn create_chat_window(app: AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("chat") {
        let _ = win.show();
        let _ = win.set_focus();
        return Ok(());
    }

    let chat_w = 400.0_f64;
    let chat_h = 60.0_f64;

    // 屏幕水平居中，底部留100px
    let (x, y) = if let Some(main_win) = app.get_webview_window("main") {
        if let Ok(Some(monitor)) = main_win.primary_monitor() {
            let scale = monitor.scale_factor();
            let phys = monitor.size();
            let screen_w = phys.width as f64 / scale;
            let screen_h = phys.height as f64 / scale;
            ((screen_w - chat_w) / 2.0, screen_h - chat_h - 100.0)
        } else {
            (100.0, 100.0)
        }
    } else {
        (100.0, 100.0)
    };

    WebviewWindowBuilder::new(&app, "chat", WebviewUrl::App("chat.html".into()))
        .title("AI 助手")
        .inner_size(chat_w, chat_h)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .resizable(false)
        .skip_taskbar(true)
        .visible(true)
        .position(x, y)
        .build()
        .map_err(|e| e.to_string())?;

    Ok(())
}

/// 切换聊天窗口显示/隐藏
#[tauri::command]
pub async fn toggle_chat_window(app: AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("chat") {
        if win.is_visible().unwrap_or(false) {
            let _ = win.hide();
        } else {
            let _ = win.show();
            let _ = win.set_focus();
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            let _ = app.emit("chat:reset", ());
        }
    } else {
        create_chat_window(app).await?;
    }
    Ok(())
}

/// 关闭聊天窗口
#[tauri::command]
pub async fn close_chat_window(app: AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("chat") {
        let _ = win.close();
    }
    Ok(())
}

/// 聊天窗口定位到精灵窗口旁边
#[tauri::command]
pub async fn position_chat_near_sprite(app: AppHandle) -> Result<(), String> {
    let chat_win = app.get_webview_window("chat");
    let sprite_win = app.get_webview_window("sprite");

    if let (Some(chat), Some(sprite)) = (chat_win, sprite_win) {
        if let (Ok(sprite_pos), Ok(sprite_size)) = (sprite.outer_position(), sprite.outer_size()) {
            let x = sprite_pos.x as f64 + sprite_size.width as f64 + 10.0;
            let y = sprite_pos.y as f64;
            let _ = chat.set_position(tauri::LogicalPosition::new(x, y));
        }
    }
    Ok(())
}

/// 平滑调整聊天窗口大小（宽高同时变化），底部居中固定
#[tauri::command]
pub async fn resize_chat_window_smooth(app: AppHandle, expanded: bool) -> Result<(), String> {
    use std::time::Duration;

    let win = app.get_webview_window("chat").ok_or("chat window not found")?;
    let compact_w = 400.0_f64;
    let compact_h = 60.0_f64;
    let expanded_w = 600.0_f64;
    let expanded_h = 640.0_f64;
    let steps = 10u32;

    let scale = win.scale_factor().unwrap_or(1.0);

    // 动画开始前一次性获取中心X和底部Y（逻辑像素）
    let (center_x, bottom_y) = if let Ok(pos) = win.outer_position() {
        let cur_w = if let Ok(sz) = win.inner_size() {
            sz.width as f64 / scale
        } else {
            if expanded { compact_w } else { expanded_w }
        };
        let cur_h = if let Ok(sz) = win.inner_size() {
            sz.height as f64 / scale
        } else {
            if expanded { compact_h } else { expanded_h }
        };
        let lx = pos.x as f64 / scale;
        let ly = pos.y as f64 / scale;
        ((lx + cur_w / 2.0), (ly + cur_h))
    } else {
        (300.0, 700.0)
    };

    let from_w = if expanded { compact_w } else { expanded_w };
    let from_h = if expanded { compact_h } else { expanded_h };
    let to_w = if expanded { expanded_w } else { compact_w };
    let to_h = if expanded { expanded_h } else { compact_h };

    for i in 1..=steps {
        let t = i as f64 / steps as f64;
        let t = 1.0 - (1.0 - t).powi(3); // easeOutCubic
        let cur_w = from_w + (to_w - from_w) * t;
        let cur_h = from_h + (to_h - from_h) * t;
        let new_x = center_x - cur_w / 2.0;
        let new_y = bottom_y - cur_h;

        let _ = win.set_position(tauri::LogicalPosition::new(new_x, new_y));
        let _ = win.set_size(tauri::LogicalSize::new(cur_w, cur_h));
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    Ok(())
}
