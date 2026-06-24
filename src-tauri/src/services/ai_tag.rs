//! AI auto-tagging — LLM-driven semantic label generation
use crate::database;
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::collections::HashSet;
use std::time::Duration;
use rand::Rng;

type BoxedTagger = Arc<dyn Fn(String) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<String>, String>> + Send>> + Send + Sync>;
static TAGGER_REGISTRY: once_cell::sync::Lazy<Mutex<std::collections::HashMap<String, BoxedTagger>>> = once_cell::sync::Lazy::new(|| Mutex::new(std::collections::HashMap::new()));
macro_rules! lock_tag { ($m:expr) => { $m.lock().unwrap_or_else(|e| e.into_inner()) }; }
static TAG_COLORS: &[&str] = &["#6366f1","#8b5cf6","#ec4899","#f43f5e","#f97316","#eab308","#22c55e","#14b8a6","#06b6d4","#3b82f6"];
static TAG_LOOP_RUNNING: AtomicBool = AtomicBool::new(false);
static WAKE_TAG: std::sync::LazyLock<tokio::sync::Notify> = std::sync::LazyLock::new(tokio::sync::Notify::new);

/// 唤醒标签管线
pub fn wake_tag() {
    WAKE_TAG.notify_one();
}

pub fn register_tagger(source_type: &str, tagger: BoxedTagger) { lock_tag!(TAGGER_REGISTRY).insert(source_type.to_string(), tagger); }

fn normalize_tag(raw: String) -> Option<String> {
    let t = raw.trim().trim_start_matches(|c:char| c.is_ascii_digit()||c=='.'||c=='-').trim().replace(|c:char| c=='（', "(").replace('）',")").trim().to_string();
    if t.is_empty()||t=="[]"||t=="null" { None } else { Some(t) }
}

/// Extract head N chars + middle random continuous M chars from text.
fn sampled_text(text: &str, head_chars: usize, mid_chars: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    let total = chars.len();
    let head: String = chars.iter().take(head_chars).collect();
    if total <= head_chars + mid_chars {
        return chars.iter().collect();
    }
    let available = total.saturating_sub(head_chars);
    let window = mid_chars.min(available);
    if window == 0 { return head; }
    let max_start = total.saturating_sub(window);
    let mid_start = if max_start <= head_chars { head_chars } else {
        let mut rng = rand::thread_rng();
        rng.gen_range(head_chars..max_start)
    };
    let mid: String = chars[mid_start..mid_start + window].iter().collect();
    format!("{head}\n\n[...中部抽样...]\n\n{mid}")
}

/// ★ Load existing manual tags (exclude ai_shadow) from unified tag_mappings
fn load_existing_tags(source_type: &str, source_id: &str) -> Vec<String> {
    database::all(
        "SELECT t.name FROM tags t INNER JOIN tag_mappings m ON m.tag_id=t.tag_id WHERE m.entity_type=?1 AND m.entity_id=?2 AND t.type!='ai_shadow'",
        &[json!(source_type), json!(source_id)],
    ).unwrap_or_default().into_iter().filter_map(|r| r["name"].as_str().map(String::from)).collect()
}

// ═══════════════════════════════════════════════════════════════════
// LLM API call
// ═══════════════════════════════════════════════════════════════════

/// Merged LLM call: returns (tags, entities) from one prompt.
async fn call_ai_tag_api(title: &str, content: &str) -> Result<(Vec<String>, Vec<(String, String, String)>), String> {
    call_ai_tag_api_with_existing(title, content, &[], &[]).await
}

async fn call_ai_tag_api_with_existing(title: &str, content: &str, _existing: &[String], all_tags: &[String]) -> Result<(Vec<String>, Vec<(String, String, String)>), String> {
    let system = format!(
r#"你是一个数据分析师。从以下文本中提取标签。

【规则】
- 只能从下方「可用标签库」中选择最匹配的标签，不能创造新标签。
- 如果内容不匹配任何可用标签，输出空数组 []。
- 输出严格 JSON 格式：["标签1","标签2"]

【可用标签库】（只能从这里选）
{}

当前输入 标题：{} 内容：{} 输出："#,
        all_tags.join("、"),
        title, content
    );

    let config = crate::services::chat_api::get_chat_config();
    let mut raw = match crate::services::chat_api::chat_completion("只输出 JSON，无多余文字", &system, Some(crate::services::chat_api::ChatConfig{max_tokens:2048,temperature:0.0,..config.clone()})).await {
        Ok(r) => r,
        Err(e) => { eprintln!("[标签API] 第一次调用失败: {e}"); String::new() }
    };
    if raw.trim().is_empty() {
        raw = match crate::services::chat_api::chat_completion("只输出 JSON", &system, Some(crate::services::chat_api::ChatConfig{max_tokens:2048,temperature:0.3,..config})).await {
            Ok(r) => r,
            Err(e) => { eprintln!("[标签API] 第二次调用失败: {e}"); String::new() }
        };
    }

    let trimmed = raw.trim().trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```").trim();
    eprintln!("[标签API] 原始: {}", trimmed.chars().take(150).collect::<String>());

    // Parse as flat JSON array of tag names
    if let Ok(arr) = serde_json::from_str::<Vec<String>>(trimmed) {
        let tags: Vec<String> = arr.into_iter().filter_map(|s| normalize_tag(s)).filter(|t| !t.is_empty() && t.chars().count() <= 20).collect();
        if !tags.is_empty() {
            eprintln!("[标签API] 已解析: {} 个标签", tags.len());
            return Ok((tags, vec![]));
        }
    }
    Ok((vec![], vec![]))
}

// ═══════════════════════════════════════════════════════════════════
// Persistence
// ═══════════════════════════════════════════════════════════════════

fn persist_tags_sync(source_type: &str, source_id: &str, tag_names: &[String]) -> Result<(), String> {
    let tag_names: Vec<String> = tag_names.iter().map(|t| t.trim().to_string()).filter(|t| !t.is_empty() && t.chars().count()<=20).collect::<HashSet<_>>().into_iter().collect();
    if tag_names.is_empty() { return Ok(()); }

    database::with_db(|conn| {
        conn.execute("BEGIN",[])?;
        // Clear existing AI shadow tags for this entity
        conn.execute(
            "DELETE FROM tag_mappings WHERE entity_type=?1 AND entity_id=?2 AND tag_id IN (SELECT tag_id FROM tags WHERE type='ai_shadow')",
            rusqlite::params![source_type, source_id],
        )?;
        for name in &tag_names {
            let tid = match conn.query_row("SELECT tag_id FROM tags WHERE name=?", rusqlite::params![name], |r| r.get::<_,String>(0)) { Ok(id) => id, Err(_) => { let nid = uuid::Uuid::new_v4().to_string(); let c = TAG_COLORS[name.len()%TAG_COLORS.len()]; conn.execute("INSERT INTO tags(tag_id,name,color,type) VALUES(?1,?2,?3,'ai_shadow')", rusqlite::params![nid, name, c])?; nid } };
            conn.execute(
                "INSERT OR IGNORE INTO tag_mappings(entity_type,entity_id,tag_id) VALUES(?1,?2,?3)",
                rusqlite::params![source_type, source_id, tid],
            )?;
        }
        conn.execute("COMMIT",[]).map(|_|())?;
        Ok(())
    }).map_err(|e| e.to_string())
}

// ═══════════════════════════════════════════════════════════════════
// Generic tagger — reads from source_data
// ═══════════════════════════════════════════════════════════════════

async fn tag_from_source_data(source_type: &str, source_id: &str) -> Result<Vec<String>, String> {
    // ai_tag_queue 存的是 "notes"/"files" (小写+s)，但 source_data 存的是 "NOTE"/"FILE" (大写)
    // 统一转大写去除尾部的 's' 来匹配 source_data
    let st_upper = source_type.trim_end_matches('s').to_uppercase();
    let row = database::get(
        "SELECT title, content FROM source_data WHERE source_type=?1 AND source_id=?2",
        &[json!(&st_upper), json!(source_id)],
    ).ok().flatten();
    let (title, content) = match row {
        Some(r) => (
            r["title"].as_str().unwrap_or("").to_string(),
            r["content"].as_str().unwrap_or("").to_string(),
        ),
        None => return Ok(Vec::new()),
    };
    if content.trim().is_empty() { return Ok(Vec::new()); }
    let title_str = if title.is_empty() { source_type } else { &title };
    let plain: String = content.chars().filter(|c| *c!='<'&&*c!='>').collect();
    let sample = sampled_text(&plain, 1000, 3000);
    if sample.len() < 10 { return Ok(Vec::new()); }
    let existing = load_existing_tags(source_type, source_id);
    // 加载全量标签库供 LLM 选择（只能选已有标签，不能创造新标签）
    let all_tags: Vec<String> = database::all("SELECT name FROM tags WHERE type!='ai_shadow' ORDER BY name",&[]).unwrap_or_default().into_iter().filter_map(|r| r["name"].as_str().map(String::from)).collect();
    let (tags, _entities) = call_ai_tag_api_with_existing(title_str, &sample, &existing, &all_tags).await?;
    // 实体抽取已关闭 — 不使用 entity_mentions / entities 表
    Ok(tags)
}

pub fn register_builtin_taggers() {
    for st in &["notes", "files", "products", "projects", "todos"] {
        let st_owned = st.to_string();
        register_tagger(st, Arc::new(move |sid| {
            let st = st_owned.clone();
            Box::pin(async move { tag_from_source_data(&st, &sid).await })
        }));
    }
}

// ═══════════════════════════════════════════════════════════════════
// Independent tag loop
// ═══════════════════════════════════════════════════════════════════

pub fn start() {
    if TAG_LOOP_RUNNING.swap(true, Ordering::AcqRel) {
        println!("[标签管线] 已在运行中");
        return;
    }
    register_builtin_taggers();
    println!("[标签管线] 独立打标线程启动");

    std::thread::spawn(|| {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("[标签管线] tokio 运行时创建失败");
        rt.block_on(tag_loop());
    });
}

async fn tag_loop() {
    let mut tick: u64 = 0;
    loop {
        if !TAG_LOOP_RUNNING.load(Ordering::Acquire) { break; }
        tick = tick.wrapping_add(1);

        // Seed new records every ~2 ticks (≈1 min at 30s idle)
        if tick % 2 == 0 {
            seed_pending_tags();
        }

        let had_work = process_tag_task().await;

        // Event-driven: no work → wait for notification or 30s timeout
        if !had_work {
            tokio::select! {
                _ = WAKE_TAG.notified() => {},
                _ = tokio::time::sleep(Duration::from_secs(30)) => {},
            }
        }
    }
}

pub fn seed_pending_tags() {
    // Find source_data entries that don't have a tag queue entry yet
    let sql = "INSERT OR IGNORE INTO ai_tag_queue(source_type, source_id, status)
               SELECT 'notes', note_id, 0 FROM notes WHERE note_id IN (
                   SELECT source_id FROM source_data WHERE source_type='NOTE'
               ) AND note_id NOT IN (SELECT source_id FROM ai_tag_queue WHERE source_type='notes')
               UNION
               SELECT 'files', id, 0 FROM file_assets WHERE parse_status='COMPLETED'
                 AND id IN (SELECT source_id FROM source_data WHERE source_type='FILE')
                 AND id NOT IN (SELECT source_id FROM ai_tag_queue WHERE source_type='files')
               UNION
               SELECT 'products', id, 0 FROM products
                 WHERE id IN (SELECT source_id FROM source_data WHERE source_type='PRODUCT')
                 AND id NOT IN (SELECT source_id FROM ai_tag_queue WHERE source_type='products')
               UNION
               SELECT 'projects', id, 0 FROM projects
                 WHERE id IN (SELECT source_id FROM source_data WHERE source_type='PROJECT')
                 AND id NOT IN (SELECT source_id FROM ai_tag_queue WHERE source_type='projects')
               UNION
               SELECT 'todos', id, 0 FROM todos WHERE done=0
                 AND id IN (SELECT source_id FROM source_data WHERE source_type='TODO')
                 AND id NOT IN (SELECT source_id FROM ai_tag_queue WHERE source_type='todos')";
    match database::run(sql, &[]) {
        Ok(r) if r.changes > 0 => println!("[标签管线] 发现 {} 个待打标新记录", r.changes),
        _ => {}
    }
}

pub async fn process_tag_task() -> bool {
    let task = match database::get("SELECT * FROM ai_tag_queue WHERE status=0 ORDER BY id ASC LIMIT 1", &[]) { Ok(Some(t)) => t, _ => return false };
    let tid = task["id"].as_i64().unwrap_or(0);
    let st = task["source_type"].as_str().unwrap_or("");
    let sid = task["source_id"].as_str().unwrap_or("").to_string();
    let tagger = match lock_tag!(TAGGER_REGISTRY).get(st) { Some(t) => t.clone(), None => { database::run("UPDATE ai_tag_queue SET status=1,last_error='no tagger' WHERE id=?",&[json!(tid)]).ok(); return true; } };
    // Wrap tagger call with timeout so the task doesn't get stuck forever
    let tagger_result = match tokio::time::timeout(Duration::from_secs(120), tagger(sid.clone())).await {
        Ok(result) => result,
        Err(_) => {
            let e = "LLM 超时 (>120s)".to_string();
            eprintln!("[标签] LLM 超时: {st}:{sid}");
            let rc: i64 = database::get("SELECT retry_count FROM ai_tag_queue WHERE id=?",&[json!(tid)]).ok().flatten().and_then(|r| r["retry_count"].as_i64()).unwrap_or(0);
            let nr = rc+1;
            if nr>=3 { database::run("UPDATE ai_tag_queue SET status=-1,retry_count=?,last_error=? WHERE id=?",&[json!(nr),json!(&e[..e.len().min(200)]),json!(tid)]).ok(); }
            else { database::run("UPDATE ai_tag_queue SET retry_count=?,last_error=? WHERE id=?",&[json!(nr),json!(&e[..e.len().min(200)]),json!(tid)]).ok(); }
            return true;
        }
    };
    match tagger_result {
        Ok(tags) if !tags.is_empty() => {
            if let Err(e) = persist_tags_sync(st, &sid, &tags) { eprintln!("[标签] 持久化错误: {e}"); } else { println!("[标签] 已打标 {st}:{sid} -> {}", tags.join(", ")); }
            let _ = database::run("UPDATE ai_tag_queue SET status=1,last_error='',updated_at=datetime('now', 'localtime') WHERE id=?",&[json!(tid)]);
        }
        Ok(_) => { let _ = database::run("UPDATE ai_tag_queue SET status=1,last_error='LLM returned empty',updated_at=datetime('now', 'localtime') WHERE id=?",&[json!(tid)]); }
        Err(e) => {
            eprintln!("[标签] LLM 调用失败: {st}:{sid} — {e}");
            let rc: i64 = database::get("SELECT retry_count FROM ai_tag_queue WHERE id=?",&[json!(tid)]).ok().flatten().and_then(|r| r["retry_count"].as_i64()).unwrap_or(0); let nr = rc+1; if nr>=3 { database::run("UPDATE ai_tag_queue SET status=-1,retry_count=?,last_error=? WHERE id=?",&[json!(nr),json!(&e[..e.len().min(200)]),json!(tid)]).ok(); } else { database::run("UPDATE ai_tag_queue SET retry_count=?,last_error=? WHERE id=?",&[json!(nr),json!(&e[..e.len().min(200)]),json!(tid)]).ok(); } }
    }
    true
}

pub fn enqueue_tag_task(source_type: &str, source_id: &str) {
    database::run("INSERT OR IGNORE INTO ai_tag_queue(source_type,source_id,status) VALUES(?,?,0)",&[json!(source_type),json!(source_id)]).ok();
}

pub fn force_retag(source_type: &str, source_id: &str) {
    database::run("INSERT OR IGNORE INTO ai_tag_queue(source_type,source_id,status) VALUES(?,?,0)",&[json!(source_type),json!(source_id)]).ok();
    database::run("UPDATE ai_tag_queue SET status=0,retry_count=0,last_error='',updated_at=datetime('now', 'localtime') WHERE source_type=? AND source_id=?",&[json!(source_type),json!(source_id)]).ok();
}

pub fn trigger_tagging(source_type: &str, source_id: &str) {
    database::run("INSERT OR IGNORE INTO ai_tag_queue(source_type,source_id,status) VALUES(?,?,0)",&[json!(source_type),json!(source_id)]).ok();
}
