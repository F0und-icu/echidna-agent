use cyber_agent::app::session::{AppendAfterDurableError, SessionProjection};
use cyber_agent::domain::ArtifactId;
use cyber_agent::domain::context::{ProviderSessionId, ProviderSessionRef, ProviderSessionRole};
use cyber_agent::domain::events::{EventEnvelope, EventId, EventSequence, OccurredAt, StreamId};
use cyber_agent::domain::model::ContentItem;
use cyber_agent::domain::research::TargetSnapshotId;
use cyber_agent::domain::session::{
    SessionError, SessionEvent, SessionReducer, SessionState, SessionSummaryDraft,
    UnfinishedExecutionRef,
};
use cyber_agent::domain::tools::{
    ExecutionId, ToolCall, ToolCallId, ToolName, ToolResult, ToolResultStatus,
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
            OccurredAt::new("2026-08-23T00:00:00Z"),
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

fn summary_draft() -> SessionSummaryDraft {
    SessionSummaryDraft::new(
        "verify the local target boundary",
        "fixture.apk",
        "1.2.3",
        TargetSnapshotId::new("target-snapshot-fixture"),
        "Android 15 / ordinary app",
    )
    .unwrap()
    .with_verified_facts(["exported component requires a final impact check"])
    .with_active_hypotheses(["the WebView bridge may retain attacker input"])
    .with_failed_paths(["root-only database read is not reportable"])
    .with_artifacts([ArtifactId::new("artifact:fixture-log")])
    .with_modified_files(["/tmp/fixture-note.md"])
    .with_unfinished_execution([UnfinishedExecutionRef::Execution(ExecutionId::new(
        "execution:pending-network-capture",
    ))])
    .with_next_steps(["retest on the latest public build"])
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

fn compaction_event(state: &SessionState, id: &str, sequence: u64) -> EventEnvelope<Value> {
    envelope(
        SessionEvent::CompactionCreated {
            compaction: state.next_compaction(summary_draft(), 2).unwrap(),
        },
        id,
        sequence,
    )
}

#[test]
fn compaction_rebuilds_the_explicit_structured_summary_without_rewriting_history() {
    let reducer = SessionReducer;
    let events = source_events();
    let source = reducer.reduce(events.clone()).unwrap();
    let original_history = source.history().items().to_vec();
    let compacted = compaction_event(&source, "compaction", 9);
    let mut state = source.clone();
    reducer.apply(&mut state, compacted.clone()).unwrap();

    let summary = state.latest_compaction().unwrap().summary();
    assert_eq!(summary.revision(), 1);
    assert_eq!(summary.goal(), "verify the local target boundary");
    assert_eq!(summary.target(), "fixture.apk");
    assert_eq!(summary.target_version(), "1.2.3");
    assert_eq!(
        summary.target_snapshot().as_str(),
        "target-snapshot-fixture"
    );
    assert_eq!(summary.environment(), "Android 15 / ordinary app");
    assert_eq!(
        summary.verified_facts(),
        ["exported component requires a final impact check"]
    );
    assert_eq!(
        summary.active_hypotheses(),
        ["the WebView bridge may retain attacker input"]
    );
    assert_eq!(
        summary.failed_paths(),
        ["root-only database read is not reportable"]
    );
    assert_eq!(
        summary.artifacts(),
        [ArtifactId::new("artifact:fixture-log")]
    );
    assert_eq!(summary.modified_files(), ["/tmp/fixture-note.md"]);
    assert_eq!(
        summary.unfinished_execution(),
        [UnfinishedExecutionRef::Execution(ExecutionId::new(
            "execution:pending-network-capture"
        ))]
    );
    assert_eq!(summary.next_steps(), ["retest on the latest public build"]);
    assert_eq!(state.history().items(), original_history);

    let replayed = reducer
        .reduce(events.into_iter().chain(std::iter::once(compacted)))
        .unwrap();
    assert_eq!(replayed, state);
    assert_eq!(
        replayed
            .latest_compaction()
            .unwrap()
            .summary()
            .fingerprint(),
        summary.fingerprint()
    );
}

#[test]
fn compaction_retention_keeps_a_real_recent_tool_pair_and_full_history_facts() {
    let reducer = SessionReducer;
    let mut state = source_state();
    let original_history = state.history().items().to_vec();
    let compacted = compaction_event(&state, "compaction", 9);
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
fn compaction_rejects_strict_schema_hash_source_retention_and_revision_changes_atomically() {
    let reducer = SessionReducer;
    let state = source_state();
    let valid = compaction_event(&state, "compaction", 9);

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
                event.payload["schema_version"] = json!(2);
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
            compaction: applied.next_compaction(summary_draft(), 2).unwrap(),
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
fn required_summary_fields_and_compaction_append_failure_leave_state_unchanged() {
    assert!(matches!(
        SessionSummaryDraft::new(
            "",
            "target",
            "version",
            TargetSnapshotId::new("snapshot"),
            "environment"
        ),
        Err(SessionError::CompactionSummaryFieldRequired { field: "goal" })
    ));

    let source = source_state();
    let duplicate = summary_draft().with_next_steps(["repeat", "repeat"]);
    assert!(matches!(
        source.next_compaction(duplicate, 2),
        Err(SessionError::CompactionSummaryEntryDuplicate {
            field: "next_steps",
            ..
        })
    ));
    let oversized = summary_draft().with_verified_facts(["x".repeat(9 * 1024)]);
    assert!(matches!(
        source.next_compaction(oversized, 2),
        Err(SessionError::CompactionSummaryTooLarge { .. })
    ));

    struct FailingSink;
    impl EventSink<Value> for FailingSink {
        type Error = &'static str;

        fn append_event(&mut self, _event: &EventEnvelope<Value>) -> Result<(), Self::Error> {
            Err("disk unavailable")
        }
    }

    let events = source_events();
    let mut projection = SessionProjection::replay(events).unwrap();
    let before = projection.state().clone();
    let failed = compaction_event(projection.state(), "compaction", 9);
    assert!(matches!(
        projection.append(&mut FailingSink, failed),
        Err(AppendAfterDurableError::Sink("disk unavailable"))
    ));
    assert_eq!(projection.state(), &before);
}
