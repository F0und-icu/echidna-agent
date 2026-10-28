use super::*;

#[test]
fn failed_request_capture_normal_cli_tracks_physical_retries_and_uses_actual_cwd() {
    let root = TemporaryDirectory::new("failure-capture-cli");
    let project = root.path().join("project");
    fs::create_dir(&project).unwrap();
    initialize_project(&project);
    let partial =
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"discarded fragment\"}\n\n";
    let complete = text_response("only complete answer", "capture-cli-complete");
    let server = spawn_responses_server_with_lengths(vec![
        (200, partial.to_owned(), partial.len() + 1),
        (200, partial.to_owned(), partial.len() + 1),
        (200, complete.clone(), complete.len()),
    ]);
    let config = write_config(&project, &server.base_url);
    let output = f2a_wait_child(
        spawn_f2a_chat(
            &project,
            &config,
            "fixture input 🐾\n/exit\n",
            &["--debug-failures", "../debug-failures"],
        ),
        "failed-request capture CLI",
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stdout}\n{stderr}");
    assert_eq!(
        stderr.matches("saved request diagnostic snapshot").count(),
        2,
        "{stderr}"
    );
    assert!(stdout.contains("only complete answer"));
    assert!(stdout.contains("admitted=3 primary=3 vision=0"));
    assert!(!project.join("debug-failures").exists());
    let requests: Vec<_> = (0..3)
        .map(|_| f2a_request_body(&server.requests, "captured retry request"))
        .collect();
    f2a_join_server(server, "captured retries");
    assert!(requests.windows(2).all(|pair| pair[0] == pair[1]));
    let captures: Vec<Value> = fs::read_dir(root.path().join("debug-failures"))
        .unwrap()
        .map(|entry| serde_json::from_slice(&fs::read(entry.unwrap().path()).unwrap()).unwrap())
        .collect();
    assert_eq!(captures.len(), 2);
    for capture in captures {
        assert_eq!(capture["role"], "primary");
        assert_eq!(capture["request"]["body"], requests[0]);
        assert_eq!(
            capture["request"]["original_encoded_body_bytes"],
            serde_json::to_vec(&requests[0]).unwrap().len()
        );
        assert_eq!(
            capture["request"]["headers"],
            json!({"accept":"text/event-stream", "content-type":"application/json"})
        );
        assert_eq!(
            serde_json::from_str::<Value>(capture["request"]["encoded_body"].as_str().unwrap())
                .unwrap(),
            requests[0]
        );
    }
    let state = reduced_session(&Project::discover(&project).unwrap(), &session_id(&project));
    assert_eq!(state.request_audits().len(), 3);
    assert_eq!(state.execution_receipt_count(), 0);
    assert_eq!(state.history().items().len(), 2);
    assert_audits_reconstruct(&project, &state);
    let help = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .args(["chat", "--help"])
        .output()
        .unwrap();
    let help = String::from_utf8(help.stdout).unwrap();
    assert!(help.contains("--debug-failures <DIR>"));
    assert!(
        help.lines()
            .any(|line| line.split_whitespace().next() == Some("--debug"))
    );
}

#[test]
fn failed_request_capture_cli_io_error_is_visible_and_preserves_http_failure() {
    let root = TemporaryDirectory::new("failure-capture-io-cli");
    initialize_project(root.path());
    fs::write(root.path().join("ordinary-file"), "preserved").unwrap();
    let server = spawn_responses_server(vec![(401, "RESPONSE_BODY_CANARY".to_owned())]);
    let config = write_config(root.path(), &server.base_url);
    let output = f2a_wait_child(
        spawn_f2a_chat(
            root.path(),
            &config,
            "fixture\n/exit\n",
            &["--debug-failures", "ordinary-file"],
        ),
        "capture IO failure",
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "{stdout}\n{stderr}");
    assert!(
        stderr.contains("[debug-failures] capture failed ("),
        "{stderr}"
    );
    assert!(
        stderr.contains("original Provider error preserved"),
        "{stderr}"
    );
    assert!(
        stderr.contains("model HTTP request failed with status 401"),
        "{stderr}"
    );
    assert!(stderr.contains("RESPONSE_BODY_CANARY"));
    assert!(!stderr.contains("loopback-primary-secret"));
    assert_eq!(
        fs::read_to_string(root.path().join("ordinary-file")).unwrap(),
        "preserved"
    );
    f2a_request_body(&server.requests, "failed HTTP attempt");
    assert!(server.requests.try_recv().is_err());
    f2a_join_server(server, "capture IO failure");
    let state = reduced_session(
        &Project::discover(root.path()).unwrap(),
        &session_id(root.path()),
    );
    assert_eq!(state.request_audits().len(), 1);
    assert_eq!(state.execution_receipt_count(), 0);
    assert_eq!(state.history().items().len(), 1);
    assert_audits_reconstruct(root.path(), &state);
}

#[test]
fn request_send_diagnostics_retry_only_header_closure_without_replaying_a_completed_tool() {
    for (failure, cause) in [
        (
            "HTTP/1.1 200 OK\r\nX-Private: CLI_HEADER_CANARY\r\n",
            "incomplete_message",
        ),
        (
            "HTTP/1.1 200 OK\r\ninvalid header name: CLI_HEADER_CANARY\r\n\r\nCLI_BODY_CANARY",
            "parse",
        ),
    ] {
        let failed_attempts = if cause == "incomplete_message" { 3 } else { 1 };
        for flags in [&[][..], &["--debug"][..]] {
            let root = TemporaryDirectory::new("request-send-diagnostics-cli");
            initialize_project(root.path());
            fs::write(
                root.path().join("target.txt"),
                "已完成工具 CLI_TOOL_BODY_CANARY 🐾",
            )
            .unwrap();
            let completed_tool = m4_long_flow_tool_response(
                "completed-before-send-failure",
                "read_file",
                json!({"path":"target.txt"}),
                "completed-before-send-failure",
            );
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let base_url = format!("http://{}/CLI_URL_CANARY", listener.local_addr().unwrap());
            let (sender, requests) = mpsc::channel();
            let worker = thread::spawn(move || {
                let first = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{completed_tool}",
                    completed_tool.len()
                );
                for response in std::iter::once(first)
                    .chain(std::iter::repeat_n(failure.to_owned(), failed_attempts))
                {
                    let (mut socket, _) = listener.accept().unwrap();
                    socket
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    sender.send(read_http_request(&mut socket)).unwrap();
                    socket.write_all(response.as_bytes()).unwrap();
                    socket.flush().unwrap();
                }
            });
            let config = write_config(root.path(), &base_url);
            let output = f2a_wait_child(
                spawn_f2a_chat(root.path(), &config, "read target.txt\n/exit\n", flags),
                "send diagnostic CLI",
            );
            worker.join().unwrap();
            let captured: Vec<_> = requests.try_iter().collect();
            assert_eq!(captured.len(), 1 + failed_attempts);
            assert!(
                captured[1..]
                    .windows(2)
                    .all(|pair| pair[0].split_once("\r\n\r\n").unwrap().1
                        == pair[1].split_once("\r\n\r\n").unwrap().1)
            );
            let sent_body = captured[1].split_once("\r\n\r\n").unwrap().1;
            assert!(sent_body.contains("CLI_TOOL_BODY_CANARY"));
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(!output.status.success(), "{stdout}\n{stderr}");
            let diagnostic = stderr
                .lines()
                .find(|line| line.contains("[turn stopped]"))
                .unwrap();
            for expected in [
                if cause == "incomplete_message" {
                    "model connection failed"
                } else {
                    "model transport failed"
                },
                "phase=send_or_headers",
                "elapsed_ms=",
                "reqwest_kind=request",
                "io_kind=None",
                &format!("http_cause={cause}"),
                &format!("request_body_bytes={}", sent_body.len()),
            ] {
                assert!(diagnostic.contains(expected), "{stderr}");
            }
            assert!(diagnostic.len() < 400, "{diagnostic}");
            for excluded in [
                "loopback-primary-secret",
                "CLI_URL_CANARY",
                "CLI_HEADER_CANARY",
                "CLI_BODY_CANARY",
                &base_url,
            ] {
                assert!(!stderr.contains(excluded), "{stderr}");
            }
            assert!(!diagnostic.contains("CLI_TOOL_BODY_CANARY"));
            assert_eq!(
                stdout
                    .matches("[response interrupted; partial primary output discarded; retry ")
                    .count(),
                0,
                "header failures have no partial response to discard"
            );
            let attempts = 1 + failed_attempts;
            assert!(stdout.contains(&format!("admitted={attempts} primary={attempts} vision=0")));
            assert!(stdout.contains("Durable checkpoint retained"));
            let state = reduced_session(
                &Project::discover(root.path()).unwrap(),
                &session_id(root.path()),
            );
            assert_eq!(state.request_audits().len(), attempts);
            assert_eq!(state.execution_receipt_count(), 1);
            assert_eq!(state.history().ensure_all_tool_calls_paired(), Ok(()));
            assert!(!state.history().items().iter().any(|item| matches!(
                item,
                echidna::domain::model::ContentItem::Text {
                    role: echidna::domain::model::MessageRole::Assistant,
                    ..
                }
            )));
            assert_audits_reconstruct(root.path(), &state);
        }
    }
}

#[test]
fn error_event_details_reach_normal_cli_without_committing_tools_or_retrying() {
    let root = TemporaryDirectory::new("error-event-cli");
    initialize_project(root.path());
    let events = [
        json!({"type":"response.output_item.done","item":{
            "type":"function_call","call_id":"staged-error-call","name":"write_file",
            "arguments":"{\"path\":\"must-not-exist.txt\",\"content\":\"uncommitted\",\"expected_hash\":null}"
        }}),
        json!({"type":"error","code":"server_error",
            "message":"backend overloaded loopback-primary-secret\u{1b}[31m",
            "param":"input loopback-primary-secret","sequence_number":19,
            "unknown":"RAW_ERROR_EVENT_CANARY"}),
    ];
    let body = events
        .iter()
        .map(|event| format!("data: {event}\n\n"))
        .collect();
    let server = spawn_responses_server(vec![(200, body)]);
    let config = write_config(root.path(), &server.base_url);
    let output = f2a_wait_child(
        spawn_f2a_chat(root.path(), &config, "fixture task\n/exit\n", &[]),
        "normal error event",
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "{stdout}\n{stderr}");
    assert!(stderr.contains("Session has an unresolved stopped Turn"));
    for expected in [
        "[turn stopped] model Provider failed",
        "server_error",
        "Responses SSE error",
        "origin=top-level",
        "backend overloaded loopback-primary-secret",
        "param=input loopback-primary-secret",
        "sequence_number=19",
        "\\u001b[31m",
        "RAW_ERROR_EVENT_CANARY",
        "[rejected response]",
    ] {
        assert!(stderr.contains(expected), "{stderr}");
    }
    assert!(
        !stderr.contains("unsupported Responses event"),
        "{stderr:?}"
    );
    assert!(stdout.contains("admitted=1 primary=1 vision=0"));
    assert!(stdout.contains("Durable checkpoint retained"));
    assert!(!stderr.contains("retry"));
    assert!(!root.path().join("must-not-exist.txt").exists());
    f2a_request_body(&server.requests, "one failed error-event request");
    assert!(server.requests.try_recv().is_err());
    f2a_join_server(server, "error-event CLI");
    let project = Project::discover(root.path()).unwrap();
    let state = reduced_session(&project, &session_id(root.path()));
    assert_eq!(state.request_audits().len(), 1);
    assert_eq!(state.execution_receipt_count(), 0);
    assert_eq!(
        state.history().items(),
        &[echidna::domain::model::ContentItem::user_text(
            "fixture task"
        )]
    );
    assert_audits_reconstruct(root.path(), &state);
}

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
        (200, interrupted.clone(), interrupted.len() + 1),
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
    assert!(
        stderr.contains("[turn stopped] model response interrupted: status=200"),
        "{stderr}"
    );
    assert!(!stderr.contains("invalid SSE stream: Transport error:"));
    for expected in [
        "elapsed_ms=",
        "timeout=false",
        "io_kind=Some(UnexpectedEof)",
    ] {
        assert!(stderr.contains(expected), "{stderr}");
    }
    assert!(stdout.contains("Task input not accepted or saved"));
    assert!(stdout.contains("explicit recovery completed"));
    let completed = f2a_request_body(&server.requests, "completed tool sample");
    let interrupted_first = f2a_request_body(&server.requests, "interrupted followup first");
    let interrupted_second = f2a_request_body(&server.requests, "interrupted followup second");
    let interrupted_third = f2a_request_body(&server.requests, "interrupted followup third");
    let resumed = f2a_request_body(&server.requests, "explicit recovery");
    assert!(completed.to_string().contains("original task"));
    assert_eq!(interrupted_first, interrupted_second);
    assert_eq!(interrupted_second, interrupted_third);
    assert_eq!(
        interrupted_first["previous_response_id"],
        "completed-response"
    );
    assert!(resumed.get("previous_response_id").is_none());
    for request in [
        &interrupted_first,
        &interrupted_second,
        &interrupted_third,
        &resumed,
    ] {
        assert!(request.to_string().contains("durable tool result"));
        for excluded in ["uncommitted fragment", "uncommitted-call", "blocked input"] {
            assert!(!request.to_string().contains(excluded));
        }
    }
    assert!(server.requests.try_recv().is_err());
    f2a_join_server(server, "body transport recovery");
    assert!(!root.path().join("must-not-exist.txt").exists());
    let state = reduced_session(
        &Project::discover(root.path()).unwrap(),
        &session_id(root.path()),
    );
    assert_eq!(state.request_audits().len(), 5);
    assert_eq!(state.execution_receipt_count(), 1);
    assert_eq!(state.history().ensure_all_tool_calls_paired(), Ok(()));
    assert_eq!(
        state.history().items().last().unwrap(),
        &echidna::domain::model::ContentItem::assistant_text("explicit recovery completed")
    );
    assert_audits_reconstruct(root.path(), &state);
}
