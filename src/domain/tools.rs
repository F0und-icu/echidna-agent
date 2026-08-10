//! Provider-neutral Tool contracts and pure execution invariants.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use super::artifacts::ArtifactId;

macro_rules! string_id {
    ($name:ident) => {
        #[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }
    };
}

string_id!(ToolName);
string_id!(ToolCallId);
string_id!(ExecutionId);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolExposure {
    Direct,
    Deferred,
    Hidden,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolParallelism {
    Parallel,
    ResourceLocked,
    Serial,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolEffect {
    ReadOnly,
    LocalMutation,
    ExternalMutation,
    Interactive,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolRetryPolicy {
    ReplaySafe,
    DedupeByKey,
    NeverAutoRetry,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolDeterminism {
    Deterministic,
    EnvironmentDependent,
    Nondeterministic,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCachePolicy {
    None,
    ContentAddressed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ToolOutputPolicy {
    pub max_bytes: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ToolSpec {
    pub name: ToolName,
    pub version: String,
    pub description: String,
    pub input_schema: Value,
    pub output_schema: Value,
    pub exposure: ToolExposure,
    pub parallelism: ToolParallelism,
    pub resource_key_template: Option<String>,
    pub effect: ToolEffect,
    pub retry_policy: ToolRetryPolicy,
    pub determinism: ToolDeterminism,
    pub cache_policy: ToolCachePolicy,
    pub timeout_ms: u64,
    pub output_policy: ToolOutputPolicy,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ToolCall {
    pub call_id: ToolCallId,
    pub name: ToolName,
    pub arguments: Value,
}

impl ToolCall {
    pub fn new(call_id: ToolCallId, name: ToolName, arguments: Value) -> Self {
        Self {
            call_id,
            name,
            arguments,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ToolExecutionPlan {
    pub execution_id: ExecutionId,
    pub call: ToolCall,
    pub tool_version: String,
    pub resolved_resource_keys: Vec<String>,
    pub effect: ToolEffect,
    pub retry_policy: ToolRetryPolicy,
    pub determinism: ToolDeterminism,
    pub cache_policy: ToolCachePolicy,
    pub timeout_ms: u64,
    pub output_policy: ToolOutputPolicy,
    pub input_fingerprint: String,
    pub plan_fingerprint: String,
}

impl ToolExecutionPlan {
    pub fn new(
        execution_id: ExecutionId,
        call: ToolCall,
        spec: &ToolSpec,
        resolved_resource_keys: Vec<String>,
    ) -> Result<Self, ToolContractError> {
        let input_fingerprint = fingerprint(&call.arguments)?;
        let mut plan = Self {
            execution_id,
            call,
            tool_version: spec.version.clone(),
            resolved_resource_keys,
            effect: spec.effect,
            retry_policy: spec.retry_policy,
            determinism: spec.determinism,
            cache_policy: spec.cache_policy,
            timeout_ms: spec.timeout_ms,
            output_policy: spec.output_policy,
            input_fingerprint,
            plan_fingerprint: String::new(),
        };
        plan.plan_fingerprint = fingerprint(&PlanFingerprintInput::from(&plan))?;
        Ok(plan)
    }
}

#[derive(Serialize)]
struct PlanFingerprintInput<'a> {
    execution_id: &'a ExecutionId,
    call: &'a ToolCall,
    tool_version: &'a str,
    resource_keys: &'a [String],
    effect: ToolEffect,
    retry_policy: ToolRetryPolicy,
    determinism: ToolDeterminism,
    cache_policy: ToolCachePolicy,
    timeout_ms: u64,
    output_policy: ToolOutputPolicy,
    input_fingerprint: &'a str,
}

impl<'a> From<&'a ToolExecutionPlan> for PlanFingerprintInput<'a> {
    fn from(plan: &'a ToolExecutionPlan) -> Self {
        Self {
            execution_id: &plan.execution_id,
            call: &plan.call,
            tool_version: &plan.tool_version,
            resource_keys: &plan.resolved_resource_keys,
            effect: plan.effect,
            retry_policy: plan.retry_policy,
            determinism: plan.determinism,
            cache_policy: plan.cache_policy,
            timeout_ms: plan.timeout_ms,
            output_policy: plan.output_policy,
            input_fingerprint: &plan.input_fingerprint,
        }
    }
}

fn fingerprint(value: &impl Serialize) -> Result<String, ToolContractError> {
    let bytes = serde_json::to_vec(value).map_err(|error| ToolContractError::Fingerprint {
        message: error.to_string(),
    })?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolResultStatus {
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ToolResult {
    pub call_id: ToolCallId,
    pub tool_name: ToolName,
    pub status: ToolResultStatus,
    pub output: Value,
}

impl ToolResult {
    pub fn failed(call: &ToolCall, message: impl Into<String>) -> Self {
        Self {
            call_id: call.call_id.clone(),
            tool_name: call.name.clone(),
            status: ToolResultStatus::Failed,
            output: serde_json::json!({"error": message.into()}),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExecutionReceipt {
    pub execution_id: ExecutionId,
    pub plan_fingerprint: String,
    pub status: ToolResultStatus,
    pub exit_reason: Option<String>,
    pub output_artifact_ids: Vec<ArtifactId>,
    pub environment_change: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ToolExecutionOutcome {
    pub receipt: ExecutionReceipt,
    pub result: ToolResult,
}

impl ToolExecutionOutcome {
    pub fn succeeded(plan: ToolExecutionPlan, output: Value) -> Self {
        Self::from_plan(plan, ToolResultStatus::Succeeded, None, output)
    }

    pub fn failed(plan: ToolExecutionPlan, reason: impl Into<String>, output: Value) -> Self {
        Self::from_plan(plan, ToolResultStatus::Failed, Some(reason.into()), output)
    }

    pub fn cancelled(plan: ToolExecutionPlan, reason: impl Into<String>) -> Self {
        let reason = reason.into();
        Self::from_plan(
            plan,
            ToolResultStatus::Cancelled,
            Some(reason.clone()),
            serde_json::json!({"error": reason}),
        )
    }

    fn from_plan(
        plan: ToolExecutionPlan,
        status: ToolResultStatus,
        exit_reason: Option<String>,
        output: Value,
    ) -> Self {
        let result = ToolResult {
            call_id: plan.call.call_id.clone(),
            tool_name: plan.call.name.clone(),
            status,
            output,
        };
        let receipt = ExecutionReceipt {
            execution_id: plan.execution_id,
            plan_fingerprint: plan.plan_fingerprint,
            status,
            exit_reason,
            output_artifact_ids: Vec::new(),
            environment_change: None,
        };
        Self { receipt, result }
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ToolContractError {
    #[error("failed to encode a stable Tool fingerprint: {message}")]
    Fingerprint { message: String },
}
