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
    #[serde(default)]
    hash_paths: Vec<String>,
}

pub(super) fn normalize(
    project_root: &Path,
    control_directory: &str,
    value: Value,
) -> Result<(Value, Vec<ResourceKey>), ToolExecutorError> {
    let arguments: InspectProjectArgs = decode_arguments("inspect_project", value)?;
    Ok((
        serde_json::to_value(arguments).expect("InspectProjectArgs serialization cannot fail"),
        vec![ResourceKeyResolver::new(project_root).project_file(control_directory, "cache")],
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
    let observed_at = match super::operation_time::current_utc() {
        Ok(value) => value,
        Err(error) => return snapshot_failure(plan, "snapshot_clock_unavailable", error),
    };
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
        &observed_at,
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
            "hash_paths": {"type": "array", "items": {"type": "string"},
                "description": "Project-relative files to read and verify with BLAKE3 now, even on cache hits. Duplicate paths are hashed once. Omit for cheap metadata inspection; unrequested cached hashes are not current-content verification."}
        },
        "additionalProperties": false
    })
}

pub(super) fn output_schema() -> Value {
    json!({
        "type": "object", "properties": {
            "status": {"type": "string", "enum": ["hit", "incremental_refresh", "full_rebuild"]},
            "rebuild_reason": {"type": ["string", "null"]},
            "snapshot": {"type": "object", "properties": {
                "cache_schema_version": {"type": "integer", "const": 1}, "project_id": {"type": "string", "minLength": 1},
                "source_fingerprint": project_fingerprint_schema(), "created_at": {"type": "string", "minLength": 1},
                "rebuild_reason": {"type": "string"}, "file_manifest_hash": {"type": "string", "minLength": 1},
                "tool_capability_hash": {"type": "string", "minLength": 1}, "file_count": {"type": "integer", "minimum": 0}
            }, "required": ["cache_schema_version", "project_id", "source_fingerprint", "created_at", "rebuild_reason", "file_manifest_hash", "tool_capability_hash", "file_count"], "additionalProperties": false},
            "file_manifest": file_manifest_schema(), "tool_capabilities": tool_capability_cache_schema(),
            "hashed_paths": {"type": "array", "items": {"type": "string"},
                "description": "Normalized paths whose bytes were hashed in this invocation; may be nonempty with status hit when verification found no cache change."}
        }, "required": ["status", "rebuild_reason", "snapshot", "file_manifest", "tool_capabilities", "hashed_paths"], "additionalProperties": false
    })
}

fn project_fingerprint_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "kind": {"type": "string", "enum": ["git", "filesystem", "git_unavailable"]},
            "digest": {"type": "string", "minLength": 1},
            "cacheable": {"type": "boolean"}
        },
        "required": ["kind", "digest", "cacheable"],
        "additionalProperties": false
    })
}

fn file_manifest_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "cache_schema_version": {"type": "integer", "const": 1},
            "project_id": {"type": "string", "minLength": 1},
            "source_fingerprint": project_fingerprint_schema(),
            "created_at": {"type": "string", "minLength": 1},
            "rebuild_reason": {"type": "string"},
            "entries": {"type": "array", "items": {
                "type": "object",
                "properties": {
                    "relative_path": {"type": "string"},
                    "kind": {"type": "string", "enum": ["file", "directory", "symlink", "other"]},
                    "size_bytes": {"type": "integer", "minimum": 0},
                    "modified_nanos": {"type": "integer", "minimum": 0},
                    "blake3": {"type": ["string", "null"], "pattern": "^[0-9a-f]{64}$"}
                },
                "required": ["relative_path", "kind", "size_bytes", "modified_nanos", "blake3"],
                "additionalProperties": false
            }}
        },
        "required": ["cache_schema_version", "project_id", "source_fingerprint", "created_at", "rebuild_reason", "entries"],
        "additionalProperties": false
    })
}

fn tool_capability_cache_schema() -> Value {
    json!({
        "description": "Only this Sampling Step's advertised ToolSpecs. Absence does not mean unavailable: Deferred tools can be discovered through tool_search. Not a complete catalog or authorization list.",
        "type": "object",
        "properties": {
            "cache_schema_version": {"type": "integer", "const": 1},
            "project_id": {"type": "string", "minLength": 1},
            "source_fingerprint": project_fingerprint_schema(),
            "created_at": {"type": "string", "minLength": 1},
            "rebuild_reason": {"type": "string"},
            "tools": {"type": "array", "items": {
                "type": "object",
                "properties": {
                    "name": {"type": "string", "minLength": 1},
                    "version": {"type": "string", "minLength": 1},
                    "spec_hash": {"type": "string", "pattern": "^[0-9a-f]{64}$"},
                    "effect": {"type": "string", "enum": ["read_only", "local_mutation", "external_mutation", "interactive"]},
                    "retry_policy": {"type": "string", "enum": ["replay_safe", "dedupe_by_key", "never_auto_retry"]},
                    "determinism": {"type": "string", "enum": ["deterministic", "environment_dependent", "nondeterministic"]},
                    "cache_policy": {"type": "string", "enum": ["none", "content_addressed"]},
                    "input_schema_hash": {"type": "string", "pattern": "^[0-9a-f]{64}$"},
                    "output_schema_hash": {"type": "string", "pattern": "^[0-9a-f]{64}$"}
                },
                "required": ["name", "version", "spec_hash", "effect", "retry_policy", "determinism", "cache_policy", "input_schema_hash", "output_schema_hash"],
                "additionalProperties": false
            }}
        },
        "required": ["cache_schema_version", "project_id", "source_fingerprint", "created_at", "rebuild_reason", "tools"],
        "additionalProperties": false
    })
}
