//! Fixed discovery Tool for bounded Deferred Tool schemas.

use crate::domain::tools::{ToolDeterminism, ToolEffect, ToolParallelism, ToolRetryPolicy};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::domain::tools::{ToolExecutionOutcome, ToolExecutionPlan, ToolExposure, ToolSpec};
use crate::ports::tool::ToolExecutorError;

use super::{decode_arguments, invalid_plan};

const MAX_RESULTS: usize = 16;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ToolSearchArgs {
    query: String,
    #[serde(default = "default_max_results")]
    max_results: usize,
}

pub(super) fn normalize(
    value: Value,
) -> Result<(Value, Vec<crate::domain::tools::ResourceKey>), ToolExecutorError> {
    let arguments: ToolSearchArgs = decode_arguments("tool_search", value)?;
    validate(&arguments)?;
    Ok((
        serde_json::to_value(arguments).expect("ToolSearchArgs serialization cannot fail"),
        Vec::new(),
    ))
}

pub(super) fn execute(plan: ToolExecutionPlan, specs: &[ToolSpec]) -> ToolExecutionOutcome {
    let arguments: ToolSearchArgs =
        match decode_arguments("tool_search", plan.call.arguments.clone()) {
            Ok(arguments) => arguments,
            Err(error) => {
                return invalid_plan(plan, error);
            }
        };
    if let Err(error) = validate(&arguments) {
        return invalid_plan(plan, error);
    }

    let normalized_query = normalize_query(&arguments.query);
    let terms = normalized_query.split(' ').collect::<Vec<_>>();
    let candidates = specs
        .iter()
        .filter(|spec| spec.exposure != ToolExposure::Hidden)
        .collect::<Vec<_>>();
    let exact_name =
        |spec: &&ToolSpec| terms.contains(&spec.name.as_str().to_ascii_lowercase().as_str());
    let has_exact_name = candidates.iter().any(exact_name);
    let mut matches = candidates
        .into_iter()
        .filter(|spec| {
            if has_exact_name {
                return exact_name(spec);
            }
            let searchable = format!(
                "{} {}",
                spec.name.as_str().to_ascii_lowercase(),
                spec.description.to_ascii_lowercase()
            );
            terms.iter().all(|term| searchable.contains(term))
        })
        .collect::<Vec<_>>();
    matches.sort_by(|left, right| left.name.cmp(&right.name));
    let truncated = matches.len() > arguments.max_results;
    matches.truncate(arguments.max_results);
    ToolExecutionOutcome::succeeded(
        plan,
        json!({
            "query": normalized_query,
            "matches": matches.iter().map(|spec| json!({
                "name": spec.name,
                "description": spec.description,
                "exposure": spec.exposure,
                "input_schema": spec.input_schema,
                "output_schema": spec.output_schema,
            })).collect::<Vec<_>>(),
            "truncated": truncated,
        }),
    )
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "query": {"type": "string", "minLength": 1,
                "description": "Prefer an exact tool-name token, optionally with descriptive words. Exact eligible names take precedence; otherwise every whitespace-separated term must match the name or description."},
            "max_results": {"type": "integer", "minimum": 1, "maximum": MAX_RESULTS}
        },
        "required": ["query"],
        "additionalProperties": false
    })
}

fn default_max_results() -> usize {
    MAX_RESULTS
}

fn validate(arguments: &ToolSearchArgs) -> Result<(), ToolExecutorError> {
    if normalize_query(&arguments.query).is_empty() {
        return Err(ToolExecutorError::InvalidArguments {
            tool_name: "tool_search".to_owned(),
            message: "`query` must contain non-whitespace text".to_owned(),
        });
    }
    if !(1..=MAX_RESULTS).contains(&arguments.max_results) {
        return Err(ToolExecutorError::InvalidArguments {
            tool_name: "tool_search".to_owned(),
            message: format!("`max_results` must be between 1 and {MAX_RESULTS}"),
        });
    }
    Ok(())
}

fn normalize_query(query: &str) -> String {
    query
        .split_whitespace()
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) const NAME: &str = "tool_search";

pub(super) fn spec() -> ToolSpec {
    super::spec(
        NAME,
        "2",
        super::description(
            "find Deferred tools",
            "a Tool is Hidden or already Direct.",
            "an exact tool-name token when known, otherwise a few precise keywords. A tool absent from inspect_project.tool_capabilities may still be Deferred and discoverable.",
            "eligible matching schemas; activation lasts only for the rest of the current Turn.",
            false,
        ),
        ToolExposure::Direct,
        input_schema(),
        output_schema(),
        ToolParallelism::Parallel,
        None,
        ToolEffect::ReadOnly,
        ToolRetryPolicy::ReplaySafe,
        ToolDeterminism::Deterministic,
        5_000,
    )
}

fn output_schema() -> Value {
    json!({
        "type": "object", "properties": {"query": json!({"type": "string", "minLength": 1}), "matches": {"type": "array", "maxItems": 16, "items": {"type": "object", "properties": {"name": json!({"type": "string", "minLength": 1}), "description": json!({"type": "string", "minLength": 1}), "exposure": {"type": "string", "enum": ["direct", "deferred", "hidden"]}, "input_schema": {"type": "object"}, "output_schema": {"type": "object"}}, "required": ["name", "description", "exposure", "input_schema", "output_schema"], "additionalProperties": false}}, "truncated": json!({"type": "boolean"})},
        "required": ["query", "matches", "truncated"], "additionalProperties": false
    })
}
