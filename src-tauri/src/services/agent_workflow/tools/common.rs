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

/// 笔记归属回显（检索/对象详情共用单一入口）——AI 用它回答"这笔记属于谁"。
/// 优先级与前端视图语义一致：闪记 → 每日闪记；笔记本 → 笔记本：X；
/// 项目笔记（note_project_mapping ⋈ projects）→ 项目：A、B（多项目顿号连接）；都没有 → 未归类。
/// 此前只查 notebook 且兜底"未归类"，项目笔记与闪记被误报为未归类。
pub(crate) fn note_attribution(note_id: &str) -> String {
    let row = database::get(
        "SELECT n.is_daily, COALESCE(nb.name,'') AS nb_name, \
         COALESCE((SELECT GROUP_CONCAT(DISTINCT p.name) FROM note_project_mapping npm \
         JOIN projects p ON p.id=npm.project_id WHERE npm.note_id=n.note_id),'') AS project_names \
         FROM notes n LEFT JOIN notebooks nb ON nb.id=n.notebook_id WHERE n.note_id=?1",
        &[json!(note_id)],
    ).ok().flatten();
    let Some(row) = row else { return "未归类".to_string() };
    let is_daily = row["is_daily"].as_i64().unwrap_or(0) != 0 || row["is_daily"].as_bool().unwrap_or(false);
    if is_daily {
        return "每日闪记".to_string();
    }
    let nb = row["nb_name"].as_str().unwrap_or("");
    if !nb.is_empty() {
        return format!("笔记本：{}", nb);
    }
    let projects = row["project_names"].as_str().unwrap_or("");
    if !projects.is_empty() {
        return format!("项目：{}", projects);
    }
    "未归类".to_string()
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



/// 引用入参归一（存储层写入前必经）：模型世界以 entity_ref 形态 `TYPE-uuid` 传递 ID，存储层只认裸 uuid。
/// 已注册前缀且类型符合预期 → 剥壳返回；类型不符 → 报错（防把 VENDOR 引用当项目写入）；
/// 裸串原样返回（entity_ref 不猜测裸串，交由调用方按 ID/名称另行解析）。
pub(crate) fn normalize_ref(raw: &str, expected: &[&str]) -> Result<String, String> {
    let raw = raw.trim();
    match super::entity_ref::parse(raw) {
        Some((t, id)) if expected.contains(&t) => Ok(id),
        Some((t, _)) => Err(format!(
            "引用类型不符：收到 {} 引用，应为 {} 引用或裸 ID",
            t, expected.join("/")
        )),
        None => Ok(raw.to_string()),
    }
}

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

#[cfg(test)]
mod tests {
    use crate::database::{run, get};

    /// 笔记归属：闪记 → 每日闪记；笔记本 → 笔记本：X；项目笔记 → 项目：A、B；无归属 → 未归类
    #[test]
    fn note_attribution_covers_daily_notebook_project_and_plain() {
        let (_dir, _g) = crate::database::db_core::test_util::setup_test_db();
        run("CREATE TABLE notes(note_id TEXT PRIMARY KEY, title TEXT, is_daily INTEGER NOT NULL DEFAULT 0, notebook_id TEXT)", &[]).unwrap();
        run("CREATE TABLE notebooks(id TEXT PRIMARY KEY, name TEXT)", &[]).unwrap();
        run("CREATE TABLE projects(id TEXT PRIMARY KEY, name TEXT)", &[]).unwrap();
        run("CREATE TABLE note_project_mapping(note_id TEXT, project_id TEXT)", &[]).unwrap();

        run("INSERT INTO notes(note_id,title,is_daily) VALUES('n-daily','随手记',1)", &[]).unwrap();
        run("INSERT INTO notes(note_id,title,is_daily,notebook_id) VALUES('n-nb','笔记',0,'nb1')", &[]).unwrap();
        run("INSERT INTO notes(note_id,title,is_daily) VALUES('n-proj','项目笔记',0)", &[]).unwrap();
        run("INSERT INTO notes(note_id,title,is_daily) VALUES('n-plain','裸笔记',0)", &[]).unwrap();
        run("INSERT INTO notebooks(id,name) VALUES('nb1','技术方案')", &[]).unwrap();
        run("INSERT INTO projects(id,name) VALUES('p1','华东医院'),('p2','智慧园区')", &[]).unwrap();
        run("INSERT INTO note_project_mapping VALUES('n-proj','p1'),('n-proj','p2')", &[]).unwrap();

        assert_eq!(super::note_attribution("n-daily"), "每日闪记");
        assert_eq!(super::note_attribution("n-nb"), "笔记本：技术方案");
        let proj = super::note_attribution("n-proj");
        assert!(proj.starts_with("项目："), "项目笔记应显示所属项目，实得: {}", proj);
        assert!(proj.contains("华东医院") && proj.contains("智慧园区"), "多项目顿号连接: {}", proj);
        assert_eq!(super::note_attribution("n-plain"), "未归类");
        assert_eq!(super::note_attribution("missing"), "未归类");
    }
}
