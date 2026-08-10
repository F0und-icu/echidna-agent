//! FullAccess file reading and deterministic directory listing.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::fs;

use crate::domain::tools::{ToolExecutionOutcome, ToolExecutionPlan};
use crate::ports::tool::ToolExecutorError;

use super::output::bounded_text;
use super::{decode_arguments, invalid_arguments, invalid_plan};

const MAX_LISTED_PATHS: usize = 2_000;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ReadFileArgs {
    path: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ListFilesArgs {
    #[serde(default = "default_path")]
    path: String,
    #[serde(default = "default_max_depth")]
    max_depth: u16,
}

fn default_path() -> String {
    ".".to_owned()
}

const fn default_max_depth() -> u16 {
    4
}

pub(super) fn normalize_read_file(
    project_root: &Path,
    value: Value,
) -> Result<(Value, Vec<String>), ToolExecutorError> {
    let arguments: ReadFileArgs = decode_arguments("read_file", value)?;
    let path = resolve_path(project_root, &arguments.path);
    Ok((
        serde_json::to_value(arguments).expect("ReadFileArgs serialization cannot fail"),
        vec![format!("file:{}", path.display())],
    ))
}

pub(super) fn normalize_list_files(
    project_root: &Path,
    value: Value,
) -> Result<(Value, Vec<String>), ToolExecutorError> {
    let arguments: ListFilesArgs = decode_arguments("list_files", value)?;
    if arguments.max_depth > 64 {
        return Err(invalid_arguments(
            "list_files",
            "`max_depth` must not exceed 64",
        ));
    }
    let path = resolve_path(project_root, &arguments.path);
    Ok((
        serde_json::to_value(arguments).expect("ListFilesArgs serialization cannot fail"),
        vec![format!("file:{}", path.display())],
    ))
}

pub(super) async fn execute_read_file(
    project_root: &Path,
    plan: ToolExecutionPlan,
) -> ToolExecutionOutcome {
    let arguments: ReadFileArgs = match decode_arguments("read_file", plan.call.arguments.clone()) {
        Ok(arguments) => arguments,
        Err(error) => return invalid_plan(plan, error),
    };
    let path = resolve_path(project_root, &arguments.path);
    match fs::read(&path).await {
        Ok(bytes) => {
            let bounded = bounded_text(&bytes, plan.output_policy.max_bytes);
            ToolExecutionOutcome::succeeded(
                plan,
                json!({
                    "path": display_path(project_root, &path),
                    "content": bounded.text,
                    "original_bytes": bounded.original_bytes,
                    "truncated": bounded.truncated,
                }),
            )
        }
        Err(error) => ToolExecutionOutcome::failed(
            plan,
            "read_failed",
            json!({"error": error.to_string(), "path": path.display().to_string()}),
        ),
    }
}

pub(super) async fn execute_list_files(
    project_root: &Path,
    plan: ToolExecutionPlan,
) -> ToolExecutionOutcome {
    let arguments: ListFilesArgs = match decode_arguments("list_files", plan.call.arguments.clone())
    {
        Ok(arguments) => arguments,
        Err(error) => return invalid_plan(plan, error),
    };
    let root = resolve_path(project_root, &arguments.path);
    let mut pending = vec![(root.clone(), 0_u16)];
    let mut paths = Vec::new();
    let mut limit_reached = false;

    while let Some((directory, depth)) = pending.pop() {
        let mut reader = match fs::read_dir(&directory).await {
            Ok(reader) => reader,
            Err(error) => {
                return ToolExecutionOutcome::failed(
                    plan,
                    "list_failed",
                    json!({"error": error.to_string(), "path": directory.display().to_string()}),
                );
            }
        };
        let mut entries = Vec::new();
        loop {
            match reader.next_entry().await {
                Ok(Some(entry)) => entries.push(entry),
                Ok(None) => break,
                Err(error) => {
                    return ToolExecutionOutcome::failed(
                        plan,
                        "list_failed",
                        json!({"error": error.to_string(), "path": directory.display().to_string()}),
                    );
                }
            }
        }
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            if paths.len() == MAX_LISTED_PATHS {
                limit_reached = true;
                break;
            }
            let path = entry.path();
            let file_type = match entry.file_type().await {
                Ok(file_type) => file_type,
                Err(error) => {
                    return ToolExecutionOutcome::failed(
                        plan,
                        "list_failed",
                        json!({"error": error.to_string(), "path": path.display().to_string()}),
                    );
                }
            };
            paths.push(display_path(project_root, &path));
            if file_type.is_dir() && depth < arguments.max_depth {
                pending.push((path, depth + 1));
            }
        }
        if limit_reached {
            break;
        }
    }

    paths.sort();
    let joined = paths.join("\n");
    let bounded = bounded_text(joined.as_bytes(), plan.output_policy.max_bytes);
    let truncated = limit_reached || bounded.truncated;
    ToolExecutionOutcome::succeeded(
        plan,
        json!({
            "path": display_path(project_root, &root),
            "paths": bounded.text,
            "path_count": paths.len(),
            "truncated": truncated,
        }),
    )
}

pub(super) fn resolve_path(project_root: &Path, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        project_root.join(path)
    }
}

pub(super) fn display_path(project_root: &Path, path: &Path) -> String {
    path.strip_prefix(project_root)
        .unwrap_or(path)
        .display()
        .to_string()
}
