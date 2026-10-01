use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use chrono::{DateTime, Local, NaiveTime, TimeZone};
use thiserror::Error;

use crate::backup::{BackupJob, JobSummary};
use crate::config::{Config, ConfigStore, JobFailure};
use crate::drive_waiter::{wait_cancellable, DriveWaitError};
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
    run_now_pending: Arc<AtomicBool>,
    stopping: Arc<AtomicBool>,
    next_at: Arc<Mutex<Option<DateTime<Local>>>>,
}

impl SchedulerSender {
    pub fn run_now(&self) -> Result<bool, mpsc::SendError<SchedulerCommand>> {
        if self.stopping.load(Ordering::SeqCst) {
            return Err(mpsc::SendError(SchedulerCommand::RunNow));
        }
        if self.run_now_pending.swap(true, Ordering::SeqCst) {
            return Ok(false);
        }
        match self.tx.send(SchedulerCommand::RunNow) {
            Ok(()) => Ok(true),
            Err(err) => {
                self.run_now_pending.store(false, Ordering::SeqCst);
                Err(err)
            }
        }
    }

    pub fn is_busy(&self) -> bool {
        self.run_now_pending.load(Ordering::SeqCst)
    }
    pub fn next_run_at(&self) -> Option<DateTime<Local>> {
        *self.next_at.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn reload(&self) -> Result<(), mpsc::SendError<SchedulerCommand>> {
        self.tx.send(SchedulerCommand::ReloadConfig)
    }

    pub fn shutdown(&self) -> Result<(), mpsc::SendError<SchedulerCommand>> {
        self.stopping.store(true, Ordering::SeqCst);
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

    pub fn shutdown_with_timeout(mut self, timeout: Duration) -> bool {
        let _ = self.sender.shutdown();
        let Some(h) = self.join.take() else {
            return true;
        };

        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = h.join();
            let _ = tx.send(());
        });
        rx.recv_timeout(timeout).is_ok()
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
    for day_offset in 0..3 {
        let date = now.date_naive() + chrono::Duration::days(day_offset);
        // Calendar arithmetic, including DST gaps/overlaps.
        for minute in 0..=180 {
            let local = date.and_time(schedule) + chrono::Duration::minutes(minute);
            if let Some(candidate) = Local.from_local_datetime(&local).earliest() {
                if candidate > now {
                    return candidate;
                }
                break;
            }
        }
    }
    now + chrono::Duration::days(1)
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
    let run_now_pending = Arc::new(AtomicBool::new(false));
    let sender = SchedulerSender {
        tx,
        run_now_pending: run_now_pending.clone(),
        stopping: Arc::new(AtomicBool::new(false)),
        next_at: Arc::new(Mutex::new(None)),
    };
    let control = sender.clone();
    let join = thread::spawn(move || run_loop(store, logger, reporter, rx, control));
    SchedulerHandle {
        sender,
        join: Some(join),
    }
}

fn run_loop(
    store: Arc<ConfigStore>,
    logger: Arc<Logger>,
    reporter: Arc<dyn JobReporter>,
    rx: mpsc::Receiver<SchedulerCommand>,
    control: SchedulerSender,
) {
    let mut armed_schedule: Option<String> = None;
    let mut deadline = None;
    let mut startup = true;
    let mut last_error: Option<String> = None;
    let mut observed_at: Option<DateTime<Local>> = None;
    while !control.stopping.load(Ordering::SeqCst) {
        let cfg = store.load().and_then(|cfg| {
            cfg.validate()?;
            Ok(cfg)
        });
        let cfg = match cfg {
            Ok(cfg) => {
                last_error = None;
                cfg
            }
            Err(err) => {
                let message = format!("設定が不正なため実行を停止しています: {err}");
                if last_error.as_ref() != Some(&message) {
                    logger.error(&message);
                    reporter.job_errored(message.clone());
                    last_error = Some(message);
                }
                set_next(&control, reporter.as_ref(), None);
                match rx.recv_timeout(Duration::from_secs(30)) {
                    Ok(SchedulerCommand::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
                    Ok(SchedulerCommand::RunNow) => {
                        control.run_now_pending.store(false, Ordering::SeqCst);
                        reporter.status_changed();
                    }
                    _ => {}
                }
                continue;
            }
        };
        let schedule = parse_schedule(&cfg.schedule_time).expect("validated schedule");
        let now = Local::now();
        if observed_at.is_some_and(|previous| clock_rewound(previous, now)) {
            armed_schedule = None;
        }
        observed_at = Some(now);
        if armed_schedule.as_ref() != Some(&cfg.schedule_time) {
            let now = Local::now();
            deadline = Some(
                if startup && should_catch_up(now, schedule, cfg.last_run_at) {
                    now
                } else {
                    next_run_after(now, schedule)
                },
            );
            armed_schedule = Some(cfg.schedule_time.clone());
            startup = false;
        }
        set_next(&control, reporter.as_ref(), deadline);
        let now = Local::now();
        if deadline.is_some_and(|due| now >= due)
            && control
                .run_now_pending
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
        {
            execute_job(&store, &logger, reporter.as_ref(), &cfg, &control);
            deadline = Some(next_run_after(Local::now(), schedule));
            continue;
        }
        // Poll wall-clock time after resume / clock changes instead of sleeping a day.
        let wait = poll_wait(deadline, now);
        match rx.recv_timeout(wait) {
            Ok(SchedulerCommand::RunNow) => {
                if control.stopping.load(Ordering::SeqCst) {
                    break;
                }
                logger.info("Manual backup triggered");
                execute_job(&store, &logger, reporter.as_ref(), &cfg, &control);
                // Do not move the armed daily deadline when a manual run crosses it.
            }
            Ok(SchedulerCommand::ReloadConfig) => armed_schedule = None,
            Ok(SchedulerCommand::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
    control.run_now_pending.store(false, Ordering::SeqCst);
    set_next(&control, reporter.as_ref(), None);
}

fn clock_rewound(previous: DateTime<Local>, now: DateTime<Local>) -> bool {
    now < previous - chrono::Duration::seconds(1) || now.offset() != previous.offset()
}

fn poll_wait(deadline: Option<DateTime<Local>>, now: DateTime<Local>) -> Duration {
    deadline
        .map(|due| (due - now).to_std().unwrap_or(Duration::ZERO))
        .unwrap_or(Duration::from_secs(30))
        .min(Duration::from_secs(30))
}

fn set_next(control: &SchedulerSender, reporter: &dyn JobReporter, value: Option<DateTime<Local>>) {
    let changed = {
        let mut current = control.next_at.lock().unwrap_or_else(|e| e.into_inner());
        let changed = *current != value;
        *current = value;
        changed
    };
    if changed {
        reporter.status_changed();
    }
}

fn execute_job(
    store: &ConfigStore,
    logger: &Logger,
    reporter: &dyn JobReporter,
    cfg: &Config,
    control: &SchedulerSender,
) {
    // Never leave the busy flag stuck if an unexpected panic escapes the job.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_job_with_waiter(store, logger, reporter, cfg, |path| {
            wait_cancellable(
                path,
                Duration::from_secs(crate::backup::DRIVE_WAIT_SECONDS),
                Duration::from_secs(10),
                &control.stopping,
            )
        });
    }));
    control.run_now_pending.store(false, Ordering::SeqCst);
    if result.is_err() {
        report_failure(
            store,
            logger,
            reporter,
            "バックアップ処理が予期せず中断しました".into(),
        );
    }
    reporter.status_changed();
}

fn report_failure(
    store: &ConfigStore,
    logger: &Logger,
    reporter: &dyn JobReporter,
    message: String,
) {
    logger.error(&message);
    let result = store.update(|updated| {
        updated.last_error = Some(JobFailure {
            at: Local::now(),
            message: message.clone(),
        });
    });
    let message = match result {
        Ok(_) => message,
        Err(err) => {
            let message = format!("{message}; エラー状態の保存にも失敗しました: {err}");
            logger.error(&message);
            message
        }
    };
    reporter.job_errored(message);
}

fn run_job_with_waiter(
    store: &ConfigStore,
    logger: &Logger,
    reporter: &dyn JobReporter,
    cfg: &Config,
    wait: impl FnOnce(&std::path::Path) -> Result<(), DriveWaitError>,
) {
    let (src, dest) = match (cfg.source.as_ref(), cfg.destination.as_ref()) {
        (Some(s), Some(d)) => (s.clone(), d.clone()),
        _ => {
            report_failure(
                store,
                logger,
                reporter,
                "監視元または出力先が未設定です。設定画面でフォルダを選択してください。"
                    .to_string(),
            );
            return;
        }
    };

    reporter.job_started();

    if !src.is_dir() {
        report_failure(
            store,
            logger,
            reporter,
            format!(
                "バックアップに失敗しました。監視元フォルダを確認してください: {}",
                src.display()
            ),
        );
        return;
    }

    logger.info(&format!(
        "Backup starting: {} -> {}",
        src.display(),
        dest.display()
    ));

    if let Err(err) = wait(&dest) {
        if matches!(err, DriveWaitError::Cancelled) {
            logger.info("Drive wait cancelled during shutdown");
            return;
        }
        let msg = format!(
            "出力先を確認できませんでした。Google Drive の起動と出力先を確認してください: {err}"
        );
        report_failure(store, logger, reporter, msg);
        return;
    }

    match BackupJob::new(&src, &dest)
        .with_excluded_folders(cfg.excluded_folders.clone())
        .with_excluded_folder_names(cfg.excluded_folder_names.clone())
        .run()
    {
        Ok(outcome) => {
            logger.info(&format!(
                "Backup complete: {} copied, {} errors, {} expired, {} deleted",
                outcome.summary.copied,
                outcome.summary.errors,
                outcome.expired_files.len(),
                outcome.deleted_files.len()
            ));
            for f in &outcome.copied_files {
                logger.info(&format!("  copied: {}", f.display()));
            }
            for f in &outcome.expired_files {
                logger.info(&format!(
                    "  expired (2 months since source update): {}",
                    f.display()
                ));
            }
            for f in &outcome.deleted_files {
                logger.info(&format!("  deleted expired backup: {}", f.display()));
            }
            for f in &outcome.retained_files {
                logger.warn(&format!(
                    "  retained untracked or externally changed backup: {}",
                    f.display()
                ));
            }
            for (f, err) in &outcome.errored_files {
                logger.warn(&format!("  error: {} — {}", f.display(), err));
            }
            for item in &outcome.orphans {
                logger.info(&format!(
                    "  retained orphan/protected backup: {} — {}",
                    item.backup.display(),
                    item.reason
                ));
            }
            if let Err(err) = store.update(|updated| {
                updated.last_run_at = Some(Local::now());
                updated.last_summary = Some(outcome.summary);
                updated.last_error = None;
            }) {
                report_failure(
                    store,
                    logger,
                    reporter,
                    format!(
                        "バックアップ処理は完了しましたが、実行結果を保存できませんでした: {err}"
                    ),
                );
                return;
            }
            reporter.job_finished(outcome.summary);
        }
        Err(err) => {
            let msg = format!("バックアップに失敗しました。設定とログを確認してください: {err}");
            report_failure(store, logger, reporter, msg);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, TimeZone};

    #[test]
    fn manual_requests_are_reserved_once_and_rejected_during_shutdown() {
        let (tx, rx) = mpsc::channel();
        let sender = SchedulerSender {
            tx,
            run_now_pending: Arc::new(AtomicBool::new(false)),
            stopping: Arc::new(AtomicBool::new(false)),
            next_at: Arc::new(Mutex::new(None)),
        };
        assert!(sender.run_now().unwrap());
        assert!(!sender.run_now().unwrap());
        assert!(sender.is_busy());
        assert!(matches!(rx.try_recv(), Ok(SchedulerCommand::RunNow)));
        assert!(rx.try_recv().is_err());
        sender.shutdown().unwrap();
        assert!(sender.run_now().is_err());
    }

    #[test]
    fn invalid_schedule_never_runs_a_job() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(ConfigStore::new(tmp.path().join("config.json")));
        store
            .save(&Config {
                schedule_time: "invalid".into(),
                ..Config::default()
            })
            .unwrap();
        let logger = Arc::new(Logger::open(&tmp.path().join("log")).unwrap());
        struct Reporter(std::sync::mpsc::Sender<String>);
        impl JobReporter for Reporter {
            fn job_started(&self) {
                self.0.send("started".into()).unwrap();
            }
            fn job_errored(&self, message: String) {
                let _ = self.0.send(message);
            }
        }
        let (tx, rx) = mpsc::channel();
        let handle = start(store, logger, Arc::new(Reporter(tx)));
        let sender = handle.sender();
        assert!(rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .contains("設定が不正"));
        assert!(sender.next_run_at().is_none());
        sender.run_now().unwrap();
        assert!(handle.shutdown_with_timeout(Duration::from_secs(2)));
        assert!(rx.try_iter().all(|event| event != "started"));
    }

    #[test]
    fn failed_channel_does_not_leave_a_pending_run() {
        let (tx, rx) = mpsc::channel();
        let sender = SchedulerSender {
            tx,
            run_now_pending: Arc::new(AtomicBool::new(false)),
            stopping: Arc::new(AtomicBool::new(false)),
            next_at: Arc::new(Mutex::new(None)),
        };
        drop(rx);
        assert!(sender.run_now().is_err());
        assert!(!sender.is_busy());
    }

    mod failure_state {
        use super::*;
        use std::{fs, sync::Mutex};
        use tempfile::{tempdir, TempDir};

        #[derive(Default)]
        struct Reporter {
            errors: Mutex<Vec<String>>,
            finished: Mutex<Vec<JobSummary>>,
        }

        impl JobReporter for Reporter {
            fn job_errored(&self, message: String) {
                self.errors.lock().unwrap().push(message);
            }
            fn job_finished(&self, summary: JobSummary) {
                self.finished.lock().unwrap().push(summary);
            }
        }

        fn fixture() -> (TempDir, ConfigStore, Logger, Config, Reporter) {
            let tmp = tempdir().unwrap();
            let source = tmp.path().join("260930(1)_project");
            let dest = tmp.path().join("dest");
            fs::create_dir_all(&source).unwrap();
            fs::create_dir_all(&dest).unwrap();
            fs::write(source.join("main.prproj"), "project").unwrap();
            let store = ConfigStore::new(tmp.path().join("config.json"));
            let cfg = Config {
                source: Some(source),
                destination: Some(dest),
                last_run_at: Some(Local::now() - chrono::Duration::days(1)),
                last_summary: Some(JobSummary {
                    copied: 4,
                    errors: 0,
                }),
                last_error: Some(JobFailure {
                    at: Local::now(),
                    message: "previous failure".into(),
                }),
                ..Config::default()
            };
            store.save(&cfg).unwrap();
            let logger = Logger::open(&tmp.path().join("backup.log")).unwrap();
            (tmp, store, logger, cfg, Reporter::default())
        }

        fn assert_failed(store: &ConfigStore, cfg: &Config, reporter: &Reporter, expected: &str) {
            let loaded = ConfigStore::new(store.path()).load().unwrap();
            assert_eq!(loaded.last_run_at, cfg.last_run_at);
            assert_eq!(loaded.last_summary, cfg.last_summary);
            let failure = loaded.last_error.unwrap();
            assert!(failure.message.contains(expected), "{}", failure.message);
            assert!(failure.at >= cfg.last_error.as_ref().unwrap().at);
            assert_eq!(reporter.errors.lock().unwrap().len(), 1);
            assert!(reporter.finished.lock().unwrap().is_empty());
        }

        #[test]
        fn timeout_is_persisted_without_losing_previous_result() {
            let (_tmp, store, logger, cfg, reporter) = fixture();
            run_job_with_waiter(&store, &logger, &reporter, &cfg, |path| {
                Err(DriveWaitError::Timeout(path.to_path_buf()))
            });
            assert_failed(&store, &cfg, &reporter, "出力先を確認できませんでした");
        }

        #[test]
        fn missing_configuration_is_persisted_without_waiting() {
            let (_tmp, store, logger, mut cfg, reporter) = fixture();
            cfg.source = None;
            run_job_with_waiter(&store, &logger, &reporter, &cfg, |_| {
                panic!("must not wait")
            });
            assert_failed(&store, &cfg, &reporter, "未設定");
        }

        #[test]
        fn backup_failure_is_persisted() {
            let (tmp, store, logger, mut cfg, reporter) = fixture();
            cfg.source = Some(tmp.path().join("missing"));
            run_job_with_waiter(&store, &logger, &reporter, &cfg, |_| Ok(()));
            assert_failed(&store, &cfg, &reporter, "バックアップに失敗");
        }

        #[test]
        fn success_clears_failure_and_updates_summary() {
            let (_tmp, store, logger, cfg, reporter) = fixture();
            run_job_with_waiter(&store, &logger, &reporter, &cfg, |_| Ok(()));
            let loaded = store.load().unwrap();
            assert!(loaded.last_error.is_none());
            assert!(loaded.last_run_at > cfg.last_run_at);
            assert_eq!(
                loaded.last_summary,
                Some(JobSummary {
                    copied: 1,
                    errors: 0
                })
            );
            assert!(reporter.errors.lock().unwrap().is_empty());
            assert_eq!(reporter.finished.lock().unwrap().len(), 1);
        }

        #[test]
        fn partial_failure_is_recorded_in_summary() {
            let (_tmp, store, logger, cfg, reporter) = fixture();
            fs::create_dir(
                cfg.destination
                    .as_ref()
                    .unwrap()
                    .join(format!("main{}", crate::backup::BACKUP_SUFFIX)),
            )
            .unwrap();
            run_job_with_waiter(&store, &logger, &reporter, &cfg, |_| Ok(()));
            let loaded = store.load().unwrap();
            assert!(loaded.last_error.is_none());
            assert_eq!(
                loaded.last_summary,
                Some(JobSummary {
                    copied: 0,
                    errors: 1
                })
            );
        }

        #[test]
        fn persistence_failure_does_not_report_success() {
            let (_tmp, store, logger, cfg, reporter) = fixture();
            fs::create_dir(store.backup_path()).unwrap();
            run_job_with_waiter(&store, &logger, &reporter, &cfg, |_| Ok(()));
            assert!(reporter.finished.lock().unwrap().is_empty());
            let errors = reporter.errors.lock().unwrap();
            assert_eq!(errors.len(), 1);
            assert!(errors[0].contains("実行結果を保存できませんでした"));
            assert!(errors[0].contains("エラー状態の保存にも失敗"));
        }
    }

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

    #[test]
    fn wall_clock_polling_is_bounded_and_overdue_runs_are_immediate() {
        let now = dt(2026, 9, 30, 10, 0);
        assert_eq!(
            poll_wait(Some(now - chrono::Duration::hours(8)), now),
            Duration::ZERO
        );
        assert_eq!(
            poll_wait(Some(now + chrono::Duration::hours(8)), now),
            Duration::from_secs(30)
        );
        assert!(clock_rewound(now, now - chrono::Duration::hours(1)));
        assert!(!clock_rewound(now, now + chrono::Duration::hours(1)));
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
