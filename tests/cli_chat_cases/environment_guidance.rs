use super::task_context::{
    ScriptedResponse::{InitialSelection, Response},
    selector_candidates, spawn_scripted_responses_server,
};
use super::*;

#[test]
fn environment_prompt_revision_resumes_known_generations_without_rewriting_selection_events() {
    use echidna::domain::context::{ProviderSessionId, ProviderSessionRef, ProviderSessionRole};
    use echidna::domain::research_control::ResearchPhase;
    for (phase, generation) in [
        (ResearchPhase::Mapping, 1),
        (ResearchPhase::Mapping, 2),
        (ResearchPhase::Hypothesis, 1),
        (ResearchPhase::Experiment, 1),
        (ResearchPhase::Verification, 1),
        (ResearchPhase::EvidenceCuration, 1),
    ] {
        let root = TemporaryDirectory::new("environment-legacy-selection");
        let harness = TemporaryDirectory::new("environment-legacy-provider");
        initialize_project(root.path());
        let project = Project::discover(root.path()).unwrap();
        let session = StreamId::new("environment-generation-one");
        let stream = ProjectEventStream::Session(session.clone()).stream_id();
        {
            let mut writer = project.open_writer().unwrap();
            let mut store = writer.open_session(session.clone(), None).unwrap();
            let events = [
                SessionEvent::Started {
                    primary_provider_session: ProviderSessionRef {
                        id: ProviderSessionId::new("environment-old-primary"),
                        role: ProviderSessionRole::Primary,
                        generation: 1,
                    },
                },
                SessionEvent::DomainProfileSelected {
                    selection: serde_json::from_value(json!({
                        "profile":"binary", "profile_generation":1,
                        "source":"operator", "reason_code":"OPERATOR_SELECTED"
                    }))
                    .unwrap(),
                },
                SessionEvent::ResearchPhaseSelected {
                    selection: serde_json::from_value(json!({
                        "phase":phase, "phase_generation":generation,
                        "source": if phase == ResearchPhase::Mapping { "default" } else { "operator" },
                        "reason_code": if phase == ResearchPhase::Mapping {
                            "NEW_SESSION_DEFAULT_MAPPING"
                        } else { "OPERATOR_SELECTED" }
                    }))
                    .unwrap(),
                },
            ];
            for (index, event) in events.into_iter().enumerate() {
                let envelope = event
                    .into_envelope(
                        EventId::new(format!("old-selection-{index}")),
                        stream.clone(),
                        store.next_sequence().unwrap(),
                        OccurredAt::new("2026-09-12T00:00:00Z"),
                    )
                    .unwrap();
                store.append_event(&envelope).unwrap();
            }
        }
        let before = session_events(root.path(), session.as_str());
        let server = spawn_scripted_responses_server(vec![
            InitialSelection,
            Response(200, text_response("resumed", "environment-resumed")),
        ]);
        let config = write_config(harness.path(), &server.base_url);
        let output = f2a_wait_child(
            spawn_f2a_chat(
                root.path(),
                &config,
                "Only answer what phase is selected.\n/exit\n",
                &["--resume", session.as_str()],
            ),
            "legacy prompt resume",
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let selector = f2a_request_body(&server.requests, "legacy prompt initial selection");
        assert_eq!(
            selector_candidates(&selector)["input"]["text"],
            "Only answer what phase is selected."
        );
        let request = f2a_request_body(&server.requests, "legacy prompt request");
        assert!(request.get("previous_response_id").is_none());
        assert!(
            request["instructions"]
                .as_str()
                .unwrap()
                .contains("Binary domain profile v3")
        );
        assert!(
            request["instructions"]
                .as_str()
                .unwrap()
                .contains(phase.prompt())
        );
        f2a_join_server(server, "legacy prompt resume");
        let after = session_events(root.path(), session.as_str());
        assert_eq!(&after[..before.len()], before.as_slice());
        assert_audits_reconstruct(root.path(), &reduced_session(&project, session.as_str()));
    }
}

#[test]
fn environment_guidance_delivers_actual_dialect_observations_and_versioned_prompts() {
    let root = TemporaryDirectory::new("environment-guidance");
    let harness = TemporaryDirectory::new("environment-guidance-provider");
    initialize_project(root.path());
    let dialects = [
        ("gnu", "GNU nm 2.43", "--defined-only"),
        ("llvm", "LLVM version 19.1", "--defined-only"),
        ("bsd", "fixture BSD nm 1.0", "-U"),
    ];
    for (name, identity, flag) in dialects {
        let path = root.path().join(name);
        fs::write(&path, format!(
            "#!/bin/sh\nprintf x >> {name}-count\ncase \"$1\" in\n--version) printf '%s\\n' '{identity}';;\n{flag}) printf '%s\\n' '{name}-symbols';;\n*) exit 64;;\nesac\n"
        )).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    // A real symlink alias, not a hardcoded macOS path assumption.
    let alias = harness.path().join("project-alias");
    std::os::unix::fs::symlink(root.path(), &alias).unwrap();
    let probes = dialects
        .iter()
        .map(|(name, _, _)| {
            json!({
                "argv":[root.path().join(name), "--version"]
            })
        })
        .chain(std::iter::once(
            json!({"argv":["/bin/pwd", "-P"], "cwd":alias}),
        ))
        .collect::<Vec<_>>();
    let actions = dialects
        .iter()
        .map(|(name, _, flag)| {
            json!({
                "argv":[root.path().join(name), flag]
            })
        })
        .collect::<Vec<_>>();
    let server = spawn_scripted_responses_server(vec![
        InitialSelection,
        Response(
            200,
            m4_long_flow_tool_response(
                "environment-probes",
                "run_commands",
                json!({"mode":"parallel", "commands":probes}),
                "probe-response",
            ),
        ),
        Response(
            200,
            m4_long_flow_tool_response(
                "environment-actions",
                "run_commands",
                json!({"mode":"serial", "commands":actions}),
                "action-response",
            ),
        ),
        Response(
            200,
            text_response("observed dialects retained", "environment-done"),
        ),
    ]);
    let config = write_config(harness.path(), &server.base_url);
    let output = f2a_wait_child(
        spawn_f2a_chat(
            &alias,
            &config,
            "inspect synthetic tool identities\n/exit\n",
            &["--domain", "binary"],
        ),
        "environment guidance",
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let selector = f2a_request_body(&server.requests, "environment initial selection");
    assert_eq!(
        selector_candidates(&selector)["input"]["text"],
        "inspect synthetic tool identities"
    );
    let first = f2a_request_body(&server.requests, "environment initial");
    assert!(
        first["instructions"]
            .as_str()
            .unwrap()
            .contains("GNU/BSD/LLVM")
    );
    assert!(
        first["instructions"]
            .as_str()
            .unwrap()
            .contains("not repeat a full probe every Turn")
    );
    let descriptions = first["tools"].to_string();
    assert!(descriptions.contains("probe_failed includes ambiguous spawn failures"));
    assert!(descriptions.contains("symlink aliases"));
    let second = f2a_request_body(&server.requests, "environment observations");
    let result = second["input"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["type"] == "function_call_output")
        .unwrap();
    let result: Value = serde_json::from_str(result["output"].as_str().unwrap()).unwrap();
    for (index, (_, identity, _)) in dialects.iter().enumerate() {
        assert_eq!(
            result["output"]["commands"][index]["result"]["stdout"],
            format!("{identity}\n")
        );
    }
    assert_eq!(
        result["output"]["commands"][3]["result"]["stdout"],
        format!("{}\n", fs::canonicalize(root.path()).unwrap().display())
    );
    let third = f2a_request_body(&server.requests, "environment execution results");
    for (name, _, _) in dialects {
        assert!(third.to_string().contains(&format!("{name}-symbols")));
        assert_eq!(
            fs::read(root.path().join(format!("{name}-count"))).unwrap(),
            b"xx"
        );
    }
    f2a_join_server(server, "environment guidance");
    let events = session_events(root.path(), &session_id(root.path()));
    let projection = echidna::app::session::SessionProjection::replay(
        events
            .into_iter()
            .map(|event| serde_json::from_value(event).unwrap()),
    )
    .unwrap();
    let state = projection.state();
    assert_eq!(state.request_audits().len(), 4);
    assert_audits_reconstruct(root.path(), state);
    let definition = state.agent_definition_binding().unwrap().definition();
    let research = definition.research().unwrap();
    for audit in &state.request_audits()[1..] {
        let audit = serde_json::to_string(audit).unwrap();
        assert!(audit.contains(&format!(
            "prompt:agent:{}:domain:binary:{}",
            definition.id().as_str(),
            research
                .domain_prompt(echidna::domain::domain_profile::DomainProfile::Binary)
                .digest()
                .as_str()
        )));
        assert!(audit.contains(&format!(
            "prompt:agent:{}:phase:mapping:{}",
            definition.id().as_str(),
            research
                .phase_prompt(echidna::domain::research_control::ResearchPhase::Mapping)
                .digest()
                .as_str()
        )));
    }
}
