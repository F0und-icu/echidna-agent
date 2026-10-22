//! The fixed normal-completion RSI boundary; never runs during recovery.

use crate::adapters::agent_evolution::{EvolutionError, EvolutionStore};
use crate::adapters::mcp_http::McpCatalogTool;
use crate::adapters::project_event_store::read_existing_session_events;
use crate::adapters::tools::ToolRegistry;
use crate::app::agent_loop::AgentLoop;
use crate::domain::session::SessionReducer;
use crate::provider_composition::ProviderComposition;

use super::{ChatError, DurableChatSession, definition};

pub(super) fn apply_completed<Provider: crate::ports::model::ModelProvider + ?Sized>(
    session: &mut DurableChatSession,
    agent: &mut AgentLoop<'_, '_, Provider, ToolRegistry>,
    tools: &ToolRegistry,
    composition: &ProviderComposition,
    discovered_mcp: &[McpCatalogTool],
    no_pending_images: bool,
) -> Result<bool, ChatError> {
    if session.binding.definition().rsi().is_none() {
        return Ok(false);
    }
    if tools.has_active_processes() || !no_pending_images {
        eprintln!("[RSI deferred] Active processes or queued images retain pending candidates.");
        return Ok(false);
    }
    agent
        .history()
        .ensure_all_tool_calls_paired()
        .map_err(crate::domain::session::SessionError::from)?;
    session.validate_stopped_runtime(agent)?;
    let events = read_existing_session_events(&session.project, &session.session_id)
        .map_err(definition::error)?;
    let state = SessionReducer.reduce(events)?;
    let store = EvolutionStore::new(&session.project, &session.binding);
    let candidate = match store.pending(&session.binding, &session.session_id, &state) {
        Ok(Some(candidate)) => candidate,
        Ok(None) => return Ok(false),
        Err(EvolutionError::Rejected(message)) => {
            eprintln!("[RSI rejected] {message}; candidate retained.");
            return Ok(false);
        }
        Err(error) => return Err(error.into()),
    };
    let preview = DurableChatSession::new(
        session.project.clone(),
        session.session_id.clone(),
        candidate,
    )?;
    if let Err(error) = preview.preflight_binding(&state, composition, discovered_mcp) {
        if is_budget_error(&error) {
            eprintln!("[RSI rejected] preflight: {error}; candidate retained.");
            return Ok(false);
        }
        return Err(error);
    }
    {
        let _writer = session.project.open_writer()?;
        store.save_revision(&preview.binding)?;
    }
    // These stores cannot form one transaction: immutable revision first,
    // durable Session transition second, publication pointer third. A later
    // failure is fatal and keeps the pending file for explicit reconciliation.
    let (state, provider_session, _, facts) =
        preview.start_or_resume(true, composition, true, discovered_mcp)?;
    store.finish(&preview.binding, &session.session_id)?;
    let validation = facts.as_ref().map(|facts| facts.context());
    agent.queue_prompt_composition(definition::prompt_for_state(
        &session.project,
        &preview.binding,
        &state,
        session.session_id.as_str(),
    )?);
    agent.resume_from_session(&state, provider_session, validation.as_ref())?;
    session.binding = preview.binding;
    eprintln!(
        "[RSI applied] revision {}; next request uses the new instructions. Structural publication permits a trial; improvement remains unverified.",
        session.binding.definition().digest().as_str()
    );
    Ok(true)
}

pub(super) fn is_budget_error(error: &ChatError) -> bool {
    matches!(
        error,
        ChatError::Agent(crate::app::agent_loop::AgentLoopError::ContextPlanner(
            crate::domain::context::ContextPlannerError::FixedInputExceedsBudget { .. }
                | crate::domain::context::ContextPlannerError::RequiredFragmentsExceedBudget { .. }
                | crate::domain::context::ContextPlannerError::RenderedInputExceedsBudget { .. }
        ))
    )
}
