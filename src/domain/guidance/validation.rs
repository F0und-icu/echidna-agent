use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use serde_json::Value;

use crate::domain::canonical_json::canonical_blake3_hash;
use crate::domain::research::{
    ActionFingerprintParts, ResearchRecordRef, TargetSnapshotId, action_fingerprint_from_parts,
};

use super::ALGORITHM_VERSION;
use super::contracts::{
    GuidanceCandidateReport, GuidanceError, GuidanceEvalCandidate,
    MAX_ACTION_IDENTITY_INPUT_ARTIFACTS, MAX_ACTION_IDENTITY_PREREQUISITES,
    MAX_ACTION_IDENTITY_STRING_BYTES, MAX_ACTION_IDENTITY_VALUE_COLLECTION,
    MAX_ACTION_IDENTITY_VALUE_DEPTH, MAX_ACTION_IDENTITY_VALUE_NODES, MAX_CANDIDATE_ID_BYTES,
    MAX_DECLARED_GAP_CLOSURES, MAX_EVAL_CANDIDATES, MAX_REFERENCE_ID_BYTES,
};

pub(super) fn validate_eval_candidates(
    candidates: &[GuidanceEvalCandidate],
) -> Result<(), GuidanceError> {
    if candidates.len() > MAX_EVAL_CANDIDATES {
        return Err(GuidanceError::TooManyEvalCandidates {
            actual: candidates.len(),
            maximum: MAX_EVAL_CANDIDATES,
        });
    }
    let mut ids = BTreeSet::new();
    for candidate in candidates {
        if !valid_candidate_id(&candidate.id) {
            return Err(GuidanceError::InvalidCandidateId {
                maximum: MAX_CANDIDATE_ID_BYTES,
            });
        }
        if !ids.insert(candidate.id.as_str()) {
            return Err(GuidanceError::DuplicateCandidateId {
                id: candidate.id.clone(),
            });
        }
        if candidate.target_snapshot_id.as_str().len() > MAX_REFERENCE_ID_BYTES
            || research_reference_id(&candidate.research_id).len() > MAX_REFERENCE_ID_BYTES
        {
            return Err(GuidanceError::ReferenceTooLarge {
                id: candidate.id.clone(),
                maximum: MAX_REFERENCE_ID_BYTES,
            });
        }
        if candidate.expected_gap_closures.len() > MAX_DECLARED_GAP_CLOSURES {
            return Err(GuidanceError::TooManyDeclaredGapClosures {
                id: candidate.id.clone(),
                actual: candidate.expected_gap_closures.len(),
                maximum: MAX_DECLARED_GAP_CLOSURES,
            });
        }
        for (factor, value) in [
            ("boundary_importance", candidate.boundary_importance),
            (
                "competing_hypothesis_discrimination",
                candidate.competing_hypothesis_discrimination,
            ),
            ("reproducibility", candidate.reproducibility),
            ("execution_cost", candidate.execution_cost),
            ("mutation_risk", candidate.mutation_risk),
            ("version_drift_risk", candidate.version_drift_risk),
        ] {
            if value > 100 {
                return Err(GuidanceError::FactorOutOfBounds {
                    id: candidate.id.clone(),
                    factor,
                });
            }
        }
        validate_action_identity(candidate)?;
    }
    Ok(())
}

pub(super) fn computed_action_fingerprints(
    candidates: &[GuidanceEvalCandidate],
) -> Result<BTreeMap<String, String>, GuidanceError> {
    candidates
        .iter()
        .map(|candidate| {
            action_fingerprint_from_parts(ActionFingerprintParts {
                hypothesis_id: candidate.action_identity.hypothesis_id.as_str(),
                target_snapshot_id: candidate.target_snapshot_id.as_str(),
                normalized_action: &candidate.action_identity.normalized_action,
                normalized_arguments: &candidate.action_identity.normalized_arguments,
                tool_version: &candidate.action_identity.tool_version,
                input_artifact_ids: candidate
                    .action_identity
                    .input_artifact_ids
                    .iter()
                    .map(|id| id.as_str())
                    .collect(),
                environment_delta: &candidate.action_identity.environment_delta,
                account_prerequisites: candidate
                    .action_identity
                    .account_prerequisites
                    .iter()
                    .map(String::as_str)
                    .collect(),
                toolchain_fingerprint: &candidate.action_identity.toolchain_fingerprint,
            })
            .map(|fingerprint| (candidate.id.clone(), fingerprint))
            .map_err(|error| GuidanceError::ActionIdentityFingerprint {
                id: candidate.id.clone(),
                message: error.to_string(),
            })
        })
        .collect()
}

pub(super) fn duplicate_action_fingerprints(
    fingerprints: &BTreeMap<String, String>,
) -> BTreeSet<String> {
    let mut counts = BTreeMap::new();
    for fingerprint in fingerprints.values() {
        *counts.entry(fingerprint.clone()).or_insert(0usize) += 1;
    }
    counts
        .into_iter()
        .filter_map(|(fingerprint, count)| (count > 1).then_some(fingerprint))
        .collect()
}

pub(super) fn controlled_candidate_identity(
    candidate: &GuidanceEvalCandidate,
) -> Result<String, GuidanceError> {
    canonical_blake3_hash(&GuidanceCandidateIdentityInput {
        algorithm_version: ALGORITHM_VERSION,
        candidate,
    })
    .map(|hash| format!("guidance-candidate-{hash}"))
    .map_err(|error| GuidanceError::InputFingerprint {
        message: error.to_string(),
    })
}

pub(super) fn guidance_input_fingerprint(
    current_target: &TargetSnapshotId,
    reports: &[GuidanceCandidateReport],
) -> Result<String, GuidanceError> {
    canonical_blake3_hash(&GuidanceFingerprintInput {
        algorithm_version: ALGORITHM_VERSION,
        current_target_snapshot: current_target.as_str(),
        candidates: reports,
    })
    .map(|hash| format!("guidance-input-{hash}"))
    .map_err(|error| GuidanceError::InputFingerprint {
        message: error.to_string(),
    })
}

fn validate_action_identity(candidate: &GuidanceEvalCandidate) -> Result<(), GuidanceError> {
    let identity = &candidate.action_identity;
    for (field, value) in [
        ("hypothesis_id", identity.hypothesis_id.as_str()),
        ("normalized_action", identity.normalized_action.as_str()),
        ("tool_version", identity.tool_version.as_str()),
        (
            "toolchain_fingerprint",
            identity.toolchain_fingerprint.as_str(),
        ),
    ] {
        validate_required_identity_string(candidate, field, value)?;
    }
    validate_bounded_identity_string(candidate, "environment_delta", &identity.environment_delta)?;
    validate_identity_array(
        candidate,
        "input_artifact_ids",
        identity.input_artifact_ids.len(),
        MAX_ACTION_IDENTITY_INPUT_ARTIFACTS,
    )?;
    for artifact_id in &identity.input_artifact_ids {
        validate_required_identity_string(candidate, "input_artifact_ids", artifact_id.as_str())?;
    }
    validate_identity_array(
        candidate,
        "account_prerequisites",
        identity.account_prerequisites.len(),
        MAX_ACTION_IDENTITY_PREREQUISITES,
    )?;
    for prerequisite in &identity.account_prerequisites {
        validate_bounded_identity_string(candidate, "account_prerequisites", prerequisite)?;
    }
    if !bounded_json_value(&identity.normalized_arguments, 0, &mut 0usize) {
        return Err(GuidanceError::ActionIdentityArgumentsTooLarge {
            id: candidate.id.clone(),
        });
    }
    Ok(())
}

fn validate_required_identity_string(
    candidate: &GuidanceEvalCandidate,
    field: &'static str,
    value: &str,
) -> Result<(), GuidanceError> {
    if value.trim().is_empty() || value.len() > MAX_ACTION_IDENTITY_STRING_BYTES {
        return Err(GuidanceError::InvalidActionIdentityString {
            id: candidate.id.clone(),
            field,
            maximum: MAX_ACTION_IDENTITY_STRING_BYTES,
        });
    }
    Ok(())
}

fn validate_bounded_identity_string(
    candidate: &GuidanceEvalCandidate,
    field: &'static str,
    value: &str,
) -> Result<(), GuidanceError> {
    if value.len() > MAX_ACTION_IDENTITY_STRING_BYTES {
        return Err(GuidanceError::InvalidActionIdentityString {
            id: candidate.id.clone(),
            field,
            maximum: MAX_ACTION_IDENTITY_STRING_BYTES,
        });
    }
    Ok(())
}

fn validate_identity_array(
    candidate: &GuidanceEvalCandidate,
    field: &'static str,
    actual: usize,
    maximum: usize,
) -> Result<(), GuidanceError> {
    if actual > maximum {
        return Err(GuidanceError::ActionIdentityArrayTooLarge {
            id: candidate.id.clone(),
            field,
            actual,
            maximum,
        });
    }
    Ok(())
}

fn bounded_json_value(value: &Value, depth: usize, nodes: &mut usize) -> bool {
    if depth > MAX_ACTION_IDENTITY_VALUE_DEPTH || *nodes == MAX_ACTION_IDENTITY_VALUE_NODES {
        return false;
    }
    *nodes += 1;
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => true,
        Value::String(value) => value.len() <= MAX_ACTION_IDENTITY_STRING_BYTES,
        Value::Array(values) => {
            values.len() <= MAX_ACTION_IDENTITY_VALUE_COLLECTION
                && values
                    .iter()
                    .all(|value| bounded_json_value(value, depth + 1, nodes))
        }
        Value::Object(values) => {
            values.len() <= MAX_ACTION_IDENTITY_VALUE_COLLECTION
                && values.iter().all(|(key, value)| {
                    key.len() <= MAX_ACTION_IDENTITY_STRING_BYTES
                        && bounded_json_value(value, depth + 1, nodes)
                })
        }
    }
}

fn valid_candidate_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_CANDIDATE_ID_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn research_reference_id(reference: &ResearchRecordRef) -> &str {
    match reference {
        ResearchRecordRef::Hypothesis(id) => id.as_str(),
        ResearchRecordRef::Experiment(id) => id.as_str(),
        ResearchRecordRef::Finding(id) => id.as_str(),
    }
}

#[derive(Serialize)]
struct GuidanceCandidateIdentityInput<'a> {
    algorithm_version: &'static str,
    candidate: &'a GuidanceEvalCandidate,
}

#[derive(Serialize)]
struct GuidanceFingerprintInput<'a> {
    algorithm_version: &'static str,
    current_target_snapshot: &'a str,
    candidates: &'a [GuidanceCandidateReport],
}
