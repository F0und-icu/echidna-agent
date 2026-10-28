//! Configured per-response ceilings are distinct from request admission or bills.
use super::*;

#[test]
fn output_limit_incomplete_stops_atomically_and_requires_explicit_resume() {
    let root = TemporaryDirectory::new("response-output-budget");
    initialize_project(root.path());
    let events = [
        json!({"type":"response.output_item.done","item":{
            "type":"function_call","call_id":"uncommitted-write","name":"write_file",
            "arguments":json!({"path":"must-not-exist.txt","content":"incomplete","expected_hash":null}).to_string()}}),
        json!({"type":"response.incomplete","response":{
            "status":"incomplete","incomplete_details":{"reason":"max_output_tokens"}}}),
    ];
    let body = events
        .iter()
        .map(|event| format!("data: {event}\n\n"))
        .collect();
    let server = spawn_responses_server(vec![
        (200, body),
        (200, text_response("new task only", "after-explicit-resume")),
    ]);
    let config = write_context_window_config(root.path(), &server.base_url, 131_072);
    fs::write(
        &config,
        fs::read_to_string(&config).unwrap().replacen(
            "[capability_mappings.text-v1]",
            "[capability_mappings.text-v1]\nmax_output_tokens = 8192",
            1,
        ),
    )
    .unwrap();
    let extra = if cfg!(debug_assertions) {
        vec!["--debug"]
    } else {
        vec![]
    };
    let output = run_chat(
        root.path(),
        &config,
        "/phase experiment\nWrite a note.\nnot accepted\n/resume\nNew task.\n/exit\n",
        &extra,
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert!(stderr.contains("response_incomplete"));
    assert!(stderr.contains("max_output_tokens"));
    assert!(!stderr.contains("protocol decoding failed"));
    assert!(String::from_utf8_lossy(&output.stdout).contains("Task input not accepted or saved"));
    let first = f2a_request_body(&server.requests, "budget first request");
    let second = f2a_request_body(&server.requests, "budget explicit resumed request");
    for request in [&first, &second] {
        assert_eq!(request["max_output_tokens"], 8192);
        assert!(request.get("previous_response_id").is_none());
    }
    assert!(second["input"].to_string().contains("New task."));
    assert!(!second["input"].to_string().contains("not accepted"));
    assert!(!second["input"].to_string().contains("uncommitted-write"));
    assert!(server.requests.try_recv().is_err());
    server.join();
    let project = Project::discover(root.path()).unwrap();
    let state = reduced_session(&project, &session_id(root.path()));
    assert_eq!(state.execution_receipt_count(), 0);
    assert_eq!(state.request_audits().len(), 2);
    assert!(!root.path().join("must-not-exist.txt").exists());
    assert_eq!(state.history().ensure_all_tool_calls_paired(), Ok(()));
    assert_audits_reconstruct(root.path(), &state);
}

fn invalid_completed_item_status_body() -> String {
    [
        json!({"type":"response.output_item.done","item":{
            "type":"function_call","call_id":"uncommitted-write","name":"write_file",
            "arguments":json!({"path":"must-not-exist.txt","content":"uncommitted","expected_hash":null}).to_string()}}),
        json!({"type":"response.output_item.done","item":{
            "type":"reasoning","id":"REMOTE_ID_CANARY","status":"REMOTE_STATUS_CANARY",
            "content":[]}}),
    ]
    .iter()
    .map(|event| format!("data: {event}\n\n"))
    .collect()
}

#[test]
fn protocol_status_failure_after_streamed_tool_remains_atomic_and_eof_requires_resume() {
    let root = TemporaryDirectory::new("response-protocol-stop-eof");
    initialize_project(root.path());
    let server = spawn_responses_server(vec![(200, invalid_completed_item_status_body())]);
    let config = write_context_window_config(root.path(), &server.base_url, 131_072);

    let output = run_chat(
        root.path(),
        &config,
        "First durable task before protocol rejection.\n",
        &[],
    );
    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("source=output_item_done"), "{stderr}");
    assert!(stderr.contains("item_family=reasoning"), "{stderr}");
    assert!(stderr.contains("status_class=unknown_string"), "{stderr}");
    assert!(stderr.contains("[rejected response]"), "{stderr}");
    assert!(stderr.contains("REMOTE_ID_CANARY"), "{stderr}");
    assert!(stderr.contains("REMOTE_STATUS_CANARY"), "{stderr}");
    assert!(
        stdout.contains(
            "Protocol sample was rejected before completion; its streamed output was not committed and no Tool from this sample was dispatched."
        ),
        "{stdout}"
    );
    assert!(stdout.contains("EOF received;"), "{stdout}");

    let request = f2a_request_body(&server.requests, "protocol failure request");
    assert!(
        request
            .to_string()
            .contains("First durable task before protocol rejection."),
        "{request}"
    );
    assert!(server.requests.try_recv().is_err());
    server.join();

    let project = Project::discover(root.path()).unwrap();
    let state = reduced_session(&project, &session_id(root.path()));
    assert_eq!(state.execution_receipt_count(), 0);
    assert!(!root.path().join("must-not-exist.txt").exists());
    assert_eq!(state.history().ensure_all_tool_calls_paired(), Ok(()));
    assert_audits_reconstruct(root.path(), &state);
}

#[test]
fn protocol_status_failure_resume_then_eof_does_not_replay_sample_or_tool() {
    let root = TemporaryDirectory::new("response-protocol-stop-resume-eof");
    initialize_project(root.path());
    let server = spawn_responses_server(vec![(200, invalid_completed_item_status_body())]);
    let config = write_context_window_config(root.path(), &server.base_url, 131_072);

    let output = run_chat(
        root.path(),
        &config,
        "First durable task before protocol rejection.\n/resume\n",
        &[],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("ProviderSession rebuilt from durable History"),
        "{stdout}"
    );
    assert!(
        stdout.contains("No request sent or Tool retried."),
        "{stdout}"
    );
    assert!(stdout.contains("EOF received;"), "{stdout}");

    let _ = f2a_request_body(&server.requests, "protocol resumed request");
    assert!(server.requests.try_recv().is_err());
    server.join();

    let project = Project::discover(root.path()).unwrap();
    let state = reduced_session(&project, &session_id(root.path()));
    assert_eq!(state.execution_receipt_count(), 0);
    assert!(!root.path().join("must-not-exist.txt").exists());
    assert_eq!(state.history().ensure_all_tool_calls_paired(), Ok(()));
    assert_audits_reconstruct(root.path(), &state);
}
