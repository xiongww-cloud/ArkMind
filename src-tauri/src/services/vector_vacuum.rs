//! VectorVacuum — LanceDB 孤儿向量低频自动回收
//!
//! 背景：embed_batch 纯追加插入（避免 LanceDB delete 全表重写），删 chunk 时仅清 vector_cache，
//! 孤儿向量靠查询时 JOIN 过滤。长期累积会拖垮存储与检索，故低频批量回收。
//!
//! 策略：
//!   - 每 24h 检查一次（last_vector_vacuum_check）
//!   - 触发条件：孤儿 ≥ 500 且占比 ≥ 20%
//!   - dry-run 保险开关（vector_vacuum_dry_run，默认开）：只扫描记日志不删除，
//!     验证孤儿判定无误后置 '0' 放行
//!   - last_vector_vacuum 仅在实际清理后更新（检查 ≠ 清理）

use serde::Serialize;
use std::collections::HashSet;

use crate::services::lancedb_client::LanceDBClient;

const CHECK_KEY: &str = "last_vector_vacuum_check";
const RUN_KEY: &str = "last_vector_vacuum";
const ORPHANS_KEY: &str = "last_vector_vacuum_orphans";
const DRY_RUN_KEY: &str = "vector_vacuum_dry_run";

const CHECK_INTERVAL_SECS: i64 = 24 * 3600;
const ORPHAN_MIN_COUNT: usize = 500;
const ORPHAN_MIN_RATIO: f64 = 0.20;

#[derive(Debug, Default, Clone, Serialize)]
pub struct VacuumReport {
    pub total_vectors: usize,
    pub orphan_count: usize,
    pub orphan_ratio: f64,
    pub triggered: bool,
    pub dry_run: bool,
    pub deleted: usize,
}

fn get_setting(key: &str) -> Option<String> {
    crate::database::get("SELECT value FROM system_settings WHERE key=?1", &[serde_json::json!(key)])
        .ok().flatten()
        .and_then(|r| r["value"].as_str().map(String::from))
}

fn set_setting(key: &str, value: &str) {
    let _ = crate::database::run(
        "INSERT OR REPLACE INTO system_settings(key, value, updated_at) VALUES(?1, ?2, datetime('now','localtime'))",
        &[serde_json::json!(key), serde_json::json!(value)],
    );
}

fn now_epoch() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// dry-run 开关：默认 false（实际删除生效——孤儿判定已被集成测试验证）；
/// 保留作保险丝：system_settings.vector_vacuum_dry_run='1' 时只扫描不删除。
pub fn dry_run_enabled() -> bool {
    get_setting(DRY_RUN_KEY).map(|v| v == "1").unwrap_or(false)
}

#[allow(dead_code)] // 保险丝开关的设置入口（预留给调试/未来设置项）
pub fn set_dry_run(enabled: bool) {
    set_setting(DRY_RUN_KEY, if enabled { "1" } else { "0" });
    log::info!("[向量回收] dry-run {}", if enabled { "已开启" } else { "已关闭（实际删除生效）" });
}

/// 触发判定：孤儿数量与占比双阈值（避免小数据量频繁全表重写）
pub fn should_vacuum(orphan_count: usize, total_vectors: usize) -> bool {
    if total_vectors == 0 { return false; }
    orphan_count >= ORPHAN_MIN_COUNT
        && (orphan_count as f64 / total_vectors as f64) >= ORPHAN_MIN_RATIO
}

fn load_valid_child_rowids() -> HashSet<i64> {
    crate::database::all("SELECT id FROM knowledge_chunks WHERE chunk_type='child'", &[])
        .unwrap_or_default()
        .iter()
        .filter_map(|r| r["id"].as_i64())
        .collect()
}

/// 核心逻辑（依赖注入，可测试）：扫描 → 判定 →（可选）删除
pub(crate) fn run_vacuum_core(client: &LanceDBClient, valid: &HashSet<i64>, dry_run: bool) -> Result<VacuumReport, String> {
    let all = client.scan_all_rowids()?;
    let total = all.len();
    let orphans: Vec<i64> = all.into_iter().filter(|id| !valid.contains(id)).collect();
    let orphan_count = orphans.len();
    let ratio = if total == 0 { 0.0 } else { orphan_count as f64 / total as f64 };
    let triggered = should_vacuum(orphan_count, total);

    let mut report = VacuumReport {
        total_vectors: total,
        orphan_count,
        orphan_ratio: ratio,
        triggered,
        dry_run,
        deleted: 0,
    };

    if triggered && !dry_run && orphan_count > 0 {
        // 二次校验：删除前重新确认孤儿集合与有效集合无交集
        let confirmed: Vec<i64> = orphans.into_iter().filter(|id| !valid.contains(id)).collect();
        client.delete_chunks_by_rowids(&confirmed)?;
        report.deleted = confirmed.len();
    }
    Ok(report)
}

/// 生产路径：全局 LanceDB + 全局 DB
pub fn run_vacuum(dry_run: bool) -> Result<VacuumReport, String> {
    let g = crate::services::lancedb_client::get_lancedb()?;
    let client = g.as_ref().ok_or("LanceDB 未初始化")?;
    let valid = load_valid_child_rowids();
    run_vacuum_core(client, &valid, dry_run)
}

/// embed_loop 每轮调用：距上次检查 ≥24h 才执行
pub fn maybe_run() {
    let last_check: i64 = get_setting(CHECK_KEY).and_then(|v| v.parse().ok()).unwrap_or(0);
    let now = now_epoch();
    if now - last_check < CHECK_INTERVAL_SECS {
        return;
    }
    set_setting(CHECK_KEY, &now.to_string());

    let dry_run = dry_run_enabled();
    match run_vacuum(dry_run) {
        Ok(report) => {
            set_setting(ORPHANS_KEY, &report.orphan_count.to_string());
            if report.triggered {
                if report.dry_run {
                    log::info!(
                        "[向量回收] dry-run：检测到 {} 个孤儿向量（占比 {:.1}%），未执行删除",
                        report.orphan_count, report.orphan_ratio * 100.0
                    );
                } else {
                    set_setting(RUN_KEY, &now.to_string());
                    log::info!(
                        "[向量回收] 已清理 {} 个孤儿向量（总量 {}，占比 {:.1}%）",
                        report.deleted, report.total_vectors, report.orphan_ratio * 100.0
                    );
                }
            } else {
                log::info!(
                    "[向量回收] 检查完成：孤儿 {} / 总量 {}（未达触发阈值）",
                    report.orphan_count, report.total_vectors
                );
            }
        }
        Err(e) => log::warn!("[向量回收] 检查失败（下周期重试）: {}", e),
    }
}

/// 状态查询（pipeline_debug_status 展示用）
pub fn vacuum_status() -> serde_json::Value {
    serde_json::json!({
        "last_check": get_setting(CHECK_KEY).unwrap_or_default(),
        "last_run": get_setting(RUN_KEY).unwrap_or_default(),
        "orphan_vectors": get_setting(ORPHANS_KEY).and_then(|v| v.parse::<i64>().ok()).unwrap_or(0),
        "dry_run": dry_run_enabled(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_vacuum_thresholds() {
        assert!(!should_vacuum(0, 0));
        assert!(!should_vacuum(499, 1000));      // 数量不足
        assert!(!should_vacuum(100, 10_000));    // 数量与占比均不足
        assert!(!should_vacuum(500, 10_000));    // 数量够但占比 5% < 20%
        assert!(should_vacuum(500, 1000));       // 50%
        assert!(should_vacuum(2000, 5000));      // 40%
        assert!(should_vacuum(200_000, 1_000_000)); // 大规模 20%
    }

    /// LanceDB 集成：插入 → 孤儿判定 → dry-run 不删 → 实际删除（验证 vacuum 依赖的原语）
    /// multi_thread runtime：与生产环境一致（sync_run 走 block_in_place 分支）
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_vacuum_core_with_real_lancedb() {
        let dir = tempfile::tempdir().unwrap();
        let client = LanceDBClient::new(&dir.path().to_string_lossy(), "test_vectors").unwrap();

        // 400 个有效 + 600 个孤儿（占比 60%，触发阈值）
        let valid: HashSet<i64> = (1..=400).collect();
        let all_ids: Vec<i64> = (1..=1000).collect();
        let vectors: Vec<Vec<f32>> = (0..1000).map(|i| vec![i as f32 * 0.001; 8]).collect();
        let empty: Vec<String> = vec![];
        client.insert_vectors_full(
            &all_ids, &vectors, &empty, &empty, &empty, &empty, &empty, &empty, &empty, &empty,
        ).unwrap();

        // dry-run：只报告不删除
        let report = run_vacuum_core(&client, &valid, true).unwrap();
        assert!(report.triggered);
        assert_eq!(report.orphan_count, 600);
        assert_eq!(report.deleted, 0);
        assert_eq!(client.scan_all_rowids().unwrap().len(), 1000);

        // 实际执行：孤儿清除、活向量保留
        let report = run_vacuum_core(&client, &valid, false).unwrap();
        assert_eq!(report.deleted, 600);
        let remaining: HashSet<i64> = client.scan_all_rowids().unwrap().into_iter().collect();
        assert_eq!(remaining.len(), 400);
        assert!(remaining.iter().all(|id| valid.contains(id)));

        // 再次运行：无孤儿，不触发
        let report = run_vacuum_core(&client, &valid, false).unwrap();
        assert_eq!(report.orphan_count, 0);
        assert!(!report.triggered);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_vacuum_core_below_threshold_no_delete() {
        let dir = tempfile::tempdir().unwrap();
        let client = LanceDBClient::new(&dir.path().to_string_lossy(), "test_vectors2").unwrap();

        let valid: HashSet<i64> = HashSet::new();
        let ids: Vec<i64> = (1..=100).collect(); // 100 个孤儿 < 500，不触发
        let vectors: Vec<Vec<f32>> = (0..100).map(|i| vec![i as f32; 8]).collect();
        let empty: Vec<String> = vec![];
        client.insert_vectors_full(&ids, &vectors, &empty, &empty, &empty, &empty, &empty, &empty, &empty, &empty).unwrap();

        let report = run_vacuum_core(&client, &valid, false).unwrap();
        assert!(!report.triggered);
        assert_eq!(report.deleted, 0);
        assert_eq!(client.scan_all_rowids().unwrap().len(), 100);
    }
}
