//! Fixed adapters for reading and appending versioned Research facts.

mod schema;

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::adapters::project_event_store::{
    Project, ProjectEventStream, read_existing_research_events,
};
use crate::domain::events::{EventEnvelope, EventId, EventType, OccurredAt};
use crate::domain::research::{
    ExperimentCapsuleDraft, ExperimentDraft, ExperimentUpdate, FindingDraft, FindingUpdate,
    HypothesisDraft, HypothesisUpdate, NextActionDraft, NextActionUpdate, ResearchError,
    ResearchEvent, ResearchReducer, ResearchState, TargetSnapshotDraft, TargetSnapshotRecord,
};
use crate::domain::tools::{
    ResourceKey, ToolCacheValidation, ToolExecutionOutcome, ToolExecutionPlan,
};
use crate::ports::tool::ToolExecutorError;
use crate::ports::{EventSink, EventSource};

use super::resources::ResourceKeyResolver;
use super::{decode_arguments, invalid_plan};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ResearchGetArgs {}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ResearchUpdateArgs {
    event_id: String,
    occurred_at: String,
    operation: ResearchUpdateOperation,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ResearchUpdateOperation {
    RecordTargetSnapshot {
        record: TargetSnapshotDraft,
    },
    RecordHypothesis {
        record: HypothesisDraft,
        #[serde(default)]
        keep_independent: bool,
    },
    UpdateHypothesis {
        update: HypothesisUpdate,
    },
    RecordExperiment {
        experiment: ExperimentDraft,
        capsule: Box<ExperimentCapsuleDraft>,
        #[serde(default)]
        rerun_reason: Option<String>,
    },
    UpdateExperiment {
        update: ExperimentUpdate,
    },
    RecordFinding {
        record: FindingDraft,
        #[serde(default)]
        keep_independent: bool,
    },
    UpdateFinding {
        update: FindingUpdate,
    },
    RecordNextAction {
        record: NextActionDraft,
    },
    UpdateNextAction {
        update: NextActionUpdate,
    },
    MergeRecords {
        from: crate::domain::research::MergeRecordRef,
        into: crate::domain::research::MergeRecordRef,
        reason: String,
    },
}

pub(super) fn normalize_get(
    project_root: &Path,
    value: Value,
) -> Result<(Value, Vec<ResourceKey>), ToolExecutorError> {
    let arguments: ResearchGetArgs = decode_arguments("research_get", value)?;
    Ok((
        serde_json::to_value(arguments).expect("ResearchGetArgs serialization cannot fail"),
        research_resource(project_root),
    ))
}

pub(super) fn normalize_update(
    project_root: &Path,
    value: Value,
) -> Result<(Value, Vec<ResourceKey>), ToolExecutorError> {
    let arguments: ResearchUpdateArgs = decode_arguments("research_update", value)?;
    if arguments.event_id.trim().is_empty() || arguments.occurred_at.trim().is_empty() {
        return Err(ToolExecutorError::InvalidArguments {
            tool_name: "research_update".to_owned(),
            message: "`event_id` and `occurred_at` must not be empty".to_owned(),
        });
    }
    Ok((
        serde_json::to_value(arguments).expect("ResearchUpdateArgs serialization cannot fail"),
        research_resource(project_root),
    ))
}

pub(super) async fn execute_get(
    project_root: &Path,
    plan: ToolExecutionPlan,
) -> ToolExecutionOutcome {
    let arguments: ResearchGetArgs =
        match decode_arguments("research_get", plan.call.arguments.clone()) {
            Ok(arguments) => arguments,
            Err(error) => return invalid_plan(plan, error),
        };
    let _ = arguments;
    let expected_resource_fingerprint = plan.cache_resource_fingerprint.clone();
    let observed_before_fingerprint = expected_resource_fingerprint
        .as_ref()
        .and_then(|_| cache_resource_fingerprint(project_root, &plan.call.arguments));
    match load_state(project_root) {
        Ok(state) => {
            let stream_event_count = state.applied_event_count();
            let observed_after_fingerprint = expected_resource_fingerprint
                .as_ref()
                .and_then(|_| cache_resource_fingerprint(project_root, &plan.call.arguments));
            let mut outcome = ToolExecutionOutcome::succeeded(
                plan,
                json!({
                    "status": "ok",
                    "state": state,
                    "stream_event_count": stream_event_count,
                }),
            );
            if let (
                Some(cache_resource_fingerprint),
                Some(observed_before_fingerprint),
                Some(observed_after_fingerprint),
            ) = (
                expected_resource_fingerprint,
                observed_before_fingerprint,
                observed_after_fingerprint,
            ) {
                outcome.record_cache_validation(ToolCacheValidation {
                    cache_resource_fingerprint,
                    observed_before_fingerprint,
                    observed_after_fingerprint,
                });
            }
            outcome
        }
        Err(error) => research_failure(plan, "research_read_failed", error),
    }
}

pub(super) async fn execute_update(
    project_root: &Path,
    plan: ToolExecutionPlan,
) -> ToolExecutionOutcome {
    let arguments: ResearchUpdateArgs =
        match decode_arguments("research_update", plan.call.arguments.clone()) {
            Ok(arguments) => arguments,
            Err(error) => return invalid_plan(plan, error),
        };
    let project = match Project::discover(project_root) {
        Ok(project) => project,
        Err(error) => return research_failure(plan, "research_project_unavailable", error),
    };
    let mut writer = match project.open_writer() {
        Ok(writer) => writer,
        Err(error) => return research_failure(plan, "research_writer_unavailable", error),
    };
    let mut store = match writer.open_research(None) {
        Ok(store) => store,
        Err(error) => return research_failure(plan, "research_stream_unavailable", error),
    };
    let reducer = ResearchReducer;
    let mut state = match reduce_store(&store, &reducer) {
        Ok(state) => state,
        Err(error) => return research_failure(plan, "research_state_invalid", error),
    };
    let event_id = EventId::new(arguments.event_id);
    if state.has_event_id(&event_id) {
        return ToolExecutionOutcome::failed(
            plan,
            "research_event_id_duplicate",
            json!({
                "error_code": "research_event_id_duplicate",
                "event_id": event_id,
                "message": "event_id already exists in the Research stream; no Event was appended"
            }),
        );
    }
    let event = match event_from_operation(arguments.operation, &state) {
        Ok(OperationEvent::ExistingTargetSnapshot(record)) => {
            return ToolExecutionOutcome::succeeded(
                plan,
                json!({
                    "status": "existing",
                    "event_appended": false,
                    "record": record,
                    "message": "the semantic TargetSnapshot already exists"
                }),
            );
        }
        Ok(OperationEvent::Append(event)) => event,
        Err(error) => return research_failure(plan, "research_record_invalid", error),
    };
    if let Err(error) = reducer.validate_event(&state, &event) {
        return match error {
            ResearchError::DedupConflict { .. } => {
                ToolExecutionOutcome::succeeded(plan, dedup_conflict_output(&state, &event))
            }
            ResearchError::DuplicateActionFingerprint { .. } => {
                ToolExecutionOutcome::succeeded(plan, action_conflict_output(&state, &event))
            }
            error => research_failure(plan, "research_transition_rejected", error),
        };
    }
    let sequence = match store.next_sequence() {
        Ok(sequence) => sequence,
        Err(error) => return research_failure(plan, "research_sequence_unavailable", error),
    };
    let payload = match event.encode_payload() {
        Ok(payload) => payload,
        Err(error) => return research_failure(plan, "research_payload_encode_failed", error),
    };
    let output_event_id = event_id.clone();
    let envelope = EventEnvelope::current(
        event_id,
        ProjectEventStream::Research.stream_id(),
        sequence,
        OccurredAt::new(arguments.occurred_at),
        EventType::new(event.event_type().as_str()),
        payload,
    );
    if let Err(error) = EventSink::append_event(&mut store, &envelope) {
        return research_failure(plan, "research_append_failed", error);
    }
    if let Err(error) = reducer.apply(&mut state, envelope) {
        return ToolExecutionOutcome::failed(
            plan,
            "research_apply_after_append_failed",
            json!({
                "error_code": "research_apply_after_append_failed",
                "error": error.to_string(),
                "message": "the Event is durable but could not be reduced in memory"
            }),
        );
    }
    ToolExecutionOutcome::succeeded(
        plan,
        json!({
            "status": "recorded",
            "event_appended": true,
            "event_type": event.event_type().as_str(),
            "event_id": output_event_id,
            "research_event_count": state.applied_event_count(),
        }),
    )
}

fn research_resource(project_root: &Path) -> Vec<ResourceKey> {
    vec![ResourceKeyResolver::new(project_root).file(".cyber-agent/research.jsonl")]
}

/// The cache resource for `research_get` is the exact authoritative JSONL
/// byte stream. A hit avoids its strict scan/upcast/reducer work, while a
/// changed stream receives a distinct project-scoped derived key.
pub(super) fn cache_resource_fingerprint(project_root: &Path, arguments: &Value) -> Option<String> {
    let _: ResearchGetArgs = serde_json::from_value(arguments.clone()).ok()?;
    let project = Project::discover(project_root).ok()?;
    let path = project.root().join(".cyber-agent/research.jsonl");
    fs::read(path)
        .ok()
        .map(|bytes| blake3::hash(&bytes).to_hex().to_string())
}

pub(super) fn load_state(project_root: &Path) -> Result<ResearchState, String> {
    let project = Project::discover(project_root).map_err(|error| error.to_string())?;
    reduce_project_research(&project, &ResearchReducer)
}

fn reduce_project_research(
    project: &Project,
    reducer: &ResearchReducer,
) -> Result<ResearchState, String> {
    let events = read_existing_research_events(project).map_err(|error| error.to_string())?;
    reducer.reduce(events).map_err(|error| error.to_string())
}

pub(super) fn reduce_store(
    store: &crate::adapters::project_event_store::JsonlEventStore<'_>,
    reducer: &ResearchReducer,
) -> Result<ResearchState, String> {
    let events = EventSource::read_events(store, &ProjectEventStream::Research.stream_id())
        .map_err(|error| error.to_string())?;
    reducer.reduce(events).map_err(|error| error.to_string())
}

enum OperationEvent {
    ExistingTargetSnapshot(Box<TargetSnapshotRecord>),
    Append(Box<ResearchEvent>),
}

fn event_from_operation(
    operation: ResearchUpdateOperation,
    state: &ResearchState,
) -> Result<OperationEvent, ResearchError> {
    let event = match operation {
        ResearchUpdateOperation::RecordTargetSnapshot { record } => {
            let record = TargetSnapshotRecord::from_draft(record)?;
            if let Some(existing) =
                state.target_snapshot_by_fingerprint(&record.snapshot_fingerprint)
            {
                return Ok(OperationEvent::ExistingTargetSnapshot(Box::new(
                    existing.clone(),
                )));
            }
            ResearchEvent::TargetSnapshotRecorded(record)
        }
        ResearchUpdateOperation::RecordHypothesis {
            record,
            keep_independent,
        } => ResearchEvent::HypothesisRecorded {
            record: crate::domain::research::HypothesisRecord::from_draft(record)?,
            keep_independent,
        },
        ResearchUpdateOperation::UpdateHypothesis { update } => {
            ResearchEvent::HypothesisUpdated(update)
        }
        ResearchUpdateOperation::RecordExperiment {
            experiment,
            capsule,
            rerun_reason,
        } => {
            let (experiment, capsule) =
                crate::domain::research::ExperimentRecord::from_drafts(experiment, *capsule)?;
            ResearchEvent::ExperimentRecorded {
                experiment,
                capsule,
                rerun_reason,
            }
        }
        ResearchUpdateOperation::UpdateExperiment { update } => {
            ResearchEvent::ExperimentUpdated(update)
        }
        ResearchUpdateOperation::RecordFinding {
            record,
            keep_independent,
        } => ResearchEvent::FindingRecorded {
            record: crate::domain::research::FindingRecord::from_draft(record)?,
            keep_independent,
        },
        ResearchUpdateOperation::UpdateFinding { update } => ResearchEvent::FindingUpdated(update),
        ResearchUpdateOperation::RecordNextAction { record } => ResearchEvent::NextActionRecorded(
            crate::domain::research::NextActionRecord::from_draft(record)?,
        ),
        ResearchUpdateOperation::UpdateNextAction { update } => {
            ResearchEvent::NextActionUpdated(update)
        }
        ResearchUpdateOperation::MergeRecords { from, into, reason } => {
            ResearchEvent::RecordsMerged { from, into, reason }
        }
    };
    Ok(OperationEvent::Append(Box::new(event)))
}

fn dedup_conflict_output(state: &ResearchState, event: &ResearchEvent) -> Value {
    match event {
        ResearchEvent::HypothesisRecorded { record, .. } => json!({
            "status": "dedup_conflict",
            "event_appended": false,
            "dedup_key": record.dedup_key,
            "existing_records": state.hypotheses_by_dedup_key(&record.dedup_key),
            "required_resolution": ["keep_independent", "records_merged"],
        }),
        ResearchEvent::FindingRecorded { record, .. } => json!({
            "status": "dedup_conflict",
            "event_appended": false,
            "dedup_key": record.dedup_key,
            "existing_records": state.findings_by_dedup_key(&record.dedup_key),
            "required_resolution": ["keep_independent", "records_merged"],
        }),
        _ => json!({"status": "transition_rejected", "event_appended": false}),
    }
}

fn action_conflict_output(state: &ResearchState, event: &ResearchEvent) -> Value {
    let ResearchEvent::ExperimentRecorded { experiment, .. } = event else {
        return json!({"status": "transition_rejected", "event_appended": false});
    };
    let existing_records = state.experiments_by_action_fingerprint(&experiment.action_fingerprint);
    json!({
        "status": "action_conflict",
        "event_appended": false,
        "action_fingerprint": experiment.action_fingerprint,
        "existing_records": existing_records,
        "required_resolution": ["rerun_reason"],
    })
}

fn research_failure(
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

pub(super) fn get_input_schema() -> Value {
    schema::get_input_schema()
}

pub(super) fn update_input_schema() -> Value {
    schema::update_input_schema()
}

pub(super) fn get_output_schema() -> Value {
    schema::get_output_schema()
}

pub(super) fn update_output_schema() -> Value {
    schema::update_output_schema()
}
