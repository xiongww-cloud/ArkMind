//! ext_tool_search：知识库双路检索（FTS5 + 向量）与批量搜索

use crate::database;
use serde_json::{json, Value};
use crate::services::agent_workflow::*;

// ═══════════════════════════════════════════════════════════════════
// Tool executors
// ═══════════════════════════════════════════════════════════════════

/// 将 "TYPE-id" 格式的 scope 列表解析为 child rowid 集合（用于限定搜索范围）
/// 优化版：直接通过 child chunk 的 source_data_id 查询，跳过 parent 中转
pub(crate) fn resolve_scope_to_rowids(scope: &[String]) -> std::collections::HashSet<i64> {
    let mut all_rowids = std::collections::HashSet::new();
    for s in scope {
        let parts: Vec<&str> = s.splitn(2, '-').collect();
        if parts.len() != 2 { log::info!("[Scope] 无法解析 scope key: {}", s); continue; }
        let st = parts[0].to_uppercase();
        let sid = parts[1];
        // source_data.id 是 TEXT 类型（= source_id），不能用 as_i64()
        match database::get("SELECT id FROM source_data WHERE source_type=?1 AND source_id=?2",&[json!(st), json!(sid)]) {
            Ok(Some(row)) => {
                let sdid_str = row["id"].as_str().unwrap_or("");
                if sdid_str.is_empty() { log::info!("[Scope] ✗ {} source_data.id 为空", s); continue; }
                match database::all("SELECT id FROM knowledge_chunks WHERE chunk_type='child' AND source_data_id=?1",&[json!(sdid_str)]) {
                    Ok(rows) => {
                        let count = rows.len();
                        for cr in &rows { if let Some(cid) = cr["id"].as_i64() { all_rowids.insert(cid); } }
                        log::info!("[Scope] ✓ {} → source_data_id={}, child_chunks={}", s, sdid_str, count);
                    }
                    Err(e) => log::info!("[Scope] ✗ {} 查询 child_chunks 失败: {}", s, e),
                }
            }
            Ok(None) => log::info!("[Scope] ✗ {} 在 source_data 中未找到 (type={}, sid={})", s, st, sid),
            Err(e) => log::info!("[Scope] ✗ {} 查询 source_data 失败: {}", s, e),
        }
    }
    all_rowids
}

/// 执行 search 并同时返回发现的来源列表
pub(crate) async fn execute_search_knowledge(keywords: &[String], scope_rowids: Option<std::collections::HashSet<i64>>, _visited_sources: &std::collections::HashSet<String>, visited_chunks: &std::collections::HashSet<String>, time_range: Option<(String, String)>, _shown_memory_ids: &std::collections::HashSet<String>) -> (String, Vec<String>) {
    if keywords.is_empty() || keywords.join(" ").trim().is_empty() { return ("no keywords".into(), vec![]); }

    // ★ 停用词过滤：避免"项目""产品"等类型词污染 FTS5/Vector 搜索（对象名精确匹配也使用过滤后关键词）
    let filtered_keywords: Vec<String> = {
        let stopwords: std::collections::HashSet<&str> = [
            "项目", "产品", "文件", "客户", "渠道", "厂商", "供应商", "联系人", "笔记", "待办", "任务",
            "project", "product", "file", "customer", "channel", "vendor", "contact", "note", "todo",
            "介绍", "详情", "什么", "查询", "检索", "搜索", "一下", "查一下", "的", "是", "和", "或",
        ].iter().cloned().collect();
        let f: Vec<String> = keywords.iter().filter(|k| !stopwords.contains(k.as_str().to_lowercase().as_str())).cloned().collect();
        if f.is_empty() { keywords.to_vec() } else { f }
    };
    let keywords = filtered_keywords.as_slice();
    let query = keywords.join(" ");

    let mut obj_head = String::new();
    let mut matched_sources: std::collections::HashSet<(String,String)> = std::collections::HashSet::new();
    if !keywords.is_empty() {
        let like = format!("%{}%", crate::database::escape_like(&keywords.join(" ")));
        if let Ok(rows) = database::all("SELECT id,name,status FROM projects WHERE name LIKE ?1 ESCAPE '\\' ORDER BY LENGTH(name) ASC LIMIT 3",&[json!(&like)]) {
            for r in &rows { let id = r["id"].as_str().unwrap_or("").to_string(); matched_sources.insert(("PROJECT".into(), id.clone())); obj_head.push_str(&format!("[Object] Project: {} | Status: {} | source_id: PROJECT-{}\n",r["name"].as_str().unwrap_or(""),r["status"].as_str().unwrap_or(""),id)); }
        }
        if let Ok(rows) = database::all("SELECT id,name FROM products WHERE name LIKE ?1 ESCAPE '\\' ORDER BY LENGTH(name) ASC LIMIT 3",&[json!(&like)]) {
            for r in &rows { let id = r["id"].as_str().unwrap_or("").to_string(); matched_sources.insert(("PRODUCT".into(), id.clone())); obj_head.push_str(&format!("[Object] Product: {} | source_id: PRODUCT-{}\n",r["name"].as_str().unwrap_or(""),id)); }
        }
        if let Ok(rows) = database::all("SELECT source_id, title FROM source_data WHERE source_type='FILE' AND title LIKE ?1 ESCAPE '\\' ORDER BY LENGTH(title) ASC LIMIT 3",&[json!(&like)]) {
            for r in &rows { let id = r["source_id"].as_str().unwrap_or("").to_string(); matched_sources.insert(("FILE".into(), id.clone())); obj_head.push_str(&format!("[Object] File: {} | source_id: FILE-{}\n",r["title"].as_str().unwrap_or(""),id)); }
        }
        if let Ok(rows) = database::all("SELECT note_id,title FROM notes WHERE title LIKE ?1 ESCAPE '\\' ORDER BY LENGTH(title) ASC LIMIT 3",&[json!(&like)]) {
            for r in &rows { let id = r["note_id"].as_str().unwrap_or("").to_string(); obj_head.push_str(&format!("[Object] Note: {} | source_id: NOTE-{}\n",r["title"].as_str().unwrap_or(""),id)); }
        }
        if let Ok(rows) = database::all("SELECT id,title,priority,due_date FROM todos WHERE title LIKE ?1 ESCAPE '\\' ORDER BY LENGTH(title) ASC LIMIT 3",&[json!(&like)]) {
            for r in &rows { let id = r["id"].as_str().unwrap_or("").to_string(); let pri = r["priority"].as_str().unwrap_or(""); let due = r["due_date"].as_str().unwrap_or(""); let extra = if !pri.is_empty() { format!(" | 优先级: {}", pri) } else { String::new() } + &if !due.is_empty() { format!(" | 截止: {}", due) } else { String::new() }; obj_head.push_str(&format!("[Object] Todo: {}{} | source_id: TODO-{}\n",r["title"].as_str().unwrap_or(""),extra,id)); }
        }
        if let Ok(rows) = database::all("SELECT id,name,phone,position FROM partner_contacts WHERE name LIKE ?1 ESCAPE '\\' ORDER BY LENGTH(name) ASC LIMIT 3",&[json!(&like)]) {
            for r in &rows { let id = r["id"].as_str().unwrap_or("").to_string(); let pos = r["position"].as_str().unwrap_or(""); let extra = if !pos.is_empty() { format!(" | {}", pos) } else { String::new() }; obj_head.push_str(&format!("[Object] Contact: {}{} | source_id: CONTACT-{}\n",r["name"].as_str().unwrap_or(""),extra,id)); }
        }
    }
    let _ = if !obj_head.is_empty() { format!("--- Related Objects ---\n{}--- Chunks ---\n", obj_head) } else { String::new() };

    // ★ 始终走双路（FTS5 + Vector），不再因 scope 而跳过向量检索
    // ★ 始终走双路（FTS5 + Vector），不再因 scope 而跳过向量检索

    let vec_rowids: Vec<(String,f64)> = {
        match crate::services::embedding_client::get_embedding_with_caller(&query, "agent_workflow").await {
            Ok(qv) => {
                match crate::services::lancedb_client::get_lancedb() {
                    Ok(g) => match g.as_ref() {
                        Some(c) => {
                            let raw = c.search_vectors_with_parent(&qv, 30).unwrap_or_default();
                            raw.into_iter().map(|(r,s,_,_,_)|(r.to_string(),s)).collect()
                        }
                        None => vec![]
                    }
                    Err(_) => vec![]
                }
            }
            Err(_e) => vec![]
        }
    };
    let fts_limit = 30;
    let fts_q = keywords.iter().map(|k| format!("\"{}\"", k.replace('"',""))).collect::<Vec<_>>().join(" OR ");
    let fts_rowids: Vec<(String,f64)> = database::fts5_search_chunks(&fts_q, fts_limit).unwrap_or_default().iter()
        .map(|fr| (fr.rowid.to_string(), fr.rank)).collect();
    log::info!("Search keywords={:?}, scope={}", keywords, scope_rowids.as_ref().map(|s| format!("Some({} rowids)", s.len())).unwrap_or_else(|| "None".to_string()));
    log::info!("Search FTS5 hits={}, Vector hits={}", fts_rowids.len(), vec_rowids.len());

    // 标签预过滤已随标签系统移除，融合直接进 RRF
    let fused = search_assets::rrf_fusion(&fts_rowids, &vec_rowids, 60.0);

    if fused.is_empty() {
        let kw_display = keywords.join("、");
        let diag = format!("未找到关键词「{}」的匹配切片。\n\n💡 已在全库范围搜索，无匹配结果。建议：\n1. 使用更简短或不同的关键词重试（如去掉型号只留品牌）\n2. 使用 object 工具先查看知识库中的文件与产品范围\n3. 确认相关文件已导入知识库", kw_display);
        return (diag, vec![]);
    }
    let mut seen_hashes = std::collections::HashSet::new();
    let mut disc_sources: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut found_sources: Vec<String> = Vec::new();
    let mut candidates: Vec<(f64, i64, String, String, String, String, String, i64, i64, bool, bool)> = Vec::new();

    for (rowid_str, rrf) in fused.iter() {
        let Ok(cr) = rowid_str.parse::<i64>() else { continue; };
        if let Ok(Some(r)) = database::get("SELECT p.content,p.content_hash,p.doc_title,p.chunk_index,p.chunk_id,p.source_data_id as parent_sdid,sd.source_type,sd.source_id,sd.id as source_data_id FROM knowledge_chunks c JOIN knowledge_chunks p ON p.chunk_id=c.parent_id LEFT JOIN source_data sd ON sd.id=p.source_data_id WHERE c.rowid=?1",&[json!(cr)]) {
            let st = r["source_type"].as_str().unwrap_or("").to_string();
            let sid = r["source_id"].as_str().unwrap_or("").to_string();
            let dedup = format!("{}-{}", st, sid);
            let h = r["content_hash"].as_str().unwrap_or("").to_string();
            let content = r["content"].as_str().unwrap_or("").to_string();
            if content.trim().chars().count() < 60 {
                continue;
            }
            let chunk_id = r["chunk_id"].as_str().unwrap_or("").to_string();
            let chunk_index = r["chunk_index"].as_i64().unwrap_or(0);
            let sdid = r["source_data_id"].as_str().unwrap_or("").to_string();
            let is_in_scope = scope_rowids.as_ref().map(|sids| sids.contains(&cr)).unwrap_or(false);

            let base_rrf = if matched_sources.contains(&(st.clone(), sid.clone())) { rrf * 1.5 } else { *rrf };
            let syn_mult = crate::services::synapse::rrf_weight_multiplier(cr);
            let effective_rrf = base_rrf * syn_mult;
            let (score, is_scope_path) = if is_in_scope {
                (effective_rrf, true)
            } else {
                // discovery path: 每个 source 每轮最多展示 2 个 chunk
                let count = disc_sources.entry(dedup.clone()).or_insert(0);
                *count += 1;
                if *count > 2 {
                    continue;
                }
                (effective_rrf * 0.3, false)
            };
            if !h.is_empty() && !seen_hashes.insert(h.clone()) {
                continue;
            }
            // ★ 时间过滤
            if let Some((ref start, ref end)) = time_range {
                let created_at: String = database::get("SELECT created_at FROM knowledge_chunks WHERE id=?1",&[json!(cr)]).ok().flatten().and_then(|r| r["created_at"].as_str().map(String::from)).unwrap_or_default();
                if !created_at.is_empty() {
                    let date = &created_at[..10.min(created_at.len())];  // "2026-06-25"
                    if date < start.as_str() || date > end.as_str() { continue; }
                }
            }
            // ★ 已在前轮展示过的 chunk 直接丢弃，让排名更靠后的新 chunk 补位
            let short_chunk: String = chunk_id.chars().take(8).collect();
            if !short_chunk.is_empty() && visited_chunks.contains(&short_chunk) { continue; }

            let total_parents: i64 = database::get("SELECT COUNT(*) as cnt FROM knowledge_chunks WHERE source_data_id=?1 AND chunk_type='parent'",&[json!(sdid)]).ok().flatten().and_then(|r| r["cnt"].as_i64()).unwrap_or(0);
            candidates.push((score, cr, st.clone(), sid.clone(), chunk_id, content, h, chunk_index, total_parents, is_scope_path, matched_sources.contains(&(st, sid))));
        }
    }

    candidates.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    log::info!("[Search] RRF fusion={}, candidates={}, scope_in={}, discovery={}", fused.len(), candidates.len(),
        candidates.iter().filter(|c| c.9).count(), candidates.iter().filter(|c| !c.9).count());

    // ★ 双路结果：非MEMORY top 3
    let non_memory: Vec<_> = candidates.iter().filter(|(_, _, st, _, _, _, _, _, _, _, _)| st != "MEMORY").take(3).collect();

    // ── 知识检索区 ──
    let mut search_out = String::new();
    if !non_memory.is_empty() {
        search_out.push_str("## 📄 知识检索");
        for (idx, (score, cr, st, sid, chunk_id, content, _h, chunk_index, total_parents, is_scope_path, is_obj_match)) in non_memory.iter().enumerate() {
            let rrf_tag = &format!("{:.3}", score);
            let boost_tag = if *is_obj_match { "🔥" } else { "" };
            let source_id_str = format!("{}-{}", st, sid);
            let meta_json = build_chunk_metadata(&source_id_str);
            let mut meta = if meta_json["found"].as_bool().unwrap_or(false) {
                format!("来源：{} · {}", 
                    match st.as_str() { "FILE" => "文件", "PRODUCT" => "产品", "PROJECT" => "项目", "NOTE" => "笔记", "TODO" => "待办", _ => st },
                    meta_json["name"].as_str().unwrap_or(sid)
                )
            } else {
                String::new()
            };
            if *total_parents > 1 {
                if !meta.is_empty(){meta.push('\n');}
                meta.push_str(&format!("父块：{}/{}", chunk_index + 1, total_parents));
            }
            if !chunk_id.is_empty() {
                let short: String = chunk_id.chars().take(8).collect();
                if !meta.is_empty(){meta.push('\n');}
                meta.push_str(&format!("ChunkID：{}", short));
            }
            if *is_scope_path {
                meta.push_str("\n📍 该段落属于当前工作区");
            } else {
                meta.push_str("\n💡 该段落来自全库发现（不属于当前工作区）");
            }
            let _source_label = match st.as_str() {
                "FILE" => "FILE",
                "PRODUCT" => "PRODUCT",
                "PROJECT" => "PROJECT",
                "NOTE" => "NOTE",
                "TODO" => "TODO",
                _ => st,
            };
            let created_at: String = database::get(
                "SELECT created_at FROM knowledge_chunks WHERE id=?1", &[json!(cr)],
            ).ok().flatten().and_then(|r| r["created_at"].as_str().map(String::from)).unwrap_or_default();
            let time_info = relative_time(&created_at);
            if !time_info.is_empty() {
                if !meta.is_empty() { meta.push('\n'); }
                meta.push_str(&time_info);
            }
            let preview = content.clone();
            if meta.is_empty() { search_out.push_str(&format!("\n\n--- [父块{}|RRF={}]{} ---\n{}", idx + 1, rrf_tag, boost_tag, preview)); }
            else { search_out.push_str(&format!("\n\n--- [父块{}|RRF={}]{} ---\n{}\n--------\n{}", idx + 1, rrf_tag, boost_tag, meta, preview)); }

            found_sources.push(format!("{}-{}", st, sid));

            // ★ 关联对象自动进入 workspace（统一从 business_relations 查询）
            match st.as_str() {
                "FILE" => {
                    // 文件关联的实体：双向查询
                    for rel_type in &["PROJECT", "PRODUCT", "TODO", "NOTE"] {
                        // 正向：FILE 是 source
                        if let Ok(rows) = database::all(
                            "SELECT target_id FROM business_relations WHERE source_type='FILE' AND source_id=?1 AND target_type=?2",
                            &[json!(sid.as_str()), json!(rel_type)],
                        ) {
                            for r in &rows {
                                if let Some(eid) = r["target_id"].as_str().filter(|s| !s.is_empty()) {
                                    let dedup = format!("{}-{}", rel_type, eid);
                                    if !found_sources.iter().any(|s| s == &dedup) { found_sources.push(dedup); }
                                }
                            }
                        }
                        // 反向：FILE 是 target
                        if let Ok(rows) = database::all(
                            "SELECT source_id FROM business_relations WHERE target_type='FILE' AND target_id=?1 AND source_type=?2",
                            &[json!(sid.as_str()), json!(rel_type)],
                        ) {
                            for r in &rows {
                                if let Some(eid) = r["source_id"].as_str().filter(|s| !s.is_empty()) {
                                    let dedup = format!("{}-{}", rel_type, eid);
                                    if !found_sources.iter().any(|s| s == &dedup) { found_sources.push(dedup); }
                                }
                            }
                        }
                    }
                }
                "PROJECT" | "PRODUCT" | "NOTE" | "TODO" => {
                    // 实体关联的文件：双向查询
                    // 正向：实体是 source，FILE 是 target
                    if let Ok(rows) = database::all(
                        "SELECT target_id FROM business_relations WHERE source_type=?1 AND source_id=?2 AND target_type='FILE'",
                        &[json!(st), json!(sid.as_str())],
                    ) {
                        for r in &rows {
                            if let Some(fid) = r["target_id"].as_str().filter(|s| !s.is_empty()) {
                                let dedup = format!("FILE-{}", fid);
                                if !found_sources.iter().any(|s| s == &dedup) { found_sources.push(dedup); }
                            }
                        }
                    }
                    // 反向：实体是 target，FILE 是 source
                    if let Ok(rows) = database::all(
                        "SELECT source_id FROM business_relations WHERE target_type=?1 AND target_id=?2 AND source_type='FILE'",
                        &[json!(st), json!(sid.as_str())],
                    ) {
                        for r in &rows {
                            if let Some(fid) = r["source_id"].as_str().filter(|s| !s.is_empty()) {
                                let dedup = format!("FILE-{}", fid);
                                if !found_sources.iter().any(|s| s == &dedup) { found_sources.push(dedup); }
                            }
                        }
                    }
                }
                _ => {}
            }

            // 父块被采纳 → 激活其下所有子块
            if !chunk_id.is_empty() {
                if let Ok(child_rows) = database::all("SELECT id FROM knowledge_chunks WHERE chunk_type='child' AND parent_id=?1",&[json!(chunk_id)]) {
                    for cr in &child_rows { if let Some(cid) = cr["id"].as_i64() { crate::services::synapse::activate_child_chunk(cid); } }
                }
            }
        }
    }

    // 构建 chunks 数组
    let chunks: Vec<Value> = non_memory.iter().map(|(score, cr, st, sid, chunk_id, content, _h, chunk_index, total_parents, is_scope_path, is_obj_match)| {
        let short_chunk_id: String = chunk_id.chars().take(8).collect();
        let preview = content.clone();
        // 查询时间信息
        let created_at: String = database::get(
            "SELECT created_at FROM knowledge_chunks WHERE id=?1",
            &[json!(cr)],
        ).ok().flatten().and_then(|r| r["created_at"].as_str().map(String::from)).unwrap_or_default();
        json!({
            "chunk_id": short_chunk_id,
            "rrf_score": score,
            "source": {
                "type": st,
                "id": sid
            },
            "chunk_index": chunk_index,
            "total_parents": total_parents,
            "is_in_workspace": is_scope_path,
            "is_boosted": is_obj_match,
            "content": preview,
            "created_at": created_at
        })
    }).collect();
    
    // 构建 related_objects 数组（带名称）
    let related_objects: Vec<Value> = matched_sources.iter().map(|(st, sid)| {
        let name = database::get(
            "SELECT title FROM source_data WHERE source_type=?1 AND source_id=?2",
            &[json!(st), json!(sid)],
        ).ok().flatten().and_then(|r| r["title"].as_str().map(String::from)).unwrap_or_default();
        json!({
            "type": st,
            "id": sid,
            "name": name
        })
    }).collect();
    
    let total_hits = chunks.len();
    let has_multi_parent = chunks.iter().any(|c| c["total_parents"].as_i64().unwrap_or(1) > 1);
    let summary = if total_hits == 0 {
        let kw_display = keywords.join("、");
        format!("未找到关键词「{}」的匹配切片。", kw_display)
    } else if has_multi_parent {
        format!("找到 {} 条相关知识片段。部分文档包含多个片段，如需继续阅读某篇文档的更多内容，请使用 chunk_navigate 工具 + 对应的 ChunkID。", total_hits)
    } else {
        format!("找到 {} 条相关知识片段。", total_hits)
    };
    
    let envelope = ToolResultEnvelope {
        result_type: "knowledge_search_result".to_string(),
        data: json!({
            "query": query,
            "total_hits": total_hits,
            "related_objects": related_objects,
            "chunks": chunks
        }),
        summary: Some(summary),
    };
    
    (envelope.to_json(), found_sources)
}



/// ★ 批量并发检索：同时检索多个独立子查询，每个子查询返回最多 2 个结果
pub(crate) async fn execute_batch_search(queries: &[Vec<String>], scope_rowids: Option<std::collections::HashSet<i64>>, visited_sources: &std::collections::HashSet<String>) -> String {
    if queries.len() < 3 {
        return format!("只有 {} 个子查询。batch_search 需要至少 3 个子查询。请使用 ext_tool_search 获取更精准的结果。", queries.len());
    }
    // 并发执行每个子查询的 FTS5 + 向量双路检索
    let mut handles = Vec::new();
    for kw in queries.iter().take(10) {
        let kw = kw.clone();
        let _sids = scope_rowids.clone();
        let _visited = visited_sources.clone();
        handles.push(tokio::spawn(async move {
            let query = kw.join(" ");
            // FTS5 路径
            let fts_q = kw.iter().map(|k| format!("\"{}\"", k.replace('"',""))).collect::<Vec<_>>().join(" OR ");
            let fts_rowids: Vec<(String,f64)> = database::fts5_search_chunks(&fts_q, 30).unwrap_or_default().iter()
                .map(|fr| (fr.rowid.to_string(), fr.rank)).collect();
            // 向量路径
            let vec_rowids: Vec<(String,f64)> = match crate::services::embedding_client::get_embedding_with_caller(&query, "batch_search").await {
                Ok(qv) => match crate::services::lancedb_client::get_lancedb() {
                    Ok(g) => match g.as_ref() {
                        Some(c) => c.search_vectors_with_parent(&qv, 30).unwrap_or_default().into_iter()
                            .map(|(r,s,_,_,_)|(r.to_string(),s)).collect(),
                        None => vec![],
                    },
                    Err(_) => vec![],
                },
                Err(_) => vec![],
            };
            // RRF 融合，每个子查询取 top 2
            let fuse = |rows: Vec<(String,f64)>| rows;
            let fused = crate::services::search_assets::rrf_fusion(&fuse(fts_rowids), &fuse(vec_rowids), 60.0);
            let mut items: Vec<(String, f64, String, String, String)> = Vec::new(); // (chunk_id, rrf, source_type, source_id, source_id_display)
            let mut seen_local = std::collections::HashSet::new();
            for (rowid_str, rrf) in fused.iter().take(2) {
                if let Ok(cr) = rowid_str.parse::<i64>() {
                    if let Ok(Some(r)) = database::get("SELECT p.content,p.chunk_id,sd.source_type,sd.source_id FROM knowledge_chunks c JOIN knowledge_chunks p ON p.chunk_id=c.parent_id LEFT JOIN source_data sd ON sd.id=p.source_data_id WHERE c.rowid=?1",&[json!(cr)]) {
                        let chunk_id = r["chunk_id"].as_str().unwrap_or("").to_string();
                        if chunk_id.is_empty() || !seen_local.insert(chunk_id.clone()) { continue; }
                        let st = r["source_type"].as_str().unwrap_or("").to_string();
                        let sid = r["source_id"].as_str().unwrap_or("").to_string();
                        items.push((chunk_id, *rrf, st.clone(), sid.clone(), format!("{}-{}", st, sid)));
                    }
                }
            }
            (kw, items)
        }));
    }
    // 收集结果
    let mut all_results: Vec<(Vec<String>, Vec<(String, f64, String, String, String)>)> = Vec::new();
    for h in handles {
        if let Ok((kw, items)) = h.await { all_results.push((kw, items)); }
    }

    // ★ 跨子查询仅按 chunk_id 去重（不按 source 去重，同一个文件不同 chunk 各自保留）
    let mut seen_chunks = std::collections::HashSet::new();
    let mut out = String::from("## 批量检索结果\n\n");
    let mut total_hits = 0usize;
    for (kw, items) in &all_results {
        let kw_str = kw.join("、");
        let mut hits = 0usize;
        for (chunk_id, rrf, _st, _sid, source_display) in items {
            if !seen_chunks.insert(chunk_id.clone()) { continue; }
            hits += 1;
            total_hits += 1;
            if let Ok(Some(r)) = database::get("SELECT p.content FROM knowledge_chunks c JOIN knowledge_chunks p ON p.chunk_id=c.parent_id WHERE c.chunk_id=?1",&[json!(chunk_id)]) {
                if let Some(content) = r["content"].as_str() {
                    out.push_str(&format!("**[{}] RRF={:.3} | source_id: {}**\n> {}\n\n", kw_str, rrf, source_display, content.replace('\n', "\n> ")));
                }
            }
        }
        if hits == 0 {
            out.push_str(&format!("**[{}]** ❌ 未找到匹配内容\n\n", kw_str));
        }
    }
    out.push_str(&format!("\n---\n共 {} 个检索结果（跨子查询按 chunk_id 去重）", total_hits));
    out
}

