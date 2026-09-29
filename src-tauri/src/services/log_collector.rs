use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use serde::Serialize;
use log::{Level, LevelFilter, Metadata, Record, SetLoggerError};

/// 磁盘日志轮转阈值：单文件超过 10MB → 改名为 app.log.1（覆盖旧的），重新开新文件
const LOG_FILE_MAX_BYTES: u64 = 10 * 1024 * 1024;

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

/// 磁盘 sink：追加写 app.log，超限自动轮转（诊断留痕——此前仅内存环形，重启即丢）
struct FileSink {
    file: Option<std::fs::File>,
    dir: PathBuf,
    written: u64,
}

impl FileSink {
    fn open(dir: &Path) -> Self {
        let _ = std::fs::create_dir_all(dir);
        let file = open_appended(dir);
        let written = dir.join("app.log").metadata().map(|m| m.len()).unwrap_or(0);
        Self { file, dir: dir.to_path_buf(), written }
    }

    fn write_line(&mut self, line: &str) {
        if self.written >= LOG_FILE_MAX_BYTES {
            self.rotate();
        }
        if let Some(f) = self.file.as_mut() {
            if writeln!(f, "{}", line).is_ok() {
                self.written += line.len() as u64 + 1;
            } else {
                // 写失败（磁盘满/权限）→ 丢弃 sink，stderr/内存日志继续
                self.file = None;
            }
        }
    }

    fn rotate(&mut self) {
        self.file = None; // 关闭旧句柄（Windows 下 rename 需先关）
        let log_path = self.dir.join("app.log");
        let bak_path = self.dir.join("app.log.1");
        let _ = std::fs::remove_file(&bak_path);
        let _ = std::fs::rename(&log_path, &bak_path);
        self.file = open_appended(&self.dir);
        self.written = self.dir.join("app.log").metadata().map(|m| m.len()).unwrap_or(0);
    }
}

fn open_appended(dir: &Path) -> Option<std::fs::File> {
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("app.log"))
        .ok()
}

/// 全局 Logger 实现 — log::Log trait
struct AppLogger {
    logs: Arc<Mutex<Vec<LogEntry>>>,
    max_size: usize,
    sink: Mutex<FileSink>,
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
        let line = format!("[{}] [{}] [{}] {}", now, level, module, message);

        // 写 stderr（开发时终端可见）
        eprintln!("{}", line);

        // 写磁盘（诊断留痕，重启不丢）
        if let Ok(mut sink) = self.sink.lock() {
            sink.write_line(&line);
        }

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
/// log_dir: 磁盘日志目录（app_data_dir/logs），创建失败时自动降级为仅内存+stderr
pub fn init_logger(max_size: usize, log_dir: &Path) -> Result<LogState, SetLoggerError> {
    let state = LogState::new();
    let logger = AppLogger {
        logs: state.logs.clone(),
        max_size,
        sink: Mutex::new(FileSink::open(log_dir)),
    };
    log::set_boxed_logger(Box::new(logger))?;
    log::set_max_level(LevelFilter::Info);
    Ok(state)
}
