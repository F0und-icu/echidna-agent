//! CLI presentation and strict durable handoff for App-classified stops.

use super::{ChatError, DurableChatSession, unresolved_receipted_tool_call};
use crate::app::agent_loop::{AgentLoop, AgentLoopError};
use crate::app::model_request_budget::ModelRequestBudget;
use crate::app::model_streaming::ModelStreamingError;
use crate::domain::session::SessionState;
use crate::ports::model::{ModelProvider, ModelProviderError};
use crate::ports::tool::ToolExecutor;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RecoveryState {
    Ready,
    ResumeRequired { capacity: bool },
    ContextChangeRequired,
}

pub(super) fn is_capacity_error(error: &AgentLoopError) -> bool {
    use crate::app::context_compilation::ContextCompilationError;
    use crate::domain::context::ContextPlannerError;
    match error {
        AgentLoopError::Model(ModelStreamingError::Provider(
            ModelProviderError::ContextCapacityExceeded { .. },
        )) => true,
        AgentLoopError::ContextPlanner(error)
        | AgentLoopError::ContextCompilation(ContextCompilationError::Planner(error)) => matches!(
            error,
            ContextPlannerError::FixedInputExceedsBudget { .. }
                | ContextPlannerError::RequiredFragmentsExceedBudget { .. }
                | ContextPlannerError::RenderedInputExceedsBudget { .. }
        ),
        _ => false,
    }
}

pub(super) fn report_budget(budget: &ModelRequestBudget) {
    let counts = budget.counts();
    let display = |value: Option<usize>| {
        value.map_or_else(|| "unlimited".to_owned(), |value| value.to_string())
    };
    console_println!(
        "[request budget] admitted={} primary={} vision={} remaining={} limit={} (admission, not Provider usage)",
        counts.total(),
        counts.primary,
        counts.vision,
        display(budget.remaining()),
        display(budget.limit())
    );
}

pub(super) fn report_stop(error: &AgentLoopError, budget: &ModelRequestBudget) {
    if let Some(display) = crate::chat_display::JsonLines::current() {
        let _ = display.send(crate::chat_display::Event::Turn { state: "stopped" });
    }
    console_eprintln!("[turn stopped] {error}");
    report_budget(budget);
    if matches!(
        error,
        AgentLoopError::Model(ModelStreamingError::Provider(
            ModelProviderError::ProtocolFailure { .. }
        ))
    ) {
        console_println!(
            "Protocol sample was rejected before completion; its streamed output was not committed and no Tool from this sample was dispatched."
        );
    }
    console_println!(
        "Durable checkpoint retained. Use /resume to rebuild ProviderSession explicitly, then enter a new task. No further automatic retry or reasoning discard."
    );
}

pub(super) fn exit_status(state: RecoveryState) -> Result<(), ChatError> {
    if state != RecoveryState::Ready {
        Err(ChatError::UnresolvedTurnStop)
    } else {
        Ok(())
    }
}

pub(super) fn validate_runtime_history<Provider, Tools>(
    state: &SessionState,
    agent: &AgentLoop<'_, '_, Provider, Tools>,
) -> Result<(), ChatError>
where
    Provider: ModelProvider + ?Sized,
    Tools: ToolExecutor + ?Sized,
{
    state.ensure_no_unknown_improvement()?;
    let history = state.compaction_source_history()?;
    if agent.history().items() != history {
        return Err(ChatError::RuntimeHistoryMismatch);
    }
    Ok(())
}

impl DurableChatSession {
    pub(super) fn validate_stopped_runtime<Provider, Tools>(
        &self,
        agent: &AgentLoop<'_, '_, Provider, Tools>,
    ) -> Result<(), ChatError>
    where
        Provider: ModelProvider + ?Sized,
        Tools: ToolExecutor + ?Sized,
    {
        self.with_projection(|projection, _, _| {
            if let Some(call_id) = unresolved_receipted_tool_call(projection.state()) {
                return Err(ChatError::UnresolvedExecutionReceipt {
                    call_id: call_id.as_str().to_owned(),
                });
            }
            validate_runtime_history(projection.state(), agent)
        })
    }
}
