//! The single recursive JSON normalization and BLAKE3 encoding owner for Domain hashes.

use serde::Serialize;
use serde_json::Value;

/// Preserves array order and scalar representation while recursively sorting
/// object keys. Callers own their enclosing semantic fingerprint fields.
pub(crate) fn canonical_json(value: &Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.iter().map(canonical_json).collect()),
        Value::Object(values) => {
            let mut entries = values
                .iter()
                .map(|(key, value)| (key.clone(), canonical_json(value)))
                .collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            Value::Object(entries.into_iter().collect())
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => value.clone(),
    }
}

pub(crate) fn canonical_blake3_hash(value: &impl Serialize) -> Result<String, serde_json::Error> {
    let value = serde_json::to_value(value)?;
    let bytes = serde_json::to_vec(&canonical_json(&value))?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}
