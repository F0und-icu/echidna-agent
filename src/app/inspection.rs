//! Deterministic read-only inspection projections.
//!
//! Each command has its own narrow projection module. This facade exposes the
//! stable report types and functions without introducing a general inspection
//! manager or registry.

pub(crate) use super::inspection_metrics::{
    corrupt_session_metrics, metrics, uninitialized_metrics,
};
pub(crate) use super::inspection_prompt::{corrupt_session_prompt, prompt, uninitialized_prompt};
pub(crate) use super::inspection_shared::{
    AuditReferenceStatus, SnapshotDisposition, SnapshotFacts,
};
pub(crate) use super::inspection_snapshot::snapshot;
