//! Bounded exact-byte readback through the existing local Blob owner.

use super::ToolRegistry;
use crate::domain::tools::{
    ToolDeterminism, ToolEffect, ToolExposure, ToolParallelism, ToolRetryPolicy, ToolSpec,
};

use super::{decode_arguments, invalid_arguments, invalid_plan};
use crate::adapters::{artifact_store::LocalArtifactStore, project_event_store::Project};
use crate::domain::context::ContextWindowPolicy;
use crate::domain::{
    artifacts::BlobId,
    tools::{ResourceKey, ToolExecutionOutcome, ToolExecutionPlan},
};
use crate::ports::tool::ToolExecutorError;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;

fn default_limit() -> usize {
    ContextWindowPolicy::for_window(32768).readback_default_bytes()
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Args {
    blob_id: BlobId,
    #[serde(default)]
    offset: u64,
    #[serde(default = "default_limit")]
    max_bytes: usize,
}

pub(super) fn normalize(
    mut value: Value,
    policy: ContextWindowPolicy,
) -> Result<(Value, Vec<ResourceKey>), ToolExecutorError> {
    if let Some(arguments) = value.as_object_mut() {
        arguments
            .entry("max_bytes")
            .or_insert(json!(policy.readback_default_bytes()));
    }
    let args: Args = decode_arguments("read_output", value)?;
    if args.max_bytes == 0 || args.max_bytes > policy.readback_max_bytes() {
        return Err(invalid_arguments(
            "read_output",
            format!("max_bytes must be 1..={}", policy.readback_max_bytes()),
        ));
    }
    Ok((
        serde_json::to_value(args).expect("read_output args"),
        Vec::new(),
    ))
}

pub(super) async fn execute(
    root: &Path,
    plan: ToolExecutionPlan,
    policy: ContextWindowPolicy,
) -> ToolExecutionOutcome {
    let (normalized, _) = match normalize(plan.call.arguments.clone(), policy) {
        Ok(value) => value,
        Err(error) => return invalid_plan(plan, error),
    };
    let args: Args = serde_json::from_value(normalized).expect("normalized output args");
    let read = Project::discover(root)
        .map_err(|error| error.to_string())
        .and_then(|project| {
            LocalArtifactStore::for_project(&project)
                .read_output_range(&args.blob_id, args.offset, args.max_bytes)
                .map_err(|error| error.to_string())
        });
    match read {
        Ok((bytes, size)) => ToolExecutionOutcome::succeeded(
            plan,
            json!({
                "blob_id":args.blob_id, "offset":args.offset,
                "next_offset":args.offset + bytes.len() as u64, "stored_bytes":size,
                "data_base64":STANDARD.encode(&bytes), "text":String::from_utf8_lossy(&bytes),
                "text_lossy":std::str::from_utf8(&bytes).is_err(),
                "has_more":args.offset + (bytes.len() as u64) < size
            }),
        ),
        Err(error) => {
            ToolExecutionOutcome::failed(plan, "output_read_failed", json!({"error":error}))
        }
    }
}

fn input_schema() -> Value {
    let max_read_bytes = ContextWindowPolicy::for_window(32768).readback_max_bytes();
    json!({"type":"object","properties":{
        "blob_id":{"type":"string","pattern":"^[0-9a-f]{64}$"},
        "offset":{"type":"integer","minimum":0},
        "max_bytes":{"type":"integer","minimum":1,"maximum":max_read_bytes}
    },"required":["blob_id"],"additionalProperties":false})
}

fn output_schema() -> Value {
    json!({"type":"object","properties":{
        "blob_id":{"type":"string","pattern":"^[0-9a-f]{64}$"},
        "offset":{"type":"integer","minimum":0},"next_offset":{"type":"integer","minimum":0},
        "stored_bytes":{"type":"integer","minimum":0},
        "data_base64":{"type":"string"},"text":{"type":"string"},
        "text_lossy":{"type":"boolean"},"has_more":{"type":"boolean"}
    },"required":["blob_id","offset","next_offset","stored_bytes","data_base64","text","text_lossy","has_more"],"additionalProperties":false})
}

pub(super) const NAME: &str = "read_output";

pub(super) fn spec(registry: &ToolRegistry) -> ToolSpec {
    let mut spec = super::spec(
        NAME,
        "1",
        super::description(
            "reading a retained process-output Blob beyond its preview",
            "a current file or running Job/PTY output is needed.",
            "the exact output_captures Blob ID and byte offset; do not rerun a completed command for truncated output.",
            "a bounded text view and exact base64 bytes; raw capture is not verified evidence.",
            false,
        ),
        ToolExposure::Deferred,
        input_schema(),
        output_schema(),
        ToolParallelism::Parallel,
        None,
        ToolEffect::ReadOnly,
        ToolRetryPolicy::ReplaySafe,
        ToolDeterminism::EnvironmentDependent,
        5_000,
    );
    spec.input_schema["properties"]["max_bytes"]["maximum"] =
        json!(registry.window_policy.readback_max_bytes());
    spec
}
