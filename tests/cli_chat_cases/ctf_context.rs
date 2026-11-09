use super::task_context::{initial_selection_response, selector_candidates};
use super::*;

#[derive(Clone, Copy, Debug)]
enum Change {
    ExternalPrompt,
    CoreMemory,
    Project,
    Research,
    PromptAndProject,
}

#[test]
fn ctf_r2a_isolates_prompt_memory_state_and_combined_continuation_changes() {
    for change in [
        Change::ExternalPrompt,
        Change::CoreMemory,
        Change::Project,
        Change::Research,
        Change::PromptAndProject,
    ] {
        let root = TemporaryDirectory::new("ctf-r2a-context");
        let harness = TemporaryDirectory::new("ctf-r2a-harness");
        initialize_project(root.path());
        let project = Project::discover(root.path()).unwrap();
        append_research_snapshot(&project, "r2a-before");
        if matches!(change, Change::Research) {
            append_research_event(
                &project,
                "r2a-hypothesis-created",
                ResearchEvent::HypothesisRecorded {
                    record: HypothesisRecord::from_draft(HypothesisDraft {
                        id: HypothesisId::new("r2a-selected-hypothesis"),
                        title: "Inspect target mutable hypothesis".to_owned(),
                        target_id: "fixture.target".to_owned(),
                        attack_surface: "fixture target".to_owned(),
                        vulnerability_class: "fixture".to_owned(),
                        entry_point: "fixture input".to_owned(),
                        sensitive_sink: "fixture boundary".to_owned(),
                        rationale: "same selected record must refresh".to_owned(),
                        status: HypothesisStatus::Active,
                        priority: 1,
                        evidence_links: Vec::new(),
                        last_action: None,
                        next_action: None,
                    })
                    .unwrap(),
                    keep_independent: false,
                },
            );
        }
        fs::write(root.path().join("target.txt"), "original target\n").unwrap();
        fs::create_dir_all(root.path().join(".echidna/memory")).unwrap();
        fs::write(
            root.path().join(".echidna/memory/core.md"),
            "memory before\n",
        )
        .unwrap();
        fs::write(harness.path().join("personal.md"), "prompt before\n").unwrap();
        let source_changed = matches!(change, Change::CoreMemory | Change::Research);
        // The first two samples select and expand the original Research
        // candidate. Same-Turn source refresh uses the accepted selection.
        let request_count = 4;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
        let config = write_config(harness.path(), &base_url);
        fs::write(
            &config,
            format!(
                "{}\n[prompts]\npersonal_file = \"personal.md\"\n",
                fs::read_to_string(&config)
                    .unwrap()
                    .replace("incremental_input = true", "incremental_input = false")
            ),
        )
        .unwrap();
        let target = root.path().to_path_buf();
        let prompt = harness.path().join("personal.md");
        let (sender, requests) = mpsc::channel();
        let server_thread = thread::spawn(move || {
            for index in 0..request_count {
                let mut socket = f3_context::accept_before_deadline(&listener, "CTF R2a");
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let raw = read_http_request(&mut socket);
                let wire = request_body(&raw);
                sender.send(raw).unwrap();
                if index < 2 {
                    f3_context::write_sse_response(
                        &mut socket,
                        &initial_selection_response(
                            &wire,
                            &format!("r2a-initial-selection-{index}"),
                        ),
                    );
                    continue;
                }
                if index == 2 {
                    match change {
                        Change::ExternalPrompt => fs::write(&prompt, "prompt after\n").unwrap(),
                        Change::CoreMemory => {
                            fs::write(target.join(".echidna/memory/core.md"), "memory after\n")
                                .unwrap()
                        }
                        Change::Project => {
                            fs::write(target.join("target.txt"), "changed target\n").unwrap()
                        }
                        Change::Research => {
                            append_research_event(
                                &Project::discover(&target).unwrap(),
                                "r2a-hypothesis-revised",
                                ResearchEvent::HypothesisUpdated(
                                    echidna::domain::research::HypothesisUpdate {
                                        id: HypothesisId::new("r2a-selected-hypothesis"),
                                        status: Some(HypothesisStatus::Testing),
                                        evidence_links_add: Vec::new(),
                                        last_action: None,
                                        next_action: None,
                                    },
                                ),
                            );
                        }
                        Change::PromptAndProject => {
                            fs::write(&prompt, "prompt after\n").unwrap();
                            fs::write(target.join("target.txt"), "changed target\n").unwrap();
                        }
                    }
                    let reasoning = json!({"type":"response.output_item.done", "item":{
                        "type":"reasoning", "content":[{"type":"reasoning_text", "text":"CTF_R2A_REASONING"}]
                    }});
                    let response = format!(
                        "data: {reasoning}\n\n{}",
                        m4_long_flow_tool_response(
                            "ctf-r2a-read",
                            "read_file",
                            json!({"path":"target.txt"}),
                            "r2a-first"
                        )
                    );
                    f3_context::write_sse_response(&mut socket, &response);
                } else {
                    f3_context::write_sse_response(
                        &mut socket,
                        &text_response("continued correctly", "r2a-final"),
                    );
                }
            }
        });
        let server = LoopbackServer {
            base_url,
            requests,
            thread: server_thread,
        };
        let output = f2a_wait_child(
            spawn_f2a_chat(root.path(), &config, "inspect target\n/exit\n", &[]),
            "CTF R2a context",
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{change:?}: {stderr}");
        let mut wires = Vec::new();
        for _ in 0..request_count {
            wires.push(f2a_request_body(&server.requests, "CTF R2a context"));
        }
        assert!(server.requests.try_recv().is_err());
        f2a_join_server(server, "CTF R2a context");
        let state = reduced_session(&project, &session_id(root.path()));
        assert_eq!(state.request_audits().len(), request_count);
        assert_eq!(state.execution_receipt_count(), 1);
        assert_eq!(state.history().ensure_all_tool_calls_paired(), Ok(()));
        assert!(
            !state
                .history()
                .items()
                .iter()
                .any(|item| matches!(item, echidna::domain::model::ContentItem::Reasoning { .. }))
        );
        assert_audits_reconstruct(root.path(), &state);
        selector_candidates(&wires[0]);
        selector_candidates(&wires[1]);
        let second = wires.last().unwrap();
        assert!(second.get("previous_response_id").is_none());
        assert_eq!(
            json_contains_type(&second["input"], "reasoning"),
            !source_changed
        );
        assert!(json_contains_type(&second["input"], "function_call_output"));
        let events = session_events(root.path(), &session_id(root.path()));
        let deliveries = events
            .iter()
            .filter(|event| event["event_type"] == "session.model_delivery_committed")
            .collect::<Vec<_>>();
        assert_eq!(deliveries.len(), 2);
        assert_eq!(
            deliveries[0]["payload"]["provider_session"]
                == deliveries[1]["payload"]["provider_session"],
            !source_changed,
        );
        match change {
            Change::ExternalPrompt => {
                let instructions = second["instructions"].as_str().unwrap();
                assert!(instructions.contains("prompt before"), "{instructions}");
                assert!(!instructions.contains("prompt after"), "{instructions}");
            }
            Change::CoreMemory => assert!(second.to_string().contains("memory after")),
            Change::Project => assert!(second.to_string().contains("changed target")),
            Change::Research => {
                assert!(second.to_string().contains("r2a-selected-hypothesis"));
                assert!(json_contains_text(
                    &second["input"],
                    "\"status\":\"testing\""
                ));
                assert!(!json_contains_text(
                    &second["input"],
                    "\"status\":\"active\""
                ));
            }
            Change::PromptAndProject => {
                let instructions = second["instructions"].as_str().unwrap();
                assert!(instructions.contains("prompt before"), "{instructions}");
                assert!(!instructions.contains("prompt after"), "{instructions}");
                assert!(second.to_string().contains("changed target"));
            }
        }
    }
}
