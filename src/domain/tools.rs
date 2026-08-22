//! Provider-neutral Tool contracts and pure execution invariants.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use super::artifacts::ArtifactId;

mod resource;

pub use resource::ResourceKey;
pub(crate) use resource::scheduling_conflicts;

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
string_id!(IdempotencyKey);
string_id!(RerunReason);
string_id!(JobId);
string_id!(PtySessionId);

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
    pub parallelism: ToolParallelism,
    pub resolved_resource_keys: Vec<ResourceKey>,
    pub effect: ToolEffect,
    pub retry_policy: ToolRetryPolicy,
    /// A deliberate re-execution marker. It is excluded from the stable
    /// idempotency key but retained in the plan and Receipt for audit.
    #[serde(default)]
    pub rerun_reason: Option<RerunReason>,
    /// Present only for the explicitly deduplicable retry policy. It is a
    /// stable execution identity, never an authorization or research key.
    pub idempotency_key: Option<IdempotencyKey>,
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
        resolved_resource_keys: Vec<ResourceKey>,
    ) -> Result<Self, ToolContractError> {
        Self::new_with_rerun_reason(execution_id, call, spec, resolved_resource_keys, None)
    }

    pub(crate) fn new_with_rerun_reason(
        execution_id: ExecutionId,
        mut call: ToolCall,
        spec: &ToolSpec,
        mut resolved_resource_keys: Vec<ResourceKey>,
        explicit_rerun_reason: Option<RerunReason>,
    ) -> Result<Self, ToolContractError> {
        let (arguments, embedded_rerun_reason) = split_execution_control(call.arguments)?;
        call.arguments = arguments;
        let rerun_reason = match (explicit_rerun_reason, embedded_rerun_reason) {
            (Some(explicit), Some(embedded)) if explicit != embedded => {
                return Err(ToolContractError::InvalidExecutionControl {
                    message: "conflicting rerun reasons".to_owned(),
                });
            }
            (Some(explicit), _) | (_, Some(explicit)) => Some(explicit),
            (None, None) => None,
        };
        let input_fingerprint = fingerprint(&call.arguments)?;
        resolved_resource_keys.sort();
        resolved_resource_keys.dedup();
        let idempotency_key = (spec.retry_policy == ToolRetryPolicy::DedupeByKey)
            .then(|| {
                fingerprint(&IdempotencyFingerprint {
                    tool_name: &call.name,
                    tool_version: &spec.version,
                    input_fingerprint: &input_fingerprint,
                })
                .map(IdempotencyKey::new)
            })
            .transpose()?;
        let mut plan = Self {
            execution_id,
            call,
            tool_version: spec.version.clone(),
            parallelism: spec.parallelism,
            resolved_resource_keys,
            effect: spec.effect,
            retry_policy: spec.retry_policy,
            rerun_reason,
            idempotency_key,
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
    parallelism: ToolParallelism,
    resource_keys: &'a [ResourceKey],
    effect: ToolEffect,
    retry_policy: ToolRetryPolicy,
    rerun_reason: Option<&'a RerunReason>,
    idempotency_key: Option<&'a IdempotencyKey>,
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
            parallelism: plan.parallelism,
            resource_keys: &plan.resolved_resource_keys,
            effect: plan.effect,
            retry_policy: plan.retry_policy,
            rerun_reason: plan.rerun_reason.as_ref(),
            idempotency_key: plan.idempotency_key.as_ref(),
            determinism: plan.determinism,
            cache_policy: plan.cache_policy,
            timeout_ms: plan.timeout_ms,
            output_policy: plan.output_policy,
            input_fingerprint: &plan.input_fingerprint,
        }
    }
}

#[derive(Serialize)]
struct IdempotencyFingerprint<'a> {
    tool_name: &'a ToolName,
    tool_version: &'a str,
    input_fingerprint: &'a str,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExecutionControl {
    rerun_reason: String,
}

/// Removes the reserved, model-facing execution control envelope before a
/// concrete Tool adapter validates its arguments. The typed reason remains on
/// the execution Plan and Receipt, but never reaches adapter semantics.
pub(crate) fn split_execution_control(
    value: Value,
) -> Result<(Value, Option<RerunReason>), ToolContractError> {
    let Value::Object(mut object) = value else {
        return Ok((value, None));
    };
    let Some(control) = object.remove("execution") else {
        return Ok((Value::Object(object), None));
    };
    let control: ExecutionControl = serde_json::from_value(control).map_err(|error| {
        ToolContractError::InvalidExecutionControl {
            message: error.to_string(),
        }
    })?;
    if control.rerun_reason.trim().is_empty() {
        return Err(ToolContractError::InvalidExecutionControl {
            message: "`execution.rerun_reason` must not be empty".to_owned(),
        });
    }
    Ok((
        Value::Object(object),
        Some(RerunReason::new(control.rerun_reason)),
    ))
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

/// Stable lifecycle states for an in-process ordinary background Job.
///
/// Job state is deliberately provider- and process-implementation-neutral.
/// The process adapter owns the live child and output buffers; a later
/// persistence slice can record this contract without importing process I/O
/// into the Domain.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Running,
    Stopping,
    Completed,
    Failed,
    Cancelled,
    TimedOut,
}

/// Stable lifecycle states for one in-process current-platform PTY session.
///
/// The PTY adapter owns the live terminal and direct child. This type remains
/// provider- and terminal-implementation-neutral so a later persistence
/// Slice can record it without importing `portable-pty` into the Domain.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PtySessionStatus {
    Running,
    Stopping,
    Completed,
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
    #[serde(default)]
    pub rerun_reason: Option<RerunReason>,
}

/// The terminal state of the process launched by a background Job.
///
/// Completion is deliberately not a second `ExecutionReceipt`: the start
/// Tool owns that receipt and its successful launch status, while this value
/// records the terminal observation and links it back to the start execution
/// and plan fingerprint.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct JobCompletion {
    pub job_id: JobId,
    pub status: JobStatus,
    pub exit_code: Option<i32>,
    pub start_execution_id: ExecutionId,
    pub start_plan_fingerprint: String,
    pub exit_reason: String,
}

/// The one terminal observation for a PTY direct child.
///
/// `start_pty` owns the successful launch Receipt. This completion is linked
/// to that launch instead of creating a second execution identity.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PtySessionCompletion {
    pub pty_session_id: PtySessionId,
    pub status: PtySessionStatus,
    pub exit_code: Option<i32>,
    pub start_execution_id: ExecutionId,
    pub start_plan_fingerprint: String,
    pub exit_reason: String,
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

    /// Records durable Artifact outputs on the authoritative execution
    /// Receipt while preserving the ordinary paired Tool Result.
    pub(crate) fn succeeded_with_artifacts(
        plan: ToolExecutionPlan,
        output: Value,
        output_artifact_ids: Vec<ArtifactId>,
    ) -> Self {
        let mut outcome = Self::succeeded(plan, output);
        outcome.receipt.output_artifact_ids = output_artifact_ids;
        outcome
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

    /// Produces the paired result for a duplicate `dedupe_by_key` invocation
    /// without creating a second external execution. The original completed
    /// Receipt stays authoritative and is intentionally reused.
    pub(crate) fn deduplicated_for(plan: &ToolExecutionPlan, source: &Self) -> Self {
        Self {
            receipt: source.receipt.clone(),
            result: ToolResult {
                call_id: plan.call.call_id.clone(),
                tool_name: plan.call.name.clone(),
                status: source.result.status,
                output: source.result.output.clone(),
            },
        }
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
            rerun_reason: plan.rerun_reason,
        };
        Self { receipt, result }
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ToolContractError {
    #[error("failed to encode a stable Tool fingerprint: {message}")]
    Fingerprint { message: String },
    #[error("invalid Tool execution control: {message}")]
    InvalidExecutionControl { message: String },
}
