//! One read-only composition of Research facts with Project Session Receipt facts.

use crate::adapters::file_revision::FileRevision;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;

use crate::adapters::project_event_store::{
    Project, read_existing_research_events, read_existing_session_events,
};
use crate::domain::artifacts::{ArtifactId, ArtifactScope};
use crate::domain::context::ContextWindowPolicy;
use crate::domain::events::{EventEnvelope, StreamId};
use crate::domain::model::{ContentItem, MessageRole};
use crate::domain::project_stream::ProjectStreamReducer;
use crate::domain::research::{
    EvidenceRef, ExecutionReceiptId, ExperimentId, FindingId, FindingStatus, HypothesisId,
    HypothesisStatus, NextActionId, NextActionStatus, ResearchRecordRef, ResearchReducer,
    ResearchReferenceCatalog, ResearchReferenceTargetApplicability, ResearchState,
};
use crate::domain::session::{
    CompactionValidationContext, EvidenceSpine, NarrativeSummary, ResearchStreamIdentity,
    SessionError, SessionReducer, SessionState, SessionSummaryDraft,
};

/// Strict, discardable projection of shared project Artifact facts.
///
/// It intentionally exposes no Research records or current target. Consumers
/// that only need generic Artifact provenance can validate the one physical
/// Project stream without manufacturing Research context.
pub(crate) struct ProjectArtifactCatalog {
    session_artifacts: BTreeMap<StreamId, BTreeSet<ArtifactId>>,
}

impl ProjectArtifactCatalog {
    pub(crate) fn from_events_with_references(
        events: impl IntoIterator<Item = EventEnvelope<serde_json::Value>>,
        references: &ResearchReferenceCatalog,
    ) -> Result<Self, String> {
        let state = ResearchReducer
            .reduce_with_references(events, references)
            .map_err(|error| error.to_string())?;
        let mut session_artifacts = BTreeMap::new();
        for record in state.retrieval_artifact_records() {
            if let ArtifactScope::Session { session_id } = &record.scope {
                session_artifacts
                    .entry(session_id.clone())
                    .or_insert_with(BTreeSet::new)
                    .insert(record.id.clone());
            }
        }
        Ok(Self { session_artifacts })
    }

    pub(crate) fn session_artifact_ids(
        &self,
        session_id: &StreamId,
    ) -> impl Iterator<Item = &ArtifactId> {
        self.session_artifacts
            .get(session_id)
            .into_iter()
            .flat_map(|ids| ids.iter())
    }
}

/// Ephemeral receipt-only cache for one interactive process. It is not a
/// fact store: unchanged filesystem revisions reuse previously validated data.
/// Changed files are read and validated from the authoritative streams.
#[derive(Default)]
pub(crate) struct SessionReceiptCatalogCache {
    sessions: BTreeMap<crate::domain::events::StreamId, CachedSessionReceipts>,
    research: Option<CachedResearchProjection>,
    research_references_changed: bool,
}

#[derive(Clone)]
struct CachedResearchProjection {
    canonical: crate::domain::canonical_json::CanonicalArrayHasher,
    revision: FileRevision,
    events: std::sync::Arc<Vec<EventEnvelope<serde_json::Value>>>,
    cursor: crate::adapters::project_event_store::SessionStreamCursor,
    memory: Result<std::sync::Arc<crate::domain::memory::MemoryState>, String>,
    byte_digest: String,
    state: std::sync::Arc<ResearchState>,
}

#[derive(Clone)]
struct CachedSessionReceipts {
    revision: FileRevision,
    receipts: Vec<(ExecutionReceiptId, crate::domain::tools::ExecutionReceipt)>,
}

impl SessionReceiptCatalogCache {
    /// Called after synchronizing and committing the active Session projection.
    /// Its append-only receipts can advance without replaying that Session again.
    pub(crate) fn remember_validated_session(
        &mut self,
        id: &StreamId,
        state: &SessionState,
        revision: FileRevision,
    ) {
        let cached = self
            .sessions
            .entry(id.clone())
            .or_insert_with(|| CachedSessionReceipts {
                revision: revision.clone(),
                receipts: Vec::new(),
            });
        cached.receipts.extend(
            state.execution_receipts()[cached.receipts.len()..]
                .iter()
                .map(|record| {
                    (
                        ExecutionReceiptId::new(record.event_id.as_str()),
                        record.receipt.clone(),
                    )
                }),
        );
        cached.revision = revision;
    }

    /// Reuses only unchanged individual Session receipt projections. A normal
    /// History checkpoint changes the active Session file, but never causes
    /// every other project Session to be replayed.
    /// The caller holds the synchronized current Session under its Project
    /// writer lease. Peers still require exact byte identity and strict replay.
    pub(crate) fn refresh_with_session(
        &mut self,
        project: &Project,
        current: Option<(&StreamId, &SessionState)>,
    ) -> Result<(ResearchReferenceCatalog, String), String> {
        let session_ids = project
            .existing_session_ids()
            .map_err(|error| error.to_string())?;
        let live = session_ids.iter().cloned().collect::<BTreeSet<_>>();
        self.sessions.retain(|id, _| {
            let keep = live.contains(id);
            self.research_references_changed |= !keep;
            keep
        });

        for session_id in &session_ids {
            if current.is_some_and(|(id, _)| id == session_id) {
                continue;
            }
            let path = project
                .session_stream_path_existing(session_id)
                .map_err(|error| error.to_string())?;
            let before = source_revision(&path)?;
            let reuse = self
                .sessions
                .get(session_id)
                .is_some_and(|cached| cached.revision == before);
            if reuse {
                continue;
            }
            let receipts = load_session_receipts(project, session_id)?;
            if let Some(previous) = self.sessions.get(session_id) {
                self.research_references_changed |= !receipts.starts_with(&previous.receipts);
            }
            let after = source_revision(&path)?;
            if before != after {
                return Err(format!(
                    "Session `{session_id}` changed while refreshing its receipt projection"
                ));
            }
            self.sessions.insert(
                session_id.clone(),
                CachedSessionReceipts {
                    revision: after,
                    receipts,
                },
            );
        }

        let mut catalog = ResearchReferenceCatalog::default();
        let mut identity = Vec::new();
        for session_id in &session_ids {
            let receipts = match current {
                Some((id, state)) if id == session_id => state
                    .execution_receipts()
                    .iter()
                    .map(|record| {
                        (
                            ExecutionReceiptId::new(record.event_id.as_str()),
                            &record.receipt,
                        )
                    })
                    .collect::<Vec<_>>(),
                _ => self.sessions[session_id]
                    .receipts
                    .iter()
                    .map(|(id, receipt)| (id.clone(), receipt))
                    .collect(),
            };
            for (receipt_id, receipt) in receipts {
                catalog
                    .insert_execution_receipt(receipt_id.clone(), receipt.clone())
                    .map_err(|error| error.to_string())?;
                identity.push((session_id.as_str(), receipt_id, receipt));
            }
        }
        let bytes = serde_json::to_vec(&identity).map_err(|error| error.to_string())?;
        Ok((catalog, blake3::hash(&bytes).to_hex().to_string()))
    }
}

fn source_revision(path: &std::path::Path) -> Result<FileRevision, String> {
    fs::metadata(path)
        .and_then(|metadata| FileRevision::of(&metadata))
        .map_err(|error| format!("inspect source revision: {error}"))
}

/// One short-lived strictly scanned project view shared by Context and Summary
/// validation in the same synchronized transaction.
pub(crate) struct ProjectReadContext {
    pub(crate) project_hash: String,
    pub(crate) cursor: crate::adapters::project_event_store::SessionStreamCursor,
    pub(crate) events: std::sync::Arc<Vec<EventEnvelope<serde_json::Value>>>,
    pub(crate) references: ResearchReferenceCatalog,
    pub(crate) research_digest: String,
    pub(crate) receipt_digest: String,
    research: std::sync::Arc<ResearchState>,
    memory: Result<std::sync::Arc<crate::domain::memory::MemoryState>, String>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ProjectProjectionError {
    #[error("{0}")]
    Stream(String),
    #[error("{0}")]
    State(String),
}

impl ProjectReadContext {
    pub(crate) fn load(
        project: &Project,
        cache: &mut SessionReceiptCatalogCache,
        current: Option<(&StreamId, &SessionState)>,
    ) -> Result<Self, String> {
        let (references, receipt_digest) = cache.refresh_with_session(project, current)?;
        Self::load_with_references(project, cache, references, receipt_digest)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn load_with_references(
        project: &Project,
        cache: &mut SessionReceiptCatalogCache,
        references: ResearchReferenceCatalog,
        receipt_digest: String,
    ) -> Result<Self, ProjectProjectionError> {
        let path = project
            .research_stream_path_existing()
            .map_err(|error| ProjectProjectionError::Stream(error.to_string()))?;
        let before = source_revision(&path).map_err(ProjectProjectionError::Stream)?;
        if let Some(value) = &mut cache.research
            && value.revision == before
            && !cache.research_references_changed
        {
            return Ok(Self {
                project_hash: value.canonical.finish(),
                cursor: value.cursor.clone(),
                events: value.events.clone(),
                references,
                research_digest: value.byte_digest.clone(),
                receipt_digest,
                research: value.state.clone(),
                memory: value.memory.clone(),
            });
        }
        // Keep the validated snapshot until both scanning and reduction succeed.
        let previous = cache.research.clone();
        let (cursor, suffix) = crate::adapters::project_event_store::read_research_projection(
            project,
            previous.as_ref().map(|value| &value.cursor),
        )
        .map_err(|error| match error {
            crate::adapters::project_event_store::ProjectEventStoreError::ValidatedPrefixChanged { .. } =>
                ProjectProjectionError::State(error.to_string()),
            _ => ProjectProjectionError::Stream(error.to_string()),
        })?;
        let digest = cursor.content_digest();
        let (mut events, mut state, mut memory, mut canonical, unchanged_receipts) = match previous
        {
            Some(value) => (
                value.events,
                value.state,
                value.memory,
                value.canonical,
                !cache.research_references_changed,
            ),
            None => (
                std::sync::Arc::new(Vec::new()),
                std::sync::Arc::new(ResearchState::default()),
                Ok(std::sync::Arc::new(
                    crate::domain::memory::MemoryState::default(),
                )),
                crate::domain::canonical_json::CanonicalArrayHasher::new(),
                true,
            ),
        };
        for event in &suffix {
            canonical
                .push(event)
                .map_err(|error| ProjectProjectionError::State(error.to_string()))?;
            let duplicate = state.has_event_id(&event.event_id);
            ProjectStreamReducer
                .apply(
                    std::sync::Arc::make_mut(&mut state),
                    event.clone(),
                    &references,
                )
                .map_err(|error| ProjectProjectionError::State(error.to_string()))?;
            if !duplicate
                && matches!(
                    event.event_type.as_str(),
                    "memory.changed" | "memory.external_change_detected"
                )
            {
                let crate::domain::research::ResearchEvent::Memory(value) =
                    crate::domain::research::ResearchEvent::decode(
                        event.event_type.as_str(),
                        event.payload.clone(),
                    )
                    .map_err(|error| ProjectProjectionError::State(error.to_string()))?
                else {
                    unreachable!()
                };
                // A bad Memory chain is reported by its Memory consumer, not
                // promoted to a failure of unrelated Project/Research reads.
                if let Ok(state) = &mut memory
                    && let Err(error) = crate::domain::memory::MemoryReducer
                        .apply(std::sync::Arc::make_mut(state), value)
                {
                    memory = Err(error.to_string());
                }
            }
        }
        std::sync::Arc::make_mut(&mut events).extend(suffix);
        if !unchanged_receipts {
            state = std::sync::Arc::new(
                ProjectStreamReducer
                    .reduce(events.iter().cloned(), &references)
                    .map_err(|error| ProjectProjectionError::State(error.to_string()))?,
            );
        }
        let project_hash = canonical.finish();
        cache.research = Some(CachedResearchProjection {
            canonical,
            revision: cursor.file_revision().clone(),
            events: events.clone(),
            cursor: cursor.clone(),
            byte_digest: digest.clone(),
            state: state.clone(),
            memory: memory.clone(),
        });
        cache.research_references_changed = false;
        Ok(Self {
            project_hash,
            cursor,
            events,
            references,
            research_digest: digest,
            receipt_digest,
            research: state,
            memory,
        })
    }

    pub(crate) fn memory_state(&self) -> Result<&crate::domain::memory::MemoryState, &str> {
        self.memory.as_deref().map_err(String::as_str)
    }

    pub(crate) fn research_state(&self) -> &ResearchState {
        &self.research
    }

    pub(crate) fn event_prefix(&self, watermark: u64) -> &[EventEnvelope<serde_json::Value>] {
        &self.events[..self
            .events
            .partition_point(|event| event.sequence.get() <= watermark)]
    }

    pub(crate) fn compaction_facts(
        &self,
        project_id: &crate::domain::project::ProjectId,
        watermark: Option<u64>,
    ) -> Result<CompactionValidationFacts, CompactionValidationFactsError> {
        let events = watermark.map_or(self.events.as_slice(), |value| self.event_prefix(value));
        compaction_facts_from_events(project_id, events, self.references.clone())
    }

    pub(crate) fn historical_compaction_facts(
        &self,
        project_id: &crate::domain::project::ProjectId,
        source: &ResearchStreamIdentity,
    ) -> Result<CompactionValidationFacts, String> {
        let facts = self
            .compaction_facts(project_id, Some(source.watermark().get()))
            .map_err(|error| error.to_string())?;
        if &facts.research_stream != source {
            return Err(
                "Compaction historical Research prefix identity/hash does not match".to_owned(),
            );
        }
        Ok(facts)
    }
}

fn load_session_receipts(
    project: &Project,
    session_id: &crate::domain::events::StreamId,
) -> Result<Vec<(ExecutionReceiptId, crate::domain::tools::ExecutionReceipt)>, String> {
    let events = read_existing_session_events(project, session_id)
        .map_err(|error| format!("Session `{session_id}` cannot be read: {error}"))?;
    let session = SessionReducer
        .reduce(events)
        .map_err(|error| format!("Session `{session_id}` cannot be reduced: {error}"))?;
    Ok(session
        .execution_receipts()
        .iter()
        .map(|record| {
            (
                ExecutionReceiptId::new(record.event_id.as_str()),
                record.receipt.clone(),
            )
        })
        .collect())
}

/// Ephemeral read-only facts borrowed by Summary v2 validation. Every field is
/// rebuilt from the authoritative Project Research and Session streams.
pub(crate) struct CompactionValidationFacts {
    project_id: crate::domain::project::ProjectId,
    research_stream: ResearchStreamIdentity,
    research_state: ResearchState,
    research_references: ResearchReferenceCatalog,
    execution_receipt_ids: BTreeSet<ExecutionReceiptId>,
}

impl CompactionValidationFacts {
    pub(crate) fn context(&self) -> CompactionValidationContext<'_> {
        CompactionValidationContext::new(
            &self.project_id,
            &self.research_stream,
            &self.research_state,
            &self.execution_receipt_ids,
        )
    }

    /// Builds the one deterministic production Summary candidate from the
    /// current strict Research projection and resumable Session History. The
    /// Evidence Spine contains references only; the bounded narrative keeps
    /// the initial/latest request and continuation and remains derived.
    pub(crate) fn compose_session_summary_draft(
        &self,
        history: &[ContentItem],
        previous: Option<&crate::domain::session::SessionSummary>,
        reference_limit: usize,
        policy: ContextWindowPolicy,
    ) -> Result<SessionSummaryDraft, SessionError> {
        let (current_goal, progress) = continuation_text(history, previous, policy)?;

        let current_target_id = self
            .research_state
            .latest_target_snapshot_id()
            .cloned()
            .ok_or(SessionError::CompactionCurrentTargetRequired)?;
        let current_target = self
            .research_state
            .target_snapshot(&current_target_id)
            .ok_or_else(|| SessionError::CompactionEvidenceReferenceMissing {
                kind: "target_snapshot",
                id: current_target_id.as_str().to_owned(),
            })?;

        let hypothesis_refs = self
            .research_state
            .hypothesis_records()
            .filter(|record| {
                record.merged_into.is_none()
                    && record.status != HypothesisStatus::Rejected
                    && matches!(
                        self.research_state.research_reference_target_applicability(
                            &ResearchRecordRef::Hypothesis(record.id.clone()),
                            &current_target_id,
                        ),
                        ResearchReferenceTargetApplicability::Applicable
                            | ResearchReferenceTargetApplicability::Unknown
                    )
            })
            .map(|record| record.id.clone())
            .collect::<BTreeSet<HypothesisId>>();
        let mut experiment_refs = self
            .research_state
            .retrieval_experiment_records()
            .filter(|record| {
                self.research_state.research_reference_target_applicability(
                    &ResearchRecordRef::Experiment(record.id.clone()),
                    &current_target_id,
                ) == ResearchReferenceTargetApplicability::Applicable
            })
            .map(|record| record.id.clone())
            .collect::<BTreeSet<ExperimentId>>();
        let finding_refs = self
            .research_state
            .finding_records()
            .filter(|record| {
                record.status != FindingStatus::Discarded
                    && record.merged_into.is_none()
                    && matches!(
                        self.research_state.research_reference_target_applicability(
                            &ResearchRecordRef::Finding(record.id.clone()),
                            &current_target_id,
                        ),
                        ResearchReferenceTargetApplicability::Applicable
                            | ResearchReferenceTargetApplicability::Unknown
                    )
            })
            .map(|record| record.id.clone())
            .collect::<BTreeSet<FindingId>>();

        let mut artifact_refs = BTreeSet::new();
        for id in &current_target.target_artifact_ids {
            match self.research_state.evidence_reference_target_applicability(
                &self.research_references,
                &EvidenceRef::Artifact(id.clone()),
                &current_target_id,
            ) {
                ResearchReferenceTargetApplicability::Applicable => {
                    artifact_refs.insert(id.clone());
                }
                ResearchReferenceTargetApplicability::Incompatible => {}
                ResearchReferenceTargetApplicability::MissingReference => {
                    return Err(SessionError::CompactionEvidenceReferenceMissing {
                        kind: "artifact",
                        id: id.as_str().to_owned(),
                    });
                }
                ResearchReferenceTargetApplicability::Unknown => unreachable!(
                    "Artifact applicability to an exact TargetSnapshot is never unknown"
                ),
            }
        }
        let mut execution_receipt_refs = BTreeSet::new();
        for record in self.research_state.hypothesis_records() {
            if hypothesis_refs.contains(&record.id) {
                self.collect_current_target_evidence_references(
                    &record.evidence_links,
                    &current_target_id,
                    &mut experiment_refs,
                    &mut artifact_refs,
                    &mut execution_receipt_refs,
                );
            }
        }
        for record in self.research_state.finding_records() {
            if finding_refs.contains(&record.id) {
                self.collect_current_target_evidence_references(
                    &record.evidence_links,
                    &current_target_id,
                    &mut experiment_refs,
                    &mut artifact_refs,
                    &mut execution_receipt_refs,
                );
            }
        }
        for record in self.research_state.retrieval_experiment_records() {
            if !experiment_refs.contains(&record.id) {
                continue;
            }
            for id in &record.artifact_refs {
                self.retain_current_target_evidence_reference(
                    &EvidenceRef::Artifact(id.clone()),
                    &current_target_id,
                    &mut experiment_refs,
                    &mut artifact_refs,
                    &mut execution_receipt_refs,
                );
            }
            if let Some(capsule) = self.research_state.capsule_for_experiment(record) {
                for id in capsule
                    .input_artifact_ids
                    .iter()
                    .chain(&capsule.output_artifact_ids)
                {
                    self.retain_current_target_evidence_reference(
                        &EvidenceRef::Artifact(id.clone()),
                        &current_target_id,
                        &mut experiment_refs,
                        &mut artifact_refs,
                        &mut execution_receipt_refs,
                    );
                }
                for id in &capsule.execution_receipt_ids {
                    self.retain_current_target_evidence_reference(
                        &EvidenceRef::ExecutionReceipt(id.clone()),
                        &current_target_id,
                        &mut experiment_refs,
                        &mut artifact_refs,
                        &mut execution_receipt_refs,
                    );
                }
            }
        }

        let next_action_refs = self
            .research_state
            .retrieval_next_action_records()
            .filter(|record| {
                record.status != NextActionStatus::Done
                    && research_reference_is_retained(
                        &record.research_id,
                        &hypothesis_refs,
                        &experiment_refs,
                        &finding_refs,
                    )
            })
            .map(|record| record.id.clone())
            .collect::<BTreeSet<NextActionId>>();

        // This is a bounded navigation spine, not a complete copied ledger.
        // Omitted references remain discoverable in the authoritative stream.
        let unresolved = hypothesis_refs
            .iter()
            .take(reference_limit.min(4))
            .map(|id| {
                format!(
                    "Re-read hypothesis {} for its current unresolved state.",
                    id.as_str()
                )
            })
            .collect();
        let mut next_steps = next_action_refs
            .iter()
            .take(reference_limit.min(4))
            .map(|id| {
                format!(
                    "Re-read next action {}; do not infer completion from this summary.",
                    id.as_str()
                )
            })
            .collect::<Vec<_>>();
        next_steps.push("This lossy summary retains a bounded reference subset; consult current Research facts before continuing.".to_owned());
        next_steps.push("Original Tool outputs and task constraints may be omitted. The latest assistant continuation is not the original baseline; consult the original observation and explicit Project instructions, or state that the historical value is unavailable. Do not rerun a completed mutation to recreate history.".to_owned());
        let evidence_spine = EvidenceSpine::new(
            self.research_stream.clone(),
            vec![current_target_id],
            hypothesis_refs.into_iter().take(reference_limit).collect(),
            experiment_refs.into_iter().take(reference_limit).collect(),
            finding_refs.into_iter().take(reference_limit).collect(),
            next_action_refs.into_iter().take(reference_limit).collect(),
            artifact_refs.into_iter().take(reference_limit).collect(),
            execution_receipt_refs
                .into_iter()
                .take(reference_limit)
                .collect(),
        )?;
        let narrative = NarrativeSummary::new(current_goal, progress, unresolved, next_steps)?;
        SessionSummaryDraft::new(evidence_spine, narrative)
    }

    fn collect_current_target_evidence_references(
        &self,
        links: &[crate::domain::research::EvidenceLink],
        current_target: &crate::domain::research::TargetSnapshotId,
        experiments: &mut BTreeSet<ExperimentId>,
        artifacts: &mut BTreeSet<ArtifactId>,
        receipts: &mut BTreeSet<ExecutionReceiptId>,
    ) {
        for link in links {
            self.retain_current_target_evidence_reference(
                &link.evidence,
                current_target,
                experiments,
                artifacts,
                receipts,
            );
        }
    }

    fn retain_current_target_evidence_reference(
        &self,
        reference: &EvidenceRef,
        current_target: &crate::domain::research::TargetSnapshotId,
        experiments: &mut BTreeSet<ExperimentId>,
        artifacts: &mut BTreeSet<ArtifactId>,
        receipts: &mut BTreeSet<ExecutionReceiptId>,
    ) {
        if self.research_state.evidence_reference_target_applicability(
            &self.research_references,
            reference,
            current_target,
        ) != ResearchReferenceTargetApplicability::Applicable
        {
            return;
        }
        match reference {
            EvidenceRef::Artifact(id) => {
                artifacts.insert(id.clone());
            }
            EvidenceRef::Experiment(id) => {
                experiments.insert(id.clone());
            }
            EvidenceRef::ExecutionReceipt(id) => {
                receipts.insert(id.clone());
            }
        }
    }
}

fn research_reference_is_retained(
    reference: &ResearchRecordRef,
    hypotheses: &BTreeSet<HypothesisId>,
    experiments: &BTreeSet<ExperimentId>,
    findings: &BTreeSet<FindingId>,
) -> bool {
    match reference {
        ResearchRecordRef::Hypothesis(id) => hypotheses.contains(id),
        ResearchRecordRef::Experiment(id) => experiments.contains(id),
        ResearchRecordRef::Finding(id) => findings.contains(id),
    }
}

pub(crate) fn continuation_text(
    history: &[ContentItem],
    previous: Option<&crate::domain::session::SessionSummary>,
    policy: ContextWindowPolicy,
) -> Result<(String, String), SessionError> {
    crate::domain::history::ConversationHistory::from_complete_content_items(history)?;
    let latest_goal = latest_text(history, MessageRole::User)
        .or_else(|| {
            previous
                .and_then(|summary| summary.narrative())
                .map(|narrative| narrative.current_goal())
        })
        .ok_or(SessionError::CompactionResumableHistoryRequired)?;
    let initial_goal = history
        .iter()
        .find_map(|item| match item {
            ContentItem::Text {
                role: MessageRole::User,
                text,
            } if !text.trim().is_empty() => Some(text.as_str()),
            _ => None,
        })
        .unwrap_or(latest_goal);
    let current_goal = if initial_goal == latest_goal {
        bounded_text(latest_goal, policy.continuation_field_bytes())
    } else {
        format!(
            "Initial request: {}\nLatest request: {}",
            bounded_text(initial_goal, policy.continuation_goal_part_bytes()),
            bounded_text(latest_goal, policy.continuation_goal_part_bytes())
        )
    };
    let progress = latest_text(history, MessageRole::Assistant).map_or_else(
        || "No assistant continuation text was durably recorded for the latest goal.".to_owned(),
        |text| format!("Non-authoritative continuation of the latest assistant reply: {text}"),
    );
    Ok((
        current_goal,
        bounded_text(&progress, policy.continuation_field_bytes()),
    ))
}

fn latest_text(history: &[ContentItem], expected_role: MessageRole) -> Option<&str> {
    history.iter().rev().find_map(|item| match item {
        ContentItem::Text { role, text } if *role == expected_role && !text.trim().is_empty() => {
            Some(text.as_str())
        }
        _ => None,
    })
}

fn bounded_text(value: &str, maximum_bytes: usize) -> String {
    if value.len() <= maximum_bytes {
        return value.to_owned();
    }
    let suffix = "…";
    let limit = maximum_bytes.saturating_sub(suffix.len());
    let boundary = value
        .char_indices()
        .map(|(index, _)| index)
        .take_while(|index| *index <= limit)
        .last()
        .unwrap_or(0);
    format!("{}{suffix}", &value[..boundary])
}

pub(crate) fn load_reference_catalog(
    project: &Project,
) -> Result<ResearchReferenceCatalog, String> {
    load_reference_catalog_from_sessions(project, None)
}

/// Reuses the current Session already strictly replayed under the caller's
/// Project writer lease. Other Sessions retain the same full validation.
pub(crate) fn load_reference_catalog_with_session(
    project: &Project,
    session_id: &StreamId,
    state: &SessionState,
) -> Result<ResearchReferenceCatalog, String> {
    load_reference_catalog_from_sessions(project, Some((session_id, state)))
}

fn load_reference_catalog_from_sessions(
    project: &Project,
    current: Option<(&StreamId, &SessionState)>,
) -> Result<ResearchReferenceCatalog, String> {
    let mut references = ResearchReferenceCatalog::default();
    for session_id in project
        .existing_session_ids()
        .map_err(|error| error.to_string())?
    {
        let replayed;
        let session = match current {
            Some((id, state)) if id == &session_id => state,
            _ => {
                let events = read_existing_session_events(project, &session_id)
                    .map_err(|error| format!("Session `{session_id}` cannot be read: {error}"))?;
                replayed = SessionReducer.reduce(events).map_err(|error| {
                    format!("Session `{session_id}` cannot be reduced: {error}")
                })?;
                &replayed
            }
        };
        for record in session.execution_receipts() {
            let receipt_id = ExecutionReceiptId::new(record.event_id.as_str());
            references
                .insert_execution_receipt(receipt_id, record.receipt.clone())
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(references)
}

pub(crate) fn load_research_state(project: &Project) -> Result<ResearchState, String> {
    load_research_projection(project).map(|(state, _)| state)
}

/// Reduces Research against a caller-refreshed ephemeral receipt catalog.
/// The caller owns the cache lifetime; this helper owns no additional state.
pub(crate) fn load_research_state_with_references(
    project: &Project,
    references: &ResearchReferenceCatalog,
) -> Result<ResearchState, String> {
    let events = read_existing_research_events(project).map_err(|error| error.to_string())?;
    ResearchReducer
        .reduce_with_references(events, references)
        .map_err(|error| error.to_string())
}

/// Rebuilds the one authoritative Research projection and the ephemeral
/// Session Receipt catalog it requires. Callers receive no writer and the
/// catalog is never serialized as a second fact source.
pub(crate) fn load_research_projection(
    project: &Project,
) -> Result<(ResearchState, ResearchReferenceCatalog), String> {
    let references = load_reference_catalog(project)?;
    let state = load_research_state_with_references(project, &references)?;
    Ok((state, references))
}

/// Only an empty Research stream is a known non-mutating prerequisite. Other
/// source/projection errors remain fatal at the automatic compaction boundary.
#[derive(Debug)]
pub(crate) enum CompactionValidationFactsError {
    ResearchStreamEmpty,
    Projection(String),
}

impl fmt::Display for CompactionValidationFactsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ResearchStreamEmpty => SessionError::CompactionResearchStreamEmpty.fmt(formatter),
            Self::Projection(message) => message.fmt(formatter),
        }
    }
}

impl From<String> for CompactionValidationFactsError {
    fn from(message: String) -> Self {
        Self::Projection(message)
    }
}

pub(crate) fn load_compaction_validation_facts(
    project: &Project,
) -> Result<CompactionValidationFacts, CompactionValidationFactsError> {
    let references = load_reference_catalog(project)?;
    load_compaction_validation_facts_with_references(project, references)
}

/// One compaction operation shares its already validated receipt catalogue
/// between generic Artifact and Research facts; no cache survives this call.
pub(crate) fn load_compaction_validation_facts_with_references(
    project: &Project,
    references: ResearchReferenceCatalog,
) -> Result<CompactionValidationFacts, CompactionValidationFactsError> {
    let events = read_existing_research_events(project).map_err(|error| error.to_string())?;
    compaction_facts_from_events(project.project_id(), &events, references)
}

fn compaction_facts_from_events(
    project_id: &crate::domain::project::ProjectId,
    events: &[EventEnvelope<serde_json::Value>],
    references: ResearchReferenceCatalog,
) -> Result<CompactionValidationFacts, CompactionValidationFactsError> {
    let execution_receipt_ids = references
        .execution_receipts()
        .map(|(id, _)| id.clone())
        .collect();
    let research_stream = ResearchStreamIdentity::from_events(project_id.clone(), events).map_err(
        |error| match error {
            SessionError::CompactionResearchStreamEmpty => {
                CompactionValidationFactsError::ResearchStreamEmpty
            }
            error => CompactionValidationFactsError::Projection(error.to_string()),
        },
    )?;
    let research_state = ResearchReducer
        .reduce_with_references(events.iter().cloned(), &references)
        .map_err(|error| error.to_string())?;
    Ok(CompactionValidationFacts {
        project_id: project_id.clone(),
        research_stream,
        research_state,
        research_references: references,
        execution_receipt_ids,
    })
}
