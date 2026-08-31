//! Strict versioned Research Event decoding and pure state reduction.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::domain::artifacts::{
    ArtifactEvent, ArtifactEventError, ArtifactId, ArtifactRecord, ArtifactState,
};
use crate::domain::canonical_json::canonical_json;
use crate::domain::events::{EventEnvelope, EventId, OccurredAt};
use crate::domain::memory::{
    MEMORY_PAYLOAD_SCHEMA_VERSION, MemoryError, MemoryEvent, MemoryEventType,
};
use crate::domain::tools::ExecutionReceipt;

use super::records::{
    EvidenceLink, EvidenceRef, ExecutionReceiptId, ExperimentCapsuleRecord, ExperimentId,
    ExperimentRecord, ExperimentUpdate, FindingId, FindingRecord, FindingUpdate, HypothesisId,
    HypothesisRecord, HypothesisUpdate, MergeRecordRef, NextActionId, NextActionRecord,
    NextActionUpdate, ResearchFactRef, ResearchRecordRef, TargetSnapshotId, TargetSnapshotRecord,
    require_text,
};
use super::upcast::{VersionedPayload, decode_and_upcast};

pub const RESEARCH_SCHEMA_VERSION: u16 = 2;
const RESEARCH_STREAM_ID: &str = "research";

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ResearchError {
    #[error("unsupported Research Event type `{event_type}`")]
    UnknownEventType { event_type: String },
    #[error("unsupported Research payload version `{received}`")]
    UnsupportedPayloadVersion { received: u16 },
    #[error("Research Event payload is malformed: {message}")]
    MalformedPayload { message: String },
    #[error("Research Event belongs to `{received}`, not the Research stream")]
    UnexpectedStream { received: String },
    #[error("Research record is invalid: {message}")]
    InvalidRecord { message: String },
    #[error("Research record ID `{id}` already exists")]
    DuplicateRecordId { id: String },
    #[error("Research TargetSnapshot fingerprint `{fingerprint}` already exists")]
    DuplicateSnapshotFingerprint { fingerprint: String },
    #[error("Research dedup key `{dedup_key}` already exists")]
    DedupConflict { dedup_key: String },
    #[error("Research action fingerprint `{fingerprint}` already exists")]
    DuplicateActionFingerprint { fingerprint: String },
    #[error("Research reference `{reference}` does not exist")]
    MissingReference { reference: String },
    #[error("ExecutionReceipt reference `{id}` is ambiguous across Session streams")]
    DuplicateExecutionReceiptReference { id: String },
    #[error("Research records cannot be merged across kinds")]
    IncompatibleMergeKinds,
    #[error("Research record `{from}` is already merged")]
    AlreadyMerged { from: String },
    #[error("Research Event ID `{event_id}` conflicts with different validated semantics")]
    EventIdConflict { event_id: String },
    #[error("failed to encode a deterministic Research fingerprint: {message}")]
    Fingerprint { message: String },
    #[error(transparent)]
    Artifact(#[from] ArtifactEventError),
    #[error(transparent)]
    Memory(#[from] MemoryError),
}

/// Project-scoped, read-only references resolved from authoritative Session
/// facts before pure Research reduction. It is never serialized or persisted
/// into the Research stream.
#[derive(Clone, Debug, Default)]
pub(crate) struct ResearchReferenceCatalog {
    execution_receipts: BTreeMap<ExecutionReceiptId, ExecutionReceipt>,
}

/// Conservative applicability of an existing Research record to one exact
/// current TargetSnapshot. `Unknown` preserves the distinction between a
/// target identity that is compatible so far and an actual Snapshot binding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ResearchReferenceTargetApplicability {
    Applicable,
    Incompatible,
    Unknown,
    MissingReference,
}

impl ResearchReferenceCatalog {
    pub(crate) fn insert_execution_receipt(
        &mut self,
        id: ExecutionReceiptId,
        receipt: ExecutionReceipt,
    ) -> Result<(), ResearchError> {
        if self
            .execution_receipts
            .insert(id.clone(), receipt)
            .is_some()
        {
            return Err(ResearchError::DuplicateExecutionReceiptReference { id: id.to_string() });
        }
        Ok(())
    }

    pub(super) fn execution_receipt(&self, id: &ExecutionReceiptId) -> Option<&ExecutionReceipt> {
        self.execution_receipts.get(id)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResearchEventType {
    TargetSnapshotRecorded,
    HypothesisRecorded,
    HypothesisUpdated,
    ExperimentRecorded,
    ExperimentUpdated,
    FindingRecorded,
    FindingUpdated,
    NextActionRecorded,
    NextActionUpdated,
    RecordsMerged,
    ArtifactCreated,
    ArtifactDerived,
    MemoryChanged,
    MemoryExternalChangeDetected,
}

impl ResearchEventType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TargetSnapshotRecorded => "research.target_snapshot_recorded",
            Self::HypothesisRecorded => "research.hypothesis_recorded",
            Self::HypothesisUpdated => "research.hypothesis_updated",
            Self::ExperimentRecorded => "research.experiment_recorded",
            Self::ExperimentUpdated => "research.experiment_updated",
            Self::FindingRecorded => "research.finding_recorded",
            Self::FindingUpdated => "research.finding_updated",
            Self::NextActionRecorded => "research.next_action_recorded",
            Self::NextActionUpdated => "research.next_action_updated",
            Self::RecordsMerged => "research.records_merged",
            Self::ArtifactCreated => "artifact.created",
            Self::ArtifactDerived => "artifact.derived",
            Self::MemoryChanged => "memory.changed",
            Self::MemoryExternalChangeDetected => "memory.external_change_detected",
        }
    }

    fn parse(value: &str) -> Result<Self, ResearchError> {
        match value {
            "research.target_snapshot_recorded" => Ok(Self::TargetSnapshotRecorded),
            "research.hypothesis_recorded" => Ok(Self::HypothesisRecorded),
            "research.hypothesis_updated" => Ok(Self::HypothesisUpdated),
            "research.experiment_recorded" => Ok(Self::ExperimentRecorded),
            "research.experiment_updated" => Ok(Self::ExperimentUpdated),
            "research.finding_recorded" => Ok(Self::FindingRecorded),
            "research.finding_updated" => Ok(Self::FindingUpdated),
            "research.next_action_recorded" => Ok(Self::NextActionRecorded),
            "research.next_action_updated" => Ok(Self::NextActionUpdated),
            "research.records_merged" => Ok(Self::RecordsMerged),
            "artifact.created" => Ok(Self::ArtifactCreated),
            "artifact.derived" => Ok(Self::ArtifactDerived),
            "memory.changed" => Ok(Self::MemoryChanged),
            "memory.external_change_detected" => Ok(Self::MemoryExternalChangeDetected),
            _ => Err(ResearchError::UnknownEventType {
                event_type: value.to_owned(),
            }),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResearchEvent {
    TargetSnapshotRecorded(TargetSnapshotRecord),
    HypothesisRecorded {
        record: HypothesisRecord,
        keep_independent: bool,
    },
    HypothesisUpdated(HypothesisUpdate),
    ExperimentRecorded {
        experiment: ExperimentRecord,
        capsule: Box<ExperimentCapsuleRecord>,
        rerun_reason: Option<String>,
    },
    ExperimentUpdated(ExperimentUpdate),
    FindingRecorded {
        record: FindingRecord,
        keep_independent: bool,
    },
    FindingUpdated(FindingUpdate),
    NextActionRecorded(NextActionRecord),
    NextActionUpdated(NextActionUpdate),
    RecordsMerged {
        from: MergeRecordRef,
        into: MergeRecordRef,
        reason: String,
    },
    Artifact(ArtifactEvent),
    /// A typed Markdown revision fact. The content itself remains authoritative
    /// in `memory/*.md`; Research State never owns Memory projection data.
    Memory(MemoryEvent),
}

impl ResearchEvent {
    pub const fn event_type(&self) -> ResearchEventType {
        match self {
            Self::TargetSnapshotRecorded(_) => ResearchEventType::TargetSnapshotRecorded,
            Self::HypothesisRecorded { .. } => ResearchEventType::HypothesisRecorded,
            Self::HypothesisUpdated(_) => ResearchEventType::HypothesisUpdated,
            Self::ExperimentRecorded { .. } => ResearchEventType::ExperimentRecorded,
            Self::ExperimentUpdated(_) => ResearchEventType::ExperimentUpdated,
            Self::FindingRecorded { .. } => ResearchEventType::FindingRecorded,
            Self::FindingUpdated(_) => ResearchEventType::FindingUpdated,
            Self::NextActionRecorded(_) => ResearchEventType::NextActionRecorded,
            Self::NextActionUpdated(_) => ResearchEventType::NextActionUpdated,
            Self::RecordsMerged { .. } => ResearchEventType::RecordsMerged,
            Self::Artifact(event) => match event {
                ArtifactEvent::Created(_) => ResearchEventType::ArtifactCreated,
                ArtifactEvent::Derived(_) => ResearchEventType::ArtifactDerived,
            },
            Self::Memory(event) => match event.event_type() {
                MemoryEventType::Changed => ResearchEventType::MemoryChanged,
                MemoryEventType::ExternalChangeDetected => {
                    ResearchEventType::MemoryExternalChangeDetected
                }
            },
        }
    }

    pub fn encode_payload(&self) -> Result<Value, ResearchError> {
        if let Self::Artifact(event) = self {
            return serde_json::to_value(VersionedPayload {
                research_schema_version: RESEARCH_SCHEMA_VERSION,
                data: event.encode_payload()?,
            })
            .map_err(|error| ResearchError::MalformedPayload {
                message: error.to_string(),
            });
        }
        if let Self::Memory(event) = self {
            let data = serde_json::to_value(MemoryVersionedPayload {
                memory_schema_version: MEMORY_PAYLOAD_SCHEMA_VERSION,
                data: event.clone(),
            })
            .map_err(|error| ResearchError::MalformedPayload {
                message: error.to_string(),
            })?;
            return serde_json::to_value(VersionedPayload {
                research_schema_version: RESEARCH_SCHEMA_VERSION,
                data,
            })
            .map_err(|error| ResearchError::MalformedPayload {
                message: error.to_string(),
            });
        }
        let data = match self {
            Self::TargetSnapshotRecorded(record) => serde_json::to_value(record),
            Self::HypothesisRecorded {
                record,
                keep_independent,
            } => serde_json::to_value(HypothesisRecordedPayload {
                record: record.clone(),
                keep_independent: *keep_independent,
            }),
            Self::HypothesisUpdated(update) => serde_json::to_value(update),
            Self::ExperimentRecorded {
                experiment,
                capsule,
                rerun_reason,
            } => serde_json::to_value(ExperimentRecordedPayload {
                experiment: experiment.clone(),
                capsule: capsule.as_ref().clone(),
                rerun_reason: rerun_reason.clone(),
            }),
            Self::ExperimentUpdated(update) => serde_json::to_value(update),
            Self::FindingRecorded {
                record,
                keep_independent,
            } => serde_json::to_value(FindingRecordedPayload {
                record: record.clone(),
                keep_independent: *keep_independent,
            }),
            Self::FindingUpdated(update) => serde_json::to_value(update),
            Self::NextActionRecorded(record) => serde_json::to_value(record),
            Self::NextActionUpdated(update) => serde_json::to_value(update),
            Self::RecordsMerged { from, into, reason } => {
                serde_json::to_value(RecordsMergedPayload {
                    from: from.clone(),
                    into: into.clone(),
                    reason: reason.clone(),
                })
            }
            Self::Artifact(_) => unreachable!("Artifact Events return before Research encoding"),
            Self::Memory(_) => unreachable!("Memory Events return before Research encoding"),
        }
        .map_err(|error| ResearchError::MalformedPayload {
            message: error.to_string(),
        })?;
        serde_json::to_value(VersionedPayload {
            research_schema_version: RESEARCH_SCHEMA_VERSION,
            data,
        })
        .map_err(|error| ResearchError::MalformedPayload {
            message: error.to_string(),
        })
    }

    pub fn decode(event_type: &str, payload: Value) -> Result<Self, ResearchError> {
        let event_type = ResearchEventType::parse(event_type)?;
        let versioned = decode_and_upcast(event_type.as_str(), payload, RESEARCH_SCHEMA_VERSION)?;
        match event_type {
            ResearchEventType::TargetSnapshotRecorded => {
                decode_record(versioned.data).map(Self::TargetSnapshotRecorded)
            }
            ResearchEventType::HypothesisRecorded => {
                decode_record(versioned.data).map(|payload: HypothesisRecordedPayload| {
                    Self::HypothesisRecorded {
                        record: payload.record,
                        keep_independent: payload.keep_independent,
                    }
                })
            }
            ResearchEventType::HypothesisUpdated => {
                decode_record(versioned.data).map(Self::HypothesisUpdated)
            }
            ResearchEventType::ExperimentRecorded => {
                decode_record(versioned.data).map(|payload: ExperimentRecordedPayload| {
                    Self::ExperimentRecorded {
                        experiment: payload.experiment,
                        capsule: Box::new(payload.capsule),
                        rerun_reason: payload.rerun_reason,
                    }
                })
            }
            ResearchEventType::ExperimentUpdated => {
                decode_record(versioned.data).map(Self::ExperimentUpdated)
            }
            ResearchEventType::FindingRecorded => {
                decode_record(versioned.data).map(|payload: FindingRecordedPayload| {
                    Self::FindingRecorded {
                        record: payload.record,
                        keep_independent: payload.keep_independent,
                    }
                })
            }
            ResearchEventType::FindingUpdated => {
                decode_record(versioned.data).map(Self::FindingUpdated)
            }
            ResearchEventType::NextActionRecorded => {
                decode_record(versioned.data).map(Self::NextActionRecorded)
            }
            ResearchEventType::NextActionUpdated => {
                decode_record(versioned.data).map(Self::NextActionUpdated)
            }
            ResearchEventType::RecordsMerged => {
                decode_record(versioned.data).map(|payload: RecordsMergedPayload| {
                    Self::RecordsMerged {
                        from: payload.from,
                        into: payload.into,
                        reason: payload.reason,
                    }
                })
            }
            ResearchEventType::ArtifactCreated | ResearchEventType::ArtifactDerived => {
                Ok(ArtifactEvent::decode(event_type.as_str(), versioned.data)
                    .map(Self::Artifact)?)
            }
            ResearchEventType::MemoryChanged | ResearchEventType::MemoryExternalChangeDetected => {
                let memory: MemoryVersionedPayload = decode_record(versioned.data)?;
                if memory.memory_schema_version != MEMORY_PAYLOAD_SCHEMA_VERSION {
                    return Err(ResearchError::Memory(
                        MemoryError::UnsupportedPayloadVersion {
                            received: memory.memory_schema_version,
                        },
                    ));
                }
                let expected_type = match event_type {
                    ResearchEventType::MemoryChanged => MemoryEventType::Changed,
                    ResearchEventType::MemoryExternalChangeDetected => {
                        MemoryEventType::ExternalChangeDetected
                    }
                    _ => unreachable!("only Memory Event types enter this decode branch"),
                };
                if memory.data.event_type() != expected_type {
                    return Err(ResearchError::MalformedPayload {
                        message: "Memory payload kind does not match outer Event type".to_owned(),
                    });
                }
                Ok(Self::Memory(memory.data))
            }
        }
    }

    /// Performs the event-local validation that is independent of prior
    /// ResearchState. State-dependent checks remain in ResearchReducer.
    pub(crate) fn validate_intrinsic(&self) -> Result<(), ResearchError> {
        match self {
            Self::TargetSnapshotRecorded(record) => record.validate(),
            Self::HypothesisRecorded { record, .. } => {
                record.validate()?;
                reject_recorded_merge("HypothesisRecorded", record.merged_into.is_some())
            }
            Self::HypothesisUpdated(update) => update.validate(),
            Self::ExperimentRecorded {
                experiment,
                capsule,
                ..
            } => experiment.validate(capsule),
            Self::ExperimentUpdated(update) => update.validate(),
            Self::FindingRecorded { record, .. } => {
                record.validate()?;
                reject_recorded_merge("FindingRecorded", record.merged_into.is_some())
            }
            Self::FindingUpdated(update) => update.validate(),
            Self::NextActionRecorded(record) => record.validate(),
            Self::NextActionUpdated(update) => update.validate(),
            Self::RecordsMerged { from, into, reason } => {
                require_text("RecordsMerged reason", reason)?;
                if !same_merge_kind(from, into) {
                    return Err(ResearchError::IncompatibleMergeKinds);
                }
                if from == into {
                    return Err(ResearchError::InvalidRecord {
                        message: "RecordsMerged source and destination must differ".to_owned(),
                    });
                }
                Ok(())
            }
            Self::Artifact(event) => event.validate().map_err(ResearchError::from),
            Self::Memory(event) => event.validate().map_err(ResearchError::from),
        }
    }
}

#[derive(Debug, Default, Serialize)]
pub struct ResearchState {
    #[serde(skip)]
    applied_event_digests: BTreeMap<EventId, String>,
    target_snapshots: BTreeMap<TargetSnapshotId, TargetSnapshotRecord>,
    hypotheses: BTreeMap<HypothesisId, HypothesisRecord>,
    experiments: BTreeMap<ExperimentId, ExperimentRecord>,
    capsules: BTreeMap<super::records::CapsuleId, ExperimentCapsuleRecord>,
    findings: BTreeMap<FindingId, FindingRecord>,
    next_actions: BTreeMap<NextActionId, NextActionRecord>,
    artifacts: ArtifactState,
}

impl ResearchState {
    pub fn applied_event_count(&self) -> usize {
        self.applied_event_digests.len()
    }

    pub fn has_event_id(&self, event_id: &EventId) -> bool {
        self.applied_event_digests.contains_key(event_id)
    }

    /// Resolves one immutable Artifact fact from the strict Research replay.
    /// Blob bytes remain owned by the Artifact Store Adapter.
    pub(crate) fn artifact(&self, id: &ArtifactId) -> Option<&ArtifactRecord> {
        self.artifacts.get(id)
    }

    pub fn target_snapshot_by_fingerprint(
        &self,
        fingerprint: &str,
    ) -> Option<&TargetSnapshotRecord> {
        self.target_snapshots
            .values()
            .find(|record| record.snapshot_fingerprint == fingerprint)
    }

    pub fn hypotheses_by_dedup_key(&self, dedup_key: &str) -> Vec<&HypothesisRecord> {
        self.hypotheses
            .values()
            .filter(|record| record.dedup_key == dedup_key)
            .collect()
    }

    pub fn findings_by_dedup_key(&self, dedup_key: &str) -> Vec<&FindingRecord> {
        self.findings
            .values()
            .filter(|record| record.dedup_key == dedup_key)
            .collect()
    }

    pub(crate) fn target_snapshot(&self, id: &TargetSnapshotId) -> Option<&TargetSnapshotRecord> {
        self.target_snapshots.get(id)
    }

    pub fn experiments_by_action_fingerprint(&self, fingerprint: &str) -> Vec<&ExperimentRecord> {
        self.experiments
            .values()
            .filter(|record| record.action_fingerprint == fingerprint)
            .collect()
    }

    /// Resolves the closed reference vocabulary shared by Context provenance
    /// and Evidence-Spine Compaction without exposing the backing maps.
    pub(crate) fn contains_fact_reference(&self, reference: &ResearchFactRef) -> bool {
        match reference {
            ResearchFactRef::TargetSnapshot(id) => self.target_snapshots.contains_key(id),
            ResearchFactRef::Hypothesis(id) => self.hypotheses.contains_key(id),
            ResearchFactRef::Experiment(id) => self.experiments.contains_key(id),
            ResearchFactRef::Finding(id) => self.findings.contains_key(id),
            ResearchFactRef::NextAction(id) => self.next_actions.contains_key(id),
        }
    }

    pub(crate) fn contains_any_fact_id(&self, id: &str) -> bool {
        self.target_snapshots
            .keys()
            .any(|value| value.as_str() == id)
            || self.hypotheses.keys().any(|value| value.as_str() == id)
            || self.experiments.keys().any(|value| value.as_str() == id)
            || self.findings.keys().any(|value| value.as_str() == id)
            || self.next_actions.keys().any(|value| value.as_str() == id)
    }

    fn contains_research_reference(&self, reference: &ResearchRecordRef) -> bool {
        match reference {
            ResearchRecordRef::Hypothesis(id) => self.hypotheses.contains_key(id),
            ResearchRecordRef::Experiment(id) => self.experiments.contains_key(id),
            ResearchRecordRef::Finding(id) => self.findings.contains_key(id),
        }
    }

    fn contains_evidence_reference(
        &self,
        references: &ResearchReferenceCatalog,
        reference: &EvidenceRef,
    ) -> bool {
        match reference {
            EvidenceRef::Artifact(id) => self.artifacts.get(id).is_some(),
            EvidenceRef::Experiment(id) => self.experiments.contains_key(id),
            EvidenceRef::ExecutionReceipt(id) => references.execution_receipt(id).is_some(),
        }
    }

    pub(crate) fn evidence_gap_reports(
        &self,
        references: &ResearchReferenceCatalog,
    ) -> Vec<super::evidence_gap::EvidenceGapReport> {
        super::evidence_gap::project(self, references)
    }

    pub(crate) fn retrieval_target_snapshot_records(
        &self,
    ) -> impl Iterator<Item = &TargetSnapshotRecord> {
        self.target_snapshots.values()
    }

    pub(crate) fn hypothesis_records(&self) -> impl Iterator<Item = &HypothesisRecord> {
        self.hypotheses.values()
    }

    pub(crate) fn finding_records(&self) -> impl Iterator<Item = &FindingRecord> {
        self.findings.values()
    }

    pub(crate) fn retrieval_experiment_records(&self) -> impl Iterator<Item = &ExperimentRecord> {
        self.experiments.values()
    }

    pub(crate) fn retrieval_next_action_records(&self) -> impl Iterator<Item = &NextActionRecord> {
        self.next_actions.values()
    }

    /// The sole TargetSnapshot applicability projection for an existing
    /// NextAction's typed Research reference. It deliberately never infers a
    /// Snapshot, version, prerequisite, or experiment identity from action
    /// prose.
    pub(crate) fn research_reference_target_applicability(
        &self,
        reference: &ResearchRecordRef,
        current_target: &TargetSnapshotId,
    ) -> ResearchReferenceTargetApplicability {
        let Some(current) = self.target_snapshot(current_target) else {
            return ResearchReferenceTargetApplicability::MissingReference;
        };
        match reference {
            ResearchRecordRef::Hypothesis(id) => match self.hypotheses.get(id) {
                Some(record) if record.target_id == current.target_id => {
                    ResearchReferenceTargetApplicability::Unknown
                }
                Some(_) => ResearchReferenceTargetApplicability::Incompatible,
                None => ResearchReferenceTargetApplicability::MissingReference,
            },
            ResearchRecordRef::Experiment(id) => match self.experiments.get(id) {
                Some(record) if &record.target_snapshot_id == current_target => {
                    ResearchReferenceTargetApplicability::Applicable
                }
                Some(_) => ResearchReferenceTargetApplicability::Incompatible,
                None => ResearchReferenceTargetApplicability::MissingReference,
            },
            ResearchRecordRef::Finding(id) => match self.findings.get(id) {
                Some(record) if !record.target_snapshot_ids.is_empty() => {
                    if record.target_snapshot_ids.contains(current_target) {
                        ResearchReferenceTargetApplicability::Applicable
                    } else {
                        ResearchReferenceTargetApplicability::Incompatible
                    }
                }
                Some(record)
                    if record.target_id == current.target_id
                        && record.target_version == current.version =>
                {
                    ResearchReferenceTargetApplicability::Unknown
                }
                Some(_) => ResearchReferenceTargetApplicability::Incompatible,
                None => ResearchReferenceTargetApplicability::MissingReference,
            },
        }
    }

    pub(crate) fn retrieval_artifact_records(&self) -> impl Iterator<Item = &ArtifactRecord> {
        self.artifacts.records()
    }

    pub(super) fn experiment(&self, id: &ExperimentId) -> Option<&ExperimentRecord> {
        self.experiments.get(id)
    }

    pub(super) fn capsule_for_experiment(
        &self,
        experiment: &ExperimentRecord,
    ) -> Option<&ExperimentCapsuleRecord> {
        self.capsules.get(&experiment.capsule_id)
    }

    pub(super) fn artifact_applies(
        &self,
        id: &ArtifactId,
        target_id: &str,
        target_version: Option<&str>,
    ) -> bool {
        self.artifacts.get(id).is_some_and(|artifact| {
            self.target_snapshot_applies(&artifact.target_snapshot_id, target_id, target_version)
        })
    }

    pub(super) fn target_snapshot_applies(
        &self,
        id: &TargetSnapshotId,
        target_id: &str,
        target_version: Option<&str>,
    ) -> bool {
        self.target_snapshot(id).is_some_and(|snapshot| {
            snapshot.target_id == target_id
                && target_version.is_none_or(|version| snapshot.version == version)
        })
    }

    pub(super) fn experiment_applies(
        &self,
        experiment: &ExperimentRecord,
        target_id: &str,
        target_version: Option<&str>,
    ) -> bool {
        self.target_snapshot_applies(&experiment.target_snapshot_id, target_id, target_version)
    }

    pub(super) fn evidence_reference_applies(
        &self,
        references: &ResearchReferenceCatalog,
        reference: &EvidenceRef,
        target_id: &str,
        target_version: Option<&str>,
    ) -> bool {
        match reference {
            EvidenceRef::Artifact(id) => self.artifact_applies(id, target_id, target_version),
            EvidenceRef::Experiment(id) => self.experiments.get(id).is_some_and(|experiment| {
                self.experiment_applies(experiment, target_id, target_version)
            }),
            EvidenceRef::ExecutionReceipt(id) => {
                references.execution_receipt(id).is_some()
                    && self.capsules.values().any(|capsule| {
                        capsule.execution_receipt_ids.contains(id)
                            && self.experiments.get(&capsule.experiment_id).is_some_and(
                                |experiment| {
                                    self.experiment_applies(experiment, target_id, target_version)
                                },
                            )
                    })
            }
        }
    }

    fn merge_target_exists(&self, reference: &MergeRecordRef) -> bool {
        match reference {
            MergeRecordRef::Hypothesis(id) => self.hypotheses.contains_key(id),
            MergeRecordRef::Finding(id) => self.findings.contains_key(id),
        }
    }

    fn record_is_merged(&self, reference: &MergeRecordRef) -> bool {
        match reference {
            MergeRecordRef::Hypothesis(id) => self
                .hypotheses
                .get(id)
                .is_some_and(|record| record.merged_into.is_some()),
            MergeRecordRef::Finding(id) => self
                .findings
                .get(id)
                .is_some_and(|record| record.merged_into.is_some()),
        }
    }

    fn apply_merge(&mut self, from: &MergeRecordRef, into: &MergeRecordRef) {
        match (from, into) {
            (MergeRecordRef::Hypothesis(source), MergeRecordRef::Hypothesis(_)) => {
                self.hypotheses.get_mut(source).unwrap().merged_into = Some(into.clone());
            }
            (MergeRecordRef::Finding(source), MergeRecordRef::Finding(_)) => {
                self.findings.get_mut(source).unwrap().merged_into = Some(into.clone());
            }
            _ => unreachable!("merge kinds are validated before application"),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ResearchReducer;

impl ResearchReducer {
    pub fn reduce<Events>(&self, events: Events) -> Result<ResearchState, ResearchError>
    where
        Events: IntoIterator<Item = EventEnvelope<Value>>,
    {
        self.reduce_with_references(events, &ResearchReferenceCatalog::default())
    }

    pub(crate) fn reduce_with_references<Events>(
        &self,
        events: Events,
        references: &ResearchReferenceCatalog,
    ) -> Result<ResearchState, ResearchError>
    where
        Events: IntoIterator<Item = EventEnvelope<Value>>,
    {
        let mut state = ResearchState::default();
        for event in events {
            self.apply_with_references(&mut state, event, references)?;
        }
        Ok(state)
    }

    pub fn apply(
        &self,
        state: &mut ResearchState,
        envelope: EventEnvelope<Value>,
    ) -> Result<(), ResearchError> {
        self.apply_with_references(state, envelope, &ResearchReferenceCatalog::default())
    }

    pub(crate) fn apply_with_references(
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
            let payload = validate_stream_recovered_payload(envelope.payload)?;
            let payload =
                serde_json::to_value(payload).map_err(|error| ResearchError::MalformedPayload {
                    message: error.to_string(),
                })?;
            let digest = semantic_event_digest(
                &envelope.occurred_at,
                envelope.event_type.as_str(),
                &payload,
            )?;
            if event_id_is_idempotent(state, &envelope.event_id, &digest)? {
                return Ok(());
            }
            state
                .applied_event_digests
                .insert(envelope.event_id, digest);
            return Ok(());
        }
        let event = ResearchEvent::decode(envelope.event_type.as_str(), envelope.payload)?;
        event.validate_intrinsic()?;
        validate_created_by_event_id(&event, &envelope.event_id)?;
        let payload = event.encode_payload()?;
        let digest = semantic_event_digest(
            &envelope.occurred_at,
            envelope.event_type.as_str(),
            &payload,
        )?;
        if event_id_is_idempotent(state, &envelope.event_id, &digest)? {
            return Ok(());
        }
        self.apply_event_with_references(state, event, references)?;
        state
            .applied_event_digests
            .insert(envelope.event_id, digest);
        Ok(())
    }

    pub fn apply_event(
        &self,
        state: &mut ResearchState,
        event: ResearchEvent,
    ) -> Result<(), ResearchError> {
        self.apply_event_with_references(state, event, &ResearchReferenceCatalog::default())
    }

    pub(crate) fn apply_event_with_references(
        &self,
        state: &mut ResearchState,
        event: ResearchEvent,
        references: &ResearchReferenceCatalog,
    ) -> Result<(), ResearchError> {
        event.validate_intrinsic()?;
        match event {
            ResearchEvent::TargetSnapshotRecorded(record) => {
                validate_target_snapshot_recorded(state, &record)?;
                state.target_snapshots.insert(record.id.clone(), record);
            }
            ResearchEvent::HypothesisRecorded {
                record,
                keep_independent,
            } => {
                validate_hypothesis_recorded(state, references, &record, keep_independent)?;
                state.hypotheses.insert(record.id.clone(), record);
            }
            ResearchEvent::HypothesisUpdated(update) => {
                let record = stage_hypothesis_update(state, references, &update)?;
                state.hypotheses.insert(record.id.clone(), record);
            }
            ResearchEvent::ExperimentRecorded {
                experiment,
                capsule,
                rerun_reason,
            } => {
                validate_experiment_recorded(
                    state,
                    references,
                    &experiment,
                    &capsule,
                    rerun_reason.as_deref(),
                )?;
                state.experiments.insert(experiment.id.clone(), experiment);
                state.capsules.insert(capsule.id.clone(), *capsule);
            }
            ResearchEvent::ExperimentUpdated(update) => {
                let record = stage_experiment_update(state, &update)?;
                state.experiments.insert(record.id.clone(), record);
            }
            ResearchEvent::FindingRecorded {
                record,
                keep_independent,
            } => {
                validate_finding_recorded(state, references, &record, keep_independent)?;
                state.findings.insert(record.id.clone(), record);
            }
            ResearchEvent::FindingUpdated(update) => {
                let record = stage_finding_update(state, references, &update)?;
                state.findings.insert(record.id.clone(), record);
            }
            ResearchEvent::NextActionRecorded(record) => {
                validate_next_action_recorded(state, &record)?;
                state.next_actions.insert(record.id.clone(), record);
            }
            ResearchEvent::NextActionUpdated(update) => {
                let record = stage_next_action_update(state, &update)?;
                state.next_actions.insert(record.id.clone(), record);
            }
            ResearchEvent::RecordsMerged {
                from,
                into,
                reason: _,
            } => {
                validate_records_merged(state, &from, &into)?;
                state.apply_merge(&from, &into);
            }
            ResearchEvent::Artifact(event) => {
                validate_artifact_event(state, &event)?;
                state.artifacts.insert(event.record().clone());
            }
            ResearchEvent::Memory(_) => {}
        }
        Ok(())
    }

    /// Checks an Event against an immutable state before a durable append.
    /// Update validation stages only the affected record, never a replay-sized
    /// clone of ResearchState.
    pub(crate) fn validate_event(
        &self,
        state: &ResearchState,
        event: &ResearchEvent,
    ) -> Result<(), ResearchError> {
        self.validate_event_with_references(state, event, &ResearchReferenceCatalog::default())
    }

    pub(crate) fn validate_event_with_references(
        &self,
        state: &ResearchState,
        event: &ResearchEvent,
        references: &ResearchReferenceCatalog,
    ) -> Result<(), ResearchError> {
        event.validate_intrinsic()?;
        match event {
            ResearchEvent::TargetSnapshotRecorded(record) => {
                validate_target_snapshot_recorded(state, record)
            }
            ResearchEvent::HypothesisRecorded {
                record,
                keep_independent,
            } => validate_hypothesis_recorded(state, references, record, *keep_independent),
            ResearchEvent::HypothesisUpdated(update) => {
                stage_hypothesis_update(state, references, update).map(|_| ())
            }
            ResearchEvent::ExperimentRecorded {
                experiment,
                capsule,
                rerun_reason,
            } => validate_experiment_recorded(
                state,
                references,
                experiment,
                capsule,
                rerun_reason.as_deref(),
            ),
            ResearchEvent::ExperimentUpdated(update) => {
                stage_experiment_update(state, update).map(|_| ())
            }
            ResearchEvent::FindingRecorded {
                record,
                keep_independent,
            } => validate_finding_recorded(state, references, record, *keep_independent),
            ResearchEvent::FindingUpdated(update) => {
                stage_finding_update(state, references, update).map(|_| ())
            }
            ResearchEvent::NextActionRecorded(record) => {
                validate_next_action_recorded(state, record)
            }
            ResearchEvent::NextActionUpdated(update) => {
                stage_next_action_update(state, update).map(|_| ())
            }
            ResearchEvent::RecordsMerged {
                from,
                into,
                reason: _,
            } => validate_records_merged(state, from, into),
            ResearchEvent::Artifact(event) => validate_artifact_event(state, event),
            ResearchEvent::Memory(event) => event.validate().map_err(ResearchError::from),
        }
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

fn validate_artifact_event(
    state: &ResearchState,
    event: &ArtifactEvent,
) -> Result<(), ResearchError> {
    event.validate()?;
    let record = event.record();
    state
        .artifacts
        .validate_record(record)
        .map_err(|error| ResearchError::InvalidRecord {
            message: error.to_string(),
        })?;
    ensure_reference(
        state.target_snapshot(&record.target_snapshot_id).is_some(),
        record.target_snapshot_id.as_str(),
    )?;
    Ok(())
}

fn validate_target_snapshot_recorded(
    state: &ResearchState,
    record: &TargetSnapshotRecord,
) -> Result<(), ResearchError> {
    if state.target_snapshots.contains_key(&record.id) {
        return Err(duplicate_id(record.id.as_str()));
    }
    if state
        .target_snapshot_by_fingerprint(&record.snapshot_fingerprint)
        .is_some()
    {
        return Err(ResearchError::DuplicateSnapshotFingerprint {
            fingerprint: record.snapshot_fingerprint.clone(),
        });
    }
    Ok(())
}

fn validate_hypothesis_recorded(
    state: &ResearchState,
    references: &ResearchReferenceCatalog,
    record: &HypothesisRecord,
    keep_independent: bool,
) -> Result<(), ResearchError> {
    if state.hypotheses.contains_key(&record.id) {
        return Err(duplicate_id(record.id.as_str()));
    }
    for link in &record.evidence_links {
        validate_evidence_reference_for_subject(
            state,
            references,
            &link.evidence,
            &record.target_id,
            None,
        )?;
    }
    if !keep_independent && !state.hypotheses_by_dedup_key(&record.dedup_key).is_empty() {
        return Err(ResearchError::DedupConflict {
            dedup_key: record.dedup_key.clone(),
        });
    }
    Ok(())
}

fn stage_hypothesis_update(
    state: &ResearchState,
    references: &ResearchReferenceCatalog,
    update: &HypothesisUpdate,
) -> Result<HypothesisRecord, ResearchError> {
    let mut record = state.hypotheses.get(&update.id).cloned().ok_or_else(|| {
        ResearchError::MissingReference {
            reference: update.id.to_string(),
        }
    })?;
    for link in &update.evidence_links_add {
        validate_evidence_reference_for_subject(
            state,
            references,
            &link.evidence,
            &record.target_id,
            None,
        )?;
    }
    if let Some(status) = update.status {
        record.status = status;
    }
    reject_existing_evidence_links(&record.evidence_links, &update.evidence_links_add)?;
    record
        .evidence_links
        .extend(update.evidence_links_add.clone());
    if let Some(last_action) = &update.last_action {
        record.last_action = Some(last_action.clone());
    }
    if let Some(next_action) = &update.next_action {
        record.next_action = Some(next_action.clone());
    }
    record.validate()?;
    Ok(record)
}

fn validate_experiment_recorded(
    state: &ResearchState,
    references: &ResearchReferenceCatalog,
    experiment: &ExperimentRecord,
    capsule: &ExperimentCapsuleRecord,
    rerun_reason: Option<&str>,
) -> Result<(), ResearchError> {
    if state.experiments.contains_key(&experiment.id) {
        return Err(duplicate_id(experiment.id.as_str()));
    }
    if state.capsules.contains_key(&capsule.id) {
        return Err(duplicate_id(capsule.id.as_str()));
    }
    let hypothesis = state
        .hypotheses
        .get(&experiment.hypothesis_id)
        .ok_or_else(|| ResearchError::MissingReference {
            reference: experiment.hypothesis_id.to_string(),
        })?;
    ensure_reference(
        state
            .target_snapshots
            .contains_key(&experiment.target_snapshot_id),
        experiment.target_snapshot_id.as_str(),
    )?;
    ensure_target_snapshot_applicable(
        state,
        &experiment.target_snapshot_id,
        &hypothesis.target_id,
        None,
    )?;
    for receipt_id in &capsule.execution_receipt_ids {
        let receipt = references.execution_receipt(receipt_id).ok_or_else(|| {
            ResearchError::MissingReference {
                reference: format!("ExecutionReceipt({receipt_id})"),
            }
        })?;
        if !capsule.execution_ids.contains(&receipt.execution_id) {
            return Err(ResearchError::InvalidRecord {
                message: format!(
                    "ExperimentCapsule Receipt `{receipt_id}` resolves to Execution `{}`, which is absent from execution_ids",
                    receipt.execution_id
                ),
            });
        }
    }
    if state
        .experiments
        .values()
        .any(|record| record.action_fingerprint == experiment.action_fingerprint)
    {
        let reason = rerun_reason.ok_or_else(|| ResearchError::DuplicateActionFingerprint {
            fingerprint: experiment.action_fingerprint.clone(),
        })?;
        require_text("Experiment rerun_reason", reason)?;
    }
    Ok(())
}

fn stage_experiment_update(
    state: &ResearchState,
    update: &ExperimentUpdate,
) -> Result<ExperimentRecord, ResearchError> {
    let mut record = state.experiments.get(&update.id).cloned().ok_or_else(|| {
        ResearchError::MissingReference {
            reference: update.id.to_string(),
        }
    })?;
    if let Some(result) = update.result {
        record.result = result;
    }
    extend_unique(&mut record.artifact_refs, update.artifact_refs_add.clone());
    if let Some(notes_append) = &update.notes_append {
        if !record.notes.is_empty() {
            record.notes.push('\n');
        }
        record.notes.push_str(notes_append);
    }
    Ok(record)
}

fn validate_finding_recorded(
    state: &ResearchState,
    references: &ResearchReferenceCatalog,
    record: &FindingRecord,
    keep_independent: bool,
) -> Result<(), ResearchError> {
    if state.findings.contains_key(&record.id) {
        return Err(duplicate_id(record.id.as_str()));
    }
    for snapshot_id in &record.target_snapshot_ids {
        ensure_reference(
            state.target_snapshot(snapshot_id).is_some(),
            snapshot_id.as_str(),
        )?;
        ensure_target_snapshot_applicable(
            state,
            snapshot_id,
            &record.target_id,
            Some(&record.target_version),
        )?;
    }
    for link in &record.evidence_links {
        validate_evidence_reference_for_subject(
            state,
            references,
            &link.evidence,
            &record.target_id,
            Some(&record.target_version),
        )?;
    }
    if !keep_independent && !state.findings_by_dedup_key(&record.dedup_key).is_empty() {
        return Err(ResearchError::DedupConflict {
            dedup_key: record.dedup_key.clone(),
        });
    }
    Ok(())
}

fn stage_finding_update(
    state: &ResearchState,
    references: &ResearchReferenceCatalog,
    update: &FindingUpdate,
) -> Result<FindingRecord, ResearchError> {
    let mut record =
        state
            .findings
            .get(&update.id)
            .cloned()
            .ok_or_else(|| ResearchError::MissingReference {
                reference: update.id.to_string(),
            })?;
    for snapshot_id in &update.target_snapshot_ids_add {
        ensure_reference(
            state.target_snapshot(snapshot_id).is_some(),
            snapshot_id.as_str(),
        )?;
        ensure_target_snapshot_applicable(
            state,
            snapshot_id,
            &record.target_id,
            Some(&record.target_version),
        )?;
    }
    for link in &update.evidence_links_add {
        validate_evidence_reference_for_subject(
            state,
            references,
            &link.evidence,
            &record.target_id,
            Some(&record.target_version),
        )?;
    }
    if let Some(status) = update.status {
        record.status = status;
    }
    extend_unique(
        &mut record.target_snapshot_ids,
        update.target_snapshot_ids_add.clone(),
    );
    extend_unique(&mut record.prerequisites, update.prerequisites_add.clone());
    reject_existing_evidence_links(&record.evidence_links, &update.evidence_links_add)?;
    record
        .evidence_links
        .extend(update.evidence_links_add.clone());
    if let Some(impact) = &update.impact {
        record.impact = impact.clone();
    }
    record.validate()?;
    Ok(record)
}

fn validate_next_action_recorded(
    state: &ResearchState,
    record: &NextActionRecord,
) -> Result<(), ResearchError> {
    if state.next_actions.contains_key(&record.id) {
        return Err(duplicate_id(record.id.as_str()));
    }
    ensure_reference(
        state.contains_research_reference(&record.research_id),
        &format!("{:?}", record.research_id),
    )
}

fn stage_next_action_update(
    state: &ResearchState,
    update: &NextActionUpdate,
) -> Result<NextActionRecord, ResearchError> {
    let mut record = state.next_actions.get(&update.id).cloned().ok_or_else(|| {
        ResearchError::MissingReference {
            reference: update.id.to_string(),
        }
    })?;
    if let Some(priority) = update.priority {
        record.priority = priority;
    }
    if let Some(action) = &update.action {
        record.action = action.clone();
    }
    if let Some(status) = update.status {
        record.status = status;
    }
    record.validate()?;
    Ok(record)
}

fn validate_records_merged(
    state: &ResearchState,
    from: &MergeRecordRef,
    into: &MergeRecordRef,
) -> Result<(), ResearchError> {
    ensure_reference(state.merge_target_exists(from), &format!("{from:?}"))?;
    ensure_reference(state.merge_target_exists(into), &format!("{into:?}"))?;
    if state.record_is_merged(from) {
        return Err(ResearchError::AlreadyMerged {
            from: format!("{from:?}"),
        });
    }
    Ok(())
}

fn duplicate_id(id: &str) -> ResearchError {
    ResearchError::DuplicateRecordId { id: id.to_owned() }
}

fn ensure_reference(exists: bool, reference: &str) -> Result<(), ResearchError> {
    if exists {
        Ok(())
    } else {
        Err(ResearchError::MissingReference {
            reference: reference.to_owned(),
        })
    }
}

fn validate_evidence_reference_for_subject(
    state: &ResearchState,
    references: &ResearchReferenceCatalog,
    reference: &EvidenceRef,
    target_id: &str,
    target_version: Option<&str>,
) -> Result<(), ResearchError> {
    ensure_reference(
        state.contains_evidence_reference(references, reference),
        &format!("{reference:?}"),
    )?;
    if state.evidence_reference_applies(references, reference, target_id, target_version) {
        Ok(())
    } else {
        Err(ResearchError::InvalidRecord {
            message: format!(
                "Evidence reference `{reference:?}` is not applicable to target `{target_id}`{}",
                target_version.map_or_else(String::new, |version| format!(" version `{version}`"))
            ),
        })
    }
}

fn ensure_target_snapshot_applicable(
    state: &ResearchState,
    snapshot_id: &TargetSnapshotId,
    target_id: &str,
    target_version: Option<&str>,
) -> Result<(), ResearchError> {
    if state.target_snapshot_applies(snapshot_id, target_id, target_version) {
        Ok(())
    } else {
        Err(ResearchError::InvalidRecord {
            message: format!(
                "TargetSnapshot `{snapshot_id}` is not applicable to target `{target_id}`{}",
                target_version.map_or_else(String::new, |version| format!(" version `{version}`"))
            ),
        })
    }
}

fn same_merge_kind(left: &MergeRecordRef, right: &MergeRecordRef) -> bool {
    matches!(
        (left, right),
        (MergeRecordRef::Hypothesis(_), MergeRecordRef::Hypothesis(_))
            | (MergeRecordRef::Finding(_), MergeRecordRef::Finding(_))
    )
}

fn reject_recorded_merge(subject: &str, merged: bool) -> Result<(), ResearchError> {
    if merged {
        Err(ResearchError::InvalidRecord {
            message: format!("{subject} cannot set merged_into; only RecordsMerged may do so"),
        })
    } else {
        Ok(())
    }
}

fn extend_unique<T: Eq>(destination: &mut Vec<T>, additions: Vec<T>) {
    for addition in additions {
        if !destination.contains(&addition) {
            destination.push(addition);
        }
    }
}

fn reject_existing_evidence_links(
    existing: &[EvidenceLink],
    additions: &[EvidenceLink],
) -> Result<(), ResearchError> {
    if let Some(duplicate) = additions.iter().find(|link| existing.contains(link)) {
        Err(ResearchError::InvalidRecord {
            message: format!("EvidenceLink already exists `{duplicate:?}`"),
        })
    } else {
        Ok(())
    }
}

fn decode_record<T: for<'de> Deserialize<'de>>(value: Value) -> Result<T, ResearchError> {
    serde_json::from_value(value).map_err(|error| ResearchError::MalformedPayload {
        message: error.to_string(),
    })
}

fn validate_stream_recovered_payload(
    value: Value,
) -> Result<StreamRecoveredPayload, ResearchError> {
    let payload: StreamRecoveredPayload = decode_record(value)?;
    require_text(
        "StreamRecovered recovery_artifact",
        &payload.recovery_artifact,
    )?;
    Ok(payload)
}

fn event_id_is_idempotent(
    state: &ResearchState,
    event_id: &EventId,
    digest: &str,
) -> Result<bool, ResearchError> {
    match state.applied_event_digests.get(event_id) {
        Some(existing) if existing == digest => Ok(true),
        Some(_) => Err(ResearchError::EventIdConflict {
            event_id: event_id.to_string(),
        }),
        None => Ok(false),
    }
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
struct MemoryVersionedPayload {
    memory_schema_version: u16,
    data: MemoryEvent,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct HypothesisRecordedPayload {
    record: HypothesisRecord,
    #[serde(default)]
    keep_independent: bool,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ExperimentRecordedPayload {
    experiment: ExperimentRecord,
    capsule: ExperimentCapsuleRecord,
    #[serde(default)]
    rerun_reason: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct FindingRecordedPayload {
    record: FindingRecord,
    #[serde(default)]
    keep_independent: bool,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RecordsMergedPayload {
    from: MergeRecordRef,
    into: MergeRecordRef,
    reason: String,
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
