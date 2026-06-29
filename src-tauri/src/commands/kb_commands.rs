use crate::database;
use serde_json::{json, Value};
use tauri::Emitter;

#[tauri::command] pub async fn kb_get_config() -> Result<Value, String> {
    let row = database::get("SELECT kb_config FROM system_config WHERE tenant_id='local-user'", &[]).map_err(|e| e.to_string())?;
    let config: Value = row.and_then(|r| r["kb_config"].as_str().map(|s| s.to_string())).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(json!({}));
    Ok(json!({"success":true,"embeddingApiUrl":config["embeddingApiUrl"],"embeddingApiKey":config["embeddingApiKey"],"embeddingModelName":config["embeddingModelName"],"embeddingActive":config["embeddingActive"],"chatApiUrl":config["chatApiUrl"],"chatApiKey":config["chatApiKey"],"chatModelName":config["chatModelName"]}))
}
#[tauri::command] pub async fn kb_update_config(data: Value) -> Result<Value, String> {
    database::run("UPDATE system_config SET kb_config=?,updated_at=datetime('now', 'localtime') WHERE tenant_id='local-user'", &[json!(data.to_string())]).map_err(|e| e.to_string())?;
    crate::services::embedding_client::invalidate_embedding_config_cache();
    Ok(json!({"success":true}))
}
#[tauri::command] pub async fn kb_agent_chat(app_handle: tauri::AppHandle, data: Value) -> Result<Value, String> {
    let query = data["query"].as_str().unwrap_or("").to_string();
    let max_iter = data["maxIterations"].as_i64().map(|n| n as i32);
    let history: Vec<Value> = data["history"].as_array().map(|a| a.to_vec()).unwrap_or_default();
    let result = crate::services::agent_workflow::agent_chat_workflow(query, max_iter, history, Some(app_handle.clone())).await;
    match &result { Ok(v) => { let _ = app_handle.emit("agent:done", v); if let Some(ans) = v["answer"].as_str() { if !ans.is_empty() { let preview: String = ans.chars().take(100).collect(); println!("[Agent] 响应: {}", preview); } } } Err(e) => eprintln!("[Agent] 错误: {}", e), }
    result
}
#[tauri::command] pub async fn kb_chat(app_handle: tauri::AppHandle, data: Value) -> Result<Value, String> {
    let query = data["query"].as_str().unwrap_or("").to_string();
    if query.is_empty() { return Ok(json!({"success":true,"answer":"请输入问题"})); }
    crate::services::agent_workflow::agent_chat_workflow(query, None, vec![], Some(app_handle)).await
}
#[tauri::command] pub async fn kb_import_file(data: Value) -> Result<Value, String> {
    let path = data["filePath"].as_str().unwrap_or("").to_string();
    let name = data["fileName"].as_str().unwrap_or("未命名文件").to_string();
    let ext = data["fileExt"].as_str().unwrap_or("").to_string();
    let project_id = data["projectId"].as_str().map(|s| s.to_string());
    if path.is_empty() || !std::path::Path::new(&path).exists() { return Ok(json!({"success":false,"error":"文件不存在"})); }
    let parsed = crate::services::file_parser::parse_file_with_hint(&path, &ext);
    if !parsed.success || parsed.text.is_empty() { return Ok(json!({"success":false,"error":"文件无法解析,请确认文件格式"})); }
    let (file_id, object_id) = crate::commands::file_commands::upload_file_core(&path, "library", &[], &name, "", "KB导入", None).await.map_err(|e| e.to_string())?;
    crate::database::file_sync_to_source_data(&file_id, &parsed.text);
    crate::database::run("UPDATE file_assets SET parse_status='COMPLETED' WHERE id=?", &[serde_json::json!(&file_id)]).ok();
    if let Some(pid) = project_id.filter(|p| !p.is_empty()) {
        let _ = crate::database::run("INSERT OR IGNORE INTO file_relations(id,file_id,entity_type,entity_id) VALUES(?1,?2,'project',?3)", &[serde_json::json!(uuid::Uuid::new_v4().to_string()), serde_json::json!(&object_id), serde_json::json!(&pid)]);
    }
    
    Ok(json!({"success":true,"fileId":file_id,"chunkCount":0}))
}
#[tauri::command] pub async fn kb_save_note(data: Value) -> Result<Value, String> {
    let title = data["title"].as_str().unwrap_or("未命名笔记").to_string();
    let content = data["content"].as_str().unwrap_or("").to_string();
    if content.is_empty() { return Ok(json!({"success":false,"error":"内容为空"})); }
    let note_id = uuid::Uuid::new_v4().to_string();
    database::run("INSERT INTO notes(note_id,title,content,is_daily) VALUES(?,?,?,0)",&[json!(&note_id),json!(&title),json!(&content)]).map_err(|e|e.to_string())?;
    Ok(json!({"success":true,"noteId":note_id}))
}
#[tauri::command] pub async fn kb_search(data: Value) -> Result<Value, String> {
    let query = data["query"].as_str().unwrap_or("");
    let limit = data["limit"].as_i64().unwrap_or(10);
    let results = database::fts5_search_chunks(query, limit).map_err(|e| e.to_string())?;
    Ok(json!({"success":true,"results":results}))
}
#[tauri::command] pub async fn kb_delete(doc_id: String) -> Result<Value, String> {
    let parts: Vec<&str> = doc_id.splitn(2, '-').collect();
    if parts.len() == 2 {
        let st = parts[0].to_uppercase().trim_end_matches('S').to_string();
        let sid = parts[1];
        if let Ok(Some(row)) = database::get("SELECT id FROM source_data WHERE source_type=?1 AND source_id=?2", &[json!(st), json!(sid)]) {
            if let Some(sdid) = row["id"].as_i64() { database::run("DELETE FROM knowledge_chunks WHERE source_data_id=?", &[json!(sdid)]).ok(); }
        }
    }
    database::run("DELETE FROM vector_cache WHERE chunk_id NOT IN (SELECT id FROM knowledge_chunks)",&[]).ok();
    Ok(json!({"success":true}))
}
#[tauri::command] pub async fn kb_list_documents(pid: Option<String>) -> Result<Value, String> {
    let docs = if let Some(ref p) = pid.filter(|s| !s.is_empty()) {
        database::all("SELECT fa.id, fa.file_ext, fa.file_size, fa.content_hash, fa.parse_status, fa.created_at, fa.updated_at, fr.custom_name as global_name, fr.custom_name as custom_name FROM file_assets fa INNER JOIN file_relations fr ON fr.file_id=fa.id AND fr.entity_type='project' WHERE fr.entity_id=? ORDER BY fa.updated_at DESC",&[json!(p)]).unwrap_or_default()
    } else {
        database::all("SELECT fa.id, fa.file_ext, fa.file_size, fa.content_hash, fa.parse_status, fa.created_at, fa.updated_at FROM file_assets fa ORDER BY fa.updated_at DESC LIMIT 200",&[]).unwrap_or_default()
    };
    Ok(json!({"success":true,"documents":docs}))
}
#[tauri::command] pub async fn kb_diagnose() -> Result<Value, String> {
    let chunks: i64 = database::with_db(|c| c.query_row("SELECT COUNT(*) FROM knowledge_chunks WHERE chunk_type='child'",[],|r|r.get(0))).unwrap_or(0);
    let fts: i64 = database::with_db(|c| c.query_row("SELECT COUNT(*) FROM kk_chunks_fts",[],|r|r.get(0))).unwrap_or(0);
    let vec: i64 = database::with_db(|c| c.query_row("SELECT COUNT(*) FROM vector_cache",[],|r|r.get(0))).unwrap_or(0);
    let doc: i64 = database::with_db(|c| c.query_row("SELECT COUNT(*) FROM file_assets",[],|r|r.get(0))).unwrap_or(0);
    Ok(json!({"success":true,"chunks":chunks,"ftsIndexed":fts,"vectorized":vec,"documents":doc}))
}
#[tauri::command] pub async fn kb_search_debug(query: String) -> Result<Value, String> {
    let results = database::fts5_search_chunks(&query, 20).map_err(|e| e.to_string())?;
    Ok(json!({"success":true,"query":query,"count":results.len(),"results":results}))
}
#[tauri::command] pub async fn kb_data_browse(query: Option<String>, page: Option<i64>) -> Result<Value, String> {
    let q = query.unwrap_or_default(); let p = page.unwrap_or(1).max(1); let offset = (p - 1) * 50;
    let rows = if q.is_empty() {
        database::all("SELECT kc.rowid,kc.parent_id,kc.content,kc.chunk_index,kc.created_at FROM knowledge_chunks kc WHERE kc.chunk_type='child' ORDER BY kc.created_at DESC LIMIT 50 OFFSET ?",&[json!(offset)]).unwrap_or_default()
    } else {
        database::all("SELECT kc.rowid,kc.parent_id,kc.content,kc.chunk_index,kc.created_at FROM knowledge_chunks kc WHERE kc.chunk_type='child' AND kc.content LIKE ?1 ORDER BY kc.created_at DESC LIMIT 50 OFFSET ?2",&[json!(format!("%{}%",q)), json!(offset)]).unwrap_or_default()
    };
    let total: i64 = database::with_db(|c| c.query_row("SELECT COUNT(*) FROM knowledge_chunks WHERE chunk_type='child'",[],|r|r.get(0))).unwrap_or(0);
    Ok(json!({"success":true,"rows":rows,"total":total,"page":p}))
}
#[tauri::command] pub async fn kb_browse_parents(query: Option<String>, page: Option<i64>) -> Result<Value, String> {
    let q = query.unwrap_or_default(); let p = page.unwrap_or(1).max(1); let offset = (p - 1) * 50;
    let rows = if q.is_empty() {
        database::all("SELECT kc.rowid,kc.content,kc.doc_title,kc.chunk_index,kc.source_data_id,kc.created_at,sd.source_type FROM knowledge_chunks kc LEFT JOIN source_data sd ON sd.id=kc.source_data_id WHERE kc.chunk_type='parent' ORDER BY kc.created_at DESC LIMIT 50 OFFSET ?",&[json!(offset)]).unwrap_or_default()
    } else {
        database::all("SELECT kc.rowid,kc.content,kc.doc_title,kc.chunk_index,kc.source_data_id,kc.created_at,sd.source_type FROM knowledge_chunks kc LEFT JOIN source_data sd ON sd.id=kc.source_data_id WHERE kc.chunk_type='parent' AND kc.content LIKE ?1 ORDER BY kc.created_at DESC LIMIT 50 OFFSET ?2",&[json!(format!("%{}%",q)), json!(offset)]).unwrap_or_default()
    };
    let total: i64 = database::with_db(|c| c.query_row("SELECT COUNT(*) FROM knowledge_chunks WHERE chunk_type='parent'",[],|r|r.get(0))).unwrap_or(0);
    Ok(json!({"success":true,"rows":rows,"total":total,"page":p}))
}
#[tauri::command] pub async fn kb_browse_vectors(limit: Option<usize>) -> Result<Value, String> {
    let lim = limit.unwrap_or(50); let mut rows: Vec<Value> = Vec::new();
    if let Ok(g) = crate::services::lancedb_client::get_lancedb() {
        if let Some(ref c) = *g {
            if let Ok(items) = c.scan_vectors(lim) {
                for (rowid, parent_id, doc_id, tags) in &items {
                    let content = database::with_db(|conn| conn.query_row::<String, _, _>("SELECT content FROM knowledge_chunks WHERE rowid=?1", [rowid], |r| r.get(0))).unwrap_or_default();
                    rows.push(json!({"rowid": rowid,"parent_id": parent_id,"doc_id": doc_id,"tags": tags,"content": content}));
                }
            }
        }
    }
    Ok(json!({"success":true,"rows":rows, "count": rows.len()}))
}
#[tauri::command] pub async fn kb_inspect_fts5(query: Option<String>, _page: Option<i64>) -> Result<Value, String> {
    let q = query.unwrap_or_default();
    let results = if q.is_empty() { database::all("SELECT rowid,content FROM kk_chunks_fts ORDER BY rowid LIMIT 50",&[]).unwrap_or_default() } else { database::all("SELECT rowid,content FROM kk_chunks_fts WHERE kk_chunks_fts MATCH ? LIMIT 50",&[json!(q)]).unwrap_or_default() };
    Ok(json!({"success":true,"results":results}))
}
#[tauri::command] pub async fn kb_fts5_inspector() -> Result<Value, String> {
    let schema: Vec<Value> = database::all("SELECT sql FROM sqlite_master WHERE type='table' AND name LIKE '%fts%'",&[]).unwrap_or_default();
    let count: i64 = database::with_db(|c| c.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name LIKE '%fts%'",[],|r|r.get(0))).unwrap_or(0);
    Ok(json!({"success":true,"ftsTables":count,"schema":schema}))
}
#[tauri::command] pub async fn kb_get_table_chunks(file_id: String) -> Result<Value, String> {
    let chunks = database::all("SELECT kc.rowid,kc.parent_id,kc.content,kc.created_at FROM knowledge_chunks kc WHERE kc.file_id=? ORDER BY kc.chunk_index ASC",&[json!(file_id)]).map_err(|e|e.to_string())?;
    Ok(json!({"success":true,"chunks":chunks}))
}
#[tauri::command] pub async fn execute_production_rag(_data: Value) -> Result<Value, String> { Ok(json!({"success":true,"message":"已通过 kb_chat / kb_agent_chat 接入生产 RAG"})) }
#[tauri::command] pub async fn kb_test_embedding(data: Value) -> Result<Value, String> {
    let _url = data["url"].as_str().unwrap_or(""); let _key = data["key"].as_str().unwrap_or(""); let _model = data["model"].as_str().unwrap_or("text-embedding-3-small");
    match crate::services::embedding_client::get_embedding_with_caller("Arkmind test embedding ping", "kb_test").await { Ok(v) => Ok(json!({"success":true,"message":format!("嵌入 API 连接成功,向量维数: {}", v.len())})), Err(e) => Ok(json!({"success":false,"error":e})) }
}
#[tauri::command] pub async fn kb_test_chat(_data: Value) -> Result<Value, String> {
    let _config = crate::services::chat_api::get_chat_config();
    match crate::services::chat_api::chat_completion("只输出OK", "ping", None).await { Ok(_) => Ok(json!({"success":true,"message":"对话 API 连接成功"})), Err(e) => Ok(json!({"success":false,"error":e})) }
}

// Agent tools

#[tauri::command]
pub async fn ext_tool_search_knowledge_base(keywords: Vec<String>, project_id: Option<String>, filter_tags: Option<Vec<String>>) -> Result<String, String> {
    let query = keywords.join(" ");
    if query.trim().is_empty() { return Ok("未提供检索关键词".into()); }
    let tag_rowids: std::collections::HashSet<i64> = match &filter_tags {
        Some(tags) if !tags.is_empty() => crate::services::agent_workflow::get_child_rowids_by_tag_names(tags).into_iter().collect(),
        _ => std::collections::HashSet::new(),
    };
    let query_vector = crate::services::embedding_client::get_embedding_with_caller(&query, "production_rag").await.map_err(|e| format!("embed: {}", e))?;
    let vec_rowids: Vec<(String, f64)> = match crate::services::lancedb_client::get_lancedb() {
        Ok(guard) => match guard.as_ref() { Some(client) => client.search_vectors_with_parent(&query_vector, 30).unwrap_or_default().into_iter().map(|(r,s,_,_,_)| (r.to_string(), s)).collect(), None => Vec::new() }, Err(_) => Vec::new()
    };
    let fts_query: Vec<String> = keywords.iter().map(|k| format!("\"{}\"", k.replace('"', ""))).collect();
    let fts_query_str = fts_query.join(" OR ");
    let fts_results = database::fts5_search_chunks(&fts_query_str, 30).unwrap_or_default();
    let fts_rowids: Vec<(String, f64)> = fts_results.iter().map(|fr| (fr.rowid.to_string(), fr.rank)).collect();
    let fuse = |rows: Vec<(String,f64)>| -> Vec<(String,f64)> { if tag_rowids.is_empty() { rows } else { rows.into_iter().filter(|(id,_)| id.parse::<i64>().map(|r| tag_rowids.contains(&r)).unwrap_or(false)).collect() } };
    let fused = crate::services::search_assets::rrf_fusion(&fuse(fts_rowids), &fuse(vec_rowids), 60.0);
    if fused.is_empty() { return Ok("未找到相关知识切片".into()); }
    let mut seen = std::collections::HashSet::new(); let mut output = String::new(); let mut count = 0usize;
    for (rowid_str, rrf_score) in fused.iter() {
        if count >= 5 { break; }
        let Ok(child_rowid) = rowid_str.parse::<i64>() else { continue; };
        let parent = database::get("SELECT p.content, p.doc_title, p.chunk_index, p.chunk_id, p.source_data_id, sd.source_type, sd.source_id FROM knowledge_chunks c JOIN knowledge_chunks p ON p.chunk_id=c.parent_id LEFT JOIN source_data sd ON sd.id=p.source_data_id WHERE c.rowid=?1", &[json!(child_rowid)]).ok().flatten();
        if let Some(ref r) = parent {
            let st = r["source_type"].as_str().unwrap_or(""); let sid = r["source_id"].as_str().unwrap_or("");
            let dedup = format!("{}-{}", st, sid);
            if !dedup.trim_matches('-').is_empty() && !seen.insert(dedup) { continue; }
            let _total_parents: i64 = r["source_data_id"].as_str().map(|sdid| database::get("SELECT COUNT(*) as cnt FROM knowledge_chunks WHERE source_data_id=?1 AND chunk_type='parent'", &[serde_json::json!(sdid)]).ok().flatten().and_then(|r| r["cnt"].as_i64()).unwrap_or(0)).unwrap_or(0);
            let _chunk_id = r["chunk_id"].as_str().unwrap_or(""); let _chunk_index = r["chunk_index"].as_i64().unwrap_or(0);
            if let Some(c) = r["content"].as_str() { count += 1; let t: String = c.chars().take(1300).collect();
                let meta = crate::services::agent_workflow::build_chunk_metadata(st, sid);
                if meta.is_empty() { output.push_str(&format!("\n\n--- [父块{}|RRF={:.3}] ---\n{}", count, rrf_score, t)); }
                else { output.push_str(&format!("\n\n--- [父块{}|RRF={:.3}] ---\n{}\n--------\n{}", count, rrf_score, meta, t)); }
            }
        }
    }
    if output.trim().is_empty() { return Ok("父块文本未能加载".into()); }
    let _ = project_id;
    Ok(output.trim().to_string())
}
#[tauri::command] pub async fn ext_tool_get_document_page(doc_id: String, _page_num: usize) -> Result<String, String> {
    let doc = database::get("SELECT content FROM documents_parent WHERE document_id=? LIMIT 1",&[json!(doc_id)]).ok().flatten();
    match doc.and_then(|d| d["content"].as_str().map(|s| s.to_string())) { Some(c) => Ok(format!("文档 {} 内容:\n{}", doc_id, c)), None => Ok(format!("文档 {} 未找到", doc_id)) }
}
#[tauri::command] pub async fn ext_tool_find_related_knowledge(_doc_id: String) -> Result<String, String> { Ok("[]".to_string()) }
#[tauri::command] pub async fn ext_tool_search_by_tag(tag_name: String) -> Result<String, String> {
    let results = database::fts5_search_chunks(&tag_name, 20).unwrap_or_default();
    let ids: Vec<String> = results.iter().map(|r| r.rowid.to_string()).collect();
    Ok(serde_json::to_string(&ids).unwrap_or_else(|_| "[]".to_string()))
}

// Chat session persistence
#[tauri::command] pub async fn kb_save_sessions(data: Value) -> Result<Value, String> {
    let sessions = data["sessions"].as_array().ok_or("missing sessions array")?;
    for s in sessions.iter() {
        let id = s["id"].as_str().ok_or("missing session id")?;
        let _title = s["title"].as_str().or_else(|| s["name"].as_str()).unwrap_or("");
        let updated_at = s["updatedAt"].as_str().or_else(|| s["updated_at"].as_str())
            .unwrap_or(&chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f").to_string())
            .to_string();
        database::run("INSERT INTO chat_sessions(id,title,created_at,updated_at) VALUES(?1,?2,?3,?3) ON CONFLICT(id) DO UPDATE SET title=excluded.title, updated_at=excluded.updated_at",
            &[json!(id), json!(_title), json!(updated_at)]).map_err(|e| e.to_string())?;
        let msgs_raw = s["messages"].as_array().cloned().or_else(|| s["messages"].as_str().and_then(|s| serde_json::from_str::<Vec<Value>>(s).ok()));
        if let Some(ref msgs) = msgs_raw {
            database::run("DELETE FROM chat_messages WHERE session_id=?", &[json!(id)]).ok();
            for m in msgs {
                let role = m["role"].as_str().unwrap_or("");
                let content = m["content"].as_str().unwrap_or("");
                if role != "user" && role != "assistant" { continue; }
                if content.trim().is_empty() { continue; }
                database::run("INSERT INTO chat_messages(session_id,role,content) VALUES(?1,?2,?3)",
                    &[json!(id), json!(role), json!(content)]).ok();
            }
            // 为每个问答对创建独立的 source_data 条目
            let mut qa_pairs: Vec<(String, String)> = Vec::new();
            let mut current_user = String::new();
            for m in msgs {
                let role = m["role"].as_str().unwrap_or("");
                let content = m["content"].as_str().unwrap_or("").trim();
                if content.is_empty() { continue; }
                if role == "user" {
                    current_user = content.to_string();
                } else if role == "assistant" && !current_user.is_empty() && !content.is_empty() {
                    qa_pairs.push((current_user.clone(), content.to_string()));
                    current_user.clear();
                }
            }
            for (qi, (q, a)) in qa_pairs.iter().enumerate() {
                let chat_id = format!("{}-{}", id, qi);
                let content = format!("[用户问]: {}\n[AI答]: {}", q, a);
                let existing = database::get(
                    "SELECT content FROM source_data WHERE source_type='CHAT' AND source_id=?1",
                    &[json!(&chat_id)],
                ).ok().flatten().and_then(|r| r["content"].as_str().map(String::from));
                if existing.as_deref() == Some(&content) { continue; }
                database::run("DELETE FROM source_data WHERE source_type='CHAT' AND source_id=?", &[json!(&chat_id)]).ok();
                if let Err(_) = database::run(
                    "INSERT INTO source_data(id,source_type,source_id,title,content,updated_at) VALUES(?1,'CHAT',?1,?2,?3,?4)",
                    &[json!(chat_id), json!(q), json!(content), json!(&updated_at)],
                ) {}
                crate::services::ai_sync::trigger_sync("chat", &chat_id);
            }
            let prefix = format!("{}-", id);
            database::run(
                "DELETE FROM source_data WHERE source_type='CHAT' AND source_id LIKE ?1 AND CAST(SUBSTR(source_id, LENGTH(?1)+1) AS INTEGER) >= ?2",
                &[json!(&prefix), json!(qa_pairs.len() as i64)],
            ).ok();
        }
    }
    Ok(json!({"success":true}))
}
#[tauri::command] pub async fn kb_load_sessions() -> Result<Value, String> {
    let mut rows = database::all(
        "SELECT s.id, s.title, s.created_at, s.updated_at, \
         (SELECT json_group_array(json_object('role',m.role,'content',m.content)) \
          FROM chat_messages m WHERE m.session_id=s.id ORDER BY m.id) as messages \
         FROM chat_sessions s ORDER BY s.updated_at DESC LIMIT 100",
        &[],
    ).unwrap_or_default();
    for row in &mut rows {
        if let Some(s) = row["messages"].as_str() {
            if let Ok(arr) = serde_json::from_str::<Value>(s) {
                let m = row.as_object_mut().unwrap();
                m.insert("messages".to_string(), arr);
            }
        }
    }
    Ok(json!({"success":true,"sessions":rows}))
}

#[tauri::command] pub async fn kb_delete_session(session_id: String) -> Result<Value, String> {
    database::run("DELETE FROM chat_sessions WHERE id=?", &[json!(&session_id)]).ok();
    Ok(json!({"success":true}))
}

// Tag management

// ═══════════════════════════════════════════════════════════════════
// Memory Explorer API
// ═══════════════════════════════════════════════════════════════════

/// 搜索所有对象（source_data），类似 Object 工具但返回 JSON
#[tauri::command]
pub async fn kb_memory_objects(query: String) -> Result<Value, String> {
    let q = query.trim().to_string();
    if q.is_empty() { return Ok(json!({"success":true,"objects":[]})); }
    let like = format!("%{}%", q);
    // FTS5 try first, fallback to LIKE
    let fts_q = q.split_whitespace().map(|k| format!("\"{}\"", k.replace('"',""))).collect::<Vec<_>>().join(" OR ");
    let mut objects: Vec<Value> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    if let Ok(rows) = database::all(
        "SELECT rowid, source_type, source_id, title FROM source_data_fts WHERE source_data_fts MATCH ?1 LIMIT 30",
        &[json!(&fts_q)],
    ) {
        for r in &rows {
            let st = r["source_type"].as_str().unwrap_or("").to_string();
            let sid = r["source_id"].as_str().unwrap_or("").to_string();
            let key = format!("{}-{}", st, sid);
            if !seen.insert(key) { continue; }
            objects.push(json!({"source_type": st, "source_id": sid, "title": r["title"].as_str().unwrap_or("")}));
        }
    }
    if objects.is_empty() {
        if let Ok(rows) = database::all(
            "SELECT source_type, source_id, title FROM source_data WHERE title LIKE ?1 OR content LIKE ?1 LIMIT 20",
            &[json!(&like)],
        ) {
            for r in &rows {
                let st = r["source_type"].as_str().unwrap_or("").to_string();
                let sid = r["source_id"].as_str().unwrap_or("").to_string();
                let key = format!("{}-{}", st, sid);
                if !seen.insert(key) { continue; }
                objects.push(json!({"source_type": st, "source_id": sid, "title": r["title"].as_str().unwrap_or("")}));
            }
        }
    }
    // ★ 补全文件标题：FILE 类型的 title 在 source_data 中为空，从 file_relations 获取
    for obj in &mut objects {
        if obj["source_type"].as_str() == Some("FILE") {
            let sid = obj["source_id"].as_str().unwrap_or("");
            let name = obj["title"].as_str().unwrap_or("");
            if name.is_empty() {
                if let Ok(Some(row)) = database::get(
                    "SELECT COALESCE(fr.custom_name, fa.file_ext, fa.id) as name \
                     FROM file_assets fa LEFT JOIN file_relations fr ON fr.file_id=fa.id \
                     WHERE fa.id=? ORDER BY COALESCE(fr.custom_name,'') DESC LIMIT 1",
                    &[json!(sid)],
                ) {
                    obj["title"] = row["name"].clone();
                }
            }
        }
    }
    // ★ 补全 NOTE/TODO 的项目关联信息
    for obj in &mut objects {
        let st = obj["source_type"].as_str().unwrap_or("");
        let sid = obj["source_id"].as_str().unwrap_or("");
        let (table, id_col) = match st {
            "NOTE" => ("note_project_mapping", "note_id"),
            "TODO" => ("todo_project_mapping", "todo_id"),
            _ => continue,
        };
        if let Ok(Some(row)) = database::get(
            &format!("SELECT p.id as project_id, p.name as project_name FROM {} npm JOIN projects p ON p.id=npm.project_id WHERE npm.{} = ? LIMIT 1", table, id_col),
            &[json!(sid)],
        ) {
            obj["project_id"] = row["project_id"].clone();
            obj["project_name"] = row["project_name"].clone();
        }
    }
    // ★ 过滤已完成的待办（不在全局搜索中出现）
    objects.retain(|obj| {
        if obj["source_type"].as_str() != Some("TODO") { return true }
        let sid = obj["source_id"].as_str().unwrap_or("");
        let done: bool = database::get("SELECT done FROM todos WHERE id=?", &[json!(sid)])
            .ok().flatten()
            .and_then(|r| r["done"].as_i64())
            .map(|d| d != 0)
            .unwrap_or(false);
        !done
    });
    // ★ 过滤已关联项目的笔记/待办（应通过项目搜索定位）
    objects.retain(|obj| {
        let st = obj["source_type"].as_str().unwrap_or("");
        if st != "NOTE" && st != "TODO" { return true }
        obj["project_id"].is_null() || obj["project_id"].as_str().map_or(true, |s| s.is_empty())
    });
    Ok(json!({"success":true,"objects":objects}))
}

/// 业务表检索：按名称模糊匹配 projects/products/notes/file_relations
#[tauri::command]
pub async fn search_business(query: String) -> Result<Value, String> {
    let q = query.trim().to_string();
    if q.is_empty() { return Ok(json!({"success":true,"objects":[]})); }
    let like = format!("%{}%", q);
    let mut objects: Vec<Value> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    // projects
    if let Ok(rows) = database::all("SELECT id, name FROM projects WHERE name LIKE ?1 LIMIT 10", &[json!(&like)]) {
        for r in &rows {
            let id = r["id"].as_str().unwrap_or("").to_string();
            let name = r["name"].as_str().unwrap_or("").to_string();
            if seen.insert(format!("PROJECT-{}", id)) {
                objects.push(json!({"source_type":"PROJECT","source_id":id,"title":name}));
            }
        }
    }
    // products
    if let Ok(rows) = database::all("SELECT id, name FROM products WHERE name LIKE ?1 LIMIT 10", &[json!(&like)]) {
        for r in &rows {
            let id = r["id"].as_str().unwrap_or("").to_string();
            let name = r["name"].as_str().unwrap_or("").to_string();
            if seen.insert(format!("PRODUCT-{}", id)) {
                objects.push(json!({"source_type":"PRODUCT","source_id":id,"title":name}));
            }
        }
    }
    // notes
    if let Ok(rows) = database::all("SELECT note_id, title FROM notes WHERE title LIKE ?1 AND deleted_at IS NULL LIMIT 10", &[json!(&like)]) {
        for r in &rows {
            let id = r["note_id"].as_str().unwrap_or("").to_string();
            let name = r["title"].as_str().unwrap_or("").to_string();
            if seen.insert(format!("NOTE-{}", id)) {
                objects.push(json!({"source_type":"NOTE","source_id":id,"title":name}));
            }
        }
    }
    // file_relations
    if let Ok(rows) = database::all("SELECT file_id, custom_name FROM file_relations WHERE custom_name LIKE ?1 LIMIT 10", &[json!(&like)]) {
        for r in &rows {
            let id = r["file_id"].as_str().unwrap_or("").to_string();
            let name = r["custom_name"].as_str().unwrap_or("").to_string();
            if seen.insert(format!("FILE-{}", id)) {
                objects.push(json!({"source_type":"FILE","source_id":id,"title":name}));
            }
        }
    }
    // todos (未完成)
    if let Ok(rows) = database::all("SELECT id, title FROM todos WHERE title LIKE ?1 AND done=0 LIMIT 10", &[json!(&like)]) {
        for r in &rows {
            let id = r["id"].as_str().unwrap_or("").to_string();
            let name = r["title"].as_str().unwrap_or("").to_string();
            if seen.insert(format!("TODO-{}", id)) {
                objects.push(json!({"source_type":"TODO","source_id":id,"title":name}));
            }
        }
    }
    Ok(json!({"success":true,"objects":objects}))
}

/// source_data FTS5 检索
#[tauri::command]
pub async fn search_source_data(query: String) -> Result<Value, String> {
    let q = query.trim().to_string();
    if q.is_empty() { return Ok(json!({"success":true,"objects":[]})); }
    let fts_q = q.split_whitespace().map(|k| format!("\"{}\"", k.replace('"',""))).collect::<Vec<_>>().join(" OR ");
    let mut objects: Vec<Value> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    if let Ok(rows) = database::all(
        "SELECT source_type, source_id, title FROM source_data_fts WHERE source_data_fts MATCH ?1 LIMIT 30",
        &[json!(&fts_q)],
    ) {
        for r in &rows {
            let st = r["source_type"].as_str().unwrap_or("").to_string();
            let sid = r["source_id"].as_str().unwrap_or("").to_string();
            let key = format!("{}-{}", st, sid);
            if seen.insert(key) {
                objects.push(json!({"source_type": st, "source_id": sid}));
            }
        }
    }
    Ok(json!({"success":true,"objects":objects}))
}

/// 获取所有关联了项目的笔记 ID 集合
#[tauri::command]
pub async fn kb_project_note_ids() -> Result<Value, String> {
    let rows = database::all("SELECT note_id FROM note_project_mapping", &[]).unwrap_or_default();
    let ids: Vec<String> = rows.iter().filter_map(|r| r["note_id"].as_str().map(|s| s.to_string())).collect();
    Ok(json!({"ids": ids}))
}

/// 获取指定对象的父块列表（含突触权重、实体、标签）
#[tauri::command]
pub async fn kb_memory_parents(source_type: String, source_id: String) -> Result<Value, String> {
    let _sid = format!("{}-{}", source_type, source_id);
    let parents = database::all(
        "SELECT kc.rowid, kc.chunk_id, kc.content, kc.doc_title, kc.chunk_index, kc.created_at, kc.base_weight, kc.last_access_at, kc.access_count, strftime('%s', kc.created_at) as created_ts \
         FROM knowledge_chunks kc WHERE kc.chunk_type='parent' \
         AND kc.source_data_id = (SELECT id FROM source_data WHERE source_type=?1 AND source_id=?2) \
         ORDER BY kc.chunk_index ASC",
        &[json!(source_type), json!(source_id)],
    ).unwrap_or_default();

    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
    const GAMMA: f64 = 0.69314718056 / (30.0 * 86400.0);

    let mut result: Vec<Value> = Vec::new();
    let mut seen_parent_ids = std::collections::HashSet::new();
    for p in &parents {
        let pid = p["chunk_id"].as_str().unwrap_or("").to_string();
        if pid.is_empty() || !seen_parent_ids.insert(pid.clone()) { continue; }
        let base = p["base_weight"].as_f64().unwrap_or(1.0);
        let lat = p["last_access_at"].as_i64().unwrap_or(0);
        let access_count = p["access_count"].as_i64().unwrap_or(0);
        let effective = if lat > 0 { base * (-GAMMA * ((now as i64 - lat) as f64)).exp() } else { let created = p["created_ts"].as_i64().unwrap_or(now as i64); base * (-GAMMA * ((now as i64 - created) as f64)).exp() };
        let content = p["content"].as_str().unwrap_or("").chars().take(300).collect::<String>();
        let doc_title = p["doc_title"].as_str().unwrap_or("").to_string();

        // Tags
        let tags: Vec<String> = database::all(
            "SELECT DISTINCT t.name FROM tags t JOIN tag_mappings tm ON tm.tag_id=t.tag_id WHERE tm.entity_type=?1 AND tm.entity_id=?2 LIMIT 10",
            &[json!(source_type.to_lowercase() + "s"), json!(&source_id)],
        ).unwrap_or_default().iter().filter_map(|r| r["name"].as_str().map(String::from)).collect();

        // Child count
        let child_count: i64 = database::with_db(|c| c.query_row(
            "SELECT COUNT(*) FROM knowledge_chunks WHERE chunk_type='child' AND parent_id=?1",
            [&pid], |r| r.get(0),
        )).unwrap_or(0);

        let stars = if effective >= 4.0 { 5 } else if effective >= 3.0 { 4 } else if effective >= 2.0 { 3 } else if effective >= 1.0 { 2 } else { 1 };

        result.push(json!({
            "parent_id": pid,
            "chunk_index": p["chunk_index"].as_i64().unwrap_or(0),
            "doc_title": doc_title,
            "content_preview": content,
            "base_weight": base,
            "effective_weight": (effective * 100.0).round() / 100.0,
            "access_count": access_count,
            "child_count": child_count,
            "stars": stars,
            "tags": tags,
            "created_at": p["created_at"].as_str().unwrap_or(""),
        }));
    }
    Ok(json!({"success":true,"parents":result}))
}

/// 热点/冷块统计
#[tauri::command]
pub async fn kb_memory_heatmap() -> Result<Value, String> {
    let total: i64 = database::with_db(|c| c.query_row("SELECT COUNT(*) FROM knowledge_chunks WHERE chunk_type='child'",[],|r|r.get(0))).unwrap_or(0);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0) as i64;
    const GAMMA: f64 = 0.69314718056 / (30.0 * 86400.0);

    // Hot parents: top 10 by effective_weight
    let mut hot: Vec<Value> = Vec::new();
    if let Ok(rows) = database::all(
        "SELECT kc.rowid, kc.chunk_id, kc.doc_title, kc.base_weight, kc.last_access_at, kc.access_count, kc.source_data_id, sd.source_type, sd.source_id, sd.title as source_title, strftime('%s', kc.created_at) as created_ts \
         FROM knowledge_chunks kc LEFT JOIN source_data sd ON sd.id=kc.source_data_id \
         WHERE kc.chunk_type='child' AND kc.base_weight > 0 ORDER BY kc.base_weight DESC LIMIT 10",
        &[],
    ) {
        for r in &rows {
            let base = r["base_weight"].as_f64().unwrap_or(1.0);
            let lat = r["last_access_at"].as_i64().unwrap_or(0);
            let effective = if lat > 0 { base * (-GAMMA * (now - lat) as f64).exp() } else { let created = r["created_ts"].as_i64().unwrap_or(now); base * (-GAMMA * (now - created) as f64).exp() };
            if effective < 0.5 { continue; }
            hot.push(json!({
                "parent_id": r["chunk_id"].as_str().unwrap_or("").to_string(),
                "title": r["doc_title"].as_str().filter(|s|!s.is_empty()).or_else(|| r["source_title"].as_str()).unwrap_or(""),
                "effective_weight": (effective * 100.0).round() / 100.0,
                "access_count": r["access_count"].as_i64().unwrap_or(0),
                "source_type": r["source_type"].as_str().unwrap_or(""),
                "source_id": r["source_id"].as_str().unwrap_or(""),
            }));
        }
    }

    // Cold parents: top 10 by effective_weight (lowest)
    let mut cold: Vec<Value> = Vec::new();
    let threshold_ts = now - 90 * 86400; // 90 days ago
    if let Ok(rows) = database::all(
        "SELECT kc.rowid, kc.chunk_id, kc.doc_title, kc.base_weight, kc.last_access_at, kc.access_count, kc.source_data_id, sd.source_type, sd.source_id, sd.title as source_title, kc.created_at \
         FROM knowledge_chunks kc LEFT JOIN source_data sd ON sd.id=kc.source_data_id \
         WHERE kc.chunk_type='child' AND COALESCE(kc.last_access_at, 0) < ?1 \
         ORDER BY COALESCE(kc.last_access_at, 0) ASC LIMIT 10",
        &[json!(threshold_ts)],
    ) {
        for r in &rows {
            cold.push(json!({
                "parent_id": r["chunk_id"].as_str().unwrap_or("").to_string(),
                "title": r["doc_title"].as_str().filter(|s|!s.is_empty()).or_else(|| r["source_title"].as_str()).unwrap_or(""),
                "last_access": r["last_access_at"].as_i64().unwrap_or(0),
                "access_count": r["access_count"].as_i64().unwrap_or(0),
                "source_type": r["source_type"].as_str().unwrap_or(""),
                "source_id": r["source_id"].as_str().unwrap_or(""),
            }));
        }
    }

    Ok(json!({"success":true,"total_chunks":total,"hot":hot,"cold":cold}))
}

#[tauri::command]
pub async fn kb_memory_dashboard_stats() -> Result<Value, String> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0) as i64;
    const GAMMA: f64 = 0.69314718056 / (30.0 * 86400.0);
    const MAX_WEIGHT: f64 = 5.0;

    // 1. By source type (child chunks)
    let by_type = database::all(
        "SELECT COALESCE(sd.source_type,'未知') as label, COUNT(*) as cnt \
         FROM knowledge_chunks kc LEFT JOIN source_data sd ON sd.id=kc.source_data_id \
         WHERE kc.chunk_type='child' GROUP BY sd.source_type ORDER BY cnt DESC", &[],
    ).unwrap_or_default();

    // 2. Weight distribution (child chunks, effective_weight → percentage buckets 0-100%, step 5%)
    let mut buckets = vec![0i64; 21]; // 0%, 5%, 10%, ..., 100%
    let mut child_total: i64 = 0;
    let mut effective_sum: f64 = 0.0;
    let mut hot_count: i64 = 0;
    let mut cold_count: i64 = 0;
    if let Ok(rows) = database::all(
        "SELECT base_weight, last_access_at, strftime('%s', created_at) as created_ts FROM knowledge_chunks WHERE chunk_type='child'", &[],
    ) {
        for r in &rows {
            let base = r["base_weight"].as_f64().unwrap_or(1.0);
            let lat = r["last_access_at"].as_i64().unwrap_or(0);
            let effective = if lat > 0 { base * (-GAMMA * (now - lat) as f64).exp() } else { let created = r["created_ts"].as_i64().unwrap_or(now); base * (-GAMMA * (now - created) as f64).exp() };
            let pct = (effective / MAX_WEIGHT * 100.0).clamp(0.0, 100.0);
            let idx = (pct / 5.0).round() as usize;
            let idx = idx.min(20);
            buckets[idx] += 1;
            child_total += 1;
            effective_sum += effective;
            if effective >= 2.0 { hot_count += 1; }
            if lat > 0 && lat < now - 90 * 86400 { cold_count += 1; }
        }
    }
    let weight_dist: Vec<Value> = (0..=20).map(|i| {
        json!({"label": format!("{}%", i * 5), "cnt": buckets[i]})
    }).collect();
    let avg_weight = if child_total > 0 { (effective_sum / child_total as f64 * 100.0).round() / 100.0 } else { 0.0 };

    // 3. Daily growth (last 365 days, child chunks — frontend filters by selected range)
    let daily_growth = database::all(
        "SELECT date(created_at) as label, COUNT(*) as cnt \
         FROM knowledge_chunks WHERE chunk_type='child' \
         AND created_at >= date('now', '-365 days', 'localtime') \
         GROUP BY date(created_at) ORDER BY label", &[],
    ).unwrap_or_default();

    // 4. Object counts from source_data
    let object_total: i64 = database::with_db(|c| c.query_row(
        "SELECT COUNT(*) FROM source_data", [], |r| r.get(0)
    )).unwrap_or(0);
    let object_by_type = database::all(
        "SELECT COALESCE(source_type,'未知') as label, COUNT(*) as cnt \
         FROM source_data GROUP BY source_type ORDER BY cnt DESC", &[],
    ).unwrap_or_default();

    // 5. Pipeline status
    let sync_embedded: i64 = database::with_db(|c| c.query_row(
        "SELECT COUNT(*) FROM ai_sync_queue WHERE is_embedded=1", [], |r| r.get(0)
    )).unwrap_or(0);
    let sync_pending: i64 = database::with_db(|c| c.query_row(
        "SELECT COUNT(*) FROM ai_sync_queue WHERE is_embedded=0", [], |r| r.get(0)
    )).unwrap_or(0);
    let sync_failed: i64 = database::with_db(|c| c.query_row(
        "SELECT COUNT(*) FROM ai_sync_queue WHERE retry_count >= 3", [], |r| r.get(0)
    )).unwrap_or(0);
    let tag_done: i64 = database::with_db(|c| c.query_row(
        "SELECT COUNT(*) FROM ai_tag_queue WHERE status=1", [], |r| r.get(0)
    )).unwrap_or(0);
    let tag_pending: i64 = database::with_db(|c| c.query_row(
        "SELECT COUNT(*) FROM ai_tag_queue WHERE status=0", [], |r| r.get(0)
    )).unwrap_or(0);

    // 6. Tag count
    let tag_count: i64 = database::with_db(|c| c.query_row(
        "SELECT COUNT(*) FROM tags WHERE type != 'ai_shadow'", [], |r| r.get(0)
    )).unwrap_or(0);

    Ok(json!({
        "success": true,
        "by_type": by_type,
        "weight_dist": weight_dist,
        "daily_growth": daily_growth,
        "object_counts": { "total": object_total, "by_type": object_by_type },
        "pipeline": { "embedded": sync_embedded, "pending": sync_pending, "failed": sync_failed, "tagged": tag_done, "untagged": tag_pending },
        "health": {
            "total": child_total,
            "hot_count": hot_count,
            "cold_count": cold_count,
            "avg_weight": avg_weight,
            "tag_count": tag_count,
            "hot_ratio": if child_total > 0 { (hot_count as f64 / child_total as f64 * 100.0).round() } else { 0.0 },
            "cold_ratio": if child_total > 0 { (cold_count as f64 / child_total as f64 * 100.0).round() } else { 0.0 },
        }
    }))
}

#[tauri::command]
pub async fn tag_get_state() -> Result<Value, String> {
    let total: i64 = database::with_db(|c| c.query_row("SELECT COUNT(*) FROM ai_tag_queue",[],|r|r.get(0))).unwrap_or(0);
    let pending: i64 = database::with_db(|c| c.query_row("SELECT COUNT(*) FROM ai_tag_queue WHERE status=0",[],|r|r.get(0))).unwrap_or(0);
    let done: i64 = database::with_db(|c| c.query_row("SELECT COUNT(*) FROM ai_tag_queue WHERE status=1",[],|r|r.get(0))).unwrap_or(0);
    let failed: i64 = database::with_db(|c| c.query_row("SELECT COUNT(*) FROM ai_tag_queue WHERE status=-1",[],|r|r.get(0))).unwrap_or(0);
    Ok(json!({"total":total,"pending":pending,"done":done,"failed":failed}))
}
#[tauri::command] pub async fn tag_list_registered() -> Result<Value, String> { Ok(json!({"types":["notes","files","products","projects"]})) }

#[tauri::command] pub async fn tag_list(category: Option<String>) -> Result<Vec<Value>, String> {
    if let Some(cat) = category.filter(|s| !s.is_empty()) {
        database::all("SELECT t.tag_id, t.name, t.color, t.type FROM tags t WHERE t.type=?1 ORDER BY t.name",&[json!(cat)]).map_err(|e| e.to_string())
    } else {
        database::all("SELECT t.tag_id, t.name, t.color, t.type FROM tags t ORDER BY t.type, t.name",&[]).map_err(|e| e.to_string())
    }
}

#[tauri::command] pub async fn tag_create(name: String, color: Option<String>, category: String) -> Result<Value, String> {
    let tag_id = uuid::Uuid::new_v4().to_string();
    let palette = ["#6366f1","#06b6d4","#10b981","#f59e0b","#ef4444","#8b5cf6","#ec4899","#14b8a6","#f97316","#84cc16"];
    let actual_color = color.filter(|c| !c.is_empty()).unwrap_or_else(|| {
        use rand::Rng;
        let idx = rand::thread_rng().gen_range(0..palette.len());
        palette[idx].to_string()
    });
    database::run("INSERT INTO tags(tag_id,name,color,type) VALUES(?1,?2,?3,?4)",&[json!(&tag_id), json!(&name), json!(&actual_color), json!(&category)]).map_err(|e| e.to_string())?;
    Ok(json!({"success":true,"tag_id":tag_id,"color":actual_color}))
}

#[tauri::command] pub async fn tag_update(tag_id: String, name: Option<String>, color: Option<String>, category: Option<String>) -> Result<Value, String> {
    let mut fields: Vec<String> = Vec::new(); let mut params: Vec<Value> = Vec::new();
    if let Some(n) = name { fields.push("name=?".into()); params.push(json!(n)); }
    if let Some(c) = color { fields.push("color=?".into()); params.push(json!(c)); }
    if let Some(c) = category { fields.push("type=?".into()); params.push(json!(c)); }
    if fields.is_empty() { return Ok(json!({"success":false,"error":"无更新字段"})); }
    params.push(json!(&tag_id));
    database::run(&format!("UPDATE tags SET {} WHERE tag_id=?", fields.join(",")), &params).map_err(|e| e.to_string())?;
    Ok(json!({"success":true}))
}

#[tauri::command] pub async fn tag_delete(tag_id: String) -> Result<Value, String> {
    database::run("DELETE FROM tag_mappings WHERE tag_id=?",&[json!(&tag_id)]).ok();
    database::run("DELETE FROM tags WHERE tag_id=?",&[json!(&tag_id)]).map_err(|e| e.to_string())?;
    Ok(json!({"success":true}))
}

#[tauri::command] pub async fn tag_list_categories() -> Result<Vec<Value>, String> {
    database::all("SELECT DISTINCT type FROM tags ORDER BY type",&[]).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn tag_stats() -> Result<Value, String> {
    // All tags with usage count
    let tags = database::all(
        "SELECT t.tag_id, t.name, t.color, t.type, \
           COALESCE((SELECT COUNT(*) FROM tag_mappings tm WHERE tm.tag_id=t.tag_id), 0) as usage_count \
         FROM tags t WHERE t.type != 'ai_shadow' \
         ORDER BY usage_count DESC, t.name", &[],
    ).unwrap_or_default();

    let total_tags = tags.len() as i64;
    let used_tags = tags.iter().filter(|t| t["usage_count"].as_i64().unwrap_or(0) > 0).count() as i64;
    let unused_tags = total_tags - used_tags;

    // By category
    let mut by_category: std::collections::HashMap<String, (i64, i64)> = std::collections::HashMap::new();
    for t in &tags {
        let cat = t["type"].as_str().unwrap_or("general").to_string();
        let entry = by_category.entry(cat).or_insert((0, 0));
        entry.0 += 1;
        entry.1 += t["usage_count"].as_i64().unwrap_or(0);
    }
    let category_stats: Vec<Value> = by_category.iter().map(|(k, v)| {
        json!({"label": k, "count": v.0, "total_usage": v.1})
    }).collect();

    // Top 10 by usage
    let top_tags: Vec<Value> = tags.iter().take(10).map(|t| {
        json!({
            "tag_id": t["tag_id"],
            "name": t["name"],
            "color": t["color"],
            "type": t["type"],
            "usage_count": t["usage_count"],
        })
    }).collect();

    // Coverage: objects with at least one tag / total objects
    let tagged_objects: i64 = database::with_db(|c| c.query_row(
        "SELECT COUNT(DISTINCT entity_type||':'||entity_id) FROM tag_mappings \
         WHERE tag_id IN (SELECT tag_id FROM tags WHERE type != 'ai_shadow')", [], |r| r.get(0)
    )).unwrap_or(0);
    let total_objects: i64 = database::with_db(|c| c.query_row(
        "SELECT COUNT(*) FROM source_data", [], |r| r.get(0)
    )).unwrap_or(0);

    Ok(json!({
        "success": true,
        "tags": tags,
        "total_tags": total_tags,
        "used_tags": used_tags,
        "unused_tags": unused_tags,
        "coverage": if total_objects > 0 { (tagged_objects as f64 / total_objects as f64 * 100.0).round() as i64 } else { 0 },
        "by_category": category_stats,
        "top_tags": top_tags,
    }))
}

#[tauri::command]
pub async fn ext_tool_get_project_context(query_name: String) -> Result<String, String> {
    if query_name.trim().is_empty() { return Ok("请提供项目名称".into()); }
    let projects = database::all("SELECT id,name,customer,customer_contact,status,owner,budget,bid_time,description,created_at,updated_at FROM projects WHERE name LIKE ?1 ORDER BY LENGTH(name) ASC LIMIT 1",&[json!(format!("%{}%",query_name.trim()))]).unwrap_or_default();
    let proj = match projects.first() { Some(p) => p, None => return Ok(format!("未找到匹配「{}」的项目", query_name)) };
    let project_id = proj["id"].as_str().unwrap_or("");
    let mut tags: Vec<String> = database::all("SELECT DISTINCT t.name FROM tags t INNER JOIN tag_mappings tm ON tm.tag_id=t.tag_id WHERE tm.entity_type='projects' AND tm.entity_id=?1",&[json!(project_id)]).unwrap_or_default().into_iter().filter_map(|r| r["name"].as_str().map(String::from)).collect();
    let note_tags: Vec<String> = database::all("SELECT DISTINCT t.name FROM tags t INNER JOIN tag_mappings tm ON tm.tag_id=t.tag_id INNER JOIN note_project_mapping npm ON npm.note_id=tm.entity_id WHERE tm.entity_type='notes' AND npm.project_id=?1",&[json!(project_id)]).unwrap_or_default().into_iter().filter_map(|r| r["name"].as_str().map(String::from)).collect();
    for nt in note_tags { if !tags.contains(&nt) { tags.push(nt); } }
    let bom: Vec<Value> = database::all("SELECT si.product_name,si.quantity,si.description,sv.version,s.name as solution_name FROM solution_items si JOIN solution_versions sv ON sv.id=si.solution_version_id JOIN solutions s ON s.id=sv.solution_id WHERE s.project_id=?1 AND sv.status='final' ORDER BY sv.updated_at DESC,si.sort_order ASC",&[json!(project_id)]).unwrap_or_default().into_iter().map(|r| json!({"product_model":r["product_name"].as_str().unwrap_or(""),"quantity":r["quantity"].as_i64().unwrap_or(0),"spec":r["description"].as_str().unwrap_or(""),"solution":r["solution_name"].as_str().unwrap_or("")})).collect();
    let result = json!({"project_id":project_id,"project_name":proj["name"].as_str().unwrap_or(""),"attributes":{"status":proj["status"].as_str().unwrap_or(""),"customer":proj["customer"].as_str().unwrap_or(""),"customer_contact":proj["customer_contact"].as_str().unwrap_or(""),"owner":proj["owner"].as_str().unwrap_or(""),"budget":proj["budget"].as_f64().unwrap_or(0.0),"bid_time":proj["bid_time"].as_str().unwrap_or(""),"description":proj["description"].as_str().unwrap_or("")},"tags":tags,"solution_bom":bom});
    Ok(serde_json::to_string_pretty(&result).unwrap_or_default())
}

// RAG 翻译官：标签/领域 -> file_id 数组
#[tauri::command]
pub async fn ext_tool_translate_filter(domain: String, tag_names: Vec<String>, project_ids: Vec<String>, product_ids: Vec<String>) -> Result<Value, String> {
    let ids = crate::services::rag_interceptor::translate_to_file_ids(&domain, &tag_names, &project_ids, &product_ids);
    Ok(json!({"success":true,"fileIds":ids,"count":ids.len()}))
}
// RAG 翻译官 + 向量检索组合
#[tauri::command]
pub async fn ext_tool_search_filtered(query: String, domain: String, tag_names: Vec<String>, project_ids: Vec<String>, product_ids: Vec<String>, limit: usize) -> Result<Value, String> {
    let file_ids = crate::services::rag_interceptor::translate_to_file_ids(&domain, &tag_names, &project_ids, &product_ids);
    if domain != "all" && file_ids.is_empty() { return Ok(json!({"success":true,"results":[],"fileIds":file_ids,"reason":"domain filter returned no files"})); }
    let fts_results = if file_ids.is_empty() { crate::database::fts5_search_content(&query, limit as i64).unwrap_or_default() }
    else { crate::database::fts5_search_content(&query, limit as i64 * 3).unwrap_or_default().into_iter().filter(|r| file_ids.contains(&r.entity_id)).take(limit).collect() };
    let query_vec = crate::services::embedding_client::get_embedding_with_caller(&query, "rag_search").await.unwrap_or_default();
    let vector_results: Vec<Value> = match crate::services::lancedb_client::get_lancedb() {
        Ok(g) => { if let Some(ref client) = *g { match client.search_vectors_with_file_ids(&query_vec, limit, &file_ids) { Ok(items) => items.into_iter().map(|(rid, sim, pid, did, _)| json!({"chunk_id":rid,"score":sim,"parent_id":pid,"doc_id":did})).collect(), Err(_) => vec![] } } else { vec![] } }
        Err(_) => vec![]
    };
    Ok(json!({"success":true,"fileIds":file_ids,"fts5Results":fts_results.into_iter().map(|r| json!({"source_type":r.entity_type,"source_id":r.entity_id,"title":r.title,"snippet":r.snippet,"rank":r.rank})).collect::<Vec<_>>(),"vectorResults":vector_results}))
}

// ═══════════════════════════════════════════════════════════════════
// Universal entity tag API — works with any entity_type via tag_mappings
// ═══════════════════════════════════════════════════════════════════

#[tauri::command]
pub async fn entity_get_tags(entity_type: String, entity_id: String) -> Result<Value, String> {
    let tags = database::all(
        "SELECT t.* FROM tags t INNER JOIN tag_mappings tm ON tm.tag_id=t.tag_id WHERE tm.entity_type=?1 AND tm.entity_id=?2 ORDER BY t.name",
        &[json!(entity_type), json!(entity_id)],
    ).map_err(|e| e.to_string())?;
    Ok(json!({"success":true,"tags":tags}))
}

#[tauri::command]
pub async fn entity_add_tag(entity_type: String, entity_id: String, tag_name: String) -> Result<Value, String> {
    let tag_id = match database::get("SELECT tag_id FROM tags WHERE name=?1", &[json!(&tag_name)]).ok().flatten() {
        Some(r) => r["tag_id"].as_str().unwrap_or("").to_string(),
        None => {
            let nid = uuid::Uuid::new_v4().to_string();
            database::run("INSERT INTO tags(tag_id,name,color,type) VALUES(?1,?2,?3,'general')", &[json!(&nid), json!(&tag_name), json!("#6366f1")]).map_err(|e| e.to_string())?;
            nid
        }
    };
    if tag_id.is_empty() { return Ok(json!({"success":false,"error":"标签ID无效"})); }
    database::run("INSERT OR IGNORE INTO tag_mappings(entity_type,entity_id,tag_id) VALUES(?1,?2,?3)", &[json!(&entity_type), json!(&entity_id), json!(&tag_id)]).map_err(|e| e.to_string())?;
    Ok(json!({"success":true}))
}

#[tauri::command]
pub async fn entity_remove_tag(entity_type: String, entity_id: String, tag_id: String) -> Result<Value, String> {
    database::run("DELETE FROM tag_mappings WHERE entity_type=?1 AND entity_id=?2 AND tag_id=?3", &[json!(&entity_type), json!(&entity_id), json!(&tag_id)]).map_err(|e| e.to_string())?;
    Ok(json!({"success":true}))
}

/// 调试：返回 ai_sync_queue 和 cleanup_queue 的原始数据
#[tauri::command]
pub async fn kb_pipeline_debug() -> Result<Value, String> {
    let sync = database::all("SELECT * FROM ai_sync_queue ORDER BY id DESC LIMIT 30", &[]).map_err(|e| e.to_string())?;
    let cleanup = database::all("SELECT * FROM cleanup_queue ORDER BY id DESC LIMIT 30", &[]).map_err(|e| e.to_string())?;
    let sync_ok = database::get("SELECT COUNT(*) as cnt FROM ai_sync_queue WHERE is_embedded=1", &[]).map_err(|e| e.to_string())?.and_then(|r| r["cnt"].as_i64()).unwrap_or(0);
    let sync_pending = database::get("SELECT COUNT(*) as cnt FROM ai_sync_queue WHERE is_embedded<=0", &[]).map_err(|e| e.to_string())?.and_then(|r| r["cnt"].as_i64()).unwrap_or(0);
    let sync_fail = database::get("SELECT COUNT(*) as cnt FROM ai_sync_queue WHERE is_embedded=-1", &[]).map_err(|e| e.to_string())?.and_then(|r| r["cnt"].as_i64()).unwrap_or(0);
    let cleanup_pending = database::get("SELECT COUNT(*) as cnt FROM cleanup_queue WHERE status=0", &[]).map_err(|e| e.to_string())?.and_then(|r| r["cnt"].as_i64()).unwrap_or(0);
    let cleanup_done = database::get("SELECT COUNT(*) as cnt FROM cleanup_queue WHERE status=1", &[]).map_err(|e| e.to_string())?.and_then(|r| r["cnt"].as_i64()).unwrap_or(0);
    Ok(json!({
        "success":true,
        "stats": {"sync_ok":sync_ok,"sync_pending":sync_pending,"sync_fail":sync_fail,"cleanup_pending":cleanup_pending,"cleanup_done":cleanup_done},
        "sync_queue": sync,
        "cleanup_queue": cleanup
    }))
}
