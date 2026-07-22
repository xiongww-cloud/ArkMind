//! AI Sync — semantic materialization pipeline
//! Six source tables → natural language paragraphs → chunking → FTS5 indexing

use crate::database;
use serde_json::json;
use sha2::{Sha256, Digest};
pub(crate) fn compute_content_hash(content: &str) -> String {
    let mut h = Sha256::new(); h.update(content.as_bytes()); hex::encode(&h.finalize()[..32])
}

/// Split a text into line-groups, each group fitting within target_chars.
/// Preserves semantic line boundaries (unlike character-level truncation).
fn split_text_by_lines(text: &str, target_chars: usize) -> Vec<String> {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= 2 || text.chars().count() <= target_chars {
        return vec![text.to_string()];
    }

    let mut result: Vec<String> = Vec::new();
    let mut group: Vec<&str> = Vec::new();
    let mut group_chars = 0usize;

    for line in &lines {
        let lc = line.chars().count();
        if group_chars + lc > target_chars && !group.is_empty() {
            result.push(group.join("\n"));
            group.clear();
            group_chars = 0;
        }
        group.push(line);
        group_chars += lc;
    }
    if !group.is_empty() {
        result.push(group.join("\n"));
    }

    if result.is_empty() { vec![text.to_string()] } else { result }
}


use crate::services::smart_chunker::{
    split_into_parents,
    parse_blocks, build_sections, accumulate_adaptive_parents,
    COMPACT_THRESHOLD, MAX_PARENT, FOCUSED_TARGET,
};

/// ★ Full-format adaptive chunking — parent→child topology
async fn import_sync_document(
    source_data_id: Option<String>, doc_title: &str, text: &str,
    chunk_prefix: Option<&str>, _tags: &[String],
    source_type: &str, source_id: &str,
) -> Result<usize, String> {
    if text.trim().is_empty() { return Err("empty text".into()); }
    const COMPACT_CHILD_LIMIT: usize = COMPACT_THRESHOLD;

    // ★ Structure-aware adaptive chunking
    let blocks = parse_blocks(text);
    let sections = build_sections(blocks);
    let parents = if sections.is_empty() || (sections.len() == 1 && sections[0].heading.is_none() && sections[0].blocks.is_empty()) {
        log::info!("[导入] ⚠ parse/build_sections 无结果，回退到 split_into_parents");
        split_into_parents(text, MAX_PARENT)
    } else {
        accumulate_adaptive_parents(sections)
    };

    let prefix = chunk_prefix.unwrap_or("");
    let mut total = 0usize;

    // ★ Skip if content hasn't changed: compare per-parent hashes
    if let Some(ref sdid) = source_data_id {
        let new_hashes: Vec<String> = parents.iter().map(|p| compute_content_hash(p)).collect();
        let existing_hashes: Vec<String> = database::all(
            "SELECT content_hash FROM knowledge_chunks WHERE source_data_id=?1 AND chunk_type='parent' ORDER BY chunk_index",
            &[json!(sdid)],
        ).unwrap_or_default().iter()
            .filter_map(|r| r["content_hash"].as_str().map(|s| s.to_string()))
            .collect();
        let unchanged = existing_hashes.len() == new_hashes.len()
            && existing_hashes.iter().zip(new_hashes.iter()).all(|(a, b)| a == b);
        if unchanged {
            log::info!("[导入] 内容未改变，跳过: source_data_id={}", sdid);
            return Ok(0);  // 内容未变，0 个块
        }
    }
    // ★ Clear old chunks for this source (content changed or first import)
    //    同时清理对应 vector_cache 和 LanceDB 旧向量
    if let Some(ref sdid) = source_data_id {
        // Capture old child rowids before deletion
        let old_rowids: Vec<i64> = database::all(
            "SELECT id FROM knowledge_chunks WHERE source_data_id=?1 AND chunk_type='child'",
            &[json!(sdid)],
        ).unwrap_or_default().iter()
            .filter_map(|r| r["id"].as_i64())
            .collect();

        // 先清理 FTS5 索引（避免触发器报错）
        let _ = database::run(
            "DELETE FROM kk_chunks_fts WHERE rowid IN (SELECT id FROM knowledge_chunks WHERE source_data_id=?1 AND chunk_type='child')",
            &[json!(sdid)]
        );

        // 删除旧块
        let old_count = database::run("DELETE FROM knowledge_chunks WHERE source_data_id=?", &[json!(sdid)]);
        log::info!("[导入] 已删除旧块: source_data_id={}, deleted={:?}", sdid, old_count.map(|r| r.changes));

        // 清理 vector_cache 中的孤儿
        if !old_rowids.is_empty() {
            let placeholders: Vec<String> = old_rowids.iter().map(|_| "?".to_string()).collect();
            database::run(
                &format!("DELETE FROM vector_cache WHERE chunk_id IN ({})", placeholders.join(",")),
                &old_rowids.iter().map(|id| json!(id)).collect::<Vec<_>>(),
            ).ok();
        }
        // 清理 LanceDB 中对应的向量
        if let Ok(g) = crate::services::lancedb_client::get_lancedb() {
            if let Some(ref c) = *g {
                let _ = c.delete_chunks_by_rowids(&old_rowids);
            }
        }
    }

    // ★ 逐父块写入（表格在 accumulate_adaptive_parents 中已被原子化处理）
    //    但父块自身如果过大（> MAX_PARENT），按行拆分为多个子父块
    let mut parent_entries: Vec<(String, usize)> = Vec::new(); // (text, original_index)
    for (_pi, parent_text) in parents.iter().enumerate() {
        let lines: Vec<&str> = parent_text.lines().collect();
        if lines.len() > 2 && parent_text.chars().count() > MAX_PARENT {
            // Keep first 2 lines as context prefix for each sub-parent
            let head: Vec<&str> = lines[..2.min(lines.len())].to_vec();
            let head_text = head.join("\n");
            let head_chars = head_text.chars().count();
            let mut group: Vec<&str> = Vec::new();
            let mut group_chars = 0usize;
            for line in &lines[head.len()..] {
                let lc = line.chars().count();
                if group_chars + lc + head_chars > MAX_PARENT && !group.is_empty() {
                    parent_entries.push((format!("{}\n{}", head_text, group.join("\n")), 0));
                    group.clear();
                    group_chars = 0;
                }
                group.push(line);
                group_chars += lc;
            }
            if !group.is_empty() {
                parent_entries.push((format!("{}\n{}", head_text, group.join("\n")), 0));
            }
        } else {
            parent_entries.push((parent_text.clone(), 0));
        }
    }
    let parent_entries_len = parent_entries.len();

    for (ei, (parent_text, _)) in parent_entries.iter().enumerate() {
        let parent_id = uuid::Uuid::new_v4().to_string();

        // Parent record in knowledge_chunks
        log::info!("[导入] 插入父块[{}/{}]: chunk_id={}, 内容长度={}", ei+1, parent_entries_len, &parent_id[..8], parent_text.len());
        let pr = database::run(
            "INSERT INTO knowledge_chunks(chunk_id,chunk_type,parent_id,source_data_id,source_type,source_id,content,doc_title,chunk_index,content_hash,created_at) VALUES(?1,'parent',NULL,?2,?3,?4,?5,?6,?7,?8,datetime('now','localtime'))",
            &[json!(&parent_id), json!(source_data_id), json!(source_type), json!(source_id), json!(parent_text), json!(doc_title), json!((ei) as i64), json!(&compute_content_hash(parent_text))],
        );
        if let Err(e) = &pr {
            log::info!("[导入] 父块插入失败: {}", e);
            return Err(format!("parent insert: {e}"));
        }

        //   L0: parent ≤ COMPACT_CHILD_LIMIT → 1 child (compact)
        //   L1: parent > COMPACT_CHILD_LIMIT → focused children (target=250)
        //   (Tables are already converted to KV text at parent level by accumulate_adaptive_parents)
        let child_len = parent_text.chars().count();
        if child_len <= COMPACT_CHILD_LIMIT {
            let child_id = uuid::Uuid::new_v4().to_string();
            let child_text = if prefix.is_empty() { parent_text.clone() } else { format!("{prefix}\n{parent_text}") };
            let hash = compute_content_hash(&child_text);
            log::info!("[导入] 插入子块: parent_id={}, source_data_id={:?}", &parent_id[..parent_id.len().min(8)], source_data_id);
            if database::run(
                "INSERT INTO knowledge_chunks(chunk_id,chunk_type,parent_id,source_data_id,source_type,source_id,content,doc_title,chunk_index,content_hash,base_weight,created_at) VALUES(?1,'child',?2,?3,?4,?5,?6,?7,0,?8,5.0,datetime('now','localtime'))",
                &[json!(&child_id), json!(&parent_id), json!(source_data_id), json!(source_type), json!(source_id), json!(&child_text), json!(doc_title), json!(&hash)],
            ).is_ok() { total += 1; } else { log::info!("[导入] 子块插入失败"); }
        } else {
            let focused = split_text_by_lines(parent_text, FOCUSED_TARGET);
            let focused: Vec<String> = if focused.is_empty() { vec![parent_text.clone()] } else { focused };
            for (ci, child_text) in focused.iter().enumerate() {
                let child_text = if prefix.is_empty() { child_text.clone() } else { format!("{prefix}\n{child_text}") };
                let child_id = uuid::Uuid::new_v4().to_string();
                let hash = compute_content_hash(&child_text);
                if database::run(
                    "INSERT INTO knowledge_chunks(chunk_id,chunk_type,parent_id,source_data_id,source_type,source_id,content,doc_title,chunk_index,content_hash,base_weight,created_at) VALUES(?1,'child',?2,?3,?4,?5,?6,?7,?8,?9,5.0,datetime('now','localtime'))",
                    &[json!(&child_id), json!(&parent_id), json!(source_data_id), json!(source_type), json!(source_id), json!(child_text), json!(doc_title), json!((ci) as i64), json!(&hash)],
                ).is_ok() { total += 1; }
            }
        }
    }

    if total > 0 { println!("[AI同步] 切片: {} 个父块 → {} 个子块 (adaptive)", parent_entries_len, total); }
    Ok(total)
}

// ═══════════════════════════════════════════════════════════════════
// Materialization: six source tables → natural language
// ═══════════════════════════════════════════════════════════════════

// ═══════════════════════════════════════════════════════════════════
// Materialize and Store (new interface, used by knowledge_pipeline)
// ═══════════════════════════════════════════════════════════════════

fn delete_chunks_by_source(dedup_key: &str) {
    let parts: Vec<&str> = dedup_key.splitn(2, '-').collect();
    if parts.len() == 2 {
        let st = parts[0].to_uppercase().trim_end_matches('S').to_string();
        let sid = parts[1];
        if let Ok(Some(row)) = database::get("SELECT id FROM source_data WHERE source_type=?1 AND source_id=?2", &[json!(st), json!(sid)]) {
            if let Some(sdid) = row["id"].as_i64() {
                database::run("DELETE FROM knowledge_chunks WHERE source_data_id=?", &[json!(sdid)]).ok();
            }
        }
    }
    database::run("DELETE FROM vector_cache WHERE chunk_id NOT IN (SELECT id FROM knowledge_chunks)",&[]).ok();
}

/// 已禁用：改用时间戳驱动的物化管线
pub fn trigger_sync(_source_table: &str, _source_id: &str) {}

/// 直接从 source_data 物化并存储到 knowledge_chunks
/// 供知识管线调用
pub async fn materialize_and_store(
    source_type: &str,
    source_id: &str,
    source_data_id: String,
    content: &str,
) -> Result<usize, String> {
    if content.trim().is_empty() {
        return Ok(0);
    }

    // 获取标题
    let title = database::get(
        "SELECT title FROM source_data WHERE source_type=?1 AND source_id=?2",
        &[json!(source_type), json!(source_id)],
    )
    .ok()
    .flatten()
    .and_then(|r| r["title"].as_str().map(String::from))
    .unwrap_or_default();

    // 清洗内容（根据类型）
    let mut text = content.to_string();
    match source_type {
        "NOTE" | "TODO" => {
            text = crate::services::data_pipeline::strip_html_for_chunking(&text);
        }
        "FILE" => {
            // 文件内容不需要额外清洗
        }
        _ => {
            // 其他类型（PROJECT, PRODUCT, CHAT 等）通过管道处理
            let source_table = match source_type {
                "CHAT" => "chat_sessions".to_string(),
                _ => source_type.to_lowercase().trim_end_matches('s').to_string(),
            };
            let pipeline = crate::services::data_pipeline::get_pipeline();
            text = pipeline.process_file_stream(&source_table, source_id, text);
        }
    }

    if text.trim().is_empty() {
        return Ok(0);
    }

    // 计算去重 key
    let dedup_key = format!("{}-{}", source_type, source_id);
    let content_hash = compute_content_hash(&text);

    // 检查是否需要更新（哈希去重）
    let existing = database::get(
        "SELECT content_hash FROM knowledge_chunks WHERE source_type=?1 AND source_id=?2 AND chunk_type='parent' ORDER BY chunk_index LIMIT 1",
        &[json!(source_type), json!(source_id)],
    ).ok().flatten();
    
    if let Some(existing_hash) = existing.and_then(|r| r["content_hash"].as_str().map(String::from)) {
        if existing_hash == content_hash {
            log::info!("[物化] 内容未变，跳过: {}:{}", source_type, source_id);
            return Ok(0);
        }
    }

    // 删除旧的 chunks（使用 source_type + source_id）
    delete_chunks_by_source(&dedup_key);

    // 导入新 chunks
    let n = import_sync_document(Some(source_data_id), &title, &text, None, &[], source_type, source_id).await
        .map_err(|e| format!("import_sync_document 失败: {}", e))?;

    if n > 0 {
        println!("[物化] 已更新: {}:{} ({} 个块)", source_type, source_id, n);
    }

    Ok(n)
}


