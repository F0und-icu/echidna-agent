//! Minimal FullAccess Tool Registry for the M1a single-Agent loop.

mod filesystem;
mod output;
mod process;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::domain::tools::{
    ExecutionId, ToolCachePolicy, ToolCall, ToolDeterminism, ToolEffect, ToolExecutionOutcome,
    ToolExecutionPlan, ToolExposure, ToolName, ToolOutputPolicy, ToolParallelism, ToolRetryPolicy,
    ToolSpec,
};
use crate::ports::tool::{ToolExecutionFuture, ToolExecutor, ToolExecutorError};

const TOOL_VERSION: &str = "1";
const DEFAULT_OUTPUT_BYTES: usize = 128 * 1024;

#[derive(Clone, Copy)]
enum BuiltinTool {
    ReadFile,
    ListFiles,
    SearchFiles,
    RunCommand,
}

pub struct ToolRegistry {
    project_root: PathBuf,
    tools: BTreeMap<ToolName, BuiltinTool>,
}

impl ToolRegistry {
    pub fn for_project(project_root: impl Into<PathBuf>) -> Self {
        let tools = [
            (ToolName::new("read_file"), BuiltinTool::ReadFile),
            (ToolName::new("list_files"), BuiltinTool::ListFiles),
            (ToolName::new("search_files"), BuiltinTool::SearchFiles),
            (ToolName::new("run_command"), BuiltinTool::RunCommand),
        ]
        .into_iter()
        .collect();
        Self {
            project_root: project_root.into(),
            tools,
        }
    }
}

impl ToolExecutor for ToolRegistry {
    fn specs(&self) -> Vec<ToolSpec> {
        self.tools.values().map(|tool| tool.spec()).collect()
    }

    fn plan(
        &self,
        execution_id: ExecutionId,
        mut call: ToolCall,
    ) -> Result<ToolExecutionPlan, ToolExecutorError> {
        let tool = self
            .tools
            .get(&call.name)
            .ok_or_else(|| ToolExecutorError::UnknownTool {
                tool_name: call.name.to_string(),
            })?;
        let arguments = std::mem::take(&mut call.arguments);
        let (normalized_arguments, resource_keys) =
            tool.normalize(&self.project_root, arguments)?;
        call.arguments = normalized_arguments;
        let spec = tool.spec();
        ToolExecutionPlan::new(execution_id, call, &spec, resource_keys).map_err(|error| {
            ToolExecutorError::PlanFailure {
                tool_name: spec.name.to_string(),
                message: error.to_string(),
            }
        })
    }

    fn execute(&self, plan: ToolExecutionPlan) -> ToolExecutionFuture<'_> {
        let tool = self.tools.get(&plan.call.name).copied();
        Box::pin(async move {
            let Some(tool) = tool else {
                return ToolExecutionOutcome::failed(
                    plan,
                    "tool_not_registered",
                    json!({"error": "execution plan names an unregistered Tool"}),
                );
            };
            let timeout_plan = plan.clone();
            match tokio::time::timeout(
                Duration::from_millis(plan.timeout_ms),
                tool.execute(&self.project_root, plan),
            )
            .await
            {
                Ok(outcome) => outcome,
                Err(_) => ToolExecutionOutcome::cancelled(timeout_plan, "tool_timeout"),
            }
        })
    }
}

impl BuiltinTool {
    fn normalize(
        self,
        project_root: &Path,
        arguments: Value,
    ) -> Result<(Value, Vec<String>), ToolExecutorError> {
        match self {
            Self::ReadFile => filesystem::normalize_read_file(project_root, arguments),
            Self::ListFiles => filesystem::normalize_list_files(project_root, arguments),
            Self::SearchFiles => process::normalize_search_files(project_root, arguments),
            Self::RunCommand => process::normalize_run_command(project_root, arguments),
        }
    }

    async fn execute(self, project_root: &Path, plan: ToolExecutionPlan) -> ToolExecutionOutcome {
        match self {
            Self::ReadFile => filesystem::execute_read_file(project_root, plan).await,
            Self::ListFiles => filesystem::execute_list_files(project_root, plan).await,
            Self::SearchFiles => process::execute_search_files(project_root, plan).await,
            Self::RunCommand => process::execute_run_command(project_root, plan).await,
        }
    }

    fn spec(self) -> ToolSpec {
        let common_output = json!({"type": "object"});
        match self {
            Self::ReadFile => spec(
                "read_file",
                "Read one local file as bounded text.",
                json!({
                    "type": "object",
                    "properties": {"path": {"type": "string"}},
                    "required": ["path"],
                    "additionalProperties": false
                }),
                common_output,
                ToolParallelism::Parallel,
                Some("file:{path}"),
                ToolEffect::ReadOnly,
                ToolRetryPolicy::ReplaySafe,
                ToolDeterminism::EnvironmentDependent,
                10_000,
            ),
            Self::ListFiles => spec(
                "list_files",
                "List local paths recursively with a bounded depth and output.",
                json!({
                    "type": "object",
                    "properties": {
                        "path": {"type": "string"},
                        "max_depth": {"type": "integer", "minimum": 0, "maximum": 64}
                    },
                    "additionalProperties": false
                }),
                common_output,
                ToolParallelism::Parallel,
                Some("file:{path}"),
                ToolEffect::ReadOnly,
                ToolRetryPolicy::ReplaySafe,
                ToolDeterminism::EnvironmentDependent,
                15_000,
            ),
            Self::SearchFiles => spec(
                "search_files",
                "Search local files with ripgrep and return bounded line matches.",
                json!({
                    "type": "object",
                    "properties": {
                        "pattern": {"type": "string"},
                        "paths": {"type": "array", "items": {"type": "string"}}
                    },
                    "required": ["pattern"],
                    "additionalProperties": false
                }),
                common_output,
                ToolParallelism::Parallel,
                Some("file:{paths}"),
                ToolEffect::ReadOnly,
                ToolRetryPolicy::ReplaySafe,
                ToolDeterminism::EnvironmentDependent,
                30_000,
            ),
            Self::RunCommand => spec(
                "run_command",
                "Run one argv array directly with FullAccess; no shell string is accepted.",
                json!({
                    "type": "object",
                    "properties": {
                        "argv": {"type": "array", "items": {"type": "string"}, "minItems": 1},
                        "cwd": {"type": "string"}
                    },
                    "required": ["argv"],
                    "additionalProperties": false
                }),
                common_output,
                ToolParallelism::Serial,
                Some("process:{cwd}"),
                ToolEffect::Interactive,
                ToolRetryPolicy::NeverAutoRetry,
                ToolDeterminism::Nondeterministic,
                120_000,
            ),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn spec(
    name: &str,
    description: &str,
    input_schema: Value,
    output_schema: Value,
    parallelism: ToolParallelism,
    resource_key_template: Option<&str>,
    effect: ToolEffect,
    retry_policy: ToolRetryPolicy,
    determinism: ToolDeterminism,
    timeout_ms: u64,
) -> ToolSpec {
    ToolSpec {
        name: ToolName::new(name),
        version: TOOL_VERSION.to_owned(),
        description: description.to_owned(),
        input_schema,
        output_schema,
        exposure: ToolExposure::Direct,
        parallelism,
        resource_key_template: resource_key_template.map(str::to_owned),
        effect,
        retry_policy,
        determinism,
        cache_policy: ToolCachePolicy::None,
        timeout_ms,
        output_policy: ToolOutputPolicy {
            max_bytes: DEFAULT_OUTPUT_BYTES,
        },
    }
}

fn decode_arguments<Arguments: for<'de> Deserialize<'de>>(
    tool_name: &str,
    value: Value,
) -> Result<Arguments, ToolExecutorError> {
    serde_json::from_value(value).map_err(|error| invalid_arguments(tool_name, error.to_string()))
}

fn invalid_arguments(tool_name: &str, message: impl Into<String>) -> ToolExecutorError {
    ToolExecutorError::InvalidArguments {
        tool_name: tool_name.to_owned(),
        message: message.into(),
    }
}

fn invalid_plan(plan: ToolExecutionPlan, error: ToolExecutorError) -> ToolExecutionOutcome {
    ToolExecutionOutcome::failed(
        plan,
        "invalid_execution_plan",
        json!({"error": error.to_string()}),
    )
}
