//! Fixed local-image Tool backed by the existing Artifact publication owner.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::adapters::artifact_store::LocalArtifactStore;
use crate::adapters::image::{PreparedLocalImage, prepare_local_image};
use crate::domain::image::{ImageDeliveryBudget, ImageInputLimits};
use crate::domain::tools::{ResourceKey, ToolExecutionOutcome, ToolExecutionPlan};
use crate::domain::vision::VIEW_IMAGE_SELECTION_SCHEMA_VERSION;
use crate::ports::artifact::ArtifactStore;
use crate::ports::tool::ToolExecutorError;

use super::artifact::{
    ArtifactStoreArgs, PublishedArtifact, publish_artifact, require_nonempty_arguments,
};
use super::filesystem::resolve_path;
use super::resources::ResourceKeyResolver;
use super::{decode_arguments, invalid_plan};

const INPUT_LIMITS: ImageInputLimits =
    ImageInputLimits::new(32 * 1024 * 1024, 12_000, 12_000, 64_000_000);
const DELIVERY_BUDGET: ImageDeliveryBudget =
    ImageDeliveryBudget::new(4 * 1024 * 1024, 2_048, 2_048, 4_000_000);

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ViewImageArgs {
    event_id: String,
    occurred_at: String,
    artifact_id: String,
    source_path: String,
    target_snapshot_id: String,
    toolchain_fingerprint: String,
}

pub(super) fn normalize(
    project_root: &Path,
    value: Value,
) -> Result<(Value, Vec<ResourceKey>), ToolExecutorError> {
    let arguments: ViewImageArgs = decode_arguments("view_image", value)?;
    require_nonempty_view_arguments(&arguments)?;
    let resolver = ResourceKeyResolver::new(project_root);
    Ok((
        serde_json::to_value(&arguments).expect("ViewImageArgs serialization cannot fail"),
        vec![
            resolver.file(&arguments.source_path),
            resolver.file(".cyber-agent/research.jsonl"),
        ],
    ))
}

pub(super) async fn execute(project_root: &Path, plan: ToolExecutionPlan) -> ToolExecutionOutcome {
    let arguments: ViewImageArgs = match decode_arguments("view_image", plan.call.arguments.clone())
    {
        Ok(arguments) => arguments,
        Err(error) => return invalid_plan(plan, error),
    };
    if let Err(error) = require_nonempty_view_arguments(&arguments) {
        return invalid_plan(plan, error);
    }
    let source = resolve_path(project_root, &arguments.source_path);
    let prepared = match prepare_local_image(&source, INPUT_LIMITS, DELIVERY_BUDGET) {
        Ok(prepared) => prepared,
        Err(error) => return image_failure(plan, "view_image_invalid", error),
    };
    let original_arguments = original_artifact_arguments(&arguments, &prepared);
    let derived_arguments = derived_artifact_arguments(&arguments, &prepared);
    let original = match publish_artifact(project_root, &plan, &original_arguments, |project| {
        LocalArtifactStore::for_project(project).publish_bytes(&prepared.original.bytes)
    }) {
        Ok(publication) => publication,
        Err(error) => return image_failure(plan, error.code, error.message),
    };
    let derived = match (derived_arguments, prepared.derivative.as_ref()) {
        (Some(arguments), Some(image)) => {
            match publish_artifact(project_root, &plan, &arguments, |project| {
                LocalArtifactStore::for_project(project).publish_bytes(&image.bytes)
            }) {
                Ok(publication) => Some(publication),
                Err(error) => {
                    return partial_derivative_failure(plan, &original, error.code, error.message);
                }
            }
        }
        (None, None) => None,
        _ => unreachable!("derivative argument construction follows prepared image state"),
    };
    let delivery = derived.as_ref().unwrap_or(&original);
    let mut output_artifact_ids = vec![original.record.id.clone()];
    if let Some(derived) = &derived {
        output_artifact_ids.push(derived.record.id.clone());
    }
    ToolExecutionOutcome::succeeded_with_artifacts(
        plan,
        json!({
            "schema_version": VIEW_IMAGE_SELECTION_SCHEMA_VERSION,
            "status": "recorded",
            "image_artifact_ref": delivery.record.id,
            "original": publication_json(&original),
            "derived": derived.as_ref().map(publication_json),
            "delivery": {
                "artifact_id": delivery.record.id,
                "media_type": delivery.record.media_type,
                "transform": if derived.is_some() { Some("image_budget_v1") } else { None::<&str> },
            },
        }),
        output_artifact_ids,
    )
}

fn original_artifact_arguments(
    arguments: &ViewImageArgs,
    prepared: &PreparedLocalImage,
) -> ArtifactStoreArgs {
    ArtifactStoreArgs {
        event_id: arguments.event_id.clone(),
        occurred_at: arguments.occurred_at.clone(),
        artifact_id: arguments.artifact_id.clone(),
        source_path: arguments.source_path.clone(),
        media_type: prepared.original.metadata.format.media_type().to_owned(),
        target_snapshot_id: arguments.target_snapshot_id.clone(),
        toolchain_fingerprint: arguments.toolchain_fingerprint.clone(),
        parent_artifact_ids: Vec::new(),
        transform: None,
    }
}

fn derived_artifact_arguments(
    arguments: &ViewImageArgs,
    prepared: &PreparedLocalImage,
) -> Option<ArtifactStoreArgs> {
    prepared
        .derivative
        .as_ref()
        .map(|derived| ArtifactStoreArgs {
            event_id: format!("{}:image_budget_v1", arguments.event_id),
            occurred_at: arguments.occurred_at.clone(),
            artifact_id: format!("{}:image_budget_v1", arguments.artifact_id),
            source_path: arguments.source_path.clone(),
            media_type: derived.metadata.format.media_type().to_owned(),
            target_snapshot_id: arguments.target_snapshot_id.clone(),
            toolchain_fingerprint: arguments.toolchain_fingerprint.clone(),
            parent_artifact_ids: vec![arguments.artifact_id.clone()],
            transform: Some("image_budget_v1".to_owned()),
        })
}

fn publication_json(publication: &PublishedArtifact) -> Value {
    json!({
        "event_id": publication.record.created_by_event_id,
        "artifact": publication.record,
        "blob": {
            "blob_id": publication.blob.blob_id,
            "size_bytes": publication.blob.size_bytes,
            "already_present": publication.blob.already_present,
        },
        "research_event_count": publication.research_event_count,
    })
}

fn require_nonempty_view_arguments(arguments: &ViewImageArgs) -> Result<(), ToolExecutorError> {
    let probe = ArtifactStoreArgs {
        event_id: arguments.event_id.clone(),
        occurred_at: arguments.occurred_at.clone(),
        artifact_id: arguments.artifact_id.clone(),
        source_path: arguments.source_path.clone(),
        // Decoding selects the actual media type before Artifact publication.
        media_type: "image/pending-decoder".to_owned(),
        target_snapshot_id: arguments.target_snapshot_id.clone(),
        toolchain_fingerprint: arguments.toolchain_fingerprint.clone(),
        parent_artifact_ids: Vec::new(),
        transform: None,
    };
    require_nonempty_arguments(&probe)
}

fn image_failure(
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

fn partial_derivative_failure(
    plan: ToolExecutionPlan,
    original: &PublishedArtifact,
    code: &str,
    error: impl ToString,
) -> ToolExecutionOutcome {
    let mut outcome = ToolExecutionOutcome::failed(
        plan,
        code,
        json!({
            "error_code": code,
            "error": error.to_string(),
            "original_artifact_ref": original.record.id,
            "message": "the original Artifact is durable; the derivative was not published",
        }),
    );
    outcome.receipt.output_artifact_ids = vec![original.record.id.clone()];
    outcome
}

pub(super) fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "event_id": {"type": "string", "minLength": 1},
            "occurred_at": {"type": "string", "minLength": 1},
            "artifact_id": {"type": "string", "minLength": 1},
            "source_path": {"type": "string", "minLength": 1},
            "target_snapshot_id": {"type": "string", "minLength": 1},
            "toolchain_fingerprint": {"type": "string", "minLength": 1}
        },
        "required": ["event_id", "occurred_at", "artifact_id", "source_path", "target_snapshot_id", "toolchain_fingerprint"],
        "additionalProperties": false
    })
}

pub(super) fn output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "schema_version": {"type": "integer", "const": VIEW_IMAGE_SELECTION_SCHEMA_VERSION},
            "status": {"type": "string", "const": "recorded"},
            "image_artifact_ref": {"type": "string", "minLength": 1},
            "original": publication_schema(),
            "derived": {"oneOf": [publication_schema(), {"type": "null"}]},
            "delivery": {
                "type": "object", "properties": {
                    "artifact_id": {"type": "string", "minLength": 1},
                    "media_type": {"type": "string", "enum": ["image/png", "image/jpeg"]},
                    "transform": {"type": ["string", "null"], "enum": ["image_budget_v1", null]}
                }, "required": ["artifact_id", "media_type", "transform"], "additionalProperties": false
            }
        },
        "required": ["schema_version", "status", "image_artifact_ref", "original", "derived", "delivery"],
        "additionalProperties": false
    })
}

fn publication_schema() -> Value {
    json!({
        "type": "object", "properties": {
            "event_id": {"type": "string", "minLength": 1},
            "artifact": {"type": "object", "properties": {
                "id": {"type": "string", "minLength": 1}, "blob_id": {"type": "string", "pattern": "^[0-9a-f]{64}$"},
                "media_type": {"type": "string", "enum": ["image/png", "image/jpeg"]}, "size_bytes": {"type": "integer", "minimum": 0},
                "source_path": {"type": "string", "minLength": 1}, "created_by_event_id": {"type": "string", "minLength": 1},
                "produced_by_tool_call_id": {"type": "string", "minLength": 1}, "target_snapshot_id": {"type": "string", "minLength": 1},
                "toolchain_fingerprint": {"type": "string", "minLength": 1}, "parent_artifact_ids": {"type": "array", "items": {"type": "string", "minLength": 1}},
                "transform": {"type": ["string", "null"]}
            }, "required": ["id", "blob_id", "media_type", "size_bytes", "source_path", "created_by_event_id", "produced_by_tool_call_id", "target_snapshot_id", "toolchain_fingerprint", "parent_artifact_ids", "transform"], "additionalProperties": false},
            "blob": {"type": "object", "properties": {"blob_id": {"type": "string", "pattern": "^[0-9a-f]{64}$"}, "size_bytes": {"type": "integer", "minimum": 0}, "already_present": {"type": "boolean"}}, "required": ["blob_id", "size_bytes", "already_present"], "additionalProperties": false},
            "research_event_count": {"type": "integer", "minimum": 0}
        }, "required": ["event_id", "artifact", "blob", "research_event_count"], "additionalProperties": false
    })
}
