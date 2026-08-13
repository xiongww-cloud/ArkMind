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

/// 纯函数：惰性衰减。effective = base × exp(−γΔt)，Δt ≤ 0 时返回 base
pub(crate) fn decayed_weight(base: f64, delta_secs: f64) -> f64 {
    if delta_secs <= 0.0 { return base; }
    base * (-GAMMA * delta_secs).exp()
}

/// 纯函数：激活回升。new_base = decayed + (MAX − decayed) × R（渐近逼近 MAX，不越界）
pub(crate) fn activation_next(decayed: f64) -> f64 {
    decayed + (MAX_BASE_WEIGHT - decayed) * ACTIVATION_R
}

/// 纯函数：RRF 融合乘子。1 + ln(w+1) × 0.5（w=5 时约 1.9）
pub(crate) fn rrf_multiplier_of(w: f64) -> f64 {
    1.0 + (w + 1.0).ln() * 0.5
}

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
    decayed_weight(base, (now - last_access) as f64)
}

/// RRF fusion multiplier: (1.0 + ln(effective_weight + 1.0))
/// Smoothly boosts hot chunks without letting any single chunk dominate.
pub fn rrf_weight_multiplier(child_id: i64) -> f64 {
    rrf_multiplier_of(effective_weight(child_id))
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
    let decayed = decayed_weight(base, delta);
    let new_base = activation_next(decayed);

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

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: f64 = 86400.0;

    #[test]
    fn test_decay_half_life_30_days() {
        // 30 天半衰期：恰好衰减到一半（锁定实现=公式）
        let w = decayed_weight(4.0, 30.0 * DAY);
        assert!((w - 2.0).abs() < 1e-6, "30 天后应为一半, got {}", w);
    }

    #[test]
    fn test_decay_zero_and_negative_delta() {
        assert_eq!(decayed_weight(4.0, 0.0), 4.0);
        assert_eq!(decayed_weight(4.0, -100.0), 4.0);
    }

    #[test]
    fn test_decay_monotonic() {
        let w1 = decayed_weight(5.0, 10.0 * DAY);
        let w2 = decayed_weight(5.0, 20.0 * DAY);
        let w3 = decayed_weight(5.0, 60.0 * DAY);
        assert!(w1 > w2 && w2 > w3);
        // 60 天 ≈ 两个半衰期 → 约 1/4
        assert!((w3 - 1.25).abs() < 1e-6, "60 天后应为 1/4, got {}", w3);
    }

    #[test]
    fn test_activation_asymptotic_to_max() {
        // 从 0 反复激活，渐近逼近 MAX=5.0 且不越界
        let mut w = 0.0;
        for _ in 0..200 {
            w = activation_next(w);
            assert!(w <= MAX_BASE_WEIGHT, "激活不得越过 MAX");
        }
        assert!((w - MAX_BASE_WEIGHT).abs() < 1e-6);
    }

    #[test]
    fn test_activation_recovers_decayed_weight() {
        // 衰减到 2.0 的块被激活后应回升：2.0 + (5-2)×0.3 = 2.9
        let w = activation_next(2.0);
        assert!((w - 2.9).abs() < 1e-9);
    }

    #[test]
    fn test_rrf_multiplier_reference_values() {
        // w=5 → 1 + ln(6)×0.5 ≈ 1.896
        assert!((rrf_multiplier_of(5.0) - 1.8959).abs() < 1e-3);
        // w=0 → 1.0（无加权）
        assert_eq!(rrf_multiplier_of(0.0), 1.0);
        // 单调递增
        assert!(rrf_multiplier_of(4.0) > rrf_multiplier_of(2.0));
    }
}
