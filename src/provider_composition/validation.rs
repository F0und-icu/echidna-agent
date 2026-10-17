use std::collections::BTreeMap;
use std::path::PathBuf;

use thiserror::Error;

use crate::domain::profile::AgentModelProfileError;

use super::config::{
    AgentProfileDefinition, CapabilityMappingDefinition, PROVIDER_CONFIG_SCHEMA_VERSION,
    ProviderConfigV1, ProviderDefinition,
};

impl ProviderConfigV1 {
    pub(super) fn validate(&self) -> Result<(), ProviderCompositionError> {
        if self.schema_version != PROVIDER_CONFIG_SCHEMA_VERSION {
            return Err(ProviderCompositionError::UnsupportedSchemaVersion {
                actual: self.schema_version,
                expected: PROVIDER_CONFIG_SCHEMA_VERSION,
            });
        }
        validate_identifier("active_agent_profile", &self.active_agent_profile)?;
        if !self.agent_profiles.contains_key(&self.active_agent_profile) {
            return Err(ProviderCompositionError::UnknownActiveAgentProfile {
                profile: self.active_agent_profile.clone(),
            });
        }
        self.prompts.validate()?;
        if self
            .agent_definition
            .as_ref()
            .is_some_and(|path| path.as_os_str().is_empty())
        {
            return Err(ProviderCompositionError::InvalidAgentDefinitionPath);
        }
        if !(1..=crate::app::model_request_budget::MAX_MODEL_REQUEST_LIMIT)
            .contains(&self.runtime.model_request_limit_per_turn)
        {
            return Err(ProviderCompositionError::InvalidModelRequestLimit);
        }

        for (name, mapping) in &self.capability_mappings {
            validate_identifier("capability_mapping", name)?;
            if mapping.version == 0 {
                return Err(ProviderCompositionError::ZeroGeneration {
                    kind: "capability_mapping",
                    name: name.clone(),
                });
            }
            if mapping.context_window == 0 {
                return Err(ProviderCompositionError::ZeroContextWindow {
                    mapping: name.clone(),
                });
            }
            crate::domain::context::ContextPlanner::current()
                .default_budget(&mapping.to_domain())
                .map_err(|source| ProviderCompositionError::InvalidContextBudget {
                    mapping: name.clone(),
                    source,
                })?;
        }
        for (name, provider) in &self.providers {
            validate_identifier("provider", name)?;
            provider.validate(name, &self.capability_mappings)?;
        }
        for (name, agent) in &self.agent_profiles {
            validate_identifier("agent_profile", name)?;
            agent.validate(name, &self.providers)?;
        }
        for name in self.agent_profiles.keys() {
            self.agent_model_profile(name)?;
        }
        Ok(())
    }
}

impl ProviderDefinition {
    fn validate(
        &self,
        name: &str,
        mappings: &BTreeMap<String, CapabilityMappingDefinition>,
    ) -> Result<(), ProviderCompositionError> {
        validate_non_empty("provider.base_url", &self.base_url)?;
        validate_non_empty("provider.model", &self.model)?;
        for (field, value) in [
            ("provider.connect_timeout_ms", self.connect_timeout_ms),
            ("provider.read_timeout_ms", self.read_timeout_ms),
        ] {
            if value == 0 || value > 86_400_000 {
                return Err(ProviderCompositionError::InvalidTimeout { field });
            }
        }
        validate_identifier("provider.capability_mapping", &self.capability_mapping)?;
        if self.generation == 0 {
            return Err(ProviderCompositionError::ZeroGeneration {
                kind: "provider",
                name: name.to_owned(),
            });
        }
        if !mappings.contains_key(&self.capability_mapping) {
            return Err(ProviderCompositionError::UnknownCapabilityMapping {
                provider: name.to_owned(),
                mapping: self.capability_mapping.clone(),
            });
        }
        validate_base_url(name, &self.base_url)?;
        match (
            self.credential_ref.as_deref(),
            self.api_key.as_ref(),
            self.chatgpt_subscription.as_ref(),
            self.codex_subscription.as_ref(),
        ) {
            (Some(reference), None, None, None) => validate_credential_reference(name, reference),
            (None, Some(api_key), None, None) if !api_key.expose().trim().is_empty() => Ok(()),
            (None, Some(_), None, None) => Err(ProviderCompositionError::EmptyInlineApiKey {
                provider: name.to_owned(),
            }),
            (None, None, Some(subscription), None) => {
                // The authentication owner validates label/path syntax. This
                // constructor is pure: doctor never touches the private store.
                super::subscription_service(subscription)?;
                if self.base_url.trim_end_matches('/')
                    != crate::adapters::subscription_auth::API_BASE_URL
                {
                    return Err(ProviderCompositionError::InvalidSubscription {
                        provider: name.to_owned(),
                        message: "subscription credentials require the official public HTTPS API audience".to_owned(),
                    });
                }
                let mapping = mappings[&self.capability_mapping].to_domain();
                if mapping.incremental_input || mapping.max_output_tokens.is_some() {
                    return Err(ProviderCompositionError::InvalidSubscription {
                        provider: name.to_owned(),
                        message: "subscription HTTP requires incremental_input=false and no max_output_tokens".to_owned(),
                    });
                }
                Ok(())
            }
            (None, None, None, Some(subscription)) => {
                // The authentication owner validates label/path syntax. This
                // constructor is pure: doctor never touches the private store.
                super::codex_subscription_service(subscription)?;
                if self.base_url.trim_end_matches('/')
                    != crate::adapters::subscription_auth::CODEX_API_BASE_URL
                {
                    return Err(ProviderCompositionError::InvalidSubscription {
                        provider: name.to_owned(),
                        message: "Codex subscription credentials require the fixed ChatGPT backend audience".to_owned(),
                    });
                }
                let mapping = mappings[&self.capability_mapping].to_domain();
                if mapping.incremental_input || mapping.max_output_tokens.is_some() {
                    return Err(ProviderCompositionError::InvalidSubscription {
                        provider: name.to_owned(),
                        message: "Codex subscription HTTP requires incremental_input=false and no max_output_tokens".to_owned(),
                    });
                }
                Ok(())
            }
            _ => Err(ProviderCompositionError::CredentialSelection {
                provider: name.to_owned(),
            }),
        }
    }
}

impl AgentProfileDefinition {
    fn validate(
        &self,
        name: &str,
        providers: &BTreeMap<String, ProviderDefinition>,
    ) -> Result<(), ProviderCompositionError> {
        validate_identifier("agent_profile.primary", &self.primary)?;
        if self.generation == 0 {
            return Err(ProviderCompositionError::ZeroGeneration {
                kind: "agent_profile",
                name: name.to_owned(),
            });
        }
        if !providers.contains_key(&self.primary) {
            return Err(ProviderCompositionError::UnknownProviderProfile {
                agent_profile: name.to_owned(),
                role: "primary",
                provider: self.primary.clone(),
            });
        }
        if let Some(vision) = self.vision.as_deref() {
            validate_identifier("agent_profile.vision", vision)?;
            if !providers.contains_key(vision) {
                return Err(ProviderCompositionError::UnknownProviderProfile {
                    agent_profile: name.to_owned(),
                    role: "vision",
                    provider: vision.to_owned(),
                });
            }
        }
        Ok(())
    }
}

fn validate_identifier(field: &'static str, value: &str) -> Result<(), ProviderCompositionError> {
    validate_non_empty(field, value)?;
    if value != value.trim()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err(ProviderCompositionError::InvalidIdentifier {
            field,
            value: value.to_owned(),
        });
    }
    Ok(())
}

fn validate_non_empty(field: &'static str, value: &str) -> Result<(), ProviderCompositionError> {
    if value.trim().is_empty() {
        return Err(ProviderCompositionError::RequiredValue { field });
    }
    Ok(())
}

fn validate_base_url(provider: &str, value: &str) -> Result<(), ProviderCompositionError> {
    let url =
        reqwest::Url::parse(value).map_err(|source| ProviderCompositionError::InvalidBaseUrl {
            provider: provider.to_owned(),
            message: source.to_string(),
        })?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ProviderCompositionError::InvalidBaseUrl {
            provider: provider.to_owned(),
            message: "expected an http(s) base URL without credentials, query, or fragment"
                .to_owned(),
        });
    }
    Ok(())
}

fn validate_credential_reference(
    provider: &str,
    reference: &str,
) -> Result<(), ProviderCompositionError> {
    if !reference.starts_with("CYBER_AGENT_")
        || !reference.ends_with("_API_KEY")
        || !reference
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Err(ProviderCompositionError::InvalidCredentialReference {
            provider: provider.to_owned(),
            reference: reference.to_owned(),
        });
    }
    Ok(())
}

#[derive(Debug, Error)]
pub enum ProviderCompositionError {
    #[error("capability mapping `{mapping}` has an invalid Context/output budget: {source}")]
    InvalidContextBudget {
        mapping: String,
        #[source]
        source: crate::domain::context::ContextPlannerError,
    },
    #[error("runtime.model_request_limit_per_turn must be between 1 and 10000")]
    InvalidModelRequestLimit,
    #[error("configuration field `{field}` must name a non-empty prompt file path")]
    InvalidPromptPath { field: &'static str },
    #[error(
        "configuration field `agent_definition` must name a non-empty definition directory or manifest path"
    )]
    InvalidAgentDefinitionPath,
    #[error("configuration field `{field}` must be between 1 and 86400000 milliseconds")]
    InvalidTimeout { field: &'static str },
    #[error("failed to read Provider configuration `{path}`: {source}")]
    ReadConfiguration {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to decode Provider configuration `{path}`: {message}")]
    DecodeConfiguration { path: PathBuf, message: String },
    #[error("unsupported Provider configuration schema {actual}; expected {expected}")]
    UnsupportedSchemaVersion { actual: u16, expected: u16 },
    #[error("configuration field `{field}` is required")]
    RequiredValue { field: &'static str },
    #[error("configuration field `{field}` has invalid identifier `{value}`")]
    InvalidIdentifier { field: &'static str, value: String },
    #[error("active Agent profile `{profile}` does not exist")]
    UnknownActiveAgentProfile { profile: String },
    #[error("{kind} `{name}` must have a non-zero generation")]
    ZeroGeneration { kind: &'static str, name: String },
    #[error("capability mapping `{mapping}` must declare a non-zero context window")]
    ZeroContextWindow { mapping: String },
    #[error("Provider `{provider}` references unknown capability mapping `{mapping}`")]
    UnknownCapabilityMapping { provider: String, mapping: String },
    #[error(
        "Agent profile `{agent_profile}` references unknown {role} Provider profile `{provider}`"
    )]
    UnknownProviderProfile {
        agent_profile: String,
        role: &'static str,
        provider: String,
    },
    #[error("Provider `{provider}` has invalid base URL: {message}")]
    InvalidBaseUrl { provider: String, message: String },
    #[error(
        "Provider `{provider}` credential reference `{reference}` must be a dedicated CYBER_AGENT_*_API_KEY name"
    )]
    InvalidCredentialReference { provider: String, reference: String },
    #[error(
        "Provider `{provider}` must configure exactly one of `api_key`, `credential_ref`, `chatgpt_subscription`, or `codex_subscription`"
    )]
    CredentialSelection { provider: String },
    #[error("Provider `{provider}` subscription configuration is invalid: {message}")]
    InvalidSubscription { provider: String, message: String },
    #[error("subscription authentication setup failed: {message}")]
    SubscriptionAuthentication { message: String },
    #[error("Provider `{provider}` inline `api_key` must not be empty")]
    EmptyInlineApiKey { provider: String },
    #[error("Agent profile `{profile}` is invalid: {source}")]
    InvalidAgentProfile {
        profile: String,
        #[source]
        source: AgentModelProfileError,
    },
}
