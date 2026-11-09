//! Resume must reconstruct task-scoped Compaction without replaying effects.
use super::task_context::{ScriptedResponse, definition, spawn_scripted_responses_server};
use super::*;

fn select_original_for_later_audit(candidates: &Value) -> Value {
    let source = candidates["context"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["excerpt"] == "LATER-AUDIT-ORIGINAL-GOAL")
        .unwrap();
    json!({"relation":"continue", "focus":{"kind":"existing", "task_ref":candidates["tasks"][0]["ref"]},
        "related_task_refs":[], "required_context_refs":[source["ref"]], "unresolved_references":[]})
}

#[test]
fn later_main_audit_rejects_coherent_substitution_for_full_and_incremental() {
    use echidna::domain::model::{ContentItem, ModelInputIR};
    use echidna::domain::request_audit::RequestAuditManifest;
    use sha2::{Digest, Sha256};

    for incremental in [false, true] {
        let root = TemporaryDirectory::new("task-later-audit-forgery");
        initialize_project(root.path());
        let agent = definition(root.path(), None);
        let server = spawn_scripted_responses_server(vec![
            ScriptedResponse::InitialSelection,
            ScriptedResponse::Response(200, text_response("Initial result.", "audit-original")),
            ScriptedResponse::SelectionDecision(select_original_for_later_audit),
            ScriptedResponse::Response(
                200,
                m4_long_flow_tool_response(
                    "later-audit-tool",
                    "run_command",
                    json!({"argv":["/bin/echo","actual tool output"]}),
                    "audit-tool",
                ),
            ),
            ScriptedResponse::Response(200, text_response("Finished.", "audit-finished")),
        ]);
        let config = write_context_window_config(root.path(), &server.base_url, 272_000);
        fs::write(
            &config,
            fs::read_to_string(&config).unwrap().replace(
                "incremental_input = true",
                &format!("incremental_input = {incremental}"),
            ),
        )
        .unwrap();
        let output = run_chat(
            root.path(),
            &config,
            "LATER-AUDIT-ORIGINAL-GOAL\nContinue this task.\n/exit\n",
            &["--agent", agent.to_str().unwrap()],
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        for _ in 0..5 {
            f2a_request_body(&server.requests, "later main audit");
        }
        server.join();
        let session = session_id(root.path());
        let events = session_events(root.path(), &session);
        let position = events
            .iter()
            .rposition(|event| event["event_type"] == "session.request_audit_recorded")
            .unwrap();
        let prefix = events[..=position].to_vec();
        let event_path = root
            .path()
            .join(".echidna/sessions")
            .join(&session)
            .join("events.jsonl");
        let write_events = |values: &[Value]| {
            fs::write(
                &event_path,
                values
                    .iter()
                    .map(|event| format!("{}\n", serde_json::to_string(event).unwrap()))
                    .collect::<String>(),
            )
            .unwrap()
        };
        write_events(&prefix);
        let valid = run_chat(root.path(), &config, "/exit\n", &["--resume", "latest"]);
        assert!(
            valid.status.success(),
            "{}",
            String::from_utf8_lossy(&valid.stderr)
        );
        let mut forged = prefix.clone();
        let payload = &mut forged[position]["payload"];
        assert_eq!(
            payload["delivery"]["mode"],
            if incremental { "incremental" } else { "full" }
        );
        let blobs = root.path().join(".echidna/artifacts/blobs/sha256");
        let components = payload["full_logical_components"].as_array().unwrap();
        let read = |component: &Value| {
            fs::read(blobs.join(component["blob_id"].as_str().unwrap())).unwrap()
        };
        let instructions = String::from_utf8(read(&components[0])).unwrap();
        let mut items = Vec::new();
        let mut tools = Vec::new();
        let mut requirements = Value::Null;
        for component in &components[1..] {
            match component["identity"]["kind"].as_str().unwrap() {
                "content_item" => {
                    items.push(serde_json::from_slice::<Value>(&read(component)).unwrap())
                }
                "tool_spec" => {
                    tools.push(serde_json::from_slice::<Value>(&read(component)).unwrap())
                }
                "request_requirements" => {
                    requirements = serde_json::from_slice(&read(component)).unwrap()
                }
                other => panic!("unexpected component {other}"),
            }
        }
        let mut logical: ModelInputIR = serde_json::from_value(json!({"schema_version":payload["full_logical_input"]["schema_version"], "instructions":instructions, "content_items":items, "tool_specs":tools, "requirements":requirements})).unwrap();
        let item_index = logical
            .content_items
            .iter()
            .position(|item| *item == ContentItem::user_text("LATER-AUDIT-ORIGINAL-GOAL"))
            .unwrap();
        let replacement = ContentItem::user_text("COHERENT-SUBSTITUTED-OLD-GOAL");
        logical.content_items[item_index] = replacement.clone();
        let bytes = delivery_lineage::canonical_bytes(&replacement);
        let blob = format!("{:x}", Sha256::digest(&bytes));
        fs::write(blobs.join(&blob), &bytes).unwrap();
        let hash = delivery_lineage::canonical_hash(&replacement);
        for (components_key, map_key) in [("full_logical_components", "full_context_delivery_map")]
            .into_iter()
            .chain((!incremental).then_some(("components", "context_delivery_map")))
        {
            let component = payload[components_key]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|component| {
                    component["identity"]["kind"] == "content_item"
                        && component["identity"]["index"] == item_index
                })
                .unwrap();
            component["blob_id"] = json!(blob);
            component["size_bytes"] = json!(bytes.len());
            component["semantic_hash"] = json!(hash);
            let entry = payload[map_key]["content_items"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|entry| entry["final_position"] == item_index)
                .unwrap();
            entry["content_hash"] = json!(hash);
        }
        let logical_hash = delivery_lineage::canonical_hash(&logical);
        payload["full_logical_input"]["semantic_hash"] = json!(logical_hash);
        if !incremental {
            payload["delivered_input"]["semantic_hash"] = json!(logical_hash);
        }
        let manifest: RequestAuditManifest = serde_json::from_value(payload.clone()).unwrap();
        manifest.validate().unwrap();
        let decoded = forged
            .iter()
            .cloned()
            .map(|event| serde_json::from_value::<EventEnvelope<Value>>(event).unwrap())
            .collect::<Vec<_>>();
        SessionReducer
            .reduce(decoded)
            .expect("coherent forgery must pass structural replay");
        write_events(&forged);
        let before = fs::read(&event_path).unwrap();
        let rejected = run_chat(root.path(), &config, "/exit\n", &["--resume", "latest"]);
        assert!(
            !rejected.status.success(),
            "coherent forged {incremental:?} audit was accepted"
        );
        assert!(
            String::from_utf8_lossy(&rejected.stderr)
                .contains("primary audit differs from independently compiled task source"),
            "{}",
            String::from_utf8_lossy(&rejected.stderr)
        );
        assert_eq!(fs::read(&event_path).unwrap(), before);
    }
}

#[test]
fn resumed_compaction_reconstructs_selected_task_and_preserves_prior_effects() {
    let root = TemporaryDirectory::new("task-context-resume-compact");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let first_body = format!("FIRST-TASK-BODY {}", "original source detail ".repeat(3000));
    let next_body = format!(
        "AFTER-FIRST-COMPACTION {}",
        "current source detail ".repeat(3000)
    );
    let initial = spawn_scripted_responses_server(vec![
        ScriptedResponse::InitialSelection,
        ScriptedResponse::Response(
            200,
            m4_long_flow_tool_response(
                "resume-once",
                "run_command",
                json!({"argv":["/bin/sh","-c","printf 'once\\n' >> resume-count.txt"]}),
                "resume-tool",
            ),
        ),
        ScriptedResponse::Response(200, text_response(&first_body, "resume-first-answer")),
        ScriptedResponse::Response(
            200,
            compaction_summary_response(
                "FIRST-VERIFIED-SUMMARY: preserve the original task and result.",
                "resume-first-summary",
            ),
        ),
        ScriptedResponse::ContinueSelection { usage: None },
        ScriptedResponse::ContinueSelection { usage: None },
        ScriptedResponse::Response(200, text_response(&next_body, "resume-next-answer")),
    ]);
    let config = write_context_window_config(root.path(), &initial.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "Perform the synthetic counted task.\n/compact\nContinue the counted task using its earlier result.\n/exit\n",
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for _ in 0..7 {
        f2a_request_body(&initial.requests, "initial task and compaction");
    }
    assert!(initial.requests.try_recv().is_err());
    initial.join();
    let session = session_id(root.path());
    let before = session_events(root.path(), &session);
    assert_eq!(
        before
            .iter()
            .filter(|event| event["event_type"] == "session.compaction_created")
            .count(),
        1
    );
    assert_eq!(
        fs::read_to_string(root.path().join("resume-count.txt")).unwrap(),
        "once\n"
    );

    let resumed_server = spawn_responses_server(vec![(
        200,
        compaction_summary_response(
            "SECOND-VERIFIED-SUMMARY: preserve the same task and its recorded result.",
            "resume-second-summary",
        ),
    )]);
    let config = write_context_window_config(root.path(), &resumed_server.base_url, 272_000);
    let resumed = run_chat(
        root.path(),
        &config,
        "/compact\n/exit\n",
        &["--resume", "latest"],
    );
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    assert!(
        String::from_utf8_lossy(&resumed.stdout).contains("Compaction revision 2 applied"),
        "{}",
        String::from_utf8_lossy(&resumed.stdout)
    );
    let handoff = f2a_request_body(&resumed_server.requests, "resumed task compaction");
    assert!(
        handoff
            .get("previous_response_id")
            .is_none_or(Value::is_null)
    );
    assert!(json_contains_text(
        &handoff["input"],
        "AFTER-FIRST-COMPACTION"
    ));
    assert!(!json_contains_text(&handoff["input"], "FIRST-TASK-BODY"));
    assert!(resumed_server.requests.try_recv().is_err());
    resumed_server.join();
    let after = session_events(root.path(), &session);
    assert_eq!(
        after
            .iter()
            .filter(|event| event["event_type"] == "session.compaction_created")
            .count(),
        2
    );
    assert_eq!(
        after
            .iter()
            .filter(|event| event["event_type"] == "tool.execution_outcome_recorded")
            .count(),
        1
    );
    assert_eq!(
        fs::read_to_string(root.path().join("resume-count.txt")).unwrap(),
        "once\n"
    );
}

#[test]
fn resumed_compaction_never_replays_or_hides_unknown_execution() {
    let root = TemporaryDirectory::new("task-context-resume-unknown-compact");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let server = spawn_scripted_responses_server(vec![
        ScriptedResponse::InitialSelection,
        ScriptedResponse::Response(
            200,
            m4_long_flow_tool_response(
                "resume-unknown",
                "run_command",
                json!({"argv":["/bin/sh","-c","printf 'started\\n' >> unknown-count.txt; exec /bin/sleep 30"]}),
                "unknown-response",
            ),
        ),
    ]);
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let child = spawn_f2a_chat(
        root.path(),
        &config,
        "Start the counted task.\n/exit\n",
        &["--agent", agent.to_str().unwrap()],
    );
    super::task_context::selector_candidates(&f2a_request_body(
        &server.requests,
        "unknown execution selection",
    ));
    f2a_request_body(&server.requests, "unknown execution request");
    let count = root.path().join("unknown-count.txt");
    let deadline = Instant::now() + F2A_WAIT;
    while !count.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(count.exists());
    assert!(
        Command::new("/bin/kill")
            .args(["-INT", &child.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    let stopped = f2a_wait_child(child, "unknown execution");
    assert!(!stopped.status.success());
    assert!(String::from_utf8_lossy(&stopped.stderr).contains("external effects are unknown"));
    server.join();
    let session = session_id(root.path());
    let before = session_events(root.path(), &session);
    let _ = run_chat(
        root.path(),
        &config,
        "/compact\n/exit\n",
        &["--resume", "latest"],
    );
    let after = session_events(root.path(), &session);
    for kind in [
        "session.request_audit_recorded",
        "session.compaction_created",
        "tool.execution_outcome_recorded",
    ] {
        assert_eq!(
            after
                .iter()
                .filter(|event| event["event_type"] == kind)
                .count(),
            before
                .iter()
                .filter(|event| event["event_type"] == kind)
                .count()
        );
    }
    assert_eq!(fs::read_to_string(count).unwrap(), "started\n");
}

#[test]
fn resumed_mixed_tool_batch_preserves_known_and_unknown_through_compaction() {
    use std::os::unix::fs::OpenOptionsExt;

    let root = TemporaryDirectory::new("task-resume-mixed-batch");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let manifest = agent.join("agent.toml");
    let mut definition = fs::read_to_string(&manifest).unwrap();
    definition.push_str("read_file = \"direct\"\n");
    fs::write(manifest, definition).unwrap();
    let fast_path = root.path().join("fast.txt");
    fs::write(&fast_path, "ORIGINAL-KNOWN-RESULT\n").unwrap();
    let fifo_path = root.path().join("pending.pipe");
    assert!(
        Command::new("/usr/bin/mkfifo")
            .arg(&fifo_path)
            .status()
            .unwrap()
            .success()
    );
    let initial = super::task_context::spawn_scripted_responses_server_with_wait(
        vec![
            ScriptedResponse::InitialSelection,
            ScriptedResponse::Response(
                200,
                tool_calls_response(
                    &[
                        ("mixed-fast", "read_file", json!({"path":"fast.txt"})),
                        ("mixed-pending", "read_file", json!({"path":"pending.pipe"})),
                    ],
                    "mixed-batch-response",
                ),
            ),
        ],
        Duration::from_secs(60),
    );
    let config = write_context_window_config(root.path(), &initial.base_url, 272_000);
    let mut child = spawn_f2a_chat(
        root.path(),
        &config,
        "Read both synthetic sources and preserve their outcomes.\n/exit\n",
        &["--agent", agent.to_str().unwrap()],
    );
    super::task_context::selector_candidates(&f2a_request_body(
        &initial.requests,
        "mixed batch initial selection",
    ));
    f2a_request_body(&initial.requests, "mixed batch main sample");
    initial.join();
    let session = session_id(root.path());
    let events_path = root
        .path()
        .join(".echidna/sessions")
        .join(&session)
        .join("events.jsonl");
    let deadline = Instant::now() + Duration::from_secs(60);
    // A successful nonblocking writer open proves the other execution has
    // actually opened the FIFO. Keep it open without writing so that read
    // remains in flight until the process is terminated.
    let fifo_writer = loop {
        match fs::OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&fifo_path)
        {
            Ok(writer) => break writer,
            Err(error) if error.raw_os_error() == Some(libc::ENXIO) => {
                assert!(Instant::now() < deadline, "FIFO reader did not start");
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("FIFO writer setup failed: {error}"),
        }
    };
    loop {
        let bytes = fs::read_to_string(&events_path).unwrap();
        if bytes.ends_with('\n') {
            let events = bytes
                .lines()
                .map(|line| serde_json::from_str::<Value>(line).unwrap());
            if events
                .filter(|event| event["event_type"] == "tool.execution_outcome_recorded")
                .count()
                == 1
            {
                break;
            }
        }
        assert!(
            Instant::now() < deadline,
            "fast outcome was not durably recorded"
        );
        thread::sleep(Duration::from_millis(10));
    }
    child.kill().unwrap();
    let stopped = f2a_wait_child(child, "mixed batch interrupted");
    assert!(!stopped.status.success());
    drop(fifo_writer);
    let prefix = fs::read(&events_path).unwrap();
    let before = session_events(root.path(), &session);
    let calls = before
        .iter()
        .filter(|event| {
            event["event_type"] == "session.history_item_recorded"
                && event["payload"]["item"]["kind"] == "tool_call"
        })
        .map(|event| {
            event["payload"]["item"]["call"]["call_id"]
                .as_str()
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(calls, ["mixed-fast", "mixed-pending"]);
    let outcomes = before
        .iter()
        .filter(|event| event["event_type"] == "tool.execution_outcome_recorded")
        .collect::<Vec<_>>();
    assert_eq!(outcomes.len(), 1);
    let recorded = outcomes[0]["payload"]["result"].clone();
    assert_eq!(recorded["call_id"], "mixed-fast");
    assert_eq!(recorded["status"], "succeeded");
    assert_eq!(
        outcomes[0]["payload"]["receipt"]["execution_id"],
        "execution:mixed-fast"
    );
    assert!(!before.iter().any(
        |event| event["event_type"] == "session.history_item_recorded"
            && event["payload"]["item"]["kind"] == "tool_result"
    ));
    fs::write(&fast_path, "CHANGED-AFTER-INTERRUPTION\n").unwrap();

    let answer = format!(
        "MIXED-RECOVERY-COMPLETE {}",
        "durable continuation detail ".repeat(3_000)
    );
    let resumed_server = super::task_context::spawn_scripted_responses_server_with_wait(
        vec![
            ScriptedResponse::ContinueSelection { usage: None },
            ScriptedResponse::Response(200, text_response(&answer, "mixed-resume-main")),
            ScriptedResponse::Response(
                200,
                compaction_summary_response(
                    "Preserve the known result and the unresolved execution.",
                    "mixed-summary",
                ),
            ),
        ],
        Duration::from_secs(60),
    );
    let config = write_context_window_config(root.path(), &resumed_server.base_url, 272_000);
    let resumed = run_chat(
        root.path(),
        &config,
        "Continue this task with the known and unknown results.\n/compact\n/exit\n",
        &["--resume", &session],
    );
    assert!(
        resumed.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&resumed.stdout),
        String::from_utf8_lossy(&resumed.stderr)
    );
    assert!(
        String::from_utf8_lossy(&resumed.stdout).contains("Compaction revision 1 applied"),
        "{}",
        String::from_utf8_lossy(&resumed.stdout)
    );
    super::task_context::selector_candidates(&f2a_request_body(
        &resumed_server.requests,
        "mixed resume selection",
    ));
    let wire = f2a_request_body(&resumed_server.requests, "mixed resume main");
    let handoff = f2a_request_body(&resumed_server.requests, "mixed resume compaction");
    assert!(resumed_server.requests.try_recv().is_err());
    resumed_server.join();
    assert!(
        handoff
            .get("previous_response_id")
            .is_none_or(Value::is_null)
    );
    let items = wire["input"].as_array().unwrap();
    let results = items
        .iter()
        .filter(|item| item["type"] == "function_call_output")
        .collect::<Vec<_>>();
    assert_eq!(results.len(), 2);
    for call in ["mixed-fast", "mixed-pending"] {
        assert_eq!(
            items
                .iter()
                .filter(|item| item["type"] == "function_call" && item["call_id"] == call)
                .count(),
            1
        );
        assert_eq!(
            results
                .iter()
                .filter(|item| item["call_id"] == call)
                .count(),
            1
        );
    }
    let result = |call| {
        let item = results.iter().find(|item| item["call_id"] == call).unwrap();
        serde_json::from_str::<Value>(item["output"].as_str().unwrap()).unwrap()
    };
    let fast = result("mixed-fast");
    assert_eq!(
        fast,
        json!({"status":recorded["status"], "output":recorded["output"]})
    );
    assert!(!fast.to_string().contains("CHANGED-AFTER-INTERRUPTION"));
    let pending = result("mixed-pending");
    assert_eq!(pending["status"], "cancelled");
    assert_eq!(pending["output"]["execution_status"], "unknown");
    assert_eq!(
        pending["output"]["guidance"],
        "do_not_assume_no_effect_or_auto_rerun"
    );
    assert!(fs::read(&events_path).unwrap().starts_with(&prefix));
    let after = session_events(root.path(), &session);
    assert_eq!(
        after
            .iter()
            .filter(|event| event["event_type"] == "tool.execution_outcome_recorded")
            .count(),
        1
    );
    assert_eq!(
        after
            .iter()
            .filter(|event| event["event_type"] == "tool.execution_receipt_recorded")
            .count(),
        0
    );
    assert!(!after.iter().any(
        |event| event["event_type"] == "session.history_item_recorded"
            && event["payload"]["item"]["kind"] == "tool_result"
    ));
    let resume_index = after
        .iter()
        .position(|event| event["event_type"] == "session.resumed")
        .unwrap();
    let compact_index = after
        .iter()
        .position(|event| event["event_type"] == "session.compaction_created")
        .unwrap();
    assert!(resume_index < compact_index);
    let project = Project::discover(root.path()).unwrap();
    let state = reduced_session(&project, &session);
    assert_eq!(state.execution_receipt_count(), 1);
    assert_audits_reconstruct(root.path(), &state);
}
