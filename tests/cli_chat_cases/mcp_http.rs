use super::*;
use echidna::domain::model::ContentItem;

const REMOTE_TOOL: &str = "remote.e2e";

fn local_name() -> String {
    format!(
        "mcp__fixture__{}",
        &blake3::hash(REMOTE_TOOL.as_bytes()).to_hex()[..20]
    )
}

fn rpc_frame(message: Value) -> String {
    format!("data: {message}\n\n")
}

fn catalog_response() -> String {
    rpc_frame(json!({"jsonrpc":"2.0", "id":"1", "result":{"tools":[{
        "name": REMOTE_TOOL,
        "description":"Remote E2E fixture",
        "inputSchema":{
            "type":"object",
            "properties":{"execution":{"$ref":"#/$defs/Execution"}},
            "$defs":{"Execution":{"type":"object","properties":{"tag":{"type":"string"}},"required":["tag"],"additionalProperties":false}},
            "required":["execution"],
            "additionalProperties":false
        }
    }]}}))
}

fn completed_response(id: &str, tag: &str) -> String {
    rpc_frame(json!({"jsonrpc":"2.0", "id":id, "result":{
        "content":[{"type":"text","text":tag}],
        "structuredContent":{"tag":tag}
    }}))
}

fn scripted_turn(tag: &str) -> Vec<(u16, String)> {
    vec![
        (
            200,
            m4_long_flow_tool_response(
                &format!("search-{tag}"),
                "tool_search",
                json!({"query":local_name(),"max_results":1}),
                &format!("search-response-{tag}"),
            ),
        ),
        (
            200,
            m4_long_flow_tool_response(
                &format!("remote-{tag}"),
                &local_name(),
                json!({"execution":{"tag":tag}}),
                &format!("remote-response-{tag}"),
            ),
        ),
        (
            200,
            text_response(&format!("completed {tag}"), &format!("final-{tag}")),
        ),
    ]
}

fn mcp_config(harness: &std::path::Path, provider: &str, remote: &str) -> std::path::PathBuf {
    let config = write_config(harness, provider);
    let encoded = fs::read_to_string(&config).unwrap();
    fs::write(
        &config,
        format!(
            "{encoded}\n[mcp_servers.fixture]\nurl = {}\nprotocol_version = \"2026-07-28\"\n",
            serde_json::to_string(remote).unwrap(),
        ),
    )
    .unwrap();
    config
}

fn writing_agent(root: &std::path::Path, name: &str) -> std::path::PathBuf {
    let definition = root.join(name);
    let output = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .current_dir(root)
        .args([
            "agent",
            "init",
            definition.to_str().unwrap(),
            "--from",
            "technical-writing",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    definition
}

fn set_remote_exposure(definition: &std::path::Path, exposure: &str) {
    let manifest = definition.join("agent.toml");
    let original = fs::read_to_string(&manifest).unwrap();
    fs::write(
        manifest,
        format!("{original}\n{} = \"{exposure}\"\n", local_name()),
    )
    .unwrap();
}

fn empty_catalog_response() -> String {
    rpc_frame(json!({"jsonrpc":"2.0", "id":"1", "result":{"tools":[]}}))
}

fn inspect_agent(
    root: &std::path::Path,
    config: &std::path::Path,
    definition: &std::path::Path,
    discover_mcp: bool,
) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_echidna"));
    command.current_dir(root).args([
        "--config",
        config.to_str().unwrap(),
        "agent",
        "inspect",
        definition.to_str().unwrap(),
    ]);
    if discover_mcp {
        command.arg("--discover-mcp");
    }
    command
        .env_remove("CYBER_AGENT_MCP_FIXTURE_API_KEY")
        .output()
        .unwrap()
}

fn take_requests(server: LoopbackServer, count: usize, label: &str) -> Vec<Value> {
    let requests = (0..count)
        .map(|_| f2a_request_body(&server.requests, label))
        .collect();
    assert!(
        server.requests.try_recv().is_err(),
        "unexpected {label} request"
    );
    f2a_join_server(server, label);
    requests
}

fn advertises_remote(request: &Value) -> bool {
    request["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tool| tool["name"] == local_name())
}

#[test]
fn agent_inspect_keeps_mcp_offline_by_default_and_exposes_only_exact_declared_names() {
    let root = TemporaryDirectory::new("mcp-agent-inspect");
    let harness = TemporaryDirectory::new("mcp-agent-inspect-config");
    initialize_project(root.path());
    let exact = writing_agent(root.path(), "exact");
    let omitted = writing_agent(root.path(), "omitted");
    let hidden = writing_agent(root.path(), "hidden");
    set_remote_exposure(&exact, "deferred");
    set_remote_exposure(&hidden, "hidden");

    let offline = mcp_config(
        harness.path(),
        "http://127.0.0.1:1/v1",
        "http://127.0.0.1:1/mcp",
    );
    let configured = fs::read_to_string(&offline).unwrap();
    fs::write(
        &offline,
        configured.replacen(
            "protocol_version = \"2026-07-28\"",
            "protocol_version = \"2026-07-28\"\ncredential_ref = \"CYBER_AGENT_MCP_FIXTURE_API_KEY\"",
            1,
        ),
    )
    .unwrap();
    let omitted_output = inspect_agent(root.path(), &offline, &omitted, false);
    assert!(
        omitted_output.status.success(),
        "{}",
        String::from_utf8_lossy(&omitted_output.stderr)
    );
    let omitted_output: Value = serde_json::from_slice(&omitted_output.stdout).unwrap();
    assert!(
        omitted_output["discovered_mcp_tools"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        !omitted_output["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["name"] == local_name())
    );
    for definition in [&exact, &hidden] {
        let output = inspect_agent(root.path(), &offline, definition, false);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("unknown Tool"));
    }

    for (definition, expected_exposure) in [
        (&exact, Some("deferred")),
        (&omitted, None),
        (&hidden, Some("hidden")),
    ] {
        let remote = spawn_responses_server(vec![(200, catalog_response())]);
        let config = mcp_config(harness.path(), "http://127.0.0.1:1/v1", &remote.base_url);
        let output = inspect_agent(root.path(), &config, definition, true);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let output: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            output["binding"]["definition"]["tools"]["exposures"]
                .get(local_name())
                .and_then(Value::as_str),
            expected_exposure
        );
        assert!(
            output["discovered_mcp_tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["name"] == local_name())
        );
        let requests = take_requests(remote, 1, "agent inspect MCP catalog");
        assert_eq!(requests[0]["method"], "tools/list");
    }
}

#[test]
fn saved_exact_mcp_binding_rejects_resume_when_the_startup_catalog_lacks_it() {
    let root = TemporaryDirectory::new("mcp-agent-missing-resume");
    let harness = TemporaryDirectory::new("mcp-agent-missing-resume-config");
    initialize_project(root.path());
    let definition = writing_agent(root.path(), "writer");
    set_remote_exposure(&definition, "deferred");

    let remote = spawn_responses_server(vec![(200, catalog_response())]);
    let config = mcp_config(harness.path(), "http://127.0.0.1:1/v1", &remote.base_url);
    let started = run_chat(
        root.path(),
        &config,
        "/exit\n",
        &["--agent", definition.to_str().unwrap()],
    );
    assert!(
        started.status.success(),
        "{}",
        String::from_utf8_lossy(&started.stderr)
    );
    let requests = take_requests(remote, 1, "initial exact MCP catalog");
    assert_eq!(requests[0]["method"], "tools/list");
    let session = session_id(root.path());
    let before = session_events(root.path(), &session);

    let remote = spawn_responses_server(vec![(200, empty_catalog_response())]);
    let config = mcp_config(harness.path(), "http://127.0.0.1:1/v1", &remote.base_url);
    let resumed = run_chat(root.path(), &config, "/exit\n", &["--session", &session]);
    assert!(!resumed.status.success());
    assert!(String::from_utf8_lossy(&resumed.stderr).contains("unknown Tool"));
    let requests = take_requests(remote, 1, "missing MCP resume catalog");
    assert_eq!(requests[0]["method"], "tools/list");
    assert_eq!(session_events(root.path(), &session), before);
}

#[test]
fn one_startup_mcp_catalog_covers_reload_use_and_compaction_preflight() {
    let root = TemporaryDirectory::new("mcp-agent-binding-lifecycle");
    let harness = TemporaryDirectory::new("mcp-agent-binding-lifecycle-config");
    initialize_project(root.path());
    let first = writing_agent(root.path(), "first");
    let second = writing_agent(root.path(), "second");
    set_remote_exposure(&first, "deferred");
    set_remote_exposure(&second, "deferred");
    fs::write(
        second.join("agent.toml"),
        fs::read_to_string(second.join("agent.toml"))
            .unwrap()
            .replacen("id = \"technical-writing\"", "id = \"second-writer\"", 1),
    )
    .unwrap();

    let remote = spawn_responses_server(vec![
        (200, catalog_response()),
        (200, completed_response("2", "lifecycle observed")),
    ]);
    let provider = spawn_responses_server(scripted_turn("lifecycle"));
    let config = mcp_config(harness.path(), &provider.base_url, &remote.base_url);
    let output = run_chat(
        root.path(),
        &config,
        &format!(
            "/agent reload\n/agent use {}\nDraft a short note.\n/compact\n/exit\n",
            second.display()
        ),
        &["--agent", first.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("reloaded at definition"), "{stdout}");
    assert!(stdout.contains("opening a new Session"), "{stdout}");
    assert!(stdout.contains("Compaction revision 1 applied"), "{stdout}");
    let requests = take_requests(remote, 2, "one MCP catalog across bindings");
    assert_eq!(requests[0]["method"], "tools/list");
    assert_eq!(requests[1]["method"], "tools/call");
    let sent = take_requests(provider, 3, "MCP binding lifecycle Provider");
    assert!(!advertises_remote(&sent[0]));
    assert!(advertises_remote(&sent[1]));
    assert!(json_contains_text(&sent[2]["input"], "lifecycle observed"));
    assert_eq!(session_ids(root.path()).len(), 2);
}

#[test]
fn deferred_remote_tool_survives_explicit_resume_without_rediscovery_or_replay() {
    let root = TemporaryDirectory::new("mcp-cli-resume");
    let harness = TemporaryDirectory::new("mcp-cli-resume-config");
    initialize_project(root.path());
    let definition = writing_agent(root.path(), "resume-writer");
    set_remote_exposure(&definition, "deferred");
    let remote = spawn_responses_server(vec![
        (200, catalog_response()),
        (200, completed_response("2", "first observed")),
        (200, completed_response("3", "second observed")),
    ]);
    let mut responses = scripted_turn("first");
    responses.extend(scripted_turn("second"));
    let provider = spawn_responses_server(responses);
    let config = mcp_config(harness.path(), &provider.base_url, &remote.base_url);
    let output = f2a_wait_child(
        spawn_f2a_chat(
            root.path(),
            &config,
            "first\n/resume\nsecond\n/exit\n",
            &["--agent", definition.to_str().unwrap()],
        ),
        "MCP explicit resume",
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let requests = take_requests(remote, 3, "MCP resume remote");
    assert_eq!(requests[0]["method"], "tools/list");
    for (request, tag) in requests[1..].iter().zip(["first", "second"]) {
        assert_eq!(request["method"], "tools/call");
        assert_eq!(request["params"]["name"], REMOTE_TOOL);
        assert_eq!(
            request["params"]["arguments"],
            json!({"execution":{"tag":tag}})
        );
    }
    let sent = take_requests(provider, 6, "MCP resume Provider");
    for start in [0, 3] {
        assert!(sent[start].get("previous_response_id").is_none());
        assert!(
            !advertises_remote(&sent[start]),
            "Resume clears deferred activation"
        );
        assert!(advertises_remote(&sent[start + 1]));
        assert!(json_contains_text(&sent[start + 2]["input"], "observed"));
    }
    let project = Project::discover(root.path()).unwrap();
    let state = reduced_session(&project, &session_id(root.path()));
    assert_eq!(state.execution_receipt_count(), 4);
    assert_eq!(state.request_audits().len(), 6);
    assert_eq!(
        state
            .history()
            .items()
            .iter()
            .filter(|item| matches!(
                item, ContentItem::ToolCall { call } if call.name.as_str() == local_name()
            ))
            .count(),
        2
    );
    for tag in ["first", "second"] {
        assert!(state.history().items().iter().any(|item| matches!(
            item, ContentItem::ToolCall { call }
                if call.call_id.as_str() == format!("remote-{tag}")
                && call.arguments == json!({"execution":{"tag":tag}})
        )));
    }
    assert!(echidna::domain::history::normalize_content_items(state.history().items()).is_ok());
}

#[test]
fn stale_compaction_resume_discovers_once_and_retains_remote_catalog_for_the_next_turn() {
    let root = TemporaryDirectory::new("mcp-cli-compaction");
    let harness = TemporaryDirectory::new("mcp-cli-compaction-config");
    initialize_project(root.path());
    let definition = writing_agent(root.path(), "compaction-writer");
    set_remote_exposure(&definition, "deferred");
    let remote = spawn_responses_server(vec![
        (200, catalog_response()),
        (
            200,
            completed_response("2", &"retained historical payload ".repeat(256)),
        ),
    ]);
    let provider = spawn_responses_server(scripted_turn("before"));
    let config = mcp_config(harness.path(), &provider.base_url, &remote.base_url);
    let first = f2a_wait_child(
        spawn_f2a_chat(
            root.path(),
            &config,
            "before\n/exit\n",
            &["--agent", definition.to_str().unwrap()],
        ),
        "MCP before compaction",
    );
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    take_requests(remote, 2, "MCP before compaction remote");
    take_requests(provider, 3, "MCP before compaction Provider");
    let session = session_id(root.path());
    let project = Project::discover(root.path()).unwrap();
    let bound = append_research_snapshot(&project, "mcp-bound");
    append_v2_compaction(&project, &session, bound);
    let _current = append_research_snapshot(&project, "mcp-current");
    let before = session_events(root.path(), &session);

    let remote = spawn_responses_server(vec![
        (200, catalog_response()),
        (200, completed_response("2", "after observed")),
    ]);
    let provider = spawn_responses_server(scripted_turn("after"));
    let config = mcp_config(harness.path(), &provider.base_url, &remote.base_url);
    let resumed = f2a_wait_child(
        spawn_f2a_chat(
            root.path(),
            &config,
            "after\n/exit\n",
            &["--session", &session],
        ),
        "MCP stale compaction resume",
    );
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    let requests = take_requests(remote, 2, "MCP after compaction remote");
    assert_eq!(requests[0]["method"], "tools/list");
    assert_eq!(requests[1]["method"], "tools/call");
    assert_eq!(
        requests[1]["params"]["arguments"],
        json!({"execution":{"tag":"after"}})
    );
    let sent = take_requests(provider, 3, "MCP after compaction Provider");
    assert!(sent[0].get("previous_response_id").is_none());
    assert!(advertises_remote(&sent[1]));
    let after = session_events(root.path(), &session);
    assert_eq!(&after[..before.len()], before.as_slice());
    assert_eq!(
        after[before.len()]["event_type"],
        "session.compaction_created"
    );
    assert_eq!(after[before.len() + 1]["event_type"], "session.resumed");
    assert!(after[before.len()]["payload"]["summary"]["evidence_spine"].is_null());
    assert_eq!(
        after[before.len()]["payload"]["summary"]["task_references"]["definition"]["agent_id"],
        "technical-writing"
    );
    let state = reduced_session(&project, &session);
    assert_eq!(state.execution_receipt_count(), 4);
    assert_eq!(state.request_audits().len(), 6);
}

#[test]
fn remote_unknown_keeps_only_known_receipts_and_resume_does_not_replay_the_remote_call() {
    let root = TemporaryDirectory::new("mcp-cli-unknown");
    let harness = TemporaryDirectory::new("mcp-cli-unknown-config");
    initialize_project(root.path());
    let definition = writing_agent(root.path(), "unknown-writer");
    set_remote_exposure(&definition, "deferred");
    let remote = spawn_responses_server(vec![
        (200, catalog_response()),
        (200, completed_response("wrong-response-id", "uncommitted")),
    ]);
    let mut responses = scripted_turn("unknown");
    responses.pop();
    let provider = spawn_responses_server(responses);
    let config = mcp_config(harness.path(), &provider.base_url, &remote.base_url);
    let output = f2a_wait_child(
        spawn_f2a_chat(
            root.path(),
            &config,
            "unknown\n/exit\n",
            &["--agent", definition.to_str().unwrap()],
        ),
        "MCP unknown outcome",
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown"));
    take_requests(remote, 2, "MCP unknown remote");
    take_requests(provider, 2, "MCP unknown Provider");
    let session = session_id(root.path());
    let project = Project::discover(root.path()).unwrap();
    let state = reduced_session(&project, &session);
    assert_eq!(
        state.execution_receipt_count(),
        1,
        "only tool_search completed"
    );
    assert_eq!(
        state.request_audits().len(),
        2,
        "no sample after unknown remote outcome"
    );
    assert!(state.history().items().iter().any(|item| matches!(
        item, ContentItem::ToolCall { call } if call.name.as_str() == local_name()
    )));
    assert!(!state.history().items().iter().any(|item| matches!(
        item, ContentItem::ToolResult { result } if result.tool_name.as_str() == local_name()
    )));
    let before = session_events(root.path(), &session);
    let remote = spawn_responses_server(vec![(200, catalog_response())]);
    let provider = spawn_responses_server(vec![(
        200,
        text_response("Prior remote outcome remains unknown.", "after-unknown"),
    )]);
    let config = mcp_config(harness.path(), &provider.base_url, &remote.base_url);
    let resumed = f2a_wait_child(
        spawn_f2a_chat(
            root.path(),
            &config,
            "do not replay\n/exit\n",
            &["--session", &session],
        ),
        "MCP explicit unknown resume",
    );
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    let requests = take_requests(remote, 1, "MCP unknown resume discovery");
    assert_eq!(requests[0]["method"], "tools/list");
    let sent = take_requests(provider, 1, "MCP explicit unknown resume Provider");
    assert!(sent[0].get("previous_response_id").is_none());
    let markers = sent[0]["input"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| {
            item["type"] == "function_call_output" && item["call_id"] == "remote-unknown"
        })
        .collect::<Vec<_>>();
    assert_eq!(markers.len(), 1);
    let marker: Value = serde_json::from_str(markers[0]["output"].as_str().unwrap()).unwrap();
    assert_eq!(marker["output"]["execution_status"], "unknown");
    assert_eq!(
        marker["output"]["guidance"],
        "do_not_assume_no_effect_or_auto_rerun"
    );
    let after = session_events(root.path(), &session);
    assert_eq!(&after[..before.len()], before.as_slice());
    assert_eq!(after[before.len()]["event_type"], "session.resumed");
    let state = reduced_session(&project, &session);
    assert_eq!(state.execution_receipt_count(), 1);
    assert_eq!(state.request_audits().len(), 3);
    assert!(
        !state.history().items().iter().any(|item| matches!(
            item, ContentItem::ToolResult { result } if result.tool_name.as_str() == local_name()
        )),
        "the recovery marker is derived delivery, never a durable remote result"
    );
}
