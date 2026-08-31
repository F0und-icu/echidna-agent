use std::collections::BTreeSet;

use crate::domain::research::{
    EvidenceGapReport, EvidenceGapState, NextActionRecord, NextActionStatus, ResearchRecordRef,
    ResearchReferenceTargetApplicability, ResearchState, TargetSnapshotId,
};

use super::contracts::{
    CandidateValidity, GapClosureSummary, GuidanceActionFingerprint, GuidanceCandidateReport,
    GuidanceCandidateSource, GuidanceError, GuidanceEvalCandidate, GuidanceFactor,
    GuidanceFactorName, GuidanceGapSummary, GuidancePrerequisite, GuidanceRankingTier,
    GuidanceReasonCode, GuidanceValueState, PrerequisiteState,
};
use super::rank::refresh_ranking_tier;
use super::validation::controlled_candidate_identity;

pub(super) fn existing_next_action_report(
    state: &ResearchState,
    current_target: &TargetSnapshotId,
    gap_reports: &[EvidenceGapReport],
    record: &NextActionRecord,
) -> GuidanceCandidateReport {
    let applicability =
        state.research_reference_target_applicability(&record.research_id, current_target);
    let mut exclusion_reasons = Vec::new();
    match record.status {
        NextActionStatus::Done => {
            exclusion_reasons.push(GuidanceReasonCode::NextActionLifecycleDone)
        }
        NextActionStatus::Blocked => {
            exclusion_reasons.push(GuidanceReasonCode::NextActionLifecycleBlocked)
        }
        NextActionStatus::Running => {
            exclusion_reasons.push(GuidanceReasonCode::NextActionLifecycleRunning)
        }
        NextActionStatus::Pending => {}
    }
    append_applicability_exclusion(applicability, &mut exclusion_reasons);
    let (associated_gaps, gap_closure) = gaps_for_reference(gap_reports, &record.research_id);
    let target_factor = target_factor(applicability);
    let factors = GuidanceFactorName::ORDER
        .into_iter()
        .map(|name| match name {
            GuidanceFactorName::TargetSnapshotApplicability => target_factor.clone(),
            GuidanceFactorName::NoveltyVsActionFingerprint => GuidanceFactor::unavailable(
                name,
                GuidanceValueState::Unavailable,
                GuidanceReasonCode::NextActionHasNoExperimentActionFingerprint,
            ),
            _ => GuidanceFactor::unavailable(
                name,
                GuidanceValueState::Unavailable,
                GuidanceReasonCode::NextActionFieldUnavailable,
            ),
        })
        .collect();
    let mut report = GuidanceCandidateReport {
        candidate_identity: format!("next_action:{}", record.id.as_str()),
        source: GuidanceCandidateSource::ExistingNextAction,
        source_id: record.id.as_str().to_owned(),
        research_id: record.research_id.clone(),
        next_action_status: Some(record.status),
        source_priority: Some(record.priority),
        action_fingerprint: GuidanceActionFingerprint {
            state: GuidanceValueState::Unavailable,
            value: None,
            reason_codes: vec![GuidanceReasonCode::NextActionHasNoExperimentActionFingerprint],
        },
        prerequisite: GuidancePrerequisite {
            state: GuidanceValueState::Unavailable,
            reason_codes: vec![GuidanceReasonCode::NextActionFieldUnavailable],
        },
        associated_gaps,
        declared_gap_closure: gap_closure,
        factors,
        exclusion_reasons,
        ranking_tier: GuidanceRankingTier::ConservativeIncompleteFactorVector,
        rank: None,
        tie_break: GuidanceReasonCode::DeterministicCandidateIdentityTieBreak,
    };
    refresh_ranking_tier(&mut report);
    report
}

pub(super) fn controlled_eval_report(
    state: &ResearchState,
    current_target: &TargetSnapshotId,
    gap_reports: &[EvidenceGapReport],
    candidate: &GuidanceEvalCandidate,
    computed_action_fingerprint: &str,
    duplicate_fingerprints: &BTreeSet<String>,
) -> Result<GuidanceCandidateReport, GuidanceError> {
    let candidate_applicability = if state
        .target_snapshot(&candidate.target_snapshot_id)
        .is_none()
    {
        ResearchReferenceTargetApplicability::MissingReference
    } else if &candidate.target_snapshot_id != current_target {
        ResearchReferenceTargetApplicability::Incompatible
    } else {
        state.research_reference_target_applicability(&candidate.research_id, current_target)
    };
    let (associated_gaps, mut closure) = gaps_for_reference(gap_reports, &candidate.research_id);
    let mut exclusion_reasons = Vec::new();
    if candidate.validity == CandidateValidity::Invalid {
        exclusion_reasons.push(GuidanceReasonCode::CandidateExplicitlyInvalid);
    }
    if candidate.prerequisite == PrerequisiteState::Missing {
        exclusion_reasons.push(GuidanceReasonCode::CandidatePrerequisiteMissing);
    }
    if state
        .target_snapshot(&candidate.target_snapshot_id)
        .is_none()
    {
        exclusion_reasons.push(GuidanceReasonCode::CandidateTargetSnapshotMissing);
    } else if &candidate.target_snapshot_id != current_target {
        exclusion_reasons.push(GuidanceReasonCode::CandidateTargetSnapshotDoesNotMatchCurrent);
    }
    append_applicability_exclusion(candidate_applicability, &mut exclusion_reasons);
    if duplicate_fingerprints.contains(computed_action_fingerprint) {
        exclusion_reasons.push(GuidanceReasonCode::DuplicateCandidateActionFingerprint);
    }
    if !state
        .experiments_by_action_fingerprint(computed_action_fingerprint)
        .is_empty()
    {
        exclusion_reasons.push(GuidanceReasonCode::PriorActionFingerprintExists);
    }

    let expected_gap_factor = declared_gap_closure_factor(
        &candidate.expected_gap_closures,
        &associated_gaps,
        &mut closure,
        &mut exclusion_reasons,
    );
    let novelty_factor =
        if exclusion_reasons.contains(&GuidanceReasonCode::PriorActionFingerprintExists) {
            GuidanceFactor::available(
                GuidanceFactorName::NoveltyVsActionFingerprint,
                0,
                GuidanceReasonCode::PriorActionFingerprintExists,
            )
        } else if duplicate_fingerprints.contains(computed_action_fingerprint) {
            GuidanceFactor::available(
                GuidanceFactorName::NoveltyVsActionFingerprint,
                0,
                GuidanceReasonCode::DuplicateCandidateActionFingerprint,
            )
        } else {
            GuidanceFactor::available(
                GuidanceFactorName::NoveltyVsActionFingerprint,
                100,
                GuidanceReasonCode::CandidateActionFingerprintNovel,
            )
        };
    let target_factor = target_factor(candidate_applicability);
    let factors = GuidanceFactorName::ORDER
        .into_iter()
        .map(|name| match name {
            GuidanceFactorName::BoundaryImportance => {
                available_candidate_factor(name, candidate.boundary_importance)
            }
            GuidanceFactorName::CompetingHypothesisDiscrimination => {
                available_candidate_factor(name, candidate.competing_hypothesis_discrimination)
            }
            GuidanceFactorName::ExpectedGapClosure => expected_gap_factor.clone(),
            GuidanceFactorName::Reproducibility => {
                available_candidate_factor(name, candidate.reproducibility)
            }
            GuidanceFactorName::NoveltyVsActionFingerprint => novelty_factor.clone(),
            GuidanceFactorName::TargetSnapshotApplicability => target_factor.clone(),
            GuidanceFactorName::ExecutionCost => {
                available_candidate_factor(name, candidate.execution_cost)
            }
            GuidanceFactorName::MutationRisk => {
                available_candidate_factor(name, candidate.mutation_risk)
            }
            GuidanceFactorName::VersionDriftRisk => {
                available_candidate_factor(name, candidate.version_drift_risk)
            }
        })
        .collect();
    let prerequisite = match candidate.prerequisite {
        PrerequisiteState::Satisfied => GuidancePrerequisite {
            state: GuidanceValueState::Available,
            reason_codes: vec![GuidanceReasonCode::CandidatePrerequisiteSatisfied],
        },
        PrerequisiteState::Missing => GuidancePrerequisite {
            state: GuidanceValueState::Missing,
            reason_codes: vec![GuidanceReasonCode::CandidatePrerequisiteMissing],
        },
        PrerequisiteState::Unknown => GuidancePrerequisite {
            state: GuidanceValueState::Unknown,
            reason_codes: vec![GuidanceReasonCode::CandidatePrerequisiteUnknown],
        },
    };
    let mut report = GuidanceCandidateReport {
        candidate_identity: controlled_candidate_identity(candidate)?,
        source: GuidanceCandidateSource::ControlledEvalCandidate,
        source_id: candidate.id.clone(),
        research_id: candidate.research_id.clone(),
        next_action_status: None,
        source_priority: None,
        action_fingerprint: GuidanceActionFingerprint {
            state: GuidanceValueState::Available,
            value: Some(computed_action_fingerprint.to_owned()),
            reason_codes: vec![GuidanceReasonCode::ComputedControlledActionFingerprint],
        },
        prerequisite,
        associated_gaps,
        declared_gap_closure: closure,
        factors,
        exclusion_reasons,
        ranking_tier: GuidanceRankingTier::CompleteFactorVector,
        rank: None,
        tie_break: GuidanceReasonCode::DeterministicCandidateIdentityTieBreak,
    };
    refresh_ranking_tier(&mut report);
    Ok(report)
}

fn gaps_for_reference(
    reports: &[EvidenceGapReport],
    reference: &ResearchRecordRef,
) -> (Vec<GuidanceGapSummary>, GapClosureSummary) {
    let Some(report) = reports
        .iter()
        .find(|report| report.research_id() == reference)
    else {
        return (
            Vec::new(),
            GapClosureSummary {
                evidence_gap_report_available: false,
                declared_critical_gap_count: 0,
                matched_missing_gap_count: None,
                total_missing_gap_count: None,
            },
        );
    };
    let gaps = report
        .gaps()
        .iter()
        .map(|gap| GuidanceGapSummary {
            kind: gap.kind(),
            state: gap.state(),
            reason_code: gap.reason_code(),
        })
        .collect::<Vec<_>>();
    let missing = gaps
        .iter()
        .filter(|gap| gap.state == EvidenceGapState::Missing)
        .count();
    (
        gaps,
        GapClosureSummary {
            evidence_gap_report_available: true,
            declared_critical_gap_count: 0,
            matched_missing_gap_count: Some(0),
            total_missing_gap_count: Some(missing),
        },
    )
}

fn declared_gap_closure_factor(
    declared: &[crate::domain::research::EvidenceGapKind],
    gaps: &[GuidanceGapSummary],
    closure: &mut GapClosureSummary,
    exclusions: &mut Vec<GuidanceReasonCode>,
) -> GuidanceFactor {
    closure.declared_critical_gap_count = declared.len();
    if !closure.evidence_gap_report_available {
        closure.matched_missing_gap_count = None;
        closure.total_missing_gap_count = None;
        return GuidanceFactor::unavailable(
            GuidanceFactorName::ExpectedGapClosure,
            GuidanceValueState::Unavailable,
            GuidanceReasonCode::ResearchReferenceHasNoEvidenceGapReport,
        );
    }
    let mut seen = BTreeSet::new();
    let mut matched_missing = 0usize;
    let mut unknown = false;
    for kind in declared {
        if !seen.insert(*kind as u8) {
            exclusions.push(GuidanceReasonCode::DuplicateDeclaredCriticalGap);
            continue;
        }
        match gaps.iter().find(|gap| gap.kind == *kind) {
            Some(gap) if gap.state == EvidenceGapState::Missing => matched_missing += 1,
            Some(gap) if gap.state == EvidenceGapState::Unknown => unknown = true,
            Some(_) => unreachable!("EvidenceGap has only missing or unknown states"),
            None => exclusions.push(GuidanceReasonCode::DeclaredCriticalGapNotPresent),
        }
    }
    closure.matched_missing_gap_count = Some(matched_missing);
    if unknown {
        return GuidanceFactor::unavailable(
            GuidanceFactorName::ExpectedGapClosure,
            GuidanceValueState::Unknown,
            GuidanceReasonCode::DeclaredCriticalGapUnknown,
        );
    }
    let total_missing = closure.total_missing_gap_count.unwrap_or(0);
    let value = if total_missing == 0 {
        0
    } else {
        ((matched_missing * 100) / total_missing).min(100) as u8
    };
    GuidanceFactor::available(
        GuidanceFactorName::ExpectedGapClosure,
        value,
        if declared.is_empty() {
            GuidanceReasonCode::NoDeclaredCriticalGap
        } else {
            GuidanceReasonCode::DeclaredCriticalGapMissing
        },
    )
}

fn append_applicability_exclusion(
    applicability: ResearchReferenceTargetApplicability,
    exclusions: &mut Vec<GuidanceReasonCode>,
) {
    match applicability {
        ResearchReferenceTargetApplicability::Applicable
        | ResearchReferenceTargetApplicability::Unknown => {}
        ResearchReferenceTargetApplicability::Incompatible => {
            exclusions.push(GuidanceReasonCode::ExistingReferenceTargetIncompatible)
        }
        ResearchReferenceTargetApplicability::MissingReference => {
            exclusions.push(GuidanceReasonCode::ExistingReferenceMissing)
        }
    }
}

fn target_factor(applicability: ResearchReferenceTargetApplicability) -> GuidanceFactor {
    match applicability {
        ResearchReferenceTargetApplicability::Applicable => GuidanceFactor::available(
            GuidanceFactorName::TargetSnapshotApplicability,
            100,
            GuidanceReasonCode::ExistingReferenceAppliesToCurrentSnapshot,
        ),
        ResearchReferenceTargetApplicability::Incompatible => GuidanceFactor::unavailable(
            GuidanceFactorName::TargetSnapshotApplicability,
            GuidanceValueState::Missing,
            GuidanceReasonCode::ExistingReferenceTargetIncompatible,
        ),
        ResearchReferenceTargetApplicability::Unknown => GuidanceFactor::unavailable(
            GuidanceFactorName::TargetSnapshotApplicability,
            GuidanceValueState::Unknown,
            GuidanceReasonCode::ExistingReferenceNotSnapshotBound,
        ),
        ResearchReferenceTargetApplicability::MissingReference => GuidanceFactor::unavailable(
            GuidanceFactorName::TargetSnapshotApplicability,
            GuidanceValueState::Unavailable,
            GuidanceReasonCode::ExistingReferenceMissing,
        ),
    }
}

fn available_candidate_factor(name: GuidanceFactorName, value: u8) -> GuidanceFactor {
    GuidanceFactor::available(
        name,
        value,
        GuidanceReasonCode::CallerSuppliedControlledFactor,
    )
}
