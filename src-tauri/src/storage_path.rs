//! 统一数据目录：外置指针 storage.json（固定放默认根，永不随迁移搬走）。
//! 鸡生蛋解法——路径配置不在 DB 里，启动读指针即可定位 arkmind.db/files/lancedb 等全部数据。

use std::path::{Path, PathBuf};
use std::sync::RwLock;

/// 指针文件名（固定在默认根）
pub const POINTER_FILE: &str = "storage.json";

/// 默认数据根（指针文件的唯一合法住址，本身永不迁移）
/// - macOS: ~/Arkmind（仅 Apple Silicon：bin/libpdfium.a 为 arm64-only，Intel Mac 无法构建）
/// - Windows: %APPDATA%/com.arkmind.app
pub fn default_data_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join("Arkmind")
    }
    #[cfg(not(target_os = "macos"))]
    {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("com.arkmind.app")
    }
}

pub fn pointer_path() -> PathBuf {
    default_data_dir().join(POINTER_FILE)
}

/// 指针内容（纯函数，可单测）
pub struct Pointer {
    /// 自定义数据目录；None = 使用默认根
    pub data_dir: Option<String>,
    /// 迁移前的原数据目录（清理/回切锚点）；None = 无待处理旧数据
    pub previous_dir: Option<String>,
}

fn opt_dir(v: &serde_json::Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(|x| x.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn parse_pointer(json_text: &str) -> Pointer {
    let v: serde_json::Value = serde_json::from_str(json_text).unwrap_or(serde_json::Value::Null);
    Pointer {
        data_dir: opt_dir(&v, "dataDir"),
        previous_dir: opt_dir(&v, "previousDir"),
    }
}

/// 解析生效数据目录（纯函数，可单测）：
/// - 指针缺失/损坏 → 默认根
/// - dataDir 为空 → 默认根
/// - dataDir 指向不存在的目录 → 默认根（自定义目录被外部删除时降级，不生成空库到坏路径）
pub fn resolve_data_dir(pointer_json: Option<&str>, default: &Path) -> PathBuf {
    let Some(text) = pointer_json else { return default.to_path_buf() };
    let p = parse_pointer(text);
    match p.data_dir {
        Some(d) => {
            let path = PathBuf::from(&d);
            if path.is_dir() {
                path
            } else {
                log::warn!("[Storage] 指针目录不存在，降级默认根: {}", d);
                default.to_path_buf()
            }
        }
        None => default.to_path_buf(),
    }
}

/// 进程内缓存：get_data_dir 在多处被调（db/lancedb/logs/temp），启动解析一次
static RESOLVED: RwLock<Option<PathBuf>> = RwLock::new(None);

/// 首次调用读指针并缓存；迁移命令改指针后必须 invalidate + 重启（重启即天然重建缓存）
pub fn resolved_data_dir() -> PathBuf {
    if let Ok(g) = RESOLVED.read() {
        if let Some(ref p) = *g {
            return p.clone();
        }
    }
    let default = default_data_dir();
    let text = std::fs::read_to_string(pointer_path()).ok();
    let resolved = resolve_data_dir(text.as_deref(), &default);
    if let Ok(mut g) = RESOLVED.write() {
        *g = Some(resolved.clone());
    }
    resolved
}

pub fn invalidate_cache() {
    if let Ok(mut g) = RESOLVED.write() {
        *g = None;
    }
}

/// 原子写指针（tmp + rename）。data_dir=None 时删除指针（回默认根）。
pub fn write_pointer(data_dir: Option<&Path>, previous_dir: Option<&Path>) -> Result<(), String> {
    let ptr = pointer_path();
    if let Some(parent) = ptr.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    match data_dir {
        None => {
            if ptr.exists() {
                let _ = std::fs::remove_file(&ptr);
            }
        }
        Some(dir) => {
            let mut obj = serde_json::json!({ "version": 1, "dataDir": dir.to_string_lossy() });
            if let Some(prev) = previous_dir {
                obj["previousDir"] = serde_json::json!(prev.to_string_lossy());
            }
            let tmp = ptr.with_extension("json.tmp");
            std::fs::write(&tmp, serde_json::to_vec_pretty(&obj).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            std::fs::rename(&tmp, &ptr).map_err(|e| e.to_string())?;
        }
    }
    invalidate_cache();
    Ok(())
}

/// 当前指针全文（设置卡解析用）
pub fn read_pointer_text() -> Option<String> {
    std::fs::read_to_string(pointer_path()).ok()
}

/// 当前设置卡展示用：返回 (生效目录, 是否自定义, 指针目标是否可用)
pub fn storage_status() -> (String, bool, bool) {
    let default = default_data_dir();
    let text = read_pointer_text();
    let p = parse_pointer(text.as_deref().unwrap_or(""));
    let custom = p.data_dir.clone();
    let effective = resolved_data_dir();
    let is_custom = custom.is_some() && effective != default;
    // 自定义指针存在但目标目录不可用 = 降级中（设置页警告）
    let broken = custom.as_ref().map_or(false, |d| !Path::new(d).is_dir());
    (effective.to_string_lossy().to_string(), is_custom, broken)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pointer_reads_datadir() {
        let p = parse_pointer(r#"{"version":1,"dataDir":"/data/x"}"#);
        assert_eq!(p.data_dir.as_deref(), Some("/data/x"));
        assert!(p.previous_dir.is_none(), "旧格式无 previousDir 应为 None");
        assert!(parse_pointer(r#"{"version":1}"#).data_dir.is_none());
        assert!(parse_pointer("").data_dir.is_none());
        assert!(parse_pointer(r#"{"dataDir":"  "}"#).data_dir.is_none());
    }

    #[test]
    fn parse_pointer_reads_previous_dir() {
        let p = parse_pointer(r#"{"version":1,"dataDir":"/new","previousDir":"/old"}"#);
        assert_eq!(p.data_dir.as_deref(), Some("/new"));
        assert_eq!(p.previous_dir.as_deref(), Some("/old"));
        // previousDir 空串 → None
        assert!(parse_pointer(r#"{"dataDir":"/new","previousDir":""}"#).previous_dir.is_none());
    }

    #[test]
    fn resolve_falls_back_to_default() {
        let d = Path::new("/default-root");
        // 无指针 / 坏 JSON → 默认
        assert_eq!(resolve_data_dir(None, d), d);
        assert_eq!(resolve_data_dir(Some("not-json"), d), d);
        // dataDir 空 → 默认
        assert_eq!(resolve_data_dir(Some(r#"{"dataDir":""}"#), d), d);
        // dataDir 指向不存在目录 → 默认（降级）
        assert_eq!(
            resolve_data_dir(Some(r#"{"dataDir":"/definitely/not/exist/xyz"}"#), d),
            d
        );
    }

    #[test]
    fn resolve_uses_existing_custom_dir() {
        // 用真实存在的临时目录验证自定义路径生效
        let tmp = std::env::temp_dir().join(format!("arkmnt-test-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let d = Path::new("/default-root");
        let json = format!(r#"{{"dataDir":"{}"}}"#, tmp.to_string_lossy());
        assert_eq!(resolve_data_dir(Some(&json), d), tmp);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
