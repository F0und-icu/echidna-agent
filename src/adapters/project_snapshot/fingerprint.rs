//! Project metadata fingerprinting and on-demand content verification.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::UNIX_EPOCH;

use crate::adapters::project_event_store::Project;
use crate::domain::snapshot::{
    FileKind, FileManifest, FileManifestEntry, ProjectFingerprint, ProjectFingerprintKind,
};

use super::LocalProjectSnapshotError;

pub(super) struct FirstLevel {
    pub(super) fingerprint: ProjectFingerprint,
    /// Non-Git projects already require the metadata walk to construct their
    /// cheap fingerprint. Git projects defer that walk until a cache miss or
    /// an explicit content-Hash refresh.
    pub(super) entries: Option<Vec<FileManifestEntry>>,
}

pub(super) fn first_level(project: &Project) -> Result<FirstLevel, LocalProjectSnapshotError> {
    let root = project.root();
    match run_git(root, ["rev-parse", "--is-inside-work-tree"]) {
        Ok(output) if output.trim() == "true" => {
            let mut control_entries = Vec::new();
            collect_control_entries(root, &mut control_entries)?;
            control_entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
            let control_hash = facts_metadata_hash(&control_entries);
            let metadata = (|| -> Result<(String, Vec<u8>, String, String), String> {
                let head = run_git(root, ["rev-parse", "HEAD"])?;
                let status = run_git_bytes(
                    root,
                    [
                        "status",
                        "--porcelain=v2",
                        "-z",
                        "--untracked-files=all",
                        "--",
                        ".",
                        ":(exclude).cyber-agent",
                        ":(exclude).cyber-agent/**",
                        ":(exclude)target",
                        ":(exclude)target/**",
                    ],
                )?;
                let index = run_git(root, ["rev-parse", "--git-path", "index"])?;
                let index_path = PathBuf::from(index.trim());
                let index_path = if index_path.is_absolute() {
                    index_path
                } else {
                    root.join(index_path)
                };
                let index_metadata =
                    metadata_token(&index_path).map_err(|error| error.to_string())?;
                let dirty_metadata = git_dirty_metadata(root, &status)?;
                Ok((head, status, index_metadata, dirty_metadata))
            })();
            match metadata {
                Ok((head, status, index_metadata, dirty_metadata)) => Ok(FirstLevel {
                    fingerprint: ProjectFingerprint {
                        kind: ProjectFingerprintKind::Git,
                        digest: hash_parts([
                            root.as_os_str().as_encoded_bytes(),
                            head.trim().as_bytes(),
                            &status,
                            index_metadata.as_bytes(),
                            dirty_metadata.as_bytes(),
                            control_hash.as_bytes(),
                        ]),
                        cacheable: true,
                    },
                    entries: None,
                }),
                Err(_) => Ok(FirstLevel {
                    fingerprint: ProjectFingerprint {
                        kind: ProjectFingerprintKind::GitUnavailable,
                        digest: hash_parts([
                            root.as_os_str().as_encoded_bytes(),
                            control_hash.as_bytes(),
                        ]),
                        cacheable: false,
                    },
                    entries: None,
                }),
            }
        }
        Ok(_) | Err(_) if root.join(".git").exists() => {
            let mut control_entries = Vec::new();
            collect_control_entries(root, &mut control_entries)?;
            control_entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
            let control_hash = facts_metadata_hash(&control_entries);
            Ok(FirstLevel {
                fingerprint: ProjectFingerprint {
                    kind: ProjectFingerprintKind::GitUnavailable,
                    digest: hash_parts([
                        root.as_os_str().as_encoded_bytes(),
                        control_hash.as_bytes(),
                    ]),
                    cacheable: false,
                },
                entries: None,
            })
        }
        Ok(_) | Err(_) => {
            let entries = collect_entries(root)?;
            let facts_hash = facts_metadata_hash(&entries);
            Ok(FirstLevel {
                fingerprint: ProjectFingerprint {
                    kind: ProjectFingerprintKind::Filesystem,
                    digest: hash_parts([
                        root.as_os_str().as_encoded_bytes(),
                        facts_hash.as_bytes(),
                    ]),
                    cacheable: true,
                },
                entries: Some(entries),
            })
        }
    }
}

pub(super) fn collect_entries(
    root: &Path,
) -> Result<Vec<FileManifestEntry>, LocalProjectSnapshotError> {
    let mut entries = Vec::new();
    collect_entries_at(root, root, true, &mut entries)?;
    collect_control_entries(root, &mut entries)?;
    entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(entries)
}

fn collect_control_entries(
    root: &Path,
    entries: &mut Vec<FileManifestEntry>,
) -> Result<(), LocalProjectSnapshotError> {
    for path in [
        root.join(".cyber-agent/project.toml"),
        root.join(".cyber-agent/research.jsonl"),
    ] {
        append_required_control_file(root, &path, entries)?;
    }
    let memory = root.join(".cyber-agent/memory");
    match fs::symlink_metadata(&memory) {
        Ok(metadata) => {
            append_entry(root, &memory, entries)?;
            if metadata.file_type().is_dir() {
                collect_entries_at(root, &memory, false, entries)?;
            }
        }
        Err(source) if source.kind() == io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(LocalProjectSnapshotError::Inspect {
                path: memory,
                source,
            });
        }
    }
    Ok(())
}

fn append_required_control_file(
    root: &Path,
    path: &Path,
    entries: &mut Vec<FileManifestEntry>,
) -> Result<(), LocalProjectSnapshotError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(LocalProjectSnapshotError::ControlFileSymlink {
                path: path.to_path_buf(),
            });
        }
        Ok(metadata) if !metadata.file_type().is_file() => {
            return Err(LocalProjectSnapshotError::ControlFileNotRegular {
                path: path.to_path_buf(),
            });
        }
        Ok(metadata) => metadata,
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            return Err(LocalProjectSnapshotError::ControlFileMissing {
                path: path.to_path_buf(),
            });
        }
        Err(source) => {
            return Err(LocalProjectSnapshotError::Inspect {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    entries.push(FileManifestEntry {
        relative_path: path
            .strip_prefix(root)
            .expect("control file stays below root")
            .to_string_lossy()
            .replace('\\', "/"),
        kind: FileKind::File,
        size_bytes: metadata.len(),
        modified_nanos: modified_nanos(&metadata),
        blake3: None,
    });
    Ok(())
}

fn collect_entries_at(
    root: &Path,
    directory: &Path,
    apply_exclusions: bool,
    entries: &mut Vec<FileManifestEntry>,
) -> Result<(), LocalProjectSnapshotError> {
    let mut children = fs::read_dir(directory)
        .map_err(|source| LocalProjectSnapshotError::Traverse {
            path: directory.to_path_buf(),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| LocalProjectSnapshotError::Traverse {
            path: directory.to_path_buf(),
            source,
        })?;
    children.sort_by_key(|entry| entry.file_name());
    for child in children {
        let path = child.path();
        let relative = path
            .strip_prefix(root)
            .expect("walked child stays below root")
            .to_string_lossy()
            .replace('\\', "/");
        if apply_exclusions && is_excluded(&relative) {
            continue;
        }
        append_entry(root, &path, entries)?;
        let metadata =
            fs::symlink_metadata(&path).map_err(|source| LocalProjectSnapshotError::Inspect {
                path: path.clone(),
                source,
            })?;
        if metadata.file_type().is_dir() {
            collect_entries_at(root, &path, apply_exclusions, entries)?;
        }
    }
    Ok(())
}

fn append_entry(
    root: &Path,
    path: &Path,
    entries: &mut Vec<FileManifestEntry>,
) -> Result<(), LocalProjectSnapshotError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(LocalProjectSnapshotError::Inspect {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    let kind = if metadata.file_type().is_symlink() {
        FileKind::Symlink
    } else if metadata.file_type().is_dir() {
        FileKind::Directory
    } else if metadata.file_type().is_file() {
        FileKind::File
    } else {
        FileKind::Other
    };
    entries.push(FileManifestEntry {
        relative_path: path
            .strip_prefix(root)
            .expect("walked child stays below root")
            .to_string_lossy()
            .replace('\\', "/"),
        kind,
        size_bytes: metadata.len(),
        modified_nanos: modified_nanos(&metadata),
        blake3: None,
    });
    Ok(())
}

pub(super) fn apply_second_level(
    root: &Path,
    mut entries: Vec<FileManifestEntry>,
    prior: Option<&FileManifest>,
    hash_paths: &[String],
) -> Result<(Vec<FileManifestEntry>, Vec<String>), LocalProjectSnapshotError> {
    let prior_entries = prior
        .map(|manifest| {
            manifest
                .entries
                .iter()
                .map(|entry| (entry.relative_path.as_str(), entry))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let mut requested = requested_hashes(root, &entries, hash_paths)?;
    let mut hashed_paths = Vec::new();
    for entry in &mut entries {
        let prior = prior_entries.get(entry.relative_path.as_str()).copied();
        let metadata_changed = prior.is_none_or(|prior| {
            prior.kind != entry.kind
                || prior.size_bytes != entry.size_bytes
                || prior.modified_nanos != entry.modified_nanos
        });
        if !metadata_changed {
            entry.blake3 = prior.and_then(|prior| prior.blake3.clone());
        }
        if requested.remove(&entry.relative_path) && entry.blake3.is_none() {
            let path = root.join(&entry.relative_path);
            entry.blake3 = Some(hash_regular_file(&path)?);
            hashed_paths.push(entry.relative_path.clone());
        }
    }
    if let Some(path) = requested.into_iter().next() {
        return Err(LocalProjectSnapshotError::HashNotRegular {
            path: root.join(path),
        });
    }
    Ok((entries, hashed_paths))
}

pub(super) fn pending_hashes(
    root: &Path,
    entries: &[FileManifestEntry],
    values: &[String],
) -> Result<Vec<String>, LocalProjectSnapshotError> {
    let requested = requested_hashes(root, entries, values)?;
    let entries = entries
        .iter()
        .map(|entry| (entry.relative_path.as_str(), entry))
        .collect::<BTreeMap<_, _>>();
    Ok(requested
        .into_iter()
        .filter(|path| {
            entries
                .get(path.as_str())
                .is_some_and(|entry| entry.blake3.is_none())
        })
        .collect())
}

fn requested_hashes(
    root: &Path,
    entries: &[FileManifestEntry],
    values: &[String],
) -> Result<BTreeSet<String>, LocalProjectSnapshotError> {
    let requested = normalize_hash_paths(values)?;
    let entries = entries
        .iter()
        .map(|entry| (entry.relative_path.as_str(), entry))
        .collect::<BTreeMap<_, _>>();
    for path in &requested {
        if !entries
            .get(path.as_str())
            .is_some_and(|entry| entry.kind == FileKind::File)
        {
            return Err(LocalProjectSnapshotError::HashNotRegular {
                path: root.join(path),
            });
        }
    }
    Ok(requested)
}

fn normalize_hash_paths(values: &[String]) -> Result<BTreeSet<String>, LocalProjectSnapshotError> {
    values
        .iter()
        .map(|value| {
            let normalized_separators = value.replace('\\', "/");
            let path = Path::new(&normalized_separators);
            if path.is_absolute()
                || value.is_empty()
                || path.components().any(|component| {
                    matches!(
                        component,
                        std::path::Component::ParentDir
                            | std::path::Component::RootDir
                            | std::path::Component::Prefix(_)
                    )
                })
            {
                Err(LocalProjectSnapshotError::InvalidHashPath {
                    path: value.clone(),
                })
            } else {
                let normalized = path
                    .components()
                    .filter_map(|component| match component {
                        std::path::Component::CurDir => None,
                        std::path::Component::Normal(segment) => Some(segment),
                        _ => unreachable!("invalid components were rejected"),
                    })
                    .collect::<PathBuf>();
                if normalized.as_os_str().is_empty() {
                    Err(LocalProjectSnapshotError::InvalidHashPath {
                        path: value.clone(),
                    })
                } else {
                    Ok(normalized.to_string_lossy().replace('\\', "/"))
                }
            }
        })
        .collect()
}

fn hash_regular_file(path: &Path) -> Result<String, LocalProjectSnapshotError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|source| LocalProjectSnapshotError::Inspect {
            path: path.to_path_buf(),
            source,
        })?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        return Err(LocalProjectSnapshotError::HashNotRegular {
            path: path.to_path_buf(),
        });
    }
    let mut file = File::open(path).map_err(|source| LocalProjectSnapshotError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|source| LocalProjectSnapshotError::Read {
                path: path.to_path_buf(),
                source,
            })?;
        if read == 0 {
            return Ok(hasher.finalize().to_hex().to_string());
        }
        hasher.update(&buffer[..read]);
    }
}

fn facts_metadata_hash(entries: &[FileManifestEntry]) -> String {
    let mut hasher = blake3::Hasher::new();
    for entry in entries {
        hasher.update(entry.relative_path.as_bytes());
        hasher.update(&[0]);
        hasher.update(
            format!(
                "{:?}:{}:{}",
                entry.kind, entry.size_bytes, entry.modified_nanos
            )
            .as_bytes(),
        );
        hasher.update(&[0]);
    }
    hasher.finalize().to_hex().to_string()
}

fn metadata_token(path: &Path) -> Result<String, LocalProjectSnapshotError> {
    match fs::symlink_metadata(path) {
        // `git status` may refresh the index mtime without changing its
        // semantics. Size stays cheap and stable; HEAD plus porcelain-v2 bytes
        // own the index/worktree meaning used by this fingerprint.
        Ok(metadata) => Ok(metadata.len().to_string()),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok("absent".to_owned()),
        Err(source) => Err(LocalProjectSnapshotError::Inspect {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn git_dirty_metadata(root: &Path, status: &[u8]) -> Result<String, String> {
    let status = std::str::from_utf8(status)
        .map_err(|_| "Git status contains a non-UTF-8 path".to_owned())?;
    let mut records = status.split_terminator('\0');
    let mut paths = BTreeSet::new();
    while let Some(record) = records.next() {
        let path = match record.as_bytes().first().copied() {
            Some(b'1') => porcelain_path(record, 9)?,
            Some(b'2') => {
                let path = porcelain_path(record, 10)?;
                let _original_path = records
                    .next()
                    .ok_or_else(|| "Git rename status is missing its original path".to_owned())?;
                path
            }
            Some(b'u') => porcelain_path(record, 11)?,
            Some(b'?') => record
                .strip_prefix("? ")
                .filter(|path| !path.is_empty())
                .ok_or_else(|| "Git untracked status is malformed".to_owned())?,
            Some(b'!') => continue,
            Some(_) | None => return Err("Git porcelain-v2 status is malformed".to_owned()),
        };
        paths.insert(path);
    }

    let mut hasher = blake3::Hasher::new();
    for relative in paths {
        let path = root.join(relative);
        let token = worktree_metadata_token(&path).map_err(|error| error.to_string())?;
        hasher.update(&(relative.len() as u64).to_le_bytes());
        hasher.update(relative.as_bytes());
        hasher.update(&(token.len() as u64).to_le_bytes());
        hasher.update(token.as_bytes());
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn porcelain_path(record: &str, fields: usize) -> Result<&str, String> {
    record
        .splitn(fields, ' ')
        .nth(fields - 1)
        .filter(|path| !path.is_empty())
        .ok_or_else(|| "Git porcelain-v2 status is malformed".to_owned())
}

fn worktree_metadata_token(path: &Path) -> Result<String, LocalProjectSnapshotError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            let kind = if metadata.file_type().is_symlink() {
                "symlink"
            } else if metadata.file_type().is_dir() {
                "directory"
            } else if metadata.file_type().is_file() {
                "file"
            } else {
                "other"
            };
            Ok(format!(
                "{kind}:{}:{}",
                metadata.len(),
                modified_nanos(&metadata)
            ))
        }
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok("absent".to_owned()),
        Err(source) => Err(LocalProjectSnapshotError::Inspect {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn modified_nanos(metadata: &fs::Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .and_then(|duration| u64::try_from(duration.as_nanos()).ok())
        .unwrap_or(u64::MAX)
}

fn is_excluded(relative: &str) -> bool {
    relative == ".git"
        || relative.starts_with(".git/")
        || relative == "target"
        || relative.starts_with("target/")
        || relative == ".cyber-agent"
        || relative.starts_with(".cyber-agent/")
}

fn run_git<const N: usize>(root: &Path, arguments: [&str; N]) -> Result<String, String> {
    String::from_utf8(run_git_bytes(root, arguments)?).map_err(|error| error.to_string())
}

fn run_git_bytes<const N: usize>(root: &Path, arguments: [&str; N]) -> Result<Vec<u8>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(arguments)
        .output()
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}

fn hash_parts<const N: usize>(parts: [&[u8]; N]) -> String {
    let mut hasher = blake3::Hasher::new();
    for part in parts {
        hasher.update(&(part.len() as u64).to_le_bytes());
        hasher.update(part);
    }
    hasher.finalize().to_hex().to_string()
}
