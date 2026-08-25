//! Strict bridge from the fixed `view_image` result to model follow-up.

use serde_json::Value;
use thiserror::Error;

use crate::domain::artifacts::ArtifactId;
use crate::domain::tools::{ToolResult, ToolResultStatus};

pub const VIEW_IMAGE_SELECTION_SCHEMA_VERSION: u16 = 1;

/// The immutable source and delivery Artifact identity emitted by the fixed
/// `view_image` Tool. Ordinary Artifact Tool Results cannot become images.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ViewImageSelection {
    pub source_artifact_id: ArtifactId,
    pub delivery_artifact_id: ArtifactId,
    pub derived_artifact_ids: Vec<ArtifactId>,
}

impl ViewImageSelection {
    /// Returns `None` for every Tool Result except a successful `view_image`.
    /// A malformed successful result is a typed failure, never an omitted
    /// image.
    pub fn from_tool_result(result: &ToolResult) -> Result<Option<Self>, ViewImageSelectionError> {
        if result.tool_name.as_str() != "view_image" || result.status != ToolResultStatus::Succeeded
        {
            return Ok(None);
        }
        let object = result
            .output
            .as_object()
            .ok_or(ViewImageSelectionError::OutputObjectRequired)?;
        let schema_version = object
            .get("schema_version")
            .and_then(Value::as_u64)
            .ok_or(ViewImageSelectionError::SchemaVersionRequired)?;
        if schema_version != u64::from(VIEW_IMAGE_SELECTION_SCHEMA_VERSION) {
            return Err(ViewImageSelectionError::UnsupportedSchemaVersion { schema_version });
        }
        let source_artifact_id = artifact_at(&result.output, "/original/artifact/id", "original")?;
        let delivery_artifact_id =
            artifact_at(&result.output, "/image_artifact_ref", "image_artifact_ref")?;
        let output_delivery_id = artifact_at(
            &result.output,
            "/delivery/artifact_id",
            "delivery.artifact_id",
        )?;
        if output_delivery_id != delivery_artifact_id {
            return Err(ViewImageSelectionError::DeliveryReferenceMismatch {
                selected: delivery_artifact_id,
                delivery: output_delivery_id,
            });
        }
        match result.output.pointer("/derived") {
            Some(Value::Null) | None => {
                if delivery_artifact_id != source_artifact_id {
                    return Err(ViewImageSelectionError::OriginalDeliveryMismatch {
                        source_artifact: source_artifact_id,
                        delivery: delivery_artifact_id,
                    });
                }
                Ok(Some(Self {
                    source_artifact_id,
                    delivery_artifact_id,
                    derived_artifact_ids: Vec::new(),
                }))
            }
            Some(Value::Object(_)) => {
                let derived_artifact_id =
                    artifact_at(&result.output, "/derived/artifact/id", "derived")?;
                if delivery_artifact_id != derived_artifact_id {
                    return Err(ViewImageSelectionError::DerivedDeliveryMismatch {
                        derived: derived_artifact_id,
                        delivery: delivery_artifact_id,
                    });
                }
                let transform = result
                    .output
                    .pointer("/derived/artifact/transform")
                    .and_then(Value::as_str)
                    .ok_or(ViewImageSelectionError::DerivedTransformRequired)?;
                if transform != "image_budget_v1" {
                    return Err(ViewImageSelectionError::UnsupportedDerivedTransform {
                        transform: transform.to_owned(),
                    });
                }
                let parents = result
                    .output
                    .pointer("/derived/artifact/parent_artifact_ids")
                    .and_then(Value::as_array)
                    .ok_or(ViewImageSelectionError::DerivedParentsRequired)?;
                if parents.len() != 1
                    || parents.first().and_then(Value::as_str) != Some(source_artifact_id.as_str())
                {
                    return Err(ViewImageSelectionError::DerivedParentMismatch {
                        source_artifact: source_artifact_id,
                    });
                }
                Ok(Some(Self {
                    source_artifact_id,
                    delivery_artifact_id,
                    derived_artifact_ids: vec![derived_artifact_id],
                }))
            }
            Some(_) => Err(ViewImageSelectionError::DerivedShapeInvalid),
        }
    }
}

fn artifact_at(
    output: &Value,
    pointer: &str,
    field: &'static str,
) -> Result<ArtifactId, ViewImageSelectionError> {
    let value = output
        .pointer(pointer)
        .and_then(Value::as_str)
        .ok_or(ViewImageSelectionError::ArtifactReferenceRequired { field })?;
    if value.trim().is_empty() {
        return Err(ViewImageSelectionError::ArtifactReferenceRequired { field });
    }
    Ok(ArtifactId::new(value))
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ViewImageSelectionError {
    #[error("successful view_image output must be a JSON object")]
    OutputObjectRequired,
    #[error("successful view_image output requires an integer schema_version")]
    SchemaVersionRequired,
    #[error("successful view_image output uses unsupported schema version {schema_version}")]
    UnsupportedSchemaVersion { schema_version: u64 },
    #[error("successful view_image output requires non-empty Artifact reference `{field}`")]
    ArtifactReferenceRequired { field: &'static str },
    #[error(
        "view_image selected Artifact `{selected}` differs from delivery Artifact `{delivery}`"
    )]
    DeliveryReferenceMismatch {
        selected: ArtifactId,
        delivery: ArtifactId,
    },
    #[error(
        "view_image without a derivative must deliver source Artifact `{source_artifact}`, not `{delivery}`"
    )]
    OriginalDeliveryMismatch {
        source_artifact: ArtifactId,
        delivery: ArtifactId,
    },
    #[error("view_image derived Artifact `{derived}` differs from delivery Artifact `{delivery}`")]
    DerivedDeliveryMismatch {
        derived: ArtifactId,
        delivery: ArtifactId,
    },
    #[error("view_image derived Artifact requires transform image_budget_v1")]
    DerivedTransformRequired,
    #[error("view_image derived Artifact uses unsupported transform `{transform}`")]
    UnsupportedDerivedTransform { transform: String },
    #[error("view_image derived Artifact requires one source parent")]
    DerivedParentsRequired,
    #[error(
        "view_image derived Artifact must retain source Artifact `{source_artifact}` as its only parent"
    )]
    DerivedParentMismatch { source_artifact: ArtifactId },
    #[error("view_image derived Artifact has an invalid output shape")]
    DerivedShapeInvalid,
}
