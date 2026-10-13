use std::collections::BTreeMap;

use serde::Serialize;
use thiserror::Error;

use super::records::{ArtifactId, ArtifactRecord};

/// Pure Artifact/Lineage validation errors.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ArtifactError {
    #[error("Artifact record is invalid: {message}")]
    InvalidRecord { message: String },
    #[error("Artifact ID `{id}` already exists")]
    DuplicateArtifactId { id: String },
    #[error("Artifact parent `{parent}` does not exist")]
    MissingParent { parent: String },
}

/// The deterministic Lineage projection derived from Artifact Events.
#[derive(Debug, Default, Serialize)]
pub(crate) struct ArtifactState {
    records: BTreeMap<ArtifactId, ArtifactRecord>,
}

impl ArtifactState {
    pub(crate) fn validate_record(&self, record: &ArtifactRecord) -> Result<(), ArtifactError> {
        record.validate()?;
        if self.records.contains_key(&record.id) {
            return Err(ArtifactError::DuplicateArtifactId {
                id: record.id.to_string(),
            });
        }
        for parent in &record.parent_artifact_ids {
            let parent_record =
                self.records
                    .get(parent)
                    .ok_or_else(|| ArtifactError::MissingParent {
                        parent: parent.to_string(),
                    })?;
            if let Some(target_snapshot_id) = record.scope.target_snapshot_id()
                && parent_record.scope.target_snapshot_id() != Some(target_snapshot_id)
            {
                return Err(ArtifactError::InvalidRecord {
                    message: format!(
                        "target-scoped Artifact parent `{parent}` does not share TargetSnapshot `{target_snapshot_id}`"
                    ),
                });
            }
        }
        Ok(())
    }

    pub(crate) fn insert(&mut self, record: ArtifactRecord) {
        self.records.insert(record.id.clone(), record);
    }

    pub(crate) fn get(&self, id: &ArtifactId) -> Option<&ArtifactRecord> {
        self.records.get(id)
    }

    /// Bounded callers project metadata only; Blob bytes stay in the Artifact
    /// Store Adapter and are never exposed by this fact projection.
    pub(crate) fn records(&self) -> impl Iterator<Item = &ArtifactRecord> {
        self.records.values()
    }
}
