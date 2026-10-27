//! Neutral same-Turn checkpoint application, using the real CLI/Session path.
use super::*;

#[test]
fn long_tool_turn_applies_verified_persona_or_methods_before_next_primary() {
    for case in ["persona", "methods", "bad-handoff"] {
        let root = TemporaryDirectory::new(&format!("rsi-long-{case}"));
        initialize_project(root.path());
        let definition = improvement_definition(root.path());
        improvement_fixture(root.path());
        fs::write(
            root.path().join("checkpoint-input.txt"),
            "neutral checkpoint input\n",
        )
        .unwrap();
        let old_persona = fs::read_to_string(definition.join("persona.md")).unwrap();
        let old_persona_for_request = old_persona.clone();
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
                2..=15 => tool(
                    &format!("neutral-read-{step}"),
                    "read_file",
                    json!({"path":"checkpoint-input.txt"}),
                ),
                16 => {
                    assert!(request.get("tools").is_none());
                    assert!(
                        request
                            .to_string()
                            .contains("operator task is still active")
                    );
                    let mut candidate = improvement_candidate();
                    if case != "methods" {
                        candidate["methods"] = Value::Null;
                        candidate["persona"] = json!(
                            "CHECKPOINT_PERSONA: preserve the task and use the verified supported command mode."
                        );
                    } else {
                        candidate["methods"] = json!([
                            "CHECKPOINT_METHOD: use the verified supported command mode and preserve input bytes."
                        ]);
                    }
                    text_response(&candidate.to_string(), "checkpoint-diagnosis")
                }
                17 | 18 => {
                    assert!(request.to_string().contains("RSI2 reference_match probe"));
                    tool(
                        "same-scoped-id",
                        "run_command",
                        json!({"argv":["/bin/sh","probe.sh",if step==17 {"bad"} else {"good"}]}),
                    )
                }
                19 => {
                    assert!(request.get("tools").is_none());
                    if case == "bad-handoff" {
                        text_response("invalid handoff", "bad-checkpoint-handoff")
                    } else {
                        compaction_summary_response(
                            "The retained failure was addressed; main-reference produced the sourced neutral result. Continue the original task after the verified instruction trial.",
                            "checkpoint-handoff",
                        )
                    }
                }
                20 => {
                    let instructions = request["instructions"].as_str().unwrap();
                    if case == "bad-handoff" {
                        assert!(instructions.contains(&old_persona_for_request));
                        assert!(!instructions.contains("CHECKPOINT_PERSONA"));
                    } else {
                        assert!(instructions.contains(if case == "persona" {
                            "CHECKPOINT_PERSONA"
                        } else {
                            "CHECKPOINT_METHOD"
                        }));
                    }
                    text_response("ORIGINAL_TASK_COMPLETED", "checkpoint-ordinary-final")
                }
                _ => panic!("unexpected extra checkpoint request {step}"),
            },
            21,
        );
        let config = write_config(root.path(), &server.base_url);
        fs::write(
            &config,
            format!(
                "{}\n[runtime]\nmodel_request_limit_per_turn = 0\nauto_compact = false\n",
                fs::read_to_string(&config).unwrap()
            ),
        )
        .unwrap();
        let output = run_chat(
            root.path(),
            &config,
            "CSV aggregation: finish the neutral long task.\n/exit\n",
            &["--agent", definition.to_str().unwrap()],
        );
        assert!(
            output.status.success(),
            "{case}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let requests = server.requests.try_iter().collect::<Vec<_>>();
        assert_eq!(requests.len(), 21);
        server.join();
        let events = session_events(root.path(), &session_id(root.path()));
        let checkpoint = events
            .iter()
            .position(|event| event["event_type"] == "session.agent_definition_checkpoint_applied");
        assert_eq!(checkpoint.is_some(), case != "bad-handoff");
        if let Some(index) = checkpoint {
            assert_eq!(
                events[index - 1]["event_type"],
                "session.compaction_created"
            );
            assert_eq!(
                events[index]["payload"]["compaction_revision"],
                events[index - 1]["payload"]["summary"]["revision"]
            );
            for field in ["attempt_id", "parent_digest", "compaction_fingerprint"] {
                let mut changed = events.clone();
                changed[index]["payload"][field] = json!("foreign-checkpoint");
                assert_rejected_rsi_events(changed, field);
            }
            // A self-consistent allowed-scope replacement is still not the
            // candidate whose exact digest the accepted contrast bound.
            let mut changed = events[..=index].to_vec();
            let binding = &mut changed[index]["payload"]["binding"];
            let definition = &mut binding["definition"];
            let replacement = "A different, structurally valid persona that was never tested.";
            definition["instructions"]["persona"]["content"] = json!(replacement);
            definition["instructions"]["persona"]["content_digest"] =
                json!(blake3::hash(replacement.as_bytes()).to_hex().to_string());
            definition["instructions"]["persona"]["source_locator"] = json!({"kind":"evolution","agent_id":definition["id"],"parent_digest":definition["rsi"]["state"]["parent_digest"],"resource":"persona"});
            let semantic = json!({
                "schema_version":definition["schema_version"],"id":definition["id"],"name":definition["name"],
                "instructions":{"persona":replacement,"work":definition["instructions"]["work"]["content"],
                    "methods":definition["instructions"]["methods"].as_array().unwrap().iter().map(|method|method["content"].clone()).collect::<Vec<_>>()},
                "context":definition["context"],"tools":definition["tools"],"research":Value::Null,
                "rsi":{"persona":definition["rsi"]["persona"],"methods":definition["rsi"]["methods"],"improvement":true,
                    "fixed_rules":definition["rsi"]["fixed_rules"]["content"],"state":definition["rsi"]["state"]}
            });
            definition["digest"] = json!(
                blake3::hash(&serde_json::to_vec(&semantic).unwrap())
                    .to_hex()
                    .to_string()
            );
            serde_json::from_value::<echidna::domain::agent_definition::AgentDefinitionBinding>(
                binding.clone(),
            )
            .unwrap()
            .validate()
            .expect("tampered binding must be structurally and cryptographically valid");
            assert_rejected_rsi_events(changed, "untested valid persona/digest");
        }
        let project = Project::discover(root.path()).unwrap();
        let state = reduced_session(&project, &session_id(root.path()));
        let user_inputs = state
            .history()
            .items()
            .iter()
            .filter_map(|item| match item {
                echidna::domain::model::ContentItem::Text {
                    role: echidna::domain::model::MessageRole::User,
                    text,
                } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            user_inputs,
            ["CSV aggregation: finish the neutral long task."]
        );
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
        assert_eq!(calls.len(), 16, "ordinary tool calls were replayed or lost");
        assert_eq!(
            calls
                .iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            16
        );
        assert_eq!(state.execution_receipt_count(), 18);
        assert_eq!(
            fs::read_to_string(definition.join("persona.md")).unwrap(),
            old_persona
        );
    }
}

#[test]
fn long_checkpoint_does_not_repeat_considered_friction_or_spend_unavailable_budget() {
    for case in ["no-change", "low-budget"] {
        let root = TemporaryDirectory::new(&format!("rsi-checkpoint-{case}"));
        initialize_project(root.path());
        let definition = improvement_definition(root.path());
        improvement_fixture(root.path());
        fs::write(
            root.path().join("checkpoint-input.txt"),
            "neutral stable input",
        )
        .unwrap();
        let count = if case == "no-change" { 100 } else { 17 };
        let server = server(
            move |step, request| {
                if step == 0 {
                    return tool(
                        "one-failure",
                        "run_command",
                        json!({"argv":["/bin/sh","probe.sh","fail"]}),
                    );
                }
                if case == "no-change" && step == 16 {
                    assert!(request.to_string().contains("RSI2 diagnosis"));
                    return text_response(
                        r#"{"decision":"no_change","reason":"The retained failure is temporary; no supported reusable gain."}"#,
                        "no-checkpoint-change",
                    );
                }
                if step == count - 1 {
                    return text_response("TASK_COMPLETED", "checkpoint-final");
                }
                assert!(
                    request.get("tools").is_some(),
                    "unexpected repeated diagnosis at{step}"
                );
                assert!(
                    !request.to_string().contains("RSI2 diagnosis at"),
                    "diagnostic source leaked into the ordinary task Context at {step}"
                );
                tool(
                    &format!("fresh-read-{step}"),
                    "read_file",
                    json!({"path":"checkpoint-input.txt"}),
                )
            },
            count,
        );
        let config = write_config(root.path(), &server.base_url);
        fs::write(
            &config,
            format!(
                "{}\n[runtime]\nmodel_request_limit_per_turn = {}\nauto_compact = false\n",
                fs::read_to_string(&config).unwrap(),
                if case == "no-change" { 0 } else { 19 }
            ),
        )
        .unwrap();
        let output = run_chat(
            root.path(),
            &config,
            "Complete the neutral long task.\n/exit\n",
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
        let starts = events
            .iter()
            .filter(|event| event["payload"]["fact"]["kind"] == "checkpoint_started")
            .count();
        assert_eq!(starts, usize::from(case == "no-change"));
        assert!(
            events
                .iter()
                .all(|event| event["event_type"] != "session.agent_definition_checkpoint_applied")
        );
    }
}
