//! Auto-compaction samples explicit tool-free handoffs through the pinned Agent.

use super::task_context::{
    ScriptedResponse, selector_candidates, spawn_scripted_responses_server,
    spawn_scripted_responses_server_with_wait,
};
use super::*;

pub(super) fn continue_from_selected_summary(candidates: &Value) -> Value {
    let tasks = candidates["tasks"].as_array().unwrap();
    let summaries = candidates["context"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|unit| unit["ref"].as_str().unwrap().starts_with("summary:"))
        .collect::<Vec<_>>();
    assert_eq!(
        summaries.len(),
        1,
        "followup explicitly chooses its source-bound summary"
    );
    let task = &summaries[0]["task_ref"];
    assert!(task.is_string());
    assert!(tasks.iter().any(|candidate| candidate["ref"] == *task));
    json!({"relation":"continue","focus":{"kind":"existing","task_ref":task},
        "related_task_refs":[],"required_context_refs":[summaries[0]["ref"]],"unresolved_references":[]})
}

fn assert_summary_expansion(initial: &Value, expanded: &Value) {
    let initial = selector_candidates(initial);
    let expanded = selector_candidates(expanded);
    let summary = |candidates: Value| {
        candidates["context"]
            .as_array()
            .unwrap()
            .iter()
            .find(|unit| unit["ref"].as_str().unwrap().starts_with("summary:"))
            .unwrap()
            .clone()
    };
    let initial = summary(initial);
    let expanded = summary(expanded);
    assert_eq!(initial["truncated"], true);
    assert_eq!(initial["ref"], expanded["ref"]);
    assert!(
        expanded["excerpt"].as_str().unwrap().len() > initial["excerpt"].as_str().unwrap().len()
    );
}

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

fn pressure_policy(path: &std::path::Path, pressure_bps: u16, target_bps: u16) {
    let original = fs::read_to_string(path).unwrap();
    fs::write(path, format!("{original}\n[runtime.context_budget]\npressure_bps = {pressure_bps}\ntarget_bps = {target_bps}\n")).unwrap();
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

fn assert_handoff_request(request: &Value) -> Value {
    assert!(
        request.get("tools").is_none(),
        "summary cannot advertise tools"
    );
    assert!(request["previous_response_id"].is_null());
    assert!(
        request["instructions"]
            .as_str()
            .unwrap()
            .contains("continuation handoff")
    );
    fn source(value: &Value) -> Option<Value> {
        match value {
            Value::String(text) => serde_json::from_str::<Value>(text)
                .ok()
                .filter(|parsed| parsed.get("source_plan_hash").is_some()),
            Value::Array(values) => values.iter().find_map(source),
            Value::Object(values) => values.values().find_map(source),
            _ => None,
        }
    }
    let source = source(&request["input"]).expect("handoff contains a source-plan node");
    assert_eq!(source["schema_version"], 1);
    assert_eq!(source["source_plan_hash"].as_str().unwrap().len(), 64);
    assert!(!source["node_id"].as_str().unwrap().is_empty());
    assert!(source["protected_exact"].is_array());
    match source["input"]["kind"].as_str() {
        Some("source") => assert!(!source["input"]["excerpts"].as_array().unwrap().is_empty()),
        Some("merge") => assert!(source["input"]["children"].as_array().unwrap().len() >= 2),
        _ => panic!("unknown compaction source node"),
    }
    source
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
        let server = spawn_scripted_responses_server(vec![
            ScriptedResponse::InitialSelection,
            ScriptedResponse::Response(
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
            ScriptedResponse::Response(
                200,
                reasoning_answer("Scoped memory saved; clerk task complete.").replace(
                    "volatile-only-reasoning",
                    &"volatile-only-reasoning ".repeat(18_000),
                ),
            ),
            ScriptedResponse::Response(
                200,
                compaction_summary_response(
                    "The scoped memory was saved and clerk notes completed.",
                    "auto-handoff",
                ),
            ),
            ScriptedResponse::SelectionDecision(continue_from_selected_summary),
            ScriptedResponse::SelectionDecision(continue_from_selected_summary),
            ScriptedResponse::Response(200, text_response("NEXT-TASK-COMPLETE", "auto-next")),
        ]);
        let config = config(root.path(), &server.base_url, Some(true));
        fs::write(
            &config,
            format!(
                "{}\n[mcp_servers.fixture]\nurl = {}\nprotocol_version = \"2026-07-28\"\n",
                fs::read_to_string(&config)
                    .unwrap()
                    .replace("context_window = 32768", "context_window = 1048576"),
                serde_json::to_string(&mcp.base_url).unwrap()
            ),
        )
        .unwrap();
        // Explicitly make this finite fixture cross pressure in a 1M window;
        // default 80% remains covered by the capacity-policy tests.
        pressure_policy(&config, 1000, 500);
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
        selector_candidates(&f2a_request_body(
            &server.requests,
            "initial task selection",
        ));
        let requests: Vec<_> = (0..6)
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
        for request in [&requests[0], &requests[1], &requests[5]] {
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
        assert_handoff_request(&requests[2]);
        assert_summary_expansion(&requests[3], &requests[4]);
        assert!(json_contains_text(
            &requests[5]["input"],
            "CLERK-AGENT-SCOPED-MEMORY"
        ));
        assert!(json_contains_text(&requests[5]["input"], "second task"));
        assert!(requests[5]["previous_response_id"].is_null());
        assert!(!json_contains_type(&requests[5]["input"], "reasoning"));
        assert!(!requests[5].to_string().contains("volatile-only-reasoning"));
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
        assert_eq!(state.request_audits().len(), 7);
        let audits = serde_json::to_value(&state.request_audits()[1..]).unwrap();
        assert_ne!(
            audits[2]["provider_session"], audits[1]["provider_session"],
            "summary has its own ProviderSession"
        );
        assert_eq!(
            state.request_audits()[6].delivery().reason(),
            echidna::domain::request_audit::RequestAuditDeliveryReason::Full(
                echidna::domain::context::BaselineInvalidationReason::CompactionReset
            )
        );
        let compaction_event = events
            .iter()
            .find(|event| event["event_type"] == "session.compaction_created")
            .expect("the completed-Turn compaction is durable");
        let next_primary_audit = events
            .iter()
            .find(|event| event["event_id"] == state.request_audits()[6].audit_event_id().as_str())
            .expect("the next Primary audit is durable");
        assert!(
            compaction_event["sequence"].as_u64().unwrap()
                < next_primary_audit["sequence"].as_u64().unwrap()
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
        assert_eq!(summary["schema_version"], 7);
        assert_eq!(
            summary["narrative"]["concise_progress"],
            "The scoped memory was saved and clerk notes completed."
        );
        let sample = events
            .iter()
            .find(|event| event["event_type"] == "session.compaction_sample_recorded")
            .unwrap();
        assert_eq!(summary["model_sample_event_ref"], sample["event_id"]);
        assert_eq!(
            sample["payload"]["request_audit_event_id"],
            state.request_audits()[3].audit_event_id().as_str()
        );
        assert_eq!(
            sample["payload"]["provider_session"],
            audits[2]["provider_session"]
        );
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
fn oversized_completed_handoff_regenerates_and_adopts_only_the_valid_summary() {
    let root = TemporaryDirectory::new("auto-compaction-size-regeneration");
    initialize_project(root.path());
    let definition = definition(root.path(), false, None);
    let server = spawn_scripted_responses_server(vec![
        ScriptedResponse::InitialSelection,
        ScriptedResponse::Response(
            200,
            reasoning_answer("PARENT-DONE").replace(
                "volatile-only-reasoning",
                &"parent-reasoning ".repeat(18_000),
            ),
        ),
        ScriptedResponse::Response(
            200,
            compaction_summary_response(&"界".repeat(20_000), "oversized-summary"),
        ),
        ScriptedResponse::Response(
            200,
            compaction_summary_response("REGENERATED-HANDOFF", "valid-summary"),
        ),
        ScriptedResponse::SelectionDecision(continue_from_selected_summary),
        ScriptedResponse::SelectionDecision(continue_from_selected_summary),
        ScriptedResponse::Response(200, text_response("FOLLOWUP-DONE", "followup")),
    ]);
    let config = config(root.path(), &server.base_url, Some(true));
    fs::write(
        &config,
        fs::read_to_string(&config)
            .unwrap()
            .replace("context_window = 32768", "context_window = 272000"),
    )
    .unwrap();
    pressure_policy(&config, 1000, 500);
    let output = run_chat(
        root.path(),
        &config,
        "first task\nfollowup task\n/exit\n",
        &["--agent", definition.to_str().unwrap(), "--debug"],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    selector_candidates(&f2a_request_body(
        &server.requests,
        "initial task selection",
    ));
    let requests = (0..6)
        .map(|_| request_body(&server.requests.recv_timeout(F2A_WAIT).unwrap()))
        .collect::<Vec<_>>();
    assert_handoff_request(&requests[1]);
    assert_eq!(
        requests[1], requests[2],
        "regeneration retains the exact prepared request"
    );
    assert_summary_expansion(&requests[3], &requests[4]);
    assert!(requests[5].to_string().contains("REGENERATED-HANDOFF"));
    assert!(!requests[5].to_string().contains(&"界".repeat(100)));
    assert!(server.requests.try_recv().is_err());
    server.join();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.contains("FOLLOWUP-DONE"), "{stdout}");
    assert!(
        stdout.contains("Automatic completed-Turn compaction applied"),
        "{stdout}"
    );
    assert!(
        stdout.contains("[request budget] admitted=4 primary=4 vision=0"),
        "{stdout}"
    );
    assert!(
        !stdout.contains("Automatic completed-Turn compaction deferred"),
        "{stdout}"
    );
    assert!(
        stderr.contains("model content appears in compaction output below"),
        "{stderr}"
    );
    assert!(
        stderr.contains("[debug compaction output · pending]"),
        "{stderr}"
    );
    assert!(stderr.contains("REGENERATED-HANDOFF"), "{stderr}");
    assert!(!stderr.contains("Vision output"), "{stderr}");
    let events = session_events(root.path(), &session_id(root.path()));
    assert_eq!(
        events
            .iter()
            .filter(|event| event["event_type"] == "session.compaction_sample_recorded")
            .count(),
        1
    );
    assert_eq!(compactions(root.path()).len(), 1);
}

#[test]
fn automatic_compaction_is_opt_in_and_low_pressure_retains_continuation() {
    for (enabled, large) in [(None, true), (Some(false), true), (Some(true), false)] {
        let root = TemporaryDirectory::new("auto-opt-in");
        initialize_project(root.path());
        let definition = definition(root.path(), false, None);
        let mut responses = vec![
            ScriptedResponse::InitialSelection,
            ScriptedResponse::Response(
                200,
                reasoning_answer(&"notes ".repeat(if large { 35_000 } else { 1 })),
            ),
        ];
        if !large {
            responses.push(ScriptedResponse::ContinueSelection { usage: None });
            responses.push(ScriptedResponse::Response(
                200,
                text_response("NEXT-LOW-PRESSURE", "low-next"),
            ));
        }
        let server = spawn_scripted_responses_server(responses);
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
        selector_candidates(&f2a_request_body(
            &server.requests,
            "initial task selection",
        ));
        server.requests.recv_timeout(F2A_WAIT).unwrap();
        if !large {
            let selector = request_body(&server.requests.recv_timeout(F2A_WAIT).unwrap());
            assert!(selector.get("tools").is_none());
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
        let mut handoff_indices = Vec::new();
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
                "item":{"type":"reasoning", "content":[{"type":"reasoning_text", "text":format!("batch-{index}-reasoning ").repeat(if index == 0 { 12_000 } else { 20_000 })}]} })
            );
            response.push_str(&m4_long_flow_tool_response(
                &format!("batch-{index}"),
                "run_command",
                json!({"argv":["/bin/sh", "-c",
                    format!("printf '%s\\n' batch-{index} >> batch-count.txt; printf '%s' \"$1\""),
                    "fixture", "neutral-output ".repeat(2_000)]}),
                &format!("batch-response-{index}"),
            ));
            responses.push((200, response));
            // First batch stays below this fixture's pressure threshold;
            // later batches cross it with older work available for reduction.
            if enabled && index > 0 {
                handoff_indices.push(responses.len());
                responses.push((200, compaction_summary_response(
                    &format!("Neutral command batches zero through {index} completed; continue without replaying them."),
                    &format!("batch-handoff-{index}"),
                )));
            }
        }
        responses.push((200, text_response("WORKING-SET-DONE", "batch-final")));
        let request_count = responses.len();
        let server = spawn_scripted_responses_server(
            std::iter::once(ScriptedResponse::InitialSelection)
                .chain(
                    responses
                        .into_iter()
                        .map(|(status, body)| ScriptedResponse::Response(status, body)),
                )
                .collect(),
        );
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
        pressure_policy(&config, 1000, 500);
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
        let initial_selection = f2a_request_body(&server.requests, "initial image task selection");
        selector_candidates(&initial_selection);
        let requests: Vec<_> = (0..request_count)
            .map(|index| request_body(&server.requests.recv_timeout(F2A_WAIT).unwrap_or_else(|error| {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let tail = stdout.char_indices().rev().nth(2000).map_or(stdout.as_ref(), |(index, _)| &stdout[index..]);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let stderr_tail = stderr.char_indices().rev().nth(1500).map_or(stderr.as_ref(), |(index, _)| &stderr[index..]);
                panic!("request {index}/{request_count} missing (enabled={enabled}, incremental={incremental}): {error}; stdout tail: {tail}; stderr tail: {stderr_tail}")
            })))
            .collect();
        assert!(server.requests.try_recv().is_err());
        server.join();
        assert!(String::from_utf8_lossy(&output.stdout).contains("WORKING-SET-DONE"));
        assert_eq!(
            fs::read_to_string(root.path().join("batch-count.txt")).unwrap(),
            "batch-0\nbatch-1\nbatch-2\n",
            "every real external mutation executes exactly once"
        );
        for index in &handoff_indices {
            assert_handoff_request(&requests[*index]);
            assert!(!json_contains_type(&requests[*index]["input"], "reasoning"));
        }
        for index in handoff_indices.iter().skip(1) {
            let source = assert_handoff_request(&requests[*index]);
            let previous = source["input"]["excerpts"]
                .as_array()
                .unwrap()
                .iter()
                .find(|entry| entry["source"]["kind"] == "summary")
                .expect("later handoff retains the previous summary source");
            let projection: Value =
                serde_json::from_str(previous["excerpt"].as_str().unwrap()).unwrap();
            assert_eq!(projection["projection_version"], 1);
            assert!(projection.get("task_references").is_none());
        }
        for (index, request) in requests.iter().enumerate() {
            if handoff_indices.contains(&index) {
                continue;
            }
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
        assert_eq!(state.request_audits().len(), request_count + 1);
        assert_eq!(
            events
                .iter()
                .filter(|event| event["event_type"] == "session.compaction_sample_recorded")
                .count(),
            handoff_indices.len(),
            "every extra summary request produces one separately scoped sample"
        );
        let resets: Vec<_> = state
            .request_audits()
            .iter()
            .skip(1)
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
                "tool-batch pressure must trigger before completion (incremental={incremental}; compactions={}; delivery_reasons={:?}; stdout={})",
                compactions(root.path()).len(),
                state
                    .request_audits()
                    .iter()
                    .map(|audit| audit.delivery().reason())
                    .collect::<Vec<_>>(),
                String::from_utf8_lossy(&output.stdout)
            );
            assert!(resets.iter().all(|index| *index > 0));
            assert_eq!(
                handoff_indices,
                if incremental { vec![3, 5] } else { vec![2, 4] }
            );
            assert_eq!(resets.len(), 2);
            assert_eq!(compactions(root.path()).len(), 2);
            if incremental {
                let activation = events
                    .iter()
                    .find(|event| {
                        event["event_type"] == "session.history_item_recorded"
                            && event["payload"]["item"]["kind"] == "tool_result"
                            && event["payload"]["item"]["result"]["call_id"] == "activate-command"
                    })
                    .expect("the current Turn has a durable tool activation");
                assert!(
                    compactions(root.path()).iter().any(|compaction| {
                        !compaction["payload"]["retained_event_ids"]
                            .as_array()
                            .unwrap()
                            .contains(&activation["event_id"])
                    }),
                    "the activated tool remains available after its source batch is compacted"
                );
            }
            let first_compaction = compactions(root.path()).remove(0);
            let summary = &first_compaction["payload"]["summary"];
            let plan_path = root
                .path()
                .join(".echidna/artifacts/blobs/sha256")
                .join(summary["source_plan_blob_id"].as_str().unwrap());
            let plan: Value = serde_json::from_slice(&fs::read(plan_path).unwrap()).unwrap();
            let entries = plan["entries"].as_array().unwrap();
            let result = |call_id: &str| {
                events
                    .iter()
                    .find(|event| {
                        event["event_type"] == "session.history_item_recorded"
                            && event["payload"]["item"]["kind"] == "tool_result"
                            && event["payload"]["item"]["result"]["call_id"] == call_id
                    })
                    .unwrap()
            };
            let older = result("batch-0");
            let latest = result("batch-1");
            let selection = events
                .iter()
                .find(|event| event["event_id"] == summary["task_selection_event"])
                .unwrap();
            assert!(
                !selection["payload"]["refresh_of"].is_null(),
                "fixture must exercise a durable source refresh"
            );
            assert!(
                older["sequence"].as_u64().unwrap() < selection["sequence"].as_u64().unwrap(),
                "the earlier settled batch predates the refreshed selection"
            );
            for (event, protected) in [(older, false), (latest, true)] {
                let entry = entries
                    .iter()
                    .find(|entry| {
                        entry["source"]["kind"] == "history"
                            && entry["source"]["id"] == event["event_id"]
                    })
                    .expect("actual batch source must be in the verified compaction plan");
                assert_eq!(entry["protected_exact"], protected);
            }
            for compaction in compactions(root.path()) {
                let payload = &compaction["payload"];
                assert_eq!(payload["schema_version"], 7);
                assert_eq!(payload["summary"]["schema_version"], 7);
                assert!(
                    !payload["summary"]["task_scope"]
                        .as_array()
                        .unwrap()
                        .is_empty()
                );
                let retained = payload["retained_history"].as_array().unwrap();
                let references = payload["retained_event_ids"].as_array().unwrap();
                assert_eq!(retained.len(), references.len());
                assert!(!references.is_empty());
                for (item, reference) in retained.iter().zip(references) {
                    let source = events
                        .iter()
                        .find(|event| event["event_id"] == *reference)
                        .expect("retained input and complete batch have genuine History EventIds");
                    assert_eq!(source["event_type"], "session.history_item_recorded");
                    assert_eq!(source["payload"]["item"], *item);
                }
            }
            for index in resets {
                assert!(!json_contains_type(&requests[index]["input"], "reasoning"));
                assert!(handoff_indices.contains(&(index - 1)));
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
        let resumed = run_chat(
            root.path(),
            &config,
            "/context\n/exit\n",
            &["--resume", "latest"],
        );
        assert!(
            resumed.status.success(),
            "{}",
            String::from_utf8_lossy(&resumed.stderr)
        );
        assert_eq!(
            fs::read_to_string(root.path().join("batch-count.txt")).unwrap(),
            "batch-0\nbatch-1\nbatch-2\n",
            "Resume never repeats a compacted mutation"
        );
        if !incremental {
            measured.push(
                initial_selection.to_string().len()
                    + requests
                        .iter()
                        .map(|request| request.to_string().len())
                        .sum::<usize>(),
            );
        }
    }
    assert!(
        measured[1] * 100 < measured[0] * 65,
        "opted-in working set must reduce total Full payload bytes, including summary requests: {measured:?}"
    );
    eprintln!(
        "Full request bytes, baseline={} bounded={}",
        measured[0], measured[1]
    );
}

#[test]
#[cfg(debug_assertions)]
fn failed_tool_handoff_still_assesses_later_settled_batches_without_resampling() {
    let root = TemporaryDirectory::new("auto-tool-deferred-assessment");
    initialize_project(root.path());
    let definition = definition(root.path(), false, None);
    let mut responses = Vec::new();
    for index in 0..3 {
        let mut response = format!(
            "data: {}\n\n",
            json!({
                "type":"response.output_item.done", "output_index":0,
                "item":{"type":"reasoning", "content":[{"type":"reasoning_text", "text":format!("batch-{index}-reasoning ").repeat(if index == 0 { 12_000 } else { 20_000 })}]}
            })
        );
        response.push_str(&m4_long_flow_tool_response(
            &format!("batch-{index}"),
            "run_command",
            json!({"argv":["/usr/bin/printf", "%s", "neutral-output ".repeat(2_000)]}),
            &format!("batch-response-{index}"),
        ));
        responses.push((200, response));
        if index == 1 {
            responses.push((200, text_response("invalid handoff", "bad-handoff")));
        }
    }
    responses.push((200, text_response("WORKING-SET-DONE", "final")));
    let server = spawn_scripted_responses_server(
        std::iter::once(ScriptedResponse::InitialSelection)
            .chain(
                responses
                    .into_iter()
                    .map(|(status, body)| ScriptedResponse::Response(status, body)),
            )
            .collect(),
    );
    let config = config(root.path(), &server.base_url, Some(true));
    fs::write(
        &config,
        fs::read_to_string(&config)
            .unwrap()
            .replace("context_window = 32768", "context_window = 1048576"),
    )
    .unwrap();
    pressure_policy(&config, 1000, 500);
    let output = run_chat(
        root.path(),
        &config,
        "Continue the task.\n/exit\n",
        &["--agent", definition.to_str().unwrap(), "--debug"],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    selector_candidates(&f2a_request_body(
        &server.requests,
        "initial task selection",
    ));
    let requests = (0..5)
        .map(|_| request_body(&server.requests.recv_timeout(F2A_WAIT).unwrap()))
        .collect::<Vec<_>>();
    assert_handoff_request(&requests[2]);
    assert!(requests[3].get("tools").is_some());
    assert!(server.requests.try_recv().is_err());
    server.join();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("[debug context pressure] settled batch evaluated; threshold_reached=true; automatic handoff deferred for this Turn"), "{stderr}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Automatic tool-batch compaction deferred"),
        "{stdout}"
    );
    let project = Project::discover(root.path()).unwrap();
    let state = reduced_session(&project, &session_id(root.path()));
    assert_eq!(state.execution_receipt_count(), 3);
    assert!(compactions(root.path()).is_empty());
}

#[test]
fn large_protected_task_allows_useful_reduction_but_defers_without_old_work() {
    for earlier_work in [false, true] {
        let root = TemporaryDirectory::new("auto-large-active-task");
        initialize_project(root.path());
        let definition = definition(root.path(), false, None);
        let mut responses = vec![
            ScriptedResponse::NewSelection,
            ScriptedResponse::NewSelection,
            ScriptedResponse::Response(
                200,
                m4_long_flow_tool_response(
                    "first-large-task",
                    "run_command",
                    json!({"argv":["/usr/bin/printf", "%s", "neutral-output ".repeat(7_000)]}),
                    "large-task-first",
                ),
            ),
        ];
        if earlier_work {
            responses.push(ScriptedResponse::Response(
                200,
                m4_long_flow_tool_response(
                    "second-large-task",
                    "run_command",
                    json!({"argv":["/usr/bin/printf", "%s", "later-output ".repeat(7_000)]}),
                    "large-task-second",
                ),
            ));
            responses.push(ScriptedResponse::Response(
                200,
                compaction_summary_response(
                    "The earlier command finished; keep the current task and latest output.",
                    "large-task-handoff",
                ),
            ));
        }
        responses.push(ScriptedResponse::Response(
            200,
            text_response("LARGE-TASK-DONE", "large-task-final"),
        ));
        let request_count = responses.len();
        // Bound fixture liveness separately from product latency.
        let server = spawn_scripted_responses_server_with_wait(responses, Duration::from_secs(60));
        let config = config(root.path(), &server.base_url, Some(true));
        fs::write(
            &config,
            fs::read_to_string(&config)
                .unwrap()
                .replace("context_window = 32768", "context_window = 1048576"),
        )
        .unwrap();
        // The protected input exceeds the soft target but fits the hard budget.
        pressure_policy(&config, 500, 200);
        let task = format!(
            "Keep this entire operator input: {}",
            "constraint ".repeat(20_000)
        );
        let usable_input = 1_048_576 - 2 * (1_048_576 / 8);
        assert!(task.len() / 4 > usable_input * 2 / 100);
        assert!(task.len() / 4 < usable_input);
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
        let requests = (0..request_count)
            .map(|_| request_body(&server.requests.recv_timeout(F2A_WAIT).unwrap()))
            .collect::<Vec<_>>();
        assert!(json_contains_text(&requests[2]["input"], &task));
        assert_eq!(requests[3]["previous_response_id"], "large-task-first");
        server.join();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("Compaction did not yield a smaller usable next-Turn Context"),
            "{stdout}"
        );
        assert!(stdout.contains("LARGE-TASK-DONE"));
        if earlier_work {
            assert_handoff_request(&requests[4]);
            assert!(requests[5]["previous_response_id"].is_null());
            assert!(json_contains_text(&requests[5]["input"], &task));
            assert!(stdout.contains("Automatic tool-batch compaction applied"));
            assert_eq!(compactions(root.path()).len(), 1);
        } else {
            assert!(!stdout.contains("Automatic tool-batch compaction applied"));
            assert!(compactions(root.path()).is_empty());
        }
        let project = Project::discover(root.path()).unwrap();
        let state = reduced_session(&project, &session_id(root.path()));
        assert_eq!(state.execution_receipt_count(), 1 + u64::from(earlier_work));
        assert_eq!(
            serde_json::to_string(state.history().items())
                .unwrap()
                .matches(&task)
                .count(),
            1
        );
    }
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
    let server = spawn_scripted_responses_server(vec![
        ScriptedResponse::InitialSelection,
        ScriptedResponse::Response(200, first),
        ScriptedResponse::Response(
            200,
            compaction_summary_response(
                "The first task finished with a short final answer.",
                "reasoning-handoff",
            ),
        ),
        ScriptedResponse::NewSelection,
        ScriptedResponse::Response(200, text_response("NEXT-COMPLETE", "next-reasoning")),
    ]);
    let config = config(root.path(), &server.base_url, Some(true));
    fs::write(
        &config,
        fs::read_to_string(&config)
            .unwrap()
            .replace("context_window = 32768", "context_window = 1048576"),
    )
    .unwrap();
    pressure_policy(&config, 1000, 500);
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
    selector_candidates(&f2a_request_body(
        &server.requests,
        "initial task selection",
    ));
    server.requests.recv_timeout(F2A_WAIT).unwrap();
    let handoff = request_body(&server.requests.recv_timeout(F2A_WAIT).unwrap());
    assert_handoff_request(&handoff);
    assert!(!handoff.to_string().contains("retained-reasoning"));
    let selector = request_body(&server.requests.recv_timeout(F2A_WAIT).unwrap());
    assert_eq!(selector_candidates(&selector)["input"]["text"], "next task");
    let next = request_body(&server.requests.recv_timeout(F2A_WAIT).unwrap());
    assert!(server.requests.try_recv().is_err());
    server.join();
    assert!(next["previous_response_id"].is_null());
    assert!(!json_contains_type(&next["input"], "reasoning"));
    assert!(json_contains_text(&next["input"], "next task"));
    assert_eq!(compactions(root.path()).len(), 1);
}

#[test]
#[cfg(debug_assertions)]
fn settled_batch_pressure_uses_core_written_by_short_command() {
    // The 204k-token usable input has a 20.4k trigger; 156k Core
    // bytes add about 39k tokens while the short delivery stays below it.
    for large_core in [false, true] {
        let root = TemporaryDirectory::new("auto-current-core-pressure");
        initialize_project(root.path());
        let definition = definition(root.path(), false, None);
        let project = Project::discover(root.path()).unwrap();
        let core = root
            .path()
            .join(".echidna/memory/agents/order-clerk/core.md");
        fs::create_dir_all(core.parent().unwrap()).unwrap();
        fs::write(
            root.path().join("core-seed"),
            if large_core {
                "current-core ".repeat(12_000)
            } else {
                String::new()
            },
        )
        .unwrap();
        let core_relative = core.strip_prefix(root.path()).unwrap().to_str().unwrap();
        let server = spawn_scripted_responses_server(vec![
            ScriptedResponse::InitialSelection,
            ScriptedResponse::Response(
                200,
                m4_long_flow_tool_response(
                    "prior",
                    "run_command",
                    json!({"argv":["/usr/bin/printf","small"]}),
                    "prior-response",
                ),
            ),
            ScriptedResponse::Response(
                200,
                m4_long_flow_tool_response(
                    "write-core",
                    "run_command",
                    json!({"argv":["/bin/cp","core-seed",core_relative]}),
                    "write-core-response",
                ),
            ),
        ]);
        let config = config(root.path(), &server.base_url, Some(true));
        fs::write(
            &config,
            format!(
                "{}\nmodel_request_limit_per_turn = 3\n",
                fs::read_to_string(&config)
                    .unwrap()
                    .replace("context_window = 32768", "context_window = 272000")
            ),
        )
        .unwrap();
        pressure_policy(&config, 1000, 500);
        let output = run_chat(
            root.path(),
            &config,
            "Complete the task.\n/exit\n",
            &["--agent", definition.to_str().unwrap(), "--debug"],
        );
        selector_candidates(&f2a_request_body(
            &server.requests,
            "initial task selection",
        ));
        for _ in 0..2 {
            server.requests.recv_timeout(F2A_WAIT).unwrap();
        }
        assert!(server.requests.try_recv().is_err());
        server.join();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{stderr}");
        assert!(
            stderr.contains("per-Turn model-request limit of 3"),
            "{stderr}"
        );
        let expected = format!(
            "settled batch evaluated; threshold_reached={large_core}; automatic handoff deferred for this Turn"
        );
        assert!(stderr.contains(&expected), "{stderr}");
        assert_eq!(
            fs::read_to_string(&core).unwrap().len(),
            if large_core { 156_000 } else { 0 }
        );
        let state = reduced_session(&project, &session_id(root.path()));
        assert_eq!(state.execution_receipt_count(), 2);
        assert!(compactions(root.path()).is_empty());
    }
}

#[test]
#[cfg(debug_assertions)]
fn tool_batch_does_not_compact_when_vision_and_selection_use_shared_admission() {
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
    let primary = spawn_scripted_responses_server(vec![
        ScriptedResponse::InitialSelection,
        ScriptedResponse::Response(200, response),
    ]);
    let config = write_dual_config(root.path(), &primary.base_url, &vision.base_url);
    fs::write(
        &config,
        format!(
            "{}\n[runtime]\nauto_compact = true\nmodel_request_limit_per_turn = 3\n",
            fs::read_to_string(&config)
                .unwrap()
                .replace("context_window = 32768", "context_window = 1048576")
        ),
    )
    .unwrap();
    pressure_policy(&config, 200, 100);
    let output = run_chat(
        root.path(),
        &config,
        "/image screen.png\nComplete this task under the shared budget.\n/exit\n",
        &["--agent", definition.to_str().unwrap(), "--debug"],
    );
    selector_candidates(&f2a_request_body(
        &primary.requests,
        "initial visual task selection",
    ));
    primary.requests.recv_timeout(F2A_WAIT).unwrap();
    assert!(primary.requests.try_recv().is_err());
    primary.join();
    vision.requests.recv_timeout(F2A_WAIT).unwrap();
    assert!(vision.requests.try_recv().is_err());
    vision.join();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("[debug context pressure] settled batch evaluated; threshold_reached=true; automatic handoff deferred for this Turn"), "{stderr}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("per-Turn model-request limit of 3"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let project = Project::discover(root.path()).unwrap();
    let state = reduced_session(&project, &session_id(root.path()));
    assert_eq!(state.request_audits().len(), 3);
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
fn automatic_compaction_samples_handoff_after_completed_rsi_diagnosis() {
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
    let server = spawn_scripted_responses_server(vec![
        ScriptedResponse::InitialSelection,
        ScriptedResponse::Response(
            200,
            text_response(&"summary ".repeat(9_000), "auto-before-diagnosis"),
        ),
        ScriptedResponse::Response(
            200,
            text_response(
                r#"{"decision":"no_change","reason":"No justified reusable improvement."}"#,
                "auto-diagnosis-completed",
            ),
        ),
        ScriptedResponse::Response(
            200,
            compaction_summary_response(
                "The task finished; RSI diagnosed no justified reusable change.",
                "auto-after-diagnosis-handoff",
            ),
        ),
        ScriptedResponse::NewSelection,
        ScriptedResponse::Response(200, text_response("NEXT-AFTER-RSI", "auto-after-rsi")),
    ]);
    let config = config(root.path(), &server.base_url, Some(true));
    pressure_policy(&config, 5000, 3000);
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
    selector_candidates(&f2a_request_body(
        &server.requests,
        "initial task selection",
    ));
    let _primary = server.requests.recv_timeout(F2A_WAIT).unwrap();
    let diagnosis = request_body(&server.requests.recv_timeout(F2A_WAIT).unwrap());
    assert!(diagnosis.to_string().contains("RSI2 diagnosis"));
    assert!(diagnosis.get("tools").is_none());
    let handoff = request_body(&server.requests.recv_timeout(F2A_WAIT).unwrap());
    assert_handoff_request(&handoff);
    let selector = request_body(&server.requests.recv_timeout(F2A_WAIT).unwrap());
    assert!(selector.get("tools").is_none());
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
        6,
        "two ordinary requests, one RSI diagnosis, one compaction and two task selections"
    );
    assert_eq!(
        state.request_audits()[5].delivery().reason(),
        echidna::domain::request_audit::RequestAuditDeliveryReason::Full(
            echidna::domain::context::BaselineInvalidationReason::CompactionReset
        )
    );
    let audits = serde_json::to_value(&state.request_audits()[1..]).unwrap();
    assert_ne!(audits[1]["provider_session"], audits[2]["provider_session"]);
    assert_ne!(audits[4]["provider_session"], audits[0]["provider_session"]);
    assert_ne!(audits[4]["provider_session"], audits[3]["provider_session"]);
    let selected = events
        .iter()
        .rposition(|event| event["event_type"] == "session.task_context_selected")
        .unwrap();
    let main_audit = events
        .iter()
        .position(|event| event["event_id"] == state.request_audits()[5].audit_event_id().as_str())
        .unwrap();
    assert!(compacted < selected && selected < main_audit);
    let sampled = events
        .iter()
        .position(|event| event["event_type"] == "session.compaction_sample_recorded")
        .unwrap();
    assert!(last_rsi < sampled && sampled < compacted);
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
        let server = spawn_scripted_responses_server(vec![
            ScriptedResponse::InitialSelection,
            ScriptedResponse::Response(200, response),
        ]);
        let config = config(root.path(), &server.base_url, Some(true));
        fs::write(
            &config,
            format!(
                "{}\nmodel_request_limit_per_turn = 2\n",
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
        selector_candidates(&f2a_request_body(
            &server.requests,
            "initial task selection",
        ));
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
    let server = spawn_pending_responses_server(true);
    let config = config(root.path(), &server.base_url, Some(true));
    let child = spawn_f2a_chat(
        root.path(),
        &config,
        "task\n/exit\n",
        &["--agent", definition.to_str().unwrap()],
    );
    selector_candidates(&f2a_request_body(
        &server.requests,
        "initial task selection",
    ));
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
    let server = spawn_f2a_first_response_then_pending_server(
        text_response(&"summary ".repeat(9_000), "auto-before-diagnosis"),
        true,
    );
    let config = config(root.path(), &server.base_url, Some(true));
    let child = spawn_f2a_chat(
        root.path(),
        &config,
        "[rsi-feedback] improve this workflow\n/exit\n",
        &["--agent", definition.to_str().unwrap()],
    );
    let child_id = child.id();
    let waiter = thread::spawn(move || f2a_wait_child(child, "auto RSI boundary cancellation"));
    selector_candidates(&f2a_request_body(
        &server.requests,
        "initial task selection",
    ));
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
fn automatic_task_compaction_needs_no_fake_research_target_and_defers_no_reduction() {
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
        let mut responses = vec![(200, text_response(&response, "auto-prerequisite"))];
        if !no_reduction {
            responses.push((200, compaction_summary_response(
                "The ordinary task finished; no Research target or verified evidence was created.",
                "task-without-research-handoff",
            )));
        }
        let server = spawn_scripted_responses_server(
            std::iter::once(ScriptedResponse::InitialSelection)
                .chain(
                    responses
                        .into_iter()
                        .map(|(status, body)| ScriptedResponse::Response(status, body)),
                )
                .collect(),
        );
        let config = config(root.path(), &server.base_url, Some(true));
        if !no_reduction {
            fs::write(
                &config,
                fs::read_to_string(&config)
                    .unwrap()
                    .replace("context_window = 32768", "context_window = 272000"),
            )
            .unwrap();
            pressure_policy(&config, 1000, 500);
        }
        let definition = no_reduction.then(|| definition(root.path(), false, None));
        if let Some(definition) = &definition {
            // Task-context now requires a source-bound receive-time fragment.
            // Leave room for it in this near-capacity no-reduction fixture;
            // the same 8192-token window and pressure policy still apply.
            fs::write(definition.join("persona.md"), "stable ".repeat(2_784)).unwrap();
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
        selector_candidates(&f2a_request_body(
            &server.requests,
            "initial task selection",
        ));
        let primary = request_body(&server.requests.recv_timeout(F2A_WAIT).unwrap());
        if no_reduction {
            assert!(json_contains_text(&primary["input"], "receive-time anchor"));
        }
        if !no_reduction {
            let handoff = request_body(&server.requests.recv_timeout(F2A_WAIT).unwrap_or_else(|error| {
                let tail = |bytes: &[u8]| String::from_utf8_lossy(bytes).chars().rev().take(4000).collect::<String>().chars().rev().collect::<String>();
                panic!("missing no-target handoff ({no_reduction}, {no_target}): {error}; stdout tail={} stderr tail={}", tail(&output.stdout), tail(&output.stderr));
            }));
            assert_handoff_request(&handoff);
        }
        assert!(server.requests.try_recv().is_err());
        server.join();
        let stdout = String::from_utf8_lossy(&output.stdout);
        if no_reduction {
            assert!(
                stdout.contains("no smaller usable next-Turn Context"),
                "{stdout}"
            );
            assert!(compactions(root.path()).is_empty());
        } else {
            assert!(
                stdout.contains("Automatic completed-Turn compaction applied"),
                "{stdout}"
            );
            let records = compactions(root.path());
            assert_eq!(records.len(), 1);
            assert_eq!(records[0]["payload"]["summary"]["schema_version"], 7);
            assert!(records[0]["payload"]["summary"]["evidence_spine"].is_null());
            let project = Project::discover(root.path()).unwrap();
            let mut writer = project.open_writer().unwrap();
            let store = writer.open_research(None).unwrap();
            let project_events =
                EventSource::read_events(&store, &ProjectEventStream::Research.stream_id())
                    .unwrap();
            assert_eq!(
                project_events.len(),
                usize::from(no_target),
                "compaction never invents Research facts"
            );
            assert!(
                project_events
                    .iter()
                    .all(|event| !event.event_type.as_str().contains("target_snapshot"))
            );
        }
    }
}
