//! Minimal OpenAI Responses HTTP Streaming Adapter.

mod credentials;
mod request;
mod response;

use std::sync::Arc;

use async_stream::try_stream;
use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderValue};

pub use credentials::{
    OPENAI_API_KEY_REFERENCE, OpenAiCredentialSource, OpenAiCredentialSourceError,
    ProcessEnvironmentCredentialSource,
};

use crate::domain::artifacts::ArtifactId;
use crate::domain::model::ModelCapabilities;
use crate::domain::research::{ResearchReducer, ResearchState};
use crate::ports::artifact::ArtifactStore;
use crate::ports::model::{
    ModelEventStream, ModelProvider, ModelProviderError, ModelRequest, ModelStreamEvent,
};

/// Configuration explicitly selected for one OpenAI Responses Provider.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenAiResponsesConfig {
    base_url: String,
    model: String,
    capabilities: ModelCapabilities,
}

impl OpenAiResponsesConfig {
    /// Creates an OpenAI Responses configuration.
    pub fn new(
        base_url: impl Into<String>,
        model: impl Into<String>,
        capabilities: ModelCapabilities,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            model: model.into(),
            capabilities,
        }
    }

    fn endpoint(&self) -> String {
        format!("{}/responses", self.base_url.trim_end_matches('/'))
    }
}

/// OpenAI Responses implementation of the provider-neutral Model Port.
#[derive(Clone)]
pub struct OpenAiResponsesAdapter {
    client: reqwest::Client,
    config: OpenAiResponsesConfig,
    credential_source: Arc<dyn OpenAiCredentialSource>,
    image_resolver: Option<Arc<ImageArtifactResolver>>,
}

/// Adapter-private erased resolver. It is deliberately a constructor input,
/// not a second public Blob Port: Artifact ownership remains with the local
/// Artifact Store and the composed application decides which immutable
/// Artifact IDs are resolvable for a Provider request.
type ImageArtifactResolver =
    dyn Fn(&ArtifactId) -> Result<(String, Vec<u8>), ModelProviderError> + Send + Sync;

impl OpenAiResponsesAdapter {
    /// Builds an Adapter with an injected credential source.
    pub fn new(
        config: OpenAiResponsesConfig,
        credential_source: Arc<dyn OpenAiCredentialSource>,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            config,
            credential_source,
            image_resolver: None,
        }
    }

    /// Supplies the composed local Artifact lookup used only while encoding an
    /// OpenAI wire request. The resolver must return the media type and exact
    /// immutable Blob bytes for the requested Artifact ID.
    fn with_image_resolver(
        mut self,
        resolver: impl Fn(&ArtifactId) -> Result<(String, Vec<u8>), ModelProviderError>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.image_resolver = Some(Arc::new(resolver));
        self
    }

    /// Resolves image bytes only from strict, project-isolated Artifact facts.
    /// The Research stream identifies the Artifact and media type; the
    /// existing SHA-256 Blob Store verifies the immutable bytes.
    pub fn with_project_image_resolver(
        self,
        project: crate::adapters::project_event_store::Project,
    ) -> Self {
        self.with_image_resolver(move |artifact_id| {
            resolve_project_image_artifact(&project, artifact_id)
        })
    }
}

fn resolve_project_image_artifact(
    project: &crate::adapters::project_event_store::Project,
    artifact_id: &ArtifactId,
) -> Result<(String, Vec<u8>), ModelProviderError> {
    let events = crate::adapters::project_event_store::read_existing_research_events(project)
        .map_err(request_encoding_error)?;
    let state: ResearchState = ResearchReducer
        .reduce(events)
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

impl ModelProvider for OpenAiResponsesAdapter {
    fn capabilities(&self) -> ModelCapabilities {
        self.config.capabilities.clone()
    }

    fn stream(&self, request: ModelRequest) -> Result<ModelEventStream, ModelProviderError> {
        request.validate_capabilities(&self.config.capabilities)?;

        let api_key = self
            .credential_source
            .read(OPENAI_API_KEY_REFERENCE)
            .map_err(|error| ModelProviderError::CredentialSourceFailure {
                credential_ref: OPENAI_API_KEY_REFERENCE.to_string(),
                message: error.to_string(),
            })?
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| ModelProviderError::CredentialUnavailable {
                credential_ref: OPENAI_API_KEY_REFERENCE.to_string(),
            })?;
        let authorization = HeaderValue::from_str(&format!("Bearer {api_key}")).map_err(|_| {
            ModelProviderError::CredentialInvalid {
                credential_ref: OPENAI_API_KEY_REFERENCE.to_string(),
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

        let stream = try_stream! {
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
            let mut response_events = if status.is_success() {
                response.bytes_stream().eventsource()
            } else {
                if continuation_requested {
                    let body = response.text().await.unwrap_or_default();
                    if response::continuation_rejected_http_body(&body) {
                        Err(ModelProviderError::ContinuationRejected {
                            code: "previous_response_not_found".to_owned(),
                        })?;
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

                let decoded = response::decode_stream_event(
                    &event.data,
                    usage_required,
                    continuation_required,
                    continuation_requested,
                )?;
                for model_event in decoded.events {
                    yield ModelStreamEvent::Event(model_event);
                }
                if let Some(completion) = decoded.completion {
                    yield ModelStreamEvent::Completed(completion);
                    break;
                }
            }
        };

        Ok(Box::pin(stream))
    }
}
