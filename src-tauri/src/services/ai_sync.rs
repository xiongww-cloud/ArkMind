//! AI Sync — semantic materialization pipeline
//! Six source tables → natural language paragraphs → chunking → FTS5 indexing

use crate::database;
use serde_json::json;
use sha2::{Sha256, Digest};
use std::sync::atomic::AtomicBool;

static WATCHDOG_RUNNING: AtomicBool = AtomicBool::new(false);
static LANCEDB_CHECKED: AtomicBool = AtomicBool::new(false);

struct MaterializedResult {
    text: String, project_id: Option<String>, dedup_key: String,
    chunk_prefix: Option<String>, created_at: Option<String>, tags: Vec<String>,
    source_data_id: Option<String>, real_title: String,
}

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
        eprintln!("[导入文档] ⚠ parse/build_sections 无结果，回退到 split_into_parents");
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
            eprintln!("[导入文档] 内容未改变，跳过: source_data_id={}", sdid);
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
        // 先手动清 FTS5 索引，避免 kk_chunks_fts_delete 触发器失败导致 DELETE 不生效
        for cid in &old_rowids {
            let _ = database::run("INSERT INTO kk_chunks_fts(kk_chunks_fts, rowid) VALUES('delete', ?)", &[json!(cid)]);
        }
        let old_count = database::run("DELETE FROM knowledge_chunks WHERE source_data_id=?", &[json!(sdid)]);
        eprintln!("[导入文档] 已删除旧块: source_data_id={}, deleted={:?}", sdid, old_count.map(|r| r.changes));
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
        eprintln!("[导入文档] 插入父块[{}/{}]: chunk_id={}, 内容长度={}", ei+1, parent_entries_len, &parent_id[..8], parent_text.len());
        let pr = database::run(
            "INSERT INTO knowledge_chunks(chunk_id,chunk_type,parent_id,source_data_id,source_type,source_id,content,doc_title,chunk_index,content_hash,created_at) VALUES(?1,'parent',NULL,?2,?3,?4,?5,?6,?7,?8,datetime('now','localtime'))",
            &[json!(&parent_id), json!(source_data_id), json!(source_type), json!(source_id), json!(parent_text), json!(doc_title), json!((ei) as i64), json!(&compute_content_hash(parent_text))],
        );
        if let Err(e) = &pr {
            eprintln!("[导入文档] 父块插入失败: {}", e);
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
            eprintln!("[导入文档] 插入子块: parent_id={}, source_data_id={:?}", &parent_id[..parent_id.len().min(8)], source_data_id);
            if database::run(
                "INSERT INTO knowledge_chunks(chunk_id,chunk_type,parent_id,source_data_id,source_type,source_id,content,doc_title,chunk_index,content_hash,base_weight,created_at) VALUES(?1,'child',?2,?3,?4,?5,?6,?7,0,?8,5.0,datetime('now','localtime'))",
                &[json!(&child_id), json!(&parent_id), json!(source_data_id), json!(source_type), json!(source_id), json!(&child_text), json!(doc_title), json!(&hash)],
            ).is_ok() { total += 1; } else { eprintln!("[导入文档] 子块插入失败"); }
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

async fn materialize_task(source_table: &str, source_id: &str) -> MaterializedResult {
    let empty = MaterializedResult { text: String::new(), project_id: None, dedup_key: format!("{source_table}-{source_id}"), chunk_prefix: None, created_at: None, tags: Vec::new(), source_data_id: None, real_title: String::new() };
    let key = format!("{source_table}-{source_id}");

    // ★ 统一从 source_data 读清洗后的内容，不做附加上下文拼接
    let source_type_q = match source_table {
        "chat_sessions" => "CHAT".to_string(),
        _ => source_table.to_uppercase().trim_end_matches('S').to_string(),
    };
    eprintln!("[物化任务] source_table={}, source_id={}, query_type={}", source_table, source_id, source_type_q);
    let sd = database::get(
        "SELECT id,title,content,updated_at FROM source_data WHERE source_type=?1 AND source_id=?2",
        &[json!(&source_type_q), json!(source_id)],
    ).ok().flatten();

    match source_table {
        "files" | "notes" | "products" | "projects" | "todos" | "chat" => {
            let row = match sd {
                Some(r) => {
                    let content_len = r["content"].as_str().map(|s| s.len()).unwrap_or(0);
                    eprintln!("[物化任务] 找到 source_data: type={} content_len={}", source_type_q, content_len);
                    r
                }
                None => {
                    eprintln!("[物化任务] 未找到 source_data: type={} id={}", source_type_q, source_id);
                    return empty;
                }
            };
            let content = row["content"].as_str().unwrap_or("");
            if content.is_empty() { return empty; }
            let title = row["title"].as_str().unwrap_or("");
            // text 不包含 title，由 build_chunk_metadata 提供元信息
            let text = content.to_string();
            let source_data_id = row["id"].as_str().map(String::from);

            // 真实标题
            let real_title = title.to_string();

            let created_at = row["updated_at"].as_str().map(|s| s.to_string());
            MaterializedResult { text, project_id: None, dedup_key: key, chunk_prefix: None, created_at, tags: Vec::new(), source_data_id, real_title }
        }
        _ => empty,
    }
}


// ═══════════════════════════════════════════════════════════════════
// Pipeline integration
// ═══════════════════════════════════════════════════════════════════

/// ★ 精准处理：按 source_table + source_id 定位并物化单个实体。
/// 供防抖调度器调用，避免扫全队列。
pub async fn process_single_entity(source_table: &str, source_id: &str) -> bool {
    let task = match database::get(
        "SELECT * FROM ai_sync_queue WHERE source_table=?1 AND source_id=?2 ORDER BY id ASC LIMIT 1",
        &[json!(source_table), json!(source_id)],
    ) {
        Ok(Some(t)) => t,
        _ => return false,
    };
    let task_id = task["id"].as_i64().unwrap_or(0);
    println!("[AI同步] 精准物化: {source_table}:{source_id}");

    tokio::task::yield_now().await;
    let mut materialized = materialize_task(source_table, source_id).await;

    // ★ 哈希去重：与队列路径相同逻辑
    let content_hash = if !materialized.text.is_empty() {
        compute_content_hash(&materialized.text)
    } else {
        materialized.dedup_key.clone()
    };
    let last_hash = task["last_content_hash"].as_str().unwrap_or("").to_string();
    if materialized.text.is_empty() || content_hash == last_hash {
        eprintln!("[精准物化] 内容未变或为空, 跳过 {source_table}:{source_id}");
        let _ = database::run(
            "UPDATE ai_sync_queue SET is_embedded=1,last_content_hash=?,updated_at=datetime('now', 'localtime') WHERE id=?",
            &[json!(&content_hash), json!(task_id)],
        );
    } else {
        if source_table != "files" {
            let before = materialized.text.len();
            if source_table == "notes" || source_table == "todos" {
                materialized.text = crate::services::data_pipeline::strip_html_for_chunking(&materialized.text);
            } else {
                let pipeline = crate::services::data_pipeline::get_pipeline();
                materialized.text = pipeline.process_file_stream(source_table, source_id, std::mem::take(&mut materialized.text));
            }
            if materialized.text.len() != before {
                println!(
                    "[AI同步] 已清理: {source_table}:{source_id} {}→{} 字符",
                    before, materialized.text.len()
                );
            }
            tokio::task::yield_now().await;
        }

        let doc_id = &materialized.dedup_key;
        let previous_key = task["last_dedup_key"].as_str().unwrap_or("").to_string();
        if !previous_key.is_empty() && previous_key != *doc_id {
            if let Ok(g) = crate::services::lancedb_client::get_lancedb() { if let Some(ref c) = *g { let _ = c.delete_by_hash_prefix(&previous_key); } }
            delete_chunks_by_source(&previous_key);
        }
        // 转换 source_table 为 source_type
        let source_type = match source_table {
            "chat_sessions" => "CHAT",
            _ => source_table.trim_end_matches('s'),
        };
        match import_sync_document(
            materialized.source_data_id, &materialized.real_title,
            &materialized.text,
            materialized.chunk_prefix.as_deref(), &materialized.tags,
            source_type, source_id,
        )
        .await
        {
            Ok(n) if n > 0 => {
                database::run(
                    "UPDATE ai_sync_queue SET is_embedded=1,last_dedup_key=?,last_content_hash=?,updated_at=datetime('now', 'localtime') WHERE id=?",
                    &[json!(doc_id), json!(&content_hash), json!(task_id)],
                ).ok();
                if source_table == "files" { }
                else if source_table == "notes" { }
                else if source_table == "products" { }
                else if source_table == "projects" { }
                else if source_table == "todos" { }
                println!("[AI同步] 已嵌入: {source_table}:{source_id} ({n} 个块)");
            }
            _ => {
                let _ = database::run(
                    "UPDATE ai_sync_queue SET is_embedded=-1,retry_count=retry_count+1,updated_at=datetime('now', 'localtime') WHERE id=?",
                    &[json!(task_id)],
                );
            }
        }
    }
    true
}

pub async fn process_sync_task() -> bool {
    let task = match database::get("SELECT * FROM ai_sync_queue WHERE is_embedded=0 ORDER BY id ASC LIMIT 1", &[]) {
        Ok(Some(t)) => t, _ => return false,
    };
    let task_id = task["id"].as_i64().unwrap_or(0);
    let source_table = task["source_table"].as_str().unwrap_or("");
    let source_id = task["source_id"].as_str().unwrap_or("");
    // ★ 跳过 project_dynamics 僵尸任务（历史残留，不再需要独立物化）
    if source_table == "project_dynamics" {
        database::run("DELETE FROM ai_sync_queue WHERE id=?", &[json!(task_id)]).ok();
        return false;
    }
    println!("[AI同步] 正在处理: {source_table}:{source_id}");

    // ★ 源数据不存在时直接标记完成，避免死循环
    let sd_exists = database::get(
        "SELECT 1 FROM source_data WHERE source_type=(SELECT CASE WHEN ?1='files' THEN 'FILE' WHEN ?1='notes' THEN 'NOTE' WHEN ?1='products' THEN 'PRODUCT' WHEN ?1='projects' THEN 'PROJECT' WHEN ?1='todos' THEN 'TODO' WHEN ?1='chat' OR ?1='chat_sessions' THEN 'CHAT' ELSE '' END) AND source_id=?2",
        &[json!(source_table), json!(source_id)],
    ).ok().flatten().is_some();
    if !sd_exists {
        eprintln!("[同步任务] source_data 不存在, 标记完成: {source_table}:{source_id}");
        if let Err(e) = database::run("UPDATE ai_sync_queue SET is_embedded=1,updated_at=datetime('now', 'localtime') WHERE id=?",&[json!(task_id)]) {
            eprintln!("[同步任务] 标记失败: {e}");
        }
        return true;
    }

    tokio::task::yield_now().await;
    let mut materialized = materialize_task(source_table, source_id).await;

    // ★ 哈希去重：内容无变化则跳过物化
    let content_hash = if !materialized.text.is_empty() {
        compute_content_hash(&materialized.text)
    } else {
        materialized.dedup_key.clone() // 空文本用dedup_key兜底
    };
    let last_hash = task["last_content_hash"].as_str().unwrap_or("").to_string();
    if materialized.text.is_empty() || content_hash == last_hash {
        eprintln!("[同步任务] hash诊断: table={} id={} content_len={} hash={} last={} isEmpty={}", source_table, source_id, materialized.text.len(), &content_hash, &last_hash, materialized.text.is_empty());
        eprintln!("[同步任务] 内容未变或为空, 跳过 {source_table}:{source_id}");
        if let Err(e) = database::run("UPDATE ai_sync_queue SET is_embedded=1,updated_at=datetime('now', 'localtime') WHERE id=?",&[json!(task_id)]) {
            eprintln!("[同步任务] 标记跳过失败: {e}");
        }
    } else {
        eprintln!("[同步任务] materialized.text 长度={}, 开始导入", materialized.text.len());
        if source_table != "files" {
            let before = materialized.text.len();
            // ★ notes/todos 含有 HTML，先清洗再分块
            if source_table == "notes" || source_table == "todos" {
                materialized.text = crate::services::data_pipeline::strip_html_for_chunking(&materialized.text);
            } else {
                let pipeline = crate::services::data_pipeline::get_pipeline();
                materialized.text = pipeline.process_file_stream(source_table, source_id, std::mem::take(&mut materialized.text));
            }
            if materialized.text.len() != before { println!("[AI同步] 已清理: {source_table}:{source_id} {}→{} 字符", before, materialized.text.len()); }
            tokio::task::yield_now().await;
        }

        let doc_id = &materialized.dedup_key;
        let previous_key = task["last_dedup_key"].as_str().unwrap_or("").to_string();
        if !previous_key.is_empty() && previous_key != *doc_id {
            if let Ok(g) = crate::services::lancedb_client::get_lancedb() { if let Some(ref c) = *g { let _ = c.delete_by_hash_prefix(&previous_key); } }
            delete_chunks_by_source(&previous_key);
        }
        let doc_title = &materialized.real_title;
        eprintln!("[同步任务] 准备导入: source_data_id={:?}, doc_title='{}', text_len={}", materialized.source_data_id, doc_title, materialized.text.len());
        // 转换 source_table 为 source_type
        let source_type = match source_table {
            "chat_sessions" => "CHAT",
            _ => source_table.trim_end_matches('s'),
        };
        match import_sync_document(materialized.source_data_id, doc_title, &materialized.text, materialized.chunk_prefix.as_deref(), &materialized.tags, source_type, source_id).await {
            Ok(n) if n > 0 => {
                database::run(
                    "UPDATE ai_sync_queue SET is_embedded=1,last_dedup_key=?,last_content_hash=?,updated_at=datetime('now', 'localtime') WHERE id=?",
                    &[json!(doc_id), json!(&content_hash), json!(task_id)],
                ).ok();
                if source_table == "files" { }
                else if source_table == "notes" { }
                else if source_table == "products" { }
                else if source_table == "projects" { }
                else if source_table == "todos" { }
                if source_table == "files" || source_table == "notes" || source_table == "products" || source_table == "projects" || source_table == "todos" {
                    
                }
                println!("[AI同步] 已嵌入: {source_table}:{source_id} ({n} 个块)");
            }
            Err(e) => {
                eprintln!("[同步任务] import_sync_document 失败: {}", e);
                let _ = database::run("UPDATE ai_sync_queue SET is_embedded=-1,retry_count=retry_count+1,last_error=?,updated_at=datetime('now', 'localtime') WHERE id=?",&[json!(e),json!(task_id)]);
            }
            _ => {
                eprintln!("[同步任务] import_sync_document 返回 0 个块（内容未变）");
                database::run(
                    "UPDATE ai_sync_queue SET is_embedded=1,last_content_hash=?,updated_at=datetime('now', 'localtime') WHERE id=?",
                    &[json!(content_hash), json!(task_id)],
                ).ok();
            }
        }
    }
    true
}

fn delete_chunks_by_source(dedup_key: &str) {
    // 从 dedup_key (格式 "{source_table}-{source_id}") 反查 source_data_id
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

// Seed / queue management
pub fn cold_start_seed_sync_queue() -> usize {
    let before: i64 = database::with_db(|c| c.query_row("SELECT COUNT(*) FROM ai_sync_queue",[],|r| r.get(0))).unwrap_or(0);
    let _ = database::run("CREATE UNIQUE INDEX IF NOT EXISTS idx_ai_sync_unique ON ai_sync_queue(source_table,source_id)",&[]);
    for sql in &[
        "INSERT OR IGNORE INTO ai_sync_queue(source_table,source_id) SELECT 'files',id FROM file_assets",
        "INSERT OR IGNORE INTO ai_sync_queue(source_table,source_id) SELECT 'products',id FROM products",
        "INSERT OR IGNORE INTO ai_sync_queue(source_table,source_id) SELECT 'projects',id FROM projects",
        "INSERT OR IGNORE INTO ai_sync_queue(source_table,source_id) SELECT 'todos',id FROM todos WHERE done=0",
    ] { database::run(sql, &[]).ok(); }
    let after: i64 = database::with_db(|c| c.query_row("SELECT COUNT(*) FROM ai_sync_queue",[],|r| r.get(0))).unwrap_or(0);
    let seeded = (after-before).max(0) as usize;
    if seeded > 0 { println!("[数据库] 冷启动播种: {seeded} 行"); }
    seeded
}

pub fn seed_new_records() {
    for sql in &[
        "INSERT OR IGNORE INTO ai_sync_queue(source_table,source_id) SELECT 'files',id FROM file_assets",
        "INSERT OR IGNORE INTO ai_sync_queue(source_table,source_id) SELECT 'products',id FROM products",
        "INSERT OR IGNORE INTO ai_sync_queue(source_table,source_id) SELECT 'projects',id FROM projects",
        "INSERT OR IGNORE INTO ai_sync_queue(source_table,source_id) SELECT 'todos',id FROM todos WHERE done=0",
    ] { database::run(sql, &[]).ok(); }
}

pub fn purge_stale_queue_entries() {
    for sql in &[
        "DELETE FROM ai_sync_queue WHERE source_table='projects' AND source_id NOT IN (SELECT id FROM projects)",
        "DELETE FROM ai_sync_queue WHERE source_table='files' AND source_id NOT IN (SELECT id FROM file_assets)",
        "DELETE FROM ai_sync_queue WHERE source_table='notes' AND source_id NOT IN (SELECT note_id FROM notes)",
        "DELETE FROM ai_sync_queue WHERE source_table='products' AND source_id NOT IN (SELECT id FROM products)",
        "DELETE FROM ai_sync_queue WHERE source_table='todos' AND source_id NOT IN (SELECT id FROM todos)",
        "DELETE FROM ai_sync_queue WHERE source_table='project_dynamics'",
    ] { database::run(sql, &[]).ok(); }
}

pub fn trigger_sync(_source_table: &str, _source_id: &str) {
    // 已禁用：改用时间戳驱动的物化管线，不再写入 ai_sync_queue
}

/// 从 source_data 补种同步队列（已禁用，改用时间戳驱动）
pub fn seed_from_source_data() {
    for (source_type, source_table) in &[
        ("FILE", "files"),
        ("NOTE", "notes"),
        ("PRODUCT", "products"),
        ("PROJECT", "projects"),
        ("TODO", "todos"),
    ] {
        let r = database::run(
            "INSERT OR IGNORE INTO ai_sync_queue(source_table,source_id) SELECT ?1,source_id FROM source_data WHERE source_type=?2",
            &[json!(source_table), json!(source_type)],
        );
        if let Ok(res) = r {
            if res.changes > 0 { println!("[数据库] 从 source_data 补种 {}: +{} 行", source_type, res.changes); }
        }
    }
}

/// 新接口：直接从 source_data 物化并存储到 knowledge_chunks
/// 供知识管线调用，不依赖 ai_sync_queue
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
            eprintln!("[物化] 内容未变，跳过: {}:{}", source_type, source_id);
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


