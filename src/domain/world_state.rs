//! Bounded, derived Research and Project snapshot Context renderers.
//!
//! Research JSONL and the project-inspection cache remain authoritative. This
//! module only turns their already-derived values into deterministic Context
//! Fragments; it never writes facts or decides Provider delivery.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;

use crate::domain::canonical_json::{canonical_blake3_hash, canonical_json};
use crate::domain::context::{
    ContextFragment, ContextFragmentContent, ContextFragmentId, ContextFragmentPriority,
    ContextFragmentSource, FragmentCachePolicy,
};
use crate::domain::model::{ContentItem, MessageRole};
use crate::domain::research::{
    ExperimentResult, FindingStatus, HypothesisStatus, NextActionStatus, ResearchRecordRef,
    ResearchReferenceTargetApplicability, ResearchState, TargetSnapshotRecord,
};
use crate::domain::snapshot::ProjectSnapshot;

/// The fixed research-only section vocabulary frozen in Architecture v1.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchWorldStateSection {
    Target,
    TargetSnapshot,
    TargetDelta,
    Environment,
    ProjectSnapshot,
    AttackSurface,
    ActiveHypotheses,
    RecentExperiments,
    Findings,
    FailedPaths,
    ActiveJobs,
    NextActions,
}

impl ResearchWorldStateSection {
    pub const ALL: [Self; 12] = [
        Self::Target,
        Self::TargetSnapshot,
        Self::TargetDelta,
        Self::Environment,
        Self::ProjectSnapshot,
        Self::AttackSurface,
        Self::ActiveHypotheses,
        Self::RecentExperiments,
        Self::Findings,
        Self::FailedPaths,
        Self::ActiveJobs,
        Self::NextActions,
    ];

    pub const fn id(self) -> &'static str {
        match self {
            Self::Target => "target",
            Self::TargetSnapshot => "target_snapshot",
            Self::TargetDelta => "target_delta",
            Self::Environment => "environment",
            Self::ProjectSnapshot => "project_snapshot",
            Self::AttackSurface => "attack_surface",
            Self::ActiveHypotheses => "active_hypotheses",
            Self::RecentExperiments => "recent_experiments",
            Self::Findings => "findings",
            Self::FailedPaths => "failed_paths",
            Self::ActiveJobs => "active_jobs",
            Self::NextActions => "next_actions",
        }
    }
}

/// A canonical structured Section and its content identity.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct WorldStateSectionSnapshot {
    pub id: ResearchWorldStateSection,
    pub value: Value,
    pub snapshot_hash: String,
}

impl WorldStateSectionSnapshot {
    fn new(id: ResearchWorldStateSection, value: Value) -> Result<Self, WorldStateError> {
        let value = canonical_json(&value);
        let snapshot_hash = canonical_blake3_hash(&value).map_err(WorldStateError::encoding)?;
        Ok(Self {
            id,
            value,
            snapshot_hash,
        })
    }
}

/// A complete derived view of the fixed research section set.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ResearchWorldState {
    sections: BTreeMap<ResearchWorldStateSection, WorldStateSectionSnapshot>,
    full_fragment: ContextFragment,
}

impl Default for ResearchWorldState {
    fn default() -> Self {
        Self::from_sections(std::iter::empty()).expect("empty WorldState values are encodable")
    }
}

impl ResearchWorldState {
    /// Fills every fixed section. Unprovided sections are explicit empty
    /// arrays, so absent and empty display state is deterministic.
    pub fn from_sections(
        values: impl IntoIterator<Item = (ResearchWorldStateSection, Value)>,
    ) -> Result<Self, WorldStateError> {
        let mut supplied = BTreeMap::new();
        for (id, value) in values {
            if supplied.insert(id, value).is_some() {
                return Err(WorldStateError::DuplicateSection { id });
            }
        }
        let sections = ResearchWorldStateSection::ALL
            .into_iter()
            .map(|id| {
                WorldStateSectionSnapshot::new(
                    id,
                    supplied
                        .get(&id)
                        .cloned()
                        .unwrap_or_else(|| Value::Array(Vec::new())),
                )
                .map(|snapshot| (id, snapshot))
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        let state = Self {
            sections,
            full_fragment: context_fragment(
                "research-world-state:full",
                ContextFragmentSource::ResearchWorldState,
                String::new(),
            )?,
        };
        let full_fragment = context_fragment(
            "research-world-state:full",
            ContextFragmentSource::ResearchWorldState,
            render_full(&state, MAX_SECTION_RENDERED_BYTES),
        )?;
        Ok(Self {
            full_fragment,
            ..state
        })
    }

    /// Projects the current typed Research facts without treating an absent
    /// record class as an observed empty fact. The latest TargetSnapshot is
    /// the sole current-target identity; every target-scoped record is
    /// filtered through its typed relationship to that snapshot.
    pub fn from_research_state(state: &ResearchState) -> Result<Self, WorldStateError> {
        let Some(current_target) = state.current_target_snapshot() else {
            return Self::from_sections(unknown_current_target_sections());
        };
        let current_target_id = &current_target.id;
        let active_hypotheses = state
            .hypothesis_records()
            .filter_map(|record| {
                if !matches!(
                    record.status,
                    HypothesisStatus::Active | HypothesisStatus::Testing
                ) || record.merged_into.is_some()
                {
                    return None;
                }
                let target_applicability = match state.research_reference_target_applicability(
                    &ResearchRecordRef::Hypothesis(record.id.clone()),
                    current_target_id,
                ) {
                    ResearchReferenceTargetApplicability::Applicable => "exact_snapshot",
                    // Hypotheses bind only to a target identity. They are useful
                    // current context, but never assert exact-Snapshot proof.
                    ResearchReferenceTargetApplicability::Unknown => "target_id_only",
                    ResearchReferenceTargetApplicability::Incompatible
                    | ResearchReferenceTargetApplicability::MissingReference => return None,
                };
                Some(json!({
                    "record": record,
                    "target_applicability": target_applicability,
                }))
            })
            .collect::<Vec<_>>();
        let experiments = state
            .retrieval_experiment_records()
            .filter(|record| {
                matches!(
                    state.research_reference_target_applicability(
                        &ResearchRecordRef::Experiment(record.id.clone()),
                        current_target_id,
                    ),
                    ResearchReferenceTargetApplicability::Applicable
                )
            })
            .map(|record| serde_json::to_value(record).map_err(WorldStateError::encoding))
            .collect::<Result<Vec<_>, _>>()?;
        let findings = state
            .finding_records()
            .filter(|record| {
                record.status != FindingStatus::Discarded
                    && record.merged_into.is_none()
                    && matches!(
                        state.research_reference_target_applicability(
                            &ResearchRecordRef::Finding(record.id.clone()),
                            current_target_id,
                        ),
                        ResearchReferenceTargetApplicability::Applicable
                    )
            })
            .map(|record| serde_json::to_value(record).map_err(WorldStateError::encoding))
            .collect::<Result<Vec<_>, _>>()?;
        let rejected_hypotheses = state
            .hypothesis_records()
            .filter_map(|record| {
                if record.status != HypothesisStatus::Rejected || record.merged_into.is_some() {
                    return None;
                }
                let target_applicability = match state.research_reference_target_applicability(
                    &ResearchRecordRef::Hypothesis(record.id.clone()),
                    current_target_id,
                ) {
                    ResearchReferenceTargetApplicability::Applicable => "exact_snapshot",
                    ResearchReferenceTargetApplicability::Unknown => "target_id_only",
                    ResearchReferenceTargetApplicability::Incompatible
                    | ResearchReferenceTargetApplicability::MissingReference => return None,
                };
                Some(json!({
                    "record": record,
                    "target_applicability": target_applicability,
                }))
            })
            .collect::<Vec<_>>();
        let negative_experiments = state
            .retrieval_experiment_records()
            .filter(|record| {
                record.result == ExperimentResult::Negative
                    && matches!(
                        state.research_reference_target_applicability(
                            &ResearchRecordRef::Experiment(record.id.clone()),
                            current_target_id,
                        ),
                        ResearchReferenceTargetApplicability::Applicable
                    )
            })
            .map(|record| serde_json::to_value(record).map_err(WorldStateError::encoding))
            .collect::<Result<Vec<_>, _>>()?;
        let next_actions = state
            .retrieval_next_action_records()
            .filter(|record| {
                matches!(
                    record.status,
                    NextActionStatus::Pending | NextActionStatus::Running
                ) && matches!(
                    state.research_reference_target_applicability(
                        &record.research_id,
                        current_target_id,
                    ),
                    ResearchReferenceTargetApplicability::Applicable
                        | ResearchReferenceTargetApplicability::Unknown
                )
            })
            .map(|record| {
                let applicability = match state
                    .research_reference_target_applicability(&record.research_id, current_target_id)
                {
                    ResearchReferenceTargetApplicability::Applicable => "exact_snapshot",
                    ResearchReferenceTargetApplicability::Unknown => "target_or_version_only",
                    ResearchReferenceTargetApplicability::Incompatible
                    | ResearchReferenceTargetApplicability::MissingReference => {
                        unreachable!("the filter retains only current-target next actions")
                    }
                };
                json!({"record": record, "target_applicability": applicability})
            })
            .collect::<Vec<_>>();
        let attack_surface = active_hypotheses
            .iter()
            .filter_map(|entry| {
                entry
                    .get("record")?
                    .get("attack_surface")?
                    .as_str()
                    .map(str::to_owned)
            })
            .collect::<std::collections::BTreeSet<_>>();

        Self::from_sections([
            (
                ResearchWorldStateSection::Target,
                target_projection(current_target),
            ),
            (
                ResearchWorldStateSection::TargetSnapshot,
                snapshot_projection(current_target)?,
            ),
            (
                ResearchWorldStateSection::TargetDelta,
                unknown("target_delta_projection_not_available"),
            ),
            (
                ResearchWorldStateSection::Environment,
                environment_projection(current_target),
            ),
            (
                ResearchWorldStateSection::ProjectSnapshot,
                unknown("project_snapshot_is_composed_separately"),
            ),
            (
                ResearchWorldStateSection::AttackSurface,
                json!({"state": "observed", "values": attack_surface}),
            ),
            (
                ResearchWorldStateSection::ActiveHypotheses,
                json!({"state": "observed", "target_snapshot_id": current_target_id, "records": active_hypotheses}),
            ),
            (
                ResearchWorldStateSection::RecentExperiments,
                json!({
                    "state": "observed",
                    "target_snapshot_id": current_target_id,
                    "recency": "not_modeled; records are exact-current-target experiments",
                    "records": experiments,
                }),
            ),
            (
                ResearchWorldStateSection::Findings,
                json!({"state": "observed", "target_snapshot_id": current_target_id, "records": findings}),
            ),
            (
                ResearchWorldStateSection::FailedPaths,
                json!({
                    "state": "observed",
                    "target_snapshot_id": current_target_id,
                    "rejected_hypotheses": rejected_hypotheses,
                    "negative_experiments": negative_experiments,
                }),
            ),
            (
                ResearchWorldStateSection::ActiveJobs,
                unknown("research_ledger_has_no_typed_active_job_projection"),
            ),
            (
                ResearchWorldStateSection::NextActions,
                json!({"state": "observed", "target_snapshot_id": current_target_id, "records": next_actions}),
            ),
        ])
    }

    pub fn sections(&self) -> impl Iterator<Item = &WorldStateSectionSnapshot> {
        ResearchWorldStateSection::ALL
            .into_iter()
            .filter_map(|id| self.sections.get(&id))
    }

    pub fn section(&self, id: ResearchWorldStateSection) -> &WorldStateSectionSnapshot {
        // Construction always fills the fixed set.
        &self.sections[&id]
    }

    pub fn full_fragment(&self) -> Result<ContextFragment, WorldStateError> {
        Ok(self.full_fragment.clone())
    }

    pub(crate) fn full_fragment_with_limit(
        &self,
        maximum_bytes: usize,
    ) -> Result<ContextFragment, WorldStateError> {
        if maximum_bytes == MAX_SECTION_RENDERED_BYTES {
            return self.full_fragment();
        }
        context_fragment(
            "research-world-state:full",
            ContextFragmentSource::ResearchWorldState,
            render_full(self, maximum_bytes),
        )
    }

    /// Renders only changed fixed Sections. An unchanged state creates no
    /// fragment rather than injecting an ambiguous no-op Context item.
    pub fn diff_fragment(
        &self,
        previous: &ResearchWorldState,
    ) -> Result<Option<ContextFragment>, WorldStateError> {
        self.diff_fragment_with_limit(previous, MAX_SECTION_RENDERED_BYTES)
    }

    pub(crate) fn diff_fragment_with_limit(
        &self,
        previous: &ResearchWorldState,
        maximum_bytes: usize,
    ) -> Result<Option<ContextFragment>, WorldStateError> {
        let changed = self
            .sections()
            .filter_map(|current| {
                let prior = previous.section(current.id);
                (prior.snapshot_hash != current.snapshot_hash).then(|| {
                    json!({
                        "section_id": current.id.id(),
                        "previous_hash": prior.snapshot_hash,
                        "current_hash": current.snapshot_hash,
                        "snapshot": bounded_value(&current.value, maximum_bytes),
                    })
                })
            })
            .collect::<Vec<_>>();
        if changed.is_empty() {
            return Ok(None);
        }
        context_fragment(
            "research-world-state:diff",
            ContextFragmentSource::ResearchWorldState,
            json!({"kind": "research_world_state_diff", "sections": changed}).to_string(),
        )
        .map(Some)
    }
}

/// The Project Snapshot equivalent for the fixed sixth Context slot.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ProjectSnapshotContext {
    snapshot: ProjectSnapshot,
    snapshot_hash: String,
    full_fragment: ContextFragment,
}

impl ProjectSnapshotContext {
    pub(crate) fn snapshot(&self) -> &ProjectSnapshot {
        &self.snapshot
    }

    pub fn new(snapshot: ProjectSnapshot) -> Result<Self, WorldStateError> {
        let semantic_snapshot = ProjectSnapshotSemanticHash::from(&snapshot);
        let snapshot_hash =
            canonical_blake3_hash(&semantic_snapshot).map_err(WorldStateError::encoding)?;
        let full_fragment =
            project_full_fragment(&snapshot, &snapshot_hash, MAX_SECTION_RENDERED_BYTES)?;
        Ok(Self {
            snapshot,
            snapshot_hash,
            full_fragment,
        })
    }

    pub fn snapshot_hash(&self) -> &str {
        &self.snapshot_hash
    }

    pub fn full_fragment(&self) -> Result<ContextFragment, WorldStateError> {
        Ok(self.full_fragment.clone())
    }

    pub(crate) fn full_fragment_with_limit(
        &self,
        maximum_bytes: usize,
    ) -> Result<ContextFragment, WorldStateError> {
        if maximum_bytes == MAX_SECTION_RENDERED_BYTES {
            return self.full_fragment();
        }
        project_full_fragment(&self.snapshot, &self.snapshot_hash, maximum_bytes)
    }

    pub fn diff_fragment(
        &self,
        previous: &ProjectSnapshotContext,
    ) -> Result<Option<ContextFragment>, WorldStateError> {
        self.diff_fragment_with_limit(previous, MAX_SECTION_RENDERED_BYTES)
    }

    pub(crate) fn diff_fragment_with_limit(
        &self,
        previous: &ProjectSnapshotContext,
        maximum_bytes: usize,
    ) -> Result<Option<ContextFragment>, WorldStateError> {
        if self.snapshot_hash == previous.snapshot_hash {
            return Ok(None);
        }
        let semantic_snapshot = ProjectSnapshotSemanticHash::from(&self.snapshot);
        context_fragment(
            "project-snapshot:diff",
            ContextFragmentSource::ProjectSnapshot,
            json!({
                "kind": "project_snapshot_diff",
                "previous_hash": previous.snapshot_hash,
                "current_hash": self.snapshot_hash,
                "snapshot": bounded_value(&serde_json::to_value(&semantic_snapshot).map_err(WorldStateError::encoding)?, maximum_bytes),
            })
            .to_string(),
        )
        .map(Some)
    }
}

const MAX_SECTION_RENDERED_BYTES: usize = 384;

fn project_full_fragment(
    snapshot: &ProjectSnapshot,
    snapshot_hash: &str,
    maximum_bytes: usize,
) -> Result<ContextFragment, WorldStateError> {
    let semantic_snapshot = ProjectSnapshotSemanticHash::from(snapshot);
    context_fragment("project-snapshot:full", ContextFragmentSource::ProjectSnapshot, json!({
        "kind": "project_snapshot_full", "snapshot_hash": snapshot_hash,
        "snapshot": bounded_value(&serde_json::to_value(&semantic_snapshot).map_err(WorldStateError::encoding)?, maximum_bytes),
    }).to_string())
}

#[derive(Serialize)]
struct ProjectSnapshotSemanticHash<'a> {
    cache_schema_version: u16,
    project_id: &'a crate::domain::project::ProjectId,
    source_fingerprint: &'a crate::domain::snapshot::ProjectFingerprint,
    file_manifest_hash: &'a str,
    tool_capability_hash: &'a str,
    file_count: usize,
}

impl<'a> From<&'a ProjectSnapshot> for ProjectSnapshotSemanticHash<'a> {
    fn from(snapshot: &'a ProjectSnapshot) -> Self {
        Self {
            cache_schema_version: snapshot.cache_schema_version,
            project_id: &snapshot.project_id,
            source_fingerprint: &snapshot.source_fingerprint,
            file_manifest_hash: &snapshot.file_manifest_hash,
            tool_capability_hash: &snapshot.tool_capability_hash,
            file_count: snapshot.file_count,
        }
    }
}

fn unknown(reason: &'static str) -> Value {
    json!({"state": "unknown", "reason": reason})
}

fn unknown_current_target_sections() -> impl IntoIterator<Item = (ResearchWorldStateSection, Value)>
{
    ResearchWorldStateSection::ALL
        .into_iter()
        .map(|section| (section, unknown("no_current_target_snapshot_recorded")))
}

fn target_projection(snapshot: &TargetSnapshotRecord) -> Value {
    json!({
        "state": "observed",
        "target_id": snapshot.target_id,
        "kind": snapshot.kind,
        "locator": snapshot.locator,
        "version": snapshot.version,
        "build_id": snapshot.build_id,
    })
}

fn snapshot_projection(snapshot: &TargetSnapshotRecord) -> Result<Value, WorldStateError> {
    Ok(json!({
        "state": "observed",
        "record": serde_json::to_value(snapshot).map_err(WorldStateError::encoding)?,
    }))
}

fn environment_projection(snapshot: &TargetSnapshotRecord) -> Value {
    json!({
        "state": "observed",
        "target_snapshot_id": snapshot.id,
        "environment_fingerprint": snapshot.environment_fingerprint,
        "device_fingerprint": snapshot.device_fingerprint,
    })
}

fn render_full(state: &ResearchWorldState, maximum_bytes: usize) -> String {
    json!({
        "kind": "research_world_state_full",
        "sections": state
            .sections()
            .map(|section| json!({
                "section_id": section.id.id(),
                "snapshot_hash": section.snapshot_hash,
                "snapshot": bounded_value(&section.value, maximum_bytes),
            }))
            .collect::<Vec<_>>(),
    })
    .to_string()
}

fn bounded_value(value: &Value, maximum_bytes: usize) -> Value {
    let canonical = canonical_json(value);
    let rendered = canonical.to_string();
    if rendered.len() <= maximum_bytes {
        return canonical;
    }
    let cut = rendered
        .char_indices()
        .map(|(index, character)| index + character.len_utf8())
        .take_while(|end| *end <= maximum_bytes)
        .last()
        .unwrap_or(0);
    json!({
        "truncated": true,
        "rendered_prefix": &rendered[..cut],
        "rendered_bytes": rendered.len(),
    })
}

fn context_fragment(
    id: &str,
    source: ContextFragmentSource,
    text: String,
) -> Result<ContextFragment, WorldStateError> {
    ContextFragment::new(
        ContextFragmentId::new(id).map_err(WorldStateError::Context)?,
        source,
        ContextFragmentPriority::High,
        FragmentCachePolicy::ContentAddressed,
        ContextFragmentContent::ContentItems {
            items: vec![ContentItem::Text {
                role: MessageRole::User,
                text,
            }],
        },
    )
    .map_err(WorldStateError::Context)
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum WorldStateError {
    #[error("ResearchWorldState Section `{id:?}` was supplied more than once")]
    DuplicateSection { id: ResearchWorldStateSection },
    #[error("failed to encode WorldState: {message}")]
    Encoding { message: String },
    #[error(transparent)]
    Context(crate::domain::context::ContextPlannerError),
}

impl WorldStateError {
    fn encoding(error: serde_json::Error) -> Self {
        Self::Encoding {
            message: error.to_string(),
        }
    }
}
