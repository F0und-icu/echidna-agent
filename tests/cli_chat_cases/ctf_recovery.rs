use super::*;

fn limit_config(config: &std::path::Path, limit: Option<usize>, incremental: bool) {
    let mut text = fs::read_to_string(config)
        .unwrap()
        .replace("context_window = 272000", "context_window = 1048576");
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
        let server = spawn_responses_server(vec![
            (
                200,
                m4_long_flow_tool_response(
                    "r2b-read",
                    "read_file",
                    json!({"path":"target.txt"}),
                    "before-stop",
                ),
            ),
            (
                200,
                text_response("explicit recovery completed", "after-resume"),
            ),
        ]);
        let config = write_config(harness.path(), &server.base_url);
        limit_config(&config, Some(1), incremental);
        let output = f2a_wait_child(
            spawn_f2a_chat(
                root.path(),
                &config,
                "original task\nnot accepted task\n/context\n/resume\nnew instruction\n/exit\n",
                &[],
            ),
            "R2b explicit recovery",
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{stdout}\n{stderr}");
        assert!(stderr.contains("per-Turn model-request limit of 1"));
        assert!(stdout.contains("Task input not accepted or saved"));
        assert!(stdout.contains("admitted=1 primary=1 vision=0 remaining=0 limit=1"));
        assert!(stdout.contains("explicit recovery completed"));
        let _first = f2a_request_body(&server.requests, "R2b first");
        let second = f2a_request_body(&server.requests, "R2b resumed");
        assert!(second.get("previous_response_id").is_none());
        assert!(json_contains_type(&second["input"], "function_call_output"));
        assert!(!second.to_string().contains("not accepted task"));
        f2a_join_server(server, "R2b resume");
        let project = Project::discover(root.path()).unwrap();
        let state = reduced_session(&project, &session_id(root.path()));
        assert_eq!(state.request_audits().len(), 2);
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
fn ctf_r2b_default32_and_explicit33_have_no_hidden_primary_cap() {
    for configured in [Some(2), None, Some(33)] {
        let count = configured.unwrap_or(32);
        let root = TemporaryDirectory::new("ctf-r2b-limit");
        let harness = TemporaryDirectory::new("ctf-r2b-harness");
        initialize_project(root.path());
        fs::write(root.path().join("target.txt"), "fixture").unwrap();
        let responses = (0..count)
            .map(|index| {
                (
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
            })
            .collect();
        let server = spawn_responses_server(responses);
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
        for _ in 0..count {
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
        let mut responses = vec![response; failed_attempts];
        responses.push((200, text_response("recovered", "new-response")));
        let server = spawn_responses_server(responses);
        let config = write_config(harness.path(), &server.base_url);
        limit_config(&config, Some(2), true);
        let output = f2a_wait_child(spawn_f2a_chat(root.path(), &config,
            "first input\n/resume\nsecond input\n/exit\n", &[]), "R2b provider failure");
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        assert!(String::from_utf8_lossy(&output.stderr).contains("[turn stopped]"));
        let first = f2a_request_body(&server.requests, "R2b failed");
        for _ in 1..failed_attempts {
            assert_eq!(first, f2a_request_body(&server.requests, "R2b failed retry"));
        }
        let second = f2a_request_body(&server.requests, "R2b new provider");
        assert!(second.get("previous_response_id").is_none());
        assert!(!second.to_string().contains("uncommitted fragment"));
        assert!(server.requests.try_recv().is_err());
        f2a_join_server(server, "R2b provider");
        let state = reduced_session(&Project::discover(root.path()).unwrap(), &session_id(root.path()));
        assert_eq!(state.request_audits().len(), failed_attempts + 1);
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
    let primary = spawn_responses_server(vec![(
        200,
        text_response("explicit image recovery", "primary-new"),
    )]);
    let config = write_dual_config(harness.path(), &primary.base_url, &vision.base_url);
    limit_config(&config, Some(1), false);
    let output = f2a_wait_child(
        spawn_f2a_chat(
            root.path(),
            &config,
            "/image screen.png\ninspect image\n/resume\ncontinue with retained observation\n/exit\n",
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
    let wire = f2a_request_body(&primary.requests, "resumed Primary");
    assert!(wire.get("previous_response_id").is_none());
    assert!(wire.to_string().contains("initial bounded observation"));
    f2a_join_server(vision, "initial Vision");
    f2a_join_server(primary, "resumed Primary");
    let state = reduced_session(&project, &session_id(root.path()));
    assert_eq!(state.request_audits().len(), 2);
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
    let primary =
        spawn_responses_server(vec![(200, text_response("text recovery", "primary-new"))]);
    let config = write_dual_config(harness.path(), &primary.base_url, &vision.base_url);
    limit_config(&config, Some(2), false);
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
    let wire = f2a_request_body(&primary.requests, "new text");
    assert!(!wire.to_string().contains("FAILED_IMAGE_TASK"));
    assert!(!json_contains_type(&wire["input"], "input_image"));
    f2a_join_server(vision, "failed Vision");
    f2a_join_server(primary, "new text");
    let state = reduced_session(&project, &session_id(root.path()));
    assert_eq!(state.request_audits().len(), 2);
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
    let primary = spawn_responses_server(vec![
        (200, tool_calls_response(&calls, "image-tool-response")),
        (
            200,
            text_response("resumed after image budget", "image-resumed"),
        ),
    ]);
    let vision = spawn_responses_server(vec![(
        200,
        vision_response("checkpointed first observation", "one-vision"),
    )]);
    let config = write_dual_config(harness.path(), &primary.base_url, &vision.base_url);
    limit_config(&config, Some(2), false);
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
    assert!(stdout.contains("admitted=2 primary=1 vision=1 remaining=0 limit=2"));
    let _ = f2a_request_body(&primary.requests, "image calls");
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
    assert_eq!(state.request_audits().len(), 3);
    assert_eq!(state.execution_receipt_count(), 2);
    assert_eq!(state.vision_observations().len(), 1);
    assert_eq!(state.history().ensure_all_tool_calls_paired(), Ok(()));
    assert_audits_reconstruct(root.path(), &state);
}

#[test]
fn ctf_r2b_incremental_fallback_cannot_exceed_the_shared_limit() {
    let root = TemporaryDirectory::new("ctf-r2b-fallback-budget");
    let harness = TemporaryDirectory::new("ctf-r2b-harness");
    initialize_project(root.path());
    fs::write(root.path().join("target.txt"), "fixture").unwrap();
    let server = spawn_responses_server(vec![
        (
            200,
            m4_long_flow_tool_response(
                "fallback-read",
                "read_file",
                json!({"path":"target.txt"}),
                "before-rejection",
            ),
        ),
        (
            400,
            "{\"error\":{\"code\":\"previous_response_not_found\",\"message\":\"fixture\"}}"
                .to_owned(),
        ),
    ]);
    let config = write_config(harness.path(), &server.base_url);
    limit_config(&config, Some(2), true);
    let output = f2a_wait_child(
        spawn_f2a_chat(root.path(), &config, "inspect target\n/exit\n", &[]),
        "R2b fallback cap",
    );
    assert!(
        !output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("per-Turn model-request limit of 2"));
    let _ = f2a_request_body(&server.requests, "fallback first");
    let second = f2a_request_body(&server.requests, "fallback incremental");
    assert_eq!(second["previous_response_id"], "before-rejection");
    f2a_join_server(server, "fallback denied");
    let state = reduced_session(
        &Project::discover(root.path()).unwrap(),
        &session_id(root.path()),
    );
    assert_eq!(state.request_audits().len(), 2);
    assert_eq!(state.execution_receipt_count(), 1);
}
