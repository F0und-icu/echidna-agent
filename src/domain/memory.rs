//! Typed project-Memory revisions and pure replay state.
//!
//! Markdown remains the authority for Memory content. This module only
//! describes the audited revision facts which are appended to the existing
//! Research stream. It deliberately has no filesystem, Blob-store, clock, or
//! Research-record state access.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::domain::artifacts::BlobId;

pub(crate) const MEMORY_PAYLOAD_SCHEMA_VERSION: u16 = 1;

/// A deliberately small, project-relative Markdown location.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct MemoryPath(String);

impl MemoryPath {
    /// Parses only the durable project-Memory paths supported by this slice.
    pub fn parse(value: impl Into<String>) -> Result<Self, MemoryError> {
        let value = value.into();
        let valid_fixed = matches!(
            value.as_str(),
            "core.md" | "decisions.md" | "failed-paths.md"
        );
        let valid_note = value.strip_prefix("notes/").is_some_and(valid_note_name);
        if valid_fixed || valid_note {
            Ok(Self(value))
        } else {
            Err(MemoryError::InvalidPath { path: value })
        }
    }

    /// Returns the persisted project-relative Markdown path.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for MemoryPath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

impl fmt::Display for MemoryPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// One immutable before/after Markdown revision audited in `research.jsonl`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryRevision {
    pub path: MemoryPath,
    #[serde(default)]
    pub old_hash: Option<String>,
    #[serde(default)]
    pub new_hash: Option<String>,
    #[serde(default)]
    pub old_blob_id: Option<BlobId>,
    #[serde(default)]
    pub new_blob_id: Option<BlobId>,
    #[serde(default)]
    pub diff_blob_id: Option<BlobId>,
}

impl MemoryRevision {
    pub(crate) fn validate(&self) -> Result<(), MemoryError> {
        validate_content_hash("old_hash", self.old_hash.as_deref())?;
        validate_content_hash("new_hash", self.new_hash.as_deref())?;
        validate_blob("old_blob_id", self.old_blob_id.as_ref())?;
        validate_blob("new_blob_id", self.new_blob_id.as_ref())?;
        validate_blob("diff_blob_id", self.diff_blob_id.as_ref())?;
        if self.old_hash.is_none() != self.old_blob_id.is_none() {
            return Err(MemoryError::InvalidRevision {
                message: "old Hash and old Blob must either both exist or both be absent"
                    .to_owned(),
            });
        }
        if self.new_hash.is_none() != self.new_blob_id.is_none() {
            return Err(MemoryError::InvalidRevision {
                message: "new Hash and new Blob must either both exist or both be absent"
                    .to_owned(),
            });
        }
        if self.old_hash == self.new_hash {
            return Err(MemoryError::InvalidRevision {
                message: "Memory revision must change content presence or Hash".to_owned(),
            });
        }
        if self.diff_blob_id.is_none() {
            return Err(MemoryError::InvalidRevision {
                message: "Memory revision requires an auditable Diff Blob".to_owned(),
            });
        }
        Ok(())
    }
}

/// Typed Memory audit facts stored in the existing Research stream.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    content = "revision",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum MemoryEvent {
    Changed(MemoryRevision),
    ExternalChangeDetected(MemoryRevision),
}

impl MemoryEvent {
    pub(crate) const fn event_type(&self) -> MemoryEventType {
        match self {
            Self::Changed(_) => MemoryEventType::Changed,
            Self::ExternalChangeDetected(_) => MemoryEventType::ExternalChangeDetected,
        }
    }

    pub(crate) fn revision(&self) -> &MemoryRevision {
        match self {
            Self::Changed(revision) | Self::ExternalChangeDetected(revision) => revision,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), MemoryError> {
        self.revision().validate()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MemoryEventType {
    Changed,
    ExternalChangeDetected,
}

/// The isolated projection used only to compare Markdown with its last audit.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct MemoryState {
    latest: BTreeMap<MemoryPath, MemoryRevision>,
}

impl MemoryState {
    pub(crate) fn latest(&self, path: &MemoryPath) -> Option<&MemoryRevision> {
        self.latest.get(path)
    }
}

/// The sole pure Memory-state reducer. Research replay validates the outer
/// stream and Event identity separately; this reducer never owns Research
/// records or Event identity state.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct MemoryReducer;

impl MemoryReducer {
    pub(crate) fn apply(
        &self,
        state: &mut MemoryState,
        event: MemoryEvent,
    ) -> Result<(), MemoryError> {
        event.validate()?;
        let revision = event.revision();
        if let Some(previous) = state.latest.get(&revision.path) {
            if revision.old_hash != previous.new_hash
                || revision.old_blob_id != previous.new_blob_id
            {
                return Err(MemoryError::InvalidRevision {
                    message: format!(
                        "revision chain for `{}` does not continue from the previous revision",
                        revision.path
                    ),
                });
            }
        } else if revision.old_hash.is_some() || revision.old_blob_id.is_some() {
            return Err(MemoryError::InvalidRevision {
                message: format!(
                    "first revision for `{}` must not reference previous content",
                    revision.path
                ),
            });
        }
        state.latest.insert(revision.path.clone(), revision.clone());
        Ok(())
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum MemoryError {
    #[error("unsupported Memory path `{path}`")]
    InvalidPath { path: String },
    #[error("unsupported Memory payload version `{received}`")]
    UnsupportedPayloadVersion { received: u16 },
    #[error("Memory Event payload is malformed: {message}")]
    MalformedPayload { message: String },
    #[error("Memory revision is invalid: {message}")]
    InvalidRevision { message: String },
}

fn valid_note_name(value: &str) -> bool {
    !value.is_empty()
        && value.ends_with(".md")
        && value != ".md"
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}

pub(crate) fn validate_content_hash(label: &str, value: Option<&str>) -> Result<(), MemoryError> {
    if value.is_none_or(|value| {
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    }) {
        Ok(())
    } else {
        Err(MemoryError::InvalidRevision {
            message: format!("{label} must be a lowercase BLAKE3 digest"),
        })
    }
}

fn validate_blob(label: &str, value: Option<&BlobId>) -> Result<(), MemoryError> {
    match value {
        None => Ok(()),
        Some(value) => BlobId::new(value.as_str().to_owned())
            .map(|_| ())
            .map_err(|error| MemoryError::InvalidRevision {
                message: format!("{label} is invalid: {error}"),
            }),
    }
}
