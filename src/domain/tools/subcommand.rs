//! Observed child facts and agreement with the one parent execution.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{
    ExecutionId, ExecutionReceipt, OutputCapture, OutputStream, ToolCall, ToolCallId,
    ToolContractError, ToolExecutionOutcome, ToolExecutionPlan, ToolName, ToolResult,
    ToolResultStatus, validate_captures,
};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SubcommandCheckpoint {
    pub(crate) schema_version: u16,
    pub(crate) execution_id: ExecutionId,
    pub(crate) call_id: ToolCallId,
    pub(crate) tool_name: ToolName,
    pub(crate) plan_fingerprint: String,
    pub(crate) child_count: u8,
    pub(crate) index: u8,
    pub(crate) status: ToolResultStatus,
    pub(crate) result: Value,
    pub(crate) output_captures: Vec<OutputCapture>,
}

pub(crate) fn reconcile_subcommands(
    children: &[SubcommandCheckpoint],
    outcome: &mut ToolExecutionOutcome,
) -> Result<(), ToolContractError> {
    let mut known = children
        .iter()
        .filter(|child| child.execution_id == outcome.receipt.execution_id)
        .collect::<Vec<_>>();
    if known.is_empty() {
        return Ok(());
    }
    known.sort_by_key(|child| child.index);
    if known
        .iter()
        .any(|child| child.plan_fingerprint != outcome.receipt.plan_fingerprint)
    {
        return Err(ToolContractError::InvalidSubcommandCheckpoint);
    }
    if outcome.result.status == ToolResultStatus::Cancelled {
        outcome.result.output["commands"] =
            Value::Array(known.iter().map(|child| child.command_value()).collect());
        outcome.result.output["unobserved_children"] = json!("unknown");
        outcome.receipt.output_captures = known
            .iter()
            .flat_map(|child| child.output_captures.clone())
            .collect();
        outcome.result.output["output_captures"] =
            serde_json::to_value(&outcome.receipt.output_captures).expect("capture serialization");
    }
    for child in known {
        if !child.agrees_with_result(&outcome.result)
            || !child.agrees_with_receipt(&outcome.receipt)
        {
            return Err(ToolContractError::InvalidSubcommandCheckpoint);
        }
    }
    Ok(())
}

impl SubcommandCheckpoint {
    pub(crate) fn observed(
        plan: &ToolExecutionPlan,
        child_count: usize,
        index: usize,
        result: Result<Value, Value>,
    ) -> Self {
        let (status, result) = match result {
            Ok(value) => (ToolResultStatus::Succeeded, value),
            Err(value) => (ToolResultStatus::Failed, value),
        };
        Self {
            schema_version: 1,
            execution_id: plan.execution_id.clone(),
            call_id: plan.call.call_id.clone(),
            tool_name: plan.call.name.clone(),
            plan_fingerprint: plan.plan_fingerprint.clone(),
            child_count: child_count as u8,
            index: index as u8,
            status,
            result,
            output_captures: Vec::new(),
        }
    }

    pub(crate) fn validate(&self) -> Result<(), ToolContractError> {
        let invalid = || ToolContractError::InvalidSubcommandCheckpoint;
        if self.schema_version != 1
            || self.tool_name.as_str() != "run_commands"
            || self.child_count == 0
            || self.child_count > 32
            || self.index >= self.child_count
            || self.execution_id != ExecutionId::for_model_call(&self.call_id)
            || self.plan_fingerprint.len() != 64
            || !self
                .plan_fingerprint
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || !matches!(
                self.status,
                ToolResultStatus::Succeeded | ToolResultStatus::Failed
            )
        {
            return Err(invalid());
        }
        let result: CommandObservation =
            serde_json::from_value(self.result.clone()).map_err(|_| invalid())?;
        crate::domain::agent_evolution::validate_input_files(&result.verified_inputs)
            .map_err(|_| invalid())?;
        if self.result.get("exit_code").is_none() {
            return Err(invalid());
        }
        if (result.exit_code == Some(0) && result.error.is_none())
            != (self.status == ToolResultStatus::Succeeded)
        {
            return Err(invalid());
        }
        validate_captures(&self.output_captures)?;
        let stream_limit = super::batch_capture_stream_limit(self.child_count as usize);
        if self.output_captures.iter().any(|capture| {
            capture.child_index != Some(self.index)
                || capture.stream == OutputStream::Pty
                || capture.retained_bytes > stream_limit as u64
        }) {
            return Err(invalid());
        }
        Ok(())
    }

    pub(crate) fn validate_after(&self, prior: &[Self]) -> Result<(), ToolContractError> {
        self.validate()?;
        if prior.iter().any(|child| {
            child.execution_id == self.execution_id
                && (child.index == self.index
                    || child.plan_fingerprint != self.plan_fingerprint
                    || child.child_count != self.child_count)
        }) {
            return Err(ToolContractError::InvalidSubcommandCheckpoint);
        }
        Ok(())
    }

    pub(crate) fn agrees_with_receipt(&self, receipt: &ExecutionReceipt) -> bool {
        self.execution_id == receipt.execution_id
            && self.plan_fingerprint == receipt.plan_fingerprint
            && self.output_captures
                == receipt
                    .output_captures
                    .iter()
                    .filter(|capture| capture.child_index == Some(self.index))
                    .cloned()
                    .collect::<Vec<_>>()
    }

    pub(crate) fn agrees_with_call(&self, call: &ToolCall) -> bool {
        self.call_id == call.call_id
            && self.tool_name == call.name
            && call
                .arguments
                .get("commands")
                .and_then(Value::as_array)
                .is_some_and(|commands| commands.len() == self.child_count as usize)
    }

    pub(crate) fn command_value(&self) -> Value {
        json!({"index": self.index, "status": self.status, "result": self.result})
    }

    pub(crate) fn agrees_with_result(&self, result: &ToolResult) -> bool {
        self.call_id == result.call_id
            && self.tool_name == result.tool_name
            && result
                .output
                .get("commands")
                .and_then(Value::as_array)
                .is_some_and(|commands| {
                    let mut matching = commands.iter().filter(|command| {
                        command.get("index").and_then(Value::as_u64) == Some(self.index as u64)
                    });
                    matching.next() == Some(&self.command_value()) && matching.next().is_none()
                })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CommandObservation {
    #[serde(rename = "cwd")]
    _cwd: String,
    exit_code: Option<i32>,
    #[serde(rename = "stdout")]
    _stdout: String,
    #[serde(rename = "stderr")]
    _stderr: String,
    #[serde(rename = "truncated")]
    _truncated: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    verified_inputs: Vec<crate::domain::agent_evolution::ScriptFile>,
}
