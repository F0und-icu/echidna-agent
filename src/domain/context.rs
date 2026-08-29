//! Provider-neutral request context and Baseline semantics.

mod baseline;
mod canonical;
mod planner;

pub use baseline::{BaselineAssessment, BaselineInvalidationReason, ProviderSession};
pub use canonical::{
    ContextError, InternalRequestMetadata, ModelIdentity, ModelProfile, ProviderSessionId,
    ProviderSessionRef, ProviderSessionRole, RequestHash, StepSnapshot, TurnContext, TurnId,
};
pub use planner::{
    AppendOnlyCandidate, CONTEXT_FRAGMENT_CACHE_KEY_VERSION, ContextBudget, ContextContentHash,
    ContextDecision, ContextDecisionEntry, ContextDecisionManifest, ContextFragment,
    ContextFragmentCacheKey, ContextFragmentContent, ContextFragmentId, ContextFragmentPriority,
    ContextFragmentSource, ContextOrigin, ContextPlan, ContextPlanner, ContextPlannerError,
    ContextPlannerVersions, ContextReasonCode, ContextSlot, ContextSourceFingerprint,
    FragmentCachePolicy, FullLogicalManifest, FullLogicalManifestEntry, LogicalAppendOnlyRejection,
    LogicalBaselineRelation, PromptRendererIdentity,
};

pub(crate) use planner::{estimate_audit_component_tokens, tool_schema_token_cost};
