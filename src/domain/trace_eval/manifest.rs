//! Versioned, reference-only Trace fixture schema.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::domain::artifacts::ArtifactId;
use crate::domain::context::PromptRendererIdentity;
use crate::domain::events::{EventId, EventSequence, StreamId};
use crate::domain::research::TargetSnapshotId;

use super::EvalSource;
use super::error::TraceFixtureError;

/// The only Trace fixture schema accepted by M4-S2.
pub const TRACE_FIXTURE_MANIFEST_SCHEMA_VERSION: u16 = 2;

/// A versioned Trace fixture containing references, never copies, of source
/// Session, Research, Artifact, Receipt or RequestAudit facts.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TraceFixtureManifest {
    schema_version: u16,
    trace_id: String,
    source_session_id: StreamId,
    event_range: TraceEventRange,
    source_stream_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    prompt_renderer: Option<PromptRendererIdentity>,
    target_snapshot_ids: Vec<TargetSnapshotId>,
    required_artifact_ids: Vec<ArtifactId>,
    provider_event_refs: Vec<TraceProviderEventReference>,
    tool_receipt_refs: Vec<EventId>,
    expected_invariants: Vec<TraceExpectedInvariant>,
}

impl TraceFixtureManifest {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        trace_id: impl Into<String>,
        source_session_id: StreamId,
        event_range: TraceEventRange,
        source_stream_hash: impl Into<String>,
        target_snapshot_ids: Vec<TargetSnapshotId>,
        required_artifact_ids: Vec<ArtifactId>,
        provider_event_refs: Vec<TraceProviderEventReference>,
        tool_receipt_refs: Vec<EventId>,
        expected_invariants: Vec<TraceExpectedInvariant>,
    ) -> Result<Self, TraceFixtureError> {
        let manifest = Self {
            schema_version: TRACE_FIXTURE_MANIFEST_SCHEMA_VERSION,
            trace_id: trace_id.into(),
            source_session_id,
            event_range,
            source_stream_hash: source_stream_hash.into(),
            prompt_renderer: Some(PromptRendererIdentity::generic_v1()),
            target_snapshot_ids,
            required_artifact_ids,
            provider_event_refs,
            tool_receipt_refs,
            expected_invariants,
        };
        manifest.validate_structure()?;
        Ok(manifest)
    }

    pub fn trace_id(&self) -> &str {
        &self.trace_id
    }

    pub fn source_session_id(&self) -> &StreamId {
        &self.source_session_id
    }

    pub const fn event_range(&self) -> TraceEventRange {
        self.event_range
    }

    pub fn source_stream_hash(&self) -> &str {
        &self.source_stream_hash
    }

    pub fn prompt_renderer(&self) -> Option<&PromptRendererIdentity> {
        self.prompt_renderer.as_ref()
    }

    pub fn target_snapshot_ids(&self) -> &[TargetSnapshotId] {
        &self.target_snapshot_ids
    }

    pub fn required_artifact_ids(&self) -> &[ArtifactId] {
        &self.required_artifact_ids
    }

    pub fn provider_event_refs(&self) -> &[TraceProviderEventReference] {
        &self.provider_event_refs
    }

    pub fn tool_receipt_refs(&self) -> &[EventId] {
        &self.tool_receipt_refs
    }

    pub fn expected_invariants(&self) -> &[TraceExpectedInvariant] {
        &self.expected_invariants
    }

    pub(super) fn validate_structure(&self) -> Result<(), TraceFixtureError> {
        if !matches!(
            self.schema_version,
            1 | TRACE_FIXTURE_MANIFEST_SCHEMA_VERSION
        ) {
            return Err(TraceFixtureError::UnsupportedSchemaVersion {
                schema_version: self.schema_version,
            });
        }
        match (self.schema_version, &self.prompt_renderer) {
            (1, None) => {}
            (TRACE_FIXTURE_MANIFEST_SCHEMA_VERSION, Some(renderer)) if renderer.is_generic_v1() => {
            }
            _ => return Err(TraceFixtureError::PromptRendererIdentityMismatch),
        }
        EvalSource::new(
            self.trace_id.clone(),
            self.source_session_id.clone(),
            self.source_stream_hash.clone(),
        )?;
        self.event_range.validate()?;
        if self.provider_event_refs.is_empty() {
            return Err(TraceFixtureError::ProviderEventReferencesRequired);
        }
        if self.expected_invariants.is_empty() {
            return Err(TraceFixtureError::ExpectedInvariantsRequired);
        }

        validate_unique_text(
            "target_snapshot_ids",
            self.target_snapshot_ids
                .iter()
                .map(|identifier| identifier.as_str()),
        )?;
        validate_unique_text(
            "required_artifact_ids",
            self.required_artifact_ids
                .iter()
                .map(|identifier| identifier.as_str()),
        )?;
        validate_unique_event_ids(&self.tool_receipt_refs)?;

        let mut audit_ids = BTreeSet::new();
        let mut history_ids = BTreeSet::new();
        for reference in &self.provider_event_refs {
            reference.validate()?;
            if !audit_ids.insert(reference.request_audit_event_id.clone()) {
                return Err(TraceFixtureError::DuplicateEventReference {
                    event_id: reference.request_audit_event_id.clone(),
                });
            }
            for event_id in &reference.history_event_ids {
                if !history_ids.insert(event_id.clone()) {
                    return Err(TraceFixtureError::DuplicateEventReference {
                        event_id: event_id.clone(),
                    });
                }
            }
        }

        let declared = self
            .expected_invariants
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        if declared.len() != self.expected_invariants.len() {
            return Err(TraceFixtureError::DuplicateExpectedInvariant);
        }
        for invariant in TraceExpectedInvariant::required() {
            if !declared.contains(&invariant) {
                return Err(TraceFixtureError::RequiredExpectedInvariantMissing { invariant });
            }
        }
        Ok(())
    }
}

/// An inclusive source Session sequence range.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TraceEventRange {
    start: EventSequence,
    end: EventSequence,
}

impl TraceEventRange {
    pub fn new(start: EventSequence, end: EventSequence) -> Result<Self, TraceFixtureError> {
        let range = Self { start, end };
        range.validate()?;
        Ok(range)
    }

    pub const fn start(self) -> EventSequence {
        self.start
    }

    pub const fn end(self) -> EventSequence {
        self.end
    }

    pub(crate) fn contains(self, sequence: EventSequence) -> bool {
        self.start <= sequence && sequence <= self.end
    }

    pub(super) fn len(self) -> Result<usize, TraceFixtureError> {
        let count = self
            .end
            .get()
            .checked_sub(self.start.get())
            .and_then(|difference| difference.checked_add(1))
            .ok_or(TraceFixtureError::EventRangeLengthOverflow)?;
        usize::try_from(count).map_err(|_| TraceFixtureError::EventRangeLengthOverflow)
    }

    fn validate(self) -> Result<(), TraceFixtureError> {
        if self.start.get() == 0 || self.end.get() == 0 {
            return Err(TraceFixtureError::EventRangeSequenceZero);
        }
        if self.end < self.start {
            return Err(TraceFixtureError::EventRangeReversed {
                start: self.start.get(),
                end: self.end.get(),
            });
        }
        Ok(())
    }
}

/// One full-request semantic model sample reconstructed from existing Session
/// output History and one existing RequestAudit fact.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TraceProviderEventReference {
    request_audit_event_id: EventId,
    history_event_ids: Vec<EventId>,
}

impl TraceProviderEventReference {
    pub fn new(
        request_audit_event_id: EventId,
        history_event_ids: Vec<EventId>,
    ) -> Result<Self, TraceFixtureError> {
        let reference = Self {
            request_audit_event_id,
            history_event_ids,
        };
        reference.validate()?;
        Ok(reference)
    }

    pub fn request_audit_event_id(&self) -> &EventId {
        &self.request_audit_event_id
    }

    pub fn history_event_ids(&self) -> &[EventId] {
        &self.history_event_ids
    }

    fn validate(&self) -> Result<(), TraceFixtureError> {
        require_text(
            "provider_event_refs.request_audit_event_id",
            self.request_audit_event_id.as_str(),
        )?;
        if self.history_event_ids.is_empty() {
            return Err(TraceFixtureError::ProviderHistoryReferencesRequired {
                request_audit_event_id: self.request_audit_event_id.clone(),
            });
        }
        validate_unique_event_ids(&self.history_event_ids)
    }
}

/// Required guarantees for every M4-S2 fixed Trace. This closed set makes a
/// fixture fail at load time when it omits a regression boundary.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceExpectedInvariant {
    FullRequestSemanticReplay,
    ToolPairing,
    ZeroExternalActions,
    NonEvidence,
}

impl TraceExpectedInvariant {
    pub const fn required() -> [Self; 4] {
        [
            Self::FullRequestSemanticReplay,
            Self::ToolPairing,
            Self::ZeroExternalActions,
            Self::NonEvidence,
        ]
    }
}

fn require_text(field: &'static str, value: &str) -> Result<(), TraceFixtureError> {
    if value.trim().is_empty() {
        return Err(TraceFixtureError::EmptyField { field });
    }
    Ok(())
}

fn validate_unique_text<'a>(
    field: &'static str,
    identifiers: impl IntoIterator<Item = &'a str>,
) -> Result<(), TraceFixtureError> {
    let mut seen = BTreeSet::new();
    for identifier in identifiers {
        if identifier.trim().is_empty() || !seen.insert(identifier) {
            return Err(TraceFixtureError::DuplicateOrEmptyIdentifier {
                field,
                value: identifier.to_owned(),
            });
        }
    }
    Ok(())
}

fn validate_unique_event_ids(event_ids: &[EventId]) -> Result<(), TraceFixtureError> {
    validate_unique_text("event_ids", event_ids.iter().map(EventId::as_str))
}
