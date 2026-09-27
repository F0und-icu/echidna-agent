use std::collections::BTreeSet;

use echidna::domain::ArtifactId;
use echidna::domain::events::{
    EventEnvelope, EventId, EventSequence, EventType, OccurredAt, StreamId,
};
use echidna::domain::project::ProjectId;
use echidna::domain::research::{
    ExecutionReceiptId, ResearchEvent, ResearchReducer, ResearchState, TargetKind,
    TargetSnapshotDraft, TargetSnapshotId, TargetSnapshotRecord,
};
use echidna::domain::session::{
    CompactionValidationContext, EvidenceSpine, NarrativeSummary, ResearchStreamIdentity,
    SessionSummaryDraft,
};
use serde::Serialize;
use serde_json::Value;

pub struct CompactionFacts {
    project_id: ProjectId,
    research_stream: ResearchStreamIdentity,
    research_state: ResearchState,
    execution_receipt_ids: BTreeSet<ExecutionReceiptId>,
    target_snapshot_id: TargetSnapshotId,
}

impl CompactionFacts {
    pub fn new(label: &str) -> Self {
        let target_snapshot_id = TargetSnapshotId::new(format!("target-snapshot-{label}"));
        let events = raw_research_events(label);
        let project_id = ProjectId::new(format!("project-{label}"));
        let research_stream =
            ResearchStreamIdentity::from_events(project_id.clone(), &events).unwrap();
        let research_state = ResearchReducer.reduce(events).unwrap();
        Self {
            project_id,
            research_stream,
            research_state,
            execution_receipt_ids: BTreeSet::new(),
            target_snapshot_id,
        }
    }

    pub fn validation(&self) -> CompactionValidationContext<'_> {
        CompactionValidationContext::new(
            &self.project_id,
            &self.research_stream,
            &self.research_state,
            &self.execution_receipt_ids,
        )
    }

    pub fn summary_draft(&self, goal: &str) -> SessionSummaryDraft {
        let spine = EvidenceSpine::new(
            self.research_stream().clone(),
            vec![self.target_snapshot_id().clone()],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        let narrative = NarrativeSummary::new(
            goal,
            "the authoritative target boundary is retained by reference",
            vec!["final impact remains unresolved".to_owned()],
            vec!["run the smallest decisive control".to_owned()],
        )
        .unwrap();
        SessionSummaryDraft::new(spine, narrative).unwrap()
    }

    pub fn research_stream(&self) -> &ResearchStreamIdentity {
        &self.research_stream
    }

    pub fn target_snapshot_id(&self) -> &TargetSnapshotId {
        &self.target_snapshot_id
    }
}

pub fn raw_research_events(label: &str) -> Vec<EventEnvelope<Value>> {
    let target_snapshot_id = TargetSnapshotId::new(format!("target-snapshot-{label}"));
    let record = TargetSnapshotRecord::from_draft(TargetSnapshotDraft {
        id: target_snapshot_id,
        target_id: format!("fixture.target.{label}"),
        kind: TargetKind::Source,
        locator: format!("src/{label}"),
        version: "1.0.0".to_owned(),
        build_id: format!("build-{label}"),
        target_artifact_ids: Vec::new(),
        environment_fingerprint: "fixture-environment".to_owned(),
        device_fingerprint: "fixture-device".to_owned(),
        captured_at: "2026-08-31T00:00:00Z".to_owned(),
        notes: String::new(),
    })
    .unwrap();
    let event = ResearchEvent::TargetSnapshotRecorded(record);
    vec![EventEnvelope::current(
        EventId::new(format!("research-event-{label}")),
        StreamId::new("research"),
        EventSequence::new(1),
        OccurredAt::new("2026-08-31T00:00:00Z"),
        EventType::new(event.event_type().as_str()),
        event.encode_payload().unwrap(),
    )]
}

#[derive(Clone, Serialize)]
struct LegacySummaryV1 {
    schema_version: u16,
    revision: u64,
    fingerprint: String,
    goal: String,
    target: String,
    target_version: String,
    target_snapshot: TargetSnapshotId,
    environment: String,
    verified_facts: Vec<String>,
    active_hypotheses: Vec<String>,
    failed_paths: Vec<String>,
    artifacts: Vec<ArtifactId>,
    modified_files: Vec<String>,
    unfinished_execution: Vec<Value>,
    next_steps: Vec<String>,
}

pub fn legacy_v1_summary_value() -> Value {
    let mut summary = LegacySummaryV1 {
        schema_version: 1,
        revision: 1,
        fingerprint: String::new(),
        goal: "legacy goal".to_owned(),
        target: "legacy.apk".to_owned(),
        target_version: "1.0".to_owned(),
        target_snapshot: TargetSnapshotId::new("legacy-snapshot"),
        environment: "legacy environment".to_owned(),
        verified_facts: vec!["legacy copied claim".to_owned()],
        active_hypotheses: vec!["legacy hypothesis".to_owned()],
        failed_paths: vec!["legacy failed path".to_owned()],
        artifacts: Vec::new(),
        modified_files: Vec::new(),
        unfinished_execution: Vec::new(),
        next_steps: vec!["legacy next step".to_owned()],
    };
    summary.fingerprint = blake3::hash(&serde_json::to_vec(&summary).unwrap())
        .to_hex()
        .to_string();
    serde_json::to_value(summary).unwrap()
}
