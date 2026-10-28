//! Frozen common context for independently replaying compaction requests.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{SessionError, task::invalid_task};

/// Frozen inputs that cannot necessarily be reconstructed from Session facts.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CompactionContextCapture {
    schema_version: u16,
    pub(crate) capabilities: crate::domain::model::ModelCapabilities,
    pub(crate) policy: crate::domain::context::ContextBudgetPolicy,
    pub(crate) reference_limit: usize,
    fragments: Vec<CapturedFragment>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CapturedFragment {
    id: crate::domain::context::ContextFragmentId,
    source: crate::domain::context::ContextFragmentSource,
    priority: crate::domain::context::ContextFragmentPriority,
    cache_policy: crate::domain::context::FragmentCachePolicy,
    content: crate::domain::context::ContextFragmentContent,
    artifact_reference: Option<crate::domain::artifacts::ArtifactId>,
    provenance: crate::domain::context::ContextProvenance,
    item_provenance: BTreeMap<u32, crate::domain::context::ContextProvenance>,
}

impl CompactionContextCapture {
    pub(crate) fn new(
        capabilities: crate::domain::model::ModelCapabilities,
        policy: crate::domain::context::ContextBudgetPolicy,
        reference_limit: usize,
        fragments: Vec<crate::domain::context::ContextFragment>,
    ) -> Self {
        let fragments = fragments
            .into_iter()
            .map(|fragment| {
                let count = match fragment.content() {
                    crate::domain::context::ContextFragmentContent::ContentItems { items } => {
                        items.len()
                    }
                    _ => 0,
                };
                let item_provenance = (0..count)
                    .filter_map(|ordinal| {
                        let ordinal =
                            u32::try_from(ordinal).expect("Context item ordinal fits u32");
                        fragment
                            .content_item_provenance(ordinal)
                            .cloned()
                            .map(|p| (ordinal, p))
                    })
                    .collect();
                CapturedFragment {
                    id: fragment.id().clone(),
                    source: fragment.source(),
                    priority: fragment.priority(),
                    cache_policy: fragment.cache_policy(),
                    content: fragment.content().clone(),
                    artifact_reference: fragment.artifact_reference().cloned(),
                    provenance: fragment.provenance().clone(),
                    item_provenance,
                }
            })
            .collect();
        Self {
            schema_version: 1,
            capabilities,
            policy,
            reference_limit,
            fragments,
        }
    }

    pub(crate) fn fragments(
        &self,
    ) -> Result<Vec<crate::domain::context::ContextFragment>, SessionError> {
        use crate::domain::context::{ContextFragment, ContextFragmentSource};
        if self.schema_version != 1 || self.reference_limit == 0 || self.policy.validate().is_err()
        {
            return Err(invalid_task("invalid Compaction context capture"));
        }
        self.fragments
            .iter()
            .map(|saved| {
                if !matches!(
                    saved.source,
                    ContextFragmentSource::ResearchWorldState
                        | ContextFragmentSource::ProjectSnapshot
                        | ContextFragmentSource::ProjectMemoryCore
                ) {
                    return Err(invalid_task("invalid Compaction common fragment source"));
                }
                let mut fragment = ContextFragment::with_artifact_reference(
                    saved.id.clone(),
                    saved.source,
                    saved.priority,
                    saved.cache_policy,
                    saved.content.clone(),
                    saved.artifact_reference.clone(),
                )
                .and_then(|fragment| fragment.with_provenance(saved.provenance.clone()))
                .map_err(|error| invalid_task(error.to_string()))?;
                for (&ordinal, provenance) in &saved.item_provenance {
                    fragment = fragment
                        .with_content_item_provenance(ordinal, provenance.clone())
                        .map_err(|error| invalid_task(error.to_string()))?;
                }
                Ok(fragment)
            })
            .collect()
    }
}
