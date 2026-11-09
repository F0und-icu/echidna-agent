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
            move |step, request| {
                if step == 0 {
                    return initial_selection_response(request, "checkpoint-initial-selection");
                }
                let step = step - 1;
                match step {
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
                }
            },
            22,
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
        assert_eq!(requests.len(), 22);
        selector_candidates(&request_body(&requests[0]));
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
            let summary = &events[index - 1]["payload"]["summary"];
            assert_eq!(summary["schema_version"], 7);
            let root_sample = events[..index - 1]
                .iter()
                .find(|event| event["event_id"] == summary["model_sample_event_ref"])
                .expect("checkpoint Summary must reference its actual root sample");
            assert_eq!(
                root_sample["event_type"],
                "session.compaction_sample_recorded"
            );
            assert_eq!(root_sample["payload"]["graph"]["node"]["is_root"], true);
            assert_eq!(
                events[index]["payload"]["compaction_revision"],
                events[index - 1]["payload"]["summary"]["revision"]
            );
            for field in ["attempt_id", "parent_digest", "compaction_fingerprint"] {
                let mut changed = events.clone();
                changed[index]["payload"][field] = json!("foreign-checkpoint");
                assert_rejected_rsi_events(changed, field);
            }
            // Preserve a self-consistent v7 fingerprint and checkpoint hash:
            // an unrelated root identity must still fail the RSI boundary.
            let mut changed = events[..=index].to_vec();
            let summary = &mut changed[index - 1]["payload"]["summary"];
            summary["model_sample_event_ref"] = json!("foreign-graph-root");
            summary["fingerprint"] = json!("");
            summary["fingerprint"] = json!(
                blake3::hash(&serde_json::to_vec(summary).unwrap())
                    .to_hex()
                    .to_string()
            );
            let fingerprint =
                blake3::hash(&serde_json::to_vec(&changed[index - 1]["payload"]).unwrap())
                    .to_hex()
                    .to_string();
            changed[index]["payload"]["compaction_fingerprint"] = json!(fingerprint);
            let prior = changed[..index]
                .iter()
                .cloned()
                .map(|event| serde_json::from_value::<EventEnvelope<Value>>(event).unwrap())
                .collect::<Vec<_>>();
            SessionReducer
                .reduce(prior)
                .expect("the altered Compaction is structurally valid before the checkpoint");
            assert_rejected_rsi_events(
                changed,
                "self-consistent v7 Summary with unrelated graph root",
            );
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
            if case == "persona" {
                use echidna::domain::model::ContentItem;
                use echidna::domain::request_audit::RequestAuditManifest;
                use sha2::{Digest, Sha256};
                let position = (index + 1..events.len())
                    .find(|position| {
                        events[*position]["event_type"] == "session.request_audit_recorded"
                            && events[*position]["payload"]["provider_session"]
                                == events[index]["payload"]["primary_provider_session"]
                            && events[*position]["payload"]["improvement_scope"].is_null()
                    })
                    .expect("checkpoint has a later main audit");
                let prefix = events[..=position].to_vec();
                let event_path = root
                    .path()
                    .join(".echidna/sessions")
                    .join(session_id(root.path()))
                    .join("events.jsonl");
                let original = fs::read(&event_path).unwrap();
                let write_events = |values: &[Value]| {
                    fs::write(
                        &event_path,
                        values
                            .iter()
                            .map(|event| format!("{}\n", serde_json::to_string(event).unwrap()))
                            .collect::<String>(),
                    )
                    .unwrap()
                };
                write_events(&prefix);
                let valid = run_chat(root.path(), &config, "/exit\n", &["--resume", "latest"]);
                assert!(
                    valid.status.success(),
                    "checkpoint valid replay: {}",
                    String::from_utf8_lossy(&valid.stderr)
                );
                let mut forged = prefix.clone();
                let payload = &mut forged[position]["payload"];
                assert_eq!(payload["delivery"]["mode"], "full");
                let blobs = root.path().join(".echidna/artifacts/blobs/sha256");
                let manifest: RequestAuditManifest =
                    serde_json::from_value(payload.clone()).unwrap();
                let mut input = manifest
                    .reconstruct(|id| fs::read(blobs.join(id.as_str())))
                    .unwrap();
                let position = input
                    .content_items
                    .iter()
                    .position(|item| {
                        *item
                            == ContentItem::user_text(
                                "CSV aggregation: finish the neutral long task.",
                            )
                    })
                    .unwrap();
                let replacement = ContentItem::user_text("FORGED CHECKPOINT INPUT");
                input.content_items[position] = replacement.clone();
                let bytes = delivery_lineage::canonical_bytes(&replacement);
                let blob = format!("{:x}", Sha256::digest(&bytes));
                fs::write(blobs.join(&blob), &bytes).unwrap();
                let hash = delivery_lineage::canonical_hash(&replacement);
                for (components, map) in [
                    ("components", "context_delivery_map"),
                    ("full_logical_components", "full_context_delivery_map"),
                ] {
                    let component = payload[components]
                        .as_array_mut()
                        .unwrap()
                        .iter_mut()
                        .find(|entry| {
                            entry["identity"]["kind"] == "content_item"
                                && entry["identity"]["index"] == position
                        })
                        .unwrap();
                    component["blob_id"] = json!(blob);
                    component["size_bytes"] = json!(bytes.len());
                    component["semantic_hash"] = json!(hash);
                    let entry = payload[map]["content_items"]
                        .as_array_mut()
                        .unwrap()
                        .iter_mut()
                        .find(|entry| entry["final_position"] == position)
                        .unwrap();
                    entry["content_hash"] = json!(hash);
                }
                let input_hash = delivery_lineage::canonical_hash(&input);
                payload["delivered_input"]["semantic_hash"] = json!(input_hash);
                payload["full_logical_input"]["semantic_hash"] = json!(input_hash);
                let rebuilt: RequestAuditManifest =
                    serde_json::from_value(payload.clone()).unwrap();
                rebuilt.validate().unwrap();
                assert_eq!(
                    rebuilt
                        .reconstruct(|id| fs::read(blobs.join(id.as_str())))
                        .unwrap(),
                    input
                );
                SessionReducer
                    .reduce(forged.iter().cloned().map(|event| {
                        serde_json::from_value::<EventEnvelope<Value>>(event).unwrap()
                    }))
                    .expect("checkpoint forgery is structurally coherent");
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
                assert_eq!(fs::read(&event_path).unwrap(), before);
                fs::write(&event_path, original).unwrap();
            }
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
                    return initial_selection_response(request, "considered-initial-selection");
                }
                let step = step - 1;
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
            count + 1,
        );
        let config = write_config(root.path(), &server.base_url);
        fs::write(
            &config,
            format!(
                "{}\n[runtime]\nmodel_request_limit_per_turn = {}\nauto_compact = false\n",
                fs::read_to_string(&config).unwrap(),
                if case == "no-change" { 0 } else { 20 }
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
        let requests = server.requests.try_iter().collect::<Vec<_>>();
        assert_eq!(requests.len(), count + 1);
        selector_candidates(&request_body(&requests[0]));
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

#[test]
#[cfg(debug_assertions)]
fn failed_handoff_suppression_is_shared_between_rsi_and_ordinary_batches() {
    for ordinary_first in [true, false] {
        let root = TemporaryDirectory::new("rsi-shared-handoff-suppression");
        initialize_project(root.path());
        let definition = improvement_definition(root.path());
        improvement_fixture(root.path());
        fs::write(
            root.path().join("checkpoint-input.txt"),
            "neutral checkpoint input",
        )
        .unwrap();
        let old_persona = fs::read_to_string(definition.join("persona.md")).unwrap();
        let request_count = if ordinary_first { 21 } else { 22 };
        let server = server(
            move |step, request| {
                if step == 0 {
                    return initial_selection_response(request, "suppression-initial-selection");
                }
                let step = step - 1;
                let diagnosis = if ordinary_first { 17 } else { 16 };
                if ordinary_first && step == 2 || !ordinary_first && step == 19 {
                    assert!(request.get("tools").is_none());
                    assert!(
                        request["instructions"]
                            .as_str()
                            .unwrap()
                            .contains("continuation handoff")
                    );
                    return text_response("invalid handoff", "bad-handoff");
                }
                if step == diagnosis {
                    assert!(
                        request
                            .to_string()
                            .contains("operator task is still active")
                    );
                    return text_response(&improvement_candidate().to_string(), "diagnosis");
                }
                if step == diagnosis + 1 || step == diagnosis + 2 {
                    assert!(request.to_string().contains("RSI2 reference_match probe"));
                    return tool(
                        "same-scoped-id",
                        "run_command",
                        json!({"argv":["/bin/sh","probe.sh",if step == diagnosis + 1 { "bad" } else { "good" }]}),
                    );
                }
                if step == request_count - 1 {
                    assert!(
                        request["instructions"]
                            .as_str()
                            .unwrap()
                            .contains(&old_persona)
                    );
                    return text_response("ORIGINAL_TASK_COMPLETED", "final");
                }
                assert!(
                    request.get("tools").is_some(),
                    "unexpected handoff at step {step}"
                );
                let batch = if ordinary_first && step > 2 {
                    step - 1
                } else if !ordinary_first && step == 20 {
                    16
                } else {
                    step
                };
                let response = match batch {
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
                    16 => tool(
                        "large-later-batch",
                        "run_command",
                        json!({"argv":["/usr/bin/printf", "%s", "neutral-output ".repeat(4000)]}),
                    ),
                    _ => tool(
                        &format!("neutral-read-{batch}"),
                        "read_file",
                        json!({"path":"checkpoint-input.txt"}),
                    ),
                };
                if ordinary_first && batch < 2 {
                    format!(
                        "data: {}\n\n{response}",
                        json!({"type":"response.output_item.done", "output_index":0, "item":{"type":"reasoning", "content":[{"type":"reasoning_text", "text":"batch-reasoning ".repeat(if batch == 0 { 12_000 } else { 20_000 })}]}})
                    )
                } else {
                    response
                }
            },
            request_count + 1,
        );
        let config = write_config(root.path(), &server.base_url);
        fs::write(&config, format!("{}\n[runtime]\nmodel_request_limit_per_turn = 0\nauto_compact = true\n[runtime.context_budget]\npressure_bps = {}\ntarget_bps = {}\n", fs::read_to_string(&config).unwrap().replace("context_window = 32768", "context_window = 1048576"), if ordinary_first { 1000 } else { 200 }, if ordinary_first { 500 } else { 100 })).unwrap();
        let output = run_chat(
            root.path(),
            &config,
            "CSV aggregation: finish the neutral long task.\n/exit\n",
            &["--agent", definition.to_str().unwrap(), "--debug"],
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let requests = server.requests.try_iter().collect::<Vec<_>>();
        assert_eq!(requests.len(), request_count + 1);
        selector_candidates(&request_body(&requests[0]));
        server.join();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("settled batch evaluated; threshold_reached=true; automatic handoff deferred for this Turn"), "{stderr}");
        assert!(
            stderr.contains("candidate remains pending and old instructions remain active"),
            "{stderr}"
        );
        let project = Project::discover(root.path()).unwrap();
        let state = reduced_session(&project, &session_id(root.path()));
        assert_eq!(
            state.execution_receipt_count(),
            if ordinary_first { 18 } else { 19 }
        );
        let events = session_events(root.path(), &session_id(root.path()));
        assert!(
            events
                .iter()
                .all(|event| event["event_type"] != "session.agent_definition_checkpoint_applied")
        );
        assert!(
            events
                .iter()
                .all(|event| event["event_type"] != "session.compaction_created")
        );
    }
}
