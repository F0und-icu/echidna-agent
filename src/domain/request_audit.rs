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
pub(crate) use reconstruction::VerifiedRequestAuditInput;
pub(crate) use render::PublishedAuditBlob;

/// Schema v4 records full logical components and their provenance map. Schema
/// v1 lacks renderer/Tool-plan binding, v2 lacks the delivery map, and v3 lacks
/// independently reconstructible logical components. All remain readable.
pub const REQUEST_AUDIT_MANIFEST_SCHEMA_VERSION: u16 = 4;
pub(crate) const TASK_REQUEST_AUDIT_MANIFEST_SCHEMA_VERSION: u16 = 5;
