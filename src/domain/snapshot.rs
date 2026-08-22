//! Pure contracts for rebuildable project inspection caches.
//!
//! Filesystem traversal, Git execution and JSON cache I/O live in the local
//! adapter. These types deliberately describe only serialized derived data.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::domain::project::ProjectId;
use crate::domain::tools::{
    ToolCachePolicy, ToolDeterminism, ToolEffect, ToolRetryPolicy, ToolSpec,
};

pub(crate) const SNAPSHOT_CACHE_SCHEMA_VERSION: u16 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectFingerprintKind {
    Git,
    Filesystem,
    GitUnavailable,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectFingerprint {
    pub kind: ProjectFingerprintKind,
    pub digest: String,
    pub cacheable: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    File,
    Directory,
    Symlink,
    Other,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FileManifestEntry {
    pub relative_path: String,
    pub kind: FileKind,
    pub size_bytes: u64,
    pub modified_nanos: u64,
    #[serde(default)]
    pub blake3: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FileManifest {
    pub cache_schema_version: u16,
    pub project_id: ProjectId,
    pub source_fingerprint: ProjectFingerprint,
    pub created_at: String,
    pub rebuild_reason: String,
    pub entries: Vec<FileManifestEntry>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ToolCapability {
    pub name: String,
    pub version: String,
    pub spec_hash: String,
    pub effect: ToolEffect,
    pub retry_policy: ToolRetryPolicy,
    pub determinism: ToolDeterminism,
    pub cache_policy: ToolCachePolicy,
    pub input_schema_hash: String,
    pub output_schema_hash: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ToolCapabilityCache {
    pub cache_schema_version: u16,
    pub project_id: ProjectId,
    pub source_fingerprint: ProjectFingerprint,
    pub created_at: String,
    pub rebuild_reason: String,
    pub tools: Vec<ToolCapability>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectSnapshot {
    pub cache_schema_version: u16,
    pub project_id: ProjectId,
    pub source_fingerprint: ProjectFingerprint,
    pub created_at: String,
    pub rebuild_reason: String,
    pub file_manifest_hash: String,
    pub tool_capability_hash: String,
    pub file_count: usize,
}

/// Derives the bounded cached Tool view from the fixed runtime Tool specs.
/// The local cache adapter owns its serialized-cache identity and persistence.
pub(crate) fn tool_capabilities(specs: &[ToolSpec]) -> Vec<ToolCapability> {
    let mut capabilities = specs
        .iter()
        .map(|spec| ToolCapability {
            name: spec.name.as_str().to_owned(),
            version: spec.version.clone(),
            spec_hash: canonical_value_hash(
                &serde_json::to_value(spec).expect("ToolSpec serializes"),
            ),
            effect: spec.effect,
            retry_policy: spec.retry_policy,
            determinism: spec.determinism,
            cache_policy: spec.cache_policy,
            input_schema_hash: canonical_value_hash(&spec.input_schema),
            output_schema_hash: canonical_value_hash(&spec.output_schema),
        })
        .collect::<Vec<_>>();
    capabilities.sort_by(|left, right| left.name.cmp(&right.name));
    capabilities
}

fn canonical_value_hash(value: &Value) -> String {
    let canonical = crate::domain::canonical_json::canonical_json(value);
    blake3::hash(&serde_json::to_vec(&canonical).expect("JSON Value serializes"))
        .to_hex()
        .to_string()
}
