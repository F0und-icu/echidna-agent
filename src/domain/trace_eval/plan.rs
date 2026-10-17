//! Pure provider-neutral semantic steps compiled from an existing Trace.

use thiserror::Error;

use crate::domain::events::EventId;
use crate::domain::model::{
    MODEL_INPUT_IR_SCHEMA_VERSION, MODEL_INPUT_IR_V2_SCHEMA_VERSION,
    MODEL_INPUT_IR_V3_SCHEMA_VERSION, ModelEvent, ModelInputIR, ModelInputIrSchemaVersion,
};
use crate::domain::tools::{ExecutionReceipt, ToolCall, ToolResult, ToolSpec};

use super::EvalSource;

/// Provenance retained for one replayed Tool outcome.
///
/// The complete historical Receipt remains immutable source evidence. The
/// replay Receipt describes only this derived execution and deliberately does
/// not copy historical Artifact, cache, environment or file-mutation claims.
/// Bare execution identifiers are interpreted under [`Self::source`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceReplayToolUse {
    source: EvalSource,
    source_receipt_event_id: EventId,
    source_receipt: ExecutionReceipt,
    replay_receipt: ExecutionReceipt,
}

impl TraceReplayToolUse {
    pub(crate) fn new(
        source: EvalSource,
        source_receipt_event_id: EventId,
        source_receipt: ExecutionReceipt,
        replay_receipt: ExecutionReceipt,
    ) -> Self {
        Self {
            source,
            source_receipt_event_id,
            source_receipt,
            replay_receipt,
        }
    }

    pub fn source(&self) -> &EvalSource {
        &self.source
    }

    pub fn source_receipt_event_id(&self) -> &EventId {
        &self.source_receipt_event_id
    }

    pub fn source_receipt(&self) -> &ExecutionReceipt {
        &self.source_receipt
    }

    pub fn replay_receipt(&self) -> &ExecutionReceipt {
        &self.replay_receipt
    }

    pub fn source_execution_id(&self) -> &crate::domain::tools::ExecutionId {
        &self.source_receipt.execution_id
    }

    pub fn replay_execution_id(&self) -> &crate::domain::tools::ExecutionId {
        &self.replay_receipt.execution_id
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceReplayModelStep {
    expected_input: ModelInputIR,
    events: Vec<ModelEvent>,
}

impl TraceReplayModelStep {
    pub fn new(
        mut expected_input: ModelInputIR,
        events: Vec<ModelEvent>,
    ) -> Result<Self, TraceReplayPlanError> {
        if !events.iter().any(|event| {
            matches!(
                event,
                ModelEvent::TextDelta { .. } | ModelEvent::ToolCall { .. }
            )
        }) {
            return Err(TraceReplayPlanError::ModelStepHasNoSemanticOutput);
        }
        if matches!(
            expected_input.schema_version.get(),
            MODEL_INPUT_IR_V2_SCHEMA_VERSION | MODEL_INPUT_IR_V3_SCHEMA_VERSION
        ) {
            // ModelInputIR v3 adds plaintext reasoning; v4 adds opaque
            // provider continuation. Historical v2/v3 requests cannot contain
            // the new variant, so this is a lossless Replay-only upcast.
            expected_input.schema_version =
                ModelInputIrSchemaVersion::new(MODEL_INPUT_IR_SCHEMA_VERSION);
        }
        Ok(Self {
            expected_input,
            events,
        })
    }

    pub(crate) fn expected_input(&self) -> &ModelInputIR {
        &self.expected_input
    }

    pub(crate) fn into_events(self) -> Vec<ModelEvent> {
        self.events
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceReplayToolStep {
    source_receipt_event_id: EventId,
    source_receipt: ExecutionReceipt,
    expected_call: ToolCall,
    source_result: ToolResult,
}

impl TraceReplayToolStep {
    pub fn new(
        source_receipt_event_id: EventId,
        source_receipt: ExecutionReceipt,
        expected_call: ToolCall,
        source_result: ToolResult,
    ) -> Result<Self, TraceReplayPlanError> {
        if source_receipt_event_id.as_str().trim().is_empty()
            || expected_call.name.as_str().trim().is_empty()
        {
            return Err(TraceReplayPlanError::EmptyToolStepField);
        }
        if source_result.call_id != expected_call.call_id
            || source_result.tool_name != expected_call.name
        {
            return Err(TraceReplayPlanError::SourceToolPairMismatch);
        }
        if source_result.status != source_receipt.status {
            return Err(TraceReplayPlanError::SourceReceiptStatusMismatch);
        }
        source_receipt
            .validate_output_captures(&source_result)
            .map_err(|_| TraceReplayPlanError::SourceOutputCaptureMismatch)?;
        Ok(Self {
            source_receipt_event_id,
            source_receipt,
            expected_call,
            source_result,
        })
    }

    pub(crate) fn expected_call(&self) -> &ToolCall {
        &self.expected_call
    }

    pub(crate) fn into_parts(self) -> (EventId, ExecutionReceipt, ToolCall, ToolResult) {
        (
            self.source_receipt_event_id,
            self.source_receipt,
            self.expected_call,
            self.source_result,
        )
    }
}

/// A derived plan compiled by App from existing facts and consumed only by
/// the concrete Replay composition root.
pub struct TraceReplayPlan {
    source: EvalSource,
    model_steps: Vec<TraceReplayModelStep>,
    tool_specs: Vec<ToolSpec>,
    tool_steps: Vec<TraceReplayToolStep>,
}

impl TraceReplayPlan {
    pub(crate) fn new(
        source: EvalSource,
        model_steps: Vec<TraceReplayModelStep>,
        tool_specs: Vec<ToolSpec>,
        tool_steps: Vec<TraceReplayToolStep>,
    ) -> Self {
        Self {
            source,
            model_steps,
            tool_specs,
            tool_steps,
        }
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        EvalSource,
        Vec<TraceReplayModelStep>,
        Vec<ToolSpec>,
        Vec<TraceReplayToolStep>,
    ) {
        (
            self.source,
            self.model_steps,
            self.tool_specs,
            self.tool_steps,
        )
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum TraceReplayPlanError {
    #[error("Replay Tool output captures disagree with its source Receipt")]
    SourceOutputCaptureMismatch,
    #[error("a Replay semantic sample has no text or Tool Call output")]
    ModelStepHasNoSemanticOutput,
    #[error("a Replay Tool step has an empty required field")]
    EmptyToolStepField,
    #[error("a Replay Tool result does not pair with its source Tool Call")]
    SourceToolPairMismatch,
    #[error("a Replay Tool Receipt status does not match its source Tool result")]
    SourceReceiptStatusMismatch,
}
