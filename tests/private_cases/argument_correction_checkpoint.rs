//! Injection at the existing production durability stage, before Tool dispatch.

use std::sync::{Arc, Mutex};

use serde_json::json;

use crate::adapters::project_event_store::Project;
use crate::adapters::project_snapshot::LocalProjectSnapshot;
use crate::adapters::tools::ToolRegistry;
use crate::app::agent_loop::{
    AgentLoop, AgentLoopError, AgentTurnOutcome, ChatDurabilityCheckpoint, CliRuntimeStages,
    CurrentContextState, ViewImageFollowUpFuture,
};
use crate::app::cancellation::AgentCancellation;
use crate::app::model_request_budget::ModelRequestBudget;
use crate::domain::events::StreamId;
use crate::domain::model::{
    CapabilityMappingVersion, ContentItem, ModelCapabilities, ModelEvent, ToolArgumentDiagnostic,
    ToolArgumentErrorCategory,
};
use crate::domain::request_audit::{
    RequestAdapterVersion, RequestAuditBinding, RequestAuditRunContext,
};
use crate::domain::tools::{ToolCall, ToolCallId, ToolName};
use crate::domain::world_state::ProjectSnapshotContext;
use crate::ports::model::{
    ModelEventStream, ModelProvider, ModelProviderError, ModelRequest, ModelResponseCompletion,
    ModelStreamEvent,
};
use crate::ports::tool::ToolExecutor;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StopAt {
    History,
    Reset,
    PartialTransport,
    Cancelled,
    CancelledThenContinued,
}

struct CorrectionProvider {
    stop: StopAt,
    requests: Mutex<usize>,
    cancellation: AgentCancellation,
}

impl ModelProvider for CorrectionProvider {
    fn capabilities(&self) -> ModelCapabilities {
        ModelCapabilities {
            text_input: true,
            image_input: false,
            streaming: true,
            tool_call: true,
            structured_output: false,
            incremental_input: false,
            usage_reporting: false,
            max_output_tokens: None,
            reasoning_effort: None,
            context_window: 32768,
            capability_mapping_version: CapabilityMappingVersion::new(1),
        }
    }

    fn stream(&self, _: ModelRequest) -> Result<ModelEventStream, ModelProviderError> {
        let mut count = self.requests.lock().unwrap();
        *count += 1;
        let continuing = self.stop == StopAt::CancelledThenContinued;
        assert!(
            *count <= if continuing { 6 } else { 2 },
            "dependent sampling after {:?}",
            self.stop
        );
        if *count == 1 || (continuing && matches!(*count, 3 | 5)) {
            return Ok(Box::pin(futures_util::stream::iter([Err(
                ModelProviderError::InvalidToolArguments {
                    diagnostic: ToolArgumentDiagnostic {
                        category: ToolArgumentErrorCategory::Syntax,
                        line: 1,
                        column: 2,
                        arguments_bytes: 2,
                    },
                },
            )])));
        }
        if *count == 2
            && matches!(
                self.stop,
                StopAt::Cancelled | StopAt::CancelledThenContinued
            )
        {
            self.cancellation.cancel();
        }
        let content = if continuing && *count == 6 {
            ModelEvent::TextDelta {
                text: "The ordinary note is complete.".to_owned(),
            }
        } else {
            ModelEvent::ToolCall {
                call: ToolCall::new(
                    ToolCallId::new(if continuing {
                        "accepted-once"
                    } else {
                        "must-not-dispatch"
                    }),
                    ToolName::new("write_file"),
                    json!({"path":"must-not-exist.txt","content":"ordinary note","expected_hash":null}),
                ),
            }
        };
        let terminal = if self.stop == StopAt::PartialTransport {
            Err(ModelProviderError::TransportFailure {
                message: "injected partial transport failure".to_owned(),
            })
        } else {
            Ok(ModelStreamEvent::Completed(
                ModelResponseCompletion::without_response_id(),
            ))
        };
        Ok(Box::pin(futures_util::stream::iter([
            Ok(ModelStreamEvent::Event(content)),
            terminal,
        ])))
    }
}

#[tokio::test]
async fn correction_reset_waits_for_durable_history_and_failure_blocks_dispatch_and_sampling() {
    for stop in [
        StopAt::History,
        StopAt::Reset,
        StopAt::PartialTransport,
        StopAt::Cancelled,
    ] {
        let root = crate::support::TemporaryDirectory::new("correction-checkpoint");
        let project = Project::initialize(root.path()).unwrap();
        let tools = ToolRegistry::for_project(root.path());
        let cancellation = AgentCancellation::new();
        let provider = CorrectionProvider {
            stop,
            requests: Mutex::new(0),
            cancellation: cancellation.clone(),
        };
        let mut agent = AgentLoop::new(&provider, &tools)
            .with_research_phase(crate::domain::research_control::ResearchPhase::Experiment);
        let audit_context = RequestAuditRunContext::new(
            RequestAuditBinding::new(
                project.project_id().clone(),
                StreamId::new("session:correction"),
            )
            .unwrap(),
            RequestAdapterVersion::new("fixture-correction:1").unwrap(),
        );
        let snapshot = LocalProjectSnapshot::new(&project)
            .inspect(&tools.specs(), &[], "fixture")
            .unwrap();
        let current = CurrentContextState::new(
            crate::domain::world_state::ResearchWorldState::default(),
            ProjectSnapshotContext::new(snapshot.snapshot).unwrap(),
            None,
            None,
            None,
        );
        let mut checkpoints = Vec::new();
        let mut durability = |checkpoint: ChatDurabilityCheckpoint<'_>| {
            match checkpoint {
                ChatDurabilityCheckpoint::History(items)
                    if items
                        .iter()
                        .any(|item| matches!(item, ContentItem::ToolCall { .. })) =>
                {
                    checkpoints.push("accepted_history");
                    if stop == StopAt::History {
                        return Err(AgentLoopError::HistoryCheckpointCommit {
                            message: "injected History append failure".to_owned(),
                        });
                    }
                }
                ChatDurabilityCheckpoint::ModelSampleRejected { .. } => {
                    checkpoints.push("rejected")
                }
                ChatDurabilityCheckpoint::ArgumentCorrectionReset => {
                    checkpoints.push("reset");
                    return Err(AgentLoopError::ModelRejectionCommit {
                        message: "injected reset append failure".to_owned(),
                    });
                }
                ChatDurabilityCheckpoint::ExecutionReceipt(_) => {
                    panic!("Tool executed after {stop:?}")
                }
                _ => {}
            }
            Ok(())
        };
        let mut follow_up = |_| -> ViewImageFollowUpFuture<'_> {
            Box::pin(async { unreachable!("no image calls") })
        };
        let mut text_delta = |_: &str| Ok(());
        let mut starts = 0;
        let mut tool_start = |_: &crate::domain::tools::ToolExecutionPlan| {
            starts += 1;
        };
        let mut refresh = || Ok(current.clone());
        let mut audits = 0;
        let mut audit = |_| {
            audits += 1;
            Ok(())
        };
        let budget = Arc::new(ModelRequestBudget::new(4));
        let outcome = agent
            .run_turn_audited_with_context_refresh_and_vision_stages(
                "Write one ordinary note",
                Vec::new(),
                cancellation,
                &audit_context,
                &mut audit,
                CliRuntimeStages::new(
                    &mut follow_up,
                    &mut text_delta,
                    &mut tool_start,
                    &mut durability,
                    &mut refresh,
                    budget.clone(),
                ),
                None,
            )
            .await;
        match stop {
            StopAt::History => assert!(
                matches!(outcome, Err(AgentLoopError::HistoryCheckpointCommit { ref message }) if message == "injected History append failure")
            ),
            StopAt::Reset => assert!(
                matches!(outcome, Err(AgentLoopError::ModelRejectionCommit { ref message }) if message == "injected reset append failure")
            ),
            StopAt::PartialTransport => assert!(
                matches!(outcome, Err(AgentLoopError::Model(crate::app::model_streaming::ModelStreamingError::Provider(ModelProviderError::TransportFailure { ref message }))) if message == "injected partial transport failure")
            ),
            StopAt::Cancelled | StopAt::CancelledThenContinued => {
                assert!(matches!(outcome, Ok(AgentTurnOutcome::Cancelled { .. })))
            }
        }
        assert_eq!(
            checkpoints,
            match stop {
                StopAt::History => vec!["rejected", "accepted_history"],
                StopAt::Reset => vec!["rejected", "accepted_history", "reset"],
                StopAt::PartialTransport | StopAt::Cancelled | StopAt::CancelledThenContinued =>
                    vec!["rejected"],
            },
            "{stop:?}"
        );
        assert_eq!(starts, 0, "{stop:?}");
        assert_eq!(audits, 2, "{stop:?}");
        assert_eq!(*provider.requests.lock().unwrap(), 2, "{stop:?}");
        assert_eq!(budget.counts().primary, 2, "{stop:?}");
        assert!(!root.path().join("must-not-exist.txt").exists());
    }
}

#[tokio::test]
async fn cancelled_chain_survives_new_operator_input_until_an_accepted_sample_is_durable() {
    let root = crate::support::TemporaryDirectory::new("correction-cancel-new-turn");
    let project = Project::initialize(root.path()).unwrap();
    let tools = ToolRegistry::for_project(root.path());
    let first_cancellation = AgentCancellation::new();
    let provider = CorrectionProvider {
        stop: StopAt::CancelledThenContinued,
        requests: Mutex::new(0),
        cancellation: first_cancellation.clone(),
    };
    let mut agent = AgentLoop::new(&provider, &tools)
        .with_research_phase(crate::domain::research_control::ResearchPhase::Experiment);
    let audit_context = RequestAuditRunContext::new(
        RequestAuditBinding::new(
            project.project_id().clone(),
            StreamId::new("session:cancel-new-turn"),
        )
        .unwrap(),
        RequestAdapterVersion::new("fixture-correction:1").unwrap(),
    );
    let snapshot = LocalProjectSnapshot::new(&project)
        .inspect(&tools.specs(), &[], "fixture")
        .unwrap();
    let current = CurrentContextState::new(
        crate::domain::world_state::ResearchWorldState::default(),
        ProjectSnapshotContext::new(snapshot.snapshot).unwrap(),
        None,
        None,
        None,
    );
    let mut rejection_counts = Vec::new();
    let mut user_checkpoints = 0;
    let mut accepted_samples = 0;
    let mut reset_after_sample = Vec::new();
    let mut receipts = 0;
    let mut starts = 0;
    for (turn, cancellation) in [first_cancellation, AgentCancellation::new()]
        .into_iter()
        .enumerate()
    {
        let mut durability = |checkpoint: ChatDurabilityCheckpoint<'_>| {
            match checkpoint {
                ChatDurabilityCheckpoint::History(items) => {
                    if items.iter().any(|item| {
                        matches!(
                            item,
                            ContentItem::Text {
                                role: crate::domain::model::MessageRole::User,
                                ..
                            }
                        )
                    }) {
                        user_checkpoints += 1;
                    }
                    if items.iter().any(|item| {
                        matches!(
                            item,
                            ContentItem::Text {
                                role: crate::domain::model::MessageRole::Assistant,
                                ..
                            } | ContentItem::ToolCall { .. }
                        )
                    }) {
                        accepted_samples += 1;
                    }
                }
                ChatDurabilityCheckpoint::ModelSampleRejected {
                    corrections_used, ..
                } => rejection_counts.push(corrections_used),
                ChatDurabilityCheckpoint::ArgumentCorrectionReset => {
                    reset_after_sample.push(accepted_samples)
                }
                ChatDurabilityCheckpoint::ExecutionReceipt(outcome) => {
                    assert_eq!(
                        outcome.result.status,
                        crate::domain::tools::ToolResultStatus::Succeeded
                    );
                    receipts += 1;
                }
                _ => {}
            }
            Ok(())
        };
        let mut follow_up = |_| -> ViewImageFollowUpFuture<'_> {
            Box::pin(async { unreachable!("no image calls") })
        };
        let mut text_delta = |_: &str| Ok(());
        let mut tool_start = |_: &crate::domain::tools::ToolExecutionPlan| {
            starts += 1;
        };
        let mut refresh = || Ok(current.clone());
        let mut audits = 0;
        let mut audit = |_| {
            audits += 1;
            Ok(())
        };
        let budget = Arc::new(ModelRequestBudget::new(8));
        let outcome = agent
            .run_turn_audited_with_context_refresh_and_vision_stages(
                if turn == 0 {
                    "Begin one ordinary note"
                } else {
                    "New operator instruction: finish that ordinary note"
                },
                Vec::new(),
                cancellation,
                &audit_context,
                &mut audit,
                CliRuntimeStages::new(
                    &mut follow_up,
                    &mut text_delta,
                    &mut tool_start,
                    &mut durability,
                    &mut refresh,
                    budget.clone(),
                ),
                None,
            )
            .await
            .unwrap();
        if turn == 0 {
            assert!(matches!(
                outcome,
                AgentTurnOutcome::Cancelled {
                    model_requests: 2,
                    ..
                }
            ));
            assert_eq!(rejection_counts, vec![0]);
            assert!(reset_after_sample.is_empty());
            assert_eq!(accepted_samples, 0);
            assert!(!root.path().join("must-not-exist.txt").exists());
        } else {
            assert!(matches!(
                outcome,
                AgentTurnOutcome::Completed {
                    model_requests: 4,
                    ..
                }
            ));
        }
        assert_eq!(audits, if turn == 0 { 2 } else { 4 });
        assert_eq!(budget.counts().primary, if turn == 0 { 2 } else { 4 });
    }
    assert_eq!(rejection_counts, vec![0, 1, 0]);
    assert_eq!(user_checkpoints, 2);
    assert_eq!(reset_after_sample, vec![1, 2]);
    assert_eq!(starts, 1);
    assert_eq!(receipts, 1);
    assert_eq!(*provider.requests.lock().unwrap(), 6);
    assert_eq!(
        std::fs::read(root.path().join("must-not-exist.txt")).unwrap(),
        b"ordinary note"
    );
    assert_eq!(agent.history().ensure_all_tool_calls_paired(), Ok(()));
}
