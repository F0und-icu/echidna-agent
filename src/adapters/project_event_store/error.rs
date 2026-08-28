//! One typed error boundary for the project Event Store Adapter.

use std::io;
use std::path::PathBuf;

use thiserror::Error;

use crate::domain::events::{EventError, EventSchema, StreamId};
use crate::domain::project::ProjectId;

/// Errors from project discovery, JSONL persistence, recovery, and export.
#[derive(Debug, Error)]
pub enum ProjectEventStoreError {
    #[error("project is already initialized at `{project_file}`")]
    AlreadyInitialized { project_file: PathBuf },
    #[error("incomplete project initialization exists at `{control_directory}`")]
    IncompleteInitialization { control_directory: PathBuf },
    #[error("no Cyber Agent project was found from `{start}`")]
    ProjectNotFound { start: PathBuf },
    #[error(
        "project configuration `{project_file}` identifies `{configured_root}`, not discovered root `{discovered_root}`"
    )]
    ProjectRootMismatch {
        project_file: PathBuf,
        configured_root: PathBuf,
        discovered_root: PathBuf,
    },
    #[error(
        "project configuration `{project_file}` identifies project `{configured}`, not canonical-root identity `{expected}`"
    )]
    ProjectIdentityMismatch {
        project_file: PathBuf,
        configured: ProjectId,
        expected: ProjectId,
    },
    #[error("failed to decode project configuration `{path}`: {source}")]
    ProjectTomlDecode {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("failed to encode project configuration: {0}")]
    ProjectTomlEncode(#[source] toml::ser::Error),
    #[error("project writer is already held for `{project_root}`: {source}")]
    WriterLocked {
        project_root: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("Chat runtime is already active for `{project_root}`: {source}")]
    ChatRuntimeLocked {
        project_root: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("initialized project Research Event stream `{path}` is missing")]
    MissingResearchStream { path: PathBuf },
    #[error("initialized project Research Event stream `{path}` is not a regular file")]
    InvalidResearchStream { path: PathBuf },
    #[error("Session stream ID `{session_id}` is not a safe directory name")]
    InvalidSessionId { session_id: StreamId },
    #[error("Event stream mismatch: expected `{expected}`, received `{received}`")]
    StreamIdMismatch {
        expected: StreamId,
        received: StreamId,
    },
    #[error("non-tail JSONL corruption in `{path}` at exact byte offset {offset}: {reason}")]
    NonTailCorruption {
        path: PathBuf,
        offset: u64,
        reason: String,
    },
    #[error("invalid Event record in `{path}` at exact byte offset {offset}: {reason}")]
    InvalidEventRecord {
        path: PathBuf,
        offset: u64,
        reason: String,
    },
    #[error(
        "final JSONL record in `{path}` at byte offset {offset} requires explicit recovery Event metadata"
    )]
    RecoveryMetadataRequired { path: PathBuf, offset: u64 },
    #[error("final JSONL record in `{path}` at byte offset {offset} has not been recovered")]
    UnrecoveredTail { path: PathBuf, offset: u64 },
    #[error("recovery artifact hash collision at `{path}`")]
    RecoveryArtifactCollision { path: PathBuf },
    #[error("export destination already exists at `{path}`")]
    ExportTargetExists { path: PathBuf },
    #[error(
        "incomplete migration output: destination `{destination}` and manifest `{manifest_path}` must either both exist or both be absent"
    )]
    IncompleteMigration {
        destination: PathBuf,
        manifest_path: PathBuf,
    },
    #[error("export destination would overwrite retained source `{path}`")]
    ExportWouldOverwriteSource { path: PathBuf },
    #[error("path `{path}` has no usable file name")]
    NoFileName { path: PathBuf },
    #[error(
        "migration target {requested:?} is unavailable; registered target is {registered_current:?}"
    )]
    UnsupportedMigrationTarget {
        requested: EventSchema,
        registered_current: EventSchema,
    },
    #[error("migration tool version must not be empty")]
    EmptyMigrationToolVersion,
    #[error("failed to encode Event record: {0}")]
    EventEncode(#[source] serde_json::Error),
    #[error("failed to encode recovery payload: {0}")]
    RecoveryPayloadEncode(#[source] serde_json::Error),
    #[error("failed to encode migration manifest: {0}")]
    MigrationManifestEncode(#[source] serde_json::Error),
    #[error(transparent)]
    Event(#[from] EventError),
    #[error("{operation} `{path}`: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}
