use super::task_context::{
    ScriptedResponse::{InitialSelection, Response, TruncatedResponse},
    selector_candidates, spawn_scripted_responses_server,
};
use super::*;

fn records(bytes: &[u8]) -> Vec<Value> {
    assert!(bytes.ends_with(b"\n"));
    std::str::from_utf8(bytes)
        .unwrap()
        .lines()
        .map(|line| {
            let record: Value = serde_json::from_str(line).unwrap();
            assert_eq!(record["v"], 1);
            assert!(record["type"].is_string());
            record
        })
        .collect()
}

#[test]
fn external_consumer_reads_chat_tools_and_model_output_in_both_debug_modes() {
    for debug in [false, true] {
        let root = TemporaryDirectory::new("jsonl-consumer");
        initialize_project(root.path());
        fs::write(root.path().join("note.txt"), "a short note").unwrap();
        let answer = "First line.\n引号 \" and emoji 🧪";
        let reply = [
            json!({"type":"response.output_text.delta","delta":"First line.\n"}),
            json!({"type":"response.output_text.delta","delta":"引号 \" and emoji 🧪"}),
            json!({"type":"response.completed","response":{"id":"answer","usage":{"input_tokens":23,"output_tokens":7}}}),
        ].iter().map(|e|format!("data: {e}\n\n")).collect();
        let server = spawn_scripted_responses_server(vec![
            InitialSelection,
            Response(
                200,
                m4_long_flow_tool_response(
                    "read-note",
                    "read_file",
                    json!({"path":"note.txt"}),
                    "read",
                ),
            ),
            Response(200, reply),
        ]);
        let config = write_config(root.path(), &server.base_url);
        let mut args = vec!["--output-format", "jsonl"];
        if debug {
            args.push("--debug");
        }
        let output = run_chat(
            root.path(),
            &config,
            "Read note.txt and report.\n/exit\n",
            &args,
        );
        let wire: Vec<Value> = (0..3)
            .map(|_| f2a_request_body(&server.requests, "JSONL exchange"))
            .collect();
        server.join();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            output.stderr.is_empty(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let events = records(&output.stdout);
        assert!(!events.iter().any(|event| {
            event["type"] == "diagnostic"
                && event["text"]
                    .as_str()
                    .is_some_and(|text| text.contains("[debug timing]"))
        }));
        let select = |kind: &str| {
            events
                .iter()
                .filter(|e| e["type"] == kind)
                .collect::<Vec<_>>()
        };
        assert_eq!(select("session")[0]["session_id"], session_id(root.path()));
        assert!(select("input").iter().any(|e| e["mode"] == "processing"));
        assert_eq!(
            select("assistant_delta")
                .iter()
                .map(|e| e["text"].as_str().unwrap())
                .collect::<String>(),
            answer
        );
        let tools = select("tool");
        assert_eq!(
            tools
                .iter()
                .map(|e| e["stage"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["started", "recorded"]
        );
        assert!(
            tools
                .iter()
                .all(|e| e["call_id"] == "read-note" && e["name"] == "read_file")
        );
        assert_eq!(tools[1]["status"], "succeeded");
        assert_eq!(select("turn")[0]["state"], "completed");
        let usage = select("usage")
            .into_iter()
            .find(|e| e["usage"]["input_tokens"] == 23)
            .unwrap();
        assert_eq!(usage["usage"]["output_tokens"], 7);
        assert!(usage["audit_id"].is_string());
        assert_eq!(!select("timing").is_empty(), debug);
        assert_eq!(
            select("model_log")
                .iter()
                .any(|e| e["text"].as_str().unwrap().contains("debug sending")),
            debug
        );
        assert!(
            select("sample")
                .iter()
                .any(|e| e["role"] == "primary" && e["state"] == "completed")
        );
        let requests = select("model_request");
        let responses = select("model_response");
        if debug {
            assert_eq!(requests.len(), 3);
            for (observed, sent) in requests.iter().zip(&wire) {
                assert_eq!(observed["model"], sent["model"]);
                assert_eq!(observed["content"]["instructions"], sent["instructions"]);
                assert_eq!(observed["content"]["input"], sent["input"]);
                assert_eq!(observed["phase"], "prepared_for_send");
                assert_eq!(observed["truncated"], false);
                assert!(
                    observed["omitted"]
                        .as_array()
                        .unwrap()
                        .contains(&json!("other_request_fields"))
                );
            }
            let tool = responses.iter().find(|e| e["kind"] == "tool_call").unwrap();
            assert_eq!(tool["call"]["call_id"], "read-note");
            assert_eq!(tool["call"]["name"], "read_file");
            assert_eq!(tool["call"]["arguments"], json!({"path":"note.txt"}));
            let answer_id = &requests[2]["request_id"];
            assert_eq!(
                responses
                    .iter()
                    .filter(|e| e["request_id"] == *answer_id && e["kind"] == "text_delta")
                    .map(|e| e["text"].as_str().unwrap())
                    .collect::<String>(),
                answer
            );
        } else {
            assert!(requests.is_empty());
            assert!(responses.is_empty());
        }
        assert!(!String::from_utf8_lossy(&output.stdout).contains("loopback-primary-secret"));
    }
}

#[test]
fn jsonl_debug_omits_image_bytes_from_both_structured_and_log_views() {
    let root = TemporaryDirectory::new("jsonl-debug-image");
    initialize_project(root.path());
    append_research_snapshot(&Project::discover(root.path()).unwrap(), "current");
    fs::write(root.path().join("screen.png"), png_bytes(16, 12)).unwrap();
    let server = spawn_scripted_responses_server(vec![
        InitialSelection,
        Response(200, text_response("image observed", "image-response")),
    ]);
    let config = write_direct_config(root.path(), &server.base_url);
    let output = run_chat(
        root.path(),
        &config,
        "/image screen.png\nDescribe this fixture\n/exit\n",
        &["--output-format", "jsonl", "--debug"],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let events = records(&output.stdout);
    let request = events
        .iter()
        .find(|event| {
            event["type"] == "model_request"
                && event["omitted"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("image_payload"))
        })
        .unwrap();
    assert!(
        request["content"]["input"]
            .to_string()
            .contains("image_payload")
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("data:image/png;base64,"));
    selector_candidates(&f2a_request_body(&server.requests, "image selection"));
    assert!(
        f2a_request_body(&server.requests, "image request")
            .to_string()
            .contains("data:image/png;base64,")
    );
    server.join();
}

#[test]
fn http_error_is_reported_while_sample_records_interrupted_stream() {
    let root = TemporaryDirectory::new("jsonl-http-error");
    initialize_project(root.path());
    let server = spawn_scripted_responses_server(vec![
        InitialSelection,
        Response(401, "fixture denied".to_owned()),
    ]);
    let config = write_config(root.path(), &server.base_url);
    let output = run_chat(
        root.path(),
        &config,
        "Reply.\n/exit\n",
        &["--output-format", "jsonl", "--debug"],
    );
    server.join();
    assert!(!output.status.success());
    let events = records(&output.stdout);
    let request = events
        .iter()
        .find(|e| e["type"] == "model_request" && e["role"] == "primary")
        .unwrap();
    assert!(events.iter().any(|e| e["type"] == "sample"
        && e["request_id"] == request["request_id"]
        && e["state"] == "interrupted"));
    assert!(events.iter().any(|e| e["type"] == "model_response"
        && e["request_id"] == request["request_id"]
        && e["kind"] == "response_status"
        && e["status"] == 401));
    assert!(events.iter().any(|e| e["type"] == "model_response"
        && e["request_id"] == request["request_id"]
        && e["kind"] == "response_error"
        && e["category"] == "http_error"
        && e["text"] == "fixture denied"));
    assert!(
        events.iter().any(
            |e| e["type"] == "diagnostic" && e["text"].as_str().unwrap().contains("status 401")
        ),
        "{events:#?}"
    );
    assert!(events.iter().any(|e| e["type"] == "model_log"
        && e["request_id"] == request["request_id"]
        && e["text"].as_str().unwrap().contains("HTTP error")
        && e["text"].as_str().unwrap().contains("401")));
}

#[test]
fn oversized_http_error_reports_why_the_body_is_omitted() {
    let root = TemporaryDirectory::new("jsonl-oversized-http-error");
    initialize_project(root.path());
    let server = spawn_scripted_responses_server(vec![
        InitialSelection,
        Response(500, "x".repeat(70 * 1024)),
    ]);
    let config = write_config(root.path(), &server.base_url);
    let output = run_chat(
        root.path(),
        &config,
        "Reply.\n/exit\n",
        &["--output-format", "jsonl", "--debug"],
    );
    server.join();
    assert!(!output.status.success());
    let events = records(&output.stdout);
    let request = events
        .iter()
        .find(|e| e["type"] == "model_request" && e["role"] == "primary")
        .unwrap();
    assert!(events.iter().any(|e| e["type"] == "model_response"
        && e["request_id"] == request["request_id"]
        && e["kind"] == "response_error"
        && e["category"] == "omitted"
        && e["text"].as_str().unwrap().contains("exceeds")));
}

#[test]
fn exchange_keeps_retries_and_public_summary_with_their_own_attempts() {
    let root = TemporaryDirectory::new("jsonl-exchange-retry");
    initialize_project(root.path());
    let partial = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"discarded\"}\n\n";
    let final_item = json!({"type":"reasoning","id":"summary-item","status":"completed",
        "content":[],"summary":[{"type":"summary_text","text":"public summary"}]});
    let completed = [
        json!({"type":"response.reasoning_summary_text.delta","output_index":0,"summary_index":0,"item_id":"summary-item","delta":"public "}),
        json!({"type":"response.reasoning_summary_text.done","output_index":0,"summary_index":0,"item_id":"summary-item","text":"public summary"}),
        json!({"type":"response.output_item.done","output_index":0,"item":final_item}),
        json!({"type":"response.output_text.done","output_index":1,"content_index":0,"item_id":"answer-item","text":"actual answer"}),
        json!({"type":"response.completed","response":{"id":"complete","usage":{"input_tokens":4,"output_tokens":2}}}),
    ].iter().map(|event| format!("data: {event}\n\n")).collect();
    let server = spawn_scripted_responses_server(vec![
        InitialSelection,
        TruncatedResponse(partial.to_owned()),
        Response(200, completed),
    ]);
    let config = write_config(root.path(), &server.base_url);
    let output = run_chat(
        root.path(),
        &config,
        "Reply.\n/exit\n",
        &["--output-format", "jsonl", "--debug"],
    );
    server.join();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let events = records(&output.stdout);
    let requests: Vec<_> = events
        .iter()
        .filter(|e| e["type"] == "model_request" && e["role"] == "primary")
        .collect();
    assert_eq!(requests.len(), 2);
    assert_ne!(requests[0]["request_id"], requests[1]["request_id"]);
    assert_eq!(requests[0]["content"], requests[1]["content"]);
    for (request, terminal) in requests.iter().zip(["interrupted", "completed"]) {
        let id = &request["request_id"];
        let states: Vec<_> = events
            .iter()
            .filter(|e| e["type"] == "sample" && e["request_id"] == *id)
            .map(|e| e["state"].as_str().unwrap())
            .collect();
        assert_eq!(states, ["started", terminal]);
    }
    let second_id = &requests[1]["request_id"];
    let summaries: Vec<_> = events
        .iter()
        .filter(|e| {
            e["type"] == "model_response"
                && e["request_id"] == *second_id
                && e["kind"] == "reasoning_summary"
        })
        .collect();
    assert_eq!(
        summaries
            .iter()
            .map(|e| e["text"].as_str().unwrap())
            .collect::<String>(),
        "public summary"
    );
    assert_eq!(
        summaries
            .iter()
            .map(|e| e["starts_part"].as_bool().unwrap())
            .collect::<Vec<_>>(),
        [true, false]
    );
    assert!(events.iter().any(|e| e["type"] == "model_response"
        && e["request_id"] == *second_id
        && e["kind"] == "text_delta"
        && e["text"] == "actual answer"));
    let state = reduced_session(
        &Project::discover(root.path()).unwrap(),
        &session_id(root.path()),
    );
    assert!(!format!("{:?}", state.history()).contains("public summary"));
}

#[test]
fn startup_failure_is_one_error_record_and_a_failing_exit_code() {
    let output = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .args(["chat", "--output-format", "jsonl"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let events = records(&output.stdout);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["type"], "error");
    assert!(events[0]["text"].as_str().unwrap().contains("--config"));
}

#[test]
fn congested_stream_counts_drops_and_preserves_child_receipts() {
    let output = super::ctf_progress::unread_previews(&["--output-format", "jsonl"]);
    let events = records(&output.stdout);
    let skipped = events
        .iter()
        .filter(|e| e["type"] == "previews_skipped")
        .map(|e| e["count"].as_u64().unwrap())
        .sum::<u64>();
    assert!(skipped > 0);
    let visible = events
        .iter()
        .filter(|e| e["type"] == "tool" && e["call_id"] == "slow")
        .count();
    assert_eq!(visible as u64 + skipped, 34); // start, 32 child checkpoints, terminal receipt
}

#[test]
fn rejected_primary_sample_never_reports_a_completed_turn() {
    for debug in [false, true] {
        let root = TemporaryDirectory::new("jsonl-rejected");
        initialize_project(root.path());
        let body = [
        json!({"type":"response.output_text.delta","output_index":0,"content_index":0,"delta":"draft"}),
        json!({"type":"response.output_text.done","output_index":0,"content_index":0,"text":"conflict"}),
        json!({"type":"response.completed","response":{"id":"unaccepted"}}),
    ].iter().map(|e| format!("data: {e}\n\n")).collect();
        let server = spawn_scripted_responses_server(vec![InitialSelection, Response(200, body)]);
        let config = write_config(root.path(), &server.base_url);
        let args = if debug {
            vec!["--output-format", "jsonl", "--debug"]
        } else {
            vec!["--output-format", "jsonl"]
        };
        let output = run_chat(root.path(), &config, "Reply.\n/exit\n", &args);
        server.join();
        assert!(!output.status.success());
        let events = records(&output.stdout);
        let primary: Vec<_> = events
            .iter()
            .filter(|e| e["type"] == "sample" && e["role"] == "primary")
            .collect();
        assert_eq!(primary.len(), 2);
        assert_eq!(primary[0]["state"], "started");
        assert_eq!(primary[1]["state"], "rejected");
        assert_eq!(primary[0]["request_id"], primary[1]["request_id"]);
        assert_eq!(
            events.iter().any(|e| e["type"] == "model_response"
                && e["request_id"] == primary[0]["request_id"]
                && e["kind"] == "text_delta"
                && e["text"] == "draft"),
            debug
        );
        if debug {
            let error_index = events
                .iter()
                .position(|e| {
                    e["type"] == "model_response"
                        && e["request_id"] == primary[0]["request_id"]
                        && e["kind"] == "response_error"
                        && e["category"] == "protocol_rejected"
                })
                .unwrap();
            let terminal_index = events
                .iter()
                .position(|e| {
                    e["type"] == "sample"
                        && e["request_id"] == primary[0]["request_id"]
                        && e["state"] == "rejected"
                })
                .unwrap();
            assert!(error_index < terminal_index);
        }
        assert!(
            events
                .iter()
                .any(|e| e["type"] == "turn" && e["state"] == "stopped")
        );
        assert!(
            !events
                .iter()
                .any(|e| e["type"] == "turn" && e["state"] == "completed")
        );
        assert_eq!(events.last().unwrap()["type"], "error");
    }
}

#[test]
fn external_client_receives_ready_before_writing_to_stdin() {
    let root = TemporaryDirectory::new("jsonl-ready");
    initialize_project(root.path());
    let config = write_config(root.path(), "http://127.0.0.1:1");
    let mut child = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .current_dir(root.path())
        .arg("--config")
        .arg(config)
        .args(["chat", "--output-format", "jsonl"])
        .env("CYBER_AGENT_PRIMARY_API_KEY", "fixture-key")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    let reader = thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let e: Value = serde_json::from_str(&line.unwrap()).unwrap();
            tx.send(e).unwrap();
        }
    });
    let ready = rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(ready["type"], "session");
    let prompt = rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(
        (prompt["type"].as_str(), prompt["mode"].as_str()),
        (Some("input"), Some("task"))
    );
    child.stdin.take().unwrap().write_all(b"/exit\n").unwrap();
    let output = child.wait_with_output().unwrap();
    reader.join().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        rx.try_iter()
            .any(|e| e["type"] == "message" && e["text"].as_str().unwrap().contains("resume"))
    );
}
