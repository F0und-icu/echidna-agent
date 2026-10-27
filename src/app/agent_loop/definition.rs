//! Offline definition admission through the existing Context compiler.

use std::collections::BTreeMap;

use crate::domain::context::ContextPlan;
use crate::domain::model::ContentItem;
use crate::domain::research_control::PromptComposition;
use crate::domain::session::SessionSummary;
use crate::domain::tools::DeferredActivationSet;
use crate::ports::model::ModelProvider;
use crate::ports::tool::ToolExecutor;

use super::{AgentLoop, AgentLoopError, context_state};

impl<Provider, Tools> AgentLoop<'_, '_, Provider, Tools>
where
    Provider: ModelProvider + ?Sized,
    Tools: ToolExecutor + ?Sized,
{
    /// Installs a durably accepted instruction checkpoint after its exact
    /// compaction retired parent delivery. It never reconstructs or Resumes a Turn.
    pub(crate) fn install_checkpoint_definition(
        &mut self,
        state: &crate::domain::session::SessionState,
        prompt: PromptComposition,
        mut provider: crate::domain::context::ProviderSession,
    ) -> Result<(), AgentLoopError> {
        let compaction = state
            .latest_compaction()
            .ok_or(AgentLoopError::ContextStateMismatch)?;
        let applied = super::compaction::AppliedCompaction::from_compaction(
            state
                .stream_id()
                .ok_or(AgentLoopError::ContextStateMismatch)?
                .clone(),
            compaction,
        )?;
        if self.committed_delivery.is_some()
            || self.applied_compaction.as_ref() != Some(&applied)
            || state.primary_provider_session() != Some(provider.reference())
            || state.normalized_history_items()? != self.history.items()
            || prompt.tool_exposure_for_catalog(&self.tools.catalog())?
                != self.active_tool_exposure_plan
        {
            return Err(AgentLoopError::ContextStateMismatch);
        }
        provider.reset(crate::domain::context::BaselineInvalidationReason::CompactionReset);
        self.prompt_composition = prompt;
        self.pending_prompt_composition = None;
        self.provider_session = provider;
        Ok(())
    }

    /// Validates a candidate against the same budgets and tool plan used by
    /// sampling. It does not change History, delivery, or the active binding.
    pub(crate) fn preflight_definition(
        &self,
        prompt: &PromptComposition,
        history: &[ContentItem],
        summary: Option<&SessionSummary>,
    ) -> Result<ContextPlan, AgentLoopError> {
        let capabilities = self.provider.capabilities();
        let budget = self.context_planner.default_budget(&capabilities)?;
        let exposure = prompt.tool_exposure_for_catalog(&self.tools.catalog())?;
        let tools = self
            .tools
            .resolve_tool_plan(&exposure, &DeferredActivationSet::default())?;
        let mut items = history.to_vec();
        items.push(ContentItem::user_text("Continue."));
        let (plan, _) = context_state::compile_context(context_state::ContextCompilationInput {
            prompt_composition: prompt,
            world_state: self.world_state.as_ref(),
            project_snapshot: self.project_snapshot.as_ref(),
            project_memory_core: self.project_memory_core.as_ref(),
            session_summary: summary,
            history: &items,
            history_provenance: &BTreeMap::new(),
            turn_history_start: history.len(),
            detached_input: None,
            planner: &self.context_planner,
            cache: None,
            budget,
            capabilities: &capabilities,
            tool_specs: tools.advertised_specs(),
            prior_manifest: None,
            correction_feedback: &[],
        })?;
        Ok(plan)
    }
}
