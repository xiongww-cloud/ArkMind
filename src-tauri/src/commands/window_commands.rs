use tauri::{AppHandle, Emitter, Manager, WebviewWindow, WebviewWindowBuilder, WebviewUrl};
use std::sync::atomic::{AtomicBool, Ordering};

// 失焦自动隐藏监听器只绑定一次（窗口由配置创建，可能多次调用 create_chat_window）
static CHAT_EVENT_SETUP: AtomicBool = AtomicBool::new(false);

/// 聊天窗口处于"系统对话框模式"（如打开文件选择对话框）时，失焦不自动隐藏
static CHAT_DIALOG_ACTIVE: AtomicBool = AtomicBool::new(false);

/// 设置聊天窗口的对话框模式。active=true 时失焦不隐藏（用于文件选择等原生对话框打开期间）
#[tauri::command]
pub async fn set_chat_dialog_mode(active: bool) -> Result<(), String> {
    CHAT_DIALOG_ACTIVE.store(active, Ordering::Relaxed);
    Ok(())
}

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
                // 非激活/未聚焦时也接收鼠标移动事件，使 CSS :hover 无需先点击即可生效
                ns_window.setAcceptsMouseMovedEvents(true);
            }
        }
    });
}

#[cfg(not(target_os = "macos"))]
fn set_sprite_window_behavior(_app: &AppHandle, _label: &str) {}

/// macOS：主窗 hide 后让出 key/前台，避免隐藏 WKWebView 卡在 key 链上间歇吞 ⌘C/⌘V。
/// 无 chat/sprite 可见时 NSApp.hide，把 Active 完全交还系统。
/// 注意：必须在主线程调用 AppKit API（使用 run_on_main_thread 调度）
#[cfg(target_os = "macos")]
pub fn yield_key_on_main_hide(app: &AppHandle) {
    let app = app.clone();
    let _ = app.clone().run_on_main_thread(move || {
        use objc2::MainThreadMarker;
        use objc2_app_kit::{NSApplication, NSWindow};

        let Some(mtm) = MainThreadMarker::new() else { return };

        // 1. 清主窗 first responder + 幂等 orderOut（hide 竞态兜底）
        if let Some(win) = app.get_webview_window("main") {
            if let Ok(ptr) = win.ns_window() {
                if !ptr.is_null() {
                    let ns_window: &NSWindow = unsafe { &*ptr.cast() };
                    let _ = ns_window.makeFirstResponder(None);
                    if ns_window.isVisible() {
                        let _ = ns_window.orderOut(None);
                    }
                }
            }
        }

        // 2. 无其它可见浮窗时，让 NSApp 彻底 inactive（交还前台）
        let any_aux_visible = ["chat", "sprite"].iter().any(|label| {
            app.get_webview_window(label)
                .and_then(|w| w.is_visible().ok())
                .unwrap_or(false)
        });
        if !any_aux_visible {
            let ns_app = NSApplication::sharedApplication(mtm);
            ns_app.hide(None);
        }
    });
}

#[cfg(not(target_os = "macos"))]
pub fn yield_key_on_main_hide(_app: &AppHandle) {}

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
        // macOS：非激活窗口第一次点击直接传给 webview（避免先聚焦再点击）
        .accept_first_mouse(true)
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

// ─── 聊天浮窗：快呼面板（固定尺寸底部居中，输入常驻面板底部、对话向上生长；窗口零位移，生长全在 webview 内 CSS）───

/// 浮窗固定逻辑尺寸：面板宽 580，四周留透明边距供 CSS 阴影渐变到消失。
/// 窗口 shadow(false) 关闭 OS 阴影、浮起全靠 webview——若窗口只比面板大一丁点，
/// box-shadow 会被窗口边界硬切成矩形。故宽 = 580 + 2×50，高 = 顶部留白 + 面板最大高 616 + 底部阴影留。
const CHAT_WIN_W: f64 = 680.0;
const CHAT_WIN_H: f64 = 700.0;

/// 定位到屏幕底部居中（x 居中，窗口底缘距屏幕底部 32px；面板在窗口内锚底、向上生长）
fn position_chat_window(app: &AppHandle, win: &WebviewWindow) {
    let (screen_w, screen_h) = app.get_webview_window("main")
        .and_then(|m| m.primary_monitor().ok().flatten())
        .map(|m| {
            let sf = m.scale_factor();
            (m.size().width as f64 / sf, m.size().height as f64 / sf)
        })
        .unwrap_or((1920.0, 1080.0));
    let x = (screen_w - CHAT_WIN_W) / 2.0;
    let y = (screen_h - CHAT_WIN_H - 32.0).max(48.0);
    let _ = win.set_size(tauri::LogicalSize::new(CHAT_WIN_W, CHAT_WIN_H));
    let _ = win.set_position(tauri::LogicalPosition::new(x, y));
}

/// 创建（或复用）聊天窗口并唤起
#[tauri::command]
pub async fn create_chat_window(app: AppHandle) -> Result<(), String> {
    let win = match app.get_webview_window("chat") {
        Some(win) => win,
        None => {
            // 代码创建聊天窗口（透明/浮窗层级，macOS 依赖 macos-private-api feature）
            let win = WebviewWindowBuilder::new(&app, "chat", WebviewUrl::App("chat.html".into()))
                .inner_size(CHAT_WIN_W, CHAT_WIN_H)
                .decorations(false)
                .transparent(true)
                .always_on_top(true)
                .shadow(false)
                .resizable(false)
                .skip_taskbar(true)
                .visible(true)
                // macOS 隐藏窗口时禁用后台节流，避免 webview 挂起导致 chat:reset 未处理、
                // 重新显示时以旧状态渲染
                .background_throttling(tauri::utils::config::BackgroundThrottlingPolicy::Disabled)
                .build()
                .map_err(|e| e.to_string())?;
            set_auxiliary_window(&app, "chat");
            win
        }
    };

    position_chat_window(&app, &win);
    setup_chat_window_events(&app);
    let _ = win.show();
    let _ = win.set_focus();

    Ok(())
}

/// 失焦自动隐藏：先通知前端重置会话态（收起/清焦点），再隐藏；不移动窗口
fn setup_chat_window_events(app: &AppHandle) {
    if CHAT_EVENT_SETUP.swap(true, Ordering::Relaxed) { return; }
    let handle = app.clone();
    if let Some(win) = app.get_webview_window("chat") {
        win.on_window_event(move |event| {
            if let tauri::WindowEvent::Focused(focused) = event {
                if !focused && !CHAT_DIALOG_ACTIVE.load(Ordering::Relaxed) {
                    if let Some(chat_win) = handle.get_webview_window("chat") {
                        if chat_win.is_visible().unwrap_or(false) {
                            let _ = handle.emit("chat:reset", ());
                            let _ = chat_win.hide();
                        }
                    }
                }
            }
        });
    }
}

/// 共享切换逻辑：全局快捷键 / 精灵点击 / 命令三入口共用。
/// 隐藏与显示都先 emit chat:reset（前端复位面板态并保持 no-animate 静默）。
pub(crate) async fn toggle_chat(app: &AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("chat") {
        if win.is_visible().unwrap_or(false) {
            let _ = app.emit("chat:reset", ());
            let _ = win.hide();
        } else {
            let _ = app.emit("chat:reset", ());
            position_chat_window(app, &win);
            setup_chat_window_events(app);
            let _ = win.show();
            let _ = win.set_focus();
        }
    } else {
        create_chat_window(app.clone()).await?;
    }
    Ok(())
}

/// 快捷键回调入口（插件 handler 非异步环境：fire-and-forget）
pub(crate) fn toggle_chat_fire_and_forget(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = toggle_chat(&app).await { log::warn!("[Chat] 快捷键切换失败: {e}"); }
    });
}

/// 切换聊天窗口显示/隐藏（精灵点击调用）
#[tauri::command]
pub async fn toggle_chat_window(app: AppHandle) -> Result<(), String> {
    toggle_chat(&app).await
}

/// 关闭聊天窗口（销毁；当前前端用 toggle 隐藏，保留命令兼容）
#[tauri::command]
pub async fn close_chat_window(app: AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("chat") {
        let _ = win.close();
    }
    Ok(())
}
