//! Shared immutable inputs and output helpers for inspection projections.

use serde::Serialize;

use crate::domain::context::estimate_audit_component_tokens;
use crate::domain::request_audit::RequestAuditManifest;

/// Result of validating the exact Blob references behind one audit manifest.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuditReferenceStatus {
    Verified,
    Corrupt,
}

/// Read-only snapshot facts supplied by the local Snapshot adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotFacts {
    pub disposition: SnapshotDisposition,
    pub reason: String,
    pub project_id: Option<String>,
    pub fingerprint: Option<String>,
    pub file_count: Option<usize>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SnapshotDisposition {
    Hit,
    IncrementalRefresh,
    FullRebuild,
    Unavailable,
}

#[derive(Debug, Serialize)]
pub struct InspectionDiagnostic {
    pub code: &'static str,
    pub severity: &'static str,
    pub message: &'static str,
}

pub fn diagnostic(
    code: &'static str,
    severity: &'static str,
    message: &'static str,
) -> InspectionDiagnostic {
    InspectionDiagnostic {
        code,
        severity,
        message,
    }
}

pub fn estimate_tokens(manifest: &RequestAuditManifest) -> u64 {
    manifest
        .components()
        .iter()
        .map(|component| estimate_bytes(component.size_bytes()))
        .sum()
}

pub fn estimate_bytes(bytes: u64) -> u64 {
    estimate_audit_component_tokens(usize::try_from(bytes).unwrap_or(usize::MAX)) as u64
}

pub fn snapshot_disposition(disposition: SnapshotDisposition) -> &'static str {
    match disposition {
        SnapshotDisposition::Hit => "hit",
        SnapshotDisposition::IncrementalRefresh => "incremental_refresh",
        SnapshotDisposition::FullRebuild => "full_rebuild",
        SnapshotDisposition::Unavailable => "unavailable",
    }
}
