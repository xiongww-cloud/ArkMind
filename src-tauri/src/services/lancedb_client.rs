//! LanceDB client — vector storage with tag pre-filtering
use arrow_array::{FixedSizeListArray, Float32Array, Int64Array, RecordBatch, RecordBatchIterator, StringArray};
use arrow_schema::{DataType, Field, Schema};
use lancedb::query::{ExecutableQuery, QueryBase};
use once_cell::sync::Lazy;
use std::sync::Mutex;
use std::path::PathBuf;
use std::sync::Arc;
use futures::StreamExt;

fn sync_run<T,E:std::fmt::Display>(f: impl std::future::Future<Output=Result<T,E>>, ctx: &str) -> Result<T,String> {
    // worker/blocking/无 runtime 线程自动适配（block_in_place 在 spawn_blocking 线程会 panic）
    match super::tokio_ctx::block_on_sync(f) {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(format!("{ctx}: {e}")),
        Err(e) => Err(format!("{ctx}: {e}")),
    }
}

fn collect(stream: lancedb::arrow::SendableRecordBatchStream) -> Result<Vec<RecordBatch>, String> {
    let f = async { stream.collect::<Vec<_>>().await.into_iter().collect::<Result<Vec<_>,_>>().map_err(|e| format!("collect: {e}")) };
    super::tokio_ctx::block_on_sync(f).map_err(|e| format!("rt: {e}"))?
}

#[derive(Clone)]
pub struct LanceDBClient { db: lancedb::Connection, table_name: String }

impl LanceDBClient {
    pub fn new(db_path: &str, table_name: &str) -> Result<Self, String> {
        let path = PathBuf::from(db_path);
        if let Some(p) = path.parent() { std::fs::create_dir_all(p).map_err(|e| format!("mkdir: {e}"))?; }
        let db = sync_run(lancedb::connect(&path.to_string_lossy()).execute(), "connect")?;
        Ok(Self { db, table_name: table_name.into() })
    }

    fn table_exists(&self) -> Result<bool, String> {
        let names = sync_run(self.db.table_names().execute(), "list tables")?;
        Ok(names.iter().any(|n| n == &self.table_name))
    }

    fn create_table_with_dim(&self, dim: usize) -> Result<(), String> {
        let schema = Arc::new(Schema::new(vec![
            Field::new("rowid", DataType::Int64, false),
            Field::new("vector", DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Float32, true)), dim as i32), false),
            Field::new("parent_id", DataType::Utf8, false),
            Field::new("doc_id", DataType::Utf8, false),
            Field::new("content_hash", DataType::Utf8, false),
            Field::new("tags", DataType::Utf8, true),
        ]));
        let empty: Vec<Result<RecordBatch, arrow_schema::ArrowError>> = vec![];
        sync_run(self.db.create_table(&self.table_name, RecordBatchIterator::new(empty, schema)).execute(), "create table")?;
        Ok(())
    }

    /// Drop the vector table (for model change reset).
    #[allow(dead_code)]
    pub fn drop_table(&self) -> Result<(), String> {
        sync_run(self.db.drop_table(&self.table_name), "drop table").map_err(|e| e.to_string())
    }

    pub fn insert_vectors_full(&self, rowids: &[i64], vectors: &[Vec<f32>], hashes: &[String], _chunk_ids: &[String], parent_ids: &[String], doc_ids: &[String], _pids: &[String], _sources: &[String], _types: &[String], tags_list: &[String]) -> Result<usize, String> {
        if rowids.is_empty() { return Ok(0); }
        if !self.table_exists()? {
            let dim = vectors[0].len();
            self.create_table_with_dim(dim)?;
        }
        let n = rowids.len(); let dim = vectors[0].len();
        let schema = Arc::new(Schema::new(vec![
            Field::new("rowid", DataType::Int64, false),
            Field::new("vector", DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Float32, true)), dim as i32), false),
            Field::new("parent_id", DataType::Utf8, false),
            Field::new("doc_id", DataType::Utf8, false),
            Field::new("content_hash", DataType::Utf8, false),
            Field::new("tags", DataType::Utf8, true),
        ]));
        let pad = |s: &[String], n: usize| -> Vec<String> { let mut v = vec!["".to_string(); n]; for (i, x) in s.iter().enumerate().take(n) { v[i] = x.clone(); } v };
        let floats: Vec<f32> = vectors.iter().flat_map(|v| v.iter()).copied().collect();
        let vec_arr = FixedSizeListArray::try_new(Arc::new(Field::new("item", DataType::Float32, true)), dim as i32, Arc::new(Float32Array::from(floats)), None).map_err(|e| format!("vec: {e}"))?;
        let batch = RecordBatch::try_new(schema.clone(), vec![
            Arc::new(Int64Array::from(rowids.to_vec())), Arc::new(vec_arr),
            Arc::new(StringArray::from(pad(parent_ids, n))), Arc::new(StringArray::from(pad(doc_ids, n))),
            Arc::new(StringArray::from(pad(hashes, n))), Arc::new(StringArray::from(pad(tags_list, n))),
        ]).map_err(|e| format!("batch: {e}"))?;
        let table = sync_run(self.db.open_table(&self.table_name).execute(), "open")?;
        sync_run(table.add(RecordBatchIterator::new(vec![Ok(batch)], schema)).execute(), "insert")?;
        Ok(n)
    }

    pub fn search_vectors_with_parent(&self, query: &[f32], limit: usize) -> Result<Vec<(i64, f64, String, String, String)>, String> {
        if query.is_empty() { return Ok(Vec::new()); }
        if !self.table_exists()? { return Ok(Vec::new()); }
        let table = sync_run(self.db.open_table(&self.table_name).execute(), "open")?;
        let stream = sync_run(table.vector_search(query.to_vec()).map_err(|e| format!("search: {e}"))?.limit(limit).execute(), "execute")?;
        let batches = collect(stream)?;
        let mut items = Vec::new();
        for b in batches {
            let rids = b.column_by_name("rowid").ok_or("no rowid")?.as_any().downcast_ref::<Int64Array>().ok_or("cast")?;
            let dists = b.column_by_name("_distance").ok_or("no dist")?.as_any().downcast_ref::<arrow_array::Float32Array>().ok_or("cast")?;
            let pids = b.column_by_name("parent_id").ok_or("no pid")?.as_any().downcast_ref::<StringArray>().ok_or("cast")?;
            let dids = b.column_by_name("doc_id").ok_or("no did")?.as_any().downcast_ref::<StringArray>().ok_or("cast")?;
            for i in 0..b.num_rows() {
                let d = dists.value(i) as f64;
                let sim = (1.0 - d * d / 2.0).max(0.0);
                items.push((rids.value(i), sim, pids.value(i).to_string(), dids.value(i).to_string(), String::new()));
            }
        }
        items.sort_by(|a,b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        Ok(items)
    }

    #[allow(dead_code)]
    pub fn search_vectors_with_tags(&self, query: &[f32], limit: usize, tags: &[String]) -> Result<Vec<(i64, f64, String)>, String> {
        if query.is_empty() { return Ok(Vec::new()); }
        if !self.table_exists()? { return Ok(Vec::new()); }
        let table = sync_run(self.db.open_table(&self.table_name).execute(), "open")?;
        let filter = tags.iter()
            .filter(|t| !t.is_empty() && t.len() <= 200)
            .map(|t| format!("tags LIKE '%{}%'", crate::database::escape_like(&t.replace('\'', "''"))))
            .collect::<Vec<_>>().join(" OR ");
        let stream = sync_run(table.vector_search(query.to_vec()).map_err(|e| format!("search: {e}"))?.only_if(&filter).limit(limit).execute(), "execute")?;
        let batches = collect(stream)?;
        let mut items = Vec::new();
        for b in batches {
            let rids = b.column_by_name("rowid").ok_or("no rowid")?.as_any().downcast_ref::<Int64Array>().ok_or("cast")?;
            let dists = b.column_by_name("_distance").ok_or("no dist")?.as_any().downcast_ref::<arrow_array::Float32Array>().ok_or("cast")?;
            let pids = b.column_by_name("parent_id").ok_or("no pid")?.as_any().downcast_ref::<StringArray>().ok_or("cast")?;
            for i in 0..b.num_rows() {
                let d = dists.value(i) as f64;
                let sim = (1.0 - d * d / 2.0).max(0.0);
                items.push((rids.value(i), sim, pids.value(i).to_string()));
            }
        }
        items.sort_by(|a,b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        Ok(items.into_iter().map(|(r,s,p)| (r,s,p)).collect())
    }

    pub fn search_vectors_with_file_ids(&self, query: &[f32], limit: usize, file_ids: &[String]) -> Result<Vec<(i64, f64, String, String, String)>, String> {
        if query.is_empty() || file_ids.is_empty() { return Ok(Vec::new()); }
        if !self.table_exists()? { return Ok(Vec::new()); }
        let table = sync_run(self.db.open_table(&self.table_name).execute(), "open")?;
        let filter = file_ids.iter()
            .filter(|id| !id.is_empty() && id.len() <= 200 && id.chars().all(|c| c.is_alphanumeric() || c == '-'))
            .map(|id| format!("doc_id = '{}'", id.replace('\'', "''")))
            .collect::<Vec<_>>().join(" OR ");
        let stream = sync_run(table.vector_search(query.to_vec()).map_err(|e| format!("search: {e}"))?.only_if(&filter).limit(limit).execute(), "execute")?;
        let batches = collect(stream)?;
        let mut items = Vec::new();
        for b in batches {
            let rids = b.column_by_name("rowid").ok_or("no rowid")?.as_any().downcast_ref::<Int64Array>().ok_or("cast")?;
            let dists = b.column_by_name("_distance").ok_or("no dist")?.as_any().downcast_ref::<arrow_array::Float32Array>().ok_or("cast")?;
            let pids = b.column_by_name("parent_id").ok_or("no pid")?.as_any().downcast_ref::<StringArray>().ok_or("cast")?;
            let dids = b.column_by_name("doc_id").ok_or("no did")?.as_any().downcast_ref::<StringArray>().ok_or("cast")?;
            for i in 0..b.num_rows() {
                let d = dists.value(i) as f64;
                let sim = (1.0 - d * d / 2.0).max(0.0);
                items.push((rids.value(i), sim, pids.value(i).to_string(), dids.value(i).to_string(), String::new()));
            }
        }
        items.sort_by(|a,b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        Ok(items)
    }

    #[allow(dead_code)]
    pub fn count_vectors(&self) -> Result<usize, String> {
        if !self.table_exists()? { return Ok(0); }
        let table = sync_run(self.db.open_table(&self.table_name).execute(), "open")?;
        sync_run(table.count_rows(None), "count")
    }

    #[allow(dead_code)]
    pub fn delete_by_hash_prefix(&self, prefix: &str) -> Result<(), String> {
        if !self.table_exists()? { return Ok(()); }
        let table = sync_run(self.db.open_table(&self.table_name).execute(), "open")?;
        sync_run(table.delete(&format!("content_hash LIKE '{}%'", prefix)), "delete").map(|_|())
    }

    /// Delete vector records by specific rowids (content re-materialization).
    pub fn delete_chunks_by_rowids(&self, rowids: &[i64]) -> Result<(), String> {
        if rowids.is_empty() { return Ok(()); }
        if !self.table_exists()? { return Ok(()); }
        let ids: Vec<String> = rowids.iter().map(|id| id.to_string()).collect();
        let table = sync_run(self.db.open_table(&self.table_name).execute(), "open")?;
        sync_run(table.delete(&format!("rowid IN ({})", ids.join(","))), "delete").map(|_|())
    }

    /// Drop and recreate the vector table (full reset).
    pub fn clear_all_vectors(&self) -> Result<(), String> {
        sync_run(self.db.drop_table(&self.table_name), "drop table").ok();
        Ok(())
    }

    /// Scan all vector records (for debug browsing)
    pub fn scan_vectors(&self, limit: usize) -> Result<Vec<(i64, String, String, String)>, String> {
        if !self.table_exists()? { return Ok(Vec::new()); }
        let table = sync_run(self.db.open_table(&self.table_name).execute(), "open")?;
        // Use query() with limit to scan records (without vector search)
        let stream = sync_run(table.query().limit(limit).execute(), "scan")?;
        let batches = collect(stream)?;
        let mut items = Vec::new();
        for b in batches {
            let rids = b.column_by_name("rowid").ok_or("no rowid")?.as_any().downcast_ref::<Int64Array>().ok_or("cast")?;
            let pids = b.column_by_name("parent_id").ok_or("no pid")?.as_any().downcast_ref::<StringArray>().ok_or("cast")?;
            let dids = b.column_by_name("doc_id").ok_or("no did")?.as_any().downcast_ref::<StringArray>().ok_or("cast")?;
            let tags_col = b.column_by_name("tags").ok_or("no tags")?.as_any().downcast_ref::<StringArray>().ok_or("cast")?;
            for i in 0..b.num_rows() {
                items.push((rids.value(i), pids.value(i).to_string(), dids.value(i).to_string(), tags_col.value(i).to_string()));
            }
        }
        Ok(items)
    }

    /// 扫描所有向量记录的 rowid（孤儿清理用）
    /// 注意：LanceDB query 默认 limit=10，必须显式设置大 limit
    pub fn scan_all_rowids(&self) -> Result<Vec<i64>, String> {
        if !self.table_exists()? { return Ok(Vec::new()); }
        let table = sync_run(self.db.open_table(&self.table_name).execute(), "open")?;
        let stream = sync_run(table.query().limit(10_000_000).execute(), "scan")?;
        let batches = collect(stream)?;
        let mut ids = Vec::new();
        for b in batches {
            let rids = b.column_by_name("rowid").ok_or("no rowid")?.as_any().downcast_ref::<Int64Array>().ok_or("cast")?;
            for i in 0..b.num_rows() {
                ids.push(rids.value(i));
            }
        }
        Ok(ids)
    }
}

static LANCEDB: Lazy<Mutex<Option<LanceDBClient>>> = Lazy::new(|| Mutex::new(None));

pub fn get_lancedb() -> Result<std::sync::MutexGuard<'static, Option<LanceDBClient>>, String> {
    let mut g = LANCEDB.lock().unwrap_or_else(|p| p.into_inner());
    if g.is_none() {
        let path = crate::get_data_dir().join("lancedb");
        match LanceDBClient::new(&path.to_string_lossy(), "knowledge_vectors") { Ok(c) => { *g = Some(c); }, Err(e) => { *g = None; return Err(e); } }
    }
    Ok(g)
}

#[allow(dead_code)]
pub fn get_lancedb_safe() -> Option<LanceDBClient> {
    LANCEDB.lock().unwrap_or_else(|p| p.into_inner()).clone()
}

/// Reset in-memory LanceDB connection (used after clearing data)
pub fn invalidate_lancedb_cache() {
    *LANCEDB.lock().unwrap_or_else(|p| p.into_inner()) = None;
}
