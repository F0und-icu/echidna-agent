//! Regressions for completed selector accounting, scope, and sampling settings.
use super::task_context::{definition, selector_candidates};
use super::task_context_paging::paged_server;
use super::*;

fn response_with_usage(text: &str, id: &str) -> String {
    format!(
        "data: {}\n\ndata: {}\n\n",
        json!({"type":"response.output_text.delta","delta":text}),
        json!({"type":"response.completed","response":{"id":id,"usage":{"input_tokens":120,"output_tokens":30,"input_tokens_details":{"cached_tokens":10},"output_tokens_details":{"reasoning_tokens":20}}}})
    )
}

fn decision(candidates: &Value) -> Value {
    json!({"relation":"new","focus":{"kind":"new_from_input","input_ref":candidates["input"]["ref"]},"related_task_refs":[],"required_context_refs":[],"unresolved_references":[]})
}

#[test]
fn repeated_active_constraints_keep_exact_scope_range_and_single_revision() {
    const TEXT: &str = "只熟悉，并告诉我你的理解就可以了";
    assert_eq!(TEXT.len(), 48);
    for mode in [
        "current",
        "explicit",
        "duplicate",
        "scope_conflict",
        "partial",
        "invalid_utf8",
    ] {
        let root = TemporaryDirectory::new("task-constraint-reassertion");
        initialize_project(root.path());
        let agent = definition(root.path(), None);
        let valid = !matches!(mode, "scope_conflict" | "invalid_utf8");
        let server = paged_server(move |step, wire| match step {
            0 => {
                assert!(wire["instructions"].as_str().unwrap().contains("Active constraints supplied as constraint_range candidates are already recorded."));
                let candidates = selector_candidates(wire);
                let mut selected = decision(&candidates);
                let update = json!({"source_ref":candidates["input"]["ref"],"start_byte":0,"end_byte":TEXT.len(),"scope":{"kind":"current_task"},"supersedes":null});
                selected["constraint_updates"] = if mode == "duplicate" {
                    json!([update.clone(), update])
                } else {
                    json!([update])
                };
                (
                    text_response(&selected.to_string(), "constraint-first-select"),
                    false,
                )
            }
            1 => (
                text_response("Synthetic analysis complete.", "constraint-first-main"),
                false,
            ),
            2 => {
                let candidates = selector_candidates(wire);
                let task = &candidates["tasks"][0]["ref"];
                let prior = candidates["context"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|c| c["constraint_range"].is_object())
                    .unwrap();
                let scope = match mode {
                    "explicit" => json!({"kind":"task","task_ref":task}),
                    "scope_conflict" => json!({"kind":"global"}),
                    _ => json!({"kind":"current_task"}),
                };
                let end = match mode {
                    "partial" => 45,
                    "invalid_utf8" => 47,
                    _ => TEXT.len(),
                };
                let update = json!({"source_ref":prior["constraint_range"]["source_event_id"],"start_byte":0,"end_byte":end,"scope":scope,"supersedes":null});
                let updates = if mode == "duplicate" {
                    json!([update.clone(), update])
                } else {
                    json!([update])
                };
                let selected = json!({"relation":"continue","focus":{"kind":"existing","task_ref":task},"related_task_refs":[],"required_context_refs":[prior["ref"]],"unresolved_references":[],"constraint_updates":updates});
                (
                    text_response(&selected.to_string(), "constraint-second-select"),
                    !valid,
                )
            }
            3 => (
                text_response(
                    "Synthetic recommendation complete.",
                    "constraint-second-main",
                ),
                true,
            ),
            _ => unreachable!(),
        });
        let config = write_context_window_config(root.path(), &server.base_url, 272_000);
        let output = run_chat(
            root.path(),
            &config,
            &format!("{TEXT}\nRecommend a synthetic next target.\n/exit\n"),
            &["--agent", agent.to_str().unwrap()],
        );
        assert_eq!(
            output.status.success(),
            valid,
            "{mode}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        server.join();
        let events = session_events(root.path(), &session_id(root.path()));
        let selections = events
            .iter()
            .filter(|e| e["event_type"] == "session.task_context_selected")
            .map(|e| &e["payload"])
            .collect::<Vec<_>>();
        assert_eq!(selections.len(), if valid { 2 } else { 1 });
        assert_eq!(
            selections[0]["constraint_revisions"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let blob = selections[0]["capture_blob"].as_str().unwrap();
        let capture: Value = serde_json::from_slice(
            &fs::read(
                root.path()
                    .join(".echidna/artifacts/blobs/sha256")
                    .join(blob),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(capture["compile"]["selection_prompt_version"], 1);
        if valid {
            let revisions = selections[1]["constraint_revisions"].as_array().unwrap();
            assert_eq!(revisions.len(), usize::from(mode == "partial"));
            assert_eq!(
                selections[1]["selected_constraints"]
                    .as_array()
                    .unwrap()
                    .len(),
                if mode == "partial" { 2 } else { 1 }
            );
            if mode == "partial" {
                assert_eq!(revisions[0]["range"]["end_byte"], 45);
            }
            assert_eq!(
                selections[1]["decision"]["constraint_updates"]
                    .as_array()
                    .unwrap()
                    .len(),
                if mode == "duplicate" { 2 } else { 1 }
            );
            let resumed = run_chat(root.path(), &config, "/exit\n", &["--resume", "latest"]);
            assert!(
                resumed.status.success(),
                "{mode}: {}",
                String::from_utf8_lossy(&resumed.stderr)
            );
        }
    }
}

#[test]
fn reasserted_corrected_constraint_retains_its_revision_and_supersession_lineage() {
    const ORIGINAL: &str = "Use synthetic source inspection.";
    const CORRECTION: &str = "Correction: use read-only synthetic source inspection.";
    let root = TemporaryDirectory::new("task-corrected-constraint-reassertion");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let server = paged_server(move |step, wire| match step {
        0 | 2 | 4 => {
            let candidates = selector_candidates(wire);
            let mut selected = decision(&candidates);
            let input = &candidates["input"]["ref"];
            let (source, end, supersedes) = if step == 0 {
                (input.clone(), ORIGINAL.len(), Value::Null)
            } else {
                let prior = candidates["context"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|c| c["constraint_range"].is_object())
                    .unwrap();
                selected["relation"] = json!("continue");
                selected["focus"] =
                    json!({"kind":"existing","task_ref":candidates["tasks"][0]["ref"]});
                if step == 2 {
                    (input.clone(), CORRECTION.len(), prior["ref"].clone())
                } else {
                    (
                        prior["constraint_range"]["source_event_id"].clone(),
                        CORRECTION.len(),
                        Value::Null,
                    )
                }
            };
            selected["constraint_updates"] = json!([{"source_ref":source,"start_byte":0,"end_byte":end,"scope":{"kind":"current_task"},"supersedes":supersedes}]);
            (
                text_response(&selected.to_string(), &format!("corrected-select-{step}")),
                false,
            )
        }
        1 | 3 | 5 => (
            text_response(
                "Synthetic analysis complete.",
                &format!("corrected-main-{step}"),
            ),
            step == 5,
        ),
        _ => unreachable!(),
    });
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        &format!("{ORIGINAL}\n{CORRECTION}\nSummarize the synthetic analysis.\n/exit\n"),
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    server.join();
    let events = session_events(root.path(), &session_id(root.path()));
    let selections = events
        .iter()
        .filter(|e| e["event_type"] == "session.task_context_selected")
        .map(|e| &e["payload"])
        .collect::<Vec<_>>();
    assert_eq!(selections.len(), 3);
    let original = &selections[0]["constraint_revisions"][0];
    let correction = &selections[1]["constraint_revisions"][0];
    assert_eq!(correction["revision"], 2);
    assert_eq!(correction["supersedes"], original["reference"]);
    assert!(
        selections[2]["constraint_revisions"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        selections[2]["selected_constraints"],
        json!([correction["range"]])
    );
    let resumed = run_chat(root.path(), &config, "/exit\n", &["--resume", "latest"]);
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
}

#[test]
fn current_input_context_references_are_idempotent_and_resume_exactly() {
    const FIRST: &str = "CURRENT-INPUT-FIRST synthetic task.";
    const SECOND: &str = "CURRENT-INPUT-SECOND use the previous synthetic result.";
    let root = TemporaryDirectory::new("task-current-input-reference");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let server = paged_server(move |step, wire| match step {
        0 | 2 => {
            let candidates = selector_candidates(wire);
            let input = &candidates["input"]["ref"];
            assert!(
                candidates["context"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|c| &c["ref"] != input)
            );
            let mut selected = decision(&candidates);
            selected["required_context_refs"] = json!([input, input]);
            if step == 2 {
                let previous = candidates["context"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|c| c["excerpt"] == "First result.")
                    .unwrap();
                selected["required_context_refs"]
                    .as_array_mut()
                    .unwrap()
                    .push(previous["ref"].clone());
            }
            (
                text_response(&selected.to_string(), &format!("current-select-{step}")),
                false,
            )
        }
        1 | 3 => {
            let rendered = serde_json::to_string(&wire["input"]).unwrap();
            assert_eq!(
                rendered
                    .matches(if step == 1 { FIRST } else { SECOND })
                    .count(),
                1
            );
            if step == 3 {
                assert!(rendered.contains("First result."));
            }
            (
                text_response(
                    if step == 1 {
                        "First result."
                    } else {
                        "Second result."
                    },
                    &format!("current-main-{step}"),
                ),
                step == 3,
            )
        }
        _ => unreachable!(),
    });
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        &format!("{FIRST}\n{SECOND}\n/exit\n"),
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    server.join();
    let events = session_events(root.path(), &session_id(root.path()));
    let selections = events
        .iter()
        .filter(|e| e["event_type"] == "session.task_context_selected")
        .collect::<Vec<_>>();
    assert_eq!(selections.len(), 2);
    for event in selections {
        let selection = &event["payload"];
        assert_eq!(
            selection["decision"]["required_context_refs"][0],
            selection["input_ref"]
        );
        assert_eq!(
            selection["decision"]["required_context_refs"][1],
            selection["input_ref"]
        );
        assert!(
            !selection["selected_history"]
                .as_array()
                .unwrap()
                .contains(&selection["input_ref"])
        );
    }
    let resumed = run_chat(root.path(), &config, "/exit\n", &["--resume", "latest"]);
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
}

#[test]
fn repeated_known_constraint_scopes_are_idempotent_and_conflicts_fail() {
    for mode in [
        "valid",
        "conflict",
        "unknown",
        "unknown_task",
        "duplicate",
        "duplicate_conflict",
        "same_task",
        "same_task_duplicate",
        "new_task_current",
    ] {
        let root = TemporaryDirectory::new("task-live-scope");
        initialize_project(root.path());
        let agent = definition(root.path(), None);
        let server = paged_server(move |step, wire| match step {
            0 => {
                let candidates = selector_candidates(wire);
                let text = candidates["input"]["text"].as_str().unwrap();
                let mut first = decision(&candidates);
                first["constraint_updates"] = json!([
                        {"source_ref":candidates["input"]["ref"],"start_byte":0,"end_byte":5,"scope":{"kind":"global"},"supersedes":null},
                        {"source_ref":candidates["input"]["ref"],"start_byte":6,"end_byte":10,"scope":{"kind":"global"},"supersedes":null},
                        {"source_ref":candidates["input"]["ref"],"start_byte":11,"end_byte":16,"scope":{"kind":"current_task"},"supersedes":null}]);
                assert!(text.starts_with("ALPHA BETA GAMMA"));
                (response_with_usage(&first.to_string(), "select-1"), false)
            }
            1 => (text_response("First done.", "main-1"), false),
            2 => {
                let candidates = selector_candidates(wire);
                let constraints = candidates["context"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|c| c["constraint_range"].is_object())
                    .collect::<Vec<_>>();
                assert_eq!(constraints.len(), 3);
                assert!(
                    constraints
                        .iter()
                        .any(|c| c["known_scope"]["kind"] == "task")
                );
                let mut classifications = constraints
                    .iter()
                    .map(|c| json!({"context_ref":c["ref"],"scope":c["known_scope"]}))
                    .collect::<Vec<_>>();
                let task_index = constraints
                    .iter()
                    .position(|candidate| candidate["known_scope"]["kind"] == "task")
                    .unwrap();
                match mode {
                    "conflict" => classifications[0]["scope"] = json!({"kind":"current_task"}),
                    "unknown" => classifications[0]["context_ref"] = json!("missing-reference"),
                    "unknown_task" => {
                        classifications[0]["scope"] =
                            json!({"kind":"task","task_ref":"missing-task"})
                    }
                    "duplicate" => classifications.push(classifications[0].clone()),
                    "duplicate_conflict" => {
                        let mut conflicting = classifications[0].clone();
                        conflicting["scope"] = json!({"kind":"unknown"});
                        classifications.push(conflicting);
                    }
                    "same_task" | "same_task_duplicate" | "new_task_current" => {
                        let explicit = classifications[task_index].clone();
                        classifications[task_index]["scope"] = json!({"kind":"current_task"});
                        if mode == "same_task_duplicate" {
                            classifications.push(explicit);
                        }
                    }
                    _ => {}
                }
                let mut selected = decision(&candidates);
                if matches!(mode, "same_task" | "same_task_duplicate") {
                    selected["relation"] = json!("return");
                    selected["focus"] = json!({"kind":"existing","task_ref":constraints[task_index]["known_scope"]["task_ref"]});
                }
                selected["scope_classifications"] = json!(classifications);
                (
                    response_with_usage(&selected.to_string(), "select-2"),
                    !matches!(
                        mode,
                        "valid" | "duplicate" | "same_task" | "same_task_duplicate"
                    ),
                )
            }
            3 => (text_response("Second done.", "main-2"), true),
            _ => unreachable!(),
        });
        let config = write_context_window_config(root.path(), &server.base_url, 272_000);
        let output = run_chat(
            root.path(),
            &config,
            "ALPHA BETA GAMMA synthetic task.\nAnother synthetic task.\n/exit\n",
            &["--agent", agent.to_str().unwrap()],
        );
        assert_eq!(
            output.status.success(),
            matches!(
                mode,
                "valid" | "duplicate" | "same_task" | "same_task_duplicate"
            ),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        server.join();
        let id = session_id(root.path());
        let events = session_events(root.path(), &id);
        let usage = events
            .iter()
            .filter(|e| e["event_type"] == "session.task_selection_usage_recorded")
            .collect::<Vec<_>>();
        assert_eq!(usage.len(), 2);
        assert_eq!(
            usage[1]["payload"]["usage"],
            json!({"input_tokens":120,"output_tokens":30,"cached_input_tokens":10,"reasoning_output_tokens":20})
        );
        assert_eq!(
            events
                .iter()
                .filter(|e| e["event_type"] == "session.task_selection_sample_recorded")
                .count(),
            if matches!(
                mode,
                "valid" | "duplicate" | "same_task" | "same_task_duplicate"
            ) {
                2
            } else {
                1
            }
        );
        let state = reduced_session(&Project::discover(root.path()).unwrap(), &id);
        assert_eq!(state.execution_receipt_count(), 0);
        if matches!(
            mode,
            "valid" | "duplicate" | "same_task" | "same_task_duplicate"
        ) {
            let resumed = run_chat(root.path(), &config, "/exit\n", &["--resume", "latest"]);
            assert!(
                resumed.status.success(),
                "{}",
                String::from_utf8_lossy(&resumed.stderr)
            );
        }
    }
}

#[test]
fn malformed_completed_selector_preserves_usage_without_acceptance_or_execution() {
    let root = TemporaryDirectory::new("task-live-invalid-usage");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let server = paged_server(|_, _| (response_with_usage("{bad", "invalid"), true));
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "Synthetic input.\n/exit\n",
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(!output.status.success());
    server.join();
    let events = session_events(root.path(), &session_id(root.path()));
    let usage = events
        .iter()
        .filter(|e| e["event_type"] == "session.task_selection_usage_recorded")
        .collect::<Vec<_>>();
    assert_eq!(usage.len(), 1);
    assert!(events.iter().any(
        |e| e["event_id"] == usage[0]["payload"]["request_audit_event_id"]
            && e["payload"]["provider_session"] == usage[0]["payload"]["provider_session"]
    ));
    assert!(
        !events
            .iter()
            .any(|e| e["event_type"] == "session.task_selection_sample_recorded")
    );
    assert!(
        !events
            .iter()
            .any(|e| e["event_type"] == "tool.execution_outcome_recorded")
    );
    reduced_session(
        &Project::discover(root.path()).unwrap(),
        &session_id(root.path()),
    );
    let resumed = run_chat(root.path(), &config, "/exit\n", &["--resume", "latest"]);
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    let resumed_events = session_events(root.path(), &session_id(root.path()));
    assert!(resumed_events.starts_with(&events));
    assert_eq!(
        resumed_events
            .iter()
            .filter(|event| event["event_type"] == "session.task_selection_usage_recorded")
            .count(),
        1
    );
    assert!(
        !resumed_events
            .iter()
            .any(|event| event["event_type"] == "session.task_selection_sample_recorded")
    );

    use echidna::domain::events::EventEnvelope;
    use echidna::domain::session::SessionReducer;
    let position = events
        .iter()
        .position(|event| event["event_type"] == "session.task_selection_usage_recorded")
        .unwrap();
    let prefix = events[..position]
        .iter()
        .cloned()
        .map(|value| serde_json::from_value::<EventEnvelope<Value>>(value).unwrap())
        .collect::<Vec<_>>();
    for field in [
        "capture_event_id",
        "request_audit_event_id",
        "provider_session",
    ] {
        let mut tampered = events[position].clone();
        match field {
            "capture_event_id" | "request_audit_event_id" => {
                tampered["payload"][field] = json!("not-the-source")
            }
            _ => tampered["payload"][field]["id"] = json!("not-the-scope"),
        }
        let mut state = SessionReducer.reduce(prefix.clone()).unwrap();
        assert!(
            SessionReducer
                .apply(&mut state, serde_json::from_value(tampered).unwrap())
                .is_err(),
            "{field}"
        );
    }
    let mut state = SessionReducer.reduce(prefix).unwrap();
    SessionReducer
        .apply(
            &mut state,
            serde_json::from_value(events[position].clone()).unwrap(),
        )
        .unwrap();
    let mut duplicate = events[position].clone();
    duplicate["event_id"] = json!("duplicate-selector-usage");
    duplicate["sequence"] = json!(events[position]["sequence"].as_u64().unwrap() + 1);
    assert!(
        SessionReducer
            .apply(&mut state, serde_json::from_value(duplicate).unwrap())
            .is_err()
    );
}

#[test]
fn selector_effort_override_is_scoped_and_default_inherits() {
    for override_effort in [None, Some("low")] {
        let root = TemporaryDirectory::new("task-selector-effort");
        initialize_project(root.path());
        let agent = definition(root.path(), None);
        let server = paged_server(|step, wire| {
            if step % 2 == 0 {
                let candidates = selector_candidates(wire);
                (
                    response_with_usage(
                        &decision(&candidates).to_string(),
                        &format!("select-{step}"),
                    ),
                    false,
                )
            } else {
                (text_response("Done.", &format!("main-{step}")), step == 3)
            }
        });
        let config = write_context_window_config(root.path(), &server.base_url, 272_000);
        let mut text = fs::read_to_string(&config).unwrap().replacen(
            "context_window = 272000",
            "context_window = 272000\nreasoning_effort = \"high\"",
            1,
        );
        if let Some(effort) = override_effort {
            text.push_str(&format!(
                "\n[runtime]\ntask_selection_reasoning_effort = \"{effort}\"\n"
            ));
        }
        fs::write(&config, text).unwrap();
        let output = run_chat(
            root.path(),
            &config,
            "Synthetic task.\nAnother synthetic task.\n/exit\n",
            &["--agent", agent.to_str().unwrap()],
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let wires = (0..4)
            .map(|_| f2a_request_body(&server.requests, "selector/main effort"))
            .collect::<Vec<_>>();
        server.join();
        for pair in wires.chunks_exact(2) {
            assert_eq!(
                pair[0]["reasoning"]["effort"],
                override_effort.unwrap_or("high")
            );
            assert_eq!(pair[1]["reasoning"]["effort"], "high");
        }
        let id = session_id(root.path());
        let events = session_events(root.path(), &id);
        let captures = events
            .iter()
            .filter(|event| event["event_type"] == "session.task_context_selected")
            .collect::<Vec<_>>();
        assert_eq!(captures.len(), 2);
        for selection in captures {
            let blob = selection["payload"]["capture_blob"].as_str().unwrap();
            let capture: Value = serde_json::from_slice(
                &fs::read(
                    root.path()
                        .join(".echidna/artifacts/blobs/sha256")
                        .join(blob),
                )
                .unwrap(),
            )
            .unwrap();
            assert_eq!(
                capture["compile"].get("selection_reasoning_effort"),
                override_effort.map(|effort| json!(effort)).as_ref()
            );
            assert_eq!(
                capture["common"]["capabilities"]["reasoning_effort"],
                "high"
            );
        }
        if override_effort.is_some() {
            let text = fs::read_to_string(&config).unwrap().replace(
                "task_selection_reasoning_effort = \"low\"",
                "task_selection_reasoning_effort = \"medium\"",
            );
            fs::write(&config, text).unwrap();
        }
        let resumed = run_chat(root.path(), &config, "/exit\n", &["--resume", "latest"]);
        assert!(
            resumed.status.success(),
            "{}",
            String::from_utf8_lossy(&resumed.stderr)
        );
    }
}

#[test]
fn partial_excerpt_retains_complete_protected_constraint_and_resumes() {
    for mode in ["partial", "full", "invalid_range", "unknown_source"] {
        let root = TemporaryDirectory::new("task-partial-constraint");
        initialize_project(root.path());
        let agent = definition(root.path(), None);
        let marker = "TASK-OVERLAP-ROOT ";
        let length = if mode == "full" { 200 } else { 12_494 };
        let first = format!("{marker}{}", "x".repeat(length - marker.len()));
        let server = paged_server(move |step, wire| match step {
            0 => {
                let candidates = selector_candidates(wire);
                let text = candidates["input"]["text"].as_str().unwrap();
                let mut selected = decision(&candidates);
                selected["constraint_updates"] = json!([{
                    "source_ref":candidates["input"]["ref"], "start_byte":0,
                    "end_byte":text.len(), "scope":{"kind":"current_task"}, "supersedes":null
                }]);
                (
                    text_response(&selected.to_string(), "overlap-select-first"),
                    false,
                )
            }
            1 => (
                text_response("First task complete.", "overlap-main-first"),
                false,
            ),
            2 => {
                let candidates = selector_candidates(wire);
                (
                    text_response(&decision(&candidates).to_string(), "overlap-select-second"),
                    false,
                )
            }
            3 => (
                text_response("Second task complete.", "overlap-main-second"),
                false,
            ),
            4 => {
                let candidates = selector_candidates(wire);
                let task = candidates["tasks"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|task| task["goal"].as_str().unwrap().contains("TASK-OVERLAP-ROOT"))
                    .unwrap();
                let source = candidates["context"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|source| {
                        source["ref"] == task["source_ref"] && source["constraint_range"].is_null()
                    })
                    .unwrap();
                let end = match mode {
                    "full" => 200,
                    "invalid_range" => source["excerpt"].as_str().unwrap().len() + 1,
                    _ => 94,
                };
                let source_ref = if mode == "unknown_source" {
                    json!("missing-source")
                } else {
                    source["ref"].clone()
                };
                let selected = json!({
                    "relation":"return", "focus":{"kind":"existing", "task_ref":task["ref"]},
                    "related_task_refs":[task["ref"]], "required_context_refs":[],
                    "source_ranges":[{"context_ref":source_ref, "start_byte":0, "end_byte":end}],
                    "constraint_updates":[{"source_ref":candidates["input"]["ref"], "start_byte":0,
                        "end_byte":candidates["input"]["text"].as_str().unwrap().len(),
                        "scope":{"kind":"current_task"}, "supersedes":null}],
                    "unresolved_references":[]
                });
                (
                    text_response(&selected.to_string(), "overlap-select-return"),
                    mode != "partial" && mode != "full",
                )
            }
            5 => (text_response("Returned.", "overlap-main-return"), true),
            _ => unreachable!(),
        });
        let config = write_context_window_config(root.path(), &server.base_url, 272_000);
        let output = run_chat(
            root.path(),
            &config,
            &format!(
                "{first}\nStart an unrelated second task.\nReturn to TASK-OVERLAP-ROOT.\n/exit\n"
            ),
            &["--agent", agent.to_str().unwrap()],
        );
        let valid = matches!(mode, "partial" | "full");
        assert_eq!(
            output.status.success(),
            valid,
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let wires = (0..if valid { 6 } else { 5 })
            .map(|_| f2a_request_body(&server.requests, "partial constraint"))
            .collect::<Vec<_>>();
        assert!(server.requests.try_recv().is_err());
        server.join();
        let id = session_id(root.path());
        let events = session_events(root.path(), &id);
        let selections = events
            .iter()
            .filter(|event| event["event_type"] == "session.task_context_selected")
            .collect::<Vec<_>>();
        assert_eq!(selections.len(), if valid { 3 } else { 2 });
        if valid {
            let selected = selections[2];
            assert_eq!(selected["payload"]["request_proof_version"], 8);
            let range = &selected["payload"]["selected_excerpts"][0];
            let constraint = &selected["payload"]["selected_constraints"][0];
            assert_eq!(range["source_event_id"], constraint["source_event_id"]);
            assert_eq!(range["start_byte"], 0);
            assert_eq!(range["end_byte"], if mode == "full" { 200 } else { 94 });
            assert_eq!(constraint["start_byte"], 0);
            assert_eq!(constraint["end_byte"], length);
            assert!(json_contains_text(&wires[5]["input"], &first));
            let state = reduced_session(&Project::discover(root.path()).unwrap(), &id);
            let audit = state.request_audits().last().unwrap();
            let map = serde_json::to_string(audit.context_delivery_map().unwrap()).unwrap();
            assert!(map.contains(range["source_event_id"].as_str().unwrap()));
            assert!(map.contains("task-context-excerpt:1"));
            assert_eq!(map.contains("task-context-constraint:1"), mode == "partial");
            let resumed = run_chat(root.path(), &config, "/exit\n", &["--resume", "latest"]);
            assert!(
                resumed.status.success(),
                "{}",
                String::from_utf8_lossy(&resumed.stderr)
            );
        }
    }
}

#[test]
fn explicitly_requested_superseded_constraint_remains_historical_context() {
    const ORIGINAL: &str =
        "TASK-SCREEN: select sealed stocks, gain 2% to 5%, volume ratio at least 1.5.";
    const CORRECTION: &str = "Change the volume ratio threshold to 2.0; keep all other conditions.";
    for mode in [
        "valid",
        "long_refresh",
        "full_source",
        "full_invalid_range",
        "unknown",
        "unknown_reference",
        "duplicate",
    ] {
        let root = TemporaryDirectory::new("task-superseded-context");
        initialize_project(root.path());
        let agent = definition(root.path(), None);
        let server = paged_server(move |step, wire| match step {
            0 => {
                let candidates = selector_candidates(wire);
                let mut selected = decision(&candidates);
                selected["constraint_updates"] = json!([{
                    "source_ref":candidates["input"]["ref"], "start_byte":0,
                    "end_byte":if mode == "long_refresh" { 12_494 } else { ORIGINAL.len() }, "scope":{"kind":"current_task"}, "supersedes":null
                }]);
                (
                    text_response(&selected.to_string(), "supersede-first-select"),
                    false,
                )
            }
            1 => (
                text_response("Screen complete.", "supersede-first-main"),
                false,
            ),
            2 => {
                let candidates = selector_candidates(wire);
                let task = candidates["tasks"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|task| task["goal"].as_str().unwrap().contains("TASK-SCREEN"))
                    .unwrap();
                let prior = candidates["context"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|candidate| {
                        candidate["constraint_range"].is_object()
                            && candidate["excerpt"]
                                .as_str()
                                .unwrap()
                                .starts_with("TASK-SCREEN")
                    })
                    .unwrap();
                let source = candidates["context"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|candidate| {
                        candidate["source_event_ref"]
                            == prior["constraint_range"]["source_event_id"]
                            && candidate["constraint_range"].is_null()
                    })
                    .unwrap();
                let previous = if mode == "unknown" {
                    json!("missing-constraint")
                } else {
                    prior["ref"].clone()
                };
                let update = json!({"source_ref":candidates["input"]["ref"], "start_byte":0,
                    "end_byte":CORRECTION.len(), "scope":{"kind":"current_task"}, "supersedes":previous});
                let updates = if mode == "duplicate" {
                    vec![update.clone(), update]
                } else {
                    vec![update]
                };
                let mut required = vec![prior["ref"].clone(), prior["ref"].clone()];
                if matches!(mode, "full_source" | "full_invalid_range") {
                    required.extend([source["ref"].clone(), source["ref"].clone()]);
                }
                if mode == "unknown_reference" {
                    required.push(json!("missing-context"));
                }
                let end = if mode == "full_invalid_range" {
                    source["excerpt"].as_str().unwrap().len() + 1
                } else {
                    10
                };
                let selected = json!({"relation":"continue", "focus":{"kind":"existing","task_ref":task["ref"]},
                    "related_task_refs":[task["ref"],task["ref"]], "required_context_refs":required,
                    "source_ranges":[{"context_ref":source["ref"],"start_byte":0,"end_byte":end},
                        {"context_ref":source["ref"],"start_byte":0,"end_byte":end},
                        {"context_ref":source["ref"],"start_byte":5,"end_byte":15}],
                    "constraint_updates":updates, "unresolved_references":["", "  "]});
                (
                    text_response(&selected.to_string(), "supersede-second-select"),
                    !matches!(mode, "valid" | "long_refresh" | "full_source"),
                )
            }
            3 if mode == "long_refresh" => (
                m4_long_flow_tool_response(
                    "supersede-refresh-write",
                    "run_command",
                    json!({"argv":["/bin/sh","-c","printf 'once\\n' >> supersede-effect.txt"]}),
                    "supersede-tool",
                ),
                false,
            ),
            3 | 4 => (
                text_response("Screen corrected.", "supersede-second-main"),
                true,
            ),
            _ => unreachable!(),
        });
        let config = write_context_window_config(root.path(), &server.base_url, 272_000);
        let first = if mode == "long_refresh" {
            format!("{ORIGINAL}{}", "x".repeat(12_494 - ORIGINAL.len()))
        } else {
            format!("{ORIGINAL} UNRELATED-SOURCE-TAIL")
        };
        let output = run_chat(
            root.path(),
            &config,
            &format!("{first}\n{CORRECTION}\n/exit\n"),
            &["--agent", agent.to_str().unwrap()],
        );
        let valid = matches!(mode, "valid" | "long_refresh" | "full_source");
        assert_eq!(
            output.status.success(),
            valid,
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let wires = (0..if mode == "long_refresh" {
            5
        } else if valid {
            4
        } else {
            3
        })
            .map(|_| f2a_request_body(&server.requests, "superseded source"))
            .collect::<Vec<_>>();
        assert!(server.requests.try_recv().is_err());
        server.join();
        let id = session_id(root.path());
        let events = session_events(root.path(), &id);
        let selections = events
            .iter()
            .filter(|event| event["event_type"] == "session.task_context_selected")
            .collect::<Vec<_>>();
        assert_eq!(
            selections.len(),
            if mode == "long_refresh" {
                3
            } else if valid {
                2
            } else {
                1
            }
        );
        if valid {
            let selection = &selections[1]["payload"];
            let revision = &selection["constraint_revisions"][0];
            let old_id = &events
                .iter()
                .find(|event| {
                    event["event_type"] == "session.history_item_recorded"
                        && event["payload"]["item"]["text"] == first
                })
                .unwrap()["event_id"];
            let excerpts = selection["selected_excerpts"].as_array().unwrap();
            if mode == "valid" || mode == "long_refresh" {
                assert_eq!(excerpts.len(), 3);
                assert!(
                    excerpts
                        .iter()
                        .any(|range| range["source_event_id"] == *old_id
                            && range["start_byte"] == 0
                            && range["end_byte"]
                                == if mode == "long_refresh" {
                                    12_494
                                } else {
                                    ORIGINAL.len()
                                })
                );
                assert!(
                    excerpts
                        .iter()
                        .any(|range| range["start_byte"] == 0 && range["end_byte"] == 10)
                );
                assert!(
                    excerpts
                        .iter()
                        .any(|range| range["start_byte"] == 5 && range["end_byte"] == 15)
                );
            } else {
                assert!(excerpts.is_empty());
                assert_eq!(
                    selection["selected_history"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter(|id| *id == old_id)
                        .count(),
                    1
                );
            }
            assert_eq!(
                selection["decision"]["required_context_refs"][0],
                revision["supersedes"]
            );
            assert!(
                selection["selected_constraints"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|range| range["source_event_id"] != *old_id)
            );
            assert!(
                selection["unresolved_gaps"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|gap| !gap.as_str().unwrap().trim().is_empty())
            );
            assert_eq!(
                selection["decision"]["related_task_refs"]
                    .as_array()
                    .unwrap()
                    .len(),
                2
            );
            assert!(json_contains_text(&wires[3]["input"], ORIGINAL));
            assert!(json_contains_text(&wires[3]["input"], CORRECTION));
            let state = reduced_session(&Project::discover(root.path()).unwrap(), &id);
            let map =
                serde_json::to_string(state.request_audits()[3].context_delivery_map().unwrap())
                    .unwrap();
            assert!(map.contains(old_id.as_str().unwrap()));
            assert_eq!(
                map.contains("task-context-excerpt:1"),
                mode != "full_source"
            );
            if mode == "long_refresh" {
                assert_eq!(
                    selections[2]["payload"]["refresh_of"],
                    selections[1]["event_id"]
                );
                assert_eq!(
                    selections[2]["payload"]["selected_excerpts"],
                    selection["selected_excerpts"]
                );
                assert_eq!(
                    fs::read_to_string(root.path().join("supersede-effect.txt")).unwrap(),
                    "once\n"
                );
            }
            let resumed = run_chat(root.path(), &config, "/exit\n", &["--resume", "latest"]);
            assert!(
                resumed.status.success(),
                "{}",
                String::from_utf8_lossy(&resumed.stderr)
            );
        }
    }
}

#[test]
fn context_read_accepts_flat_advertised_fields_for_each_operation() {
    use echidna::domain::model::ContentItem;
    use echidna::domain::tools::ToolResultStatus;
    let root = TemporaryDirectory::new("context-read-flat-operations");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let mut original_ref = String::new();
    let server = paged_server(move |step, wire| match step {
        0 => {
            let candidates = selector_candidates(wire);
            original_ref = candidates["input"]["ref"].as_str().unwrap().to_owned();
            (
                text_response(&decision(&candidates).to_string(), "flat-first-select"),
                false,
            )
        }
        1 => (
            text_response("SOURCE-NEEDLE-429 original observation.", "flat-first-main"),
            false,
        ),
        2 => {
            let candidates = selector_candidates(wire);
            (
                text_response(&decision(&candidates).to_string(), "flat-second-select"),
                false,
            )
        }
        3 => {
            let task = format!("task:{original_ref}");
            let args = |operation: &str| {
                json!({
                    "operation":operation, "query":"SOURCE-NEEDLE-429", "reference":original_ref,
                    "task_ref":task, "offset":0, "max_bytes":12000, "after_unix":null,
                    "before_unix":null, "cursor":null, "execution":{"rerun_reason":"Inspect earlier source."}
                })
            };
            let mut bad_query = args("search");
            bad_query["query"] = json!(17);
            let mut missing_reference = args("read");
            missing_reference
                .as_object_mut()
                .unwrap()
                .remove("reference");
            let calls = [
                ("flat-search", "context_read", args("search")),
                ("flat-read", "context_read", args("read")),
                ("flat-progress", "context_read", args("progress")),
                ("flat-bad-query", "context_read", bad_query),
                ("flat-missing-reference", "context_read", missing_reference),
                ("flat-unknown-operation", "context_read", args("unknown")),
            ];
            (tool_calls_response(&calls, "flat-calls"), false)
        }
        4 => (text_response("Reads complete.", "flat-final"), true),
        _ => unreachable!(),
    });
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "Record SOURCE-NEEDLE-429 as the original request.\nRead the earlier source.\n/exit\n",
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for _ in 0..5 {
        f2a_request_body(&server.requests, "flat context read");
    }
    assert!(server.requests.try_recv().is_err());
    server.join();
    let id = session_id(root.path());
    let state = reduced_session(&Project::discover(root.path()).unwrap(), &id);
    state.history().ensure_all_tool_calls_paired().unwrap();
    let results = state
        .history()
        .items()
        .iter()
        .filter_map(|item| match item {
            ContentItem::ToolResult { result } => Some((result.call_id.as_str(), result)),
            _ => None,
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    for id in ["flat-search", "flat-read", "flat-progress"] {
        assert_eq!(
            results[id].status,
            ToolResultStatus::Succeeded,
            "{id}: {:?}",
            results[id]
        );
        assert_eq!(results[id].output["status"], "found", "{id}");
    }
    assert!(
        results["flat-search"]
            .output
            .to_string()
            .contains("SOURCE-NEEDLE-429")
    );
    assert!(
        results["flat-read"]
            .output
            .to_string()
            .contains("SOURCE-NEEDLE-429")
    );
    for id in [
        "flat-bad-query",
        "flat-missing-reference",
        "flat-unknown-operation",
    ] {
        assert_eq!(results[id].status, ToolResultStatus::Failed);
        assert_eq!(results[id].output["failure"]["stage"], "pre_execution");
    }
    assert_eq!(state.execution_receipt_count(), 3);
    let call = state
        .history()
        .items()
        .iter()
        .find_map(|item| match item {
            ContentItem::ToolCall { call } if call.call_id.as_str() == "flat-search" => Some(call),
            _ => None,
        })
        .unwrap();
    assert_eq!(call.arguments["offset"], 0);
    assert!(call.arguments["execution"].is_object());
    let resumed = run_chat(root.path(), &config, "/exit\n", &["--resume", "latest"]);
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
}
