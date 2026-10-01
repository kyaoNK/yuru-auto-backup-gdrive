use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::atomic_file;
use crate::backup::JobSummary;

const DEFAULT_SCHEDULE_TIME: &str = "09:00";

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read config file at {path}: {source}")]
    ReadFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse config file at {path}: {source}")]
    ParseFailed {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to write config file at {path}: {source}")]
    WriteFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to serialize config: {0}")]
    SerializeFailed(#[from] serde_json::Error),
    #[error("config store lock was poisoned")]
    LockPoisoned,
    #[error("invalid configuration: {0}")]
    Invalid(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JobFailure {
    pub at: DateTime<Local>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    #[serde(default)]
    pub source: Option<PathBuf>,
    #[serde(default)]
    pub destination: Option<PathBuf>,
    #[serde(default = "default_schedule_time")]
    pub schedule_time: String,
    #[serde(default = "default_true")]
    pub auto_start: bool,
    #[serde(default)]
    pub excluded_folders: Vec<PathBuf>,
    #[serde(default)]
    pub excluded_folder_names: Vec<String>,
    #[serde(default)]
    pub last_run_at: Option<DateTime<Local>>,
    #[serde(default)]
    pub last_summary: Option<JobSummary>,
    #[serde(default)]
    pub last_error: Option<JobFailure>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            source: None,
            destination: None,
            schedule_time: default_schedule_time(),
            auto_start: true,
            excluded_folders: Vec::new(),
            excluded_folder_names: Vec::new(),
            last_run_at: None,
            last_summary: None,
            last_error: None,
        }
    }
}

impl Config {
    pub fn validate(&self) -> Result<(), ConfigError> {
        crate::scheduler::parse_schedule(&self.schedule_time)
            .map_err(|e| ConfigError::Invalid(e.to_string()))?;
        for path in self
            .source
            .iter()
            .chain(self.destination.iter())
            .chain(self.excluded_folders.iter())
        {
            if !path.is_absolute() {
                return Err(ConfigError::Invalid(format!(
                    "絶対パスを指定してください: {}",
                    path.display()
                )));
            }
        }
        if let (Some(source), Some(destination)) = (&self.source, &self.destination) {
            let a = fs::canonicalize(source).unwrap_or_else(|_| source.clone());
            let b = fs::canonicalize(destination).unwrap_or_else(|_| destination.clone());
            if a.to_string_lossy()
                .eq_ignore_ascii_case(&b.to_string_lossy())
            {
                return Err(ConfigError::Invalid(
                    "監視元と出力先は別のフォルダにしてください".into(),
                ));
            }
        }
        if self
            .excluded_folder_names
            .iter()
            .any(|n| n.trim().is_empty() || n.contains(['/', '\\']) || n == "." || n == "..")
        {
            return Err(ConfigError::Invalid(
                "除外名は空欄・パスではなくフォルダ名を指定してください".into(),
            ));
        }
        Ok(())
    }
}

fn default_schedule_time() -> String {
    DEFAULT_SCHEDULE_TIME.to_string()
}

fn default_true() -> bool {
    true
}

pub struct ConfigStore {
    path: PathBuf,
    lock: Mutex<()>,
}

impl ConfigStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            lock: Mutex::new(()),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn backup_path(&self) -> PathBuf {
        self.path.with_extension("json.bak")
    }

    pub fn load(&self) -> Result<Config, ConfigError> {
        let _guard = self.lock.lock().map_err(lock_poisoned)?;
        self.load_unlocked()
    }

    fn load_unlocked(&self) -> Result<Config, ConfigError> {
        if !self
            .path
            .try_exists()
            .map_err(|source| ConfigError::ReadFailed {
                path: self.path.clone(),
                source,
            })?
        {
            if self.backup_path().exists() {
                return self.load_from_path(&self.backup_path());
            }
            return Ok(Config::default());
        }
        match self.load_from_path(&self.path) {
            Ok(cfg) => Ok(cfg),
            Err(primary_err) => {
                let backup = self.backup_path();
                if backup.exists() {
                    if let Ok(cfg) = self.load_from_path(&backup) {
                        return Ok(cfg);
                    }
                }
                Err(primary_err)
            }
        }
    }

    fn load_from_path(&self, path: &Path) -> Result<Config, ConfigError> {
        let text = fs::read_to_string(path).map_err(|source| ConfigError::ReadFailed {
            path: path.to_path_buf(),
            source,
        })?;
        serde_json::from_str(&text).map_err(|source| ConfigError::ParseFailed {
            path: path.to_path_buf(),
            source,
        })
    }

    pub fn save(&self, config: &Config) -> Result<(), ConfigError> {
        let _guard = self.lock.lock().map_err(lock_poisoned)?;
        self.save_unlocked(config)
    }

    pub fn update<F>(&self, f: F) -> Result<Config, ConfigError>
    where
        F: FnOnce(&mut Config),
    {
        let _guard = self.lock.lock().map_err(lock_poisoned)?;
        let mut config = self.load_unlocked()?;
        f(&mut config);
        self.save_unlocked(&config)?;
        Ok(config)
    }

    fn save_unlocked(&self, config: &Config) -> Result<(), ConfigError> {
        let text = serde_json::to_string_pretty(config)?;
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|source| ConfigError::WriteFailed {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        if self.load_from_path(&self.path).is_ok() {
            let backup = self.backup_path();
            let bytes = fs::read(&self.path).map_err(|source| ConfigError::ReadFailed {
                path: self.path.clone(),
                source,
            })?;
            atomic_file::write(&backup, &bytes).map_err(|source| ConfigError::WriteFailed {
                path: backup,
                source,
            })?;
        }
        atomic_file::write(&self.path, text.as_bytes()).map_err(|source| ConfigError::WriteFailed {
            path: self.path.clone(),
            source,
        })
    }

    /// Hold the store lock across OS changes and persistence so settings saves
    /// cannot race each other or overwrite a job's freshly persisted result.
    pub fn update_preferences(
        &self,
        mut next: Config,
        read_autostart: impl FnOnce() -> Result<bool, String>,
        mut set_autostart: impl FnMut(bool) -> Result<(), String>,
        reload: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        next.schedule_time = next.schedule_time.trim().to_string();
        next.excluded_folder_names = next
            .excluded_folder_names
            .into_iter()
            .map(|n| n.trim().to_string())
            .collect();
        next.validate().map_err(|e| e.to_string())?;
        let _guard = self.lock.lock().map_err(|e| lock_poisoned(e).to_string())?;
        let previous = self.load_unlocked().map_err(|e| e.to_string())?;
        next.last_run_at = previous.last_run_at;
        next.last_summary = previous.last_summary;
        next.last_error = previous.last_error;
        let old_autostart = read_autostart()
            .map_err(|e| format!("自動起動状態を確認できないため、設定は保存していません: {e}"))?;
        let changed = old_autostart != next.auto_start;
        if changed {
            if let Err(err) = set_autostart(next.auto_start) {
                let rollback = set_autostart(old_autostart);
                return Err(with_rollback_error(
                    format!("自動起動の変更に失敗したため、設定は保存していません: {err}"),
                    rollback,
                ));
            }
        }
        if let Err(err) = self.save_unlocked(&next) {
            let rollback = if changed {
                set_autostart(old_autostart)
            } else {
                Ok(())
            };
            return Err(with_rollback_error(
                format!("設定を保存できませんでした: {err}"),
                rollback,
            ));
        }
        reload().map_err(|e| format!("設定は保存済みですが、スケジュールに反映できませんでした。アプリを再起動してください: {e}"))
    }
}

fn with_rollback_error(message: String, rollback: Result<(), String>) -> String {
    match rollback {
        Ok(()) => message,
        Err(err) => format!("{message}; 自動起動状態の復元にも失敗しました。OS の自動起動設定を確認してください: {err}"),
    }
}

fn lock_poisoned<T>(_: PoisonError<T>) -> ConfigError {
    ConfigError::LockPoisoned
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    mod preferences {
        use super::*;
        use std::cell::{Cell, RefCell};

        #[test]
        fn success_preserves_runtime_state_and_reloads() {
            let (_tmp, store) = store_in_tmp();
            let previous = Config {
                last_run_at: Some(Local::now()),
                last_summary: Some(JobSummary {
                    copied: 2,
                    errors: 1,
                }),
                last_error: Some(JobFailure {
                    at: Local::now(),
                    message: "keep".into(),
                }),
                ..Config::default()
            };
            store.save(&previous).unwrap();
            let actual = Cell::new(true);
            let reloaded = Cell::new(false);
            let next = Config {
                auto_start: false,
                schedule_time: "18:00".into(),
                ..Config::default()
            };
            store
                .update_preferences(
                    next,
                    || Ok(actual.get()),
                    |value| {
                        actual.set(value);
                        Ok(())
                    },
                    || {
                        reloaded.set(true);
                        Ok(())
                    },
                )
                .unwrap();
            let saved = store.load().unwrap();
            assert!(!actual.get());
            assert!(reloaded.get());
            assert_eq!(saved.schedule_time, "18:00");
            assert_eq!(saved.last_run_at, previous.last_run_at);
            assert_eq!(saved.last_summary, previous.last_summary);
            assert_eq!(saved.last_error, previous.last_error);
        }

        #[test]
        fn os_change_failure_restores_state_without_saving_or_reloading() {
            let (_tmp, store) = store_in_tmp();
            store.save(&Config::default()).unwrap();
            let calls = RefCell::new(Vec::new());
            let result = store.update_preferences(
                Config {
                    auto_start: false,
                    ..Config::default()
                },
                || Ok(true),
                |value| {
                    calls.borrow_mut().push(value);
                    if value {
                        Ok(())
                    } else {
                        Err("OS error".into())
                    }
                },
                || panic!("must not reload"),
            );
            assert!(result.unwrap_err().contains("設定は保存していません"));
            assert_eq!(*calls.borrow(), vec![false, true]);
            assert_eq!(store.load().unwrap(), Config::default());
        }

        #[test]
        fn write_failure_restores_actual_os_state_not_stale_config() {
            let (_tmp, store) = store_in_tmp();
            store
                .save(&Config {
                    auto_start: false,
                    ..Config::default()
                })
                .unwrap();
            fs::create_dir(store.backup_path()).unwrap();
            let actual = Cell::new(true);
            let result = store.update_preferences(
                Config {
                    auto_start: false,
                    ..Config::default()
                },
                || Ok(actual.get()),
                |value| {
                    actual.set(value);
                    Ok(())
                },
                || panic!("must not reload"),
            );
            assert!(result.unwrap_err().contains("設定を保存できませんでした"));
            assert!(actual.get());
            assert!(!store.load().unwrap().auto_start);
        }

        #[test]
        fn reports_rollback_failure() {
            let (_tmp, store) = store_in_tmp();
            let result = store.update_preferences(
                Config {
                    auto_start: false,
                    ..Config::default()
                },
                || Ok(true),
                |_| Err("denied".into()),
                || panic!("must not reload"),
            );
            assert!(result.unwrap_err().contains("復元にも失敗"));
            assert!(!store.path().exists());
        }

        #[test]
        fn read_failure_leaves_settings_untouched() {
            let (_tmp, store) = store_in_tmp();
            let result = store.update_preferences(
                Config::default(),
                || Err("unavailable".into()),
                |_| panic!("must not change OS"),
                || panic!("must not reload"),
            );
            assert!(result.is_err());
            assert!(!store.path().exists());
        }

        #[test]
        fn reload_failure_explicitly_reports_that_settings_are_saved() {
            let (_tmp, store) = store_in_tmp();
            let result = store.update_preferences(
                Config {
                    schedule_time: "18:00".into(),
                    ..Config::default()
                },
                || Ok(true),
                |_| panic!("unchanged OS"),
                || Err("closed".into()),
            );
            let err = result.unwrap_err();
            assert!(err.contains("設定は保存済み"));
            assert!(err.contains("再起動"));
            assert_eq!(store.load().unwrap().schedule_time, "18:00");
        }

        #[test]
        fn invalid_time_does_not_touch_os_or_disk() {
            let (_tmp, store) = store_in_tmp();
            assert!(store
                .update_preferences(
                    Config {
                        schedule_time: "25:00".into(),
                        ..Config::default()
                    },
                    || panic!("must not query OS"),
                    |_| panic!("must not change OS"),
                    || panic!("must not reload")
                )
                .is_err());
            assert!(!store.path().exists());
        }
    }

    fn store_in_tmp() -> (tempfile::TempDir, ConfigStore) {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("config.json");
        let store = ConfigStore::new(path);
        (tmp, store)
    }

    #[test]
    fn recovery_does_not_overwrite_a_valid_backup_with_corrupt_primary() {
        let (_tmp, store) = store_in_tmp();
        let good = Config {
            schedule_time: "11:00".into(),
            ..Config::default()
        };
        store.save(&good).unwrap();
        fs::copy(store.path(), store.backup_path()).unwrap();
        fs::write(store.path(), "broken").unwrap();
        store
            .update(|cfg| cfg.schedule_time = "12:00".into())
            .unwrap();
        let backup: Config =
            serde_json::from_slice(&fs::read(store.backup_path()).unwrap()).unwrap();
        assert_eq!(backup, good);
        fs::remove_file(store.path()).unwrap();
        assert_eq!(store.load().unwrap(), good);
    }

    #[test]
    fn validates_paths_and_exclusion_names_but_allows_offline_drive() {
        let mut cfg = Config {
            source: Some(PathBuf::from("relative")),
            ..Config::default()
        };
        assert!(cfg.validate().is_err());
        cfg.source = None;
        cfg.excluded_folder_names = vec!["../folder".into()];
        assert!(cfg.validate().is_err());
        cfg.excluded_folder_names.clear();
        let temp = tempdir().unwrap();
        cfg.destination = Some(temp.path().join("not-mounted"));
        assert!(cfg.validate().is_ok());
        cfg.source = cfg.destination.clone();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn load_returns_default_when_file_missing() {
        let (_tmp, store) = store_in_tmp();
        let cfg = store.load().unwrap();
        assert_eq!(cfg, Config::default());
        assert_eq!(cfg.schedule_time, "09:00");
        assert!(cfg.auto_start);
    }

    #[test]
    fn save_then_load_roundtrip() {
        let (_tmp, store) = store_in_tmp();
        let cfg = Config {
            source: Some(PathBuf::from("D:/src")),
            destination: Some(PathBuf::from("E:/dest")),
            schedule_time: "14:30".into(),
            auto_start: false,
            last_summary: Some(JobSummary {
                copied: 3,
                errors: 1,
            }),
            ..Config::default()
        };

        store.save(&cfg).unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(cfg, loaded);
    }

    #[test]
    fn save_uses_camel_case_keys_matching_design_doc() {
        let (_tmp, store) = store_in_tmp();
        let cfg = Config::default();
        store.save(&cfg).unwrap();

        let text = fs::read_to_string(store.path()).unwrap();
        assert!(text.contains("\"scheduleTime\""));
        assert!(text.contains("\"autoStart\""));
        assert!(text.contains("\"lastRunAt\""));
        assert!(text.contains("\"lastSummary\""));
    }

    #[test]
    fn load_applies_defaults_for_missing_fields() {
        let (_tmp, store) = store_in_tmp();
        fs::write(store.path(), "{}").unwrap();

        let cfg = store.load().unwrap();
        assert_eq!(cfg.schedule_time, "09:00");
        assert!(cfg.auto_start);
        assert!(cfg.source.is_none());
        assert!(cfg.last_error.is_none());
    }

    #[test]
    fn failure_state_survives_reopening_store() {
        let (_tmp, store) = store_in_tmp();
        let failure = JobFailure {
            at: Local::now(),
            message: "Drive unavailable".into(),
        };
        store
            .update(|cfg| cfg.last_error = Some(failure.clone()))
            .unwrap();
        assert_eq!(
            ConfigStore::new(store.path()).load().unwrap().last_error,
            Some(failure)
        );
    }

    #[test]
    fn save_creates_parent_dir_if_missing() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("nested").join("config.json");
        let store = ConfigStore::new(&path);

        store.save(&Config::default()).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn save_cleans_up_tmp_file_on_success() {
        let (_tmp, store) = store_in_tmp();
        store.save(&Config::default()).unwrap();

        let tmp_path = store.path().with_extension("json.tmp");
        assert!(!tmp_path.exists());
    }

    #[test]
    fn save_keeps_previous_config_as_backup() {
        let (_tmp, store) = store_in_tmp();
        let first = Config {
            schedule_time: "08:00".into(),
            ..Config::default()
        };
        let second = Config {
            schedule_time: "10:00".into(),
            ..Config::default()
        };

        store.save(&first).unwrap();
        store.save(&second).unwrap();

        let backup_text = fs::read_to_string(store.backup_path()).unwrap();
        let backup: Config = serde_json::from_str(&backup_text).unwrap();
        assert_eq!(backup.schedule_time, "08:00");
        assert_eq!(store.load().unwrap().schedule_time, "10:00");
    }

    #[test]
    fn load_falls_back_to_backup_when_primary_is_broken() {
        let (_tmp, store) = store_in_tmp();
        let cfg = Config {
            schedule_time: "11:30".into(),
            ..Config::default()
        };
        store.save(&cfg).unwrap();
        fs::write(store.backup_path(), serde_json::to_string(&cfg).unwrap()).unwrap();
        fs::write(store.path(), "{ broken json").unwrap();

        let loaded = store.load().unwrap();
        assert_eq!(loaded.schedule_time, "11:30");
    }

    #[test]
    fn load_reports_error_when_primary_is_broken_and_no_backup_works() {
        let (_tmp, store) = store_in_tmp();
        fs::write(store.path(), "{ broken json").unwrap();

        assert!(store.load().is_err());
        assert_eq!(fs::read_to_string(store.path()).unwrap(), "{ broken json");
    }

    #[test]
    fn update_loads_modifies_and_saves_under_one_store_operation() {
        let (_tmp, store) = store_in_tmp();
        store
            .save(&Config {
                schedule_time: "08:00".into(),
                ..Config::default()
            })
            .unwrap();

        let updated = store
            .update(|cfg| {
                cfg.schedule_time = "17:45".into();
                cfg.last_summary = Some(JobSummary {
                    copied: 2,
                    errors: 0,
                });
            })
            .unwrap();

        assert_eq!(updated.schedule_time, "17:45");
        assert_eq!(store.load().unwrap().last_summary.unwrap().copied, 2);
    }
}
