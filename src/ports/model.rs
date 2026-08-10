//! Provider-neutral streaming Model Port.

use std::pin::Pin;

use futures_core::Stream;
use thiserror::Error;

use crate::domain::model::{ModelCapabilities, ModelCapabilityError, ModelEvent, ModelInputIR};

/// The asynchronous event sequence returned for one model request.
pub type ModelEventStream =
    Pin<Box<dyn Stream<Item = Result<ModelEvent, ModelProviderError>> + Send>>;

/// Provider-neutral boundary for a configured model implementation.
pub trait ModelProvider: Send + Sync {
    fn capabilities(&self) -> ModelCapabilities;

    fn stream(&self, input: ModelInputIR) -> Result<ModelEventStream, ModelProviderError>;
}

/// Errors crossing the Model Port without exposing a transport protocol.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ModelProviderError {
    #[error(transparent)]
    Capability(#[from] ModelCapabilityError),
    #[error("model credential `{credential_ref}` is not configured")]
    CredentialUnavailable { credential_ref: String },
    #[error("model credential source `{credential_ref}` failed: {message}")]
    CredentialSourceFailure {
        credential_ref: String,
        message: String,
    },
    #[error("model credential `{credential_ref}` is invalid")]
    CredentialInvalid { credential_ref: String },
    #[error("model request encoding failed: {message}")]
    RequestEncodingFailure { message: String },
    #[error("model transport failed: {message}")]
    TransportFailure { message: String },
    #[error("model HTTP request failed with status {status}")]
    HttpFailure { status: u16 },
    #[error("model stream protocol failed: {message}")]
    ProtocolFailure { message: String },
    #[error("model Provider failed ({code:?}): {message}")]
    ProviderFailure {
        code: Option<String>,
        message: String,
    },
    #[error("model stream failed: {message}")]
    StreamFailure { message: String },
}
