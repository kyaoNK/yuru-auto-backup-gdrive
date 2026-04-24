use std::collections::VecDeque;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::Local;

pub struct Logger {
    path: PathBuf,
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
            path: path.to_path_buf(),
            file: Mutex::new(file),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn tail(&self, limit: usize) -> io::Result<Vec<String>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let text = fs::read_to_string(&self.path)?;
        let mut buf: VecDeque<String> = VecDeque::with_capacity(limit);
        for line in text.lines() {
            if buf.len() == limit {
                buf.pop_front();
            }
            buf.push_back(line.to_string());
        }
        Ok(buf.into_iter().collect())
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
    fn tail_returns_last_n_lines() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("backup.log");
        let logger = Logger::open(&path).unwrap();

        for i in 0..10 {
            logger.info(&format!("line {i}"));
        }
        let tail = logger.tail(3).unwrap();
        assert_eq!(tail.len(), 3);
        assert!(tail[2].contains("line 9"));
        assert!(tail[0].contains("line 7"));
    }

    #[test]
    fn tail_returns_empty_when_file_does_not_exist() {
        let tmp = tempdir().unwrap();
        let logger = Logger {
            path: tmp.path().join("missing.log"),
            file: std::sync::Mutex::new(fs::File::create(tmp.path().join("other.log")).unwrap()),
        };
        let tail = logger.tail(5).unwrap();
        assert!(tail.is_empty());
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
