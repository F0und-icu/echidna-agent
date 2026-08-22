//! Confined file-system and audit facts for the local mutation Tools.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use crate::adapters::file_diff::replacement_diff;
use crate::adapters::project_event_store::replace_durable_file;
use crate::domain::tools::FileMutationAudit;

pub(super) struct Preimage {
    pub(super) bytes: Option<Vec<u8>>,
    pub(super) hash: Option<String>,
    pub(super) size: Option<u64>,
}

/// File writes have a narrower path contract than FullAccess reads: all paths
/// stay under the canonical project root and no component may be a symlink.
pub(super) fn confined_path(project_root: &Path, value: &str) -> Result<PathBuf, String> {
    let relative = Path::new(value);
    if relative.as_os_str().is_empty() || relative.is_absolute() {
        return Err("path must be a non-empty project-relative path".to_owned());
    }
    if relative.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err("path must not contain `..` or an absolute prefix".to_owned());
    }
    let root = fs::canonicalize(project_root)
        .map_err(|error| format!("canonicalize project root: {error}"))?;
    if !root.is_dir() {
        return Err("project root is not a directory".to_owned());
    }
    let target = root.join(relative);
    let mut current = root.clone();
    let mut component_count = 0_usize;
    for component in relative.components() {
        let Component::Normal(segment) = component else {
            continue;
        };
        component_count += 1;
        current.push(segment);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!(
                    "path component `{}` must not be a symlink",
                    current.display()
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                if current != target {
                    return Err(format!(
                        "parent directory `{}` does not exist",
                        current.display()
                    ));
                }
            }
            Err(error) => return Err(format!("inspect path `{}`: {error}", current.display())),
        }
    }
    if component_count == 0 {
        return Err("path must name a project-relative file".to_owned());
    }
    let parent = current
        .parent()
        .ok_or_else(|| "target path has no parent".to_owned())?;
    match fs::symlink_metadata(parent) {
        Ok(metadata) if metadata.file_type().is_dir() => Ok(target),
        Ok(_) => Err(format!("parent `{}` must be a directory", parent.display())),
        Err(error) => Err(format!("inspect parent `{}`: {error}", parent.display())),
    }
}

pub(super) fn read_preimage(path: &Path) -> Result<Preimage, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => {
            let bytes = fs::read(path).map_err(|error| error.to_string())?;
            Ok(Preimage {
                hash: Some(content_hash(&bytes)),
                size: Some(bytes.len() as u64),
                bytes: Some(bytes),
            })
        }
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err("target must not be a symlink".to_owned())
        }
        Ok(_) => Err("target must be a regular file".to_owned()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Preimage {
            bytes: None,
            hash: None,
            size: None,
        }),
        Err(error) => Err(error.to_string()),
    }
}

pub(super) fn durable_replace(path: &Path, bytes: &[u8]) -> Result<(), String> {
    replace_durable_file(path, bytes).map_err(|error| error.to_string())
}

pub(super) fn mutation_audit(
    path: &str,
    expected_preimage_hash: Option<String>,
    before: &Preimage,
    after: &Preimage,
) -> Result<FileMutationAudit, String> {
    let after_bytes = after
        .bytes
        .as_deref()
        .ok_or_else(|| "afterimage is unexpectedly absent".to_owned())?;
    let old = before
        .bytes
        .as_deref()
        .map(std::str::from_utf8)
        .transpose()
        .map_err(|_| {
            "preimage is non-UTF-8; this text-only write adapter cannot produce an exact Diff"
                .to_owned()
        })?;
    let new = std::str::from_utf8(after_bytes).map_err(|_| {
        "afterimage is non-UTF-8; this text-only write adapter cannot produce an exact Diff"
            .to_owned()
    })?;
    Ok(FileMutationAudit {
        path: path.to_owned(),
        expected_preimage_hash,
        before_hash: before.hash.clone(),
        before_size: before.size,
        after_hash: after.hash.clone(),
        after_size: after.size,
        actual_diff: replacement_diff(old, Some(new), path, path),
    })
}

pub(super) fn require_auditable_preimage(preimage: &Preimage) -> Result<(), String> {
    if let Some(bytes) = &preimage.bytes {
        std::str::from_utf8(bytes).map_err(|_| {
            "preimage is non-UTF-8; this text-only write adapter cannot produce an exact Diff"
                .to_owned()
        })?;
    }
    Ok(())
}

fn content_hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}
