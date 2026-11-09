//! Durable task-context sample and adoption facts.

use serde::{Deserialize, Serialize};

use crate::domain::artifacts::BlobId;
use crate::domain::context::ProviderSessionRef;
use crate::domain::events::EventId;
use crate::domain::history::HistoryDeliveryProjection;
use crate::domain::request_audit::{RequestAuditComponentSource, RequestAuditDeliveryMode};
use crate::domain::task_context::{
    ConstraintRange, ContextCandidate, CoreRange, SelectionCandidates, SelectionInput,
    SourceExcerpt, SourceRange, TaskCandidate, TaskSelectionDecision, UnitScope,
};

use super::{OperatorTurnBoundary, SessionError, SessionSourceIdentity, SessionState};

const TASK_SELECTION_INPUT_BYTES: usize = 16 * 1024;
fn is_zero(value: &u8) -> bool {
    *value == 0
}
pub(crate) const TASK_CANDIDATE_POLICY_VERSION: u16 = 4;
pub(crate) const TASK_REQUEST_PROOF_VERSION: u8 = 8;
type PostingReader<'a> =
    dyn Fn(bool, &str, usize, usize) -> Result<(Vec<usize>, bool), SessionError> + 'a;
type NavigationReader<'a> =
    dyn Fn(&crate::domain::model::ContentItem) -> Result<(String, bool), SessionError> + 'a;

pub(crate) trait TaskNavigationAccess {
    fn task_count(&self) -> usize;
    fn recent_cards(&self) -> Result<Vec<usize>, SessionError>;
    fn all_cards(&self) -> Result<Vec<usize>, SessionError>;
    fn task_ordinal(&self, id: &str) -> Result<Option<usize>, SessionError>;
    fn card(&self, ordinal: usize) -> Result<TaskCandidate, SessionError>;
    fn task_at(&self, index: usize) -> Result<Option<String>, SessionError>;
    fn semantics(
        &self,
        index: usize,
    ) -> Result<Option<super::HistorySourceSemantics>, SessionError>;
    fn related_history(
        &self,
        ids: &[String],
        before: usize,
        limit: usize,
    ) -> Result<Vec<usize>, SessionError>;
}

/// Version 1 of the bounded source-pool policy. Posting readers return the
/// newest ordinals in ascending order and report whether older hits exist.
/// Replay supplies the same tails reconstructed from the original source.
pub(crate) fn task_posting_pool<E>(
    query: &str,
    before: usize,
    required: &std::collections::BTreeSet<usize>,
    recent: impl Iterator<Item = usize>,
    mut posting: impl FnMut(&str, usize) -> Result<(Vec<usize>, bool), E>,
) -> Result<Option<(std::collections::BTreeSet<usize>, bool)>, E> {
    const MAX_KEYS: usize = 64;
    const POSTING_TAIL: usize = 16;
    const MAX_SOURCES: usize = 512;
    if required.len() > MAX_SOURCES {
        return Ok(None);
    }
    let keys = crate::domain::retrieval::task_posting_keys(query);
    let mut incomplete = keys.len() > MAX_KEYS;
    let mut lists = Vec::new();
    for key in keys.iter().take(MAX_KEYS) {
        let (values, omitted) = posting(key, POSTING_TAIL)?;
        incomplete |= omitted;
        lists.push(values);
    }
    let mut selected = required.clone();
    for ordinal in recent {
        if selected.len() == MAX_SOURCES {
            break;
        }
        selected.insert(ordinal);
    }
    for depth in 0..POSTING_TAIL {
        for values in &lists {
            if let Some(ordinal) = values.iter().rev().nth(depth) {
                if *ordinal < before && selected.len() < MAX_SOURCES {
                    selected.insert(*ordinal);
                } else if *ordinal < before && !selected.contains(ordinal) {
                    incomplete = true;
                }
            }
        }
    }
    Ok(Some((selected, incomplete)))
}

/// Mutable external fragments are frozen before the source prefix. Session
/// candidates are also recorded for replay comparison with the exact prefix.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TaskContextCapture {
    pub(crate) schema_version: u16,
    pub(crate) candidate_policy_version: u16,
    pub(crate) common: super::CompactionContextCapture,
    pub(crate) candidates: SelectionCandidates,
    pub(crate) expansion: Option<TaskSelectionDecision>,
    #[serde(
        default,
        skip_serializing_if = "crate::domain::agent_evolution::is_false"
    )]
    pub(crate) source_refresh: bool,
    pub(crate) compile: TaskCompileCapture,
    pub(crate) research_candidates:
        Vec<crate::domain::task_context_research::ResearchTaskCandidate>,
    pub(crate) research_omitted: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) memory: Vec<TaskMemoryDocument>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TaskMemoryDocument {
    pub(crate) scope: crate::domain::memory::MemoryScope,
    pub(crate) path: crate::domain::memory::MemoryPath,
    pub(crate) revision: String,
    pub(crate) content: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TaskCompileCapture {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) selection_prompt_version: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) selection_reasoning_effort: Option<crate::domain::model::ReasoningEffort>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) project_snapshot: Option<crate::domain::snapshot::ProjectSnapshot>,
    pub(crate) prompt: Vec<super::CapturedFragment>,
    pub(crate) research_enabled: bool,
    pub(crate) tool_specs: Vec<crate::domain::tools::ToolSpec>,
    pub(crate) tool_catalog: Vec<crate::domain::tools::ToolSpec>,
    pub(crate) resolved_tool_specs: Vec<crate::domain::tools::ToolSpec>,
    pub(crate) resolved_tool_plan_hash: String,
    pub(crate) live_processes: Vec<crate::domain::tools::LiveProcessObservation>,
    pub(crate) history_hash: String,
    pub(crate) history_provenance:
        std::collections::BTreeMap<usize, crate::domain::context::ContextProvenance>,
    pub(crate) protected_items: Vec<crate::domain::model::ContentItem>,
    pub(crate) protected_provenance:
        std::collections::BTreeMap<usize, crate::domain::context::ContextProvenance>,
    pub(crate) turn_history_start: usize,
    pub(crate) detached_input: Option<(
        Vec<crate::domain::model::ContentItem>,
        std::collections::BTreeMap<usize, crate::domain::context::ContextProvenance>,
    )>,
    pub(crate) correction_feedback: Vec<super::CapturedFragment>,
    pub(crate) control_directory: String,
}

impl TaskCompileCapture {
    pub(crate) fn selection_capabilities(
        &self,
        primary: &crate::domain::model::ModelCapabilities,
    ) -> crate::domain::model::ModelCapabilities {
        let mut capabilities = primary.clone();
        if let Some(effort) = &self.selection_reasoning_effort {
            capabilities.reasoning_effort = Some(effort.clone());
        }
        capabilities
    }

    pub(crate) fn research_state(
        &self,
        events: &[crate::domain::events::EventEnvelope<serde_json::Value>],
        references: &crate::domain::research::ResearchReferenceCatalog,
    ) -> Result<Option<crate::domain::research::ResearchState>, SessionError> {
        if !self.research_enabled {
            return Ok(None);
        }
        let state = crate::domain::project_stream::ProjectStreamReducer
            .reduce(events.iter().cloned(), references)
            .map_err(|error| SessionError::TaskContext {
                message: format!("Research source reduction failed: {error}"),
            })?;
        Ok(Some(state))
    }
}

/// An isolated selector output. The corresponding audited request and frozen
/// candidates are retained separately; this is not conversation History.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TaskSelectionSample {
    pub(crate) schema_version: u16,
    pub(crate) source: SessionSourceIdentity,
    pub(crate) capture_event_id: EventId,
    pub(crate) capture_blob: BlobId,
    pub(crate) input_ref: EventId,
    pub(crate) audit_event_id: EventId,
    pub(crate) provider_session: ProviderSessionRef,
    pub(crate) decision: TaskSelectionDecision,
}

/// A source-bound interpretation adopted before a primary request. It is a
/// derived navigation record and does not change the underlying facts.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TaskViewRevision {
    pub(crate) task_id: String,
    pub(crate) revision: u64,
    pub(crate) goal: String,
    /// Original current-input byte range, when this revision creates an
    /// objective from part of one input. The full input remains unchanged.
    pub(crate) input_range: Option<(usize, usize)>,
}

pub(crate) struct TaskProgressSource {
    pub(crate) task_id: String,
    pub(crate) text: String,
    pub(crate) text_when_goal_in_history: String,
    pub(crate) text_when_goal_in_operator_input: String,
    pub(crate) goal_source_event_id: EventId,
    pub(crate) event_refs: Vec<EventId>,
    pub(crate) receipt_refs: Vec<crate::domain::research::ExecutionReceiptId>,
    pub(crate) artifact_refs: Vec<crate::domain::artifacts::ArtifactId>,
}

pub(crate) struct TaskResumeHistoryItem {
    pub(crate) item: crate::domain::model::ContentItem,
    pub(crate) source_refs: Vec<EventId>,
    pub(crate) semantics: super::HistorySourceSemantics,
    pub(crate) vision_references: Option<crate::domain::context::ContextReferences>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CoreScopeRevision {
    pub(crate) reference: String,
    pub(crate) scope: UnitScope,
    #[serde(
        default,
        skip_serializing_if = "crate::domain::agent_evolution::is_false"
    )]
    pub(crate) suppressed: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConstraintRevision {
    pub(crate) reference: String,
    pub(crate) range: ConstraintRange,
    pub(crate) scope: UnitScope,
    pub(crate) revision: u64,
    pub(crate) supersedes: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LiveProcessReference {
    pub(crate) observation: crate::domain::tools::LiveProcessObservation,
    pub(crate) result_event_id: EventId,
    pub(crate) receipt_event_id: EventId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TaskContextSelected {
    pub(crate) schema_version: u16,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub(crate) request_proof_version: u8,
    pub(crate) source: SessionSourceIdentity,
    pub(crate) capture_event_id: EventId,
    pub(crate) capture_blob: BlobId,
    pub(crate) input_ref: EventId,
    pub(crate) input_boundary_event_id: EventId,
    pub(crate) input_time: super::TaskInputTime,
    pub(crate) sample_event_id: Option<EventId>,
    /// A settled-batch source refresh reuses the prior task interpretation;
    /// it is not a new semantic sample or an execution retry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) refresh_of: Option<EventId>,
    pub(crate) decision: TaskSelectionDecision,
    pub(crate) selected_history: Vec<EventId>,
    pub(crate) selected_excerpts: Vec<SourceRange>,
    pub(crate) selected_core: Vec<CoreRange>,
    pub(crate) selected_research: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) selected_memory: Vec<String>,
    pub(crate) selected_summary: Option<String>,
    pub(crate) core_scopes: Vec<CoreScopeRevision>,
    pub(crate) constraint_revisions: Vec<ConstraintRevision>,
    pub(crate) selected_constraints: Vec<ConstraintRange>,
    pub(crate) unresolved_gaps: Vec<String>,
    pub(crate) live_processes: Vec<LiveProcessReference>,
    pub(crate) task_revisions: Vec<TaskViewRevision>,
    pub(crate) policy_version: u16,
    pub(crate) binding_digest: String,
    pub(crate) provider_session: ProviderSessionRef,
    pub(crate) planned_context_hash: String,
}

impl TaskContextSelected {
    pub(crate) fn has_complete_request_proof(&self) -> bool {
        self.request_proof_version != 0
    }

    fn has_multiple_objectives(&self) -> bool {
        self.task_revisions.len() > 1
    }
}

const MULTI_OBJECTIVE_PROGRESS_GAP: &str = "Observations from this multi-objective input have unresolved task attribution; select or read their original sources explicitly.";

fn invalid(message: &'static str) -> SessionError {
    SessionError::TaskContext {
        message: message.to_owned(),
    }
}

// A Core reference binds the namespace, whole-file revision, byte range and
// section content hash. Only the namespace and content hash can be compared
// across revisions; uniqueness is checked against both complete snapshots.
fn core_reference_identity(reference: &str) -> Option<(&str, &str)> {
    let (without_hash, hash) = reference.strip_prefix("core:")?.rsplit_once(':')?;
    let (without_end, end) = without_hash.rsplit_once(':')?;
    let (fragment, start) = without_end.rsplit_once(':')?;
    let (namespace, _) = fragment.rsplit_once(':')?;
    let (start, end) = (start.parse::<usize>().ok()?, end.parse::<usize>().ok()?);
    (start < end && crate::domain::agent_definition::is_blake3_digest(hash))
        .then_some((namespace, hash))
}

fn unique_prior_core_scope<'a>(
    scopes: &'a [CoreScopeRevision],
    reference: &str,
    current_matches: usize,
) -> Option<&'a CoreScopeRevision> {
    let (namespace, hash) = core_reference_identity(reference)?;
    if current_matches != 1 {
        return None;
    }
    let mut matches = scopes.iter().filter(|scope| {
        core_reference_identity(&scope.reference)
            .is_some_and(|(old_namespace, old_hash)| old_namespace == namespace && old_hash == hash)
    });
    let first = matches.next()?;
    matches.next().is_none().then_some(first)
}

fn excerpt(value: &str, maximum: usize) -> &str {
    let mut end = value.len().min(maximum);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

fn source_text(
    item: &crate::domain::model::ContentItem,
) -> Result<std::borrow::Cow<'_, str>, SessionError> {
    match item {
        crate::domain::model::ContentItem::Text { text, .. } => Ok(text.as_str().into()),
        _ => serde_json::to_string(item)
            .map(std::borrow::Cow::Owned)
            .map_err(|_| invalid("History source could not be encoded")),
    }
}

/// A disposable, bounded hot index inside the validated Session projection.
/// Cold entries remain in authoritative History and are expanded on demand.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct TaskNavigationIndex {
    entries: std::collections::VecDeque<(usize, Option<(String, bool)>)>,
}

/// Incremental derived task metadata. A late revision of an already sealed
/// input invalidates this projection and is rebuilt from the original facts.
#[derive(Default)]
pub(crate) struct TaskNavigationMetadata {
    tasks: std::collections::BTreeMap<String, TaskCardSource>,
    anchors: std::collections::BTreeMap<usize, String>,
    card_ordinals: std::collections::BTreeMap<usize, String>,
    task_ordinals: std::collections::BTreeMap<String, usize>,
    task_anchors: std::collections::BTreeMap<String, Vec<usize>>,
    recent: std::collections::VecDeque<String>,
    selection_cursor: usize,
    boundary_cursor: usize,
    sealed_before: usize,
}

#[derive(Clone, Copy)]
pub(crate) enum TaskCardSource {
    Legacy(usize),
    Revision { selection: usize, revision: usize },
}

#[derive(Default, Clone, Copy)]
pub(crate) struct TaskNavigationCursor {
    selection_cursor: usize,
    boundary_cursor: usize,
    sealed_before: usize,
}

pub(crate) enum TaskNavigationChange {
    Card {
        ordinal: usize,
        source: TaskCardSource,
        card: TaskCandidate,
    },
    Anchor {
        index: usize,
        task_id: String,
    },
    Recent(String),
}

impl TaskNavigationMetadata {
    fn card(&self, state: &SessionState, id: &str) -> TaskCandidate {
        match self.tasks[id] {
            TaskCardSource::Legacy(index) => {
                let crate::domain::model::ContentItem::Text { text, .. } =
                    &state.history.items()[index]
                else {
                    unreachable!("legacy task source was validated")
                };
                TaskCandidate {
                    reference: id.to_owned(),
                    goal: excerpt(text, 1024).to_owned(),
                    source_ref: state.history_event_ids[index].to_string(),
                }
            }
            TaskCardSource::Revision {
                selection,
                revision,
            } => {
                let selection = &state.task_context_selections[selection].1;
                let revision = &selection.task_revisions[revision];
                TaskCandidate {
                    reference: id.to_owned(),
                    goal: excerpt(&revision.goal, 1024).to_owned(),
                    source_ref: selection.input_ref.to_string(),
                }
            }
        }
    }

    pub(crate) fn cards_after<'a>(
        &'a self,
        state: &'a SessionState,
        after: Option<usize>,
    ) -> impl Iterator<Item = (usize, TaskCandidate)> + 'a {
        use std::ops::Bound::{Excluded, Unbounded};
        let start = after.map_or(Unbounded, Excluded);
        self.card_ordinals
            .range((start, Unbounded))
            .map(move |(ordinal, id)| (*ordinal, self.card(state, id)))
    }
}

struct ReplayedTaskNavigation<'a> {
    metadata: &'a TaskNavigationMetadata,
    state: &'a SessionState,
}

impl TaskNavigationAccess for ReplayedTaskNavigation<'_> {
    fn task_count(&self) -> usize {
        self.metadata.tasks.len()
    }
    fn recent_cards(&self) -> Result<Vec<usize>, SessionError> {
        Ok(self
            .metadata
            .recent
            .iter()
            .rev()
            .filter_map(|id| self.metadata.task_ordinals.get(id).copied())
            .collect())
    }
    fn all_cards(&self) -> Result<Vec<usize>, SessionError> {
        Ok(self.metadata.card_ordinals.keys().copied().collect())
    }
    fn task_ordinal(&self, id: &str) -> Result<Option<usize>, SessionError> {
        Ok(self.metadata.task_ordinals.get(id).copied())
    }
    fn card(&self, ordinal: usize) -> Result<TaskCandidate, SessionError> {
        let id = self
            .metadata
            .card_ordinals
            .get(&ordinal)
            .ok_or_else(|| invalid("task card is absent"))?;
        Ok(self.metadata.card(self.state, id))
    }
    fn task_at(&self, index: usize) -> Result<Option<String>, SessionError> {
        Ok(self
            .metadata
            .anchors
            .range(..=index)
            .next_back()
            .map(|(_, id)| id.clone()))
    }
    fn semantics(
        &self,
        _index: usize,
    ) -> Result<Option<super::HistorySourceSemantics>, SessionError> {
        Ok(None)
    }
    fn related_history(
        &self,
        ids: &[String],
        before: usize,
        limit: usize,
    ) -> Result<Vec<usize>, SessionError> {
        let mut values = Vec::new();
        for id in ids {
            if values.len() == limit {
                break;
            }
            if let Some(starts) = self.metadata.task_anchors.get(id) {
                for start in starts.iter().rev() {
                    let end = self
                        .metadata
                        .anchors
                        .range((
                            std::ops::Bound::Excluded(*start),
                            std::ops::Bound::Unbounded,
                        ))
                        .next()
                        .map_or(before, |(end, _)| (*end).min(before));
                    for index in (*start..end).rev().take(limit - values.len()) {
                        values.push(index);
                    }
                    if values.len() == limit {
                        break;
                    }
                }
            }
        }
        Ok(values)
    }
}

impl TaskNavigationIndex {
    const LIMIT: usize = 512;

    pub(crate) fn append(&mut self, index: usize, item: &crate::domain::model::ContentItem) {
        let value = if let crate::domain::model::ContentItem::Text { text, .. } = item {
            let prefix = excerpt(text, 1024);
            Some((prefix.to_owned(), prefix.len() < text.len()))
        } else {
            None
        };
        if self.entries.len() == Self::LIMIT {
            self.entries.pop_front();
        }
        self.entries.push_back((index, value));
    }

    pub(crate) fn get(&self, index: usize) -> Option<(&str, bool)> {
        let first = self.entries.front()?.0;
        self.entries
            .get(index.checked_sub(first)?)
            .filter(|(ordinal, _)| *ordinal == index)
            .and_then(|(_, value)| value.as_ref())
            .map(|(text, truncated)| (text.as_str(), *truncated))
    }
}

impl SessionState {
    pub(crate) fn append_current_visual_candidates(
        &self,
        input_ref: &EventId,
        candidates: &mut SelectionCandidates,
    ) -> Result<(), SessionError> {
        use crate::domain::context::SemanticClass;
        use crate::domain::model::ContentItem;
        use crate::domain::task_context::SelectionVisualSource;

        let mut remaining = TASK_SELECTION_INPUT_BYTES;
        for input in self.operator_inputs_for_anchor(input_ref)? {
            let text = match &input.item {
                ContentItem::ImageArtifact { .. } => None,
                ContentItem::Text { text, .. }
                    if input.provenance.semantic_class() == SemanticClass::ModelObservation =>
                {
                    Some(text.as_str())
                }
                _ => continue,
            };
            let inspected = text.map(|text| excerpt(text, remaining));
            let inspected_bytes = inspected.map_or(0, str::len);
            remaining -= inspected_bytes;
            let total_bytes = text.map_or(0, str::len);
            if text.is_none() {
                candidates.gaps.push(format!(
                    "Current image {} was not inspected by task selection",
                    input.event_id
                ));
            } else if inspected_bytes < total_bytes {
                candidates.gaps.push(format!(
                    "Current visual observation {} bytes {}..{} were not inspected",
                    input.event_id, inspected_bytes, total_bytes
                ));
            }
            candidates.input.visual_sources.push(SelectionVisualSource {
                history_event_ref: input.event_id.to_string(),
                source_event_refs: input
                    .provenance
                    .references()
                    .source_event_refs()
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
                artifact_refs: input
                    .provenance
                    .references()
                    .artifact_refs()
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
                observation_text: inspected.map(str::to_owned),
                inspected_bytes,
                total_bytes,
            });
        }
        Ok(())
    }
    pub(crate) fn task_navigation_rebuild_sources(
        &self,
        input_ref: &EventId,
        mut position: impl FnMut(&EventId) -> Result<usize, SessionError>,
        mut selection: impl FnMut(usize, usize) -> Result<(), SessionError>,
        mut boundary: impl FnMut(usize) -> Result<(), SessionError>,
    ) -> Result<TaskNavigationCursor, SessionError> {
        let input_index = position(input_ref)?;
        let mut selection_cursor = 0;
        for (_, value) in &self.task_context_selections {
            let index = position(&value.input_ref)?;
            if index > input_index {
                break;
            }
            selection(index, selection_cursor)?;
            selection_cursor += 1;
        }
        let mut boundary_cursor = 0;
        let cancelled = self.cancelled_initial_inputs();
        for value in &self.operator_boundaries {
            if let OperatorTurnBoundary::Accepted { anchor, .. } = value {
                let index = position(anchor)?;
                if index >= input_index {
                    break;
                }
                if !cancelled.contains(anchor) {
                    boundary(index)?;
                }
            }
            boundary_cursor += 1;
        }
        Ok(TaskNavigationCursor {
            selection_cursor,
            boundary_cursor,
            sealed_before: input_index,
        })
    }

    pub(crate) fn task_navigation_rebuild_selection(
        &self,
        index: usize,
        selection: usize,
        slot: &mut usize,
        mut emit: impl FnMut(TaskNavigationChange) -> Result<bool, SessionError>,
    ) -> Result<String, SessionError> {
        let value = &self.task_context_selections[selection].1;
        for (revision, task) in value.task_revisions.iter().enumerate() {
            let ordinal = index
                .checked_mul(65536)
                .and_then(|value| value.checked_add(*slot))
                .ok_or_else(|| invalid("task card ordinal overflow"))?;
            let source = TaskCardSource::Revision {
                selection,
                revision,
            };
            if emit(TaskNavigationChange::Card {
                ordinal,
                source,
                card: self.task_card_from_source(&task.task_id, source)?,
            })? {
                *slot += 1;
                if *slot > 65536 {
                    return Err(invalid("too many task cards for one input"));
                }
            }
            emit(TaskNavigationChange::Recent(task.task_id.clone()))?;
        }
        Ok(value.task_revisions[0].task_id.clone())
    }

    pub(crate) fn task_navigation_rebuild_legacy(
        &self,
        index: usize,
    ) -> Result<Option<(usize, TaskCardSource, TaskCandidate)>, SessionError> {
        if !matches!(
            self.history.items()[index],
            crate::domain::model::ContentItem::Text { .. }
        ) {
            return Ok(None);
        }
        let id = format!("legacy:{}", self.history_event_ids[index]);
        let source = TaskCardSource::Legacy(index);
        let ordinal = index
            .checked_mul(65536)
            .ok_or_else(|| invalid("task card ordinal overflow"))?;
        Ok(Some((
            ordinal,
            source,
            self.task_card_from_source(&id, source)?,
        )))
    }

    pub(crate) fn task_id_from_source(
        &self,
        source: TaskCardSource,
    ) -> Result<String, SessionError> {
        match source {
            TaskCardSource::Legacy(index) => {
                let event = self
                    .history_event_ids
                    .get(index)
                    .ok_or_else(|| invalid("task card History reference is absent"))?;
                Ok(format!("legacy:{event}"))
            }
            TaskCardSource::Revision {
                selection,
                revision,
            } => Ok(self
                .task_context_selections
                .get(selection)
                .and_then(|(_, selection)| selection.task_revisions.get(revision))
                .ok_or_else(|| invalid("task card revision source is absent"))?
                .task_id
                .clone()),
        }
    }

    pub(crate) fn task_card_from_source(
        &self,
        id: &str,
        source: TaskCardSource,
    ) -> Result<TaskCandidate, SessionError> {
        match source {
            TaskCardSource::Legacy(index) => {
                let item = self
                    .history
                    .items()
                    .get(index)
                    .ok_or_else(|| invalid("task card History source is absent"))?;
                let event = self
                    .history_event_ids
                    .get(index)
                    .ok_or_else(|| invalid("task card History reference is absent"))?;
                let crate::domain::model::ContentItem::Text { text, .. } = item else {
                    return Err(invalid("task card History source is not text"));
                };
                if id != format!("legacy:{event}") {
                    return Err(invalid("task card History identity changed"));
                }
                Ok(TaskCandidate {
                    reference: id.to_owned(),
                    goal: excerpt(text, 1024).to_owned(),
                    source_ref: event.to_string(),
                })
            }
            TaskCardSource::Revision {
                selection,
                revision,
            } => {
                let selection = &self
                    .task_context_selections
                    .get(selection)
                    .ok_or_else(|| invalid("task card selection source is absent"))?
                    .1;
                let revision = selection
                    .task_revisions
                    .get(revision)
                    .ok_or_else(|| invalid("task card revision source is absent"))?;
                if id != revision.task_id {
                    return Err(invalid("task card revision identity changed"));
                }
                Ok(TaskCandidate {
                    reference: id.to_owned(),
                    goal: excerpt(&revision.goal, 1024).to_owned(),
                    source_ref: selection.input_ref.to_string(),
                })
            }
        }
    }

    pub(crate) fn task_navigation_changes(
        &self,
        cursor: TaskNavigationCursor,
        input_ref: &EventId,
        anchor_task: &dyn Fn(usize) -> Result<Option<String>, SessionError>,
        card_exists: &dyn Fn(&str) -> Result<bool, SessionError>,
    ) -> Result<Option<(TaskNavigationCursor, Vec<TaskNavigationChange>)>, SessionError> {
        let input_index = self
            .history_event_ids
            .iter()
            .rposition(|id| id == input_ref)
            .ok_or_else(|| invalid("selection input is absent from History"))?;
        let mut updates =
            std::collections::BTreeMap::<usize, Vec<(usize, &TaskContextSelected)>>::new();
        let mut changes = Vec::new();
        let mut history_cursor = cursor.sealed_before.min(input_index);
        let mut selection_cursor = cursor.selection_cursor;
        for (_, selection) in &self.task_context_selections[selection_cursor..] {
            let index = self
                .task_history_position(&selection.input_ref, &mut history_cursor)
                .ok_or_else(|| invalid("task view input is absent from History"))?;
            if index > input_index {
                break;
            }
            let anchored = anchor_task(index)?;
            if index < cursor.sealed_before || anchored.is_some() {
                if selection.refresh_of.is_none()
                    || anchored.as_deref() != Some(selection.task_revisions[0].task_id.as_str())
                {
                    return Ok(None);
                }
                for revision in &selection.task_revisions {
                    if !card_exists(&revision.task_id)? {
                        return Ok(None);
                    }
                    changes.push(TaskNavigationChange::Recent(revision.task_id.clone()));
                }
                selection_cursor += 1;
                continue;
            }
            updates
                .entry(index)
                .or_default()
                .push((selection_cursor, selection));
            selection_cursor += 1;
        }
        let mut boundaries = std::collections::BTreeMap::<usize, &EventId>::new();
        let mut boundary_cursor = cursor.boundary_cursor;
        let cancelled = self.cancelled_initial_inputs();
        history_cursor = cursor.sealed_before.min(input_index);
        for boundary in &self.operator_boundaries[boundary_cursor..] {
            if let OperatorTurnBoundary::Accepted { anchor, .. } = boundary {
                let index = self
                    .task_history_position(anchor, &mut history_cursor)
                    .ok_or_else(|| invalid("accepted task input is absent from History"))?;
                if index >= input_index {
                    break;
                }
                if !cancelled.contains(anchor) {
                    boundaries.insert(index, anchor);
                }
            }
            boundary_cursor += 1;
        }
        let mut new_cards = std::collections::BTreeSet::new();
        for index in boundaries
            .keys()
            .chain(updates.keys())
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
        {
            if let Some(selections) = updates.get(&index) {
                let mut slot = 0usize;
                for (selection_index, _) in selections {
                    self.task_navigation_rebuild_selection(
                        index,
                        *selection_index,
                        &mut slot,
                        |change| {
                            if let TaskNavigationChange::Card { card, .. } = &change {
                                if new_cards.contains(&card.reference)
                                    || card_exists(&card.reference)?
                                {
                                    return Ok(false);
                                }
                                new_cards.insert(card.reference.clone());
                            }
                            changes.push(change);
                            Ok(true)
                        },
                    )?;
                }
                let selection = selections.last().expect("selection group is nonempty").1;
                changes.push(TaskNavigationChange::Anchor {
                    index,
                    task_id: selection.task_revisions[0].task_id.clone(),
                });
            } else if anchor_task(index)?.is_none()
                && let Some(anchor) = boundaries.get(&index)
                && matches!(
                    &self.history.items()[index],
                    crate::domain::model::ContentItem::Text { .. }
                )
            {
                let id = format!("legacy:{anchor}");
                let source = TaskCardSource::Legacy(index);
                let ordinal = index
                    .checked_mul(65536)
                    .ok_or_else(|| invalid("task card ordinal overflow"))?;
                changes.push(TaskNavigationChange::Card {
                    ordinal,
                    source,
                    card: self.task_card_from_source(&id, source)?,
                });
                changes.push(TaskNavigationChange::Recent(id.clone()));
                changes.push(TaskNavigationChange::Anchor { index, task_id: id });
            }
        }
        Ok(Some((
            TaskNavigationCursor {
                selection_cursor,
                boundary_cursor,
                sealed_before: input_index,
            },
            changes,
        )))
    }

    pub(crate) fn update_task_navigation_metadata(
        &self,
        metadata: &mut TaskNavigationMetadata,
        input_ref: &EventId,
        candidate_policy_version: u16,
    ) -> Result<bool, SessionError> {
        let input_index = self
            .history_event_ids
            .iter()
            .rposition(|id| id == input_ref)
            .ok_or_else(|| invalid("selection input is absent from History"))?;
        let mut updates =
            std::collections::BTreeMap::<usize, Vec<(usize, &TaskContextSelected)>>::new();
        let mut history_cursor = metadata.sealed_before.min(input_index);
        let mut selection_cursor = metadata.selection_cursor;
        for (_, selection) in &self.task_context_selections[selection_cursor..] {
            let index = self
                .task_history_position(&selection.input_ref, &mut history_cursor)
                .ok_or_else(|| invalid("task view input is absent from History"))?;
            if index > input_index {
                break;
            }
            if index < metadata.sealed_before || metadata.anchors.contains_key(&index) {
                *metadata = TaskNavigationMetadata::default();
                self.update_task_navigation_metadata(
                    metadata,
                    input_ref,
                    candidate_policy_version,
                )?;
                return Ok(true);
            }
            updates
                .entry(index)
                .or_default()
                .push((selection_cursor, selection));
            selection_cursor += 1;
        }
        let mut boundary_cursor = metadata.boundary_cursor;
        history_cursor = metadata.sealed_before.min(input_index);
        let mut boundaries = std::collections::BTreeMap::<usize, &EventId>::new();
        let cancelled = self.cancelled_initial_inputs();
        for boundary in &self.operator_boundaries[boundary_cursor..] {
            if let OperatorTurnBoundary::Accepted { anchor, .. } = boundary {
                let index = self
                    .task_history_position(anchor, &mut history_cursor)
                    .ok_or_else(|| invalid("accepted task input is absent from History"))?;
                if index >= input_index {
                    break;
                }
                if candidate_policy_version < 4 || !cancelled.contains(anchor) {
                    boundaries.insert(index, anchor);
                }
            }
            boundary_cursor += 1;
        }
        for index in boundaries
            .keys()
            .chain(updates.keys())
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
        {
            if let Some(selections) = updates.get(&index) {
                let mut slot = 0usize;
                for (selection_index, _) in selections {
                    self.task_navigation_rebuild_selection(
                        index,
                        *selection_index,
                        &mut slot,
                        |change| {
                            let inserted = match change {
                                TaskNavigationChange::Card {
                                    ordinal,
                                    source,
                                    card,
                                } => {
                                    if !metadata.tasks.contains_key(&card.reference) {
                                        metadata.tasks.insert(card.reference.clone(), source);
                                        metadata
                                            .card_ordinals
                                            .insert(ordinal, card.reference.clone());
                                        metadata.task_ordinals.insert(card.reference, ordinal);
                                        true
                                    } else {
                                        false
                                    }
                                }
                                TaskNavigationChange::Recent(id) => {
                                    metadata.recent.push_back(id);
                                    if metadata.recent.len() > 64 {
                                        metadata.recent.pop_front();
                                    }
                                    false
                                }
                                TaskNavigationChange::Anchor { .. } => {
                                    unreachable!("selection emits cards and recency")
                                }
                            };
                            Ok(inserted)
                        },
                    )?;
                }
                let id = selections
                    .last()
                    .expect("selection group is nonempty")
                    .1
                    .task_revisions[0]
                    .task_id
                    .clone();
                metadata.anchors.insert(index, id.clone());
                metadata.task_anchors.entry(id).or_default().push(index);
            } else if !metadata.anchors.contains_key(&index)
                && let Some(anchor) = boundaries.get(&index)
                && let crate::domain::model::ContentItem::Text { .. } = &self.history.items()[index]
            {
                let reference = format!("legacy:{anchor}");
                metadata
                    .tasks
                    .insert(reference.clone(), TaskCardSource::Legacy(index));
                let ordinal = index
                    .checked_mul(65536)
                    .ok_or_else(|| invalid("task card ordinal overflow"))?;
                metadata.card_ordinals.insert(ordinal, reference.clone());
                metadata.task_ordinals.insert(reference.clone(), ordinal);
                metadata.recent.push_back(reference.clone());
                if metadata.recent.len() > 64 {
                    metadata.recent.pop_front();
                }
                metadata.anchors.insert(index, reference);
                metadata
                    .task_anchors
                    .entry(format!("legacy:{anchor}"))
                    .or_default()
                    .push(index);
            }
        }
        metadata.selection_cursor = selection_cursor;
        metadata.boundary_cursor = boundary_cursor;
        metadata.sealed_before = input_index;
        Ok(false)
    }
    fn task_recovered_calls(&self) -> std::collections::BTreeSet<crate::domain::tools::ToolCallId> {
        self.history
            .items()
            .iter()
            .take(self.recovered_history_item_count)
            .filter_map(|item| match item {
                crate::domain::model::ContentItem::ToolCall { call } => Some(call.call_id.clone()),
                _ => None,
            })
            .collect()
    }
    /// Build a resumed task view through the existing History normalizer.
    /// Synthetic interruption markers cite the real Call and Resume events.
    pub(crate) fn task_resume_projection(
        &self,
        selection_event: &EventId,
        selection: &TaskContextSelected,
        events: &[crate::domain::events::EventEnvelope<serde_json::Value>],
    ) -> Result<(Vec<TaskResumeHistoryItem>, Option<String>), SessionError> {
        let end = events.partition_point(|event| event.sequence <= selection.source.watermark());
        let start = events
            .iter()
            .position(|event| &event.event_id == selection_event)
            .ok_or_else(|| invalid("resumed task selection event is absent"))?;
        let mut wanted = selection
            .selected_history
            .iter()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        wanted.insert(selection.input_ref.clone());
        wanted.extend(
            events[end..]
                .iter()
                .filter(|event| event.event_type.as_str() == "session.history_item_recorded")
                .map(|event| event.event_id.clone()),
        );
        let sequences = events
            .iter()
            .map(|event| (&event.event_id, event.sequence))
            .collect::<std::collections::BTreeMap<_, _>>();
        let mut summary = selection.selected_summary.clone();
        if selection.request_proof_version == 0 {
            if events[start + 1..]
                .iter()
                .any(|event| event.event_type.as_str() == "session.compaction_created")
                && let Some(compaction) = self.latest_compaction()
            {
                for id in compaction.summary().summarized_history() {
                    wanted.remove(id);
                }
                if compaction.summary().selected_task_id()
                    == selection
                        .task_revisions
                        .first()
                        .map(|revision| revision.task_id.as_str())
                {
                    summary = Some(format!("summary:{}", compaction.summary().fingerprint()));
                }
            }
        } else {
            for event in &events[start + 1..] {
                if let super::SessionEvent::CompactionCreated { compaction } =
                    super::SessionEvent::decode(&event.event_type, event.payload.clone())?
                    && compaction.summary().schema_version() == 7
                {
                    let retained = compaction.task_history_retention()?;
                    wanted.retain(|id| {
                        sequences
                            .get(id)
                            .is_some_and(|sequence| *sequence > compaction.source_event_sequence())
                            || retained.retains(std::slice::from_ref(id))
                    });
                    if compaction.summary().selected_task_id()
                        == selection
                            .task_revisions
                            .first()
                            .map(|revision| revision.task_id.as_str())
                    {
                        summary = Some(format!("summary:{}", compaction.summary().fingerprint()));
                    }
                }
            }
        }
        let protected = self.current_protected_operator_context()?;
        if let Some(start) = protected.retained_provenance.keys().next_back().copied() {
            let ids = self.normalized_history_event_ids()?;
            let current = self.compaction_source_history()?;
            let offset = ids
                .len()
                .checked_sub(current.len())
                .ok_or_else(|| invalid("resumed normalized History is inconsistent"))?;
            for id in ids[offset + start..].iter().flatten() {
                wanted.remove(id);
            }
        }
        let history = self.task_resume_history(
            &wanted.into_iter().collect::<Vec<_>>(),
            events,
            selection.request_proof_version,
        )?;
        Ok((history, summary))
    }

    pub(crate) fn task_resume_history(
        &self,
        wanted: &[EventId],
        events: &[crate::domain::events::EventEnvelope<serde_json::Value>],
        proof_version: u8,
    ) -> Result<Vec<TaskResumeHistoryItem>, SessionError> {
        use crate::domain::model::ContentItem;
        let normalized = self.full_normalized_history()?;
        let ids = self.normalized_history_event_ids()?;
        let mut remaining = wanted
            .iter()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        if remaining.len() != wanted.len() {
            return Err(invalid("duplicate resumed History source"));
        }
        let mut calls = std::collections::BTreeSet::new();
        for (id, item) in ids.iter().zip(&normalized) {
            if id.as_ref().is_some_and(|id| remaining.contains(id)) {
                match item {
                    ContentItem::ToolCall { call } => {
                        calls.insert(call.call_id.clone());
                    }
                    ContentItem::ToolResult { result } => {
                        calls.insert(result.call_id.clone());
                    }
                    _ => {}
                }
            }
        }
        let mut selected = Vec::new();
        for (id, item) in ids.iter().zip(&normalized) {
            let paired = match item {
                ContentItem::ToolCall { call } => calls.contains(&call.call_id),
                ContentItem::ToolResult { result } => calls.contains(&result.call_id),
                _ => false,
            };
            let direct = id.as_ref().is_some_and(|id| remaining.remove(id));
            if direct || paired {
                selected.push((id.as_ref(), item));
            }
        }
        if !remaining.is_empty() {
            return Err(invalid("resumed History source is absent"));
        }
        let content = selected
            .iter()
            .map(|(_, item)| (*item).clone())
            .collect::<Vec<_>>();
        crate::domain::history::ConversationHistory::from_complete_content_items(&content)?;
        crate::domain::history::normalize_content_items(&content)?;
        let real = selected
            .iter()
            .filter_map(|(id, item)| id.map(|id| (id, *item)))
            .collect::<Vec<_>>();
        let semantics = self.history_source_semantics_many(&real)?;
        let has_vision = real.iter().zip(&semantics).any(|((_, item), semantic)| {
            matches!(item, ContentItem::ImageArtifact { .. })
                || semantic.class == crate::domain::context::SemanticClass::ModelObservation
        });
        let followups = if has_vision {
            self.followup_vision_inputs()?
        } else {
            Vec::new()
        };
        let mut semantics = semantics.into_iter();
        selected
            .into_iter()
            .map(|(id, item)| {
                if let Some(id) = id {
                    let mut source_refs = vec![id.clone()];
                    if let Some(followup) =
                        followups.iter().find(|followup| &followup.event_id == id)
                    {
                        for reference in followup.provenance.references().source_event_refs() {
                            if !source_refs.contains(reference) {
                                source_refs.push(reference.clone());
                            }
                        }
                    }
                    return Ok(TaskResumeHistoryItem {
                        item: item.clone(),
                        source_refs,
                        semantics: semantics
                            .next()
                            .ok_or_else(|| invalid("resumed History semantics are absent"))?,
                        vision_references: if proof_version >= 6 {
                            self.history_vision_provenance(id, item)?
                                .map(|value| value.references().clone())
                        } else {
                            None
                        },
                    });
                }
                let ContentItem::ToolResult { result } = item else {
                    return Err(invalid("resumed marker is not a Tool Result"));
                };
                let (call_id, _) = self.history_items_with_event_ids().find(|(_, item)|
                matches!(item, ContentItem::ToolCall { call } if call.call_id == result.call_id))
                .ok_or_else(|| invalid("resumed marker has no original Call"))?;
                let (source, origin) = if let Some(record) = self
                    .execution_receipts()
                    .iter()
                    .find(|record| record.result.as_ref() == Some(result))
                {
                    (
                        record.event_id.clone(),
                        crate::domain::context::ContextOrigin::ToolResult,
                    )
                } else {
                    let call_index = events
                        .iter()
                        .position(|event| &event.event_id == call_id)
                        .ok_or_else(|| invalid("resumed marker Call event is absent"))?;
                    let mut resumed = None;
                    for event in &events[call_index + 1..] {
                        if matches!(
                            super::SessionEvent::decode(&event.event_type, event.payload.clone())?,
                            super::SessionEvent::Resumed { .. }
                                | super::SessionEvent::AgentDefinitionApplied { .. }
                        ) {
                            resumed = Some(event.event_id.clone());
                            break;
                        }
                    }
                    (
                        resumed
                            .ok_or_else(|| invalid("resumed marker has no recovery boundary"))?,
                        crate::domain::context::ContextOrigin::SessionProjection,
                    )
                };
                Ok(TaskResumeHistoryItem {
                    item: item.clone(),
                    source_refs: vec![call_id.clone(), source],
                    vision_references: None,
                    semantics: super::HistorySourceSemantics {
                        origin,
                        class: crate::domain::context::SemanticClass::DerivedProjection,
                        authority: crate::domain::context::AuthorityLevel::DerivedNonEvidence,
                    },
                })
            })
            .collect()
    }
    /// Selection and accepted-input facts usually advance through History in
    /// order. Search the remaining suffix first, retaining a full-prefix
    /// lookup for valid references to an earlier input.
    fn task_history_position(&self, id: &EventId, cursor: &mut usize) -> Option<usize> {
        let position = self.history_event_ids[*cursor..]
            .iter()
            .position(|candidate| candidate == id)
            .map(|offset| *cursor + offset)
            .or_else(|| {
                self.history_event_ids[..*cursor]
                    .iter()
                    .position(|candidate| candidate == id)
            })?;
        *cursor = position;
        Some(position)
    }

    fn task_navigation_excerpt<'a>(
        &'a self,
        index: usize,
        item: &'a crate::domain::model::ContentItem,
        read: &NavigationReader<'_>,
    ) -> Result<(std::borrow::Cow<'a, str>, bool), SessionError> {
        match self.task_navigation.get(index) {
            Some((text, truncated)) => Ok((text.into(), truncated)),
            None => match item {
                crate::domain::model::ContentItem::Text { text, .. } => {
                    let value = excerpt(text, 1024);
                    Ok((value.into(), value.len() < text.len()))
                }
                _ => read(item).map(|(text, truncated)| (text.into(), truncated)),
            },
        }
    }

    fn task_progress_goal(
        &self,
        task_id: &str,
        source_id: &EventId,
        input_range: Option<(usize, usize)>,
        superseded: &[(&ConstraintRevision, &str)],
        read: &NavigationReader<'_>,
    ) -> Result<(String, [usize; 2], [usize; 2], bool), SessionError> {
        let index = self
            .history_event_ids
            .iter()
            .position(|id| id == source_id)
            .ok_or_else(|| invalid("task objective source is absent"))?;
        let item = &self.history.items()[index];
        let crate::domain::model::ContentItem::Text { text, .. } = item else {
            return Err(invalid("task objective source is not text"));
        };
        let (start, end) = input_range.unwrap_or((0, text.len()));
        let source = text
            .get(start..end)
            .ok_or_else(|| invalid("task objective source range is invalid"))?;
        let inspected = if input_range.is_some() {
            excerpt(source, 1024).to_owned()
        } else {
            self.task_navigation_excerpt(index, item, read)?
                .0
                .into_owned()
        };
        let inspected_end = start + inspected.len();
        let mut ranges = superseded.iter().filter_map(|(revision, _)| {
            let applies = matches!(revision.scope, UnitScope::Global | UnitScope::Unknown)
                || matches!(&revision.scope, UnitScope::Task { task_ref } if task_ref == task_id);
            (applies && revision.range.source_event_id == *source_id
                && revision.range.start_byte < inspected_end && revision.range.end_byte > start)
                .then_some((revision.range.start_byte.max(start), revision.range.end_byte.min(inspected_end)))
        }).collect::<Vec<_>>();
        ranges.sort_unstable();
        let mut goal = String::new();
        let mut cursor = start;
        for (from, to) in ranges {
            if to <= cursor {
                continue;
            }
            if from > cursor {
                goal.push_str(&text[cursor..from]);
            }
            goal.push_str("[superseded constraint omitted]");
            cursor = to;
        }
        goal.push_str(&text[cursor..inspected_end]);
        Ok((
            goal,
            [start, end],
            [start, inspected_end],
            inspected_end < end,
        ))
    }
    pub(crate) fn latest_task_context_selection(&self) -> Option<(&EventId, &TaskContextSelected)> {
        self.task_context_selections
            .last()
            .map(|(id, selection)| (id, selection))
    }

    /// A selected task Summary retains the Research dependencies of the
    /// verified source selection; rebind their identities to the current
    /// Project prefix and resolve them through the original Research owner.
    pub(crate) fn selected_summary_research_refs(
        &self,
        decision: &TaskSelectionDecision,
        source_hash: &str,
    ) -> Result<Vec<String>, SessionError> {
        let Some(summary) = self
            .latest_compaction()
            .map(|compaction| compaction.summary())
        else {
            return Ok(Vec::new());
        };
        let reference = format!("summary:{}", summary.fingerprint());
        if !decision.required_context_refs.contains(&reference) {
            return Ok(Vec::new());
        }
        let event = summary
            .task_selection_event()
            .ok_or_else(|| invalid("selected task Summary has no source selection"))?;
        let selection = self
            .task_context_selections
            .iter()
            .find(|(id, _)| id == event)
            .map(|(_, selection)| selection)
            .ok_or_else(|| invalid("selected task Summary source selection is absent"))?;
        selection
            .selected_research
            .iter()
            .map(|reference| {
                let (identity, _) = reference
                    .rsplit_once(':')
                    .ok_or_else(|| invalid("Summary Research source reference is invalid"))?;
                Ok(format!("{identity}:{source_hash}"))
            })
            .collect()
    }

    fn task_refresh_projection<'a>(
        &'a self,
        prior_id: &EventId,
        previous: &'a TaskContextSelected,
        current_compaction: bool,
    ) -> (&'a [EventId], Option<&'a String>) {
        if current_compaction
            && let Some((_, history, summary)) = self
                .task_compacted_selection
                .as_ref()
                .filter(|(id, _, _)| id == prior_id)
        {
            return (history, Some(summary));
        }
        (
            &previous.selected_history,
            previous.selected_summary.as_ref(),
        )
    }

    /// Preserve an adopted task interpretation while rebinding only its
    /// already selected dependencies to the current captured source.
    pub(crate) fn deterministic_task_refresh(
        &self,
        input: &EventId,
        candidates: &SelectionCandidates,
        current_compaction: bool,
        proof_version: u8,
    ) -> Result<(EventId, TaskSelectionDecision), SessionError> {
        use crate::domain::task_context::{SourceRangeSelection, TaskFocus, TaskRelation};
        let (event_id, previous) = self
            .latest_task_context_selection()
            .filter(|(_, previous)| &previous.input_ref == input)
            .ok_or_else(|| invalid("source refresh has no adopted current-Turn task"))?;
        let focus = previous
            .task_revisions
            .first()
            .ok_or_else(|| invalid("source refresh has no task"))?;
        if !candidates
            .tasks
            .iter()
            .any(|candidate| candidate.reference == focus.task_id)
        {
            return Err(invalid(
                "source refresh task is not in the captured candidates",
            ));
        }
        let (selected_history, selected_summary) =
            self.task_refresh_projection(event_id, previous, current_compaction);
        let mut required = std::collections::BTreeSet::new();
        for id in selected_history {
            let candidate = candidates
                .context
                .iter()
                .find(|candidate| candidate.source_event_ref.as_deref() == Some(id.as_str()))
                .ok_or_else(|| invalid("source refresh History dependency is unavailable"))?;
            required.insert(candidate.reference.clone());
        }
        for reference in previous
            .selected_research
            .iter()
            .chain(&previous.selected_memory)
        {
            let (identity, _) = reference
                .rsplit_once(':')
                .ok_or_else(|| invalid("source refresh dependency reference is invalid"))?;
            let candidate = candidates
                .context
                .iter()
                .find(|candidate| {
                    candidate
                        .research_ref
                        .as_ref()
                        .or(candidate.memory_ref.as_ref())
                        .and_then(|value| value.rsplit_once(':'))
                        .is_some_and(|(key, _)| key == identity)
                })
                .ok_or_else(|| invalid("source refresh dependency is unavailable"))?;
            required.insert(candidate.reference.clone());
        }
        if let Some(summary) = selected_summary {
            let candidate = candidates
                .context
                .iter()
                .find(|candidate| candidate.summary_ref.as_ref() == Some(summary))
                .ok_or_else(|| invalid("source refresh Summary is unavailable"))?;
            required.insert(candidate.reference.clone());
        }
        for range in &previous.selected_core {
            let original_reference = format!(
                "core:{}:{}:{}:{}",
                range.fragment_id, range.start_byte, range.end_byte, range.content_hash
            );
            let matching = candidates
                .context
                .iter()
                .filter(|candidate| {
                    candidate
                        .core_range
                        .as_ref()
                        .is_some_and(|current| current.content_hash == range.content_hash)
                        && (proof_version < 6
                            || core_reference_identity(&candidate.reference)
                                .zip(core_reference_identity(&original_reference))
                                .is_some_and(|(current, original)| current.0 == original.0))
                })
                .collect::<Vec<_>>();
            let candidate = if proof_version >= 5 {
                let exact = matching
                    .iter()
                    .copied()
                    .find(|candidate| candidate.core_range.as_ref() == Some(range));
                if exact.is_none() && matching.len() > 1 {
                    return Err(invalid(
                        "source refresh Core dependency relocation is ambiguous",
                    ));
                }
                exact.or_else(|| {
                    if proof_version >= 6
                        && unique_prior_core_scope(
                            &previous.core_scopes,
                            &original_reference,
                            matching.len(),
                        )
                        .is_none()
                    {
                        None
                    } else {
                        matching.first().copied()
                    }
                })
            } else {
                matching.first().copied()
            };
            if let Some(candidate) = candidate {
                required.insert(candidate.reference.clone());
            }
        }
        let mut source_ranges = Vec::new();
        for range in &previous.selected_excerpts {
            let candidate = candidates
                .context
                .iter()
                .find(|candidate| {
                    candidate.source_event_ref.as_deref() == Some(range.source_event_id.as_str())
                })
                .ok_or_else(|| invalid("source refresh excerpt is unavailable"))?;
            if range.end_byte > candidate.excerpt.len() {
                return Err(invalid(
                    "source refresh excerpt is outside the captured preview",
                ));
            }
            source_ranges.push(SourceRangeSelection {
                context_ref: candidate.reference.clone(),
                start_byte: range.start_byte,
                end_byte: range.end_byte,
            });
        }
        let related = previous.decision.related_task_refs.clone();
        let decision = TaskSelectionDecision {
            relation: TaskRelation::Continue,
            focus: TaskFocus::Existing {
                task_ref: focus.task_id.clone(),
            },
            related_task_refs: related,
            required_context_refs: required.into_iter().collect(),
            unresolved_references: previous.decision.unresolved_references.clone(),
            new_objectives: Vec::new(),
            scope_classifications: Vec::new(),
            constraint_updates: Vec::new(),
            source_ranges,
        };
        candidates
            .validate_decision(&decision)
            .map_err(|_| invalid("source refresh dependencies are invalid"))?;
        Ok((event_id.clone(), decision))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn task_research_candidates(
        &self,
        project_id: &str,
        input: &EventId,
        source_hash: &str,
        compile: &TaskCompileCapture,
        events: &[crate::domain::events::EventEnvelope<serde_json::Value>],
        references: &crate::domain::research::ResearchReferenceCatalog,
        expansion: Option<&TaskSelectionDecision>,
        source_refresh: bool,
    ) -> Result<
        (
            Vec<crate::domain::task_context_research::ResearchTaskCandidate>,
            usize,
        ),
        SessionError,
    > {
        let Some(state) = compile.research_state(events, references)? else {
            return Ok((Vec::new(), 0));
        };
        self.task_research_candidates_from_state(
            project_id,
            input,
            source_hash,
            &state,
            references,
            expansion,
            source_refresh,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn task_research_candidates_from_state(
        &self,
        project_id: &str,
        input: &EventId,
        source_hash: &str,
        state: &crate::domain::research::ResearchState,
        references: &crate::domain::research::ResearchReferenceCatalog,
        expansion: Option<&TaskSelectionDecision>,
        source_refresh: bool,
    ) -> Result<
        (
            Vec<crate::domain::task_context_research::ResearchTaskCandidate>,
            usize,
        ),
        SessionError,
    > {
        let text = self
            .history_items_with_event_ids()
            .find_map(|(id, item)| {
                if id != input {
                    return None;
                }
                match item {
                    crate::domain::model::ContentItem::Text { text, .. } => Some(text.as_str()),
                    _ => None,
                }
            })
            .ok_or_else(|| invalid("selection input is absent or not text"))?;
        let input_text = excerpt(
            text,
            if expansion.is_some() {
                TASK_SELECTION_INPUT_BYTES * 2
            } else {
                TASK_SELECTION_INPUT_BYTES
            },
        );
        let expanded_query = expansion.map(|decision| decision.unresolved_references.join(" "));
        let query = expanded_query
            .as_deref()
            .filter(|query| !query.trim().is_empty())
            .or_else(|| (!input_text.trim().is_empty()).then_some(input_text))
            .unwrap_or(input.as_str());
        let refresh_hint = if source_refresh && expansion.is_none() {
            self.latest_task_context_selection()
                .filter(|(_, selection)| &selection.input_ref == input)
                .map(|(_, selection)| {
                    let mut hint = selection.decision.clone();
                    hint.required_context_refs = selection
                        .selected_research
                        .iter()
                        .filter_map(|reference| {
                            reference
                                .rsplit_once(':')
                                .map(|(identity, _)| format!("{identity}:{source_hash}"))
                        })
                        .collect();
                    hint
                })
        } else {
            None
        };
        crate::domain::task_context_research::candidates(
            state,
            references,
            source_hash,
            project_id,
            query,
            expansion.or(refresh_hint.as_ref()),
        )
        .map_err(|error| SessionError::TaskContext {
            message: format!("Research source navigation failed: {error}"),
        })
    }

    pub(crate) fn task_research_units(
        &self,
        source_hash: &str,
        compile: &TaskCompileCapture,
        events: &[crate::domain::events::EventEnvelope<serde_json::Value>],
        references: &crate::domain::research::ResearchReferenceCatalog,
        selected: &[String],
    ) -> Result<Vec<crate::domain::task_context_research::ResearchTaskUnit>, SessionError> {
        let Some(state) = compile.research_state(events, references)? else {
            if selected.is_empty() {
                return Ok(Vec::new());
            }
            return Err(invalid(
                "Research references require a Research-enabled definition",
            ));
        };
        crate::domain::task_context_research::resolve_selection(
            &state,
            references,
            source_hash,
            selected,
        )
        .map_err(|error| SessionError::TaskContext {
            message: format!("Research source dependency expansion failed: {error}"),
        })
    }

    fn live_process_references(
        &self,
        observations: &[crate::domain::tools::LiveProcessObservation],
    ) -> Result<Vec<LiveProcessReference>, SessionError> {
        use crate::domain::tools::{
            JobStatus, LiveProcessObservation, PtySessionStatus, ToolResultStatus,
        };
        let mut seen = std::collections::BTreeSet::new();
        let mut references = Vec::with_capacity(observations.len());
        for observation in observations {
            let (name, field, id) = match observation {
                LiveProcessObservation::Job {
                    job_id,
                    status: JobStatus::Running | JobStatus::Stopping,
                    ..
                } => ("start_job", "job_id", job_id.as_str()),
                LiveProcessObservation::Pty {
                    pty_session_id,
                    status: PtySessionStatus::Running | PtySessionStatus::Stopping,
                    ..
                } => ("start_pty", "pty_session_id", pty_session_id.as_str()),
                _ => return Err(invalid("live process observation has a terminal status")),
            };
            if !seen.insert((name, id)) {
                return Err(invalid("live process observation is duplicated"));
            }
            let (result_event_id, result) = self
                .history_items_with_event_ids()
                .find_map(|(event_id, item)| match item {
                    crate::domain::model::ContentItem::ToolResult { result }
                        if result.tool_name.as_str() == name
                            && result.status == ToolResultStatus::Succeeded
                            && result.output.get(field).and_then(serde_json::Value::as_str)
                                == Some(id) =>
                    {
                        Some((event_id, result))
                    }
                    _ => None,
                })
                .ok_or_else(|| invalid("live process has no durable start result"))?;
            let execution = crate::domain::tools::ExecutionId::for_model_call(&result.call_id);
            let receipt = self
                .execution_receipts
                .iter()
                .find(|record| {
                    record.receipt.execution_id == execution
                        && record.receipt.status == result.status
                })
                .ok_or_else(|| invalid("live process has no matching durable start receipt"))?;
            references.push(LiveProcessReference {
                observation: observation.clone(),
                result_event_id: result_event_id.clone(),
                receipt_event_id: receipt.event_id.clone(),
            });
        }
        Ok(references)
    }

    fn active_task_constraints(
        &self,
    ) -> Result<std::collections::BTreeMap<String, ConstraintRevision>, SessionError> {
        let mut active = std::collections::BTreeMap::new();
        let mut global_core = std::collections::BTreeSet::new();
        for (_, selection) in &self.task_context_selections {
            for revision in &selection.constraint_revisions {
                if let Some(previous) = &revision.supersedes
                    && active.remove(previous).is_none()
                    && !global_core.remove(previous)
                {
                    return Err(invalid("constraint supersedes an absent revision"));
                }
                if active
                    .insert(revision.reference.clone(), revision.clone())
                    .is_some()
                {
                    return Err(invalid("constraint revision identity is duplicated"));
                }
            }
            global_core.extend(
                selection
                    .core_scopes
                    .iter()
                    .filter(|scope| scope.scope == UnitScope::Global && !scope.suppressed)
                    .map(|scope| scope.reference.clone()),
            );
        }
        Ok(active)
    }

    pub(crate) fn constraint_contents(
        &self,
        selection: &TaskContextSelected,
    ) -> Result<Vec<(ConstraintRange, String)>, SessionError> {
        let mut contents = Vec::new();
        for range in &selection.selected_constraints {
            let source = self
                .history_items_with_event_ids()
                .find(|(id, _)| *id == &range.source_event_id)
                .map(|(_, item)| item)
                .ok_or_else(|| invalid("constraint source is absent"))?;
            let semantic = self.history_source_semantics(&range.source_event_id, source)?;
            if semantic.origin != crate::domain::context::ContextOrigin::OperatorInput
                || semantic.authority != crate::domain::context::AuthorityLevel::OperatorAssertion
            {
                return Err(invalid("constraint source lacks operator authority"));
            }
            if range.source_event_id == selection.input_ref {
                continue;
            }
            let mut covered = false;
            for excerpt in &selection.selected_excerpts {
                if excerpt.source_event_id != range.source_event_id
                    || excerpt.end_byte <= range.start_byte
                    || excerpt.start_byte >= range.end_byte
                {
                    continue;
                }
                if excerpt.start_byte <= range.start_byte && excerpt.end_byte >= range.end_byte {
                    covered = true;
                } else if selection.request_proof_version < 8 {
                    return Err(invalid(
                        "source excerpt partially overlaps a protected constraint",
                    ));
                }
            }
            if covered {
                continue;
            }
            let crate::domain::model::ContentItem::Text {
                role: crate::domain::model::MessageRole::User,
                text,
            } = source
            else {
                return Err(invalid("constraint source is not a user input"));
            };
            if range.start_byte >= range.end_byte
                || range.end_byte > text.len()
                || !text.is_char_boundary(range.start_byte)
                || !text.is_char_boundary(range.end_byte)
            {
                return Err(invalid("constraint source range is invalid"));
            }
            let content = &text[range.start_byte..range.end_byte];
            if crate::domain::canonical_json::canonical_blake3_hash(&content)
                .map_err(|_| invalid("constraint hash failed"))?
                != range.content_hash
            {
                return Err(invalid("constraint source hash mismatch"));
            }
            contents.push((range.clone(), content.to_owned()));
        }
        Ok(contents)
    }

    pub(crate) fn excerpt_contents(
        &self,
        selection: &TaskContextSelected,
    ) -> Result<Vec<SourceExcerpt>, SessionError> {
        let mut excerpts = Vec::with_capacity(selection.selected_excerpts.len());
        for range in &selection.selected_excerpts {
            let item = self
                .history_items_with_event_ids()
                .find(|(id, _)| *id == &range.source_event_id)
                .map(|(_, item)| item)
                .ok_or_else(|| invalid("excerpt source is absent"))?;
            let text = source_text(item)?;
            if range.start_byte >= range.end_byte
                || range.end_byte > text.len()
                || !text.is_char_boundary(range.start_byte)
                || !text.is_char_boundary(range.end_byte)
            {
                return Err(invalid("excerpt source range is invalid"));
            }
            let content = &text[range.start_byte..range.end_byte];
            if crate::domain::canonical_json::canonical_blake3_hash(&content)
                .map_err(|_| invalid("excerpt hash failed"))?
                != range.content_hash
            {
                return Err(invalid("excerpt source hash mismatch"));
            }
            let semantics = self.history_source_semantics(&range.source_event_id, item)?;
            let (role, receipt_event_id) = match item {
                crate::domain::model::ContentItem::Text { role, .. } => (Some(*role), None),
                crate::domain::model::ContentItem::ToolResult { result } => {
                    let execution =
                        crate::domain::tools::ExecutionId::for_model_call(&result.call_id);
                    let receipt = self
                        .execution_receipts
                        .iter()
                        .find(|record| record.receipt.execution_id == execution)
                        .map(|record| record.event_id.clone());
                    (None, receipt)
                }
                _ => {
                    return Err(invalid(
                        "this History item cannot be represented as a source excerpt",
                    ));
                }
            };
            let index = self
                .history_event_ids
                .iter()
                .position(|id| id == &range.source_event_id)
                .ok_or_else(|| invalid("excerpt Event time is absent"))?;
            excerpts.push(SourceExcerpt {
                range: range.clone(),
                text: content.to_owned(),
                semantics,
                role,
                receipt_event_id,
                vision_references: if selection.request_proof_version >= 6 {
                    self.history_vision_provenance(&range.source_event_id, item)?
                        .map(|value| value.references().clone())
                } else {
                    None
                },
                observed_at: self.history_event_times[index].as_str().to_owned(),
            });
        }
        Ok(excerpts)
    }

    pub(crate) fn task_selection_sample_count(&self, input: &EventId) -> usize {
        self.task_selection_samples
            .iter()
            .filter(|(_, sample)| &sample.input_ref == input)
            .count()
    }

    pub(crate) fn latest_task_selection_sample(
        &self,
        input: &EventId,
    ) -> Option<&TaskSelectionSample> {
        self.task_selection_samples
            .iter()
            .rev()
            .find(|(_, sample)| &sample.input_ref == input)
            .map(|(_, sample)| sample)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn task_selection_candidates_with_capture(
        &self,
        project_id: &str,
        input_ref: &EventId,
        common: &super::CompactionContextCapture,
        research_candidates: &[crate::domain::task_context_research::ResearchTaskCandidate],
        research_omitted: usize,
        memory: &[TaskMemoryDocument],
        expansion: Option<&TaskSelectionDecision>,
        source_refresh: bool,
        candidate_policy_version: u16,
        read: &NavigationReader<'_>,
    ) -> Result<SelectionCandidates, SessionError> {
        self.task_selection_candidates_with_index(
            project_id,
            input_ref,
            common,
            research_candidates,
            research_omitted,
            memory,
            expansion,
            source_refresh,
            candidate_policy_version,
            read,
            None,
            None,
        )
    }

    fn append_core_candidates(
        &self,
        candidates: &mut SelectionCandidates,
        common: &super::CompactionContextCapture,
        expansion: Option<&TaskSelectionDecision>,
    ) -> Result<(), SessionError> {
        for fragment in common.fragments()? {
            if fragment.source() != crate::domain::context::ContextFragmentSource::ProjectMemoryCore
            {
                continue;
            }
            let crate::domain::context::ContextFragmentContent::ContentItems { items } =
                fragment.content()
            else {
                return Err(invalid("Core Memory capture is not content"));
            };
            let [crate::domain::model::ContentItem::Text { text, .. }] = items.as_slice() else {
                return Err(invalid("Core Memory capture is not a single text item"));
            };
            if fragment.id().as_str().ends_with(":empty") {
                continue;
            }
            let mut starts = vec![0];
            let mut offset = 0;
            let mut fence: Option<(char, usize)> = None;
            for line in text.split_inclusive('\n') {
                let trimmed = line.trim_start();
                let marker = trimmed
                    .chars()
                    .next()
                    .filter(|character| matches!(character, '`' | '~'));
                if let Some(marker) = marker {
                    let count = trimmed
                        .chars()
                        .take_while(|character| *character == marker)
                        .count();
                    if count >= 3 {
                        match fence {
                            None => fence = Some((marker, count)),
                            Some((open, length)) if open == marker && count >= length => {
                                fence = None
                            }
                            _ => {}
                        }
                    }
                }
                if fence.is_none() && offset != 0 && trimmed.starts_with('#') {
                    let hashes = trimmed
                        .chars()
                        .take_while(|character| *character == '#')
                        .count();
                    if hashes <= 6 && trimmed.as_bytes().get(hashes) == Some(&b' ') {
                        starts.push(offset);
                    }
                }
                offset += line.len();
            }
            starts.push(text.len());
            let sections = starts
                .windows(2)
                .filter(|pair| pair[0] != pair[1])
                .map(|pair| {
                    let (start, end) = (pair[0], pair[1]);
                    let hash =
                        crate::domain::canonical_json::canonical_blake3_hash(&&text[start..end])
                            .map_err(|_| invalid("Core Memory section hash failed"))?;
                    Ok((start, end, hash))
                })
                .collect::<Result<Vec<_>, SessionError>>()?;
            let previous_scopes = self
                .task_context_selections
                .last()
                .map(|(_, selection)| selection.core_scopes.as_slice())
                .unwrap_or_default();
            for (start, end, hash) in &sections {
                let (start, end) = (*start, *end);
                let part = &text[start..end];
                let reference = format!("core:{}:{start}:{end}:{hash}", fragment.id().as_str());
                let exact = previous_scopes
                    .iter()
                    .find(|scope| scope.reference == reference);
                let previous = exact.or_else(|| {
                    unique_prior_core_scope(
                        previous_scopes,
                        &reference,
                        sections
                            .iter()
                            .filter(|(_, _, current)| current == hash)
                            .count(),
                    )
                });
                let inspected = excerpt(part, if expansion.is_some() { 4096 } else { 1024 });
                let range = CoreRange {
                    fragment_id: fragment.id().as_str().to_owned(),
                    start_byte: start,
                    end_byte: end,
                    content_hash: hash.clone(),
                    inspected_bytes: inspected.len(),
                };
                if previous.is_some_and(|scope| scope.suppressed) {
                    candidates.suppressed_core.push(range);
                    continue;
                }
                let previous = previous.map(|scope| scope.scope.clone());
                if inspected.len() != part.len() {
                    candidates.gaps.push(format!(
                        "Core section {reference} has an uninspected suffix"
                    ));
                }
                let task_ref = match &previous {
                    Some(UnitScope::Task { task_ref }) => Some(task_ref.clone()),
                    _ => None,
                };
                candidates.context.push(ContextCandidate {
                    reference,
                    task_ref,
                    excerpt: inspected.to_owned(),
                    source_event_ref: None,
                    truncated: inspected.len() != part.len(),
                    core_range: Some(range),
                    constraint_range: None,
                    research_ref: None,
                    memory_ref: None,
                    summary_ref: None,
                    source_origin: Some(fragment.provenance().origin()),
                    source_authority: Some(fragment.provenance().authority_level()),
                    known_scope: previous,
                });
            }
        }
        Ok(())
    }

    /// Replay recomputes Core suppression from the frozen source and the
    /// preceding Session snapshot rather than trusting captured candidates.
    pub(crate) fn validate_task_core_candidates(
        &self,
        candidates: &SelectionCandidates,
        common: &super::CompactionContextCapture,
        expansion: Option<&TaskSelectionDecision>,
    ) -> Result<(), SessionError> {
        let mut expected = candidates.clone();
        expected.context.clear();
        expected.gaps.clear();
        expected.suppressed_core.clear();
        self.append_core_candidates(&mut expected, common, expansion)?;
        if expected.suppressed_core != candidates.suppressed_core
            || !candidates
                .context
                .iter()
                .filter(|candidate| candidate.core_range.is_some())
                .eq(expected.context.iter())
        {
            return Err(invalid(
                "captured Core candidates differ from their frozen source",
            ));
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn task_selection_candidates_with_index(
        &self,
        project_id: &str,
        input_ref: &EventId,
        common: &super::CompactionContextCapture,
        research_candidates: &[crate::domain::task_context_research::ResearchTaskCandidate],
        research_omitted: usize,
        memory: &[TaskMemoryDocument],
        expansion: Option<&TaskSelectionDecision>,
        source_refresh: bool,
        candidate_policy_version: u16,
        read: &NavigationReader<'_>,
        postings: Option<&PostingReader<'_>>,
        metadata: Option<&dyn TaskNavigationAccess>,
    ) -> Result<SelectionCandidates, SessionError> {
        if let Some(expansion) = expansion
            && self
                .latest_task_selection_sample(input_ref)
                .is_none_or(|sample| &sample.decision != expansion)
        {
            return Err(invalid(
                "candidate expansion does not match the prior accepted selection sample",
            ));
        }
        let mut candidates = if source_refresh {
            self.task_selection_candidates_inner(
                project_id,
                input_ref,
                expansion,
                true,
                candidate_policy_version,
                read,
                postings,
                metadata,
            )?
        } else if postings.is_none() && candidate_policy_version < 4 {
            self.task_selection_candidates(project_id, input_ref, expansion, read)?
        } else {
            self.task_selection_candidates_inner(
                project_id,
                input_ref,
                expansion,
                false,
                candidate_policy_version,
                read,
                postings,
                metadata,
            )?
        };
        for constraint in self.active_task_constraints()?.into_values() {
            let item = self
                .history_items_with_event_ids()
                .find(|(id, _)| *id == &constraint.range.source_event_id)
                .map(|(_, item)| item)
                .ok_or_else(|| invalid("constraint source is absent"))?;
            let semantic =
                self.history_source_semantics(&constraint.range.source_event_id, item)?;
            if semantic.origin != crate::domain::context::ContextOrigin::OperatorInput
                || semantic.authority != crate::domain::context::AuthorityLevel::OperatorAssertion
            {
                return Err(invalid("constraint source lacks operator authority"));
            }
            let crate::domain::model::ContentItem::Text {
                role: crate::domain::model::MessageRole::User,
                text,
            } = item
            else {
                return Err(invalid("constraint source is not an original user input"));
            };
            let range = &constraint.range;
            if range.start_byte >= range.end_byte
                || range.end_byte > text.len()
                || !text.is_char_boundary(range.start_byte)
                || !text.is_char_boundary(range.end_byte)
            {
                return Err(invalid("constraint source range is invalid"));
            }
            let content = &text[range.start_byte..range.end_byte];
            if crate::domain::canonical_json::canonical_blake3_hash(&content)
                .map_err(|_| invalid("constraint hash failed"))?
                != range.content_hash
            {
                return Err(invalid("constraint source content changed"));
            }
            candidates.context.push(ContextCandidate {
                reference: constraint.reference,
                task_ref: match &constraint.scope {
                    UnitScope::Task { task_ref } => Some(task_ref.clone()),
                    _ => None,
                },
                excerpt: content.to_owned(),
                source_event_ref: Some(range.source_event_id.to_string()),
                truncated: false,
                core_range: None,
                constraint_range: Some(range.clone()),
                research_ref: None,
                memory_ref: None,
                summary_ref: None,
                source_origin: Some(semantic.origin),
                source_authority: Some(semantic.authority),
                known_scope: Some(constraint.scope),
            });
        }
        self.append_core_candidates(&mut candidates, common, expansion)?;
        if research_omitted != 0 {
            candidates.gaps.push(format!(
                "{research_omitted} Research candidates require expansion"
            ));
        }
        for candidate in research_candidates {
            candidates.context.push(ContextCandidate {
                reference: candidate.reference.clone(),
                task_ref: None,
                excerpt: candidate.excerpt.clone(),
                source_event_ref: None,
                truncated: candidate.truncated,
                core_range: None,
                constraint_range: None,
                research_ref: Some(candidate.reference.clone()),
                memory_ref: None,
                summary_ref: None,
                source_origin: Some(crate::domain::context::ContextOrigin::ResearchProjection),
                source_authority: Some(crate::domain::context::AuthorityLevel::DerivedNonEvidence),
                known_scope: None,
            });
        }
        if let Some(summary) = self
            .latest_compaction()
            .map(|compaction| compaction.summary())
            && let Some(task_id) = summary.selected_task_id()
        {
            let text = serde_json::to_string(&serde_json::json!({"task_scope":summary.task_scope(), "summary":summary.model_projection()?}))
                .map_err(|_| invalid("task Summary could not be encoded"))?;
            let reference = format!("summary:{}", summary.fingerprint());
            let inspected = excerpt(&text, if expansion.is_some() { 4096 } else { 1024 });
            candidates.context.push(ContextCandidate {
                reference: reference.clone(),
                task_ref: Some(task_id.to_owned()),
                excerpt: inspected.to_owned(),
                source_event_ref: None,
                truncated: inspected.len() != text.len(),
                core_range: None,
                constraint_range: None,
                research_ref: None,
                memory_ref: None,
                summary_ref: Some(reference),
                source_origin: Some(crate::domain::context::ContextOrigin::SessionProjection),
                source_authority: Some(crate::domain::context::AuthorityLevel::DerivedNonEvidence),
                known_scope: None,
            });
        }
        let expected_scope = self.agent_definition_binding().and_then(|binding| {
            match binding.definition().context().memory() {
                crate::domain::agent_definition::AgentMemoryScope::None => None,
                crate::domain::agent_definition::AgentMemoryScope::Project => {
                    Some(crate::domain::memory::MemoryScope::Project)
                }
                crate::domain::agent_definition::AgentMemoryScope::Agent => {
                    Some(crate::domain::memory::MemoryScope::Agent {
                        agent_id: binding.definition().id().clone(),
                    })
                }
            }
        });
        let mut paths = std::collections::BTreeSet::new();
        if memory.len() > 64 {
            return Err(invalid("captured Memory document limit exceeded"));
        }
        if memory.len() == 64 {
            candidates.gaps.push(
                "Memory candidate limit reached; additional documents may be omitted".to_owned(),
            );
        }
        for document in memory {
            if Some(&document.scope) != expected_scope.as_ref()
                || document.path.as_str() == "core.md"
                || !paths.insert(&document.path)
                || document.content.len() > 64 * 1024
                || document.revision
                    != blake3::hash(document.content.as_bytes())
                        .to_hex()
                        .to_string()
            {
                return Err(invalid(
                    "captured Memory scope, path or revision is invalid",
                ));
            }
            let reference = format!("memory:{}:{}", document.path, document.revision);
            let inspected = excerpt(
                &document.content,
                if expansion.is_some() { 4096 } else { 1024 },
            );
            candidates.context.push(ContextCandidate {
                reference: reference.clone(),
                task_ref: None,
                excerpt: inspected.to_owned(),
                source_event_ref: None,
                truncated: inspected.len() != document.content.len(),
                core_range: None,
                constraint_range: None,
                research_ref: None,
                memory_ref: Some(reference),
                summary_ref: None,
                source_origin: Some(crate::domain::context::ContextOrigin::ProjectMemory),
                source_authority: Some(crate::domain::context::AuthorityLevel::DerivedNonEvidence),
                known_scope: None,
            });
        }
        Ok(candidates)
    }

    pub(crate) fn task_progress_source(
        &self,
        task_id: &str,
        read: &NavigationReader<'_>,
    ) -> Result<Option<(EventId, String)>, SessionError> {
        const MAX_OBSERVATIONS: usize = 32;
        const MAX_EXCERPT_BYTES: usize = 512;
        let Some((event_id, selection, revision)) = self
            .task_context_selections
            .iter()
            .rev()
            .find_map(|(event_id, selection)| {
                selection
                    .task_revisions
                    .iter()
                    .find(|revision| revision.task_id == task_id)
                    .map(|revision| (event_id, selection, revision))
            })
        else {
            return Ok(None);
        };
        let mut anchors = std::collections::BTreeMap::new();
        let mut history_cursor = 0;
        for (_, candidate) in &self.task_context_selections {
            let index = self
                .task_history_position(&candidate.input_ref, &mut history_cursor)
                .ok_or_else(|| invalid("task progress input is absent from History"))?;
            anchors.insert(index, candidate);
        }
        let mut active = None;
        let mut recent = std::collections::VecDeque::new();
        let mut omitted = 0usize;
        let mut unresolved = std::collections::VecDeque::new();
        let mut omitted_unresolved = 0usize;
        let cancelled = self.cancelled_initial_inputs();
        for (index, (id, item)) in self.history_items_with_event_ids().enumerate() {
            if let Some(candidate) = anchors.get(&index) {
                active = Some(*candidate);
            }
            if cancelled.contains(id) {
                continue;
            }
            if active.is_none_or(|candidate| {
                !candidate
                    .task_revisions
                    .iter()
                    .any(|value| value.task_id == task_id)
            }) {
                continue;
            }
            if matches!(item, crate::domain::model::ContentItem::ToolCall { call } if call.name.as_str() == "context_read")
                || matches!(item, crate::domain::model::ContentItem::ToolResult { result } if result.tool_name.as_str() == "context_read")
            {
                continue;
            }
            if active.is_some_and(TaskContextSelected::has_multiple_objectives) {
                if matches!(item, crate::domain::model::ContentItem::ToolResult { .. }) {
                    if unresolved.len() == MAX_OBSERVATIONS {
                        unresolved.pop_front();
                        omitted_unresolved += 1;
                    }
                    unresolved.push_back(id.clone());
                }
                continue;
            }
            if recent.len() == MAX_OBSERVATIONS {
                recent.pop_front();
                omitted += 1;
            }
            recent.push_back((index, id, item));
        }
        let recent = recent.into_iter().collect::<Vec<_>>();
        let sources = recent
            .iter()
            .map(|(_, id, item)| (*id, *item))
            .collect::<Vec<_>>();
        let semantics = self.history_source_semantics_many(&sources)?;
        let observations = recent
            .into_iter()
            .zip(semantics)
            .map(|((index, id, item), semantics)| {
                let kind = match item {
                    crate::domain::model::ContentItem::Text {
                        role: crate::domain::model::MessageRole::User,
                        ..
                    } => "user_role_text",
                    crate::domain::model::ContentItem::Text {
                        role: crate::domain::model::MessageRole::Assistant,
                        ..
                    } => "assistant_observation",
                    crate::domain::model::ContentItem::ToolCall { .. } => "tool_call",
                    crate::domain::model::ContentItem::ToolResult { .. } => "tool_result",
                    crate::domain::model::ContentItem::ImageArtifact { .. } => "image_artifact",
                    _ => "history_item",
                };
                let (rendered, source_truncated) = match item {
                    crate::domain::model::ContentItem::Text { text, .. } => {
                        let prefix = excerpt(text, MAX_EXCERPT_BYTES);
                        (std::borrow::Cow::Borrowed(prefix), prefix.len() < text.len())
                    }
                    _ => self.task_navigation_excerpt(index, item, read)?,
                };
                let receipt = match item {
                    crate::domain::model::ContentItem::ToolResult { result } => {
                        let execution =
                            crate::domain::tools::ExecutionId::for_model_call(&result.call_id);
                        self.execution_receipts
                            .iter()
                            .find(|record| record.receipt.execution_id == execution)
                    }
                    _ => None,
                };
                Ok(serde_json::json!({
                    "source_event_ref": id, "kind": kind, "source_origin": semantics.origin,
                    "semantic_class": semantics.class, "source_authority": semantics.authority,
                    "excerpt": excerpt(&rendered, MAX_EXCERPT_BYTES),
                    "truncated": source_truncated || rendered.len() > excerpt(&rendered, MAX_EXCERPT_BYTES).len(),
                    "receipt_event_ref": receipt.map(|record| &record.event_id),
                    "receipt_status": receipt.map(|record| record.receipt.status),
                    "artifact_refs": receipt.map(|record| &record.receipt.output_artifact_ids),
                }))
            })
            .collect::<Result<Vec<_>, SessionError>>()?;
        let summary_ref = self
            .latest_compaction
            .as_ref()
            .map(|compaction| compaction.summary())
            .filter(|summary| summary.selected_task_id() == Some(task_id))
            .map(|summary| format!("summary:{}", summary.fingerprint()));
        let mut projection = serde_json::json!({
            "task": revision, "selection_event_ref": event_id, "input_ref": selection.input_ref,
            "observations": observations, "omitted_observations": omitted,
            "unresolved_references": selection.decision.unresolved_references,
            "gaps": selection.unresolved_gaps, "summary_ref": summary_ref,
        });
        if !unresolved.is_empty() {
            projection["unresolved_attribution"] = serde_json::json!({
                "gap": MULTI_OBJECTIVE_PROGRESS_GAP,
                "source_event_refs": unresolved,
                "omitted_sources": omitted_unresolved,
            });
        }
        let text = serde_json::to_string(&projection)
            .map_err(|_| invalid("task progress could not be encoded"))?;
        Ok(Some((event_id.clone(), text)))
    }

    /// A bounded progress projection derived from facts available at the
    /// selected source watermark. It is navigation, never a completion fact.
    pub(crate) fn selected_task_progress(
        &self,
        selection: &TaskContextSelected,
        read: &NavigationReader<'_>,
    ) -> Result<Vec<TaskProgressSource>, SessionError> {
        const MAX_OBSERVATIONS: usize = 8;
        const MAX_EXCERPT_BYTES: usize = 256;
        let mut tasks = selection
            .decision
            .related_task_refs
            .iter()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        if let crate::domain::task_context::TaskFocus::Existing { task_ref } =
            &selection.decision.focus
        {
            tasks.insert(task_ref.clone());
        }
        if tasks.is_empty() {
            return Ok(Vec::new());
        }
        let previous = self
            .task_context_selections
            .iter()
            .filter(|(_, candidate)| candidate.source.watermark() < selection.source.watermark())
            .collect::<Vec<_>>();
        let revisions = previous
            .iter()
            .flat_map(|(_, candidate)| &candidate.constraint_revisions)
            .chain(&selection.constraint_revisions)
            .collect::<Vec<_>>();
        let superseded = revisions
            .iter()
            .filter_map(|revision| revision.supersedes.as_deref())
            .collect::<std::collections::BTreeSet<_>>();
        let mut superseded_text = Vec::new();
        for revision in revisions
            .iter()
            .filter(|revision| superseded.contains(revision.reference.as_str()))
        {
            let item = self
                .history_items_with_event_ids()
                .find_map(|(id, item)| (id == &revision.range.source_event_id).then_some(item))
                .ok_or_else(|| invalid("superseded constraint source is absent"))?;
            let crate::domain::model::ContentItem::Text { text, .. } = item else {
                return Err(invalid("superseded constraint source is not text"));
            };
            let value = text
                .get(revision.range.start_byte..revision.range.end_byte)
                .ok_or_else(|| invalid("superseded constraint range is invalid"))?;
            superseded_text.push((*revision, value));
        }
        let mut anchors = std::collections::BTreeMap::new();
        let mut history_cursor = 0;
        for (_, candidate) in &previous {
            let index = self
                .task_history_position(&candidate.input_ref, &mut history_cursor)
                .ok_or_else(|| invalid("task progress input is absent from History"))?;
            anchors.insert(index, candidate);
        }
        // A narrowed source excludes automatic expansion of its owning task,
        // but does not exclude progress for a different focal task.
        for range in &selection.selected_excerpts {
            let index = self
                .history_event_ids
                .iter()
                .position(|id| id == &range.source_event_id)
                .ok_or_else(|| invalid("task progress excerpt source is absent"))?;
            if let Some((_, owner)) = anchors.range(..=index).next_back() {
                if selection.request_proof_version < 3 || !owner.has_multiple_objectives() {
                    for revision in &owner.task_revisions {
                        tasks.remove(&revision.task_id);
                    }
                } else if owner.input_ref == range.source_event_id {
                    let owners = owner
                        .task_revisions
                        .iter()
                        .filter(|revision| {
                            revision.input_range.is_some_and(|(start, end)| {
                                start <= range.start_byte && range.end_byte <= end
                            })
                        })
                        .collect::<Vec<_>>();
                    if let [revision] = owners.as_slice() {
                        tasks.remove(&revision.task_id);
                    }
                }
            }
        }
        let current_index = self
            .history_event_ids
            .iter()
            .position(|id| id == &selection.input_ref)
            .ok_or_else(|| invalid("task progress current input is absent"))?;
        let mut prior_by_task = std::collections::BTreeMap::new();
        for pair in &previous {
            let (id, candidate) = *pair;
            for revision in &candidate.task_revisions {
                if tasks.contains(&revision.task_id) {
                    prior_by_task.insert(revision.task_id.clone(), (id, candidate, revision));
                }
            }
        }
        let mut observations_by_task = tasks
            .iter()
            .map(|id| (id.clone(), (std::collections::VecDeque::new(), 0usize)))
            .collect::<std::collections::BTreeMap<_, _>>();
        let mut unresolved_by_task = tasks
            .iter()
            .map(|id| (id.clone(), (std::collections::VecDeque::new(), 0usize)))
            .collect::<std::collections::BTreeMap<_, _>>();
        let summarized = if selection.request_proof_version >= 3 {
            selection
                .selected_summary
                .as_ref()
                .map(|reference| {
                    let compaction = self
                        .latest_compaction()
                        .ok_or_else(|| invalid("selected Summary is absent"))?;
                    if reference != &format!("summary:{}", compaction.summary().fingerprint()) {
                        return Err(invalid(
                            "selected Summary does not match the current task source",
                        ));
                    }
                    Ok(compaction
                        .summary()
                        .summarized_history()
                        .iter()
                        .collect::<std::collections::BTreeSet<_>>())
                })
                .transpose()?
        } else {
            None
        };
        let mut active = None;
        for (index, (id, item)) in self.history_items_with_event_ids().enumerate() {
            if self.history_event_sequences[index] > selection.source.watermark()
                || index >= current_index
            {
                break;
            }
            if selection.request_proof_version < 3 && selection.selected_summary.is_some() {
                continue;
            }
            if let Some(candidate) = anchors.get(&index) {
                active = Some(*candidate);
            }
            if summarized
                .as_ref()
                .is_some_and(|covered| covered.contains(id))
            {
                continue;
            }
            if !matches!(item, crate::domain::model::ContentItem::ToolResult { result } if result.tool_name.as_str() != "context_read")
            {
                continue;
            }
            if let Some(candidate) = active {
                for revision in &candidate.task_revisions {
                    if selection.request_proof_version >= 2 && candidate.has_multiple_objectives() {
                        if let Some((recent, omitted)) =
                            unresolved_by_task.get_mut(&revision.task_id)
                        {
                            if recent.len() == MAX_OBSERVATIONS {
                                recent.pop_front();
                                *omitted += 1;
                            }
                            recent.push_back(id.clone());
                        }
                        continue;
                    }
                    if let Some((recent, omitted)) = observations_by_task.get_mut(&revision.task_id)
                    {
                        if recent.len() == MAX_OBSERVATIONS {
                            recent.pop_front();
                            *omitted += 1;
                        }
                        recent.push_back((index, id, item));
                    }
                }
            }
        }
        let mut result = Vec::new();
        for task_id in tasks {
            let prior = prior_by_task.get(&task_id).copied();
            let Some((selection_id, _prior, revision)) = prior else {
                let legacy = task_id.strip_prefix("legacy:").map(EventId::new)
                    .filter(|id| self.operator_boundaries.iter().any(|boundary| matches!(boundary, OperatorTurnBoundary::Accepted { anchor, .. } if anchor == id)))
                    .ok_or_else(|| invalid("selected task progress source is absent"))?;
                let (goal, goal_source_range, goal_inspected_range, goal_truncated) =
                    self.task_progress_goal(&task_id, &legacy, None, &superseded_text, read)?;
                let mut projection = serde_json::json!({"task_id":task_id, "source_event_ref":legacy, "goal":goal,
                    "goal_in_selected_history":false,
                    "goal_source_range":goal_source_range, "goal_inspected_range":goal_inspected_range,
                    "goal_truncated":goal_truncated,
                    "gaps":["Legacy task has no recorded task-scoped progress projection; inspect its original source."]});
                let text = projection.to_string();
                projection["goal"] = serde_json::Value::Null;
                projection["goal_in_selected_history"] = serde_json::Value::Bool(true);
                let text_when_goal_in_history = projection.to_string();
                projection["goal_in_selected_history"] = serde_json::Value::Bool(false);
                projection["goal_in_operator_input"] = serde_json::Value::Bool(true);
                result.push(TaskProgressSource {
                    task_id,
                    text,
                    text_when_goal_in_history,
                    text_when_goal_in_operator_input: projection.to_string(),
                    goal_source_event_id: legacy.clone(),
                    event_refs: vec![legacy],
                    receipt_refs: Vec::new(),
                    artifact_refs: Vec::new(),
                });
                continue;
            };
            let (recent, mut omitted) = observations_by_task
                .remove(&task_id)
                .ok_or_else(|| invalid("selected task progress projection is absent"))?;
            let items = recent
                .iter()
                .map(|(_, id, item)| (*id, *item))
                .collect::<Vec<_>>();
            let semantics = self.history_source_semantics_many(&items)?;
            let mut event_refs = std::collections::BTreeSet::from([selection_id.clone()]);
            let (goal_selection_id, goal_selection, goal_revision) = previous
                .iter()
                .find_map(|(id, candidate)| {
                    candidate
                        .task_revisions
                        .iter()
                        .find(|item| item.task_id == task_id)
                        .map(|item| (id, candidate, item))
                })
                .ok_or_else(|| invalid("task objective source is absent"))?;
            event_refs.insert(goal_selection_id.clone());
            let goal_source = task_id
                .strip_prefix("legacy:")
                .map(EventId::new)
                .unwrap_or_else(|| goal_selection.input_ref.clone());
            event_refs.insert(goal_source.clone());
            let (goal, goal_source_range, goal_inspected_range, goal_truncated) = self
                .task_progress_goal(
                    &task_id,
                    &goal_source,
                    goal_revision.input_range,
                    &superseded_text,
                    read,
                )?;
            let applies = |revision: &ConstraintRevision| {
                matches!(revision.scope, UnitScope::Global | UnitScope::Unknown)
                    || matches!(&revision.scope, UnitScope::Task { task_ref } if task_ref == &task_id)
            };
            let mut unresolved = std::collections::VecDeque::<(EventId, &str)>::new();
            let mut omitted_gaps = 0usize;
            for (id, candidate) in &previous {
                if candidate
                    .task_revisions
                    .iter()
                    .any(|item| item.task_id == task_id)
                {
                    for gap in &candidate.unresolved_gaps {
                        if selection.request_proof_version < 4
                            && superseded_text
                                .iter()
                                .any(|(revision, value)| applies(revision) && gap.contains(*value))
                        {
                            continue;
                        }
                        if selection.request_proof_version < 5
                            && let Some(index) = unresolved
                                .iter()
                                .position(|(_, value)| *value == gap.as_str())
                        {
                            unresolved.remove(index);
                        }
                        if unresolved.len() == MAX_OBSERVATIONS {
                            unresolved.pop_front();
                            omitted_gaps += 1;
                        }
                        unresolved.push_back(((*id).clone(), gap));
                    }
                }
            }
            let unresolved = unresolved.into_iter().map(|(id, gap)| {
                event_refs.insert(id.clone());
                serde_json::json!({"selection_event_ref":id, "gap":excerpt(gap, MAX_EXCERPT_BYTES),
                    "truncated":gap.len() > MAX_EXCERPT_BYTES})
            }).collect::<Vec<_>>();
            let mut receipt_refs = std::collections::BTreeSet::new();
            let mut artifact_refs = std::collections::BTreeSet::new();
            let mut observations = Vec::with_capacity(recent.len());
            for ((index, id, item), semantics) in recent.into_iter().zip(semantics) {
                let (text, truncated) = match item {
                    crate::domain::model::ContentItem::ToolResult { result }
                        if result
                            .output
                            .get("stdout")
                            .and_then(serde_json::Value::as_str)
                            .is_some_and(|text| !text.is_empty()) =>
                    {
                        let stdout = result.output["stdout"]
                            .as_str()
                            .expect("stdout was checked");
                        let preview = excerpt(stdout, MAX_EXCERPT_BYTES);
                        (preview.to_owned(), preview.len() < stdout.len())
                    }
                    _ => {
                        let (text, truncated) = self.task_navigation_excerpt(index, item, read)?;
                        (text.into_owned(), truncated)
                    }
                };
                if selection.request_proof_version < 4
                    && superseded_text
                        .iter()
                        .any(|(revision, value)| applies(revision) && text.contains(*value))
                {
                    omitted += 1;
                    continue;
                }
                let receipt = match item {
                    crate::domain::model::ContentItem::ToolResult { result } => {
                        let execution =
                            crate::domain::tools::ExecutionId::for_model_call(&result.call_id);
                        self.execution_receipts
                            .iter()
                            .find(|record| record.receipt.execution_id == execution)
                    }
                    _ => None,
                };
                event_refs.insert(id.clone());
                if let Some(record) = receipt {
                    receipt_refs.insert(crate::domain::research::ExecutionReceiptId::new(
                        record.event_id.as_str(),
                    ));
                    artifact_refs.extend(record.receipt.output_artifact_ids.iter().cloned());
                }
                observations.push(serde_json::json!({ "source_event_ref":id, "source_origin":semantics.origin,
                    "source_authority":semantics.authority, "excerpt":excerpt(&text, MAX_EXCERPT_BYTES),
                    "truncated":truncated || text.len() > MAX_EXCERPT_BYTES,
                    "receipt_event_ref":receipt.map(|record| &record.event_id),
                    "receipt_status":receipt.map(|record| record.receipt.status) }));
            }
            let mut projection = serde_json::json!({ "task_id":revision.task_id, "task_revision":revision.revision, "selection_event_ref":selection_id,
                "goal":goal, "goal_source_event_ref":goal_source,
                "goal_in_selected_history":false,
                "goal_source_range":goal_source_range, "goal_inspected_range":goal_inspected_range,
                "goal_truncated":goal_truncated,
                "unresolved_at_prior_selections":unresolved,
                "omitted_unresolved_gaps":omitted_gaps,
                "observations":observations, "omitted_observations":omitted,
                "summary_ref":selection.selected_summary });
            if let Some((sources, omitted_sources)) = unresolved_by_task.remove(&task_id)
                && !sources.is_empty()
            {
                event_refs.extend(sources.iter().cloned());
                projection["unresolved_attribution"] = serde_json::json!({
                    "gap": MULTI_OBJECTIVE_PROGRESS_GAP,
                    "source_event_refs": sources,
                    "omitted_sources": omitted_sources,
                });
            }
            let text = serde_json::to_string(&projection)
                .map_err(|_| invalid("task progress could not be encoded"))?;
            projection["goal"] = serde_json::Value::Null;
            projection["goal_in_selected_history"] = serde_json::Value::Bool(true);
            let text_when_goal_in_history = serde_json::to_string(&projection)
                .map_err(|_| invalid("task progress could not be encoded"))?;
            projection["goal_in_selected_history"] = serde_json::Value::Bool(false);
            projection["goal_in_operator_input"] = serde_json::Value::Bool(true);
            result.push(TaskProgressSource {
                task_id,
                text,
                text_when_goal_in_history,
                text_when_goal_in_operator_input: serde_json::to_string(&projection)
                    .map_err(|_| invalid("task progress could not be encoded"))?,
                goal_source_event_id: goal_source,
                event_refs: event_refs.into_iter().collect(),
                receipt_refs: receipt_refs.into_iter().collect(),
                artifact_refs: artifact_refs.into_iter().collect(),
            });
        }
        Ok(result)
    }

    pub(crate) fn task_selection_sample(&self, event_id: &EventId) -> Option<&TaskSelectionSample> {
        self.task_selection_samples
            .iter()
            .find(|(id, _)| id == event_id)
            .map(|(_, sample)| sample)
    }

    /// Rebuilds navigation candidates from the original Session facts and
    /// adopted task revisions. Candidate excerpts are explicitly bounded;
    /// selecting a reference still reads the original History item.
    pub(crate) fn task_selection_candidates(
        &self,
        project_id: &str,
        input_ref: &EventId,
        expansion: Option<&TaskSelectionDecision>,
        read: &NavigationReader<'_>,
    ) -> Result<SelectionCandidates, SessionError> {
        self.task_selection_candidates_inner(
            project_id, input_ref, expansion, false, 1, read, None, None,
        )
    }

    // The caller supplies the exact task source, expansion and optional index
    // inputs separately so indexed and source-only reconstruction share one owner.
    #[allow(clippy::too_many_arguments)]
    fn task_selection_candidates_inner(
        &self,
        project_id: &str,
        input_ref: &EventId,
        expansion: Option<&TaskSelectionDecision>,
        source_refresh: bool,
        candidate_policy_version: u16,
        read: &NavigationReader<'_>,
        postings: Option<&PostingReader<'_>>,
        metadata: Option<&dyn TaskNavigationAccess>,
    ) -> Result<SelectionCandidates, SessionError> {
        const MAX_CONTEXT_UNITS: usize = 512;
        let history = self.history.items();
        let history_ids = &self.history_event_ids;
        let input_index = history_ids
            .iter()
            .rposition(|id| id == input_ref)
            .ok_or_else(|| invalid("selection input is absent from History"))?;
        let crate::domain::model::ContentItem::Text { text, .. } = &history[input_index] else {
            return Err(invalid("selection input is not text"));
        };
        let input_text = excerpt(
            text,
            if expansion.is_some() {
                TASK_SELECTION_INPUT_BYTES * 2
            } else {
                TASK_SELECTION_INPUT_BYTES
            },
        );
        let received_at = self
            .operator_boundaries
            .iter()
            .rev()
            .find_map(|boundary| match boundary {
                OperatorTurnBoundary::Accepted {
                    anchor,
                    received_at,
                    ..
                } if anchor == input_ref => received_at.clone(),
                _ => None,
            })
            .ok_or_else(|| invalid("new task input has no recorded receive time"))?;
        let mut gaps = Vec::new();
        if input_text.len() != text.len() {
            gaps.push(format!(
                "input bytes {}..{} were not inspected",
                input_text.len(),
                text.len()
            ));
        }
        let mut reconstructed = TaskNavigationMetadata::default();
        if metadata.is_none() {
            self.update_task_navigation_metadata(
                &mut reconstructed,
                input_ref,
                candidate_policy_version,
            )?;
        }
        let replayed = ReplayedTaskNavigation {
            metadata: &reconstructed,
            state: self,
        };
        let metadata: &dyn TaskNavigationAccess = metadata.unwrap_or(&replayed);
        let expanded_query = expansion.map(|decision| decision.unresolved_references.join(" "));
        let full_query = expanded_query
            .as_deref()
            .filter(|query| !query.trim().is_empty())
            .or_else(|| (!input_text.trim().is_empty()).then_some(input_text))
            .unwrap_or(input_ref.as_str());
        let query = excerpt(full_query, crate::domain::retrieval::MAX_QUERY_BYTES);
        if query.len() < full_query.len() {
            gaps.push(format!(
                "task navigation query bytes {}..{} were not inspected",
                query.len(),
                full_query.len()
            ));
        }
        let mut pinned = expansion
            .map(|decision| {
                decision
                    .related_task_refs
                    .iter()
                    .cloned()
                    .collect::<std::collections::BTreeSet<_>>()
            })
            .unwrap_or_default();
        if let Some(TaskSelectionDecision {
            focus: crate::domain::task_context::TaskFocus::Existing { task_ref },
            ..
        }) = expansion
        {
            pinned.insert(task_ref.clone());
        }
        if source_refresh
            && let Some((_, previous)) = self.latest_task_context_selection()
            && &previous.input_ref == input_ref
        {
            pinned.extend(
                previous
                    .task_revisions
                    .iter()
                    .map(|revision| revision.task_id.clone()),
            );
            pinned.extend(previous.decision.related_task_refs.iter().cloned());
        }
        let mut task_postings =
            std::collections::BTreeMap::<String, (std::collections::VecDeque<usize>, bool)>::new();
        if postings.is_none() && metadata.task_count() > MAX_CONTEXT_UNITS {
            let keys = crate::domain::retrieval::task_posting_keys(query)
                .into_iter()
                .take(64)
                .collect::<std::collections::BTreeSet<_>>();
            for (ordinal, card) in reconstructed.cards_after(self, None) {
                for key in crate::domain::retrieval::task_posting_keys(&card.goal) {
                    if keys.contains(&key) {
                        let (tail, omitted) = task_postings.entry(key).or_default();
                        if tail.len() == 16 {
                            tail.pop_front();
                            *omitted = true;
                        }
                        tail.push_back(ordinal);
                    }
                }
            }
        }
        let task_pool = if metadata.task_count() > MAX_CONTEXT_UNITS {
            let mut required = std::collections::BTreeSet::new();
            for id in &pinned {
                if let Some(ordinal) = metadata.task_ordinal(id)? {
                    required.insert(ordinal);
                }
            }
            let recent = metadata.recent_cards()?;
            let (pool, incomplete) = task_posting_pool(
                query,
                usize::MAX,
                &required,
                recent.into_iter(),
                |key, limit| {
                    if let Some(postings) = postings {
                        postings(true, key, usize::MAX, limit)
                    } else {
                        let value = task_postings.get(key);
                        Ok((
                            value
                                .map(|(tail, _)| tail.iter().copied().collect())
                                .unwrap_or_default(),
                            value.is_some_and(|(_, omitted)| *omitted),
                        ))
                    }
                },
            )?
            .ok_or_else(|| invalid("pinned task cards exceed the candidate limit"))?;
            if incomplete {
                gaps.push(
                    "Task posting coverage is bounded; other matches require expansion".to_owned(),
                );
            }
            pool
        } else {
            metadata.all_cards()?.into_iter().collect()
        };
        if metadata.task_count() > task_pool.len() {
            gaps.push(
                "Task candidate limit reached; other tasks require search or expansion".to_owned(),
            );
        }
        let mut task_ranker = crate::domain::retrieval::ScopedTaskRanker::new(project_id, query)
            .map_err(|_| invalid("task card ranking failed"))?;
        let mut task_cards = std::collections::BTreeMap::new();
        for ordinal in &task_pool {
            let card = metadata.card(*ordinal)?;
            task_ranker
                .push(&card.reference, &card.goal)
                .map_err(|_| invalid("task card ranking failed"))?;
            task_cards.insert(card.reference.clone(), card);
        }
        let mut tasks = std::collections::BTreeMap::<String, TaskCandidate>::new();
        for id in task_ranker.finish() {
            let card = task_cards
                .remove(&id)
                .ok_or_else(|| invalid("ranked task card is absent"))?;
            tasks.insert(id, card);
        }
        tasks.extend(task_cards);
        let mut navigation = std::collections::BTreeMap::new();
        let selected = if input_index > MAX_CONTEXT_UNITS {
            let mut related = expansion
                .map(|decision| decision.related_task_refs.clone())
                .unwrap_or_default();
            let mut seen = std::collections::BTreeSet::new();
            related.retain(|id| seen.insert(id.clone()));
            if let Some(expansion) = expansion
                && let crate::domain::task_context::TaskFocus::Existing { task_ref } =
                    &expansion.focus
            {
                related.push(task_ref.clone());
            }
            let mut required = metadata
                .related_history(&related, input_index, MAX_CONTEXT_UNITS / 2)?
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>();
            if source_refresh
                && expansion.is_none()
                && let Some((_, previous)) = self.latest_task_context_selection()
                && &previous.input_ref == input_ref
            {
                for id in &previous.selected_history {
                    let index = history_ids[..input_index]
                        .iter()
                        .position(|value| value == id)
                        .ok_or_else(|| invalid("source refresh History dependency is absent"))?;
                    required.insert(index);
                }
                if candidate_policy_version >= 3 {
                    for excerpt in &previous.selected_excerpts {
                        let index = history_ids[..input_index]
                            .iter()
                            .position(|value| value == &excerpt.source_event_id)
                            .ok_or_else(|| {
                                invalid("source refresh excerpt dependency is absent")
                            })?;
                        required.insert(index);
                    }
                }
                if required.len() > MAX_CONTEXT_UNITS {
                    return Err(invalid(
                        "source refresh History dependencies exceed the candidate limit",
                    ));
                }
            }
            // Replay reconstructs posting tails from the source, independently
            // of the capture. Runtime supplies the same tails from its cache.
            let keys = crate::domain::retrieval::task_posting_keys(query)
                .into_iter()
                .take(64)
                .collect::<std::collections::BTreeSet<_>>();
            let mut rebuilt = std::collections::BTreeMap::<
                String,
                (std::collections::VecDeque<usize>, bool),
            >::new();
            if postings.is_none() {
                for (index, item) in history.iter().enumerate().take(input_index) {
                    let (rendered, _) = self.task_navigation_excerpt(index, item, read)?;
                    for key in crate::domain::retrieval::task_posting_keys(&rendered) {
                        if keys.contains(&key) {
                            let (tail, omitted) = rebuilt.entry(key).or_default();
                            if tail.len() == 16 {
                                tail.pop_front();
                                *omitted = true;
                            }
                            tail.push_back(index);
                        }
                    }
                }
            }
            let (pool, incomplete) = task_posting_pool(
                query,
                input_index,
                &required,
                (input_index.saturating_sub(64)..input_index).rev(),
                |key, limit| {
                    if let Some(postings) = postings {
                        postings(false, key, input_index, limit)
                    } else {
                        let value = rebuilt.get(key);
                        Ok((
                            value
                                .map(|(tail, _)| tail.iter().copied().collect())
                                .unwrap_or_default(),
                            value.is_some_and(|(_, omitted)| *omitted),
                        ))
                    }
                },
            )?
            .ok_or_else(|| invalid("required History dependencies exceed the candidate limit"))?;
            let mut ranker = crate::domain::retrieval::ScopedTaskRanker::new(project_id, query)
                .map_err(|_| invalid("task candidate ranking failed"))?;
            for index in &pool {
                let (rendered, truncated) =
                    self.task_navigation_excerpt(*index, &history[*index], read)?;
                ranker
                    .push(history_ids[*index].as_str(), &rendered)
                    .map_err(|_| invalid("task candidate ranking failed"))?;
                navigation.insert(*index, (rendered.into_owned(), truncated));
            }
            let ranked = ranker
                .finish()
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>();
            let mut selected = required;
            for index in &pool {
                if ranked.contains(history_ids[*index].as_str()) {
                    selected.insert(*index);
                }
            }
            for index in pool.iter().rev() {
                if selected.len() == MAX_CONTEXT_UNITS {
                    break;
                }
                selected.insert(*index);
            }
            if incomplete {
                gaps.push(
                    "Task posting coverage is bounded; other matches require expansion".to_owned(),
                );
            }
            let omitted = input_index - selected.len();
            gaps.push(format!(
                "{omitted} older context candidates require expansion"
            ));
            Some(selected)
        } else {
            None
        };
        let mut context = Vec::with_capacity(input_index.min(MAX_CONTEXT_UNITS));
        let mut items = Vec::with_capacity(input_index.min(MAX_CONTEXT_UNITS));
        let mut item_indices = Vec::with_capacity(input_index.min(MAX_CONTEXT_UNITS));
        let cancelled = self.cancelled_initial_inputs();
        let indices: Box<dyn Iterator<Item = usize> + '_> = match &selected {
            Some(selected) => Box::new(selected.iter().copied()),
            None => Box::new(0..input_index),
        };
        for index in indices {
            let id = &history_ids[index];
            if candidate_policy_version >= 4 && cancelled.contains(id) {
                continue;
            }
            let item = &history[index];
            let current_task = metadata.task_at(index)?;
            let (mut rendered, mut truncated) = match navigation.remove(&index) {
                Some((text, truncated)) => (std::borrow::Cow::Owned(text), truncated),
                None => self.task_navigation_excerpt(index, item, read)?,
            };
            if source_refresh
                && let Some(end) = self
                    .latest_task_context_selection()
                    .filter(|(_, selection)| &selection.input_ref == input_ref)
                    .into_iter()
                    .flat_map(|(_, selection)| &selection.selected_excerpts)
                    .filter(|range| &range.source_event_id == id)
                    .map(|range| range.end_byte)
                    .max()
                && end > rendered.len()
            {
                let original = source_text(item)?;
                let prefix = original.get(..end).ok_or_else(|| {
                    invalid("source refresh excerpt is outside its original source")
                })?;
                rendered = std::borrow::Cow::Owned(prefix.to_owned());
                truncated = end < original.len();
            }
            items.push((id, item));
            item_indices.push(index);
            context.push(ContextCandidate {
                reference: id.to_string(),
                task_ref: current_task.clone(),
                excerpt: rendered.into_owned(),
                source_event_ref: Some(id.to_string()),
                truncated,
                core_range: None,
                constraint_range: None,
                research_ref: None,
                memory_ref: None,
                summary_ref: None,
                source_origin: None,
                source_authority: None,
                known_scope: None,
            });
        }
        let semantics = if postings.is_some() {
            let mut values = Vec::with_capacity(context.len());
            for index in item_indices {
                values.push(
                    metadata
                        .semantics(index)?
                        .ok_or_else(|| invalid("candidate source semantics are absent"))?,
                );
            }
            values
        } else {
            self.history_source_semantics_many(&items)?
        };
        for (candidate, semantic) in context.iter_mut().zip(semantics) {
            candidate.source_origin = Some(semantic.origin);
            candidate.source_authority = Some(semantic.authority);
        }
        let input_semantic = if postings.is_some() {
            metadata
                .semantics(input_index)?
                .ok_or_else(|| invalid("input source semantics are absent"))?
        } else {
            self.history_source_semantics(input_ref, &history[input_index])?
        };
        Ok(SelectionCandidates {
            input: SelectionInput {
                reference: input_ref.to_string(),
                text: input_text.to_owned(),
                inspected_bytes: input_text.len(),
                total_bytes: text.len(),
                received_at,
                source_origin: input_semantic.origin,
                source_authority: input_semantic.authority,
                visual_sources: Vec::new(),
            },
            tasks: tasks.into_values().collect(),
            context,
            gaps,
            suppressed_core: Vec::new(),
            expansion_references: expansion
                .map(|decision| decision.unresolved_references.clone())
                .unwrap_or_default(),
        })
    }

    // These distinct durable source, sample and execution inputs must be
    // checked together when the existing Session owner adopts a selection.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_task_selection(
        &self,
        source: SessionSourceIdentity,
        capture_event_id: EventId,
        capture_blob: BlobId,
        input_ref: EventId,
        sample_event_id: Option<EventId>,
        candidates: &SelectionCandidates,
        research_units: &[crate::domain::task_context_research::ResearchTaskUnit],
        live_observations: &[crate::domain::tools::LiveProcessObservation],
        decision: TaskSelectionDecision,
    ) -> Result<(TaskContextSelected, HistoryDeliveryProjection), SessionError> {
        candidates
            .validate_decision(&decision)
            .map_err(|_| invalid("selection references are invalid"))?;
        if candidates.input.reference != input_ref.as_str() {
            return Err(invalid("selection candidates do not belong to this input"));
        }
        let mut selected = Vec::new();
        for reference in &decision.required_context_refs {
            // The current input is already delivered by the protected-input owner.
            if reference == input_ref.as_str() {
                continue;
            }
            let candidate = candidates
                .context
                .iter()
                .find(|candidate| &candidate.reference == reference)
                .ok_or_else(|| invalid("selected context candidate is absent"))?;
            match (
                &candidate.source_event_ref,
                &candidate.core_range,
                &candidate.constraint_range,
                &candidate.research_ref,
                &candidate.memory_ref,
                &candidate.summary_ref,
            ) {
                (Some(event_id), None, None, None, None, None) => {
                    selected.push(EventId::new(event_id))
                }
                (None, Some(_), None, None, None, None)
                | (Some(_), None, Some(_), None, None, None)
                | (None, None, None, Some(_), None, None)
                | (None, None, None, None, Some(_), None)
                | (None, None, None, None, None, Some(_)) => {}
                _ => return Err(invalid("context candidate has an invalid source identity")),
            }
        }
        selected.sort();
        selected.dedup();
        let summaries = candidates
            .context
            .iter()
            .filter_map(|candidate| {
                candidate
                    .summary_ref
                    .as_ref()
                    .filter(|_| {
                        decision
                            .required_context_refs
                            .contains(&candidate.reference)
                    })
                    .cloned()
            })
            .collect::<Vec<_>>();
        if summaries.len() > 1 {
            return Err(invalid("multiple task summaries were selected"));
        }
        let selected_summary = summaries.into_iter().next();
        if let Some(reference) = &selected_summary {
            let compaction = self
                .latest_compaction()
                .ok_or_else(|| invalid("selected Summary is absent"))?;
            if reference != &format!("summary:{}", compaction.summary().fingerprint())
                || compaction.summary().selected_task_id().is_none()
            {
                return Err(invalid(
                    "selected Summary does not match the current task source",
                ));
            }
            let retention = compaction.task_history_retention()?;
            selected.retain(|id| retention.retains(std::slice::from_ref(id)));
        }
        let mut research_refs = candidates
            .context
            .iter()
            .filter_map(|candidate| {
                candidate
                    .research_ref
                    .as_ref()
                    .filter(|_| {
                        decision
                            .required_context_refs
                            .contains(&candidate.reference)
                    })
                    .cloned()
            })
            .collect::<Vec<_>>();
        research_refs
            .extend(self.selected_summary_research_refs(&decision, source.project_stream_hash())?);
        research_refs.sort();
        research_refs.dedup();
        let selected_research = crate::domain::task_context_research::close_research_selection(
            research_units,
            &research_refs,
        )
        .map_err(|_| invalid("Research task dependencies are invalid"))?
        .into_iter()
        .map(|unit| unit.reference.clone())
        .collect::<Vec<_>>();
        let selected_memory = candidates
            .context
            .iter()
            .filter_map(|candidate| {
                candidate
                    .memory_ref
                    .as_ref()
                    .filter(|_| {
                        decision
                            .required_context_refs
                            .contains(&candidate.reference)
                    })
                    .cloned()
            })
            .collect::<Vec<_>>();
        let history = self
            .history_items_with_event_ids()
            .zip(&self.history_event_sequences)
            .filter(|(_, sequence)| **sequence <= source.watermark())
            .map(|(item, _)| item);
        let projection = HistoryDeliveryProjection::from_dependencies(
            history,
            &selected,
            &self.task_recovered_calls(),
        )?;
        let selected_history = projection
            .items()
            .iter()
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        let mut selected_excerpts = Vec::new();
        for requested in &decision.source_ranges {
            let candidate = candidates
                .context
                .iter()
                .find(|candidate| candidate.reference == requested.context_ref)
                .ok_or_else(|| invalid("excerpt candidate is absent"))?;
            let source_id = EventId::new(
                candidate
                    .source_event_ref
                    .as_ref()
                    .ok_or_else(|| invalid("excerpt source Event is absent"))?,
            );
            let item = self
                .history_items_with_event_ids()
                .find(|(id, _)| *id == &source_id)
                .map(|(_, item)| item)
                .ok_or_else(|| invalid("excerpt source Event is absent"))?;
            let text = source_text(item)?;
            if requested.end_byte > text.len()
                || !text.is_char_boundary(requested.start_byte)
                || !text.is_char_boundary(requested.end_byte)
            {
                return Err(invalid("excerpt range is outside its original source"));
            }
            if selected_history.contains(&source_id) {
                continue;
            }
            let content = &text[requested.start_byte..requested.end_byte];
            let hash = crate::domain::canonical_json::canonical_blake3_hash(&content)
                .map_err(|_| invalid("excerpt hash failed"))?;
            selected_excerpts.push(SourceRange {
                source_event_id: source_id,
                start_byte: requested.start_byte,
                end_byte: requested.end_byte,
                content_hash: hash,
            });
        }
        for previous in decision.constraint_updates.iter().filter_map(|update| {
            update
                .supersedes
                .as_ref()
                .filter(|previous| decision.required_context_refs.contains(previous))
        }) {
            let range = candidates
                .context
                .iter()
                .find(|candidate| &candidate.reference == previous)
                .and_then(|candidate| candidate.constraint_range.as_ref())
                .ok_or_else(|| invalid("superseded constraint source is absent"))?;
            if !selected_history.contains(&range.source_event_id)
                && !selected_excerpts.iter().any(|excerpt| {
                    excerpt.source_event_id == range.source_event_id
                        && excerpt.start_byte <= range.start_byte
                        && excerpt.end_byte >= range.end_byte
                })
            {
                selected_excerpts.push(range.clone());
            }
        }
        let excerpt_ids = selected_excerpts
            .iter()
            .map(|range| range.source_event_id.clone())
            .collect::<std::collections::BTreeSet<_>>();
        let order = self
            .history_items_with_event_ids()
            .enumerate()
            .filter(|(_, (id, _))| excerpt_ids.contains(*id))
            .map(|(index, (id, _))| (id.clone(), index))
            .collect::<std::collections::BTreeMap<_, _>>();
        selected_excerpts.sort_by_key(|range| {
            (
                order.get(&range.source_event_id).copied(),
                range.start_byte,
                range.end_byte,
            )
        });
        selected_excerpts.dedup();
        let live_processes = self.live_process_references(live_observations)?;
        let task_id = decision.focus.task_id();
        let (goal, source_range) = match &decision.focus {
            crate::domain::task_context::TaskFocus::Existing { task_ref } => {
                let task = candidates
                    .tasks
                    .iter()
                    .find(|task| &task.reference == task_ref)
                    .ok_or_else(|| invalid("selected task is absent"))?;
                (task.goal.clone(), None)
            }
            crate::domain::task_context::TaskFocus::NewFromInput { .. } => {
                (candidates.input.text.clone(), None)
            }
        };
        if let Some(reference) = &selected_summary {
            let candidate = candidates
                .context
                .iter()
                .find(|candidate| &candidate.reference == reference)
                .ok_or_else(|| invalid("selected Summary candidate is absent"))?;
            if candidate.task_ref.as_deref().is_none_or(|scope| {
                scope != task_id && !decision.related_task_refs.iter().any(|id| id == scope)
            }) {
                return Err(invalid("selected Summary is outside the chosen task scope"));
            }
            let summary = self
                .latest_compaction()
                .ok_or_else(|| invalid("selected Summary is absent"))?
                .summary();
            if summary
                .task_scope()
                .iter()
                .any(|scope| scope != &task_id && !decision.related_task_refs.contains(scope))
            {
                return Err(invalid(
                    "selected Summary contains a task outside the chosen scope",
                ));
            }
        }
        let mut selected_core = Vec::new();
        let mut core_scopes = candidates
            .suppressed_core
            .iter()
            .map(|range| CoreScopeRevision {
                reference: format!(
                    "core:{}:{}:{}:{}",
                    range.fragment_id, range.start_byte, range.end_byte, range.content_hash
                ),
                scope: UnitScope::Global,
                suppressed: true,
            })
            .collect::<Vec<_>>();
        for candidate in &candidates.context {
            let Some(range) = &candidate.core_range else {
                continue;
            };
            if decision
                .constraint_updates
                .iter()
                .any(|update| update.supersedes.as_deref() == Some(candidate.reference.as_str()))
            {
                core_scopes.push(CoreScopeRevision {
                    reference: candidate.reference.clone(),
                    scope: UnitScope::Global,
                    suppressed: true,
                });
                continue;
            }
            let scope = decision
                .scope_classifications
                .iter()
                .find(|classification| classification.context_ref == candidate.reference)
                .map(|classification| classification.scope.clone())
                .or_else(|| candidate.known_scope.clone())
                .unwrap_or(UnitScope::Unknown)
                .for_task(&task_id);
            let selected_explicitly = decision
                .required_context_refs
                .contains(&candidate.reference);
            if selected_explicitly
                || matches!(scope, UnitScope::Global | UnitScope::Unknown)
                || matches!(&scope, UnitScope::Task { task_ref } if task_ref == &task_id)
            {
                selected_core.push(range.clone());
            }
            core_scopes.push(CoreScopeRevision {
                reference: candidate.reference.clone(),
                scope,
                suppressed: false,
            });
        }
        let mut active_constraints = self.active_task_constraints()?;
        let mut constraint_revisions = Vec::new();
        for update in &decision.constraint_updates {
            let source_id = if update.source_ref == candidates.input.reference {
                input_ref.clone()
            } else {
                let candidate = candidates
                    .context
                    .iter()
                    .find(|candidate| candidate.reference == update.source_ref)
                    .ok_or_else(|| invalid("constraint source candidate is absent"))?;
                EventId::new(
                    candidate
                        .source_event_ref
                        .as_ref()
                        .ok_or_else(|| invalid("constraint source is not an Event"))?,
                )
            };
            let item = self
                .history_items_with_event_ids()
                .find(|(id, _)| *id == &source_id)
                .map(|(_, item)| item)
                .ok_or_else(|| invalid("constraint source Event is absent"))?;
            let crate::domain::model::ContentItem::Text {
                role: crate::domain::model::MessageRole::User,
                text,
            } = item
            else {
                return Err(invalid("constraint source must be a user input"));
            };
            let semantic = self.history_source_semantics(&source_id, item)?;
            if semantic.origin != crate::domain::context::ContextOrigin::OperatorInput
                || semantic.authority != crate::domain::context::AuthorityLevel::OperatorAssertion
            {
                return Err(invalid("constraint source lacks operator authority"));
            }
            if update.end_byte > text.len()
                || !text.is_char_boundary(update.start_byte)
                || !text.is_char_boundary(update.end_byte)
            {
                return Err(invalid("constraint range is outside its original input"));
            }
            let content = &text[update.start_byte..update.end_byte];
            let hash = crate::domain::canonical_json::canonical_blake3_hash(&content)
                .map_err(|_| invalid("constraint hash failed"))?;
            let range = ConstraintRange {
                source_event_id: source_id.clone(),
                start_byte: update.start_byte,
                end_byte: update.end_byte,
                content_hash: hash.clone(),
            };
            let reference = format!(
                "constraint:{source_id}:{}:{}:{hash}",
                update.start_byte, update.end_byte
            );
            let scope = update.scope.for_task(&task_id);
            if let Some(existing) = active_constraints.get(&reference) {
                if update.supersedes.is_none() && existing.scope == scope {
                    continue;
                }
                return Err(invalid(
                    "constraint identity conflicts with its active scope or supersession",
                ));
            }
            let revision = match &update.supersedes {
                Some(previous) => {
                    if let Some(prior) = active_constraints.remove(previous) {
                        prior
                            .revision
                            .checked_add(1)
                            .ok_or_else(|| invalid("constraint revision overflow"))?
                    } else {
                        let core = candidates
                            .context
                            .iter()
                            .find(|candidate| {
                                &candidate.reference == previous
                                    && candidate.core_range.is_some()
                                    && candidate.known_scope == Some(UnitScope::Global)
                            })
                            .ok_or_else(|| {
                                invalid("constraint supersedes an absent active revision")
                            })?;
                        if core.truncated {
                            return Err(invalid(
                                "Core correction requires an inspected global source",
                            ));
                        }
                        1
                    }
                }
                None => 1,
            };
            let value = ConstraintRevision {
                reference: reference.clone(),
                range,
                scope,
                revision,
                supersedes: update.supersedes.clone(),
            };
            active_constraints.insert(reference, value.clone());
            constraint_revisions.push(value);
        }
        let mut selected_constraints = active_constraints.into_values().filter(|constraint| {
            decision.required_context_refs.contains(&constraint.reference)
                || matches!(constraint.scope, UnitScope::Global | UnitScope::Unknown)
                || matches!(&constraint.scope, UnitScope::Task { task_ref } if task_ref == &task_id || decision.related_task_refs.contains(task_ref))
        }).map(|constraint| constraint.range).collect::<Vec<_>>();
        let constraint_ids = selected_constraints
            .iter()
            .map(|range| range.source_event_id.clone())
            .collect::<std::collections::BTreeSet<_>>();
        let order = self
            .history_items_with_event_ids()
            .enumerate()
            .filter(|(_, (id, _))| constraint_ids.contains(*id))
            .map(|(index, (id, _))| (id.clone(), index))
            .collect::<std::collections::BTreeMap<_, _>>();
        selected_constraints.sort_by_key(|range| {
            (
                order.get(&range.source_event_id).copied(),
                range.start_byte,
                range.end_byte,
            )
        });
        let previous = self
            .task_context_selections
            .iter()
            .rev()
            .flat_map(|(_, selection)| &selection.task_revisions)
            .find(|revision| revision.task_id == task_id);
        let revision = previous
            .map_or(Some(1), |revision| revision.revision.checked_add(1))
            .ok_or_else(|| invalid("task revision overflow"))?;
        let mut task_revisions = vec![TaskViewRevision {
            task_id: task_id.clone(),
            revision,
            goal,
            input_range: source_range,
        }];
        for (index, objective) in decision.new_objectives.iter().enumerate() {
            task_revisions.push(TaskViewRevision {
                task_id: format!("task:{input_ref}:{}", index + 1),
                revision: 1,
                goal: objective.description.clone(),
                input_range: Some((objective.start_byte, objective.end_byte)),
            });
        }
        let mut unresolved_gaps = Vec::new();
        if candidates.input.inspected_bytes < candidates.input.total_bytes {
            unresolved_gaps.push(format!(
                "input bytes {}..{} were not inspected",
                candidates.input.inspected_bytes, candidates.input.total_bytes
            ));
        }
        for candidate in &candidates.context {
            if candidate.truncated
                && candidate
                    .core_range
                    .as_ref()
                    .is_some_and(|range| selected_core.contains(range))
            {
                unresolved_gaps.push(format!(
                    "Core section {} has an uninspected suffix",
                    candidate.reference
                ));
            }
        }
        unresolved_gaps.extend(
            decision
                .unresolved_references
                .iter()
                .filter(|gap| !gap.trim().is_empty())
                .cloned(),
        );
        let current = self
            .primary_provider_session()
            .ok_or_else(|| invalid("primary session is absent"))?;
        let preserve_epoch = self
            .task_context_selections
            .last()
            .is_some_and(|(_, previous)| {
                if previous
                    .task_revisions
                    .first()
                    .is_none_or(|old| old.task_id != task_id)
                {
                    return false;
                }
                if previous.selected_core != selected_core
                    || previous.selected_research != selected_research
                    || previous.selected_memory != selected_memory
                    || previous.selected_summary != selected_summary
                    || previous.selected_constraints != selected_constraints
                    || previous.selected_excerpts != selected_excerpts
                    || previous.unresolved_gaps != unresolved_gaps
                    || previous.live_processes != live_processes
                {
                    return false;
                }
                if !selected_research.is_empty()
                    && previous.source.project_stream_hash() != source.project_stream_hash()
                {
                    return false;
                }
                let selected = selected_history
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>();
                if previous
                    .selected_history
                    .iter()
                    .any(|id| !selected.contains(id))
                {
                    return false;
                }
                let Some(start) = self
                    .history_event_ids
                    .iter()
                    .position(|id| id == &previous.input_ref)
                else {
                    return false;
                };
                let Some(end) = self
                    .history_event_ids
                    .iter()
                    .position(|id| id == &input_ref)
                else {
                    return false;
                };
                start <= end
                    && self.history_event_ids[start..end]
                        .iter()
                        .all(|id| selected.contains(id))
            });
        let provider_session = if self.task_context_selections.is_empty() || preserve_epoch {
            current.clone()
        } else {
            crate::domain::context::ProviderSessionRef {
                id: crate::domain::context::ProviderSessionId::new(format!(
                    "task-epoch:{capture_event_id}"
                )),
                role: crate::domain::context::ProviderSessionRole::Primary,
                generation: current
                    .generation
                    .checked_add(1)
                    .ok_or_else(|| invalid("provider epoch generation overflow"))?,
            }
        };
        let binding_digest = self
            .agent_definition_binding
            .as_ref()
            .ok_or_else(|| invalid("task selection requires a bound definition"))?
            .definition()
            .digest()
            .as_str()
            .to_owned();
        let (input_boundary_event_id, input_time) = self.task_input_time(&input_ref)?;
        Ok((
            TaskContextSelected {
                schema_version: 1,
                request_proof_version: TASK_REQUEST_PROOF_VERSION,
                source,
                capture_event_id,
                capture_blob,
                input_ref,
                input_boundary_event_id,
                input_time,
                sample_event_id,
                refresh_of: None,
                unresolved_gaps,
                live_processes,
                decision,
                selected_history,
                selected_excerpts,
                selected_core,
                selected_research,
                selected_memory,
                selected_summary,
                core_scopes,
                constraint_revisions,
                selected_constraints,
                task_revisions,
                policy_version: 1,
                binding_digest,
                provider_session,
                planned_context_hash: String::new(),
            },
            projection,
        ))
    }
}

fn source_is_available(
    state: &SessionState,
    source: &SessionSourceIdentity,
    capture_event: &EventId,
    capture: &BlobId,
    input: &EventId,
) -> bool {
    state.stream_id() == Some(source.stream_id())
        && state.task_context_captures.iter().any(|(id, sequence, blob)| {
            id == capture_event && blob == capture && *sequence == source.watermark()
        })
        && state.operator_boundaries.iter().any(|boundary| {
            matches!(boundary, OperatorTurnBoundary::Accepted { anchor, .. } if anchor == input)
        })
        && state.history_items_with_event_ids().zip(&state.history_event_sequences).any(|((id, _), sequence)| {
            id == input && *sequence <= source.watermark()
        })
}

pub(super) fn apply_sample(
    state: &mut SessionState,
    sample: TaskSelectionSample,
    event_id: &EventId,
) -> Result<(), SessionError> {
    state.ensure_completed_history()?;
    super::require_primary_provider_session(&sample.provider_session)?;
    let audit = state
        .request_audits
        .iter()
        .find(|audit| audit.audit_event_id() == &sample.audit_event_id)
        .ok_or_else(|| invalid("selection sample audit is absent"))?;
    if !source_is_available(
        state,
        &sample.source,
        &sample.capture_event_id,
        &sample.capture_blob,
        &sample.input_ref,
    ) || audit.provider_session() != Some(&sample.provider_session)
        || audit.improvement_scope().is_some()
        || audit.delivery().mode() != RequestAuditDeliveryMode::Full
        || audit.context_delivery_map().is_none()
        || audit
            .components()
            .iter()
            .any(|component| component.source() == RequestAuditComponentSource::RenderedToolSpec)
        || state.primary_provider_session.as_ref() == Some(&sample.provider_session)
        || state
            .used_primary_provider_session_ids
            .contains(&sample.provider_session.id)
        || state.task_selection_samples.iter().any(|(_, previous)| {
            previous.audit_event_id == sample.audit_event_id
                || previous.provider_session == sample.provider_session
        })
        || state
            .task_selection_samples
            .iter()
            .filter(|(_, previous)| previous.input_ref == sample.input_ref)
            .count()
            >= 2
    {
        return Err(invalid(
            "selection sample does not match its isolated audited source",
        ));
    }
    state
        .task_selection_samples
        .push((event_id.clone(), sample));
    Ok(())
}

pub(super) fn apply_selection(
    state: &mut SessionState,
    selection: TaskContextSelected,
    event_id: &EventId,
) -> Result<(), SessionError> {
    state.ensure_completed_history()?;
    super::require_primary_provider_session(&selection.provider_session)?;
    if !source_is_available(
        state,
        &selection.source,
        &selection.capture_event_id,
        &selection.capture_blob,
        &selection.input_ref,
    ) || selection.policy_version != 1
        || selection.task_revisions.is_empty()
        || !crate::domain::agent_definition::is_blake3_digest(&selection.binding_digest)
        || !crate::domain::agent_definition::is_blake3_digest(&selection.planned_context_hash)
        || state
            .agent_definition_binding
            .as_ref()
            .is_none_or(|binding| {
                binding.definition().digest().as_str() != selection.binding_digest
            })
    {
        return Err(invalid(
            "task selection source, binding or policy is invalid",
        ));
    }
    match (&selection.sample_event_id, &selection.refresh_of) {
        (Some(sample_id), None) => {
            let sample = state
                .task_selection_samples
                .iter()
                .find(|(id, _)| id == sample_id)
                .map(|(_, sample)| sample)
                .ok_or_else(|| invalid("selection sample is absent"))?;
            if sample.source != selection.source
                || sample.capture_event_id != selection.capture_event_id
                || sample.capture_blob != selection.capture_blob
                || sample.input_ref != selection.input_ref
                || sample.decision != selection.decision
            {
                return Err(invalid("selection differs from its accepted sample"));
            }
        }
        (None, Some(prior_id)) => {
            let (id, previous) = state
                .latest_task_context_selection()
                .ok_or_else(|| invalid("source refresh has no prior task"))?;
            let (selected_history, selected_summary) =
                state.task_refresh_projection(id, previous, selection.has_complete_request_proof());
            if id != prior_id
                || previous.input_ref != selection.input_ref
                || previous.task_revisions != selection.task_revisions
                || selected_history != selection.selected_history
                || previous.selected_excerpts != selection.selected_excerpts
                || previous.selected_constraints != selection.selected_constraints
                || selected_summary != selection.selected_summary.as_ref()
                || !selection.constraint_revisions.is_empty()
                || selection.decision.relation
                    != crate::domain::task_context::TaskRelation::Continue
                || !matches!(&selection.decision.focus, crate::domain::task_context::TaskFocus::Existing { task_ref } if previous.task_revisions.first().is_some_and(|revision| &revision.task_id == task_ref))
            {
                return Err(invalid(
                    "source refresh changed the adopted task interpretation",
                ));
            }
        }
        (None, None) => {
            if selection.request_proof_version >= 5
                || state.history_event_ids.first() != Some(&selection.input_ref)
                || !state.task_context_selections.is_empty()
                || selection.decision.relation != crate::domain::task_context::TaskRelation::New
                || !matches!(&selection.decision.focus,
                    crate::domain::task_context::TaskFocus::NewFromInput { input_ref }
                    if input_ref == selection.input_ref.as_str())
                || !selection.decision.related_task_refs.is_empty()
                || !selection.decision.required_context_refs.is_empty()
            {
                return Err(invalid(
                    "only an empty initial task may omit a selection sample",
                ));
            }
        }
        (Some(_), Some(_)) => {
            return Err(invalid("source refresh cannot claim a selection sample"));
        }
    }
    let source = state
        .history_items_with_event_ids()
        .zip(&state.history_event_sequences)
        .filter(|(_, sequence)| **sequence <= selection.source.watermark())
        .map(|(item, _)| item);
    let projection = HistoryDeliveryProjection::from_selected(
        source,
        &selection.selected_history,
        &state.task_recovered_calls(),
    )?;
    let active_input = state.protected_operator_inputs()?;
    if projection.items().iter().any(|(id, _)| {
        active_input
            .iter()
            .any(|input| input.anchor == selection.input_ref && input.event_id == *id)
    }) {
        return Err(invalid(
            "current input must not be duplicated in selected history",
        ));
    }
    let mut seen = std::collections::BTreeSet::new();
    for revision in &selection.task_revisions {
        if revision.task_id.trim().is_empty()
            || revision.goal.trim().is_empty()
            || !seen.insert(&revision.task_id)
        {
            return Err(invalid("task revision identity or goal is invalid"));
        }
        let previous = state
            .task_context_selections
            .iter()
            .rev()
            .flat_map(|(_, old)| &old.task_revisions)
            .find(|old| old.task_id == revision.task_id);
        match previous {
            Some(old) if selection.refresh_of.is_some() && old == revision => {}
            Some(old) if old.revision.checked_add(1) == Some(revision.revision) => {}
            None if revision.revision == 1 => {}
            _ => return Err(invalid("task revision is not the next revision")),
        }
        if let Some((start, end)) = revision.input_range {
            let input = state
                .history_items_with_event_ids()
                .find(|(id, _)| *id == &selection.input_ref)
                .map(|(_, item)| item)
                .ok_or_else(|| invalid("task input is absent"))?;
            let crate::domain::model::ContentItem::Text { text, .. } = input else {
                return Err(invalid("task input range requires text"));
            };
            if start >= end
                || end > text.len()
                || !text.is_char_boundary(start)
                || !text.is_char_boundary(end)
            {
                return Err(invalid("task input range is invalid"));
            }
        }
    }
    if state
        .task_selection_samples
        .iter()
        .any(|(_, sample)| sample.provider_session.id == selection.provider_session.id)
    {
        return Err(invalid(
            "primary epoch collides with an isolated selection scope",
        ));
    }
    if state.primary_provider_session.as_ref() != Some(&selection.provider_session) {
        super::record_fresh_primary_provider_session_id(state, &selection.provider_session)?;
        state.primary_provider_session = Some(selection.provider_session.clone());
    }
    state
        .task_context_selections
        .push((event_id.clone(), selection));
    state.task_compacted_selection = None;
    Ok(())
}

pub(super) fn apply_task_compaction(
    state: &mut SessionState,
    compaction: &super::SessionCompaction,
) -> Result<(), SessionError> {
    let Some((selection_id, selection)) = state.latest_task_context_selection() else {
        return Ok(());
    };
    if compaction.summary().schema_version() != 7
        || compaction.summary().task_selection_event() != Some(selection_id)
    {
        return Ok(());
    }
    let selection_id = selection_id.clone();
    let mut history = state
        .task_compacted_selection
        .as_ref()
        .filter(|(id, _, _)| id == &selection_id)
        .map_or_else(
            || selection.selected_history.clone(),
            |(_, history, _)| history.clone(),
        );
    let retention = compaction.task_history_retention()?;
    history.retain(|id| retention.retains(std::slice::from_ref(id)));
    state.task_compacted_selection = Some((
        selection_id,
        history,
        format!("summary:{}", compaction.summary().fingerprint()),
    ));
    Ok(())
}
