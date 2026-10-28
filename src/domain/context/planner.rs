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

pub(crate) use estimator::ESTIMATED_BYTES_PER_TOKEN;

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
use super::window_policy::ContextBudgetPolicy;
use crate::domain::model::{ModelCapabilities, ModelInputIR, ModelRequestRequirements};
use crate::domain::tools::ToolSpec;

/// Bounded numerical summary of the exact fragments supplied to one failed
/// compilation. These are planner estimates, not Provider token counts.
#[derive(Clone, Debug, Default)]
pub(crate) struct ContextCapacityBreakdown {
    pub(crate) tool_schema_tokens: u32,
    pub(crate) fragment_budget: Option<u32>,
    pub(crate) instructions: u64,
    pub(crate) memory: u64,
    pub(crate) summary: u64,
    pub(crate) input: u64,
    pub(crate) history: u64,
    pub(crate) turn_updates: u64,
    pub(crate) project_facts: u64,
    pub(crate) required_fragments: u64,
    pub(crate) images: usize,
    pub(crate) opaque_items: usize,
}

impl ContextCapacityBreakdown {
    fn from_fragments(fragments: &[ContextFragment], budget: ContextBudget, tools: u32) -> Self {
        let mut result = Self {
            tool_schema_tokens: tools,
            fragment_budget: budget.input_token_limit().checked_sub(tools),
            ..Self::default()
        };
        for fragment in fragments {
            let estimate = u64::from(fragment.token_estimate());
            let bucket = match fragment.source() {
                ContextFragmentSource::BasePrompt
                | ContextFragmentSource::PersonalPreferences
                | ContextFragmentSource::ProjectPrompt
                | ContextFragmentSource::ModePrompt
                | ContextFragmentSource::DomainPrompt
                | ContextFragmentSource::PhasePrompt => &mut result.instructions,
                ContextFragmentSource::ProjectMemoryCore
                | ContextFragmentSource::RetrievedMemory => &mut result.memory,
                ContextFragmentSource::SessionSummary => &mut result.summary,
                ContextFragmentSource::UserInput => &mut result.input,
                ContextFragmentSource::RecentHistory => &mut result.history,
                ContextFragmentSource::TurnLocalUpdates => &mut result.turn_updates,
                ContextFragmentSource::ResearchWorldState
                | ContextFragmentSource::ProjectSnapshot => &mut result.project_facts,
            };
            *bucket = bucket.saturating_add(estimate);
            if fragment.priority() == ContextFragmentPriority::Required {
                result.required_fragments = result.required_fragments.saturating_add(estimate);
            }
            if let ContextFragmentContent::ContentItems { items } = fragment.content() {
                for item in items {
                    match item {
                        crate::domain::model::ContentItem::ImageArtifact { .. } => {
                            result.images += 1
                        }
                        crate::domain::model::ContentItem::OpaqueContinuation { .. } => {
                            result.opaque_items += 1
                        }
                        _ => {}
                    }
                }
            }
        }
        result
    }
}

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
    budget_policy: ContextBudgetPolicy,
}

impl ContextPlanner {
    pub fn current() -> Self {
        Self::new(ContextPlannerVersions::new(
            "context-planner:1",
            "utf8-rendered-channel-budget:2",
            "recoverable-capacity-scoped-handoff:5",
        ))
        .with_budget_policy(ContextBudgetPolicy::default())
    }

    pub fn new(versions: ContextPlannerVersions) -> Self {
        Self {
            versions,
            renderer: PromptRendererIdentity::generic_v1(),
            budget_policy: ContextBudgetPolicy::default(),
        }
    }

    pub(crate) fn with_budget_policy(mut self, policy: ContextBudgetPolicy) -> Self {
        self.budget_policy = policy;
        // Policy participates in every existing fingerprint/cache key through
        // the planner version snapshot; there is still one fingerprint owner.
        self.versions.budget_policy_version = format!(
            "recoverable-capacity-configured:5:{}",
            serde_json::to_string(&policy).expect("fixed policy is serializable")
        );
        self
    }

    pub fn renderer_identity(&self) -> &PromptRendererIdentity {
        &self.renderer
    }

    pub(crate) fn budget_policy(&self) -> ContextBudgetPolicy {
        self.budget_policy
    }

    pub(crate) fn window_policy(
        &self,
        capabilities: &ModelCapabilities,
    ) -> Result<super::window_policy::ContextWindowPolicy, ContextPlannerError> {
        self.budget_policy.for_capabilities(capabilities)
    }

    pub fn default_budget(
        &self,
        capabilities: &ModelCapabilities,
    ) -> Result<ContextBudget, ContextPlannerError> {
        Ok(self.window_policy(capabilities)?.budget())
    }

    pub(crate) fn tool_free_compaction_budget(
        &self,
        capabilities: &ModelCapabilities,
    ) -> Result<ContextBudget, ContextPlannerError> {
        let ordinary = self.default_budget(capabilities)?;
        ordinary.without_tool_reserve()
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
        fragments: Vec<ContextFragment>,
        budget: ContextBudget,
        capabilities: &ModelCapabilities,
        tool_specs: Vec<ToolSpec>,
        requirements: ModelRequestRequirements,
        prior_manifest: Option<&FullLogicalManifest>,
    ) -> Result<ContextPlan, ContextPlannerError> {
        self.compile_inner(
            fragments,
            budget,
            capabilities,
            tool_specs,
            requirements,
            prior_manifest,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn compile_with_capacity_breakdown(
        &self,
        fragments: Vec<ContextFragment>,
        budget: ContextBudget,
        capabilities: &ModelCapabilities,
        tool_specs: Vec<ToolSpec>,
        requirements: ModelRequestRequirements,
        prior_manifest: Option<&FullLogicalManifest>,
        breakdown: &mut Option<ContextCapacityBreakdown>,
    ) -> Result<ContextPlan, ContextPlannerError> {
        *breakdown = None;
        let result = self.compile_inner(
            fragments,
            budget,
            capabilities,
            tool_specs,
            requirements,
            prior_manifest,
            Some(&mut *breakdown),
        );
        if !matches!(
            &result,
            Err(ContextPlannerError::FixedInputExceedsBudget { .. }
                | ContextPlannerError::RequiredFragmentsExceedBudget { .. }
                | ContextPlannerError::RenderedInputExceedsBudget { .. })
        ) {
            *breakdown = None;
        }
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn compile_inner(
        &self,
        mut fragments: Vec<ContextFragment>,
        budget: ContextBudget,
        capabilities: &ModelCapabilities,
        tool_specs: Vec<ToolSpec>,
        requirements: ModelRequestRequirements,
        prior_manifest: Option<&FullLogicalManifest>,
        breakdown: Option<&mut Option<ContextCapacityBreakdown>>,
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
        let mut requirements =
            requirements.normalized_for_tool_specs(!canonical_tools.ordered_specs.is_empty());
        requirements.reasoning_effort =
            capabilities.resolve_reasoning_effort(requirements.reasoning_effort)?;
        requirements.max_output_tokens =
            capabilities.resolve_output_token_limit(requirements.max_output_tokens)?;
        if let Some(requested) = requirements.max_output_tokens
            && requested > budget.reserved_output_tokens()
        {
            return Err(ContextPlannerError::OutputReserveBelowLimit {
                requested,
                reserved: budget.reserved_output_tokens(),
            });
        }
        ensure_unique_ids(&fragments)?;
        sort_full_order(&mut fragments);
        if let Some(breakdown) = breakdown {
            *breakdown = Some(ContextCapacityBreakdown::from_fragments(
                &fragments,
                budget,
                fixed_input_tokens,
            ));
        }
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
            requirements: requirements.clone(),
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
        let rendered_input_tokens = self.estimate_rendered_model_input(model_input)?;
        if rendered_input_tokens > budget.input_token_limit() {
            return Err(ContextPlannerError::RenderedInputExceedsBudget {
                rendered_input_tokens,
                input_token_limit: budget.input_token_limit(),
            });
        }
        Ok(())
    }

    pub(crate) fn estimate_rendered_model_input(
        &self,
        model_input: &ModelInputIR,
    ) -> Result<u32, ContextPlannerError> {
        let canonical_tools = canonical_tool_schemas(&model_input.tool_specs).map_err(|error| {
            ContextPlannerError::CanonicalEncoding {
                message: error.to_string(),
            }
        })?;
        Ok(fixed_input_tokens(&canonical_tools).saturating_add(
            estimator::estimate_rendered_channels_tokens(
                &model_input.instructions,
                &model_input.content_items,
            )?,
        ))
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
