//! Provider-neutral request context and Baseline semantics.

mod baseline;
mod canonical;

pub use baseline::{BaselineAssessment, BaselineInvalidationReason, ProviderSession};
pub use canonical::{
    ContextError, InternalRequestMetadata, ModelIdentity, ModelProfile, ProviderSessionId,
    ProviderSessionRef, ProviderSessionRole, RequestHash, StepSnapshot, TurnContext, TurnId,
};
