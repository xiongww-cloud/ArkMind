use crate::database; use crate::services::embedding_client; use crate::services::lancedb_client; use serde_json::json;

/// 获取待向量化的子块（按 id 升序，确定性排序，毒丸批次稳定重试）
fn get_pending_chunks(batch_size: usize) -> Vec<(i64, String, String)> {
    database::all(
        "SELECT kc.id, kc.content, kc.parent_id FROM knowledge_chunks kc \
         WHERE kc.id NOT IN (SELECT chunk_id FROM vector_cache) \
         AND kc.chunk_type='child' AND kc.content IS NOT NULL AND kc.content != '' \
         ORDER BY kc.id ASC LIMIT ?",
        &[json!(batch_size as i64)],
    ).unwrap_or_default().iter()
        .filter_map(|r| {
            let id = r["id"].as_i64()?;
            let content = r["content"].as_str().unwrap_or("").to_string();
            let parent_id = r["parent_id"].as_str().unwrap_or("").to_string();
            Some((id, content, parent_id))
        })
        .collect()
}

/// 向量化一批块并写入 LanceDB + vector_cache
async fn embed_batch(chunks: &[(i64, String, String)]) -> Result<usize, String> {
    if chunks.is_empty() { return Ok(0); }
    let chunk_ids: Vec<i64> = chunks.iter().map(|c| c.0).collect();
    let parent_ids: Vec<String> = chunks.iter().map(|c| c.2.clone()).collect();
    let texts: Vec<String> = chunks.iter().map(|c| c.1.clone()).collect();
    let vectors = embedding_client::embed_texts_with_caller(&texts, "vectorize").await.map_err(|e| format!("embed: {e}"))?;
    if vectors.len() != chunk_ids.len() { return Err(format!("count mismatch")); }

    let guard = lancedb_client::get_lancedb()?;
    if let Some(ref c) = *guard {
        // 纯追加写入。孤儿/重复向量被搜索 WHERE c.rowid=? JOIN knowledge_chunks 过滤，
        // 不再逐批 delete（LanceDB delete = 全表重写，会拖垮向量化）
        c.insert_vectors_full(&chunk_ids, &vectors, &[], &[], &parent_ids, &[], &[], &[], &[], &[])?;
    }

    for cid in &chunk_ids { database::run("INSERT OR IGNORE INTO vector_cache(chunk_id) VALUES(?)", &[json!(cid)]).ok(); }
    Ok(chunk_ids.len())
}

/// 标记块跳过（毒丸隔离：写入 vector_cache 防止永久重试，重向量化时可恢复）
fn mark_chunks_skipped(ids: &[i64]) {
    for cid in ids { database::run("INSERT OR IGNORE INTO vector_cache(chunk_id) VALUES(?)", &[json!(cid)]).ok(); }
}

/// 向量化待处理块：先批量尝试，失败时拆单隔离毒丸块（区分"坏块"与"API 不可用"）
pub async fn vectorize_new_chunks(batch_size: usize) -> Result<usize, String> {
    let chunks = get_pending_chunks(batch_size);
    if chunks.is_empty() { return Ok(0); }

    match embed_batch(&chunks).await {
        Ok(n) => Ok(n),
        Err(e) => {
            // API/网络不可用（超时、连接失败）→ 不拆单，直接返回错误交由 embed_loop 退避，
            // 避免"批量超时 + 逐条重试"的超时灾难性放大（最多 批数×单条超时）
            if e.contains("timed out") || e.contains("error sending request") || e.contains("connect") {
                return Err(e);
            }
            // 否则可能为单条毒丸文本 → 拆单隔离：
            // 本批有成功的 → API 正常，失败块为毒丸 → 跳过；全部失败 → 整体报错交由 embed_loop 退避
            let mut vectorized = 0usize;
            let mut failed_ids: Vec<i64> = Vec::new();
            let mut any_success = false;
            for (id, content, parent_id) in &chunks {
                match embed_batch(&[(*id, content.clone(), parent_id.clone())]).await {
                    Ok(_) => { vectorized += 1; any_success = true; }
                    Err(_) => failed_ids.push(*id),
                }
            }
            if any_success && !failed_ids.is_empty() {
                mark_chunks_skipped(&failed_ids);
                log::warn!("[知识管线] 毒丸隔离: 跳过 {} 个块（单块嵌入失败）: {:?}", failed_ids.len(), failed_ids);
                Ok(vectorized)
            } else {
                Err(e)
            }
        }
    }
}
