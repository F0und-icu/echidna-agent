//! One strict replay boundary for the physical project Research stream.
//!
//! Research owns typed record semantics. This module owns the outer stream
//! contract shared by Research and generic consumers: stream identity,
//! recovery-event validation, semantic identity and dispatch ordering.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::domain::canonical_json::canonical_json;
use crate::domain::events::{EventEnvelope, EventId, OccurredAt};
use crate::domain::research::{
    ResearchError, ResearchEvent, ResearchReducer, ResearchReferenceCatalog, ResearchState,
};

pub(crate) const RESEARCH_STREAM_ID: &str = "research";

/// Global Event identity is shared by every consumer of `research.jsonl`.
/// It is deliberately distinct from typed Research records, whose semantics
/// are still reduced by `ResearchReducer`.
#[derive(Debug, Default)]
pub(crate) struct ProjectStreamIntegrity {
    applied_event_digests: BTreeMap<EventId, String>,
}

impl ProjectStreamIntegrity {
    pub(crate) fn applied_event_count(&self) -> usize {
        self.applied_event_digests.len()
    }

    pub(crate) fn has_event_id(&self, event_id: &EventId) -> bool {
        self.applied_event_digests.contains_key(event_id)
    }

    fn is_idempotent(&self, event_id: &EventId, digest: &str) -> Result<bool, ResearchError> {
        match self.applied_event_digests.get(event_id) {
            Some(existing) if existing == digest => Ok(true),
            Some(_) => Err(ResearchError::EventIdConflict {
                event_id: event_id.to_string(),
            }),
            None => Ok(false),
        }
    }

    fn record(&mut self, event_id: EventId, digest: String) {
        self.applied_event_digests.insert(event_id, digest);
    }
}

/// The one outer reducer for `research.jsonl`. It validates the physical
/// stream before delegating one typed event to the existing semantic reducer.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ProjectStreamReducer;

impl ProjectStreamReducer {
    pub(crate) fn reduce<Events>(
        &self,
        events: Events,
        references: &ResearchReferenceCatalog,
    ) -> Result<ResearchState, ResearchError>
    where
        Events: IntoIterator<Item = EventEnvelope<Value>>,
    {
        let mut state = ResearchState::default();
        for event in events {
            self.apply(&mut state, event, references)?;
        }
        Ok(state)
    }

    pub(crate) fn apply(
        &self,
        state: &mut ResearchState,
        envelope: EventEnvelope<Value>,
        references: &ResearchReferenceCatalog,
    ) -> Result<(), ResearchError> {
        if envelope.stream_id.as_str() != RESEARCH_STREAM_ID {
            return Err(ResearchError::UnexpectedStream {
                received: envelope.stream_id.to_string(),
            });
        }
        if envelope.event_type.as_str() == "StreamRecovered" {
            let payload =
                serde_json::to_value(validate_stream_recovered_payload(envelope.payload)?)
                    .map_err(|error| ResearchError::MalformedPayload {
                        message: error.to_string(),
                    })?;
            let digest = semantic_event_digest(
                &envelope.occurred_at,
                envelope.event_type.as_str(),
                &payload,
            )?;
            if state
                .stream_integrity
                .is_idempotent(&envelope.event_id, &digest)?
            {
                return Ok(());
            }
            state.stream_integrity.record(envelope.event_id, digest);
            return Ok(());
        }
        let event = ResearchEvent::decode(envelope.event_type.as_str(), envelope.payload)?;
        event.validate_intrinsic()?;
        validate_created_by_event_id(&event, &envelope.event_id)?;
        let payload = event.encode_semantic_payload()?;
        let digest = semantic_event_digest(
            &envelope.occurred_at,
            envelope.event_type.as_str(),
            &payload,
        )?;
        if state
            .stream_integrity
            .is_idempotent(&envelope.event_id, &digest)?
        {
            return Ok(());
        }
        ResearchReducer.apply_event_with_references(state, event, references)?;
        state.stream_integrity.record(envelope.event_id, digest);
        Ok(())
    }
}

fn validate_created_by_event_id(
    event: &ResearchEvent,
    envelope_event_id: &EventId,
) -> Result<(), ResearchError> {
    let ResearchEvent::Artifact(event) = event else {
        return Ok(());
    };
    if event.record().created_by_event_id == *envelope_event_id {
        Ok(())
    } else {
        Err(ResearchError::InvalidRecord {
            message: format!(
                "Artifact created_by_event_id `{}` does not match Event ID `{envelope_event_id}`",
                event.record().created_by_event_id
            ),
        })
    }
}

fn validate_stream_recovered_payload(
    value: Value,
) -> Result<StreamRecoveredPayload, ResearchError> {
    let payload: StreamRecoveredPayload =
        serde_json::from_value(value).map_err(|error| ResearchError::MalformedPayload {
            message: error.to_string(),
        })?;
    if payload.recovery_artifact.trim().is_empty() {
        return Err(ResearchError::InvalidRecord {
            message: "StreamRecovered recovery_artifact must not be empty".to_owned(),
        });
    }
    Ok(payload)
}

fn semantic_event_digest(
    occurred_at: &OccurredAt,
    event_type: &str,
    payload: &Value,
) -> Result<String, ResearchError> {
    let input = SemanticEventInput {
        occurred_at,
        event_type,
        payload: canonical_json(payload),
    };
    let bytes = serde_json::to_vec(&input).map_err(|error| ResearchError::Fingerprint {
        message: error.to_string(),
    })?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StreamRecoveredPayload {
    recovery_artifact: String,
    truncated_from_offset: u64,
    resumed_after_sequence: Option<u64>,
}

#[derive(Serialize)]
struct SemanticEventInput<'a> {
    occurred_at: &'a OccurredAt,
    event_type: &'a str,
    payload: Value,
}
