//! Versioned, replayable vulnerability-research facts.
//!
//! Research owns typed records, deterministic fingerprints, and strict Event
//! reduction. It has no filesystem, clock, Provider, or tool-execution I/O.

mod evidence_gap;
mod fingerprint;
mod records;
mod reducer;
mod upcast;

pub use fingerprint::{action_fingerprint, dedup_key, snapshot_fingerprint};
pub use records::{
    CapsuleId, EvidenceLink, EvidenceRef, EvidenceRelation, ExecutionReceiptId,
    ExperimentCapsuleDraft, ExperimentCapsuleRecord, ExperimentDraft, ExperimentId,
    ExperimentRecord, ExperimentResult, ExperimentUpdate, FindingDraft, FindingId, FindingRecord,
    FindingStatus, FindingUpdate, HypothesisDraft, HypothesisId, HypothesisRecord,
    HypothesisStatus, HypothesisUpdate, MergeRecordRef, NextActionDraft, NextActionId,
    NextActionRecord, NextActionStatus, NextActionUpdate, ResearchRecordRef, TargetKind,
    TargetSnapshotDraft, TargetSnapshotId, TargetSnapshotRecord,
};
pub(crate) use reducer::ResearchReferenceCatalog;
pub use reducer::{
    ResearchError, ResearchEvent, ResearchEventType, ResearchReducer, ResearchState,
};
