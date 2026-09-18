//! 防抖 AI 物化调度器 (Debounced Materializer)
//!
//! 问题背景：
//!   前端 Tiptap 富文本编辑器以 600ms 防抖自动保存，用户快速打字时
//!   每个保存周期都会触发 `source_data` 写入 → AI pipeline 重算。
//!   在本地桌面端，这会导致 CPU 被反复榨干。
//!
//! 解决方案：
//!   在 `ai_sync_queue` 之上引入一层内存态防抖调度器：
//!   - 同一 (source_type, source_id) 的连续写入合并为一个 5 秒静默窗口
//!   - 窗口内重复写入仅重置倒计时，不创建新的 tokio 任务
//!   - 正在 PROCESSING 的实体加锁，后续写入排队到下一个周期
//!
//! 并发模型：
//!   - DashMap<(String,String), DebounceState> 提供无锁读取
//!   - 单线程 tokio worker 集中管理定时器和触发逻辑
//!   - WAL 模式保证写入与读取无冲突

use dashmap::DashMap;
use once_cell::sync::Lazy;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, Semaphore};

/// 全局防抖物化调度器实例（在 Tauri setup 中显式 `Lazy::force` 初始化，
/// 确保 tokio runtime 就绪时调用 spawn()）
pub static SCHEDULER: Lazy<Arc<DebouncedMaterializer>> =
    Lazy::new(|| DebouncedMaterializer::spawn());

// ═══════════════════════════════════════════════════════════════════
// Constants
// ═══════════════════════════════════════════════════════════════════

/// 防抖窗口：此时间内连续写入不触发物化
const DEBOUNCE_WINDOW_SECS: u64 = 5;

/// 最大并发物化数。SQLite 全局 Mutex 串行化所有 DB 操作，
/// 此处限制避免过多任务同时排队等待锁。
const MAX_CONCURRENT_MATERIALIZATIONS: usize = 4;

/// ─── 物化债务表（materialize_retry）：重试耗尽后落库持久补偿，杜绝永久缺失 ───

fn now_secs() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// 记债/加债：退避 10min×n 封顶 2h；超过 50 次降为每日一次（不再静默丢弃）
pub fn record_materialize_debt(source_type: &str, source_id: &str) {
    let prev: i64 = crate::database::get(
        "SELECT retry_count FROM materialize_retry WHERE source_type=?1 AND source_id=?2",
        &[serde_json::json!(source_type), serde_json::json!(source_id)],
    ).ok().flatten().and_then(|r| r["retry_count"].as_i64()).unwrap_or(0);
    let n = prev + 1;
    let delay = if n > 50 { 86_400 } else { (600 * n).min(7_200) };
    let r = crate::database::run(
        "INSERT OR REPLACE INTO materialize_retry(source_type, source_id, retry_count, next_at) VALUES(?1, ?2, ?3, ?4)",
        &[serde_json::json!(source_type), serde_json::json!(source_id), serde_json::json!(n), serde_json::json!(now_secs() + delay)],
    );
    if r.is_ok() {
        log::warn!("[防抖调度器] 物化失败已记债(第{}次，{}s后补偿): {}:{}", n, delay, source_type, source_id);
    }
}

/// 销债：物化成功后清理
pub fn clear_materialize_debt(source_type: &str, source_id: &str) {
    let _ = crate::database::run(
        "DELETE FROM materialize_retry WHERE source_type=?1 AND source_id=?2",
        &[serde_json::json!(source_type), serde_json::json!(source_id)],
    );
}

/// 单实体物化最大重试次数（失败后按退避间隔重试）
const MAX_MATERIALIZE_RETRIES: u32 = 5;

/// 物化失败后的退避重试间隔
const RETRY_BACKOFF_SECS: u64 = 30;

/// 重试跟踪队列上限（O9：防止海量失败实体导致内存无界增长；
/// 超限放弃的实体由 sync_loop 兜底重新入队）
const MAX_RETRY_TRACKED: usize = 10000;

// ═══════════════════════════════════════════════════════════════════
// Types
// ═══════════════════════════════════════════════════════════════════

/// 实体键：唯一标识一个待物化的业务实体
type EntityKey = (String, String); // (source_type, source_id)

/// 每个实体的防抖/处理状态
#[derive(Debug, Clone)]
enum DebounceState {
    /// 静默等待中，携带下次到期时间戳。
    /// 在此期间收到的重复写入仅更新 `Instant` 而不会触发物化。
    Pending(Instant),

    /// 正在执行物化管线（切片 + FTS5 + LanceDB）。
    /// O5: dirty=true 表示物化期间收到新的写入，完成后需立即重新入队物化最新内容
    Processing { dirty: bool },
}

/// 控制通道消息
enum ControlMsg {
    /// 入队或重置倒计时
    Enqueue,
    /// 优雅关闭
    #[allow(dead_code)]
    Shutdown,
}

// ═══════════════════════════════════════════════════════════════════
// Scheduler
// ═══════════════════════════════════════════════════════════════════

/// 防抖 AI 物化调度器。
///
/// 使用方法：
/// ```ignore
/// let scheduler = DebouncedMaterializer::spawn();
/// scheduler.enqueue("notes", "some-note-uuid");
/// ```
///
/// 线程安全，可在任意上下文中调用 `enqueue()`。
pub struct DebouncedMaterializer {
    /// 无锁并发状态映射。DashMap 的分片锁机制使得 enqueue() 的
    /// 快速路径（检查键是否存在）几乎无竞争。
    state: Arc<DashMap<EntityKey, DebounceState>>,

    /// 控制通道发送端。clone 后可在多线程中共享。
    tx: mpsc::UnboundedSender<ControlMsg>,
}

impl DebouncedMaterializer {
    /// 在任意上下文中启动后台 worker：
    /// - 已有 tokio runtime 上下文（如单元测试）→ 直接用当前 handle
    /// - 无 tokio 上下文（如 Tauri setup 主线程）→ 用 tauri async runtime
    fn spawn_worker<F>(future: F)
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn(future);
            }
            Err(_) => {
                tauri::async_runtime::spawn(future);
            }
        }
    }

    /// 创建调度器并启动后台 worker。
    ///
    /// worker 运行在独立的 tokio 任务中，由调用方的 tokio runtime 驱动。
    /// 建议在 Tauri `setup` 阶段调用，确保 tokio runtime 已就绪。
    pub fn spawn() -> Arc<Self> {
        let state: Arc<DashMap<EntityKey, DebounceState>> = Arc::new(DashMap::new());
        let (tx, rx) = mpsc::unbounded_channel();
        let materialize_permits = Arc::new(Semaphore::new(MAX_CONCURRENT_MATERIALIZATIONS));
        let retries: Arc<DashMap<EntityKey, u32>> = Arc::new(DashMap::new());

        let worker_state = state.clone();
        let worker_permits = materialize_permits;
        let worker_retries = retries;
        let worker_tx = tx.clone();
        Self::spawn_worker(async move {
            Self::worker_loop(worker_state, worker_permits, worker_retries, worker_tx, rx).await;
        });

        let sched = Self { state, tx };
        log::info!(
            "[防抖调度器] 已启动 (窗口={}s, 并发={})",
            DEBOUNCE_WINDOW_SECS, MAX_CONCURRENT_MATERIALIZATIONS
        );
        Arc::new(sched)
    }

    /// 入队一个实体用于异步物化。
    ///
    /// 幂等语义：
    ///   - 首次入队 → 插入 `Pending(now + 5s)`，通知 worker
    ///   - 重复入队（仍在 Pending）→ 更新 deadline 为 `now + 5s`，重置倒计时
    ///   - 重复入队（正在 Processing）→ O5: 标记 dirty，物化完成后立即重新入队
    ///
    /// 调用方无需关心返回值 — 入队操作是 fire-and-forget。
    pub fn enqueue(&self, source_type: &str, source_id: &str) {
        let key: EntityKey = (source_type.to_string(), source_id.to_string());
        let now = Instant::now();
        let deadline = now + Duration::from_secs(DEBOUNCE_WINDOW_SECS);

        // ── 快速路径：更新 DashMap ──
        match self.state.get_mut(&key) {
            Some(mut entry) => {
                match *entry {
                    DebounceState::Pending(_) => {
                        // ★ Upsert 核心：重置倒计时，不创建新任务
                        *entry = DebounceState::Pending(deadline);
                        // 通知 worker 重新计算最早截止时间
                        let _ = self.tx.send(ControlMsg::Enqueue);
                    }
                    DebounceState::Processing { .. } => {
                        // O5: 物化期间收到新写入 → 标记 dirty，完成后立即重新物化最新内容
                        *entry = DebounceState::Processing { dirty: true };
                    }
                }
            }
            None => {
                self.state
                    .insert(key.clone(), DebounceState::Pending(deadline));
                let _ = self.tx.send(ControlMsg::Enqueue);
            }
        }
    }

    /// 优雅关闭调度器及其 worker。
    #[allow(dead_code)]
    pub fn shutdown(&self) {
        let _ = self.tx.send(ControlMsg::Shutdown);
    }

    /// 获取当前跟踪的实体数量（调试用）。
    #[allow(dead_code)]
    pub fn pending_count(&self) -> usize {
        self.state.len()
    }

    /// 检查实体是否已在调度器跟踪中（Pending 或 Processing）。
    /// 供 sync_loop 兜底 enqueue 时跳过已入队实体，避免重置其防抖窗口。
    pub fn is_tracked(&self, source_type: &str, source_id: &str) -> bool {
        self.state.contains_key(&(source_type.to_string(), source_id.to_string()))
    }

    // ═══════════════════════════════════════════════════════════════
    // Worker internals
    // ═══════════════════════════════════════════════════════════════

    /// 单线程事件循环：扫描过期任务、派生物化、等待下一截止时间。
    async fn worker_loop(
        state: Arc<DashMap<EntityKey, DebounceState>>,
        permits: Arc<Semaphore>,
        retries: Arc<DashMap<EntityKey, u32>>,
        tx: mpsc::UnboundedSender<ControlMsg>,
        mut rx: mpsc::UnboundedReceiver<ControlMsg>,
    ) {
        loop {
            let now = Instant::now();
            let mut next_deadline: Option<Instant> = None;
            let mut due_keys: Vec<EntityKey> = Vec::new();

            // ── 扫描所有 PENDING 实体，找出已到期的 ──
            for entry in state.iter() {
                if let DebounceState::Pending(deadline) = *entry.value() {
                    if deadline <= now {
                        due_keys.push(entry.key().clone());
                    } else if next_deadline.map_or(true, |nd| deadline < nd) {
                        next_deadline = Some(deadline);
                    }
                }
            }

            // ── 派发到期任务（受并发令牌限制） ──
            for key in &due_keys {
                // 原子切换状态：Pending → Processing
                state.insert(key.clone(), DebounceState::Processing { dirty: false });

                let st = key.0.clone();
                let sid = key.1.clone();
                let state_ref = state.clone();
                let key_clone = key.clone();
                let permits_ref = permits.clone();
                let retries_ref = retries.clone();
                let tx_ref = tx.clone();

                tokio::spawn(async move {
                    // ★ 获取并发令牌后再执行物化
                    let _permit = permits_ref.acquire().await;
                    // 二次确认：当前实体是否仍在 Processing 状态
                    // （极端场景下 worker 可能在获取令牌前被意外唤醒重复派发）
                    if let Some(entry) = state_ref.get(&key_clone) {
                        if !matches!(*entry, DebounceState::Processing { .. }) {
                            return;
                        }
                    }
                    // 执行实际物化管线
                    let ok =
                        crate::services::ai_sync::process_single_entity(&st, &sid).await;
                    if ok {
                        retries_ref.remove(&key_clone);
                        // O5: 物化期间收到新写入（dirty）→ 立即重新入队物化最新内容
                        let was_dirty = state_ref.get(&key_clone)
                            .map_or(false, |e| matches!(*e, DebounceState::Processing { dirty: true }));
                        state_ref.remove(&key_clone);
                        if was_dirty {
                            state_ref.insert(
                                key_clone.clone(),
                                DebounceState::Pending(
                                    Instant::now() + Duration::from_secs(DEBOUNCE_WINDOW_SECS),
                                ),
                            );
                            let _ = tx_ref.send(ControlMsg::Enqueue);
                            log::info!("[防抖调度器] 已物化，期间有新写入，重新排队: {st}:{sid}");
                        } else {
                            log::info!("[防抖调度器] 已物化: {st}:{sid}");
                        }
                        clear_materialize_debt(&st, &sid);
                    } else {
                        // 物化失败 → 记录重试次数，按退避间隔重新排队
                        // ★ 防自死锁：不得跨 len()/remove()/insert() 持有 entry 的 RefMut（DashMap 分片锁非重入）
                        let prev = retries_ref.get(&key_clone).map(|x| *x);
                        // O9: 重试队列达到上限时放弃新失败实体（已跟踪的继续重试）
                        if prev.is_none() && retries_ref.len() >= MAX_RETRY_TRACKED {
                            state_ref.remove(&key_clone);
                            record_materialize_debt(&st, &sid);
                            log::warn!("[防抖调度器] 重试队列已满，落债务表: {st}:{sid}");
                        } else {
                            let count = prev.unwrap_or(0) + 1;
                            if count >= MAX_MATERIALIZE_RETRIES {
                                retries_ref.remove(&key_clone);
                                state_ref.remove(&key_clone);
                                record_materialize_debt(&st, &sid);
                                log::error!("[防抖调度器] 物化失败 {count} 次，落债务表持续补偿: {st}:{sid}");
                            } else {
                                retries_ref.insert(key_clone.clone(), count);
                                state_ref.insert(
                                    key_clone.clone(),
                                    DebounceState::Pending(
                                        Instant::now() + Duration::from_secs(RETRY_BACKOFF_SECS),
                                    ),
                                );
                                log::warn!("[防抖调度器] 物化失败，{count}/{MAX_MATERIALIZE_RETRIES} 次后重试: {st}:{sid}");
                                let _ = tx_ref.send(ControlMsg::Enqueue);
                            }
                        }
                    }
                    // _permit 在此处 drop，释放并发槽位
                });
            }

            // ── 计算睡眠时长，等待下一次截止 ──
            let sleep_dur = next_deadline
                .map(|nd| nd.saturating_duration_since(Instant::now()));

            let msg = match sleep_dur {
                Some(dur) if !dur.is_zero() => {
                    tokio::select! {
                        m = rx.recv() => m,
                        _ = tokio::time::sleep(dur) => {
                            // 超时 → 回到循环顶部重新扫描
                            continue;
                        }
                    }
                }
                _ => {
                    // 没有待处理任务 → 阻塞等待下一控制消息
                    rx.recv().await
                }
            };

            match msg {
                Some(ControlMsg::Shutdown) | None => {
                    log::info!("[防抖调度器] 已关闭");
                    break;
                }
                Some(ControlMsg::Enqueue) => {
                    // 回到循环顶部重新扫描截止时间
                }
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_entity_key_equality() {
        let k1: EntityKey = ("notes".into(), "abc".into());
        let k2: EntityKey = ("notes".into(), "abc".into());
        let k3: EntityKey = ("notes".into(), "def".into());
        assert_eq!(k1, k2);
        assert_ne!(k1, k3);
    }

    #[tokio::test]
    async fn test_spawn_and_shutdown() {
        let sched = DebouncedMaterializer::spawn();
        assert_eq!(sched.pending_count(), 0);
        sched.shutdown();
        // Give worker a moment to exit
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    #[tokio::test]
    async fn test_enqueue_creates_pending() {
        let sched = DebouncedMaterializer::spawn();
        sched.enqueue("notes", "test-entity-1");
        // Allow worker to process the message
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(sched.pending_count() > 0);
        sched.shutdown();
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    #[tokio::test]
    async fn test_duplicate_enqueue_does_not_double_count() {
        let sched = DebouncedMaterializer::spawn();
        sched.enqueue("notes", "test-entity-2");
        tokio::time::sleep(Duration::from_millis(10)).await;
        sched.enqueue("notes", "test-entity-2"); // Upsert — should NOT create second entry
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert_eq!(sched.pending_count(), 1);
        sched.shutdown();
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
