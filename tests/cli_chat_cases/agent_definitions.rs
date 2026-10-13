use super::*;

fn export_agent(root: &std::path::Path, name: &str, template: &str) -> std::path::PathBuf {
    let definition = root.join(name);
    let output = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .current_dir(root)
        .args([
            "agent",
            "init",
            definition.to_str().unwrap(),
            "--from",
            template,
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    definition
}

fn write_definition_config(
    root: &std::path::Path,
    base_url: &str,
    definition: &std::path::Path,
) -> std::path::PathBuf {
    let config = write_config(root, base_url);
    let configured = fs::read_to_string(&config).unwrap().replacen(
        "active_agent_profile = \"default\"",
        &format!(
            "active_agent_profile = \"default\"\nagent_definition = \"{}\"",
            definition.display()
        ),
        1,
    );
    fs::write(&config, configured).unwrap();
    config
}

fn user_texts(state: &SessionState) -> Vec<&str> {
    state
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
        .collect()
}

fn advertises(request: &Value, name: &str) -> bool {
    request["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tool| tool["name"] == name)
}

#[test]
fn generic_agent_omits_research_prompt_and_events() {
    let root = TemporaryDirectory::new("agent-definition-generic-loopback");
    initialize_project(root.path());
    let definition = export_agent(root.path(), "writer", "technical-writing");
    let server = spawn_responses_server(vec![(
        200,
        text_response("TECHNICAL_WRITER_REPLY", "generic-agent-reply"),
    )]);
    let config = write_definition_config(root.path(), &server.base_url, &definition);

    let output = run_chat(
        root.path(),
        &config,
        "Draft the release note.\n/exit\n",
        &[],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let request = f2a_request_body(&server.requests, "generic Agent request");
    f2a_join_server(server, "generic Agent");
    let instructions = request["instructions"].as_str().unwrap();
    assert!(instructions.contains("technical writer"), "{instructions}");
    assert!(instructions.contains(
        "Runtime binding facts: stable Agent ID `technical-writing`. Configured Memory scope: `agent`."
    ));
    assert!(instructions.contains(
        "Logical Memory path `core.md` is in the Agent Memory namespace for Agent ID `technical-writing`."
    ));
    assert!(!instructions.contains("ResearchPhase"), "{instructions}");
    assert!(!instructions.contains("TargetSnapshot"), "{instructions}");
    assert!(!advertises(&request, "research_get"));
    assert!(!advertises(&request, "research_update"));

    let events = session_events(root.path(), &session_id(root.path()));
    assert!(events.iter().all(|event| {
        !event["event_type"]
            .as_str()
            .is_some_and(|event_type| event_type.starts_with("research."))
    }));
    for event_type in [
        "session.domain_profile_selected",
        "session.research_phase_selected",
    ] {
        assert!(
            events.iter().all(|event| event["event_type"] != event_type),
            "generic Agent recorded {event_type}"
        );
    }
}

#[test]
fn memoryless_agent_reports_no_logical_memory_namespace() {
    let root = TemporaryDirectory::new("agent-definition-memoryless-loopback");
    initialize_project(root.path());
    let definition = export_agent(root.path(), "memoryless", "technical-writing");
    let manifest = definition.join("agent.toml");
    let content = fs::read_to_string(&manifest)
        .unwrap()
        .replacen("memory = \"agent\"", "memory = \"none\"", 1)
        .replacen("memory_read = \"direct\"", "memory_read = \"hidden\"", 1)
        .replacen("memory_write = \"direct\"", "memory_write = \"hidden\"", 1);
    fs::write(&manifest, content).unwrap();
    let server = spawn_responses_server(vec![(
        200,
        text_response("MEMORYLESS_WRITER_REPLY", "memoryless-agent-reply"),
    )]);
    let config = write_config(root.path(), &server.base_url);

    let output = run_chat(
        root.path(),
        &config,
        "Draft without Memory.\n/exit\n",
        &["--agent", definition.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let request = f2a_request_body(&server.requests, "memoryless Agent request");
    f2a_join_server(server, "memoryless Agent");
    let instructions = request["instructions"].as_str().unwrap();
    assert!(instructions.contains(
        "Runtime binding facts: stable Agent ID `technical-writing`. Configured Memory scope: `none`."
    ));
    assert!(
        instructions
            .contains("Logical Memory path `core.md` is unavailable because Memory is disabled.")
    );
    assert!(!advertises(&request, "memory_read"));
    assert!(!advertises(&request, "memory_write"));
}

#[test]
fn inspect_renders_bound_core_persona_work_and_twelve_methods_in_declared_order() {
    let root = TemporaryDirectory::new("agent-definition-instruction-order");
    let definition = export_agent(root.path(), "ordered-writer", "technical-writing");
    let methods = (0..12)
        .map(|index| {
            let path = definition.join(format!("methods/{index:02}.md"));
            fs::write(&path, format!("ORDERED_METHOD_{index:02}\n")).unwrap();
            format!("\"methods/{index:02}.md\"")
        })
        .collect::<Vec<_>>()
        .join(", ");
    let manifest = definition.join("agent.toml");
    fs::write(
        &manifest,
        fs::read_to_string(&manifest)
            .unwrap()
            .replacen("id = \"technical-writing\"", "id = \"a\"", 1)
            .replacen(
                "method_files = [\"methods/clarity.md\"]",
                &format!("method_files = [{methods}]"),
                1,
            ),
    )
    .unwrap();
    let config = write_config(root.path(), "http://127.0.0.1:1/v1");
    let output = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .current_dir(root.path())
        .args([
            "--config",
            config.to_str().unwrap(),
            "agent",
            "inspect",
            definition.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let instructions = serde_json::from_slice::<Value>(&output.stdout).unwrap()["instructions"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut expected = vec![
        "Follow the current operator task".to_owned(),
        "Runtime binding facts: stable Agent ID `a`. Configured Memory scope: `agent`.".to_owned(),
        "You are a careful technical writer".to_owned(),
        "Work on the operator's authorized technical-writing task".to_owned(),
    ];
    expected.extend((0..12).map(|index| format!("ORDERED_METHOD_{index:02}")));
    let mut previous = 0;
    for marker in &expected {
        let index = instructions[previous..]
            .find(marker.as_str())
            .map(|offset| previous + offset)
            .unwrap_or_else(|| panic!("missing `{marker}` in {instructions}"));
        assert!(index >= previous, "{marker} rendered out of order");
        previous = index + marker.len();
    }
}

#[test]
fn research_definition_keeps_existing_domain_and_phase_tool_overlays() {
    for (name, args, expected_direct, expected_deferred) in [
        (
            "source-mapping",
            Vec::new(),
            vec!["run_command"],
            vec!["write_file", "view_image", "http_request"],
        ),
        (
            "mobile-experiment",
            vec!["--domain", "mobile", "--phase", "experiment"],
            vec!["write_file", "view_image", "http_request"],
            vec!["read_output"],
        ),
    ] {
        let root = TemporaryDirectory::new(&format!("agent-definition-{name}"));
        initialize_project(root.path());
        let definition = export_agent(root.path(), "research", "builtin-research");
        let server =
            spawn_responses_server(vec![(200, text_response("RESEARCH_OVERLAY_REPLY", name))]);
        let config = write_config(root.path(), &server.base_url);
        let mut arguments = vec!["--agent", definition.to_str().unwrap()];
        arguments.extend(args);
        let output = run_chat(
            root.path(),
            &config,
            "Inspect the fixture.\n/exit\n",
            &arguments,
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let request = f2a_request_body(&server.requests, name);
        f2a_join_server(server, name);
        let instructions = request["instructions"].as_str().unwrap();
        assert!(instructions.contains(
            "Runtime binding facts: stable Agent ID `research`. Configured Memory scope: `project`."
        ));
        assert!(
            instructions.contains(
                "Logical Memory path `core.md` is in the shared project Memory namespace."
            )
        );
        for tool in expected_direct {
            assert!(
                advertises(&request, tool),
                "{name} did not advertise {tool}"
            );
        }
        for tool in expected_deferred {
            assert!(
                !advertises(&request, tool),
                "{name} unexpectedly advertised {tool}"
            );
        }
    }
}

#[test]
fn resume_uses_the_pinned_binding_after_its_definition_source_is_deleted() {
    let root = TemporaryDirectory::new("agent-definition-resume-pinned");
    initialize_project(root.path());
    let definition = export_agent(root.path(), "writer", "technical-writing");
    let initial = spawn_responses_server(vec![(
        200,
        text_response("INITIAL_WRITER_REPLY", "pinned-initial"),
    )]);
    let config = write_config(root.path(), &initial.base_url);
    let output = run_chat(
        root.path(),
        &config,
        "INITIAL_WRITER_TASK\n/exit\n",
        &["--agent", definition.to_str().unwrap()],
    );
    assert!(output.status.success());
    f2a_request_body(&initial.requests, "initial writer request");
    f2a_join_server(initial, "initial writer");
    let session = session_id(root.path());
    fs::remove_dir_all(&definition).unwrap();

    let resumed = spawn_responses_server(vec![(
        200,
        text_response("RESUMED_WRITER_REPLY", "pinned-resume"),
    )]);
    let config = write_config(root.path(), &resumed.base_url);
    let output = run_chat(
        root.path(),
        &config,
        "RESUMED_WRITER_TASK\n/exit\n",
        &["--resume", &session],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let request = f2a_request_body(&resumed.requests, "pinned resume request");
    f2a_join_server(resumed, "pinned resume");
    assert!(
        request["instructions"]
            .as_str()
            .unwrap()
            .contains("technical writer")
    );
    assert!(
        request["instructions"]
            .as_str()
            .unwrap()
            .contains("Runtime binding facts: stable Agent ID `technical-writing`. Configured Memory scope: `agent`.")
    );
    let project = Project::discover(root.path()).unwrap();
    assert_eq!(
        user_texts(&reduced_session(&project, &session)),
        ["INITIAL_WRITER_TASK", "RESUMED_WRITER_TASK"]
    );
}

#[test]
fn persona_reload_keeps_history_and_forces_a_fresh_full_provider_request() {
    let root = TemporaryDirectory::new("agent-definition-reload");
    initialize_project(root.path());
    let definition = export_agent(root.path(), "writer", "technical-writing");
    let persona = definition.join("persona.md");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (sender, requests) = mpsc::channel();
    let initial_response = format!(
        "data: {}\n\n{}",
        json!({"type":"response.output_item.done","item":{
            "type":"reasoning",
            "content":[{"type":"reasoning_text","text":"RELOAD_PLAINTEXT_REASONING"}]
        }}),
        text_response("INITIAL_PERSONA_REPLY", "reload-initial")
    );
    let server = thread::spawn(move || {
        for (index, response) in [
            initial_response,
            text_response("RELOADED_PERSONA_REPLY", "reload-second"),
        ]
        .into_iter()
        .enumerate()
        {
            let (mut socket, _) = listener.accept().unwrap();
            socket.set_read_timeout(Some(F2A_WAIT)).unwrap();
            sender.send(read_http_request(&mut socket)).unwrap();
            if index == 0 {
                fs::write(
                    &persona,
                    "Technical writing persona after explicit reload.\n",
                )
                .unwrap();
            }
            f3_context::write_sse_response(&mut socket, &response);
        }
    });
    let config = write_config(root.path(), &format!("http://{address}/v1"));
    let output = run_chat(
        root.path(),
        &config,
        "INITIAL_RELOAD_TASK\n/agent reload\nRELOADED_TASK\n/exit\n",
        &["--agent", definition.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let first = f2a_request_body(&requests, "initial reload request");
    let second = f2a_request_body(&requests, "reloaded request");
    server.join().unwrap();
    assert!(
        first["instructions"]
            .as_str()
            .unwrap()
            .contains("careful technical writer")
    );
    assert!(
        second["instructions"]
            .as_str()
            .unwrap()
            .contains("after explicit reload")
    );
    assert!(second["previous_response_id"].is_null(), "{second}");
    assert_eq!(
        second.to_string().matches("INITIAL_RELOAD_TASK").count(),
        1,
        "the new Full request retains one History item, not a replayed input"
    );
    assert!(
        !second.to_string().contains("RELOAD_PLAINTEXT_REASONING"),
        "a fresh ProviderSession must not replay completed reasoning"
    );

    let session = session_id(root.path());
    let project = Project::discover(root.path()).unwrap();
    let state = reduced_session(&project, &session);
    assert_eq!(user_texts(&state), ["INITIAL_RELOAD_TASK", "RELOADED_TASK"]);
    let events = session_events(root.path(), &session);
    assert_eq!(
        events
            .iter()
            .filter(|event| event["event_type"] == "session.agent_definition_applied")
            .count(),
        1
    );
}

#[test]
fn agent_use_starts_a_new_session_without_changing_the_old_session() {
    let root = TemporaryDirectory::new("agent-definition-use");
    initialize_project(root.path());
    let definition = export_agent(root.path(), "writer", "technical-writing");
    let server = spawn_responses_server(vec![
        (200, text_response("OLD_SESSION_REPLY", "use-old")),
        (200, text_response("NEW_SESSION_REPLY", "use-new")),
    ]);
    let config = write_config(root.path(), &server.base_url);
    let output = run_chat(
        root.path(),
        &config,
        &format!(
            "OLD_SESSION_TASK\n/agent use {}\nNEW_SESSION_TASK\n/exit\n",
            definition.display()
        ),
        &[],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let old_request = f2a_request_body(&server.requests, "old Session request");
    let new_request = f2a_request_body(&server.requests, "new Session request");
    f2a_join_server(server, "agent use");
    let old_instructions = old_request["instructions"].as_str().unwrap();
    assert!(old_instructions.contains(
        "Runtime binding facts: stable Agent ID `research`. Configured Memory scope: `project`."
    ));
    assert!(!old_instructions.contains("Agent ID `technical-writing`"));
    let new_instructions = new_request["instructions"].as_str().unwrap();
    assert!(new_instructions.contains(
        "Runtime binding facts: stable Agent ID `technical-writing`. Configured Memory scope: `agent`."
    ));
    assert!(!new_instructions.contains("Agent ID `research`"));

    let project = Project::discover(root.path()).unwrap();
    let sessions = session_ids(root.path());
    assert_eq!(sessions.len(), 2);
    let states = sessions
        .iter()
        .map(|session| reduced_session(&project, session))
        .collect::<Vec<_>>();
    assert!(
        states
            .iter()
            .any(|state| user_texts(state) == ["OLD_SESSION_TASK"])
    );
    assert!(
        states
            .iter()
            .any(|state| user_texts(state) == ["NEW_SESSION_TASK"])
    );
}

#[test]
fn invalid_definition_sends_no_model_request_or_session_event() {
    let root = TemporaryDirectory::new("agent-definition-invalid-loopback");
    initialize_project(root.path());
    let definition = export_agent(root.path(), "invalid", "technical-writing");
    let manifest = definition.join("agent.toml");
    fs::write(
        &manifest,
        format!(
            "{}\nunknown_tool = \"direct\"\n",
            fs::read_to_string(&manifest).unwrap()
        ),
    )
    .unwrap();
    let server = spawn_f2a_pending_vision_server();
    let config = write_config(root.path(), &server.base_url);
    let output = run_chat(
        root.path(),
        &config,
        "THIS_MUST_NOT_REACH_A_PROVIDER\n",
        &["--agent", definition.to_str().unwrap()],
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown Tool"));
    assert!(server.shutdown().is_empty());
    let sessions = root.path().join(".cyber-agent/sessions");
    assert_eq!(fs::read_dir(sessions).unwrap().count(), 0);
}

#[test]
fn pending_image_and_running_job_reject_agent_switching() {
    let image_root = TemporaryDirectory::new("agent-definition-pending-image");
    initialize_project(image_root.path());
    let first = export_agent(image_root.path(), "writer-one", "technical-writing");
    let second = export_agent(image_root.path(), "writer-two", "technical-writing");
    fs::write(image_root.path().join("screen.png"), png_bytes(4, 4)).unwrap();
    let image_server = spawn_f2a_pending_vision_server();
    let image_config = write_config(image_root.path(), &image_server.base_url);
    let output = run_chat(
        image_root.path(),
        &image_config,
        &format!(
            "/image screen.png\n/agent use {}\n/exit\n",
            second.display()
        ),
        &["--agent", first.to_str().unwrap()],
    );
    assert!(output.status.success());
    assert!(image_server.shutdown().is_empty());
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("Agent changes require no queued image")
    );
    assert_eq!(session_ids(image_root.path()).len(), 1);

    let job_root = TemporaryDirectory::new("agent-definition-running-job");
    initialize_project(job_root.path());
    let next = export_agent(job_root.path(), "writer", "technical-writing");
    let server = spawn_responses_server(vec![
        (
            200,
            m4_long_flow_tool_response(
                "running-job",
                "start_job",
                json!({"argv": ["sh", "-c", "sleep 60"]}),
                "running-job-started",
            ),
        ),
        (200, text_response("JOB_STARTED", "running-job-final")),
    ]);
    let config = write_config(job_root.path(), &server.base_url);
    let output = run_chat(
        job_root.path(),
        &config,
        &format!("Start a Job.\n/agent use {}\n/exit\n", next.display()),
        &["--phase", "experiment"],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    f2a_request_body(&server.requests, "running Job start");
    f2a_request_body(&server.requests, "running Job completion");
    f2a_join_server(server, "running Job");
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("Agent changes require all running or stopping Jobs/PTYs to finish")
    );
    assert_eq!(session_ids(job_root.path()).len(), 1);
}
