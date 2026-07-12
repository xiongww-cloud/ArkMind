use rusqlite::{Connection, Result as SqliteResult};
use once_cell::sync::Lazy;
use std::sync::Mutex;
use base64::Engine;

static DB: Lazy<Mutex<Option<Connection>>> = Lazy::new(|| Mutex::new(None));

pub fn db_path(data_dir: &std::path::Path) -> std::path::PathBuf {
    data_dir.join("arkmind.db")
}

pub fn with_db<F, T>(f: F) -> SqliteResult<T>
where F: FnOnce(&Connection) -> SqliteResult<T> {
    let db = DB.lock().unwrap_or_else(|e| e.into_inner());
    let conn = db.as_ref().ok_or_else(|| rusqlite::Error::InvalidPath("Database not initialized".into()))?;
    f(conn)
}

pub(crate) fn set_db(conn: Connection) {
    let mut db = DB.lock().unwrap_or_else(|e| e.into_inner());
    *db = Some(conn);
}

/// 转义 LIKE 查询中的通配符（% 和 _）
pub fn escape_like(input: &str) -> String {
    input.replace('\\', "\\\\")
         .replace('%', "\\%")
         .replace('_', "\\_")
}

// Type conversion
pub(crate) fn json_to_sqlite(val: &serde_json::Value) -> rusqlite::types::Value {
    match val { serde_json::Value::Null => rusqlite::types::Value::Null, serde_json::Value::Bool(b) => rusqlite::types::Value::Integer(if *b{1}else{0}), serde_json::Value::Number(n) => { if let Some(i)=n.as_i64(){rusqlite::types::Value::Integer(i)}else if let Some(f)=n.as_f64(){rusqlite::types::Value::Real(f)}else{rusqlite::types::Value::Text(n.to_string())} }, serde_json::Value::String(s) => rusqlite::types::Value::Text(s.clone()), _ => rusqlite::types::Value::Text(val.to_string()) }
}
pub(crate) fn sqlite_to_json(val: rusqlite::types::Value) -> serde_json::Value {
    match val { rusqlite::types::Value::Null=>serde_json::Value::Null, rusqlite::types::Value::Integer(i)=>serde_json::Value::Number(i.into()), rusqlite::types::Value::Real(f)=>serde_json::json!(f), rusqlite::types::Value::Text(s)=>serde_json::Value::String(s), rusqlite::types::Value::Blob(b)=>serde_json::Value::String(base64::engine::general_purpose::STANDARD.encode(&b)) }
}

#[derive(Debug,Clone,serde::Serialize)]
pub struct RunResult { pub changes: u32, pub last_insert_rowid: i64 }

pub(crate) fn all(sql: &str, params: &[serde_json::Value]) -> rusqlite::Result<Vec<serde_json::Value>> {
    with_db(|conn| {
        let mut stmt = conn.prepare(sql)?;
        let cols: Vec<String> = stmt.column_names().iter().map(|c|c.to_string()).collect();
        let p: Vec<rusqlite::types::Value> = params.iter().map(json_to_sqlite).collect();
        let rows = stmt.query_map(rusqlite::params_from_iter(p.iter()), |row| {
            let mut map = serde_json::Map::new();
            for (i,name) in cols.iter().enumerate() { map.insert(name.clone(), sqlite_to_json(row.get_unwrap(i))); }
            Ok(serde_json::Value::Object(map))
        })?;
        Ok(rows.filter_map(|r|r.ok()).collect())
    })
}

pub(crate) fn get(sql: &str, params: &[serde_json::Value]) -> rusqlite::Result<Option<serde_json::Value>> {
    all(sql, params).map(|rows| rows.into_iter().next())
}

pub(crate) fn run(sql: &str, params: &[serde_json::Value]) -> rusqlite::Result<RunResult> {
    with_db(|conn| {
        let p: Vec<rusqlite::types::Value> = params.iter().map(json_to_sqlite).collect();
        let mut stmt = conn.prepare(sql)?;
        let changes = stmt.execute(rusqlite::params_from_iter(p.iter()))?;
        Ok(RunResult { changes: changes as u32, last_insert_rowid: conn.last_insert_rowid() })
    })
}

pub(crate) fn save() -> rusqlite::Result<()> {
    with_db(|conn| conn.pragma_update(None, "wal_checkpoint", "TRUNCATE"))
}
