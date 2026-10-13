//! Interactive Chat's fixed current-context refresh.
//!
//! This module reads only explicit project/configuration facts. It does not
//! own Context ordering, prompt rendering, or fact projection: it supplies
//! typed current values to the existing AgentLoop/ContextPlanner path.

use std::fs;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::adapters::project_event_store::Project;
use crate::adapters::project_memory::{LocalProjectMemory, LocalProjectMemoryError};
use crate::adapters::project_snapshot::LocalProjectSnapshot;
use crate::adapters::research_projection::{
    SessionReceiptCatalogCache, load_project_stream_integrity, load_research_state_with_references,
};
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
    receipt_cache: SessionReceiptCatalogCache,
}

impl ChatContextCache {
    /// Captures the current inputs for one immutable later SamplingStepPlan.
    /// Facts are refreshed by content identity. Instructions come from the
    /// durable binding and change only through an explicit reload.
    pub(crate) fn refresh(
        &mut self,
        project: &Project,
        tools: &ToolRegistry,
        binding: &AgentDefinitionBinding,
    ) -> Result<CurrentContextState, ChatContextRefreshError> {
        let world_state = if binding.definition().research().is_some() {
            Some(self.current_world_state(project)?)
        } else {
            load_project_stream_integrity(project)
                .map_err(ChatContextRefreshError::ResearchProjection)?;
            None
        };
        let snapshot = LocalProjectSnapshot::new(project)
            .inspect(&tools.specs(), &[], "context-refresh")
            .map_err(ChatContextRefreshError::ProjectSnapshot)?;
        let project_snapshot = ProjectSnapshotContext::new(snapshot.snapshot)?;
        let core_memory = load_core_memory(project, binding)?;
        Ok(CurrentContextState::new(
            world_state,
            project_snapshot,
            core_memory,
        ))
    }

    fn current_world_state(
        &mut self,
        project: &Project,
    ) -> Result<ResearchWorldState, ChatContextRefreshError> {
        // A concurrent writer is outside the one-Chat lease normal path, but
        // never cache an ambiguous read. Two matching exact revisions prove
        // that the projection came from stable source bytes.
        for _ in 0..2 {
            let before = ResearchProjectionRevision::capture(project)?;
            let (references, receipt_revision) = self
                .receipt_cache
                .refresh(project)
                .map_err(ChatContextRefreshError::ResearchProjection)?;
            let before = before.with_receipt_catalog(receipt_revision);
            if self.research_revision.as_ref() == Some(&before) {
                return self
                    .world_state
                    .clone()
                    .ok_or(ChatContextRefreshError::MissingCachedWorldState);
            }
            let state = load_research_state_with_references(project, &references)
                .map_err(ChatContextRefreshError::ResearchProjection)?;
            let world_state = ResearchWorldState::from_research_state(&state)?;
            let (_, receipt_revision) = self
                .receipt_cache
                .refresh(project)
                .map_err(ChatContextRefreshError::ResearchProjection)?;
            let after = ResearchProjectionRevision::capture(project)?
                .with_receipt_catalog(receipt_revision);
            if before == after {
                self.research_revision = Some(after);
                self.world_state = Some(world_state.clone());
                return Ok(world_state);
            }
        }
        Err(ChatContextRefreshError::ResearchChangedDuringRefresh)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ResearchProjectionRevision {
    research_digest: String,
    receipt_catalog_digest: String,
}

impl ResearchProjectionRevision {
    fn capture(project: &Project) -> Result<Self, ChatContextRefreshError> {
        Ok(Self {
            research_digest: digest_file(&project.research_stream_path_existing()?)?,
            receipt_catalog_digest: String::new(),
        })
    }

    fn with_receipt_catalog(mut self, receipt_catalog_digest: String) -> Self {
        self.receipt_catalog_digest = receipt_catalog_digest;
        self
    }
}

fn digest_file(path: &Path) -> Result<String, ChatContextRefreshError> {
    let bytes = fs::read(path).map_err(|source| ChatContextRefreshError::ReadRevision {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}

fn load_core_memory(
    project: &Project,
    binding: &AgentDefinitionBinding,
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
    let document = LocalProjectMemory::with_scope(project, scope).read(&path)?;
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
    #[error("rebuild Research projection for current Context: {0}")]
    ResearchProjection(String),
    #[error("Research source changed during Context refresh; retry the Turn")]
    ResearchChangedDuringRefresh,
    #[error("unchanged Research revision did not retain a cached WorldState")]
    MissingCachedWorldState,
    #[error("read Context source revision `{path}`: {source}")]
    ReadRevision {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
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
