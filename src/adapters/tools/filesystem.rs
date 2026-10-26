//! FullAccess file reading and deterministic directory listing.

use super::ToolRegistry;
use crate::domain::tools::{
    ToolDeterminism, ToolEffect, ToolExposure, ToolParallelism, ToolRetryPolicy, ToolSpec,
};

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use futures_util::future::join_all;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::fs;

use crate::domain::tools::{ResourceKey, ToolExecutionOutcome, ToolExecutionPlan};
use crate::ports::tool::ToolExecutorError;

use super::output::bounded_text;
use super::resources::ResourceKeyResolver;
use super::{decode_arguments, invalid_arguments, invalid_plan};

const MAX_LISTED_PATHS: usize = 2_000;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ReadFileArgs {
    path: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ReadFileRangeArgs {
    path: String,
    #[serde(default)]
    start_line: Option<u64>,
    #[serde(default)]
    end_line: Option<u64>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ReadFilesArgs {
    files: Vec<ReadFileRangeArgs>,
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

fn normalize_read_file(
    project_root: &Path,
    value: Value,
) -> Result<(Value, Vec<ResourceKey>), ToolExecutorError> {
    let arguments: ReadFileArgs = decode_arguments("read_file", value)?;
    let resource_keys = vec![ResourceKeyResolver::new(project_root).file(&arguments.path)];
    Ok((
        serde_json::to_value(arguments).expect("ReadFileArgs serialization cannot fail"),
        resource_keys,
    ))
}

fn normalize_read_files(
    project_root: &Path,
    value: Value,
) -> Result<(Value, Vec<ResourceKey>), ToolExecutorError> {
    let arguments: ReadFilesArgs = decode_arguments("read_files", value)?;
    if arguments.files.is_empty() || arguments.files.len() > 32 {
        return Err(invalid_arguments(
            "read_files",
            "`files` must contain between 1 and 32 entries",
        ));
    }
    for file in &arguments.files {
        validate_range(file, "read_files")?;
    }
    let resolver = ResourceKeyResolver::new(project_root);
    let resource_keys = arguments
        .files
        .iter()
        .map(|file| resolver.file(&file.path))
        .collect();
    Ok((
        serde_json::to_value(arguments).expect("ReadFilesArgs serialization cannot fail"),
        resource_keys,
    ))
}

fn normalize_list_files(
    project_root: &Path,
    value: Value,
) -> Result<(Value, Vec<ResourceKey>), ToolExecutorError> {
    let arguments: ListFilesArgs = decode_arguments("list_files", value)?;
    if arguments.max_depth > 64 {
        return Err(invalid_arguments(
            "list_files",
            "`max_depth` must not exceed 64",
        ));
    }
    let resource_keys = vec![ResourceKeyResolver::new(project_root).file(&arguments.path)];
    Ok((
        serde_json::to_value(arguments).expect("ListFilesArgs serialization cannot fail"),
        resource_keys,
    ))
}

async fn execute_read_file(project_root: &Path, plan: ToolExecutionPlan) -> ToolExecutionOutcome {
    let arguments: ReadFileArgs = match decode_arguments("read_file", plan.call.arguments.clone()) {
        Ok(arguments) => arguments,
        Err(error) => return invalid_plan(plan, error),
    };
    let range = ReadFileRangeArgs {
        path: arguments.path,
        start_line: None,
        end_line: None,
    };
    match read_file_value(project_root, &range, plan.output_policy.max_bytes).await {
        Ok(value) => ToolExecutionOutcome::succeeded(plan, value),
        Err(value) => ToolExecutionOutcome::failed(plan, "read_failed", value),
    }
}

async fn execute_read_files(project_root: &Path, plan: ToolExecutionPlan) -> ToolExecutionOutcome {
    let arguments: ReadFilesArgs = match decode_arguments("read_files", plan.call.arguments.clone())
    {
        Ok(arguments) => arguments,
        Err(error) => return invalid_plan(plan, error),
    };
    if let Err(error) = validate_read_files(&arguments) {
        return invalid_plan(plan, error);
    }

    let max_each = per_item_limit(plan.output_policy.max_bytes, arguments.files.len());
    let results = join_all(
        arguments
            .files
            .iter()
            .map(|file| read_file_value(project_root, file, max_each)),
    )
    .await;
    let partial_failure = results.iter().any(Result::is_err);
    let files = results
        .into_iter()
        .map(|result| match result {
            Ok(value) => json!({"status": "succeeded", "result": value}),
            Err(value) => json!({"status": "failed", "result": value}),
        })
        .collect::<Vec<_>>();
    ToolExecutionOutcome::succeeded(
        plan,
        json!({"files": files, "partial_failure": partial_failure}),
    )
}

async fn execute_list_files(project_root: &Path, plan: ToolExecutionPlan) -> ToolExecutionOutcome {
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
    let relative = path.strip_prefix(project_root).unwrap_or(path);
    if relative.as_os_str().is_empty() {
        ".".to_owned()
    } else {
        relative.display().to_string()
    }
}

fn validate_read_files(arguments: &ReadFilesArgs) -> Result<(), ToolExecutorError> {
    if arguments.files.is_empty() || arguments.files.len() > 32 {
        return Err(invalid_arguments(
            "read_files",
            "`files` must contain between 1 and 32 entries",
        ));
    }
    for file in &arguments.files {
        validate_range(file, "read_files")?;
    }
    Ok(())
}

fn validate_range(arguments: &ReadFileRangeArgs, tool_name: &str) -> Result<(), ToolExecutorError> {
    if arguments.start_line == Some(0) {
        return Err(invalid_arguments(
            tool_name,
            "`start_line` must be at least 1",
        ));
    }
    if arguments.end_line == Some(0) {
        return Err(invalid_arguments(
            tool_name,
            "`end_line` must be at least 1",
        ));
    }
    if let (Some(start), Some(end)) = (arguments.start_line, arguments.end_line)
        && end < start
    {
        return Err(invalid_arguments(
            tool_name,
            "`end_line` must not be before `start_line`",
        ));
    }
    Ok(())
}

async fn read_file_value(
    project_root: &Path,
    arguments: &ReadFileRangeArgs,
    max_bytes: usize,
) -> Result<Value, Value> {
    let path = resolve_path(project_root, &arguments.path);
    let bytes = fs::read(&path)
        .await
        .map_err(|error| json!({"error": error.to_string(), "path": path.display().to_string()}))?;
    let selected = if arguments.start_line.is_none() && arguments.end_line.is_none() {
        Cow::Borrowed(bytes.as_slice())
    } else {
        Cow::Owned(select_lines(&bytes, arguments.start_line, arguments.end_line).into_bytes())
    };
    let bounded = bounded_text(&selected, max_bytes);
    Ok(json!({
        "path": display_path(project_root, &path),
        "content": bounded.text,
        "original_bytes": bytes.len(),
        "selected_bytes": bounded.original_bytes,
        "truncated": bounded.truncated,
        "start_line": arguments.start_line,
        "end_line": arguments.end_line,
    }))
}

fn select_lines(bytes: &[u8], start_line: Option<u64>, end_line: Option<u64>) -> String {
    let start = start_line.unwrap_or(1).saturating_sub(1) as usize;
    let end = end_line.map(|line| line as usize);
    String::from_utf8_lossy(bytes)
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            (index >= start && end.is_none_or(|end| index < end)).then_some(line)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn per_item_limit(total: usize, count: usize) -> usize {
    total / count.max(1)
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum Tool {
    ReadFile,
    ReadFiles,
    ListFiles,
}

impl Tool {
    pub(super) const ALL: [Self; 3] = [Self::ReadFile, Self::ReadFiles, Self::ListFiles];

    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::ReadFile => "read_file",
            Self::ReadFiles => "read_files",
            Self::ListFiles => "list_files",
        }
    }

    pub(super) fn spec(self) -> ToolSpec {
        match self {
            Self::ReadFile => super::spec(
                self.name(),
                "1",
                super::description(
                    "one local file",
                    "a batch or search is needed.",
                    "line ranges.",
                    "content, bytes and truncation.",
                    false,
                ),
                ToolExposure::Direct,
                json!({
                    "type": "object",
                    "properties": {"path": {"type": "string"}},
                    "required": ["path"],
                    "additionalProperties": false
                }),
                self.output_schema(),
                ToolParallelism::Parallel,
                Some("file:{path}"),
                ToolEffect::ReadOnly,
                ToolRetryPolicy::ReplaySafe,
                ToolDeterminism::EnvironmentDependent,
                10_000,
            ),
            Self::ReadFiles => super::spec(
                self.name(),
                "1",
                super::description(
                    "several independent local files",
                    "one file or a text search is enough.",
                    "a bounded independent batch.",
                    "per-file content and failures.",
                    false,
                ),
                ToolExposure::Direct,
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
                self.output_schema(),
                ToolParallelism::Parallel,
                Some("file:{files[*].path}"),
                ToolEffect::ReadOnly,
                ToolRetryPolicy::ReplaySafe,
                ToolDeterminism::EnvironmentDependent,
                15_000,
            ),
            Self::ListFiles => super::spec(
                self.name(),
                "1",
                super::description(
                    "paths or a bounded tree",
                    "content or text matches are needed.",
                    "an explicit depth.",
                    "paths and truncation.",
                    false,
                ),
                ToolExposure::Direct,
                json!({
                    "type": "object",
                    "properties": {
                        "path": {"type": "string"},
                        "max_depth": {"type": "integer", "minimum": 0, "maximum": 64}
                    },
                    "additionalProperties": false
                }),
                self.output_schema(),
                ToolParallelism::Parallel,
                Some("file:{path}"),
                ToolEffect::ReadOnly,
                ToolRetryPolicy::ReplaySafe,
                ToolDeterminism::EnvironmentDependent,
                15_000,
            ),
        }
    }

    pub(super) fn normalize(
        self,
        registry: &ToolRegistry,
        arguments: Value,
    ) -> Result<(Value, Vec<ResourceKey>), ToolExecutorError> {
        let project_root = &registry.project_root;
        match self {
            Self::ReadFile => normalize_read_file(project_root, arguments),
            Self::ReadFiles => normalize_read_files(project_root, arguments),
            Self::ListFiles => normalize_list_files(project_root, arguments),
        }
    }

    pub(super) async fn execute(
        self,
        registry: &ToolRegistry,
        plan: ToolExecutionPlan,
    ) -> ToolExecutionOutcome {
        match self {
            Self::ReadFile => execute_read_file(&registry.project_root, plan).await,
            Self::ReadFiles => execute_read_files(&registry.project_root, plan).await,
            Self::ListFiles => execute_list_files(&registry.project_root, plan).await,
        }
    }

    fn output_schema(self) -> Value {
        match self {
            Self::ReadFile => read_file_output_schema(),
            Self::ReadFiles => json!({
                "type": "object",
                "properties": {
                    "files": {"type": "array", "minItems": 1, "maxItems": 32, "items": {"oneOf": [
                        {"type": "object", "properties": {
                            "status": {"type": "string", "const": "succeeded"},
                            "result": read_file_output_schema()
                        }, "required": ["status", "result"], "additionalProperties": false},
                        {"type": "object", "properties": {
                            "status": {"type": "string", "const": "failed"},
                            "result": file_read_failure_schema()
                        }, "required": ["status", "result"], "additionalProperties": false}
                    ]}},
                    "partial_failure": json!({"type": "boolean"})
                }, "required": ["files", "partial_failure"], "additionalProperties": false
            }),
            Self::ListFiles => json!({
                "type": "object", "properties": {
                    "path": json!({"type": "string", "minLength": 1}), "paths": {"type": "string"},
                    "path_count": json!({"type": "integer", "minimum": 0}), "truncated": json!({"type": "boolean"})
                }, "required": ["path", "paths", "path_count", "truncated"], "additionalProperties": false
            }),
        }
    }
}

fn read_file_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "path": {"type": "string", "minLength": 1},
            "content": {"type": "string"},
            "original_bytes": {"type": "integer", "minimum": 0},
            "selected_bytes": {"type": "integer", "minimum": 0},
            "truncated": {"type": "boolean"},
            "start_line": {"type": ["integer", "null"], "minimum": 1},
            "end_line": {"type": ["integer", "null"], "minimum": 1}
        },
        "required": ["path", "content", "original_bytes", "selected_bytes", "truncated", "start_line", "end_line"],
        "additionalProperties": false
    })
}

fn file_read_failure_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "error": {"type": "string", "minLength": 1},
            "path": {"type": "string", "minLength": 1}
        },
        "required": ["error", "path"],
        "additionalProperties": false
    })
}
