//! Read-only Session administration and the one explicit lifecycle close.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::adapters::project_event_store::{
    Project, ProjectEventStream, read_existing_session_events,
};
use crate::app::session::SessionProjection;
use crate::domain::events::{EventId, OccurredAt, StreamId};
use crate::domain::model::{ContentItem, MessageRole};
use crate::domain::session::{SessionEvent, SessionLifecycle, SessionState};
use crate::ports::EventSource;
use crate::{CliError, discover_current_project, print_json};

#[derive(Clone, Copy, Debug)]
pub(crate) enum ExportFormat {
    Markdown,
    Json,
}

#[derive(Debug, Serialize)]
struct SessionListReport {
    command: &'static str,
    status: &'static str,
    sessions: Vec<SessionListEntry>,
}

#[derive(Debug, Serialize)]
struct SessionListEntry {
    session_id: String,
    status: &'static str,
    lifecycle: Option<&'static str>,
    agent_id: Option<String>,
    definition_digest: Option<String>,
    domain: Option<&'static str>,
    phase: Option<&'static str>,
    history_items: Option<usize>,
    requests: Option<usize>,
    receipts: Option<u64>,
    compaction_revision: Option<u64>,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct SessionShowReport {
    command: &'static str,
    status: &'static str,
    session_id: String,
    lifecycle: &'static str,
    agent_id: Option<String>,
    definition_digest: Option<String>,
    domain: Option<&'static str>,
    domain_selection_recorded: bool,
    phase: Option<&'static str>,
    phase_selection_recorded: bool,
    history_items: usize,
    requests: usize,
    receipts: u64,
    subcommand_checkpoints: usize,
    cache_hits: u64,
    recoveries: u64,
    vision_routes: usize,
    vision_observations: usize,
    vision_failures: usize,
    compaction_revision: Option<u64>,
}

#[derive(Debug, Serialize)]
struct SessionEndedReport {
    command: &'static str,
    status: &'static str,
    session_id: String,
    lifecycle: &'static str,
}

#[derive(Debug, Serialize)]
struct SessionExport<'state> {
    schema_version: u16,
    session_id: &'state str,
    lifecycle: &'static str,
    agent_binding: Option<&'state crate::domain::agent_definition::AgentDefinitionBinding>,
    domain_profile: Option<&'static str>,
    research_phase: Option<&'static str>,
    authority_boundary: ExportAuthorityBoundary,
    transcript: &'state [ContentItem],
    execution_receipts: Vec<ExecutionReceiptExport<'state>>,
    vision_routes: &'state [crate::domain::vision::VisionRouteRecord],
    vision_observations: &'state [crate::domain::vision::VisionObservation],
    vision_failures: &'state [crate::domain::vision::VisionAnalysisFailure],
    latest_compaction: Option<&'state crate::domain::session::SessionCompaction>,
}

#[derive(Debug, Serialize)]
struct ExportAuthorityBoundary {
    transcript: &'static str,
    receipts: &'static str,
    research_references: &'static str,
    vision: &'static str,
    narrative: &'static str,
}

#[derive(Debug, Serialize)]
struct ExecutionReceiptExport<'receipt> {
    event_id: &'receipt str,
    receipt: &'receipt crate::domain::tools::ExecutionReceipt,
}

pub(crate) fn resolve_resume_target(
    project: &Project,
    requested: Option<&str>,
) -> Result<Option<String>, CliError> {
    let Some(requested) = requested else {
        return Ok(None);
    };
    if requested != "latest" {
        return Ok(Some(requested.to_owned()));
    }

    let mut latest: Option<(String, String)> = None;
    let project_events =
        crate::adapters::project_event_store::read_existing_research_events(project)
            .map_err(|error| session_error(error.to_string()))?;
    for session_id in project.existing_session_ids()? {
        let events = read_existing_session_events(project, &session_id)
            .map_err(|error| session_error(error.to_string()))?;
        let activity = events
            .last()
            .map(|event| activity_sort_key(event.occurred_at.as_str()))
            .ok_or_else(|| CliError::SessionOperation {
                message: format!("Session `{session_id}` has no durable lifecycle Event"),
            })?;
        let state = SessionProjection::replay_project_verified(
            project.project_id().clone(),
            &events,
            &project_events,
            || {
                crate::adapters::research_projection::load_reference_catalog(project).map_err(
                    |message| crate::domain::session::SessionError::TaskContext { message },
                )
            },
            &crate::adapters::artifact_store::LocalArtifactStore::for_project(project),
        )
        .map_err(|error| session_error(error.to_string()))?
        .state()
        .clone();
        if state.lifecycle() == SessionLifecycle::Active {
            let candidate = (activity, session_id.as_str().to_owned());
            if latest.as_ref().is_none_or(|current| candidate > *current) {
                latest = Some(candidate);
            }
        }
    }
    latest
        .map(|(_, session_id)| Some(session_id))
        .ok_or_else(|| CliError::SessionOperation {
            message: "the current project has no strictly replayable Active Session".to_owned(),
        })
}

pub(crate) fn print_list() -> Result<(), CliError> {
    let project = discover_current_project()?;
    let sessions = project
        .existing_session_ids()?
        .into_iter()
        .map(|session_id| match reduce(&project, &session_id) {
            Ok(state) => SessionListEntry {
                session_id: session_id.as_str().to_owned(),
                status: "ok",
                lifecycle: Some(lifecycle(state.lifecycle())),
                agent_id: agent_id(&state),
                definition_digest: definition_digest(&state),
                domain: research_domain(&state),
                phase: research_phase(&state),
                history_items: Some(state.history().items().len()),
                requests: Some(state.request_audits().len()),
                receipts: Some(state.execution_receipt_count()),
                compaction_revision: state
                    .latest_compaction()
                    .map(|value| value.summary().revision()),
                error: None,
            },
            Err(error) => SessionListEntry {
                session_id: session_id.as_str().to_owned(),
                status: "corrupt",
                lifecycle: None,
                agent_id: None,
                definition_digest: None,
                domain: None,
                phase: None,
                history_items: None,
                requests: None,
                receipts: None,
                compaction_revision: None,
                error: Some(error),
            },
        })
        .collect::<Vec<_>>();
    print_json(&SessionListReport {
        command: "session list",
        status: "ok",
        sessions,
    })
}

pub(crate) fn print_show(requested: &str) -> Result<(), CliError> {
    let project = discover_current_project()?;
    let session_id = require_session(&project, requested)?;
    let state = reduce(&project, &session_id).map_err(session_error)?;
    print_json(&SessionShowReport {
        command: "session show",
        status: "ok",
        session_id: session_id.as_str().to_owned(),
        lifecycle: lifecycle(state.lifecycle()),
        agent_id: agent_id(&state),
        definition_digest: definition_digest(&state),
        domain: research_domain(&state),
        domain_selection_recorded: state.domain_profile_selection().is_some(),
        phase: research_phase(&state),
        phase_selection_recorded: state.research_phase_selection().is_some(),
        history_items: state.history().items().len(),
        requests: state.request_audits().len(),
        receipts: state.execution_receipt_count(),
        subcommand_checkpoints: state.subcommand_checkpoints().len(),
        cache_hits: state.cache_hit_count(),
        recoveries: state.recovery_count(),
        vision_routes: state.vision_routes().len(),
        vision_observations: state.vision_observations().len(),
        vision_failures: state.vision_failures().len(),
        compaction_revision: state
            .latest_compaction()
            .map(|value| value.summary().revision()),
    })
}

pub(crate) fn end(requested: &str) -> Result<(), CliError> {
    let project = discover_current_project()?;
    let session_id = require_session(&project, requested)?;
    let _runtime_lease = project.acquire_chat_runtime_lease()?;
    let mut writer = project.open_writer()?;
    let mut store = writer.open_session(session_id.clone(), None)?;
    let stream_id = ProjectEventStream::Session(session_id.clone()).stream_id();
    let events = EventSource::read_events(&store, &stream_id)?;
    let project_events =
        crate::adapters::project_event_store::read_existing_research_events(&project)?;
    let mut projection = SessionProjection::replay_project_verified(
        project.project_id().clone(),
        &events,
        &project_events,
        || {
            crate::adapters::research_projection::load_reference_catalog(&project)
                .map_err(|message| crate::domain::session::SessionError::TaskContext { message })
        },
        &crate::adapters::artifact_store::LocalArtifactStore::for_project(&project),
    )
    .map_err(|error| session_error(error.to_string()))?;
    if projection.state().lifecycle() != SessionLifecycle::Active {
        return Err(CliError::SessionOperation {
            message: format!("Session `{requested}` is not Active"),
        });
    }
    let sequence = store.next_sequence()?;
    let event = SessionEvent::Ended
        .into_envelope(
            EventId::new(format!(
                "session-admin:{}:{}:{}:ended",
                project.project_id(),
                session_id,
                sequence.get()
            )),
            stream_id,
            sequence,
            occurred_at(),
        )
        .map_err(|error| session_error(error.to_string()))?;
    projection
        .append(&mut store, event)
        .map_err(|error| match error {
            crate::app::session::AppendAfterDurableError::Session(error) => {
                session_error(error.to_string())
            }
            crate::app::session::AppendAfterDurableError::Sink(error) => CliError::Project(error),
        })?;
    print_json(&SessionEndedReport {
        command: "session end",
        status: "ended",
        session_id: session_id.as_str().to_owned(),
        lifecycle: "ended",
    })
}

pub(crate) fn export(
    requested: &str,
    format: ExportFormat,
    output: Option<&Path>,
) -> Result<(), CliError> {
    let project = discover_current_project()?;
    let session_id = require_session(&project, requested)?;
    let state = reduce(&project, &session_id).map_err(session_error)?;
    let export = build_export(session_id.as_str(), &state);
    let rendered = match format {
        ExportFormat::Json => serde_json::to_string_pretty(&export)?,
        ExportFormat::Markdown => render_markdown(&export)?,
    };
    if let Some(path) = output {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|error| CliError::SessionOperation {
                message: format!("cannot create export `{}`: {error}", path.display()),
            })?;
        file.write_all(rendered.as_bytes())
            .map_err(|error| CliError::SessionOperation {
                message: format!("cannot write export `{}`: {error}", path.display()),
            })?;
    } else {
        println!("{rendered}");
    }
    Ok(())
}

fn build_export<'state>(
    session_id: &'state str,
    state: &'state SessionState,
) -> SessionExport<'state> {
    SessionExport {
        schema_version: 2,
        session_id,
        lifecycle: lifecycle(state.lifecycle()),
        agent_binding: state.agent_definition_binding(),
        domain_profile: research_domain(state),
        research_phase: research_phase(state),
        authority_boundary: ExportAuthorityBoundary {
            transcript: "durable Session History facts",
            receipts: "authoritative execution Receipt facts with original Event IDs",
            research_references: "typed references only; Research stream remains authoritative",
            vision: "derived model observations; not Finding evidence",
            narrative: "lossy non-authoritative continuation text",
        },
        transcript: state.history().items(),
        execution_receipts: state
            .execution_receipts()
            .iter()
            .map(|record| ExecutionReceiptExport {
                event_id: record.event_id.as_str(),
                receipt: &record.receipt,
            })
            .collect(),
        vision_routes: state.vision_routes(),
        vision_observations: state.vision_observations(),
        vision_failures: state.vision_failures(),
        latest_compaction: state.latest_compaction(),
    }
}

fn render_markdown(export: &SessionExport<'_>) -> Result<String, CliError> {
    let mut output = format!(
        "# Echidna Session {}\n\n- Lifecycle: `{}`\n- Agent: `{}`\n- Domain: `{}`\n- Phase: `{}`\n\n> Research references remain authoritative only in the Research stream. Vision observations and Summary narrative are derived non-evidence.\n\n## Transcript\n",
        export.session_id,
        export.lifecycle,
        export
            .agent_binding
            .map_or("legacy research", |binding| binding
                .definition()
                .id()
                .as_str()),
        export.domain_profile.unwrap_or("disabled"),
        export.research_phase.unwrap_or("disabled")
    );
    for item in export.transcript {
        match item {
            ContentItem::Text { role, text } => {
                let label = match role {
                    MessageRole::User => "User",
                    MessageRole::Assistant => "Assistant",
                };
                output.push_str(&format!("\n### {label}\n\n{}\n", indent(text)));
            }
            ContentItem::ImageArtifact { role, artifact_id } => {
                output.push_str(&format!(
                    "\n### {:?} image\n\n- Artifact reference: `{}`\n",
                    role, artifact_id
                ));
            }
            ContentItem::ToolCall { call } => {
                output.push_str(&format!(
                    "\n### Tool call `{}`\n\n- Call ID: `{}`\n\n{}\n",
                    call.name,
                    call.call_id,
                    indent(&serde_json::to_string_pretty(&call.arguments)?)
                ));
            }
            ContentItem::ToolResult { result } => {
                output.push_str(&format!(
                    "\n### Tool result `{}`\n\n- Call ID: `{}`\n- Status: `{:?}`\n\n{}\n",
                    result.tool_name,
                    result.call_id,
                    result.status,
                    indent(&serde_json::to_string_pretty(&result.output)?)
                ));
            }
            ContentItem::Reasoning { .. } | ContentItem::OpaqueContinuation { .. } => output.push_str(
                "\n### Unsupported transcript item\n\nProvider reasoning is not exported as Conversation History.\n",
            ),
            ContentItem::Unsupported { content_kind } => output.push_str(&format!(
                "\n### Unsupported transcript item\n\n- Kind: `{content_kind}`\n"
            )),
        }
    }
    output.push_str("\n## Execution Receipts\n");
    for record in &export.execution_receipts {
        output.push_str(&format!(
            "\n### `{}`\n\n- Event ID: `{}`\n- Status: `{:?}`\n- Execution ID: `{}`\n",
            record.receipt.execution_id,
            record.event_id,
            record.receipt.status,
            record.receipt.execution_id
        ));
    }
    output.push_str("\n## Vision (derived non-evidence)\n\n");
    output.push_str(&indent(&serde_json::to_string_pretty(
        &serde_json::json!({
            "routes": export.vision_routes,
            "observations": export.vision_observations,
            "failures": export.vision_failures,
        }),
    )?));
    output.push_str("\n\n## Latest Compaction (derived continuation)\n\n");
    output.push_str(&indent(&serde_json::to_string_pretty(
        &export.latest_compaction,
    )?));
    output.push('\n');
    Ok(output)
}

fn agent_id(state: &SessionState) -> Option<String> {
    state
        .agent_definition_binding()
        .map(|binding| binding.definition().id().as_str().to_owned())
}

fn definition_digest(state: &SessionState) -> Option<String> {
    state
        .agent_definition_binding()
        .map(|binding| binding.definition().digest().as_str().to_owned())
}

fn research_enabled(state: &SessionState) -> bool {
    state
        .agent_definition_binding()
        .is_none_or(|binding| binding.definition().research().is_some())
}

fn research_domain(state: &SessionState) -> Option<&'static str> {
    research_enabled(state).then(|| state.effective_domain_profile().id())
}

fn research_phase(state: &SessionState) -> Option<&'static str> {
    research_enabled(state).then(|| state.effective_research_phase().id())
}

fn indent(value: &str) -> String {
    value
        .lines()
        .map(|line| format!("    {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn reduce(project: &Project, session_id: &StreamId) -> Result<SessionState, String> {
    let events =
        read_existing_session_events(project, session_id).map_err(|error| error.to_string())?;
    let project_events =
        crate::adapters::project_event_store::read_existing_research_events(project)
            .map_err(|error| error.to_string())?;
    SessionProjection::replay_project_verified(
        project.project_id().clone(),
        &events,
        &project_events,
        || {
            crate::adapters::research_projection::load_reference_catalog(project)
                .map_err(|message| crate::domain::session::SessionError::TaskContext { message })
        },
        &crate::adapters::artifact_store::LocalArtifactStore::for_project(project),
    )
    .map(|projection| projection.state().clone())
    .map_err(|error| error.to_string())
}

fn require_session(project: &Project, requested: &str) -> Result<StreamId, CliError> {
    project
        .existing_session_ids()?
        .into_iter()
        .find(|session_id| session_id.as_str() == requested)
        .ok_or_else(|| CliError::SessionNotFound {
            session_id: requested.to_owned(),
        })
}

fn session_error(message: String) -> CliError {
    CliError::SessionOperation { message }
}

/// Produces a stable ordering for the runtime's `unix:seconds.nanos` facts.
/// Historical non-Unix values remain deterministic and use Session ID as the
/// final tie-break.
fn activity_sort_key(value: &str) -> String {
    let Some(unix) = value.strip_prefix("unix:") else {
        return format!("0:{value}");
    };
    let Some((seconds, nanos)) = unix.split_once('.') else {
        return format!("0:{value}");
    };
    match (seconds.parse::<u64>(), nanos.parse::<u32>()) {
        (Ok(seconds), Ok(nanos)) if nanos < 1_000_000_000 => {
            format!("1:{seconds:020}.{nanos:09}")
        }
        _ => format!("0:{value}"),
    }
}

fn lifecycle(value: SessionLifecycle) -> &'static str {
    match value {
        SessionLifecycle::Unstarted => "unstarted",
        SessionLifecycle::Active => "active",
        SessionLifecycle::Ended => "ended",
    }
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
