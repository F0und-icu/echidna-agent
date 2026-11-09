use super::task_context::{
    ScriptedResponse::{InitialSelection, Response},
    selector_candidates, spawn_scripted_responses_server,
};
use super::*;
use echidna::domain::context::{ProviderSessionId, ProviderSessionRef, ProviderSessionRole};
use echidna::domain::model::ContentItem;
use echidna::domain::tools::{ToolCall, ToolCallId, ToolName, ToolResult, ToolResultStatus};

#[test]
fn ctf_r0_clean_binary_paste_matches_durable_history_audit_and_actual_request() {
    let root = TemporaryDirectory::new("ctf-r0-clean-project");
    let harness = TemporaryDirectory::new("ctf-r0-external-harness");
    fs::write(root.path().join("target.txt"), "synthetic target only").unwrap();
    initialize_project(root.path());
    assert!(session_ids(root.path()).is_empty());
    let server = spawn_scripted_responses_server(vec![
        InitialSelection,
        Response(200, text_response("recorded", "ctf-r0")),
    ]);
    let config = write_config(harness.path(), &server.base_url);
    let task = "Inspect target.txt in the declared host environment.\n/phase verification\nDo not read WP, EXP, solutions or prior sessions.";
    let child = spawn_f2a_chat(
        root.path(),
        &config,
        &format!("/paste\n{task}\n/end\n/exit\n"),
        &["--domain", "binary"],
    );
    let output = f2a_wait_child(child, "CTF clean paste");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let selection = f2a_request_body(&server.requests, "CTF clean paste selection");
    assert_eq!(selector_candidates(&selection)["input"]["text"], task);
    let request = f2a_request_body(&server.requests, "CTF clean paste");
    f2a_join_server(server, "CTF clean paste");
    let users: Vec<_> = request["input"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["role"] == "user")
        .collect();
    assert_eq!(users.len(), 1);
    assert!(json_contains_text(users[0], task));
    let session = session_id(root.path());
    let project = Project::discover(root.path()).unwrap();
    let state = reduced_session(&project, &session);
    assert_eq!(state.history().items()[0], ContentItem::user_text(task));
    assert_eq!(state.history().items().len(), 2);
    assert_eq!(state.request_audits().len(), 2);
    let audit = state.request_audits()[1]
        .reconstruct(|blob| {
            fs::read(
                root.path()
                    .join(".echidna/artifacts/blobs/sha256")
                    .join(blob.as_str()),
            )
        })
        .unwrap();
    // The audited IR also contains derived Project/Research context; it is
    // not a second Conversation History or another pasted user Turn.
    let task_item = ContentItem::user_text(task);
    assert_eq!(audit.content_items.last(), Some(&task_item));
    assert_eq!(
        audit
            .content_items
            .iter()
            .filter(|item| **item == task_item)
            .count(),
        1
    );
    let events = session_events(root.path(), &session);
    assert!(events.iter().any(
        |event| event["event_type"] == "session.domain_profile_selected"
            && event["payload"]["selection"]["profile"] == "binary"
    ));
    assert!(
        events
            .iter()
            .filter(|event| event["event_type"] == "session.research_phase_selected")
            .all(|event| event["payload"]["selection"]["phase"] == "mapping")
    );
    fs::write(harness.path().join("stdout.txt"), output.stdout).unwrap();
    fs::write(harness.path().join("stderr.txt"), output.stderr).unwrap();
    let target_entries: BTreeSet<_> = fs::read_dir(root.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        target_entries,
        BTreeSet::from([".echidna".to_owned(), "target.txt".to_owned()])
    );
}

fn seed_interrupted_session(project: &Project, late_real_result: bool) -> StreamId {
    let session = StreamId::new("ctf-r1-interrupted");
    let stream = ProjectEventStream::Session(session.clone()).stream_id();
    let mut writer = project.open_writer().unwrap();
    let mut store = writer.open_session(session.clone(), None).unwrap();
    let call = ToolCall::new(
        ToolCallId::new("ctf-r1-open"),
        ToolName::new("read_file"),
        json!({"path":"must-not-run.txt"}),
    );
    let mut events = vec![
        SessionEvent::Started {
            primary_provider_session: ProviderSessionRef {
                id: ProviderSessionId::new("ctf-r1-old-primary"),
                role: ProviderSessionRole::Primary,
                generation: 1,
            },
        },
        SessionEvent::HistoryItemRecorded {
            item: ContentItem::user_text("initial task"),
        },
        SessionEvent::HistoryItemRecorded {
            item: ContentItem::ToolCall { call: call.clone() },
        },
        SessionEvent::HistoryItemRecorded {
            item: ContentItem::user_text("earlier failed resume"),
        },
    ];
    if late_real_result {
        events.push(SessionEvent::HistoryItemRecorded {
            item: ContentItem::ToolResult {
                result: ToolResult {
                    call_id: call.call_id,
                    tool_name: call.name,
                    status: ToolResultStatus::Succeeded,
                    output: json!({"text":"late real result"}),
                },
            },
        });
    }
    for (index, event) in events.into_iter().enumerate() {
        let envelope = event
            .into_envelope(
                EventId::new(format!("ctf-r1-seed-{index}")),
                stream.clone(),
                store.next_sequence().unwrap(),
                OccurredAt::new("2026-09-12T00:00:00Z"),
            )
            .unwrap();
        EventSink::append_event(&mut store, &envelope).unwrap();
    }
    session
}

#[test]
fn ctf_r1_repeated_full_resume_repairs_wire_without_fabricating_durable_results() {
    let root = TemporaryDirectory::new("ctf-r1-resume");
    let harness = TemporaryDirectory::new("ctf-r1-harness");
    initialize_project(root.path());
    let project = Project::discover(root.path()).unwrap();
    let session = seed_interrupted_session(&project, false);
    let event_path = root
        .path()
        .join(".echidna/sessions")
        .join(session.as_str())
        .join("events.jsonl");
    let mut prefix = fs::read(&event_path).unwrap();
    use super::task_context::{
        ScriptedResponse::{ContinueSelection, InitialSelection, Response},
        selector_candidates, spawn_scripted_responses_server,
    };
    let server = spawn_scripted_responses_server(vec![
        InitialSelection,
        Response(200, text_response("first recovery", "ctf-r1-first")),
        ContinueSelection { usage: None },
        Response(200, text_response("second recovery", "ctf-r1-second")),
    ]);
    let config = write_config(harness.path(), &server.base_url);
    fs::write(
        &config,
        fs::read_to_string(&config)
            .unwrap()
            .replace("incremental_input = true", "incremental_input = false"),
    )
    .unwrap();
    for turn in ["continue once", "continue twice"] {
        let child = spawn_f2a_chat(
            root.path(),
            &config,
            &format!("{turn}\n/exit\n"),
            &["--session", session.as_str()],
        );
        let output = f2a_wait_child(child, "CTF Full Resume");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        selector_candidates(&f2a_request_body(
            &server.requests,
            "CTF Full Resume source selection",
        ));
        let request = f2a_request_body(&server.requests, "CTF Full Resume");
        assert!(request.get("previous_response_id").is_none());
        let items = request["input"].as_array().unwrap();
        let call = items
            .iter()
            .position(|item| item["type"] == "function_call" && item["call_id"] == "ctf-r1-open")
            .unwrap();
        assert!(
            items[..call]
                .iter()
                .any(|item| json_contains_text(item, "initial task"))
        );
        assert_eq!(items[call + 1]["type"], "function_call_output");
        assert_eq!(items[call + 1]["call_id"], "ctf-r1-open");
        assert!(
            items[call + 1]["output"]
                .as_str()
                .unwrap()
                .contains("unknown")
        );
        assert!(json_contains_text(
            &items[call + 2],
            "earlier failed resume"
        ));
        assert!(json_contains_text(items.last().unwrap(), turn));
        assert_eq!(
            items
                .iter()
                .filter(|item| item["type"] == "function_call_output")
                .count(),
            1
        );
        let current = fs::read(&event_path).unwrap();
        assert!(
            current.starts_with(&prefix),
            "old durable bytes must remain unchanged"
        );
        prefix = current;
        let state = reduced_session(&project, session.as_str());
        assert_eq!(state.execution_receipt_count(), 0);
        assert!(
            !state
                .history()
                .items()
                .iter()
                .any(|item| matches!(item, ContentItem::ToolResult { .. }))
        );
        assert_audits_reconstruct(root.path(), &state);
    }
    f2a_join_server(server, "CTF repeated Resume");
}

#[test]
fn resume_uses_atomic_recorded_result_without_replaying_or_inventing_history() {
    for complete_outcome in [true, false] {
        let root = TemporaryDirectory::new("recorded-outcome-resume");
        let harness = TemporaryDirectory::new("recorded-outcome-harness");
        initialize_project(root.path());
        let project = Project::discover(root.path()).unwrap();
        let id = StreamId::new("known-outcome");
        let stream = ProjectEventStream::Session(id.clone()).stream_id();
        let call = ToolCall::new(
            ToolCallId::new("known-call"),
            ToolName::new("read_file"),
            json!({"path":"must-not-run.txt"}),
        );
        let receipt = serde_json::from_value(json!({
            "execution_id":"execution:known-call", "plan_fingerprint":"actual-plan",
            "status":"succeeded", "exit_reason":null, "output_artifact_ids":[], "environment_change":null
        })).unwrap();
        let result = ToolResult {
            call_id: call.call_id.clone(),
            tool_name: call.name.clone(),
            status: ToolResultStatus::Succeeded,
            output: json!({"text":"OBSERVED_VALUE"}),
        };
        let events = vec![
            SessionEvent::Started {
                primary_provider_session: ProviderSessionRef {
                    id: ProviderSessionId::new("old-known-primary"),
                    role: ProviderSessionRole::Primary,
                    generation: 1,
                },
            },
            SessionEvent::HistoryItemRecorded {
                item: ContentItem::user_text("original task"),
            },
            SessionEvent::HistoryItemRecorded {
                item: ContentItem::ToolCall { call },
            },
            if complete_outcome {
                SessionEvent::ExecutionOutcomeRecorded { receipt, result }
            } else {
                SessionEvent::ExecutionReceiptRecorded { receipt }
            },
        ];
        {
            let mut writer = project.open_writer().unwrap();
            let mut store = writer.open_session(id.clone(), None).unwrap();
            for (index, event) in events.into_iter().enumerate() {
                let event = event
                    .into_envelope(
                        EventId::new(format!("known-outcome-{index}")),
                        stream.clone(),
                        store.next_sequence().unwrap(),
                        OccurredAt::new("2026-10-25T00:00:00Z"),
                    )
                    .unwrap();
                EventSink::append_event(&mut store, &event).unwrap();
            }
        }
        let path = root
            .path()
            .join(".echidna/sessions/known-outcome/events.jsonl");
        let prefix = fs::read(&path).unwrap();
        if complete_outcome {
            use super::task_context::{
                ScriptedResponse::{InitialSelection, Response},
                selector_candidates, spawn_scripted_responses_server,
            };
            let server = spawn_scripted_responses_server(vec![
                InitialSelection,
                Response(200, text_response("reported known value", "known-resume")),
            ]);
            let config = write_config(harness.path(), &server.base_url);
            let output = f2a_wait_child(
                spawn_f2a_chat(
                    root.path(),
                    &config,
                    "report previous result\n/exit\n",
                    &["--session", id.as_str()],
                ),
                "atomic outcome Resume",
            );
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            selector_candidates(&f2a_request_body(
                &server.requests,
                "known durable source selection",
            ));
            let request = f2a_request_body(&server.requests, "known durable result");
            let outputs = request["input"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|item| item["type"] == "function_call_output")
                .collect::<Vec<_>>();
            assert_eq!(outputs.len(), 1);
            assert_eq!(outputs[0]["call_id"], "known-call");
            assert_eq!(
                serde_json::from_str::<Value>(outputs[0]["output"].as_str().unwrap()).unwrap(),
                json!({"status":"succeeded","output":{"text":"OBSERVED_VALUE"}})
            );
            f2a_join_server(server, "atomic outcome Resume");
            assert!(fs::read(&path).unwrap().starts_with(&prefix));
            let state = reduced_session(&project, id.as_str());
            assert_eq!(state.execution_receipt_count(), 1);
            assert!(
                !state
                    .history()
                    .items()
                    .iter()
                    .any(|item| matches!(item, ContentItem::ToolResult { .. })),
                "normalization must not append invented History facts"
            );
            assert_audits_reconstruct(root.path(), &state);
        } else {
            let config = write_config(harness.path(), "http://127.0.0.1:1");
            let output = f2a_wait_child(
                spawn_f2a_chat(root.path(), &config, "/exit\n", &["--session", id.as_str()]),
                "bare Receipt rejection",
            );
            assert!(!output.status.success());
            assert!(String::from_utf8_lossy(&output.stderr).contains("durable ExecutionReceipt"));
            assert_eq!(fs::read(&path).unwrap(), prefix);
        }
    }
}

#[test]
fn ctf_r1_late_real_result_is_rejected_before_provider_request_or_history_rewrite() {
    let root = TemporaryDirectory::new("ctf-r1-late-result");
    let harness = TemporaryDirectory::new("ctf-r1-late-harness");
    initialize_project(root.path());
    let project = Project::discover(root.path()).unwrap();
    let session = seed_interrupted_session(&project, true);
    let before = reduced_session(&project, session.as_str())
        .history()
        .items()
        .to_vec();
    let config = write_config(harness.path(), "http://127.0.0.1:1/v1");
    let child = spawn_f2a_chat(
        root.path(),
        &config,
        "continue\n",
        &["--session", session.as_str()],
    );
    let output = f2a_wait_child(child, "CTF late result rejection");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("crosses a later User message"));
    let after = reduced_session(&project, session.as_str());
    assert_eq!(after.history().items(), before);
    assert_eq!(after.execution_receipt_count(), 0);
    assert!(after.request_audits().is_empty());
}
