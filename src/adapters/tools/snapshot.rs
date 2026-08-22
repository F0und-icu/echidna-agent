//! Fixed project inspection Tool backed by the local rebuildable cache.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::adapters::project_event_store::Project;
use crate::adapters::project_snapshot::{CacheDisposition, LocalProjectSnapshot};
use crate::domain::tools::{ResourceKey, ToolExecutionOutcome, ToolExecutionPlan, ToolSpec};
use crate::ports::tool::ToolExecutorError;

use super::resources::ResourceKeyResolver;
use super::{decode_arguments, invalid_plan};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct InspectProjectArgs {
    observed_at: String,
    #[serde(default)]
    hash_paths: Vec<String>,
}

pub(super) fn normalize(
    project_root: &Path,
    value: Value,
) -> Result<(Value, Vec<ResourceKey>), ToolExecutorError> {
    let arguments: InspectProjectArgs = decode_arguments("inspect_project", value)?;
    if arguments.observed_at.trim().is_empty() {
        return Err(ToolExecutorError::InvalidArguments {
            tool_name: "inspect_project".to_owned(),
            message: "`observed_at` must not be empty".to_owned(),
        });
    }
    Ok((
        serde_json::to_value(arguments).expect("InspectProjectArgs serialization cannot fail"),
        vec![ResourceKeyResolver::new(project_root).file(".cyber-agent/cache")],
    ))
}

pub(super) async fn execute(
    project_root: &Path,
    specs: &[ToolSpec],
    plan: ToolExecutionPlan,
) -> ToolExecutionOutcome {
    let arguments: InspectProjectArgs =
        match decode_arguments("inspect_project", plan.call.arguments.clone()) {
            Ok(arguments) => arguments,
            Err(error) => return invalid_plan(plan, error),
        };
    if arguments.observed_at.trim().is_empty() {
        return ToolExecutionOutcome::failed(
            plan,
            "invalid_execution_plan",
            json!({"error": "`observed_at` must not be empty"}),
        );
    }
    let project = match Project::discover(project_root) {
        Ok(project) => project,
        Err(error) => return snapshot_failure(plan, "snapshot_project_unavailable", error),
    };
    let _writer = match project.open_writer() {
        Ok(writer) => writer,
        Err(error) => return snapshot_failure(plan, "snapshot_writer_unavailable", error),
    };
    let result = match LocalProjectSnapshot::new(&project).inspect(
        specs,
        &arguments.hash_paths,
        &arguments.observed_at,
    ) {
        Ok(result) => result,
        Err(error) => return snapshot_failure(plan, "snapshot_inspection_failed", error),
    };
    let (status, rebuild_reason) = match &result.disposition {
        CacheDisposition::Hit => ("hit", None),
        CacheDisposition::IncrementalRefresh { reason } => {
            ("incremental_refresh", Some(reason.clone()))
        }
        CacheDisposition::FullRebuild { reason } => ("full_rebuild", Some(reason.clone())),
    };
    ToolExecutionOutcome::succeeded(
        plan,
        json!({
            "status": status,
            "rebuild_reason": rebuild_reason,
            "snapshot": result.snapshot,
            "file_manifest": result.manifest,
            "tool_capabilities": result.tool_capabilities,
            "hashed_paths": result.hashed_paths,
        }),
    )
}

fn snapshot_failure(
    plan: ToolExecutionPlan,
    code: &str,
    error: impl ToString,
) -> ToolExecutionOutcome {
    ToolExecutionOutcome::failed(
        plan,
        code,
        json!({"error_code": code, "error": error.to_string()}),
    )
}

pub(super) fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "observed_at": {"type": "string", "minLength": 1},
            "hash_paths": {"type": "array", "items": {"type": "string"}}
        },
        "required": ["observed_at"],
        "additionalProperties": false
    })
}

pub(super) fn output_schema() -> Value {
    json!({"type": "object"})
}
