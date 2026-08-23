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
    ContextFragmentSource, ContextPlan, ContextPlanner, ContextPlannerError,
    ContextPlannerVersions, ContextReasonCode, ContextSourceFingerprint, FragmentCachePolicy,
    FullLogicalManifest, FullLogicalManifestEntry, LogicalAppendOnlyRejection,
    LogicalBaselineRelation,
};
