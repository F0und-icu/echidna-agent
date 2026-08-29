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

/// The current additive RequestAuditManifest schema. Schema v1 remains
/// readable and explicitly lacks verified renderer/Tool-plan binding.
pub const REQUEST_AUDIT_MANIFEST_SCHEMA_VERSION: u16 = 2;
