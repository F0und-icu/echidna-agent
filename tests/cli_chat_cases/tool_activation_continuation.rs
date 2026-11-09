use super::*;

#[test]
fn full_reasoning_with_project_diff_survives_additive_tool_activation() {
    activation_case(None);
}

#[test]
fn additive_activation_pins_personal_instructions_and_commits_core_epoch_change() {
    for changed in ["prompt", "memory"] {
        activation_case(Some(changed));
    }
}

fn activation_case(change: Option<&str>) {
    let root = TemporaryDirectory::new("activation-project");
    let harness = TemporaryDirectory::new("activation-harness");
    initialize_project(root.path());
    fs::write(root.path().join("target.txt"), "before").unwrap();
    fs::create_dir_all(root.path().join(".echidna/memory")).unwrap();
    let memory = root.path().join(".echidna/memory/core.md");
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
    let main_count = match change {
        Some("prompt" | "memory") => 4,
        None => 6,
        Some(_) => unreachable!("the fixture only changes prompt or Core Memory"),
    };
    let selector_steps: &[usize] = match change {
        None => &[0, 5, 7],
        Some("memory") => &[0],
        Some("prompt") => &[0],
        Some(_) => unreachable!(),
    };
    let count = main_count + selector_steps.len();
    let (sender, requests) = mpsc::channel();
    let thread = thread::spawn(move || {
        for step in 0..count {
            let mut socket = f3_context::accept_before_deadline(&listener, "activation");
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let raw = read_http_request(&mut socket);
            let wire = request_body(&raw);
            sender.send(raw).unwrap();
            if step == 0 {
                let response =
                    task_context::initial_selection_response(&wire, "activation-initial-selection");
                f3_context::write_sse_response(&mut socket, &response);
                continue;
            }
            if selector_steps.contains(&step) {
                let response = task_context::continue_selection_response(
                    &wire,
                    &format!("activation-selection-{step}"),
                    None,
                );
                f3_context::write_sse_response(&mut socket, &response);
                continue;
            }
            let index = step
                - selector_steps
                    .iter()
                    .filter(|selector| **selector < step)
                    .count();
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
    let all_wires: Vec<_> = (0..count)
        .map(|_| f2a_request_body(&server.requests, "activation wire"))
        .collect();
    assert!(server.requests.try_recv().is_err());
    f2a_join_server(server, "activation server");
    let wires = all_wires
        .iter()
        .enumerate()
        .filter_map(|(step, wire)| (!selector_steps.contains(&step)).then_some(wire))
        .collect::<Vec<_>>();
    assert_eq!(wires.len(), main_count);
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
    assert!(!has_reader(wires[0]));
    assert!(!has_reader(wires[1]));
    assert!(has_reader(wires[2]));
    assert!(has_reader(wires[3]));
    if change != Some("memory") {
        for marker in [
            "RETAIN_REASONING_0",
            "RETAIN_REASONING_1",
            "project_snapshot_diff",
        ] {
            assert!(wires[2].to_string().contains(marker));
        }
    }
    if change == Some("prompt") {
        let instructions = wires[2]["instructions"].as_str().unwrap();
        assert!(instructions.contains("prompt-before"), "{instructions}");
        assert!(!instructions.contains("prompt-after"), "{instructions}");
    }
    if change == Some("memory") {
        // ADR-0074 permits retirement after the durable source selection
        // whose order and Provider identity are checked below.
        assert!(wires[2].to_string().contains("memory-after"));
        for retired in ["memory-before", "RETAIN_REASONING_0", "RETAIN_REASONING_1"] {
            assert!(
                !wires[2].to_string().contains(retired),
                "the source-verified Core selection must exclude retired content: {retired}"
            );
        }
        let project_fragment = |wire: &Value, kind: &str| {
            wire["input"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|item| item["content"].as_array().into_iter().flatten())
                .filter_map(|content| serde_json::from_str::<Value>(content["text"].as_str()?).ok())
                .find(|value| value["kind"] == kind)
                .unwrap_or_else(|| panic!("missing {kind}"))
        };
        let current_full = project_fragment(wires[2], "project_snapshot_full");
        let events = session_events(root.path(), &session_id(root.path()));
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
        let captured_project = capture["common"]["fragments"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|fragment| {
                fragment["content"]["items"]
                    .as_array()
                    .into_iter()
                    .flatten()
            })
            .filter_map(|item| serde_json::from_str::<Value>(item["text"].as_str()?).ok())
            .find(|value| value["kind"] == "project_snapshot_full")
            .unwrap();
        assert_eq!(current_full, captured_project);
        assert!(
            wires[3].to_string().contains("RETAIN_REASONING_2"),
            "reasoning accepted in the new epoch continues normally"
        );
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
    if change.is_none() {
        // Normal expiry removes dispatch eligibility, not retained reasoning
        // or state diffs. A later explicit process Resume remains supported.
        assert!(!has_reader(wires[4]), "activation must clear at next Turn");
        assert!(!has_reader(wires[5]), "activation must clear at Resume");
        assert!(wires[4].to_string().contains("RETAIN_REASONING_0"));
        assert!(wires[4].to_string().contains("project_snapshot_diff"));
    }
    let project = Project::discover(root.path()).unwrap();
    let state = reduced_session(&project, &session_id(root.path()));
    assert_eq!(state.request_audits().len(), count);
    assert_eq!(state.execution_receipt_count(), 3);
    assert_eq!(state.history().ensure_all_tool_calls_paired(), Ok(()));
    assert_audits_reconstruct(root.path(), &state);
    if change == Some("memory") {
        let events = session_events(root.path(), &session_id(root.path()));
        let selections = events
            .iter()
            .filter(|event| event["event_type"] == "session.task_context_selected")
            .collect::<Vec<_>>();
        assert_eq!(selections.len(), 3);
        assert_eq!(
            selections[1]["payload"]["refresh_of"],
            selections[0]["event_id"]
        );
        assert_eq!(
            selections[2]["payload"]["refresh_of"],
            selections[1]["event_id"]
        );
        assert_eq!(
            selections[1]["payload"]["provider_session"],
            selections[0]["payload"]["provider_session"],
            "snapshot-only refresh must retain the accepted epoch"
        );
        assert!(selections[1]["payload"]["sample_event_id"].is_null());
        assert!(selections[2]["payload"]["sample_event_id"].is_null());
        let audits = events
            .iter()
            .filter(|event| event["event_type"] == "session.request_audit_recorded")
            .collect::<Vec<_>>();
        assert_ne!(
            audits[2]["payload"]["provider_session"],
            audits[4]["payload"]["provider_session"]
        );
        assert_eq!(
            selections[2]["payload"]["provider_session"],
            audits[4]["payload"]["provider_session"]
        );
        assert!(
            selections[2]["sequence"].as_u64().unwrap() < audits[4]["sequence"].as_u64().unwrap()
        );
        let durable_result = events
            .iter()
            .find(|event| {
                event["event_type"] == "session.history_item_recorded"
                    && event["payload"]["item"]["kind"] == "tool_result"
                    && event["payload"]["item"]["result"]["call_id"] == "activate-reader"
            })
            .unwrap();
        assert!(
            durable_result["sequence"].as_u64().unwrap()
                < selections[2]["sequence"].as_u64().unwrap(),
            "a complete durable Tool batch must precede Core epoch retirement"
        );
    }
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
    assert!(state.history().items().iter().any(|item| match item {
        echidna::domain::model::ContentItem::ToolResult { result } =>
            result.tool_name.as_str() == "read_output" && result.output["text"] == "ACTIVATION_RAW",
        _ => false,
    }));
}
