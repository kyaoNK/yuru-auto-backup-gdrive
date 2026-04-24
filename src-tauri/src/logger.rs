use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::sync::Mutex;

use chrono::Local;

pub struct Logger {
    file: Mutex<fs::File>,
}

impl Logger {
    pub fn open(path: &Path) -> io::Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        Ok(Self {
            file: Mutex::new(file),
        })
    }

    pub fn info(&self, msg: &str) {
        self.write("INFO", msg);
    }

    pub fn warn(&self, msg: &str) {
        self.write("WARN", msg);
    }

    pub fn error(&self, msg: &str) {
        self.write("ERROR", msg);
    }

    fn write(&self, level: &str, msg: &str) {
        let ts = Local::now().format("%Y-%m-%dT%H:%M:%S%:z");
        let line = format!("[{ts}] [{level}] {msg}\n");
        if let Ok(mut f) = self.file.lock() {
            let _ = f.write_all(line.as_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn open_creates_parent_directory_and_file() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("nested").join("backup.log");
        let _ = Logger::open(&path).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn info_warn_error_each_append_a_line_with_level_tag() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("backup.log");
        let logger = Logger::open(&path).unwrap();

        logger.info("job started");
        logger.warn("slow io");
        logger.error("boom");
        drop(logger);

        let content = fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].contains("[INFO]") && lines[0].contains("job started"));
        assert!(lines[1].contains("[WARN]") && lines[1].contains("slow io"));
        assert!(lines[2].contains("[ERROR]") && lines[2].contains("boom"));
    }

    #[test]
    fn reopening_appends_rather_than_truncating() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("backup.log");

        {
            let logger = Logger::open(&path).unwrap();
            logger.info("first run");
        }
        {
            let logger = Logger::open(&path).unwrap();
            logger.info("second run");
        }

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("first run"));
        assert!(content.contains("second run"));
    }
}
