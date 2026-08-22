//! Minimal FullAccess Tool Registry for the M1a single-Agent loop.

mod command;
mod filesystem;
mod output;
mod resources;
mod search;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::domain::tools::{
    ExecutionId, ResourceKey, ToolCachePolicy, ToolCall, ToolDeterminism, ToolEffect,
    ToolExecutionOutcome, ToolExecutionPlan, ToolExposure, ToolName, ToolOutputPolicy,
    ToolParallelism, ToolRetryPolicy, ToolSpec, split_execution_control,
};
use crate::ports::tool::{ToolExecutionFuture, ToolExecutor, ToolExecutorError};

const TOOL_VERSION: &str = "1";
const DEFAULT_OUTPUT_BYTES: usize = 128 * 1024;

#[derive(Clone, Copy)]
enum BuiltinTool {
    ReadFile,
    ReadFiles,
    ListFiles,
    SearchFiles,
    RunCommand,
    RunCommands,
}

pub struct ToolRegistry {
    project_root: PathBuf,
    tools: BTreeMap<ToolName, BuiltinTool>,
}

impl ToolRegistry {
    pub fn for_project(project_root: impl Into<PathBuf>) -> Self {
        let tools = [
            (ToolName::new("read_file"), BuiltinTool::ReadFile),
            (ToolName::new("read_files"), BuiltinTool::ReadFiles),
            (ToolName::new("list_files"), BuiltinTool::ListFiles),
            (ToolName::new("search_files"), BuiltinTool::SearchFiles),
            (ToolName::new("run_command"), BuiltinTool::RunCommand),
            (ToolName::new("run_commands"), BuiltinTool::RunCommands),
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
        let (arguments, rerun_reason) = split_execution_control(arguments).map_err(|error| {
            ToolExecutorError::InvalidArguments {
                tool_name: call.name.to_string(),
                message: error.to_string(),
            }
        })?;
        let (normalized_arguments, resource_keys) =
            tool.normalize(&self.project_root, arguments)?;
        call.arguments = normalized_arguments;
        let spec = tool.spec();
        ToolExecutionPlan::new_with_rerun_reason(
            execution_id,
            call,
            &spec,
            resource_keys,
            rerun_reason,
        )
        .map_err(|error| ToolExecutorError::PlanFailure {
            tool_name: spec.name.to_string(),
            message: error.to_string(),
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
    ) -> Result<(Value, Vec<ResourceKey>), ToolExecutorError> {
        match self {
            Self::ReadFile => filesystem::normalize_read_file(project_root, arguments),
            Self::ReadFiles => filesystem::normalize_read_files(project_root, arguments),
            Self::ListFiles => filesystem::normalize_list_files(project_root, arguments),
            Self::SearchFiles => search::normalize(project_root, arguments),
            Self::RunCommand => command::normalize_one(project_root, arguments),
            Self::RunCommands => command::normalize_batch(project_root, arguments),
        }
    }

    async fn execute(self, project_root: &Path, plan: ToolExecutionPlan) -> ToolExecutionOutcome {
        match self {
            Self::ReadFile => filesystem::execute_read_file(project_root, plan).await,
            Self::ReadFiles => filesystem::execute_read_files(project_root, plan).await,
            Self::ListFiles => filesystem::execute_list_files(project_root, plan).await,
            Self::SearchFiles => search::execute(project_root, plan).await,
            Self::RunCommand => command::execute_one(project_root, plan).await,
            Self::RunCommands => command::execute_batch(project_root, plan).await,
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
            Self::ReadFiles => spec(
                "read_files",
                "Read multiple local files as bounded text, preserving per-file failures.",
                json!({
                    "type": "object",
                    "properties": {
                        "files": {
                            "type": "array",
                            "minItems": 1,
                            "maxItems": 32,
                            "items": {
                                "type": "object",
                                "properties": {
                                    "path": {"type": "string"},
                                    "start_line": {"type": "integer", "minimum": 1},
                                    "end_line": {"type": "integer", "minimum": 1}
                                },
                                "required": ["path"],
                                "additionalProperties": false
                            }
                        }
                    },
                    "required": ["files"],
                    "additionalProperties": false
                }),
                common_output,
                ToolParallelism::Parallel,
                Some("file:{files[*].path}"),
                ToolEffect::ReadOnly,
                ToolRetryPolicy::ReplaySafe,
                ToolDeterminism::EnvironmentDependent,
                15_000,
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
                        "patterns": {
                            "type": "array",
                            "minItems": 1,
                            "maxItems": 32,
                            "items": {"type": "string"}
                        },
                        "paths": {"type": "array", "items": {"type": "string"}}
                    },
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
                        "cwd": {"type": "string"},
                        "resources": {"type": "array", "items": command::resource_schema()}
                    },
                    "required": ["argv"],
                    "additionalProperties": false
                }),
                common_output,
                ToolParallelism::ResourceLocked,
                Some("resource:{resources}"),
                ToolEffect::Interactive,
                ToolRetryPolicy::NeverAutoRetry,
                ToolDeterminism::Nondeterministic,
                120_000,
            ),
            Self::RunCommands => spec(
                "run_commands",
                "Run several argv arrays with an explicit serial or parallel mode; each result is retained.",
                json!({
                    "type": "object",
                    "properties": {
                        "mode": {"type": "string", "enum": ["serial", "parallel"]},
                        "commands": {
                            "type": "array",
                            "minItems": 1,
                            "maxItems": 32,
                            "items": {
                                "type": "object",
                                "properties": {
                                    "argv": {"type": "array", "items": {"type": "string"}, "minItems": 1},
                                    "cwd": {"type": "string"},
                                    "resources": {"type": "array", "items": command::resource_schema()}
                                },
                                "required": ["argv"],
                                "additionalProperties": false
                            }
                        }
                    },
                    "required": ["mode", "commands"],
                    "additionalProperties": false
                }),
                common_output,
                ToolParallelism::ResourceLocked,
                Some("resource:{commands[*].resources}"),
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
        input_schema: with_execution_control(input_schema),
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

fn with_execution_control(mut schema: Value) -> Value {
    let Some(properties) = schema
        .as_object_mut()
        .and_then(|object| object.get_mut("properties"))
        .and_then(Value::as_object_mut)
    else {
        return schema;
    };
    properties.insert(
        "execution".to_owned(),
        json!({
            "type": "object",
            "properties": {
                "rerun_reason": {"type": "string", "minLength": 1}
            },
            "required": ["rerun_reason"],
            "additionalProperties": false
        }),
    );
    schema
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
