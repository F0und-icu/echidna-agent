#[path = "support/compaction.rs"]
mod compaction_support;

use std::collections::BTreeSet;

use compaction_support::{CompactionFacts, legacy_v1_summary_value, raw_research_events};
use cyber_agent::app::session::{AppendAfterDurableError, SessionProjection};
use cyber_agent::domain::ArtifactId;
use cyber_agent::domain::artifacts::{ArtifactEvent, ArtifactRecord, BlobId};
use cyber_agent::domain::context::{ProviderSessionId, ProviderSessionRef, ProviderSessionRole};
use cyber_agent::domain::events::{
    EventEnvelope, EventId, EventSequence, EventType, OccurredAt, StreamId,
};
use cyber_agent::domain::model::ContentItem;
use cyber_agent::domain::project::ProjectId;
use cyber_agent::domain::research::{
    ExecutionReceiptId, HypothesisId, ResearchEvent, ResearchReducer, TargetSnapshotId,
};
use cyber_agent::domain::session::{
    CompactionValidationContext, EvidenceSpine, NarrativeSummary, ResearchStreamIdentity,
    SessionError, SessionEvent, SessionReducer, SessionState, SessionSummaryDraft,
};
use cyber_agent::domain::tools::{
    ExecutionId, ExecutionReceipt, ToolCall, ToolCallId, ToolName, ToolResult, ToolResultStatus,
};
use cyber_agent::ports::EventSink;
use serde_json::{Value, json};

type CompactionEventAlteration = Box<dyn Fn(EventEnvelope<Value>) -> EventEnvelope<Value>>;

fn stream_id() -> StreamId {
    StreamId::new("session:compaction-fixture")
}

fn primary_session() -> ProviderSessionRef {
    ProviderSessionRef {
        id: ProviderSessionId::new("primary-compaction"),
        role: ProviderSessionRole::Primary,
        generation: 1,
    }
}

fn envelope(event: SessionEvent, id: &str, sequence: u64) -> EventEnvelope<Value> {
    event
        .into_envelope(
            EventId::new(id),
            stream_id(),
            EventSequence::new(sequence),
            OccurredAt::new("2026-08-31T00:00:00Z"),
        )
        .unwrap()
}

fn call(id: &str) -> ToolCall {
    ToolCall::new(
        ToolCallId::new(id),
        ToolName::new("read_file"),
        json!({"path": "Cargo.toml"}),
    )
}

fn result(id: &str) -> ToolResult {
    ToolResult {
        call_id: ToolCallId::new(id),
        tool_name: ToolName::new("read_file"),
        status: ToolResultStatus::Succeeded,
        output: json!({"contents": "[package]"}),
    }
}

fn source_events() -> Vec<EventEnvelope<Value>> {
    vec![
        envelope(
            SessionEvent::Started {
                primary_provider_session: primary_session(),
            },
            "started",
            1,
        ),
        envelope(
            SessionEvent::HistoryItemRecorded {
                item: ContentItem::user_text("inspect the target"),
            },
            "history-user",
            2,
        ),
        envelope(
            SessionEvent::HistoryItemRecorded {
                item: ContentItem::ToolCall {
                    call: call("older-call"),
                },
            },
            "history-old-call",
            3,
        ),
        envelope(
            SessionEvent::HistoryItemRecorded {
                item: ContentItem::ToolResult {
                    result: result("older-call"),
                },
            },
            "history-old-result",
            4,
        ),
        envelope(
            SessionEvent::HistoryItemRecorded {
                item: ContentItem::assistant_text("the older pair is complete"),
            },
            "history-assistant",
            5,
        ),
        envelope(
            SessionEvent::HistoryItemRecorded {
                item: ContentItem::ToolCall {
                    call: call("recent-call"),
                },
            },
            "history-recent-call",
            6,
        ),
        envelope(
            SessionEvent::HistoryItemRecorded {
                item: ContentItem::ToolResult {
                    result: result("recent-call"),
                },
            },
            "history-recent-result",
            7,
        ),
        envelope(
            SessionEvent::HistoryItemRecorded {
                item: ContentItem::user_text("preserve the recent pair"),
            },
            "history-latest-user",
            8,
        ),
    ]
}

fn source_state() -> SessionState {
    SessionReducer.reduce(source_events()).unwrap()
}

fn authoritative_research_events_with_artifact(label: &str) -> Vec<EventEnvelope<Value>> {
    let mut events = raw_research_events(label);
    let artifact_event_id = EventId::new(format!("artifact-event-{label}"));
    let artifact = ArtifactRecord {
        id: ArtifactId::new(format!("artifact-{label}")),
        blob_id: BlobId::new("a".repeat(64)).unwrap(),
        media_type: "application/octet-stream".to_owned(),
        size_bytes: 4,
        source_path: format!("fixtures/{label}.bin"),
        created_by_event_id: artifact_event_id.clone(),
        produced_by_tool_call_id: ToolCallId::new(format!("artifact-call-{label}")),
        target_snapshot_id: TargetSnapshotId::new(format!("target-snapshot-{label}")),
        toolchain_fingerprint: "fixture-toolchain-v1".to_owned(),
        parent_artifact_ids: Vec::new(),
        transform: None,
    };
    let event = ResearchEvent::Artifact(ArtifactEvent::Created(artifact));
    events.push(EventEnvelope::current(
        artifact_event_id,
        StreamId::new("research"),
        EventSequence::new(2),
        OccurredAt::new("2026-08-31T00:00:00Z"),
        EventType::new(event.event_type().as_str()),
        event.encode_payload().unwrap(),
    ));
    events
}

fn authoritative_session_events_with_receipt(label: &str) -> Vec<EventEnvelope<Value>> {
    let receipt = ExecutionReceipt {
        execution_id: ExecutionId::new(format!("execution-{label}")),
        plan_fingerprint: format!("plan-{label}"),
        resolved_tool_plan_fingerprint: None,
        status: ToolResultStatus::Succeeded,
        exit_reason: None,
        output_artifact_ids: vec![ArtifactId::new(format!("artifact-{label}"))],
        output_captures: Vec::new(),
        environment_change: None,
        rerun_reason: None,
        cache_source: None,
        cache_validation: None,
        file_mutations: Vec::new(),
    };
    vec![
        envelope(
            SessionEvent::Started {
                primary_provider_session: primary_session(),
            },
            &format!("started-{label}"),
            1,
        ),
        envelope(
            SessionEvent::ExecutionReceiptRecorded { receipt },
            &format!("receipt-{label}"),
            2,
        ),
        envelope(
            SessionEvent::HistoryItemRecorded {
                item: ContentItem::user_text("continue from authoritative facts"),
            },
            &format!("history-{label}"),
            3,
        ),
    ]
}

fn compaction_event(
    state: &SessionState,
    facts: &CompactionFacts,
    id: &str,
    sequence: u64,
) -> EventEnvelope<Value> {
    envelope(
        SessionEvent::CompactionCreated {
            compaction: state
                .next_compaction(
                    facts.summary_draft("verify the local target boundary"),
                    2,
                    &facts.validation(),
                )
                .unwrap(),
        },
        id,
        sequence,
    )
}

#[test]
fn summary_v2_rebuilds_reference_only_continuation_without_rewriting_history() {
    let facts = CompactionFacts::new("summary-v2");
    let reducer = SessionReducer;
    let events = source_events();
    let source = reducer.reduce(events.clone()).unwrap();
    let original_history = source.history().items().to_vec();
    let compacted = compaction_event(&source, &facts, "compaction", 9);
    let mut state = source.clone();
    reducer.apply(&mut state, compacted.clone()).unwrap();

    let summary = state.latest_compaction().unwrap().summary();
    assert_eq!(summary.schema_version(), 2);
    assert_eq!(summary.revision(), 1);
    let spine = summary.evidence_spine().unwrap();
    assert_eq!(spine.source(), facts.research_stream());
    assert_eq!(
        spine.target_snapshot_refs(),
        [facts.target_snapshot_id().clone()]
    );
    assert!(spine.active_hypothesis_refs().is_empty());
    let narrative = summary.narrative().unwrap();
    assert_eq!(narrative.current_goal(), "verify the local target boundary");
    assert_eq!(
        narrative.concise_progress(),
        "the authoritative target boundary is retained by reference"
    );
    let encoded = serde_json::to_value(summary).unwrap();
    assert!(encoded.get("verified_facts").is_none());
    assert!(encoded.get("active_hypotheses").is_none());
    assert!(encoded.get("failed_paths").is_none());
    assert_eq!(state.history().items(), original_history);

    let replayed = reducer
        .reduce(events.into_iter().chain(std::iter::once(compacted)))
        .unwrap();
    assert_eq!(replayed, state);
}

#[test]
fn authoritative_research_artifact_and_receipt_facts_rebuild_without_summary_or_delivery_map() {
    let label = "derived-deletion";
    let project_id = ProjectId::new(format!("project-{label}"));
    let research_events = authoritative_research_events_with_artifact(label);
    let session_events = authoritative_session_events_with_receipt(label);

    // These raw streams deliberately contain no Compaction Summary and no
    // RequestAudit/ContextDeliveryMap. Rebuild therefore has only the
    // authoritative Research, Artifact and Receipt facts available.
    let research_stream =
        ResearchStreamIdentity::from_events(project_id.clone(), &research_events).unwrap();
    let research_state = ResearchReducer.reduce(research_events).unwrap();
    let session_state = SessionReducer.reduce(session_events).unwrap();
    assert_eq!(session_state.execution_receipt_count(), 1);

    let receipt_id = ExecutionReceiptId::new(format!("receipt-{label}"));
    let receipt_ids = BTreeSet::from([receipt_id.clone()]);
    let validation = CompactionValidationContext::new(
        &project_id,
        &research_stream,
        &research_state,
        &receipt_ids,
    );
    let draft = SessionSummaryDraft::new(
        EvidenceSpine::new(
            research_stream.clone(),
            vec![TargetSnapshotId::new(format!("target-snapshot-{label}"))],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            vec![ArtifactId::new(format!("artifact-{label}"))],
            vec![receipt_id],
        )
        .unwrap(),
        NarrativeSummary::new(
            "rebuild continuation",
            "facts were rebuilt without derived state",
            Vec::new(),
            Vec::new(),
        )
        .unwrap(),
    )
    .unwrap();

    let rebuilt = session_state
        .next_compaction(draft, 1, &validation)
        .unwrap();
    let spine = rebuilt.summary().evidence_spine().unwrap();
    assert_eq!(
        spine.artifact_refs(),
        [ArtifactId::new(format!("artifact-{label}"))]
    );
    assert_eq!(
        spine.execution_receipt_refs(),
        [ExecutionReceiptId::new(format!("receipt-{label}"))]
    );
}

#[test]
fn compaction_retention_keeps_a_real_recent_tool_pair_and_full_history_facts() {
    let facts = CompactionFacts::new("retention");
    let reducer = SessionReducer;
    let mut state = source_state();
    let original_history = state.history().items().to_vec();
    let compacted = compaction_event(&state, &facts, "compaction", 9);
    reducer.apply(&mut state, compacted).unwrap();

    let compaction = state.latest_compaction().unwrap();
    assert_eq!(
        compaction.source_history_item_count(),
        original_history.len()
    );
    assert_eq!(compaction.source_event_sequence(), EventSequence::new(8));
    assert!(compaction.retained_from_normalized_history_index() > 0);
    assert_eq!(
        compaction.retained_history(),
        [
            ContentItem::ToolCall {
                call: call("recent-call")
            },
            ContentItem::ToolResult {
                result: result("recent-call")
            },
            ContentItem::user_text("preserve the recent pair"),
        ]
    );
    assert_eq!(state.history().items(), original_history);
}

#[test]
fn compaction_rejects_schema_hash_source_retention_and_revision_changes_atomically() {
    let facts = CompactionFacts::new("tamper");
    let reducer = SessionReducer;
    let state = source_state();
    let valid = compaction_event(&state, &facts, "compaction", 9);

    let cases: Vec<(&str, CompactionEventAlteration)> = vec![
        (
            "unknown field",
            Box::new(|mut event| {
                event
                    .payload
                    .as_object_mut()
                    .unwrap()
                    .insert("unexpected".to_owned(), json!(true));
                event
            }),
        ),
        (
            "schema",
            Box::new(|mut event| {
                event.payload["schema_version"] = json!(99);
                event
            }),
        ),
        (
            "fingerprint",
            Box::new(|mut event| {
                event.payload["summary"]["fingerprint"] = json!("forged");
                event
            }),
        ),
        (
            "source sequence",
            Box::new(|mut event| {
                event.payload["source_event_sequence"] = json!(999);
                event
            }),
        ),
        (
            "retention",
            Box::new(|mut event| {
                event.payload["retained_from_normalized_history_index"] = json!(0);
                event
            }),
        ),
    ];
    for (name, alter) in cases {
        let mut candidate = state.clone();
        let before = candidate.clone();
        assert!(
            reducer.apply(&mut candidate, alter(valid.clone())).is_err(),
            "{name} must fail"
        );
        assert_eq!(candidate, before, "{name} must not partially apply");
    }

    let mut applied = state.clone();
    reducer.apply(&mut applied, valid).unwrap();
    let second = envelope(
        SessionEvent::CompactionCreated {
            compaction: applied
                .next_compaction(
                    facts.summary_draft("verify the local target boundary"),
                    2,
                    &facts.validation(),
                )
                .unwrap(),
        },
        "forged-revision",
        9,
    );
    let mut forged_revision = second;
    forged_revision.payload["source_event_sequence"] = json!(8);
    let mut candidate = state.clone();
    let before = candidate.clone();
    assert!(matches!(
        reducer.apply(&mut candidate, forged_revision),
        Err(SessionError::CompactionRevisionMismatch {
            expected: 1,
            received: 2,
        })
    ));
    assert_eq!(candidate, before);
}

#[test]
fn evidence_spine_rejects_cross_project_stale_wrong_type_and_missing_references() {
    let facts = CompactionFacts::new("reference-validation");
    let source = source_state();
    let events = raw_research_events("reference-validation");
    let cross_project =
        ResearchStreamIdentity::from_events(ProjectId::new("other"), &events).unwrap();
    let cross_project_draft = SessionSummaryDraft::new(
        EvidenceSpine::new(
            cross_project,
            vec![facts.target_snapshot_id().clone()],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap(),
        NarrativeSummary::new("goal", "progress", Vec::new(), Vec::new()).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        source.next_compaction(cross_project_draft, 2, &facts.validation()),
        Err(SessionError::CompactionEvidenceCrossProject { .. })
    ));

    let mut stale_source = serde_json::to_value(facts.research_stream()).unwrap();
    stale_source["stream_hash"] = json!("a".repeat(64));
    let stale_source: ResearchStreamIdentity = serde_json::from_value(stale_source).unwrap();
    let stale_draft = SessionSummaryDraft::new(
        EvidenceSpine::new(
            stale_source,
            vec![facts.target_snapshot_id().clone()],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap(),
        NarrativeSummary::new("goal", "progress", Vec::new(), Vec::new()).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        source.next_compaction(stale_draft, 2, &facts.validation()),
        Err(SessionError::CompactionEvidenceStaleResearchRevision { .. })
    ));

    let wrong_type = SessionSummaryDraft::new(
        EvidenceSpine::new(
            facts.research_stream().clone(),
            vec![facts.target_snapshot_id().clone()],
            vec![HypothesisId::new(facts.target_snapshot_id().as_str())],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap(),
        NarrativeSummary::new("goal", "progress", Vec::new(), Vec::new()).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        source.next_compaction(wrong_type, 2, &facts.validation()),
        Err(SessionError::CompactionEvidenceTypeMismatch { .. })
    ));

    let missing = SessionSummaryDraft::new(
        EvidenceSpine::new(
            facts.research_stream().clone(),
            vec![facts.target_snapshot_id().clone()],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            vec![ArtifactId::new("missing-artifact")],
            vec![ExecutionReceiptId::new("missing-receipt")],
        )
        .unwrap(),
        NarrativeSummary::new("goal", "progress", Vec::new(), Vec::new()).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        source.next_compaction(missing, 2, &facts.validation()),
        Err(SessionError::CompactionEvidenceReferenceMissing { .. })
    ));
}

#[test]
fn research_stream_identity_requires_the_complete_authoritative_stream() {
    let events = raw_research_events("stream-identity");
    let mut missing_prefix = events.clone();
    missing_prefix[0].sequence = EventSequence::new(2);
    assert!(matches!(
        ResearchStreamIdentity::from_events(
            ProjectId::new("project-stream-identity"),
            &missing_prefix
        ),
        Err(SessionError::CompactionResearchSequenceMismatch {
            expected: 1,
            received: 2,
        })
    ));

    let mut wrong_stream = events;
    wrong_stream[0].stream_id = StreamId::new("session:not-research");
    assert!(matches!(
        ResearchStreamIdentity::from_events(
            ProjectId::new("project-stream-identity"),
            &wrong_stream
        ),
        Err(SessionError::CompactionResearchStreamChanged { .. })
    ));
}

#[test]
fn raw_summary_v1_event_remains_readable_without_typed_evidence_upgrade() {
    let events = source_events();
    let state = SessionReducer.reduce(events.clone()).unwrap();
    let summary = legacy_v1_summary_value();
    let raw = EventEnvelope::current(
        EventId::new("legacy-compaction"),
        stream_id(),
        EventSequence::new(9),
        OccurredAt::new("2026-08-31T00:00:00Z"),
        EventType::new("session.compaction_created"),
        json!({
            "schema_version": 1,
            "summary": summary,
            "source_history_item_count": state.history().items().len(),
            "source_event_sequence": 8,
            "retained_from_normalized_history_index": 0,
            "retained_history": state.history().items(),
        }),
    );
    let replayed = SessionReducer
        .reduce(events.into_iter().chain(std::iter::once(raw)))
        .unwrap();
    let summary = replayed.latest_compaction().unwrap().summary();
    assert_eq!(summary.schema_version(), 1);
    assert!(summary.evidence_spine().is_none());
    assert!(summary.narrative().is_none());
    let encoded = serde_json::to_value(summary).unwrap();
    assert_eq!(encoded["verified_facts"], json!(["legacy copied claim"]));
    assert!(encoded.get("evidence_spine").is_none());
}

#[test]
fn invalid_or_oversized_v2_summary_and_append_failure_leave_state_unchanged() {
    assert!(matches!(
        NarrativeSummary::new("", "progress", Vec::new(), Vec::new()),
        Err(SessionError::CompactionSummaryFieldRequired {
            field: "narrative.current_goal"
        })
    ));
    assert!(matches!(
        NarrativeSummary::new(
            "goal",
            "progress",
            Vec::new(),
            vec!["repeat".to_owned(), "repeat".to_owned()]
        ),
        Err(SessionError::CompactionSummaryEntryDuplicate {
            field: "narrative.next_step_notes",
            ..
        })
    ));

    let facts = CompactionFacts::new("bounds");
    let source = source_state();
    let oversized = SessionSummaryDraft::new(
        EvidenceSpine::new(
            facts.research_stream().clone(),
            vec![facts.target_snapshot_id().clone()],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap(),
        NarrativeSummary::new("goal", "x".repeat(9 * 1024), Vec::new(), Vec::new()).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        source.next_compaction(oversized, 2, &facts.validation()),
        Err(SessionError::CompactionSummaryTooLarge { .. })
    ));

    struct FailingSink;
    impl EventSink<Value> for FailingSink {
        type Error = &'static str;

        fn append_event(&mut self, _event: &EventEnvelope<Value>) -> Result<(), Self::Error> {
            Err("disk unavailable")
        }
    }

    let mut projection = SessionProjection::replay(source_events()).unwrap();
    let before = projection.state().clone();
    let failed = compaction_event(projection.state(), &facts, "compaction", 9);
    assert!(matches!(
        projection.append(&mut FailingSink, failed),
        Err(AppendAfterDurableError::Sink("disk unavailable"))
    ));
    assert_eq!(projection.state(), &before);
}
