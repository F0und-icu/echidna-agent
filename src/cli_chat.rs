//! Durable first-version CLI Chat composition over the one AgentLoop.
//!
//! This UI composition module deliberately owns only the CLI-facing Session handoff: it
//! replays the authoritative JSONL stream before every append so independent
//! Tool Result Cache receipt facts cannot leave a stale projection allocating
//! duplicate sequences. Context, History, RequestAudit, Tool execution and
//! model streaming retain their existing owners.

mod compaction;
mod vision;

use std::io::{self, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use thiserror::Error;
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::adapters::artifact_store::LocalArtifactStore;
use crate::adapters::fragment_cache::LocalFragmentCache;
use crate::adapters::project_event_store::{Project, ProjectEventStream};
use crate::adapters::project_snapshot::LocalProjectSnapshot;
use crate::adapters::research_projection::{
    CompactionValidationFacts, load_compaction_validation_facts, load_research_state,
};
use crate::adapters::tools::ToolRegistry;
use crate::app::agent_loop::{
    AgentLoop, AgentLoopError, AgentTurnOutcome, AssistantTextDeltaStage, CliRuntimeStages,
    ToolExecutionReceiptStage, ToolExecutionStartStage, ViewImageFollowUpFuture,
};
use crate::app::cancellation::AgentCancellation;
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
use crate::domain::session::{SessionEvent, SessionLifecycle, SessionState};
use crate::domain::tools::ToolExecutionOutcome;
use crate::domain::world_state::{ProjectSnapshotContext, ResearchWorldState};
use crate::ports::EventSource;
use crate::ports::model::ModelProvider;
use crate::ports::tool::ToolExecutor;
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
    let (session_state, provider_session, disposition, compaction_validation) =
        session.start_or_resume(explicit_resume)?;
    let domain = requested_domain.unwrap_or_else(|| session_state.effective_domain_profile());

    let tools = ToolRegistry::for_project(session.project.root());
    let mut agent = AgentLoop::with_primary_context(
        composition.primary_provider(),
        &tools,
        composition.agent_profile().primary().model().clone(),
        composition.agent_profile().primary().profile().clone(),
        provider_session.clone(),
    )?
    .with_fragment_cache(Arc::new(LocalFragmentCache::for_project(&session.project)))
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

    println!(
        "{} session `{}` with domain `{}` and phase `{}`. Enter a line, or use /paste, /image, /domain, /phase, /context, /compact, /exit or EOF.",
        disposition.message(),
        session_id.as_str(),
        domain.id(),
        requested_phase
            .unwrap_or_else(|| session_state.effective_research_phase())
            .id(),
    );
    let mut pending_domain = requested_domain.map(DomainProfileSelection::operator);
    let mut pending_phase = requested_phase.map(ResearchPhaseSelection::operator);
    let mut pending_image: Option<vision::PendingImage> = None;
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    loop {
        print!("you> ");
        io::stdout().flush()?;
        let line = tokio::select! {
            line = lines.next_line() => line?,
            signal = tokio::signal::ctrl_c() => {
                signal?;
                println!("\nInterrupted while waiting for input; Session `{}` remains active.", session_id.as_str());
                return Ok(());
            }
        };
        let Some(line) = line else {
            println!(
                "EOF received; Session `{}` remains active.",
                session_id.as_str()
            );
            return Ok(());
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
                        return Ok(());
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
            return Ok(());
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
        if line == "/compact" {
            if let Some(image) = &pending_image {
                println!(
                    "Compaction is unavailable while image `{}` is queued; submit its Turn first.",
                    image.delivery_artifact_id()
                );
                continue;
            }
            println!("{}", session.compact(&mut agent)?.message());
            continue;
        }
        if line == "/image" {
            println!("Usage: /image <path>");
            continue;
        }
        if let Some(path) = line.strip_prefix("/image ") {
            if let Some(image) = &pending_image {
                println!(
                    "Image `{}` is already queued for the next Turn; submit a prompt before attaching another image.",
                    image.delivery_artifact_id()
                );
                continue;
            }
            match vision::ingest_image(&session, &tools, composition, path).await {
                Ok(image) => {
                    println!(
                        "Queued image `{}` for TargetSnapshot `{}` and the next Turn.",
                        image.delivery_artifact_id(),
                        image
                            .target_snapshot_id()
                            .expect("operator image intake always binds a TargetSnapshot")
                    );
                    pending_image = Some(image);
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

        refresh_agent_context(&session.project, &tools, &mut agent)?;
        let cancellation = AgentCancellation::new();
        let initial_image = match pending_image.take() {
            Some(image) => {
                let preparation = vision::prepare_primary_context(
                    &session,
                    composition,
                    image,
                    &line,
                    cancellation.clone(),
                );
                tokio::pin!(preparation);
                let prepared = tokio::select! {
                    result = &mut preparation => result,
                    signal = tokio::signal::ctrl_c() => {
                        signal?;
                        cancellation.cancel();
                        match preparation.as_mut().await {
                            Ok(_) | Err(ChatError::VisionCancelled) => {
                                println!("\n[vision request cancelled; no Primary request was started]");
                                continue;
                            }
                            Err(error) => return Err(error),
                        }
                    }
                }?;
                Some(prepared)
            }
            None => None,
        };
        print!("assistant> ");
        io::stdout().flush()?;

        let signal_cancellation = cancellation.clone();
        let history_start = agent.history().items().len();
        let mut text_delta = |text: &str| {
            print!("{text}");
            io::stdout()
                .flush()
                .map_err(|error| AgentLoopError::TextDeltaOutput {
                    message: error.to_string(),
                })
        };
        let mut record_attempt = |attempt: RequestAuditAttempt| {
            session
                .record_request_audit(attempt)
                .map_err(ChatError::into_request_audit_agent_loop_error)
        };
        let mut tool_start = |plan: &crate::domain::tools::ToolExecutionPlan| {
            let _ = writeln!(
                io::stderr(),
                "[tool started] name={} execution_id={} call_id={}",
                plan.call.name,
                plan.execution_id,
                plan.call.call_id
            );
        };
        let mut record_execution = |outcome: &ToolExecutionOutcome| {
            let disposition = session
                .record_execution(outcome)
                .map_err(ChatError::into_execution_receipt_agent_loop_error)?;
            match disposition {
                ReceiptRecordDisposition::Recorded { event_id } => {
                    let _ = writeln!(
                        io::stderr(),
                        "[tool {}] name={} execution_id={} receipt_event={} call_id={}",
                        tool_terminal_label(outcome.receipt.status),
                        outcome.result.tool_name,
                        outcome.receipt.execution_id,
                        event_id,
                        outcome.result.call_id
                    );
                }
                ReceiptRecordDisposition::Existing { event_id } => {
                    let _ = writeln!(
                        io::stderr(),
                        "[tool deduplicated] name={} source_execution_id={} source_receipt_event={} call_id={}",
                        outcome.result.tool_name,
                        outcome.receipt.execution_id,
                        event_id,
                        outcome.result.call_id
                    );
                }
            }
            Ok(())
        };
        let follow_up_session = &session;
        let follow_up_composition = composition;
        let follow_up_task = line.clone();
        let follow_up_cancellation = cancellation.clone();
        let mut follow_up = move |selection| -> ViewImageFollowUpFuture<'_> {
            let task = follow_up_task.clone();
            let cancellation = follow_up_cancellation.clone();
            Box::pin(async move {
                vision::prepare_primary_context(
                    follow_up_session,
                    follow_up_composition,
                    vision::PendingImage::from_selection(selection),
                    &task,
                    cancellation,
                )
                .await
                .map_err(vision::follow_up_error)
            })
        };
        let outcome = {
            let turn = agent.run_turn_audited_with_vision_stages(
                line,
                initial_image,
                cancellation,
                &audit_context,
                &mut record_attempt,
                CliRuntimeStages::new(
                    &mut follow_up,
                    &mut text_delta as &mut AssistantTextDeltaStage<'_>,
                    &mut tool_start as &mut ToolExecutionStartStage<'_>,
                    &mut record_execution as &mut ToolExecutionReceiptStage<'_>,
                ),
            );
            tokio::pin!(turn);
            tokio::select! {
                outcome = &mut turn => outcome,
                signal = tokio::signal::ctrl_c() => {
                    signal?;
                    signal_cancellation.cancel();
                    turn.as_mut().await
                }
            }
        };

        match outcome {
            Ok(AgentTurnOutcome::Completed { .. }) => {
                session.record_history(&agent.history().items()[history_start..])?;
                println!();
            }
            Ok(AgentTurnOutcome::Cancelled { .. }) => {
                session.record_history(&agent.history().items()[history_start..])?;
                println!("\n[request cancelled]");
            }
            Err(error) => {
                session.record_history(&agent.history().items()[history_start..])?;
                println!();
                return Err(ChatError::Agent(error));
            }
        }
        if let Some(plan) = agent.last_context_plan() {
            let pressure = compaction::ContextPressure::from_plan(plan);
            if pressure.should_remind() {
                println!("{}", pressure.message());
            }
        }
    }
}

fn refresh_agent_context<Provider>(
    project: &Project,
    tools: &ToolRegistry,
    agent: &mut AgentLoop<'_, '_, Provider, ToolRegistry>,
) -> Result<(), ChatError>
where
    Provider: ModelProvider + ?Sized,
{
    let research_state = load_research_state(project).map_err(ChatError::ResearchProjection)?;
    agent.set_world_state(ResearchWorldState::from_research_state(&research_state)?);
    let timestamp = occurred_at();
    let snapshot =
        LocalProjectSnapshot::new(project).inspect(&tools.specs(), &[], timestamp.as_str())?;
    agent.set_project_snapshot(ProjectSnapshotContext::new(snapshot.snapshot)?);
    Ok(())
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
            if let Some(compaction) = projection
                .state()
                .latest_compaction()
                .filter(|compaction| compaction.summary().schema_version() == 2)
            {
                let facts = load_compaction_validation_facts(&self.project)
                    .map_err(ChatError::ResearchProjection)?;
                compaction.validate_references(&facts.context())?;
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
        if let Some(record) = projection
            .state()
            .execution_receipts()
            .iter()
            .find(|record| record.receipt == outcome.receipt)
        {
            return Ok(ReceiptRecordDisposition::Existing {
                event_id: record.event_id.clone(),
            });
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
        .into_envelope(receipt_event_id.clone(), stream_id, sequence, occurred_at())?;
        projection
            .append(&mut store, event)
            .map_err(ChatError::from_session_append)?;
        Ok(ReceiptRecordDisposition::Recorded {
            event_id: receipt_event_id,
        })
    }

    fn record_history(&self, items: &[crate::domain::model::ContentItem]) -> Result<(), ChatError> {
        for item in items {
            let mut writer = self.project.open_writer()?;
            let mut store = writer.open_session(self.session_id.clone(), None)?;
            let stream_id = ProjectEventStream::Session(self.session_id.clone()).stream_id();
            let mut projection =
                SessionProjection::replay(EventSource::read_events(&store, &stream_id)?)?;
            let sequence = store.next_sequence()?;
            let event = SessionEvent::HistoryItemRecorded { item: item.clone() }.into_envelope(
                event_id(
                    self.project.project_id(),
                    &self.session_id,
                    sequence.get(),
                    "history",
                ),
                stream_id,
                sequence,
                occurred_at(),
            )?;
            projection
                .append(&mut store, event)
                .map_err(ChatError::from_session_append)?;
        }
        Ok(())
    }
}

enum ReceiptRecordDisposition {
    Recorded { event_id: EventId },
    Existing { event_id: EventId },
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

fn tool_terminal_label(status: crate::domain::tools::ToolResultStatus) -> &'static str {
    match status {
        crate::domain::tools::ToolResultStatus::Succeeded => "completed",
        crate::domain::tools::ToolResultStatus::Failed => "failed",
        crate::domain::tools::ToolResultStatus::Cancelled => "cancelled",
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
    RequestAudit(#[from] crate::domain::request_audit::RequestAuditError),
    #[error("rebuild Project Snapshot for Chat: {0}")]
    ProjectSnapshot(#[from] crate::adapters::project_snapshot::LocalProjectSnapshotError),
    #[error(transparent)]
    Research(#[from] crate::domain::research::ResearchError),
    #[error("Research projection is invalid: {0}")]
    ResearchProjection(String),
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
        "strict Session History changed independently from the active AgentLoop; Compaction was not appended"
    )]
    CompactionRuntimeHistoryMismatch,
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

    fn into_execution_receipt_agent_loop_error(self) -> AgentLoopError {
        match self {
            Self::Agent(error) => error,
            error => AgentLoopError::ExecutionReceiptCommit {
                message: error.to_string(),
            },
        }
    }
}
