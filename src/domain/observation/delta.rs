//! Deterministic AndroidManifest attack-surface comparison.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::domain::artifacts::ArtifactId;
use crate::domain::events::EventId;
use crate::domain::research::TargetSnapshotId;
use crate::domain::tools::ToolCallId;

use super::{
    AndroidComponentType, AndroidIntentFilter, AndroidManifestComponent, ExtractorIdentity,
    ObservationBatch, ObservationEnvelope, ObservationError, ObservationId, ObservationKind,
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct ValidatedObservationSet {
    target_id: String,
    target_snapshot_id: TargetSnapshotId,
    extractor: ExtractorIdentity,
    observations: Vec<ObservationEnvelope>,
}

impl ValidatedObservationSet {
    pub(crate) fn from_validated_batch(batch: ObservationBatch, target_id: String) -> Self {
        Self {
            target_id,
            target_snapshot_id: batch.target_snapshot_id().clone(),
            extractor: batch.extractor(),
            observations: batch.observations,
        }
    }

    fn components(&self) -> BTreeMap<&str, &ObservationEnvelope> {
        self.observations
            .iter()
            .map(|observation| (observation.subject.as_str(), observation))
            .collect()
    }

    fn source_provenance(&self) -> ObservationSetProvenance {
        let first = &self.observations[0];
        ObservationSetProvenance {
            target_snapshot_id: self.target_snapshot_id.clone(),
            raw_artifact_id: first.raw_artifact_id.clone(),
            produced_by_tool_call_id: first.produced_by_tool_call_id.clone(),
            execution_receipt_event_id: first.execution_receipt_event_id.clone(),
            extractor: self.extractor.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct ObservationSetProvenance {
    target_snapshot_id: TargetSnapshotId,
    raw_artifact_id: ArtifactId,
    produced_by_tool_call_id: ToolCallId,
    execution_receipt_event_id: EventId,
    extractor: ExtractorIdentity,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct TargetDelta {
    baseline_source: ObservationSetProvenance,
    candidate_source: ObservationSetProvenance,
    changes: Vec<TargetDeltaEntry>,
}

impl TargetDelta {
    pub(crate) fn compare(
        baseline: &ValidatedObservationSet,
        candidate: &ValidatedObservationSet,
    ) -> Result<Self, ObservationError> {
        if baseline.target_id != candidate.target_id {
            return Err(ObservationError::TargetIdentityMismatch {
                baseline: baseline.target_id.clone(),
                candidate: candidate.target_id.clone(),
            });
        }
        if baseline.target_snapshot_id == candidate.target_snapshot_id {
            return Err(ObservationError::Invalid {
                message: "TargetDelta requires two distinct TargetSnapshot identities".to_owned(),
            });
        }
        if baseline.extractor != candidate.extractor {
            return Err(ObservationError::ExtractorMismatch {
                baseline_id: baseline.extractor.id.clone(),
                baseline_version: baseline.extractor.version.clone(),
                candidate_id: candidate.extractor.id.clone(),
                candidate_version: candidate.extractor.version.clone(),
            });
        }

        let before = baseline.components();
        let after = candidate.components();
        let mut changes = Vec::new();
        let components = before
            .keys()
            .chain(after.keys())
            .copied()
            .collect::<BTreeSet<_>>();
        for component in components {
            match (before.get(component), after.get(component)) {
                (None, Some(after)) => changes.push(TargetDeltaEntry::component_added(after)),
                (Some(before), None) => changes.push(TargetDeltaEntry::component_removed(before)),
                (Some(before), Some(after)) => compare_component(before, after, &mut changes),
                (None, None) => unreachable!("component is selected from one source map"),
            }
        }
        changes.sort_by_key(TargetDeltaEntry::sort_key);

        Ok(Self {
            baseline_source: baseline.source_provenance(),
            candidate_source: candidate.source_provenance(),
            changes,
        })
    }
}

impl ObservationEnvelope {
    fn manifest_component(&self) -> &AndroidManifestComponent {
        match &self.kind {
            ObservationKind::AndroidManifestComponent(details) => details,
        }
    }

    fn provenance(&self) -> ObservationProvenance {
        ObservationProvenance {
            observation_id: self.observation_id.clone(),
            target_snapshot_id: self.target_snapshot_id.clone(),
            raw_artifact_id: self.raw_artifact_id.clone(),
            produced_by_tool_call_id: self.produced_by_tool_call_id.clone(),
            execution_receipt_event_id: self.execution_receipt_event_id.clone(),
            extractor: ExtractorIdentity {
                id: self.extractor_id.clone(),
                version: self.extractor_version.clone(),
            },
        }
    }
}

fn compare_component(
    before: &ObservationEnvelope,
    after: &ObservationEnvelope,
    changes: &mut Vec<TargetDeltaEntry>,
) {
    let before_component = before.manifest_component();
    let after_component = after.manifest_component();
    if before_component.component_type != after_component.component_type {
        changes.push(TargetDeltaEntry::component_type_changed(before, after));
    }
    if before_component.exported != after_component.exported {
        changes.push(TargetDeltaEntry::exported_changed(before, after));
    }
    if before_component.permission != after_component.permission {
        changes.push(TargetDeltaEntry::permission_changed(before, after));
    }

    let before_filters = before_component
        .intent_filters
        .iter()
        .map(|filter| (filter.stable_key(), filter))
        .collect::<BTreeMap<_, _>>();
    let after_filters = after_component
        .intent_filters
        .iter()
        .map(|filter| (filter.stable_key(), filter))
        .collect::<BTreeMap<_, _>>();
    for key in before_filters
        .keys()
        .chain(after_filters.keys())
        .collect::<BTreeSet<_>>()
    {
        match (before_filters.get(key), after_filters.get(key)) {
            (None, Some(filter)) => {
                changes.push(TargetDeltaEntry::intent_filter_added(before, after, filter))
            }
            (Some(filter), None) => changes.push(TargetDeltaEntry::intent_filter_removed(
                before, after, filter,
            )),
            (Some(_), Some(_)) => {}
            (None, None) => unreachable!("filter is selected from one source map"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
enum TargetDeltaChangeKind {
    ComponentAdded,
    ComponentRemoved,
    ComponentTypeChanged,
    ExportedChanged,
    PermissionChanged,
    IntentFilterAdded,
    IntentFilterRemoved,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct TargetDeltaEntry {
    kind: TargetDeltaChangeKind,
    component: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    before_component_type: Option<AndroidComponentType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    after_component_type: Option<AndroidComponentType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    before_exported: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    after_exported: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    before_permission: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    after_permission: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    before_manifest_component: Option<AndroidManifestComponent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    after_manifest_component: Option<AndroidManifestComponent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    intent_filter: Option<AndroidIntentFilter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    before_provenance: Option<ObservationProvenance>,
    #[serde(skip_serializing_if = "Option::is_none")]
    after_provenance: Option<ObservationProvenance>,
}

impl TargetDeltaEntry {
    fn component_added(after: &ObservationEnvelope) -> Self {
        let mut entry = Self::base(
            TargetDeltaChangeKind::ComponentAdded,
            after.subject.clone(),
            None,
            Some(after),
        );
        entry.after_manifest_component = Some(after.manifest_component().clone());
        entry
    }

    fn component_removed(before: &ObservationEnvelope) -> Self {
        let mut entry = Self::base(
            TargetDeltaChangeKind::ComponentRemoved,
            before.subject.clone(),
            Some(before),
            None,
        );
        entry.before_manifest_component = Some(before.manifest_component().clone());
        entry
    }

    fn component_type_changed(before: &ObservationEnvelope, after: &ObservationEnvelope) -> Self {
        let mut entry = Self::base(
            TargetDeltaChangeKind::ComponentTypeChanged,
            before.subject.clone(),
            Some(before),
            Some(after),
        );
        entry.before_component_type = Some(before.manifest_component().component_type);
        entry.after_component_type = Some(after.manifest_component().component_type);
        entry
    }

    fn exported_changed(before: &ObservationEnvelope, after: &ObservationEnvelope) -> Self {
        let mut entry = Self::base(
            TargetDeltaChangeKind::ExportedChanged,
            before.subject.clone(),
            Some(before),
            Some(after),
        );
        entry.before_exported = Some(before.manifest_component().exported);
        entry.after_exported = Some(after.manifest_component().exported);
        entry
    }

    fn permission_changed(before: &ObservationEnvelope, after: &ObservationEnvelope) -> Self {
        let mut entry = Self::base(
            TargetDeltaChangeKind::PermissionChanged,
            before.subject.clone(),
            Some(before),
            Some(after),
        );
        entry.before_permission = before.manifest_component().permission.clone();
        entry.after_permission = after.manifest_component().permission.clone();
        entry
    }

    fn intent_filter_added(
        before: &ObservationEnvelope,
        after: &ObservationEnvelope,
        filter: &AndroidIntentFilter,
    ) -> Self {
        let mut entry = Self::base(
            TargetDeltaChangeKind::IntentFilterAdded,
            after.subject.clone(),
            Some(before),
            Some(after),
        );
        entry.intent_filter = Some(filter.clone());
        entry
    }

    fn intent_filter_removed(
        before: &ObservationEnvelope,
        after: &ObservationEnvelope,
        filter: &AndroidIntentFilter,
    ) -> Self {
        let mut entry = Self::base(
            TargetDeltaChangeKind::IntentFilterRemoved,
            before.subject.clone(),
            Some(before),
            Some(after),
        );
        entry.intent_filter = Some(filter.clone());
        entry
    }

    fn base(
        kind: TargetDeltaChangeKind,
        component: String,
        before: Option<&ObservationEnvelope>,
        after: Option<&ObservationEnvelope>,
    ) -> Self {
        Self {
            kind,
            component,
            before_component_type: None,
            after_component_type: None,
            before_exported: None,
            after_exported: None,
            before_permission: None,
            after_permission: None,
            before_manifest_component: None,
            after_manifest_component: None,
            intent_filter: None,
            before_provenance: before.map(ObservationEnvelope::provenance),
            after_provenance: after.map(ObservationEnvelope::provenance),
        }
    }

    fn sort_key(&self) -> (String, TargetDeltaChangeKind, String) {
        let filter_key = self
            .intent_filter
            .as_ref()
            .map(AndroidIntentFilter::stable_key)
            .unwrap_or_default();
        (self.component.clone(), self.kind, filter_key)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct ObservationProvenance {
    observation_id: ObservationId,
    target_snapshot_id: TargetSnapshotId,
    raw_artifact_id: ArtifactId,
    produced_by_tool_call_id: ToolCallId,
    execution_receipt_event_id: EventId,
    extractor: ExtractorIdentity,
}
