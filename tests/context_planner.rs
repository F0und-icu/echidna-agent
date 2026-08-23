use cyber_agent::domain::artifacts::ArtifactId;
use cyber_agent::domain::context::{
    AppendOnlyCandidate, ContextBudget, ContextContentHash, ContextDecision, ContextFragment,
    ContextFragmentContent, ContextFragmentId, ContextFragmentPriority, ContextFragmentSource,
    ContextPlanner, ContextPlannerError, ContextPlannerVersions, ContextReasonCode,
    FragmentCachePolicy, LogicalAppendOnlyRejection, LogicalBaselineRelation,
};
use cyber_agent::domain::model::{
    CapabilityMappingVersion, ContentItem, ModelCapabilities, ModelRequestRequirements,
};
use cyber_agent::domain::tools::{
    ToolCachePolicy, ToolCall, ToolCallId, ToolDeterminism, ToolEffect, ToolExposure, ToolName,
    ToolOutputPolicy, ToolParallelism, ToolRetryPolicy, ToolSpec,
};
use serde_json::json;

fn capabilities() -> ModelCapabilities {
    ModelCapabilities {
        text_input: true,
        image_input: false,
        streaming: true,
        tool_call: true,
        structured_output: true,
        incremental_input: true,
        usage_reporting: true,
        context_window: 128,
        capability_mapping_version: CapabilityMappingVersion::new(1),
    }
}

fn planner() -> ContextPlanner {
    ContextPlanner::new(ContextPlannerVersions::new(
        "planner:1",
        "estimate:1",
        "budget:1",
    ))
}

fn tool(name: &str) -> ToolSpec {
    ToolSpec {
        name: ToolName::new(name),
        version: "1".to_owned(),
        description: format!("{name} fixture"),
        input_schema: json!({"type": "object", "properties": {}}),
        output_schema: json!({"type": "object"}),
        exposure: ToolExposure::Direct,
        parallelism: ToolParallelism::Parallel,
        resource_key_template: None,
        effect: ToolEffect::ReadOnly,
        retry_policy: ToolRetryPolicy::ReplaySafe,
        determinism: ToolDeterminism::Deterministic,
        cache_policy: ToolCachePolicy::None,
        timeout_ms: 1_000,
        output_policy: ToolOutputPolicy { max_bytes: 1_024 },
    }
}

fn fragment(
    id: &str,
    source: ContextFragmentSource,
    priority: ContextFragmentPriority,
) -> ContextFragment {
    let content = if matches!(
        source,
        ContextFragmentSource::BasePrompt
            | ContextFragmentSource::PersonalPreferences
            | ContextFragmentSource::ProjectPrompt
            | ContextFragmentSource::ModePrompt
    ) {
        ContextFragmentContent::Instructions {
            text: format!("instruction:{id}"),
        }
    } else {
        ContextFragmentContent::ContentItems {
            items: vec![ContentItem::user_text(format!("item:{id}"))],
        }
    };
    ContextFragment::new(
        ContextFragmentId::new(id).unwrap(),
        source,
        priority,
        FragmentCachePolicy::ContentAddressed,
        content,
    )
    .unwrap()
}

fn compile(
    fragments: Vec<ContextFragment>,
    budget: ContextBudget,
) -> cyber_agent::domain::context::ContextPlan {
    planner()
        .compile(
            fragments,
            budget,
            &capabilities(),
            Vec::new(),
            ModelRequestRequirements::default(),
            None,
        )
        .unwrap()
}

fn rendered_channel_tokens(plan: &cyber_agent::domain::context::ContextPlan) -> u32 {
    let input = plan.model_input();
    let instruction_tokens = (input.instructions.len().saturating_add(3) / 4) as u32;
    let content_tokens = (!input.content_items.is_empty())
        .then(|| serde_json::to_vec(&input.content_items).unwrap())
        .map_or(0, |encoded| (encoded.len().saturating_add(3) / 4) as u32);
    instruction_tokens.saturating_add(content_tokens)
}

#[test]
fn planner_preserves_the_twelve_fixed_slots_and_provider_neutral_channels() {
    let sources = vec![
        ContextFragmentSource::BasePrompt,
        ContextFragmentSource::PersonalPreferences,
        ContextFragmentSource::ProjectPrompt,
        ContextFragmentSource::ModePrompt,
        ContextFragmentSource::ResearchWorldState,
        ContextFragmentSource::ProjectSnapshot,
        ContextFragmentSource::ProjectMemoryCore,
        ContextFragmentSource::RetrievedMemory,
        ContextFragmentSource::SessionSummary,
        ContextFragmentSource::RecentHistory,
        ContextFragmentSource::UserInput,
        ContextFragmentSource::TurnLocalUpdates,
    ];
    let fragments = sources
        .iter()
        .enumerate()
        .rev()
        .map(|(index, source)| {
            fragment(
                &format!("slot-{index}"),
                *source,
                ContextFragmentPriority::Normal,
            )
        })
        .collect();
    let plan = compile(fragments, ContextBudget::new(128, 0, 0).unwrap());

    assert_eq!(
        plan.full_logical_manifest()
            .entries()
            .iter()
            .map(|entry| entry.source())
            .collect::<Vec<_>>(),
        sources
    );
    let input = plan.model_input();
    assert_eq!(
        input.instructions,
        "instruction:slot-0\n\ninstruction:slot-1\n\ninstruction:slot-2\n\ninstruction:slot-3"
    );
    assert_eq!(input.content_items.len(), 8);
    assert!(
        input
            .content_items
            .iter()
            .all(|item| matches!(item, ContentItem::Text { .. }))
    );
}

#[test]
fn decisions_explain_include_truncate_artifact_reference_and_exclusion() {
    let artifact = ContextFragment::with_artifact_reference(
        ContextFragmentId::new("artifact").unwrap(),
        ContextFragmentSource::ProjectSnapshot,
        ContextFragmentPriority::High,
        FragmentCachePolicy::ContentAddressed,
        ContextFragmentContent::ContentItems {
            items: vec![ContentItem::user_text("large artifact payload ".repeat(16))],
        },
        Some(ArtifactId::new("artifact:snapshot")),
    )
    .unwrap();
    let base = fragment(
        "base",
        ContextFragmentSource::BasePrompt,
        ContextFragmentPriority::Required,
    );
    let user = fragment(
        "user",
        ContextFragmentSource::UserInput,
        ContextFragmentPriority::Required,
    );
    let truncated = ContextFragment::new(
        ContextFragmentId::new("truncated").unwrap(),
        ContextFragmentSource::RetrievedMemory,
        ContextFragmentPriority::Normal,
        FragmentCachePolicy::ContentAddressed,
        ContextFragmentContent::ContentItems {
            items: vec![ContentItem::user_text(
                "a deliberately long Context Fragment ".repeat(16),
            )],
        },
    )
    .unwrap();
    let excluded = ContextFragment::new(
        ContextFragmentId::new("excluded").unwrap(),
        ContextFragmentSource::SessionSummary,
        ContextFragmentPriority::Low,
        FragmentCachePolicy::ContentAddressed,
        ContextFragmentContent::ContentItems {
            items: vec![ContentItem::user_text(
                "another deliberately long Context Fragment ".repeat(16),
            )],
        },
    )
    .unwrap();
    let artifact_reference = ContextFragment::new(
        ContextFragmentId::new("artifact-reference-equivalent").unwrap(),
        ContextFragmentSource::UserInput,
        ContextFragmentPriority::Normal,
        FragmentCachePolicy::DoNotCache,
        ContextFragmentContent::ContentItems {
            items: vec![ContentItem::user_text(
                "Artifact reference: artifact:snapshot",
            )],
        },
    )
    .unwrap();
    let truncation_budget = 20;
    let input_budget = base
        .token_estimate()
        .saturating_add(user.token_estimate())
        .saturating_add(artifact_reference.token_estimate())
        .saturating_add(truncation_budget);
    let plan = compile(
        vec![base, user, artifact, truncated, excluded],
        ContextBudget::new(input_budget, 0, 0).unwrap(),
    );
    let decisions = plan.context_decision_manifest().candidates();
    assert!(matches!(decisions[0].decision(), ContextDecision::Include));
    assert!(matches!(
        decisions[1].decision(),
        ContextDecision::ArtifactRef { .. }
    ));
    assert!(matches!(
        decisions[2].decision(),
        ContextDecision::Truncate { retained_tokens } if *retained_tokens == truncation_budget
    ));
    assert!(matches!(decisions[3].decision(), ContextDecision::Exclude));
    assert_eq!(
        decisions[0].reason_code(),
        ContextReasonCode::RequiredBaseInstructions
    );
    assert_eq!(
        serde_json::to_value(&decisions[1]).unwrap()["reason_code"],
        "BUDGET_ARTIFACT_REFERENCE"
    );
}

#[test]
fn rendered_budget_handles_utf8_instruction_separators_and_ellipsis() {
    let base = ContextFragment::new(
        ContextFragmentId::new("utf8-base").unwrap(),
        ContextFragmentSource::BasePrompt,
        ContextFragmentPriority::Required,
        FragmentCachePolicy::DoNotCache,
        ContextFragmentContent::Instructions {
            text: "系统检查".to_owned(),
        },
    )
    .unwrap();
    let project = ContextFragment::new(
        ContextFragmentId::new("utf8-project").unwrap(),
        ContextFragmentSource::ProjectPrompt,
        ContextFragmentPriority::Required,
        FragmentCachePolicy::DoNotCache,
        ContextFragmentContent::Instructions {
            text: "仅报告证据".to_owned(),
        },
    )
    .unwrap();
    let memory = ContextFragment::new(
        ContextFragmentId::new("utf8-memory").unwrap(),
        ContextFragmentSource::RetrievedMemory,
        ContextFragmentPriority::Normal,
        FragmentCachePolicy::DoNotCache,
        ContextFragmentContent::ContentItems {
            items: vec![ContentItem::user_text("多字节上下文".repeat(64))],
        },
    )
    .unwrap();
    let truncation_budget = 20;
    let budget = ContextBudget::new(
        base.token_estimate()
            .saturating_add(project.token_estimate())
            .saturating_add(truncation_budget),
        0,
        0,
    )
    .unwrap();

    let plan = compile(vec![memory, project, base], budget);
    assert_eq!(plan.base_instructions(), "系统检查\n\n仅报告证据");
    let ContentItem::Text { text, .. } = &plan.input_items()[0] else {
        panic!("the selected UTF-8 memory Fragment must render as Text");
    };
    assert!(text.starts_with('多'));
    assert!(text.ends_with('…'));
    assert!(matches!(
        plan.context_decision_manifest().candidates()[2].decision(),
        ContextDecision::Truncate { retained_tokens } if *retained_tokens == truncation_budget
    ));
    assert!(rendered_channel_tokens(&plan) <= budget.input_token_limit());
}

#[test]
fn text_and_artifact_reference_wrappers_use_the_rendered_channel_budget() {
    let text = ContextFragment::new(
        ContextFragmentId::new("single-text").unwrap(),
        ContextFragmentSource::UserInput,
        ContextFragmentPriority::Required,
        FragmentCachePolicy::DoNotCache,
        ContextFragmentContent::ContentItems {
            items: vec![ContentItem::user_text("x")],
        },
    )
    .unwrap();
    let text_budget = ContextBudget::new(text.token_estimate(), 0, 0).unwrap();
    let text_plan = compile(vec![text.clone()], text_budget);
    assert_eq!(text_plan.input_items(), &[ContentItem::user_text("x")]);
    assert_eq!(
        text_plan.context_decision_manifest().candidates()[0].token_estimate(),
        text.token_estimate()
    );
    assert!(rendered_channel_tokens(&text_plan) <= text_budget.input_token_limit());

    let artifact_id = ArtifactId::new("artifact:payload");
    let artifact = ContextFragment::with_artifact_reference(
        ContextFragmentId::new("artifact-payload").unwrap(),
        ContextFragmentSource::ProjectSnapshot,
        ContextFragmentPriority::High,
        FragmentCachePolicy::DoNotCache,
        ContextFragmentContent::ContentItems {
            items: vec![ContentItem::user_text("payload ".repeat(64))],
        },
        Some(artifact_id.clone()),
    )
    .unwrap();
    let reference_equivalent = ContextFragment::new(
        ContextFragmentId::new("artifact-wrapper-equivalent").unwrap(),
        ContextFragmentSource::UserInput,
        ContextFragmentPriority::Normal,
        FragmentCachePolicy::DoNotCache,
        ContextFragmentContent::ContentItems {
            items: vec![ContentItem::user_text(format!(
                "Artifact reference: {artifact_id}"
            ))],
        },
    )
    .unwrap();
    let artifact_budget = ContextBudget::new(reference_equivalent.token_estimate(), 0, 0).unwrap();
    let artifact_plan = compile(vec![artifact], artifact_budget);
    assert!(matches!(
        artifact_plan.context_decision_manifest().candidates()[0].decision(),
        ContextDecision::ArtifactRef { artifact_id: selected } if selected == &artifact_id
    ));
    assert_eq!(
        artifact_plan.input_items(),
        &[ContentItem::user_text(format!(
            "Artifact reference: {artifact_id}"
        ))]
    );
    assert!(rendered_channel_tokens(&artifact_plan) <= artifact_budget.input_token_limit());
}

#[test]
fn required_reason_codes_are_not_granted_to_low_priority_prompts() {
    let plan = compile(
        vec![fragment(
            "optional-base",
            ContextFragmentSource::BasePrompt,
            ContextFragmentPriority::Low,
        )],
        ContextBudget::new(16, 0, 0).unwrap(),
    );
    assert_eq!(
        plan.context_decision_manifest().candidates()[0].reason_code(),
        ContextReasonCode::SelectedByPriority
    );
}

#[test]
fn fingerprints_are_stable_for_semantic_order_and_exclude_comparison_history() {
    let first = compile(
        vec![
            fragment(
                "base",
                ContextFragmentSource::BasePrompt,
                ContextFragmentPriority::Required,
            ),
            fragment(
                "user",
                ContextFragmentSource::UserInput,
                ContextFragmentPriority::Required,
            ),
        ],
        ContextBudget::new(32, 0, 0).unwrap(),
    );
    let reversed = compile(
        vec![
            fragment(
                "user",
                ContextFragmentSource::UserInput,
                ContextFragmentPriority::Required,
            ),
            fragment(
                "base",
                ContextFragmentSource::BasePrompt,
                ContextFragmentPriority::Required,
            ),
        ],
        ContextBudget::new(32, 0, 0).unwrap(),
    );
    let compared = planner()
        .compile(
            vec![
                fragment(
                    "base",
                    ContextFragmentSource::BasePrompt,
                    ContextFragmentPriority::Required,
                ),
                fragment(
                    "user",
                    ContextFragmentSource::UserInput,
                    ContextFragmentPriority::Required,
                ),
            ],
            ContextBudget::new(32, 0, 0).unwrap(),
            &capabilities(),
            Vec::new(),
            ModelRequestRequirements::default(),
            Some(first.full_logical_manifest()),
        )
        .unwrap();
    assert_eq!(first.context_fingerprint(), reversed.context_fingerprint());
    assert_eq!(first.context_fingerprint(), compared.context_fingerprint());
    assert!(matches!(
        compared.append_only_candidate(),
        AppendOnlyCandidate::LogicalPrefixDelta {
            retained_prefix_len: 2,
            ..
        }
    ));

    let changed_estimator = ContextPlanner::new(ContextPlannerVersions::new(
        "planner:1",
        "estimate:2",
        "budget:1",
    ))
    .compile(
        vec![
            fragment(
                "base",
                ContextFragmentSource::BasePrompt,
                ContextFragmentPriority::Required,
            ),
            fragment(
                "user",
                ContextFragmentSource::UserInput,
                ContextFragmentPriority::Required,
            ),
        ],
        ContextBudget::new(32, 0, 0).unwrap(),
        &capabilities(),
        Vec::new(),
        ModelRequestRequirements::default(),
        None,
    )
    .unwrap();
    let changed_budget = compile(
        vec![
            fragment(
                "base",
                ContextFragmentSource::BasePrompt,
                ContextFragmentPriority::Required,
            ),
            fragment(
                "user",
                ContextFragmentSource::UserInput,
                ContextFragmentPriority::Required,
            ),
        ],
        ContextBudget::new(34, 1, 0).unwrap(),
    );
    let mut changed_capabilities = capabilities();
    changed_capabilities.capability_mapping_version = CapabilityMappingVersion::new(2);
    let changed_capability = planner()
        .compile(
            vec![
                fragment(
                    "base",
                    ContextFragmentSource::BasePrompt,
                    ContextFragmentPriority::Required,
                ),
                fragment(
                    "user",
                    ContextFragmentSource::UserInput,
                    ContextFragmentPriority::Required,
                ),
            ],
            ContextBudget::new(32, 0, 0).unwrap(),
            &changed_capabilities,
            Vec::new(),
            ModelRequestRequirements::default(),
            None,
        )
        .unwrap();
    assert_ne!(
        first.context_fingerprint(),
        changed_estimator.context_fingerprint()
    );
    assert_ne!(
        first.context_fingerprint(),
        changed_budget.context_fingerprint()
    );
    assert_ne!(
        first.context_fingerprint(),
        changed_capability.context_fingerprint()
    );
}

#[test]
fn logical_append_and_replacement_are_non_destructive_to_the_prior_manifest() {
    let first = compile(
        vec![fragment(
            "base",
            ContextFragmentSource::BasePrompt,
            ContextFragmentPriority::Required,
        )],
        ContextBudget::new(10, 0, 0).unwrap(),
    );
    let historical = first.full_logical_manifest().clone();
    let historical_json = serde_json::to_string(&historical).unwrap();
    let appended = planner()
        .compile(
            vec![
                fragment(
                    "base",
                    ContextFragmentSource::BasePrompt,
                    ContextFragmentPriority::Required,
                ),
                fragment(
                    "update",
                    ContextFragmentSource::TurnLocalUpdates,
                    ContextFragmentPriority::Normal,
                ),
            ],
            ContextBudget::new(10, 0, 0).unwrap(),
            &capabilities(),
            Vec::new(),
            ModelRequestRequirements::default(),
            Some(&historical),
        )
        .unwrap();
    assert_eq!(serde_json::to_string(&historical).unwrap(), historical_json);
    assert!(matches!(
        appended.append_only_candidate(),
        AppendOnlyCandidate::LogicalPrefixDelta {
            retained_prefix_len: 1,
            ..
        }
    ));

    let replaced = planner()
        .compile(
            vec![
                ContextFragment::new(
                    ContextFragmentId::new("base").unwrap(),
                    ContextFragmentSource::BasePrompt,
                    ContextFragmentPriority::Required,
                    FragmentCachePolicy::ContentAddressed,
                    ContextFragmentContent::Instructions {
                        text: "instruction:base changed".to_owned(),
                    },
                )
                .unwrap(),
            ],
            ContextBudget::new(10, 0, 0).unwrap(),
            &capabilities(),
            Vec::new(),
            ModelRequestRequirements::default(),
            Some(&historical),
        )
        .unwrap();
    assert_eq!(
        replaced.append_only_candidate(),
        &AppendOnlyCandidate::NotAppendOnly {
            reason: LogicalAppendOnlyRejection::PriorFragmentChanged
        }
    );
}

#[test]
fn fragment_ids_are_unique_across_slots_and_channels_are_explicit() {
    let duplicate = planner().compile(
        vec![
            fragment(
                "same",
                ContextFragmentSource::BasePrompt,
                ContextFragmentPriority::Required,
            ),
            fragment(
                "same",
                ContextFragmentSource::UserInput,
                ContextFragmentPriority::Required,
            ),
        ],
        ContextBudget::new(10, 0, 0).unwrap(),
        &capabilities(),
        Vec::new(),
        ModelRequestRequirements::default(),
        None,
    );
    assert!(matches!(
        duplicate,
        Err(ContextPlannerError::DuplicateFragmentId { .. })
    ));
    assert_eq!(
        ContextFragmentId::new(""),
        Err(ContextPlannerError::EmptyFragmentId)
    );
    assert!(matches!(
        ContextFragment::new(
            ContextFragmentId::new("bad-channel").unwrap(),
            ContextFragmentSource::UserInput,
            ContextFragmentPriority::Required,
            FragmentCachePolicy::DoNotCache,
            ContextFragmentContent::Instructions {
                text: "not an instruction slot".to_owned(),
            },
        ),
        Err(ContextPlannerError::InvalidFragmentChannel { .. })
    ));
    assert!(matches!(
        ContextFragment::with_artifact_reference(
            ContextFragmentId::new("instruction-artifact").unwrap(),
            ContextFragmentSource::BasePrompt,
            ContextFragmentPriority::Normal,
            FragmentCachePolicy::ContentAddressed,
            ContextFragmentContent::Instructions {
                text: "base".to_owned(),
            },
            Some(ArtifactId::new("artifact:base")),
        ),
        Err(ContextPlannerError::InstructionArtifactReference { .. })
    ));
    assert!(serde_json::from_str::<ContextFragmentId>(r#""""#).is_err());
    assert!(serde_json::from_str::<ContextContentHash>(r#""forged""#).is_err());
}

#[test]
fn required_fragments_fail_the_plan_instead_of_being_degraded() {
    let required = ContextFragment::with_artifact_reference(
        ContextFragmentId::new("required-user").unwrap(),
        ContextFragmentSource::UserInput,
        ContextFragmentPriority::Required,
        FragmentCachePolicy::ContentAddressed,
        ContextFragmentContent::ContentItems {
            items: vec![ContentItem::user_text("required")],
        },
        Some(ArtifactId::new("artifact:required")),
    )
    .unwrap();

    let required_tokens = required.token_estimate();
    let input_token_limit = required_tokens - 1;
    assert_eq!(
        planner().compile(
            vec![required],
            ContextBudget::new(required_tokens, 1, 0).unwrap(),
            &capabilities(),
            Vec::new(),
            ModelRequestRequirements::default(),
            None,
        ),
        Err(ContextPlannerError::RequiredFragmentsExceedBudget {
            required_tokens: u64::from(required_tokens),
            input_token_limit,
        })
    );
}

#[test]
fn tool_order_is_canonical_and_excluded_fragments_are_not_replacements() {
    let base = fragment(
        "base",
        ContextFragmentSource::BasePrompt,
        ContextFragmentPriority::Required,
    );
    let base_only_budget = ContextBudget::new(base.token_estimate(), 0, 0).unwrap();
    let first = compile(vec![base.clone()], base_only_budget);
    let compile_with_tools = |tool_specs| {
        planner()
            .compile(
                vec![fragment(
                    "base",
                    ContextFragmentSource::BasePrompt,
                    ContextFragmentPriority::Required,
                )],
                ContextBudget::new(128, 0, 0).unwrap(),
                &capabilities(),
                tool_specs,
                ModelRequestRequirements::default(),
                None,
            )
            .unwrap()
    };
    let forward = compile_with_tools(vec![tool("alpha"), tool("beta")]);
    let reverse = compile_with_tools(vec![tool("beta"), tool("alpha")]);

    assert_eq!(forward.context_fingerprint(), reverse.context_fingerprint());
    assert_eq!(forward.model_input(), reverse.model_input());
    assert_eq!(
        forward
            .tool_specs()
            .iter()
            .map(|spec| spec.name.as_str())
            .collect::<Vec<_>>(),
        vec!["alpha", "beta"]
    );

    let with_excluded = planner()
        .compile(
            vec![
                base,
                fragment(
                    "excluded",
                    ContextFragmentSource::RetrievedMemory,
                    ContextFragmentPriority::Low,
                ),
            ],
            base_only_budget,
            &capabilities(),
            Vec::new(),
            ModelRequestRequirements::default(),
            Some(first.full_logical_manifest()),
        )
        .unwrap();
    let excluded = with_excluded
        .context_decision_manifest()
        .candidates()
        .iter()
        .find(|entry| entry.fragment_id().as_str() == "excluded")
        .unwrap();
    assert_eq!(
        excluded.baseline_relation(),
        &LogicalBaselineRelation::Excluded
    );
}

#[test]
fn budget_reserves_fixed_input_and_never_exceeds_the_model_window() {
    let budget = ContextBudget::new(128, 16, 12).unwrap();
    assert_eq!(budget.input_token_limit(), 100);
    assert_eq!(budget.reserved_tool_loop_tokens(), 12);
    assert!(matches!(
        ContextBudget::new(10, 8, 3),
        Err(ContextPlannerError::InvalidBudget { .. })
    ));
    assert!(matches!(
        planner().compile(
            Vec::new(),
            ContextBudget::new(129, 0, 0).unwrap(),
            &capabilities(),
            Vec::new(),
            ModelRequestRequirements::default(),
            None,
        ),
        Err(ContextPlannerError::BudgetExceedsModelContext { .. })
    ));
    assert!(matches!(
        planner().compile(
            Vec::new(),
            ContextBudget::new(1, 0, 0).unwrap(),
            &capabilities(),
            vec![tool("schema-is-not-free")],
            ModelRequestRequirements::default(),
            None,
        ),
        Err(ContextPlannerError::FixedInputExceedsBudget { .. })
    ));

    let planned = planner()
        .compile(
            Vec::new(),
            budget,
            &capabilities(),
            vec![tool("alpha")],
            ModelRequestRequirements::default(),
            None,
        )
        .unwrap();
    assert!(planned.context_decision_manifest().fixed_input_tokens() > 0);
    assert!(planned.requirements().tool_call);
    assert!(planned.model_input().requirements.tool_call);
}

#[test]
fn planner_reuses_history_pairing_validation_for_external_content_items() {
    let unpaired = ContextFragment::new(
        ContextFragmentId::new("unpaired-history").unwrap(),
        ContextFragmentSource::RecentHistory,
        ContextFragmentPriority::Required,
        FragmentCachePolicy::DoNotCache,
        ContextFragmentContent::ContentItems {
            items: vec![ContentItem::ToolCall {
                call: ToolCall::new(
                    ToolCallId::new("call:unpaired"),
                    ToolName::new("alpha"),
                    json!({}),
                ),
            }],
        },
    )
    .unwrap();
    assert!(matches!(
        planner().compile(
            vec![unpaired],
            ContextBudget::new(128, 0, 0).unwrap(),
            &capabilities(),
            vec![tool("alpha")],
            ModelRequestRequirements::default(),
            None,
        ),
        Err(ContextPlannerError::InvalidHistoryPairing { .. })
    ));
}
