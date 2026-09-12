use super::*;

#[test]
fn f5_tool_views_show_directory_results_partial_failures_and_bounded_text() {
    let root = TemporaryDirectory::new("f5-tool-views");
    initialize_project(root.path());
    fs::write(
        root.path().join("small.txt"),
        "visible file content\nnext line",
    )
    .unwrap();
    fs::write(
        root.path().join("large.txt"),
        format!(
            "VISIBLE_START\u{1b}[31m\n{}HIDDEN_PREVIEW_TAIL",
            "界".repeat(6000)
        ),
    )
    .unwrap();
    let server = spawn_responses_server(vec![
        // Both byte and line preview limits leave the original result intact.
        (
            200,
            tool_calls_response(
                &[
                    ("f5-list", "list_files", json!({"path":".", "max_depth":1})),
                    (
                        "f5-read",
                        "read_files",
                        json!({"files":[{"path":"small.txt"},{"path":"missing.txt"}]}),
                    ),
                    ("f5-large", "read_file", json!({"path":"large.txt"})),
                    ("f5-lines", "read_file", json!({"path":"lines.txt"})),
                ],
                "f5-tools",
            ),
        ),
        (200, text_response("done", "f5-complete")),
    ]);
    fs::write(
        root.path().join("lines.txt"),
        format!("LINES_START\n{}HIDDEN_LINES_TAIL", "line\n".repeat(100)),
    )
    .unwrap();
    let config = write_config(root.path(), &server.base_url);
    let output = run_chat(root.path(), &config, "inspect fixtures\n/exit\n", &[]);
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(output.status.success(), "{stderr}");
    for marker in [
        "· list_files · 完成",
        "paths:",
        "small.txt",
        "large.txt",
        "· read_files · 完成（部分失败）",
        "missing.txt",
        "status: failed",
        "visible file content\nnext line",
        "VISIBLE_START\\u{1b}[31m",
        "LINES_START",
        "仅显示预览",
    ] {
        assert!(stderr.contains(marker), "missing {marker}: {stderr}");
    }
    assert!(!stderr.contains('\u{1b}'));
    assert!(!stderr.contains("HIDDEN_PREVIEW_TAIL"));
    assert!(!stderr.contains("HIDDEN_LINES_TAIL"));
    assert!(!stderr.contains("execution_id="));
    assert!(stderr.len() < 16000, "preview must be bounded");
    let _ = f2a_request_body(&server.requests, "initial request");
    let followup = f2a_request_body(&server.requests, "Tool result request");
    // UI truncation must not mutate actual Tool results sent to the Provider.
    assert!(followup.to_string().contains("HIDDEN_PREVIEW_TAIL"));
    assert!(followup.to_string().contains("HIDDEN_LINES_TAIL"));
    assert_eq!(
        session_events(root.path(), &session_id(root.path()))
            .iter()
            .filter(|event| event["event_type"] == "tool.execution_receipt_recorded")
            .count(),
        4
    );
    server.join();
}

#[test]
fn f5_million_token_mapping_reaches_formal_chat_without_32k_working_limit() {
    let root = TemporaryDirectory::new("f5-large-context");
    initialize_project(root.path());
    let server = spawn_responses_server(vec![(
        200,
        text_response("large accepted", "f5-large-window"),
    )]);
    let config = write_config(root.path(), &server.base_url);
    let encoded = fs::read_to_string(&config)
        .unwrap()
        .replace("context_window = 32768", "context_window = 1048576");
    fs::write(&config, encoded).unwrap();
    let prompt = format!("F5_LARGE_INPUT{}", "x".repeat(150000));
    let output = run_chat(root.path(), &config, &format!("{prompt}\n/exit\n"), &[]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let wire = f2a_request_body(&server.requests, "million-window request");
    assert!(wire["input"].as_array().unwrap().iter().any(|item| {
        item["content"]
            .as_array()
            .is_some_and(|items| items.iter().any(|content| content["text"] == prompt))
    }));
    server.join();
}
