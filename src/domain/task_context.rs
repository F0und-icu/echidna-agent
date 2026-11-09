//! Source-bound interpretations for the fixed task-context preparation stage.
//!
//! These values are derived navigation, not user instructions or execution
//! facts. The Session and original source stores remain authoritative.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub(crate) const SELECTION_INPUT_MARKER: &str = "TASK_CONTEXT_CANDIDATES_V1\n";

/// One actual input-receive instant and the observed host timezone at that
/// same instant. Missing platform timezone information stays explicitly unknown.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TaskInputTime {
    pub(crate) unix_seconds: i64,
    pub(crate) nanoseconds: u32,
    pub(crate) timezone: TimeZoneObservation,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
pub enum TimeZoneObservation {
    HostSystem {
        offset_seconds: i32,
        name: Option<String>,
    },
    Unknown,
}

impl TaskInputTime {
    pub(crate) fn validate(&self) -> bool {
        self.nanoseconds < 1_000_000_000
            && match &self.timezone {
                TimeZoneObservation::HostSystem {
                    offset_seconds,
                    name,
                } => {
                    (-86_400..=86_400).contains(offset_seconds)
                        && name.as_ref().is_none_or(|name| !name.is_empty())
                }
                TimeZoneObservation::Unknown => true,
            }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SelectionInput {
    #[serde(rename = "ref")]
    pub(crate) reference: String,
    pub(crate) text: String,
    pub(crate) inspected_bytes: usize,
    pub(crate) total_bytes: usize,
    pub(crate) received_at: TaskInputTime,
    pub(crate) source_origin: crate::domain::context::ContextOrigin,
    pub(crate) source_authority: crate::domain::context::AuthorityLevel,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) visual_sources: Vec<SelectionVisualSource>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SelectionVisualSource {
    pub(crate) history_event_ref: String,
    pub(crate) source_event_refs: Vec<String>,
    pub(crate) artifact_refs: Vec<String>,
    pub(crate) observation_text: Option<String>,
    pub(crate) inspected_bytes: usize,
    pub(crate) total_bytes: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TaskCandidate {
    #[serde(rename = "ref")]
    pub(crate) reference: String,
    pub(crate) goal: String,
    pub(crate) source_ref: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ContextCandidate {
    #[serde(rename = "ref")]
    pub(crate) reference: String,
    pub(crate) task_ref: Option<String>,
    pub(crate) excerpt: String,
    pub(crate) source_event_ref: Option<String>,
    pub(crate) truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) core_range: Option<CoreRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) constraint_range: Option<ConstraintRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) research_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) memory_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) summary_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) source_origin: Option<crate::domain::context::ContextOrigin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) source_authority: Option<crate::domain::context::AuthorityLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) known_scope: Option<UnitScope>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SourceRange {
    pub(crate) source_event_id: crate::domain::events::EventId,
    pub(crate) start_byte: usize,
    pub(crate) end_byte: usize,
    pub(crate) content_hash: String,
}

pub(crate) type ConstraintRange = SourceRange;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SourceRangeSelection {
    pub(crate) context_ref: String,
    pub(crate) start_byte: usize,
    pub(crate) end_byte: usize,
}

pub(crate) struct SourceExcerpt {
    pub(crate) range: SourceRange,
    pub(crate) text: String,
    pub(crate) semantics: crate::domain::session::HistorySourceSemantics,
    pub(crate) role: Option<crate::domain::model::MessageRole>,
    pub(crate) receipt_event_id: Option<crate::domain::events::EventId>,
    pub(crate) vision_references: Option<crate::domain::context::ContextReferences>,
    pub(crate) observed_at: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CoreRange {
    pub(crate) fragment_id: String,
    pub(crate) start_byte: usize,
    pub(crate) end_byte: usize,
    pub(crate) content_hash: String,
    pub(crate) inspected_bytes: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum UnitScope {
    Global,
    CurrentTask,
    Task { task_ref: String },
    Unknown,
}

impl UnitScope {
    pub(crate) fn for_task(&self, task_ref: &str) -> Self {
        match self {
            Self::CurrentTask => Self::Task {
                task_ref: task_ref.to_owned(),
            },
            _ => self.clone(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScopeClassification {
    pub(crate) context_ref: String,
    pub(crate) scope: UnitScope,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConstraintUpdate {
    pub(crate) source_ref: String,
    pub(crate) start_byte: usize,
    pub(crate) end_byte: usize,
    pub(crate) scope: UnitScope,
    pub(crate) supersedes: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SelectionCandidates {
    pub(crate) input: SelectionInput,
    pub(crate) tasks: Vec<TaskCandidate>,
    pub(crate) context: Vec<ContextCandidate>,
    pub(crate) gaps: Vec<String>,
    /// Current exact Core ranges whose prior, uniquely matched section was
    /// explicitly superseded. Their bodies are not selection candidates.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) suppressed_core: Vec<CoreRange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) expansion_references: Vec<String>,
}

impl SelectionCandidates {
    pub(crate) fn validate_decision(
        &self,
        decision: &TaskSelectionDecision,
    ) -> Result<(), TaskSelectionError> {
        let tasks = self
            .tasks
            .iter()
            .map(|candidate| candidate.reference.clone())
            .collect();
        let context = self
            .context
            .iter()
            .map(|candidate| candidate.reference.clone())
            .chain(std::iter::once(self.input.reference.clone()))
            .collect();
        decision.validate_references(&self.input.reference, &tasks, &context)?;
        let current_task = decision.focus.task_id();
        for objective in &decision.new_objectives {
            if objective.description.trim().is_empty()
                || objective.start_byte >= objective.end_byte
                || objective.end_byte > self.input.inspected_bytes
                || !self.input.text.is_char_boundary(objective.start_byte)
                || !self.input.text.is_char_boundary(objective.end_byte)
            {
                return Err(TaskSelectionError::InvalidObjectiveRange);
            }
        }
        let mut classified = BTreeMap::new();
        for classification in &decision.scope_classifications {
            let candidate = self
                .context
                .iter()
                .find(|candidate| candidate.reference == classification.context_ref)
                .ok_or(TaskSelectionError::InvalidScopeClassification)?;
            let scope = classification.scope.for_task(&current_task);
            if classified
                .insert(&classification.context_ref, scope.clone())
                .is_some_and(|previous| previous != scope)
                || matches!(&classification.scope, UnitScope::Task { task_ref } if !tasks.contains(task_ref))
            {
                return Err(TaskSelectionError::InvalidScopeClassification);
            }
            let Some(core) = candidate.core_range.as_ref() else {
                if candidate.constraint_range.is_some()
                    && candidate
                        .known_scope
                        .as_ref()
                        .is_some_and(|known| known.for_task(&current_task) == scope)
                {
                    continue;
                }
                return Err(TaskSelectionError::InvalidScopeClassification);
            };
            if (core.inspected_bytes < core.end_byte - core.start_byte
                && classification.scope != UnitScope::Unknown)
                || (candidate.known_scope == Some(UnitScope::Global)
                    && classification.scope != UnitScope::Global)
            {
                return Err(TaskSelectionError::InvalidScopeClassification);
            }
        }
        let mut superseded = BTreeSet::new();
        for update in &decision.constraint_updates {
            let text = if update.source_ref == self.input.reference {
                if self.input.source_origin != crate::domain::context::ContextOrigin::OperatorInput
                    || self.input.source_authority
                        != crate::domain::context::AuthorityLevel::OperatorAssertion
                {
                    return Err(TaskSelectionError::InvalidConstraintUpdate);
                }
                Some(self.input.text.as_str())
            } else {
                self.context
                    .iter()
                    .find(|candidate| {
                        candidate.reference == update.source_ref
                            && candidate.source_event_ref.is_some()
                            && candidate.constraint_range.is_none()
                            && candidate.source_origin
                                == Some(crate::domain::context::ContextOrigin::OperatorInput)
                            && candidate.source_authority
                                == Some(crate::domain::context::AuthorityLevel::OperatorAssertion)
                    })
                    .map(|candidate| candidate.excerpt.as_str())
            }
            .ok_or(TaskSelectionError::InvalidConstraintUpdate)?;
            if update.start_byte >= update.end_byte
                || update.end_byte > text.len()
                || !text.is_char_boundary(update.start_byte)
                || !text.is_char_boundary(update.end_byte)
                || matches!(&update.scope, UnitScope::Task { task_ref } if !tasks.contains(task_ref))
                || (update.supersedes.is_some() && update.source_ref != self.input.reference)
                || update
                    .supersedes
                    .as_ref()
                    .is_some_and(|previous| !superseded.insert(previous))
                || update.supersedes.as_ref().is_some_and(|previous| {
                    !self.context.iter().any(|candidate| {
                        &candidate.reference == previous
                            && (candidate.constraint_range.is_some()
                                || (candidate.core_range.is_some()
                                    && candidate.known_scope == Some(UnitScope::Global)
                                    && !decision.required_context_refs.contains(previous)))
                    })
                })
            {
                return Err(TaskSelectionError::InvalidConstraintUpdate);
            }
        }
        for range in &decision.source_ranges {
            let candidate = self
                .context
                .iter()
                .find(|candidate| candidate.reference == range.context_ref)
                .ok_or(TaskSelectionError::InvalidSourceRange)?;
            if candidate.source_event_ref.is_none()
                || candidate.constraint_range.is_some()
                || candidate.core_range.is_some()
                || range.start_byte >= range.end_byte
                || range.end_byte > candidate.excerpt.len()
                || !candidate.excerpt.is_char_boundary(range.start_byte)
                || !candidate.excerpt.is_char_boundary(range.end_byte)
            {
                return Err(TaskSelectionError::InvalidSourceRange);
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TaskRelation {
    Continue,
    New,
    Return,
    Compose,
    Unresolved,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum TaskFocus {
    NewFromInput { input_ref: String },
    Existing { task_ref: String },
}

impl TaskFocus {
    pub(crate) fn task_id(&self) -> String {
        match self {
            Self::NewFromInput { input_ref } => format!("task:{input_ref}"),
            Self::Existing { task_ref } => task_ref.clone(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NewObjective {
    pub(crate) start_byte: usize,
    pub(crate) end_byte: usize,
    pub(crate) description: String,
}

/// The exact shape accepted from a tool-free selection sample. Every reference
/// is checked against the candidates supplied to that sample before adoption.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TaskSelectionDecision {
    pub(crate) relation: TaskRelation,
    pub(crate) focus: TaskFocus,
    pub(crate) related_task_refs: Vec<String>,
    pub(crate) required_context_refs: Vec<String>,
    pub(crate) unresolved_references: Vec<String>,
    /// Multiple objectives remain ranges of the same original input, never
    /// fabricated independent User messages.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) new_objectives: Vec<NewObjective>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) scope_classifications: Vec<ScopeClassification>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) constraint_updates: Vec<ConstraintUpdate>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) source_ranges: Vec<SourceRangeSelection>,
}

impl TaskSelectionDecision {
    pub(crate) fn validate_references(
        &self,
        input_ref: &str,
        task_refs: &BTreeSet<String>,
        context_refs: &BTreeSet<String>,
    ) -> Result<(), TaskSelectionError> {
        match (&self.relation, &self.focus) {
            (
                TaskRelation::New | TaskRelation::Compose | TaskRelation::Unresolved,
                TaskFocus::NewFromInput {
                    input_ref: selected,
                },
            ) if selected == input_ref => {}
            (
                TaskRelation::Continue
                | TaskRelation::Return
                | TaskRelation::Compose
                | TaskRelation::Unresolved,
                TaskFocus::Existing { task_ref },
            ) if task_refs.contains(task_ref) => {}
            _ => return Err(TaskSelectionError::InvalidFocus),
        }
        require_known(&self.related_task_refs, task_refs)?;
        require_known(&self.required_context_refs, context_refs)?;
        Ok(())
    }
}

fn require_known(
    selected: &[String],
    candidates: &BTreeSet<String>,
) -> Result<(), TaskSelectionError> {
    for reference in selected {
        if !candidates.contains(reference) {
            return Err(TaskSelectionError::UnknownReference {
                reference: reference.clone(),
            });
        }
    }
    Ok(())
}

#[derive(Debug, Error)]
pub(crate) enum TaskSelectionError {
    #[error("task selection focus does not match its relation or supplied candidates")]
    InvalidFocus,
    #[error("task selection references unknown candidate `{reference}`")]
    UnknownReference { reference: String },
    #[error("task objective range is outside the inspected input")]
    InvalidObjectiveRange,
    #[error("context scope classification is not supported by the inspected source")]
    InvalidScopeClassification,
    #[error("constraint update is not bound to the inspected input or an existing constraint")]
    InvalidConstraintUpdate,
    #[error("selected source range is not an inspected original Event range")]
    InvalidSourceRange,
}
