//! Bounded standard API-key GET /models. This is an explicit CLI operation;
//! inference never discovers models or changes its declared capabilities.

use std::collections::BTreeSet;

use reqwest::header::{ACCEPT, AUTHORIZATION};
use serde::Deserialize;
use thiserror::Error;

use crate::domain::model::ReasoningEffort;
use crate::model_catalog::{CatalogModel, CatalogReasoningLevel};

use super::{ResponsesHttpAdapter, resolve_pending_credential, source_io_kind};

const MAX_CATALOG_BYTES: usize = 1024 * 1024;
const MAX_CATALOG_MODELS: usize = 4096;

#[derive(Debug, Error)]
pub(crate) enum CatalogError {
    #[error("model catalog credential is unavailable or invalid")]
    Credential,
    #[error("model catalog transport failed ({phase}; timeout={timeout}; io_kind={io_kind})")]
    Transport {
        phase: &'static str,
        timeout: bool,
        io_kind: String,
    },
    #[error("model catalog request failed with HTTP {status}")]
    Http { status: u16 },
    #[error("model catalog exceeds its byte or model-count limit")]
    TooLarge,
    #[error("model catalog response is malformed")]
    Malformed,
    #[error("model catalog is empty")]
    Empty,
    #[error("model catalog contains duplicate model IDs")]
    Duplicate,
}

impl ResponsesHttpAdapter {
    pub(crate) async fn models(&self) -> Result<Vec<CatalogModel>, CatalogError> {
        let credential = self
            .resolve_credential()
            .map_err(|_| CatalogError::Credential)?;
        let credential = resolve_pending_credential(credential, &self.config.credential_reference)
            .await
            .map_err(|_| CatalogError::Credential)?;
        let mut response = self
            .client
            .get(format!(
                "{}/models",
                self.config.base_url.trim_end_matches('/')
            ))
            .header(ACCEPT, "application/json")
            .header(AUTHORIZATION, credential.authorization)
            .send()
            .await
            .map_err(|error| transport_error("request", &error))?;
        if !response.status().is_success() {
            // Status is enough to distinguish authentication, authorization,
            // rate limiting and unsupported endpoints without retaining a body.
            return Err(CatalogError::Http {
                status: response.status().as_u16(),
            });
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_CATALOG_BYTES as u64)
        {
            return Err(CatalogError::TooLarge);
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| transport_error("body", &error))?
        {
            if chunk.len() > MAX_CATALOG_BYTES.saturating_sub(body.len()) {
                return Err(CatalogError::TooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        decode_catalog(&body)
    }
}

fn transport_error(phase: &'static str, error: &reqwest::Error) -> CatalogError {
    CatalogError::Transport {
        phase,
        timeout: error.is_timeout(),
        io_kind: source_io_kind(error),
    }
}

#[derive(Deserialize)]
struct CatalogResponse {
    data: Vec<Model>,
}

#[derive(Deserialize)]
struct Model {
    id: String,
    name: Option<String>,
    context_window: Option<u32>,
    input_modalities: Option<Vec<String>>,
    effort: Option<Effort>,
}

#[derive(Deserialize)]
struct Effort {
    supported_levels: Vec<String>,
    default_level: Option<String>,
}

fn decode_catalog(body: &[u8]) -> Result<Vec<CatalogModel>, CatalogError> {
    let response: CatalogResponse =
        serde_json::from_slice(body).map_err(|_| CatalogError::Malformed)?;
    if response.data.is_empty() {
        return Err(CatalogError::Empty);
    }
    if response.data.len() > MAX_CATALOG_MODELS {
        return Err(CatalogError::TooLarge);
    }
    let mut ids = BTreeSet::new();
    let mut models = Vec::with_capacity(response.data.len());
    for model in response.data {
        if !valid_metadata(&model.id, 256)
            || model.id.chars().any(char::is_whitespace)
            || model
                .name
                .as_ref()
                .is_some_and(|name| !valid_metadata(name, 256))
            || model.context_window == Some(0)
        {
            return Err(CatalogError::Malformed);
        }
        if !ids.insert(model.id.clone()) {
            return Err(CatalogError::Duplicate);
        }
        if let Some(modalities) = &model.input_modalities {
            let unique: BTreeSet<_> = modalities.iter().collect();
            if modalities.is_empty()
                || modalities.len() > 32
                || unique.len() != modalities.len()
                || modalities.iter().any(|value| !valid_metadata(value, 64))
            {
                return Err(CatalogError::Malformed);
            }
        }
        let advertised = model.effort.is_some();
        let (levels, default_level) = match model.effort {
            Some(effort) => {
                let unique: BTreeSet<_> = effort.supported_levels.iter().collect();
                if effort.supported_levels.len() > 64
                    || unique.len() != effort.supported_levels.len()
                    || effort
                        .supported_levels
                        .iter()
                        .any(|level| level.parse::<ReasoningEffort>().is_err())
                    || effort
                        .default_level
                        .as_ref()
                        .is_some_and(|level| !unique.contains(level))
                {
                    return Err(CatalogError::Malformed);
                }
                (effort.supported_levels, effort.default_level)
            }
            None => (Vec::new(), None),
        };
        models.push(CatalogModel {
            display_name: model.name.unwrap_or_else(|| model.id.clone()),
            slug: model.id,
            default_reasoning_level: default_level,
            supported_reasoning_levels: levels
                .into_iter()
                .map(|effort| CatalogReasoningLevel {
                    effort,
                    description: String::new(),
                })
                .collect(),
            context_window: model.context_window,
            max_context_window: None,
            input_modalities: model.input_modalities,
            supports_reasoning_summary_parameter: None,
            reasoning_levels_advertised: advertised,
        });
    }
    Ok(models)
}

fn valid_metadata(value: &str, max_bytes: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max_bytes && !value.chars().any(char::is_control)
}
