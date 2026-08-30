//! Pure, provider-neutral Context planning and decision manifests.

mod cache_key;
mod delivery_map;
mod error;
mod estimator;
mod fingerprint;
mod fragment;
mod manifest;
mod render;
mod selection;

/// Estimates audit component bytes with the same deterministic conservative
/// rule used by Context budgeting. Audit inspection does not own a second
/// token formula.
pub(crate) fn estimate_audit_component_tokens(encoded_bytes: usize) -> u32 {
    estimator::estimate_encoded_tokens(encoded_bytes)
}

/// Uses the same canonical Tool-schema encoding and conservative estimator as
/// Context planning. Derived Tool-accuracy reports consume this owner rather
/// than introducing another token formula.
pub(crate) fn tool_schema_token_cost(specs: &[ToolSpec]) -> Result<u32, ContextPlannerError> {
    let canonical_tools =
        canonical_tool_schemas(specs).map_err(|error| ContextPlannerError::CanonicalEncoding {
            message: error.to_string(),
        })?;
    Ok(fixed_input_tokens(&canonical_tools))
}

use serde::Serialize;

use super::canonical::{RequestHash, canonical_tool_schemas};
use crate::domain::model::{ModelCapabilities, ModelInputIR, ModelRequestRequirements};
use crate::domain::tools::ToolSpec;

pub use cache_key::{
    CONTEXT_FRAGMENT_CACHE_KEY_VERSION, ContextFragmentCacheKey, ContextSourceFingerprint,
};
pub use delivery_map::{
    AuthorityLevel, CONTEXT_DELIVERY_MAP_SCHEMA_VERSION, ContextContentDelivery,
    ContextDeliveryMap, ContextInstructionSpan, ContextProvenance, ContextReferences,
    ContextSubitemId, ContextTransform, FreshnessIdentity, RetentionClass, SemanticClass,
};
pub use error::ContextPlannerError;
pub use fragment::{
    ContextContentHash, ContextFragment, ContextFragmentContent, ContextFragmentId,
    ContextFragmentPriority, ContextFragmentSource, ContextOrigin, ContextSlot,
    FragmentCachePolicy,
};
pub use manifest::{
    AppendOnlyCandidate, ContextBudget, ContextDecision, ContextDecisionEntry,
    ContextDecisionManifest, ContextPlan, ContextPlannerVersions, ContextReasonCode,
    FullLogicalManifest, FullLogicalManifestEntry, LogicalAppendOnlyRejection,
    LogicalBaselineRelation,
};
pub use render::PromptRendererIdentity;

use fingerprint::hash;
use selection::{
    append_only_candidate, apply_baseline_relations, ensure_unique_ids, select_decisions,
    sort_full_order,
};

/// The only owner of Context ordering, budget selection and logical-manifest
/// comparison. ProviderSession alone determines actual Provider compatibility.
#[derive(Clone, Debug)]
pub struct ContextPlanner {
    versions: ContextPlannerVersions,
    renderer: PromptRendererIdentity,
}

impl ContextPlanner {
    pub fn current() -> Self {
        Self::new(ContextPlannerVersions::new(
            "context-planner:1",
            "utf8-rendered-channel-budget:2",
            "output-and-tool-loop-eighths:1",
        ))
    }

    pub fn new(versions: ContextPlannerVersions) -> Self {
        Self {
            versions,
            renderer: PromptRendererIdentity::generic_v1(),
        }
    }

    pub fn renderer_identity(&self) -> &PromptRendererIdentity {
        &self.renderer
    }

    pub fn default_budget(
        &self,
        capabilities: &ModelCapabilities,
    ) -> Result<ContextBudget, ContextPlannerError> {
        let reserve = capabilities.context_window / 8;
        ContextBudget::new(capabilities.context_window, reserve, reserve)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn fragment_cache_key(
        &self,
        fragment_id: ContextFragmentId,
        source: ContextFragmentSource,
        priority: ContextFragmentPriority,
        cache_policy: FragmentCachePolicy,
        source_fingerprint: ContextSourceFingerprint,
        content_hash: ContextContentHash,
        capabilities: &ModelCapabilities,
        budget: ContextBudget,
    ) -> Result<ContextFragmentCacheKey, ContextPlannerError> {
        ContextFragmentCacheKey::new(
            fragment_id,
            source,
            priority,
            cache_policy,
            source_fingerprint,
            content_hash,
            &self.versions,
            capabilities,
            budget,
        )
    }

    pub fn compile(
        &self,
        mut fragments: Vec<ContextFragment>,
        budget: ContextBudget,
        capabilities: &ModelCapabilities,
        tool_specs: Vec<ToolSpec>,
        requirements: ModelRequestRequirements,
        prior_manifest: Option<&FullLogicalManifest>,
    ) -> Result<ContextPlan, ContextPlannerError> {
        if budget.context_window_tokens() > capabilities.context_window {
            return Err(ContextPlannerError::BudgetExceedsModelContext {
                requested_tokens: budget.context_window_tokens(),
                capability_tokens: capabilities.context_window,
            });
        }
        let canonical_tools = canonical_tool_schemas(&tool_specs).map_err(|error| {
            ContextPlannerError::CanonicalEncoding {
                message: error.to_string(),
            }
        })?;
        let fixed_input_tokens = fixed_input_tokens(&canonical_tools);
        let requirements =
            requirements.normalized_for_tool_specs(!canonical_tools.ordered_specs.is_empty());
        ensure_unique_ids(&fragments)?;
        sort_full_order(&mut fragments);
        let decisions = select_decisions(&fragments, budget, fixed_input_tokens)?;
        let full_logical_manifest = FullLogicalManifest {
            entries: fragments
                .iter()
                .enumerate()
                .filter(|(index, _)| decisions[*index].decision.is_rendered())
                .map(|(index, fragment)| FullLogicalManifestEntry {
                    fragment_id: fragment.id.clone(),
                    source: fragment.source,
                    slot: fragment.slot,
                    origin: fragment.origin,
                    content_hash: fragment.content_hash.clone(),
                    decision: decisions[index].decision.clone(),
                })
                .collect(),
        };
        let append_only_candidate = append_only_candidate(prior_manifest, &full_logical_manifest);
        let candidates =
            apply_baseline_relations(decisions, &append_only_candidate, &full_logical_manifest);
        // Deliberately exclude prior manifest and baseline relations: they
        // describe comparison history, not current Provider input semantics.
        let fingerprint = hash(&ContextFingerprintInput {
            versions: &self.versions,
            renderer: &self.renderer,
            budget,
            capabilities,
            tool_schema_hash: &canonical_tools.hash,
            fixed_input_tokens,
            requirements,
            full_logical_manifest: &full_logical_manifest,
            decisions: candidates
                .iter()
                .map(ContextDecisionFingerprintEntry::from)
                .collect(),
        })?;
        let rendered = render::render_model_input(
            &fragments,
            &candidates,
            canonical_tools.ordered_specs,
            requirements,
            &self.renderer,
        )?;
        self.validate_rendered_model_input(&rendered.model_input, budget)?;
        Ok(ContextPlan {
            context_fingerprint: fingerprint.clone(),
            prompt_cache_key: hash(&(
                &self.versions.planner_version,
                &self.renderer,
                &full_logical_manifest,
            ))?,
            prompt_renderer: self.renderer.clone(),
            full_logical_manifest,
            append_only_candidate,
            context_decision_manifest: ContextDecisionManifest {
                context_fingerprint: fingerprint,
                budget,
                fixed_input_tokens,
                candidates,
            },
            model_input: rendered.model_input,
            delivery_map: rendered.delivery_map,
        })
    }

    /// Renders one already-selected additive content Fragment through the
    /// same generic-v1 owner used by complete Context compilation.
    pub(crate) fn render_additive_fragment(
        &self,
        fragment: &ContextFragment,
    ) -> Result<
        (
            Vec<crate::domain::model::ContentItem>,
            Vec<ContextContentDelivery>,
        ),
        ContextPlannerError,
    > {
        render::render_included_content_fragment(fragment)
    }

    /// Revalidates a concrete rendered Provider input with the same canonical
    /// Tool-schema and channel estimator used by [`Self::compile`].
    ///
    /// The AgentLoop uses this only when it derives a delivery-only
    /// Full-plus-Diff candidate from a previously compiled Full request. It
    /// cannot authorize a request that exceeds the Planner's budget.
    pub(crate) fn validate_rendered_model_input(
        &self,
        model_input: &ModelInputIR,
        budget: ContextBudget,
    ) -> Result<(), ContextPlannerError> {
        let canonical_tools = canonical_tool_schemas(&model_input.tool_specs).map_err(|error| {
            ContextPlannerError::CanonicalEncoding {
                message: error.to_string(),
            }
        })?;
        let rendered_input_tokens = fixed_input_tokens(&canonical_tools).saturating_add(
            estimator::estimate_rendered_channels_tokens(
                &model_input.instructions,
                &model_input.content_items,
            )?,
        );
        if rendered_input_tokens > budget.input_token_limit() {
            return Err(ContextPlannerError::RenderedInputExceedsBudget {
                rendered_input_tokens,
                input_token_limit: budget.input_token_limit(),
            });
        }
        Ok(())
    }
}

fn fixed_input_tokens(canonical_tools: &super::canonical::CanonicalToolSchemas) -> u32 {
    if canonical_tools.ordered_specs.is_empty() {
        0
    } else {
        estimator::estimate_encoded_tokens(canonical_tools.encoded_len)
    }
}

#[derive(Serialize)]
struct ContextFingerprintInput<'a> {
    versions: &'a ContextPlannerVersions,
    renderer: &'a PromptRendererIdentity,
    budget: ContextBudget,
    capabilities: &'a ModelCapabilities,
    tool_schema_hash: &'a RequestHash,
    fixed_input_tokens: u32,
    requirements: ModelRequestRequirements,
    full_logical_manifest: &'a FullLogicalManifest,
    decisions: Vec<ContextDecisionFingerprintEntry>,
}

#[derive(Serialize)]
struct ContextDecisionFingerprintEntry {
    fragment_id: ContextFragmentId,
    source: ContextFragmentSource,
    content_hash: ContextContentHash,
    token_estimate: u32,
    decision: ContextDecision,
    reason_code: ContextReasonCode,
}

impl From<&ContextDecisionEntry> for ContextDecisionFingerprintEntry {
    fn from(entry: &ContextDecisionEntry) -> Self {
        Self {
            fragment_id: entry.fragment_id.clone(),
            source: entry.source,
            content_hash: entry.content_hash.clone(),
            token_estimate: entry.token_estimate,
            decision: entry.decision.clone(),
            reason_code: entry.reason_code,
        }
    }
}
