//! CLI presentation and strict durable handoff for App-classified stops.

use super::{ChatError, DurableChatSession, unresolved_receipted_tool_call};
use crate::adapters::project_event_store::ProjectEventStream;
use crate::app::agent_loop::{AgentLoop, AgentLoopError};
use crate::app::model_request_budget::ModelRequestBudget;
use crate::app::session::SessionProjection;
use crate::domain::session::{SessionError, SessionState};
use crate::ports::EventSource;
use crate::ports::model::ModelProvider;
use crate::ports::tool::ToolExecutor;

pub(super) fn report_budget(budget: &ModelRequestBudget) {
    let counts = budget.counts();
    println!(
        "[request budget] admitted={} primary={} vision={} remaining={} limit={} (admission, not Provider usage)",
        counts.total(),
        counts.primary,
        counts.vision,
        budget.limit().saturating_sub(counts.total()),
        budget.limit()
    );
}

pub(super) fn report_stop(error: &AgentLoopError, budget: &ModelRequestBudget) {
    eprintln!("[turn stopped] {error}");
    report_budget(budget);
    println!(
        "Durable checkpoint retained. Use /resume to rebuild ProviderSession explicitly, then enter a new task. No automatic retry or reasoning discard."
    );
}

pub(super) fn exit_status(stopped: bool) -> Result<(), ChatError> {
    if stopped {
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
    let history = state
        .normalized_history_items()
        .map_err(SessionError::from)?;
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
        let mut writer = self.project.open_writer()?;
        let store = writer.open_session(self.session_id.clone(), None)?;
        let stream = ProjectEventStream::Session(self.session_id.clone()).stream_id();
        let projection = SessionProjection::replay(EventSource::read_events(&store, &stream)?)?;
        if let Some(call_id) = unresolved_receipted_tool_call(projection.state()) {
            return Err(ChatError::UnresolvedExecutionReceipt {
                call_id: call_id.as_str().to_owned(),
            });
        }
        validate_runtime_history(projection.state(), agent)
    }
}
