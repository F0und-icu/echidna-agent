use super::task_context::{ScriptedResponse, selector_candidates, spawn_scripted_responses_server};
use super::*;

#[test]
fn resume_before_the_first_task_preserves_external_research_defaults() {
    let root = TemporaryDirectory::new("agent-definition-empty-resume");
    initialize_project(root.path());
    let definition = root.path().join("researcher");
    let exported = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .args([
            "agent",
            "init",
            definition.to_str().unwrap(),
            "--from",
            "builtin-research",
        ])
        .output()
        .unwrap();
    assert!(exported.status.success());
    let manifest = definition.join("agent.toml");
    fs::write(
        &manifest,
        fs::read_to_string(&manifest)
            .unwrap()
            .replace("domain = \"source\"", "domain = \"mobile\"")
            .replace("phase = \"mapping\"", "phase = \"experiment\""),
    )
    .unwrap();
    let server = spawn_scripted_responses_server(vec![
        ScriptedResponse::InitialSelection,
        ScriptedResponse::Response(200, text_response("CUSTOM_DEFAULTS", "defaults-response")),
    ]);
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let initial = run_chat(
        root.path(),
        &config,
        "/exit\n",
        &["--agent", definition.to_str().unwrap()],
    );
    assert!(
        initial.status.success(),
        "{}",
        String::from_utf8_lossy(&initial.stderr)
    );
    let session = session_id(root.path());
    let resumed = run_chat(
        root.path(),
        &config,
        "First real task.\n/exit\n",
        &["--resume", &session],
    );
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    selector_candidates(&f2a_request_body(
        &server.requests,
        "custom default initial selection",
    ));
    let request = f2a_request_body(&server.requests, "custom default resume");
    f2a_join_server(server, "custom default resume");
    assert!(
        request["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["name"] == "start_job")
    );
    let project = Project::discover(root.path()).unwrap();
    let state = reduced_session(&project, &session);
    assert_eq!(
        state.effective_domain_profile(),
        echidna::domain::domain_profile::DomainProfile::Mobile
    );
    assert_eq!(
        state.effective_research_phase(),
        echidna::domain::research_control::ResearchPhase::Experiment
    );
}

#[test]
fn oversized_reload_preserves_binding_and_next_turn_without_a_transition_or_request() {
    let root = TemporaryDirectory::new("agent-definition-budget-admission");
    initialize_project(root.path());
    let original = root.path().join("original");
    let candidate = root.path().join("candidate");
    for destination in [&original, &candidate] {
        let output = Command::new(env!("CARGO_BIN_EXE_echidna"))
            .args([
                "agent",
                "init",
                destination.to_str().unwrap(),
                "--id",
                "budget-writer",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    fs::write(original.join("persona.md"), "ADMITTED_WRITER_PERSONA\n").unwrap();
    // Below the loader's resource bound, above this fixture model's request budget.
    for file in ["persona.md", "work.md"] {
        fs::write(candidate.join(file), "OVERSIZED_CANDIDATE ".repeat(3_200)).unwrap();
    }
    let server = spawn_scripted_responses_server(vec![
        ScriptedResponse::InitialSelection,
        ScriptedResponse::Response(
            200,
            text_response("BEFORE_RELOAD", "before-rejected-reload"),
        ),
        ScriptedResponse::NewSelection,
        ScriptedResponse::Response(
            200,
            text_response("AFTER_REJECTION", "after-rejected-reload"),
        ),
    ]);
    let config = write_config(root.path(), &server.base_url);
    let output = run_chat(
        root.path(),
        &config,
        &format!(
            "FIRST_TASK\n/agent reload {}\nSECOND_TASK\n/exit\n",
            candidate.display()
        ),
        &["--agent", original.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Agent reload rejected"), "{stdout}");
    assert!(stdout.contains("budget"), "{stdout}");
    for index in 0..4 {
        let request = f2a_request_body(&server.requests, &format!("admission request {index}"));
        if index == 0 {
            assert_eq!(selector_candidates(&request)["input"]["text"], "FIRST_TASK");
            continue;
        }
        if index == 2 {
            assert_eq!(
                selector_candidates(&request)["input"]["text"],
                "SECOND_TASK"
            );
            continue;
        }
        let instructions = request["instructions"].as_str().unwrap();
        assert!(instructions.contains("ADMITTED_WRITER_PERSONA"));
        assert!(!instructions.contains("OVERSIZED_CANDIDATE"));
    }
    assert!(server.requests.try_recv().is_err());
    f2a_join_server(server, "rejected definition admission");
    let session = session_id(root.path());
    let events = session_events(root.path(), &session);
    assert!(
        !events
            .iter()
            .any(|event| event["event_type"] == "session.agent_definition_applied")
    );
    let shown = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .current_dir(root.path())
        .args(["session", "show", &session])
        .output()
        .unwrap();
    assert!(shown.status.success());
    let shown: Value = serde_json::from_slice(&shown.stdout).unwrap();
    assert_eq!(shown["agent_id"], "budget-writer");
    assert!(shown["domain"].is_null());
    assert!(shown["phase"].is_null());
    assert_eq!(shown["requests"], 4);
}
