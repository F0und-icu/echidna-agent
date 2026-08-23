//! Canonical Context fingerprint encoding.

use serde::Serialize;

use crate::domain::canonical_json::canonical_blake3_hash;

use super::error::ContextPlannerError;

/// The Context fingerprint owner reuses Domain's one JSON-normalization rule.
pub(super) fn hash(value: &impl Serialize) -> Result<String, ContextPlannerError> {
    canonical_blake3_hash(value).map_err(|error| ContextPlannerError::CanonicalEncoding {
        message: error.to_string(),
    })
}
