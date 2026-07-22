use rusqlite::{Connection, Result as SqliteResult};
use once_cell::sync::Lazy;
use std::sync::Mutex;
use base64::Engine;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static DB: Lazy<Mutex<Option<Connection>>> = Lazy::new(|| Mutex::new(None));
static LAST_CHECKPOINT: AtomicI64 = AtomicI64::new(0);
const CHECKPOINT_INTERVAL_SECS: i64 = 30;

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
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
    let last = LAST_CHECKPOINT.load(Ordering::Relaxed);
    if now - last < CHECKPOINT_INTERVAL_SECS {
        return Ok(());
    }
    LAST_CHECKPOINT.store(now, Ordering::Relaxed);
    with_db(|conn| conn.pragma_update(None, "wal_checkpoint", "TRUNCATE"))
}

pub(crate) fn save_forced() -> rusqlite::Result<()> {
    LAST_CHECKPOINT.store(
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64,
        Ordering::Relaxed,
    );
    with_db(|conn| conn.pragma_update(None, "wal_checkpoint", "TRUNCATE"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn setup_test_db() -> tempfile::TempDir {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
        set_db(conn);
        dir
    }

    #[test]
    fn test_escape_like() {
        assert_eq!(escape_like("hello"), "hello");
        assert_eq!(escape_like("hello%world"), "hello\\%world");
        assert_eq!(escape_like("hello_world"), "hello\\_world");
        assert_eq!(escape_like("hello\\world"), "hello\\\\world");
        assert_eq!(escape_like("%_\\abc"), "\\%\\_\\\\abc");
    }

    #[test]
    fn test_json_to_sqlite_null() {
        let val = serde_json::Value::Null;
        assert!(matches!(json_to_sqlite(&val), rusqlite::types::Value::Null));
    }

    #[test]
    fn test_json_to_sqlite_bool() {
        assert!(matches!(json_to_sqlite(&serde_json::json!(true)), rusqlite::types::Value::Integer(1)));
        assert!(matches!(json_to_sqlite(&serde_json::json!(false)), rusqlite::types::Value::Integer(0)));
    }

    #[test]
    fn test_json_to_sqlite_integer() {
        let val = json_to_sqlite(&serde_json::json!(42));
        assert!(matches!(val, rusqlite::types::Value::Integer(42)));
    }

    #[test]
    fn test_json_to_sqlite_real() {
        let val = json_to_sqlite(&serde_json::json!(3.14));
        assert!(matches!(val, rusqlite::types::Value::Real(3.14)));
    }

    #[test]
    fn test_json_to_sqlite_string() {
        let val = json_to_sqlite(&serde_json::json!("hello"));
        assert!(matches!(val, rusqlite::types::Value::Text(s) if s == "hello"));
    }

    #[test]
    fn test_sqlite_to_json_null() {
        let val = sqlite_to_json(rusqlite::types::Value::Null);
        assert!(val.is_null());
    }

    #[test]
    fn test_sqlite_to_json_integer() {
        let val = sqlite_to_json(rusqlite::types::Value::Integer(42));
        assert_eq!(val.as_i64(), Some(42));
    }

    #[test]
    fn test_sqlite_to_json_real() {
        let val = sqlite_to_json(rusqlite::types::Value::Real(3.14));
        assert_eq!(val.as_f64(), Some(3.14));
    }

    #[test]
    fn test_sqlite_to_json_text() {
        let val = sqlite_to_json(rusqlite::types::Value::Text("hello".to_string()));
        assert_eq!(val.as_str(), Some("hello"));
    }

    #[test]
    fn test_sqlite_to_json_blob() {
        let val = sqlite_to_json(rusqlite::types::Value::Blob(vec![72, 101]));
        // base64 of "He"
        assert_eq!(val.as_str(), Some("SGU="));
    }

    #[test]
    fn test_json_roundtrip() {
        // Test simple types that survive SQLite roundtrip
        // Note: bools become integers in SQLite (no native bool type)
        let cases = vec![
            (serde_json::json!(null), serde_json::json!(null)),
            (serde_json::json!(42), serde_json::json!(42)),
            (serde_json::json!(3.14), serde_json::json!(3.14)),
            (serde_json::json!("hello world"), serde_json::json!("hello world")),
            (serde_json::json!(true), serde_json::json!(1)),   // bool → integer
            (serde_json::json!(false), serde_json::json!(0)),  // bool → integer
        ];
        for (original, expected) in cases {
            let sqlite_val = json_to_sqlite(&original);
            let back = sqlite_to_json(sqlite_val);
            assert_eq!(expected, back, "Roundtrip failed for {:?}", original);
        }
    }

    #[test]
    fn test_run_create_table() {
        let _dir = setup_test_db();
        let result = run("CREATE TABLE test(id INTEGER PRIMARY KEY, name TEXT)", &[]);
        assert!(result.is_ok());
    }

    #[test]
    fn test_run_insert() {
        let _dir = setup_test_db();
        run("CREATE TABLE test(id INTEGER PRIMARY KEY, name TEXT)", &[]).unwrap();
        let result = run("INSERT INTO test(name) VALUES(?1)", &[serde_json::json!("hello")]);
        assert!(result.is_ok());
        let r = result.unwrap();
        assert_eq!(r.changes, 1);
        assert!(r.last_insert_rowid > 0);
    }

    #[test]
    fn test_run_update() {
        let _dir = setup_test_db();
        run("CREATE TABLE test(id INTEGER PRIMARY KEY, name TEXT)", &[]).unwrap();
        run("INSERT INTO test(name) VALUES(?1)", &[serde_json::json!("old")]).unwrap();
        let result = run("UPDATE test SET name=?1 WHERE id=1", &[serde_json::json!("new")]);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().changes, 1);
    }

    #[test]
    fn test_run_delete() {
        let _dir = setup_test_db();
        run("CREATE TABLE test(id INTEGER PRIMARY KEY, name TEXT)", &[]).unwrap();
        run("INSERT INTO test(name) VALUES(?1)", &[serde_json::json!("delete_me")]).unwrap();
        let result = run("DELETE FROM test WHERE id=1", &[]);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().changes, 1);
    }

    #[test]
    fn test_all_select() {
        let _dir = setup_test_db();
        run("CREATE TABLE test(id INTEGER PRIMARY KEY, name TEXT)", &[]).unwrap();
        run("INSERT INTO test(name) VALUES(?1)", &[serde_json::json!("alice")]).unwrap();
        run("INSERT INTO test(name) VALUES(?1)", &[serde_json::json!("bob")]).unwrap();
        let rows = all("SELECT * FROM test ORDER BY id", &[]).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["name"], "alice");
        assert_eq!(rows[1]["name"], "bob");
    }

    #[test]
    fn test_all_select_with_params() {
        let _dir = setup_test_db();
        run("CREATE TABLE test(id INTEGER PRIMARY KEY, name TEXT)", &[]).unwrap();
        run("INSERT INTO test(name) VALUES(?1)", &[serde_json::json!("alice")]).unwrap();
        run("INSERT INTO test(name) VALUES(?1)", &[serde_json::json!("bob")]).unwrap();
        let rows = all("SELECT * FROM test WHERE name=?1", &[serde_json::json!("bob")]).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["name"], "bob");
    }

    #[test]
    fn test_get_select_one() {
        let _dir = setup_test_db();
        run("CREATE TABLE test(id INTEGER PRIMARY KEY, name TEXT)", &[]).unwrap();
        run("INSERT INTO test(name) VALUES(?1)", &[serde_json::json!("alice")]).unwrap();
        let row = get("SELECT * FROM test WHERE id=1", &[]).unwrap();
        assert!(row.is_some());
        assert_eq!(row.unwrap()["name"], "alice");
    }

    #[test]
    fn test_get_returns_none() {
        let _dir = setup_test_db();
        run("CREATE TABLE test(id INTEGER PRIMARY KEY, name TEXT)", &[]).unwrap();
        let row = get("SELECT * FROM test WHERE id=999", &[]).unwrap();
        assert!(row.is_none());
    }

    #[test]
    fn test_all_empty_table() {
        let _dir = setup_test_db();
        run("CREATE TABLE test(id INTEGER PRIMARY KEY, name TEXT)", &[]).unwrap();
        let rows = all("SELECT * FROM test", &[]).unwrap();
        assert_eq!(rows.len(), 0);
    }

    #[test]
    fn test_db_path() {
        let dir = tempdir().unwrap();
        let path = db_path(dir.path());
        assert!(path.to_string_lossy().ends_with("arkmind.db"));
    }

    #[test]
    fn test_save() {
        let _dir = setup_test_db();
        let result = save();
        assert!(result.is_ok());
    }
}
