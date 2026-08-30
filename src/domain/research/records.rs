//! Typed Research record contracts and constructor validation.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::domain::artifacts::ArtifactId;
use crate::domain::tools::ExecutionId;

use super::fingerprint::{action_fingerprint, dedup_key, normalize_action, snapshot_fingerprint};
use super::reducer::ResearchError;

macro_rules! research_id {
    ($name:ident, $documentation:literal) => {
        #[doc = $documentation]
        #[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }
    };
}

research_id!(
    TargetSnapshotId,
    "A stable target-version snapshot identity."
);
research_id!(HypothesisId, "A stable Research hypothesis identity.");
research_id!(ExperimentId, "A stable Research experiment identity.");
research_id!(FindingId, "A stable Research finding identity.");
research_id!(NextActionId, "A stable next-action identity.");
research_id!(CapsuleId, "A stable reproducibility capsule identity.");
research_id!(
    ExecutionReceiptId,
    "A typed reference to a persisted execution receipt."
);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    Source,
    Web,
    Mobile,
    Binary,
    Firmware,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HypothesisStatus {
    Active,
    Testing,
    Supported,
    Rejected,
    Blocked,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExperimentResult {
    Pending,
    Positive,
    Negative,
    Inconclusive,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingStatus {
    Candidate,
    Reproduced,
    Confirmed,
    Discarded,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NextActionStatus {
    Pending,
    Running,
    Done,
    Blocked,
}

/// Evidence may point to a typed future fact without pulling Blob or Lineage
/// storage into this Slice.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    content = "id",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum EvidenceRef {
    Artifact(ArtifactId),
    Experiment(ExperimentId),
    ExecutionReceipt(ExecutionReceiptId),
}

/// The explicit semantic role asserted for one authoritative evidence
/// reference. A relation is created only by a Research update; derived
/// observations, summaries, replay and model prose cannot infer one.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceRelation {
    Supports,
    Contradicts,
    PositiveControl,
    NegativeControl,
    EnvironmentControl,
    ReachabilityProof,
    PrerequisiteProof,
    ImpactProof,
}

impl EvidenceRelation {
    pub(crate) const fn supports_claim(self) -> bool {
        matches!(
            self,
            Self::Supports | Self::ReachabilityProof | Self::PrerequisiteProof | Self::ImpactProof
        )
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceLink {
    pub relation: EvidenceRelation,
    pub evidence: EvidenceRef,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum ResearchRecordRef {
    Hypothesis(HypothesisId),
    Experiment(ExperimentId),
    Finding(FindingId),
}

/// A closed typed reference to any Research fact that may survive Context
/// compaction. Unlike `ResearchRecordRef`, this includes the target boundary
/// and operator NextAction facts needed by an Evidence Spine.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(
    tag = "kind",
    content = "id",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ResearchFactRef {
    TargetSnapshot(TargetSnapshotId),
    Hypothesis(HypothesisId),
    Experiment(ExperimentId),
    Finding(FindingId),
    NextAction(NextActionId),
}

impl ResearchFactRef {
    pub fn id(&self) -> &str {
        match self {
            Self::TargetSnapshot(id) => id.as_str(),
            Self::Hypothesis(id) => id.as_str(),
            Self::Experiment(id) => id.as_str(),
            Self::Finding(id) => id.as_str(),
            Self::NextAction(id) => id.as_str(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum MergeRecordRef {
    Hypothesis(HypothesisId),
    Finding(FindingId),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TargetSnapshotDraft {
    pub id: TargetSnapshotId,
    pub target_id: String,
    pub kind: TargetKind,
    pub locator: String,
    pub version: String,
    pub build_id: String,
    #[serde(default)]
    pub target_artifact_ids: Vec<ArtifactId>,
    pub environment_fingerprint: String,
    pub device_fingerprint: String,
    pub captured_at: String,
    #[serde(default)]
    pub notes: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TargetSnapshotRecord {
    pub id: TargetSnapshotId,
    pub target_id: String,
    pub kind: TargetKind,
    pub locator: String,
    pub version: String,
    pub build_id: String,
    #[serde(default)]
    pub target_artifact_ids: Vec<ArtifactId>,
    pub environment_fingerprint: String,
    pub device_fingerprint: String,
    pub snapshot_fingerprint: String,
    pub captured_at: String,
    #[serde(default)]
    pub notes: String,
}

impl TargetSnapshotRecord {
    pub fn from_draft(draft: TargetSnapshotDraft) -> Result<Self, ResearchError> {
        let mut record = Self {
            id: draft.id,
            target_id: draft.target_id,
            kind: draft.kind,
            locator: draft.locator,
            version: draft.version,
            build_id: draft.build_id,
            target_artifact_ids: draft.target_artifact_ids,
            environment_fingerprint: draft.environment_fingerprint,
            device_fingerprint: draft.device_fingerprint,
            snapshot_fingerprint: String::new(),
            captured_at: draft.captured_at,
            notes: draft.notes,
        };
        validate_target_snapshot_fields(&record)?;
        record.snapshot_fingerprint = snapshot_fingerprint(&record)?;
        Ok(record)
    }

    pub(crate) fn validate(&self) -> Result<(), ResearchError> {
        validate_target_snapshot_fields(self)?;
        super::fingerprint::validate_snapshot(self)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HypothesisDraft {
    pub id: HypothesisId,
    pub title: String,
    pub target_id: String,
    pub attack_surface: String,
    pub vulnerability_class: String,
    pub entry_point: String,
    pub sensitive_sink: String,
    pub rationale: String,
    pub status: HypothesisStatus,
    pub priority: i32,
    #[serde(default)]
    pub evidence_links: Vec<EvidenceLink>,
    #[serde(default)]
    pub last_action: Option<NextActionId>,
    #[serde(default)]
    pub next_action: Option<NextActionId>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HypothesisRecord {
    pub id: HypothesisId,
    pub title: String,
    pub target_id: String,
    pub attack_surface: String,
    pub vulnerability_class: String,
    pub entry_point: String,
    pub sensitive_sink: String,
    pub dedup_key: String,
    pub rationale: String,
    pub status: HypothesisStatus,
    pub priority: i32,
    #[serde(default)]
    pub evidence_links: Vec<EvidenceLink>,
    #[serde(default)]
    pub last_action: Option<NextActionId>,
    #[serde(default)]
    pub next_action: Option<NextActionId>,
    #[serde(default)]
    pub merged_into: Option<MergeRecordRef>,
}

impl HypothesisRecord {
    pub fn from_draft(draft: HypothesisDraft) -> Result<Self, ResearchError> {
        let dedup_key = dedup_key(
            &draft.target_id,
            &draft.vulnerability_class,
            &draft.entry_point,
            &draft.sensitive_sink,
        )?;
        let record = Self {
            id: draft.id,
            title: draft.title,
            target_id: draft.target_id,
            attack_surface: draft.attack_surface,
            vulnerability_class: draft.vulnerability_class,
            entry_point: draft.entry_point,
            sensitive_sink: draft.sensitive_sink,
            dedup_key,
            rationale: draft.rationale,
            status: draft.status,
            priority: draft.priority,
            evidence_links: draft.evidence_links,
            last_action: draft.last_action,
            next_action: draft.next_action,
            merged_into: None,
        };
        record.validate()?;
        Ok(record)
    }

    pub(crate) fn validate(&self) -> Result<(), ResearchError> {
        validate_hypothesis_fields(self)?;
        super::fingerprint::validate_dedup(self)
    }
}

/// An append-only change to mutable Hypothesis research state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HypothesisUpdate {
    pub id: HypothesisId,
    #[serde(default)]
    pub status: Option<HypothesisStatus>,
    #[serde(default)]
    pub evidence_links_add: Vec<EvidenceLink>,
    #[serde(default)]
    pub last_action: Option<NextActionId>,
    #[serde(default)]
    pub next_action: Option<NextActionId>,
}

impl HypothesisUpdate {
    pub(crate) fn validate(&self) -> Result<(), ResearchError> {
        require_id("HypothesisUpdate id", self.id.as_str())?;
        validate_evidence_links(&self.evidence_links_add)?;
        if self.status.is_none()
            && self.evidence_links_add.is_empty()
            && self.last_action.is_none()
            && self.next_action.is_none()
        {
            return Err(ResearchError::InvalidRecord {
                message: "HypothesisUpdate must change status, evidence, or action linkage"
                    .to_owned(),
            });
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExperimentDraft {
    pub id: ExperimentId,
    pub hypothesis_id: HypothesisId,
    pub target_snapshot_id: TargetSnapshotId,
    pub capsule_id: CapsuleId,
    pub goal: String,
    pub result: ExperimentResult,
    #[serde(default)]
    pub artifact_refs: Vec<ArtifactId>,
    #[serde(default)]
    pub notes: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExperimentRecord {
    pub id: ExperimentId,
    pub hypothesis_id: HypothesisId,
    pub target_snapshot_id: TargetSnapshotId,
    pub capsule_id: CapsuleId,
    pub goal: String,
    pub action_fingerprint: String,
    pub result: ExperimentResult,
    #[serde(default)]
    pub artifact_refs: Vec<ArtifactId>,
    #[serde(default)]
    pub notes: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExperimentCapsuleDraft {
    pub id: CapsuleId,
    pub experiment_id: ExperimentId,
    pub target_snapshot_id: TargetSnapshotId,
    pub action: String,
    #[serde(default)]
    pub arguments: Value,
    pub tool_version: String,
    #[serde(default)]
    pub execution_ids: Vec<ExecutionId>,
    #[serde(default)]
    pub input_artifact_ids: Vec<ArtifactId>,
    #[serde(default)]
    pub environment_delta: String,
    #[serde(default)]
    pub account_prerequisites: Vec<String>,
    pub toolchain_fingerprint: String,
    #[serde(default)]
    pub execution_receipt_ids: Vec<ExecutionReceiptId>,
    #[serde(default)]
    pub output_artifact_ids: Vec<ArtifactId>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExperimentCapsuleRecord {
    pub id: CapsuleId,
    pub experiment_id: ExperimentId,
    pub target_snapshot_id: TargetSnapshotId,
    pub normalized_action: String,
    #[serde(default)]
    pub normalized_arguments: Value,
    pub tool_version: String,
    #[serde(default)]
    pub execution_ids: Vec<ExecutionId>,
    #[serde(default)]
    pub input_artifact_ids: Vec<ArtifactId>,
    #[serde(default)]
    pub environment_delta: String,
    #[serde(default)]
    pub account_prerequisites: Vec<String>,
    pub toolchain_fingerprint: String,
    #[serde(default)]
    pub execution_receipt_ids: Vec<ExecutionReceiptId>,
    #[serde(default)]
    pub output_artifact_ids: Vec<ArtifactId>,
}

impl ExperimentRecord {
    pub fn from_drafts(
        experiment: ExperimentDraft,
        capsule: ExperimentCapsuleDraft,
    ) -> Result<(Self, ExperimentCapsuleRecord), ResearchError> {
        let capsule = ExperimentCapsuleRecord::from_draft(capsule)?;
        let mut record = Self {
            id: experiment.id,
            hypothesis_id: experiment.hypothesis_id,
            target_snapshot_id: experiment.target_snapshot_id,
            capsule_id: experiment.capsule_id,
            goal: experiment.goal,
            action_fingerprint: String::new(),
            result: experiment.result,
            artifact_refs: experiment.artifact_refs,
            notes: experiment.notes,
        };
        validate_experiment_fields(&record, &capsule)?;
        record.action_fingerprint = action_fingerprint(&record, &capsule)?;
        Ok((record, capsule))
    }

    pub(crate) fn validate(&self, capsule: &ExperimentCapsuleRecord) -> Result<(), ResearchError> {
        validate_experiment_fields(self, capsule)?;
        super::fingerprint::validate_action(self, capsule)
    }
}

impl ExperimentCapsuleRecord {
    fn from_draft(draft: ExperimentCapsuleDraft) -> Result<Self, ResearchError> {
        let record = Self {
            id: draft.id,
            experiment_id: draft.experiment_id,
            target_snapshot_id: draft.target_snapshot_id,
            normalized_action: normalize_action(&draft.action),
            normalized_arguments: draft.arguments,
            tool_version: draft.tool_version,
            execution_ids: draft.execution_ids,
            input_artifact_ids: draft.input_artifact_ids,
            environment_delta: draft.environment_delta,
            account_prerequisites: draft.account_prerequisites,
            toolchain_fingerprint: draft.toolchain_fingerprint,
            execution_receipt_ids: draft.execution_receipt_ids,
            output_artifact_ids: draft.output_artifact_ids,
        };
        validate_capsule_fields(&record)?;
        Ok(record)
    }

    pub(crate) fn validate(&self) -> Result<(), ResearchError> {
        validate_capsule_fields(self)
    }
}

/// An append-only change to an Experiment result or its resulting references.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExperimentUpdate {
    pub id: ExperimentId,
    #[serde(default)]
    pub result: Option<ExperimentResult>,
    #[serde(default)]
    pub artifact_refs_add: Vec<ArtifactId>,
    #[serde(default)]
    pub notes_append: Option<String>,
}

impl ExperimentUpdate {
    pub(crate) fn validate(&self) -> Result<(), ResearchError> {
        require_id("ExperimentUpdate id", self.id.as_str())?;
        validate_artifact_ids(&self.artifact_refs_add)?;
        if let Some(notes) = &self.notes_append {
            require_text("ExperimentUpdate notes_append", notes)?;
        }
        if self.result.is_none() && self.artifact_refs_add.is_empty() && self.notes_append.is_none()
        {
            return Err(ResearchError::InvalidRecord {
                message: "ExperimentUpdate must change result, artifacts, or notes".to_owned(),
            });
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FindingDraft {
    pub id: FindingId,
    pub title: String,
    pub status: FindingStatus,
    pub target_id: String,
    pub target_version: String,
    pub vulnerability_class: String,
    pub entry_point: String,
    pub sensitive_sink: String,
    #[serde(default)]
    pub target_snapshot_ids: Vec<TargetSnapshotId>,
    #[serde(default)]
    pub prerequisites: Vec<String>,
    pub impact: String,
    #[serde(default)]
    pub evidence_links: Vec<EvidenceLink>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FindingRecord {
    pub id: FindingId,
    pub title: String,
    pub status: FindingStatus,
    pub target_id: String,
    pub target_version: String,
    pub vulnerability_class: String,
    pub entry_point: String,
    pub sensitive_sink: String,
    pub dedup_key: String,
    #[serde(default)]
    pub target_snapshot_ids: Vec<TargetSnapshotId>,
    #[serde(default)]
    pub prerequisites: Vec<String>,
    pub impact: String,
    #[serde(default)]
    pub evidence_links: Vec<EvidenceLink>,
    #[serde(default)]
    pub merged_into: Option<MergeRecordRef>,
}

impl FindingRecord {
    pub fn from_draft(draft: FindingDraft) -> Result<Self, ResearchError> {
        let dedup_key = dedup_key(
            &draft.target_id,
            &draft.vulnerability_class,
            &draft.entry_point,
            &draft.sensitive_sink,
        )?;
        let record = Self {
            id: draft.id,
            title: draft.title,
            status: draft.status,
            target_id: draft.target_id,
            target_version: draft.target_version,
            vulnerability_class: draft.vulnerability_class,
            entry_point: draft.entry_point,
            sensitive_sink: draft.sensitive_sink,
            dedup_key,
            target_snapshot_ids: draft.target_snapshot_ids,
            prerequisites: draft.prerequisites,
            impact: draft.impact,
            evidence_links: draft.evidence_links,
            merged_into: None,
        };
        record.validate()?;
        Ok(record)
    }

    pub(crate) fn validate(&self) -> Result<(), ResearchError> {
        validate_finding_fields(self)?;
        super::fingerprint::validate_finding_dedup(self)
    }
}

/// An append-only change to Finding research state and evidence.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FindingUpdate {
    pub id: FindingId,
    #[serde(default)]
    pub status: Option<FindingStatus>,
    #[serde(default)]
    pub target_snapshot_ids_add: Vec<TargetSnapshotId>,
    #[serde(default)]
    pub prerequisites_add: Vec<String>,
    #[serde(default)]
    pub evidence_links_add: Vec<EvidenceLink>,
    #[serde(default)]
    pub impact: Option<String>,
}

impl FindingUpdate {
    pub(crate) fn validate(&self) -> Result<(), ResearchError> {
        require_id("FindingUpdate id", self.id.as_str())?;
        validate_evidence_links(&self.evidence_links_add)?;
        if let Some(impact) = &self.impact {
            require_text("FindingUpdate impact", impact)?;
        }
        if self.status.is_none()
            && self.target_snapshot_ids_add.is_empty()
            && self.prerequisites_add.is_empty()
            && self.evidence_links_add.is_empty()
            && self.impact.is_none()
        {
            return Err(ResearchError::InvalidRecord {
                message: "FindingUpdate must change status, references, prerequisites, or impact"
                    .to_owned(),
            });
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NextActionDraft {
    pub id: NextActionId,
    pub research_id: ResearchRecordRef,
    pub priority: i32,
    pub action: String,
    pub status: NextActionStatus,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NextActionRecord {
    pub id: NextActionId,
    pub research_id: ResearchRecordRef,
    pub priority: i32,
    pub action: String,
    pub status: NextActionStatus,
}

impl NextActionRecord {
    pub fn from_draft(draft: NextActionDraft) -> Result<Self, ResearchError> {
        let record = Self {
            id: draft.id,
            research_id: draft.research_id,
            priority: draft.priority,
            action: draft.action,
            status: draft.status,
        };
        record.validate()?;
        Ok(record)
    }

    pub(crate) fn validate(&self) -> Result<(), ResearchError> {
        require_id("NextAction id", self.id.as_str())?;
        require_text("NextAction action", &self.action)
    }
}

/// An append-only change to a NextAction lifecycle state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NextActionUpdate {
    pub id: NextActionId,
    #[serde(default)]
    pub priority: Option<i32>,
    #[serde(default)]
    pub action: Option<String>,
    #[serde(default)]
    pub status: Option<NextActionStatus>,
}

impl NextActionUpdate {
    pub(crate) fn validate(&self) -> Result<(), ResearchError> {
        require_id("NextActionUpdate id", self.id.as_str())?;
        if let Some(action) = &self.action {
            require_text("NextActionUpdate action", action)?;
        }
        if self.priority.is_none() && self.action.is_none() && self.status.is_none() {
            return Err(ResearchError::InvalidRecord {
                message: "NextActionUpdate must change priority, action, or status".to_owned(),
            });
        }
        Ok(())
    }
}

fn validate_target_snapshot_fields(record: &TargetSnapshotRecord) -> Result<(), ResearchError> {
    require_id("TargetSnapshot id", record.id.as_str())?;
    require_text("TargetSnapshot target_id", &record.target_id)?;
    require_text("TargetSnapshot locator", &record.locator)?;
    require_text("TargetSnapshot version", &record.version)?;
    require_text("TargetSnapshot build_id", &record.build_id)?;
    require_text(
        "TargetSnapshot environment_fingerprint",
        &record.environment_fingerprint,
    )?;
    require_text(
        "TargetSnapshot device_fingerprint",
        &record.device_fingerprint,
    )?;
    require_text("TargetSnapshot captured_at", &record.captured_at)?;
    validate_artifact_ids(&record.target_artifact_ids)
}

fn validate_hypothesis_fields(record: &HypothesisRecord) -> Result<(), ResearchError> {
    require_id("Hypothesis id", record.id.as_str())?;
    require_text("Hypothesis title", &record.title)?;
    require_text("Hypothesis target_id", &record.target_id)?;
    require_text("Hypothesis attack_surface", &record.attack_surface)?;
    require_text(
        "Hypothesis vulnerability_class",
        &record.vulnerability_class,
    )?;
    require_text("Hypothesis entry_point", &record.entry_point)?;
    require_text("Hypothesis sensitive_sink", &record.sensitive_sink)?;
    require_text("Hypothesis rationale", &record.rationale)?;
    validate_evidence_links(&record.evidence_links)
}

fn validate_experiment_fields(
    record: &ExperimentRecord,
    capsule: &ExperimentCapsuleRecord,
) -> Result<(), ResearchError> {
    require_id("Experiment id", record.id.as_str())?;
    require_id("Experiment hypothesis_id", record.hypothesis_id.as_str())?;
    require_id(
        "Experiment target_snapshot_id",
        record.target_snapshot_id.as_str(),
    )?;
    require_id("Experiment capsule_id", record.capsule_id.as_str())?;
    require_text("Experiment goal", &record.goal)?;
    validate_artifact_ids(&record.artifact_refs)?;
    capsule.validate()?;
    if capsule.experiment_id != record.id
        || capsule.target_snapshot_id != record.target_snapshot_id
        || capsule.id != record.capsule_id
    {
        return Err(ResearchError::InvalidRecord {
            message: "Experiment and ExperimentCapsule identifiers do not agree".to_owned(),
        });
    }
    Ok(())
}

fn validate_capsule_fields(record: &ExperimentCapsuleRecord) -> Result<(), ResearchError> {
    require_id("ExperimentCapsule id", record.id.as_str())?;
    require_id(
        "ExperimentCapsule experiment_id",
        record.experiment_id.as_str(),
    )?;
    require_id(
        "ExperimentCapsule target_snapshot_id",
        record.target_snapshot_id.as_str(),
    )?;
    require_text(
        "ExperimentCapsule normalized_action",
        &record.normalized_action,
    )?;
    require_text("ExperimentCapsule tool_version", &record.tool_version)?;
    require_text(
        "ExperimentCapsule toolchain_fingerprint",
        &record.toolchain_fingerprint,
    )?;
    validate_execution_ids(&record.execution_ids)?;
    validate_artifact_ids(&record.input_artifact_ids)?;
    validate_receipt_ids(&record.execution_receipt_ids)?;
    validate_artifact_ids(&record.output_artifact_ids)
}

fn validate_finding_fields(record: &FindingRecord) -> Result<(), ResearchError> {
    require_id("Finding id", record.id.as_str())?;
    require_text("Finding title", &record.title)?;
    require_text("Finding target_id", &record.target_id)?;
    require_text("Finding target_version", &record.target_version)?;
    require_text("Finding vulnerability_class", &record.vulnerability_class)?;
    require_text("Finding entry_point", &record.entry_point)?;
    require_text("Finding sensitive_sink", &record.sensitive_sink)?;
    require_text("Finding impact", &record.impact)?;
    validate_evidence_links(&record.evidence_links)?;
    if matches!(
        record.status,
        FindingStatus::Reproduced | FindingStatus::Confirmed
    ) && (record.target_snapshot_ids.is_empty()
        || !record
            .evidence_links
            .iter()
            .any(|link| link.relation.supports_claim()))
    {
        return Err(ResearchError::InvalidRecord {
            message: "reproduced or confirmed Finding requires TargetSnapshot and claim-supporting EvidenceLink"
                .to_owned(),
        });
    }
    Ok(())
}

fn validate_artifact_ids(ids: &[ArtifactId]) -> Result<(), ResearchError> {
    for id in ids {
        require_id("Artifact reference", id.as_str())?;
    }
    Ok(())
}

fn validate_execution_ids(ids: &[ExecutionId]) -> Result<(), ResearchError> {
    for id in ids {
        require_id("Execution reference", id.as_str())?;
    }
    Ok(())
}

fn validate_receipt_ids(ids: &[ExecutionReceiptId]) -> Result<(), ResearchError> {
    for id in ids {
        require_id("ExecutionReceipt reference", id.as_str())?;
    }
    Ok(())
}

pub(crate) fn validate_evidence_refs(refs: &[EvidenceRef]) -> Result<(), ResearchError> {
    for reference in refs {
        match reference {
            EvidenceRef::Artifact(id) => require_id("Artifact evidence reference", id.as_str())?,
            EvidenceRef::Experiment(id) => {
                require_id("Experiment evidence reference", id.as_str())?
            }
            EvidenceRef::ExecutionReceipt(id) => {
                require_id("ExecutionReceipt evidence reference", id.as_str())?
            }
        }
    }
    Ok(())
}

pub(crate) fn validate_evidence_links(links: &[EvidenceLink]) -> Result<(), ResearchError> {
    for (index, link) in links.iter().enumerate() {
        validate_evidence_refs(std::slice::from_ref(&link.evidence))?;
        if links[..index].contains(link) {
            return Err(ResearchError::InvalidRecord {
                message: format!("duplicate EvidenceLink `{link:?}`"),
            });
        }
    }
    Ok(())
}

pub(crate) fn require_id(subject: &str, value: &str) -> Result<(), ResearchError> {
    require_text(subject, value)
}

pub(crate) fn require_text(subject: &str, value: &str) -> Result<(), ResearchError> {
    if value.trim().is_empty() {
        Err(ResearchError::InvalidRecord {
            message: format!("{subject} must not be empty"),
        })
    } else {
        Ok(())
    }
}
