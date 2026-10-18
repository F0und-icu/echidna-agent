//! Offline operator interface for the one local Agent-definition loader.

use std::path::{Path, PathBuf};

use clap::{Subcommand, ValueEnum};
use serde_json::json;

use crate::adapters::agent_definition::{
    builtin_research_binding, load_definition_binding, write_builtin_definition,
};
use crate::adapters::tools::ToolRegistry;
use crate::app::agent_loop::AgentLoop;
use crate::domain::agent_definition::{
    AgentDefinitionBinding, AgentMemoryScope, BuiltinAgentTemplate,
};
use crate::domain::context::{ProviderSession, ProviderSessionId, ProviderSessionRole};
use crate::domain::memory::MemoryScope;
use crate::domain::research_control::PromptComposition;
use crate::domain::tools::DeferredActivationSet;
use crate::ports::tool::ToolExecutor;
use crate::provider_composition::ProviderComposition;

use crate::{CliError, load_provider_composition, print_json};

#[derive(Debug, Subcommand)]
pub(crate) enum AgentCommand {
    /// Export a complete editable definition into a new directory.
    Init {
        path: PathBuf,
        #[arg(long)]
        id: Option<String>,
        #[arg(long, value_enum, default_value_t = AgentTemplate::TechnicalWriting)]
        from: AgentTemplate,
    },
    /// Validate all resources and configured model context budgets offline.
    Check { path: PathBuf },
    /// Show resolved instruction snapshots, their sources, and effective tools.
    Inspect {
        path: PathBuf,
        /// Discover configured MCP tools once and include their authoring specs.
        #[arg(long)]
        discover_mcp: bool,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum AgentTemplate {
    BuiltinResearch,
    TechnicalWriting,
}

fn definition_error(error: impl std::fmt::Display) -> CliError {
    CliError::AgentDefinition {
        message: error.to_string(),
    }
}

pub(crate) fn select_binding(
    path: Option<&Path>,
    composition: &ProviderComposition,
    tools: &ToolRegistry,
) -> Result<AgentDefinitionBinding, CliError> {
    let catalog = tools.catalog();
    let sources = composition.prompt_sources();
    match path.or_else(|| composition.agent_definition_path()) {
        Some(path) => load_definition_binding(
            path,
            sources.personal_file.as_deref(),
            sources.project_file.as_deref(),
            &catalog,
        ),
        None => builtin_research_binding(
            sources.personal_file.as_deref(),
            sources.project_file.as_deref(),
            &catalog,
        ),
    }
    .map_err(definition_error)
}

pub(crate) fn memory_scope(binding: &AgentDefinitionBinding) -> Option<MemoryScope> {
    match binding.definition().context().memory() {
        AgentMemoryScope::None => None,
        AgentMemoryScope::Project => Some(MemoryScope::Project),
        AgentMemoryScope::Agent => Some(MemoryScope::Agent {
            agent_id: binding.definition().id().clone(),
        }),
    }
}

pub(crate) fn run(config: Option<&Path>, command: AgentCommand) -> Result<(), CliError> {
    if let AgentCommand::Init { path, id, from } = command {
        let template = match from {
            AgentTemplate::BuiltinResearch => BuiltinAgentTemplate::Research,
            AgentTemplate::TechnicalWriting => BuiltinAgentTemplate::TechnicalWriting,
        };
        write_builtin_definition(template, &path, id.as_deref()).map_err(definition_error)?;
        return print_json(&json!({"status":"created", "path":path, "template":template}));
    }
    let (path, inspect, discover_mcp) = match command {
        AgentCommand::Check { path } => (path, false, false),
        AgentCommand::Inspect { path, discover_mcp } => (path, true, discover_mcp),
        AgentCommand::Init { .. } => unreachable!("init returns above"),
    };
    let composition = load_provider_composition(config)?.ok_or_else(|| {
        definition_error("check/inspect requires --config for model and context validation")
    })?;
    let root = std::env::current_dir().map_err(CliError::WorkingDirectory)?;
    let discovered_mcp = if discover_mcp {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(definition_error)?;
        runtime
            .block_on(composition.discover_chat_mcp())
            .map_err(definition_error)?
    } else {
        Vec::new()
    };
    let tools = if discover_mcp {
        ToolRegistry::for_project_with_mcp(&root, &discovered_mcp).map_err(definition_error)?
    } else {
        ToolRegistry::for_project(&root)
    };
    let binding = select_binding(Some(&path), &composition, &tools)?;
    let tools = if discover_mcp {
        ToolRegistry::for_agent_with_mcp(
            &root,
            crate::domain::events::StreamId::new("definition-check"),
            memory_scope(&binding),
            binding.definition().research().is_some(),
            &discovered_mcp,
        )
        .map_err(definition_error)?
    } else {
        ToolRegistry::for_agent(
            &root,
            crate::domain::events::StreamId::new("definition-check"),
            memory_scope(&binding),
            binding.definition().research().is_some(),
        )
    };
    let prompt = PromptComposition::from_binding(binding.clone()).map_err(definition_error)?;
    let preview = AgentLoop::with_primary_context(
        composition.primary_provider(),
        &tools,
        composition.agent_profile().primary().model().clone(),
        composition.agent_profile().primary().profile().clone(),
        ProviderSession::new(
            ProviderSessionId::new("definition-check"),
            ProviderSessionRole::Primary,
            1,
        ),
    )
    .map_err(definition_error)?;
    let plan = preview
        .preflight_definition(&prompt, &[], None)
        .map_err(definition_error)?;
    let tool_plan = tools
        .resolve_tool_plan(
            &prompt
                .tool_exposure_for_catalog(&tools.catalog())
                .map_err(definition_error)?,
            &DeferredActivationSet::default(),
        )
        .map_err(definition_error)?;
    if inspect {
        print_json(&json!({
            "status":"valid", "binding":binding,
            "instructions":plan.model_input().instructions,
            "tools":tool_plan.resolved_specs(),
            "discovered_mcp_tools":tools.discovered_mcp_specs(),
            "validation":if discover_mcp {
                "MCP discovered once for authoring; static instructions and tools were then validated without a model request"
            } else {
                "offline; static instructions and tools; task/history/current facts are checked at runtime"
            }
        }))
    } else {
        print_json(&json!({
            "status":"valid", "agent_id":binding.definition().id(),
            "definition_digest":binding.definition().digest(),
            "validation":"offline; static instructions and tools; task/history/current facts are checked at runtime"
        }))
    }
}
