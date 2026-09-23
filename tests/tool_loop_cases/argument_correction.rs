use super::*;
use cyber_agent::domain::model::{ToolArgumentDiagnostic, ToolArgumentErrorCategory};

fn invalid_arguments() -> ModelProviderError {
    ModelProviderError::InvalidToolArguments {
        diagnostic: ToolArgumentDiagnostic {
            category: ToolArgumentErrorCategory::Syntax,
            line: 1,
            column: 335,
            arguments_bytes: 657,
        },
    }
}

#[tokio::test]
async fn correction_audit_failure_or_cancellation_never_starts_the_next_request() {
    for cancel in [false, true] {
        let provider = ScriptedProvider::with_capabilities(
            capabilities(),
            [failed_response(invalid_arguments())],
        );
        let project = TestProject::new("correction-audit-stop");
        let tools = ToolRegistry::for_project(&project.root);
        let mut agent = AgentLoop::new(&provider, &tools);
        let cancellation = AgentCancellation::new();
        let mut attempts = 0;
        let context = RequestAuditRunContext::new(
            RequestAuditBinding::new(ProjectId::new("fixture"), StreamId::new("session:fixture"))
                .unwrap(),
            RequestAdapterVersion::new("fixture:1").unwrap(),
        );
        let mut audit = |_| {
            attempts += 1;
            if attempts == 2 {
                if cancel {
                    cancellation.cancel();
                } else {
                    return Err(AgentLoopError::RequestAuditCommit {
                        message: "fixture refusal".to_owned(),
                    });
                }
            }
            Ok(())
        };
        let outcome = agent
            .run_turn_audited("Inspect a note", cancellation.clone(), &context, &mut audit)
            .await;
        if cancel {
            assert!(matches!(
                outcome,
                Ok(AgentTurnOutcome::Cancelled {
                    model_requests: 2,
                    ..
                })
            ));
        } else {
            assert!(matches!(
                outcome,
                Err(AgentLoopError::RequestAuditCommit { .. })
            ));
        }
        assert_eq!(provider.requests().len(), 1);
        assert_eq!(
            agent.history().items(),
            &[ContentItem::user_text("Inspect a note")]
        );
    }
}

#[tokio::test]
async fn non_argument_failures_never_enter_regeneration() {
    for error in [
        ModelProviderError::ProtocolFailure {
            message: "invalid outer JSON".to_owned(),
        },
        ModelProviderError::TransportFailure {
            message: "fixture disconnect".to_owned(),
        },
        ModelProviderError::HttpFailure { status: 429 },
        ModelProviderError::ProviderFailure {
            code: Some("response_incomplete".to_owned()),
            message: "limit".to_owned(),
        },
    ] {
        let provider =
            ScriptedProvider::with_capabilities(capabilities(), [failed_response(error)]);
        let tools = ToolRegistry::for_project(env!("CARGO_MANIFEST_DIR"));
        let mut agent = AgentLoop::new(&provider, &tools);
        assert!(
            agent
                .run_turn("Inspect", AgentCancellation::new())
                .await
                .is_err()
        );
        assert_eq!(provider.requests().len(), 1);
    }
}

#[tokio::test]
async fn malformed_first_step_after_activation_expiry_preserves_the_previous_turn_proof() {
    let provider = ScriptedProvider::with_capabilities(
        capabilities(),
        [
            completed_response(
                vec![
                    ModelEvent::ReasoningItem {
                        text: "retained reasoning".to_owned(),
                    },
                    ModelEvent::ToolCall {
                        call: tool_call("discover", "tool_search", json!({"query":"read_output"})),
                    },
                ],
                None,
            ),
            completed_response(
                vec![ModelEvent::TextDelta {
                    text: "first Turn done".to_owned(),
                }],
                None,
            ),
            failed_response(invalid_arguments()),
            completed_response(
                vec![ModelEvent::TextDelta {
                    text: "second Turn done".to_owned(),
                }],
                None,
            ),
        ],
    );
    let tools = ToolRegistry::for_project(env!("CARGO_MANIFEST_DIR"));
    let mut agent = AgentLoop::new(&provider, &tools);
    agent
        .run_turn("Discover read_output", AgentCancellation::new())
        .await
        .unwrap();
    agent
        .run_turn("Now report only", AgentCancellation::new())
        .await
        .unwrap();
    let requests = provider.requests();
    assert_eq!(requests.len(), 4);
    assert!(
        requests[1]
            .input()
            .tool_specs
            .iter()
            .any(|s| s.name.as_str() == "read_output")
    );
    for request in &requests[2..] {
        assert!(
            !request
                .input()
                .tool_specs
                .iter()
                .any(|s| s.name.as_str() == "read_output")
        );
        assert!(
            request.input().content_items.iter().any(
                |i| matches!(i, ContentItem::Reasoning { text } if text == "retained reasoning")
            )
        );
    }
}
