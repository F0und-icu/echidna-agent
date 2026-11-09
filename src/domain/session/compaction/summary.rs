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
    CompactionValidationContext, EvidenceSpine, SessionTaskReferences, fingerprint_value,
    require_text, validate_unique_ids,
};

const MAX_SESSION_SUMMARY_BYTES: usize = 8 * 1024;
const MAX_EXTENDED_SESSION_SUMMARY_BYTES: usize = 64 * 1024;

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

    pub(super) fn validate(&self) -> Result<(), SessionError> {
        require_text("narrative.current_goal", &self.current_goal)?;
        require_text("narrative.concise_progress", &self.concise_progress)?;
        validate_text_entries("narrative.unresolved_questions", &self.unresolved_questions)?;
        validate_text_entries("narrative.next_step_notes", &self.next_step_notes)
    }
}

/// Caller-supplied Summary content. Original limits preserve v2/v3 creation;
/// extended limits produce v4 without changing historical reader semantics.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionSummaryDraft {
    evidence_spine: Option<EvidenceSpine>,
    task_references: Option<SessionTaskReferences>,
    narrative: NarrativeSummary,
    byte_limit: usize,
    graph: Option<super::VerifiedCompactionGraph>,
    preview: bool,
}

impl SessionSummaryDraft {
    pub(crate) fn v6_handoff_byte_limit(
        goal: &str,
        byte_limit: usize,
        minimum_handoff_bytes: usize,
    ) -> Result<usize, SessionError> {
        let narrative = NarrativeSummary::new(goal, "a", Vec::new(), Vec::new())?;
        let minimum_narrative_bytes = serde_json::to_vec(&narrative)
            .map_err(|error| super::task::invalid_task(error.to_string()))?
            .len();
        let remaining = byte_limit.checked_sub(minimum_narrative_bytes).ok_or(
            SessionError::CompactionSummaryTooLarge {
                actual: minimum_narrative_bytes,
                maximum: byte_limit,
            },
        )?;
        remaining
            .checked_add(minimum_handoff_bytes)
            .ok_or(SessionError::StateOverflow {
                field: "v6 handoff byte limit",
            })
    }
    pub fn new(
        evidence_spine: EvidenceSpine,
        narrative: NarrativeSummary,
    ) -> Result<Self, SessionError> {
        let draft = Self {
            evidence_spine: Some(evidence_spine),
            task_references: None,
            narrative,
            byte_limit: MAX_SESSION_SUMMARY_BYTES,
            graph: None,
            preview: false,
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
            byte_limit: MAX_SESSION_SUMMARY_BYTES,
            graph: None,
            preview: false,
        };
        draft.validate_intrinsic()?;
        Ok(draft)
    }

    pub fn narrative(&self) -> &NarrativeSummary {
        &self.narrative
    }

    pub(crate) fn with_verified_graph(mut self, graph: super::VerifiedCompactionGraph) -> Self {
        self.narrative = graph.narrative.clone();
        self.graph = Some(graph);
        self.preview = false;
        self
    }

    /// A local capacity probe has no model sample and can never be serialized
    /// as a durable Summary. The real v6 candidate is checked after sampling.
    pub(crate) fn with_preview_byte_limit(
        mut self,
        byte_limit: usize,
    ) -> Result<Self, SessionError> {
        if byte_limit == 0 {
            return Err(super::task::invalid_task(
                "preview Summary byte limit must be positive",
            ));
        }
        self.byte_limit = byte_limit;
        self.preview = true;
        Ok(self)
    }

    pub(crate) fn with_historical_narrative(mut self, narrative: NarrativeSummary) -> Self {
        self.narrative = narrative;
        self
    }

    pub(crate) fn with_byte_limit(mut self, byte_limit: usize) -> Result<Self, SessionError> {
        if self.graph.is_some() {
            if byte_limit == 0 {
                return Err(super::task::invalid_task(
                    "v6 Summary byte limit must be positive",
                ));
            }
        } else {
            validate_byte_limit(byte_limit)?;
        }
        self.byte_limit = byte_limit;
        Ok(self)
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
        if let Some(graph) = &self.graph {
            let recorded = context
                .task
                .and_then(|facts| facts.verified_graph.as_ref())
                .ok_or_else(|| {
                    super::task::invalid_task("v6 Summary requires a verified Compaction graph")
                })?;
            if graph.root_event_id != recorded.root_event_id
                || graph.plan_hash != recorded.plan_hash
                || graph.plan_blob_id != recorded.plan_blob_id
                || graph.source != recorded.source
                || graph.narrative != recorded.narrative
                || graph.task_selection_event != recorded.task_selection_event
                || graph.task_id != recorded.task_id
                || graph.task_scope != recorded.task_scope
                || graph.summarized_history != recorded.summarized_history
                || self.narrative != graph.narrative
            {
                return Err(super::task::invalid_task("v6 Summary graph proof mismatch"));
            }
            if let Some(task) = &self.task_references {
                task.validate_graph_projection(recorded)?;
            }
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
struct SessionSummaryV4 {
    schema_version: u16,
    revision: u64,
    fingerprint: String,
    byte_limit: usize,
    task_references: Option<SessionTaskReferences>,
    evidence_spine: Option<EvidenceSpine>,
    narrative: NarrativeSummary,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SessionSummaryV5 {
    schema_version: u16,
    revision: u64,
    fingerprint: String,
    byte_limit: usize,
    task_references: SessionTaskReferences,
    evidence_spine: Option<EvidenceSpine>,
    narrative: NarrativeSummary,
    model_sample_event_ref: crate::domain::events::EventId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ProtectedInputReference {
    anchor: crate::domain::events::EventId,
    event_id: crate::domain::events::EventId,
    content_hash: String,
    provenance: ContextProvenance,
}

fn protected_references(
    graph: &super::VerifiedCompactionGraph,
) -> Result<Vec<ProtectedInputReference>, SessionError> {
    protected_references_from_inputs(&graph.protected_inputs)
}

fn protected_references_from_inputs(
    inputs: &[super::super::operator_boundary::ProtectedOperatorInput],
) -> Result<Vec<ProtectedInputReference>, SessionError> {
    inputs
        .iter()
        .map(|input| {
            Ok(ProtectedInputReference {
                anchor: input.anchor.clone(),
                event_id: input.event_id.clone(),
                content_hash: fingerprint_value(&input.item)?,
                provenance: input.provenance.clone(),
            })
        })
        .collect()
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SessionSummaryV6 {
    schema_version: u16,
    revision: u64,
    fingerprint: String,
    byte_limit: usize,
    task_references: SessionTaskReferences,
    evidence_spine: Option<EvidenceSpine>,
    narrative: NarrativeSummary,
    narrative_source: super::SessionSourceIdentity,
    model_sample_event_ref: crate::domain::events::EventId,
    source_plan_blob_id: crate::domain::artifacts::BlobId,
    source_plan_hash: String,
    protected_inputs: Vec<ProtectedInputReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    task_selection_event: Option<crate::domain::events::EventId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    task_scope: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    summarized_history: Vec<crate::domain::events::EventId>,
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
    CurrentV4(Box<SessionSummaryV4>),
    CurrentV5(Box<SessionSummaryV5>),
    CurrentV6(Box<SessionSummaryV6>),
    CurrentV7(Box<SessionSummaryV6>),
    Preview(Box<SessionSummaryV4>),
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
            SessionSummaryVersion::CurrentV4(summary) => summary.serialize(serializer),
            SessionSummaryVersion::CurrentV5(summary) => summary.serialize(serializer),
            SessionSummaryVersion::CurrentV6(summary)
            | SessionSummaryVersion::CurrentV7(summary) => summary.serialize(serializer),
            SessionSummaryVersion::Preview(_) => Err(serde::ser::Error::custom(
                "a compaction preview cannot be persisted",
            )),
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
            4 => serde_json::from_value::<SessionSummaryV4>(value)
                .map(Box::new)
                .map(SessionSummaryVersion::CurrentV4)
                .map_err(serde::de::Error::custom)?,
            5 => serde_json::from_value::<SessionSummaryV5>(value)
                .map(Box::new)
                .map(SessionSummaryVersion::CurrentV5)
                .map_err(serde::de::Error::custom)?,
            6 => serde_json::from_value::<SessionSummaryV6>(value)
                .map(Box::new)
                .map(SessionSummaryVersion::CurrentV6)
                .map_err(serde::de::Error::custom)?,
            7 => serde_json::from_value::<SessionSummaryV6>(value)
                .map(Box::new)
                .map(SessionSummaryVersion::CurrentV7)
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
    pub(crate) fn is_preview(&self) -> bool {
        matches!(self.version, SessionSummaryVersion::Preview(_))
    }
    /// Historical refresh preserves the byte contract recorded by the old
    /// Summary; versions without a recorded limit use their historical default.
    pub(crate) fn historical_byte_limit(&self) -> Option<usize> {
        match &self.version {
            SessionSummaryVersion::LegacyV1(_)
            | SessionSummaryVersion::CurrentV2(_)
            | SessionSummaryVersion::CurrentV3(_) => Some(MAX_SESSION_SUMMARY_BYTES),
            SessionSummaryVersion::CurrentV4(summary) => Some(summary.byte_limit),
            SessionSummaryVersion::CurrentV5(summary) => Some(summary.byte_limit),
            SessionSummaryVersion::CurrentV6(summary)
            | SessionSummaryVersion::CurrentV7(summary) => Some(summary.byte_limit),
            SessionSummaryVersion::Preview(_) => None,
        }
    }

    pub(crate) fn from_draft(
        draft: SessionSummaryDraft,
        revision: u64,
        context: &CompactionValidationContext<'_>,
    ) -> Result<Self, SessionError> {
        draft.validate_against(context)?;
        let version = if draft.preview {
            let mut summary = SessionSummaryV4 {
                schema_version: 4,
                revision,
                fingerprint: String::new(),
                byte_limit: draft.byte_limit,
                task_references: draft.task_references,
                evidence_spine: draft.evidence_spine,
                narrative: draft.narrative,
            };
            summary.fingerprint = expected_v4_fingerprint(&summary)?;
            SessionSummaryVersion::Preview(Box::new(summary))
        } else if let Some(graph) = &draft.graph {
            let mut summary = SessionSummaryV6 {
                schema_version: if graph.task_selection_event.is_some() {
                    7
                } else {
                    6
                },
                revision,
                fingerprint: String::new(),
                byte_limit: draft.byte_limit,
                task_references: draft.task_references.ok_or_else(|| {
                    super::task::invalid_task("v6 Summary requires task provenance")
                })?,
                evidence_spine: draft.evidence_spine,
                narrative: draft.narrative,
                narrative_source: graph.source.clone(),
                model_sample_event_ref: graph.root_event_id.clone(),
                source_plan_blob_id: graph.plan_blob_id.clone(),
                source_plan_hash: graph.plan_hash.clone(),
                protected_inputs: protected_references(graph)?,
                task_selection_event: graph.task_selection_event.clone(),
                task_id: graph.task_id.clone(),
                task_scope: graph.task_scope.clone(),
                summarized_history: graph.summarized_history.clone(),
            };
            summary.fingerprint = expected_v6_fingerprint(&summary)?;
            if summary.schema_version == 7 {
                SessionSummaryVersion::CurrentV7(Box::new(summary))
            } else {
                SessionSummaryVersion::CurrentV6(Box::new(summary))
            }
        } else if draft.byte_limit > MAX_SESSION_SUMMARY_BYTES {
            let mut summary = SessionSummaryV4 {
                schema_version: 4,
                revision,
                fingerprint: String::new(),
                byte_limit: draft.byte_limit,
                task_references: draft.task_references,
                evidence_spine: draft.evidence_spine,
                narrative: draft.narrative,
            };
            summary.fingerprint = expected_v4_fingerprint(&summary)?;
            SessionSummaryVersion::CurrentV4(Box::new(summary))
        } else if let Some(task_references) = draft.task_references {
            let mut summary = SessionSummaryV3 {
                schema_version: 3,
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
                if summary.schema_version != 3 {
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
            SessionSummaryVersion::CurrentV4(summary) => {
                if summary.schema_version != 4 {
                    return Err(unsupported_summary_schema(summary.schema_version));
                }
                validate_byte_limit(summary.byte_limit)?;
                if summary.task_references.is_none() && summary.evidence_spine.is_none() {
                    return Err(SessionError::CompactionSummaryFieldRequired {
                        field: "summary.references",
                    });
                }
                if let Some(task) = &summary.task_references {
                    task.validate_intrinsic()?;
                }
                if let Some(spine) = &summary.evidence_spine {
                    spine.validate_intrinsic()?;
                }
                summary.narrative.validate()?;
                if summary.fingerprint != expected_v4_fingerprint(summary)? {
                    return Err(SessionError::CompactionFingerprintMismatch);
                }
            }
            SessionSummaryVersion::CurrentV5(summary) => {
                if summary.schema_version != 5 {
                    return Err(unsupported_summary_schema(summary.schema_version));
                }
                validate_byte_limit(summary.byte_limit)?;
                summary.task_references.validate_intrinsic()?;
                if let Some(spine) = &summary.evidence_spine {
                    spine.validate_intrinsic()?;
                }
                summary.narrative.validate()?;
                require_text(
                    "summary.model_sample_event_ref",
                    summary.model_sample_event_ref.as_str(),
                )?;
                if summary.fingerprint != expected_v5_fingerprint(summary)? {
                    return Err(SessionError::CompactionFingerprintMismatch);
                }
            }
            SessionSummaryVersion::CurrentV6(summary)
            | SessionSummaryVersion::CurrentV7(summary) => {
                let task_scoped = matches!(self.version, SessionSummaryVersion::CurrentV7(_));
                if summary.schema_version != if task_scoped { 7 } else { 6 }
                    || summary.byte_limit == 0
                    || task_scoped
                        != (summary.task_selection_event.is_some()
                            && summary
                                .task_id
                                .as_ref()
                                .is_some_and(|id| !id.trim().is_empty()))
                    || (!task_scoped
                        && (summary.task_selection_event.is_some()
                            || summary.task_id.is_some()
                            || !summary.task_scope.is_empty()
                            || !summary.summarized_history.is_empty()))
                    || (task_scoped
                        && (summary.task_scope.is_empty()
                            || summary
                                .task_id
                                .as_ref()
                                .is_none_or(|id| !summary.task_scope.contains(id))))
                {
                    return Err(unsupported_summary_schema(summary.schema_version));
                }
                summary.task_references.validate_intrinsic()?;
                summary.narrative_source.validate()?;
                if let Some(spine) = &summary.evidence_spine {
                    spine.validate_intrinsic()?;
                }
                summary.narrative.validate()?;
                require_text(
                    "summary.model_sample_event_ref",
                    summary.model_sample_event_ref.as_str(),
                )?;
                validate_unique_ids(
                    "summary.protected_inputs",
                    summary
                        .protected_inputs
                        .iter()
                        .map(|input| input.event_id.as_str()),
                )?;
                validate_unique_ids(
                    "summary.summarized_history",
                    summary.summarized_history.iter().map(|id| id.as_str()),
                )?;
                validate_unique_ids(
                    "summary.task_scope",
                    summary.task_scope.iter().map(String::as_str),
                )?;
                if summary.fingerprint != expected_v6_fingerprint(summary)? {
                    return Err(SessionError::CompactionFingerprintMismatch);
                }
            }
            SessionSummaryVersion::Preview(summary) => {
                if summary.byte_limit == 0 {
                    return Err(unsupported_summary_schema(summary.schema_version));
                }
                if let Some(task) = &summary.task_references {
                    task.validate_intrinsic()?;
                }
                if let Some(spine) = &summary.evidence_spine {
                    spine.validate_intrinsic()?;
                }
                summary.narrative.validate()?;
                if summary.fingerprint != expected_v4_fingerprint(summary)? {
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
            SessionSummaryVersion::CurrentV4(_) => 4,
            SessionSummaryVersion::CurrentV5(_) => 5,
            SessionSummaryVersion::CurrentV6(_) => 6,
            SessionSummaryVersion::CurrentV7(_) => 7,
            SessionSummaryVersion::Preview(_) => 4,
        }
    }

    pub const fn revision(&self) -> u64 {
        match &self.version {
            SessionSummaryVersion::LegacyV1(summary) => summary.revision,
            SessionSummaryVersion::CurrentV2(summary) => summary.revision,
            SessionSummaryVersion::CurrentV3(summary) => summary.revision,
            SessionSummaryVersion::CurrentV4(summary) => summary.revision,
            SessionSummaryVersion::CurrentV5(summary) => summary.revision,
            SessionSummaryVersion::CurrentV6(summary)
            | SessionSummaryVersion::CurrentV7(summary) => summary.revision,
            SessionSummaryVersion::Preview(summary) => summary.revision,
        }
    }

    pub fn fingerprint(&self) -> &str {
        match &self.version {
            SessionSummaryVersion::LegacyV1(summary) => &summary.fingerprint,
            SessionSummaryVersion::CurrentV2(summary) => &summary.fingerprint,
            SessionSummaryVersion::CurrentV3(summary) => &summary.fingerprint,
            SessionSummaryVersion::CurrentV4(summary) => &summary.fingerprint,
            SessionSummaryVersion::CurrentV5(summary) => &summary.fingerprint,
            SessionSummaryVersion::CurrentV6(summary)
            | SessionSummaryVersion::CurrentV7(summary) => &summary.fingerprint,
            SessionSummaryVersion::Preview(summary) => &summary.fingerprint,
        }
    }

    pub fn evidence_spine(&self) -> Option<&EvidenceSpine> {
        match &self.version {
            SessionSummaryVersion::LegacyV1(_) => None,
            SessionSummaryVersion::CurrentV2(summary) => Some(&summary.evidence_spine),
            SessionSummaryVersion::CurrentV3(summary) => summary.evidence_spine.as_ref(),
            SessionSummaryVersion::CurrentV4(summary) => summary.evidence_spine.as_ref(),
            SessionSummaryVersion::CurrentV5(summary) => summary.evidence_spine.as_ref(),
            SessionSummaryVersion::CurrentV6(summary)
            | SessionSummaryVersion::CurrentV7(summary) => summary.evidence_spine.as_ref(),
            SessionSummaryVersion::Preview(summary) => summary.evidence_spine.as_ref(),
        }
    }

    pub fn narrative(&self) -> Option<&NarrativeSummary> {
        match &self.version {
            SessionSummaryVersion::LegacyV1(_) => None,
            SessionSummaryVersion::CurrentV2(summary) => Some(&summary.narrative),
            SessionSummaryVersion::CurrentV3(summary) => Some(&summary.narrative),
            SessionSummaryVersion::CurrentV4(summary) => Some(&summary.narrative),
            SessionSummaryVersion::CurrentV5(summary) => Some(&summary.narrative),
            SessionSummaryVersion::CurrentV6(summary)
            | SessionSummaryVersion::CurrentV7(summary) => Some(&summary.narrative),
            SessionSummaryVersion::Preview(summary) => Some(&summary.narrative),
        }
    }

    pub(crate) fn task_references(&self) -> Option<&SessionTaskReferences> {
        match &self.version {
            SessionSummaryVersion::CurrentV3(summary) => Some(&summary.task_references),
            SessionSummaryVersion::CurrentV4(summary) => summary.task_references.as_ref(),
            SessionSummaryVersion::CurrentV5(summary) => Some(&summary.task_references),
            SessionSummaryVersion::CurrentV6(summary)
            | SessionSummaryVersion::CurrentV7(summary) => Some(&summary.task_references),
            SessionSummaryVersion::Preview(summary) => summary.task_references.as_ref(),
            _ => None,
        }
    }

    pub(crate) fn validate_references(
        &self,
        context: &CompactionValidationContext<'_>,
    ) -> Result<(), SessionError> {
        if let Some(task) = self.task_references() {
            task.validate_against(context.task.ok_or_else(|| {
                super::task::invalid_task("task Summary requires exact Session validation facts")
            })?)?;
        }
        if let Some(spine) = self.evidence_spine() {
            spine.validate_against(context)?;
        }
        if let SessionSummaryVersion::CurrentV5(summary) = &self.version {
            validate_model_sample(
                Some(&summary.model_sample_event_ref),
                &summary.narrative,
                context,
            )?;
        }
        if matches!(
            self.version,
            SessionSummaryVersion::CurrentV6(_) | SessionSummaryVersion::CurrentV7(_)
        ) {
            let graph = context
                .task
                .and_then(|facts| facts.verified_graph.as_ref())
                .ok_or_else(|| {
                    super::task::invalid_task("v6 Summary requires verified graph source")
                })?;
            self.validate_graph_reference(graph)?;
        }
        Ok(())
    }

    pub(crate) fn graph_reference(
        &self,
    ) -> Option<(
        &crate::domain::events::EventId,
        &super::SessionSourceIdentity,
        &crate::domain::artifacts::BlobId,
        &str,
    )> {
        match &self.version {
            SessionSummaryVersion::CurrentV6(summary)
            | SessionSummaryVersion::CurrentV7(summary) => Some((
                &summary.model_sample_event_ref,
                &summary.narrative_source,
                &summary.source_plan_blob_id,
                &summary.source_plan_hash,
            )),
            _ => None,
        }
    }

    pub(crate) fn validate_graph_reference(
        &self,
        graph: &super::VerifiedCompactionGraph,
    ) -> Result<(), SessionError> {
        let (SessionSummaryVersion::CurrentV6(summary) | SessionSummaryVersion::CurrentV7(summary)) =
            &self.version
        else {
            return Err(super::task::invalid_task("Summary has no v6 graph"));
        };
        if summary.model_sample_event_ref != graph.root_event_id
            || summary.narrative_source != graph.source
            || summary.source_plan_blob_id != graph.plan_blob_id
            || summary.source_plan_hash != graph.plan_hash
            || summary.narrative != graph.narrative
            || summary.protected_inputs != protected_references(graph)?
            || summary.task_selection_event != graph.task_selection_event
            || summary.task_id != graph.task_id
            || summary.task_scope != graph.task_scope
            || summary.summarized_history != graph.summarized_history
        {
            return Err(super::task::invalid_task(
                "v6 Summary source or protected inputs do not match verified graph",
            ));
        }
        summary.task_references.validate_graph_projection(graph)?;
        Ok(())
    }

    pub(crate) fn selected_task_id(&self) -> Option<&str> {
        match &self.version {
            SessionSummaryVersion::CurrentV7(summary) => summary.task_id.as_deref(),
            _ => None,
        }
    }

    pub(crate) fn task_selection_event(&self) -> Option<&crate::domain::events::EventId> {
        match &self.version {
            SessionSummaryVersion::CurrentV7(summary) => summary.task_selection_event.as_ref(),
            _ => None,
        }
    }

    pub(crate) fn summarized_history(&self) -> &[crate::domain::events::EventId] {
        match &self.version {
            SessionSummaryVersion::CurrentV7(summary) => &summary.summarized_history,
            _ => &[],
        }
    }

    pub(crate) fn task_scope(&self) -> &[String] {
        match &self.version {
            SessionSummaryVersion::CurrentV7(summary) => &summary.task_scope,
            _ => &[],
        }
    }

    pub(crate) const fn requires_reference_validation(&self) -> bool {
        !matches!(self.version, SessionSummaryVersion::LegacyV1(_))
    }

    pub(crate) fn context_item(&self) -> Result<ContentItem, SessionError> {
        let payload = if self.narrative().is_none() {
            serde_json::to_string(self)
        } else {
            serde_json::to_string(&self.model_projection()?)
        }
        .map_err(|error| SessionError::PayloadEncode {
            event_type: EventType::new(COMPACTION_CREATED_EVENT_TYPE),
            message: error.to_string(),
        })?;
        let prefix = format!("derived_session_summary_v{}:", self.schema_version());
        Ok(ContentItem::user_text(format!("{prefix}{payload}")))
    }

    pub(crate) fn model_projection(&self) -> Result<Value, SessionError> {
        #[derive(Serialize)]
        struct Projection<'a> {
            projection_version: u16,
            schema_version: u16,
            revision: u64,
            fingerprint: &'a str,
            source: Option<&'a super::task::SessionSourceIdentity>,
            evidence_spine: Option<&'a EvidenceSpine>,
            narrative: &'a NarrativeSummary,
        }
        let value = match self.narrative() {
            Some(narrative) => serde_json::to_value(Projection {
                projection_version: 1,
                schema_version: self.schema_version(),
                revision: self.revision(),
                fingerprint: self.fingerprint(),
                source: self.task_references().map(SessionTaskReferences::source),
                evidence_spine: self.evidence_spine(),
                narrative,
            }),
            None => serde_json::to_value(self),
        };
        value.map_err(|error| SessionError::PayloadEncode {
            event_type: EventType::new(COMPACTION_CREATED_EVENT_TYPE),
            message: error.to_string(),
        })
    }

    pub(crate) fn context_provenance(&self) -> Result<Option<ContextProvenance>, SessionError> {
        if let Some(task) = self.task_references() {
            let references = task.context_references(
                self.evidence_spine()
                    .map(EvidenceSpine::context_references)
                    .transpose()?,
                match &self.version {
                    SessionSummaryVersion::CurrentV5(summary) => {
                        Some(&summary.model_sample_event_ref)
                    }
                    SessionSummaryVersion::CurrentV6(summary)
                    | SessionSummaryVersion::CurrentV7(summary) => {
                        Some(&summary.model_sample_event_ref)
                    }
                    _ => None,
                },
            )?;
            let freshness = FreshnessIdentity::new(task.source().stream_hash())
                .map_err(context_provenance_error)?
                .with_producer_version(format!("session-summary:{}", self.schema_version()));
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
            .with_producer_version(format!("session-summary:{}", self.schema_version()));
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
        if matches!(self.version, SessionSummaryVersion::Preview(_)) {
            return Ok(());
        }
        let actual = serde_json::to_vec(self)
            .map_err(|error| SessionError::PayloadEncode {
                event_type: EventType::new(COMPACTION_CREATED_EVENT_TYPE),
                message: error.to_string(),
            })?
            .len();
        let maximum = match &self.version {
            SessionSummaryVersion::CurrentV4(summary) => summary.byte_limit,
            SessionSummaryVersion::CurrentV5(summary) => summary.byte_limit,
            SessionSummaryVersion::CurrentV6(summary)
            | SessionSummaryVersion::CurrentV7(summary) => {
                let narrative_bytes = serde_json::to_vec(&summary.narrative)
                    .map_err(|error| SessionError::PayloadEncode {
                        event_type: EventType::new(COMPACTION_CREATED_EVENT_TYPE),
                        message: error.to_string(),
                    })?
                    .len();
                actual
                    .checked_sub(narrative_bytes)
                    .and_then(|metadata| metadata.checked_add(summary.byte_limit))
                    .ok_or(SessionError::StateOverflow {
                        field: "v6 Summary byte allowance",
                    })?
            }
            _ => MAX_SESSION_SUMMARY_BYTES,
        };
        if actual > maximum {
            Err(SessionError::CompactionSummaryTooLarge { actual, maximum })
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

fn expected_v4_fingerprint(value: &SessionSummaryV4) -> Result<String, SessionError> {
    let mut value = value.clone();
    value.fingerprint.clear();
    fingerprint_value(&value)
}

pub(super) fn validate_byte_limit(byte_limit: usize) -> Result<(), SessionError> {
    if !(MAX_SESSION_SUMMARY_BYTES..=MAX_EXTENDED_SESSION_SUMMARY_BYTES).contains(&byte_limit) {
        return Err(SessionError::CompactionSummaryByteLimit {
            actual: byte_limit,
            minimum: MAX_SESSION_SUMMARY_BYTES,
            maximum: MAX_EXTENDED_SESSION_SUMMARY_BYTES,
        });
    }
    Ok(())
}

fn expected_v5_fingerprint(value: &SessionSummaryV5) -> Result<String, SessionError> {
    let mut value = value.clone();
    value.fingerprint.clear();
    fingerprint_value(&value)
}

fn expected_v6_fingerprint(value: &SessionSummaryV6) -> Result<String, SessionError> {
    let mut value = value.clone();
    value.fingerprint.clear();
    fingerprint_value(&value)
}

fn validate_model_sample(
    event_id: Option<&crate::domain::events::EventId>,
    narrative: &NarrativeSummary,
    context: &CompactionValidationContext<'_>,
) -> Result<(), SessionError> {
    if let Some(event_id) = event_id
        && context
            .task
            .and_then(|facts| facts.model_samples.get(event_id))
            .is_none_or(|recorded| recorded != narrative)
    {
        return Err(super::task::invalid_task(
            "Summary does not match its recorded model handoff",
        ));
    }
    Ok(())
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
