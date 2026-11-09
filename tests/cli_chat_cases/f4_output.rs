use super::task_context::{
    ScriptedResponse::{ContinueSelection, InitialSelection, Response},
    initial_selection_response, selector_candidates, spawn_scripted_responses_server,
};
use super::*;

fn reasoning_response() -> String {
    format!(
        "data: {}\n\ndata: {}\n\n{}",
        json!({"type":"response.reasoning_text.delta","delta":"F4_THINKING_MARKER"}),
        json!({"type":"response.output_item.done","item":{"type":"reasoning","content":[{"type":"reasoning_text","text":"F4_THINKING_MARKER"}],"encrypted_content":"OPAQUE_MUST_NOT_DISPLAY"}}),
        text_response("F4_FINAL_MARKER", "f4-response")
    )
}

#[test]
fn f4_default_reasoning_is_separate_from_answer_and_history_in_all_builds() {
    let root = TemporaryDirectory::new("f4-default-reasoning");
    initialize_project(root.path());
    let server = spawn_scripted_responses_server(vec![
        InitialSelection,
        Response(200, reasoning_response()),
    ]);
    let config = write_config(root.path(), &server.base_url);
    let output = run_chat(root.path(), &config, "hello\n/exit\n", &[]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stdout.contains("F4_FINAL_MARKER"));
    assert!(!stdout.contains("F4_THINKING_MARKER"));
    assert_eq!(stderr.matches("[reasoning primary #").count(), 1);
    assert_eq!(stderr.matches("F4_THINKING_MARKER").count(), 1);
    assert!(stderr.contains("[reasoning completed]"));
    assert!(!stderr.contains("[provider-reasoning]"));
    assert!(!stderr.contains("reasoning_delta_uncommitted"));
    assert!(!stderr.contains("[provider-debug]"));
    assert!(!stderr.contains("OPAQUE_MUST_NOT_DISPLAY"));
    assert!(!stderr.contains("loopback-primary-secret"));
    let history = session_events(root.path(), &session_id(root.path()));
    assert!(
        !serde_json::to_string(
            &history
                .iter()
                .filter(|e| e["event_type"] == "session.history_item_recorded")
                .collect::<Vec<_>>()
        )
        .unwrap()
        .contains("F4_THINKING_MARKER")
    );
    server.join();
}

#[test]
fn f4_item_only_reasoning_and_incomplete_stream_are_labeled_without_success() {
    let root = TemporaryDirectory::new("f4-incomplete-reasoning");
    initialize_project(root.path());
    let body = format!(
        "data: {}\n\n",
        json!({"type":"response.output_item.done","item":{"type":"reasoning","content":[{"type":"reasoning_text","text":"F4_ITEM_ONLY"}]}})
    );
    let server = spawn_scripted_responses_server(vec![InitialSelection, Response(200, body)]);
    let config = write_config(root.path(), &server.base_url);
    let output = run_chat(root.path(), &config, "hello\n", &[]);
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("F4_ITEM_ONLY"));
    assert!(stderr.contains("[reasoning interrupted: response not completed]"));
    assert!(!stderr.contains("[reasoning completed]"));
    server.join();
}

#[test]
fn f4_default_shows_raw_rejected_events_without_committing_them() {
    for event in [
        json!({"type":"response.failed","response":{"error":{"code":"server_error","message":"failure","param":"RAW_FAILED_PARAM"}}}),
        json!({"type":"response.incomplete","response":{"incomplete_details":{"reason":"RAW_INCOMPLETE_REASON"}}}),
        json!({"type":"error","error":{"code":"context_length_exceeded","message":"RAW_CAPACITY_DETAIL"}}),
        json!({"type":"response.future","detail":"RAW_PROTOCOL_DETAIL"}),
    ] {
        let root = TemporaryDirectory::new("f4-rejected-event-default");
        initialize_project(root.path());
        let body = format!("data: {event}\n\n");
        let server = spawn_scripted_responses_server(vec![InitialSelection, Response(200, body)]);
        let config = write_config(root.path(), &server.base_url);
        let output = run_chat(root.path(), &config, "hello\n/exit\n", &[]);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("[rejected response]"), "{stderr}");
        for marker in [
            "RAW_FAILED_PARAM",
            "RAW_INCOMPLETE_REASON",
            "RAW_CAPACITY_DETAIL",
            "RAW_PROTOCOL_DETAIL",
        ] {
            if event.to_string().contains(marker) {
                assert!(stderr.contains(marker), "{stderr}");
            }
        }
        let project = Project::discover(root.path()).unwrap();
        let state = reduced_session(&project, &session_id(root.path()));
        assert_eq!(state.execution_receipt_count(), 0);
        assert_eq!(
            state.history().items(),
            &[echidna::domain::model::ContentItem::user_text("hello")]
        );
        server.join();
    }
}

fn debug_requests(stderr: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(stderr)
        .split("[debug sending ")
        .skip(1)
        .map(|section| {
            section
                .split("[debug send finished")
                .next()
                .unwrap()
                .to_owned()
        })
        .collect()
}

fn assert_request_content(view: &str, actual: &Value) {
    // Compare displayed content with the wire; Tool schemas and transport
    // fields are deliberately omitted from the terminal projection.
    fn strings_visible(view: &str, value: &Value) {
        match value {
            Value::String(text) => assert!(view.contains(text), "missing wire content {text:?}"),
            Value::Array(items) => items.iter().for_each(|item| strings_visible(view, item)),
            Value::Object(fields) => fields.values().for_each(|item| strings_visible(view, item)),
            _ => {}
        }
    }
    for key in ["instructions", "input", "text"] {
        if let Some(value) = actual.get(key) {
            strings_visible(view, value);
        }
    }
    assert_eq!(
        view.contains("incremental (this delivery only)"),
        actual.get("previous_response_id").is_some()
    );
}

#[test]
fn f4_reasoning_dedup_requires_exact_delta_text_match() {
    for completed in ["F4_A_F4_B", "F4_DISTINCT_ITEM"] {
        let root = TemporaryDirectory::new("f4-reasoning-dedup");
        initialize_project(root.path());
        let body = format!(
            "data: {}\n\ndata: {}\n\ndata: {}\n\n{}",
            json!({"type":"response.reasoning_text.delta","delta":"F4_A_"}),
            json!({"type":"response.reasoning_text.delta","delta":"F4_B"}),
            json!({"type":"response.output_item.done","item":{"type":"reasoning","content":[{"type":"reasoning_text","text":completed}]}}),
            text_response("F4_FINAL_MARKER", "f4-reasoning-dedup")
        );
        let server = spawn_scripted_responses_server(vec![InitialSelection, Response(200, body)]);
        let config = write_config(root.path(), &server.base_url);
        let output = run_chat(root.path(), &config, "hello\n/exit\n", &[]);
        assert!(output.status.success());
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert_eq!(stderr.matches("F4_A_").count(), 1);
        assert_eq!(stderr.matches("F4_B").count(), 1);
        assert!(stderr.contains("F4_A_F4_B\n"));
        assert_eq!(stderr.matches("[reasoning primary #").count(), 1);
        if completed == "F4_DISTINCT_ITEM" {
            assert_eq!(stderr.matches(completed).count(), 1);
        }
        server.join();
    }
}

#[test]
fn f4_reasoning_text_is_flushed_before_response_completion() {
    assert_reasoning_streams(&[]);
}

#[test]
fn f5_debug_reasoning_also_streams_without_sse_envelopes() {
    assert_reasoning_streams(&["--debug"]);
}

fn assert_reasoning_streams(extra_args: &[&str]) {
    let root = TemporaryDirectory::new("f4-streaming-text");
    initialize_project(root.path());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let config = write_config(
        root.path(),
        &format!("http://{}", listener.local_addr().unwrap()),
    );
    let (release, released) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut selection_socket, _) = listener.accept().unwrap();
        selection_socket.set_read_timeout(Some(F2A_WAIT)).unwrap();
        let selection_wire = request_body(&read_http_request(&mut selection_socket));
        let selection_body = initial_selection_response(&selection_wire, "f4-stream-selection");
        f3_context::write_sse_response(&mut selection_socket, &selection_body);
        drop(selection_socket);
        let (mut socket, _) = listener.accept().unwrap();
        socket.set_read_timeout(Some(F2A_WAIT)).unwrap();
        let _ = read_http_request(&mut socket);
        let start = format!(
            "data: {}\n\n",
            json!({"type":"response.reasoning_text.delta","delta":"F4_STREAM_"})
        );
        let end = format!(
            "data: {}\n\ndata: {}\n\n{}",
            json!({"type":"response.reasoning_text.delta","delta":"VISIBLE\nnext\tline\u{1b}[31m"}),
            json!({"type":"response.output_item.done","item":{"type":"reasoning","content":[{"type":"reasoning_text","text":"F4_STREAM_VISIBLE\nnext\tline\u{1b}[31m"}]}}),
            text_response("F4_FINAL", "f4-stream")
        );
        write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{start}", start.len() + end.len()).unwrap();
        socket.flush().unwrap();
        let _ = released.recv_timeout(F2A_WAIT);
        socket.write_all(end.as_bytes()).unwrap();
    });
    let mut child = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .current_dir(root.path())
        .args(["--config", config.to_str().unwrap(), "chat"])
        .args(extra_args)
        .env("CYBER_AGENT_PRIMARY_API_KEY", "loopback-primary-secret")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"hello\n/exit\n")
        .unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let (chunks, received) = mpsc::channel();
    let reader = thread::spawn(move || {
        let mut all = Vec::new();
        let mut buf = [0; 512];
        loop {
            let n = stderr.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            all.extend_from_slice(&buf[..n]);
            let _ = chunks.send(buf[..n].to_vec());
        }
        all
    });
    let deadline = Instant::now() + F2A_WAIT;
    let mut early = Vec::new();
    while Instant::now() < deadline && !String::from_utf8_lossy(&early).contains("F4_STREAM_") {
        match received.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(chunk) => early.extend(chunk),
            Err(_) => break,
        }
    }
    let _ = release.send(());
    let output = child.wait_with_output().unwrap();
    let stderr = String::from_utf8(reader.join().unwrap()).unwrap();
    server.join().unwrap();
    assert!(output.status.success(), "{stderr}");
    assert!(
        String::from_utf8_lossy(&early).contains("F4_STREAM_"),
        "text was buffered until completion"
    );
    assert!(!String::from_utf8_lossy(&early).contains("[reasoning completed]"));
    assert!(
        stderr.contains("F4_STREAM_VISIBLE\nnext\tline\\u{1b}[31m\n"),
        "{stderr}"
    );
    assert!(!stderr.contains('\u{1b}'));
    assert_eq!(stderr.matches("[reasoning primary #").count(), 1);
    assert!(stderr.contains(if extra_args.is_empty() {
        "[reasoning completed]"
    } else {
        "response completed]"
    }));
    assert!(!stderr.contains("response.reasoning_text.delta"));
}

#[test]
fn f4_debug_shows_unparseable_sse_and_http_bodies() {
    for status in [200, 500] {
        let root = TemporaryDirectory::new("f4-malformed-debug");
        initialize_project(root.path());
        let malformed =
            r#"{"type":"response.output_item.done","item":{"encrypted_content":"OPAQUE_MALFORMED""#;
        let body = if status == 200 {
            format!("data: {malformed}\n\n")
        } else {
            malformed.to_owned()
        };
        let server =
            spawn_scripted_responses_server(vec![InitialSelection, Response(status, body)]);
        let config = write_config(root.path(), &server.base_url);
        let output = run_chat(root.path(), &config, "hello\n", &["--debug"]);
        assert!(!output.status.success());
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains("OPAQUE_MALFORMED"));
        assert!(!stderr.contains("loopback-primary-secret"));
        server.join();
    }
}

#[test]
fn f4_debug_bounds_http_error_capture_without_partial_body_output() {
    let root = TemporaryDirectory::new("f4-bounded-debug");
    initialize_project(root.path());
    let body = json!({"error":{"message":format!("F4_OVERSIZED_BODY{}", "x".repeat(70 * 1024))}})
        .to_string();
    let server = spawn_scripted_responses_server(vec![InitialSelection, Response(500, body)]);
    let config = write_config(root.path(), &server.base_url);
    let output = run_chat(root.path(), &config, "hello\n", &["--debug"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("HTTP error body exceeds the configured read limit"));
    assert!(stderr.contains("HTTP error]\n500"));
    assert!(!stderr.contains("F4_OVERSIZED_BODY"));
    server.join();
}

#[test]
fn f4_debug_matches_actual_full_and_incremental_wire_and_paired_tool_output() {
    let root = TemporaryDirectory::new("f4-debug-wire");
    initialize_project(root.path());
    fs::write(root.path().join("fixture.txt"), "F4_TOOL_RESULT").unwrap();
    let server = spawn_scripted_responses_server(vec![
        InitialSelection,
        Response(
            200,
            tool_calls_response(
                &[("f4-read", "read_file", json!({"path":"fixture.txt"}))],
                "f4-tool",
            ),
        ),
        Response(200, reasoning_response()),
    ]);
    let config = write_config(root.path(), &server.base_url);
    let output = run_chat(
        root.path(),
        &config,
        "F4_USER_PROMPT\n/exit\n",
        &["--debug"],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let requests = debug_requests(&output.stderr);
    assert_eq!(requests.len(), 3);
    let selection = f2a_request_body(&server.requests, "F4 initial selection");
    selector_candidates(&selection);
    let first = f2a_request_body(&server.requests, "F4 first request");
    let second = f2a_request_body(&server.requests, "F4 second request");
    assert_request_content(&requests[0], &selection);
    assert_request_content(&requests[1], &first);
    assert_request_content(&requests[2], &second);
    let tools = first["tools"].as_array().unwrap();
    assert!(
        !tools.is_empty(),
        "debug must preserve actual wire ToolSpecs"
    );
    for request in &requests {
        assert!(request.contains("tool definitions and transport fields omitted"));
        assert!(request.contains("not a verbatim wire capture"));
        assert!(!request.contains("── Tools ──"));
        for tool in tools {
            let description = tool["description"].as_str().unwrap();
            assert!(!description.is_empty());
            assert!(
                !request.contains(description),
                "Tool schema leaked into debug"
            );
        }
    }
    assert_eq!(second["previous_response_id"], "f4-tool");
    assert_ne!(requests[1].lines().next(), requests[2].lines().next());
    let stderr = String::from_utf8_lossy(&output.stderr);
    for marker in [
        "F4_USER_PROMPT",
        "F4_TOOL_RESULT",
        "F4_THINKING_MARKER",
        "model tool call (not executed)",
    ] {
        assert!(stderr.contains(marker), "missing {marker}");
    }
    assert_eq!(
        String::from_utf8_lossy(&output.stdout)
            .matches("F4_FINAL_MARKER")
            .count(),
        1
    );
    assert!(
        !stderr.contains("F4_FINAL_MARKER"),
        "Primary answer must not be duplicated"
    );
    assert!(!stderr.contains("response.output_item.done"));
    assert!(!stderr.contains("[provider-debug]"));
    assert_eq!(stderr.matches("F4_THINKING_MARKER").count(), 1);
    assert!(!stderr.contains("OPAQUE_MUST_NOT_DISPLAY"));
    assert!(!stderr.contains("loopback-primary-secret"));
    server.join();
}

#[test]
fn f4_large_continuation_rejection_preserves_full_fallback() {
    for debug in [false, true] {
        let root = TemporaryDirectory::new("f4-large-continuation");
        initialize_project(root.path());
        let body = json!({"error":{"code":"previous_response_not_found","message":format!("F4_LARGE_REJECTION{}", "x".repeat(70 * 1024))}}).to_string();
        let server = spawn_scripted_responses_server(vec![
            InitialSelection,
            Response(200, text_response("F4_FIRST", "f4-first")),
            ContinueSelection { usage: None },
            Response(400, body),
            Response(200, text_response("F4_RECOVERED", "f4-recovered")),
        ]);
        let config = write_config(root.path(), &server.base_url);
        let args = if debug { &["--debug"][..] } else { &[][..] };
        let output = run_chat(root.path(), &config, "first\nsecond\n/exit\n", args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("F4_RECOVERED"));
        let initial = f2a_request_body(&server.requests, "initial selection");
        selector_candidates(&initial);
        let first = f2a_request_body(&server.requests, "first Full request");
        let selection = f2a_request_body(&server.requests, "continuation selection");
        selector_candidates(&selection);
        let incremental = f2a_request_body(&server.requests, "rejected incremental request");
        let fallback = f2a_request_body(&server.requests, "Full fallback request");
        assert!(first.get("previous_response_id").is_none());
        assert_eq!(incremental["previous_response_id"], "f4-first");
        assert!(fallback.get("previous_response_id").is_none());
        {
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert_eq!(stderr.matches("F4_LARGE_REJECTION").count(), 1);
            if debug {
                let requests = debug_requests(&output.stderr);
                assert_eq!(requests.len(), 5);
                for (record, actual) in
                    requests
                        .iter()
                        .zip([initial, first, selection, incremental, fallback])
                {
                    assert_request_content(record, &actual);
                }
            }
        }
        server.join();
    }
}

#[test]
fn f4_debug_reports_http_error_body_without_printing_credential() {
    let root = TemporaryDirectory::new("f4-debug-http-error");
    initialize_project(root.path());
    let body = json!({"error":{"message":"F4_HTTP_ERROR loopback-primary-secret", "api_key":"other-secret-value"}}).to_string();
    let server = spawn_scripted_responses_server(vec![InitialSelection, Response(500, body)]);
    let config = write_config(root.path(), &server.base_url);
    let output = run_chat(root.path(), &config, "hello\n", &["--debug"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("F4_HTTP_ERROR"));
    assert!(stderr.contains("HTTP error]\n500"));
    assert!(stderr.contains("response interrupted: completion not observed"));
    assert!(stderr.contains("loopback-primary-secret"));
    assert!(stderr.contains("other-secret-value"));
    server.join();
}

#[test]
fn f4_debug_covers_both_image_routes_and_omits_image_bytes() {
    for direct in [false, true] {
        let root = TemporaryDirectory::new("f4-debug-vision");
        initialize_project(root.path());
        append_research_snapshot(&Project::discover(root.path()).unwrap(), "current");
        fs::write(root.path().join("screen.png"), png_bytes(16, 12)).unwrap();
        let primary = spawn_scripted_responses_server(vec![
            InitialSelection,
            Response(200, text_response("F4_IMAGE_DONE", "f4-primary")),
        ]);
        let vision = (!direct).then(|| {
            spawn_responses_server(vec![(
                200,
                vision_response("F4_IMAGE_OBSERVATION", "f4-vision"),
            )])
        });
        let config = match &vision {
            Some(vision) => write_dual_config(root.path(), &primary.base_url, &vision.base_url),
            None => write_direct_config(root.path(), &primary.base_url),
        };
        let output = run_chat(
            root.path(),
            &config,
            "/image screen.png\nDescribe this fixture\n/exit\n",
            &["--debug"],
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let requests = debug_requests(&output.stderr);
        let roles: Vec<_> = requests
            .iter()
            .map(|r| r.split_once(" #").unwrap().0)
            .collect();
        assert_eq!(
            roles,
            if direct {
                vec!["task selection", "primary"]
            } else {
                vec!["vision", "task selection", "primary"]
            }
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("data:image/png;base64,"));
        assert!(!stderr.contains("loopback-primary-secret"));
        assert!(!stderr.contains("loopback-vision-secret"));
        if !direct {
            assert!(stderr.contains("[debug Vision output · pending]"));
            assert!(stderr.contains("F4_IMAGE_OBSERVATION"));
        }
        selector_candidates(&f2a_request_body(&primary.requests, "F4 image selection"));
        let primary_wire = f2a_request_body(&primary.requests, "F4 image Primary wire");
        let wire = match &vision {
            Some(vision) => f2a_request_body(&vision.requests, "F4 Vision wire"),
            None => primary_wire,
        };
        assert!(wire.to_string().contains("data:image/png;base64,"));
        primary.join();
        if let Some(vision) = vision {
            vision.join();
        }
    }
}

#[test]
fn f4_debug_flag_works_in_all_builds() {
    let root = TemporaryDirectory::new("f4-debug-all-builds");
    initialize_project(root.path());
    let server = spawn_scripted_responses_server(vec![
        InitialSelection,
        Response(200, text_response("F4_ALL_BUILDS", "f4-all")),
    ]);
    let config = write_config(root.path(), &server.base_url);
    let output = run_chat(root.path(), &config, "hello\n/exit\n", &["--debug"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("F4_ALL_BUILDS"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("[debug sending primary"));
    server.join();
}

#[test]
fn f4_reasoning_redaction_does_not_merge_distinct_original_items() {
    let root = TemporaryDirectory::new("f4-redaction-equality");
    initialize_project(root.path());
    let body = format!(
        "data: {}\n\ndata: {}\n\n{}",
        json!({"type":"response.reasoning_text.delta","delta":"loopback-primary-secret"}),
        json!({"type":"response.output_item.done","item":{"type":"reasoning","content":[{"type":"reasoning_text","text":"[redacted]"}]}}),
        text_response("F4_FINAL_MARKER", "f4-redaction-equality")
    );
    let server = spawn_scripted_responses_server(vec![InitialSelection, Response(200, body)]);
    let config = write_config(root.path(), &server.base_url);
    let output = run_chat(root.path(), &config, "hello\n/exit\n", &[]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(
        stderr.matches("[redacted]").count(),
        1,
        "distinct original items must remain distinct: {stderr}"
    );
    assert!(stderr.contains("loopback-primary-secret"));
    server.join();
}
