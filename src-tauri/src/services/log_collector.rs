use std::sync::{Arc, Mutex};
use serde::Serialize;
use log::{Level, LevelFilter, Metadata, Record, SetLoggerError};

#[derive(Clone, Serialize)]
pub struct LogEntry {
    pub timestamp: String,
    pub level: String,
    pub module: String,
    pub message: String,
}

/// Tauri managed state — 前端通过 get_logs / clear_logs 命令访问
pub struct LogState {
    pub logs: Arc<Mutex<Vec<LogEntry>>>,
}

impl LogState {
    pub fn new() -> Self {
        Self {
            logs: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn get_all(&self) -> Vec<LogEntry> {
        self.logs.lock().unwrap().clone()
    }

    pub fn clear(&self) {
        self.logs.lock().unwrap().clear();
    }
}

/// 全局 Logger 实现 — log::Log trait
struct AppLogger {
    logs: Arc<Mutex<Vec<LogEntry>>>,
    max_size: usize,
}

impl log::Log for AppLogger {
    fn enabled(&self, _metadata: &Metadata) -> bool {
        true
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) { return; }

        let now = chrono::Local::now().format("%H:%M:%S%.3f").to_string();
        let level = match record.level() {
            Level::Error => "ERROR",
            Level::Warn => "WARN",
            Level::Info => "INFO",
            Level::Debug => "DEBUG",
            Level::Trace => "TRACE",
        };
        let module = record.module_path().unwrap_or("unknown")
            .split("::").last().unwrap_or("unknown").to_string();
        let message = format!("{}", record.args());

        // 写 stderr（开发时终端可见）
        eprintln!("[{}] [{}] [{}] {}", now, level, module, message);

        // 写内存
        let entry = LogEntry {
            timestamp: now,
            level: level.to_string(),
            module,
            message,
        };
        let mut logs = self.logs.lock().unwrap();
        if logs.len() >= self.max_size {
            logs.remove(0);
        }
        logs.push(entry);
    }

    fn flush(&self) {}
}

/// 初始化全局 logger，返回 LogState（供 Tauri managed state 使用）
pub fn init_logger(max_size: usize) -> Result<LogState, SetLoggerError> {
    let state = LogState::new();
    let logger = AppLogger {
        logs: state.logs.clone(),
        max_size,
    };
    log::set_boxed_logger(Box::new(logger))?;
    log::set_max_level(LevelFilter::Info);
    Ok(state)
}
