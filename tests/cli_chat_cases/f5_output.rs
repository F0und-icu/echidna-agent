use super::*;

#[test]
fn r5a_assistant_sampling_replies_are_separate_without_changing_history() {
    let root = TemporaryDirectory::new("r5a-reply-boundaries");
    let harness = TemporaryDirectory::new("r5a-reply-config");
    initialize_project(root.path());
    fs::write(root.path().join("fixture.txt"), "fixture").unwrap();
    let first = format!(
        "data: {}\n\ndata: {}\n\n{}",
        json!({"type":"response.output_text.delta","delta":"FIRST_"}),
        json!({"type":"response.output_text.delta","delta":"REPLY"}),
        tool_calls_response(
            &[("read", "read_file", json!({"path":"fixture.txt"}))],
            "step1"
        )
    );
    let server = spawn_responses_server(vec![
        (200, first),
        (200, text_response("SECOND_REPLY", "step2")),
    ]);
    let config = write_config(harness.path(), &server.base_url);
    let output = f2a_wait_child(
        spawn_f2a_chat(root.path(), &config, "inspect fixture\n/exit\n", &[]),
        "reply boundaries",
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Echidna> FIRST_REPLY\n"), "{stdout}");
    assert!(stdout.contains("Echidna> SECOND_REPLY\n"), "{stdout}");
    assert_eq!(stdout.matches("Echidna> ").count(), 2);
    assert!(!stdout.contains('\u{1b}'));
    assert!(!stdout.contains("Question. Observe. Verify."));
    let _ = f2a_request_body(&server.requests, "first step");
    let second = f2a_request_body(&server.requests, "second step");
    assert!(!second.to_string().contains("Echidna>"));
    let history = session_events(root.path(), &session_id(root.path()));
    let items: Vec<_> = history
        .iter()
        .filter(|event| event["event_type"] == "session.history_item_recorded")
        .map(|event| &event["payload"]["item"])
        .collect();
    let history = serde_json::to_string(&items).unwrap();
    assert!(history.contains("FIRST_REPLY"));
    assert!(history.contains("SECOND_REPLY"));
    assert!(!history.contains("Echidna>"));
    f2a_join_server(server, "reply boundaries");
}

#[test]
fn r5a_raw_capture_hints_distinguish_complete_and_unretained_output() {
    for (count, complete, batch) in [(8, true, false), (1025, false, false), (8, true, true)] {
        let root = TemporaryDirectory::new("r5a-raw-hints");
        let harness = TemporaryDirectory::new("r5a-raw-config");
        initialize_project(root.path());
        let command = json!({
            "argv":["sh", "-c", format!("dd if=/dev/zero bs=1024 count={count} 2>/dev/null")]
        });
        let (tool, arguments) = if batch {
            (
                "run_commands",
                json!({"mode":"serial","commands":[command.clone(),command.clone(),command]}),
            )
        } else {
            ("run_command", command)
        };
        let server = spawn_responses_server(vec![
            (
                200,
                tool_calls_response(&[("raw", tool, arguments)], "raw-output"),
            ),
            (200, text_response("DONE", "done")),
        ]);
        let config = write_context_window_config(harness.path(), &server.base_url, 1_000_000);
        let output = f2a_wait_child(
            spawn_f2a_chat(root.path(), &config, "capture bytes\n/exit\n", &[]),
            "raw capture hints",
        );
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(output.status.success(), "{stderr}");
        assert!(stderr.contains("仅显示预览"));
        assert!(!stderr.contains("完整结果保留"));
        let events = session_events(root.path(), &session_id(root.path()));
        let receipt = events
            .iter()
            .find(|event| event["event_type"] == "tool.execution_outcome_recorded")
            .unwrap();
        let capture = &receipt["payload"]["receipt"]["output_captures"][0];
        assert_eq!(capture["complete"], complete);
        let blob = capture["blob_id"].as_str().unwrap();
        assert!(
            stderr.contains(&format!("read_output blob_id={blob} offset=0")),
            "{stderr}"
        );
        let label = if complete {
            "已完整捕获"
        } else {
            "捕获不完整；未保留字节不可回读"
        };
        assert!(stderr.contains(label), "{stderr}");
        if batch {
            assert!(stderr.contains("原始输出 子命令 1 stdout"));
            assert!(stderr.contains("[另有 2 个捕获引用，见 Session 记录]"));
        }
        assert!(stderr.contains(&format!(
            "保留 {}/已观察 {} bytes",
            capture["retained_bytes"],
            count * 1024
        )));
        let _ = f2a_request_body(&server.requests, "capture request");
        let second = f2a_request_body(&server.requests, "capture followup");
        assert!(second.to_string().contains(blob));
        assert!(!second.to_string().contains("未保留字节不可回读"));
        f2a_join_server(server, "raw capture hints");
    }
}

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
            .filter(|event| event["event_type"] == "tool.execution_outcome_recorded")
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
