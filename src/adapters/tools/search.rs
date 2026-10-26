//! Bounded ripgrep-backed single and multi-pattern file search.

use crate::domain::tools::{
    ToolDeterminism, ToolEffect, ToolExposure, ToolParallelism, ToolRetryPolicy, ToolSpec,
};

use std::path::Path;

use futures_util::future::join_all;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::process::Command;

use crate::domain::tools::{ResourceKey, ToolExecutionOutcome, ToolExecutionPlan};
use crate::ports::tool::ToolExecutorError;

use super::filesystem::resolve_path;
use super::output::bounded_text;
use super::resources::ResourceKeyResolver;
use super::{decode_arguments, invalid_arguments, invalid_plan};

const MAX_PATTERNS: usize = 32;
const MAX_PATHS: usize = 32;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SearchFilesArgs {
    #[serde(default)]
    pattern: Option<String>,
    #[serde(default)]
    patterns: Vec<String>,
    #[serde(default = "default_search_paths")]
    paths: Vec<String>,
}

fn default_search_paths() -> Vec<String> {
    vec![".".to_owned()]
}

pub(super) fn normalize(
    project_root: &Path,
    value: Value,
) -> Result<(Value, Vec<ResourceKey>), ToolExecutorError> {
    let arguments: SearchFilesArgs = decode_arguments("search_files", value)?;
    let patterns = patterns(&arguments)?;
    if arguments.paths.is_empty() || arguments.paths.len() > MAX_PATHS {
        return Err(invalid_arguments(
            "search_files",
            "`paths` must contain between 1 and 32 entries",
        ));
    }
    if patterns.len() > MAX_PATTERNS {
        return Err(invalid_arguments(
            "search_files",
            "`patterns` must contain at most 32 entries",
        ));
    }
    let resolver = ResourceKeyResolver::new(project_root);
    let keys = arguments
        .paths
        .iter()
        .map(|path| resolver.file(path))
        .collect();
    Ok((
        serde_json::to_value(arguments).expect("SearchFilesArgs serialization cannot fail"),
        keys,
    ))
}

pub(super) async fn execute(project_root: &Path, plan: ToolExecutionPlan) -> ToolExecutionOutcome {
    let arguments: SearchFilesArgs =
        match decode_arguments("search_files", plan.call.arguments.clone()) {
            Ok(arguments) => arguments,
            Err(error) => return invalid_plan(plan, error),
        };
    let patterns = match patterns(&arguments) {
        Ok(patterns)
            if !arguments.paths.is_empty()
                && arguments.paths.len() <= MAX_PATHS
                && patterns.len() <= MAX_PATTERNS =>
        {
            patterns
        }
        Ok(_) => {
            return invalid_plan(
                plan,
                invalid_arguments("search_files", "invalid `paths` or `patterns` batch size"),
            );
        }
        Err(error) => return invalid_plan(plan, error),
    };

    let max_each = per_item_limit(plan.output_policy.max_bytes, patterns.len());
    let searches = join_all(
        patterns
            .iter()
            .map(|pattern| search(project_root, pattern, &arguments.paths, max_each)),
    )
    .await;
    if arguments.pattern.is_some() {
        let Some(result) = searches.into_iter().next() else {
            return invalid_plan(
                plan,
                invalid_arguments(
                    "search_files",
                    "the single pattern was lost during execution",
                ),
            );
        };
        return match result {
            Ok(value) => ToolExecutionOutcome::succeeded(plan, value),
            Err(value) => ToolExecutionOutcome::failed(plan, "search_failed", value),
        };
    }

    let partial_failure = searches.iter().any(Result::is_err);
    let results = searches
        .into_iter()
        .zip(patterns)
        .map(|(result, pattern)| match result {
            Ok(value) => json!({"pattern": pattern, "status": "succeeded", "result": value}),
            Err(value) => json!({"pattern": pattern, "status": "failed", "result": value}),
        })
        .collect::<Vec<_>>();
    ToolExecutionOutcome::succeeded(
        plan,
        json!({"results": results, "partial_failure": partial_failure}),
    )
}

fn patterns(arguments: &SearchFilesArgs) -> Result<Vec<String>, ToolExecutorError> {
    if arguments.pattern.is_some() && !arguments.patterns.is_empty() {
        return Err(invalid_arguments(
            "search_files",
            "provide either `pattern` or `patterns`, not both",
        ));
    }
    let patterns = arguments
        .pattern
        .clone()
        .into_iter()
        .chain(arguments.patterns.clone())
        .collect::<Vec<_>>();
    if patterns.is_empty() || patterns.iter().any(String::is_empty) {
        return Err(invalid_arguments(
            "search_files",
            "a non-empty `pattern` or `patterns` entry is required",
        ));
    }
    Ok(patterns)
}

async fn search(
    project_root: &Path,
    pattern: &str,
    paths: &[String],
    max_bytes: usize,
) -> Result<Value, Value> {
    let mut command = Command::new("rg");
    command
        .arg("--line-number")
        .arg("--no-heading")
        .arg("--color=never")
        .arg("--")
        .arg(pattern)
        .args(paths.iter().map(|path| resolve_path(project_root, path)))
        .current_dir(project_root)
        .kill_on_drop(true);
    match command.output().await {
        Ok(output) if output.status.success() || output.status.code() == Some(1) => {
            let bounded = bounded_text(&output.stdout, max_bytes);
            Ok(json!({
                "matches": bounded.text,
                "original_bytes": bounded.original_bytes,
                "truncated": bounded.truncated,
            }))
        }
        Ok(output) => Err(failure_value(
            output.status.code(),
            &output.stdout,
            &output.stderr,
            max_bytes,
        )),
        Err(error) => Err(json!({"error": error.to_string()})),
    }
}

fn failure_value(
    exit_code: Option<i32>,
    stdout_bytes: &[u8],
    stderr_bytes: &[u8],
    max_bytes: usize,
) -> Value {
    let max_each = max_bytes / 2;
    let stdout = bounded_text(stdout_bytes, max_each);
    let stderr = bounded_text(stderr_bytes, max_each);
    json!({
        "exit_code": exit_code,
        "stdout": stdout.text,
        "stderr": stderr.text,
        "truncated": stdout.truncated || stderr.truncated,
    })
}

fn per_item_limit(total: usize, count: usize) -> usize {
    total / count.max(1)
}

pub(super) const NAME: &str = "search_files";

pub(super) fn spec() -> ToolSpec {
    super::spec(
        NAME,
        "1",
        super::description(
            "bounded text matches in local files",
            "file content or a command result is needed.",
            "narrow patterns.",
            "matches, bytes and truncation.",
            false,
        ),
        ToolExposure::Direct,
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
                "paths": {"type": "array", "minItems": 1, "maxItems": 32, "items": {"type": "string"}}
            },
            "oneOf": [
                {"required": ["pattern"], "not": {"required": ["patterns"]}},
                {"required": ["patterns"], "not": {"required": ["pattern"]}}
            ],
            "additionalProperties": false
        }),
        output_schema(),
        ToolParallelism::Parallel,
        Some("file:{paths}"),
        ToolEffect::ReadOnly,
        ToolRetryPolicy::ReplaySafe,
        ToolDeterminism::EnvironmentDependent,
        30_000,
    )
}

fn output_schema() -> Value {
    json!({
        "oneOf": [
            search_result_schema(),
            {"type": "object", "properties": {
                "results": {"type": "array", "minItems": 1, "maxItems": 32, "items": {"oneOf": [
                    {"type": "object", "properties": {
                        "pattern": json!({"type": "string", "minLength": 1}), "status": {"type": "string", "const": "succeeded"},
                        "result": search_result_schema()
                    }, "required": ["pattern", "status", "result"], "additionalProperties": false},
                    {"type": "object", "properties": {
                        "pattern": json!({"type": "string", "minLength": 1}), "status": {"type": "string", "const": "failed"},
                        "result": search_failure_schema()
                    }, "required": ["pattern", "status", "result"], "additionalProperties": false}
                ]}},
                "partial_failure": json!({"type": "boolean"})
            }, "required": ["results", "partial_failure"], "additionalProperties": false}
        ]
    })
}

fn search_result_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "matches": {"type": "string"},
            "original_bytes": {"type": "integer", "minimum": 0},
            "truncated": {"type": "boolean"}
        },
        "required": ["matches", "original_bytes", "truncated"],
        "additionalProperties": false
    })
}

fn search_failure_schema() -> Value {
    json!({
        "oneOf": [
            {
                "type": "object",
                "properties": {"error": {"type": "string", "minLength": 1}},
                "required": ["error"],
                "additionalProperties": false
            },
            {
                "type": "object",
                "properties": {
                    "exit_code": {"type": ["integer", "null"]},
                    "stdout": {"type": "string"},
                    "stderr": {"type": "string"},
                    "truncated": {"type": "boolean"}
                },
                "required": ["exit_code", "stdout", "stderr", "truncated"],
                "additionalProperties": false
            }
        ]
    })
}
