//! Actual Secondary Vision proof must remain readable and match its observation.
use super::task_context::{ScriptedResponse, spawn_scripted_responses_server};
use super::*;

const TASK_VISION_MARKER: &str = "VISION-AUTHORITY-FIXTURE";

fn vision_candidate(candidates: &Value) -> &Value {
    let candidate = candidates["context"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| {
            item["excerpt"]
                .as_str()
                .is_some_and(|text| text.contains(TASK_VISION_MARKER))
        })
        .expect("Vision observation is offered as a source");
    assert_eq!(candidate["source_origin"], "model_output");
    assert_eq!(candidate["source_authority"], "model_observation");
    candidate
}

fn vision_decision(candidates: &Value) -> Value {
    let source = vision_candidate(candidates);
    json!({
        "relation":"continue", "focus":{"kind":"existing", "task_ref":candidates["tasks"][0]["ref"]},
        "related_task_refs":[], "required_context_refs":[source["ref"]],
        "unresolved_references":[]
    })
}

fn vision_excerpt_decision(candidates: &Value) -> Value {
    let source = vision_candidate(candidates);
    let start = source["excerpt"]
        .as_str()
        .unwrap()
        .find(TASK_VISION_MARKER)
        .unwrap();
    let mut decision = vision_decision(candidates);
    decision["required_context_refs"] = json!([]);
    decision["source_ranges"] = json!([{"context_ref":source["ref"], "start_byte":start, "end_byte":start + TASK_VISION_MARKER.len()}]);
    decision
}

fn vision_constraint_decision(candidates: &Value) -> Value {
    let source = vision_candidate(candidates);
    let mut decision = vision_decision(candidates);
    decision["required_context_refs"] = json!([]);
    decision["constraint_updates"] = json!([{"source_ref":source["ref"], "start_byte":0, "end_byte":1, "scope":{"kind":"global"}, "supersedes":null}]);
    decision
}

#[test]
fn task_selection_preserves_vision_authority_for_history_and_excerpts_and_rejects_constraints() {
    for decide in [
        vision_decision as fn(&Value) -> Value,
        vision_excerpt_decision,
        vision_constraint_decision,
    ] {
        let root = TemporaryDirectory::new("task-vision-authority");
        initialize_project(root.path());
        let agent = root.path().join("vision-agent");
        fs::create_dir(&agent).unwrap();
        fs::write(agent.join("persona.md"), "A synthetic diagram assistant.").unwrap();
        fs::write(
            agent.join("work.md"),
            "Report only the supplied observation.",
        )
        .unwrap();
        fs::write(agent.join("agent.toml"), "schema_version = 1\nid = \"vision-authority\"\nname = \"Vision authority\"\n[instructions]\npersona_file = \"persona.md\"\nwork_file = \"work.md\"\nmethod_files = []\n[context]\npersonal = \"none\"\nproject = \"none\"\nmemory = \"none\"\n[tools]\nview_image = \"direct\"\n").unwrap();
        fs::write(root.path().join("screen.png"), png_bytes(8, 8)).unwrap();
        let invalid =
            std::ptr::fn_addr_eq(decide, vision_constraint_decision as fn(&Value) -> Value);
        let mut replies = vec![
            ScriptedResponse::InitialSelection,
            ScriptedResponse::Response(200, text_response("first answer", "vision-first")),
            ScriptedResponse::SelectionDecision(decide),
        ];
        if !invalid {
            replies.push(ScriptedResponse::Response(
                200,
                text_response("second answer", "vision-second"),
            ));
        }
        let primary = spawn_scripted_responses_server(replies);
        let vision = spawn_responses_server(vec![(
            200,
            vision_response(TASK_VISION_MARKER, "vision-observation"),
        )]);
        let config = write_dual_config(root.path(), &primary.base_url, &vision.base_url);
        let output = run_chat(
            root.path(),
            &config,
            "/image screen.png\nExplain this image.\nContinue using the observation.\n/exit\n",
            &["--agent", agent.to_str().unwrap()],
        );
        assert_eq!(
            output.status.success(),
            !invalid,
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        super::task_context::selector_candidates(&f2a_request_body(
            &primary.requests,
            "vision initial selection",
        ));
        let first = f2a_request_body(&primary.requests, "vision initial Primary");
        let selector = f2a_request_body(&primary.requests, "vision selector");
        assert!(json_contains_text(&first["input"], TASK_VISION_MARKER));
        let candidates = super::task_context::selector_candidates(&selector);
        let source = vision_candidate(&candidates)["ref"].as_str().unwrap();
        if !invalid {
            let second = f2a_request_body(&primary.requests, "vision selected Primary");
            assert!(json_contains_text(&second["input"], TASK_VISION_MARKER));
            let project = Project::discover(root.path()).unwrap();
            let state = reduced_session(&project, &session_id(root.path()));
            let audit = state.request_audits().last().unwrap();
            let map = serde_json::to_value(audit.context_delivery_map().unwrap()).unwrap();
            let matching = map["content_items"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|item| {
                    item["provenance"]["references"]["source_event_refs"]
                        .as_array()
                        .is_some_and(|refs| refs.iter().any(|id| id == source))
                })
                .collect::<Vec<_>>();
            assert!(
                !matching.is_empty(),
                "selected Vision source has provenance"
            );
            for item in matching {
                assert_eq!(item["provenance"]["authority_level"], "model_observation");
            }
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(stderr.contains("constraint update is not bound to the inspected input or an existing constraint"), "{stderr}");
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
        }
        primary.join();
        let _ = f2a_request_body(&vision.requests, "vision provider");
        vision.join();
    }
}

#[test]
fn selected_vision_history_and_excerpt_keep_reference_chain_through_resume() {
    for (representation, decide) in [
        ("full", vision_decision as fn(&Value) -> Value),
        ("excerpt", vision_excerpt_decision as fn(&Value) -> Value),
    ] {
        let root = TemporaryDirectory::new("task-vision-excerpt-references");
        initialize_project(root.path());
        let agent = root.path().join("vision-agent");
        fs::create_dir(&agent).unwrap();
        fs::write(agent.join("persona.md"), "A synthetic diagram assistant.").unwrap();
        fs::write(
            agent.join("work.md"),
            "Report only the supplied observation.",
        )
        .unwrap();
        fs::write(agent.join("agent.toml"), "schema_version = 1\nid = \"vision-authority\"\nname = \"Vision authority\"\n[instructions]\npersona_file = \"persona.md\"\nwork_file = \"work.md\"\nmethod_files = []\n[context]\npersonal = \"none\"\nproject = \"none\"\nmemory = \"none\"\n[tools]\nview_image = \"direct\"\n").unwrap();
        fs::write(root.path().join("screen.png"), png_bytes(8, 8)).unwrap();
        let primary = super::task_context::spawn_scripted_responses_server_with_wait(
            vec![
                ScriptedResponse::InitialSelection,
                ScriptedResponse::Response(
                    200,
                    text_response("first answer", "vision-chain-first"),
                ),
                ScriptedResponse::SelectionDecision(decide),
                ScriptedResponse::Response(
                    200,
                    text_response("excerpt selected", "vision-chain-second"),
                ),
                ScriptedResponse::SelectionDecision(decide),
                ScriptedResponse::Response(
                    200,
                    text_response("excerpt resumed", "vision-chain-resumed"),
                ),
            ],
            Duration::from_secs(60),
        );
        let vision = spawn_responses_server(vec![(
            200,
            vision_response(TASK_VISION_MARKER, "vision-chain-observation"),
        )]);
        let config = write_dual_config(root.path(), &primary.base_url, &vision.base_url);
        let first = run_chat(
            root.path(),
            &config,
            "/image screen.png\nExplain this image.\nContinue using the observation.\n/exit\n",
            &["--agent", agent.to_str().unwrap()],
        );
        assert!(
            first.status.success(),
            "{}",
            String::from_utf8_lossy(&first.stderr)
        );
        super::task_context::selector_candidates(&f2a_request_body(
            &primary.requests,
            "Vision initial selection",
        ));
        let _first_main = f2a_request_body(&primary.requests, "Vision first main");
        let selector = f2a_request_body(&primary.requests, "Vision excerpt selection");
        let candidates = super::task_context::selector_candidates(&selector);
        let source = vision_candidate(&candidates)["ref"]
            .as_str()
            .unwrap()
            .to_owned();
        let excerpt_main = f2a_request_body(&primary.requests, "Vision excerpt main");
        assert!(json_contains_text(
            &excerpt_main["input"],
            TASK_VISION_MARKER
        ));
        let id = session_id(root.path());
        let project = Project::discover(root.path()).unwrap();
        let events = session_events(root.path(), &id);
        let selected = events
            .iter()
            .rev()
            .find(|event| event["event_type"] == "session.task_context_selected")
            .unwrap();
        if representation == "excerpt" {
            let ranges = selected["payload"]["selected_excerpts"].as_array().unwrap();
            assert_eq!(ranges.len(), 1);
            assert_eq!(ranges[0]["source_event_id"], json!(source));
            assert_eq!(
                ranges[0]["end_byte"].as_u64().unwrap() - ranges[0]["start_byte"].as_u64().unwrap(),
                TASK_VISION_MARKER.len() as u64
            );
        } else {
            assert!(
                selected["payload"]["selected_history"]
                    .as_array()
                    .unwrap()
                    .contains(&json!(source))
            );
        }
        let observation = events
            .iter()
            .find(|event| event["event_type"] == "vision.observation_recorded")
            .unwrap();
        let payload = &observation["payload"];
        let event_refs = [
            json!(source),
            payload["route_event_id"].clone(),
            payload["request_audit_event_id"].clone(),
            observation["event_id"].clone(),
        ];
        let mut artifact_refs = vec![payload["source_artifact_id"].clone()];
        artifact_refs.extend(
            payload["derived_artifact_ids"]
                .as_array()
                .unwrap()
                .iter()
                .cloned(),
        );
        for reference in &event_refs {
            assert!(events.iter().any(|event| event["event_id"] == *reference));
        }
        let check = |state: &SessionState| {
            let map = serde_json::to_value(
                state
                    .request_audits()
                    .last()
                    .unwrap()
                    .context_delivery_map()
                    .unwrap(),
            )
            .unwrap();
            let matching = map["content_items"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|item| {
                    item["provenance"]["authority_level"] == "model_observation"
                        && item["provenance"]["references"]["source_event_refs"]
                            .as_array()
                            .is_some_and(|refs| refs.contains(&event_refs[0]))
                })
                .collect::<Vec<_>>();
            assert!(
                !matching.is_empty(),
                "selected Vision source lacks observation authority"
            );
            assert!(
                matching.iter().any(|item| {
                    let refs = &item["provenance"]["references"];
                    let actual_events = refs["source_event_refs"].as_array().unwrap();
                    event_refs
                        .iter()
                        .all(|expected| actual_events.contains(expected))
                        && refs["artifact_refs"]
                            .as_array()
                            .is_some_and(|actual_artifacts| {
                                artifact_refs
                                    .iter()
                                    .all(|expected| actual_artifacts.contains(expected))
                            })
                }),
                "selected Vision source must preserve its original event and Artifact chain"
            );
        };
        let state = reduced_session(&project, &id);
        check(&state);
        assert_audits_reconstruct(root.path(), &state);

        let resumed = run_chat(
            root.path(),
            &config,
            "Continue with the same observation.\n/exit\n",
            &["--resume", &id],
        );
        assert!(
            resumed.status.success(),
            "{}",
            String::from_utf8_lossy(&resumed.stderr)
        );
        super::task_context::selector_candidates(&f2a_request_body(
            &primary.requests,
            "Vision resumed selection",
        ));
        let resumed_main = f2a_request_body(&primary.requests, "Vision resumed main");
        assert!(json_contains_text(
            &resumed_main["input"],
            TASK_VISION_MARKER
        ));
        assert!(primary.requests.try_recv().is_err());
        primary.join();
        let _ = f2a_request_body(&vision.requests, "Vision provider");
        vision.join();
        let state = reduced_session(&project, &id);
        check(&state);
        assert_audits_reconstruct(root.path(), &state);
    }
}

#[test]
fn protected_secondary_requires_audited_geometry_and_matching_raw_reply() {
    for followup in [false, true] {
        for tamper in ["missing-audit", "corrupt-audit", "different-reply"] {
            let root = TemporaryDirectory::new("protected-secondary-proof");
            let harness = TemporaryDirectory::new("protected-secondary-config");
            initialize_project(root.path());
            let project = Project::discover(root.path()).unwrap();
            let target = append_research_snapshot(&project, "secondary-proof");
            fs::write(root.path().join("screen.png"), png_bytes(8, 8)).unwrap();
            let replies = if followup {
                vec![
                    (
                        200,
                        tool_calls_response(
                            &[(
                                "proof-image",
                                "view_image",
                                json!({
                                    "event_id":"proof-image-event", "occurred_at":"2026-09-12T00:00:00Z",
                                    "artifact_id":"proof-image-artifact", "source_path":"screen.png",
                                    "target_snapshot_id":"snapshot-secondary-proof", "toolchain_fingerprint":"fixture"
                                }),
                            )],
                            "proof-tool",
                        ),
                    ),
                    (200, text_response("inspected", "proof-final")),
                ]
            } else {
                vec![(200, text_response("inspected", "proof-final"))]
            };
            let primary = spawn_scripted_responses_server(
                std::iter::once(ScriptedResponse::InitialSelection)
                    .chain(
                        replies
                            .into_iter()
                            .map(|(status, body)| ScriptedResponse::Response(status, body)),
                    )
                    .collect(),
            );
            let vision = spawn_responses_server(vec![(
                200,
                vision_response("original observation", "proof-vision"),
            )]);
            let config = write_dual_config(harness.path(), &primary.base_url, &vision.base_url);
            let input = if followup {
                "Inspect image.\n/exit\n"
            } else {
                "/image screen.png\nInspect image.\n/exit\n"
            };
            let initial = run_chat(root.path(), &config, input, &["--domain", "mobile"]);
            assert!(
                initial.status.success(),
                "{}",
                String::from_utf8_lossy(&initial.stderr)
            );
            let selector = f2a_request_body(&primary.requests, "proof task selector");
            super::task_context::selector_candidates(&selector);
            for _ in 0..if followup { 2 } else { 1 } {
                let _ = f2a_request_body(&primary.requests, "proof Primary");
            }
            f2a_join_server(primary, "proof Primary");
            let _ = f2a_request_body(&vision.requests, "proof Vision");
            f2a_join_server(vision, "proof Vision");
            let original_id = session_id(root.path());
            let (root, id, target) = if followup {
                // Start from the byte-verified pre-task-context Secondary fixture.
                // The boundary removal below is the original synthetic legacy
                // projection for this protected-Vision proof, not a rewrite of
                // a task capture or accepted task source.
                let legacy = TemporaryDirectory::new("protected-secondary-legacy");
                let id = load_pre_task_context_fixture(legacy.path(), "secondary");
                (
                    legacy,
                    id,
                    TargetSnapshotId::new("snapshot-legacy-followup-source"),
                )
            } else {
                (root, original_id, target)
            };
            let project = Project::discover(root.path()).unwrap();
            let mut events = session_events(root.path(), &id);
            if followup {
                events.retain(|event| {
                    !matches!(
                        event["event_type"].as_str(),
                        Some(
                            "session.operator_turn_boundary_recorded"
                                | "session.model_delivery_committed"
                        )
                    )
                });
                for (index, event) in events.iter_mut().enumerate() {
                    event["sequence"] = json!(index + 1);
                }
            } else {
                let accepted = events
                    .iter()
                    .position(|event| {
                        event["event_type"] == "session.operator_turn_boundary_recorded"
                            && event["payload"]["kind"] == "accepted"
                    })
                    .unwrap();
                events.truncate(accepted);
            }
            let observation_index = events
                .iter()
                .position(|event| event["event_type"] == "vision.observation_recorded")
                .unwrap();
            let audit_id = events[observation_index]["payload"]["request_audit_event_id"]
                .as_str()
                .unwrap()
                .to_owned();
            let audit = events
                .iter()
                .find(|event| event["event_id"] == audit_id)
                .unwrap();
            let blobs = root.path().join(".echidna/artifacts/blobs/sha256");
            let geometry_blob = audit["payload"]["components"]
                .as_array()
                .unwrap()
                .iter()
                .find_map(|component| {
                    let id = component["blob_id"].as_str()?;
                    let bytes = fs::read(blobs.join(id)).ok()?;
                    String::from_utf8_lossy(&bytes)
                        .contains("vision_image_geometry")
                        .then(|| blobs.join(id))
                })
                .expect("real Vision request includes its geometry component");
            if tamper == "different-reply" {
                let other = json!({"schema_version":2, "summary":"different valid observation", "extracted_text":"fixture screenshot", "regions":[], "task_observations":["fixture boundary marker"], "uncertainty":["synthetic input only"]}).to_string().into_bytes();
                let other_id = format!("{:x}", Sha256::digest(&other));
                fs::write(blobs.join(&other_id), &other).unwrap();
                events[observation_index]["payload"]["raw_observation_blob_id"] = json!(other_id);
            }
            let path = root
                .path()
                .join(".echidna/sessions")
                .join(&id)
                .join("events.jsonl");
            fs::write(
                &path,
                events
                    .iter()
                    .map(|event| format!("{event}\n"))
                    .collect::<String>(),
            )
            .unwrap();
            if followup {
                append_v2_compaction_with_progress(&project, &id, target, "A legacy summary.");
                assert!(
                    reduced_session(&project, &id)
                        .latest_compaction()
                        .unwrap()
                        .retained_history()
                        .is_empty()
                );
            }
            match tamper {
                "missing-audit" => fs::remove_file(&geometry_blob).unwrap(),
                "corrupt-audit" => {
                    fs::write(&geometry_blob, b"corrupt Vision audit component").unwrap()
                }
                _ => {}
            }
            let before = session_events(root.path(), &id);
            let inert = write_dual_config(
                harness.path(),
                "http://127.0.0.1:1/v1",
                "http://127.0.0.1:1/v1",
            );
            let rejected = run_chat(
                root.path(),
                &inert,
                "/exit\n",
                &["--domain", "mobile", "--resume", &id],
            );
            let stderr = String::from_utf8_lossy(&rejected.stderr);
            assert!(
                !rejected.status.success(),
                "followup={followup} tamper={tamper}: {stderr}"
            );
            let expected = if tamper == "different-reply" {
                "does not match the durable observation fact"
            } else {
                "Vision geometry RequestAudit cannot be reconstructed"
            };
            assert!(
                stderr.contains(expected),
                "followup={followup} tamper={tamper}: {stderr}"
            );
            assert_eq!(session_events(root.path(), &id), before);
        }
    }
}
