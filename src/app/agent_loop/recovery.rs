//! Narrow stop classification; neither this view nor the CLI retries work.

use super::{AgentLoop, AgentLoopError};
use crate::app::context_compilation::ContextCompilationError;
use crate::app::model_streaming::ModelStreamingError;
use crate::domain::context::ContextPlannerError;
use crate::ports::model::ModelProvider;
use crate::ports::tool::ToolExecutor;

impl<Provider, Tools> AgentLoop<'_, '_, Provider, Tools>
where
    Provider: ModelProvider + ?Sized,
    Tools: ToolExecutor + ?Sized,
{
    pub(crate) fn permits_explicit_recovery(&self, error: &AgentLoopError) -> bool {
        if self.history.ensure_all_tool_calls_paired().is_err() {
            return false;
        }
        match error {
            AgentLoopError::ModelRequestLimit { .. }
            | AgentLoopError::ProviderContinuationUnavailable { .. }
            | AgentLoopError::EmptyModelResponse
            | AgentLoopError::ContinuationIdentityMissing
            | AgentLoopError::VisionProviderStopped { .. }
            | AgentLoopError::Model(
                ModelStreamingError::Provider(_) | ModelStreamingError::EndedBeforeCompletion,
            ) => true,
            AgentLoopError::ContextPlanner(error)
            | AgentLoopError::ContextCompilation(ContextCompilationError::Planner(error)) => {
                matches!(
                    error,
                    ContextPlannerError::FixedInputExceedsBudget { .. }
                        | ContextPlannerError::RequiredFragmentsExceedBudget { .. }
                        | ContextPlannerError::RenderedInputExceedsBudget { .. }
                )
            }
            _ => false,
        }
    }
}
