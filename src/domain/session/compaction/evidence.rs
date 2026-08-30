//! Evidence-Spine references and their authoritative validation view.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::domain::artifacts::ArtifactId;
use crate::domain::canonical_json::canonical_blake3_hash;
use crate::domain::context::ContextReferences;
use crate::domain::events::{EventEnvelope, EventSequence, StreamId};
use crate::domain::project::ProjectId;
use crate::domain::research::{
    ExecutionReceiptId, ExperimentId, FindingId, HypothesisId, NextActionId, ResearchFactRef,
    ResearchState, TargetSnapshotId,
};

use super::{require_text, validate_unique_ids};
use crate::domain::session::SessionError;

/// The exact authoritative Research stream revision referenced by one
/// Evidence Spine. The hash covers complete canonical Event envelopes through
/// the watermark; original JSONL remains unchanged.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchStreamIdentity {
    project_id: ProjectId,
    stream_id: StreamId,
    watermark: EventSequence,
    stream_hash: String,
}

impl ResearchStreamIdentity {
    pub fn from_events(
        project_id: ProjectId,
        events: &[EventEnvelope<Value>],
    ) -> Result<Self, SessionError> {
        let first = events
            .first()
            .ok_or(SessionError::CompactionResearchStreamEmpty)?;
        let expected_stream = StreamId::new("research");
        if first.stream_id != expected_stream {
            return Err(SessionError::CompactionResearchStreamChanged {
                expected: expected_stream,
                received: first.stream_id.clone(),
            });
        }
        if first.sequence.get() != 1 {
            return Err(SessionError::CompactionResearchSequenceMismatch {
                expected: 1,
                received: first.sequence.get(),
            });
        }
        let stream_id = first.stream_id.clone();
        let mut previous: Option<u64> = None;
        for event in events {
            if event.stream_id != stream_id {
                return Err(SessionError::CompactionResearchStreamChanged {
                    expected: stream_id,
                    received: event.stream_id.clone(),
                });
            }
            if let Some(previous) = previous {
                let expected = previous.checked_add(1).ok_or(SessionError::StateOverflow {
                    field: "Research stream watermark",
                })?;
                if event.sequence.get() != expected {
                    return Err(SessionError::CompactionResearchSequenceMismatch {
                        expected,
                        received: event.sequence.get(),
                    });
                }
            }
            previous = Some(event.sequence.get());
        }
        let identity = Self {
            project_id,
            stream_id,
            watermark: events
                .last()
                .expect("non-empty Research stream has a watermark")
                .sequence,
            stream_hash: canonical_blake3_hash(&events).map_err(|error| {
                SessionError::CompactionResearchStreamHash {
                    message: error.to_string(),
                }
            })?,
        };
        identity.validate()?;
        Ok(identity)
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

    fn validate(&self) -> Result<(), SessionError> {
        require_text("evidence_spine.source.project_id", self.project_id.as_str())?;
        require_text("evidence_spine.source.stream_id", self.stream_id.as_str())?;
        if self.watermark.get() == 0 {
            return Err(SessionError::CompactionResearchWatermarkRequired);
        }
        if !is_lowercase_hash(&self.stream_hash) {
            return Err(SessionError::CompactionResearchStreamHash {
                message: "stream_hash must be lowercase 64-character BLAKE3 hex".to_owned(),
            });
        }
        Ok(())
    }
}

/// The authoritative facts available when a new Summary is built or a v2
/// Summary is resumed. This is a borrowed validation view, never a second
/// store.
pub struct CompactionValidationContext<'facts> {
    project_id: &'facts ProjectId,
    research_stream: &'facts ResearchStreamIdentity,
    research_state: &'facts ResearchState,
    execution_receipt_ids: &'facts BTreeSet<ExecutionReceiptId>,
}

impl<'facts> CompactionValidationContext<'facts> {
    pub fn new(
        project_id: &'facts ProjectId,
        research_stream: &'facts ResearchStreamIdentity,
        research_state: &'facts ResearchState,
        execution_receipt_ids: &'facts BTreeSet<ExecutionReceiptId>,
    ) -> Self {
        Self {
            project_id,
            research_stream,
            research_state,
            execution_receipt_ids,
        }
    }
}

/// Typed references retained across Compaction. No copied Finding or
/// Hypothesis prose is authoritative here.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceSpine {
    source: ResearchStreamIdentity,
    target_snapshot_refs: Vec<TargetSnapshotId>,
    active_hypothesis_refs: Vec<HypothesisId>,
    experiment_refs: Vec<ExperimentId>,
    finding_refs: Vec<FindingId>,
    next_action_refs: Vec<NextActionId>,
    artifact_refs: Vec<ArtifactId>,
    execution_receipt_refs: Vec<ExecutionReceiptId>,
}

impl EvidenceSpine {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        source: ResearchStreamIdentity,
        target_snapshot_refs: Vec<TargetSnapshotId>,
        active_hypothesis_refs: Vec<HypothesisId>,
        experiment_refs: Vec<ExperimentId>,
        finding_refs: Vec<FindingId>,
        next_action_refs: Vec<NextActionId>,
        artifact_refs: Vec<ArtifactId>,
        execution_receipt_refs: Vec<ExecutionReceiptId>,
    ) -> Result<Self, SessionError> {
        let spine = Self {
            source,
            target_snapshot_refs,
            active_hypothesis_refs,
            experiment_refs,
            finding_refs,
            next_action_refs,
            artifact_refs,
            execution_receipt_refs,
        };
        spine.validate_intrinsic()?;
        Ok(spine)
    }

    pub fn source(&self) -> &ResearchStreamIdentity {
        &self.source
    }

    pub fn target_snapshot_refs(&self) -> &[TargetSnapshotId] {
        &self.target_snapshot_refs
    }

    pub fn active_hypothesis_refs(&self) -> &[HypothesisId] {
        &self.active_hypothesis_refs
    }

    pub fn experiment_refs(&self) -> &[ExperimentId] {
        &self.experiment_refs
    }

    pub fn finding_refs(&self) -> &[FindingId] {
        &self.finding_refs
    }

    pub fn next_action_refs(&self) -> &[NextActionId] {
        &self.next_action_refs
    }

    pub fn artifact_refs(&self) -> &[ArtifactId] {
        &self.artifact_refs
    }

    pub fn execution_receipt_refs(&self) -> &[ExecutionReceiptId] {
        &self.execution_receipt_refs
    }

    pub(super) fn validate_intrinsic(&self) -> Result<(), SessionError> {
        self.source.validate()?;
        if self.target_snapshot_refs.is_empty() {
            return Err(SessionError::CompactionEvidenceSpineTargetRequired);
        }
        validate_unique_ids(
            "target_snapshot_refs",
            self.target_snapshot_refs
                .iter()
                .map(TargetSnapshotId::as_str),
        )?;
        validate_unique_ids(
            "active_hypothesis_refs",
            self.active_hypothesis_refs.iter().map(HypothesisId::as_str),
        )?;
        validate_unique_ids(
            "experiment_refs",
            self.experiment_refs.iter().map(ExperimentId::as_str),
        )?;
        validate_unique_ids(
            "finding_refs",
            self.finding_refs.iter().map(FindingId::as_str),
        )?;
        validate_unique_ids(
            "next_action_refs",
            self.next_action_refs.iter().map(NextActionId::as_str),
        )?;
        validate_unique_ids(
            "artifact_refs",
            self.artifact_refs.iter().map(ArtifactId::as_str),
        )?;
        validate_unique_ids(
            "execution_receipt_refs",
            self.execution_receipt_refs
                .iter()
                .map(ExecutionReceiptId::as_str),
        )
    }

    pub(super) fn validate_against(
        &self,
        context: &CompactionValidationContext<'_>,
    ) -> Result<(), SessionError> {
        self.validate_intrinsic()?;
        if self.source.project_id() != context.project_id {
            return Err(SessionError::CompactionEvidenceCrossProject {
                expected: context.project_id.clone(),
                received: self.source.project_id().clone(),
            });
        }
        if self.source.stream_id() != context.research_stream.stream_id() {
            return Err(SessionError::CompactionEvidenceResearchStreamMismatch {
                expected: context.research_stream.stream_id().clone(),
                received: self.source.stream_id().clone(),
            });
        }
        if self.source.watermark() != context.research_stream.watermark()
            || self.source.stream_hash() != context.research_stream.stream_hash()
        {
            return Err(SessionError::CompactionEvidenceStaleResearchRevision {
                expected_watermark: context.research_stream.watermark(),
                received_watermark: self.source.watermark(),
            });
        }
        for reference in self.research_fact_refs() {
            if !context.research_state.contains_fact_reference(&reference) {
                if context.research_state.contains_any_fact_id(reference.id()) {
                    return Err(SessionError::CompactionEvidenceTypeMismatch {
                        id: reference.id().to_owned(),
                    });
                }
                return Err(SessionError::CompactionEvidenceReferenceMissing {
                    kind: "research_fact",
                    id: reference.id().to_owned(),
                });
            }
        }
        for id in &self.artifact_refs {
            if context.research_state.artifact(id).is_none() {
                return Err(SessionError::CompactionEvidenceReferenceMissing {
                    kind: "artifact",
                    id: id.as_str().to_owned(),
                });
            }
        }
        for id in &self.execution_receipt_refs {
            if !context.execution_receipt_ids.contains(id) {
                return Err(SessionError::CompactionEvidenceReferenceMissing {
                    kind: "execution_receipt",
                    id: id.as_str().to_owned(),
                });
            }
        }
        Ok(())
    }

    fn research_fact_refs(&self) -> Vec<ResearchFactRef> {
        self.target_snapshot_refs
            .iter()
            .cloned()
            .map(ResearchFactRef::TargetSnapshot)
            .chain(
                self.active_hypothesis_refs
                    .iter()
                    .cloned()
                    .map(ResearchFactRef::Hypothesis),
            )
            .chain(
                self.experiment_refs
                    .iter()
                    .cloned()
                    .map(ResearchFactRef::Experiment),
            )
            .chain(
                self.finding_refs
                    .iter()
                    .cloned()
                    .map(ResearchFactRef::Finding),
            )
            .chain(
                self.next_action_refs
                    .iter()
                    .cloned()
                    .map(ResearchFactRef::NextAction),
            )
            .collect()
    }

    pub(super) fn context_references(&self) -> Result<ContextReferences, SessionError> {
        ContextReferences::new(
            self.research_fact_refs(),
            self.artifact_refs.clone(),
            self.execution_receipt_refs.clone(),
            Vec::new(),
        )
        .map_err(|error| SessionError::CompactionContextProvenance {
            message: error.to_string(),
        })
    }
}

fn is_lowercase_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}
