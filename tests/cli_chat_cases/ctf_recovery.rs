use super::task_context::{
    ScriptedResponse::{ContinueSelection, InitialSelection, Response},
    selector_candidates, spawn_scripted_responses_server,
    spawn_scripted_responses_server_with_wait,
};
use super::*;

fn limit_config(config: &std::path::Path, limit: Option<usize>, incremental: bool) {
    let mut text = fs::read_to_string(config)
        .unwrap()
        .replace("context_window = 32768", "context_window = 1048576");
    if !incremental {
        text = text.replace("incremental_input = true", "incremental_input = false");
    }
    if let Some(limit) = limit {
        text.push_str(&format!(
            "\n[runtime]\nmodel_request_limit_per_turn = {limit}\n"
        ));
    }
    fs::write(config, text).unwrap();
}

#[test]
fn ctf_r2b_budget_stop_requires_explicit_resume_without_repeating_input_or_tool() {
    for incremental in [false, true] {
        let root = TemporaryDirectory::new("ctf-r2b-resume");
        let harness = TemporaryDirectory::new("ctf-r2b-harness");
        initialize_project(root.path());
        fs::write(root.path().join("target.txt"), "bounded fixture").unwrap();
        let responses = vec![
            InitialSelection,
            Response(
                200,
                m4_long_flow_tool_response(
                    "r2b-read",
                    "read_file",
                    json!({"path":"target.txt"}),
                    "before-stop",
                ),
            ),
            ContinueSelection { usage: None },
            Response(
                200,
                text_response("explicit recovery completed", "after-resume"),
            ),
        ];
        let server = spawn_scripted_responses_server_with_wait(responses, Duration::from_secs(60));
        let config = write_config(harness.path(), &server.base_url);
        limit_config(&config, Some(2), incremental);
        // These bounds cover fixture completion, not product latency.
        let output = wait_child_before_deadline(
            spawn_f2a_chat(
                root.path(),
                &config,
                "original task\nnot accepted task\n/context\n/resume\n/exit\n",
                &[],
            ),
            "R2b explicit recovery",
            Duration::from_secs(60),
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{stdout}\n{stderr}");
        assert!(stderr.contains("per-Turn model-request limit of 2"));
        assert!(stdout.contains("Task input not accepted or saved"));
        assert!(stdout.contains("admitted=2 primary=2 vision=0 remaining=0 limit=2"));
        selector_candidates(&f2a_request_body(&server.requests, "R2b initial selection"));
        let _first = f2a_request_body(&server.requests, "R2b first");
        // The stopped Turn used exactly two admissions: selector and main.
        // The resumed input receives a fresh budget for its selector and main.
        let config = write_config(harness.path(), &server.base_url);
        limit_config(&config, Some(2), incremental);
        let resumed = wait_child_before_deadline(
            spawn_f2a_chat(
                root.path(),
                &config,
                "new instruction\n/exit\n",
                &["--resume", "latest"],
            ),
            "R2b selected recovery",
            Duration::from_secs(60),
        );
        assert!(
            resumed.status.success(),
            "{}",
            String::from_utf8_lossy(&resumed.stderr)
        );
        assert!(String::from_utf8_lossy(&resumed.stdout).contains("explicit recovery completed"));
        selector_candidates(&f2a_request_body(&server.requests, "R2b resumed selection"));
        let second = f2a_request_body(&server.requests, "R2b resumed");
        assert!(second.get("previous_response_id").is_none());
        assert!(json_contains_type(&second["input"], "function_call_output"));
        assert!(!second.to_string().contains("not accepted task"));
        f2a_join_server(server, "R2b resume");
        let project = Project::discover(root.path()).unwrap();
        let state = reduced_session(&project, &session_id(root.path()));
        assert_eq!(state.request_audits().len(), 4);
        assert_eq!(state.execution_receipt_count(), 1);
        assert_eq!(state.history().ensure_all_tool_calls_paired(), Ok(()));
        let users: Vec<_> = state
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
            .collect();
        assert_eq!(users, ["original task", "new instruction"]);
        assert_audits_reconstruct(root.path(), &state);
    }
}

#[test]
fn unlimited_budget_completes_more_than_32_steps_with_exact_durable_counts() {
    let tool_calls = 33usize;
    let root = TemporaryDirectory::new("unlimited-admission-cli");
    let harness = TemporaryDirectory::new("unlimited-admission-config");
    initialize_project(root.path());
    let mut responses = vec![InitialSelection];
    for index in 0..tool_calls {
        let path = format!("input-{index}.txt");
        fs::write(root.path().join(&path), format!("fixture {index}")).unwrap();
        responses.push(Response(
            200,
            m4_long_flow_tool_response(
                &format!("unlimited-call-{index}"),
                "read_file",
                json!({"path":path}),
                &format!("unlimited-response-{index}"),
            ),
        ));
    }
    responses.push(Response(
        200,
        text_response("UNLIMITED_COMPLETED", "unlimited-complete"),
    ));
    let server = spawn_scripted_responses_server(responses);
    let config = write_config(harness.path(), &server.base_url);
    limit_config(&config, Some(0), false);
    let output = wait_child_before_deadline(
        spawn_f2a_chat(
            root.path(),
            &config,
            "complete the bounded fixture\n/exit\n",
            &[],
        ),
        "unlimited admission fixture",
        Duration::from_secs(180),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stdout}\n{stderr}");
    assert!(stdout.contains("UNLIMITED_COMPLETED"));
    assert!(stdout.contains("admitted=35 primary=35 vision=0 remaining=unlimited limit=unlimited"));
    assert!(!stderr.contains("[turn stopped]"));
    selector_candidates(&f2a_request_body(
        &server.requests,
        "unlimited initial selection",
    ));
    for _ in 0..=tool_calls {
        f2a_request_body(&server.requests, "unlimited request");
    }
    assert!(server.requests.try_recv().is_err());
    f2a_join_server(server, "unlimited admission");
    let project = Project::discover(root.path()).unwrap();
    let state = reduced_session(&project, &session_id(root.path()));
    assert_eq!(state.request_audits().len(), 35);
    assert_eq!(
        state.execution_receipt_count(),
        u64::try_from(tool_calls).unwrap()
    );
    assert_eq!(state.history().items().len(), 2 + 2 * tool_calls);
    assert_eq!(state.history().ensure_all_tool_calls_paired(), Ok(()));
    assert_eq!(
        state.history().items().last(),
        Some(&echidna::domain::model::ContentItem::assistant_text(
            "UNLIMITED_COMPLETED"
        ))
    );
    assert_audits_reconstruct(root.path(), &state);
}

#[test]
fn ctf_r2b_default32_and_explicit33_have_no_hidden_primary_cap() {
    for configured in [Some(2), None, Some(33)] {
        let count = configured.unwrap_or(32);
        let root = TemporaryDirectory::new("ctf-r2b-limit");
        let harness = TemporaryDirectory::new("ctf-r2b-harness");
        initialize_project(root.path());
        fs::write(root.path().join("target.txt"), "fixture").unwrap();
        let mut responses = vec![InitialSelection];
        responses.extend((0..count - 1).map(|index| {
            Response(
                200,
                m4_long_flow_tool_response(
                    &format!("budget-call-{index}"),
                    "read_file",
                    // High-count lanes only need the request loop, not
                    // repeated filesystem execution/Receipt I/O. Invalid
                    // arguments yield real pre-execution paired failures.
                    if count > 2 {
                        json!({})
                    } else {
                        json!({"path":"target.txt"})
                    },
                    &format!("budget-response-{index}"),
                ),
            )
        }));
        let server = spawn_scripted_responses_server(responses);
        let config = write_config(harness.path(), &server.base_url);
        limit_config(&config, configured, false);
        let output = wait_child_before_deadline(
            spawn_f2a_chat(root.path(), &config, "bounded loop\n/exit\n", &[]),
            "R2b long budget fixture",
            Duration::from_secs(180),
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{stdout}\n{stderr}");
        assert!(stdout.contains("Use /resume"));
        assert!(
            stderr.contains(&format!("per-Turn model-request limit of {count}")),
            "{stderr}"
        );
        assert!(stdout.contains(&format!(
            "admitted={count} primary={count} vision=0 remaining=0 limit={count}"
        )));
        selector_candidates(&f2a_request_body(&server.requests, "R2b limit selection"));
        for _ in 1..count {
            let _ = f2a_request_body(&server.requests, "R2b limit");
        }
        assert!(server.requests.try_recv().is_err());
        f2a_join_server(server, "R2b limit");
        let project = Project::discover(root.path()).unwrap();
        let state = reduced_session(&project, &session_id(root.path()));
        assert_eq!(state.request_audits().len(), count);
        if count > 2 {
            assert_eq!(state.execution_receipt_count(), 0);
        }
        assert_eq!(state.history().ensure_all_tool_calls_paired(), Ok(()));
    }
}

#[test]
fn ctf_r2b_failed_provider_returns_to_prompt_without_committing_partial_output() {
    for response in [(400, "{\"error\":{\"message\":\"fixture rejection\"}}".to_owned()),
        (200, "data: {\"type\":\"response.output_text.delta\",\"delta\":\"uncommitted fragment\"}\n\n".to_owned())] {
        let root = TemporaryDirectory::new("ctf-r2b-provider");
        let harness = TemporaryDirectory::new("ctf-r2b-harness");
        initialize_project(root.path());
        let failed_attempts = if response.0 == 200 { 2 } else { 1 };
        let mut responses = vec![response; failed_attempts].into_iter()
            .map(|(status,body)| Response(status, body)).collect::<Vec<_>>();
        responses.insert(0, InitialSelection);
        responses.push(ContinueSelection { usage: None });
        responses.push(Response(200, text_response("recovered", "new-response")));
        let server = spawn_scripted_responses_server(responses);
        let config = write_config(harness.path(), &server.base_url);
        limit_config(&config, Some(3), true);
        let output = f2a_wait_child(spawn_f2a_chat(root.path(), &config,
            "first input\n/resume\nsecond input\n/exit\n", &[]), "R2b provider failure");
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        assert!(String::from_utf8_lossy(&output.stderr).contains("[turn stopped]"));
        selector_candidates(&f2a_request_body(&server.requests, "R2b failed input selection"));
        let first = f2a_request_body(&server.requests, "R2b failed");
        for _ in 1..failed_attempts {
            assert_eq!(first, f2a_request_body(&server.requests, "R2b failed retry"));
        }
        selector_candidates(&f2a_request_body(&server.requests, "R2b recovered source selection"));
        let second = f2a_request_body(&server.requests, "R2b new provider");
        assert!(second.get("previous_response_id").is_none());
        assert!(!second.to_string().contains("uncommitted fragment"));
        assert!(server.requests.try_recv().is_err());
        f2a_join_server(server, "R2b provider");
        let state = reduced_session(&Project::discover(root.path()).unwrap(), &session_id(root.path()));
        assert_eq!(state.request_audits().len(), failed_attempts + 3);
        assert_eq!(state.execution_receipt_count(), 0);
        assert!(!serde_json::to_string(state.history().items()).unwrap().contains("uncommitted fragment"));
        assert_audits_reconstruct(root.path(), &state);
    }
}

#[test]
fn ctf_r2b_initial_vision_shares_the_primary_admission_budget() {
    let root = TemporaryDirectory::new("ctf-r2b-initial-vision");
    let harness = TemporaryDirectory::new("ctf-r2b-harness");
    initialize_project(root.path());
    let project = Project::discover(root.path()).unwrap();
    append_research_snapshot(&project, "r2b-image");
    fs::write(root.path().join("screen.png"), png_bytes(8, 8)).unwrap();
    let vision = spawn_responses_server(vec![(
        200,
        vision_response("initial bounded observation", "vision-first"),
    )]);
    let primary = spawn_scripted_responses_server(vec![
        ContinueSelection { usage: None },
        Response(200, text_response("explicit image recovery", "primary-new")),
    ]);
    let config = write_dual_config(harness.path(), &primary.base_url, &vision.base_url);
    limit_config(&config, Some(1), false);
    let output = f2a_wait_child(
        spawn_f2a_chat(
            root.path(),
            &config,
            "/image screen.png\ninspect image\n/resume\n/exit\n",
            &[],
        ),
        "R2b initial Vision budget",
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("admitted=1 primary=0 vision=1 remaining=0 limit=1"));
    let _ = f2a_request_body(&vision.requests, "initial Vision");
    let config = write_dual_config(harness.path(), &primary.base_url, &vision.base_url);
    limit_config(&config, Some(3), false);
    let resumed = f2a_wait_child(
        spawn_f2a_chat(
            root.path(),
            &config,
            "continue with retained observation\n/exit\n",
            &["--resume", "latest"],
        ),
        "R2b initial Vision selected recovery",
    );
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    selector_candidates(&f2a_request_body(
        &primary.requests,
        "initial Vision resumed selection",
    ));
    let wire = f2a_request_body(&primary.requests, "resumed Primary");
    assert!(wire.get("previous_response_id").is_none());
    assert!(wire.to_string().contains("initial bounded observation"));
    f2a_join_server(vision, "initial Vision");
    f2a_join_server(primary, "resumed Primary");
    let state = reduced_session(&project, &session_id(root.path()));
    assert_eq!(state.request_audits().len(), 3);
    assert_eq!(state.vision_observations().len(), 1);
    assert_eq!(state.execution_receipt_count(), 1);
    assert_audits_reconstruct(root.path(), &state);
}

#[test]
fn ctf_r2b_initial_vision_failure_is_durable_and_does_not_requeue_input() {
    let root = TemporaryDirectory::new("ctf-r2b-vision-failure");
    let harness = TemporaryDirectory::new("ctf-r2b-harness");
    initialize_project(root.path());
    let project = Project::discover(root.path()).unwrap();
    append_research_snapshot(&project, "r2b-failed-image");
    fs::write(root.path().join("screen.png"), png_bytes(8, 8)).unwrap();
    let vision = spawn_responses_server(vec![(
        400,
        "{\"error\":{\"message\":\"fixture Vision failure\"}}".to_owned(),
    )]);
    let primary = spawn_scripted_responses_server(vec![
        InitialSelection,
        Response(200, text_response("text recovery", "primary-new")),
    ]);
    let config = write_dual_config(harness.path(), &primary.base_url, &vision.base_url);
    limit_config(&config, Some(3), false);
    let output = f2a_wait_child(
        spawn_f2a_chat(
            root.path(),
            &config,
            "/image screen.png\nFAILED_IMAGE_TASK\n/resume\nNEW_TEXT_TASK\n/exit\n",
            &[],
        ),
        "R2b Vision failure",
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("reattach each image explicitly"));
    let _ = f2a_request_body(&vision.requests, "failed Vision");
    selector_candidates(&f2a_request_body(
        &primary.requests,
        "failed Vision resumed selection",
    ));
    let wire = f2a_request_body(&primary.requests, "new text");
    assert!(!wire.to_string().contains("FAILED_IMAGE_TASK"));
    assert!(!json_contains_type(&wire["input"], "input_image"));
    f2a_join_server(vision, "failed Vision");
    f2a_join_server(primary, "new text");
    let state = reduced_session(&project, &session_id(root.path()));
    assert_eq!(state.request_audits().len(), 3);
    assert_eq!(state.vision_failures().len(), 1);
    assert!(state.vision_observations().is_empty());
    assert_eq!(state.execution_receipt_count(), 1);
    assert_audits_reconstruct(root.path(), &state);
}

#[test]
fn ctf_r2b_partial_image_followups_checkpoint_before_later_budget_stop() {
    let root = TemporaryDirectory::new("ctf-r2b-followup-budget");
    let harness = TemporaryDirectory::new("ctf-r2b-harness");
    initialize_project(root.path());
    let project = Project::discover(root.path()).unwrap();
    append_research_snapshot(&project, "r2b-followup");
    fs::write(root.path().join("screen.png"), png_bytes(8, 8)).unwrap();
    let calls: Vec<_> = (0..2).map(|index| (
        format!("r2b-image-{index}"),
        json!({"event_id":format!("r2b-image-event-{index}"), "occurred_at":"2026-09-12T00:00:00Z",
            "artifact_id":format!("artifact-r2b-{index}"), "source_path":"screen.png",
            "target_snapshot_id":"snapshot-r2b-followup", "toolchain_fingerprint":"r2b-fixture"}),
    )).collect();
    let calls: Vec<_> = calls
        .iter()
        .map(|(id, args)| (id.as_str(), "view_image", args.clone()))
        .collect();
    let primary = spawn_scripted_responses_server(vec![
        InitialSelection,
        Response(200, tool_calls_response(&calls, "image-tool-response")),
        ContinueSelection { usage: None },
        Response(
            200,
            text_response("resumed after image budget", "image-resumed"),
        ),
    ]);
    let vision = spawn_responses_server(vec![(
        200,
        vision_response("checkpointed first observation", "one-vision"),
    )]);
    let config = write_dual_config(harness.path(), &primary.base_url, &vision.base_url);
    limit_config(&config, Some(3), false);
    let output = f2a_wait_child(
        spawn_f2a_chat(
            root.path(),
            &config,
            "inspect both images\n/resume\ncontinue without rerunning images\n/exit\n",
            &["--domain", "mobile"],
        ),
        "R2b partial image checkpoints",
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("admitted=3 primary=2 vision=1 remaining=0 limit=3"));
    selector_candidates(&f2a_request_body(
        &primary.requests,
        "image initial selection",
    ));
    let _ = f2a_request_body(&primary.requests, "image calls");
    selector_candidates(&f2a_request_body(
        &primary.requests,
        "image resumed selection",
    ));
    let resumed = f2a_request_body(&primary.requests, "image resume");
    assert!(
        resumed
            .to_string()
            .contains("checkpointed first observation")
    );
    assert!(resumed.get("previous_response_id").is_none());
    let _ = f2a_request_body(&vision.requests, "single Vision");
    f2a_join_server(primary, "image Primary");
    f2a_join_server(vision, "image Vision");
    let state = reduced_session(&project, &session_id(root.path()));
    assert_eq!(state.request_audits().len(), 5);
    assert_eq!(state.execution_receipt_count(), 2);
    assert_eq!(state.vision_observations().len(), 1);
    assert_eq!(state.history().ensure_all_tool_calls_paired(), Ok(()));
    assert_audits_reconstruct(root.path(), &state);

    // A corrupted observation must never be reclassified as a new operator
    // instruction merely because its original provenance no longer matches.
    let session = session_id(root.path());
    let mut events = session_events(root.path(), &session);
    let observation = events
        .iter_mut()
        .find(|event| {
            event["event_type"] == "session.history_item_recorded"
                && event["payload"]["item"]["text"]
                    .as_str()
                    .is_some_and(|text| text.contains("checkpointed first observation"))
        })
        .expect("the real observation was checkpointed");
    let mut text: Value =
        serde_json::from_str(observation["payload"]["item"]["text"].as_str().unwrap()).unwrap();
    text["summary"] = json!("forged observation");
    observation["payload"]["item"]["text"] = json!(serde_json::to_string(&text).unwrap());
    let path = root
        .path()
        .join(".echidna/sessions")
        .join(&session)
        .join("events.jsonl");
    let altered = events
        .iter()
        .map(|event| serde_json::to_string(event).unwrap())
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    fs::write(&path, &altered).unwrap();
    let rejected = run_chat(
        root.path(),
        &config,
        "/exit\n",
        &["--domain", "mobile", "--resume", "latest"],
    );
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("no matching durable observation"));
    assert_eq!(fs::read_to_string(path).unwrap(), altered);
}

#[test]
fn ctf_r2b_direct_image_followup_survives_budget_stop_before_primary_delivery() {
    let root = TemporaryDirectory::new("ctf-r2b-direct-followup");
    let harness = TemporaryDirectory::new("ctf-r2b-direct-harness");
    initialize_project(root.path());
    let project = Project::discover(root.path()).unwrap();
    append_research_snapshot(&project, "r2b-direct-followup");
    fs::write(root.path().join("screen.png"), png_bytes(8, 8)).unwrap();
    let args = json!({"event_id":"direct-image-event", "occurred_at":"2026-09-12T00:00:00Z",
        "artifact_id":"artifact-direct-followup", "source_path":"screen.png",
        "target_snapshot_id":"snapshot-r2b-direct-followup", "toolchain_fingerprint":"r2b-fixture"});
    let primary = spawn_scripted_responses_server(vec![
        InitialSelection,
        Response(
            200,
            tool_calls_response(
                &[("direct-image-call", "view_image", args)],
                "direct-tool-response",
            ),
        ),
        ContinueSelection { usage: None },
        Response(
            200,
            text_response("direct image recovered", "direct-resumed"),
        ),
    ]);
    let config = write_direct_config(harness.path(), &primary.base_url);
    limit_config(&config, Some(2), false);
    let output = f2a_wait_child(
        spawn_f2a_chat(
            root.path(),
            &config,
            "inspect the image\n/resume\n/exit\n",
            &["--domain", "mobile"],
        ),
        "R2b direct follow-up",
    );
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    selector_candidates(&f2a_request_body(
        &primary.requests,
        "direct image initial selection",
    ));
    let _ = f2a_request_body(&primary.requests, "direct image call");
    let config = write_direct_config(harness.path(), &primary.base_url);
    limit_config(&config, Some(3), false);
    let output = f2a_wait_child(
        spawn_f2a_chat(
            root.path(),
            &config,
            "continue without rerunning the tool\n/exit\n",
            &["--domain", "mobile", "--resume", "latest"],
        ),
        "R2b direct selected recovery",
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    selector_candidates(&f2a_request_body(
        &primary.requests,
        "direct image resumed selection",
    ));
    let resumed = f2a_request_body(&primary.requests, "direct image resume");
    assert_eq!(
        resumed["input"]
            .to_string()
            .matches("data:image/png;base64,")
            .count(),
        1
    );
    assert!(resumed.get("previous_response_id").is_none());
    f2a_join_server(primary, "direct image Primary");
    let state = reduced_session(&project, &session_id(root.path()));
    assert_eq!(state.execution_receipt_count(), 1);
    assert_eq!(
        state
            .history()
            .items()
            .iter()
            .filter(|item| matches!(
                item,
                echidna::domain::model::ContentItem::ImageArtifact { .. }
            ))
            .count(),
        1
    );
    let events = session_events(root.path(), &session_id(root.path()));
    let first_input = events
        .iter()
        .find(|event| {
            event["event_type"] == "session.operator_turn_boundary_recorded"
                && event["payload"]["kind"] == "accepted"
        })
        .unwrap();
    assert_eq!(
        first_input["payload"]["inputs"].as_array().unwrap().len(),
        1,
        "the Tool follow-up is not a new operator input"
    );
    assert_audits_reconstruct(root.path(), &state);
}

#[test]
fn ctf_r2b_incremental_fallback_cannot_exceed_the_shared_limit() {
    let root = TemporaryDirectory::new("ctf-r2b-fallback-budget");
    let harness = TemporaryDirectory::new("ctf-r2b-harness");
    initialize_project(root.path());
    fs::write(root.path().join("target.txt"), "fixture").unwrap();
    let server = spawn_scripted_responses_server(vec![
        InitialSelection,
        Response(
            200,
            m4_long_flow_tool_response(
                "fallback-read",
                "read_file",
                json!({"path":"target.txt"}),
                "before-rejection",
            ),
        ),
        Response(
            400,
            "{\"error\":{\"code\":\"previous_response_not_found\",\"message\":\"fixture\"}}"
                .to_owned(),
        ),
    ]);
    let config = write_config(harness.path(), &server.base_url);
    limit_config(&config, Some(3), true);
    let output = f2a_wait_child(
        spawn_f2a_chat(root.path(), &config, "inspect target\n/exit\n", &[]),
        "R2b fallback cap",
    );
    assert!(
        !output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("per-Turn model-request limit of 3"));
    selector_candidates(&f2a_request_body(
        &server.requests,
        "fallback input selection",
    ));
    let _ = f2a_request_body(&server.requests, "fallback first");
    let second = f2a_request_body(&server.requests, "fallback incremental");
    assert_eq!(second["previous_response_id"], "before-rejection");
    f2a_join_server(server, "fallback denied");
    let state = reduced_session(
        &Project::discover(root.path()).unwrap(),
        &session_id(root.path()),
    );
    assert_eq!(state.request_audits().len(), 3);
    assert_eq!(state.execution_receipt_count(), 1);
}
