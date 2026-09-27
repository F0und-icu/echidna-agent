use std::collections::BTreeMap;

use echidna::domain::context::{
    BaselineAssessment, BaselineInvalidationReason, ContextError, InternalRequestMetadata,
    ModelIdentity, ModelProfile, ProviderSession, ProviderSessionId, ProviderSessionRef,
    ProviderSessionRole, StepSnapshot, TurnContext, TurnId,
};
use echidna::domain::model::{
    CapabilityMappingVersion, ContentItem, ModelCapabilities, ModelInputIR,
    ModelInputIrSchemaVersion, ModelRequestRequirements,
};
use echidna::domain::tools::{
    ToolCachePolicy, ToolCall, ToolCallId, ToolDeterminism, ToolEffect, ToolExposure, ToolName,
    ToolOutputPolicy, ToolParallelism, ToolResult, ToolResultStatus, ToolRetryPolicy, ToolSpec,
};
use serde_json::{Value, json};

fn capabilities() -> ModelCapabilities {
    ModelCapabilities {
        text_input: true,
        image_input: false,
        streaming: true,
        tool_call: true,
        structured_output: true,
        incremental_input: true,
        usage_reporting: true,
        max_output_tokens: None,
        context_window: 128_000,
        capability_mapping_version: CapabilityMappingVersion::new(1),
    }
}

fn tool_spec(name: &str, input_schema: Value) -> ToolSpec {
    ToolSpec {
        name: ToolName::new(name),
        version: "1".to_owned(),
        description: format!("{name} fixture"),
        input_schema,
        output_schema: json!({"type": "object", "properties": {"result": {"type": "string"}}}),
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

fn request(
    instructions: &str,
    items: Vec<ContentItem>,
    tool_specs: Vec<ToolSpec>,
    requirements: ModelRequestRequirements,
) -> ModelInputIR {
    ModelInputIR::current(instructions, items, requirements).with_tool_specs(tool_specs)
}

fn turn(
    model: ModelIdentity,
    profile: ModelProfile,
    session_id: &str,
    session_role: ProviderSessionRole,
    tool_specs: &[ToolSpec],
) -> TurnContext {
    TurnContext::new(
        TurnId::new("turn-1"),
        model,
        profile,
        ProviderSessionRef {
            id: ProviderSessionId::new(session_id),
            role: session_role,
            generation: 1,
        },
        128_000,
        tool_specs,
    )
    .unwrap()
}

fn snapshot(
    model: ModelIdentity,
    profile: ModelProfile,
    session_id: &str,
    session_role: ProviderSessionRole,
    input: &ModelInputIR,
    capabilities: &ModelCapabilities,
    metadata: InternalRequestMetadata,
) -> StepSnapshot {
    StepSnapshot::capture(
        &turn(model, profile, session_id, session_role, &input.tool_specs),
        0,
        input,
        capabilities,
        metadata,
    )
    .unwrap()
}

fn primary_session() -> ProviderSession {
    ProviderSession::new(
        ProviderSessionId::new("primary-1"),
        ProviderSessionRole::Primary,
        1,
    )
}

fn establish(
    session: &mut ProviderSession,
    snapshot: &StepSnapshot,
    server_output: Vec<ContentItem>,
) {
    assert_eq!(
        session.assess_delivery(snapshot, true),
        BaselineAssessment::FullRequest {
            reason: BaselineInvalidationReason::NoBaseline
        }
    );
    session
        .record_completed_sample(snapshot, server_output)
        .unwrap();
}

fn baseline_input() -> (ModelInputIR, Vec<ContentItem>) {
    let tools = vec![tool_spec(
        "read_file",
        json!({"required": ["path"], "type": "object", "properties": {"path": {"type": "string"}}}),
    )];
    (
        request(
            "prompt-v1",
            vec![ContentItem::user_text("inspect manifest")],
            tools,
            ModelRequestRequirements::default(),
        ),
        vec![ContentItem::assistant_text("I will inspect it.")],
    )
}

fn append_input(baseline: &ModelInputIR, server_output: &[ContentItem]) -> ModelInputIR {
    let mut items = baseline.content_items.clone();
    items.extend_from_slice(server_output);
    items.push(ContentItem::user_text("continue"));
    request(
        &baseline.instructions,
        items,
        baseline.tool_specs.clone(),
        baseline.requirements,
    )
}

#[test]
fn output_limit_changes_invalidate_the_output_contract_without_rewriting_history() {
    let mut caps = capabilities();
    caps.max_output_tokens = Some(8192);
    let (mut input, output) = baseline_input();
    input.requirements.max_output_tokens = Some(4096);
    let capture = |input: &ModelInputIR| {
        snapshot(
            ModelIdentity::new("primary", "fixture"),
            ModelProfile::new("configured", 1),
            "primary-1",
            ProviderSessionRole::Primary,
            input,
            &caps,
            InternalRequestMetadata::default(),
        )
    };
    let mut session = primary_session();
    establish(&mut session, &capture(&input), output.clone());
    let mut next = append_input(&input, &output);
    next.requirements.max_output_tokens = Some(2048);
    assert_eq!(
        session.assess_delivery(&capture(&next), true),
        BaselineAssessment::FullRequest {
            reason: BaselineInvalidationReason::OutputContractChanged,
        }
    );
}

#[test]
fn invalidation_matrix_separates_model_profile_and_uses_stable_priority() {
    #[derive(Clone, Copy)]
    enum Change {
        Model,
        Profile,
        IrSchema,
        Capability,
        Prompt,
        Tool,
        Output,
        Properties,
    }

    let cases = [
        (Change::Model, BaselineInvalidationReason::ModelChanged),
        (Change::Profile, BaselineInvalidationReason::ProfileChanged),
        (
            Change::IrSchema,
            BaselineInvalidationReason::ModelInputIrSchemaChanged,
        ),
        (
            Change::Capability,
            BaselineInvalidationReason::CapabilityMappingChanged,
        ),
        (Change::Prompt, BaselineInvalidationReason::PromptChanged),
        (Change::Tool, BaselineInvalidationReason::ToolsChanged),
        (
            Change::Output,
            BaselineInvalidationReason::OutputContractChanged,
        ),
        (
            Change::Properties,
            BaselineInvalidationReason::RequestPropertiesChanged,
        ),
    ];

    for (change, expected_reason) in cases {
        let (baseline_request, server_output) = baseline_input();
        let baseline_model = ModelIdentity::new("primary", "model-v1");
        let baseline_profile = ModelProfile::new("research", 1);
        let baseline_snapshot = snapshot(
            baseline_model.clone(),
            baseline_profile.clone(),
            "primary-1",
            ProviderSessionRole::Primary,
            &baseline_request,
            &capabilities(),
            InternalRequestMetadata::default(),
        );
        let mut session = primary_session();
        establish(&mut session, &baseline_snapshot, server_output.clone());
        let mut candidate_request = append_input(&baseline_request, &server_output);
        let mut candidate_model = baseline_model;
        let mut candidate_profile = baseline_profile;
        let mut candidate_capabilities = capabilities();

        match change {
            Change::Model => candidate_model = ModelIdentity::new("primary", "model-v2"),
            Change::Profile => candidate_profile = ModelProfile::new("fast", 2),
            Change::IrSchema => {
                candidate_request.schema_version = ModelInputIrSchemaVersion::new(99)
            }
            Change::Capability => {
                candidate_capabilities.capability_mapping_version =
                    CapabilityMappingVersion::new(2);
            }
            Change::Prompt => candidate_request.instructions = "prompt-v2".to_owned(),
            Change::Tool => candidate_request.tool_specs[0].version = "2".to_owned(),
            Change::Output => candidate_request.requirements.structured_output = true,
            Change::Properties => candidate_request.requirements.usage_reporting = true,
        }
        let candidate = snapshot(
            candidate_model,
            candidate_profile,
            "primary-1",
            ProviderSessionRole::Primary,
            &candidate_request,
            &candidate_capabilities,
            InternalRequestMetadata::default(),
        );
        assert_eq!(
            session.assess_delivery(&candidate, true),
            BaselineAssessment::FullRequest {
                reason: expected_reason
            }
        );
    }

    let (baseline_request, server_output) = baseline_input();
    let baseline_snapshot = snapshot(
        ModelIdentity::new("primary", "model-v1"),
        ModelProfile::new("research", 1),
        "primary-1",
        ProviderSessionRole::Primary,
        &baseline_request,
        &capabilities(),
        InternalRequestMetadata::default(),
    );
    let mut session = primary_session();
    establish(&mut session, &baseline_snapshot, server_output.clone());
    let mut input = append_input(&baseline_request, &server_output);
    input.content_items[0] = ContentItem::user_text("rewritten history");
    input.instructions = "prompt-v2".to_owned();
    input.tool_specs[0].version = "2".to_owned();
    input.requirements.structured_output = true;
    input.requirements.usage_reporting = true;
    input.schema_version = ModelInputIrSchemaVersion::new(99);
    let mut model = ModelIdentity::new("primary", "model-v2");
    let mut profile = ModelProfile::new("fast", 2);
    let mut mapping = capabilities();
    mapping.capability_mapping_version = CapabilityMappingVersion::new(2);

    let expected = [
        BaselineInvalidationReason::ModelChanged,
        BaselineInvalidationReason::ProfileChanged,
        BaselineInvalidationReason::ModelInputIrSchemaChanged,
        BaselineInvalidationReason::CapabilityMappingChanged,
        BaselineInvalidationReason::PromptChanged,
        BaselineInvalidationReason::ToolsChanged,
        BaselineInvalidationReason::OutputContractChanged,
        BaselineInvalidationReason::RequestPropertiesChanged,
        BaselineInvalidationReason::InputNotAppendOnly,
    ];
    for (step, reason) in expected.into_iter().enumerate() {
        let candidate = snapshot(
            model.clone(),
            profile.clone(),
            "primary-1",
            ProviderSessionRole::Primary,
            &input,
            &mapping,
            InternalRequestMetadata::default(),
        );
        assert_eq!(
            session.assess_delivery(&candidate, true),
            BaselineAssessment::FullRequest { reason }
        );
        if step < 8 {
            match step {
                0 => model = ModelIdentity::new("primary", "model-v1"),
                1 => profile = ModelProfile::new("research", 1),
                2 => input.schema_version = ModelInputIrSchemaVersion::new(3),
                3 => mapping.capability_mapping_version = CapabilityMappingVersion::new(1),
                4 => input.instructions = "prompt-v1".to_owned(),
                5 => input.tool_specs[0].version = "1".to_owned(),
                6 => input.requirements.structured_output = false,
                7 => input.requirements.usage_reporting = false,
                _ => unreachable!("the priority table is exhaustive"),
            }
        }
    }
}

#[test]
fn canonical_tool_semantics_are_stable_and_exclude_local_execution_metadata() {
    let alpha = tool_spec(
        "alpha",
        serde_json::from_str(
            r#"{"type":"object","properties":{"b":{"type":"string"},"a":{"type":"number"}}}"#,
        )
        .unwrap(),
    );
    let zeta = tool_spec(
        "zeta",
        serde_json::from_str(
            r#"{"properties":{"a":{"type":"number"},"b":{"type":"string"}},"type":"object"}"#,
        )
        .unwrap(),
    );
    let first_turn = turn(
        ModelIdentity::new("primary", "model-v1"),
        ModelProfile::new("research", 1),
        "primary-1",
        ProviderSessionRole::Primary,
        &[zeta.clone(), alpha.clone()],
    );
    let second_turn = turn(
        ModelIdentity::new("primary", "model-v1"),
        ModelProfile::new("research", 1),
        "primary-1",
        ProviderSessionRole::Primary,
        &[alpha.clone(), zeta.clone()],
    );
    assert_eq!(
        first_turn.tool_exposure_hash,
        second_turn.tool_exposure_hash
    );

    let mut local_only = alpha.clone();
    local_only.timeout_ms = 9_999;
    local_only.cache_policy = ToolCachePolicy::ContentAddressed;
    local_only.retry_policy = ToolRetryPolicy::NeverAutoRetry;
    local_only.effect = ToolEffect::Interactive;
    local_only.parallelism = ToolParallelism::Serial;
    local_only.resource_key_template = Some("project:{root}".to_owned());
    local_only.determinism = ToolDeterminism::Nondeterministic;
    local_only.output_policy = ToolOutputPolicy { max_bytes: 2_048 };
    let baseline = request(
        "prompt-v1",
        vec![ContentItem::user_text("inspect")],
        vec![alpha.clone()],
        ModelRequestRequirements::default(),
    );
    let output = vec![ContentItem::assistant_text("observed")];
    let baseline_snapshot = snapshot(
        ModelIdentity::new("primary", "model-v1"),
        ModelProfile::new("research", 1),
        "primary-1",
        ProviderSessionRole::Primary,
        &baseline,
        &capabilities(),
        InternalRequestMetadata::default(),
    );
    let mut session = primary_session();
    establish(&mut session, &baseline_snapshot, output.clone());
    let candidate = request(
        "prompt-v1",
        vec![
            ContentItem::user_text("inspect"),
            ContentItem::assistant_text("observed"),
            ContentItem::user_text("continue"),
        ],
        vec![local_only],
        ModelRequestRequirements::default(),
    );
    let candidate = snapshot(
        ModelIdentity::new("primary", "model-v1"),
        ModelProfile::new("research", 1),
        "primary-1",
        ProviderSessionRole::Primary,
        &candidate,
        &capabilities(),
        InternalRequestMetadata::default(),
    );
    assert!(matches!(
        session.assess_delivery(&candidate, true),
        BaselineAssessment::AppendOnlyCandidate { .. }
    ));

    for changed in [
        ToolSpec {
            exposure: ToolExposure::Deferred,
            ..alpha.clone()
        },
        ToolSpec {
            input_schema: json!({"type": "object", "required": ["other"]}),
            ..alpha
        },
    ] {
        let request = request(
            "prompt-v1",
            vec![
                ContentItem::user_text("inspect"),
                ContentItem::assistant_text("observed"),
                ContentItem::user_text("continue"),
            ],
            vec![changed],
            ModelRequestRequirements::default(),
        );
        let candidate = snapshot(
            ModelIdentity::new("primary", "model-v1"),
            ModelProfile::new("research", 1),
            "primary-1",
            ProviderSessionRole::Primary,
            &request,
            &capabilities(),
            InternalRequestMetadata::default(),
        );
        assert_eq!(
            session.assess_delivery(&candidate, true),
            BaselineAssessment::FullRequest {
                reason: BaselineInvalidationReason::ToolsChanged
            }
        );
    }
}

#[test]
fn metadata_is_excluded_but_history_order_pair_identity_and_captured_turn_state_remain_semantic() {
    let (baseline_request, server_output) = baseline_input();
    let context = turn(
        ModelIdentity::new("primary", "model-v1"),
        ModelProfile::new("research", 1),
        "primary-1",
        ProviderSessionRole::Primary,
        &baseline_request.tool_specs,
    );
    let first = StepSnapshot::capture(
        &context,
        0,
        &baseline_request,
        &capabilities(),
        InternalRequestMetadata {
            local_request_id: Some("request-a".to_owned()),
            local_timestamp: Some("2026-08-22T00:00:00Z".to_owned()),
            annotations: BTreeMap::from([("trace".to_owned(), "one".to_owned())]),
        },
    )
    .unwrap();
    assert_eq!(first.context_budget, context.context_budget);
    assert_eq!(first.tool_exposure_hash, context.tool_exposure_hash);
    let mut session = primary_session();
    establish(&mut session, &first, server_output.clone());
    let appended = append_input(&baseline_request, &server_output);
    let second = snapshot(
        ModelIdentity::new("primary", "model-v1"),
        ModelProfile::new("research", 1),
        "primary-1",
        ProviderSessionRole::Primary,
        &appended,
        &capabilities(),
        InternalRequestMetadata {
            local_request_id: Some("request-b".to_owned()),
            local_timestamp: Some("2026-08-22T01:00:00Z".to_owned()),
            annotations: BTreeMap::from([("trace".to_owned(), "two".to_owned())]),
        },
    );
    assert_eq!(
        first.request,
        StepSnapshot::capture(
            &context,
            0,
            &baseline_request,
            &capabilities(),
            InternalRequestMetadata::default(),
        )
        .unwrap()
        .request
    );
    assert_eq!(
        session.assess_delivery(&second, true),
        BaselineAssessment::AppendOnlyCandidate {
            delta_items: vec![ContentItem::user_text("continue")]
        }
    );

    let call = ToolCall::new(
        ToolCallId::new("call-1"),
        ToolName::new("read_file"),
        serde_json::from_str(r#"{"b":{"y":2,"x":1},"a":0}"#).unwrap(),
    );
    let result = ToolResult {
        call_id: ToolCallId::new("call-1"),
        tool_name: ToolName::new("read_file"),
        status: ToolResultStatus::Succeeded,
        output: serde_json::from_str(r#"{"result":{"b":2,"a":1}}"#).unwrap(),
    };
    let paired = request(
        "prompt-v1",
        vec![
            ContentItem::user_text("read it"),
            ContentItem::ToolCall { call: call.clone() },
            ContentItem::ToolResult {
                result: result.clone(),
            },
        ],
        Vec::new(),
        ModelRequestRequirements::default(),
    );
    let paired_snapshot = snapshot(
        ModelIdentity::new("primary", "model-v1"),
        ModelProfile::new("research", 1),
        "primary-1",
        ProviderSessionRole::Primary,
        &paired,
        &capabilities(),
        InternalRequestMetadata::default(),
    );
    let mut paired_session = primary_session();
    establish(&mut paired_session, &paired_snapshot, Vec::new());
    let equivalent = request(
        "prompt-v1",
        vec![
            ContentItem::user_text("read it"),
            ContentItem::ToolCall {
                call: ToolCall::new(
                    ToolCallId::new("call-1"),
                    ToolName::new("read_file"),
                    serde_json::from_str(r#"{"a":0,"b":{"x":1,"y":2}}"#).unwrap(),
                ),
            },
            ContentItem::ToolResult {
                result: ToolResult {
                    call_id: ToolCallId::new("call-1"),
                    tool_name: ToolName::new("read_file"),
                    status: ToolResultStatus::Succeeded,
                    output: serde_json::from_str(r#"{"result":{"a":1,"b":2}}"#).unwrap(),
                },
            },
            ContentItem::user_text("continue"),
        ],
        Vec::new(),
        ModelRequestRequirements::default(),
    );
    let equivalent = snapshot(
        ModelIdentity::new("primary", "model-v1"),
        ModelProfile::new("research", 1),
        "primary-1",
        ProviderSessionRole::Primary,
        &equivalent,
        &capabilities(),
        InternalRequestMetadata::default(),
    );
    assert!(matches!(
        paired_session.assess_delivery(&equivalent, true),
        BaselineAssessment::AppendOnlyCandidate { .. }
    ));

    let changed_id = request(
        "prompt-v1",
        vec![
            ContentItem::user_text("read it"),
            ContentItem::ToolCall {
                call: ToolCall::new(
                    ToolCallId::new("call-2"),
                    ToolName::new("read_file"),
                    json!({"a": 0, "b": {"x": 1, "y": 2}}),
                ),
            },
            ContentItem::ToolResult {
                result: ToolResult {
                    call_id: ToolCallId::new("call-2"),
                    ..result
                },
            },
            ContentItem::user_text("continue"),
        ],
        Vec::new(),
        ModelRequestRequirements::default(),
    );
    let changed_id = snapshot(
        ModelIdentity::new("primary", "model-v1"),
        ModelProfile::new("research", 1),
        "primary-1",
        ProviderSessionRole::Primary,
        &changed_id,
        &capabilities(),
        InternalRequestMetadata::default(),
    );
    assert_eq!(
        paired_session.assess_delivery(&changed_id, true),
        BaselineAssessment::FullRequest {
            reason: BaselineInvalidationReason::InputNotAppendOnly
        }
    );
}

#[test]
fn modified_deleted_or_reordered_history_is_not_append_only() {
    let call = ToolCall::new(
        ToolCallId::new("call-1"),
        ToolName::new("read_file"),
        json!({"path": "Cargo.toml"}),
    );
    let result = ToolResult {
        call_id: ToolCallId::new("call-1"),
        tool_name: ToolName::new("read_file"),
        status: ToolResultStatus::Succeeded,
        output: json!({"content": "one"}),
    };
    let baseline = request(
        "prompt-v1",
        vec![
            ContentItem::user_text("read it"),
            ContentItem::ToolCall { call: call.clone() },
            ContentItem::ToolResult {
                result: result.clone(),
            },
        ],
        Vec::new(),
        ModelRequestRequirements::default(),
    );
    let baseline_snapshot = snapshot(
        ModelIdentity::new("primary", "model-v1"),
        ModelProfile::new("research", 1),
        "primary-1",
        ProviderSessionRole::Primary,
        &baseline,
        &capabilities(),
        InternalRequestMetadata::default(),
    );
    let mut session = primary_session();
    establish(&mut session, &baseline_snapshot, Vec::new());
    let cases = [
        vec![
            ContentItem::user_text("read it"),
            ContentItem::ToolCall {
                call: ToolCall::new(
                    ToolCallId::new("call-1"),
                    ToolName::new("read_file"),
                    json!({"path": "src/lib.rs"}),
                ),
            },
            ContentItem::ToolResult {
                result: result.clone(),
            },
            ContentItem::user_text("continue"),
        ],
        vec![
            ContentItem::user_text("read it"),
            ContentItem::ToolCall { call: call.clone() },
            ContentItem::user_text("continue"),
        ],
        vec![
            ContentItem::user_text("read it"),
            ContentItem::ToolResult { result },
            ContentItem::ToolCall { call },
            ContentItem::user_text("continue"),
        ],
    ];
    for items in cases {
        let candidate = request(
            "prompt-v1",
            items,
            Vec::new(),
            ModelRequestRequirements::default(),
        );
        let candidate = snapshot(
            ModelIdentity::new("primary", "model-v1"),
            ModelProfile::new("research", 1),
            "primary-1",
            ProviderSessionRole::Primary,
            &candidate,
            &capabilities(),
            InternalRequestMetadata::default(),
        );
        assert_eq!(
            session.assess_delivery(&candidate, true),
            BaselineAssessment::FullRequest {
                reason: BaselineInvalidationReason::InputNotAppendOnly
            }
        );
    }
}

#[test]
fn reset_assessment_is_repeatable_until_success_and_session_identity_cannot_cross() {
    let (baseline, output) = baseline_input();
    let baseline_snapshot = snapshot(
        ModelIdentity::new("primary", "model-v1"),
        ModelProfile::new("research", 1),
        "primary-1",
        ProviderSessionRole::Primary,
        &baseline,
        &capabilities(),
        InternalRequestMetadata::default(),
    );
    let candidate_request = append_input(&baseline, &output);
    let candidate = snapshot(
        ModelIdentity::new("primary", "model-v1"),
        ModelProfile::new("research", 1),
        "primary-1",
        ProviderSessionRole::Primary,
        &candidate_request,
        &capabilities(),
        InternalRequestMetadata::default(),
    );
    let mut session = primary_session();
    establish(&mut session, &baseline_snapshot, output.clone());
    session.reset(BaselineInvalidationReason::CompactionReset);
    session.reset(BaselineInvalidationReason::ToolsChanged);
    for _ in 0..2 {
        assert_eq!(
            session.assess_delivery(&candidate, true),
            BaselineAssessment::FullRequest {
                reason: BaselineInvalidationReason::CompactionReset
            }
        );
    }
    session
        .record_completed_sample(
            &candidate,
            vec![ContentItem::assistant_text("full replacement")],
        )
        .unwrap();
    let next = request(
        "prompt-v1",
        vec![
            ContentItem::user_text("inspect manifest"),
            ContentItem::assistant_text("I will inspect it."),
            ContentItem::user_text("continue"),
            ContentItem::assistant_text("full replacement"),
            ContentItem::user_text("next"),
        ],
        baseline.tool_specs.clone(),
        ModelRequestRequirements::default(),
    );
    let next = snapshot(
        ModelIdentity::new("primary", "model-v1"),
        ModelProfile::new("research", 1),
        "primary-1",
        ProviderSessionRole::Primary,
        &next,
        &capabilities(),
        InternalRequestMetadata::default(),
    );
    assert!(matches!(
        session.assess_delivery(&next, true),
        BaselineAssessment::AppendOnlyCandidate { .. }
    ));

    let vision = snapshot(
        ModelIdentity::new("vision", "model-v1"),
        ModelProfile::new("vision", 1),
        "vision-1",
        ProviderSessionRole::Vision,
        &candidate_request,
        &capabilities(),
        InternalRequestMetadata::default(),
    );
    assert_eq!(
        session.record_completed_sample(&vision, output),
        Err(ContextError::ProviderSessionMismatch {
            expected: ProviderSessionRef {
                id: ProviderSessionId::new("primary-1"),
                role: ProviderSessionRole::Primary,
                generation: 1,
            },
            received: vision.provider_session.clone(),
        })
    );

    let stable_codes = [
        (BaselineInvalidationReason::NoBaseline, "NO_BASELINE"),
        (
            BaselineInvalidationReason::ProviderSessionChanged,
            "PROVIDER_SESSION_CHANGED",
        ),
        (BaselineInvalidationReason::ModelChanged, "MODEL_CHANGED"),
        (
            BaselineInvalidationReason::ProfileChanged,
            "PROFILE_CHANGED",
        ),
        (
            BaselineInvalidationReason::ModelInputIrSchemaChanged,
            "MODEL_INPUT_IR_SCHEMA_CHANGED",
        ),
        (
            BaselineInvalidationReason::CapabilityMappingChanged,
            "CAPABILITY_MAPPING_CHANGED",
        ),
        (BaselineInvalidationReason::PromptChanged, "PROMPT_CHANGED"),
        (BaselineInvalidationReason::ToolsChanged, "TOOLS_CHANGED"),
        (
            BaselineInvalidationReason::OutputContractChanged,
            "OUTPUT_CONTRACT_CHANGED",
        ),
        (
            BaselineInvalidationReason::RequestPropertiesChanged,
            "REQUEST_PROPERTIES_CHANGED",
        ),
        (
            BaselineInvalidationReason::InputNotAppendOnly,
            "INPUT_NOT_APPEND_ONLY",
        ),
        (
            BaselineInvalidationReason::CompactionReset,
            "COMPACTION_RESET",
        ),
        (
            BaselineInvalidationReason::SessionResumed,
            "SESSION_RESUMED",
        ),
        (
            BaselineInvalidationReason::ProviderRejected,
            "PROVIDER_REJECTED",
        ),
    ];
    for (reason, expected_code) in stable_codes {
        assert_eq!(serde_json::to_value(reason).unwrap(), json!(expected_code));
    }
}

#[test]
fn append_only_history_stays_full_when_incremental_delivery_is_not_declared() {
    let (baseline_request, server_output) = baseline_input();
    let baseline_snapshot = snapshot(
        ModelIdentity::new("primary", "model-v1"),
        ModelProfile::new("research", 1),
        "primary-1",
        ProviderSessionRole::Primary,
        &baseline_request,
        &capabilities(),
        InternalRequestMetadata::default(),
    );
    let candidate_request = append_input(&baseline_request, &server_output);
    let candidate = snapshot(
        ModelIdentity::new("primary", "model-v1"),
        ModelProfile::new("research", 1),
        "primary-1",
        ProviderSessionRole::Primary,
        &candidate_request,
        &capabilities(),
        InternalRequestMetadata::default(),
    );
    let mut session = primary_session();
    establish(&mut session, &baseline_snapshot, server_output);

    assert_eq!(
        session.assess_delivery(&candidate, false),
        BaselineAssessment::FullRequest {
            reason: BaselineInvalidationReason::IncrementalInputUnsupported,
        }
    );
    assert!(matches!(
        session.assess_delivery(&candidate, true),
        BaselineAssessment::AppendOnlyCandidate { .. }
    ));
    assert_eq!(
        serde_json::to_value(BaselineInvalidationReason::IncrementalInputUnsupported).unwrap(),
        json!("INCREMENTAL_INPUT_UNSUPPORTED")
    );
}
