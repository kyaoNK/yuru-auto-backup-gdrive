use std::path::{Path, PathBuf};
use std::{fs, io};

use crate::atomic_file;
use thiserror::Error;

const USER_HOME_FOLDER: &str = "yuru-auto-backup-gdrive";
const INSTALL_FOLDER_NAME: &str = "yuru-auto-backup-gdrive";

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
        let home = user_home_path()?;
        let exe_dir = current_exe_dir();
        let portable = exe_dir.as_ref().map(|dir| dir.join("data"));

        if let (Some(exe_dir), Some(portable)) = (exe_dir.as_ref(), portable.as_ref()) {
            if is_managed_install_dir(exe_dir) {
                migrate_existing_state(portable, &home).map_err(|source| {
                    AppDirError::CreateFailed {
                        path: home.clone(),
                        source,
                    }
                })?;
                create_dir_all(&home)?;
                return Ok(Self { root: home });
            }
        }

        if let Some(portable) = portable.as_ref().filter(|p| has_existing_state(p)) {
            if let Some(writable) = try_portable() {
                return Ok(Self { root: writable });
            }
            return Err(AppDirError::CreateFailed {
                path: portable.clone(),
                source: io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "existing portable state is not writable",
                ),
            });
        }
        if has_existing_state(&home) {
            create_dir_all(&home)?;
            return Ok(Self { root: home });
        }

        if let Some(portable) = try_portable() {
            return Ok(Self { root: portable });
        }
        create_dir_all(&home)?;
        Ok(Self { root: home })
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
    let exe_dir = current_exe_dir()?;
    let data = exe_dir.join("data");

    if fs::create_dir_all(&data).is_err() {
        return None;
    }

    let _probe = tempfile::NamedTempFile::new_in(&data).ok()?;

    Some(data)
}

fn current_exe_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    exe.parent().map(Path::to_path_buf)
}

fn user_home_path() -> Result<PathBuf, AppDirError> {
    let home = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .ok_or(AppDirError::HomeNotFound)?;
    Ok(home.join(USER_HOME_FOLDER))
}

fn has_existing_state(path: &Path) -> bool {
    path.join("config.json").exists()
        || path.join("config.json.bak").exists()
        || path.join("logs").join("backup.log").exists()
}

fn is_managed_install_dir(exe_dir: &Path) -> bool {
    ["LOCALAPPDATA", "ProgramFiles", "ProgramFiles(x86)"]
        .iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .map(|base| base.join(INSTALL_FOLDER_NAME))
        .any(|install_root| path_starts_with_ci(exe_dir, &install_root))
}

fn path_starts_with_ci(path: &Path, base: &Path) -> bool {
    let path = path.to_string_lossy().replace('/', "\\").to_lowercase();
    let mut base = base.to_string_lossy().replace('/', "\\").to_lowercase();
    while base.ends_with('\\') {
        base.pop();
    }
    path == base || path.starts_with(&format!("{base}\\"))
}

fn migrate_existing_state(from: &Path, to: &Path) -> io::Result<()> {
    if !has_existing_state(from) {
        return Ok(());
    }

    fs::create_dir_all(to)?;
    copy_if_missing(&from.join("config.json"), &to.join("config.json"))?;
    copy_if_missing(&from.join("config.json.bak"), &to.join("config.json.bak"))?;
    copy_if_missing(
        &from.join("logs").join("backup.log"),
        &to.join("logs").join("backup.log"),
    )?;
    Ok(())
}

fn copy_if_missing(from: &Path, to: &Path) -> std::io::Result<()> {
    if !from.exists() || to.exists() {
        return Ok(());
    }
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    atomic_file::write(to, &fs::read(from)?)?;
    Ok(())
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

    #[test]
    fn has_existing_state_detects_config_backup_or_log() {
        let tmp = tempdir().unwrap();
        assert!(!has_existing_state(tmp.path()));

        fs::write(tmp.path().join("config.json"), "{}").unwrap();
        assert!(has_existing_state(tmp.path()));
    }

    #[test]
    fn path_starts_with_ci_matches_child_paths_case_insensitively() {
        assert!(path_starts_with_ci(
            Path::new(r"C:\Users\ME\AppData\Local\yuru-auto-backup-gdrive\data"),
            Path::new(r"c:\users\me\appdata\local\YURU-AUTO-BACKUP-GDRIVE"),
        ));
        assert!(!path_starts_with_ci(
            Path::new(r"C:\Users\ME\AppData\Local\yuru-auto-backup-gdrive2"),
            Path::new(r"C:\Users\ME\AppData\Local\yuru-auto-backup-gdrive"),
        ));
    }

    #[test]
    fn migrate_existing_state_copies_config_and_log_without_overwriting() {
        let from = tempdir().unwrap();
        let to = tempdir().unwrap();
        fs::create_dir_all(from.path().join("logs")).unwrap();
        fs::write(from.path().join("config.json"), "{\"source\":null}").unwrap();
        fs::write(from.path().join("logs").join("backup.log"), "old log").unwrap();
        fs::write(to.path().join("config.json"), "{\"source\":\"keep\"}").unwrap();

        migrate_existing_state(from.path(), to.path()).unwrap();

        assert_eq!(
            fs::read_to_string(to.path().join("config.json")).unwrap(),
            "{\"source\":\"keep\"}"
        );
        assert_eq!(
            fs::read_to_string(to.path().join("logs").join("backup.log")).unwrap(),
            "old log"
        );
    }
}
