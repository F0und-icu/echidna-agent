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
mod context;
mod definition;
mod evolution;
mod improvement;
mod input;
mod memory;
mod persistence;
mod recovery;
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
    load_session_compaction_facts,
};
use crate::adapters::tools::ToolRegistry;
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
    let _runtime_lease = project.acquire_chat_runtime_lease()?;
    let discovered_mcp = composition.discover_chat_mcp().await?;
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
        if !research_enabled && (requested_domain.is_some() || requested_phase.is_some()) {
            return Err(definition::error(
                "--domain/--phase require a research-enabled Agent definition",
            ));
        }
        let mut session = DurableChatSession::new(project.clone(), session_id.clone(), binding)?;
        session.projection = std::sync::Mutex::new(projection);
        crate::adapters::agent_evolution::EvolutionStore::new(&session.project, &session.binding)
            .prepare(&session.binding)?;
        let (session_state, provider_session, mut disposition, compaction_validation) =
            session.start_or_resume(explicit_resume, composition, false, &discovered_mcp)?;
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

        let tools = session.bound_tools(
            &discovered_mcp,
            composition
                .agent_profile()
                .primary()
                .capabilities()
                .context_window,
        )?;
        let mut context_cache = context::ChatContextCache::default();
        let mut agent = AgentLoop::with_primary_context(
            composition.primary_provider(),
            &tools,
            composition.agent_profile().primary().model().clone(),
            composition.agent_profile().primary().profile().clone(),
            provider_session.clone(),
        )?
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
        let mut recovery_required = false;
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
                            return recovery::exit_status(recovery_required);
                        }
                        result => result?,
                    },
                    signal = tokio::signal::ctrl_c() => {
                        signal?;
                        console_println!("\nInterrupted while waiting for input; Session `{}` remains active.", session_id.as_str());
                        return recovery::exit_status(recovery_required);
                    }
                }
            };
            let Some(line) = line else {
                console_println!(
                    "EOF received; Session `{}` remains active.",
                    session_id.as_str()
                );
                return recovery::exit_status(recovery_required);
            };
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
                        Some(value) if value == "/end" => break Some(pasted.join("\n")),
                        Some(value) => pasted.push(value),
                        None => {
                            console_println!(
                                "EOF received during paste; Session `{}` remains active and the incomplete paste was discarded.",
                                session_id.as_str()
                            );
                            return recovery::exit_status(recovery_required);
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
                return recovery::exit_status(recovery_required);
            }
            if line == "/stop" {
                console_println!(
                    "No Turn is running. Background Jobs/PTYs, if any, are not stopped by /stop."
                );
                continue;
            }
            if line == "/context" {
                match agent.last_context_plan() {
                    Some(plan) => {
                        console_println!(
                            "{}",
                            compaction::ContextPressure::from_plan(plan).message()
                        )
                    }
                    None => console_println!(
                        "Context pressure is unavailable before the first model request; Provider usage is not inferred."
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
                if !recovery_required {
                    console_println!(
                        "No stopped Turn needs recovery; no ProviderSession was replaced."
                    );
                    continue;
                }
                let (state, provider_session, resumed, validation) =
                    session.start_or_resume(true, composition, false, &discovered_mcp)?;
                let validation = validation.as_ref().map(|facts| facts.context());
                agent.resume_from_session(&state, provider_session, validation.as_ref())?;
                disposition = resumed;
                recovery_required = false;
                console_println!(
                    "ProviderSession rebuilt from durable History; volatile reasoning cleared by explicit /resume. No request sent or Tool retried. Enter a new instruction or /compact."
                );
                continue;
            }
            if recovery_required {
                console_println!(
                    "Task input not accepted or saved: use /resume first, or /context /exit. No old input or Tool will be retried."
                );
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
                let (state, provider_session, resumed, facts) =
                    candidate_session.start_or_resume(true, composition, true, &discovered_mcp)?;
                let validation = facts.as_ref().map(|facts| facts.context());
                agent.queue_prompt_composition(definition::prompt_for_state(
                    &session.project,
                    &candidate_session.binding,
                    &state,
                    session_id.as_str(),
                )?);
                agent.resume_from_session(&state, provider_session, validation.as_ref())?;
                session.binding = candidate_session.binding;
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
                let current = context_cache.refresh(&session.project, &tools, &session.binding)?;
                agent.refresh_current_context(current)?;
                console_println!(
                    "{}",
                    session
                        .compact(
                            &mut agent,
                            composition
                                .agent_profile()
                                .primary()
                                .capabilities()
                                .context_window
                        )?
                        .message()
                );
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
            let current = context_cache.refresh(&session.project, &tools, &session.binding)?;
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
                            recovery_required = true;
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
                session
                    .record_request_audit_manifest(attempt, None)
                    .map(|manifest| {
                        accepted_attempt.replace(Some(manifest.audit_event_id().clone()));
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
                    let audit = audit
                        .as_ref()
                        .expect("a completed audited sample has an actual attempt");
                    session
                        .record_model_usage(audit, provider_session, usage)
                        .map_err(|error| AgentLoopError::ModelUsageCommit {
                            message: error.to_string(),
                        })
                }
                ChatDurabilityCheckpoint::ModelSampleRejected {
                    diagnostic,
                    corrections_used,
                } => {
                    end_answer();
                    session
                        .record_model_rejection(*diagnostic, corrections_used)
                        .map_err(|error| AgentLoopError::ModelRejectionCommit {
                            message: error.to_string(),
                        })?;
                    let limit = crate::domain::model::MAX_TOOL_ARGUMENT_CORRECTIONS_PER_TURN;
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
                context_cache
                    .refresh(&session.project, &tools, &session.binding)
                    .map_err(|error| AgentLoopError::ContextRefresh {
                        message: error.to_string(),
                    })
            };
            let improvement_task = line.clone();
            let improvement_history_start = improvement::history_start(&session)?;
            let mut batch_compaction_deferred = false;
            let mut tool_batch =
                |agent: &mut AgentLoop<'_, '_, _, ToolRegistry>,
                 active: &crate::app::agent_loop::ToolBatchContext<'_>| {
                    if !composition.runtime().auto_compact
                        || batch_compaction_deferred
                        || signal_cancellation.is_cancelled()
                        || !agent.delivery_needs_compaction()?
                    {
                        return Ok(());
                    }
                    end_answer();
                    match session.compact_tool_batch(
                        agent,
                        composition
                            .agent_profile()
                            .primary()
                            .capabilities()
                            .context_window,
                        active,
                    ) {
                        Ok(summary) => console_println!(
                            "Automatic tool-batch compaction applied. {} Current task and Turn budget retained.",
                            summary.message()
                        ),
                        Err(ChatError::CompactionFacts(
                            CompactionValidationFactsError::ResearchStreamEmpty,
                        )) => {
                            console_println!(
                                "Automatic tool-batch compaction deferred: the Project Research stream is empty; Provider delivery retained."
                            );
                            batch_compaction_deferred = true;
                        }
                        Err(ChatError::Session(SessionError::CompactionCurrentTargetRequired)) => {
                            console_println!(
                                "Automatic tool-batch compaction deferred: no current TargetSnapshot; Provider delivery retained."
                            );
                            batch_compaction_deferred = true;
                        }
                        Err(ChatError::CompactionNotUseful) => {
                            console_println!(
                                "Automatic tool-batch compaction deferred: no smaller usable Context working set; Provider delivery retained."
                            );
                            batch_compaction_deferred = true;
                        }
                        Err(error) => {
                            return Err(AgentLoopError::ToolBatchCompaction {
                                message: error.to_string(),
                            });
                        }
                    }
                    Ok(())
                };
            let outcome = {
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
                    ),
                    composition
                        .runtime()
                        .auto_compact
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

            timing.checkpoint("primary_turn_settled");
            tool_output.borrow_mut().finish_turn();
            let mut auto_compacted = false;
            match outcome {
                Ok(AgentTurnOutcome::Completed { .. }) => {
                    console_println!();
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
                        let current =
                            context_cache.refresh(&session.project, &tools, &session.binding)?;
                        agent.refresh_current_context(current)?;
                        let should_compact = agent.delivery_needs_compaction()?
                            || match agent.fresh_compaction_context_probe()? {
                                Some(plan) => {
                                    compaction::ContextPressure::from_plan(&plan).should_remind()
                                }
                                None => true,
                            };
                        if should_compact {
                            match session.compact(
                                &mut agent,
                                composition
                                    .agent_profile()
                                    .primary()
                                    .capabilities()
                                    .context_window,
                            ) {
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
                                Err(error) => return Err(error),
                            }
                        }
                    }
                }
                Ok(AgentTurnOutcome::Cancelled { .. }) => {
                    console_println!("\n[request cancelled]");
                }
                Err(error) => {
                    console_println!();
                    input::reject_pending(&mut pending_input);
                    if !agent.permits_explicit_recovery(&error) {
                        return Err(ChatError::Agent(error));
                    }
                    session.validate_stopped_runtime(&agent)?;
                    recovery_required = true;
                    recovery::report_stop(&error, &request_budget);
                }
            }
            if !recovery_required {
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
    projection: std::sync::Mutex<Option<persistence::CachedSessionProjection>>,
}

impl DurableChatSession {
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
            binding,
            projection: std::sync::Mutex::new(None),
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
        if explicit_resume {
            projection.state().ensure_no_unknown_improvement()?;
            self.preflight_binding(projection.state(), composition, discovered_mcp)?;
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
                .normalized_history_items()
                .map_err(crate::domain::session::SessionError::from)?;
            if let Some(compaction) = projection
                .state()
                .latest_compaction()
                .filter(|compaction| compaction.summary().requires_reference_validation())
            {
                let mut facts = load_resume_compaction_facts(&self.project, compaction.summary())
                    .map_err(ChatError::ResearchProjection)?;
                match compaction.validate_references(&facts.context()) {
                    Ok(()) => {},
                    Err(crate::domain::session::SessionError::CompactionEvidenceStaleResearchRevision { .. }) => {
                        let historical = load_historical_session_compaction_facts(&self.project, compaction.summary())
                            .map_err(ChatError::ResearchProjection)?;
                        compaction.validate_references(&historical.context())?;
                        facts = load_session_compaction_facts(&self.project, projection.state())
                            .map_err(ChatError::CompactionFacts)?;
                        let rebuilt = self.rebuild_compaction(
                            projection.state(),
                            &facts,
                            composition,
                            provider_session.clone(),
                            discovered_mcp,
                        )?;
                        let recovery_sequence = store.next_sequence()?;
                        let recovery = SessionEvent::CompactionCreated { compaction: rebuilt }.into_envelope(
                            event_id(self.project.project_id(), &self.session_id, recovery_sequence.get(), "compaction-rebuild"),
                            stream_id.clone(), recovery_sequence, occurred_at(),
                        )?;
                        projection.append(store, recovery).map_err(ChatError::from_session_append)?;
                    },
                    Err(error) => return Err(error.into()),
                }
                compaction_validation = Some(facts);
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
            let event = SessionEvent::ModelSampleRejected {
                request_audit_event_id,
                diagnostic,
                corrections_used,
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

    fn record_history(&self, items: &[crate::domain::model::ContentItem]) -> Result<(), ChatError> {
        if items.is_empty() {
            return Ok(());
        }
        self.with_projection(|projection, store, stream_id| {
            self.append_history_items(projection, store, &stream_id, items)
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
