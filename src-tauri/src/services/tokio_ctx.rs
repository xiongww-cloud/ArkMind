//! tokio 线程上下文区分：安全执行 sync→async 桥接。
//!
//! 背景：`block_in_place` 在 worker 线程上合法，但在 `spawn_blocking` 线程上**必然 panic**
//! （"block_in_place() is not supported on this thread"）。而 `Handle::try_current()` 在两种
//! 线程上都返回 Ok，无法区分——必须自行标记。spawn_blocking 闭包内用 `BlockingThreadEnter`
//! 打标记，桥接函数改用 `block_on_sync`：worker 走 block_in_place，blocking/无 runtime 线程
//! 走独立 current-thread runtime。

thread_local! {
    static IN_BLOCKING_THREAD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub struct BlockingThreadEnter;

impl BlockingThreadEnter {
    pub fn new() -> Self {
        IN_BLOCKING_THREAD.with(|c| c.set(true));
        BlockingThreadEnter
    }
}

impl Drop for BlockingThreadEnter {
    fn drop(&mut self) {
        IN_BLOCKING_THREAD.with(|c| c.set(false));
    }
}

pub fn in_blocking_thread() -> bool {
    IN_BLOCKING_THREAD.with(|c| c.get())
}

/// 从同步代码执行 future：自动适配 worker / spawn_blocking / 无 runtime 三种线程。
/// 返回 Err 仅当线程既无 runtime 又无法构建 runtime（理论上不可达）。
pub fn block_on_sync<F: std::future::Future>(f: F) -> Result<F::Output, String> {
    if tokio::runtime::Handle::try_current().is_ok() && !in_blocking_thread() {
        let h = tokio::runtime::Handle::current();
        Ok(tokio::task::block_in_place(move || h.block_on(f)))
    } else {
        match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(rt) => Ok(rt.block_on(f)),
            Err(e) => Err(format!("无可用 tokio 运行时: {e}")),
        }
    }
}
