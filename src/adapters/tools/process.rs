//! Argv-only file search and process execution without a shell intermediary.

use std::path::Path;
use std::process::Output;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::process::Command;

use crate::domain::tools::{ToolExecutionOutcome, ToolExecutionPlan};
use crate::ports::tool::ToolExecutorError;

use super::filesystem::{display_path, resolve_path};
use super::output::bounded_text;
use super::{decode_arguments, invalid_arguments, invalid_plan};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SearchFilesArgs {
    pattern: String,
    #[serde(default = "default_search_paths")]
    paths: Vec<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RunCommandArgs {
    argv: Vec<String>,
    #[serde(default)]
    cwd: Option<String>,
}

fn default_search_paths() -> Vec<String> {
    vec![".".to_owned()]
}

pub(super) fn normalize_search_files(
    project_root: &Path,
    value: Value,
) -> Result<(Value, Vec<String>), ToolExecutorError> {
    let arguments: SearchFilesArgs = decode_arguments("search_files", value)?;
    if arguments.pattern.is_empty() {
        return Err(invalid_arguments("search_files", "`pattern` is empty"));
    }
    if arguments.paths.is_empty() {
        return Err(invalid_arguments("search_files", "`paths` is empty"));
    }
    let keys = arguments
        .paths
        .iter()
        .map(|path| format!("file:{}", resolve_path(project_root, path).display()))
        .collect();
    Ok((
        serde_json::to_value(arguments).expect("SearchFilesArgs serialization cannot fail"),
        keys,
    ))
}

pub(super) fn normalize_run_command(
    project_root: &Path,
    value: Value,
) -> Result<(Value, Vec<String>), ToolExecutorError> {
    let arguments: RunCommandArgs = decode_arguments("run_command", value)?;
    if arguments.argv.is_empty() || arguments.argv[0].is_empty() {
        return Err(invalid_arguments(
            "run_command",
            "`argv` must contain a non-empty program",
        ));
    }
    let cwd = resolve_path(project_root, arguments.cwd.as_deref().unwrap_or("."));
    Ok((
        serde_json::to_value(arguments).expect("RunCommandArgs serialization cannot fail"),
        vec![format!("process:{}", cwd.display())],
    ))
}

pub(super) async fn execute_search_files(
    project_root: &Path,
    plan: ToolExecutionPlan,
) -> ToolExecutionOutcome {
    let arguments: SearchFilesArgs =
        match decode_arguments("search_files", plan.call.arguments.clone()) {
            Ok(arguments) => arguments,
            Err(error) => return invalid_plan(plan, error),
        };
    let mut command = Command::new("rg");
    command
        .arg("--line-number")
        .arg("--no-heading")
        .arg("--color=never")
        .arg("--")
        .arg(&arguments.pattern)
        .args(
            arguments
                .paths
                .iter()
                .map(|path| resolve_path(project_root, path)),
        )
        .current_dir(project_root)
        .kill_on_drop(true);
    match command.output().await {
        Ok(output) if output.status.success() || output.status.code() == Some(1) => {
            let bounded = bounded_text(&output.stdout, plan.output_policy.max_bytes);
            ToolExecutionOutcome::succeeded(
                plan,
                json!({
                    "matches": bounded.text,
                    "original_bytes": bounded.original_bytes,
                    "truncated": bounded.truncated,
                }),
            )
        }
        Ok(output) => process_failure(plan, "search_failed", output),
        Err(error) => {
            ToolExecutionOutcome::failed(plan, "search_failed", json!({"error": error.to_string()}))
        }
    }
}

pub(super) async fn execute_run_command(
    project_root: &Path,
    plan: ToolExecutionPlan,
) -> ToolExecutionOutcome {
    let arguments: RunCommandArgs =
        match decode_arguments("run_command", plan.call.arguments.clone()) {
            Ok(arguments) => arguments,
            Err(error) => return invalid_plan(plan, error),
        };
    let cwd = resolve_path(project_root, arguments.cwd.as_deref().unwrap_or("."));
    let mut command = Command::new(&arguments.argv[0]);
    command
        .args(&arguments.argv[1..])
        .current_dir(&cwd)
        .kill_on_drop(true);
    match command.output().await {
        Ok(output) if output.status.success() => process_success(plan, project_root, &cwd, output),
        Ok(output) => process_failure(plan, "command_failed", output),
        Err(error) => ToolExecutionOutcome::failed(
            plan,
            "command_failed",
            json!({"error": error.to_string(), "cwd": cwd.display().to_string()}),
        ),
    }
}

fn process_success(
    plan: ToolExecutionPlan,
    project_root: &Path,
    cwd: &Path,
    output: Output,
) -> ToolExecutionOutcome {
    let max_each = plan.output_policy.max_bytes / 2;
    let stdout = bounded_text(&output.stdout, max_each);
    let stderr = bounded_text(&output.stderr, max_each);
    ToolExecutionOutcome::succeeded(
        plan,
        json!({
            "cwd": display_path(project_root, cwd),
            "exit_code": output.status.code(),
            "stdout": stdout.text,
            "stderr": stderr.text,
            "truncated": stdout.truncated || stderr.truncated,
        }),
    )
}

fn process_failure(
    plan: ToolExecutionPlan,
    reason: &'static str,
    output: Output,
) -> ToolExecutionOutcome {
    let max_each = plan.output_policy.max_bytes / 2;
    let stdout = bounded_text(&output.stdout, max_each);
    let stderr = bounded_text(&output.stderr, max_each);
    ToolExecutionOutcome::failed(
        plan,
        reason,
        json!({
            "exit_code": output.status.code(),
            "stdout": stdout.text,
            "stderr": stderr.text,
            "truncated": stdout.truncated || stderr.truncated,
        }),
    )
}
