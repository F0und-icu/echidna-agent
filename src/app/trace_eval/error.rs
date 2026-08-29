//! Typed failures for M4-S2 evaluation composition.

use thiserror::Error;

use crate::app::agent_loop::AgentLoopError;
use crate::domain::artifacts::{ArtifactId, BlobId};
use crate::domain::events::EventId;
use crate::domain::request_audit::RequestAuditReconstructionError;
use crate::domain::trace_eval::TraceReplayPlanError;
use crate::domain::trace_eval::{ToolAccuracyError, TraceFixtureError};

#[derive(Debug, Error)]
pub enum TraceEvalServiceError<BlobError> {
    #[error(transparent)]
    Trace(TraceFixtureError),
    #[error(transparent)]
    TracePlan(#[from] TraceReplayPlanError),
    #[error(transparent)]
    ToolAccuracy(#[from] ToolAccuracyError),
    #[error("failed to reconstruct the historical RequestAudit input: {0:?}")]
    RequestAuditReconstruction(RequestAuditReconstructionError<BlobError>),
    #[error(transparent)]
    Context(#[from] crate::domain::context::ContextPlannerError),
    #[error(
        "Trace `{audit_event_id}` uses incremental delivery, which M4-S2 Replay does not support"
    )]
    IncrementalRequestUnsupported { audit_event_id: EventId },
    #[error(
        "Context Recompile RequestAudit Event `{audit_event_id}` is not referenced by the Trace"
    )]
    ContextRequestAuditNotReferenced { audit_event_id: EventId },
    #[error(
        "Trace-required Artifact `{artifact_id}` Blob `{blob_id}` could not be read from the Artifact Store"
    )]
    RequiredArtifactBlobUnreadable {
        artifact_id: ArtifactId,
        blob_id: BlobId,
    },
    #[error(
        "Trace-required Artifact `{artifact_id}` Blob `{blob_id}` exceeds supported in-memory size"
    )]
    RequiredArtifactBlobSizeOverflow {
        artifact_id: ArtifactId,
        blob_id: BlobId,
    },
    #[error(
        "Trace-required Artifact `{artifact_id}` Blob `{blob_id}` size mismatch: expected {expected_size_bytes}, received {received_size_bytes}"
    )]
    RequiredArtifactBlobSizeMismatch {
        artifact_id: ArtifactId,
        blob_id: BlobId,
        expected_size_bytes: u64,
        received_size_bytes: u64,
    },
    #[error(
        "Trace-required Artifact `{artifact_id}` Blob hash mismatch: expected `{expected_blob_id}`, received `{received_blob_id}`"
    )]
    RequiredArtifactBlobHashMismatch {
        artifact_id: ArtifactId,
        expected_blob_id: BlobId,
        received_blob_id: BlobId,
    },
    #[error("M4-S2 full-request Replay requires stable Tool schemas across the Trace")]
    ToolSchemasChanged,
    #[error("Trace Tool Call `{call_id}` has no paired historical Tool result")]
    ToolResultMissing { call_id: String },
    #[error(
        "Trace Tool Call `{call_id}` pairs to History Event `{event_id}` outside the declared range"
    )]
    ToolResultOutsideRange { call_id: String, event_id: EventId },
    #[error("Trace Tool Call `{call_id}` has no referenced historical ExecutionReceipt")]
    ToolReceiptForCallMissing { call_id: String },
    #[error(
        "Trace Tool Receipt references are not in source Tool Call order: Call `{call_event_id}` resolves to `{expected_receipt_event_id}`, not declared `{declared_receipt_event_id}`"
    )]
    ToolReceiptReferenceOrderInvalid {
        call_event_id: EventId,
        expected_receipt_event_id: EventId,
        declared_receipt_event_id: EventId,
    },
    #[error(
        "Trace Tool facts are out of order: Call `{call_event_id}`, Receipt `{receipt_event_id}`, Result `{result_event_id}`"
    )]
    ToolFactOrderInvalid {
        call_event_id: EventId,
        receipt_event_id: EventId,
        result_event_id: EventId,
    },
    #[error("Trace has referenced Tool Receipts which are not consumed by a Provider Tool Call")]
    UnconsumedToolReceiptReferences,
}

#[derive(Debug, Error)]
pub enum TraceReplayRunError {
    #[error(transparent)]
    Agent(AgentLoopError),
    #[error(
        "Trace Replay completed with {remaining_model_steps} model and {remaining_tool_steps} Tool steps unconsumed"
    )]
    TraceNotFullyConsumed {
        remaining_model_steps: usize,
        remaining_tool_steps: usize,
    },
    #[error("Shadow Model Eval completed with {remaining_tool_steps} Replay Tool steps unconsumed")]
    ReplayToolsNotFullyConsumed { remaining_tool_steps: usize },
}
