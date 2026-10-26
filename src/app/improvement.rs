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
    let current = state
        .history_items_with_event_ids()
        .skip(history_start)
        .collect::<Vec<_>>();
    let mut friction = Vec::new();
    if task.starts_with("[rsi-feedback]")
        && let Some((id, _)) = current.first()
    {
        friction.push((*id).clone());
    }
    let mut commands = BTreeMap::new();
    for (id, item) in &current {
        if let ContentItem::ToolResult { result } = item {
            let receipt = state.execution_receipts().iter().find(|record| {
                record.receipt.execution_id == ExecutionId::for_model_call(&result.call_id)
            });
            if result.status == ToolResultStatus::Failed
                && let Some(receipt) = receipt
            {
                friction.push(receipt.event_id.clone());
            }
            if let Some(call) = current.iter().find_map(|(_, item)| match item {
                ContentItem::ToolCall { call }
                    if call.call_id == result.call_id
                        && call.name == ToolName::new("run_command") =>
                {
                    Some(call)
                }
                _ => None,
            }) && result
                .output
                .get("verified_inputs")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|inputs| !inputs.is_empty())
            {
                let key = serde_json::to_string(&json!([
                    call.arguments,
                    result.output["verified_inputs"]
                ]))
                .expect("validated command serializes");
                if commands.insert(key, (*id).clone()).is_some() {
                    friction.push((*id).clone());
                }
            }
        }
    }
    friction.truncate(8);
    if friction.is_empty() {
        ImprovementReadiness::NoFriction
    } else if budget.remaining().is_some_and(|remaining| remaining < 3) {
        ImprovementReadiness::InsufficientBudget
    } else {
        ImprovementReadiness::Ready { current, friction }
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
