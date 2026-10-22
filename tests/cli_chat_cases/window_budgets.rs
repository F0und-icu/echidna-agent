//! Capacity-derived limits through the real CLI and immutable request catalog.

use super::*;

fn definition(root: &std::path::Path) -> std::path::PathBuf {
    let directory = root.join("window-agent");
    fs::create_dir(&directory).unwrap();
    fs::write(directory.join("persona.md"), "A precise local assistant.").unwrap();
    fs::write(directory.join("work.md"), "Complete the requested task.").unwrap();
    fs::write(
        directory.join("agent.toml"),
        r#"schema_version = 1
id = "window-agent"
name = "Window agent"
[instructions]
persona_file = "persona.md"
work_file = "work.md"
method_files = []
[context]
personal = "none"
project = "none"
memory = "project"
[tools]
read_file = "direct"
run_command = "direct"
memory_read = "direct"
memory_write = "direct"
read_output = "direct"
http_request = "direct"
"#,
    )
    .unwrap();
    directory.join("agent.toml")
}

fn calls(calls: &[(&str, &str, Value)]) -> String {
    let mut events: Vec<_> = calls
        .iter()
        .enumerate()
        .map(|(index, (id, name, args))| {
            json!({"type":"response.output_item.done","output_index":index,"item":{
                "type":"function_call","call_id":id,"name":name,"arguments":args.to_string()
            }})
        })
        .collect();
    events.push(json!({"type":"response.completed","response":{"id":"window-tools"}}));
    events
        .iter()
        .map(|event| format!("data: {event}\n\n"))
        .collect()
}

fn result(request: &Value, call: &str) -> Value {
    let item = request["input"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["type"] == "function_call_output" && item["call_id"] == call)
        .unwrap_or_else(|| panic!("missing result {call}"));
    let body: Value = serde_json::from_str(item["output"].as_str().unwrap()).unwrap();
    body["output"].clone()
}

fn parameters<'a>(request: &'a Value, name: &str) -> &'a Value {
    &request["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == name)
        .unwrap()["parameters"]
}

fn project_snapshot(request: &Value) -> Value {
    request["input"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|item| item["content"].as_array().into_iter().flatten())
        .filter_map(|content| serde_json::from_str::<Value>(content["text"].as_str()?).ok())
        .find(|value| value["kind"] == "project_snapshot_full")
        .unwrap()
}

fn contains_input_text(request: &Value, text: &str) -> bool {
    request["input"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|item| item["content"].as_array().into_iter().flatten())
        .any(|content| content["type"] == "input_text" && content["text"] == text)
}

#[test]
fn capacity_window_scales_actual_file_and_command_results_with_the_advertised_catalog() {
    for (window, limit) in [(131072, 131072usize), (272000, 262144usize)] {
        let root = TemporaryDirectory::new("window-tool-results");
        initialize_project(root.path());
        let definition = definition(root.path());
        let content = format!("{}END_OF_LARGE_FILE", "abcdefgh".repeat(23000));
        fs::write(root.path().join("large.txt"), &content).unwrap();
        let command_content = format!("{}END_OF_LARGE_FILE", "abcdefgh".repeat(11500));
        fs::write(root.path().join("command.txt"), &command_content).unwrap();
        let server = spawn_responses_server(vec![
            (
                200,
                calls(&[
                    ("window-file", "read_file", json!({"path":"large.txt"})),
                    (
                        "window-command",
                        "run_command",
                        json!({"argv":["/bin/cat","command.txt"]}),
                    ),
                ]),
            ),
            (200, text_response("COMPLETE", "window-done")),
        ]);
        let config = write_context_window_config(root.path(), &server.base_url, window);
        let output = run_chat(
            root.path(),
            &config,
            "Read the local file both ways.\n/exit\n",
            &["--agent", definition.to_str().unwrap()],
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let first = request_body(
            &server
                .requests
                .recv_timeout(Duration::from_secs(10))
                .unwrap(),
        );
        let second = request_body(
            &server
                .requests
                .recv_timeout(Duration::from_secs(10))
                .unwrap(),
        );
        server.join();
        assert_eq!(
            parameters(&first, "read_output")["properties"]["max_bytes"]["maximum"],
            limit / 8
        );
        assert_eq!(
            parameters(&first, "http_request")["properties"]["preview_bytes"]["maximum"],
            limit / 8
        );
        assert_eq!(
            parameters(&first, "memory_write")["allOf"][0]["then"]["properties"]["content"]["maxLength"],
            limit / 2
        );
        let snapshot = project_snapshot(&first);
        assert_eq!(snapshot["snapshot"]["truncated"] == true, window == 131072);
        if window == 131072 {
            assert_eq!(
                snapshot["snapshot"]["rendered_prefix"]
                    .as_str()
                    .unwrap()
                    .len(),
                384
            );
        } else {
            assert!(snapshot["snapshot"]["file_count"].as_u64().unwrap() >= 2);
        }
        let file = result(&second, "window-file");
        let command = result(&second, "window-command");
        let large = window == 272000;
        let delivered_file = file["content"].as_str().unwrap();
        assert!(delivered_file.len() <= limit);
        assert_eq!(delivered_file == content, large);
        assert_eq!(file["truncated"], !large);
        let delivered_command = command["stdout"].as_str().unwrap();
        assert!(delivered_command.len() <= limit / 2);
        assert_eq!(delivered_command == command_content, large);
        assert_eq!(command["truncated"], !large);
        let project = Project::discover(root.path()).unwrap();
        let state = reduced_session(&project, &session_id(root.path()));
        assert_eq!(state.execution_receipt_count(), 2);
        assert_audits_reconstruct(root.path(), &state);
    }
}

#[test]
fn capacity_core_memory_stays_valid_when_a_smaller_active_tool_budget_rejects_delivery() {
    let root = TemporaryDirectory::new("window-core-memory");
    initialize_project(root.path());
    let definition = definition(root.path());
    let content = format!("{}CORE_MEMORY_END", "persistent ".repeat(7200));
    assert!(content.len() > 65536 && content.len() < 131072);
    let server = spawn_responses_server(vec![
        (
            200,
            calls(&[(
                "core-write",
                "memory_write",
                json!({"path":"core.md","content":content,"expected_hash":null,"event_id":"large-core"}),
            )]),
        ),
        (200, text_response("SAVED", "large-core-done")),
    ]);
    let config = write_context_window_config(root.path(), &server.base_url, 272000);
    let first = run_chat(
        root.path(),
        &config,
        "Save the core.\n/exit\n",
        &["--agent", definition.to_str().unwrap()],
    );
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let requests: Vec<_> = server
        .requests
        .try_iter()
        .map(|r| request_body(&r))
        .collect();
    server.join();
    assert_eq!(requests.len(), 2);
    assert!(contains_input_text(&requests[1], &content));
    let path = root.path().join(".echidna/memory/core.md");
    assert_eq!(fs::read_to_string(&path).unwrap(), content);
    let original_hash = blake3::hash(content.as_bytes());
    let session = session_id(root.path());
    let server = spawn_responses_server(vec![
        (
            200,
            calls(&[(
                "core-read",
                "memory_read",
                json!({"path":"core.md","event_id":"read-small-core"}),
            )]),
        ),
        (200, text_response("LIMIT_VISIBLE", "small-core-done")),
    ]);
    let config = write_context_window_config(root.path(), &server.base_url, 131072);
    let resumed = run_chat(
        root.path(),
        &config,
        "Read the saved core.\n/exit\n",
        &["--session", &session],
    );
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    let first = request_body(
        &server
            .requests
            .recv_timeout(Duration::from_secs(10))
            .unwrap(),
    );
    let second = request_body(
        &server
            .requests
            .recv_timeout(Duration::from_secs(10))
            .unwrap(),
    );
    server.join();
    assert!(
        contains_input_text(&first, &content),
        "automatic Core context remains the complete source fragment"
    );
    assert!(
        result(&second, "core-read")
            .to_string()
            .contains("memory_read_failed")
    );
    assert!(
        result(&second, "core-read")
            .to_string()
            .contains("active Tool limit is 65536")
    );
    assert_eq!(blake3::hash(&fs::read(&path).unwrap()), original_hash);
    let state = reduced_session(&Project::discover(root.path()).unwrap(), &session);
    assert_eq!(state.execution_receipt_count(), 2);
    assert_audits_reconstruct(root.path(), &state);
}

#[test]
fn capacity_readback_admits_the_active_limit_and_uses_the_scaled_default() {
    for (window, default_bytes, extended) in [(131072, 4096, false), (272000, 8192, true)] {
        let root = TemporaryDirectory::new("window-readback");
        initialize_project(root.path());
        let definition = definition(root.path());
        let bytes = "raw output\n".repeat(3000);
        fs::write(root.path().join("output.txt"), &bytes).unwrap();
        let blob_id = format!("{:x}", Sha256::digest(bytes.as_bytes()));
        let server = spawn_responses_server(vec![
            (
                200,
                calls(&[(
                    "produce",
                    "run_command",
                    json!({"argv":["/bin/cat","output.txt"]}),
                )]),
            ),
            (
                200,
                calls(&[
                    (
                        "read-extended",
                        "read_output",
                        json!({"blob_id":blob_id,"max_bytes":24000}),
                    ),
                    ("read-default", "read_output", json!({"blob_id":blob_id})),
                ]),
            ),
            (200, text_response("READBACK_DONE", "readback-done")),
        ]);
        let config = write_context_window_config(root.path(), &server.base_url, window);
        let output = run_chat(
            root.path(),
            &config,
            "Read the captured bytes.\n/exit\n",
            &["--agent", definition.to_str().unwrap()],
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let requests: Vec<_> = server
            .requests
            .try_iter()
            .map(|request| request_body(&request))
            .collect();
        server.join();
        assert_eq!(requests.len(), 3);
        let exact = result(&requests[2], "read-extended");
        if extended {
            assert_eq!(exact["text"].as_str().unwrap().len(), 24000);
            assert_eq!(exact["next_offset"], 24000);
        } else {
            assert!(exact.to_string().contains("16384"));
        }
        let default = result(&requests[2], "read-default");
        assert_eq!(default["text"].as_str().unwrap().len(), default_bytes);
        assert_eq!(default["has_more"], true);
        let state = reduced_session(
            &Project::discover(root.path()).unwrap(),
            &session_id(root.path()),
        );
        assert_eq!(
            state.execution_receipt_count(),
            if extended { 3 } else { 2 }
        );
        assert_audits_reconstruct(root.path(), &state);
    }
}

#[test]
fn capacity_compaction_keeps_more_real_references_and_v4_resumes_under_a_smaller_window() {
    for window in [131072, 272000] {
        let root = TemporaryDirectory::new("window-compaction");
        initialize_project(root.path());
        let definition = definition(root.path());
        let server = spawn_responses_server(
            (0..20)
                .map(|index| {
                    (
                        200,
                        text_response(
                            &format!("ANSWER_{index} {}", "continuation ".repeat(1000)),
                            &format!("turn-{index}"),
                        ),
                    )
                })
                .collect(),
        );
        let config = write_context_window_config(root.path(), &server.base_url, window);
        let mut input: String = (0..20).map(|index| format!("Task {index}\n")).collect();
        input.push_str("/compact\n/exit\n");
        let output = run_chat(
            root.path(),
            &config,
            &input,
            &["--agent", definition.to_str().unwrap()],
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(server.requests.try_iter().count(), 20);
        server.join();
        let session = session_id(root.path());
        let state = reduced_session(&Project::discover(root.path()).unwrap(), &session);
        let summary = serde_json::to_value(state.latest_compaction().unwrap().summary()).unwrap();
        let large = window == 272000;
        assert_eq!(summary["schema_version"], if large { 4 } else { 3 });
        assert_eq!(
            summary["task_references"]["history_event_refs"]
                .as_array()
                .unwrap()
                .len(),
            if large { 40 } else { 32 }
        );
        assert_eq!(
            summary["narrative"]["concise_progress"]
                .as_str()
                .unwrap()
                .len(),
            if large { 3072 } else { 1536 }
        );
        if large {
            assert_eq!(summary["byte_limit"], 16384);
            let server =
                spawn_responses_server(vec![(200, text_response("RESUMED_V4", "small-resume"))]);
            let config = write_context_window_config(root.path(), &server.base_url, 131072);
            let output = run_chat(
                root.path(),
                &config,
                "Continue from the saved summary.\n/exit\n",
                &["--session", &session],
            );
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            server
                .requests
                .recv_timeout(Duration::from_secs(10))
                .unwrap();
            server.join();
            let state = reduced_session(&Project::discover(root.path()).unwrap(), &session);
            assert_eq!(
                serde_json::to_value(state.latest_compaction().unwrap().summary()).unwrap(),
                summary
            );
        }
    }
}
