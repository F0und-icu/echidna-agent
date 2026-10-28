//! Actual Secondary Vision proof must remain readable and match its observation.
use super::*;

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
            let primary = spawn_responses_server(replies);
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
            for _ in 0..if followup { 2 } else { 1 } {
                let _ = f2a_request_body(&primary.requests, "proof Primary");
            }
            f2a_join_server(primary, "proof Primary");
            let _ = f2a_request_body(&vision.requests, "proof Vision");
            f2a_join_server(vision, "proof Vision");
            let id = session_id(root.path());
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
