//! Durable first-version CLI Chat composition over the one AgentLoop.
//!
//! This UI composition module deliberately owns only the CLI-facing Session handoff: it
//! verifies the authoritative JSONL prefix before applying newly appended
//! facts, including independent Tool Result Cache receipts. Context, History,
//! RequestAudit, Tool execution and
//! model streaming retain their existing owners.

// These macros route the existing CLI presentation through the active Chat
// console. Outside a capable terminal the original streams/bytes are retained.
macro_rules! console_print {
    ($($arg:tt)*) => {{ use std::io::Write as _; write!(crate::terminal_console::stdout(), $($arg)*).expect("CLI stdout failed"); }};
}
macro_rules! console_println {
    () => { console_print!("\n") };
    ($($arg:tt)*) => { console_print!("{}\n", format_args!($($arg)*)) };
}
macro_rules! console_eprintln {
    ($($arg:tt)*) => {{ use std::io::Write as _; writeln!(crate::terminal_console::stderr(), $($arg)*).expect("CLI stderr failed"); }};
}

mod compaction;
mod compaction_graph;
mod context;
mod context_read;
mod definition;
mod evolution;
mod improvement;
mod input;
mod memory;
mod persistence;
mod recovery;
mod task_context;
mod task_index;
mod task_posting_lengths;
mod theme;
mod timing;
mod tool_output;
mod usage;
mod vision;

use std::cell::Cell;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use thiserror::Error;

use crate::adapters::artifact_store::LocalArtifactStore;
use crate::adapters::fragment_cache::LocalFragmentCache;
use crate::adapters::project_event_store::{Project, ProjectEventStream};
use crate::adapters::research_projection::CompactionValidationFactsError;
use crate::adapters::session_compaction::{
    SessionCompactionFacts, load_historical_session_compaction_facts, load_resume_compaction_facts,
};
use crate::app::agent_loop::{
    AgentLoop, AgentLoopError, AgentTurnOutcome, AssistantTextDeltaStage, ChatDurabilityCheckpoint,
    ChatDurabilityStage, CliRuntimeStages, ToolExecutionStartStage, ViewImageFollowUpFuture,
};
use crate::app::cancellation::AgentCancellation;
use crate::app::model_request_budget::ModelRequestBudget;
use crate::app::request_audit::RequestAuditService;
use crate::app::session::{AppendAfterDurableError, SessionProjection};
use crate::domain::agent_definition::AgentDefinitionBinding;
use crate::domain::context::{ProviderSession, ProviderSessionId, ProviderSessionRole};
use crate::domain::domain_profile::{DomainProfile, DomainProfileSelection};
use crate::domain::events::{EventId, OccurredAt, StreamId};
use crate::domain::project::ProjectId;
use crate::domain::request_audit::{
    RequestAuditAttempt, RequestAuditBinding, RequestAuditManifest, RequestAuditRunContext,
};
use crate::domain::research_control::{ResearchPhase, ResearchPhaseSelection};
use crate::domain::session::{SessionError, SessionEvent, SessionLifecycle, SessionState};
use crate::domain::tools::ToolExecutionOutcome;
use crate::provider_composition::ProviderComposition;

static NEXT_CHAT_SESSION: AtomicU64 = AtomicU64::new(1);

/// Runs the fixed line-oriented CLI path. The caller has already loaded the
/// one configuration-driven ProviderComposition; this path never constructs a
/// Provider, URL, model, credential source, or protocol Adapter itself.
pub(crate) async fn run(
    project: Project,
    composition: &ProviderComposition,
    mut requested_agent: Option<PathBuf>,
    mut requested_session: Option<String>,
    mut requested_domain: Option<DomainProfile>,
    mut requested_phase: Option<ResearchPhase>,
) -> Result<(), ChatError> {
    let startup_timing = timing::TurnTiming::new(composition.diagnostic_output_enabled());
    let _runtime_lease = project.acquire_chat_runtime_lease()?;
    let discovered_mcp = composition.discover_chat_mcp().await?;
    startup_timing.checkpoint("resume_discovery");
    let terminal_input = io::stdin().is_terminal();
    let mut input_closed = false;
    let mut pending_input = None;
    let mut chat_input = None;
    let mut next_binding = None;
    'sessions: loop {
        let explicit_resume = requested_session.is_some();
        let session_id = match requested_session.take() {
            Some(session_id) => StreamId::new(session_id),
            None => new_session_id(),
        };
        let (binding, projection) = match next_binding.take() {
            Some(binding) => (binding, None),
            None => definition::initial_binding(
                &project,
                &session_id,
                explicit_resume,
                requested_agent.take().as_deref(),
                composition,
                &discovered_mcp,
            )?,
        };
        let research_enabled = binding.definition().research().is_some();
        startup_timing.checkpoint("resume_projection");
        if !research_enabled && (requested_domain.is_some() || requested_phase.is_some()) {
            return Err(definition::error(
                "--domain/--phase require a research-enabled Agent definition",
            ));
        }
        let mut session = DurableChatSession::new(project.clone(), session_id.clone(), binding)?;
        session.projection = Arc::new(std::sync::Mutex::new(projection));
        crate::adapters::agent_evolution::EvolutionStore::new(&session.project, &session.binding)
            .prepare(&session.binding)?;
        let (
            session_state,
            provider_session,
            mut disposition,
            compaction_validation,
            mut capacity_required,
        ) = session.start_or_resume(explicit_resume, composition, false, &discovered_mcp)?;
        startup_timing.checkpoint("resume_binding");
        let domain = requested_domain.unwrap_or_else(|| {
            session_state
                .domain_profile_selection()
                .map(|selection| selection.profile())
                .or_else(|| {
                    session
                        .binding
                        .definition()
                        .research()
                        .map(|research| research.domain())
                })
                .unwrap_or(DomainProfile::Source)
        });

        let tools = session.bound_tools(&discovered_mcp, composition)?;
        let mut context_cache = context::ChatContextCache::default();
        let mut agent = AgentLoop::with_primary_context(
            composition.primary_provider(),
            &tools,
            composition.agent_profile().primary().model().clone(),
            composition.agent_profile().primary().profile().clone(),
            provider_session.clone(),
        )?
        .with_context_budget_policy(composition.runtime().context_budget)
        .with_fragment_cache(Arc::new(LocalFragmentCache::for_project(&session.project)))
        .with_output_store(LocalArtifactStore::for_project(&session.project))
        .with_prompt_composition(definition::prompt_for_state(
            &session.project,
            &session.binding,
            &session_state,
            session_id.as_str(),
        )?);
        if research_enabled {
            agent.queue_domain_profile(domain);
        }
        agent.queue_agent_model_profile(composition.agent_profile().clone())?;
        if disposition == SessionDisposition::Resumed {
            let validation = compaction_validation.as_ref().map(|facts| facts.context());
            agent.resume_from_session(&session_state, provider_session, validation.as_ref())?;
            let current = session.refresh_context(&mut context_cache, &tools)?;
            agent.refresh_current_context(current)?;
            task_context::TaskContextRuntime::new(&session).restore_after_resume(&mut agent)?;
            startup_timing.checkpoint("resume_task_context");
            if capacity_required && agent.has_task_context() {
                capacity_required = agent.fresh_compaction_context_diagnostic()?.is_err();
            }
        }

        let binding = RequestAuditBinding::new(
            session.project.project_id().clone(),
            ProjectEventStream::Session(session_id.clone()).stream_id(),
        )?;
        let audit_context = RequestAuditRunContext::new(
            binding,
            composition.primary_request_adapter_version().clone(),
        );

        let theme = theme::Theme::detect();
        theme.welcome();
        if research_enabled {
            console_println!(
                "{} session `{}` with domain `{}` and phase `{}`. Enter a line, or use /paste, /image, /domain, /phase, /context, /memory, /compact, /resume, /stop, /exit or EOF. While running in a terminal, /stop or a new instruction interrupts the current Turn before further input is handled.",
                disposition.message(),
                session_id.as_str(),
                domain.id(),
                requested_phase
                    .unwrap_or_else(|| session_state.effective_research_phase())
                    .id(),
            );
        } else {
            console_println!(
                "{} session `{}` with Agent `{}`. Enter a task, or use /agent, /agent reload, /agent use <path>, /memory, /image, /compact, /resume or /exit.",
                disposition.message(),
                session_id.as_str(),
                session.binding.definition().id().as_str()
            );
        }
        // Publish the first editable prompt only after the Session is ready.
        // Agent switches retain this same terminal and its restoration guard.
        if chat_input.is_none() {
            chat_input = Some(input::ChatInput::new()?);
        }
        let lines = chat_input.as_mut().expect("Chat input was initialized");
        let mut pending_domain = requested_domain.map(DomainProfileSelection::operator);
        let mut pending_phase = requested_phase.map(ResearchPhaseSelection::operator);
        let mut pending_images: Vec<vision::PendingImage> = Vec::new();
        let mut recovery_state = if capacity_required {
            recovery::RecoveryState::ContextChangeRequired
        } else {
            recovery::RecoveryState::Ready
        };
        let committed_step = std::cell::RefCell::new(
            None::<(String, EventId, crate::domain::context::ProviderSessionRef)>,
        );
        let tool_output = std::cell::RefCell::new(tool_output::ToolOutput::default());
        loop {
            lines.prompt(crate::terminal_console::InputMode::Task)?;
            let follow_up = pending_input.is_some();
            let line = if pending_input.is_some() {
                pending_input.take()
            } else if input_closed {
                None
            } else {
                tokio::select! {
                    line = lines.next_line() => match line {
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                            console_println!("\nInterrupted while waiting for input; Session `{}` remains active.", session_id.as_str());
                            return recovery::exit_status(recovery_state);
                        }
                        result => result?,
                    },
                    signal = tokio::signal::ctrl_c() => {
                        signal?;
                        console_println!("\nInterrupted while waiting for input; Session `{}` remains active.", session_id.as_str());
                        return recovery::exit_status(recovery_state);
                    }
                }
            };
            let Some(received) = line else {
                console_println!(
                    "EOF received; Session `{}` remains active.",
                    session_id.as_str()
                );
                return recovery::exit_status(recovery_state);
            };
            let mut input_time = received.time;
            let line = received.text;
            if line == "/paste" && recovery_state != recovery::RecoveryState::Ready {
                console_println!("Task input not accepted or saved while recovery is required.");
                continue;
            }
            let line = if line == "/paste" {
                console_println!("Paste mode; finish with /end on its own line.");
                let mut pasted = Vec::new();
                let pasted = loop {
                    lines.prompt(crate::terminal_console::InputMode::Paste)?;
                    let next = tokio::select! {
                        line = lines.next_line() => match line {
                            Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                                console_println!("\n[paste cancelled]");
                                pasted.clear();
                                break None;
                            }
                            result => result?,
                        },
                        signal = tokio::signal::ctrl_c() => {
                            signal?;
                            console_println!("\n[paste cancelled]");
                            pasted.clear();
                            break None;
                        }
                    };
                    match next {
                        Some(value) if value.text == "/end" => {
                            input_time = value.time;
                            break Some(pasted.join("\n"));
                        }
                        Some(value) => pasted.push(value.text),
                        None => {
                            console_println!(
                                "EOF received during paste; Session `{}` remains active and the incomplete paste was discarded.",
                                session_id.as_str()
                            );
                            return recovery::exit_status(recovery_state);
                        }
                    }
                };
                lines.end_paste()?;
                pasted
            } else {
                Some(line)
            };
            let Some(line) = line else {
                continue;
            };
            if line.is_empty() {
                console_println!("Empty input ignored.");
                continue;
            }
            if line == "/exit" {
                console_println!(
                    "Session `{}` remains active for explicit resume.",
                    session_id.as_str()
                );
                return recovery::exit_status(recovery_state);
            }
            if recovery_state != recovery::RecoveryState::Ready
                && line != "/context"
                && line != "/resume"
                && !(recovery_state == recovery::RecoveryState::ContextChangeRequired
                    && line == "/compact")
            {
                match recovery_state {
                    recovery::RecoveryState::ResumeRequired { .. } => console_println!(
                        "Task input not accepted or saved: use /resume first, or /context /exit. No old input or Tool will be retried."
                    ),
                    recovery::RecoveryState::ContextChangeRequired => console_println!(
                        "Task input not accepted or saved: use /context, /compact or /exit. No old input or Tool will be retried."
                    ),
                    recovery::RecoveryState::Ready => unreachable!(),
                }
                continue;
            }
            if line == "/stop" {
                console_println!(
                    "No Turn is running. Background Jobs/PTYs, if any, are not stopped by /stop."
                );
                continue;
            }
            if line == "/context" {
                if !matches!(
                    recovery_state,
                    recovery::RecoveryState::ResumeRequired { .. }
                ) {
                    let current = session.refresh_context(&mut context_cache, &tools)?;
                    agent.refresh_current_context(current)?;
                    match agent.fresh_compaction_context_diagnostic()? {
                        Ok(plan) => console_println!(
                            "[current refreshed candidate] {}",
                            compaction::ContextPressure::from_plan(&plan).message()
                        ),
                        Err((error, breakdown)) => {
                            let budget = agent.context_budget()?;
                            console_println!(
                                "[current refreshed candidate] {error}; window={}, output reserve={}, tool-loop reserve={}, full input budget B={}. Committed delivery estimate={} (includes retained reasoning/opaque when present and overlaps the logical candidate; do not add them). Image token cost is unknown. The last successful plan below is not the current failure.",
                                budget.context_window_tokens(),
                                budget.reserved_output_tokens(),
                                budget.reserved_tool_loop_tokens(),
                                budget.input_token_limit(),
                                agent.delivery_input_estimate()?.map_or_else(
                                    || "unavailable".to_owned(),
                                    |value| value.to_string()
                                )
                            );
                            if let Some(detail) = breakdown {
                                let available = detail.fragment_budget.map_or_else(
                                    || "unavailable: tool schema exceeds B".to_owned(),
                                    |value| value.to_string(),
                                );
                                console_println!(
                                    "[current candidate components] tool schema={} tokens; fragment budget after tools={available}; instructions={}, Memory={}, Summary={}, current/protected input={}, History/results={}, Turn updates/results={}, Research/project facts={}; required fragments={}; image items={}, opaque references={}. Component values are local fragment estimates, not Provider token counts; History and updates may contain mixed item kinds.",
                                    detail.tool_schema_tokens,
                                    detail.instructions,
                                    detail.memory,
                                    detail.summary,
                                    detail.input,
                                    detail.history,
                                    detail.turn_updates,
                                    detail.project_facts,
                                    detail.required_fragments,
                                    detail.images,
                                    detail.opaque_items
                                );
                            } else {
                                console_println!(
                                    "[current candidate components] unavailable for this failure."
                                );
                            }
                        }
                    }
                }
                if recovery_state != recovery::RecoveryState::Ready {
                    console_println!(
                        "[original stopped candidate] breakdown unavailable; it was not recorded."
                    );
                }
                match agent.last_successful_context() {
                    Some(context) => {
                        console_println!(
                            "[last successful plan] {}",
                            compaction::ContextPressure::from_plan(context.plan()).message()
                        )
                    }
                    None => console_println!(
                        "No successful model plan is available; Provider usage is not inferred."
                    ),
                }
                continue;
            }
            if line == "/agent" {
                console_println!(
                    "{}",
                    serde_json::json!({
                        "agent_id": session.binding.definition().id(),
                        "definition_digest": session.binding.definition().digest(),
                        "source": session.binding.source_locator(),
                        "memory": session.binding.definition().context().memory(),
                        "research_enabled": research_enabled,
                        "session_id": session_id.as_str(),
                    })
                );
                continue;
            }
            if line == "/memory" {
                console_println!(
                    "{}",
                    memory::status(
                        &session.project,
                        &session.binding,
                        &tools,
                        agent.last_context_plan()
                    )?
                );
                continue;
            }
            if line == "/resume" {
                let recovery::RecoveryState::ResumeRequired { capacity } = recovery_state else {
                    console_println!(
                        "No stopped Turn needs recovery; no ProviderSession was replaced."
                    );
                    continue;
                };
                let (state, provider_session, resumed, validation, mut capacity_required) =
                    session.start_or_resume(true, composition, false, &discovered_mcp)?;
                let validation = validation.as_ref().map(|facts| facts.context());
                agent.resume_from_session(&state, provider_session, validation.as_ref())?;
                let current = session.refresh_context(&mut context_cache, &tools)?;
                agent.refresh_current_context(current)?;
                task_context::TaskContextRuntime::new(&session).restore_after_resume(&mut agent)?;
                if capacity_required && agent.has_task_context() {
                    capacity_required = agent.fresh_compaction_context_diagnostic()?.is_err();
                }
                disposition = resumed;
                recovery_state = if capacity || capacity_required {
                    recovery::RecoveryState::ContextChangeRequired
                } else {
                    recovery::RecoveryState::Ready
                };
                if recovery_state == recovery::RecoveryState::ContextChangeRequired {
                    console_println!(
                        "ProviderSession rebuilt after source validation. Context exceeds available capacity; use /context, /compact or /exit. No request sent or Tool retried."
                    );
                } else {
                    console_println!(
                        "ProviderSession rebuilt from durable History; volatile reasoning cleared by explicit /resume. No request sent or Tool retried. Enter a new instruction or /compact."
                    );
                }
                continue;
            }
            if line == "/agent reload"
                || line.starts_with("/agent reload ")
                || line.starts_with("/agent use ")
            {
                if !pending_images.is_empty() {
                    console_println!(
                        "Agent changes require no queued images; submit them or use /image cancel."
                    );
                    continue;
                }
                if tools.has_active_processes() {
                    console_println!(
                        "Agent changes require all running or stopping Jobs/PTYs to finish. Their existing controls remain available."
                    );
                    continue;
                }
                if let Err(error) = agent.history().ensure_all_tool_calls_paired() {
                    console_println!("Agent change rejected: {error}");
                    continue;
                }
                session.validate_stopped_runtime(&agent)?;
                if let Some(path) = line.strip_prefix("/agent use ") {
                    let path = path.trim();
                    if path.is_empty() {
                        console_println!("Usage: /agent use <path>");
                        continue;
                    }
                    let catalog = crate::adapters::tools::ToolRegistry::for_project_with_mcp(
                        session.project.root(),
                        &discovered_mcp,
                    )?
                    .with_project(&session.project);
                    let candidate = match crate::cli_agent::select_binding(
                        Some(Path::new(path)),
                        composition,
                        &catalog,
                    ) {
                        Ok(candidate) => candidate,
                        Err(error) => {
                            console_println!("Agent change rejected: {error}");
                            continue;
                        }
                    };
                    let preview = DurableChatSession::new(
                        session.project.clone(),
                        new_session_id(),
                        candidate.clone(),
                    )?;
                    if let Err(error) = preview.preflight_binding(
                        &SessionState::default(),
                        composition,
                        &discovered_mcp,
                    ) {
                        console_println!("Agent change rejected: {error}");
                        continue;
                    }
                    console_println!(
                        "Session `{}` remains available for Resume; opening a new Session with Agent `{}`.",
                        session_id.as_str(),
                        candidate.definition().id().as_str()
                    );
                    next_binding = Some(candidate);
                    requested_domain = None;
                    requested_phase = None;
                    continue 'sessions;
                }
                let path = line.strip_prefix("/agent reload ").map(str::trim);
                let catalog = crate::adapters::tools::ToolRegistry::for_project_with_mcp(
                    session.project.root(),
                    &discovered_mcp,
                )?
                .with_project(&session.project);
                let candidate = match definition::reload_candidate(
                    &session.binding,
                    path.map(Path::new),
                    composition,
                    &catalog,
                ) {
                    Ok(candidate) => candidate,
                    Err(error) => {
                        console_println!("Agent reload rejected: {error}");
                        continue;
                    }
                };
                let candidate = match crate::adapters::agent_evolution::EvolutionStore::new(
                    &session.project,
                    &candidate,
                )
                .select(candidate)
                {
                    Ok(candidate) => candidate,
                    Err(crate::adapters::agent_evolution::EvolutionError::Rejected(message)) => {
                        console_println!("Agent reload rejected: {message}");
                        continue;
                    }
                    Err(error) => return Err(error.into()),
                };
                let candidate_session = DurableChatSession::new(
                    session.project.clone(),
                    session_id.clone(),
                    candidate,
                )?;
                let state = candidate_session.durable_state()?;
                if let Err(error) =
                    candidate_session.preflight_binding(&state, composition, &discovered_mcp)
                {
                    console_println!("Agent reload rejected: {error}");
                    continue;
                }
                let (state, provider_session, resumed, facts, capacity_required) =
                    candidate_session.start_or_resume(true, composition, true, &discovered_mcp)?;
                if capacity_required {
                    return Err(ChatError::CompactionNotUseful);
                }
                let validation = facts.as_ref().map(|facts| facts.context());
                agent.queue_prompt_composition(definition::prompt_for_state(
                    &session.project,
                    &candidate_session.binding,
                    &state,
                    session_id.as_str(),
                )?);
                agent.resume_from_session(&state, provider_session, validation.as_ref())?;
                session.set_binding(candidate_session.binding)?;
                disposition = resumed;
                context_cache = context::ChatContextCache::default();
                console_println!(
                    "Agent `{}` reloaded at definition `{}`; ProviderSession rebuilt. No model request or Tool execution was started. Enter a new instruction.",
                    session.binding.definition().id().as_str(),
                    session.binding.definition().digest().as_str()
                );
                continue;
            }
            if line == "/rsi reconcile" {
                if session.binding.definition().rsi().is_none() {
                    console_println!("RSI is not enabled for this Agent.");
                    continue;
                }
                if tools.has_active_processes() || !pending_images.is_empty() {
                    console_println!(
                        "RSI reconciliation requires an idle boundary with no queued images."
                    );
                    continue;
                }
                agent
                    .history()
                    .ensure_all_tool_calls_paired()
                    .map_err(crate::domain::session::SessionError::from)?;
                session.validate_stopped_runtime(&agent)?;
                match crate::adapters::agent_evolution::EvolutionStore::new(
                    &session.project,
                    &session.binding,
                )
                .reconcile(&session.binding)
                {
                    Ok(()) => console_println!(
                        "RSI publication pointer agrees with this durable binding; no request, execution or Provider reset occurred."
                    ),
                    Err(crate::adapters::agent_evolution::EvolutionError::Rejected(message)) => {
                        console_println!("RSI reconciliation rejected: {message}")
                    }
                    Err(error) => return Err(error.into()),
                }
                continue;
            }
            if line == "/agent use" {
                console_println!("Usage: /agent use <path>");
                continue;
            }
            if line == "/image cancel" {
                let count = pending_images.len();
                pending_images.clear();
                console_println!(
                    "{}",
                    if count == 0 {
                        "No image is queued."
                    } else {
                        "Queued images detached; their durable Artifacts remain available."
                    }
                );
                continue;
            }
            if line == "/compact" {
                if let Some(image) = pending_images.first() {
                    console_println!(
                        "Compaction is unavailable while {} image(s), starting with `{}`, are queued; submit their Turn first.",
                        pending_images.len(),
                        image.delivery_artifact_id()
                    );
                    continue;
                }
                let current = session.refresh_context(&mut context_cache, &tools)?;
                agent.refresh_current_context(current)?;
                if !agent.has_task_context() {
                    task_context::TaskContextRuntime::new(&session)
                        .restore_after_resume(&mut agent)?;
                }
                let cancellation = AgentCancellation::new();
                let budget =
                    ModelRequestBudget::new(composition.runtime().model_request_limit_per_turn);
                lines.processing(false)?;
                let result = input::interruptible(
                    session.compact(
                        &mut agent,
                        composition
                            .agent_profile()
                            .primary()
                            .capabilities()
                            .context_window,
                        compaction::CompactionRequestContext {
                            composition,
                            budget: &budget,
                            cancellation: cancellation.clone(),
                            audit: &audit_context,
                        },
                    ),
                    &cancellation,
                    lines,
                    terminal_input,
                    &mut input_closed,
                    &mut pending_input,
                )
                .await?;
                match result {
                    Ok(summary) => {
                        console_println!("{}", summary.message());
                        if recovery_state == recovery::RecoveryState::ContextChangeRequired
                            && agent.fresh_compaction_context_probe()?.is_some()
                        {
                            recovery_state = recovery::RecoveryState::Ready;
                            console_println!(
                                "Context is locally admissible; enter a new instruction when ready."
                            );
                        }
                    }
                    Err(ChatError::CompactionNotUseful)
                        if recovery_state == recovery::RecoveryState::ContextChangeRequired =>
                    {
                        console_println!(
                            "Compaction could not find a smaller admissible Context. The current configuration cannot resume normal sending; retain the Session or explicitly choose a suitable larger-window configuration before --resume."
                        )
                    }
                    Err(error) if compaction::explicitly_deferred(&error) => console_println!(
                        "Compaction deferred: {error}; original Context and Provider delivery retained."
                    ),
                    Err(error) => return Err(error),
                }
                continue;
            }
            if line == "/image" {
                console_println!("Usage: /image <path>");
                continue;
            }
            if let Some(path) = line.strip_prefix("/image ") {
                match vision::ingest_image(&session, &tools, composition, path).await {
                    Ok(image) => {
                        match image.target_snapshot_id() {
                            Some(target) => console_println!(
                                "Queued image `{}` for TargetSnapshot `{}` and the next Turn.",
                                image.delivery_artifact_id(),
                                target
                            ),
                            None => console_println!(
                                "Queued image `{}` for Session `{}` and the next Turn.",
                                image.delivery_artifact_id(),
                                session_id.as_str()
                            ),
                        }
                        pending_images.push(image);
                    }
                    Err(error) => console_println!("Image rejected: {error}"),
                }
                continue;
            }
            if let Some(value) = line.strip_prefix("/phase ") {
                if !research_enabled {
                    console_println!("This Agent has no ResearchPhase.");
                    continue;
                }
                let phase = parse_phase(value)?;
                pending_phase = Some(ResearchPhaseSelection::operator(phase));
                console_println!("Queued phase `{}` for the next Turn.", phase.id());
                continue;
            }
            if let Some(value) = line.strip_prefix("/domain ") {
                if !research_enabled {
                    console_println!("This Agent has no Research DomainProfile.");
                    continue;
                }
                let selected_domain = parse_domain(value)?;
                pending_domain = Some(DomainProfileSelection::operator(selected_domain));
                console_println!(
                    "Queued domain `{}` for the next Turn.",
                    selected_domain.id()
                );
                continue;
            }

            // Show local preparation immediately; a follow-up is only accepted
            // after its current sources and durable Session have been checked.
            lines.prompt(crate::terminal_console::InputMode::Processing)?;
            let timing = timing::TurnTiming::new(composition.diagnostic_output_enabled());
            if research_enabled {
                let (domain, phase) = session.prepare_research_for_turn(
                    disposition,
                    pending_domain.take(),
                    pending_phase.take(),
                )?;
                agent.queue_domain_profile(domain);
                agent.queue_research_phase(phase);
            }
            timing.checkpoint("research_selection");

            // Prompt/core filesystem failures occur before a possible scoped
            // Vision request as well as before the Primary Provider request.
            let current = session.refresh_context(&mut context_cache, &tools)?;
            agent.refresh_current_context(current)?;
            timing.checkpoint("context_refresh");
            lines.processing(follow_up)?;
            let cancellation = AgentCancellation::new();
            let request_budget = Arc::new(ModelRequestBudget::new(
                composition.runtime().model_request_limit_per_turn,
            ));
            let initial_images = if pending_images.is_empty() {
                Vec::new()
            } else {
                let images = std::mem::take(&mut pending_images);
                let preparation = vision::prepare_primary_contexts(
                    &session,
                    composition,
                    images,
                    &line,
                    cancellation.clone(),
                    Arc::clone(&request_budget),
                );
                let prepared = input::interruptible(
                    preparation,
                    &cancellation,
                    lines,
                    terminal_input,
                    &mut input_closed,
                    &mut pending_input,
                )
                .await?;
                if cancellation.is_cancelled() {
                    match prepared {
                        Ok(_) | Err(ChatError::VisionCancelled) => {
                            console_println!(
                                "\n[vision request cancelled; no Primary request was started]"
                            );
                            continue;
                        }
                        Err(error) => {
                            input::reject_pending(&mut pending_input);
                            return Err(error);
                        }
                    }
                }
                match prepared {
                    Ok(prepared) => prepared,
                    Err(error) => {
                        let error = vision::follow_up_error(error);
                        if agent.permits_explicit_recovery(&error) {
                            session.validate_stopped_runtime(&agent)?;
                            recovery_state = recovery::RecoveryState::ResumeRequired {
                                capacity: recovery::is_capacity_error(&error),
                            };
                            recovery::report_stop(&error, &request_budget);
                            input::reject_pending(&mut pending_input);
                            console_println!(
                                "Initial images/task were not submitted to Primary; reattach each image explicitly after /resume if needed."
                            );
                            continue;
                        }
                        return Err(ChatError::Agent(error));
                    }
                }
            };
            timing.checkpoint("initial_sources_ready");
            let signal_cancellation = cancellation.clone();
            let answer_started = Cell::new(false);
            let end_answer = || {
                if answer_started.replace(false) {
                    console_println!();
                }
            };
            let mut text_delta = |text: &str| {
                timing.observe_text(text);
                if !answer_started.get() && !text.is_empty() {
                    console_print!("{}", theme.answer_prefix());
                    answer_started.set(true);
                }
                write!(crate::terminal_console::stdout(), "{text}")
                    .and_then(|_| crate::terminal_console::stdout().flush())
                    .map_err(|error| AgentLoopError::TextDeltaOutput {
                        message: error.to_string(),
                    })
            };
            let accepted_attempt = std::cell::RefCell::new(None);
            let mut record_attempt = |attempt: RequestAuditAttempt| {
                // Sampling boundaries affect only presentation, never History bytes.
                end_answer();
                let step_id = attempt.sampling_step_id().map(str::to_owned);
                session
                    .record_request_audit_manifest(attempt, None)
                    .map(|manifest| {
                        accepted_attempt.replace(Some((step_id, manifest)));
                        timing.checkpoint("primary_request_ready");
                    })
                    .map_err(ChatError::into_request_audit_agent_loop_error)
            };
            let mut tool_start = |plan: &crate::domain::tools::ToolExecutionPlan| {
                end_answer();
                tool_output.borrow_mut().started(plan);
            };
            let mut durability = |checkpoint: ChatDurabilityCheckpoint<'_>| match checkpoint {
                ChatDurabilityCheckpoint::Usage {
                    provider_session,
                    usage,
                } => {
                    end_answer();
                    let audit = accepted_attempt.borrow();
                    let (_, audit) = audit
                        .as_ref()
                        .expect("a completed audited sample has an actual attempt");
                    session
                        .record_model_usage(audit.audit_event_id(), provider_session, usage)
                        .map_err(|error| AgentLoopError::ModelUsageCommit {
                            message: error.to_string(),
                        })
                }
                ChatDurabilityCheckpoint::ModelSampleRejected {
                    diagnostic,
                    corrections_used,
                    sampling_step_id,
                } => {
                    end_answer();
                    session
                        .record_model_rejection(*diagnostic, corrections_used, sampling_step_id)
                        .map_err(|error| AgentLoopError::ModelRejectionCommit {
                            message: error.to_string(),
                        })?;
                    let limit = crate::domain::model::MAX_TOOL_ARGUMENT_CORRECTIONS_PER_CHAIN;
                    console_eprintln!(
                        "\n[model sample rejected] {diagnostic}; streamed prose was not committed and no Tool from this sample ran."
                    );
                    if corrections_used < limit {
                        console_eprintln!(
                            "[argument correction {}/{limit}] Regenerating the failed sample if budget permits; completed tools will not be replayed.",
                            corrections_used + 1
                        );
                    } else {
                        console_eprintln!(
                            "[argument corrections exhausted] Returning control without another request."
                        );
                    }
                    Ok(())
                }
                ChatDurabilityCheckpoint::ArgumentCorrectionReset => session
                    .record_correction_reset()
                    .map_err(|error| AgentLoopError::ModelRejectionCommit {
                        message: error.to_string(),
                    }),
                ChatDurabilityCheckpoint::Subcommand(checkpoint) => {
                    session
                        .record_subcommand(checkpoint)
                        .map_err(ChatError::into_execution_receipt_agent_loop_error)?;
                    tool_output.borrow_mut().checkpointed(checkpoint);
                    Ok(())
                }
                ChatDurabilityCheckpoint::History(items) => session
                    .record_history(items)
                    .map_err(ChatError::into_history_checkpoint_agent_loop_error),
                ChatDurabilityCheckpoint::OperatorInputAccepted { items, provenance } => session
                    .record_operator_input(items, provenance, &input_time)
                    .map_err(ChatError::into_history_checkpoint_agent_loop_error),
                ChatDurabilityCheckpoint::OperatorTurnCompleted => {
                    let audit = accepted_attempt.borrow();
                    let (_, audit) = audit
                        .as_ref()
                        .expect("a completed sample has an actual audit");
                    session
                        .record_operator_completed(audit.audit_event_id())
                        .map_err(ChatError::into_history_checkpoint_agent_loop_error)
                }
                ChatDurabilityCheckpoint::ModelDeliveryCommitted {
                    step_id,
                    previous_step_id,
                    provider_session,
                    server_output_items,
                } => {
                    use crate::domain::request_audit::RequestAuditDeliveryMode;
                    let audit = accepted_attempt.borrow();
                    let (audited_step, manifest) = audit
                        .as_ref()
                        .expect("a completed sample has an actual audit");
                    if audited_step.as_deref() != Some(step_id)
                        || manifest.provider_session() != Some(provider_session)
                    {
                        return Err(AgentLoopError::HistoryCheckpointCommit { message: "accepted sample does not match its actual audited Step and ProviderSession".to_owned() });
                    }
                    let previous = match manifest.delivery().mode() {
                        RequestAuditDeliveryMode::Full => None,
                        RequestAuditDeliveryMode::Incremental => {
                            let committed = committed_step.borrow();
                            let (step, id, session) = committed.as_ref().ok_or_else(|| {
                                AgentLoopError::HistoryCheckpointCommit {
                                    message: "incremental sample has no durable accepted baseline"
                                        .to_owned(),
                                }
                            })?;
                            if Some(step.as_str()) != previous_step_id
                                || session != provider_session
                            {
                                return Err(AgentLoopError::HistoryCheckpointCommit { message: "incremental sample does not match its actual committed baseline".to_owned() });
                            }
                            Some(id.clone())
                        }
                    };
                    session
                        .record_model_delivery(
                            manifest.audit_event_id(),
                            previous,
                            step_id,
                            server_output_items,
                        )
                        .map_err(ChatError::into_history_checkpoint_agent_loop_error)?;
                    committed_step.replace(Some((
                        step_id.to_owned(),
                        manifest.audit_event_id().clone(),
                        provider_session.clone(),
                    )));
                    Ok(())
                }
                ChatDurabilityCheckpoint::ExecutionReceipt(outcome) => {
                    let disposition = session
                        .record_execution(outcome)
                        .map_err(ChatError::into_execution_receipt_agent_loop_error)?;
                    tool_output.borrow_mut().recorded(
                        outcome,
                        matches!(disposition, ReceiptRecordDisposition::Existing),
                    );
                    Ok(())
                }
            };
            let follow_up_session = &session;
            let follow_up_composition = composition;
            let follow_up_task = line.clone();
            let follow_up_cancellation = cancellation.clone();
            let follow_up_budget = Arc::clone(&request_budget);
            let mut follow_up = move |selection| -> ViewImageFollowUpFuture<'_> {
                let task = follow_up_task.clone();
                let cancellation = follow_up_cancellation.clone();
                let request_budget = Arc::clone(&follow_up_budget);
                Box::pin(async move {
                    vision::prepare_primary_context(
                        follow_up_session,
                        follow_up_composition,
                        vision::PendingImage::from_selection(selection),
                        &task,
                        cancellation,
                        request_budget,
                    )
                    .await
                    .map_err(vision::follow_up_error)
                })
            };
            let mut refresh_context = || {
                session
                    .refresh_context(&mut context_cache, &tools)
                    .map_err(|error| AgentLoopError::ContextRefresh {
                        message: error.to_string(),
                    })
            };
            let improvement_task = line.clone();
            let improvement_history_start = improvement::history_start(&session)?;
            let mut tool_batch = compaction::AutomaticToolBatchCompaction {
                session: &session,
                window_tokens: composition
                    .agent_profile()
                    .primary()
                    .capabilities()
                    .context_window,
                request: compaction::CompactionRequestContext {
                    composition,
                    budget: &request_budget,
                    cancellation: signal_cancellation.clone(),
                    audit: &audit_context,
                },
                deferred: false,
                end_answer: &end_answer,
                composition,
                tools: &tools,
                discovered_mcp: &discovered_mcp,
                task: &improvement_task,
                history_start: improvement_history_start,
                no_pending_images: pending_images.is_empty(),
                binding: session.binding.clone(),
                batches: 0,
            };
            let outcome = {
                let mut task_context = task_context::TaskContextRuntime::new(&session)
                    .with_diagnostics(composition.diagnostic_output_enabled())
                    .with_selection_provider(composition.task_selection_provider());
                let turn = agent.run_turn_audited_with_context_refresh_and_vision_stages(
                    line,
                    initial_images,
                    cancellation,
                    &audit_context,
                    &mut record_attempt,
                    CliRuntimeStages::new(
                        &mut follow_up,
                        &mut text_delta as &mut AssistantTextDeltaStage<'_>,
                        &mut tool_start as &mut ToolExecutionStartStage<'_>,
                        &mut durability as &mut ChatDurabilityStage<'_>,
                        &mut refresh_context,
                        Arc::clone(&request_budget),
                    )
                    .with_task_context(&mut task_context),
                    (composition.runtime().auto_compact
                        || crate::app::improvement::policy(session.binding.definition()).is_some())
                    .then_some(&mut tool_batch),
                );
                input::interruptible(
                    turn,
                    &signal_cancellation,
                    lines,
                    terminal_input,
                    &mut input_closed,
                    &mut pending_input,
                )
                .await?
            };

            let tool_batch_compaction_deferred = tool_batch.deferred;
            let effective_binding = tool_batch.binding.clone();
            drop(tool_batch);
            session.set_binding(effective_binding)?;
            timing.checkpoint("primary_turn_settled");
            tool_output.borrow_mut().finish_turn();
            let mut auto_compacted = false;
            match outcome {
                Ok(AgentTurnOutcome::Completed { .. }) => {
                    console_println!();
                    let protected = session.with_projection(|projection, _, _| {
                        Ok(projection.state().current_protected_operator_context()?)
                    })?;
                    agent.set_protected_context(protected);
                    input::interruptible(
                        improvement::after_completed(
                            &session,
                            &agent,
                            &tools,
                            &improvement_task,
                            improvement_history_start,
                            pending_images.is_empty(),
                            &request_budget,
                            signal_cancellation.clone(),
                            &audit_context,
                            composition,
                            &discovered_mcp,
                        ),
                        &signal_cancellation,
                        lines,
                        terminal_input,
                        &mut input_closed,
                        &mut pending_input,
                    )
                    .await??;
                    if evolution::apply_completed(
                        &mut session,
                        &mut agent,
                        &tools,
                        composition,
                        &discovered_mcp,
                        pending_images.is_empty(),
                    )? {
                        disposition = SessionDisposition::Resumed;
                        context_cache = context::ChatContextCache::default();
                    }
                    if composition.runtime().auto_compact && !signal_cancellation.is_cancelled() {
                        let current = session.refresh_context(&mut context_cache, &tools)?;
                        agent.refresh_current_context(current)?;
                        let should_compact = agent.delivery_needs_compaction()?
                            || match agent.fresh_compaction_context_probe()? {
                                Some(plan) => {
                                    compaction::ContextPressure::from_plan(&plan).should_remind()
                                }
                                None => true,
                            };
                        if should_compact {
                            if tool_batch_compaction_deferred
                                || request_budget.remaining() == Some(0)
                            {
                                console_println!(
                                    "Automatic completed-Turn compaction was not sampled; the completed answer remains durable. Use /compact to attempt a new handoff."
                                );
                            } else {
                                let compact = session.compact(
                                    &mut agent,
                                    composition
                                        .agent_profile()
                                        .primary()
                                        .capabilities()
                                        .context_window,
                                    compaction::CompactionRequestContext {
                                        composition,
                                        budget: &request_budget,
                                        cancellation: signal_cancellation.clone(),
                                        audit: &audit_context,
                                    },
                                );
                                match input::interruptible(
                                    compact,
                                    &signal_cancellation,
                                    lines,
                                    terminal_input,
                                    &mut input_closed,
                                    &mut pending_input,
                                )
                                .await?
                                {
                                    Ok(summary) => {
                                        console_println!("{}", summary.automatic_message());
                                        auto_compacted = true;
                                    }
                                    Err(ChatError::CompactionFacts(
                                        CompactionValidationFactsError::ResearchStreamEmpty,
                                    )) => {
                                        console_println!(
                                            "Automatic completed-Turn compaction deferred: the Project Research stream is empty; no Compaction was appended and Provider delivery state was retained."
                                        );
                                    }
                                    Err(ChatError::Session(
                                        SessionError::CompactionCurrentTargetRequired,
                                    )) => {
                                        console_println!(
                                            "Automatic completed-Turn compaction deferred: the current Project has no TargetSnapshot; no Compaction was appended and Provider delivery state was retained."
                                        );
                                    }
                                    Err(ChatError::CompactionNotUseful) => {
                                        console_println!(
                                            "Automatic completed-Turn compaction deferred: no smaller usable next-Turn Context was found; no Compaction was appended and Provider delivery state was retained."
                                        );
                                    }
                                    Err(error) if compaction::automatically_deferred(&error) => {
                                        console_println!(
                                            "Automatic completed-Turn compaction deferred: {error}; original Context and Provider delivery retained."
                                        );
                                    }
                                    Err(error) => return Err(error),
                                }
                            }
                        }
                    }
                }
                Ok(AgentTurnOutcome::InitialSelectionCancelled { anchor, .. }) => {
                    session.record_initial_selection_cancelled(anchor)?;
                    console_println!("\n[request cancelled]");
                    let protected = session.with_projection(|projection, _, _| {
                        Ok(projection.state().current_protected_operator_context()?)
                    })?;
                    agent.set_protected_context(protected);
                }
                Ok(AgentTurnOutcome::Cancelled { .. }) => {
                    console_println!("\n[request cancelled]");
                    let protected = session.with_projection(|projection, _, _| {
                        Ok(projection.state().current_protected_operator_context()?)
                    })?;
                    agent.set_protected_context(protected);
                }
                Err(error) => {
                    console_println!();
                    input::reject_pending(&mut pending_input);
                    if !agent.permits_explicit_recovery(&error) {
                        return Err(ChatError::Agent(error));
                    }
                    session.validate_stopped_runtime(&agent)?;
                    recovery_state = recovery::RecoveryState::ResumeRequired {
                        capacity: recovery::is_capacity_error(&error),
                    };
                    recovery::report_stop(&error, &request_budget);
                }
            }
            if recovery_state == recovery::RecoveryState::Ready {
                recovery::report_budget(&request_budget);
            }
            if !auto_compacted && let Some(plan) = agent.last_context_plan() {
                let pressure = compaction::ContextPressure::from_plan(plan);
                if pressure.should_remind() {
                    console_println!("{}", pressure.message());
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SessionDisposition {
    Started,
    Resumed,
}

impl SessionDisposition {
    const fn message(self) -> &'static str {
        match self {
            Self::Started => "Started",
            Self::Resumed => "Resumed",
        }
    }
}

/// The fixed Session persistence stage keeps one disposable projection and
/// verifies its exact source prefix under the Project writer lease before
/// reducing any suffix, including independently appended cache Receipts.
struct DurableChatSession {
    project: Project,
    session_id: StreamId,
    artifacts: LocalArtifactStore,
    binding: AgentDefinitionBinding,
    current_binding: Arc<std::sync::RwLock<AgentDefinitionBinding>>,
    projection: Arc<std::sync::Mutex<Option<persistence::CachedSessionProjection>>>,
    receipt_cache:
        std::sync::Mutex<crate::adapters::research_projection::SessionReceiptCatalogCache>,
    context_source_index: Arc<std::sync::Mutex<Option<Arc<context_read::index::ColdIndex>>>>,
    task_index: std::sync::Mutex<Option<task_index::TaskIndex>>,
}

impl DurableChatSession {
    fn publish_binding(&self, binding: &AgentDefinitionBinding) -> Result<(), ChatError> {
        *self
            .current_binding
            .write()
            .map_err(|_| io::Error::other("Agent binding lock poisoned"))? = binding.clone();
        Ok(())
    }

    fn set_binding(&mut self, binding: AgentDefinitionBinding) -> Result<(), ChatError> {
        self.publish_binding(&binding)?;
        self.binding = binding;
        Ok(())
    }

    fn new(
        project: Project,
        session_id: StreamId,
        binding: AgentDefinitionBinding,
    ) -> Result<Self, ChatError> {
        let artifacts = LocalArtifactStore::for_project(&project);
        Ok(Self {
            project,
            session_id,
            artifacts,
            current_binding: Arc::new(std::sync::RwLock::new(binding.clone())),
            binding,
            projection: Arc::new(std::sync::Mutex::new(None)),
            receipt_cache: std::sync::Mutex::new(
                crate::adapters::research_projection::SessionReceiptCatalogCache::default(),
            ),
            context_source_index: Arc::new(std::sync::Mutex::new(None)),
            task_index: std::sync::Mutex::new(None),
        })
    }

    fn start_or_resume(
        &self,
        explicit_resume: bool,
        composition: &ProviderComposition,
        replace_definition: bool,
        discovered_mcp: &[crate::adapters::mcp::McpCatalogTool],
    ) -> Result<
        (
            SessionState,
            ProviderSession,
            SessionDisposition,
            Option<SessionCompactionFacts>,
            bool,
        ),
        ChatError,
    > {
        if explicit_resume
            && !self
                .project
                .existing_session_ids()?
                .iter()
                .any(|id| id == &self.session_id)
        {
            return Err(ChatError::SessionNotFound {
                session_id: self.session_id.as_str().to_owned(),
            });
        }

        if !explicit_resume {
            self.preflight_binding(&SessionState::default(), composition, discovered_mcp)?;
        }

        self.with_projection(|projection, store, stream_id| {
        let read = if explicit_resume { Some(self.project_read(projection.state())?) } else { None };
        if explicit_resume {
            projection.state().ensure_no_unknown_improvement()?;
        }
        let sequence = store.next_sequence()?;
        let provider_session = ProviderSession::new(
            ProviderSessionId::new(format!(
                "primary:{}:{}",
                self.session_id.as_str(),
                sequence.get()
            )),
            ProviderSessionRole::Primary,
            sequence.get(),
        );
        let mut compaction_validation = None;
        let mut capacity_required = false;
        let (event, disposition) = if explicit_resume {
            if projection.state().lifecycle() != SessionLifecycle::Active {
                return Err(ChatError::SessionNotActive {
                    session_id: self.session_id.as_str().to_owned(),
                });
            }
            if let Some(call_id) = unresolved_receipted_tool_call(projection.state()) {
                return Err(ChatError::UnresolvedExecutionReceipt {
                    call_id: call_id.as_str().to_owned(),
                });
            }
            projection
                .state()
                .compaction_source_history()?;
            self.validate_protected_sources(projection.state())?;
            if let Some(compaction) = projection
                .state()
                .latest_compaction()
                .filter(|compaction| compaction.summary().requires_reference_validation())
            {
                let read = read.as_ref().expect("Resume captured its read context");
                let mut facts = load_resume_compaction_facts(&self.project, compaction.summary(), read, projection.compaction_source_for(compaction.summary()))
                    .map_err(ChatError::ResearchProjection)?;
                match compaction.validate_references(&facts.context()) {
                    Ok(()) => {},
                    Err(crate::domain::session::SessionError::CompactionEvidenceStaleResearchRevision { .. }) => {
                        let historical = load_historical_session_compaction_facts(&self.project, compaction.summary(), read, projection.compaction_source_for(compaction.summary()))
                            .map_err(ChatError::ResearchProjection)?;
                        compaction.validate_references(&historical.context())?;
                        facts = crate::adapters::session_compaction::load_session_compaction_facts_from_read(&self.project, projection.state(), projection.current_source(), read)
                            .map_err(ChatError::CompactionFacts)?;
                        if let Some(graph) = historical.verified_graph() {
                            facts = facts.with_verified_graph(graph.clone())?;
                        }
                        let rebuilt = self.rebuild_compaction(
                            projection.state(),
                            &facts,
                            composition,
                        )?;
                        let recovery_sequence = store.next_sequence()?;
                        let recovery = SessionEvent::CompactionCreated { compaction: rebuilt }.into_envelope(
                            event_id(self.project.project_id(), &self.session_id, recovery_sequence.get(), "compaction-rebuild"),
                            stream_id.clone(), recovery_sequence, occurred_at(),
                        )?;
                        projection.append(store, recovery).map_err(ChatError::from_session_append)?;
                        if let Some(source) = facts.task_projection() { projection.retain_compaction_source(source.clone())?; }
                    },
                    Err(error) => return Err(error.into()),
                }
                compaction_validation = Some(facts);
            }
            match self.preflight_binding_from_read(projection.state(), composition, discovered_mcp, read.as_ref().expect("Resume captured its read context")) {
                Ok(()) => {},
                Err(ChatError::Agent(ref error)) if recovery::is_capacity_error(error) => capacity_required = true,
                Err(error) => return Err(error),
            }
            (
                if replace_definition || projection.state().agent_definition_binding().is_none() {
                    SessionEvent::AgentDefinitionApplied {
                        primary_provider_session: provider_session.reference().clone(),
                        binding: self.binding.clone(),
                    }
                } else {
                    SessionEvent::Resumed {
                        primary_provider_session: provider_session.reference().clone(),
                    }
                },
                SessionDisposition::Resumed,
            )
        } else {
            if projection.state().lifecycle() != SessionLifecycle::Unstarted {
                return Err(ChatError::SessionAlreadyExists {
                    session_id: self.session_id.as_str().to_owned(),
                });
            }
            (
                SessionEvent::StartedWithAgent {
                    primary_provider_session: provider_session.reference().clone(),
                    binding: self.binding.clone(),
                },
                SessionDisposition::Started,
            )
        };
        let sequence = store.next_sequence()?;
        let event = event.into_envelope(
            event_id(
                self.project.project_id(),
                &self.session_id,
                sequence.get(),
                disposition.message(),
            ),
            stream_id,
            sequence,
            occurred_at(),
        )?;
        projection
            .append(store, event)
            .map_err(ChatError::from_session_append)?;
        Ok((
            projection.state().clone(),
            provider_session,
            disposition,
            compaction_validation,
            capacity_required,
        ))        })
    }

    /// App owns research selection policy; this boundary supplies durable I/O.
    fn prepare_research_for_turn(
        &self,
        disposition: SessionDisposition,
        domain: Option<DomainProfileSelection>,
        phase: Option<ResearchPhaseSelection>,
    ) -> Result<(DomainProfile, ResearchPhase), ChatError> {
        let research = self
            .binding
            .definition()
            .research()
            .expect("research command admission");
        self.with_projection(|projection, store, stream_id| {
            projection
                .prepare_research_turn(
                    store,
                    stream_id,
                    crate::app::session::ResearchTurnRequest {
                        started: disposition == SessionDisposition::Started,
                        default_domain: research.domain(),
                        default_phase: research.phase(),
                        domain,
                        phase,
                    },
                    |sequence, kind| {
                        (
                            event_id(
                                self.project.project_id(),
                                &self.session_id,
                                sequence.get(),
                                kind,
                            ),
                            occurred_at(),
                        )
                    },
                )
                .map_err(ChatError::from_session_append)
        })
    }

    fn record_request_audit_manifest(
        &self,
        attempt: RequestAuditAttempt,
        stamp: Option<(EventId, OccurredAt)>,
    ) -> Result<RequestAuditManifest, ChatError> {
        self.with_projection(|projection, store, stream_id| {
            let sequence = store.next_sequence()?;
            let (event_id, occurred_at) = stamp.unwrap_or_else(|| {
                (
                    event_id(
                        self.project.project_id(),
                        &self.session_id,
                        sequence.get(),
                        "request-audit",
                    ),
                    occurred_at(),
                )
            });
            let service = RequestAuditService::new(
                &self.artifacts,
                RequestAuditBinding::new(self.project.project_id().clone(), stream_id)?,
            );
            service
                .record(attempt, event_id, occurred_at, projection, store)
                .map_err(|error| ChatError::Agent(error.into_agent_loop_error()))
        })
    }

    fn record_execution(
        &self,
        outcome: &ToolExecutionOutcome,
    ) -> Result<ReceiptRecordDisposition, ChatError> {
        self.with_projection(|projection, store, stream_id| {
            self.record_execution_receipt(projection, store, &stream_id, outcome)
        })
    }

    fn record_subcommand(
        &self,
        checkpoint: &crate::domain::tools::SubcommandCheckpoint,
    ) -> Result<(), ChatError> {
        self.with_projection(|projection, store, stream_id| {
            let sequence = store.next_sequence()?;
            let event = SessionEvent::SubcommandCheckpointed {
                checkpoint: checkpoint.clone(),
            }
            .into_envelope(
                event_id(
                    self.project.project_id(),
                    &self.session_id,
                    sequence.get(),
                    "subcommand-checkpoint",
                ),
                stream_id,
                sequence,
                occurred_at(),
            )?;
            projection
                .append(store, event)
                .map_err(ChatError::from_session_append)?;
            Ok(())
        })
    }

    /// Operator image intake has no model-originated History Call. Keep its
    /// original standalone Receipt rather than inventing a model pair.
    fn record_operator_execution_receipt(
        &self,
        outcome: &ToolExecutionOutcome,
    ) -> Result<ReceiptRecordDisposition, ChatError> {
        self.with_projection(|projection, store, stream_id| {
            if projection
                .state()
                .execution_receipts()
                .iter()
                .any(|record| record.receipt == outcome.receipt)
            {
                return Ok(ReceiptRecordDisposition::Existing);
            }
            let sequence = store.next_sequence()?;
            let event = if outcome.receipt.cache_source.is_some() {
                SessionEvent::ToolResultCacheHit {
                    receipt: outcome.receipt.clone(),
                }
            } else {
                SessionEvent::ExecutionReceiptRecorded {
                    receipt: outcome.receipt.clone(),
                }
            }
            .into_envelope(
                event_id(
                    self.project.project_id(),
                    &self.session_id,
                    sequence.get(),
                    "execution-receipt",
                ),
                stream_id,
                sequence,
                occurred_at(),
            )?;
            projection
                .append(store, event)
                .map_err(ChatError::from_session_append)?;
            Ok(ReceiptRecordDisposition::Recorded)
        })
    }

    fn record_execution_receipt(
        &self,
        projection: &mut SessionProjection,
        store: &mut crate::adapters::project_event_store::JsonlEventStore<'_>,
        stream_id: &StreamId,
        outcome: &ToolExecutionOutcome,
    ) -> Result<ReceiptRecordDisposition, ChatError> {
        if projection
            .state()
            .execution_receipts()
            .iter()
            .any(|record| {
                record.receipt == outcome.receipt
                    && (record.result.as_ref() == Some(&outcome.result)
                        // The existing deduplicated_for contract reuses its
                        // leader's Receipt with a separately paired follower.
                        || outcome.receipt.execution_id
                            != crate::domain::tools::ExecutionId::for_model_call(&outcome.result.call_id))
            })
        {
            return Ok(ReceiptRecordDisposition::Existing);
        }
        let sequence = store.next_sequence()?;
        let receipt_event_id = event_id(
            self.project.project_id(),
            &self.session_id,
            sequence.get(),
            "execution-receipt",
        );
        let event = SessionEvent::ExecutionOutcomeRecorded {
            receipt: outcome.receipt.clone(),
            result: outcome.result.clone(),
        }
        .into_envelope(receipt_event_id, stream_id.clone(), sequence, occurred_at())?;
        projection
            .append(store, event)
            .map_err(ChatError::from_session_append)?;
        Ok(ReceiptRecordDisposition::Recorded)
    }

    fn record_model_rejection(
        &self,
        diagnostic: crate::domain::model::ToolArgumentDiagnostic,
        corrections_used: u8,
        sampling_step_id: &str,
    ) -> Result<(), ChatError> {
        self.with_projection(|projection, store, stream_id| {
            let request_audit_event_id = projection
                .state()
                .request_audits()
                .last()
                .ok_or(crate::domain::session::SessionError::InvalidModelSampleRejection)?
                .audit_event_id()
                .clone();
            let sequence = store.next_sequence()?;
            let event = SessionEvent::ConsecutiveModelSampleRejectedV3 {
                request_audit_event_id,
                diagnostic,
                corrections_used,
                reset_anchor: projection.state().correction_reset_anchor().cloned(),
                sampling_step_id: sampling_step_id.to_owned(),
            }
            .into_envelope(
                event_id(
                    self.project.project_id(),
                    &self.session_id,
                    sequence.get(),
                    "model-rejected",
                ),
                stream_id,
                sequence,
                occurred_at(),
            )?;
            projection
                .append(store, event)
                .map_err(ChatError::from_session_append)?;
            Ok(())
        })
    }

    fn record_correction_reset(&self) -> Result<(), ChatError> {
        self.with_projection(|projection, store, stream_id| {
            let state = projection.state();
            let (prior_rejection_audit_event_id, accepted_history_event_refs) =
                state.correction_reset_source()?;
            let accepted_request_audit_event_id = state
                .request_audits()
                .last()
                .ok_or(SessionError::InvalidModelSampleRejection)?
                .audit_event_id()
                .clone();
            let sequence = store.next_sequence()?;
            let event = SessionEvent::ModelArgumentCorrectionReset {
                accepted_request_audit_event_id,
                prior_rejection_audit_event_id,
                accepted_history_event_refs,
            }
            .into_envelope(
                event_id(
                    self.project.project_id(),
                    &self.session_id,
                    sequence.get(),
                    "argument-correction-reset",
                ),
                stream_id,
                sequence,
                occurred_at(),
            )?;
            projection
                .append(store, event)
                .map_err(ChatError::from_session_append)?;
            Ok(())
        })
    }

    fn record_history(&self, items: &[crate::domain::model::ContentItem]) -> Result<(), ChatError> {
        if items.is_empty() {
            return Ok(());
        }
        self.with_projection(|projection, store, stream_id| {
            self.append_history_items(projection, store, &stream_id, items)
        })
    }

    fn record_operator_input(
        &self,
        items: &[crate::domain::model::ContentItem],
        provenance: &mut std::collections::BTreeMap<
            usize,
            crate::domain::context::ContextProvenance,
        >,
        received_at: &crate::domain::session::TaskInputTime,
    ) -> Result<(), ChatError> {
        self.with_projection(|projection, store, stream_id| {
            let boundary = projection.state().accepted_operator_input(
                items,
                provenance,
                received_at.clone(),
            )?;
            let sequence = store.next_sequence()?;
            let input_provenance = boundary.input_provenance()?;
            let event = SessionEvent::OperatorTurnBoundaryRecorded { boundary }.into_envelope(
                event_id(
                    self.project.project_id(),
                    &self.session_id,
                    sequence.get(),
                    "operator-input-accepted",
                ),
                stream_id,
                sequence,
                occurred_at(),
            )?;
            projection
                .append(store, event)
                .map_err(ChatError::from_session_append)?;
            *provenance = input_provenance;
            Ok(())
        })
    }

    fn record_initial_selection_cancelled(&self, anchor: EventId) -> Result<(), ChatError> {
        self.with_projection(|projection, store, stream_id| {
            let boundary =
                crate::domain::session::OperatorTurnBoundary::InitialSelectionCancelled {
                    schema_version: 1,
                    anchor,
                };
            let sequence = store.next_sequence()?;
            let event = SessionEvent::OperatorTurnBoundaryRecorded { boundary }.into_envelope(
                event_id(
                    self.project.project_id(),
                    &self.session_id,
                    sequence.get(),
                    "initial-selection-cancelled",
                ),
                stream_id,
                sequence,
                occurred_at(),
            )?;
            projection
                .append(store, event)
                .map_err(ChatError::from_session_append)?;
            Ok(())
        })
    }

    fn validate_protected_sources(&self, state: &SessionState) -> Result<(), ChatError> {
        let mut artifacts = std::collections::BTreeSet::new();
        for input in state.protected_operator_inputs()? {
            artifacts.extend(
                input
                    .provenance
                    .references()
                    .artifact_refs()
                    .iter()
                    .cloned(),
            );
        }
        artifacts.extend(state.retained_followup_artifacts()?);
        for id in artifacts {
            crate::adapters::image::resolve_project_image_artifact(&self.project, &id)
                .map_err(ChatError::ProtectedImage)?;
        }
        crate::app::session::verify_protected_vision_sources(state, &self.artifacts)?;
        Ok(())
    }

    fn record_operator_completed(&self, audit_id: &EventId) -> Result<(), ChatError> {
        self.with_projection(|projection, store, stream_id| {
            let audit = projection
                .state()
                .request_audits()
                .last()
                .filter(|audit| audit.audit_event_id() == audit_id)
                .ok_or(ChatError::RuntimeHistoryMismatch)?;
            let service = RequestAuditService::new(
                &self.artifacts,
                RequestAuditBinding::new(self.project.project_id().clone(), stream_id.clone())?,
            );
            let verified = service.reconstruct_full_logical(audit).map_err(|error| {
                ChatError::CompactionSummarySource {
                    message: format!("completed audit reconstruction failed: {error:?}"),
                }
            })?;
            let Some(verified) = verified else {
                return Ok(());
            };
            let Some(chain) = projection.state().model_delivery_chain(audit_id)? else {
                return Ok(());
            };
            let mut delivered = std::collections::BTreeMap::new();
            for accepted in chain {
                let manifest = projection
                    .state()
                    .request_audits()
                    .iter()
                    .find(|audit| audit.audit_event_id() == accepted.audit_event_id())
                    .ok_or(ChatError::RuntimeHistoryMismatch)?;
                let input = service.reconstruct(manifest).map_err(|error| {
                    ChatError::CompactionSummarySource {
                        message: format!("accepted delivery reconstruction failed: {error:?}"),
                    }
                })?;
                let map = manifest.context_delivery_map().ok_or_else(|| {
                    ChatError::CompactionSummarySource {
                        message: "accepted delivery has no provenance map".to_owned(),
                    }
                })?;
                map.validate_against(&input).map_err(|error| {
                    ChatError::CompactionSummarySource {
                        message: format!("accepted delivery provenance mismatch: {error}"),
                    }
                })?;
                delivered.insert(accepted.audit_event_id().clone(), input);
            }
            let Some(boundary) = projection
                .state()
                .completed_operator_turn(&verified, &delivered)?
            else {
                return Ok(());
            };
            let sequence = store.next_sequence()?;
            let event = SessionEvent::OperatorTurnBoundaryRecorded { boundary }.into_envelope(
                event_id(
                    self.project.project_id(),
                    &self.session_id,
                    sequence.get(),
                    "operator-turn-completed",
                ),
                stream_id,
                sequence,
                occurred_at(),
            )?;
            projection
                .append(store, event)
                .map_err(ChatError::from_session_append)?;
            Ok(())
        })
    }

    fn record_model_delivery(
        &self,
        audit_id: &EventId,
        previous: Option<EventId>,
        step_id: &str,
        server_output_items: &[crate::domain::model::ContentItem],
    ) -> Result<(), ChatError> {
        self.with_projection(|projection, store, stream_id| {
            let proof_v4 = projection
                .state()
                .latest_task_context_selection()
                .is_some_and(|(_, selection)| selection.request_proof_version >= 4);
            let delivery = if proof_v4 {
                projection
                    .state()
                    .verify_model_delivery_output(server_output_items)?;
                use crate::ports::artifact::ArtifactStore;
                let bytes = serde_json::to_vec(server_output_items).map_err(|error| {
                    ChatError::CompactionSummarySource {
                        message: format!("accepted output encoding failed: {error}"),
                    }
                })?;
                let blob = self
                    .artifacts
                    .publish_bytes(&bytes)
                    .map_err(|error| ChatError::CompactionSummarySource {
                        message: format!("accepted output publication failed: {error}"),
                    })?
                    .blob_id;
                projection
                    .state()
                    .new_model_delivery_with_output(audit_id, previous, blob, step_id)?
            } else {
                projection.state().new_model_delivery(audit_id, previous)?
            };
            let sequence = store.next_sequence()?;
            let event = SessionEvent::ModelDeliveryCommitted { delivery }.into_envelope(
                event_id(
                    self.project.project_id(),
                    &self.session_id,
                    sequence.get(),
                    "model-delivery-committed",
                ),
                stream_id,
                sequence,
                occurred_at(),
            )?;
            projection
                .append(store, event)
                .map_err(ChatError::from_session_append)?;
            Ok(())
        })
    }

    fn append_history_items(
        &self,
        projection: &mut SessionProjection,
        store: &mut crate::adapters::project_event_store::JsonlEventStore<'_>,
        stream_id: &StreamId,
        items: &[crate::domain::model::ContentItem],
    ) -> Result<(), ChatError> {
        for item in items {
            let sequence = store.next_sequence()?;
            let event = SessionEvent::HistoryItemRecorded { item: item.clone() }.into_envelope(
                event_id(
                    self.project.project_id(),
                    &self.session_id,
                    sequence.get(),
                    "history",
                ),
                stream_id.clone(),
                sequence,
                occurred_at(),
            )?;
            projection
                .append(store, event)
                .map_err(ChatError::from_session_append)?;
        }
        Ok(())
    }
}

enum ReceiptRecordDisposition {
    Recorded,
    Existing,
}

fn parse_phase(value: &str) -> Result<ResearchPhase, ChatError> {
    match value.trim() {
        "mapping" => Ok(ResearchPhase::Mapping),
        "hypothesis" => Ok(ResearchPhase::Hypothesis),
        "experiment" => Ok(ResearchPhase::Experiment),
        "verification" => Ok(ResearchPhase::Verification),
        "evidence_curation" | "evidence-curation" => Ok(ResearchPhase::EvidenceCuration),
        value => Err(ChatError::UnknownPhase {
            value: value.to_owned(),
        }),
    }
}

fn parse_domain(value: &str) -> Result<DomainProfile, ChatError> {
    match value.trim() {
        "source" => Ok(DomainProfile::Source),
        "web" => Ok(DomainProfile::Web),
        "mobile" => Ok(DomainProfile::Mobile),
        "binary" => Ok(DomainProfile::Binary),
        value => Err(ChatError::UnknownDomain {
            value: value.to_owned(),
        }),
    }
}

fn new_session_id() -> StreamId {
    let sequence = NEXT_CHAT_SESSION.fetch_add(1, Ordering::Relaxed);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    StreamId::new(format!(
        "chat-{}-{}-{}-{sequence}",
        timestamp.as_secs(),
        timestamp.subsec_nanos(),
        std::process::id(),
    ))
}

fn occurred_at() -> OccurredAt {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    OccurredAt::new(format!(
        "unix:{}.{}",
        timestamp.as_secs(),
        timestamp.subsec_nanos()
    ))
}

fn event_id(project_id: &ProjectId, session_id: &StreamId, sequence: u64, kind: &str) -> EventId {
    EventId::new(format!(
        "chat:{}:{}:{sequence}:{kind}",
        project_id.as_str(),
        session_id.as_str()
    ))
}

#[derive(Debug, Error)]
pub(crate) enum ChatError {
    #[error(transparent)]
    Evolution(#[from] crate::adapters::agent_evolution::EvolutionError),
    #[error("Agent definition: {message}")]
    Definition { message: String },
    #[error("Session has an unresolved stopped Turn; resume its Session explicitly")]
    UnresolvedTurnStop,
    #[error(transparent)]
    ContextRefresh(#[from] context::ChatContextRefreshError),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Project(#[from] crate::adapters::project_event_store::ProjectEventStoreError),
    #[error(transparent)]
    Session(#[from] crate::domain::session::SessionError),
    #[error(transparent)]
    AgentConfiguration(#[from] crate::app::agent_loop::AgentLoopConfigurationError),
    #[error(transparent)]
    Agent(#[from] AgentLoopError),
    #[error(transparent)]
    ToolExecution(#[from] crate::ports::tool::ToolExecutionError),
    #[error(transparent)]
    Mcp(#[from] crate::adapters::mcp::McpError),
    #[error(transparent)]
    RequestAudit(#[from] crate::domain::request_audit::RequestAuditError),
    #[error("rebuild Project Snapshot for Chat: {0}")]
    ProjectSnapshot(#[from] crate::adapters::project_snapshot::LocalProjectSnapshotError),
    #[error(transparent)]
    Research(#[from] crate::domain::research::ResearchError),
    #[error("Research projection is invalid: {0}")]
    ResearchProjection(String),
    #[error("Research projection is invalid: {0}")]
    CompactionFacts(CompactionValidationFactsError),
    #[error(transparent)]
    WorldState(#[from] crate::domain::world_state::WorldStateError),
    #[error("Session `{session_id}` does not exist in the current project")]
    SessionNotFound { session_id: String },
    #[error("Session `{session_id}` is not active and cannot be resumed")]
    SessionNotActive { session_id: String },
    #[error("Session `{session_id}` already exists; resume it explicitly with --session")]
    SessionAlreadyExists { session_id: String },
    #[error("unknown research phase `{value}`")]
    UnknownPhase { value: String },
    #[error("unknown vulnerability-research domain `{value}`")]
    UnknownDomain { value: String },
    #[error("/image requires a non-empty local path")]
    ImagePathRequired,
    #[error("/image requires at least one recorded TargetSnapshot in the current Project")]
    ImageTargetSnapshotRequired,
    #[error("local image intake failed: {message}")]
    ImageIntake { message: String },
    #[error("the selected Agent Model Profile cannot accept images: {reason}")]
    VisionUnavailable { reason: &'static str },
    #[error("CLI Vision composition failed: {message}")]
    VisionComposition { message: String },
    #[error("Vision analysis was cancelled before the Primary request")]
    VisionCancelled,
    #[error(
        "strict Session History changed independently from the active AgentLoop; recovery or Compaction was not applied"
    )]
    RuntimeHistoryMismatch,
    #[error(
        "Compaction did not yield a smaller usable next-Turn Context within the configured budget; no Compaction appended"
    )]
    CompactionNotUseful,
    #[error("Compaction source integrity failed: {message}")]
    CompactionSummarySource { message: String },
    #[error("protected image cannot be verified: {0}")]
    ProtectedImage(#[source] crate::adapters::image::LocalImageError),
    #[error(
        "Tool Call `{call_id}` has a durable ExecutionReceipt but no paired durable ToolResult; recovery refuses to invent an aborted result or replay the Tool"
    )]
    UnresolvedExecutionReceipt { call_id: String },
}

fn unresolved_receipted_tool_call(
    state: &SessionState,
) -> Option<crate::domain::tools::ToolCallId> {
    state
        .history()
        .unpaired_tool_calls_in_call_order()
        .into_iter()
        .find_map(|(call_id, _)| {
            state
                .execution_receipts()
                .iter()
                .any(|record| {
                    record.receipt.execution_id
                        == crate::domain::tools::ExecutionId::for_model_call(&call_id)
                })
                .then_some(call_id.clone())
                .filter(|call_id| state.recorded_tool_result(call_id).is_none())
        })
}

impl ChatError {
    fn from_session_append(
        error: AppendAfterDurableError<
            crate::adapters::project_event_store::ProjectEventStoreError,
        >,
    ) -> Self {
        match error {
            AppendAfterDurableError::Session(error) => Self::Session(error),
            AppendAfterDurableError::Sink(error) => Self::Project(error),
        }
    }

    fn into_request_audit_agent_loop_error(self) -> AgentLoopError {
        match self {
            Self::Agent(error) => error,
            error => AgentLoopError::RequestAuditCommit {
                message: error.to_string(),
            },
        }
    }

    fn into_history_checkpoint_agent_loop_error(self) -> AgentLoopError {
        match self {
            Self::Agent(error) => error,
            error => AgentLoopError::HistoryCheckpointCommit {
                message: error.to_string(),
            },
        }
    }

    fn into_execution_receipt_agent_loop_error(self) -> AgentLoopError {
        match self {
            Self::Agent(error) => error,
            error => AgentLoopError::ExecutionReceiptCommit {
                message: error.to_string(),
            },
        }
    }
}
