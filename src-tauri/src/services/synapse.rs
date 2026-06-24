//! Synapse — neural energy / lazy decay / activation
//!
//! Architecture:
//!   - Chunk weight (knowledge_chunks):
//!     - Lazy decay: effective = base_weight × 0.5^(Δt/30d), O(1), zero I/O
//!     - Activation: new_base = decayed + (5.0 - decayed) × 0.3, asymptotically approaches MAX=5.0
//!     - RRF fusion multiplier: 1.0 + ln(w + 1.0) — bounded by ~2.79 at w=5.0
//!     - New chunks start at base_weight = 5.0 (top priority)
//!
//! Grain: child chunks only (parents don't participate in retrieval).

use crate::database;
use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};

// ═══════════════════════════════════════════════════════════════════
// Global constants
// ═══════════════════════════════════════════════════════════════════

/// Half-life in days — after 30 days of inactivity, weight decays to 0.5
const HALF_LIFE_DAYS: f64 = 30.0;

/// Gamma coefficient: γ = ln(2) / (half_life_days * 86400)
const GAMMA: f64 = 0.69314718056 / (HALF_LIFE_DAYS * 86400.0);

/// Maximum base weight — newly created chunks start here, activated chunks asymptotically approach here
const MAX_BASE_WEIGHT: f64 = 5.0;

/// Activation pull ratio R — how fast a chunk recovers toward MAX when adopted
/// Formula: new_base = decayed + (MAX_BASE_WEIGHT - decayed) × ACTIVATION_R
const ACTIVATION_R: f64 = 0.3;

pub struct GlobalHitCounter {
    count: std::sync::atomic::AtomicU64,
}
impl GlobalHitCounter {
    pub const fn new() -> Self {
        Self { count: std::sync::atomic::AtomicU64::new(0) }
    }
    pub fn increment(&self) {
        self.count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    pub fn take(&self) -> u64 {
        self.count.swap(0, std::sync::atomic::Ordering::Relaxed)
    }
}

pub static GLOBAL_HITS: GlobalHitCounter = GlobalHitCounter::new();

// ═══════════════════════════════════════════════════════════════════
// Lazy decay — pure function, O(1), zero I/O
// ═══════════════════════════════════════════════════════════════════

/// Compute the effective weight of a child chunk at retrieval time.
/// Formula: effective = base_weight * exp(-γ * Δt_secs)
pub fn effective_weight(child_id: i64) -> f64 {
    let row = match database::get(
        "SELECT base_weight, last_access_at FROM knowledge_chunks WHERE id=?1 AND chunk_type='child'",
        &[json!(child_id)],
    ) {
        Ok(Some(r)) => r,
        _ => return 1.0,
    };
    let base = row["base_weight"].as_f64().unwrap_or(1.0);
    let last_access = match row["last_access_at"].as_i64() {
        Some(ts) if ts > 0 => ts,
        _ => return base,
    };
    let now = now_secs() as i64;
    let delta = (now - last_access) as f64;
    if delta <= 0.0 {
        return base;
    }
    base * (-GAMMA * delta).exp()
}

/// RRF fusion multiplier: (1.0 + ln(effective_weight + 1.0))
/// Smoothly boosts hot chunks without letting any single chunk dominate.
pub fn rrf_weight_multiplier(child_id: i64) -> f64 {
    let w = effective_weight(child_id);
    1.0 + (w + 1.0).ln() * 0.5
}

// ═══════════════════════════════════════════════════════════════════
// Activation — write-back when a chunk's parent is adopted
// ═══════════════════════════════════════════════════════════════════

/// Activate a child chunk (called when its parent block is adopted into LLM context).
/// 1. Apply time decay from last access → now:  decayed = base × 0.5^(Δt/30d)
/// 2. Lerp toward MAX:  new_base = decayed + (MAX - decayed) × R
pub fn activate_child_chunk(child_id: i64) {
    let now = now_secs() as i64;
    let row = match database::get(
        "SELECT base_weight, last_access_at, access_count FROM knowledge_chunks WHERE id=?1 AND chunk_type='child'",
        &[json!(child_id)],
    ) {
        Ok(Some(r)) => r,
        _ => return,
    };

    let base = row["base_weight"].as_f64().unwrap_or(1.0);
    let last_access = row["last_access_at"].as_i64().unwrap_or(now);
    let count = row["access_count"].as_i64().unwrap_or(0);

    // Apply time decay, then lerp toward MAX
    let delta = ((now - last_access) as f64).max(0.0);
    let decayed = base * (-GAMMA * delta).exp();
    let new_base = decayed + (MAX_BASE_WEIGHT - decayed) * ACTIVATION_R;

    if let Err(e) = database::run(
        "UPDATE knowledge_chunks SET base_weight=?1, last_access_at=?2, access_count=?3 WHERE id=?4 AND chunk_type='child'",
        &[json!(new_base), json!(now), json!(count + 1), json!(child_id)],
    ) {
        eprintln!("[突触] 激活子块 {} 失败: {}", child_id, e);
    } else {
        GLOBAL_HITS.increment();
    }
}

// ═══════════════════════════════════════════════════════════════════
// Object Links — Hebbian object co-activation (Object-level Memory)
// ═══════════════════════════════════════════════════════════════════

/// 确保 object_links 表存在（在 database::init 启动时调用）
pub fn init_object_links() {
    let _ = database::run(
        "CREATE TABLE IF NOT EXISTS object_links (
            source_id TEXT NOT NULL,
            target_id TEXT NOT NULL,
            weight REAL DEFAULT 1.0,
            last_seen TEXT NOT NULL,
            PRIMARY KEY(source_id, target_id)
        )",
        &[],
    );
    let _ = database::run("CREATE INDEX IF NOT EXISTS idx_ol_source ON object_links(source_id)", &[]);
    let _ = database::run("CREATE INDEX IF NOT EXISTS idx_ol_target ON object_links(target_id)", &[]);
}

/// 共同激活：将本轮被 LLM 采纳的 object_ids 建立 Hebbian 链接
pub fn update_object_links(object_ids: &[String]) {
    let mut unique: Vec<String> = object_ids.iter().cloned().collect();
    unique.sort();
    unique.dedup();
    let n = unique.len();
    if n < 2 { return; }
    for i in 0..n {
        for j in (i+1)..n {
            let (a, b) = if unique[i] < unique[j] {
                (&unique[i], &unique[j])
            } else {
                (&unique[j], &unique[i])
            };
            let _ = database::run(
                "INSERT INTO object_links(source_id,target_id,weight,last_seen) \
                 VALUES(?1,?2,1.0,datetime('now','localtime')) \
                 ON CONFLICT(source_id,target_id) DO UPDATE SET \
                 weight = weight + 0.3 * (5.0 - weight), \
                 last_seen = excluded.last_seen",
                &[json!(a), json!(b)],
            );
        }
    }
}

/// 从种子 object_id 扩散邻居，按衰减权重排序
pub fn expand_object_links(seed_ids: &[String], limit: usize) -> Vec<(String, f64)> {
    if seed_ids.is_empty() { return vec![]; }
    let ph: Vec<String> = seed_ids.iter().map(|_| "?".to_string()).collect();
    let mut params: Vec<serde_json::Value> = seed_ids.iter().map(|c| json!(c)).collect();
    params.extend(seed_ids.iter().map(|c| json!(c)));
    params.push(json!(limit as i64));
    let sql = format!(
        "SELECT neighbor, eff FROM (
            SELECT target_id as neighbor, weight * EXP(-0.01 * (julianday('now') - julianday(last_seen))) as eff \
             FROM object_links WHERE source_id IN ({})
            UNION ALL
            SELECT source_id as neighbor, weight * EXP(-0.01 * (julianday('now') - julianday(last_seen))) as eff \
             FROM object_links WHERE target_id IN ({})
        ) ORDER BY eff DESC LIMIT ?",
        ph.join(","), ph.join(",")
    );
    database::all(&sql, &params)
        .unwrap_or_default()
        .iter()
        .filter_map(|r| {
            let id = r["neighbor"].as_str().unwrap_or("").to_string();
            let w = r["eff"].as_f64().unwrap_or(0.0);
            if id.is_empty() || w <= 0.0 { None } else { Some((id, w)) }
        })
        .collect()
}

// ═══════════════════════════════════════════════════════════════════
// now_secs helper
// ═══════════════════════════════════════════════════════════════════

fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

// ═══════════════════════════════════════════════════════════════════
// Project risk audit
// ═══════════════════════════════════════════════════════════════════

pub async fn audit_project_risk(project_id: &str) -> Result<(), String> {
    let rows = database::all(
        "SELECT kc.content, kc.tags, kc.source_data_id
         FROM knowledge_chunks kc
         WHERE kc.source_data_id IN (
           SELECT sd.id FROM source_data sd
           WHERE sd.source_type='PROJECT' AND sd.source_id IN (
             SELECT fr.entity_id FROM file_relations fr WHERE fr.entity_type='project' AND fr.entity_id = ?
           )
         )
         ORDER BY kc.created_at DESC
         LIMIT 50",
        &[json!(project_id)],
    )
    .map_err(|e| format!("audit query failed: {}", e))?;

    if rows.len() < 10 {
        return Ok(());
    }

    let mut context = String::new();
    for (i, row) in rows.iter().enumerate() {
        let content = row["content"].as_str().unwrap_or("");
        let tags = row["tags"].as_str().unwrap_or("");
        context.push_str(&format!("[{i}] {tags}\n{content}\n\n"));
        if context.len() > 8000 {
            break;
        }
    }

    let system = format!(
        "你是一个项目管理审计员。请检查以下项目最新知识片段，识别潜在的冲突、矛盾或风险。
         只输出 JSON 数组，每个元素为：{{\"level\":\"warning|info|risk\",\"title\":\"简短标题\",\"detail\":\"详细说明（≤50字）\"}}。
         无问题时输出空数组 []。不要输出其他任何文字。"
    );

    let config = crate::services::chat_api::get_chat_config();
    let raw = crate::services::chat_api::chat_completion(
        "只输出 JSON 数组",
        &format!("{} 项目片段：{}", system, context),
        Some(crate::services::chat_api::ChatConfig {
            temperature: 0.0,
            max_tokens: 1024,
            ..config
        }),
    )
    .await
    .map_err(|e| format!("audit LLM call failed: {}", e))?;

    let trimmed = raw.trim().trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```").trim();
    if let Ok(notifications) = serde_json::from_str::<Vec<serde_json::Value>>(trimmed) {
        for n in &notifications {
            let level = n["level"].as_str().unwrap_or("info");
            let title = n["title"].as_str().unwrap_or("审计发现");
            let detail = n["detail"].as_str().unwrap_or("");
            let payload = json!({
                "project_id": project_id,
                "level": level,
                "title": title,
                "detail": detail,
                "source": "ai_audit",
            });
            let _ = database::run(
                "INSERT INTO system_notifications (notification_type, project_id, payload) VALUES ('ai_audit', ?, ?)",
                &[json!(project_id), json!(payload.to_string())],
            );
        }
        if !notifications.is_empty() {
            println!(
                "[突触] 项目风险审计: 为项目 {} 生成 {} 条通知",
                project_id,
                notifications.len()
            );
        }
    }

    Ok(())
}
