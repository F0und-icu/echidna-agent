use super::*;

#[tokio::test]
async fn queued_mode_and_agent_profile_apply_at_turn_boundary_with_fresh_primary_session() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/m3-agent-loop");
    fs::create_dir_all(&root).unwrap();
    let tools = ToolRegistry::for_project(&root);
    let provider = MockModelProvider::new(MockModelFixture {
        capabilities: capabilities(false),
        events: vec![ModelEvent::TextDelta {
            text: "complete".to_owned(),
        }],
        completion: ModelResponseCompletion::without_response_id(),
    });
    let world_state = ResearchWorldState::from_sections([(
        ResearchWorldStateSection::NextActions,
        json!([{"id": "next-1", "action": "verify fixture"}]),
    )])
    .unwrap();
    let project_snapshot = ProjectSnapshotContext::new(ProjectSnapshot {
        cache_schema_version: 1,
        project_id: ProjectId::new("project-agent-loop"),
        source_fingerprint: ProjectFingerprint {
            kind: ProjectFingerprintKind::Filesystem,
            digest: "agent-loop-fingerprint".to_owned(),
            cacheable: true,
        },
        created_at: "2026-08-23T00:00:00Z".to_owned(),
        rebuild_reason: "fixture".to_owned(),
        file_manifest_hash: "files-agent-loop".to_owned(),
        tool_capability_hash: "tools-agent-loop".to_owned(),
        file_count: 1,
    })
    .unwrap();
    let mut agent = AgentLoop::with_primary_context(
        &provider,
        &tools,
        ModelIdentity::new("fixture-provider", "model-primary"),
        ModelProfile::new("initial-primary", 1),
        ProviderSession::new(
            ProviderSessionId::new("primary:initial"),
            ProviderSessionRole::Primary,
            1,
        ),
    )
    .unwrap()
    .with_world_state(world_state)
    .with_project_snapshot(project_snapshot);
    assert!(matches!(
        agent
            .run_turn("first", AgentCancellation::new())
            .await
            .unwrap(),
        AgentTurnOutcome::Completed { .. }
    ));

    agent.queue_prompt_composition(
        PromptComposition::current()
            .with_project("base-v2")
            .unwrap()
            .with_domain_profile(DomainProfile::Mobile)
            .with_research_phase(ResearchPhase::Hypothesis),
    );
    agent
        .queue_agent_model_profile(agent_profile(
            "agent-v2",
            2,
            false,
            Some(provider_profile("vision", 2, true)),
            VisionMode::Auto,
        ))
        .unwrap();
    assert!(matches!(
        agent
            .run_turn("second", AgentCancellation::new())
            .await
            .unwrap(),
        AgentTurnOutcome::Completed { .. }
    ));
    assert_eq!(
        agent.last_baseline_assessment(),
        Some(&BaselineAssessment::FullRequest {
            reason: BaselineInvalidationReason::NoBaseline
        })
    );
    assert_eq!(
        agent
            .agent_model_profile()
            .unwrap()
            .profile()
            .id
            .to_string(),
        "agent-v2"
    );
    let plan = agent.last_context_plan().unwrap();
    assert!(plan.base_instructions().contains("base-v2"));
    assert!(
        plan.base_instructions()
            .contains(DomainProfile::Mobile.prompt())
    );
    assert!(
        plan.base_instructions()
            .contains(ResearchPhase::Hypothesis.prompt())
    );
    assert!(
        plan.full_logical_manifest()
            .entries()
            .iter()
            .any(|entry| entry.source()
                == cyber_agent::domain::context::ContextFragmentSource::ResearchWorldState)
    );
    assert!(
        plan.full_logical_manifest()
            .entries()
            .iter()
            .any(|entry| entry.source()
                == cyber_agent::domain::context::ContextFragmentSource::ProjectSnapshot)
    );
    assert!(
        !plan
            .tool_specs()
            .iter()
            .any(|spec| spec.name.as_str() == "write_file")
    );
}

#[tokio::test]
async fn model_tool_search_updates_the_next_agent_loop_schema_and_resets_the_baseline() {
    let root = TemporaryDirectory::new("m3-agent-loop-tool-search");
    let tools = ToolRegistry::for_project(root.path());
    let provider = ScriptedProvider::new(
        incremental_capabilities(),
        [
            completed_tool_call_response(
                ToolCall::new(
                    ToolCallId::new("search-deferred"),
                    ToolName::new("tool_search"),
                    json!({"query": "atomic write"}),
                ),
                "search-response",
            ),
            completed_response("schemas refreshed", "final-response"),
        ],
    );
    let mut agent = AgentLoop::new(&provider, &tools);

    let outcome = agent
        .run_turn("find the atomic writer", AgentCancellation::new())
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        AgentTurnOutcome::Completed {
            model_requests: 2,
            ..
        }
    ));

    let requests = provider.requests();
    assert!(
        requests[0]
            .input()
            .tool_specs
            .iter()
            .all(|spec| spec.name.as_str() != "write_file")
    );
    assert!(
        requests[1]
            .input()
            .tool_specs
            .iter()
            .any(|spec| spec.name.as_str() == "write_file")
    );
    assert!(!requests[1].is_incremental());
    assert_eq!(
        agent.last_baseline_assessment(),
        Some(&BaselineAssessment::FullRequest {
            reason: BaselineInvalidationReason::ToolsChanged
        })
    );
}

#[tokio::test]
async fn agent_loop_rejects_model_calls_absent_from_the_sent_schema_before_execution() {
    let registry = ToolRegistry::for_project(env!("CARGO_MANIFEST_DIR"));
    let visible_spec = registry
        .specs()
        .into_iter()
        .find(|spec| spec.name.as_str() == "read_file")
        .unwrap();
    let executions = Arc::new(AtomicUsize::new(0));
    let tools = PermissiveExecutor {
        visible_spec,
        executions: executions.clone(),
    };
    let provider = ScriptedProvider::new(
        capabilities(false),
        [
            completed_tool_call_response(
                ToolCall::new(
                    ToolCallId::new("hidden-call"),
                    ToolName::new("write_file"),
                    json!({"path": "must-not-execute", "content": "x"}),
                ),
                "hidden-response",
            ),
            completed_response("the hidden call was rejected", "done-response"),
        ],
    );
    let mut agent = AgentLoop::new(&provider, &tools);

    let outcome = agent
        .run_turn("try an unadvertised Tool", AgentCancellation::new())
        .await
        .unwrap();
    assert!(
        matches!(outcome, AgentTurnOutcome::Completed { executions, .. } if executions.is_empty())
    );
    assert_eq!(executions.load(Ordering::SeqCst), 0);
    assert!(agent.history().items().iter().any(|item| matches!(
        item,
        ContentItem::ToolResult { result }
            if result.call_id.as_str() == "hidden-call" && result.status == cyber_agent::domain::tools::ToolResultStatus::Failed
    )));
}

#[tokio::test]
async fn changed_world_state_uses_a_causal_incremental_diff_and_full_retry_preserves_the_full_snapshot()
 {
    let root = TemporaryDirectory::new("m3-world-diff-rejection");
    let tools = ToolRegistry::for_project(root.path());
    let provider = ScriptedProvider::new(
        incremental_capabilities(),
        [
            completed_response("first", "response-one"),
            rejected_response(),
            completed_response("recovered", "response-two"),
        ],
    );
    let mut agent = AgentLoop::new(&provider, &tools).with_world_state(world_state("a"));
    agent
        .run_turn("first user", AgentCancellation::new())
        .await
        .unwrap();
    agent.set_world_state(world_state("b"));
    agent
        .run_turn("second user", AgentCancellation::new())
        .await
        .unwrap();

    let requests = provider.requests();
    assert_eq!(requests.len(), 3);
    assert!(requests[1].is_incremental());
    assert!(
        requests[1]
            .input()
            .content_items
            .iter()
            .any(|item| matches!(
                item,
                ContentItem::Text { text, .. }
                    if text.contains("research_world_state_diff") && text.contains("\"b\"")
            ))
    );
    assert_eq!(
        requests[1].input().content_items.last(),
        Some(&ContentItem::user_text("second user"))
    );
    assert!(!requests[2].is_incremental());
    assert!(
        requests[2]
            .input()
            .content_items
            .iter()
            .any(|item| matches!(
                item,
                ContentItem::Text { text, .. }
                    if text.contains("research_world_state_full") && text.contains("\"b\"")
            ))
    );
    assert_eq!(
        agent.last_baseline_assessment(),
        Some(&BaselineAssessment::FullRequest {
            reason: BaselineInvalidationReason::ProviderRejected
        })
    );
}

#[tokio::test]
async fn truncated_world_state_plan_never_injects_an_unplanned_diff() {
    let root = TemporaryDirectory::new("m3-truncated-world-state");
    let tools = ToolRegistry::for_project(root.path());
    let constrained_capabilities = ModelCapabilities {
        // Exact M4-S3e1 Research schemas are part of the fixed model input.
        // Keep enough room for that contract while still forcing the
        // deliberately oversized WorldState through the truncation path.
        context_window: 18_432,
        ..incremental_capabilities()
    };
    let provider = ScriptedProvider::new(
        constrained_capabilities,
        [
            completed_response("first", "response-one"),
            completed_response("second", "response-two"),
        ],
    );
    let mut agent =
        AgentLoop::new(&provider, &tools).with_world_state(bounded_large_world_state("a"));
    agent
        .run_turn("first user", AgentCancellation::new())
        .await
        .unwrap();
    agent.set_world_state(bounded_large_world_state("b"));
    agent
        .run_turn("second user", AgentCancellation::new())
        .await
        .unwrap();

    let state_decision = agent
        .last_context_plan()
        .unwrap()
        .context_decision_manifest()
        .candidates()
        .iter()
        .find(|entry| entry.source() == ContextFragmentSource::ResearchWorldState)
        .unwrap()
        .decision();
    assert!(
        !matches!(
            state_decision,
            cyber_agent::domain::context::ContextDecision::Include
        ),
        "the constrained Plan must not select the complete WorldState"
    );
    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert!(
        !requests[1].is_incremental(),
        "a truncated state projection must be sent as the compiled Full request"
    );
    assert!(
        requests[1]
            .input()
            .content_items
            .iter()
            .all(|item| !matches!(
                item,
                ContentItem::Text { text, .. } if text.contains("research_world_state_diff")
            ))
    );
}

#[tokio::test]
async fn changed_project_snapshot_uses_a_causal_incremental_diff() {
    let root = TemporaryDirectory::new("m3-project-snapshot-diff");
    let tools = ToolRegistry::for_project(root.path());
    let provider = ScriptedProvider::new(
        incremental_capabilities(),
        [
            completed_response("first", "response-one"),
            completed_response("second", "response-two"),
        ],
    );
    let mut agent = AgentLoop::new(&provider, &tools).with_project_snapshot(
        project_snapshot_context(1, "2026-08-23T00:00:00Z", "initial"),
    );
    agent
        .run_turn("first user", AgentCancellation::new())
        .await
        .unwrap();
    agent.set_project_snapshot(project_snapshot_context(
        2,
        "2026-08-23T01:00:00Z",
        "filesystem changed",
    ));
    agent
        .run_turn("second user", AgentCancellation::new())
        .await
        .unwrap();

    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].is_incremental());
    assert!(
        requests[1]
            .input()
            .content_items
            .iter()
            .any(|item| matches!(
                item,
                ContentItem::Text { text, .. }
                    if text.contains("project_snapshot_diff") && text.contains("\"file_count\":2")
            ))
    );
    assert_eq!(
        requests[1].input().content_items.last(),
        Some(&ContentItem::user_text("second user"))
    );
}

#[tokio::test]
async fn failed_state_diff_request_does_not_commit_derived_state_markers() {
    let root = TemporaryDirectory::new("m3-world-diff-failure");
    let tools = ToolRegistry::for_project(root.path());
    let provider = ScriptedProvider::new(
        incremental_capabilities(),
        [
            completed_response("first", "response-one"),
            failed_response("fixture network failure"),
            completed_response("third", "response-three"),
        ],
    );
    let mut agent = AgentLoop::new(&provider, &tools).with_world_state(world_state("a"));
    agent
        .run_turn("first user", AgentCancellation::new())
        .await
        .unwrap();
    agent.set_world_state(world_state("b"));
    assert!(
        agent
            .run_turn("failed user", AgentCancellation::new())
            .await
            .is_err()
    );
    agent.set_world_state(world_state("c"));
    agent
        .run_turn("third user", AgentCancellation::new())
        .await
        .unwrap();

    let requests = provider.requests();
    assert_eq!(requests.len(), 3);
    assert!(requests[1].is_incremental());
    assert!(requests[2].is_incremental());
    let third_delta = requests[2]
        .input()
        .content_items
        .iter()
        .find_map(|item| match item {
            ContentItem::Text { text, .. } if text.contains("research_world_state_diff") => {
                Some(text)
            }
            _ => None,
        })
        .unwrap();
    assert!(third_delta.contains("\"c\""));
    assert!(!third_delta.contains("\"b\""));
}

#[tokio::test]
async fn unchanged_world_and_snapshot_fragments_hit_the_fragment_cache() {
    let root = TemporaryDirectory::new("m3-world-fragment-cache");
    let project = Project::initialize(root.path()).unwrap();
    let tools = ToolRegistry::for_project(project.root());
    let provider = MockModelProvider::new(MockModelFixture {
        capabilities: capabilities(false),
        events: vec![ModelEvent::TextDelta {
            text: "complete".to_owned(),
        }],
        completion: ModelResponseCompletion::without_response_id(),
    });
    let snapshot = ProjectSnapshot {
        cache_schema_version: 1,
        project_id: ProjectId::new("cache-project"),
        source_fingerprint: ProjectFingerprint {
            kind: ProjectFingerprintKind::Filesystem,
            digest: "cache-source".to_owned(),
            cacheable: true,
        },
        created_at: "2026-08-23T00:00:00Z".to_owned(),
        rebuild_reason: "fixture".to_owned(),
        file_manifest_hash: "files".to_owned(),
        tool_capability_hash: "tools".to_owned(),
        file_count: 1,
    };
    let cache = Arc::new(LocalFragmentCache::for_project(&project));
    let mut agent = AgentLoop::new(&provider, &tools)
        .with_fragment_cache(cache)
        .with_world_state(world_state("a"))
        .with_project_snapshot(ProjectSnapshotContext::new(snapshot.clone()).unwrap());
    let mut volatile_snapshot = snapshot;
    volatile_snapshot.created_at = "2026-08-23T01:00:00Z".to_owned();
    volatile_snapshot.rebuild_reason = "fresh cache metadata only".to_owned();
    agent.set_project_snapshot(ProjectSnapshotContext::new(volatile_snapshot).unwrap());
    agent
        .run_turn("first", AgentCancellation::new())
        .await
        .unwrap();
    assert_eq!(
        agent
            .last_successful_context()
            .unwrap()
            .cache_summary()
            .misses,
        5
    );
    agent
        .run_turn("second", AgentCancellation::new())
        .await
        .unwrap();
    assert_eq!(
        agent
            .last_successful_context()
            .unwrap()
            .cache_summary()
            .hits,
        5
    );
}

#[tokio::test]
async fn secondary_profile_and_vision_mode_changes_keep_primary_session_but_primary_changes_reset_freshly()
 {
    let root = TemporaryDirectory::new("m3-profile-sessions");
    let tools = ToolRegistry::for_project(root.path());
    let provider = ScriptedProvider::new(
        incremental_capabilities(),
        [
            completed_response("one", "response-one"),
            completed_response("two", "response-two"),
            completed_response("three", "response-three"),
            completed_response("four", "response-four"),
            completed_response("five", "response-five"),
        ],
    );
    let mut agent = AgentLoop::with_primary_context(
        &provider,
        &tools,
        ModelIdentity::new("fixture-provider", "model-primary"),
        ModelProfile::new("primary", 1),
        ProviderSession::new(
            ProviderSessionId::new("primary:runtime:1"),
            ProviderSessionRole::Primary,
            1,
        ),
    )
    .unwrap();
    agent
        .queue_agent_model_profile(incremental_agent_profile(
            "portable",
            1,
            Some(provider_profile("vision-a", 1, true)),
            VisionMode::Auto,
        ))
        .unwrap();
    agent
        .run_turn("one", AgentCancellation::new())
        .await
        .unwrap();

    agent
        .queue_agent_model_profile(incremental_agent_profile(
            "portable-with-new-secondary",
            2,
            Some(provider_profile("vision-b", 2, true)),
            VisionMode::Auto,
        ))
        .unwrap();
    agent
        .run_turn("two", AgentCancellation::new())
        .await
        .unwrap();

    agent
        .queue_agent_model_profile(incremental_agent_profile(
            "portable-disabled-vision",
            3,
            Some(provider_profile("vision-b", 2, true)),
            VisionMode::Disabled,
        ))
        .unwrap();
    agent
        .run_turn("three", AgentCancellation::new())
        .await
        .unwrap();

    let primary_b = AgentModelProfile::new(
        ModelProfile::new("primary-b", 2),
        ProviderProfile::new(
            ModelIdentity::new("fixture-provider", "model-primary"),
            ModelProfile::new("primary-b", 2),
            incremental_capabilities(),
        ),
        Some(provider_profile("vision-b", 2, true)),
        VisionMode::Disabled,
    )
    .unwrap();
    agent.queue_agent_model_profile(primary_b).unwrap();
    agent
        .run_turn("four", AgentCancellation::new())
        .await
        .unwrap();

    let primary_a_again = AgentModelProfile::new(
        ModelProfile::new("primary", 1),
        ProviderProfile::new(
            ModelIdentity::new("fixture-provider", "model-primary"),
            ModelProfile::new("primary", 1),
            incremental_capabilities(),
        ),
        Some(provider_profile("vision-b", 2, true)),
        VisionMode::Disabled,
    )
    .unwrap();
    agent.queue_agent_model_profile(primary_a_again).unwrap();
    agent
        .run_turn("five", AgentCancellation::new())
        .await
        .unwrap();

    let requests = provider.requests();
    assert!(requests[1].is_incremental());
    assert!(requests[2].is_incremental());
    assert!(!requests[3].is_incremental());
    assert!(!requests[4].is_incremental());
}

#[tokio::test]
async fn unchanged_state_after_a_successful_diff_remains_incremental() {
    let root = TemporaryDirectory::new("m3-world-diff-steady-state");
    let tools = ToolRegistry::for_project(root.path());
    let provider = ScriptedProvider::new(
        incremental_capabilities(),
        [
            completed_response("first", "response-one"),
            completed_response("second", "response-two"),
            completed_response("third", "response-three"),
        ],
    );
    let mut agent = AgentLoop::new(&provider, &tools).with_world_state(world_state("a"));
    agent
        .run_turn("first user", AgentCancellation::new())
        .await
        .unwrap();
    agent.set_world_state(world_state("b"));
    agent
        .run_turn("second user", AgentCancellation::new())
        .await
        .unwrap();
    agent
        .run_turn("third user", AgentCancellation::new())
        .await
        .unwrap();

    let requests = provider.requests();
    assert_eq!(requests.len(), 3);
    assert!(requests[1].is_incremental());
    assert!(requests[2].is_incremental());
    assert_eq!(
        requests[2].input().content_items,
        vec![ContentItem::user_text("third user")]
    );
}

#[tokio::test]
async fn accumulated_state_diffs_that_exceed_the_rendered_budget_fall_back_to_full_then_resume_incremental()
 {
    let root = TemporaryDirectory::new("m3-state-diff-budget");
    let registry = ToolRegistry::for_project(root.path());
    let visible_spec = registry
        .specs()
        .into_iter()
        .find(|spec| spec.name.as_str() == "read_file")
        .unwrap();
    let executions = Arc::new(AtomicUsize::new(0));
    let tools = PermissiveExecutor {
        visible_spec,
        executions,
    };
    let provider = ScriptedProvider::new(
        ModelCapabilities {
            context_window: 4_096,
            ..incremental_capabilities()
        },
        (0..28).map(|index| completed_response("complete", format!("response-{index}").as_str())),
    );
    let mut agent = AgentLoop::new(&provider, &tools).with_world_state(diff_heavy_world_state(0));

    agent
        .run_turn("turn-0", AgentCancellation::new())
        .await
        .unwrap();
    for version in 1..28 {
        agent.set_world_state(diff_heavy_world_state(version));
        agent
            .run_turn(format!("turn-{version}"), AgentCancellation::new())
            .await
            .unwrap();
    }

    let requests = provider.requests();
    let fallback_index = requests
        .iter()
        .enumerate()
        .skip(1)
        .find_map(|(index, request)| (!request.is_incremental()).then_some(index))
        .expect("cumulative Full-plus-Diff delivery must fall back before exceeding the budget");
    assert!(
        requests[1..fallback_index]
            .iter()
            .any(|request| request.is_incremental()),
        "the fallback must follow successful state-diff incremental delivery"
    );
    assert!(
        requests[fallback_index]
            .input()
            .content_items
            .iter()
            .any(|item| matches!(
                item,
                ContentItem::Text { text, .. } if text.contains("research_world_state_full")
            )),
        "the fallback must use the Planner-verified Full state projection"
    );
    assert!(
        requests[fallback_index]
            .input()
            .content_items
            .iter()
            .all(|item| !matches!(
                item,
                ContentItem::Text { text, .. } if text.contains("research_world_state_diff")
            )),
        "the overflow candidate must not reach the Provider"
    );
    assert!(
        requests
            .get(fallback_index + 1)
            .is_some_and(|request| request.is_incremental()),
        "a successful Full fallback must commit a fresh baseline for the next state Diff"
    );
}

#[tokio::test]
async fn phase_exposure_change_resets_the_primary_baseline() {
    let root = TemporaryDirectory::new("m3-exposure-directory-reset");
    let tools = ToolRegistry::for_project(root.path());
    let provider = ScriptedProvider::new(
        incremental_capabilities(),
        [
            completed_response("first", "response-one"),
            completed_response("second", "response-two"),
            completed_response("third", "response-three"),
        ],
    );
    let mut agent = AgentLoop::new(&provider, &tools).with_research_phase(ResearchPhase::Mapping);

    agent
        .run_turn("first", AgentCancellation::new())
        .await
        .unwrap();
    agent.queue_research_phase(ResearchPhase::Experiment);
    agent
        .run_turn("second", AgentCancellation::new())
        .await
        .unwrap();
    assert_eq!(
        agent.last_baseline_assessment(),
        Some(&BaselineAssessment::FullRequest {
            reason: BaselineInvalidationReason::ToolsChanged
        })
    );
    agent
        .run_turn("third", AgentCancellation::new())
        .await
        .unwrap();

    let requests = provider.requests();
    assert_eq!(requests.len(), 3);
    assert_ne!(
        requests[0].input().tool_specs,
        requests[1].input().tool_specs
    );
    assert!(!requests[1].is_incremental());
    assert!(requests[2].is_incremental());
}

#[tokio::test]
async fn failed_turn_boundary_configuration_keeps_pending_prompt_and_exposure_unapplied() {
    let root = TemporaryDirectory::new("m3-atomic-configuration");
    let tools = ToolRegistry::for_project(root.path());
    let provider = MockModelProvider::new(MockModelFixture {
        capabilities: capabilities(false),
        events: vec![ModelEvent::TextDelta {
            text: "complete".to_owned(),
        }],
        completion: ModelResponseCompletion::without_response_id(),
    });
    let mut agent = AgentLoop::new(&provider, &tools);
    agent.queue_prompt_composition(
        PromptComposition::current().with_research_phase(ResearchPhase::Experiment),
    );
    agent
        .queue_agent_model_profile(agent_profile(
            "wrong-bound-model",
            1,
            false,
            None,
            VisionMode::Disabled,
        ))
        .unwrap();
    assert!(matches!(
        agent.run_turn("blocked", AgentCancellation::new()).await,
        Err(cyber_agent::app::agent_loop::AgentLoopError::Configuration(
            cyber_agent::app::agent_loop::AgentLoopConfigurationError::PrimaryModelDoesNotMatchProvider
        ))
    ));
    assert!(
        tools
            .specs()
            .iter()
            .all(|spec| spec.name.as_str() != "write_file")
    );

    let bound_profile = AgentModelProfile::new(
        ModelProfile::new("bound-primary", 1),
        ProviderProfile::new(
            ModelIdentity::new("primary", "default"),
            ModelProfile::new("bound-primary", 1),
            capabilities(false),
        ),
        None,
        VisionMode::Disabled,
    )
    .unwrap();
    agent.queue_agent_model_profile(bound_profile).unwrap();
    agent
        .run_turn("applied", AgentCancellation::new())
        .await
        .unwrap();
    assert!(
        agent
            .last_context_plan()
            .unwrap()
            .base_instructions()
            .contains(ResearchPhase::Experiment.prompt())
    );
    assert!(
        agent
            .last_context_plan()
            .unwrap()
            .tool_specs()
            .iter()
            .any(|spec| spec.name.as_str() == "write_file")
    );
}

#[tokio::test]
async fn resume_replaces_the_primary_identity_forces_full_then_allows_fresh_incremental_delivery() {
    let root = TemporaryDirectory::new("m3-session-resume");
    let tools = ToolRegistry::for_project(root.path());
    let provider = ScriptedProvider::new(
        incremental_capabilities(),
        [
            completed_response("initial", "response-initial"),
            completed_response("resumed", "response-resumed"),
            completed_response("continued", "response-continued"),
        ],
    );
    let mut agent = AgentLoop::with_primary_context(
        &provider,
        &tools,
        ModelIdentity::new("fixture", "model"),
        ModelProfile::new("profile", 1),
        ProviderSession::new(
            ProviderSessionId::new("primary-initial"),
            ProviderSessionRole::Primary,
            1,
        ),
    )
    .unwrap();

    agent
        .run_turn("initial user input", AgentCancellation::new())
        .await
        .unwrap();
    let resumed = resumed_session_state("primary-resumed", 2);
    assert!(matches!(
        agent.resume_from_session(
            &resumed,
            ProviderSession::new(
                ProviderSessionId::new("primary-wrong"),
                ProviderSessionRole::Primary,
                2,
            ),
            None,
        ),
        Err(cyber_agent::app::agent_loop::AgentLoopConfigurationError::ReplayedSessionProviderMismatch { .. })
    ));
    agent
        .resume_from_session(
            &resumed,
            ProviderSession::new(
                ProviderSessionId::new("primary-resumed"),
                ProviderSessionRole::Primary,
                2,
            ),
            None,
        )
        .unwrap();
    assert!(agent.last_sampling_step_plan().is_none());
    agent
        .run_turn("resumed user input", AgentCancellation::new())
        .await
        .unwrap();
    assert_eq!(
        agent.last_baseline_assessment(),
        Some(&BaselineAssessment::FullRequest {
            reason: BaselineInvalidationReason::SessionResumed,
        })
    );
    agent
        .run_turn("continue after resume", AgentCancellation::new())
        .await
        .unwrap();

    let requests = provider.requests();
    assert_eq!(requests.len(), 3);
    assert!(!requests[0].is_incremental());
    assert!(
        !requests[1].is_incremental(),
        "the first resumed request must not reuse the old response identity"
    );
    assert!(matches!(
        requests[2].delivery(),
        cyber_agent::ports::model::ModelRequestDelivery::Incremental { continuation }
            if continuation.as_str() == "response-resumed"
    ));
}

#[tokio::test]
async fn failed_resumed_full_keeps_session_resumed_pending_for_the_next_attempt() {
    let root = TemporaryDirectory::new("m3-session-resume-failure");
    let tools = ToolRegistry::for_project(root.path());
    let provider = ScriptedProvider::new(
        incremental_capabilities(),
        [
            completed_response("initial", "response-initial"),
            vec![Err(ModelProviderError::ProviderFailure {
                code: Some("temporary_failure".to_owned()),
                message: "the resumed full request failed".to_owned(),
            })],
            completed_response("retry", "response-resumed-retry"),
        ],
    );
    let mut agent = AgentLoop::with_primary_context(
        &provider,
        &tools,
        ModelIdentity::new("fixture", "model"),
        ModelProfile::new("profile", 1),
        ProviderSession::new(
            ProviderSessionId::new("primary-initial"),
            ProviderSessionRole::Primary,
            1,
        ),
    )
    .unwrap();
    agent
        .run_turn("initial user input", AgentCancellation::new())
        .await
        .unwrap();
    agent
        .resume_from_session(
            &resumed_session_state("primary-resumed", 2),
            ProviderSession::new(
                ProviderSessionId::new("primary-resumed"),
                ProviderSessionRole::Primary,
                2,
            ),
            None,
        )
        .unwrap();

    assert!(matches!(
        agent
            .run_turn("failed resumed input", AgentCancellation::new())
            .await,
        Err(cyber_agent::app::agent_loop::AgentLoopError::Model(_))
    ));
    assert_eq!(
        agent.last_baseline_assessment(),
        Some(&BaselineAssessment::FullRequest {
            reason: BaselineInvalidationReason::SessionResumed,
        })
    );
    agent
        .run_turn("retry resumed input", AgentCancellation::new())
        .await
        .unwrap();

    let requests = provider.requests();
    assert_eq!(requests.len(), 3);
    assert!(requests.iter().all(|request| !request.is_incremental()));
    assert_eq!(
        agent.last_baseline_assessment(),
        Some(&BaselineAssessment::FullRequest {
            reason: BaselineInvalidationReason::SessionResumed,
        })
    );
}

#[tokio::test]
async fn resume_from_replayed_session_history_replaces_runtime_history_and_forces_full() {
    let root = TemporaryDirectory::new("m3-replayed-session-resume");
    let tools = ToolRegistry::for_project(root.path());
    let provider = ScriptedProvider::new(
        incremental_capabilities(),
        [
            completed_response("initial", "response-initial"),
            completed_response("restored", "response-restored"),
        ],
    );
    let mut agent = AgentLoop::with_primary_context(
        &provider,
        &tools,
        ModelIdentity::new("fixture", "model"),
        ModelProfile::new("profile", 1),
        ProviderSession::new(
            ProviderSessionId::new("primary-initial"),
            ProviderSessionRole::Primary,
            1,
        ),
    )
    .unwrap();
    agent
        .run_turn("volatile user input", AgentCancellation::new())
        .await
        .unwrap();
    let mut restored = resumed_session_state("primary-restored", 2);
    SessionReducer
        .apply(
            &mut restored,
            SessionEvent::HistoryItemRecorded {
                item: ContentItem::user_text("durably recorded user input"),
            }
            .into_envelope(
                EventId::new("history-user"),
                StreamId::new("session:resume-fixture"),
                EventSequence::new(3),
                OccurredAt::new("2026-08-23T00:00:02Z"),
            )
            .unwrap(),
        )
        .unwrap();
    agent
        .resume_from_session(
            &restored,
            ProviderSession::new(
                ProviderSessionId::new("primary-restored"),
                ProviderSessionRole::Primary,
                2,
            ),
            None,
        )
        .unwrap();
    agent
        .run_turn("continue restored session", AgentCancellation::new())
        .await
        .unwrap();

    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert!(!requests[1].is_incremental());
    assert!(
        requests[1]
            .input()
            .content_items
            .iter()
            .any(|item| matches!(
                item,
                ContentItem::Text { text, .. } if text == "durably recorded user input"
            ))
    );
    let restored_position = requests[1]
        .input()
        .content_items
        .iter()
        .position(|item| {
            matches!(
                item,
                ContentItem::Text { text, .. } if text == "durably recorded user input"
            )
        })
        .unwrap();
    let restored_provenance = agent
        .last_context_plan()
        .unwrap()
        .delivery_map()
        .content_items()[restored_position]
        .provenance();
    assert_eq!(
        restored_provenance.authority_level(),
        AuthorityLevel::UnclassifiedLegacy,
        "Resume must not guess that a durable user-role item was an operator assertion"
    );
    assert_eq!(
        agent.last_baseline_assessment(),
        Some(&BaselineAssessment::FullRequest {
            reason: BaselineInvalidationReason::SessionResumed,
        })
    );
}
