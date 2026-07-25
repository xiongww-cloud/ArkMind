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
    let source_files: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM source_data WHERE source_type='FILE'", [], |r| r.get(0))).unwrap_or(0);
    let source_notes: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM source_data WHERE source_type='NOTE'", [], |r| r.get(0))).unwrap_or(0);
    let source_products: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM source_data WHERE source_type='PRODUCT'", [], |r| r.get(0))).unwrap_or(0);
    let source_projects: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM source_data WHERE source_type='PROJECT'", [], |r| r.get(0))).unwrap_or(0);
    let source_todos: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM source_data WHERE source_type='TODO'", [], |r| r.get(0))).unwrap_or(0);
    let source_total: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM source_data", [], |r| r.get(0))).unwrap_or(0);
    // 从 knowledge_push_queue 查询文件解析状态
    let files_pending: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM knowledge_push_queue WHERE source_type='FILE' AND status='pending'", [], |r| r.get(0))).unwrap_or(0);
    let files_processing: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM knowledge_push_queue WHERE source_type='FILE' AND status='processing'", [], |r| r.get(0))).unwrap_or(0);
    let files_failed: i64 = crate::database::with_db(|c| c.query_row("SELECT COUNT(*) FROM knowledge_push_queue WHERE source_type='FILE' AND status='failed'", [], |r| r.get(0))).unwrap_or(0);
    Ok(json!({
        "knowledge": {"chunks":chunks,"parents":parents,"children":children,"vectorized":vectorized},
        "source": {"files":source_files,"notes":source_notes,"products":source_products,"projects":source_projects,"todos":source_todos,"total":source_total},
        "fileParse": {"pending":files_pending,"processing":files_processing,"failed":files_failed},
    }))
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
    // 重置 knowledge_push_queue 中的文件任务
    crate::database::run("UPDATE knowledge_push_queue SET status='pending', retry_count=0 WHERE source_type='FILE' AND status IN ('processing', 'failed')", &[]).ok();
    Ok(json!({"success":true,"message":"已清空所有管线数据"}))
}

#[tauri::command]
pub async fn pipeline_resync() -> Result<Value, String> {
    crate::database::run("DELETE FROM source_data", &[]).ok();
    crate::database::with_db(|conn| {
        conn.execute_batch(
            "INSERT OR IGNORE INTO source_data(source_type,source_id,title,content) SELECT 'NOTE',n.note_id,n.title,'title: '||n.title||CASE WHEN COALESCE(n.content,'')!='' THEN CHAR(10)||'content: '||n.content ELSE '' END FROM notes n WHERE n.deleted_at IS NULL;
             INSERT OR IGNORE INTO source_data(source_type,source_id,title,content) SELECT 'PRODUCT',p.id,p.name,'name: '||p.name||CASE WHEN COALESCE(p.description,'')!='' THEN CHAR(10)||'description: '||p.description ELSE '' END FROM products p;
             INSERT OR IGNORE INTO source_data(source_type,source_id,title,content) SELECT 'PROJECT',p.id,p.name,'name: '||p.name||CASE WHEN COALESCE(p.status,'')!='' THEN CHAR(10)||'status: '||p.status ELSE '' END FROM projects p;
             INSERT OR IGNORE INTO source_data(source_type,source_id,title,content) SELECT 'TODO',t.id,t.title,'title: '||t.title||CASE WHEN COALESCE(t.content,'')!='' THEN CHAR(10)||'content: '||t.content ELSE '' END FROM todos t;"
        )
    }).ok();
    let _ = crate::database::run("INSERT OR IGNORE INTO source_data(source_type,source_id,title,content) SELECT 'FILE',id,id,'' FROM file_assets", &[]);
    // 重置 knowledge_push_queue 中的文件任务
    crate::database::run("UPDATE knowledge_push_queue SET status='pending', retry_count=0 WHERE source_type='FILE' AND status IN ('processing', 'failed')", &[]).ok();
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
