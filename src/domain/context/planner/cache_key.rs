//! Cache-key material for derived rendered Context fragments.

use serde::Serialize;

use crate::domain::canonical_json::canonical_blake3_hash;
use crate::domain::model::ModelCapabilities;

use super::error::ContextPlannerError;
use super::fragment::{
    ContextContentHash, ContextFragment, ContextFragmentId, ContextFragmentPriority,
    ContextFragmentSource, FragmentCachePolicy,
};
use super::manifest::{ContextBudget, ContextPlannerVersions};

/// Changes only if cache-key serialization semantics change incompatibly.
pub const CONTEXT_FRAGMENT_CACHE_KEY_VERSION: u16 = 2;

/// A caller-owned source/content fingerprint available before the expensive
/// Context Fragment renderer runs. It is deliberately distinct from the
/// rendered `ContextContentHash` calculated by `ContextFragment`.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ContextSourceFingerprint(String);

impl ContextSourceFingerprint {
    pub fn from_source(source: &impl Serialize) -> Result<Self, ContextPlannerError> {
        canonical_blake3_hash(source).map(Self).map_err(|error| {
            ContextPlannerError::CanonicalEncoding {
                message: error.to_string(),
            }
        })
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A complete pre-render identity for one derived Fragment Cache entry.
/// `ProjectId` intentionally belongs to the project-isolated Adapter entry,
/// not this reusable Domain key.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContextFragmentCacheKey {
    value: String,
    fragment_id: ContextFragmentId,
    source: ContextFragmentSource,
    priority: ContextFragmentPriority,
    cache_policy: FragmentCachePolicy,
    content_hash: ContextContentHash,
}

impl ContextFragmentCacheKey {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        fragment_id: ContextFragmentId,
        source: ContextFragmentSource,
        priority: ContextFragmentPriority,
        cache_policy: FragmentCachePolicy,
        source_fingerprint: ContextSourceFingerprint,
        content_hash: ContextContentHash,
        versions: &ContextPlannerVersions,
        capabilities: &ModelCapabilities,
        budget: ContextBudget,
    ) -> Result<Self, ContextPlannerError> {
        let value = canonical_blake3_hash(&ContextFragmentCacheKeyMaterial {
            cache_key_version: CONTEXT_FRAGMENT_CACHE_KEY_VERSION,
            fragment_id: &fragment_id,
            source,
            priority,
            cache_policy,
            source_fingerprint: &source_fingerprint,
            content_hash: &content_hash,
            planner_version: &versions.planner_version,
            estimator_version: &versions.estimator_version,
            budget_policy_version: &versions.budget_policy_version,
            capabilities,
            budget,
        })
        .map_err(|error| ContextPlannerError::CanonicalEncoding {
            message: error.to_string(),
        })?;
        Ok(Self {
            value,
            fragment_id,
            source,
            priority,
            cache_policy,
            content_hash,
        })
    }

    pub fn as_str(&self) -> &str {
        &self.value
    }

    pub const fn cache_policy(&self) -> FragmentCachePolicy {
        self.cache_policy
    }

    /// Validates the identity fields known before rendering. The reconstructed
    /// fragment's content hash is independently checked by the Adapter.
    pub fn matches_fragment(&self, fragment: &ContextFragment) -> bool {
        self.fragment_id == *fragment.id()
            && self.source == fragment.source()
            && self.priority == fragment.priority()
            && self.cache_policy == fragment.cache_policy()
            && self.content_hash == *fragment.content_hash()
    }
}

#[derive(Serialize)]
struct ContextFragmentCacheKeyMaterial<'a> {
    cache_key_version: u16,
    fragment_id: &'a ContextFragmentId,
    source: ContextFragmentSource,
    priority: ContextFragmentPriority,
    cache_policy: FragmentCachePolicy,
    source_fingerprint: &'a ContextSourceFingerprint,
    content_hash: &'a ContextContentHash,
    planner_version: &'a str,
    estimator_version: &'a str,
    budget_policy_version: &'a str,
    capabilities: &'a ModelCapabilities,
    budget: ContextBudget,
}
