//! Structured Session Summary and Compaction fact data.
//!
//! This narrow module owns Summary construction, deterministic fingerprinting,
//! retained-History selection, and Compaction payload-shape validation. The
//! parent Session module retains Event mapping, lifecycle transitions and the
//! strict reducer boundary.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::domain::artifacts::ArtifactId;
use crate::domain::events::{EventSequence, EventType};
use crate::domain::history::{ConversationHistory, retain_recent_complete_history};
use crate::domain::model::ContentItem;
use crate::domain::research::TargetSnapshotId;
use crate::domain::tools::{ExecutionId, JobId, PtySessionId};

use super::{COMPACTION_CREATED_EVENT_TYPE, CURRENT_SESSION_PAYLOAD_SCHEMA_VERSION, SessionError};

const MAX_SESSION_SUMMARY_BYTES: usize = 8 * 1024;

/// A typed reference to work that remains unfinished at Compaction time.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum UnfinishedExecutionRef {
    Execution(ExecutionId),
    Job(JobId),
    Pty(PtySessionId),
}

/// Caller-supplied structured content for one derived Session Summary.
///
/// The Session reducer does not parse Assistant text into Research facts.
/// The caller must provide each required research field explicitly; durable
/// Compaction then makes this draft reproducible from the Session stream.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionSummaryDraft {
    goal: String,
    target: String,
    target_version: String,
    target_snapshot: TargetSnapshotId,
    environment: String,
    verified_facts: Vec<String>,
    active_hypotheses: Vec<String>,
    failed_paths: Vec<String>,
    artifacts: Vec<ArtifactId>,
    modified_files: Vec<String>,
    unfinished_execution: Vec<UnfinishedExecutionRef>,
    next_steps: Vec<String>,
}

impl SessionSummaryDraft {
    pub fn new(
        goal: impl Into<String>,
        target: impl Into<String>,
        target_version: impl Into<String>,
        target_snapshot: TargetSnapshotId,
        environment: impl Into<String>,
    ) -> Result<Self, SessionError> {
        let draft = Self {
            goal: goal.into(),
            target: target.into(),
            target_version: target_version.into(),
            target_snapshot,
            environment: environment.into(),
            verified_facts: Vec::new(),
            active_hypotheses: Vec::new(),
            failed_paths: Vec::new(),
            artifacts: Vec::new(),
            modified_files: Vec::new(),
            unfinished_execution: Vec::new(),
            next_steps: Vec::new(),
        };
        draft.validate()?;
        Ok(draft)
    }

    pub fn with_verified_facts(
        mut self,
        values: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.verified_facts = values.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_active_hypotheses(
        mut self,
        values: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.active_hypotheses = values.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_failed_paths(
        mut self,
        values: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.failed_paths = values.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_artifacts(mut self, values: impl IntoIterator<Item = ArtifactId>) -> Self {
        self.artifacts = values.into_iter().collect();
        self
    }

    pub fn with_modified_files(
        mut self,
        values: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.modified_files = values.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_unfinished_execution(
        mut self,
        values: impl IntoIterator<Item = UnfinishedExecutionRef>,
    ) -> Self {
        self.unfinished_execution = values.into_iter().collect();
        self
    }

    pub fn with_next_steps(mut self, values: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.next_steps = values.into_iter().map(Into::into).collect();
        self
    }

    fn validate(&self) -> Result<(), SessionError> {
        for (field, value) in [
            ("goal", &self.goal),
            ("target", &self.target),
            ("target_version", &self.target_version),
            ("environment", &self.environment),
        ] {
            if value.trim().is_empty() {
                return Err(SessionError::CompactionSummaryFieldRequired { field });
            }
        }
        if self.target_snapshot.as_str().trim().is_empty() {
            return Err(SessionError::CompactionSummaryFieldRequired {
                field: "target_snapshot",
            });
        }
        validate_text_entries("verified_facts", &self.verified_facts)?;
        validate_text_entries("active_hypotheses", &self.active_hypotheses)?;
        validate_text_entries("failed_paths", &self.failed_paths)?;
        validate_text_entries("modified_files", &self.modified_files)?;
        validate_text_entries("next_steps", &self.next_steps)?;
        validate_unique_ids("artifacts", self.artifacts.iter().map(ArtifactId::as_str))?;
        validate_unique_ids(
            "unfinished_execution",
            self.unfinished_execution.iter().map(|item| match item {
                UnfinishedExecutionRef::Execution(id) => id.as_str(),
                UnfinishedExecutionRef::Job(id) => id.as_str(),
                UnfinishedExecutionRef::Pty(id) => id.as_str(),
            }),
        )?;
        Ok(())
    }
}

fn validate_text_entries(field: &'static str, values: &[String]) -> Result<(), SessionError> {
    validate_unique_ids(field, values.iter().map(String::as_str))
}

fn validate_unique_ids<'a>(
    field: &'static str,
    values: impl IntoIterator<Item = &'a str>,
) -> Result<(), SessionError> {
    let mut seen = BTreeSet::new();
    for value in values {
        if value.trim().is_empty() {
            return Err(SessionError::CompactionSummaryEntryEmpty { field });
        }
        if !seen.insert(value) {
            return Err(SessionError::CompactionSummaryEntryDuplicate {
                field,
                value: value.to_owned(),
            });
        }
    }
    Ok(())
}

/// Deletable, deterministic projection carried by one Compaction fact. The
/// strict Session reducer rebuilds the active view by replaying ordered
/// Compaction Events; it never regenerates research claims from Assistant
/// prose.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionSummary {
    schema_version: u16,
    revision: u64,
    fingerprint: String,
    goal: String,
    target: String,
    target_version: String,
    target_snapshot: TargetSnapshotId,
    environment: String,
    verified_facts: Vec<String>,
    active_hypotheses: Vec<String>,
    failed_paths: Vec<String>,
    artifacts: Vec<ArtifactId>,
    modified_files: Vec<String>,
    unfinished_execution: Vec<UnfinishedExecutionRef>,
    next_steps: Vec<String>,
}

impl SessionSummary {
    pub(super) fn from_draft(
        draft: SessionSummaryDraft,
        revision: u64,
    ) -> Result<Self, SessionError> {
        draft.validate()?;
        let mut summary = Self {
            schema_version: CURRENT_SESSION_PAYLOAD_SCHEMA_VERSION,
            revision,
            fingerprint: String::new(),
            goal: draft.goal,
            target: draft.target,
            target_version: draft.target_version,
            target_snapshot: draft.target_snapshot,
            environment: draft.environment,
            verified_facts: draft.verified_facts,
            active_hypotheses: draft.active_hypotheses,
            failed_paths: draft.failed_paths,
            artifacts: draft.artifacts,
            modified_files: draft.modified_files,
            unfinished_execution: draft.unfinished_execution,
            next_steps: draft.next_steps,
        };
        summary.fingerprint = summary.expected_fingerprint()?;
        summary.validate_encoded_size()?;
        Ok(summary)
    }

    fn validate(&self) -> Result<(), SessionError> {
        if self.schema_version != CURRENT_SESSION_PAYLOAD_SCHEMA_VERSION {
            return Err(SessionError::UnsupportedPayloadSchema {
                event_type: EventType::new(COMPACTION_CREATED_EVENT_TYPE),
                schema_version: self.schema_version,
            });
        }
        if self.fingerprint != self.expected_fingerprint()? {
            return Err(SessionError::CompactionFingerprintMismatch);
        }
        SessionSummaryDraft {
            goal: self.goal.clone(),
            target: self.target.clone(),
            target_version: self.target_version.clone(),
            target_snapshot: self.target_snapshot.clone(),
            environment: self.environment.clone(),
            verified_facts: self.verified_facts.clone(),
            active_hypotheses: self.active_hypotheses.clone(),
            failed_paths: self.failed_paths.clone(),
            artifacts: self.artifacts.clone(),
            modified_files: self.modified_files.clone(),
            unfinished_execution: self.unfinished_execution.clone(),
            next_steps: self.next_steps.clone(),
        }
        .validate()?;
        self.validate_encoded_size()?;
        Ok(())
    }

    pub const fn revision(&self) -> u64 {
        self.revision
    }

    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    pub fn goal(&self) -> &str {
        &self.goal
    }

    pub fn target(&self) -> &str {
        &self.target
    }

    pub fn target_version(&self) -> &str {
        &self.target_version
    }

    pub fn target_snapshot(&self) -> &TargetSnapshotId {
        &self.target_snapshot
    }

    pub fn environment(&self) -> &str {
        &self.environment
    }

    pub fn verified_facts(&self) -> &[String] {
        &self.verified_facts
    }

    pub fn active_hypotheses(&self) -> &[String] {
        &self.active_hypotheses
    }

    pub fn failed_paths(&self) -> &[String] {
        &self.failed_paths
    }

    pub fn artifacts(&self) -> &[ArtifactId] {
        &self.artifacts
    }

    pub fn modified_files(&self) -> &[String] {
        &self.modified_files
    }

    pub fn unfinished_execution(&self) -> &[UnfinishedExecutionRef] {
        &self.unfinished_execution
    }

    pub fn next_steps(&self) -> &[String] {
        &self.next_steps
    }

    pub(crate) fn context_item(&self) -> Result<ContentItem, SessionError> {
        let payload = serde_json::to_string(self).map_err(|error| SessionError::PayloadEncode {
            event_type: EventType::new(COMPACTION_CREATED_EVENT_TYPE),
            message: error.to_string(),
        })?;
        Ok(ContentItem::user_text(format!(
            "derived_session_summary_v1:{payload}"
        )))
    }

    pub(crate) fn matches_draft(&self, draft: &SessionSummaryDraft) -> Result<bool, SessionError> {
        Ok(Self::from_draft(draft.clone(), self.revision)? == *self)
    }

    fn validate_encoded_size(&self) -> Result<(), SessionError> {
        let actual = serde_json::to_vec(self)
            .map_err(|error| SessionError::PayloadEncode {
                event_type: EventType::new(COMPACTION_CREATED_EVENT_TYPE),
                message: error.to_string(),
            })?
            .len();
        if actual > MAX_SESSION_SUMMARY_BYTES {
            Err(SessionError::CompactionSummaryTooLarge {
                actual,
                maximum: MAX_SESSION_SUMMARY_BYTES,
            })
        } else {
            Ok(())
        }
    }

    fn expected_fingerprint(&self) -> Result<String, SessionError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint_value(&value)
    }
}

/// The one append-only Compaction fact. The retained view is derived from,
/// and verified against, still-retained original Session History.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionCompaction {
    schema_version: u16,
    summary: SessionSummary,
    source_history_item_count: usize,
    source_event_sequence: EventSequence,
    retained_from_normalized_history_index: usize,
    retained_history: Vec<ContentItem>,
}

impl SessionCompaction {
    /// Builds a pair-safe recent suffix from normalized History. Original
    /// Session facts remain untouched.
    pub(super) fn from_history(
        history: &ConversationHistory,
        summary: SessionSummary,
        source_event_sequence: EventSequence,
        requested_recent_items: usize,
    ) -> Result<Self, SessionError> {
        let (retained_from_normalized_history_index, retained_history) =
            retain_recent_complete_history(history.items(), requested_recent_items)?;
        Ok(Self {
            schema_version: CURRENT_SESSION_PAYLOAD_SCHEMA_VERSION,
            summary,
            source_history_item_count: history.items().len(),
            source_event_sequence,
            retained_from_normalized_history_index,
            retained_history,
        })
    }

    pub fn summary(&self) -> &SessionSummary {
        &self.summary
    }

    pub const fn source_history_item_count(&self) -> usize {
        self.source_history_item_count
    }

    pub const fn source_event_sequence(&self) -> EventSequence {
        self.source_event_sequence
    }

    pub const fn retained_from_normalized_history_index(&self) -> usize {
        self.retained_from_normalized_history_index
    }

    pub fn retained_history(&self) -> &[ContentItem] {
        &self.retained_history
    }

    pub(super) fn validate_shape(&self) -> Result<(), SessionError> {
        if self.schema_version != CURRENT_SESSION_PAYLOAD_SCHEMA_VERSION {
            return Err(SessionError::UnsupportedPayloadSchema {
                event_type: EventType::new(COMPACTION_CREATED_EVENT_TYPE),
                schema_version: self.schema_version,
            });
        }
        self.summary.validate()?;
        ConversationHistory::from_complete_content_items(&self.retained_history)?;
        Ok(())
    }

    pub(crate) fn fingerprint(&self) -> Result<String, SessionError> {
        fingerprint_value(self)
    }
}

fn fingerprint_value(value: &impl Serialize) -> Result<String, SessionError> {
    let bytes = serde_json::to_vec(value).map_err(|error| SessionError::PayloadEncode {
        event_type: EventType::new(COMPACTION_CREATED_EVENT_TYPE),
        message: error.to_string(),
    })?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}
