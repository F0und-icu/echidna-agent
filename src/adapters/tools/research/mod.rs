//! Fixed adapters for reading and appending versioned Research facts.

use super::ToolRegistry;
use crate::domain::tools::{
    ToolDeterminism, ToolEffect, ToolExposure, ToolParallelism, ToolRetryPolicy, ToolSpec,
};

mod schema;

use std::path::Path;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};

use crate::adapters::project_event_store::{Project, ProjectEventStream};
use crate::adapters::research_projection::{load_reference_catalog, load_research_state};
use crate::domain::events::{EventEnvelope, EventId, EventType, OccurredAt};
use crate::domain::research::{
    ExperimentCapsuleDraft, ExperimentDraft, ExperimentUpdate, FindingDraft, FindingUpdate,
    HypothesisDraft, HypothesisUpdate, NextActionDraft, NextActionUpdate, ResearchError,
    ResearchEvent, ResearchReducer, ResearchReferenceCatalog, ResearchState, TargetSnapshotDraft,
    TargetSnapshotRecord,
};
use crate::domain::tools::{ResourceKey, ToolExecutionOutcome, ToolExecutionPlan};
use crate::ports::tool::ToolExecutorError;
use crate::ports::{EventSink, EventSource};

use super::resources::ResourceKeyResolver;
use super::{decode_arguments, invalid_plan};

const DEFAULT_RECEIPT_PAGE_LIMIT: usize = 16;
const MAX_RECEIPT_PAGE_LIMIT: usize = 32;
const RESEARCH_REFERENCE_HINT: &str = "Use research_get.receipts to discover real Receipt Event IDs; reuse the exact target_id/version from an existing TargetSnapshot, record a missing subject before updating it, and bind the matching receipt_id and execution_id through an Experiment plus ExperimentCapsule for that TargetSnapshot before using the Receipt as target-scoped Finding evidence. A path or execution_id is not a receipt_id";

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ResearchGetArgs {
    #[serde(
        default,
        deserialize_with = "deserialize_non_null_option",
        skip_serializing_if = "Option::is_none"
    )]
    receipts: Option<ResearchReceiptQuery>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ResearchReceiptQuery {
    #[serde(default)]
    offset: usize,
    #[serde(default = "default_receipt_page_limit")]
    limit: usize,
    #[serde(
        default,
        deserialize_with = "deserialize_non_null_option",
        skip_serializing_if = "Option::is_none"
    )]
    execution_id: Option<String>,
}

fn deserialize_non_null_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

const fn default_receipt_page_limit() -> usize {
    DEFAULT_RECEIPT_PAGE_LIMIT
}

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

fn normalize_get(
    project_root: &Path,
    control_directory: &str,
    value: Value,
) -> Result<(Value, Vec<ResourceKey>), ToolExecutorError> {
    let arguments = decode_get_arguments(value)?;
    Ok((
        serde_json::to_value(arguments).expect("ResearchGetArgs serialization cannot fail"),
        research_resource(project_root, control_directory),
    ))
}

fn normalize_update(
    project_root: &Path,
    control_directory: &str,
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
        research_resource(project_root, control_directory),
    ))
}

async fn execute_get(project_root: &Path, plan: ToolExecutionPlan) -> ToolExecutionOutcome {
    let arguments = match decode_get_arguments(plan.call.arguments.clone()) {
        Ok(arguments) => arguments,
        Err(error) => return invalid_plan(plan, error),
    };
    match load_state_with_references(project_root) {
        Ok((state, references)) => {
            let stream_event_count = state.applied_event_count();
            let evidence_gap_reports = state.evidence_gap_reports(&references);
            let mut output = serde_json::Map::from_iter([
                ("status".to_owned(), json!("ok")),
                ("state".to_owned(), json!(state)),
                ("stream_event_count".to_owned(), json!(stream_event_count)),
                (
                    "evidence_gap_reports".to_owned(),
                    json!(evidence_gap_reports),
                ),
            ]);
            if let Some(query) = arguments.receipts.as_ref() {
                output.insert("receipt_page".to_owned(), receipt_page(&references, query));
            }
            ToolExecutionOutcome::succeeded(plan, Value::Object(output))
        }
        Err(error) => research_failure(plan, "research_read_failed", error),
    }
}

fn decode_get_arguments(value: Value) -> Result<ResearchGetArgs, ToolExecutorError> {
    let arguments: ResearchGetArgs = decode_arguments("research_get", value)?;
    if let Some(query) = arguments.receipts.as_ref() {
        if !(1..=MAX_RECEIPT_PAGE_LIMIT).contains(&query.limit) {
            return Err(ToolExecutorError::InvalidArguments {
                tool_name: "research_get".to_owned(),
                message: format!("`receipts.limit` must be between 1 and {MAX_RECEIPT_PAGE_LIMIT}"),
            });
        }
        if query.offset.checked_add(query.limit).is_none() {
            return Err(ToolExecutorError::InvalidArguments {
                tool_name: "research_get".to_owned(),
                message: "`receipts.offset` plus `receipts.limit` overflows pagination".to_owned(),
            });
        }
        if query
            .execution_id
            .as_deref()
            .is_some_and(|execution_id| execution_id.trim().is_empty())
        {
            return Err(ToolExecutorError::InvalidArguments {
                tool_name: "research_get".to_owned(),
                message: "`receipts.execution_id` must not be empty".to_owned(),
            });
        }
    }
    Ok(arguments)
}

fn receipt_page(references: &ResearchReferenceCatalog, query: &ResearchReceiptQuery) -> Value {
    let mut total = 0usize;
    let mut items = Vec::with_capacity(query.limit);
    for (receipt_id, receipt) in references.execution_receipts().filter(|(_, receipt)| {
        query
            .execution_id
            .as_deref()
            .is_none_or(|execution_id| receipt.execution_id.as_str() == execution_id)
    }) {
        let index = total;
        total += 1;
        if index >= query.offset && items.len() < query.limit {
            items.push(json!({
                "receipt_id": receipt_id,
                "execution_id": receipt.execution_id,
                "status": receipt.status,
            }));
        }
    }
    let consumed = query
        .offset
        .checked_add(items.len())
        .expect("validated receipt pagination cannot overflow");
    let next_offset = (consumed < total).then_some(consumed);
    json!({
        "items": items,
        "total": total,
        "offset": query.offset,
        "next_offset": next_offset,
    })
}

async fn execute_update(project_root: &Path, plan: ToolExecutionPlan) -> ToolExecutionOutcome {
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
    let references = match load_reference_catalog(&project) {
        Ok(references) => references,
        Err(error) => return research_failure(plan, "research_receipts_invalid", error),
    };
    let mut state = match reduce_store(&store, &reducer, &references) {
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
    if let Err(error) = reducer.validate_event_with_references(&state, &event, &references) {
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
    if let Err(error) = reducer.apply_with_references(&mut state, envelope, &references) {
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

fn research_resource(project_root: &Path, control_directory: &str) -> Vec<ResourceKey> {
    vec![ResourceKeyResolver::new(project_root).project_file(control_directory, "research.jsonl")]
}

pub(super) fn load_state(project_root: &Path) -> Result<ResearchState, String> {
    let project = Project::discover(project_root).map_err(|error| error.to_string())?;
    load_research_state(&project)
}

fn load_state_with_references(
    project_root: &Path,
) -> Result<(ResearchState, ResearchReferenceCatalog), String> {
    let project = Project::discover(project_root).map_err(|error| error.to_string())?;
    let references = load_reference_catalog(&project)?;
    let events = crate::adapters::project_event_store::read_existing_research_events(&project)
        .map_err(|error| error.to_string())?;
    let state = ResearchReducer
        .reduce_with_references(events, &references)
        .map_err(|error| error.to_string())?;
    Ok((state, references))
}

pub(super) fn reduce_store(
    store: &crate::adapters::project_event_store::JsonlEventStore<'_>,
    reducer: &ResearchReducer,
    references: &ResearchReferenceCatalog,
) -> Result<ResearchState, String> {
    let events = EventSource::read_events(store, &ProjectEventStream::Research.stream_id())
        .map_err(|error| error.to_string())?;
    reducer
        .reduce_with_references(events, references)
        .map_err(|error| error.to_string())
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
                capsule: Box::new(capsule),
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
    let mut error = error.to_string();
    if matches!(
        code,
        "research_record_invalid" | "research_transition_rejected"
    ) {
        error.push_str(". Hint: ");
        error.push_str(RESEARCH_REFERENCE_HINT);
    }
    ToolExecutionOutcome::failed(plan, code, json!({"error_code": code, "error": error}))
}

fn get_input_schema() -> Value {
    schema::get_input_schema()
}

fn update_input_schema() -> Value {
    schema::update_input_schema()
}

fn get_output_schema() -> Value {
    schema::get_output_schema()
}

fn update_output_schema() -> Value {
    schema::update_output_schema()
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum Tool {
    ResearchGet,
    ResearchUpdate,
}

impl Tool {
    pub(super) const ALL: [Self; 2] = [Self::ResearchGet, Self::ResearchUpdate];

    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::ResearchGet => "research_get",
            Self::ResearchUpdate => "research_update",
        }
    }

    pub(super) fn spec(self) -> ToolSpec {
        match self {
            Self::ResearchGet => super::spec(
                self.name(),
                "2",
                super::description(
                    "existing typed Research state",
                    "a mutation or transition is needed.",
                    "reuse current delivered Research facts; fetch only to fill a concrete state/reference gap. Request receipts only to discover bounded, real Receipt Event IDs; filter by an exact execution_id when known. Paths and execution IDs are not receipt IDs.",
                    "typed Research state and, when explicitly requested, a bounded receipt page sorted by receipt_id.",
                    false,
                ),
                ToolExposure::Direct,
                get_input_schema(),
                get_output_schema(),
                ToolParallelism::Parallel,
                Some("file:.echidna/research.jsonl"),
                ToolEffect::ReadOnly,
                ToolRetryPolicy::ReplaySafe,
                ToolDeterminism::Deterministic,
                10_000,
            ),
            Self::ResearchUpdate => super::spec(
                self.name(),
                "2",
                super::description(
                    "supported typed Research facts",
                    "the source is unbacked.",
                    "typed facts with real references. Reuse the exact target_id/version from research_get, create a missing record before updating it, and discover real Receipt Event IDs through research_get.receipts. A Receipt becomes target-scoped evidence only after an explicit Experiment and ExperimentCapsule bind its receipt_id and matching execution_id to that TargetSnapshot. Keep tentative analysis out of verified facts and do not manufacture records for routine progress narration.",
                    "record status.",
                    true,
                ),
                ToolExposure::Direct,
                update_input_schema(),
                update_output_schema(),
                ToolParallelism::ResourceLocked,
                Some("file:.echidna/research.jsonl"),
                ToolEffect::LocalMutation,
                ToolRetryPolicy::NeverAutoRetry,
                ToolDeterminism::EnvironmentDependent,
                10_000,
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
        match self {
            Self::ResearchGet => normalize_get(project_root, control, arguments),
            Self::ResearchUpdate => normalize_update(project_root, control, arguments),
        }
    }

    pub(super) async fn execute(
        self,
        registry: &ToolRegistry,
        plan: ToolExecutionPlan,
    ) -> ToolExecutionOutcome {
        match self {
            Self::ResearchGet => execute_get(&registry.project_root, plan).await,
            Self::ResearchUpdate => execute_update(&registry.project_root, plan).await,
        }
    }
}
