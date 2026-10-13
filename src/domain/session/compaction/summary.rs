//! Versioned Session summaries with typed task and optional Research references.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::domain::artifacts::ArtifactId;
use crate::domain::context::{
    AuthorityLevel, ContextOrigin, ContextProvenance, ContextSlot, FreshnessIdentity,
    RetentionClass, SemanticClass,
};
use crate::domain::events::EventType;
use crate::domain::model::ContentItem;
use crate::domain::research::TargetSnapshotId;
use crate::domain::session::{
    COMPACTION_CREATED_EVENT_TYPE, CURRENT_SESSION_PAYLOAD_SCHEMA_VERSION, SessionError,
};

use super::{
    CURRENT_SESSION_SUMMARY_SCHEMA_VERSION, CompactionValidationContext, EvidenceSpine,
    SessionTaskReferences, fingerprint_value, require_text, validate_unique_ids,
};

const MAX_SESSION_SUMMARY_BYTES: usize = 8 * 1024;

/// Bounded lossy continuation prose. It is never parsed into Research facts.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NarrativeSummary {
    current_goal: String,
    concise_progress: String,
    unresolved_questions: Vec<String>,
    next_step_notes: Vec<String>,
}

impl NarrativeSummary {
    pub fn new(
        current_goal: impl Into<String>,
        concise_progress: impl Into<String>,
        unresolved_questions: Vec<String>,
        next_step_notes: Vec<String>,
    ) -> Result<Self, SessionError> {
        let narrative = Self {
            current_goal: current_goal.into(),
            concise_progress: concise_progress.into(),
            unresolved_questions,
            next_step_notes,
        };
        narrative.validate()?;
        Ok(narrative)
    }

    pub fn current_goal(&self) -> &str {
        &self.current_goal
    }

    pub fn concise_progress(&self) -> &str {
        &self.concise_progress
    }

    pub fn unresolved_questions(&self) -> &[String] {
        &self.unresolved_questions
    }

    pub fn next_step_notes(&self) -> &[String] {
        &self.next_step_notes
    }

    fn validate(&self) -> Result<(), SessionError> {
        require_text("narrative.current_goal", &self.current_goal)?;
        require_text("narrative.concise_progress", &self.concise_progress)?;
        validate_text_entries("narrative.unresolved_questions", &self.unresolved_questions)?;
        validate_text_entries("narrative.next_step_notes", &self.next_step_notes)
    }
}

/// Caller-supplied Summary content. Bound Sessions produce v3; the legacy
/// constructor retains exact v2 behavior for unbound historical consumers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionSummaryDraft {
    evidence_spine: Option<EvidenceSpine>,
    task_references: Option<SessionTaskReferences>,
    narrative: NarrativeSummary,
}

impl SessionSummaryDraft {
    pub fn new(
        evidence_spine: EvidenceSpine,
        narrative: NarrativeSummary,
    ) -> Result<Self, SessionError> {
        let draft = Self {
            evidence_spine: Some(evidence_spine),
            task_references: None,
            narrative,
        };
        draft.validate_intrinsic()?;
        Ok(draft)
    }

    pub fn evidence_spine(&self) -> Option<&EvidenceSpine> {
        self.evidence_spine.as_ref()
    }

    pub(crate) fn for_task(
        task_references: SessionTaskReferences,
        evidence_spine: Option<EvidenceSpine>,
        narrative: NarrativeSummary,
    ) -> Result<Self, SessionError> {
        let draft = Self {
            evidence_spine,
            task_references: Some(task_references),
            narrative,
        };
        draft.validate_intrinsic()?;
        Ok(draft)
    }

    pub fn narrative(&self) -> &NarrativeSummary {
        &self.narrative
    }

    pub(crate) fn task_references(&self) -> Option<&SessionTaskReferences> {
        self.task_references.as_ref()
    }

    fn validate_intrinsic(&self) -> Result<(), SessionError> {
        if let Some(spine) = &self.evidence_spine {
            spine.validate_intrinsic()?;
        }
        if let Some(task) = &self.task_references {
            task.validate_intrinsic()?;
        }
        self.narrative.validate()
    }

    fn validate_against(
        &self,
        context: &CompactionValidationContext<'_>,
    ) -> Result<(), SessionError> {
        self.validate_intrinsic()?;
        if let Some(spine) = &self.evidence_spine {
            spine.validate_against(context)?;
        }
        if let Some(task) = &self.task_references {
            task.validate_against(context.task.ok_or_else(|| {
                super::task::invalid_task("task Summary requires exact Session validation facts")
            })?)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SessionSummaryV2 {
    schema_version: u16,
    revision: u64,
    fingerprint: String,
    evidence_spine: EvidenceSpine,
    narrative: NarrativeSummary,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SessionSummaryV3 {
    schema_version: u16,
    revision: u64,
    fingerprint: String,
    task_references: SessionTaskReferences,
    evidence_spine: Option<EvidenceSpine>,
    narrative: NarrativeSummary,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct LegacySessionSummaryV1 {
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
    unfinished_execution: Vec<LegacyUnfinishedExecutionRef>,
    next_steps: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
enum LegacyUnfinishedExecutionRef {
    Execution(crate::domain::tools::ExecutionId),
    Job(crate::domain::tools::JobId),
    Pty(crate::domain::tools::PtySessionId),
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum SessionSummaryVersion {
    LegacyV1(LegacySessionSummaryV1),
    CurrentV2(SessionSummaryV2),
    CurrentV3(Box<SessionSummaryV3>),
}

/// Deletable derived Summary carried by one Compaction fact.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionSummary {
    version: SessionSummaryVersion,
}

impl Serialize for SessionSummary {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match &self.version {
            SessionSummaryVersion::LegacyV1(summary) => summary.serialize(serializer),
            SessionSummaryVersion::CurrentV2(summary) => summary.serialize(serializer),
            SessionSummaryVersion::CurrentV3(summary) => summary.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for SessionSummary {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        let schema_version = value
            .get("schema_version")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                serde::de::Error::custom("Session Summary schema_version is required")
            })?;
        let version = match schema_version {
            1 => serde_json::from_value(value)
                .map(SessionSummaryVersion::LegacyV1)
                .map_err(serde::de::Error::custom)?,
            2 => serde_json::from_value(value)
                .map(SessionSummaryVersion::CurrentV2)
                .map_err(serde::de::Error::custom)?,
            3 => serde_json::from_value::<SessionSummaryV3>(value)
                .map(Box::new)
                .map(SessionSummaryVersion::CurrentV3)
                .map_err(serde::de::Error::custom)?,
            _ => {
                return Err(serde::de::Error::custom(format!(
                    "unsupported Session Summary schema version {schema_version}"
                )));
            }
        };
        Ok(Self { version })
    }
}

impl SessionSummary {
    pub(crate) fn from_draft(
        draft: SessionSummaryDraft,
        revision: u64,
        context: &CompactionValidationContext<'_>,
    ) -> Result<Self, SessionError> {
        draft.validate_against(context)?;
        let version = if let Some(task_references) = draft.task_references {
            let mut summary = SessionSummaryV3 {
                schema_version: CURRENT_SESSION_SUMMARY_SCHEMA_VERSION,
                revision,
                fingerprint: String::new(),
                task_references,
                evidence_spine: draft.evidence_spine,
                narrative: draft.narrative,
            };
            summary.fingerprint = expected_v3_fingerprint(&summary)?;
            SessionSummaryVersion::CurrentV3(Box::new(summary))
        } else {
            let mut summary = SessionSummaryV2 {
                schema_version: 2,
                revision,
                fingerprint: String::new(),
                evidence_spine: draft
                    .evidence_spine
                    .expect("legacy draft contains its EvidenceSpine"),
                narrative: draft.narrative,
            };
            summary.fingerprint = expected_v2_fingerprint(&summary)?;
            SessionSummaryVersion::CurrentV2(summary)
        };
        let summary = Self { version };
        summary.validate_shape()?;
        summary.validate_encoded_size()?;
        Ok(summary)
    }

    pub(super) fn validate_shape(&self) -> Result<(), SessionError> {
        match &self.version {
            SessionSummaryVersion::LegacyV1(summary) => {
                if summary.schema_version != CURRENT_SESSION_PAYLOAD_SCHEMA_VERSION {
                    return Err(unsupported_summary_schema(summary.schema_version));
                }
                validate_legacy_summary(summary)?;
            }
            SessionSummaryVersion::CurrentV2(summary) => {
                if summary.schema_version != 2 {
                    return Err(unsupported_summary_schema(summary.schema_version));
                }
                summary.evidence_spine.validate_intrinsic()?;
                summary.narrative.validate()?;
                if summary.fingerprint != expected_v2_fingerprint(summary)? {
                    return Err(SessionError::CompactionFingerprintMismatch);
                }
            }
            SessionSummaryVersion::CurrentV3(summary) => {
                if summary.schema_version != CURRENT_SESSION_SUMMARY_SCHEMA_VERSION {
                    return Err(unsupported_summary_schema(summary.schema_version));
                }
                summary.task_references.validate_intrinsic()?;
                if let Some(spine) = &summary.evidence_spine {
                    spine.validate_intrinsic()?;
                }
                summary.narrative.validate()?;
                if summary.fingerprint != expected_v3_fingerprint(summary)? {
                    return Err(SessionError::CompactionFingerprintMismatch);
                }
            }
        }
        self.validate_encoded_size()
    }

    pub const fn schema_version(&self) -> u16 {
        match &self.version {
            SessionSummaryVersion::LegacyV1(_) => 1,
            SessionSummaryVersion::CurrentV2(_) => 2,
            SessionSummaryVersion::CurrentV3(_) => 3,
        }
    }

    pub const fn revision(&self) -> u64 {
        match &self.version {
            SessionSummaryVersion::LegacyV1(summary) => summary.revision,
            SessionSummaryVersion::CurrentV2(summary) => summary.revision,
            SessionSummaryVersion::CurrentV3(summary) => summary.revision,
        }
    }

    pub fn fingerprint(&self) -> &str {
        match &self.version {
            SessionSummaryVersion::LegacyV1(summary) => &summary.fingerprint,
            SessionSummaryVersion::CurrentV2(summary) => &summary.fingerprint,
            SessionSummaryVersion::CurrentV3(summary) => &summary.fingerprint,
        }
    }

    pub fn evidence_spine(&self) -> Option<&EvidenceSpine> {
        match &self.version {
            SessionSummaryVersion::LegacyV1(_) => None,
            SessionSummaryVersion::CurrentV2(summary) => Some(&summary.evidence_spine),
            SessionSummaryVersion::CurrentV3(summary) => summary.evidence_spine.as_ref(),
        }
    }

    pub fn narrative(&self) -> Option<&NarrativeSummary> {
        match &self.version {
            SessionSummaryVersion::LegacyV1(_) => None,
            SessionSummaryVersion::CurrentV2(summary) => Some(&summary.narrative),
            SessionSummaryVersion::CurrentV3(summary) => Some(&summary.narrative),
        }
    }

    pub(crate) fn task_references(&self) -> Option<&SessionTaskReferences> {
        match &self.version {
            SessionSummaryVersion::CurrentV3(summary) => Some(&summary.task_references),
            _ => None,
        }
    }

    pub(crate) fn validate_references(
        &self,
        context: &CompactionValidationContext<'_>,
    ) -> Result<(), SessionError> {
        match &self.version {
            SessionSummaryVersion::LegacyV1(_) => Ok(()),
            SessionSummaryVersion::CurrentV2(summary) => {
                summary.evidence_spine.validate_against(context)
            }
            SessionSummaryVersion::CurrentV3(summary) => {
                summary
                    .task_references
                    .validate_against(context.task.ok_or_else(|| {
                        super::task::invalid_task(
                            "task Summary requires exact Session validation facts",
                        )
                    })?)?;
                if let Some(spine) = &summary.evidence_spine {
                    spine.validate_against(context)?;
                }
                Ok(())
            }
        }
    }

    pub(crate) const fn requires_reference_validation(&self) -> bool {
        !matches!(self.version, SessionSummaryVersion::LegacyV1(_))
    }

    pub(crate) fn context_item(&self) -> Result<ContentItem, SessionError> {
        let payload = serde_json::to_string(self).map_err(|error| SessionError::PayloadEncode {
            event_type: EventType::new(COMPACTION_CREATED_EVENT_TYPE),
            message: error.to_string(),
        })?;
        let prefix = format!("derived_session_summary_v{}:", self.schema_version());
        Ok(ContentItem::user_text(format!("{prefix}{payload}")))
    }

    pub(crate) fn context_provenance(&self) -> Result<Option<ContextProvenance>, SessionError> {
        if let Some(task) = self.task_references() {
            let references = task.context_references(
                self.evidence_spine()
                    .map(EvidenceSpine::context_references)
                    .transpose()?,
            )?;
            let freshness = FreshnessIdentity::new(task.source().stream_hash())
                .map_err(context_provenance_error)?
                .with_producer_version("session-summary:3");
            return ContextProvenance::new(
                ContextSlot::Summary,
                ContextOrigin::SessionProjection,
                SemanticClass::DerivedProjection,
                AuthorityLevel::FactReference,
                RetentionClass::EvidenceSpine,
                freshness,
                references,
            )
            .map(Some)
            .map_err(context_provenance_error);
        }
        let Some(spine) = self.evidence_spine() else {
            return Ok(None);
        };
        let freshness = FreshnessIdentity::new(spine.source().stream_hash())
            .map_err(context_provenance_error)?
            .with_target_snapshot(spine.target_snapshot_refs()[0].clone())
            .with_producer_version("session-summary:2");
        ContextProvenance::new(
            ContextSlot::Summary,
            ContextOrigin::SessionProjection,
            SemanticClass::DerivedProjection,
            AuthorityLevel::FactReference,
            RetentionClass::EvidenceSpine,
            freshness,
            spine.context_references()?,
        )
        .map(Some)
        .map_err(context_provenance_error)
    }

    pub(crate) fn matches_draft(
        &self,
        draft: &SessionSummaryDraft,
        context: &CompactionValidationContext<'_>,
    ) -> Result<bool, SessionError> {
        Ok(Self::from_draft(draft.clone(), self.revision(), context)? == *self)
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
}

fn validate_legacy_summary(summary: &LegacySessionSummaryV1) -> Result<(), SessionError> {
    for (field, value) in [
        ("goal", summary.goal.as_str()),
        ("target", summary.target.as_str()),
        ("target_version", summary.target_version.as_str()),
        ("environment", summary.environment.as_str()),
    ] {
        require_text(field, value)?;
    }
    require_text("target_snapshot", summary.target_snapshot.as_str())?;
    validate_text_entries("verified_facts", &summary.verified_facts)?;
    validate_text_entries("active_hypotheses", &summary.active_hypotheses)?;
    validate_text_entries("failed_paths", &summary.failed_paths)?;
    validate_unique_ids(
        "artifacts",
        summary.artifacts.iter().map(ArtifactId::as_str),
    )?;
    validate_text_entries("modified_files", &summary.modified_files)?;
    validate_unique_ids(
        "unfinished_execution",
        summary.unfinished_execution.iter().map(|item| match item {
            LegacyUnfinishedExecutionRef::Execution(id) => id.as_str(),
            LegacyUnfinishedExecutionRef::Job(id) => id.as_str(),
            LegacyUnfinishedExecutionRef::Pty(id) => id.as_str(),
        }),
    )?;
    validate_text_entries("next_steps", &summary.next_steps)?;
    if summary.fingerprint != expected_legacy_fingerprint(summary)? {
        return Err(SessionError::CompactionFingerprintMismatch);
    }
    Ok(())
}

fn unsupported_summary_schema(schema_version: u16) -> SessionError {
    SessionError::UnsupportedPayloadSchema {
        event_type: EventType::new(COMPACTION_CREATED_EVENT_TYPE),
        schema_version,
    }
}

fn expected_v2_fingerprint(value: &SessionSummaryV2) -> Result<String, SessionError> {
    let mut value = value.clone();
    value.clear_fingerprint();
    fingerprint_value(&value)
}

fn expected_v3_fingerprint(value: &SessionSummaryV3) -> Result<String, SessionError> {
    let mut value = value.clone();
    value.fingerprint.clear();
    fingerprint_value(&value)
}

fn expected_legacy_fingerprint(value: &LegacySessionSummaryV1) -> Result<String, SessionError> {
    let mut value = value.clone();
    value.clear_fingerprint();
    let bytes = serde_json::to_vec(&value).map_err(|error| SessionError::PayloadEncode {
        event_type: EventType::new(COMPACTION_CREATED_EVENT_TYPE),
        message: error.to_string(),
    })?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}

trait ClearFingerprint {
    fn clear_fingerprint(&mut self);
}

impl ClearFingerprint for SessionSummaryV2 {
    fn clear_fingerprint(&mut self) {
        self.fingerprint.clear();
    }
}

impl ClearFingerprint for LegacySessionSummaryV1 {
    fn clear_fingerprint(&mut self) {
        self.fingerprint.clear();
    }
}

fn validate_text_entries(field: &'static str, values: &[String]) -> Result<(), SessionError> {
    validate_unique_ids(field, values.iter().map(String::as_str))
}

fn context_provenance_error(error: impl std::fmt::Display) -> SessionError {
    SessionError::CompactionContextProvenance {
        message: error.to_string(),
    }
}
