use std::collections::VecDeque;
use std::fs;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::Local;

pub struct Logger {
    path: PathBuf,
    file: Mutex<fs::File>,
    last_error: Mutex<Option<String>>,
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
            last_error: Mutex::new(None),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn tail(&self, limit: usize) -> io::Result<Vec<String>> {
        let limit = limit.min(5000);
        if limit == 0 {
            return Ok(Vec::new());
        }
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let mut file = fs::File::open(&self.path)?;
        let len = file.metadata()?.len();
        let mut pos = len;
        let mut bytes = Vec::new();
        let mut newline_count = 0usize;
        const CHUNK_SIZE: u64 = 8192;

        while pos > 0 && newline_count <= limit && bytes.len() < 1024 * 1024 {
            let read_len = CHUNK_SIZE.min(pos);
            pos -= read_len;
            file.seek(SeekFrom::Start(pos))?;
            let mut chunk = vec![0u8; read_len as usize];
            file.read_exact(&mut chunk)?;
            newline_count += chunk.iter().filter(|&&b| b == b'\n').count();
            chunk.extend(bytes);
            bytes = chunk;
        }

        let text = String::from_utf8_lossy(&bytes);
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
        let msg = msg.replace('\r', "\\r").replace('\n', "\\n");
        let ts = Local::now().format("%Y-%m-%dT%H:%M:%S%:z");
        let line = format!("[{ts}] [{level}] {msg}\n");
        let result = self
            .file
            .lock()
            .map_err(|_| io::Error::other("logger lock poisoned"))
            .and_then(|mut file| file.write_all(line.as_bytes()));
        if let Err(err) = result {
            *self.last_error.lock().unwrap_or_else(|e| e.into_inner()) =
                Some(format!("ログを書き込めません: {err}"));
        }
    }

    pub fn last_error(&self) -> Option<String> {
        self.last_error
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn escapes_multiline_messages_and_surfaces_write_errors() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("log");
        let logger = Logger::open(&path).unwrap();
        logger.info("one\n[ERROR] fake\rline");
        assert_eq!(fs::read_to_string(&path).unwrap().lines().count(), 1);
        let failed = Logger {
            path: path.clone(),
            file: Mutex::new(fs::File::open(&path).unwrap()),
            last_error: Mutex::new(None),
        };
        failed.info("cannot write to read-only handle");
        assert!(failed.last_error().is_some());
    }

    #[test]
    fn bounds_tail_of_a_single_huge_line() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("log");
        fs::write(&path, vec![b'x'; 2 * 1024 * 1024]).unwrap();
        let logger = Logger::open(&path).unwrap();
        let lines = logger.tail(usize::MAX).unwrap();
        assert!(lines.iter().map(String::len).sum::<usize>() <= 1024 * 1024);
    }

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
    fn tail_zero_returns_empty() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("backup.log");
        let logger = Logger::open(&path).unwrap();
        logger.info("line");

        assert!(logger.tail(0).unwrap().is_empty());
    }

    #[test]
    fn tail_returns_empty_when_file_does_not_exist() {
        let tmp = tempdir().unwrap();
        let logger = Logger {
            path: tmp.path().join("missing.log"),
            file: std::sync::Mutex::new(fs::File::create(tmp.path().join("other.log")).unwrap()),
            last_error: Mutex::new(None),
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
