//! Task-bound Core changes retire the old epoch only after durable selection.

use super::task_context::{
    ScriptedResponse::{InitialSelection, Response},
    initial_selection_response, selector_candidates, spawn_scripted_responses_server_with_wait,
};
use super::*;

#[derive(Clone, Copy, Debug)]
enum CoreMemoryTransition {
    Unchanged,
    Create,
    Replace,
    Empty,
    Delete,
}

#[test]
fn core_memory_transition_matrix_preserves_pairing_and_source_bound_epochs() {
    for (transition, initial, expected_current, replacement_items) in [
        (CoreMemoryTransition::Unchanged, None, None, 0),
        (
            CoreMemoryTransition::Create,
            None,
            Some("CORE_MEMORY_CREATED"),
            2,
        ),
        (
            CoreMemoryTransition::Replace,
            Some("CORE_MEMORY_BEFORE"),
            Some("CORE_MEMORY_REPLACED"),
            2,
        ),
        (
            CoreMemoryTransition::Empty,
            Some("CORE_MEMORY_BEFORE"),
            None,
            2,
        ),
        (
            CoreMemoryTransition::Delete,
            Some("CORE_MEMORY_BEFORE"),
            None,
            1,
        ),
    ] {
        let root = TemporaryDirectory::new("core-memory-continuation");
        let harness = TemporaryDirectory::new("core-memory-continuation-provider");
        initialize_project(root.path());
        fs::write(root.path().join("target.txt"), "core memory fixture\n").unwrap();
        let memory = root.path().join(".echidna/memory/core.md");
        if let Some(initial) = initial {
            fs::create_dir_all(memory.parent().unwrap()).unwrap();
            fs::write(&memory, initial).unwrap();
        }

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
        let config = write_context_window_config(harness.path(), &base_url, 1_048_576);
        fs::write(
            &config,
            fs::read_to_string(&config)
                .unwrap()
                .replace("incremental_input = true", "incremental_input = false"),
        )
        .unwrap();
        let (sender, requests) = mpsc::channel();
        let server_memory = memory.clone();
        let changed = replacement_items > 0;
        let request_count = 3;
        let server = thread::spawn(move || {
            for request_index in 0..request_count {
                let mut socket = f3_context::accept_before_deadline_with_wait(
                    &listener,
                    "Core Memory transition",
                    Duration::from_secs(60),
                );
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let raw = read_http_request(&mut socket);
                let wire = request_body(&raw);
                sender.send(raw).unwrap();
                if request_index == 0 {
                    f3_context::write_sse_response(
                        &mut socket,
                        &initial_selection_response(&wire, "core-initial-selection"),
                    );
                    continue;
                }
                let response = if request_index == 1 {
                    match transition {
                        CoreMemoryTransition::Unchanged => {}
                        CoreMemoryTransition::Create | CoreMemoryTransition::Replace => {
                            fs::create_dir_all(server_memory.parent().unwrap()).unwrap();
                            let content = match transition {
                                CoreMemoryTransition::Create => "CORE_MEMORY_CREATED",
                                CoreMemoryTransition::Replace => "CORE_MEMORY_REPLACED",
                                _ => unreachable!(),
                            };
                            fs::write(&server_memory, content).unwrap();
                        }
                        CoreMemoryTransition::Empty => fs::write(&server_memory, "").unwrap(),
                        CoreMemoryTransition::Delete => fs::remove_file(&server_memory).unwrap(),
                    }
                    let reasoning = json!({"type":"response.output_item.done","item":{
                        "type":"reasoning","content":[{"type":"reasoning_text","text":"CORE_MEMORY_CONTINUATION_REASONING"}]
                    }});
                    format!(
                        "data: {reasoning}\n\n{}",
                        m4_long_flow_tool_response(
                            "core-memory-read",
                            "read_file",
                            json!({"path":"target.txt"}),
                            "core-memory-first",
                        )
                    )
                } else {
                    text_response("core memory continuation complete", "core-memory-final")
                };
                f3_context::write_sse_response(&mut socket, &response);
            }
        });
        let server = LoopbackServer {
            base_url,
            requests,
            thread: server,
        };
        // These bounds cover fixture completion, not product latency.
        let output = wait_child_before_deadline(
            spawn_f2a_chat(root.path(), &config, "inspect target\n/exit\n", &[]),
            "Core Memory transition",
            Duration::from_secs(60),
        );
        assert!(
            output.status.success(),
            "{transition:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        selector_candidates(&f2a_request_body(
            &server.requests,
            "initial Core selection",
        ));
        let first = f2a_request_body(&server.requests, "Core Memory first request");
        let second = f2a_request_body(&server.requests, "Core Memory continuation request");
        assert!(server.requests.try_recv().is_err());
        f2a_join_server(server, "Core Memory transition");
        assert!(first.get("previous_response_id").is_none());
        assert!(second.get("previous_response_id").is_none());
        let second_input = second["input"].to_string();
        assert_eq!(
            second_input.contains("CORE_MEMORY_CONTINUATION_REASONING"),
            !changed
        );
        assert!(!second_input.contains("CORE_MEMORY_BEFORE"));
        assert!(json_contains_type(&second["input"], "function_call"));
        assert!(json_contains_type(&second["input"], "function_call_output"));
        assert!(!second_input.contains("Core Memory replacement for the current state."));
        if let Some(expected_current) = expected_current {
            assert!(
                second_input.contains(expected_current),
                "{transition:?}: {second_input}"
            );
        }

        let project = Project::discover(root.path()).unwrap();
        let state = reduced_session(&project, &session_id(root.path()));
        assert_eq!(state.request_audits().len(), request_count);
        assert_eq!(state.execution_receipt_count(), 1);
        assert_eq!(state.history().ensure_all_tool_calls_paired(), Ok(()));
        assert!(state.history().items().iter().all(|item| {
            !matches!(item, echidna::domain::model::ContentItem::Reasoning { .. })
        }));
        let final_audit = state.request_audits().last().unwrap();
        let selected_entries = final_audit
            .context_delivery_map()
            .unwrap()
            .content_items()
            .iter()
            .filter(|entry| {
                entry
                    .fragment_id()
                    .as_str()
                    .starts_with("memory-core-selected:")
            })
            .collect::<Vec<_>>();
        assert_eq!(
            !selected_entries.is_empty(),
            expected_current.is_some(),
            "{transition:?}"
        );
        for entry in selected_entries {
            assert_eq!(
                entry.provenance().origin(),
                echidna::domain::context::ContextOrigin::ProjectMemory
            );
            assert_eq!(
                entry.provenance().authority_level(),
                echidna::domain::context::AuthorityLevel::DerivedNonEvidence
            );
        }
        let events = session_events(root.path(), &session_id(root.path()));
        let mains = events
            .iter()
            .filter(|event| event["event_type"] == "session.model_delivery_committed")
            .collect::<Vec<_>>();
        assert_eq!(mains.len(), 2);
        assert_eq!(
            mains[0]["payload"]["provider_session"] == mains[1]["payload"]["provider_session"],
            !changed
        );
        if changed {
            let selection = events
                .iter()
                .rfind(|event| event["event_type"] == "session.task_context_selected")
                .unwrap();
            let capture: Value = serde_json::from_slice(
                &fs::read(
                    root.path()
                        .join(".echidna/artifacts/blobs/sha256")
                        .join(selection["payload"]["capture_blob"].as_str().unwrap()),
                )
                .unwrap(),
            )
            .unwrap();
            let captured_core = capture["common"]["fragments"]
                .as_array()
                .unwrap()
                .iter()
                .find(|fragment| fragment["source"] == "project_memory_core");
            assert_eq!(
                captured_core.is_some(),
                !matches!(transition, CoreMemoryTransition::Delete)
            );
            if matches!(transition, CoreMemoryTransition::Empty) {
                assert_eq!(fs::read_to_string(&memory).unwrap(), "");
                assert!(
                    !captured_core
                        .unwrap()
                        .to_string()
                        .contains("CORE_MEMORY_BEFORE")
                );
            }
            let result = events
                .iter()
                .find(|event| {
                    event["event_type"] == "session.history_item_recorded"
                        && event["payload"]["item"]["kind"] == "tool_result"
                })
                .unwrap();
            assert!(result["sequence"].as_u64().unwrap() < selection["sequence"].as_u64().unwrap());
            assert!(
                selection["sequence"].as_u64().unwrap() < mains[1]["sequence"].as_u64().unwrap()
            );
        }
        assert_audits_reconstruct(root.path(), &state);
    }
}

#[test]
fn evidence_curation_memory_write_creates_auditable_task_context_epoch() {
    let root = TemporaryDirectory::new("core-memory-write-continuation");
    let harness = TemporaryDirectory::new("core-memory-write-provider");
    initialize_project(root.path());
    let responses = vec![
        InitialSelection,
        Response(200, {
            let reasoning = json!({"type":"response.output_item.done","item":{
                "type":"reasoning","content":[{"type":"reasoning_text","text":"CORE_MEMORY_WRITE_REASONING"}]
            }});
            format!(
                "data: {reasoning}\n\n{}",
                m4_long_flow_tool_response(
                    "core-memory-write",
                    "memory_write",
                    json!({
                        "path":"core.md",
                        "content":"CORE_MEMORY_WRITTEN_CURRENT",
                        "expected_hash":null,
                        "event_id":"core-memory-write-continuation"
                    }),
                    "core-memory-write-first",
                )
            )
        }),
        Response(
            200,
            text_response(
                "memory write continuation complete",
                "core-memory-write-final",
            ),
        ),
    ];
    let server = spawn_scripted_responses_server_with_wait(responses, Duration::from_secs(60));
    let config = write_context_window_config(harness.path(), &server.base_url, 1_048_576);
    fs::write(
        &config,
        fs::read_to_string(&config)
            .unwrap()
            .replace("incremental_input = true", "incremental_input = false"),
    )
    .unwrap();
    // These bounds cover fixture completion, not product latency.
    let output = wait_child_before_deadline(
        spawn_f2a_chat(
            root.path(),
            &config,
            "/phase evidence-curation\ncurate Core Memory\n/exit\n",
            &[],
        ),
        "Core Memory write continuation",
        Duration::from_secs(60),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    selector_candidates(&f2a_request_body(
        &server.requests,
        "initial Core selection",
    ));
    let first = f2a_request_body(&server.requests, "Core Memory write first request");
    let second = f2a_request_body(&server.requests, "Core Memory write continuation request");
    assert!(server.requests.try_recv().is_err());
    server.join();
    assert!(
        first["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["name"] == "memory_write")
    );
    assert!(second.get("previous_response_id").is_none());
    let second_input = second["input"].to_string();
    assert!(!second_input.contains("CORE_MEMORY_WRITE_REASONING"));
    assert!(second_input.contains("CORE_MEMORY_WRITTEN_CURRENT"));
    assert!(json_contains_type(&second["input"], "function_call"));
    assert!(json_contains_type(&second["input"], "function_call_output"));
    assert!(!second_input.contains("Core Memory replacement for the current state."));
    assert_eq!(
        fs::read_to_string(root.path().join(".echidna/memory/core.md")).unwrap(),
        "CORE_MEMORY_WRITTEN_CURRENT"
    );
    assert!(
        fs::read_to_string(root.path().join(".echidna/research.jsonl"))
            .unwrap()
            .contains("memory.changed")
    );

    let project = Project::discover(root.path()).unwrap();
    let state = reduced_session(&project, &session_id(root.path()));
    assert_eq!(state.request_audits().len(), 3);
    assert_eq!(state.execution_receipt_count(), 1);
    assert_eq!(state.history().ensure_all_tool_calls_paired(), Ok(()));
    let selected_entries: Vec<_> = state.request_audits()[2]
        .context_delivery_map()
        .unwrap()
        .content_items()
        .iter()
        .filter(|entry| {
            entry
                .fragment_id()
                .as_str()
                .starts_with("memory-core-selected:")
        })
        .collect();
    assert_eq!(selected_entries.len(), 1);
    for entry in selected_entries {
        assert_eq!(
            entry.provenance().origin(),
            echidna::domain::context::ContextOrigin::ProjectMemory
        );
        assert_eq!(
            entry.provenance().authority_level(),
            echidna::domain::context::AuthorityLevel::DerivedNonEvidence
        );
    }
    assert_audits_reconstruct(root.path(), &state);
}

#[test]
fn oversized_selected_core_stops_before_new_epoch_adoption_and_preserves_the_tool_result() {
    // Legacy no-task retention failures remain in private opaque/Core and
    // tool_loop_integration tests. Task mode must not adopt an inadmissible
    // required Core selection merely to retire the old accepted reasoning.
    let root = TemporaryDirectory::new("core-memory-selection-capacity");
    let harness = TemporaryDirectory::new("core-memory-selection-provider");
    initialize_project(root.path());
    fs::write(
        root.path().join("target.txt"),
        "bounded core memory fixture\n",
    )
    .unwrap();
    let memory = root.path().join(".echidna/memory/core.md");
    let server = super::agent_evolution::server_with_wait(
        move |step, request| match step {
            0 => initial_selection_response(request, "oversized-core-initial-selection"),
            1 => {
                fs::create_dir_all(memory.parent().unwrap()).unwrap();
                fs::write(&memory, "M".repeat(64 * 1024)).unwrap();
                let reasoning = json!({"type":"response.output_item.done","item":{
                    "type":"reasoning","content":[{"type":"reasoning_text","text":"CORE_MEMORY_REJECTION_REASONING"}]
                }});
                format!(
                    "data: {reasoning}\n\n{}",
                    m4_long_flow_tool_response(
                        "core-memory-rejection-read",
                        "read_file",
                        json!({"path":"target.txt"}),
                        "core-memory-rejection-first"
                    )
                )
            }
            _ => unreachable!(),
        },
        2,
        Duration::from_secs(25),
    );
    let config = write_context_window_config(harness.path(), &server.base_url, 32_768);
    let output = run_chat(root.path(), &config, "inspect target\n/exit\n", &[]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "{stderr}");
    assert!(stderr.to_lowercase().contains("context"), "{stderr}");
    selector_candidates(&f2a_request_body(
        &server.requests,
        "oversized Core initial selection",
    ));
    f2a_request_body(&server.requests, "Tool before oversized Core refresh");
    assert!(server.requests.try_recv().is_err());
    server.join();
    let state = reduced_session(
        &Project::discover(root.path()).unwrap(),
        &session_id(root.path()),
    );
    assert_eq!(state.request_audits().len(), 2);
    assert_eq!(state.execution_receipt_count(), 1);
    assert_eq!(state.history().ensure_all_tool_calls_paired(), Ok(()));
    let events = session_events(root.path(), &session_id(root.path()));
    assert_eq!(
        events
            .iter()
            .filter(|event| event["event_type"] == "session.task_context_selected")
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event["event_type"] == "session.model_delivery_committed")
            .count(),
        1
    );
    assert_audits_reconstruct(root.path(), &state);
}
