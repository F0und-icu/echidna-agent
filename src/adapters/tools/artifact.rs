//! Fixed Tool adapter that records one durable Artifact fact.

mod schema;

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::adapters::artifact_store::LocalArtifactStore;
use crate::adapters::project_event_store::{Project, ProjectEventStream};
use crate::domain::artifacts::{ArtifactEvent, ArtifactId, ArtifactRecord, BlobId};
use crate::domain::events::{EventEnvelope, EventId, EventType, OccurredAt};
use crate::domain::research::{ResearchEvent, ResearchReducer, TargetSnapshotId};
use crate::domain::tools::{ResourceKey, ToolCallId, ToolExecutionOutcome, ToolExecutionPlan};
use crate::ports::EventSink;
use crate::ports::artifact::ArtifactStore;
use crate::ports::tool::ToolExecutorError;

use super::filesystem::resolve_path;
use super::research::reduce_store;
use super::resources::ResourceKeyResolver;
use super::{decode_arguments, invalid_plan};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ArtifactStoreArgs {
    event_id: String,
    occurred_at: String,
    artifact_id: String,
    source_path: String,
    media_type: String,
    target_snapshot_id: String,
    toolchain_fingerprint: String,
    #[serde(default)]
    parent_artifact_ids: Vec<String>,
    #[serde(default)]
    transform: Option<String>,
}

pub(super) fn normalize(
    project_root: &Path,
    value: Value,
) -> Result<(Value, Vec<ResourceKey>), ToolExecutorError> {
    let arguments: ArtifactStoreArgs = decode_arguments("artifact_store", value)?;
    require_nonempty_arguments(&arguments)?;
    let resolver = ResourceKeyResolver::new(project_root);
    Ok((
        serde_json::to_value(&arguments).expect("ArtifactStoreArgs serialization cannot fail"),
        vec![
            resolver.file(&arguments.source_path),
            resolver.file(".cyber-agent/research.jsonl"),
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

    let project = match Project::discover(project_root) {
        Ok(project) => project,
        Err(error) => return artifact_failure(plan, "artifact_project_unavailable", error),
    };
    let mut writer = match project.open_writer() {
        Ok(writer) => writer,
        Err(error) => return artifact_failure(plan, "artifact_writer_unavailable", error),
    };
    let mut store = match writer.open_research(None) {
        Ok(store) => store,
        Err(error) => return artifact_failure(plan, "artifact_stream_unavailable", error),
    };
    let reducer = ResearchReducer;
    let mut state = match reduce_store(&store, &reducer) {
        Ok(state) => state,
        Err(error) => return artifact_failure(plan, "artifact_state_invalid", error),
    };
    let event_id = EventId::new(arguments.event_id.clone());
    if state.has_event_id(&event_id) {
        return ToolExecutionOutcome::failed(
            plan,
            "artifact_event_id_duplicate",
            json!({
                "error_code": "artifact_event_id_duplicate",
                "event_id": event_id,
                "message": "event_id already exists in the Research stream; no Blob or Event was created"
            }),
        );
    }

    let source = resolve_path(project.root(), &arguments.source_path);
    let preflight = artifact_event(&arguments, &plan, zero_blob_id(), 0);
    if let Err(error) = reducer.validate_event(&state, &preflight) {
        return artifact_failure(plan, "artifact_provenance_rejected", error);
    }

    let stored_blob = match LocalArtifactStore::for_project(&project).publish_file(&source) {
        Ok(blob) => blob,
        Err(error) => return artifact_failure(plan, "artifact_blob_publish_failed", error),
    };
    let event = artifact_event(
        &arguments,
        &plan,
        stored_blob.blob_id.clone(),
        stored_blob.size_bytes,
    );
    if let Err(error) = reducer.validate_event(&state, &event) {
        return artifact_failure(plan, "artifact_provenance_rejected", error);
    }
    let sequence = match store.next_sequence() {
        Ok(sequence) => sequence,
        Err(error) => return artifact_failure(plan, "artifact_sequence_unavailable", error),
    };
    let payload = match event.encode_payload() {
        Ok(payload) => payload,
        Err(error) => return artifact_failure(plan, "artifact_payload_encode_failed", error),
    };
    let envelope = EventEnvelope::current(
        event_id.clone(),
        ProjectEventStream::Research.stream_id(),
        sequence,
        OccurredAt::new(arguments.occurred_at),
        EventType::new(event.event_type().as_str()),
        payload,
    );
    if let Err(error) = EventSink::append_event(&mut store, &envelope) {
        return artifact_failure(plan, "artifact_append_failed", error);
    }
    if let Err(error) = reducer.apply(&mut state, envelope) {
        return ToolExecutionOutcome::failed(
            plan,
            "artifact_apply_after_append_failed",
            json!({
                "error_code": "artifact_apply_after_append_failed",
                "error": error.to_string(),
                "message": "the Artifact Event is durable but could not be reduced in memory"
            }),
        );
    }

    let ResearchEvent::Artifact(artifact_event) = &event else {
        unreachable!("artifact_store only builds Artifact Events")
    };
    let output_artifact_id = artifact_event.record().id.clone();
    ToolExecutionOutcome::succeeded_with_artifacts(
        plan,
        json!({
            "status": "recorded",
            "event_appended": true,
            "event_type": event.event_type().as_str(),
            "event_id": event_id,
            "blob": {
                "blob_id": stored_blob.blob_id,
                "size_bytes": stored_blob.size_bytes,
                "already_present": stored_blob.already_present,
            },
            "artifact": artifact_event.record(),
            "research_event_count": state.applied_event_count(),
        }),
        vec![output_artifact_id],
    )
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

fn require_nonempty_arguments(arguments: &ArtifactStoreArgs) -> Result<(), ToolExecutorError> {
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
