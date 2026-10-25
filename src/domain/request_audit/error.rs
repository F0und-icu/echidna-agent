use thiserror::Error;

use super::RequestAuditComponentIdentity;
use super::manifest::RequestAuditBinding;

/// Reconstruction has exactly two boundaries: a malformed audit or
/// service-binding failure, or the ArtifactStore read failure. It
/// intentionally has no Provider or credential variant.
#[derive(Debug)]
pub enum RequestAuditReconstructionError<E> {
    Audit(RequestAuditError),
    Blob(E),
}

/// Strict RequestAudit construction, persisted-shape and reconstruction
/// failures.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum RequestAuditError {
    #[error("RequestAudit binding field `{field}` must not be empty")]
    BindingFieldRequired { field: &'static str },
    #[error("RequestAudit Adapter version must not be empty")]
    AdapterVersionRequired,
    #[error("RequestAudit Manifest schema version {schema_version} is unsupported")]
    UnsupportedManifestSchema { schema_version: u16 },
    #[error("RequestAudit {input_field} IR schema version {schema_version} is unsupported")]
    UnsupportedModelInputIrSchema {
        input_field: &'static str,
        schema_version: u16,
    },
    #[error("RequestAudit delivery mode and reason are inconsistent")]
    DeliveryReasonMismatch,
    #[error("a full RequestAudit delivery must have identical logical and delivered inputs")]
    FullDeliveryInputMismatch,
    #[error("RequestAudit ToolSpecs do not match the Sampling Step ResolvedToolPlan")]
    SamplingToolPlanMismatch,
    #[error("RequestAudit full logical input does not match the Sampling Step ContextPlan")]
    SamplingContextPlanMismatch,
    #[error("RequestAudit schema and renderer/Tool-plan identity are inconsistent")]
    SamplingIdentityMismatch,
    #[error("RequestAudit ProviderSession does not match its scoped request identity")]
    ProviderSessionMismatch,
    #[error("RequestAudit ContextDeliveryMap is invalid: {message}")]
    ContextDeliveryMap { message: String },
    #[error(
        "RequestAudit service binding `{received:?}` does not match the expected binding `{expected:?}`"
    )]
    ServiceBindingMismatch {
        expected: RequestAuditBinding,
        received: RequestAuditBinding,
    },
    #[error("RequestAudit Event ID `{event_id}` has already been applied to this Session")]
    EventIdAlreadyApplied { event_id: String },
    #[error("failed to encode canonical RequestAudit content: {message}")]
    CanonicalEncoding { message: String },
    #[error("RequestAudit semantic hash `{field}` is not lowercase BLAKE3 hex")]
    InvalidSemanticHash { field: &'static str },
    #[error("RequestAudit component index overflowed the schema bound")]
    ComponentIndexOverflow,
    #[error("RequestAudit component position overflowed the schema bound")]
    ComponentPositionOverflow,
    #[error("RequestAudit published Blob count mismatch: expected {expected}, received {received}")]
    PublishedBlobCountMismatch { expected: usize, received: usize },
    #[error("RequestAudit component layout requires instructions and final requirements")]
    ComponentLayoutIncomplete,
    #[error("RequestAudit component position mismatch: expected {expected}, received {received}")]
    ComponentPositionMismatch { expected: u32, received: u32 },
    #[error("RequestAudit component identity `{identity:?}` is duplicated")]
    DuplicateComponentIdentity {
        identity: RequestAuditComponentIdentity,
    },
    #[error("RequestAudit component at position {position} has an invalid Blob reference")]
    InvalidBlobReference { position: u32 },
    #[error("RequestAudit component order at position {position} must be {expected}")]
    ComponentOrderMismatch {
        position: u32,
        expected: &'static str,
    },
    #[error("RequestAudit component at position {position} is not valid UTF-8 or JSON: {message}")]
    ComponentDecode { position: u32, message: String },
    #[error("RequestAudit component byte length at position {position} overflows the schema bound")]
    ComponentSizeOverflow { position: u32 },
    #[error(
        "RequestAudit component byte length mismatch at position {position}: expected {expected}, received {received}"
    )]
    ComponentSizeMismatch {
        position: u32,
        expected: u64,
        received: u64,
    },
    #[error("RequestAudit component semantic hash mismatch at position {position}")]
    ComponentHashMismatch { position: u32 },
    #[error("RequestAudit reconstructed delivered input does not match its recorded semantic hash")]
    DeliveredInputHashMismatch,
}
