//! Strict, versioned Session facts and deterministic reduction.
//!
//! Session facts reuse the shared `cyber-agent.event` envelope. This module
//! owns only the Session payload vocabulary and lifecycle transitions; the
//! generic envelope remains the owner of schema/upcasting, event identity and
//! sequence validation.

use std::collections::BTreeSet;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use super::context::{ProviderSessionId, ProviderSessionRef, ProviderSessionRole};
use super::domain_profile::{DomainProfile, DomainProfileSelection};
use super::events::{
    ApplyOutcome, CurrentEventUpcaster, EventEnvelope, EventError, EventId, EventReducer,
    EventSequence, EventType, EventUpcaster, OccurredAt, State, StreamId,
};
use super::history::{ConversationHistory, HistoryError, normalize_with_subcommands};
use super::model::ContentItem;
use super::model::{MAX_TOOL_ARGUMENT_CORRECTIONS_PER_TURN, ToolArgumentDiagnostic};
use super::request_audit::{RequestAuditError, RequestAuditManifest};
use super::research_control::{ResearchPhase, ResearchPhaseSelection};
use super::tools::SubcommandCheckpoint;
use super::tools::{ExecutionReceipt, ExecutionReceiptEventPayload, ExecutionReceiptPayloadError};
use super::vision::{
    VisionAnalysisFailure, VisionFactError, VisionObservation, VisionObservationId,
    VisionRouteRecord,
};

const SESSION_STREAM_PREFIX: &str = "session:";
const SESSION_STARTED_EVENT_TYPE: &str = "session.started";
const SESSION_RESUMED_EVENT_TYPE: &str = "session.resumed";
const SESSION_ENDED_EVENT_TYPE: &str = "session.ended";
const RECEIPT_RECORDED_EVENT_TYPE: &str = "tool.execution_receipt_recorded";
const SUBCOMMAND_CHECKPOINT_EVENT_TYPE: &str = "tool.subcommand_checkpointed";
const CACHE_HIT_EVENT_TYPE: &str = "tool.result_cache_hit";
const STREAM_RECOVERED_EVENT_TYPE: &str = "StreamRecovered";
const HISTORY_ITEM_RECORDED_EVENT_TYPE: &str = "session.history_item_recorded";
const MODEL_SAMPLE_REJECTED_EVENT_TYPE: &str = "model.sample_rejected";
pub(super) const COMPACTION_CREATED_EVENT_TYPE: &str = "session.compaction_created";
pub(super) const REQUEST_AUDIT_RECORDED_EVENT_TYPE: &str = "session.request_audit_recorded";
const VISION_ROUTE_DECIDED_EVENT_TYPE: &str = "vision.route_decided";
const VISION_OBSERVATION_RECORDED_EVENT_TYPE: &str = "vision.observation_recorded";
const VISION_ANALYSIS_FAILED_EVENT_TYPE: &str = "vision.analysis_failed";
const RESEARCH_PHASE_SELECTED_EVENT_TYPE: &str = "session.research_phase_selected";
const DOMAIN_PROFILE_SELECTED_EVENT_TYPE: &str = "session.domain_profile_selected";
pub(super) const CURRENT_SESSION_PAYLOAD_SCHEMA_VERSION: u16 = 1;

mod compaction;

pub use compaction::{
    CompactionValidationContext, EvidenceSpine, NarrativeSummary, ResearchStreamIdentity,
    SessionCompaction, SessionSummary, SessionSummaryDraft,
};

/// A read-only projection of one existing Receipt fact. It deliberately
/// retains the original Event identity instead of introducing another Receipt
/// schema for replay consumers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SessionExecutionReceiptRecord {
    pub(crate) event_id: EventId,
    pub(crate) receipt: ExecutionReceipt,
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
    ModelSampleRejected {
        request_audit_event_id: EventId,
        diagnostic: ToolArgumentDiagnostic,
        corrections_used: u8,
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
            Self::ModelSampleRejected { .. } => EventType::new(MODEL_SAMPLE_REJECTED_EVENT_TYPE),
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

    fn decode(event_type: &EventType, payload: Value) -> Result<Self, SessionError> {
        match event_type.as_str() {
            MODEL_SAMPLE_REJECTED_EVENT_TYPE => {
                let payload: ModelSampleRejectedPayload =
                    decode_strict_payload(event_type, payload)?;
                require_current_payload_schema(event_type, payload.schema_version)?;
                Ok(Self::ModelSampleRejected {
                    request_audit_event_id: payload.request_audit_event_id,
                    diagnostic: payload.diagnostic,
                    corrections_used: payload.corrections_used,
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
    event_state: State,
    stream_id: Option<StreamId>,
    lifecycle: SessionLifecycle,
    primary_provider_session: Option<ProviderSessionRef>,
    used_primary_provider_session_ids: BTreeSet<ProviderSessionId>,
    history: ConversationHistory,
    history_event_ids: Vec<EventId>,
    resume_count: u64,
    execution_receipt_count: u64,
    execution_receipts: Vec<SessionExecutionReceiptRecord>,
    subcommand_checkpoints: Vec<SubcommandCheckpoint>,
    cache_hit_count: u64,
    recovery_count: u64,
    latest_compaction: Option<SessionCompaction>,
    request_audits: Vec<RequestAuditManifest>,
    last_rejected_request: Option<EventId>,
    vision_routes: Vec<VisionRouteRecord>,
    vision_observations: Vec<VisionObservation>,
    vision_failures: Vec<VisionAnalysisFailure>,
    research_phase_selection: Option<ResearchPhaseSelection>,
    domain_profile_selection: Option<DomainProfileSelection>,
}

impl Default for SessionState {
    fn default() -> Self {
        Self {
            event_state: State::default(),
            stream_id: None,
            lifecycle: SessionLifecycle::Unstarted,
            primary_provider_session: None,
            used_primary_provider_session_ids: BTreeSet::new(),
            history: ConversationHistory::default(),
            history_event_ids: Vec::new(),
            resume_count: 0,
            execution_receipt_count: 0,
            execution_receipts: Vec::new(),
            subcommand_checkpoints: Vec::new(),
            cache_hit_count: 0,
            recovery_count: 0,
            latest_compaction: None,
            request_audits: Vec::new(),
            last_rejected_request: None,
            vision_routes: Vec::new(),
            vision_observations: Vec::new(),
            vision_failures: Vec::new(),
            research_phase_selection: None,
            domain_profile_selection: None,
        }
    }
}

impl SessionState {
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
        normalize_with_subcommands(&items, &self.subcommand_checkpoints)
    }

    pub(crate) fn full_normalized_history(&self) -> Result<Vec<ContentItem>, HistoryError> {
        normalize_with_subcommands(self.history.items(), &self.subcommand_checkpoints)
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
    ) -> impl Iterator<Item = (&EventId, &ContentItem)> {
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
        SessionCompaction::from_history(
            &self.history,
            &self.full_normalized_history()?,
            SessionSummary::from_draft(summary, revision, validation)?,
            source_event_sequence,
            requested_recent_items,
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
            .map_or(ResearchPhase::Mapping, ResearchPhaseSelection::phase)
    }

    /// The latest explicit durable Domain selection. Legacy Sessions remain
    /// read-only until their next real Turn appends the stable source default.
    pub const fn domain_profile_selection(&self) -> Option<DomainProfileSelection> {
        self.domain_profile_selection
    }

    pub fn effective_domain_profile(&self) -> DomainProfile {
        self.domain_profile_selection
            .map_or(DomainProfile::Source, DomainProfileSelection::profile)
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
            self.apply(&mut state, event)?;
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
        let event = EventUpcaster::upcast(&CurrentEventUpcaster, event)?;
        validate_session_stream(state, &event.stream_id)?;
        let session_event = SessionEvent::decode(&event.event_type, event.payload.clone())?;
        let stream_id = event.stream_id.clone();
        let sequence = event.sequence;
        let event_id = event.event_id.clone();

        let mut next = state.clone();
        let outcome = EventReducer.apply(&mut next.event_state, event)?;
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
        require_next_sequence(state, &stream_id, sequence)?;
        next.stream_id = Some(stream_id);
        let source_event_sequence = state
            .stream_id()
            .and_then(|stream_id| state.event_state.last_sequence_for(stream_id));
        apply_session_event(&mut next, session_event, source_event_sequence, &event_id)?;
        *state = next;
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
    state: &SessionState,
    stream_id: &StreamId,
    received: EventSequence,
) -> Result<(), SessionError> {
    let expected = match state.event_state.last_sequence_for(stream_id) {
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
) -> Result<(), SessionError> {
    match event {
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
            require_primary_provider_session(&primary_provider_session)?;
            require_transition(state.lifecycle, "session.resumed", SessionLifecycle::Active)?;
            record_fresh_primary_provider_session_id(state, &primary_provider_session)?;
            state.resume_count = increment("resume count", state.resume_count)?;
            state.primary_provider_session = Some(primary_provider_session);
        }
        SessionEvent::Ended => {
            require_transition(state.lifecycle, "session.ended", SessionLifecycle::Active)?;
            state.lifecycle = SessionLifecycle::Ended;
        }
        SessionEvent::ExecutionReceiptRecorded { receipt } => {
            validate_child_receipt(state, &receipt)?;
            state.execution_receipt_count =
                increment("execution receipt count", state.execution_receipt_count)?;
            state
                .execution_receipts
                .push(SessionExecutionReceiptRecord {
                    event_id: event_id.clone(),
                    receipt,
                });
        }
        SessionEvent::ToolResultCacheHit { receipt } => {
            validate_child_receipt(state, &receipt)?;
            state.cache_hit_count = increment("cache hit count", state.cache_hit_count)?;
            state
                .execution_receipts
                .push(SessionExecutionReceiptRecord {
                    event_id: event_id.clone(),
                    receipt,
                });
        }
        SessionEvent::StreamRecovered { .. } => {
            state.recovery_count = increment("recovery count", state.recovery_count)?;
        }
        SessionEvent::HistoryItemRecorded { item } => {
            require_transition(
                state.lifecycle,
                "session.history_item_recorded",
                SessionLifecycle::Active,
            )?;
            if let ContentItem::ToolResult { result } = &item {
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
            if state.latest_compaction.is_some() {
                state.normalized_history_items()?;
            }
        }
        SessionEvent::CompactionCreated { compaction } => {
            require_transition(
                state.lifecycle,
                "session.compaction_created",
                SessionLifecycle::Active,
            )?;
            compaction.validate_shape()?;
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
            state.latest_compaction = Some(compaction);
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
            state.history.ensure_all_tool_calls_paired()?;
            if corrections_used > MAX_TOOL_ARGUMENT_CORRECTIONS_PER_TURN
                || state.last_rejected_request.as_ref() == Some(&request_audit_event_id)
                || state
                    .request_audits
                    .last()
                    .is_none_or(|audit| audit.audit_event_id() != &request_audit_event_id)
            {
                return Err(SessionError::InvalidModelSampleRejection);
            }
            state.last_rejected_request = Some(request_audit_event_id);
        }
        SessionEvent::RequestAuditRecorded { manifest } => {
            require_transition(
                state.lifecycle,
                "session.request_audit_recorded",
                SessionLifecycle::Active,
            )?;
            manifest.validate()?;
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
