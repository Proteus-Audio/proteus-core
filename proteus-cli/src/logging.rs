//! Log capture and routing for TUI-friendly rendering.

use log::{LevelFilter, Log, Metadata, Record};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;

const LOG_CAPACITY: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Classification used for styling log lines in the TUI.
pub enum LogKind {
    Error,
    Stderr,
    Warn,
    Info,
    Debug,
    Trace,
}

#[derive(Debug, Clone)]
/// Structured log line with classification for styling.
pub struct LogLine {
    pub kind: LogKind,
    pub text: String,
}

struct SharedLogger {
    level: LevelFilter,
    buffer: Arc<Mutex<VecDeque<LogLine>>>,
    echo_stderr: AtomicBool,
}

impl Log for SharedLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= self.level
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let line = format!("[{}] {}", record.level(), record.args());
        if self.echo_stderr.load(Ordering::Relaxed) {
            eprintln!("{}", line);
        }

        let mut buffer = self.buffer.lock().unwrap();
        if buffer.len() >= LOG_CAPACITY {
            buffer.pop_front();
        }
        let kind = match record.level() {
            log::Level::Error => LogKind::Error,
            log::Level::Warn => LogKind::Warn,
            log::Level::Info => LogKind::Info,
            log::Level::Debug => LogKind::Debug,
            log::Level::Trace => LogKind::Trace,
        };
        buffer.push_back(LogLine { kind, text: line });
    }

    fn flush(&self) {}
}

static LOG_BUFFER: OnceLock<Arc<Mutex<VecDeque<LogLine>>>> = OnceLock::new();
static LOGGER: OnceLock<SharedLogger> = OnceLock::new();

/// Initialize the logger and return the shared log buffer.
pub fn init() -> Arc<Mutex<VecDeque<LogLine>>> {
    let buffer = LOG_BUFFER
        .get_or_init(|| Arc::new(Mutex::new(VecDeque::with_capacity(LOG_CAPACITY))))
        .clone();

    let level = match std::env::var("RUST_LOG") {
        Ok(level) => match level.to_lowercase().as_str() {
            "error" => LevelFilter::Error,
            "warn" => LevelFilter::Warn,
            "debug" => LevelFilter::Debug,
            "trace" => LevelFilter::Trace,
            _ => LevelFilter::Info,
        },
        Err(_) => LevelFilter::Info,
    };

    let echo_stderr = std::env::var("PROTEUS_LOG_STDERR")
        .map(|value| value != "0")
        .unwrap_or(true);

    let logger = SharedLogger {
        level,
        buffer: buffer.clone(),
        echo_stderr: AtomicBool::new(echo_stderr),
    };

    let logger_ref = LOGGER.get_or_init(|| logger);
    if log::set_logger(logger_ref).is_ok() {
        log::set_max_level(level);
    }

    buffer
}

/// Enable or disable stderr echoing for log lines.
pub fn set_echo_stderr(enabled: bool) {
    if let Some(logger) = LOGGER.get() {
        logger.echo_stderr.store(enabled, Ordering::Relaxed);
    }
}

/// Snapshot the current log buffer for rendering.
pub fn snapshot_lines(buffer: &Arc<Mutex<VecDeque<LogLine>>>) -> Vec<LogLine> {
    buffer.lock().unwrap().iter().cloned().collect()
}

/// Restores stderr on drop for capture sessions.
pub struct StderrCaptureGuard {
    original_fd: RawFd,
    stderr_fd: RawFd,
    reader_handle: Option<JoinHandle<()>>,
}

impl Drop for StderrCaptureGuard {
    fn drop(&mut self) {
        unsafe {
            libc::dup2(self.original_fd, self.stderr_fd);
            libc::close(self.original_fd);
            libc::close(self.stderr_fd);
        }
        if let Some(handle) = self.reader_handle.take() {
            let _ = handle.join();
        }
    }
}

/// Redirect stderr into the log buffer for the active TUI session.
pub fn capture_stderr(buffer: Arc<Mutex<VecDeque<LogLine>>>) -> Option<StderrCaptureGuard> {
    let stderr_fd = std::io::stderr().as_raw_fd();
    let mut fds = [0; 2];
    let pipe_result = unsafe { libc::pipe(fds.as_mut_ptr()) };
    if pipe_result != 0 {
        return None;
    }

    let read_fd = fds[0];
    let write_fd = fds[1];
    let original_fd = unsafe { libc::dup(stderr_fd) };
    if original_fd < 0 {
        unsafe {
            libc::close(read_fd);
            libc::close(write_fd);
        }
        return None;
    }

    let dup_result = unsafe { libc::dup2(write_fd, stderr_fd) };
    if dup_result < 0 {
        unsafe {
            libc::close(read_fd);
            libc::close(write_fd);
            libc::close(original_fd);
        }
        return None;
    }

    let handle = std::thread::spawn(move || {
        let file = unsafe { std::fs::File::from_raw_fd(read_fd) };
        let mut reader = BufReader::new(file);
        let mut line = String::new();
        loop {
            line.clear();
            let bytes = reader.read_line(&mut line).unwrap_or(0);
            if bytes == 0 {
                break;
            }
            let trimmed = line.trim_end();
            if trimmed.is_empty() {
                continue;
            }
            let mut buffer = buffer.lock().unwrap();
            if buffer.len() >= LOG_CAPACITY {
                buffer.pop_front();
            }
            buffer.push_back(LogLine {
                kind: LogKind::Stderr,
                text: format!("[STDERR] {}", trimmed),
            });
        }
    });

    unsafe {
        libc::close(write_fd);
    }

    Some(StderrCaptureGuard {
        original_fd,
        stderr_fd,
        reader_handle: Some(handle),
    })
}
