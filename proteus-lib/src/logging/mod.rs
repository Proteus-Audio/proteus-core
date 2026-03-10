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
pub fn log(message: &str) -> io::Result<()> {
    append_to_log(message)
}

#[cfg(feature = "buffer-map")]
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
