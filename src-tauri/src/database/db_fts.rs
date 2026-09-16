use rusqlite::{Connection, Result as SqliteResult};
use crate::database::db_core::with_db;

#[derive(Debug,Clone,serde::Serialize)]
pub struct Fts5Result { pub rowid: i64, pub entity_type: String, pub entity_id: String, pub title: String, pub snippet: String, pub rank: f64, pub source: serde_json::Value }

/// Lightweight FTS5 hit: (chunk_rowid, bm25_score)
#[derive(Debug,Clone,serde::Serialize)]
pub struct Fts5Hit { pub chunk_id: i64, pub score: f64 }

/// Search kk_chunks_fts
pub fn fts5_search_chunks_slim(query: &str, limit: usize) -> SqliteResult<Vec<Fts5Hit>> {
    with_db(|conn| {
        let sql = "SELECT rowid, bm25(kk_chunks_fts, 5.0, 1.0) as rank FROM kk_chunks_fts WHERE kk_chunks_fts MATCH ? ORDER BY rank LIMIT ?";
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map(rusqlite::params![query, limit as i64], |row| {
            Ok(Fts5Hit { chunk_id: row.get(0)?, score: row.get::<_, f64>(1).unwrap_or(0.0) })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    })
}

/// Search kk_chunks_fts with snippets (for debug)
pub fn fts5_search_chunks(query: &str, limit: i64) -> SqliteResult<Vec<Fts5Result>> {
    with_db(|conn| {
        let sql = "SELECT rowid, snippet(kk_chunks_fts,0,'<mark>','</mark>','…',64) as snippet, bm25(kk_chunks_fts, 5.0, 1.0) as rank FROM kk_chunks_fts WHERE kk_chunks_fts MATCH ? ORDER BY rank LIMIT ?";
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map(rusqlite::params![query, limit], |row| Ok(Fts5Result{rowid:row.get(0)?,entity_type:String::new(),entity_id:String::new(),title:String::new(),snippet:row.get(1)?,rank:row.get(2)?,source:serde_json::Value::Null}))?;
        Ok(rows.filter_map(|r|r.ok()).collect())
    })
}

/// 大一统全局搜索 — global_search_fts
pub fn fts5_search_content(query: &str, limit: i64) -> SqliteResult<Vec<Fts5Result>> {
    with_db(|conn| {
        let sql = "SELECT rowid,source_type,source_id,title,snippet(global_search_fts,3,'<mark>','</mark>','…',64),bm25(global_search_fts,1.0,1.0) as rank FROM global_search_fts WHERE global_search_fts MATCH ? ORDER BY rank LIMIT ?";
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map(rusqlite::params![query, limit], |row| Ok(Fts5Result{rowid:row.get(0)?,entity_type:row.get::<_,String>(1).unwrap_or_default(),entity_id:row.get::<_,String>(2).unwrap_or_default(),title:row.get::<_,String>(3).unwrap_or_default(),snippet:row.get::<_,String>(4).unwrap_or_default(),rank:row.get::<_,f64>(5).unwrap_or(0.0),source:serde_json::Value::Null}))?;
        Ok(rows.filter_map(|r|r.ok()).collect())
    })
}

/// 文件解析完成后写入 source_data（FTS5 external content 自动同步索引）
/// 返回 Err 时调用方必须失败重试——此前吞错导致解析成果蒸发且队列照常删除
pub fn file_sync_to_source_data(file_id: &str, parsed_text: &str) -> Result<(), String> {
    if parsed_text.is_empty() { return Err("source_data 写入跳过：文本为空".to_string()); }
    // 从 file_assets 获取原始文件名作为 title
    let title = with_db(|c| {
        c.query_row(
            "SELECT original_name FROM file_assets WHERE id=?1",
            rusqlite::params![file_id],
            |row| row.get::<_, String>(0),
        )
    }).unwrap_or_default();
    let result = with_db(|c| {
        c.execute(
            "INSERT INTO source_data(id,source_type,source_id,title,content,updated_at) VALUES(?1,'FILE',?1,?2,?3,datetime('now', 'localtime')) ON CONFLICT(source_type,source_id) DO UPDATE SET content=excluded.content,updated_at=excluded.updated_at",
            rusqlite::params![file_id, title, parsed_text],
        ).map(|_|())
    });
    match result {
        Ok(_) => Ok(()),
        Err(e) => {
            log::warn!("[数据库] 写入 source_data 失败 (file={}): {}", file_id, e);
            Err(format!("写入 source_data 失败: {}", e))
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
// FTS5 triggers init — external content on source_data
// ═══════════════════════════════════════════════════════════════════

/// ★ 初始化 source_data → global_search_fts 的 3 个自动镜像触发器。
pub(crate) fn init_source_data_fts_triggers(conn: &Connection) -> rusqlite::Result<()> {
    let result = conn.execute_batch(
        "DROP TRIGGER IF EXISTS sd_fts_insert;
         DROP TRIGGER IF EXISTS sd_fts_update;
         DROP TRIGGER IF EXISTS sd_fts_delete;
         DROP TRIGGER IF EXISTS sd_chunks_delete;

         -- ★ AFTER INSERT: 将新行加入 FTS5 倒排索引。
         -- SQLite 根据 rowid 从 source_data 读取 title/content 并分词。
         CREATE TRIGGER IF NOT EXISTS sd_fts_insert AFTER INSERT ON source_data
         BEGIN
             INSERT INTO global_search_fts(rowid, source_type, source_id, title, content)
             VALUES (new.rowid, new.source_type, new.source_id, new.title, new.content);
         END;

         -- ★ AFTER UPDATE: 先删旧、再插新，保证索引与 source_data 一致。
         -- 注意：本次使用 INSERT OR REPLACE 写 source_data 时，SQLite 将其分解为
         -- DELETE + INSERT，分别触发 sd_fts_delete 和 sd_fts_insert，本触发器不参与。
         -- 此触发器仅覆盖对 source_data 执行显式 UPDATE 的边缘场景。
         CREATE TRIGGER IF NOT EXISTS sd_fts_update AFTER UPDATE ON source_data
         BEGIN
             INSERT INTO global_search_fts(global_search_fts, rowid, source_type, source_id, title, content)
             VALUES ('delete', old.rowid, old.source_type, old.source_id, old.title, old.content);
             INSERT INTO global_search_fts(rowid, source_type, source_id, title, content)
             VALUES (new.rowid, new.source_type, new.source_id, new.title, new.content);
         END;"
    );
    match result {
        Ok(()) => log::info!("[数据库] source_data → global_search_fts 镜像触发器已就绪"),
        Err(ref e) => log::info!("[数据库] FTS5 镜像触发器初始化失败: {e}"),
    }
    Ok(())
}

/// ★ Initialize AFTER INSERT/UPDATE/DELETE triggers for kk_chunks_fts (content-sync)
pub(crate) fn init_kb_fts_triggers(conn: &Connection) -> SqliteResult<()> {
    let result = conn.execute_batch(
        "DROP TRIGGER IF EXISTS kk_chunks_fts_insert;
         DROP TRIGGER IF EXISTS kk_chunks_fts_delete;
         DROP TRIGGER IF EXISTS kk_chunks_fts_update;

         -- ★ chunk_type='child' 确保仅子块入 FTS5 索引
         CREATE TRIGGER IF NOT EXISTS kk_chunks_fts_insert AFTER INSERT ON knowledge_chunks
         WHEN new.chunk_type = 'child'
         BEGIN
             INSERT INTO kk_chunks_fts(rowid, doc_title, content) VALUES (new.rowid, new.doc_title, new.content);
         END;

         CREATE TRIGGER IF NOT EXISTS kk_chunks_fts_delete AFTER DELETE ON knowledge_chunks
         BEGIN
             INSERT INTO kk_chunks_fts(kk_chunks_fts, rowid) VALUES('delete', old.rowid);
         END;

         CREATE TRIGGER IF NOT EXISTS kk_chunks_fts_update AFTER UPDATE ON knowledge_chunks
         WHEN old.chunk_type = 'child' OR new.chunk_type = 'child'
         BEGIN
             DELETE FROM kk_chunks_fts WHERE rowid = old.rowid;
             INSERT INTO kk_chunks_fts(rowid, doc_title, content)
               SELECT new.rowid, new.doc_title, new.content
               WHERE new.chunk_type = 'child';
         END;"
    );
    match result {
        Ok(()) => log::info!("[数据库] kk_chunks_fts 内容同步触发器已初始化 (仅限子块)"),
        Err(e) => log::info!("[数据库] 警告: kk_chunks_fts 触发器初始化失败 (可能来自旧版本 schema): {e}"),
    }
    Ok(())
}

/// Seed kk_chunks_fts from existing knowledge_chunks rows (idempotent)
pub(crate) fn seed_kb_fts(conn: &Connection) {
    let before: i64 = conn.query_row("SELECT COUNT(*) FROM kk_chunks_fts", [], |r| r.get(0)).unwrap_or(0);
    if let Err(e) = conn.execute_batch(
        "-- Child-only seed: chunk_type='child'
         INSERT OR IGNORE INTO kk_chunks_fts(rowid, doc_title, content)
         SELECT rowid, doc_title, content FROM knowledge_chunks WHERE chunk_type = 'child';"
    ) {
        log::info!("[数据库] kk_chunks_fts 播种批量失败: {}", e);
    }
    let after: i64 = conn.query_row("SELECT COUNT(*) FROM kk_chunks_fts", [], |r| r.get(0)).unwrap_or(0);
    if after > before {
        log::info!("[数据库] kk_chunks_fts 播种 (仅限子块): {} 行", after - before);
    } else {
        log::info!("[数据库] kk_chunks_fts 索引已是最新");
    }
}
