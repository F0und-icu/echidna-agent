use super::task_context::{
    ScriptedResponse::{ContinueSelection, InitialSelection, Response},
    selector_candidates, spawn_scripted_responses_server,
};
use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};

#[test]
fn ctf_r3a2_blob_failure_never_records_a_durable_receipt_or_tool_result() {
    for conflicting_stream in [b"OUT".as_slice(), b"ERR".as_slice()] {
        let root = TemporaryDirectory::new("r3a2-cli-publication-failure");
        let harness = TemporaryDirectory::new("r3a2-failure-config");
        initialize_project(root.path());
        let blob_directory = root.path().join(".echidna/artifacts/blobs/sha256");
        fs::create_dir_all(&blob_directory).unwrap();
        // A same-sized conflicting Blob exercises first/second publication
        // failure in the actual CLI durability path without a public test hook.
        fs::write(
            blob_directory.join(format!("{:x}", Sha256::digest(conflicting_stream))),
            b"bad",
        )
        .unwrap();
        let server = spawn_scripted_responses_server(vec![
            InitialSelection,
            Response(
                200,
                m4_long_flow_tool_response(
                    "capture",
                    "run_command",
                    json!({"argv":["sh","-c","printf x >> executed; printf OUT; printf ERR >&2"]}),
                    "capture-response",
                ),
            ),
        ]);
        let config = write_config(harness.path(), &server.base_url);
        let output = f2a_wait_child(
            spawn_f2a_chat(root.path(), &config, "capture\n/exit\n", &[]),
            "R3a2 publication failure",
        );
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("Tool output persistence failed"),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        selector_candidates(&f2a_request_body(
            &server.requests,
            "R3a2 failed publication selection",
        ));
        let _ = f2a_request_body(&server.requests, "R3a2 failed publication");
        f2a_join_server(server, "R3a2 failed publication");
        assert_eq!(fs::read(root.path().join("executed")).unwrap(), b"x");
        let events = session_events(root.path(), &session_id(root.path()));
        assert!(
            events
                .iter()
                .any(|event| event["payload"]["item"]["kind"] == "tool_call")
        );
        assert!(!events.iter().any(|event| event["event_type"]
            == "tool.execution_outcome_recorded"
            || event["event_type"] == "tool.result_cache_hit"));
        let projection = echidna::app::session::SessionProjection::replay(
            events
                .into_iter()
                .map(|event| serde_json::from_value(event).unwrap()),
        )
        .unwrap();
        assert!(
            !projection
                .state()
                .history()
                .items()
                .iter()
                .any(|item| matches!(item, echidna::domain::model::ContentItem::ToolResult { .. }))
        );
    }
}

#[test]
fn ctf_r3a2_durable_output_can_be_read_after_explicit_resume() {
    let root = TemporaryDirectory::new("r3a2-cli");
    let harness = TemporaryDirectory::new("r3a2-config");
    initialize_project(root.path());
    let mut expected = vec![0u8; 150 * 1024];
    expected.extend_from_slice(b"\xffTAIL");
    let blob_id = format!("{:x}", Sha256::digest(&expected));
    let first_server = spawn_scripted_responses_server(vec![
        InitialSelection,
        Response(
            200,
            m4_long_flow_tool_response(
                "capture",
                "run_command",
                json!({"argv":["sh","-c","dd if=/dev/zero bs=1024 count=150 2>/dev/null; printf '\\377TAIL'; printf x >> executed"]}),
                "capture-response",
            ),
        ),
    ]);
    let config = write_config(harness.path(), &first_server.base_url);
    let mut config_text = fs::read_to_string(&config)
        .unwrap()
        .replace("incremental_input = true", "incremental_input = false")
        .replace("context_window = 32768", "context_window = 1048576");
    config_text.push_str("\n[runtime]\nmodel_request_limit_per_turn = 2\n");
    fs::write(&config, config_text).unwrap();
    let stopped = f2a_wait_child(
        spawn_f2a_chat(root.path(), &config, "capture\n/exit\n", &[]),
        "R3a2 bounded first turn",
    );
    assert!(!stopped.status.success());
    assert!(String::from_utf8_lossy(&stopped.stderr).contains("per-Turn model-request limit"));
    selector_candidates(&f2a_request_body(
        &first_server.requests,
        "R3a2 initial selection",
    ));
    let _ = f2a_request_body(&first_server.requests, "R3a2 first");
    f2a_join_server(first_server, "R3a2 first");
    let server = spawn_scripted_responses_server(vec![
        ContinueSelection { usage: None },
        Response(
            200,
            m4_long_flow_tool_response(
                "search",
                "tool_search",
                json!({"query":"read_output"}),
                "search-response",
            ),
        ),
        Response(
            200,
            m4_long_flow_tool_response(
                "readback",
                "read_output",
                json!({"blob_id":blob_id,"offset":153600,"max_bytes":5}),
                "readback-response",
            ),
        ),
        Response(200, text_response("RECOVERED RAW OUTPUT", "finished")),
    ]);
    let config = write_config(harness.path(), &server.base_url);
    let config_text = fs::read_to_string(&config)
        .unwrap()
        .replace("incremental_input = true", "incremental_input = false")
        .replace("context_window = 32768", "context_window = 1048576");
    fs::write(&config, config_text).unwrap();
    let output = f2a_wait_child(
        spawn_f2a_chat(
            root.path(),
            &config,
            "read retained bytes\n/exit\n",
            &["--resume", "latest"],
        ),
        "R3a2 raw recovery",
    );
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    selector_candidates(&f2a_request_body(
        &server.requests,
        "R3a2 resumed source selection",
    ));
    let requests: Vec<Value> = (0..3)
        .map(|_| f2a_request_body(&server.requests, "R3a2 recovery"))
        .collect();
    f2a_join_server(server, "R3a2 recovery");
    assert!(requests[0].to_string().contains(&blob_id));
    assert!(
        requests[2]
            .to_string()
            .contains(&STANDARD.encode(b"\xffTAIL"))
    );
    assert_eq!(fs::read(root.path().join("executed")).unwrap(), b"x");
    let events = session_events(root.path(), &session_id(root.path()));
    let receipt = events
        .iter()
        .find(|event| {
            event["event_type"] == "tool.execution_outcome_recorded"
                && event["payload"]["receipt"]["execution_id"] == "execution:capture"
        })
        .unwrap();
    assert_eq!(receipt["payload"]["schema_version"], 1);
    let captures = &receipt["payload"]["receipt"]["output_captures"];
    assert_eq!(captures[0]["blob_id"], blob_id);
    assert_eq!(captures[0]["complete"], true);
    let blob_path = root
        .path()
        .join(".echidna/artifacts/blobs/sha256")
        .join(&blob_id);
    assert_eq!(fs::read(&blob_path).unwrap(), expected);
    // Exercise exact binary readback via the production Tool adapter on the
    // resumed project's retained Blob, not a private-path read implementation.
    let tools = echidna::adapters::tools::ToolRegistry::for_project(root.path());
    use echidna::domain::tools::{ExecutionId, ToolCall, ToolCallId, ToolName};
    use echidna::ports::tool::ToolExecutor;
    let plan = tools
        .plan(
            ExecutionId::new("read-saved"),
            ToolCall::new(
                ToolCallId::new("read-saved"),
                ToolName::new("read_output"),
                json!({"blob_id":blob_id,"offset":153600,"max_bytes":5}),
            ),
        )
        .unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let read = runtime
        .block_on(tools.execute(plan.clone()))
        .expect("Tool execution completes");
    assert_eq!(
        read.result.output["data_base64"],
        STANDARD.encode(b"\xffTAIL")
    );
    assert_eq!(read.result.output["text_lossy"], true);
    assert_eq!(read.result.output["has_more"], false);
    let mut tampered = expected;
    tampered[0] ^= 1;
    // Same size and valid offset: rejection must reach SHA-256 verification,
    // not merely reject an offset beyond a shortened file.
    fs::write(&blob_path, &tampered).unwrap();
    let rejected = runtime
        .block_on(tools.execute(plan))
        .expect("Tool execution completes");
    assert_eq!(
        rejected.result.status,
        echidna::domain::tools::ToolResultStatus::Failed
    );
}
