use crate::database;
use serde_json::{json, Value};

#[tauri::command]
pub async fn ai_sync_status() -> Result<Value, String> {
    let running = crate::services::knowledge_pipeline::is_running();
    let total: i64 = database::with_db(|c| {
        c.query_row("SELECT COUNT(*) FROM ai_sync_queue", [], |r| r.get(0))
    }).unwrap_or(0);
    let pending: i64 = database::with_db(|c| {
        c.query_row("SELECT COUNT(*) FROM ai_sync_queue WHERE is_embedded=0", [], |r| r.get(0))
    }).unwrap_or(0);
    let done: i64 = database::with_db(|c| {
        c.query_row("SELECT COUNT(*) FROM ai_sync_queue WHERE is_embedded=1", [], |r| r.get(0))
    }).unwrap_or(0);
    let failed: i64 = database::with_db(|c| {
        c.query_row("SELECT COUNT(*) FROM ai_sync_queue WHERE is_embedded=-1", [], |r| r.get(0))
    }).unwrap_or(0);
    Ok(json!({
        "running": running,
        "total": total,
        "pending": pending,
        "done": done,
        "failed": failed
    }))
}

#[tauri::command]
pub async fn ai_sync_trigger() -> Result<Value, String> {
    crate::services::knowledge_pipeline::start();
    Ok(json!({"success": true}))
}

#[tauri::command]
pub async fn ai_sync_diagnose() -> Result<Value, String> {
    let mut tables = Vec::new();
    for (st, tbl, _id_col) in &[
        ("projects","projects","id"),
        ("products","products","id"),
        ("files","file_assets","id"), ("notes","notes","note_id"),
    ] {
        let biz: i64 = database::with_db(|c| c.query_row(&format!("SELECT COUNT(*) FROM {tbl}"),[],|r|r.get(0))).unwrap_or(0);
        let q: i64 = database::with_db(|c| c.query_row("SELECT COUNT(*) FROM ai_sync_queue WHERE source_table=?1",[st],|r|r.get(0))).unwrap_or(0);
        let p: i64 = database::with_db(|c| c.query_row("SELECT COUNT(*) FROM ai_sync_queue WHERE source_table=?1 AND is_embedded=0",[st],|r|r.get(0))).unwrap_or(0);
        let e: i64 = database::with_db(|c| c.query_row("SELECT COUNT(*) FROM ai_sync_queue WHERE source_table=?1 AND is_embedded=1",[st],|r|r.get(0))).unwrap_or(0);
        let f: i64 = database::with_db(|c| c.query_row("SELECT COUNT(*) FROM ai_sync_queue WHERE source_table=?1 AND is_embedded=-1",[st],|r|r.get(0))).unwrap_or(0);
        tables.push(json!({"table":st,"bizRows":biz,"queued":q,"pending":p,"embedded":e,"failed":f}));
    }
    Ok(json!({"success":true,"tables":tables}))
}

#[tauri::command]
pub async fn ai_sync_reset() -> Result<Value, String> {
    let r = database::run("UPDATE ai_sync_queue SET is_embedded=0,retry_count=0,last_error='' WHERE is_embedded IN (1,-1)",&[]).map_err(|e|e.to_string())?;
    // ★ 清除不应出现在队列中的类型（旧 trigger 残留、手动操作残留等）
    let _ = database::run("DELETE FROM ai_sync_queue WHERE source_table IN ('products','todos','projects')",&[]);
    crate::services::knowledge_pipeline::start();
    Ok(json!({"success":true,"reset":r.changes}))
}

#[tauri::command]
pub async fn ai_sync_retry_failed() -> Result<Value, String> {
    let r = database::run("UPDATE ai_sync_queue SET is_embedded=0,retry_count=0,last_error='' WHERE is_embedded=-1",&[]).map_err(|e|e.to_string())?;
    crate::services::knowledge_pipeline::start();
    Ok(json!({"success":true,"retried":r.changes}))
}

#[tauri::command]
pub async fn cleanup_retry_failed() -> Result<Value, String> {
    let r = database::run("UPDATE cleanup_queue SET status=0,retry_count=0,last_error='' WHERE status=-1",&[]).map_err(|e|e.to_string())?;
    crate::services::cleanup_pipeline::start();
    Ok(json!({"success":true,"retried":r.changes}))
}

#[tauri::command]
pub async fn cleanup_orphan_data() -> Result<Value, String> {
    crate::services::cleanup_pipeline::cleanup_orphan_source_data();
    Ok(json!({"success":true,"message":"孤儿数据清理已完成"}))
}

#[tauri::command]
pub async fn pipeline_full_resync() -> Result<Value, String> {
    // 清空旧的 ai_sync_queue
    crate::database::run("DELETE FROM ai_sync_queue", &[]).ok();
    // 重置同步时间戳，让管线重新处理所有对象
    crate::services::knowledge_pipeline::reset_sync_time();
    Ok(json!({"success":true,"message":"已清空 ai_sync_queue 并重置同步时间戳，知识管线将在下一轮重新物化所有对象"}))
}

#[tauri::command]
pub async fn ai_sync_force_resync() -> Result<Value, String> {
    let r = database::run("UPDATE ai_sync_queue SET is_embedded=0,retry_count=0,last_error='' WHERE is_embedded IN (1,-1)",&[]).map_err(|e|e.to_string())?;
    crate::services::knowledge_pipeline::start();
    Ok(json!({"success":true,"reset":r.changes}))
}

#[tauri::command]
pub async fn ai_sync_diagnose_failed_files() -> Result<Value, String> {
    let rows = database::all(
        "SELECT aq.id,aq.source_id,aq.retry_count,aq.last_error,fa.global_name,fa.file_ext
         FROM ai_sync_queue aq LEFT JOIN file_assets fa ON fa.id=aq.source_id
         WHERE aq.source_table='files' AND aq.is_embedded=-1 ORDER BY aq.id DESC LIMIT 50",
        &[],
    ).map_err(|e|e.to_string())?;
    Ok(json!({"success":true,"failedFiles":rows}))
}

#[tauri::command]
pub async fn knowledge_pipeline_status() -> Result<Value, String> {
    let running = crate::services::knowledge_pipeline::is_running();
    Ok(json!({"running":running}))
}

#[tauri::command]
pub async fn pipeline_debug_status() -> Result<Value, String> {
    // ★ 清理孤儿队列条目（source_data 已删除但队列未清理）
    crate::database::run(
        "DELETE FROM ai_sync_queue WHERE NOT EXISTS (
           SELECT 1 FROM source_data
           WHERE source_data.source_id = ai_sync_queue.source_id
           AND source_data.source_type = CASE ai_sync_queue.source_table
             WHEN 'files' THEN 'FILE'
             WHEN 'notes' THEN 'NOTE'
             WHEN 'products' THEN 'PRODUCT'
             WHEN 'projects' THEN 'PROJECT'
             WHEN 'todos' THEN 'TODO'
           END
         )",
        &[],
    ).ok();
    // ★ 同时清理孤儿打标队列
    crate::database::run(
        "DELETE FROM ai_tag_queue WHERE NOT EXISTS (
           SELECT 1 FROM source_data
           WHERE source_data.source_id = ai_tag_queue.source_id
           AND source_data.source_type = CASE ai_tag_queue.source_type
             WHEN 'files' THEN 'FILE'
             WHEN 'notes' THEN 'NOTE'
             WHEN 'products' THEN 'PRODUCT'
             WHEN 'projects' THEN 'PROJECT'
           END
         )",
        &[],
    ).ok();
    let sync_total: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM ai_sync_queue", [], |r| r.get(0))).unwrap_or(0);
    let sync_pending: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM ai_sync_queue WHERE is_embedded=0", [], |r| r.get(0))).unwrap_or(0);
    let sync_done: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM ai_sync_queue WHERE is_embedded=1", [], |r| r.get(0))).unwrap_or(0);
    let sync_failed: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM ai_sync_queue WHERE is_embedded=-1", [], |r| r.get(0))).unwrap_or(0);
    let tag_pending: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM ai_tag_queue WHERE status=0", [], |r| r.get(0))).unwrap_or(0);
    let tag_done: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM ai_tag_queue WHERE status=1", [], |r| r.get(0))).unwrap_or(0);
    let tag_failed: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM ai_tag_queue WHERE status=-1", [], |r| r.get(0))).unwrap_or(0);
    let chunks: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM knowledge_chunks", [], |r| r.get(0))).unwrap_or(0);
    let parents: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM knowledge_chunks WHERE chunk_type='parent'", [], |r| r.get(0))).unwrap_or(0);
    let children: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM knowledge_chunks WHERE chunk_type='child'", [], |r| r.get(0))).unwrap_or(0);
    let vectorized: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM vector_cache", [], |r| r.get(0))).unwrap_or(0);
    // Per-source-type counts from source_data and file_assets
    let source_files: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM source_data WHERE source_type='FILE'", [], |r| r.get(0))).unwrap_or(0);
    let source_notes: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM source_data WHERE source_type='NOTE'", [], |r| r.get(0))).unwrap_or(0);
    let source_products: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM source_data WHERE source_type='PRODUCT'", [], |r| r.get(0))).unwrap_or(0);
    let source_projects: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM source_data WHERE source_type='PROJECT'", [], |r| r.get(0))).unwrap_or(0);
    let source_todos: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM source_data WHERE source_type='TODO'", [], |r| r.get(0))).unwrap_or(0);
    let source_total: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM source_data", [], |r| r.get(0))).unwrap_or(0);
    // Queue counts per source type for debugging
    let q_files: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM ai_sync_queue WHERE source_table='files'", [], |r| r.get(0))).unwrap_or(0);
    let q_notes: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM ai_sync_queue WHERE source_table='notes'", [], |r| r.get(0))).unwrap_or(0);
    let q_products: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM ai_sync_queue WHERE source_table='products'", [], |r| r.get(0))).unwrap_or(0);
    let q_projects: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM ai_sync_queue WHERE source_table='projects'", [], |r| r.get(0))).unwrap_or(0);
    // Compare: source_data entries that have NO sync queue entry
    let q_total: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM ai_sync_queue", [], |r| r.get(0))).unwrap_or(0);
    let sd_orphans: i64 = source_files + source_notes + source_products + source_projects - q_total;
    let files_pending: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM file_assets WHERE parse_status='PENDING'", [], |r| r.get(0))).unwrap_or(0);
    let files_failed: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM file_assets WHERE parse_status='FAILED'", [], |r| r.get(0))).unwrap_or(0);
    let files_done: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM file_assets WHERE parse_status='COMPLETED'", [], |r| r.get(0))).unwrap_or(0);
    Ok(json!({
        "sync": {"total":sync_total,"pending":sync_pending,"done":sync_done,"failed":sync_failed},
        "tag": {"pending":tag_pending,"done":tag_done,"failed":tag_failed},
        "knowledge": {"chunks":chunks,"parents":parents,"children":children,"vectorized":vectorized},
        "source": {"files":source_files,"notes":source_notes,"products":source_products,"projects":source_projects,"todos":source_todos,"total":source_total},
        "queue": {"files":q_files,"notes":q_notes,"products":q_products,"projects":q_projects,"total":q_total},
        "diagnose": {"sdOrphans":sd_orphans},
        "fileParse": {"pending":files_pending,"failed":files_failed,"done":files_done},
    }))
}

#[tauri::command]
pub async fn pipeline_clear_knowledge() -> Result<Value, String> {
    crate::database::run("DELETE FROM vector_cache", &[]).ok();
    crate::database::run("DROP TRIGGER IF EXISTS kk_chunks_fts_insert", &[]).ok();
    crate::database::run("DROP TRIGGER IF EXISTS kk_chunks_fts_delete", &[]).ok();
    crate::database::run("DROP TRIGGER IF EXISTS kk_chunks_fts_update", &[]).ok();
    crate::database::run("DELETE FROM knowledge_chunks", &[]).ok();
    // ★ 删除 LanceDB 数据目录（下次向量化自动重建）
    let lancedb_dir = dirs::data_dir().unwrap_or_default().join("arkmind").join("lancedb");
    let _ = std::fs::remove_dir_all(&lancedb_dir);
    crate::services::lancedb_client::invalidate_lancedb_cache();
    crate::database::run("INSERT INTO kk_chunks_fts(kk_chunks_fts) VALUES('rebuild')", &[]).ok();
    crate::database::with_db(|conn| {
        conn.execute_batch(
            "CREATE TRIGGER IF NOT EXISTS kk_chunks_fts_insert AFTER INSERT ON knowledge_chunks
             WHEN new.chunk_type = 'child'
             BEGIN
                 INSERT INTO kk_chunks_fts(rowid, doc_title, content) VALUES (new.rowid, new.doc_title, new.content);
             END;
             CREATE TRIGGER IF NOT EXISTS kk_chunks_fts_delete AFTER DELETE ON knowledge_chunks
             WHEN old.chunk_type = 'child'
             BEGIN
                 INSERT INTO kk_chunks_fts(kk_chunks_fts, rowid) VALUES ('delete', old.rowid);
             END;
             CREATE TRIGGER IF NOT EXISTS kk_chunks_fts_update AFTER UPDATE ON knowledge_chunks
             WHEN old.chunk_type = 'child' OR new.chunk_type = 'child'
             BEGIN
                 DELETE FROM kk_chunks_fts WHERE rowid = old.rowid;
                 INSERT INTO kk_chunks_fts(rowid, doc_title, content)
                   SELECT new.rowid, new.doc_title, new.content
                   WHERE new.chunk_type = 'child';
             END;"
        )
    }).ok();
    // ★ 重置后重新播种同步队列并启动管线
    let _ = crate::database::run("DELETE FROM ai_sync_queue", &[]);
    for (st, st_type) in &[("files", "FILE"), ("notes", "NOTE"), ("products", "PRODUCT"), ("projects", "PROJECT"), ("todos", "TODO")] {
        let _ = crate::database::run(
            &format!("INSERT OR IGNORE INTO ai_sync_queue(source_table,source_id) SELECT '{}',source_id FROM source_data WHERE source_type='{}'", st, st_type),
            &[],
        );
    }
    crate::services::knowledge_pipeline::start();
    println!("[调试] 已重置知识管线，已触发重新物化");
    Ok(json!({"success":true,"message":"已重置知识管线，正在重新物化"}))
}

#[tauri::command]
pub async fn pipeline_clear_vectors() -> Result<Value, String> {
    crate::database::run("DELETE FROM vector_cache", &[]).ok();
    // ★ Also drop and recreate LanceDB table to clear stale rowids
    if let Ok(g) = crate::services::lancedb_client::get_lancedb() {
        if let Some(ref c) = *g {
            c.clear_all_vectors().ok();
        }
    }
    println!("[调试] 已清除向量缓存标记和LanceDB表");
    Ok(json!({"success":true,"message":"已清除向量缓存和LanceDB表，下次向量化时将重新生成"}))
}

#[tauri::command]
pub async fn pipeline_clear_all() -> Result<Value, String> {
    crate::database::run("DELETE FROM vector_cache", &[]).ok();
    // 临时禁用触发器，避免 DELETE 时触发器写 FTS 失败导致事务回滚
    crate::database::run("DROP TRIGGER IF EXISTS kk_chunks_fts_insert", &[]).ok();
    crate::database::run("DROP TRIGGER IF EXISTS kk_chunks_fts_delete", &[]).ok();
    crate::database::run("DROP TRIGGER IF EXISTS kk_chunks_fts_update", &[]).ok();
    // 直接删 knowledge_chunks（无触发器+Fk干扰）
    let result = crate::database::run("DELETE FROM knowledge_chunks", &[]);
    eprintln!("[调试] 删除 knowledge_chunks 结果: {:?}", result);
    eprintln!("[调试] 删除 knowledge_chunks 结果: {:?}", result);
    if result.is_err() {
        crate::database::run("DELETE FROM knowledge_chunks", &[]).ok();
    }
    let after: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM knowledge_chunks", [], |r| r.get(0))).unwrap_or(-1);
    eprintln!("[调试] 删除后 knowledge_chunks 剩余: {}", after);
    // 重建空 FTS 索引
    crate::database::run("INSERT INTO kk_chunks_fts(kk_chunks_fts) VALUES('rebuild')", &[]).ok();
    // ★ 清空 LanceDB 向量（删除物理目录，比 drop_table 更可靠）
    let lancedb_dir2 = dirs::data_dir().unwrap_or_default().join("arkmind").join("lancedb");
    let _ = std::fs::remove_dir_all(&lancedb_dir2);
    crate::services::lancedb_client::invalidate_lancedb_cache();
    // 重新创建触发器
    crate::database::with_db(|conn| {
        conn.execute_batch(
            "CREATE TRIGGER IF NOT EXISTS kk_chunks_fts_insert AFTER INSERT ON knowledge_chunks
             WHEN new.chunk_type = 'child'
             BEGIN
                 INSERT INTO kk_chunks_fts(rowid, doc_title, content) VALUES (new.rowid, new.doc_title, new.content);
             END;
             CREATE TRIGGER IF NOT EXISTS kk_chunks_fts_delete AFTER DELETE ON knowledge_chunks
             WHEN old.chunk_type = 'child'
             BEGIN
                 INSERT INTO kk_chunks_fts(kk_chunks_fts, rowid) VALUES ('delete', old.rowid);
             END;
             CREATE TRIGGER IF NOT EXISTS kk_chunks_fts_update AFTER UPDATE ON knowledge_chunks
             WHEN old.chunk_type = 'child' OR new.chunk_type = 'child'
             BEGIN
                 DELETE FROM kk_chunks_fts WHERE rowid = old.rowid;
                 INSERT INTO kk_chunks_fts(rowid, doc_title, content)
                   SELECT new.rowid, new.doc_title, new.content
                   WHERE new.chunk_type = 'child';
             END;"
        )
    }).ok();
    crate::database::run("DELETE FROM ai_sync_queue", &[]).ok();
    crate::database::run("DELETE FROM ai_tag_queue", &[]).ok();
    crate::database::run("DELETE FROM source_data", &[]).ok();
    // Reset file parse status
    crate::database::run("UPDATE file_assets SET parse_status='PENDING' WHERE parse_status IN ('COMPLETED','FAILED')", &[]).ok();
    println!("[调试] 已清空所有管线数据（含 source_data），文件解析状态已重置");
    Ok(json!({"success":true,"message":"已清空所有管线数据（含 source_data），请在清空完成后手动点击「触发同步」重新物化"}))
}

#[tauri::command]
pub async fn pipeline_clear_tag_pipeline() -> Result<Value, String> {
    crate::database::run("DELETE FROM ai_tag_queue", &[]).ok();
    // ★ 重新播种打标队列
    crate::services::ai_tag::seed_pending_tags();
    println!("[调试] 已重置标签管线：清空后重新播种");
    Ok(json!({"success":true,"message":"已重置标签管线，正在重新打标"}))
}

#[tauri::command]
pub async fn pipeline_resync() -> Result<Value, String> {
    // Step 1: 从业务表重新播种 source_data
    crate::database::run("DELETE FROM source_data", &[]).ok();
    crate::database::with_db(|conn| {
        conn.execute_batch(
            "INSERT OR IGNORE INTO source_data(source_type,source_id,title,content) SELECT 'NOTE',n.note_id,n.title,
               'title: '||n.title||
               CASE WHEN COALESCE(n.content,'')!='' THEN CHAR(10)||'content: '||n.content ELSE '' END
             FROM notes n WHERE n.deleted_at IS NULL;
             INSERT OR IGNORE INTO source_data(source_type,source_id,title,content) SELECT 'PRODUCT',p.id,p.name,
               'name: '||p.name||
               CASE WHEN COALESCE(p.description,'')!='' THEN CHAR(10)||'description: '||p.description ELSE '' END
             FROM products p;
             INSERT OR IGNORE INTO source_data(source_type,source_id,title,content) SELECT 'PROJECT',p.id,p.name,
               'name: '||p.name||
               CASE WHEN COALESCE(p.status,'')!='' THEN CHAR(10)||'status: '||p.status ELSE '' END||
               CASE WHEN COALESCE(p.owner,'')!='' THEN CHAR(10)||'owner: '||p.owner ELSE '' END||
               CASE WHEN COALESCE(p.customer,'')!='' THEN CHAR(10)||'customer: '||p.customer ELSE '' END
             FROM projects p;
             INSERT OR IGNORE INTO source_data(source_type,source_id,title,content) SELECT 'TODO',t.id,t.title,
               'title: '||t.title||
               CASE WHEN COALESCE(t.content,'')!='' THEN CHAR(10)||'content: '||t.content ELSE '' END
             FROM todos t;"
        )
    }).ok();
    // FILE 类型: 重建 source_data 条目并标记文件重新解析
    let _ = crate::database::run("INSERT OR IGNORE INTO source_data(source_type,source_id,title,content) SELECT 'FILE',id,id,'' FROM file_assets", &[]);
    crate::database::run("UPDATE file_assets SET parse_status='PENDING' WHERE parse_status IN ('COMPLETED','FAILED')", &[]).ok();
    // ★ 触发 sd_*_update：让触发器用完整格式重建 source_data（含渠道/厂商/方案/动态等）
    let _ = crate::database::run("UPDATE projects SET updated_at=updated_at", &[]);
    let _ = crate::database::run("UPDATE products SET updated_at=updated_at", &[]);
    let _ = crate::database::run("UPDATE notes SET updated_at=updated_at WHERE deleted_at IS NULL", &[]);
    let _ = crate::database::run("UPDATE todos SET updated_at=updated_at", &[]);
    let source_count: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM source_data", [], |r| r.get(0))).unwrap_or(0);
    println!("[调试] source_data 播种完成: {} 行", source_count);

    // Step 2: 清空并补种 ai_sync_queue
    crate::database::run("DELETE FROM ai_sync_queue", &[]).ok();
    crate::database::run(
        "INSERT OR IGNORE INTO ai_sync_queue(source_table,source_id) SELECT 'notes',source_id FROM source_data WHERE source_type='NOTE'", &[]
    ).ok();
    crate::database::run(
        "INSERT OR IGNORE INTO ai_sync_queue(source_table,source_id) SELECT 'files',source_id FROM source_data WHERE source_type='FILE'", &[]
    ).ok();
    crate::database::run(
        "INSERT OR IGNORE INTO ai_sync_queue(source_table,source_id) SELECT 'products',source_id FROM source_data WHERE source_type='PRODUCT'", &[]
    ).ok();
    crate::database::run(
        "INSERT OR IGNORE INTO ai_sync_queue(source_table,source_id) SELECT 'projects',source_id FROM source_data WHERE source_type='PROJECT'", &[]
    ).ok();
    crate::database::run(
        "INSERT OR IGNORE INTO ai_sync_queue(source_table,source_id) SELECT 'todos',source_id FROM source_data WHERE source_type='TODO'", &[]
    ).ok();

    // Step 3: 启动管线
    crate::services::knowledge_pipeline::start();
    Ok(json!({"success":true,"message":format!("已重新同步 {} 条数据并启动管线", source_count)}))
}

#[tauri::command]
pub async fn pipeline_browse_source(source_type: Option<String>, limit: Option<i64>) -> Result<Value, String> {
    let limit = limit.unwrap_or(50);
    let rows = if let Some(st) = source_type {
        database::all(
            "SELECT id, source_type, source_id, title, substr(content,1,500) as content_preview, length(content) as content_len, created_at, updated_at FROM source_data WHERE source_type=?1 ORDER BY updated_at DESC LIMIT ?2",
            &[json!(st), json!(limit)],
        ).map_err(|e| e.to_string())?
    } else {
        database::all(
            "SELECT id, source_type, source_id, title, substr(content,1,500) as content_preview, length(content) as content_len, created_at, updated_at FROM source_data ORDER BY updated_at DESC LIMIT ?1",
            &[json!(limit)],
        ).map_err(|e| e.to_string())?
    };
    Ok(json!({"success":true,"rows":rows}))
}

#[tauri::command]
pub async fn pipeline_get_source_content(id: String) -> Result<Value, String> {
    let row = database::get(
        "SELECT id, source_type, source_id, title, content, created_at, updated_at FROM source_data WHERE id=?",
        &[json!(id)],
    ).map_err(|e| e.to_string())?;
    Ok(json!({"success":true,"row":row}))
}

#[tauri::command]
pub async fn pipeline_retag_all() -> Result<Value, String> {
    // 重置所有已完成/失败的打标任务为待处理
    crate::database::run("UPDATE ai_tag_queue SET status=0,retry_count=0,last_error='' WHERE status IN (1,-1)", &[]).ok();
    // 补种 source_data 中有但队列中没有的记录
    let sql = "INSERT OR IGNORE INTO ai_tag_queue(source_type, source_id, status)
               SELECT 'files', source_id, 0 FROM source_data WHERE source_type='FILE'
               UNION
               SELECT 'notes', source_id, 0 FROM source_data WHERE source_type='NOTE'
               UNION
               SELECT 'products', source_id, 0 FROM source_data WHERE source_type='PRODUCT'
               UNION
               SELECT 'projects', source_id, 0 FROM source_data WHERE source_type='PROJECT'";
    let added = crate::database::run(sql, &[]).ok().map(|r| r.changes).unwrap_or(0);
    let msg = format!("已重置所有打标任务为待处理 (+{} 新增)", added);
    println!("[调试] {}", msg);
    Ok(json!({"success":true,"message":msg}))
}
