//! Deterministic non-evidence projection over typed Research facts.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::records::{
    EvidenceLink, EvidenceRef, EvidenceRelation, FindingRecord, FindingStatus, HypothesisRecord,
    HypothesisStatus, ResearchRecordRef,
};
use super::reducer::{ResearchReferenceCatalog, ResearchState};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EvidenceGapKind {
    EntryUnproven,
    SinkUnproven,
    ReachabilityUnproven,
    PrerequisiteUnproven,
    PositiveControlMissing,
    NegativeControlMissing,
    EnvironmentControlMissing,
    ImpactUnproven,
    RepeatabilityMissing,
    TargetOrVersionBindingMissing,
    ArtifactOrReceiptProvenanceMissing,
    ContradictionUnresolved,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EvidenceGapState {
    Missing,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EvidenceGapReasonCode {
    EntryReachabilityProofAbsent,
    SinkReachabilityProofAbsent,
    ReachabilityProofAbsent,
    PrerequisiteFactUnavailable,
    PrerequisiteProofAbsent,
    PositiveControlAbsent,
    NegativeControlAbsent,
    EnvironmentControlAbsent,
    ImpactFactUnavailable,
    ImpactProofAbsent,
    FewerThanTwoPositiveExperiments,
    TargetSnapshotBindingAbsent,
    ArtifactOrReceiptReferenceAbsent,
    ContradictionResolutionUnmodeled,
}

struct EvidenceGapSubject<'a> {
    links: &'a [EvidenceLink],
    target_id: &'a str,
    target_version: Option<&'a str>,
    prerequisites_declared: Option<bool>,
    impact_declared: Option<bool>,
    has_target_binding: bool,
    contradiction_resolved: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EvidenceGap {
    kind: EvidenceGapKind,
    state: EvidenceGapState,
    reason_code: EvidenceGapReasonCode,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EvidenceGapReport {
    research_id: ResearchRecordRef,
    gaps: Vec<EvidenceGap>,
}

impl EvidenceGapReport {
    pub(crate) fn research_id(&self) -> &ResearchRecordRef {
        &self.research_id
    }

    pub(crate) fn gaps(&self) -> &[EvidenceGap] {
        &self.gaps
    }
}

impl EvidenceGap {
    pub(crate) const fn kind(&self) -> EvidenceGapKind {
        self.kind
    }

    pub(crate) const fn state(&self) -> EvidenceGapState {
        self.state
    }

    pub(crate) const fn reason_code(&self) -> EvidenceGapReasonCode {
        self.reason_code
    }
}

pub(super) fn project(
    state: &ResearchState,
    references: &ResearchReferenceCatalog,
) -> Vec<EvidenceGapReport> {
    state
        .hypothesis_records()
        .map(|record| project_hypothesis(state, references, record))
        .chain(
            state
                .finding_records()
                .map(|record| project_finding(state, references, record)),
        )
        .collect()
}

fn project_hypothesis(
    state: &ResearchState,
    references: &ResearchReferenceCatalog,
    record: &HypothesisRecord,
) -> EvidenceGapReport {
    let has_target_binding = record.evidence_links.iter().any(|link| {
        state.evidence_reference_applies(references, &link.evidence, &record.target_id, None)
    });
    EvidenceGapReport {
        research_id: ResearchRecordRef::Hypothesis(record.id.clone()),
        gaps: project_gaps(
            state,
            references,
            EvidenceGapSubject {
                links: &record.evidence_links,
                target_id: &record.target_id,
                target_version: None,
                prerequisites_declared: None,
                impact_declared: None,
                has_target_binding,
                contradiction_resolved: record.status == HypothesisStatus::Rejected,
            },
        ),
    }
}

fn project_finding(
    state: &ResearchState,
    references: &ResearchReferenceCatalog,
    record: &FindingRecord,
) -> EvidenceGapReport {
    let has_target_binding = record.target_snapshot_ids.iter().any(|id| {
        state.target_snapshot_applies(id, &record.target_id, Some(&record.target_version))
    });
    EvidenceGapReport {
        research_id: ResearchRecordRef::Finding(record.id.clone()),
        gaps: project_gaps(
            state,
            references,
            EvidenceGapSubject {
                links: &record.evidence_links,
                target_id: &record.target_id,
                target_version: Some(&record.target_version),
                prerequisites_declared: Some(!record.prerequisites.is_empty()),
                impact_declared: Some(true),
                has_target_binding,
                contradiction_resolved: record.status == FindingStatus::Discarded,
            },
        ),
    }
}

fn project_gaps(
    state: &ResearchState,
    references: &ResearchReferenceCatalog,
    subject: EvidenceGapSubject<'_>,
) -> Vec<EvidenceGap> {
    let EvidenceGapSubject {
        links,
        target_id,
        target_version,
        prerequisites_declared,
        impact_declared,
        has_target_binding,
        contradiction_resolved,
    } = subject;
    let mut gaps = Vec::new();
    if !has_relation(links, EvidenceRelation::ReachabilityProof) {
        gaps.push(missing(
            EvidenceGapKind::EntryUnproven,
            EvidenceGapReasonCode::EntryReachabilityProofAbsent,
        ));
        gaps.push(missing(
            EvidenceGapKind::SinkUnproven,
            EvidenceGapReasonCode::SinkReachabilityProofAbsent,
        ));
        gaps.push(missing(
            EvidenceGapKind::ReachabilityUnproven,
            EvidenceGapReasonCode::ReachabilityProofAbsent,
        ));
    }
    if !has_relation(links, EvidenceRelation::PrerequisiteProof) {
        gaps.push(match prerequisites_declared {
            Some(true) => missing(
                EvidenceGapKind::PrerequisiteUnproven,
                EvidenceGapReasonCode::PrerequisiteProofAbsent,
            ),
            Some(false) | None => unknown(
                EvidenceGapKind::PrerequisiteUnproven,
                EvidenceGapReasonCode::PrerequisiteFactUnavailable,
            ),
        });
    }
    for (relation, kind, reason_code) in [
        (
            EvidenceRelation::PositiveControl,
            EvidenceGapKind::PositiveControlMissing,
            EvidenceGapReasonCode::PositiveControlAbsent,
        ),
        (
            EvidenceRelation::NegativeControl,
            EvidenceGapKind::NegativeControlMissing,
            EvidenceGapReasonCode::NegativeControlAbsent,
        ),
        (
            EvidenceRelation::EnvironmentControl,
            EvidenceGapKind::EnvironmentControlMissing,
            EvidenceGapReasonCode::EnvironmentControlAbsent,
        ),
    ] {
        if !has_relation(links, relation) {
            gaps.push(missing(kind, reason_code));
        }
    }
    if !has_relation(links, EvidenceRelation::ImpactProof) {
        gaps.push(match impact_declared {
            Some(true) => missing(
                EvidenceGapKind::ImpactUnproven,
                EvidenceGapReasonCode::ImpactProofAbsent,
            ),
            Some(false) | None => unknown(
                EvidenceGapKind::ImpactUnproven,
                EvidenceGapReasonCode::ImpactFactUnavailable,
            ),
        });
    }
    if positive_experiment_count(state, links, target_id, target_version) < 2 {
        gaps.push(missing(
            EvidenceGapKind::RepeatabilityMissing,
            EvidenceGapReasonCode::FewerThanTwoPositiveExperiments,
        ));
    }
    if !has_target_binding {
        gaps.push(missing(
            EvidenceGapKind::TargetOrVersionBindingMissing,
            EvidenceGapReasonCode::TargetSnapshotBindingAbsent,
        ));
    }
    if !has_artifact_or_receipt_provenance(state, references, links, target_id, target_version) {
        gaps.push(missing(
            EvidenceGapKind::ArtifactOrReceiptProvenanceMissing,
            EvidenceGapReasonCode::ArtifactOrReceiptReferenceAbsent,
        ));
    }
    if has_relation(links, EvidenceRelation::Contradicts) && !contradiction_resolved {
        gaps.push(unknown(
            EvidenceGapKind::ContradictionUnresolved,
            EvidenceGapReasonCode::ContradictionResolutionUnmodeled,
        ));
    }
    gaps
}

fn positive_experiment_count(
    state: &ResearchState,
    links: &[EvidenceLink],
    target_id: &str,
    target_version: Option<&str>,
) -> usize {
    links
        .iter()
        .filter(|link| link.relation.supports_claim())
        .filter_map(|link| match &link.evidence {
            EvidenceRef::Experiment(id) => state
                .experiment(id)
                .filter(|experiment| {
                    experiment.result == super::records::ExperimentResult::Positive
                        && state.experiment_applies(experiment, target_id, target_version)
                })
                .map(|_| id),
            _ => None,
        })
        .collect::<BTreeSet<_>>()
        .len()
}

fn has_artifact_or_receipt_provenance(
    state: &ResearchState,
    references: &ResearchReferenceCatalog,
    links: &[EvidenceLink],
    target_id: &str,
    target_version: Option<&str>,
) -> bool {
    links.iter().any(|link| match &link.evidence {
        EvidenceRef::Artifact(_) => {
            state.evidence_reference_applies(references, &link.evidence, target_id, target_version)
        }
        EvidenceRef::ExecutionReceipt(id) => {
            references.execution_receipt(id).is_some()
                && state.evidence_reference_applies(
                    references,
                    &link.evidence,
                    target_id,
                    target_version,
                )
        }
        EvidenceRef::Experiment(id) => state.experiment(id).is_some_and(|experiment| {
            state.experiment_applies(experiment, target_id, target_version)
                && (experiment
                    .artifact_refs
                    .iter()
                    .any(|id| state.artifact_applies(id, target_id, target_version))
                    || state
                        .capsule_for_experiment(experiment)
                        .is_some_and(|capsule| {
                            capsule
                                .execution_receipt_ids
                                .iter()
                                .any(|id| references.execution_receipt(id).is_some())
                                || capsule
                                    .input_artifact_ids
                                    .iter()
                                    .chain(capsule.output_artifact_ids.iter())
                                    .any(|id| state.artifact_applies(id, target_id, target_version))
                        }))
        }),
    })
}

fn has_relation(links: &[EvidenceLink], relation: EvidenceRelation) -> bool {
    links.iter().any(|link| link.relation == relation)
}

const fn missing(kind: EvidenceGapKind, reason_code: EvidenceGapReasonCode) -> EvidenceGap {
    EvidenceGap {
        kind,
        state: EvidenceGapState::Missing,
        reason_code,
    }
}

const fn unknown(kind: EvidenceGapKind, reason_code: EvidenceGapReasonCode) -> EvidenceGap {
    EvidenceGap {
        kind,
        state: EvidenceGapState::Unknown,
        reason_code,
    }
}
