use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use thiserror::Error;

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
        }
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
        if !self.path.exists() {
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
                if matches!(primary_err, ConfigError::ParseFailed { .. }) {
                    Ok(Config::default())
                } else {
                    Err(primary_err)
                }
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
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, text).map_err(|source| ConfigError::WriteFailed {
            path: tmp.clone(),
            source,
        })?;
        if self.path.exists() {
            let backup = self.backup_path();
            fs::copy(&self.path, &backup).map_err(|source| ConfigError::WriteFailed {
                path: backup,
                source,
            })?;
        }
        fs::rename(&tmp, &self.path).map_err(|source| ConfigError::WriteFailed {
            path: self.path.clone(),
            source,
        })
    }
}

fn lock_poisoned<T>(_: PoisonError<T>) -> ConfigError {
    ConfigError::LockPoisoned
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn store_in_tmp() -> (tempfile::TempDir, ConfigStore) {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("config.json");
        let store = ConfigStore::new(path);
        (tmp, store)
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
    fn load_returns_default_when_primary_is_broken_and_no_backup_works() {
        let (_tmp, store) = store_in_tmp();
        fs::write(store.path(), "{ broken json").unwrap();

        let loaded = store.load().unwrap();
        assert_eq!(loaded, Config::default());
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
