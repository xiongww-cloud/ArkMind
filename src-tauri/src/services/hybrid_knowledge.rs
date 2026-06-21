use crate::database; use crate::services::embedding_client; use crate::services::lancedb_client; use serde_json::json;

pub async fn vectorize_new_chunks(batch_size: usize) -> Result<usize, String> {
    let chunks = database::all(
        "SELECT kc.id, kc.content, kc.parent_id FROM knowledge_chunks kc \
         WHERE kc.id NOT IN (SELECT chunk_id FROM vector_cache) \
         AND kc.chunk_type='child' AND kc.content IS NOT NULL AND kc.content != '' \
         LIMIT ?",
        &[json!(batch_size as i64)],
    ).unwrap_or_default();
    if chunks.is_empty() { return Ok(0); }

    let chunk_ids: Vec<i64> = chunks.iter().map(|r| r["id"].as_i64().unwrap_or(0)).collect();
    let parent_ids: Vec<String> = chunks.iter().map(|r| r["parent_id"].as_str().unwrap_or("").to_string()).collect();

    let texts: Vec<String> = chunks.iter().map(|r| {
        let c = r["content"].as_str().unwrap_or("");
        c.chars().take(150).collect()
    }).collect();
    let vectors = embedding_client::embed_texts_with_caller(&texts, "vectorize").await.map_err(|e| format!("embed: {e}"))?;
    if vectors.len() != chunk_ids.len() { return Err(format!("count mismatch")); }

    let guard = lancedb_client::get_lancedb()?;
    if let Some(ref c) = *guard {
        c.insert_vectors_full(&chunk_ids, &vectors, &[], &[], &parent_ids, &[], &[], &[], &[], &[])?;
    }

    for cid in &chunk_ids { database::run("INSERT OR IGNORE INTO vector_cache(chunk_id) VALUES(?)", &[json!(cid)]).ok(); }
    Ok(chunk_ids.len())
}
