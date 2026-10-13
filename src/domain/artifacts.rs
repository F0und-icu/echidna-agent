//! Provider-neutral Artifact facts and their pure Lineage projection.
//!
//! Blob identity is content-addressed. Artifact identity records one durable
//! research use of that content and deliberately remains distinct even when
//! several records refer to the same Blob.

mod event;
mod records;
mod reducer;

pub use event::{ARTIFACT_SCHEMA_VERSION, ArtifactEvent, ArtifactEventError};
pub use records::{ArtifactId, ArtifactRecord, ArtifactScope, BlobId};
pub use reducer::ArtifactError;
pub(crate) use reducer::ArtifactState;
