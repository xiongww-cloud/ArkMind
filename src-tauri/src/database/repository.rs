use serde_json::json;

/// 查询 source_data 标题（不存在时返回空字符串）
pub fn source_title(source_type: &str, source_id: &str) -> String {
    super::get(
        "SELECT title FROM source_data WHERE source_type=?1 AND source_id=?2",
        &[json!(source_type), json!(source_id)],
    )
    .ok()
    .flatten()
    .and_then(|r| r["title"].as_str().map(String::from))
    .unwrap_or_default()
}

/// 查询 source_data.id（TEXT 类型），不存在返回 None
pub fn source_data_id(source_type: &str, source_id: &str) -> Option<String> {
    super::get(
        "SELECT id FROM source_data WHERE source_type=?1 AND source_id=?2",
        &[json!(source_type), json!(source_id)],
    )
    .ok()
    .flatten()
    .and_then(|r| r["id"].as_str().map(String::from))
    .filter(|s| !s.is_empty())
}

/// 查询 chunk 创建时间（不存在时返回空字符串）
pub fn chunk_created_at(chunk_rowid: i64) -> String {
    super::get(
        "SELECT created_at FROM knowledge_chunks WHERE id=?1",
        &[json!(chunk_rowid)],
    )
    .ok()
    .flatten()
    .and_then(|r| r["created_at"].as_str().map(String::from))
    .unwrap_or_default()
}

/// 统计某个 source_data 下的 parent chunk 数量
pub fn count_parent_chunks(source_data_id: &str) -> i64 {
    super::get(
        "SELECT COUNT(*) as cnt FROM knowledge_chunks WHERE source_data_id=?1 AND chunk_type='parent'",
        &[json!(source_data_id)],
    )
    .ok()
    .flatten()
    .and_then(|r| r["cnt"].as_i64())
    .unwrap_or(0)
}

/// 解析 "TYPE-uuid" 格式的 source 标识，返回 (大写类型, 裸id)
/// 无法识别类型时返回 ("__raw__", 原始字符串)
pub fn parse_source_id(source_id_str: &str) -> (String, String) {
    const KNOWN_TYPES: [&str; 9] = [
        "PROJECT", "PRODUCT", "FILE", "NOTE", "TODO",
        "CHANNEL", "VENDOR", "CONTACT", "CUSTOMER",
    ];
    if let Some(dash) = source_id_str.find('-') {
        let prefix = &source_id_str[..dash];
        let upper = prefix.to_uppercase();
        if KNOWN_TYPES.contains(&upper.as_str()) {
            return (upper, source_id_str[dash + 1..].to_string());
        }
    }
    ("__raw__".to_string(), source_id_str.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_source_id_with_prefix() {
        let (t, id) = parse_source_id("PROJECT-abc123");
        assert_eq!(t, "PROJECT");
        assert_eq!(id, "abc123");
    }

    #[test]
    fn test_parse_source_id_lowercase_prefix() {
        let (t, id) = parse_source_id("project-abc123");
        assert_eq!(t, "PROJECT");
        assert_eq!(id, "abc123");
    }

    #[test]
    fn test_parse_source_id_all_types() {
        for ty in ["PROJECT", "PRODUCT", "FILE", "NOTE", "TODO", "CHANNEL", "VENDOR", "CONTACT", "CUSTOMER"] {
            let (t, id) = parse_source_id(&format!("{}-x1", ty));
            assert_eq!(t, ty);
            assert_eq!(id, "x1");
        }
    }

    #[test]
    fn test_parse_source_id_raw_uuid() {
        let (t, id) = parse_source_id("b8c7ee85-d810-4da2-81b9-c6a33348520a");
        assert_eq!(t, "__raw__");
        assert_eq!(id, "b8c7ee85-d810-4da2-81b9-c6a33348520a");
    }

    #[test]
    fn test_parse_source_id_unknown_prefix() {
        let (t, id) = parse_source_id("MEMORY-abc123");
        assert_eq!(t, "__raw__");
        assert_eq!(id, "MEMORY-abc123");
    }
}
