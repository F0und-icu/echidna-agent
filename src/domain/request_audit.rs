//! Versioned semantic request audit facts and exact Blob-backed reconstruction.

mod components;
mod error;
mod manifest;
mod reconstruction;
mod render;

pub use components::{
    RequestAuditComponentIdentity, RequestAuditComponentReference, RequestAuditComponentSource,
    SemanticHash,
};
pub use error::{RequestAuditError, RequestAuditReconstructionError};
pub use manifest::{
    RequestAdapterVersion, RequestAuditAttempt, RequestAuditBinding, RequestAuditCapabilityMapping,
    RequestAuditDelivery, RequestAuditDeliveryMode, RequestAuditDeliveryReason,
    RequestAuditManifest, RequestAuditModelInput, RequestAuditRunContext,
};
pub(crate) use render::PublishedAuditBlob;

/// The first persisted RequestAuditManifest schema.
pub const REQUEST_AUDIT_MANIFEST_SCHEMA_VERSION: u16 = 1;
