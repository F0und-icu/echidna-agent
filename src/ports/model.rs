//! Provider-neutral streaming Model Port.

use std::pin::Pin;

use futures_core::Stream;
use thiserror::Error;

use crate::domain::model::{
    ContentItem, ModelCapabilities, ModelCapabilityError, ModelEvent, ModelInputIR,
    OpaqueContinuationScope,
};

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

/// Whether Adapter-local continuation state outlives this sample.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum ContinuationRetention {
    #[default]
    Session,
    SingleSample,
}

/// Presentation of this request is separate from its delivery and retention.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum ModelRequestPresentation {
    #[default]
    Standard,
    TaskSelection,
}

/// One semantic request plus the App-selected delivery mode.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelRequest {
    input: ModelInputIR,
    delivery: ModelRequestDelivery,
    opaque_continuation_scope: Option<OpaqueContinuationScope>,
    continuation_retention: ContinuationRetention,
    presentation: ModelRequestPresentation,
}

impl ModelRequest {
    pub fn full(input: ModelInputIR) -> Self {
        Self {
            input,
            delivery: ModelRequestDelivery::Full,
            opaque_continuation_scope: None,
            continuation_retention: ContinuationRetention::Session,
            presentation: ModelRequestPresentation::Standard,
        }
    }

    pub fn incremental(input: ModelInputIR, continuation: ProviderResponseId) -> Self {
        Self {
            input,
            delivery: ModelRequestDelivery::Incremental { continuation },
            opaque_continuation_scope: None,
            continuation_retention: ContinuationRetention::Session,
            presentation: ModelRequestPresentation::Standard,
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

    /// Binds Adapter-local opaque continuation references to the current
    /// provider delivery session. Normal providers leave this absent.
    pub fn with_opaque_continuation_scope(mut self, scope: OpaqueContinuationScope) -> Self {
        self.opaque_continuation_scope = Some(scope);
        self
    }

    pub fn opaque_continuation_scope(&self) -> Option<&OpaqueContinuationScope> {
        self.opaque_continuation_scope.as_ref()
    }

    pub(crate) fn with_continuation_retention(mut self, retention: ContinuationRetention) -> Self {
        self.continuation_retention = retention;
        self
    }

    pub(crate) fn continuation_retention(&self) -> ContinuationRetention {
        self.continuation_retention
    }

    pub(crate) fn with_presentation(mut self, presentation: ModelRequestPresentation) -> Self {
        self.presentation = presentation;
        self
    }

    pub(crate) fn presentation(&self) -> ModelRequestPresentation {
        self.presentation
    }

    pub fn into_parts(
        self,
    ) -> (
        ModelInputIR,
        ModelRequestDelivery,
        Option<OpaqueContinuationScope>,
    ) {
        (self.input, self.delivery, self.opaque_continuation_scope)
    }

    /// Validates semantic input and the selected delivery against one explicit
    /// capability mapping. Adapter implementations share this Port rule.
    pub fn validate_capabilities(
        &self,
        capabilities: &ModelCapabilities,
    ) -> Result<(), ModelProviderError> {
        if self.continuation_retention == ContinuationRetention::SingleSample
            && (self.is_incremental()
                || self
                    .input
                    .content_items
                    .iter()
                    .any(|item| matches!(item, ContentItem::OpaqueContinuation { .. })))
        {
            return Err(ModelProviderError::RequestEncodingFailure {
                message: "single-sample continuation requires Full input without opaque references"
                    .to_owned(),
            });
        }
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

/// One encoded Provider request retained for bounded retries of an
/// uncompleted transport attempt. Implementations may retain credentials and
/// wire bytes here so retrying does not repeat credential maintenance or
/// request encoding.
pub trait PreparedModelRequest {
    fn stream(&self) -> Result<ModelEventStream, ModelProviderError>;

    /// Reports a discarded incomplete attempt through the Provider's existing
    /// presentation boundary. Ordinary Providers have no presentation work.
    fn retrying(&self, _retry: usize) {}
}

struct StreamPreparedModelRequest<'provider, Provider: ?Sized> {
    provider: &'provider Provider,
    request: ModelRequest,
}

impl<Provider> PreparedModelRequest for StreamPreparedModelRequest<'_, Provider>
where
    Provider: ModelProvider + ?Sized,
{
    fn stream(&self) -> Result<ModelEventStream, ModelProviderError> {
        self.provider.stream(self.request.clone())
    }
}

/// Provider-neutral boundary for a configured model implementation.
pub trait ModelProvider: Send + Sync {
    fn capabilities(&self) -> ModelCapabilities;

    fn stream(&self, request: ModelRequest) -> Result<ModelEventStream, ModelProviderError>;

    /// Prepares one logical request. The default preserves existing providers
    /// by delegating each physical attempt to their single-attempt stream.
    fn prepare(
        &self,
        request: ModelRequest,
    ) -> Result<Box<dyn PreparedModelRequest + '_>, ModelProviderError> {
        Ok(Box::new(StreamPreparedModelRequest {
            provider: self,
            request,
        }))
    }
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
    #[error("model connection failed: {message}; timeout={timeout}, io_kind={io_kind}")]
    ConnectionFailure {
        timeout: bool,
        io_kind: String,
        message: String,
    },
    #[error(
        "model response interrupted: status={status}, elapsed_ms={elapsed_ms}, timeout={timeout}, io_kind={io_kind}"
    )]
    ResponseInterrupted {
        status: u16,
        elapsed_ms: u64,
        timeout: bool,
        io_kind: String,
    },
    #[error("model HTTP request failed with status {status}: {body}")]
    HttpFailure { status: u16, body: String },
    #[error("model context capacity exceeded ({code})")]
    ContextCapacityExceeded { code: String },
    #[error("model stream protocol failed: {message}")]
    ProtocolFailure { message: String },
    #[error(
        "model stream protocol failed: function call arguments are not valid JSON: {diagnostic}; rejected before dispatch"
    )]
    InvalidToolArguments {
        diagnostic: crate::domain::model::ToolArgumentDiagnostic,
    },
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
