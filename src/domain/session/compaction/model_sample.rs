//! Accepted, isolated model handoff. It is derived text, never task evidence.

use serde::{Deserialize, Serialize};

use crate::domain::context::{ProviderSessionId, ProviderSessionRef, ProviderSessionRole};
use crate::domain::events::{EventEnvelope, EventId};
use crate::domain::session::SessionEvent;
use serde_json::Value;

use super::{NarrativeSummary, SessionError, SessionSourceIdentity};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CompactionModelSample {
    schema_version: u16,
    source: SessionSourceIdentity,
    request_audit_event_id: EventId,
    provider_session: ProviderSessionRef,
    narrative: NarrativeSummary,
    byte_limit: usize,
}

impl CompactionModelSample {
    pub(crate) fn new(
        source: SessionSourceIdentity,
        request_audit_event_id: EventId,
        narrative: NarrativeSummary,
        byte_limit: usize,
    ) -> Result<Self, SessionError> {
        let sample = Self {
            schema_version: 1,
            provider_session: source.compaction_provider_session(),
            source,
            request_audit_event_id,
            narrative,
            byte_limit,
        };
        sample.validate()?;
        Ok(sample)
    }

    pub(crate) fn validate(&self) -> Result<(), SessionError> {
        self.source.validate()?;
        super::require_text(
            "compaction_sample.audit",
            self.request_audit_event_id.as_str(),
        )?;
        super::summary::validate_byte_limit(self.byte_limit)?;
        self.narrative.validate()?;
        if self.schema_version != 1
            || self.provider_session != self.source.compaction_provider_session()
            || serde_json::to_vec(&self.narrative)
                .map_err(|error| super::task::invalid_task(error.to_string()))?
                .len()
                > self.byte_limit / 2
        {
            return Err(super::task::invalid_task(
                "invalid bounded Compaction model sample",
            ));
        }
        Ok(())
    }

    pub(crate) fn source(&self) -> &SessionSourceIdentity {
        &self.source
    }
    pub(crate) fn audit_event_id(&self) -> &EventId {
        &self.request_audit_event_id
    }
    pub(crate) fn provider_session(&self) -> &ProviderSessionRef {
        &self.provider_session
    }
    pub(crate) fn narrative(&self) -> &NarrativeSummary {
        &self.narrative
    }

    /// Historical samples stay valid facts, but only the immediate sample/usage
    /// tail can authorize a new replacement of the current source.
    pub(crate) fn is_current_at(
        &self,
        sample_id: &EventId,
        events: &[EventEnvelope<Value>],
        current: &SessionSourceIdentity,
    ) -> Result<bool, SessionError> {
        if self.source.project_stream_watermark() != current.project_stream_watermark()
            || self.source.project_stream_hash() != current.project_stream_hash()
        {
            return Ok(false);
        }
        let last = events
            .last()
            .expect("a validated sample has its durable source events");
        if &last.event_id == sample_id {
            return Ok(true);
        }
        Ok(events
            .iter()
            .rev()
            .nth(1)
            .is_some_and(|event| &event.event_id == sample_id)
            && matches!(SessionEvent::decode(&last.event_type, last.payload.clone())?,
                SessionEvent::ModelUsageRecorded { request_audit_event_id, provider_session, .. }
                if &request_audit_event_id == self.audit_event_id() && &provider_session == self.provider_session()))
    }
}

impl SessionSourceIdentity {
    pub(crate) fn validate_project_prefix(
        &self,
        events: &[EventEnvelope<Value>],
    ) -> Result<(), SessionError> {
        self.validate_project_prefix_with_hash(events, &super::fingerprint_value(&events)?)
    }

    fn validate_project_prefix_with_hash(
        &self,
        events: &[EventEnvelope<Value>],
        hash: &str,
    ) -> Result<(), SessionError> {
        if events.last().map_or(0, |event| event.sequence.get()) != self.project_stream_watermark()
            || hash != self.project_stream_hash()
        {
            return Err(super::task::invalid_task(
                "Compaction source prefix changed during model sampling",
            ));
        }
        Ok(())
    }

    pub(crate) fn compaction_provider_session(&self) -> ProviderSessionRef {
        ProviderSessionRef {
            id: ProviderSessionId::new(format!(
                "compaction:{}:{}",
                self.stream_id(),
                self.watermark().get()
            )),
            role: ProviderSessionRole::Primary,
            generation: 0,
        }
    }

    pub(crate) fn validate_sampling_suffix(
        &self,
        events: &[EventEnvelope<Value>],
        project_events: &[EventEnvelope<Value>],
        attempts: &[EventId],
        sample_event: Option<&EventId>,
    ) -> Result<(), SessionError> {
        let boundary = events.partition_point(|event| event.sequence <= self.watermark());
        self.validate_sampling_suffix_with_hashes(
            events,
            project_events,
            attempts,
            sample_event,
            &super::fingerprint_value(&&events[..boundary])?,
            &super::fingerprint_value(&project_events)?,
        )
    }

    pub(super) fn validate_sampling_suffix_with_hashes(
        &self,
        events: &[EventEnvelope<Value>],
        project_events: &[EventEnvelope<Value>],
        attempts: &[EventId],
        sample_event: Option<&EventId>,
        prefix_hash: &str,
        project_hash: &str,
    ) -> Result<(), SessionError> {
        self.validate()?;
        let boundary = events.partition_point(|event| event.sequence <= self.watermark());
        self.validate_project_prefix_with_hash(project_events, project_hash)?;
        if events[..boundary].last().is_none_or(|event| {
            event.sequence != self.watermark() || &event.stream_id != self.stream_id()
        }) || prefix_hash != self.stream_hash()
        {
            return Err(super::task::invalid_task(
                "Compaction source prefix changed during model sampling",
            ));
        }
        let mut seen = Vec::new();
        let mut sample_seen = false;
        let mut usage_seen = false;
        for event in &events[boundary..] {
            match SessionEvent::decode(&event.event_type, event.payload.clone())? {
                SessionEvent::RequestAuditRecorded { manifest }
                    if !sample_seen
                        && attempts.get(seen.len()) == Some(&event.event_id)
                        && manifest.provider_session()
                            == Some(&self.compaction_provider_session()) =>
                {
                    seen.push(event.event_id.clone())
                }
                SessionEvent::CompactionSampleRecorded { sample }
                    if !sample_seen
                        && sample_event == Some(&event.event_id)
                        && sample.source() == self
                        && Some(sample.audit_event_id()) == attempts.last() =>
                {
                    sample_seen = true
                }
                SessionEvent::ModelUsageRecorded {
                    request_audit_event_id,
                    provider_session,
                    ..
                } if sample_seen
                    && !usage_seen
                    && Some(&request_audit_event_id) == attempts.last()
                    && provider_session == self.compaction_provider_session() =>
                {
                    usage_seen = true
                }
                _ => {
                    return Err(super::task::invalid_task(
                        "Unrelated Session change during Compaction model sampling; replacement refused",
                    ));
                }
            }
        }
        if seen != attempts || sample_seen != sample_event.is_some() {
            return Err(super::task::invalid_task(
                "Compaction sample-owned suffix does not match its recorded attempts",
            ));
        }
        Ok(())
    }
}
