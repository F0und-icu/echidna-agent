use super::*;

#[test]
fn full_reasoning_with_project_diff_survives_additive_tool_activation() {
    activation_case(None);
}

#[test]
fn additive_activation_keeps_personal_instructions_pinned_and_rejects_memory_replacement() {
    for changed in ["prompt", "memory"] {
        activation_case(Some(changed));
    }
}

fn activation_case(change: Option<&str>) {
    let root = TemporaryDirectory::new("activation-project");
    let harness = TemporaryDirectory::new("activation-harness");
    initialize_project(root.path());
    fs::write(root.path().join("target.txt"), "before").unwrap();
    fs::create_dir_all(root.path().join(".cyber-agent/memory")).unwrap();
    let memory = root.path().join(".cyber-agent/memory/core.md");
    fs::write(&memory, "memory-before").unwrap();
    let prompt = harness.path().join("personal.md");
    fs::write(&prompt, "prompt-before").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let config = write_context_window_config(harness.path(), &base_url, 1_048_576);
    let configured = fs::read_to_string(&config)
        .unwrap()
        .replace("incremental_input = true", "incremental_input = false");
    fs::write(
        &config,
        format!("{configured}\n[prompts]\npersonal_file = \"personal.md\"\n"),
    )
    .unwrap();
    let target = root.path().join("target.txt");
    let changed = change.map(str::to_owned);
    let count = match change {
        Some("memory") => 2,
        Some("prompt") => 4,
        None => 6,
        Some(_) => unreachable!(),
    };
    let (sender, requests) = mpsc::channel();
    let thread = thread::spawn(move || {
        for index in 0..count {
            let mut socket = f3_context::accept_before_deadline(&listener, "activation");
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            sender.send(read_http_request(&mut socket)).unwrap();
            let response = match index {
                0 => {
                    fs::write(&target, "after: forces a current Project diff").unwrap();
                    m4_long_flow_tool_response(
                        "raw-once",
                        "run_command",
                        json!({"argv":["/usr/bin/printf","ACTIVATION_RAW"]}),
                        "first",
                    )
                }
                1 => {
                    match changed.as_deref() {
                        Some("prompt") => fs::write(&prompt, "prompt-after").unwrap(),
                        Some("memory") => fs::write(&memory, "memory-after").unwrap(),
                        _ => {}
                    }
                    m4_long_flow_tool_response(
                        "activate-reader",
                        "tool_search",
                        json!({"query":"read_output"}),
                        "search",
                    )
                }
                2 => m4_long_flow_tool_response(
                    "read-retained",
                    "read_output",
                    json!({"blob_id":format!("{:x}", Sha256::digest(b"ACTIVATION_RAW"))}),
                    "read",
                ),
                _ => text_response("completed fixture", &format!("final-{index}")),
            };
            let response = if index < 3 {
                let reasoning_part = json!({"type":"response.content_part.done","part":{
                    "type":"reasoning_text","text":format!("RETAIN_REASONING_{index}")}});
                let reasoning = json!({"type":"response.output_item.done","item":{
                    "type":"reasoning","content":[{"type":"reasoning_text","text":format!("RETAIN_REASONING_{index}")}]}});
                format!("data: {reasoning_part}\n\ndata: {reasoning}\n\n{response}")
            } else {
                response
            };
            f3_context::write_sse_response(&mut socket, &response);
        }
    });
    let server = LoopbackServer {
        base_url,
        requests,
        thread,
    };
    let output = wait_child_before_deadline(
        spawn_f2a_chat(
            root.path(),
            &config,
            if change.is_some() {
                "read retained output\n/exit\n"
            } else {
                "read retained output\nnew turn\n/exit\n"
            },
            &[],
        ),
        "activation continuation",
        Duration::from_secs(45),
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    if change == Some("memory") {
        assert!(!output.status.success(), "memory: {stderr}");
        assert!(stderr.contains("CORE_MEMORY_CHANGED"), "{stderr}");
    } else {
        // Normal expiry removes dispatch eligibility, not retained reasoning
        // or state diffs. A later explicit process Resume remains supported.
        assert!(output.status.success(), "{stderr}");
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("completed fixture"),
            "{stderr}"
        );
        if change.is_none() {
            let resumed = f2a_wait_child(
                spawn_f2a_chat(
                    root.path(),
                    &config,
                    "resumed turn\n/exit\n",
                    &["--resume", "latest"],
                ),
                "activation resume",
            );
            assert!(
                resumed.status.success(),
                "{}",
                String::from_utf8_lossy(&resumed.stderr)
            );
        }
    }
    let wires: Vec<_> = (0..count)
        .map(|_| f2a_request_body(&server.requests, "activation wire"))
        .collect();
    assert!(server.requests.try_recv().is_err());
    f2a_join_server(server, "activation server");
    for wire in &wires {
        assert!(wire.get("previous_response_id").is_none());
    }
    assert!(wires[1].to_string().contains("project_snapshot_diff"));
    let has_reader = |wire: &Value| {
        wire["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["name"] == "read_output")
    };
    assert!(!has_reader(&wires[0]));
    assert!(!has_reader(&wires[1]));
    if change != Some("memory") {
        assert!(has_reader(&wires[2]));
        assert!(has_reader(&wires[3]));
        if change == Some("prompt") {
            let instructions = wires[2]["instructions"].as_str().unwrap();
            assert!(instructions.contains("prompt-before"), "{instructions}");
            assert!(!instructions.contains("prompt-after"), "{instructions}");
        } else {
            assert!(!has_reader(&wires[4]), "activation must clear at next Turn");
            assert!(!has_reader(&wires[5]), "activation must clear at Resume");
            assert!(wires[4].to_string().contains("RETAIN_REASONING_0"));
            assert!(wires[4].to_string().contains("project_snapshot_diff"));
        }
        for marker in [
            "RETAIN_REASONING_0",
            "RETAIN_REASONING_1",
            "project_snapshot_diff",
        ] {
            assert!(wires[2].to_string().contains(marker));
        }
        let items = wires[3]["input"].as_array().unwrap();
        let mut pending = BTreeSet::new();
        for item in items {
            if item["type"] == "function_call" {
                assert!(pending.insert(item["call_id"].as_str().unwrap()));
            }
            if item["type"] == "function_call_output" {
                assert!(pending.remove(item["call_id"].as_str().unwrap()));
            }
        }
        assert!(pending.is_empty());
    }
    let project = Project::discover(root.path()).unwrap();
    let state = reduced_session(&project, &session_id(root.path()));
    assert_eq!(state.request_audits().len(), count);
    assert_eq!(
        state.execution_receipt_count(),
        if change == Some("memory") { 2 } else { 3 }
    );
    assert_eq!(state.history().ensure_all_tool_calls_paired(), Ok(()));
    assert_audits_reconstruct(root.path(), &state);
    let calls: Vec<_> = state
        .history()
        .items()
        .iter()
        .filter_map(|item| match item {
            echidna::domain::model::ContentItem::ToolCall { call } => Some(call.name.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        calls.iter().filter(|name| **name == "run_command").count(),
        1
    );
    if change != Some("memory") {
        assert!(state.history().items().iter().any(|item| match item {
            echidna::domain::model::ContentItem::ToolResult { result } =>
                result.tool_name.as_str() == "read_output"
                    && result.output["text"] == "ACTIVATION_RAW",
            _ => false,
        }));
    }
}
