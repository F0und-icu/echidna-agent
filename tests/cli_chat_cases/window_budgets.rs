use super::*;

use sha2::{Digest, Sha256};

#[test]
fn explicit_large_window_extends_core_memory_schema_and_execution_together() {
    let root = TemporaryDirectory::new("large-window-core-memory");
    let harness = TemporaryDirectory::new("large-window-core-memory-provider");
    initialize_project(root.path());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let config = write_context_window_config(harness.path(), &base_url, 1_048_576);
    let content = "m".repeat(64 * 1024 + 1);
    let expected_content = content.clone();
    let (sender, requests) = mpsc::channel();
    let server = thread::spawn(move || {
        for index in 0..3 {
            let mut socket = f3_context::accept_before_deadline(&listener, "large window");
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            sender.send(read_http_request(&mut socket)).unwrap();
            let response = match index {
                0 => m4_long_flow_tool_response(
                    "find-memory-write",
                    "tool_search",
                    json!({"query":"memory_write"}),
                    "large-window-search",
                ),
                1 => m4_long_flow_tool_response(
                    "write-large-core",
                    "memory_write",
                    json!({
                        "path":"core.md",
                        "content":content,
                        "expected_hash":null,
                        "event_id":"large-window-memory-write",
                        "occurred_at":"2026-10-22T00:00:00Z"
                    }),
                    "large-window-write",
                ),
                2 => text_response("large window complete", "large-window-complete"),
                _ => unreachable!(),
            };
            f3_context::write_sse_response(&mut socket, &response);
        }
    });

    let output = f2a_wait_child(
        spawn_f2a_chat(
            root.path(),
            &config,
            "record large core memory\n/exit\n",
            &[],
        ),
        "large-window core memory",
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let wires = (0..3)
        .map(|_| f2a_request_body(&requests, "large-window request"))
        .collect::<Vec<_>>();
    server.join().unwrap();
    let memory_write = wires[1]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "memory_write")
        .unwrap();
    assert_eq!(
        memory_write["parameters"]["allOf"][0]["then"]["properties"]["content"]["maxLength"],
        json!(512 * 1024)
    );
    assert_eq!(
        fs::read_to_string(root.path().join(".echidna/memory/core.md")).unwrap(),
        expected_content
    );
}

#[test]
fn explicit_large_window_reads_extended_core_memory_during_context_refresh() {
    let root = TemporaryDirectory::new("large-window-core-memory-refresh");
    let harness = TemporaryDirectory::new("large-window-core-memory-refresh-provider");
    initialize_project(root.path());
    let content = "c".repeat(64 * 1024 + 1);
    let memory = root.path().join(".echidna/memory/core.md");
    fs::create_dir_all(memory.parent().unwrap()).unwrap();
    fs::write(&memory, &content).unwrap();

    let server = spawn_responses_server(vec![(
        200,
        text_response(
            "large window memory refresh complete",
            "large-window-memory-refresh",
        ),
    )]);
    let config = write_context_window_config(harness.path(), &server.base_url, 1_048_576);
    let output = f2a_wait_child(
        spawn_f2a_chat(
            root.path(),
            &config,
            "read extended core memory\n/exit\n",
            &[],
        ),
        "large-window Core Memory refresh",
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let request = f2a_request_body(&server.requests, "large-window Core Memory refresh");
    assert!(
        request["input"].to_string().contains(&content),
        "extended Core Memory was not delivered"
    );
    f2a_join_server(server, "large-window Core Memory refresh");
}

#[test]
fn explicit_large_window_extends_read_output_schema_default_and_execution_together() {
    let root = TemporaryDirectory::new("large-window-read-output");
    let harness = TemporaryDirectory::new("large-window-read-output-provider");
    initialize_project(root.path());
    let bytes = vec![b'r'; 160 * 1024];
    let blob_id = format!("{:x}", Sha256::digest(&bytes));
    let blobs = root.path().join(".echidna/artifacts/blobs/sha256");
    fs::create_dir_all(&blobs).unwrap();
    fs::write(blobs.join(&blob_id), bytes).unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let config = write_context_window_config(harness.path(), &base_url, 1_048_576);
    let (sender, requests) = mpsc::channel();
    let server = thread::spawn(move || {
        for index in 0..3 {
            let mut socket = f3_context::accept_before_deadline(&listener, "large window readback");
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            sender.send(read_http_request(&mut socket)).unwrap();
            let response = match index {
                0 => m4_long_flow_tool_response(
                    "find-read-output",
                    "tool_search",
                    json!({"query":"read_output"}),
                    "large-window-readback-search",
                ),
                1 => m4_long_flow_tool_response(
                    "read-large-output",
                    "read_output",
                    json!({"blob_id":blob_id,"offset":0}),
                    "large-window-readback",
                ),
                2 => text_response(
                    "large window readback complete",
                    "large-window-readback-complete",
                ),
                _ => unreachable!(),
            };
            f3_context::write_sse_response(&mut socket, &response);
        }
    });

    let output = f2a_wait_child(
        spawn_f2a_chat(root.path(), &config, "read retained output\n/exit\n", &[]),
        "large-window readback",
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let wires = (0..3)
        .map(|_| f2a_request_body(&requests, "large-window readback request"))
        .collect::<Vec<_>>();
    server.join().unwrap();
    let read_output = wires[1]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "read_output")
        .unwrap();
    assert_eq!(
        read_output["parameters"]["properties"]["max_bytes"]["maximum"],
        json!(128 * 1024)
    );
    let delivered = wires[2]["input"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| {
            item["type"] == "function_call_output" && item["call_id"] == "read-large-output"
        })
        .unwrap();
    let delivered: Value = serde_json::from_str(delivered["output"].as_str().unwrap()).unwrap();
    assert_eq!(delivered["output"]["next_offset"], json!(32 * 1024));
    assert_eq!(delivered["output"]["has_more"], true);
}

#[test]
fn explicit_large_window_extends_http_preview_schema_and_execution_together() {
    let root = TemporaryDirectory::new("large-window-http-preview");
    let harness = TemporaryDirectory::new("large-window-http-preview-provider");
    initialize_project(root.path());
    let http_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    http_listener.set_nonblocking(true).unwrap();
    let http_url = format!("http://{}/preview", http_listener.local_addr().unwrap());
    let response_body = "p".repeat(128 * 1024);
    let http_response_body = response_body.clone();
    let http_server = thread::spawn(move || {
        let mut socket = f3_context::accept_before_deadline(&http_listener, "large HTTP preview");
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let request = read_http_request(&mut socket);
        assert!(request.starts_with("POST /preview? HTTP/1.1"), "{request}");
        write!(
            socket,
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            http_response_body.len(),
            http_response_body
        )
        .unwrap();
    });

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let config = write_context_window_config(harness.path(), &base_url, 1_048_576);
    let (sender, requests) = mpsc::channel();
    let server = thread::spawn(move || {
        for index in 0..4 {
            let mut socket =
                f3_context::accept_before_deadline(&listener, "large-window HTTP preview");
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            sender.send(read_http_request(&mut socket)).unwrap();
            let response = match index {
                0 => m4_long_flow_tool_response(
                    "record-preview-target",
                    "research_update",
                    json!({
                        "event_id":"large-window-preview-target",
                        "occurred_at":"2026-10-22T00:00:00Z",
                        "operation":{"kind":"record_target_snapshot","record":{
                            "id":"large-window-preview-target",
                            "target_id":"fixture.target",
                            "kind":"web",
                            "locator":"http://fixture.invalid",
                            "version":"1",
                            "build_id":"1",
                            "target_artifact_ids":["artifact-root"],
                            "environment_fingerprint":"fixture-environment",
                            "device_fingerprint":"fixture-device",
                            "captured_at":"2026-10-22T00:00:00Z"
                        }}
                    }),
                    "large-window-preview-target-response",
                ),
                1 => m4_long_flow_tool_response(
                    "find-http-request",
                    "tool_search",
                    json!({"query":"http_request"}),
                    "large-window-preview-search",
                ),
                2 => m4_long_flow_tool_response(
                    "preview-large-response",
                    "http_request",
                    json!({
                        "method":"post",
                        "url":http_url,
                        "body":{"kind":"text","value":"fixture"},
                        "preview_bytes":128 * 1024,
                        "capture_bytes":128 * 1024,
                        "artifact":{
                            "event_id":"large-window-preview-artifact",
                            "occurred_at":"2026-10-22T00:00:01Z",
                            "artifact_id":"large-window-preview-artifact",
                            "target_snapshot_id":"large-window-preview-target",
                            "toolchain_fingerprint":"large-window-preview-fixture"
                        }
                    }),
                    "large-window-preview-response",
                ),
                3 => text_response(
                    "large HTTP preview complete",
                    "large-window-preview-complete",
                ),
                _ => unreachable!(),
            };
            f3_context::write_sse_response(&mut socket, &response);
        }
    });

    let output = f2a_wait_child(
        spawn_f2a_chat(
            root.path(),
            &config,
            "preview retained HTTP output\n/exit\n",
            &[],
        ),
        "large-window HTTP preview",
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let wires = (0..4)
        .map(|_| f2a_request_body(&requests, "large-window HTTP preview request"))
        .collect::<Vec<_>>();
    server.join().unwrap();
    http_server.join().unwrap();
    let http_request = wires[2]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "http_request")
        .unwrap();
    assert_eq!(
        http_request["parameters"]["properties"]["preview_bytes"]["maximum"],
        json!(128 * 1024)
    );
    let delivered = wires[3]["input"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| {
            item["type"] == "function_call_output" && item["call_id"] == "preview-large-response"
        })
        .unwrap();
    let delivered: Value = serde_json::from_str(delivered["output"].as_str().unwrap()).unwrap();
    assert_eq!(
        delivered["output"]["body_preview"]["captured_bytes"],
        json!(128 * 1024)
    );
    assert_eq!(delivered["output"]["body_preview"]["data"], response_body);
    assert_eq!(delivered["output"]["body_preview"]["truncated"], false);
}
