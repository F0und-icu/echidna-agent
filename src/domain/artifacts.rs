//! Minimal provider-neutral Artifact identity contract.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A stable Artifact identifier shared by model, research, and storage slices.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ArtifactId(String);

impl ArtifactId {
    /// Constructs an artifact reference without loading its content.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the artifact identifier as persisted.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ArtifactId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}
