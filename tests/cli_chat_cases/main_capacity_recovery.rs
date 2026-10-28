use super::*;

#[test]
fn research_refresh_retains_completed_turn_after_root_when_capacity_shrinks() {
    let root = TemporaryDirectory::new("main-stale-completed-turn");
    initialize_project(root.path());
    let project = Project::discover(root.path()).unwrap();
    append_research_snapshot(&project, "completed-old");
    let large_answer = format!(
        "COMPLETED-ASSISTANT-MARKER {}",
        "answer-data-".repeat(100_000)
    );
    let server = spawn_responses_server(vec![
        (
            200,
            text_response(&"Earlier observation. ".repeat(500), "completed-initial"),
        ),
        (
            200,
            compaction_handoff_response("Earlier observations.", "completed-root"),
        ),
        (
            200,
            tool_calls_response(
                &[(
                    "completed-call",
                    "run_command",
                    json!({
                        "argv": ["/usr/bin/printf", "%s", "COMPLETED-TOOL-RESULT"]
                    }),
                )],
                "completed-tool",
            ),
        ),
        (200, text_response(&large_answer, "completed-answer")),
    ]);
    let config = write_context_window_config(root.path(), &server.base_url, 1_000_000);
    let first = run_chat(
        root.path(),
        &config,
        "Initial task.\n/compact\n/phase experiment\nRun the command.\n/exit\n",
        &[],
    );
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(String::from_utf8_lossy(&first.stdout).contains("Compaction revision 1 applied"));
    for _ in 0..4 {
        server.requests.recv_timeout(F2A_WAIT).unwrap();
    }
    server.join();
    let session = session_id(root.path());
    let completed_items = session_events(root.path(), &session)
        .into_iter()
        .filter(|event| event["event_type"] == "session.history_item_recorded")
        .map(|event| event["payload"]["item"].clone())
        .filter(|item| {
            item["call"]["call_id"] == "completed-call"
                || item["result"]["call_id"] == "completed-call"
                || item["text"].as_str() == Some(large_answer.as_str())
        })
        .collect::<Vec<_>>();
    assert_eq!(completed_items.len(), 3);
    append_research_snapshot(&project, "completed-current");
    let offline = write_context_window_config(root.path(), "http://127.0.0.1:1/v1", 272_000);
    for _ in 0..2 {
        let resumed = run_chat(
            root.path(),
            &offline,
            "/context\n/exit\n",
            &["--resume", &session],
        );
        assert!(!resumed.status.success());
        assert!(String::from_utf8_lossy(&resumed.stderr).contains("unresolved stopped Turn"));
        let stdout = String::from_utf8_lossy(&resumed.stdout);
        assert!(
            stdout.contains("Context exceeds available capacity; use /context, /compact or /exit"),
            "{stdout}"
        );
        let events = session_events(root.path(), &session);
        let refreshed = events
            .iter()
            .rfind(|event| event["event_type"] == "session.compaction_created")
            .unwrap();
        let retained = refreshed["payload"]["retained_history"].as_array().unwrap();
        for expected in &completed_items {
            assert_eq!(
                retained.iter().filter(|item| *item == expected).count(),
                1,
                "a complete post-root History item was lost or duplicated"
            );
        }
        assert_eq!(
            events
                .iter()
                .filter(|event| event["event_type"] == "session.compaction_created")
                .count(),
            2
        );
    }
}

#[test]
fn stale_verified_handoff_with_oversized_input_restores_commands_after_strict_source_check() {
    let root = TemporaryDirectory::new("main-stale-oversized-resume");
    initialize_project(root.path());
    let project = Project::discover(root.path()).unwrap();
    append_research_snapshot(&project, "oversized-old");
    let server = spawn_responses_server(vec![
        (
            200,
            text_response(&"Earlier observation. ".repeat(500), "oversized-initial"),
        ),
        (
            200,
            compaction_handoff_response(
                "Earlier observations remain available.",
                "oversized-handoff",
            ),
        ),
        (
            400,
            r#"{"error":{"code":"context_length_exceeded","message":"fixture limit"}}"#.to_owned(),
        ),
    ]);
    let config = write_context_window_config(root.path(), &server.base_url, 1_000_000);
    let large = "protected-input-".repeat(75_000);
    let first = run_chat(
        root.path(),
        &config,
        &format!("Initial task.\n/compact\n{large}\n/exit\n"),
        &[],
    );
    assert!(!first.status.success());
    assert!(String::from_utf8_lossy(&first.stdout).contains("Compaction revision 1 applied"));
    for _ in 0..3 {
        server.requests.recv_timeout(F2A_WAIT).unwrap();
    }
    server.join();
    let session = session_id(root.path());
    let before = session_events(root.path(), &session);
    let original = before
        .iter()
        .find(|event| event["event_type"] == "session.compaction_created")
        .unwrap()["payload"]["summary"]
        .clone();
    let capture = before
        .iter()
        .find(|event| event["event_type"] == "session.compaction_context_captured")
        .unwrap()["payload"]["blob_id"]
        .as_str()
        .unwrap();
    let capture_path = root
        .path()
        .join(".echidna/artifacts/blobs/sha256")
        .join(capture);
    let capture_bytes = fs::read(&capture_path).unwrap();
    let current_target = append_research_snapshot(&project, "oversized-current");
    let offline = write_context_window_config(root.path(), "http://127.0.0.1:1/v1", 272_000);

    fs::write(&capture_path, b"corrupted capture").unwrap();
    let rejected = run_chat(
        root.path(),
        &offline,
        "/context\n/exit\n",
        &["--resume", &session],
    );
    assert!(!rejected.status.success());
    assert!(
        String::from_utf8_lossy(&rejected.stderr).contains("Compaction context Blob read failed")
    );
    assert_eq!(session_events(root.path(), &session), before);
    fs::write(&capture_path, capture_bytes).unwrap();

    let started = std::time::Instant::now();
    let resumed = run_chat(
        root.path(),
        &offline,
        "/context\n/compact\nDO-NOT-SEND\n/exit\n",
        &["--resume", &session],
    );
    assert!(
        started.elapsed() < std::time::Duration::from_secs(30),
        "mandatory input preflight took {:?}",
        started.elapsed()
    );
    assert!(!resumed.status.success());
    assert!(String::from_utf8_lossy(&resumed.stderr).contains("unresolved stopped Turn"));
    let stdout = String::from_utf8_lossy(&resumed.stdout);
    assert!(
        stdout.contains("Context exceeds available capacity; use /context, /compact or /exit"),
        "{stdout}"
    );
    assert!(
        stdout.contains("Task input not accepted or saved"),
        "{stdout}"
    );
    assert!(
        stdout.contains("Compaction could not find a smaller admissible Context"),
        "{stdout}"
    );
    let after = session_events(root.path(), &session);
    let summaries = after
        .iter()
        .filter(|event| event["event_type"] == "session.compaction_created")
        .collect::<Vec<_>>();
    assert_eq!(summaries.len(), 2);
    for event_type in [
        "session.compaction_context_captured",
        "session.request_audit_recorded",
    ] {
        assert_eq!(
            after
                .iter()
                .filter(|event| event["event_type"] == event_type)
                .count(),
            before
                .iter()
                .filter(|event| event["event_type"] == event_type)
                .count()
        );
    }
    let refreshed = &summaries[1]["payload"]["summary"];
    assert_eq!(
        refreshed["evidence_spine"]["target_snapshot_refs"][0],
        current_target.as_str()
    );
    assert_eq!(refreshed["narrative"], original["narrative"]);
    assert_eq!(refreshed["narrative_source"], original["narrative_source"]);
    assert_eq!(
        refreshed["model_sample_event_ref"],
        original["model_sample_event_ref"]
    );
    assert!(
        !serde_json::to_string(&after)
            .unwrap()
            .contains("DO-NOT-SEND")
    );
}

#[test]
fn research_refresh_preserves_the_original_large_handoff_allowance() {
    let root = TemporaryDirectory::new("main-stale-large-narrative");
    initialize_project(root.path());
    let project = Project::discover(root.path()).unwrap();
    append_research_snapshot(&project, "large-handoff-old");
    let narrative = "prior finding remains open; ".repeat(850);
    let server = spawn_responses_server(vec![
        (
            200,
            text_response(
                &"earlier observation ".repeat(5_000),
                "large-handoff-initial",
            ),
        ),
        (
            200,
            compaction_handoff_response(&narrative, "large-handoff-root"),
        ),
    ]);
    let config = write_context_window_config(root.path(), &server.base_url, 1_000_000);
    let first = run_chat(root.path(), &config, "Investigate.\n/compact\n/exit\n", &[]);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(String::from_utf8_lossy(&first.stdout).contains("Compaction revision 1 applied"));
    for _ in 0..2 {
        server.requests.recv_timeout(F2A_WAIT).unwrap();
    }
    server.join();
    let session = session_id(root.path());
    let before = session_events(root.path(), &session);
    let old = &before
        .iter()
        .find(|event| event["event_type"] == "session.compaction_created")
        .unwrap()["payload"]["summary"];
    assert!(serde_json::to_vec(&old["narrative"]).unwrap().len() > 16_320);
    let new_target = append_research_snapshot(&project, "large-handoff-current");
    let offline = write_context_window_config(root.path(), "http://127.0.0.1:1/v1", 272_000);
    let resumed = run_chat(
        root.path(),
        &offline,
        "/context\n/exit\n",
        &["--resume", &session],
    );
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    let after = session_events(root.path(), &session);
    let summaries = after
        .iter()
        .filter(|event| event["event_type"] == "session.compaction_created")
        .collect::<Vec<_>>();
    assert_eq!(summaries.len(), 2);
    let new = &summaries[1]["payload"]["summary"];
    assert_eq!(new["byte_limit"], old["byte_limit"]);
    assert_eq!(new["narrative"], old["narrative"]);
    assert_eq!(new["narrative_source"], old["narrative_source"]);
    assert_eq!(new["model_sample_event_ref"], old["model_sample_event_ref"]);
    assert_eq!(
        new["evidence_spine"]["target_snapshot_refs"][0],
        new_target.as_str()
    );
}

#[test]
fn configured_pressure_changes_fresh_context_guidance_without_a_model_request() {
    for (pressure, level) in [(1, "warning"), (10000, "normal")] {
        let root = TemporaryDirectory::new("main-configured-pressure");
        initialize_project(root.path());
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let config = write_config(
            root.path(),
            &format!("http://{}/v1", listener.local_addr().unwrap()),
        );
        let configured = format!(
            "{}\n[runtime.context_budget]\npressure_bps = {pressure}\ntarget_bps = 1\n",
            fs::read_to_string(&config).unwrap()
        );
        fs::write(&config, configured).unwrap();
        let output = run_chat(root.path(), &config, "/context\n/exit\n", &[]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains(&format!(
                "[current refreshed candidate] [context pressure: {level}]"
            )),
            "{stdout}"
        );
        assert!(
            stdout.contains("No successful model plan is available"),
            "{stdout}"
        );
        assert!(
            matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
        );
    }
}

#[test]
fn explicit_capacity_rejection_resumes_to_commands_without_replaying_or_saving_new_task() {
    let root = TemporaryDirectory::new("main-capacity-state");
    initialize_project(root.path());
    let server = spawn_responses_server(vec![(
        400,
        r#"{"error":{"code":"context_length_exceeded","message":"too long"}}"#.to_owned(),
    )]);
    let config = write_config(root.path(), &server.base_url);
    let output = run_chat(
        root.path(),
        &config,
        "ORIGINAL-CAPACITY-TASK\n/resume\n/context\nMUST-NOT-BE-SAVED\n/exit\n",
        &[],
    );
    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("use /context, /compact or /exit"),
        "{stdout}"
    );
    assert!(
        stdout.contains("Task input not accepted or saved"),
        "{stdout}"
    );
    assert!(
        stdout.contains("No successful model plan is available"),
        "{stdout}"
    );
    assert!(!stdout.contains("[last successful plan]"), "{stdout}");
    server.requests.recv_timeout(F2A_WAIT).unwrap();
    assert!(server.requests.try_recv().is_err());
    server.join();
    let state = reduced_session(
        &Project::discover(root.path()).unwrap(),
        &session_id(root.path()),
    );
    let history = serde_json::to_string(state.history().items()).unwrap();
    assert!(history.contains("ORIGINAL-CAPACITY-TASK"));
    assert!(!history.contains("MUST-NOT-BE-SAVED"));
    assert_eq!(state.request_audits().len(), 1);
    assert_eq!(state.execution_receipt_count(), 0);
}

#[test]
fn context_keeps_accepted_plan_after_a_later_capacity_rejection() {
    let root = TemporaryDirectory::new("main-successful-plan");
    initialize_project(root.path());
    let server = spawn_responses_server(vec![
        (200, text_response("First answer.", "accepted")),
        (
            400,
            r#"{"error":{"code":"context_length_exceeded","message":"too long"}}"#.to_owned(),
        ),
    ]);
    let config = write_config(root.path(), &server.base_url);
    let output = run_chat(
        root.path(),
        &config,
        "First input.\n/context\nSecond input.\n/context\n/exit\n",
        &[],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let plans = stdout
        .lines()
        .filter(|line| line.contains("[last successful plan]"))
        .collect::<Vec<_>>();
    assert_eq!(plans.len(), 2, "{stdout}");
    assert_eq!(plans[0], plans[1], "{stdout}");
    for _ in 0..2 {
        server.requests.recv_timeout(F2A_WAIT).unwrap();
    }
    assert!(server.requests.try_recv().is_err());
    server.join();
}
