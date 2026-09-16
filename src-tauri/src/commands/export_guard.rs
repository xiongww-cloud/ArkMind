//! 导出/保存路径守卫：写盘命令只接受"用户刚刚在原生保存对话框里确认过"的路径。
//! 防止渲染层一旦被注入（XSS）后可借 export 类命令向磁盘任意位置写文件。

use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Mutex;
use serde_json::{json, Value};

static ALLOWED: Mutex<Option<(HashSet<PathBuf>, VecDeque<PathBuf>)>> = Mutex::new(None);

const MAX_KEEP: usize = 50;

fn normalize(p: &PathBuf) -> Option<PathBuf> {
    // 目标文件通常尚不存在：canonicalize 父目录后接回文件名
    let (dir, name) = match (p.parent(), p.file_name()) {
        (Some(d), Some(n)) => (d, Some(n.to_os_string())),
        _ => (p.as_path(), None),
    };
    let canon = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    Some(match name {
        Some(n) => canon.join(n),
        None => canon,
    })
}

fn register_inner(path: PathBuf) {
    let mut guard = ALLOWED.lock().unwrap_or_else(|e| e.into_inner());
    let (set, order) = guard.get_or_insert_with(|| (HashSet::new(), VecDeque::new()));
    let np = normalize(&path).unwrap_or(path);
    if set.contains(&np) { return; }
    set.insert(np.clone());
    order.push_back(np);
    while order.len() > MAX_KEEP {
        if let Some(old) = order.pop_front() { set.remove(&old); }
    }
}

/// 校验 save_path 是否曾经由 export_pick_save_path 用户确认；未登记则拒绝
pub fn ensure_registered(save_path: &str) -> Result<(), String> {
    if save_path.trim().is_empty() { return Err("未指定保存路径".into()); }
    let p = PathBuf::from(save_path);
    let np = normalize(&p).ok_or_else(|| "保存路径无效".to_string())?;
    let guard = ALLOWED.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((set, _)) = guard.as_ref() {
        if set.contains(&np) { return Ok(()); }
    }
    Err("保存路径未经对话框确认，已拒绝写入".into())
}

/// 前端替代 plugin-dialog save()：后端弹保存框，用户确认后路径登记白名单并返回
#[tauri::command]
pub async fn export_pick_save_path(
    app: tauri::AppHandle,
    default_path: Option<String>,
    filter_extensions: Option<Vec<String>>,
) -> Result<Value, String> {
    use tauri_plugin_dialog::DialogExt;
    let mut builder = app.dialog().file();
    if let Some(dp) = default_path.as_deref().filter(|s| !s.is_empty()) {
        builder = builder.set_file_name(dp);
    }
    let exts: Vec<String> = filter_extensions.unwrap_or_default()
        .into_iter()
        .map(|e| e.trim_start_matches('.').to_lowercase())
        .filter(|e| !e.is_empty() && e.chars().all(|c| c.is_ascii_alphanumeric()))
        .collect();
    if !exts.is_empty() {
        builder = builder.add_filter("导出文件", &exts.iter().map(|s| s.as_str()).collect::<Vec<_>>());
    }
    // async 命令不在主线程，blocking 可用
    let picked = tokio::task::spawn_blocking(move || builder.blocking_save_file())
        .await
        .map_err(|e| format!("对话框任务失败: {}", e))?;
    match picked {
        Some(fp) => {
            let path = fp.into_path().map_err(|e| format!("路径无效: {}", e))?;
            register_inner(path.clone());
            Ok(json!({ "cancelled": false, "path": path.to_string_lossy() }))
        }
        None => Ok(json!({ "cancelled": true })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unregistered_path_rejected() {
        assert!(ensure_registered("").is_err());
        assert!(ensure_registered("C:\\definitely-not-picked-12345\\out.zip").is_err());
    }

    #[test]
    fn registered_parent_dir_path_matches() {
        let dir = std::env::temp_dir();
        let target = dir.join(format!("arkmind_guard_test_{}.zip", std::process::id()));
        register_inner(target.clone());
        assert!(ensure_registered(&target.to_string_lossy()).is_ok());
        let other = dir.join(format!("arkmind_guard_other_{}.zip", std::process::id()));
        assert!(ensure_registered(&other.to_string_lossy()).is_err());
        let _ = std::fs::remove_file(&target);
    }
}
