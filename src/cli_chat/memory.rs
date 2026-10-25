//! Read-only visibility into the bound Markdown memory namespace.

use serde_json::{Value, json};

use crate::adapters::project_event_store::Project;
use crate::adapters::project_memory::LocalProjectMemory;
use crate::adapters::tools::ToolRegistry;
use crate::domain::agent_definition::AgentDefinitionBinding;
use crate::domain::memory::MemoryPath;
use crate::domain::tools::{ToolExposure, ToolName};
use crate::ports::tool::ToolExecutor;

use super::{ChatError, context::ChatContextRefreshError};

pub(super) fn status(
    project: &Project,
    binding: &AgentDefinitionBinding,
    tools: &ToolRegistry,
) -> Result<Value, ChatError> {
    let Some(scope) = crate::cli_agent::memory_scope(binding) else {
        return Ok(json!({"memory": "disabled", "automatic_extraction": false}));
    };
    let memory = LocalProjectMemory::with_scope(project, scope.clone());
    let core = memory
        .read(&MemoryPath::parse("core.md").expect("fixed Memory path"))
        .map_err(ChatContextRefreshError::from)?;
    let exposures = binding
        .definition()
        .tool_exposure_for(&tools.catalog())
        .map_err(super::definition::error)?;
    Ok(json!({
        "memory": "enabled",
        "scope": scope,
        "directory": memory.memory_root().display().to_string(),
        "core_path": memory.memory_root().join("core.md").display().to_string(),
        "core_present": core.content.is_some(),
        "core_bytes": core.content.as_ref().map(String::len),
        "core_hash": core.hash,
        "configured_memory_read": exposures.exposure_for(&ToolName::new("memory_read"), ToolExposure::Hidden),
        "configured_memory_write": exposures.exposure_for(&ToolName::new("memory_write"), ToolExposure::Hidden),
        "generation": "model_tool_writes",
        "automatic_extraction": false,
        "automatic_context": "core.md; read detail documents explicitly with memory_read",
        "creation": "The first successful memory_write creates its Markdown document."
    }))
}
