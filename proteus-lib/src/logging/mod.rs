#[cfg(feature = "debug")]
pub mod pivot_buffer_trace;

#[cfg(feature = "buffer-map")]
use std::{
    fs::{File, OpenOptions},
    io::{BufWriter, Write},
};

#[cfg(feature = "buffer-map")]
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

#[cfg(feature = "buffer-map")]
use std::io::{self, Read, Seek, SeekFrom};

#[cfg(feature = "buffer-map")]
pub fn append_on_line(file: &mut File, message: &str, line_number: usize) -> io::Result<()> {
    // Read entire file
    let mut contents = String::new();
    file.seek(SeekFrom::Start(0))?;
    file.read_to_string(&mut contents)?;

    // Split into owned lines
    let mut lines: Vec<String> = if contents.is_empty() {
        Vec::new()
    } else {
        contents.lines().map(|l| l.to_string()).collect()
    };

    // Extend with empty lines if necessary
    if line_number >= lines.len() {
        lines.resize(line_number + 1, String::new());
    }

    // Append message to target line
    lines[line_number].push_str(message);

    // Rebuild file contents
    let new_contents = lines.join("\n");

    // Truncate and rewrite
    file.set_len(0)?;
    file.seek(SeekFrom::Start(0))?;
    file.write_all(new_contents.as_bytes())?;

    Ok(())
}

#[cfg(feature = "buffer-map")]
pub fn log(message: &str) {
    append_to_log(message);
}

#[cfg(feature = "buffer-map")]
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

#[cfg(feature = "buffer-map")]
pub fn log_on_line(message: &str, line_number: usize) -> io::Result<()> {
    let filepath = std::env::current_dir().expect("Failed to get current directory");
    let log_path = filepath.join("log.txt");

    let mut file = OpenOptions::new()
        .write(true)
        .read(true)
        .truncate(true)
        .create(true)
        .open(log_path)
        .expect("Failed to open log file");

    append_on_line(&mut file, message, line_number)
}

#[cfg(all(test, feature = "buffer-map"))]
mod tests {
    use super::append_on_line;
    use std::fs::{self, File};
    use std::io::Read;

    #[test]
    fn append_on_line_extends_and_updates_requested_line() {
        let mut path = std::env::temp_dir();
        path.push(format!("proteus-log-test-{}.txt", std::process::id()));
        let mut file = File::create(&path).unwrap();

        append_on_line(&mut file, "hello", 1).unwrap();

        let mut content = String::new();
        File::open(&path)
            .unwrap()
            .read_to_string(&mut content)
            .unwrap();
        fs::remove_file(path).ok();

        assert_eq!(content, "\nhello");
    }
}
