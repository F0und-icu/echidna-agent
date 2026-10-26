//! Typed task references over an exact durable Session prefix.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::domain::agent_definition::AgentDefinitionBinding;
use crate::domain::artifacts::ArtifactId;
use crate::domain::context::ContextReferences;
use crate::domain::events::{EventEnvelope, EventId, EventSequence, StreamId};
use crate::domain::project::ProjectId;
use crate::domain::research::ExecutionReceiptId;
use crate::domain::session::{SessionError, SessionReducer, SessionState};

use super::{fingerprint_value, require_text, validate_unique_ids};

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
        let last = events
            .last()
            .ok_or(SessionError::CompactionSourceSequenceMissing)?;
        let stream_id = state
            .stream_id()
            .ok_or(SessionError::CompactionSourceSequenceMissing)?;
        let source = Self {
            project_id,
            stream_id: stream_id.clone(),
            watermark: last.sequence,
            stream_hash: fingerprint_value(&events)?,
            project_stream_watermark: project_events
                .last()
                .map_or(0, |event| event.sequence.get()),
            project_stream_hash: fingerprint_value(&project_events)?,
        };
        Ok((source, state))
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

    fn validate(&self) -> Result<(), SessionError> {
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
            self.history_event_refs.clone(),
        )
        .map_err(|error| invalid_task(error.to_string()))
    }
}

/// Ephemeral exact-source validation view, built only from durable records.
pub(crate) struct TaskCompactionValidationFacts {
    source: SessionSourceIdentity,
    definition: DefinitionProvenance,
    artifact_ids: BTreeSet<ArtifactId>,
    execution_receipt_ids: BTreeSet<ExecutionReceiptId>,
    history_event_ids: BTreeSet<EventId>,
}

impl TaskCompactionValidationFacts {
    pub(crate) fn new(
        source: SessionSourceIdentity,
        binding: &AgentDefinitionBinding,
        artifact_ids: BTreeSet<ArtifactId>,
        execution_receipt_ids: BTreeSet<ExecutionReceiptId>,
        history_event_ids: BTreeSet<EventId>,
    ) -> Result<Self, SessionError> {
        source.validate()?;
        Ok(Self {
            source,
            definition: DefinitionProvenance::from_binding(binding)?,
            artifact_ids,
            execution_receipt_ids,
            history_event_ids,
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
                .take(limit)
                .cloned()
                .collect(),
            history_event_refs: self.history_event_ids.iter().take(limit).cloned().collect(),
        }
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
