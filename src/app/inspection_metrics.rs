//! Local Session/Event-derived metrics projection.

use serde::Serialize;

use crate::domain::context::BaselineInvalidationReason;
use crate::domain::request_audit::{RequestAuditDeliveryMode, RequestAuditDeliveryReason};
use crate::domain::session::{SessionCompaction, SessionLifecycle, SessionState};

use super::inspection_shared::{
    AuditReferenceStatus, InspectionDiagnostic, SnapshotFacts, diagnostic, estimate_tokens,
    snapshot_disposition,
};

#[derive(Debug, Serialize)]
pub struct MetricsReport {
    pub command: &'static str,
    pub status: &'static str,
    pub session_id: Option<String>,
    pub lifecycle: &'static str,
    pub requests: usize,
    pub estimated_tokens: u64,
    pub provider_usage: &'static str,
    pub full_requests: usize,
    pub incremental_requests: usize,
    pub fallbacks: usize,
    pub tool_cache_hits: u64,
    pub compactions: u64,
    pub snapshot_cache: &'static str,
    pub snapshot_reason: String,
    pub coverage: &'static str,
    pub diagnostics: Vec<InspectionDiagnostic>,
}

pub fn metrics(
    session_id: Option<&str>,
    state: Option<&SessionState>,
    snapshot: SnapshotFacts,
    audit_status: AuditReferenceStatus,
) -> MetricsReport {
    let Some(state) = state else {
        return MetricsReport {
            command: "metrics",
            status: "no_session",
            session_id: session_id.map(str::to_owned),
            lifecycle: "unstarted",
            requests: 0,
            estimated_tokens: 0,
            provider_usage: "unavailable",
            full_requests: 0,
            incremental_requests: 0,
            fallbacks: 0,
            tool_cache_hits: 0,
            compactions: 0,
            snapshot_cache: snapshot_disposition(snapshot.disposition),
            snapshot_reason: snapshot.reason,
            coverage: "no Session Event stream",
            diagnostics: vec![diagnostic(
                "NO_SESSION",
                "info",
                "no Session Event stream is available for this project",
            )],
        };
    };
    let mut full_requests = 0;
    let mut incremental_requests = 0;
    let mut fallbacks = 0;
    let mut estimated_tokens = 0_u64;
    for manifest in state.request_audits() {
        match manifest.delivery().mode() {
            RequestAuditDeliveryMode::Full => full_requests += 1,
            RequestAuditDeliveryMode::Incremental => incremental_requests += 1,
        }
        if matches!(
            manifest.delivery().reason(),
            RequestAuditDeliveryReason::Full(BaselineInvalidationReason::ProviderRejected)
        ) {
            fallbacks += 1;
        }
        estimated_tokens = estimated_tokens.saturating_add(estimate_tokens(manifest));
    }
    let compactions = state
        .latest_compaction()
        .map(SessionCompaction::summary)
        .map_or(0, |summary| summary.revision());
    let audit_is_corrupt = audit_status == AuditReferenceStatus::Corrupt;
    MetricsReport {
        command: "metrics",
        status: if audit_is_corrupt {
            "corrupt_reference"
        } else if state.request_audits().is_empty() {
            "no_requests"
        } else {
            "ok"
        },
        session_id: session_id.map(str::to_owned),
        lifecycle: lifecycle(state.lifecycle()),
        requests: state.request_audits().len(),
        estimated_tokens,
        provider_usage: "unavailable",
        full_requests,
        incremental_requests,
        fallbacks,
        tool_cache_hits: state.cache_hit_count(),
        compactions,
        snapshot_cache: snapshot_disposition(snapshot.disposition),
        snapshot_reason: snapshot.reason,
        coverage: "local RequestAudit/Event facts; Provider usage is unavailable",
        diagnostics: if audit_is_corrupt {
            vec![diagnostic(
                "CORRUPT_REQUEST_AUDIT_REFERENCE",
                "error",
                "one or more committed RequestAudit manifests reference invalid or missing Blob bytes",
            )]
        } else if state.request_audits().is_empty() {
            vec![diagnostic(
                "NO_COMMITTED_REQUEST",
                "info",
                "the Session has no committed RequestAudit manifest",
            )]
        } else {
            Vec::new()
        },
    }
}

pub fn uninitialized_metrics() -> MetricsReport {
    MetricsReport {
        command: "metrics",
        status: "uninitialized",
        session_id: None,
        lifecycle: "unstarted",
        requests: 0,
        estimated_tokens: 0,
        provider_usage: "unavailable",
        full_requests: 0,
        incremental_requests: 0,
        fallbacks: 0,
        tool_cache_hits: 0,
        compactions: 0,
        snapshot_cache: "unavailable",
        snapshot_reason: "project_not_initialized".to_owned(),
        coverage: "no initialized Cyber Agent project",
        diagnostics: vec![diagnostic(
            "PROJECT_NOT_INITIALIZED",
            "info",
            "no initialized Cyber Agent project was found",
        )],
    }
}

pub fn corrupt_session_metrics(session_id: Option<&str>, snapshot: SnapshotFacts) -> MetricsReport {
    MetricsReport {
        command: "metrics",
        status: "corrupt_session",
        session_id: session_id.map(str::to_owned),
        lifecycle: "unknown",
        requests: 0,
        estimated_tokens: 0,
        provider_usage: "unavailable",
        full_requests: 0,
        incremental_requests: 0,
        fallbacks: 0,
        tool_cache_hits: 0,
        compactions: 0,
        snapshot_cache: snapshot_disposition(snapshot.disposition),
        snapshot_reason: snapshot.reason,
        coverage: "Session Event stream could not be replayed",
        diagnostics: vec![diagnostic(
            "CORRUPT_SESSION_STREAM",
            "error",
            "the Session Event stream could not be replayed without mutation",
        )],
    }
}

fn lifecycle(lifecycle: SessionLifecycle) -> &'static str {
    match lifecycle {
        SessionLifecycle::Unstarted => "unstarted",
        SessionLifecycle::Active => "active",
        SessionLifecycle::Ended => "ended",
    }
}
