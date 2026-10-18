//! The single configuration-driven Provider composition boundary used by CLI
//! diagnostics, interactive runtime entrypoints, and generic Live Smokes.

mod config;
mod selection;
mod validation;

use std::path::Path;
use std::sync::Arc;

use crate::adapters::credentials::{
    AccountBoundCredential, CredentialResolution, CredentialSource, CredentialSourceError,
};
use crate::adapters::mcp_http::{McpCatalogTool, McpError, McpRuntime};
use crate::adapters::project_event_store::Project;
use crate::adapters::responses::{ResponsesHttpAdapter, ResponsesHttpConfig, ResponsesWireProfile};
use crate::adapters::subscription_auth::SubscriptionAuthService;
use crate::domain::model::{ModelCapabilities, ReasoningEffort};
use crate::domain::profile::AgentModelProfile;
use crate::domain::request_audit::RequestAdapterVersion;
use crate::ports::model::ModelProvider;

pub(crate) use config::{PromptSources, RuntimeSettings};
use config::{ProviderBinding, ProviderConfigV1, ProviderCredential, ProviderDefinition, WireApi};
pub(crate) use selection::{CatalogCapabilityImport, ModelSelectionError};
pub use validation::ProviderCompositionError;

/// One validated Agent profile and the exact Provider instances selected for
/// its Primary and optional Vision roles.
pub struct ProviderComposition {
    agent_profile: AgentModelProfile,
    primary: Arc<dyn ModelProvider>,
    vision: Option<Arc<dyn ModelProvider>>,
    primary_request_adapter_version: RequestAdapterVersion,
    vision_request_adapter_version: Option<RequestAdapterVersion>,
    prompt_sources: PromptSources,
    agent_definition_path: Option<std::path::PathBuf>,
    runtime: RuntimeSettings,
    mcp: McpRuntime,
}

/// One strictly decoded and validated Config v1 value awaiting runtime-only
/// inputs such as the current Project image resolver. This crate-private
/// token preserves config-error precedence without parsing or composing a
/// Provider twice.
pub(crate) struct ValidatedProviderConfig(ProviderConfigV1);

impl ProviderComposition {
    /// Loads and validates one explicit config path without resolving a
    /// credential or performing network I/O.
    pub fn load(
        path: impl AsRef<Path>,
        credential_source: Arc<dyn CredentialSource>,
    ) -> Result<Self, ProviderCompositionError> {
        Self::compose(
            Self::load_config(path)?,
            credential_source,
            None,
            #[cfg(debug_assertions)]
            false,
        )
    }

    /// Strictly decodes and validates Config v1 before any runtime-only input
    /// is resolved. The returned token has no credential or network action.
    pub(crate) fn load_config(
        path: impl AsRef<Path>,
    ) -> Result<ValidatedProviderConfig, ProviderCompositionError> {
        let config = ProviderConfigV1::load(path)?;
        config.validate()?;
        Ok(ValidatedProviderConfig(config))
    }

    /// Selects an explicitly configured subscription account without resolving
    /// credentials. Auth commands do not need a research Project or AgentLoop.
    pub(crate) fn subscription_auth(
        path: impl AsRef<Path>,
        provider_id: &str,
    ) -> Result<SubscriptionAuthService, ProviderCompositionError> {
        let config = Self::load_config(path)?.0;
        let definition = config.providers.get(provider_id).ok_or_else(|| {
            ProviderCompositionError::InvalidSubscription {
                provider: provider_id.to_owned(),
                message: "unknown Provider profile".to_owned(),
            }
        })?;
        match (
            definition.chatgpt_subscription.as_ref(),
            definition.codex_subscription.as_ref(),
        ) {
            (Some(subscription), None) => subscription_service(subscription),
            (None, Some(subscription)) => codex_subscription_service(subscription),
            _ => Err(ProviderCompositionError::InvalidSubscription {
                provider: provider_id.to_owned(),
                message: "this Provider does not select a subscription credential".to_owned(),
            }),
        }
    }

    /// Saves an explicit catalog choice through the one configuration-edit
    /// owner. The candidate is reloaded through the normal Config owner before
    /// the original file is atomically replaced.
    pub(crate) fn select_codex_model(
        path: impl AsRef<Path>,
        provider_id: &str,
        model: &str,
        reasoning_effort: Option<ReasoningEffort>,
        capabilities: Option<CatalogCapabilityImport>,
    ) -> Result<(), ModelSelectionError> {
        selection::publish_model_selection(
            path.as_ref(),
            provider_id,
            model,
            reasoning_effort.as_ref().map(ReasoningEffort::as_str),
            capabilities.as_ref(),
            |candidate| Self::load_config(candidate).map(|_| ()).map_err(|_| ()),
        )
    }

    /// Binds one validated configuration to an already-discovered Project.
    /// Both Primary and optional Vision Providers retain the one protocol
    /// Adapter construction owner.
    pub(crate) fn compose_for_project(
        config: ValidatedProviderConfig,
        credential_source: Arc<dyn CredentialSource>,
        project: Project,
        #[cfg(debug_assertions)] debug: bool,
    ) -> Result<Self, ProviderCompositionError> {
        Self::compose(
            config,
            credential_source,
            Some(project),
            #[cfg(debug_assertions)]
            debug,
        )
    }

    pub fn agent_profile(&self) -> &AgentModelProfile {
        &self.agent_profile
    }

    pub fn primary_provider(&self) -> &dyn ModelProvider {
        self.primary.as_ref()
    }

    pub fn vision_provider(&self) -> Option<&dyn ModelProvider> {
        self.vision.as_deref()
    }

    /// Returns the RequestAudit identity for the configured Primary wire
    /// implementation. CLI and generic runtime callers do not infer it from
    /// a model, URL, credential reference, or Provider type.
    pub fn primary_request_adapter_version(&self) -> &RequestAdapterVersion {
        &self.primary_request_adapter_version
    }

    /// Returns the scoped Vision request Adapter identity when a Vision
    /// Provider is configured. Each role carries its configured protocol profile.
    pub(crate) fn vision_request_adapter_version(&self) -> Option<&RequestAdapterVersion> {
        self.vision_request_adapter_version.as_ref()
    }

    /// Paths only: the CLI refresh stage reads optional prompt content at a
    /// Turn boundary and reports filesystem errors before Provider activity.
    pub(crate) fn prompt_sources(&self) -> &PromptSources {
        &self.prompt_sources
    }

    /// An explicit configured definition source for a new Session. Existing
    /// Session bindings remain their own immutable authority.
    pub(crate) fn agent_definition_path(&self) -> Option<&Path> {
        self.agent_definition_path.as_deref()
    }

    pub(crate) fn runtime(&self) -> RuntimeSettings {
        self.runtime
    }

    /// Discovers configured MCP tools only for live Chat setup. Default
    /// diagnostics remain offline and never resolve MCP credentials.
    pub(crate) async fn discover_chat_mcp(&self) -> Result<Vec<McpCatalogTool>, McpError> {
        self.mcp.discover().await
    }

    fn compose(
        config: ValidatedProviderConfig,
        credential_source: Arc<dyn CredentialSource>,
        project: Option<Project>,
        #[cfg(debug_assertions)] debug: bool,
    ) -> Result<Self, ProviderCompositionError> {
        let config = config.0;
        let prompt_sources = config.prompts.clone();
        let agent_definition_path = config.agent_definition_path().map(Path::to_path_buf);
        let mcp = McpRuntime::new(config.mcp_servers.clone(), Arc::clone(&credential_source));

        let agent = config.active_agent_definition();
        let primary_definition = config.provider(&agent.primary);
        let (primary, primary_request_adapter_version) = compose_provider(
            &agent.primary,
            primary_definition,
            config.capabilities(primary_definition),
            Arc::clone(&credential_source),
            project.as_ref(),
            project.as_ref().map(|_| "primary"),
            #[cfg(debug_assertions)]
            debug,
        )?;
        let vision = agent
            .vision
            .as_deref()
            .map(|vision_id| {
                let definition = config.provider(vision_id);
                compose_provider(
                    vision_id,
                    definition,
                    config.capabilities(definition),
                    Arc::clone(&credential_source),
                    project.as_ref(),
                    project.as_ref().map(|_| "vision"),
                    #[cfg(debug_assertions)]
                    debug,
                )
            })
            .transpose()?;
        let (vision, vision_request_adapter_version) = match vision {
            Some((provider, version)) => (Some(provider), Some(version)),
            None => (None, None),
        };
        let agent_profile = config.active_agent_model_profile()?;

        Ok(Self {
            agent_profile,
            primary,
            vision,
            primary_request_adapter_version,
            vision_request_adapter_version,
            prompt_sources,
            agent_definition_path,
            runtime: config.runtime,
            mcp,
        })
    }
}

fn compose_provider(
    provider_id: &str,
    definition: &ProviderDefinition,
    capabilities: ModelCapabilities,
    credential_source: Arc<dyn CredentialSource>,
    project: Option<&Project>,
    cli_role: Option<&'static str>,
    #[cfg(debug_assertions)] debug: bool,
) -> Result<(Arc<dyn ModelProvider>, RequestAdapterVersion), ProviderCompositionError> {
    let (credential_reference, credential_source) = match definition.credential() {
        ProviderCredential::Reference(reference) => (reference.to_owned(), credential_source),
        ProviderCredential::Inline(value) => {
            let reference = format!("config:inline:{provider_id}");
            let source: Arc<dyn CredentialSource> = Arc::new(InlineCredentialSource {
                reference: reference.clone(),
                value: value.to_owned(),
            });
            (reference, source)
        }
        ProviderCredential::ChatGptSubscription(subscription) => {
            let reference = format!("chatgpt_subscription:{provider_id}");
            let source: Arc<dyn CredentialSource> = Arc::new(SubscriptionCredentialSource {
                reference: reference.clone(),
                service: Arc::new(subscription_service(subscription)?),
            });
            (reference, source)
        }
        ProviderCredential::CodexSubscription(subscription) => {
            let reference = format!("codex_subscription:{provider_id}");
            let source: Arc<dyn CredentialSource> = Arc::new(CodexSubscriptionCredentialSource {
                reference: reference.clone(),
                service: Arc::new(codex_subscription_service(subscription)?),
            });
            (reference, source)
        }
    };
    match (definition.binding, definition.wire_api) {
        (ProviderBinding::HttpSse, WireApi::Responses) => {
            let config = ResponsesHttpConfig::new(
                &definition.base_url,
                &definition.model,
                credential_reference,
                capabilities,
            )
            .with_timeouts(definition.connect_timeout_ms, definition.read_timeout_ms);
            let config = match wire_profile(definition) {
                ResponsesWireProfile::Standard => config,
                ResponsesWireProfile::ChatGptSubscription => {
                    config.with_chatgpt_subscription_profile()
                }
                ResponsesWireProfile::CodexSubscription => config
                    .with_codex_subscription_profile()
                    .with_codex_reasoning_summary_support(
                        definition
                            .codex_subscription
                            .as_ref()
                            .and_then(|subscription| {
                                subscription.supports_reasoning_summary_parameter
                            }),
                    ),
            };
            let request_adapter_version = config.request_adapter_version();
            let adapter = ResponsesHttpAdapter::new(config, credential_source);
            let adapter = match project {
                Some(project) => adapter.with_project_image_resolver(project.clone()),
                None => adapter,
            };
            let adapter = adapter.with_cli_output(
                cli_role,
                #[cfg(debug_assertions)]
                debug,
            );
            Ok((Arc::new(adapter), request_adapter_version))
        }
    }
}

fn wire_profile(definition: &ProviderDefinition) -> ResponsesWireProfile {
    match (
        definition.chatgpt_subscription.is_some(),
        definition.codex_subscription.is_some(),
    ) {
        (true, false) => ResponsesWireProfile::ChatGptSubscription,
        (false, true) => ResponsesWireProfile::CodexSubscription,
        (false, false) => ResponsesWireProfile::Standard,
        (true, true) => unreachable!("validated Provider configuration has exactly one credential"),
    }
}

fn subscription_service(
    subscription: &config::ChatGptSubscription,
) -> Result<SubscriptionAuthService, ProviderCompositionError> {
    SubscriptionAuthService::new(
        subscription.auth_store.clone(),
        subscription.account.clone(),
    )
    .map_err(
        |error| ProviderCompositionError::SubscriptionAuthentication {
            message: error.to_string(),
        },
    )
}

fn codex_subscription_service(
    subscription: &config::CodexSubscription,
) -> Result<SubscriptionAuthService, ProviderCompositionError> {
    SubscriptionAuthService::new_codex(
        subscription.auth_store.clone(),
        subscription.account.clone(),
        subscription.catalog_client_version.as_deref(),
    )
    .map_err(
        |error| ProviderCompositionError::SubscriptionAuthentication {
            message: error.to_string(),
        },
    )
}

struct SubscriptionCredentialSource {
    reference: String,
    service: Arc<SubscriptionAuthService>,
}

impl CredentialSource for SubscriptionCredentialSource {
    fn read(&self, _reference: &str) -> Result<Option<String>, CredentialSourceError> {
        Err(CredentialSourceError::ReadFailed {
            message: "subscription credentials require asynchronous resolution".to_owned(),
        })
    }

    fn resolve(&self, reference: &str) -> CredentialResolution {
        if reference != self.reference {
            return CredentialResolution::Ready(Ok(None));
        }
        let service = Arc::clone(&self.service);
        CredentialResolution::Pending(Box::pin(async move {
            service.access_token().await.map(Some).map_err(|error| {
                CredentialSourceError::ReadFailed {
                    message: error.to_string(),
                }
            })
        }))
    }
}

struct CodexSubscriptionCredentialSource {
    reference: String,
    service: Arc<SubscriptionAuthService>,
}

impl CredentialSource for CodexSubscriptionCredentialSource {
    fn read(&self, _reference: &str) -> Result<Option<String>, CredentialSourceError> {
        Err(CredentialSourceError::ReadFailed {
            message: "Codex subscription credentials require asynchronous resolution".to_owned(),
        })
    }

    fn resolve(&self, reference: &str) -> CredentialResolution {
        if reference != self.reference {
            return CredentialResolution::Ready(Ok(None));
        }
        let service = Arc::clone(&self.service);
        CredentialResolution::PendingAccountBound(Box::pin(async move {
            service
                .access_codex_credential()
                .await
                .map(|credential| {
                    let (bearer, account_id) = credential.into_parts();
                    AccountBoundCredential::new(bearer, account_id)
                })
                .map_err(|error| CredentialSourceError::ReadFailed {
                    message: error.to_string(),
                })
        }))
    }
}

struct InlineCredentialSource {
    reference: String,
    value: String,
}

impl CredentialSource for InlineCredentialSource {
    fn read(&self, reference: &str) -> Result<Option<String>, CredentialSourceError> {
        Ok((reference == self.reference).then(|| self.value.clone()))
    }
}
