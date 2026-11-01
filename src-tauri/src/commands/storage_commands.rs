//! 存储位置管理：统一数据目录（DB/文件/向量库整体迁移）。
//! 完整性设计：拷贝-校验-切指针-保留源——校验通过前旧位置一字节不删；
//! 任意时点中断，下次启动仍在旧路径，数据完好（最坏 = 新目录残渣 + 迁移重来）。

use crate::storage_path;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{Emitter, Manager};

static MIGRATING: AtomicBool = AtomicBool::new(false);

/// 设置页「存储位置」状态
#[tauri::command]
pub async fn storage_location() -> Result<Value, String> {
    let (data_dir, is_custom, broken) = storage_path::storage_status();
    let text = storage_path::read_pointer_text().unwrap_or_default();
    let p = storage_path::parse_pointer(&text);
    let previous = p.previous_dir.clone();
    let previous_exists = previous.as_ref().map_or(false, |d| Path::new(d).is_dir());
    Ok(json!({
        "dataDir": data_dir,
        "defaultDir": storage_path::default_data_dir().to_string_lossy(),
        "isCustom": is_custom,
        "broken": broken,
        "previousDir": previous,
        "previousDirExists": previous_exists,
    }))
}

/// 迁移后复核：当前库 integrity_check + files/ 与原目录对账（文件数/总字节）。
/// 判定规则：数据库完整 + 文件不缩减即通过——迁移后新增文件属正常（用户已在新目录产生数据），
/// 只有「当前少于原目录」才视为数据缺失。库用只读直连检测（全局连接关闭时也能校验）。
#[tauri::command]
pub async fn storage_verify_previous() -> Result<Value, String> {
    let text = storage_path::read_pointer_text().unwrap_or_default();
    let p = storage_path::parse_pointer(&text);
    let prev = p.previous_dir.ok_or("无迁移前目录记录（无待校验的旧数据）")?;
    let prev_path = PathBuf::from(&prev);
    if !prev_path.is_dir() {
        return Err("原数据目录已不存在".into());
    }
    let cur = crate::get_data_dir();
    // 当前库完整性：只读直连（不经全局单例——迁移后未重启/连接关闭时也能校验）
    let db_ok: bool = {
        let db = cur.join("arkmind.db");
        match rusqlite::Connection::open_with_flags(
            &db,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ) {
            Ok(conn) => conn
                .query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
                .map(|v| v == "ok")
                .unwrap_or(false),
            Err(_) => false,
        }
    };
    // files 对账：目录不存在按 (0,0)；当前 ≥ 原 = 正常（含迁移后新增）
    let (cur_files, cur_bytes) = dir_stats(&cur.join("files"));
    let (prev_files, prev_bytes) = dir_stats(&prev_path.join("files"));
    let no_shrink = cur_files >= prev_files && cur_bytes >= prev_bytes;
    let exact = cur_files == prev_files && cur_bytes == prev_bytes;
    Ok(json!({
        "dbOk": db_ok,
        "filesMatch": no_shrink,
        "filesExact": exact,
        "hasNewFiles": !exact,
        "current": {"files": cur_files, "bytes": cur_bytes},
        "previous": {"files": prev_files, "bytes": prev_bytes},
        "ok": db_ok && no_shrink,
    }))
}

/// 清理旧数据：删除原目录的数据资产（白名单），永不触碰 storage.json / logs。
/// 成功后指针移除 previousDir（回切入口随之消失——这是单向终点）。
#[tauri::command]
pub async fn storage_cleanup_previous() -> Result<Value, String> {
    if MIGRATING.swap(true, Ordering::Relaxed) {
        return Err("已有存储任务进行中".into());
    }
    let result = run_cleanup_previous();
    MIGRATING.store(false, Ordering::Relaxed);
    result
}

/// 清理白名单（纯函数，可单测）：只删数据资产，指针/日志绝不在此列
fn cleanup_targets(dir: &Path) -> Vec<PathBuf> {
    ["arkmind.db", "arkmind.db-wal", "arkmind.db-shm", "files", "lancedb", "backups"]
        .iter()
        .map(|n| dir.join(n))
        .collect()
}

fn run_cleanup_previous() -> Result<Value, String> {
    let text = storage_path::read_pointer_text().unwrap_or_default();
    let p = storage_path::parse_pointer(&text);
    let prev = p.previous_dir.ok_or("无迁移前目录记录")?;
    let prev_path = PathBuf::from(&prev);
    let cur = crate::get_data_dir();
    let cur_c = cur.canonicalize().unwrap_or(cur.clone());
    let prev_c = prev_path.canonicalize().unwrap_or(prev_path.clone());
    if prev_c == cur_c {
        return Err("原目录与当前数据目录相同，拒绝清理".into());
    }
    if !prev_path.is_dir() {
        return Err("原数据目录已不存在".into());
    }
    // 双保险：绝不删除指针文件与日志目录（即便未来白名单被误改）
    let ptr = storage_path::pointer_path();
    let mut deleted_files = 0u64;
    let mut deleted_bytes = 0u64;
    for target in cleanup_targets(&prev_path) {
        if target == ptr || target.ends_with(storage_path::POINTER_FILE) {
            continue;
        }
        if !target.exists() {
            continue;
        }
        let (f, b) = dir_stats(&target);
        if target.is_dir() {
            std::fs::remove_dir_all(&target).map_err(|e| format!("删除 {:?} 失败: {}", target, e))?;
        } else {
            std::fs::remove_file(&target).map_err(|e| format!("删除 {:?} 失败: {}", target, e))?;
        }
        deleted_files += f;
        deleted_bytes += b;
    }
    // 清 previousDir（保留 dataDir）
    storage_path::write_pointer(Some(&cur), None)
        .map_err(|e| format!("更新指针失败（旧数据已删）: {}", e))?;
    log::info!(
        "[Storage] 旧数据清理完成: {}（{} 文件/{} 字节）",
        prev, deleted_files, deleted_bytes
    );
    Ok(json!({"success": true, "files": deleted_files, "bytes": deleted_bytes}))
}

/// 一键回切原路径：校验原目录库完好 → 指针 dataDir↔previousDir 对调 → 重启生效。
/// 零拷贝——原目录数据在清理前一直完整保留。
#[tauri::command]
pub async fn storage_rollback() -> Result<Value, String> {
    if MIGRATING.swap(true, Ordering::Relaxed) {
        return Err("已有存储任务进行中".into());
    }
    let result = run_rollback();
    MIGRATING.store(false, Ordering::Relaxed);
    result
}

fn run_rollback() -> Result<Value, String> {
    let text = storage_path::read_pointer_text().unwrap_or_default();
    let p = storage_path::parse_pointer(&text);
    let prev = p.previous_dir.ok_or("无迁移前目录记录，无法回切")?;
    let prev_path = PathBuf::from(&prev);
    if !prev_path.is_dir() {
        return Err("原数据目录已不存在，无法回切".into());
    }
    // 原目录库必须完好才允许切回
    let prev_db = prev_path.join("arkmind.db");
    if !prev_db.exists() {
        return Err("原目录缺少数据库文件，无法回切".into());
    }
    {
        let conn = rusqlite::Connection::open(&prev_db)
            .map_err(|e| format!("原目录数据库无法打开: {}", e))?;
        let ok: bool = conn
            .query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
            .map(|v| v == "ok")
            .unwrap_or(false);
        if !ok {
            return Err("原目录数据库完整性校验失败，无法回切".into());
        }
    }
    // 对调：新目录成为 previousDir（迁移后的新数据仍在其上，可再切回）
    let cur = crate::get_data_dir();
    let cur_c = cur.canonicalize().unwrap_or(cur.clone());
    let prev_c = prev_path.canonicalize().unwrap_or(prev_path.clone());
    if prev_c == cur_c {
        return Err("原目录与当前目录相同".into());
    }
    storage_path::write_pointer(Some(&prev_path), Some(&cur))
        .map_err(|e| format!("写入指针失败: {}", e))?;
    log::info!("[Storage] 回切原路径: {} → {}", cur_c.display(), prev_c.display());
    Ok(json!({"success": true, "restartRequired": true, "newDataDir": prev_path.to_string_lossy()}))
}

/// 统计目录（文件数, 字节数）；不存在按 0
fn dir_stats(path: &Path) -> (u64, u64) {
    let mut files = 0u64;
    let mut bytes = 0u64;
    if !path.exists() {
        return (0, 0);
    }
    if path.is_file() {
        return (1, std::fs::metadata(path).map(|m| m.len()).unwrap_or(0));
    }
    for entry in std::fs::read_dir(path).into_iter().flatten().flatten() {
        let (f, b) = dir_stats(&entry.path());
        files += f;
        bytes += b;
    }
    (files, bytes)
}

fn dir_non_empty(path: &Path) -> bool {
    path.is_dir() && std::fs::read_dir(path).map(|mut d| d.next().is_some()).unwrap_or(false)
}

/// 递归拷贝 + 进度事件（每 50 文件或每 8MB 上报一次）
fn copy_dir_recursive(
    app: &tauri::AppHandle,
    src: &Path,
    dst: &Path,
    phase: &str,
    progress: &mut (u64, u64),
    total: (u64, u64),
) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    let mut since_emit = 0u64;
    for entry in std::fs::read_dir(src).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir_recursive(app, &from, &to, phase, progress, total)?;
        } else {
            std::fs::copy(&from, &to).map_err(|e| format!("{:?} → {:?}: {}", from, to, e))?;
            let size = std::fs::metadata(&to).map(|m| m.len()).unwrap_or(0);
            progress.0 += 1;
            progress.1 += size;
            since_emit += size;
            if progress.0 % 50 == 0 || since_emit >= 8 * 1024 * 1024 {
                since_emit = 0;
                let _ = app.emit(
                    "storage:progress",
                    json!({
                        "phase": phase,
                        "current": progress.1,
                        "total": total.1,
                        "files": progress.0,
                        "totalFiles": total.0,
                    }),
                );
            }
        }
    }
    Ok(())
}

/// 迁移到新的统一数据目录：关库 → 拷贝 → 校验 → 切指针（保留源）。
/// 返回 { success, restartRequired: true }；前端负责提示重启。
#[tauri::command]
pub async fn storage_change_dir(app: tauri::AppHandle, new_path: String) -> Result<Value, String> {
    if MIGRATING.swap(true, Ordering::Relaxed) {
        return Err("已有迁移任务进行中".into());
    }
    let result = run_migration(&app, &new_path);
    MIGRATING.store(false, Ordering::Relaxed);
    result
}

fn run_migration(app: &tauri::AppHandle, new_path: &str) -> Result<Value, String> {
    // ── 1. 目标校验 ─────────────────────────────────────────────
    let dest = PathBuf::from(new_path.trim());
    if new_path.trim().is_empty() {
        return Err("目标路径不能为空".into());
    }
    let dest = dest.canonicalize().unwrap_or(dest);
    let src = crate::get_data_dir();
    let src_c = src.canonicalize().unwrap_or(src.clone());
    if dest == src_c {
        return Err("目标目录与当前数据目录相同".into());
    }
    if dest.starts_with(&src_c) || src_c.starts_with(&dest) {
        return Err("目标目录与当前数据目录存在嵌套关系，请选择独立目录".into());
    }
    std::fs::create_dir_all(&dest).map_err(|e| format!("无法创建目标目录: {}", e))?;
    if !dest.is_dir() {
        return Err("目标路径不是目录".into());
    }
    // 目标必须为空——避免与用户已有数据混杂、也保证失败后可整体清理
    if dir_non_empty(&dest) {
        return Err("目标目录非空，请选择空目录（或先清理其中内容）".into());
    }
    if !dest.metadata().map(|m| m.permissions().readonly() == false).unwrap_or(false) {
        // readonly() 对目录的语义较弱，仅作提示性检查；实际写入失败由拷贝阶段返回错误
    }

    let db_src = src.join("arkmind.db");
    if !db_src.exists() {
        return Err("当前数据库文件不存在，无法迁移".into());
    }

    // ── 2. 预估（仅用于进度条初始值；权威对账在关库后重新统计）──
    let mut est_files = 0u64;
    let mut est_bytes = 0u64;
    for name in ["arkmind.db", "arkmind.db-wal", "arkmind.db-shm", "files", "lancedb", "backups"] {
        let (f, b) = dir_stats(&src.join(name));
        est_files += f;
        est_bytes += b;
    }
    let _ = app.emit(
        "storage:progress",
        json!({"phase":"prepare","current":0,"total":est_bytes,"files":0,"totalFiles":est_files}),
    );

    // ── 3. 关库（迁移窗口冻结写入；后台线程查询报错属预期，重启即恢复）──
    // ★ 关库触发 WAL checkpoint：-wal/-shm 被并入主库甚至删除——可拷内容在这一刻才定型，
    //   对账基准必须在此之后统计（关库前统计必然与拷贝结果不一致）
    crate::database::close_db();
    crate::commands::file_commands::invalidate_storage_base();
    crate::services::lancedb_client::invalidate_lancedb_cache();

    let mut total_files = 0u64;
    let mut total_bytes = 0u64;
    for name in ["arkmind.db", "arkmind.db-wal", "arkmind.db-shm", "files", "lancedb", "backups"] {
        let (f, b) = dir_stats(&src.join(name));
        total_files += f;
        total_bytes += b;
    }

    let mut copied_top: Vec<PathBuf> = Vec::new();
    let failure: Result<(), String> = (|| {
        // ── 4. 拷贝（db 三件套 + files/lancedb/backups）──
        let mut progress = (0u64, 0u64);
        let total = (total_files, total_bytes);
        for name in ["arkmind.db", "arkmind.db-wal", "arkmind.db-shm"] {
            let from = src.join(name);
            if !from.exists() {
                continue;
            }
            let to = dest.join(name);
            std::fs::copy(&from, &to).map_err(|e| format!("拷贝 {} 失败: {}", name, e))?;
            copied_top.push(to.clone());
            let size = std::fs::metadata(&to).map(|m| m.len()).unwrap_or(0);
            progress.0 += 1;
            progress.1 += size;
            let _ = app.emit(
                "storage:progress",
                json!({"phase":"database","current":progress.1,"total":total.1,"files":progress.0,"totalFiles":total.0}),
            );
        }
        for name in ["files", "lancedb", "backups"] {
            let from = src.join(name);
            if !from.exists() {
                continue;
            }
            let to = dest.join(name);
            copy_dir_recursive(app, &from, &to, name, &mut progress, total)?;
            copied_top.push(to.clone());
        }

        // ── 5. 校验：目标库可打开且 integrity_check 通过 ──
        let _ = app.emit(
            "storage:progress",
            json!({"phase":"verify","current":progress.1,"total":total.1,"files":progress.0,"totalFiles":total.0}),
        );
        let dest_db = dest.join("arkmind.db");
        {
            let conn = rusqlite::Connection::open(&dest_db)
                .map_err(|e| format!("目标数据库无法打开: {}", e))?;
            let ok: bool = conn
                .query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
                .map(|v| v == "ok")
                .unwrap_or(false);
            if !ok {
                return Err("目标数据库完整性校验失败（integrity_check）".into());
            }
        }
        // 拷贝完整性：逐文件数+字节数对账（关库后源不再变化）
        let mut dst_files = 0u64;
        let mut dst_bytes = 0u64;
        for name in ["arkmind.db", "arkmind.db-wal", "arkmind.db-shm", "files", "lancedb", "backups"] {
            let (f, b) = dir_stats(&dest.join(name));
            dst_files += f;
            dst_bytes += b;
        }
        if dst_files != total_files || dst_bytes != total_bytes {
            return Err(format!(
                "拷贝对账失败：源 {} 文件/{} 字节，目标 {} 文件/{} 字节",
                total_files, total_bytes, dst_files, dst_bytes
            ));
        }
        Ok(())
    })();

    if let Err(e) = failure {
        // ── 6a. 失败：清理目标残渣（要求空目录起步，可整体删）+ 重开源库 ──
        for p in &copied_top {
            if p.is_dir() {
                let _ = std::fs::remove_dir_all(p);
            } else {
                let _ = std::fs::remove_file(p);
            }
        }
        let reopen = crate::database::init_database(&src.join("arkmind.db"));
        if let Err(re) = reopen {
            log::error!("[Storage] 迁移失败后重开数据库失败: {}", re);
        }
        crate::commands::file_commands::invalidate_storage_base();
        return Err(e);
    }

    // ── 6b. 成功：原子写指针（源数据保留不动，作为旧副本；previousDir 供校验/清理/回切）──
    storage_path::write_pointer(Some(&dest), Some(&src)).map_err(|e| {
        // 指针写失败 → 回滚目标（源仍完好）
        for p in &copied_top {
            if p.is_dir() {
                let _ = std::fs::remove_dir_all(p);
            } else {
                let _ = std::fs::remove_file(p);
            }
        }
        let _ = crate::database::init_database(&src.join("arkmind.db"));
        crate::commands::file_commands::invalidate_storage_base();
        format!("写入路径指针失败，已回滚: {}", e)
    })?;

    log::info!(
        "[Storage] 数据目录迁移完成: {} → {}（旧数据保留，确认无误后可手动清理）",
        src_c.display(),
        dest.display()
    );
    let _ = app.emit(
        "storage:progress",
        json!({"phase":"done","current":total_bytes,"total":total_bytes,"files":total_files,"totalFiles":total_files}),
    );
    Ok(json!({ "success": true, "restartRequired": true, "oldDir": src_c.to_string_lossy() }))
}

/// 重启应用（迁移/恢复后生效）
#[tauri::command]
pub async fn app_restart(app: tauri::AppHandle) -> Result<(), String> {
    app.restart();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 清理白名单：只含数据资产，指针文件与日志目录绝不在列
    #[test]
    fn cleanup_targets_never_include_pointer_or_logs() {
        let dir = Path::new("/data/root");
        let targets = cleanup_targets(dir);
        let names: Vec<String> = targets
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(
            names,
            vec!["arkmind.db", "arkmind.db-wal", "arkmind.db-shm", "files", "lancedb", "backups"]
        );
        assert!(!names.iter().any(|n| n == "storage.json"));
        assert!(!names.iter().any(|n| n == "logs"));
        assert!(!targets.iter().any(|p| p == &crate::storage_path::pointer_path()));
    }
}
