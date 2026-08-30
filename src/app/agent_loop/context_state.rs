//! Context compilation and state-diff delivery for the single AgentLoop.
//!
//! The Context Planner remains the sole owner of fragment ordering and budget
//! decisions. This module only lets the AgentLoop substitute a bounded state
//! diff when the committed and current Plans both prove that the corresponding
//! full state projection was completely selected.

use crate::app::context_compilation::{ContextCacheSummary, resolve_context_fragment};
use crate::domain::context::{
    ContextBudget, ContextDecision, ContextFragment, ContextFragmentContent, ContextFragmentId,
    ContextFragmentPriority, ContextFragmentSource, ContextPlan, ContextPlanner,
    ContextPlannerError, ContextSourceFingerprint, FragmentCachePolicy, FullLogicalManifest,
};
use crate::domain::model::{
    ContentItem, ModelCapabilities, ModelInputIR, ModelRequestRequirements,
};
use crate::domain::research_control::PromptComposition;
use crate::domain::session::SessionSummary;
use crate::domain::tools::ToolSpec;
use crate::domain::world_state::{ProjectSnapshotContext, ResearchWorldState};
use crate::ports::context::ContextFragmentCache;

use super::AgentLoopError;

/// The complete local inputs for one Planner-owned Context compilation.
pub(super) struct ContextCompilationInput<'a> {
    pub(super) prompt_composition: &'a PromptComposition,
    pub(super) world_state: Option<&'a ResearchWorldState>,
    pub(super) project_snapshot: Option<&'a ProjectSnapshotContext>,
    pub(super) session_summary: Option<&'a SessionSummary>,
    pub(super) history: &'a [ContentItem],
    pub(super) turn_history_start: usize,
    pub(super) planner: &'a ContextPlanner,
    pub(super) cache: Option<&'a dyn ContextFragmentCache>,
    pub(super) budget: ContextBudget,
    pub(super) capabilities: &'a ModelCapabilities,
    pub(super) tool_specs: &'a [ToolSpec],
    pub(super) prior_manifest: Option<&'a FullLogicalManifest>,
}

/// The committed and current local facts needed to prove one state-diff
/// replacement is safe for incremental delivery.
pub(super) struct StateDiffDeliveryInput<'a> {
    pub(super) full_input: &'a ModelInputIR,
    pub(super) current_plan: &'a ContextPlan,
    pub(super) planner: &'a ContextPlanner,
    pub(super) budget: ContextBudget,
    pub(super) committed_input: Option<&'a ModelInputIR>,
    pub(super) committed_history_len: usize,
    pub(super) committed_manifest: Option<&'a FullLogicalManifest>,
    pub(super) committed_world_state: Option<&'a ResearchWorldState>,
    pub(super) world_state: Option<&'a ResearchWorldState>,
    pub(super) committed_project_snapshot: Option<&'a ProjectSnapshotContext>,
    pub(super) project_snapshot: Option<&'a ProjectSnapshotContext>,
    pub(super) history: &'a [ContentItem],
    pub(super) turn_history_start: usize,
}

pub(super) fn compile_context(
    input: ContextCompilationInput<'_>,
) -> Result<(ContextPlan, ContextCacheSummary), AgentLoopError> {
    let ContextCompilationInput {
        prompt_composition,
        world_state,
        project_snapshot,
        session_summary,
        history,
        turn_history_start,
        planner,
        cache,
        budget,
        capabilities,
        tool_specs,
        prior_manifest,
    } = input;
    if history.len() <= turn_history_start {
        return Err(AgentLoopError::ContextStateMismatch);
    }

    let mut cache_summary = ContextCacheSummary::default();
    let mut fragments = Vec::new();
    for prompt_fragment in prompt_composition.fragments()? {
        push_cached_fragment(
            &mut fragments,
            &mut cache_summary,
            prompt_fragment,
            planner,
            cache,
            capabilities,
            budget,
        )?;
    }
    if let Some(world_state) = world_state {
        push_cached_fragment(
            &mut fragments,
            &mut cache_summary,
            world_state.full_fragment()?,
            planner,
            cache,
            capabilities,
            budget,
        )?;
    }
    if let Some(project_snapshot) = project_snapshot {
        push_cached_fragment(
            &mut fragments,
            &mut cache_summary,
            project_snapshot.full_fragment()?,
            planner,
            cache,
            capabilities,
            budget,
        )?;
    }
    if let Some(session_summary) = session_summary {
        push_content_fragment(
            &mut fragments,
            "session:summary",
            ContextFragmentSource::SessionSummary,
            &[session_summary.context_item()?],
        )?;
    }
    push_content_fragment(
        &mut fragments,
        "history:recent",
        ContextFragmentSource::RecentHistory,
        &history[..turn_history_start],
    )?;
    push_content_fragment(
        &mut fragments,
        "input:current-user",
        ContextFragmentSource::UserInput,
        &history[turn_history_start..=turn_history_start],
    )?;
    push_content_fragment(
        &mut fragments,
        "turn:local-updates",
        ContextFragmentSource::TurnLocalUpdates,
        &history[turn_history_start + 1..],
    )?;

    let plan = planner.compile(
        fragments,
        budget,
        capabilities,
        tool_specs.to_vec(),
        ModelRequestRequirements::default(),
        prior_manifest,
    )?;
    Ok((plan, cache_summary))
}

pub(super) fn state_diff_delivery_input(
    input: StateDiffDeliveryInput<'_>,
) -> Result<Option<ModelInputIR>, AgentLoopError> {
    let StateDiffDeliveryInput {
        full_input,
        current_plan,
        planner,
        budget,
        committed_input,
        committed_history_len,
        committed_manifest,
        committed_world_state,
        world_state,
        committed_project_snapshot,
        project_snapshot,
        history,
        turn_history_start,
    } = input;
    let Some(committed_input) = committed_input else {
        return Ok(None);
    };
    let retains_reasoning = committed_input
        .content_items
        .iter()
        .any(|item| matches!(item, ContentItem::Reasoning { .. }));
    let Some(committed_manifest) = committed_manifest else {
        return continuation_unavailable_or_none(retains_reasoning);
    };
    if committed_history_len > history.len() || !plan_allows_history_append(current_plan) {
        return continuation_unavailable_or_none(retains_reasoning);
    }

    let mut state_items = Vec::new();
    match (committed_world_state, world_state) {
        (Some(previous), Some(current)) => {
            let previous_full = previous.full_fragment()?;
            let current_full = current.full_fragment()?;
            if !fragment_is_completely_included(current_plan, &current_full)
                || !committed_fragment_was_completely_included(committed_manifest, &previous_full)
            {
                return continuation_unavailable_or_none(retains_reasoning);
            }
            if let Some(fragment) = current.diff_fragment(previous)? {
                append_context_items(&mut state_items, fragment)?;
            }
        }
        (None, None) => {}
        _ => return continuation_unavailable_or_none(retains_reasoning),
    }
    match (committed_project_snapshot, project_snapshot) {
        (Some(previous), Some(current)) => {
            let previous_full = previous.full_fragment()?;
            let current_full = current.full_fragment()?;
            if !fragment_is_completely_included(current_plan, &current_full)
                || !committed_fragment_was_completely_included(committed_manifest, &previous_full)
            {
                return continuation_unavailable_or_none(retains_reasoning);
            }
            if let Some(fragment) = current.diff_fragment(previous)? {
                append_context_items(&mut state_items, fragment)?;
            }
        }
        (None, None) => {}
        _ => return continuation_unavailable_or_none(retains_reasoning),
    }
    let mut candidate = committed_input.clone();
    if committed_history_len <= turn_history_start {
        candidate
            .content_items
            .extend_from_slice(&history[committed_history_len..turn_history_start]);
        candidate.content_items.extend(state_items);
        candidate
            .content_items
            .extend_from_slice(&history[turn_history_start..]);
    } else {
        candidate
            .content_items
            .extend_from_slice(&history[committed_history_len..]);
        candidate.content_items.extend(state_items);
    }
    candidate.instructions = full_input.instructions.clone();
    candidate.tool_specs = full_input.tool_specs.clone();
    candidate.requirements = full_input.requirements;
    match planner.validate_rendered_model_input(&candidate, budget) {
        Ok(()) => Ok(Some(candidate)),
        Err(ContextPlannerError::RenderedInputExceedsBudget { .. }) if retains_reasoning => {
            Err(AgentLoopError::ProviderContinuationUnavailable)
        }
        Err(ContextPlannerError::RenderedInputExceedsBudget { .. }) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn continuation_unavailable_or_none(
    retains_reasoning: bool,
) -> Result<Option<ModelInputIR>, AgentLoopError> {
    if retains_reasoning {
        Err(AgentLoopError::ProviderContinuationUnavailable)
    } else {
        Ok(None)
    }
}

fn push_cached_fragment(
    fragments: &mut Vec<ContextFragment>,
    cache_summary: &mut ContextCacheSummary,
    fragment: ContextFragment,
    planner: &ContextPlanner,
    cache: Option<&dyn ContextFragmentCache>,
    capabilities: &ModelCapabilities,
    budget: ContextBudget,
) -> Result<(), AgentLoopError> {
    let cache_key = planner.fragment_cache_key(
        fragment.id().clone(),
        fragment.source(),
        fragment.priority(),
        fragment.cache_policy(),
        ContextSourceFingerprint::from_source(fragment.content())?,
        fragment.content_hash().clone(),
        capabilities,
        budget,
    )?;
    let resolved = resolve_context_fragment(cache, &cache_key, || Ok(fragment))?;
    cache_summary.record(resolved.cache_disposition);
    fragments.push(resolved.fragment);
    Ok(())
}

fn push_content_fragment(
    fragments: &mut Vec<ContextFragment>,
    id: &str,
    source: ContextFragmentSource,
    items: &[ContentItem],
) -> Result<(), AgentLoopError> {
    if items.is_empty() {
        return Ok(());
    }
    fragments.push(ContextFragment::new(
        ContextFragmentId::new(id)?,
        source,
        ContextFragmentPriority::Required,
        FragmentCachePolicy::DoNotCache,
        ContextFragmentContent::ContentItems {
            items: items.to_vec(),
        },
    )?);
    Ok(())
}

fn plan_allows_history_append(plan: &ContextPlan) -> bool {
    let decisions = plan.context_decision_manifest().candidates();
    decisions
        .iter()
        .filter(|entry| {
            matches!(
                entry.source(),
                ContextFragmentSource::RecentHistory | ContextFragmentSource::TurnLocalUpdates
            )
        })
        .all(|entry| matches!(entry.decision(), ContextDecision::Include))
        && decisions.iter().any(|entry| {
            entry.source() == ContextFragmentSource::UserInput
                && matches!(entry.decision(), ContextDecision::Include)
        })
}

fn fragment_is_completely_included(plan: &ContextPlan, fragment: &ContextFragment) -> bool {
    plan.context_decision_manifest()
        .candidates()
        .iter()
        .any(|entry| {
            entry.fragment_id() == fragment.id()
                && entry.source() == fragment.source()
                && entry.content_hash() == fragment.content_hash()
                && matches!(entry.decision(), ContextDecision::Include)
        })
}

fn committed_fragment_was_completely_included(
    manifest: &FullLogicalManifest,
    fragment: &ContextFragment,
) -> bool {
    manifest.entries().iter().any(|entry| {
        entry.fragment_id() == fragment.id()
            && entry.source() == fragment.source()
            && entry.content_hash() == fragment.content_hash()
            && matches!(entry.decision(), ContextDecision::Include)
    })
}

fn append_context_items(
    destination: &mut Vec<ContentItem>,
    fragment: ContextFragment,
) -> Result<(), AgentLoopError> {
    match fragment.content() {
        ContextFragmentContent::ContentItems { items } => {
            destination.extend(items.clone());
            Ok(())
        }
        ContextFragmentContent::Instructions { .. } => Err(AgentLoopError::ContextStateMismatch),
    }
}
