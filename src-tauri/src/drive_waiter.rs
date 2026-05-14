use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use thiserror::Error;

use crate::backup::DRIVE_WAIT_SECONDS;

const POLL_INTERVAL: Duration = Duration::from_secs(10);

#[derive(Debug, Error)]
pub enum DriveWaitError {
    #[error("destination directory never appeared: {0}")]
    Timeout(PathBuf),
}

pub fn wait_for_destination(destination: &Path) -> Result<(), DriveWaitError> {
    wait_with(
        destination,
        Duration::from_secs(DRIVE_WAIT_SECONDS),
        POLL_INTERVAL,
    )
}

pub fn wait_with(
    destination: &Path,
    timeout: Duration,
    interval: Duration,
) -> Result<(), DriveWaitError> {
    let start = Instant::now();
    loop {
        if destination.exists() {
            return Ok(());
        }
        if start.elapsed() >= timeout {
            return Err(DriveWaitError::Timeout(destination.to_path_buf()));
        }
        thread::sleep(interval);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::mpsc;
    use tempfile::tempdir;

    #[test]
    fn returns_ok_immediately_when_destination_exists() {
        let tmp = tempdir().unwrap();
        let elapsed_before = Instant::now();
        wait_with(
            tmp.path(),
            Duration::from_secs(5),
            Duration::from_millis(50),
        )
        .unwrap();
        assert!(elapsed_before.elapsed() < Duration::from_millis(500));
    }

    #[test]
    fn returns_timeout_error_when_destination_never_appears() {
        let tmp = tempdir().unwrap();
        let missing = tmp.path().join("never");
        let err = wait_with(
            &missing,
            Duration::from_millis(150),
            Duration::from_millis(30),
        )
        .unwrap_err();
        assert!(matches!(err, DriveWaitError::Timeout(_)));
    }

    #[test]
    fn succeeds_when_destination_appears_before_timeout() {
        let tmp = tempdir().unwrap();
        let late = tmp.path().join("late");
        let late_clone = late.clone();

        let (tx, rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            thread::sleep(Duration::from_millis(80));
            fs::create_dir_all(&late_clone).unwrap();
            tx.send(()).unwrap();
        });

        wait_with(&late, Duration::from_millis(500), Duration::from_millis(20)).unwrap();

        rx.recv().unwrap();
        handle.join().unwrap();
    }
}
