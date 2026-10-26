//! Existing RSI trigger rules, now owned by App rather than CLI composition.
use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::app::improvement::{
    ImprovementReadiness, completed_boundary_is_eligible, experience_status, policy, readiness,
};
use crate::app::model_request_budget::ModelRequestBudget;
use crate::domain::agent_definition::{
    AgentConfiguredContext, AgentContextSelection, AgentDefinition, AgentDefinitionBinding,
    AgentId, AgentInstructions, AgentMemoryScope, AgentToolExposure, BuiltinAgentTemplate,
    DefinitionSourceLocator, InstructionSnapshot, InstructionSourceLocator,
};
use crate::domain::agent_evolution::{AgentRsi, EvolutionExperience, EvolutionState};
use crate::domain::context::{
    ContextBudget, ContextPlanner, ProviderSessionId, ProviderSessionRef, ProviderSessionRole,
};
use crate::domain::events::{EventEnvelope, EventId, EventSequence, OccurredAt, StreamId};
use crate::domain::model::{CapabilityMappingVersion, ContentItem, ModelCapabilities};
use crate::domain::research_control::PromptComposition;
use crate::domain::session::{SessionEvent, SessionReducer};
use crate::domain::tools::{
    ExecutionId, ExecutionReceipt, ToolCall, ToolCallId, ToolName, ToolResult, ToolResultStatus,
};

fn binding(improvement: Option<bool>) -> AgentDefinitionBinding {
    let template = BuiltinAgentTemplate::TechnicalWriting;
    let snapshot = |resource: &str| {
        InstructionSnapshot::loaded(
            format!("Fixture {resource}."),
            InstructionSourceLocator::Builtin {
                template,
                resource: resource.to_owned(),
            },
        )
        .unwrap()
    };
    AgentDefinition::loaded(
        DefinitionSourceLocator::Builtin { template },
        AgentId::new("improvement-policy").unwrap(),
        "Improvement policy fixture".to_owned(),
        AgentInstructions::new(snapshot("persona"), snapshot("work"), Vec::new()).unwrap(),
        AgentContextSelection::new(
            AgentConfiguredContext::None,
            AgentConfiguredContext::None,
            AgentMemoryScope::None,
        ),
        AgentToolExposure::new(BTreeMap::new()),
        None,
        improvement.map(|improvement| AgentRsi {
            persona: false,
            methods: true,
            improvement,
            fixed_rules: None,
            state: None,
        }),
    )
    .unwrap()
    .bind(None, None)
    .unwrap()
}

fn append(events: &mut Vec<EventEnvelope<Value>>, event: SessionEvent) -> EventId {
    let sequence = events.len() as u64 + 1;
    let id = EventId::new(format!("improvement-policy-{sequence}"));
    events.push(
        event
            .into_envelope(
                id.clone(),
                StreamId::new("session:improvement-policy"),
                EventSequence::new(sequence),
                OccurredAt::new("2026-10-26T00:00:00Z"),
            )
            .unwrap(),
    );
    id
}

fn started() -> Vec<EventEnvelope<Value>> {
    let mut events = Vec::new();
    append(
        &mut events,
        SessionEvent::Started {
            primary_provider_session: ProviderSessionRef {
                id: ProviderSessionId::new("improvement-policy-provider"),
                role: ProviderSessionRole::Primary,
                generation: 1,
            },
        },
    );
    events
}

fn recorded_tool(
    events: &mut Vec<EventEnvelope<Value>>,
    name: &str,
    status: ToolResultStatus,
    verified_inputs: Value,
    has_receipt: bool,
) -> (Option<EventId>, EventId) {
    let call = ToolCall::new(
        ToolCallId::new(format!("call-{}", events.len())),
        ToolName::new(name),
        json!({"argv":["fixture", "same-input"]}),
    );
    append(
        events,
        SessionEvent::HistoryItemRecorded {
            item: ContentItem::ToolCall { call: call.clone() },
        },
    );
    let receipt_id = has_receipt.then(|| {
        let receipt: ExecutionReceipt = serde_json::from_value(json!({
            "execution_id":ExecutionId::for_model_call(&call.call_id),
            "plan_fingerprint":"fixture-plan", "status":status, "exit_reason":null,
            "output_artifact_ids":[], "environment_change":null
        }))
        .unwrap();
        append(events, SessionEvent::ExecutionReceiptRecorded { receipt })
    });
    let result_id = append(
        events,
        SessionEvent::HistoryItemRecorded {
            item: ContentItem::ToolResult {
                result: ToolResult {
                    call_id: call.call_id,
                    tool_name: call.name,
                    status,
                    output: json!({"verified_inputs":verified_inputs}),
                },
            },
        },
    );
    (receipt_id, result_id)
}

#[test]
fn app_improvement_policy_preserves_opt_in_idle_image_and_cancel_boundaries() {
    for (enabled, expected) in [(None, false), (Some(false), false), (Some(true), true)] {
        assert_eq!(policy(binding(enabled).definition()).is_some(), expected);
    }
    assert!(completed_boundary_is_eligible(false, true, false));
    for conditions in [
        (true, true, false),
        (false, false, false),
        (false, true, true),
    ] {
        assert!(!completed_boundary_is_eligible(
            conditions.0,
            conditions.1,
            conditions.2
        ));
    }
}

#[test]
fn app_improvement_friction_retains_exact_receipt_and_duplicate_result_references() {
    let mut events = started();
    append(
        &mut events,
        SessionEvent::HistoryItemRecorded {
            item: ContentItem::user_text("ordinary task"),
        },
    );
    let failure = recorded_tool(
        &mut events,
        "read_file",
        ToolResultStatus::Failed,
        json!([]),
        true,
    )
    .0
    .unwrap();
    recorded_tool(
        &mut events,
        "read_file",
        ToolResultStatus::Failed,
        json!([]),
        false,
    );
    recorded_tool(
        &mut events,
        "run_command",
        ToolResultStatus::Succeeded,
        json!([]),
        true,
    );
    recorded_tool(
        &mut events,
        "run_command",
        ToolResultStatus::Succeeded,
        json!([]),
        true,
    );
    let inputs = json!([{"path":"input.txt","blake3":"a".repeat(64)}]);
    recorded_tool(
        &mut events,
        "run_command",
        ToolResultStatus::Succeeded,
        inputs.clone(),
        true,
    );
    let repeated = recorded_tool(
        &mut events,
        "run_command",
        ToolResultStatus::Succeeded,
        inputs,
        true,
    )
    .1;
    let state = SessionReducer.reduce(events).unwrap();
    let budget = ModelRequestBudget::new(3);
    let ImprovementReadiness::Ready { current, friction } =
        readiness(&state, "ordinary task", 0, &budget)
    else {
        panic!("known retained friction was not eligible");
    };
    assert_eq!(friction, [failure, repeated]);
    assert_eq!(current.len(), state.history().items().len());
    assert_eq!(budget.counts().total(), 0);
    assert!(matches!(
        readiness(
            &state,
            "ordinary task",
            state.history().items().len(),
            &budget
        ),
        ImprovementReadiness::NoFriction
    ));
}

#[test]
fn app_improvement_feedback_is_bounded_and_uses_actual_shared_remaining_budget() {
    let mut events = started();
    let feedback = append(
        &mut events,
        SessionEvent::HistoryItemRecorded {
            item: ContentItem::user_text("[rsi-feedback] Retained operator feedback"),
        },
    );
    let mut expected = vec![feedback];
    for _ in 0..10 {
        expected.push(
            recorded_tool(
                &mut events,
                "read_file",
                ToolResultStatus::Failed,
                json!([]),
                true,
            )
            .0
            .unwrap(),
        );
    }
    expected.truncate(8);
    let state = SessionReducer.reduce(events).unwrap();
    let unlimited = ModelRequestBudget::new(0);
    let ImprovementReadiness::Ready { friction, .. } =
        readiness(&state, "[rsi-feedback] retained", 0, &unlimited)
    else {
        panic!("unlimited admission must retain the fixed trigger");
    };
    assert_eq!(friction, expected);
    let budget = ModelRequestBudget::new(3);
    budget.admit(ProviderSessionRole::Vision).unwrap();
    assert!(matches!(
        readiness(&state, "[rsi-feedback] retained", 0, &budget),
        ImprovementReadiness::InsufficientBudget
    ));
    let empty = SessionReducer.reduce(started()).unwrap();
    assert!(matches!(
        readiness(&empty, "ordinary task", 0, &budget),
        ImprovementReadiness::NoFriction
    ));
}

#[test]
fn rsi_visibility_separates_available_experience_plan_selection_and_model_adoption() {
    let initial = binding(Some(true));
    let source_lineage = initial.definition().digest().as_str().to_owned();
    let evolved = initial
        .definition()
        .with_evolution(
            None,
            None,
            EvolutionState {
                base_source_digest: source_lineage.clone(),
                parent_digest: source_lineage.clone(),
                reason: "Retained fixture contrast".to_owned(),
                basis: vec!["fixture retained observations".to_owned()],
                rollback_to: None,
                tools: Vec::new(),
                experiences: vec![EvolutionExperience {
                    cue: "matching task".to_owned(),
                    change: "Use the retained bounded method.".to_owned(),
                    attempt_id: "fixture-attempt".to_owned(),
                    references: vec![EventId::new("fixture-contrast-reference")],
                }],
            },
        )
        .unwrap()
        .bind(None, None)
        .unwrap();
    let prompt = PromptComposition::from_binding(evolved.clone()).unwrap();
    assert!(
        prompt
            .applicable_improvement("unrelated task")
            .unwrap()
            .is_none()
    );
    let fragment = prompt
        .applicable_improvement("MATCHING TASK")
        .unwrap()
        .unwrap();
    let capabilities = ModelCapabilities {
        text_input: true,
        image_input: false,
        streaming: true,
        tool_call: true,
        structured_output: true,
        incremental_input: false,
        usage_reporting: false,
        max_output_tokens: None,
        reasoning_effort: None,
        context_window: 4096,
        capability_mapping_version: CapabilityMappingVersion::new(1),
    };
    let no_plan = experience_status(&evolved, None);
    assert_eq!(no_plan["experience_count"], 1);
    assert_eq!(no_plan["source_lineage"], source_lineage);
    assert_eq!(no_plan["last_context"]["status"], "no_compiled_plan");
    for (tokens, expected) in [(4096, "include"), (1, "exclude")] {
        let plan = ContextPlanner::current()
            .compile(
                vec![fragment.clone()],
                ContextBudget::new(tokens, 0, 0).unwrap(),
                &capabilities,
                Vec::new(),
                Default::default(),
                None,
            )
            .unwrap();
        let visibility = experience_status(&evolved, Some(&plan));
        assert_eq!(visibility["last_context"]["decision"]["kind"], expected);
        assert_eq!(
            visibility["last_context"]["context_fingerprint"],
            plan.context_fingerprint()
        );
        assert_eq!(visibility["model_adoption"], "unobserved");
        assert_eq!(visibility["source_lineage"], source_lineage);
    }
}
