/// 本地时间戳 — 匹配 SQLite datetime('now','localtime') 格式
pub fn now_db() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}
