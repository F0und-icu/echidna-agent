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
    pub(super) frozen_base_fragments: Option<&'a [ContextFragment]>,
    pub(super) world_state: Option<&'a ResearchWorldState>,
    pub(super) project_snapshot: Option<&'a ProjectSnapshotContext>,
    pub(super) project_memory_core: Option<&'a ContextFragment>,
    pub(super) session_summary: Option<&'a SessionSummary>,
    pub(super) history: &'a [ContentItem],
    pub(super) history_provenance: &'a BTreeMap<usize, ContextProvenance>,
    pub(super) selected_history: Option<&'a SelectedHistory>,
    pub(super) selected_constraints: &'a [ContextFragment],
    pub(super) protected_input: Option<(&'a [ContentItem], &'a BTreeMap<usize, ContextProvenance>)>,
    pub(super) turn_history_start: usize,
    pub(super) detached_input: Option<(&'a [ContentItem], &'a BTreeMap<usize, ContextProvenance>)>,
    pub(super) planner: &'a ContextPlanner,
    pub(super) cache: Option<&'a dyn ContextFragmentCache>,
    pub(super) budget: ContextBudget,
    pub(super) capabilities: &'a ModelCapabilities,
    pub(super) tool_specs: &'a [ToolSpec],
    pub(super) prior_manifest: Option<&'a FullLogicalManifest>,
    pub(super) correction_feedback: &'a [ContextFragment],
    pub(super) capacity_breakdown:
        Option<&'a mut Option<crate::domain::context::ContextCapacityBreakdown>>,
}

/// Exact, event-bound History delivery chosen for this task. The original
/// complete History continues to drive persistence and execution pairing.
pub(super) struct SelectedHistory {
    items: Vec<ContentItem>,
    provenance: BTreeMap<usize, ContextProvenance>,
}

/// Source-bound task material follows its exact delivered operator source.
pub(super) enum TaskFragment {
    Fixed(ContextFragment),
    Constraint(crate::domain::events::EventId, ContextFragment),
    Progress(
        crate::domain::events::EventId,
        ContextFragment,
        Box<(ContextFragment, ContextFragment)>,
    ),
}

#[derive(Default)]
pub(super) struct TaskFragments {
    fragments: Vec<TaskFragment>,
    operator_input_dedup: bool,
}

impl TaskFragments {
    pub(super) fn push(&mut self, fragment: TaskFragment) {
        self.fragments.push(fragment);
    }

    pub(super) fn extend_fixed(&mut self, fragments: impl IntoIterator<Item = ContextFragment>) {
        self.fragments
            .extend(fragments.into_iter().map(TaskFragment::Fixed));
    }

    pub(super) fn for_history(
        &self,
        history: Option<&SelectedHistory>,
        operator_provenance: &[&BTreeMap<usize, ContextProvenance>],
    ) -> Vec<ContextFragment> {
        let selected = |id: &crate::domain::events::EventId| {
            history.is_some_and(|history| history.contains_exact_source(id))
        };
        let operator = |id: &crate::domain::events::EventId| {
            self.operator_input_dedup
                && operator_provenance
                    .iter()
                    .flat_map(|map| map.values())
                    .any(|provenance| {
                        provenance.origin() == crate::domain::context::ContextOrigin::OperatorInput
                            && provenance.references().source_event_refs().last() == Some(id)
                    })
        };
        self.fragments
            .iter()
            .filter_map(|fragment| match fragment {
                TaskFragment::Fixed(value) => Some(value.clone()),
                TaskFragment::Constraint(id, value) => {
                    (!(selected(id) || operator(id))).then(|| value.clone())
                }
                TaskFragment::Progress(id, with_goal, without_goal) => Some(
                    if selected(id) {
                        &without_goal.0
                    } else if operator(id) {
                        &without_goal.1
                    } else {
                        with_goal
                    }
                    .clone(),
                ),
            })
            .collect()
    }
}

impl SelectedHistory {
    fn contains_exact_source(&self, id: &crate::domain::events::EventId) -> bool {
        self.provenance.values().any(|provenance| {
            let refs = provenance.references().source_event_refs();
            refs.len() == 1 && &refs[0] == id
        })
    }
    pub(super) fn without_selected_protected(
        &self,
        protected: &(Vec<ContentItem>, BTreeMap<usize, ContextProvenance>),
    ) -> (Vec<ContentItem>, BTreeMap<usize, ContextProvenance>) {
        let selected = self
            .provenance
            .values()
            .flat_map(|value| value.references().source_event_refs())
            .collect::<std::collections::BTreeSet<_>>();
        let mut items = Vec::new();
        let mut provenance = BTreeMap::new();
        for (index, item) in protected.0.iter().enumerate() {
            let source = &protected.1[&index];
            if source
                .references()
                .source_event_refs()
                .last()
                .is_some_and(|id| selected.contains(id))
            {
                continue;
            }
            provenance.insert(items.len(), source.clone());
            items.push(item.clone());
        }
        (items, provenance)
    }
    pub(super) fn from_resume(
        history: &[crate::domain::session::TaskResumeHistoryItem],
        source_hash: &str,
        producer: &str,
    ) -> Result<Self, AgentLoopError> {
        use crate::domain::context::{ContextSlot, FreshnessIdentity, RetentionClass};
        let mut items = Vec::with_capacity(history.len());
        let mut provenance = BTreeMap::new();
        for source in history {
            let artifacts = match &source.item {
                ContentItem::ImageArtifact { artifact_id, .. } => vec![artifact_id.clone()],
                _ => Vec::new(),
            };
            let references = task_source_references(
                source.vision_references.as_ref(),
                artifacts,
                Vec::new(),
                source.source_refs.clone(),
            )?;
            let value = ContextProvenance::new(
                ContextSlot::History,
                source.semantics.origin,
                source.semantics.class,
                source.semantics.authority,
                RetentionClass::Recent,
                FreshnessIdentity::new(source_hash)?.with_producer_version(
                    if source.source_refs.len() > 1
                        && source.semantics.class
                            == crate::domain::context::SemanticClass::DerivedProjection
                    {
                        "task-context-resume:1"
                    } else {
                        producer
                    },
                ),
                references,
            )?;
            provenance.insert(items.len(), value);
            items.push(source.item.clone());
        }
        Ok(Self { items, provenance })
    }
    pub(super) fn retained_for_compaction(
        &self,
        compaction: &crate::domain::session::SessionCompaction,
    ) -> Result<Self, AgentLoopError> {
        let retained = compaction.task_history_retention()?;
        let mut items = Vec::new();
        let mut provenance = BTreeMap::new();
        for (index, item) in self.items.iter().enumerate() {
            let source = self
                .provenance
                .get(&index)
                .ok_or(AgentLoopError::ContextStateMismatch)?;
            let keep = retained.retains(source.references().source_event_refs());
            if keep {
                provenance.insert(items.len(), source.clone());
                items.push(item.clone());
            }
        }
        Ok(Self { items, provenance })
    }
}

fn task_source_references(
    vision: Option<&crate::domain::context::ContextReferences>,
    mut artifacts: Vec<crate::domain::artifacts::ArtifactId>,
    mut receipts: Vec<crate::domain::research::ExecutionReceiptId>,
    mut events: Vec<crate::domain::events::EventId>,
) -> Result<crate::domain::context::ContextReferences, crate::domain::context::ContextPlannerError>
{
    let mut research = Vec::new();
    if let Some(vision) = vision {
        research.extend_from_slice(vision.research_fact_refs());
        for id in vision.artifact_refs() {
            if !artifacts.contains(id) {
                artifacts.push(id.clone());
            }
        }
        for id in vision.execution_receipt_refs() {
            if !receipts.contains(id) {
                receipts.push(id.clone());
            }
        }
        for id in vision.source_event_refs() {
            if !events.contains(id) {
                events.push(id.clone());
            }
        }
    }
    crate::domain::context::ContextReferences::new(research, artifacts, receipts, events)
}

/// Assemble the exact operator inputs used by offline compaction/definition
/// previews, including earlier retained inputs that remain uncompleted.
pub(super) struct PreviewOperatorContext {
    pub(super) turn_history_start: usize,
    pub(super) detached: Option<(Vec<ContentItem>, BTreeMap<usize, ContextProvenance>)>,
    pub(super) protected: Option<(Vec<ContentItem>, BTreeMap<usize, ContextProvenance>)>,
    pub(super) active_provenance: BTreeMap<usize, ContextProvenance>,
}

pub(super) fn preview_operator_context(
    history: &mut Vec<ContentItem>,
    protected: &crate::domain::session::ProtectedOperatorContext,
    selected: Option<&SelectedHistory>,
) -> Result<PreviewOperatorContext, AgentLoopError> {
    if let Some(index) = protected.retained_provenance.keys().next_back().copied() {
        let active_provenance =
            BTreeMap::from([(0, protected.retained_provenance[&index].clone())]);
        let delivered = match selected {
            Some(selected) => {
                let all = protected.all_without_active(&active_provenance, history)?;
                selected.without_selected_protected(&all)
            }
            // Without a selected projection, the ordinary History fragment
            // already contains earlier retained operator inputs.
            None => protected.without_active(&active_provenance),
        };
        Ok(PreviewOperatorContext {
            turn_history_start: index,
            detached: None,
            protected: Some(delivered),
            active_provenance,
        })
    } else if !protected.detached_items.is_empty() {
        Ok(PreviewOperatorContext {
            turn_history_start: history.len(),
            detached: Some((
                protected.detached_items.clone(),
                protected.detached_provenance.clone(),
            )),
            protected: None,
            active_provenance: protected.detached_provenance.clone(),
        })
    } else {
        let turn_history_start = history.len();
        history.push(ContentItem::user_text("Continue."));
        Ok(PreviewOperatorContext {
            turn_history_start,
            detached: None,
            protected: None,
            active_provenance: BTreeMap::new(),
        })
    }
}

pub(super) fn selected_summary<'a>(
    summary: Option<&'a SessionSummary>,
    reference: Option<&str>,
) -> Result<Option<&'a SessionSummary>, AgentLoopError> {
    match reference {
        None => Ok(None),
        Some(reference) => summary
            .filter(|summary| {
                summary.selected_task_id().is_some()
                    && reference == format!("summary:{}", summary.fingerprint())
            })
            .map(Some)
            .ok_or(AgentLoopError::ContextStateMismatch),
    }
}

/// Render exact selected Core sections while retaining their original
/// namespace, revision, byte ranges and content hashes in the provenance map.
pub(super) fn selected_core_fragment(
    original: Option<&ContextFragment>,
    ranges: &[crate::domain::task_context::CoreRange],
) -> Result<Option<ContextFragment>, AgentLoopError> {
    if ranges.is_empty() {
        return Ok(None);
    }
    let original = original.ok_or(AgentLoopError::ContextStateMismatch)?;
    let ContextFragmentContent::ContentItems { items } = original.content() else {
        return Err(AgentLoopError::ContextStateMismatch);
    };
    let [ContentItem::Text { text, .. }] = items.as_slice() else {
        return Err(AgentLoopError::ContextStateMismatch);
    };
    let mut selected = Vec::with_capacity(ranges.len());
    let mut item_provenance = Vec::with_capacity(ranges.len());
    let mut previous_end = 0;
    for range in ranges {
        if range.fragment_id != original.id().as_str()
            || range.start_byte < previous_end
            || range.start_byte >= range.end_byte
            || range.end_byte > text.len()
            || !text.is_char_boundary(range.start_byte)
            || !text.is_char_boundary(range.end_byte)
        {
            return Err(AgentLoopError::ContextStateMismatch);
        }
        let content = &text[range.start_byte..range.end_byte];
        if crate::domain::canonical_json::canonical_blake3_hash(&content)
            .map_err(|_| AgentLoopError::ContextStateMismatch)?
            != range.content_hash
        {
            return Err(AgentLoopError::ContextStateMismatch);
        }
        let original_provenance = original.provenance();
        let provenance = ContextProvenance::new(
            original_provenance.slot(),
            original_provenance.origin(),
            original_provenance.semantic_class(),
            original_provenance.authority_level(),
            original_provenance.retention_class(),
            crate::domain::context::FreshnessIdentity::new(format!(
                "{}#{}..{}:{}",
                range.fragment_id, range.start_byte, range.end_byte, range.content_hash
            ))?
            .with_producer_version("task-context-core:1"),
            original_provenance.references().clone(),
        )?;
        item_provenance.push(provenance);
        selected.push(ContentItem::user_text(content));
        previous_end = range.end_byte;
    }
    let hash = crate::domain::canonical_json::canonical_blake3_hash(&ranges)
        .map_err(|_| AgentLoopError::ContextStateMismatch)?;
    let mut fragment = ContextFragment::new(
        ContextFragmentId::new(format!("memory-core-selected:{hash}"))?,
        original.source(),
        ContextFragmentPriority::Required,
        FragmentCachePolicy::DoNotCache,
        ContextFragmentContent::ContentItems { items: selected },
    )?
    .with_provenance(original.provenance().clone())?;
    for (ordinal, provenance) in item_provenance.into_iter().enumerate() {
        fragment = fragment.with_content_item_provenance(
            u32::try_from(ordinal).map_err(|_| AgentLoopError::ContextStateMismatch)?,
            provenance,
        )?;
    }
    Ok(Some(fragment))
}

pub(super) fn selected_constraint_fragments(
    sources: &[(crate::domain::task_context::ConstraintRange, String)],
) -> Result<Vec<ContextFragment>, AgentLoopError> {
    use crate::domain::context::{
        AuthorityLevel, ContextOrigin, ContextReferences, ContextSlot, FreshnessIdentity,
        RetentionClass, SemanticClass,
    };
    let mut fragments = Vec::with_capacity(sources.len());
    for (index, (range, content)) in sources.iter().enumerate() {
        let id = crate::domain::canonical_json::canonical_blake3_hash(range)
            .map_err(|_| AgentLoopError::ContextStateMismatch)?;
        let provenance = ContextProvenance::new(
            ContextSlot::History,
            ContextOrigin::OperatorInput,
            SemanticClass::Conversation,
            AuthorityLevel::OperatorAssertion,
            RetentionClass::Stable,
            FreshnessIdentity::new(format!(
                "{}#{}..{}:{}",
                range.source_event_id, range.start_byte, range.end_byte, range.content_hash
            ))?
            .with_producer_version("task-context-constraint:1"),
            ContextReferences::new(
                Vec::new(),
                Vec::new(),
                Vec::new(),
                vec![range.source_event_id.clone()],
            )?,
        )?;
        fragments.push(
            ContextFragment::new(
                ContextFragmentId::new(format!("task-constraint:{index:08}:{id}"))?,
                ContextFragmentSource::RecentHistory,
                ContextFragmentPriority::Required,
                FragmentCachePolicy::DoNotCache,
                ContextFragmentContent::ContentItems {
                    items: vec![ContentItem::user_text(content)],
                },
            )?
            .with_provenance(provenance.clone())?
            .with_content_item_provenance(0, provenance)?,
        );
    }
    Ok(fragments)
}

pub(super) fn selected_excerpt_fragments(
    excerpts: &[crate::domain::task_context::SourceExcerpt],
) -> Result<Vec<ContextFragment>, AgentLoopError> {
    use crate::domain::context::{ContextSlot, FreshnessIdentity, RetentionClass};
    let mut fragments = Vec::with_capacity(excerpts.len());
    for (index, excerpt) in excerpts.iter().enumerate() {
        let range = &excerpt.range;
        let id = crate::domain::canonical_json::canonical_blake3_hash(range)
            .map_err(|_| AgentLoopError::ContextStateMismatch)?;
        let receipts = excerpt
            .receipt_event_id
            .iter()
            .map(|id| crate::domain::research::ExecutionReceiptId::new(id.as_str()))
            .collect();
        let provenance = ContextProvenance::new(
            ContextSlot::History,
            excerpt.semantics.origin,
            excerpt.semantics.class,
            excerpt.semantics.authority,
            RetentionClass::Recent,
            FreshnessIdentity::new(format!(
                "{}#{}..{}:{}",
                range.source_event_id, range.start_byte, range.end_byte, range.content_hash
            ))?
            .with_producer_version("task-context-excerpt:1"),
            task_source_references(
                excerpt.vision_references.as_ref(),
                Vec::new(),
                receipts,
                vec![range.source_event_id.clone()],
            )?,
        )?;
        let text = format!(
            "Source excerpt {} bytes {}..{}; role: {:?}; origin: {:?}; authority: {:?}; recorded_at: {}; valid_time: unknown:\n{}",
            range.source_event_id,
            range.start_byte,
            range.end_byte,
            excerpt.role,
            excerpt.semantics.origin,
            excerpt.semantics.authority,
            excerpt.observed_at,
            excerpt.text
        );
        fragments.push(
            ContextFragment::new(
                ContextFragmentId::new(format!("task-excerpt:{index:08}:{id}"))?,
                ContextFragmentSource::RecentHistory,
                ContextFragmentPriority::Required,
                FragmentCachePolicy::DoNotCache,
                ContextFragmentContent::ContentItems {
                    items: vec![ContentItem::user_text(text)],
                },
            )?
            .with_provenance(provenance.clone())?
            .with_content_item_provenance(0, provenance)?,
        );
    }
    Ok(fragments)
}

#[derive(Clone, Copy)]
enum ProgressGoalPlacement {
    Progress,
    SelectedHistory,
    OperatorInput,
}

fn selected_task_progress_fragments(
    selection: &crate::domain::session::TaskContextSelected,
    sources: &[crate::domain::session::TaskProgressSource],
    placement: ProgressGoalPlacement,
) -> Result<Vec<ContextFragment>, AgentLoopError> {
    use crate::domain::context::{
        AuthorityLevel, ContextOrigin, ContextReferences, ContextSlot, FreshnessIdentity,
        RetentionClass, SemanticClass,
    };
    sources.iter().map(|source| {
        let progress = match placement {
            ProgressGoalPlacement::Progress => &source.text,
            ProgressGoalPlacement::SelectedHistory => &source.text_when_goal_in_history,
            ProgressGoalPlacement::OperatorInput => &source.text_when_goal_in_operator_input,
        };
        let hash = crate::domain::canonical_json::canonical_blake3_hash(progress)
            .map_err(|_| AgentLoopError::ContextStateMismatch)?;
        let provenance = ContextProvenance::new(ContextSlot::History, ContextOrigin::SessionProjection,
            SemanticClass::DerivedProjection, AuthorityLevel::DerivedNonEvidence, RetentionClass::Recent,
            FreshnessIdentity::new(selection.source.stream_hash())?.with_producer_version("task-progress:1"),
            ContextReferences::new(Vec::new(), source.artifact_refs.clone(), source.receipt_refs.clone(), source.event_refs.clone())?)?;
        let text = format!("Source-bound task progress for {}; derived navigation, not an independent fact:\n{}", source.task_id, progress);
        Ok(ContextFragment::new(ContextFragmentId::new(format!("task-progress:{hash}"))?,
            ContextFragmentSource::RecentHistory, ContextFragmentPriority::Required, FragmentCachePolicy::DoNotCache,
            ContextFragmentContent::ContentItems { items: vec![ContentItem::user_text(text)] })?
            .with_provenance(provenance.clone())?.with_content_item_provenance(0, provenance)?)
    }).collect()
}

pub(super) fn selected_task_fragments(
    selection: &crate::domain::session::TaskContextSelected,
    constraints: &[(crate::domain::task_context::ConstraintRange, String)],
    excerpts: &[crate::domain::task_context::SourceExcerpt],
    progress: &[crate::domain::session::TaskProgressSource],
    research_units: &[crate::domain::task_context_research::ResearchTaskUnit],
    memory: &[crate::domain::session::TaskMemoryDocument],
) -> Result<TaskFragments, AgentLoopError> {
    let mut fragments = TaskFragments {
        operator_input_dedup: selection.has_complete_request_proof(),
        ..TaskFragments::default()
    };
    for ((range, _), fragment) in constraints
        .iter()
        .zip(selected_constraint_fragments(constraints)?)
    {
        fragments.push(TaskFragment::Constraint(
            range.source_event_id.clone(),
            fragment,
        ));
    }
    fragments.extend_fixed(selected_excerpt_fragments(excerpts)?);
    let with_goal =
        selected_task_progress_fragments(selection, progress, ProgressGoalPlacement::Progress)?;
    let in_history = selected_task_progress_fragments(
        selection,
        progress,
        ProgressGoalPlacement::SelectedHistory,
    )?;
    let in_operator = selected_task_progress_fragments(
        selection,
        progress,
        ProgressGoalPlacement::OperatorInput,
    )?;
    for (((source, full), selected), operator) in progress
        .iter()
        .zip(with_goal)
        .zip(in_history)
        .zip(in_operator)
    {
        fragments.push(TaskFragment::Progress(
            source.goal_source_event_id.clone(),
            full,
            Box::new((selected, operator)),
        ));
    }
    fragments.extend_fixed(task_gap_fragment(selection)?);
    fragments.push(TaskFragment::Fixed(task_input_time_fragment(selection)?));
    fragments.extend_fixed(live_process_fragment(selection)?);
    fragments.extend_fixed(selected_research_fragments(selection, research_units)?);
    fragments.extend_fixed(selected_memory_fragments(selection, memory)?);
    Ok(fragments)
}

pub(super) fn task_gap_fragment(
    selection: &crate::domain::session::TaskContextSelected,
) -> Result<Option<ContextFragment>, AgentLoopError> {
    use crate::domain::context::{
        AuthorityLevel, ContextOrigin, ContextReferences, ContextSlot, FreshnessIdentity,
        RetentionClass, SemanticClass,
    };
    if selection.unresolved_gaps.is_empty() {
        return Ok(None);
    }
    let text = format!(
        "Task context selection has the following unresolved source gaps; current operator input remains separately available in full:\n{}",
        selection.unresolved_gaps.join("\n")
    );
    let hash = crate::domain::canonical_json::canonical_blake3_hash(&selection.unresolved_gaps)
        .map_err(|_| AgentLoopError::ContextStateMismatch)?;
    let mut event_refs = vec![
        selection.input_ref.clone(),
        selection.capture_event_id.clone(),
    ];
    if let Some(sample_id) = &selection.sample_event_id {
        event_refs.push(sample_id.clone());
    }
    let provenance = ContextProvenance::new(
        ContextSlot::History,
        ContextOrigin::SessionProjection,
        SemanticClass::DerivedProjection,
        AuthorityLevel::DerivedNonEvidence,
        RetentionClass::Ephemeral,
        FreshnessIdentity::new(selection.source.stream_hash())?
            .with_producer_version("task-context-gaps:1"),
        ContextReferences::new(Vec::new(), Vec::new(), Vec::new(), event_refs)?,
    )?;
    Ok(Some(
        ContextFragment::new(
            ContextFragmentId::new(format!("task-context-gaps:{hash}"))?,
            ContextFragmentSource::RecentHistory,
            ContextFragmentPriority::Required,
            FragmentCachePolicy::DoNotCache,
            ContextFragmentContent::ContentItems {
                items: vec![ContentItem::user_text(text)],
            },
        )?
        .with_provenance(provenance.clone())?
        .with_content_item_provenance(0, provenance)?,
    ))
}

pub(super) fn selected_memory_fragments(
    selection: &crate::domain::session::TaskContextSelected,
    documents: &[crate::domain::session::TaskMemoryDocument],
) -> Result<Vec<ContextFragment>, AgentLoopError> {
    use crate::domain::context::{
        AuthorityLevel, ContextOrigin, ContextReferences, ContextSlot, FreshnessIdentity,
        RetentionClass, SemanticClass,
    };
    selection.selected_memory.iter().map(|reference| {
        let document = documents.iter().find(|document| {
            *reference == format!("memory:{}:{}", document.path, document.revision)
        }).ok_or(AgentLoopError::ContextStateMismatch)?;
        if document.revision != blake3::hash(document.content.as_bytes()).to_hex().to_string() {
            return Err(AgentLoopError::ContextStateMismatch);
        }
        let text = serde_json::json!({"kind":"task_memory", "reference":reference, "scope":document.scope, "content":document.content}).to_string();
        let provenance = ContextProvenance::new(
            ContextSlot::RetrievedContext, ContextOrigin::ProjectMemory, SemanticClass::DerivedProjection,
            AuthorityLevel::DerivedNonEvidence, RetentionClass::Stable,
            FreshnessIdentity::new(&document.revision)?.with_producer_version("task-memory:1"),
            ContextReferences::new(Vec::new(), Vec::new(), Vec::new(), vec![selection.capture_event_id.clone()])?,
        )?;
        Ok(ContextFragment::new(
            ContextFragmentId::new(format!("task-memory:{reference}"))?, ContextFragmentSource::RetrievedMemory,
            ContextFragmentPriority::Required, FragmentCachePolicy::ContentAddressed,
            ContextFragmentContent::ContentItems { items: vec![ContentItem::user_text(text)] },
        )?.with_provenance(provenance.clone())?.with_content_item_provenance(0, provenance)?)
    }).collect()
}

pub(super) fn selected_research_fragments(
    selection: &crate::domain::session::TaskContextSelected,
    units: &[crate::domain::task_context_research::ResearchTaskUnit],
) -> Result<Vec<ContextFragment>, AgentLoopError> {
    use crate::domain::context::{
        AuthorityLevel, ContextOrigin, ContextReferences, ContextSlot, FreshnessIdentity,
        RetentionClass, SemanticClass,
    };
    let closed = crate::domain::task_context_research::close_research_selection(
        units,
        &selection.selected_research,
    )
    .map_err(|_| AgentLoopError::ContextStateMismatch)?;
    if closed
        .iter()
        .map(|unit| &unit.reference)
        .ne(selection.selected_research.iter())
    {
        return Err(AgentLoopError::ContextStateMismatch);
    }
    closed.into_iter().map(|unit| {
        let text = serde_json::json!({"kind":"task_research_record", "reference":unit.reference, "source_project_hash":selection.source.project_stream_hash(), "value":unit.value}).to_string();
        let provenance = ContextProvenance::new(
            ContextSlot::ResearchState, ContextOrigin::ResearchProjection, SemanticClass::DerivedProjection,
            AuthorityLevel::DerivedNonEvidence, RetentionClass::Stable,
            FreshnessIdentity::new(selection.source.project_stream_hash())?.with_producer_version("task-research:1"),
            ContextReferences::new(Vec::new(), Vec::new(), Vec::new(), vec![selection.capture_event_id.clone()])?,
        )?;
        Ok(ContextFragment::new(
            ContextFragmentId::new(format!("task-research:{}", unit.reference))?, ContextFragmentSource::ResearchWorldState,
            ContextFragmentPriority::Required, FragmentCachePolicy::ContentAddressed,
            ContextFragmentContent::ContentItems { items: vec![ContentItem::user_text(text)] },
        )?.with_provenance(provenance.clone())?.with_content_item_provenance(0, provenance)?)
    }).collect()
}

pub(super) fn task_input_time_fragment(
    selection: &crate::domain::session::TaskContextSelected,
) -> Result<ContextFragment, AgentLoopError> {
    use crate::domain::context::{
        AuthorityLevel, ContextOrigin, ContextReferences, ContextSlot, FreshnessIdentity,
        RetentionClass, SemanticClass,
    };
    let encoded = serde_json::to_string(&selection.input_time)
        .map_err(|_| AgentLoopError::ContextStateMismatch)?;
    let hash = crate::domain::canonical_json::canonical_blake3_hash(&selection.input_time)
        .map_err(|_| AgentLoopError::ContextStateMismatch)?;
    let text = format!(
        "Input {} receive-time anchor (UTC Unix seconds and nanoseconds; host timezone observation): {encoded}. A trading day or other domain calendar requires its own source.",
        selection.input_ref
    );
    let provenance = ContextProvenance::new(
        ContextSlot::History,
        ContextOrigin::SessionProjection,
        SemanticClass::AuthoritativeReference,
        AuthorityLevel::FactReference,
        RetentionClass::Stable,
        FreshnessIdentity::new(hash.clone())?.with_producer_version("task-input-time:1"),
        ContextReferences::new(
            Vec::new(),
            Vec::new(),
            Vec::new(),
            vec![
                selection.input_boundary_event_id.clone(),
                selection.input_ref.clone(),
            ],
        )?,
    )?;
    Ok(ContextFragment::new(
        ContextFragmentId::new(format!("task-input-time:{hash}"))?,
        ContextFragmentSource::RecentHistory,
        ContextFragmentPriority::Required,
        FragmentCachePolicy::DoNotCache,
        ContextFragmentContent::ContentItems {
            items: vec![ContentItem::user_text(text)],
        },
    )?
    .with_provenance(provenance.clone())?
    .with_content_item_provenance(0, provenance)?)
}

pub(super) fn live_process_fragment(
    selection: &crate::domain::session::TaskContextSelected,
) -> Result<Option<ContextFragment>, AgentLoopError> {
    use crate::domain::context::{
        AuthorityLevel, ContextOrigin, ContextReferences, FreshnessIdentity, RetentionClass,
        SemanticClass,
    };
    if selection.live_processes.is_empty() {
        return Ok(None);
    }
    let encoded = serde_json::to_string(&selection.live_processes)
        .map_err(|_| AgentLoopError::ContextStateMismatch)?;
    let hash = crate::domain::canonical_json::canonical_blake3_hash(&selection.live_processes)
        .map_err(|_| AgentLoopError::ContextStateMismatch)?;
    let text = format!(
        "Volatile live process observations captured during task preparation; statuses may change, so poll a handle for current state. Each handle refers to its original start result and receipt: {encoded}"
    );
    let mut fragment = ContextFragment::new(
        ContextFragmentId::new(format!("task-live-processes:{hash}"))?,
        ContextFragmentSource::ProjectSnapshot,
        ContextFragmentPriority::Required,
        FragmentCachePolicy::DoNotCache,
        ContextFragmentContent::ContentItems {
            items: vec![ContentItem::user_text(text)],
        },
    )?;
    let mut events = selection
        .live_processes
        .iter()
        .map(|process| process.result_event_id.clone())
        .collect::<std::collections::BTreeSet<_>>();
    events.insert(selection.capture_event_id.clone());
    let receipts = selection
        .live_processes
        .iter()
        .map(|process| {
            crate::domain::research::ExecutionReceiptId::new(process.receipt_event_id.as_str())
        })
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let provenance = ContextProvenance::new(
        fragment.slot(),
        ContextOrigin::SessionProjection,
        SemanticClass::DerivedProjection,
        AuthorityLevel::DerivedNonEvidence,
        RetentionClass::Ephemeral,
        FreshnessIdentity::new(hash)?.with_producer_version("task-live-processes:1"),
        ContextReferences::new(
            Vec::new(),
            Vec::new(),
            receipts,
            events.into_iter().collect(),
        )?,
    )?;
    fragment = fragment
        .with_provenance(provenance.clone())?
        .with_content_item_provenance(0, provenance)?;
    Ok(Some(fragment))
}

/// One fixed, typed replacement for volatile facts before a later sampling
/// Step. It contains no ordering or delivery decision: ContextPlanner remains
/// the sole compiler after AgentLoop installs these candidates.
#[derive(Clone, Debug)]
pub(crate) struct CurrentContextState {
    world_state: Option<ResearchWorldState>,
    project_snapshot: ProjectSnapshotContext,
    project_memory_core: Option<ContextFragment>,
    research_revision: Option<String>,
    memory_revision: Option<String>,
}

impl CurrentContextState {
    pub(crate) fn new(
        world_state: Option<ResearchWorldState>,
        project_snapshot: ProjectSnapshotContext,
        project_memory_core: Option<ContextFragment>,
    ) -> Self {
        Self {
            world_state,
            project_snapshot,
            project_memory_core,
            research_revision: None,
            memory_revision: None,
        }
    }

    pub(crate) fn with_research_revision(mut self, revision: Option<String>) -> Self {
        self.research_revision = revision;
        self
    }

    pub(crate) fn with_memory_revision(mut self, revision: String) -> Self {
        self.memory_revision = Some(revision);
        self
    }

    pub(super) fn memory_revision(&self) -> Option<&str> {
        self.memory_revision.as_deref()
    }

    pub(super) fn research_revision(&self) -> Option<&str> {
        self.research_revision.as_deref()
    }

    pub(crate) fn world_state(&self) -> Option<&ResearchWorldState> {
        self.world_state.as_ref()
    }

    pub(crate) fn project_snapshot(&self) -> &ProjectSnapshotContext {
        &self.project_snapshot
    }

    pub(crate) fn project_memory_core(&self) -> Option<&ContextFragment> {
        self.project_memory_core.as_ref()
    }
}

/// The committed and current local facts needed to prove one state-diff
/// replacement is safe for incremental delivery.
pub(super) struct StateDiffDeliveryInput<'a> {
    pub(super) correction_feedback: &'a [ContextFragment],
    pub(super) selected_constraints: &'a [ContextFragment],
    pub(super) step_id: &'a str,
    pub(super) full_input: &'a ModelInputIR,
    pub(super) full_delivery_map: &'a ContextDeliveryMap,
    pub(super) current_plan: &'a ContextPlan,
    pub(super) planner: &'a ContextPlanner,
    pub(super) budget: ContextBudget,
    pub(super) committed_delivery: Option<&'a super::CommittedPrimaryDelivery>,
    pub(super) world_state: Option<&'a ResearchWorldState>,
    pub(super) project_snapshot: Option<&'a ProjectSnapshotContext>,
    pub(super) project_memory_core: Option<&'a ContextFragment>,
    pub(super) history: &'a [ContentItem],
    pub(super) history_provenance: &'a BTreeMap<usize, ContextProvenance>,
    pub(super) turn_history_start: usize,
}

/// The same delivery selection is used by the live loop and source replay.
pub(super) struct DeliveryChoiceInput<'a> {
    pub(super) state_diff: StateDiffDeliveryInput<'a>,
    pub(super) provider_session: &'a crate::domain::context::ProviderSession,
    pub(super) turn: &'a crate::domain::context::TurnContext,
    pub(super) request_index: usize,
    pub(super) capabilities: &'a ModelCapabilities,
    pub(super) renderer: &'a crate::domain::context::PromptRendererIdentity,
    pub(super) activation_or_expiry: bool,
    pub(super) expired_plan: Option<&'a crate::domain::tools::ResolvedToolPlan>,
}

pub(super) fn choose_delivery(
    choice: DeliveryChoiceInput<'_>,
) -> Result<
    (
        ModelInputIR,
        ContextDeliveryMap,
        crate::domain::context::StepSnapshot,
        crate::domain::context::BaselineAssessment,
    ),
    AgentLoopError,
> {
    use crate::domain::context::{
        BaselineAssessment, BaselineInvalidationReason, InternalRequestMetadata, StepSnapshot,
    };
    let full_input = choice.state_diff.full_input;
    let full_map = choice.state_diff.full_delivery_map;
    let committed = choice.state_diff.committed_delivery;
    let full_snapshot = StepSnapshot::capture_with_renderer(
        choice.turn,
        choice.request_index,
        full_input,
        choice.capabilities,
        choice.renderer,
        InternalRequestMetadata::default(),
    )?;
    let full_assessment = choice
        .provider_session
        .assess_delivery(&full_snapshot, choice.capabilities.incremental_input);
    let retains_reasoning = committed.is_some_and(|state| {
        state.input.content_items.iter().any(|item| {
            matches!(
                item,
                ContentItem::Reasoning { .. } | ContentItem::OpaqueContinuation { .. }
            )
        })
    });
    if (retains_reasoning
        || matches!(
            full_assessment,
            BaselineAssessment::FullRequest {
                reason: BaselineInvalidationReason::InputNotAppendOnly
            }
        ))
        && let Some((candidate, map)) = state_diff_delivery_input(choice.state_diff)?
    {
        let snapshot = StepSnapshot::capture_with_renderer(
            choice.turn,
            choice.request_index,
            &candidate,
            choice.capabilities,
            choice.renderer,
            InternalRequestMetadata::default(),
        )?;
        let assessment = choice
            .provider_session
            .assess_delivery(&snapshot, choice.capabilities.incremental_input);
        match assessment {
            BaselineAssessment::AppendOnlyCandidate { .. } => {
                return Ok((candidate, map, snapshot, assessment));
            }
            BaselineAssessment::FullRequest { reason }
                if reason.permits_full_input_relationship(full_input, &candidate)
                    && (reason != BaselineInvalidationReason::ToolsChanged
                        || candidate.is_reasoning_replay_of(full_input)
                        || (choice.activation_or_expiry
                            && committed.is_some_and(|previous| {
                                choice.provider_session.tool_change_preserves_delivery(
                                    &snapshot,
                                    &previous.input,
                                    &candidate,
                                    choice.expired_plan,
                                )
                            }))) =>
            {
                return Ok((candidate, map, snapshot, assessment));
            }
            BaselineAssessment::FullRequest { reason } if retains_reasoning => {
                return Err(AgentLoopError::ProviderContinuationUnavailable {
                    reason: ContinuationRejection::BaselineRejected { reason },
                });
            }
            _ => {}
        }
    }
    Ok((
        full_input.clone(),
        full_map.clone(),
        full_snapshot,
        full_assessment,
    ))
}

pub(super) fn project_delivery(
    input: &ModelInputIR,
    map: &ContextDeliveryMap,
    assessment: &crate::domain::context::BaselineAssessment,
) -> Result<(ModelInputIR, ContextDeliveryMap), AgentLoopError> {
    map.validate_against(input)?;
    match assessment {
        crate::domain::context::BaselineAssessment::FullRequest { .. } => {
            Ok((input.clone(), map.clone()))
        }
        crate::domain::context::BaselineAssessment::AppendOnlyCandidate { delta_items } => {
            let mut delta = input.clone();
            delta.content_items = delta_items.clone();
            let delta_map = map.for_incremental_delta(input, &delta)?;
            Ok((delta, delta_map))
        }
    }
}

pub(super) fn compile_context(
    input: ContextCompilationInput<'_>,
) -> Result<(ContextPlan, ContextCacheSummary), AgentLoopError> {
    let ContextCompilationInput {
        prompt_composition,
        frozen_base_fragments,
        world_state,
        project_snapshot,
        project_memory_core,
        session_summary,
        history,
        history_provenance,
        selected_history,
        selected_constraints,
        protected_input,
        turn_history_start,
        detached_input,
        planner,
        cache,
        budget,
        capabilities,
        tool_specs,
        prior_manifest,
        correction_feedback,
        capacity_breakdown,
    } = input;
    if history.len() < turn_history_start
        || (detached_input.is_none() && history.len() == turn_history_start)
    {
        return Err(AgentLoopError::ContextStateMismatch);
    }

    let mut cache_summary = ContextCacheSummary::default();
    let section_bytes = planner.window_policy(capabilities)?.state_section_bytes();
    let mut fragments = Vec::new();
    if let Some(frozen) = frozen_base_fragments {
        for fragment in frozen {
            push_cached_fragment(
                &mut fragments,
                &mut cache_summary,
                fragment.clone(),
                planner,
                cache,
                capabilities,
                budget,
            )?;
        }
    } else {
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
                world_state.full_fragment_with_limit(section_bytes)?,
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
                project_snapshot.full_fragment_with_limit(section_bytes)?,
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
    let (input_items, input_start, input_provenance, updates_start) = match detached_input {
        Some((items, provenance)) => (items, 0, provenance, turn_history_start),
        None => (
            &history[turn_history_start..=turn_history_start],
            turn_history_start,
            history_provenance,
            turn_history_start + 1,
        ),
    };
    if let Some((items, provenance)) = protected_input {
        push_content_fragment(
            &mut fragments,
            ContentFragmentInput {
                id: "input:protected".to_owned(),
                source: ContextFragmentSource::UserInput,
                items,
                history_start: 0,
                history_provenance: provenance,
            },
        )?;
    }
    let (recent_items, recent_provenance) = match selected_history {
        Some(selected) => (selected.items.as_slice(), &selected.provenance),
        None => (&history[..turn_history_start], history_provenance),
    };
    push_content_fragment(
        &mut fragments,
        ContentFragmentInput {
            id: "history:recent".to_owned(),
            source: ContextFragmentSource::RecentHistory,
            items: recent_items,
            history_start: 0,
            history_provenance: recent_provenance,
        },
    )?;
    fragments.extend_from_slice(selected_constraints);
    push_content_fragment(
        &mut fragments,
        ContentFragmentInput {
            id: "input:current-user".to_owned(),
            source: ContextFragmentSource::UserInput,
            items: input_items,
            history_start: input_start,
            history_provenance: input_provenance,
        },
    )?;
    push_content_fragment(
        &mut fragments,
        ContentFragmentInput {
            id: "turn:local-updates".to_owned(),
            source: ContextFragmentSource::TurnLocalUpdates,
            items: &history[updates_start..],
            history_start: updates_start,
            history_provenance,
        },
    )?;

    fragments.extend_from_slice(correction_feedback);
    if let Some(ContentItem::Text { text, .. }) = input_items.first()
        && let Some(experience) = prompt_composition.applicable_improvement(text)?
    {
        fragments.push(experience);
    }
    let plan = if let Some(breakdown) = capacity_breakdown {
        planner.compile_with_capacity_breakdown(
            fragments,
            budget,
            capabilities,
            tool_specs.to_vec(),
            ModelRequestRequirements::default(),
            prior_manifest,
            breakdown,
        )?
    } else {
        planner.compile(
            fragments,
            budget,
            capabilities,
            tool_specs.to_vec(),
            ModelRequestRequirements::default(),
            prior_manifest,
        )?
    };
    Ok((plan, cache_summary))
}

pub(super) fn state_diff_delivery_input(
    input: StateDiffDeliveryInput<'_>,
) -> Result<Option<(ModelInputIR, ContextDeliveryMap)>, AgentLoopError> {
    let StateDiffDeliveryInput {
        correction_feedback,
        selected_constraints,
        step_id,
        full_input,
        full_delivery_map,
        current_plan,
        planner,
        budget,
        committed_delivery,
        world_state,
        project_snapshot,
        project_memory_core,
        history,
        history_provenance,
        turn_history_start,
    } = input;
    let Some(committed) = committed_delivery else {
        return Ok(None);
    };
    let committed_input = &committed.input;
    let committed_delivery_map = &committed.map;
    let committed_manifest = &committed.context_manifest;
    let committed_history_len = committed.history_len;
    let committed_world_state = committed.world_state.as_ref();
    let committed_project_snapshot = committed.project_snapshot.as_ref();
    let committed_project_memory_core = committed.project_memory_core.as_ref();
    let retains_reasoning = committed_input.content_items.iter().any(|item| {
        matches!(
            item,
            ContentItem::Reasoning { .. } | ContentItem::OpaqueContinuation { .. }
        )
    });
    committed_delivery_map.validate_against(committed_input)?;
    if committed_history_len > history.len() || !plan_allows_history_append(current_plan) {
        return continuation_unavailable_or_none(
            retains_reasoning,
            ContinuationRejection::HistoryReplaced,
        );
    }

    let mut state_fragments = Vec::new();
    let section_bytes = planner
        .budget_policy()
        .for_budget(budget)
        .state_section_bytes();
    match (committed_world_state, world_state) {
        (Some(previous), Some(current)) => {
            let previous_full = previous.full_fragment_with_limit(section_bytes)?;
            let current_full = current.full_fragment_with_limit(section_bytes)?;
            if !fragment_is_completely_included(current_plan, &current_full)
                || !committed_fragment_was_completely_included(committed_manifest, &previous_full)
            {
                return continuation_unavailable_or_none(
                    retains_reasoning,
                    ContinuationRejection::ResearchProjectionChanged,
                );
            }
            if let Some(fragment) = current.diff_fragment_with_limit(previous, section_bytes)? {
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
            let previous_full = previous.full_fragment_with_limit(section_bytes)?;
            let current_full = current.full_fragment_with_limit(section_bytes)?;
            if !fragment_is_completely_included(current_plan, &current_full)
                || !committed_fragment_was_completely_included(committed_manifest, &previous_full)
            {
                return continuation_unavailable_or_none(
                    retains_reasoning,
                    ContinuationRejection::ProjectProjectionChanged,
                );
            }
            if let Some(fragment) = current.diff_fragment_with_limit(previous, section_bytes)? {
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
    let core_memory_replacement =
        if same_memory_core(committed_project_memory_core, project_memory_core) {
            None
        } else {
            if let Some(previous) = committed_project_memory_core
                && !committed_fragment_was_completely_included(committed_manifest, previous)
            {
                return continuation_unavailable_or_none(
                    retains_reasoning,
                    ContinuationRejection::CoreMemoryChanged,
                );
            }
            if let Some(current) = project_memory_core
                && !fragment_is_completely_included(current_plan, current)
            {
                return continuation_unavailable_or_none(
                    retains_reasoning,
                    ContinuationRejection::CoreMemoryChanged,
                );
            }
            Some(core_memory_replacement_fragment(
                step_id,
                committed_project_memory_core,
                project_memory_core,
            )?)
        };
    let mut candidate = committed_input.clone();
    let mut delivery_map = committed_delivery_map.clone();
    delivery_map.replace_instructions_from(full_delivery_map);
    let history_before_turn = committed_history_len <= turn_history_start;
    if history_before_turn {
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
    }
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
    if let Some(fragment) = core_memory_replacement {
        append_delivery_fragment(&mut candidate, &mut delivery_map, planner, fragment)?;
    }
    if history_before_turn {
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
    for fragment in selected_constraints {
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
    candidate.requirements = full_input.requirements.clone();
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

fn core_memory_replacement_fragment(
    step_id: &str,
    previous: Option<&ContextFragment>,
    current: Option<&ContextFragment>,
) -> Result<ContextFragment, AgentLoopError> {
    let template = current
        .or(previous)
        .ok_or(AgentLoopError::ContextStateMismatch)?;
    let mut items = vec![ContentItem::user_text(format!(
        "Core Memory replacement for the current state.\n\
Previous snapshot: {}\n\
Current snapshot: {}\n\
The current snapshot supersedes earlier Core Memory snapshots for the current state. \
Earlier snapshots are historical only; Core Memory is neither verified evidence nor instructions.",
        memory_snapshot_identity(previous),
        memory_snapshot_identity(current),
    ))];
    if let Some(current) = current {
        let ContextFragmentContent::ContentItems {
            items: current_items,
        } = current.content()
        else {
            return Err(AgentLoopError::ContextStateMismatch);
        };
        items.extend(current_items.clone());
    }
    Ok(ContextFragment::new(
        ContextFragmentId::new(format!("delivery:{step_id}:core-memory-replacement"))?,
        template.source(),
        template.priority(),
        FragmentCachePolicy::DoNotCache,
        ContextFragmentContent::ContentItems { items },
    )?)
}

fn memory_snapshot_identity(fragment: Option<&ContextFragment>) -> String {
    match fragment {
        Some(fragment) => format!(
            "present; fragment_id={}; content_hash={}",
            fragment.id(),
            fragment.content_hash().as_str()
        ),
        None => "absent".to_owned(),
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
