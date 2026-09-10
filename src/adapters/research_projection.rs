//! One read-only composition of Research facts with Project Session Receipt facts.

use std::collections::BTreeSet;

use crate::adapters::project_event_store::{
    Project, read_existing_research_events, read_existing_session_events,
};
use crate::domain::artifacts::ArtifactId;
use crate::domain::model::{ContentItem, MessageRole};
use crate::domain::research::{
    EvidenceRef, ExecutionReceiptId, ExperimentId, FindingId, FindingStatus, HypothesisId,
    HypothesisStatus, NextActionId, NextActionStatus, ResearchRecordRef, ResearchReducer,
    ResearchReferenceCatalog, ResearchReferenceTargetApplicability, ResearchState,
};
use crate::domain::session::{
    CompactionValidationContext, EvidenceSpine, NarrativeSummary, ResearchStreamIdentity,
    SessionError, SessionReducer, SessionSummaryDraft,
};

const MAX_COMPACTION_NARRATIVE_FIELD_BYTES: usize = 1536;

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
    /// Evidence Spine contains references only; the bounded narrative copies
    /// only the latest user/assistant continuation text and remains derived.
    pub(crate) fn compose_session_summary_draft(
        &self,
        history: &[ContentItem],
    ) -> Result<SessionSummaryDraft, SessionError> {
        crate::domain::history::ConversationHistory::from_complete_content_items(history)?;
        let current_goal = latest_text(history, MessageRole::User)
            .ok_or(SessionError::CompactionResumableHistoryRequired)?;
        let progress = latest_text(history, MessageRole::Assistant).map_or_else(
            || {
                "No assistant continuation text was durably recorded for the latest goal."
                    .to_owned()
            },
            |text| format!("Non-authoritative continuation of the latest assistant reply: {text}"),
        );

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

        let evidence_spine = EvidenceSpine::new(
            self.research_stream.clone(),
            vec![current_target_id],
            hypothesis_refs.into_iter().collect(),
            experiment_refs.into_iter().collect(),
            finding_refs.into_iter().collect(),
            next_action_refs.into_iter().collect(),
            artifact_refs.into_iter().collect(),
            execution_receipt_refs.into_iter().collect(),
        )?;
        let narrative = NarrativeSummary::new(
            bounded_text(current_goal, MAX_COMPACTION_NARRATIVE_FIELD_BYTES),
            bounded_text(&progress, MAX_COMPACTION_NARRATIVE_FIELD_BYTES),
            Vec::new(),
            Vec::new(),
        )?;
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
    load_reference_catalog_and_receipt_ids(project).map(|(catalog, _)| catalog)
}

fn load_reference_catalog_and_receipt_ids(
    project: &Project,
) -> Result<(ResearchReferenceCatalog, BTreeSet<ExecutionReceiptId>), String> {
    let mut references = ResearchReferenceCatalog::default();
    let mut receipt_ids = BTreeSet::new();
    for session_id in project
        .existing_session_ids()
        .map_err(|error| error.to_string())?
    {
        let events = read_existing_session_events(project, &session_id)
            .map_err(|error| format!("Session `{session_id}` cannot be read: {error}"))?;
        let session = SessionReducer
            .reduce(events)
            .map_err(|error| format!("Session `{session_id}` cannot be reduced: {error}"))?;
        for record in session.execution_receipts() {
            let receipt_id = ExecutionReceiptId::new(record.event_id.as_str());
            references
                .insert_execution_receipt(receipt_id.clone(), record.receipt.clone())
                .map_err(|error| error.to_string())?;
            receipt_ids.insert(receipt_id);
        }
    }
    Ok((references, receipt_ids))
}

pub(crate) fn load_research_state(project: &Project) -> Result<ResearchState, String> {
    load_research_projection(project).map(|(state, _)| state)
}

/// Rebuilds the one authoritative Research projection and the ephemeral
/// Session Receipt catalog it requires. Callers receive no writer and the
/// catalog is never serialized as a second fact source.
pub(crate) fn load_research_projection(
    project: &Project,
) -> Result<(ResearchState, ResearchReferenceCatalog), String> {
    let references = load_reference_catalog(project)?;
    let events = read_existing_research_events(project).map_err(|error| error.to_string())?;
    let state = ResearchReducer
        .reduce_with_references(events, &references)
        .map_err(|error| error.to_string())?;
    Ok((state, references))
}

pub(crate) fn load_compaction_validation_facts(
    project: &Project,
) -> Result<CompactionValidationFacts, String> {
    let (references, execution_receipt_ids) = load_reference_catalog_and_receipt_ids(project)?;
    let events = read_existing_research_events(project).map_err(|error| error.to_string())?;
    let research_stream =
        ResearchStreamIdentity::from_events(project.project_id().clone(), &events)
            .map_err(|error| error.to_string())?;
    let research_state = ResearchReducer
        .reduce_with_references(events, &references)
        .map_err(|error| error.to_string())?;
    Ok(CompactionValidationFacts {
        project_id: project.project_id().clone(),
        research_stream,
        research_state,
        research_references: references,
        execution_receipt_ids,
    })
}
