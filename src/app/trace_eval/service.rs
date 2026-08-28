//! Read-only Trace preparation and Context recompilation.

use std::collections::BTreeSet;

use sha2::{Digest, Sha256};

use crate::domain::artifacts::BlobId;
use crate::domain::events::{EventEnvelope, EventId};
use crate::domain::model::{ContentItem, MessageRole, ModelEvent};
use crate::domain::request_audit::RequestAuditDeliveryMode;
use crate::domain::research::ResearchState;
use crate::domain::session::SessionState;
use crate::domain::tools::{ExecutionId, ToolCall};
use crate::domain::trace_eval::{
    EvalResult, EvalSource, TraceFixtureError, TraceFixtureManifest, TraceReplayModelStep,
    TraceReplayPlan, TraceReplayToolStep,
};
use crate::ports::artifact::ArtifactStore;

use super::error::TraceEvalServiceError;
use super::results::{ContextRecompileInput, ContextRecompileResult};

/// The only App service for isolated M4-S2 evaluation. Its sole dependency is
/// the existing immutable Blob reader needed for RequestAudit reconstruction.
pub struct TraceEvalService<'store, Store: ?Sized> {
    blobs: &'store Store,
}

impl<'store, Store> TraceEvalService<'store, Store>
where
    Store: ArtifactStore + ?Sized,
{
    pub fn new(blobs: &'store Store) -> Self {
        Self { blobs }
    }

    /// Resolves a strictly validated, full-request-only Trace into fixed local
    /// adapters. It reads existing Blob bytes only; no Event or fact is
    /// written and no real Tool/Provider is reachable through the result.
    pub fn prepare_mechanical_replay(
        &self,
        fixture: &TraceFixtureManifest,
        source_events: &[EventEnvelope<serde_json::Value>],
        session: &SessionState,
        research: &ResearchState,
    ) -> Result<TraceReplayPlan, TraceEvalServiceError<Store::Error>> {
        self.validate_trace_sources(fixture, source_events, session, research)?;
        let mut expected_tool_specs = None;
        let mut model_steps = Vec::new();
        let mut source_calls = Vec::new();

        for reference in fixture.provider_event_refs() {
            let audit = session
                .request_audits()
                .iter()
                .find(|audit| audit.audit_event_id() == reference.request_audit_event_id())
                .expect("validated Trace Provider reference has a RequestAudit");
            if audit.delivery().mode() != RequestAuditDeliveryMode::Full {
                return Err(TraceEvalServiceError::IncrementalRequestUnsupported {
                    audit_event_id: audit.audit_event_id().clone(),
                });
            }
            let input = audit
                .reconstruct(|blob_id| self.blobs.read_blob(blob_id))
                .map_err(TraceEvalServiceError::RequestAuditReconstruction)?;
            match &expected_tool_specs {
                Some(specs) if specs != &input.tool_specs => {
                    return Err(TraceEvalServiceError::ToolSchemasChanged);
                }
                Some(_) => {}
                None => expected_tool_specs = Some(input.tool_specs.clone()),
            }

            let events = reference
                .history_event_ids()
                .iter()
                .map(|event_id| {
                    let record = session
                        .history_items_with_event_ids()
                        .find(|(history_event_id, _)| *history_event_id == event_id)
                        .expect("validated Trace Provider reference has a History item");
                    model_event_from_history(record.1, event_id)
                        .map_err(TraceEvalServiceError::Trace)
                        .inspect(|event| {
                            if let ModelEvent::ToolCall { call } = event {
                                source_calls.push((call.clone(), event_id.clone()));
                            }
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;
            model_steps.push(TraceReplayModelStep::new(input, events)?);
        }

        let tool_steps = self.replay_tool_steps(fixture, source_events, session, &source_calls)?;
        Ok(TraceReplayPlan::new(
            EvalSource::from_validated_manifest(fixture),
            model_steps,
            expected_tool_specs.unwrap_or_default(),
            tool_steps,
        ))
    }

    /// Reconstructs historical delivered input through the existing
    /// `RequestAuditManifest` byte path, then compiles a caller-supplied
    /// current candidate without touching any historic fact.
    pub fn context_recompile(
        &self,
        fixture: &TraceFixtureManifest,
        source_events: &[EventEnvelope<serde_json::Value>],
        session: &SessionState,
        research: &ResearchState,
        input: ContextRecompileInput<'_>,
    ) -> Result<ContextRecompileResult, TraceEvalServiceError<Store::Error>> {
        self.validate_trace_sources(fixture, source_events, session, research)?;
        if !fixture
            .provider_event_refs()
            .iter()
            .any(|reference| reference.request_audit_event_id() == input.request_audit_event_id)
        {
            return Err(TraceEvalServiceError::ContextRequestAuditNotReferenced {
                audit_event_id: input.request_audit_event_id.clone(),
            });
        }
        let manifest = session
            .request_audits()
            .iter()
            .find(|manifest| manifest.audit_event_id() == input.request_audit_event_id)
            .expect("validated Trace RequestAudit reference is present in Session state");
        if manifest.delivery().mode() != RequestAuditDeliveryMode::Full {
            return Err(TraceEvalServiceError::IncrementalRequestUnsupported {
                audit_event_id: manifest.audit_event_id().clone(),
            });
        }
        let historical_delivered_input = manifest
            .reconstruct(|blob_id| self.blobs.read_blob(blob_id))
            .map_err(TraceEvalServiceError::RequestAuditReconstruction)?;
        let candidate = input.planner.compile(
            input.fragments,
            input.budget,
            input.capabilities,
            input.tool_specs,
            input.requirements,
            input.prior_manifest,
        )?;
        let candidate_matches_historical_delivery =
            candidate.model_input() == historical_delivered_input;
        Ok(ContextRecompileResult::new(
            EvalResult::context_recompile(
                EvalSource::from_validated_manifest(fixture),
                manifest.audit_event_id().clone(),
            ),
            historical_delivered_input,
            candidate,
            candidate_matches_historical_delivery,
        ))
    }

    fn validate_trace_sources(
        &self,
        fixture: &TraceFixtureManifest,
        source_events: &[EventEnvelope<serde_json::Value>],
        session: &SessionState,
        research: &ResearchState,
    ) -> Result<(), TraceEvalServiceError<Store::Error>> {
        fixture
            .validate_against_sources(source_events, session, research)
            .map_err(TraceEvalServiceError::Trace)?;
        self.validate_required_artifacts(fixture, research)
    }

    /// Ensures a Trace-required Artifact remains backed by the immutable Blob
    /// named by its existing Research fact. The Research reducer owns fact
    /// existence; this App service owns the only required Blob read.
    fn validate_required_artifacts(
        &self,
        fixture: &TraceFixtureManifest,
        research: &ResearchState,
    ) -> Result<(), TraceEvalServiceError<Store::Error>> {
        for artifact_id in fixture.required_artifact_ids() {
            let artifact = research
                .artifact(artifact_id)
                .expect("validated Trace Artifact reference has a Research fact");
            let expected_blob_id = artifact.blob_id.clone();
            let bytes = self.blobs.read_blob(&expected_blob_id).map_err(|_| {
                TraceEvalServiceError::RequiredArtifactBlobUnreadable {
                    artifact_id: artifact_id.clone(),
                    blob_id: expected_blob_id.clone(),
                }
            })?;
            let received_size_bytes = u64::try_from(bytes.len()).map_err(|_| {
                TraceEvalServiceError::RequiredArtifactBlobSizeOverflow {
                    artifact_id: artifact_id.clone(),
                    blob_id: expected_blob_id.clone(),
                }
            })?;
            if received_size_bytes != artifact.size_bytes {
                return Err(TraceEvalServiceError::RequiredArtifactBlobSizeMismatch {
                    artifact_id: artifact_id.clone(),
                    blob_id: expected_blob_id,
                    expected_size_bytes: artifact.size_bytes,
                    received_size_bytes,
                });
            }
            let received_blob_id = BlobId::new(format!("{:x}", Sha256::digest(&bytes)))
                .expect("SHA-256 digest is always a valid BlobId");
            if received_blob_id != artifact.blob_id {
                return Err(TraceEvalServiceError::RequiredArtifactBlobHashMismatch {
                    artifact_id: artifact_id.clone(),
                    expected_blob_id: artifact.blob_id.clone(),
                    received_blob_id,
                });
            }
        }
        Ok(())
    }

    fn replay_tool_steps(
        &self,
        fixture: &TraceFixtureManifest,
        source_events: &[EventEnvelope<serde_json::Value>],
        session: &SessionState,
        source_calls: &[(ToolCall, EventId)],
    ) -> Result<Vec<TraceReplayToolStep>, TraceEvalServiceError<Store::Error>> {
        let range_event_sequences = source_events
            .iter()
            .filter(|event| fixture.event_range().contains(event.sequence))
            .map(|event| (event.event_id.clone(), event.sequence))
            .collect::<std::collections::BTreeMap<_, _>>();
        let receipts = fixture
            .tool_receipt_refs()
            .iter()
            .map(|event_id| {
                session
                    .execution_receipts()
                    .iter()
                    .find(|record| &record.event_id == event_id)
                    .expect("validated Trace Receipt reference is available")
            })
            .collect::<Vec<_>>();
        let mut used_receipt_events = BTreeSet::new();
        let mut steps = Vec::new();
        for (call_index, (call, call_event_id)) in source_calls.iter().enumerate() {
            // Session reduction already rejects duplicate Tool Call IDs and
            // duplicate/mismatched Results, so this pair lookup has exactly
            // one possible result even when a Trace contains several calls.
            let result_record = session
                .history_items_with_event_ids()
                .find(|(_, item)| match item {
                    ContentItem::ToolResult { result }
                        if result.call_id == call.call_id && result.tool_name == call.name =>
                    {
                        true
                    }
                    ContentItem::Text { .. }
                    | ContentItem::ImageArtifact { .. }
                    | ContentItem::ToolCall { .. }
                    | ContentItem::ToolResult { .. }
                    | ContentItem::Unsupported { .. } => false,
                })
                .ok_or_else(|| TraceEvalServiceError::ToolResultMissing {
                    call_id: call.call_id.as_str().to_owned(),
                })?;
            let Some(result_sequence) = range_event_sequences.get(result_record.0) else {
                return Err(TraceEvalServiceError::ToolResultOutsideRange {
                    call_id: call.call_id.as_str().to_owned(),
                    event_id: result_record.0.clone(),
                });
            };
            let ContentItem::ToolResult { result } = result_record.1 else {
                unreachable!("the Tool Result filter accepts only Tool Result history items");
            };
            let expected_execution_id = ExecutionId::new(format!("execution:{}", call.call_id));
            let receipt = receipts
                .iter()
                .find(|record| record.receipt.execution_id == expected_execution_id)
                .ok_or_else(|| TraceEvalServiceError::ToolReceiptForCallMissing {
                    call_id: call.call_id.as_str().to_owned(),
                })?;
            let declared_receipt_event_id = fixture
                .tool_receipt_refs()
                .get(call_index)
                .ok_or(TraceEvalServiceError::UnconsumedToolReceiptReferences)?;
            if &receipt.event_id != declared_receipt_event_id {
                return Err(TraceEvalServiceError::ToolReceiptReferenceOrderInvalid {
                    call_event_id: call_event_id.clone(),
                    expected_receipt_event_id: receipt.event_id.clone(),
                    declared_receipt_event_id: declared_receipt_event_id.clone(),
                });
            }
            let call_sequence = range_event_sequences
                .get(call_event_id)
                .expect("validated Provider History reference is in the Trace range");
            let receipt_sequence = range_event_sequences
                .get(&receipt.event_id)
                .expect("validated Receipt reference is in the Trace range");
            if !(call_sequence < receipt_sequence && receipt_sequence < result_sequence) {
                return Err(TraceEvalServiceError::ToolFactOrderInvalid {
                    call_event_id: call_event_id.clone(),
                    receipt_event_id: receipt.event_id.clone(),
                    result_event_id: result_record.0.clone(),
                });
            }
            used_receipt_events.insert(receipt.event_id.clone());
            steps.push(TraceReplayToolStep::new(
                receipt.event_id.clone(),
                receipt.receipt.clone(),
                call.clone(),
                result.clone(),
            )?);
        }
        if used_receipt_events.len() != fixture.tool_receipt_refs().len() {
            return Err(TraceEvalServiceError::UnconsumedToolReceiptReferences);
        }
        Ok(steps)
    }
}

fn model_event_from_history(
    item: &ContentItem,
    event_id: &EventId,
) -> Result<ModelEvent, TraceFixtureError> {
    match item {
        ContentItem::Text {
            role: MessageRole::Assistant,
            text,
        } => Ok(ModelEvent::TextDelta { text: text.clone() }),
        ContentItem::ToolCall { call } => Ok(ModelEvent::ToolCall { call: call.clone() }),
        ContentItem::Text { .. }
        | ContentItem::ImageArtifact { .. }
        | ContentItem::ToolResult { .. }
        | ContentItem::Unsupported { .. } => {
            Err(TraceFixtureError::ProviderHistoryNotModelOutput {
                event_id: event_id.clone(),
            })
        }
    }
}
