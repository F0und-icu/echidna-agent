use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::domain::context::{ModelIdentity, ModelProfile};
use crate::domain::model::{CapabilityMappingVersion, ModelCapabilities, ReasoningEffort};
use crate::domain::profile::{AgentModelProfile, ProviderProfile, VisionMode};

use super::ProviderCompositionError;

pub(super) const PROVIDER_CONFIG_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProviderConfigV1 {
    pub(super) schema_version: u16,
    pub(super) active_agent_profile: String,
    pub(super) providers: BTreeMap<String, ProviderDefinition>,
    pub(super) capability_mappings: BTreeMap<String, CapabilityMappingDefinition>,
    pub(super) agent_profiles: BTreeMap<String, AgentProfileDefinition>,
    #[serde(default)]
    pub(super) prompts: PromptSources,
    pub(super) agent_definition: Option<PathBuf>,
    #[serde(default)]
    pub(super) runtime: RuntimeSettings,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct RuntimeSettings {
    pub(crate) model_request_limit_per_turn: usize,
}

impl Default for RuntimeSettings {
    fn default() -> Self {
        Self {
            model_request_limit_per_turn:
                crate::app::model_request_budget::DEFAULT_MODEL_REQUEST_LIMIT,
        }
    }
}

impl ProviderConfigV1 {
    pub(super) fn load(path: impl AsRef<Path>) -> Result<Self, ProviderCompositionError> {
        let path = path.as_ref();
        let encoded = fs::read_to_string(path).map_err(|source| {
            ProviderCompositionError::ReadConfiguration {
                path: path.to_path_buf(),
                source,
            }
        })?;
        let mut config: Self = toml::from_str(&encoded).map_err(|source| {
            ProviderCompositionError::DecodeConfiguration {
                path: path.to_path_buf(),
                message: redact_inline_api_key(source.to_string()),
            }
        })?;
        config.prompts.validate()?;
        config
            .prompts
            .resolve_relative_to(path.parent().unwrap_or_else(|| Path::new(".")));
        if let Some(agent_definition) = &mut config.agent_definition
            && agent_definition.is_relative()
        {
            *agent_definition = path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(&*agent_definition);
        }
        Ok(config)
    }

    pub(super) fn active_agent_definition(&self) -> &AgentProfileDefinition {
        self.agent_profiles
            .get(&self.active_agent_profile)
            .expect("validation guarantees the active Agent profile exists")
    }

    pub(super) fn agent_definition_path(&self) -> Option<&Path> {
        self.agent_definition.as_deref()
    }

    pub(super) fn provider(&self, id: &str) -> &ProviderDefinition {
        self.providers
            .get(id)
            .expect("validation guarantees every Provider reference exists")
    }

    pub(super) fn capabilities(&self, provider: &ProviderDefinition) -> ModelCapabilities {
        self.capability_mappings
            .get(&provider.capability_mapping)
            .expect("validation guarantees every capability mapping reference exists")
            .to_domain()
    }

    pub(super) fn active_agent_model_profile(
        &self,
    ) -> Result<AgentModelProfile, ProviderCompositionError> {
        self.agent_model_profile(&self.active_agent_profile)
    }

    pub(super) fn agent_model_profile(
        &self,
        name: &str,
    ) -> Result<AgentModelProfile, ProviderCompositionError> {
        let agent = self
            .agent_profiles
            .get(name)
            .expect("the validated Agent profile name exists");
        let primary = self.provider_profile(&agent.primary);
        let vision = agent
            .vision
            .as_deref()
            .map(|vision_id| self.provider_profile(vision_id));
        AgentModelProfile::new(
            ModelProfile::new(name, agent.generation),
            primary,
            vision,
            agent.vision_mode,
        )
        .map_err(|source| ProviderCompositionError::InvalidAgentProfile {
            profile: name.to_owned(),
            source,
        })
    }

    fn provider_profile(&self, id: &str) -> ProviderProfile {
        let definition = self.provider(id);
        ProviderProfile::new(
            ModelIdentity::new(id, &definition.model),
            ModelProfile::new(id, definition.generation),
            self.capabilities(definition),
        )
    }
}

/// Explicit prompt paths, deliberately without prompt bytes. Interactive
/// Context loading owns their I/O so `doctor` remains content-free.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PromptSources {
    pub(crate) personal_file: Option<PathBuf>,
    pub(crate) project_file: Option<PathBuf>,
}

impl PromptSources {
    fn resolve_relative_to(&mut self, parent: &Path) {
        for path in [&mut self.personal_file, &mut self.project_file]
            .into_iter()
            .flatten()
        {
            if path.is_relative() {
                *path = parent.join(&*path);
            }
        }
    }

    pub(super) fn validate(&self) -> Result<(), ProviderCompositionError> {
        for (field, path) in [
            ("prompts.personal_file", self.personal_file.as_ref()),
            ("prompts.project_file", self.project_file.as_ref()),
        ] {
            if path.is_some_and(|path| path.as_os_str().is_empty()) {
                return Err(ProviderCompositionError::InvalidPromptPath { field });
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub(super) enum ProviderBinding {
    #[serde(rename = "http_sse")]
    HttpSse,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum WireApi {
    Responses,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProviderDefinition {
    pub(super) binding: ProviderBinding,
    pub(super) wire_api: WireApi,
    pub(super) base_url: String,
    pub(super) model: String,
    pub(super) credential_ref: Option<String>,
    pub(super) api_key: Option<InlineApiKey>,
    pub(super) chatgpt_subscription: Option<ChatGptSubscription>,
    pub(super) codex_subscription: Option<CodexSubscription>,
    pub(super) capability_mapping: String,
    pub(super) generation: u64,
    #[serde(default = "default_connect_timeout_ms")]
    pub(super) connect_timeout_ms: u64,
    #[serde(default = "default_read_timeout_ms")]
    pub(super) read_timeout_ms: u64,
}

const fn default_connect_timeout_ms() -> u64 {
    crate::adapters::responses::DEFAULT_CONNECT_TIMEOUT_MS
}

const fn default_read_timeout_ms() -> u64 {
    crate::adapters::responses::DEFAULT_READ_TIMEOUT_MS
}

impl ProviderDefinition {
    pub(super) fn credential(&self) -> ProviderCredential<'_> {
        match (
            self.credential_ref.as_deref(),
            self.api_key.as_ref(),
            self.chatgpt_subscription.as_ref(),
            self.codex_subscription.as_ref(),
        ) {
            (Some(reference), None, None, None) => ProviderCredential::Reference(reference),
            (None, Some(api_key), None, None) => ProviderCredential::Inline(api_key.expose()),
            (None, None, Some(subscription), None) => {
                ProviderCredential::ChatGptSubscription(subscription)
            }
            (None, None, None, Some(subscription)) => {
                ProviderCredential::CodexSubscription(subscription)
            }
            _ => unreachable!("validated Provider configuration has exactly one credential"),
        }
    }
}

pub(super) enum ProviderCredential<'a> {
    Reference(&'a str),
    Inline(&'a str),
    ChatGptSubscription(&'a ChatGptSubscription),
    CodexSubscription(&'a CodexSubscription),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ChatGptSubscription {
    pub(super) account: String,
    pub(super) auth_store: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CodexSubscription {
    pub(super) account: String,
    pub(super) auth_store: PathBuf,
    pub(super) catalog_client_version: Option<String>,
}

#[derive(Deserialize)]
#[serde(transparent)]
pub(super) struct InlineApiKey(String);

impl InlineApiKey {
    pub(super) fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for InlineApiKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("InlineApiKey(<redacted>)")
    }
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CapabilityMappingDefinition {
    pub(super) version: u16,
    text_input: bool,
    image_input: bool,
    streaming: bool,
    tool_call: bool,
    structured_output: bool,
    incremental_input: bool,
    usage_reporting: bool,
    pub(super) context_window: u32,
    #[serde(default)]
    max_output_tokens: Option<u32>,
    #[serde(default)]
    reasoning_effort: Option<ReasoningEffort>,
}

impl CapabilityMappingDefinition {
    pub(super) fn to_domain(&self) -> ModelCapabilities {
        ModelCapabilities {
            text_input: self.text_input,
            image_input: self.image_input,
            streaming: self.streaming,
            tool_call: self.tool_call,
            structured_output: self.structured_output,
            incremental_input: self.incremental_input,
            usage_reporting: self.usage_reporting,
            context_window: self.context_window,
            max_output_tokens: self.max_output_tokens,
            reasoning_effort: self.reasoning_effort,
            capability_mapping_version: CapabilityMappingVersion::new(self.version),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AgentProfileDefinition {
    pub(super) primary: String,
    pub(super) vision: Option<String>,
    pub(super) vision_mode: VisionMode,
    pub(super) generation: u64,
}

fn redact_inline_api_key(message: String) -> String {
    message
        .lines()
        .map(|line| {
            let Some(key_offset) = line.find("api_key") else {
                return line.to_owned();
            };
            if !line[key_offset + "api_key".len()..].contains('=') {
                return line.to_owned();
            }
            format!("{}api_key = \"<redacted>\"", &line[..key_offset])
        })
        .collect::<Vec<_>>()
        .join("\n")
}
