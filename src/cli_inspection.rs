//! Filesystem composition for the read-only inspection commands.
//!
//! The serializable reports and their semantic projection live in
//! `app::inspection`; this module only discovers the local project and feeds
//! already-existing facts into that projection.

use crate::adapters::artifact_store::LocalArtifactStore;
use crate::adapters::project_event_store::{
    Project, ProjectEventStoreError, ProjectEventStream, read_existing_session_events,
};
use crate::adapters::project_snapshot::{CacheDisposition, LocalProjectSnapshot};
use crate::adapters::research_projection::load_research_state;
use crate::adapters::tools::ToolRegistry;
use crate::app::inspection::{self, AuditReferenceStatus, SnapshotDisposition, SnapshotFacts};
use crate::app::observation::ObservationService;
use crate::app::request_audit::RequestAuditService;
use crate::app::session::SessionProjection;
use crate::domain::artifacts::ArtifactId;
use crate::domain::events::StreamId;
use crate::domain::observation::ValidatedObservationSet;
use crate::domain::request_audit::RequestAuditBinding;
use crate::domain::research::ResearchState;
use crate::domain::session::SessionState;
use crate::ports::artifact::ArtifactStore;
use crate::ports::tool::ToolExecutor;
use serde_json::json;

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
    let registry = ToolRegistry::for_project(project.root());
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
