//! Fixed Tool adapter that records one durable Artifact fact.

mod schema;

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::adapters::artifact_store::LocalArtifactStore;
use crate::adapters::project_event_store::{Project, ProjectEventStream};
use crate::adapters::research_projection::load_reference_catalog;
use crate::domain::artifacts::{ArtifactEvent, ArtifactId, ArtifactRecord, BlobId};
use crate::domain::events::{EventEnvelope, EventId, EventType, OccurredAt};
use crate::domain::research::{ResearchEvent, ResearchReducer, TargetSnapshotId};
use crate::domain::tools::{ResourceKey, ToolCallId, ToolExecutionOutcome, ToolExecutionPlan};
use crate::ports::EventSink;
use crate::ports::artifact::{ArtifactStore, StoredBlob};
use crate::ports::tool::ToolExecutorError;

use super::filesystem::resolve_path;
use super::research::reduce_store;
use super::resources::ResourceKeyResolver;
use super::{decode_arguments, invalid_plan};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ArtifactStoreArgs {
    pub(crate) event_id: String,
    pub(crate) occurred_at: String,
    pub(crate) artifact_id: String,
    pub(crate) source_path: String,
    pub(crate) media_type: String,
    pub(crate) target_snapshot_id: String,
    pub(crate) toolchain_fingerprint: String,
    #[serde(default)]
    pub(crate) parent_artifact_ids: Vec<String>,
    #[serde(default)]
    pub(crate) transform: Option<String>,
}

/// One successful Artifact publication through the sole Blob/preflight/Event
/// owner. Tools may format different result JSON but must not reimplement
/// Blob provenance validation or append-before-apply.
pub(crate) struct PublishedArtifact {
    pub(crate) record: ArtifactRecord,
    pub(crate) blob: StoredBlob,
    pub(crate) research_event_count: usize,
}

pub(crate) struct ArtifactPublicationError {
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

/// Verifies that the supplied provenance can become one Artifact fact before
/// an adapter performs an irreversible external action. The final publisher
/// repeats this same validation while it holds the Research writer, so a
/// concurrent update cannot turn a successful external action into an
/// unvalidated Artifact fact.
pub(crate) fn preflight_artifact(
    project_root: &Path,
    plan: &ToolExecutionPlan,
    arguments: &ArtifactStoreArgs,
) -> Result<(), ArtifactPublicationError> {
    let project = Project::discover(project_root)
        .map_err(|error| publication_error("artifact_project_unavailable", error))?;
    let mut writer = project
        .open_writer()
        .map_err(|error| publication_error("artifact_writer_unavailable", error))?;
    let store = writer
        .open_research(None)
        .map_err(|error| publication_error("artifact_stream_unavailable", error))?;
    let reducer = ResearchReducer;
    let references = load_reference_catalog(&project)
        .map_err(|error| publication_error("artifact_receipts_invalid", error))?;
    let state = reduce_store(&store, &reducer, &references)
        .map_err(|error| publication_error("artifact_state_invalid", error))?;
    validate_preflight(&state, &reducer, plan, arguments)
}

/// Resolves an Artifact reference through the authoritative Research replay
/// and asks the one Blob Store owner to revalidate and read its bytes. This
/// is intentionally narrower than a Blob-path API: callers can consume an
/// existing Artifact, but cannot name or bypass a content-addressed file.
pub(crate) fn read_artifact_bytes_bounded(
    project_root: &Path,
    artifact_id: &ArtifactId,
    maximum_bytes: usize,
) -> Result<Vec<u8>, ArtifactPublicationError> {
    let project = Project::discover(project_root)
        .map_err(|error| publication_error("artifact_project_unavailable", error))?;
    let state = super::research::load_state(project.root())
        .map_err(|error| publication_error("artifact_state_invalid", error))?;
    let record = state
        .artifact(artifact_id)
        .ok_or_else(|| ArtifactPublicationError {
            code: "artifact_not_found",
            message: "Artifact reference is not present in the Research stream".to_owned(),
        })?;
    if record.size_bytes > maximum_bytes as u64 {
        return Err(ArtifactPublicationError {
            code: "artifact_blob_too_large",
            message: format!(
                "Artifact Blob size {} exceeds the {maximum_bytes}-byte consumer limit",
                record.size_bytes
            ),
        });
    }
    let bytes = LocalArtifactStore::for_project(&project)
        .read_blob(&record.blob_id)
        .map_err(|error| publication_error("artifact_blob_read_failed", error))?;
    if bytes.len() > maximum_bytes {
        return Err(ArtifactPublicationError {
            code: "artifact_blob_too_large",
            message: format!(
                "Artifact Blob size {} exceeds the {maximum_bytes}-byte consumer limit",
                bytes.len()
            ),
        });
    }
    Ok(bytes)
}

pub(super) fn normalize(
    project_root: &Path,
    control_directory: &str,
    value: Value,
) -> Result<(Value, Vec<ResourceKey>), ToolExecutorError> {
    let arguments: ArtifactStoreArgs = decode_arguments("artifact_store", value)?;
    require_nonempty_arguments(&arguments)?;
    let resolver = ResourceKeyResolver::new(project_root);
    Ok((
        serde_json::to_value(&arguments).expect("ArtifactStoreArgs serialization cannot fail"),
        vec![
            resolver.file(&arguments.source_path),
            resolver.project_file(control_directory, "research.jsonl"),
        ],
    ))
}

pub(super) async fn execute(project_root: &Path, plan: ToolExecutionPlan) -> ToolExecutionOutcome {
    let arguments: ArtifactStoreArgs =
        match decode_arguments("artifact_store", plan.call.arguments.clone()) {
            Ok(arguments) => arguments,
            Err(error) => return invalid_plan(plan, error),
        };
    if let Err(error) = require_nonempty_arguments(&arguments) {
        return invalid_plan(plan, error);
    }

    let source_path = arguments.source_path.clone();
    let publication = publish_artifact(project_root, &plan, &arguments, move |project| {
        let source = resolve_path(project.root(), &source_path);
        LocalArtifactStore::for_project(project).publish_file(&source)
    });
    match publication {
        Ok(publication) => ToolExecutionOutcome::succeeded_with_artifacts(
            plan,
            json!({
                "status": "recorded",
                "event_appended": true,
                "event_type": if publication.record.parent_artifact_ids.is_empty() { "artifact.created" } else { "artifact.derived" },
                "event_id": arguments.event_id,
                "blob": {
                    "blob_id": publication.blob.blob_id,
                    "size_bytes": publication.blob.size_bytes,
                    "already_present": publication.blob.already_present,
                },
                "artifact": publication.record,
                "research_event_count": publication.research_event_count,
            }),
            vec![publication.record.id],
        ),
        Err(error) => artifact_failure(plan, error.code, error.message),
    }
}

/// The only local path allowed to turn immutable Blob bytes into a durable
/// Artifact Research fact. Preflight occurs before byte publication; the
/// Event is appended before the replayed state is promoted.
pub(crate) fn publish_artifact<Publish, PublishError>(
    project_root: &Path,
    plan: &ToolExecutionPlan,
    arguments: &ArtifactStoreArgs,
    publish_blob: Publish,
) -> Result<PublishedArtifact, ArtifactPublicationError>
where
    Publish: FnOnce(&Project) -> Result<StoredBlob, PublishError>,
    PublishError: std::fmt::Display,
{
    let project = Project::discover(project_root)
        .map_err(|error| publication_error("artifact_project_unavailable", error))?;
    let mut writer = project
        .open_writer()
        .map_err(|error| publication_error("artifact_writer_unavailable", error))?;
    let mut store = writer
        .open_research(None)
        .map_err(|error| publication_error("artifact_stream_unavailable", error))?;
    let reducer = ResearchReducer;
    let references = load_reference_catalog(&project)
        .map_err(|error| publication_error("artifact_receipts_invalid", error))?;
    let mut state = reduce_store(&store, &reducer, &references)
        .map_err(|error| publication_error("artifact_state_invalid", error))?;
    validate_preflight(&state, &reducer, plan, arguments)?;
    let blob = publish_blob(&project)
        .map_err(|error| publication_error("artifact_blob_publish_failed", error))?;
    let event = artifact_event(arguments, plan, blob.blob_id.clone(), blob.size_bytes);
    reducer
        .validate_event_with_references(&state, &event, &references)
        .map_err(|error| publication_error("artifact_provenance_rejected", error))?;
    let sequence = store
        .next_sequence()
        .map_err(|error| publication_error("artifact_sequence_unavailable", error))?;
    let event_id = EventId::new(arguments.event_id.clone());
    let payload = event
        .encode_payload()
        .map_err(|error| publication_error("artifact_payload_encode_failed", error))?;
    let envelope = EventEnvelope::current(
        event_id,
        ProjectEventStream::Research.stream_id(),
        sequence,
        OccurredAt::new(arguments.occurred_at.clone()),
        EventType::new(event.event_type().as_str()),
        payload,
    );
    EventSink::append_event(&mut store, &envelope)
        .map_err(|error| publication_error("artifact_append_failed", error))?;
    reducer
        .apply_with_references(&mut state, envelope, &references)
        .map_err(|error| publication_error("artifact_apply_after_append_failed", error))?;
    let ResearchEvent::Artifact(artifact_event) = event else {
        unreachable!("Artifact publisher receives Artifact Events")
    };
    Ok(PublishedArtifact {
        record: artifact_event.record().clone(),
        blob,
        research_event_count: state.applied_event_count(),
    })
}

fn validate_preflight(
    state: &crate::domain::research::ResearchState,
    reducer: &ResearchReducer,
    plan: &ToolExecutionPlan,
    arguments: &ArtifactStoreArgs,
) -> Result<(), ArtifactPublicationError> {
    let event_id = EventId::new(arguments.event_id.clone());
    if state.has_event_id(&event_id) {
        return Err(ArtifactPublicationError {
            code: "artifact_event_id_duplicate",
            message: "event_id already exists in the Research stream; no Blob or Event was created"
                .to_owned(),
        });
    }
    let preflight = artifact_event(arguments, plan, zero_blob_id(), 0);
    reducer
        .validate_event(state, &preflight)
        .map_err(|error| publication_error("artifact_provenance_rejected", error))
}

fn publication_error(code: &'static str, error: impl ToString) -> ArtifactPublicationError {
    ArtifactPublicationError {
        code,
        message: error.to_string(),
    }
}

fn artifact_event(
    arguments: &ArtifactStoreArgs,
    plan: &ToolExecutionPlan,
    blob_id: BlobId,
    size_bytes: u64,
) -> ResearchEvent {
    let record = ArtifactRecord {
        id: ArtifactId::new(arguments.artifact_id.clone()),
        blob_id,
        media_type: arguments.media_type.clone(),
        size_bytes,
        source_path: arguments.source_path.clone(),
        created_by_event_id: EventId::new(arguments.event_id.clone()),
        produced_by_tool_call_id: ToolCallId::new(plan.call.call_id.as_str()),
        target_snapshot_id: TargetSnapshotId::new(arguments.target_snapshot_id.clone()),
        toolchain_fingerprint: arguments.toolchain_fingerprint.clone(),
        parent_artifact_ids: arguments
            .parent_artifact_ids
            .iter()
            .cloned()
            .map(ArtifactId::new)
            .collect(),
        transform: arguments.transform.clone(),
    };
    let event = if record.parent_artifact_ids.is_empty() {
        ArtifactEvent::Created(record)
    } else {
        ArtifactEvent::Derived(record)
    };
    ResearchEvent::Artifact(event)
}

fn zero_blob_id() -> BlobId {
    BlobId::new("0".repeat(64)).expect("zero SHA-256 digest is valid syntax")
}

pub(crate) fn require_nonempty_arguments(
    arguments: &ArtifactStoreArgs,
) -> Result<(), ToolExecutorError> {
    for (name, value) in [
        ("event_id", &arguments.event_id),
        ("occurred_at", &arguments.occurred_at),
        ("artifact_id", &arguments.artifact_id),
        ("source_path", &arguments.source_path),
        ("media_type", &arguments.media_type),
        ("target_snapshot_id", &arguments.target_snapshot_id),
        ("toolchain_fingerprint", &arguments.toolchain_fingerprint),
    ] {
        if value.trim().is_empty() {
            return Err(ToolExecutorError::InvalidArguments {
                tool_name: "artifact_store".to_owned(),
                message: format!("`{name}` must not be empty"),
            });
        }
    }
    Ok(())
}

fn artifact_failure(
    plan: ToolExecutionPlan,
    code: &str,
    error: impl ToString,
) -> ToolExecutionOutcome {
    ToolExecutionOutcome::failed(
        plan,
        code,
        json!({"error_code": code, "error": error.to_string()}),
    )
}

pub(super) fn input_schema() -> Value {
    schema::input_schema()
}

pub(super) fn output_schema() -> Value {
    schema::output_schema()
}
