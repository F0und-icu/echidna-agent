use super::*;

#[test]
fn truncated_http_body_requires_explicit_resume_without_replaying_completed_tools() {
    let root = TemporaryDirectory::new("body-transport-recovery");
    initialize_project(root.path());
    fs::write(root.path().join("target.txt"), "durable tool result").unwrap();
    let completed_tool = m4_long_flow_tool_response(
        "completed-call",
        "read_file",
        json!({"path":"target.txt"}),
        "completed-response",
    );
    let interrupted = [
        json!({"type":"response.output_text.delta", "delta":"uncommitted fragment"}),
        json!({"type":"response.output_item.done", "output_index":1,
            "item":{"type":"function_call", "call_id":"uncommitted-call", "name":"write_file",
                "arguments":"{\"path\":\"must-not-exist.txt\",\"content\":\"uncommitted\",\"expected_hash\":null}"}}),
    ].iter().map(|event| format!("data: {event}\n\n")).collect::<String>();
    let recovered = text_response("explicit recovery completed", "new-response");
    let responses = vec![
        (200, completed_tool.clone(), completed_tool.len()),
        (200, interrupted.clone(), interrupted.len() + 1),
        (200, recovered.clone(), recovered.len()),
    ];
    let server = spawn_responses_server_with_lengths(responses);
    let config = write_config(root.path(), &server.base_url);
    let output = f2a_wait_child(
        spawn_f2a_chat(
            root.path(),
            &config,
            "original task\nblocked input\n/resume\nnew instruction\n/exit\n",
            &["--debug"],
        ),
        "body transport recovery",
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stdout}\n{stderr}");
    assert!(stderr.contains("[turn stopped] model transport failed: response body read failed before `response.completed`"), "{stderr}");
    assert!(!stderr.contains("invalid SSE stream: Transport error:"));
    for expected in [
        "HTTP 200",
        "elapsed_ms=",
        "timeout=false",
        "io_kind=Some(UnexpectedEof)",
    ] {
        assert!(stderr.contains(expected), "{stderr}");
    }
    assert!(stdout.contains("Task input not accepted or saved"));
    assert!(stdout.contains("explicit recovery completed"));
    let _ = f2a_request_body(&server.requests, "completed tool sample");
    let _ = f2a_request_body(&server.requests, "interrupted followup");
    let resumed = f2a_request_body(&server.requests, "explicit recovery");
    assert!(resumed.get("previous_response_id").is_none());
    assert!(resumed.to_string().contains("durable tool result"));
    for excluded in ["uncommitted fragment", "uncommitted-call", "blocked input"] {
        assert!(!resumed.to_string().contains(excluded));
    }
    assert!(server.requests.try_recv().is_err());
    f2a_join_server(server, "body transport recovery");
    assert!(!root.path().join("must-not-exist.txt").exists());
    let state = reduced_session(
        &Project::discover(root.path()).unwrap(),
        &session_id(root.path()),
    );
    assert_eq!(state.request_audits().len(), 3);
    assert_eq!(state.execution_receipt_count(), 1);
    assert_eq!(state.history().ensure_all_tool_calls_paired(), Ok(()));
    assert_eq!(
        state.history().items().last().unwrap(),
        &echidna::domain::model::ContentItem::assistant_text("explicit recovery completed")
    );
    assert_audits_reconstruct(root.path(), &state);
}
