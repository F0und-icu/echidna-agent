use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use super::records::ArtifactRecord;
use super::reducer::ArtifactError;

/// The first independently versioned Artifact payload shape.
pub const ARTIFACT_SCHEMA_VERSION: u16 = 1;

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
            artifact_schema_version: ARTIFACT_SCHEMA_VERSION,
            record: self.record().clone(),
        };
        serde_json::to_value(payload).map_err(|error| ArtifactEventError::MalformedPayload {
            message: error.to_string(),
        })
    }

    pub fn decode(event_type: &str, payload: Value) -> Result<Self, ArtifactEventError> {
        let payload: VersionedArtifactPayload =
            serde_json::from_value(payload).map_err(|error| {
                ArtifactEventError::MalformedPayload {
                    message: error.to_string(),
                }
            })?;
        if payload.artifact_schema_version != ARTIFACT_SCHEMA_VERSION {
            return Err(ArtifactEventError::UnsupportedPayloadVersion {
                received: payload.artifact_schema_version,
            });
        }
        let event = match event_type {
            "artifact.created" => Self::Created(payload.record),
            "artifact.derived" => Self::Derived(payload.record),
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
    artifact_schema_version: u16,
    record: ArtifactRecord,
}
