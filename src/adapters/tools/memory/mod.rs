//! Fixed project-Memory Tools with audited Markdown revisions.

use super::ToolRegistry;
use crate::domain::tools::{
    ToolDeterminism, ToolEffect, ToolExposure, ToolParallelism, ToolRetryPolicy, ToolSpec,
};

mod schema;

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::adapters::artifact_store::LocalArtifactStore;
use crate::adapters::project_event_store::{Project, ProjectEventStream};
use crate::adapters::project_memory::{
    LocalProjectMemory, MemoryDocument, content_hash, replacement_diff,
};
use crate::adapters::research_projection::load_reference_catalog;
use crate::domain::events::{EventEnvelope, EventId, EventType, OccurredAt};
use crate::domain::memory::{
    MemoryEvent, MemoryPath, MemoryReducer, MemoryRevision, MemoryScope, MemoryState,
    validate_content_hash,
};
use crate::domain::research::{
    ResearchEvent, ResearchReducer, ResearchReferenceCatalog, ResearchState,
};
use crate::domain::tools::{ResourceKey, ToolExecutionOutcome, ToolExecutionPlan};
use crate::ports::artifact::ArtifactStore;
use crate::ports::tool::ToolExecutorError;
use crate::ports::{EventSink, EventSource};

use super::resources::ResourceKeyResolver;
use super::{decode_arguments, invalid_plan};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct MemoryReadArgs {
    path: String,
    event_id: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct MemoryWriteArgs {
    path: String,
    content: String,
    expected_hash: Option<String>,
    event_id: String,
}

fn normalize_read(
    project_root: &Path,
    control_directory: &str,
    scope: &MemoryScope,
    value: Value,
) -> Result<(Value, Vec<ResourceKey>), ToolExecutorError> {
    let arguments: MemoryReadArgs = decode_arguments("memory_read", value)?;
    let path = parse_path("memory_read", &arguments.path)?;
    require_event_id("memory_read", &arguments.event_id)?;
    Ok((
        serde_json::to_value(&arguments).expect("MemoryReadArgs serialization cannot fail"),
        memory_resources(project_root, control_directory, scope, &path),
    ))
}

fn normalize_write(
    project_root: &Path,
    control_directory: &str,
    scope: &MemoryScope,
    value: Value,
) -> Result<(Value, Vec<ResourceKey>), ToolExecutorError> {
    let arguments: MemoryWriteArgs = decode_arguments("memory_write", value)?;
    let path = parse_path("memory_write", &arguments.path)?;
    require_event_id("memory_write", &arguments.event_id)?;
    validate_expected_hash("memory_write", arguments.expected_hash.as_deref())?;
    Ok((
        serde_json::to_value(&arguments).expect("MemoryWriteArgs serialization cannot fail"),
        memory_resources(project_root, control_directory, scope, &path),
    ))
}

async fn execute_read(
    project_root: &Path,
    scope: &MemoryScope,
    plan: ToolExecutionPlan,
    core_byte_limit: usize,
) -> ToolExecutionOutcome {
    let arguments: MemoryReadArgs =
        match decode_arguments("memory_read", plan.call.arguments.clone()) {
            Ok(arguments) => arguments,
            Err(error) => return invalid_plan(plan, error),
        };
    let path = match parse_path("memory_read", &arguments.path) {
        Ok(path) => path,
        Err(error) => return invalid_plan(plan, error),
    };
    if let Err(error) = require_event_id("memory_read", &arguments.event_id) {
        return invalid_plan(plan, error);
    }
    let occurred_at = match super::operation_time::current_utc() {
        Ok(occurred_at) => occurred_at,
        Err(error) => return memory_failure(plan, "memory_clock_unavailable", error),
    };
    let project = match Project::discover(project_root) {
        Ok(project) => project,
        Err(error) => return memory_failure(plan, "memory_project_unavailable", error),
    };
    let mut writer = match project.open_writer() {
        Ok(writer) => writer,
        Err(error) => return memory_failure(plan, "memory_writer_unavailable", error),
    };
    let mut store = match writer.open_research(None) {
        Ok(store) => store,
        Err(error) => return memory_failure(plan, "memory_stream_unavailable", error),
    };
    let references = match load_reference_catalog(&project) {
        Ok(references) => references,
        Err(error) => return memory_failure(plan, "memory_receipts_invalid", error),
    };
    let (mut research_state, mut memory_state) = match reduce_states(&store, &references) {
        Ok(states) => states,
        Err(error) => return memory_failure(plan, "memory_state_invalid", error),
    };
    let memory = LocalProjectMemory::with_scope(&project, scope.clone());
    let document = match memory.read(&path) {
        Ok(document) => document,
        Err(error) => return memory_failure(plan, "memory_read_failed", error),
    };
    if let Some(content) = &document.content
        && let Err(error) = validate_core_tool_limit(&path, content, core_byte_limit)
    {
        return memory_failure(plan, "memory_read_failed", error);
    }
    let external_event = match external_revision(&path, scope, &document, &memory_state, &project) {
        Ok(event) => event,
        Err(error) => return memory_failure(plan, "memory_revision_unavailable", error),
    };
    let mut external_detected = false;
    if let Some(event) = external_event {
        let event_id = EventId::new(arguments.event_id.clone());
        if research_state.has_event_id(&event_id) {
            return duplicate_event(plan, event_id);
        }
        if let Err(error) = append_memory_event(
            &mut store,
            &mut research_state,
            &mut memory_state,
            event_id,
            &occurred_at,
            event,
        ) {
            return memory_failure(plan, "memory_external_audit_failed", error);
        }
        external_detected = true;
    }
    ToolExecutionOutcome::succeeded(
        plan,
        json!({
            "status": "ok",
            "path": path.as_str(),
            "content": document.content,
            "hash": document.hash,
            "external_change_detected": external_detected,
            "stream_event_count": research_state.applied_event_count(),
        }),
    )
}

async fn execute_write(
    project_root: &Path,
    scope: &MemoryScope,
    plan: ToolExecutionPlan,
    core_byte_limit: usize,
) -> ToolExecutionOutcome {
    let arguments: MemoryWriteArgs =
        match decode_arguments("memory_write", plan.call.arguments.clone()) {
            Ok(arguments) => arguments,
            Err(error) => return invalid_plan(plan, error),
        };
    let path = match parse_path("memory_write", &arguments.path) {
        Ok(path) => path,
        Err(error) => return invalid_plan(plan, error),
    };
    if let Err(error) = require_event_id("memory_write", &arguments.event_id)
        .and_then(|()| validate_expected_hash("memory_write", arguments.expected_hash.as_deref()))
    {
        return invalid_plan(plan, error);
    }
    if let Err(error) = validate_core_tool_limit(&path, &arguments.content, core_byte_limit) {
        return memory_failure(plan, "memory_content_too_large", error);
    }
    let occurred_at = match super::operation_time::current_utc() {
        Ok(occurred_at) => occurred_at,
        Err(error) => return memory_failure(plan, "memory_clock_unavailable", error),
    };
    let project = match Project::discover(project_root) {
        Ok(project) => project,
        Err(error) => return memory_failure(plan, "memory_project_unavailable", error),
    };
    let mut writer = match project.open_writer() {
        Ok(writer) => writer,
        Err(error) => return memory_failure(plan, "memory_writer_unavailable", error),
    };
    let mut store = match writer.open_research(None) {
        Ok(store) => store,
        Err(error) => return memory_failure(plan, "memory_stream_unavailable", error),
    };
    let references = match load_reference_catalog(&project) {
        Ok(references) => references,
        Err(error) => return memory_failure(plan, "memory_receipts_invalid", error),
    };
    let (mut research_state, mut memory_state) = match reduce_states(&store, &references) {
        Ok(states) => states,
        Err(error) => return memory_failure(plan, "memory_state_invalid", error),
    };
    let event_id = EventId::new(arguments.event_id.clone());
    if research_state.has_event_id(&event_id) {
        return duplicate_event(plan, event_id);
    }
    let memory = LocalProjectMemory::with_scope(&project, scope.clone());
    if let Err(error) = memory.validate_content(&path, &arguments.content) {
        return memory_failure(plan, "memory_content_too_large", error);
    }
    let current = match memory.read(&path) {
        Ok(document) => document,
        Err(error) => return memory_failure(plan, "memory_read_failed", error),
    };
    let pending_external = match external_revision(&path, scope, &current, &memory_state, &project)
    {
        Ok(event) => event,
        Err(error) => return memory_failure(plan, "memory_revision_unavailable", error),
    };
    if let Some(event) = pending_external {
        let external_id = EventId::new(format!("{}:external", arguments.event_id));
        if research_state.has_event_id(&external_id) {
            return duplicate_event(plan, external_id);
        }
        if let Err(error) = append_memory_event(
            &mut store,
            &mut research_state,
            &mut memory_state,
            external_id,
            &occurred_at,
            event,
        ) {
            return memory_failure(plan, "memory_external_audit_failed", error);
        }
    }
    if current.hash != arguments.expected_hash {
        return ToolExecutionOutcome::failed(
            plan,
            "memory_expected_hash_mismatch",
            json!({
                "error_code": "memory_expected_hash_mismatch",
                "path": path.as_str(),
                "expected_hash": arguments.expected_hash,
                "current_hash": current.hash,
                "message": "Markdown changed after the caller read it; no replacement was made"
            }),
        );
    }
    if current.content.as_deref() == Some(arguments.content.as_str()) {
        return ToolExecutionOutcome::succeeded(
            plan,
            json!({
                "status": "unchanged",
                "event_appended": false,
                "path": path.as_str(),
                "hash": current.hash,
                "stream_event_count": research_state.applied_event_count(),
            }),
        );
    }
    let revision =
        match revision_from_documents(scope, &path, &current, Some(&arguments.content), &project) {
            Ok(revision) => revision,
            Err(error) => return memory_failure(plan, "memory_revision_unavailable", error),
        };
    if let Err(error) = memory.replace(&path, &arguments.content) {
        return memory_failure(plan, "memory_replace_failed", error);
    }
    if let Err(error) = append_memory_event(
        &mut store,
        &mut research_state,
        &mut memory_state,
        event_id.clone(),
        &occurred_at,
        MemoryEvent::Changed(revision.clone()),
    ) {
        return ToolExecutionOutcome::failed(
            plan,
            "memory_append_after_replace_failed",
            json!({
                "error_code": "memory_append_after_replace_failed",
                "event_id": event_id,
                "error": error.to_string(),
                "message": "Markdown was replaced; the next Memory read will audit this as an external change"
            }),
        );
    }
    ToolExecutionOutcome::succeeded(
        plan,
        json!({
            "status": "recorded",
            "event_appended": true,
            "event_type": "memory.changed",
            "event_id": event_id,
            "path": path.as_str(),
            "old_hash": revision.old_hash,
            "new_hash": revision.new_hash,
            "diff_blob_id": revision.diff_blob_id,
            "stream_event_count": research_state.applied_event_count(),
        }),
    )
}

fn memory_resources(
    project_root: &Path,
    control_directory: &str,
    scope: &MemoryScope,
    path: &MemoryPath,
) -> Vec<ResourceKey> {
    let resolver = ResourceKeyResolver::new(project_root);
    vec![
        resolver.project_file(
            control_directory,
            &format!("{}/{}", scope.storage_directory(), path.as_str()),
        ),
        resolver.project_file(control_directory, "research.jsonl"),
    ]
}

fn parse_path(tool_name: &str, value: &str) -> Result<MemoryPath, ToolExecutorError> {
    MemoryPath::parse(value.to_owned()).map_err(|error| ToolExecutorError::InvalidArguments {
        tool_name: tool_name.to_owned(),
        message: error.to_string(),
    })
}

fn require_event_id(tool_name: &str, event_id: &str) -> Result<(), ToolExecutorError> {
    if event_id.trim().is_empty() {
        Err(ToolExecutorError::InvalidArguments {
            tool_name: tool_name.to_owned(),
            message: "`event_id` must not be empty".to_owned(),
        })
    } else {
        Ok(())
    }
}

fn validate_expected_hash(tool_name: &str, value: Option<&str>) -> Result<(), ToolExecutorError> {
    validate_content_hash("expected_hash", value).map_err(|error| {
        ToolExecutorError::InvalidArguments {
            tool_name: tool_name.to_owned(),
            message: error.to_string(),
        }
    })
}

fn reduce_states(
    store: &crate::adapters::project_event_store::JsonlEventStore<'_>,
    references: &ResearchReferenceCatalog,
) -> Result<(ResearchState, MemoryState), String> {
    let events = EventSource::read_events(store, &ProjectEventStream::Research.stream_id())
        .map_err(|error| error.to_string())?;
    let mut research_state = ResearchState::default();
    let mut memory_state = MemoryState::default();
    let research_reducer = ResearchReducer;
    let memory_reducer = MemoryReducer;
    for envelope in events {
        let already_applied = research_state.has_event_id(&envelope.event_id);
        if envelope.event_type.as_str() == "StreamRecovered" {
            research_reducer
                .apply_with_references(&mut research_state, envelope, references)
                .map_err(|error| error.to_string())?;
            continue;
        }
        let event = ResearchEvent::decode(envelope.event_type.as_str(), envelope.payload.clone())
            .map_err(|error| error.to_string())?;
        research_reducer
            .apply_with_references(&mut research_state, envelope, references)
            .map_err(|error| error.to_string())?;
        if !already_applied && let ResearchEvent::Memory(event) = event {
            memory_reducer
                .apply(&mut memory_state, event)
                .map_err(|error| error.to_string())?;
        }
    }
    Ok((research_state, memory_state))
}

fn external_revision(
    path: &MemoryPath,
    scope: &MemoryScope,
    current: &MemoryDocument,
    state: &MemoryState,
    project: &Project,
) -> Result<Option<MemoryEvent>, String> {
    let Some(previous) = state.latest(scope, path) else {
        return if current.hash.is_some() {
            revision_from_contents(scope, path, None, current.content.as_deref(), project)
                .map(MemoryEvent::ExternalChangeDetected)
                .map(Some)
        } else {
            Ok(None)
        };
    };
    if previous.new_hash == current.hash {
        return Ok(None);
    }
    let old_content = match &previous.new_blob_id {
        Some(blob_id) => Some(
            String::from_utf8(
                LocalArtifactStore::for_project(project)
                    .read_blob(blob_id)
                    .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?,
        ),
        None => None,
    };
    revision_from_contents(
        scope,
        path,
        old_content.as_deref(),
        current.content.as_deref(),
        project,
    )
    .map(MemoryEvent::ExternalChangeDetected)
    .map(Some)
}

fn revision_from_documents(
    scope: &MemoryScope,
    path: &MemoryPath,
    old: &MemoryDocument,
    new: Option<&str>,
    project: &Project,
) -> Result<MemoryRevision, String> {
    revision_from_contents(scope, path, old.content.as_deref(), new, project)
}

fn revision_from_contents(
    scope: &MemoryScope,
    path: &MemoryPath,
    old: Option<&str>,
    new: Option<&str>,
    project: &Project,
) -> Result<MemoryRevision, String> {
    let old_hash = old.map(|value| content_hash(value.as_bytes()));
    let new_hash = new.map(|value| content_hash(value.as_bytes()));
    let blobs = LocalArtifactStore::for_project(project);
    let old_blob_id = old
        .map(|value| {
            blobs
                .publish_bytes(value.as_bytes())
                .map(|stored| stored.blob_id)
        })
        .transpose()
        .map_err(|error| error.to_string())?;
    let new_blob_id = new
        .map(|value| {
            blobs
                .publish_bytes(value.as_bytes())
                .map(|stored| stored.blob_id)
        })
        .transpose()
        .map_err(|error| error.to_string())?;
    let diff = replacement_diff(old, new);
    let diff_blob_id = blobs
        .publish_bytes(diff.as_bytes())
        .map_err(|error| error.to_string())?
        .blob_id;
    let revision = MemoryRevision {
        scope: scope.clone(),
        path: path.clone(),
        old_hash,
        new_hash,
        old_blob_id,
        new_blob_id,
        diff_blob_id: Some(diff_blob_id),
    };
    revision.validate().map_err(|error| error.to_string())?;
    Ok(revision)
}

fn append_memory_event(
    store: &mut crate::adapters::project_event_store::JsonlEventStore<'_>,
    research_state: &mut ResearchState,
    memory_state: &mut MemoryState,
    event_id: EventId,
    occurred_at: &str,
    event: MemoryEvent,
) -> Result<(), String> {
    let research_event = ResearchEvent::Memory(event.clone());
    let research_reducer = ResearchReducer;
    research_reducer
        .validate_event(research_state, &research_event)
        .map_err(|error| error.to_string())?;
    // Validate the Memory-specific revision chain before the durable append.
    // The Research reducer intentionally does not own this projection, so
    // applying it only after append could otherwise persist an invalid
    // revision that is discovered too late.
    let mut next_memory_state = memory_state.clone();
    MemoryReducer
        .apply(&mut next_memory_state, event.clone())
        .map_err(|error| error.to_string())?;
    let sequence = store.next_sequence().map_err(|error| error.to_string())?;
    let payload = research_event
        .encode_payload()
        .map_err(|error| error.to_string())?;
    let envelope = EventEnvelope::current(
        event_id,
        ProjectEventStream::Research.stream_id(),
        sequence,
        OccurredAt::new(occurred_at.to_owned()),
        EventType::new(research_event.event_type().as_str()),
        payload,
    );
    EventSink::append_event(store, &envelope).map_err(|error| error.to_string())?;
    research_reducer
        .apply(research_state, envelope)
        .map_err(|error| error.to_string())?;
    *memory_state = next_memory_state;
    Ok(())
}

fn duplicate_event(plan: ToolExecutionPlan, event_id: EventId) -> ToolExecutionOutcome {
    ToolExecutionOutcome::failed(
        plan,
        "memory_event_id_duplicate",
        json!({
            "error_code": "memory_event_id_duplicate",
            "event_id": event_id,
            "message": "event_id already exists in the Research stream; no Markdown or Event was changed"
        }),
    )
}

fn memory_failure(
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

fn validate_core_tool_limit(path: &MemoryPath, content: &str, limit: usize) -> Result<(), String> {
    if path.as_str() == "core.md" && content.len() > limit {
        Err(format!(
            "core.md has {} UTF-8 bytes; active Tool limit is {limit}",
            content.len()
        ))
    } else {
        Ok(())
    }
}

fn read_input_schema() -> Value {
    schema::read_input_schema()
}

fn write_input_schema(core_byte_limit: usize) -> Value {
    schema::write_input_schema(core_byte_limit)
}

fn output_schema() -> Value {
    schema::output_schema()
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum Tool {
    MemoryRead,
    MemoryWrite,
}

impl Tool {
    pub(super) const ALL: [Self; 2] = [Self::MemoryRead, Self::MemoryWrite];

    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::MemoryRead => "memory_read",
            Self::MemoryWrite => "memory_write",
        }
    }

    pub(super) fn spec(self, registry: &ToolRegistry) -> ToolSpec {
        match self {
            Self::MemoryRead => super::spec(
                self.name(),
                "2",
                super::description(
                    "bounded Memory in the Agent's configured scope",
                    "a mutation or verified Research fact is needed.",
                    "the smallest missing document; reuse current delivered Core Memory instead of rereading it routinely.",
                    "path, content and hash; content:null and hash:null mean the document is absent in the bound Memory namespace.",
                    false,
                ),
                ToolExposure::Deferred,
                read_input_schema(),
                output_schema(),
                ToolParallelism::ResourceLocked,
                Some("file:.echidna/memory/{path},file:.echidna/research.jsonl"),
                ToolEffect::LocalMutation,
                ToolRetryPolicy::NeverAutoRetry,
                ToolDeterminism::EnvironmentDependent,
                10_000,
            ),
            Self::MemoryWrite => super::spec(
                self.name(),
                "2",
                super::description(
                    "a focused Memory update in the Agent's configured scope",
                    "content is transient, or an existing document lacks its current hash.",
                    "a focused update; use expected_hash:null only to create a missing document, otherwise read the current hash first.",
                    "hashes, Blob and status.",
                    false,
                ),
                ToolExposure::Deferred,
                write_input_schema(registry.window_policy.core_memory_bytes()),
                output_schema(),
                ToolParallelism::ResourceLocked,
                Some("file:.echidna/memory/{path},file:.echidna/research.jsonl"),
                ToolEffect::LocalMutation,
                ToolRetryPolicy::NeverAutoRetry,
                ToolDeterminism::EnvironmentDependent,
                30_000,
            ),
        }
    }

    pub(super) fn normalize(
        self,
        registry: &ToolRegistry,
        arguments: Value,
    ) -> Result<(Value, Vec<ResourceKey>), ToolExecutorError> {
        let project_root = &registry.project_root;
        let control = registry.control_directory_name;
        let memory_scope = registry.memory_scope.as_ref();
        match self {
            Self::MemoryRead => normalize_read(
                project_root,
                control,
                memory_scope.expect("Memory Tool is registered only with a scope"),
                arguments,
            ),
            Self::MemoryWrite => normalize_write(
                project_root,
                control,
                memory_scope.expect("Memory Tool is registered only with a scope"),
                arguments,
            ),
        }
    }

    pub(super) async fn execute(
        self,
        registry: &ToolRegistry,
        plan: ToolExecutionPlan,
    ) -> ToolExecutionOutcome {
        match self {
            Self::MemoryRead => {
                execute_read(
                    &registry.project_root,
                    registry
                        .memory_scope
                        .as_ref()
                        .expect("Memory Tool is registered only with a scope"),
                    plan,
                    registry.window_policy.core_memory_bytes(),
                )
                .await
            }
            Self::MemoryWrite => {
                execute_write(
                    &registry.project_root,
                    registry
                        .memory_scope
                        .as_ref()
                        .expect("Memory Tool is registered only with a scope"),
                    plan,
                    registry.window_policy.core_memory_bytes(),
                )
                .await
            }
        }
    }
}
