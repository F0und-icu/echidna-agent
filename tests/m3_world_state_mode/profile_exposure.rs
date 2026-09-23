use super::*;

#[test]
fn vision_route_matrix_is_capability_only_and_forced_modes_validate() {
    let cases = [
        (
            VisionMode::Auto,
            true,
            None,
            VisionRoute::PrimaryDirect,
            Some(VisionRouteReason::PrimaryImageCapable),
        ),
        (
            VisionMode::Auto,
            false,
            Some(true),
            VisionRoute::SecondaryTool,
            Some(VisionRouteReason::PrimaryImageUnsupportedSecondarySelected),
        ),
        (
            VisionMode::Auto,
            false,
            Some(false),
            VisionRoute::Rejected,
            Some(VisionRouteReason::NoImageCapableProvider),
        ),
        (
            VisionMode::PrimaryDirect,
            true,
            Some(false),
            VisionRoute::PrimaryDirect,
            Some(VisionRouteReason::ForcedPrimaryDirect),
        ),
        (
            VisionMode::SecondaryTool,
            false,
            Some(true),
            VisionRoute::SecondaryTool,
            Some(VisionRouteReason::ForcedSecondaryTool),
        ),
        (
            VisionMode::Disabled,
            true,
            Some(true),
            VisionRoute::Rejected,
            Some(VisionRouteReason::VisionDisabled),
        ),
    ];
    for (mode, primary_image, vision_image, route, reason) in cases {
        let primary = capabilities(primary_image);
        let vision = vision_image.map(capabilities);
        assert_eq!(
            resolve_vision_route(mode, &primary, vision.as_ref(), true),
            cyber_agent::domain::profile::VisionRouteDecision { route, reason }
        );
        assert_eq!(
            resolve_vision_route(mode, &primary, vision.as_ref(), false).route,
            VisionRoute::NoImage
        );
    }

    assert_eq!(
        AgentModelProfile::new(
            ModelProfile::new("direct", 1),
            provider_profile("primary", 1, false),
            Some(provider_profile("vision", 1, true)),
            VisionMode::PrimaryDirect,
        ),
        Err(AgentModelProfileError::PrimaryImageInputRequired)
    );
    assert_eq!(
        AgentModelProfile::new(
            ModelProfile::new("secondary", 1),
            provider_profile("primary", 1, false),
            None,
            VisionMode::SecondaryTool,
        ),
        Err(AgentModelProfileError::VisionProfileRequired)
    );
    assert_eq!(
        AgentModelProfile::new(
            ModelProfile::new("same", 1),
            provider_profile("shared", 1, true),
            Some(provider_profile("shared", 1, true)),
            VisionMode::Auto,
        ),
        Err(AgentModelProfileError::PrimaryVisionProfileMustDiffer)
    );
    let incomplete_vision = ModelCapabilities {
        structured_output: false,
        ..capabilities(true)
    };
    assert_eq!(
        resolve_vision_route(
            VisionMode::Auto,
            &capabilities(false),
            Some(&incomplete_vision),
            true
        ),
        cyber_agent::domain::profile::VisionRouteDecision {
            route: VisionRoute::Rejected,
            reason: Some(VisionRouteReason::NoImageCapableProvider),
        }
    );
    assert_eq!(
        AgentModelProfile::new(
            ModelProfile::new("incomplete-vision", 1),
            provider_profile("primary", 1, false),
            Some(ProviderProfile::new(
                ModelIdentity::new("fixture-provider", "vision-incomplete"),
                ModelProfile::new("vision-incomplete", 1),
                incomplete_vision,
            )),
            VisionMode::SecondaryTool,
        ),
        Err(AgentModelProfileError::VisionCapabilitiesIncomplete)
    );
}

#[test]
fn doctor_reports_declared_profile_capabilities_without_credential_access() {
    let valid = agent_profile(
        "portable",
        2,
        false,
        Some(provider_profile("vision", 2, true)),
        VisionMode::Auto,
    );
    assert_eq!(
        inspect_agent_model_profile(&valid).status,
        AgentProfileDoctorStatus::Ready
    );

    assert_eq!(
        AgentModelProfile::new(
            ModelProfile::new("broken", 3),
            provider_profile("primary", 3, false),
            None,
            VisionMode::PrimaryDirect,
        ),
        Err(AgentModelProfileError::PrimaryImageInputRequired)
    );
}

#[test]
fn domain_phase_prompt_fragments_and_tool_exposure_are_stable() {
    let composition = PromptComposition::current()
        .with_personal("Prefer explicit evidence.")
        .unwrap()
        .with_project("Target only the fixture project.")
        .unwrap()
        .with_domain_profile(DomainProfile::Mobile)
        .with_research_phase(ResearchPhase::Hypothesis);
    let first = composition.fragments().unwrap();
    let second = composition.fragments().unwrap();
    assert_eq!(first, second);
    assert_eq!(
        first
            .iter()
            .map(|fragment| fragment.id().as_str())
            .filter(|id| id.starts_with("prompt:base:")
                || id.starts_with("prompt:domain:")
                || id.starts_with("prompt:phase:"))
            .collect::<Vec<_>>(),
        vec![
            "prompt:base:pragmatic-vulnerability-researcher:3",
            "prompt:domain:mobile:1",
            "prompt:phase:hypothesis:1",
        ]
    );
    assert!(first[1].id().as_str().starts_with("prompt:personal:"));
    assert!(first[2].id().as_str().starts_with("prompt:project:"));
    let exposure = composition.tool_exposure();
    assert_eq!(
        exposure.exposure_for(&ToolName::new("run_command"), ToolExposure::Direct),
        ToolExposure::Deferred
    );
    let tools = ToolRegistry::for_project(Path::new(env!("CARGO_MANIFEST_DIR")));
    let all_direct = tools
        .resolve_tool_plan(
            &ToolExposurePlan::new([(ToolName::new("run_command"), ToolExposure::Direct)]),
            &DeferredActivationSet::default(),
        )
        .unwrap()
        .advertised_specs()
        .to_vec();
    let phase_direct = tools
        .resolve_tool_plan(&exposure, &DeferredActivationSet::default())
        .unwrap()
        .advertised_specs()
        .to_vec();
    let stable = context_metrics(&composition, phase_direct.clone());
    assert_eq!(stable, context_metrics(&composition, phase_direct.clone()));
    assert_ne!(stable.0, context_metrics(&composition, all_direct).0);
    let changed_phase = PromptComposition::current()
        .with_personal("Prefer explicit evidence.")
        .unwrap()
        .with_project("Target only the fixture project.")
        .unwrap()
        .with_domain_profile(DomainProfile::Mobile)
        .with_research_phase(ResearchPhase::Experiment);
    assert_ne!(
        stable.0,
        context_metrics(
            &changed_phase,
            tools
                .resolve_tool_plan(
                    &changed_phase.tool_exposure(),
                    &DeferredActivationSet::default(),
                )
                .unwrap()
                .advertised_specs()
                .to_vec(),
        )
        .0
    );

    let changed_domain = composition.clone().with_domain_profile(DomainProfile::Web);
    assert_eq!(
        changed_domain.fragments().unwrap()[3].id().as_str(),
        "prompt:domain:web:1"
    );
    assert_ne!(stable.0, context_metrics(&changed_domain, phase_direct).0);
}
#[tokio::test]
async fn deferred_tool_search_changes_actual_schema_and_blocks_pre_discovery_planning() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/m3-tool-exposure");
    fs::create_dir_all(&root).unwrap();
    let exposure = ToolExposurePlan::new([
        (ToolName::new("write_file"), ToolExposure::Deferred),
        (ToolName::new("run_command"), ToolExposure::Hidden),
    ]);
    let tools = ToolRegistry::for_project(&root);
    let mut activations = DeferredActivationSet::default();
    let initial_plan = tools.resolve_tool_plan(&exposure, &activations).unwrap();
    let initial = initial_plan.advertised_specs().to_vec();
    assert!(
        initial
            .iter()
            .any(|spec| spec.name.as_str() == "tool_search")
    );
    assert!(
        !initial
            .iter()
            .any(|spec| spec.name.as_str() == "write_file")
    );
    assert!(
        !initial
            .iter()
            .any(|spec| spec.name.as_str() == "run_command")
    );
    assert!(matches!(
        tools.plan_model_call(
            &initial_plan,
            ExecutionId::new("write-before-search"),
            ToolCall::new(
                ToolCallId::new("write-before-search"),
                ToolName::new("write_file"),
                json!({"path": "x"})
            ),
        ),
        Err(ToolExecutorError::ToolNotExposed { .. })
    ));

    let search = tools
        .execute(
            tools
                .plan_model_call(
                    &initial_plan,
                    ExecutionId::new("tool-search"),
                    ToolCall::new(
                        ToolCallId::new("tool-search"),
                        ToolName::new("tool_search"),
                        json!({"query": "ATOMIC"}),
                    ),
                )
                .unwrap(),
        )
        .await;
    assert_eq!(
        search.result.status,
        cyber_agent::domain::tools::ToolResultStatus::Succeeded
    );
    assert!(
        search.result.output["matches"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["name"] == "write_file")
    );
    activations
        .apply_search_result(&search.result, &initial_plan)
        .unwrap();
    let activated_plan = tools.resolve_tool_plan(&exposure, &activations).unwrap();
    let activated = activated_plan.advertised_specs().to_vec();
    assert!(
        activated
            .iter()
            .any(|spec| spec.name.as_str() == "write_file")
    );
    assert!(
        !activated
            .iter()
            .any(|spec| spec.name.as_str() == "run_command")
    );
    assert!(
        serde_json::to_vec(&activated).unwrap().len() > serde_json::to_vec(&initial).unwrap().len()
    );
    assert!(
        tools
            .resolve_tool_plan(&exposure, &DeferredActivationSet::default())
            .unwrap()
            .advertised_specs()
            .iter()
            .all(|spec| spec.name.as_str() != "write_file")
    );
    let metric_prompt = PromptComposition::current();
    assert!(
        context_metrics(&metric_prompt, activated.clone()).1
            > context_metrics(&metric_prompt, initial.clone()).1
    );

    let primary = ModelIdentity::new("fixture-provider", "model-primary");
    let profile = ModelProfile::new("fixture-profile", 1);
    let session_reference = ProviderSessionRef {
        id: ProviderSessionId::new("primary:exposure"),
        role: ProviderSessionRole::Primary,
        generation: 1,
    };
    let provider_session = ProviderSession::new(
        session_reference.id.clone(),
        session_reference.role,
        session_reference.generation,
    );
    let baseline_input = ModelInputIR::current(
        "fixture",
        vec![ContentItem::user_text("first")],
        ModelRequestRequirements::default(),
    )
    .with_tool_specs(initial.clone());
    let baseline_turn = TurnContext::new(
        TurnId::new("turn:exposure"),
        primary.clone(),
        profile.clone(),
        session_reference.clone(),
        capabilities(false).context_window,
        &initial,
    )
    .unwrap();
    let baseline = StepSnapshot::capture(
        &baseline_turn,
        0,
        &baseline_input,
        &capabilities(false),
        InternalRequestMetadata::default(),
    )
    .unwrap();
    let mut session = provider_session;
    session
        .record_completed_sample(&baseline, Vec::new())
        .unwrap();
    let candidate_input = ModelInputIR::current(
        "fixture",
        vec![
            ContentItem::user_text("first"),
            ContentItem::user_text("next"),
        ],
        ModelRequestRequirements::default(),
    )
    .with_tool_specs(activated.clone());
    let candidate_turn = TurnContext::new(
        TurnId::new("turn:exposure"),
        primary,
        profile,
        session_reference,
        capabilities(false).context_window,
        &activated,
    )
    .unwrap();
    let candidate = StepSnapshot::capture(
        &candidate_turn,
        1,
        &candidate_input,
        &capabilities(false),
        InternalRequestMetadata::default(),
    )
    .unwrap();
    assert_eq!(
        session.assess_delivery(&candidate, true),
        BaselineAssessment::FullRequest {
            reason: BaselineInvalidationReason::ToolsChanged
        }
    );
}

#[test]
fn exposure_overrides_reject_unknown_names_and_respect_tool_search_visibility() {
    let root = TemporaryDirectory::new("m3-exposure-validation");
    let tools = ToolRegistry::for_project(root.path());
    let initial = tools.specs();
    assert!(matches!(
        tools.resolve_tool_plan(
            &ToolExposurePlan::new([(
                ToolName::new("not-a-built-in-tool"),
                ToolExposure::Direct,
            )]),
            &DeferredActivationSet::default(),
        ),
        Err(ToolExecutorError::UnknownTool { tool_name }) if tool_name == "not-a-built-in-tool"
    ));
    assert_eq!(tools.specs(), initial);
    assert!(matches!(
        tools.resolve_tool_plan(
            &ToolExposurePlan::new([(
                ToolName::new("not-a-built-in-tool"),
                ToolExposure::Hidden,
            )]),
            &DeferredActivationSet::default(),
        ),
        Err(ToolExecutorError::UnknownTool { tool_name }) if tool_name == "not-a-built-in-tool"
    ));

    for exposure in [ToolExposure::Hidden, ToolExposure::Deferred] {
        let resolved = tools
            .resolve_tool_plan(
                &ToolExposurePlan::new([(ToolName::new("tool_search"), exposure)]),
                &DeferredActivationSet::default(),
            )
            .unwrap();
        assert!(
            resolved
                .advertised_specs()
                .iter()
                .all(|spec| spec.name.as_str() != "tool_search")
        );
        assert!(matches!(
            tools.plan_model_call(
                &resolved,
                ExecutionId::new(format!("tool-search-{exposure:?}")),
                ToolCall::new(
                    ToolCallId::new(format!("tool-search-{exposure:?}")),
                    ToolName::new("tool_search"),
                    json!({"query": "writer"}),
                ),
            ),
            Err(ToolExecutorError::ToolNotExposed { .. })
        ));
    }
}
