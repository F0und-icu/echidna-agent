//! Context budget decisions, logical manifests and immutable compiled plans.

use serde::{Deserialize, Serialize};

use crate::domain::model::ModelRequestRequirements;
use crate::domain::tools::ToolSpec;

use super::error::ContextPlannerError;
use super::fragment::{
    ContextContentHash, ContextFragmentId, ContextFragmentSource, ContextOrigin, ContextSlot,
};
use super::render::PromptRendererIdentity;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct ContextBudget {
    context_window_tokens: u32,
    reserved_output_tokens: u32,
    reserved_tool_loop_tokens: u32,
}

impl ContextBudget {
    pub fn new(
        context_window_tokens: u32,
        reserved_output_tokens: u32,
        reserved_tool_loop_tokens: u32,
    ) -> Result<Self, ContextPlannerError> {
        context_window_tokens
            .checked_sub(reserved_output_tokens)
            .and_then(|remaining| remaining.checked_sub(reserved_tool_loop_tokens))
            .ok_or(ContextPlannerError::InvalidBudget {
                context_window_tokens,
                reserved_output_tokens,
                reserved_tool_loop_tokens,
            })?;
        Ok(Self {
            context_window_tokens,
            reserved_output_tokens,
            reserved_tool_loop_tokens,
        })
    }

    pub const fn context_window_tokens(self) -> u32 {
        self.context_window_tokens
    }

    pub const fn reserved_output_tokens(self) -> u32 {
        self.reserved_output_tokens
    }

    pub const fn reserved_tool_loop_tokens(self) -> u32 {
        self.reserved_tool_loop_tokens
    }

    pub const fn input_token_limit(self) -> u32 {
        self.context_window_tokens - self.reserved_output_tokens - self.reserved_tool_loop_tokens
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContextDecision {
    Include,
    Truncate {
        retained_tokens: u32,
    },
    ArtifactRef {
        artifact_id: crate::domain::artifacts::ArtifactId,
    },
    Exclude,
}

impl ContextDecision {
    pub(super) fn is_rendered(&self) -> bool {
        !matches!(self, Self::Exclude)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ContextReasonCode {
    RequiredBaseInstructions,
    RequiredRecentInput,
    SelectedByPriority,
    BudgetTruncated,
    BudgetArtifactReference,
    BudgetExcluded,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LogicalBaselineRelation {
    NoPriorManifest,
    PrefixRetained,
    Appended,
    Replaced,
    Excluded,
}

/// A pure comparison between logical fragment manifests.
///
/// This value is inspection data only. It never authorizes incremental
/// Provider delivery; `ProviderSession` remains the sole Baseline authority.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AppendOnlyCandidate {
    NoPriorManifest,
    LogicalPrefixDelta {
        retained_prefix_len: usize,
        appended_fragment_ids: Vec<ContextFragmentId>,
    },
    NotAppendOnly {
        reason: LogicalAppendOnlyRejection,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LogicalAppendOnlyRejection {
    PriorFragmentChanged,
    CurrentManifestShorter,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct FullLogicalManifestEntry {
    pub(super) fragment_id: ContextFragmentId,
    pub(super) source: ContextFragmentSource,
    #[serde(skip)]
    pub(super) slot: ContextSlot,
    #[serde(skip)]
    pub(super) origin: ContextOrigin,
    pub(super) content_hash: ContextContentHash,
    pub(super) decision: ContextDecision,
}

impl FullLogicalManifestEntry {
    pub fn fragment_id(&self) -> &ContextFragmentId {
        &self.fragment_id
    }

    pub const fn source(&self) -> ContextFragmentSource {
        self.source
    }

    pub const fn slot(&self) -> ContextSlot {
        self.slot
    }

    pub const fn origin(&self) -> ContextOrigin {
        self.origin
    }

    pub fn content_hash(&self) -> &ContextContentHash {
        &self.content_hash
    }

    pub fn decision(&self) -> &ContextDecision {
        &self.decision
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct FullLogicalManifest {
    pub(super) entries: Vec<FullLogicalManifestEntry>,
}

impl FullLogicalManifest {
    pub fn entries(&self) -> &[FullLogicalManifestEntry] {
        &self.entries
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ContextDecisionEntry {
    pub(super) fragment_id: ContextFragmentId,
    pub(super) source: ContextFragmentSource,
    #[serde(skip)]
    pub(super) slot: ContextSlot,
    #[serde(skip)]
    pub(super) origin: ContextOrigin,
    pub(super) content_hash: ContextContentHash,
    pub(super) token_estimate: u32,
    pub(super) decision: ContextDecision,
    pub(super) reason_code: ContextReasonCode,
    pub(super) baseline_relation: LogicalBaselineRelation,
}

impl ContextDecisionEntry {
    pub fn fragment_id(&self) -> &ContextFragmentId {
        &self.fragment_id
    }

    pub const fn source(&self) -> ContextFragmentSource {
        self.source
    }

    pub const fn slot(&self) -> ContextSlot {
        self.slot
    }

    pub const fn origin(&self) -> ContextOrigin {
        self.origin
    }

    pub fn content_hash(&self) -> &ContextContentHash {
        &self.content_hash
    }

    pub const fn token_estimate(&self) -> u32 {
        self.token_estimate
    }

    pub fn decision(&self) -> &ContextDecision {
        &self.decision
    }

    pub const fn reason_code(&self) -> ContextReasonCode {
        self.reason_code
    }

    pub fn baseline_relation(&self) -> &LogicalBaselineRelation {
        &self.baseline_relation
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ContextDecisionManifest {
    pub(super) context_fingerprint: String,
    pub(super) budget: ContextBudget,
    pub(super) fixed_input_tokens: u32,
    pub(super) candidates: Vec<ContextDecisionEntry>,
}

impl ContextDecisionManifest {
    pub fn context_fingerprint(&self) -> &str {
        &self.context_fingerprint
    }

    pub const fn budget(&self) -> ContextBudget {
        self.budget
    }

    pub const fn fixed_input_tokens(&self) -> u32 {
        self.fixed_input_tokens
    }

    pub fn candidates(&self) -> &[ContextDecisionEntry] {
        &self.candidates
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ContextPlannerVersions {
    pub(super) planner_version: String,
    pub(super) estimator_version: String,
    pub(super) budget_policy_version: String,
}

impl ContextPlannerVersions {
    pub fn new(
        planner_version: impl Into<String>,
        estimator_version: impl Into<String>,
        budget_policy_version: impl Into<String>,
    ) -> Self {
        Self {
            planner_version: planner_version.into(),
            estimator_version: estimator_version.into(),
            budget_policy_version: budget_policy_version.into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ContextPlan {
    pub(super) context_fingerprint: String,
    pub(super) prompt_cache_key: String,
    pub(super) prompt_renderer: PromptRendererIdentity,
    pub(super) full_logical_manifest: FullLogicalManifest,
    pub(super) append_only_candidate: AppendOnlyCandidate,
    pub(super) context_decision_manifest: ContextDecisionManifest,
    pub(super) model_input: crate::domain::model::ModelInputIR,
}

impl ContextPlan {
    pub fn context_fingerprint(&self) -> &str {
        &self.context_fingerprint
    }

    pub fn prompt_cache_key(&self) -> &str {
        &self.prompt_cache_key
    }

    pub fn prompt_renderer(&self) -> &PromptRendererIdentity {
        &self.prompt_renderer
    }

    pub fn full_logical_manifest(&self) -> &FullLogicalManifest {
        &self.full_logical_manifest
    }

    pub fn append_only_candidate(&self) -> &AppendOnlyCandidate {
        &self.append_only_candidate
    }

    pub fn context_decision_manifest(&self) -> &ContextDecisionManifest {
        &self.context_decision_manifest
    }

    pub fn base_instructions(&self) -> &str {
        &self.model_input.instructions
    }

    pub fn input_items(&self) -> &[crate::domain::model::ContentItem] {
        &self.model_input.content_items
    }

    pub fn tool_specs(&self) -> &[ToolSpec] {
        &self.model_input.tool_specs
    }

    pub const fn requirements(&self) -> ModelRequestRequirements {
        self.model_input.requirements
    }

    pub fn model_input(&self) -> crate::domain::model::ModelInputIR {
        self.model_input.clone()
    }
}
