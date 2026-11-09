//! Source identity and repeated handoffs remain verifiable through Resume.
use super::task_context::{ScriptedResponse, definition, spawn_scripted_responses_server};
use super::*;

const ORIGINAL: &str = "Investigate the synthetic report; use red.";
const CORRECTION: &str = "Use blue instead.";
const OBSERVATION: &str = "red team report found";
const GAP: &str = "red team report still needs independent verification";

fn record_constraint_and_gap(candidates: &Value) -> Value {
    let source = candidates["context"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["excerpt"] == ORIGINAL)
        .unwrap();
    let start = ORIGINAL.find("red").unwrap();
    json!({"relation":"continue", "focus":{"kind":"existing", "task_ref":candidates["tasks"][0]["ref"]},
        "related_task_refs":[], "required_context_refs":[], "unresolved_references":[GAP],
        "constraint_updates":[{"source_ref":source["ref"], "start_byte":start, "end_byte":start+3,
            "scope":{"kind":"current_task"}, "supersedes":null}]})
}

fn correct_constraint(candidates: &Value) -> Value {
    let old = candidates["context"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["excerpt"] == "red" && entry["constraint_range"].is_object())
        .unwrap();
    json!({"relation":"continue", "focus":{"kind":"existing", "task_ref":candidates["tasks"][0]["ref"]},
        "related_task_refs":[], "required_context_refs":[], "unresolved_references":[],
        "constraint_updates":[{"source_ref":candidates["input"]["ref"], "start_byte":0, "end_byte":CORRECTION.len(),
            "scope":{"kind":"current_task"}, "supersedes":old["ref"]}]})
}

fn continue_task(candidates: &Value) -> Value {
    json!({"relation":"continue", "focus":{"kind":"existing", "task_ref":candidates["tasks"][0]["ref"]},
        "related_task_refs":[], "required_context_refs":[], "unresolved_references":[]})
}

fn duplicate_core_sections(candidates: &Value) -> Vec<Value> {
    let mut sections = candidates["context"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|candidate| candidate["core_range"].is_object())
        .cloned()
        .collect::<Vec<_>>();
    sections.sort_by_key(|candidate| candidate["core_range"]["start_byte"].as_u64().unwrap());
    assert_eq!(sections.len(), 2);
    assert_eq!(
        sections[0]["core_range"]["content_hash"],
        sections[1]["core_range"]["content_hash"]
    );
    sections
}

fn initial_duplicate_core(candidates: &Value) -> Value {
    let sections = duplicate_core_sections(candidates);
    json!({"relation":"new","focus":{"kind":"new_from_input","input_ref":candidates["input"]["ref"]},"related_task_refs":[],"required_context_refs":[],"unresolved_references":[],
        "scope_classifications":[{"context_ref":sections[0]["ref"],"scope":{"kind":"current_task"}}]})
}

fn second_duplicate_core(candidates: &Value) -> Value {
    let sections = duplicate_core_sections(candidates);
    json!({"relation":"new","focus":{"kind":"new_from_input","input_ref":candidates["input"]["ref"]},"related_task_refs":[],"required_context_refs":[],"unresolved_references":[],
        "scope_classifications":[{"context_ref":sections[1]["ref"],"scope":{"kind":"current_task"}}]})
}

fn return_with_duplicate_dependency(candidates: &Value) -> Value {
    let sections = duplicate_core_sections(candidates);
    let alpha = candidates["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|task| task["goal"] == "DUPLICATE-CORE-TASK-A")
        .unwrap();
    json!({"relation":"return","focus":{"kind":"existing","task_ref":alpha["ref"]},"related_task_refs":[],"required_context_refs":[sections[1]["ref"]],"unresolved_references":[]})
}

fn continue_duplicate_a(candidates: &Value) -> Value {
    let alpha = candidates["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|task| task["goal"] == "DUPLICATE-CORE-TASK-A")
        .unwrap();
    json!({"relation":"continue","focus":{"kind":"existing","task_ref":alpha["ref"]},
        "related_task_refs":[],"required_context_refs":[],"unresolved_references":[]})
}

const OLD_METHOD_MARKER: &str = "EXCERPTPINUNIQUE987";

fn request_old_method(candidates: &Value) -> Value {
    assert!(
        !candidates["context"]
            .as_array()
            .unwrap()
            .iter()
            .any(|unit| {
                unit["excerpt"]
                    .as_str()
                    .is_some_and(|text| text.contains(OLD_METHOD_MARKER))
            })
    );
    json!({"relation":"new", "focus":{"kind":"new_from_input","input_ref":candidates["input"]["ref"]},
        "related_task_refs":[], "required_context_refs":[], "unresolved_references":[OLD_METHOD_MARKER]})
}

fn select_old_method(candidates: &Value) -> Value {
    let source = candidates["context"]
        .as_array()
        .unwrap()
        .iter()
        .find(|unit| {
            unit["excerpt"]
                .as_str()
                .is_some_and(|text| text.contains(OLD_METHOD_MARKER))
        })
        .expect("expanded candidates contain the old method");
    let start = source["excerpt"]
        .as_str()
        .unwrap()
        .find(OLD_METHOD_MARKER)
        .unwrap();
    let tasks = candidates["tasks"].as_array().unwrap();
    let (relation, focus) = if tasks.is_empty() {
        (
            "new",
            json!({"kind":"new_from_input", "input_ref":candidates["input"]["ref"]}),
        )
    } else {
        (
            "continue",
            json!({"kind":"existing", "task_ref":tasks[0]["ref"]}),
        )
    };
    json!({"relation":relation, "focus":focus, "related_task_refs":[], "required_context_refs":[],
        "unresolved_references":[], "source_ranges":[{"context_ref":source["ref"],
            "start_byte":start, "end_byte":start+OLD_METHOD_MARKER.len()}]})
}

#[test]
fn source_refresh_pins_old_selected_excerpt_past_candidate_limit() {
    let root = TemporaryDirectory::new("task-old-excerpt-refresh");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let offline = write_context_window_config(root.path(), "http://127.0.0.1:1/v1", 272_000);
    let created = run_chat(
        root.path(),
        &offline,
        "/exit\n",
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    let id = session_id(root.path());
    let project = Project::discover(root.path()).unwrap();
    let mut sources = vec![echidna::domain::model::ContentItem::assistant_text(
        OLD_METHOD_MARKER,
    )];
    sources.extend((0..530).map(|index| {
        echidna::domain::model::ContentItem::assistant_text(format!(
            "unrelated historical item {index}"
        ))
    }));
    let ids = super::task_context_paging::append_source_items(&project, &id, sources);
    let server = super::task_context::spawn_scripted_responses_server_with_wait(
        vec![
            ScriptedResponse::SelectionDecision(request_old_method),
            ScriptedResponse::SelectionDecision(select_old_method),
            ScriptedResponse::Response(
                200,
                m4_long_flow_tool_response(
                    "excerpt-write-once",
                    "run_command",
                    json!({"argv":["/bin/sh","-c","printf 'once\\n' >> excerpt-once.txt"]}),
                    "excerpt-write",
                ),
            ),
            ScriptedResponse::Response(
                200,
                text_response("Method retained.", "excerpt-after-write"),
            ),
            ScriptedResponse::SelectionDecision(select_old_method),
            ScriptedResponse::Response(200, text_response("Resumed.", "excerpt-resumed")),
        ],
        Duration::from_secs(60),
    );
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let first = run_chat(root.path(), &config, "ZZZZZZ\n/exit\n", &["--resume", &id]);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let wires = (0..4)
        .map(|_| f2a_request_body(&server.requests, "old excerpt refresh"))
        .collect::<Vec<_>>();
    let expanded = super::task_context::selector_candidates(&wires[1]);
    assert!(
        expanded["context"]
            .as_array()
            .unwrap()
            .iter()
            .any(|unit| unit["source_event_ref"] == ids[0])
    );
    assert!(json_contains_text(&wires[2]["input"], OLD_METHOD_MARKER));
    if wires[3]["previous_response_id"].is_null() {
        assert!(json_contains_text(&wires[3]["input"], OLD_METHOD_MARKER));
    } else {
        assert_eq!(wires[3]["previous_response_id"], "excerpt-write");
    }
    let events = session_events(root.path(), &id);
    let audit = events
        .iter()
        .rfind(|event| event["event_type"] == "session.request_audit_recorded")
        .unwrap();
    assert!(
        super::task_context_progress::logical_user_texts(root.path(), audit)
            .iter()
            .any(|text| text.contains(OLD_METHOD_MARKER))
    );
    assert!(
        audit["payload"]["full_context_delivery_map"]
            .to_string()
            .contains(ids[0].as_str())
    );
    let refresh = events
        .iter()
        .find(|event| {
            event["event_type"] == "session.task_context_selected"
                && event["payload"]["refresh_of"].is_string()
        })
        .unwrap();
    assert_eq!(
        refresh["payload"]["selected_excerpts"][0]["source_event_id"],
        ids[0]
    );
    assert_eq!(
        fs::read_to_string(root.path().join("excerpt-once.txt")).unwrap(),
        "once\n"
    );
    assert_eq!(reduced_session(&project, &id).execution_receipt_count(), 1);
    let resumed = run_chat(
        root.path(),
        &config,
        &format!("Continue {OLD_METHOD_MARKER}.\n/exit\n"),
        &["--resume", &id],
    );
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    super::task_context::selector_candidates(&f2a_request_body(
        &server.requests,
        "old excerpt resumed selector",
    ));
    let wire = f2a_request_body(&server.requests, "old excerpt resumed main");
    assert!(json_contains_text(&wire["input"], OLD_METHOD_MARKER));
    assert!(server.requests.try_recv().is_err());
    server.join();
    assert_eq!(
        fs::read_to_string(root.path().join("excerpt-once.txt")).unwrap(),
        "once\n"
    );
    let state = reduced_session(&project, &id);
    assert_eq!(state.execution_receipt_count(), 1);
    assert_audits_reconstruct(root.path(), &state);
}

#[test]
fn snapshot_refresh_preserves_explicit_dependency_on_duplicate_core_section() {
    let root = TemporaryDirectory::new("task-duplicate-core-refresh");
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
    let section = "# Duplicate\nIDENTICAL-SCOPED-CORE\n";
    fs::write(&memory, format!("{section}{section}")).unwrap();
    let server = spawn_scripted_responses_server(vec![
        ScriptedResponse::SelectionDecision(initial_duplicate_core),
        ScriptedResponse::Response(200, text_response("A ready.", "duplicate-a")),
        ScriptedResponse::SelectionDecision(second_duplicate_core),
        ScriptedResponse::Response(200, text_response("B ready.", "duplicate-b")),
        ScriptedResponse::SelectionDecision(return_with_duplicate_dependency),
        ScriptedResponse::Response(
            200,
            m4_long_flow_tool_response(
                "duplicate-refresh",
                "run_command",
                json!({"argv":["/bin/sh","-c","printf changed > snapshot-change.txt"]}),
                "duplicate-write",
            ),
        ),
        ScriptedResponse::Response(200, text_response("A continued.", "duplicate-final")),
    ]);
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "DUPLICATE-CORE-TASK-A\nDUPLICATE-CORE-TASK-B\nReturn to A with B's exact section.\n/exit\n",
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let wires = (0..7)
        .map(|_| f2a_request_body(&server.requests, "duplicate Core refresh"))
        .collect::<Vec<_>>();
    server.join();
    assert_eq!(wires[6]["previous_response_id"], "duplicate-write");
    let id = session_id(root.path());
    let events = session_events(root.path(), &id);
    let refresh = events
        .iter()
        .find(|event| {
            event["event_type"] == "session.task_context_selected"
                && event["payload"]["refresh_of"].is_string()
        })
        .unwrap();
    let prior = events
        .iter()
        .find(|event| event["event_id"] == refresh["payload"]["refresh_of"])
        .unwrap();
    assert_eq!(
        prior["payload"]["selected_core"].as_array().unwrap().len(),
        2
    );
    assert_eq!(
        refresh["payload"]["selected_core"],
        prior["payload"]["selected_core"]
    );
    assert_eq!(
        fs::read(root.path().join("snapshot-change.txt")).unwrap(),
        b"changed"
    );
    let state = reduced_session(&Project::discover(root.path()).unwrap(), &id);
    let final_audit = events
        .iter()
        .rfind(|event| event["event_type"] == "session.request_audit_recorded")
        .unwrap();
    let full = super::task_context_progress::logical_user_texts(root.path(), final_audit);
    assert_eq!(
        full.iter()
            .map(|text| text.matches("IDENTICAL-SCOPED-CORE").count())
            .sum::<usize>(),
        2
    );
    assert_audits_reconstruct(root.path(), &state);
}

#[test]
fn deleting_a_duplicate_core_section_does_not_relocate_its_dependency() {
    let root = TemporaryDirectory::new("task-duplicate-core-deletion");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let manifest = agent.join("agent.toml");
    let configured = fs::read_to_string(&manifest)
        .unwrap()
        .replace("memory = \"none\"", "memory = \"agent\"");
    fs::write(
        &manifest,
        format!("{configured}memory_write = \"direct\"\n"),
    )
    .unwrap();
    let memory = root
        .path()
        .join(".echidna/memory/agents/task-context-fixture/core.md");
    fs::create_dir_all(memory.parent().unwrap()).unwrap();
    let section = "# Duplicate\nIDENTICAL-SCOPED-CORE\n";
    let original = format!("{section}{section}");
    fs::write(&memory, &original).unwrap();
    let hash = blake3::hash(original.as_bytes()).to_hex().to_string();
    let server = super::task_context::spawn_scripted_responses_server_with_wait(
        vec![
            ScriptedResponse::SelectionDecision(initial_duplicate_core),
            ScriptedResponse::Response(200, text_response("A ready.", "delete-core-a")),
            ScriptedResponse::SelectionDecision(second_duplicate_core),
            ScriptedResponse::Response(200, text_response("B ready.", "delete-core-b")),
            ScriptedResponse::SelectionDecision(return_with_duplicate_dependency),
            ScriptedResponse::Response(
                200,
                m4_long_flow_tool_response(
                    "delete-core-b-once",
                    "memory_write",
                    json!({"path":"core.md", "content":section, "expected_hash":hash, "event_id":"delete-core-b-once"}),
                    "delete-core-write",
                ),
            ),
            ScriptedResponse::Response(200, text_response("Core updated.", "delete-core-done")),
            ScriptedResponse::SelectionDecision(continue_duplicate_a),
            ScriptedResponse::Response(200, text_response("Resumed.", "delete-core-resumed")),
        ],
        Duration::from_secs(60),
    );
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "DUPLICATE-CORE-TASK-A\nDUPLICATE-CORE-TASK-B\nReturn to A with B's exact section.\n/exit\n",
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let wires = (0..7)
        .map(|_| f2a_request_body(&server.requests, "duplicate Core deletion"))
        .collect::<Vec<_>>();
    let selected = super::task_context::selector_candidates(&wires[4]);
    let sections = duplicate_core_sections(&selected);
    let id = session_id(root.path());
    let project = Project::discover(root.path()).unwrap();
    let events = session_events(root.path(), &id);
    let refresh = events
        .iter()
        .find(|event| {
            event["event_type"] == "session.task_context_selected"
                && event["payload"]["refresh_of"].is_string()
        })
        .unwrap();
    let prior = events
        .iter()
        .find(|event| event["event_id"] == refresh["payload"]["refresh_of"])
        .unwrap();
    assert!(
        prior["payload"]["decision"]["required_context_refs"]
            .as_array()
            .unwrap()
            .contains(&sections[1]["ref"])
    );
    assert_eq!(prior["payload"]["core_scopes"].as_array().unwrap().len(), 2);
    assert_ne!(
        prior["payload"]["core_scopes"][0]["scope"],
        prior["payload"]["core_scopes"][1]["scope"]
    );
    let refreshed_scopes = refresh["payload"]["core_scopes"].as_array().unwrap();
    assert_eq!(refreshed_scopes.len(), 1);
    assert_eq!(refreshed_scopes[0]["scope"]["kind"], "unknown");
    let survivor = &refreshed_scopes[0]["reference"];
    assert!(
        !refresh["payload"]["decision"]["required_context_refs"]
            .as_array()
            .unwrap()
            .contains(survivor),
        "deleted B must not be mapped onto A"
    );
    assert_eq!(fs::read_to_string(&memory).unwrap(), section);
    let state = reduced_session(&project, &id);
    assert_eq!(state.execution_receipt_count(), 1);
    let final_audit = events
        .iter()
        .rfind(|event| event["event_type"] == "session.request_audit_recorded")
        .unwrap();
    let texts = super::task_context_progress::logical_user_texts(root.path(), final_audit);
    assert_eq!(
        texts
            .iter()
            .map(|text| text.matches("IDENTICAL-SCOPED-CORE").count())
            .sum::<usize>(),
        1
    );
    assert_audits_reconstruct(root.path(), &state);

    let resumed = run_chat(
        root.path(),
        &config,
        "Continue the task.\n/exit\n",
        &["--resume", &id],
    );
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    super::task_context::selector_candidates(&f2a_request_body(
        &server.requests,
        "Core deletion resumed selection",
    ));
    let wire = f2a_request_body(&server.requests, "Core deletion resumed main");
    assert!(json_contains_text(&wire["input"], "IDENTICAL-SCOPED-CORE"));
    assert!(server.requests.try_recv().is_err());
    server.join();
    assert_eq!(fs::read_to_string(&memory).unwrap(), section);
    let state = reduced_session(&project, &id);
    assert_eq!(state.execution_receipt_count(), 1);
    assert_audits_reconstruct(root.path(), &state);
}

#[test]
fn current_visual_sources_drive_selector_and_preserve_original_main_input() {
    use super::task_context::{new_selection_response, selector_candidates};
    use super::task_context_paging::paged_server;
    for secondary in [false, true] {
        let root = TemporaryDirectory::new("task-current-visual-source");
        initialize_project(root.path());
        let agent = definition(root.path(), None);
        fs::write(root.path().join("screen.png"), png_bytes(8, 8)).unwrap();
        let mut main_count = 0;
        let server = paged_server(move |_, wire| {
            if wire.get("tools").is_none() {
                let candidates = selector_candidates(wire);
                if candidates["input"]["text"] != "Continue based on the image." {
                    return (new_selection_response(wire, "visual-seed-selection"), false);
                }
                let visual = candidates["input"]["visual_sources"].as_array().unwrap();
                assert_eq!(visual.len(), 1);
                assert!(visual[0]["history_event_ref"].as_str().is_some());
                assert!(
                    !visual[0]["source_event_refs"]
                        .as_array()
                        .unwrap()
                        .is_empty()
                );
                assert!(!visual[0]["artifact_refs"].as_array().unwrap().is_empty());
                if secondary {
                    let text = visual[0]["observation_text"].as_str().unwrap();
                    assert!(text.contains("VISUAL-ROUTE-BETA"));
                    assert_eq!(visual[0]["inspected_bytes"], visual[0]["total_bytes"]);
                    let beta = candidates["tasks"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|task| task["goal"] == "Visual task beta.")
                        .unwrap();
                    let decision = json!({"relation":"return", "focus":{"kind":"existing", "task_ref":beta["ref"]}, "related_task_refs":[], "required_context_refs":[], "unresolved_references":[]});
                    return (
                        text_response(&decision.to_string(), "visual-beta-selection"),
                        false,
                    );
                }
                assert!(visual[0]["observation_text"].is_null());
                assert!(
                    candidates["gaps"]
                        .to_string()
                        .contains(visual[0]["history_event_ref"].as_str().unwrap())
                );
                assert!(candidates["gaps"].to_string().contains("not inspected"));
                return (
                    new_selection_response(wire, "visual-uninspected-selection"),
                    false,
                );
            }
            main_count += 1;
            if main_count == 3 {
                if secondary {
                    assert!(wire.to_string().contains("VISUAL-ROUTE-BETA"));
                } else {
                    assert!(json_contains_type(&wire["input"], "input_image"));
                }
                return (text_response("Visual input handled.", "visual-final"), true);
            }
            (
                text_response("Seed complete.", &format!("visual-seed-{main_count}")),
                false,
            )
        });
        let vision = secondary.then(|| {
            spawn_responses_server(vec![(
                200,
                vision_response("VISUAL-ROUTE-BETA", "visual-observation"),
            )])
        });
        let config = if let Some(vision) = &vision {
            write_dual_config(root.path(), &server.base_url, &vision.base_url)
        } else {
            write_direct_config(root.path(), &server.base_url)
        };
        let output = run_chat(
            root.path(),
            &config,
            "Visual task alpha.\nVisual task beta.\n/image screen.png\nContinue based on the image.\n/exit\n",
            &["--agent", agent.to_str().unwrap()],
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let wires = (0..6)
            .map(|_| f2a_request_body(&server.requests, "current visual selection"))
            .collect::<Vec<_>>();
        server.join();
        if let Some(vision) = vision {
            f2a_request_body(&vision.requests, "current secondary vision");
            vision.join();
        }
        let selected = selector_candidates(&wires[4]);
        let visual = &selected["input"]["visual_sources"][0];
        let id = session_id(root.path());
        let events = session_events(root.path(), &id);
        assert!(
            events
                .iter()
                .any(|event| event["event_id"] == visual["history_event_ref"])
        );
        let state = reduced_session(&Project::discover(root.path()).unwrap(), &id);
        assert_audits_reconstruct(root.path(), &state);
        let resumed = run_chat(root.path(), &config, "/exit\n", &["--resume", "latest"]);
        assert!(
            resumed.status.success(),
            "{}",
            String::from_utf8_lossy(&resumed.stderr)
        );
    }
}

#[test]
fn superseded_constraint_does_not_remove_unrelated_result_or_gap() {
    let root = TemporaryDirectory::new("task-progress-source-identity");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let server = spawn_scripted_responses_server(vec![
        ScriptedResponse::InitialSelection,
        ScriptedResponse::Response(
            200,
            m4_long_flow_tool_response(
                "report-call",
                "run_command",
                json!({"argv":["/bin/echo",OBSERVATION]}),
                "report-tool",
            ),
        ),
        ScriptedResponse::Response(200, text_response("Report observed.", "report-done")),
        ScriptedResponse::SelectionDecision(record_constraint_and_gap),
        ScriptedResponse::SelectionDecision(record_constraint_and_gap),
        ScriptedResponse::Response(200, text_response("Gap noted.", "gap-done")),
        ScriptedResponse::SelectionDecision(correct_constraint),
        ScriptedResponse::Response(200, text_response("Constraint corrected.", "corrected")),
        ScriptedResponse::SelectionDecision(continue_task),
        ScriptedResponse::Response(200, text_response("Continued.", "continued")),
    ]);
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        &format!("{ORIGINAL}\nRecord the requirement and gap.\n{CORRECTION}\nContinue.\n/exit\n"),
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let wires = (0..10)
        .map(|_| f2a_request_body(&server.requests, "source identity"))
        .collect::<Vec<_>>();
    server.join();
    assert!(json_contains_text(&wires[9]["input"], OBSERVATION));
    assert!(json_contains_text(&wires[9]["input"], GAP));
    assert!(json_contains_text(&wires[9]["input"], CORRECTION));
    assert!(json_contains_text(
        &wires[9]["input"],
        "[superseded constraint omitted]"
    ));
    let events = session_events(root.path(), &session_id(root.path()));
    let receipt = events
        .iter()
        .find(|event| {
            event["event_type"] == "tool.execution_outcome_recorded"
                && event["payload"]["result"]["call_id"] == "report-call"
        })
        .unwrap();
    let state = reduced_session(
        &Project::discover(root.path()).unwrap(),
        &session_id(root.path()),
    );
    let map = serde_json::to_string(
        state
            .request_audits()
            .last()
            .unwrap()
            .context_delivery_map()
            .unwrap(),
    )
    .unwrap();
    assert!(map.contains(receipt["event_id"].as_str().unwrap()));
    assert_audits_reconstruct(root.path(), &state);
    let resumed = run_chat(root.path(), &config, "/exit\n", &["--resume", "latest"]);
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
}

#[test]
fn second_lossy_handoff_includes_prior_protected_batch_and_receipt() {
    use super::task_context::selector_candidates;
    use super::task_context_paging::paged_server_with_limit;

    let root = TemporaryDirectory::new("task-source-two-handoffs");
    let effects = TemporaryDirectory::new("task-source-two-effects");
    let count = effects.path().join("count.txt");
    let count_for_server = count.clone();
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let mut main = 0;
    let mut handoffs = 0;
    let server = paged_server_with_limit(F2A_WAIT, F2A_WAIT, 32, move |_, wire| {
        if wire["instructions"]
            .as_str()
            .unwrap_or("")
            .contains("continuation handoff")
        {
            handoffs += 1;
            return (
                compaction_summary_response(
                    "Lossy synthetic handoff.",
                    &format!("handoff-{handoffs}"),
                ),
                false,
            );
        }
        if wire.get("tools").is_none() {
            let candidates = selector_candidates(wire);
            if candidates["tasks"].as_array().unwrap().is_empty() {
                return (
                    super::task_context::initial_selection_response(wire, "initial-selection"),
                    false,
                );
            }
            let summary = candidates["context"]
                .as_array()
                .unwrap()
                .iter()
                .find(|unit| {
                    unit["ref"]
                        .as_str()
                        .is_some_and(|value| value.starts_with("summary:"))
                })
                .expect("the first handoff is offered as a selected summary");
            let decision = json!({"relation":"continue", "focus":{"kind":"existing", "task_ref":candidates["tasks"][0]["ref"]},
                "related_task_refs":[], "required_context_refs":[summary["ref"]], "unresolved_references":[]});
            return (
                text_response(&decision.to_string(), "summary-selection"),
                false,
            );
        }
        main += 1;
        if main == 1 {
            return (
                text_response(
                    &format!(
                        "Original task details {}",
                        "historical answer ".repeat(10_000)
                    ),
                    "original-task",
                ),
                false,
            );
        }
        if main <= 9 {
            let batch = main - 1;
            let reasoning = json!({"type":"response.output_item.done", "output_index":0,
                "item":{"type":"reasoning", "content":[{"type":"reasoning_text",
                    "text":format!("batch-{batch} reasoning ").repeat(20_000)}]}});
            let command = format!(
                "printf '{batch}\\n' >> '{}'; printf 'UNSUMMARIZED-RESULT-{batch} '; printf '%s' \"$1\"",
                count_for_server.display()
            );
            return (
                format!(
                    "data: {reasoning}\n\n{}",
                    m4_long_flow_tool_response(
                        &format!("batch-{batch}"),
                        "run_command",
                        json!({"argv":["/bin/sh","-c",command,"fixture","neutral-output ".repeat(2_000)]}),
                        &format!("batch-response-{batch}")
                    )
                ),
                false,
            );
        }
        assert_eq!(main, 10);
        (
            text_response("Finished both batches.", "batches-final"),
            true,
        )
    });
    let config = write_context_window_config(root.path(), &server.base_url, 1_048_576);
    fs::write(&config, format!("{}\n[runtime]\nauto_compact = true\n[runtime.context_budget]\npressure_bps = 1000\ntarget_bps = 250\n", fs::read_to_string(&config).unwrap())).unwrap();
    let output = run_chat(
        root.path(),
        &config,
        "Prepare the synthetic task.\n/compact\nContinue the synthetic task and execute the batches.\n/exit\n",
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let wires = server
        .requests
        .try_iter()
        .map(|request| request_body(&request))
        .collect::<Vec<_>>();
    server.join();
    let handoff_wires = wires
        .iter()
        .filter(|wire| {
            wire["instructions"]
                .as_str()
                .unwrap_or("")
                .contains("continuation handoff")
        })
        .collect::<Vec<_>>();
    assert!(
        handoff_wires.len() >= 3,
        "expected the explicit initial handoff and two settled-batch handoffs; observed {} handoffs in {} requests",
        handoff_wires.len(),
        wires.len()
    );
    assert!(json_contains_text(
        &handoff_wires[2]["input"],
        "UNSUMMARIZED-RESULT-1"
    ));
    let session = session_id(root.path());
    let events = session_events(root.path(), &session);
    let history = |kind: &str| {
        events
            .iter()
            .find(|event| {
                event["event_type"] == "session.history_item_recorded"
                    && event["payload"]["item"][kind]["call_id"] == "batch-1"
            })
            .unwrap()
    };
    let call = history("call");
    let result = history("result");
    let receipt = events
        .iter()
        .find(|event| {
            event["event_type"] == "tool.execution_outcome_recorded"
                && event["payload"]["result"]["call_id"] == "batch-1"
        })
        .unwrap();
    let selection = events
        .iter()
        .find(|event| {
            event["event_type"] == "session.task_context_selected"
                && event["payload"]["selected_summary"].is_string()
        })
        .unwrap();
    let compactions = events
        .iter()
        .filter(|event| {
            event["event_type"] == "session.compaction_created"
                && event["sequence"].as_u64().unwrap() > selection["sequence"].as_u64().unwrap()
        })
        .collect::<Vec<_>>();
    assert!(
        compactions.len() >= 3,
        "three actual task handoffs are required for transitive coverage"
    );
    let first = &compactions[0]["payload"];
    assert!(
        first["retained_event_ids"]
            .as_array()
            .unwrap()
            .contains(&result["event_id"])
    );
    assert!(
        first["summary"]
            .get("summarized_history")
            .is_none_or(|value| {
                !value
                    .as_array()
                    .expect("summarized_history must be an array")
                    .contains(&result["event_id"])
            })
    );
    let second = &compactions[1]["payload"];
    let plan: Value = serde_json::from_slice(
        &fs::read(
            root.path()
                .join(".echidna/artifacts/blobs/sha256")
                .join(second["summary"]["source_plan_blob_id"].as_str().unwrap()),
        )
        .unwrap(),
    )
    .unwrap();
    let entries = plan["entries"].as_array().unwrap();
    for source in [call, result] {
        assert!(
            entries
                .iter()
                .any(|entry| entry["source"]["kind"] == "history"
                    && entry["source"]["id"] == source["event_id"])
        );
    }
    assert!(
        second["summary"]["summarized_history"]
            .as_array()
            .unwrap()
            .contains(&result["event_id"]),
        "second handoff must actually summarize the first retained batch"
    );
    assert!(
        second["summary"]["task_references"]["execution_receipt_refs"]
            .as_array()
            .unwrap()
            .contains(&receipt["event_id"])
    );
    let third = &compactions[2]["payload"];
    let third_plan: Value = serde_json::from_slice(
        &fs::read(
            root.path()
                .join(".echidna/artifacts/blobs/sha256")
                .join(third["summary"]["source_plan_blob_id"].as_str().unwrap()),
        )
        .unwrap(),
    )
    .unwrap();
    for source in [call, result] {
        assert!(
            third["summary"]["summarized_history"]
                .as_array()
                .unwrap()
                .contains(&source["event_id"]),
            "the third Summary must inherit the exact first batch coverage"
        );
        assert!(
            !third_plan["entries"]
                .as_array()
                .unwrap()
                .iter()
                .any(|entry| entry["source"]["kind"] == "history"
                    && entry["source"]["id"] == source["event_id"]),
            "covered first-batch History must not be re-expanded into the third handoff"
        );
    }
    assert!(
        third["summary"]["task_references"]["history_event_refs"]
            .as_array()
            .unwrap()
            .contains(&result["event_id"])
    );
    assert!(
        third["summary"]["task_references"]["execution_receipt_refs"]
            .as_array()
            .unwrap()
            .contains(&receipt["event_id"])
    );
    let protected_result = events
        .iter()
        .rev()
        .find(|event| {
            event["event_type"] == "session.history_item_recorded"
                && event["payload"]["item"]["result"]["call_id"]
                    .as_str()
                    .is_some_and(|id| id.starts_with("batch-"))
                && event["sequence"].as_u64().unwrap()
                    < compactions[2]["sequence"].as_u64().unwrap()
        })
        .unwrap();
    assert!(
        third["retained_event_ids"]
            .as_array()
            .unwrap()
            .contains(&protected_result["event_id"])
    );
    assert!(
        !third["summary"]["summarized_history"]
            .as_array()
            .unwrap()
            .contains(&protected_result["event_id"]),
        "the retained latest batch was not summarized"
    );
    let later_audit = events
        .iter()
        .find(|event| {
            event["event_type"] == "session.request_audit_recorded"
                && event["sequence"].as_u64().unwrap()
                    > compactions[1]["sequence"].as_u64().unwrap()
        })
        .unwrap();
    assert!(
        later_audit["payload"]["full_context_delivery_map"]
            .to_string()
            .contains(receipt["event_id"].as_str().unwrap())
    );
    let state = reduced_session(&Project::discover(root.path()).unwrap(), &session);
    assert_audits_reconstruct(root.path(), &state);
    let resumed = run_chat(root.path(), &config, "/exit\n", &["--resume", "latest"]);
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    assert_eq!(
        fs::read_to_string(count).unwrap(),
        "1\n2\n3\n4\n5\n6\n7\n8\n"
    );
}

#[test]
fn rejected_sample_refresh_replays_and_rejects_forged_or_missing_feedback() {
    use sha2::{Digest, Sha256};

    let root = TemporaryDirectory::new("task-source-rejected-refresh");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let invalid = [
        json!({"type":"response.output_item.done","item":{"type":"function_call","call_id":"bad-call","name":"run_command","arguments":"{\\q}"}}),
        json!({"type":"response.completed","response":{"id":"rejected-sample"}}),
    ].iter().map(|event| format!("data: {event}\n\n")).collect();
    let server = spawn_scripted_responses_server(vec![
        ScriptedResponse::InitialSelection,
        ScriptedResponse::Response(200, invalid),
        ScriptedResponse::Response(
            200,
            m4_long_flow_tool_response(
                "one-effect",
                "run_command",
                json!({"argv":["/bin/sh","-c","printf one > refreshed-snapshot.txt"]}),
                "accepted-tool",
            ),
        ),
        ScriptedResponse::Response(200, text_response("Finished.", "refreshed-final")),
    ]);
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "Update the synthetic snapshot once.\n/exit\n",
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for _ in 0..4 {
        f2a_request_body(&server.requests, "rejected refresh");
    }
    server.join();
    let session = session_id(root.path());
    let events = session_events(root.path(), &session);
    let rejection = events
        .iter()
        .find(|event| event["event_type"] == "model.sample_rejected")
        .unwrap();
    let step = rejection["payload"]["sampling_step_id"].as_str().unwrap();
    assert!(
        events
            .iter()
            .any(|event| event["event_type"] == "model.argument_correction_reset")
    );
    let selected = events
        .iter()
        .rposition(|event| {
            event["event_type"] == "session.task_context_selected"
                && event["payload"]["refresh_of"].is_string()
        })
        .unwrap();
    let blobs = root.path().join(".echidna/artifacts/blobs/sha256");
    let capture_id = events[selected]["payload"]["capture_blob"]
        .as_str()
        .unwrap();
    let capture: Value =
        serde_json::from_slice(&fs::read(blobs.join(capture_id)).unwrap()).unwrap();
    let feedback = capture["compile"]["correction_feedback"]
        .as_array()
        .unwrap();
    assert_eq!(
        feedback.len(),
        1,
        "successful reset must not clear the in-memory vector"
    );
    assert_eq!(feedback[0]["id"], format!("correction:{step}"));
    let state = reduced_session(&Project::discover(root.path()).unwrap(), &session);
    assert_audits_reconstruct(root.path(), &state);
    let valid = run_chat(root.path(), &config, "/exit\n", &["--resume", "latest"]);
    assert!(
        valid.status.success(),
        "{}",
        String::from_utf8_lossy(&valid.stderr)
    );

    let event_path = root
        .path()
        .join(".echidna/sessions")
        .join(&session)
        .join("events.jsonl");
    // Change the rejection and the corresponding complete feedback together;
    // the original attempted audit remains the independent identity source.
    let mut coherent_capture = capture.clone();
    let fragment = &mut coherent_capture["compile"]["correction_feedback"][0];
    fragment["id"] = json!("correction:forged-step");
    let text = fragment["content"]["items"][0]["text"]
        .as_str()
        .unwrap()
        .replace(step, "forged-step");
    fragment["content"]["items"][0]["text"] = json!(text);
    let content: echidna::domain::context::ContextFragmentContent =
        serde_json::from_value(fragment["content"].clone()).unwrap();
    let hash = echidna::domain::context::ContextContentHash::for_fragment(&content, None).unwrap();
    fragment["provenance"]["freshness"]["source_revision_or_hash"] = json!(hash.as_str());
    fragment["item_provenance"]["0"]["freshness"]["source_revision_or_hash"] = json!(hash.as_str());
    let bytes = serde_json::to_vec(&coherent_capture).unwrap();
    let blob = format!("{:x}", Sha256::digest(&bytes));
    fs::write(blobs.join(&blob), bytes).unwrap();
    let mut coherent = events[..=selected].to_vec();
    let rejection_index = coherent
        .iter()
        .position(|event| event["event_type"] == "model.sample_rejected")
        .unwrap();
    coherent[rejection_index]["payload"]["sampling_step_id"] = json!("forged-step");
    let capture_event = coherent
        .iter()
        .position(|event| event["event_id"] == coherent[selected]["payload"]["capture_event_id"])
        .unwrap();
    coherent[capture_event]["payload"]["blob_id"] = json!(blob);
    coherent[selected]["payload"]["capture_blob"] = json!(blob);
    let coherent_source_hash = delivery_lineage::canonical_hash(&&coherent[..=capture_event]);
    coherent[selected]["payload"]["source"]["stream_hash"] = json!(coherent_source_hash);
    let decoded = coherent
        .into_iter()
        .map(|event| serde_json::from_value::<EventEnvelope<Value>>(event).unwrap())
        .collect::<Vec<_>>();
    assert!(
        SessionReducer.reduce(decoded).is_err(),
        "rejection and feedback cannot forge the unchanged audit5 Step"
    );
    for forged_id in [false, true] {
        let mut modified = capture.clone();
        if forged_id {
            let fragment = &mut modified["compile"]["correction_feedback"][0];
            fragment["id"] = json!("correction:forged-step");
            let text = fragment["content"]["items"][0]["text"]
                .as_str()
                .unwrap()
                .replace(step, "forged-step");
            fragment["content"]["items"][0]["text"] = json!(text);
            let content: echidna::domain::context::ContextFragmentContent =
                serde_json::from_value(fragment["content"].clone()).unwrap();
            let hash =
                echidna::domain::context::ContextContentHash::for_fragment(&content, None).unwrap();
            fragment["provenance"]["freshness"]["source_revision_or_hash"] = json!(hash.as_str());
            fragment["item_provenance"]["0"]["freshness"]["source_revision_or_hash"] =
                json!(hash.as_str());
        } else {
            modified["compile"]["correction_feedback"] = json!([]);
        }
        let bytes = serde_json::to_vec(&modified).unwrap();
        let blob = format!("{:x}", Sha256::digest(&bytes));
        fs::write(blobs.join(&blob), bytes).unwrap();
        let mut prefix = events[..=selected].to_vec();
        let capture_event = prefix
            .iter()
            .position(|event| event["event_id"] == prefix[selected]["payload"]["capture_event_id"])
            .unwrap();
        prefix[capture_event]["payload"]["blob_id"] = json!(blob);
        prefix[selected]["payload"]["capture_blob"] = json!(blob);
        let source_hash = delivery_lineage::canonical_hash(&&prefix[..=capture_event]);
        prefix[selected]["payload"]["source"]["stream_hash"] = json!(source_hash);
        let decoded = prefix
            .iter()
            .cloned()
            .map(|event| serde_json::from_value::<EventEnvelope<Value>>(event).unwrap())
            .collect::<Vec<_>>();
        SessionReducer
            .reduce(decoded)
            .expect("modified prefix remains structurally coherent");
        let bytes = prefix
            .iter()
            .map(|event| format!("{}\n", serde_json::to_string(event).unwrap()))
            .collect::<String>();
        fs::write(&event_path, &bytes).unwrap();
        let rejected = run_chat(root.path(), &config, "/exit\n", &["--resume", "latest"]);
        assert!(
            !rejected.status.success(),
            "forged feedback accepted: {forged_id}"
        );
        assert!(
            String::from_utf8_lossy(&rejected.stderr)
                .contains("primary task source compilation failed"),
            "{}",
            String::from_utf8_lossy(&rejected.stderr)
        );
        assert_eq!(fs::read(&event_path).unwrap(), bytes.as_bytes());
    }
    assert_eq!(
        fs::read(root.path().join("refreshed-snapshot.txt")).unwrap(),
        b"one"
    );
}
