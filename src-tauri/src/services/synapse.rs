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
    #[allow(dead_code)]
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
        log::info!("[突触] 激活子块 {} 失败: {}", child_id, e);
    } else {
        GLOBAL_HITS.increment();
    }
}

fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}
