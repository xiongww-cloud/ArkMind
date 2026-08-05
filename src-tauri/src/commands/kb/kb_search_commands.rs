use crate::database;
use serde_json::{json, Value};

#[tauri::command] pub async fn kb_search(data: Value) -> Result<Value, String> {
    let query = data["query"].as_str().unwrap_or("");
    let limit = data["limit"].as_i64().unwrap_or(10);
    let results = database::fts5_search_chunks(query, limit).map_err(|e| e.to_string())?;
    Ok(json!({"success":true,"results":results}))
}
#[tauri::command] pub async fn kb_delete(doc_id: String) -> Result<Value, String> {
    let parts: Vec<&str> = doc_id.splitn(2, '-').collect();
    if parts.len() == 2 {
        let st = parts[0].to_uppercase();
        // M3: 使用 notify_deleted 走完整清理链路（source_data + chunks + FTS + LanceDB）
        crate::services::knowledge_lifecycle::notify_deleted(&st, parts[1]);
    }
    Ok(json!({"success":true}))
}
#[tauri::command] pub async fn kb_list_documents(pid: Option<String>) -> Result<Value, String> {
    let docs = if let Some(ref p) = pid.filter(|s| !s.is_empty()) {
        database::all("SELECT fa.id, fa.file_ext, fa.file_size, fa.content_hash, fa.created_at, fa.updated_at, fr.custom_name as global_name, fr.custom_name as custom_name FROM file_assets fa INNER JOIN file_relations fr ON fr.file_id=fa.id AND fr.entity_type='project' WHERE fr.entity_id=? ORDER BY fa.updated_at DESC",&[json!(p)]).unwrap_or_default()
    } else {
        database::all("SELECT fa.id, fa.file_ext, fa.file_size, fa.content_hash, fa.created_at, fa.updated_at FROM file_assets fa ORDER BY fa.updated_at DESC LIMIT 200",&[]).unwrap_or_default()
    };
    Ok(json!({"success":true,"documents":docs}))
}
#[tauri::command] pub async fn kb_import_file(data: Value) -> Result<Value, String> {
    let path = data["filePath"].as_str().unwrap_or("").to_string();
    let name = data["fileName"].as_str().unwrap_or("未命名文件").to_string();
    let _ext = data["fileExt"].as_str().unwrap_or("").to_string();
    let project_id = data["projectId"].as_str().map(|s| s.to_string());
    if path.is_empty() || !std::path::Path::new(&path).exists() { return Ok(json!({"success":false,"error":"文件不存在"})); }
    // upload_file_core 内部已调用 notify_file_upload，文件进入推送管线处理
    let (file_id, object_id) = crate::commands::file_commands::upload_file_core(&path, "library", &[], &name, "", "KB导入", None).await.map_err(|e| e.to_string())?;
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
        database::all("SELECT kc.rowid,kc.parent_id,kc.content,kc.chunk_index,kc.created_at FROM knowledge_chunks kc WHERE kc.chunk_type='child' AND kc.content LIKE ?1 ESCAPE '\\' ORDER BY kc.created_at DESC LIMIT 50 OFFSET ?2",&[json!(format!("%{}%",crate::database::escape_like(&q))), json!(offset)]).unwrap_or_default()
    };
    let total: i64 = database::with_db(|c| c.query_row("SELECT COUNT(*) FROM knowledge_chunks WHERE chunk_type='child'",[],|r|r.get(0))).unwrap_or(0);
    Ok(json!({"success":true,"rows":rows,"total":total,"page":p}))
}
#[tauri::command] pub async fn kb_browse_parents(query: Option<String>, page: Option<i64>) -> Result<Value, String> {
    let q = query.unwrap_or_default(); let p = page.unwrap_or(1).max(1); let offset = (p - 1) * 50;
    let rows = if q.is_empty() {
        database::all("SELECT kc.rowid,kc.content,kc.doc_title,kc.chunk_index,kc.source_data_id,kc.created_at,sd.source_type FROM knowledge_chunks kc LEFT JOIN source_data sd ON sd.id=kc.source_data_id WHERE kc.chunk_type='parent' ORDER BY kc.created_at DESC LIMIT 50 OFFSET ?",&[json!(offset)]).unwrap_or_default()
    } else {
        database::all("SELECT kc.rowid,kc.content,kc.doc_title,kc.chunk_index,kc.source_data_id,kc.created_at,sd.source_type FROM knowledge_chunks kc LEFT JOIN source_data sd ON sd.id=kc.source_data_id WHERE kc.chunk_type='parent' AND kc.content LIKE ?1 ESCAPE '\\' ORDER BY kc.created_at DESC LIMIT 50 OFFSET ?2",&[json!(format!("%{}%",crate::database::escape_like(&q))), json!(offset)]).unwrap_or_default()
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
pub async fn ext_tool_search_knowledge_base(keywords: Vec<String>, project_id: Option<String>, _filter_tags: Option<Vec<String>>) -> Result<String, String> {
    let query = keywords.join(" ");
    if query.trim().is_empty() { return Ok("未提供检索关键词".into()); }
    let query_vector = crate::services::embedding_client::get_embedding_with_caller(&query, "production_rag").await.map_err(|e| format!("embed: {}", e))?;
    let vec_rowids: Vec<(String, f64)> = match crate::services::lancedb_client::get_lancedb() {
        Ok(guard) => match guard.as_ref() { Some(client) => client.search_vectors_with_parent(&query_vector, 30).unwrap_or_default().into_iter().map(|(r,s,_,_,_)| (r.to_string(), s)).collect(), None => Vec::new() }, Err(_) => Vec::new()
    };
    let fts_query: Vec<String> = keywords.iter().map(|k| format!("\"{}\"", k.replace('"', ""))).collect();
    let fts_query_str = fts_query.join(" OR ");
    let fts_results = database::fts5_search_chunks(&fts_query_str, 30).unwrap_or_default();
    let fts_rowids: Vec<(String, f64)> = fts_results.iter().map(|fr| (fr.rowid.to_string(), fr.rank)).collect();
    let fused = crate::services::search_assets::rrf_fusion(&fts_rowids, &vec_rowids, 60.0);
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
                let source_id_str = format!("{}-{}", st, sid);
                let meta_json = crate::services::agent_workflow::build_chunk_metadata(&source_id_str);
                let meta = if meta_json["found"].as_bool().unwrap_or(false) {
                    format!("来源：{} · {}", 
                        match st { "FILE" => "文件", "PRODUCT" => "产品", "PROJECT" => "项目", "NOTE" => "笔记", "TODO" => "待办", _ => st },
                        meta_json["name"].as_str().unwrap_or(sid)
                    )
                } else {
                    String::new()
                };
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

#[tauri::command]
pub async fn ext_tool_get_project_context(query_name: String) -> Result<String, String> {
    if query_name.trim().is_empty() { return Ok("请提供项目名称".into()); }
    let projects = database::all("SELECT id,name,status,owner,budget,bid_time,description,created_at,updated_at FROM projects WHERE name LIKE ?1 ESCAPE '\\' ORDER BY LENGTH(name) ASC LIMIT 1",&[json!(format!("%{}%",crate::database::escape_like(query_name.trim())))]).unwrap_or_default();
    let proj = match projects.first() { Some(p) => p, None => return Ok(format!("未找到匹配「{}」的项目", query_name)) };
    let project_id = proj["id"].as_str().unwrap_or("");
    let bom: Vec<Value> = database::all("SELECT si.product_name,si.quantity,si.description,sv.version,s.name as solution_name FROM solution_items si JOIN solution_versions sv ON sv.id=si.solution_version_id JOIN solutions s ON s.id=sv.solution_id WHERE s.project_id=?1 AND sv.status='final' ORDER BY sv.updated_at DESC,si.sort_order ASC",&[json!(project_id)]).unwrap_or_default().into_iter().map(|r| json!({"product_model":r["product_name"].as_str().unwrap_or(""),"quantity":r["quantity"].as_i64().unwrap_or(0),"spec":r["description"].as_str().unwrap_or(""),"solution":r["solution_name"].as_str().unwrap_or("")})).collect();
    let result = json!({"project_id":project_id,"project_name":proj["name"].as_str().unwrap_or(""),"attributes":{"status":proj["status"].as_str().unwrap_or(""),"owner":proj["owner"].as_str().unwrap_or(""),"budget":proj["budget"].as_f64().unwrap_or(0.0),"bid_time":proj["bid_time"].as_str().unwrap_or(""),"description":proj["description"].as_str().unwrap_or("")},"solution_bom":bom});
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
