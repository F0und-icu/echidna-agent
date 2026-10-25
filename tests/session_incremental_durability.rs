//! Exact-prefix incremental projection and complete execution-outcome contracts.
include!("../src/lib.rs");
mod support;

mod durability {
    use std::fs;
    use std::io::Write;

    use serde_json::{Value, json};

    use crate::adapters::project_event_store::{
        Project, ProjectEventStoreError, ProjectEventStream,
    };
    use crate::app::session::SessionProjection;
    use crate::domain::context::{ProviderSessionId, ProviderSessionRef, ProviderSessionRole};
    use crate::domain::events::{EventEnvelope, EventId, EventSequence, OccurredAt, StreamId};
    use crate::domain::model::ContentItem;
    use crate::domain::session::{SessionEvent, SessionReducer};
    use crate::domain::tools::{
        ExecutionId, ExecutionReceipt, ToolCall, ToolCallId, ToolName, ToolResult, ToolResultStatus,
    };
    use crate::ports::{EventSink, EventSource};
    use crate::support::TemporaryDirectory;

    fn envelope(event: SessionEvent, sequence: u64) -> EventEnvelope<Value> {
        event
            .into_envelope(
                EventId::new(format!("durable-{sequence}")),
                ProjectEventStream::Session(StreamId::new("durability")).stream_id(),
                EventSequence::new(sequence),
                OccurredAt::new("2026-10-25T00:00:00Z"),
            )
            .unwrap()
    }

    fn started() -> EventEnvelope<Value> {
        envelope(
            SessionEvent::Started {
                primary_provider_session: ProviderSessionRef {
                    id: ProviderSessionId::new("durable-primary"),
                    role: ProviderSessionRole::Primary,
                    generation: 1,
                },
            },
            1,
        )
    }

    fn call(id: &str) -> ToolCall {
        ToolCall::new(
            ToolCallId::new(id),
            ToolName::new("read_file"),
            json!({"path":"never-executed.txt"}),
        )
    }

    fn outcome(call: &ToolCall) -> (ExecutionReceipt, ToolResult) {
        let receipt = serde_json::from_value(json!({
            "execution_id":ExecutionId::for_model_call(&call.call_id),
            "plan_fingerprint":"fixture-plan",
            "status":"succeeded", "exit_reason":null, "output_artifact_ids":[], "environment_change":null
        })).unwrap();
        let result = ToolResult {
            call_id: call.call_id.clone(),
            tool_name: call.name.clone(),
            status: ToolResultStatus::Succeeded,
            output: json!({"text":format!("real-recorded-{}",call.call_id)}),
        };
        (receipt, result)
    }

    #[test]
    fn suffix_refresh_sees_independent_receipts_and_matches_full_replay() {
        let root = TemporaryDirectory::new("session-suffix");
        let project = Project::initialize(root.path()).unwrap();
        let id = StreamId::new("durability");
        let stream = ProjectEventStream::Session(id.clone()).stream_id();
        let mut writer = project.open_writer().unwrap();
        let (mut store, events) = writer.open_session_projection(id.clone(), None).unwrap();
        let mut projection = SessionProjection::replay(events).unwrap();
        projection.append(&mut store, started()).unwrap();
        let cursor = store.projection_cursor();
        drop(store);
        drop(writer);

        let mut writer = project.open_writer().unwrap();
        let mut independent = writer.open_session(id.clone(), None).unwrap();
        independent
            .append_event(&envelope(
                SessionEvent::ExecutionReceiptRecorded {
                    receipt: outcome(&call("external")).0,
                },
                2,
            ))
            .unwrap();
        drop(independent);
        drop(writer);

        let mut writer = project.open_writer().unwrap();
        let (mut store, suffix) = writer
            .open_session_projection(id.clone(), Some(&cursor))
            .unwrap();
        assert_eq!(suffix.len(), 1);
        projection.ingest_durable(suffix).unwrap();
        assert_eq!(projection.state().execution_receipt_count(), 1);
        assert_eq!(store.next_sequence().unwrap().get(), 3);
        projection
            .append(
                &mut store,
                envelope(
                    SessionEvent::HistoryItemRecorded {
                        item: ContentItem::user_text("next"),
                    },
                    3,
                ),
            )
            .unwrap();
        let full = SessionReducer
            .reduce(store.read_events(&stream).unwrap())
            .unwrap();
        assert_eq!(projection.state(), &full);
        let cursor = store.projection_cursor();
        drop(store);
        drop(writer);
        let mut writer = project.open_writer().unwrap();
        let (_, suffix) = writer.open_session_projection(id, Some(&cursor)).unwrap();
        assert!(suffix.is_empty());
    }

    #[test]
    fn cursor_rejects_same_length_prefix_mutation_and_truncation() {
        for truncate in [false, true] {
            let root = TemporaryDirectory::new("session-prefix-change");
            let project = Project::initialize(root.path()).unwrap();
            let id = StreamId::new("durability");
            let mut writer = project.open_writer().unwrap();
            let (mut store, _) = writer.open_session_projection(id.clone(), None).unwrap();
            store.append_event(&started()).unwrap();
            let cursor = store.projection_cursor();
            drop(store);
            drop(writer);
            let path = root
                .path()
                .join(".echidna/sessions/durability/events.jsonl");
            let mut bytes = fs::read(&path).unwrap();
            if truncate {
                bytes.pop();
            } else {
                let i = bytes.iter().position(|b| *b == b'2').unwrap();
                bytes[i] = b'3';
            }
            fs::write(&path, &bytes).unwrap();
            let mut writer = project.open_writer().unwrap();
            assert!(matches!(
                writer.open_session_projection(id, Some(&cursor)),
                Err(ProjectEventStoreError::ValidatedPrefixChanged { .. })
            ));
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
    }

    #[test]
    fn suffix_keeps_scanner_sequence_corruption_and_tail_boundaries() {
        for (tail, label) in [
            (b"not-json\n{}\n".as_slice(), "middle"),
            (b"not-json".as_slice(), "tail"),
        ] {
            let root = TemporaryDirectory::new("session-suffix-invalid");
            let project = Project::initialize(root.path()).unwrap();
            let id = StreamId::new("durability");
            let mut writer = project.open_writer().unwrap();
            let (mut store, _) = writer.open_session_projection(id.clone(), None).unwrap();
            store.append_event(&started()).unwrap();
            let cursor = store.projection_cursor();
            drop(store);
            drop(writer);
            let path = root
                .path()
                .join(".echidna/sessions/durability/events.jsonl");
            let offset = fs::metadata(&path).unwrap().len();
            fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap()
                .write_all(tail)
                .unwrap();
            let mut writer = project.open_writer().unwrap();
            let error = writer
                .open_session_projection(id, Some(&cursor))
                .err()
                .unwrap();
            match (label, error) {
                ("middle", ProjectEventStoreError::NonTailCorruption { offset: actual, .. })
                | (
                    "tail",
                    ProjectEventStoreError::RecoveryMetadataRequired { offset: actual, .. },
                ) => assert_eq!(actual, offset),
                (_, error) => panic!("unexpected error: {error}"),
            }
        }
    }

    #[test]
    fn recorded_parallel_outcomes_recover_exact_results_in_call_order() {
        let calls = [call("a"), call("b"), call("unknown")];
        let mut events = vec![started()];
        for call in &calls {
            events.push(envelope(
                SessionEvent::HistoryItemRecorded {
                    item: ContentItem::ToolCall { call: call.clone() },
                },
                events.len() as u64 + 1,
            ));
        }
        for call in calls[..2].iter().rev() {
            let (receipt, result) = outcome(call);
            events.push(envelope(
                SessionEvent::ExecutionOutcomeRecorded { receipt, result },
                events.len() as u64 + 1,
            ));
        }
        let mut state = SessionReducer.reduce(events.clone()).unwrap();
        let normalized = state.normalized_history_items().unwrap();
        assert_eq!(normalized.len(), 6);
        assert_eq!(
            normalized[3],
            ContentItem::ToolResult {
                result: outcome(&calls[0]).1
            }
        );
        assert_eq!(
            normalized[4],
            ContentItem::ToolResult {
                result: outcome(&calls[1]).1
            }
        );
        let ContentItem::ToolResult { result } = &normalized[5] else {
            panic!("missing unknown marker")
        };
        assert_eq!(result.output["execution_status"], "unknown");
        assert_eq!(
            state.history().items().len(),
            3,
            "recovery never rewrites original History"
        );
        for call in &calls[..2] {
            let event = envelope(
                SessionEvent::HistoryItemRecorded {
                    item: ContentItem::ToolResult {
                        result: outcome(call).1,
                    },
                },
                events.len() as u64 + 1,
            );
            SessionReducer.apply(&mut state, event.clone()).unwrap();
            events.push(event);
        }
        assert_eq!(state.history().items()[3], normalized[3]);
        assert_eq!(state.history().items()[4], normalized[4]);
        assert_eq!(SessionReducer.reduce(events).unwrap(), state);
    }

    #[test]
    fn outcome_identity_status_capture_and_later_history_conflicts_are_atomic() {
        let call = call("validated");
        let prefix = vec![
            started(),
            envelope(
                SessionEvent::HistoryItemRecorded {
                    item: ContentItem::ToolCall { call: call.clone() },
                },
                2,
            ),
        ];
        for variant in 0..5 {
            let mut state = SessionReducer.reduce(prefix.clone()).unwrap();
            let before = state.clone();
            let (mut receipt, mut result) = outcome(&call);
            match variant {
                0 => result.tool_name = ToolName::new("other"),
                1 => receipt.execution_id = ExecutionId::new("unrelated"),
                2 => result.status = ToolResultStatus::Failed,
                3 => result.output["output_captures"] = json!([{"invalid":true}]),
                _ => result.call_id = ToolCallId::new("missing"),
            }
            assert!(
                SessionReducer
                    .apply(
                        &mut state,
                        envelope(
                            SessionEvent::ExecutionOutcomeRecorded { receipt, result },
                            3
                        )
                    )
                    .is_err()
            );
            assert_eq!(state, before);
        }
        let (receipt, result) = outcome(&call);
        let mut state = SessionReducer.reduce(prefix).unwrap();
        SessionReducer
            .apply(
                &mut state,
                envelope(
                    SessionEvent::ExecutionOutcomeRecorded {
                        receipt: receipt.clone(),
                        result: result.clone(),
                    },
                    3,
                ),
            )
            .unwrap();
        let before = state.clone();
        assert!(
            SessionReducer
                .apply(
                    &mut state,
                    envelope(
                        SessionEvent::ExecutionOutcomeRecorded {
                            receipt,
                            result: result.clone()
                        },
                        4
                    )
                )
                .is_err()
        );
        assert_eq!(state, before);
        let mut conflicting = result.clone();
        conflicting.output = json!({"text":"different"});
        assert!(
            SessionReducer
                .apply(
                    &mut state,
                    envelope(
                        SessionEvent::HistoryItemRecorded {
                            item: ContentItem::ToolResult {
                                result: conflicting
                            }
                        },
                        4
                    )
                )
                .is_err()
        );
        assert_eq!(state, before);
        SessionReducer
            .apply(
                &mut state,
                envelope(
                    SessionEvent::HistoryItemRecorded {
                        item: ContentItem::ToolResult { result },
                    },
                    4,
                ),
            )
            .unwrap();
    }

    #[test]
    fn invalid_durable_suffix_does_not_partially_mutate_projection() {
        let mut projection = SessionProjection::replay(vec![started()]).unwrap();
        let before = projection.state().clone();
        let good = envelope(
            SessionEvent::HistoryItemRecorded {
                item: ContentItem::user_text("not-promoted"),
            },
            2,
        );
        let bad = envelope(
            SessionEvent::HistoryItemRecorded {
                item: ContentItem::ToolResult {
                    result: outcome(&call("missing")).1,
                },
            },
            3,
        );
        assert!(projection.ingest_durable(vec![good, bad]).is_err());
        assert_eq!(projection.state(), &before);
    }

    #[test]
    fn existing_cache_hit_receipt_is_enriched_with_its_exact_recorded_result_once() {
        let call = call("cached-call");
        let (mut receipt, result) = outcome(&call);
        receipt.cache_source = Some(serde_json::from_value(json!({
            "cache_key":"cache-key", "source_stream_id":"session:source", "source_receipt_event_id":"source-receipt",
            "source_execution_id":"execution:source", "source_plan_fingerprint":"source-plan"
        })).unwrap());
        let events = vec![
            started(),
            envelope(
                SessionEvent::HistoryItemRecorded {
                    item: ContentItem::ToolCall { call },
                },
                2,
            ),
            envelope(
                SessionEvent::ToolResultCacheHit {
                    receipt: receipt.clone(),
                },
                3,
            ),
            envelope(
                SessionEvent::ExecutionOutcomeRecorded {
                    receipt,
                    result: result.clone(),
                },
                4,
            ),
        ];
        let state = SessionReducer.reduce(events).unwrap();
        assert_eq!(state.execution_receipts().len(), 1);
        assert_eq!(state.execution_receipt_count(), 0);
        assert_eq!(state.cache_hit_count(), 1);
        assert_eq!(state.execution_receipts()[0].event_id.as_str(), "durable-3");
        assert_eq!(state.recorded_tool_result(&result.call_id), Some(&result));
        assert_eq!(
            state.normalized_history_items().unwrap()[1],
            ContentItem::ToolResult { result }
        );
        assert_eq!(state.history().items().len(), 1);
    }

    #[test]
    fn suffix_first_sequence_must_follow_the_verified_prefix() {
        let root = TemporaryDirectory::new("session-suffix-sequence");
        let project = Project::initialize(root.path()).unwrap();
        let id = StreamId::new("durability");
        let mut writer = project.open_writer().unwrap();
        let (mut store, _) = writer.open_session_projection(id.clone(), None).unwrap();
        store.append_event(&started()).unwrap();
        let cursor = store.projection_cursor();
        drop(store);
        drop(writer);
        let path = root
            .path()
            .join(".echidna/sessions/durability/events.jsonl");
        let offset = fs::metadata(&path).unwrap().len();
        let mut bytes = serde_json::to_vec(&envelope(
            SessionEvent::HistoryItemRecorded {
                item: ContentItem::user_text("sequence-gap"),
            },
            3,
        ))
        .unwrap();
        bytes.push(b'\n');
        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(&bytes)
            .unwrap();
        let mut writer = project.open_writer().unwrap();
        assert!(
            matches!(writer.open_session_projection(id, Some(&cursor)), Err(ProjectEventStoreError::InvalidEventRecord { offset: actual, .. }) if actual == offset)
        );
    }

    #[test]
    fn unknown_suffix_event_is_not_silently_filtered_or_promoted() {
        let root = TemporaryDirectory::new("session-suffix-unknown");
        let project = Project::initialize(root.path()).unwrap();
        let id = StreamId::new("durability");
        let mut writer = project.open_writer().unwrap();
        let (mut store, events) = writer.open_session_projection(id.clone(), None).unwrap();
        let mut projection = SessionProjection::replay(events).unwrap();
        projection.append(&mut store, started()).unwrap();
        let before = projection.state().clone();
        let cursor = store.projection_cursor();
        drop(store);
        drop(writer);
        let mut writer = project.open_writer().unwrap();
        let mut store = writer.open_session(id.clone(), None).unwrap();
        let mut unknown = envelope(
            SessionEvent::HistoryItemRecorded {
                item: ContentItem::user_text("never promoted"),
            },
            2,
        );
        unknown.event_type = crate::domain::events::EventType::new("unknown.session.fact");
        store.append_event(&unknown).unwrap();
        drop(store);
        drop(writer);
        let mut writer = project.open_writer().unwrap();
        let (_, suffix) = writer.open_session_projection(id, Some(&cursor)).unwrap();
        assert_eq!(suffix.len(), 1);
        assert!(matches!(
            projection.ingest_durable(suffix),
            Err(crate::domain::session::SessionError::UnknownEventType { .. })
        ));
        assert_eq!(projection.state(), &before);
    }
}
