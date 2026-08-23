//! Bounded, derived Research and Project snapshot Context renderers.
//!
//! Research JSONL and the project-inspection cache remain authoritative. This
//! module only turns their already-derived values into deterministic Context
//! Fragments; it never writes facts or decides Provider delivery.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use thiserror::Error;

use crate::domain::canonical_json::{canonical_blake3_hash, canonical_json};
use crate::domain::context::{
    ContextFragment, ContextFragmentContent, ContextFragmentId, ContextFragmentPriority,
    ContextFragmentSource, FragmentCachePolicy,
};
use crate::domain::model::{ContentItem, MessageRole};
use crate::domain::research::ResearchState;
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
            render_full(&state),
        )?;
        Ok(Self {
            full_fragment,
            ..state
        })
    }

    /// Derives only facts whose current status is expressible without adding
    /// a second Research query API. In particular, this never labels every
    /// stored hypothesis as active or every experiment as recent.
    pub fn from_research_state(state: &ResearchState) -> Result<Self, WorldStateError> {
        let serialized = serde_json::to_value(state).map_err(WorldStateError::encoding)?;
        let object = serialized.as_object().cloned().unwrap_or_default();
        Self::from_sections([
            (ResearchWorldStateSection::Target, Value::Array(Vec::new())),
            (
                ResearchWorldStateSection::TargetSnapshot,
                Value::Array(Vec::new()),
            ),
            (
                ResearchWorldStateSection::TargetDelta,
                Value::Array(Vec::new()),
            ),
            (
                ResearchWorldStateSection::Environment,
                Value::Array(Vec::new()),
            ),
            (
                ResearchWorldStateSection::ProjectSnapshot,
                Value::Array(Vec::new()),
            ),
            (
                ResearchWorldStateSection::AttackSurface,
                Value::Array(Vec::new()),
            ),
            (
                ResearchWorldStateSection::ActiveHypotheses,
                records_with_status(&object, "hypotheses", &["active", "testing"]),
            ),
            (
                ResearchWorldStateSection::RecentExperiments,
                Value::Array(Vec::new()),
            ),
            (
                ResearchWorldStateSection::Findings,
                records_without_status(&object, "findings", "discarded"),
            ),
            (
                ResearchWorldStateSection::FailedPaths,
                Value::Array(Vec::new()),
            ),
            (
                ResearchWorldStateSection::ActiveJobs,
                Value::Array(Vec::new()),
            ),
            (
                ResearchWorldStateSection::NextActions,
                records_with_status(&object, "next_actions", &["pending", "running"]),
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

    /// Renders only changed fixed Sections. An unchanged state creates no
    /// fragment rather than injecting an ambiguous no-op Context item.
    pub fn diff_fragment(
        &self,
        previous: &ResearchWorldState,
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
                        "snapshot": bounded_value(&current.value),
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
    pub fn new(snapshot: ProjectSnapshot) -> Result<Self, WorldStateError> {
        let semantic_snapshot = ProjectSnapshotSemanticHash::from(&snapshot);
        let snapshot_hash =
            canonical_blake3_hash(&semantic_snapshot).map_err(WorldStateError::encoding)?;
        let full_fragment = context_fragment(
            "project-snapshot:full",
            ContextFragmentSource::ProjectSnapshot,
            json!({
                "kind": "project_snapshot_full",
                "snapshot_hash": snapshot_hash,
                "snapshot": bounded_value(&serde_json::to_value(&semantic_snapshot).map_err(WorldStateError::encoding)?),
            })
            .to_string(),
        )?;
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

    pub fn diff_fragment(
        &self,
        previous: &ProjectSnapshotContext,
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
                "snapshot": bounded_value(&serde_json::to_value(&semantic_snapshot).map_err(WorldStateError::encoding)?),
            })
            .to_string(),
        )
        .map(Some)
    }
}

const MAX_SECTION_RENDERED_BYTES: usize = 384;

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

fn record_values<'a>(object: &'a Map<String, Value>, key: &str) -> impl Iterator<Item = &'a Value> {
    object
        .get(key)
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|records| records.values())
}

fn records_with_status(object: &Map<String, Value>, key: &str, statuses: &[&str]) -> Value {
    Value::Array(
        record_values(object, key)
            .filter(|record| {
                record
                    .get("status")
                    .and_then(Value::as_str)
                    .is_some_and(|status| statuses.contains(&status))
            })
            .cloned()
            .collect(),
    )
}

fn records_without_status(object: &Map<String, Value>, key: &str, excluded: &str) -> Value {
    Value::Array(
        record_values(object, key)
            .filter(|record| record.get("status").and_then(Value::as_str) != Some(excluded))
            .cloned()
            .collect(),
    )
}

fn render_full(state: &ResearchWorldState) -> String {
    json!({
        "kind": "research_world_state_full",
        "sections": state
            .sections()
            .map(|section| json!({
                "section_id": section.id.id(),
                "snapshot_hash": section.snapshot_hash,
                "snapshot": bounded_value(&section.value),
            }))
            .collect::<Vec<_>>(),
    })
    .to_string()
}

fn bounded_value(value: &Value) -> Value {
    let canonical = canonical_json(value);
    let rendered = canonical.to_string();
    if rendered.len() <= MAX_SECTION_RENDERED_BYTES {
        return canonical;
    }
    let cut = rendered
        .char_indices()
        .map(|(index, character)| index + character.len_utf8())
        .take_while(|end| *end <= MAX_SECTION_RENDERED_BYTES)
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
