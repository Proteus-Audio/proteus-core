use std::{
    fs::{File, OpenOptions},
    io::{BufWriter, Write},
};

use log::info;

pub fn log(message: &str) {
    // create_log_if_not_exists();
    append_to_log(message);
}

fn create_log_if_not_exists() {
    let filepath = std::env::current_dir().expect("Failed to get current directory");
    let log_path = filepath.join("log.txt");
    if !log_path.exists() {
        let file = File::create(log_path).expect("Failed to create log file");

        let mut writer = BufWriter::new(file);
        writer
            .write(b"Created log file")
            .expect("Failed to write to log file");
    }
}

fn append_to_log(message: &str) {
    let filepath = std::env::current_dir().expect("Failed to get current directory");
    let log_path = filepath.join("log.txt");

    let file = OpenOptions::new()
        .append(true)
        .create(true)
        .open(log_path)
        .expect("Failed to open log file");

    let mut writer = BufWriter::new(file);
    writer
        .write_all(message.as_bytes())
        .expect("Failed to write to log file");
}

pub fn clear_logfile() {
    let filepath = std::env::current_dir().expect("Failed to get current directory");
    let log_path = filepath.join("log.txt");

    let file = OpenOptions::new()
        .write(true)
        .truncate(true)
        .create(true)
        .open(log_path)
        .expect("Failed to open log file");

    let mut writer = BufWriter::new(file);
    writer
        .write_all("".as_bytes())
        .expect("Failed to write to log file");
}
