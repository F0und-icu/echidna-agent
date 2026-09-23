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
    assert!(!stderr.contains("协议解码失败"));
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
