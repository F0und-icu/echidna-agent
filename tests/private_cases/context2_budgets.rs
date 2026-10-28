use crate::adapters::tools::ToolRegistry;
use crate::domain::context::{ContextBudget, ContextBudgetPolicy, ContextPlanner};
use crate::domain::model::{CapabilityMappingVersion, ModelCapabilities};
use crate::domain::tools::{ExecutionId, ToolCall, ToolCallId, ToolName};
use crate::ports::tool::ToolExecutor;

fn capabilities(window: u32, output: Option<u32>) -> ModelCapabilities {
    ModelCapabilities {
        capability_mapping_version: CapabilityMappingVersion::new(1),
        text_input: true,
        image_input: false,
        streaming: true,
        tool_call: true,
        structured_output: false,
        reasoning_effort: None,
        max_output_tokens: output,
        usage_reporting: false,
        incremental_input: false,
        context_window: window,
    }
}

#[test]
fn capacity_failure_breakdown_uses_the_same_planner_estimates_and_keeps_error() {
    use crate::domain::artifacts::ArtifactId;
    use crate::domain::context::{
        ContextFragment, ContextFragmentContent, ContextFragmentId, ContextFragmentPriority,
        ContextFragmentSource, ContextPlannerError, FragmentCachePolicy, tool_schema_token_cost,
    };
    use crate::domain::model::{
        ContentItem, MessageRole, ModelRequestRequirements, OpaqueContinuationReference,
    };

    let make = |id: &str, source, content| {
        ContextFragment::new(
            ContextFragmentId::new(id).unwrap(),
            source,
            ContextFragmentPriority::Required,
            FragmentCachePolicy::DoNotCache,
            content,
        )
        .unwrap()
    };
    let opaque: OpaqueContinuationReference = serde_json::from_value(serde_json::json!({
        "scope":"provider-session:fixture", "content_identity":"fixture", "encoded_byte_length":20000
    })).unwrap();
    let fragments = vec![
        make(
            "base",
            ContextFragmentSource::BasePrompt,
            ContextFragmentContent::Instructions {
                text: "fixed instruction".to_owned(),
            },
        ),
        make(
            "memory",
            ContextFragmentSource::ProjectMemoryCore,
            ContextFragmentContent::ContentItems {
                items: vec![ContentItem::user_text("bounded memory")],
            },
        ),
        make(
            "history",
            ContextFragmentSource::RecentHistory,
            ContextFragmentContent::ContentItems {
                items: vec![ContentItem::assistant_text("old result")],
            },
        ),
        make(
            "input",
            ContextFragmentSource::UserInput,
            ContextFragmentContent::ContentItems {
                items: vec![
                    ContentItem::user_text("current"),
                    ContentItem::ImageArtifact {
                        role: MessageRole::User,
                        artifact_id: ArtifactId::new("fixture-image"),
                    },
                    ContentItem::OpaqueContinuation { reference: opaque },
                ],
            },
        ),
    ];
    let tools = ToolRegistry::for_project(".")
        .specs()
        .into_iter()
        .take(1)
        .collect::<Vec<_>>();
    let caps = capabilities(16_384, None);
    let planner = ContextPlanner::current();
    for limit in [1, 4_096] {
        let budget = ContextBudget::new(limit, 0, 0).unwrap();
        let ordinary = planner
            .compile(
                fragments.clone(),
                budget,
                &caps,
                tools.clone(),
                ModelRequestRequirements::default(),
                None,
            )
            .unwrap_err();
        let mut breakdown = None;
        let diagnostic = planner
            .compile_with_capacity_breakdown(
                fragments.clone(),
                budget,
                &caps,
                tools.clone(),
                ModelRequestRequirements::default(),
                None,
                &mut breakdown,
            )
            .unwrap_err();
        assert_eq!(diagnostic, ordinary);
        let detail = breakdown.expect("capacity failure exposes bounded estimates");
        assert_eq!(
            detail.tool_schema_tokens,
            tool_schema_token_cost(&tools).unwrap()
        );
        assert_eq!(
            detail.fragment_budget,
            limit.checked_sub(detail.tool_schema_tokens)
        );
        assert_eq!(
            detail.instructions,
            u64::from(fragments[0].token_estimate())
        );
        assert_eq!(detail.memory, u64::from(fragments[1].token_estimate()));
        assert_eq!(detail.history, u64::from(fragments[2].token_estimate()));
        assert_eq!(detail.input, u64::from(fragments[3].token_estimate()));
        assert_eq!(
            detail.required_fragments,
            fragments
                .iter()
                .map(|fragment| u64::from(fragment.token_estimate()))
                .sum::<u64>()
        );
        assert_eq!((detail.images, detail.opaque_items), (1, 1));
        if limit == 1 {
            assert!(matches!(
                diagnostic,
                ContextPlannerError::FixedInputExceedsBudget { .. }
            ));
        } else {
            assert!(matches!(
                diagnostic,
                ContextPlannerError::RequiredFragmentsExceedBudget { .. }
            ));
        }
    }
    let budget = ContextBudget::new(16_384, 0, 0).unwrap();
    let ordinary = planner
        .compile(
            vec![fragments[0].clone()],
            budget,
            &caps,
            tools.clone(),
            ModelRequestRequirements::default(),
            None,
        )
        .unwrap();
    let mut breakdown = None;
    let detailed = planner
        .compile_with_capacity_breakdown(
            vec![fragments[0].clone()],
            budget,
            &caps,
            tools,
            ModelRequestRequirements::default(),
            None,
            &mut breakdown,
        )
        .unwrap();
    assert_eq!(detailed.model_input(), ordinary.model_input());
    assert_eq!(
        detailed.context_fingerprint(),
        ordinary.context_fingerprint()
    );
    assert!(breakdown.is_none());
}

#[test]
fn pressure_and_soft_target_follow_reserved_capacity_without_a_long_window_ceiling() {
    for (window, output) in [
        (32768, None),
        (272000, None),
        (1_000_000, None),
        (1_048_576, None),
        (272000, Some(100000)),
        (u32::MAX, None),
    ] {
        let budget = ContextPlanner::current()
            .default_budget(&capabilities(window, output))
            .unwrap();
        let input = budget.input_token_limit();
        assert_eq!(
            budget.working_set_input_tokens(),
            (u64::from(input) * 4 / 5) as u32
        );
        assert_eq!(budget.compaction_target_input_tokens(), input / 2);
        assert!(budget.compaction_target_input_tokens() <= budget.working_set_input_tokens());
    }
    let budget = ContextBudget::new(272000, 34000, 34000).unwrap();
    assert_eq!(budget.working_set_input_tokens(), 163200);
    assert_eq!(budget.compaction_target_input_tokens(), 102000);
}

#[test]
fn tool_free_handoff_releases_only_tool_reserve_and_preserves_explicit_output() {
    let planner = ContextPlanner::current();
    for output in [None, Some(100000)] {
        let caps = capabilities(272000, output);
        let parent = planner.default_budget(&caps).unwrap();
        let summary = planner.tool_free_compaction_budget(&caps).unwrap();
        assert_eq!(
            summary.context_window_tokens(),
            parent.context_window_tokens()
        );
        assert_eq!(
            summary.reserved_output_tokens(),
            parent.reserved_output_tokens()
        );
        assert_eq!(summary.reserved_tool_loop_tokens(), 0);
        assert_eq!(
            summary.input_token_limit(),
            parent.input_token_limit() + 34000
        );
        assert_eq!(planner.default_budget(&caps).unwrap(), parent);
    }
}

#[test]
fn policy_uses_actual_usable_capacity_and_has_no_old_long_window_content_ceiling() {
    let policy = ContextBudgetPolicy::default();
    let small = policy
        .for_capabilities(&capabilities(272_000, None))
        .unwrap();
    let large = policy
        .for_capabilities(&capabilities(4_000_000, None))
        .unwrap();
    let limited = policy
        .for_capabilities(&capabilities(272_000, Some(100_000)))
        .unwrap();
    assert_eq!(small.budget().input_token_limit(), 204_000);
    assert_eq!(small.summary_bytes(), 16_320);
    assert_eq!(limited.budget().input_token_limit(), 138_000);
    assert_eq!(limited.summary_bytes(), 11_040);
    assert_eq!(large.summary_bytes(), 240_000);
    assert!(large.tool_output_bytes() > 1024 * 1024);
    assert_eq!(large.core_memory_bytes(), 512 * 1024);
    let larger_summary = ContextBudgetPolicy {
        summary_bps: 400,
        ..policy
    }
    .for_capabilities(&capabilities(272_000, None))
    .unwrap();
    assert_eq!(larger_summary.summary_bytes(), small.summary_bytes() * 2);
    assert_eq!(larger_summary.diagnostic_bytes(), small.diagnostic_bytes());
    assert_eq!(
        larger_summary.diagnostic_task_bytes(),
        small.diagnostic_task_bytes()
    );
    let enlarged = ContextBudgetPolicy {
        max_core_file_bytes: 4 * 1024 * 1024,
        ..policy
    }
    .for_capabilities(&capabilities(4_000_000, None))
    .unwrap();
    assert!(enlarged.core_memory_bytes() > 512 * 1024);
    assert_eq!(enlarged.core_file_bytes(), 4 * 1024 * 1024);
    assert!(large.readback_max_bytes() > 128 * 1024);
    let capped = ContextBudgetPolicy {
        max_content_bytes: Some(4096),
        ..policy
    }
    .for_capabilities(&capabilities(4_000_000, None))
    .unwrap();
    assert_eq!(capped.summary_bytes(), 4096);
    assert_eq!(capped.tool_output_bytes(), 4096);
}

#[test]
fn config_defaults_validation_and_fingerprint_share_one_policy_snapshot() {
    let default: crate::provider_composition::RuntimeSettings = toml::from_str("").unwrap();
    assert_eq!(default.context_budget, ContextBudgetPolicy::default());
    let configured: crate::provider_composition::RuntimeSettings = toml::from_str(
        "[context_budget]\npressure_bps = 7500\ntarget_bps = 4000\nsummary_bps = 300\ncompaction_reference_items = 42\n"
    ).unwrap();
    let policy = configured.context_budget;
    assert!(policy.validate().is_ok());
    assert_eq!(policy.compaction_reference_items, 42);
    let capabilities = capabilities(272_000, None);
    let configured_planner = ContextPlanner::current().with_budget_policy(policy);
    let budget = configured_planner.default_budget(&capabilities).unwrap();
    assert_eq!(budget.working_set_input_tokens(), 153_000);
    assert_eq!(budget.compaction_target_input_tokens(), 81_600);
    let default_planner = ContextPlanner::current();
    let make_plan = |planner: &ContextPlanner| {
        planner
            .compile(
                vec![],
                budget,
                &capabilities,
                vec![],
                Default::default(),
                None,
            )
            .unwrap()
    };
    assert_ne!(
        make_plan(&configured_planner).context_fingerprint(),
        make_plan(&default_planner).context_fingerprint()
    );
    assert!(
        toml::from_str::<crate::provider_composition::RuntimeSettings>(
            "[context_budget]\nunknown_bps = 1"
        )
        .is_err()
    );
    for invalid in [
        ContextBudgetPolicy {
            summary_bps: 0,
            ..policy
        },
        ContextBudgetPolicy {
            output_reserve_bps: 9000,
            ..policy
        },
        ContextBudgetPolicy {
            target_bps: 8000,
            ..policy
        },
        ContextBudgetPolicy {
            readback_default_bps: 500,
            ..policy
        },
        ContextBudgetPolicy {
            diagnostic_task_bps: 6000,
            diagnostic_handoff_bps: 4000,
            ..policy
        },
        ContextBudgetPolicy {
            compaction_reference_items: 0,
            ..policy
        },
        ContextBudgetPolicy {
            max_content_bytes: Some(0),
            ..policy
        },
        ContextBudgetPolicy {
            max_core_file_bytes: 0,
            ..policy
        },
    ] {
        assert!(invalid.validate().is_err());
    }
}

#[test]
fn tool_schema_and_dispatch_use_same_configured_readback_bound() {
    let policy = ContextBudgetPolicy {
        readback_default_bps: 50,
        readback_max_bps: 200,
        ..Default::default()
    }
    .for_capabilities(&capabilities(272_000, Some(100_000)))
    .unwrap();
    let tools = ToolRegistry::for_project(".").with_window_policy(policy);
    let spec = tools
        .catalog()
        .into_iter()
        .find(|spec| spec.name.as_str() == "read_output")
        .unwrap();
    assert_eq!(
        spec.input_schema["properties"]["max_bytes"]["maximum"],
        policy.readback_max_bytes()
    );
    assert_eq!(spec.output_policy.max_bytes, policy.tool_output_bytes());
    let call = |limit| {
        ToolCall::new(
            ToolCallId::new("read"),
            ToolName::new("read_output"),
            serde_json::json!({"blob_id":"a".repeat(64), "max_bytes":limit}),
        )
    };
    assert!(
        tools
            .plan(
                ExecutionId::new("accepted"),
                call(policy.readback_max_bytes())
            )
            .is_ok()
    );
    assert!(
        tools
            .plan(
                ExecutionId::new("rejected"),
                call(policy.readback_max_bytes() + 1)
            )
            .is_err()
    );
}

#[test]
fn composition_rejects_invalid_configured_policy_before_runtime() {
    use crate::provider_composition::{ProviderComposition, ProviderCompositionError};
    let root = crate::support::TemporaryDirectory::new("context2-policy-config");
    let path = root.path().join("provider.toml");
    for (settings, valid) in [
        (
            "[runtime.context_budget]\nsummary_bps = 300\nmax_core_file_bytes = 1048576\n",
            true,
        ),
        ("[runtime.context_budget]\nsummary_bps = 0\n", false),
        (
            "[runtime.context_budget]\noutput_reserve_bps = 9000\n",
            false,
        ),
        ("[runtime.context_budget]\nmax_core_file_bytes = 0\n", false),
    ] {
        std::fs::write(
            &path,
            format!("{}\n{settings}", crate::provider_config::fixture()),
        )
        .unwrap();
        let result = ProviderComposition::load_config(&path);
        if valid {
            assert!(result.is_ok());
        } else {
            assert!(matches!(
                result,
                Err(ProviderCompositionError::InvalidContextBudgetPolicy { .. })
            ));
        }
    }
}
