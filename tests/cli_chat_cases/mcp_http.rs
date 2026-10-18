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
fn deferred_remote_tool_survives_explicit_resume_without_rediscovery_or_replay() {
    let root = TemporaryDirectory::new("mcp-cli-resume");
    let harness = TemporaryDirectory::new("mcp-cli-resume-config");
    initialize_project(root.path());
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
        spawn_f2a_chat(root.path(), &config, "first\n/resume\nsecond\n/exit\n", &[]),
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
        spawn_f2a_chat(root.path(), &config, "before\n/exit\n", &[]),
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
    let current = append_research_snapshot(&project, "mcp-current");
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
    assert_eq!(
        after[before.len()]["payload"]["summary"]["evidence_spine"]["target_snapshot_refs"],
        json!([current.as_str()])
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
    let remote = spawn_responses_server(vec![
        (200, catalog_response()),
        (200, completed_response("wrong-response-id", "uncommitted")),
    ]);
    let mut responses = scripted_turn("unknown");
    responses.pop();
    let provider = spawn_responses_server(responses);
    let config = mcp_config(harness.path(), &provider.base_url, &remote.base_url);
    let output = f2a_wait_child(
        spawn_f2a_chat(root.path(), &config, "unknown\n/exit\n", &[]),
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
