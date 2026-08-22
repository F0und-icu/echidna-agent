use std::fmt;

use serde::{Deserialize, Serialize};

use crate::domain::events::EventId;
use crate::domain::research::TargetSnapshotId;
use crate::domain::tools::ToolCallId;

use super::reducer::ArtifactError;

/// A stable Artifact identifier shared by model, research, and storage slices.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ArtifactId(String);

impl ArtifactId {
    /// Constructs an Artifact reference without loading its content.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the Artifact identifier as persisted.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ArtifactId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// The lowercase hexadecimal SHA-256 identity of durable Blob content.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct BlobId(String);

impl BlobId {
    /// Validates one SHA-256 digest that was produced outside Domain I/O.
    pub fn new(value: impl Into<String>) -> Result<Self, ArtifactError> {
        let value = value.into();
        if value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            Ok(Self(value))
        } else {
            Err(ArtifactError::InvalidRecord {
                message: "Blob ID must be a lowercase 64-character SHA-256 digest".to_owned(),
            })
        }
    }

    /// Returns the SHA-256 digest without a storage-path prefix.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for BlobId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// One provenance-preserving use of a deduplicated content Blob.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactRecord {
    pub id: ArtifactId,
    pub blob_id: BlobId,
    pub media_type: String,
    pub size_bytes: u64,
    /// Caller-supplied local provenance label, not Blob identity or content.
    ///
    /// This may be a local absolute path and is never telemetry.
    pub source_path: String,
    pub created_by_event_id: EventId,
    pub produced_by_tool_call_id: ToolCallId,
    pub target_snapshot_id: TargetSnapshotId,
    pub toolchain_fingerprint: String,
    #[serde(default)]
    pub parent_artifact_ids: Vec<ArtifactId>,
    #[serde(default)]
    pub transform: Option<String>,
}

impl ArtifactRecord {
    pub(crate) fn validate(&self) -> Result<(), ArtifactError> {
        require_nonempty("Artifact id", self.id.as_str())?;
        BlobId::new(self.blob_id.as_str().to_owned())?;
        require_nonempty("Artifact media_type", &self.media_type)?;
        require_nonempty("Artifact source_path", &self.source_path)?;
        require_nonempty(
            "Artifact created_by_event_id",
            self.created_by_event_id.as_str(),
        )?;
        require_nonempty(
            "Artifact produced_by_tool_call_id",
            self.produced_by_tool_call_id.as_str(),
        )?;
        require_nonempty(
            "Artifact target_snapshot_id",
            self.target_snapshot_id.as_str(),
        )?;
        require_nonempty(
            "Artifact toolchain_fingerprint",
            &self.toolchain_fingerprint,
        )?;

        let has_parents = !self.parent_artifact_ids.is_empty();
        match (has_parents, self.transform.as_deref()) {
            (true, Some(transform)) => require_nonempty("Artifact transform", transform)?,
            (true, None) => {
                return Err(ArtifactError::InvalidRecord {
                    message: "derived Artifact requires a transform".to_owned(),
                });
            }
            (false, Some(_)) => {
                return Err(ArtifactError::InvalidRecord {
                    message: "root Artifact must not declare a transform".to_owned(),
                });
            }
            (false, None) => {}
        }

        for (index, parent) in self.parent_artifact_ids.iter().enumerate() {
            require_nonempty("Artifact parent id", parent.as_str())?;
            if parent == &self.id {
                return Err(ArtifactError::InvalidRecord {
                    message: "Artifact cannot be its own parent".to_owned(),
                });
            }
            if self.parent_artifact_ids[..index].contains(parent) {
                return Err(ArtifactError::InvalidRecord {
                    message: format!("Artifact parent `{parent}` is duplicated"),
                });
            }
        }
        Ok(())
    }

    pub(crate) fn is_derived(&self) -> bool {
        !self.parent_artifact_ids.is_empty()
    }
}

fn require_nonempty(label: &str, value: &str) -> Result<(), ArtifactError> {
    if value.trim().is_empty() {
        Err(ArtifactError::InvalidRecord {
            message: format!("{label} must not be empty"),
        })
    } else {
        Ok(())
    }
}
