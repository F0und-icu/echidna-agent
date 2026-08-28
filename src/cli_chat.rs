//! Durable first-version CLI Chat composition over the one AgentLoop.
//!
//! This UI composition module deliberately owns only the CLI-facing Session handoff: it
//! replays the authoritative JSONL stream before every append so independent
//! Tool Result Cache receipt facts cannot leave a stale projection allocating
//! duplicate sequences. Context, History, RequestAudit, Tool execution and
//! model streaming retain their existing owners.

use std::io::{self, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use thiserror::Error;
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::adapters::artifact_store::LocalArtifactStore;
use crate::adapters::fragment_cache::LocalFragmentCache;
use crate::adapters::project_event_store::read_existing_research_events;
use crate::adapters::project_event_store::{Project, ProjectEventStream};
use crate::adapters::project_snapshot::LocalProjectSnapshot;
use crate::adapters::tools::ToolRegistry;
use crate::app::agent_loop::{
    AgentLoop, AgentLoopError, AgentTurnOutcome, AssistantTextDeltaStage, ToolExecutionReceiptStage,
};
use crate::app::cancellation::AgentCancellation;
use crate::app::request_audit::RequestAuditService;
use crate::app::session::{AppendAfterDurableError, SessionProjection};
use crate::domain::context::{ProviderSession, ProviderSessionId, ProviderSessionRole};
use crate::domain::events::{EventId, OccurredAt, StreamId};
use crate::domain::project::ProjectId;
use crate::domain::request_audit::{
    RequestAuditAttempt, RequestAuditBinding, RequestAuditRunContext,
};
use crate::domain::research::{ResearchReducer, ResearchState};
use crate::domain::research_profile::ResearchProfile;
use crate::domain::session::{SessionEvent, SessionLifecycle, SessionState};
use crate::domain::tools::ToolExecutionOutcome;
use crate::domain::world_state::{ProjectSnapshotContext, ResearchWorldState};
use crate::ports::EventSource;
use crate::ports::model::ModelProvider;
use crate::ports::tool::ToolExecutor;
use crate::provider_composition::ProviderComposition;

const BASE_INSTRUCTIONS: &str = "Operate as a local-first vulnerability research assistant. Keep evidence, hypotheses, and verified findings distinct.";
static NEXT_CHAT_SESSION: AtomicU64 = AtomicU64::new(1);

/// Runs the fixed line-oriented CLI path. The caller has already loaded the
/// one configuration-driven ProviderComposition; this path never constructs a
/// Provider, URL, model, credential source, or protocol Adapter itself.
pub(crate) async fn run(
    project: Project,
    composition: &ProviderComposition,
    requested_session: Option<String>,
    profile: ResearchProfile,
) -> Result<(), ChatError> {
    let _runtime_lease = project.acquire_chat_runtime_lease()?;
    let explicit_resume = requested_session.is_some();
    let session_id = match requested_session {
        Some(session_id) => StreamId::new(session_id),
        None => new_session_id(),
    };
    let session = DurableChatSession::new(project, session_id.clone())?;
    let (session_state, provider_session, disposition) =
        session.start_or_resume(explicit_resume)?;

    let tools = ToolRegistry::for_project_with_session(session.project.root(), session_id.clone())?;
    let mut agent = AgentLoop::with_primary_context(
        composition.primary_provider(),
        &tools,
        BASE_INSTRUCTIONS,
        composition.agent_profile().primary().model().clone(),
        composition.agent_profile().primary().profile().clone(),
        provider_session.clone(),
    )?
    .with_fragment_cache(Arc::new(LocalFragmentCache::for_project(&session.project)))
    .with_research_profile(profile);
    agent.queue_agent_model_profile(composition.agent_profile().clone())?;
    if disposition == SessionDisposition::Resumed {
        agent.resume_from_session(&session_state, provider_session)?;
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
        "{} session `{}` with profile `{}`. Enter one line at a time; use /exit or EOF to leave the Session active.",
        disposition.message(),
        session_id.as_str(),
        profile.id(),
    );
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
        if line == "/exit" {
            println!(
                "Session `{}` remains active for explicit resume.",
                session_id.as_str()
            );
            return Ok(());
        }
        if line.is_empty() {
            continue;
        }

        refresh_agent_context(&session.project, &tools, &mut agent)?;
        print!("assistant> ");
        io::stdout().flush()?;

        let cancellation = AgentCancellation::new();
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
        let mut record_execution = |outcome: &ToolExecutionOutcome| {
            session
                .record_execution(outcome)
                .map_err(ChatError::into_execution_receipt_agent_loop_error)
        };
        let outcome = {
            let turn = agent.run_turn_audited_with_stages(
                line,
                cancellation,
                &audit_context,
                &mut record_attempt,
                &mut text_delta as &mut AssistantTextDeltaStage<'_>,
                &mut record_execution as &mut ToolExecutionReceiptStage<'_>,
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
    let research_events = read_existing_research_events(project)?;
    let research_state: ResearchState = ResearchReducer.reduce(research_events)?;
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
    ) -> Result<(SessionState, ProviderSession, SessionDisposition), ChatError> {
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
        let (event, disposition) = if explicit_resume {
            if projection.state().lifecycle() != SessionLifecycle::Active {
                return Err(ChatError::SessionNotActive {
                    session_id: self.session_id.as_str().to_owned(),
                });
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
        Ok((projection.state().clone(), provider_session, disposition))
    }

    fn record_request_audit(&self, attempt: RequestAuditAttempt) -> Result<(), ChatError> {
        let mut writer = self.project.open_writer()?;
        let mut store = writer.open_session(self.session_id.clone(), None)?;
        let stream_id = ProjectEventStream::Session(self.session_id.clone()).stream_id();
        let mut projection =
            SessionProjection::replay(EventSource::read_events(&store, &stream_id)?)?;
        let sequence = store.next_sequence()?;
        let service = RequestAuditService::new(
            &self.artifacts,
            RequestAuditBinding::new(self.project.project_id().clone(), stream_id)?,
        );
        service
            .record(
                attempt,
                event_id(
                    self.project.project_id(),
                    &self.session_id,
                    sequence.get(),
                    "request-audit",
                ),
                occurred_at(),
                &mut projection,
                &mut store,
            )
            .map(|_| ())
            .map_err(|error| ChatError::Agent(error.into_agent_loop_error()))
    }

    fn record_execution(&self, outcome: &ToolExecutionOutcome) -> Result<(), ChatError> {
        let mut writer = self.project.open_writer()?;
        let mut store = writer.open_session(self.session_id.clone(), None)?;
        let stream_id = ProjectEventStream::Session(self.session_id.clone()).stream_id();
        let mut projection =
            SessionProjection::replay(EventSource::read_events(&store, &stream_id)?)?;
        if projection
            .state()
            .execution_receipts()
            .iter()
            .any(|record| record.receipt == outcome.receipt)
        {
            return Ok(());
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
            .append(&mut store, event)
            .map_err(ChatError::from_session_append)?;
        Ok(())
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
    #[error(transparent)]
    WorldState(#[from] crate::domain::world_state::WorldStateError),
    #[error("Session `{session_id}` does not exist in the current project")]
    SessionNotFound { session_id: String },
    #[error("Session `{session_id}` is not active and cannot be resumed")]
    SessionNotActive { session_id: String },
    #[error("Session `{session_id}` already exists; resume it explicitly with --session")]
    SessionAlreadyExists { session_id: String },
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
