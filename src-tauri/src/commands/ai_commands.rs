use crate::database;
use serde_json::{json, Value};

#[tauri::command]
pub async fn ai_sync_status() -> Result<Value, String> {
    let running = crate::services::knowledge_pipeline::is_running();
    let chunk_count: i64 = database::with_db(|c| {
        c.query_row("SELECT COUNT(*) FROM knowledge_chunks WHERE chunk_type='child'", [], |r| r.get(0))
    }).unwrap_or(0);
    let source_count: i64 = database::with_db(|c| {
        c.query_row("SELECT COUNT(*) FROM source_data", [], |r| r.get(0))
    }).unwrap_or(0);
    Ok(json!({"running": running, "chunks": chunk_count, "sources": source_count}))
}

#[tauri::command]
pub async fn ai_sync_trigger() -> Result<Value, String> {
    crate::services::knowledge_pipeline::start();
    Ok(json!({"success": true}))
}

#[tauri::command]
pub async fn ai_sync_diagnose() -> Result<Value, String> {
    let mut tables = Vec::new();
    for (st, tbl) in &[
        ("projects","projects"),
        ("products","products"),
        ("files","file_assets"),
        ("notes","notes"),
    ] {
        let biz: i64 = database::with_db(|c| c.query_row(&format!("SELECT COUNT(*) FROM {tbl}"),[],|r|r.get(0))).unwrap_or(0);
        tables.push(json!({"table":st,"bizRows":biz}));
    }
    Ok(json!({"success":true,"tables":tables}))
}

#[tauri::command]
pub async fn cleanup_retry_failed() -> Result<Value, String> {
    // 重置 knowledge_push_queue 中的失败任务
    let r = database::run("UPDATE knowledge_push_queue SET status='pending', retry_count=0, last_error='' WHERE status='failed'", &[]).map_err(|e|e.to_string())?;
    Ok(json!({"success":true,"retried":r.changes}))
}

#[tauri::command]
pub async fn cleanup_orphan_data() -> Result<Value, String> {
    crate::services::file_ingestion::cleanup_orphan_source_data();
    Ok(json!({"success":true,"message":"孤儿数据清理已完成"}))
}

#[tauri::command]
pub async fn knowledge_pipeline_status() -> Result<Value, String> {
    let running = crate::services::knowledge_pipeline::is_running();
    Ok(json!({"running":running}))
}

#[tauri::command]
pub async fn pipeline_debug_status() -> Result<Value, String> {
    let chunks: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM knowledge_chunks", [], |r| r.get(0))).unwrap_or(0);
    let parents: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM knowledge_chunks WHERE chunk_type='parent'", [], |r| r.get(0))).unwrap_or(0);
    let children: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM knowledge_chunks WHERE chunk_type='child'", [], |r| r.get(0))).unwrap_or(0);
    let vectorized: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM vector_cache", [], |r| r.get(0))).unwrap_or(0);
    // 向量化有效指标：排除空内容块（按设计永不向量化）和 vector_cache 孤儿
    let children_nonempty: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM knowledge_chunks WHERE chunk_type='child' AND content != ''", [], |r| r.get(0))).unwrap_or(0);
    let vectorized_valid: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM vector_cache WHERE chunk_id IN (SELECT id FROM knowledge_chunks WHERE chunk_type='child')", [], |r| r.get(0))).unwrap_or(0);
    // 待向量化积压：有内容但未进 vector_cache 的子块（持续不降 = 队列卡死信号）
    let embed_pending: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM knowledge_chunks WHERE chunk_type='child' AND content != '' AND id NOT IN (SELECT chunk_id FROM vector_cache)", [], |r| r.get(0))).unwrap_or(0);
    let last_embed_error = crate::services::knowledge_pipeline::get_last_embed_error();
    let source_files: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM source_data WHERE source_type='FILE'", [], |r| r.get(0))).unwrap_or(0);
    let source_notes: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM source_data WHERE source_type='NOTE'", [], |r| r.get(0))).unwrap_or(0);
    let source_products: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM source_data WHERE source_type='PRODUCT'", [], |r| r.get(0))).unwrap_or(0);
    let source_projects: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM source_data WHERE source_type='PROJECT'", [], |r| r.get(0))).unwrap_or(0);
    let source_todos: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM source_data WHERE source_type='TODO'", [], |r| r.get(0))).unwrap_or(0);
    let source_total: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM source_data", [], |r| r.get(0))).unwrap_or(0);
    // 从 knowledge_push_queue 查询文件解析状态。
    // ★ 排除 operation='delete'：删除任务（如孤儿资产回收入队）与解析共用 FILE 队列，
    //   计入"文件解析"会让删除显示成"正在解析"、delete 失败混进"解析失败文件"、done 双扣
    let files_pending: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM knowledge_push_queue WHERE source_type='FILE' AND status='pending' AND operation!='delete'", [], |r| r.get(0))).unwrap_or(0);
    let files_processing: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM knowledge_push_queue WHERE source_type='FILE' AND status='processing' AND operation!='delete'", [], |r| r.get(0))).unwrap_or(0);
    let files_failed: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM knowledge_push_queue WHERE source_type='FILE' AND status='failed' AND operation!='delete'", [], |r| r.get(0))).unwrap_or(0);
    let files_unsupported: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM knowledge_push_queue WHERE source_type='FILE' AND status='unsupported' AND operation!='delete'", [], |r| r.get(0))).unwrap_or(0);
    // 已完成文件 = file_assets 总数 − 队列中 FILE 解析任务数（成功任务会从队列删除；unsupported 为终态视为完成）
    let files_total: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM file_assets", [], |r| r.get(0))).unwrap_or(0);
    let files_done = files_total - files_pending - files_processing - files_failed - files_unsupported;
    // 当前正在解析的文件（页级进度）
    let current_file: Option<Value> = crate::database::get(
        "SELECT f.original_name AS name, q.progress_page AS page, q.progress_total AS total FROM knowledge_push_queue q LEFT JOIN file_assets f ON f.id=q.source_id WHERE q.source_type='FILE' AND q.status='processing' AND q.operation!='delete' ORDER BY q.id DESC LIMIT 1",
        &[],
    ).ok().flatten();
    // 解析失败/不支持的文件明细（供设置页展示，便于用户确认是哪些文件）
    let failed_files: Vec<Value> = crate::database::all(
        "SELECT COALESCE(NULLIF(f.original_name, ''), q.source_id) AS name, q.last_error AS error, q.retry_count AS retries, q.updated_at AS at \
         FROM knowledge_push_queue q LEFT JOIN file_assets f ON f.id=q.source_id \
         WHERE q.source_type='FILE' AND q.status='failed' AND q.operation!='delete' ORDER BY q.updated_at DESC LIMIT 500",
        &[],
    ).unwrap_or_default();
    let unsupported_files: Vec<Value> = crate::database::all(
        "SELECT COALESCE(NULLIF(f.original_name, ''), q.source_id) AS name, q.last_error AS error, q.updated_at AS at \
         FROM knowledge_push_queue q LEFT JOIN file_assets f ON f.id=q.source_id \
         WHERE q.source_type='FILE' AND q.status='unsupported' AND q.operation!='delete' ORDER BY q.updated_at DESC LIMIT 500",
        &[],
    ).unwrap_or_default();
    Ok(json!({
        "knowledge": {"chunks":chunks,"parents":parents,"children":children,"vectorized":vectorized,"childrenNonempty":children_nonempty,"vectorizedValid":vectorized_valid,"embedPending":embed_pending,"lastEmbedError":last_embed_error},
        "source": {"files":source_files,"notes":source_notes,"products":source_products,"projects":source_projects,"todos":source_todos,"total":source_total},
        "fileParse": {"done":files_done.max(0),"pending":files_pending,"processing":files_processing,"failed":files_failed,"unsupported":files_unsupported,"total":files_total,"current":current_file,"failedFiles":failed_files,"unsupportedFiles":unsupported_files},
        "vectorVacuum": crate::services::vector_vacuum::vacuum_status(),
    }))
}

/// 立即重试所有"解析失败"的 FILE 任务（免冷启动）；unsupported 为终态不重试。
/// 口径与设置页"重试失败文件"按钮展示数一致：仅 FILE、仅解析类（排除 delete 清理任务，
/// 且不再误伤 NOTE/PROJECT 等非文件域的 failed 任务）
#[tauri::command]
pub async fn pipeline_retry_failed() -> Result<Value, String> {
    let count = crate::database::run(
        "UPDATE knowledge_push_queue SET status='pending', retry_count=0, last_error='', debounce_until=NULL \
         WHERE status='failed' AND source_type='FILE' AND operation!='delete'",
        &[],
    ).map(|r| r.changes).map_err(|e| e.to_string())?;
    Ok(json!({"success": true, "count": count}))
}

#[tauri::command]
pub async fn pipeline_clear_knowledge() -> Result<Value, String> {
    crate::services::knowledge_lifecycle::reset_pipeline();
    Ok(json!({"success":true,"message":"已重置知识管线，正在重新物化"}))
}

#[tauri::command]
pub async fn pipeline_clear_vectors() -> Result<Value, String> {
    crate::database::run("DELETE FROM vector_cache", &[]).ok();
    if let Ok(g) = crate::services::lancedb_client::get_lancedb() {
        if let Some(ref c) = *g { c.clear_all_vectors().ok(); }
    }
    Ok(json!({"success":true,"message":"已清除向量缓存和LanceDB表"}))
}

#[tauri::command]
pub async fn pipeline_revectorize() -> Result<Value, String> {
    // 1. 清除向量缓存和 LanceDB
    crate::database::run("DELETE FROM vector_cache", &[]).ok();
    if let Ok(g) = crate::services::lancedb_client::get_lancedb() {
        if let Some(ref c) = *g { c.clear_all_vectors().ok(); }
    }
    // 2. 获取需要向量化的 chunk 数量
    let chunk_count: i64 = crate::database::with_db(|c| {
        c.query_row(
            "SELECT COUNT(*) FROM knowledge_chunks WHERE chunk_type='child' AND content IS NOT NULL AND content != ''",
            [], |r| r.get(0)
        )
    }).unwrap_or(0);
    // 3. 启动知识管线进行重新向量化
    crate::services::knowledge_pipeline::start();
    Ok(json!({"success":true,"message":format!("已清除向量数据并启动重新向量化（{} 个碎片）", chunk_count),"chunk_count":chunk_count}))
}

#[tauri::command]
pub async fn pipeline_clear_all() -> Result<Value, String> {
    crate::services::knowledge_lifecycle::reset_pipeline();
    crate::database::run("DELETE FROM source_data", &[]).ok();
    // 重新入队所有已有文件（成功任务已从队列删除，需重新入队解析写入 source_data）
    let _ = crate::database::run("DELETE FROM knowledge_push_queue WHERE source_type='FILE'", &[]);
    let _ = crate::database::run(
        "INSERT INTO knowledge_push_queue(source_type, source_id, operation, status) SELECT 'FILE', id, 'file_upload', 'pending' FROM file_assets",
        &[],
    );
    Ok(json!({"success":true,"message":"已清空所有管线数据"}))
}

#[tauri::command]
pub async fn pipeline_resync() -> Result<Value, String> {
    // M6: 先清空向量缓存和 LanceDB，防止残留陈旧向量
    crate::services::knowledge_lifecycle::reset_pipeline();
    crate::database::run("DELETE FROM source_data", &[]).ok();
    crate::database::with_db(|conn| {
        conn.execute_batch(
            // id 列必须显式提供（source_data.id 现为 TEXT PRIMARY KEY，无默认值；旧代码漏 id 导致整批 NOT NULL 违约被 .ok() 吞→静默无事发生）。约定 id=source_id。
            "INSERT OR IGNORE INTO source_data(id,source_type,source_id,title,content) SELECT n.note_id,'NOTE',n.note_id,n.title,'title: '||n.title||CASE WHEN COALESCE(n.content,'')!='' THEN CHAR(10)||'content: '||n.content ELSE '' END FROM notes n WHERE n.deleted_at IS NULL;
             INSERT OR IGNORE INTO source_data(id,source_type,source_id,title,content) SELECT p.id,'PRODUCT',p.id,p.name,'name: '||p.name||CASE WHEN COALESCE(p.description,'')!='' THEN CHAR(10)||'description: '||p.description ELSE '' END FROM products p;
             INSERT OR IGNORE INTO source_data(id,source_type,source_id,title,content) SELECT p.id,'PROJECT',p.id,p.name,'name: '||p.name||CASE WHEN COALESCE(p.status,'')!='' THEN CHAR(10)||'status: '||p.status ELSE '' END FROM projects p;
             INSERT OR IGNORE INTO source_data(id,source_type,source_id,title,content) SELECT t.id,'TODO',t.id,t.title,'title: '||t.title||CASE WHEN COALESCE(t.content,'')!='' THEN CHAR(10)||'content: '||t.content ELSE '' END FROM todos t;"
        )
    }).map_err(|e| format!("source_data 重灌失败: {}", e))?;
    // 重新入队所有已有文件（重新解析写入 source_data，而非插入空内容行）
    let _ = crate::database::run("DELETE FROM knowledge_push_queue WHERE source_type='FILE'", &[]);
    let _ = crate::database::run(
        "INSERT INTO knowledge_push_queue(source_type, source_id, operation, status) SELECT 'FILE', id, 'file_upload', 'pending' FROM file_assets",
        &[],
    );
    let source_count: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM source_data", [], |r| r.get(0))).unwrap_or(0);
    crate::services::knowledge_pipeline::reset_sync_time();
    crate::services::knowledge_pipeline::start();
    Ok(json!({"success":true,"message":format!("已重新同步 {} 条数据并启动管线", source_count)}))
}

#[tauri::command]
pub async fn pipeline_browse_source(source_type: Option<String>, limit: Option<i64>) -> Result<Value, String> {
    let limit = limit.unwrap_or(50);
    let rows = if let Some(st) = source_type {
        database::all("SELECT id, source_type, source_id, title, substr(content,1,500) as content_preview, length(content) as content_len, created_at, updated_at FROM source_data WHERE source_type=?1 ORDER BY updated_at DESC LIMIT ?2", &[json!(st), json!(limit)]).map_err(|e| e.to_string())?
    } else {
        database::all("SELECT id, source_type, source_id, title, substr(content,1,500) as content_preview, length(content) as content_len, created_at, updated_at FROM source_data ORDER BY updated_at DESC LIMIT ?1", &[json!(limit)]).map_err(|e| e.to_string())?
    };
    Ok(json!({"success":true,"rows":rows}))
}

#[tauri::command]
pub async fn pipeline_get_source_content(id: String) -> Result<Value, String> {
    let row = database::get("SELECT id, source_type, source_id, title, content, created_at, updated_at FROM source_data WHERE id=?", &[json!(id)]).map_err(|e| e.to_string())?;
    Ok(json!({"success":true,"row":row}))
}
