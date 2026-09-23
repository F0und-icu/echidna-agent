//! Context compilation and state-diff delivery for the single AgentLoop.
//!
//! The Context Planner remains the sole owner of fragment ordering and budget
//! decisions. This module only lets the AgentLoop substitute a bounded state
//! diff when the committed and current Plans both prove that the corresponding
//! full state projection was completely selected.

use std::collections::BTreeMap;

use crate::app::context_compilation::{ContextCacheSummary, resolve_context_fragment};
use crate::domain::context::{
    ContextBudget, ContextDecision, ContextDeliveryMap, ContextFragment, ContextFragmentContent,
    ContextFragmentId, ContextFragmentPriority, ContextFragmentSource, ContextPlan, ContextPlanner,
    ContextPlannerError, ContextProvenance, ContextSourceFingerprint, FragmentCachePolicy,
    FullLogicalManifest,
};
use crate::domain::model::{
    ContentItem, ModelCapabilities, ModelInputIR, ModelRequestRequirements,
};
use crate::domain::research_control::PromptComposition;
use crate::domain::session::SessionSummary;
use crate::domain::tools::ToolSpec;
use crate::domain::world_state::{ProjectSnapshotContext, ResearchWorldState};
use crate::ports::context::ContextFragmentCache;

use super::{AgentLoopError, ContinuationRejection};

/// The complete local inputs for one Planner-owned Context compilation.
pub(super) struct ContextCompilationInput<'a> {
    pub(super) prompt_composition: &'a PromptComposition,
    pub(super) world_state: Option<&'a ResearchWorldState>,
    pub(super) project_snapshot: Option<&'a ProjectSnapshotContext>,
    pub(super) project_memory_core: Option<&'a ContextFragment>,
    pub(super) session_summary: Option<&'a SessionSummary>,
    pub(super) history: &'a [ContentItem],
    pub(super) history_provenance: &'a BTreeMap<usize, ContextProvenance>,
    pub(super) turn_history_start: usize,
    pub(super) planner: &'a ContextPlanner,
    pub(super) cache: Option<&'a dyn ContextFragmentCache>,
    pub(super) budget: ContextBudget,
    pub(super) capabilities: &'a ModelCapabilities,
    pub(super) tool_specs: &'a [ToolSpec],
    pub(super) prior_manifest: Option<&'a FullLogicalManifest>,
    pub(super) correction_feedback: &'a [ContextFragment],
}

/// One fixed, typed replacement for volatile facts before a later sampling
/// Step. It contains no ordering or delivery decision: ContextPlanner remains
/// the sole compiler after AgentLoop installs these candidates.
#[derive(Clone, Debug)]
pub(crate) struct CurrentContextState {
    world_state: ResearchWorldState,
    project_snapshot: ProjectSnapshotContext,
    project_memory_core: Option<ContextFragment>,
    personal_prompt: Option<String>,
    project_prompt: Option<String>,
}

impl CurrentContextState {
    pub(crate) fn new(
        world_state: ResearchWorldState,
        project_snapshot: ProjectSnapshotContext,
        project_memory_core: Option<ContextFragment>,
        personal_prompt: Option<String>,
        project_prompt: Option<String>,
    ) -> Self {
        Self {
            world_state,
            project_snapshot,
            project_memory_core,
            personal_prompt,
            project_prompt,
        }
    }

    pub(crate) fn world_state(&self) -> &ResearchWorldState {
        &self.world_state
    }

    pub(crate) fn project_snapshot(&self) -> &ProjectSnapshotContext {
        &self.project_snapshot
    }

    pub(crate) fn project_memory_core(&self) -> Option<&ContextFragment> {
        self.project_memory_core.as_ref()
    }

    pub(crate) fn apply_prompt_sources(
        &self,
        composition: PromptComposition,
    ) -> Result<PromptComposition, crate::domain::research_control::ResearchControlError> {
        let composition = match &self.personal_prompt {
            Some(prompt) => composition.with_personal(prompt.clone())?,
            None => composition,
        };
        match &self.project_prompt {
            Some(prompt) => composition.with_project(prompt.clone()),
            None => Ok(composition),
        }
    }
}

/// The committed and current local facts needed to prove one state-diff
/// replacement is safe for incremental delivery.
pub(super) struct StateDiffDeliveryInput<'a> {
    pub(super) correction_feedback: &'a [ContextFragment],
    pub(super) step_id: &'a str,
    pub(super) full_input: &'a ModelInputIR,
    pub(super) full_delivery_map: &'a ContextDeliveryMap,
    pub(super) current_plan: &'a ContextPlan,
    pub(super) planner: &'a ContextPlanner,
    pub(super) budget: ContextBudget,
    pub(super) committed_input: Option<&'a ModelInputIR>,
    pub(super) committed_delivery_map: Option<&'a ContextDeliveryMap>,
    pub(super) committed_history_len: usize,
    pub(super) committed_manifest: Option<&'a FullLogicalManifest>,
    pub(super) committed_world_state: Option<&'a ResearchWorldState>,
    pub(super) world_state: Option<&'a ResearchWorldState>,
    pub(super) committed_project_snapshot: Option<&'a ProjectSnapshotContext>,
    pub(super) project_snapshot: Option<&'a ProjectSnapshotContext>,
    pub(super) committed_project_memory_core: Option<&'a ContextFragment>,
    pub(super) project_memory_core: Option<&'a ContextFragment>,
    pub(super) history: &'a [ContentItem],
    pub(super) history_provenance: &'a BTreeMap<usize, ContextProvenance>,
    pub(super) turn_history_start: usize,
}

pub(super) fn compile_context(
    input: ContextCompilationInput<'_>,
) -> Result<(ContextPlan, ContextCacheSummary), AgentLoopError> {
    let ContextCompilationInput {
        prompt_composition,
        world_state,
        project_snapshot,
        project_memory_core,
        session_summary,
        history,
        history_provenance,
        turn_history_start,
        planner,
        cache,
        budget,
        capabilities,
        tool_specs,
        prior_manifest,
        correction_feedback,
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
    if let Some(project_memory_core) = project_memory_core {
        push_cached_fragment(
            &mut fragments,
            &mut cache_summary,
            project_memory_core.clone(),
            planner,
            cache,
            capabilities,
            budget,
        )?;
    }
    if let Some(session_summary) = session_summary {
        let mut summary_fragment = ContextFragment::new(
            ContextFragmentId::new("session:summary")?,
            ContextFragmentSource::SessionSummary,
            ContextFragmentPriority::Required,
            FragmentCachePolicy::DoNotCache,
            ContextFragmentContent::ContentItems {
                items: vec![session_summary.context_item()?],
            },
        )?;
        if let Some(provenance) = session_summary.context_provenance()? {
            summary_fragment = summary_fragment.with_provenance(provenance)?;
        }
        fragments.push(summary_fragment);
    }
    push_content_fragment(
        &mut fragments,
        ContentFragmentInput {
            id: "history:recent".to_owned(),
            source: ContextFragmentSource::RecentHistory,
            items: &history[..turn_history_start],
            history_start: 0,
            history_provenance,
        },
    )?;
    push_content_fragment(
        &mut fragments,
        ContentFragmentInput {
            id: "input:current-user".to_owned(),
            source: ContextFragmentSource::UserInput,
            items: &history[turn_history_start..=turn_history_start],
            history_start: turn_history_start,
            history_provenance,
        },
    )?;
    push_content_fragment(
        &mut fragments,
        ContentFragmentInput {
            id: "turn:local-updates".to_owned(),
            source: ContextFragmentSource::TurnLocalUpdates,
            items: &history[turn_history_start + 1..],
            history_start: turn_history_start + 1,
            history_provenance,
        },
    )?;

    fragments.extend_from_slice(correction_feedback);
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
) -> Result<Option<(ModelInputIR, ContextDeliveryMap)>, AgentLoopError> {
    let StateDiffDeliveryInput {
        correction_feedback,
        step_id,
        full_input,
        full_delivery_map,
        current_plan,
        planner,
        budget,
        committed_input,
        committed_delivery_map,
        committed_history_len,
        committed_manifest,
        committed_world_state,
        world_state,
        committed_project_snapshot,
        project_snapshot,
        committed_project_memory_core,
        project_memory_core,
        history,
        history_provenance,
        turn_history_start,
    } = input;
    let Some(committed_input) = committed_input else {
        return Ok(None);
    };
    let retains_reasoning = committed_input
        .content_items
        .iter()
        .any(|item| matches!(item, ContentItem::Reasoning { .. }));
    let Some(committed_delivery_map) = committed_delivery_map else {
        return continuation_unavailable_or_none(
            retains_reasoning,
            ContinuationRejection::MissingBaseline,
        );
    };
    committed_delivery_map.validate_against(committed_input)?;
    let Some(committed_manifest) = committed_manifest else {
        return continuation_unavailable_or_none(
            retains_reasoning,
            ContinuationRejection::MissingBaseline,
        );
    };
    if committed_history_len > history.len() || !plan_allows_history_append(current_plan) {
        return continuation_unavailable_or_none(
            retains_reasoning,
            ContinuationRejection::HistoryReplaced,
        );
    }

    let mut state_fragments = Vec::new();
    match (committed_world_state, world_state) {
        (Some(previous), Some(current)) => {
            let previous_full = previous.full_fragment()?;
            let current_full = current.full_fragment()?;
            if !fragment_is_completely_included(current_plan, &current_full)
                || !committed_fragment_was_completely_included(committed_manifest, &previous_full)
            {
                return continuation_unavailable_or_none(
                    retains_reasoning,
                    ContinuationRejection::ResearchProjectionChanged,
                );
            }
            if let Some(fragment) = current.diff_fragment(previous)? {
                state_fragments.push(fragment);
            }
        }
        (None, None) => {}
        _ => {
            return continuation_unavailable_or_none(
                retains_reasoning,
                ContinuationRejection::ResearchProjectionChanged,
            );
        }
    }
    match (committed_project_snapshot, project_snapshot) {
        (Some(previous), Some(current)) => {
            let previous_full = previous.full_fragment()?;
            let current_full = current.full_fragment()?;
            if !fragment_is_completely_included(current_plan, &current_full)
                || !committed_fragment_was_completely_included(committed_manifest, &previous_full)
            {
                return continuation_unavailable_or_none(
                    retains_reasoning,
                    ContinuationRejection::ProjectProjectionChanged,
                );
            }
            if let Some(fragment) = current.diff_fragment(previous)? {
                state_fragments.push(fragment);
            }
        }
        (None, None) => {}
        _ => {
            return continuation_unavailable_or_none(
                retains_reasoning,
                ContinuationRejection::ProjectProjectionChanged,
            );
        }
    }
    // Core Memory has no diff representation. It is always a complete
    // authoritative Markdown fragment, so a content/presence change requires
    // the normal Full logical Context rather than an append that could retain
    // stale memory beside reasoning continuation.
    if !same_memory_core(committed_project_memory_core, project_memory_core) {
        return continuation_unavailable_or_none(
            retains_reasoning,
            ContinuationRejection::CoreMemoryChanged,
        );
    }
    let mut candidate = committed_input.clone();
    let mut delivery_map = committed_delivery_map.clone();
    delivery_map.replace_instructions_from(full_delivery_map);
    if committed_history_len <= turn_history_start {
        append_delivery_items(
            &mut candidate,
            &mut delivery_map,
            planner,
            ContentFragmentInput {
                id: format!("delivery:{step_id}:history-before-turn"),
                source: ContextFragmentSource::RecentHistory,
                items: &history[committed_history_len..turn_history_start],
                history_start: committed_history_len,
                history_provenance,
            },
        )?;
        for (index, fragment) in state_fragments.into_iter().enumerate() {
            append_delivery_fragment(
                &mut candidate,
                &mut delivery_map,
                planner,
                fragment.reidentify(ContextFragmentId::new(format!(
                    "delivery:{step_id}:state-diff:{index}"
                ))?),
            )?;
        }
        append_delivery_items(
            &mut candidate,
            &mut delivery_map,
            planner,
            ContentFragmentInput {
                id: format!("delivery:{step_id}:turn-updates"),
                source: ContextFragmentSource::TurnLocalUpdates,
                items: &history[turn_history_start..],
                history_start: turn_history_start,
                history_provenance,
            },
        )?;
    } else {
        append_delivery_items(
            &mut candidate,
            &mut delivery_map,
            planner,
            ContentFragmentInput {
                id: format!("delivery:{step_id}:history-after-commit"),
                source: ContextFragmentSource::TurnLocalUpdates,
                items: &history[committed_history_len..],
                history_start: committed_history_len,
                history_provenance,
            },
        )?;
        for (index, fragment) in state_fragments.into_iter().enumerate() {
            append_delivery_fragment(
                &mut candidate,
                &mut delivery_map,
                planner,
                fragment.reidentify(ContextFragmentId::new(format!(
                    "delivery:{step_id}:state-diff:{index}"
                ))?),
            )?;
        }
    }
    for fragment in correction_feedback {
        if !delivery_map
            .content_items()
            .iter()
            .any(|item| item.fragment_id() == fragment.id())
        {
            append_delivery_fragment(&mut candidate, &mut delivery_map, planner, fragment.clone())?;
        }
    }
    candidate.instructions = full_input.instructions.clone();
    candidate.tool_specs = full_input.tool_specs.clone();
    candidate.requirements = full_input.requirements;
    match planner.validate_rendered_model_input(&candidate, budget) {
        Ok(()) => {
            delivery_map.validate_against(&candidate)?;
            Ok(Some((candidate, delivery_map)))
        }
        Err(ContextPlannerError::RenderedInputExceedsBudget {
            rendered_input_tokens,
            input_token_limit,
        }) if retains_reasoning => Err(AgentLoopError::ProviderContinuationUnavailable {
            reason: ContinuationRejection::DeliveryBudgetExceeded {
                rendered_input_tokens,
                input_token_limit,
            },
        }),
        Err(ContextPlannerError::RenderedInputExceedsBudget { .. }) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn same_memory_core(previous: Option<&ContextFragment>, current: Option<&ContextFragment>) -> bool {
    match (previous, current) {
        (None, None) => true,
        (Some(previous), Some(current)) => {
            previous.id() == current.id()
                && previous.content_hash() == current.content_hash()
                && previous.source() == current.source()
        }
        _ => false,
    }
}

fn continuation_unavailable_or_none(
    retains_reasoning: bool,
    reason: ContinuationRejection,
) -> Result<Option<(ModelInputIR, ContextDeliveryMap)>, AgentLoopError> {
    if retains_reasoning {
        Err(AgentLoopError::ProviderContinuationUnavailable { reason })
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

struct ContentFragmentInput<'a> {
    id: String,
    source: ContextFragmentSource,
    items: &'a [ContentItem],
    history_start: usize,
    history_provenance: &'a BTreeMap<usize, ContextProvenance>,
}

impl ContentFragmentInput<'_> {
    fn into_fragment(self) -> Result<Option<ContextFragment>, AgentLoopError> {
        if self.items.is_empty() {
            return Ok(None);
        }
        let mut fragment = ContextFragment::new(
            ContextFragmentId::new(self.id)?,
            self.source,
            ContextFragmentPriority::Required,
            FragmentCachePolicy::DoNotCache,
            ContextFragmentContent::ContentItems {
                items: self.items.to_vec(),
            },
        )?;
        let history_end = self
            .history_start
            .checked_add(self.items.len())
            .ok_or(AgentLoopError::ContextStateMismatch)?;
        let slot = fragment.slot();
        for (history_index, provenance) in self
            .history_provenance
            .range(self.history_start..history_end)
        {
            let ordinal = u32::try_from(history_index - self.history_start)
                .map_err(|_| ContextPlannerError::ContextPositionOverflow)?;
            fragment =
                fragment.with_content_item_provenance(ordinal, provenance.repositioned(slot))?;
        }
        Ok(Some(fragment))
    }
}

fn push_content_fragment(
    fragments: &mut Vec<ContextFragment>,
    input: ContentFragmentInput<'_>,
) -> Result<(), AgentLoopError> {
    fragments.extend(input.into_fragment()?);
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

fn append_delivery_fragment(
    candidate: &mut ModelInputIR,
    delivery_map: &mut ContextDeliveryMap,
    planner: &ContextPlanner,
    fragment: ContextFragment,
) -> Result<(), AgentLoopError> {
    let (items, entries) = planner.render_additive_fragment(&fragment)?;
    candidate.content_items.extend(items);
    delivery_map.append_content_entries(entries)?;
    Ok(())
}

fn append_delivery_items(
    candidate: &mut ModelInputIR,
    delivery_map: &mut ContextDeliveryMap,
    planner: &ContextPlanner,
    input: ContentFragmentInput<'_>,
) -> Result<(), AgentLoopError> {
    if let Some(fragment) = input.into_fragment()? {
        append_delivery_fragment(candidate, delivery_map, planner, fragment)?;
    }
    Ok(())
}
