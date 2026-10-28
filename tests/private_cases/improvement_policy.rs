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
    assert_eq!(friction, [repeated, failure]);
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
    expected.reverse();
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
fn long_rsi_diagnostic_source_bounds_hundreds_of_steps_and_keeps_latest_signals() {
    let mut events = started();
    append(
        &mut events,
        SessionEvent::HistoryItemRecorded {
            item: ContentItem::user_text("A neutral task".repeat(4000)),
        },
    );
    let mut receipt_ids = Vec::new();
    for index in 0..160 {
        append(
            &mut events,
            SessionEvent::HistoryItemRecorded {
                item: ContentItem::assistant_text(format!(
                    "Historical observation{index}: {}",
                    "old material ".repeat(400)
                )),
            },
        );
        receipt_ids.push(
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
    let state = SessionReducer.reduce(events).unwrap();
    let budget = ModelRequestBudget::new(0);
    let ImprovementReadiness::Ready { current, friction } =
        readiness(&state, "A neutral task", 0, &budget)
    else {
        panic!("retained failures must be available");
    };
    assert_eq!(
        friction,
        receipt_ids
            .iter()
            .rev()
            .take(8)
            .cloned()
            .collect::<Vec<_>>()
    );
    let source = crate::app::improvement::diagnostic_source(
        &state,
        &"neutral goal ".repeat(5000),
        &current,
        &friction,
        8192,
        256,
        256,
    )
    .unwrap();
    let encoded = serde_json::to_vec(&source).unwrap();
    assert!(encoded.len() <= 8192);
    assert_eq!(
        source["operator_task_original_bytes"],
        ("neutral goal ".len() * 5000) as u64
    );
    assert_eq!(source["operator_task_excerpted"], true);
    assert!(source["previous_derived_handoff_original_bytes"].is_null());
    assert!(source["previous_derived_handoff_excerpted"].is_null());
    assert!(source["previous_derived_handoff_source"].is_null());
    assert_eq!(source["friction"], json!(friction));
    assert!(source["omitted_history_items"].as_u64().unwrap() > 100);
    assert!(
        source["observations"]
            .as_array()
            .unwrap()
            .iter()
            .all(|pair| pair["source_pair"].as_array().unwrap().len() == 2)
    );
    assert!(
        !String::from_utf8(encoded)
            .unwrap()
            .contains("Historical observation0:")
    );
}

#[test]
fn diagnostic_source_retains_command_semantics_before_clipping_large_values() {
    let long_path = ["a".repeat(225), "b".repeat(225), "c".repeat(225)].join("/");
    let oversized = (0..8)
        .map(|index| {
            json!({
                "path":format!("input-{index}/{}",vec!["n".repeat(225);17].join("/")),
                "blake3":"a".repeat(64),
            })
        })
        .collect::<Vec<_>>();
    for identities in [
        json!([{"path":"neutral-input.csv","blake3":"a".repeat(64)}]),
        json!([{"path":long_path,"blake3":"a".repeat(64)},{"path":"other-neutral-input.csv","blake3":"b".repeat(64)}]),
        json!(oversized),
    ] {
        let files: Vec<crate::domain::agent_evolution::ScriptFile> =
            serde_json::from_value(identities.clone()).unwrap();
        crate::domain::agent_evolution::validate_input_files(&files).unwrap();
        let mut events = started();
        append(
            &mut events,
            SessionEvent::HistoryItemRecorded {
                item: ContentItem::user_text("Review a neutral CSV formatting result."),
            },
        );
        let large = "A neutral Unicode observation: αβγ \"quoted\"\n".repeat(50_000);
        let mut friction = Vec::new();
        for (index, status, exit_code) in [
            (0, ToolResultStatus::Failed, 7),
            (1, ToolResultStatus::Succeeded, 0),
        ] {
            let call = ToolCall::new(
                ToolCallId::new(format!("neutral-command-{index}-{}", "c".repeat(80))),
                ToolName::new("run_command"),
                json!({"argv":["/usr/bin/printf",large]}),
            );
            append(
                &mut events,
                SessionEvent::HistoryItemRecorded {
                    item: ContentItem::ToolCall { call: call.clone() },
                },
            );
            events.last_mut().unwrap().event_id =
                EventId::new(format!("neutral-call-event-{index}-{}", "i".repeat(180)));
            let receipt: ExecutionReceipt=serde_json::from_value(json!({
            "execution_id":ExecutionId::for_model_call(&call.call_id),"plan_fingerprint":"neutral-plan",
            "status":status,"exit_reason":null,"output_artifact_ids":[],"environment_change":null
        })).unwrap();
            let receipt_id = append(
                &mut events,
                SessionEvent::ExecutionReceiptRecorded { receipt },
            );
            if status == ToolResultStatus::Failed {
                friction.push(receipt_id);
            }
            append(
                &mut events,
                SessionEvent::HistoryItemRecorded {
                    item: ContentItem::ToolResult {
                        result: ToolResult {
                            call_id: call.call_id,
                            tool_name: call.name,
                            status,
                            output: json!({"exit_code":exit_code,"verified_inputs":identities,"stdout":large,"stderr":""}),
                        },
                    },
                },
            );
            events.last_mut().unwrap().event_id =
                EventId::new(format!("neutral-result-event-{index}-{}", "i".repeat(180)));
        }
        let state = SessionReducer.reduce(events).unwrap();
        let current = state.history_items_with_event_ids().collect::<Vec<_>>();
        for limit in [8192, 65536] {
            let source = crate::app::improvement::diagnostic_source(
                &state,
                "Review the retained CSV observations.",
                &current,
                &friction,
                limit,
                limit / 32,
                limit / 32,
            )
            .unwrap();
            let encoded = serde_json::to_vec(&source).unwrap();
            assert!(encoded.len() <= limit);
            let observations = source["observations"].as_array().unwrap();
            if serde_json::to_vec(&identities).unwrap().len() > limit {
                assert!(
                    observations.is_empty(),
                    "oversized exact identities must omit whole pairs"
                );
                assert_eq!(source["omitted_history_items"], current.len());
            } else {
                assert!(
                    !observations.is_empty(),
                    "a complete identity pair fits the package"
                );
            }
            for observation in observations {
                let pair = observation["source_pair"].as_array().unwrap();
                assert_eq!(pair.len(), 2);
                assert_eq!(pair[0]["tool_name"], "run_command");
                assert_eq!(pair[0]["arguments"]["excerpted"], true);
                let result = &pair[1];
                assert_eq!(result["verified_inputs"], identities);
                assert_eq!(result["output"]["excerpted"], true);
                assert_eq!(
                    result["exit_code"],
                    if result["status"] == "failed" { 7 } else { 0 }
                );
                assert!(result["event_id"].as_str().unwrap().len() > 180);
                let excerpt = result["output"]["encoded_value_excerpt"].as_str().unwrap();
                assert!(!excerpt.contains('\u{fffd}'));
                assert!(excerpt.len() <= limit / 8);
            }
            assert!(!String::from_utf8(encoded).unwrap().contains(&large));
        }
        assert_eq!(state.history().items().iter().filter(|item|matches!(item,ContentItem::ToolResult {result} if result.output["stdout"].as_str().unwrap().len()==large.len())).count(),2);
    }
}

#[test]
fn readiness_compares_large_borrowed_commands_without_prefix_based_duplicates() {
    let mut events = started();
    append(
        &mut events,
        SessionEvent::HistoryItemRecorded {
            item: ContentItem::user_text("Review neutral command results."),
        },
    );
    let inputs = json!([{"path":"neutral.csv","blake3":"a".repeat(64)}]);
    let mut results = Vec::new();
    for suffix in ["first", "different", "first"] {
        let first = events.len();
        let (_, result) = recorded_tool(
            &mut events,
            "run_command",
            ToolResultStatus::Succeeded,
            inputs.clone(),
            true,
        );
        events[first].payload["item"]["call"]["arguments"] =
            json!({"argv":["fixture",format!("{}{suffix}","neutral argument ".repeat(20_000))]});
        results.push(result);
    }
    let state = SessionReducer.reduce(events).unwrap();
    let budget = ModelRequestBudget::new(0);
    let ImprovementReadiness::Ready { friction, .. } =
        readiness(&state, "Review neutral command results.", 0, &budget)
    else {
        panic!("exactly repeated command must be retained");
    };
    assert_eq!(friction, vec![results[2].clone()]);
}

#[test]
fn checkpoint_assessment_watermark_keeps_old_activity_readable_and_filters_only_considered_friction()
 {
    use crate::domain::session::{ImprovementActivity, ImprovementFact};
    let mut events = Vec::new();
    append(
        &mut events,
        SessionEvent::StartedWithAgent {
            primary_provider_session: ProviderSessionRef {
                id: ProviderSessionId::new("checkpoint-policy"),
                role: ProviderSessionRole::Primary,
                generation: 1,
            },
            binding: binding(Some(true)),
        },
    );
    append(
        &mut events,
        SessionEvent::HistoryItemRecorded {
            item: ContentItem::user_text("neutral task"),
        },
    );
    let first = recorded_tool(
        &mut events,
        "read_file",
        ToolResultStatus::Failed,
        json!([]),
        true,
    )
    .0
    .unwrap();
    let before = SessionReducer.reduce(events.clone()).unwrap();
    let start = SessionEvent::ImprovementActivityRecorded {
        activity: ImprovementActivity {
            schema_version: 2,
            attempt_id: "checkpoint-policy-first".to_owned(),
            fact: ImprovementFact::CheckpointStarted {
                parent_digest: binding(Some(true))
                    .definition()
                    .digest()
                    .as_str()
                    .to_owned(),
                history_start: 0,
                history_end: before.history().items().len(),
                friction: vec![first],
            },
        },
    };
    for schema in [0, 1, 3] {
        let mut changed = events.clone();
        append(&mut changed, start.clone());
        changed.last_mut().unwrap().payload["schema_version"] = json!(schema);
        assert!(
            SessionReducer.reduce(changed).is_err(),
            "checkpoint fact accepted wrong schema{schema}"
        );
    }
    append(&mut events, start);
    append(
        &mut events,
        SessionEvent::ImprovementActivityRecorded {
            activity: ImprovementActivity {
                schema_version: 1,
                attempt_id: "checkpoint-policy-first".to_owned(),
                fact: ImprovementFact::Finished {
                    disposition: "no_change".to_owned(),
                    reason: "No local oracle available.".to_owned(),
                },
            },
        },
    );
    let state = SessionReducer.reduce(events.clone()).unwrap();
    let budget = ModelRequestBudget::new(0);
    assert!(matches!(
        readiness(&state, "neutral task", 0, &budget),
        ImprovementReadiness::NoFriction
    ));
    let recent = recorded_tool(
        &mut events,
        "read_file",
        ToolResultStatus::Failed,
        json!([]),
        true,
    )
    .0
    .unwrap();
    let state = SessionReducer.reduce(events).unwrap();
    let ImprovementReadiness::Ready { friction, .. } =
        readiness(&state, "neutral task", 0, &budget)
    else {
        panic!("new friction was discarded with old history");
    };
    assert_eq!(friction, [recent]);
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
