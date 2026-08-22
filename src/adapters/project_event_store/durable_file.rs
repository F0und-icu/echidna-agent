//! Durable no-overwrite file commits for project sentinels and exports.
//!
//! A caller supplies complete bytes. This module makes them durable in a
//! same-directory temporary file before publishing a new final name through a
//! hard link, which never replaces an existing file.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::ProjectEventStoreError;

static NEXT_DURABLE_TEMPORARY_FILE: AtomicU64 = AtomicU64::new(0);

/// Publishes fully persisted bytes at a previously absent path without
/// overwriting an existing file.
///
/// The temporary file is deliberately created beside the final file so the
/// hard-link publication and directory sync stay on one filesystem. Failed
/// preparation never creates the final name. Once the hard link succeeds, the
/// final name is committed and temporary cleanup is best effort.
pub(crate) fn commit_new_durable_file(
    destination: &Path,
    bytes: &[u8],
) -> Result<(), ProjectEventStoreError> {
    commit_new_durable_file_with(destination, |temporary_file| {
        temporary_file.write_all(bytes)
    })
}

/// Atomically replaces an existing private project file after fully syncing a
/// same-directory temporary file. This is intentionally separate from the
/// no-overwrite publication path used by sentinels and content-addressed
/// Blobs: only callers that already own the project Writer lock may use it.
pub(crate) fn replace_durable_file(
    destination: &Path,
    bytes: &[u8],
) -> Result<(), ProjectEventStoreError> {
    let parent = destination_parent(destination)?;
    let (mut temporary_file, temporary_path) = create_durable_temporary_file(parent)?;
    let persist_result = temporary_file
        .write_all(bytes)
        .and_then(|()| temporary_file.sync_all());
    if let Err(source) = persist_result {
        drop(temporary_file);
        remove_durable_temporary_file_best_effort(&temporary_path);
        return Err(ProjectEventStoreError::Io {
            operation: "persist durable replacement temporary file",
            path: temporary_path,
            source,
        });
    }
    drop(temporary_file);
    if let Err(source) = fs::rename(&temporary_path, destination) {
        remove_durable_temporary_file_best_effort(&temporary_path);
        return Err(ProjectEventStoreError::Io {
            operation: "atomically replace durable file",
            path: destination.to_path_buf(),
            source,
        });
    }
    sync_directory(parent)
}

fn commit_new_durable_file_with(
    destination: &Path,
    persist: impl FnOnce(&mut File) -> io::Result<()>,
) -> Result<(), ProjectEventStoreError> {
    let parent = destination_parent(destination)?;
    let (mut temporary_file, temporary_path) = create_durable_temporary_file(parent)?;
    let persist_result = persist(&mut temporary_file).and_then(|()| temporary_file.sync_all());
    if let Err(source) = persist_result {
        drop(temporary_file);
        remove_durable_temporary_file_best_effort(&temporary_path);
        return Err(ProjectEventStoreError::Io {
            operation: "persist durable temporary file",
            path: temporary_path,
            source,
        });
    }
    drop(temporary_file);

    let result = commit_durable_temporary_file(&temporary_path, destination);
    if result.is_err() {
        remove_durable_temporary_file_best_effort(&temporary_path);
    }
    result
}

/// Creates one same-directory temporary file for a caller that must derive a
/// final content-addressed name while streaming bytes. The caller must sync
/// its completed content before calling `commit_durable_temporary_file`.
pub(crate) fn create_durable_temporary_file(
    parent: &Path,
) -> Result<(File, PathBuf), ProjectEventStoreError> {
    let process_id = std::process::id();
    let starting_nonce = NEXT_DURABLE_TEMPORARY_FILE.fetch_add(1, Ordering::Relaxed);

    for attempt in 0..128_u64 {
        let nonce = starting_nonce.wrapping_add(attempt);
        let path = parent.join(format!(".cyber-agent-durable-{process_id}-{nonce}.tmp"));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((file, path)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(source) => {
                return Err(ProjectEventStoreError::Io {
                    operation: "create durable temporary file",
                    path,
                    source,
                });
            }
        }
    }

    Err(ProjectEventStoreError::Io {
        operation: "allocate durable temporary file",
        path: parent.to_path_buf(),
        source: io::Error::new(
            io::ErrorKind::AlreadyExists,
            "durable temporary file name attempts exhausted",
        ),
    })
}

/// Publishes a fully synced temporary file under a previously absent final
/// name. The temporary and destination must share one directory so hard-link
/// publication cannot replace a pre-existing final file.
pub(crate) fn commit_durable_temporary_file(
    temporary_path: &Path,
    destination: &Path,
) -> Result<(), ProjectEventStoreError> {
    let parent = destination_parent(destination)?;
    if temporary_path.parent() != Some(parent) {
        return Err(ProjectEventStoreError::Io {
            operation: "publish durable temporary file from another directory",
            path: temporary_path.to_path_buf(),
            source: io::Error::new(
                io::ErrorKind::InvalidInput,
                "temporary and destination must share a directory",
            ),
        });
    }
    if let Err(source) = fs::hard_link(temporary_path, destination) {
        return Err(ProjectEventStoreError::Io {
            operation: "commit new durable file without overwrite",
            path: destination.to_path_buf(),
            source,
        });
    }
    remove_durable_temporary_file_best_effort(temporary_path);
    sync_directory(parent)
}

/// Best-effort removal for a staged file that did not obtain a final name.
pub(crate) fn remove_durable_temporary_file_best_effort(path: &Path) {
    let _ = fs::remove_file(path);
}

/// Synchronizes a directory after a persisted filesystem-name change.
#[cfg(unix)]
pub(crate) fn sync_directory(path: &Path) -> Result<(), ProjectEventStoreError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| ProjectEventStoreError::Io {
            operation: "sync filesystem directory",
            path: path.to_path_buf(),
            source,
        })
}

/// Directory syncing is unavailable on non-Unix targets in this M2-S1 slice.
#[cfg(not(unix))]
pub(crate) fn sync_directory(_path: &Path) -> Result<(), ProjectEventStoreError> {
    Ok(())
}

fn destination_parent(destination: &Path) -> Result<&Path, ProjectEventStoreError> {
    destination
        .parent()
        .ok_or_else(|| ProjectEventStoreError::NoFileName {
            path: destination.to_path_buf(),
        })
}
