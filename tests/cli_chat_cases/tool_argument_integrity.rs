//! Ordinary document fixtures; no model requests or security task execution.
use super::*;

#[test]
fn malformed_arguments_reject_the_whole_sample_without_writes_or_payload_disclosure() {
    for suffix in [r#"\q"}"#, r#"" "missing-comma"}"#, ""] {
        let root = TemporaryDirectory::new("malformed-tool-arguments");
        initialize_project(root.path());
        let arguments = format!(
            "{{\"content\":\"PRIVATE_CONTENT{}{}",
            "x".repeat(9_200),
            suffix
        );
        let events = [
            json!({"type":"response.output_item.done","item":{
                "type":"function_call","call_id":"valid-but-uncommitted","name":"write_file",
                "arguments":json!({"path":"must-not-exist.txt","content":"uncommitted","expected_hash":null}).to_string()}}),
            json!({"type":"response.output_item.done","item":{
                "type":"function_call","call_id":"PRIVATE_CALL_ID","name":"PRIVATE_TOOL_NAME",
                "arguments":arguments}}),
            json!({"type":"response.completed","response":{"id":"must-not-commit"}}),
        ];
        let body: String = events
            .iter()
            .map(|event| format!("data: {event}\n\n"))
            .collect();
        let server = spawn_responses_server(vec![(200, body); 3]);
        let config = write_config(root.path(), &server.base_url);
        let extra = if cfg!(debug_assertions) {
            vec!["--debug"]
        } else {
            vec![]
        };
        let output = run_chat(
            root.path(),
            &config,
            "/phase experiment\nWrite an ordinary note.\n/exit\n",
            &extra,
        );
        assert!(!output.status.success());
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(
            stderr.contains("function call arguments are not valid JSON"),
            "{stderr}"
        );
        for field in [
            "category=",
            "line=1",
            "column=",
            "arguments_bytes=",
            "rejected before dispatch",
        ] {
            assert!(stderr.contains(field), "{stderr}");
        }
        for private in ["PRIVATE_CONTENT", "PRIVATE_CALL_ID", "PRIVATE_TOOL_NAME"] {
            assert!(
                !stderr.contains(private),
                "invalid JSON/identity must not bypass redaction"
            );
        }
        if cfg!(debug_assertions) {
            assert!(stderr.contains("协议解码失败：响应未提交（不代表网络中断）"));
            assert!(!stderr.contains("响应中断：未观察到完成"));
        }
        let project = Project::discover(root.path()).unwrap();
        let state = reduced_session(&project, &session_id(root.path()));
        assert_eq!(state.execution_receipt_count(), 0);
        assert_eq!(state.request_audits().len(), 3);
        let rejections: Vec<_> = session_events(root.path(), &session_id(root.path()))
            .into_iter()
            .filter(|event| event["event_type"] == "model.sample_rejected")
            .collect();
        assert_eq!(rejections.len(), 3);
        for (index, rejection) in rejections.iter().enumerate() {
            assert_eq!(rejection["payload"]["corrections_used"], index);
            assert_eq!(
                rejection["payload"]["request_audit_event_id"],
                state.request_audits()[index].audit_event_id().as_str()
            );
        }
        assert!(stderr.contains("argument corrections exhausted"));
        assert_eq!(
            state.history().items(),
            &[cyber_agent::domain::model::ContentItem::user_text(
                "Write an ordinary note."
            )]
        );
        assert!(!root.path().join("must-not-exist.txt").exists());
        for _ in 0..3 {
            server
                .requests
                .recv_timeout(Duration::from_secs(10))
                .unwrap();
        }
        assert!(server.requests.try_recv().is_err(), "no unbounded retry");
        assert_audits_reconstruct(root.path(), &state);
        server.join();
    }
}

#[test]
fn long_json_escaped_file_content_round_trips_through_wire_tool_and_durable_history() {
    let root = TemporaryDirectory::new("long-tool-arguments");
    initialize_project(root.path());
    let content =
        "中文 😀 quote=\"; path=C:\\notes\\new; regex=\\d+; tab=\t; line=\nWPEof\n".repeat(200);
    assert!(content.len() > 9_194);
    let arguments = json!({"path":"note.txt","content":content,"expected_hash":null});
    let server = spawn_responses_server(vec![
        (
            200,
            m4_long_flow_tool_response(
                "long-write",
                "write_file",
                arguments.clone(),
                "long-write-response",
            ),
        ),
        (
            200,
            text_response("ordinary note saved", "long-write-answer"),
        ),
    ]);
    let config = write_context_window_config(root.path(), &server.base_url, 131_072);
    let output = run_chat(
        root.path(),
        &config,
        "/phase experiment\nSave the ordinary note.\n/exit\n",
        &[],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(root.path().join("note.txt")).unwrap(),
        content
    );
    let project = Project::discover(root.path()).unwrap();
    let state = reduced_session(&project, &session_id(root.path()));
    assert_eq!(state.execution_receipt_count(), 1);
    assert!(state.history().items().iter().any(|item| matches!(item,
        cyber_agent::domain::model::ContentItem::ToolCall { call }
        if call.call_id.as_str() == "long-write" && call.arguments == arguments)));
    assert_eq!(state.history().ensure_all_tool_calls_paired(), Ok(()));
    assert_audits_reconstruct(root.path(), &state);
    let request = f2a_request_body(&server.requests, "file contract wire");
    let tools = request["tools"].as_array().unwrap();
    let write = tools
        .iter()
        .find(|tool| tool["name"] == "write_file")
        .unwrap();
    assert!(
        write["parameters"]["properties"]["expected_hash"]["description"]
            .as_str()
            .unwrap()
            .contains("does NOT exist")
    );
    assert!(
        write["parameters"]["properties"]["content"]["description"]
            .as_str()
            .unwrap()
            .contains("heredoc still requires JSON escaping")
    );
    let patch = tools
        .iter()
        .find(|tool| tool["name"] == "apply_patch")
        .unwrap();
    assert!(
        patch["parameters"]["properties"]["patch"]["description"]
            .as_str()
            .unwrap()
            .contains("--- a/<path>")
    );
    f2a_request_body(&server.requests, "file result wire");
    assert!(server.requests.try_recv().is_err());
    server.join();
}
