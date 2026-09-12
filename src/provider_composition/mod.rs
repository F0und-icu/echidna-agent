//! The single configuration-driven Provider composition boundary used by CLI
//! diagnostics, interactive runtime entrypoints, and generic Live Smokes.

mod config;
mod validation;

use std::path::Path;
use std::sync::Arc;

use crate::adapters::credentials::{CredentialSource, CredentialSourceError};
use crate::adapters::project_event_store::Project;
use crate::adapters::responses::{ResponsesHttpAdapter, ResponsesHttpConfig};
use crate::domain::model::ModelCapabilities;
use crate::domain::profile::AgentModelProfile;
use crate::domain::request_audit::RequestAdapterVersion;
use crate::ports::model::ModelProvider;

pub(crate) use config::{PromptSources, RuntimeSettings};
use config::{ProviderBinding, ProviderConfigV1, ProviderCredential, ProviderDefinition, WireApi};
pub use validation::ProviderCompositionError;

/// One validated Agent profile and the exact Provider instances selected for
/// its Primary and optional Vision roles.
pub struct ProviderComposition {
    agent_profile: AgentModelProfile,
    primary: Arc<dyn ModelProvider>,
    vision: Option<Arc<dyn ModelProvider>>,
    primary_request_adapter_version: RequestAdapterVersion,
    prompt_sources: PromptSources,
    runtime: RuntimeSettings,
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
    /// Provider is configured. The fixed Responses binding currently shares
    /// one wire owner, but callers do not infer that from profile names.
    pub(crate) fn vision_request_adapter_version(&self) -> Option<&RequestAdapterVersion> {
        self.vision
            .as_ref()
            .map(|_| &self.primary_request_adapter_version)
    }

    /// Paths only: the CLI refresh stage reads optional prompt content at a
    /// Turn boundary and reports filesystem errors before Provider activity.
    pub(crate) fn prompt_sources(&self) -> &PromptSources {
        &self.prompt_sources
    }

    pub(crate) fn runtime(&self) -> RuntimeSettings {
        self.runtime
    }

    fn compose(
        config: ValidatedProviderConfig,
        credential_source: Arc<dyn CredentialSource>,
        project: Option<Project>,
        #[cfg(debug_assertions)] debug: bool,
    ) -> Result<Self, ProviderCompositionError> {
        let config = config.0;
        let prompt_sources = config.prompts.clone();

        let agent = config.active_agent_definition();
        let primary_definition = config.provider(&agent.primary);
        let primary = compose_provider(
            &agent.primary,
            primary_definition,
            config.capabilities(primary_definition),
            Arc::clone(&credential_source),
            project.as_ref(),
            project.as_ref().map(|_| "primary"),
            #[cfg(debug_assertions)]
            debug,
        );
        let vision = agent.vision.as_deref().map(|vision_id| {
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
        });
        let agent_profile = config.active_agent_model_profile()?;

        Ok(Self {
            agent_profile,
            primary,
            vision,
            primary_request_adapter_version: ResponsesHttpAdapter::request_adapter_version(),
            prompt_sources,
            runtime: config.runtime,
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
) -> Arc<dyn ModelProvider> {
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
    };
    match (definition.binding, definition.wire_api) {
        (ProviderBinding::HttpSse, WireApi::Responses) => {
            let adapter = ResponsesHttpAdapter::new(
                ResponsesHttpConfig::new(
                    &definition.base_url,
                    &definition.model,
                    credential_reference,
                    capabilities,
                )
                .with_timeouts(definition.connect_timeout_ms, definition.read_timeout_ms),
                credential_source,
            );
            let adapter = match project {
                Some(project) => adapter.with_project_image_resolver(project.clone()),
                None => adapter,
            };
            let adapter = adapter.with_cli_output(
                cli_role,
                #[cfg(debug_assertions)]
                debug,
            );
            Arc::new(adapter)
        }
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
