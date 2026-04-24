use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use thiserror::Error;

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
    pub last_run_at: Option<DateTime<Local>>,
    #[serde(default)]
    pub last_summary: Option<JobSummary>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct JobSummary {
    pub copied: u32,
    pub errors: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            source: None,
            destination: None,
            schedule_time: default_schedule_time(),
            auto_start: true,
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
}

impl ConfigStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<Config, ConfigError> {
        if !self.path.exists() {
            return Ok(Config::default());
        }
        let text = fs::read_to_string(&self.path).map_err(|source| ConfigError::ReadFailed {
            path: self.path.clone(),
            source,
        })?;
        serde_json::from_str(&text).map_err(|source| ConfigError::ParseFailed {
            path: self.path.clone(),
            source,
        })
    }

    pub fn save(&self, config: &Config) -> Result<(), ConfigError> {
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
        fs::rename(&tmp, &self.path).map_err(|source| ConfigError::WriteFailed {
            path: self.path.clone(),
            source,
        })
    }
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
        let mut cfg = Config::default();
        cfg.source = Some(PathBuf::from("F:/src"));
        cfg.destination = Some(PathBuf::from("G:/dest"));
        cfg.schedule_time = "14:30".into();
        cfg.auto_start = false;
        cfg.last_summary = Some(JobSummary {
            copied: 3,
            errors: 1,
        });

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
}
