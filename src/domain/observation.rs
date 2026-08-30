//! Provider-neutral, externally produced attack-surface observations.
//!
//! This module deliberately decodes a strict JSON contract only.  It does
//! not parse Android binary XML, invoke a process, write a fact stream, or
//! promote an observation into Research.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::domain::artifacts::ArtifactId;
use crate::domain::events::EventId;
use crate::domain::research::TargetSnapshotId;
use crate::domain::tools::ToolCallId;

mod delta;

pub(crate) use delta::{TargetDelta, ValidatedObservationSet};

pub(crate) const OBSERVATION_BATCH_SCHEMA_VERSION: u16 = 1;
pub(crate) const OBSERVATION_ENVELOPE_SCHEMA_VERSION: u16 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ObservationBatch {
    pub(crate) schema_version: u16,
    pub(crate) observations: Vec<ObservationEnvelope>,
}

impl ObservationBatch {
    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, ObservationError> {
        let batch: Self =
            serde_json::from_slice(bytes).map_err(|error| ObservationError::Malformed {
                message: error.to_string(),
            })?;
        batch.validate()?;
        Ok(batch)
    }

    pub(crate) fn validate(&self) -> Result<(), ObservationError> {
        if self.schema_version != OBSERVATION_BATCH_SCHEMA_VERSION {
            return Err(ObservationError::UnsupportedBatchSchema {
                received: self.schema_version,
            });
        }
        if self.observations.is_empty() {
            return Err(ObservationError::Invalid {
                message: "Observation batch must contain at least one ObservationEnvelope"
                    .to_owned(),
            });
        }

        let first = &self.observations[0];
        let mut observation_ids = BTreeSet::new();
        let mut component_subjects = BTreeSet::new();
        for observation in &self.observations {
            observation.validate()?;
            if observation.target_snapshot_id != first.target_snapshot_id
                || observation.raw_artifact_id != first.raw_artifact_id
                || observation.produced_by_tool_call_id != first.produced_by_tool_call_id
                || observation.execution_receipt_event_id != first.execution_receipt_event_id
                || observation.extractor_id != first.extractor_id
                || observation.extractor_version != first.extractor_version
            {
                return Err(ObservationError::Invalid {
                    message: "all observations in one raw batch must bind one TargetSnapshot, raw Artifact, ToolCall, Receipt Event, and Extractor identity/version".to_owned(),
                });
            }
            if !observation_ids.insert(observation.observation_id.clone()) {
                return Err(ObservationError::Invalid {
                    message: format!(
                        "Observation batch duplicates observation ID `{}`",
                        observation.observation_id
                    ),
                });
            }
            if !component_subjects.insert(observation.subject.clone()) {
                return Err(ObservationError::Invalid {
                    message: format!(
                        "Observation batch duplicates Android manifest component `{}`",
                        observation.subject
                    ),
                });
            }
        }
        Ok(())
    }

    pub(crate) fn raw_artifact_id(&self) -> &ArtifactId {
        &self.observations[0].raw_artifact_id
    }

    pub(crate) fn target_snapshot_id(&self) -> &TargetSnapshotId {
        &self.observations[0].target_snapshot_id
    }

    pub(crate) fn extractor(&self) -> ExtractorIdentity {
        ExtractorIdentity {
            id: self.observations[0].extractor_id.clone(),
            version: self.observations[0].extractor_version.clone(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub(crate) struct ObservationId(String);

impl ObservationId {
    fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ObservationId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ObservationConfidence {
    Low,
    Medium,
    High,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ObservationEnvelope {
    pub(crate) schema_version: u16,
    pub(crate) observation_id: ObservationId,
    #[serde(flatten)]
    pub(crate) kind: ObservationKind,
    pub(crate) target_snapshot_id: TargetSnapshotId,
    pub(crate) raw_artifact_id: ArtifactId,
    pub(crate) produced_by_tool_call_id: ToolCallId,
    pub(crate) execution_receipt_event_id: EventId,
    pub(crate) location: String,
    pub(crate) subject: String,
    pub(crate) entry_point: String,
    pub(crate) source: String,
    pub(crate) sink: String,
    pub(crate) prerequisites: Vec<String>,
    pub(crate) confidence: ObservationConfidence,
    pub(crate) evidence_refs: Vec<ArtifactId>,
    pub(crate) extractor_id: String,
    pub(crate) extractor_version: String,
}

impl ObservationEnvelope {
    fn validate(&self) -> Result<(), ObservationError> {
        if self.schema_version != OBSERVATION_ENVELOPE_SCHEMA_VERSION {
            return Err(ObservationError::UnsupportedEnvelopeSchema {
                received: self.schema_version,
            });
        }
        for (label, value) in [
            ("observation_id", self.observation_id.as_str()),
            ("target_snapshot_id", self.target_snapshot_id.as_str()),
            ("raw_artifact_id", self.raw_artifact_id.as_str()),
            (
                "produced_by_tool_call_id",
                self.produced_by_tool_call_id.as_str(),
            ),
            (
                "execution_receipt_event_id",
                self.execution_receipt_event_id.as_str(),
            ),
            ("location", &self.location),
            ("subject", &self.subject),
            ("entry_point", &self.entry_point),
            ("source", &self.source),
            ("sink", &self.sink),
            ("extractor_id", &self.extractor_id),
            ("extractor_version", &self.extractor_version),
        ] {
            require_text(label, value)?;
        }
        validate_extractor_component("extractor_id", &self.extractor_id)?;
        validate_extractor_component("extractor_version", &self.extractor_version)?;
        validate_text_list("prerequisites", &self.prerequisites, true)?;
        if self.evidence_refs.is_empty() {
            return Err(ObservationError::Invalid {
                message: "Observation evidence_refs must include its raw Artifact".to_owned(),
            });
        }
        let evidence = self
            .evidence_refs
            .iter()
            .map(ArtifactId::as_str)
            .collect::<Vec<_>>();
        validate_unique_text_list("evidence_refs", &evidence)?;
        if !self.evidence_refs.contains(&self.raw_artifact_id) {
            return Err(ObservationError::Invalid {
                message: format!(
                    "Observation `{}` evidence_refs must include raw Artifact `{}`",
                    self.observation_id, self.raw_artifact_id
                ),
            });
        }
        self.kind.validate(&self.entry_point)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "details", rename_all = "snake_case")]
pub(crate) enum ObservationKind {
    AndroidManifestComponent(AndroidManifestComponent),
}

impl ObservationKind {
    fn validate(&self, entry_point: &str) -> Result<(), ObservationError> {
        match self {
            Self::AndroidManifestComponent(details) => {
                details.validate()?;
                if details.component_type.as_str() != entry_point {
                    return Err(ObservationError::Invalid {
                        message: format!(
                            "Android manifest entry_point `{entry_point}` must match component_type `{}`",
                            details.component_type.as_str()
                        ),
                    });
                }
                Ok(())
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AndroidComponentType {
    Activity,
    ActivityAlias,
    Service,
    Receiver,
    Provider,
}

impl AndroidComponentType {
    fn as_str(self) -> &'static str {
        match self {
            Self::Activity => "activity",
            Self::ActivityAlias => "activity_alias",
            Self::Service => "service",
            Self::Receiver => "receiver",
            Self::Provider => "provider",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AndroidManifestComponent {
    pub(crate) component_type: AndroidComponentType,
    pub(crate) exported: bool,
    #[serde(default)]
    pub(crate) permission: Option<String>,
    #[serde(default)]
    pub(crate) intent_filters: Vec<AndroidIntentFilter>,
}

impl AndroidManifestComponent {
    fn validate(&self) -> Result<(), ObservationError> {
        if let Some(permission) = &self.permission {
            require_text("Android manifest permission", permission)?;
        }
        let mut seen = BTreeSet::new();
        for filter in &self.intent_filters {
            filter.validate()?;
            let key = filter.stable_key();
            if !seen.insert(key) {
                return Err(ObservationError::Invalid {
                    message: "Android manifest component contains a duplicate intent filter"
                        .to_owned(),
                });
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AndroidIntentFilter {
    #[serde(default)]
    pub(crate) actions: Vec<String>,
    #[serde(default)]
    pub(crate) categories: Vec<String>,
    #[serde(default)]
    pub(crate) data_schemes: Vec<String>,
}

impl AndroidIntentFilter {
    fn validate(&self) -> Result<(), ObservationError> {
        validate_text_list("Android intent-filter actions", &self.actions, false)?;
        validate_text_list("Android intent-filter categories", &self.categories, false)?;
        validate_text_list(
            "Android intent-filter data_schemes",
            &self.data_schemes,
            false,
        )?;
        if self.actions.is_empty() && self.categories.is_empty() && self.data_schemes.is_empty() {
            return Err(ObservationError::Invalid {
                message: "Android intent filter must include an action, category, or data scheme"
                    .to_owned(),
            });
        }
        Ok(())
    }

    fn stable_key(&self) -> String {
        let mut actions = self.actions.clone();
        let mut categories = self.categories.clone();
        let mut data_schemes = self.data_schemes.clone();
        actions.sort();
        categories.sort();
        data_schemes.sort();
        serde_json::to_string(&(actions, categories, data_schemes))
            .expect("Android intent-filter tuple is serializable")
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct ExtractorIdentity {
    pub(crate) id: String,
    pub(crate) version: String,
}

impl ExtractorIdentity {
    pub(crate) fn artifact_toolchain_fingerprint(&self) -> String {
        format!("{}@{}", self.id, self.version)
    }
}

fn require_text(label: &str, value: &str) -> Result<(), ObservationError> {
    if value.trim().is_empty() {
        Err(ObservationError::Invalid {
            message: format!("Observation {label} must not be empty"),
        })
    } else {
        Ok(())
    }
}

fn validate_extractor_component(label: &str, value: &str) -> Result<(), ObservationError> {
    if value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'+' | b'-'))
    {
        Ok(())
    } else {
        Err(ObservationError::Invalid {
            message: format!(
                "Observation {label} must use only ASCII letters, digits, dot, underscore, plus, or hyphen"
            ),
        })
    }
}

fn validate_text_list(
    label: &str,
    values: &[String],
    required: bool,
) -> Result<(), ObservationError> {
    if required && values.is_empty() {
        return Err(ObservationError::Invalid {
            message: format!("Observation {label} must not be empty"),
        });
    }
    let references = values.iter().map(String::as_str).collect::<Vec<_>>();
    validate_unique_text_list(label, &references)
}

fn validate_unique_text_list(label: &str, values: &[&str]) -> Result<(), ObservationError> {
    let mut seen = BTreeSet::new();
    for value in values {
        require_text(label, value)?;
        if !seen.insert(*value) {
            return Err(ObservationError::Invalid {
                message: format!("Observation {label} contains duplicate value `{value}`"),
            });
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub(crate) enum ObservationError {
    #[error("Observation batch schema version `{received}` is unsupported")]
    UnsupportedBatchSchema { received: u16 },
    #[error("ObservationEnvelope schema version `{received}` is unsupported")]
    UnsupportedEnvelopeSchema { received: u16 },
    #[error("Observation payload is malformed: {message}")]
    Malformed { message: String },
    #[error("Observation payload is invalid: {message}")]
    Invalid { message: String },
    #[error("TargetDelta requires one target identity, not `{baseline}` and `{candidate}")]
    TargetIdentityMismatch { baseline: String, candidate: String },
    #[error(
        "TargetDelta requires one exact Extractor identity/version, not `{baseline_id}@{baseline_version}` and `{candidate_id}@{candidate_version}`"
    )]
    ExtractorMismatch {
        baseline_id: String,
        baseline_version: String,
        candidate_id: String,
        candidate_version: String,
    },
}
