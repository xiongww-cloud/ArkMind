use tauri::WebviewWindow;

#[tauri::command] pub fn window_minimize(w: WebviewWindow) { let _ = w.minimize(); }
#[tauri::command] pub fn window_toggle_maximize(w: WebviewWindow) { if w.is_maximized().unwrap_or(false) { let _ = w.unmaximize(); } else { let _ = w.maximize(); } }
#[tauri::command] pub fn window_close(w: WebviewWindow) { let _ = w.close(); }
