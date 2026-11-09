use super::task_context::{
    ScriptedResponse::{InitialSelection, Response},
    selector_candidates, spawn_scripted_responses_server,
    spawn_scripted_responses_server_with_wait,
};
use super::*;

fn stop_held_pipe_descendant(root: &std::path::Path) {
    let pid_path = root.join("held-pipe.pid");
    let deadline = Instant::now() + Duration::from_secs(2);
    while !pid_path.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    let pid = fs::read_to_string(&pid_path)
        .expect("held-pipe helper must record its descendant PID")
        .trim()
        .to_owned();
    let _ = Command::new("/bin/kill").args(["-TERM", &pid]).output();
    while Instant::now() < deadline {
        let running = Command::new("/bin/kill")
            .args(["-0", &pid])
            .output()
            .is_ok_and(|output| output.status.success());
        if !running {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("held-pipe descendant {pid} did not stop after TERM");
}

fn held_pipe_command(resource_path: &str) -> serde_json::Value {
    json!({
        "argv": ["sh", "-c", "( exec /bin/sleep 30 ) & descendant=$!; printf '%s\\n' \"$descendant\" > held-pipe.pid; printf HELD_CHILD"],
        "resources": [{"kind": "output", "path": resource_path}]
    })
}

#[test]
fn ctf_r3b1_real_executor_deadline_preserves_acknowledged_children() {
    let root = TemporaryDirectory::new("r3b1-timeout");
    let harness = TemporaryDirectory::new("r3b1-timeout-provider");
    initialize_project(root.path());
    // The main follow-up arrives after the real 120s executor deadline. Match
    // the existing child deadline instead of the ordinary 15s request wait.
    let server = spawn_scripted_responses_server_with_wait(
        vec![
            InitialSelection,
            Response(
                200,
                m4_long_flow_tool_response(
                    "timeout-batch",
                    "run_commands",
                    json!({"mode":"serial","commands":[
                        {"argv":["sh","-c","printf x >> first; printf RETAINED_BEFORE_TIMEOUT"]},
                        {"argv":["sh","-c","printf x > second; exec sleep 125"]},
                        {"argv":["sh","-c","printf x > third"]}
                    ]}),
                    "timeout-response",
                ),
            ),
            Response(
                200,
                text_response("known child; others unknown", "timeout-final"),
            ),
        ],
        Duration::from_secs(140),
    );
    let config = write_config(harness.path(), &server.base_url);
    // Exercise the real catalog's 120s deadline rather than adding a test-only
    // timeout setting or exposing the private durability path publicly.
    let output = wait_child_before_deadline(
        spawn_f2a_chat(root.path(), &config, "bounded batch\n/exit\n", &[]),
        "real batch deadline",
        Duration::from_secs(140),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("· run_commands · cancelled · reason tool_timeout"),
        "{stderr}"
    );
    assert!(stderr.contains("configured deadline"), "{stderr}");
    assert!(
        stderr.contains("elapsed") && stderr.contains("including persistence"),
        "{stderr}"
    );
    selector_candidates(&f2a_request_body(&server.requests, "timeout selection"));
    let _ = f2a_request_body(&server.requests, "timeout first");
    let request = f2a_request_body(&server.requests, "timeout followup");
    f2a_join_server(server, "timeout");
    assert!(request.to_string().contains("RETAINED_BEFORE_TIMEOUT"));
    assert!(request.to_string().contains("unknown"));
    assert_eq!(fs::read(root.path().join("first")).unwrap(), b"x");
    assert!(root.path().join("second").exists());
    assert!(!root.path().join("third").exists());
    let events = session_events(root.path(), &session_id(root.path()));
    assert_eq!(
        events
            .iter()
            .filter(|event| event["event_type"] == "tool.subcommand_checkpointed")
            .count(),
        1
    );
    let receipt = events
        .iter()
        .find(|event| event["event_type"] == "tool.execution_outcome_recorded")
        .unwrap();
    assert_eq!(receipt["payload"]["receipt"]["status"], "cancelled");
    assert_eq!(
        receipt["payload"]["receipt"]["output_captures"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn ctf_r3b1_session_append_failure_does_not_ack_or_launch_the_next_child() {
    use std::os::unix::fs::PermissionsExt;
    let root = TemporaryDirectory::new("r3b1-event-failure");
    let harness = TemporaryDirectory::new("r3b1-event-provider");
    initialize_project(root.path());
    let server = spawn_scripted_responses_server(vec![
        InitialSelection,
        Response(
            200,
            m4_long_flow_tool_response(
                "event-failure",
                "run_commands",
                json!({"mode":"serial","commands":[
                    {"argv":["sh","-c","printf FIRST"]},
                    {"argv":["sh","-c","chmod 400 .echidna/sessions/*/events.jsonl; printf SECOND"]},
                    {"argv":["sh","-c","printf x > forbidden-next"]}
                ]}),
                "event-failure-response",
            ),
        ),
    ]);
    let config = write_config(harness.path(), &server.base_url);
    let output = f2a_wait_child(
        spawn_f2a_chat(root.path(), &config, "run once\n/exit\n", &[]),
        "event failure",
    );
    let session = session_id(root.path());
    fs::set_permissions(
        root.path()
            .join(".echidna/sessions")
            .join(&session)
            .join("events.jsonl"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    assert!(!output.status.success());
    assert!(!root.path().join("forbidden-next").exists());
    selector_candidates(&f2a_request_body(
        &server.requests,
        "event failure selection",
    ));
    let _ = f2a_request_body(&server.requests, "event failure");
    f2a_join_server(server, "event failure");
    let events = session_events(root.path(), &session);
    assert_eq!(
        events
            .iter()
            .filter(|event| event["event_type"] == "tool.subcommand_checkpointed")
            .count(),
        1
    );
    assert!(!events.iter().any(
        |event| event["event_type"] == "tool.execution_outcome_recorded"
            || event["payload"]["item"]["kind"] == "tool_result"
    ));
}

#[test]
fn ctf_r3b1_completed_child_survives_killed_batch_and_resume_without_rerun() {
    for (mode, interrupt) in [
        ("serial", false),
        ("parallel", false),
        ("serial", true),
        ("parallel", true),
    ] {
        let root = TemporaryDirectory::new("r3b1-killed-batch");
        let harness = TemporaryDirectory::new("r3b1-provider");
        initialize_project(root.path());
        let server = spawn_scripted_responses_server(vec![
            InitialSelection,
            Response(
                200,
                m4_long_flow_tool_response(
                    "partial-batch",
                    "run_commands",
                    json!({"mode":mode,"commands":[
                        {"argv":["sh","-c","printf CHILD_OUTPUT; printf x >> first-count"],"resources":[{"kind":"output","path":"first-count"}]},
                        {"argv":["sh","-c","printf x > second-start; sleep 5"],"resources":[{"kind":"output","path":"second-start"}]}
                    ]}),
                    "partial-response",
                ),
            ),
        ]);
        let config = write_config(harness.path(), &server.base_url);
        let mut child = spawn_f2a_chat(root.path(), &config, "run the bounded batch\n", &[]);
        selector_candidates(&f2a_request_body(
            &server.requests,
            "R3b1 initial selection",
        ));
        let _ = f2a_request_body(&server.requests, "R3b1 first request");
        f2a_join_server(server, "R3b1 first request");
        let session = session_id(root.path());
        let events_path = root
            .path()
            .join(".echidna/sessions")
            .join(&session)
            .join("events.jsonl");
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if root.path().join("second-start").exists()
                && fs::read_to_string(&events_path)
                    .unwrap()
                    .contains("tool.subcommand_checkpointed")
            {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        // Always stop this exact owned child before asserting, including the
        // initial red run. The synthetic sibling self-terminates within 5s.
        if interrupt {
            assert!(
                Command::new("kill")
                    .args(["-INT", &child.id().to_string()])
                    .status()
                    .unwrap()
                    .success()
            );
        } else {
            let _ = child.kill();
        }
        let _ = f2a_wait_child(child, "R3b1 killed child");
        assert_eq!(fs::read(root.path().join("first-count")).unwrap(), b"x");
        assert!(root.path().join("second-start").exists());
        let shown = Command::new(env!("CARGO_BIN_EXE_echidna"))
            .current_dir(root.path())
            .args(["session", "show", &session])
            .output()
            .unwrap();
        assert!(
            shown.status.success(),
            "{}",
            String::from_utf8_lossy(&shown.stderr)
        );
        let shown: Value = serde_json::from_slice(&shown.stdout).unwrap();
        assert_eq!(shown["subcommand_checkpoints"], 1);
        let events = session_events(root.path(), &session);
        let checkpoints: Vec<_> = events
            .iter()
            .filter(|event| event["event_type"] == "tool.subcommand_checkpointed")
            .collect();
        assert_eq!(
            checkpoints.len(),
            1,
            "{mode}: completed child must be durable before parent completion"
        );
        assert!(
            !events
                .iter()
                .any(|event| event["event_type"] == "tool.execution_outcome_recorded")
        );
        assert!(checkpoints[0].to_string().contains("CHILD_OUTPUT"));
        let blob = checkpoints[0]["payload"]["output_captures"][0]["blob_id"]
            .as_str()
            .unwrap();
        assert_eq!(
            fs::read(
                root.path()
                    .join(".echidna/artifacts/blobs/sha256")
                    .join(blob)
            )
            .unwrap(),
            b"CHILD_OUTPUT"
        );

        use super::task_context::ScriptedResponse::ContinueSelection;
        let resumed = spawn_scripted_responses_server(vec![
            ContinueSelection { usage: None },
            Response(
                200,
                text_response("KNOWN CHILD RETAINED; PARENT UNKNOWN", "resumed"),
            ),
        ]);
        let config = write_config(harness.path(), &resumed.base_url);
        let output = f2a_wait_child(
            spawn_f2a_chat(
                root.path(),
                &config,
                "continue using retained child facts\n/exit\n",
                &["--resume", "latest"],
            ),
            "R3b1 resume",
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        selector_candidates(&f2a_request_body(
            &resumed.requests,
            "R3b1 resumed source selection",
        ));
        let request = f2a_request_body(&resumed.requests, "R3b1 resumed request");
        f2a_join_server(resumed, "R3b1 resumed request");
        assert!(request.to_string().contains("CHILD_OUTPUT"));
        assert!(request.to_string().contains("unknown"));
        assert_eq!(fs::read(root.path().join("first-count")).unwrap(), b"x");
        assert_eq!(
            session_events(root.path(), &session)
                .iter()
                .filter(|event| event["event_type"] == "tool.subcommand_checkpointed")
                .count(),
            1
        );
    }
}

#[test]
fn ctf_r3b1_failed_publication_stops_before_next_serial_child_and_preserves_prior_checkpoint() {
    for failed_child in [0, 1] {
        let root = TemporaryDirectory::new("r3b1-publication-failure");
        let harness = TemporaryDirectory::new("r3b1-failure-provider");
        initialize_project(root.path());
        let bytes = if failed_child == 0 {
            b"FIRST".as_slice()
        } else {
            b"SECOND".as_slice()
        };
        let blobs = root.path().join(".echidna/artifacts/blobs/sha256");
        fs::create_dir_all(&blobs).unwrap();
        fs::write(
            blobs.join(format!("{:x}", Sha256::digest(bytes))),
            vec![b'x'; bytes.len()],
        )
        .unwrap();
        let server = spawn_scripted_responses_server(vec![
            InitialSelection,
            Response(
                200,
                m4_long_flow_tool_response(
                    "failing-batch",
                    "run_commands",
                    json!({"mode":"serial","commands":[
                        {"argv":["sh","-c","printf x >> first; printf FIRST"]},
                        {"argv":["sh","-c","printf x >> second; printf SECOND"]},
                        {"argv":["sh","-c","printf x >> third"]}
                    ]}),
                    "failure",
                ),
            ),
        ]);
        let config = write_config(harness.path(), &server.base_url);
        let output = f2a_wait_child(
            spawn_f2a_chat(root.path(), &config, "run once\n/exit\n", &[]),
            "child publication failure",
        );
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("Tool output persistence failed"));
        selector_candidates(&f2a_request_body(&server.requests, "failure selection"));
        let _ = f2a_request_body(&server.requests, "failure");
        f2a_join_server(server, "failure");
        assert_eq!(fs::read(root.path().join("first")).unwrap(), b"x");
        assert_eq!(root.path().join("second").exists(), failed_child == 1);
        assert!(!root.path().join("third").exists());
        let events = session_events(root.path(), &session_id(root.path()));
        assert_eq!(
            events
                .iter()
                .filter(|event| event["event_type"] == "tool.subcommand_checkpointed")
                .count(),
            failed_child
        );
        assert!(!events.iter().any(|event| event["event_type"]
            == "tool.execution_outcome_recorded"
            || event["payload"]["item"]["kind"] == "tool_result"));
    }
}

#[test]
fn ctf_r3b1_completed_batch_has_ordered_parent_and_individual_durable_children() {
    for mode in ["serial", "parallel"] {
        let root = TemporaryDirectory::new("r3b1-completed");
        let harness = TemporaryDirectory::new("r3b1-completed-provider");
        initialize_project(root.path());
        let server = spawn_scripted_responses_server(vec![
            InitialSelection,
            Response(
                200,
                m4_long_flow_tool_response(
                    "children",
                    "run_commands",
                    json!({"mode":mode,"commands":[
                        {"argv":["sh","-c","sleep 0.2; printf FIRST; printf x >> first-count"],"resources":[{"kind":"output","path":"first-count"}]},
                        {"argv":["sh","-c","printf SECOND; exit 7"],"resources":[{"kind":"output","path":"second-count"}]}
                    ]}),
                    "batch-completed",
                ),
            ),
            Response(200, text_response("done", "batch-final")),
        ]);
        let config = write_config(harness.path(), &server.base_url);
        let output = f2a_wait_child(
            spawn_f2a_chat(root.path(), &config, "run once\n/exit\n", &[]),
            "R3b1 complete",
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        selector_candidates(&f2a_request_body(
            &server.requests,
            "completed batch selection",
        ));
        let _ = f2a_request_body(&server.requests, "first");
        let request = f2a_request_body(&server.requests, "second");
        f2a_join_server(server, "complete");
        assert!(request.to_string().contains("FIRST"));
        assert!(request.to_string().contains("SECOND"));
        let events = session_events(root.path(), &session_id(root.path()));
        let children = events
            .iter()
            .filter(|event| event["event_type"] == "tool.subcommand_checkpointed")
            .collect::<Vec<_>>();
        assert_eq!(children.len(), 2);
        if mode == "parallel" {
            assert_eq!(children[0]["payload"]["index"], 1);
        }
        let receipts = events
            .iter()
            .filter(|event| event["event_type"] == "tool.execution_outcome_recorded")
            .collect::<Vec<_>>();
        assert_eq!(receipts.len(), 1);
        for checkpoint in &children {
            assert!(
                checkpoint["sequence"].as_u64().unwrap()
                    < receipts[0]["sequence"].as_u64().unwrap()
            );
            let index = checkpoint["payload"]["index"].as_u64().unwrap();
            let captures = receipts[0]["payload"]["receipt"]["output_captures"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|capture| capture["child_index"] == index)
                .cloned()
                .collect::<Vec<_>>();
            assert_eq!(json!(captures), checkpoint["payload"]["output_captures"]);
        }
        assert_eq!(fs::read(root.path().join("first-count")).unwrap(), b"x");
    }
}

#[test]
fn ec_k4_incomplete_batch_child_checkpoints_before_parent_receipt_in_serial_and_parallel() {
    for mode in ["serial", "parallel"] {
        let root = TemporaryDirectory::new("ec-k4-incomplete-batch");
        let harness = TemporaryDirectory::new("ec-k4-incomplete-batch-provider");
        initialize_project(root.path());
        let server = spawn_scripted_responses_server(vec![
            InitialSelection,
            Response(
                200,
                m4_long_flow_tool_response(
                    "incomplete-batch",
                    "run_commands",
                    json!({"mode":mode,"commands":[
                        held_pipe_command("held-output"),
                        {"argv":["sh","-c","printf COMPLETE_CHILD"],"resources":[{"kind":"output","path":"complete-output"}]}
                    ]}),
                    "incomplete-batch-response",
                ),
            ),
            Response(
                200,
                text_response("batch complete", "incomplete-batch-final"),
            ),
        ]);
        let config = write_config(harness.path(), &server.base_url);
        let output = f2a_wait_child(
            spawn_f2a_chat(root.path(), &config, "run incomplete batch\n/exit\n", &[]),
            "EC-K4 incomplete batch",
        );
        stop_held_pipe_descendant(root.path());
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        selector_candidates(&f2a_request_body(
            &server.requests,
            "EC-K4 incomplete batch selection",
        ));
        let _ = f2a_request_body(&server.requests, "EC-K4 incomplete batch tool call");
        let result = f2a_request_body(&server.requests, "EC-K4 incomplete batch result");
        f2a_join_server(server, "EC-K4 incomplete batch");
        assert!(result.to_string().contains("HELD_CHILD"));
        assert!(result.to_string().contains("stdout_reader_grace_timeout"));

        let events = session_events(root.path(), &session_id(root.path()));
        let children = events
            .iter()
            .filter(|event| event["event_type"] == "tool.subcommand_checkpointed")
            .collect::<Vec<_>>();
        assert_eq!(children.len(), 2);
        let receipt = events
            .iter()
            .find(|event| event["event_type"] == "tool.execution_outcome_recorded")
            .expect("parent Receipt follows both child checkpoints");
        let held = children
            .iter()
            .find(|event| event["payload"]["index"] == 0)
            .expect("held child is checkpointed");
        let held_captures = held["payload"]["output_captures"]
            .as_array()
            .expect("held child captures are durable");
        assert_eq!(held_captures.len(), 2);
        assert!(held_captures.iter().all(|capture| {
            capture["child_index"] == 0 && capture["complete"] == false && capture["eof"] == false
        }));
        assert!(
            held_captures
                .iter()
                .any(|capture| { capture["read_error"] == "stdout_reader_grace_timeout" })
        );
        for checkpoint in children {
            assert!(
                checkpoint["sequence"].as_u64().unwrap() < receipt["sequence"].as_u64().unwrap()
            );
            let index = checkpoint["payload"]["index"].as_u64().unwrap();
            let parent_captures = receipt["payload"]["receipt"]["output_captures"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|capture| capture["child_index"] == index)
                .cloned()
                .collect::<Vec<_>>();
            assert_eq!(
                json!(parent_captures),
                checkpoint["payload"]["output_captures"]
            );
        }
    }
}

#[test]
fn ec_k4_incomplete_batch_capture_publication_failure_is_fatal_before_parent_receipt() {
    let root = TemporaryDirectory::new("ec-k4-incomplete-batch-publication");
    let harness = TemporaryDirectory::new("ec-k4-incomplete-batch-publication-provider");
    initialize_project(root.path());
    let blob_directory = root.path().join(".echidna/artifacts/blobs/sha256");
    fs::create_dir_all(&blob_directory).unwrap();
    fs::write(
        blob_directory.join(format!("{:x}", Sha256::digest(b"HELD_CHILD"))),
        b"CONFLICTED",
    )
    .unwrap();
    let server = spawn_scripted_responses_server(vec![
        InitialSelection,
        Response(
            200,
            m4_long_flow_tool_response(
                "incomplete-batch-publication",
                "run_commands",
                json!({"mode":"serial","commands":[held_pipe_command("held-output")]}),
                "incomplete-batch-publication-response",
            ),
        ),
    ]);
    let config = write_config(harness.path(), &server.base_url);
    let output = f2a_wait_child(
        spawn_f2a_chat(root.path(), &config, "run incomplete batch\n/exit\n", &[]),
        "EC-K4 incomplete batch publication failure",
    );
    stop_held_pipe_descendant(root.path());
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Tool output persistence failed"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    selector_candidates(&f2a_request_body(
        &server.requests,
        "EC-K4 publication selection",
    ));
    let _ = f2a_request_body(
        &server.requests,
        "EC-K4 incomplete batch publication failure",
    );
    f2a_join_server(server, "EC-K4 incomplete batch publication failure");
    let events = session_events(root.path(), &session_id(root.path()));
    assert!(!events.iter().any(|event| {
        event["event_type"] == "tool.subcommand_checkpointed"
            || event["event_type"] == "tool.execution_outcome_recorded"
            || event["payload"]["item"]["kind"] == "tool_result"
    }));
}
