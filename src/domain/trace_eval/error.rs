//! Typed failures for strict Trace fixture validation.

use thiserror::Error;

use crate::domain::artifacts::ArtifactId;
use crate::domain::events::{EventId, StreamId};
use crate::domain::research::TargetSnapshotId;

use super::manifest::TraceExpectedInvariant;

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum TraceFixtureError {
    #[error("TraceFixtureManifest schema version {schema_version} is unsupported")]
    UnsupportedSchemaVersion { schema_version: u16 },
    #[error("TraceFixtureManifest schema and Prompt renderer identity are inconsistent")]
    PromptRendererIdentityMismatch,
    #[error("TraceFixtureManifest field `{field}` must not be empty")]
    EmptyField { field: &'static str },
    #[error("TraceFixtureManifest source stream hash is not lowercase BLAKE3 hex")]
    InvalidSourceStreamHash,
    #[error("TraceFixtureManifest event range cannot contain sequence zero")]
    EventRangeSequenceZero,
    #[error("TraceFixtureManifest event range is reversed: {start}..={end}")]
    EventRangeReversed { start: u64, end: u64 },
    #[error("TraceFixtureManifest event range length overflows this platform")]
    EventRangeLengthOverflow,
    #[error("TraceFixtureManifest requires at least one Provider event reference")]
    ProviderEventReferencesRequired,
    #[error("TraceFixtureManifest requires the complete M4-S2 invariant set")]
    ExpectedInvariantsRequired,
    #[error("TraceFixtureManifest is missing required invariant {invariant:?}")]
    RequiredExpectedInvariantMissing { invariant: TraceExpectedInvariant },
    #[error(
        "TraceFixtureManifest Provider request `{request_audit_event_id}` requires History references"
    )]
    ProviderHistoryReferencesRequired { request_audit_event_id: EventId },
    #[error("TraceFixtureManifest `{field}` contains an empty or duplicate identifier `{value}`")]
    DuplicateOrEmptyIdentifier { field: &'static str, value: String },
    #[error("TraceFixtureManifest references Event `{event_id}` more than once")]
    DuplicateEventReference { event_id: EventId },
    #[error("TraceFixtureManifest references the same expected invariant more than once")]
    DuplicateExpectedInvariant,
    #[error(
        "TraceFixtureManifest source Event `{event_id}` is not present in the declared range for {kind}"
    )]
    EventReferenceOutsideRange {
        event_id: EventId,
        kind: &'static str,
    },
    #[error(
        "TraceFixtureManifest {kind} Event `{later_event_id}` is not after `{earlier_event_id}`"
    )]
    EventReferenceOrderInvalid {
        earlier_event_id: EventId,
        later_event_id: EventId,
        kind: &'static str,
    },
    #[error("TraceFixtureManifest source stream differs at Event `{event_id}`")]
    SourceStreamMismatch { event_id: EventId },
    #[error(
        "TraceFixtureManifest source Event sequence is not contiguous: expected {expected}, received {received}"
    )]
    SourceStreamSequenceMismatch { expected: u64, received: u64 },
    #[error("TraceFixtureManifest source Session stream is empty")]
    SourceStreamEmpty,
    #[error("TraceFixtureManifest source Session facts cannot be reduced: {message}")]
    SourceSessionInvalid { message: String },
    #[error(
        "TraceFixtureManifest source Session identity mismatch: expected `{expected}`, received {received:?}"
    )]
    SessionIdentityMismatch {
        expected: StreamId,
        received: Option<StreamId>,
    },
    #[error("the supplied Session projection does not match the declared source Event stream")]
    SessionProjectionMismatch,
    #[error(
        "TraceFixtureManifest source stream hash mismatch: expected `{expected}`, received `{received}`"
    )]
    SourceStreamHashMismatch { expected: String, received: String },
    #[error(
        "TraceFixtureManifest Event range {start}..={end} is incomplete; received {received} Events"
    )]
    EventRangeMissing {
        start: u64,
        end: u64,
        received: usize,
    },
    #[error("TraceFixtureManifest Provider RequestAudit Event `{event_id}` is missing")]
    ProviderRequestAuditMissing { event_id: EventId },
    #[error("TraceFixtureManifest Provider History Event `{event_id}` is missing")]
    ProviderHistoryMissing { event_id: EventId },
    #[error("TraceFixtureManifest Provider History Event `{event_id}` is not model output")]
    ProviderHistoryNotModelOutput { event_id: EventId },
    #[error("TraceFixtureManifest Tool Receipt Event `{event_id}` is missing")]
    ToolReceiptMissing { event_id: EventId },
    #[error("TraceFixtureManifest TargetSnapshot `{target_snapshot_id}` is missing")]
    TargetSnapshotMissing {
        target_snapshot_id: TargetSnapshotId,
    },
    #[error("TraceFixtureManifest Artifact `{artifact_id}` is missing")]
    ArtifactMissing { artifact_id: ArtifactId },
    #[error("failed to canonicalize TraceFixtureManifest source stream: {message}")]
    SourceStreamCanonicalEncoding { message: String },
}
