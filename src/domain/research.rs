//! Versioned, replayable vulnerability-research facts.
//!
//! Research owns typed records, deterministic fingerprints, and strict Event
//! reduction. It has no filesystem, clock, Provider, or tool-execution I/O.

mod fingerprint;
mod records;
mod reducer;

pub use fingerprint::{action_fingerprint, dedup_key, snapshot_fingerprint};
pub use records::{
    CapsuleId, EvidenceRef, ExecutionReceiptId, ExperimentCapsuleDraft, ExperimentCapsuleRecord,
    ExperimentDraft, ExperimentId, ExperimentRecord, ExperimentResult, ExperimentUpdate,
    FindingDraft, FindingId, FindingRecord, FindingStatus, FindingUpdate, HypothesisDraft,
    HypothesisId, HypothesisRecord, HypothesisStatus, HypothesisUpdate, MergeRecordRef,
    NextActionDraft, NextActionId, NextActionRecord, NextActionStatus, NextActionUpdate,
    ResearchRecordRef, TargetKind, TargetSnapshotDraft, TargetSnapshotId, TargetSnapshotRecord,
};
pub use reducer::{
    ResearchError, ResearchEvent, ResearchEventType, ResearchReducer, ResearchState,
};
