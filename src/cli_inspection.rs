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
use crate::adapters::tools::ToolRegistry;
use crate::app::inspection::{self, AuditReferenceStatus, SnapshotDisposition, SnapshotFacts};
use crate::app::request_audit::RequestAuditService;
use crate::app::session::SessionProjection;
use crate::domain::events::StreamId;
use crate::domain::request_audit::RequestAuditBinding;
use crate::domain::session::SessionState;
use crate::ports::tool::ToolExecutor;

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
