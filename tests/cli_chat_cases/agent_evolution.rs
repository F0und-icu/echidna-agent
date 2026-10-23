use super::*;

pub(super) fn improvement_definition(root: &std::path::Path) -> std::path::PathBuf {
    let path = definition(root, true);
    let manifest = path.join("agent.toml");
    let text = fs::read_to_string(&manifest)
        .unwrap()
        .replace("[rsi]", "[rsi]\nimprovement = true");
    fs::write(manifest, text).unwrap();
    path
}

fn improvement_candidate() -> Value {
    json!({"decision":"candidate","reason":"Use one supported parser invocation for this CSV task.","basis":["friction:0"],"cue":"CSV aggregation","persona":null,"methods":["For CSV aggregation, invoke the current checked probe once with good mode; preserve input bytes and verify outputs."],"probe":{"task":"CSV aggregation: compute the checked result using probe.sh, choosing its supported mode.","inputs":["probe.sh"],"source_call_id":"main-reference"}})
}

fn improvement_fixture(root: &std::path::Path) {
    fs::write(
        root.join("probe.sh"),
        "case \"$1\" in fail) exit 1;; good) printf '42\\n';; *) printf '41\\n';; esac\n",
    )
    .unwrap();
}

fn repeated_rsi_audit(prefix: &[Value], audit: &Value, id: &str) -> Vec<Value> {
    let mut repeated = audit.clone();
    repeated["event_id"] = json!(id);
    repeated["sequence"] = json!(prefix.last().unwrap()["sequence"].as_u64().unwrap() + 1);
    repeated["payload"]["audit_event_id"] = json!(id);
    let mut events = prefix.to_vec();
    events.push(repeated);
    events
}

fn assert_rejected_rsi_events(events: Vec<Value>, case: &str) {
    let parsed = events
        .into_iter()
        .map(|event| serde_json::from_value::<EventEnvelope<Value>>(event).unwrap())
        .collect::<Vec<_>>();
    assert!(
        SessionReducer.reduce(parsed).is_err(),
        "RSI retry fixture accepted {case}"
    );
}

fn assert_rsi_retry_audit_integrity(events: &[Value]) {
    let audits = events
        .iter()
        .enumerate()
        .filter(|(_, event)| {
            event["event_type"] == "session.request_audit_recorded"
                && !event["payload"]["improvement_scope"].is_null()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        audits.len(),
        3,
        "two diagnosis attempts and one Old attempt"
    );
    let (second_index, second_audit) = audits[1];
    let third = repeated_rsi_audit(
        &events[..=second_index],
        second_audit,
        "fixture-third-diagnosis-audit",
    );
    SessionReducer
        .reduce(
            third
                .iter()
                .cloned()
                .map(|event| serde_json::from_value::<EventEnvelope<Value>>(event).unwrap()),
        )
        .expect("a pending exact request permits its third physical attempt");
    assert_rejected_rsi_events(
        repeated_rsi_audit(&third, second_audit, "fixture-fourth-diagnosis-audit"),
        "fourth physical attempt",
    );
    for (field, replacement) in [
        ("/model/model_id", json!("other-model")),
        ("/profile/generation", json!(99)),
        ("/binding/project_id", json!("other-project")),
        ("/capability_mapping/semantic_hash", json!("0".repeat(64))),
        ("/full_logical_input/semantic_hash", json!("0".repeat(64))),
        ("/delivered_input/semantic_hash", json!("0".repeat(64))),
        ("/resolved_tool_plan_fingerprint", json!("0".repeat(64))),
        (
            "/improvement_scope",
            audits[2].1["payload"]["improvement_scope"].clone(),
        ),
    ] {
        let mut changed = third.clone();
        *changed.last_mut().unwrap()["payload"]
            .pointer_mut(field)
            .unwrap() = replacement;
        assert_rejected_rsi_events(changed, field);
    }
    let mut changed_inputs = third.clone();
    let request = &mut changed_inputs.last_mut().unwrap()["payload"];
    request["full_logical_input"]["semantic_hash"] = json!("0".repeat(64));
    request["delivered_input"]["semantic_hash"] = json!("0".repeat(64));
    assert_rejected_rsi_events(
        changed_inputs,
        "matching but changed logical/delivered input",
    );
    let sample = events
        .iter()
        .position(|event| {
            event["payload"]["fact"]["kind"] == "sample"
                && event["payload"]["fact"]["stage"] == "diagnosis"
        })
        .unwrap();
    let mut stale = events[..=sample].to_vec();
    stale[sample]["payload"]["fact"]["audit_event_id"] = audits[0].1["event_id"].clone();
    assert_rejected_rsi_events(stale, "sample bound to superseded audit");
    for terminal in [
        sample,
        events
            .iter()
            .position(|event| event["payload"]["fact"]["kind"] == "finished")
            .unwrap(),
    ] {
        assert_rejected_rsi_events(
            repeated_rsi_audit(
                &events[..=terminal],
                second_audit,
                "fixture-reopened-diagnosis",
            ),
            "completed stage or finished activity reopened",
        );
    }
}

#[test]
fn rsi_retry_budget_exhaustion_closes_activity_without_executing_a_partial_probe() {
    let root = TemporaryDirectory::new("rsi-retry-budget");
    initialize_project(root.path());
    let definition = improvement_definition(root.path());
    improvement_fixture(root.path());
    let marker = root.path().join("partial-probe-must-not-run");
    let marker_for_response = marker.clone();
    let server = server(
        move |step, request| match step {
            0 => tool(
                "main-failed",
                "run_command",
                json!({"argv":["/bin/sh","probe.sh","fail"]}),
            ),
            1 => tool(
                "main-reference",
                "run_command",
                json!({"argv":["/bin/sh","probe.sh","good"],"code_inputs":["probe.sh"]}),
            ),
            2 => text_response("ORDINARY_TASK_COMPLETED", "ordinary-complete"),
            3 => {
                assert!(request.to_string().contains("RSI2 diagnosis"));
                "data: {\"type\":\"response.output_text.delta\",\"delta\":\"discarded incomplete diagnosis\"}\n\n".to_owned()
            }
            4 => text_response(&improvement_candidate().to_string(), "diagnosis-recovered"),
            5 => {
                assert!(request.to_string().contains("Arm=Old"));
                format!(
                    "data: {}\n\n",
                    json!({"type":"response.output_item.done","output_index":0,"item":{
                        "type":"function_call","call_id":"partial-probe","name":"run_command",
                        "arguments":json!({"argv":["/usr/bin/touch",marker_for_response]}).to_string()
                    }})
                )
            }
            _ => unreachable!(),
        },
        6,
    );
    let config = write_config(root.path(), &server.base_url);
    fs::write(
        &config,
        format!(
            "{}\n[runtime]\nmodel_request_limit_per_turn = 6\n",
            fs::read_to_string(&config).unwrap()
        ),
    )
    .unwrap();
    let output = run_chat(
        root.path(),
        &config,
        "Complete the CSV aggregation task.\n/resume\n/exit\n",
        &["--agent", definition.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let requests = server.requests.try_iter().collect::<Vec<_>>();
    assert_eq!(requests.len(), 6);
    assert_eq!(
        request_body(&requests[3]),
        request_body(&requests[4]),
        "diagnosis retries keep exact logical request bytes"
    );
    server.join();
    assert!(
        !marker.exists(),
        "partial probe call cannot dispatch before completion"
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("[RSI2 inconclusive]"));
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("No stopped Turn needs recovery; no ProviderSession was replaced."),
        "an inconclusive maintenance sample must not stop the completed operator Turn"
    );
    let session = session_id(root.path());
    let resumed = run_chat(
        root.path(),
        &config,
        "/agent\n/exit\n",
        &["--resume", &session],
    );
    assert!(
        resumed.status.success(),
        "explicit resume must accept the closed RSI activity: {}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    assert!(String::from_utf8_lossy(&resumed.stdout).contains("Resumed session"));
    let events = session_events(root.path(), &session);
    assert_rsi_retry_audit_integrity(&events);
    assert_eq!(
        events
            .iter()
            .filter(|event| event["event_type"] == "tool.execution_receipt_recorded")
            .count(),
        2,
        "only the two original ordinary commands executed"
    );
    let facts = events
        .iter()
        .filter(|event| event["event_type"] == "session.rsi_activity_recorded")
        .map(|event| &event["payload"]["fact"])
        .collect::<Vec<_>>();
    assert!(
        facts
            .iter()
            .any(|fact| fact["kind"] == "finished" && fact["disposition"] == "inconclusive")
    );
    assert!(
        facts
            .iter()
            .all(|fact| fact["stage"] != "old" && fact["stage"] != "new"),
        "no incomplete arm sample or result becomes durable"
    );
    assert!(
        events
            .iter()
            .all(|event| event["event_type"] != "session.agent_definition_applied")
    );
}

#[test]
fn rsi2_reference_match_uses_actual_scoped_commands_and_only_adopts_a_local_gain() {
    for case in [
        "gain",
        "tie",
        "regression",
        "echo",
        "mutated",
        "no-change",
        "bad-json",
        "empty-reason",
    ] {
        let root = TemporaryDirectory::new(&format!("rsi2-{case}"));
        initialize_project(root.path());
        let definition = improvement_definition(root.path());
        improvement_fixture(root.path());
        let count = if matches!(case, "no-change" | "bad-json" | "empty-reason") {
            4
        } else {
            6
        };
        let server = server(
            move |step, request| match step {
                0 => tool(
                    "main-failed",
                    "run_command",
                    json!({"argv":["/bin/sh","probe.sh","fail"]}),
                ),
                1 => tool(
                    "main-reference",
                    "run_command",
                    json!({"argv":["/bin/sh","probe.sh","good"],"code_inputs":["probe.sh"]}),
                ),
                2 => text_response("ORDINARY_TASK_COMPLETED", "main-done"),
                3 => {
                    assert!(request.to_string().contains("RSI2 diagnosis"));
                    assert!(request.get("tools").is_none());
                    let mut candidate = improvement_candidate();
                    let text = match case {
                    "no-change" => json!({"decision":"no_change","reason":"Temporary failure; no reusable change."}).to_string(),
                    "bad-json" => "not a candidate".to_owned(),
                    "empty-reason" => {candidate["reason"]=json!("");candidate.to_string()},
                    _ => candidate.to_string(),
                };
                    text_response(&text, "diagnosis-done")
                }
                4 | 5 => {
                    assert!(request.to_string().contains("RSI2 reference_match probe"));
                    assert!(request.get("previous_response_id").is_none());
                    assert_eq!(request["tools"].as_array().unwrap().len(), 1);
                    let mode = if case == "tie"
                        || (case == "regression" && step == 4)
                        || (step == 5 && case != "regression")
                    {
                        "good"
                    } else {
                        "bad"
                    };
                    let args = if case == "echo" && step == 5 {
                        json!({"argv":["/usr/bin/printf","PASS\\n"]})
                    } else if case == "mutated" && step == 5 {
                        json!({"argv":["/bin/sh","-c","printf changed > probe.sh; printf '42\\n'"]})
                    } else {
                        json!({"argv":["/bin/sh","probe.sh",mode]})
                    };
                    tool("same-provider-call-id", "run_command", args)
                }
                _ => unreachable!(),
            },
            count,
        );
        let config = write_config(root.path(), &server.base_url);
        let input = if case == "gain" {
            "CSV aggregation: fix the command failure and finish.\n/compact\n/exit\n"
        } else {
            "CSV aggregation: fix the command failure and finish.\n/exit\n"
        };
        let output = run_chat(
            root.path(),
            &config,
            input,
            &["--agent", definition.to_str().unwrap()],
        );
        assert!(
            output.status.success(),
            "{case}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let requests = server.requests.try_iter().collect::<Vec<_>>();
        assert_eq!(requests.len(), count);
        server.join();
        let events = session_events(root.path(), &session_id(root.path()));
        let original_session = session_id(root.path());
        let facts = events
            .iter()
            .filter(|event| event["event_type"] == "session.rsi_activity_recorded")
            .map(|event| &event["payload"]["fact"])
            .collect::<Vec<_>>();
        assert_eq!(
            facts
                .iter()
                .filter(|fact| fact["kind"] == "started")
                .count(),
            1
        );
        let finish = facts
            .iter()
            .find(|fact| fact["kind"] == "finished")
            .unwrap();
        assert_eq!(finish["disposition"] == "accepted", case == "gain");
        assert_eq!(
            events
                .iter()
                .filter(|event| event["event_type"] == "session.agent_definition_applied")
                .count(),
            usize::from(case == "gain")
        );
        let project = Project::discover(root.path()).unwrap();
        let state = reduced_session(&project, &session_id(root.path()));
        let calls = state
            .history()
            .items()
            .iter()
            .filter_map(|item| match item {
                echidna::domain::model::ContentItem::ToolCall { call } => {
                    Some(call.call_id.as_str())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(calls, vec!["main-failed", "main-reference"]);
        assert_eq!(state.request_audits().len(), count);
        assert_eq!(
            state.execution_receipt_count(),
            if count == 6 { 4 } else { 2 }
        );
        if count == 6 {
            let scoped = facts
                .iter()
                .filter(|fact| fact["kind"] == "sample" && fact["stage"] != "diagnosis")
                .collect::<Vec<_>>();
            assert_eq!(scoped.len(), 2);
            assert_eq!(
                scoped[0]["original_call_ids"],
                json!(["same-provider-call-id"])
            );
            assert_eq!(
                scoped[1]["original_call_ids"],
                json!(["same-provider-call-id"])
            );
            assert_ne!(
                scoped[0]["items"][0]["call"]["call_id"],
                scoped[1]["items"][0]["call"]["call_id"]
            );
            assert_eq!(
                fs::read_to_string(root.path().join("probe.sh")).unwrap(),
                "case \"$1\" in fail) exit 1;; good) printf '42\\n';; *) printf '41\\n';; esac\n"
            );
        }
        if case == "gain" {
            assert_rsi2_strict_activity(&events);
            let published = active(root.path());
            let revision: Value = serde_json::from_slice(
                &fs::read(root.path().join(format!(
                    ".echidna/evolution/technical-writing/revisions/{}.json",
                    published["digest"].as_str().unwrap()
                )))
                .unwrap(),
            )
            .unwrap();
            assert_eq!(
                revision["rsi"]["state"]["experiences"][0]["cue"],
                "CSV aggregation"
            );
            assert_eq!(
                revision["rsi"]["state"]["experiences"][0]["references"]
                    .as_array()
                    .unwrap()
                    .len(),
                7
            );
            for (task, applicable) in [
                ("CSV aggregation on fresh data.", true),
                ("JSONL conversion on fresh data.", false),
            ] {
                let fresh_server = self::server(
                    move |_, request| {
                        assert_eq!(
                            request.to_string().contains("rsi_applicable_experience_v2"),
                            applicable
                        );
                        assert!(
                            request["instructions"]
                                .as_str()
                                .unwrap()
                                .contains("For CSV aggregation, invoke")
                        );
                        text_response("FRESH_DONE", "fresh-done")
                    },
                    1,
                );
                let fresh_config = write_config(root.path(), &fresh_server.base_url);
                let fresh = run_chat(
                    root.path(),
                    &fresh_config,
                    &format!("{task}\n/exit\n"),
                    &["--agent", definition.to_str().unwrap()],
                );
                assert!(
                    fresh.status.success(),
                    "{}",
                    String::from_utf8_lossy(&fresh.stderr)
                );
                assert_eq!(fresh_server.requests.try_iter().count(), 1);
                fresh_server.join();
            }
            // A valid controlled compaction retaining no ordinary History
            // distinguishes raw durable offsets from the normalized runtime.
            let mut compacted = events.clone();
            let compaction = compacted
                .iter_mut()
                .find(|event| event["event_type"] == "session.compaction_created")
                .unwrap();
            compaction["payload"]["retained_from_normalized_history_index"] =
                compaction["payload"]["source_history_item_count"].clone();
            compaction["payload"]["retained_history"] = json!([]);
            let parsed = compacted
                .iter()
                .map(|event| serde_json::from_value::<EventEnvelope<Value>>(event.clone()).unwrap())
                .collect::<Vec<_>>();
            SessionReducer.reduce(parsed).unwrap();
            let log = root
                .path()
                .join(".echidna/sessions")
                .join(&original_session)
                .join("events.jsonl");
            fs::write(
                log,
                compacted
                    .iter()
                    .map(|event| format!("{event}\n"))
                    .collect::<String>(),
            )
            .unwrap();
            let resumed_server = self::server(
                |_, _| text_response("AFTER_COMPACTION_DONE", "after-compaction"),
                1,
            );
            let resumed_config = write_config(root.path(), &resumed_server.base_url);
            let resumed = run_chat(
                root.path(),
                &resumed_config,
                "A new ordinary text task.\n/exit\n",
                &["--resume", &original_session],
            );
            assert!(
                resumed.status.success(),
                "{}",
                String::from_utf8_lossy(&resumed.stderr)
            );
            assert_eq!(resumed_server.requests.try_iter().count(), 1);
            resumed_server.join();
            let resumed_events = session_events(root.path(), &original_session);
            assert_eq!(
                resumed_events
                    .iter()
                    .filter(
                        |event| event["event_type"] == "session.rsi_activity_recorded"
                            && event["payload"]["fact"]["kind"] == "started"
                    )
                    .count(),
                1,
                "old failures must not trigger a new diagnostic after compaction"
            );
        }
    }
}

#[test]
fn rsi2_no_signal_and_insufficient_shared_budget_do_not_add_requests() {
    for case in ["no-signal", "small-budget", "feedback"] {
        let root = TemporaryDirectory::new(&format!("rsi2-{case}"));
        initialize_project(root.path());
        let definition = improvement_definition(root.path());
        improvement_fixture(root.path());
        let count = match case {
            "small-budget" => 3,
            "feedback" => 2,
            _ => 1,
        };
        let server = server(
            move |step, _| {
                match (case,step) {
            ("small-budget",0)=>tool("failed","run_command",json!({"argv":["/bin/sh","probe.sh","fail"]})),
            ("small-budget",1)=>tool("main-reference","run_command",json!({"argv":["/bin/sh","probe.sh","good"],"code_inputs":["probe.sh"]})),
            ("feedback",1)=>text_response(&json!({"decision":"no_change","reason":"No trustworthy local contrast for this preference."}).to_string(),"no-change"),
            _=>text_response("DONE","completed"),
        }
            },
            count,
        );
        let config = write_config(root.path(), &server.base_url);
        if case == "small-budget" {
            let mut content = fs::read_to_string(&config).unwrap();
            content.push_str("\n[runtime]\nmodel_request_limit_per_turn = 5\n");
            fs::write(&config, content).unwrap();
        }
        let input = if case == "feedback" {
            "[rsi-feedback] Persist concise replies.\n/exit\n"
        } else {
            "Complete the ordinary task.\n/exit\n"
        };
        let output = run_chat(
            root.path(),
            &config,
            input,
            &["--agent", definition.to_str().unwrap()],
        );
        assert!(
            output.status.success(),
            "{case}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(server.requests.try_iter().count(), count);
        server.join();
        let events = session_events(root.path(), &session_id(root.path()));
        assert_eq!(
            events
                .iter()
                .filter(|event| event["event_type"] == "session.rsi_activity_recorded")
                .count(),
            if case == "feedback" { 3 } else { 0 }
        );
        if case == "small-budget" {
            assert!(
                String::from_utf8_lossy(&output.stderr).contains("Fewer than 3 shared admissions")
            );
        }
    }
}

fn assert_rsi2_strict_activity(events: &[Value]) {
    let old = events
        .iter()
        .position(|event| {
            event["payload"]["fact"]["kind"] == "sample"
                && event["payload"]["fact"]["stage"] == "old"
        })
        .unwrap();
    let old_result = events
        .iter()
        .position(|event| {
            event["payload"]["fact"]["kind"] == "results"
                && event["payload"]["fact"]["stage"] == "old"
        })
        .unwrap();
    let old_audit = events
        .iter()
        .find(|event| event["event_id"] == events[old]["payload"]["fact"]["audit_event_id"])
        .unwrap();
    for terminal in [old, old_result] {
        assert_rejected_rsi_events(
            repeated_rsi_audit(&events[..=terminal], old_audit, "fixture-reopened-old-arm"),
            "Old sample or results already committed",
        );
    }
    let first_audit = events
        .iter()
        .find(|event| event["event_type"] == "session.request_audit_recorded")
        .unwrap()["event_id"]
        .clone();
    let scoped_call = events[old]["payload"]["fact"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["kind"] == "tool_call")
        .unwrap()
        .clone();
    let scoped_id = scoped_call["call"]["call_id"].clone();
    let mut main_then_scope = events[..=old].to_vec();
    for event in &mut main_then_scope {
        if event["payload"]["item"]["call"]["call_id"] == "main-failed" {
            event["payload"]["item"]["call"]["call_id"] = scoped_id.clone();
        }
        if event["payload"]["item"]["result"]["call_id"] == "main-failed" {
            event["payload"]["item"]["result"]["call_id"] = scoped_id.clone();
        }
        if event["payload"]["receipt"]["execution_id"] == "execution:main-failed" {
            event["payload"]["receipt"]["execution_id"] =
                json!(format!("execution:{}", scoped_id.as_str().unwrap()));
        }
    }
    let parsed = main_then_scope
        .into_iter()
        .map(|event| serde_json::from_value::<EventEnvelope<Value>>(event).unwrap())
        .collect::<Vec<_>>();
    assert!(matches!(
        SessionReducer.reduce(parsed),
        Err(echidna::domain::session::SessionError::InvalidImprovementActivity)
    ));
    let mut scope_then_main = events[..=old_result].to_vec();
    let mut next = scope_then_main.last().unwrap().clone();
    next["event_id"] = json!("fixture-main-scope-collision");
    next["sequence"] = json!(next["sequence"].as_u64().unwrap() + 1);
    next["event_type"] = json!("session.history_item_recorded");
    next["payload"] = json!({"schema_version":1,"item":scoped_call});
    scope_then_main.push(next);
    let parsed = scope_then_main
        .into_iter()
        .map(|event| serde_json::from_value::<EventEnvelope<Value>>(event).unwrap())
        .collect::<Vec<_>>();
    assert!(
        matches!(
            SessionReducer.reduce(parsed),
            Err(echidna::domain::session::SessionError::InvalidImprovementActivity)
        ),
        "future main calls must reject prior scope identities before execution"
    );
    for case in [
        "audit",
        "provider",
        "stage",
        "mapping",
        "receipt",
        "pairing",
        "unknown-field",
    ] {
        let mut changed = events.to_vec();
        match case {
            "audit" => changed[old]["payload"]["fact"]["audit_event_id"] = first_audit.clone(),
            "provider" => {
                changed[old]["payload"]["fact"]["provider_session"]["id"] = json!("primary:wrong")
            }
            "stage" => changed[old]["payload"]["fact"]["stage"] = json!("new"),
            "mapping" => {
                changed[old]["payload"]["fact"]["original_call_ids"] = json!(["wrong-original"])
            }
            "receipt" => changed[old_result]["payload"]["fact"]["receipt_event_ids"] = json!([]),
            "pairing" => {
                changed[old_result]["payload"]["fact"]["items"][0]["result"]["call_id"] =
                    json!("main-reference")
            }
            "unknown-field" => changed[old]["payload"]["fact"]["invented"] = json!(true),
            _ => unreachable!(),
        }
        let envelopes = changed
            .into_iter()
            .map(|event| serde_json::from_value::<EventEnvelope<Value>>(event).unwrap())
            .collect::<Vec<_>>();
        assert!(
            SessionReducer.reduce(envelopes).is_err(),
            "{case} was accepted"
        );
    }
    for kind in ["session.resumed", "session.compaction_created"] {
        let mut stopped = events[..=old].to_vec();
        let mut next = if kind == "session.resumed" {
            stopped.last().unwrap().clone()
        } else {
            events
                .iter()
                .find(|event| event["event_type"] == kind)
                .unwrap()
                .clone()
        };
        next["event_id"] = json!(format!("fixture-rsi-unresolved-{kind}"));
        next["sequence"] = json!(stopped.last().unwrap()["sequence"].as_u64().unwrap() + 1);
        next["event_type"] = json!(kind);
        if kind == "session.resumed" {
            next["payload"] = json!({"schema_version":1,"primary_provider_session":{"id":"primary:fresh-after-rsi","role":"primary","generation":999}});
        }
        stopped.push(next);
        let envelopes = stopped
            .into_iter()
            .map(|event| serde_json::from_value::<EventEnvelope<Value>>(event).unwrap())
            .collect::<Vec<_>>();
        assert!(
            matches!(
                SessionReducer.reduce(envelopes),
                Err(echidna::domain::session::SessionError::InvalidImprovementActivity)
            ),
            "unpaired scoped execution must block {kind} despite paired ordinary History"
        );
    }
}

#[test]
fn rsi2_code_inputs_attests_actual_bytes_and_keeps_explicit_expected_digest_checks() {
    for case in ["automatic", "expected-mismatch", "post-change"] {
        let root = TemporaryDirectory::new(&format!("rsi2-code-{case}"));
        initialize_project(root.path());
        let definition = improvement_definition(root.path());
        fs::write(root.path().join("code.sh"), "printf verified").unwrap();
        let server = server(
            move |step, _| {
                if step == 0 {
                    let mut args = json!({"argv":["/bin/sh","code.sh"],"code_inputs":["code.sh"]});
                    if case == "expected-mismatch" {
                        args["verified_inputs"] =
                            json!([{"path":"code.sh","blake3":"0".repeat(64)}]);
                        args["argv"] = json!(["/bin/sh", "-c", "touch must-not-run"]);
                    }
                    if case == "post-change" {
                        args["argv"] = json!(["/bin/sh", "-c", "printf changed > code.sh"]);
                    }
                    tool("code-attestation", "run_command", args)
                } else {
                    text_response("DONE", "done")
                }
            },
            2,
        );
        let config = write_config(root.path(), &server.base_url);
        let mut content = fs::read_to_string(&config).unwrap();
        content.push_str("\n[runtime]\nmodel_request_limit_per_turn = 2\n");
        fs::write(&config, content).unwrap();
        let output = run_chat(
            root.path(),
            &config,
            "Run the checked code.\n/exit\n",
            &["--agent", definition.to_str().unwrap()],
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        server.join();
        let project = Project::discover(root.path()).unwrap();
        let state = reduced_session(&project, &session_id(root.path()));
        let result = state
            .history()
            .items()
            .iter()
            .find_map(|item| match item {
                echidna::domain::model::ContentItem::ToolResult { result } => Some(result),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            result.status == echidna::domain::tools::ToolResultStatus::Succeeded,
            case == "automatic"
        );
        assert_eq!(
            result.output.get("verified_inputs").is_some(),
            case == "automatic"
        );
        assert!(!root.path().join("must-not-run").exists());
    }
}

#[test]
#[cfg(unix)]
fn rsi2_attestation_rejects_special_and_oversized_inputs_before_command_without_hanging() {
    use std::os::unix::fs::symlink;

    for automatic in [true, false] {
        for case in [
            "fifo",
            "directory",
            "device-link",
            "regular-link",
            "limit-plus-one",
            "sparse",
            "boundary",
            "post-fifo",
        ] {
            let root = TemporaryDirectory::new(&format!("rsi2-bounded-{automatic}-{case}"));
            initialize_project(root.path());
            let definition = improvement_definition(root.path());
            let path = root.path().join("input");
            let initial = if case == "boundary" {
                vec![b'x'; 1024 * 1024]
            } else {
                b"checked input".to_vec()
            };
            let digest = blake3::hash(&initial).to_hex().to_string();
            match case {
                "fifo" => assert!(
                    Command::new("mkfifo")
                        .arg(&path)
                        .status()
                        .unwrap()
                        .success()
                ),
                "directory" => fs::create_dir(&path).unwrap(),
                "device-link" => symlink("/dev/zero", &path).unwrap(),
                "regular-link" => {
                    fs::write(root.path().join("source"), &initial).unwrap();
                    symlink("source", &path).unwrap();
                }
                "limit-plus-one" | "sparse" => fs::File::create(&path)
                    .unwrap()
                    .set_len(if case == "sparse" {
                        64 * 1024 * 1024
                    } else {
                        1024 * 1024 + 1
                    })
                    .unwrap(),
                _ => fs::write(&path, &initial).unwrap(),
            }
            let mut args = json!({"argv":["/bin/sh","-c",if case=="post-fifo" {
                "printf started > command-started; rm input; mkfifo input"
            } else {"printf started > command-started"}]});
            if automatic {
                args["code_inputs"] = json!(["input"]);
            } else {
                args["verified_inputs"] = json!([{"path":"input","blake3":digest}]);
            }
            let server = server(
                move |step, _| {
                    if step == 0 {
                        tool("bounded-attestation", "run_command", args.clone())
                    } else {
                        text_response("DONE", "done")
                    }
                },
                2,
            );
            let config = write_config(root.path(), &server.base_url);
            let mut content = fs::read_to_string(&config).unwrap();
            content.push_str("\n[runtime]\nmodel_request_limit_per_turn = 2\n");
            fs::write(&config, content).unwrap();
            let child = spawn_f2a_chat(
                root.path(),
                &config,
                "Check the bounded input.\n/exit\n",
                &["--agent", definition.to_str().unwrap()],
            );
            let output = wait_child_before_deadline(
                child,
                &format!("bounded RSI attestation {automatic}/{case}"),
                Duration::from_secs(15),
            );
            assert!(
                output.status.success(),
                "{automatic}/{case}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            server.join();
            let project = Project::discover(root.path()).unwrap();
            let state = reduced_session(&project, &session_id(root.path()));
            let result = state
                .history()
                .items()
                .iter()
                .find_map(|item| match item {
                    echidna::domain::model::ContentItem::ToolResult { result } => Some(result),
                    _ => None,
                })
                .unwrap();
            assert_eq!(state.execution_receipt_count(), 1);
            assert_eq!(
                result.status == echidna::domain::tools::ToolResultStatus::Succeeded,
                case == "boundary"
            );
            assert_eq!(
                root.path().join("command-started").exists(),
                matches!(case, "boundary" | "post-fifo")
            );
            assert_eq!(
                result.output.get("verified_inputs").is_some(),
                case == "boundary"
            );
            if case == "post-fifo" {
                assert_eq!(result.output["exit_code"], 0);
            } else if case != "boundary" {
                assert!(result.output["exit_code"].is_null());
            }
            if case != "boundary" {
                let error = result.output["error"].as_str().unwrap();
                assert!(
                    error.contains(if matches!(case, "limit-plus-one" | "sparse") {
                        "1MiB"
                    } else {
                        "regular file"
                    }),
                    "{automatic}/{case}: {error}"
                );
            }
        }
    }
}

#[test]
fn rsi2_probes_preserve_actual_current_turn_deferred_availability_and_do_not_promote_hidden() {
    for exposure in ["deferred", "hidden"] {
        let root = TemporaryDirectory::new(&format!("rsi2-exposure-{exposure}"));
        initialize_project(root.path());
        let definition = improvement_definition(root.path());
        improvement_fixture(root.path());
        let manifest = definition.join("agent.toml");
        let text = fs::read_to_string(&manifest).unwrap().replace(
            "run_command = \"direct\"",
            &format!("run_command = \"{exposure}\""),
        );
        fs::write(manifest, text).unwrap();
        let count = if exposure == "deferred" { 8 } else { 3 };
        let server = server(
            move |step, request| {
                let advertised = request["tools"]
                    .as_array()
                    .is_some_and(|tools| tools.iter().any(|tool| tool["name"] == "run_command"));
                if exposure == "hidden" {
                    assert!(!advertised);
                    return match step {
                        0 => tool(
                            "main-reference",
                            "run_command",
                            json!({"argv":["/bin/sh","probe.sh","good"],"code_inputs":["probe.sh"]}),
                        ),
                        1 => text_response("ORDINARY_COMPLETED", "hidden-completed"),
                        2 => {
                            text_response(&improvement_candidate().to_string(), "hidden-diagnosis")
                        }
                        _ => unreachable!(),
                    };
                }
                match step {
                    0 => {
                        assert!(!advertised);
                        tool(
                            "activate-command",
                            "tool_search",
                            json!({"query":"run_command"}),
                        )
                    }
                    1 => {
                        assert!(advertised);
                        tool(
                            "main-failed",
                            "run_command",
                            json!({"argv":["/bin/sh","probe.sh","fail"]}),
                        )
                    }
                    2 => {
                        assert!(advertised);
                        tool(
                            "main-reference",
                            "run_command",
                            json!({"argv":["/bin/sh","probe.sh","good"],"code_inputs":["probe.sh"]}),
                        )
                    }
                    3 => text_response("ORDINARY_COMPLETED", "deferred-completed"),
                    4 => {
                        assert!(!advertised);
                        text_response(&improvement_candidate().to_string(), "deferred-diagnosis")
                    }
                    5 | 6 => {
                        assert!(advertised);
                        assert_eq!(request["tools"].as_array().unwrap().len(), 1);
                        tool(
                            "same-provider-command",
                            "run_command",
                            json!({"argv":["/bin/sh","probe.sh",if step==5 {"bad"}else {"good"}]}),
                        )
                    }
                    7 => {
                        assert!(
                            !advertised,
                            "RSI scope must not persist activation into the next operator Turn"
                        );
                        text_response("NEXT_DONE", "next-completed")
                    }
                    _ => unreachable!(),
                }
            },
            count,
        );
        let config = write_config(root.path(), &server.base_url);
        let input = if exposure == "deferred" {
            "CSV aggregation: recover and finish.\nA new ordinary text task.\n/exit\n"
        } else {
            "[rsi-feedback] CSV aggregation: retain a useful method.\n/exit\n"
        };
        let output = run_chat(
            root.path(),
            &config,
            input,
            &["--agent", definition.to_str().unwrap()],
        );
        assert!(
            output.status.success(),
            "{exposure}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(server.requests.try_iter().count(), count);
        server.join();
        let events = session_events(root.path(), &session_id(root.path()));
        assert_eq!(
            events
                .iter()
                .filter(|event| event["event_type"] == "session.agent_definition_applied")
                .count(),
            usize::from(exposure == "deferred")
        );
        if exposure == "hidden" {
            assert!(
                !events
                    .iter()
                    .any(|event| event["event_type"] == "tool.execution_receipt_recorded")
            );
            assert!(
                events
                    .iter()
                    .any(|event| event["payload"]["fact"]["kind"] == "finished"
                        && event["payload"]["fact"]["disposition"] == "inconclusive")
            );
        } else {
            for sample in events.iter().filter(|event| {
                event["payload"]["fact"]["kind"] == "sample"
                    && event["payload"]["fact"]["stage"] != "diagnosis"
            }) {
                let fact = &sample["payload"]["fact"];
                let audit = events
                    .iter()
                    .find(|event| event["event_id"] == fact["audit_event_id"])
                    .unwrap();
                let result = events
                    .iter()
                    .find(|event| {
                        event["payload"]["fact"]["kind"] == "results"
                            && event["payload"]["fact"]["stage"] == fact["stage"]
                    })
                    .unwrap();
                let receipt = events
                    .iter()
                    .find(|event| {
                        event["event_id"] == result["payload"]["fact"]["receipt_event_ids"][0]
                    })
                    .unwrap();
                assert_eq!(
                    audit["payload"]["resolved_tool_plan_fingerprint"],
                    receipt["payload"]["receipt"]["resolved_tool_plan_fingerprint"]
                );
            }
        }
    }
}

fn definition(root: &std::path::Path, enabled: bool) -> std::path::PathBuf {
    let path = root.join("writer");
    let output = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .current_dir(root)
        .args(["agent", "init", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());
    fs::write(path.join("persona.md"), "PERSONA_ORIGINAL").unwrap();
    fs::write(path.join("fixed.md"), "FIXED_OPERATOR_RULE").unwrap();
    let manifest = path.join("agent.toml");
    let mut content = fs::read_to_string(&manifest).unwrap();
    content.push_str("\nrun_command = \"direct\"\nstart_job = \"direct\"\n");
    if enabled {
        content
            .push_str("\n[rsi]\npersona = true\nmethods = true\nfixed_rules_file = \"fixed.md\"\n");
    }
    fs::write(manifest, content).unwrap();
    path
}

fn server(
    respond: impl FnMut(usize, &Value) -> String + Send + 'static,
    count: usize,
) -> LoopbackServer {
    server_with_wait(respond, count, Duration::from_secs(25))
}

fn server_with_wait(
    mut respond: impl FnMut(usize, &Value) -> String + Send + 'static,
    count: usize,
    request_wait: Duration,
) -> LoopbackServer {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let (sender, receiver) = mpsc::channel();
    let thread = thread::spawn(move || {
        for index in 0..count {
            let deadline = Instant::now() + request_wait;
            let mut socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Err(error) => panic!("RSI request {index} missing: {error}"),
                }
            };
            socket.set_nonblocking(false).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let request = read_http_request(&mut socket);
            let body = respond(index, &request_body(&request));
            sender.send(request).unwrap();
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
    });
    LoopbackServer {
        base_url: format!("http://{address}/v1"),
        requests: receiver,
        thread,
    }
}

fn identity(request: &Value) -> (Value, String) {
    let instructions = request["instructions"].as_str().unwrap();
    let line = instructions
        .split("Runtime RSI binding: ")
        .nth(1)
        .unwrap()
        .lines()
        .next()
        .unwrap();
    let field = |key: &str| {
        line.split(&format!("{key}="))
            .nth(1)
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_owned()
    };
    (
        json!({"schema_version":1,"agent_id":field("agent_id"),"session_id":field("session_id"),"parent_digest":field("parent_digest"),"reason":"operator corrected persistent preference","basis":["current task correction; fixture does not claim semantic improvement"]}),
        field("pending_path"),
    )
}

fn tool(call: &str, name: &str, arguments: Value) -> String {
    let events = [
        json!({"type":"response.output_item.done","output_index":0,"item":{"type":"function_call","call_id":call,"name":name,"arguments":arguments.to_string()}}),
        json!({"type":"response.completed","response":{"id":format!("response-{call}")}}),
    ];
    events
        .iter()
        .map(|event| format!("data: {event}\n\n"))
        .collect()
}

fn queue(request: &Value, mutate: impl FnOnce(&mut Value)) -> String {
    let (mut candidate, path) = identity(request);
    mutate(&mut candidate);
    let call = format!(
        "stage-rsi-{}",
        &candidate["parent_digest"].as_str().unwrap()[..12]
    );
    tool(
        &call,
        "write_file",
        json!({"path":path,"content":candidate.to_string(),"expected_hash":null}),
    )
}

fn active(root: &std::path::Path) -> Value {
    serde_json::from_slice(
        &fs::read(root.join(".echidna/evolution/technical-writing/active.json")).unwrap(),
    )
    .unwrap()
}

#[cfg(unix)]
fn published_index(request: &Value) -> Value {
    serde_json::from_str(
        request["instructions"]
            .as_str()
            .unwrap()
            .split("Published ordinary script index: ")
            .nth(1)
            .unwrap()
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap()
}

#[cfg(unix)]
#[test]
fn project_layout_chat_resume_preserves_legacy_rsi_artifacts_memory_and_catalog() {
    for (control, git) in [
        (".echidna", false),
        (".cyber-agent", false),
        (".echidna", true),
        (".cyber-agent", true),
    ] {
        let root = TemporaryDirectory::new("project-layout-cli");
        if git {
            fs::write(root.path().join("source.txt"), b"tracked fixture").unwrap();
            for args in [
                vec!["init", "--quiet"],
                vec!["add", "source.txt"],
                vec![
                    "-c",
                    "user.name=Fixture",
                    "-c",
                    "user.email=fixture@example.invalid",
                    "commit",
                    "--quiet",
                    "-m",
                    "fixture",
                ],
            ] {
                assert!(
                    Command::new("git")
                        .current_dir(root.path())
                        .args(args)
                        .output()
                        .unwrap()
                        .status
                        .success()
                );
            }
        }
        initialize_project(root.path());
        if control == ".cyber-agent" {
            // Empty fixture setup, before path-bearing facts; never user migration.
            fs::rename(root.path().join(".echidna"), root.path().join(control)).unwrap();
        }
        let project = Project::discover(root.path()).unwrap();
        let project_id = project.project_id().clone();
        let config_bytes = fs::read(root.path().join(control).join("project.toml")).unwrap();
        let definition = definition(root.path(), true);
        let manifest = definition.join("agent.toml");
        fs::write(
            &manifest,
            fs::read_to_string(&manifest).unwrap().replace(
                "run_command = \"direct\"",
                "run_command = \"direct\"\nread_output = \"direct\"",
            ),
        )
        .unwrap();
        fs::create_dir(root.path().join("tools")).unwrap();
        let script = "printf 42\n";
        fs::write(root.path().join("tools/answer.sh"), script).unwrap();
        let files = json!([{"path":"tools/answer.sh","blake3":blake3::hash(script.as_bytes()).to_hex().to_string()}]);
        let first = server(
            move |step, request| {
                let (_, pending) = identity(request);
                assert!(pending.starts_with(&format!("{control}/evolution/")));
                match step {
                    0 => tool(
                        "layout-original",
                        "run_command",
                        json!({"argv":["/bin/sh","tools/answer.sh"],"verified_inputs":files}),
                    ),
                    1 => queue(request, |candidate| {
                        candidate["persona"] = json!("PERSONA_LAYOUT");
                        candidate["tools"] = json!([{"name":"answer","description":"fixture arithmetic script","argv":["/bin/sh","tools/answer.sh"],"dependencies":["POSIX sh"],"files":files,"test_call_id":"layout-original"}]);
                    }),
                    2 | 5 => text_response("LAYOUT_READY", &format!("layout-{step}")),
                    3 => tool(
                        "layout-memory-write",
                        "memory_write",
                        json!({"path":"core.md","content":"LAYOUT_MEMORY","expected_hash":null,"event_id":"layout-memory-write"}),
                    ),
                    4 => tool("layout-inspect", "inspect_project", json!({})),
                    _ => unreachable!(),
                }
            },
            6,
        );
        let config = write_config(root.path(), &first.base_url);
        let output = run_chat(
            root.path(),
            &config,
            "Publish script.\nStore memory and inspect.\n/agent reload\n/exit\n",
            &["--agent", definition.to_str().unwrap()],
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let requests: Vec<_> = first
            .requests
            .try_iter()
            .map(|request| request_body(&request))
            .collect();
        first.join();
        assert_eq!(requests.len(), 6);
        assert!(
            requests[2]["instructions"]
                .as_str()
                .unwrap()
                .contains("PERSONA_ORIGINAL")
        );
        assert!(
            requests[3]["instructions"]
                .as_str()
                .unwrap()
                .contains("PERSONA_LAYOUT")
        );
        let scripts = published_index(&requests[3]);
        let published_path = scripts[0]["argv"][1].as_str().unwrap();
        assert!(published_path.starts_with(&format!("{control}/evolution/")));
        let script_bytes = fs::read(root.path().join(published_path)).unwrap();
        let sessions = root.path().join(control).join("sessions");
        let mut ids: Vec<_> = fs::read_dir(&sessions)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(ids.len(), 1);
        let session = ids.pop().unwrap();
        let events_path = sessions.join(&session).join("events.jsonl");
        let prefix = fs::read(&events_path).unwrap();
        let state = reduced_session(&project, &session);
        let binding = state.agent_definition_binding().unwrap();
        let revision = root
            .path()
            .join(control)
            .join("evolution/technical-writing/revisions")
            .join(format!("{}.json", binding.definition().digest().as_str()));
        let revision_bytes = fs::read(&revision).unwrap();
        let original = state
            .history()
            .items()
            .iter()
            .find_map(|item| match item {
                echidna::domain::model::ContentItem::ToolResult { result }
                    if result.call_id.as_str() == "layout-original" =>
                {
                    Some(&result.output)
                }
                _ => None,
            })
            .unwrap();
        let blob_id = original["output_captures"][0]["blob_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let blob_path = root
            .path()
            .join(control)
            .join("artifacts/blobs/sha256")
            .join(&blob_id);
        assert_eq!(fs::read(&blob_path).unwrap(), b"42");
        let tools = scripts[0].clone();
        let second = server(
            move |step, request| {
                assert!(
                    request["instructions"]
                        .as_str()
                        .unwrap()
                        .contains("PERSONA_LAYOUT")
                );
                match step {
                    0 => tool(
                        "layout-read-output",
                        "read_output",
                        json!({"blob_id":blob_id}),
                    ),
                    1 => tool(
                        "layout-reuse",
                        "run_command",
                        json!({"argv":tools["argv"],"verified_inputs":tools["files"]}),
                    ),
                    2 => tool(
                        "layout-memory-read",
                        "memory_read",
                        json!({"path":"core.md","event_id":"layout-memory-read"}),
                    ),
                    3 => tool("layout-resume-inspect", "inspect_project", json!({})),
                    4 => text_response("LAYOUT_RESUMED", "layout-resumed"),
                    _ => unreachable!(),
                }
            },
            5,
        );
        let config = write_config(root.path(), &second.base_url);
        let output = run_chat(
            root.path(),
            &config,
            "Read old capture, reuse script, read memory and inspect.\n/exit\n",
            &["--resume", &session],
        );
        second.join();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("LAYOUT_RESUMED"));
        assert!(fs::read(&events_path).unwrap().starts_with(&prefix));
        assert_eq!(fs::read(&revision).unwrap(), revision_bytes);
        assert_eq!(
            fs::read(root.path().join(published_path)).unwrap(),
            script_bytes
        );
        assert_eq!(fs::read(&blob_path).unwrap(), b"42");
        assert_eq!(
            fs::read(root.path().join(control).join("project.toml")).unwrap(),
            config_bytes
        );
        assert_eq!(
            Project::discover(root.path()).unwrap().project_id(),
            &project_id
        );
        assert_eq!(
            fs::read(
                root.path()
                    .join(control)
                    .join("memory/agents/technical-writing/core.md")
            )
            .unwrap(),
            b"LAYOUT_MEMORY"
        );
        let state = reduced_session(&project, &session);
        for audit in state.request_audits() {
            let input = audit
                .reconstruct(|blob_id| {
                    fs::read(
                        root.path()
                            .join(control)
                            .join("artifacts/blobs/sha256")
                            .join(blob_id.as_str()),
                    )
                })
                .unwrap();
            for spec in input.tool_specs {
                if let Some(template) = &spec.resource_key_template
                    && matches!(
                        spec.name.as_str(),
                        "memory_read"
                            | "memory_write"
                            | "inspect_project"
                            | "view_image"
                            | "http_request"
                    )
                {
                    assert!(
                        template.contains(&format!("file:{control}/")),
                        "{}: {template}",
                        spec.name
                    );
                }
            }
        }
        for item in state.history().items() {
            if let echidna::domain::model::ContentItem::ToolResult { result } = item {
                assert_eq!(
                    result.status,
                    echidna::domain::tools::ToolResultStatus::Succeeded,
                    "{}: {:?}",
                    result.call_id,
                    result.output
                );
                if result.call_id.as_str() == "layout-read-output" {
                    assert_eq!(result.output["text"], "42");
                }
                if result.call_id.as_str() == "layout-reuse" {
                    assert_eq!(result.output["stdout"], "42");
                }
                if result.call_id.as_str() == "layout-memory-read" {
                    assert_eq!(result.output["content"], "LAYOUT_MEMORY");
                }
                if matches!(
                    result.call_id.as_str(),
                    "layout-inspect" | "layout-resume-inspect"
                ) {
                    assert_eq!(
                        result.output["snapshot"]["source_fingerprint"]["kind"],
                        if git { "git" } else { "filesystem" }
                    );
                    let entries = result.output["file_manifest"]["entries"]
                        .as_array()
                        .unwrap();
                    assert!(
                        entries
                            .iter()
                            .any(|entry| entry["relative_path"]
                                == format!("{control}/research.jsonl"))
                    );
                    assert!(entries.iter().all(|entry| {
                        let path = entry["relative_path"].as_str().unwrap();
                        !path.starts_with(&format!("{control}/cache/"))
                            && !path.starts_with(&format!("{control}/sessions/"))
                            && !path.starts_with(&format!("{control}/evolution/"))
                    }));
                }
            }
        }
        let other = if control == ".echidna" {
            ".cyber-agent"
        } else {
            ".echidna"
        };
        assert!(!root.path().join(other).exists());
        assert!(
            root.path()
                .join(control)
                .join("cache/context-fragments")
                .is_dir()
        );
        assert!(
            root.path()
                .join(control)
                .join("chat.runtime.lock")
                .is_file()
        );
    }
}

#[cfg(unix)]
#[test]
fn rsi_direct_executable_bundle_preserves_helpers_and_does_not_mutate_old_modes() {
    let root = TemporaryDirectory::new("rsi-direct-executable");
    initialize_project(root.path());
    let definition = definition(root.path(), true);
    fs::create_dir(root.path().join("tools")).unwrap();
    let main = "#!/bin/sh\n\"$(dirname \"$0\")/helper.sh\"\n";
    let helper = "#!/bin/sh\nprintf 42\n";
    for (path, bytes) in [("tools/main.sh", main), ("tools/helper.sh", helper)] {
        fs::write(root.path().join(path), bytes).unwrap();
        fs::set_permissions(root.path().join(path), fs::Permissions::from_mode(0o755)).unwrap();
    }
    let files = json!([
        {"path":"tools/main.sh","blake3":blake3::hash(main.as_bytes()).to_hex().to_string()},
        {"path":"tools/helper.sh","blake3":blake3::hash(helper.as_bytes()).to_hex().to_string()},
    ]);
    let test_root = root.path().to_path_buf();
    let server = server(
        move |step, request| match step {
            0 | 5 => {
                let argv = if step == 0 {
                    json!(["tools/main.sh"])
                } else {
                    fs::set_permissions(
                        test_root.join("tools/main.sh"),
                        fs::Permissions::from_mode(0o600),
                    )
                    .unwrap();
                    json!(["/bin/sh", "tools/main.sh"])
                };
                tool(
                    &format!("original-{step}"),
                    "run_command",
                    json!({"argv":argv,"verified_inputs":files}),
                )
            }
            1 | 6 => queue(request, |candidate| {
                let direct = step == 1;
                candidate["tools"] = json!([{
                    "name":"direct-helper", "description":"direct main invokes its executable helper",
                    "argv":if direct {json!(["tools/main.sh"])} else {json!(["/bin/sh","tools/main.sh"])},
                    "dependencies":["POSIX sh"],"files":files,
                    "test_call_id":if direct {"original-0"} else {"original-5"},
                }]);
            }),
            3 | 8 => {
                let index = published_index(request);
                let record = &index[0];
                tool(
                    &format!("published-{step}"),
                    "run_command",
                    json!({"argv":record["argv"],"verified_inputs":record["files"]}),
                )
            }
            2 | 4 | 7 | 9 => text_response("DONE", &format!("direct-{step}")),
            _ => unreachable!(),
        },
        10,
    );
    let config = write_config(root.path(), &server.base_url);
    let output = run_chat(
        root.path(),
        &config,
        "Publish direct.\nReuse direct.\nPublish interpreter.\nReuse interpreter.\n/exit\n",
        &["--agent", definition.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let requests: Vec<_> = server
        .requests
        .try_iter()
        .map(|request| request_body(&request))
        .collect();
    server.join();
    assert_eq!(requests.len(), 10);
    let first = published_index(&requests[3]);
    let second = published_index(&requests[8]);
    let first_main = first[0]["argv"][0].as_str().unwrap();
    let second_main = second[0]["argv"][1].as_str().unwrap();
    assert_ne!(
        first_main, second_main,
        "execution property is part of bundle identity"
    );
    for (path, mode) in [(first_main, 0o700), (second_main, 0o600)] {
        assert_eq!(
            fs::metadata(root.path().join(path))
                .unwrap()
                .permissions()
                .mode()
                & 0o7777,
            mode
        );
    }
    for index in [&first, &second] {
        let helper = index[0]["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|file| file["path"].as_str().unwrap().ends_with("helper.sh"))
            .unwrap();
        assert_eq!(
            fs::metadata(root.path().join(helper["path"].as_str().unwrap()))
                .unwrap()
                .permissions()
                .mode()
                & 0o7777,
            0o700
        );
    }
    let state = reduced_session(
        &Project::discover(root.path()).unwrap(),
        &session_id(root.path()),
    );
    for id in ["original-0", "published-3", "original-5", "published-8"] {
        let result = state
            .history()
            .items()
            .iter()
            .find_map(|item| match item {
                echidna::domain::model::ContentItem::ToolResult { result }
                    if result.call_id.as_str() == id =>
                {
                    Some(result)
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(
            result.status,
            echidna::domain::tools::ToolResultStatus::Succeeded
        );
        assert_eq!(result.output["stdout"], "42");
    }
}

fn verify_size_rejection_and_fresh_selection(
    root: &std::path::Path,
    definition: &std::path::Path,
    applied: usize,
) {
    let session = session_id(root);
    let events = session_events(root, &session);
    let transitions: Vec<_> = events
        .iter()
        .filter(|event| event["event_type"] == "session.agent_definition_applied")
        .collect();
    assert_eq!(transitions.len(), applied);
    let head = active(root);
    let state = reduced_session(&Project::discover(root).unwrap(), &session);
    assert_eq!(
        state
            .agent_definition_binding()
            .unwrap()
            .definition()
            .digest()
            .as_str(),
        head["digest"].as_str().unwrap()
    );
    let pending = root.join(".echidna/evolution/technical-writing/pending");
    assert_eq!(
        fs::read_dir(&pending).unwrap().count(),
        1,
        "oversized candidate stays pending"
    );
    let revisions = root.join(".echidna/evolution/technical-writing/revisions");
    assert_eq!(fs::read_dir(&revisions).unwrap().count(), applied + 1);
    for path in fs::read_dir(revisions).unwrap() {
        assert!(path.unwrap().metadata().unwrap().len() <= 1024 * 1024);
    }
    let server = server(
        |_, request| {
            assert!(
                request["instructions"]
                    .as_str()
                    .unwrap()
                    .contains("RSI protocol")
            );
            text_response("FRESH_SELECTION_USABLE", "fresh-after-rejection")
        },
        1,
    );
    let config = write_context_window_config(root, &server.base_url, 2_097_152);
    let output = run_chat(
        root,
        &config,
        "Check the published head.\n/exit\n",
        &["--agent", definition.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    server.join();
    assert_eq!(active(root), head);
}

#[cfg(unix)]
#[test]
fn rsi_direct_entrypoint_that_loses_execution_after_its_test_is_rejected() {
    let root = TemporaryDirectory::new("rsi-direct-mode-changed");
    initialize_project(root.path());
    let definition = definition(root.path(), true);
    let script = "#!/bin/sh\nexit 0\n";
    fs::create_dir(root.path().join("tools")).unwrap();
    fs::write(root.path().join("tools/tool.sh"), script).unwrap();
    fs::set_permissions(
        root.path().join("tools/tool.sh"),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let files = json!([{"path":"tools/tool.sh","blake3":blake3::hash(script.as_bytes()).to_hex().to_string()}]);
    let test_root = root.path().to_path_buf();
    let server = server(
        move |step, request| match step {
            0 => tool(
                "direct-test",
                "run_command",
                json!({"argv":["tools/tool.sh"],"verified_inputs":files}),
            ),
            1 => {
                fs::set_permissions(
                    test_root.join("tools/tool.sh"),
                    fs::Permissions::from_mode(0o600),
                )
                .unwrap();
                queue(request, |candidate| {
                    candidate["tools"] = json!([{"name":"changed-mode","description":"direct mode changed after real test", "argv":["tools/tool.sh"],"dependencies":["POSIX sh"],"files":files,"test_call_id":"direct-test"}]);
                })
            }
            2 => text_response("DONE", "changed-mode-done"),
            _ => unreachable!(),
        },
        3,
    );
    let config = write_config(root.path(), &server.base_url);
    let output = run_chat(
        root.path(),
        &config,
        "Reject a changed executable.\n/exit\n",
        &["--agent", definition.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    server.join();
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("direct script entrypoint is no longer executable")
    );
    let head = active(root.path());
    assert_eq!(head["digest"], head["base_source_digest"]);
}

#[cfg(unix)]
#[test]
fn rsi_encoded_revision_limit_rejects_cumulative_script_growth_before_publication() {
    let root = TemporaryDirectory::new("rsi-cumulative-revision-limit");
    initialize_project(root.path());
    let definition = definition(root.path(), true);
    fs::create_dir(root.path().join("tools")).unwrap();
    let script = "exit 0\n";
    fs::write(root.path().join("tools/noop.sh"), script).unwrap();
    let files = json!([{"path":"tools/noop.sh","blake3":blake3::hash(script.as_bytes()).to_hex().to_string()}]);
    let mut argv = vec!["/bin/sh".to_owned(), "tools/noop.sh".to_owned()];
    argv.extend((0..30).map(|_| "x".repeat(4096)));
    let server = server_with_wait(
        move |step, request| {
            if step == 0 {
                return tool(
                    "large-argv-test",
                    "run_command",
                    json!({"argv":argv,"verified_inputs":files}),
                );
            }
            if matches!(step, 1 | 3 | 5 | 7 | 9) {
                let batch = (step - 1) / 2;
                return queue(request, |candidate| {
                    candidate["tools"] = Value::Array((0..2).map(|offset| json!({
                    "name":format!("large-{}", batch * 2 + offset),"description":"bounded argv cumulative growth fixture",
                    "argv":argv,"dependencies":["POSIX sh"],"files":files,"test_call_id":"large-argv-test",
                })).collect());
                    assert!(candidate.to_string().len() < 320 * 1024);
                });
            }
            text_response("DONE", &format!("cumulative-{step}"))
        },
        12,
        Duration::from_secs(90),
    );
    let config = write_context_window_config(root.path(), &server.base_url, 2_097_152);
    let output = run_chat(
        root.path(),
        &config,
        "Publish zero.\nPublish one.\nPublish two.\nPublish three.\nReject overflow.\nCheck unchanged head.\n/exit\n",
        &["--agent", definition.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("[RSI rejected] encoded RSI revision")
    );
    let requests: Vec<_> = server
        .requests
        .try_iter()
        .map(|request| request_body(&request))
        .collect();
    server.join();
    assert_eq!(requests.len(), 12);
    assert_eq!(published_index(&requests[11]).as_array().unwrap().len(), 8);
    verify_size_rejection_and_fresh_selection(root.path(), &definition, 4);
}

#[test]
fn rsi_encoded_revision_limit_counts_json_escaping_and_retains_the_previous_binding() {
    let root = TemporaryDirectory::new("rsi-escaped-revision-limit");
    initialize_project(root.path());
    let definition = definition(root.path(), true);
    fs::write(
        definition.join("work.md"),
        format!("WORK{}", "\0".repeat(60000)),
    )
    .unwrap();
    fs::write(
        definition.join("fixed.md"),
        format!("FIXED{}", "\0".repeat(60000)),
    )
    .unwrap();
    let server = server(
        |step, request| match step {
            0 => queue(request, |candidate| {
                candidate["persona"] = json!(format!("ESCAPED_PERSONA{}", "\0".repeat(50000)));
                assert!(candidate.to_string().len() < 320 * 1024);
            }),
            2 => queue(request, |candidate| {
                candidate["methods"] =
                    json!([format!("REJECTED_ESCAPED_METHOD{}", "\0".repeat(20000))]);
                assert!(candidate.to_string().len() < 320 * 1024);
            }),
            4 => {
                let prompt = request["instructions"].as_str().unwrap();
                assert!(prompt.contains("ESCAPED_PERSONA"));
                assert!(!prompt.contains("REJECTED_ESCAPED_METHOD"));
                text_response("OLD_HEAD_USABLE", "escaped-probe")
            }
            1 | 3 => text_response("DONE", &format!("escaping-{step}")),
            _ => unreachable!(),
        },
        5,
    );
    let config = write_context_window_config(root.path(), &server.base_url, 2_097_152);
    let output = run_chat(
        root.path(),
        &config,
        "Publish persona.\nReject escaped overflow.\nCheck unchanged head.\n/exit\n",
        &["--agent", definition.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("[RSI rejected] encoded RSI revision")
    );
    server.join();
    verify_size_rejection_and_fresh_selection(root.path(), &definition, 1);
}

#[test]
fn rsi_pins_the_current_turn_and_applies_persona_and_method_replacement_next_turn() {
    let root = TemporaryDirectory::new("rsi-pinned-turn");
    initialize_project(root.path());
    let definition = definition(root.path(), true);
    let server = server(
        |step, request| match step {
            0 => {
                assert!(
                    request["instructions"]
                        .as_str()
                        .unwrap()
                        .contains("FIXED_OPERATOR_RULE")
                );
                queue(request, |candidate| {
                    candidate["persona"] = json!("PERSONA_EVOLVED");
                    candidate["methods"] = json!([]);
                })
            }
            1 => {
                assert!(
                    request["instructions"]
                        .as_str()
                        .unwrap()
                        .contains("PERSONA_ORIGINAL")
                );
                text_response("FIRST_DONE", "first-rsi")
            }
            2 => {
                let text = request["instructions"].as_str().unwrap();
                assert!(text.contains("PERSONA_EVOLVED"));
                assert!(text.contains("FIXED_OPERATOR_RULE"));
                assert!(!text.contains("Clarity method"));
                assert!(request.get("previous_response_id").is_none());
                text_response("SECOND_DONE", "second-rsi")
            }
            _ => unreachable!(),
        },
        3,
    );
    let config = write_config(root.path(), &server.base_url);
    let output = run_chat(
        root.path(),
        &config,
        "Change lasting preference.\nUse the new preference.\n/exit\n",
        &["--agent", definition.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("[RSI applied]"));
    let requests: Vec<_> = server.requests.try_iter().collect();
    assert_eq!(requests.len(), 3);
    server.join();
    let events = session_events(root.path(), &session_id(root.path()));
    let transitions: Vec<_> = events
        .iter()
        .filter(|event| event["event_type"] == "session.agent_definition_applied")
        .collect();
    assert_eq!(transitions.len(), 1);
    let published = active(root.path());
    let revision: Value = serde_json::from_slice(
        &fs::read(root.path().join(format!(
            ".echidna/evolution/technical-writing/revisions/{}.json",
            published["digest"].as_str().unwrap()
        )))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(revision["instructions"]["methods"], json!([]));
    assert_eq!(
        revision["instructions"]["persona"]["source_locator"]["kind"],
        "evolution"
    );
    assert_eq!(
        fs::read_to_string(definition.join("persona.md")).unwrap(),
        "PERSONA_ORIGINAL"
    );
}

#[test]
fn rsi_disabled_keeps_legacy_instructions_and_does_not_initialize_a_queue() {
    let root = TemporaryDirectory::new("rsi-disabled");
    initialize_project(root.path());
    let definition = definition(root.path(), false);
    let server = server(
        |_, request| {
            let text = request["instructions"].as_str().unwrap();
            assert!(!text.contains("RSI protocol"));
            assert!(text.contains("PERSONA_ORIGINAL"));
            text_response("OFF", "off-rsi")
        },
        1,
    );
    let config = write_config(root.path(), &server.base_url);
    let output = run_chat(
        root.path(),
        &config,
        "One task.\n/exit\n",
        &["--agent", definition.to_str().unwrap()],
    );
    assert!(output.status.success());
    server.join();
    assert!(!root.path().join(".echidna/evolution").exists());
    let events = session_events(root.path(), &session_id(root.path()));
    let started = events
        .iter()
        .find(|event| event["event_type"] == "session.started_with_agent")
        .unwrap();
    assert!(
        started["payload"]["binding"]["definition"]
            .get("rsi")
            .is_none()
    );
}

#[test]
fn rsi_rejects_foreign_stale_unknown_and_oversized_candidates_without_consumption() {
    for case in ["foreign", "stale", "immutable", "oversized", "budget"] {
        let root = TemporaryDirectory::new(&format!("rsi-reject-{case}"));
        initialize_project(root.path());
        let definition = definition(root.path(), true);
        let test_root = root.path().to_path_buf();
        let server = server(
            move |_, request| {
                let (mut candidate, path) = identity(request);
                candidate["persona"] = json!("PERSONA_EVOLVED");
                match case {
                    "foreign" => candidate["agent_id"] = json!("another-agent"),
                    "stale" => candidate["parent_digest"] = json!("0".repeat(64)),
                    "immutable" => candidate["work"] = json!("replace fixed work"),
                    "oversized" => candidate["persona"] = json!("x".repeat(70 * 1024)),
                    "budget" => candidate["methods"] = json!(vec!["x".repeat(64000); 4]),
                    _ => unreachable!(),
                }
                fs::write(test_root.join(path), candidate.to_string()).unwrap();
                text_response("DONE", "reject-rsi")
            },
            1,
        );
        let config = write_config(root.path(), &server.base_url);
        let output = run_chat(
            root.path(),
            &config,
            "Try candidate.\n/exit\n",
            &["--agent", definition.to_str().unwrap()],
        );
        assert!(
            output.status.success(),
            "{case}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        server.join();
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("[RSI rejected]"),
            "{case}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let published = active(root.path());
        assert_eq!(published["digest"], published["base_source_digest"]);
        assert_eq!(
            fs::read_dir(
                root.path()
                    .join(".echidna/evolution/technical-writing/pending")
            )
            .unwrap()
            .count(),
            1
        );
    }
}

#[test]
fn rsi_stopped_turn_keeps_candidate_until_an_explicit_resume_and_normal_completion() {
    let root = TemporaryDirectory::new("rsi-stopped");
    initialize_project(root.path());
    let definition = definition(root.path(), true);
    let first = server(
        |step, request| {
            if step == 0 {
                queue(request, |candidate| {
                    candidate["persona"] = json!("PERSONA_EVOLVED")
                })
            } else {
                "data: {\"type\":\"response.failed\",\"response\":{\"error\":{\"message\":\"fixture stop\"}}}\n\n".to_owned()
            }
        },
        2,
    );
    let config = write_config(root.path(), &first.base_url);
    let output = run_chat(
        root.path(),
        &config,
        "Stage then stop.\n/exit\n",
        &["--agent", definition.to_str().unwrap()],
    );
    assert!(!output.status.success());
    first.join();
    let seed = active(root.path());
    assert_eq!(seed["digest"], seed["base_source_digest"]);
    let session = session_id(root.path());
    let resumed = server(
        |_, request| {
            assert!(
                request["instructions"]
                    .as_str()
                    .unwrap()
                    .contains("PERSONA_ORIGINAL")
            );
            text_response("RECOVERED", "rsi-recovered")
        },
        1,
    );
    let config = write_config(root.path(), &resumed.base_url);
    let output = run_chat(
        root.path(),
        &config,
        "Finish a new task.\n/exit\n",
        &["--resume", &session],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    resumed.join();
    assert_ne!(active(root.path())["digest"], seed["digest"]);
}

#[test]
fn rsi_running_job_defers_a_pending_candidate() {
    let root = TemporaryDirectory::new("rsi-active-job");
    initialize_project(root.path());
    let definition = definition(root.path(), true);
    let server = server(
        |step, request| match step {
            0 => queue(request, |candidate| {
                candidate["persona"] = json!("PERSONA_EVOLVED")
            }),
            1 => tool(
                "rsi-live-job",
                "start_job",
                json!({"argv":["/bin/sleep","1"]}),
            ),
            2 => text_response("BACKGROUND_STARTED", "rsi-background"),
            _ => unreachable!(),
        },
        3,
    );
    let config = write_config(root.path(), &server.base_url);
    let output = run_chat(
        root.path(),
        &config,
        "Start bounded job.\n/exit\n",
        &["--agent", definition.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    server.join();
    assert!(String::from_utf8_lossy(&output.stderr).contains("[RSI deferred]"));
    let published = active(root.path());
    assert_eq!(published["digest"], published["base_source_digest"]);
}

#[test]
fn rsi_rollback_is_a_new_revision_and_preserves_original_snapshots() {
    let root = TemporaryDirectory::new("rsi-rollback");
    initialize_project(root.path());
    let definition = definition(root.path(), true);
    let mut seed = String::new();
    let server = server(
        move |step, request| match step {
            0 => {
                seed = identity(request).0["parent_digest"]
                    .as_str()
                    .unwrap()
                    .to_owned();
                queue(request, |candidate| {
                    candidate["persona"] = json!("PERSONA_EVOLVED")
                })
            }
            1 | 3 => text_response("DONE", &format!("rollback-{step}")),
            2 => queue(request, |candidate| candidate["rollback_to"] = json!(seed)),
            4 => {
                assert!(
                    request["instructions"]
                        .as_str()
                        .unwrap()
                        .contains("PERSONA_ORIGINAL")
                );
                text_response("ROLLED_BACK", "rollback-final")
            }
            _ => unreachable!(),
        },
        5,
    );
    let config = write_config(root.path(), &server.base_url);
    let output = run_chat(
        root.path(),
        &config,
        "Evolve.\nRoll back.\nCheck.\n/exit\n",
        &["--agent", definition.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    server.join();
    let published = active(root.path());
    assert_ne!(published["digest"], published["base_source_digest"]);
    assert_eq!(
        fs::read_dir(
            root.path()
                .join(".echidna/evolution/technical-writing/revisions")
        )
        .unwrap()
        .count(),
        3
    );
    let events = session_events(root.path(), &session_id(root.path()));
    assert_eq!(
        events
            .iter()
            .filter(|event| event["event_type"] == "session.agent_definition_applied")
            .count(),
        2
    );
}

#[test]
fn rsi_new_sessions_bind_current_personal_project_snapshots_and_relocated_source() {
    let root = TemporaryDirectory::new("rsi-personal-binding");
    initialize_project(root.path());
    let definition = definition(root.path(), true);
    fs::write(root.path().join("personal.md"), "PERSONAL_OLD").unwrap();
    let first = server(
        |_, request| {
            assert!(
                request["instructions"]
                    .as_str()
                    .unwrap()
                    .contains("PERSONAL_OLD")
            );
            text_response("OLD", "personal-old")
        },
        1,
    );
    let config = write_config(root.path(), &first.base_url);
    fs::write(
        &config,
        format!(
            "{}\n[prompts]\npersonal_file = \"personal.md\"\n",
            fs::read_to_string(&config).unwrap()
        ),
    )
    .unwrap();
    let output = run_chat(
        root.path(),
        &config,
        "Original task.\n/exit\n",
        &["--agent", definition.to_str().unwrap()],
    );
    assert!(output.status.success());
    first.join();
    let old_session = session_id(root.path());
    let moved = root.path().join("writer-moved");
    fs::rename(&definition, &moved).unwrap();
    fs::write(root.path().join("personal.md"), "PERSONAL_NEW").unwrap();
    let second = server(
        |_, request| {
            assert!(
                request["instructions"]
                    .as_str()
                    .unwrap()
                    .contains("PERSONAL_NEW")
            );
            text_response("NEW", "personal-new")
        },
        1,
    );
    let config = write_config(root.path(), &second.base_url);
    fs::write(
        &config,
        format!(
            "{}\n[prompts]\npersonal_file = \"personal.md\"\n",
            fs::read_to_string(&config).unwrap()
        ),
    )
    .unwrap();
    let output = run_chat(
        root.path(),
        &config,
        "/agent reload\nNew task.\n/exit\n",
        &["--agent", moved.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    second.join();
    assert!(!String::from_utf8_lossy(&output.stdout).contains("reload rejected"));
    let resumed = server(
        |_, request| {
            let text = request["instructions"].as_str().unwrap();
            assert!(text.contains("PERSONAL_OLD"));
            assert!(!text.contains("PERSONAL_NEW"));
            text_response("PINNED", "personal-resume")
        },
        1,
    );
    let config = write_config(root.path(), &resumed.base_url);
    let output = run_chat(
        root.path(),
        &config,
        "Saved task.\n/exit\n",
        &["--resume", &old_session],
    );
    assert!(output.status.success());
    resumed.join();
}

#[test]
fn rsi_script_publication_requires_tested_bytes_and_reuses_a_complete_helper_bundle() {
    for case in ["valid", "unattested", "changed", "cwd"] {
        let root = TemporaryDirectory::new(&format!("rsi-script-{case}"));
        initialize_project(root.path());
        let definition = definition(root.path(), true);
        fs::create_dir_all(root.path().join("tools")).unwrap();
        let main = ". \"$(dirname \"$0\")/helper.sh\"; printf '%s' \"$VALUE\"";
        let helper = "VALUE=42";
        fs::write(root.path().join("tools/main.sh"), main).unwrap();
        fs::write(root.path().join("tools/helper.sh"), helper).unwrap();
        let files = json!([{"path":"tools/main.sh","blake3":blake3::hash(main.as_bytes()).to_hex().to_string()},{"path":"tools/helper.sh","blake3":blake3::hash(helper.as_bytes()).to_hex().to_string()}]);
        let test_root = root.path().to_path_buf();
        let server = server(
            move |step, request| match step {
                0 => {
                    let mut arguments = json!({"argv":["/bin/sh","tools/main.sh"]});
                    if case != "unattested" {
                        arguments["verified_inputs"] = files.clone();
                    }
                    if case == "cwd" {
                        arguments["cwd"] = json!(test_root.to_str().unwrap());
                    }
                    tool("script-test", "run_command", arguments)
                }
                1 => {
                    if case == "changed" {
                        fs::write(test_root.join("tools/helper.sh"), "VALUE=wrong").unwrap();
                    }
                    queue(request, |candidate| {
                        candidate["tools"] = json!([{"name":"fixture-helper","description":"prints the verified fixture value","argv":["/bin/sh","tools/main.sh"],"dependencies":["POSIX sh"],"files":files,"test_call_id":"script-test"}]);
                    })
                }
                2 | 4 => text_response("SCRIPT_DONE", &format!("script-{step}")),
                3 => {
                    let instructions = request["instructions"].as_str().unwrap();
                    let index: Value = serde_json::from_str(
                        instructions
                            .split("Published ordinary script index: ")
                            .nth(1)
                            .unwrap()
                            .lines()
                            .next()
                            .unwrap(),
                    )
                    .unwrap();
                    let script = &index[0];
                    assert_eq!(script["name"], "fixture-helper");
                    tool(
                        "script-reuse",
                        "run_command",
                        json!({"argv":script["argv"],"verified_inputs":script["files"]}),
                    )
                }
                _ => unreachable!(),
            },
            if case == "valid" { 5 } else { 3 },
        );
        let config = write_config(root.path(), &server.base_url);
        let output = run_chat(
            root.path(),
            &config,
            if case == "valid" {
                "Publish script.\nReuse script.\n/exit\n"
            } else {
                "Reject script.\n/exit\n"
            },
            &["--agent", definition.to_str().unwrap()],
        );
        assert!(
            output.status.success(),
            "{case}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        server.join();
        let published = active(root.path());
        assert_eq!(
            published["digest"] != published["base_source_digest"],
            case == "valid",
            "{case}"
        );
        if case == "valid" {
            let project = Project::discover(root.path()).unwrap();
            let state = reduced_session(&project, &session_id(root.path()));
            let result = state
                .history()
                .items()
                .iter()
                .find_map(|item| match item {
                    echidna::domain::model::ContentItem::ToolResult { result }
                        if result.call_id.as_str() == "script-reuse" =>
                    {
                        Some(result)
                    }
                    _ => None,
                })
                .unwrap();
            assert_eq!(result.output["stdout"], "42");
            assert_eq!(
                result.output["verified_inputs"].as_array().unwrap().len(),
                2
            );
        } else {
            assert!(String::from_utf8_lossy(&output.stderr).contains("[RSI rejected]"));
        }
    }
}

#[test]
fn rsi_source_changes_require_explicit_new_lineage_and_retain_revisions() {
    let root = TemporaryDirectory::new("rsi-source-lineage");
    initialize_project(root.path());
    let definition = definition(root.path(), true);
    let config = write_config(root.path(), "http://127.0.0.1:1/v1");
    assert!(
        run_chat(
            root.path(),
            &config,
            "/exit\n",
            &["--agent", definition.to_str().unwrap()]
        )
        .status
        .success()
    );
    let old = active(root.path());
    fs::write(definition.join("persona.md"), "OPERATOR_NEW_SOURCE").unwrap();
    let conflict = run_chat(
        root.path(),
        &config,
        "/exit\n",
        &["--agent", definition.to_str().unwrap()],
    );
    assert!(!conflict.status.success());
    assert!(String::from_utf8_lossy(&conflict.stderr).contains("source definition changed"));
    assert_eq!(active(root.path()), old);
    let selected = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .current_dir(root.path())
        .args([
            "--config",
            config.to_str().unwrap(),
            "agent",
            "lineage",
            definition.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        selected.status.success(),
        "{}",
        String::from_utf8_lossy(&selected.stderr)
    );
    assert_ne!(active(root.path())["digest"], old["digest"]);
    assert_eq!(
        fs::read_dir(
            root.path()
                .join(".echidna/evolution/technical-writing/revisions")
        )
        .unwrap()
        .count(),
        2
    );
    assert!(
        run_chat(
            root.path(),
            &config,
            "/exit\n",
            &["--agent", definition.to_str().unwrap()]
        )
        .status
        .success()
    );
}

#[test]
fn rsi_explicit_reconciliation_repairs_a_pointer_behind_a_durable_binding_without_sampling() {
    let root = TemporaryDirectory::new("rsi-reconcile");
    initialize_project(root.path());
    let definition = definition(root.path(), true);
    let server = server(
        |step, request| {
            if step == 0 {
                queue(request, |candidate| {
                    candidate["persona"] = json!("PERSONA_EVOLVED")
                })
            } else {
                text_response("DONE", "reconcile-done")
            }
        },
        2,
    );
    let config = write_config(root.path(), &server.base_url);
    let output = run_chat(
        root.path(),
        &config,
        "Publish.\n/exit\n",
        &["--agent", definition.to_str().unwrap()],
    );
    assert!(output.status.success());
    server.join();
    let published = active(root.path());
    let session = session_id(root.path());
    // Materialize the documented crash-window state: Session transition and
    // revision are durable, but the publication pointer still names its parent.
    let mut behind = published.clone();
    behind["digest"] = published["base_source_digest"].clone();
    fs::write(
        root.path()
            .join(".echidna/evolution/technical-writing/active.json"),
        behind.to_string(),
    )
    .unwrap();
    let inert = write_config(root.path(), "http://127.0.0.1:1/v1");
    let output = run_chat(
        root.path(),
        &inert,
        "/rsi reconcile\n/exit\n",
        &["--resume", &session],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("no request, execution or Provider reset occurred")
    );
    assert_eq!(active(root.path()), published);
    let project = Project::discover(root.path()).unwrap();
    let state = reduced_session(&project, &session);
    assert_eq!(state.request_audits().len(), 2);
    assert_eq!(state.execution_receipt_count(), 1);
}

#[test]
fn rsi_verified_command_inputs_bind_pre_post_bytes_and_leave_default_results_unchanged() {
    for case in ["verified", "default", "before-mismatch", "changed-after"] {
        let root = TemporaryDirectory::new(&format!("rsi-verified-{case}"));
        initialize_project(root.path());
        let definition = definition(root.path(), true);
        fs::write(root.path().join("tool.sh"), "printf verified").unwrap();
        let digest = blake3::hash(b"printf verified").to_hex().to_string();
        let server = server(
            move |step, _| {
                if step == 0 {
                    let mut args = json!({"argv":["/bin/sh","tool.sh"]});
                    if case != "default" {
                        args["verified_inputs"] = json!([{"path":"tool.sh","blake3":if case == "before-mismatch" { "0".repeat(64) } else { digest.clone() }}]);
                    }
                    if case == "before-mismatch" {
                        args["argv"] = json!(["/bin/sh", "-c", "touch must-not-run"]);
                    }
                    if case == "changed-after" {
                        args["argv"] = json!(["/bin/sh", "-c", "printf changed > tool.sh"]);
                    }
                    tool("verification-test", "run_command", args)
                } else {
                    text_response("VERIFICATION_DONE", "verification-done")
                }
            },
            2,
        );
        let config = write_config(root.path(), &server.base_url);
        let output = run_chat(
            root.path(),
            &config,
            "Verify command inputs.\n/exit\n",
            &["--agent", definition.to_str().unwrap()],
        );
        assert!(
            output.status.success(),
            "{case}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        server.join();
        let project = Project::discover(root.path()).unwrap();
        let state = reduced_session(&project, &session_id(root.path()));
        let result = state
            .history()
            .items()
            .iter()
            .find_map(|item| match item {
                echidna::domain::model::ContentItem::ToolResult { result } => Some(result),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            result.status == echidna::domain::tools::ToolResultStatus::Succeeded,
            matches!(case, "verified" | "default")
        );
        assert_eq!(
            result.output.get("verified_inputs").is_some(),
            case == "verified"
        );
        assert_eq!(state.execution_receipt_count(), 1);
        assert!(!root.path().join("must-not-run").exists());
        if case == "changed-after" {
            assert_eq!(result.output["exit_code"], 0);
            assert_eq!(
                fs::read_to_string(root.path().join("tool.sh")).unwrap(),
                "changed"
            );
        }
    }
}

#[test]
fn rsi_verified_inputs_match_batch_schema_and_off_bindings_reject_the_unadvertised_field() {
    for enabled in [false, true] {
        let root = TemporaryDirectory::new("rsi-verified-batch");
        initialize_project(root.path());
        let definition = definition(root.path(), enabled);
        let manifest = definition.join("agent.toml");
        fs::write(
            &manifest,
            fs::read_to_string(&manifest).unwrap().replace(
                "run_command = \"direct\"",
                "run_command = \"direct\"\nrun_commands = \"direct\"",
            ),
        )
        .unwrap();
        fs::write(root.path().join("tool.sh"), "printf batch").unwrap();
        let digest = blake3::hash(b"printf batch").to_hex().to_string();
        let server = server(
            move |step, request| {
                if step == 0 {
                    let spec = request["tools"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|tool| tool["name"] == "run_commands")
                        .unwrap();
                    assert_eq!(
                        spec["parameters"]["properties"]["commands"]["items"]["properties"]
                            .get("verified_inputs")
                            .is_some(),
                        enabled
                    );
                    tool(
                        "batch-verification",
                        "run_commands",
                        json!({"mode":"serial","commands":[{"argv":["/bin/sh","tool.sh"],"verified_inputs":[{"path":"tool.sh","blake3":digest}]}]}),
                    )
                } else {
                    text_response("BATCH_DONE", "batch-done")
                }
            },
            2,
        );
        let config = write_config(root.path(), &server.base_url);
        let output = run_chat(
            root.path(),
            &config,
            "Verify batch inputs.\n/exit\n",
            &["--agent", definition.to_str().unwrap()],
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        server.join();
        let project = Project::discover(root.path()).unwrap();
        let state = reduced_session(&project, &session_id(root.path()));
        assert_eq!(state.execution_receipt_count(), u64::from(enabled));
        let result = state
            .history()
            .items()
            .iter()
            .find_map(|item| match item {
                echidna::domain::model::ContentItem::ToolResult { result } => Some(result),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            result.status == echidna::domain::tools::ToolResultStatus::Succeeded,
            enabled
        );
        if enabled {
            assert_eq!(
                result.output["commands"][0]["result"]["verified_inputs"][0]["path"],
                "tool.sh"
            );
        }
    }
}

#[test]
fn rsi_reload_cannot_switch_the_immutable_catalog_capability_in_an_existing_session() {
    for enabled in [false, true] {
        let root = TemporaryDirectory::new("rsi-reload-capability");
        initialize_project(root.path());
        let definition = definition(root.path(), enabled);
        let manifest = definition.join("agent.toml");
        let source = fs::read_to_string(&manifest).unwrap();
        let server = server(
            move |_, _| {
                let changed = if enabled {
                    source.split("[rsi]").next().unwrap().to_owned()
                } else {
                    format!("{source}\n[rsi]\npersona=true\nmethods=true\n")
                };
                fs::write(&manifest, changed).unwrap();
                text_response("DONE", "capability-done")
            },
            1,
        );
        let config = write_config(root.path(), &server.base_url);
        let output = run_chat(
            root.path(),
            &config,
            "Finish task.\n/agent reload\n/exit\n",
            &["--agent", definition.to_str().unwrap()],
        );
        assert!(output.status.success());
        server.join();
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("reload cannot switch RSI capability")
        );
        assert!(
            session_events(root.path(), &session_id(root.path()))
                .iter()
                .all(|event| event["event_type"] != "session.agent_definition_applied")
        );
    }
}
