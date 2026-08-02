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
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, Semaphore};

// ═══════════════════════════════════════════════════════════════════
// Constants
// ═══════════════════════════════════════════════════════════════════

/// 防抖窗口：此时间内连续写入不触发物化
const DEBOUNCE_WINDOW_SECS: u64 = 30;

/// 最大并发物化数。SQLite 全局 Mutex 串行化所有 DB 操作，
/// 此处限制避免过多任务同时排队等待锁。
const MAX_CONCURRENT_MATERIALIZATIONS: usize = 4;

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
    /// 此状态下拒绝一切并发请求，后续写入自动排入下一周期。
    Processing,
}

/// 控制通道消息
enum ControlMsg {
    /// 入队或重置倒计时
    Enqueue { source_type: String, source_id: String },
    /// 优雅关闭
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

    /// 并发令牌：限制同时进行的物化任务数量。
    /// SQLite 使用全局 Mutex<Connection> 串行化写入，4 个并发任务
    /// 可在排队时避免饥饿，同时不会产生过多无谓的锁竞争。
    materialize_permits: Arc<Semaphore>,
}

impl DebouncedMaterializer {
    /// 创建调度器并启动后台 worker。
    ///
    /// worker 运行在独立的 tokio 任务中，由调用方的 tokio runtime 驱动。
    /// 建议在 Tauri `setup` 阶段调用，确保 tokio runtime 已就绪。
    pub fn spawn() -> Arc<Self> {
        let state: Arc<DashMap<EntityKey, DebounceState>> = Arc::new(DashMap::new());
        let (tx, rx) = mpsc::unbounded_channel();
        let materialize_permits = Arc::new(Semaphore::new(MAX_CONCURRENT_MATERIALIZATIONS));

        let worker_state = state.clone();
        let worker_permits = materialize_permits.clone();
        tokio::spawn(async move {
            Self::worker_loop(worker_state, worker_permits, rx).await;
        });

        let sched = Self { state, tx, materialize_permits };
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
    ///   - 重复入队（正在 Processing）→ 忽略，物化完成后源数据已更新即自动下一轮
    ///
    /// 调用方无需关心返回值 — 入队操作是 fire-and-forget。
    pub fn enqueue(&self, source_type: &str, source_id: &str) {
        let key: EntityKey = (source_type.to_string(), source_id.to_string());
        let now = Instant::now();
        let deadline = now + Duration::from_secs(5);

        // ── 快速路径：更新 DashMap ──
        match self.state.get_mut(&key) {
            Some(mut entry) => {
                match *entry {
                    DebounceState::Pending(_) => {
                        // ★ Upsert 核心：重置倒计时，不创建新任务
                        *entry = DebounceState::Pending(deadline);
                        // 通知 worker 重新计算最早截止时间
                        let _ = self.tx.send(ControlMsg::Enqueue {
                            source_type: source_type.to_string(),
                            source_id: source_id.to_string(),
                        });
                    }
                    DebounceState::Processing => {
                        // 已在处理中，无需动作。
                        // 当物化完成后 worker 会删除该键，后续写入自动进入新周期。
                    }
                }
            }
            None => {
                self.state
                    .insert(key.clone(), DebounceState::Pending(deadline));
                let _ = self.tx.send(ControlMsg::Enqueue {
                    source_type: source_type.to_string(),
                    source_id: source_id.to_string(),
                });
            }
        }
    }

    /// 优雅关闭调度器及其 worker。
    pub fn shutdown(&self) {
        let _ = self.tx.send(ControlMsg::Shutdown);
    }

    /// 获取当前跟踪的实体数量（调试用）。
    #[allow(dead_code)]
    pub fn pending_count(&self) -> usize {
        self.state.len()
    }

    // ═══════════════════════════════════════════════════════════════
    // Worker internals
    // ═══════════════════════════════════════════════════════════════

    /// 单线程事件循环：扫描过期任务、派生物化、等待下一截止时间。
    async fn worker_loop(
        state: Arc<DashMap<EntityKey, DebounceState>>,
        permits: Arc<Semaphore>,
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
                state.insert(key.clone(), DebounceState::Processing);

                let st = key.0.clone();
                let sid = key.1.clone();
                let state_ref = state.clone();
                let key_clone = key.clone();
                let permits_ref = permits.clone();

                tokio::spawn(async move {
                    // ★ 获取并发令牌后再执行物化
                    let _permit = permits_ref.acquire().await;
                    // 二次确认：当前实体是否仍在 Processing 状态
                    // （极端场景下 worker 可能在获取令牌前被意外唤醒重复派发）
                    if let Some(entry) = state_ref.get(&key_clone) {
                        if !matches!(*entry, DebounceState::Processing) {
                            return;
                        }
                    }
                    // 执行实际物化管线
                    let ok =
                        crate::services::ai_sync::process_single_entity(&st, &sid).await;
                    if ok {
                        log::info!("[防抖调度器] 已物化: {st}:{sid}");
                    }
                    // 清除任务记录，为下一轮写入腾出空间
                    state_ref.remove(&key_clone);
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
                Some(ControlMsg::Enqueue { .. }) => {
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
