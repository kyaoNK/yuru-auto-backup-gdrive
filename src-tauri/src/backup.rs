use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use walkdir::WalkDir;

pub const TARGET_EXTENSION: &str = "prproj";
pub const EXCLUDE_PATH_KEYWORD: &str = "Auto-Save";
pub const FOLDER_NAME_REGEX: &str = r"^\d{6}\(";
pub const BACKUP_SUFFIX: &str = "_Latest.prproj";
pub const DRIVE_WAIT_SECONDS: u64 = 300;

#[derive(Debug, Error)]
pub enum BackupError {
    #[error("source directory does not exist: {0}")]
    SourceMissing(PathBuf),
    #[error("destination directory does not exist: {0}")]
    DestinationMissing(PathBuf),
    #[error("io error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct JobSummary {
    pub copied: u32,
    pub errors: u32,
}

#[derive(Debug, Default)]
pub struct JobOutcome {
    pub summary: JobSummary,
    pub copied_files: Vec<PathBuf>,
    pub errored_files: Vec<(PathBuf, String)>,
}

pub struct BackupJob {
    source: PathBuf,
    destination: PathBuf,
}

impl BackupJob {
    pub fn new(source: impl Into<PathBuf>, destination: impl Into<PathBuf>) -> Self {
        Self {
            source: source.into(),
            destination: destination.into(),
        }
    }

    pub fn run(&self) -> Result<JobOutcome, BackupError> {
        if !self.source.exists() {
            return Err(BackupError::SourceMissing(self.source.clone()));
        }
        if !self.destination.exists() {
            return Err(BackupError::DestinationMissing(self.destination.clone()));
        }

        let folder_re = Regex::new(FOLDER_NAME_REGEX).expect("invariant: FOLDER_NAME_REGEX is valid");
        let mut outcome = JobOutcome::default();

        for entry in WalkDir::new(&self.source).into_iter().filter_map(Result::ok) {
            let path = entry.path();
            if !should_backup(path, &folder_re) {
                continue;
            }

            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let dest_name = format!("{stem}{BACKUP_SUFFIX}");
            let dest = self.destination.join(dest_name);

            match copy_atomic(path, &dest) {
                Ok(()) => {
                    outcome.summary.copied += 1;
                    outcome.copied_files.push(path.to_path_buf());
                }
                Err(err) => {
                    outcome.summary.errors += 1;
                    outcome
                        .errored_files
                        .push((path.to_path_buf(), err.to_string()));
                }
            }
        }

        Ok(outcome)
    }
}

pub fn should_backup(path: &Path, folder_re: &Regex) -> bool {
    if !path.is_file() {
        return false;
    }
    if path.extension().and_then(|e| e.to_str()) != Some(TARGET_EXTENSION) {
        return false;
    }
    if path.to_string_lossy().contains(EXCLUDE_PATH_KEYWORD) {
        return false;
    }
    ancestor_folder_matches(path, folder_re)
}

fn ancestor_folder_matches(path: &Path, re: &Regex) -> bool {
    path.ancestors().any(|a| {
        a.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|name| re.is_match(name))
    })
}

fn copy_atomic(src: &Path, dest: &Path) -> io::Result<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut tmp_os: OsString = dest.as_os_str().to_owned();
    tmp_os.push(".part");
    let tmp = PathBuf::from(tmp_os);

    let copy_result = fs::copy(src, &tmp).map(|_| ());
    if copy_result.is_err() {
        let _ = fs::remove_file(&tmp);
        return copy_result;
    }

    if let Err(err) = fs::rename(&tmp, dest) {
        let _ = fs::remove_file(&tmp);
        return Err(err);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn regex() -> Regex {
        Regex::new(FOLDER_NAME_REGEX).unwrap()
    }

    fn touch(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, format!("content of {}", path.display())).unwrap();
    }

    mod should_backup {
        use super::*;

        #[test]
        fn includes_prproj_under_six_digit_folder() {
            let tmp = tempdir().unwrap();
            let p = tmp
                .path()
                .join("250304(3)_クイズ")
                .join("project.prproj");
            touch(&p);
            assert!(should_backup(&p, &regex()));
        }

        #[test]
        fn excludes_prproj_under_plain_folder() {
            let tmp = tempdir().unwrap();
            let p = tmp.path().join("MyProject").join("project.prproj");
            touch(&p);
            assert!(!should_backup(&p, &regex()));
        }

        #[test]
        fn excludes_non_prproj_under_six_digit_folder() {
            let tmp = tempdir().unwrap();
            let p = tmp.path().join("250304(1)_foo").join("notes.txt");
            touch(&p);
            assert!(!should_backup(&p, &regex()));
        }

        #[test]
        fn excludes_prproj_under_auto_save() {
            let tmp = tempdir().unwrap();
            let p = tmp
                .path()
                .join("250304(2)_foo")
                .join("Adobe Premiere Pro Auto-Save")
                .join("project.prproj");
            touch(&p);
            assert!(!should_backup(&p, &regex()));
        }

        #[test]
        fn includes_nested_prproj_when_any_ancestor_matches() {
            let tmp = tempdir().unwrap();
            let p = tmp
                .path()
                .join("250304(1)_outer")
                .join("sub")
                .join("deeper")
                .join("file.prproj");
            touch(&p);
            assert!(should_backup(&p, &regex()));
        }

        #[test]
        fn excludes_when_digits_are_not_six() {
            let tmp = tempdir().unwrap();
            let p = tmp.path().join("12345(X)_five").join("project.prproj");
            touch(&p);
            assert!(!should_backup(&p, &regex()));
        }

        #[test]
        fn requires_opening_paren_after_six_digits() {
            let tmp = tempdir().unwrap();
            let p = tmp.path().join("250304_nosep").join("project.prproj");
            touch(&p);
            assert!(!should_backup(&p, &regex()));
        }
    }

    mod backup_job {
        use super::*;

        struct Env {
            _tmp: tempfile::TempDir,
            src: PathBuf,
            dest: PathBuf,
        }

        fn env() -> Env {
            let tmp = tempdir().unwrap();
            let src = tmp.path().join("src");
            let dest = tmp.path().join("dest");
            fs::create_dir_all(&src).unwrap();
            fs::create_dir_all(&dest).unwrap();
            Env {
                _tmp: tmp,
                src,
                dest,
            }
        }

        #[test]
        fn copies_matching_file_with_latest_suffix() {
            let e = env();
            let f = e.src.join("250304(3)_クイズ").join("main.prproj");
            touch(&f);

            let outcome = BackupJob::new(&e.src, &e.dest).run().unwrap();

            assert_eq!(outcome.summary.copied, 1);
            assert_eq!(outcome.summary.errors, 0);
            assert!(e.dest.join("main_Latest.prproj").is_file());
        }

        #[test]
        fn flattens_nested_structure_to_destination() {
            let e = env();
            touch(&e.src.join("250304(1)_a").join("deep").join("one.prproj"));
            touch(&e.src.join("250304(2)_b").join("two.prproj"));

            let outcome = BackupJob::new(&e.src, &e.dest).run().unwrap();

            assert_eq!(outcome.summary.copied, 2);
            assert!(e.dest.join("one_Latest.prproj").is_file());
            assert!(e.dest.join("two_Latest.prproj").is_file());

            let entries: Vec<_> = fs::read_dir(&e.dest)
                .unwrap()
                .map(|r| r.unwrap().path())
                .collect();
            assert!(entries.iter().all(|p| p.is_file()));
        }

        #[test]
        fn overwrites_existing_destination_file() {
            let e = env();
            let src_file = e.src.join("250304(1)_a").join("p.prproj");
            touch(&src_file);
            fs::write(&src_file, "new content").unwrap();

            let existing = e.dest.join("p_Latest.prproj");
            fs::write(&existing, "old content").unwrap();

            BackupJob::new(&e.src, &e.dest).run().unwrap();

            let copied = fs::read_to_string(&existing).unwrap();
            assert_eq!(copied, "new content");
        }

        #[test]
        fn excludes_auto_save_and_non_prproj_and_non_matching_folders() {
            let e = env();
            touch(&e.src.join("250304(1)_ok").join("keep.prproj"));
            touch(
                &e.src
                    .join("250304(1)_ok")
                    .join("Adobe Premiere Pro Auto-Save")
                    .join("skip.prproj"),
            );
            touch(&e.src.join("250304(1)_ok").join("readme.txt"));
            touch(&e.src.join("plain_folder").join("skip.prproj"));

            let outcome = BackupJob::new(&e.src, &e.dest).run().unwrap();

            assert_eq!(outcome.summary.copied, 1);
            assert!(e.dest.join("keep_Latest.prproj").is_file());
            assert!(!e.dest.join("skip_Latest.prproj").is_file());
        }

        #[test]
        fn returns_empty_outcome_when_nothing_matches() {
            let e = env();
            touch(&e.src.join("plain").join("a.prproj"));
            touch(&e.src.join("250304(1)_ok").join("a.txt"));

            let outcome = BackupJob::new(&e.src, &e.dest).run().unwrap();

            assert_eq!(outcome.summary.copied, 0);
            assert_eq!(outcome.summary.errors, 0);
            assert_eq!(fs::read_dir(&e.dest).unwrap().count(), 0);
        }

        #[test]
        fn errors_when_source_missing() {
            let tmp = tempdir().unwrap();
            let dest = tmp.path().join("dest");
            fs::create_dir_all(&dest).unwrap();
            let err = BackupJob::new(tmp.path().join("missing"), &dest)
                .run()
                .unwrap_err();
            assert!(matches!(err, BackupError::SourceMissing(_)));
        }

        #[test]
        fn errors_when_destination_missing() {
            let tmp = tempdir().unwrap();
            let src = tmp.path().join("src");
            fs::create_dir_all(&src).unwrap();
            let err = BackupJob::new(&src, tmp.path().join("missing"))
                .run()
                .unwrap_err();
            assert!(matches!(err, BackupError::DestinationMissing(_)));
        }

        #[test]
        fn leaves_no_part_file_after_successful_copy() {
            let e = env();
            touch(&e.src.join("250304(1)_a").join("p.prproj"));

            BackupJob::new(&e.src, &e.dest).run().unwrap();

            let leftovers: Vec<_> = fs::read_dir(&e.dest)
                .unwrap()
                .filter_map(|r| r.ok())
                .filter(|e| {
                    e.path()
                        .to_string_lossy()
                        .ends_with(".part")
                })
                .collect();
            assert!(leftovers.is_empty());
        }
    }
}
