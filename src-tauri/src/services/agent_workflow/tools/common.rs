//! 工具共享底座：source_id 解析 / chunk metadata / parent chunks / 时间解析 / 项目 ID 解析

use crate::database;
use serde_json::json;

// ═══════════════════════════════════════════════════════════════════

/// 从引用串解析 source_type 与裸 source_id（委托 entity_ref 单一入口）。
/// 兼容格式：TYPE-uuid；无法识别类型的输入整体视为裸 uuid（__raw__ 分支，向后兼容）。
pub(crate) fn parse_source_id(source_id_str: &str) -> (String, String) {
    match super::entity_ref::parse(source_id_str) {
        Some((t, id)) => (t.to_string(), id),
        None => ("__raw__".to_string(), source_id_str.trim().to_string()),
    }
}

/// 查询 source_data 表获取来源信息，返回 JSON 对象
/// 返回结构：{"found": true, "id": "uuid", "type": "PROJECT", "name": "名称", "source_data_id": "123"}
pub fn build_chunk_metadata(source_id_str: &str) -> serde_json::Value {
    if source_id_str.is_empty() { return json!({"found": false}); }

    let (source_type, source_id) = parse_source_id(source_id_str);
    
    let result = if source_type == "__raw__" {
        // 裸 UUID：搜索所有类型的 source_data
        database::get(
            "SELECT id, source_type, source_id, title FROM source_data WHERE source_id=?1 LIMIT 1",
            &[json!(&source_id)],
        )
    } else {
        // 带前缀：精确查询
        database::get(
            "SELECT id, source_type, source_id, title FROM source_data WHERE source_type=?1 AND source_id=?2",
            &[json!(&source_type), json!(&source_id)],
        )
    };

    match result {
        Ok(Some(r)) => {
            let sdid = r["id"].as_str().unwrap_or("").to_string();
            let st = r["source_type"].as_str().unwrap_or("").to_string();
            let sid = r["source_id"].as_str().unwrap_or("").to_string();
            let title = r["title"].as_str().unwrap_or("").to_string();
            json!({
                "found": true,
                "id": sid,
                "type": st,
                "name": title,
                "source_data_id": sdid
            })
        }
        _ => json!({"found": false})
    }
}

// ═══════════════════════════════════════════════════════════════════
// Tag-based pre-filtering (已废弃，tags 表已移除)
// ═══════════════════════════════════════════════════════════════════



/// 解析项目 ID：支持按 ID 或名称查找项目
pub(crate) fn resolve_project_id(project_id: &str) -> Option<String> {
    let raw_pid = project_id.trim();
    let clean_pid = raw_pid.strip_prefix("PROJECT-").unwrap_or(raw_pid);
    if clean_pid.is_empty() {
        return None;
    }
    // 先按 ID 查找
    if let Ok(Some(_)) = database::get("SELECT 1 FROM projects WHERE id=?1", &[json!(clean_pid)]) {
        return Some(clean_pid.to_string());
    }
    // 再按名称查找
    database::get(
        "SELECT id FROM projects WHERE name LIKE ?1 ESCAPE '\\' ORDER BY LENGTH(name) ASC LIMIT 1",
        &[json!(format!("%{}%", crate::database::escape_like(clean_pid)))],
    )
    .ok()
    .flatten()
    .and_then(|r| r["id"].as_str().map(String::from))
}


pub(crate) fn resolve_time(s: &str) -> String {
    let s = s.trim();
    if s.is_empty() { return String::new(); }
    // ISO 日期前缀直通：仅接受 ASCII 数字与连字符构成的前10字符（防多字节字符串按字节切片 panic）
    let head: String = s.chars().take(10).collect();
    if head.chars().count() == 10 && head.bytes().all(|b| b.is_ascii_digit() || b == b'-') && head.contains('-') { return head; }
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    // Handle relative expressions via simple matching
    if s.contains("昨天") || s.to_lowercase().contains("yesterday") { let d = chrono::Local::now() - chrono::Duration::days(1); return d.format("%Y-%m-%d").to_string(); }
    if s.contains("前天") { let d = chrono::Local::now() - chrono::Duration::days(2); return d.format("%Y-%m-%d").to_string(); }
    if s.contains("今天") || s.is_empty() { return today; }
    // fallback: return as-is
    s.to_string()
}

pub(crate) fn fetch_parent_chunks(source_data_id: &str, source_type: &str, source_id: &str, visited_chunks: &std::collections::HashSet<String>, visited_sources: &std::collections::HashSet<String>) -> serde_json::Value {
    log::info!("[fetch_parent_chunks] 查询: source_data_id={}, type={}, id={}", source_data_id, source_type, source_id);

    // 检查是否已在前文展示过
    let source_full_id = format!("{}-{}", source_type, source_id);
    let rows = database::all("SELECT chunk_id, content, chunk_index FROM knowledge_chunks WHERE source_data_id=?1 AND parent_id IS NULL ORDER BY chunk_index LIMIT 3", &[json!(source_data_id)]).unwrap_or_default();
    let chunk_ids: Vec<String> = rows.iter().map(|r| r["chunk_id"].as_str().unwrap_or("").chars().take(8).collect()).collect();
    if !chunk_ids.is_empty() && chunk_ids.iter().all(|id| visited_chunks.contains(id)) && visited_sources.contains(&source_full_id) {
        return json!({
            "found": true,
            "already_shown": true,
            "total_chunks": 0,
            "chunks": []
        });
    }

    // 查询 chunk 总数
    let total_chunks: i64 = database::get(
        "SELECT COUNT(*) as cnt FROM knowledge_chunks WHERE source_data_id=?1 AND parent_id IS NULL",
        &[json!(source_data_id)]
    ).ok().flatten().and_then(|r| r["cnt"].as_i64()).unwrap_or(0);

    let chunks: Vec<serde_json::Value> = rows.iter().enumerate().map(|(_i, r)| {
        let content = r["content"].as_str().unwrap_or("");
        let chunk_id = r["chunk_id"].as_str().unwrap_or("");
        let content_truncated = content;
        json!({
            "index": r["chunk_index"].as_i64().unwrap_or(0) + 1,
            "chunk_id": chunk_id.chars().take(8).collect::<String>(),
            "content": content_truncated
        })
    }).collect();

    json!({
        "found": true,
        "already_shown": false,
        "total_chunks": total_chunks,
        "chunks": chunks
    })
}
