//! Generic task continuation through the real CLI, persistence and providers.

use super::*;

fn definition(root: &std::path::Path) -> std::path::PathBuf {
    let directory = root.join("writing-agent");
    fs::create_dir(&directory).unwrap();
    fs::write(
        directory.join("persona.md"),
        "You are a technical writer. PINNED-WRITER-IDENTITY.",
    )
    .unwrap();
    fs::write(
        directory.join("work.md"),
        "Explain the supplied material clearly, preserving uncertainty.",
    )
    .unwrap();
    fs::write(
        directory.join("agent.toml"),
        r#"schema_version = 1
id = "summary-writer"
name = "Summary writer"
[instructions]
persona_file = "persona.md"
work_file = "work.md"
method_files = []
[context]
personal = "none"
project = "none"
memory = "none"
[tools]
view_image = "direct"
"#,
    )
    .unwrap();
    directory.join("agent.toml")
}

#[test]
fn generic_image_compacts_without_target_and_resumes_deleted_definition() {
    let root = TemporaryDirectory::new("generic-image-compaction");
    initialize_project(root.path());
    let definition = definition(root.path());
    fs::write(root.path().join("screen.png"), png_bytes(32, 16)).unwrap();
    let primary = spawn_responses_server(vec![(
        200,
        text_response(&"Detailed writing notes. ".repeat(1000), "generic-first"),
    )]);
    let vision = spawn_responses_server(vec![(
        200,
        vision_response("A synthetic diagram.", "generic-vision"),
    )]);
    let config = write_dual_config(root.path(), &primary.base_url, &vision.base_url);
    let output = run_chat(
        root.path(),
        &config,
        "/image screen.png\nExplain this diagram.\n/compact\n/exit\n",
        &["--agent", definition.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Compaction revision 1 applied"));
    let primary_wire = request_body(
        &primary
            .requests
            .recv_timeout(Duration::from_secs(10))
            .unwrap(),
    );
    let vision_wire = request_body(
        &vision
            .requests
            .recv_timeout(Duration::from_secs(10))
            .unwrap(),
    );
    primary.join();
    vision.join();
    assert!(
        primary_wire["instructions"]
            .as_str()
            .unwrap()
            .contains("PINNED-WRITER-IDENTITY")
    );
    assert!(
        !vision_wire["instructions"]
            .as_str()
            .unwrap()
            .contains("vulnerability")
    );
    assert!(
        vision_wire["instructions"]
            .as_str()
            .unwrap()
            .contains("task_observations")
    );
    let project = Project::discover(root.path()).unwrap();
    let session = session_id(root.path());
    let state = reduced_session(&project, &session);
    let summary = serde_json::to_value(state.latest_compaction().unwrap().summary()).unwrap();
    assert_eq!(summary["schema_version"], 3);
    assert!(summary["evidence_spine"].is_null());
    assert_eq!(
        summary["task_references"]["definition"]["agent_id"],
        "summary-writer"
    );
    assert!(
        !summary["task_references"]["artifact_refs"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        !summary["task_references"]["execution_receipt_refs"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        !summary["task_references"]["history_event_refs"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(state.vision_observations()[0].schema_version, 3);
    let observation = serde_json::to_value(&state.vision_observations()[0]).unwrap();
    assert!(observation.get("security_relevant_clues").is_none());
    assert_eq!(
        observation["task_observations"],
        json!(["fixture boundary marker"])
    );
    let mut writer = project.open_writer().unwrap();
    let store = writer.open_research(None).unwrap();
    let events =
        EventSource::read_events(&store, &ProjectEventStream::Research.stream_id()).unwrap();
    assert!(
        events
            .iter()
            .all(|event| !event.event_type.as_str().contains("target_snapshot"))
    );
    drop(store);
    drop(writer);
    fs::remove_dir_all(definition.parent().unwrap()).unwrap();
    let resumed_server = spawn_responses_server(vec![(
        200,
        text_response(
            "Continued from the saved writer binding.",
            "generic-resumed",
        ),
    )]);
    let config = write_config(root.path(), &resumed_server.base_url);
    let resumed = run_chat(
        root.path(),
        &config,
        "Continue the document.\n/exit\n",
        &["--resume", "latest"],
    );
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    let wire = request_body(
        &resumed_server
            .requests
            .recv_timeout(Duration::from_secs(10))
            .unwrap(),
    );
    resumed_server.join();
    assert!(wire["previous_response_id"].is_null());
    assert!(
        wire["instructions"]
            .as_str()
            .unwrap()
            .contains("PINNED-WRITER-IDENTITY")
    );
    assert!(
        serde_json::to_string(&wire)
            .unwrap()
            .contains("derived_session_summary_v3")
    );
    let resumed_state = reduced_session(&project, &session);
    assert_eq!(
        resumed_state
            .latest_compaction()
            .unwrap()
            .summary()
            .fingerprint(),
        state.latest_compaction().unwrap().summary().fingerprint()
    );
    assert_audits_reconstruct(root.path(), &resumed_state);
}

#[test]
fn generic_summary_rejects_cross_session_wrong_prefix_and_forged_references() {
    let root = TemporaryDirectory::new("generic-summary-negative");
    initialize_project(root.path());
    let definition = definition(root.path());
    let server = spawn_responses_server(vec![(
        200,
        text_response(
            &"Durable writing progress. ".repeat(1000),
            "generic-negative",
        ),
    )]);
    let config = write_config(root.path(), &server.base_url);
    let output = run_chat(
        root.path(),
        &config,
        "Draft the document.\n/compact\n/exit\n",
        &["--agent", definition.to_str().unwrap()],
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
    let valid_resume = run_chat(root.path(), &config, "/exit\n", &["--resume", "latest"]);
    assert!(
        valid_resume.status.success(),
        "{}",
        String::from_utf8_lossy(&valid_resume.stderr)
    );
    let session = StreamId::new(session_id(root.path()));
    let path = root
        .path()
        .join(".cyber-agent/sessions")
        .join(session.as_str())
        .join("events.jsonl");
    let original = fs::read_to_string(&path).unwrap();
    let events = original
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    let index = events
        .iter()
        .position(|event| event["event_type"] == "session.compaction_created")
        .unwrap();
    let config = write_config(root.path(), "http://127.0.0.1:1/v1");
    for (pointer, replacement, expected) in [
        (
            "/task_references/source/stream_id",
            json!("session:another"),
            "prefix",
        ),
        ("/task_references/source/watermark", json!(1), "prefix"),
        (
            "/task_references/source/stream_hash",
            json!("0".repeat(64)),
            "prefix",
        ),
        (
            "/task_references/source/project_stream_hash",
            json!("0".repeat(64)),
            "prefix",
        ),
        (
            "/task_references/definition/definition_digest",
            json!("0".repeat(64)),
            "provenance",
        ),
        (
            "/task_references/artifact_refs",
            json!(["artifact:absent"]),
            "reference",
        ),
        (
            "/task_references/execution_receipt_refs",
            json!(["receipt:absent"]),
            "reference",
        ),
        (
            "/task_references/history_event_refs",
            json!(["history:absent"]),
            "reference",
        ),
    ] {
        let mut changed = events.clone();
        let summary = &mut changed[index]["payload"]["summary"];
        *summary.pointer_mut(pointer).unwrap() = replacement;
        summary["fingerprint"] = json!("");
        summary["fingerprint"] = json!(
            blake3::hash(&serde_json::to_vec(summary).unwrap())
                .to_hex()
                .to_string()
        );
        let raw = changed
            .iter()
            .map(|event| serde_json::to_string(event).unwrap())
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        fs::write(&path, raw).unwrap();
        let resumed = run_chat(root.path(), &config, "/exit\n", &["--resume", "latest"]);
        let stderr = String::from_utf8_lossy(&resumed.stderr);
        assert!(!resumed.status.success(), "accepted forged {pointer}");
        assert!(
            !stderr.contains("fingerprint"),
            "hash fixture failed before semantic validation: {stderr}"
        );
        assert!(
            stderr.to_lowercase().contains(expected),
            "{pointer}: {stderr}"
        );
    }
    fs::write(path, original).unwrap();
}

#[test]
fn immediate_reload_compaction_budgets_the_new_instruction_binding() {
    let root = TemporaryDirectory::new("generic-reload-compaction");
    initialize_project(root.path());
    let original = definition(root.path());
    let revision = root.path().join("writer-revision");
    fs::create_dir(&revision).unwrap();
    fs::copy(&original, revision.join("agent.toml")).unwrap();
    fs::copy(
        original.parent().unwrap().join("work.md"),
        revision.join("work.md"),
    )
    .unwrap();
    fs::write(revision.join("persona.md"), "RELOADED WRITER. ".repeat(200)).unwrap();
    let server = spawn_responses_server(vec![
        (200, text_response(&"A".repeat(8_000), "before-reload-one")),
        (200, text_response(&"B".repeat(4_000), "before-reload-two")),
        (
            200,
            text_response("Continue with the new definition.", "after-reload"),
        ),
    ]);
    let config = write_context_window_config(root.path(), &server.base_url, 8192);
    let input = format!(
        "Draft the first part.\nDraft the second part.\n/agent reload {}\n/compact\nContinue.\n/exit\n",
        revision.join("agent.toml").display()
    );
    let output = run_chat(
        root.path(),
        &config,
        &input,
        &["--agent", original.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("reloaded at definition"), "{stdout}");
    assert!(stdout.contains("Compaction revision 1 applied"), "{stdout}");
    let mut wires = Vec::new();
    for _ in 0..3 {
        wires.push(request_body(
            &server
                .requests
                .recv_timeout(Duration::from_secs(10))
                .unwrap(),
        ));
    }
    server.join();
    let project = Project::discover(root.path()).unwrap();
    let state = reduced_session(&project, &session_id(root.path()));
    let compaction = state.latest_compaction().unwrap();
    let summary = serde_json::to_value(compaction.summary()).unwrap();
    assert_eq!(
        summary["task_references"]["definition"]["definition_digest"],
        state
            .agent_definition_binding()
            .unwrap()
            .definition()
            .digest()
            .as_str()
    );
    assert!(
        serde_json::to_string(&wires[2])
            .unwrap()
            .contains(&"B".repeat(4_000)),
        "candidate retention must fit the new instruction budget without truncation"
    );
    assert!(
        wires[2]["instructions"]
            .as_str()
            .unwrap()
            .contains("RELOADED WRITER.")
    );
    assert!(
        !wires[2]["instructions"]
            .as_str()
            .unwrap()
            .contains("PINNED-WRITER-IDENTITY")
    );
    assert!(wires[2]["previous_response_id"].is_null());
    assert_audits_reconstruct(root.path(), &state);
}
