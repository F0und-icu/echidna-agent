//! Read-only source composition for the deterministic retrieval Shadow.
//!
//! Candidate eligibility and ranking remain in `domain::retrieval`. This
//! module only projects existing bounded facts into those candidate contracts;
//! it neither owns filesystem access nor writes derived output.

use std::collections::BTreeMap;

use crate::domain::research::{
    FindingRecord, HypothesisRecord, ResearchRecordRef, ResearchState, TargetSnapshotId,
};
use crate::domain::retrieval::{
    CandidateTarget, CurrentTarget, RetrievalCandidate, RetrievalError, RetrievalField,
    RetrievalQuery, RetrievalShadowReport, RetrievalSource, shadow_report,
};
use crate::domain::vision::VisionObservation;

const MAX_RESEARCH_CANDIDATES_PER_KIND: usize = 48;
const MAX_MEMORY_SECTIONS: usize = 64;
const MAX_MEMORY_SECTION_BYTES: usize = 8 * 1024;

pub(crate) struct RetrievalMemoryDocument {
    pub(crate) path: String,
    pub(crate) hash: String,
    pub(crate) content: String,
}

pub(crate) struct HistoricalSummaryCandidate {
    pub(crate) session_id: String,
    pub(crate) target_snapshot_ids: Vec<String>,
    pub(crate) content: String,
}

pub(crate) struct RetrievalSources {
    pub(crate) memory_documents: Vec<RetrievalMemoryDocument>,
    pub(crate) observations: Vec<(String, VisionObservation)>,
    pub(crate) historical_summaries: Vec<HistoricalSummaryCandidate>,
}

pub(crate) fn project_shadow_report(
    project_id: &str,
    research: &ResearchState,
    current_target_id: &TargetSnapshotId,
    query_text: &str,
    historical_comparison: bool,
    sources: RetrievalSources,
) -> Result<RetrievalShadowReport, RetrievalError> {
    let current_target = research.target_snapshot(current_target_id).ok_or({
        RetrievalError::RequiredText {
            field: "current_target_snapshot",
        }
    })?;
    let query = RetrievalQuery::new(
        project_id,
        CurrentTarget::new(
            current_target.id.as_str(),
            &current_target.target_id,
            &current_target.version,
        )?,
        query_text,
        historical_comparison,
    )?;
    let mut candidates = research_candidates(project_id, research)?;
    candidates.extend(memory_candidates(project_id, sources.memory_documents)?);
    candidates.extend(observation_candidates(
        project_id,
        research,
        sources.observations,
    )?);
    candidates.extend(historical_candidates(
        project_id,
        sources.historical_summaries,
    )?);
    shadow_report(query, candidates)
}

fn research_candidates(
    project_id: &str,
    research: &ResearchState,
) -> Result<Vec<RetrievalCandidate>, RetrievalError> {
    let mut candidates = Vec::new();
    candidates.extend(
        research
            .retrieval_target_snapshot_records()
            .take(MAX_RESEARCH_CANDIDATES_PER_KIND)
            .map(|record| {
                candidate(
                    format!("research:target_snapshot:{}", record.id),
                    project_id,
                    RetrievalSource::Research,
                    CandidateTarget::snapshot(record.id.as_str()),
                    vec![
                        (RetrievalField::Title, record.target_id.clone()),
                        (
                            RetrievalField::Identifiers,
                            join([
                                record.id.as_str(),
                                &record.target_id,
                                &record.locator,
                                &record.version,
                                &record.build_id,
                            ]),
                        ),
                        (
                            RetrievalField::Target,
                            join([
                                &record.target_id,
                                &record.version,
                                &record.environment_fingerprint,
                            ]),
                        ),
                        (RetrievalField::Metadata, record.notes.clone()),
                    ],
                )
            })
            .collect::<Result<Vec<_>, _>>()?,
    );
    candidates.extend(
        research
            .hypothesis_records()
            .take(MAX_RESEARCH_CANDIDATES_PER_KIND)
            .map(|record| {
                candidate(
                    format!("research:hypothesis:{}", record.id),
                    project_id,
                    RetrievalSource::Research,
                    CandidateTarget::target_identity(&record.target_id, None),
                    vec![
                        (RetrievalField::Title, record.title.clone()),
                        (
                            RetrievalField::Identifiers,
                            join([
                                record.id.as_str(),
                                &record.target_id,
                                &record.entry_point,
                                &record.sensitive_sink,
                            ]),
                        ),
                        (RetrievalField::Target, record.target_id.clone()),
                        (
                            RetrievalField::Body,
                            join([
                                &record.attack_surface,
                                &record.vulnerability_class,
                                &record.rationale,
                            ]),
                        ),
                    ],
                )
            })
            .collect::<Result<Vec<_>, _>>()?,
    );
    candidates.extend(
        research
            .retrieval_experiment_records()
            .take(MAX_RESEARCH_CANDIDATES_PER_KIND)
            .map(|record| {
                candidate(
                    format!("research:experiment:{}", record.id),
                    project_id,
                    RetrievalSource::Research,
                    CandidateTarget::snapshot(record.target_snapshot_id.as_str()),
                    vec![
                        (RetrievalField::Title, record.goal.clone()),
                        (
                            RetrievalField::Identifiers,
                            join([
                                record.id.as_str(),
                                record.hypothesis_id.as_str(),
                                record.target_snapshot_id.as_str(),
                                &record.action_fingerprint,
                            ]),
                        ),
                        (RetrievalField::Body, join([&record.goal, &record.notes])),
                    ],
                )
            })
            .collect::<Result<Vec<_>, _>>()?,
    );
    candidates.extend(
        research
            .finding_records()
            .take(MAX_RESEARCH_CANDIDATES_PER_KIND)
            .map(|record| {
                candidate(
                    format!("research:finding:{}", record.id),
                    project_id,
                    RetrievalSource::Research,
                    finding_target(record),
                    vec![
                        (RetrievalField::Title, record.title.clone()),
                        (
                            RetrievalField::Identifiers,
                            join([
                                record.id.as_str(),
                                &record.target_id,
                                &record.target_version,
                                &record.entry_point,
                                &record.sensitive_sink,
                            ]),
                        ),
                        (
                            RetrievalField::Target,
                            join([&record.target_id, &record.target_version]),
                        ),
                        (
                            RetrievalField::Body,
                            join([
                                &record.vulnerability_class,
                                &record.impact,
                                &record.prerequisites.join(" "),
                            ]),
                        ),
                    ],
                )
            })
            .collect::<Result<Vec<_>, _>>()?,
    );
    for record in research
        .retrieval_next_action_records()
        .take(MAX_RESEARCH_CANDIDATES_PER_KIND)
    {
        let Some(target) = target_for_research_reference(research, &record.research_id) else {
            continue;
        };
        candidates.push(candidate(
            format!("research:next_action:{}", record.id),
            project_id,
            RetrievalSource::Research,
            target,
            vec![
                (RetrievalField::Title, record.action.clone()),
                (
                    RetrievalField::Identifiers,
                    join([
                        record.id.as_str(),
                        research_reference_id(&record.research_id),
                    ]),
                ),
                (RetrievalField::Body, record.action.clone()),
            ],
        )?);
    }
    candidates.extend(
        research
            .retrieval_artifact_records()
            .take(MAX_RESEARCH_CANDIDATES_PER_KIND)
            .map(|record| {
                let scope_identifier = artifact_scope_identifier(record);
                candidate(
                    format!("artifact:{}", record.id),
                    project_id,
                    RetrievalSource::ArtifactMetadata,
                    artifact_target(record),
                    vec![
                        (RetrievalField::Title, record.id.as_str().to_owned()),
                        (
                            RetrievalField::Identifiers,
                            join([
                                record.id.as_str(),
                                record.blob_id.as_str(),
                                scope_identifier.as_str(),
                            ]),
                        ),
                        (
                            RetrievalField::Metadata,
                            join([
                                &record.media_type,
                                &record.source_path,
                                &record.toolchain_fingerprint,
                                record.transform.as_deref().unwrap_or(""),
                            ]),
                        ),
                    ],
                )
            })
            .collect::<Result<Vec<_>, _>>()?,
    );
    Ok(candidates)
}

fn memory_candidates(
    project_id: &str,
    documents: Vec<RetrievalMemoryDocument>,
) -> Result<Vec<RetrievalCandidate>, RetrievalError> {
    documents
        .into_iter()
        .flat_map(|document| split_memory_sections(document).into_iter())
        .take(MAX_MEMORY_SECTIONS)
        .map(|section| {
            RetrievalCandidate::new(
                format!("memory:{}:{}", section.path, section.ordinal),
                project_id,
                RetrievalSource::ProjectMemory,
                CandidateTarget::ProjectScoped,
                vec![
                    (RetrievalField::Title, section.title),
                    (RetrievalField::Body, section.content),
                    (RetrievalField::Metadata, section.hash),
                ],
            )
        })
        .collect()
}

fn observation_candidates(
    project_id: &str,
    research: &ResearchState,
    observations: Vec<(String, VisionObservation)>,
) -> Result<Vec<RetrievalCandidate>, RetrievalError> {
    let artifact_targets = research
        .retrieval_artifact_records()
        .filter_map(|record| {
            record.scope.target_snapshot_id().map(|target_snapshot_id| {
                (
                    record.id.as_str().to_owned(),
                    target_snapshot_id.as_str().to_owned(),
                )
            })
        })
        .collect::<BTreeMap<_, _>>();
    observations
        .into_iter()
        .take(MAX_RESEARCH_CANDIDATES_PER_KIND)
        .map(|(session_id, observation)| {
            let observation_notes = observation.observation_notes().join(" ");
            let target = artifact_targets
                .get(observation.source_artifact_id.as_str())
                .cloned()
                .map(CandidateTarget::snapshot)
                .unwrap_or(CandidateTarget::Unknown);
            RetrievalCandidate::new(
                format!("observation:{session_id}:{}", observation.id.as_str()),
                project_id,
                RetrievalSource::ObservationMetadata,
                target,
                vec![
                    (RetrievalField::Title, observation.summary),
                    (
                        RetrievalField::Identifiers,
                        join([
                            observation.id.as_str(),
                            observation.source_artifact_id.as_str(),
                            &observation.task_hash,
                        ]),
                    ),
                    (
                        RetrievalField::Body,
                        join([
                            observation.extracted_text.as_deref().unwrap_or(""),
                            &observation_notes,
                            &observation.uncertainty.join(" "),
                        ]),
                    ),
                ],
            )
        })
        .collect()
}

fn historical_candidates(
    project_id: &str,
    summaries: Vec<HistoricalSummaryCandidate>,
) -> Result<Vec<RetrievalCandidate>, RetrievalError> {
    summaries
        .into_iter()
        .take(MAX_RESEARCH_CANDIDATES_PER_KIND)
        .map(|summary| {
            let target = if summary.target_snapshot_ids.is_empty() {
                CandidateTarget::Unknown
            } else {
                CandidateTarget::snapshot_set(summary.target_snapshot_ids)
            };
            RetrievalCandidate::new(
                format!("historical_summary:{}", summary.session_id),
                project_id,
                RetrievalSource::HistoricalSummary,
                target,
                vec![
                    (
                        RetrievalField::Title,
                        format!("historical session {}", summary.session_id),
                    ),
                    (RetrievalField::Body, summary.content),
                ],
            )
        })
        .collect()
}

fn candidate(
    id: String,
    project_id: &str,
    source: RetrievalSource,
    target: CandidateTarget,
    fields: Vec<(RetrievalField, String)>,
) -> Result<RetrievalCandidate, RetrievalError> {
    RetrievalCandidate::new(id, project_id, source, target, fields)
}

fn finding_target(record: &FindingRecord) -> CandidateTarget {
    if record.target_snapshot_ids.is_empty() {
        CandidateTarget::target_identity(&record.target_id, Some(record.target_version.clone()))
    } else {
        CandidateTarget::snapshot_set(
            record
                .target_snapshot_ids
                .iter()
                .map(|id| id.as_str().to_owned())
                .collect(),
        )
    }
}

fn target_for_research_reference(
    research: &ResearchState,
    reference: &ResearchRecordRef,
) -> Option<CandidateTarget> {
    match reference {
        ResearchRecordRef::Hypothesis(id) => research
            .hypothesis_records()
            .find(|record| &record.id == id)
            .map(hypothesis_target),
        ResearchRecordRef::Experiment(id) => research
            .retrieval_experiment_records()
            .find(|record| &record.id == id)
            .map(|record| CandidateTarget::snapshot(record.target_snapshot_id.as_str())),
        ResearchRecordRef::Finding(id) => research
            .finding_records()
            .find(|record| &record.id == id)
            .map(finding_target),
    }
}

fn hypothesis_target(record: &HypothesisRecord) -> CandidateTarget {
    CandidateTarget::target_identity(&record.target_id, None)
}

fn artifact_target(record: &crate::domain::artifacts::ArtifactRecord) -> CandidateTarget {
    record
        .scope
        .target_snapshot_id()
        .map(|target_snapshot_id| CandidateTarget::snapshot(target_snapshot_id.as_str()))
        .unwrap_or(CandidateTarget::ProjectScoped)
}

fn artifact_scope_identifier(record: &crate::domain::artifacts::ArtifactRecord) -> String {
    match &record.scope {
        crate::domain::artifacts::ArtifactScope::Session { session_id } => {
            format!("session:{session_id}")
        }
        crate::domain::artifacts::ArtifactScope::Target { target_snapshot_id } => {
            target_snapshot_id.as_str().to_owned()
        }
    }
}

fn research_reference_id(reference: &ResearchRecordRef) -> &str {
    match reference {
        ResearchRecordRef::Hypothesis(id) => id.as_str(),
        ResearchRecordRef::Experiment(id) => id.as_str(),
        ResearchRecordRef::Finding(id) => id.as_str(),
    }
}

struct MemorySection {
    path: String,
    hash: String,
    ordinal: usize,
    title: String,
    content: String,
}

fn split_memory_sections(document: RetrievalMemoryDocument) -> Vec<MemorySection> {
    let mut sections = Vec::new();
    let mut title = document.path.clone();
    let mut content = String::new();
    let mut ordinal = 0;
    for line in document.content.lines() {
        if let Some(heading) = line
            .strip_prefix('#')
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            if !content.trim().is_empty() {
                sections.push(MemorySection {
                    path: document.path.clone(),
                    hash: document.hash.clone(),
                    ordinal,
                    title: title.clone(),
                    content: bounded_text(&content),
                });
                ordinal += 1;
                content.clear();
            }
            title = heading.to_owned();
        } else {
            content.push_str(line);
            content.push('\n');
        }
    }
    if !content.trim().is_empty() {
        sections.push(MemorySection {
            path: document.path,
            hash: document.hash,
            ordinal,
            title,
            content: bounded_text(&content),
        });
    }
    sections
}

fn bounded_text(value: &str) -> String {
    if value.len() <= MAX_MEMORY_SECTION_BYTES {
        return value.to_owned();
    }
    let mut end = MAX_MEMORY_SECTION_BYTES;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

fn join<T: AsRef<str>>(values: impl IntoIterator<Item = T>) -> String {
    values
        .into_iter()
        .map(|value| value.as_ref().to_owned())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}
