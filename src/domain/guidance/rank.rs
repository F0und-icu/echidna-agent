use std::cmp::Ordering;

use super::contracts::{
    GuidanceCandidateReport, GuidanceFactor, GuidanceFactorName, GuidanceRankingTier,
    GuidanceReasonCode, GuidanceValueState,
};

pub(super) fn refresh_ranking_tier(report: &mut GuidanceCandidateReport) {
    if !report.exclusion_reasons.is_empty() {
        report.exclusion_reasons.sort();
        report.exclusion_reasons.dedup();
        report.ranking_tier = GuidanceRankingTier::Excluded;
        return;
    }
    let complete = report.prerequisite.state == GuidanceValueState::Available
        && report
            .factors
            .iter()
            .all(|factor| factor.state == GuidanceValueState::Available);
    if !complete {
        report.ranking_tier = GuidanceRankingTier::ConservativeIncompleteFactorVector;
        for factor in &mut report.factors {
            if factor.state != GuidanceValueState::Available {
                factor
                    .reason_codes
                    .push(GuidanceReasonCode::IncompleteFactorVectorConservativeTier);
            }
        }
    }
}

pub(super) fn rank_reports(reports: &mut [GuidanceCandidateReport]) {
    reports.sort_by(|left, right| {
        ranking_bucket(left)
            .cmp(&ranking_bucket(right))
            .then_with(|| complete_factor_order(left, right))
            .then_with(|| left.candidate_identity.cmp(&right.candidate_identity))
    });
    for (index, report) in reports.iter_mut().enumerate() {
        if report.ranking_tier != GuidanceRankingTier::Excluded {
            report.rank = Some(index + 1);
        }
    }
}

fn ranking_bucket(report: &GuidanceCandidateReport) -> u8 {
    match report.ranking_tier {
        GuidanceRankingTier::CompleteFactorVector => 0,
        GuidanceRankingTier::ConservativeIncompleteFactorVector => 1,
        GuidanceRankingTier::Excluded => 2,
    }
}

fn complete_factor_order(
    left: &GuidanceCandidateReport,
    right: &GuidanceCandidateReport,
) -> Ordering {
    if left.ranking_tier != GuidanceRankingTier::CompleteFactorVector
        || right.ranking_tier != GuidanceRankingTier::CompleteFactorVector
    {
        return Ordering::Equal;
    }
    for (index, name) in GuidanceFactorName::ORDER.into_iter().enumerate() {
        let left_value = factor_value(&left.factors, name);
        let right_value = factor_value(&right.factors, name);
        let order = match index {
            6..=8 => left_value.cmp(&right_value),
            _ => right_value.cmp(&left_value),
        };
        if order != Ordering::Equal {
            return order;
        }
    }
    Ordering::Equal
}

fn factor_value(factors: &[GuidanceFactor], name: GuidanceFactorName) -> u8 {
    factors
        .iter()
        .find(|factor| factor.name == name)
        .and_then(|factor| factor.value)
        .expect("complete factor vector has every bounded factor")
}
