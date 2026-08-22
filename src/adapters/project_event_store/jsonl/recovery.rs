use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::super::ProjectEventStoreError;
use super::super::durable_file::{commit_new_durable_file, sync_directory};
use super::super::project::{Project, RECOVERY_DIRECTORY, create_directory, read_bytes};

static NEXT_RECOVERY_TEMPORARY_FILE: AtomicU64 = AtomicU64::new(0);

/// The deterministic location and reference for one raw corrupt-tail Artifact.
pub(super) struct RecoveryArtifact {
    path: PathBuf,
    reference: String,
}

impl RecoveryArtifact {
    /// Derives the Artifact location without changing project facts.
    pub(super) fn for_tail(project: &Project, raw_tail: &[u8]) -> Self {
        let digest = blake3::hash(raw_tail).to_hex().to_string();
        let filename = format!("{digest}.bin");
        Self {
            path: project
                .control_directory()
                .join(RECOVERY_DIRECTORY)
                .join(&filename),
            reference: format!("{RECOVERY_DIRECTORY}/{filename}"),
        }
    }

    pub(super) fn reference(&self) -> &str {
        &self.reference
    }
}

/// Persists exact corrupt-tail bytes before the source stream can change.
pub(super) fn persist_recovery_artifact(
    artifact: &RecoveryArtifact,
    raw_tail: &[u8],
) -> Result<(), ProjectEventStoreError> {
    let parent = artifact
        .path
        .parent()
        .expect("recovery artifact has a parent");
    create_directory(parent)?;

    if artifact.path.exists() {
        return verify_existing_recovery_artifact(artifact, raw_tail);
    }

    match commit_new_durable_file(&artifact.path, raw_tail) {
        Ok(()) => Ok(()),
        Err(ProjectEventStoreError::Io { source, .. })
            if source.kind() == io::ErrorKind::AlreadyExists =>
        {
            verify_existing_recovery_artifact(artifact, raw_tail)
        }
        Err(error) => Err(error),
    }
}

fn verify_existing_recovery_artifact(
    artifact: &RecoveryArtifact,
    raw_tail: &[u8],
) -> Result<(), ProjectEventStoreError> {
    if read_bytes(&artifact.path, "verify existing recovery artifact")? == raw_tail {
        let parent = artifact
            .path
            .parent()
            .expect("recovery artifact has a parent");
        sync_directory(parent)
    } else {
        Err(ProjectEventStoreError::RecoveryArtifactCollision {
            path: artifact.path.clone(),
        })
    }
}

/// Atomically replaces a corrupt stream after its raw tail is durable.
///
/// Before rename the original corrupt source remains intact. After rename the
/// stream contains the exact valid prefix followed by `StreamRecovered`.
pub(super) fn replace_stream_atomically(
    path: &Path,
    valid_prefix: &[u8],
    recovery_record: &[u8],
) -> Result<(), ProjectEventStoreError> {
    let parent = path.parent().ok_or_else(|| ProjectEventStoreError::Io {
        operation: "resolve Event stream recovery directory",
        path: path.to_path_buf(),
        source: io::Error::new(
            io::ErrorKind::NotFound,
            "Event stream has no parent directory",
        ),
    })?;
    let (mut temporary_file, temporary_path) = create_recovery_temporary_file(parent, path)?;
    let persist_result = temporary_file
        .write_all(valid_prefix)
        .and_then(|()| temporary_file.write_all(recovery_record))
        .and_then(|()| temporary_file.sync_all());
    if let Err(source) = persist_result {
        drop(temporary_file);
        let _ = fs::remove_file(&temporary_path);
        return Err(ProjectEventStoreError::Io {
            operation: "persist replacement Event stream before atomic recovery rename",
            path: temporary_path.clone(),
            source,
        });
    }
    drop(temporary_file);

    if let Err(source) = fs::rename(&temporary_path, path) {
        let _ = fs::remove_file(&temporary_path);
        return Err(ProjectEventStoreError::Io {
            operation: "atomically replace recovered Event stream",
            path: path.to_path_buf(),
            source,
        });
    }
    sync_directory(parent)
}

fn create_recovery_temporary_file(
    parent: &Path,
    source: &Path,
) -> Result<(File, PathBuf), ProjectEventStoreError> {
    let filename = source
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("events.jsonl");
    let process_id = std::process::id();
    let starting_nonce = NEXT_RECOVERY_TEMPORARY_FILE.fetch_add(1, Ordering::Relaxed);

    for attempt in 0..128_u64 {
        let nonce = starting_nonce.wrapping_add(attempt);
        let path = parent.join(format!(".{filename}.recovery-{process_id}-{nonce}.tmp"));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((file, path)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(source) => {
                return Err(ProjectEventStoreError::Io {
                    operation: "create replacement Event stream temporary file",
                    path,
                    source,
                });
            }
        }
    }

    Err(ProjectEventStoreError::Io {
        operation: "allocate replacement Event stream temporary file",
        path: parent.to_path_buf(),
        source: io::Error::new(
            io::ErrorKind::AlreadyExists,
            "recovery temporary file name attempts exhausted",
        ),
    })
}
