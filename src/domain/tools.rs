//! Provider-neutral Tool contracts and pure execution invariants.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use super::artifacts::ArtifactId;
use super::events::{EventId, StreamId};

mod failure;
mod resolution;
mod resource;

pub(crate) use failure::{ToolFailure, ToolFailureRetryability};
pub use resolution::{DeferredActivationSet, ResolvedToolPlan};
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
string_id!(ToolResultCacheKey);
string_id!(RerunReason);
string_id!(JobId);
string_id!(PtySessionId);

impl ExecutionId {
    /// The one stable execution identity for a model-originated Tool Call.
    /// Recovery uses the same identity to distinguish a durable Receipt with a
    /// missing paired History result from an execution that never started.
    pub(crate) fn for_model_call(call_id: &ToolCallId) -> Self {
        Self::new(format!("execution:{call_id}"))
    }
}

// Stable semantic identity of one completely resolved model-visible Tool
// plan. It is distinct from an individual ToolExecutionPlan fingerprint.
string_id!(ResolvedToolPlanFingerprint);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolExposure {
    Direct,
    Deferred,
    Hidden,
}

/// A fixed Domain, Phase or local configuration override for named Tool exposure.
///
/// Unmentioned Tools retain the registry's built-in exposure. This is a
/// bounded data contract, not a dynamic plugin or policy system.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ToolExposurePlan {
    overrides: BTreeMap<ToolName, ToolExposure>,
}

impl ToolExposurePlan {
    pub fn new(overrides: impl IntoIterator<Item = (ToolName, ToolExposure)>) -> Self {
        Self {
            overrides: overrides.into_iter().collect(),
        }
    }

    pub fn exposure_for(&self, tool_name: &ToolName, default: ToolExposure) -> ToolExposure {
        self.overrides.get(tool_name).copied().unwrap_or(default)
    }

    pub fn overrides(&self) -> &BTreeMap<ToolName, ToolExposure> {
        &self.overrides
    }

    /// Composes ordered Domain/Phase layers once. Hidden is terminal; later
    /// layers may promote or defer only a non-Hidden Tool.
    pub fn layered(layers: impl IntoIterator<Item = Self>) -> Self {
        let mut overrides = BTreeMap::new();
        for layer in layers {
            for (name, exposure) in layer.overrides {
                if overrides.get(&name) != Some(&ToolExposure::Hidden) {
                    overrides.insert(name, exposure);
                }
            }
        }
        Self { overrides }
    }

    /// Narrows fixed Domain/Phase overrides to the executor's immutable
    /// catalog. Explicit callers that pass an unknown override directly to
    /// plan resolution still receive the strict unknown-Tool error.
    pub fn restricted_to_catalog(mut self, catalog: &[ToolSpec]) -> Self {
        let names = catalog
            .iter()
            .map(|spec| spec.name.clone())
            .collect::<BTreeSet<_>>();
        self.overrides.retain(|name, _| names.contains(name));
        self
    }

    /// Applies the explicit override to a concrete schema while leaving all
    /// model-facing schema fields under the existing ToolSpec owner.
    pub fn apply_to_spec(&self, mut spec: ToolSpec) -> ToolSpec {
        if spec.exposure != ToolExposure::Hidden {
            spec.exposure = self.exposure_for(&spec.name, spec.exposure);
        }
        spec
    }
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
    /// A Tool-owned stable description of every mutable read dependency. It
    /// is absent unless the adapter can prove a result-cache hit is safe.
    #[serde(default)]
    pub cache_resource_fingerprint: Option<String>,
    /// Present only for a model-originated call planned through the exact
    /// immutable Tool plan which advertised it.
    #[serde(default)]
    resolved_tool_plan_fingerprint: Option<ResolvedToolPlanFingerprint>,
    /// Exact Step-local schemas needed by Tool implementations such as
    /// `tool_search` and `inspect_project`. They are transient execution
    /// inputs; the Receipt retains only their resolved-plan identity.
    #[serde(default)]
    resolved_tool_specs: Vec<ToolSpec>,
    #[serde(default)]
    deferred_tool_specs: Vec<ToolSpec>,
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
            cache_resource_fingerprint: None,
            resolved_tool_plan_fingerprint: None,
            resolved_tool_specs: Vec::new(),
            deferred_tool_specs: Vec::new(),
            plan_fingerprint: String::new(),
        };
        plan.plan_fingerprint = fingerprint(&PlanFingerprintInput::from(&plan))?;
        Ok(plan)
    }

    pub fn bind_resolved_tool_plan(
        &mut self,
        resolved: &ResolvedToolPlan,
    ) -> Result<(), ToolContractError> {
        let Some(spec) = resolved.advertised_spec(&self.call.name) else {
            return Err(ToolContractError::ToolNotAdvertised {
                tool_name: self.call.name.to_string(),
            });
        };
        if !self.matches_spec(spec) {
            return Err(ToolContractError::ResolvedToolSpecMismatch {
                tool_name: self.call.name.to_string(),
            });
        }
        self.resolved_tool_plan_fingerprint = Some(resolved.fingerprint().clone());
        self.resolved_tool_specs = resolved.advertised_specs().to_vec();
        self.deferred_tool_specs = resolved.deferred_specs().to_vec();
        self.plan_fingerprint = fingerprint(&PlanFingerprintInput::from(&*self))?;
        Ok(())
    }

    pub fn resolved_tool_plan_fingerprint(&self) -> Option<&ResolvedToolPlanFingerprint> {
        self.resolved_tool_plan_fingerprint.as_ref()
    }

    pub(crate) fn resolved_tool_specs(&self) -> &[ToolSpec] {
        &self.resolved_tool_specs
    }

    pub(crate) fn deferred_tool_specs(&self) -> &[ToolSpec] {
        &self.deferred_tool_specs
    }

    fn matches_spec(&self, spec: &ToolSpec) -> bool {
        self.call.name == spec.name
            && self.tool_version == spec.version
            && self.parallelism == spec.parallelism
            && self.effect == spec.effect
            && self.retry_policy == spec.retry_policy
            && self.determinism == spec.determinism
            && self.cache_policy == spec.cache_policy
            && self.timeout_ms == spec.timeout_ms
            && self.output_policy == spec.output_policy
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
    cache_resource_fingerprint: Option<&'a str>,
    resolved_tool_plan_fingerprint: Option<&'a ResolvedToolPlanFingerprint>,
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
            cache_resource_fingerprint: plan.cache_resource_fingerprint.as_deref(),
            resolved_tool_plan_fingerprint: plan.resolved_tool_plan_fingerprint.as_ref(),
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionReceipt {
    pub execution_id: ExecutionId,
    pub plan_fingerprint: String,
    #[serde(default)]
    pub resolved_tool_plan_fingerprint: Option<ResolvedToolPlanFingerprint>,
    pub status: ToolResultStatus,
    pub exit_reason: Option<String>,
    pub output_artifact_ids: Vec<ArtifactId>,
    pub environment_change: Option<String>,
    #[serde(default)]
    pub rerun_reason: Option<RerunReason>,
    /// Present only on a persistent derived-cache hit. This remains a
    /// reference to the original completed Receipt; it never reuses that
    /// Receipt's execution identity.
    #[serde(default)]
    pub cache_source: Option<ToolCacheSource>,
    /// Verification facts for the mutable read dependency that made a source
    /// result safe to retain in the derived cache.
    #[serde(default)]
    pub cache_validation: Option<ToolCacheValidation>,
    /// Successful local file mutations retain their exact before/after facts
    /// on the Receipt, independently of any UI rendering.
    #[serde(default)]
    pub file_mutations: Vec<FileMutationAudit>,
}

/// The single versioned payload shape for authoritative Session Events that
/// persist an execution Receipt. The Event type distinguishes an original
/// completion from a derived-cache hit; the Receipt itself owns the optional
/// cache-source link so the persisted fact never duplicates that semantic.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExecutionReceiptEventPayload {
    schema_version: u16,
    receipt: ExecutionReceipt,
}

impl ExecutionReceiptEventPayload {
    const CURRENT_SCHEMA_VERSION: u16 = 1;

    pub(crate) fn current(receipt: ExecutionReceipt) -> Self {
        Self {
            schema_version: Self::CURRENT_SCHEMA_VERSION,
            receipt,
        }
    }

    /// Strictly decodes the already-published Session Receipt payload without
    /// exposing or duplicating its schema in another Session owner.
    pub(crate) fn decode_current(
        value: Value,
    ) -> Result<ExecutionReceipt, ExecutionReceiptPayloadError> {
        let payload: Self = serde_json::from_value(value).map_err(|error| {
            ExecutionReceiptPayloadError::Malformed {
                message: error.to_string(),
            }
        })?;
        if payload.schema_version != Self::CURRENT_SCHEMA_VERSION {
            return Err(ExecutionReceiptPayloadError::UnsupportedSchemaVersion {
                schema_version: payload.schema_version,
            });
        }
        Ok(payload.receipt)
    }
}

/// Strict decoding errors for the already-published Receipt Event payload.
/// Session reduction maps these onto its event-type-aware error boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ExecutionReceiptPayloadError {
    Malformed { message: String },
    UnsupportedSchemaVersion { schema_version: u16 },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ToolCacheSource {
    pub cache_key: ToolResultCacheKey,
    pub source_stream_id: StreamId,
    pub source_receipt_event_id: EventId,
    pub source_execution_id: ExecutionId,
    pub source_plan_fingerprint: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ToolCacheValidation {
    pub cache_resource_fingerprint: String,
    pub observed_before_fingerprint: String,
    pub observed_after_fingerprint: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FileMutationAudit {
    pub path: String,
    pub expected_preimage_hash: Option<String>,
    pub before_hash: Option<String>,
    pub before_size: Option<u64>,
    pub after_hash: Option<String>,
    pub after_size: Option<u64>,
    pub actual_diff: String,
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

    pub fn failed(plan: ToolExecutionPlan, code: impl Into<String>, output: Value) -> Self {
        let code = code.into();
        let message = tool_failure_message(&output, &code);
        let execution_id = plan.execution_id.clone();
        let mut outcome =
            Self::from_plan(plan, ToolResultStatus::Failed, Some(code.clone()), output);
        outcome.result.output = ToolFailure::execution(
            execution_id,
            code,
            ToolFailureRetryability::ExplicitRerunOnly,
            message,
        )
        .into_result_output(outcome.result.output);
        outcome
    }

    /// Replay preserves the exact recorded Tool output.  The source failure
    /// is already an immutable fact and must not be rewritten by a derived
    /// replay adapter.
    pub(crate) fn failed_preserving_output(
        plan: ToolExecutionPlan,
        reason: impl Into<String>,
        output: Value,
    ) -> Self {
        Self::from_plan(plan, ToolResultStatus::Failed, Some(reason.into()), output)
    }

    pub fn cancelled(plan: ToolExecutionPlan, reason: impl Into<String>) -> Self {
        let reason = reason.into();
        let execution_id = plan.execution_id.clone();
        let mut outcome =
            Self::cancelled_with_output(plan, reason.clone(), serde_json::json!({"error": reason}));
        outcome.result.output = ToolFailure::execution(
            execution_id,
            "tool_execution_cancelled",
            ToolFailureRetryability::Terminal,
            reason,
        )
        .into_result_output(outcome.result.output);
        outcome
    }

    /// Produces a cancelled paired outcome while retaining a previously
    /// recorded semantic Tool output. Replay uses this instead of rebuilding
    /// the output from the cancellation reason.
    pub(crate) fn cancelled_with_output(
        plan: ToolExecutionPlan,
        reason: impl Into<String>,
        output: Value,
    ) -> Self {
        Self::from_plan(
            plan,
            ToolResultStatus::Cancelled,
            Some(reason.into()),
            output,
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

    pub(crate) fn record_file_mutations(&mut self, file_mutations: Vec<FileMutationAudit>) {
        self.receipt.file_mutations = file_mutations;
        self.receipt.environment_change = Some("local_files_mutated".to_owned());
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
            resolved_tool_plan_fingerprint: plan.resolved_tool_plan_fingerprint,
            status,
            exit_reason,
            output_artifact_ids: Vec::new(),
            environment_change: None,
            rerun_reason: plan.rerun_reason,
            cache_source: None,
            cache_validation: None,
            file_mutations: Vec::new(),
        };
        Self { receipt, result }
    }
}

fn tool_failure_message(output: &Value, fallback_code: &str) -> String {
    output
        .get("error")
        .or_else(|| output.get("message"))
        .and_then(Value::as_str)
        .filter(|message| !message.trim().is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| fallback_code.to_owned())
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ToolContractError {
    #[error("failed to encode a stable Tool fingerprint: {message}")]
    Fingerprint { message: String },
    #[error("invalid Tool execution control: {message}")]
    InvalidExecutionControl { message: String },
    #[error("the canonical Tool catalog contains a duplicate name")]
    DuplicateToolName,
    #[error("Tool exposure names unknown Tool `{tool_name}`")]
    UnknownToolExposure { tool_name: String },
    #[error("Tool `{tool_name}` cannot be activated from its resolved exposure")]
    InvalidDeferredActivation { tool_name: String },
    #[error("Tool `{tool_name}` was not advertised by the resolved Tool plan")]
    ToolNotAdvertised { tool_name: String },
    #[error("Tool `{tool_name}` execution semantics differ from its resolved ToolSpec")]
    ResolvedToolSpecMismatch { tool_name: String },
    #[error("successful tool_search output is malformed")]
    InvalidToolSearchResult,
}
