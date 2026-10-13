use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use super::records::{ArtifactId, ArtifactRecord, ArtifactScope, BlobId};
use super::reducer::ArtifactError;
use crate::domain::events::EventId;
use crate::domain::research::TargetSnapshotId;
use crate::domain::tools::ToolCallId;

/// The first independently versioned Artifact payload shape.
pub const ARTIFACT_SCHEMA_VERSION: u16 = 2;

/// Versioned Artifact facts carried by the existing Research Event stream.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArtifactEvent {
    Created(ArtifactRecord),
    Derived(ArtifactRecord),
}

impl ArtifactEvent {
    pub const fn event_type(&self) -> &'static str {
        match self {
            Self::Created(_) => "artifact.created",
            Self::Derived(_) => "artifact.derived",
        }
    }

    pub fn record(&self) -> &ArtifactRecord {
        match self {
            Self::Created(record) | Self::Derived(record) => record,
        }
    }

    pub fn encode_payload(&self) -> Result<Value, ArtifactEventError> {
        let payload = VersionedArtifactPayload {
            _artifact_schema_version: ARTIFACT_SCHEMA_VERSION,
            record: self.record().clone(),
        };
        serde_json::to_value(payload).map_err(|error| ArtifactEventError::MalformedPayload {
            message: error.to_string(),
        })
    }

    /// Encodes the stable semantic form used for project-stream Event identity.
    ///
    /// Target-scoped records retain their v1 semantic payload so historical
    /// target Artifacts and their v2 scoped representation remain the same
    /// immutable fact. Session-scoped records have no v1 equivalent.
    pub(crate) fn encode_semantic_payload(&self) -> Result<Value, ArtifactEventError> {
        match &self.record().scope {
            ArtifactScope::Target { target_snapshot_id } => {
                let payload = LegacyVersionedArtifactPayload {
                    _artifact_schema_version: 1,
                    record: LegacyArtifactRecord::from_target(self.record(), target_snapshot_id),
                };
                serde_json::to_value(payload).map_err(|error| {
                    ArtifactEventError::MalformedPayload {
                        message: error.to_string(),
                    }
                })
            }
            ArtifactScope::Session { .. } => self.encode_payload(),
        }
    }

    pub fn decode(event_type: &str, payload: Value) -> Result<Self, ArtifactEventError> {
        let schema_version = payload
            .get("artifact_schema_version")
            .and_then(Value::as_u64)
            .ok_or_else(|| ArtifactEventError::MalformedPayload {
                message: "Artifact Event payload is missing artifact_schema_version".to_owned(),
            })?;
        let record = match schema_version {
            1 => serde_json::from_value::<LegacyVersionedArtifactPayload>(payload)
                .map(|payload| payload.record.into_current())
                .map_err(|error| ArtifactEventError::MalformedPayload {
                    message: error.to_string(),
                })?,
            value if value == u64::from(ARTIFACT_SCHEMA_VERSION) => {
                serde_json::from_value::<VersionedArtifactPayload>(payload)
                    .map(|payload| payload.record)
                    .map_err(|error| ArtifactEventError::MalformedPayload {
                        message: error.to_string(),
                    })?
            }
            value => {
                return Err(ArtifactEventError::UnsupportedPayloadVersion {
                    received: u16::try_from(value).unwrap_or(u16::MAX),
                });
            }
        };
        let event = match event_type {
            "artifact.created" => Self::Created(record),
            "artifact.derived" => Self::Derived(record),
            _ => {
                return Err(ArtifactEventError::UnknownEventType {
                    event_type: event_type.to_owned(),
                });
            }
        };
        event.validate()?;
        Ok(event)
    }

    pub(crate) fn validate(&self) -> Result<(), ArtifactEventError> {
        self.record().validate()?;
        if matches!(self, Self::Created(_)) && self.record().is_derived() {
            return Err(ArtifactEventError::KindMismatch {
                event_type: self.event_type().to_owned(),
                message: "created Artifact cannot have parents".to_owned(),
            });
        }
        if matches!(self, Self::Derived(_)) && !self.record().is_derived() {
            return Err(ArtifactEventError::KindMismatch {
                event_type: self.event_type().to_owned(),
                message: "derived Artifact requires parents and transform".to_owned(),
            });
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ArtifactEventError {
    #[error("unsupported Artifact Event type `{event_type}`")]
    UnknownEventType { event_type: String },
    #[error("unsupported Artifact payload version `{received}`")]
    UnsupportedPayloadVersion { received: u16 },
    #[error("Artifact Event payload is malformed: {message}")]
    MalformedPayload { message: String },
    #[error("Artifact Event `{event_type}` is invalid: {message}")]
    KindMismatch { event_type: String, message: String },
    #[error(transparent)]
    Artifact(#[from] ArtifactError),
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct VersionedArtifactPayload {
    #[serde(rename = "artifact_schema_version")]
    _artifact_schema_version: u16,
    record: ArtifactRecord,
}

/// The exact v1 Record shape remains decodable so historical payload bytes
/// retain their original semantic hashes and target interpretation.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LegacyVersionedArtifactPayload {
    #[serde(rename = "artifact_schema_version")]
    _artifact_schema_version: u16,
    record: LegacyArtifactRecord,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LegacyArtifactRecord {
    id: ArtifactId,
    blob_id: BlobId,
    media_type: String,
    size_bytes: u64,
    source_path: String,
    created_by_event_id: EventId,
    produced_by_tool_call_id: ToolCallId,
    target_snapshot_id: TargetSnapshotId,
    toolchain_fingerprint: String,
    #[serde(default)]
    parent_artifact_ids: Vec<ArtifactId>,
    #[serde(default)]
    transform: Option<String>,
}

impl LegacyArtifactRecord {
    fn from_target(record: &ArtifactRecord, target_snapshot_id: &TargetSnapshotId) -> Self {
        Self {
            id: record.id.clone(),
            blob_id: record.blob_id.clone(),
            media_type: record.media_type.clone(),
            size_bytes: record.size_bytes,
            source_path: record.source_path.clone(),
            created_by_event_id: record.created_by_event_id.clone(),
            produced_by_tool_call_id: record.produced_by_tool_call_id.clone(),
            target_snapshot_id: target_snapshot_id.clone(),
            toolchain_fingerprint: record.toolchain_fingerprint.clone(),
            parent_artifact_ids: record.parent_artifact_ids.clone(),
            transform: record.transform.clone(),
        }
    }

    fn into_current(self) -> ArtifactRecord {
        ArtifactRecord {
            id: self.id,
            blob_id: self.blob_id,
            media_type: self.media_type,
            size_bytes: self.size_bytes,
            source_path: self.source_path,
            created_by_event_id: self.created_by_event_id,
            produced_by_tool_call_id: self.produced_by_tool_call_id,
            scope: ArtifactScope::Target {
                target_snapshot_id: self.target_snapshot_id,
            },
            toolchain_fingerprint: self.toolchain_fingerprint,
            parent_artifact_ids: self.parent_artifact_ids,
            transform: self.transform,
        }
    }
}
