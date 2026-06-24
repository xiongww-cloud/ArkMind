//! 独立清理管线：异步删除 source_data → 级联 knowledge_chunks + FTS5 → 清理向量
//!
//! 由业务删除命令入队（enqueue_cleanup），独立调度循环处理。
//! 不依赖 knowledge_pipeline，互不阻塞。

use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::LazyLock;
use std::time::Duration;

static STARTED: AtomicBool = AtomicBool::new(false);
static WAKE: LazyLock<tokio::sync::Notify> = LazyLock::new(tokio::sync::Notify::new);

/// 唤醒清理管线（由业务删除命令调用，零延迟异步通知）
pub fn wake_cleanup() {
    WAKE.notify_one();
}

/// 将清理任务加入队列
pub fn enqueue_cleanup(source_type: &str, source_id: &str) {
    // ★ 快照当前 source_data_id 和 chunk_ids，存入队列（不依赖后续 source_data 存在）
    let sd_id: Option<String> = crate::database::get(
        "SELECT id FROM source_data WHERE source_type=?1 AND source_id=?2",
        &[json!(source_type), json!(source_id)],
    ).ok().flatten().and_then(|r| r["id"].as_i64().map(|id| id.to_string()));

    let chunk_ids_json = if let Some(ref sid_str) = sd_id {
        if let Ok(sid) = sid_str.parse::<i64>() {
            let ids: Vec<String> = crate::database::all(
                "SELECT id FROM knowledge_chunks WHERE source_data_id=?1", &[json!(sid)],
            ).unwrap_or_default().iter().filter_map(|r| r["id"].as_i64().map(|id| id.to_string())).collect();
            if ids.is_empty() { "[]".to_string() } else { format!("[{}]", ids.join(",")) }
        } else { "[]".to_string() }
    } else { "[]".to_string() };

    let sql = "INSERT OR IGNORE INTO cleanup_queue(source_type,source_id,source_data_id,chunk_ids) VALUES(?1,?2,?3,?4)";
    crate::database::run(sql, &[
        json!(source_type),
        json!(source_id),
        sd_id.map(|s| json!(s)).unwrap_or(json!(serde_json::Value::Null)),
        json!(chunk_ids_json),
    ]).ok();
}

/// 处理 cleanup_queue 中的一条清理任务
pub fn process_cleanup_task() -> bool {
    let task = match crate::database::get(
        "SELECT id, source_type, source_id, source_data_id, chunk_ids FROM cleanup_queue WHERE status=0 ORDER BY id ASC LIMIT 1",
        &[],
    ) {
        Ok(Some(t)) => t,
        _ => return false,
    };
    let task_id = task["id"].as_i64().unwrap_or(0);
    let source_type = task["source_type"].as_str().unwrap_or("");
    let source_id = task["source_id"].as_str().unwrap_or("");
    println!("[清理管线] 处理: {}/{} (任务#{})", source_type, source_id, task_id);

    // ★ 使用快照 ID，不依赖 source_data 当前是否存在
    let sd_id = task["source_data_id"].as_i64();
    let chunk_ids: Vec<i64> = task["chunk_ids"].as_str()
        .and_then(|s| serde_json::from_str::<Vec<i64>>(s).ok())
        .unwrap_or_else(|| {
            // 无快照时回退到实时查询（兼容旧队列条目）
            sd_id.map(|sid| {
                crate::database::all("SELECT id FROM knowledge_chunks WHERE source_data_id=?1", &[json!(sid)])
                    .unwrap_or_default().iter().filter_map(|r| r["id"].as_i64()).collect()
            }).unwrap_or_default()
        });

    // ★ 仅清理 LanceDB + vector_cache（source_data / knowledge_chunks 由业务删除流程 FK cascade 负责）
    if !chunk_ids.is_empty() {
        let ph: Vec<String> = chunk_ids.iter().map(|_| "?".to_string()).collect();
        let params: Vec<serde_json::Value> = chunk_ids.iter().map(|id| json!(id)).collect();
        crate::database::run(&format!("DELETE FROM vector_cache WHERE chunk_id IN ({})", ph.join(",")), &params).ok();
        if let Ok(g) = crate::services::lancedb_client::get_lancedb() {
            if let Some(ref c) = *g {
                if let Err(e) = c.delete_chunks_by_rowids(&chunk_ids) {
                    eprintln!("[清理管线] LanceDB 清理失败: {}", e);
                }
            }
        }
    }

    // 标记完成
    crate::database::run("UPDATE cleanup_queue SET status=1 WHERE id=?", &[json!(task_id)]).ok();
    println!("[清理管线] 完成: {}/{} ({} 个块已清理)", source_type, source_id, chunk_ids.len());
    true
}

/// 兜底扫描：清理 source_data 中业务对象已不存在的孤儿记录
pub fn cleanup_orphan_source_data() {
    for (source_type, table, id_col) in &[
        ("NOTE", "notes", "note_id"),
        ("PRODUCT", "products", "id"),
        ("PROJECT", "projects", "id"),
        ("TODO", "todos", "id"),
        ("FILE", "file_assets", "id"),
    ] {
        let fts_sql = format!(
            "INSERT INTO global_search_fts(global_search_fts, rowid, source_type, source_id, title, content) \
             SELECT 'delete', rowid, source_type, source_id, title, content FROM source_data \
             WHERE source_type='{}' AND source_id NOT IN (SELECT {} FROM {})",
            source_type, id_col, table
        );
        crate::database::run(&fts_sql, &[]).ok();
        let chunks_sql = format!(
            "DELETE FROM knowledge_chunks WHERE source_data_id IN \
             (SELECT id FROM source_data WHERE source_type='{}' AND source_id NOT IN (SELECT {} FROM {}))",
            source_type, id_col, table
        );
        crate::database::run(&chunks_sql, &[]).ok();
        let sql = format!(
            "DELETE FROM source_data WHERE source_type='{}' AND source_id NOT IN (SELECT {} FROM {})",
            source_type, id_col, table
        );
        match crate::database::run(&sql, &[]) {
            Ok(r) if r.changes > 0 => println!("[清理管线] 孤儿 source_data 清理 ({}): {} 行", source_type, r.changes),
            Err(e) => eprintln!("[清理管线] 孤儿 source_data 清理失败 ({}): {}", source_type, e),
            _ => {}
        }
    }
}

/// 独立调度循环（在独立线程中运行，不阻塞主循环）
pub fn start() {
    if STARTED.swap(true, Ordering::SeqCst) {
        println!("[清理管线] 已在运行中");
        return;
    }
    println!("[清理管线] 独立清理管线启动");
    std::thread::spawn(|| {
        let rt = match tokio::runtime::Runtime::new() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("[清理管线] tokio 运行时失败: {}", e);
                return;
            }
        };
        rt.block_on(cleanup_scheduler_loop());
    });
}

async fn cleanup_scheduler_loop() {
    let mut tick: u64 = 0;
    loop {
        tick = tick.wrapping_add(1);

        let mut had_work = false;
        // 每次 tick 处理最多 5 个清理任务
        for _ in 0..5 {
            if !process_cleanup_task() {
                break;
            }
            had_work = true;
        }

        // 每 ~10 分钟执行兜底扫描
        if tick % 20 == 0 {
            cleanup_orphan_source_data();
            tick = 0;
        }

        // Event-driven: no work → wait for notification or 30s timeout
        if !had_work {
            tokio::select! {
                _ = WAKE.notified() => {},
                _ = tokio::time::sleep(Duration::from_secs(30)) => {},
            }
        }
    }
}
