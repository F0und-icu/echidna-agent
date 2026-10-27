//! Scripted responses prove delivery, not a model's adherence to guidance.
use super::*;
use echidna::domain::model::ContentItem;

#[test]
fn legacy_zero_history_compaction_cannot_bypass_latest_tool_batch_floor() {
    let root = TemporaryDirectory::new("legacy-empty-compaction-floor");
    initialize_project(root.path());
    fs::write(
        root.path().join("current.txt"),
        "RETAIN-CURRENT-RESULT ".repeat(900),
    )
    .unwrap();
    let project = Project::discover(root.path()).unwrap();
    let target = append_research_snapshot(&project, "legacy-zero-floor");
    let server = spawn_responses_server(vec![
        (
            200,
            m4_long_flow_tool_response(
                "latest-read",
                "read_file",
                json!({"path":"current.txt"}),
                "legacy-read",
            ),
        ),
        (200, text_response("Current read complete.", "legacy-final")),
    ]);
    let config = write_config(root.path(), &server.base_url);
    let initial = run_chat(root.path(), &config, "Read the current file.\n/exit\n", &[]);
    assert!(
        initial.status.success(),
        "{}",
        String::from_utf8_lossy(&initial.stderr)
    );
    for _ in 0..2 {
        server.requests.recv_timeout(F2A_WAIT).unwrap();
    }
    server.join();
    let id = session_id(root.path());
    append_v2_compaction_with_progress(&project, &id, target, &"legacy lossy handoff ".repeat(250));
    let before = reduced_session(&project, &id);
    assert!(
        before
            .latest_compaction()
            .unwrap()
            .retained_history()
            .is_empty()
    );
    let resumed = run_chat(
        root.path(),
        &config,
        "/compact\n/exit\n",
        &["--resume", &id],
    );
    let stderr = String::from_utf8_lossy(&resumed.stderr);
    assert!(
        !resumed.status.success(),
        "unsafe empty suffix accepted: {}",
        String::from_utf8_lossy(&resumed.stdout)
    );
    assert!(
        stderr.contains("smaller usable next-Turn Context"),
        "{stderr}"
    );
    assert!(!stderr.contains("runtime Context state"), "{stderr}");
    let after = reduced_session(&project, &id);
    assert_eq!(
        after.request_audits(),
        before.request_audits(),
        "no model request when the required latest batch cannot shrink Context"
    );
    assert_eq!(after.history(), before.history());
    assert_eq!(after.latest_compaction(), before.latest_compaction());
}

#[test]
fn append_only_research_refresh_keeps_v5_handoff_without_hidden_sampling() {
    let root = TemporaryDirectory::new("v5-research-refresh");
    initialize_project(root.path());
    let project = Project::discover(root.path()).unwrap();
    append_research_snapshot(&project, "before-model-handoff");
    let server = spawn_responses_server(vec![
        (
            200,
            text_response(
                &"Old assistant preparation prose. ".repeat(600),
                "before-handoff",
            ),
        ),
        (
            200,
            compaction_summary_response(
                "MODEL-HANDOFF-ACTUAL-PROGRESS: the neutral task completed and retained observations must be consulted.",
                "model-handoff",
            ),
        ),
        (
            200,
            text_response("CURRENT-NEUTRAL-ANSWER", "after-refresh"),
        ),
    ]);
    let config = write_config(root.path(), &server.base_url);
    let first = run_chat(
        root.path(),
        &config,
        "Inspect the neutral fixture.\n/compact\n/exit\n",
        &[],
    );
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    for _ in 0..2 {
        server.requests.recv_timeout(F2A_WAIT).unwrap();
    }
    let id = session_id(root.path());
    let before = session_events(root.path(), &id);
    let original = reduced_session(&project, &id);
    let old_summary = original.latest_compaction().unwrap().summary();
    assert_eq!(old_summary.schema_version(), 5);
    let original_narrative = old_summary.narrative().unwrap().clone();
    let current = append_research_snapshot(&project, "after-model-handoff");
    let resumed = run_chat(
        root.path(),
        &config,
        "Continue the neutral fixture.\n/exit\n",
        &["--resume", &id],
    );
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    let wire = request_body(&server.requests.recv_timeout(F2A_WAIT).unwrap());
    assert!(server.requests.try_recv().is_err());
    server.join();
    assert!(
        wire["input"]
            .to_string()
            .contains("MODEL-HANDOFF-ACTUAL-PROGRESS")
    );
    assert!(
        !wire["input"]
            .to_string()
            .contains("Old assistant preparation prose.")
    );
    assert!(wire["input"].to_string().contains(current.as_str()));
    let after = session_events(root.path(), &id);
    assert_eq!(&after[..before.len()], before.as_slice());
    let state = reduced_session(&project, &id);
    let refreshed = state.latest_compaction().unwrap().summary();
    assert_eq!(refreshed.revision(), 2);
    assert_eq!(refreshed.narrative(), Some(&original_narrative));
    assert_eq!(
        state.request_audits().len(),
        3,
        "Resume refresh must not sample a model"
    );
    assert_audits_reconstruct(root.path(), &state);
}

#[test]
fn task_scope_is_delivered_in_a_fresh_session_and_on_resume_with_old_project_facts() {
    let root = TemporaryDirectory::new("task-scope-project");
    let harness = TemporaryDirectory::new("task-scope-provider");
    initialize_project(root.path());
    let project = Project::discover(root.path()).unwrap();
    append_research_snapshot(&project, "old-report-target");
    let research_path = root.path().join(".echidna/research.jsonl");
    let research_before = fs::read(&research_path).unwrap();
    let memory = root.path().join(".echidna/memory/core.md");
    fs::create_dir_all(memory.parent().unwrap()).unwrap();
    fs::write(
        &memory,
        "Historical project note: old-report-target analysis is unfinished.",
    )
    .unwrap();
    let server = spawn_responses_server(vec![
        (
            200,
            text_response("Old plan: inspect src/old-report-target next.", "task-old"),
        ),
        (
            200,
            text_response(
                "Scripted greeting, not model-quality evidence.",
                "task-hello",
            ),
        ),
        (
            200,
            text_response(
                "Scripted scope reply, not model-quality evidence.",
                "task-scope",
            ),
        ),
    ]);
    let config = write_config(harness.path(), &server.base_url);
    let old = f2a_wait_child(
        spawn_f2a_chat(
            root.path(),
            &config,
            "Inspect old-report-target later.\n/exit\n",
            &[],
        ),
        "old project task",
    );
    assert!(
        old.status.success(),
        "{}",
        String::from_utf8_lossy(&old.stderr)
    );
    f2a_request_body(&server.requests, "old task");
    let old_id = session_id(root.path());
    let old_events = session_events(root.path(), &old_id);
    let fresh = f2a_wait_child(
        spawn_f2a_chat(root.path(), &config, "hello\n/exit\n", &[]),
        "new session greeting",
    );
    assert!(
        fresh.status.success(),
        "{}",
        String::from_utf8_lossy(&fresh.stderr)
    );
    let hello = f2a_request_body(&server.requests, "greeting request");
    let ids = session_ids(root.path());
    assert_eq!(ids.len(), 2);
    let id = ids.iter().find(|id| **id != old_id).unwrap();
    let before_resume = session_events(root.path(), id);
    let scope =
        "Only check the report service connection and stop; do not continue the old analysis.";
    let resumed = f2a_wait_child(
        spawn_f2a_chat(
            root.path(),
            &config,
            &format!("{scope}\n/exit\n"),
            &["--resume", id],
        ),
        "resume narrowed scope",
    );
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    let narrowed = f2a_request_body(&server.requests, "narrowed request");
    for (request, current) in [(&hello, "hello"), (&narrowed, scope)] {
        assert!(request.get("previous_response_id").is_none());
        let instructions = request["instructions"].as_str().unwrap();
        assert!(instructions.contains("Phases/history aren't tasks"));
        assert!(instructions.contains("Greet without research"));
        assert!(instructions.contains("When the requested task calls for mapping:"));
        assert!(!instructions.contains("Current objective:"));
        let input = request["input"].as_array().unwrap();
        assert!(
            input
                .iter()
                .any(|item| item["role"] == "user" && item.to_string().contains(current))
        );
        // Retaining real background is intentional; do not solve scope by deleting it.
        assert!(request.to_string().contains("snapshot-old-report-target"));
        assert!(request.to_string().contains("Historical project note"));
        assert!(!request.to_string().contains("Old plan: inspect"));
    }
    f2a_join_server(server, "task scope delivery");
    assert_eq!(session_events(root.path(), &old_id), old_events);
    let after = session_events(root.path(), id);
    assert_eq!(&after[..before_resume.len()], before_resume.as_slice());
    assert_eq!(fs::read(research_path).unwrap(), research_before);
    let state = reduced_session(&project, id);
    assert_eq!(state.request_audits().len(), 2);
    assert_audits_reconstruct(root.path(), &state);
    let users = state
        .history()
        .items()
        .iter()
        .filter_map(|item| match item {
            ContentItem::Text {
                role: echidna::domain::model::MessageRole::User,
                text,
            } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(users, ["hello", scope]);
}

#[test]
fn repeated_compaction_then_process_resume_refreshes_facts_without_reexecuting_completed_tools() {
    let root = TemporaryDirectory::new("engineering-compact-resume");
    let harness = TemporaryDirectory::new("engineering-provider");
    initialize_project(root.path());
    let project = Project::discover(root.path()).unwrap();
    fs::create_dir_all(root.path().join("src")).unwrap();
    let settings = root.path().join("src/report-settings");
    fs::write(&settings, "currency=USD\n").unwrap();
    let snapshot = append_research_snapshot(&project, "report-settings");
    let memory = root.path().join(".echidna/memory/core.md");
    fs::create_dir_all(memory.parent().unwrap()).unwrap();
    fs::write(&memory, "report.currency=USD\n").unwrap();
    // Bounded old prose makes production compaction measurably useful. These
    // are scripted observations, never claims of real-model performance.
    let old_prose = "Earlier report currency was USD. ".repeat(256);
    let obsolete_prose_then_call = |response: String, prose: &str| {
        format!(
            "data: {}\n\n{response}",
            json!({"type":"response.output_text.delta", "delta":prose})
        )
    };
    let server = spawn_responses_server(vec![
        (
            200,
            obsolete_prose_then_call(
                m4_long_flow_tool_response(
                    "engineering-once",
                    "run_command",
                    json!({"argv":["/bin/sh", "-c", "printf 'executed\\n' >> once.txt"]}),
                    "engineering-command",
                ),
                &old_prose,
            ),
        ),
        (
            200,
            m4_long_flow_tool_response(
                "engineering-old-read",
                "read_file",
                json!({"path":"src/report-settings"}),
                "engineering-read",
            ),
        ),
        (
            200,
            text_response(
                "Scripted USD observation complete.",
                "engineering-old-answer",
            ),
        ),
        (
            200,
            compaction_summary_response(
                "engineering-once completed the once-only marker; engineering-old-read observed USD. The completed mutation must not repeat.",
                "engineering-summary-1",
            ),
        ),
        (
            200,
            obsolete_prose_then_call(
                m4_long_flow_tool_response(
                    "engineering-new-read",
                    "read_file",
                    json!({"path":"src/report-settings"}),
                    "engineering-fresh-read",
                ),
                &"Earlier currency was USD; consult the current read. ".repeat(256),
            ),
        ),
        (
            200,
            text_response(
                "Scripted current currency: EUR; earlier USD was observed before the operator update.",
                "engineering-current-answer",
            ),
        ),
        (
            200,
            compaction_summary_response(
                "engineering-new-read observed EUR after the operator update; engineering-once previously completed and must not repeat.",
                "engineering-summary-2",
            ),
        ),
        (
            200,
            m4_long_flow_tool_response(
                "engineering-latest-read",
                "read_file",
                json!({"path":"src/report-settings"}),
                "engineering-latest-result",
            ),
        ),
        (
            200,
            text_response(
                "scripted current currency: JPY",
                "engineering-latest-answer",
            ),
        ),
    ]);
    let config = write_config(harness.path(), &server.base_url);
    // Explicit rules are not dependent on historical README reads or Summary.
    let rules = "Only use the configured report/test commands. Never rerun the once-only marker.";
    fs::write(harness.path().join("rules.md"), rules).unwrap();
    // Full-only delivery exercises the wire history used by strict backends.
    let configured = fs::read_to_string(&config).unwrap();
    fs::write(
        &config,
        format!(
            "{}\n[prompts]\nproject_file = \"rules.md\"\n",
            configured.replace("incremental_input = true", "incremental_input = false")
        ),
    )
    .unwrap();
    let first = f2a_wait_child(
        spawn_f2a_chat(
            root.path(),
            &config,
            "Record the once-only marker and read report settings.\n/exit\n",
            &[],
        ),
        "engineering initial",
    );
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let initial_wires: Vec<_> = (0..3)
        .map(|_| f2a_request_body(&server.requests, "engineering initial wire"))
        .collect();
    assert!(
        initial_wires[2]["input"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["type"] == "function_call_output"
                && item["call_id"] == "engineering-old-read"
                && item["output"].as_str().unwrap().contains("currency=USD"))
    );
    let id = session_id(root.path());
    let before = reduced_session(&project, &id);
    assert_eq!(before.execution_receipt_count(), 2);
    assert_eq!(
        fs::read_to_string(root.path().join("once.txt")).unwrap(),
        "executed\n"
    );
    assert_audits_reconstruct(root.path(), &before);

    for (cycle, currency, old_currency, call_id) in [
        (1, "EUR", "USD", "engineering-new-read"),
        (2, "JPY", "EUR", "engineering-latest-read"),
    ] {
        let before_events = session_events(root.path(), &id);
        let before = reduced_session(&project, &id);
        let compact = f2a_wait_child(
            spawn_f2a_chat(
                root.path(),
                &config,
                "/compact\n/exit\n",
                &["--resume", &id],
            ),
            "engineering explicit compaction",
        );
        assert!(
            compact.status.success(),
            "{}",
            String::from_utf8_lossy(&compact.stderr)
        );
        assert!(
            String::from_utf8_lossy(&compact.stdout)
                .contains(&format!("Compaction revision {cycle} applied"))
        );
        let summary_wire = f2a_request_body(&server.requests, "tool-free compaction handoff");
        assert!(summary_wire["tools"].as_array().is_none_or(Vec::is_empty));
        assert!(summary_wire.get("previous_response_id").is_none());
        assert!(
            summary_wire["input"]
                .to_string()
                .contains("engineering-once")
        );
        assert!(server.requests.try_recv().is_err());
        let compact_events = session_events(root.path(), &id);
        assert_eq!(
            &compact_events[..before_events.len()],
            before_events.as_slice()
        );
        let compact_state = reduced_session(&project, &id);
        let compaction = compact_state.latest_compaction().unwrap();
        assert!(compaction.retained_history().len() < compaction.source_history_item_count());
        assert_eq!(compact_state.history().items(), before.history().items());
        let summary = serde_json::to_value(compaction.summary()).unwrap();
        assert!(
            summary["narrative"]["next_step_notes"]
                .to_string()
                .contains("latest assistant continuation is not the original baseline")
        );
        assert_eq!(
            summary["evidence_spine"]["target_snapshot_refs"],
            json!([snapshot.as_str()])
        );
        assert_eq!(
            compact_state.request_audits().len(),
            before.request_audits().len() + 1
        );
        assert_eq!(
            &compact_state.request_audits()[..before.request_audits().len()],
            before.request_audits()
        );

        // Operator-boundary changes after exit, not an unsupported mid-turn
        // replacement underneath completed reasoning. Same-size current values.
        fs::write(&settings, format!("currency={currency}\n")).unwrap();
        fs::write(&memory, format!("report.currency={currency}\n")).unwrap();
        let question =
            "Read current settings; report only current currency. Do not rerun the marker.";
        let resumed = f2a_wait_child(
            spawn_f2a_chat(
                root.path(),
                &config,
                &format!("{question}\n/exit\n"),
                &["--resume", &id],
            ),
            "engineering compacted resume",
        );
        assert!(
            resumed.status.success(),
            "{}",
            String::from_utf8_lossy(&resumed.stderr)
        );
        let resumed_wire = f2a_request_body(&server.requests, "engineering resumed wire");
        let result_wire = f2a_request_body(&server.requests, "engineering fresh result");
        assert!(server.requests.try_recv().is_err());
        for wire in [&resumed_wire, &result_wire] {
            assert!(wire["instructions"].as_str().unwrap().contains(rules));
            assert!(wire.get("previous_response_id").is_none());
            let input = wire["input"].as_array().unwrap();
            assert!(input.iter().any(|item| {
                item["role"] == "user"
                    && item
                        .to_string()
                        .contains(&format!("report.currency={currency}"))
            }));
            assert!(!input.iter().any(|item| {
                item["role"] == "user"
                    && item
                        .to_string()
                        .contains(&format!("report.currency={old_currency}"))
            }));
            assert!(
                wire["input"]
                    .to_string()
                    .contains("derived_session_summary_v5")
            );
            assert!(wire["input"].to_string().contains(snapshot.as_str()));
            assert!(
                input
                    .iter()
                    .any(|item| item["role"] == "user" && item.to_string().contains(question))
            );
        }
        assert!(result_wire["input"].as_array().unwrap().iter().any(|item| {
            item["type"] == "function_call_output"
                && item["call_id"] == call_id
                && item["output"]
                    .as_str()
                    .unwrap()
                    .contains(&format!("currency={currency}"))
        }));
        let after_events = session_events(root.path(), &id);
        assert_eq!(
            &after_events[..compact_events.len()],
            compact_events.as_slice()
        );
        assert_eq!(
            after_events
                .iter()
                .filter(|event| event["event_type"] == "session.compaction_created")
                .count(),
            cycle
        );
        let after = reduced_session(&project, &id);
        assert_eq!(
            &after.history().items()[..before.history().items().len()],
            before.history().items()
        );
        assert_eq!(after.history().ensure_all_tool_calls_paired(), Ok(()));
        assert_eq!(
            after.execution_receipt_count(),
            u64::try_from(2 + cycle).unwrap()
        );
        assert_eq!(after.request_audits().len(), 3 + 3 * cycle);
        assert_eq!(
            &after.request_audits()[..before.request_audits().len()],
            before.request_audits()
        );
        assert_audits_reconstruct(root.path(), &after);
        assert_eq!(
            fs::read_to_string(root.path().join("once.txt")).unwrap(),
            "executed\n"
        );
        let calls: Vec<_> = after
            .history()
            .items()
            .iter()
            .filter_map(|item| match item {
                ContentItem::ToolCall { call } => Some((call.call_id.as_str(), call.name.as_str())),
                _ => None,
            })
            .collect();
        assert_eq!(
            calls,
            &[
                ("engineering-once", "run_command"),
                ("engineering-old-read", "read_file"),
                ("engineering-new-read", "read_file"),
                ("engineering-latest-read", "read_file"),
            ][..2 + cycle]
        );
    }
    f2a_join_server(server, "engineering completion");
}

#[test]
fn resume_delivers_current_memory_alongside_old_prose_without_rewriting_history() {
    let root = TemporaryDirectory::new("current-fact-guidance");
    let harness = TemporaryDirectory::new("current-fact-provider");
    initialize_project(root.path());
    let memory = root.path().join(".echidna/memory/core.md");
    fs::create_dir_all(memory.parent().unwrap()).unwrap();
    fs::write(&memory, "report.currency=USD\n").unwrap();
    let server = spawn_responses_server(vec![
        (
            200,
            text_response(
                "Earlier report currency was USD; current value uncertain.",
                "fact-old",
            ),
        ),
        (
            200,
            text_response("scripted reply, not model-quality evidence", "fact-current"),
        ),
    ]);
    let config = write_config(harness.path(), &server.base_url);
    let first = f2a_wait_child(
        spawn_f2a_chat(
            root.path(),
            &config,
            "What is the report currency?\n/exit\n",
            &[],
        ),
        "fact initial",
    );
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let initial = f2a_request_body(&server.requests, "fact initial request");
    assert!(initial["input"].to_string().contains("report.currency=USD"));
    let id = session_id(root.path());
    let before = session_events(root.path(), &id);
    let project = Project::discover(root.path()).unwrap();
    let before_state = reduced_session(&project, &id);
    assert_audits_reconstruct(root.path(), &before_state);

    fs::write(&memory, "report.currency=EUR\n").unwrap();
    let question = "Give only the current report currency in one line.";
    let resumed = f2a_wait_child(
        spawn_f2a_chat(
            root.path(),
            &config,
            &format!("{question}\n/exit\n"),
            &["--resume", &id],
        ),
        "fact resume",
    );
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    let request = f2a_request_body(&server.requests, "fact resumed request");
    assert!(request.get("previous_response_id").is_none());
    assert!(
        request["instructions"]
            .as_str()
            .unwrap()
            .contains(echidna::domain::research_control::BaseContract.prompt())
    );
    let input = request["input"].as_array().unwrap();
    assert!(
        input
            .iter()
            .any(|item| item["role"] == "user" && item.to_string().contains("report.currency=EUR"))
    );
    assert!(
        !input
            .iter()
            .any(|item| item["role"] == "user" && item.to_string().contains("report.currency=USD"))
    );
    assert!(input.iter().any(|item| item["role"] == "assistant"
        && item.to_string().contains("Earlier report currency was USD")));
    assert!(
        input
            .iter()
            .any(|item| item["role"] == "user" && item.to_string().contains(question))
    );
    f2a_join_server(server, "fact guidance");
    let after = session_events(root.path(), &id);
    assert_eq!(&after[..before.len()], before.as_slice());
    let after_state = reduced_session(&project, &id);
    assert_eq!(after_state.request_audits().len(), 2);
    assert_eq!(
        &after_state.request_audits()[..1],
        before_state.request_audits()
    );
    for audit in after_state.request_audits() {
        let binding = after_state.agent_definition_binding().unwrap();
        let definition = binding.definition();
        assert!(serde_json::to_string(audit).unwrap().contains(&format!(
            "prompt:agent:01:{}:persona:{}",
            definition.id().as_str(),
            definition.instructions().persona().digest().as_str()
        )));
    }
    assert_audits_reconstruct(root.path(), &after_state);
}
