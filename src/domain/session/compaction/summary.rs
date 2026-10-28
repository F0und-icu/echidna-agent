//! Session Summary v1 compatibility and reference-only v2 representation.

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
    fingerprint_value, require_text, validate_unique_ids,
};

use super::{ResearchSourceIdentity, SessionSourceIdentity};

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

/// Caller-supplied current Summary content. Every new Compaction uses schema
/// v2; schema v1 exists only in historical Event decoding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionSummaryDraft {
    evidence_spine: EvidenceSpine,
    narrative: NarrativeSummary,
    byte_limit: usize,
}

impl SessionSummaryDraft {
    pub fn new(
        evidence_spine: EvidenceSpine,
        narrative: NarrativeSummary,
    ) -> Result<Self, SessionError> {
        let draft = Self {
            evidence_spine,
            narrative,
            byte_limit: MAX_SESSION_SUMMARY_BYTES,
        };
        draft.validate_intrinsic()?;
        Ok(draft)
    }

    pub fn evidence_spine(&self) -> &EvidenceSpine {
        &self.evidence_spine
    }

    pub fn narrative(&self) -> &NarrativeSummary {
        &self.narrative
    }

    pub(crate) fn with_byte_limit(mut self, byte_limit: usize) -> Result<Self, SessionError> {
        validate_byte_limit(byte_limit)?;
        self.byte_limit = byte_limit;
        Ok(self)
    }

    fn validate_intrinsic(&self) -> Result<(), SessionError> {
        self.evidence_spine.validate_intrinsic()?;
        self.narrative.validate()
    }

    fn validate_against(
        &self,
        context: &CompactionValidationContext<'_>,
    ) -> Result<(), SessionError> {
        self.validate_intrinsic()?;
        self.evidence_spine.validate_against(context)
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
struct SessionSummaryV4 {
    schema_version: u16,
    revision: u64,
    fingerprint: String,
    byte_limit: usize,
    evidence_spine: EvidenceSpine,
    narrative: NarrativeSummary,
}

/// Main-only v5: the format marker makes the unrelated persona v5/v6
/// impossible to decode as this contract even when fields happen to overlap.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SessionSummaryV5 {
    schema_version: u16,
    format: String,
    revision: u64,
    fingerprint: String,
    byte_limit: usize,
    session_source: SessionSourceIdentity,
    research_source: ResearchSourceIdentity,
    evidence_spine: Option<EvidenceSpine>,
    history_event_refs: Vec<crate::domain::events::EventId>,
    receipt_refs: Vec<crate::domain::research::ExecutionReceiptId>,
    artifact_refs: Vec<ArtifactId>,
    narrative: NarrativeSummary,
    narrative_source: SessionSourceIdentity,
    model_sample_event_ref: crate::domain::events::EventId,
    source_plan_blob_id: crate::domain::artifacts::BlobId,
    source_plan_hash: String,
    protected_inputs: Vec<ProtectedInputReference>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ProtectedInputReference {
    anchor: crate::domain::events::EventId,
    event_id: crate::domain::events::EventId,
    content_hash: String,
    provenance: ContextProvenance,
}

/// Disposable verification view derived from a strict Session prefix and an
/// independently reconstructed compaction graph.
pub(crate) struct MainV5ValidationFacts {
    pub(crate) source: SessionSourceIdentity,
    pub(crate) research_source: ResearchSourceIdentity,
    pub(crate) history_ids: std::collections::BTreeSet<crate::domain::events::EventId>,
    history_order: Vec<crate::domain::events::EventId>,
    pub(crate) receipt_ids: std::collections::BTreeSet<crate::domain::research::ExecutionReceiptId>,
    pub(crate) artifact_ids: std::collections::BTreeSet<ArtifactId>,
    pub(crate) graph: super::VerifiedCompactionGraph,
}

impl MainV5ValidationFacts {
    pub(crate) fn from_verified_sources(
        project_id: crate::domain::project::ProjectId,
        events: &[crate::domain::events::EventEnvelope<Value>],
        research: &super::ValidatedResearchSource,
        graph: super::VerifiedCompactionGraph,
    ) -> Result<Self, SessionError> {
        let (source, state) = SessionSourceIdentity::from_events(project_id, events)?;
        let graph_end = events.partition_point(|event| event.sequence <= graph.source.watermark());
        let (graph_source, _) =
            SessionSourceIdentity::from_events(source.project_id().clone(), &events[..graph_end])?;
        if graph.source != graph_source {
            return Err(SessionError::CompactionSourceInvalid {
                message: "verified graph belongs to a different Session source".to_owned(),
            });
        }
        let history_order: Vec<_> = state
            .history_items_with_event_ids()
            .map(|(id, _)| id.clone())
            .collect();
        Ok(Self {
            source,
            research_source: research.identity().clone(),
            history_ids: history_order.iter().cloned().collect(),
            history_order,
            receipt_ids: state
                .execution_receipts()
                .iter()
                .map(|record| {
                    crate::domain::research::ExecutionReceiptId::new(record.event_id.as_str())
                })
                .collect(),
            artifact_ids: research
                .state()
                .retrieval_artifact_records()
                .map(|record| record.id.clone())
                .collect(),
            graph,
        })
    }
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
    CurrentV4(Box<SessionSummaryV4>),
    MainV5(Box<SessionSummaryV5>),
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
            SessionSummaryVersion::CurrentV4(summary) => summary.serialize(serializer),
            SessionSummaryVersion::MainV5(summary) => summary.serialize(serializer),
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
            4 => serde_json::from_value::<SessionSummaryV4>(value)
                .map(Box::new)
                .map(SessionSummaryVersion::CurrentV4)
                .map_err(serde::de::Error::custom)?,
            5 => serde_json::from_value::<SessionSummaryV5>(value)
                .map(Box::new)
                .map(SessionSummaryVersion::MainV5)
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
    pub(crate) fn handoff_byte_limit(
        goal: &str,
        byte_limit: usize,
        minimum_handoff_bytes: usize,
    ) -> Result<usize, SessionError> {
        let narrative = NarrativeSummary::new(goal, "a", Vec::new(), Vec::new())?;
        let minimum = serde_json::to_vec(&narrative)
            .map_err(|error| SessionError::CompactionSourceInvalid {
                message: error.to_string(),
            })?
            .len();
        byte_limit
            .checked_sub(minimum)
            .ok_or(SessionError::CompactionSummaryTooLarge {
                actual: minimum,
                maximum: byte_limit,
            })?
            .checked_add(minimum_handoff_bytes)
            .ok_or(SessionError::StateOverflow {
                field: "main v5 handoff byte limit",
            })
    }
    pub(crate) fn from_verified_graph(
        revision: u64,
        facts: &MainV5ValidationFacts,
        evidence_spine: Option<EvidenceSpine>,
        byte_limit: usize,
        reference_limit: usize,
        context: &CompactionValidationContext<'_>,
    ) -> Result<Self, SessionError> {
        let graph = &facts.graph;
        let protected_inputs = graph
            .protected_inputs
            .iter()
            .map(|input| {
                Ok(ProtectedInputReference {
                    anchor: input.anchor.clone(),
                    event_id: input.event_id.clone(),
                    content_hash: fingerprint_value(&input.item)?,
                    provenance: input.provenance.clone(),
                })
            })
            .collect::<Result<Vec<_>, SessionError>>()?;
        let mut artifact_refs = std::collections::BTreeSet::new();
        for input in &graph.protected_inputs {
            artifact_refs.extend(
                input
                    .provenance
                    .references()
                    .artifact_refs()
                    .iter()
                    .cloned(),
            );
        }
        if let Some(spine) = &evidence_spine {
            artifact_refs.extend(spine.artifact_refs().iter().cloned());
        }
        let mut summary = SessionSummaryV5 {
            schema_version: 5,
            format: "echidna-research-main-v5".to_owned(),
            revision,
            fingerprint: String::new(),
            byte_limit,
            session_source: facts.source.clone(),
            research_source: facts.research_source.clone(),
            evidence_spine,
            history_event_refs: facts
                .history_order
                .iter()
                .rev()
                .take(reference_limit)
                .cloned()
                .collect(),
            receipt_refs: facts
                .receipt_ids
                .iter()
                .take(reference_limit)
                .cloned()
                .collect(),
            artifact_refs: artifact_refs.into_iter().collect(),
            narrative: graph.narrative.clone(),
            narrative_source: graph.source.clone(),
            model_sample_event_ref: graph.root_event_id.clone(),
            source_plan_blob_id: graph.plan_blob_id.clone(),
            source_plan_hash: graph.plan_hash.clone(),
            protected_inputs,
        };
        summary.fingerprint = fingerprint_value(&("echidna-research-main-v5", &summary))?;
        let summary = Self {
            version: SessionSummaryVersion::MainV5(Box::new(summary)),
        };
        summary.validate_shape()?;
        summary.validate_references(context)?;
        Ok(summary)
    }
    pub(crate) fn from_draft(
        draft: SessionSummaryDraft,
        revision: u64,
        context: &CompactionValidationContext<'_>,
    ) -> Result<Self, SessionError> {
        draft.validate_against(context)?;
        let version = if draft.byte_limit > MAX_SESSION_SUMMARY_BYTES {
            let mut summary = SessionSummaryV4 {
                schema_version: CURRENT_SESSION_SUMMARY_SCHEMA_VERSION,
                revision,
                fingerprint: String::new(),
                byte_limit: draft.byte_limit,
                evidence_spine: draft.evidence_spine,
                narrative: draft.narrative,
            };
            summary.fingerprint = expected_v4_fingerprint(&summary)?;
            SessionSummaryVersion::CurrentV4(Box::new(summary))
        } else {
            let mut summary = SessionSummaryV2 {
                schema_version: 2,
                revision,
                fingerprint: String::new(),
                evidence_spine: draft.evidence_spine,
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
            SessionSummaryVersion::CurrentV4(summary) => {
                if summary.schema_version != CURRENT_SESSION_SUMMARY_SCHEMA_VERSION {
                    return Err(unsupported_summary_schema(summary.schema_version));
                }
                validate_byte_limit(summary.byte_limit)?;
                summary.evidence_spine.validate_intrinsic()?;
                summary.narrative.validate()?;
                if summary.fingerprint != expected_v4_fingerprint(summary)? {
                    return Err(SessionError::CompactionFingerprintMismatch);
                }
            }
            SessionSummaryVersion::MainV5(summary) => validate_main_v5(summary)?,
        }
        self.validate_encoded_size()
    }

    pub const fn schema_version(&self) -> u16 {
        match &self.version {
            SessionSummaryVersion::LegacyV1(_) => 1,
            SessionSummaryVersion::CurrentV2(_) => 2,
            SessionSummaryVersion::CurrentV4(_) => 4,
            SessionSummaryVersion::MainV5(_) => 5,
        }
    }

    pub const fn revision(&self) -> u64 {
        match &self.version {
            SessionSummaryVersion::LegacyV1(summary) => summary.revision,
            SessionSummaryVersion::CurrentV2(summary) => summary.revision,
            SessionSummaryVersion::CurrentV4(summary) => summary.revision,
            SessionSummaryVersion::MainV5(summary) => summary.revision,
        }
    }

    pub(crate) fn byte_limit(&self) -> usize {
        match &self.version {
            SessionSummaryVersion::CurrentV4(summary) => summary.byte_limit,
            SessionSummaryVersion::MainV5(summary) => summary.byte_limit,
            _ => MAX_SESSION_SUMMARY_BYTES,
        }
    }

    pub fn fingerprint(&self) -> &str {
        match &self.version {
            SessionSummaryVersion::LegacyV1(summary) => &summary.fingerprint,
            SessionSummaryVersion::CurrentV2(summary) => &summary.fingerprint,
            SessionSummaryVersion::CurrentV4(summary) => &summary.fingerprint,
            SessionSummaryVersion::MainV5(summary) => &summary.fingerprint,
        }
    }

    pub fn evidence_spine(&self) -> Option<&EvidenceSpine> {
        match &self.version {
            SessionSummaryVersion::LegacyV1(_) => None,
            SessionSummaryVersion::CurrentV2(summary) => Some(&summary.evidence_spine),
            SessionSummaryVersion::CurrentV4(summary) => Some(&summary.evidence_spine),
            SessionSummaryVersion::MainV5(summary) => summary.evidence_spine.as_ref(),
        }
    }

    pub fn narrative(&self) -> Option<&NarrativeSummary> {
        match &self.version {
            SessionSummaryVersion::LegacyV1(_) => None,
            SessionSummaryVersion::CurrentV2(summary) => Some(&summary.narrative),
            SessionSummaryVersion::CurrentV4(summary) => Some(&summary.narrative),
            SessionSummaryVersion::MainV5(summary) => Some(&summary.narrative),
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
            SessionSummaryVersion::CurrentV4(summary) => {
                summary.evidence_spine.validate_against(context)
            }
            SessionSummaryVersion::MainV5(summary) => {
                let facts =
                    context
                        .main_v5()
                        .ok_or_else(|| {
                            SessionError::CompactionSourceInvalid {
                        message:
                            "main v5 requires exact Session, Research and graph validation facts"
                                .to_owned(),
                    }
                        })?;
                if summary.session_source != facts.source
                    || summary.research_source != facts.research_source
                {
                    return Err(SessionError::CompactionSourceInvalid {
                        message: "main v5 Session or Research source mismatch".to_owned(),
                    });
                }
                match (
                    context.research_state().latest_target_snapshot_id(),
                    &summary.evidence_spine,
                ) {
                    (Some(target), Some(spine))
                        if spine.target_snapshot_refs().first() == Some(target) =>
                    {
                        spine.validate_against(context)?
                    }
                    (None, None) => {}
                    _ => {
                        return Err(SessionError::CompactionSourceInvalid {
                            message: "main v5 EvidenceSpine does not match current Target"
                                .to_owned(),
                        });
                    }
                }
                if summary
                    .history_event_refs
                    .iter()
                    .any(|id| !facts.history_ids.contains(id))
                    || summary
                        .receipt_refs
                        .iter()
                        .any(|id| !facts.receipt_ids.contains(id))
                    || summary
                        .artifact_refs
                        .iter()
                        .any(|id| !facts.artifact_ids.contains(id))
                {
                    return Err(SessionError::CompactionSourceInvalid {
                        message: "main v5 source reference is absent".to_owned(),
                    });
                }
                self.validate_graph_reference(&facts.graph)
            }
        }
    }

    pub(crate) const fn requires_reference_validation(&self) -> bool {
        matches!(
            self.version,
            SessionSummaryVersion::CurrentV2(_)
                | SessionSummaryVersion::CurrentV4(_)
                | SessionSummaryVersion::MainV5(_)
        )
    }

    pub(crate) fn graph_reference(
        &self,
    ) -> Option<(
        &crate::domain::events::EventId,
        &SessionSourceIdentity,
        &crate::domain::artifacts::BlobId,
        &str,
    )> {
        match &self.version {
            SessionSummaryVersion::MainV5(summary) => Some((
                &summary.model_sample_event_ref,
                &summary.narrative_source,
                &summary.source_plan_blob_id,
                &summary.source_plan_hash,
            )),
            _ => None,
        }
    }

    pub(crate) fn main_v5_sources(
        &self,
    ) -> Option<(&SessionSourceIdentity, &ResearchSourceIdentity)> {
        match &self.version {
            SessionSummaryVersion::MainV5(summary) => {
                Some((&summary.session_source, &summary.research_source))
            }
            _ => None,
        }
    }

    pub(crate) fn validate_graph_reference(
        &self,
        graph: &super::VerifiedCompactionGraph,
    ) -> Result<(), SessionError> {
        let SessionSummaryVersion::MainV5(summary) = &self.version else {
            return Err(SessionError::CompactionSourceInvalid {
                message: "Summary has no main v5 graph".to_owned(),
            });
        };
        let protected = graph
            .protected_inputs
            .iter()
            .map(|input| {
                Ok(ProtectedInputReference {
                    anchor: input.anchor.clone(),
                    event_id: input.event_id.clone(),
                    content_hash: fingerprint_value(&input.item)?,
                    provenance: input.provenance.clone(),
                })
            })
            .collect::<Result<Vec<_>, SessionError>>()?;
        if summary.model_sample_event_ref != graph.root_event_id
            || summary.narrative_source != graph.source
            || summary.source_plan_blob_id != graph.plan_blob_id
            || summary.source_plan_hash != graph.plan_hash
            || summary.narrative != graph.narrative
            || summary.protected_inputs != protected
        {
            return Err(SessionError::CompactionSourceInvalid {
                message: "main v5 Summary graph or protected input mismatch".to_owned(),
            });
        }
        Ok(())
    }

    pub(crate) fn context_item(&self) -> Result<ContentItem, SessionError> {
        let payload = if self.schema_version() == 5 {
            serde_json::to_string(&self.model_projection()?)
        } else {
            serde_json::to_string(self)
        }
        .map_err(|error| SessionError::PayloadEncode {
            event_type: EventType::new(COMPACTION_CREATED_EVENT_TYPE),
            message: error.to_string(),
        })?;
        let prefix = match self.schema_version() {
            1 => "derived_session_summary_v1:",
            2 => "derived_session_summary_v2:",
            4 => "derived_session_summary_v4:",
            5 => "derived_session_summary_main_v5:",
            _ => unreachable!("validated Summary schema"),
        };
        Ok(ContentItem::user_text(format!("{prefix}{payload}")))
    }

    /// Compact model-facing view; the authoritative reference set stays in
    /// the durable Summary and provenance sidecar.
    pub(crate) fn model_projection(&self) -> Result<Value, SessionError> {
        if self.schema_version() != 5 {
            return serde_json::to_value(self).map_err(|error| SessionError::PayloadEncode {
                event_type: EventType::new(COMPACTION_CREATED_EVENT_TYPE),
                message: error.to_string(),
            });
        }
        #[derive(Serialize)]
        struct Projection<'a> {
            projection_version: u16,
            schema_version: u16,
            revision: u64,
            fingerprint: &'a str,
            source: Option<&'a SessionSourceIdentity>,
            evidence_spine: Option<&'a EvidenceSpine>,
            narrative: &'a NarrativeSummary,
        }
        let value = if let Some(narrative) = self.narrative() {
            let source = match &self.version {
                SessionSummaryVersion::MainV5(summary) => Some(&summary.session_source),
                _ => None,
            };
            serde_json::to_value(Projection {
                projection_version: 1,
                schema_version: self.schema_version(),
                revision: self.revision(),
                fingerprint: self.fingerprint(),
                source,
                evidence_spine: self.evidence_spine(),
                narrative,
            })
        } else {
            serde_json::to_value(self)
        };
        value.map_err(|error| SessionError::PayloadEncode {
            event_type: EventType::new(COMPACTION_CREATED_EVENT_TYPE),
            message: error.to_string(),
        })
    }

    pub(crate) fn context_provenance(&self) -> Result<Option<ContextProvenance>, SessionError> {
        if let SessionSummaryVersion::MainV5(summary) = &self.version {
            let mut freshness = FreshnessIdentity::new(summary.session_source.stream_hash())
                .map_err(context_provenance_error)?
                .with_producer_version("session-summary:main-v5");
            let mut research_refs = Vec::new();
            if let Some(spine) = &summary.evidence_spine {
                freshness = freshness.with_target_snapshot(spine.target_snapshot_refs()[0].clone());
                research_refs = spine.context_references()?.research_fact_refs().to_vec();
            }
            let references = crate::domain::context::ContextReferences::new(
                research_refs,
                summary.artifact_refs.clone(),
                summary.receipt_refs.clone(),
                summary.history_event_refs.clone(),
            )
            .map_err(context_provenance_error)?;
            return ContextProvenance::new(
                ContextSlot::Summary,
                ContextOrigin::SessionProjection,
                SemanticClass::DerivedProjection,
                AuthorityLevel::DerivedNonEvidence,
                RetentionClass::Stable,
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
        let actual = match &self.version {
            SessionSummaryVersion::MainV5(summary) => serde_json::to_vec(&summary.narrative),
            _ => serde_json::to_vec(self),
        }
        .map_err(|error| SessionError::PayloadEncode {
            event_type: EventType::new(COMPACTION_CREATED_EVENT_TYPE),
            message: error.to_string(),
        })?
        .len();
        let maximum = match &self.version {
            SessionSummaryVersion::CurrentV4(summary) => summary.byte_limit,
            SessionSummaryVersion::MainV5(summary) => summary.byte_limit,
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

fn validate_main_v5(summary: &SessionSummaryV5) -> Result<(), SessionError> {
    if summary.schema_version != 5
        || summary.format != "echidna-research-main-v5"
        || summary.byte_limit == 0
    {
        return Err(SessionError::CompactionSourceInvalid {
            message: "invalid main v5 format or narrative limit".to_owned(),
        });
    }
    summary.session_source.validate()?;
    summary.narrative_source.validate()?;
    if summary.narrative_source.project_id() != summary.session_source.project_id()
        || summary.narrative_source.stream_id() != summary.session_source.stream_id()
    {
        return Err(SessionError::CompactionSourceInvalid {
            message: "narrative source belongs to another Session".to_owned(),
        });
    }
    summary
        .research_source
        .validate(summary.session_source.project_id())?;
    if let Some(spine) = &summary.evidence_spine {
        spine.validate_intrinsic()?;
        if !matches!(&summary.research_source, ResearchSourceIdentity::Nonempty { source } if source == spine.source())
        {
            return Err(SessionError::CompactionSourceInvalid {
                message: "EvidenceSpine Research source mismatch".to_owned(),
            });
        }
    }
    validate_unique_ids(
        "history_event_refs",
        summary.history_event_refs.iter().map(|id| id.as_str()),
    )?;
    validate_unique_ids(
        "receipt_refs",
        summary.receipt_refs.iter().map(|id| id.as_str()),
    )?;
    validate_unique_ids(
        "artifact_refs",
        summary.artifact_refs.iter().map(|id| id.as_str()),
    )?;
    validate_unique_ids(
        "protected_inputs",
        summary
            .protected_inputs
            .iter()
            .map(|input| input.event_id.as_str()),
    )?;
    for input in &summary.protected_inputs {
        require_text("protected_inputs.anchor", input.anchor.as_str())?;
        if !super::source::is_hash(&input.content_hash) {
            return Err(SessionError::CompactionSourceInvalid {
                message: "invalid protected input hash".to_owned(),
            });
        }
    }
    require_text(
        "model_sample_event_ref",
        summary.model_sample_event_ref.as_str(),
    )?;
    crate::domain::artifacts::BlobId::new(summary.source_plan_blob_id.as_str()).map_err(
        |error| SessionError::CompactionSourceInvalid {
            message: error.to_string(),
        },
    )?;
    if !super::source::is_hash(&summary.source_plan_hash) {
        return Err(SessionError::CompactionSourceInvalid {
            message: "invalid Compaction source plan hash".to_owned(),
        });
    }
    summary.narrative.validate()?;
    let mut value = summary.clone();
    value.fingerprint.clear();
    if summary.fingerprint != fingerprint_value(&("echidna-research-main-v5", value))? {
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

fn expected_v4_fingerprint(value: &SessionSummaryV4) -> Result<String, SessionError> {
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

fn validate_byte_limit(byte_limit: usize) -> Result<(), SessionError> {
    if !(MAX_SESSION_SUMMARY_BYTES..=MAX_EXTENDED_SESSION_SUMMARY_BYTES).contains(&byte_limit) {
        return Err(SessionError::CompactionSummaryByteLimit {
            actual: byte_limit,
            minimum: MAX_SESSION_SUMMARY_BYTES,
            maximum: MAX_EXTENDED_SESSION_SUMMARY_BYTES,
        });
    }
    Ok(())
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
