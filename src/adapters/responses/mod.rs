//! One provider-neutral Responses HTTP/SSE implementation selected entirely
//! by explicit configuration.

mod cli_output;
mod request;
mod response;

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use async_stream::try_stream;
use eventsource_stream::{EventStreamError, Eventsource};
use futures_util::StreamExt;
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderName, HeaderValue};

use crate::adapters::credentials::{
    AccountBoundCredential, CredentialResolution, CredentialSource, CredentialSourceError,
};
use crate::adapters::image::resolve_project_image_artifact;
use crate::domain::artifacts::ArtifactId;
use crate::domain::model::{
    ContentItem, ModelCapabilities, OpaqueContinuationReference, OpaqueContinuationScope,
};
use crate::domain::request_audit::RequestAdapterVersion;
use crate::ports::model::{
    ContinuationRetention, ModelEventStream, ModelProvider, ModelProviderError, ModelRequest,
    ModelStreamEvent, PreparedModelRequest,
};

pub(crate) const DEFAULT_CONNECT_TIMEOUT_MS: u64 = 30_000;
pub(crate) const DEFAULT_READ_TIMEOUT_MS: u64 = 300_000;
/// Four encoded MiB is the largest continuation item that can enter the
/// Adapter-local store. The Context estimator still rejects it when it cannot
/// fit the configured model window.
pub(crate) const MAX_OPAQUE_CONTINUATION_ITEM_BYTES: usize = 4 * 1024 * 1024;
const MAX_OPAQUE_CONTINUATION_ITEMS: usize = 512;
const MAX_OPAQUE_CONTINUATION_STORE_BYTES: usize = 16 * 1024 * 1024;
const MAX_HTTP_ERROR_BODY_BYTES: usize = 64 * 1024;
const MAX_CHATGPT_ACCOUNT_ID_BYTES: usize = 256;
const CHATGPT_ACCOUNT_ID: HeaderName = HeaderName::from_static("chatgpt-account-id");

/// The explicit wire contract selected by Config. Provider names, endpoints
/// and model slugs never select this profile implicitly.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ResponsesWireProfile {
    Standard,
    ChatGptSubscription,
    CodexSubscription,
}

impl ResponsesWireProfile {
    pub(crate) fn request_adapter_version(self) -> Option<RequestAdapterVersion> {
        let version = match self {
            Self::Standard => "responses-http-sse-v1",
            Self::ChatGptSubscription => "responses-http-sse-chatgpt-subscription-v1",
            Self::CodexSubscription => return None,
        };
        Some(
            RequestAdapterVersion::new(version)
                .expect("the fixed Responses HTTP/SSE Adapter version is non-empty"),
        )
    }

    pub(super) const fn is_subscription(self) -> bool {
        matches!(self, Self::ChatGptSubscription | Self::CodexSubscription)
    }

    pub(super) const fn subscription_label(self) -> &'static str {
        match self {
            Self::ChatGptSubscription => "ChatGPT subscription",
            Self::CodexSubscription => "Codex subscription",
            Self::Standard => "Responses provider",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResponsesHttpConfig {
    base_url: String,
    model: String,
    credential_reference: String,
    capabilities: ModelCapabilities,
    connect_timeout: Duration,
    read_timeout: Duration,
    wire_profile: ResponsesWireProfile,
    codex_reasoning_summary_supported: bool,
}

impl ResponsesHttpConfig {
    pub fn new(
        base_url: impl Into<String>,
        model: impl Into<String>,
        credential_reference: impl Into<String>,
        capabilities: ModelCapabilities,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            model: model.into(),
            credential_reference: credential_reference.into(),
            capabilities,
            connect_timeout: Duration::from_millis(DEFAULT_CONNECT_TIMEOUT_MS),
            read_timeout: Duration::from_millis(DEFAULT_READ_TIMEOUT_MS),
            wire_profile: ResponsesWireProfile::Standard,
            codex_reasoning_summary_supported: true,
        }
    }

    pub(crate) fn with_timeouts(mut self, connect_ms: u64, read_ms: u64) -> Self {
        self.connect_timeout = Duration::from_millis(connect_ms);
        self.read_timeout = Duration::from_millis(read_ms);
        self
    }

    /// Selects the explicit ChatGPT subscription Responses wire contract.
    /// This remains opt-in; [`Self::new`] preserves the standard profile.
    pub fn with_chatgpt_subscription_profile(mut self) -> Self {
        self.wire_profile = ResponsesWireProfile::ChatGptSubscription;
        self
    }

    /// Selects the explicit Codex subscription Responses wire contract.
    /// This remains opt-in; [`Self::new`] preserves the standard profile.
    pub fn with_codex_subscription_profile(mut self) -> Self {
        self.wire_profile = ResponsesWireProfile::CodexSubscription;
        self
    }

    /// Binds the Codex summary wire option and its RequestAudit identity to
    /// this explicit Responses configuration.
    pub(crate) fn with_codex_reasoning_summary_support(mut self, supported: Option<bool>) -> Self {
        self.codex_reasoning_summary_supported = supported.unwrap_or(true);
        self
    }

    pub(crate) fn request_adapter_version(&self) -> RequestAdapterVersion {
        self.wire_profile
            .request_adapter_version()
            .unwrap_or_else(|| {
                Self::codex_subscription_request_adapter_version(
                    self.codex_reasoning_summary_supported,
                )
            })
    }

    pub(crate) fn codex_subscription_request_adapter_version(
        summary_supported: bool,
    ) -> RequestAdapterVersion {
        RequestAdapterVersion::new(if summary_supported {
            "responses-http-sse-codex-subscription-v2"
        } else {
            "responses-http-sse-codex-subscription-no-summary-v1"
        })
        .expect("the fixed Codex Responses HTTP/SSE Adapter version is non-empty")
    }

    fn endpoint(&self) -> String {
        format!("{}/responses", self.base_url.trim_end_matches('/'))
    }
}

#[derive(Clone)]
pub struct ResponsesHttpAdapter {
    client: reqwest::Client,
    config: ResponsesHttpConfig,
    credential_source: Arc<dyn CredentialSource>,
    image_resolver: Option<Arc<ImageArtifactResolver>>,
    cli_role: Option<&'static str>,
    opaque_continuations: Arc<OpaqueContinuationStore>,
    #[cfg(debug_assertions)]
    debug: bool,
}

pub(crate) type ImageArtifactResolver =
    dyn Fn(&ArtifactId) -> Result<(String, Vec<u8>), ModelProviderError> + Send + Sync;

impl ResponsesHttpAdapter {
    /// Returns the fixed RequestAudit identity for this concrete wire
    /// implementation. Runtime consumers receive it from ProviderComposition
    /// instead of encoding protocol knowledge themselves.
    pub fn request_adapter_version() -> RequestAdapterVersion {
        ResponsesWireProfile::Standard
            .request_adapter_version()
            .expect("the standard Responses wire profile has a fixed identity")
    }

    pub fn new(config: ResponsesHttpConfig, credential_source: Arc<dyn CredentialSource>) -> Self {
        Self {
            client: reqwest::Client::builder()
                .connect_timeout(config.connect_timeout)
                .read_timeout(config.read_timeout)
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("Responses HTTP client initialization failed"),
            config,
            credential_source,
            image_resolver: None,
            cli_role: None,
            opaque_continuations: Arc::new(OpaqueContinuationStore::default()),
            #[cfg(debug_assertions)]
            debug: false,
        }
    }

    pub fn with_project_image_resolver(
        mut self,
        project: crate::adapters::project_event_store::Project,
    ) -> Self {
        self.image_resolver = Some(Arc::new(move |artifact_id| {
            resolve_project_image_artifact(&project, artifact_id).map_err(request_encoding_error)
        }));
        self
    }

    pub(crate) fn with_cli_output(
        mut self,
        role: Option<&'static str>,
        #[cfg(debug_assertions)] debug: bool,
    ) -> Self {
        self.cli_role = role;
        #[cfg(debug_assertions)]
        {
            self.debug = debug;
        }
        self
    }
}

fn request_encoding_error(error: impl ToString) -> ModelProviderError {
    ModelProviderError::RequestEncodingFailure {
        message: error.to_string(),
    }
}

fn response_body_failure(
    error: reqwest::Error,
    status: reqwest::StatusCode,
    elapsed: Duration,
) -> ModelProviderError {
    ModelProviderError::ResponseInterrupted {
        status: status.as_u16(),
        elapsed_ms: elapsed.as_millis().try_into().unwrap_or(u64::MAX),
        timeout: error.is_timeout(),
        io_kind: source_io_kind(&error),
    }
}

fn connection_failure(error: &reqwest::Error) -> ModelProviderError {
    ModelProviderError::ConnectionFailure {
        timeout: error.is_timeout(),
        io_kind: source_io_kind(error),
    }
}

fn source_io_kind(error: &reqwest::Error) -> String {
    let mut io_kind = None;
    let mut source = error.source();
    while let Some(cause) = source {
        if let Some(error) = cause.downcast_ref::<std::io::Error>() {
            io_kind = Some(error.kind());
        }
        source = cause.source();
    }
    // Only the finite error-kind label crosses the Port, never a source message.
    format!("{io_kind:?}")
}

impl ModelProvider for ResponsesHttpAdapter {
    fn capabilities(&self) -> ModelCapabilities {
        self.config.capabilities.clone()
    }

    fn stream(&self, request: ModelRequest) -> Result<ModelEventStream, ModelProviderError> {
        self.prepare(request)?.stream()
    }

    fn prepare(
        &self,
        request: ModelRequest,
    ) -> Result<Box<dyn PreparedModelRequest + '_>, ModelProviderError> {
        request.validate_capabilities(&self.config.capabilities)?;

        let credential_reference = self.config.credential_reference.clone();
        let usage_required = request.input().requirements.usage_reporting;
        let continuation_required = self.config.capabilities.incremental_input;
        let continuation_requested = request.is_incremental();
        let opaque_scope = request.opaque_continuation_scope().cloned();
        let retained_opaque = OpaqueContinuationStore::references_in(request.input());
        let opaque_continuations = match request.continuation_retention() {
            ContinuationRetention::Session => self.opaque_continuations.clone(),
            ContinuationRetention::SingleSample => Arc::new(OpaqueContinuationStore::default()),
        };
        let profile = self.config.wire_profile;
        let request = request::encode_request(
            request,
            self.config.model.clone(),
            self.image_resolver.as_deref(),
            profile,
            self.config.codex_reasoning_summary_supported,
            &|reference, scope| opaque_continuations.resolve(reference, scope),
        )?;
        let request = serde_json::to_value(request).map_err(request_encoding_error)?;
        let request_body = serde_json::to_vec(&request).map_err(request_encoding_error)?;
        let credential = match (
            profile,
            self.credential_source.resolve(&credential_reference),
        ) {
            (
                ResponsesWireProfile::CodexSubscription,
                CredentialResolution::PendingAccountBound(resolution),
            ) => CredentialForStream::PendingAccountBound(resolution),
            (ResponsesWireProfile::CodexSubscription, _) => {
                return Err(request_encoding_error(
                    "Codex subscription Responses requires an account-bound credential",
                ));
            }
            (_, CredentialResolution::PendingAccountBound(_)) => {
                return Err(request_encoding_error(
                    "account-bound credentials require the explicit Codex subscription Responses profile",
                ));
            }
            (_, CredentialResolution::Ready(result)) => {
                CredentialForStream::Ready(validate_credential(result, &credential_reference)?)
            }
            (_, CredentialResolution::Pending(resolution)) => {
                CredentialForStream::Pending(resolution)
            }
        };
        Ok(Box::new(PreparedResponsesRequest {
            client: self.client.clone(),
            endpoint: self.config.endpoint(),
            #[cfg(debug_assertions)]
            request,
            request_body,
            credential: Arc::new(Mutex::new(Some(credential))),
            credential_reference,
            usage_required,
            continuation_required,
            continuation_requested,
            opaque_scope,
            retained_opaque,
            opaque_continuations,
            profile,
            cli_role: self.cli_role,
            partial_text: Arc::new(AtomicBool::new(false)),
            #[cfg(debug_assertions)]
            full_debug: self.debug,
        }))
    }
}

struct PreparedResponsesRequest {
    client: reqwest::Client,
    endpoint: String,
    #[cfg(debug_assertions)]
    request: serde_json::Value,
    request_body: Vec<u8>,
    credential: Arc<Mutex<Option<CredentialForStream>>>,
    credential_reference: String,
    usage_required: bool,
    continuation_required: bool,
    continuation_requested: bool,
    opaque_scope: Option<OpaqueContinuationScope>,
    retained_opaque: BTreeSet<OpaqueContinuationReference>,
    opaque_continuations: Arc<OpaqueContinuationStore>,
    profile: ResponsesWireProfile,
    cli_role: Option<&'static str>,
    partial_text: Arc<AtomicBool>,
    #[cfg(debug_assertions)]
    full_debug: bool,
}

impl PreparedModelRequest for PreparedResponsesRequest {
    fn stream(&self) -> Result<ModelEventStream, ModelProviderError> {
        let client = self.client.clone();
        let endpoint = self.endpoint.clone();
        #[cfg(debug_assertions)]
        let request = self.request.clone();
        let request_body = self.request_body.clone();
        let credential = Arc::clone(&self.credential);
        let credential_reference = self.credential_reference.clone();
        let usage_required = self.usage_required;
        let continuation_required = self.continuation_required;
        let continuation_requested = self.continuation_requested;
        let opaque_scope = self.opaque_scope.clone();
        let retained_opaque = self.retained_opaque.clone();
        let opaque_continuations = Arc::clone(&self.opaque_continuations);
        let profile = self.profile;
        let cli_role = self.cli_role;
        let partial_text = Arc::clone(&self.partial_text);
        #[cfg(debug_assertions)]
        let full_debug = self.full_debug;

        let stream = try_stream! {
            let credential = prepared_credential(&credential, &credential_reference).await?;
            let StreamCredential {
                bearer,
                authorization,
                account_id,
            } = credential;
            let mut diagnostics = cli_role.map(|role| cli_output::RequestOutput::new(role, bearer,
                #[cfg(debug_assertions)]
                full_debug,
            ));
            #[cfg(debug_assertions)]
            if let Some(diagnostics) = &diagnostics {
                diagnostics.request(&request);
            }
            let request_started = Instant::now();
            let request_builder = client
                .post(endpoint)
                .header(ACCEPT, "text/event-stream")
                .header(AUTHORIZATION, authorization)
                .header(CONTENT_TYPE, "application/json")
                .body(request_body);
            let request_builder = match account_id {
                Some(account_id) => request_builder.header(CHATGPT_ACCOUNT_ID, account_id),
                None => request_builder,
            };
            let response = request_builder
                .send()
                .await
                .map_err(|error| {
                    if error.is_connect() || error.is_timeout() {
                        connection_failure(&error)
                    } else {
                        ModelProviderError::TransportFailure {
                            message: "request transport failed before a response".to_owned(),
                        }
                    }
                })?;

            let status = response.status();
            #[cfg(debug_assertions)]
            if let Some(diagnostics) = &diagnostics {
                diagnostics.status(status.as_u16());
            }
            let mut response_events = match (status.is_success(), profile, continuation_requested) {
                (true, _, _) => response.bytes_stream().eventsource(),
                (false, profile, _) if profile.is_subscription() => {
                    let body = read_bounded_http_error_body(response).await;
                    Err(response::subscription_http_failure(profile, status.as_u16(), &body))?;
                    unreachable!("an HTTP failure is returned through the stream")
                }
                (false, _, true) => {
                    let body = response.text().await.unwrap_or_default();
                    #[cfg(debug_assertions)]
                    if let Some(diagnostics) = &diagnostics {
                        diagnostics.http_error_body(&body);
                    }
                    if response::continuation_rejected_http_body(&body) {
                        Err(ModelProviderError::ContinuationRejected {
                            code: "previous_response_not_found".to_owned(),
                        })?;
                    }
                    Err(ModelProviderError::HttpFailure {
                        status: status.as_u16(),
                    })?;
                    unreachable!("an HTTP failure is returned through the stream")
                }
                (false, _, false) => {
                    #[cfg(debug_assertions)]
                    if full_debug {
                        let body = read_bounded_http_error_body(response).await;
                        if let Some(diagnostics) = &diagnostics {
                            match body {
                                response::BoundedHttpErrorBody::Complete(body) => {
                                    diagnostics.http_error_body(&body)
                                }
                                response::BoundedHttpErrorBody::Unreadable => {
                                    diagnostics.response_omitted("HTTP error body unavailable")
                                }
                                response::BoundedHttpErrorBody::Oversized => diagnostics
                                    .response_omitted("HTTP error body omitted: exceeds 65536 bytes"),
                            }
                        }
                        Err(ModelProviderError::HttpFailure {
                            status: status.as_u16(),
                        })?;
                        unreachable!("an HTTP failure is returned through the stream")
                    }
                    Err(ModelProviderError::HttpFailure {
                        status: status.as_u16(),
                    })?;
                    unreachable!("an HTTP failure is returned through the stream")
                }
            };
            let mut decoder = response::ResponseDecoder::new(profile, opaque_scope);
            let mut staged_opaque = BTreeMap::new();
            loop {
                let event = response_events
                    .next()
                    .await
                    .ok_or_else(|| ModelProviderError::ResponseInterrupted {
                        status: status.as_u16(),
                        elapsed_ms: request_started.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
                        timeout: false,
                        io_kind: "None".to_owned(),
                    })?
                    .map_err(|error| match error {
                        EventStreamError::Transport(error) =>
                            response_body_failure(error, status, request_started.elapsed()),
                        error => ModelProviderError::ProtocolFailure {
                            message: format!("invalid SSE stream: {error}"),
                        },
                    })?;

                if event.data.trim().is_empty() {
                    continue;
                }

                #[cfg(debug_assertions)]
                if !profile.is_subscription()
                    && let Some(diagnostics) = &diagnostics
                {
                    diagnostics.response(&event.data);
                }

                let decoded = decoder.decode_stream_event(
                    &event.data,
                    usage_required,
                    continuation_required,
                    continuation_requested,
                ).inspect_err(|error| {
                    if let Some(diagnostics) = &mut diagnostics {
                        diagnostics.rejected(error);
                    }
                })?;
                let response::DecodedResponsesEvent {
                    events,
                    reasoning_summaries,
                    completion,
                    opaque_continuations: decoded_opaque,
                } = decoded;
                for opaque in decoded_opaque {
                    opaque_continuations.stage(
                        &retained_opaque,
                        &mut staged_opaque,
                        opaque.reference,
                        opaque.item,
                    )?;
                }
                for summary in reasoning_summaries {
                    if let Some(diagnostics) = &mut diagnostics {
                        diagnostics.reasoning_summary(&summary.text, summary.starts_part);
                    }
                }
                for model_event in events {
                    if let Some(diagnostics) = &mut diagnostics {
                        diagnostics.model_event(&model_event);
                    }
                    if matches!(model_event, crate::domain::model::ModelEvent::TextDelta { .. }) {
                        partial_text.store(true, Ordering::Relaxed);
                    }
                    yield ModelStreamEvent::Event(model_event);
                }
                if let Some(completion) = completion {
                    opaque_continuations.commit(&retained_opaque, staged_opaque)?;
                    if let Some(diagnostics) = &mut diagnostics {
                        diagnostics.completed();
                    }
                    yield ModelStreamEvent::Completed(completion);
                    break;
                }
            }
        };

        Ok(Box::pin(stream))
    }

    fn retrying(&self, retry: usize) {
        if self.partial_text.swap(false, Ordering::Relaxed)
            && let Some(role) = self.cli_role
        {
            cli_output::discarded_partial_response(role, retry);
        }
    }
}

enum CredentialForStream {
    Ready(StreamCredential),
    Pending(
        futures_util::future::BoxFuture<'static, Result<Option<String>, CredentialSourceError>>,
    ),
    PendingAccountBound(
        futures_util::future::BoxFuture<
            'static,
            Result<AccountBoundCredential, CredentialSourceError>,
        >,
    ),
}

#[derive(Clone)]
struct StreamCredential {
    bearer: String,
    authorization: HeaderValue,
    account_id: Option<HeaderValue>,
}

async fn prepared_credential(
    credential: &Arc<Mutex<Option<CredentialForStream>>>,
    credential_reference: &str,
) -> Result<StreamCredential, ModelProviderError> {
    let unresolved_credential = credential
        .lock()
        .map_err(|_| ModelProviderError::CredentialSourceFailure {
            credential_ref: credential_reference.to_owned(),
            message: "prepared credential state is unavailable".to_owned(),
        })?
        .take()
        .ok_or_else(|| ModelProviderError::CredentialSourceFailure {
            credential_ref: credential_reference.to_owned(),
            message: "prepared credential is already in use".to_owned(),
        })?;
    let resolved_credential = match unresolved_credential {
        CredentialForStream::Ready(credential) => credential,
        CredentialForStream::Pending(resolution) => {
            validate_credential(resolution.await, credential_reference)?
        }
        CredentialForStream::PendingAccountBound(resolution) => {
            validate_account_bound_credential(resolution.await, credential_reference)?
        }
    };
    *credential
        .lock()
        .map_err(|_| ModelProviderError::CredentialSourceFailure {
            credential_ref: credential_reference.to_owned(),
            message: "prepared credential state is unavailable".to_owned(),
        })? = Some(CredentialForStream::Ready(resolved_credential.clone()));
    Ok(resolved_credential)
}

fn validate_credential(
    result: Result<Option<String>, CredentialSourceError>,
    credential_reference: &str,
) -> Result<StreamCredential, ModelProviderError> {
    let credential = result
        .map_err(|error| ModelProviderError::CredentialSourceFailure {
            credential_ref: credential_reference.to_owned(),
            message: error.to_string(),
        })?
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| ModelProviderError::CredentialUnavailable {
            credential_ref: credential_reference.to_owned(),
        })?;
    let mut authorization =
        HeaderValue::from_str(&format!("Bearer {credential}")).map_err(|_| {
            ModelProviderError::CredentialInvalid {
                credential_ref: credential_reference.to_owned(),
            }
        })?;
    authorization.set_sensitive(true);
    Ok(StreamCredential {
        bearer: credential,
        authorization,
        account_id: None,
    })
}

fn validate_account_bound_credential(
    result: Result<AccountBoundCredential, CredentialSourceError>,
    credential_reference: &str,
) -> Result<StreamCredential, ModelProviderError> {
    let (bearer, account_id) = result
        .map_err(|error| ModelProviderError::CredentialSourceFailure {
            credential_ref: credential_reference.to_owned(),
            message: error.to_string(),
        })?
        .into_parts();
    let mut credential = validate_credential(Ok(Some(bearer)), credential_reference)?;
    if account_id.is_empty() || account_id.len() > MAX_CHATGPT_ACCOUNT_ID_BYTES {
        return Err(ModelProviderError::CredentialInvalid {
            credential_ref: credential_reference.to_owned(),
        });
    }
    let mut account_id =
        HeaderValue::from_str(&account_id).map_err(|_| ModelProviderError::CredentialInvalid {
            credential_ref: credential_reference.to_owned(),
        })?;
    account_id.set_sensitive(true);
    credential.account_id = Some(account_id);
    Ok(credential)
}

#[derive(Default)]
struct OpaqueContinuationStore {
    items: Mutex<BTreeMap<OpaqueContinuationReference, request::SubscriptionOpaqueReasoning>>,
}

impl OpaqueContinuationStore {
    fn references_in(
        input: &crate::domain::model::ModelInputIR,
    ) -> BTreeSet<OpaqueContinuationReference> {
        input
            .content_items
            .iter()
            .filter_map(|item| match item {
                ContentItem::OpaqueContinuation { reference } => Some(reference.clone()),
                _ => None,
            })
            .collect()
    }

    fn resolve(
        &self,
        reference: &OpaqueContinuationReference,
        scope: &OpaqueContinuationScope,
    ) -> Result<request::SubscriptionOpaqueReasoning, ModelProviderError> {
        if reference.scope() != scope {
            return Err(request_encoding_error(
                "opaque provider continuation belongs to another delivery session",
            ));
        }
        let items = self.lock_items()?;
        let item = items.get(reference).ok_or_else(|| {
            request_encoding_error("opaque provider continuation is unavailable in this Adapter")
        })?;
        if !item.validates_reference(reference)? {
            return Err(request_encoding_error(
                "opaque provider continuation identity does not match retained content",
            ));
        }
        Ok(item.clone())
    }

    fn stage(
        &self,
        retained: &BTreeSet<OpaqueContinuationReference>,
        staged: &mut BTreeMap<OpaqueContinuationReference, request::SubscriptionOpaqueReasoning>,
        reference: OpaqueContinuationReference,
        item: request::SubscriptionOpaqueReasoning,
    ) -> Result<(), ModelProviderError> {
        if let Some(prior) = staged.get(&reference) {
            if prior != &item {
                return Err(ModelProviderError::ProtocolFailure {
                    message: "subscription reasoning reference changed within one response"
                        .to_owned(),
                });
            }
            return Ok(());
        }
        let mut candidate = staged.clone();
        candidate.insert(reference, item);
        let items = self.lock_items()?;
        Self::next_items(&items, retained, candidate.clone())?;
        *staged = candidate;
        Ok(())
    }

    fn commit(
        &self,
        retained: &BTreeSet<OpaqueContinuationReference>,
        staged: BTreeMap<OpaqueContinuationReference, request::SubscriptionOpaqueReasoning>,
    ) -> Result<(), ModelProviderError> {
        let mut items = self.lock_items()?;
        let next = Self::next_items(&items, retained, staged)?;
        *items = next;
        Ok(())
    }

    fn lock_items(
        &self,
    ) -> Result<
        MutexGuard<'_, BTreeMap<OpaqueContinuationReference, request::SubscriptionOpaqueReasoning>>,
        ModelProviderError,
    > {
        self.items
            .lock()
            .map_err(|_| ModelProviderError::ProtocolFailure {
                message: "subscription opaque continuation store lock is poisoned".to_owned(),
            })
    }

    fn next_items(
        items: &BTreeMap<OpaqueContinuationReference, request::SubscriptionOpaqueReasoning>,
        retained: &BTreeSet<OpaqueContinuationReference>,
        staged: BTreeMap<OpaqueContinuationReference, request::SubscriptionOpaqueReasoning>,
    ) -> Result<
        BTreeMap<OpaqueContinuationReference, request::SubscriptionOpaqueReasoning>,
        ModelProviderError,
    > {
        let mut next = items
            .iter()
            .filter(|(reference, _)| retained.contains(*reference))
            .map(|(reference, item)| (reference.clone(), item.clone()))
            .collect::<BTreeMap<_, _>>();
        for (reference, item) in staged {
            if !item.validates_reference(&reference)? {
                return Err(request_encoding_error(
                    "opaque provider continuation identity does not match completed content",
                ));
            }
            next.insert(reference, item);
        }
        if next.len() > MAX_OPAQUE_CONTINUATION_ITEMS {
            return Err(ModelProviderError::ProtocolFailure {
                message: "subscription opaque continuation store reached its bounded item limit"
                    .to_owned(),
            });
        }
        let encoded_bytes = next.values().try_fold(0usize, |total, item| {
            item.encoded_byte_length()
                .map(|item_bytes| total.saturating_add(item_bytes))
        })?;
        if encoded_bytes > MAX_OPAQUE_CONTINUATION_STORE_BYTES {
            return Err(ModelProviderError::ProtocolFailure {
                message: "subscription opaque continuation store reached its bounded byte limit"
                    .to_owned(),
            });
        }
        Ok(next)
    }
}

async fn read_bounded_http_error_body(
    mut response: reqwest::Response,
) -> response::BoundedHttpErrorBody {
    let mut body = Vec::new();
    loop {
        let chunk = match response.chunk().await {
            Ok(chunk) => chunk,
            Err(_) => return response::BoundedHttpErrorBody::Unreadable,
        };
        let Some(chunk) = chunk else {
            break;
        };
        if chunk.len() > MAX_HTTP_ERROR_BODY_BYTES.saturating_sub(body.len()) {
            return response::BoundedHttpErrorBody::Oversized;
        }
        body.extend_from_slice(&chunk);
    }
    match String::from_utf8(body) {
        Ok(body) => response::BoundedHttpErrorBody::Complete(body),
        Err(_) => response::BoundedHttpErrorBody::Unreadable,
    }
}
