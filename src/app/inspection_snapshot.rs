//! Read-only projection of the existing Project Snapshot cache outcome.

use serde::Serialize;

use super::inspection_shared::{
    InspectionDiagnostic, SnapshotDisposition, SnapshotFacts, diagnostic,
};

#[derive(Debug, Serialize)]
pub struct SnapshotInspectionReport {
    pub command: &'static str,
    pub status: &'static str,
    pub disposition: &'static str,
    pub reason: String,
    pub project_id: Option<String>,
    pub fingerprint: Option<String>,
    pub file_count: Option<usize>,
    pub diagnostics: Vec<InspectionDiagnostic>,
}

pub fn snapshot(facts: SnapshotFacts) -> SnapshotInspectionReport {
    let uninitialized = facts.reason == "project_not_initialized";
    let (status, disposition) = if uninitialized {
        ("uninitialized", "unavailable")
    } else {
        match facts.disposition {
            SnapshotDisposition::Hit => ("ok", "hit"),
            SnapshotDisposition::IncrementalRefresh => ("ok", "incremental_refresh"),
            SnapshotDisposition::FullRebuild => ("ok", "full_rebuild"),
            SnapshotDisposition::Unavailable => ("unavailable", "unavailable"),
        }
    };
    SnapshotInspectionReport {
        command: "snapshot",
        status,
        disposition,
        reason: facts.reason,
        project_id: facts.project_id,
        fingerprint: facts.fingerprint,
        file_count: facts.file_count,
        diagnostics: if uninitialized {
            vec![diagnostic(
                "PROJECT_NOT_INITIALIZED",
                "info",
                "no initialized Cyber Agent project was found",
            )]
        } else if facts.disposition == SnapshotDisposition::Unavailable {
            vec![diagnostic(
                "SNAPSHOT_UNAVAILABLE",
                "error",
                "the existing Project Snapshot cache could not be inspected read-only",
            )]
        } else {
            Vec::new()
        },
    }
}
