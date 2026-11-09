//! Source-bound task-context reads through the fixed scoped Port.

use serde_json::{Value, json};

use super::{ToolRegistry, decode_arguments, invalid_arguments, invalid_plan};
use crate::domain::context::ContextWindowPolicy;
use crate::domain::tools::{
    ResourceKey, ToolDeterminism, ToolEffect, ToolExecutionOutcome, ToolExecutionPlan,
    ToolExposure, ToolParallelism, ToolRetryPolicy, ToolSpec,
};
use crate::ports::context_read::ContextReadRequest;
use crate::ports::tool::ToolExecutorError;

pub(super) const NAME: &str = "context_read";

pub(super) fn normalize(
    mut value: Value,
    policy: ContextWindowPolicy,
) -> Result<(Value, Vec<ResourceKey>), ToolExecutorError> {
    if let Some(arguments) = value.as_object_mut() {
        arguments
            .entry("max_bytes")
            .or_insert(json!(policy.readback_default_bytes()));
    }
    let args: ContextReadRequest = decode_arguments(NAME, value)?;
    let (text, max_bytes) = match &args {
        ContextReadRequest::Search {
            query,
            after_unix,
            before_unix,
            max_bytes,
            ..
        } => {
            if after_unix
                .zip(*before_unix)
                .is_some_and(|(after, before)| after > before)
            {
                return Err(invalid_arguments(
                    NAME,
                    "after_unix must not exceed before_unix".to_owned(),
                ));
            }
            (query, *max_bytes)
        }
        ContextReadRequest::Read {
            reference,
            max_bytes,
            ..
        } => (reference, *max_bytes),
        ContextReadRequest::Progress {
            task_ref,
            max_bytes,
        } => (task_ref, *max_bytes),
    };
    if text.trim().is_empty()
        || text.len() > 4096
        || max_bytes == 0
        || max_bytes > policy.readback_max_bytes()
    {
        return Err(invalid_arguments(
            NAME,
            format!(
                "reference/query must be 1..=4096 bytes and max_bytes must be 1..={}",
                policy.readback_max_bytes()
            ),
        ));
    }
    Ok((
        serde_json::to_value(args).expect("normalized context_read arguments serialize"),
        Vec::new(),
    ))
}

pub(super) async fn execute(
    registry: &ToolRegistry,
    plan: ToolExecutionPlan,
) -> ToolExecutionOutcome {
    let (normalized, _) = match normalize(plan.call.arguments.clone(), registry.window_policy) {
        Ok(value) => value,
        Err(error) => return invalid_plan(plan, error),
    };
    let request: ContextReadRequest =
        serde_json::from_value(normalized).expect("normalized context_read arguments");
    let Some(reader) = &registry.context_reader else {
        return ToolExecutionOutcome::failed(
            plan,
            "context_read_unavailable",
            json!({"error":"no Session-bound context reader"}),
        );
    };
    match reader.read(request) {
        Ok(response) => ToolExecutionOutcome::succeeded(
            plan,
            serde_json::to_value(response).expect("context read response serializes"),
        ),
        Err(error) => ToolExecutionOutcome::failed(
            plan,
            "context_read_failed",
            json!({"error":error.to_string()}),
        ),
    }
}

pub(super) fn spec(registry: &ToolRegistry) -> ToolSpec {
    let maximum = registry.window_policy.readback_max_bytes();
    super::spec(
        NAME,
        "1",
        super::description(
            "searching prior task context, expanding a known source reference, or reading task progress",
            "a current file, a raw process output Blob, or a Memory document path should be read directly.",
            "precise references and source ranges; verify time-sensitive facts through their current source.",
            "scoped source references, original role/origin/authority, revisions, observed times, ranges, truncation and found/not_found/stale/ambiguous status. Source content retains its recorded authority. At most four unfinished searches may hold cursors; when this limit is reached, complete an existing search before starting another.",
            false,
        ),
        ToolExposure::Direct,
        json!({"type":"object","properties":{
            "operation":{"type":"string","enum":["search","read","progress"]},
            "query":{"type":"string","minLength":1},"reference":{"type":"string","minLength":1},
            "task_ref":{"type":"string","minLength":1},"offset":{"type":"integer","minimum":0},
            "after_unix":{"type":["integer","null"],"minimum":0},"before_unix":{"type":["integer","null"],"minimum":0},
            "cursor":{"type":["string","null"]},
            "max_bytes":{"type":"integer","minimum":1,"maximum":maximum}
        },"required":["operation"],"additionalProperties":false}),
        json!({"type":"object","properties":{
            "status":{"type":"string","enum":["found","not_found","stale","ambiguous"]},
            "entries":{"type":"array","items":{"type":"object","properties":{
                "reference":{"type":"string"},"scope":{"type":"string"},"source_revision":{"type":"string"},
                "source_role":{"type":["string","null"],"enum":["user","assistant",null]},
                "source_origin":{"type":"string","enum":["prompt_resource","operator_input","project_file","project_memory","research_projection","session_projection","artifact_projection","tool_result","model_output","replay_fixture"]},
                "semantic_class":{"type":"string","enum":["instruction","authoritative_reference","derived_projection","model_observation","conversation","narrative"]},
                "source_authority":{"type":"string","enum":["fact_reference","operator_assertion","derived_non_evidence","model_observation","unclassified_legacy"]},
                "observed_at":{"type":["string","null"]},"valid_time":{"type":["string","null"]},
                "offset":{"type":"integer"},"next_offset":{"type":["integer","null"]},"total_bytes":{"type":"integer"},
                "text":{"type":"string"},"truncated":{"type":"boolean"}
            },"required":["reference","scope","source_revision","source_role","source_origin","semantic_class","source_authority","observed_at","valid_time","offset","next_offset","total_bytes","text","truncated"],"additionalProperties":false}},
            "truncated":{"type":"boolean"},"next_cursor":{"type":["string","null"]}
        },"required":["status","entries","truncated","next_cursor"],"additionalProperties":false}),
        ToolParallelism::Parallel,
        None,
        ToolEffect::ReadOnly,
        ToolRetryPolicy::ReplaySafe,
        ToolDeterminism::EnvironmentDependent,
        5_000,
    )
}
