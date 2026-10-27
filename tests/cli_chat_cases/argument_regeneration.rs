//! Offline protocol recovery over the real CLI, audit and Tool runtime.
use super::*;

fn invalid_sample() -> String {
    [
        json!({"type":"response.output_text.delta","delta":"UNCOMMITTED_PROSE"}),
        json!({"type":"response.output_item.done","item":{
            "type":"function_call","call_id":"uncommitted-sibling","name":"write_file",
            "arguments":json!({"path":"must-not-exist.txt","content":"uncommitted","expected_hash":null}).to_string()}}),
        json!({"type":"response.output_item.done","item":{
            "type":"function_call","call_id":"private-call","name":"private-tool",
            "arguments":"{\"content\":\"PRIVATE_CANARY\\q\"}"}}),
        json!({"type":"response.completed","response":{"id":"must-not-continue"}}),
    ].iter().map(|event| format!("data: {event}\n\n")).collect()
}

fn reasoned_call(id: &str, name: &str, args: Value, response: &str) -> String {
    let reasoning = json!({"type":"response.output_item.done","item":{
        "type":"reasoning","content":[{"type":"reasoning_text","text":"RETAIN_VALID_REASONING"}]}});
    format!(
        "data: {reasoning}\n\n{}",
        m4_long_flow_tool_response(id, name, args, response)
    )
}

fn rejection_events(root: &std::path::Path) -> Vec<Value> {
    session_events(root, &session_id(root))
        .into_iter()
        .filter(|event| event["event_type"] == "model.sample_rejected")
        .collect()
}

fn replay_events(
    events: Vec<Value>,
) -> Result<echidna::domain::session::SessionState, echidna::domain::session::SessionError> {
    SessionReducer.reduce(
        events
            .into_iter()
            .map(|event| {
                serde_json::from_value::<echidna::domain::events::EventEnvelope<Value>>(event)
                    .unwrap()
            })
            .collect::<Vec<_>>(),
    )
}

#[test]
fn argument_correction_retains_prior_effects_reasoning_and_exact_audit_without_history_feedback() {
    for incremental in [false, true] {
        let root = TemporaryDirectory::new("argument-correction");
        let harness = TemporaryDirectory::new("argument-correction-provider");
        initialize_project(root.path());
        let server = spawn_responses_server(vec![
            (
                200,
                reasoned_call(
                    "done-once",
                    "run_command",
                    json!({"argv":["sh","-c","printf x >> marker; printf done"]}),
                    "committed-before-error",
                ),
            ),
            (200, invalid_sample()),
            (
                200,
                m4_long_flow_tool_response(
                    "corrected",
                    "write_file",
                    json!({"path":"corrected.txt","content":"corrected","expected_hash":null}),
                    "corrected-response",
                ),
            ),
            (200, text_response("RECOVERED", "final")),
            (200, text_response("NEXT_TURN", "next")),
        ]);
        let config = write_context_window_config(harness.path(), &server.base_url, 1_048_576);
        if !incremental {
            fs::write(
                &config,
                fs::read_to_string(&config)
                    .unwrap()
                    .replace("incremental_input = true", "incremental_input = false"),
            )
            .unwrap();
        }
        let output = run_chat(
            root.path(),
            &config,
            "/phase experiment\nWrite two ordinary notes.\nReport current status.\n/exit\n",
            &[],
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{stderr}");
        assert!(stderr.contains("[argument correction 1/2]"), "{stderr}");
        assert!(!stderr.contains("[turn stopped]"));
        assert_eq!(fs::read(root.path().join("marker")).unwrap(), b"x");
        assert_eq!(
            fs::read(root.path().join("corrected.txt")).unwrap(),
            b"corrected"
        );
        assert!(!root.path().join("must-not-exist.txt").exists());
        let wires: Vec<_> = (0..5)
            .map(|_| f2a_request_body(&server.requests, "argument correction"))
            .collect();
        assert!(
            wires[2]["input"]
                .to_string()
                .contains("Runtime validation notice")
        );
        for wire in &wires {
            assert!(!wire.to_string().contains("PRIVATE_CANARY"));
            assert!(!wire.to_string().contains("UNCOMMITTED_PROSE"));
            assert!(!wire.to_string().contains("uncommitted-sibling"));
        }
        if incremental {
            assert_eq!(wires[1]["previous_response_id"], "committed-before-error");
            assert_eq!(wires[2]["previous_response_id"], "committed-before-error");
            assert_eq!(wires[3]["previous_response_id"], "corrected-response");
        } else {
            assert!(
                wires
                    .iter()
                    .all(|wire| wire.get("previous_response_id").is_none())
            );
            for wire in &wires[2..] {
                assert!(wire["input"].to_string().contains("RETAIN_VALID_REASONING"));
                assert_eq!(
                    wire["input"]
                        .to_string()
                        .matches("Runtime validation notice")
                        .count(),
                    1
                );
            }
        }
        assert!(server.requests.try_recv().is_err());
        server.join();
        let state = reduced_session(
            &Project::discover(root.path()).unwrap(),
            &session_id(root.path()),
        );
        assert_eq!(state.execution_receipt_count(), 2);
        assert_eq!(state.request_audits().len(), 5);
        let history = serde_json::to_string(state.history().items()).unwrap();
        for omitted in [
            "Runtime validation notice",
            "UNCOMMITTED_PROSE",
            "RETAIN_VALID_REASONING",
            "uncommitted-sibling",
        ] {
            assert!(!history.contains(omitted));
        }
        let rejection = rejection_events(root.path());
        assert_eq!(rejection.len(), 1);
        assert_eq!(
            rejection[0]["payload"]["request_audit_event_id"],
            state.request_audits()[1].audit_event_id().as_str()
        );
        let correction_map = state.request_audits()[2].context_delivery_map().unwrap();
        let feedback = correction_map
            .content_items()
            .iter()
            .find(|item| item.fragment_id().as_str().starts_with("correction:"))
            .unwrap();
        assert_eq!(
            feedback.provenance().origin(),
            echidna::domain::context::ContextOrigin::SessionProjection
        );
        assert_eq!(
            feedback.provenance().authority_level(),
            echidna::domain::context::AuthorityLevel::DerivedNonEvidence
        );
        assert_audits_reconstruct(root.path(), &state);
    }
}

#[test]
fn accepted_samples_reset_consecutive_corrections_with_pending_calls_and_without_usage() {
    let root = TemporaryDirectory::new("correction-success-reset");
    initialize_project(root.path());
    let server = spawn_responses_server(vec![
        (200, invalid_sample()),
        (
            200,
            m4_long_flow_tool_response(
                "success-one",
                "run_command",
                json!({"argv":["sh","-c","printf one >> marker"]}),
                "one",
            ),
        ),
        (200, invalid_sample()),
        (
            200,
            m4_long_flow_tool_response(
                "success-two",
                "run_command",
                json!({"argv":["sh","-c","printf two >> marker"]}),
                "two",
            ),
        ),
        (200, invalid_sample()),
        (200, text_response("ACCEPTED_WITHOUT_USAGE", "accepted")),
    ]);
    let config = write_context_window_config(root.path(), &server.base_url, 1_048_576);
    let output = run_chat(
        root.path(),
        &config,
        "/phase experiment\nWrite two ordinary notes and report completion.\n/exit\n",
        &[],
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert!(!stderr.contains("[turn stopped]"), "{stderr}");
    assert_eq!(stderr.matches("[argument correction 1/2]").count(), 3);
    assert!(!stderr.contains("[argument correction 2/2]"));
    assert!(String::from_utf8_lossy(&output.stdout).contains("ACCEPTED_WITHOUT_USAGE"));
    assert_eq!(fs::read(root.path().join("marker")).unwrap(), b"onetwo");
    assert!(!root.path().join("must-not-exist.txt").exists());
    let wires: Vec<_> = (0..6)
        .map(|_| f2a_request_body(&server.requests, "bounded correction"))
        .collect();
    assert_eq!(wires[3]["previous_response_id"], "one");
    assert_eq!(wires[5]["previous_response_id"], "two");
    assert!(server.requests.try_recv().is_err());
    server.join();
    let state = reduced_session(
        &Project::discover(root.path()).unwrap(),
        &session_id(root.path()),
    );
    assert_eq!(state.execution_receipt_count(), 2);
    assert_eq!(state.history().ensure_all_tool_calls_paired(), Ok(()));
    let events = session_events(root.path(), &session_id(root.path()));
    let rejections: Vec<_> = events
        .iter()
        .filter(|event| event["event_type"] == "model.sample_rejected")
        .collect();
    let resets: Vec<_> = events
        .iter()
        .enumerate()
        .filter(|(_, event)| event["event_type"] == "model.argument_correction_reset")
        .collect();
    assert_eq!(rejections.len(), 3);
    assert_eq!(resets.len(), 3);
    assert!(
        !events
            .iter()
            .any(|event| event["event_type"] == "model.usage_recorded")
    );
    for (index, rejection) in rejections.iter().enumerate() {
        assert_eq!(rejection["payload"]["schema_version"], 2);
        assert_eq!(rejection["payload"]["corrections_used"], 0);
        assert_eq!(
            rejection["payload"]["reset_anchor"],
            if index == 0 {
                Value::Null
            } else {
                resets[index - 1].1["event_id"].clone()
            }
        );
        let (reset_index, reset) = resets[index];
        assert_eq!(
            reset["payload"]["accepted_request_audit_event_id"],
            state.request_audits()[2 * index + 1]
                .audit_event_id()
                .as_str()
        );
        assert_eq!(
            reset["payload"]["prior_rejection_audit_event_id"],
            state.request_audits()[2 * index].audit_event_id().as_str()
        );
        assert_eq!(
            reset["payload"]["accepted_history_event_refs"],
            json!([events[reset_index - 1]["event_id"]])
        );
        let accepted_prefix = replay_events(events[..=reset_index].to_vec()).unwrap();
        assert_eq!(accepted_prefix.execution_receipt_count(), index as u64);
        assert_eq!(
            accepted_prefix
                .history()
                .ensure_all_tool_calls_paired()
                .is_ok(),
            index == 2
        );
    }
    assert_audits_reconstruct(root.path(), &state);

    // Historical v1 keeps its original per-Turn counters and needs no reset fact.
    let mut legacy: Vec<_> = events
        .into_iter()
        .filter(|event| event["event_type"] != "model.argument_correction_reset")
        .collect();
    let mut used = 0;
    for (index, event) in legacy.iter_mut().enumerate() {
        event["sequence"] = json!(index + 1);
        if event["event_type"] == "model.sample_rejected" {
            event["payload"]["schema_version"] = json!(1);
            event["payload"]["corrections_used"] = json!(used);
            event["payload"]
                .as_object_mut()
                .unwrap()
                .remove("reset_anchor");
            used += 1;
        }
    }
    assert!(replay_events(legacy).is_ok());
}

#[test]
fn consecutive_correction_limit_stops_and_explicit_resume_does_not_requeue() {
    let root = TemporaryDirectory::new("correction-chain-bound");
    initialize_project(root.path());
    let server = spawn_responses_server(vec![
        (
            200,
            m4_long_flow_tool_response(
                "once",
                "run_command",
                json!({"argv":["sh","-c","printf x >> marker"]}),
                "prior-success",
            ),
        ),
        (200, invalid_sample()),
        (200, invalid_sample()),
        (200, invalid_sample()),
        (200, text_response("EXPLICIT_NEW_TASK", "resumed")),
    ]);
    let config = write_context_window_config(root.path(), &server.base_url, 1_048_576);
    let output = run_chat(
        root.path(),
        &config,
        "/phase experiment\nInspect an ordinary fixture.\nnot accepted\n/resume\nNew explicit task.\n/exit\n",
        &[],
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert!(
        stderr.contains("argument corrections exhausted"),
        "{stderr}"
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Task input not accepted or saved"));
    let wires: Vec<_> = (0..5)
        .map(|_| f2a_request_body(&server.requests, "consecutive correction bound"))
        .collect();
    assert!(wires[4].get("previous_response_id").is_none());
    assert!(
        !wires[4]["input"]
            .to_string()
            .contains("Runtime validation notice")
    );
    assert!(!wires[4]["input"].to_string().contains("not accepted"));
    assert!(server.requests.try_recv().is_err());
    server.join();
    assert_eq!(fs::read(root.path().join("marker")).unwrap(), b"x");
    assert!(!root.path().join("must-not-exist.txt").exists());
    let state = reduced_session(
        &Project::discover(root.path()).unwrap(),
        &session_id(root.path()),
    );
    assert_eq!(state.execution_receipt_count(), 1);
    assert_eq!(state.history().ensure_all_tool_calls_paired(), Ok(()));
    let rejections = rejection_events(root.path());
    assert_eq!(rejections.len(), 3);
    for (index, rejection) in rejections.iter().enumerate() {
        assert_eq!(rejection["payload"]["corrections_used"], index);
    }
    assert!(
        !session_events(root.path(), &session_id(root.path()))
            .iter()
            .any(|event| event["event_type"] == "model.argument_correction_reset")
    );
    assert_audits_reconstruct(root.path(), &state);
}

#[test]
fn argument_correction_respects_the_existing_request_budget() {
    let root = TemporaryDirectory::new("correction-admission");
    initialize_project(root.path());
    let server = spawn_responses_server(vec![(200, invalid_sample())]);
    let config = write_config(root.path(), &server.base_url);
    fs::write(
        &config,
        format!(
            "{}\n[runtime]\nmodel_request_limit_per_turn = 1\n",
            fs::read_to_string(&config).unwrap()
        ),
    )
    .unwrap();
    let output = run_chat(root.path(), &config, "Inspect a note.\n/exit\n", &[]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("model-request limit of 1"));
    f2a_request_body(&server.requests, "only admitted attempt");
    assert!(server.requests.try_recv().is_err());
    server.join();
    let state = reduced_session(
        &Project::discover(root.path()).unwrap(),
        &session_id(root.path()),
    );
    assert_eq!(state.request_audits().len(), 1);
    assert_eq!(state.execution_receipt_count(), 0);
    assert_eq!(rejection_events(root.path()).len(), 1);
}

#[test]
fn rejection_checkpoint_failure_is_fatal_before_regeneration() {
    use std::os::unix::fs::PermissionsExt;
    let root = TemporaryDirectory::new("rejection-durability");
    initialize_project(root.path());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let config = write_config(
        root.path(),
        &format!("http://{}", listener.local_addr().unwrap()),
    );
    let target = root.path().to_owned();
    let server = thread::spawn(move || {
        let mut socket = f3_context::accept_before_deadline(&listener, "rejection durability");
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        read_http_request(&mut socket);
        let events = target
            .join(".echidna/sessions")
            .join(session_id(&target))
            .join("events.jsonl");
        fs::set_permissions(events, fs::Permissions::from_mode(0o444)).unwrap();
        f3_context::write_sse_response(&mut socket, &invalid_sample());
    });
    let output = run_chat(root.path(), &config, "Inspect a note.\n/exit\n", &[]);
    server.join().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("model rejection checkpoint failed"),
        "{stderr}"
    );
    assert!(!stderr.contains("[argument correction"));
    let events = session_events(root.path(), &session_id(root.path()));
    assert_eq!(
        events
            .iter()
            .filter(|e| e["event_type"] == "session.request_audit_recorded")
            .count(),
        1
    );
    assert!(
        !events
            .iter()
            .any(|e| e["event_type"] == "model.sample_rejected")
    );
    assert!(!root.path().join("must-not-exist.txt").exists());
}

#[test]
fn rejection_event_replay_rejects_wrong_audit_duplicate_unknown_schema_and_excess_budget() {
    let root = TemporaryDirectory::new("rejection-replay");
    initialize_project(root.path());
    let server = spawn_responses_server(vec![
        (200, invalid_sample()),
        (200, text_response("ok", "valid")),
    ]);
    let config = write_config(root.path(), &server.base_url);
    let output = run_chat(root.path(), &config, "Check a note.\n/exit\n", &[]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for _ in 0..2 {
        f2a_request_body(&server.requests, "replay request");
    }
    server.join();
    let events = session_events(root.path(), &session_id(root.path()));
    let rejected = events
        .iter()
        .position(|e| e["event_type"] == "model.sample_rejected")
        .unwrap();
    for field in [
        "request_audit_event_id",
        "schema_version",
        "corrections_used",
        "reset_anchor",
        "unknown",
    ] {
        let mut tampered = events[..=rejected].to_vec();
        tampered[rejected]["payload"][field] =
            if matches!(field, "request_audit_event_id" | "reset_anchor") {
                json!("not-the-request")
            } else {
                json!(99)
            };
        assert!(replay_events(tampered).is_err(), "accepted {field}");
    }
    let mut duplicate = events[..=rejected].to_vec();
    let mut second = duplicate[rejected].clone();
    second["event_id"] = json!("duplicate-rejection");
    second["sequence"] = json!(second["sequence"].as_u64().unwrap() + 1);
    duplicate.push(second);
    assert!(replay_events(duplicate).is_err());
    let accepted_history = events
        .iter()
        .rposition(|event| event["event_type"] == "session.history_item_recorded")
        .unwrap();
    let accepted_audit = events[..accepted_history]
        .iter()
        .rfind(|event| event["event_type"] == "session.request_audit_recorded")
        .unwrap();
    let mut late = events[rejected].clone();
    assert_eq!(late["payload"]["schema_version"], 2);
    late["event_id"] = json!("rejection-after-accepted-history");
    late["sequence"] = json!(events[accepted_history]["sequence"].as_u64().unwrap() + 1);
    late["payload"]["request_audit_event_id"] = accepted_audit["event_id"].clone();
    late["payload"]["corrections_used"] = json!(1);
    let prefix = events[..=accepted_history].to_vec();
    let before = replay_events(prefix).unwrap();
    let mut after = before.clone();
    assert!(
        SessionReducer
            .apply(&mut after, serde_json::from_value(late).unwrap())
            .is_err()
    );
    assert_eq!(
        after, before,
        "a rejected late event must not mutate the projection"
    );
    assert!(replay_events(events).is_ok());
}

#[test]
fn correction_reset_replay_requires_exact_audit_rejection_and_accepted_history_suffix() {
    let root = TemporaryDirectory::new("correction-reset-replay");
    initialize_project(root.path());
    let accepted = format!(
        "data: {}\n\n{}",
        json!({"type":"response.output_text.delta","delta":"Accepted note before the call."}),
        m4_long_flow_tool_response(
            "accepted-call",
            "run_command",
            json!({"argv":["printf","ordinary note"]}),
            "accepted-call-response"
        )
    );
    let server = spawn_responses_server(vec![
        (200, invalid_sample()),
        (200, accepted),
        (200, text_response("Done.", "done")),
    ]);
    let config = write_config(root.path(), &server.base_url);
    let output = run_chat(
        root.path(),
        &config,
        "/phase experiment\nInspect one ordinary note.\n/exit\n",
        &[],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for _ in 0..3 {
        f2a_request_body(&server.requests, "reset source binding");
    }
    assert!(server.requests.try_recv().is_err());
    server.join();
    let events = session_events(root.path(), &session_id(root.path()));
    let reset_index = events
        .iter()
        .position(|event| event["event_type"] == "model.argument_correction_reset")
        .unwrap();
    let reset = &events[reset_index];
    let refs = reset["payload"]["accepted_history_event_refs"]
        .as_array()
        .unwrap();
    assert_eq!(refs.len(), 2);
    let before = replay_events(events[..reset_index].to_vec()).unwrap();
    assert!(before.history().ensure_all_tool_calls_paired().is_err());
    for (field, value) in [
        ("accepted_request_audit_event_id", json!("wrong-audit")),
        (
            "accepted_request_audit_event_id",
            reset["payload"]["prior_rejection_audit_event_id"].clone(),
        ),
        ("prior_rejection_audit_event_id", json!("wrong-rejection")),
        ("accepted_history_event_refs", json!([])),
        ("accepted_history_event_refs", json!([refs[0]])),
        ("accepted_history_event_refs", json!([refs[1], refs[0]])),
        (
            "accepted_history_event_refs",
            json!([refs[0], "wrong-history"]),
        ),
        ("schema_version", json!(99)),
        ("unknown", json!(true)),
    ] {
        let mut bad = reset.clone();
        bad["payload"][field] = value;
        let mut state = before.clone();
        assert!(
            SessionReducer
                .apply(&mut state, serde_json::from_value(bad).unwrap())
                .is_err(),
            "accepted reset field {field}"
        );
        assert_eq!(state, before, "failed reset mutated state for {field}");
    }
    let accepted_audit_index = events[..reset_index]
        .iter()
        .rposition(|event| event["event_type"] == "session.request_audit_recorded")
        .unwrap();
    let mut empty_suffix = events[..=accepted_audit_index].to_vec();
    let mut early_reset = reset.clone();
    early_reset["sequence"] = json!(empty_suffix.len() + 1);
    early_reset["payload"]["accepted_history_event_refs"] = json!([]);
    empty_suffix.push(early_reset);
    assert!(
        replay_events(empty_suffix).is_err(),
        "empty sample reset the chain"
    );

    let mut user_suffix = events[..=reset_index].to_vec();
    let text_index = user_suffix
        .iter()
        .position(|event| event["event_id"] == refs[0])
        .unwrap();
    user_suffix[text_index]["payload"]["item"] = serde_json::to_value(
        echidna::domain::model::ContentItem::user_text("Not an accepted assistant sample"),
    )
    .unwrap();
    assert!(
        replay_events(user_suffix).is_err(),
        "user input reset the chain"
    );

    let later_audit = events[reset_index + 1..]
        .iter()
        .find(|event| event["event_type"] == "session.request_audit_recorded")
        .unwrap();
    let mut stale_suffix = events[..reset_index].to_vec();
    let mut newer_audit = later_audit.clone();
    newer_audit["sequence"] = json!(stale_suffix.len() + 1);
    stale_suffix.push(newer_audit);
    let mut rebound_reset = reset.clone();
    rebound_reset["sequence"] = json!(stale_suffix.len() + 1);
    rebound_reset["payload"]["accepted_request_audit_event_id"] = later_audit["event_id"].clone();
    stale_suffix.push(rebound_reset);
    assert!(
        replay_events(stale_suffix).is_err(),
        "earlier History proved a newer unaccepted audit"
    );
    assert!(replay_events(events).is_ok());
}
