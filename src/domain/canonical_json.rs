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

/// Hash one array without allocating a second copy of all of its elements.
pub(crate) fn canonical_array_hash(values: &[impl Serialize]) -> Result<String, serde_json::Error> {
    let hashes = canonical_array_prefix_hashes(values, &BTreeSet::from([values.len()]))?;
    Ok(hashes[&values.len()].clone())
}

/// Incremental form of the same canonical array hash, for a single ordered
/// replay. Snapshots close a cloned hasher and never mutate the prefix.
pub(crate) struct CanonicalArrayHasher {
    hasher: blake3::Hasher,
    len: usize,
}

impl CanonicalArrayHasher {
    pub(crate) fn new() -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"[");
        Self { hasher, len: 0 }
    }

    pub(crate) fn push(&mut self, value: &impl Serialize) -> Result<(), serde_json::Error> {
        if self.len != 0 {
            self.hasher.update(b",");
        }
        let value = serde_json::to_value(value)?;
        self.hasher
            .update(&serde_json::to_vec(&canonical_json(&value))?);
        self.len += 1;
        Ok(())
    }

    pub(crate) fn finish(&self) -> String {
        let mut hash = self.hasher.clone();
        hash.update(b"]");
        hash.finalize().to_hex().to_string()
    }
}

/// Hash selected array prefixes while encoding each element only once. The
/// closing bracket is added to a cloned hasher, preserving the ordinary array
/// fingerprint without rescanning archived event bodies for every watermark.
pub(crate) fn canonical_array_prefix_hashes(
    values: &[impl Serialize],
    lengths: &BTreeSet<usize>,
) -> Result<BTreeMap<usize, String>, serde_json::Error> {
    let mut hashes = BTreeMap::new();
    let mut hasher = CanonicalArrayHasher::new();
    for length in 0..=values.len() {
        if lengths.contains(&length) {
            hashes.insert(length, hasher.finish());
        }
        if let Some(value) = values.get(length) {
            hasher.push(value)?;
        }
    }
    Ok(hashes)
}
