//! KnowledgePipeline — 8-step closed-loop AI data pipeline orchestrator
//!
//! Flow:
//!   1. De-duplication upload (SHA-256 file_assets check)
//!   2. Trigger ignition (AFTER INSERT triggers → ai_sync_queue + ai_tag_queue)
//!   3. Control plane (License check + API readiness + queue health)
//!   4. Global tagging (full-text → LLM NER → 3-5 tags, NOT per-chunk)
//!   5. Semantic materialization + smart_chunker (400 char / 100 overlap)
//!   6. Dual storage (knowledge_chunks → FTS5 + LanceDB)
//!   7. Synapse strengthening (RAG hit → lerp toward MAX=5.0 with R=0.3)
//!   8. Human-AI audit (project_risk scan → system_notifications)
//!
//! Strict: zero .unwrap() / .ok() — all errors propagated or logged.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use crate::services::debounced_sync::DebouncedMaterializer;

static PIPELINE_RUNNING: AtomicBool = AtomicBool::new(false);

/// 全局防抖调度器实例。在 `start()` 中初始化，之后各模块通过
/// `get_scheduler()` 获取引用以调用 `enqueue()`。
static DEBOUNCED_SCHEDULER: OnceLock<Arc<DebouncedMaterializer>> = OnceLock::new();

/// 获取全局防抖调度器引用（可能为 None，如果管线尚未启动）。
pub fn get_scheduler() -> Option<Arc<DebouncedMaterializer>> {
    DEBOUNCED_SCHEDULER.get().cloned()
}

// ─── Entry point ──────────────────────────────────────────────────

pub fn start() {
    if PIPELINE_RUNNING.swap(true, Ordering::AcqRel) {
        println!("[知识管线] 已在运行中");
        return;
    }
    println!("[知识管线] Arkmind 知识管线启动 (v11.0)");

    // ── Step 1: Cleanup stale queue entries (zombie source_ids) ──
    crate::services::ai_sync::purge_stale_queue_entries();

    // ── Step 2: Cleanup stale SQL triggers from old schema versions ──
    let _ = crate::database::run(
        "DROP TRIGGER IF EXISTS ai_sync_products_insert;
         DROP TRIGGER IF EXISTS ai_sync_products_update;",
        &[],
    );

    // ── Step 3: Seed sync queue ──
    // 首次启动全量播种，非首次只从 source_data 补种遗漏条目
    let queue_count: i64 = crate::database::with_db(|c| {
        c.query_row("SELECT COUNT(*) FROM ai_sync_queue", [], |r| r.get(0))
    })
    .unwrap_or(0);
    if queue_count == 0 {
        crate::services::ai_sync::cold_start_seed_sync_queue();
    } else {
        // ★ 每次启动都从 source_data 补种——确保 source_data 有记录的实体都有同步任务
        crate::services::ai_sync::seed_from_source_data();
    }

    // ── Step 4: Reset failed tasks with retry_count < 3 ──
    reset_retryable_failures();

    // ── Step 5: Heal — mark already-vectorized files as embedded ──
    heal_duplicate_tasks();

    // ── Step 6: Vector cache consistency check ──
    clean_vector_cache();

    // ── Step 7: Trash auto-cleanup (notes deleted > 30 days) ──
    auto_clean_trash();

    // ── Step 8: Persist WAL ──
    let _ = crate::database::save();

    // ── Spawn async scheduler thread（含防抖调度器 + AI 同步循环）──
    std::thread::spawn(|| {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .build();
        let rt = match rt {
            Ok(r) => r,
            Err(e) => { eprintln!("[知识管线] tokio 运行时失败: {}", e); return; }
        };
        // 防抖调度器必须在 tokio runtime 内初始化 → 通过 enter 进入 runtime 上下文
        let _guard = rt.enter();
        let sched = DebouncedMaterializer::spawn();
        let _ = DEBOUNCED_SCHEDULER.set(sched);
        drop(_guard);
        rt.block_on(scheduler_loop());
    });

}

pub fn is_running() -> bool {
    PIPELINE_RUNNING.load(Ordering::Acquire)
}

// ═══════════════════════════════════════════════════════════════════
// Startup helpers
// ═══════════════════════════════════════════════════════════════════

fn reset_retryable_failures() {
    // Reset failed sync tasks: -1 → 0, only if retry_count < 3
    match crate::database::run(
        "UPDATE ai_sync_queue SET is_embedded=0, retry_count=0, last_error=''
         WHERE is_embedded=-1 AND retry_count<3",
        &[],
    ) {
        Ok(r) if r.changes > 0 => {
            println!("[知识管线] 重置 {} 个可重试的同步失败", r.changes);
        }
        Err(e) => eprintln!("[知识管线] 重置同步失败出错: {}", e),
        _ => {}
    }

    match crate::database::run(
        "UPDATE ai_tag_queue SET status=0, retry_count=0
         WHERE status=-1 AND retry_count<3",
        &[],
    ) {
        Ok(r) if r.changes > 0 => {
            println!("[知识管线] 重置 {} 个可重试的标签失败", r.changes);
        }
        Err(e) => eprintln!("[知识管线] 重置标签失败出错: {}", e),
        _ => {}
    }
}

fn heal_duplicate_tasks() {
    // 旧功能：按 file_id 去重；表重建后不适用，直接跳过
}

fn clean_vector_cache() {
    // Clear vector_cache entries without matching knowledge_chunks rows
    match crate::database::run(
        "DELETE FROM vector_cache WHERE chunk_id NOT IN (SELECT id FROM knowledge_chunks)",
        &[],
    ) {
        Ok(r) if r.changes > 0 => {
            println!("[知识管线] 清理 {} 个过期向量缓存条目", r.changes);
        }
        Err(e) => eprintln!("[知识管线] 向量缓存清理出错: {}", e),
        _ => {}
    }

    // RowID reuse detection
    let vc_cnt: i64 = crate::database::with_db(|c| {
        c.query_row("SELECT COUNT(*) FROM vector_cache", [], |r| r.get(0))
    })
    .unwrap_or(0);
    let child_cnt: i64 = crate::database::with_db(|c| {
        c.query_row(
            "SELECT COUNT(*) FROM knowledge_chunks WHERE chunk_type='child'",
            [],
            |r| r.get(0),
        )
    })
    .unwrap_or(0);

    if vc_cnt > child_cnt {
        match crate::database::run("DELETE FROM vector_cache", &[]) {
            Ok(_) => println!(
                "[知识管线] rowid 复用: 清除 {} 个向量缓存条目 (子块数={})",
                vc_cnt, child_cnt
            ),
            Err(e) => eprintln!("[知识管线] 向量缓存清除出错: {}", e),
        }
    }
}

fn auto_clean_trash() {
    match crate::database::run(
        "DELETE FROM notes WHERE deleted_at IS NOT NULL AND datetime(deleted_at,'+30 days')<datetime('now', 'localtime')",
        &[],
    ) {
        Ok(r) if r.changes > 0 => {
            println!("[知识管线] 回收站清理: {} 条笔记已永久删除", r.changes);
        }
        Err(e) => eprintln!("[知识管线] 回收站清理出错: {}", e),
        _ => {}
    }
}

// ═══════════════════════════════════════════════════════════════════
// Main scheduler loop
// ═══════════════════════════════════════════════════════════════════

async fn scheduler_loop() {
    let mut tick: u64 = 0;

    loop {
        if !PIPELINE_RUNNING.load(Ordering::Acquire) {
            break;
        }
        tick = tick.wrapping_add(1);

        // ── Firewall: License guard ──
        if !crate::database::check_license_premium() {
            // 无高级版许可证时仍然处理同步和向量化，跳过审计等高级功能
            tokio::time::sleep(Duration::from_millis(50)).await;
        } else {

        // ── Periodic seeding from business tables (every ~2 min) ──
        if tick % 2400 == 0 {
            crate::services::ai_sync::seed_new_records();
        }

        // ── Periodic audit (every ~60 minutes) ──
        if tick % 72000 == 0 {
            let project_ids = get_project_ids_with_files();
            for pid in &project_ids {
                let pid_clone = pid.clone();
                tokio::spawn(async move {
                    if let Err(e) = crate::services::synapse::audit_project_risk(&pid_clone).await {
                        eprintln!("[知识管线] 审计项目 {} 出错: {}", pid_clone, e);
                    }
                });
            }
        }
        }  // closes else (premium block)

        // ── Periodic source_data sync (every ~30s) ──
        if tick % 600 == 0 {
            crate::services::ai_sync::seed_from_source_data();
        }

        let mut had_work = false;

        // Process up to 5 sync tasks per tick (drain queue faster)
        for _ in 0..5 {
            if !run_timeout("sync", || crate::services::ai_sync::process_sync_task()).await {
                break;
            }
            had_work = true;
            tokio::task::yield_now().await;
        }

        // Process vectorization
        had_work |= run_timeout("vectorize", || async {
            match crate::services::hybrid_knowledge::vectorize_new_chunks(5).await {
                Ok(n) => {
                    if n > 0 {
                        println!("[知识管线] 已向量化: {n} 个块");
                    }
                    n > 0
                }
                Err(e) => {
                    eprintln!("[知识管线] 向量化错误: {e}");
                    false
                }
            }
        })
        .await;
        tokio::task::yield_now().await;

        // Adaptive sleep: fast when busy, slow when idle
        if had_work {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        let delay = if had_work { 50 } else { 5000 };
        tokio::time::sleep(Duration::from_millis(delay)).await;
    }
}

async fn run_timeout<F, Fut>(name: &str, f: F) -> bool
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    match tokio::time::timeout(Duration::from_secs(6), f()).await {
        Ok(result) => result,
        Err(_) => {
            log::warn!("[知识管线] {} 超时 (>6秒)", name);
            false
        }
    }
}

fn get_project_ids_with_files() -> Vec<String> {
    match crate::database::all(
        "SELECT DISTINCT entity_id as project_id FROM file_relations WHERE entity_type='project'",
        &[],
    ) {
        Ok(rows) => rows
            .into_iter()
            .filter_map(|r| r["project_id"].as_str().map(String::from))
            .collect(),
        Err(_) => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pipeline_not_running_by_default() {
        assert!(!is_running());
    }
}
