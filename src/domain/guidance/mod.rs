//! Fixed, local, deterministic evidence-gap and next-action guidance Shadow.
//!
//! This module owns the one pure report orchestration over existing typed
//! EvidenceGap facts, NextActions, and controlled typed candidates. It cannot
//! create an Experiment, execute a Tool, or mutate Research, Memory, Phase,
//! Context, or retrieval state.

mod contracts;
mod projection;
mod rank;
mod validation;

use contracts::GuidanceCandidateReport;
use projection::{controlled_eval_report, existing_next_action_report};
use rank::rank_reports;
use validation::{
    computed_action_fingerprints, duplicate_action_fingerprints, guidance_input_fingerprint,
    validate_eval_candidates,
};

use crate::domain::research::{ResearchReferenceCatalog, ResearchState, TargetSnapshotId};

pub(crate) use contracts::{
    CandidateValidity, GuidanceActionIdentityWitness, GuidanceError, GuidanceEvalCandidate,
    GuidanceEvalCandidates, GuidanceShadowReport, PrerequisiteState,
};

pub(super) const ALGORITHM_VERSION: &str = "m4-s3g-evidence-gap-guidance-v1";

/// Builds one side-effect-free report. Existing NextActions contribute only
/// typed lifecycle/reference fields; their operator action text and priority
/// are deliberately not used as substitute experiment semantics.
pub(crate) fn shadow_report(
    state: &ResearchState,
    references: &ResearchReferenceCatalog,
    current_target: &TargetSnapshotId,
    eval_candidates: GuidanceEvalCandidates,
) -> Result<GuidanceShadowReport, GuidanceError> {
    if state.target_snapshot(current_target).is_none() {
        return Err(GuidanceError::CurrentTargetMissing {
            id: current_target.as_str().to_owned(),
        });
    }
    validate_eval_candidates(&eval_candidates.candidates)?;
    let fingerprints = computed_action_fingerprints(&eval_candidates.candidates)?;
    let duplicate_fingerprints = duplicate_action_fingerprints(&fingerprints);
    let gap_reports = state.evidence_gap_reports(references);
    let mut reports = state
        .retrieval_next_action_records()
        .map(|record| existing_next_action_report(state, current_target, &gap_reports, record))
        .collect::<Vec<_>>();

    let mut eval_candidates = eval_candidates.candidates;
    eval_candidates.sort_by(|left, right| left.id.cmp(&right.id));
    reports.extend(
        eval_candidates
            .iter()
            .map(|candidate| {
                let fingerprint = fingerprints
                    .get(&candidate.id)
                    .expect("validated candidate has a computed action fingerprint");
                controlled_eval_report(
                    state,
                    current_target,
                    &gap_reports,
                    candidate,
                    fingerprint,
                    &duplicate_fingerprints,
                )
            })
            .collect::<Result<Vec<GuidanceCandidateReport>, _>>()?,
    );

    let input_fingerprint = guidance_input_fingerprint(current_target, &reports)?;
    rank_reports(&mut reports);
    Ok(GuidanceShadowReport::new(
        input_fingerprint,
        current_target.as_str().to_owned(),
        reports,
    ))
}
