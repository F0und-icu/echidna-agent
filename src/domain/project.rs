//! Pure project identity contracts.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A stable identifier for one isolated Cyber Agent project.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ProjectId(String);

impl ProjectId {
    /// Constructs a project identity from an externally supplied value.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the stored identifier without changing its representation.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProjectId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}
