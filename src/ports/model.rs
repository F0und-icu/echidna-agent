//! Provider-neutral streaming Model Port.

use std::pin::Pin;

use futures_core::Stream;
use thiserror::Error;

use crate::domain::model::{ModelCapabilities, ModelCapabilityError, ModelEvent, ModelInputIR};

/// An opaque Provider-issued identity for one completed response.
///
/// The Core never interprets this value. An Adapter may use it to continue a
/// compatible request with its own protocol, while the App keeps it paired
/// with the local, committed semantic Baseline.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderResponseId(String);

impl ProviderResponseId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The requested wire delivery for one otherwise provider-neutral input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ModelRequestDelivery {
    Full,
    Incremental { continuation: ProviderResponseId },
}

/// One semantic request plus the App-selected delivery mode.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelRequest {
    input: ModelInputIR,
    delivery: ModelRequestDelivery,
}

impl ModelRequest {
    pub fn full(input: ModelInputIR) -> Self {
        Self {
            input,
            delivery: ModelRequestDelivery::Full,
        }
    }

    pub fn incremental(input: ModelInputIR, continuation: ProviderResponseId) -> Self {
        Self {
            input,
            delivery: ModelRequestDelivery::Incremental { continuation },
        }
    }

    pub fn input(&self) -> &ModelInputIR {
        &self.input
    }

    pub fn delivery(&self) -> &ModelRequestDelivery {
        &self.delivery
    }

    pub fn is_incremental(&self) -> bool {
        matches!(self.delivery, ModelRequestDelivery::Incremental { .. })
    }

    pub fn into_parts(self) -> (ModelInputIR, ModelRequestDelivery) {
        (self.input, self.delivery)
    }

    /// Validates semantic input and the selected delivery against one explicit
    /// capability mapping. Adapter implementations share this Port rule.
    pub fn validate_capabilities(
        &self,
        capabilities: &ModelCapabilities,
    ) -> Result<(), ModelProviderError> {
        capabilities.validate_input(&self.input)?;
        if self.is_incremental() && !capabilities.incremental_input {
            return Err(ModelProviderError::Capability(
                ModelCapabilityError::IncrementalInputUnsupported,
            ));
        }
        Ok(())
    }
}

/// Metadata emitted exactly once when a model request completes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelResponseCompletion {
    response_id: Option<ProviderResponseId>,
}

impl ModelResponseCompletion {
    pub fn new(response_id: ProviderResponseId) -> Self {
        Self {
            response_id: Some(response_id),
        }
    }

    pub fn without_response_id() -> Self {
        Self { response_id: None }
    }

    pub fn response_id(&self) -> Option<&ProviderResponseId> {
        self.response_id.as_ref()
    }
}

/// One item from a Provider stream. Completion metadata intentionally belongs
/// to the Port rather than the provider-neutral Domain event vocabulary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ModelStreamEvent {
    Event(ModelEvent),
    Completed(ModelResponseCompletion),
}

/// The asynchronous event sequence returned for one model request.
pub type ModelEventStream =
    Pin<Box<dyn Stream<Item = Result<ModelStreamEvent, ModelProviderError>> + Send>>;

/// Provider-neutral boundary for a configured model implementation.
pub trait ModelProvider: Send + Sync {
    fn capabilities(&self) -> ModelCapabilities;

    fn stream(&self, request: ModelRequest) -> Result<ModelEventStream, ModelProviderError>;
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
    #[error("model Provider rejected the requested continuation ({code})")]
    ContinuationRejected { code: String },
    #[error("model stream failed: {message}")]
    StreamFailure { message: String },
}
