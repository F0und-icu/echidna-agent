//! Recovery of the real History prefix immediately before input acceptance.
use super::*;

#[test]
fn initial_vision_checkpoint_before_acceptance_resumes_without_claiming_delivery() {
    for (secondary, derived) in [(false, false), (false, true), (true, false)] {
        let root = TemporaryDirectory::new("initial-image-before-acceptance");
        let harness = TemporaryDirectory::new("initial-image-config");
        initialize_project(root.path());
        let project = Project::discover(root.path()).unwrap();
        append_research_snapshot(&project, "initial-image-prefix");
        fs::write(
            root.path().join("screen.png"),
            png_bytes(if derived { 2050 } else { 8 }, 8),
        )
        .unwrap();
        let primary = spawn_responses_server(vec![(200, text_response("first answer", "first"))]);
        let vision = secondary.then(|| {
            spawn_responses_server(vec![(200, vision_response("saved observation", "vision"))])
        });
        let config = if let Some(vision) = &vision {
            write_dual_config(harness.path(), &primary.base_url, &vision.base_url)
        } else {
            write_direct_config(harness.path(), &primary.base_url)
        };
        let first = run_chat(
            root.path(),
            &config,
            "/image screen.png\nORIGINAL-IMAGE-TASK\n/exit\n",
            &[],
        );
        assert!(
            first.status.success(),
            "{}",
            String::from_utf8_lossy(&first.stderr)
        );
        let _ = f2a_request_body(&primary.requests, "initial Primary");
        f2a_join_server(primary, "initial Primary");
        if let Some(vision) = vision {
            let _ = f2a_request_body(&vision.requests, "initial Vision");
            f2a_join_server(vision, "initial Vision");
        }
        let id = session_id(root.path());
        let events = session_events(root.path(), &id);
        let accepted = events
            .iter()
            .position(|event| {
                event["event_type"] == "session.operator_turn_boundary_recorded"
                    && event["payload"]["kind"] == "accepted"
            })
            .unwrap();
        let prefix = &events[..accepted];
        assert!(
            prefix
                .iter()
                .any(|event| event["event_type"] == "vision.route_decided")
        );
        assert!(
            !prefix
                .iter()
                .any(|event| event["event_type"] == "session.model_delivery_committed")
        );
        if derived {
            assert!(
                prefix.iter().any(|event| {
                    event["payload"]["receipt"]["output_artifact_ids"]
                        .as_array()
                        .is_some_and(|ids| ids.len() == 2)
                }),
                "the direct image fixture must actually create a derivative"
            );
        }
        let path = root
            .path()
            .join(".echidna/sessions")
            .join(&id)
            .join("events.jsonl");
        fs::write(
            &path,
            prefix
                .iter()
                .map(|event| format!("{event}\n"))
                .collect::<String>(),
        )
        .unwrap();
        let resumed_server =
            spawn_responses_server(vec![(200, text_response("recovered", "recovered"))]);
        let resumed_config = if secondary {
            write_dual_config(
                harness.path(),
                &resumed_server.base_url,
                "http://127.0.0.1:1/v1",
            )
        } else {
            write_direct_config(harness.path(), &resumed_server.base_url)
        };
        let resumed = run_chat(
            root.path(),
            &resumed_config,
            "Continue.\n/exit\n",
            &["--resume", &id],
        );
        assert!(
            resumed.status.success(),
            "stdout={} stderr={}",
            String::from_utf8_lossy(&resumed.stdout),
            String::from_utf8_lossy(&resumed.stderr)
        );
        let wire = f2a_request_body(&resumed_server.requests, "recovered Primary");
        assert!(wire.to_string().contains("ORIGINAL-IMAGE-TASK"));
        if secondary {
            assert!(wire.to_string().contains("saved observation"));
        } else {
            assert!(wire.to_string().contains("data:image/png;base64,"));
        }
        f2a_join_server(resumed_server, "recovered Primary");
        let after = session_events(root.path(), &id);
        assert_eq!(&after[..accepted], prefix);
        assert!(
            !after[..accepted]
                .iter()
                .any(|event| event["event_type"] == "session.operator_turn_boundary_recorded")
        );
    }
}

#[test]
fn unaccepted_initial_vision_survives_failed_and_successful_compaction() {
    for (secondary, derived) in [(false, false), (false, true), (true, false)] {
        let root = TemporaryDirectory::new("initial-image-compaction-recovery");
        let harness = TemporaryDirectory::new("initial-image-compaction-config");
        initialize_project(root.path());
        let project = Project::discover(root.path()).unwrap();
        append_research_snapshot(&project, "initial-image-compaction");
        fs::write(
            root.path().join("screen.png"),
            png_bytes(if derived { 2050 } else { 8 }, 8),
        )
        .unwrap();
        let primary = spawn_responses_server(vec![
            (
                200,
                text_response(&"Earlier completed work. ".repeat(1000), "earlier"),
            ),
            (200, text_response("first image answer", "image-answer")),
        ]);
        let vision = secondary.then(|| {
            spawn_responses_server(vec![(
                200,
                vision_response("saved compaction observation", "vision"),
            )])
        });
        let config = if let Some(vision) = &vision {
            write_dual_config(harness.path(), &primary.base_url, &vision.base_url)
        } else {
            write_direct_config(harness.path(), &primary.base_url)
        };
        let first = run_chat(
            root.path(),
            &config,
            "Earlier task.\n/image screen.png\nORIGINAL-IMAGE-COMPACTION-TASK\n/exit\n",
            &[],
        );
        assert!(
            first.status.success(),
            "{}",
            String::from_utf8_lossy(&first.stderr)
        );
        for _ in 0..2 {
            let _ = f2a_request_body(&primary.requests, "initial Primary");
        }
        f2a_join_server(primary, "initial Primary");
        if let Some(vision) = vision {
            let _ = f2a_request_body(&vision.requests, "initial Vision");
            f2a_join_server(vision, "initial Vision");
        }
        let id = session_id(root.path());
        let events = session_events(root.path(), &id);
        let accepted = events
            .iter()
            .enumerate()
            .filter(|(_, event)| {
                event["event_type"] == "session.operator_turn_boundary_recorded"
                    && event["payload"]["kind"] == "accepted"
            })
            .nth(1)
            .unwrap()
            .0;
        let prefix = &events[..accepted];
        let path = root
            .path()
            .join(".echidna/sessions")
            .join(&id)
            .join("events.jsonl");
        fs::write(
            &path,
            prefix
                .iter()
                .map(|event| format!("{event}\n"))
                .collect::<String>(),
        )
        .unwrap();

        let failed_server = spawn_responses_server(vec![(
            200,
            text_response("not valid handoff JSON", "bad-summary"),
        )]);
        let failed_config = if secondary {
            write_dual_config(
                harness.path(),
                &failed_server.base_url,
                "http://127.0.0.1:1/v1",
            )
        } else {
            write_direct_config(harness.path(), &failed_server.base_url)
        };
        let failed = run_chat(
            root.path(),
            &failed_config,
            "/compact\n/exit\n",
            &["--resume", &id],
        );
        let _ = f2a_request_body(&failed_server.requests, "failed compaction");
        f2a_join_server(failed_server, "failed compaction");
        let after_failure = session_events(root.path(), &id);
        assert!(after_failure.len() > prefix.len());
        assert!(
            !after_failure
                .iter()
                .any(|event| event["event_type"] == "session.compaction_created")
        );
        assert!(
            String::from_utf8_lossy(&failed.stderr).contains("ompaction")
                || String::from_utf8_lossy(&failed.stdout).contains("ompaction")
        );

        let success_server = spawn_responses_server(vec![(
            200,
            compaction_summary_response("Earlier task completed.", "good-summary"),
        )]);
        let success_config = if secondary {
            write_dual_config(
                harness.path(),
                &success_server.base_url,
                "http://127.0.0.1:1/v1",
            )
        } else {
            write_direct_config(harness.path(), &success_server.base_url)
        };
        let success = run_chat(
            root.path(),
            &success_config,
            "/compact\n/exit\n",
            &["--resume", &id],
        );
        assert!(
            success.status.success(),
            "stdout={} stderr={}",
            String::from_utf8_lossy(&success.stdout),
            String::from_utf8_lossy(&success.stderr)
        );
        let _ = f2a_request_body(&success_server.requests, "successful compaction");
        f2a_join_server(success_server, "successful compaction");
        assert!(
            session_events(root.path(), &id)
                .iter()
                .any(|event| event["event_type"] == "session.compaction_created")
        );

        let resumed_server = spawn_responses_server(vec![(
            200,
            text_response("recovered after compaction", "final"),
        )]);
        let resumed_config = if secondary {
            write_dual_config(
                harness.path(),
                &resumed_server.base_url,
                "http://127.0.0.1:1/v1",
            )
        } else {
            write_direct_config(harness.path(), &resumed_server.base_url)
        };
        let resumed = run_chat(
            root.path(),
            &resumed_config,
            "Continue.\n/exit\n",
            &["--resume", &id],
        );
        assert!(
            resumed.status.success(),
            "stdout={} stderr={}",
            String::from_utf8_lossy(&resumed.stdout),
            String::from_utf8_lossy(&resumed.stderr)
        );
        let wire = f2a_request_body(&resumed_server.requests, "post-compaction Primary");
        assert!(wire.to_string().contains("ORIGINAL-IMAGE-COMPACTION-TASK"));
        assert!(wire.to_string().contains(if secondary {
            "saved compaction observation"
        } else {
            "data:image/png;base64,"
        }));
        f2a_join_server(resumed_server, "post-compaction Primary");
    }
}
