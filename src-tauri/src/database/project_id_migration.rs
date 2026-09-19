use rusqlite::Connection;
use serde::{Deserialize, Serialize};

pub const MIGRATION_VERSION: i64 = 1;

const STATE_KEY: &str = "project_id_migration_state";
const VERSION_KEY: &str = "project_id_migration_version";
const AT_KEY: &str = "project_id_migration_at";
const COUNT_KEY: &str = "project_id_migration_count";
const CONFLICTS_KEY: &str = "project_id_migration_conflicts";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdPair {
    #[serde(rename = "localId")]
    pub local_id: String,
    #[serde(rename = "backendId")]
    pub backend_id: String,
}

#[derive(Debug, Default, Serialize)]
pub struct MigrationReport {
    pub state: String,
    pub total_pairs: usize,
    pub migrated: usize,
    pub skipped: usize,
    pub conflicts: Vec<ConflictItem>,
    pub residue: Vec<ResidueItem>,
    pub duration_ms: u128,
}

#[derive(Debug, Serialize)]
pub struct ConflictItem {
    pub local_id: String,
    pub backend_id: String,
    pub reason: String,
}

#[derive(Debug, Serialize)]
pub struct ResidueItem {
    pub location: String,
    pub old_id: String,
    pub count: i64,
}

const REFS: &[(&str, &str, &str)] = &[
    ("project_dynamics.project_id",
     "UPDATE project_dynamics SET project_id=?1 WHERE project_id=?2",
     "SELECT COUNT(*) FROM project_dynamics WHERE project_id=?1"),
    ("solutions.project_id",
     "UPDATE solutions SET project_id=?1 WHERE project_id=?2",
     "SELECT COUNT(*) FROM solutions WHERE project_id=?1"),
    ("note_project_mapping.project_id",
     "UPDATE note_project_mapping SET project_id=?1 WHERE project_id=?2",
     "SELECT COUNT(*) FROM note_project_mapping WHERE project_id=?1"),
    ("todo_project_mapping.project_id",
     "UPDATE todo_project_mapping SET project_id=?1 WHERE project_id=?2",
     "SELECT COUNT(*) FROM todo_project_mapping WHERE project_id=?1"),
    ("partner_project_mapping.project_id",
     "UPDATE partner_project_mapping SET project_id=?1 WHERE project_id=?2",
     "SELECT COUNT(*) FROM partner_project_mapping WHERE project_id=?1"),
    ("contact_project_mapping.project_id",
     "UPDATE contact_project_mapping SET project_id=?1 WHERE project_id=?2",
     "SELECT COUNT(*) FROM contact_project_mapping WHERE project_id=?1"),
    ("file_relations.entity_id(project)",
     "UPDATE file_relations SET entity_id=?1 WHERE entity_id=?2 AND LOWER(entity_type)='project'",
     "SELECT COUNT(*) FROM file_relations WHERE entity_id=?1 AND LOWER(entity_type)='project'"),
    ("business_relations.source_id(PROJECT)",
     "UPDATE business_relations SET source_id=?1 WHERE source_id=?2 AND source_type='PROJECT'",
     "SELECT COUNT(*) FROM business_relations WHERE source_id=?1 AND source_type='PROJECT'"),
    ("business_relations.target_id(PROJECT)",
     "UPDATE business_relations SET target_id=?1 WHERE target_id=?2 AND target_type='PROJECT'",
     "SELECT COUNT(*) FROM business_relations WHERE target_id=?1 AND target_type='PROJECT'"),
    ("source_data.source_id(PROJECT)",
     "UPDATE source_data SET source_id=?1 WHERE source_id=?2 AND source_type='PROJECT'",
     "SELECT COUNT(*) FROM source_data WHERE source_id=?1 AND source_type='PROJECT'"),
    ("library_folders.project_id",
     "UPDATE library_folders SET project_id=?1 WHERE project_id=?2",
     "SELECT COUNT(*) FROM library_folders WHERE project_id=?1"),
    ("knowledge_push_queue.source_id(PROJECT)",
     "UPDATE knowledge_push_queue SET source_id=?1 WHERE source_id=?2 AND source_type='PROJECT'",
     "SELECT COUNT(*) FROM knowledge_push_queue WHERE source_id=?1 AND source_type='PROJECT'"),
];

fn set_setting(conn: &Connection, key: &str, value: &str) -> Result<(), String> {
    conn.execute(
        "INSERT OR REPLACE INTO system_settings(key, value, updated_at) VALUES(?1, ?2, datetime('now','localtime'))",
        rusqlite::params![key, value],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

fn get_setting(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row(
        "SELECT value FROM system_settings WHERE key=?1",
        rusqlite::params![key],
        |r| r.get::<_, String>(0),
    ).ok()
}

pub fn get_migration_state(conn: &Connection) -> String {
    get_setting(conn, STATE_KEY).unwrap_or_else(|| "pending".to_string())
}

pub(crate) fn ensure_migration_infrastructure(conn: &Connection) {
    if let Err(e) = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS project_id_migration_log (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            old_id TEXT NOT NULL, new_id TEXT NOT NULL,
            status TEXT NOT NULL,
            detail TEXT DEFAULT '',
            migrated_at TEXT DEFAULT (datetime('now','localtime'))
        );"
    ) {
        log::warn!("[ID迁移] 审计表创建失败: {}", e);
    }
    if get_setting(conn, STATE_KEY).is_none() {
        let _ = set_setting(conn, STATE_KEY, "pending");
    }
}

fn count_where(conn: &Connection, sql: &str, id: &str) -> i64 {
    conn.query_row(sql, rusqlite::params![id], |r| r.get::<_, i64>(0)).unwrap_or(0)
}

pub fn migrate_project_ids_canonical(conn: &Connection, pairs: &[IdPair]) -> Result<MigrationReport, String> {
    let start = std::time::Instant::now();
    ensure_migration_infrastructure(conn);

    let existing_state = get_migration_state(conn);
    if existing_state == "completed" || existing_state == "completed_with_conflicts" {
        return Ok(MigrationReport {
            state: existing_state,
            total_pairs: 0,
            migrated: get_setting(conn, COUNT_KEY).and_then(|v| v.parse().ok()).unwrap_or(0),
            skipped: 0,
            conflicts: Vec::new(),
            residue: Vec::new(),
            duration_ms: start.elapsed().as_millis(),
        });
    }

    set_setting(conn, STATE_KEY, "running")?;

    conn.pragma_update(None, "foreign_keys", "OFF").map_err(|e| e.to_string())?;
    let result = migrate_inner(conn, pairs);
    let _ = conn.pragma_update(None, "foreign_keys", "ON");

    match result {
        Ok(mut report) => {
            let state = if report.conflicts.is_empty() { "completed" } else { "completed_with_conflicts" };
            report.state = state.to_string();
            report.duration_ms = start.elapsed().as_millis();
            set_setting(conn, STATE_KEY, state)?;
            set_setting(conn, VERSION_KEY, &MIGRATION_VERSION.to_string())?;
            set_setting(conn, AT_KEY, &crate::time_utils::now_db())?;
            set_setting(conn, COUNT_KEY, &report.migrated.to_string())?;
            set_setting(conn, CONFLICTS_KEY, &report.conflicts.len().to_string())?;
            log::info!("[ID迁移] 完成: state={}, migrated={}, conflicts={}, {}ms",
                state, report.migrated, report.conflicts.len(), report.duration_ms);
            Ok(report)
        }
        Err(e) => {
            let _ = set_setting(conn, STATE_KEY, "failed");
            log::warn!("[ID迁移] 失败（已回滚）: {}", e);
            Err(e)
        }
    }
}

fn migrate_inner(conn: &Connection, pairs: &[IdPair]) -> Result<MigrationReport, String> {
    let mut report = MigrationReport { total_pairs: pairs.len(), ..Default::default() };

    let mut tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;

    for pair in pairs {
        let local = pair.local_id.trim();
        let backend = pair.backend_id.trim();
        if local.is_empty() || backend.is_empty() || local == backend {
            report.skipped += 1;
            continue;
        }

        let backend_exists = count_where(&tx, "SELECT COUNT(*) FROM projects WHERE id=?1", backend) > 0;
        if !backend_exists {
            report.skipped += 1;
            tx.execute(
                "INSERT INTO project_id_migration_log(old_id, new_id, status, detail) VALUES(?1,?2,'skipped','backend_not_found')",
                rusqlite::params![backend, local],
            ).map_err(|e| e.to_string())?;
            continue;
        }

        let local_exists = count_where(&tx, "SELECT COUNT(*) FROM projects WHERE id=?1", local) > 0;
        if local_exists {
            report.conflicts.push(ConflictItem {
                local_id: local.to_string(),
                backend_id: backend.to_string(),
                reason: "local_id_already_exists".to_string(),
            });
            tx.execute(
                "INSERT INTO project_id_migration_log(old_id, new_id, status, detail) VALUES(?1,?2,'conflict','local_id_already_exists')",
                rusqlite::params![backend, local],
            ).map_err(|e| e.to_string())?;
            continue;
        }

        let mut sp = tx.savepoint().map_err(|e| e.to_string())?;
        let pair_result: Result<(), String> = (|| {
            sp.execute("UPDATE projects SET id=?1 WHERE id=?2", rusqlite::params![local, backend])
                .map_err(|e| e.to_string())?;
            for (label, update_sql, _) in REFS {
                sp.execute(update_sql, rusqlite::params![local, backend])
                    .map_err(|e| format!("{}: {}", label, e))?;
            }
            let mut residue: Vec<ResidueItem> = Vec::new();
            let in_projects = count_where(&sp, "SELECT COUNT(*) FROM projects WHERE id=?1", backend);
            if in_projects > 0 {
                residue.push(ResidueItem { location: "projects.id".into(), old_id: backend.into(), count: in_projects });
            }
            for (label, _, count_sql) in REFS {
                let n = count_where(&sp, count_sql, backend);
                if n > 0 {
                    residue.push(ResidueItem { location: label.to_string(), old_id: backend.to_string(), count: n });
                }
            }
            if !residue.is_empty() {
                return Err(format!("integrity residue: {:?}", residue.iter().map(|r| &r.location).collect::<Vec<_>>()));
            }
            Ok(())
        })();

        match pair_result {
            Ok(()) => {
                sp.execute(
                    "INSERT INTO project_id_migration_log(old_id, new_id, status, detail) VALUES(?1,?2,'migrated','')",
                    rusqlite::params![backend, local],
                ).map_err(|e| e.to_string())?;
                sp.commit().map_err(|e| e.to_string())?;
                report.migrated += 1;
            }
            Err(reason) => {
                sp.rollback().map_err(|e| e.to_string())?;
                drop(sp);
                report.conflicts.push(ConflictItem {
                    local_id: local.to_string(),
                    backend_id: backend.to_string(),
                    reason: reason.clone(),
                });
                tx.execute(
                    "INSERT INTO project_id_migration_log(old_id, new_id, status, detail) VALUES(?1,?2,'conflict',?3)",
                    rusqlite::params![backend, local, reason],
                ).map_err(|e| e.to_string())?;
            }
        }
    }

    tx.commit().map_err(|e| e.to_string())?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("
            CREATE TABLE projects (id TEXT PRIMARY KEY, name TEXT NOT NULL DEFAULT '');
            CREATE TABLE project_dynamics (id TEXT PRIMARY KEY, project_id TEXT);
            CREATE TABLE solutions (id TEXT PRIMARY KEY, project_id TEXT);
            CREATE TABLE note_project_mapping (note_id TEXT, project_id TEXT, PRIMARY KEY(note_id, project_id));
            CREATE TABLE todo_project_mapping (todo_id TEXT, project_id TEXT, PRIMARY KEY(todo_id, project_id));
            CREATE TABLE partner_project_mapping (partner_id TEXT, project_id TEXT, PRIMARY KEY(partner_id, project_id));
            CREATE TABLE contact_project_mapping (contact_id TEXT, project_id TEXT, PRIMARY KEY(contact_id, project_id));
            CREATE TABLE file_relations (id TEXT PRIMARY KEY, entity_type TEXT, entity_id TEXT);
            CREATE TABLE business_relations (id TEXT PRIMARY KEY, source_type TEXT, source_id TEXT, target_type TEXT, target_id TEXT, relation_type TEXT DEFAULT 'belongs_to', metadata TEXT DEFAULT '{}');
            CREATE UNIQUE INDEX idx_br_unique ON business_relations(source_type, source_id, target_type, target_id, relation_type);
            CREATE TABLE source_data (source_type TEXT, source_id TEXT, content TEXT DEFAULT '');
            CREATE TABLE library_folders (id TEXT PRIMARY KEY, project_id TEXT);
            CREATE TABLE knowledge_push_queue (id INTEGER PRIMARY KEY AUTOINCREMENT, source_type TEXT, source_id TEXT);
            CREATE TABLE system_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at TEXT DEFAULT '');
        ").unwrap();
        conn
    }

    fn seed_project_with_refs(conn: &Connection, backend: &str) {
        let s = |p: &str| format!("{}-{}", p, backend);
        conn.execute("INSERT INTO projects(id, name) VALUES(?1, 'P')", [backend]).unwrap();
        conn.execute("INSERT INTO project_dynamics(id, project_id) VALUES(?2, ?1)", rusqlite::params![backend, s("d")]).unwrap();
        conn.execute("INSERT INTO solutions(id, project_id) VALUES(?2, ?1)", rusqlite::params![backend, s("s")]).unwrap();
        conn.execute("INSERT INTO note_project_mapping(note_id, project_id) VALUES(?2, ?1)", rusqlite::params![backend, s("n")]).unwrap();
        conn.execute("INSERT INTO todo_project_mapping(todo_id, project_id) VALUES(?2, ?1)", rusqlite::params![backend, s("t")]).unwrap();
        conn.execute("INSERT INTO partner_project_mapping(partner_id, project_id) VALUES(?2, ?1)", rusqlite::params![backend, s("pa")]).unwrap();
        conn.execute("INSERT INTO contact_project_mapping(contact_id, project_id) VALUES(?2, ?1)", rusqlite::params![backend, s("c")]).unwrap();
        conn.execute("INSERT INTO file_relations(id, entity_type, entity_id) VALUES(?2, 'project', ?1)", rusqlite::params![backend, s("f")]).unwrap();
        conn.execute("INSERT INTO business_relations(id, source_type, source_id, target_type, target_id) VALUES(?2, 'PROJECT', ?1, 'PRODUCT', 'prod1')", rusqlite::params![backend, s("b1")]).unwrap();
        conn.execute("INSERT INTO business_relations(id, source_type, source_id, target_type, target_id) VALUES(?2, 'NOTE', 'n1', 'PROJECT', ?1)", rusqlite::params![backend, s("b2")]).unwrap();
        conn.execute("INSERT INTO source_data(source_type, source_id) VALUES('PROJECT', ?1)", [backend]).unwrap();
        conn.execute("INSERT INTO library_folders(id, project_id) VALUES(?2, ?1)", rusqlite::params![backend, s("lf")]).unwrap();
        conn.execute("INSERT INTO knowledge_push_queue(source_type, source_id) VALUES('PROJECT', ?1)", [backend]).unwrap();
    }

    fn pair(local: &str, backend: &str) -> IdPair {
        IdPair { local_id: local.into(), backend_id: backend.into() }
    }

    fn count_sql(conn: &Connection, sql: &str) -> i64 {
        conn.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap()
    }

    fn assert_no_residue(conn: &Connection, old_id: &str) {
        assert_eq!(count_where(conn, "SELECT COUNT(*) FROM projects WHERE id=?1", old_id), 0, "projects");
        for (label, _, count_sql) in REFS {
            assert_eq!(count_where(conn, count_sql, old_id), 0, "residue in {}", label);
        }
    }

    fn snapshot(conn: &Connection) -> String {
        let tables = [
            "projects", "project_dynamics", "solutions", "note_project_mapping",
            "todo_project_mapping", "partner_project_mapping",
            "contact_project_mapping", "file_relations", "business_relations",
            "source_data", "library_folders", "knowledge_push_queue",
            "project_id_migration_log",
        ];
        let mut out = String::new();
        for t in tables {
            let cols: usize = conn.prepare(&format!("SELECT * FROM {} LIMIT 0", t)).unwrap().column_count();
            let order: String = (1..=cols).map(|i| i.to_string()).collect::<Vec<_>>().join(",");
            let mut stmt = conn.prepare(&format!("SELECT * FROM {} ORDER BY {}", t, order)).unwrap();
            let rows = stmt.query_map([], |r| {
                let mut line = String::new();
                for i in 0..cols {
                    let v = r.get_ref(i).unwrap();
                    line.push_str(&format!("{:?}|", v));
                }
                Ok(line)
            }).unwrap();
            out.push_str(&format!("== {} ==\n", t));
            for r in rows.flatten() { out.push_str(&r); out.push('\n'); }
        }
        out
    }

    #[test]
    fn test_migrate_full_integrity() {
        let conn = fixture_conn();
        seed_project_with_refs(&conn, "backend-aaa");

        let report = migrate_project_ids_canonical(&conn, &[pair("local-aaa", "backend-aaa")]).unwrap();
        assert_eq!(report.state, "completed");
        assert_eq!(report.migrated, 1);
        assert!(report.conflicts.is_empty());

        assert_no_residue(&conn, "backend-aaa");
        assert_eq!(count_sql(&conn, "SELECT COUNT(*) FROM projects WHERE id='local-aaa'"), 1);
        assert_eq!(count_sql(&conn, "SELECT COUNT(*) FROM project_dynamics WHERE project_id='local-aaa'"), 1);
        assert_eq!(count_sql(&conn, "SELECT COUNT(*) FROM business_relations WHERE source_type='PROJECT' AND source_id='local-aaa'"), 1);
        assert_eq!(count_sql(&conn, "SELECT COUNT(*) FROM business_relations WHERE target_type='PROJECT' AND target_id='local-aaa'"), 1);

        let log_status: String = conn.query_row(
            "SELECT status FROM project_id_migration_log WHERE old_id='backend-aaa'", [], |r| r.get(0)
        ).unwrap();
        assert_eq!(log_status, "migrated");
        assert_eq!(get_migration_state(&conn), "completed");
    }

    #[test]
    fn test_migrate_idempotent_second_run_is_noop() {
        let conn = fixture_conn();
        seed_project_with_refs(&conn, "backend-bbb");

        migrate_project_ids_canonical(&conn, &[pair("local-bbb", "backend-bbb")]).unwrap();
        let snap1 = snapshot(&conn);

        let report2 = migrate_project_ids_canonical(&conn, &[pair("local-bbb", "backend-bbb")]).unwrap();
        assert_eq!(report2.state, "completed");
        assert_eq!(report2.total_pairs, 0);

        let snap2 = snapshot(&conn);
        assert_eq!(snap1, snap2, "第二次迁移必须完全不改变数据");
    }

    #[test]
    fn test_migrate_conflict_local_id_occupied() {
        let conn = fixture_conn();
        seed_project_with_refs(&conn, "backend-ccc");
        conn.execute("INSERT INTO projects(id, name) VALUES('local-ccc', 'Other')", []).unwrap();
        seed_project_with_refs(&conn, "backend-ddd");

        let report = migrate_project_ids_canonical(&conn, &[
            pair("local-ccc", "backend-ccc"),
            pair("local-ddd", "backend-ddd"),
        ]).unwrap();

        assert_eq!(report.state, "completed_with_conflicts");
        assert_eq!(report.migrated, 1);
        assert_eq!(report.conflicts.len(), 1);
        assert_eq!(report.conflicts[0].reason, "local_id_already_exists");

        assert_eq!(count_sql(&conn, "SELECT COUNT(*) FROM projects WHERE id='backend-ccc'"), 1);
        assert_eq!(count_sql(&conn, "SELECT COUNT(*) FROM project_dynamics WHERE project_id='backend-ccc'"), 1);
        assert_no_residue(&conn, "backend-ddd");
    }

    #[test]
    fn test_migrate_skip_missing_backend_and_equal_ids() {
        let conn = fixture_conn();
        let report = migrate_project_ids_canonical(&conn, &[
            pair("local-x", "backend-nonexistent"),
            pair("same-id", "same-id"),
        ]).unwrap();
        assert_eq!(report.state, "completed");
        assert_eq!(report.migrated, 0);
        assert_eq!(report.skipped, 2);

        let status: String = conn.query_row(
            "SELECT status FROM project_id_migration_log WHERE old_id='backend-nonexistent'", [], |r| r.get(0)
        ).unwrap();
        assert_eq!(status, "skipped");
    }

    #[test]
    fn test_migrate_per_pair_rollback_on_injected_failure() {
        let conn = fixture_conn();
        seed_project_with_refs(&conn, "backend-eee");
        seed_project_with_refs(&conn, "backend-fff");
        conn.execute_batch("
            CREATE TRIGGER fail_note_update BEFORE UPDATE ON note_project_mapping
            WHEN OLD.project_id='backend-eee'
            BEGIN SELECT RAISE(ABORT, 'injected failure'); END;
        ").unwrap();

        let report = migrate_project_ids_canonical(&conn, &[
            pair("local-eee", "backend-eee"),
            pair("local-fff", "backend-fff"),
        ]).unwrap();

        assert_eq!(report.state, "completed_with_conflicts");
        assert_eq!(report.migrated, 1);
        assert_eq!(report.conflicts.len(), 1);
        assert!(report.conflicts[0].reason.contains("injected failure") || report.conflicts[0].reason.contains("note_project_mapping"));

        assert_eq!(count_sql(&conn, "SELECT COUNT(*) FROM projects WHERE id='backend-eee'"), 1);
        assert_eq!(count_sql(&conn, "SELECT COUNT(*) FROM project_dynamics WHERE project_id='backend-eee'"), 1);
        assert_eq!(count_sql(&conn, "SELECT COUNT(*) FROM solutions WHERE project_id='backend-eee'"), 1);
        assert_eq!(count_sql(&conn, "SELECT COUNT(*) FROM projects WHERE id='local-eee'"), 0);
        assert_no_residue(&conn, "backend-fff");
    }

    #[test]
    fn test_migrate_whole_failure_marks_failed_and_rolls_back() {
        let conn = fixture_conn();
        seed_project_with_refs(&conn, "backend-ggg");
        conn.execute_batch("CREATE VIEW project_id_migration_log AS SELECT 1 AS id;").unwrap();

        let result = migrate_project_ids_canonical(&conn, &[pair("local-ggg", "backend-ggg")]);
        assert!(result.is_err());
        assert_eq!(get_migration_state(&conn), "failed");

        assert_eq!(count_sql(&conn, "SELECT COUNT(*) FROM projects WHERE id='backend-ggg'"), 1);
        assert_eq!(count_sql(&conn, "SELECT COUNT(*) FROM projects WHERE id='local-ggg'"), 0);
        assert_eq!(count_sql(&conn, "SELECT COUNT(*) FROM project_dynamics WHERE project_id='backend-ggg'"), 1);

        conn.execute_batch("DROP VIEW project_id_migration_log;").unwrap();
        let report = migrate_project_ids_canonical(&conn, &[pair("local-ggg", "backend-ggg")]).unwrap();
        assert_eq!(report.state, "completed");
        assert_no_residue(&conn, "backend-ggg");
    }

    #[test]
    fn test_file_relations_entity_type_case_insensitive() {
        let conn = fixture_conn();
        conn.execute("INSERT INTO projects(id) VALUES('backend-hhh')", []).unwrap();
        conn.execute("INSERT INTO file_relations(id, entity_type, entity_id) VALUES('f1', 'PROJECT', 'backend-hhh')", []).unwrap();
        conn.execute("INSERT INTO file_relations(id, entity_type, entity_id) VALUES('f2', 'project', 'backend-hhh')", []).unwrap();

        let report = migrate_project_ids_canonical(&conn, &[pair("local-hhh", "backend-hhh")]).unwrap();
        assert_eq!(report.state, "completed");
        assert_eq!(count_sql(&conn, "SELECT COUNT(*) FROM file_relations WHERE entity_id='local-hhh'"), 2);
    }
}
