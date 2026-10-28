//! Fixed completed-boundary RSI eligibility over retained Session facts.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::domain::agent_definition::{AgentDefinition, AgentDefinitionBinding};
use crate::domain::agent_evolution::AgentRsi;
use crate::domain::context::ContextPlan;
use crate::domain::events::EventId;
use crate::domain::model::ContentItem;
use crate::domain::session::SessionState;
use crate::domain::tools::{ExecutionId, ToolName, ToolResultStatus};

use super::model_request_budget::ModelRequestBudget;

pub(crate) fn policy(definition: &AgentDefinition) -> Option<&AgentRsi> {
    definition.rsi().filter(|rsi| rsi.improvement)
}

pub(crate) fn completed_boundary_is_eligible(
    has_active_processes: bool,
    no_pending_images: bool,
    cancelled: bool,
) -> bool {
    !has_active_processes && no_pending_images && !cancelled
}

pub(crate) enum ImprovementReadiness<'a> {
    NoFriction,
    InsufficientBudget,
    Ready {
        current: Vec<(&'a EventId, &'a ContentItem)>,
        friction: Vec<EventId>,
    },
}

pub(crate) fn readiness<'a>(
    state: &'a SessionState,
    task: &str,
    history_start: usize,
    budget: &ModelRequestBudget,
) -> ImprovementReadiness<'a> {
    readiness_with_admissions(state, task, history_start, budget, 3)
}

pub(crate) fn readiness_with_admissions<'a>(
    state: &'a SessionState,
    task: &str,
    history_start: usize,
    budget: &ModelRequestBudget,
    admissions: usize,
) -> ImprovementReadiness<'a> {
    let current = state
        .history_items_with_event_ids()
        .skip(history_start)
        .collect::<Vec<_>>();
    let mut friction = Vec::new();
    let considered = state.assessed_improvement_history_end(history_start);
    if task.starts_with("[rsi-feedback]")
        && considered == history_start
        && let Some((id, _)) = current.first()
    {
        friction.push((*id).clone());
    }
    let detect_repetition = current.iter().enumerate().any(|(index, (_, item))| {
        index + history_start >= considered
            && matches!(item, ContentItem::ToolResult { result }
                if result.tool_name == ToolName::new("run_command")
                && result.output.get("verified_inputs").and_then(Value::as_array)
                    .is_some_and(|inputs| !inputs.is_empty()))
    });
    let mut commands: BTreeMap<[u8; 32], Vec<(&Value, &Value)>> = BTreeMap::new();
    for (index, (id, item)) in current.iter().enumerate() {
        if let ContentItem::ToolResult { result } = item {
            if result.status == ToolResultStatus::Failed
                && index + history_start >= considered
                && let Some(receipt) = state.execution_receipts().iter().find(|record| {
                    record.receipt.execution_id == ExecutionId::for_model_call(&result.call_id)
                })
            {
                friction.push(receipt.event_id.clone());
            }
            if detect_repetition
                && let Some(call) = current.iter().find_map(|(_, item)| match item {
                    ContentItem::ToolCall { call }
                        if call.call_id == result.call_id
                            && call.name == ToolName::new("run_command") =>
                    {
                        Some(call)
                    }
                    _ => None,
                })
                && result
                    .output
                    .get("verified_inputs")
                    .and_then(serde_json::Value::as_array)
                    .is_some_and(|inputs| !inputs.is_empty())
            {
                let identity = (&call.arguments, &result.output["verified_inputs"]);
                let mut hasher = blake3::Hasher::new();
                serde_json::to_writer(&mut hasher, &identity)
                    .expect("validated command serializes");
                let matches = commands.entry(*hasher.finalize().as_bytes()).or_default();
                // Hashes bound the lookup key; borrowed equality preserves the
                // exact duplicate rule even if two different values share a hash.
                let repeated = matches.contains(&identity);
                if !repeated {
                    matches.push(identity);
                }
                if repeated && index + history_start >= considered {
                    friction.push((*id).clone());
                }
            }
        }
    }
    friction.reverse();
    friction.truncate(8);
    if friction.is_empty() {
        ImprovementReadiness::NoFriction
    } else if budget
        .remaining()
        .is_some_and(|remaining| remaining < admissions)
    {
        ImprovementReadiness::InsufficientBudget
    } else {
        ImprovementReadiness::Ready { current, friction }
    }
}

/// One bounded, explicitly excerpted source package. Full durable facts still
/// own oracle validation; excerpts never become execution evidence.
pub(crate) fn diagnostic_source(
    state: &SessionState,
    task: &str,
    current: &[(&EventId, &ContentItem)],
    friction: &[EventId],
    byte_limit: usize,
    task_byte_limit: usize,
    handoff_byte_limit: usize,
) -> Result<Value, String> {
    let excerpt = |text: &str, limit: usize| {
        let end = text
            .char_indices()
            .map(|(index, _)| index)
            .chain(std::iter::once(text.len()))
            .take_while(|index| *index <= limit)
            .last()
            .unwrap_or(0);
        text[..end].to_owned()
    };
    let previous = state
        .latest_compaction()
        .and_then(|value| value.summary().narrative())
        .map(|value| serde_json::to_string(value).expect("validated narrative serializes"));
    let mut package = json!({
        "operator_task_excerpt": excerpt(task, task_byte_limit),
        "operator_task_excerpted": task.len() > task_byte_limit,
        "operator_task_original_bytes": task.len(),
        "previous_derived_handoff_excerpt": previous.as_deref().map(|text| excerpt(text, handoff_byte_limit)),
        "previous_derived_handoff_original_bytes": previous.as_deref().map(str::len),
        "previous_derived_handoff_excerpted": previous.as_deref().map(|text| text.len() > handoff_byte_limit),
        "previous_derived_handoff_source": state.latest_compaction().map(|value| json!({
            "revision": value.summary().revision(),
            "fingerprint": value.summary().fingerprint(),
        })),
        "friction": friction,
        "observations": [],
        "omitted_history_items": current.len(),
        "limitation": "Derived diagnostic excerpts; inspect retained source IDs before treating any statement as a fact. Pairs whose exact input identities cannot fit are omitted whole."
    });
    let mut calls = Vec::new();
    if serde_json::to_vec(&package)
        .expect("source package serializes")
        .len()
        > byte_limit
    {
        return Err("RSI source identities exceed the diagnostic byte allowance".to_owned());
    }
    for reference in friction {
        let receipt = state
            .execution_receipts()
            .iter()
            .find(|record| &record.event_id == reference);
        if let Some(call_id) = current.iter().find_map(|(id, item)| match item {
            ContentItem::ToolResult { result }
                if *id == reference
                    || receipt.is_some_and(|record| {
                        record.receipt.execution_id == ExecutionId::for_model_call(&result.call_id)
                    }) =>
            {
                Some(result.call_id.clone())
            }
            _ => None,
        }) && !calls.contains(&call_id)
        {
            calls.push(call_id);
        }
    }
    // Recent attested successful commands are possible sourced oracles, even
    // when the actionable friction was a failed file read rather than a command.
    let mut oracle_position = usize::from(!calls.is_empty());
    let mut oracle_count = 0;
    for (_, item) in current.iter().rev() {
        if let ContentItem::ToolResult { result } = item
            && result.tool_name == ToolName::new("run_command")
            && result.status == ToolResultStatus::Succeeded
            && result
                .output
                .get("verified_inputs")
                .and_then(Value::as_array)
                .is_some_and(|inputs| !inputs.is_empty())
            && !calls.contains(&result.call_id)
        {
            calls.insert(oracle_position, result.call_id.clone());
            oracle_position += 1;
            oracle_count += 1;
            if oracle_count == 3 {
                break;
            }
        }
    }
    let mut included = 0usize;
    for call_id in calls {
        let pair = current.iter().filter(|(_, item)| matches!(item,
            ContentItem::ToolCall { call } if call.call_id == call_id)
            || matches!(item, ContentItem::ToolResult { result } if result.call_id == call_id))
            .map(|(id, item)| diagnostic_item(id, item, byte_limit))
            .collect::<Result<Vec<_>, _>>()?;
        let Some(pair) = pair.into_iter().collect::<Option<Vec<_>>>() else {
            continue;
        };
        let count = pair.len();
        package["observations"]
            .as_array_mut()
            .expect("fixed array")
            .push(json!({"call_id":call_id,"source_pair":pair}));
        if serde_json::to_vec(&package)
            .expect("source package serializes")
            .len()
            > byte_limit
        {
            package["observations"]
                .as_array_mut()
                .expect("fixed array")
                .pop();
        } else {
            included += count;
        }
    }
    package["omitted_history_items"] = json!(current.len() - included);
    Ok(package)
}

/// The existing serializer writes into a fixed standard-library buffer. Large
/// values are never cloned or fully encoded before their excerpt is selected.
fn diagnostic_value(value: &Value, byte_limit: usize) -> Result<(Value, bool), String> {
    let mut bytes = vec![0; byte_limit];
    let mut cursor = std::io::Cursor::new(bytes.as_mut_slice());
    let encoded = serde_json::to_writer(&mut cursor, value);
    let written = cursor.position() as usize;
    match encoded {
        Ok(()) => Ok((value.clone(), false)),
        Err(error) if error.io_error_kind() == Some(std::io::ErrorKind::WriteZero) => {
            let prefix = &bytes[..written];
            let text = match std::str::from_utf8(prefix) {
                Ok(text) => text,
                Err(error) => std::str::from_utf8(&prefix[..error.valid_up_to()])
                    .expect("JSON encoding can split UTF-8 only at the bounded tail"),
            };
            Ok((json!({"excerpted":true,"encoded_value_excerpt":text}), true))
        }
        Err(error) => Err(format!("RSI diagnostic encoding failed: {error}")),
    }
}

fn diagnostic_item(
    id: &EventId,
    item: &ContentItem,
    byte_limit: usize,
) -> Result<Option<Value>, String> {
    let body_limit = byte_limit / 8;
    match item {
        ContentItem::ToolCall { call } => Ok(Some(json!({
            "event_id":id,"type":"tool_call","call_id":call.call_id,
            "tool_name":call.name,"arguments":diagnostic_value(&call.arguments,body_limit)?.0,
        }))),
        ContentItem::ToolResult { result } => {
            let mut summary = json!({
                "event_id":id,"type":"tool_result","call_id":result.call_id,
                "tool_name":result.tool_name,"status":result.status,
                "output":diagnostic_value(&result.output,body_limit)?.0,
            });
            // Preserve known command outcome/identity fields independently of
            // an oversized body. Absent fields remain absent, never defaults.
            for field in ["exit_code", "verified_inputs", "error"] {
                if let Some(value) = result.output.get(field) {
                    let identity = field == "verified_inputs";
                    let (value, excerpted) = diagnostic_value(
                        value,
                        if identity { byte_limit } else { body_limit / 2 },
                    )?;
                    if identity && excerpted {
                        return Ok(None);
                    }
                    summary[field] = value;
                }
            }
            Ok(Some(summary))
        }
        _ => unreachable!("diagnostic pairs select only Call and Result items"),
    }
}

/// Inspect retained experience and the last compiled plan without claiming
/// successful transport, model adoption or a general learning gain.
pub(crate) fn experience_status(
    binding: &AgentDefinitionBinding,
    last_plan: Option<&ContextPlan>,
) -> Value {
    let policy = binding.definition().rsi();
    let state = policy.and_then(|policy| policy.state.as_ref());
    let last_context = match last_plan {
        None => json!({"status": "no_compiled_plan"}),
        Some(plan) => {
            let candidate = plan
                .context_decision_manifest()
                .candidates()
                .iter()
                .find(|entry| entry.fragment_id().as_str() == "rsi:applicable-experience:2");
            match candidate {
                Some(entry) => json!({
                    "status": "candidate_in_last_compiled_plan",
                    "context_fingerprint": plan.context_fingerprint(),
                    "fragment_id": entry.fragment_id(),
                    "decision": entry.decision(),
                    "reason_code": entry.reason_code(),
                }),
                None => json!({
                    "status": "not_selected_in_last_compiled_plan",
                    "context_fingerprint": plan.context_fingerprint(),
                }),
            }
        }
    };
    json!({
        "enabled": policy.is_some(),
        "improvement_enabled": policy.is_some_and(|policy| policy.improvement),
        "definition_digest": binding.definition().digest().as_str(),
        "experience_count": state.map_or(0, |state| state.experiences.len()),
        "source_lineage": state.map(|state| &state.base_source_digest),
        "discovery": "bound AgentDefinition state; independent of Markdown memory",
        "context_selection": "case-insensitive task substring cues through ContextPlanner",
        "last_context": last_context,
        "model_adoption": "unobserved",
        "limitation": "The last compiled plan does not prove successful transport, model use or general improvement."
    })
}
