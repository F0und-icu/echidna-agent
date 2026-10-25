//! Completed-Turn auto-compaction uses the generic pinned Agent and live catalog.

use super::*;

fn config(root: &std::path::Path, url: &str, enabled: Option<bool>) -> std::path::PathBuf {
    let path = write_config(root, url);
    if let Some(enabled) = enabled {
        fs::write(
            &path,
            format!(
                "{}\n[runtime]\nauto_compact = {enabled}\n",
                fs::read_to_string(&path).unwrap()
            ),
        )
        .unwrap();
    }
    path
}

fn definition(
    root: &std::path::Path,
    passive_rsi: bool,
    remote: Option<&str>,
) -> std::path::PathBuf {
    let path = root.join("order-clerk");
    fs::create_dir(&path).unwrap();
    fs::write(
        path.join("persona.md"),
        "PINNED-ORDER-CLERK. Execute the client's stated requirements.",
    )
    .unwrap();
    fs::write(
        path.join("work.md"),
        "Respect the exact target limit price and current evidence.",
    )
    .unwrap();
    let mut manifest = r#"schema_version = 1
id = "order-clerk"
name = "Order clerk fixture"
[instructions]
persona_file = "persona.md"
work_file = "work.md"
method_files = []
[context]
personal = "none"
project = "none"
memory = "agent"
[tools]
memory_write = "direct"
run_command = "direct"
start_job = "direct"
"#
    .to_owned();
    if let Some(remote) = remote {
        manifest.push_str(&format!("{remote} = \"direct\"\n"));
    }
    if passive_rsi {
        manifest.push_str("\n[rsi]\npersona = true\nmethods = true\n");
    }
    fs::write(path.join("agent.toml"), manifest).unwrap();
    path
}

fn compactions(root: &std::path::Path) -> Vec<Value> {
    session_events(root, &session_id(root))
        .into_iter()
        .filter(|event| event["event_type"] == "session.compaction_created")
        .collect()
}

fn reasoning_answer(text: &str) -> String {
    [
        json!({"type":"response.output_item.done","output_index":0,"item":{
            "type":"reasoning","content":[{"type":"reasoning_text","text":"volatile-only-reasoning"}]
        }}),
        json!({"type":"response.output_text.delta","output_index":1,"content_index":0,"delta":text}),
        json!({"type":"response.completed","response":{"id":"auto-first"}}),
    ].iter().map(|event|format!("data: {event}\n\n")).collect()
}

#[test]
fn generic_auto_compaction_keeps_scoped_memory_persona_rsi_and_one_mcp_catalog() {
    for rsi in [false, true] {
        let root = TemporaryDirectory::new("auto-generic-clerk");
        initialize_project(root.path());
        let remote_name =
            "mcp__fixture__".to_owned() + &blake3::hash(b"fixture.quote").to_hex()[..20];
        let definition = definition(root.path(), rsi, Some(&remote_name));
        let mcp = spawn_responses_server(vec![(
            200,
            format!(
                "data: {}\n\n",
                json!({
                    "jsonrpc":"2.0","id":"1","result":{"tools":[{
                        "name":"fixture.quote","description":"Fixture quote",
                        "inputSchema":{"type":"object","properties":{},"additionalProperties":false}
                    }]}
                })
            ),
        )]);
        let server = spawn_responses_server(vec![
            (
                200,
                m4_long_flow_tool_response(
                    "scoped-core",
                    "memory_write",
                    json!({
                        "path":"core.md", "content":"CLERK-AGENT-SCOPED-MEMORY", "expected_hash":null,
                        "event_id":"clerk-memory-created"
                    }),
                    "auto-memory",
                ),
            ),
            (
                200,
                reasoning_answer(&"completed clerk notes ".repeat(10_000)),
            ),
            (200, text_response("NEXT-TASK-COMPLETE", "auto-next")),
        ]);
        let config = config(root.path(), &server.base_url, Some(true));
        fs::write(
            &config,
            format!(
                "{}\n[mcp_servers.fixture]\nurl = {}\nprotocol_version = \"2026-07-28\"\n",
                fs::read_to_string(&config).unwrap(),
                serde_json::to_string(&mcp.base_url).unwrap()
            ),
        )
        .unwrap();
        let output = run_chat(
            root.path(),
            &config,
            "first task\nsecond task\n/exit\n",
            &["--agent", definition.to_str().unwrap()],
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let requests: Vec<_> = (0..3)
            .map(|_| request_body(&server.requests.recv_timeout(F2A_WAIT).unwrap()))
            .collect();
        assert!(server.requests.try_recv().is_err());
        server.join();
        let catalog = request_body(&mcp.requests.recv_timeout(F2A_WAIT).unwrap());
        assert_eq!(catalog["method"], "tools/list");
        assert!(
            mcp.requests.try_recv().is_err(),
            "no catalog rediscovery after reset"
        );
        mcp.join();
        for request in &requests {
            assert!(
                request["instructions"]
                    .as_str()
                    .unwrap()
                    .contains("PINNED-ORDER-CLERK")
            );
            assert!(
                request["tools"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|tool| tool["name"] == remote_name)
            );
        }
        assert!(json_contains_text(
            &requests[2]["input"],
            "CLERK-AGENT-SCOPED-MEMORY"
        ));
        assert!(json_contains_text(&requests[2]["input"], "second task"));
        assert!(requests[2]["previous_response_id"].is_null());
        assert!(!json_contains_type(&requests[2]["input"], "reasoning"));
        assert!(!requests[2].to_string().contains("volatile-only-reasoning"));
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_eq!(
            stdout
                .matches("Automatic completed-Turn compaction applied")
                .count(),
            1,
            "{stdout}"
        );
        assert!(stdout.contains("NEXT-TASK-COMPLETE"));
        let events = session_events(root.path(), &session_id(root.path()));
        assert_eq!(
            events
                .iter()
                .filter(
                    |event| event["event_type"] == "session.history_item_recorded"
                        && event["payload"]["item"]["text"] == "second task"
                )
                .count(),
            1
        );
        let project = Project::discover(root.path()).unwrap();
        let state = reduced_session(&project, &session_id(root.path()));
        assert_eq!(state.request_audits().len(), 3);
        assert_eq!(
            state.request_audits()[2].delivery().reason(),
            echidna::domain::request_audit::RequestAuditDeliveryReason::Full(
                echidna::domain::context::BaselineInvalidationReason::CompactionReset
            )
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| event["event_type"] == "tool.execution_outcome_recorded")
                .count(),
            1,
            "only the actual local memory operation ran"
        );
        let summary = serde_json::to_value(state.latest_compaction().unwrap().summary()).unwrap();
        assert!(
            summary["evidence_spine"].is_null(),
            "generic task needs no fake research target"
        );
        assert_eq!(
            summary["task_references"]["definition"]["agent_id"],
            "order-clerk"
        );
        assert_eq!(
            serde_json::to_value(state.agent_definition_binding().unwrap()).unwrap()["definition"]
                ["rsi"]
                .is_object(),
            rsi
        );
        assert_eq!(compactions(root.path()).len(), 1);
        let mut writer = project.open_writer().unwrap();
        let store = writer.open_research(None).unwrap();
        assert!(
            EventSource::read_events(&store, &ProjectEventStream::Research.stream_id())
                .unwrap()
                .iter()
                .all(|event| !event.event_type.as_str().contains("target_snapshot"))
        );
    }
}

#[test]
fn automatic_compaction_is_opt_in_and_low_pressure_retains_continuation() {
    for (enabled, large) in [(None, true), (Some(false), true), (Some(true), false)] {
        let root = TemporaryDirectory::new("auto-opt-in");
        initialize_project(root.path());
        let definition = definition(root.path(), false, None);
        let mut responses = vec![(
            200,
            reasoning_answer(&"notes ".repeat(if large { 35_000 } else { 1 })),
        )];
        if !large {
            responses.push((200, text_response("NEXT-LOW-PRESSURE", "low-next")));
        }
        let server = spawn_responses_server(responses);
        let config = config(root.path(), &server.base_url, enabled);
        let output = run_chat(
            root.path(),
            &config,
            if large {
                "one task\n/exit\n"
            } else {
                "one task\nnext task\n/exit\n"
            },
            &["--agent", definition.to_str().unwrap()],
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        server.requests.recv_timeout(F2A_WAIT).unwrap();
        if !large {
            let next = request_body(&server.requests.recv_timeout(F2A_WAIT).unwrap());
            assert_eq!(
                next["previous_response_id"], "auto-first",
                "low pressure retains the successful continuation"
            );
        }
        assert!(server.requests.try_recv().is_err());
        server.join();
        assert!(
            !String::from_utf8_lossy(&output.stdout)
                .contains("Automatic completed-Turn compaction")
        );
        assert!(compactions(root.path()).is_empty());
    }
}

#[test]
fn long_tool_loop_compacts_settled_batches_without_replaying_or_losing_task() {
    let mut measured = Vec::new();
    for (enabled, incremental) in [(false, false), (true, false), (true, true)] {
        let root = TemporaryDirectory::new("auto-tool-working-set");
        initialize_project(root.path());
        let definition = definition(root.path(), false, None);
        fs::write(root.path().join("task.png"), png_bytes(9, 7)).unwrap();
        let mut responses = Vec::new();
        if incremental {
            let manifest = definition.join("agent.toml");
            fs::write(
                &manifest,
                fs::read_to_string(&manifest).unwrap().replace(
                    "run_command = \"direct\"",
                    "run_command = \"deferred\"\ntool_search = \"direct\"",
                ),
            )
            .unwrap();
            responses.push((
                200,
                m4_long_flow_tool_response(
                    "activate-command",
                    "tool_search",
                    json!({"query":"run_command"}),
                    "activate-response",
                ),
            ));
        }
        for index in 0..3 {
            let mut response = format!(
                "data: {}\n\n",
                json!({
                "type":"response.output_item.done", "output_index":0,
                "item":{"type":"reasoning", "content":[{"type":"reasoning_text", "text":format!("batch-{index}-reasoning")}]} })
            );
            response.push_str(&m4_long_flow_tool_response(
                &format!("batch-{index}"),
                "run_command",
                json!({"argv":["/usr/bin/printf", "%s", "neutral-output ".repeat(7_000)]}),
                &format!("batch-response-{index}"),
            ));
            responses.push((200, response));
        }
        responses.push((200, text_response("WORKING-SET-DONE", "batch-final")));
        let server = spawn_responses_server(responses);
        let config = config(root.path(), &server.base_url, Some(enabled));
        fs::write(
            &config,
            fs::read_to_string(&config)
                .unwrap()
                .replace("context_window = 32768", "context_window = 1048576")
                .replacen(
                    "vision_mode = \"secondary_tool\"",
                    "vision_mode = \"primary_direct\"",
                    1,
                )
                .replacen("image_input = false", "image_input = true", 1)
                .replace(
                    "incremental_input = true",
                    &format!("incremental_input = {incremental}"),
                ),
        )
        .unwrap();
        let task = "ORIGINAL-ACTIVE-TASK: retain this constraint through every batch";
        let output = run_chat(
            root.path(),
            &config,
            &format!("/image task.png\n{task}\n/exit\n"),
            &["--agent", definition.to_str().unwrap()],
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let requests: Vec<_> = (0..4 + usize::from(incremental))
            .map(|_| request_body(&server.requests.recv_timeout(F2A_WAIT).unwrap()))
            .collect();
        assert!(server.requests.try_recv().is_err());
        server.join();
        assert!(String::from_utf8_lossy(&output.stdout).contains("WORKING-SET-DONE"));
        for request in &requests {
            if !request["previous_response_id"].is_null() {
                continue;
            }
            assert!(json_contains_text(&request["input"], task));
            let task_count = request["input"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|item| {
                    item["role"] == "user"
                        && item["content"]
                            .as_array()
                            .is_some_and(|parts| parts.iter().any(|part| part["text"] == task))
                })
                .count();
            assert_eq!(
                task_count, 1,
                "original input is present exactly once per request"
            );
            assert_eq!(
                request["input"]
                    .to_string()
                    .matches("\"type\":\"input_image\"")
                    .count(),
                1,
                "original image remains once across reset"
            );
        }
        let project = Project::discover(root.path()).unwrap();
        let state = reduced_session(&project, &session_id(root.path()));
        let events = session_events(root.path(), &session_id(root.path()));
        assert_eq!(
            events
                .iter()
                .filter(|event| event["event_type"] == "tool.execution_outcome_recorded")
                .count(),
            3 + usize::from(incremental)
        );
        assert_eq!(
            events
                .iter()
                .filter(
                    |event| event["event_type"] == "session.history_item_recorded"
                        && event["payload"]["item"]["text"] == task
                )
                .count(),
            1,
            "no fabricated repeated user History"
        );
        assert_eq!(state.request_audits().len(), 4 + usize::from(incremental));
        let resets: Vec<_> = state
            .request_audits()
            .iter()
            .enumerate()
            .filter(|(_, audit)| {
                audit.delivery().reason()
                    == echidna::domain::request_audit::RequestAuditDeliveryReason::Full(
                        echidna::domain::context::BaselineInvalidationReason::CompactionReset,
                    )
            })
            .map(|(index, _)| index)
            .collect();
        if enabled {
            assert!(
                !resets.is_empty(),
                "tool-batch pressure must trigger before completion"
            );
            assert!(resets.iter().all(|index| *index > 0));
            for index in resets {
                assert!(!json_contains_type(&requests[index]["input"], "reasoning"));
                assert!(requests[index].to_string().len() < requests[index - 1].to_string().len());
                assert!(
                    requests[index]["tools"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|tool| tool["name"] == "run_command"),
                    "deferred activation remains in the same Turn after reset"
                );
            }
        } else {
            assert!(compactions(root.path()).is_empty());
        }
        if !incremental {
            measured.push(
                requests
                    .iter()
                    .map(|request| request.to_string().len())
                    .sum::<usize>(),
            );
        }
    }
    assert!(
        measured[1] * 100 < measured[0] * 65,
        "opted-in working set must measurably reduce Full payload bytes: {measured:?}"
    );
    eprintln!(
        "Full request bytes, baseline={} bounded={}",
        measured[0], measured[1]
    );
}

#[test]
fn tool_batch_compaction_defers_when_original_task_alone_exceeds_working_target() {
    let root = TemporaryDirectory::new("auto-large-active-task");
    initialize_project(root.path());
    let definition = definition(root.path(), false, None);
    let server = spawn_responses_server(vec![
        (
            200,
            m4_long_flow_tool_response(
                "once-large-task",
                "run_command",
                json!({"argv":["/usr/bin/printf", "%s", "neutral-output ".repeat(7_000)]}),
                "large-task-tool",
            ),
        ),
        (200, text_response("LARGE-TASK-DONE", "large-task-final")),
    ]);
    let config = config(root.path(), &server.base_url, Some(true));
    fs::write(
        &config,
        fs::read_to_string(&config)
            .unwrap()
            .replace("context_window = 32768", "context_window = 1048576"),
    )
    .unwrap();
    let task = format!(
        "Keep this entire operator input: {}",
        "constraint ".repeat(20_000)
    );
    let output = run_chat(
        root.path(),
        &config,
        &format!("{task}\n/exit\n"),
        &["--agent", definition.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    server.requests.recv_timeout(F2A_WAIT).unwrap();
    let second = request_body(&server.requests.recv_timeout(F2A_WAIT).unwrap());
    assert_eq!(second["previous_response_id"], "large-task-tool");
    server.join();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("no smaller usable Context working set"),
        "{stdout}"
    );
    assert!(stdout.contains("LARGE-TASK-DONE"));
    assert!(!stdout.contains("Automatic tool-batch compaction applied"));
}

#[test]
fn completed_reasoning_delivery_alone_can_trigger_opted_in_compaction() {
    let root = TemporaryDirectory::new("auto-reasoning-delivery");
    initialize_project(root.path());
    let definition = definition(root.path(), false, None);
    let first = format!(
        "data: {}\n\n{}",
        json!({"type":"response.output_item.done", "item":{"type":"reasoning", "content":[{"type":"reasoning_text", "text":"retained-reasoning ".repeat(18_000)}]}}),
        text_response("SHORT-FINAL", "large-reasoning")
    );
    let server = spawn_responses_server(vec![
        (200, first),
        (200, text_response("NEXT-COMPLETE", "next-reasoning")),
    ]);
    let config = config(root.path(), &server.base_url, Some(true));
    fs::write(
        &config,
        fs::read_to_string(&config)
            .unwrap()
            .replace("context_window = 32768", "context_window = 1048576"),
    )
    .unwrap();
    let output = run_chat(
        root.path(),
        &config,
        "first task\nnext task\n/exit\n",
        &["--agent", definition.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    server.requests.recv_timeout(F2A_WAIT).unwrap();
    let next = request_body(&server.requests.recv_timeout(F2A_WAIT).unwrap());
    server.join();
    assert!(next["previous_response_id"].is_null());
    assert!(!json_contains_type(&next["input"], "reasoning"));
    assert!(json_contains_text(&next["input"], "next task"));
    assert_eq!(compactions(root.path()).len(), 1);
}

#[test]
fn tool_batch_does_not_compact_when_initial_vision_consumed_the_final_admission() {
    let root = TemporaryDirectory::new("auto-shared-budget-boundary");
    initialize_project(root.path());
    let definition = definition(root.path(), false, None);
    fs::write(root.path().join("screen.png"), png_bytes(9, 7)).unwrap();
    let vision = spawn_responses_server(vec![(
        200,
        vision_response("neutral fixture", "budget-vision"),
    )]);
    let response = format!(
        "data: {}\n\n{}",
        json!({"type":"response.output_item.done", "item":{"type":"reasoning", "content":[{"type":"reasoning_text", "text":"preserve-budget-stop-reasoning"}]}}),
        m4_long_flow_tool_response(
            "budget-real-tool",
            "run_command",
            json!({"argv":["/usr/bin/printf", "%s", "neutral-output ".repeat(10_000)]}),
            "budget-primary"
        )
    );
    let primary = spawn_responses_server(vec![(200, response)]);
    let config = write_dual_config(root.path(), &primary.base_url, &vision.base_url);
    fs::write(
        &config,
        format!(
            "{}\n[runtime]\nauto_compact = true\nmodel_request_limit_per_turn = 2\n",
            fs::read_to_string(&config)
                .unwrap()
                .replace("context_window = 32768", "context_window = 1048576")
        ),
    )
    .unwrap();
    let output = run_chat(
        root.path(),
        &config,
        "/image screen.png\nComplete this task under the shared budget.\n/exit\n",
        &["--agent", definition.to_str().unwrap()],
    );
    primary.requests.recv_timeout(F2A_WAIT).unwrap();
    assert!(primary.requests.try_recv().is_err());
    primary.join();
    vision.requests.recv_timeout(F2A_WAIT).unwrap();
    assert!(vision.requests.try_recv().is_err());
    vision.join();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("per-Turn model-request limit of 2"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let project = Project::discover(root.path()).unwrap();
    let state = reduced_session(&project, &session_id(root.path()));
    assert_eq!(state.request_audits().len(), 2);
    assert_eq!(state.vision_observations().len(), 1);
    assert_eq!(
        session_events(root.path(), &session_id(root.path()))
            .iter()
            .filter(|event| event["event_type"] == "tool.execution_outcome_recorded")
            .count(),
        1
    );
    assert!(state.history().items().iter().any(|item| matches!(item, echidna::domain::model::ContentItem::ToolResult { result } if result.call_id.as_str() == "budget-real-tool")));
    assert!(
        compactions(root.path()).is_empty(),
        "a spent shared budget cannot authorize a compaction/reset: {stdout}"
    );
}

#[test]
fn automatic_compaction_runs_after_completed_rsi_diagnosis_without_extra_requests() {
    let root = TemporaryDirectory::new("auto-rsi-completed");
    initialize_project(root.path());
    let definition = definition(root.path(), true, None);
    let manifest = definition.join("agent.toml");
    fs::write(
        &manifest,
        fs::read_to_string(&manifest)
            .unwrap()
            .replace("[rsi]", "[rsi]\nimprovement = true"),
    )
    .unwrap();
    let server = spawn_responses_server(vec![
        (
            200,
            text_response(&"summary ".repeat(9_000), "auto-before-diagnosis"),
        ),
        (
            200,
            text_response(
                r#"{"decision":"no_change","reason":"No justified reusable improvement."}"#,
                "auto-diagnosis-completed",
            ),
        ),
        (200, text_response("NEXT-AFTER-RSI", "auto-after-rsi")),
    ]);
    let config = config(root.path(), &server.base_url, Some(true));
    let output = run_chat(
        root.path(),
        &config,
        "[rsi-feedback] improve this workflow\nnew task\n/exit\n",
        &["--agent", definition.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let _primary = server.requests.recv_timeout(F2A_WAIT).unwrap();
    let diagnosis = request_body(&server.requests.recv_timeout(F2A_WAIT).unwrap());
    assert!(diagnosis.to_string().contains("RSI2 diagnosis"));
    assert!(diagnosis.get("tools").is_none());
    let next = request_body(&server.requests.recv_timeout(F2A_WAIT).unwrap());
    assert!(next["previous_response_id"].is_null());
    assert!(!next.to_string().contains("auto-diagnosis-completed"));
    assert!(server.requests.try_recv().is_err());
    server.join();
    let events = session_events(root.path(), &session_id(root.path()));
    let compacted = events
        .iter()
        .position(|event| event["event_type"] == "session.compaction_created")
        .unwrap();
    let last_rsi = events
        .iter()
        .rposition(|event| event["event_type"] == "session.rsi_activity_recorded")
        .unwrap();
    assert!(
        last_rsi < compacted,
        "RSI scope must finish before durable compaction"
    );
    assert_eq!(compactions(root.path()).len(), 1);
    let project = Project::discover(root.path()).unwrap();
    let state = reduced_session(&project, &session_id(root.path()));
    assert_eq!(
        state.request_audits().len(),
        3,
        "two ordinary requests and one existing RSI diagnostic only"
    );
    assert_eq!(
        state.request_audits()[2].delivery().reason(),
        echidna::domain::request_audit::RequestAuditDeliveryReason::Full(
            echidna::domain::context::BaselineInvalidationReason::CompactionReset
        )
    );
}

#[test]
fn automatic_compaction_preserves_explicit_recovery_for_failed_and_budget_stopped_turns() {
    for budget_stop in [false, true] {
        let root = TemporaryDirectory::new("auto-failed-turn");
        initialize_project(root.path());
        let definition = definition(root.path(), false, None);
        let response = if budget_stop {
            m4_long_flow_tool_response(
                "real-once",
                "run_command",
                json!({"argv":["/usr/bin/printf","once"]}),
                "auto-stop",
            )
        } else {
            format!(
                "data: {}\n\n",
                json!({"type":"response.failed","response":{"error":{"code":"fixture","message":"auto-fixture-failure"}}})
            )
        };
        let server = spawn_responses_server(vec![(200, response)]);
        let config = config(root.path(), &server.base_url, Some(true));
        fs::write(
            &config,
            format!(
                "{}\nmodel_request_limit_per_turn = 1\n",
                fs::read_to_string(&config).unwrap()
            ),
        )
        .unwrap();
        let output = run_chat(
            root.path(),
            &config,
            "task\nblocked input\n/exit\n",
            &["--agent", definition.to_str().unwrap()],
        );
        assert!(!output.status.success());
        server.requests.recv_timeout(F2A_WAIT).unwrap();
        assert!(server.requests.try_recv().is_err());
        server.join();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("use /resume first"), "{stdout}");
        assert!(!stdout.contains("Automatic completed-Turn compaction"));
        assert!(compactions(root.path()).is_empty());
        let project = Project::discover(root.path()).unwrap();
        let state = reduced_session(&project, &session_id(root.path()));
        assert_eq!(
            session_events(root.path(), &session_id(root.path()))
                .iter()
                .filter(|event| event["event_type"] == "tool.execution_outcome_recorded")
                .count(),
            usize::from(budget_stop)
        );
        assert!(!state.history().items().iter().any(|item|matches!(item,echidna::domain::model::ContentItem::Text{text,..} if text=="blocked input")));
    }
}

#[test]
fn automatic_compaction_skips_cancelled_turn() {
    let root = TemporaryDirectory::new("auto-cancelled-turn");
    initialize_project(root.path());
    let definition = definition(root.path(), false, None);
    let server = spawn_pending_responses_server();
    let config = config(root.path(), &server.base_url, Some(true));
    let child = spawn_f2a_chat(
        root.path(),
        &config,
        "task\n/exit\n",
        &["--agent", definition.to_str().unwrap()],
    );
    server.requests.recv_timeout(F2A_WAIT).unwrap();
    assert!(
        Command::new("/bin/kill")
            .args(["-INT", &child.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    let output = f2a_wait_child(child, "auto cancellation");
    server.join();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[request cancelled]"));
    assert!(!stdout.contains("Automatic completed-Turn compaction"));
    assert!(compactions(root.path()).is_empty());
}

#[test]
fn automatic_compaction_skips_a_completed_boundary_cancelled_during_rsi_diagnosis() {
    let root = TemporaryDirectory::new("auto-rsi-cancelled");
    initialize_project(root.path());
    let definition = definition(root.path(), true, None);
    let manifest = definition.join("agent.toml");
    fs::write(
        &manifest,
        fs::read_to_string(&manifest)
            .unwrap()
            .replace("[rsi]", "[rsi]\nimprovement = true"),
    )
    .unwrap();
    let server = spawn_f2a_first_response_then_pending_server(text_response(
        &"summary ".repeat(9_000),
        "auto-before-diagnosis",
    ));
    let config = config(root.path(), &server.base_url, Some(true));
    let child = spawn_f2a_chat(
        root.path(),
        &config,
        "[rsi-feedback] improve this workflow\n/exit\n",
        &["--agent", definition.to_str().unwrap()],
    );
    let child_id = child.id();
    let waiter = thread::spawn(move || f2a_wait_child(child, "auto RSI boundary cancellation"));
    let _primary = server.requests.recv_timeout(F2A_WAIT).unwrap();
    let diagnosis = request_body(&server.requests.recv_timeout(F2A_WAIT).unwrap());
    assert!(diagnosis.to_string().contains("RSI2 diagnosis"));
    assert!(
        Command::new("/bin/kill")
            .args(["-INT", &child_id.to_string()])
            .status()
            .unwrap()
            .success()
    );
    let output = waiter.join().unwrap();
    server.release.send(()).unwrap();
    server.thread.join().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains("Automatic completed-Turn compaction")
    );
    assert!(compactions(root.path()).is_empty());
}

#[test]
fn automatic_compaction_visibly_defers_missing_research_prerequisite_and_no_reduction() {
    for (no_reduction, no_target) in [(false, false), (false, true), (true, false)] {
        let root = TemporaryDirectory::new("auto-prerequisite");
        initialize_project(root.path());
        if no_target {
            use echidna::domain::artifacts::BlobId;
            use echidna::domain::memory::{MemoryEvent, MemoryPath, MemoryRevision, MemoryScope};
            let project = Project::discover(root.path()).unwrap();
            append_research_event(
                &project,
                "auto-no-target-memory",
                ResearchEvent::Memory(MemoryEvent::Changed(MemoryRevision {
                    scope: MemoryScope::Project,
                    path: MemoryPath::parse("core.md").unwrap(),
                    old_hash: None,
                    new_hash: Some("a".repeat(64)),
                    old_blob_id: None,
                    new_blob_id: Some(BlobId::new("b".repeat(64)).unwrap()),
                    diff_blob_id: Some(BlobId::new("c".repeat(64)).unwrap()),
                })),
            );
        }
        let response = if no_reduction {
            "small".to_owned()
        } else {
            "large completed notes ".repeat(10_000)
        };
        let server =
            spawn_responses_server(vec![(200, text_response(&response, "auto-prerequisite"))]);
        let config = config(root.path(), &server.base_url, Some(true));
        let definition = no_reduction.then(|| definition(root.path(), false, None));
        if let Some(definition) = &definition {
            fs::write(definition.join("persona.md"), "stable ".repeat(2_800)).unwrap();
            let manifest = definition.join("agent.toml");
            fs::write(
                &manifest,
                fs::read_to_string(&manifest)
                    .unwrap()
                    .replace("memory_write = \"direct\"", "")
                    .replace("run_command = \"direct\"", "")
                    .replace("start_job = \"direct\"", ""),
            )
            .unwrap();
            fs::write(
                &config,
                fs::read_to_string(&config).unwrap().replacen(
                    "context_window = 32768",
                    "context_window = 8192",
                    1,
                ),
            )
            .unwrap();
        }
        let extra = definition
            .as_ref()
            .map(|definition| vec!["--agent", definition.to_str().unwrap()])
            .unwrap_or_default();
        let output = run_chat(root.path(), &config, "task\n/exit\n", &extra);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        server.requests.recv_timeout(F2A_WAIT).unwrap();
        assert!(server.requests.try_recv().is_err());
        server.join();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains(if no_reduction {
                "no smaller usable next-Turn Context"
            } else if no_target {
                "the current Project has no TargetSnapshot"
            } else {
                "the Project Research stream is empty"
            }),
            "{stdout}"
        );
        assert!(compactions(root.path()).is_empty());
    }
}
