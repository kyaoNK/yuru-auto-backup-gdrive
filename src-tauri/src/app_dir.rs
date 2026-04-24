use std::fs;
use std::path::{Path, PathBuf};

use thiserror::Error;

const USER_HOME_FOLDER: &str = "yuru-auto-backup-gdrive";

#[derive(Debug, Error)]
pub enum AppDirError {
    #[error("could not determine user home directory")]
    HomeNotFound,
    #[error("failed to create data directory at {path}: {source}")]
    CreateFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

pub struct AppDir {
    root: PathBuf,
}

impl AppDir {
    pub fn resolve() -> Result<Self, AppDirError> {
        if let Some(portable) = try_portable() {
            return Ok(Self { root: portable });
        }
        let root = user_home_fallback()?;
        Ok(Self { root })
    }

    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn config_path(&self) -> PathBuf {
        self.root.join("config.json")
    }

    pub fn log_dir(&self) -> PathBuf {
        self.root.join("logs")
    }

    pub fn log_file(&self) -> PathBuf {
        self.log_dir().join("backup.log")
    }

    pub fn ensure_exists(&self) -> Result<(), AppDirError> {
        create_dir_all(&self.root)?;
        create_dir_all(&self.log_dir())?;
        Ok(())
    }
}

fn try_portable() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?.to_path_buf();
    let data = exe_dir.join("data");

    if fs::create_dir_all(&data).is_err() {
        return None;
    }

    let probe = data.join(".write_probe");
    if fs::write(&probe, b"").is_err() {
        return None;
    }
    let _ = fs::remove_file(&probe);

    Some(data)
}

fn user_home_fallback() -> Result<PathBuf, AppDirError> {
    let home = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .ok_or(AppDirError::HomeNotFound)?;
    let root = home.join(USER_HOME_FOLDER);
    create_dir_all(&root)?;
    Ok(root)
}

fn create_dir_all(path: &Path) -> Result<(), AppDirError> {
    fs::create_dir_all(path).map_err(|source| AppDirError::CreateFailed {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn at_sets_root_and_derives_paths() {
        let tmp = tempdir().unwrap();
        let dir = AppDir::at(tmp.path());
        assert_eq!(dir.root(), tmp.path());
        assert_eq!(dir.config_path(), tmp.path().join("config.json"));
        assert_eq!(dir.log_dir(), tmp.path().join("logs"));
        assert_eq!(dir.log_file(), tmp.path().join("logs").join("backup.log"));
    }

    #[test]
    fn ensure_exists_creates_root_and_log_dir() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("nested").join("app");
        let dir = AppDir::at(&root);

        dir.ensure_exists().unwrap();

        assert!(root.is_dir());
        assert!(dir.log_dir().is_dir());
    }

    #[test]
    fn ensure_exists_is_idempotent() {
        let tmp = tempdir().unwrap();
        let dir = AppDir::at(tmp.path());

        dir.ensure_exists().unwrap();
        dir.ensure_exists().unwrap();

        assert!(dir.log_dir().is_dir());
    }
}
