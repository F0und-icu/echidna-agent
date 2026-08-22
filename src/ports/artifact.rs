//! Provider-neutral durable Blob publication boundary.

use std::path::Path;

use crate::domain::artifacts::BlobId;

/// The typed result of one content-addressed Blob publication attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredBlob {
    pub blob_id: BlobId,
    pub size_bytes: u64,
    pub already_present: bool,
}

/// Publishes one local regular file as durable content-addressed Blob bytes.
///
/// Artifact facts and Event ordering remain with the caller. This Port owns
/// only byte identity and no-overwrite Blob publication.
pub trait ArtifactStore: Send + Sync {
    type Error;

    fn publish_file(&self, source: &Path) -> Result<StoredBlob, Self::Error>;

    /// Publishes already-owned bytes without forcing a caller to create a
    /// temporary source file merely to obtain an audited Blob reference.
    fn publish_bytes(&self, bytes: &[u8]) -> Result<StoredBlob, Self::Error>;
}
