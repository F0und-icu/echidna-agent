//! Read-only composition for externally produced Observation batches.
//!
//! The Domain owner validates the strict batch schema and derives the Delta.
//! This service validates each observation against the existing Research
//! Artifact/TargetSnapshot facts and the Session's paired Tool/Receipt facts.

use std::collections::BTreeSet;

use thiserror::Error;

use crate::domain::artifacts::ArtifactRecord;
use crate::domain::model::ContentItem;
use crate::domain::observation::{
    ObservationBatch, ObservationError, TargetDelta, ValidatedObservationSet,
};
use crate::domain::research::ResearchState;
use crate::domain::session::SessionState;
use crate::domain::tools::{ExecutionId, ToolCall, ToolResultStatus};

pub(crate) struct ObservationService;

impl ObservationService {
    /// Validates one exact external AndroidManifest batch. It is read-only:
    /// neither the source Artifact nor the derived observations are written
    /// to Research, Memory, Session, or a separate Observation index.
    pub(crate) fn ingest_android_manifest(
        research: &ResearchState,
        session: &SessionState,
        artifact: &ArtifactRecord,
        bytes: &[u8],
    ) -> Result<ValidatedObservationSet, ObservationServiceError> {
        validate_artifact_bytes(artifact, bytes)?;
        let batch = ObservationBatch::decode(bytes)?;
        if batch.raw_artifact_id() != &artifact.id {
            return Err(ObservationServiceError::RawArtifactMismatch {
                declared: batch.raw_artifact_id().to_string(),
                supplied: artifact.id.to_string(),
            });
        }
        if artifact.target_snapshot_id != *batch.target_snapshot_id() {
            return Err(ObservationServiceError::ArtifactSnapshotMismatch {
                artifact_id: artifact.id.to_string(),
                artifact_snapshot_id: artifact.target_snapshot_id.to_string(),
                observation_snapshot_id: batch.target_snapshot_id().to_string(),
            });
        }
        let extractor = batch.extractor();
        let expected_toolchain_fingerprint = extractor.artifact_toolchain_fingerprint();
        if artifact.toolchain_fingerprint != expected_toolchain_fingerprint {
            return Err(ObservationServiceError::ExtractorArtifactMismatch {
                artifact_id: artifact.id.to_string(),
                declared_extractor: expected_toolchain_fingerprint,
                artifact_toolchain: artifact.toolchain_fingerprint.clone(),
            });
        }
        let snapshot = research
            .target_snapshot(batch.target_snapshot_id())
            .ok_or_else(|| ObservationServiceError::TargetSnapshotMissing {
                target_snapshot_id: batch.target_snapshot_id().to_string(),
            })?;

        let mut checked_calls = BTreeSet::new();
        let mut checked_evidence = BTreeSet::new();
        for observation in &batch.observations {
            for evidence in &observation.evidence_refs {
                if checked_evidence.insert(evidence.clone()) {
                    let evidence_artifact = research.artifact(evidence).ok_or_else(|| {
                        ObservationServiceError::EvidenceArtifactMissing {
                            artifact_id: evidence.to_string(),
                        }
                    })?;
                    if evidence_artifact.target_snapshot_id != observation.target_snapshot_id {
                        return Err(ObservationServiceError::EvidenceArtifactSnapshotMismatch {
                            artifact_id: evidence.to_string(),
                            artifact_snapshot_id: evidence_artifact.target_snapshot_id.to_string(),
                            observation_snapshot_id: observation.target_snapshot_id.to_string(),
                        });
                    }
                }
            }
            if checked_calls.insert((
                observation.produced_by_tool_call_id.clone(),
                observation.execution_receipt_event_id.clone(),
            )) {
                validate_external_command_source(
                    session,
                    &observation.produced_by_tool_call_id,
                    &observation.execution_receipt_event_id,
                )?;
            }
        }
        Ok(ValidatedObservationSet::from_validated_batch(
            batch,
            snapshot.target_id.clone(),
        ))
    }

    pub(crate) fn target_delta(
        baseline: &ValidatedObservationSet,
        candidate: &ValidatedObservationSet,
    ) -> Result<TargetDelta, ObservationServiceError> {
        Ok(TargetDelta::compare(baseline, candidate)?)
    }
}

fn validate_external_command_source(
    session: &SessionState,
    call_id: &crate::domain::tools::ToolCallId,
    receipt_event_id: &crate::domain::events::EventId,
) -> Result<(), ObservationServiceError> {
    let call = session
        .history()
        .items()
        .iter()
        .find_map(|item| match item {
            ContentItem::ToolCall { call } if call.call_id == *call_id => Some(call),
            ContentItem::Text { .. }
            | ContentItem::ImageArtifact { .. }
            | ContentItem::Reasoning { .. }
            | ContentItem::ToolCall { .. }
            | ContentItem::ToolResult { .. }
            | ContentItem::Unsupported { .. } => None,
        })
        .ok_or_else(|| ObservationServiceError::ToolCallMissing {
            call_id: call_id.to_string(),
        })?;
    if call.name.as_str() != "run_commands" {
        return Err(ObservationServiceError::ExtractorToolMismatch {
            call_id: call_id.to_string(),
            tool_name: call.name.to_string(),
        });
    }
    validate_paired_success(session, call)?;

    let expected_execution_id = ExecutionId::new(format!("execution:{call_id}"));
    let receipt = session
        .execution_receipts()
        .iter()
        .find(|record| record.event_id == *receipt_event_id)
        .ok_or_else(|| ObservationServiceError::ReceiptMissing {
            receipt_event_id: receipt_event_id.to_string(),
        })?;
    if receipt.receipt.execution_id != expected_execution_id {
        return Err(ObservationServiceError::ReceiptExecutionMismatch {
            receipt_event_id: receipt_event_id.to_string(),
            expected_execution_id: expected_execution_id.to_string(),
            received_execution_id: receipt.receipt.execution_id.to_string(),
        });
    }
    if receipt.receipt.status != ToolResultStatus::Succeeded {
        return Err(ObservationServiceError::ReceiptNotSuccessful {
            receipt_event_id: receipt_event_id.to_string(),
        });
    }
    Ok(())
}

/// Session reduction is the only pairing owner. This lookup merely resolves
/// the already-paired durable Result that belongs to the exact source Call.
fn validate_paired_success(
    session: &SessionState,
    call: &ToolCall,
) -> Result<(), ObservationServiceError> {
    let result = session
        .history()
        .items()
        .iter()
        .find_map(|item| match item {
            ContentItem::ToolResult { result }
                if result.call_id == call.call_id && result.tool_name == call.name =>
            {
                Some(result)
            }
            ContentItem::Text { .. }
            | ContentItem::ImageArtifact { .. }
            | ContentItem::Reasoning { .. }
            | ContentItem::ToolCall { .. }
            | ContentItem::ToolResult { .. }
            | ContentItem::Unsupported { .. } => None,
        })
        .ok_or_else(|| ObservationServiceError::ToolResultMissing {
            call_id: call.call_id.to_string(),
        })?;
    if result.status != ToolResultStatus::Succeeded {
        return Err(ObservationServiceError::ToolResultNotSuccessful {
            call_id: call.call_id.to_string(),
        });
    }
    Ok(())
}

#[derive(Debug, Error)]
pub(crate) enum ObservationServiceError {
    #[error(transparent)]
    Observation(#[from] ObservationError),
    #[error(
        "Observation source Artifact `{artifact_id}` declares {declared_size} bytes, but its verified Blob contains {actual_size} bytes"
    )]
    ArtifactSizeMismatch {
        artifact_id: String,
        declared_size: u64,
        actual_size: u64,
    },
    #[error(
        "Observation source Artifact `{artifact_id}` is {actual_size} bytes, exceeding the {maximum_size}-byte limit"
    )]
    ArtifactTooLarge {
        artifact_id: String,
        actual_size: u64,
        maximum_size: u64,
    },
    #[error(
        "Observation batch declares raw Artifact `{declared}`, but the supplied Artifact is `{supplied}`"
    )]
    RawArtifactMismatch { declared: String, supplied: String },
    #[error(
        "Artifact `{artifact_id}` binds TargetSnapshot `{artifact_snapshot_id}`, not Observation TargetSnapshot `{observation_snapshot_id}`"
    )]
    ArtifactSnapshotMismatch {
        artifact_id: String,
        artifact_snapshot_id: String,
        observation_snapshot_id: String,
    },
    #[error(
        "Observation Extractor `{declared_extractor}` does not match Artifact `{artifact_id}` toolchain `{artifact_toolchain}`"
    )]
    ExtractorArtifactMismatch {
        artifact_id: String,
        declared_extractor: String,
        artifact_toolchain: String,
    },
    #[error("Observation TargetSnapshot `{target_snapshot_id}` is not present in Research")]
    TargetSnapshotMissing { target_snapshot_id: String },
    #[error("Observation evidence Artifact `{artifact_id}` is not present in Research")]
    EvidenceArtifactMissing { artifact_id: String },
    #[error(
        "Observation evidence Artifact `{artifact_id}` binds TargetSnapshot `{artifact_snapshot_id}`, not `{observation_snapshot_id}`"
    )]
    EvidenceArtifactSnapshotMismatch {
        artifact_id: String,
        artifact_snapshot_id: String,
        observation_snapshot_id: String,
    },
    #[error("Observation source ToolCall `{call_id}` is not present in Session History")]
    ToolCallMissing { call_id: String },
    #[error("Observation source ToolCall `{call_id}` must be `run_commands`, not `{tool_name}`")]
    ExtractorToolMismatch { call_id: String, tool_name: String },
    #[error("Observation source ToolCall `{call_id}` does not have a paired ToolResult")]
    ToolResultMissing { call_id: String },
    #[error("Observation source ToolCall `{call_id}` did not complete successfully")]
    ToolResultNotSuccessful { call_id: String },
    #[error("Observation execution Receipt Event `{receipt_event_id}` is not present in Session")]
    ReceiptMissing { receipt_event_id: String },
    #[error(
        "Observation Receipt Event `{receipt_event_id}` has Execution ID `{received_execution_id}`, expected `{expected_execution_id}`"
    )]
    ReceiptExecutionMismatch {
        receipt_event_id: String,
        expected_execution_id: String,
        received_execution_id: String,
    },
    #[error("Observation execution Receipt Event `{receipt_event_id}` is not successful")]
    ReceiptNotSuccessful { receipt_event_id: String },
}

/// The bounded raw Observation Artifact size consumed by the local
/// inspection path. Bigger raw outputs remain durable Artifacts but require a
/// later bounded extractor contract; they are never partially parsed.
pub(crate) const MAX_ANDROID_MANIFEST_OBSERVATION_BYTES: u64 = 512 * 1024;

fn validate_artifact_bytes(
    artifact: &ArtifactRecord,
    bytes: &[u8],
) -> Result<(), ObservationServiceError> {
    let actual_size =
        u64::try_from(bytes.len()).map_err(|_| ObservationServiceError::ArtifactTooLarge {
            artifact_id: artifact.id.to_string(),
            actual_size: u64::MAX,
            maximum_size: MAX_ANDROID_MANIFEST_OBSERVATION_BYTES,
        })?;
    if actual_size != artifact.size_bytes {
        return Err(ObservationServiceError::ArtifactSizeMismatch {
            artifact_id: artifact.id.to_string(),
            declared_size: artifact.size_bytes,
            actual_size,
        });
    }
    if actual_size > MAX_ANDROID_MANIFEST_OBSERVATION_BYTES {
        return Err(ObservationServiceError::ArtifactTooLarge {
            artifact_id: artifact.id.to_string(),
            actual_size,
            maximum_size: MAX_ANDROID_MANIFEST_OBSERVATION_BYTES,
        });
    }
    Ok(())
}
