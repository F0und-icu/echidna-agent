//! The single recursive JSON normalization and BLAKE3 encoding owner for Domain hashes.

use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

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

/// Hash selected array prefixes while encoding each element only once. The
/// closing bracket is added to a cloned hasher, preserving the ordinary array
/// fingerprint without rescanning archived event bodies for every watermark.
pub(crate) fn canonical_array_prefix_hashes(
    values: &[impl Serialize],
    lengths: &BTreeSet<usize>,
) -> Result<BTreeMap<usize, String>, serde_json::Error> {
    let mut hashes = BTreeMap::new();
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"[");
    for length in 0..=values.len() {
        if lengths.contains(&length) {
            let mut prefix = hasher.clone();
            prefix.update(b"]");
            hashes.insert(length, prefix.finalize().to_hex().to_string());
        }
        if let Some(value) = values.get(length) {
            if length != 0 {
                hasher.update(b",");
            }
            let value = serde_json::to_value(value)?;
            hasher.update(&serde_json::to_vec(&canonical_json(&value))?);
        }
    }
    Ok(hashes)
}
