use tauri::{AppHandle, Emitter, Manager, WebviewWindow, WebviewWindowBuilder, WebviewUrl};
use std::sync::atomic::{AtomicBool, Ordering};

// 失焦自动隐藏监听器只绑定一次（窗口由配置创建，可能多次调用 create_chat_window）
static CHAT_EVENT_SETUP: AtomicBool = AtomicBool::new(false);
// 记录聊天打开前主窗口是否可见（macOS：打开聊天时隐藏主窗口，关闭时恢复）
static MAIN_WAS_VISIBLE: AtomicBool = AtomicBool::new(false);

/// macOS：将窗口设为辅助窗口（不参与前台调度/Stage Manager 分组）
#[cfg(target_os = "macos")]
fn set_auxiliary_window(win: &WebviewWindow) {
    use objc2_app_kit::{NSWindow, NSWindowCollectionBehavior};
    if let Ok(ns_window_ptr) = win.ns_window() {
        let ns_window: &NSWindow = unsafe { &*ns_window_ptr.cast() };
        let behavior = NSWindowCollectionBehavior::Auxiliary
            | NSWindowCollectionBehavior::Stationary
            | NSWindowCollectionBehavior::FullScreenAuxiliary;
        ns_window.setCollectionBehavior(behavior);
    }
}

#[cfg(not(target_os = "macos"))]
fn set_auxiliary_window(_win: &WebviewWindow) {}

/// 隐藏主窗口（记录可见状态，聊天关闭时恢复）
fn hide_main_window(app: &AppHandle) {
    if let Some(main) = app.get_webview_window("main") {
        let visible = main.is_visible().unwrap_or(false);
        MAIN_WAS_VISIBLE.store(visible, Ordering::Relaxed);
        if visible { let _ = main.hide(); }
    }
}

/// 若聊天打开前主窗口可见，则恢复显示
fn restore_main_window(app: &AppHandle) {
    if MAIN_WAS_VISIBLE.load(Ordering::Relaxed) {
        if let Some(main) = app.get_webview_window("main") {
            let _ = main.show();
        }
        MAIN_WAS_VISIBLE.store(false, Ordering::Relaxed);
    }
}

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

    set_auxiliary_window(&win);
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
        set_auxiliary_window(&win);
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
                        let _ = chat_win.hide();
                        // ★ 关闭时恢复主窗口（若聊天打开前可见）
                        restore_main_window(&handle);
                        // ★ 关闭时清空展开状态
                        let _ = handle.emit("chat:reset", ());
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
            let _ = win.hide();
            // ★ 关闭时恢复主窗口（若聊天打开前可见）
            restore_main_window(&app);
            // ★ 关闭时清空展开状态（下次打开无收起动画）
            let _ = app.emit("chat:reset", ());
            // ★ 关闭时位置重置回收起态（不可见，无动画）
            let scale = win.scale_factor().unwrap_or(1.0);
            let cur_x = win.outer_position().map(|p| p.x as f64 / scale).unwrap_or(0.0);
            let collapsed_y = collapsed_chat_y(&app);
            let _ = win.set_position(tauri::LogicalPosition::new(cur_x, collapsed_y));
        } else {
            // ★ 显示前隐藏主窗口（macOS：避免主窗口被带到前台）+ 恢复收起态位置 + 绑定失焦监听
            hide_main_window(&app);
            position_chat_window_collapsed(&app, &win, width, height);
            setup_chat_window_events(&app);
            let _ = win.show();
            let _ = win.set_focus();
        }
    } else {
        // 聊天窗口不存在，先隐藏主窗口再创建显示
        hide_main_window(&app);
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
