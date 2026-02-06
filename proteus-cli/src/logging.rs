use log::{LevelFilter, Log, Metadata, Record};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;

const LOG_CAPACITY: usize = 500;

struct SharedLogger {
    level: LevelFilter,
    buffer: Arc<Mutex<VecDeque<String>>>,
    echo_stderr: bool,
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
        if self.echo_stderr {
            eprintln!("{}", line);
        }

        let mut buffer = self.buffer.lock().unwrap();
        if buffer.len() >= LOG_CAPACITY {
            buffer.pop_front();
        }
        buffer.push_back(line);
    }

    fn flush(&self) {}
}

static LOG_BUFFER: OnceLock<Arc<Mutex<VecDeque<String>>>> = OnceLock::new();
static LOGGER: OnceLock<SharedLogger> = OnceLock::new();

pub fn init() -> Arc<Mutex<VecDeque<String>>> {
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
        .unwrap_or(false);

    let logger = SharedLogger {
        level,
        buffer: buffer.clone(),
        echo_stderr,
    };

    let logger_ref = LOGGER.get_or_init(|| logger);
    if log::set_logger(logger_ref).is_ok() {
        log::set_max_level(level);
    }

    buffer
}

pub fn snapshot(buffer: &Arc<Mutex<VecDeque<String>>>) -> Vec<String> {
    buffer.lock().unwrap().iter().cloned().collect()
}

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

pub fn capture_stderr(buffer: Arc<Mutex<VecDeque<String>>>) -> Option<StderrCaptureGuard> {
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
            buffer.push_back(format!("[STDERR] {}", trimmed));
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
