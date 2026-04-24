use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use chrono::{DateTime, Local, NaiveTime, TimeZone};
use thiserror::Error;

use crate::backup::{BackupJob, JobSummary};
use crate::config::{Config, ConfigStore};
use crate::drive_waiter::wait_for_destination;
use crate::logger::Logger;

#[derive(Debug, Error)]
pub enum ScheduleError {
    #[error("invalid schedule time (expected HH:MM): {0}")]
    InvalidFormat(String),
}

pub enum SchedulerCommand {
    RunNow,
    ReloadConfig,
    Shutdown,
}

pub trait JobReporter: Send + Sync + 'static {
    fn job_started(&self) {}
    fn job_finished(&self, _summary: JobSummary) {}
    fn job_errored(&self, _message: String) {}
    fn status_changed(&self) {}
}

pub struct NoopReporter;
impl JobReporter for NoopReporter {}

#[derive(Clone)]
pub struct SchedulerSender {
    tx: mpsc::Sender<SchedulerCommand>,
}

impl SchedulerSender {
    pub fn run_now(&self) -> Result<(), mpsc::SendError<SchedulerCommand>> {
        self.tx.send(SchedulerCommand::RunNow)
    }

    pub fn reload(&self) -> Result<(), mpsc::SendError<SchedulerCommand>> {
        self.tx.send(SchedulerCommand::ReloadConfig)
    }

    pub fn shutdown(&self) -> Result<(), mpsc::SendError<SchedulerCommand>> {
        self.tx.send(SchedulerCommand::Shutdown)
    }
}

pub struct SchedulerHandle {
    sender: SchedulerSender,
    join: Option<thread::JoinHandle<()>>,
}

impl SchedulerHandle {
    pub fn sender(&self) -> SchedulerSender {
        self.sender.clone()
    }

    pub fn shutdown(mut self) {
        let _ = self.sender.shutdown();
        if let Some(h) = self.join.take() {
            let _ = h.join();
        }
    }
}

pub fn parse_schedule(s: &str) -> Result<NaiveTime, ScheduleError> {
    let trimmed = s.trim();
    let bytes = trimmed.as_bytes();
    if bytes.len() != 5
        || bytes[2] != b':'
        || !bytes[0].is_ascii_digit()
        || !bytes[1].is_ascii_digit()
        || !bytes[3].is_ascii_digit()
        || !bytes[4].is_ascii_digit()
    {
        return Err(ScheduleError::InvalidFormat(s.to_string()));
    }
    NaiveTime::parse_from_str(trimmed, "%H:%M")
        .map_err(|_| ScheduleError::InvalidFormat(s.to_string()))
}

pub fn next_run_after(now: DateTime<Local>, schedule: NaiveTime) -> DateTime<Local> {
    let today_run = Local
        .from_local_datetime(&now.date_naive().and_time(schedule))
        .single()
        .unwrap_or(now);

    if now < today_run {
        today_run
    } else {
        today_run + chrono::Duration::days(1)
    }
}

pub fn should_catch_up(
    now: DateTime<Local>,
    schedule: NaiveTime,
    last_run_at: Option<DateTime<Local>>,
) -> bool {
    let today = now.date_naive();
    if now.time() < schedule {
        return false;
    }
    match last_run_at {
        None => true,
        Some(last) => last.date_naive() != today,
    }
}

pub fn start(
    store: Arc<ConfigStore>,
    logger: Arc<Logger>,
    reporter: Arc<dyn JobReporter>,
) -> SchedulerHandle {
    let (tx, rx) = mpsc::channel();
    let join = thread::spawn(move || run_loop(store, logger, reporter, rx));
    SchedulerHandle {
        sender: SchedulerSender { tx },
        join: Some(join),
    }
}

fn run_loop(
    store: Arc<ConfigStore>,
    logger: Arc<Logger>,
    reporter: Arc<dyn JobReporter>,
    rx: mpsc::Receiver<SchedulerCommand>,
) {
    if let Ok(cfg) = store.load() {
        if let Ok(sched) = parse_schedule(&cfg.schedule_time) {
            if should_catch_up(Local::now(), sched, cfg.last_run_at) {
                logger.info("Catching up on a missed scheduled run");
                run_job(&store, &logger, reporter.as_ref(), &cfg);
            }
        }
    }

    loop {
        let cfg = match store.load() {
            Ok(c) => c,
            Err(err) => {
                logger.error(&format!("Failed to load config: {err}"));
                match rx.recv_timeout(Duration::from_secs(60)) {
                    Ok(SchedulerCommand::Shutdown) | Err(RecvTimeoutError::Disconnected) => return,
                    _ => continue,
                }
            }
        };

        let wait = match parse_schedule(&cfg.schedule_time) {
            Ok(sched) => {
                let now = Local::now();
                (next_run_after(now, sched) - now)
                    .to_std()
                    .unwrap_or(Duration::from_secs(1))
            }
            Err(err) => {
                logger.error(&format!("{err}; retrying in 60s"));
                Duration::from_secs(60)
            }
        };

        reporter.status_changed();

        match rx.recv_timeout(wait) {
            Ok(SchedulerCommand::RunNow) => {
                logger.info("Manual backup triggered");
                run_job(&store, &logger, reporter.as_ref(), &cfg);
            }
            Ok(SchedulerCommand::ReloadConfig) => continue,
            Ok(SchedulerCommand::Shutdown) => return,
            Err(RecvTimeoutError::Timeout) => {
                run_job(&store, &logger, reporter.as_ref(), &cfg);
            }
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

fn run_job(store: &ConfigStore, logger: &Logger, reporter: &dyn JobReporter, cfg: &Config) {
    let (src, dest) = match (cfg.source.as_ref(), cfg.destination.as_ref()) {
        (Some(s), Some(d)) => (s.clone(), d.clone()),
        _ => {
            logger.warn("Backup skipped: source or destination not configured");
            reporter.job_errored("Source or destination not configured".to_string());
            return;
        }
    };

    reporter.job_started();

    logger.info(&format!(
        "Backup starting: {} -> {}",
        src.display(),
        dest.display()
    ));

    if let Err(err) = wait_for_destination(&dest) {
        let msg = format!("Destination not ready: {err}");
        logger.error(&msg);
        reporter.job_errored(msg);
        return;
    }

    match BackupJob::new(&src, &dest).run() {
        Ok(outcome) => {
            logger.info(&format!(
                "Backup complete: {} copied, {} errors",
                outcome.summary.copied, outcome.summary.errors
            ));
            for f in &outcome.copied_files {
                logger.info(&format!("  copied: {}", f.display()));
            }
            for (f, err) in &outcome.errored_files {
                logger.warn(&format!("  error: {} — {}", f.display(), err));
            }
            if let Ok(mut updated) = store.load() {
                updated.last_run_at = Some(Local::now());
                updated.last_summary = Some(outcome.summary);
                if let Err(err) = store.save(&updated) {
                    logger.error(&format!("Failed to persist run summary: {err}"));
                }
            }
            reporter.job_finished(outcome.summary);
        }
        Err(err) => {
            let msg = format!("Backup failed: {err}");
            logger.error(&msg);
            reporter.job_errored(msg);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, TimeZone};

    fn dt(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Local> {
        Local
            .from_local_datetime(
                &NaiveDate::from_ymd_opt(y, mo, d)
                    .unwrap()
                    .and_hms_opt(h, mi, 0)
                    .unwrap(),
            )
            .single()
            .unwrap()
    }

    fn time(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    mod parse_schedule {
        use super::*;

        #[test]
        fn parses_valid_hh_mm() {
            assert_eq!(parse_schedule("09:00").unwrap(), time(9, 0));
            assert_eq!(parse_schedule("23:59").unwrap(), time(23, 59));
            assert_eq!(parse_schedule("00:00").unwrap(), time(0, 0));
        }

        #[test]
        fn trims_whitespace() {
            assert_eq!(parse_schedule("  09:00  ").unwrap(), time(9, 0));
        }

        #[test]
        fn rejects_invalid_formats() {
            assert!(parse_schedule("9:00").is_err());
            assert!(parse_schedule("25:00").is_err());
            assert!(parse_schedule("09:60").is_err());
            assert!(parse_schedule("").is_err());
            assert!(parse_schedule("nine").is_err());
        }
    }

    mod next_run_after {
        use super::*;

        #[test]
        fn schedules_for_today_when_before_scheduled_time() {
            let now = dt(2026, 4, 24, 7, 0);
            let next = next_run_after(now, time(9, 0));
            assert_eq!(next, dt(2026, 4, 24, 9, 0));
        }

        #[test]
        fn schedules_for_tomorrow_when_after_scheduled_time() {
            let now = dt(2026, 4, 24, 10, 0);
            let next = next_run_after(now, time(9, 0));
            assert_eq!(next, dt(2026, 4, 25, 9, 0));
        }

        #[test]
        fn schedules_for_tomorrow_when_exactly_at_scheduled_time() {
            let now = dt(2026, 4, 24, 9, 0);
            let next = next_run_after(now, time(9, 0));
            assert_eq!(next, dt(2026, 4, 25, 9, 0));
        }
    }

    mod should_catch_up {
        use super::*;

        #[test]
        fn returns_false_when_before_scheduled_time() {
            let now = dt(2026, 4, 24, 7, 0);
            assert!(!should_catch_up(now, time(9, 0), None));
        }

        #[test]
        fn returns_true_when_past_scheduled_time_and_never_ran() {
            let now = dt(2026, 4, 24, 10, 0);
            assert!(should_catch_up(now, time(9, 0), None));
        }

        #[test]
        fn returns_true_when_past_scheduled_time_and_last_run_was_yesterday() {
            let now = dt(2026, 4, 24, 10, 0);
            let last = dt(2026, 4, 23, 9, 0);
            assert!(should_catch_up(now, time(9, 0), Some(last)));
        }

        #[test]
        fn returns_false_when_past_scheduled_time_but_already_ran_today() {
            let now = dt(2026, 4, 24, 15, 0);
            let last = dt(2026, 4, 24, 9, 0);
            assert!(!should_catch_up(now, time(9, 0), Some(last)));
        }
    }
}
