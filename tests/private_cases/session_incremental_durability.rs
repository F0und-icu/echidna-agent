//! Exact-prefix incremental projection and complete execution-outcome contracts.

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

    fn assert_source(projection: &SessionProjection, events: &[EventEnvelope<Value>]) {
        let project = crate::domain::project::ProjectId::new("source-tracking");
        let (expected, state) = crate::domain::session::SessionSourceIdentity::from_events(
            project.clone(),
            events,
            &[],
        )
        .unwrap();
        assert_eq!(
            projection.task_source_identity(project, &[]).unwrap(),
            expected
        );
        assert_eq!(projection.state(), &state);
    }

    #[test]
    fn incremental_source_identity_matches_replay_and_failed_writes_are_atomic() {
        struct Sink {
            fail: bool,
            events: Vec<EventEnvelope<Value>>,
        }
        impl EventSink<Value> for Sink {
            type Error = &'static str;
            fn append_event(&mut self, event: &EventEnvelope<Value>) -> Result<(), Self::Error> {
                if self.fail {
                    return Err("injected append failure");
                }
                self.events.push(event.clone());
                Ok(())
            }
        }
        let root = TemporaryDirectory::new("tracked-source");
        let project = Project::initialize(root.path()).unwrap();
        let blobs = crate::adapters::artifact_store::LocalArtifactStore::for_project(&project);
        let mut sink = Sink {
            fail: false,
            events: vec![started()],
        };
        let mut projection = SessionProjection::replay(sink.events.clone()).unwrap();
        assert_source(&projection, &sink.events);
        for sequence in 2..6 {
            let event = envelope(
                SessionEvent::HistoryItemRecorded {
                    item: ContentItem::user_text(format!("input-{sequence}")),
                },
                sequence,
            );
            sink.fail = true;
            assert!(projection.append(&mut sink, event.clone()).is_err());
            assert_source(&projection, &sink.events);
            sink.fail = false;
            projection.append(&mut sink, event.clone()).unwrap();
            assert_source(&projection, &sink.events);
            assert_eq!(
                projection.append(&mut sink, event).unwrap(),
                crate::domain::events::ApplyOutcome::DuplicateIgnored
            );
            assert_source(&projection, &sink.events);
        }
        let suffix = envelope(
            SessionEvent::HistoryItemRecorded {
                item: ContentItem::assistant_text("durable suffix"),
            },
            6,
        );
        projection
            .ingest_durable_verified(vec![suffix.clone()], &blobs)
            .unwrap();
        sink.events.push(suffix.clone());
        assert_source(&projection, &sink.events);
        projection
            .ingest_durable_verified(vec![suffix], &blobs)
            .unwrap();
        assert_source(&projection, &sink.events);
        let invalid = envelope(
            SessionEvent::HistoryItemRecorded {
                item: ContentItem::ToolResult {
                    result: outcome(&call("missing")).1,
                },
            },
            8,
        );
        let valid = envelope(
            SessionEvent::HistoryItemRecorded {
                item: ContentItem::user_text("must not commit"),
            },
            7,
        );
        let retained = projection.task_source_events().to_vec();
        assert!(
            retained
                .iter()
                .zip(projection.task_source_events())
                .all(|(copy, original)| std::sync::Arc::ptr_eq(copy, original)),
            "prepared sources share immutable payloads"
        );
        assert!(
            projection
                .ingest_durable_verified(vec![valid, invalid], &blobs)
                .is_err()
        );
        assert_source(&projection, &sink.events);
        assert_eq!(projection.task_source_events(), retained);
        let replay = SessionProjection::replay(sink.events.clone()).unwrap();
        assert_source(&replay, &sink.events);
        assert_eq!(projection.task_source_events(), replay.task_source_events());
    }

    #[test]
    fn retained_task_events_preserve_recorded_and_unknown_tool_recovery() {
        let mut events = vec![started()];
        for id in ["known", "unknown"] {
            events.push(envelope(
                SessionEvent::HistoryItemRecorded {
                    item: ContentItem::ToolCall { call: call(id) },
                },
                events.len() as u64 + 1,
            ));
        }
        let (receipt, result) = outcome(&call("known"));
        events.push(envelope(
            SessionEvent::ExecutionOutcomeRecorded { receipt, result },
            4,
        ));
        events.push(envelope(
            SessionEvent::Resumed {
                primary_provider_session: ProviderSessionRef {
                    id: ProviderSessionId::new("durable-resumed"),
                    role: ProviderSessionRole::Primary,
                    generation: 2,
                },
            },
            5,
        ));
        let projection = SessionProjection::replay(events.clone()).unwrap();
        assert_source(&projection, &events);
        let wanted = vec![events[1].event_id.clone(), events[2].event_id.clone()];
        for version in 0..=crate::domain::session::TASK_REQUEST_PROOF_VERSION {
            let full = projection
                .state()
                .task_resume_history(&wanted, events.iter(), version)
                .unwrap();
            let retained = projection
                .state()
                .task_resume_history(
                    &wanted,
                    projection.task_source_events().iter().map(AsRef::as_ref),
                    version,
                )
                .unwrap();
            assert_eq!(full.len(), 4);
            assert_eq!(retained.len(), full.len());
            for (full, retained) in full.iter().zip(&retained) {
                assert_eq!(full.item, retained.item);
                assert_eq!(full.source_refs, retained.source_refs);
                assert_eq!(full.semantics, retained.semantics);
                assert_eq!(full.vision_references, retained.vision_references);
            }
        }
        assert_eq!(projection.task_source_events().len(), 3);
    }

    #[test]
    fn reused_result_checkpoint_is_atomic_and_preserves_task_source_identity() {
        struct Sink {
            fail: bool,
            events: Vec<EventEnvelope<Value>>,
        }
        impl EventSink<Value> for Sink {
            type Error = &'static str;
            fn append_event(&mut self, event: &EventEnvelope<Value>) -> Result<(), Self::Error> {
                if self.fail {
                    return Err("injected reused-result append failure");
                }
                self.events.push(event.clone());
                Ok(())
            }
        }
        let leader = call("leader");
        let follower = call("follower");
        let (receipt, result) = outcome(&leader);
        let mut events = vec![started()];
        for call in [&leader, &follower] {
            events.push(envelope(
                SessionEvent::HistoryItemRecorded {
                    item: ContentItem::ToolCall { call: call.clone() },
                },
                events.len() as u64 + 1,
            ));
        }
        events.push(envelope(
            SessionEvent::ExecutionOutcomeRecorded {
                receipt,
                result: result.clone(),
            },
            4,
        ));
        let reused_result = result.for_deduplicated_call(&follower);
        let reused = envelope(
            SessionEvent::ExecutionResultReused {
                receipt_event_id: events[3].event_id.clone(),
                result: reused_result.clone(),
            },
            5,
        );
        let mut projection = SessionProjection::replay(events.clone()).unwrap();
        let mut sink = Sink { fail: true, events };
        assert!(projection.append(&mut sink, reused.clone()).is_err());
        assert_source(&projection, &sink.events);
        assert!(
            projection
                .state()
                .recorded_tool_result(&follower.call_id)
                .is_none()
        );
        sink.fail = false;
        projection.append(&mut sink, reused.clone()).unwrap();
        assert_source(&projection, &sink.events);
        assert_eq!(projection.state().execution_receipts().len(), 1);
        assert_eq!(
            projection.state().recorded_tool_result(&follower.call_id),
            Some(&reused_result)
        );
        assert_eq!(
            projection
                .state()
                .recorded_result_by_event(&reused.event_id),
            Some(&reused_result)
        );
        let wanted = vec![
            sink.events[1].event_id.clone(),
            sink.events[2].event_id.clone(),
        ];
        for version in 0..=crate::domain::session::TASK_REQUEST_PROOF_VERSION {
            let items = projection
                .state()
                .task_resume_history(
                    &wanted,
                    projection.task_source_events().iter().map(AsRef::as_ref),
                    version,
                )
                .unwrap();
            let recovered = items.iter().find(|item| matches!(
                &item.item, ContentItem::ToolResult { result } if result.call_id == follower.call_id
            )).unwrap();
            assert_eq!(
                recovered.item,
                ContentItem::ToolResult {
                    result: reused_result.clone()
                }
            );
            assert_eq!(
                recovered.source_refs,
                vec![wanted[1].clone(), reused.event_id.clone()]
            );
            assert_eq!(
                recovered.semantics.origin,
                crate::domain::context::ContextOrigin::ToolResult
            );
        }
        for change in ["arguments", "rerun", "result", "source"] {
            let mut invalid = sink.events.clone();
            match change {
                "arguments" => {
                    invalid[2].payload["item"]["call"]["arguments"]["path"] = json!("different.txt")
                }
                "rerun" => {
                    invalid[2].payload["item"]["call"]["arguments"]["execution"] =
                        json!({"rerun_reason":"run again"})
                }
                "result" => invalid[4].payload["result"]["output"] = json!({"fabricated":true}),
                "source" => invalid[4].payload["receipt_event_id"] = json!("missing"),
                _ => unreachable!(),
            }
            assert!(SessionReducer.reduce(invalid).is_err(), "{change}");
        }
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
        projection
            .ingest_durable_verified(
                suffix,
                &crate::adapters::artifact_store::LocalArtifactStore::for_project(&project),
            )
            .unwrap();
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
    fn cursor_rejects_truncation_and_wrong_session_identity() {
        for wrong_session in [false, true] {
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
            if !wrong_session {
                bytes.pop();
            }
            fs::write(&path, &bytes).unwrap();
            let mut writer = project.open_writer().unwrap();
            let id = if wrong_session {
                let other = StreamId::new("different-session");
                drop(writer.open_session_projection(other.clone(), None).unwrap());
                other
            } else {
                id
            };
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
        let root = TemporaryDirectory::new("invalid-durable-suffix");
        let project = Project::initialize(root.path()).unwrap();
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
        assert!(
            projection
                .ingest_durable_verified(
                    vec![good, bad],
                    &crate::adapters::artifact_store::LocalArtifactStore::for_project(&project)
                )
                .is_err()
        );
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
    fn open_writer_rejects_external_rewrite_before_its_next_append() {
        for replace in [false, true] {
            let root = TemporaryDirectory::new("open-writer-revision");
            let project = Project::initialize(root.path()).unwrap();
            let mut writer = project.open_writer().unwrap();
            let (mut store, _) = writer
                .open_session_projection(StreamId::new("durability"), None)
                .unwrap();
            store.append_event(&started()).unwrap();
            let path = root
                .path()
                .join(".echidna/sessions/durability/events.jsonl");
            let original = fs::read(&path).unwrap();
            let changed = String::from_utf8(original.clone())
                .unwrap()
                .replace("durable-primary", "changed-primary");
            assert_eq!(original.len(), changed.len());
            if replace {
                let replacement = path.with_extension("replacement");
                fs::write(&replacement, &changed).unwrap();
                fs::rename(replacement, &path).unwrap();
            } else {
                fs::write(&path, &changed).unwrap();
            }
            let next = envelope(
                SessionEvent::HistoryItemRecorded {
                    item: ContentItem::user_text("must not append"),
                },
                2,
            );
            assert!(matches!(
                store.append_event(&next),
                Err(ProjectEventStoreError::ValidatedPrefixChanged { .. })
            ));
            assert_eq!(fs::read(&path).unwrap(), changed.as_bytes());
        }
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
            projection.ingest_durable_verified(
                suffix,
                &crate::adapters::artifact_store::LocalArtifactStore::for_project(&project)
            ),
            Err(crate::domain::session::SessionError::UnknownEventType { .. })
        ));
        assert_eq!(projection.state(), &before);
    }
}
