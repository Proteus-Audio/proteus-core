//! Optional log-file helpers used only by feature-gated diagnostics.
//!
//! `buffer-map` enables the `log`/`clear_logfile` helpers consumed by the buffer
//! mixer's occupancy tracing, while `debug` exposes `pivot_buffer_trace` for
//! post-processing those traces. These helpers are intentionally feature-gated
//! and are expected to disappear from builds that do not opt into diagnostics.

#[cfg(feature = "debug")]
pub mod pivot_buffer_trace;

#[cfg(feature = "buffer-map")]
use std::io;

#[cfg(feature = "buffer-map")]
use std::{
    fs::OpenOptions,
    io::{BufWriter, Write},
};

#[cfg(feature = "buffer-map")]
fn append_to_log(message: &str) -> io::Result<()> {
    let filepath = std::env::current_dir()?;
    let log_path = filepath.join("log.txt");

    let file = OpenOptions::new()
        .append(true)
        .create(true)
        .open(log_path)?;

    let mut writer = BufWriter::new(file);
    writer.write_all(message.as_bytes())
}

#[cfg(feature = "buffer-map")]
/// Append one message to the buffer-map debug log file in the current directory.
pub fn log(message: &str) -> io::Result<()> {
    append_to_log(message)
}

#[cfg(feature = "buffer-map")]
/// Truncate the buffer-map debug log file in the current directory.
pub fn clear_logfile() -> io::Result<()> {
    let filepath = std::env::current_dir()?;
    let log_path = filepath.join("log.txt");

    let file = OpenOptions::new()
        .write(true)
        .truncate(true)
        .create(true)
        .open(log_path)?;

    let mut writer = BufWriter::new(file);
    writer.write_all("".as_bytes())
}
