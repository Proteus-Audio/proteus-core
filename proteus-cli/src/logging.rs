use log::{LevelFilter, Log, Metadata, Record};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};

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
