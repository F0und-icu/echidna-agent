//! Automatic task-context acceptance uses synthetic inputs and an explicit
//! scripted Provider. These tests verify delivery mechanics, not model routing
//! quality, market facts, or Live latency.
use super::*;
use std::collections::BTreeMap;

pub(super) fn definition(root: &std::path::Path, context_read: Option<&str>) -> std::path::PathBuf {
    let path = root.join("task-context-agent");
    fs::create_dir(&path).unwrap();
    fs::write(
        path.join("persona.md"),
        "A neutral synthetic analysis assistant.",
    )
    .unwrap();
    fs::write(
        path.join("work.md"),
        "Keep original constraints and exact sources. Never replay a completed mutation.",
    )
    .unwrap();
    let mut manifest = String::from(
        "schema_version = 1\nid = \"task-context-fixture\"\nname = \"Task context fixture\"\n\
         [instructions]\npersona_file = \"persona.md\"\nwork_file = \"work.md\"\nmethod_files = []\n\
         [context]\npersonal = \"none\"\nproject = \"none\"\nmemory = \"none\"\n\
         [tools]\nrun_command = \"direct\"\n",
    );
    if let Some(exposure) = context_read {
        manifest.push_str(&format!("context_read = \"{exposure}\"\n"));
    }
    fs::write(path.join("agent.toml"), manifest).unwrap();
    path
}

#[test]
fn task_context_first_input_and_tool_loop_preserve_exact_input_and_execute_once() {
    for explicit_exposure in [None, Some("hidden")] {
        let root = TemporaryDirectory::new("task-context-first-tool-loop");
        initialize_project(root.path());
        let agent = definition(root.path(), explicit_exposure);
        let original_manifest = fs::read(agent.join("agent.toml")).unwrap();
        let server = spawn_scripted_responses_server(vec![
            ScriptedResponse::InitialSelection,
            ScriptedResponse::Response(
                200,
                m4_long_flow_tool_response(
                    "only-execution",
                    "run_command",
                    json!({"argv":["/bin/sh","-c","printf 'executed\\n' >> execution-count.txt"]}),
                    "first-task-tool-response",
                ),
            ),
            ScriptedResponse::Response(
                200,
                text_response("TASK-FINISHED", "first-task-final-response"),
            ),
        ]);
        let config = write_context_window_config(root.path(), &server.base_url, 272_000);
        let input = "Append exactly one line for TASK-ALPHA; retain limit 17 and label 原始约束.";
        let output = run_chat(
            root.path(),
            &config,
            &format!("{input}\n/exit\n"),
            &["--agent", agent.to_str().unwrap()],
        );
        assert!(
            output.status.success(),
            "stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let wires = (0..3)
            .map(|_| f2a_request_body(&server.requests, "task-context first tool loop"))
            .collect::<Vec<_>>();
        assert!(server.requests.try_recv().is_err());
        server.join();
        assert_eq!(
            fs::read_to_string(root.path().join("execution-count.txt")).unwrap(),
            "executed\n"
        );
        assert_eq!(
            fs::read(agent.join("agent.toml")).unwrap(),
            original_manifest
        );
        assert!(
            wires[1]
                .get("previous_response_id")
                .is_none_or(Value::is_null)
        );
        assert_eq!(wires[2]["previous_response_id"], "first-task-tool-response");
        assert_eq!(wires[1]["input"].to_string().matches(input).count(), 1);
        assert_eq!(selector_candidates(&wires[0])["input"]["text"], input);
        for wire in &wires[1..] {
            let names = wire["tools"]
                .as_array()
                .unwrap()
                .iter()
                .map(|tool| tool["name"].as_str().unwrap())
                .collect::<BTreeSet<_>>();
            assert!(names.contains("run_command"));
            assert_eq!(names.contains("context_read"), explicit_exposure.is_none());
        }
        let project = Project::discover(root.path()).unwrap();
        let state = reduced_session(&project, &session_id(root.path()));
        assert_eq!(
            state.request_audits().len(),
            3,
            "one first-input selection and two main requests are audited"
        );
        assert_eq!(state.execution_receipt_count(), 1);
        let history = state.history().items();
        assert_eq!(
            history.first(),
            Some(&echidna::domain::model::ContentItem::user_text(input))
        );
        assert_eq!(
            history
                .iter()
                .filter(|item| matches!(item, echidna::domain::model::ContentItem::ToolCall { .. }))
                .count(),
            1
        );
        assert_eq!(
            history
                .iter()
                .filter(|item| matches!(
                    item,
                    echidna::domain::model::ContentItem::ToolResult { .. }
                ))
                .count(),
            1
        );
        assert_eq!(
            history.last(),
            Some(&echidna::domain::model::ContentItem::assistant_text(
                "TASK-FINISHED"
            ))
        );
        let events = session_events(root.path(), &session_id(root.path()));
        let accepted = events
            .iter()
            .find(|event| {
                event["event_type"] == "session.operator_turn_boundary_recorded"
                    && event["payload"]["kind"] == "accepted"
            })
            .unwrap();
        assert_eq!(accepted["payload"]["schema_version"], 2);
        let anchor = accepted["payload"]["anchor"].as_str().unwrap();
        let time_text = wires[1]["input"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|item| item["content"].as_array().into_iter().flatten())
            .filter_map(|part| part["text"].as_str())
            .find(|text| text.starts_with(&format!("Input {anchor} receive-time anchor")))
            .unwrap();
        let delivered_time: Value = serde_json::from_str(
            &time_text[time_text.find('{').unwrap()..=time_text.rfind('}').unwrap()],
        )
        .unwrap();
        assert_eq!(delivered_time, accepted["payload"]["received_at"]);
        let provenance =
            serde_json::to_string(state.request_audits()[1].context_delivery_map().unwrap())
                .unwrap();
        assert!(provenance.contains(accepted["event_id"].as_str().unwrap()));
        assert!(provenance.contains(anchor));
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("TASK-FINISHED"));
        assert!(!stdout.contains("new_from_input"));
    }
}

const TASK_A: &str = "分析中际旭创（TASK-ALPHA），保留本次条件 A_LIMIT_17。";
const TASK_B: &str = "分析今日首板弱转强（TASK-BETA），仅使用 B_LIMIT_23。";
const FOLLOW_B: &str = "它们的量能呢？继续保留 B_LIMIT_23。";
const RETURN_A: &str = "回到中际旭创（TASK-ALPHA），之前的 A_LIMIT_17 仍保留。";
const COMPOSE: &str = "比较 TASK-ALPHA 与 TASK-BETA 的结果，保留两项来源和各自条件。";
const A_RESULT: &str = "A_ONLY_RESULT: synthetic alpha evidence value 17; observation revision A1.";
const B_RESULT: &str = "B_ONLY_RESULT: synthetic beta list B1/B2; observation revision B1.";

#[test]
fn task_context_entity_overlap_does_not_override_the_selected_goal_relationship() {
    // The Provider's choices are explicit oracles: this checks the harness
    // contract for goal relationships, not a model's semantic accuracy.
    for (continuing, next_input) in [
        (
            false,
            "For ACME, draft an unrelated onboarding checklist; do not use the earlier valuation.",
        ),
        (
            true,
            "Apply the same valuation task and its comparison criteria to BRAVO.",
        ),
    ] {
        let root = TemporaryDirectory::new("task-context-goal-versus-entity");
        initialize_project(root.path());
        let agent = definition(root.path(), None);
        let first_input = "Evaluate ACME using valuation criterion SOURCE-CRITERION-17.";
        let previous_answer =
            "GOAL-SPECIFIC-OBSERVATION-271: synthetic ACME valuation and comparison criteria.";
        let server = spawn_scripted_responses_server(vec![
            ScriptedResponse::InitialSelection,
            ScriptedResponse::Response(200, text_response(previous_answer, "goal-first")),
            if continuing {
                ScriptedResponse::ContinueSelection { usage: None }
            } else {
                ScriptedResponse::NewSelection
            },
            ScriptedResponse::Response(200, text_response("GOAL-RELATION-DONE", "goal-second")),
        ]);
        let config = write_context_window_config(root.path(), &server.base_url, 272_000);
        let output = run_chat(
            root.path(),
            &config,
            &format!("{first_input}\n{next_input}\n/exit\n"),
            &["--agent", agent.to_str().unwrap()],
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        selector_candidates(&f2a_request_body(
            &server.requests,
            "initial goal selection",
        ));
        let first = f2a_request_body(&server.requests, "first goal");
        let candidates =
            selector_candidates(&f2a_request_body(&server.requests, "goal relationship"));
        assert_eq!(candidates["input"]["text"], next_input);
        assert_eq!(candidates["tasks"].as_array().unwrap().len(), 1);
        let second = f2a_request_body(&server.requests, "second goal");
        assert!(server.requests.try_recv().is_err());
        server.join();
        assert!(first["input"].to_string().contains(first_input));
        assert_eq!(second["input"].to_string().matches(next_input).count(), 1);
        if continuing {
            if second
                .get("previous_response_id")
                .is_none_or(Value::is_null)
            {
                assert!(second["input"].to_string().contains("SOURCE-CRITERION-17"));
                assert!(second["input"].to_string().contains(previous_answer));
            } else {
                assert_eq!(second["previous_response_id"], "goal-first");
                let events = session_events(root.path(), &session_id(root.path()));
                let accepted = events
                    .iter()
                    .filter(|event| event["event_type"] == "session.model_delivery_committed")
                    .map(|event| &event["payload"])
                    .collect::<Vec<_>>();
                assert_eq!(accepted.len(), 2);
                assert!(accepted[0]["previous_accepted_audit"].is_null());
                assert_eq!(
                    accepted[1]["previous_accepted_audit"],
                    accepted[0]["audit_event_id"]
                );
                assert_eq!(
                    accepted[1]["provider_session"],
                    accepted[0]["provider_session"]
                );
                let first_output = events
                    .iter()
                    .find(|event| event["event_id"] == accepted[0]["history_cutoff_event_id"])
                    .unwrap();
                assert!(
                    first_output["payload"]
                        .to_string()
                        .contains(previous_answer)
                );
            }
        } else {
            assert!(
                second
                    .get("previous_response_id")
                    .is_none_or(Value::is_null)
            );
            assert!(!second.to_string().contains("SOURCE-CRITERION-17"));
            assert!(!second.to_string().contains("GOAL-SPECIFIC-OBSERVATION-271"));
        }
        let project = Project::discover(root.path()).unwrap();
        let state = reduced_session(&project, &session_id(root.path()));
        assert_eq!(state.request_audits().len(), 4);
        let events = session_events(root.path(), &session_id(root.path()));
        let samples = events
            .iter()
            .filter(|event| event["event_type"] == "session.task_selection_sample_recorded")
            .collect::<Vec<_>>();
        assert_eq!(samples.len(), 2);
        for (sample, audit_index) in samples.iter().zip([0, 2]) {
            assert_eq!(
                sample["payload"]["audit_event_id"],
                state.request_audits()[audit_index]
                    .audit_event_id()
                    .as_str()
            );
        }
        assert_eq!(state.execution_receipt_count(), 0);
        assert_eq!(state.history().items().len(), 4);
        assert!(
            serde_json::to_string(state.history().items())
                .unwrap()
                .contains(previous_answer)
        );
        assert_audits_reconstruct(root.path(), &state);
    }
}

pub(super) fn selector_candidates(wire: &Value) -> Value {
    fn collect<'a>(value: &'a Value, candidates: &mut Vec<&'a str>) {
        match value {
            Value::String(text) => {
                if let Some(json) = text.strip_prefix("TASK_CONTEXT_CANDIDATES_V1\n") {
                    candidates.push(json);
                }
            }
            Value::Array(items) => {
                for item in items {
                    collect(item, candidates);
                }
            }
            Value::Object(fields) => {
                for field in fields.values() {
                    collect(field, candidates);
                }
            }
            _ => {}
        }
    }
    assert!(wire.get("previous_response_id").is_none_or(Value::is_null));
    assert!(
        wire.get("tools")
            .is_none_or(|tools| tools.as_array().unwrap().is_empty())
    );
    let mut candidates = Vec::new();
    collect(&wire["input"], &mut candidates);
    assert_eq!(
        candidates.len(),
        1,
        "one explicitly identified selector request"
    );
    let value: Value = serde_json::from_str(candidates[0]).unwrap();
    assert!(
        value["input"]["ref"]
            .as_str()
            .is_some_and(|id| !id.is_empty())
    );
    assert!(value["tasks"].is_array());
    assert!(value["context"].is_array());
    value
}

fn task_reference(candidates: &Value, goal_marker: &str) -> String {
    let tasks = candidates["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|task| task["goal"].as_str().unwrap().contains(goal_marker))
        .collect::<Vec<_>>();
    assert_eq!(
        tasks.len(),
        1,
        "one original task for {goal_marker}: {tasks:?}"
    );
    assert!(tasks[0]["source_ref"].as_str().is_some());
    tasks[0]["ref"].as_str().unwrap().to_owned()
}

fn selected_context(candidates: &Value, task_refs: &[String]) -> Vec<String> {
    candidates["context"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|unit| {
            unit["task_ref"]
                .as_str()
                .is_some_and(|task| task_refs.iter().any(|id| id == task))
        })
        .map(|unit| {
            assert!(unit["excerpt"].as_str().is_some());
            unit["ref"].as_str().unwrap().to_owned()
        })
        .collect()
}

/// Existing multi-input fixtures declare every added selector request at its
/// exact expected position. A normal response can never implicitly become a
/// selection, and an unexpected selector fails the fixture's request count.
pub(super) enum ScriptedResponse {
    Response(u16, String),
    TruncatedResponse(String),
    InitialSelection,
    NewSelection,
    ContinueSelection { usage: Option<Value> },
    ContinueSelectionWithReasoning,
    SelectionDecision(fn(&Value) -> Value),
}

pub(super) fn initial_selection_response(wire: &Value, response_id: &str) -> String {
    let candidates = selector_candidates(wire);
    assert!(candidates["tasks"].as_array().unwrap().is_empty());
    let required = candidates["context"]
        .as_array()
        .unwrap()
        .iter()
        .map(|unit| unit["ref"].as_str().unwrap())
        .collect::<Vec<_>>();
    text_response(
        &json!({
            "relation":"new",
            "focus":{"kind":"new_from_input","input_ref":candidates["input"]["ref"]},
            "related_task_refs":[],"required_context_refs":required,
            "unresolved_references":[]
        })
        .to_string(),
        response_id,
    )
}

pub(super) fn continue_selection_response(
    wire: &Value,
    response_id: &str,
    usage: Option<Value>,
) -> String {
    let candidates = selector_candidates(wire);
    let tasks = candidates["tasks"].as_array().unwrap();
    assert_eq!(
        tasks.len(),
        1,
        "this explicit fixture continues one known task; ambiguous tasks require a different fixture"
    );
    let selected = candidates["context"]
        .as_array()
        .unwrap()
        .iter()
        .map(|unit| unit["ref"].as_str().unwrap())
        .collect::<Vec<_>>();
    let decision = json!({
        "relation":"continue","focus":{"kind":"existing","task_ref":tasks[0]["ref"]},
        "related_task_refs":[],"required_context_refs":selected,
        "unresolved_references":[]
    });
    let mut completed = json!({"type":"response.completed","response":{"id":response_id}});
    if let Some(usage) = usage {
        completed["response"]["usage"] = usage;
    }
    format!(
        "data: {}\n\ndata: {completed}\n\n",
        json!({"type":"response.output_text.delta","delta":decision.to_string()})
    )
}

pub(super) fn new_selection_response(wire: &Value, response_id: &str) -> String {
    let candidates = selector_candidates(wire);
    text_response(&json!({
        "relation":"new", "focus":{"kind":"new_from_input","input_ref":candidates["input"]["ref"]},
        "related_task_refs":[],"required_context_refs":[],"unresolved_references":[]
    }).to_string(), response_id)
}

pub(super) fn spawn_scripted_responses_server(steps: Vec<ScriptedResponse>) -> LoopbackServer {
    spawn_scripted_responses_server_with_wait(steps, F2A_WAIT)
}

pub(super) fn spawn_scripted_responses_server_with_wait(
    steps: Vec<ScriptedResponse>,
    wait: Duration,
) -> LoopbackServer {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let (sender, requests) = mpsc::channel();
    let thread = thread::spawn(move || {
        for (index, step) in steps.into_iter().enumerate() {
            let deadline = Instant::now() + wait;
            let mut socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            Instant::now() < deadline,
                            "explicit fixture request {index} did not arrive"
                        );
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("explicit fixture accept failed: {error}"),
                }
            };
            socket.set_nonblocking(false).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let raw = read_http_request(&mut socket);
            let wire = request_body(&raw);
            let truncated = matches!(&step, ScriptedResponse::TruncatedResponse(_));
            let (status, body) = match step {
                ScriptedResponse::Response(status, body) => (status, body),
                ScriptedResponse::TruncatedResponse(body) => (200, body),
                ScriptedResponse::NewSelection => (
                    200,
                    new_selection_response(&wire, &format!("fixture-selection-{index}")),
                ),
                ScriptedResponse::InitialSelection => (
                    200,
                    initial_selection_response(&wire, &format!("initial-selection-{index}")),
                ),
                ScriptedResponse::ContinueSelection { usage } => (
                    200,
                    continue_selection_response(
                        &wire,
                        &format!("fixture-selection-{index}"),
                        usage,
                    ),
                ),
                ScriptedResponse::ContinueSelectionWithReasoning => (
                    200,
                    format!(
                        "data: {}\n\ndata: {}\n\n{}",
                        json!({"type":"response.reasoning_text.delta","delta":"SELECTOR_REASONING_DELTA"}),
                        json!({"type":"response.output_item.done","item":{"type":"reasoning","content":[{"type":"reasoning_text","text":"SELECTOR_REASONING_ITEM"}]}}),
                        continue_selection_response(
                            &wire,
                            &format!("fixture-selection-{index}"),
                            None
                        )
                    ),
                ),
                ScriptedResponse::SelectionDecision(decide) => {
                    let candidates = selector_candidates(&wire);
                    (
                        200,
                        text_response(
                            &decide(&candidates).to_string(),
                            &format!("fixture-selection-{index}"),
                        ),
                    )
                }
            };
            sender.send(raw).unwrap();
            let reason = if status == 200 { "OK" } else { "Error" };
            write!(socket, "HTTP/1.1 {status} {reason}\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len() + usize::from(truncated)).unwrap();
            socket.flush().unwrap();
        }
    });
    LoopbackServer {
        base_url: format!("http://{address}/v1"),
        requests,
        thread,
    }
}

fn spawn_task_relation_server() -> LoopbackServer {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let (sender, requests) = mpsc::channel();
    let thread = thread::spawn(move || {
        for request_index in 0..10 {
            let deadline = Instant::now() + F2A_WAIT;
            let mut socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            Instant::now() < deadline,
                            "task-context request {request_index} did not arrive"
                        );
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("task-context fixture accept failed: {error}"),
                }
            };
            socket.set_nonblocking(false).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let raw = read_http_request(&mut socket);
            let wire = request_body(&raw);
            sender.send(raw).unwrap();
            let response = if request_index == 0 {
                initial_selection_response(&wire, "task-initial-selection")
            } else {
                let index = request_index - 1;
                if index % 2 == 0 {
                    let (input, answer) = match index {
                        0 => (TASK_A, A_RESULT),
                        2 => (TASK_B, B_RESULT),
                        4 => (FOLLOW_B, "B_FOLLOW_RESULT: B_LIMIT_23 retained."),
                        6 => (RETURN_A, "A_RETURN_RESULT: A_LIMIT_17 retained."),
                        8 => (COMPOSE, "COMPOSE_RESULT: both synthetic sources compared."),
                        _ => unreachable!(),
                    };
                    assert!(
                        json_contains_text(&wire["input"], input),
                        "main request {index} retains its exact current input"
                    );
                    assert!(
                        wire["tools"]
                            .as_array()
                            .is_some_and(|tools| !tools.is_empty())
                    );
                    text_response(answer, &format!("task-main-{index}"))
                } else {
                    let candidates = selector_candidates(&wire);
                    let expected_input = match index {
                        1 => TASK_B,
                        3 => FOLLOW_B,
                        5 => RETURN_A,
                        7 => COMPOSE,
                        _ => unreachable!(),
                    };
                    assert_eq!(candidates["input"]["text"], expected_input);
                    let (relation, focus, related, required) = match index {
                        1 => (
                            "new",
                            json!({"kind":"new_from_input","input_ref":candidates["input"]["ref"]}),
                            Vec::new(),
                            Vec::new(),
                        ),
                        3 | 5 => {
                            let task = task_reference(
                                &candidates,
                                if index == 3 {
                                    "TASK-BETA"
                                } else {
                                    "TASK-ALPHA"
                                },
                            );
                            let required =
                                selected_context(&candidates, std::slice::from_ref(&task));
                            assert!(
                                !required.is_empty(),
                                "continuation/return must expose actual task sources"
                            );
                            (
                                if index == 3 { "continue" } else { "return" },
                                json!({"kind":"existing","task_ref":task}),
                                Vec::new(),
                                required,
                            )
                        }
                        7 => {
                            let related = vec![
                                task_reference(&candidates, "TASK-ALPHA"),
                                task_reference(&candidates, "TASK-BETA"),
                            ];
                            let required = selected_context(&candidates, &related);
                            assert!(!required.is_empty());
                            (
                                "compose",
                                json!({"kind":"new_from_input","input_ref":candidates["input"]["ref"]}),
                                related,
                                required,
                            )
                        }
                        _ => unreachable!(),
                    };
                    text_response(
                        &json!({
                            "relation":relation,
                            "focus":focus,
                            "related_task_refs":related,
                            "required_context_refs":required,
                            "unresolved_references":[]
                        })
                        .to_string(),
                        &format!("task-selector-{index}"),
                    )
                }
            };
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).unwrap();
            socket.flush().unwrap();
        }
    });
    LoopbackServer {
        base_url: format!("http://{address}/v1"),
        requests,
        thread,
    }
}

#[test]
fn task_context_switch_follow_return_and_compose_use_exact_sources_and_actual_epochs() {
    let root = TemporaryDirectory::new("task-context-relations");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let server = spawn_task_relation_server();
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    fs::write(
        &config,
        format!(
            "{}\n[runtime]\nmodel_request_limit_per_turn = 0\n",
            fs::read_to_string(&config).unwrap()
        ),
    )
    .unwrap();
    let output = run_chat(
        root.path(),
        &config,
        &format!("{TASK_A}\n{TASK_B}\n{FOLLOW_B}\n{RETURN_A}\n{COMPOSE}\n/exit\n"),
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    selector_candidates(&f2a_request_body(
        &server.requests,
        "task relation initial selection",
    ));
    let wires = (0..9)
        .map(|_| f2a_request_body(&server.requests, "automatic task relation"))
        .collect::<Vec<_>>();
    assert!(server.requests.try_recv().is_err());
    server.join();
    for index in [0, 2, 6, 8] {
        assert!(
            wires[index]
                .get("previous_response_id")
                .is_none_or(Value::is_null),
            "main request {index} starts a fresh accepted chain"
        );
    }
    assert_eq!(
        wires[4]["previous_response_id"], "task-main-2",
        "isolated selection must not replace the same-task accepted predecessor"
    );
    for marker in ["TASK-ALPHA", "A_LIMIT_17", "A_ONLY_RESULT"] {
        assert!(
            !wires[2].to_string().contains(marker),
            "unrelated main request leaked {marker}"
        );
        assert!(
            !wires[4].to_string().contains(marker),
            "follow-up leaked {marker}"
        );
    }
    for marker in [
        "TASK-BETA",
        "B_LIMIT_23",
        "B_ONLY_RESULT",
        "B_FOLLOW_RESULT",
    ] {
        assert!(
            !wires[6].to_string().contains(marker),
            "return to A leaked {marker}"
        );
    }
    assert!(json_contains_text(&wires[6]["input"], A_RESULT));
    assert!(json_contains_text(&wires[8]["input"], A_RESULT));
    assert!(json_contains_text(&wires[8]["input"], B_RESULT));
    let project = Project::discover(root.path()).unwrap();
    let session = session_id(root.path());
    let state = reduced_session(&project, &session);
    assert_eq!(state.request_audits().len(), 10);
    assert_eq!(state.execution_receipt_count(), 0);
    let blobs = root.path().join(".echidna/artifacts/blobs/sha256");
    for (index, excluded) in [(2, "A_ONLY_RESULT"), (6, "B_ONLY_RESULT")] {
        let audit = &state.request_audits()[index + 1];
        assert_eq!(audit.full_logical_input(), audit.delivered_input());
        let reconstructed = audit
            .reconstruct(|id| fs::read(blobs.join(id.as_str())))
            .unwrap();
        assert!(
            !serde_json::to_string(&reconstructed)
                .unwrap()
                .contains(excluded),
            "the logical/audited Full request must exclude the old body as well as the HTTP wire"
        );
    }
    let history = state.history().items();
    assert_eq!(
        history.len(),
        10,
        "all five original inputs and actual answers remain, without selector prose"
    );
    for (index, input) in [TASK_A, TASK_B, FOLLOW_B, RETURN_A, COMPOSE]
        .iter()
        .enumerate()
    {
        assert_eq!(
            history[index * 2],
            echidna::domain::model::ContentItem::user_text(*input)
        );
    }
    let events = session_events(root.path(), &session);
    let original_answer = serde_json::to_value(
        echidna::domain::model::ContentItem::assistant_text(A_RESULT),
    )
    .unwrap();
    let original_event_id = &events
        .iter()
        .find(|event| {
            event["event_type"] == "session.history_item_recorded"
                && event["payload"]["item"] == original_answer
        })
        .unwrap()["event_id"];
    let returned_audit = &state.request_audits()[7];
    let returned_ir = returned_audit
        .reconstruct(|id| fs::read(blobs.join(id.as_str())))
        .unwrap();
    let original_position = returned_ir
        .content_items
        .iter()
        .position(|item| *item == echidna::domain::model::ContentItem::assistant_text(A_RESULT))
        .unwrap();
    let returned_map =
        serde_json::to_value(returned_audit.context_delivery_map().unwrap()).unwrap();
    let source_map = returned_map["content_items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["final_position"] == original_position)
        .unwrap();
    assert!(
        source_map["provenance"]["references"]["source_event_refs"]
            .as_array()
            .unwrap()
            .contains(original_event_id),
        "a returned noncontiguous history item keeps its original EventId rather than an invented current ordinal"
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event["event_type"] == "session.task_context_selected")
            .count(),
        5
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event["event_type"] == "session.task_selection_sample_recorded")
            .count(),
        5
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event["event_type"] == "session.model_delivery_committed")
            .count(),
        5
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("COMPOSE_RESULT"));
    assert!(!stdout.contains("new_from_input"));
    assert!(!stdout.contains("TASK_CONTEXT_CANDIDATES_V1"));
    // Resume validates the saved sources; merely opening it must not resubmit
    // old tasks, selectors or tools to the now-closed fixture Provider.
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
    let restored = reduced_session(&project, &session);
    assert_eq!(restored.history().items(), history);
    assert_eq!(restored.request_audits().len(), 10);
    let captured = events
        .iter()
        .rev()
        .find(|event| event["event_type"] == "session.task_context_selected")
        .unwrap()["payload"]["capture_blob"]
        .as_str()
        .unwrap();
    let source_path = blobs.join(captured);
    let source_bytes = fs::read(&source_path).unwrap();
    let before_corruption = session_events(root.path(), &session);
    fs::write(&source_path, b"damaged frozen task source").unwrap();
    let corrupted = run_chat(root.path(), &config, "/exit\n", &["--resume", "latest"]);
    assert!(
        !corrupted.status.success(),
        "corrupt task source cannot be accepted from the current files or an actual-request template"
    );
    assert_eq!(session_events(root.path(), &session), before_corruption);
    fs::write(&source_path, source_bytes).unwrap();
}

#[test]
fn task_context_reserves_main_admission_before_selection_and_each_transport_retry() {
    for limit in [1, 2] {
        let root = TemporaryDirectory::new("task-context-budget-reserve");
        initialize_project(root.path());
        let agent = definition(root.path(), None);
        let mut responses = Vec::new();
        if limit == 2 {
            let partial = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"UNCOMMITTED-SELECTOR\"}\n\n";
            responses.push(ScriptedResponse::InitialSelection);
            responses.push(ScriptedResponse::Response(
                200,
                text_response(A_RESULT, "budget-completed-a"),
            ));
            responses.push(ScriptedResponse::TruncatedResponse(partial.to_owned()));
        }
        let server = spawn_scripted_responses_server(responses);
        let config = write_context_window_config(root.path(), &server.base_url, 272_000);
        fs::write(
            &config,
            format!(
                "{}\n[runtime]\nmodel_request_limit_per_turn = {limit}\n",
                fs::read_to_string(&config).unwrap()
            ),
        )
        .unwrap();
        let output = run_chat(
            root.path(),
            &config,
            &format!("{TASK_A}\n{TASK_B}\nMUST-NOT-ACCEPT-AFTER-STOP\n/context\n/exit\n"),
            &["--agent", agent.to_str().unwrap()],
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !output.status.success(),
            "exiting with an unresolved stopped Turn preserves the existing nonzero exit: {stdout}\n{stderr}"
        );
        assert!(stdout.contains("Task input not accepted or saved"));
        assert!(stderr.contains("unresolved stopped Turn"));
        assert!(
            stdout.contains("[turn stopped]") || stderr.contains("[turn stopped]"),
            "the unavailable selection budget must remain visible: {stdout}\n{stderr}"
        );
        assert!(!stdout.contains("UNCOMMITTED-SELECTOR"));
        let expected_requests = if limit == 1 { 0 } else { 3 };
        let wires = (0..expected_requests)
            .map(|_| f2a_request_body(&server.requests, "selection admission reserve"))
            .collect::<Vec<_>>();
        if limit == 2 {
            assert_eq!(selector_candidates(&wires[0])["input"]["text"], TASK_A);
            assert_eq!(selector_candidates(&wires[2])["input"]["text"], TASK_B);
        }
        assert!(server.requests.try_recv().is_err());
        server.join();
        let session = session_id(root.path());
        let state = reduced_session(&Project::discover(root.path()).unwrap(), &session);
        assert_eq!(state.request_audits().len(), expected_requests);
        assert_eq!(state.execution_receipt_count(), 0);
        let mut expected_history = vec![echidna::domain::model::ContentItem::user_text(TASK_A)];
        if limit == 2 {
            expected_history.push(echidna::domain::model::ContentItem::assistant_text(
                A_RESULT,
            ));
            expected_history.push(echidna::domain::model::ContentItem::user_text(TASK_B));
        }
        assert_eq!(state.history().items(), &expected_history);
        let events = session_events(root.path(), &session);
        for event_type in [
            "session.task_context_selected",
            "session.model_delivery_committed",
        ] {
            assert_eq!(
                events
                    .iter()
                    .filter(|event| event["event_type"] == event_type)
                    .count(),
                usize::from(limit == 2),
                "a failed preparation must not replace the accepted A epoch"
            );
        }
        assert_eq!(
            events
                .iter()
                .filter(|event| event["event_type"] == "session.task_selection_sample_recorded")
                .count(),
            usize::from(limit == 2)
        );
    }
}

#[test]
fn task_context_two_or_unlimited_admissions_preserve_selection_usage_and_main_delivery() {
    for limit in [2, 0] {
        let root = TemporaryDirectory::new("task-context-two-admissions");
        initialize_project(root.path());
        let agent = definition(root.path(), None);
        let server = spawn_scripted_responses_server(vec![
            ScriptedResponse::InitialSelection,
            ScriptedResponse::Response(200, text_response(A_RESULT, "two-admissions-a")),
            ScriptedResponse::ContinueSelection {
                usage: Some(
                    json!({"input_tokens":30,"output_tokens":7,"input_tokens_details":{"cached_tokens":20},"output_tokens_details":{"reasoning_tokens":3}}),
                ),
            },
            ScriptedResponse::Response(
                200,
                text_response("ALPHA-FOLLOW-COMPLETE", "two-admissions-follow"),
            ),
        ]);
        let config = write_context_window_config(root.path(), &server.base_url, 272_000);
        fs::write(
            &config,
            format!(
                "{}\n[runtime]\nmodel_request_limit_per_turn = {limit}\n",
                fs::read_to_string(&config).unwrap()
            ),
        )
        .unwrap();
        let follow = "它的量能呢？继续保留 A_LIMIT_17。";
        let output = run_chat(
            root.path(),
            &config,
            &format!("{TASK_A}\n{follow}\n/exit\n"),
            &["--agent", agent.to_str().unwrap()],
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{stdout}\n{stderr}");
        assert!(stdout.contains("ALPHA-FOLLOW-COMPLETE"));
        if limit == 0 {
            assert!(
                stdout
                    .contains("admitted=2 primary=2 vision=0 remaining=unlimited limit=unlimited"),
                "{stdout}"
            );
        }
        assert!(!stdout.contains("[turn stopped]") && !stderr.contains("[turn stopped]"));
        let wires = (0..4)
            .map(|_| f2a_request_body(&server.requests, "exact two-admission turn"))
            .collect::<Vec<_>>();
        assert!(server.requests.try_recv().is_err());
        server.join();
        assert_eq!(selector_candidates(&wires[0])["input"]["text"], TASK_A);
        assert_eq!(selector_candidates(&wires[2])["input"]["text"], follow);
        assert_eq!(wires[3]["previous_response_id"], "two-admissions-a");
        assert!(json_contains_text(&wires[3]["input"], follow));
        let session = session_id(root.path());
        let state = reduced_session(&Project::discover(root.path()).unwrap(), &session);
        assert_eq!(state.request_audits().len(), 4);
        assert_eq!(state.execution_receipt_count(), 0);
        assert_eq!(state.history().items().len(), 4);
        let events = session_events(root.path(), &session);
        let usages = events
            .iter()
            .filter(|event| event["event_type"] == "session.task_selection_usage_recorded")
            .collect::<Vec<_>>();
        assert_eq!(
            usages.len(),
            1,
            "the selector's actual usage is durable even though it is not a Primary delivery"
        );
        assert_eq!(
            usages[0]["payload"]["usage"],
            json!({"input_tokens":30,"output_tokens":7,"cached_input_tokens":20,"reasoning_output_tokens":3})
        );
        let selected_sample = events
            .iter()
            .rfind(|event| event["event_type"] == "session.task_selection_sample_recorded")
            .unwrap();
        assert_eq!(
            usages[0]["payload"]["request_audit_event_id"],
            selected_sample["payload"]["audit_event_id"]
        );
        assert_eq!(
            usages[0]["payload"]["provider_session"],
            selected_sample["payload"]["provider_session"]
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| event["event_type"] == "session.task_selection_sample_recorded")
                .count(),
            2
        );
        let initial_sample = events
            .iter()
            .find(|event| event["event_type"] == "session.task_selection_sample_recorded")
            .unwrap();
        assert_eq!(
            initial_sample["payload"]["audit_event_id"],
            state.request_audits()[0].audit_event_id().as_str()
        );
        assert_eq!(
            selected_sample["payload"]["audit_event_id"],
            state.request_audits()[2].audit_event_id().as_str()
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| event["event_type"] == "session.model_delivery_committed")
                .count(),
            2
        );
    }
}

#[test]
fn task_context_read_finds_an_excluded_source_without_crossing_agent_scope_or_resetting_epoch() {
    let root = TemporaryDirectory::new("task-context-read-scope");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let other_memory = root
        .path()
        .join(".echidna/memory/agents/another-agent/core.md");
    fs::create_dir_all(other_memory.parent().unwrap()).unwrap();
    fs::write(
        &other_memory,
        "ZXQJ907531VPK: synthetic separate identity.\n",
    )
    .unwrap();
    let server = spawn_scripted_responses_server(vec![
        ScriptedResponse::InitialSelection,
        ScriptedResponse::Response(200, text_response(A_RESULT, "readback-a")),
        ScriptedResponse::NewSelection,
        ScriptedResponse::Response(
            200,
            m4_long_flow_tool_response(
                "read-prior-a",
                "context_read",
                json!({"operation":"search","query":"A_ONLY_RESULT","max_bytes":2048}),
                "readback-b-first",
            ),
        ),
        ScriptedResponse::Response(
            200,
            m4_long_flow_tool_response(
                "read-other-scope",
                "context_read",
                json!({"operation":"search","query":"ZXQJ907531VPK","max_bytes":2048}),
                "readback-b-second",
            ),
        ),
        ScriptedResponse::Response(200, text_response("SOURCE-READ-COMPLETE", "readback-final")),
    ]);
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        &format!("{TASK_A}\n{TASK_B}\n/exit\n"),
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let wires = (0..6)
        .map(|_| f2a_request_body(&server.requests, "scoped context read"))
        .collect::<Vec<_>>();
    assert!(server.requests.try_recv().is_err());
    server.join();
    assert!(!wires[3].to_string().contains("A_ONLY_RESULT"));
    assert!(
        wires[3]
            .get("previous_response_id")
            .is_none_or(Value::is_null)
    );
    assert_eq!(wires[4]["previous_response_id"], "readback-b-first");
    assert_eq!(wires[5]["previous_response_id"], "readback-b-second");
    let state = reduced_session(
        &Project::discover(root.path()).unwrap(),
        &session_id(root.path()),
    );
    assert_eq!(
        state.request_audits().len(),
        6,
        "readback stays within the one Tool loop without a second selector"
    );
    assert_eq!(state.execution_receipt_count(), 2);
    state.history().ensure_all_tool_calls_paired().unwrap();
    let results = state
        .history()
        .items()
        .iter()
        .filter_map(|item| match item {
            echidna::domain::model::ContentItem::ToolResult { result } => Some(result),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].call_id.as_str(), "read-prior-a");
    assert_eq!(results[0].output["status"], "found");
    let entries = results[0].output["entries"].as_array().unwrap();
    assert!(
        entries
            .iter()
            .any(|entry| entry["text"].as_str().unwrap().contains(A_RESULT))
    );
    for entry in entries {
        assert!(
            entry["reference"]
                .as_str()
                .is_some_and(|reference| !reference.is_empty())
        );
        assert!(
            entry["source_revision"]
                .as_str()
                .is_some_and(|revision| !revision.is_empty())
        );
        assert!(
            entry["scope"]
                .as_str()
                .is_some_and(|scope| !scope.is_empty())
        );
    }
    assert_eq!(results[1].call_id.as_str(), "read-other-scope");
    assert_eq!(
        results[1].output["status"], "not_found",
        "unexpected scoped sources: {}",
        results[1].output
    );
    assert!(results[1].output["entries"].as_array().unwrap().is_empty());
    assert_eq!(
        fs::read_to_string(other_memory).unwrap(),
        "ZXQJ907531VPK: synthetic separate identity.\n"
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("SOURCE-READ-COMPLETE"));
}

const GLOBAL_RULE: &str = "GLOBAL-PREF: always express timestamps in UTC.";
const GLOBAL_SOURCE_A: &str = "GLOBAL-PREF: always express timestamps in UTC. Analyze TASK-ALPHA with LOCAL-ALPHA-ONLY evidence.";
const NEW_GLOBAL_RULE: &str = "REVISED-PREF: from now on express timestamps in UTC+08:00.";
const GLOBAL_SOURCE_D: &str =
    "REVISED-PREF: from now on express timestamps in UTC+08:00. Explain TASK-DELTA.";

fn adopt_prior_global_range(candidates: &Value) -> Value {
    let source = candidates["context"]
        .as_array()
        .unwrap()
        .iter()
        .find(|unit| unit["excerpt"] == GLOBAL_SOURCE_A)
        .unwrap();
    json!({
        "relation":"new","focus":{"kind":"new_from_input","input_ref":candidates["input"]["ref"]},
        "related_task_refs":[],"required_context_refs":[],"unresolved_references":[],
        "constraint_updates":[{"source_ref":source["ref"],"start_byte":0,"end_byte":GLOBAL_RULE.len(),"scope":{"kind":"global"},"supersedes":null}]
    })
}

fn supersede_global_range(candidates: &Value) -> Value {
    assert_eq!(candidates["input"]["text"], GLOBAL_SOURCE_D);
    let prior = candidates["context"]
        .as_array()
        .unwrap()
        .iter()
        .find(|unit| unit["excerpt"] == GLOBAL_RULE && unit["constraint_range"].is_object())
        .unwrap();
    json!({
        "relation":"new","focus":{"kind":"new_from_input","input_ref":candidates["input"]["ref"]},
        "related_task_refs":[],"required_context_refs":[],"unresolved_references":[],
        "constraint_updates":[{"source_ref":candidates["input"]["ref"],"start_byte":0,"end_byte":NEW_GLOBAL_RULE.len(),"scope":{"kind":"global"},"supersedes":prior["ref"]}]
    })
}

fn global_constraint_to_current_task(candidates: &Value) -> Value {
    let prior = candidates["context"]
        .as_array()
        .unwrap()
        .iter()
        .find(|unit| unit["excerpt"] == GLOBAL_RULE && unit["constraint_range"].is_object())
        .unwrap();
    json!({"relation":"new", "focus":{"kind":"new_from_input", "input_ref":candidates["input"]["ref"]},
        "related_task_refs":[], "required_context_refs":[], "unresolved_references":[],
        "constraint_updates":[{"source_ref":candidates["input"]["ref"], "start_byte":0,
            "end_byte":GLOBAL_TO_LOCAL.len(), "scope":{"kind":"current_task"}, "supersedes":prior["ref"]}]})
}

const GLOBAL_TO_LOCAL: &str =
    "Correct the earlier global timestamp rule: only for this task use UTC+08:00.";

#[test]
fn task_context_global_constraint_can_be_explicitly_corrected_to_current_task() {
    let root = TemporaryDirectory::new("task-context-global-scope-correction");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let server = spawn_scripted_responses_server(vec![
        ScriptedResponse::InitialSelection,
        ScriptedResponse::Response(200, text_response(A_RESULT, "scope-a")),
        ScriptedResponse::SelectionDecision(adopt_prior_global_range),
        ScriptedResponse::Response(200, text_response(B_RESULT, "scope-b")),
        ScriptedResponse::SelectionDecision(global_constraint_to_current_task),
        ScriptedResponse::Response(200, text_response("Scope corrected.", "scope-c")),
        ScriptedResponse::NewSelection,
        ScriptedResponse::Response(200, text_response("Separate task.", "scope-d")),
    ]);
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        &format!("{GLOBAL_SOURCE_A}\n{TASK_B}\n{GLOBAL_TO_LOCAL}\nA separate task.\n/exit\n"),
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let wires = (0..8)
        .map(|_| f2a_request_body(&server.requests, "global scope correction"))
        .collect::<Vec<_>>();
    assert!(server.requests.try_recv().is_err());
    server.join();
    let events = session_events(root.path(), &session_id(root.path()));
    assert!(json_contains_text(&wires[5]["input"], GLOBAL_TO_LOCAL));
    assert!(!json_contains_text(&wires[5]["input"], GLOBAL_RULE));
    assert!(!json_contains_text(&wires[7]["input"], GLOBAL_RULE));
    assert!(!json_contains_text(&wires[7]["input"], GLOBAL_TO_LOCAL));
    let revision = events
        .iter()
        .flat_map(|event| {
            event["payload"]["constraint_revisions"]
                .as_array()
                .into_iter()
                .flatten()
        })
        .find(|revision| revision["supersedes"].is_string())
        .unwrap();
    assert_eq!(revision["scope"]["kind"], "task");
}

#[test]
fn task_context_global_constraint_keeps_only_its_exact_range_and_requires_explicit_supersession() {
    let root = TemporaryDirectory::new("task-context-global-range");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let server = spawn_scripted_responses_server(vec![
        ScriptedResponse::InitialSelection,
        ScriptedResponse::Response(200, text_response(A_RESULT, "global-a")),
        ScriptedResponse::SelectionDecision(adopt_prior_global_range),
        ScriptedResponse::Response(200, text_response(B_RESULT, "global-b")),
        ScriptedResponse::NewSelection,
        ScriptedResponse::Response(200, text_response("GAMMA-RESULT", "global-c")),
        ScriptedResponse::SelectionDecision(supersede_global_range),
        ScriptedResponse::Response(200, text_response("DELTA-RESULT", "global-d")),
    ]);
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let third = "Explain a separate arithmetic example, TASK-GAMMA.";
    let output = run_chat(
        root.path(),
        &config,
        &format!("{GLOBAL_SOURCE_A}\n{TASK_B}\n{third}\n{GLOBAL_SOURCE_D}\n/exit\n"),
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let wires = (0..8)
        .map(|_| f2a_request_body(&server.requests, "global constraint range"))
        .collect::<Vec<_>>();
    assert!(server.requests.try_recv().is_err());
    server.join();
    for index in [3, 5] {
        assert!(
            wires[index].to_string().contains(GLOBAL_RULE),
            "known global constraints survive without reselection or query-word overlap"
        );
        for excluded in ["LOCAL-ALPHA-ONLY", "TASK-ALPHA", "A_ONLY_RESULT"] {
            assert!(
                !wires[index].to_string().contains(excluded),
                "global source range dragged in local task body: {excluded}"
            );
        }
    }
    assert!(!wires[5].to_string().contains("B_ONLY_RESULT"));
    assert!(wires[7].to_string().contains(NEW_GLOBAL_RULE));
    assert!(!wires[7].to_string().contains(GLOBAL_RULE));
    assert!(
        wires[7]
            .get("previous_response_id")
            .is_none_or(Value::is_null)
    );
    let session = session_id(root.path());
    let state = reduced_session(&Project::discover(root.path()).unwrap(), &session);
    assert_eq!(
        state.history().items().len(),
        8,
        "derived constraint ranges never become fabricated User History"
    );
    assert_eq!(state.request_audits().len(), 8);
    let events = session_events(root.path(), &session);
    let samples = events
        .iter()
        .filter(|event| event["event_type"] == "session.task_selection_sample_recorded")
        .collect::<Vec<_>>();
    assert_eq!(samples.len(), 4);
    for (sample, audit_index) in samples.iter().zip([0, 2, 4, 6]) {
        assert_eq!(
            sample["payload"]["audit_event_id"],
            state.request_audits()[audit_index]
                .audit_event_id()
                .as_str()
        );
    }
    let original_a = serde_json::to_value(echidna::domain::model::ContentItem::user_text(
        GLOBAL_SOURCE_A,
    ))
    .unwrap();
    let source_a = &events
        .iter()
        .find(|event| {
            event["event_type"] == "session.history_item_recorded"
                && event["payload"]["item"] == original_a
        })
        .unwrap()["event_id"];
    let selections = events
        .iter()
        .filter(|event| event["event_type"] == "session.task_context_selected")
        .collect::<Vec<_>>();
    assert_eq!(selections.len(), 4);
    let protected = selections[2]["payload"]["selected_constraints"]
        .as_array()
        .unwrap();
    assert_eq!(protected.len(), 1);
    assert_eq!(protected[0]["source_event_id"], *source_a);
    assert_eq!(protected[0]["start_byte"], 0);
    assert_eq!(protected[0]["end_byte"], GLOBAL_RULE.len());
    assert!(
        serde_json::to_string(state.request_audits()[4].context_delivery_map().unwrap())
            .unwrap()
            .contains(source_a.as_str().unwrap())
    );
    assert!(
        selections[3]["payload"]["selected_constraints"]
            .as_array()
            .unwrap()
            .iter()
            .all(|range| range["source_event_id"] != *source_a)
    );
}

const LOCAL_RULE: &str = "LOCAL-LIMIT: 17 units for TASK-ALPHA only.";
const LOCAL_CORRECTION: &str = "LOCAL-LIMIT: 19 units for TASK-ALPHA only.";

fn select_local_constraint(candidates: &Value) -> Value {
    let task = task_reference(candidates, "TASK-ALPHA");
    let source = candidates["context"]
        .as_array()
        .unwrap()
        .iter()
        .find(|unit| unit["excerpt"] == LOCAL_RULE)
        .unwrap();
    json!({"relation":"continue","focus":{"kind":"existing","task_ref":task},
        "related_task_refs":[],"required_context_refs":[],"unresolved_references":[],
        "constraint_updates":[{"source_ref":source["ref"],"start_byte":0,"end_byte":LOCAL_RULE.len(),"scope":{"kind":"task","task_ref":task},"supersedes":null}]})
}

fn correct_local_constraint(candidates: &Value) -> Value {
    let task = task_reference(candidates, "TASK-ALPHA");
    let prior = candidates["context"]
        .as_array()
        .unwrap()
        .iter()
        .find(|unit| unit["excerpt"] == LOCAL_RULE && unit["constraint_range"].is_object())
        .unwrap();
    json!({"relation":"continue","focus":{"kind":"existing","task_ref":task},
        "related_task_refs":[],"required_context_refs":[],"unresolved_references":[],
        "constraint_updates":[{"source_ref":candidates["input"]["ref"],"start_byte":0,"end_byte":LOCAL_CORRECTION.len(),"scope":{"kind":"task","task_ref":task},"supersedes":prior["ref"]}]})
}

fn return_with_local_constraint(candidates: &Value) -> Value {
    json!({"relation":"return","focus":{"kind":"existing","task_ref":task_reference(candidates,"TASK-ALPHA")},
        "related_task_refs":[],"required_context_refs":[],"unresolved_references":[]})
}

#[test]
fn task_context_local_correction_supersedes_only_its_task_and_survives_return() {
    let root = TemporaryDirectory::new("task-context-local-correction");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let server = spawn_scripted_responses_server(vec![
        ScriptedResponse::InitialSelection,
        ScriptedResponse::Response(200, text_response(A_RESULT, "local-a")),
        ScriptedResponse::SelectionDecision(select_local_constraint),
        ScriptedResponse::Response(200, text_response("LOCAL-CHECKED", "local-follow")),
        ScriptedResponse::SelectionDecision(correct_local_constraint),
        ScriptedResponse::Response(200, text_response("LOCAL-CORRECTED", "local-correct")),
        ScriptedResponse::NewSelection,
        ScriptedResponse::Response(200, text_response(B_RESULT, "local-b")),
        ScriptedResponse::SelectionDecision(return_with_local_constraint),
        ScriptedResponse::Response(200, text_response("RETURN-CORRECTED", "local-return")),
    ]);
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        &format!(
            "{LOCAL_RULE}\nContinue TASK-ALPHA and retain its exact local limit.\n{LOCAL_CORRECTION}\n{TASK_B}\nReturn to TASK-ALPHA using its corrected limit.\n/exit\n"
        ),
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let wires = (0..10)
        .map(|_| f2a_request_body(&server.requests, "task-local constraint correction"))
        .collect::<Vec<_>>();
    assert!(server.requests.try_recv().is_err());
    server.join();
    assert!(wires[3].to_string().contains(LOCAL_RULE));
    for index in [5, 9] {
        assert!(wires[index].to_string().contains(LOCAL_CORRECTION));
        assert!(!wires[index].to_string().contains(LOCAL_RULE));
        assert!(wires[index]["previous_response_id"].is_null());
    }
    for text in [LOCAL_RULE, LOCAL_CORRECTION, A_RESULT] {
        assert!(
            !wires[7].to_string().contains(text),
            "task-local constraint leaked into B: {text}"
        );
    }
    let session = session_id(root.path());
    let events = session_events(root.path(), &session);
    let selections = events
        .iter()
        .filter(|event| event["event_type"] == "session.task_context_selected")
        .collect::<Vec<_>>();
    let correction = &selections[2]["payload"]["constraint_revisions"][0];
    assert_eq!(correction["scope"]["kind"], "task");
    assert!(correction["supersedes"].is_string());
    assert!(
        selections[3]["payload"]["selected_constraints"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        selections[4]["payload"]["selected_constraints"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let state = reduced_session(&Project::discover(root.path()).unwrap(), &session);
    assert_eq!(state.history().items().len(), 10);
    assert_eq!(state.execution_receipt_count(), 0);
    let resumed = run_chat(root.path(), &config, "/exit\n", &["--resume", "latest"]);
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
}

const MULTI_INPUT: &str = "先分析 TASK-DELTA 的构建失败；再比较 TASK-EPSILON 的迁移方案。";

fn select_multiple_objectives(candidates: &Value) -> Value {
    assert_eq!(candidates["input"]["text"], MULTI_INPUT);
    let split = MULTI_INPUT.find('；').unwrap();
    json!({
        "relation":"new","focus":{"kind":"new_from_input","input_ref":candidates["input"]["ref"]},
        "related_task_refs":[],"required_context_refs":[],"unresolved_references":[],
        "new_objectives":[
            {"start_byte":0,"end_byte":split,"description":"TASK-DELTA build failure"},
            {"start_byte":split + '；'.len_utf8(),"end_byte":MULTI_INPUT.len(),"description":"TASK-EPSILON migration comparison"}
        ]
    })
}

#[test]
fn task_context_multiple_objectives_keep_one_exact_original_input() {
    let root = TemporaryDirectory::new("task-context-multiple-objectives");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let server = spawn_scripted_responses_server(vec![
        ScriptedResponse::InitialSelection,
        ScriptedResponse::Response(200, text_response(A_RESULT, "multi-a")),
        ScriptedResponse::SelectionDecision(select_multiple_objectives),
        ScriptedResponse::Response(
            200,
            text_response("BOTH-OBJECTIVES-ANSWERED", "multi-final"),
        ),
    ]);
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        &format!("{TASK_A}\n{MULTI_INPUT}\n/exit\n"),
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let wires = (0..4)
        .map(|_| f2a_request_body(&server.requests, "multiple objectives"))
        .collect::<Vec<_>>();
    assert!(server.requests.try_recv().is_err());
    server.join();
    assert_eq!(
        wires[3]["input"].to_string().matches(MULTI_INPUT).count(),
        1
    );
    assert!(!wires[3].to_string().contains("A_ONLY_RESULT"));
    let session = session_id(root.path());
    let state = reduced_session(&Project::discover(root.path()).unwrap(), &session);
    assert_eq!(state.history().items().len(), 4);
    assert_eq!(
        state.history().items()[2],
        echidna::domain::model::ContentItem::user_text(MULTI_INPUT)
    );
    let events = session_events(root.path(), &session);
    let selection = events
        .iter()
        .rfind(|event| event["event_type"] == "session.task_context_selected")
        .unwrap();
    let revisions = selection["payload"]["task_revisions"].as_array().unwrap();
    assert_eq!(revisions.len(), 3);
    let ranges = revisions
        .iter()
        .skip(1)
        .map(|revision| {
            let range = revision["input_range"].as_array().unwrap();
            &MULTI_INPUT[range[0].as_u64().unwrap() as usize..range[1].as_u64().unwrap() as usize]
        })
        .collect::<Vec<_>>();
    assert_eq!(
        ranges,
        vec![
            "先分析 TASK-DELTA 的构建失败",
            "再比较 TASK-EPSILON 的迁移方案。"
        ]
    );
}

fn request_reference_expansion(candidates: &Value) -> Value {
    assert!(candidates.get("expansion_references").is_none());
    json!({
        "relation":"new","focus":{"kind":"new_from_input","input_ref":candidates["input"]["ref"]},
        "related_task_refs":[],"required_context_refs":[],"unresolved_references":["A_ONLY_RESULT"]
    })
}

fn select_expanded_reference(candidates: &Value) -> Value {
    assert_eq!(candidates["expansion_references"], json!(["A_ONLY_RESULT"]));
    let source = candidates["context"]
        .as_array()
        .unwrap()
        .iter()
        .find(|unit| unit["excerpt"] == A_RESULT)
        .unwrap();
    json!({
        "relation":"new","focus":{"kind":"new_from_input","input_ref":candidates["input"]["ref"]},
        "related_task_refs":[],"required_context_refs":[source["ref"]],"unresolved_references":[]
    })
}

fn retain_explicit_gap(candidates: &Value) -> Value {
    assert_eq!(candidates["expansion_references"], json!(["A_ONLY_RESULT"]));
    json!({
        "relation":"unresolved","focus":{"kind":"new_from_input","input_ref":candidates["input"]["ref"]},
        "related_task_refs":[],"required_context_refs":[],"unresolved_references":["UNAVAILABLE-EXACT-OBSERVATION"]
    })
}

#[test]
fn task_context_expands_once_and_retains_unresolved_gaps_without_full_history_fallback() {
    for resolve in [true, false] {
        let root = TemporaryDirectory::new("task-context-expansion");
        initialize_project(root.path());
        let agent = definition(root.path(), None);
        let server = spawn_scripted_responses_server(vec![
            ScriptedResponse::InitialSelection,
            ScriptedResponse::Response(200, text_response(A_RESULT, "expand-a")),
            ScriptedResponse::SelectionDecision(request_reference_expansion),
            ScriptedResponse::SelectionDecision(if resolve {
                select_expanded_reference
            } else {
                retain_explicit_gap
            }),
            ScriptedResponse::Response(200, text_response("EXPANSION-COMPLETE", "expand-final")),
        ]);
        let config = write_context_window_config(root.path(), &server.base_url, 272_000);
        let input = "TASK-BETA: use the exact earlier observation if its source can be resolved.";
        let output = run_chat(
            root.path(),
            &config,
            &format!("{TASK_A}\n{input}\n/exit\n"),
            &["--agent", agent.to_str().unwrap()],
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let wires = (0..5)
            .map(|_| f2a_request_body(&server.requests, "bounded expansion"))
            .collect::<Vec<_>>();
        assert!(server.requests.try_recv().is_err());
        server.join();
        assert!(wires[4].to_string().contains(input));
        assert_eq!(wires[4].to_string().contains(A_RESULT), resolve);
        assert_eq!(
            wires[4]
                .to_string()
                .contains("UNAVAILABLE-EXACT-OBSERVATION"),
            !resolve
        );
        assert!(
            !wires[4].to_string().contains(TASK_A),
            "a selected fact must not import the whole originating task"
        );
        let session = session_id(root.path());
        let state = reduced_session(&Project::discover(root.path()).unwrap(), &session);
        assert_eq!(state.request_audits().len(), 5);
        assert_eq!(state.history().items().len(), 4);
        let events = session_events(root.path(), &session);
        let samples = events
            .iter()
            .filter(|event| event["event_type"] == "session.task_selection_sample_recorded")
            .collect::<Vec<_>>();
        assert_eq!(
            samples.len(),
            3,
            "an unresolved second sample must not start a third selector"
        );
        let selection = events
            .iter()
            .rfind(|event| event["event_type"] == "session.task_context_selected")
            .unwrap();
        assert_eq!(
            selection["payload"]["sample_event_id"],
            samples[2]["event_id"]
        );
    }
}

fn invalid_reference(candidates: &Value) -> Value {
    json!({
        "relation":"new","focus":{"kind":"new_from_input","input_ref":candidates["input"]["ref"]},
        "related_task_refs":[],"required_context_refs":["fabricated-source"],"unresolved_references":[]
    })
}

fn invalid_utf8_objective(candidates: &Value) -> Value {
    let mut decision = invalid_reference(candidates);
    decision["required_context_refs"] = json!([]);
    decision["new_objectives"] =
        json!([{"start_byte":1,"end_byte":2,"description":"inside a Chinese codepoint"}]);
    decision
}

fn invalid_assistant_constraint(candidates: &Value) -> Value {
    let source = candidates["context"]
        .as_array()
        .unwrap()
        .iter()
        .find(|unit| unit["excerpt"] == A_RESULT)
        .unwrap();
    let mut decision = invalid_reference(candidates);
    decision["required_context_refs"] = json!([]);
    decision["constraint_updates"] = json!([{"source_ref":source["ref"],"start_byte":0,"end_byte":5,"scope":{"kind":"global"},"supersedes":null}]);
    decision
}

fn invalid_obsolete_change_only(candidates: &Value) -> Value {
    let mut decision = invalid_reference(candidates);
    decision["required_context_refs"] = json!([]);
    decision["constraint_changes"] = json!([{"prior_context_ref":candidates["context"][0]["ref"],
        "replacement_input_ref":candidates["input"]["ref"]}]);
    decision
}

fn invalid_obsolete_inconsistent_change_and_update(candidates: &Value) -> Value {
    let mut decision = invalid_obsolete_change_only(candidates);
    decision["constraint_updates"] = json!([{"source_ref":candidates["input"]["ref"],
        "start_byte":0, "end_byte":1, "scope":{"kind":"global"}, "supersedes":null}]);
    decision
}

#[test]
fn task_context_invalid_sources_ranges_and_constraint_authority_stop_before_main_delivery() {
    for decide in [
        invalid_reference as fn(&Value) -> Value,
        invalid_utf8_objective,
        invalid_assistant_constraint,
        invalid_obsolete_change_only,
        invalid_obsolete_inconsistent_change_and_update,
    ] {
        let root = TemporaryDirectory::new("task-context-invalid-selection");
        initialize_project(root.path());
        let agent = definition(root.path(), None);
        let server = spawn_scripted_responses_server(vec![
            ScriptedResponse::InitialSelection,
            ScriptedResponse::Response(200, text_response(A_RESULT, "invalid-a")),
            ScriptedResponse::SelectionDecision(decide),
        ]);
        let config = write_context_window_config(root.path(), &server.base_url, 272_000);
        let output = run_chat(
            root.path(),
            &config,
            &format!("{TASK_A}\n{TASK_B}\n/exit\n"),
            &["--agent", agent.to_str().unwrap()],
        );
        assert!(
            !output.status.success(),
            "invalid selection cannot become successful delivery"
        );
        let _wires = (0..3)
            .map(|_| f2a_request_body(&server.requests, "invalid selection"))
            .collect::<Vec<_>>();
        assert!(server.requests.try_recv().is_err());
        server.join();
        let events = session_events(root.path(), &session_id(root.path()));
        assert_eq!(
            events
                .iter()
                .filter(|event| event["event_type"] == "session.task_context_selected")
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| event["event_type"] == "session.model_delivery_committed")
                .count(),
            1
        );
        assert!(String::from_utf8_lossy(&output.stderr).contains("task"));
    }
}

const REUSABLE_METHOD: &str = "Compare current volume with the same observed interval from the previous session; keep units and observation times explicit.";
const MIXED_ANSWER: &str = "TASK-ALPHA-PRIVATE: synthetic price 173.29, quantity 821. Reusable method: Compare current volume with the same observed interval from the previous session; keep units and observation times explicit. ALPHA-CONCLUSION-ONLY: requires fresh verification.";

fn select_only_method_range(candidates: &Value) -> Value {
    let source = candidates["context"]
        .as_array()
        .unwrap()
        .iter()
        .find(|unit| unit["excerpt"] == MIXED_ANSWER)
        .unwrap();
    let start = MIXED_ANSWER.find(REUSABLE_METHOD).unwrap();
    json!({
        "relation":"new","focus":{"kind":"new_from_input","input_ref":candidates["input"]["ref"]},
        "related_task_refs":[],"required_context_refs":[],"unresolved_references":[],
        "source_ranges":[{"context_ref":source["ref"],"start_byte":start,"end_byte":start+REUSABLE_METHOD.len()}]
    })
}

fn continue_with_method_range(candidates: &Value) -> Value {
    let mut selection = select_only_method_range(candidates);
    selection["relation"] = json!("continue");
    selection["focus"] =
        json!({"kind":"existing","task_ref":task_reference(candidates, "TASK-ALPHA")});
    selection
}

#[test]
fn task_context_method_dependency_uses_exact_range_from_mixed_answer() {
    for choose in [
        select_only_method_range as fn(&Value) -> Value,
        continue_with_method_range,
    ] {
        let root = TemporaryDirectory::new("task-context-method-range");
        initialize_project(root.path());
        let agent = definition(root.path(), None);
        let server = spawn_scripted_responses_server(vec![
            ScriptedResponse::InitialSelection,
            ScriptedResponse::Response(200, text_response(MIXED_ANSWER, "method-source")),
            ScriptedResponse::SelectionDecision(choose),
            ScriptedResponse::Response(200, text_response("B-METHOD-APPLIED", "method-applied")),
        ]);
        let config = write_context_window_config(root.path(), &server.base_url, 272_000);
        let input =
            "Use the earlier volume comparison method for TASK-BETA; obtain new data for B.";
        let output = run_chat(
            root.path(),
            &config,
            &format!("{TASK_A}\n{input}\n/exit\n"),
            &["--agent", agent.to_str().unwrap()],
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let wires = (0..4)
            .map(|_| f2a_request_body(&server.requests, "method-only dependency"))
            .collect::<Vec<_>>();
        assert!(server.requests.try_recv().is_err());
        server.join();
        assert!(wires[3].to_string().contains(REUSABLE_METHOD));
        assert!(wires[3].to_string().contains(input));
        for excluded in [
            "TASK-ALPHA-PRIVATE",
            "173.29",
            "ALPHA-CONCLUSION-ONLY",
            TASK_A,
        ] {
            assert!(
                !wires[3].to_string().contains(excluded),
                "method range reintroduced excluded body: {excluded}"
            );
        }
        assert!(
            wires[3]
                .get("previous_response_id")
                .is_none_or(Value::is_null)
        );
        let session = session_id(root.path());
        let state = reduced_session(&Project::discover(root.path()).unwrap(), &session);
        assert_eq!(state.history().items().len(), 4);
        assert_eq!(
            state.history().items()[1],
            echidna::domain::model::ContentItem::assistant_text(MIXED_ANSWER)
        );
        let events = session_events(root.path(), &session);
        let source = events
            .iter()
            .find(|event| {
                event["event_type"] == "session.history_item_recorded"
                    && event["payload"]["item"]["text"] == MIXED_ANSWER
            })
            .unwrap();
        let audit = state.request_audits().last().unwrap();
        let delivery_map = serde_json::to_string(audit.context_delivery_map().unwrap()).unwrap();
        assert!(
            delivery_map.contains(source["event_id"].as_str().unwrap()),
            "the excerpt must retain the original source EventId"
        );
        let selection = events
            .iter()
            .rfind(|event| event["event_type"] == "session.task_context_selected")
            .unwrap();
        let recorded_ranges = &selection["payload"]["decision"]["source_ranges"];
        assert_eq!(recorded_ranges.as_array().unwrap().len(), 1);
        assert_eq!(
            recorded_ranges[0]["start_byte"],
            MIXED_ANSWER.find(REUSABLE_METHOD).unwrap()
        );
        assert_eq!(
            recorded_ranges[0]["end_byte"],
            MIXED_ANSWER.find(REUSABLE_METHOD).unwrap() + REUSABLE_METHOD.len()
        );
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
    }
}

fn select_initial_memory(candidates: &Value) -> Value {
    assert!(candidates["tasks"].as_array().unwrap().is_empty());
    let references = candidates["context"]
        .as_array()
        .unwrap()
        .iter()
        .map(|unit| unit["ref"].clone())
        .collect::<Vec<_>>();
    json!({
        "relation":"new","focus":{"kind":"new_from_input","input_ref":candidates["input"]["ref"]},
        "related_task_refs":[],"required_context_refs":references,"unresolved_references":[]
    })
}

#[test]
fn task_context_read_reports_ranges_missing_stale_and_unknown_time_separately() {
    let root = TemporaryDirectory::new("task-context-read-statuses");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let manifest = agent.join("agent.toml");
    fs::write(
        &manifest,
        fs::read_to_string(&manifest)
            .unwrap()
            .replace("memory = \"none\"", "memory = \"agent\""),
    )
    .unwrap();
    let memory = root
        .path()
        .join(".echidna/memory/agents/task-context-fixture/core.md");
    fs::create_dir_all(memory.parent().unwrap()).unwrap();
    let original = "TIMELESS-MEMORY: retain exact units；未知生效日期，不推断为今天。";
    fs::write(&memory, original).unwrap();
    let server = spawn_scripted_responses_server(vec![
        ScriptedResponse::SelectionDecision(select_initial_memory),
        ScriptedResponse::Response(
            200,
            tool_calls_response(
                &[
                    (
                        "bounded-source",
                        "context_read",
                        json!({"operation":"search","query":"TIMELESS-MEMORY","max_bytes":19}),
                    ),
                    (
                        "missing-source",
                        "context_read",
                        json!({"operation":"read","reference":"nonexistent-source","max_bytes":128}),
                    ),
                    (
                        "stale-source",
                        "context_read",
                        json!({"operation":"read","reference":"memory:core.md:old-revision","max_bytes":128}),
                    ),
                    (
                        "unknown-time",
                        "context_read",
                        json!({"operation":"search","query":"TIMELESS-MEMORY","after_unix":1,"max_bytes":128}),
                    ),
                ],
                "status-reads",
            ),
        ),
        ScriptedResponse::Response(200, text_response("READ-STATUSES-CHECKED", "status-final")),
    ]);
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "Read the scoped source status without assuming its valid time.\n/exit\n",
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for _ in 0..3 {
        f2a_request_body(&server.requests, "context source statuses");
    }
    assert!(server.requests.try_recv().is_err());
    server.join();
    let state = reduced_session(
        &Project::discover(root.path()).unwrap(),
        &session_id(root.path()),
    );
    assert_eq!(state.request_audits().len(), 3);
    assert_eq!(state.execution_receipt_count(), 4);
    state.history().ensure_all_tool_calls_paired().unwrap();
    let results = state
        .history()
        .items()
        .iter()
        .filter_map(|item| match item {
            echidna::domain::model::ContentItem::ToolResult { result } => {
                Some((result.call_id.as_str(), &result.output))
            }
            _ => None,
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(results["bounded-source"]["status"], "found");
    let entry = &results["bounded-source"]["entries"][0];
    assert_eq!(entry["offset"], 0);
    assert_eq!(entry["next_offset"], 19);
    assert_eq!(entry["text"], &original[..19]);
    assert_eq!(entry["total_bytes"], original.len());
    assert_eq!(entry["truncated"], true);
    assert!(entry["valid_time"].is_null());
    assert!(entry["observed_at"].is_null());
    assert!(entry["source_role"].is_null());
    assert_eq!(entry["source_origin"], "project_memory");
    assert_eq!(entry["source_authority"], "derived_non_evidence");
    assert_eq!(results["missing-source"]["status"], "not_found");
    assert_eq!(results["stale-source"]["status"], "stale");
    assert_eq!(results["unknown-time"]["status"], "ambiguous");
    assert!(
        results["unknown-time"]["entries"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(fs::read_to_string(memory).unwrap(), original);
}

#[test]
fn task_context_selector_cannot_execute_tools_or_silently_repair_invalid_json() {
    for invalid_response in [
        text_response("not a selection JSON object", "invalid-selector-json"),
        m4_long_flow_tool_response(
            "selector-must-not-execute",
            "run_command",
            json!({"argv":["/bin/sh","-c","printf executed > forbidden-selector-effect.txt"]}),
            "invalid-selector-tool",
        ),
    ] {
        let root = TemporaryDirectory::new("task-context-selector-no-execution");
        initialize_project(root.path());
        let agent = definition(root.path(), None);
        let server = spawn_scripted_responses_server(vec![
            ScriptedResponse::InitialSelection,
            ScriptedResponse::Response(200, text_response(A_RESULT, "selector-boundary-a")),
            ScriptedResponse::Response(200, invalid_response),
        ]);
        let config = write_context_window_config(root.path(), &server.base_url, 272_000);
        let output = run_chat(
            root.path(),
            &config,
            &format!("{TASK_A}\n{TASK_B}\n/exit\n"),
            &["--agent", agent.to_str().unwrap()],
        );
        assert!(!output.status.success());
        selector_candidates(&f2a_request_body(
            &server.requests,
            "initial selector before invalid selector",
        ));
        f2a_request_body(&server.requests, "first main before invalid selector");
        let selector = f2a_request_body(&server.requests, "invalid tool-free selector");
        selector_candidates(&selector);
        assert!(server.requests.try_recv().is_err());
        server.join();
        assert!(!root.path().join("forbidden-selector-effect.txt").exists());
        let session = session_id(root.path());
        let state = reduced_session(&Project::discover(root.path()).unwrap(), &session);
        assert_eq!(
            state.request_audits().len(),
            3,
            "invalid selector content does not trigger transport retry or a main request"
        );
        assert_eq!(state.execution_receipt_count(), 0);
        assert_eq!(
            state.history().items().len(),
            3,
            "only the two real user inputs and the prior real answer are History"
        );
        let events = session_events(root.path(), &session);
        assert_eq!(
            events
                .iter()
                .filter(|event| event["event_type"] == "session.task_context_selected")
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| event["event_type"] == "session.task_selection_sample_recorded")
                .count(),
            1
        );
    }
}

#[test]
fn task_context_large_input_keeps_original_utf8_and_reports_uninspected_selection_ranges() {
    let root = TemporaryDirectory::new("task-context-large-input");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let server = spawn_scripted_responses_server(vec![
        ScriptedResponse::NewSelection,
        ScriptedResponse::NewSelection,
        ScriptedResponse::Response(
            200,
            text_response("LARGE-INPUT-ACCEPTED", "large-input-final"),
        ),
    ]);
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let input = format!("ORIGINAL-PREFIX:{}:EXACT-TAIL-7359", "字".repeat(40_000));
    let output = run_chat(
        root.path(),
        &config,
        &format!("{input}\n/exit\n"),
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let wires = (0..3)
        .map(|_| f2a_request_body(&server.requests, "large input selection"))
        .collect::<Vec<_>>();
    assert!(server.requests.try_recv().is_err());
    server.join();
    let first = selector_candidates(&wires[0]);
    let second = selector_candidates(&wires[1]);
    for candidate in [&first, &second] {
        assert_eq!(candidate["input"]["total_bytes"], input.len());
        let inspected = candidate["input"]["inspected_bytes"].as_u64().unwrap() as usize;
        assert!(inspected < input.len());
        assert_eq!(candidate["input"]["text"], &input[..inspected]);
        assert!(
            candidate["gaps"]
                .as_array()
                .unwrap()
                .iter()
                .any(|gap| gap.as_str().unwrap().contains("not inspected"))
        );
    }
    assert!(
        second["input"]["inspected_bytes"].as_u64().unwrap()
            > first["input"]["inspected_bytes"].as_u64().unwrap()
    );
    assert_eq!(wires[2]["input"].to_string().matches(&input).count(), 1);
    let state = reduced_session(
        &Project::discover(root.path()).unwrap(),
        &session_id(root.path()),
    );
    assert_eq!(state.request_audits().len(), 3);
    assert_eq!(state.history().items().len(), 2);
    assert_eq!(
        state.history().items()[0],
        echidna::domain::model::ContentItem::user_text(input)
    );
}

const MIXED_CORE: &str = "# Shared\nGLOBAL-CORE-UNITS: always preserve original units.\n# Alpha\nLOCAL-CORE-ALPHA: a synthetic task-only conclusion.\n# Unknown\nUNKNOWN-CORE-SCOPE: applicability has not been established.\n";

const CORE_CORRECTION: &str = "GLOBAL-CORRECTED-UNITS: always use the source units explicitly.";
const CORE_TO_LOCAL: &str =
    "Correct the earlier global Core rule: only for this task use source units explicitly.";

const ORIGINAL_CORE_RULE: &str = "GLOBAL-CORE-UNITS: always preserve original units.";

fn contains_non_tool_text(value: &Value, text: &str) -> bool {
    match value {
        Value::String(value) => value.contains(text),
        Value::Array(values) => values
            .iter()
            .any(|value| contains_non_tool_text(value, text)),
        Value::Object(values) => {
            if values
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|kind| matches!(kind, "function_call" | "function_call_output"))
            {
                return false;
            }
            values
                .values()
                .any(|value| contains_non_tool_text(value, text))
        }
        _ => false,
    }
}

fn core_lineage_fixture(root: &std::path::Path) -> (std::path::PathBuf, std::path::PathBuf) {
    initialize_project(root);
    let agent = definition(root, None);
    let manifest = agent.join("agent.toml");
    let content = fs::read_to_string(&manifest)
        .unwrap()
        .replace("memory = \"none\"", "memory = \"agent\"");
    fs::write(&manifest, format!("{content}memory_write = \"direct\"\n")).unwrap();
    let memory = root.join(".echidna/memory/agents/task-context-fixture/core.md");
    fs::create_dir_all(memory.parent().unwrap()).unwrap();
    fs::write(&memory, MIXED_CORE).unwrap();
    (agent, memory)
}

#[test]
fn corrected_core_section_tracks_only_unique_continuous_revision_sources() {
    for mode in ["unrelated", "duplicate", "changed", "deleted"] {
        let root = TemporaryDirectory::new("core-correction-lineage");
        let (agent, memory) = core_lineage_fixture(root.path());
        let initial = spawn_scripted_responses_server(vec![
            ScriptedResponse::SelectionDecision(classify_mixed_core),
            ScriptedResponse::Response(200, text_response(A_RESULT, "lineage-original")),
            ScriptedResponse::SelectionDecision(correct_known_global_core),
            ScriptedResponse::Response(200, text_response("Corrected.", "lineage-corrected")),
        ]);
        let config = write_context_window_config(root.path(), &initial.base_url, 272_000);
        let output = run_chat(
            root.path(),
            &config,
            &format!("{TASK_A}\n{CORE_CORRECTION}\n/exit\n"),
            &["--agent", agent.to_str().unwrap()],
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        for _ in 0..4 {
            f2a_request_body(&initial.requests, "initial Core correction");
        }
        assert!(initial.requests.try_recv().is_err());
        initial.join();
        let session = session_id(root.path());
        let revised = match mode {
            "unrelated" => format!("# Added before original\nNEW-UNRELATED-SECTION\n{MIXED_CORE}"),
            "duplicate" => MIXED_CORE.replace(
                "# Alpha",
                &format!("# Shared\n{ORIGINAL_CORE_RULE}\n# Alpha"),
            ),
            "changed" => MIXED_CORE.replace(
                ORIGINAL_CORE_RULE,
                "ACTUAL-CORE-NEW-RULE: preserve new values.",
            ),
            "deleted" => MIXED_CORE.replace(&format!("# Shared\n{ORIGINAL_CORE_RULE}\n"), ""),
            _ => unreachable!(),
        };
        fs::write(&memory, &revised).unwrap();
        for phase in 0..2 {
            if phase == 1 {
                let next = if mode == "unrelated" {
                    format!("# Another before original\nSECOND-UNRELATED\n{MIXED_CORE}")
                } else {
                    MIXED_CORE.to_owned()
                };
                fs::write(&memory, next).unwrap();
            }
            let server = spawn_scripted_responses_server(vec![
                ScriptedResponse::NewSelection,
                ScriptedResponse::Response(
                    200,
                    text_response("Observed current Core.", "lineage-next"),
                ),
            ]);
            let config = write_context_window_config(root.path(), &server.base_url, 272_000);
            let output = run_chat(
                root.path(),
                &config,
                "New unrelated task.\n/exit\n",
                &["--resume", &session],
            );
            assert!(
                output.status.success(),
                "mode={mode} phase={phase} {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let _selector = f2a_request_body(&server.requests, "lineage selector");
            let wire = f2a_request_body(&server.requests, "lineage main");
            assert!(server.requests.try_recv().is_err());
            server.join();
            assert!(json_contains_text(&wire["input"], CORE_CORRECTION));
            let original_expected = mode == "duplicate" || phase == 1 && mode != "unrelated";
            assert_eq!(
                json_contains_text(&wire["input"], ORIGINAL_CORE_RULE),
                original_expected,
                "mode={mode} phase={phase}"
            );
            if mode == "changed" && phase == 0 {
                assert!(json_contains_text(&wire["input"], "ACTUAL-CORE-NEW-RULE"));
            }
        }
        let state = reduced_session(&Project::discover(root.path()).unwrap(), &session);
        assert_audits_reconstruct(root.path(), &state);
    }
}

#[test]
fn corrected_core_section_survives_same_turn_core_write_and_resume() {
    let root = TemporaryDirectory::new("core-correction-same-turn-refresh");
    let (agent, memory) = core_lineage_fixture(root.path());
    let updated = format!("# Added before original\nSAME-TURN-UNRELATED\n{MIXED_CORE}");
    let hash = blake3::hash(MIXED_CORE.as_bytes()).to_hex().to_string();
    let server = spawn_scripted_responses_server(vec![
        ScriptedResponse::SelectionDecision(classify_mixed_core),
        ScriptedResponse::Response(200, text_response(A_RESULT, "core-write-original")),
        ScriptedResponse::SelectionDecision(correct_known_global_core),
        ScriptedResponse::Response(
            200,
            m4_long_flow_tool_response(
                "core-lineage-write",
                "memory_write",
                json!({"path":"core.md","content":updated,"expected_hash":hash,"event_id":"core-lineage-write"}),
                "core-write-call",
            ),
        ),
        ScriptedResponse::Response(200, text_response("Core updated.", "core-write-done")),
        ScriptedResponse::NewSelection,
        ScriptedResponse::Response(200, text_response("Separate task.", "core-write-next")),
    ]);
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        &format!("{TASK_A}\n{CORE_CORRECTION}\nA separate task.\n/exit\n"),
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let wires = (0..7)
        .map(|_| f2a_request_body(&server.requests, "Core same-turn refresh"))
        .collect::<Vec<_>>();
    assert!(server.requests.try_recv().is_err());
    server.join();
    for index in [3, 4, 6] {
        assert!(json_contains_text(&wires[index]["input"], CORE_CORRECTION));
        // The real memory_write Call may contain the complete file bytes;
        // its paired execution history is not a selected Core instruction.
        assert!(!contains_non_tool_text(
            &wires[index]["input"],
            ORIGINAL_CORE_RULE
        ));
    }
    assert!(
        fs::read_to_string(memory)
            .unwrap()
            .contains("SAME-TURN-UNRELATED")
    );
    let session = session_id(root.path());
    let state = reduced_session(&Project::discover(root.path()).unwrap(), &session);
    assert_eq!(state.execution_receipt_count(), 1);
    assert_audits_reconstruct(root.path(), &state);
    let resumed = run_chat(
        root.path(),
        &config,
        "/context\n/exit\n",
        &["--resume", &session],
    );
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
}

fn correct_known_global_core(candidates: &Value) -> Value {
    let core = candidates["context"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| {
            entry["excerpt"]
                .as_str()
                .is_some_and(|text| text.contains("GLOBAL-CORE-UNITS"))
        })
        .unwrap();
    assert_eq!(core["known_scope"]["kind"], "global");
    json!({"relation":"new", "focus":{"kind":"new_from_input", "input_ref":candidates["input"]["ref"]},
        "related_task_refs":[], "required_context_refs":[], "unresolved_references":[],
        "constraint_updates":[{"source_ref":candidates["input"]["ref"], "start_byte":0,
            "end_byte":CORE_CORRECTION.len(), "scope":{"kind":"global"}, "supersedes":core["ref"]}]})
}

fn invalid_global_core_downgrade(candidates: &Value) -> Value {
    let core = candidates["context"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| {
            entry["excerpt"]
                .as_str()
                .is_some_and(|text| text.contains("GLOBAL-CORE-UNITS"))
        })
        .unwrap();
    json!({"relation":"new", "focus":{"kind":"new_from_input", "input_ref":candidates["input"]["ref"]},
        "related_task_refs":[], "required_context_refs":[], "unresolved_references":[],
        "scope_classifications":[{"context_ref":core["ref"],"scope":{"kind":"current_task"}}]})
}

fn correct_global_core_to_current_task(candidates: &Value) -> Value {
    let mut decision = correct_known_global_core(candidates);
    decision["constraint_updates"][0]["end_byte"] = json!(CORE_TO_LOCAL.len());
    decision["constraint_updates"][0]["scope"] = json!({"kind":"current_task"});
    decision
}

#[test]
fn task_context_global_core_correction_requires_source_range_and_survives_resume() {
    for mode in ["global", "local", "invalid"] {
        let valid = mode != "invalid";
        let correction_text = if mode == "local" {
            CORE_TO_LOCAL
        } else {
            CORE_CORRECTION
        };
        let root = TemporaryDirectory::new("task-global-core-correction");
        initialize_project(root.path());
        let agent = definition(root.path(), None);
        let manifest = agent.join("agent.toml");
        fs::write(
            &manifest,
            fs::read_to_string(&manifest)
                .unwrap()
                .replace("memory = \"none\"", "memory = \"agent\""),
        )
        .unwrap();
        let memory = root
            .path()
            .join(".echidna/memory/agents/task-context-fixture/core.md");
        fs::create_dir_all(memory.parent().unwrap()).unwrap();
        fs::write(&memory, MIXED_CORE).unwrap();
        let mut steps = vec![
            ScriptedResponse::SelectionDecision(classify_mixed_core),
            ScriptedResponse::Response(200, text_response(A_RESULT, "core-original")),
            ScriptedResponse::SelectionDecision(match mode {
                "global" => correct_known_global_core,
                "local" => correct_global_core_to_current_task,
                _ => invalid_global_core_downgrade,
            }),
        ];
        if valid {
            steps.extend([
                ScriptedResponse::Response(200, text_response("Corrected.", "core-corrected")),
                ScriptedResponse::NewSelection,
                ScriptedResponse::Response(200, text_response("Retained.", "core-retained")),
            ]);
        }
        let server = spawn_scripted_responses_server(steps);
        let config = write_context_window_config(root.path(), &server.base_url, 272_000);
        let output = run_chat(
            root.path(),
            &config,
            &format!("{TASK_A}\n{correction_text}\nA separate task.\n/exit\n"),
            &["--agent", agent.to_str().unwrap()],
        );
        assert_eq!(
            output.status.success(),
            valid,
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let wires = (0..if valid { 6 } else { 3 })
            .map(|_| f2a_request_body(&server.requests, "global Core correction"))
            .collect::<Vec<_>>();
        assert!(server.requests.try_recv().is_err());
        server.join();
        let id = session_id(root.path());
        let state = reduced_session(&Project::discover(root.path()).unwrap(), &id);
        if valid {
            for index in [3, 5] {
                assert_eq!(
                    json_contains_text(&wires[index]["input"], correction_text),
                    mode == "global" || index == 3
                );
                assert!(!json_contains_text(
                    &wires[index]["input"],
                    "GLOBAL-CORE-UNITS"
                ));
            }
            let events = session_events(root.path(), &id);
            let correction = events
                .iter()
                .find(|event| {
                    event["event_type"] == "session.task_context_selected"
                        && event["payload"]["constraint_revisions"]
                            .to_string()
                            .contains("GLOBAL-CORRECTED")
                })
                .cloned();
            // The range is stored as a hash and source EventId, not duplicated text.
            assert!(correction.is_none());
            let revision = events
                .iter()
                .flat_map(|event| {
                    event["payload"]["constraint_revisions"]
                        .as_array()
                        .into_iter()
                        .flatten()
                })
                .find(|revision| {
                    revision["supersedes"]
                        .as_str()
                        .is_some_and(|value| value.starts_with("core:"))
                })
                .unwrap();
            let source = events
                .iter()
                .find(|event| {
                    event["event_type"] == "session.history_item_recorded"
                        && event["payload"]["item"]["text"] == correction_text
                })
                .unwrap();
            assert_eq!(revision["range"]["source_event_id"], source["event_id"]);
            assert_eq!(revision["range"]["end_byte"], correction_text.len());
            assert_eq!(
                revision["scope"]["kind"],
                if mode == "global" { "global" } else { "task" }
            );
            assert_audits_reconstruct(root.path(), &state);
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
        } else {
            assert_eq!(state.request_audits().len(), 3);
        }
        assert_eq!(fs::read_to_string(memory).unwrap(), MIXED_CORE);
    }
}

fn classify_mixed_core(candidates: &Value) -> Value {
    let sections = candidates["context"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|unit| unit["core_range"].is_object())
        .collect::<Vec<_>>();
    assert_eq!(sections.len(), 3);
    let global = sections
        .iter()
        .find(|unit| {
            unit["excerpt"]
                .as_str()
                .unwrap()
                .contains("GLOBAL-CORE-UNITS")
        })
        .unwrap();
    let local = sections
        .iter()
        .find(|unit| {
            unit["excerpt"]
                .as_str()
                .unwrap()
                .contains("LOCAL-CORE-ALPHA")
        })
        .unwrap();
    json!({
        "relation":"new","focus":{"kind":"new_from_input","input_ref":candidates["input"]["ref"]},
        "related_task_refs":[],"required_context_refs":[],"unresolved_references":[],
        "scope_classifications":[
            {"context_ref":global["ref"],"scope":{"kind":"global"}},
            {"context_ref":local["ref"],"scope":{"kind":"current_task"}}
        ]
    })
}

#[test]
fn task_context_mixed_core_preserves_global_and_unknown_scopes_and_excludes_local_task() {
    let root = TemporaryDirectory::new("task-context-mixed-core");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let manifest = agent.join("agent.toml");
    fs::write(
        &manifest,
        fs::read_to_string(&manifest)
            .unwrap()
            .replace("memory = \"none\"", "memory = \"agent\""),
    )
    .unwrap();
    let memory = root
        .path()
        .join(".echidna/memory/agents/task-context-fixture/core.md");
    fs::create_dir_all(memory.parent().unwrap()).unwrap();
    fs::write(&memory, MIXED_CORE).unwrap();
    let persona_before = fs::read(agent.join("persona.md")).unwrap();
    let work_before = fs::read(agent.join("work.md")).unwrap();
    let server = spawn_scripted_responses_server(vec![
        ScriptedResponse::SelectionDecision(classify_mixed_core),
        ScriptedResponse::Response(200, text_response(A_RESULT, "mixed-core-a")),
        ScriptedResponse::NewSelection,
        ScriptedResponse::Response(200, text_response(B_RESULT, "mixed-core-b")),
    ]);
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        &format!("{TASK_A}\n{TASK_B}\n/exit\n"),
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let wires = (0..4)
        .map(|_| f2a_request_body(&server.requests, "mixed Core scopes"))
        .collect::<Vec<_>>();
    assert!(server.requests.try_recv().is_err());
    server.join();
    assert!(wires[1].to_string().contains("LOCAL-CORE-ALPHA"));
    assert!(wires[3].to_string().contains("GLOBAL-CORE-UNITS"));
    assert!(wires[3].to_string().contains("UNKNOWN-CORE-SCOPE"));
    assert!(!wires[3].to_string().contains("LOCAL-CORE-ALPHA"));
    assert!(!wires[3].to_string().contains("A_ONLY_RESULT"));
    assert!(
        wires[3]
            .get("previous_response_id")
            .is_none_or(Value::is_null)
    );
    assert_eq!(fs::read_to_string(memory).unwrap(), MIXED_CORE);
    assert_eq!(fs::read(agent.join("persona.md")).unwrap(), persona_before);
    assert_eq!(fs::read(agent.join("work.md")).unwrap(), work_before);
}

#[test]
fn task_context_switch_preserves_real_running_job_handle_without_replaying_start() {
    switch_with_real_running_process("start_job", "stop_job", "job_id");
}

#[test]
fn task_context_switch_preserves_real_running_pty_handle_without_replaying_start() {
    switch_with_real_running_process("start_pty", "stop_pty", "pty_session_id");
}

fn switch_with_real_running_process(
    start_tool: &'static str,
    stop_tool: &'static str,
    handle_key: &'static str,
) {
    let root = TemporaryDirectory::new("task-context-running-process");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let manifest = agent.join("agent.toml");
    let mut declared = fs::read_to_string(&manifest).unwrap();
    declared.push_str(&format!(
        "{start_tool} = \"direct\"\n{stop_tool} = \"direct\"\n"
    ));
    fs::write(manifest, declared).unwrap();
    let counter = root.path().join("job-count.txt");
    let observed_counter = counter.clone();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let (sender, requests) = mpsc::channel();
    let thread = thread::spawn(move || {
        let mut job_id = None;
        for index in 0..6 {
            let mut socket =
                f3_context::accept_before_deadline(&listener, "task switch with running Job");
            socket.set_nonblocking(false).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let raw = read_http_request(&mut socket);
            let wire = request_body(&raw);
            sender.send(raw).unwrap();
            let response = if index == 0 {
                initial_selection_response(&wire, "job-initial-selection")
            } else {
                match index - 1 {
                    0 => m4_long_flow_tool_response(
                        "start-once",
                        start_tool,
                        json!({"argv":["/bin/sh","-c","printf 'started\\n' >> job-count.txt; sleep 30"]}),
                        "job-start-main",
                    ),
                    1 => {
                        let result = wire["input"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .find(|item| {
                                item["type"] == "function_call_output"
                                    && item["call_id"] == "start-once"
                            })
                            .unwrap();
                        let result: Value =
                            serde_json::from_str(result["output"].as_str().unwrap()).unwrap();
                        job_id = Some(result["output"][handle_key].as_str().unwrap().to_owned());
                        let deadline = Instant::now() + Duration::from_secs(3);
                        while !observed_counter.exists() {
                            assert!(
                                Instant::now() < deadline,
                                "the actual background child did not start"
                            );
                            thread::sleep(Duration::from_millis(10));
                        }
                        text_response(
                            "A-PRIVATE-JOB-COMMENT: the synthetic operation is running.",
                            "job-a-final",
                        )
                    }
                    2 => {
                        let candidates = selector_candidates(&wire);
                        text_response(&json!({
                        "relation":"new","focus":{"kind":"new_from_input","input_ref":candidates["input"]["ref"]},
                        "related_task_refs":[],"required_context_refs":[],"unresolved_references":[]
                    }).to_string(), "job-b-selection")
                    }
                    3 => {
                        let handle = job_id.as_ref().unwrap();
                        assert!(
                            wire.to_string().contains(handle),
                            "new task must receive the real active Job handle even without selected old-task context"
                        );
                        assert!(!wire.to_string().contains("A-PRIVATE-JOB-COMMENT"));
                        assert!(wire.get("previous_response_id").is_none_or(Value::is_null));
                        m4_long_flow_tool_response(
                            "stop-existing",
                            stop_tool,
                            json!({handle_key:handle}),
                            "job-b-stop",
                        )
                    }
                    4 => text_response("EXISTING-JOB-STOPPED", "job-b-final"),
                    _ => unreachable!(),
                }
            };
            f3_context::write_sse_response(&mut socket, &response);
        }
    });
    let server = LoopbackServer {
        base_url: format!("http://{address}/v1"),
        requests,
        thread,
    };
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "TASK-ALPHA: start the synthetic background observation once.\nTASK-BETA: prepare an independent report and stop any active background process first.\n/exit\n",
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for _ in 0..6 {
        f2a_request_body(&server.requests, "running Job task switch");
    }
    assert!(server.requests.try_recv().is_err());
    server.join();
    assert_eq!(fs::read_to_string(counter).unwrap(), "started\n");
    let state = reduced_session(
        &Project::discover(root.path()).unwrap(),
        &session_id(root.path()),
    );
    assert_eq!(state.request_audits().len(), 6);
    assert_eq!(state.execution_receipt_count(), 2);
    state.history().ensure_all_tool_calls_paired().unwrap();
    let starts = state.history().items().iter().filter(|item| matches!(item, echidna::domain::model::ContentItem::ToolCall { call } if call.name.as_str() == start_tool)).count();
    assert_eq!(starts, 1);
    assert!(String::from_utf8_lossy(&output.stdout).contains("EXISTING-JOB-STOPPED"));
}

#[test]
fn task_context_main_replay_rejects_coherent_audit_forgery_against_unchanged_source() {
    use echidna::domain::model::ContentItem;
    use echidna::domain::request_audit::RequestAuditManifest;
    let root = TemporaryDirectory::new("task-context-independent-main-proof");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let server = spawn_scripted_responses_server(vec![
        ScriptedResponse::InitialSelection,
        ScriptedResponse::Response(200, text_response("ORIGINAL-REPLY", "proof-main")),
    ]);
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let original_input = "ORIGINAL-INPUT: preserve the exact operator requirement.";
    let output = run_chat(
        root.path(),
        &config,
        &format!("{original_input}\n/exit\n"),
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    selector_candidates(&f2a_request_body(
        &server.requests,
        "original proof selector",
    ));
    f2a_request_body(&server.requests, "original proof main request");
    assert!(server.requests.try_recv().is_err());
    server.join();
    let session = session_id(root.path());
    let events = session_events(root.path(), &session);
    let audit_position = events
        .iter()
        .rposition(|event| event["event_type"] == "session.request_audit_recorded")
        .unwrap();
    let prefix = events[..=audit_position].to_vec();
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
        .unwrap();
    };
    // The same independently valid attempted-request boundary can be opened
    // without resending the unaccepted request to the closed fixture server.
    write_events(&prefix);
    let valid = run_chat(
        root.path(),
        &config,
        "/context\n/exit\n",
        &["--resume", "latest"],
    );
    assert!(
        valid.status.success(),
        "{}",
        String::from_utf8_lossy(&valid.stderr)
    );
    let resumed = reduced_session(&Project::discover(root.path()).unwrap(), &session);
    assert_eq!(resumed.request_audits().len(), 2);
    let selection_sample = prefix
        .iter()
        .find(|event| event["event_type"] == "session.task_selection_sample_recorded")
        .unwrap();
    assert_eq!(
        selection_sample["payload"]["audit_event_id"],
        resumed.request_audits()[0].audit_event_id().as_str()
    );
    assert_eq!(
        prefix[audit_position]["payload"]["audit_event_id"],
        resumed.request_audits()[1].audit_event_id().as_str()
    );
    assert_eq!(
        resumed.history().items(),
        &[ContentItem::user_text(original_input)]
    );

    let mut forged = prefix.clone();
    let payload = &mut forged[audit_position]["payload"];
    let manifest: RequestAuditManifest = serde_json::from_value(payload.clone()).unwrap();
    let blobs = root.path().join(".echidna/artifacts/blobs/sha256");
    let mut input = manifest
        .reconstruct(|id| fs::read(blobs.join(id.as_str())))
        .unwrap();
    let position = input
        .content_items
        .iter()
        .position(|item| *item == ContentItem::user_text(original_input))
        .unwrap();
    let replacement =
        ContentItem::user_text("COHERENT-FORGED-INPUT: no operator supplied this content.");
    input.content_items[position] = replacement.clone();
    let bytes = delivery_lineage::canonical_bytes(&replacement);
    let blob = format!("{:x}", Sha256::digest(&bytes));
    fs::write(blobs.join(&blob), &bytes).unwrap();
    let item_hash = delivery_lineage::canonical_hash(&replacement);
    for key in ["components", "full_logical_components"] {
        let component = payload[key]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|component| {
                component["identity"]["kind"] == "content_item"
                    && component["identity"]["index"] == position
            })
            .unwrap();
        component["blob_id"] = json!(blob);
        component["size_bytes"] = json!(bytes.len());
        component["semantic_hash"] = json!(item_hash);
    }
    for key in ["context_delivery_map", "full_context_delivery_map"] {
        let entry = payload[key]["content_items"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|entry| entry["final_position"] == position)
            .unwrap();
        entry["content_hash"] = json!(item_hash);
    }
    let input_hash = delivery_lineage::canonical_hash(&input);
    payload["delivered_input"]["semantic_hash"] = json!(input_hash);
    payload["full_logical_input"]["semantic_hash"] = json!(input_hash);
    let rebuilt: RequestAuditManifest = serde_json::from_value(payload.clone()).unwrap();
    rebuilt.validate().unwrap();
    assert_eq!(
        rebuilt
            .reconstruct(|id| fs::read(blobs.join(id.as_str())))
            .unwrap(),
        input
    );
    assert_eq!(payload["components"], payload["full_logical_components"]);
    assert_eq!(
        payload["context_delivery_map"],
        payload["full_context_delivery_map"]
    );
    assert_eq!(&forged[..audit_position], &prefix[..audit_position]);
    let decoded = forged
        .iter()
        .cloned()
        .map(|event| serde_json::from_value::<EventEnvelope<Value>>(event).unwrap())
        .collect::<Vec<_>>();
    SessionReducer
        .reduce(decoded)
        .expect("the forged audit is structurally valid and all its self-hashes match");
    write_events(&forged);
    let before = fs::read(&event_path).unwrap();
    let rejected = run_chat(root.path(), &config, "/exit\n", &["--resume", "latest"]);
    assert!(!rejected.status.success());
    assert!(
        String::from_utf8_lossy(&rejected.stderr)
            .contains("primary audit differs from independently compiled task source"),
        "{}",
        String::from_utf8_lossy(&rejected.stderr)
    );
    assert_eq!(
        fs::read(&event_path).unwrap(),
        before,
        "failed independent verification must not mutate Session facts"
    );
}

fn select_research_experiment_alpha(candidates: &Value) -> Value {
    let contexts = candidates["context"].as_array().unwrap();
    let selected = contexts
        .iter()
        .find(|unit| {
            unit["ref"]
                .as_str()
                .unwrap()
                .starts_with("research:experiment:experiment-task-alpha:")
        })
        .expect("the older, explicitly requested Target's Experiment must be a candidate");
    json!({
        "relation":"new","focus":{"kind":"new_from_input","input_ref":candidates["input"]["ref"]},
        "related_task_refs":[],"required_context_refs":[selected["ref"]],"unresolved_references":[]
    })
}

#[test]
fn task_context_research_selects_one_experiment_and_closes_real_source_dependencies() {
    let root = TemporaryDirectory::new("task-context-research-dependencies");
    initialize_project(root.path());
    let first_server = spawn_scripted_responses_server(vec![
        ScriptedResponse::InitialSelection,
        ScriptedResponse::Response(
            200,
            tool_calls_response(
                &[
                    (
                        "task-research-alpha",
                        "run_command",
                        json!({"argv":["/usr/bin/printf","ALPHA-RECEIPT-OBSERVATION"]}),
                    ),
                    (
                        "task-research-beta",
                        "run_command",
                        json!({"argv":["/usr/bin/printf","BETA-UNRELATED-RECEIPT"]}),
                    ),
                ],
                "research-source-tools",
            ),
        ),
        ScriptedResponse::Response(
            200,
            text_response(
                "SETUP-COMPLETE: two genuine synthetic command observations.",
                "research-setup-final",
            ),
        ),
    ]);
    let config = write_context_window_config(root.path(), &first_server.base_url, 272_000);
    let initial = run_chat(
        root.path(),
        &config,
        "Record two synthetic local command observations.\n/exit\n",
        &["--phase", "experiment"],
    );
    assert!(
        initial.status.success(),
        "{}",
        String::from_utf8_lossy(&initial.stderr)
    );
    for _ in 0..3 {
        f2a_request_body(&first_server.requests, "real Research receipt setup");
    }
    assert!(first_server.requests.try_recv().is_err());
    first_server.join();
    let session = session_id(root.path());
    let initial_events = session_events(root.path(), &session);
    let project = Project::discover(root.path()).unwrap();
    let mut receipt_ids = Vec::new();
    for (label, first_sequence, observation) in [
        ("alpha", 10, "ALPHA-RECEIPT-OBSERVATION"),
        ("beta", 20, "BETA-UNRELATED-RECEIPT"),
    ] {
        let call = format!("task-research-{label}");
        let outcome = initial_events
            .iter()
            .find(|event| {
                event["event_type"] == "tool.execution_outcome_recorded"
                    && event["payload"]["result"]["call_id"] == call
            })
            .unwrap();
        let receipt = ExecutionReceiptId::new(outcome["event_id"].as_str().unwrap());
        let execution = echidna::domain::tools::ExecutionId::new(
            outcome["payload"]["receipt"]["execution_id"]
                .as_str()
                .unwrap(),
        );
        receipt_ids.push(receipt.clone());
        let snapshot_id = TargetSnapshotId::new(format!("snapshot-task-{label}"));
        let target_id = format!("synthetic.target.{label}");
        let snapshot = TargetSnapshotRecord::from_draft(TargetSnapshotDraft {
            id: snapshot_id.clone(),
            target_id: target_id.clone(),
            kind: TargetKind::Source,
            locator: format!("synthetic/{label}"),
            version: "fixture-1".to_owned(),
            build_id: format!("BUILD-{label}"),
            target_artifact_ids: Vec::new(),
            environment_fingerprint: format!("ENV-{label}"),
            device_fingerprint: "fixture-host".to_owned(),
            captured_at: "2026-08-31T00:00:00Z".to_owned(),
            notes: format!("TARGET-DETAIL-{label}"),
        })
        .unwrap();
        append_research_event(
            &project,
            &first_sequence.to_string(),
            ResearchEvent::TargetSnapshotRecorded(snapshot),
        );
        let hypothesis_id = HypothesisId::new(format!("hypothesis-task-{label}"));
        let hypothesis = HypothesisRecord::from_draft(HypothesisDraft {
            id: hypothesis_id.clone(),
            title: format!("HYPOTHESIS-DETAIL-{label}"),
            target_id,
            attack_surface: format!("synthetic-{label}"),
            vulnerability_class: "synthetic boundary observation".to_owned(),
            entry_point: format!("fixture-entry-{label}"),
            sensitive_sink: format!("fixture-sink-{label}"),
            rationale: "No vulnerability is claimed by this synthetic fixture.".to_owned(),
            status: HypothesisStatus::Testing,
            priority: 1,
            evidence_links: Vec::new(),
            last_action: None,
            next_action: None,
        })
        .unwrap();
        append_research_event(
            &project,
            &(first_sequence + 1).to_string(),
            ResearchEvent::HypothesisRecorded {
                record: hypothesis,
                keep_independent: false,
            },
        );
        let experiment_id = ExperimentId::new(format!("experiment-task-{label}"));
        let capsule_id = echidna::domain::research::CapsuleId::new(format!("capsule-task-{label}"));
        let (experiment, capsule) = ExperimentRecord::from_drafts(
            ExperimentDraft {
                id: experiment_id.clone(),
                hypothesis_id,
                target_snapshot_id: snapshot_id.clone(),
                capsule_id: capsule_id.clone(),
                goal: format!("EXPERIMENT-DETAIL-{label}"),
                result: ExperimentResult::Inconclusive,
                artifact_refs: Vec::new(),
                notes: "Observed a command, not proof of a security finding.".to_owned(),
            },
            ExperimentCapsuleDraft {
                id: capsule_id,
                experiment_id,
                target_snapshot_id: snapshot_id,
                action: "run_command".to_owned(),
                arguments: json!({"argv":["/usr/bin/printf",observation]}),
                tool_version: "1".to_owned(),
                execution_ids: vec![execution],
                input_artifact_ids: Vec::new(),
                environment_delta: String::new(),
                account_prerequisites: Vec::new(),
                toolchain_fingerprint: "synthetic-local-printf".to_owned(),
                execution_receipt_ids: vec![receipt],
                output_artifact_ids: Vec::new(),
            },
        )
        .unwrap();
        append_research_event(
            &project,
            &(first_sequence + 2).to_string(),
            ResearchEvent::ExperimentRecorded {
                experiment,
                capsule: Box::new(capsule),
                rerun_reason: None,
            },
        );
    }
    let second_server = spawn_scripted_responses_server(vec![
        ScriptedResponse::SelectionDecision(select_research_experiment_alpha),
        ScriptedResponse::SelectionDecision(select_research_experiment_alpha),
        ScriptedResponse::Response(
            200,
            text_response(
                &"ALPHA-RESEARCH-REVIEWED: source observations remain inconclusive. ".repeat(600),
                "research-selected-main",
            ),
        ),
    ]);
    let config = write_context_window_config(root.path(), &second_server.base_url, 272_000);
    let selected = run_chat(
        root.path(),
        &config,
        "Review experiment-task-alpha and its original reproducibility sources; the beta target is unrelated.\n/exit\n",
        &["--resume", "latest"],
    );
    assert!(
        selected.status.success(),
        "{}",
        String::from_utf8_lossy(&selected.stderr)
    );
    let first_selection = selector_candidates(&f2a_request_body(
        &second_server.requests,
        "Research selection request",
    ));
    let expanded_selection = selector_candidates(&f2a_request_body(
        &second_server.requests,
        "expanded Research selection request",
    ));
    let experiment_candidate = |candidates: &Value| {
        candidates["context"]
            .as_array()
            .unwrap()
            .iter()
            .find(|unit| {
                unit["ref"]
                    .as_str()
                    .unwrap()
                    .starts_with("research:experiment:experiment-task-alpha:")
            })
            .unwrap()
            .clone()
    };
    let first_candidate = experiment_candidate(&first_selection);
    let expanded_candidate = experiment_candidate(&expanded_selection);
    assert_eq!(first_candidate["truncated"], true);
    assert_eq!(first_candidate["ref"], expanded_candidate["ref"]);
    assert!(
        expanded_candidate["excerpt"].as_str().unwrap().len()
            > first_candidate["excerpt"].as_str().unwrap().len()
    );
    let main = f2a_request_body(&second_server.requests, "selected Research main request");
    assert!(second_server.requests.try_recv().is_err());
    second_server.join();
    let main_text = main.to_string();
    for required in [
        "experiment-task-alpha",
        "hypothesis-task-alpha",
        "snapshot-task-alpha",
        "capsule-task-alpha",
        "TARGET-DETAIL-alpha",
        "HYPOTHESIS-DETAIL-alpha",
        receipt_ids[0].as_str(),
    ] {
        assert!(
            main_text.contains(required),
            "selected Experiment omitted dependency {required}"
        );
    }
    for excluded in [
        "EXPERIMENT-DETAIL-beta",
        "TARGET-DETAIL-beta",
        "HYPOTHESIS-DETAIL-beta",
        "BETA-UNRELATED-RECEIPT",
        receipt_ids[1].as_str(),
        "SETUP-COMPLETE",
    ] {
        assert!(
            !main_text.contains(excluded),
            "unrelated Research source reentered main context: {excluded}"
        );
    }
    assert!(main.get("previous_response_id").is_none_or(Value::is_null));
    let state = reduced_session(&project, &session);
    assert_eq!(
        state.execution_receipt_count(),
        2,
        "Research selection cannot replay either real command"
    );
    assert_eq!(state.request_audits().len(), 6);
    let selected_events = session_events(root.path(), &session);
    let samples = selected_events
        .iter()
        .filter(|event| event["event_type"] == "session.task_selection_sample_recorded")
        .collect::<Vec<_>>();
    assert_eq!(samples.len(), 3);
    for (sample, audit_index) in samples.iter().zip([0, 3, 4]) {
        assert_eq!(
            sample["payload"]["audit_event_id"],
            state.request_audits()[audit_index]
                .audit_event_id()
                .as_str()
        );
    }
    // The handoff prose deliberately contains no source IDs. Dependencies
    // must come from the frozen selected sources, not from a model copying
    // identifiers into a summary or explicitly selecting Research again.
    let compacting = spawn_responses_server(vec![(
        200,
        compaction_summary_response(
            "Keep the reviewed experiment inconclusive and consult its original observations.",
            "research-task-summary",
        ),
    )]);
    let compact_config = write_context_window_config(root.path(), &compacting.base_url, 272_000);
    let compacted = run_chat(
        root.path(),
        &compact_config,
        "/compact\n/exit\n",
        &["--resume", "latest"],
    );
    assert!(
        compacted.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&compacted.stdout),
        String::from_utf8_lossy(&compacted.stderr)
    );
    assert!(String::from_utf8_lossy(&compacted.stdout).contains("Compaction revision 1 applied"));
    f2a_request_body(&compacting.requests, "Research task handoff");
    assert!(compacting.requests.try_recv().is_err());
    compacting.join();
    let resumed_server = spawn_scripted_responses_server(vec![
        ScriptedResponse::SelectionDecision(
            super::automatic_compaction::continue_from_selected_summary,
        ),
        ScriptedResponse::SelectionDecision(
            super::automatic_compaction::continue_from_selected_summary,
        ),
        ScriptedResponse::Response(
            200,
            text_response("RESEARCH-SUMMARY-CONTINUED", "research-summary-main"),
        ),
    ]);
    let config = write_context_window_config(root.path(), &resumed_server.base_url, 272_000);
    let reopened = run_chat(
        root.path(),
        &config,
        "Continue the reviewed experiment from its task summary.\n/exit\n",
        &["--resume", "latest"],
    );
    assert!(
        reopened.status.success(),
        "{}",
        String::from_utf8_lossy(&reopened.stderr)
    );
    for _ in 0..2 {
        selector_candidates(&f2a_request_body(
            &resumed_server.requests,
            "summary-only Research selection",
        ));
    }
    let resumed_main = f2a_request_body(&resumed_server.requests, "Research summary continuation");
    assert!(resumed_server.requests.try_recv().is_err());
    resumed_server.join();
    assert!(
        resumed_main
            .get("previous_response_id")
            .is_none_or(Value::is_null)
    );
    let resumed_text = resumed_main.to_string();
    for required in [
        "experiment-task-alpha",
        "hypothesis-task-alpha",
        "snapshot-task-alpha",
        "capsule-task-alpha",
        "TARGET-DETAIL-alpha",
        "HYPOTHESIS-DETAIL-alpha",
        receipt_ids[0].as_str(),
    ] {
        assert!(
            resumed_text.contains(required),
            "summary-only Resume lost Research dependency {required}"
        );
    }
    for excluded in [
        "EXPERIMENT-DETAIL-beta",
        "TARGET-DETAIL-beta",
        "HYPOTHESIS-DETAIL-beta",
        "BETA-UNRELATED-RECEIPT",
        receipt_ids[1].as_str(),
        "SETUP-COMPLETE",
    ] {
        assert!(
            !resumed_text.contains(excluded),
            "summary-only Resume imported unrelated source {excluded}"
        );
    }
    let resumed_state = reduced_session(&project, &session);
    assert_eq!(resumed_state.execution_receipt_count(), 2);
    assert_eq!(resumed_state.request_audits().len(), 10);
    let resumed_events = session_events(root.path(), &session);
    let resumed_samples = resumed_events
        .iter()
        .filter(|event| event["event_type"] == "session.task_selection_sample_recorded")
        .collect::<Vec<_>>();
    assert_eq!(resumed_samples.len(), 5);
    for (sample, audit_index) in resumed_samples.iter().skip(3).zip([7, 8]) {
        assert_eq!(
            sample["payload"]["audit_event_id"],
            resumed_state.request_audits()[audit_index]
                .audit_event_id()
                .as_str()
        );
    }
    assert_eq!(
        resumed_state.history().ensure_all_tool_calls_paired(),
        Ok(())
    );
    assert_audits_reconstruct(root.path(), &resumed_state);
    // Keep a valid Research record shape while changing the exact Project
    // prefix that the selected task captured. Resume must reject before append.
    let research_path = root.path().join(".echidna/research.jsonl");
    let original_research = fs::read(&research_path).unwrap();
    let mut research_events = String::from_utf8(original_research.clone())
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    let hypothesis = research_events
        .iter_mut()
        .find(|event| {
            event["event_type"] == "research.hypothesis_recorded"
                && event["payload"]["data"]["record"]["id"] == "hypothesis-task-alpha"
        })
        .unwrap();
    hypothesis["payload"]["data"]["record"]["rationale"] =
        json!("COHERENTLY-CHANGED-PROJECT-SOURCE");
    ResearchEvent::decode(
        "research.hypothesis_recorded",
        hypothesis["payload"].clone(),
    )
    .unwrap();
    let forged_research = research_events
        .iter()
        .map(|event| format!("{}\n", serde_json::to_string(event).unwrap()))
        .collect::<String>();
    fs::write(&research_path, &forged_research).unwrap();
    let session_path = root
        .path()
        .join(".echidna/sessions")
        .join(&session)
        .join("events.jsonl");
    let before_rejected_resume = fs::read(&session_path).unwrap();
    let rejected = run_chat(root.path(), &config, "/exit\n", &["--resume", "latest"]);
    assert!(!rejected.status.success());
    assert!(
        String::from_utf8_lossy(&rejected.stderr).contains("source"),
        "{}",
        String::from_utf8_lossy(&rejected.stderr)
    );
    assert_eq!(fs::read(&session_path).unwrap(), before_rejected_resume);
    assert_eq!(fs::read_to_string(&research_path).unwrap(), forged_research);
    fs::write(research_path, original_research).unwrap();
}

fn return_alpha_for_task_summary(candidates: &Value) -> Value {
    let task = task_reference(candidates, "TASK-ALPHA");
    let references = selected_context(candidates, std::slice::from_ref(&task));
    let original = candidates["context"]
        .as_array()
        .unwrap()
        .iter()
        .find(|unit| unit["excerpt"] == TASK_A)
        .unwrap();
    let start = TASK_A.find("A_LIMIT_17").unwrap();
    json!({
        "relation":"return","focus":{"kind":"existing","task_ref":task},
        "related_task_refs":[],"required_context_refs":references,"unresolved_references":[],
        "constraint_updates":[{"source_ref":original["ref"],"start_byte":start,"end_byte":start+"A_LIMIT_17".len(),"scope":{"kind":"current_task"},"supersedes":null}]
    })
}

const ALPHA_SUMMARY: &str = "A-TASK-SUMMARY: Alpha's original constraint A_LIMIT_17 remains applicable. Consult retained original sources before drawing a current conclusion.";

fn continue_alpha_using_only_its_summary(candidates: &Value) -> Value {
    let task = task_reference(candidates, "TASK-ALPHA");
    let summaries = candidates["context"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|unit| unit["excerpt"].as_str().unwrap().contains("A-TASK-SUMMARY"))
        .collect::<Vec<_>>();
    assert_eq!(
        summaries.len(),
        1,
        "one source-bound Alpha summary candidate"
    );
    json!({
        "relation":"continue","focus":{"kind":"existing","task_ref":task},
        "related_task_refs":[],"required_context_refs":[summaries[0]["ref"]],"unresolved_references":[]
    })
}

#[test]
fn task_context_v7_compacts_noncontiguous_task_sources_and_delivers_only_selected_summary() {
    let root = TemporaryDirectory::new("task-context-v7-summary");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let alpha_body = format!("A-RAW-BODY:{}", "ALPHA-ORIGINAL-DETAIL ".repeat(3000));
    let server = spawn_scripted_responses_server(vec![
        ScriptedResponse::InitialSelection,
        ScriptedResponse::Response(200, text_response(&alpha_body, "summary-a-first")),
        ScriptedResponse::NewSelection,
        ScriptedResponse::Response(200, text_response(B_RESULT, "summary-b")),
        ScriptedResponse::SelectionDecision(return_alpha_for_task_summary),
        ScriptedResponse::Response(
            200,
            text_response(
                "A-RETURN-COMPLETED: the original constraint is unchanged.",
                "summary-a-return",
            ),
        ),
        ScriptedResponse::Response(
            200,
            compaction_summary_response(ALPHA_SUMMARY, "alpha-handoff"),
        ),
        ScriptedResponse::SelectionDecision(continue_alpha_using_only_its_summary),
        ScriptedResponse::SelectionDecision(continue_alpha_using_only_its_summary),
        ScriptedResponse::Response(
            200,
            text_response("ALPHA-SUMMARY-CONSUMED", "alpha-after-summary"),
        ),
    ]);
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let follow =
        "Continue TASK-ALPHA from its verified source handoff; retain the original constraint.";
    let output = run_chat(
        root.path(),
        &config,
        &format!("{TASK_A}\n{TASK_B}\n{RETURN_A}\n/compact\n{follow}\n/exit\n"),
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let wires = (0..10)
        .map(|_| f2a_request_body(&server.requests, "task-aware v7 compaction"))
        .collect::<Vec<_>>();
    assert!(server.requests.try_recv().is_err());
    server.join();
    let handoff = &wires[6];
    assert!(
        handoff
            .get("tools")
            .is_none_or(|tools| tools.as_array().unwrap().is_empty())
    );
    assert!(
        handoff
            .get("previous_response_id")
            .is_none_or(Value::is_null)
    );
    assert!(
        handoff["instructions"]
            .as_str()
            .unwrap()
            .contains("continuation handoff")
    );
    assert!(json_contains_text(&handoff["input"], "source_plan_hash"));
    assert!(json_contains_text(&handoff["input"], "A-RAW-BODY"));
    assert!(!json_contains_text(handoff, "B_ONLY_RESULT"));
    assert!(!json_contains_text(handoff, TASK_B));
    let initial_summary = selector_candidates(&wires[7]);
    let expanded_summary = selector_candidates(&wires[8]);
    let first = initial_summary["context"]
        .as_array()
        .unwrap()
        .iter()
        .find(|unit| unit["ref"].as_str().unwrap().starts_with("summary:"))
        .unwrap();
    let second = expanded_summary["context"]
        .as_array()
        .unwrap()
        .iter()
        .find(|unit| unit["ref"] == first["ref"])
        .unwrap();
    assert_eq!(first["truncated"], true);
    assert!(second["excerpt"].as_str().unwrap().len() > first["excerpt"].as_str().unwrap().len());
    assert!(json_contains_text(&wires[9], ALPHA_SUMMARY));
    assert!(json_contains_text(&wires[9], "A_LIMIT_17"));
    for omitted in [
        "A-RAW-BODY",
        "ALPHA-ORIGINAL-DETAIL",
        "B_ONLY_RESULT",
        TASK_B,
    ] {
        assert!(
            !json_contains_text(&wires[9], omitted),
            "summary delivery reintroduced omitted source body: {omitted}"
        );
    }
    let session = session_id(root.path());
    let state = reduced_session(&Project::discover(root.path()).unwrap(), &session);
    assert_eq!(state.request_audits().len(), 10);
    assert_eq!(
        state
            .latest_compaction()
            .unwrap()
            .summary()
            .schema_version(),
        7
    );
    assert!(state.history().items().contains(
        &echidna::domain::model::ContentItem::assistant_text(&alpha_body)
    ));
    assert!(state.history().items().contains(
        &echidna::domain::model::ContentItem::assistant_text(B_RESULT)
    ));
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
    let events = session_events(root.path(), &session);
    let summary = &events
        .iter()
        .rfind(|event| event["event_type"] == "session.compaction_created")
        .unwrap()["payload"]["summary"];
    let plan_path = root
        .path()
        .join(".echidna/artifacts/blobs/sha256")
        .join(summary["source_plan_blob_id"].as_str().unwrap());
    let original_plan = fs::read(&plan_path).unwrap();
    fs::write(&plan_path, b"corrupt task-aware v7 source plan").unwrap();
    let rejected = run_chat(root.path(), &config, "/exit\n", &["--resume", "latest"]);
    assert!(!rejected.status.success());
    assert_eq!(session_events(root.path(), &session), events);
    fs::write(plan_path, original_plan).unwrap();
}

#[test]
fn task_context_durable_boundaries_resume_without_adopting_prepared_or_replaying_work() {
    let root = TemporaryDirectory::new("task-context-crash-boundaries");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let server = spawn_scripted_responses_server(vec![
        ScriptedResponse::InitialSelection,
        ScriptedResponse::Response(
            200,
            m4_long_flow_tool_response(
                "crash-once",
                "run_command",
                json!({"argv":["/bin/sh","-c","printf 'executed\\n' >> crash-count.txt"]}),
                "crash-a-tool",
            ),
        ),
        ScriptedResponse::Response(200, text_response(A_RESULT, "crash-a-completed")),
        ScriptedResponse::NewSelection,
        ScriptedResponse::Response(200, text_response(B_RESULT, "crash-b-completed")),
    ]);
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        &format!("{TASK_A}\n{TASK_B}\n/exit\n"),
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for _ in 0..5 {
        f2a_request_body(&server.requests, "durable boundary original request");
    }
    assert!(server.requests.try_recv().is_err());
    server.join();
    let session = session_id(root.path());
    let events = session_events(root.path(), &session);
    let second_input = events
        .iter()
        .position(|event| {
            event["event_type"] == "session.history_item_recorded"
                && event["payload"]["item"]["text"] == TASK_B
        })
        .unwrap();
    let boundary_types = [
        "session.operator_turn_boundary_recorded",
        "session.task_context_captured",
        "session.request_audit_recorded",
        "session.task_selection_sample_recorded",
        "session.task_context_selected",
        "session.model_delivery_committed",
    ];
    let positions = events
        .iter()
        .enumerate()
        .skip(second_input)
        .filter(|(_, event)| boundary_types.contains(&event["event_type"].as_str().unwrap()))
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    assert_eq!(
        positions.len(),
        8,
        "accepted/capture/two audits/sample/selection/delivery/completed"
    );
    let event_path = root
        .path()
        .join(".echidna/sessions")
        .join(&session)
        .join("events.jsonl");
    let original = fs::read(&event_path).unwrap();
    for position in positions {
        let prefix = &events[..=position];
        let prefix_bytes = prefix
            .iter()
            .map(|event| format!("{}\n", serde_json::to_string(event).unwrap()))
            .collect::<String>();
        fs::write(&event_path, &prefix_bytes).unwrap();
        let resumed = run_chat(
            root.path(),
            &config,
            "/context\n/exit\n",
            &["--resume", "latest"],
        );
        assert!(
            resumed.status.success(),
            "boundary {} at {}: {}",
            events[position]["event_type"],
            position,
            String::from_utf8_lossy(&resumed.stderr)
        );
        assert!(
            fs::read(&event_path)
                .unwrap()
                .starts_with(prefix_bytes.as_bytes()),
            "Resume must preserve each original committed prefix"
        );
        let after = session_events(root.path(), &session);
        for kind in [
            "session.request_audit_recorded",
            "session.task_selection_sample_recorded",
            "session.task_context_selected",
            "session.model_delivery_committed",
            "tool.execution_outcome_recorded",
        ] {
            assert_eq!(
                after
                    .iter()
                    .filter(|event| event["event_type"] == kind)
                    .count(),
                prefix
                    .iter()
                    .filter(|event| event["event_type"] == kind)
                    .count(),
                "Resume created {kind} from prepared or unknown work at boundary {position}"
            );
        }
        assert_eq!(
            fs::read_to_string(root.path().join("crash-count.txt")).unwrap(),
            "executed\n"
        );
    }
    fs::write(event_path, original).unwrap();
}

#[test]
fn task_context_unknown_tool_effect_blocks_a_queued_task_and_is_not_replayed_on_resume() {
    let root = TemporaryDirectory::new("task-context-unknown-effect");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let server = spawn_scripted_responses_server(vec![
        ScriptedResponse::InitialSelection,
        ScriptedResponse::Response(
            200,
            m4_long_flow_tool_response(
                "unknown-effect-once",
                "run_command",
                json!({"argv":["/bin/sh","-c","printf 'started\\n' >> unknown-count.txt; exec /bin/sleep 30"]}),
                "accepted-before-unknown-effect",
            ),
        ),
    ]);
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let child = spawn_f2a_chat(
        root.path(),
        &config,
        &format!("{TASK_A}\n{TASK_B}\n/exit\n"),
        &["--agent", agent.to_str().unwrap()],
    );
    selector_candidates(&f2a_request_body(
        &server.requests,
        "unknown effect initial selection",
    ));
    f2a_request_body(&server.requests, "unknown effect original request");
    let counter = root.path().join("unknown-count.txt");
    let deadline = Instant::now() + F2A_WAIT;
    while !counter.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        counter.exists(),
        "the external mutation must precede interruption"
    );
    assert!(
        Command::new("/bin/kill")
            .args(["-INT", &child.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    let output = f2a_wait_child(child, "unknown effect before task switch");
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("external effects are unknown"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(server.requests.try_recv().is_err());
    server.join();
    let session = session_id(root.path());
    let events = session_events(root.path(), &session);
    assert_eq!(
        events
            .iter()
            .filter(|event| event["event_type"] == "session.task_context_selected")
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event["event_type"] == "session.task_selection_sample_recorded")
            .count(),
        1
    );
    let state = reduced_session(&Project::discover(root.path()).unwrap(), &session);
    assert_eq!(
        state.execution_receipt_count(),
        0,
        "unknown is not a fabricated terminal outcome"
    );
    assert!(state.history().ensure_all_tool_calls_paired().is_err());
    assert!(
        !state
            .history()
            .items()
            .contains(&echidna::domain::model::ContentItem::user_text(TASK_B))
    );
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
    let after = reduced_session(&Project::discover(root.path()).unwrap(), &session);
    assert_eq!(after.request_audits().len(), state.request_audits().len());
    assert_eq!(after.execution_receipt_count(), 0);
    assert_eq!(after.history().items(), state.history().items());
    assert_eq!(fs::read_to_string(counter).unwrap(), "started\n");
}

#[test]
fn task_context_cancelled_selection_keeps_prior_adoption_and_never_sends_main() {
    let root = TemporaryDirectory::new("task-context-cancelled-selection");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let initial = spawn_scripted_responses_server(vec![
        ScriptedResponse::InitialSelection,
        ScriptedResponse::Response(200, text_response(A_RESULT, "cancel-prior-main")),
    ]);
    let config = write_context_window_config(root.path(), &initial.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        &format!("{TASK_A}\n/exit\n"),
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    selector_candidates(&f2a_request_body(
        &initial.requests,
        "cancel fixture initial selector",
    ));
    f2a_request_body(&initial.requests, "cancel fixture initial main");
    initial.join();
    let session = session_id(root.path());
    let before = session_events(root.path(), &session);
    let original_selection = before
        .iter()
        .find(|event| event["event_type"] == "session.task_context_selected")
        .unwrap();
    let pending = spawn_pending_responses_server(false);
    let config = write_context_window_config(root.path(), &pending.base_url, 272_000);
    let child = spawn_f2a_chat(
        root.path(),
        &config,
        &format!("{TASK_B}\n/exit\n"),
        &["--resume", "latest"],
    );
    let wire = f2a_request_body(&pending.requests, "pending selector");
    let candidates = selector_candidates(&wire);
    assert!(candidates["input"].to_string().contains("TASK-BETA"));
    assert!(
        Command::new("/bin/kill")
            .args(["-INT", &child.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    let output = f2a_wait_child(child, "cancel task selection");
    pending.join();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("[request cancelled]"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let events = session_events(root.path(), &session);
    let selections = events
        .iter()
        .filter(|event| event["event_type"] == "session.task_context_selected")
        .collect::<Vec<_>>();
    assert_eq!(selections, vec![original_selection]);
    assert_eq!(
        events
            .iter()
            .filter(|event| event["event_type"] == "session.task_selection_sample_recorded")
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event["event_type"] == "session.model_delivery_committed")
            .count(),
        1
    );
    let state = reduced_session(&Project::discover(root.path()).unwrap(), &session);
    assert_eq!(
        state.request_audits().len(),
        3,
        "cancelled selector is an attempt, never a main acceptance"
    );
    assert_eq!(state.execution_receipt_count(), 0);
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
    let after = reduced_session(&Project::discover(root.path()).unwrap(), &session);
    assert_eq!(after.request_audits().len(), 3);
    assert_eq!(after.history().items(), state.history().items());
}

#[test]
fn task_context_read_reopens_same_agent_session_sources_and_excludes_other_scopes() {
    let root = TemporaryDirectory::new("task-context-cross-session");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let record_source =
        |project_root: &std::path::Path, definition: &std::path::Path, content: &str| {
            let before = session_ids(project_root);
            let server = spawn_scripted_responses_server(vec![
                ScriptedResponse::NewSelection,
                ScriptedResponse::Response(200, text_response(content, "stored-context-source")),
            ]);
            let config = write_context_window_config(project_root, &server.base_url, 272_000);
            let output = run_chat(
                project_root,
                &config,
                "Record the synthetic source.\n/exit\n",
                &["--agent", definition.to_str().unwrap()],
            );
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            selector_candidates(&f2a_request_body(
                &server.requests,
                "cross-Session source selector",
            ));
            f2a_request_body(&server.requests, "cross-Session source setup");
            assert!(server.requests.try_recv().is_err());
            server.join();
            let session = session_ids(project_root)
                .into_iter()
                .find(|id| !before.contains(id))
                .unwrap();
            let events = session_events(project_root, &session);
            let source = events
                .iter()
                .find(|event| {
                    event["event_type"] == "session.history_item_recorded"
                        && event["payload"]["item"]["text"] == content
                })
                .unwrap();
            (session, source["event_id"].as_str().unwrap().to_owned())
        };
    let original =
        "SCOPE-LOOKUP-METHOD: require volume ratio 7.25; preserve exact units and source revision.";
    let (prior_session, prior_event) = record_source(root.path(), &agent, original);
    let manifest = agent.join("agent.toml");
    let original_manifest = fs::read_to_string(&manifest).unwrap();
    fs::write(
        &manifest,
        original_manifest.replace("id = \"task-context-fixture\"", "id = \"different-agent\""),
    )
    .unwrap();
    let (foreign_session, foreign_event) =
        record_source(root.path(), &agent, "SCOPE-LOOKUP-METHOD: OTHER-AGENT-ONLY");
    fs::write(&manifest, &original_manifest).unwrap();
    let other_project = TemporaryDirectory::new("task-context-other-project");
    initialize_project(other_project.path());
    let other_definition = definition(other_project.path(), None);
    record_source(
        other_project.path(),
        &other_definition,
        "SCOPE-LOOKUP-METHOD: OTHER-PROJECT-ONLY",
    );
    let reference = format!("session:{prior_session}:{prior_event}");
    let foreign_reference = format!("session:{foreign_session}:{foreign_event}");
    let source_revision = blake3::hash(
        &serde_json::to_vec(&echidna::domain::model::ContentItem::assistant_text(
            original,
        ))
        .unwrap(),
    )
    .to_hex()
    .to_string();
    let searched_reference =
        format!("session-revision:{prior_session}:{prior_event}:{source_revision}");
    let offset = original.find("volume").unwrap();
    let server = spawn_scripted_responses_server(vec![
        ScriptedResponse::NewSelection,
        ScriptedResponse::Response(
            200,
            m4_long_flow_tool_response(
                "scoped-search",
                "context_read",
                json!({"operation":"search","query":"SCOPE-LOOKUP-METHOD","max_bytes":4096}),
                "cross-search",
            ),
        ),
        ScriptedResponse::Response(
            200,
            m4_long_flow_tool_response(
                "scoped-read",
                "context_read",
                json!({"operation":"read","reference":reference,"offset":offset,"max_bytes":18}),
                "cross-read",
            ),
        ),
        ScriptedResponse::Response(
            200,
            m4_long_flow_tool_response(
                "foreign-read",
                "context_read",
                json!({"operation":"read","reference":foreign_reference,"max_bytes":4096}),
                "cross-foreign",
            ),
        ),
        ScriptedResponse::Response(
            200,
            text_response("SCOPED-COLD-READ-COMPLETE", "cross-final"),
        ),
    ]);
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let before = session_ids(root.path());
    let output = run_chat(
        root.path(),
        &config,
        "Execute the bounded fixture reads.\n/exit\n",
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let wires = (0..5)
        .map(|_| f2a_request_body(&server.requests, "scoped cold read"))
        .collect::<Vec<_>>();
    assert!(server.requests.try_recv().is_err());
    server.join();
    let result = |wire: &Value, call: &str| {
        let item = wire["input"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["type"] == "function_call_output" && item["call_id"] == call)
            .unwrap();
        serde_json::from_str::<Value>(item["output"].as_str().unwrap()).unwrap()["output"].clone()
    };
    let search = result(&wires[2], "scoped-search");
    assert_eq!(search["status"], "found");
    let entries = search["entries"].as_array().unwrap();
    assert_eq!(
        entries.len(),
        1,
        "only same-Project, same-durable-Agent history is eligible"
    );
    assert_eq!(entries[0]["reference"], searched_reference);
    assert_eq!(entries[0]["source_revision"], source_revision);
    assert_eq!(entries[0]["text"], original);
    assert_eq!(entries[0]["source_role"], "assistant");
    assert_eq!(entries[0]["source_origin"], "model_output");
    assert_eq!(entries[0]["source_authority"], "model_observation");
    let read = result(&wires[3], "scoped-read");
    assert_eq!(read["status"], "found");
    assert_eq!(read["entries"][0]["reference"], reference);
    assert_eq!(read["entries"][0]["text"], &original[offset..offset + 18]);
    assert_eq!(read["entries"][0]["next_offset"], offset + 18);
    assert_eq!(
        read["entries"][0]["source_revision"],
        entries[0]["source_revision"]
    );
    assert_eq!(result(&wires[4], "foreign-read")["status"], "not_found");
    for wire in &wires {
        assert!(!wire.to_string().contains("OTHER-AGENT-ONLY"));
        assert!(!wire.to_string().contains("OTHER-PROJECT-ONLY"));
    }
    assert_eq!(wires[3]["previous_response_id"], "cross-read");
    assert_eq!(wires[4]["previous_response_id"], "cross-foreign");
    let session = session_ids(root.path())
        .into_iter()
        .find(|id| !before.contains(id))
        .unwrap();
    let state = reduced_session(&Project::discover(root.path()).unwrap(), &session);
    assert_eq!(state.request_audits().len(), 5);
    assert_eq!(state.execution_receipt_count(), 3);
    assert_eq!(fs::read_to_string(manifest).unwrap(), original_manifest);
}

#[test]
fn task_context_artifact_read_verifies_published_scope_revision_ranges_and_storage_failure() {
    use echidna::domain::model::ContentItem;
    use echidna::domain::tools::ToolResultStatus;
    let root = TemporaryDirectory::new("task-context-artifact-source");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let manifest = agent.join("agent.toml");
    let original_manifest = format!(
        "{}artifact_store = \"direct\"\n",
        fs::read_to_string(&manifest).unwrap()
    );
    fs::write(&manifest, &original_manifest).unwrap();
    let content = "ARTIFACT-EXACT-SOURCE: quantity 17.25 units; original UTF-8 证据。";
    fs::write(root.path().join("locatorzxqj907531.bin"), content).unwrap();
    let publisher = spawn_scripted_responses_server(vec![
        ScriptedResponse::NewSelection,
        ScriptedResponse::Response(
            200,
            m4_long_flow_tool_response(
                "publish-source-once",
                "artifact_store",
                json!({
                    "event_id":"task-context-artifact-created", "occurred_at":"2026-11-05T00:00:00Z",
                    "artifact_id":"task-context-source-artifact", "source_path":"locatorzxqj907531.bin",
                    "media_type":"text/plain", "toolchain_fingerprint":"synthetic-fixture-v1"
                }),
                "artifact-published",
            ),
        ),
        ScriptedResponse::Response(
            200,
            text_response("SOURCE-PUBLISHED", "artifact-publisher-final"),
        ),
    ]);
    let config = write_context_window_config(root.path(), &publisher.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "Publish the synthetic source once.\n/exit\n",
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for _ in 0..3 {
        f2a_request_body(&publisher.requests, "Artifact publication");
    }
    assert!(publisher.requests.try_recv().is_err());
    publisher.join();
    let producer = session_id(root.path());
    let state = reduced_session(&Project::discover(root.path()).unwrap(), &producer);
    assert_eq!(state.execution_receipt_count(), 1);
    let published = state
        .history()
        .items()
        .iter()
        .find_map(|item| match item {
            ContentItem::ToolResult { result }
                if result.call_id.as_str() == "publish-source-once" =>
            {
                Some(result)
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(published.status, ToolResultStatus::Succeeded);
    assert_eq!(
        published.output["artifact"]["scope"],
        json!({"kind":"session","session_id":producer})
    );
    let blob = published.output["blob"]["blob_id"].as_str().unwrap();
    let reference = format!("artifact:task-context-source-artifact:{blob}");
    let metadata: echidna::domain::artifacts::ArtifactRecord =
        serde_json::from_value(published.output["artifact"].clone()).unwrap();
    let metadata_text = serde_json::to_string(&metadata).unwrap();
    let metadata_revision = blake3::hash(metadata_text.as_bytes()).to_hex().to_string();
    let metadata_reference =
        format!("artifact-metadata:task-context-source-artifact:{blob}:{metadata_revision}");
    let source_stream = root.path().join(".echidna/research.jsonl");
    let original_project = fs::read(&source_stream).unwrap();
    let producer_path = root
        .path()
        .join(".echidna/sessions")
        .join(&producer)
        .join("events.jsonl");
    let original_producer = fs::read(&producer_path).unwrap();
    let offset = content.find("quantity").unwrap();
    let read = |request: Value| {
        let before = session_ids(root.path());
        let server = spawn_scripted_responses_server(vec![
            ScriptedResponse::NewSelection,
            ScriptedResponse::Response(
                200,
                m4_long_flow_tool_response(
                    "read-published-source",
                    "context_read",
                    request,
                    "artifact-read",
                ),
            ),
            ScriptedResponse::Response(200, text_response("READ-OBSERVED", "artifact-read-final")),
        ]);
        let config = write_context_window_config(root.path(), &server.base_url, 272_000);
        let output = run_chat(
            root.path(),
            &config,
            "Perform the bounded fixture read.\n/exit\n",
            &["--agent", agent.to_str().unwrap()],
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        for _ in 0..3 {
            f2a_request_body(&server.requests, "Artifact context read");
        }
        assert!(server.requests.try_recv().is_err());
        server.join();
        let session = session_ids(root.path())
            .into_iter()
            .find(|id| !before.contains(id))
            .unwrap();
        let state = reduced_session(&Project::discover(root.path()).unwrap(), &session);
        assert_eq!(state.execution_receipt_count(), 1);
        assert_eq!(state.request_audits().len(), 3);
        state
            .history()
            .items()
            .iter()
            .find_map(|item| match item {
                ContentItem::ToolResult { result }
                    if result.call_id.as_str() == "read-published-source" =>
                {
                    Some(result.clone())
                }
                _ => None,
            })
            .unwrap()
    };
    // The unique locator occurs only in metadata. Body-search coverage uses
    // separate queries, so fuzzy matching cannot satisfy this metadata oracle.
    let search =
        read(json!({"operation":"search","query":"locatorzxqj907531.bin","max_bytes":512}));
    assert_eq!(search.status, ToolResultStatus::Succeeded, "{search:?}");
    assert_eq!(search.output["status"], "found");
    let discovered = search.output["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["reference"] == metadata_reference)
        .expect("published Artifact metadata is searchable");
    assert_eq!(discovered["scope"], "artifact_metadata");
    assert_eq!(discovered["source_revision"], metadata_revision);
    let metadata_offset = discovered["offset"].as_u64().unwrap() as usize;
    let metadata_excerpt = discovered["text"].as_str().unwrap();
    assert_eq!(
        metadata_excerpt,
        &metadata_text[metadata_offset..metadata_offset + metadata_excerpt.len()]
    );
    let metadata_read = read(
        json!({"operation":"read","reference":discovered["reference"],"offset":0,"max_bytes":64}),
    );
    assert_eq!(metadata_read.status, ToolResultStatus::Succeeded);
    assert_eq!(metadata_read.output["status"], "found");
    let metadata_entry = &metadata_read.output["entries"][0];
    assert_eq!(metadata_entry["reference"], metadata_reference);
    assert_eq!(metadata_entry["source_revision"], metadata_revision);
    assert_eq!(metadata_entry["text"], &metadata_text[..64]);
    assert_eq!(metadata_entry["total_bytes"], metadata_text.len());
    assert_eq!(metadata_entry["next_offset"], 64);
    // The body is a separate representation with the existing Blob reference.
    let found =
        read(json!({"operation":"read","reference":reference,"offset":offset,"max_bytes":14}));
    assert_eq!(found.status, ToolResultStatus::Succeeded);
    assert_eq!(found.output["status"], "found");
    let entry = &found.output["entries"][0];
    assert_eq!(entry["reference"], reference);
    assert_eq!(entry["source_revision"], blob);
    assert_eq!(entry["text"], &content[offset..offset + 14]);
    assert_eq!(entry["next_offset"], offset + 14);
    assert_eq!(entry["total_bytes"], content.len());
    assert!(entry["source_role"].is_null());
    assert_eq!(entry["source_origin"], "artifact_projection");
    assert_eq!(entry["source_authority"], "fact_reference");
    let stale = read(
        json!({"operation":"read","reference":format!("artifact:task-context-source-artifact:{}", "0".repeat(64)),"offset":offset,"max_bytes":14}),
    );
    assert_eq!(stale.status, ToolResultStatus::Succeeded);
    assert_eq!(stale.output["status"], "stale");
    let blob_path = root
        .path()
        .join(".echidna/artifacts/blobs/sha256")
        .join(blob);
    let original_blob = fs::read(&blob_path).unwrap();
    fs::write(&blob_path, b"corrupted exact Artifact source").unwrap();
    let damaged =
        read(json!({"operation":"read","reference":reference,"offset":offset,"max_bytes":14}));
    assert_eq!(
        damaged.status,
        ToolResultStatus::Failed,
        "storage corruption must remain an execution failure"
    );
    assert!(
        damaged.output["status"].is_null(),
        "failure cannot become found/not_found/stale success"
    );
    assert!(
        damaged.output["error"]
            .as_str()
            .is_some_and(|message| !message.is_empty())
    );
    fs::write(&blob_path, original_blob).unwrap();
    fs::write(
        &manifest,
        original_manifest.replace("id = \"task-context-fixture\"", "id = \"different-agent\""),
    )
    .unwrap();
    let foreign_search =
        read(json!({"operation":"search","query":"locatorzxqj907531.bin","max_bytes":512}));
    assert_eq!(foreign_search.status, ToolResultStatus::Succeeded);
    assert_eq!(foreign_search.output["status"], "not_found");
    let foreign =
        read(json!({"operation":"read","reference":reference,"offset":offset,"max_bytes":14}));
    assert_eq!(foreign.status, ToolResultStatus::Succeeded);
    assert_eq!(foreign.output["status"], "not_found");
    assert!(foreign.output["entries"].as_array().unwrap().is_empty());
    fs::write(&manifest, original_manifest).unwrap();
    assert_eq!(fs::read(producer_path).unwrap(), original_producer);
    assert_eq!(
        fs::read(source_stream).unwrap(),
        original_project,
        "readback cannot republish Artifact or Research facts"
    );
}

#[test]
fn task_context_historical_memory_requires_valid_revision_chain_and_blob() {
    use echidna::domain::model::ContentItem;
    use echidna::domain::tools::ToolResultStatus;
    let root = TemporaryDirectory::new("task-context-historical-memory");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let manifest = agent.join("agent.toml");
    fs::write(
        &manifest,
        format!(
            "{}memory_write = \"direct\"\n",
            fs::read_to_string(&manifest)
                .unwrap()
                .replace("memory = \"none\"", "memory = \"agent\"")
        ),
    )
    .unwrap();
    let old = "HISTORICAL-METHOD: exact threshold 17.25; applicable only to fixture A.";
    let current = "CURRENT-METHOD: exact threshold 23.50; revised for fixture B.";
    let old_hash = blake3::hash(old.as_bytes()).to_hex().to_string();
    let current_hash = blake3::hash(current.as_bytes()).to_hex().to_string();
    let publisher = spawn_scripted_responses_server(vec![
        ScriptedResponse::NewSelection,
        ScriptedResponse::Response(
            200,
            m4_long_flow_tool_response(
                "write-old-method",
                "memory_write",
                json!({
                    "path":"notes/method.md", "content":old, "expected_hash":null, "event_id":"method-original"
                }),
                "memory-original",
            ),
        ),
        ScriptedResponse::Response(
            200,
            m4_long_flow_tool_response(
                "revise-method",
                "memory_write",
                json!({
                    "path":"notes/method.md", "content":current, "expected_hash":old_hash, "event_id":"method-revised"
                }),
                "memory-revised",
            ),
        ),
        ScriptedResponse::Response(
            200,
            text_response("METHOD-REVISIONS-PUBLISHED", "memory-final"),
        ),
    ]);
    let config = write_context_window_config(root.path(), &publisher.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "Write and revise a synthetic method.\n/exit\n",
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for _ in 0..4 {
        f2a_request_body(&publisher.requests, "Memory revision publication");
    }
    assert!(publisher.requests.try_recv().is_err());
    publisher.join();
    let producer = session_id(root.path());
    let state = reduced_session(&Project::discover(root.path()).unwrap(), &producer);
    assert_eq!(state.execution_receipt_count(), 2);
    assert_eq!(state.request_audits().len(), 4);
    for item in state.history().items() {
        if let ContentItem::ToolResult { result } = item {
            assert_eq!(result.status, ToolResultStatus::Succeeded);
        }
    }
    let memory_path = root
        .path()
        .join(".echidna/memory/agents/task-context-fixture/notes/method.md");
    assert_eq!(fs::read_to_string(&memory_path).unwrap(), current);
    let stream_path = root.path().join(".echidna/research.jsonl");
    let original_stream = fs::read_to_string(&stream_path).unwrap();
    let events = original_stream
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    let old_event = events
        .iter()
        .find(|event| event["event_id"] == "method-original")
        .unwrap();
    let old_blob = old_event["payload"]["data"]["data"]["revision"]["new_blob_id"]
        .as_str()
        .unwrap();
    let reference = format!("memory:notes/method.md:{old_hash}");
    let offset = old.find("threshold").unwrap();
    let read = |requests: Vec<(&str, Value)>| {
        let before = session_ids(root.path());
        let calls = requests
            .iter()
            .map(|(id, args)| (*id, "context_read", args.clone()))
            .collect::<Vec<_>>();
        let server = spawn_scripted_responses_server(vec![
            ScriptedResponse::InitialSelection,
            ScriptedResponse::Response(200, tool_calls_response(&calls, "memory-read")),
            ScriptedResponse::Response(
                200,
                text_response("MEMORY-READ-OBSERVED", "memory-read-final"),
            ),
        ]);
        let config = write_context_window_config(root.path(), &server.base_url, 272_000);
        let output = run_chat(
            root.path(),
            &config,
            "Read the requested exact method sources.\n/exit\n",
            &["--agent", agent.to_str().unwrap()],
        );
        assert!(
            output.status.success(),
            "stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        for _ in 0..3 {
            f2a_request_body(&server.requests, "historical Memory read");
        }
        assert!(server.requests.try_recv().is_err());
        server.join();
        let session = session_ids(root.path())
            .into_iter()
            .find(|id| !before.contains(id))
            .unwrap();
        let state = reduced_session(&Project::discover(root.path()).unwrap(), &session);
        assert_eq!(state.execution_receipt_count(), requests.len() as u64);
        state
            .history()
            .items()
            .iter()
            .filter_map(|item| match item {
                ContentItem::ToolResult { result } => {
                    Some((result.call_id.as_str().to_owned(), result.clone()))
                }
                _ => None,
            })
            .collect::<BTreeMap<_, _>>()
    };
    let old_request =
        json!({"operation":"read","reference":reference,"offset":offset,"max_bytes":15});
    let found = read(vec![
        ("old", old_request.clone()),
        (
            "current",
            json!({"operation":"read","reference":format!("memory:notes/method.md:{current_hash}"),"max_bytes":256}),
        ),
        (
            "search",
            json!({"operation":"search","query":"CURRENT-METHOD","max_bytes":256}),
        ),
    ]);
    assert_eq!(found["old"].status, ToolResultStatus::Succeeded);
    assert_eq!(found["old"].output["status"], "found");
    let entry = &found["old"].output["entries"][0];
    assert_eq!(entry["reference"], reference);
    assert_eq!(entry["source_revision"], old_hash);
    assert_eq!(entry["text"], &old[offset..offset + 15]);
    assert_eq!(entry["next_offset"], offset + 15);
    assert_eq!(entry["total_bytes"], old.len());
    assert_eq!(entry["source_origin"], "project_memory");
    assert_eq!(entry["source_authority"], "derived_non_evidence");
    assert!(entry["valid_time"].is_null());
    assert!(entry["observed_at"].is_string());
    assert_eq!(found["current"].output["entries"][0]["text"], current);
    assert!(
        found["search"].output["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["reference"] == format!("memory:notes/method.md:{current_hash}"))
    );

    let mut damaged_events = events.clone();
    let revision = &mut damaged_events
        .iter_mut()
        .find(|event| event["event_id"] == "method-revised")
        .unwrap()["payload"]["data"]["data"]["revision"];
    revision["old_hash"] = json!("0".repeat(64));
    fs::write(
        &stream_path,
        damaged_events
            .iter()
            .map(|event| format!("{event}\n"))
            .collect::<String>(),
    )
    .unwrap();
    let bad_chain = read(vec![("bad-chain", old_request.clone())]);
    assert_eq!(bad_chain["bad-chain"].status, ToolResultStatus::Failed);
    assert!(bad_chain["bad-chain"].output["status"].is_null());
    assert!(
        bad_chain["bad-chain"].output["error"]
            .as_str()
            .unwrap()
            .contains("revision chain")
    );
    fs::write(&stream_path, &original_stream).unwrap();

    let blob_path = root
        .path()
        .join(".echidna/artifacts/blobs/sha256")
        .join(old_blob);
    let blob_bytes = fs::read(&blob_path).unwrap();
    fs::write(&blob_path, b"invalid old Memory Blob bytes").unwrap();
    let bad_blob = read(vec![("bad-blob", old_request)]);
    assert_eq!(bad_blob["bad-blob"].status, ToolResultStatus::Failed);
    assert!(bad_blob["bad-blob"].output["status"].is_null());
    assert!(
        bad_blob["bad-blob"].output["error"]
            .as_str()
            .is_some_and(|error| !error.is_empty())
    );
    fs::write(&blob_path, blob_bytes).unwrap();
    assert_eq!(fs::read_to_string(memory_path).unwrap(), current);
    assert_eq!(fs::read_to_string(stream_path).unwrap(), original_stream);
}

#[test]
fn task_context_progress_references_real_failures_results_and_never_replays_them() {
    use echidna::domain::model::ContentItem;
    let root = TemporaryDirectory::new("task-context-progress-sources");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let mut task = String::new();
    let server = super::agent_evolution::server_with_wait(
        move |index, wire| {
            if index == 0 {
                return initial_selection_response(wire, "progress-initial-selection");
            }
            match index - 1 {
                0 => m4_long_flow_tool_response(
                    "failed-progress-attempt",
                    "run_command",
                    json!({"argv":["/bin/sh","-c","printf 'REAL-FAILED-ATTEMPT'; exit 7"]}),
                    "progress-failed",
                ),
                1 => m4_long_flow_tool_response(
                    "successful-progress-attempt",
                    "run_command",
                    json!({"argv":["/bin/sh","-c","printf 'executed\\n' >> progress-counter.txt; printf 'REAL-SUCCESS-OBSERVATION'"]}),
                    "progress-success",
                ),
                2 => text_response("ALPHA-ATTEMPTS-RECORDED", "progress-a-final"),
                3 => {
                    let candidates = selector_candidates(wire);
                    task = task_reference(&candidates, "TASK-ALPHA");
                    text_response(&json!({"relation":"new","focus":{"kind":"new_from_input","input_ref":candidates["input"]["ref"]},
                "related_task_refs":[],"required_context_refs":[],"unresolved_references":[]}).to_string(),"progress-selection")
                }
                4 => tool_calls_response(
                    &[
                        (
                            "read-task-progress",
                            "context_read",
                            json!({"operation":"progress","task_ref":task,"max_bytes":16384}),
                        ),
                        (
                            "read-missing-progress",
                            "context_read",
                            json!({"operation":"progress","task_ref":"task:nonexistent","max_bytes":128}),
                        ),
                    ],
                    "progress-read",
                ),
                5 => {
                    let result = wire["input"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|item| {
                            item["type"] == "function_call_output"
                                && item["call_id"] == "read-task-progress"
                        })
                        .unwrap();
                    let result: Value =
                        serde_json::from_str(result["output"].as_str().unwrap()).unwrap();
                    let reference = result["output"]["entries"][0]["reference"]
                        .as_str()
                        .unwrap();
                    let stale = format!(
                        "{}:{}",
                        reference.rsplit_once(':').unwrap().0,
                        "0".repeat(64)
                    );
                    tool_calls_response(
                        &[
                            (
                                "read-progress-range",
                                "context_read",
                                json!({"operation":"read","reference":reference,"offset":16,"max_bytes":31}),
                            ),
                            (
                                "read-progress-stale",
                                "context_read",
                                json!({"operation":"read","reference":stale,"max_bytes":128}),
                            ),
                        ],
                        "progress-range",
                    )
                }
                6 => text_response("PROGRESS-READ-ONLY", "progress-b-final"),
                _ => unreachable!(),
            }
        },
        8,
        F2A_WAIT,
    );
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "TASK-ALPHA: record a failed attempt and one successful mutation.\nTASK-BETA: read the earlier task progress without repeating its commands.\n/exit\n",
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    selector_candidates(&f2a_request_body(
        &server.requests,
        "progress initial selection",
    ));
    let wires = (0..7)
        .map(|_| f2a_request_body(&server.requests, "source-bound progress"))
        .collect::<Vec<_>>();
    assert!(server.requests.try_recv().is_err());
    server.join();
    assert!(!wires[4].to_string().contains("REAL-FAILED-ATTEMPT"));
    assert!(!wires[4].to_string().contains("REAL-SUCCESS-OBSERVATION"));
    assert_eq!(wires[5]["previous_response_id"], "progress-read");
    assert_eq!(wires[6]["previous_response_id"], "progress-range");
    assert_eq!(
        fs::read_to_string(root.path().join("progress-counter.txt")).unwrap(),
        "executed\n"
    );
    let session = session_id(root.path());
    let project = Project::discover(root.path()).unwrap();
    let state = reduced_session(&project, &session);
    assert_eq!(state.execution_receipt_count(), 6);
    let results = state
        .history()
        .items()
        .iter()
        .filter_map(|item| match item {
            ContentItem::ToolResult { result } => Some((result.call_id.as_str(), result)),
            _ => None,
        })
        .collect::<BTreeMap<_, _>>();
    let progress = &results["read-task-progress"].output;
    assert_eq!(progress["status"], "found");
    let entry = &progress["entries"][0];
    assert_eq!(entry["scope"], "task_progress");
    assert_eq!(entry["source_authority"], "derived_non_evidence");
    assert_eq!(entry["truncated"], false);
    let range = &results["read-progress-range"].output;
    assert_eq!(range["status"], "found");
    assert_eq!(
        range["entries"][0]["text"],
        &entry["text"].as_str().unwrap()[16..47]
    );
    assert_eq!(range["entries"][0]["reference"], entry["reference"]);
    assert_eq!(range["entries"][0]["next_offset"], 47);
    assert_eq!(results["read-progress-stale"].output["status"], "stale");
    let body: Value = serde_json::from_str(entry["text"].as_str().unwrap()).unwrap();
    let events = session_events(root.path(), &session);
    let initial = events
        .iter()
        .find(|event| event["event_type"] == "session.task_context_selected")
        .unwrap();
    let selected = events
        .iter()
        .find(|event| {
            event["event_type"] == "session.task_context_selected"
                && event["event_id"] == body["selection_event_ref"]
        })
        .unwrap();
    assert_eq!(body["input_ref"], initial["payload"]["input_ref"]);
    assert_eq!(
        body["task"]["task_id"],
        initial["payload"]["task_revisions"][0]["task_id"]
    );
    let mut cursor = selected;
    while cursor["event_id"] != initial["event_id"] {
        assert!(cursor["payload"]["sample_event_id"].is_null());
        assert_eq!(
            cursor["payload"]["input_ref"],
            initial["payload"]["input_ref"]
        );
        assert_eq!(
            cursor["payload"]["task_revisions"],
            initial["payload"]["task_revisions"]
        );
        let parent = events
            .iter()
            .find(|event| event["event_id"] == cursor["payload"]["refresh_of"])
            .unwrap();
        assert!(parent["sequence"].as_u64().unwrap() < cursor["sequence"].as_u64().unwrap());
        cursor = parent;
    }
    assert_eq!(
        events
            .iter()
            .filter(|event| event["event_type"] == "session.task_selection_sample_recorded")
            .count(),
        2,
        "each natural input has one selector; source refresh adds none"
    );
    let observations = body["observations"].as_array().unwrap();
    let receipts = observations
        .iter()
        .filter_map(|observation| observation["receipt_event_ref"].as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        receipts.len(),
        2,
        "both real failed/successful executions retain their receipt sources"
    );
    for observation in observations {
        assert!(
            events
                .iter()
                .any(|event| event["event_id"] == observation["source_event_ref"])
        );
    }
    for receipt in receipts {
        assert!(events.iter().any(|event| event["event_id"] == receipt
            && event["event_type"] == "tool.execution_outcome_recorded"));
    }
    assert!(body.to_string().contains("REAL-FAILED-ATTEMPT"));
    assert!(body.to_string().contains("REAL-SUCCESS-OBSERVATION"));
    assert_eq!(body["omitted_observations"], 0);
    assert_eq!(
        results["read-missing-progress"].output["status"],
        "not_found"
    );
    let resumed = run_chat(root.path(), &config, "/exit\n", &["--resume", "latest"]);
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    assert_eq!(
        fs::read_to_string(root.path().join("progress-counter.txt")).unwrap(),
        "executed\n"
    );
    assert_eq!(
        reduced_session(&project, &session).execution_receipt_count(),
        6
    );
}
