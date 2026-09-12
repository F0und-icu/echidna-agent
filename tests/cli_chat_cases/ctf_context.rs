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
    for (change, recover) in [
        (Change::ExternalPrompt, false),
        (Change::CoreMemory, false),
        (Change::Project, false),
        (Change::Research, false),
        (Change::PromptAndProject, false),
        (Change::CoreMemory, true),
        (Change::PromptAndProject, true),
    ] {
        let root = TemporaryDirectory::new("ctf-r2a-context");
        let harness = TemporaryDirectory::new("ctf-r2a-harness");
        initialize_project(root.path());
        let project = Project::discover(root.path()).unwrap();
        append_research_snapshot(&project, "r2a-before");
        fs::write(root.path().join("target.txt"), "original target\n").unwrap();
        fs::create_dir_all(root.path().join(".cyber-agent/memory")).unwrap();
        fs::write(
            root.path().join(".cyber-agent/memory/core.md"),
            "memory before\n",
        )
        .unwrap();
        fs::write(harness.path().join("personal.md"), "prompt before\n").unwrap();
        let rejected = match change {
            Change::CoreMemory => Some("CORE_MEMORY_CHANGED"),
            Change::PromptAndProject => Some("BASELINE_REJECTED: PromptChanged"),
            _ => None,
        };
        let request_count = if rejected.is_some() && !recover { 1 } else { 2 };
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
                sender.send(read_http_request(&mut socket)).unwrap();
                if index == 0 {
                    match change {
                        Change::ExternalPrompt => fs::write(&prompt, "prompt after\n").unwrap(),
                        Change::CoreMemory => {
                            fs::write(target.join(".cyber-agent/memory/core.md"), "memory after\n")
                                .unwrap()
                        }
                        Change::Project => {
                            fs::write(target.join("target.txt"), "changed target\n").unwrap()
                        }
                        Change::Research => {
                            append_research_snapshot(
                                &Project::discover(&target).unwrap(),
                                "r2a-after",
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
            spawn_f2a_chat(
                root.path(),
                &config,
                if recover {
                    "inspect target\n/resume\ncontinue explicitly\n/exit\n"
                } else {
                    "inspect target\n/exit\n"
                },
                &[],
            ),
            "CTF R2a context",
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(
            output.status.success(),
            rejected.is_none() || recover,
            "{change:?}: {stderr}"
        );
        if let Some(reason) = rejected {
            assert!(stderr.contains(reason), "{change:?}: {stderr}");
            assert!(stderr.contains("candidate was not sent"));
            assert!(!stderr.contains("DELIVERY_BUDGET_EXCEEDED"));
            assert!(String::from_utf8_lossy(&output.stdout).contains("Use /resume"));
        }
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
        assert!(!state.history().items().iter().any(|item| matches!(
            item,
            cyber_agent::domain::model::ContentItem::Reasoning { .. }
        )));
        assert_audits_reconstruct(root.path(), &state);
        if recover {
            assert!(wires[1].get("previous_response_id").is_none());
            assert!(!json_contains_type(&wires[1]["input"], "reasoning"));
            assert!(json_contains_type(
                &wires[1]["input"],
                "function_call_output"
            ));
            assert!(wires[1].to_string().contains("continue explicitly"));
            assert!(wires[1].to_string().contains(match change {
                Change::CoreMemory => "memory after",
                Change::PromptAndProject => "prompt after",
                _ => unreachable!(),
            }));
        }
        if rejected.is_none() {
            let second = &wires[1];
            assert!(second.get("previous_response_id").is_none());
            assert!(json_contains_type(&second["input"], "reasoning"));
            assert!(json_contains_type(&second["input"], "function_call_output"));
            match change {
                Change::ExternalPrompt => assert!(
                    second["instructions"]
                        .as_str()
                        .unwrap()
                        .contains("prompt after")
                ),
                Change::Project => assert!(second.to_string().contains("changed target")),
                Change::Research => assert!(second.to_string().contains("snapshot-r2a-after")),
                _ => unreachable!(),
            }
        }
    }
}
