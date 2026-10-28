//! Required source bytes must remain valid through adoption and graph replay.
use super::*;
use echidna::domain::model::ContentItem;

#[test]
fn legacy_omitted_followup_sources_are_verified_during_adoption_and_replay() {
    for (secondary, after_adoption, audit_component) in [
        (false, false, false),
        (true, false, false),
        (false, true, false),
        (true, true, false),
        (true, false, true),
        (true, true, true),
    ] {
        let root = TemporaryDirectory::new("legacy-followup-source");
        let harness = TemporaryDirectory::new("legacy-followup-config");
        initialize_project(root.path());
        let project = Project::discover(root.path()).unwrap();
        let target = append_research_snapshot(&project, "legacy-followup-source");
        let png = png_bytes(8, 8);
        fs::write(root.path().join("screen.png"), &png).unwrap();
        let primary = spawn_responses_server(vec![
            (
                200,
                text_response(&"Earlier preparation. ".repeat(900), "prepare"),
            ),
            (
                200,
                tool_calls_response(
                    &[(
                        "followup-call",
                        "view_image",
                        json!({"event_id":"followup-image-event", "occurred_at":"2026-09-12T00:00:00Z",
                            "artifact_id":"artifact-followup", "source_path":"screen.png",
                            "target_snapshot_id":"snapshot-legacy-followup-source", "toolchain_fingerprint":"fixture"}),
                    )],
                    "followup-response",
                ),
            ),
            (200, text_response("Image inspected.", "image-final")),
        ]);
        let vision = secondary.then(|| {
            spawn_responses_server(vec![(
                200,
                vision_response("original observation", "vision"),
            )])
        });
        let config = if let Some(vision) = &vision {
            write_dual_config(harness.path(), &primary.base_url, &vision.base_url)
        } else {
            write_direct_config(harness.path(), &primary.base_url)
        };
        let initial = run_chat(
            root.path(),
            &config,
            "Prepare.\nInspect the image.\n/exit\n",
            &["--domain", "mobile"],
        );
        assert!(
            initial.status.success(),
            "{}",
            String::from_utf8_lossy(&initial.stderr)
        );
        for _ in 0..3 {
            let _ = f2a_request_body(&primary.requests, "legacy followup Primary");
        }
        f2a_join_server(primary, "legacy followup Primary");
        if let Some(vision) = vision {
            let _ = f2a_request_body(&vision.requests, "legacy followup Vision");
            f2a_join_server(vision, "legacy followup Vision");
        }
        let id = session_id(root.path());
        let mut legacy = session_events(root.path(), &id);
        let original_event_count = legacy.len();
        let blob_id = if audit_component {
            let audit_id = legacy
                .iter()
                .find_map(|event| {
                    event["payload"]["request_audit_event_id"]
                        .as_str()
                        .filter(|_| event["event_type"] == "vision.observation_recorded")
                })
                .unwrap();
            let audit = legacy
                .iter()
                .find(|event| event["event_id"] == audit_id)
                .unwrap();
            audit["payload"]["components"]
                .as_array()
                .unwrap()
                .iter()
                .find_map(|component| {
                    let id = component["blob_id"].as_str().unwrap();
                    let bytes =
                        fs::read(root.path().join(".echidna/artifacts/blobs/sha256").join(id))
                            .unwrap();
                    String::from_utf8_lossy(&bytes)
                        .contains("vision_image_geometry")
                        .then(|| id.to_owned())
                })
                .expect("the real Vision geometry audit component")
        } else if secondary {
            legacy
                .iter()
                .find_map(|event| {
                    event["payload"]["raw_observation_blob_id"]
                        .as_str()
                        .map(str::to_owned)
                })
                .expect("a real secondary observation Blob")
        } else {
            format!("{:x}", Sha256::digest(&png))
        };
        let blob = root
            .path()
            .join(".echidna/artifacts/blobs/sha256")
            .join(blob_id);
        let original = fs::read(&blob).unwrap();
        let expected_error = if audit_component {
            "protected Vision observation verification failed"
        } else {
            "required Compaction source Blob"
        };
        legacy.retain(|event| {
            !matches!(
                event["event_type"].as_str(),
                Some(
                    "session.operator_turn_boundary_recorded" | "session.model_delivery_committed"
                )
            )
        });
        let removed_event_count = original_event_count - legacy.len();
        for (index, event) in legacy.iter_mut().enumerate() {
            event["sequence"] = json!(index + 1);
        }
        fs::write(
            root.path()
                .join(".echidna/sessions")
                .join(&id)
                .join("events.jsonl"),
            legacy
                .iter()
                .map(|event| format!("{event}\n"))
                .collect::<String>(),
        )
        .unwrap();
        append_v2_compaction_with_progress(&project, &id, target, &"Legacy progress. ".repeat(250));
        let before = reduced_session(&project, &id);
        assert!(
            before
                .latest_compaction()
                .unwrap()
                .retained_history()
                .is_empty()
        );
        let original_receipts = before.execution_receipt_count();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let changed = blob.clone();
        let server = thread::spawn(move || {
            let mut socket = f3_context::accept_before_deadline(&listener, "legacy source handoff");
            let wire = request_body(&read_http_request(&mut socket));
            assert!(wire.get("tools").is_none());
            if !after_adoption {
                if secondary && !audit_component {
                    fs::write(&changed, b"corrupt observation").unwrap();
                } else {
                    fs::remove_file(&changed).unwrap();
                }
            }
            f3_context::write_sse_response(
                &mut socket,
                &compaction_summary_response(
                    "Preparation complete; keep the image batch.",
                    "legacy-summary",
                ),
            );
        });
        let config = if secondary {
            write_dual_config(harness.path(), &url, "http://127.0.0.1:1/v1")
        } else {
            write_direct_config(harness.path(), &url)
        };
        // Preserve original audit IDs, whose generated suffixes contain the
        // former sequence, while advancing the fixture with real Resume events.
        for _ in 0..removed_event_count {
            let resumed = run_chat(root.path(), &config, "/exit\n", &["--resume", &id]);
            assert!(
                resumed.status.success(),
                "{}",
                String::from_utf8_lossy(&resumed.stderr)
            );
        }
        let compacted = run_chat(
            root.path(),
            &config,
            "/compact\n/exit\n",
            &["--resume", &id],
        );
        server.join().unwrap_or_else(|error| {
            panic!(
                "secondary={secondary} after_adoption={after_adoption}: {error:?}; stdout={} stderr={}",
                String::from_utf8_lossy(&compacted.stdout),
                String::from_utf8_lossy(&compacted.stderr)
            )
        });
        if after_adoption {
            assert!(
                compacted.status.success(),
                "{}",
                String::from_utf8_lossy(&compacted.stderr)
            );
            assert_eq!(
                reduced_session(&project, &id)
                    .latest_compaction()
                    .unwrap()
                    .summary()
                    .schema_version(),
                6
            );
            fs::write(&blob, b"corrupt restored source after adoption").unwrap();
            let before_replay = session_events(root.path(), &id);
            let replay = run_chat(root.path(), &config, "/exit\n", &["--resume", &id]);
            assert!(!replay.status.success());
            assert!(
                String::from_utf8_lossy(&replay.stderr).contains(expected_error),
                "{}",
                String::from_utf8_lossy(&replay.stderr)
            );
            assert_eq!(session_events(root.path(), &id), before_replay);
        } else {
            assert!(!compacted.status.success());
            assert!(
                String::from_utf8_lossy(&compacted.stderr).contains(expected_error),
                "{}",
                String::from_utf8_lossy(&compacted.stderr)
            );
            assert_eq!(
                reduced_session(&project, &id).latest_compaction(),
                before.latest_compaction()
            );
        }
        fs::write(&blob, original).unwrap();
        let after = reduced_session(&project, &id);
        assert_eq!(after.execution_receipt_count(), original_receipts);
        assert_eq!(after.history().items(), before.history().items());
    }
}

#[test]
fn missing_or_corrupt_protected_image_blocks_adoption_and_verified_replay() {
    for stage in [
        "missing-during-sample",
        "corrupt-during-sample",
        "missing-capture-during-sample",
        "after-adoption",
    ] {
        let root = TemporaryDirectory::new(stage);
        initialize_project(root.path());
        let png = png_bytes(12, 9);
        fs::write(root.path().join("original.png"), &png).unwrap();
        fs::write(root.path().join("large.txt"), "payload ".repeat(15_000)).unwrap();
        let blob_id = format!("{:x}", Sha256::digest(&png));
        let blob = root
            .path()
            .join(".echidna/artifacts/blobs/sha256")
            .join(blob_id);
        let definition = root.path().join("blob-agent");
        fs::create_dir(&definition).unwrap();
        fs::write(
            definition.join("persona.md"),
            "A precise offline assistant.",
        )
        .unwrap();
        fs::write(
            definition.join("work.md"),
            "Preserve the original task and image.",
        )
        .unwrap();
        fs::write(
            definition.join("agent.toml"),
            r#"schema_version = 1
id = "blob-agent"
name = "Blob source fixture"
[instructions]
persona_file = "persona.md"
work_file = "work.md"
method_files = []
[context]
personal = "none"
project = "none"
memory = "none"
[tools]
read_file = "direct"
"#,
        )
        .unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let (sender, requests) = mpsc::channel();
        let changed_blob = blob.clone();
        let fixture_root = root.path().to_path_buf();
        let thread = thread::spawn(move || {
            let count = if stage == "after-adoption" { 5 } else { 4 };
            for index in 0..count {
                let mut socket =
                    f3_context::accept_before_deadline(&listener, "source Blob request");
                socket
                    .set_read_timeout(Some(Duration::from_secs(10)))
                    .unwrap();
                let request = request_body(&read_http_request(&mut socket));
                if index == 3 {
                    assert!(
                        request.get("tools").is_none(),
                        "expected a tool-free handoff"
                    );
                    match stage {
                        "missing-during-sample" => fs::remove_file(&changed_blob).unwrap(),
                        "corrupt-during-sample" => {
                            fs::write(&changed_blob, b"corrupt source").unwrap()
                        }
                        "missing-capture-during-sample" => {
                            let session = session_id(&fixture_root);
                            let events = session_events(&fixture_root, &session);
                            let id = events
                                .iter()
                                .find(|event| {
                                    event["event_type"] == "session.compaction_context_captured"
                                })
                                .unwrap()["payload"]["blob_id"]
                                .as_str()
                                .unwrap();
                            fs::remove_file(
                                fixture_root
                                    .join(".echidna/artifacts/blobs/sha256")
                                    .join(id),
                            )
                            .unwrap();
                        }
                        _ => {}
                    }
                }
                sender.send(request).unwrap();
                let response = match index {
                    0..=2 => m4_long_flow_tool_response(
                        &format!("read-{index}"),
                        "read_file",
                        json!({"path":"large.txt"}),
                        &format!("read-response-{index}"),
                    ),
                    3 => compaction_summary_response(
                        "Earlier reads completed; preserve the original input.",
                        "blob-summary",
                    ),
                    _ => text_response("COMPLETE", "blob-final"),
                };
                f3_context::write_sse_response(&mut socket, &response);
            }
        });
        let config = write_direct_config(root.path(), &url);
        fs::write(
            &config,
            format!(
                "{}\n[runtime]\nauto_compact = true\n",
                fs::read_to_string(&config).unwrap()
            ),
        )
        .unwrap();
        let output = run_chat(
            root.path(),
            &config,
            "/image original.png\nPerform the neutral reads and retain the image.\n/exit\n",
            &["--agent", definition.to_str().unwrap()],
        );
        thread.join().unwrap();
        let wires = requests.try_iter().collect::<Vec<_>>();
        assert_eq!(wires.len(), if stage == "after-adoption" { 5 } else { 4 });
        let id = session_id(root.path());
        let events = session_events(root.path(), &id);
        let compactions = events
            .iter()
            .filter(|event| event["event_type"] == "session.compaction_created")
            .count();
        if stage == "after-adoption" {
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(compactions, 1);
            fs::write(&blob, b"corrupt after successful adoption").unwrap();
            let resumed = run_chat(root.path(), &config, "/exit\n", &["--resume", "latest"]);
            assert!(!resumed.status.success());
            assert!(
                String::from_utf8_lossy(&resumed.stderr)
                    .contains("required Compaction source Blob")
            );
            assert_eq!(session_events(root.path(), &id), events);
        } else {
            assert!(!output.status.success());
            let expected = if stage == "missing-capture-during-sample" {
                "Compaction context Blob read failed"
            } else {
                "required Compaction source Blob"
            };
            assert!(
                String::from_utf8_lossy(&output.stderr).contains(expected),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(compactions, 0, "a root sample is not durable adoption");
        }
        assert_eq!(
            events
                .iter()
                .filter(|event| event["event_type"] == "tool.execution_outcome_recorded")
                .count(),
            3
        );
        fs::write(blob, &png).unwrap();
        let state = reduced_session(&Project::discover(root.path()).unwrap(), &id);
        assert_eq!(state.history().ensure_all_tool_calls_paired(), Ok(()));
        assert_eq!(
            state
                .history()
                .items()
                .iter()
                .filter(|item| matches!(item, ContentItem::ToolCall { .. }))
                .count(),
            3
        );
    }
}
