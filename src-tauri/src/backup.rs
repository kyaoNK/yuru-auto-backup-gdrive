use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

use crate::atomic_file;
use chrono::{DateTime, Local, Months};
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use walkdir::WalkDir;

pub const TARGET_EXTENSION: &str = "prproj";
pub const EXCLUDE_PATH_KEYWORD: &str = "Auto-Save";
pub const FOLDER_NAME_REGEX: &str = r"^\d{6}\(";
pub const BACKUP_SUFFIX: &str = "_Latest.prproj";
pub const DRIVE_WAIT_SECONDS: u64 = 300;
pub const RETENTION_MONTHS: u32 = 2;
const MANIFEST_NAME: &str = ".yuru-backup-manifest.json";

#[derive(Debug, Serialize, Deserialize)]
struct ManagedBackup {
    source: PathBuf,
    size: u64,
    modified: SystemTime,
    #[serde(default)]
    sha256: Option<String>,
}

type Manifest = BTreeMap<String, ManagedBackup>;

fn expired(modified: SystemTime, now: DateTime<Local>) -> bool {
    DateTime::<Local>::from(modified)
        .checked_add_months(Months::new(RETENTION_MONTHS))
        .is_some_and(|deadline| now >= deadline)
}

fn fingerprint(path: &Path, source: &Path) -> io::Result<ManagedBackup> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() {
        return Err(io::Error::other("backup is not a regular file"));
    }
    Ok(ManagedBackup {
        source: fs::canonicalize(source)?,
        size: metadata.len(),
        modified: metadata.modified()?,
        sha256: Some(hash_file(path)?),
    })
}

#[derive(Debug, Error)]
pub enum BackupError {
    #[error("source directory does not exist: {0}")]
    SourceMissing(PathBuf),
    #[error("destination directory does not exist: {0}")]
    DestinationMissing(PathBuf),
    #[error("source and destination must be different directories")]
    SameDirectory,
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
    pub deleted_files: Vec<PathBuf>,
    pub expired_files: Vec<PathBuf>,
    pub retained_files: Vec<PathBuf>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletionPreview {
    pub checked_at: DateTime<Local>,
    pub candidates: Vec<PathBuf>,
    pub retained: Vec<PathBuf>,
    pub errors: Vec<(PathBuf, String)>,
}

pub struct BackupJob {
    source: PathBuf,
    destination: PathBuf,
    excluded_folders: Vec<PathBuf>,
    excluded_folder_names: Vec<String>,
}

impl BackupJob {
    pub fn new(source: impl Into<PathBuf>, destination: impl Into<PathBuf>) -> Self {
        Self {
            source: source.into(),
            destination: destination.into(),
            excluded_folders: Vec::new(),
            excluded_folder_names: Vec::new(),
        }
    }

    pub fn with_excluded_folders(mut self, folders: Vec<PathBuf>) -> Self {
        self.excluded_folders = folders;
        self
    }

    pub fn with_excluded_folder_names(mut self, names: Vec<String>) -> Self {
        self.excluded_folder_names = names;
        self
    }

    pub fn run(&self) -> Result<JobOutcome, BackupError> {
        self.run_at(Local::now())
    }

    fn run_at(&self, now: DateTime<Local>) -> Result<JobOutcome, BackupError> {
        self.execute_at(now, false)
    }

    pub fn preview_deletions(&self) -> Result<DeletionPreview, BackupError> {
        self.preview_at(Local::now())
    }

    fn preview_at(&self, now: DateTime<Local>) -> Result<DeletionPreview, BackupError> {
        let outcome = self.execute_at(now, true)?;
        Ok(DeletionPreview {
            checked_at: now,
            candidates: outcome.deleted_files,
            retained: outcome.retained_files,
            errors: outcome.errored_files,
        })
    }

    // In preview mode deleted_files contains candidates; no filesystem writes occur.
    fn execute_at(&self, now: DateTime<Local>, preview: bool) -> Result<JobOutcome, BackupError> {
        if !self.source.is_dir() {
            return Err(BackupError::SourceMissing(self.source.clone()));
        }
        if !self.destination.is_dir() {
            return Err(BackupError::DestinationMissing(self.destination.clone()));
        }
        if paths_equal_for_exclusion(&self.source, &self.destination) {
            return Err(BackupError::SameDirectory);
        }

        let folder_re =
            Regex::new(FOLDER_NAME_REGEX).expect("invariant: FOLDER_NAME_REGEX is valid");
        let lowered_names: Vec<String> = self
            .excluded_folder_names
            .iter()
            .filter(|n| !n.trim().is_empty())
            .map(|n| n.trim().to_lowercase())
            .collect();
        let mut effective_excluded_folders = self.excluded_folders.clone();
        if self
            .destination
            .ancestors()
            .any(|ancestor| paths_equal_for_exclusion(ancestor, &self.source))
        {
            effective_excluded_folders.push(self.destination.clone());
        }
        let mut outcome = JobOutcome::default();
        let manifest_path = self.destination.join(MANIFEST_NAME);
        let mut manifest: Manifest = match fs::read(&manifest_path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|err| BackupError::Io {
                path: manifest_path.clone(),
                source: io::Error::new(io::ErrorKind::InvalidData, err),
            })?,
            Err(err) if err.kind() == io::ErrorKind::NotFound => Manifest::new(),
            Err(source) => {
                return Err(BackupError::Io {
                    path: manifest_path,
                    source,
                })
            }
        };
        let mut manifest_changed = false;

        for entry in WalkDir::new(&self.source)
            .follow_links(false)
            .into_iter()
            .filter_entry(|entry| {
                !entry.file_type().is_symlink()
                    && (!entry.file_type().is_dir()
                        || !directory_excluded(
                            entry.path(),
                            &effective_excluded_folders,
                            &lowered_names,
                        ))
            })
        {
            let entry = match entry {
                Ok(entry) => entry,
                Err(err) => {
                    outcome.summary.errors += 1;
                    outcome.errored_files.push((
                        err.path().unwrap_or(&self.source).to_path_buf(),
                        err.to_string(),
                    ));
                    continue;
                }
            };
            let path = entry.path();
            if !should_backup(
                path,
                &folder_re,
                &effective_excluded_folders,
                &lowered_names,
            ) {
                continue;
            }

            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let dest_name = format!("{stem}{BACKUP_SUFFIX}");
            let dest = self.destination.join(&dest_name);

            let result = (|| -> io::Result<()> {
                let modified = fs::metadata(path)?.modified()?;
                if expired(modified, now) {
                    outcome.expired_files.push(path.to_path_buf());
                    if let Some(record) = manifest.get(&dest_name) {
                        match fingerprint(&dest, path) {
                            Ok(current)
                                if current.source == record.source
                                    && current.size == record.size
                                    && current.modified == record.modified
                                    && record.sha256.is_some()
                                    && current.sha256 == record.sha256 =>
                            {
                                if !preview {
                                    // Keep a Windows read lock until deletion, preventing an edit
                                    // from making this source active after the expiry check.
                                    let source_lock = open_stable_source(path)?;
                                    if source_lock.metadata()?.modified()? != modified {
                                        return Err(io::Error::other(
                                            "source changed before deletion",
                                        ));
                                    }
                                    fs::remove_file(&dest)?;
                                    manifest.remove(&dest_name);
                                    manifest_changed = true;
                                }
                                outcome.deleted_files.push(dest.clone());
                            }
                            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                                if !preview {
                                    manifest.remove(&dest_name);
                                    manifest_changed = true;
                                }
                            }
                            Err(err) => return Err(err),
                            _ => outcome.retained_files.push(dest.clone()),
                        }
                    } else if dest.exists() {
                        outcome.retained_files.push(dest.clone());
                    }
                    return Ok(());
                }

                if preview {
                    return Ok(());
                }
                copy_atomic(path, &dest)?;
                outcome.summary.copied += 1;
                outcome.copied_files.push(path.to_path_buf());
                manifest.insert(dest_name, fingerprint(&dest, path)?);
                manifest_changed = true;
                Ok(())
            })();
            if let Err(err) = result {
                outcome.summary.errors += 1;
                outcome
                    .errored_files
                    .push((path.to_path_buf(), err.to_string()));
            }
        }

        if manifest_changed {
            let result = (|| -> io::Result<()> {
                let bytes = serde_json::to_vec_pretty(&manifest).map_err(io::Error::other)?;
                atomic_file::write(&manifest_path, &bytes)
            })();
            if let Err(err) = result {
                outcome.summary.errors += 1;
                outcome.errored_files.push((manifest_path, err.to_string()));
            }
        }

        Ok(outcome)
    }
}

pub fn should_backup(
    path: &Path,
    folder_re: &Regex,
    excluded_folders: &[PathBuf],
    excluded_folder_names_lower: &[String],
) -> bool {
    if !path.is_file() {
        return false;
    }
    if !path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case(TARGET_EXTENSION))
    {
        return false;
    }
    if contains_case_insensitive_ascii(&path.to_string_lossy(), EXCLUDE_PATH_KEYWORD) {
        return false;
    }
    if !ancestor_folder_matches(path, folder_re) {
        return false;
    }
    if is_under_excluded_folder(path, excluded_folders) {
        return false;
    }
    if has_ancestor_with_excluded_name(path, excluded_folder_names_lower) {
        return false;
    }
    true
}

fn ancestor_folder_matches(path: &Path, re: &Regex) -> bool {
    path.ancestors().skip(1).any(|a| {
        a.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|name| re.is_match(name))
    })
}

fn is_under_excluded_folder(path: &Path, excluded: &[PathBuf]) -> bool {
    if excluded.is_empty() {
        return false;
    }
    path.ancestors()
        .skip(1)
        .any(|a| excluded.iter().any(|e| paths_equal_for_exclusion(a, e)))
}

fn has_ancestor_with_excluded_name(path: &Path, lowered_names: &[String]) -> bool {
    if lowered_names.is_empty() {
        return false;
    }
    path.ancestors().skip(1).any(|a| {
        a.file_name().and_then(|n| n.to_str()).is_some_and(|name| {
            let lower = name.to_lowercase();
            lowered_names.iter().any(|n| n == &lower)
        })
    })
}

fn copy_atomic(src: &Path, dest: &Path) -> io::Result<()> {
    copy_atomic_checked(src, dest, || {})
}

fn copy_atomic_checked(src: &Path, dest: &Path, after_copy: impl FnOnce()) -> io::Result<()> {
    let mut source = open_stable_source(src)?;
    let before = source.metadata()?;
    let mut temp = atomic_file::temporary(dest)?;
    let copied = io::copy(&mut source, &mut temp)?;
    after_copy();
    let after = fs::metadata(src)?;
    if copied != before.len()
        || before.len() != after.len()
        || before.modified()? != after.modified()?
    {
        return Err(io::Error::other(
            "source changed during copy; previous backup preserved",
        ));
    }
    // Also compare contents: timestamp precision alone cannot detect every change.
    source.seek(SeekFrom::Start(0))?;
    temp.as_file_mut().seek(SeekFrom::Start(0))?;
    if hash_reader(&mut source)? != hash_reader(temp.as_file_mut())? {
        return Err(io::Error::other("source content changed during copy"));
    }
    atomic_file::commit(temp, dest)
}

fn open_stable_source(path: &Path) -> io::Result<fs::File> {
    if !fs::symlink_metadata(path)?.file_type().is_file() {
        return Err(io::Error::other("source is not a regular file"));
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1); // FILE_SHARE_READ; deny concurrent writes and replacement.
    }
    options.open(path)
}

fn hash_reader(reader: &mut impl Read) -> io::Result<String> {
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn hash_file(path: &Path) -> io::Result<String> {
    hash_reader(&mut open_stable_source(path)?)
}

fn directory_excluded(path: &Path, folders: &[PathBuf], names: &[String]) -> bool {
    contains_case_insensitive_ascii(&path.to_string_lossy(), EXCLUDE_PATH_KEYWORD)
        || path
            .ancestors()
            .any(|a| folders.iter().any(|f| paths_equal_for_exclusion(a, f)))
        || path.ancestors().any(|a| {
            a.file_name()
                .is_some_and(|n| names.contains(&n.to_string_lossy().to_lowercase()))
        })
}

fn contains_case_insensitive_ascii(haystack: &str, needle: &str) -> bool {
    haystack
        .to_ascii_lowercase()
        .contains(&needle.to_ascii_lowercase())
}

fn paths_equal_for_exclusion(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }

    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(ca), Ok(cb)) => path_key(&ca) == path_key(&cb),
        _ => path_key(a) == path_key(b),
    }
}

fn path_key(path: &Path) -> String {
    let mut parts = Vec::new();
    for component in path.components() {
        let part = match component {
            Component::Prefix(prefix) => prefix.as_os_str().to_string_lossy().into_owned(),
            Component::RootDir => String::from(std::path::MAIN_SEPARATOR),
            Component::CurDir => continue,
            Component::ParentDir => String::from(".."),
            Component::Normal(s) => s.to_string_lossy().into_owned(),
        };
        parts.push(part);
    }

    let separator = std::path::MAIN_SEPARATOR.to_string();
    let joined = parts.join(&separator);
    if cfg!(windows) {
        joined.to_ascii_lowercase()
    } else {
        joined
    }
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

    #[test]
    fn cannot_copy_over_original_when_source_equals_destination() {
        let tmp = tempdir().unwrap();
        assert!(matches!(
            BackupJob::new(tmp.path(), tmp.path()).run(),
            Err(BackupError::SameDirectory)
        ));
    }

    #[test]
    fn destination_parent_does_not_exclude_the_entire_source() {
        let tmp = tempdir().unwrap();
        let source = tmp.path().join("260930(1)_project");
        touch(&source.join("main.prproj"));
        let outcome = BackupJob::new(&source, tmp.path()).run().unwrap();
        assert_eq!(outcome.summary.copied, 1);
        assert!(tmp.path().join(format!("main{BACKUP_SUFFIX}")).exists());
    }

    #[test]
    fn refuses_to_copy_while_an_editor_holds_a_write_handle() {
        let tmp = tempdir().unwrap();
        let source = tmp.path().join("source.prproj");
        let dest = tmp.path().join("dest.prproj");
        fs::write(&source, "new").unwrap();
        fs::write(&dest, "old").unwrap();
        let _editor = fs::OpenOptions::new().write(true).open(&source).unwrap();
        #[cfg(windows)]
        {
            assert!(copy_atomic(&source, &dest).is_err());
            assert_eq!(fs::read(&dest).unwrap(), b"old");
        }
    }

    #[test]
    fn failed_replace_preserves_destination_and_removes_temp() {
        let tmp = tempdir().unwrap();
        let source = tmp.path().join("source.prproj");
        let dest = tmp.path().join("dest.prproj");
        fs::write(&source, "new").unwrap();
        fs::create_dir(&dest).unwrap();
        fs::write(dest.join("keep"), "old").unwrap();
        assert!(copy_atomic(&source, &dest).is_err());
        assert!(dest.join("keep").exists());
        assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 2);
    }

    #[test]
    fn edit_during_copy_is_blocked_or_detected_before_replacement() {
        let tmp = tempdir().unwrap();
        let source = tmp.path().join("source.prproj");
        let dest = tmp.path().join("dest.prproj");
        fs::write(&source, "original").unwrap();
        fs::write(&dest, "old backup").unwrap();
        let result = copy_atomic_checked(&source, &dest, || {
            let edit = fs::write(&source, "changed while copying");
            #[cfg(windows)]
            assert!(edit.is_err());
            #[cfg(not(windows))]
            edit.unwrap();
        });
        #[cfg(windows)]
        {
            result.unwrap();
            assert_eq!(fs::read(&dest).unwrap(), b"original");
        }
        #[cfg(not(windows))]
        {
            assert!(result.is_err());
            assert_eq!(fs::read(&dest).unwrap(), b"old backup");
        }
        assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 2);
    }

    mod retention {
        use super::*;
        use chrono::TimeZone;

        fn date(month: u32, day: u32) -> DateTime<Local> {
            Local
                .with_ymd_and_hms(2026, month, day, 12, 0, 0)
                .single()
                .unwrap()
        }

        fn fixture() -> (tempfile::TempDir, BackupJob, PathBuf, PathBuf) {
            let tmp = tempdir().unwrap();
            let src = tmp.path().join("src");
            let dest = tmp.path().join("dest");
            let file = src.join("260101(1)_Project").join("main.prproj");
            touch(&file);
            fs::File::options()
                .write(true)
                .open(&file)
                .unwrap()
                .set_modified(date(1, 31).into())
                .unwrap();
            fs::create_dir_all(&dest).unwrap();
            let output = dest.join(format!("main{BACKUP_SUFFIX}"));
            (tmp, BackupJob::new(src, dest), file, output)
        }

        #[test]
        fn preserves_changed_content_even_with_identical_size_and_timestamp() {
            let (_tmp, job, _source, output) = fixture();
            job.run_at(date(2, 1)).unwrap();
            let metadata = fs::metadata(&output).unwrap();
            let contents = vec![b'x'; metadata.len() as usize];
            fs::write(&output, &contents).unwrap();
            fs::File::options()
                .write(true)
                .open(&output)
                .unwrap()
                .set_modified(metadata.modified().unwrap())
                .unwrap();
            assert!(job.preview_at(date(4, 1)).unwrap().candidates.is_empty());
            assert!(job.run_at(date(4, 1)).unwrap().deleted_files.is_empty());
            assert_eq!(fs::read(output).unwrap(), contents);
        }

        #[test]
        fn legacy_manifest_without_hash_is_not_authority_to_delete() {
            let (_tmp, job, _source, output) = fixture();
            job.run_at(date(2, 1)).unwrap();
            let manifest_path = job.destination.join(MANIFEST_NAME);
            let mut manifest: Manifest =
                serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
            for item in manifest.values_mut() {
                item.sha256 = None;
            }
            fs::write(manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
            assert!(job.run_at(date(4, 1)).unwrap().deleted_files.is_empty());
            assert!(output.exists());
        }

        #[test]
        fn preview_matches_deletion_without_modifying_files_or_manifest() {
            let (_tmp, job, source, output) = fixture();
            job.run_at(date(2, 1)).unwrap();
            let manifest_path = job.destination.join(MANIFEST_NAME);
            let manifest = fs::read(&manifest_path).unwrap();
            let contents = fs::read(&output).unwrap();
            let preview = job.preview_at(date(4, 1)).unwrap();
            assert_eq!(preview.candidates, vec![output.clone()]);
            assert!(preview.errors.is_empty());
            assert!(source.exists());
            assert_eq!(fs::read(&output).unwrap(), contents);
            assert_eq!(fs::read(&manifest_path).unwrap(), manifest);
            assert_eq!(
                job.run_at(date(4, 1)).unwrap().deleted_files,
                preview.candidates
            );
        }

        #[test]
        fn preview_never_copies_fresh_files_or_creates_manifest() {
            let (_tmp, job, _source, output) = fixture();
            assert!(job.preview_at(date(2, 1)).unwrap().candidates.is_empty());
            assert!(!output.exists());
            assert_eq!(fs::read_dir(&job.destination).unwrap().count(), 0);
        }

        #[test]
        fn preview_preserves_untracked_and_changed_backups() {
            let (_tmp, job, _source, output) = fixture();
            fs::write(&output, "legacy").unwrap();
            let preview = job.preview_at(date(4, 1)).unwrap();
            assert!(preview.candidates.is_empty());
            assert_eq!(preview.retained, vec![output.clone()]);
            job.run_at(date(2, 1)).unwrap();
            fs::write(&output, "external change").unwrap();
            let preview = job.preview_at(date(4, 1)).unwrap();
            assert!(preview.candidates.is_empty());
            assert_eq!(preview.retained, vec![output]);
        }

        #[test]
        fn preview_respects_exclusions() {
            let (_tmp, job, source, _output) = fixture();
            job.run_at(date(2, 1)).unwrap();
            let job = job.with_excluded_folders(vec![source.parent().unwrap().to_path_buf()]);
            assert!(job.preview_at(date(4, 1)).unwrap().candidates.is_empty());
        }

        #[test]
        fn preview_reports_bad_manifest_and_missing_destination() {
            let (_tmp, job, _source, _output) = fixture();
            fs::write(job.destination.join(MANIFEST_NAME), "broken").unwrap();
            assert!(job.preview_at(date(4, 1)).is_err());
            let missing = BackupJob::new(&job.source, job.destination.join("missing"));
            assert!(matches!(
                missing.preview_at(date(4, 1)),
                Err(BackupError::DestinationMissing(_))
            ));
        }

        #[test]
        fn preview_surfaces_inspection_errors() {
            let (_tmp, job, _source, output) = fixture();
            job.run_at(date(2, 1)).unwrap();
            fs::remove_file(&output).unwrap();
            fs::create_dir(&output).unwrap();
            let preview = job.preview_at(date(4, 1)).unwrap();
            assert!(preview.candidates.is_empty());
            assert_eq!(preview.errors.len(), 1);
            assert!(output.is_dir());
        }

        #[test]
        fn expires_at_calendar_month_boundary_not_sixty_days() {
            assert!(!expired(date(1, 31).into(), date(3, 30)));
            assert!(expired(date(1, 31).into(), date(3, 31)));
            assert!(!expired(date(7, 31).into(), date(9, 29)));
            assert!(expired(date(7, 31).into(), date(9, 30)));
            assert!(!expired(date(12, 1).into(), date(9, 30)));
        }

        #[test]
        fn deletes_tracked_backup_only_and_resumes_after_source_edit() {
            let (_tmp, job, source, output) = fixture();
            assert_eq!(job.run_at(date(2, 1)).unwrap().summary.copied, 1);
            assert!(output.exists());
            let result = job.run_at(date(3, 31)).unwrap();
            assert_eq!(result.summary.copied, 0);
            assert_eq!(result.summary.errors, 0);
            assert_eq!(result.deleted_files, vec![output.clone()]);
            assert!(!output.exists());
            assert!(source.exists());
            assert!(job.run_at(date(4, 1)).unwrap().deleted_files.is_empty());
            fs::File::options()
                .write(true)
                .open(&source)
                .unwrap()
                .set_modified(date(4, 1).into())
                .unwrap();
            assert_eq!(job.run_at(date(4, 1)).unwrap().summary.copied, 1);
            assert!(output.exists());
        }

        #[test]
        fn does_not_copy_expired_source_or_delete_untracked_backup() {
            let (_tmp, job, source, output) = fixture();
            fs::write(&output, "legacy backup").unwrap();
            let result = job.run_at(date(4, 1)).unwrap();
            assert_eq!(result.summary.copied, 0);
            assert_eq!(result.retained_files, vec![output.clone()]);
            assert_eq!(fs::read_to_string(output).unwrap(), "legacy backup");
            assert!(source.exists());
        }

        #[test]
        fn preserves_externally_changed_backup() {
            let (_tmp, job, _source, output) = fixture();
            job.run_at(date(2, 1)).unwrap();
            fs::write(&output, "external change").unwrap();
            let result = job.run_at(date(4, 1)).unwrap();
            assert_eq!(result.retained_files, vec![output.clone()]);
            assert_eq!(fs::read_to_string(output).unwrap(), "external change");
        }

        #[test]
        fn preserves_backups_when_source_is_missing_or_excluded() {
            let (_tmp, job, source, output) = fixture();
            job.run_at(date(2, 1)).unwrap();
            let job = job.with_excluded_folder_names(vec!["260101(1)_Project".into()]);
            assert!(job.run_at(date(4, 1)).unwrap().deleted_files.is_empty());
            assert!(output.exists());
            fs::remove_file(source).unwrap();
            assert!(job.run_at(date(4, 1)).unwrap().deleted_files.is_empty());
            assert!(output.exists());
        }

        #[test]
        fn corrupt_manifest_fails_closed() {
            let (_tmp, job, source, output) = fixture();
            job.run_at(date(2, 1)).unwrap();
            fs::write(job.destination.join(MANIFEST_NAME), "broken").unwrap();
            assert!(job.run_at(date(4, 1)).is_err());
            assert!(source.exists());
            assert!(output.exists());
        }

        #[test]
        fn refuses_to_delete_a_directory_replacing_a_backup() {
            let (_tmp, job, source, output) = fixture();
            job.run_at(date(2, 1)).unwrap();
            fs::remove_file(&output).unwrap();
            fs::create_dir(&output).unwrap();
            fs::write(output.join("keep.txt"), "keep").unwrap();
            let result = job.run_at(date(4, 1)).unwrap();
            assert_eq!(result.summary.errors, 1);
            assert!(result.deleted_files.is_empty());
            assert!(output.join("keep.txt").exists());
            assert!(source.exists());
        }

        #[test]
        fn different_source_cannot_delete_an_owned_backup() {
            let (_tmp, job, source, output) = fixture();
            job.run_at(date(2, 1)).unwrap();
            let other = job.source.join("260102(1)_Other").join("main.prproj");
            fs::create_dir_all(other.parent().unwrap()).unwrap();
            fs::rename(source, other).unwrap();
            let result = job.run_at(date(4, 1)).unwrap();
            assert_eq!(result.retained_files, vec![output.clone()]);
            assert!(output.exists());
        }
    }

    mod should_backup {
        use super::*;

        #[test]
        fn matching_filename_is_not_a_matching_ancestor_folder() {
            let tmp = tempdir().unwrap();
            let p = tmp.path().join("plain").join("260930(1)_project.prproj");
            touch(&p);
            assert!(!should_backup(&p, &regex(), &[], &[]));
        }

        #[test]
        fn folder_name_exclusion_does_not_match_filename() {
            let tmp = tempdir().unwrap();
            let p = tmp.path().join("260930(1)_project").join("cache.prproj");
            touch(&p);
            assert!(should_backup(&p, &regex(), &[], &["cache.prproj".into()]));
            assert!(should_backup(&p, &regex(), std::slice::from_ref(&p), &[]));
        }

        #[test]
        fn includes_prproj_under_six_digit_folder() {
            let tmp = tempdir().unwrap();
            let p = tmp.path().join("250304(3)_クイズ").join("project.prproj");
            touch(&p);
            assert!(should_backup(&p, &regex(), &[], &[]));
        }

        #[test]
        fn includes_prproj_extension_case_insensitive() {
            let tmp = tempdir().unwrap();
            let p = tmp.path().join("250304(3)_クイズ").join("PROJECT.PRPROJ");
            touch(&p);
            assert!(should_backup(&p, &regex(), &[], &[]));
        }

        #[test]
        fn excludes_prproj_under_plain_folder() {
            let tmp = tempdir().unwrap();
            let p = tmp.path().join("MyProject").join("project.prproj");
            touch(&p);
            assert!(!should_backup(&p, &regex(), &[], &[]));
        }

        #[test]
        fn excludes_non_prproj_under_six_digit_folder() {
            let tmp = tempdir().unwrap();
            let p = tmp.path().join("250304(1)_foo").join("notes.txt");
            touch(&p);
            assert!(!should_backup(&p, &regex(), &[], &[]));
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
            assert!(!should_backup(&p, &regex(), &[], &[]));
        }

        #[test]
        fn excludes_auto_save_case_insensitive() {
            let tmp = tempdir().unwrap();
            let p = tmp
                .path()
                .join("250304(2)_foo")
                .join("adobe premiere pro auto-save")
                .join("project.prproj");
            touch(&p);
            assert!(!should_backup(&p, &regex(), &[], &[]));
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
            assert!(should_backup(&p, &regex(), &[], &[]));
        }

        #[test]
        fn excludes_when_digits_are_not_six() {
            let tmp = tempdir().unwrap();
            let p = tmp.path().join("12345(X)_five").join("project.prproj");
            touch(&p);
            assert!(!should_backup(&p, &regex(), &[], &[]));
        }

        #[test]
        fn requires_opening_paren_after_six_digits() {
            let tmp = tempdir().unwrap();
            let p = tmp.path().join("250304_nosep").join("project.prproj");
            touch(&p);
            assert!(!should_backup(&p, &regex(), &[], &[]));
        }

        #[test]
        fn excludes_when_under_excluded_folder_path() {
            let tmp = tempdir().unwrap();
            let proxy = tmp.path().join("250304(3)_クイズ").join("Proxy");
            let p = proxy.join("sub").join("clip.prproj");
            touch(&p);
            let excluded = vec![proxy];
            assert!(!should_backup(&p, &regex(), &excluded, &[]));
        }

        #[cfg(windows)]
        #[test]
        fn excludes_when_under_excluded_folder_path_with_different_case_on_windows() {
            let tmp = tempdir().unwrap();
            let proxy = tmp.path().join("250304(3)_クイズ").join("Proxy");
            let p = proxy.join("sub").join("clip.prproj");
            touch(&p);
            let excluded = vec![PathBuf::from(proxy.to_string_lossy().to_ascii_lowercase())];
            assert!(!should_backup(&p, &regex(), &excluded, &[]));
        }

        #[test]
        fn excluded_folder_path_does_not_affect_siblings() {
            let tmp = tempdir().unwrap();
            let project = tmp.path().join("250304(3)_クイズ");
            let proxy = project.join("Proxy");
            touch(&proxy.join("p.prproj"));
            let kept = project.join("main.prproj");
            touch(&kept);
            let excluded = vec![proxy];
            assert!(should_backup(&kept, &regex(), &excluded, &[]));
        }

        #[test]
        fn excludes_when_ancestor_name_matches_case_insensitive() {
            let tmp = tempdir().unwrap();
            let p = tmp
                .path()
                .join("250304(3)_クイズ")
                .join("CACHE")
                .join("c.prproj");
            touch(&p);
            let names = vec!["cache".to_string()];
            assert!(!should_backup(&p, &regex(), &[], &names));
        }

        #[test]
        fn name_exclusion_does_not_partial_match() {
            let tmp = tempdir().unwrap();
            let p = tmp
                .path()
                .join("250304(3)_クイズ")
                .join("MyCache_v2")
                .join("c.prproj");
            touch(&p);
            let names = vec!["cache".to_string()];
            assert!(should_backup(&p, &regex(), &[], &names));
        }

        #[test]
        fn empty_exclusions_are_no_op() {
            let tmp = tempdir().unwrap();
            let p = tmp.path().join("250304(3)_クイズ").join("project.prproj");
            touch(&p);
            assert!(should_backup(&p, &regex(), &[], &[]));
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
        fn respects_excluded_folder_paths_in_run() {
            let e = env();
            let project = e.src.join("250304(1)_a");
            let proxy = project.join("Proxy");
            touch(&project.join("keep.prproj"));
            touch(&proxy.join("skip.prproj"));

            let outcome = BackupJob::new(&e.src, &e.dest)
                .with_excluded_folders(vec![proxy])
                .run()
                .unwrap();

            assert_eq!(outcome.summary.copied, 1);
            assert!(e.dest.join("keep_Latest.prproj").is_file());
            assert!(!e.dest.join("skip_Latest.prproj").is_file());
        }

        #[test]
        fn respects_excluded_folder_names_in_run() {
            let e = env();
            touch(&e.src.join("250304(1)_a").join("Cache").join("a.prproj"));
            touch(&e.src.join("250304(1)_a").join("keep.prproj"));

            let outcome = BackupJob::new(&e.src, &e.dest)
                .with_excluded_folder_names(vec!["cache".into()])
                .run()
                .unwrap();

            assert_eq!(outcome.summary.copied, 1);
            assert!(e.dest.join("keep_Latest.prproj").is_file());
        }

        #[test]
        fn skips_destination_subtree_even_when_destination_is_inside_source() {
            let tmp = tempdir().unwrap();
            let src = tmp.path().join("src");
            let project = src.join("250304(1)_a");
            let dest = project.join("backup");
            fs::create_dir_all(&dest).unwrap();
            touch(&project.join("main.prproj"));
            touch(&dest.join("old_Latest.prproj"));

            let outcome = BackupJob::new(&src, &dest).run().unwrap();

            assert_eq!(outcome.summary.copied, 1);
            assert!(dest.join("main_Latest.prproj").is_file());
            assert!(!dest.join("old_Latest_Latest.prproj").exists());
        }

        #[test]
        fn leaves_no_part_file_after_successful_copy() {
            let e = env();
            touch(&e.src.join("250304(1)_a").join("p.prproj"));

            BackupJob::new(&e.src, &e.dest).run().unwrap();

            let leftovers: Vec<_> = fs::read_dir(&e.dest)
                .unwrap()
                .filter_map(|r| r.ok())
                .filter(|e| e.path().to_string_lossy().ends_with(".part"))
                .collect();
            assert!(leftovers.is_empty());
        }
    }
}
