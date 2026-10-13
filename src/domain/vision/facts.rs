//! Strict persisted Vision route, observation and failure contracts.

use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;

use crate::domain::artifacts::{ArtifactId, BlobId};
use crate::domain::context::{ModelProfile, ProviderSessionRef, ProviderSessionRole};
use crate::domain::events::EventId;
use crate::domain::profile::{VisionMode, VisionRoute, VisionRouteReason};

pub const VISION_FACT_SCHEMA_VERSION: u16 = 1;
pub const VISION_OBSERVATION_SCHEMA_VERSION: u16 = 3;

macro_rules! vision_id {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

vision_id!(
    VisionObservationId,
    "A stable identity for one targeted Vision analysis."
);

/// Persisted resolution before a selected image can reach any model request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VisionRouteRecord {
    pub schema_version: u16,
    pub route_event_id: EventId,
    pub mode: VisionMode,
    pub route: VisionRoute,
    pub reason: Option<VisionRouteReason>,
    pub source_artifact_ids: Vec<ArtifactId>,
    pub primary_profile: ModelProfile,
    pub vision_profile: Option<ModelProfile>,
    pub primary_capability_hash: String,
    pub vision_capability_hash: Option<String>,
}

impl VisionRouteRecord {
    pub fn validate(&self) -> Result<(), VisionFactError> {
        if self.schema_version != VISION_FACT_SCHEMA_VERSION {
            return Err(VisionFactError::UnsupportedSchemaVersion {
                received: self.schema_version,
            });
        }
        require_nonempty("route_event_id", self.route_event_id.as_str())?;
        require_nonempty("primary_capability_hash", &self.primary_capability_hash)?;
        if self.source_artifact_ids.is_empty() {
            return Err(VisionFactError::RequiredField {
                field: "source_artifact_ids",
            });
        }
        validate_unique_artifacts("source_artifact_ids", &self.source_artifact_ids)?;
        if self.route == VisionRoute::NoImage {
            return Err(VisionFactError::NoImageRouteCannotBeRecorded);
        }
        if self.reason.is_none() {
            return Err(VisionFactError::RequiredField { field: "reason" });
        }
        match (self.mode, self.route, self.reason) {
            (
                VisionMode::Auto,
                VisionRoute::PrimaryDirect,
                Some(VisionRouteReason::PrimaryImageCapable),
            )
            | (
                VisionMode::Auto,
                VisionRoute::SecondaryTool,
                Some(VisionRouteReason::PrimaryImageUnsupportedSecondarySelected),
            )
            | (
                VisionMode::Auto,
                VisionRoute::Rejected,
                Some(VisionRouteReason::NoImageCapableProvider),
            )
            | (
                VisionMode::PrimaryDirect,
                VisionRoute::PrimaryDirect,
                Some(VisionRouteReason::ForcedPrimaryDirect),
            )
            | (
                VisionMode::PrimaryDirect,
                VisionRoute::Rejected,
                Some(VisionRouteReason::NoImageCapableProvider),
            )
            | (
                VisionMode::SecondaryTool,
                VisionRoute::SecondaryTool,
                Some(VisionRouteReason::ForcedSecondaryTool),
            )
            | (
                VisionMode::SecondaryTool,
                VisionRoute::Rejected,
                Some(VisionRouteReason::NoImageCapableProvider),
            )
            | (
                VisionMode::Disabled,
                VisionRoute::Rejected,
                Some(VisionRouteReason::VisionDisabled),
            ) => {}
            _ => return Err(VisionFactError::RouteReasonMismatch),
        }
        match self.route {
            VisionRoute::SecondaryTool
                if self.vision_profile.is_none()
                    || self
                        .vision_capability_hash
                        .as_deref()
                        .is_none_or(str::is_empty) =>
            {
                return Err(VisionFactError::SecondaryRouteMissingVisionIdentity);
            }
            VisionRoute::Rejected
            | VisionRoute::PrimaryDirect
            | VisionRoute::NoImage
            | VisionRoute::SecondaryTool => {}
        }
        Ok(())
    }
}

/// One optional bounded region supplied by a Vision model. Coordinates are
/// observations, not verified target locations.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VisionRegion {
    pub label: String,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl VisionRegion {
    pub(super) fn validate(&self) -> Result<(), VisionFactError> {
        require_nonempty("region.label", &self.label)?;
        if self.width == 0 || self.height == 0 {
            return Err(VisionFactError::InvalidRegion);
        }
        Ok(())
    }
}

/// A structured model-derived observation. Its distinct type intentionally
/// prevents direct use as a verified Research Evidence reference.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VisionObservation {
    pub schema_version: u16,
    pub id: VisionObservationId,
    pub route_event_id: EventId,
    pub source_artifact_id: ArtifactId,
    pub derived_artifact_ids: Vec<ArtifactId>,
    /// Absent only for historical schema v1; never infer legacy geometry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_geometry: Option<super::VisionImageGeometry>,
    pub task_hash: String,
    pub summary: String,
    pub extracted_text: Option<String>,
    pub regions: Vec<VisionRegion>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_present_observations"
    )]
    pub security_relevant_clues: Option<Vec<String>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_present_observations"
    )]
    pub task_observations: Option<Vec<String>>,
    pub uncertainty: Vec<String>,
    pub provider_profile: ModelProfile,
    pub capability_hash: String,
    pub provider_session: ProviderSessionRef,
    pub request_audit_event_id: EventId,
    /// The full logical-input semantic hash from the referenced RequestAudit
    /// manifest. It prevents swapping an otherwise compatible audit.
    pub request_input_hash: String,
    pub raw_observation_blob_id: BlobId,
}

/// Absence selects the other schema's field; a present field must be an
/// array. Explicit null must not hide an incompatible field name.
pub(crate) fn deserialize_present_observations<'de, D>(
    deserializer: D,
) -> Result<Option<Vec<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Vec::<String>::deserialize(deserializer).map(Some)
}

impl VisionObservation {
    pub fn validate(&self) -> Result<(), VisionFactError> {
        if !matches!(
            self.schema_version,
            VISION_FACT_SCHEMA_VERSION | 2 | VISION_OBSERVATION_SCHEMA_VERSION
        ) {
            return Err(VisionFactError::UnsupportedSchemaVersion {
                received: self.schema_version,
            });
        }
        match (self.schema_version, &self.image_geometry) {
            (VISION_FACT_SCHEMA_VERSION, None) => {}
            (2 | VISION_OBSERVATION_SCHEMA_VERSION, Some(geometry)) => geometry.validate(
                &self.source_artifact_id,
                &self.derived_artifact_ids,
                &self.regions,
            )?,
            _ => return Err(VisionFactError::ImageGeometryVersionMismatch),
        }
        require_nonempty("observation_id", self.id.as_str())?;
        require_nonempty("route_event_id", self.route_event_id.as_str())?;
        require_nonempty("source_artifact_id", self.source_artifact_id.as_str())?;
        require_nonempty("task_hash", &self.task_hash)?;
        require_nonempty("summary", &self.summary)?;
        require_nonempty("capability_hash", &self.capability_hash)?;
        require_nonempty(
            "request_audit_event_id",
            self.request_audit_event_id.as_str(),
        )?;
        require_nonempty("request_input_hash", &self.request_input_hash)?;
        BlobId::new(self.raw_observation_blob_id.as_str().to_owned()).map_err(|error| {
            VisionFactError::InvalidBlob {
                message: error.to_string(),
            }
        })?;
        if self.provider_session.role != ProviderSessionRole::Vision {
            return Err(VisionFactError::VisionProviderSessionRequired);
        }
        validate_unique_artifacts("derived_artifact_ids", &self.derived_artifact_ids)?;
        if self.derived_artifact_ids.contains(&self.source_artifact_id) {
            return Err(VisionFactError::SourceArtifactRepeatedAsDerivative {
                artifact: self.source_artifact_id.clone(),
            });
        }
        match (
            self.schema_version,
            &self.security_relevant_clues,
            &self.task_observations,
        ) {
            (1 | 2, Some(clues), None) => {
                validate_nonempty_strings("security_relevant_clues", clues)?
            }
            (3, None, Some(observations)) => {
                validate_nonempty_strings("task_observations", observations)?
            }
            _ => return Err(VisionFactError::ObservationFieldsVersionMismatch),
        }
        validate_nonempty_strings("uncertainty", &self.uncertainty)?;
        for region in &self.regions {
            region.validate()?;
        }
        Ok(())
    }

    /// The one canonical Primary-context representation for a secondary
    /// observation. It retains Artifact references and uncertainty while
    /// making the evidence boundary explicit.
    pub fn primary_context_text(&self) -> Result<String, VisionFactError> {
        self.validate()?;
        let mut value = json!({
            "kind": "vision_observation",
            "schema_version": self.schema_version,
            "observation_id": self.id,
            "source_artifact_id": self.source_artifact_id,
            "derived_artifact_ids": self.derived_artifact_ids,
            "image_geometry": self.image_geometry,
            "region_coordinate_space": if self.image_geometry.is_some() { "delivery_pixels" } else { "unknown_legacy" },
            "task_hash": self.task_hash,
            "summary": self.summary,
            "extracted_text": self.extracted_text,
            "regions": self.regions,
            "uncertainty": self.uncertainty,
            "evidence_status": "unverified_model_observation",
        });
        let (field, observations) = if self.schema_version == 3 {
            ("task_observations", self.task_observations.as_ref())
        } else {
            (
                "security_relevant_clues",
                self.security_relevant_clues.as_ref(),
            )
        };
        value[field] = json!(observations.expect("validated observation has its versioned fields"));
        serde_json::to_string(&crate::domain::canonical_json::canonical_json(&value)).map_err(
            |error| VisionFactError::PrimaryContextEncoding {
                message: error.to_string(),
            },
        )
    }

    /// Consumers can search observation text without reinterpreting legacy
    /// security clues as new task-oriented facts.
    pub fn observation_notes(&self) -> &[String] {
        if self.schema_version == 3 {
            self.task_observations
                .as_deref()
                .expect("validated v3 observation contains task observations")
        } else {
            self.security_relevant_clues
                .as_deref()
                .expect("validated legacy observation contains security clues")
        }
    }
}

/// A durable failed Vision attempt after its request was audited. No provider
/// fallback is encoded by this fact.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VisionAnalysisFailure {
    pub schema_version: u16,
    pub route_event_id: EventId,
    pub source_artifact_id: ArtifactId,
    pub task_hash: String,
    pub provider_profile: ModelProfile,
    pub capability_hash: String,
    pub provider_session: ProviderSessionRef,
    pub request_audit_event_id: EventId,
    pub request_input_hash: String,
    pub error_code: String,
    pub message: String,
}

impl VisionAnalysisFailure {
    pub fn validate(&self) -> Result<(), VisionFactError> {
        if self.schema_version != VISION_FACT_SCHEMA_VERSION {
            return Err(VisionFactError::UnsupportedSchemaVersion {
                received: self.schema_version,
            });
        }
        for (field, value) in [
            ("route_event_id", self.route_event_id.as_str()),
            ("source_artifact_id", self.source_artifact_id.as_str()),
            ("task_hash", &self.task_hash),
            ("capability_hash", &self.capability_hash),
            (
                "request_audit_event_id",
                self.request_audit_event_id.as_str(),
            ),
            ("request_input_hash", &self.request_input_hash),
            ("error_code", &self.error_code),
            ("message", &self.message),
        ] {
            require_nonempty(field, value)?;
        }
        if self.provider_session.role != ProviderSessionRole::Vision {
            return Err(VisionFactError::VisionProviderSessionRequired);
        }
        Ok(())
    }
}

fn validate_unique_artifacts(
    field: &'static str,
    values: &[ArtifactId],
) -> Result<(), VisionFactError> {
    for (index, artifact) in values.iter().enumerate() {
        require_nonempty(field, artifact.as_str())?;
        if values[..index].contains(artifact) {
            return Err(VisionFactError::DuplicateArtifact {
                artifact: artifact.clone(),
            });
        }
    }
    Ok(())
}

fn validate_nonempty_strings(
    field: &'static str,
    values: &[String],
) -> Result<(), VisionFactError> {
    for value in values {
        require_nonempty(field, value)?;
    }
    Ok(())
}

fn require_nonempty(field: &'static str, value: &str) -> Result<(), VisionFactError> {
    if value.trim().is_empty() {
        Err(VisionFactError::RequiredField { field })
    } else {
        Ok(())
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum VisionFactError {
    #[error("unsupported Vision fact schema version {received}")]
    UnsupportedSchemaVersion { received: u16 },
    #[error("Vision field `{field}` must not be empty")]
    RequiredField { field: &'static str },
    #[error("Vision Artifact `{artifact}` is duplicated")]
    DuplicateArtifact { artifact: ArtifactId },
    #[error("source Artifact `{artifact}` cannot also be a derived Artifact")]
    SourceArtifactRepeatedAsDerivative { artifact: ArtifactId },
    #[error("a no-image route cannot be persisted as an image route")]
    NoImageRouteCannotBeRecorded,
    #[error(
        "Vision mode, resolved route and stable reason code do not form a valid resolver result"
    )]
    RouteReasonMismatch,
    #[error("a secondary Vision route requires Vision profile and capability identities")]
    SecondaryRouteMissingVisionIdentity,
    #[error("a Vision observation region must have non-zero dimensions")]
    InvalidRegion,
    #[error("Vision image geometry is inconsistent with its selected Artifacts or dimensions")]
    InvalidImageGeometry,
    #[error("Vision observation v2/v3 requires image geometry; legacy v1 must not invent it")]
    ImageGeometryVersionMismatch,
    #[error("Vision observation fields do not match its schema version")]
    ObservationFieldsVersionMismatch,
    #[error("Vision region {index} exceeds the delivered-image pixel bounds")]
    RegionOutOfBounds { index: usize },
    #[error("a Vision observation requires a ProviderSession with the Vision role")]
    VisionProviderSessionRequired,
    #[error("Vision observation Blob ID is invalid: {message}")]
    InvalidBlob { message: String },
    #[error("failed to render fixed Vision Primary context: {message}")]
    PrimaryContextEncoding { message: String },
}
