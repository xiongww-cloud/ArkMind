use tauri::{AppHandle, Emitter, Manager, WebviewWindow, WebviewWindowBuilder, WebviewUrl};
use std::sync::atomic::{AtomicBool, Ordering};

// 失焦自动隐藏监听器只绑定一次（窗口由配置创建，可能多次调用 create_chat_window）
static CHAT_EVENT_SETUP: AtomicBool = AtomicBool::new(false);

/// macOS：将窗口设为辅助窗口（不参与前台调度/Stage Manager 分组）
/// 注意：必须在主线程调用 AppKit API（使用 run_on_main_thread 调度）
#[cfg(target_os = "macos")]
fn set_auxiliary_window(app: &AppHandle, label: &str) {
    let app = app.clone();
    let label = label.to_string();
    // 方法接收者用克隆，move 闭包持有 app 副本
    let _ = app.clone().run_on_main_thread(move || {
        use objc2_app_kit::{NSWindow, NSWindowCollectionBehavior};
        if let Some(win) = app.get_webview_window(&label) {
            if let Ok(ptr) = win.ns_window() {
                if ptr.is_null() { return; }
                let ns_window: &NSWindow = unsafe { &*ptr.cast() };
                let behavior = NSWindowCollectionBehavior::Auxiliary
                    | NSWindowCollectionBehavior::Stationary
                    | NSWindowCollectionBehavior::FullScreenAuxiliary;
                ns_window.setCollectionBehavior(behavior);
            }
        }
    });
}

#[cfg(not(target_os = "macos"))]
fn set_auxiliary_window(_app: &AppHandle, _label: &str) {}

/// macOS：精灵窗口行为（Stationary + FullScreenAuxiliary，不含 Auxiliary 以确保能接收鼠标事件）
/// 注意：必须在主线程调用 AppKit API
#[cfg(target_os = "macos")]
fn set_sprite_window_behavior(app: &AppHandle, label: &str) {
    let app = app.clone();
    let label = label.to_string();
    let _ = app.clone().run_on_main_thread(move || {
        use objc2_app_kit::{NSWindow, NSWindowCollectionBehavior};
        if let Some(win) = app.get_webview_window(&label) {
            if let Ok(ptr) = win.ns_window() {
                if ptr.is_null() { return; }
                let ns_window: &NSWindow = unsafe { &*ptr.cast() };
                let behavior = NSWindowCollectionBehavior::Stationary
                    | NSWindowCollectionBehavior::FullScreenAuxiliary;
                ns_window.setCollectionBehavior(behavior);
            }
        }
    });
}

#[cfg(not(target_os = "macos"))]
fn set_sprite_window_behavior(_app: &AppHandle, _label: &str) {}

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
        let _ = win.set_always_on_top(true);
        let _ = win.show();
        let _ = win.set_focus();
        return Ok(());
    }

    // 代码创建精灵窗口（透明/置顶/无边框，macOS 依赖 macos-private-api feature）
    let win = WebviewWindowBuilder::new(&app, "sprite", WebviewUrl::App("sprite.html".into()))
        .inner_size(48.0, 74.0)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .resizable(false)
        .skip_taskbar(true)
        .shadow(false)
        .build()
        .map_err(|e| e.to_string())?;

    // 精灵窗口：Stationary（不出现在 Mission Control）但不含 Auxiliary，确保能接收鼠标事件
    set_sprite_window_behavior(&app, "sprite");
    let _ = win.show();
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
            let _ = win.set_always_on_top(true);
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

/// 创建聊天窗口（透明窗口，初始位于收起态：屏幕底部）
#[tauri::command]
pub async fn create_chat_window(app: AppHandle, width: Option<f64>, height: Option<f64>) -> Result<(), String> {
    let win = if let Some(win) = app.get_webview_window("chat") {
        win
    } else {
        // 代码创建聊天窗口（透明/浮窗层级，macOS 依赖 macos-private-api feature）
        let win_w = width.unwrap_or(620.0);
        let win_h = height.unwrap_or(700.0);
        let win = WebviewWindowBuilder::new(&app, "chat", WebviewUrl::App("chat.html".into()))
            .inner_size(win_w, win_h)
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .shadow(false)
            .resizable(false)
            .skip_taskbar(true)
            .visible(true)
            .build()
            .map_err(|e| e.to_string())?;
        set_auxiliary_window(&app, "chat");
        win
    };

    position_chat_window_collapsed(&app, &win, width, height);
    setup_chat_window_events(&app);
    let _ = win.show();
    let _ = win.set_focus();

    Ok(())
}

/// 设置聊天窗口收起态位置（屏幕底部居中）
fn position_chat_window_collapsed(app: &AppHandle, win: &WebviewWindow, width: Option<f64>, height: Option<f64>) {
    let win_w = width.unwrap_or(620.0);
    let win_h = height.unwrap_or(700.0);

    // 屏幕宽度（水平居中）
    let screen_w = if let Some(main_win) = app.get_webview_window("main") {
        main_win.primary_monitor().ok().flatten()
            .map(|m| m.size().width as f64 / m.scale_factor())
            .unwrap_or(1920.0)
    } else { 1920.0 };

    // 收起态：输入框在屏幕底部可见
    let collapsed_y = collapsed_chat_y(app);
    let x = (screen_w - win_w) / 2.0;

    let _ = win.set_size(tauri::LogicalSize::new(win_w, win_h));
    let _ = win.set_position(tauri::LogicalPosition::new(x, collapsed_y));
}

/// 绑定失焦自动隐藏监听器（只绑定一次，窗口由配置创建可能复用）
fn setup_chat_window_events(app: &AppHandle) {
    if CHAT_EVENT_SETUP.swap(true, Ordering::Relaxed) { return; }
    let handle = app.clone();
    if let Some(win) = app.get_webview_window("chat") {
        win.on_window_event(move |event| {
            if let tauri::WindowEvent::Focused(focused) = event {
                if !focused {
                    if let Some(chat_win) = handle.get_webview_window("chat") {
                        // ★ 先重置前端展开状态，再隐藏窗口
                        let _ = handle.emit("chat:reset", ());
                        let _ = chat_win.hide();
                        // ★ 关闭时位置重置回收起态
                        let scale = chat_win.scale_factor().unwrap_or(1.0);
                        let cur_x = chat_win.outer_position().map(|p| p.x as f64 / scale).unwrap_or(0.0);
                        let collapsed_y = collapsed_chat_y(&handle);
                        let _ = chat_win.set_position(tauri::LogicalPosition::new(cur_x, collapsed_y));
                    }
                }
            }
        });
    }
}

/// 展开/收起聊天窗口（瞬时移动 Y 坐标，配合前端 CSS 动画同步）
#[tauri::command]
pub async fn expand_chat_window(app: AppHandle, expanded: bool) -> Result<(), String> {
    let win = app.get_webview_window("chat").ok_or("chat window not found")?;
    let scale = win.scale_factor().unwrap_or(1.0);

    let collapsed_y = collapsed_chat_y(&app);
    let delta = 564.0;  // = 窗口高 - 底部边距(68) - 输入栏高(48) - 顶部边距(20) + 修正 = 700-136
    let expanded_y = collapsed_y - delta;

    let target_y = if expanded { expanded_y } else { collapsed_y };

    let cur_x = win.outer_position().map(|p| p.x as f64 / scale).unwrap_or(0.0);
    win.set_position(tauri::LogicalPosition::new(cur_x, target_y))
        .map_err(|e| e.to_string())?;

    Ok(())
}

/// 计算聊天窗口收起态的 Y 坐标（屏幕底 - 168）
fn collapsed_chat_y(app: &AppHandle) -> f64 {
    let screen_h = if let Some(main_win) = app.get_webview_window("main") {
        main_win.primary_monitor().ok().flatten()
            .map(|m| m.size().height as f64 / m.scale_factor())
            .unwrap_or(1080.0)
    } else { 1080.0 };
    screen_h - 168.0
}

/// 切换聊天窗口显示/隐藏
#[tauri::command]
pub async fn toggle_chat_window(app: AppHandle, width: Option<f64>, height: Option<f64>) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("chat") {
        if win.is_visible().unwrap_or(false) {
            // ★ 先重置前端展开状态，再隐藏窗口
            let _ = app.emit("chat:reset", ());
            let _ = win.hide();
            // ★ 关闭时位置重置回收起态（不可见，无动画）
            let scale = win.scale_factor().unwrap_or(1.0);
            let cur_x = win.outer_position().map(|p| p.x as f64 / scale).unwrap_or(0.0);
            let collapsed_y = collapsed_chat_y(&app);
            let _ = win.set_position(tauri::LogicalPosition::new(cur_x, collapsed_y));
        } else {
            // ★ 先重置前端展开状态，再显示窗口（避免展开→收起动画）
            let _ = app.emit("chat:reset", ());
            position_chat_window_collapsed(&app, &win, width, height);
            setup_chat_window_events(&app);
            let _ = win.show();
            let _ = win.set_focus();
        }
    } else {
        create_chat_window(app, width, height).await?;
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
