use super::task_context::{
    ScriptedResponse::{InitialSelection, Response},
    spawn_scripted_responses_server,
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
        assert!(!String::from_utf8_lossy(&output.stdout).contains("loopback-primary-secret"));
    }
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
    let root = TemporaryDirectory::new("jsonl-rejected");
    initialize_project(root.path());
    let body = [
        json!({"type":"response.output_text.delta","output_index":0,"content_index":0,"delta":"draft"}),
        json!({"type":"response.output_text.done","output_index":0,"content_index":0,"text":"conflict"}),
        json!({"type":"response.completed","response":{"id":"unaccepted"}}),
    ].iter().map(|e| format!("data: {e}\n\n")).collect();
    let server = spawn_scripted_responses_server(vec![InitialSelection, Response(200, body)]);
    let config = write_config(root.path(), &server.base_url);
    let output = run_chat(
        root.path(),
        &config,
        "Reply.\n/exit\n",
        &["--output-format", "jsonl"],
    );
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
