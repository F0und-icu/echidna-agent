//! Settled Tool batches compact without replaying effects or losing input.
use super::*;

#[test]
fn dedupe_results_survive_interruption_before_history_for_same_batch_and_later_turn() {
    for later_turn in [false, true] {
        let root = TemporaryDirectory::new(if later_turn {
            "dedupe-later-turn-recovery"
        } else {
            "dedupe-same-batch-recovery"
        });
        initialize_project(root.path());
        let arguments = json!({"argv": ["/bin/sh", "-c", "printf x >> execution-marker"]});
        let calls = if later_turn {
            vec![
                (
                    200,
                    tool_calls_response(
                        &[("leader", "start_job", arguments.clone())],
                        "dedupe-leader",
                    ),
                ),
                (
                    200,
                    text_response("First turn completed.", "dedupe-first-answer"),
                ),
                (
                    200,
                    tool_calls_response(
                        &[("follower", "start_job", arguments.clone())],
                        "dedupe-follower",
                    ),
                ),
                (
                    200,
                    text_response("Second turn completed.", "dedupe-second-answer"),
                ),
            ]
        } else {
            vec![
                (
                    200,
                    tool_calls_response(
                        &[
                            ("leader", "start_job", arguments.clone()),
                            ("follower", "start_job", arguments.clone()),
                        ],
                        "dedupe-batch",
                    ),
                ),
                (
                    200,
                    text_response("Batch completed.", "dedupe-batch-answer"),
                ),
            ]
        };
        let expected_requests = calls.len();
        let mut steps = vec![task_context::ScriptedResponse::InitialSelection];
        for (index, (status, body)) in calls.into_iter().enumerate() {
            if later_turn && index == 2 {
                steps.push(task_context::ScriptedResponse::ContinueSelection { usage: None });
            }
            steps.push(task_context::ScriptedResponse::Response(status, body));
        }
        let expected_requests = expected_requests + if later_turn { 2 } else { 1 };
        let server = task_context::spawn_scripted_responses_server(steps);
        let config = write_config(root.path(), &server.base_url);
        let input = if later_turn {
            "/phase experiment\nLaunch first.\nLaunch the same job.\n/exit\n"
        } else {
            "/phase experiment\nLaunch the same job twice.\n/exit\n"
        };
        let output = run_chat(root.path(), &config, input, &[]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        for _ in 0..expected_requests {
            server.requests.recv_timeout(F2A_WAIT).unwrap();
        }
        server.join();
        let marker = root.path().join("execution-marker");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !marker.exists() && std::time::Instant::now() < deadline {
            thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(fs::read_to_string(&marker).unwrap(), "x");
        let id = session_id(root.path());
        let events = session_events(root.path(), &id);
        let receipt = events
            .iter()
            .find(|event| event["event_type"] == "tool.execution_outcome_recorded")
            .unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| event["event_type"] == "tool.execution_outcome_recorded")
                .count(),
            1
        );
        let reused_index = events
            .iter()
            .position(|event| event["event_type"] == "tool.execution_result_reused")
            .unwrap();
        let reused = &events[reused_index];
        assert_eq!(reused["payload"]["receipt_event_id"], receipt["event_id"]);
        assert_eq!(reused["payload"]["result"]["call_id"], "follower");
        let cut = &events[..=reused_index];
        assert!(!cut.iter().any(
            |event| event["event_type"] == "session.history_item_recorded"
                && event["payload"]["item"]["result"]["call_id"] == "follower"
        ));
        let path = root
            .path()
            .join(".echidna/sessions")
            .join(&id)
            .join("events.jsonl");
        fs::write(
            &path,
            cut.iter()
                .map(|event| format!("{}\n", serde_json::to_string(event).unwrap()))
                .collect::<String>(),
        )
        .unwrap();
        let resumed_server = task_context::spawn_scripted_responses_server(vec![
            task_context::ScriptedResponse::ContinueSelection { usage: None },
            task_context::ScriptedResponse::Response(
                200,
                text_response(
                    &"Recovered without replay. ".repeat(2_000),
                    "dedupe-resumed",
                ),
            ),
            task_context::ScriptedResponse::Response(
                200,
                text_response(
                    &json!({"concise_progress":"The original job completed once.","unresolved_questions":[],"next_step_notes":[]}).to_string(),
                    "dedupe-recovered-handoff",
                ),
            ),
        ]);
        let resumed_config = write_config(root.path(), &resumed_server.base_url);
        let resumed = run_chat(
            root.path(),
            &resumed_config,
            "Continue from the observed results.\n/compact\n/exit\n",
            &["--resume", &id],
        );
        assert!(
            resumed.status.success(),
            "{}",
            String::from_utf8_lossy(&resumed.stderr)
        );
        task_context::selector_candidates(&request_body(
            &resumed_server.requests.recv_timeout(F2A_WAIT).unwrap(),
        ));
        let request = request_body(&resumed_server.requests.recv_timeout(F2A_WAIT).unwrap());
        let handoff = request_body(&resumed_server.requests.recv_timeout(F2A_WAIT).unwrap());
        assert!(
            handoff
                .get("tools")
                .and_then(Value::as_array)
                .is_none_or(Vec::is_empty)
        );
        resumed_server.join();
        let input = request["input"].as_array().unwrap();
        for call_id in ["leader", "follower"] {
            assert_eq!(
                input
                    .iter()
                    .filter(
                        |item| item["type"] == "function_call_output" && item["call_id"] == call_id
                    )
                    .count(),
                1,
                "{request}"
            );
        }
        assert_eq!(fs::read_to_string(&marker).unwrap(), "x");
        let after = session_events(root.path(), &id);
        assert_eq!(
            after
                .iter()
                .filter(|event| event["event_type"] == "tool.execution_outcome_recorded")
                .count(),
            1
        );
        assert_eq!(
            after
                .iter()
                .filter(|event| event["event_type"] == "tool.execution_result_reused")
                .count(),
            1
        );
        assert!(String::from_utf8_lossy(&resumed.stdout).contains("Compaction revision 1 applied"));
        let summary = &after
            .iter()
            .find(|event| event["event_type"] == "session.compaction_created")
            .unwrap()["payload"]["summary"];
        let plan_blob = summary["source_plan_blob_id"].as_str().unwrap();
        let plan: Value = serde_json::from_slice(
            &fs::read(
                root.path()
                    .join(".echidna/artifacts/blobs/sha256")
                    .join(plan_blob),
            )
            .unwrap(),
        )
        .unwrap();
        let entry = plan["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| {
                entry["source"]["kind"] == "recorded_outcome"
                    && entry["source"]["id"] == reused["event_id"]
            })
            .unwrap();
        assert_eq!(entry["tool"]["call_id"], "follower");
        assert!(
            entry["tool"]["receipt_event_refs"]
                .as_array()
                .unwrap()
                .contains(&receipt["event_id"])
        );

        // A different output or an unrelated Receipt is not the observed reuse.
        for wrong_source in [false, true] {
            let mut tampered = cut.to_vec();
            if wrong_source {
                tampered[reused_index]["payload"]["receipt_event_id"] = "missing-receipt".into();
            } else {
                tampered[reused_index]["payload"]["result"]["output"] = json!({"fabricated": true});
            }
            let decoded = tampered
                .into_iter()
                .map(|event| serde_json::from_value::<EventEnvelope<Value>>(event).unwrap())
                .collect::<Vec<_>>();
            assert!(SessionReducer.reduce(decoded).is_err());
        }
        let mut conflict = cut
            .iter()
            .cloned()
            .map(|event| serde_json::from_value::<EventEnvelope<Value>>(event).unwrap())
            .collect::<Vec<_>>();
        let mut separate_receipt: echidna::domain::tools::ExecutionReceipt =
            serde_json::from_value(receipt["payload"]["receipt"].clone()).unwrap();
        separate_receipt.execution_id =
            echidna::domain::tools::ExecutionId::new("execution:follower");
        let follower_event = &conflict[reused_index];
        let next_sequence = follower_event.sequence.get() + 1;
        let separate = SessionEvent::ExecutionReceiptRecorded {
            receipt: separate_receipt,
        }
        .into_envelope(
            EventId::new("conflicting-follower-receipt"),
            follower_event.stream_id.clone(),
            follower_event.sequence,
            follower_event.occurred_at.clone(),
        )
        .unwrap();
        conflict[reused_index].sequence =
            echidna::domain::events::EventSequence::new(next_sequence);
        conflict.insert(reused_index, separate);
        assert!(SessionReducer.reduce(conflict).is_err());
    }
}
