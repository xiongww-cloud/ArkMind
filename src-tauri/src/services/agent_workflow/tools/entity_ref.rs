//! EntityRef：对象引用串的唯一约定。
//!
//! 规则（两层）：
//! - 存储层（DB 列、结构体字段）：裸 uuid + type 字段分离；
//! - 引用层（模型/前端可见的一切单串引用：文本输出、id 参数、列表、卡片）：
//!   `TYPE-大写-裸uuid`，如 `FILE-b94adcaf-63d0-4fc4-b371-5a3de9be7bdf`。
//!
//! 拼接与解析只允许经过本模块（禁止手写 split('-')/strip 前缀）；
//! 新增实体类型只需在 ENTITY_TYPES 登记一处，scope/注意力/引用解析自动一致。

/// 全部可作引用前缀的实体类型（大写，与 TYPE- 前缀字面一致；含下划线型不误切——按首个 '-' 切分）
pub(crate) const ENTITY_TYPES: &[&str] = &[
    "PROJECT", "PRODUCT", "FILE", "NOTE", "TODO", "CHANNEL", "VENDOR", "CUSTOMER", "CONTACT",
    "SOLUTION", "SOLUTION_VERSION", "SOLUTION_ITEM", "PROJECT_DYNAMIC", "EMAIL", "CHAT",
    // 容器引用（只读定位用，可作 ext_tool_search scope 入参自动展开；不进注意力管线）
    "NOTEBOOK", "FOLDER",
];

/// 可进入注意力/scope 的实体类型（有 source_data 检索语义的业务对象）
pub(crate) const SCOPE_TYPES: &[&str] = &[
    "PROJECT", "PRODUCT", "FILE", "NOTE", "TODO", "CHANNEL", "VENDOR", "CUSTOMER", "CONTACT", "CHAT",
];

/// (TYPE, 裸id) → EntityRef 引用串
pub(crate) fn fmt(entity_type: &str, bare_id: &str) -> String {
    format!("{}-{}", entity_type.to_ascii_uppercase(), bare_id)
}

/// 解析 EntityRef；非 `已注册TYPE-...` 形态返回 None（不猜测裸串）
pub(crate) fn parse(s: &str) -> Option<(&'static str, String)> {
    let s = s.trim();
    let dash = s.find('-')?;
    let up = s[..dash].to_ascii_uppercase();
    let t: &'static str = ENTITY_TYPES.iter().copied().find(|e| *e == up.as_str())?;
    let id = s[dash + 1..].trim();
    if id.is_empty() { return None; }
    Some((t, id.to_string()))
}

/// 任意输入（EntityRef 或裸 uuid+上下文类型）归一为 EntityRef；类型不合法返回 None
pub(crate) fn normalize(s: &str, fallback_type: &str) -> Option<String> {
    if let Some((t, id)) = parse(s) { return Some(fmt(t, &id)); }
    let s = s.trim();
    if s.is_empty() { return None; }
    let ft = fallback_type.trim().to_ascii_uppercase();
    if ENTITY_TYPES.iter().any(|e| *e == ft.as_str()) { Some(fmt(&ft, s)) } else { None }
}

/// 剥去已知 TYPE 前缀取裸 id（存储层写入用）；非注册前缀原样返回（绝不误切 "FOO-bar" 式 id）
pub(crate) fn bare_id(s: &str) -> String {
    match parse(s) {
        Some((_, id)) => id,
        None => s.trim().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{bare_id, fmt, normalize, parse};

    #[test]
    fn fmt_parse_roundtrip() {
        let r = fmt("file", "b94adcaf-63d0-4fc4-b371-5a3de9be7bdf");
        assert_eq!(r, "FILE-b94adcaf-63d0-4fc4-b371-5a3de9be7bdf");
        let (t, id) = parse(&r).unwrap();
        assert_eq!(t, "FILE");
        assert_eq!(id, "b94adcaf-63d0-4fc4-b371-5a3de9be7bdf");
    }

    #[test]
    fn underscore_types_split_at_first_dash_only() {
        let (t, id) = parse("SOLUTION_VERSION-aaaa-bbbb").unwrap();
        assert_eq!(t, "SOLUTION_VERSION");
        assert_eq!(id, "aaaa-bbbb");
    }

    #[test]
    fn bare_and_unknown_forms() {
        assert!(parse("b94adcaf-63d0-4fc4").is_none(), "裸 uuid 首段非注册类型 → 不猜测");
        // normalize 靠上下文类型成形
        let n = normalize("b94adcaf-63d0-4fc4-b371-5a3de9be7bdf", "file").unwrap();
        assert!(n.starts_with("FILE-b94adcaf"));
        assert!(normalize("x", "WIDGET").is_none(), "未登记类型拒绝");
        assert!(normalize("", "file").is_none());
    }

    #[test]
    fn bare_id_never_mangles_unknown_prefix() {
        assert_eq!(bare_id("FILE-abc-def"), "abc-def");
        assert_eq!(bare_id("FOO-bar"), "FOO-bar", "非注册前缀不得剥");
        assert_eq!(bare_id("abc"), "abc");
        assert_eq!(fmt("project", &bare_id("PROJECT-uuid7")), "PROJECT-uuid7");
    }
}
