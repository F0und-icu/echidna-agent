//! Durable first-version CLI Chat composition over the one AgentLoop.
//!
//! This UI composition module deliberately owns only the CLI-facing Session handoff: it
//! replays the authoritative JSONL stream before every append so independent
//! Tool Result Cache receipt facts cannot leave a stale projection allocating
//! duplicate sequences. Context, History, RequestAudit, Tool execution and
//! model streaming retain their existing owners.

mod compaction;
mod context;
mod input;
mod recovery;
mod theme;
mod tool_output;
mod vision;

use std::cell::Cell;
use std::io::{self, IsTerminal, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use thiserror::Error;
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::adapters::artifact_store::LocalArtifactStore;
use crate::adapters::fragment_cache::LocalFragmentCache;
use crate::adapters::project_event_store::{Project, ProjectEventStream};
use crate::adapters::research_projection::{
    CompactionValidationFacts, CompactionValidationFactsError, load_compaction_validation_facts,
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
use crate::ports::EventSource;
use crate::provider_composition::ProviderComposition;

static NEXT_CHAT_SESSION: AtomicU64 = AtomicU64::new(1);

/// Runs the fixed line-oriented CLI path. The caller has already loaded the
/// one configuration-driven ProviderComposition; this path never constructs a
/// Provider, URL, model, credential source, or protocol Adapter itself.
pub(crate) async fn run(
    project: Project,
    composition: &ProviderComposition,
    requested_session: Option<String>,
    requested_domain: Option<DomainProfile>,
    requested_phase: Option<ResearchPhase>,
) -> Result<(), ChatError> {
    let _runtime_lease = project.acquire_chat_runtime_lease()?;
    let explicit_resume = requested_session.is_some();
    let session_id = match requested_session {
        Some(session_id) => StreamId::new(session_id),
        None => new_session_id(),
    };
    let session = DurableChatSession::new(project, session_id.clone())?;
    let tools = composition
        .discover_chat_tools(session.project.root())
        .await?
        .with_project(&session.project)
        .with_context_window(
            composition
                .agent_profile()
                .primary()
                .capabilities()
                .context_window,
        );
    let (session_state, provider_session, mut disposition, compaction_validation) =
        session.start_or_resume(explicit_resume, composition, &tools)?;
    let domain = requested_domain.unwrap_or_else(|| session_state.effective_domain_profile());

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
    .with_domain_profile(domain);
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
    println!(
        "{} session `{}` with domain `{}` and phase `{}`. Enter a line, or use /paste, /image, /domain, /phase, /context, /compact, /resume, /stop, /exit or EOF. While running in a terminal, /stop or a new instruction interrupts the current Turn before further input is handled.",
        disposition.message(),
        session_id.as_str(),
        domain.id(),
        requested_phase
            .unwrap_or_else(|| session_state.effective_research_phase())
            .id(),
    );
    let mut pending_domain = requested_domain.map(DomainProfileSelection::operator);
    let mut pending_phase = requested_phase.map(ResearchPhaseSelection::operator);
    let mut pending_images: Vec<vision::PendingImage> = Vec::new();
    let mut recovery_required = false;
    let terminal_input = io::stdin().is_terminal();
    let mut input_closed = false;
    let mut pending_input = None;
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let tool_output = std::cell::RefCell::new(tool_output::ToolOutput::default());
    loop {
        print!("you> ");
        io::stdout().flush()?;
        let line = if pending_input.is_some() {
            pending_input.take()
        } else if input_closed {
            None
        } else {
            tokio::select! {
                line = lines.next_line() => line?,
                signal = tokio::signal::ctrl_c() => {
                    signal?;
                    println!("\nInterrupted while waiting for input; Session `{}` remains active.", session_id.as_str());
                    return recovery::exit_status(recovery_required);
                }
            }
        };
        let Some(line) = line else {
            println!(
                "EOF received; Session `{}` remains active.",
                session_id.as_str()
            );
            return recovery::exit_status(recovery_required);
        };
        let line = if line == "/paste" {
            println!("Paste mode; finish with /end on its own line.");
            let mut pasted = Vec::new();
            loop {
                print!("...> ");
                io::stdout().flush()?;
                let next = tokio::select! {
                    line = lines.next_line() => line?,
                    signal = tokio::signal::ctrl_c() => {
                        signal?;
                        println!("\n[paste cancelled]");
                        pasted.clear();
                        break None;
                    }
                };
                match next {
                    Some(value) if value == "/end" => break Some(pasted.join("\n")),
                    Some(value) => pasted.push(value),
                    None => {
                        println!(
                            "EOF received during paste; Session `{}` remains active and the incomplete paste was discarded.",
                            session_id.as_str()
                        );
                        return recovery::exit_status(recovery_required);
                    }
                }
            }
        } else {
            Some(line)
        };
        let Some(line) = line else {
            continue;
        };
        if line.is_empty() {
            println!("Empty input ignored.");
            continue;
        }
        if line == "/exit" {
            println!(
                "Session `{}` remains active for explicit resume.",
                session_id.as_str()
            );
            return recovery::exit_status(recovery_required);
        }
        if line == "/stop" {
            println!("No Turn is running. Background Jobs/PTYs, if any, are not stopped by /stop.");
            continue;
        }
        if line == "/context" {
            match agent.last_context_plan() {
                Some(plan) => {
                    println!("{}", compaction::ContextPressure::from_plan(plan).message())
                }
                None => println!(
                    "Context pressure is unavailable before the first model request; Provider usage is not inferred."
                ),
            }
            continue;
        }
        if line == "/resume" {
            if !recovery_required {
                println!("No stopped Turn needs recovery; no ProviderSession was replaced.");
                continue;
            }
            let (state, provider_session, resumed, validation) =
                session.start_or_resume(true, composition, &tools)?;
            let validation = validation.as_ref().map(|facts| facts.context());
            agent.resume_from_session(&state, provider_session, validation.as_ref())?;
            disposition = resumed;
            recovery_required = false;
            println!(
                "ProviderSession rebuilt from durable History; volatile reasoning cleared by explicit /resume. No request sent or Tool retried. Enter a new instruction or /compact."
            );
            continue;
        }
        if recovery_required {
            println!(
                "Task input not accepted or saved: use /resume first, or /context /exit. No old input or Tool will be retried."
            );
            continue;
        }
        if line == "/compact" {
            if let Some(image) = pending_images.first() {
                println!(
                    "Compaction is unavailable while {} image(s), starting with `{}`, are queued; submit their Turn first.",
                    pending_images.len(),
                    image.delivery_artifact_id(),
                );
                continue;
            }
            let current = context_cache.refresh(&session.project, &tools, composition)?;
            agent.refresh_current_context(current)?;
            println!(
                "{}",
                session
                    .compact(
                        &mut agent,
                        crate::domain::context::ContextWindowPolicy::for_window(
                            composition
                                .agent_profile()
                                .primary()
                                .capabilities()
                                .context_window,
                        ),
                    )?
                    .message()
            );
            continue;
        }
        if line == "/image" {
            println!("Usage: /image <path>");
            continue;
        }
        if let Some(path) = line.strip_prefix("/image ") {
            match vision::ingest_image(&session, &tools, composition, path).await {
                Ok(image) => {
                    println!(
                        "Queued image `{}` for TargetSnapshot `{}` and the next Turn.",
                        image.delivery_artifact_id(),
                        image
                            .target_snapshot_id()
                            .expect("operator image intake always binds a TargetSnapshot")
                    );
                    pending_images.push(image);
                }
                Err(error) => println!("Image rejected: {error}"),
            }
            continue;
        }
        if let Some(value) = line.strip_prefix("/phase ") {
            let phase = parse_phase(value)?;
            pending_phase = Some(ResearchPhaseSelection::operator(phase));
            println!("Queued phase `{}` for the next Turn.", phase.id());
            continue;
        }
        if let Some(value) = line.strip_prefix("/domain ") {
            let selected_domain = parse_domain(value)?;
            pending_domain = Some(DomainProfileSelection::operator(selected_domain));
            println!(
                "Queued domain `{}` for the next Turn.",
                selected_domain.id()
            );
            continue;
        }

        let domain = session.prepare_domain_for_turn(disposition, pending_domain.take())?;
        agent.queue_domain_profile(domain);
        let phase = session.prepare_phase_for_turn(disposition, pending_phase.take())?;
        agent.queue_research_phase(phase);

        // Prompt/core filesystem failures occur before a possible scoped
        // Vision request as well as before the Primary Provider request.
        let current = context_cache.refresh(&session.project, &tools, composition)?;
        agent.refresh_current_context(current)?;
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
                &mut lines,
                terminal_input,
                &mut input_closed,
                &mut pending_input,
            )
            .await?;
            if cancellation.is_cancelled() {
                match prepared {
                    Ok(_) | Err(ChatError::VisionCancelled) => {
                        println!("\n[vision request cancelled; no Primary request was started]");
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
                        println!(
                            "Initial images/task were not submitted to Primary; reattach each image explicitly after /resume if needed."
                        );
                        continue;
                    }
                    return Err(ChatError::Agent(error));
                }
            }
        };
        let signal_cancellation = cancellation.clone();
        let answer_started = Cell::new(false);
        let end_answer = || {
            if answer_started.replace(false) {
                println!();
            }
        };
        let mut text_delta = |text: &str| {
            if !answer_started.get() && !text.is_empty() {
                print!("{}", theme.answer_prefix());
                answer_started.set(true);
            }
            print!("{text}");
            io::stdout()
                .flush()
                .map_err(|error| AgentLoopError::TextDeltaOutput {
                    message: error.to_string(),
                })
        };
        let mut record_attempt = |attempt: RequestAuditAttempt| {
            // Sampling boundaries affect only presentation, never History bytes.
            end_answer();
            session
                .record_request_audit(attempt)
                .map_err(ChatError::into_request_audit_agent_loop_error)
        };
        let mut tool_start = |plan: &crate::domain::tools::ToolExecutionPlan| {
            end_answer();
            tool_output.borrow_mut().started(plan);
        };
        let mut durability = |checkpoint: ChatDurabilityCheckpoint<'_>| match checkpoint {
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
                eprintln!(
                    "\n[model sample rejected] {diagnostic}; streamed prose was not committed and no Tool from this sample ran."
                );
                if corrections_used < limit {
                    eprintln!(
                        "[argument correction {}/{limit}] Regenerating the failed sample if budget permits; completed tools will not be replayed.",
                        corrections_used + 1
                    );
                } else {
                    eprintln!(
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
                .refresh(&session.project, &tools, composition)
                .map_err(|error| AgentLoopError::ContextRefresh {
                    message: error.to_string(),
                })
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
            );
            input::interruptible(
                turn,
                &signal_cancellation,
                &mut lines,
                terminal_input,
                &mut input_closed,
                &mut pending_input,
            )
            .await?
        };

        tool_output.borrow_mut().finish_turn();
        let mut auto_compacted = false;
        match outcome {
            Ok(AgentTurnOutcome::Completed { .. }) => {
                println!();
                if composition.runtime().auto_compact {
                    let current = context_cache.refresh(&session.project, &tools, composition)?;
                    agent.refresh_current_context(current)?;
                    let should_compact = match agent.fresh_compaction_context_probe() {
                        Ok(Some(plan)) => {
                            compaction::ContextPressure::from_plan(&plan).should_remind()
                        }
                        Ok(None) => true,
                        Err(error) => return Err(ChatError::Agent(error)),
                    };
                    if should_compact {
                        match session.compact(
                            &mut agent,
                            crate::domain::context::ContextWindowPolicy::for_window(
                                composition
                                    .agent_profile()
                                    .primary()
                                    .capabilities()
                                    .context_window,
                            ),
                        ) {
                            Err(ChatError::CompactionFacts(
                                CompactionValidationFactsError::ResearchStreamEmpty,
                            )) => {
                                println!(
                                    "Automatic completed-Turn compaction deferred: the Project Research stream is empty; no Compaction was appended and Provider delivery state was retained."
                                );
                            }
                            Ok(summary) => {
                                println!("{}", summary.automatic_message());
                                auto_compacted = true;
                            }
                            Err(ChatError::Session(
                                SessionError::CompactionCurrentTargetRequired,
                            )) => {
                                println!(
                                    "Automatic completed-Turn compaction deferred: the current Project has no TargetSnapshot; no Compaction was appended and Provider delivery state was retained."
                                );
                            }
                            Err(ChatError::CompactionNotUseful) => {
                                println!(
                                    "Automatic completed-Turn compaction deferred: no smaller usable next-Turn Context was found; no Compaction was appended and Provider delivery state was retained."
                                );
                            }
                            Err(error) => return Err(error),
                        }
                    }
                }
            }
            Ok(AgentTurnOutcome::Cancelled { .. }) => {
                println!("\n[request cancelled]");
            }
            Err(error) => {
                println!();
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
                println!("{}", pressure.message());
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

/// The fixed Session persistence stage for first-version Chat. It always
/// replays authoritative Session facts under the existing project Writer
/// lock, so independently appended cache Receipts are never guessed or
/// duplicated by a stale in-memory projection.
struct DurableChatSession {
    project: Project,
    session_id: StreamId,
    artifacts: LocalArtifactStore,
}

impl DurableChatSession {
    fn new(project: Project, session_id: StreamId) -> Result<Self, ChatError> {
        let artifacts = LocalArtifactStore::for_project(&project);
        Ok(Self {
            project,
            session_id,
            artifacts,
        })
    }

    fn start_or_resume(
        &self,
        explicit_resume: bool,
        composition: &ProviderComposition,
        tools: &ToolRegistry,
    ) -> Result<
        (
            SessionState,
            ProviderSession,
            SessionDisposition,
            Option<CompactionValidationFacts>,
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

        let mut writer = self.project.open_writer()?;
        let mut store = writer.open_session(self.session_id.clone(), None)?;
        let stream_id = ProjectEventStream::Session(self.session_id.clone()).stream_id();
        let mut projection =
            SessionProjection::replay(EventSource::read_events(&store, &stream_id)?)?;
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
                .filter(|compaction| compaction.summary().schema_version() == 2)
            {
                let facts = load_compaction_validation_facts(&self.project)
                    .map_err(|error| ChatError::ResearchProjection(error.to_string()))?;
                match compaction.validate_references(&facts.context()) {
                    Ok(()) => {},
                    Err(crate::domain::session::SessionError::CompactionEvidenceStaleResearchRevision { .. }) => {
                        let historical = crate::adapters::research_projection::load_historical_compaction_facts(
                            &self.project,
                            compaction.summary().evidence_spine().expect("v2 has an EvidenceSpine").source(),
                        ).map_err(ChatError::ResearchProjection)?;
                        compaction.validate_references(&historical.context())?;
                        let rebuilt = self.rebuild_compaction(
                            projection.state(),
                            &facts,
                            composition,
                            provider_session.clone(),
                            tools,
                        )?;
                        let recovery_sequence = store.next_sequence()?;
                        let recovery = SessionEvent::CompactionCreated { compaction: rebuilt }.into_envelope(
                            event_id(self.project.project_id(), &self.session_id, recovery_sequence.get(), "compaction-rebuild"),
                            stream_id.clone(), recovery_sequence, occurred_at(),
                        )?;
                        projection.append(&mut store, recovery).map_err(ChatError::from_session_append)?;
                    },
                    Err(error) => return Err(error.into()),
                }
                compaction_validation = Some(facts);
            }
            (
                SessionEvent::Resumed {
                    primary_provider_session: provider_session.reference().clone(),
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
                SessionEvent::Started {
                    primary_provider_session: provider_session.reference().clone(),
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
            .append(&mut store, event)
            .map_err(ChatError::from_session_append)?;
        Ok((
            projection.state().clone(),
            provider_session,
            disposition,
            compaction_validation,
        ))
    }

    /// Makes the selected Phase durable before AgentLoop can apply it or send
    /// a Provider request. A failed append leaves caller configuration alone.
    fn prepare_phase_for_turn(
        &self,
        disposition: SessionDisposition,
        requested: Option<ResearchPhaseSelection>,
    ) -> Result<ResearchPhase, ChatError> {
        let mut writer = self.project.open_writer()?;
        let mut store = writer.open_session(self.session_id.clone(), None)?;
        let stream_id = ProjectEventStream::Session(self.session_id.clone()).stream_id();
        let mut projection =
            SessionProjection::replay(EventSource::read_events(&store, &stream_id)?)?;
        let selection = match requested {
            Some(selection) => selection,
            None => match projection.state().research_phase_selection() {
                Some(selection) => return Ok(selection.phase()),
                None if disposition == SessionDisposition::Started => {
                    ResearchPhaseSelection::new_session_default()
                }
                None => ResearchPhaseSelection::legacy_default(),
            },
        };
        let sequence = store.next_sequence()?;
        let event = SessionEvent::ResearchPhaseSelected { selection }.into_envelope(
            event_id(
                self.project.project_id(),
                &self.session_id,
                sequence.get(),
                "research-phase",
            ),
            stream_id,
            sequence,
            occurred_at(),
        )?;
        projection
            .append(&mut store, event)
            .map_err(ChatError::from_session_append)?;
        Ok(selection.phase())
    }

    /// Makes the selected Domain durable before AgentLoop can apply it or
    /// send a Provider request. Legacy replay remains read-only until here.
    fn prepare_domain_for_turn(
        &self,
        disposition: SessionDisposition,
        requested: Option<DomainProfileSelection>,
    ) -> Result<DomainProfile, ChatError> {
        let mut writer = self.project.open_writer()?;
        let mut store = writer.open_session(self.session_id.clone(), None)?;
        let stream_id = ProjectEventStream::Session(self.session_id.clone()).stream_id();
        let mut projection =
            SessionProjection::replay(EventSource::read_events(&store, &stream_id)?)?;
        let selection = match requested {
            Some(selection) => selection,
            None => match projection.state().domain_profile_selection() {
                Some(selection) => return Ok(selection.profile()),
                None if disposition == SessionDisposition::Started => {
                    DomainProfileSelection::new_session_default()
                }
                None => DomainProfileSelection::legacy_default(),
            },
        };
        let sequence = store.next_sequence()?;
        let event = SessionEvent::DomainProfileSelected { selection }.into_envelope(
            event_id(
                self.project.project_id(),
                &self.session_id,
                sequence.get(),
                "domain-profile",
            ),
            stream_id,
            sequence,
            occurred_at(),
        )?;
        projection
            .append(&mut store, event)
            .map_err(ChatError::from_session_append)?;
        Ok(selection.profile())
    }

    fn record_request_audit(&self, attempt: RequestAuditAttempt) -> Result<(), ChatError> {
        self.record_request_audit_manifest(attempt, None)
            .map(|_| ())
    }

    fn record_stamped_request_audit(
        &self,
        attempt: RequestAuditAttempt,
        event_id: EventId,
        occurred_at: OccurredAt,
    ) -> Result<RequestAuditManifest, ChatError> {
        self.record_request_audit_manifest(attempt, Some((event_id, occurred_at)))
    }

    fn record_request_audit_manifest(
        &self,
        attempt: RequestAuditAttempt,
        stamp: Option<(EventId, OccurredAt)>,
    ) -> Result<RequestAuditManifest, ChatError> {
        let mut writer = self.project.open_writer()?;
        let mut store = writer.open_session(self.session_id.clone(), None)?;
        let stream_id = ProjectEventStream::Session(self.session_id.clone()).stream_id();
        let mut projection =
            SessionProjection::replay(EventSource::read_events(&store, &stream_id)?)?;
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
            .record(attempt, event_id, occurred_at, &mut projection, &mut store)
            .map_err(|error| ChatError::Agent(error.into_agent_loop_error()))
    }

    fn record_execution(
        &self,
        outcome: &ToolExecutionOutcome,
    ) -> Result<ReceiptRecordDisposition, ChatError> {
        let mut writer = self.project.open_writer()?;
        let mut store = writer.open_session(self.session_id.clone(), None)?;
        let stream_id = ProjectEventStream::Session(self.session_id.clone()).stream_id();
        let mut projection =
            SessionProjection::replay(EventSource::read_events(&store, &stream_id)?)?;
        self.record_execution_receipt(&mut projection, &mut store, &stream_id, outcome)
    }

    fn record_subcommand(
        &self,
        checkpoint: &crate::domain::tools::SubcommandCheckpoint,
    ) -> Result<(), ChatError> {
        let mut writer = self.project.open_writer()?;
        let mut store = writer.open_session(self.session_id.clone(), None)?;
        let stream_id = ProjectEventStream::Session(self.session_id.clone()).stream_id();
        let mut projection =
            SessionProjection::replay(EventSource::read_events(&store, &stream_id)?)?;
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
            .append(&mut store, event)
            .map_err(ChatError::from_session_append)?;
        Ok(())
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
            .any(|record| record.receipt == outcome.receipt)
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
        let event = if outcome.receipt.cache_source.is_some() {
            SessionEvent::ToolResultCacheHit {
                receipt: outcome.receipt.clone(),
            }
        } else {
            SessionEvent::ExecutionReceiptRecorded {
                receipt: outcome.receipt.clone(),
            }
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
        let mut writer = self.project.open_writer()?;
        let mut store = writer.open_session(self.session_id.clone(), None)?;
        let stream_id = ProjectEventStream::Session(self.session_id.clone()).stream_id();
        let mut projection =
            SessionProjection::replay(EventSource::read_events(&store, &stream_id)?)?;
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
            .append(&mut store, event)
            .map_err(ChatError::from_session_append)?;
        Ok(())
    }

    fn record_history(&self, items: &[crate::domain::model::ContentItem]) -> Result<(), ChatError> {
        if items.is_empty() {
            return Ok(());
        }
        let mut writer = self.project.open_writer()?;
        let mut store = writer.open_session(self.session_id.clone(), None)?;
        let stream_id = ProjectEventStream::Session(self.session_id.clone()).stream_id();
        let mut projection =
            SessionProjection::replay(EventSource::read_events(&store, &stream_id)?)?;
        self.append_history_items(&mut projection, &mut store, &stream_id, items)
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
    Mcp(#[from] crate::adapters::mcp_http::McpError),
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
                .then_some(call_id)
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
