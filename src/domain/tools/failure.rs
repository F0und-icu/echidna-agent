//! Provider-neutral, model-facing Tool failure semantics.
//!
//! This is deliberately a small value contract.  Adapters may retain richer
//! implementation errors internally, but the only failure envelope that can
//! cross the ToolResult boundary is this one.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{ExecutionId, ToolCall, ToolResult, ToolResultStatus};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ToolFailureStage {
    PreExecution,
    Execution,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ToolFailureRetryability {
    CorrectArguments,
    ExplicitRerunOnly,
    Terminal,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolFailure {
    code: String,
    stage: ToolFailureStage,
    retryability: ToolFailureRetryability,
    message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    execution_id: Option<ExecutionId>,
}

/// A failure which is known to have happened before an execution plan exists.
///
/// This wrapper is the only failure type accepted by
/// [`ToolResult::from_pre_execution_failure`].  It makes it impossible for an
/// execution failure to be returned without a Receipt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PreExecutionToolFailure(ToolFailure);

impl PreExecutionToolFailure {
    fn into_result_output(self, details: Value) -> Value {
        self.0.into_result_output(details)
    }
}

impl ToolFailure {
    pub(crate) fn pre_execution(
        code: impl Into<String>,
        retryability: ToolFailureRetryability,
        message: impl Into<String>,
    ) -> PreExecutionToolFailure {
        PreExecutionToolFailure(Self {
            code: code.into(),
            stage: ToolFailureStage::PreExecution,
            retryability,
            message: message.into(),
            execution_id: None,
        })
    }

    pub(crate) fn execution(
        execution_id: ExecutionId,
        code: impl Into<String>,
        retryability: ToolFailureRetryability,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code: code.into(),
            stage: ToolFailureStage::Execution,
            retryability,
            message: message.into(),
            execution_id: Some(execution_id),
        }
    }

    fn as_json(&self) -> Value {
        serde_json::to_value(self).expect("ToolFailure is always JSON serializable")
    }

    pub(crate) fn into_result_output(self, details: Value) -> Value {
        match details {
            Value::Object(mut object) => {
                object.insert("failure".to_owned(), self.as_json());
                Value::Object(object)
            }
            details => json!({"failure": self, "details": details}),
        }
    }
}

impl ToolResult {
    /// Identifies the only paired failure that deliberately has no execution
    /// plan or Receipt. The structured envelope is the single semantic owner
    /// for this distinction across Runtime and derived Eval views.
    pub(crate) fn is_pre_execution_failure(&self) -> bool {
        self.status == ToolResultStatus::Failed
            && self
                .output
                .get("failure")
                .and_then(|failure| failure.get("stage"))
                .and_then(Value::as_str)
                == Some("pre_execution")
            && self
                .output
                .get("failure")
                .and_then(|failure| failure.get("execution_id"))
                .is_none_or(Value::is_null)
    }

    /// Builds a paired pre-execution result.  No execution plan or Receipt is
    /// created, but the original model ToolCall is never dropped.
    pub(crate) fn from_pre_execution_failure(
        call: &ToolCall,
        failure: PreExecutionToolFailure,
        details: Value,
    ) -> Self {
        Self {
            call_id: call.call_id.clone(),
            tool_name: call.name.clone(),
            status: ToolResultStatus::Failed,
            output: failure.into_result_output(details),
        }
    }
}
