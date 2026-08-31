//! The sole owner of Research normalization and deterministic fingerprints.

use serde::Serialize;
use serde_json::Value;

use crate::domain::canonical_json::canonical_json;

use super::records::{
    ExperimentCapsuleRecord, ExperimentRecord, FindingRecord, HypothesisRecord,
    TargetSnapshotRecord,
};
use super::reducer::ResearchError;

/// Normalizes the stable textual fields that define Research identity.
///
/// This intentionally does not normalize arbitrary action argument strings:
/// command and payload argument case may be semantically significant. Object
/// keys are canonicalized separately before they contribute to a fingerprint.
pub(crate) fn normalize_identity(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace('\\', "/")
        .to_lowercase()
}

pub(crate) fn normalize_action(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Computes the exact identity used to detect equivalent Hypotheses and
/// Findings. The hash avoids treating display title wording as identity.
pub fn dedup_key(
    target_id: &str,
    vulnerability_class: &str,
    entry_point: &str,
    sensitive_sink: &str,
) -> Result<String, ResearchError> {
    fingerprint(
        "dedup",
        &DedupInput {
            target_id: normalize_identity(target_id),
            vulnerability_class: normalize_identity(vulnerability_class),
            entry_point: normalize_identity(entry_point),
            sensitive_sink: normalize_identity(sensitive_sink),
        },
    )
}

/// Computes a target-only fingerprint. Capture time, notes, and analysis-tool
/// version are deliberately absent.
pub fn snapshot_fingerprint(record: &TargetSnapshotRecord) -> Result<String, ResearchError> {
    let mut target_artifact_ids = record
        .target_artifact_ids
        .iter()
        .map(|id| id.as_str())
        .collect::<Vec<_>>();
    target_artifact_ids.sort_unstable();
    target_artifact_ids.dedup();
    fingerprint(
        "snapshot",
        &SnapshotInput {
            target_id: normalize_identity(&record.target_id),
            kind: record.kind,
            locator: normalize_identity(&record.locator),
            version: normalize_identity(&record.version),
            build_id: normalize_identity(&record.build_id),
            target_artifact_ids,
            environment_fingerprint: normalize_identity(&record.environment_fingerprint),
            device_fingerprint: normalize_identity(&record.device_fingerprint),
        },
    )
}

/// Computes the exact executable Research action identity. A TargetSnapshot
/// and tool-version change each affect this fingerprint independently.
pub fn action_fingerprint(
    experiment: &ExperimentRecord,
    capsule: &ExperimentCapsuleRecord,
) -> Result<String, ResearchError> {
    action_fingerprint_from_parts(ActionFingerprintParts {
        hypothesis_id: experiment.hypothesis_id.as_str(),
        target_snapshot_id: experiment.target_snapshot_id.as_str(),
        normalized_action: &capsule.normalized_action,
        normalized_arguments: &capsule.normalized_arguments,
        tool_version: &capsule.tool_version,
        input_artifact_ids: capsule
            .input_artifact_ids
            .iter()
            .map(|id| id.as_str())
            .collect(),
        environment_delta: &capsule.environment_delta,
        account_prerequisites: capsule
            .account_prerequisites
            .iter()
            .map(String::as_str)
            .collect(),
        toolchain_fingerprint: &capsule.toolchain_fingerprint,
    })
}

/// The one action-identity normalization and fingerprint owner. Callers must
/// provide typed identity parts; this function owns sorting, deduplication,
/// text normalization, canonical JSON, and the `action-` namespace.
pub(crate) fn action_fingerprint_from_parts(
    parts: ActionFingerprintParts<'_>,
) -> Result<String, ResearchError> {
    let mut input_artifact_ids = parts.input_artifact_ids;
    input_artifact_ids.sort_unstable();
    input_artifact_ids.dedup();
    let mut account_prerequisites = parts
        .account_prerequisites
        .into_iter()
        .map(normalize_action)
        .collect::<Vec<_>>();
    account_prerequisites.sort_unstable();
    account_prerequisites.dedup();
    fingerprint(
        "action",
        &ActionInput {
            hypothesis_id: parts.hypothesis_id,
            target_snapshot_id: parts.target_snapshot_id,
            normalized_action: normalize_action(parts.normalized_action),
            normalized_arguments: canonical_json(parts.normalized_arguments),
            tool_version: normalize_action(parts.tool_version),
            input_artifact_ids,
            environment_delta: normalize_action(parts.environment_delta),
            account_prerequisites,
            toolchain_fingerprint: normalize_action(parts.toolchain_fingerprint),
        },
    )
}

pub(crate) struct ActionFingerprintParts<'a> {
    pub(crate) hypothesis_id: &'a str,
    pub(crate) target_snapshot_id: &'a str,
    pub(crate) normalized_action: &'a str,
    pub(crate) normalized_arguments: &'a Value,
    pub(crate) tool_version: &'a str,
    pub(crate) input_artifact_ids: Vec<&'a str>,
    pub(crate) environment_delta: &'a str,
    pub(crate) account_prerequisites: Vec<&'a str>,
    pub(crate) toolchain_fingerprint: &'a str,
}

pub(crate) fn validate_dedup(record: &HypothesisRecord) -> Result<(), ResearchError> {
    let actual = dedup_key(
        &record.target_id,
        &record.vulnerability_class,
        &record.entry_point,
        &record.sensitive_sink,
    )?;
    ensure_fingerprint("Hypothesis dedup_key", &record.dedup_key, &actual)
}

pub(crate) fn validate_finding_dedup(record: &FindingRecord) -> Result<(), ResearchError> {
    let actual = dedup_key(
        &record.target_id,
        &record.vulnerability_class,
        &record.entry_point,
        &record.sensitive_sink,
    )?;
    ensure_fingerprint("Finding dedup_key", &record.dedup_key, &actual)
}

pub(crate) fn validate_snapshot(record: &TargetSnapshotRecord) -> Result<(), ResearchError> {
    let actual = snapshot_fingerprint(record)?;
    ensure_fingerprint(
        "TargetSnapshot snapshot_fingerprint",
        &record.snapshot_fingerprint,
        &actual,
    )
}

pub(crate) fn validate_action(
    experiment: &ExperimentRecord,
    capsule: &ExperimentCapsuleRecord,
) -> Result<(), ResearchError> {
    let actual = action_fingerprint(experiment, capsule)?;
    ensure_fingerprint(
        "Experiment action_fingerprint",
        &experiment.action_fingerprint,
        &actual,
    )
}

fn ensure_fingerprint(subject: &str, expected: &str, actual: &str) -> Result<(), ResearchError> {
    if expected == actual {
        Ok(())
    } else {
        Err(ResearchError::InvalidRecord {
            message: format!("{subject} does not match its deterministic value"),
        })
    }
}

fn fingerprint<T: Serialize>(namespace: &str, value: &T) -> Result<String, ResearchError> {
    let bytes = serde_json::to_vec(value).map_err(|error| ResearchError::Fingerprint {
        message: error.to_string(),
    })?;
    Ok(format!("{namespace}-{}", blake3::hash(&bytes).to_hex()))
}

#[derive(Serialize)]
struct DedupInput {
    target_id: String,
    vulnerability_class: String,
    entry_point: String,
    sensitive_sink: String,
}

#[derive(Serialize)]
struct SnapshotInput<'a> {
    target_id: String,
    kind: super::records::TargetKind,
    locator: String,
    version: String,
    build_id: String,
    target_artifact_ids: Vec<&'a str>,
    environment_fingerprint: String,
    device_fingerprint: String,
}

#[derive(Serialize)]
struct ActionInput<'a> {
    hypothesis_id: &'a str,
    target_snapshot_id: &'a str,
    normalized_action: String,
    normalized_arguments: Value,
    tool_version: String,
    input_artifact_ids: Vec<&'a str>,
    environment_delta: String,
    account_prerequisites: Vec<String>,
    toolchain_fingerprint: String,
}
