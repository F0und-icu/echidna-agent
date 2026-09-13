use std::io;
use std::path::PathBuf;

use thiserror::Error;

use crate::adapters::project_event_store::ProjectEventStoreError;
use crate::domain::artifacts::BlobId;

#[derive(Debug, Error)]
pub(crate) enum LocalArtifactStoreError {
    #[error(
        "output Blob range unavailable: offset={offset}, stored size={size}; maximum capture is 1 MiB"
    )]
    OutputRange { offset: u64, size: u64 },
    #[error("inspect Artifact source `{path}`: {source}")]
    SourceInspect {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("Artifact source `{path}` must not be a symlink")]
    SourceSymlink { path: PathBuf },
    #[error("Artifact source `{path}` must be a regular file")]
    SourceNotRegular { path: PathBuf },
    #[error("read Artifact source `{path}`: {source}")]
    SourceRead {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("write staged Artifact Blob `{path}`: {source}")]
    StagedBlobWrite {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("sync staged Artifact Blob `{path}`: {source}")]
    StagedBlobSync {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("Artifact Blob directory component `{path}` must not be a symlink")]
    BlobDirectorySymlink { path: PathBuf },
    #[error("Artifact Blob directory component `{path}` must be a directory")]
    BlobDirectoryNotDirectory { path: PathBuf },
    #[error("create Artifact Blob directory `{path}`: {source}")]
    CreateBlobDirectory {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("persist Artifact Blob directory `{path}`: {error}")]
    PersistBlobDirectory {
        path: PathBuf,
        #[source]
        error: Box<ProjectEventStoreError>,
    },
    #[error("inspect Artifact Blob directory `{path}`: {source}")]
    InspectBlobDirectory {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("existing Artifact Blob `{path}` must not be a symlink")]
    ExistingBlobSymlink { path: PathBuf },
    #[error("existing Artifact Blob `{path}` must be a regular file")]
    ExistingBlobNotRegular { path: PathBuf },
    #[error("inspect existing Artifact Blob `{path}`: {source}")]
    ExistingBlobInspect {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("read existing Artifact Blob `{path}`: {source}")]
    ExistingBlobRead {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("existing Artifact Blob `{path}` conflicts with SHA-256 `{blob_id}")]
    ExistingBlobConflict { path: PathBuf, blob_id: BlobId },
    #[error("stage Artifact Blob `{path}`: {error}")]
    Stage {
        path: PathBuf,
        #[source]
        error: Box<ProjectEventStoreError>,
    },
    #[error("publish Artifact Blob `{path}`: {error}")]
    Publish {
        path: PathBuf,
        #[source]
        error: Box<ProjectEventStoreError>,
    },
}
