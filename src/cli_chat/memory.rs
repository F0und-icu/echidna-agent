//! Read-only visibility into the existing project Markdown memory.

use serde_json::{Value, json};

use crate::adapters::project_event_store::Project;
use crate::adapters::project_memory::LocalProjectMemory;
use crate::adapters::tools::ToolRegistry;
use crate::domain::memory::MemoryPath;
use crate::ports::tool::ToolExecutor;

use super::{ChatError, context::ChatContextRefreshError};

pub(super) fn status(project: &Project, tools: &ToolRegistry) -> Result<Value, ChatError> {
    let memory = LocalProjectMemory::with_window_policy(project, tools.window_policy());
    let core = memory
        .read(&MemoryPath::parse("core.md").expect("fixed Memory path"))
        .map_err(ChatContextRefreshError::from)?;
    let catalog = tools.catalog();
    let exposure = |name| {
        catalog
            .iter()
            .find(|spec| spec.name.as_str() == name)
            .map(|spec| spec.exposure)
    };
    Ok(json!({
        "memory": "enabled",
        "directory": memory.memory_root().display().to_string(),
        "core_path": memory.memory_root().join("core.md").display().to_string(),
        "core_present": core.content.is_some(),
        "core_bytes": core.content.as_ref().map(String::len),
        "core_hash": core.hash,
        "configured_memory_read": exposure("memory_read"),
        "configured_memory_write": exposure("memory_write"),
        "generation": "model_tool_writes",
        "automatic_extraction": false,
        "automatic_context": "core.md; read detail documents explicitly with memory_read",
        "creation": "The first successful memory_write creates its Markdown document.",
    }))
}
