//! Argv-only single and batched command execution.

use std::collections::VecDeque;
use std::path::Path;

use futures_util::future::join_all;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::process::Command;

use crate::domain::tools::{
    ResourceKey, ToolEffect, ToolExecutionOutcome, ToolExecutionPlan, ToolParallelism,
    scheduling_conflicts,
};
use crate::ports::tool::ToolExecutorError;

use super::filesystem::{display_path, resolve_path};
use super::output::bounded_text;
use super::resources::ResourceKeyResolver;
use super::{decode_arguments, invalid_arguments, invalid_plan};

const MAX_COMMANDS: usize = 32;

pub(super) fn resource_schema() -> Value {
    json!({
        "oneOf": [
            {
                "type": "object",
                "properties": {
                    "kind": {"const": "device"},
                    "serial": {"type": "string", "minLength": 1}
                },
                "required": ["kind", "serial"],
                "additionalProperties": false
            },
            {
                "type": "object",
                "properties": {
                    "kind": {"const": "pty"},
                    "session": {"type": "string", "minLength": 1}
                },
                "required": ["kind", "session"],
                "additionalProperties": false
            },
            {
                "type": "object",
                "properties": {
                    "kind": {"const": "output"},
                    "path": {"type": "string", "minLength": 1}
                },
                "required": ["kind", "path"],
                "additionalProperties": false
            }
        ]
    })
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
enum CommandResource {
    Device { serial: String },
    Pty { session: String },
    Output { path: String },
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RunCommandArgs {
    argv: Vec<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    resources: Vec<CommandResource>,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum CommandBatchMode {
    Serial,
    Parallel,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RunCommandsArgs {
    mode: CommandBatchMode,
    commands: Vec<RunCommandArgs>,
}

pub(super) fn normalize_one(
    project_root: &Path,
    value: Value,
) -> Result<(Value, Vec<ResourceKey>), ToolExecutorError> {
    let arguments: RunCommandArgs = decode_arguments("run_command", value)?;
    validate_command(&arguments, "run_command")?;
    let keys = command_resource_keys(project_root, &arguments);
    Ok((
        serde_json::to_value(arguments).expect("RunCommandArgs serialization cannot fail"),
        keys,
    ))
}

pub(super) fn normalize_batch(
    project_root: &Path,
    value: Value,
) -> Result<(Value, Vec<ResourceKey>), ToolExecutorError> {
    let arguments: RunCommandsArgs = decode_arguments("run_commands", value)?;
    let keys = command_resource_sets(&arguments, project_root)?
        .into_iter()
        .flatten()
        .collect();
    Ok((
        serde_json::to_value(arguments).expect("RunCommandsArgs serialization cannot fail"),
        keys,
    ))
}

pub(super) async fn execute_one(
    project_root: &Path,
    plan: ToolExecutionPlan,
) -> ToolExecutionOutcome {
    let arguments: RunCommandArgs =
        match decode_arguments("run_command", plan.call.arguments.clone()) {
            Ok(arguments) => arguments,
            Err(error) => return invalid_plan(plan, error),
        };
    if let Err(error) = validate_command(&arguments, "run_command") {
        return invalid_plan(plan, error);
    }
    match run(project_root, &arguments, plan.output_policy.max_bytes).await {
        Ok(value) => ToolExecutionOutcome::succeeded(plan, value),
        Err(value) => ToolExecutionOutcome::failed(plan, "command_failed", value),
    }
}

pub(super) async fn execute_batch(
    project_root: &Path,
    plan: ToolExecutionPlan,
) -> ToolExecutionOutcome {
    let arguments: RunCommandsArgs =
        match decode_arguments("run_commands", plan.call.arguments.clone()) {
            Ok(arguments) => arguments,
            Err(error) => return invalid_plan(plan, error),
        };
    let resource_sets = match command_resource_sets(&arguments, project_root) {
        Ok(keys) => keys,
        Err(error) => return invalid_plan(plan, error),
    };
    let max_each = per_item_limit(plan.output_policy.max_bytes, arguments.commands.len());
    let results = match arguments.mode {
        CommandBatchMode::Serial => run_serial(project_root, &arguments.commands, max_each).await,
        CommandBatchMode::Parallel => {
            run_parallel(project_root, &arguments.commands, &resource_sets, max_each).await
        }
    };
    let partial_failure = results.iter().any(Result::is_err);
    let commands = results
        .into_iter()
        .enumerate()
        .map(|(index, result)| match result {
            Ok(value) => json!({"index": index, "status": "succeeded", "result": value}),
            Err(value) => json!({"index": index, "status": "failed", "result": value}),
        })
        .collect::<Vec<_>>();
    ToolExecutionOutcome::succeeded(
        plan,
        json!({"commands": commands, "partial_failure": partial_failure}),
    )
}

fn validate_command(arguments: &RunCommandArgs, tool_name: &str) -> Result<(), ToolExecutorError> {
    if arguments.argv.is_empty() || arguments.argv[0].is_empty() {
        return Err(invalid_arguments(
            tool_name,
            "`argv` must contain a non-empty program",
        ));
    }
    for resource in &arguments.resources {
        match resource {
            CommandResource::Device { serial } if serial.is_empty() => {
                return Err(invalid_arguments(
                    tool_name,
                    "device `serial` must not be empty",
                ));
            }
            CommandResource::Pty { session } if session.is_empty() => {
                return Err(invalid_arguments(
                    tool_name,
                    "PTY `session` must not be empty",
                ));
            }
            CommandResource::Output { path } if path.is_empty() => {
                return Err(invalid_arguments(
                    tool_name,
                    "output `path` must not be empty",
                ));
            }
            CommandResource::Device { .. }
            | CommandResource::Pty { .. }
            | CommandResource::Output { .. } => {}
        }
    }
    Ok(())
}

fn command_resource_sets(
    arguments: &RunCommandsArgs,
    project_root: &Path,
) -> Result<Vec<Vec<ResourceKey>>, ToolExecutorError> {
    if arguments.commands.is_empty() || arguments.commands.len() > MAX_COMMANDS {
        return Err(invalid_arguments(
            "run_commands",
            "`commands` must contain between 1 and 32 entries",
        ));
    }
    arguments
        .commands
        .iter()
        .map(|command| {
            validate_command(command, "run_commands")?;
            Ok(command_resource_keys(project_root, command))
        })
        .collect()
}

fn command_resource_keys(project_root: &Path, arguments: &RunCommandArgs) -> Vec<ResourceKey> {
    if arguments.resources.is_empty() {
        return vec![ResourceKey::GlobalSerial];
    }
    let resolver = ResourceKeyResolver::new(project_root);
    arguments
        .resources
        .iter()
        .map(|resource| match resource {
            CommandResource::Device { serial } => resolver.device(serial),
            CommandResource::Pty { session } => resolver.pty(session),
            CommandResource::Output { path } => resolver.output(path),
        })
        .collect()
}

async fn run_serial(
    project_root: &Path,
    commands: &[RunCommandArgs],
    max_bytes: usize,
) -> Vec<Result<Value, Value>> {
    let mut results = Vec::with_capacity(commands.len());
    for command in commands {
        results.push(run(project_root, command, max_bytes).await);
    }
    results
}

/// The command batch has no second Resource Key definition: it forms waves
/// with the Domain conflict predicate used by the AgentLoop scheduler.
async fn run_parallel(
    project_root: &Path,
    commands: &[RunCommandArgs],
    resource_sets: &[Vec<ResourceKey>],
    max_bytes: usize,
) -> Vec<Result<Value, Value>> {
    let mut pending = (0..commands.len()).collect::<VecDeque<_>>();
    let mut results = (0..commands.len()).map(|_| None).collect::<Vec<_>>();

    while !pending.is_empty() {
        let mut wave: Vec<usize> = Vec::new();
        while let Some(position) = pending.iter().position(|candidate| {
            wave.iter().all(|active| {
                !scheduling_conflicts(
                    ToolParallelism::ResourceLocked,
                    ToolEffect::Interactive,
                    resource_sets[*candidate].as_slice(),
                    ToolParallelism::ResourceLocked,
                    ToolEffect::Interactive,
                    resource_sets[*active].as_slice(),
                )
            })
        }) {
            if let Some(index) = pending.remove(position) {
                wave.push(index);
            }
        }
        let completed = join_all(
            wave.iter()
                .map(|index| run(project_root, &commands[*index], max_bytes)),
        )
        .await;
        for (index, result) in wave.into_iter().zip(completed) {
            results[index] = Some(result);
        }
    }

    results
        .into_iter()
        .map(|result| {
            result.unwrap_or_else(|| Err(json!({"error": "parallel_batch_scheduler_incomplete"})))
        })
        .collect()
}

async fn run(
    project_root: &Path,
    arguments: &RunCommandArgs,
    max_bytes: usize,
) -> Result<Value, Value> {
    let cwd = resolve_path(project_root, arguments.cwd.as_deref().unwrap_or("."));
    let mut command = Command::new(&arguments.argv[0]);
    command
        .args(&arguments.argv[1..])
        .current_dir(&cwd)
        .kill_on_drop(true);
    match command.output().await {
        Ok(output) if output.status.success() => Ok(output_value(
            project_root,
            &cwd,
            output.status.code(),
            &output.stdout,
            &output.stderr,
            max_bytes,
        )),
        Ok(output) => Err(output_value(
            project_root,
            &cwd,
            output.status.code(),
            &output.stdout,
            &output.stderr,
            max_bytes,
        )),
        Err(error) => Err(json!({"error": error.to_string(), "cwd": cwd.display().to_string()})),
    }
}

fn output_value(
    project_root: &Path,
    cwd: &Path,
    exit_code: Option<i32>,
    stdout_bytes: &[u8],
    stderr_bytes: &[u8],
    max_bytes: usize,
) -> Value {
    let max_each = max_bytes / 2;
    let stdout = bounded_text(stdout_bytes, max_each);
    let stderr = bounded_text(stderr_bytes, max_each);
    json!({
        "cwd": display_path(project_root, cwd),
        "exit_code": exit_code,
        "stdout": stdout.text,
        "stderr": stderr.text,
        "truncated": stdout.truncated || stderr.truncated,
    })
}

fn per_item_limit(total: usize, count: usize) -> usize {
    total / count.max(1)
}
