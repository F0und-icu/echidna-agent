//! Explicit subscription model metadata, independent of OAuth token lifecycle.
use super::oauth::SubscriptionAuthService;
use super::types::{
    SubscriptionAuthError, SubscriptionAuthHttpOperation, SubscriptionAuthProtocol,
    auth_http_failure, classify_http_status,
};
use crate::model_catalog::{CatalogModel, CatalogReasoningLevel};
use serde::Deserialize;
use std::collections::BTreeSet;
const ECHIDNA_USER_AGENT: &str = concat!("echidna/", env!("CARGO_PKG_VERSION"));

pub(crate) struct SubscriptionCatalog {
    auth: SubscriptionAuthService,
    http: reqwest::Client,
    api_base_url: String,
    protocol: SubscriptionAuthProtocol,
    catalog_client_version: String,
}
impl SubscriptionCatalog {
    pub(crate) fn protocol(&self) -> SubscriptionAuthProtocol {
        self.protocol
    }

    pub(super) fn new(
        auth: SubscriptionAuthService,
        http: reqwest::Client,
        api_base_url: String,
        protocol: SubscriptionAuthProtocol,
        catalog_client_version: String,
    ) -> Self {
        Self {
            auth,
            http,
            api_base_url,
            protocol,
            catalog_client_version,
        }
    }
    /// Performs the explicit, account-scoped model discovery path after
    /// obtaining a usable subscription bearer.
    pub(crate) async fn models(&self) -> Result<Vec<CatalogModel>, SubscriptionAuthError> {
        if self.protocol == SubscriptionAuthProtocol::Codex {
            return self.codex_models().await;
        }
        let token = self.auth.access_token().await?;
        let response = self
            .http
            .get(format!(
                "{}/models",
                self.api_base_url.as_str().trim_end_matches('/')
            ))
            .bearer_auth(token)
            .send()
            .await
            .map_err(|_| SubscriptionAuthError::TemporarilyUnavailable)?;
        if !response.status().is_success() {
            return Err(classify_http_status(response.status()));
        }
        let models: SiwcModelsResponse = response
            .json()
            .await
            .map_err(|_| SubscriptionAuthError::InvalidResponse)?;
        siwc_subscription_models(
            models
                .models
                .into_iter()
                .filter(|model| model.visibility == "list")
                .collect(),
        )
    }

    async fn codex_models(&self) -> Result<Vec<CatalogModel>, SubscriptionAuthError> {
        let credential = self.auth.access_codex_credential().await?;
        let (bearer, account_id) = credential.into_parts();
        let response = self
            .http
            .get(format!(
                "{}/models",
                self.api_base_url.as_str().trim_end_matches('/')
            ))
            .query(&[("client_version", self.catalog_client_version.as_str())])
            .bearer_auth(bearer)
            .header("ChatGPT-Account-ID", account_id)
            .header("version", self.catalog_client_version.as_str())
            .header("User-Agent", ECHIDNA_USER_AGENT)
            .send()
            .await
            .map_err(|_| SubscriptionAuthError::TemporarilyUnavailable)?;
        if !response.status().is_success() {
            return Err(auth_http_failure(
                SubscriptionAuthHttpOperation::ModelCatalog,
                response.status(),
            ));
        }
        let models: CodexModelsResponse = response
            .json()
            .await
            .map_err(|_| SubscriptionAuthError::InvalidResponse)?;
        codex_subscription_models(models.models)
    }
}

#[derive(Debug, Deserialize)]
struct SiwcModelsResponse {
    models: Vec<SiwcModelResponse>,
}

#[derive(Debug, Deserialize)]
struct SiwcModelResponse {
    slug: String,
    display_name: String,
    visibility: String,
}

#[derive(Debug, Deserialize)]
struct CodexModelsResponse {
    models: Vec<ModelResponse>,
}

#[derive(Debug, Deserialize)]
struct ModelResponse {
    slug: String,
    display_name: String,
    #[serde(default)]
    default_reasoning_level: Option<String>,
    #[serde(default)]
    supported_reasoning_levels: Vec<ReasoningLevelResponse>,
    #[serde(default, deserialize_with = "optional_positive_u32")]
    context_window: Option<u32>,
    #[serde(default, deserialize_with = "optional_positive_u32")]
    max_context_window: Option<u32>,
    #[serde(default, deserialize_with = "optional_modalities")]
    input_modalities: Option<Vec<String>>,
    #[serde(default, deserialize_with = "optional_bool")]
    supports_reasoning_summary_parameter: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct ReasoningLevelResponse {
    effort: String,
    description: String,
}

fn siwc_subscription_models(
    models: Vec<SiwcModelResponse>,
) -> Result<Vec<CatalogModel>, SubscriptionAuthError> {
    if models.iter().any(|model| {
        model.slug.is_empty()
            || model.slug.len() > 256
            || model.display_name.is_empty()
            || model.display_name.len() > 256
    }) {
        return Err(SubscriptionAuthError::InvalidResponse);
    }
    Ok(models
        .into_iter()
        .map(|model| CatalogModel {
            slug: model.slug,
            display_name: model.display_name,
            default_reasoning_level: None,
            supported_reasoning_levels: Vec::new(),
            context_window: None,
            max_context_window: None,
            input_modalities: None,
            supports_reasoning_summary_parameter: None,
            reasoning_levels_advertised: true,
        })
        .collect())
}

fn codex_subscription_models(
    models: Vec<ModelResponse>,
) -> Result<Vec<CatalogModel>, SubscriptionAuthError> {
    let mut slugs = BTreeSet::new();
    for model in &models {
        if model.slug.is_empty()
            || model.slug.len() > 256
            || model.display_name.is_empty()
            || model.display_name.len() > 256
            || !slugs.insert(&model.slug)
        {
            return Err(SubscriptionAuthError::InvalidResponse);
        }
        if let Some(default) = &model.default_reasoning_level
            && !valid_reasoning_metadata(default, 64)
        {
            return Err(SubscriptionAuthError::InvalidResponse);
        }
        let mut efforts = BTreeSet::new();
        for level in &model.supported_reasoning_levels {
            if !valid_reasoning_metadata(&level.effort, 64)
                || level.description.is_empty()
                || level.description.len() > 512
                || !efforts.insert(&level.effort)
            {
                return Err(SubscriptionAuthError::InvalidResponse);
            }
        }
        if model.input_modalities.as_ref().is_some_and(|modalities| {
            modalities.len() > 32
                || modalities
                    .iter()
                    .any(|value| !valid_reasoning_metadata(value, 64))
        }) {
            return Err(SubscriptionAuthError::InvalidResponse);
        }
    }
    Ok(models
        .into_iter()
        .map(|model| CatalogModel {
            slug: model.slug,
            display_name: model.display_name,
            default_reasoning_level: model.default_reasoning_level,
            supported_reasoning_levels: model
                .supported_reasoning_levels
                .into_iter()
                .map(|level| CatalogReasoningLevel {
                    effort: level.effort,
                    description: level.description,
                })
                .collect(),
            context_window: model.context_window,
            max_context_window: model.max_context_window,
            input_modalities: model.input_modalities,
            supports_reasoning_summary_parameter: model.supports_reasoning_summary_parameter,
            reasoning_levels_advertised: true,
        })
        .collect())
}

fn optional_positive_u32<'de, D>(deserializer: D) -> Result<Option<u32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    match Option::<u32>::deserialize(deserializer)? {
        Some(value) if value > 0 => Ok(Some(value)),
        Some(_) => Err(serde::de::Error::custom(
            "expected a positive unsigned integer",
        )),
        None => Ok(None),
    }
}

fn optional_modalities<'de, D>(deserializer: D) -> Result<Option<Vec<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let values = Vec::<String>::deserialize(deserializer)?;
    Ok(Some(values))
}

fn optional_bool<'de, D>(deserializer: D) -> Result<Option<bool>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    bool::deserialize(deserializer).map(Some)
}

fn valid_reasoning_metadata(value: &str, maximum_length: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum_length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}
