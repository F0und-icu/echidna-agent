//! Explicit definition admission and Session selection; never executes work.

use std::path::Path;

use crate::adapters::agent_definition::{
    bind_definition, builtin_definition, builtin_research_binding, load_definition_binding,
};
use crate::adapters::mcp::McpCatalogTool;
use crate::adapters::project_event_store::Project;
use crate::adapters::tools::ToolRegistry;
use crate::app::agent_loop::AgentLoop;
use crate::domain::agent_definition::{AgentDefinitionBinding, DefinitionSourceLocator};
use crate::domain::context::{ProviderSession, ProviderSessionId, ProviderSessionRole};
use crate::domain::events::StreamId;
use crate::domain::research_control::PromptComposition;
use crate::domain::session::SessionState;
use crate::ports::tool::ToolExecutor;
use crate::provider_composition::ProviderComposition;

use super::{ChatError, DurableChatSession, context};

pub(super) fn error(value: impl std::fmt::Display) -> ChatError {
    ChatError::Definition {
        message: value.to_string(),
    }
}

pub(super) fn initial_binding(
    project: &Project,
    session: &StreamId,
    resume: bool,
    requested: Option<&Path>,
    composition: &ProviderComposition,
    discovered_mcp: &[McpCatalogTool],
) -> Result<
    (
        AgentDefinitionBinding,
        Option<super::persistence::CachedSessionProjection>,
    ),
    ChatError,
> {
    let tools = ToolRegistry::for_project_with_mcp(project.root(), discovered_mcp)?
        .with_project(project)
        .with_window_policy(
            composition
                .runtime()
                .context_budget
                .for_capabilities(composition.agent_profile().primary().capabilities())
                .map_err(crate::app::agent_loop::AgentLoopError::from)?,
        );
    if resume {
        if !project
            .existing_session_ids()?
            .iter()
            .any(|id| id == session)
        {
            return Err(ChatError::SessionNotFound {
                session_id: session.as_str().to_owned(),
            });
        }
        let projection = super::persistence::CachedSessionProjection::load(project, session)?;
        if let Some(binding) = projection.state().agent_definition_binding() {
            binding
                .definition()
                .validate_for_catalog(&tools.catalog())
                .map_err(error)?;
            return Ok((binding.clone(), Some(projection)));
        }
        let sources = composition.prompt_sources();
        return builtin_research_binding(
            sources.personal_file.as_deref(),
            sources.project_file.as_deref(),
            &tools.catalog(),
        )
        .map(|binding| (binding, Some(projection)))
        .map_err(error);
    }
    let source = crate::cli_agent::select_binding(requested, composition, &tools).map_err(error)?;
    crate::adapters::agent_evolution::EvolutionStore::new(project, &source)
        .select(source)
        .map(|binding| (binding, None))
        .map_err(Into::into)
}

pub(super) fn reload_candidate(
    current: &AgentDefinitionBinding,
    requested: Option<&Path>,
    composition: &ProviderComposition,
    tools: &ToolRegistry,
) -> Result<AgentDefinitionBinding, ChatError> {
    let catalog = tools.catalog();
    let sources = composition.prompt_sources();
    let candidate = if let Some(path) = requested {
        load_definition_binding(
            path,
            sources.personal_file.as_deref(),
            sources.project_file.as_deref(),
            &catalog,
        )
        .map_err(error)?
    } else {
        match current.source_locator() {
            DefinitionSourceLocator::Local { manifest_path } => load_definition_binding(
                manifest_path,
                sources.personal_file.as_deref(),
                sources.project_file.as_deref(),
                &catalog,
            )
            .map_err(error)?,
            DefinitionSourceLocator::Builtin { template } => {
                let definition = builtin_definition(*template, &catalog).map_err(error)?;
                bind_definition(
                    definition,
                    sources.personal_file.as_deref(),
                    sources.project_file.as_deref(),
                )
                .map_err(error)?
            }
        }
    };
    current
        .reload_compatible_with(candidate.definition())
        .map_err(error)?;
    Ok(candidate)
}

pub(super) fn prompt_for_state(
    project: &Project,
    binding: &AgentDefinitionBinding,
    state: &SessionState,
    session_id: &str,
) -> Result<PromptComposition, ChatError> {
    let mut prompt = PromptComposition::from_binding(binding.clone())
        .map_err(error)?
        .with_evolution_session(
            session_id,
            format!(
                "{}/{}",
                project.control_directory_name(),
                crate::domain::agent_evolution::pending_relative(
                    binding.definition().id().as_str(),
                    session_id
                )
            ),
        );
    if binding.definition().research().is_some() {
        if let Some(selection) = state.domain_profile_selection() {
            prompt = prompt.with_domain_profile(selection.profile());
        }
        if let Some(selection) = state.research_phase_selection() {
            prompt = prompt.with_research_phase(selection.phase());
        }
    }
    Ok(prompt)
}

impl DurableChatSession {
    pub(super) fn agent_binding(&self) -> &AgentDefinitionBinding {
        &self.binding
    }

    pub(super) fn bound_tools(
        &self,
        discovered_mcp: &[McpCatalogTool],
        composition: &ProviderComposition,
    ) -> Result<ToolRegistry, ChatError> {
        let window_policy = composition
            .runtime()
            .context_budget
            .for_capabilities(composition.agent_profile().primary().capabilities())
            .map_err(crate::app::agent_loop::AgentLoopError::from)?;
        ToolRegistry::for_agent_with_mcp(
            self.project.root(),
            self.session_id.clone(),
            crate::cli_agent::memory_scope(&self.binding),
            self.binding.definition().research().is_some(),
            discovered_mcp,
        )
        .map(|tools| {
            tools
                .with_project(&self.project)
                .with_input_verification(self.binding.definition().rsi().is_some())
                .with_automatic_verification(
                    self.binding
                        .definition()
                        .rsi()
                        .is_some_and(|rsi| rsi.improvement),
                )
                .with_window_policy(window_policy)
        })
        .map_err(ChatError::from)
    }

    pub(super) fn preflight_binding(
        &self,
        state: &SessionState,
        composition: &ProviderComposition,
        discovered_mcp: &[McpCatalogTool],
    ) -> Result<(), ChatError> {
        if state.stream_id().is_some() {
            self.with_projection(|projection, _, _| {
                let read = self.project_read(projection.state())?;
                self.preflight_binding_from_read(
                    projection.state(),
                    composition,
                    discovered_mcp,
                    &read,
                )
            })
        } else {
            let read = self.project_read(state)?;
            self.preflight_binding_from_read(state, composition, discovered_mcp, &read)
        }
    }

    pub(super) fn preflight_binding_from_read(
        &self,
        state: &SessionState,
        composition: &ProviderComposition,
        discovered_mcp: &[McpCatalogTool],
        read: &crate::adapters::research_projection::ProjectReadContext,
    ) -> Result<(), ChatError> {
        let tools = self.bound_tools(discovered_mcp, composition)?;
        let prompt = prompt_for_state(
            &self.project,
            &self.binding,
            state,
            self.session_id.as_str(),
        )?;
        let mut preview = AgentLoop::with_primary_context(
            composition.primary_provider(),
            &tools,
            composition.agent_profile().primary().model().clone(),
            composition.agent_profile().primary().profile().clone(),
            ProviderSession::new(
                ProviderSessionId::new("definition-preflight"),
                ProviderSessionRole::Primary,
                1,
            ),
        )?
        .with_context_budget_policy(composition.runtime().context_budget)
        .with_prompt_composition(prompt.clone());
        preview.refresh_current_context(
            context::ChatContextCache::default().refresh_from_read_context(
                &self.project,
                &tools,
                &self.binding,
                read,
            )?,
        )?;
        let history = state.compaction_source_history()?;
        preview.set_protected_context(state.current_protected_operator_context()?);
        preview.preflight_definition(
            &prompt,
            &history,
            state.latest_compaction().map(|value| value.summary()),
        )?;
        Ok(())
    }
}
