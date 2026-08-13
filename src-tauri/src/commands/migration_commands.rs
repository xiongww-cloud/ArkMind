use crate::database;
use crate::database::project_id_migration::{self, IdPair};
use serde_json::{json, Value};

/// 迁移前数据库快照：checkpoint → 复制 DB(+wal/shm) → 完整性校验 → 打 zip。
/// 返回 zip 路径。备份未通过校验则拒绝启动迁移。
fn backup_database_for_migration() -> Result<String, String> {
    let _ = database::with_db(|conn| conn.pragma_update(None, "wal_checkpoint", "TRUNCATE"));

    let data_dir = crate::get_data_dir();
    let db_path = data_dir.join("arkmind.db");
    if !db_path.exists() {
        return Err("数据库文件不存在，无法备份".to_string());
    }

    let backups_dir = data_dir.join("backups");
    std::fs::create_dir_all(&backups_dir).map_err(|e| e.to_string())?;

    let ts = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
    let staging = backups_dir.join(format!("pre_id_migration_{}", ts));
    std::fs::create_dir_all(&staging).map_err(|e| e.to_string())?;

    std::fs::copy(&db_path, staging.join("arkmind.db")).map_err(|e| e.to_string())?;
    let wal = db_path.with_extension("db-wal");
    let shm = db_path.with_extension("db-shm");
    if wal.exists() { std::fs::copy(&wal, staging.join("arkmind.db-wal")).map_err(|e| e.to_string())?; }
    if shm.exists() { std::fs::copy(&shm, staging.join("arkmind.db-shm")).map_err(|e| e.to_string())?; }

    let live_projects: i64 = database::with_db(|conn| {
        conn.query_row("SELECT COUNT(*) FROM projects", [], |r| r.get(0))
    }).map_err(|e| e.to_string())?;

    let backup_conn = rusqlite::Connection::open(staging.join("arkmind.db")).map_err(|e| e.to_string())?;
    let backup_projects: i64 = backup_conn
        .query_row("SELECT COUNT(*) FROM projects", [], |r| r.get(0))
        .map_err(|e| format!("备份完整性校验失败: {}", e))?;
    drop(backup_conn);
    if backup_projects != live_projects {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(format!("备份完整性校验失败: live={} backup={}", live_projects, backup_projects));
    }

    let zip_path = backups_dir.join(format!("pre_id_migration_{}.zip", ts));
    crate::commands::export_commands::create_zip_archive(&staging, &zip_path)
        .map_err(|e| format!("备份打包失败: {}", e))?;
    let _ = std::fs::remove_dir_all(&staging);

    log::info!("[ID迁移] 迁移前备份完成: {:?} ({} 个项目)", zip_path, backup_projects);
    Ok(zip_path.to_string_lossy().to_string())
}

/// 阶段 B：前端启动后提交项目身份映射（localId → backendId），触发 canonical ID 迁移。
/// 幂等：state 为 completed/completed_with_conflicts 时直接返回上次结果。
#[tauri::command]
pub async fn migration_submit_project_id_map(map: Vec<IdPair>) -> Result<Value, String> {
    tokio::task::spawn_blocking(move || {
        let state = database::with_db(|conn| Ok(project_id_migration::get_migration_state(conn)))
            .map_err(|e| e.to_string())?;

        let backup_path = if state == "pending" || state == "failed" {
            Some(backup_database_for_migration()?)
        } else {
            None
        };

        let report = database::with_db(|conn| {
            Ok(project_id_migration::migrate_project_ids_canonical(conn, &map))
        }).map_err(|e| e.to_string())??;

        let _ = database::save();

        Ok(json!({
            "backupPath": backup_path,
            "report": serde_json::to_value(&report).map_err(|e| e.to_string())?,
        }))
    }).await.map_err(|e| e.to_string())?
}

/// 查询迁移状态（前端启动时决定是否提交映射）
#[tauri::command]
pub async fn migration_project_id_state() -> Result<Value, String> {
    database::with_db(|conn| {
        let state = project_id_migration::get_migration_state(conn);
        Ok(json!({ "state": state }))
    }).map_err(|e| e.to_string())
}

/// Stage 0 轨 A：在真实数据库的副本上演练迁移，不触碰运行中的库。
#[cfg(debug_assertions)]
#[tauri::command]
pub async fn debug_rehearse_id_migration(map: Vec<IdPair>) -> Result<Value, String> {
    tokio::task::spawn_blocking(move || {
        let _ = database::with_db(|conn| conn.pragma_update(None, "wal_checkpoint", "TRUNCATE"));

        let src = crate::get_data_dir().join("arkmind.db");
        if !src.exists() {
            return Err("数据库文件不存在".to_string());
        }
        let tmp = std::env::temp_dir().join(format!("arkmind_rehearse_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
        let copy_path = tmp.join("arkmind.db");
        std::fs::copy(&src, &copy_path).map_err(|e| e.to_string())?;

        let conn = rusqlite::Connection::open(&copy_path).map_err(|e| e.to_string())?;
        let report = project_id_migration::migrate_project_ids_canonical(&conn, &map)?;

        let total_projects: i64 = conn
            .query_row("SELECT COUNT(*) FROM projects", [], |r| r.get(0))
            .unwrap_or(0);
        let audit_rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM project_id_migration_log", [], |r| r.get(0))
            .unwrap_or(0);
        drop(conn);
        let _ = std::fs::remove_dir_all(&tmp);

        log::info!("[ID迁移演练] state={}, migrated={}, conflicts={}",
            report.state, report.migrated, report.conflicts.len());

        Ok(json!({
            "report": serde_json::to_value(&report).map_err(|e| e.to_string())?,
            "totalProjects": total_projects,
            "auditRows": audit_rows,
        }))
    }).await.map_err(|e| e.to_string())?
}
