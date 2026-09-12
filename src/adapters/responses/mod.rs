//! One provider-neutral Responses HTTP/SSE implementation selected entirely
//! by explicit configuration.

mod cli_output;
mod request;
mod response;

use std::sync::Arc;
use std::time::Duration;

use async_stream::try_stream;
use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderValue};

use crate::adapters::credentials::CredentialSource;
use crate::domain::artifacts::ArtifactId;
use crate::domain::model::ModelCapabilities;
use crate::domain::request_audit::RequestAdapterVersion;
use crate::domain::research::ResearchState;
use crate::ports::artifact::ArtifactStore;
use crate::ports::model::{
    ModelEventStream, ModelProvider, ModelProviderError, ModelRequest, ModelStreamEvent,
};

pub(crate) const DEFAULT_CONNECT_TIMEOUT_MS: u64 = 30_000;
pub(crate) const DEFAULT_READ_TIMEOUT_MS: u64 = 300_000;
#[cfg(debug_assertions)]
const MAX_DEBUG_ERROR_BODY_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResponsesHttpConfig {
    base_url: String,
    model: String,
    credential_reference: String,
    capabilities: ModelCapabilities,
    connect_timeout: Duration,
    read_timeout: Duration,
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
        }
    }

    pub(crate) fn with_timeouts(mut self, connect_ms: u64, read_ms: u64) -> Self {
        self.connect_timeout = Duration::from_millis(connect_ms);
        self.read_timeout = Duration::from_millis(read_ms);
        self
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
        RequestAdapterVersion::new("responses-http-sse-v1")
            .expect("the fixed Responses HTTP/SSE Adapter version is non-empty")
    }

    pub fn new(config: ResponsesHttpConfig, credential_source: Arc<dyn CredentialSource>) -> Self {
        Self {
            client: reqwest::Client::builder()
                .connect_timeout(config.connect_timeout)
                .read_timeout(config.read_timeout)
                .build()
                .expect("Responses HTTP client initialization failed"),
            config,
            credential_source,
            image_resolver: None,
            cli_role: None,
            #[cfg(debug_assertions)]
            debug: false,
        }
    }

    pub fn with_project_image_resolver(
        mut self,
        project: crate::adapters::project_event_store::Project,
    ) -> Self {
        self.image_resolver = Some(Arc::new(move |artifact_id| {
            resolve_project_image_artifact(&project, artifact_id)
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

fn resolve_project_image_artifact(
    project: &crate::adapters::project_event_store::Project,
    artifact_id: &ArtifactId,
) -> Result<(String, Vec<u8>), ModelProviderError> {
    let state: ResearchState = crate::adapters::research_projection::load_research_state(project)
        .map_err(request_encoding_error)?;
    let record = state.artifact(artifact_id).ok_or_else(|| {
        request_encoding_error(format!(
            "image Artifact `{artifact_id}` is not present in the selected project"
        ))
    })?;
    if !matches!(record.media_type.as_str(), "image/png" | "image/jpeg") {
        return Err(request_encoding_error(format!(
            "Artifact `{artifact_id}` media type `{}` is not PNG or JPEG",
            record.media_type
        )));
    }
    let bytes = crate::adapters::artifact_store::LocalArtifactStore::for_project(project)
        .read_blob(&record.blob_id)
        .map_err(request_encoding_error)?;
    if u64::try_from(bytes.len()).expect("usize always fits u64") != record.size_bytes {
        return Err(request_encoding_error(format!(
            "Artifact `{artifact_id}` size does not match its immutable Blob"
        )));
    }
    Ok((record.media_type.clone(), bytes))
}

fn request_encoding_error(error: impl ToString) -> ModelProviderError {
    ModelProviderError::RequestEncodingFailure {
        message: error.to_string(),
    }
}

impl ModelProvider for ResponsesHttpAdapter {
    fn capabilities(&self) -> ModelCapabilities {
        self.config.capabilities.clone()
    }

    fn stream(&self, request: ModelRequest) -> Result<ModelEventStream, ModelProviderError> {
        request.validate_capabilities(&self.config.capabilities)?;

        let credential_reference = self.config.credential_reference.clone();
        let api_key = self
            .credential_source
            .read(&credential_reference)
            .map_err(|error| ModelProviderError::CredentialSourceFailure {
                credential_ref: credential_reference.clone(),
                message: error.to_string(),
            })?
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| ModelProviderError::CredentialUnavailable {
                credential_ref: credential_reference.clone(),
            })?;
        let authorization = HeaderValue::from_str(&format!("Bearer {api_key}")).map_err(|_| {
            ModelProviderError::CredentialInvalid {
                credential_ref: credential_reference,
            }
        })?;

        let usage_required = request.input().requirements.usage_reporting;
        let continuation_required = self.config.capabilities.incremental_input;
        let continuation_requested = request.is_incremental();
        let request = request::encode_request(
            request,
            self.config.model.clone(),
            self.image_resolver.as_deref(),
        )?;
        let endpoint = self.config.endpoint();
        let client = self.client.clone();
        let cli_role = self.cli_role;
        #[cfg(debug_assertions)]
        let full_debug = self.debug;

        let stream = try_stream! {
            let mut diagnostics = cli_role.map(|role| cli_output::RequestOutput::new(role, api_key,
                #[cfg(debug_assertions)]
                full_debug,
            ));
            #[cfg(debug_assertions)]
            if let Some(diagnostics) = &diagnostics {
                diagnostics.request(&request);
            }
            let response = client
                .post(endpoint)
                .header(ACCEPT, "text/event-stream")
                .header(AUTHORIZATION, authorization)
                .json(&request)
                .send()
                .await
                .map_err(|error| ModelProviderError::TransportFailure {
                    message: error.to_string(),
                })?;

            let status = response.status();
            #[cfg(debug_assertions)]
            if let Some(diagnostics) = &diagnostics {
                diagnostics.status(status.as_u16());
            }
            let mut response_events = if status.is_success() {
                response.bytes_stream().eventsource()
            } else {
                if continuation_requested {
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
                } else {
                    #[cfg(debug_assertions)]
                    if full_debug {
                        let body = read_debug_error_body(response).await;
                        if let Some(diagnostics) = &diagnostics {
                            match &body {
                                Ok(body) => diagnostics.http_error_body(body),
                                Err(reason) => diagnostics.response_omitted(reason),
                            }
                        }
                    }
                }
                Err(ModelProviderError::HttpFailure {
                    status: status.as_u16(),
                })?;
                unreachable!("an HTTP failure is returned through the stream")
            };
            loop {
                let event = response_events
                    .next()
                    .await
                    .ok_or_else(|| ModelProviderError::ProtocolFailure {
                        message: "response stream ended before `response.completed`".to_string(),
                    })?
                    .map_err(|error| ModelProviderError::ProtocolFailure {
                        message: format!("invalid SSE stream: {error}"),
                    })?;

                if event.data.trim().is_empty() {
                    continue;
                }

                #[cfg(debug_assertions)]
                if let Some(diagnostics) = &diagnostics {
                    diagnostics.response(&event.data);
                }

                let decoded = response::decode_stream_event(
                    &event.data,
                    usage_required,
                    continuation_required,
                    continuation_requested,
                )?;
                for model_event in decoded.events {
                    if let Some(diagnostics) = &mut diagnostics {
                        diagnostics.model_event(&model_event);
                    }
                    yield ModelStreamEvent::Event(model_event);
                }
                if let Some(completion) = decoded.completion {
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
}

/// Bound only the extra diagnostic read; existing continuation classification
/// retains its complete-body behavior independently of presentation.
#[cfg(debug_assertions)]
async fn read_debug_error_body(mut response: reqwest::Response) -> Result<String, &'static str> {
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "HTTP error body unavailable")?
    {
        if chunk.len() > MAX_DEBUG_ERROR_BODY_BYTES - body.len() {
            return Err("HTTP error body omitted: exceeds 65536 bytes");
        }
        body.extend_from_slice(&chunk);
    }
    String::from_utf8(body).map_err(|_| "HTTP error body is not UTF-8")
}
