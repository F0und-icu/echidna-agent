use super::task_context::{
    ScriptedResponse::{InitialSelection, Response},
    selector_candidates, spawn_scripted_responses_server,
};
use super::*;

#[test]
fn ctf_r3b2_unread_tool_preview_does_not_block_child_durability() {
    let root = TemporaryDirectory::new("r3b2-slow-terminal");
    let harness = TemporaryDirectory::new("r3b2-slow-provider");
    initialize_project(root.path());
    let commands = (0..32)
        .map(|_| {
            json!({"argv":["sh","-c",
        "printf '%6000s' x; printf x >> completed", "p".repeat(3500)]})
        })
        .collect::<Vec<_>>();
    let server = spawn_scripted_responses_server(vec![
        InitialSelection,
        Response(
            200,
            m4_long_flow_tool_response(
                "slow",
                "run_commands",
                json!({"mode":"serial","commands":commands}),
                "slow-response",
            ),
        ),
        Response(200, text_response("DONE", "slow-final")),
    ]);
    let config = write_context_window_config(harness.path(), &server.base_url, 1_000_000);
    // Intentionally leave stderr unread until every real child and parent
    // checkpoint exists. More than 128 KiB of previews exceeds the pipe/queue.
    let child = spawn_f2a_chat(root.path(), &config, "bounded output\n/exit\n", &[]);
    selector_candidates(&f2a_request_body(&server.requests, "slow selection"));
    let _ = f2a_request_body(&server.requests, "slow first");
    let session = session_id(root.path());
    let path = root
        .path()
        .join(".echidna/sessions")
        .join(&session)
        .join("events.jsonl");
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut checkpoints = 0;
    let mut terminal = false;
    while Instant::now() < deadline {
        // Concurrent append may leave an unfinished final line; this is only
        // a test observation, not a replacement for the strict Session reader.
        let events = fs::read_to_string(&path).unwrap();
        let events = events
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .collect::<Vec<_>>();
        checkpoints = events
            .iter()
            .filter(|event| event["event_type"] == "tool.subcommand_checkpointed")
            .count();
        terminal = events
            .iter()
            .any(|event| event["event_type"] == "tool.execution_outcome_recorded");
        if checkpoints == 32 && terminal {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    // Always release the real pipe before asserting, so a red regression does
    // not leave the owned CLI or provider fixture blocked.
    let output = f2a_wait_child(child, "slow preview cleanup");
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let _ = f2a_request_body(&server.requests, "slow followup");
    f2a_join_server(server, "slow preview");
    assert_eq!(
        checkpoints, 32,
        "unread terminal prevented child checkpoints"
    );
    assert!(terminal, "unread terminal prevented the parent Receipt");
    assert_eq!(
        fs::read(root.path().join("completed")).unwrap(),
        vec![b'x'; 32]
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("presentation congested"),
        "queue saturation must be explicit: {stderr}"
    );
    // Bytes already accepted by the OS pipe are not queue occupancy. Prove
    // that presentation was dropped while all32 execution facts survived.
    assert!(
        stderr.matches("· subcommand").count() < 32,
        "saturated previews must be dropped"
    );
}

#[test]
fn ctf_r3b2_child_progress_is_visible_before_a_blocked_parent_finishes() {
    for mode in ["serial", "parallel"] {
        let root = TemporaryDirectory::new("r3b2-progress");
        let harness = TemporaryDirectory::new("r3b2-provider");
        initialize_project(root.path());
        let server = spawn_scripted_responses_server(vec![
            InitialSelection,
            Response(
                200,
                m4_long_flow_tool_response(
                    "progress",
                    "run_commands",
                    json!({"mode":mode,"commands":[
                        {"argv":["sh","-c","printf CHILD_VISIBLE"],"resources":[{"kind":"output","path":"first"}]},
                        {"argv":["sh","-c","printf x > sibling; sleep 4"],"resources":[{"kind":"output","path":"sibling"}]}
                    ]}),
                    "progress-response",
                ),
            ),
        ]);
        let config = write_config(harness.path(), &server.base_url);
        let mut child = spawn_f2a_chat(root.path(), &config, "bounded progress\n", &[]);
        selector_candidates(&f2a_request_body(&server.requests, "progress selection"));
        let _ = f2a_request_body(&server.requests, "progress");
        f2a_join_server(server, "progress");
        let mut stderr = child.stderr.take().unwrap();
        let (lines, received) = mpsc::channel();
        let reader = thread::spawn(move || {
            let mut bytes = Vec::new();
            let mut chunk = [0; 1024];
            while let Ok(count) = stderr.read(&mut chunk) {
                if count == 0 {
                    break;
                }
                bytes.extend_from_slice(&chunk[..count]);
                let _ = lines.send(String::from_utf8_lossy(&bytes).into_owned());
            }
            bytes
        });
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut visible = String::new();
        while Instant::now() < deadline {
            if let Ok(text) = received.recv_timeout(Duration::from_millis(50)) {
                visible = text;
                if visible.contains("subcommand 1/2 · persisted")
                    && visible.contains("CHILD_VISIBLE")
                {
                    break;
                }
            }
        }
        let events = session_events(root.path(), &session_id(root.path()));
        let _ = child.kill();
        let _ = f2a_wait_child(child, "progress cleanup");
        let _ = reader.join().unwrap();
        assert!(visible.contains("subcommand 1/2"), "{visible}");
        assert!(
            visible.contains("subcommand 1/2 · persisted") && visible.contains("CHILD_VISIBLE"),
            "{visible}"
        );
        assert!(
            visible.contains("deadline") && visible.contains("elapsed"),
            "{visible}"
        );
        assert!(
            !events
                .iter()
                .any(|event| event["event_type"] == "tool.execution_outcome_recorded")
        );
        assert!(
            events
                .iter()
                .any(|event| event["event_type"] == "tool.subcommand_checkpointed")
        );
    }
}
