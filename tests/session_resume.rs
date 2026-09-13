mod support;

use cyber_agent::adapters::project_event_store::{Project, ProjectEventStream};
use cyber_agent::app::session::{AppendAfterDurableError, SessionProjection};
use cyber_agent::domain::context::{ProviderSessionId, ProviderSessionRef, ProviderSessionRole};
use cyber_agent::domain::events::{
    EventEnvelope, EventId, EventSequence, EventType, OccurredAt, StreamId,
};
use cyber_agent::domain::history::normalize_content_items;
use cyber_agent::domain::model::ContentItem;
use cyber_agent::domain::session::{
    SessionError, SessionEvent, SessionLifecycle, SessionReducer, SessionState,
};
use cyber_agent::domain::tools::{ToolCall, ToolCallId, ToolName, ToolResult, ToolResultStatus};
use cyber_agent::ports::{EventSink, EventSource};
use serde_json::{Value, json};
use support::TemporaryDirectory;

fn primary_session(id: &str, generation: u64) -> ProviderSessionRef {
    ProviderSessionRef {
        id: ProviderSessionId::new(id),
        role: ProviderSessionRole::Primary,
        generation,
    }
}

fn session_stream() -> StreamId {
    ProjectEventStream::Session(StreamId::new("resume-fixture")).stream_id()
}

fn event(event_id: &str, sequence: u64, event_type: &str, payload: Value) -> EventEnvelope<Value> {
    EventEnvelope::current(
        EventId::new(event_id),
        session_stream(),
        EventSequence::new(sequence),
        OccurredAt::new("2026-08-23T00:00:00Z"),
        EventType::new(event_type),
        payload,
    )
}

fn session_started(sequence: u64) -> EventEnvelope<Value> {
    SessionEvent::Started {
        primary_provider_session: primary_session("primary-initial", 1),
    }
    .into_envelope(
        EventId::new(format!("started-{sequence}")),
        session_stream(),
        EventSequence::new(sequence),
        OccurredAt::new("2026-08-23T00:00:00Z"),
    )
    .unwrap()
}

#[test]
fn output_capture_receipts_upcast_legacy_and_reject_corrupt_v2_or_unpaired_refs() {
    let receipt = json!({"execution_id":"execution:capture","plan_fingerprint":"plan","status":"succeeded",
        "exit_reason":null,"output_artifact_ids":[],"environment_change":null});
    let capture = json!({"stream":"stdout","child_index":null,"blob_id":"a".repeat(64),
        "observed_bytes":4,"retained_bytes":4,"eof":true,"complete":true,"read_error":null});
    let reducer = SessionReducer;
    let mut legacy = SessionState::default();
    reducer.apply(&mut legacy, session_started(1)).unwrap();
    reducer
        .apply(
            &mut legacy,
            event(
                "legacy",
                2,
                "tool.execution_receipt_recorded",
                json!({"schema_version":1,"receipt":receipt}),
            ),
        )
        .unwrap();
    assert_eq!(legacy.execution_receipt_count(), 1);
    for kind in [
        "valid",
        "legacy-smuggle",
        "bad-completeness",
        "oversized",
        "duplicate",
        "future",
    ] {
        let mut payload = json!({"schema_version":2,"receipt":receipt});
        payload["receipt"]["output_captures"] = json!([capture]);
        match kind {
            "legacy-smuggle" => payload["schema_version"] = json!(1),
            "bad-completeness" => payload["receipt"]["output_captures"][0]["eof"] = json!(false),
            "oversized" => {
                payload["receipt"]["output_captures"][0]["retained_bytes"] = json!(1048577)
            }
            "duplicate" => payload["receipt"]["output_captures"] = json!([capture, capture]),
            "future" => payload["schema_version"] = json!(3),
            _ => (),
        }
        let mut state = SessionState::default();
        reducer.apply(&mut state, session_started(1)).unwrap();
        let before = state.clone();
        let result = reducer.apply(
            &mut state,
            event("capture", 2, "tool.execution_receipt_recorded", payload),
        );
        if kind != "valid" {
            assert!(result.is_err(), "{kind}");
            assert_eq!(state, before);
            continue;
        }
        result.unwrap();
        let call = ToolCall::new(
            ToolCallId::new("capture"),
            ToolName::new("run_command"),
            json!({"argv":["true"]}),
        );
        let item = ContentItem::ToolCall { call };
        reducer
            .apply(
                &mut state,
                event(
                    "call",
                    3,
                    "session.history_item_recorded",
                    json!({"schema_version":1,"item":item}),
                ),
            )
            .unwrap();
        let base = state.clone();
        for refs in [None, Some(json!([])), Some(json!([capture]))] {
            let mut output = json!({});
            if let Some(refs) = refs {
                output["output_captures"] = refs;
            }
            let valid = output["output_captures"] == json!([capture]);
            let item = ContentItem::ToolResult {
                result: ToolResult {
                    call_id: ToolCallId::new("capture"),
                    tool_name: ToolName::new("run_command"),
                    status: ToolResultStatus::Succeeded,
                    output,
                },
            };
            let mut candidate = base.clone();
            let result = reducer.apply(
                &mut candidate,
                event(
                    "result",
                    4,
                    "session.history_item_recorded",
                    json!({"schema_version":1,"item":item}),
                ),
            );
            assert_eq!(result.is_ok(), valid);
            if !valid {
                assert_eq!(candidate, base);
            }
        }
    }
}

#[test]
fn session_reducer_rejects_unknown_schema_and_illegal_transitions_without_mutating_state() {
    let reducer = SessionReducer;
    let mut state = SessionState::default();
    reducer.apply(&mut state, session_started(1)).unwrap();
    let started_state = state.clone();

    assert!(matches!(
        reducer.apply(
            &mut state,
            event(
                "unknown",
                2,
                "session.unknown",
                json!({"schema_version": 1})
            )
        ),
        Err(SessionError::UnknownEventType { .. })
    ));
    assert_eq!(state, started_state);

    assert!(matches!(
        reducer.apply(
            &mut state,
            event(
                "unknown-receipt-field",
                2,
                "tool.execution_receipt_recorded",
                json!({
                    "schema_version": 1,
                    "receipt": {
                        "execution_id": "execution-unknown",
                        "plan_fingerprint": "fingerprint",
                        "status": "succeeded",
                        "exit_reason": null,
                        "output_artifact_ids": [],
                        "environment_change": null,
                        "unexpected": true
                    }
                })
            )
        ),
        Err(SessionError::MalformedPayload { .. })
    ));
    assert_eq!(state, started_state);

    assert!(matches!(
        reducer.apply(
            &mut state,
            event(
                "nested-unknown-field",
                2,
                "session.resumed",
                json!({
                    "schema_version": 1,
                    "primary_provider_session": {
                        "id": "primary-nested",
                        "role": "primary",
                        "generation": 2,
                        "unexpected": true
                    }
                })
            )
        ),
        Err(SessionError::MalformedPayload { .. })
    ));
    assert_eq!(state, started_state);

    assert!(matches!(
        reducer.apply(
            &mut state,
            event(
                "invalid-domain-selection",
                2,
                "session.domain_profile_selected",
                json!({
                    "schema_version": 1,
                    "selection": {
                        "profile": "mobile",
                        "profile_generation": 1,
                        "source": "default",
                        "reason_code": "NEW_SESSION_DEFAULT_SOURCE"
                    }
                })
            )
        ),
        Err(SessionError::MalformedPayload { .. })
    ));
    assert_eq!(state, started_state);

    assert!(matches!(
        reducer.apply(
            &mut state,
            event(
                "unknown-domain-field",
                2,
                "session.domain_profile_selected",
                json!({
                    "schema_version": 1,
                    "selection": {
                        "profile": "source",
                        "profile_generation": 1,
                        "source": "default",
                        "reason_code": "NEW_SESSION_DEFAULT_SOURCE",
                        "unexpected": true
                    }
                })
            )
        ),
        Err(SessionError::MalformedPayload { .. })
    ));
    assert_eq!(state, started_state);

    assert!(matches!(
        reducer.apply(
            &mut state,
            event(
                "unsupported-schema",
                2,
                "session.ended",
                json!({"schema_version": 2})
            )
        ),
        Err(SessionError::UnsupportedPayloadSchema {
            schema_version: 2,
            ..
        })
    ));
    assert_eq!(state, started_state);

    assert!(matches!(
        reducer.apply(
            &mut state,
            event(
                "unsupported-receipt-schema",
                2,
                "tool.execution_receipt_recorded",
                json!({
                    "schema_version": 3,
                    "receipt": {
                        "execution_id": "execution-unsupported",
                        "plan_fingerprint": "fingerprint",
                        "status": "succeeded",
                        "exit_reason": null,
                        "output_artifact_ids": [],
                        "environment_change": null
                    }
                })
            )
        ),
        Err(SessionError::UnsupportedPayloadSchema {
            schema_version: 3,
            ..
        })
    ));
    assert_eq!(state, started_state);

    assert!(matches!(
        reducer.apply(
            &mut state,
            SessionEvent::Ended
                .into_envelope(
                    EventId::new("sequence-gap"),
                    session_stream(),
                    EventSequence::new(3),
                    OccurredAt::new("2026-08-23T00:00:00Z"),
                )
                .unwrap(),
        ),
        Err(SessionError::Event(
            cyber_agent::domain::events::EventError::UnexpectedSequence { .. }
        ))
    ));
    assert_eq!(state, started_state);

    let same_primary = SessionEvent::Resumed {
        primary_provider_session: primary_session("primary-initial", 2),
    }
    .into_envelope(
        EventId::new("same-primary"),
        session_stream(),
        EventSequence::new(2),
        OccurredAt::new("2026-08-23T00:00:00Z"),
    )
    .unwrap();
    assert!(matches!(
        reducer.apply(&mut state, same_primary),
        Err(SessionError::ProviderSessionIdReused { provider_session_id })
            if provider_session_id.to_string() == "primary-initial"
    ));
    assert_eq!(state, started_state);

    let resumed_with_b = SessionEvent::Resumed {
        primary_provider_session: primary_session("primary-b", 2),
    }
    .into_envelope(
        EventId::new("primary-b"),
        session_stream(),
        EventSequence::new(2),
        OccurredAt::new("2026-08-23T00:00:00Z"),
    )
    .unwrap();
    reducer.apply(&mut state, resumed_with_b).unwrap();
    let state_after_b = state.clone();
    let reused_a = SessionEvent::Resumed {
        primary_provider_session: primary_session("primary-initial", 3),
    }
    .into_envelope(
        EventId::new("reused-primary-a"),
        session_stream(),
        EventSequence::new(3),
        OccurredAt::new("2026-08-23T00:00:00Z"),
    )
    .unwrap();
    assert!(matches!(
        reducer.apply(&mut state, reused_a),
        Err(SessionError::ProviderSessionIdReused { provider_session_id })
            if provider_session_id.to_string() == "primary-initial"
    ));
    assert_eq!(state, state_after_b);

    let mut empty = SessionState::default();
    assert!(matches!(
        reducer.apply(
            &mut empty,
            SessionEvent::Ended
                .into_envelope(
                    EventId::new("ended-without-start"),
                    session_stream(),
                    EventSequence::new(1),
                    OccurredAt::new("2026-08-23T00:00:00Z"),
                )
                .unwrap(),
        ),
        Err(SessionError::InvalidTransition {
            event_type: "session.ended",
            current: SessionLifecycle::Unstarted,
        })
    ));
    assert_eq!(empty, SessionState::default());

    let mut unstarted_history = SessionState::default();
    assert!(matches!(
        reducer.apply(
            &mut unstarted_history,
            SessionEvent::HistoryItemRecorded {
                item: ContentItem::user_text("must follow session start"),
            }
            .into_envelope(
                EventId::new("history-before-start"),
                session_stream(),
                EventSequence::new(1),
                OccurredAt::new("2026-08-23T00:00:00Z"),
            )
            .unwrap(),
        ),
        Err(SessionError::InvalidTransition {
            event_type: "session.history_item_recorded",
            current: SessionLifecycle::Unstarted,
        })
    ));
    assert_eq!(unstarted_history, SessionState::default());
}

#[test]
fn session_reducer_strictly_replays_existing_tool_cache_and_recovery_events() {
    let receipt = json!({
        "schema_version": 1,
        "receipt": {
            "execution_id": "execution-cache-source",
            "plan_fingerprint": "fingerprint",
            "status": "succeeded",
            "exit_reason": null,
            "output_artifact_ids": [],
            "environment_change": null
        }
    });
    let state = SessionReducer
        .reduce([
            event(
                "receipt-recorded",
                1,
                "tool.execution_receipt_recorded",
                receipt.clone(),
            ),
            event("cache-hit", 2, "tool.result_cache_hit", receipt),
            event(
                "recovery",
                3,
                "StreamRecovered",
                json!({
                    "recovery_artifact": "artifacts/recovery/tail.bin",
                    "truncated_from_offset": 42,
                    "resumed_after_sequence": 2
                }),
            ),
        ])
        .unwrap();

    assert_eq!(state.lifecycle(), SessionLifecycle::Unstarted);
    assert_eq!(state.execution_receipt_count(), 1);
    assert_eq!(state.cache_hit_count(), 1);
    assert_eq!(state.recovery_count(), 1);
    assert_eq!(state.applied_event_count(), 3);
}

#[test]
fn session_history_facts_replay_through_the_single_pairing_owner_and_normalize_interruption() {
    let completed_call = ToolCall::new(
        ToolCallId::new("completed"),
        ToolName::new("read_file"),
        json!({"path": "Cargo.toml"}),
    );
    let open_call = ToolCall::new(
        ToolCallId::new("open"),
        ToolName::new("list_files"),
        json!({}),
    );
    let history_event = |event_id: &str, sequence: u64, item: ContentItem| {
        SessionEvent::HistoryItemRecorded { item }
            .into_envelope(
                EventId::new(event_id),
                session_stream(),
                EventSequence::new(sequence),
                OccurredAt::new("2026-08-23T00:00:00Z"),
            )
            .unwrap()
    };
    let state = SessionReducer
        .reduce([
            session_started(1),
            history_event("user", 2, ContentItem::user_text("inspect project")),
            history_event(
                "assistant",
                3,
                ContentItem::assistant_text("I will inspect it."),
            ),
            history_event(
                "call-completed",
                4,
                ContentItem::ToolCall {
                    call: completed_call.clone(),
                },
            ),
            history_event(
                "result-completed",
                5,
                ContentItem::ToolResult {
                    result: ToolResult {
                        call_id: completed_call.call_id.clone(),
                        tool_name: completed_call.name.clone(),
                        status: ToolResultStatus::Succeeded,
                        output: json!({"contents": "package"}),
                    },
                },
            ),
            history_event("call-open", 6, ContentItem::ToolCall { call: open_call }),
        ])
        .unwrap();
    assert_eq!(state.history().items().len(), 5);
    let normalized = normalize_content_items(state.history().items()).unwrap();
    assert_eq!(normalized.len(), 6);
    assert!(matches!(
        normalized.last(),
        Some(ContentItem::ToolResult { result })
            if result.call_id.as_str() == "open"
                && result.status == ToolResultStatus::Cancelled
                && result.output == json!({
                    "error": "aborted",
                    "execution_status": "unknown",
                    "guidance": "do_not_assume_no_effect_or_auto_rerun"
                })
    ));

    let duplicate = SessionReducer.reduce([
        session_started(1),
        history_event(
            "first-call",
            2,
            ContentItem::ToolCall {
                call: ToolCall::new(
                    ToolCallId::new("duplicate"),
                    ToolName::new("read_file"),
                    json!({}),
                ),
            },
        ),
        history_event(
            "second-call",
            3,
            ContentItem::ToolCall {
                call: ToolCall::new(
                    ToolCallId::new("duplicate"),
                    ToolName::new("read_file"),
                    json!({}),
                ),
            },
        ),
    ]);
    assert!(matches!(
        duplicate,
        Err(SessionError::History(
            cyber_agent::domain::history::HistoryError::DuplicateToolCall { .. }
        ))
    ));

    let mismatched = SessionReducer.reduce([
        session_started(1),
        history_event(
            "mismatch-call",
            2,
            ContentItem::ToolCall {
                call: ToolCall::new(
                    ToolCallId::new("mismatch"),
                    ToolName::new("read_file"),
                    json!({}),
                ),
            },
        ),
        history_event(
            "mismatch-result",
            3,
            ContentItem::ToolResult {
                result: ToolResult {
                    call_id: ToolCallId::new("mismatch"),
                    tool_name: ToolName::new("list_files"),
                    status: ToolResultStatus::Succeeded,
                    output: json!({}),
                },
            },
        ),
    ]);
    assert!(matches!(
        mismatched,
        Err(SessionError::History(
            cyber_agent::domain::history::HistoryError::ToolNameMismatch { .. }
        ))
    ));
}

#[test]
fn session_projection_replays_project_events_and_never_applies_a_failed_append() {
    let root = TemporaryDirectory::new("session-journal");
    let project = Project::initialize(root.path()).unwrap();
    let session_id = StreamId::new("resume-fixture");
    let stream_id = ProjectEventStream::Session(session_id.clone()).stream_id();
    {
        let mut writer = project.open_writer().unwrap();
        let mut store = writer.open_session(session_id, None).unwrap();
        let mut projection =
            SessionProjection::replay(EventSource::read_events(&store, &stream_id).unwrap())
                .unwrap();
        assert_eq!(projection.state(), &SessionState::default());
        let started = SessionEvent::Started {
            primary_provider_session: primary_session("primary-initial", 1),
        }
        .into_envelope(
            EventId::new("started"),
            stream_id.clone(),
            store.next_sequence().unwrap(),
            OccurredAt::new("2026-08-23T00:00:00Z"),
        )
        .unwrap();
        projection.append(&mut store, started.clone()).unwrap();
        assert_eq!(projection.state().lifecycle(), SessionLifecycle::Active);
        assert_eq!(
            projection.append(&mut store, started).unwrap(),
            cyber_agent::domain::events::ApplyOutcome::DuplicateIgnored
        );
        assert_eq!(store.next_sequence().unwrap(), EventSequence::new(2));
    }
    let replayed = {
        let mut writer = project.open_writer().unwrap();
        let store = writer
            .open_session(StreamId::new("resume-fixture"), None)
            .unwrap();
        SessionProjection::replay(EventSource::read_events(&store, &stream_id).unwrap()).unwrap()
    };
    assert_eq!(replayed.state().lifecycle(), SessionLifecycle::Active);
    assert_eq!(replayed.state().applied_event_count(), 1);

    struct FailingSink;
    impl EventSink<Value> for FailingSink {
        type Error = &'static str;

        fn append_event(&mut self, _event: &EventEnvelope<Value>) -> Result<(), Self::Error> {
            Err("disk unavailable")
        }
    }

    let mut projection = SessionProjection::replay(Vec::<EventEnvelope<Value>>::new()).unwrap();
    let before = projection.state().clone();
    let mut sink = FailingSink;
    let failed_event = session_started(1);
    assert!(matches!(
        projection.append(&mut sink, failed_event),
        Err(AppendAfterDurableError::Sink("disk unavailable"))
    ));
    assert_eq!(projection.state(), &before);
}
