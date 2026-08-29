//! Fixed discovery Tool for bounded Deferred Tool schemas.

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
    let matches = specs
        .iter()
        .filter(|spec| spec.exposure != ToolExposure::Hidden)
        .filter(|spec| {
            let searchable = format!(
                "{} {}",
                spec.name.as_str().to_ascii_lowercase(),
                spec.description.to_ascii_lowercase()
            );
            terms.iter().all(|term| searchable.contains(term))
        })
        .take(arguments.max_results)
        .collect::<Vec<_>>();
    let truncated = specs
        .iter()
        .filter(|spec| spec.exposure != ToolExposure::Hidden)
        .filter(|spec| {
            let searchable = format!(
                "{} {}",
                spec.name.as_str().to_ascii_lowercase(),
                spec.description.to_ascii_lowercase()
            );
            terms.iter().all(|term| searchable.contains(term))
        })
        .count()
        > matches.len();
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

pub(super) fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "query": {"type": "string", "minLength": 1},
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
