//! Validation of Trace references against existing reduced facts.

use std::collections::BTreeSet;

use serde_json::Value;

use crate::domain::events::{EventEnvelope, EventId, EventSequence, StreamId};
use crate::domain::model::{ContentItem, MessageRole};
use crate::domain::research::ResearchState;
use crate::domain::session::{SessionReducer, SessionState};

use super::error::TraceFixtureError;
use super::manifest::TraceFixtureManifest;

impl TraceFixtureManifest {
    /// Computes the stable semantic identity of the complete source Session
    /// stream through the existing canonical JSON owner.
    pub fn source_stream_hash_for(
        events: &[EventEnvelope<Value>],
    ) -> Result<String, TraceFixtureError> {
        crate::domain::canonical_json::canonical_blake3_hash(&events).map_err(|error| {
            TraceFixtureError::SourceStreamCanonicalEncoding {
                message: error.to_string(),
            }
        })
    }

    /// Validates every reference through existing reduced facts. Re-reducing
    /// the source stream proves that the supplied Session projection is not a
    /// second or unrelated authority.
    pub fn validate_against_sources(
        &self,
        source_events: &[EventEnvelope<Value>],
        session: &SessionState,
        research: &ResearchState,
    ) -> Result<(), TraceFixtureError> {
        self.validate_structure()?;
        validate_source_stream(source_events, self.source_session_id())?;
        if session.stream_id() != Some(self.source_session_id()) {
            return Err(TraceFixtureError::SessionIdentityMismatch {
                expected: self.source_session_id().clone(),
                received: session.stream_id().cloned(),
            });
        }
        let reduced_session = SessionReducer
            .reduce(source_events.iter().cloned())
            .map_err(|error| TraceFixtureError::SourceSessionInvalid {
                message: error.to_string(),
            })?;
        if &reduced_session != session {
            return Err(TraceFixtureError::SessionProjectionMismatch);
        }

        let actual_hash = Self::source_stream_hash_for(source_events)?;
        if actual_hash != self.source_stream_hash() {
            return Err(TraceFixtureError::SourceStreamHashMismatch {
                expected: self.source_stream_hash().to_owned(),
                received: actual_hash,
            });
        }
        let range_events = source_events
            .iter()
            .filter(|event| self.event_range().contains(event.sequence))
            .collect::<Vec<_>>();
        let expected_range_len = self.event_range().len()?;
        if range_events.len() != expected_range_len {
            return Err(TraceFixtureError::EventRangeMissing {
                start: self.event_range().start().get(),
                end: self.event_range().end().get(),
                received: range_events.len(),
            });
        }

        let mut referenced_events = BTreeSet::new();
        let mut prior_provider_event: Option<(EventId, EventSequence)> = None;
        for reference in self.provider_event_refs() {
            require_event_in_range(
                &range_events,
                reference.request_audit_event_id(),
                "provider request audit",
            )?;
            if !referenced_events.insert(reference.request_audit_event_id().clone()) {
                return Err(TraceFixtureError::DuplicateEventReference {
                    event_id: reference.request_audit_event_id().clone(),
                });
            }
            if !session
                .request_audits()
                .iter()
                .any(|manifest| manifest.audit_event_id() == reference.request_audit_event_id())
            {
                return Err(TraceFixtureError::ProviderRequestAuditMissing {
                    event_id: reference.request_audit_event_id().clone(),
                });
            }
            let audit_sequence = event_sequence(
                &range_events,
                reference.request_audit_event_id(),
                "provider request audit",
            )?;
            if let Some((prior_event_id, prior_sequence)) = &prior_provider_event
                && audit_sequence <= *prior_sequence
            {
                return Err(TraceFixtureError::EventReferenceOrderInvalid {
                    earlier_event_id: prior_event_id.clone(),
                    later_event_id: reference.request_audit_event_id().clone(),
                    kind: "provider request",
                });
            }
            let mut prior_sample_event =
                (reference.request_audit_event_id().clone(), audit_sequence);
            for history_event_id in reference.history_event_ids() {
                require_event_in_range(&range_events, history_event_id, "provider history")?;
                if !referenced_events.insert(history_event_id.clone()) {
                    return Err(TraceFixtureError::DuplicateEventReference {
                        event_id: history_event_id.clone(),
                    });
                }
                let record = session
                    .history_items_with_event_ids()
                    .find(|(event_id, _)| *event_id == history_event_id)
                    .ok_or_else(|| TraceFixtureError::ProviderHistoryMissing {
                        event_id: history_event_id.clone(),
                    })?;
                if !is_replayable_provider_output(record.1) {
                    return Err(TraceFixtureError::ProviderHistoryNotModelOutput {
                        event_id: history_event_id.clone(),
                    });
                }
                let history_sequence =
                    event_sequence(&range_events, history_event_id, "provider history")?;
                if history_sequence <= prior_sample_event.1 {
                    return Err(TraceFixtureError::EventReferenceOrderInvalid {
                        earlier_event_id: prior_sample_event.0,
                        later_event_id: history_event_id.clone(),
                        kind: "provider output",
                    });
                }
                prior_sample_event = (history_event_id.clone(), history_sequence);
            }
            prior_provider_event = Some(prior_sample_event);
        }

        for receipt_event_id in self.tool_receipt_refs() {
            require_event_in_range(&range_events, receipt_event_id, "tool receipt")?;
            if !referenced_events.insert(receipt_event_id.clone()) {
                return Err(TraceFixtureError::DuplicateEventReference {
                    event_id: receipt_event_id.clone(),
                });
            }
            if !session
                .execution_receipts()
                .iter()
                .any(|record| &record.event_id == receipt_event_id)
            {
                return Err(TraceFixtureError::ToolReceiptMissing {
                    event_id: receipt_event_id.clone(),
                });
            }
        }

        for target_snapshot_id in self.target_snapshot_ids() {
            if research.target_snapshot(target_snapshot_id).is_none() {
                return Err(TraceFixtureError::TargetSnapshotMissing {
                    target_snapshot_id: target_snapshot_id.clone(),
                });
            }
        }
        for artifact_id in self.required_artifact_ids() {
            if research.artifact(artifact_id).is_none() {
                return Err(TraceFixtureError::ArtifactMissing {
                    artifact_id: artifact_id.clone(),
                });
            }
        }
        Ok(())
    }
}

fn validate_source_stream(
    source_events: &[EventEnvelope<Value>],
    source_session_id: &StreamId,
) -> Result<(), TraceFixtureError> {
    if source_events.is_empty() {
        return Err(TraceFixtureError::SourceStreamEmpty);
    }
    let mut expected_sequence = 1_u64;
    for event in source_events {
        if event.stream_id != *source_session_id {
            return Err(TraceFixtureError::SourceStreamMismatch {
                event_id: event.event_id.clone(),
            });
        }
        if event.sequence.get() != expected_sequence {
            return Err(TraceFixtureError::SourceStreamSequenceMismatch {
                expected: expected_sequence,
                received: event.sequence.get(),
            });
        }
        expected_sequence = expected_sequence
            .checked_add(1)
            .ok_or(TraceFixtureError::EventRangeLengthOverflow)?;
    }
    Ok(())
}

fn require_event_in_range(
    range_events: &[&EventEnvelope<Value>],
    event_id: &EventId,
    kind: &'static str,
) -> Result<(), TraceFixtureError> {
    if range_events.iter().any(|event| &event.event_id == event_id) {
        Ok(())
    } else {
        Err(TraceFixtureError::EventReferenceOutsideRange {
            event_id: event_id.clone(),
            kind,
        })
    }
}

fn event_sequence(
    range_events: &[&EventEnvelope<Value>],
    event_id: &EventId,
    kind: &'static str,
) -> Result<EventSequence, TraceFixtureError> {
    range_events
        .iter()
        .find(|event| &event.event_id == event_id)
        .map(|event| event.sequence)
        .ok_or_else(|| TraceFixtureError::EventReferenceOutsideRange {
            event_id: event_id.clone(),
            kind,
        })
}

fn is_replayable_provider_output(item: &ContentItem) -> bool {
    matches!(
        item,
        ContentItem::Text {
            role: MessageRole::Assistant,
            ..
        } | ContentItem::ToolCall { .. }
    )
}
