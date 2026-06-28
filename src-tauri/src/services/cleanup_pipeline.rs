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

/// 唤醒清理管线（由 knowledge_lifecycle 调用，零延迟异步通知）
pub(crate) fn wake_cleanup() {
    WAKE.notify_one();
}

/// 将清理任务加入队列（由 knowledge_lifecycle 调用）
pub(crate) fn enqueue_cleanup(source_type: &str, source_id: &str) {
    // 直接使用 source_type + source_id 清理，不依赖 source_data.id
    let chunk_ids: Vec<String> = crate::database::all(
        "SELECT id FROM knowledge_chunks WHERE source_type=?1 AND source_id=?2",
        &[json!(source_type), json!(source_id)],
    ).unwrap_or_default().iter()
        .filter_map(|r| r["id"].as_i64().map(|id| id.to_string()))
        .collect();
    
    let chunk_ids_json = if chunk_ids.is_empty() { "[]".to_string() } else { format!("[{}]", chunk_ids.join(",")) };

    let sql = "INSERT OR IGNORE INTO cleanup_queue(source_type,source_id,chunk_ids) VALUES(?1,?2,?3)";
    crate::database::run(sql, &[
        json!(source_type),
        json!(source_id),
        json!(chunk_ids_json),
    ]).ok();
}

/// 处理 cleanup_queue 中的一条清理任务
pub fn process_cleanup_task() -> bool {
    let task = match crate::database::get(
        "SELECT id, source_type, source_id, chunk_ids FROM cleanup_queue WHERE status=0 ORDER BY id ASC LIMIT 1",
        &[],
    ) {
        Ok(Some(t)) => t,
        _ => return false,
    };
    let task_id = task["id"].as_i64().unwrap_or(0);
    let source_type = task["source_type"].as_str().unwrap_or("");
    let source_id = task["source_id"].as_str().unwrap_or("");
    println!("[清理管线] 处理: {}/{} (任务#{})", source_type, source_id, task_id);

    // 1. 通过 source_type + source_id 清理 knowledge_chunks
    let chunks_deleted = match crate::database::run(
        "DELETE FROM knowledge_chunks WHERE source_type=?1 AND source_id=?2",
        &[json!(source_type), json!(source_id)],
    ) {
        Ok(r) => { println!("[清理管线] 已清理 knowledge_chunks: {} 行", r.changes); r.changes as i64 }
        Err(e) => { eprintln!("[清理管线] knowledge_chunks 清理失败: {}", e); 0 }
    };

    // 2. 清理 vector_cache（通过 chunk_ids 快照）
    let chunk_ids: Vec<i64> = task["chunk_ids"].as_str()
        .and_then(|s| serde_json::from_str::<Vec<i64>>(s).ok())
        .unwrap_or_default();
    if !chunk_ids.is_empty() {
        let ph: Vec<String> = chunk_ids.iter().map(|_| "?".to_string()).collect();
        let params: Vec<serde_json::Value> = chunk_ids.iter().map(|id| json!(id)).collect();
        crate::database::run(&format!("DELETE FROM vector_cache WHERE chunk_id IN ({})", ph.join(",")), &params).ok();
    }

    // 3. 清理 LanceDB（通过 chunk_ids 快照）
    if !chunk_ids.is_empty() {
        if let Ok(g) = crate::services::lancedb_client::get_lancedb() {
            if let Some(ref c) = *g {
                if let Err(e) = c.delete_chunks_by_rowids(&chunk_ids) {
                    eprintln!("[清理管线] LanceDB 清理失败: {}", e);
                }
            }
        }
    }

    // 4. 清理 source_data
    match crate::database::run(
        "DELETE FROM source_data WHERE source_type=?1 AND source_id=?2",
        &[json!(source_type), json!(source_id)],
    ) {
        Ok(r) => println!("[清理管线] 已清理 source_data: {} 行", r.changes),
        Err(e) => eprintln!("[清理管线] source_data 清理失败: {}", e),
    }

    // 标记完成
    crate::database::run("UPDATE cleanup_queue SET status=1 WHERE id=?", &[json!(task_id)]).ok();
    println!("[清理管线] 完成: {}/{}", source_type, source_id);
    true
}

/// 兜底扫描：清理所有孤儿数据
/// 1. knowledge_chunks 中 source_data 不存在的
/// 2. vector_cache 中 knowledge_chunks 不存在的
/// 3. source_data 中业务对象已不存在的
pub fn cleanup_orphan_source_data() {
    // 调试：检查数据量
    let kc_count: i64 = crate::database::get("SELECT COUNT(*) as cnt FROM knowledge_chunks", &[])
        .ok().flatten().and_then(|r| r["cnt"].as_i64()).unwrap_or(0);
    let sd_count: i64 = crate::database::get("SELECT COUNT(*) as cnt FROM source_data", &[])
        .ok().flatten().and_then(|r| r["cnt"].as_i64()).unwrap_or(0);
    let vc_count: i64 = crate::database::get("SELECT COUNT(*) as cnt FROM vector_cache", &[])
        .ok().flatten().and_then(|r| r["cnt"].as_i64()).unwrap_or(0);
    println!("[清理管线] 孤儿扫描前: knowledge_chunks={}, source_data={}, vector_cache={}", kc_count, sd_count, vc_count);

    // 1. 清理 knowledge_chunks 孤儿（source_data 不存在）
    let orphan_count: i64 = crate::database::get(
        "SELECT COUNT(*) as cnt FROM knowledge_chunks kc WHERE NOT EXISTS (SELECT 1 FROM source_data sd WHERE sd.source_type=kc.source_type AND sd.source_id=kc.source_id)",
        &[],
    ).ok().flatten().and_then(|r| r["cnt"].as_i64()).unwrap_or(0);
    println!("[清理管线] 发现 {} 条孤儿 knowledge_chunks", orphan_count);
    
    if orphan_count > 0 {
        match crate::database::run(
            "DELETE FROM knowledge_chunks WHERE NOT EXISTS (SELECT 1 FROM source_data sd WHERE sd.source_type=knowledge_chunks.source_type AND sd.source_id=knowledge_chunks.source_id)",
            &[],
        ) {
            Ok(r) => println!("[清理管线] 已清理 knowledge_chunks: {} 行", r.changes),
            Err(e) => eprintln!("[清理管线] knowledge_chunks 清理失败: {}", e),
        }
    }

    // 2. 清理 vector_cache 孤儿（knowledge_chunks 不存在）
    match crate::database::run(
        "DELETE FROM vector_cache WHERE chunk_id NOT IN (SELECT id FROM knowledge_chunks)",
        &[],
    ) {
        Ok(r) if r.changes > 0 => println!("[清理管线] 孤儿 vector_cache 清理: {} 行", r.changes),
        _ => {}
    }

    // 3. 清理 source_data 孤儿（业务对象已不存在）
    for (source_type, table, id_col) in &[
        ("NOTE", "notes", "note_id"),
        ("PRODUCT", "products", "id"),
        ("PROJECT", "projects", "id"),
        ("TODO", "todos", "id"),
        ("FILE", "file_assets", "id"),
    ] {
        // 先清理 FTS5 索引
        let fts_sql = format!(
            "INSERT INTO global_search_fts(global_search_fts, rowid, source_type, source_id, title, content) \
             SELECT 'delete', rowid, source_type, source_id, title, content FROM source_data \
             WHERE source_type='{}' AND source_id NOT IN (SELECT {} FROM {})",
            source_type, id_col, table
        );
        crate::database::run(&fts_sql, &[]).ok();
        // 清理 source_data
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

        // Event-driven: no work → wait for notification or 30s timeout
        if !had_work {
            tokio::select! {
                _ = WAKE.notified() => {},
                _ = tokio::time::sleep(Duration::from_secs(30)) => {},
            }
        }
    }
}
