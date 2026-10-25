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
