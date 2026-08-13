//! File clipboard — 复制文件到系统剪贴板（供资源管理器/Finder/微信等直接粘贴）
//!
//! 平台原语：
//!   - Windows: clipboard-win 的 CF_HDROP（文件列表），任意线程可调用
//!   - macOS:   NSPasteboard 写入 public.file-url（AppKit 需主线程，
//!             由命令层经 run_on_main_thread 调用 copy_files_macos）

/// Windows：把文件路径写入剪贴板（CF_HDROP）
#[cfg(target_os = "windows")]
pub fn set_clipboard_files(paths: &[String]) -> Result<(), String> {
    if paths.is_empty() {
        return Err("没有可复制的文件".to_string());
    }
    // 必须先在当前线程打开剪贴板，且写入前必须 EmptyClipboard（MSDN 要求），
    // 否则 SetClipboardData 虽返回成功但数据不被系统接管（粘贴无效）。
    // Clipboard::new_attempts(10) 带重试地 OpenClipboard；set_file_list_with(DoClear)
    // 先清空再写入；Clipboard 在 Drop 时自动 CloseClipboard。
    match clipboard_win::Clipboard::new_attempts(10) {
        Ok(_clip) => match clipboard_win::raw::set_file_list_with(paths, clipboard_win::options::DoClear) {
            Ok(_) => {
                log::info!("[剪贴板] 已写入 {} 个文件（CF_HDROP）", paths.len());
                Ok(())
            }
            Err(e) => Err(format!("Windows 剪贴板写入失败: {}", e)),
        },
        Err(e) => Err(format!("Windows 剪贴板打开失败: {}", e)),
    }
}

/// macOS：主线程内写入 public.file-url（由命令层 run_on_main_thread 调用）
#[cfg(target_os = "macos")]
pub fn copy_files_macos(paths: &[String]) -> Result<(), String> {
    use objc2_app_kit::{NSPasteboard, NSPasteboardTypeFileURL};
    use objc2_foundation::NSString;

    if paths.is_empty() {
        return Err("没有可复制的文件".to_string());
    }
    objc2::rc::autoreleasepool(|_| {
        let pb = NSPasteboard::generalPasteboard();
        pb.clearContents();
        let file_url_type = unsafe { NSPasteboardTypeFileURL };
        let mut ok_count = 0usize;
        for p in paths {
            let url = format!("file://{}", percent_encode_path(p));
            let ns = NSString::from_str(&url);
            if pb.setString_forType(&ns, &file_url_type) {
                ok_count += 1;
            }
        }
        if ok_count == paths.len() {
            log::info!("[剪贴板] 已写入 {} 个文件（public.file-url）", ok_count);
            Ok(())
        } else {
            Err(format!("macOS 剪贴板写入失败: {}/{}", ok_count, paths.len()))
        }
    })
}

/// 路径 → file URL 编码（空格、中文、% 等需百分号编码，保留 /）
#[cfg(target_os = "macos")]
fn percent_encode_path(p: &str) -> String {
    let mut out = String::with_capacity(p.len() + 8);
    for b in p.bytes() {
        match b {
            b'/' => out.push('/'),
            b'%' => out.push_str("%25"),
            b'#' => out.push_str("%23"),
            b'?' => out.push_str("%3F"),
            b' ' => out.push_str("%20"),
            0x21..=0x7E => out.push(b as char),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}
