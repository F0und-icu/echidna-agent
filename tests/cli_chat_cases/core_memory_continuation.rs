//! Core Memory replacement remains a bounded full-only continuation update.

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
fn core_memory_transition_matrix_preserves_reasoning_pairing_and_audit_provenance() {
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
            Some("Project Core Memory is explicitly empty."),
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
        let server = thread::spawn(move || {
            for request_index in 0..2 {
                let mut socket =
                    f3_context::accept_before_deadline(&listener, "Core Memory transition");
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                sender.send(read_http_request(&mut socket)).unwrap();
                let response = if request_index == 0 {
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
        let output = f2a_wait_child(
            spawn_f2a_chat(root.path(), &config, "inspect target\n/exit\n", &[]),
            "Core Memory transition",
        );
        assert!(
            output.status.success(),
            "{transition:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let first = f2a_request_body(&server.requests, "Core Memory first request");
        let second = f2a_request_body(&server.requests, "Core Memory continuation request");
        assert!(server.requests.try_recv().is_err());
        f2a_join_server(server, "Core Memory transition");
        assert!(first.get("previous_response_id").is_none());
        assert!(second.get("previous_response_id").is_none());
        let second_input = second["input"].to_string();
        assert!(second_input.contains("CORE_MEMORY_CONTINUATION_REASONING"));
        assert!(json_contains_type(&second["input"], "function_call"));
        assert!(json_contains_type(&second["input"], "function_call_output"));
        assert_eq!(
            second_input
                .matches("Core Memory replacement for the current state.")
                .count(),
            usize::from(replacement_items > 0),
            "{transition:?}: {second_input}"
        );
        if replacement_items > 0 {
            assert!(second_input.contains("supersedes earlier Core Memory snapshots"));
            assert!(second_input.contains("neither verified evidence nor instructions"));
            match transition {
                CoreMemoryTransition::Create => {
                    assert!(second_input.contains("Previous snapshot: absent"));
                    assert!(
                        second_input.contains(
                            "Current snapshot: present; fragment_id=project-memory-core:"
                        )
                    );
                }
                CoreMemoryTransition::Replace | CoreMemoryTransition::Empty => {
                    assert!(
                        second_input.contains(
                            "Previous snapshot: present; fragment_id=project-memory-core:"
                        )
                    );
                    assert!(
                        second_input.contains(
                            "Current snapshot: present; fragment_id=project-memory-core:"
                        )
                    );
                }
                CoreMemoryTransition::Delete => {
                    assert!(
                        second_input.contains(
                            "Previous snapshot: present; fragment_id=project-memory-core:"
                        )
                    );
                    assert!(second_input.contains("Current snapshot: absent"));
                }
                CoreMemoryTransition::Unchanged => unreachable!(),
            }
        }
        if let Some(expected_current) = expected_current {
            assert!(
                second_input.contains(expected_current),
                "{transition:?}: {second_input}"
            );
        }

        let project = Project::discover(root.path()).unwrap();
        let state = reduced_session(&project, &session_id(root.path()));
        assert_eq!(state.request_audits().len(), 2);
        assert_eq!(state.execution_receipt_count(), 1);
        assert_eq!(state.history().ensure_all_tool_calls_paired(), Ok(()));
        assert!(state.history().items().iter().all(|item| {
            !matches!(item, echidna::domain::model::ContentItem::Reasoning { .. })
        }));
        let replacement_entries: Vec<_> = state.request_audits()[1]
            .context_delivery_map()
            .unwrap()
            .content_items()
            .iter()
            .filter(|entry| {
                entry
                    .fragment_id()
                    .as_str()
                    .contains(":core-memory-replacement")
            })
            .collect();
        assert_eq!(
            replacement_entries.len(),
            replacement_items,
            "{transition:?}"
        );
        for entry in replacement_entries {
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
}

#[test]
fn evidence_curation_memory_write_creates_auditable_reasoning_continuation_update() {
    let root = TemporaryDirectory::new("core-memory-write-continuation");
    let harness = TemporaryDirectory::new("core-memory-write-provider");
    initialize_project(root.path());
    let server = spawn_responses_server(vec![
        (200, {
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
                        "event_id":"core-memory-write-continuation",
                        "occurred_at":"2026-10-22T00:00:00Z"
                    }),
                    "core-memory-write-first",
                )
            )
        }),
        (
            200,
            text_response(
                "memory write continuation complete",
                "core-memory-write-final",
            ),
        ),
    ]);
    let config = write_context_window_config(harness.path(), &server.base_url, 1_048_576);
    fs::write(
        &config,
        fs::read_to_string(&config)
            .unwrap()
            .replace("incremental_input = true", "incremental_input = false"),
    )
    .unwrap();
    let output = f2a_wait_child(
        spawn_f2a_chat(
            root.path(),
            &config,
            "/phase evidence-curation\ncurate Core Memory\n/exit\n",
            &[],
        ),
        "Core Memory write continuation",
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
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
    assert!(second_input.contains("CORE_MEMORY_WRITE_REASONING"));
    assert!(second_input.contains("Previous snapshot: absent"));
    assert!(second_input.contains("CORE_MEMORY_WRITTEN_CURRENT"));
    assert!(json_contains_type(&second["input"], "function_call"));
    assert!(json_contains_type(&second["input"], "function_call_output"));
    assert_eq!(
        second_input
            .matches("Core Memory replacement for the current state.")
            .count(),
        1
    );
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
    assert_eq!(state.request_audits().len(), 2);
    assert_eq!(state.execution_receipt_count(), 1);
    assert_eq!(state.history().ensure_all_tool_calls_paired(), Ok(()));
    let replacement_entries: Vec<_> = state.request_audits()[1]
        .context_delivery_map()
        .unwrap()
        .content_items()
        .iter()
        .filter(|entry| {
            entry
                .fragment_id()
                .as_str()
                .contains(":core-memory-replacement")
        })
        .collect();
    assert_eq!(replacement_entries.len(), 2);
    for entry in replacement_entries {
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
fn core_memory_omission_and_delivery_budget_retain_the_existing_fatal_boundaries() {
    for (case, context_window, initial, replacement, expected) in [
        (
            "omitted",
            32_768,
            None,
            "M".repeat(64 * 1024),
            "CORE_MEMORY_CHANGED",
        ),
        (
            "delivery-budget",
            32_768,
            Some("O".repeat(24 * 1024)),
            "N".repeat(24 * 1024),
            "DELIVERY_BUDGET_EXCEEDED",
        ),
    ] {
        let root = TemporaryDirectory::new(&format!("core-memory-{case}"));
        let harness = TemporaryDirectory::new(&format!("core-memory-{case}-provider"));
        initialize_project(root.path());
        fs::write(
            root.path().join("target.txt"),
            "bounded core memory fixture\n",
        )
        .unwrap();
        let memory = root.path().join(".echidna/memory/core.md");
        if let Some(initial) = initial {
            fs::create_dir_all(memory.parent().unwrap()).unwrap();
            fs::write(&memory, initial).unwrap();
        }
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
        let config = write_context_window_config(harness.path(), &base_url, context_window);
        let (sender, requests) = mpsc::channel();
        let server_memory = memory.clone();
        let server = thread::spawn(move || {
            let mut socket = f3_context::accept_before_deadline(&listener, "Core Memory rejection");
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            sender.send(read_http_request(&mut socket)).unwrap();
            fs::create_dir_all(server_memory.parent().unwrap()).unwrap();
            fs::write(&server_memory, replacement).unwrap();
            let reasoning = json!({"type":"response.output_item.done","item":{
                "type":"reasoning","content":[{"type":"reasoning_text","text":"CORE_MEMORY_REJECTION_REASONING"}]
            }});
            let response = format!(
                "data: {reasoning}\n\n{}",
                m4_long_flow_tool_response(
                    "core-memory-rejection-read",
                    "read_file",
                    json!({"path":"target.txt"}),
                    "core-memory-rejection-first",
                )
            );
            f3_context::write_sse_response(&mut socket, &response);
        });
        let server = LoopbackServer {
            base_url,
            requests,
            thread: server,
        };
        let output = f2a_wait_child(
            spawn_f2a_chat(root.path(), &config, "inspect target\n/exit\n", &[]),
            "Core Memory rejection",
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{case}: {stderr}");
        assert!(stderr.contains(expected), "{case}: {stderr}");
        assert!(
            stderr.contains("candidate was not sent"),
            "{case}: {stderr}"
        );
        assert_eq!(
            stderr.contains("DELIVERY_BUDGET_EXCEEDED"),
            expected == "DELIVERY_BUDGET_EXCEEDED",
            "{case}: {stderr}"
        );
        f2a_request_body(&server.requests, "Core Memory rejection first request");
        assert!(server.requests.try_recv().is_err());
        f2a_join_server(server, "Core Memory rejection");

        let project = Project::discover(root.path()).unwrap();
        let state = reduced_session(&project, &session_id(root.path()));
        assert_eq!(state.request_audits().len(), 1, "{case}");
        assert_eq!(state.execution_receipt_count(), 1, "{case}");
        assert_eq!(state.history().ensure_all_tool_calls_paired(), Ok(()));
        assert!(state.history().items().iter().all(|item| {
            !matches!(item, echidna::domain::model::ContentItem::Reasoning { .. })
        }));
        assert_audits_reconstruct(root.path(), &state);
    }
}
