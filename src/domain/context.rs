//! Provider-neutral request context and Baseline semantics.

mod baseline;
mod canonical;
mod planner;
mod window_policy;

pub use baseline::{BaselineAssessment, BaselineInvalidationReason, ProviderSession};
pub use canonical::{
    ContextError, InternalRequestMetadata, ModelIdentity, ModelProfile, ProviderSessionId,
    ProviderSessionRef, ProviderSessionRole, RequestHash, StepSnapshot, TurnContext, TurnId,
};
pub use planner::{
    AppendOnlyCandidate, AuthorityLevel, CONTEXT_DELIVERY_MAP_SCHEMA_VERSION,
    CONTEXT_FRAGMENT_CACHE_KEY_VERSION, ContextBudget, ContextContentDelivery, ContextContentHash,
    ContextDecision, ContextDecisionEntry, ContextDecisionManifest, ContextDeliveryMap,
    ContextFragment, ContextFragmentCacheKey, ContextFragmentContent, ContextFragmentId,
    ContextFragmentPriority, ContextFragmentSource, ContextInstructionSpan, ContextOrigin,
    ContextPlan, ContextPlanner, ContextPlannerError, ContextPlannerVersions, ContextProvenance,
    ContextReasonCode, ContextReferences, ContextSlot, ContextSourceFingerprint, ContextSubitemId,
    ContextTransform, FragmentCachePolicy, FreshnessIdentity, FullLogicalManifest,
    FullLogicalManifestEntry, LogicalAppendOnlyRejection, LogicalBaselineRelation,
    PromptRendererIdentity, RetentionClass, SemanticClass,
};

pub(crate) use planner::{
    ContextCapacityBreakdown, estimate_audit_component_tokens, tool_schema_token_cost,
};
pub(crate) use window_policy::{ContextBudgetPolicy, ContextWindowPolicy};
