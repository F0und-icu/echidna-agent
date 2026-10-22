//! Filesystem composition for the read-only inspection commands.
//!
//! The serializable reports and their semantic projection live in
//! `app::inspection`; this module only discovers the local project and feeds
//! already-existing facts into that projection.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use crate::adapters::artifact_store::LocalArtifactStore;
use crate::adapters::project_event_store::{
    Project, ProjectEventStoreError, ProjectEventStream, read_existing_session_events,
};
use crate::adapters::project_memory::LocalProjectMemory;
use crate::adapters::project_snapshot::{CacheDisposition, LocalProjectSnapshot};
use crate::adapters::research_projection::{load_research_projection, load_research_state};
use crate::adapters::tools::ToolRegistry;
use crate::app::guidance::project_shadow_report as project_guidance_shadow_report;
use crate::app::inspection::{self, AuditReferenceStatus, SnapshotDisposition, SnapshotFacts};
use crate::app::observation::ObservationService;
use crate::app::request_audit::RequestAuditService;
use crate::app::retrieval::{
    HistoricalSummaryCandidate, RetrievalMemoryDocument, RetrievalSources, project_shadow_report,
};
use crate::app::session::SessionProjection;
use crate::domain::artifacts::ArtifactId;
use crate::domain::events::StreamId;
use crate::domain::guidance::{
    CandidateValidity, GuidanceActionIdentityWitness, GuidanceEvalCandidate,
    GuidanceEvalCandidates, PrerequisiteState,
};
use crate::domain::observation::ValidatedObservationSet;
use crate::domain::request_audit::RequestAuditBinding;
use crate::domain::research::{
    EvidenceGapKind, ExperimentId, FindingId, HypothesisId, ResearchRecordRef, ResearchState,
    TargetSnapshotId,
};
use crate::domain::session::SessionState;
use crate::ports::artifact::ArtifactStore;
use crate::ports::tool::ToolExecutor;
use serde::Deserialize;
use serde_json::{Value, json};

const MAX_RETRIEVAL_OBSERVATION_SESSIONS: usize = 48;
const MAX_RETRIEVAL_OBSERVATIONS: usize = 48;
const MAX_GUIDANCE_CANDIDATE_FILE_BYTES: usize = 64 * 1024;

use super::{CliError, print_json};

struct LocalSessionInspection {
    session_id: Option<StreamId>,
    session_state: Option<SessionState>,
    session_corrupt: bool,
}

pub(crate) fn print_prompt_inspection(requested_session: Option<&str>) -> Result<(), CliError> {
    let Some(project) = discover_project()? else {
        return print_json(&inspection::uninitialized_prompt());
    };
    let local = read_session_inspection(&project, requested_session)?;
    if local.session_corrupt {
        let session_id = local
            .session_id
            .as_ref()
            .map(StreamId::as_str)
            .expect("corrupt Session has an identified stream");
        return print_json(&inspection::corrupt_session_prompt(session_id));
    }
    let session_id = local.session_id.as_ref().map(StreamId::as_str);
    let manifest = local
        .session_state
        .as_ref()
        .and_then(|state| state.request_audits().last());
    let audit_status = manifest.map_or(AuditReferenceStatus::Verified, |manifest| {
        validate_audit_reference(&project, local.session_id.as_ref(), manifest)
    });
    print_json(&inspection::prompt(session_id, manifest, audit_status))
}

pub(crate) fn print_snapshot_inspection() -> Result<(), CliError> {
    let Some(project) = discover_project()? else {
        return print_json(&inspection::snapshot(uninitialized_snapshot()));
    };
    print_json(&inspection::snapshot(read_snapshot(&project)))
}

pub(crate) fn print_metrics(requested_session: Option<&str>) -> Result<(), CliError> {
    let Some(project) = discover_project()? else {
        return print_json(&inspection::uninitialized_metrics());
    };
    let snapshot = read_snapshot(&project);
    let local = read_session_inspection(&project, requested_session)?;
    if local.session_corrupt {
        return print_json(&inspection::corrupt_session_metrics(
            local.session_id.as_ref().map(StreamId::as_str),
            snapshot,
        ));
    }
    let session_id = local.session_id.as_ref().map(StreamId::as_str);
    let audit_status = local
        .session_state
        .as_ref()
        .map(|state| {
            if state.request_audits().iter().all(|manifest| {
                validate_audit_reference(&project, local.session_id.as_ref(), manifest)
                    == AuditReferenceStatus::Verified
            }) {
                AuditReferenceStatus::Verified
            } else {
                AuditReferenceStatus::Corrupt
            }
        })
        .unwrap_or(AuditReferenceStatus::Verified);
    print_json(&inspection::metrics(
        session_id,
        local.session_state.as_ref(),
        snapshot,
        audit_status,
    ))
}

pub(crate) fn print_android_manifest_observations(
    requested_session: &str,
    artifact_id: &str,
) -> Result<(), CliError> {
    let project = require_project()?;
    let observations =
        read_android_manifest_observations(&project, requested_session, artifact_id)?;
    print_json(&json!({
        "command": "inspect android-manifest",
        "status": "ok",
        "observations": observations,
    }))
}

pub(crate) fn print_android_manifest_target_delta(
    baseline_session: &str,
    baseline_artifact: &str,
    candidate_session: &str,
    candidate_artifact: &str,
) -> Result<(), CliError> {
    let project = require_project()?;
    let baseline =
        read_android_manifest_observations(&project, baseline_session, baseline_artifact)?;
    let candidate =
        read_android_manifest_observations(&project, candidate_session, candidate_artifact)?;
    let delta =
        ObservationService::target_delta(&baseline, &candidate).map_err(observation_error)?;
    print_json(&json!({
        "command": "inspect target-delta",
        "status": "ok",
        "delta": delta,
    }))
}

pub(crate) fn print_retrieval_shadow(
    query: &str,
    current_target_snapshot: &str,
    historical_sessions: &[String],
    historical_comparison: bool,
) -> Result<(), CliError> {
    let project = require_retrieval_project()?;
    let research = load_research_state(&project).map_err(retrieval_error)?;
    let memory_documents = LocalProjectMemory::new(&project)
        .read_retrieval_documents()
        .map_err(retrieval_error)?
        .into_iter()
        .filter_map(
            |document| match (document.document.content, document.document.hash) {
                (Some(content), Some(hash)) => Some(RetrievalMemoryDocument {
                    path: document.path.as_str().to_owned(),
                    hash,
                    content,
                }),
                (None, None) => None,
                _ => None,
            },
        )
        .collect();
    let observations = read_current_observations(&project)?;
    let historical_summaries = read_historical_summaries(&project, historical_sessions)?;
    let report = project_shadow_report(
        project.project_id().as_str(),
        &research,
        &TargetSnapshotId::new(current_target_snapshot),
        query,
        historical_comparison,
        RetrievalSources {
            memory_documents,
            observations,
            historical_summaries,
        },
    )
    .map_err(retrieval_error)?;
    print_json(&json!({
        "command": "inspect retrieval",
        "status": "ok",
        "shadow": true,
        "model_context_changed": false,
        "effects": {
            "provider_requests": 0,
            "tool_actions": 0,
            "research_writes": 0,
            "memory_writes": 0,
            "event_writes": 0,
            "cache_writes": 0
        },
        "report": report,
    }))
}

pub(crate) fn print_guidance_shadow(
    current_target_snapshot: &str,
    eval_candidates_path: Option<&Path>,
) -> Result<(), CliError> {
    let project = require_guidance_project()?;
    let (research, references) = load_research_projection(&project).map_err(guidance_error)?;
    let candidates = read_guidance_candidates(eval_candidates_path)?;
    let report = project_guidance_shadow_report(
        &research,
        &references,
        &TargetSnapshotId::new(current_target_snapshot),
        candidates,
    )
    .map_err(guidance_error)?;
    print_json(&json!({
        "command": "inspect guidance",
        "status": "ok",
        "shadow": true,
        "model_context_changed": false,
        "effects": {
            "provider_requests": 0,
            "tool_actions": 0,
            "experiment_creations": 0,
            "research_writes": 0,
            "memory_writes": 0,
            "phase_changes": 0,
            "finding_promotions": 0,
            "observation_promotions": 0,
            "retrieval_state_changes": 0,
            "event_writes": 0,
            "cache_writes": 0
        },
        "report": report,
    }))
}

fn uninitialized_snapshot() -> SnapshotFacts {
    SnapshotFacts {
        disposition: SnapshotDisposition::Unavailable,
        reason: "project_not_initialized".to_owned(),
        project_id: None,
        fingerprint: None,
        file_count: None,
    }
}

fn discover_project() -> Result<Option<Project>, CliError> {
    let root = std::env::current_dir().map_err(CliError::WorkingDirectory)?;
    match Project::discover(&root) {
        Ok(project) => Ok(Some(project)),
        Err(ProjectEventStoreError::ProjectNotFound { .. }) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn require_project() -> Result<Project, CliError> {
    discover_project()?.ok_or_else(|| CliError::Observation {
        message: "the current directory is not an initialized Cyber Agent project".to_owned(),
    })
}

fn require_retrieval_project() -> Result<Project, CliError> {
    discover_project()?.ok_or_else(|| CliError::Retrieval {
        message: "the current directory is not an initialized Cyber Agent project".to_owned(),
    })
}

fn require_guidance_project() -> Result<Project, CliError> {
    discover_project()?.ok_or_else(|| CliError::Guidance {
        message: "the current directory is not an initialized Cyber Agent project".to_owned(),
    })
}

fn read_guidance_candidates(path: Option<&Path>) -> Result<GuidanceEvalCandidates, CliError> {
    let Some(path) = path else {
        return Ok(GuidanceEvalCandidates::empty());
    };
    let file = File::open(path).map_err(|error| CliError::Guidance {
        message: format!(
            "cannot open controlled candidate file `{}`: {error}",
            path.display()
        ),
    })?;
    let mut bytes = Vec::new();
    file.take((MAX_GUIDANCE_CANDIDATE_FILE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| CliError::Guidance {
            message: format!(
                "cannot read controlled candidate file `{}`: {error}",
                path.display()
            ),
        })?;
    if bytes.len() > MAX_GUIDANCE_CANDIDATE_FILE_BYTES {
        return Err(CliError::Guidance {
            message: format!(
                "controlled candidate file `{}` exceeds the {}-byte limit",
                path.display(),
                MAX_GUIDANCE_CANDIDATE_FILE_BYTES
            ),
        });
    }
    let wire = serde_json::from_slice::<GuidanceEvalCandidatesWire>(&bytes).map_err(|error| {
        CliError::Guidance {
            message: format!(
                "controlled candidate file `{}` is invalid: {error}",
                path.display()
            ),
        }
    })?;
    Ok(wire.into())
}

/// Strict CLI-only wire input for controlled guidance cases. It deliberately
/// does not deserialize through the durable Research schema: this caller has
/// its own bounded input contract and explicitly converts to typed Domain
/// candidates before the projection runs.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GuidanceEvalCandidatesWire {
    candidates: Vec<GuidanceEvalCandidateWire>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GuidanceEvalCandidateWire {
    id: String,
    research_id: GuidanceResearchRecordRefWire,
    target_snapshot_id: String,
    action_identity: GuidanceActionIdentityWitnessWire,
    validity: CandidateValidityWire,
    prerequisite: PrerequisiteStateWire,
    boundary_importance: u8,
    competing_hypothesis_discrimination: u8,
    expected_gap_closures: Vec<EvidenceGapKind>,
    reproducibility: u8,
    execution_cost: u8,
    mutation_risk: u8,
    version_drift_risk: u8,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GuidanceActionIdentityWitnessWire {
    hypothesis_id: String,
    normalized_action: String,
    #[serde(default)]
    normalized_arguments: Value,
    #[serde(default)]
    tool_version: String,
    #[serde(default)]
    input_artifact_ids: Vec<String>,
    #[serde(default)]
    environment_delta: String,
    #[serde(default)]
    account_prerequisites: Vec<String>,
    #[serde(default)]
    toolchain_fingerprint: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GuidanceResearchRecordRefWire {
    kind: GuidanceResearchRecordKindWire,
    id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum GuidanceResearchRecordKindWire {
    Hypothesis,
    Experiment,
    Finding,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum CandidateValidityWire {
    Valid,
    Invalid,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum PrerequisiteStateWire {
    Satisfied,
    Missing,
    Unknown,
}

impl From<GuidanceEvalCandidatesWire> for GuidanceEvalCandidates {
    fn from(wire: GuidanceEvalCandidatesWire) -> Self {
        Self {
            candidates: wire
                .candidates
                .into_iter()
                .map(GuidanceEvalCandidate::from)
                .collect(),
        }
    }
}

impl From<GuidanceEvalCandidateWire> for GuidanceEvalCandidate {
    fn from(wire: GuidanceEvalCandidateWire) -> Self {
        Self {
            id: wire.id,
            research_id: wire.research_id.into(),
            target_snapshot_id: TargetSnapshotId::new(wire.target_snapshot_id),
            action_identity: wire.action_identity.into(),
            validity: wire.validity.into(),
            prerequisite: wire.prerequisite.into(),
            boundary_importance: wire.boundary_importance,
            competing_hypothesis_discrimination: wire.competing_hypothesis_discrimination,
            expected_gap_closures: wire.expected_gap_closures,
            reproducibility: wire.reproducibility,
            execution_cost: wire.execution_cost,
            mutation_risk: wire.mutation_risk,
            version_drift_risk: wire.version_drift_risk,
        }
    }
}

impl From<GuidanceActionIdentityWitnessWire> for GuidanceActionIdentityWitness {
    fn from(wire: GuidanceActionIdentityWitnessWire) -> Self {
        Self {
            hypothesis_id: HypothesisId::new(wire.hypothesis_id),
            normalized_action: wire.normalized_action,
            normalized_arguments: wire.normalized_arguments,
            tool_version: wire.tool_version,
            input_artifact_ids: wire
                .input_artifact_ids
                .into_iter()
                .map(ArtifactId::new)
                .collect(),
            environment_delta: wire.environment_delta,
            account_prerequisites: wire.account_prerequisites,
            toolchain_fingerprint: wire.toolchain_fingerprint,
        }
    }
}

impl From<GuidanceResearchRecordRefWire> for ResearchRecordRef {
    fn from(wire: GuidanceResearchRecordRefWire) -> Self {
        match wire.kind {
            GuidanceResearchRecordKindWire::Hypothesis => {
                ResearchRecordRef::Hypothesis(HypothesisId::new(wire.id))
            }
            GuidanceResearchRecordKindWire::Experiment => {
                ResearchRecordRef::Experiment(ExperimentId::new(wire.id))
            }
            GuidanceResearchRecordKindWire::Finding => {
                ResearchRecordRef::Finding(FindingId::new(wire.id))
            }
        }
    }
}

impl From<CandidateValidityWire> for CandidateValidity {
    fn from(value: CandidateValidityWire) -> Self {
        match value {
            CandidateValidityWire::Valid => Self::Valid,
            CandidateValidityWire::Invalid => Self::Invalid,
        }
    }
}

impl From<PrerequisiteStateWire> for PrerequisiteState {
    fn from(value: PrerequisiteStateWire) -> Self {
        match value {
            PrerequisiteStateWire::Satisfied => Self::Satisfied,
            PrerequisiteStateWire::Missing => Self::Missing,
            PrerequisiteStateWire::Unknown => Self::Unknown,
        }
    }
}

fn read_current_observations(
    project: &Project,
) -> Result<Vec<(String, crate::domain::vision::VisionObservation)>, CliError> {
    let mut observations = Vec::new();
    let mut session_ids = project.existing_session_ids().map_err(retrieval_error)?;
    session_ids.sort_by(|left, right| left.as_str().cmp(right.as_str()));
    for session_id in session_ids
        .into_iter()
        .take(MAX_RETRIEVAL_OBSERVATION_SESSIONS)
    {
        let events = read_existing_session_events(project, &session_id).map_err(retrieval_error)?;
        let projection = SessionProjection::replay(events).map_err(retrieval_error)?;
        let state = projection.state();
        for observation in state.vision_observations() {
            if observations.len() == MAX_RETRIEVAL_OBSERVATIONS {
                return Ok(observations);
            }
            observations.push((session_id.as_str().to_owned(), observation.clone()));
        }
    }
    Ok(observations)
}

fn read_historical_summaries(
    project: &Project,
    requested_sessions: &[String],
) -> Result<Vec<HistoricalSummaryCandidate>, CliError> {
    let mut selected = requested_sessions.to_vec();
    selected.sort();
    selected.dedup();
    selected
        .into_iter()
        .map(|session_id| {
            let local = read_session_inspection(project, Some(&session_id))?;
            if local.session_corrupt {
                return Err(CliError::Retrieval {
                    message: format!(
                        "Session `{session_id}` cannot be reduced for historical retrieval"
                    ),
                });
            }
            let Some(summary) = local
                .session_state
                .as_ref()
                .and_then(SessionState::latest_compaction)
                .map(|compaction| compaction.summary())
            else {
                return Ok(None);
            };
            let target_snapshot_ids = summary
                .evidence_spine()
                .map(|spine| {
                    spine
                        .target_snapshot_refs()
                        .iter()
                        .map(|id| id.as_str().to_owned())
                        .collect()
                })
                .unwrap_or_default();
            let content = match summary.narrative() {
                Some(narrative) => [
                    narrative.current_goal(),
                    narrative.concise_progress(),
                    &narrative.unresolved_questions().join("\n"),
                    &narrative.next_step_notes().join("\n"),
                ]
                .join("\n"),
                None => serde_json::to_string(summary).map_err(CliError::Json)?,
            };
            Ok(Some(HistoricalSummaryCandidate {
                session_id,
                target_snapshot_ids,
                content,
            }))
        })
        .collect::<Result<Vec<_>, CliError>>()
        .map(|summaries| summaries.into_iter().flatten().collect())
}

fn retrieval_error(error: impl std::fmt::Display) -> CliError {
    CliError::Retrieval {
        message: error.to_string(),
    }
}

fn guidance_error(error: impl std::fmt::Display) -> CliError {
    CliError::Guidance {
        message: error.to_string(),
    }
}

fn read_android_manifest_observations(
    project: &Project,
    requested_session: &str,
    artifact_id: &str,
) -> Result<ValidatedObservationSet, CliError> {
    let local = read_session_inspection(project, Some(requested_session))?;
    if local.session_corrupt {
        return Err(CliError::Observation {
            message: format!("Session `{requested_session}` cannot be reduced"),
        });
    }
    let session = local
        .session_state
        .as_ref()
        .ok_or_else(|| CliError::Observation {
            message: format!("Session `{requested_session}` has no durable Session state"),
        })?;
    let research = read_research_state(project)?;
    let artifact_id = ArtifactId::new(artifact_id);
    let artifact = research
        .artifact(&artifact_id)
        .ok_or_else(|| CliError::Observation {
            message: format!("Artifact `{artifact_id}` is not present in Research"),
        })?;
    let bytes = LocalArtifactStore::for_project(project)
        .read_blob(&artifact.blob_id)
        .map_err(|error| CliError::Observation {
            message: format!("Artifact `{artifact_id}` Blob could not be read: {error}"),
        })?;
    ObservationService::ingest_android_manifest(&research, session, artifact, &bytes)
        .map_err(observation_error)
}

fn read_research_state(project: &Project) -> Result<ResearchState, CliError> {
    load_research_state(project).map_err(|error| CliError::Observation {
        message: format!("Research stream cannot be reduced: {error}"),
    })
}

fn observation_error(error: impl std::fmt::Display) -> CliError {
    CliError::Observation {
        message: error.to_string(),
    }
}

fn read_snapshot(project: &Project) -> SnapshotFacts {
    let registry = ToolRegistry::for_project(project.root()).with_project(project);
    let snapshot_specs = registry.specs();
    match LocalProjectSnapshot::new(project).inspect_read_only(&snapshot_specs) {
        Ok(result) => {
            let (disposition, reason) = match result.disposition {
                CacheDisposition::Hit => (SnapshotDisposition::Hit, "cache_valid".to_owned()),
                CacheDisposition::IncrementalRefresh { reason } => {
                    (SnapshotDisposition::IncrementalRefresh, reason)
                }
                CacheDisposition::FullRebuild { reason } => {
                    (SnapshotDisposition::FullRebuild, reason)
                }
            };
            SnapshotFacts {
                disposition,
                reason,
                project_id: Some(project.project_id().as_str().to_owned()),
                fingerprint: Some(result.source_fingerprint.digest.clone()),
                file_count: result.snapshot.as_ref().map(|snapshot| snapshot.file_count),
            }
        }
        Err(_) => SnapshotFacts {
            disposition: SnapshotDisposition::Unavailable,
            reason: "cache_inspection_failed".to_owned(),
            project_id: Some(project.project_id().as_str().to_owned()),
            fingerprint: None,
            file_count: None,
        },
    }
}

fn read_session_inspection(
    project: &Project,
    requested_session: Option<&str>,
) -> Result<LocalSessionInspection, CliError> {
    let session_ids = project.existing_session_ids()?;
    let session_id = match requested_session {
        Some(requested) => session_ids
            .into_iter()
            .find(|session_id| session_id.as_str() == requested)
            .ok_or_else(|| CliError::SessionNotFound {
                session_id: requested.to_owned(),
            })
            .map(Some)?,
        None if session_ids.len() > 1 => {
            return Err(CliError::SessionSelectionRequired {
                session_count: session_ids.len(),
            });
        }
        None => session_ids.into_iter().next(),
    };
    let Some(session_id) = session_id else {
        return Ok(LocalSessionInspection {
            session_id: None,
            session_state: None,
            session_corrupt: false,
        });
    };
    let (state, session_corrupt) = match read_existing_session_events(project, &session_id) {
        Ok(events) => match SessionProjection::replay(events) {
            Ok(projection) => (Some(projection.state().clone()), false),
            Err(_) => (None, true),
        },
        Err(_) => (None, true),
    };
    Ok(LocalSessionInspection {
        session_id: Some(session_id),
        session_state: state,
        session_corrupt,
    })
}

fn validate_audit_reference(
    project: &Project,
    session_id: Option<&StreamId>,
    manifest: &crate::domain::request_audit::RequestAuditManifest,
) -> AuditReferenceStatus {
    let Some(session_id) = session_id else {
        return AuditReferenceStatus::Corrupt;
    };
    let expected_stream_id = ProjectEventStream::Session(session_id.clone()).stream_id();
    let blobs = LocalArtifactStore::for_project(project);
    let Ok(binding) = RequestAuditBinding::new(project.project_id().clone(), expected_stream_id)
    else {
        return AuditReferenceStatus::Corrupt;
    };
    if RequestAuditService::new(&blobs, binding)
        .reconstruct(manifest)
        .is_ok()
    {
        AuditReferenceStatus::Verified
    } else {
        AuditReferenceStatus::Corrupt
    }
}
