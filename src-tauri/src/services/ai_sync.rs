//! AI Sync — semantic materialization pipeline
//! Six source tables → natural language paragraphs → chunking → FTS5 indexing

use crate::database;
use serde_json::json;
use sha2::{Sha256, Digest};
pub(crate) fn compute_content_hash(content: &str) -> String {
    let mut h = Sha256::new(); h.update(content.as_bytes()); hex::encode(&h.finalize()[..32])
}

/// 纯函数：per-parent 哈希对比（"内容未变跳过"的判定核心）
pub(crate) fn hashes_unchanged(existing: &[String], new: &[String]) -> bool {
    existing.len() == new.len() && existing.iter().zip(new.iter()).all(|(a, b)| a == b)
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
        log::info!("[物化] ⚠ parse/build_sections 无结果，回退到 split_into_parents");
        split_into_parents(text, MAX_PARENT)
    } else {
        accumulate_adaptive_parents(sections)
    };

    let prefix = chunk_prefix.unwrap_or("");

    // ★ 先构建 parent_entries（切分后的最终父块，含 head 前缀拆分）
    //    去重必须比较真正入库的 parent_entries，而非切分前的 parents——
    //    否则超长父块被拆后数量不一致，去重恒失败导致重复物化
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

    // ★ Skip if content hasn't changed: compare per-parent hashes
    if let Some(ref sdid) = source_data_id {
        let new_hashes: Vec<String> = parent_entries.iter().map(|(t, _)| compute_content_hash(t)).collect();
        let existing_hashes: Vec<String> = database::all(
            "SELECT content_hash FROM knowledge_chunks WHERE source_data_id=?1 AND chunk_type='parent' ORDER BY chunk_index",
            &[json!(sdid)],
        ).unwrap_or_default().iter()
            .filter_map(|r| r["content_hash"].as_str().map(|s| s.to_string()))
            .collect();
        let unchanged = hashes_unchanged(&existing_hashes, &new_hashes);
        if unchanged {
            log::info!("[物化] 内容未改变，跳过: source_data_id={}", sdid);
            return Ok(0);  // 内容未变，0 个块
        }
    }
    // ★ 计算旧子块 rowid（事务外只读）
    let old_rowids: Vec<i64> = if let Some(ref sdid) = source_data_id {
        database::all(
            "SELECT id FROM knowledge_chunks WHERE source_data_id=?1 AND chunk_type='child'",
            &[json!(sdid)],
        ).unwrap_or_default().iter()
            .filter_map(|r| r["id"].as_i64())
            .collect()
    } else {
        Vec::new()
    };

    // O2: 删除旧块 + 插入新块在单个事务中执行（崩溃原子性，避免"删了未插"中间态）
    //     vector_cache 清理并入事务；LanceDB 清理放事务外（跨存储无法原子）
    let exec = |tx: &mut rusqlite::Transaction, sql: &str, params: &[serde_json::Value]| -> rusqlite::Result<usize> {
        let p: Vec<rusqlite::types::Value> = params.iter().map(crate::database::json_to_sqlite).collect();
        let mut stmt = tx.prepare(sql)?;
        stmt.execute(rusqlite::params_from_iter(p.iter()))
    };

    let total = {
        let sd_ref = source_data_id.as_deref().unwrap_or("");
        crate::database::transaction(|tx| {
            // 先清理 FTS5 索引（避免触发器报错）
            exec(tx, "DELETE FROM kk_chunks_fts WHERE rowid IN (SELECT id FROM knowledge_chunks WHERE source_data_id=?1 AND chunk_type='child')", &[json!(sd_ref)])?;
            // 删除旧块
            let deleted = exec(tx, "DELETE FROM knowledge_chunks WHERE source_data_id=?", &[json!(sd_ref)])?;
            log::info!("[物化] 已删除旧块: source_data_id={}, deleted={}", sd_ref, deleted);
            // 清理 vector_cache 中的孤儿
            if !old_rowids.is_empty() {
                let placeholders: Vec<String> = old_rowids.iter().map(|_| "?".to_string()).collect();
                exec(tx, &format!("DELETE FROM vector_cache WHERE chunk_id IN ({})", placeholders.join(",")),
                    &old_rowids.iter().map(|id| json!(id)).collect::<Vec<_>>())?;
            }
            // 逐父块插入
            let mut total = 0usize;
            for (ei, (parent_text, _)) in parent_entries.iter().enumerate() {
                let parent_id = uuid::Uuid::new_v4().to_string();
                // Parent record in knowledge_chunks
                log::info!("[物化] 插入父块[{}/{}]: chunk_id={}, 内容长度={}", ei+1, parent_entries_len, &parent_id[..8], parent_text.len());
                exec(tx,
                    "INSERT INTO knowledge_chunks(chunk_id,chunk_type,parent_id,source_data_id,source_type,source_id,content,doc_title,chunk_index,content_hash,created_at) VALUES(?1,'parent',NULL,?2,?3,?4,?5,?6,?7,?8,datetime('now','localtime'))",
                    &[json!(&parent_id), json!(source_data_id), json!(source_type), json!(source_id), json!(parent_text), json!(doc_title), json!((ei) as i64), json!(&compute_content_hash(parent_text))])?;

                //   L0: parent ≤ COMPACT_CHILD_LIMIT → 1 child (compact)
                //   L1: parent > COMPACT_CHILD_LIMIT → focused children (target=250)
                let child_len = parent_text.chars().count();
                if child_len <= COMPACT_CHILD_LIMIT {
                    let child_id = uuid::Uuid::new_v4().to_string();
                    let child_text = if prefix.is_empty() { parent_text.clone() } else { format!("{prefix}\n{parent_text}") };
                    let hash = compute_content_hash(&child_text);
                    log::info!("[物化] 插入子块: parent_id={}", crate::preview(&parent_id, 8));
                    if exec(tx,
                        "INSERT INTO knowledge_chunks(chunk_id,chunk_type,parent_id,source_data_id,source_type,source_id,content,doc_title,chunk_index,content_hash,base_weight,created_at) VALUES(?1,'child',?2,?3,?4,?5,?6,?7,0,?8,5.0,datetime('now','localtime'))",
                        &[json!(&child_id), json!(&parent_id), json!(source_data_id), json!(source_type), json!(source_id), json!(&child_text), json!(doc_title), json!(&hash)]).is_ok() { total += 1; } else { log::info!("[物化] 子块插入失败"); }
                } else {
                    let focused = split_text_by_lines(parent_text, FOCUSED_TARGET);
                    let focused: Vec<String> = if focused.is_empty() { vec![parent_text.clone()] } else { focused };
                    for (ci, child_text) in focused.iter().enumerate() {
                        let child_text = if prefix.is_empty() { child_text.clone() } else { format!("{prefix}\n{child_text}") };
                        let child_id = uuid::Uuid::new_v4().to_string();
                        let hash = compute_content_hash(&child_text);
                        if exec(tx,
                            "INSERT INTO knowledge_chunks(chunk_id,chunk_type,parent_id,source_data_id,source_type,source_id,content,doc_title,chunk_index,content_hash,base_weight,created_at) VALUES(?1,'child',?2,?3,?4,?5,?6,?7,?8,?9,5.0,datetime('now','localtime'))",
                            &[json!(&child_id), json!(&parent_id), json!(source_data_id), json!(source_type), json!(source_id), json!(child_text), json!(doc_title), json!((ci) as i64), json!(&hash)]).is_ok() { total += 1; }
                    }
                }
            }
            Ok(total)
        }).map_err(|e| format!("物化事务失败: {e}"))?
    };

    // LanceDB 孤儿向量清理已移出物化热路径（LanceDB delete 是全表重写，单次 ~6s）
    // 改为知识管线启动时批量清理（见 knowledge_pipeline::cleanup_orphan_vectors）

    if total > 0 { log::info!("[物化] 切片: {} 个父块 → {} 个子块 (adaptive)", parent_entries_len, total); }
    Ok(total)
}

// ═══════════════════════════════════════════════════════════════════
// Materialization: six source tables → natural language
// ═══════════════════════════════════════════════════════════════════

// ═══════════════════════════════════════════════════════════════════
// Materialize and Store (new interface, used by knowledge_pipeline)
// ═══════════════════════════════════════════════════════════════════

/// 从 source_data 读取内容并物化（供防抖调度器调用）
pub async fn process_single_entity(source_type: &str, source_id: &str) -> bool {
    match database::get(
        "SELECT id, content FROM source_data WHERE source_type=?1 AND source_id=?2",
        &[json!(source_type), json!(source_id)],
    ).ok().flatten() {
        Some(r) => {
            let sd_id = r["id"].as_str().unwrap_or("").to_string();
            let content = r["content"].as_str().unwrap_or("").to_string();
            materialize_and_store(source_type, source_id, sd_id, &content).await.is_ok()
        }
        None => {
            // source_data 已被删除，清理债务表避免死循环重试
            log::info!("[物化] source_data 已不存在，清理债务: {}:{}", source_type, source_id);
            database::run(
                "DELETE FROM materialize_retry WHERE source_type=?1 AND source_id=?2",
                &[json!(source_type), json!(source_id)],
            ).ok();
            true // 不是失败，是实体已被删除，返回成功避免重试
        }
    }
}

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
        // FILE / PROJECT / PRODUCT / CHAT 等：内容已在 source_data 中规范化，无需二次清洗
        _ => {}
    }

    if text.trim().is_empty() {
        return Ok(0);
    }

    // O1: 去重由 import_sync_document 内部 per-parent 哈希对比负责（唯一去重入口）
    //     已删除失效的 delete_chunks_by_source（as_i64 解析 TEXT id 永不生效）和
    //     永不命中的整篇 hash 对比死代码；同时消除了每次物化的全局 vector_cache 扫描

    // 导入新 chunks
    let n = import_sync_document(Some(source_data_id), &title, &text, None, &[], source_type, source_id).await
        .map_err(|e| format!("import_sync_document 失败: {}", e))?;

    if n > 0 {
        log::info!("[物化] 已更新: {}:{} ({} 个块)", source_type, source_id, n);
    }

    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_content_hash_deterministic() {
        let h1 = compute_content_hash("内容A");
        let h2 = compute_content_hash("内容A");
        let h3 = compute_content_hash("内容B");
        assert_eq!(h1, h2);
        assert_ne!(h1, h3);
        assert_eq!(h1.len(), 64, "SHA-256 hex 应为 64 字符");
    }

    #[test]
    fn test_hashes_unchanged() {
        let a = vec!["h1".to_string(), "h2".to_string()];
        assert!(hashes_unchanged(&a, &a.clone()));
        assert!(!hashes_unchanged(&a, &["h1".to_string()]));            // 数量变化
        assert!(!hashes_unchanged(&a, &["h1".to_string(), "h9".to_string()])); // 内容变化
        assert!(!hashes_unchanged(&a, &["h2".to_string(), "h1".to_string()])); // 顺序变化
        assert!(!hashes_unchanged(&[], &["h1".to_string()]));           // 新增
    }

    #[test]
    fn test_split_text_by_lines_respects_target() {
        let lines: Vec<String> = (0..20).map(|i| format!("第{}行，包含一定量的内容。", i)).collect();
        let text = lines.join("\n");
        let groups = split_text_by_lines(&text, 60);
        assert!(groups.len() > 1);
        // 内容不丢
        let joined = groups.join("\n");
        for l in &lines {
            assert!(joined.contains(l), "丢失行: {}", l);
        }
    }

    #[test]
    fn test_split_text_by_lines_short_passthrough() {
        assert_eq!(split_text_by_lines("单行", 100), vec!["单行".to_string()]);
        assert_eq!(split_text_by_lines("两行\n文本", 100), vec!["两行\n文本".to_string()]);
    }
}


