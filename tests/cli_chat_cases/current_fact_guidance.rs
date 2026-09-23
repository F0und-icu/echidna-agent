//! Scripted responses prove delivery, not a model's adherence to guidance.
use super::*;
use cyber_agent::domain::model::ContentItem;

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
    let memory = root.path().join(".cyber-agent/memory/core.md");
    fs::create_dir_all(memory.parent().unwrap()).unwrap();
    fs::write(&memory, "report.currency=USD\n").unwrap();
    // Bounded old prose makes production compaction measurably useful. These
    // are scripted observations, never claims of real-model performance.
    let old_prose = "Earlier report currency was USD. ".repeat(256);
    let server = spawn_responses_server(vec![
        (
            200,
            m4_long_flow_tool_response(
                "engineering-once",
                "run_command",
                json!({"argv":["/bin/sh", "-c", "printf 'executed\\n' >> once.txt"]}),
                "engineering-command",
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
        (200, text_response(&old_prose, "engineering-old-answer")),
        (
            200,
            m4_long_flow_tool_response(
                "engineering-new-read",
                "read_file",
                json!({"path":"src/report-settings"}),
                "engineering-fresh-read",
            ),
        ),
        (
            200,
            text_response(
                &"Scripted current currency: EUR; earlier USD was observed before the operator update. ".repeat(256),
                "engineering-current-answer",
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
        (200, text_response("scripted current currency: JPY", "engineering-latest-answer")),
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
        assert!(
            server.requests.try_recv().is_err(),
            "compaction must not sample a model"
        );
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
        assert_eq!(compact_state.request_audits(), before.request_audits());

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
                    .contains("derived_session_summary_v2")
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
        assert_eq!(after.request_audits().len(), 3 + 2 * cycle);
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
    let memory = root.path().join(".cyber-agent/memory/core.md");
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
            .contains(cyber_agent::domain::research_control::BaseContract.prompt())
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
        assert!(
            serde_json::to_string(audit)
                .unwrap()
                .contains("prompt:base:pragmatic-vulnerability-researcher:4")
        );
    }
    assert_audits_reconstruct(root.path(), &after_state);
}
