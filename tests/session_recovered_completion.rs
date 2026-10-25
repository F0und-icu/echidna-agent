//! Completed-boundary validation distinguishes explicit recovery from a new gap.
include!("../src/lib.rs");

#[test]
fn model_usage_subsets_validate_without_inventing_missing_counts() {
    use crate::domain::model::{ModelUsage, ModelUsageError};
    let usage = ModelUsage {
        input_tokens: 21,
        output_tokens: 34,
        cached_input_tokens: None,
        reasoning_output_tokens: None,
    };
    assert_eq!(usage.validate(), Ok(()));
    for cached in [0, usage.input_tokens] {
        assert_eq!(
            ModelUsage {
                cached_input_tokens: Some(cached),
                ..usage
            }
            .validate(),
            Ok(())
        );
    }
    for reasoning in [0, usage.output_tokens] {
        assert_eq!(
            ModelUsage {
                reasoning_output_tokens: Some(reasoning),
                ..usage
            }
            .validate(),
            Ok(())
        );
    }
    assert_eq!(
        ModelUsage {
            cached_input_tokens: Some(22),
            ..usage
        }
        .validate(),
        Err(ModelUsageError::CachedInputExceedsTotal {
            cached_input_tokens: 22,
            input_tokens: 21
        })
    );
    assert_eq!(
        ModelUsage {
            reasoning_output_tokens: Some(35),
            ..usage
        }
        .validate(),
        Err(ModelUsageError::ReasoningOutputExceedsTotal {
            reasoning_output_tokens: 35,
            output_tokens: 34
        })
    );
}

mod recovered_completion {
    use std::collections::BTreeMap;

    use serde_json::{Value, json};
    use sha2::{Digest, Sha256};

    use crate::domain::agent_definition::{
        AgentConfiguredContext, AgentContextSelection, AgentDefinition, AgentDefinitionBinding,
        AgentId, AgentInstructions, AgentMemoryScope, AgentToolExposure, BuiltinAgentTemplate,
        DefinitionSourceLocator, InstructionSnapshot, InstructionSourceLocator,
    };
    use crate::domain::agent_evolution::AgentRsi;
    use crate::domain::artifacts::BlobId;
    use crate::domain::context::{
        BaselineInvalidationReason, ModelIdentity, ModelProfile, ProviderSessionId,
        ProviderSessionRef, ProviderSessionRole,
    };
    use crate::domain::events::{EventEnvelope, EventId, EventSequence, OccurredAt, StreamId};
    use crate::domain::history::HistoryError;
    use crate::domain::model::{
        CapabilityMappingVersion, ContentItem, ModelCapabilities, ModelInputIR,
        ModelRequestRequirements, ModelUsage, ToolArgumentDiagnostic, ToolArgumentErrorCategory,
    };
    use crate::domain::project::ProjectId;
    use crate::domain::request_audit::{
        PublishedAuditBlob, RequestAdapterVersion, RequestAuditAttempt, RequestAuditBinding,
        RequestAuditDelivery, RequestAuditRunContext,
    };
    use crate::domain::session::{
        ImprovementActivity, ImprovementFact, SessionError, SessionEvent, SessionReducer,
        SessionState,
    };
    use crate::domain::tools::{
        ExecutionId, ExecutionReceipt, ToolCall, ToolCallId, ToolName, ToolResult, ToolResultStatus,
    };

    fn provider(generation: u64) -> ProviderSessionRef {
        ProviderSessionRef {
            id: ProviderSessionId::new(format!("recovered-primary-{generation}")),
            role: ProviderSessionRole::Primary,
            generation,
        }
    }

    fn binding() -> AgentDefinitionBinding {
        let template = BuiltinAgentTemplate::TechnicalWriting;
        let snapshot = |resource: &str| {
            InstructionSnapshot::loaded(
                format!("Fixture {resource} instructions."),
                InstructionSourceLocator::Builtin {
                    template,
                    resource: resource.to_owned(),
                },
            )
            .unwrap()
        };
        AgentDefinition::loaded(
            DefinitionSourceLocator::Builtin { template },
            AgentId::new("recovered-completion").unwrap(),
            "Recovered completion fixture".to_owned(),
            AgentInstructions::new(snapshot("persona"), snapshot("work"), vec![]).unwrap(),
            AgentContextSelection::new(
                AgentConfiguredContext::None,
                AgentConfiguredContext::None,
                AgentMemoryScope::None,
            ),
            AgentToolExposure::new(BTreeMap::new()),
            None,
            Some(AgentRsi {
                persona: false,
                methods: true,
                improvement: true,
                fixed_rules: None,
                state: None,
            }),
        )
        .unwrap()
        .bind(None, None)
        .unwrap()
    }

    fn envelope(event: SessionEvent, sequence: usize) -> EventEnvelope<Value> {
        event
            .into_envelope(
                EventId::new(format!("recovered-{sequence}")),
                StreamId::new("session:recovered-completion"),
                EventSequence::new(sequence as u64),
                OccurredAt::new("2026-10-25T00:00:00Z"),
            )
            .unwrap()
    }

    fn push(events: &mut Vec<EventEnvelope<Value>>, event: SessionEvent) -> EventId {
        let event = envelope(event, events.len() + 1);
        let id = event.event_id.clone();
        events.push(event);
        id
    }

    fn item(events: &mut Vec<EventEnvelope<Value>>, item: ContentItem) -> EventId {
        push(events, SessionEvent::HistoryItemRecorded { item })
    }

    fn call(id: &str) -> ToolCall {
        ToolCall::new(
            ToolCallId::new(id),
            ToolName::new("read_file"),
            json!({"path":"fixture-only.txt"}),
        )
    }

    fn outcome(call: &ToolCall) -> (ExecutionReceipt, ToolResult) {
        let receipt = serde_json::from_value(json!({
            "execution_id":ExecutionId::for_model_call(&call.call_id),
            "plan_fingerprint":"fixture-plan", "status":"succeeded", "exit_reason":null,
            "output_artifact_ids":[], "environment_change":null
        }))
        .unwrap();
        let result = ToolResult {
            call_id: call.call_id.clone(),
            tool_name: call.name.clone(),
            status: ToolResultStatus::Succeeded,
            output: json!({"text":"actual recorded fixture result"}),
        };
        (receipt, result)
    }

    fn resumed_prefix() -> Vec<EventEnvelope<Value>> {
        let mut events = Vec::new();
        push(
            &mut events,
            SessionEvent::StartedWithAgent {
                primary_provider_session: provider(1),
                binding: binding(),
            },
        );
        item(&mut events, ContentItem::ToolCall { call: call("old") });
        push(
            &mut events,
            SessionEvent::Resumed {
                primary_provider_session: provider(2),
            },
        );
        events
    }

    fn improvement(state: &SessionState, history_start: usize, friction: EventId) -> SessionEvent {
        SessionEvent::ImprovementActivityRecorded {
            activity: ImprovementActivity {
                schema_version: 1,
                attempt_id: "completion-attempt".to_owned(),
                fact: ImprovementFact::Started {
                    parent_digest: state
                        .agent_definition_binding()
                        .unwrap()
                        .definition()
                        .digest()
                        .as_str()
                        .to_owned(),
                    history_start,
                    friction: vec![friction],
                },
            },
        }
    }

    fn assert_missing(state: &SessionState, expected: &str) {
        assert!(matches!(
            state.ensure_completed_history(),
            Err(HistoryError::MissingToolResult { call_id }) if call_id.as_str() == expected
        ));
    }

    #[test]
    fn recovered_unknown_does_not_block_rsi_after_a_complete_137_sample_turn() {
        let mut events = resumed_prefix();
        let history_start = 1;
        item(
            &mut events,
            ContentItem::user_text("Complete the next task."),
        );
        let mut friction = None;
        for step in 0..136 {
            let call = call(&format!("current-{step}"));
            item(&mut events, ContentItem::ToolCall { call: call.clone() });
            let mut result = outcome(&call).1;
            if step == 0 {
                result.status = ToolResultStatus::Failed;
                result.output = json!({"error":"observed fixture failure"});
            }
            let id = item(&mut events, ContentItem::ToolResult { result });
            if step == 0 {
                friction = Some(id);
            }
        }
        item(&mut events, ContentItem::assistant_text("Task completed."));
        let mut state = SessionReducer.reduce(events.clone()).unwrap();
        let original = state.history().clone();
        assert_missing(&SessionReducer.reduce(events[..2].to_vec()).unwrap(), "old");
        state.ensure_completed_history().unwrap();
        let normalized = state.normalized_history_items().unwrap();
        assert_eq!(normalized.len(), original.items().len() + 1);
        let ContentItem::ToolResult { result } = &normalized[1] else {
            panic!("recovery marker missing")
        };
        assert_eq!(result.output["execution_status"], "unknown");
        let event = improvement(&state, history_start, friction.unwrap());
        let accepted = envelope(event, events.len() + 1);
        SessionReducer.apply(&mut state, accepted.clone()).unwrap();
        events.push(accepted);
        assert_eq!(state.history(), &original);
        assert_eq!(SessionReducer.reduce(events).unwrap(), state);
    }

    #[test]
    fn a_new_missing_result_after_resume_still_rejects_rsi_and_definition_apply() {
        let mut events = resumed_prefix();
        item(&mut events, ContentItem::user_text("New task."));
        let friction = item(&mut events, ContentItem::ToolCall { call: call("new") });
        let mut state = SessionReducer.reduce(events.clone()).unwrap();
        assert_missing(&state, "new");
        let before = state.clone();
        for event in [
            improvement(&state, 1, friction),
            SessionEvent::AgentDefinitionApplied {
                primary_provider_session: provider(3),
                binding: binding(),
            },
        ] {
            assert!(matches!(
                SessionReducer.apply(&mut state, envelope(event, events.len() + 1)),
                Err(SessionError::History(HistoryError::MissingToolResult { call_id }))
                    if call_id.as_str() == "new"
            ));
            assert_eq!(state, before);
        }
    }

    #[test]
    fn an_old_bare_receipt_is_not_replaced_by_recovery_uncertainty() {
        let mut events = resumed_prefix();
        events.pop();
        push(
            &mut events,
            SessionEvent::ExecutionReceiptRecorded {
                receipt: outcome(&call("old")).0,
            },
        );
        push(
            &mut events,
            SessionEvent::Resumed {
                primary_provider_session: provider(2),
            },
        );
        let mut state = SessionReducer.reduce(events.clone()).unwrap();
        assert_missing(&state, "old");
        let before = state.clone();
        assert!(
            SessionReducer
                .apply(
                    &mut state,
                    envelope(
                        SessionEvent::AgentDefinitionApplied {
                            primary_provider_session: provider(3),
                            binding: binding(),
                        },
                        events.len() + 1,
                    ),
                )
                .is_err()
        );
        assert_eq!(state, before);
    }

    #[test]
    fn real_recorded_outcomes_complete_missing_history_results_on_either_side_of_resume() {
        for resumed in [false, true] {
            let mut events = resumed_prefix();
            if !resumed {
                events.pop();
            }
            let (receipt, result) = outcome(&call("old"));
            push(
                &mut events,
                SessionEvent::ExecutionOutcomeRecorded {
                    receipt,
                    result: result.clone(),
                },
            );
            let state = SessionReducer.reduce(events).unwrap();
            state.ensure_completed_history().unwrap();
            assert_eq!(state.history().items().len(), 1);
            assert_eq!(
                state.normalized_history_items().unwrap()[1],
                ContentItem::ToolResult { result }
            );
        }
    }

    #[test]
    fn only_an_accepted_fresh_resume_advances_the_recovery_cutoff() {
        let mut events = resumed_prefix();
        item(&mut events, ContentItem::ToolCall { call: call("new") });
        let mut state = SessionReducer.reduce(events.clone()).unwrap();
        let before = state.clone();
        assert!(
            SessionReducer
                .apply(
                    &mut state,
                    envelope(
                        SessionEvent::Resumed {
                            primary_provider_session: provider(2),
                        },
                        events.len() + 1,
                    ),
                )
                .is_err()
        );
        assert_eq!(state, before);
        assert_missing(&state, "new");
        SessionReducer
            .apply(
                &mut state,
                envelope(
                    SessionEvent::Resumed {
                        primary_provider_session: provider(3),
                    },
                    events.len() + 1,
                ),
            )
            .unwrap();
        state.ensure_completed_history().unwrap();
    }

    fn audit(events: &mut Vec<EventEnvelope<Value>>) -> EventId {
        let input = ModelInputIR::current(
            "fixture instructions",
            vec![ContentItem::user_text("fixture input")],
            ModelRequestRequirements::default(),
        );
        let context = RequestAuditRunContext::new(
            RequestAuditBinding::new(
                ProjectId::new("fixture-project"),
                StreamId::new("session:recovered-completion"),
            )
            .unwrap(),
            RequestAdapterVersion::new("fixture-adapter").unwrap(),
        );
        let rendered = RequestAuditAttempt::new(
            &context,
            ModelIdentity::new("fixture", "fixture-model"),
            ModelProfile::new("fixture", 1),
            ModelCapabilities {
                text_input: true,
                image_input: false,
                streaming: true,
                tool_call: true,
                structured_output: true,
                incremental_input: false,
                usage_reporting: false,
                max_output_tokens: None,
                reasoning_effort: None,
                context_window: 128_000,
                capability_mapping_version: CapabilityMappingVersion::new(1),
            },
            input.clone(),
            input,
            RequestAuditDelivery::full(BaselineInvalidationReason::NoBaseline),
        )
        .unwrap()
        .bind_provider_session(provider(2))
        .render()
        .unwrap();
        let blobs = rendered
            .component_bytes()
            .map(|bytes| PublishedAuditBlob {
                blob_id: BlobId::new(format!("{:x}", Sha256::digest(bytes))).unwrap(),
                size_bytes: bytes.len() as u64,
            })
            .collect::<Vec<_>>();
        let id = EventId::new(format!("recovered-{}", events.len() + 1));
        let manifest = rendered.materialize(id.clone(), blobs).unwrap();
        push(events, SessionEvent::RequestAuditRecorded { manifest });
        id
    }

    #[test]
    fn model_sample_rejection_uses_the_same_precise_completion_boundary() {
        for new_gap in [false, true] {
            let mut events = resumed_prefix();
            item(&mut events, ContentItem::user_text("Current task."));
            if new_gap {
                item(&mut events, ContentItem::ToolCall { call: call("new") });
            }
            let audit = audit(&mut events);
            let mut state = SessionReducer.reduce(events.clone()).unwrap();
            let before = state.clone();
            let result = SessionReducer.apply(
                &mut state,
                envelope(
                    SessionEvent::ModelSampleRejected {
                        request_audit_event_id: audit,
                        diagnostic: ToolArgumentDiagnostic {
                            category: ToolArgumentErrorCategory::Syntax,
                            line: 1,
                            column: 2,
                            arguments_bytes: 2,
                        },
                        corrections_used: 1,
                    },
                    events.len() + 1,
                ),
            );
            if new_gap {
                assert!(matches!(
                    result,
                    Err(SessionError::History(HistoryError::MissingToolResult { call_id }))
                        if call_id.as_str() == "new"
                ));
                assert_eq!(state, before);
            } else {
                result.unwrap();
                assert_eq!(state.history(), before.history());
            }
        }
    }

    #[test]
    fn definition_apply_after_resume_preserves_the_old_gap_and_rejects_later_gaps() {
        let mut events = resumed_prefix();
        item(&mut events, ContentItem::user_text("Completed task."));
        item(&mut events, ContentItem::assistant_text("Done."));
        push(
            &mut events,
            SessionEvent::AgentDefinitionApplied {
                primary_provider_session: provider(3),
                binding: binding(),
            },
        );
        let state = SessionReducer.reduce(events.clone()).unwrap();
        state.ensure_completed_history().unwrap();
        assert_eq!(state.history().items().len(), 3);
        item(
            &mut events,
            ContentItem::ToolCall {
                call: call("after-definition"),
            },
        );
        let state = SessionReducer.reduce(events).unwrap();
        assert_missing(&state, "after-definition");
    }

    #[test]
    fn legacy_research_definition_binding_establishes_its_own_precise_recovery_boundary() {
        use crate::adapters::agent_definition::builtin_research_binding;
        use crate::adapters::tools::ToolRegistry;
        use crate::ports::tool::ToolExecutor;

        let mut events = Vec::new();
        push(
            &mut events,
            SessionEvent::Started {
                primary_provider_session: provider(1),
            },
        );
        item(&mut events, ContentItem::ToolCall { call: call("old") });
        let catalog = ToolRegistry::for_project(env!("CARGO_MANIFEST_DIR")).catalog();
        push(
            &mut events,
            SessionEvent::AgentDefinitionApplied {
                primary_provider_session: provider(2),
                binding: builtin_research_binding(None, None, &catalog).unwrap(),
            },
        );
        let state = SessionReducer.reduce(events.clone()).unwrap();
        state.ensure_completed_history().unwrap();
        item(&mut events, ContentItem::ToolCall { call: call("new") });
        let state = SessionReducer.reduce(events).unwrap();
        assert_missing(&state, "new");
    }

    fn usage() -> ModelUsage {
        ModelUsage {
            input_tokens: 1_000,
            output_tokens: 200,
            cached_input_tokens: Some(750),
            reasoning_output_tokens: Some(150),
        }
    }

    #[test]
    fn usage_references_the_exact_audit_and_is_not_recorded_twice() {
        let mut events = resumed_prefix();
        let audit = audit(&mut events);
        let usage = SessionEvent::ModelUsageRecorded {
            request_audit_event_id: audit,
            provider_session: provider(2),
            usage: usage(),
        };
        push(&mut events, usage.clone());
        let mut state = SessionReducer.reduce(events.clone()).unwrap();
        let before = state.clone();
        assert!(matches!(
            SessionReducer.apply(&mut state, envelope(usage, events.len() + 1)),
            Err(SessionError::InvalidModelUsageReference { .. })
        ));
        assert_eq!(state, before);
        assert_eq!(
            serde_json::to_value(events.last().unwrap()).unwrap()["payload"]["usage"],
            json!({
                "input_tokens":1000,"output_tokens":200,
                "cached_input_tokens":750,"reasoning_output_tokens":150
            })
        );
    }

    #[test]
    fn an_earlier_physical_attempt_cannot_receive_usage_for_the_final_attempt() {
        let mut events = resumed_prefix();
        let earlier = audit(&mut events);
        let latest = audit(&mut events);
        let mut state = SessionReducer.reduce(events.clone()).unwrap();
        let before = state.clone();
        assert!(matches!(
            SessionReducer.apply(
                &mut state,
                envelope(
                    SessionEvent::ModelUsageRecorded {
                        request_audit_event_id: earlier,
                        provider_session: provider(2),
                        usage: usage(),
                    },
                    events.len() + 1,
                ),
            ),
            Err(SessionError::InvalidModelUsageReference { .. })
        ));
        assert_eq!(state, before);
        SessionReducer
            .apply(
                &mut state,
                envelope(
                    SessionEvent::ModelUsageRecorded {
                        request_audit_event_id: latest,
                        provider_session: provider(2),
                        usage: usage(),
                    },
                    events.len() + 1,
                ),
            )
            .unwrap();
    }

    #[test]
    fn usage_rejects_unknown_audits_session_role_generation_and_invalid_subsets_atomically() {
        let mut events = resumed_prefix();
        let audit = audit(&mut events);
        for variant in 0..6 {
            let mut id = audit.clone();
            let mut scope = provider(2);
            let mut usage = usage();
            match variant {
                0 => id = EventId::new("never-recorded"),
                1 => scope.role = ProviderSessionRole::Vision,
                2 => scope.generation = 99,
                3 => scope.id = ProviderSessionId::new("other-provider"),
                4 => usage.cached_input_tokens = Some(usage.input_tokens + 1),
                _ => usage.reasoning_output_tokens = Some(usage.output_tokens + 1),
            }
            let mut state = SessionReducer.reduce(events.clone()).unwrap();
            let before = state.clone();
            assert!(
                SessionReducer
                    .apply(
                        &mut state,
                        envelope(
                            SessionEvent::ModelUsageRecorded {
                                request_audit_event_id: id,
                                provider_session: scope,
                                usage,
                            },
                            events.len() + 1,
                        ),
                    )
                    .is_err(),
                "accepted variant {variant}"
            );
            assert_eq!(state, before);
        }
    }

    #[test]
    fn historical_unbound_or_rejected_audits_cannot_back_new_usage() {
        for rejected_attempts in 0..=2 {
            let mut events = resumed_prefix();
            let audit = audit(&mut events);
            if rejected_attempts > 0 {
                push(
                    &mut events,
                    SessionEvent::ModelSampleRejected {
                        request_audit_event_id: audit.clone(),
                        diagnostic: ToolArgumentDiagnostic {
                            category: ToolArgumentErrorCategory::Syntax,
                            line: 1,
                            column: 2,
                            arguments_bytes: 2,
                        },
                        corrections_used: 1,
                    },
                );
                if rejected_attempts == 2 {
                    let later_audit = self::audit(&mut events);
                    push(
                        &mut events,
                        SessionEvent::ModelSampleRejected {
                            request_audit_event_id: later_audit,
                            diagnostic: ToolArgumentDiagnostic {
                                category: ToolArgumentErrorCategory::Syntax,
                                line: 1,
                                column: 3,
                                arguments_bytes: 3,
                            },
                            corrections_used: 2,
                        },
                    );
                }
            } else {
                events
                    .last_mut()
                    .unwrap()
                    .payload
                    .as_object_mut()
                    .unwrap()
                    .remove("provider_session");
            }
            let mut state = SessionReducer.reduce(events.clone()).unwrap();
            let before = state.clone();
            assert!(matches!(
                SessionReducer.apply(
                    &mut state,
                    envelope(
                        SessionEvent::ModelUsageRecorded {
                            request_audit_event_id: audit,
                            provider_session: provider(2),
                            usage: usage(),
                        },
                        events.len() + 1,
                    ),
                ),
                Err(SessionError::InvalidModelUsageReference { .. })
            ));
            assert_eq!(state, before);
        }
    }
}
