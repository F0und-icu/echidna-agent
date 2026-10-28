//! The fixed normal-completion RSI boundary; never runs during recovery.

use crate::adapters::agent_evolution::{EvolutionError, EvolutionStore};
use crate::adapters::mcp::McpCatalogTool;
use crate::adapters::tools::ToolRegistry;
use crate::app::agent_loop::AgentLoop;
use crate::provider_composition::ProviderComposition;

use super::{ChatError, DurableChatSession, definition};

pub(super) enum CheckpointOutcome {
    Applied(Box<crate::domain::agent_definition::AgentDefinitionBinding>),
    Deferred { failed_handoff: bool },
}

/// Applies only the exact candidate returned by this checkpoint's accepted
/// contrast. A pre-existing passive pending file never enters this path.
#[allow(clippy::too_many_arguments)]
pub(super) async fn apply_checkpoint<Provider: crate::ports::model::ModelProvider + ?Sized>(
    session: &DurableChatSession,
    current: &crate::domain::agent_definition::AgentDefinitionBinding,
    accepted: super::improvement::AcceptedImprovement,
    agent: &mut AgentLoop<'_, '_, Provider, ToolRegistry>,
    active: &crate::app::agent_loop::ToolBatchContext<'_>,
    window_tokens: u32,
    request: super::compaction::CompactionRequestContext<'_>,
) -> Result<CheckpointOutcome, ChatError> {
    let store = EvolutionStore::new(&session.project, current);
    let state = session.durable_state()?;
    state.improvement_evidence(&accepted.attempt_id, current.definition().digest().as_str())?;
    if store
        .pending(current, &session.session_id, &state)?
        .as_ref()
        != Some(&accepted.binding)
    {
        return Err(super::definition::error(
            "Accepted RSI candidate changed before checkpoint application",
        ));
    }
    let prompt = definition::prompt_for_state(
        &session.project,
        &accepted.binding,
        &state,
        session.session_id.as_str(),
    )?;
    let cancellation = request.cancellation.clone();
    let admitted_before = request.budget.counts().primary;
    let budget = request.budget;
    if let Err(error) = session
        .compact_for_definition(agent, window_tokens, active, &prompt, request)
        .await
    {
        if super::compaction::automatically_deferred(&error) {
            console_eprintln!(
                "[RSI checkpoint deferred] {error}; candidate remains pending and old instructions remain active."
            );
            return Ok(CheckpointOutcome::Deferred {
                failed_handoff: super::compaction::failed_handoff(&error, admitted_before, budget),
            });
        }
        return Err(error);
    }
    if cancellation.is_cancelled() {
        return Ok(CheckpointOutcome::Deferred {
            failed_handoff: false,
        });
    }
    let state = session.durable_state()?;
    if store
        .pending(current, &session.session_id, &state)?
        .as_ref()
        != Some(&accepted.binding)
    {
        return Err(super::definition::error(
            "Accepted RSI pending candidate changed during handoff",
        ));
    }
    {
        let _writer = session.project.open_writer()?;
        store.save_revision(&accepted.binding)?;
    }
    session.with_projection(|projection, sink, stream| {
        let state = projection.state();
        let compaction = state
            .latest_compaction()
            .ok_or(crate::domain::session::SessionError::CompactionSourceSequenceMissing)?;
        let revision = compaction.summary().revision();
        let fingerprint = compaction.fingerprint()?;
        let sequence = sink.next_sequence()?;
        let provider = crate::domain::context::ProviderSession::new(
            crate::domain::context::ProviderSessionId::new(format!(
                "primary:{}:checkpoint:{}",
                session.session_id,
                sequence.get()
            )),
            crate::domain::context::ProviderSessionRole::Primary,
            sequence.get(),
        );
        let event = crate::domain::session::SessionEvent::AgentDefinitionAppliedAtCheckpoint {
            primary_provider_session: provider.reference().clone(),
            binding: accepted.binding.clone(),
            attempt_id: accepted.attempt_id.clone(),
            parent_digest: current.definition().digest().as_str().to_owned(),
            compaction_revision: revision,
            compaction_fingerprint: fingerprint,
        }
        .into_envelope(
            super::event_id(
                session.project.project_id(),
                &session.session_id,
                sequence.get(),
                "agent-checkpoint-applied",
            ),
            stream,
            sequence,
            super::occurred_at(),
        )?;
        projection
            .append(sink, event)
            .map_err(ChatError::from_session_append)?;
        agent.install_checkpoint_definition(projection.state(), prompt, provider)?;
        Ok(())
    })?;
    store.finish(&accepted.binding, &session.session_id)?;
    console_eprintln!(
        "[RSI checkpoint applied] revision {}; next request in this Turn uses the new instructions; task and budget retained.",
        accepted.binding.definition().digest().as_str()
    );
    Ok(CheckpointOutcome::Applied(Box::new(accepted.binding)))
}

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
        console_eprintln!(
            "[RSI deferred] Active processes or queued images retain pending candidates."
        );
        return Ok(false);
    }
    agent
        .history()
        .ensure_all_tool_calls_paired()
        .map_err(crate::domain::session::SessionError::from)?;
    session.validate_stopped_runtime(agent)?;
    let state = session.durable_state()?;
    let store = EvolutionStore::new(&session.project, &session.binding);
    let candidate = match store.pending(&session.binding, &session.session_id, &state) {
        Ok(Some(candidate)) => candidate,
        Ok(None) => return Ok(false),
        Err(EvolutionError::Rejected(message)) => {
            console_eprintln!("[RSI rejected] {message}; candidate retained.");
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
            console_eprintln!("[RSI rejected] preflight: {error}; candidate retained.");
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
    let (state, provider_session, _, facts, capacity_required) =
        preview.start_or_resume(true, composition, true, discovered_mcp)?;
    if capacity_required {
        return Err(ChatError::CompactionNotUseful);
    }
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
    console_eprintln!(
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
