//! Strict, versioned Session facts and deterministic reduction.
//!
//! Session facts reuse the shared `cyber-agent.event` envelope. This module
//! owns only the Session payload vocabulary and lifecycle transitions; the
//! generic envelope remains the owner of schema/upcasting, event identity and
//! sequence validation.

use std::collections::{BTreeMap, BTreeSet};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

mod argument_correction;
mod checkpoint_binding;
mod model_delivery;
mod operator_boundary;
mod task_context;

use super::agent_definition::AgentDefinitionBinding;
use super::context::{ProviderSessionId, ProviderSessionRef, ProviderSessionRole};
use super::domain_profile::{DomainProfile, DomainProfileSelection};
use super::events::{
    ApplyOutcome, CurrentEventUpcaster, EventEnvelope, EventError, EventId, EventReducer,
    EventSequence, EventType, EventUpcaster, OccurredAt, State, StreamId,
};
use super::history::{ConversationHistory, HistoryError, normalize_with_recorded_results};
use super::model::ContentItem;
use super::model::{
    MAX_TOOL_ARGUMENT_CORRECTIONS_PER_TURN, ModelUsage, ModelUsageError, ToolArgumentDiagnostic,
};
use super::request_audit::{RequestAuditError, RequestAuditManifest};
use super::research_control::{ResearchPhase, ResearchPhaseSelection};
use super::tools::SubcommandCheckpoint;
use super::tools::{
    ExecutionReceipt, ExecutionReceiptEventPayload, ExecutionReceiptPayloadError, ToolCallId,
    ToolResult,
};
use super::vision::{
    VisionAnalysisFailure, VisionFactError, VisionObservation, VisionObservationId,
    VisionRouteRecord,
};

const SESSION_STREAM_PREFIX: &str = "session:";
const SESSION_STARTED_EVENT_TYPE: &str = "session.started";
const SESSION_STARTED_WITH_AGENT_EVENT_TYPE: &str = "session.started_with_agent";
const AGENT_DEFINITION_APPLIED_EVENT_TYPE: &str = "session.agent_definition_applied";
const SESSION_RESUMED_EVENT_TYPE: &str = "session.resumed";
const SESSION_ENDED_EVENT_TYPE: &str = "session.ended";
const RECEIPT_RECORDED_EVENT_TYPE: &str = "tool.execution_receipt_recorded";
const EXECUTION_OUTCOME_RECORDED_EVENT_TYPE: &str = "tool.execution_outcome_recorded";
const SUBCOMMAND_CHECKPOINT_EVENT_TYPE: &str = "tool.subcommand_checkpointed";
const CACHE_HIT_EVENT_TYPE: &str = "tool.result_cache_hit";
const STREAM_RECOVERED_EVENT_TYPE: &str = "StreamRecovered";
pub(crate) const HISTORY_ITEM_RECORDED_EVENT_TYPE: &str = "session.history_item_recorded";
const MODEL_SAMPLE_REJECTED_EVENT_TYPE: &str = "model.sample_rejected";
const MODEL_USAGE_RECORDED_EVENT_TYPE: &str = "model.usage_recorded";
pub(super) const COMPACTION_CREATED_EVENT_TYPE: &str = "session.compaction_created";
pub(super) const REQUEST_AUDIT_RECORDED_EVENT_TYPE: &str = "session.request_audit_recorded";
const VISION_ROUTE_DECIDED_EVENT_TYPE: &str = "vision.route_decided";
const VISION_OBSERVATION_RECORDED_EVENT_TYPE: &str = "vision.observation_recorded";
const VISION_ANALYSIS_FAILED_EVENT_TYPE: &str = "vision.analysis_failed";
const RESEARCH_PHASE_SELECTED_EVENT_TYPE: &str = "session.research_phase_selected";
const DOMAIN_PROFILE_SELECTED_EVENT_TYPE: &str = "session.domain_profile_selected";
pub(super) const CURRENT_SESSION_PAYLOAD_SCHEMA_VERSION: u16 = 1;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContextCapturedPayload {
    schema_version: u16,
    blob_id: crate::domain::artifacts::BlobId,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskSelectionUsagePayload {
    schema_version: u16,
    capture_event_id: EventId,
    request_audit_event_id: EventId,
    provider_session: ProviderSessionRef,
    usage: ModelUsage,
}

mod compaction;
mod improvement;
pub use super::task_context::{TaskInputTime, TimeZoneObservation};
pub use improvement::ImprovementActivity;
pub(crate) use improvement::{ImprovementFact, ImprovementStage, scoped_call_id};
pub use model_delivery::ModelDeliveryCommitted;
pub(crate) use operator_boundary::{HistorySourceSemantics, ProtectedOperatorContext};
pub use operator_boundary::{OperatorInputRef, OperatorTurnBoundary};
pub(crate) use task_context::TaskNavigationAccess;
pub(crate) use task_context::TaskNavigationIndex;
pub(crate) use task_context::TaskProgressSource;
pub(crate) use task_context::TaskResumeHistoryItem;
pub(crate) use task_context::{TASK_CANDIDATE_POLICY_VERSION, TASK_REQUEST_PROOF_VERSION};
pub(crate) use task_context::{TaskCardSource, TaskNavigationChange, TaskNavigationCursor};
pub(crate) use task_context::{TaskCompileCapture, TaskContextCapture, TaskMemoryDocument};
pub use task_context::{TaskContextSelected, TaskSelectionSample};

pub(crate) use compaction::VerifiedCompactionGraph;
pub(crate) use compaction::{CapturedFragment, capture_fragments, restore_fragments};
pub(crate) use compaction::{
    CompactionContextCapture, CompactionSourceEntry, CompactionSourceNode,
    CompactionSourceNodeInput, CompactionSourcePlan, CompactionSourceRange,
};
pub use compaction::{
    CompactionModelSample, CompactionValidationContext, EvidenceSpine, NarrativeSummary,
    ResearchStreamIdentity, SessionCompaction, SessionSummary, SessionSummaryDraft,
};
pub(crate) use compaction::{SessionSourceIdentity, TaskCompactionValidationFacts};
pub(crate) use compaction::{compaction_instructions, node_template_hash};

/// A read-only projection of one existing Receipt fact. It deliberately
/// retains the original Event identity instead of introducing another Receipt
/// schema for replay consumers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SessionExecutionReceiptRecord {
    pub(crate) event_id: EventId,
    pub(crate) sequence: EventSequence,
    pub(crate) receipt: ExecutionReceipt,
    pub(crate) result: Option<ToolResult>,
}

/// The distinct lifecycle state reconstructed from a Session stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionLifecycle {
    Unstarted,
    Active,
    Ended,
}

/// A strictly known Session fact. Tool Receipt variants deliberately reuse
/// the Tool domain's `ExecutionReceipt` rather than defining a Session copy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionEvent {
    TaskContextCaptured {
        blob_id: crate::domain::artifacts::BlobId,
    },
    TaskSelectionSampleRecorded {
        sample: TaskSelectionSample,
    },
    TaskSelectionUsageRecorded {
        capture_event_id: EventId,
        request_audit_event_id: EventId,
        provider_session: ProviderSessionRef,
        usage: ModelUsage,
    },
    TaskContextSelected {
        selection: TaskContextSelected,
    },
    CompactionContextCaptured {
        blob_id: crate::domain::artifacts::BlobId,
    },
    ModelDeliveryCommitted {
        delivery: ModelDeliveryCommitted,
    },
    OperatorTurnBoundaryRecorded {
        boundary: OperatorTurnBoundary,
    },
    CompactionSampleRecorded {
        sample: CompactionModelSample,
    },
    ModelUsageRecorded {
        request_audit_event_id: EventId,
        provider_session: ProviderSessionRef,
        usage: ModelUsage,
    },
    ExecutionOutcomeRecorded {
        receipt: ExecutionReceipt,
        result: ToolResult,
    },
    ImprovementActivityRecorded {
        activity: ImprovementActivity,
    },
    StartedWithAgent {
        primary_provider_session: ProviderSessionRef,
        binding: AgentDefinitionBinding,
    },
    AgentDefinitionApplied {
        primary_provider_session: ProviderSessionRef,
        binding: AgentDefinitionBinding,
    },
    AgentDefinitionAppliedAtCheckpoint {
        primary_provider_session: ProviderSessionRef,
        binding: AgentDefinitionBinding,
        attempt_id: String,
        parent_digest: String,
        compaction_revision: u64,
        compaction_fingerprint: String,
    },
    ModelSampleRejected {
        request_audit_event_id: EventId,
        diagnostic: ToolArgumentDiagnostic,
        corrections_used: u8,
    },
    ConsecutiveModelSampleRejected {
        request_audit_event_id: EventId,
        diagnostic: ToolArgumentDiagnostic,
        corrections_used: u8,
        reset_anchor: Option<EventId>,
    },
    ConsecutiveModelSampleRejectedV3 {
        request_audit_event_id: EventId,
        diagnostic: ToolArgumentDiagnostic,
        corrections_used: u8,
        reset_anchor: Option<EventId>,
        sampling_step_id: String,
    },
    ModelArgumentCorrectionReset {
        accepted_request_audit_event_id: EventId,
        prior_rejection_audit_event_id: EventId,
        accepted_history_event_refs: Vec<EventId>,
    },
    SubcommandCheckpointed {
        checkpoint: SubcommandCheckpoint,
    },
    Started {
        primary_provider_session: ProviderSessionRef,
    },
    Resumed {
        primary_provider_session: ProviderSessionRef,
    },
    Ended,
    ExecutionReceiptRecorded {
        receipt: ExecutionReceipt,
    },
    ToolResultCacheHit {
        receipt: ExecutionReceipt,
    },
    StreamRecovered {
        recovery_artifact: String,
        truncated_from_offset: u64,
        resumed_after_sequence: Option<u64>,
    },
    HistoryItemRecorded {
        item: ContentItem,
    },
    CompactionCreated {
        compaction: SessionCompaction,
    },
    RequestAuditRecorded {
        manifest: RequestAuditManifest,
    },
    VisionRouteDecided {
        route: VisionRouteRecord,
    },
    VisionObservationRecorded {
        observation: VisionObservation,
    },
    VisionAnalysisFailed {
        failure: VisionAnalysisFailure,
    },
    ResearchPhaseSelected {
        selection: ResearchPhaseSelection,
    },
    DomainProfileSelected {
        selection: DomainProfileSelection,
    },
}

impl SessionEvent {
    /// Builds a versioned Session envelope using the shared current Event
    /// schema. Sequence allocation remains the Event Store's responsibility.
    pub fn into_envelope(
        self,
        event_id: EventId,
        stream_id: StreamId,
        sequence: EventSequence,
        occurred_at: OccurredAt,
    ) -> Result<EventEnvelope<Value>, SessionError> {
        let event_type = self.event_type();
        let payload = self.encode_payload(&event_type)?;
        Ok(EventEnvelope::current(
            event_id,
            stream_id,
            sequence,
            occurred_at,
            event_type,
            payload,
        ))
    }

    fn event_type(&self) -> EventType {
        match self {
            Self::TaskContextCaptured { .. } => EventType::new("session.task_context_captured"),
            Self::TaskSelectionSampleRecorded { .. } => {
                EventType::new("session.task_selection_sample_recorded")
            }
            Self::TaskSelectionUsageRecorded { .. } => {
                EventType::new("session.task_selection_usage_recorded")
            }
            Self::TaskContextSelected { .. } => EventType::new("session.task_context_selected"),
            Self::CompactionContextCaptured { .. } => {
                EventType::new("session.compaction_context_captured")
            }
            Self::ModelDeliveryCommitted { .. } => {
                EventType::new("session.model_delivery_committed")
            }
            Self::OperatorTurnBoundaryRecorded { .. } => {
                EventType::new("session.operator_turn_boundary_recorded")
            }
            Self::CompactionSampleRecorded { .. } => {
                EventType::new("session.compaction_sample_recorded")
            }
            Self::ModelUsageRecorded { .. } => EventType::new(MODEL_USAGE_RECORDED_EVENT_TYPE),
            Self::ExecutionOutcomeRecorded { .. } => {
                EventType::new(EXECUTION_OUTCOME_RECORDED_EVENT_TYPE)
            }
            Self::ImprovementActivityRecorded { .. } => {
                EventType::new("session.rsi_activity_recorded")
            }
            Self::StartedWithAgent { .. } => EventType::new(SESSION_STARTED_WITH_AGENT_EVENT_TYPE),
            Self::AgentDefinitionApplied { .. } => {
                EventType::new(AGENT_DEFINITION_APPLIED_EVENT_TYPE)
            }
            Self::AgentDefinitionAppliedAtCheckpoint { .. } => {
                EventType::new("session.agent_definition_checkpoint_applied")
            }
            Self::ModelSampleRejected { .. } => EventType::new(MODEL_SAMPLE_REJECTED_EVENT_TYPE),
            Self::ConsecutiveModelSampleRejected { .. }
            | Self::ConsecutiveModelSampleRejectedV3 { .. } => {
                EventType::new(MODEL_SAMPLE_REJECTED_EVENT_TYPE)
            }
            Self::ModelArgumentCorrectionReset { .. } => {
                EventType::new("model.argument_correction_reset")
            }
            Self::SubcommandCheckpointed { .. } => EventType::new(SUBCOMMAND_CHECKPOINT_EVENT_TYPE),
            Self::Started { .. } => EventType::new(SESSION_STARTED_EVENT_TYPE),
            Self::Resumed { .. } => EventType::new(SESSION_RESUMED_EVENT_TYPE),
            Self::Ended => EventType::new(SESSION_ENDED_EVENT_TYPE),
            Self::ExecutionReceiptRecorded { .. } => EventType::new(RECEIPT_RECORDED_EVENT_TYPE),
            Self::ToolResultCacheHit { .. } => EventType::new(CACHE_HIT_EVENT_TYPE),
            Self::StreamRecovered { .. } => EventType::new(STREAM_RECOVERED_EVENT_TYPE),
            Self::HistoryItemRecorded { .. } => EventType::new(HISTORY_ITEM_RECORDED_EVENT_TYPE),
            Self::CompactionCreated { .. } => EventType::new(COMPACTION_CREATED_EVENT_TYPE),
            Self::RequestAuditRecorded { .. } => EventType::new(REQUEST_AUDIT_RECORDED_EVENT_TYPE),
            Self::VisionRouteDecided { .. } => EventType::new(VISION_ROUTE_DECIDED_EVENT_TYPE),
            Self::VisionObservationRecorded { .. } => {
                EventType::new(VISION_OBSERVATION_RECORDED_EVENT_TYPE)
            }
            Self::VisionAnalysisFailed { .. } => EventType::new(VISION_ANALYSIS_FAILED_EVENT_TYPE),
            Self::ResearchPhaseSelected { .. } => {
                EventType::new(RESEARCH_PHASE_SELECTED_EVENT_TYPE)
            }
            Self::DomainProfileSelected { .. } => {
                EventType::new(DOMAIN_PROFILE_SELECTED_EVENT_TYPE)
            }
        }
    }

    fn encode_payload(&self, event_type: &EventType) -> Result<Value, SessionError> {
        let payload = match self {
            Self::TaskContextCaptured { blob_id } => serde_json::to_value(ContextCapturedPayload {
                schema_version: 1,
                blob_id: blob_id.clone(),
            }),
            Self::TaskSelectionSampleRecorded { sample } => serde_json::to_value(sample),
            Self::TaskSelectionUsageRecorded {
                capture_event_id,
                request_audit_event_id,
                provider_session,
                usage,
            } => serde_json::to_value(TaskSelectionUsagePayload {
                schema_version: 1,
                capture_event_id: capture_event_id.clone(),
                request_audit_event_id: request_audit_event_id.clone(),
                provider_session: provider_session.clone(),
                usage: *usage,
            }),
            Self::TaskContextSelected { selection } => serde_json::to_value(selection),
            Self::CompactionContextCaptured { blob_id } => {
                serde_json::to_value(ContextCapturedPayload {
                    schema_version: 1,
                    blob_id: blob_id.clone(),
                })
            }
            Self::ModelDeliveryCommitted { delivery } => serde_json::to_value(delivery),
            Self::OperatorTurnBoundaryRecorded { boundary } => serde_json::to_value(boundary),
            Self::CompactionSampleRecorded { sample } => serde_json::to_value(sample),
            Self::ModelUsageRecorded {
                request_audit_event_id,
                provider_session,
                usage,
            } => serde_json::to_value(ModelUsagePayload {
                schema_version: CURRENT_SESSION_PAYLOAD_SCHEMA_VERSION,
                request_audit_event_id: request_audit_event_id.clone(),
                provider_session: provider_session.clone(),
                usage: *usage,
            }),
            Self::ExecutionOutcomeRecorded { receipt, result } => {
                serde_json::to_value(ExecutionOutcomePayload {
                    schema_version: CURRENT_SESSION_PAYLOAD_SCHEMA_VERSION,
                    receipt: receipt.clone(),
                    result: result.clone(),
                })
            }
            Self::ImprovementActivityRecorded { activity } => serde_json::to_value(activity),
            Self::StartedWithAgent {
                primary_provider_session,
                binding,
            }
            | Self::AgentDefinitionApplied {
                primary_provider_session,
                binding,
            } => serde_json::to_value(AgentDefinitionPayload {
                schema_version: CURRENT_SESSION_PAYLOAD_SCHEMA_VERSION,
                primary_provider_session: primary_provider_session.clone(),
                binding: binding.clone(),
            }),
            Self::ModelSampleRejected {
                request_audit_event_id,
                diagnostic,
                corrections_used,
            } => serde_json::to_value(ModelSampleRejectedPayload {
                schema_version: CURRENT_SESSION_PAYLOAD_SCHEMA_VERSION,
                request_audit_event_id: request_audit_event_id.clone(),
                diagnostic: *diagnostic,
                corrections_used: *corrections_used,
            }),
            Self::AgentDefinitionAppliedAtCheckpoint {
                primary_provider_session,
                binding,
                attempt_id,
                parent_digest,
                compaction_revision,
                compaction_fingerprint,
            } => serde_json::to_value(checkpoint_binding::CheckpointDefinitionPayload {
                schema_version: 1,
                primary_provider_session: primary_provider_session.clone(),
                binding: binding.clone(),
                attempt_id: attempt_id.clone(),
                parent_digest: parent_digest.clone(),
                compaction_revision: *compaction_revision,
                compaction_fingerprint: compaction_fingerprint.clone(),
            }),
            Self::ConsecutiveModelSampleRejected {
                request_audit_event_id,
                diagnostic,
                corrections_used,
                reset_anchor,
            } => serde_json::to_value(argument_correction::ConsecutiveRejectionPayload {
                schema_version: 2,
                request_audit_event_id: request_audit_event_id.clone(),
                diagnostic: *diagnostic,
                corrections_used: *corrections_used,
                reset_anchor: reset_anchor.clone(),
            }),
            Self::ConsecutiveModelSampleRejectedV3 {
                request_audit_event_id,
                diagnostic,
                corrections_used,
                reset_anchor,
                sampling_step_id,
            } => serde_json::to_value(argument_correction::ConsecutiveRejectionV3Payload {
                schema_version: 3,
                request_audit_event_id: request_audit_event_id.clone(),
                diagnostic: *diagnostic,
                corrections_used: *corrections_used,
                reset_anchor: reset_anchor.clone(),
                sampling_step_id: sampling_step_id.clone(),
            }),
            Self::ModelArgumentCorrectionReset {
                accepted_request_audit_event_id,
                prior_rejection_audit_event_id,
                accepted_history_event_refs,
            } => serde_json::to_value(argument_correction::CorrectionResetPayload {
                schema_version: 1,
                accepted_request_audit_event_id: accepted_request_audit_event_id.clone(),
                prior_rejection_audit_event_id: prior_rejection_audit_event_id.clone(),
                accepted_history_event_refs: accepted_history_event_refs.clone(),
            }),
            Self::SubcommandCheckpointed { checkpoint } => serde_json::to_value(checkpoint),
            Self::Started {
                primary_provider_session,
            }
            | Self::Resumed {
                primary_provider_session,
            } => serde_json::to_value(SessionLifecyclePayload::current(
                primary_provider_session.clone(),
            )),
            Self::Ended => serde_json::to_value(EmptySessionPayload::current()),
            Self::ExecutionReceiptRecorded { receipt } | Self::ToolResultCacheHit { receipt } => {
                serde_json::to_value(ExecutionReceiptEventPayload::current(receipt.clone()))
            }
            Self::StreamRecovered {
                recovery_artifact,
                truncated_from_offset,
                resumed_after_sequence,
            } => serde_json::to_value(StreamRecoveredPayload {
                recovery_artifact: recovery_artifact.clone(),
                truncated_from_offset: *truncated_from_offset,
                resumed_after_sequence: *resumed_after_sequence,
            }),
            Self::HistoryItemRecorded { item } => {
                serde_json::to_value(SessionHistoryItemPayload::current(item.clone()))
            }
            Self::CompactionCreated { compaction } => serde_json::to_value(compaction),
            Self::RequestAuditRecorded { manifest } => serde_json::to_value(manifest),
            Self::VisionRouteDecided { route } => serde_json::to_value(route),
            Self::VisionObservationRecorded { observation } => serde_json::to_value(observation),
            Self::VisionAnalysisFailed { failure } => serde_json::to_value(failure),
            Self::ResearchPhaseSelected { selection } => {
                serde_json::to_value(ResearchPhaseSelectedPayload::current(*selection))
            }
            Self::DomainProfileSelected { selection } => {
                serde_json::to_value(DomainProfileSelectedPayload::current(*selection))
            }
        };
        payload.map_err(|error| SessionError::PayloadEncode {
            event_type: event_type.clone(),
            message: error.to_string(),
        })
    }

    pub(crate) fn decode(event_type: &EventType, payload: Value) -> Result<Self, SessionError> {
        match event_type.as_str() {
            "session.task_context_captured" => {
                let captured: ContextCapturedPayload = decode_strict_payload(event_type, payload)?;
                require_current_payload_schema(event_type, captured.schema_version)?;
                Ok(Self::TaskContextCaptured {
                    blob_id: captured.blob_id,
                })
            }
            "session.task_selection_sample_recorded" => {
                let sample: TaskSelectionSample = decode_strict_payload(event_type, payload)?;
                require_current_payload_schema(event_type, sample.schema_version)?;
                Ok(Self::TaskSelectionSampleRecorded { sample })
            }
            "session.task_selection_usage_recorded" => {
                let payload: TaskSelectionUsagePayload =
                    decode_strict_payload(event_type, payload)?;
                require_current_payload_schema(event_type, payload.schema_version)?;
                Ok(Self::TaskSelectionUsageRecorded {
                    capture_event_id: payload.capture_event_id,
                    request_audit_event_id: payload.request_audit_event_id,
                    provider_session: payload.provider_session,
                    usage: payload.usage,
                })
            }
            "session.task_context_selected" => {
                let selection: TaskContextSelected = decode_strict_payload(event_type, payload)?;
                require_current_payload_schema(event_type, selection.schema_version)?;
                if selection.request_proof_version > task_context::TASK_REQUEST_PROOF_VERSION {
                    return Err(SessionError::TaskContext {
                        message: "unknown task request proof version".to_owned(),
                    });
                }
                Ok(Self::TaskContextSelected { selection })
            }
            "session.model_delivery_committed" => {
                let delivery: ModelDeliveryCommitted = decode_strict_payload(event_type, payload)?;
                Ok(Self::ModelDeliveryCommitted { delivery })
            }
            "session.operator_turn_boundary_recorded" => {
                let boundary: OperatorTurnBoundary = decode_strict_payload(event_type, payload)?;
                Ok(Self::OperatorTurnBoundaryRecorded { boundary })
            }
            MODEL_USAGE_RECORDED_EVENT_TYPE => {
                let payload: ModelUsagePayload = decode_strict_payload(event_type, payload)?;
                require_current_payload_schema(event_type, payload.schema_version)?;
                Ok(Self::ModelUsageRecorded {
                    request_audit_event_id: payload.request_audit_event_id,
                    provider_session: payload.provider_session,
                    usage: payload.usage,
                })
            }
            "session.rsi_activity_recorded" => {
                let activity: ImprovementActivity = decode_strict_payload(event_type, payload)?;
                activity.validate_schema()?;
                Ok(Self::ImprovementActivityRecorded { activity })
            }
            SESSION_STARTED_WITH_AGENT_EVENT_TYPE | AGENT_DEFINITION_APPLIED_EVENT_TYPE => {
                let payload: AgentDefinitionPayload = decode_strict_payload(event_type, payload)?;
                require_current_payload_schema(event_type, payload.schema_version)?;
                payload
                    .binding
                    .validate()
                    .map_err(|error| SessionError::AgentDefinition {
                        message: error.to_string(),
                    })?;
                if event_type.as_str() == SESSION_STARTED_WITH_AGENT_EVENT_TYPE {
                    Ok(Self::StartedWithAgent {
                        primary_provider_session: payload.primary_provider_session,
                        binding: payload.binding,
                    })
                } else {
                    Ok(Self::AgentDefinitionApplied {
                        primary_provider_session: payload.primary_provider_session,
                        binding: payload.binding,
                    })
                }
            }
            MODEL_SAMPLE_REJECTED_EVENT_TYPE => {
                if payload.get("schema_version").and_then(Value::as_u64) == Some(3) {
                    let payload: argument_correction::ConsecutiveRejectionV3Payload =
                        decode_strict_payload(event_type, payload)?;
                    if payload.sampling_step_id.trim().is_empty() {
                        return Err(SessionError::InvalidModelSampleRejection);
                    }
                    return Ok(Self::ConsecutiveModelSampleRejectedV3 {
                        request_audit_event_id: payload.request_audit_event_id,
                        diagnostic: payload.diagnostic,
                        corrections_used: payload.corrections_used,
                        reset_anchor: payload.reset_anchor,
                        sampling_step_id: payload.sampling_step_id,
                    });
                }
                if payload.get("schema_version").and_then(Value::as_u64) == Some(2) {
                    let payload: argument_correction::ConsecutiveRejectionPayload =
                        decode_strict_payload(event_type, payload)?;
                    return Ok(Self::ConsecutiveModelSampleRejected {
                        request_audit_event_id: payload.request_audit_event_id,
                        diagnostic: payload.diagnostic,
                        corrections_used: payload.corrections_used,
                        reset_anchor: payload.reset_anchor,
                    });
                }
                let payload: ModelSampleRejectedPayload =
                    decode_strict_payload(event_type, payload)?;
                require_current_payload_schema(event_type, payload.schema_version)?;
                Ok(Self::ModelSampleRejected {
                    request_audit_event_id: payload.request_audit_event_id,
                    diagnostic: payload.diagnostic,
                    corrections_used: payload.corrections_used,
                })
            }
            "session.agent_definition_checkpoint_applied" => {
                let payload: checkpoint_binding::CheckpointDefinitionPayload =
                    decode_strict_payload(event_type, payload)?;
                require_current_payload_schema(event_type, payload.schema_version)?;
                payload
                    .binding
                    .validate()
                    .map_err(|error| SessionError::AgentDefinition {
                        message: error.to_string(),
                    })?;
                Ok(Self::AgentDefinitionAppliedAtCheckpoint {
                    primary_provider_session: payload.primary_provider_session,
                    binding: payload.binding,
                    attempt_id: payload.attempt_id,
                    parent_digest: payload.parent_digest,
                    compaction_revision: payload.compaction_revision,
                    compaction_fingerprint: payload.compaction_fingerprint,
                })
            }
            "model.argument_correction_reset" => {
                let payload: argument_correction::CorrectionResetPayload =
                    decode_strict_payload(event_type, payload)?;
                require_current_payload_schema(event_type, payload.schema_version)?;
                Ok(Self::ModelArgumentCorrectionReset {
                    accepted_request_audit_event_id: payload.accepted_request_audit_event_id,
                    prior_rejection_audit_event_id: payload.prior_rejection_audit_event_id,
                    accepted_history_event_refs: payload.accepted_history_event_refs,
                })
            }
            SUBCOMMAND_CHECKPOINT_EVENT_TYPE => {
                let checkpoint: SubcommandCheckpoint = decode_strict_payload(event_type, payload)?;
                checkpoint
                    .validate()
                    .map_err(|_| SessionError::SubcommandCheckpointMismatch)?;
                Ok(Self::SubcommandCheckpointed { checkpoint })
            }
            SESSION_STARTED_EVENT_TYPE => {
                let payload: SessionLifecyclePayload = decode_strict_payload(event_type, payload)?;
                require_current_payload_schema(event_type, payload.schema_version)?;
                Ok(Self::Started {
                    primary_provider_session: payload.primary_provider_session,
                })
            }
            SESSION_RESUMED_EVENT_TYPE => {
                let payload: SessionLifecyclePayload = decode_strict_payload(event_type, payload)?;
                require_current_payload_schema(event_type, payload.schema_version)?;
                Ok(Self::Resumed {
                    primary_provider_session: payload.primary_provider_session,
                })
            }
            SESSION_ENDED_EVENT_TYPE => {
                let payload: EmptySessionPayload = decode_strict_payload(event_type, payload)?;
                require_current_payload_schema(event_type, payload.schema_version)?;
                Ok(Self::Ended)
            }
            EXECUTION_OUTCOME_RECORDED_EVENT_TYPE => {
                let payload: ExecutionOutcomePayload = decode_strict_payload(event_type, payload)?;
                require_current_payload_schema(event_type, payload.schema_version)?;
                Ok(Self::ExecutionOutcomeRecorded {
                    receipt: decode_execution_receipt(
                        serde_json::to_value(ExecutionReceiptEventPayload::current(
                            payload.receipt,
                        ))
                        .map_err(|error| {
                            SessionError::MalformedPayload {
                                event_type: event_type.clone(),
                                message: error.to_string(),
                            }
                        })?,
                        event_type,
                    )?,
                    result: payload.result,
                })
            }
            RECEIPT_RECORDED_EVENT_TYPE => decode_execution_receipt(payload, event_type)
                .map(|receipt| Self::ExecutionReceiptRecorded { receipt }),
            CACHE_HIT_EVENT_TYPE => decode_execution_receipt(payload, event_type)
                .map(|receipt| Self::ToolResultCacheHit { receipt }),
            STREAM_RECOVERED_EVENT_TYPE => {
                let payload: StreamRecoveredPayload = decode_strict_payload(event_type, payload)?;
                if payload.recovery_artifact.is_empty() {
                    return Err(SessionError::MalformedPayload {
                        event_type: event_type.clone(),
                        message: "StreamRecovered recovery_artifact must not be empty".to_owned(),
                    });
                }
                Ok(Self::StreamRecovered {
                    recovery_artifact: payload.recovery_artifact,
                    truncated_from_offset: payload.truncated_from_offset,
                    resumed_after_sequence: payload.resumed_after_sequence,
                })
            }
            "session.compaction_context_captured" => {
                let captured: ContextCapturedPayload = decode_strict_payload(event_type, payload)?;
                require_current_payload_schema(event_type, captured.schema_version)?;
                crate::domain::artifacts::BlobId::new(captured.blob_id.as_str()).map_err(
                    |error| SessionError::MalformedPayload {
                        event_type: event_type.clone(),
                        message: error.to_string(),
                    },
                )?;
                Ok(Self::CompactionContextCaptured {
                    blob_id: captured.blob_id,
                })
            }
            "session.compaction_sample_recorded" => {
                let sample: CompactionModelSample = decode_strict_payload(event_type, payload)?;
                sample.validate()?;
                Ok(Self::CompactionSampleRecorded { sample })
            }
            HISTORY_ITEM_RECORDED_EVENT_TYPE => {
                let payload: SessionHistoryItemPayload =
                    decode_strict_payload(event_type, payload)?;
                require_current_payload_schema(event_type, payload.schema_version)?;
                Ok(Self::HistoryItemRecorded { item: payload.item })
            }
            COMPACTION_CREATED_EVENT_TYPE => {
                let compaction: SessionCompaction = decode_strict_payload(event_type, payload)?;
                Ok(Self::CompactionCreated { compaction })
            }
            REQUEST_AUDIT_RECORDED_EVENT_TYPE => {
                let manifest: RequestAuditManifest = decode_strict_payload(event_type, payload)?;
                manifest
                    .validate()
                    .map_err(|error| SessionError::MalformedPayload {
                        event_type: event_type.clone(),
                        message: error.to_string(),
                    })?;
                Ok(Self::RequestAuditRecorded { manifest })
            }
            VISION_ROUTE_DECIDED_EVENT_TYPE => {
                let route: VisionRouteRecord = decode_strict_payload(event_type, payload)?;
                route.validate().map_err(vision_payload_error(event_type))?;
                Ok(Self::VisionRouteDecided { route })
            }
            VISION_OBSERVATION_RECORDED_EVENT_TYPE => {
                let observation: VisionObservation = decode_strict_payload(event_type, payload)?;
                observation
                    .validate()
                    .map_err(vision_payload_error(event_type))?;
                Ok(Self::VisionObservationRecorded { observation })
            }
            VISION_ANALYSIS_FAILED_EVENT_TYPE => {
                let failure: VisionAnalysisFailure = decode_strict_payload(event_type, payload)?;
                failure
                    .validate()
                    .map_err(vision_payload_error(event_type))?;
                Ok(Self::VisionAnalysisFailed { failure })
            }
            RESEARCH_PHASE_SELECTED_EVENT_TYPE => {
                let payload: ResearchPhaseSelectedPayload =
                    decode_strict_payload(event_type, payload)?;
                require_current_payload_schema(event_type, payload.schema_version)?;
                let selection = payload.selection;
                selection
                    .validate()
                    .map_err(|error| SessionError::MalformedPayload {
                        event_type: event_type.clone(),
                        message: error.to_string(),
                    })?;
                Ok(Self::ResearchPhaseSelected { selection })
            }
            DOMAIN_PROFILE_SELECTED_EVENT_TYPE => {
                let payload: DomainProfileSelectedPayload =
                    decode_strict_payload(event_type, payload)?;
                require_current_payload_schema(event_type, payload.schema_version)?;
                let selection = payload.selection;
                selection
                    .validate()
                    .map_err(|error| SessionError::MalformedPayload {
                        event_type: event_type.clone(),
                        message: error.to_string(),
                    })?;
                Ok(Self::DomainProfileSelected { selection })
            }
            _ => Err(SessionError::UnknownEventType {
                event_type: event_type.clone(),
            }),
        }
    }
}

/// The pure Session state reconstructed from one ordered Session stream.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionState {
    task_context_captures: Vec<(EventId, EventSequence, crate::domain::artifacts::BlobId)>,
    task_selection_samples: Vec<(EventId, TaskSelectionSample)>,
    task_context_selections: Vec<(EventId, TaskContextSelected)>,
    task_compacted_selection: Option<(EventId, Vec<EventId>, String)>,
    improvement_records: Vec<improvement::ImprovementRecord>,
    agent_definition_binding: Option<AgentDefinitionBinding>,
    event_state: State,
    stream_id: Option<StreamId>,
    lifecycle: SessionLifecycle,
    primary_provider_session: Option<ProviderSessionRef>,
    used_primary_provider_session_ids: BTreeSet<ProviderSessionId>,
    history: ConversationHistory,
    history_event_ids: Vec<EventId>,
    history_event_sequences: Vec<EventSequence>,
    history_event_times: Vec<OccurredAt>,
    task_navigation: TaskNavigationIndex,
    operator_boundaries: Vec<OperatorTurnBoundary>,
    operator_boundary_event_ids: Vec<EventId>,
    model_deliveries: Vec<ModelDeliveryCommitted>,
    recovered_history_item_count: usize,
    recovered_history_start: Option<usize>,
    resume_count: u64,
    execution_receipt_count: u64,
    execution_receipts: Vec<SessionExecutionReceiptRecord>,
    subcommand_checkpoints: Vec<SubcommandCheckpoint>,
    cache_hit_count: u64,
    recovery_count: u64,
    latest_compaction: Option<SessionCompaction>,
    compaction_samples: Vec<(EventId, EventSequence, CompactionModelSample)>,
    request_audits: Vec<RequestAuditManifest>,
    request_audit_sequences: Vec<EventSequence>,
    latest_audit_history_len: usize,
    model_usage: BTreeMap<EventId, ModelUsage>,
    rejected_requests: BTreeSet<EventId>,
    consecutive_rejection: Option<argument_correction::ConsecutiveRejection>,
    correction_reset_anchor: Option<EventId>,
    vision_routes: Vec<VisionRouteRecord>,
    vision_route_sequences: Vec<EventSequence>,
    vision_observations: Vec<VisionObservation>,
    vision_observation_event_ids: Vec<(EventId, EventSequence)>,
    vision_failures: Vec<VisionAnalysisFailure>,
    research_phase_selection: Option<ResearchPhaseSelection>,
    domain_profile_selection: Option<DomainProfileSelection>,
}

impl Default for SessionState {
    fn default() -> Self {
        Self {
            task_context_captures: Vec::new(),
            task_selection_samples: Vec::new(),
            task_context_selections: Vec::new(),
            task_compacted_selection: None,
            improvement_records: Vec::new(),
            agent_definition_binding: None,
            event_state: State::default(),
            stream_id: None,
            lifecycle: SessionLifecycle::Unstarted,
            primary_provider_session: None,
            used_primary_provider_session_ids: BTreeSet::new(),
            history: ConversationHistory::default(),
            history_event_ids: Vec::new(),
            history_event_sequences: Vec::new(),
            history_event_times: Vec::new(),
            task_navigation: TaskNavigationIndex::default(),
            operator_boundaries: Vec::new(),
            operator_boundary_event_ids: Vec::new(),
            model_deliveries: Vec::new(),
            recovered_history_item_count: 0,
            recovered_history_start: None,
            resume_count: 0,
            execution_receipt_count: 0,
            execution_receipts: Vec::new(),
            subcommand_checkpoints: Vec::new(),
            cache_hit_count: 0,
            recovery_count: 0,
            latest_compaction: None,
            compaction_samples: Vec::new(),
            request_audits: Vec::new(),
            request_audit_sequences: Vec::new(),
            latest_audit_history_len: 0,
            model_usage: BTreeMap::new(),
            rejected_requests: BTreeSet::new(),
            consecutive_rejection: None,
            correction_reset_anchor: None,
            vision_routes: Vec::new(),
            vision_route_sequences: Vec::new(),
            vision_observations: Vec::new(),
            vision_observation_event_ids: Vec::new(),
            vision_failures: Vec::new(),
            research_phase_selection: None,
            domain_profile_selection: None,
        }
    }
}

impl SessionState {
    pub(crate) fn compaction_samples(&self) -> &[(EventId, EventSequence, CompactionModelSample)] {
        &self.compaction_samples
    }
    pub fn agent_definition_binding(&self) -> Option<&AgentDefinitionBinding> {
        self.agent_definition_binding.as_ref()
    }

    pub fn stream_id(&self) -> Option<&StreamId> {
        self.stream_id.as_ref()
    }

    pub const fn lifecycle(&self) -> SessionLifecycle {
        self.lifecycle
    }

    pub fn primary_provider_session(&self) -> Option<&ProviderSessionRef> {
        self.primary_provider_session.as_ref()
    }

    /// The exact History items replayed from durable Session facts. It may
    /// contain an interrupted Tool Call until the normalized recovery view is
    /// requested.
    pub fn history(&self) -> &ConversationHistory {
        &self.history
    }

    /// Produces the active recovery History using the sole History normalizer.
    /// After Compaction, the retained normalized suffix is followed only by
    /// durable History facts recorded after that boundary; the original full
    /// History remains available through [`Self::history`].
    pub(crate) fn normalized_history_items(&self) -> Result<Vec<ContentItem>, HistoryError> {
        let Some(compaction) = self.latest_compaction.as_ref() else {
            return self.full_normalized_history();
        };
        let mut items = compaction.retained_history().to_vec();
        items.extend_from_slice(&self.history.items()[compaction.source_history_item_count()..]);
        self.normalize_items(&items)
    }

    pub(crate) fn full_normalized_history(&self) -> Result<Vec<ContentItem>, HistoryError> {
        self.normalize_items(self.history.items())
    }

    /// Match each normalized item to its original History Event when one
    /// exists. Synthesized recovery results have no History Event identity.
    pub(crate) fn normalized_history_event_ids(
        &self,
    ) -> Result<Vec<Option<EventId>>, SessionError> {
        let history = self.history_items_with_event_ids().collect::<Vec<_>>();
        let normalized = self.full_normalized_history()?;
        let mut cursor = history.len();
        let mut ids = Vec::with_capacity(normalized.len());
        for item in normalized.iter().rev() {
            if let Some(index) = history[..cursor]
                .iter()
                .rposition(|(_, original)| *original == item)
            {
                ids.push(Some(history[index].0.clone()));
                cursor = index;
            } else if matches!(item, ContentItem::ToolResult { .. }) {
                ids.push(None);
            } else {
                return Err(SessionError::CompactionContextProvenance {
                    message: "normalized History item has no original Event".to_owned(),
                });
            }
        }
        ids.reverse();
        Ok(ids)
    }

    fn normalize_items(&self, items: &[ContentItem]) -> Result<Vec<ContentItem>, HistoryError> {
        let results = self
            .execution_receipts
            .iter()
            .filter_map(|record| record.result.as_ref())
            .collect::<Vec<_>>();
        normalize_with_recorded_results(items, &self.subcommand_checkpoints, &results)
    }

    pub(crate) fn recorded_tool_result(&self, call_id: &ToolCallId) -> Option<&ToolResult> {
        self.execution_receipts
            .iter()
            .filter_map(|record| record.result.as_ref())
            .find(|result| &result.call_id == call_id)
    }

    /// A completed boundary may retain an explicitly recovered older unknown
    /// execution, but cannot normalize a newly missing Result or manufacture
    /// one for a known Receipt. Actual recorded outcomes are already durable.
    pub(crate) fn ensure_completed_history(&self) -> Result<(), HistoryError> {
        let recovered_calls = self.history.items()[..self.recovered_history_item_count]
            .iter()
            .filter_map(|item| match item {
                ContentItem::ToolCall { call } => Some(&call.call_id),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        for (call_id, _) in self.history.unpaired_tool_calls_in_call_order() {
            if self.recorded_tool_result(&call_id).is_some() {
                continue;
            }
            if !recovered_calls.contains(&call_id)
                || self.execution_receipts.iter().any(|record| {
                    record.receipt.execution_id
                        == super::tools::ExecutionId::for_model_call(&call_id)
                })
            {
                return Err(HistoryError::MissingToolResult { call_id });
            }
        }
        Ok(())
    }

    pub(crate) fn subcommand_checkpoints(&self) -> &[SubcommandCheckpoint] {
        &self.subcommand_checkpoints
    }

    /// Whether a durable `session.resumed` fact has established the current
    /// lifecycle's resumed Provider identity.
    pub(crate) const fn has_resumed(&self) -> bool {
        self.resume_count > 0
    }

    pub const fn execution_receipt_count(&self) -> u64 {
        self.execution_receipt_count
    }

    /// Existing Receipt facts in append order, with their authoritative Event
    /// identities. This is derived strictly through the Session reducer.
    pub(crate) fn execution_receipts(&self) -> &[SessionExecutionReceiptRecord] {
        &self.execution_receipts
    }

    /// Existing History facts in append order paired with their authoritative
    /// Event identities, without duplicating the stored ContentItems.
    pub(crate) fn history_items_with_event_ids(
        &self,
    ) -> impl DoubleEndedIterator<Item = (&EventId, &ContentItem)> + Clone {
        self.history_event_ids.iter().zip(self.history.items())
    }

    pub const fn cache_hit_count(&self) -> u64 {
        self.cache_hit_count
    }

    pub const fn recovery_count(&self) -> u64 {
        self.recovery_count
    }

    /// The latest derived Summary/retention view reconstructed from the
    /// ordered append-only Session Event stream.
    pub fn latest_compaction(&self) -> Option<&SessionCompaction> {
        self.latest_compaction.as_ref()
    }

    pub fn next_compaction(
        &self,
        summary: SessionSummaryDraft,
        requested_recent_items: usize,
        validation: &CompactionValidationContext<'_>,
    ) -> Result<SessionCompaction, SessionError> {
        self.ensure_no_unknown_improvement()?;
        require_transition(
            self.lifecycle,
            "session.compaction_created",
            SessionLifecycle::Active,
        )?;
        let revision = match self.latest_compaction.as_ref() {
            Some(compaction) => increment("Compaction revision", compaction.summary().revision())?,
            None => 1,
        };
        let source_event_sequence = self
            .stream_id()
            .and_then(|stream_id| self.event_state.last_sequence_for(stream_id))
            .ok_or(SessionError::CompactionSourceSequenceMissing)?;
        if let Some(task) = summary.task_references()
            && (Some(task.source().stream_id()) != self.stream_id()
                || task.source().watermark() != source_event_sequence)
        {
            return Err(SessionError::CompactionContextProvenance {
                message: "task references must bind this exact Session prefix".to_owned(),
            });
        }
        let normalized = self.full_normalized_history()?;
        let summary = SessionSummary::from_draft(summary, revision, validation)?;
        let needs_event_ids = summary.schema_version() == 7
            || (summary.is_preview() && self.latest_task_context_selection().is_some());
        let ids = needs_event_ids
            .then(|| self.normalized_history_event_ids())
            .transpose()?;
        SessionCompaction::from_history(
            &self.history,
            &normalized,
            summary,
            source_event_sequence,
            requested_recent_items,
            ids.as_deref(),
        )
    }

    pub fn applied_event_count(&self) -> usize {
        self.event_state.applied_event_count()
    }

    pub(crate) fn contains_event_id(&self, event_id: &EventId) -> bool {
        self.event_state.contains_event_id(event_id)
    }

    /// The immutable audit manifests committed through this Session stream in
    /// append order. Blob reconstruction remains outside the pure reducer.
    pub fn request_audits(&self) -> &[RequestAuditManifest] {
        &self.request_audits
    }

    /// Ordered, durable decisions made before an image could enter a Provider request.
    pub fn vision_routes(&self) -> &[VisionRouteRecord] {
        &self.vision_routes
    }

    /// Ordered model-derived observations. These remain distinct from Research evidence.
    pub fn vision_observations(&self) -> &[VisionObservation] {
        &self.vision_observations
    }

    pub fn vision_failures(&self) -> &[VisionAnalysisFailure] {
        &self.vision_failures
    }

    /// The latest explicit durable Phase selection. A missing value remains
    /// distinguishable from a real Mapping Event for legacy replay.
    pub const fn research_phase_selection(&self) -> Option<ResearchPhaseSelection> {
        self.research_phase_selection
    }

    /// Read-only legacy projection. It never backfills the Session stream.
    pub fn effective_research_phase(&self) -> ResearchPhase {
        self.research_phase_selection
            .map(ResearchPhaseSelection::phase)
            .or_else(|| {
                self.agent_definition_binding
                    .as_ref()?
                    .definition()
                    .research()
                    .map(|research| research.phase())
            })
            .unwrap_or(ResearchPhase::Mapping)
    }

    /// The latest explicit durable Domain selection. Legacy Sessions remain
    /// read-only until their next real Turn appends the stable source default.
    pub const fn domain_profile_selection(&self) -> Option<DomainProfileSelection> {
        self.domain_profile_selection
    }

    pub fn effective_domain_profile(&self) -> DomainProfile {
        self.domain_profile_selection
            .map(DomainProfileSelection::profile)
            .or_else(|| {
                self.agent_definition_binding
                    .as_ref()?
                    .definition()
                    .research()
                    .map(|research| research.domain())
            })
            .unwrap_or(DomainProfile::Source)
    }

    /// Returns the precise next Session Event sequence without allocating or
    /// mutating it. The existing Event Store remains sequence/durability owner.
    pub(crate) fn next_event_sequence(&self) -> Result<EventSequence, SessionError> {
        let stream_id = self
            .stream_id()
            .ok_or(SessionError::RequestAuditSessionStreamMissing)?;
        match self.event_state.last_sequence_for(stream_id) {
            Some(last) => Ok(EventSequence::new(last.get().checked_add(1).ok_or_else(
                || {
                    SessionError::Event(EventError::SequenceOverflow {
                        stream_id: stream_id.clone(),
                        last,
                    })
                },
            )?)),
            None => Ok(EventSequence::new(1)),
        }
    }
}

/// The sole pure reducer for known Session event types.
#[derive(Clone, Copy, Debug, Default)]
pub struct SessionReducer;

impl SessionReducer {
    pub fn reduce<Events>(&self, events: Events) -> Result<SessionState, SessionError>
    where
        Events: IntoIterator<Item = EventEnvelope<Value>>,
    {
        let mut state = SessionState::default();
        for event in events {
            self.apply_in_place(&mut state, event)?;
        }
        Ok(state)
    }

    /// Validates and reduces an Event without mutating `state` if any envelope,
    /// payload, stream or transition rule fails.
    pub fn apply(
        &self,
        state: &mut SessionState,
        event: EventEnvelope<Value>,
    ) -> Result<ApplyOutcome, SessionError> {
        let mut next = state.clone();
        let outcome = self.apply_in_place(&mut next, event)?;
        *state = next;
        Ok(outcome)
    }

    /// Applies one transition to state exclusively owned by the caller.
    /// Full replay owns a fresh state and discards it on any error; public
    /// incremental apply stages a clone before entering this owner.
    pub(crate) fn apply_in_place(
        &self,
        state: &mut SessionState,
        event: EventEnvelope<Value>,
    ) -> Result<ApplyOutcome, SessionError> {
        let event = EventUpcaster::upcast(&CurrentEventUpcaster, event)?;
        validate_session_stream(state, &event.stream_id)?;
        let session_event = SessionEvent::decode(&event.event_type, event.payload.clone())?;
        let stream_id = event.stream_id.clone();
        let sequence = event.sequence;
        let event_id = event.event_id.clone();
        let occurred_at = event.occurred_at.clone();
        let source_event_sequence = state
            .stream_id()
            .and_then(|stream_id| state.event_state.last_sequence_for(stream_id));

        let outcome = EventReducer.apply(&mut state.event_state, event)?;
        if outcome == ApplyOutcome::DuplicateIgnored {
            if let SessionEvent::RequestAuditRecorded { manifest } = session_event {
                let exact_existing = state.request_audits.iter().any(|existing| {
                    existing.audit_event_id() == &event_id && existing == &manifest
                });
                if !exact_existing {
                    return Err(SessionError::RequestAuditDuplicateEventConflict { event_id });
                }
            }
            return Ok(outcome);
        }
        require_next_sequence(source_event_sequence, &stream_id, sequence)?;
        state.stream_id = Some(stream_id);
        apply_session_event(
            state,
            session_event,
            source_event_sequence,
            &event_id,
            &occurred_at,
        )?;
        Ok(outcome)
    }
}

fn decode_execution_receipt(
    payload: Value,
    event_type: &EventType,
) -> Result<ExecutionReceipt, SessionError> {
    ExecutionReceiptEventPayload::decode_current(payload).map_err(|error| match error {
        ExecutionReceiptPayloadError::Malformed { message } => SessionError::MalformedPayload {
            event_type: event_type.clone(),
            message,
        },
        ExecutionReceiptPayloadError::UnsupportedSchemaVersion { schema_version } => {
            SessionError::UnsupportedPayloadSchema {
                event_type: event_type.clone(),
                schema_version,
            }
        }
    })
}

fn require_next_sequence(
    previous: Option<EventSequence>,
    stream_id: &StreamId,
    received: EventSequence,
) -> Result<(), SessionError> {
    let expected = match previous {
        Some(last) => EventSequence::new(last.get().checked_add(1).ok_or_else(|| {
            SessionError::Event(EventError::SequenceOverflow {
                stream_id: stream_id.clone(),
                last,
            })
        })?),
        None => EventSequence::new(1),
    };
    if received == expected {
        Ok(())
    } else {
        Err(SessionError::Event(EventError::UnexpectedSequence {
            stream_id: stream_id.clone(),
            expected,
            received,
        }))
    }
}

fn validate_session_stream(state: &SessionState, stream_id: &StreamId) -> Result<(), SessionError> {
    if stream_id
        .as_str()
        .strip_prefix(SESSION_STREAM_PREFIX)
        .is_none_or(str::is_empty)
    {
        return Err(SessionError::UnexpectedStream {
            stream_id: stream_id.clone(),
        });
    }
    if let Some(expected) = state.stream_id()
        && expected != stream_id
    {
        return Err(SessionError::StreamChanged {
            expected: expected.clone(),
            received: stream_id.clone(),
        });
    }
    Ok(())
}

fn validate_child_receipt(
    state: &SessionState,
    receipt: &ExecutionReceipt,
) -> Result<(), SessionError> {
    for child in state
        .subcommand_checkpoints
        .iter()
        .filter(|child| child.execution_id == receipt.execution_id)
    {
        if !child.agrees_with_receipt(receipt) {
            return Err(SessionError::SubcommandCheckpointMismatch);
        }
    }
    Ok(())
}

fn apply_session_event(
    state: &mut SessionState,
    event: SessionEvent,
    source_event_sequence: Option<EventSequence>,
    event_id: &EventId,
    occurred_at: &OccurredAt,
) -> Result<(), SessionError> {
    match event {
        SessionEvent::TaskContextCaptured { blob_id } => {
            require_transition(
                state.lifecycle,
                "session.task_context_captured",
                SessionLifecycle::Active,
            )?;
            state.ensure_completed_history()?;
            let previous = source_event_sequence.expect("active Session has a preceding sequence");
            state.task_context_captures.push((
                event_id.clone(),
                EventSequence::new(previous.get() + 1),
                blob_id,
            ));
        }
        SessionEvent::TaskSelectionSampleRecorded { sample } => {
            require_transition(
                state.lifecycle,
                "session.task_selection_sample_recorded",
                SessionLifecycle::Active,
            )?;
            task_context::apply_sample(state, sample, event_id)?;
        }
        SessionEvent::TaskSelectionUsageRecorded {
            capture_event_id,
            request_audit_event_id,
            provider_session,
            usage,
        } => {
            require_transition(
                state.lifecycle,
                "session.task_selection_usage_recorded",
                SessionLifecycle::Active,
            )?;
            usage.validate()?;
            let invalid = || SessionError::InvalidModelUsageReference {
                request_audit_event_id: request_audit_event_id.clone(),
            };
            let audit = state.request_audits.last().ok_or_else(invalid)?;
            let expected_prefix = format!("task-selection:{capture_event_id}:");
            if state.task_context_captures.last().is_none_or(|(id, _, _)| id != &capture_event_id)
                || provider_session.role != ProviderSessionRole::Primary
                || provider_session.generation != 0
                || !matches!(provider_session.id.to_string().strip_prefix(&expected_prefix), Some("0" | "1"))
                || state.primary_provider_session.as_ref() == Some(&provider_session)
                || audit.audit_event_id() != &request_audit_event_id
                || audit.provider_session() != Some(&provider_session)
                || audit.improvement_scope().is_some()
                || audit.delivery().mode() != crate::domain::request_audit::RequestAuditDeliveryMode::Full
                || audit.context_delivery_map().is_none()
                || audit.components().iter().any(|component| component.source() == crate::domain::request_audit::RequestAuditComponentSource::RenderedToolSpec)
                || state.model_usage.contains_key(&request_audit_event_id)
            {
                return Err(invalid());
            }
            state.model_usage.insert(request_audit_event_id, usage);
        }
        SessionEvent::TaskContextSelected { selection } => {
            require_transition(
                state.lifecycle,
                "session.task_context_selected",
                SessionLifecycle::Active,
            )?;
            task_context::apply_selection(state, selection, event_id)?;
        }
        SessionEvent::CompactionContextCaptured { .. } => {
            require_transition(
                state.lifecycle,
                "session.compaction_context_captured",
                SessionLifecycle::Active,
            )?;
            state.ensure_completed_history()?;
        }
        SessionEvent::ModelDeliveryCommitted { delivery } => {
            require_transition(
                state.lifecycle,
                "session.model_delivery_committed",
                SessionLifecycle::Active,
            )?;
            model_delivery::apply(state, delivery)?;
        }
        SessionEvent::OperatorTurnBoundaryRecorded { boundary } => {
            require_transition(
                state.lifecycle,
                "session.operator_turn_boundary_recorded",
                SessionLifecycle::Active,
            )?;
            operator_boundary::apply(state, boundary, source_event_sequence)?;
            state.operator_boundary_event_ids.push(event_id.clone());
        }
        SessionEvent::CompactionSampleRecorded { sample } => {
            require_transition(
                state.lifecycle,
                "session.compaction_sample_recorded",
                SessionLifecycle::Active,
            )?;
            state.ensure_completed_history()?;
            sample.validate()?;
            let audit = state.request_audits.last().ok_or_else(|| {
                SessionError::CompactionContextProvenance {
                    message: "Compaction sample requires its actual audited request".to_owned(),
                }
            })?;
            if audit.audit_event_id() != sample.audit_event_id()
                || audit.provider_session() != Some(sample.provider_session())
                || audit.improvement_scope().is_some()
                || audit.delivery().mode() != crate::domain::request_audit::RequestAuditDeliveryMode::Full
                || audit.resolved_tool_plan_fingerprint().is_none()
                || audit.context_delivery_map().is_none()
                || audit.components().iter().any(|component| component.source() == crate::domain::request_audit::RequestAuditComponentSource::RenderedToolSpec)
                || Some(sample.source().stream_id()) != state.stream_id()
                || source_event_sequence
                    .is_none_or(|sequence| sample.source().watermark() >= sequence)
                || state
                    .compaction_samples
                    .iter()
                    .any(|(_, _, previous)| previous.provider_session() == sample.provider_session())
            {
                return Err(SessionError::CompactionContextProvenance {
                    message: "Compaction sample source/audit/scope mismatch".to_owned(),
                });
            }
            let sequence = EventSequence::new(
                source_event_sequence
                    .and_then(|sequence| sequence.get().checked_add(1))
                    .ok_or(SessionError::StateOverflow {
                        field: "Compaction sample sequence",
                    })?,
            );
            state
                .compaction_samples
                .push((event_id.clone(), sequence, sample));
        }
        SessionEvent::ModelUsageRecorded {
            request_audit_event_id,
            provider_session,
            usage,
        } => {
            require_transition(
                state.lifecycle,
                MODEL_USAGE_RECORDED_EVENT_TYPE,
                SessionLifecycle::Active,
            )?;
            usage.validate()?;
            let invalid = || SessionError::InvalidModelUsageReference {
                request_audit_event_id: request_audit_event_id.clone(),
            };
            let audit = state
                .request_audits
                .iter()
                .rev()
                .find(|audit| audit.provider_session() == Some(&provider_session))
                .ok_or_else(invalid)?;
            // Usage is recorded at the immediate accepted-sample checkpoint.
            // An earlier attempt in this exact scope may have failed before
            // completion, so it cannot stand in for the supplied final audit.
            if audit.audit_event_id() != &request_audit_event_id
                || state.model_usage.contains_key(&request_audit_event_id)
                || state.rejected_requests.contains(&request_audit_event_id)
            {
                return Err(invalid());
            }
            let accepted = match (provider_session.role, audit.improvement_scope()) {
                (ProviderSessionRole::Primary, Some(scope)) => {
                    state.improvement_records.iter().any(|record| {
                        matches!(
                            &record.activity.fact,
                            ImprovementFact::Sample { audit_event_id, provider_session, .. }
                                if audit_event_id == &request_audit_event_id
                                    && provider_session == scope
                        )
                    })
                }
                (ProviderSessionRole::Primary, None) => {
                    state.primary_provider_session.as_ref() == Some(&provider_session)
                        || state.task_selection_samples.iter().any(|(_, sample)| {
                            sample.audit_event_id == request_audit_event_id
                                && sample.provider_session == provider_session
                        })
                        || state.compaction_samples.iter().any(|(_, _, sample)| {
                            sample.audit_event_id() == &request_audit_event_id
                                && sample.provider_session() == &provider_session
                        })
                }
                (ProviderSessionRole::Vision, None) => {
                    state.vision_observations.iter().any(|observation| {
                        observation.request_audit_event_id == request_audit_event_id
                            && observation.provider_session == provider_session
                    })
                }
                (ProviderSessionRole::Vision, Some(_)) => false,
            };
            if !accepted {
                return Err(invalid());
            }
            state.model_usage.insert(request_audit_event_id, usage);
        }
        SessionEvent::ImprovementActivityRecorded { activity } => {
            improvement::apply(state, activity, event_id)?
        }
        SessionEvent::StartedWithAgent {
            primary_provider_session,
            binding,
        } => {
            require_primary_provider_session(&primary_provider_session)?;
            require_transition(
                state.lifecycle,
                SESSION_STARTED_WITH_AGENT_EVENT_TYPE,
                SessionLifecycle::Unstarted,
            )?;
            record_fresh_primary_provider_session_id(state, &primary_provider_session)?;
            state.agent_definition_binding = Some(binding);
            state.lifecycle = SessionLifecycle::Active;
            state.primary_provider_session = Some(primary_provider_session);
        }
        SessionEvent::AgentDefinitionAppliedAtCheckpoint {
            primary_provider_session,
            binding,
            attempt_id,
            parent_digest,
            compaction_revision,
            compaction_fingerprint,
        } => {
            checkpoint_binding::apply(
                state,
                checkpoint_binding::CheckpointDefinitionPayload {
                    schema_version: 1,
                    primary_provider_session,
                    binding,
                    attempt_id,
                    parent_digest,
                    compaction_revision,
                    compaction_fingerprint,
                },
                source_event_sequence,
            )?;
        }
        SessionEvent::AgentDefinitionApplied {
            primary_provider_session,
            binding,
        } => {
            state.ensure_no_unknown_improvement()?;
            require_primary_provider_session(&primary_provider_session)?;
            require_transition(
                state.lifecycle,
                AGENT_DEFINITION_APPLIED_EVENT_TYPE,
                SessionLifecycle::Active,
            )?;
            if let Some(previous) = &state.agent_definition_binding {
                state.ensure_completed_history()?;
                previous
                    .reload_compatible_with(binding.definition())
                    .map_err(|error| SessionError::AgentDefinition {
                        message: error.to_string(),
                    })?;
            } else if binding.definition().research().is_none() {
                return Err(SessionError::AgentDefinition {
                    message: "legacy Sessions require a research compatibility binding".to_owned(),
                });
            }
            record_fresh_primary_provider_session_id(state, &primary_provider_session)?;
            state.capture_recovered_history_start()?;
            state.resume_count = increment("resume count", state.resume_count)?;
            state.primary_provider_session = Some(primary_provider_session);
            state.agent_definition_binding = Some(binding);
            state.recovered_history_item_count = state.history.items().len();
            state.consecutive_rejection = None;
            state.correction_reset_anchor = None;
        }
        SessionEvent::SubcommandCheckpointed { checkpoint } => {
            require_transition(
                state.lifecycle,
                SUBCOMMAND_CHECKPOINT_EVENT_TYPE,
                SessionLifecycle::Active,
            )?;
            checkpoint
                .validate_after(&state.subcommand_checkpoints)
                .map_err(|_| SessionError::SubcommandCheckpointMismatch)?;
            let parent = state
                .history
                .items()
                .iter()
                // Compaction already froze a complete recovery projection for
                // this prefix. A late child cannot enrich its synthetic Result.
                .skip(
                    state
                        .latest_compaction
                        .as_ref()
                        .map_or(0, |compaction| compaction.source_history_item_count()),
                )
                .find_map(|item| match item {
                    ContentItem::ToolCall { call } if call.call_id == checkpoint.call_id => {
                        Some(call)
                    }
                    _ => None,
                })
                .ok_or(SessionError::SubcommandCheckpointMismatch)?;
            if !checkpoint.agrees_with_call(parent)
                || !state
                    .history
                    .unpaired_tool_calls_in_call_order()
                    .iter()
                    .any(|(call_id, _)| call_id == &checkpoint.call_id)
                || state
                    .execution_receipts
                    .iter()
                    .any(|record| record.receipt.execution_id == checkpoint.execution_id)
            {
                return Err(SessionError::SubcommandCheckpointMismatch);
            }
            state.subcommand_checkpoints.push(checkpoint);
        }
        SessionEvent::Started {
            primary_provider_session,
        } => {
            require_primary_provider_session(&primary_provider_session)?;
            require_transition(
                state.lifecycle,
                "session.started",
                SessionLifecycle::Unstarted,
            )?;
            record_fresh_primary_provider_session_id(state, &primary_provider_session)?;
            state.lifecycle = SessionLifecycle::Active;
            state.primary_provider_session = Some(primary_provider_session);
        }
        SessionEvent::Resumed {
            primary_provider_session,
        } => {
            state.consecutive_rejection = None;
            state.correction_reset_anchor = None;
            state.ensure_no_unknown_improvement()?;
            require_primary_provider_session(&primary_provider_session)?;
            require_transition(state.lifecycle, "session.resumed", SessionLifecycle::Active)?;
            record_fresh_primary_provider_session_id(state, &primary_provider_session)?;
            state.capture_recovered_history_start()?;
            state.resume_count = increment("resume count", state.resume_count)?;
            state.primary_provider_session = Some(primary_provider_session);
            state.recovered_history_item_count = state.history.items().len();
        }
        SessionEvent::Ended => {
            require_transition(state.lifecycle, "session.ended", SessionLifecycle::Active)?;
            state.lifecycle = SessionLifecycle::Ended;
        }
        SessionEvent::ExecutionOutcomeRecorded { receipt, result } => {
            require_transition(
                state.lifecycle,
                EXECUTION_OUTCOME_RECORDED_EVENT_TYPE,
                SessionLifecycle::Active,
            )?;
            let scoped_items =
                state
                    .improvement_records
                    .iter()
                    .flat_map(|record| match &record.activity.fact {
                        ImprovementFact::Sample { items, .. }
                        | ImprovementFact::Results { items, .. } => items.as_slice(),
                        _ => &[],
                    });
            let items = state
                .history
                .items()
                .iter()
                .chain(scoped_items)
                .collect::<Vec<_>>();
            let call = items
                .iter()
                .find_map(|item| match item {
                    ContentItem::ToolCall { call } if call.call_id == result.call_id => Some(call),
                    _ => None,
                })
                .ok_or(SessionError::ExecutionOutcomeMismatch)?;
            if call.name != result.tool_name
                || receipt.execution_id != super::tools::ExecutionId::for_model_call(&result.call_id)
                || receipt.status != result.status
                || items.iter().any(|item| matches!(item, ContentItem::ToolResult { result: previous } if previous.call_id == result.call_id))
                || state.subcommand_checkpoints.iter().filter(|child| child.call_id == result.call_id).any(|child| !child.agrees_with_result(&result))
            {
                return Err(SessionError::ExecutionOutcomeMismatch);
            }
            validate_child_receipt(state, &receipt)?;
            receipt
                .validate_output_captures(&result)
                .map_err(|_| SessionError::OutputCaptureMismatch)?;
            if let Some(record) = state
                .execution_receipts
                .iter_mut()
                .find(|record| record.receipt.execution_id == receipt.execution_id)
            {
                if record.receipt != receipt || record.result.is_some() {
                    return Err(SessionError::ExecutionOutcomeMismatch);
                }
                record.result = Some(result);
            } else {
                let fact = if receipt.cache_source.is_some() {
                    SessionEvent::ToolResultCacheHit { receipt }
                } else {
                    SessionEvent::ExecutionReceiptRecorded { receipt }
                };
                apply_session_event(state, fact, source_event_sequence, event_id, occurred_at)?;
                state
                    .execution_receipts
                    .last_mut()
                    .expect("Receipt transition appended its record")
                    .result = Some(result);
            }
        }
        SessionEvent::ExecutionReceiptRecorded { receipt } => {
            validate_child_receipt(state, &receipt)?;
            state.execution_receipt_count =
                increment("execution receipt count", state.execution_receipt_count)?;
            state
                .execution_receipts
                .push(SessionExecutionReceiptRecord {
                    event_id: event_id.clone(),
                    sequence: EventSequence::new(
                        source_event_sequence.map_or(1, |sequence| sequence.get() + 1),
                    ),
                    receipt,
                    result: None,
                });
        }
        SessionEvent::ToolResultCacheHit { receipt } => {
            validate_child_receipt(state, &receipt)?;
            state.cache_hit_count = increment("cache hit count", state.cache_hit_count)?;
            state
                .execution_receipts
                .push(SessionExecutionReceiptRecord {
                    event_id: event_id.clone(),
                    sequence: EventSequence::new(
                        source_event_sequence.map_or(1, |sequence| sequence.get() + 1),
                    ),
                    receipt,
                    result: None,
                });
        }
        SessionEvent::StreamRecovered { .. } => {
            state.recovery_count = increment("recovery count", state.recovery_count)?;
        }
        SessionEvent::HistoryItemRecorded { item } => {
            if let ContentItem::ToolCall { call } = &item {
                state.ensure_no_scope_call_identity_collision(&call.call_id)?;
            }
            require_transition(
                state.lifecycle,
                "session.history_item_recorded",
                SessionLifecycle::Active,
            )?;
            if let ContentItem::ToolResult { result } = &item {
                if state
                    .recorded_tool_result(&result.call_id)
                    .is_some_and(|recorded| recorded != result)
                {
                    return Err(SessionError::ExecutionOutcomeMismatch);
                }
                if state
                    .subcommand_checkpoints
                    .iter()
                    .filter(|child| child.call_id == result.call_id)
                    .any(|child| !child.agrees_with_result(result))
                {
                    return Err(SessionError::SubcommandCheckpointMismatch);
                }
                let execution_id = super::tools::ExecutionId::for_model_call(&result.call_id);
                if let Some(record) = state
                    .execution_receipts
                    .iter()
                    .find(|record| record.receipt.execution_id == execution_id)
                {
                    record
                        .receipt
                        .validate_output_captures(result)
                        .map_err(|_| SessionError::OutputCaptureMismatch)?;
                } else if result.output.get("output_captures").is_some() {
                    return Err(SessionError::OutputCaptureMismatch);
                }
            }
            state.history.append_content_item(item)?;
            state.history_event_ids.push(event_id.clone());
            state.history_event_times.push(occurred_at.clone());
            let index = state.history.items().len() - 1;
            state
                .task_navigation
                .append(index, &state.history.items()[index]);
            state.history_event_sequences.push(EventSequence::new(
                source_event_sequence.map_or(1, |sequence| sequence.get() + 1),
            ));
            if state.latest_compaction.is_some() {
                state.normalized_history_items()?;
            }
        }
        SessionEvent::CompactionCreated { compaction } => {
            state.ensure_no_unknown_improvement()?;
            require_transition(
                state.lifecycle,
                "session.compaction_created",
                SessionLifecycle::Active,
            )?;
            compaction.validate_shape()?;
            if let Some(task) = compaction.summary().task_references()
                && (Some(task.source().stream_id()) != state.stream_id()
                    || task.source().watermark() != compaction.source_event_sequence())
            {
                return Err(SessionError::CompactionContextProvenance {
                    message: "task references must bind this exact Session prefix".to_owned(),
                });
            }
            if Some(compaction.source_event_sequence()) != source_event_sequence {
                return Err(SessionError::CompactionSourceSequenceMismatch {
                    expected: source_event_sequence,
                    received: compaction.source_event_sequence(),
                });
            }
            let normalized_history = state.full_normalized_history()?;
            if compaction.source_history_item_count() != state.history.items().len()
                || compaction.retained_from_normalized_history_index() > normalized_history.len()
                || compaction.retained_history()
                    != &normalized_history[compaction.retained_from_normalized_history_index()..]
                || (matches!(compaction.summary().schema_version(), 6 | 7)
                    && compaction.retained_history().len()
                        < state.latest_uncompleted_batch_items()?)
            {
                return Err(SessionError::CompactionRetentionMismatch);
            }
            if compaction.summary().schema_version() == 7
                && compaction.retained_event_ids()
                    != &state.normalized_history_event_ids()?
                        [compaction.retained_from_normalized_history_index()..]
            {
                return Err(SessionError::CompactionRetentionMismatch);
            }
            let expected_revision = state.latest_compaction.as_ref().map_or(Ok(1), |previous| {
                increment("Compaction revision", previous.summary().revision())
            })?;
            if compaction.summary().revision() != expected_revision {
                return Err(SessionError::CompactionRevisionMismatch {
                    expected: expected_revision,
                    received: compaction.summary().revision(),
                });
            }
            task_context::apply_task_compaction(state, &compaction)?;
            state.latest_compaction = Some(compaction);
            state.recovered_history_start = None;
        }
        SessionEvent::ConsecutiveModelSampleRejected {
            request_audit_event_id,
            corrections_used,
            reset_anchor,
            ..
        } => {
            argument_correction::reject(
                state,
                request_audit_event_id,
                corrections_used,
                reset_anchor,
                None,
            )?;
        }
        SessionEvent::ConsecutiveModelSampleRejectedV3 {
            request_audit_event_id,
            corrections_used,
            reset_anchor,
            sampling_step_id,
            ..
        } => {
            if sampling_step_id.trim().is_empty() {
                return Err(SessionError::InvalidModelSampleRejection);
            }
            argument_correction::reject(
                state,
                request_audit_event_id,
                corrections_used,
                reset_anchor,
                Some(&sampling_step_id),
            )?;
        }
        SessionEvent::ModelArgumentCorrectionReset {
            accepted_request_audit_event_id,
            prior_rejection_audit_event_id,
            accepted_history_event_refs,
        } => {
            argument_correction::reset(
                state,
                event_id,
                accepted_request_audit_event_id,
                prior_rejection_audit_event_id,
                accepted_history_event_refs,
            )?;
        }
        SessionEvent::ModelSampleRejected {
            request_audit_event_id,
            corrections_used,
            ..
        } => {
            require_transition(
                state.lifecycle,
                MODEL_SAMPLE_REJECTED_EVENT_TYPE,
                SessionLifecycle::Active,
            )?;
            state.ensure_completed_history()?;
            if corrections_used > MAX_TOOL_ARGUMENT_CORRECTIONS_PER_TURN
                || state.rejected_requests.contains(&request_audit_event_id)
                || state
                    .request_audits
                    .last()
                    .is_none_or(|audit| audit.audit_event_id() != &request_audit_event_id)
            {
                return Err(SessionError::InvalidModelSampleRejection);
            }
            state.rejected_requests.insert(request_audit_event_id);
        }
        SessionEvent::RequestAuditRecorded { manifest } => {
            if let Some(scope) = manifest.improvement_scope() {
                improvement::validate_audit_scope(state, scope, &manifest)?;
            }
            require_transition(
                state.lifecycle,
                "session.request_audit_recorded",
                SessionLifecycle::Active,
            )?;
            // SessionEvent::decode already validates this manifest at the
            // untrusted payload boundary. Internal receipt expansion cannot
            // produce RequestAuditRecorded events.
            if manifest.audit_event_id() != event_id {
                return Err(SessionError::RequestAuditEventIdentityMismatch {
                    expected: event_id.clone(),
                    received: manifest.audit_event_id().clone(),
                });
            }
            let expected_stream_id = state
                .stream_id()
                .ok_or(SessionError::RequestAuditSessionStreamMissing)?;
            if manifest.binding().session_stream_id() != expected_stream_id {
                return Err(SessionError::RequestAuditSessionStreamMismatch {
                    expected: expected_stream_id.clone(),
                    received: manifest.binding().session_stream_id().clone(),
                });
            }
            state.request_audits.push(manifest);
            state.request_audit_sequences.push(EventSequence::new(
                source_event_sequence.map_or(1, |sequence| sequence.get() + 1),
            ));
            state.latest_audit_history_len = state.history.items().len();
        }
        SessionEvent::VisionRouteDecided { route } => {
            require_transition(
                state.lifecycle,
                VISION_ROUTE_DECIDED_EVENT_TYPE,
                SessionLifecycle::Active,
            )?;
            route.validate().map_err(SessionError::Vision)?;
            if route.route_event_id != *event_id {
                return Err(SessionError::VisionRouteEventIdentityMismatch {
                    expected: event_id.clone(),
                    received: route.route_event_id,
                });
            }
            if state
                .vision_routes
                .iter()
                .any(|existing| existing.route_event_id == *event_id)
            {
                return Err(SessionError::VisionRouteDuplicate {
                    event_id: event_id.clone(),
                });
            }
            state.vision_routes.push(route);
            state.vision_route_sequences.push(EventSequence::new(
                source_event_sequence
                    .and_then(|sequence| sequence.get().checked_add(1))
                    .ok_or(SessionError::StateOverflow {
                        field: "Vision route sequence",
                    })?,
            ));
        }
        SessionEvent::VisionObservationRecorded { observation } => {
            require_transition(
                state.lifecycle,
                VISION_OBSERVATION_RECORDED_EVENT_TYPE,
                SessionLifecycle::Active,
            )?;
            observation.validate().map_err(SessionError::Vision)?;
            require_vision_terminal_references(
                state,
                VisionTerminalReferences {
                    route_event_id: &observation.route_event_id,
                    source_artifact_id: &observation.source_artifact_id,
                    provider_profile: &observation.provider_profile,
                    capability_hash: &observation.capability_hash,
                    provider_session: &observation.provider_session,
                    request_audit_event_id: &observation.request_audit_event_id,
                    request_input_hash: &observation.request_input_hash,
                },
            )?;
            require_unused_vision_audit(state, &observation.request_audit_event_id)?;
            require_unused_vision_session(state, &observation.provider_session)?;
            if state
                .vision_observations
                .iter()
                .any(|existing| existing.id == observation.id)
            {
                return Err(SessionError::VisionObservationDuplicate {
                    observation_id: observation.id,
                });
            }
            state.vision_observations.push(observation);
            state.vision_observation_event_ids.push((
                event_id.clone(),
                EventSequence::new(
                    source_event_sequence
                        .and_then(|sequence| sequence.get().checked_add(1))
                        .ok_or(SessionError::StateOverflow {
                            field: "Vision observation sequence",
                        })?,
                ),
            ));
        }
        SessionEvent::VisionAnalysisFailed { failure } => {
            require_transition(
                state.lifecycle,
                VISION_ANALYSIS_FAILED_EVENT_TYPE,
                SessionLifecycle::Active,
            )?;
            failure.validate().map_err(SessionError::Vision)?;
            require_vision_terminal_references(
                state,
                VisionTerminalReferences {
                    route_event_id: &failure.route_event_id,
                    source_artifact_id: &failure.source_artifact_id,
                    provider_profile: &failure.provider_profile,
                    capability_hash: &failure.capability_hash,
                    provider_session: &failure.provider_session,
                    request_audit_event_id: &failure.request_audit_event_id,
                    request_input_hash: &failure.request_input_hash,
                },
            )?;
            require_unused_vision_audit(state, &failure.request_audit_event_id)?;
            require_unused_vision_session(state, &failure.provider_session)?;
            state.vision_failures.push(failure);
        }
        SessionEvent::ResearchPhaseSelected { selection } => {
            require_research_binding(state)?;
            require_transition(
                state.lifecycle,
                RESEARCH_PHASE_SELECTED_EVENT_TYPE,
                SessionLifecycle::Active,
            )?;
            selection
                .validate()
                .map_err(|error| SessionError::MalformedPayload {
                    event_type: EventType::new(RESEARCH_PHASE_SELECTED_EVENT_TYPE),
                    message: error.to_string(),
                })?;
            state.research_phase_selection = Some(selection);
        }
        SessionEvent::DomainProfileSelected { selection } => {
            require_research_binding(state)?;
            require_transition(
                state.lifecycle,
                "session.domain_profile_selected",
                SessionLifecycle::Active,
            )?;
            selection
                .validate()
                .map_err(|error| SessionError::MalformedPayload {
                    event_type: EventType::new(DOMAIN_PROFILE_SELECTED_EVENT_TYPE),
                    message: error.to_string(),
                })?;
            state.domain_profile_selection = Some(selection);
        }
    }
    Ok(())
}

fn require_research_binding(state: &SessionState) -> Result<(), SessionError> {
    if state
        .agent_definition_binding
        .as_ref()
        .is_some_and(|binding| binding.definition().research().is_none())
    {
        return Err(SessionError::AgentDefinition {
            message: "generic Sessions cannot select research Domain/Phase".to_owned(),
        });
    }
    Ok(())
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ModelUsagePayload {
    schema_version: u16,
    request_audit_event_id: EventId,
    provider_session: ProviderSessionRef,
    usage: ModelUsage,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ModelSampleRejectedPayload {
    schema_version: u16,
    request_audit_event_id: EventId,
    diagnostic: ToolArgumentDiagnostic,
    corrections_used: u8,
}

struct VisionTerminalReferences<'fact> {
    route_event_id: &'fact EventId,
    source_artifact_id: &'fact crate::domain::artifacts::ArtifactId,
    provider_profile: &'fact crate::domain::context::ModelProfile,
    capability_hash: &'fact str,
    provider_session: &'fact crate::domain::context::ProviderSessionRef,
    request_audit_event_id: &'fact EventId,
    request_input_hash: &'fact str,
}

fn require_vision_terminal_references(
    state: &SessionState,
    references: VisionTerminalReferences<'_>,
) -> Result<(), SessionError> {
    if state
        .primary_provider_session
        .as_ref()
        .is_some_and(|primary| primary.id == references.provider_session.id)
    {
        return Err(SessionError::VisionPrimarySessionIdentityCollision {
            event_id: references.request_audit_event_id.clone(),
        });
    }
    let route = state
        .vision_routes
        .iter()
        .find(|route| &route.route_event_id == references.route_event_id)
        .ok_or_else(|| SessionError::VisionRouteReferenceMissing {
            event_id: references.route_event_id.clone(),
        })?;
    if route.route != crate::domain::profile::VisionRoute::SecondaryTool {
        return Err(SessionError::VisionTerminalRequiresSecondaryRoute {
            event_id: references.route_event_id.clone(),
        });
    }
    if !route
        .source_artifact_ids
        .contains(references.source_artifact_id)
    {
        return Err(SessionError::VisionSourceArtifactNotRouted {
            route_event_id: references.route_event_id.clone(),
            artifact_id: references.source_artifact_id.clone(),
        });
    }
    if route.vision_profile.as_ref() != Some(references.provider_profile)
        || route.vision_capability_hash.as_deref() != Some(references.capability_hash)
    {
        return Err(SessionError::VisionRouteProviderIdentityMismatch {
            route_event_id: references.route_event_id.clone(),
        });
    }
    let audit = state
        .request_audits
        .iter()
        .find(|audit| audit.audit_event_id() == references.request_audit_event_id)
        .ok_or_else(|| SessionError::VisionRequestAuditReferenceMissing {
            event_id: references.request_audit_event_id.clone(),
        })?;
    if audit.profile() != references.provider_profile
        || audit.capability_mapping().semantic_hash().as_str() != references.capability_hash
    {
        return Err(SessionError::VisionRequestAuditIdentityMismatch {
            event_id: references.request_audit_event_id.clone(),
        });
    }
    if audit.full_logical_input().semantic_hash().as_str() != references.request_input_hash {
        return Err(SessionError::VisionRequestAuditInputMismatch {
            event_id: references.request_audit_event_id.clone(),
        });
    }
    Ok(())
}

fn require_unused_vision_session(
    state: &SessionState,
    provider_session: &crate::domain::context::ProviderSessionRef,
) -> Result<(), SessionError> {
    let reused = state
        .vision_observations
        .iter()
        .any(|existing| existing.provider_session.id == provider_session.id)
        || state
            .vision_failures
            .iter()
            .any(|existing| existing.provider_session.id == provider_session.id);
    if reused {
        return Err(SessionError::VisionSessionIdentityReused {
            session_id: provider_session.id.clone(),
        });
    }
    Ok(())
}

fn require_unused_vision_audit(
    state: &SessionState,
    request_audit_event_id: &EventId,
) -> Result<(), SessionError> {
    if state
        .vision_observations
        .iter()
        .any(|existing| &existing.request_audit_event_id == request_audit_event_id)
        || state
            .vision_failures
            .iter()
            .any(|existing| &existing.request_audit_event_id == request_audit_event_id)
    {
        return Err(SessionError::VisionRequestAlreadyTerminated {
            event_id: request_audit_event_id.clone(),
        });
    }
    Ok(())
}

fn vision_payload_error(
    event_type: &EventType,
) -> impl FnOnce(VisionFactError) -> SessionError + '_ {
    |error| SessionError::MalformedPayload {
        event_type: event_type.clone(),
        message: error.to_string(),
    }
}

fn increment(field: &'static str, value: u64) -> Result<u64, SessionError> {
    value
        .checked_add(1)
        .ok_or(SessionError::StateOverflow { field })
}

fn require_primary_provider_session(
    provider_session: &ProviderSessionRef,
) -> Result<(), SessionError> {
    if provider_session.role == ProviderSessionRole::Primary {
        Ok(())
    } else {
        Err(SessionError::PrimaryProviderSessionRequired {
            provider_session: provider_session.clone(),
        })
    }
}

fn record_fresh_primary_provider_session_id(
    state: &mut SessionState,
    provider_session: &ProviderSessionRef,
) -> Result<(), SessionError> {
    if state
        .used_primary_provider_session_ids
        .insert(provider_session.id.clone())
    {
        Ok(())
    } else {
        Err(SessionError::ProviderSessionIdReused {
            provider_session_id: provider_session.id.clone(),
        })
    }
}

fn require_transition(
    current: SessionLifecycle,
    event_type: &'static str,
    expected: SessionLifecycle,
) -> Result<(), SessionError> {
    if current == expected {
        Ok(())
    } else {
        Err(SessionError::InvalidTransition {
            event_type,
            current,
        })
    }
}

fn decode_strict_payload<T>(event_type: &EventType, payload: Value) -> Result<T, SessionError>
where
    T: DeserializeOwned + Serialize,
{
    let decoded = serde_json::from_value(payload.clone()).map_err(|error| {
        SessionError::MalformedPayload {
            event_type: event_type.clone(),
            message: error.to_string(),
        }
    })?;
    let canonical =
        serde_json::to_value(&decoded).map_err(|error| SessionError::PayloadEncode {
            event_type: event_type.clone(),
            message: error.to_string(),
        })?;
    if canonical != payload {
        return Err(SessionError::MalformedPayload {
            event_type: event_type.clone(),
            message: "payload contains unsupported or non-canonical fields".to_owned(),
        });
    }
    Ok(decoded)
}

fn require_current_payload_schema(
    event_type: &EventType,
    schema_version: u16,
) -> Result<(), SessionError> {
    if schema_version == CURRENT_SESSION_PAYLOAD_SCHEMA_VERSION {
        Ok(())
    } else {
        Err(SessionError::UnsupportedPayloadSchema {
            event_type: event_type.clone(),
            schema_version,
        })
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SessionLifecyclePayload {
    schema_version: u16,
    primary_provider_session: ProviderSessionRef,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct AgentDefinitionPayload {
    schema_version: u16,
    primary_provider_session: ProviderSessionRef,
    binding: AgentDefinitionBinding,
}

impl SessionLifecyclePayload {
    fn current(primary_provider_session: ProviderSessionRef) -> Self {
        Self {
            schema_version: CURRENT_SESSION_PAYLOAD_SCHEMA_VERSION,
            primary_provider_session,
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct EmptySessionPayload {
    schema_version: u16,
}

impl EmptySessionPayload {
    const fn current() -> Self {
        Self {
            schema_version: CURRENT_SESSION_PAYLOAD_SCHEMA_VERSION,
        }
    }
}

/// This technical recovery payload predates the strict Session vocabulary.
/// It remains explicitly accepted for existing Session JSONL streams.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StreamRecoveredPayload {
    recovery_artifact: String,
    truncated_from_offset: u64,
    resumed_after_sequence: Option<u64>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SessionHistoryItemPayload {
    schema_version: u16,
    item: ContentItem,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ExecutionOutcomePayload {
    schema_version: u16,
    receipt: ExecutionReceipt,
    result: ToolResult,
}

impl SessionHistoryItemPayload {
    fn current(item: ContentItem) -> Self {
        Self {
            schema_version: CURRENT_SESSION_PAYLOAD_SCHEMA_VERSION,
            item,
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ResearchPhaseSelectedPayload {
    schema_version: u16,
    selection: ResearchPhaseSelection,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DomainProfileSelectedPayload {
    schema_version: u16,
    selection: DomainProfileSelection,
}

impl DomainProfileSelectedPayload {
    const fn current(selection: DomainProfileSelection) -> Self {
        Self {
            schema_version: CURRENT_SESSION_PAYLOAD_SCHEMA_VERSION,
            selection,
        }
    }
}

impl ResearchPhaseSelectedPayload {
    const fn current(selection: ResearchPhaseSelection) -> Self {
        Self {
            schema_version: CURRENT_SESSION_PAYLOAD_SCHEMA_VERSION,
            selection,
        }
    }
}

/// Explicit Session schema, stream, payload and transition failures.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum SessionError {
    #[error("task context: {message}")]
    TaskContext { message: String },
    #[error(
        "model usage must reference one accepted request with the same ProviderSession: `{request_audit_event_id}`"
    )]
    InvalidModelUsageReference { request_audit_event_id: EventId },
    #[error(transparent)]
    ModelUsage(#[from] ModelUsageError),
    #[error(
        "recorded execution outcome does not agree with its pending Call, Receipt or later Result"
    )]
    ExecutionOutcomeMismatch,
    #[error(
        "RSI activity violates scoped identity, ordering, pairing or retained-reference constraints"
    )]
    InvalidImprovementActivity,
    #[error("invalid Agent definition binding: {message}")]
    AgentDefinition { message: String },
    #[error(
        "model sample rejection does not reference one unrejected latest request audit or exceeds the correction bound"
    )]
    InvalidModelSampleRejection,
    #[error("subcommand checkpoint does not agree with parent execution or prior child facts")]
    SubcommandCheckpointMismatch,
    #[error("Session Tool output captures lack a matching prior Receipt or disagree with it")]
    OutputCaptureMismatch,
    #[error(transparent)]
    Event(#[from] EventError),
    #[error(transparent)]
    History(#[from] HistoryError),
    #[error(transparent)]
    RequestAudit(#[from] RequestAuditError),
    #[error(transparent)]
    Vision(#[from] VisionFactError),
    #[error("Session Event stream `{stream_id}` must use the `session:` namespace")]
    UnexpectedStream { stream_id: StreamId },
    #[error("Session Event stream changed from `{expected}` to `{received}` during reduction")]
    StreamChanged {
        expected: StreamId,
        received: StreamId,
    },
    #[error("unknown Session Event type `{event_type}`")]
    UnknownEventType { event_type: EventType },
    #[error("Session Event `{event_type}` has malformed payload: {message}")]
    MalformedPayload {
        event_type: EventType,
        message: String,
    },
    #[error(
        "Session Event `{event_type}` uses unsupported payload schema version {schema_version}"
    )]
    UnsupportedPayloadSchema {
        event_type: EventType,
        schema_version: u16,
    },
    #[error("failed to encode Session Event `{event_type}` payload: {message}")]
    PayloadEncode {
        event_type: EventType,
        message: String,
    },
    #[error("Session Event `{event_type}` is invalid while lifecycle is {current:?}")]
    InvalidTransition {
        event_type: &'static str,
        current: SessionLifecycle,
    },
    #[error("Session Event requires a Primary ProviderSession, received {provider_session:?}")]
    PrimaryProviderSessionRequired {
        provider_session: ProviderSessionRef,
    },
    #[error("Session lifecycle must not reuse Primary ProviderSession ID `{provider_session_id}`")]
    ProviderSessionIdReused {
        provider_session_id: ProviderSessionId,
    },
    #[error("Session {field} overflowed while reducing a finite Event stream")]
    StateOverflow { field: &'static str },
    #[error("RequestAudit requires an initialized Session Event stream")]
    RequestAuditSessionStreamMissing,
    #[error("RequestAudit Session stream `{received}` does not match Event stream `{expected}")]
    RequestAuditSessionStreamMismatch {
        expected: StreamId,
        received: StreamId,
    },
    #[error(
        "RequestAudit Manifest event ID `{received}` does not match Event envelope `{expected}"
    )]
    RequestAuditEventIdentityMismatch {
        expected: EventId,
        received: EventId,
    },
    #[error("duplicate RequestAudit Event ID `{event_id}` has conflicting durable content")]
    RequestAuditDuplicateEventConflict { event_id: EventId },
    #[error("Session Compaction Summary fingerprint does not match its structured content")]
    CompactionFingerprintMismatch,
    #[error("Session Compaction Summary field `{field}` must not be empty")]
    CompactionSummaryFieldRequired { field: &'static str },
    #[error("Session Compaction Summary list `{field}` contains an empty entry")]
    CompactionSummaryEntryEmpty { field: &'static str },
    #[error("Session Compaction Summary list `{field}` repeats `{value}`")]
    CompactionSummaryEntryDuplicate { field: &'static str, value: String },
    #[error("Session Compaction Summary is {actual} bytes, above the {maximum}-byte local bound")]
    CompactionSummaryTooLarge { actual: usize, maximum: usize },
    #[error(
        "Session Compaction Summary byte limit {actual} is outside {minimum}..={maximum} bytes"
    )]
    CompactionSummaryByteLimit {
        actual: usize,
        minimum: usize,
        maximum: usize,
    },
    #[error("Session Compaction requires a current TargetSnapshot in the Project Research stream")]
    CompactionCurrentTargetRequired,
    #[error("Session Compaction requires non-empty resumable History with a user goal")]
    CompactionResumableHistoryRequired,
    #[error("Session Compaction requires a non-empty authoritative Research stream")]
    CompactionResearchStreamEmpty,
    #[error("Session Compaction Research stream changed from `{expected}` to `{received}`")]
    CompactionResearchStreamChanged {
        expected: StreamId,
        received: StreamId,
    },
    #[error(
        "Session Compaction Research sequence must be contiguous: expected {expected}, received {received}"
    )]
    CompactionResearchSequenceMismatch { expected: u64, received: u64 },
    #[error("Session Compaction Research stream watermark must be non-zero")]
    CompactionResearchWatermarkRequired,
    #[error("Session Compaction Research stream hash is invalid: {message}")]
    CompactionResearchStreamHash { message: String },
    #[error("Session Compaction Evidence Spine requires at least one TargetSnapshot reference")]
    CompactionEvidenceSpineTargetRequired,
    #[error("Session Compaction Evidence Spine project `{received}` does not match `{expected}`")]
    CompactionEvidenceCrossProject {
        expected: crate::domain::project::ProjectId,
        received: crate::domain::project::ProjectId,
    },
    #[error(
        "Session Compaction Evidence Spine Research stream `{received}` does not match `{expected}`"
    )]
    CompactionEvidenceResearchStreamMismatch {
        expected: StreamId,
        received: StreamId,
    },
    #[error(
        "Session Compaction Evidence Spine Research revision is stale: expected watermark {expected_watermark}, received {received_watermark}"
    )]
    CompactionEvidenceStaleResearchRevision {
        expected_watermark: EventSequence,
        received_watermark: EventSequence,
    },
    #[error("Session Compaction Evidence Spine reference `{id}` has the wrong Research type")]
    CompactionEvidenceTypeMismatch { id: String },
    #[error("Session Compaction Evidence Spine {kind} reference `{id}` is missing")]
    CompactionEvidenceReferenceMissing { kind: &'static str, id: String },
    #[error("Session Compaction Context provenance is invalid: {message}")]
    CompactionContextProvenance { message: String },
    #[error("Session Compaction requires the preceding durable Event sequence")]
    CompactionSourceSequenceMissing,
    #[error(
        "Session Compaction source Event sequence mismatch: expected {expected:?}, received {received}"
    )]
    CompactionSourceSequenceMismatch {
        expected: Option<EventSequence>,
        received: EventSequence,
    },
    #[error("Session Compaction retained History is not the normalized complete History boundary")]
    CompactionRetentionMismatch,
    #[error("Session Compaction revision mismatch: expected {expected}, received {received}")]
    CompactionRevisionMismatch { expected: u64, received: u64 },
    #[error("Vision route event ID `{received}` does not match Session envelope `{expected}`")]
    VisionRouteEventIdentityMismatch {
        expected: EventId,
        received: EventId,
    },
    #[error("Vision route event `{event_id}` is duplicated")]
    VisionRouteDuplicate { event_id: EventId },
    #[error("Vision observation `{observation_id:?}` is duplicated")]
    VisionObservationDuplicate { observation_id: VisionObservationId },
    #[error("Vision route Event `{event_id}` has not been recorded")]
    VisionRouteReferenceMissing { event_id: EventId },
    #[error("Vision RequestAudit Event `{event_id}` has not been recorded")]
    VisionRequestAuditReferenceMissing { event_id: EventId },
    #[error("Vision terminal fact requires secondary route Event `{event_id}`")]
    VisionTerminalRequiresSecondaryRoute { event_id: EventId },
    #[error(
        "Vision source Artifact `{artifact_id}` was not selected by route Event `{route_event_id}`"
    )]
    VisionSourceArtifactNotRouted {
        route_event_id: EventId,
        artifact_id: crate::domain::artifacts::ArtifactId,
    },
    #[error("Vision terminal provider identity does not match route Event `{route_event_id}`")]
    VisionRouteProviderIdentityMismatch { route_event_id: EventId },
    #[error(
        "Vision ProviderSession identity collides with the active Primary Session for RequestAudit Event `{event_id}`"
    )]
    VisionPrimarySessionIdentityCollision { event_id: EventId },
    #[error("Vision terminal provider identity does not match RequestAudit Event `{event_id}`")]
    VisionRequestAuditIdentityMismatch { event_id: EventId },
    #[error("Vision terminal request input does not match RequestAudit Event `{event_id}`")]
    VisionRequestAuditInputMismatch { event_id: EventId },
    #[error(
        "Vision RequestAudit Event `{event_id}` already has an observation or failure terminal fact"
    )]
    VisionRequestAlreadyTerminated { event_id: EventId },
    #[error("Vision scoped ProviderSession `{session_id}` was already used by another request")]
    VisionSessionIdentityReused {
        session_id: crate::domain::context::ProviderSessionId,
    },
}
