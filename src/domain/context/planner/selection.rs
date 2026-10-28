//! Context budget decisions and logical prefix comparison.

use std::cmp::Reverse;
use std::collections::BTreeSet;

use super::estimator::{estimate_artifact_reference_tokens, truncate_content_to_token_limit};
use super::{
    AppendOnlyCandidate, ContextBudget, ContextDecision, ContextDecisionEntry, ContextFragment,
    ContextFragmentPriority, ContextFragmentSource, ContextPlannerError, ContextReasonCode,
    FullLogicalManifest, LogicalAppendOnlyRejection, LogicalBaselineRelation,
};

pub(super) fn sort_full_order(fragments: &mut [ContextFragment]) {
    fragments.sort_by_key(|fragment| {
        (
            fragment.slot.order(),
            Reverse(fragment.priority),
            fragment.id.clone(),
        )
    });
}

/// This is the only uniqueness owner for Context fragment IDs. It runs before
/// source ordering so equal IDs cannot hide in distinct fixed-order slots.
pub(super) fn ensure_unique_ids(fragments: &[ContextFragment]) -> Result<(), ContextPlannerError> {
    let mut seen = BTreeSet::new();
    for fragment in fragments {
        if !seen.insert(fragment.id.clone()) {
            return Err(ContextPlannerError::DuplicateFragmentId {
                id: fragment.id.clone(),
            });
        }
    }
    Ok(())
}

pub(super) fn select_decisions(
    fragments: &[ContextFragment],
    budget: ContextBudget,
    fixed_input_tokens: u32,
    core_memory_limit: u32,
) -> Result<Vec<ContextDecisionEntry>, ContextPlannerError> {
    let available_fragment_tokens = budget
        .input_token_limit()
        .checked_sub(fixed_input_tokens)
        .ok_or(ContextPlannerError::FixedInputExceedsBudget {
            fixed_input_tokens,
            input_token_limit: budget.input_token_limit(),
        })?;
    let required_tokens = fragments
        .iter()
        .filter(|fragment| fragment.priority == ContextFragmentPriority::Required)
        .map(|fragment| u64::from(fragment.token_estimate))
        .sum::<u64>();
    if required_tokens > u64::from(available_fragment_tokens) {
        return Err(ContextPlannerError::RequiredFragmentsExceedBudget {
            required_tokens,
            input_token_limit: available_fragment_tokens,
        });
    }

    let mut selected = vec![None; fragments.len()];
    let mut selection_order = (0..fragments.len()).collect::<Vec<_>>();
    selection_order.sort_by_key(|index| {
        (
            Reverse(fragments[*index].priority),
            fragments[*index].slot.order(),
            fragments[*index].id.clone(),
        )
    });
    let mut remaining = available_fragment_tokens;
    for index in selection_order {
        let fragment = &fragments[index];
        // Keep one selection/truncation owner. The Core allowance is an
        // additional ceiling on the same token budget, not a file limit.
        let available = if fragment.source == ContextFragmentSource::ProjectMemoryCore
            && fragment.priority != ContextFragmentPriority::Required
        {
            remaining.min(core_memory_limit)
        } else {
            remaining
        };
        let artifact_reference = fragment
            .artifact_reference
            .clone()
            .map(|artifact_id| {
                estimate_artifact_reference_tokens(&artifact_id)
                    .map(|token_estimate| (artifact_id, token_estimate))
            })
            .transpose()?;
        let (decision, reason_code) = if fragment.token_estimate <= available {
            remaining -= fragment.token_estimate;
            (ContextDecision::Include, include_reason(fragment))
        } else if let Some((artifact_id, token_estimate)) = artifact_reference
            && available >= token_estimate
        {
            remaining -= token_estimate;
            (
                ContextDecision::ArtifactRef { artifact_id },
                ContextReasonCode::BudgetArtifactReference,
            )
        } else if available > 0
            && truncate_content_to_token_limit(&fragment.content, available)?.is_some()
        {
            let retained_tokens = available;
            remaining -= available;
            (
                ContextDecision::Truncate { retained_tokens },
                ContextReasonCode::BudgetTruncated,
            )
        } else {
            (ContextDecision::Exclude, ContextReasonCode::BudgetExcluded)
        };
        selected[index] = Some(ContextDecisionEntry {
            fragment_id: fragment.id.clone(),
            source: fragment.source,
            slot: fragment.slot,
            origin: fragment.origin,
            content_hash: fragment.content_hash.clone(),
            token_estimate: fragment.token_estimate,
            decision,
            reason_code,
            baseline_relation: LogicalBaselineRelation::NoPriorManifest,
        });
    }
    Ok(selected
        .into_iter()
        .map(|entry| entry.expect("each Context fragment receives one decision"))
        .collect())
}

fn include_reason(fragment: &ContextFragment) -> ContextReasonCode {
    match (fragment.priority, fragment.source) {
        (ContextFragmentPriority::Required, source) if source.placement().order() <= 5 => {
            ContextReasonCode::RequiredBaseInstructions
        }
        (ContextFragmentPriority::Required, ContextFragmentSource::UserInput) => {
            ContextReasonCode::RequiredRecentInput
        }
        _ => ContextReasonCode::SelectedByPriority,
    }
}

pub(super) fn append_only_candidate(
    prior: Option<&FullLogicalManifest>,
    current: &FullLogicalManifest,
) -> AppendOnlyCandidate {
    let Some(prior) = prior else {
        return AppendOnlyCandidate::NoPriorManifest;
    };
    if current.entries.len() < prior.entries.len() {
        return AppendOnlyCandidate::NotAppendOnly {
            reason: LogicalAppendOnlyRejection::CurrentManifestShorter,
        };
    }
    if current.entries[..prior.entries.len()] != prior.entries[..] {
        return AppendOnlyCandidate::NotAppendOnly {
            reason: LogicalAppendOnlyRejection::PriorFragmentChanged,
        };
    }
    AppendOnlyCandidate::LogicalPrefixDelta {
        retained_prefix_len: prior.entries.len(),
        appended_fragment_ids: current.entries[prior.entries.len()..]
            .iter()
            .map(|entry| entry.fragment_id.clone())
            .collect(),
    }
}

pub(super) fn apply_baseline_relations(
    mut decisions: Vec<ContextDecisionEntry>,
    candidate: &AppendOnlyCandidate,
    current: &FullLogicalManifest,
) -> Vec<ContextDecisionEntry> {
    for decision in &mut decisions {
        if matches!(decision.decision, ContextDecision::Exclude) {
            decision.baseline_relation = LogicalBaselineRelation::Excluded;
            continue;
        }
        decision.baseline_relation = match candidate {
            AppendOnlyCandidate::NoPriorManifest => LogicalBaselineRelation::NoPriorManifest,
            AppendOnlyCandidate::LogicalPrefixDelta {
                retained_prefix_len,
                ..
            } => current
                .entries
                .iter()
                .position(|entry| entry.fragment_id == decision.fragment_id)
                .map_or(LogicalBaselineRelation::Replaced, |index| {
                    if index < *retained_prefix_len {
                        LogicalBaselineRelation::PrefixRetained
                    } else {
                        LogicalBaselineRelation::Appended
                    }
                }),
            AppendOnlyCandidate::NotAppendOnly { .. } => LogicalBaselineRelation::Replaced,
        };
    }
    decisions
}
