//! Interactive Chat's fixed current-context refresh.
//!
//! This module reads only explicit project/configuration facts. It does not
//! own Context ordering, prompt rendering, or fact projection: it supplies
//! typed current values to the existing AgentLoop/ContextPlanner path.

use thiserror::Error;

use crate::adapters::project_event_store::Project;
use crate::adapters::project_memory::{LocalProjectMemory, LocalProjectMemoryError};
use crate::adapters::project_snapshot::LocalProjectSnapshot;
use crate::adapters::research_projection::ProjectReadContext;
use crate::adapters::tools::ToolRegistry;
use crate::app::agent_loop::CurrentContextState;
use crate::domain::agent_definition::{AgentDefinitionBinding, AgentMemoryScope};
use crate::domain::context::{
    ContextFragment, ContextFragmentContent, ContextFragmentId, ContextFragmentPriority,
    ContextFragmentSource, FragmentCachePolicy,
};
use crate::domain::memory::{MemoryPath, MemoryScope};
use crate::domain::model::{ContentItem, MessageRole};
use crate::domain::world_state::{ProjectSnapshotContext, ResearchWorldState};
use crate::ports::tool::ToolExecutor;

/// A short-lived cache owned by one interactive Chat process. Its revision is
/// an exact digest of every source consumed by `load_research_state`, never a
/// mtime-only guess. An unchanged revision avoids replaying every Session
/// receipt catalog and reducing the Research Ledger at each Tool-loop Step.
#[derive(Default)]
pub(crate) struct ChatContextCache {
    research_revision: Option<ResearchProjectionRevision>,
    world_state: Option<ResearchWorldState>,
}

impl ChatContextCache {
    /// Captures the current inputs for one immutable later SamplingStepPlan.
    /// Facts are refreshed by content identity. Instructions come from the
    /// durable binding and change only through an explicit reload.
    pub(crate) fn refresh_from_read_context(
        &mut self,
        project: &Project,
        tools: &ToolRegistry,
        binding: &AgentDefinitionBinding,
        read: &ProjectReadContext,
        memory_cache: &std::sync::Mutex<crate::adapters::project_memory::MemoryReadCache>,
    ) -> Result<CurrentContextState, ChatContextRefreshError> {
        let mut memory_cache = memory_cache.lock().map_err(|_| {
            ChatContextRefreshError::MemoryRevision("Memory cache lock poisoned".to_owned())
        })?;
        let world_state =
            self.current_project_context(read, binding.definition().research().is_some())?;
        let snapshot = LocalProjectSnapshot::new(project)
            .inspect(&tools.specs(), &[], "context-refresh")
            .map_err(ChatContextRefreshError::ProjectSnapshot)?;
        let project_snapshot = ProjectSnapshotContext::new(snapshot.snapshot)?;
        let core_memory = load_core_memory(
            project,
            binding,
            tools.window_policy().core_file_bytes(),
            &mut memory_cache,
        )?;
        let memory_revision = crate::domain::canonical_json::canonical_blake3_hash(
            &load_task_memory(project, binding, &mut memory_cache)?,
        )
        .map_err(|error| ChatContextRefreshError::MemoryRevision(error.to_string()))?;
        Ok(
            CurrentContextState::new(world_state, project_snapshot, core_memory)
                .with_research_revision(
                    binding
                        .definition()
                        .research()
                        .map(|_| read.research_digest.clone()),
                )
                .with_memory_revision(memory_revision),
        )
    }

    fn current_project_context(
        &mut self,
        read: &ProjectReadContext,
        research: bool,
    ) -> Result<Option<ResearchWorldState>, ChatContextRefreshError> {
        let revision = ResearchProjectionRevision {
            research_digest: read.research_digest.clone(),
            receipt_catalog_digest: read.receipt_digest.clone(),
        };
        if self.research_revision.as_ref() == Some(&revision) {
            return if research {
                self.world_state
                    .clone()
                    .map(Some)
                    .ok_or(ChatContextRefreshError::MissingCachedWorldState)
            } else {
                Ok(None)
            };
        }
        let state = read.research_state();
        let world = if research {
            Some(ResearchWorldState::from_research_state(state)?)
        } else {
            None
        };
        self.research_revision = Some(revision);
        self.world_state = world.clone();
        Ok(world)
    }
}

pub(super) fn load_task_memory(
    project: &Project,
    binding: &AgentDefinitionBinding,
    cache: &mut crate::adapters::project_memory::MemoryReadCache,
) -> Result<Vec<crate::domain::session::TaskMemoryDocument>, ChatContextRefreshError> {
    let Some(scope) = crate::cli_agent::memory_scope(binding) else {
        return Ok(Vec::new());
    };
    LocalProjectMemory::with_scope(project, scope.clone())
        .read_retrieval_documents_cached(cache)?
        .into_iter()
        .map(
            |document| match (document.document.content, document.document.hash) {
                (Some(content), Some(revision)) => Ok(crate::domain::session::TaskMemoryDocument {
                    scope: scope.clone(),
                    path: document.path,
                    revision,
                    content,
                }),
                _ => Err(ChatContextRefreshError::MemoryRevision(
                    "Memory document and revision differ".to_owned(),
                )),
            },
        )
        .collect()
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ResearchProjectionRevision {
    research_digest: String,
    receipt_catalog_digest: String,
}

fn load_core_memory(
    project: &Project,
    binding: &AgentDefinitionBinding,
    core_file_bytes: usize,
    cache: &mut crate::adapters::project_memory::MemoryReadCache,
) -> Result<Option<ContextFragment>, ChatContextRefreshError> {
    let scope = match binding.definition().context().memory() {
        AgentMemoryScope::None => return Ok(None),
        AgentMemoryScope::Project => MemoryScope::Project,
        AgentMemoryScope::Agent => MemoryScope::Agent {
            agent_id: binding.definition().id().clone(),
        },
    };
    let path = MemoryPath::parse("core.md").expect("core.md is a fixed supported Memory path");
    let namespace = match &scope {
        MemoryScope::Project => "project".to_owned(),
        MemoryScope::Agent { agent_id } => format!("agent:{}", agent_id.as_str()),
    };
    let document = LocalProjectMemory::with_scope(project, scope)
        .with_core_byte_limit(core_file_bytes)
        .read_cached(&path, cache)?;
    match document.content {
        None => Ok(None),
        Some(content) if content.is_empty() => {
            memory_core_fragment(&namespace, "empty", "Core Memory is explicitly empty.")
        }
        Some(content) => {
            let hash = document
                .hash
                .expect("a present LocalProjectMemory document has a content hash");
            memory_core_fragment(&namespace, &hash, &content)
        }
    }
}

fn memory_core_fragment(
    namespace: &str,
    revision: &str,
    content: &str,
) -> Result<Option<ContextFragment>, ChatContextRefreshError> {
    Ok(Some(ContextFragment::new(
        ContextFragmentId::new(format!("memory-core:{namespace}:{revision}"))?,
        ContextFragmentSource::ProjectMemoryCore,
        ContextFragmentPriority::High,
        FragmentCachePolicy::ContentAddressed,
        ContextFragmentContent::ContentItems {
            items: vec![ContentItem::Text {
                role: MessageRole::User,
                text: content.to_owned(),
            }],
        },
    )?))
}

#[derive(Debug, Error)]
pub(crate) enum ChatContextRefreshError {
    #[error("Memory snapshot revision failed: {0}")]
    MemoryRevision(String),
    #[error("unchanged Research revision did not retain a cached WorldState")]
    MissingCachedWorldState,
    #[error(transparent)]
    Project(#[from] crate::adapters::project_event_store::ProjectEventStoreError),
    #[error(transparent)]
    ProjectSnapshot(#[from] crate::adapters::project_snapshot::LocalProjectSnapshotError),
    #[error(transparent)]
    Memory(#[from] LocalProjectMemoryError),
    #[error(transparent)]
    WorldState(#[from] crate::domain::world_state::WorldStateError),
    #[error(transparent)]
    Context(#[from] crate::domain::context::ContextPlannerError),
}
