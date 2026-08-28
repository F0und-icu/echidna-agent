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
        match (self.credential_ref.as_deref(), self.api_key.as_ref()) {
            (Some(reference), None) => validate_credential_reference(name, reference),
            (None, Some(api_key)) if !api_key.expose().trim().is_empty() => Ok(()),
            (None, Some(_)) => Err(ProviderCompositionError::EmptyInlineApiKey {
                provider: name.to_owned(),
            }),
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
    #[error("Provider `{provider}` must configure exactly one of `api_key` or `credential_ref`")]
    CredentialSelection { provider: String },
    #[error("Provider `{provider}` inline `api_key` must not be empty")]
    EmptyInlineApiKey { provider: String },
    #[error("Agent profile `{profile}` is invalid: {source}")]
    InvalidAgentProfile {
        profile: String,
        #[source]
        source: AgentModelProfileError,
    },
}
