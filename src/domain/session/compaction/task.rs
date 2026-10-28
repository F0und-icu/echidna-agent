//! Typed task references over an exact durable Session prefix.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::domain::agent_definition::AgentDefinitionBinding;
use crate::domain::artifacts::ArtifactId;
use crate::domain::context::ContextReferences;
use crate::domain::events::{EventEnvelope, EventId, EventSequence, StreamId};
use crate::domain::project::ProjectId;
use crate::domain::research::ExecutionReceiptId;
use crate::domain::session::{SessionError, SessionReducer, SessionState};

use super::{require_text, validate_unique_ids};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionSourceIdentity {
    project_id: ProjectId,
    stream_id: StreamId,
    watermark: EventSequence,
    stream_hash: String,
    project_stream_watermark: u64,
    project_stream_hash: String,
}

impl SessionSourceIdentity {
    pub(crate) fn from_events(
        project_id: ProjectId,
        events: &[EventEnvelope<Value>],
        project_events: &[EventEnvelope<Value>],
    ) -> Result<(Self, SessionState), SessionError> {
        let state = SessionReducer.reduce(events.iter().cloned())?;
        let source = Self::from_reduced_events(project_id, events, project_events, &state)?;
        Ok((source, state))
    }

    pub(in crate::domain::session) fn from_reduced_events(
        project_id: ProjectId,
        events: &[EventEnvelope<Value>],
        project_events: &[EventEnvelope<Value>],
        state: &SessionState,
    ) -> Result<Self, SessionError> {
        let last = events
            .last()
            .ok_or(SessionError::CompactionSourceSequenceMissing)?;
        let stream_id = state
            .stream_id()
            .ok_or(SessionError::CompactionSourceSequenceMissing)?;
        let event_positions = events
            .iter()
            .enumerate()
            .map(|(index, event)| (&event.event_id, index))
            .collect::<BTreeMap<_, _>>();
        let mut session_lengths = BTreeSet::from([events.len()]);
        let mut project_lengths = BTreeSet::from([project_events.len()]);
        for (_, _, sample) in state.compaction_samples() {
            session_lengths.insert(
                events.partition_point(|event| event.sequence <= sample.source().watermark()),
            );
            project_lengths.insert(project_events.partition_point(|event| {
                event.sequence.get() <= sample.source().project_stream_watermark()
            }));
        }
        let encoding_error = |error: serde_json::Error| SessionError::PayloadEncode {
            event_type: crate::domain::events::EventType::new(super::COMPACTION_CREATED_EVENT_TYPE),
            message: error.to_string(),
        };
        let session_hashes =
            crate::domain::canonical_json::canonical_array_prefix_hashes(events, &session_lengths)
                .map_err(encoding_error)?;
        let project_hashes = crate::domain::canonical_json::canonical_array_prefix_hashes(
            project_events,
            &project_lengths,
        )
        .map_err(encoding_error)?;
        for (sample_id, _, sample) in state.compaction_samples() {
            let end = event_positions[sample_id];
            let start =
                events.partition_point(|event| event.sequence <= sample.source().watermark());
            let attempts = events[start..end]
                .iter()
                .map(|event| event.event_id.clone())
                .collect::<Vec<_>>();
            let project_end = project_events.partition_point(|event| {
                event.sequence.get() <= sample.source().project_stream_watermark()
            });
            if sample.source().project_id() != &project_id {
                return Err(invalid_task(
                    "Compaction sample belongs to a different Project",
                ));
            }
            if sample.graph_node().is_some() {
                sample.source().validate_graph_sampling_suffix(
                    &events[..=end],
                    &project_events[..project_end],
                    sample_id,
                )?;
            } else {
                sample.source().validate_sampling_suffix_with_hashes(
                    &events[..=end],
                    &project_events[..project_end],
                    &attempts,
                    Some(sample_id),
                    &session_hashes[&start],
                    &project_hashes[&project_end],
                )?;
            }
        }
        let source = Self {
            project_id,
            stream_id: stream_id.clone(),
            watermark: last.sequence,
            stream_hash: session_hashes[&events.len()].clone(),
            project_stream_watermark: project_events
                .last()
                .map_or(0, |event| event.sequence.get()),
            project_stream_hash: project_hashes[&project_events.len()].clone(),
        };
        Ok(source)
    }

    pub fn project_id(&self) -> &ProjectId {
        &self.project_id
    }
    pub fn stream_id(&self) -> &StreamId {
        &self.stream_id
    }
    pub const fn watermark(&self) -> EventSequence {
        self.watermark
    }
    pub fn stream_hash(&self) -> &str {
        &self.stream_hash
    }
    pub(crate) const fn project_stream_watermark(&self) -> u64 {
        self.project_stream_watermark
    }
    pub(super) fn project_stream_hash(&self) -> &str {
        &self.project_stream_hash
    }

    pub(crate) fn with_current_project(
        mut self,
        events: &[EventEnvelope<Value>],
    ) -> Result<Self, SessionError> {
        self.project_stream_watermark = events.last().map_or(0, |event| event.sequence.get());
        self.project_stream_hash = super::fingerprint_value(&events)?;
        Ok(self)
    }

    pub(super) fn validate(&self) -> Result<(), SessionError> {
        require_text("task.source.project_id", self.project_id.as_str())?;
        require_text("task.source.stream_id", self.stream_id.as_str())?;
        if self.watermark.get() == 0
            || !is_hash(&self.stream_hash)
            || !is_hash(&self.project_stream_hash)
        {
            return Err(invalid_task("invalid task Session prefix identity"));
        }
        Ok(())
    }
}

/// Definition and selected instruction identities at the source watermark.
/// A later explicit reload cannot retroactively change this provenance.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct DefinitionProvenance {
    agent_id: String,
    definition_digest: String,
    personal_digest: Option<String>,
    project_digest: Option<String>,
}

impl DefinitionProvenance {
    fn from_binding(binding: &AgentDefinitionBinding) -> Result<Self, SessionError> {
        binding
            .validate()
            .map_err(|error| invalid_task(error.to_string()))?;
        Ok(Self {
            agent_id: binding.definition().id().as_str().to_owned(),
            definition_digest: binding.definition().digest().as_str().to_owned(),
            personal_digest: binding
                .personal()
                .map(|snapshot| snapshot.digest().as_str().to_owned()),
            project_digest: binding
                .project()
                .map(|snapshot| snapshot.digest().as_str().to_owned()),
        })
    }

    fn validate(&self) -> Result<(), SessionError> {
        require_text("task.definition.agent_id", &self.agent_id)?;
        if !is_hash(&self.definition_digest)
            || self
                .personal_digest
                .iter()
                .chain(self.project_digest.iter())
                .any(|digest| !is_hash(digest))
        {
            return Err(invalid_task("invalid task definition provenance"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionTaskReferences {
    source: SessionSourceIdentity,
    definition: DefinitionProvenance,
    artifact_refs: Vec<ArtifactId>,
    execution_receipt_refs: Vec<ExecutionReceiptId>,
    history_event_refs: Vec<EventId>,
}

impl SessionTaskReferences {
    pub fn source(&self) -> &SessionSourceIdentity {
        &self.source
    }

    pub(super) fn validate_intrinsic(&self) -> Result<(), SessionError> {
        self.source.validate()?;
        self.definition.validate()?;
        validate_unique_ids(
            "task.artifact_refs",
            self.artifact_refs.iter().map(ArtifactId::as_str),
        )?;
        validate_unique_ids(
            "task.execution_receipt_refs",
            self.execution_receipt_refs
                .iter()
                .map(ExecutionReceiptId::as_str),
        )?;
        validate_unique_ids(
            "task.history_event_refs",
            self.history_event_refs.iter().map(EventId::as_str),
        )
    }

    pub(super) fn validate_against(
        &self,
        facts: &TaskCompactionValidationFacts,
    ) -> Result<(), SessionError> {
        self.validate_intrinsic()?;
        if self.source != facts.source {
            return Err(invalid_task(
                "task Summary Session prefix/project identity does not match",
            ));
        }
        if self.definition != facts.definition {
            return Err(invalid_task(
                "task Summary definition provenance does not match its Session prefix",
            ));
        }
        for (kind, missing) in [
            (
                "session_artifact",
                self.artifact_refs
                    .iter()
                    .find(|id| !facts.artifact_ids.contains(id))
                    .map(ArtifactId::as_str),
            ),
            (
                "session_execution_receipt",
                self.execution_receipt_refs
                    .iter()
                    .find(|id| !facts.execution_receipt_ids.contains(id))
                    .map(ExecutionReceiptId::as_str),
            ),
            (
                "session_history_event",
                self.history_event_refs
                    .iter()
                    .find(|id| !facts.history_event_ids.contains(id))
                    .map(EventId::as_str),
            ),
        ] {
            if let Some(id) = missing {
                return Err(SessionError::CompactionEvidenceReferenceMissing {
                    kind,
                    id: id.to_owned(),
                });
            }
        }
        Ok(())
    }

    pub(super) fn context_references(
        &self,
        research: Option<ContextReferences>,
        model_sample: Option<&EventId>,
    ) -> Result<ContextReferences, SessionError> {
        let research = research.unwrap_or_default();
        let artifacts = research
            .artifact_refs()
            .iter()
            .chain(&self.artifact_refs)
            .cloned()
            .collect::<BTreeSet<_>>();
        let receipts = research
            .execution_receipt_refs()
            .iter()
            .chain(&self.execution_receipt_refs)
            .cloned()
            .collect::<BTreeSet<_>>();
        ContextReferences::new(
            research.research_fact_refs().to_vec(),
            artifacts.into_iter().collect(),
            receipts.into_iter().collect(),
            self.history_event_refs
                .iter()
                .chain(model_sample)
                .cloned()
                .collect(),
        )
        .map_err(|error| invalid_task(error.to_string()))
    }
}

/// Ephemeral exact-source validation view, built only from durable records.
#[derive(Clone)]
pub(crate) struct TaskCompactionValidationFacts {
    source: SessionSourceIdentity,
    definition: DefinitionProvenance,
    artifact_ids: BTreeSet<ArtifactId>,
    execution_receipt_ids: Vec<ExecutionReceiptId>,
    history_event_ids: Vec<EventId>,
    pub(super) model_samples: std::collections::BTreeMap<EventId, super::NarrativeSummary>,
    pub(super) verified_graph: Option<super::VerifiedCompactionGraph>,
}

impl TaskCompactionValidationFacts {
    pub(crate) fn source(&self) -> &SessionSourceIdentity {
        &self.source
    }

    pub(crate) fn from_session_source(
        source: SessionSourceIdentity,
        state: &SessionState,
        artifact_ids: BTreeSet<ArtifactId>,
        events: &[EventEnvelope<Value>],
    ) -> Result<Self, SessionError> {
        let mut samples = BTreeMap::new();
        if let Some((id, _, sample)) = state.compaction_samples().last()
            && sample.is_current_at(id, events, &source)?
        {
            samples.insert(id.clone(), sample.narrative().clone());
        }
        Self::from_projection(source, state, artifact_ids, samples)
    }

    /// Current synchronized references for an offline rebuild. A cached view
    /// cannot authorize a newly generated v5 sample.
    pub(crate) fn current_projection(
        source: SessionSourceIdentity,
        state: &SessionState,
        artifact_ids: BTreeSet<ArtifactId>,
    ) -> Result<Self, SessionError> {
        Self::from_projection(source, state, artifact_ids, BTreeMap::new())
    }

    fn from_projection(
        source: SessionSourceIdentity,
        state: &SessionState,
        artifact_ids: BTreeSet<ArtifactId>,
        samples: BTreeMap<EventId, super::NarrativeSummary>,
    ) -> Result<Self, SessionError> {
        let binding = state
            .agent_definition_binding()
            .ok_or_else(|| invalid_task("Summary source has no durable Agent definition"))?;
        Self::new(
            source,
            binding,
            artifact_ids,
            state
                .execution_receipts()
                .iter()
                .map(|record| ExecutionReceiptId::new(record.event_id.as_str()))
                .collect(),
            state
                .history_items_with_event_ids()
                .map(|(id, _)| id.clone())
                .collect(),
            samples,
        )
    }

    pub(crate) fn with_artifacts(mut self, ids: BTreeSet<ArtifactId>) -> Self {
        self.artifact_ids = ids;
        self
    }

    pub(crate) fn with_verified_graph(mut self, graph: super::VerifiedCompactionGraph) -> Self {
        self.verified_graph = Some(graph);
        self
    }

    pub(crate) fn verified_graph(&self) -> Option<&super::VerifiedCompactionGraph> {
        self.verified_graph.as_ref()
    }

    pub(crate) fn new(
        source: SessionSourceIdentity,
        binding: &AgentDefinitionBinding,
        artifact_ids: BTreeSet<ArtifactId>,
        execution_receipt_ids: Vec<ExecutionReceiptId>,
        history_event_ids: Vec<EventId>,
        model_samples: std::collections::BTreeMap<EventId, super::NarrativeSummary>,
    ) -> Result<Self, SessionError> {
        source.validate()?;
        Ok(Self {
            source,
            definition: DefinitionProvenance::from_binding(binding)?,
            artifact_ids,
            execution_receipt_ids,
            history_event_ids,
            model_samples,
            verified_graph: None,
        })
    }

    pub(crate) fn references(&self, limit: usize) -> SessionTaskReferences {
        SessionTaskReferences {
            source: self.source.clone(),
            definition: self.definition.clone(),
            artifact_refs: self.artifact_ids.iter().take(limit).cloned().collect(),
            execution_receipt_refs: self
                .execution_receipt_ids
                .iter()
                .rev()
                .take(limit)
                .cloned()
                .collect(),
            history_event_refs: self
                .history_event_ids
                .iter()
                .rev()
                .take(limit)
                .cloned()
                .collect(),
        }
    }
}

impl SessionReducer {
    pub(crate) fn reduce_with_compaction_source_checked(
        &self,
        project_id: ProjectId,
        events: &[EventEnvelope<Value>],
        project_events: &[EventEnvelope<Value>],
        mut check: impl FnMut(&SessionState, &EventEnvelope<Value>) -> Result<(), SessionError>,
    ) -> Result<
        (
            SessionState,
            Option<TaskCompactionValidationFacts>,
            Option<SessionSourceIdentity>,
        ),
        SessionError,
    > {
        let source = events
            .iter()
            .rev()
            .find(|event| event.event_type.as_str() == super::super::COMPACTION_CREATED_EVENT_TYPE)
            .map(|event| {
                super::super::SessionEvent::decode(&event.event_type, event.payload.clone())
            })
            .transpose()?
            .and_then(|event| match event {
                super::super::SessionEvent::CompactionCreated { compaction } => compaction
                    .summary()
                    .task_references()
                    .map(|refs| refs.source().clone()),
                _ => unreachable!("the selected event has the Compaction event type"),
            });
        if source
            .as_ref()
            .is_some_and(|source| source.project_id() != &project_id)
        {
            return Err(invalid_task(
                "Summary source belongs to a different Project",
            ));
        }
        let mut state = SessionState::default();
        let mut checkpoint = None;
        for (index, event) in events.iter().enumerate() {
            check(&state, event)?;
            self.apply_in_place(&mut state, event.clone())?;
            if let Some(source) = &source
                && event.sequence == source.watermark()
            {
                let end = project_events.partition_point(|event| {
                    event.sequence.get() <= source.project_stream_watermark()
                });
                source.validate_sampling_suffix(
                    &events[..=index],
                    &project_events[..end],
                    &[],
                    None,
                )?;
                checkpoint = Some(TaskCompactionValidationFacts::from_session_source(
                    source.clone(),
                    &state,
                    BTreeSet::new(),
                    &events[..=index],
                )?);
            }
        }
        let current = if events.is_empty() {
            None
        } else {
            Some(SessionSourceIdentity::from_reduced_events(
                project_id,
                events,
                project_events,
                &state,
            )?)
        };
        if source.is_some() && checkpoint.is_none() {
            return Err(invalid_task(
                "Summary source watermark is absent from its strict Session replay",
            ));
        }
        Ok((state, checkpoint, current))
    }
}

pub(super) fn invalid_task(message: impl Into<String>) -> SessionError {
    SessionError::CompactionContextProvenance {
        message: message.into(),
    }
}

fn is_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}
