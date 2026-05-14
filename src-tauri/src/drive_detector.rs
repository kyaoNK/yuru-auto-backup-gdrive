use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::Serialize;

const DRIVE_FOLDER_NAMES: &[&str] = &["マイドライブ", "My Drive", "共有ドライブ", "Shared drives"];

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DriveCandidate {
    pub path: PathBuf,
    pub label: String,
    pub source: DetectionSource,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DetectionSource {
    Registry,
    DriveLetter,
    Conventional,
}

pub fn detect() -> Vec<DriveCandidate> {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from);

    let mut out = Vec::new();
    #[cfg(windows)]
    {
        out.extend(from_registry());
        out.extend(from_drive_letters());
    }
    if let Some(h) = home.as_ref() {
        out.extend(from_conventional_paths_in(h));
    }
    dedup_existing(out)
}

pub fn from_conventional_paths_in(home: &Path) -> Vec<DriveCandidate> {
    let gd = home.join("Google Drive");
    if gd.is_dir() {
        return vec![DriveCandidate {
            path: gd,
            label: "Google Drive".to_string(),
            source: DetectionSource::Conventional,
        }];
    }
    Vec::new()
}

#[cfg(windows)]
fn from_drive_letters() -> Vec<DriveCandidate> {
    let mut out = Vec::new();
    for letter in b'A'..=b'Z' {
        let root = PathBuf::from(format!("{}:\\", letter as char));
        if !root.is_dir() {
            continue;
        }
        for name in DRIVE_FOLDER_NAMES {
            let p = root.join(name);
            if p.is_dir() {
                out.push(DriveCandidate {
                    path: p,
                    label: (*name).to_string(),
                    source: DetectionSource::DriveLetter,
                });
            }
        }
    }
    out
}

#[cfg(windows)]
fn from_registry() -> Vec<DriveCandidate> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let Ok(drivefs) = hkcu.open_subkey(r"Software\Google\DriveFS") else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for sub_name in drivefs.enum_keys().filter_map(Result::ok) {
        let Ok(sub) = drivefs.open_subkey(&sub_name) else {
            continue;
        };
        for val_result in sub.enum_values() {
            let Ok((val_name, _)) = val_result else {
                continue;
            };
            let Ok(raw) = sub.get_value::<String, _>(&val_name) else {
                continue;
            };
            let path = PathBuf::from(raw);
            if path.is_dir() {
                out.push(DriveCandidate {
                    path,
                    label: val_name,
                    source: DetectionSource::Registry,
                });
            }
        }
    }
    out
}

pub fn dedup_existing(mut items: Vec<DriveCandidate>) -> Vec<DriveCandidate> {
    items.retain(|c| c.path.is_dir());
    let mut seen: HashSet<PathBuf> = HashSet::new();
    items.retain(|c| seen.insert(c.path.clone()));
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn conventional_returns_empty_when_no_google_drive_folder() {
        let tmp = tempdir().unwrap();
        let result = from_conventional_paths_in(tmp.path());
        assert!(result.is_empty());
    }

    #[test]
    fn conventional_returns_candidate_when_google_drive_folder_exists() {
        let tmp = tempdir().unwrap();
        let gd = tmp.path().join("Google Drive");
        fs::create_dir_all(&gd).unwrap();

        let result = from_conventional_paths_in(tmp.path());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].path, gd);
        assert_eq!(result[0].source, DetectionSource::Conventional);
    }

    #[test]
    fn dedup_filters_out_nonexistent_paths() {
        let tmp = tempdir().unwrap();
        let existing = tmp.path().join("keep");
        fs::create_dir_all(&existing).unwrap();
        let missing = tmp.path().join("gone");

        let input = vec![
            DriveCandidate {
                path: existing.clone(),
                label: "a".into(),
                source: DetectionSource::Conventional,
            },
            DriveCandidate {
                path: missing,
                label: "b".into(),
                source: DetectionSource::Conventional,
            },
        ];
        let out = dedup_existing(input);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].path, existing);
    }

    #[test]
    fn dedup_removes_duplicate_paths_preserving_first() {
        let tmp = tempdir().unwrap();
        let p = tmp.path().join("dup");
        fs::create_dir_all(&p).unwrap();

        let input = vec![
            DriveCandidate {
                path: p.clone(),
                label: "first".into(),
                source: DetectionSource::Registry,
            },
            DriveCandidate {
                path: p.clone(),
                label: "second".into(),
                source: DetectionSource::Conventional,
            },
        ];
        let out = dedup_existing(input);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].label, "first");
        assert_eq!(out[0].source, DetectionSource::Registry);
    }

    #[test]
    fn detect_does_not_panic_when_no_drive_is_installed() {
        let _ = detect();
    }
}
