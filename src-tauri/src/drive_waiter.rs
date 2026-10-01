use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use thiserror::Error;

use crate::backup::DRIVE_WAIT_SECONDS;

const POLL_INTERVAL: Duration = Duration::from_secs(10);

#[derive(Debug, Error)]
pub enum DriveWaitError {
    #[error("destination directory never appeared: {0}")]
    Timeout(PathBuf),
    #[error("backup wait cancelled during shutdown")]
    Cancelled,
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
    wait_cancellable(destination, timeout, interval, &AtomicBool::new(false))
}

pub fn wait_cancellable(
    destination: &Path,
    timeout: Duration,
    interval: Duration,
    stopping: &AtomicBool,
) -> Result<(), DriveWaitError> {
    let start = Instant::now();
    loop {
        if stopping.load(Ordering::SeqCst) {
            return Err(DriveWaitError::Cancelled);
        }
        if destination.is_dir() {
            return Ok(());
        }
        if start.elapsed() >= timeout {
            return Err(DriveWaitError::Timeout(destination.to_path_buf()));
        }
        let remaining = timeout.saturating_sub(start.elapsed());
        let pause = remaining.min(interval.max(Duration::from_millis(1)));
        let poll_end = Instant::now() + pause;
        while Instant::now() < poll_end {
            if stopping.load(Ordering::SeqCst) {
                return Err(DriveWaitError::Cancelled);
            }
            thread::sleep(
                poll_end
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(100)),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::mpsc;
    use tempfile::tempdir;

    #[test]
    fn rejects_file_and_does_not_sleep_past_deadline() {
        let tmp = tempdir().unwrap();
        let file = tmp.path().join("not-a-folder");
        fs::write(&file, "file").unwrap();
        let start = Instant::now();
        assert!(wait_with(&file, Duration::from_millis(30), Duration::from_secs(10)).is_err());
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn shutdown_cancels_wait_without_poll_delay() {
        let stop = AtomicBool::new(true);
        assert!(matches!(
            wait_cancellable(
                Path::new("missing"),
                Duration::from_secs(300),
                Duration::from_secs(10),
                &stop
            ),
            Err(DriveWaitError::Cancelled)
        ));
    }

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
