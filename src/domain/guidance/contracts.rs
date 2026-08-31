use serde::Serialize;
use serde_json::Value;
use thiserror::Error;

use crate::domain::artifacts::ArtifactId;
use crate::domain::research::{
    EvidenceGapKind, EvidenceGapReasonCode, EvidenceGapState, HypothesisId, NextActionStatus,
    ResearchRecordRef,
};

pub(super) const MAX_EVAL_CANDIDATES: usize = 128;
pub(super) const MAX_CANDIDATE_ID_BYTES: usize = 128;
pub(super) const MAX_REFERENCE_ID_BYTES: usize = 128;
pub(super) const MAX_DECLARED_GAP_CLOSURES: usize = 12;
pub(super) const MAX_ACTION_IDENTITY_STRING_BYTES: usize = 4 * 1024;
pub(super) const MAX_ACTION_IDENTITY_INPUT_ARTIFACTS: usize = 24;
pub(super) const MAX_ACTION_IDENTITY_PREREQUISITES: usize = 24;
pub(super) const MAX_ACTION_IDENTITY_VALUE_DEPTH: usize = 12;
pub(super) const MAX_ACTION_IDENTITY_VALUE_NODES: usize = 128;
pub(super) const MAX_ACTION_IDENTITY_VALUE_COLLECTION: usize = 32;

/// Typed controlled cases supplied only to the read-only guidance projection.
/// This is neither a Research Event nor a persisted candidate registry.
#[derive(Clone, Debug)]
pub(crate) struct GuidanceEvalCandidates {
    pub(crate) candidates: Vec<GuidanceEvalCandidate>,
}

impl GuidanceEvalCandidates {
    pub(crate) fn empty() -> Self {
        Self {
            candidates: Vec::new(),
        }
    }
}

/// One complete executable-action preimage. The Guidance CLI receives a
/// strict wire DTO and converts it into this typed Domain value before any
/// rank calculation. The resulting action fingerprint is always computed,
/// never caller-declared.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct GuidanceActionIdentityWitness {
    pub(crate) hypothesis_id: HypothesisId,
    pub(crate) normalized_action: String,
    pub(crate) normalized_arguments: Value,
    pub(crate) tool_version: String,
    pub(crate) input_artifact_ids: Vec<ArtifactId>,
    pub(crate) environment_delta: String,
    pub(crate) account_prerequisites: Vec<String>,
    pub(crate) toolchain_fingerprint: String,
}

/// A controlled candidate may carry only typed factors and an executable
/// action witness. There is intentionally no action prose or caller-provided
/// fingerprint: neither establishes an Experiment action identity.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct GuidanceEvalCandidate {
    pub(crate) id: String,
    pub(crate) research_id: ResearchRecordRef,
    pub(crate) target_snapshot_id: crate::domain::research::TargetSnapshotId,
    pub(crate) action_identity: GuidanceActionIdentityWitness,
    pub(crate) validity: CandidateValidity,
    pub(crate) prerequisite: PrerequisiteState,
    pub(crate) boundary_importance: u8,
    pub(crate) competing_hypothesis_discrimination: u8,
    /// Each listed kind is a caller-declared critical gap the candidate is
    /// expected to close for `research_id`; the typed gap projection verifies
    /// the declaration before it receives a score.
    pub(crate) expected_gap_closures: Vec<EvidenceGapKind>,
    pub(crate) reproducibility: u8,
    pub(crate) execution_cost: u8,
    pub(crate) mutation_risk: u8,
    pub(crate) version_drift_risk: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub(crate) enum CandidateValidity {
    Valid,
    Invalid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub(crate) enum PrerequisiteState {
    Satisfied,
    Missing,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum GuidanceFactorName {
    BoundaryImportance,
    CompetingHypothesisDiscrimination,
    ExpectedGapClosure,
    Reproducibility,
    NoveltyVsActionFingerprint,
    TargetSnapshotApplicability,
    ExecutionCost,
    MutationRisk,
    VersionDriftRisk,
}

impl GuidanceFactorName {
    pub(super) const ORDER: [Self; 9] = [
        Self::BoundaryImportance,
        Self::CompetingHypothesisDiscrimination,
        Self::ExpectedGapClosure,
        Self::Reproducibility,
        Self::NoveltyVsActionFingerprint,
        Self::TargetSnapshotApplicability,
        Self::ExecutionCost,
        Self::MutationRisk,
        Self::VersionDriftRisk,
    ];
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum GuidanceValueState {
    Available,
    Missing,
    Unknown,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum GuidanceReasonCode {
    CallerSuppliedControlledFactor,
    ComputedControlledActionFingerprint,
    NextActionFieldUnavailable,
    NextActionHasNoExperimentActionFingerprint,
    NextActionLifecycleDone,
    NextActionLifecycleBlocked,
    NextActionLifecycleRunning,
    CandidateExplicitlyInvalid,
    CandidatePrerequisiteSatisfied,
    CandidatePrerequisiteMissing,
    CandidatePrerequisiteUnknown,
    CandidateActionFingerprintNovel,
    PriorActionFingerprintExists,
    DuplicateCandidateActionFingerprint,
    CandidateTargetSnapshotDoesNotMatchCurrent,
    CandidateTargetSnapshotMissing,
    ExistingReferenceAppliesToCurrentSnapshot,
    ExistingReferenceTargetIncompatible,
    ExistingReferenceNotSnapshotBound,
    ExistingReferenceMissing,
    DeclaredCriticalGapMissing,
    DeclaredCriticalGapUnknown,
    DeclaredCriticalGapNotPresent,
    DuplicateDeclaredCriticalGap,
    NoDeclaredCriticalGap,
    ResearchReferenceHasNoEvidenceGapReport,
    IncompleteFactorVectorConservativeTier,
    DeterministicCandidateIdentityTieBreak,
    ActualOutcomeMetricUnavailable,
    CostEffectivenessMetricUnavailable,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(super) struct GuidanceFactor {
    pub(super) name: GuidanceFactorName,
    pub(super) state: GuidanceValueState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) value: Option<u8>,
    pub(super) reason_codes: Vec<GuidanceReasonCode>,
}

impl GuidanceFactor {
    pub(super) fn available(
        name: GuidanceFactorName,
        value: u8,
        reason: GuidanceReasonCode,
    ) -> Self {
        Self {
            name,
            state: GuidanceValueState::Available,
            value: Some(value),
            reason_codes: vec![reason],
        }
    }

    pub(super) fn unavailable(
        name: GuidanceFactorName,
        state: GuidanceValueState,
        reason: GuidanceReasonCode,
    ) -> Self {
        Self {
            name,
            state,
            value: None,
            reason_codes: vec![reason],
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(super) struct GuidanceActionFingerprint {
    pub(super) state: GuidanceValueState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) value: Option<String>,
    pub(super) reason_codes: Vec<GuidanceReasonCode>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(super) struct GuidanceGapSummary {
    pub(super) kind: EvidenceGapKind,
    pub(super) state: EvidenceGapState,
    pub(super) reason_code: EvidenceGapReasonCode,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(super) struct GapClosureSummary {
    pub(super) evidence_gap_report_available: bool,
    pub(super) declared_critical_gap_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) matched_missing_gap_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) total_missing_gap_count: Option<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(super) struct GuidancePrerequisite {
    pub(super) state: GuidanceValueState,
    pub(super) reason_codes: Vec<GuidanceReasonCode>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum GuidanceCandidateSource {
    ExistingNextAction,
    ControlledEvalCandidate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum GuidanceRankingTier {
    CompleteFactorVector,
    ConservativeIncompleteFactorVector,
    Excluded,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(super) struct GuidanceCandidateReport {
    pub(super) candidate_identity: String,
    pub(super) source: GuidanceCandidateSource,
    pub(super) source_id: String,
    pub(super) research_id: ResearchRecordRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) next_action_status: Option<NextActionStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) source_priority: Option<i32>,
    pub(super) action_fingerprint: GuidanceActionFingerprint,
    pub(super) prerequisite: GuidancePrerequisite,
    pub(super) associated_gaps: Vec<GuidanceGapSummary>,
    pub(super) declared_gap_closure: GapClosureSummary,
    pub(super) factors: Vec<GuidanceFactor>,
    pub(super) exclusion_reasons: Vec<GuidanceReasonCode>,
    pub(super) ranking_tier: GuidanceRankingTier,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) rank: Option<usize>,
    pub(super) tie_break: GuidanceReasonCode,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(super) struct UnavailableMetric {
    pub(super) state: GuidanceValueState,
    pub(super) reason_code: GuidanceReasonCode,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(super) struct GuidanceDeferredMetrics {
    pub(super) actual_critical_gap_closure: UnavailableMetric,
    pub(super) cost_effectiveness: UnavailableMetric,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct GuidanceShadowReport {
    algorithm_version: &'static str,
    input_fingerprint: String,
    current_target_snapshot: String,
    considered_candidate_count: usize,
    ranked_candidate_count: usize,
    candidates: Vec<GuidanceCandidateReport>,
    deferred_metrics: GuidanceDeferredMetrics,
}

impl GuidanceShadowReport {
    pub(super) fn new(
        input_fingerprint: String,
        current_target_snapshot: String,
        candidates: Vec<GuidanceCandidateReport>,
    ) -> Self {
        let ranked_candidate_count = candidates
            .iter()
            .filter(|report| report.rank.is_some())
            .count();
        Self {
            algorithm_version: super::ALGORITHM_VERSION,
            input_fingerprint,
            current_target_snapshot,
            considered_candidate_count: candidates.len(),
            ranked_candidate_count,
            candidates,
            deferred_metrics: GuidanceDeferredMetrics {
                actual_critical_gap_closure: unavailable_metric(
                    GuidanceReasonCode::ActualOutcomeMetricUnavailable,
                ),
                cost_effectiveness: unavailable_metric(
                    GuidanceReasonCode::CostEffectivenessMetricUnavailable,
                ),
            },
        }
    }
}

pub(super) fn unavailable_metric(reason_code: GuidanceReasonCode) -> UnavailableMetric {
    UnavailableMetric {
        state: GuidanceValueState::Unavailable,
        reason_code,
    }
}

#[derive(Debug, Error)]
pub(crate) enum GuidanceError {
    #[error("guidance current TargetSnapshot `{id}` does not exist")]
    CurrentTargetMissing { id: String },
    #[error(
        "guidance received {actual} controlled candidates, above the {maximum}-candidate limit"
    )]
    TooManyEvalCandidates { actual: usize, maximum: usize },
    #[error(
        "guidance controlled candidate ID must be non-empty, at most {maximum} bytes, and contain only ASCII letters, digits, `-`, `_`, or `.`"
    )]
    InvalidCandidateId { maximum: usize },
    #[error("guidance controlled candidate ID `{id}` is duplicated")]
    DuplicateCandidateId { id: String },
    #[error("guidance candidate `{id}` factor `{factor}` exceeds the 0..=100 bound")]
    FactorOutOfBounds { id: String, factor: &'static str },
    #[error("guidance candidate `{id}` references an identifier above the {maximum}-byte bound")]
    ReferenceTooLarge { id: String, maximum: usize },
    #[error(
        "guidance candidate `{id}` declares {actual} critical gaps, above the {maximum}-gap bound"
    )]
    TooManyDeclaredGapClosures {
        id: String,
        actual: usize,
        maximum: usize,
    },
    #[error(
        "guidance candidate `{id}` action identity field `{field}` must be non-empty and at most {maximum} bytes"
    )]
    InvalidActionIdentityString {
        id: String,
        field: &'static str,
        maximum: usize,
    },
    #[error(
        "guidance candidate `{id}` action identity array `{field}` has {actual} entries, above the {maximum}-entry bound"
    )]
    ActionIdentityArrayTooLarge {
        id: String,
        field: &'static str,
        actual: usize,
        maximum: usize,
    },
    #[error("guidance candidate `{id}` action identity arguments exceed the bounded JSON shape")]
    ActionIdentityArgumentsTooLarge { id: String },
    #[error("guidance candidate `{id}` action identity cannot be fingerprinted: {message}")]
    ActionIdentityFingerprint { id: String, message: String },
    #[error("failed to fingerprint deterministic guidance input: {message}")]
    InputFingerprint { message: String },
}
