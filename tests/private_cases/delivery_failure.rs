//! Fault at the production accepted-delivery checkpoint, before tool dispatch.

use std::{cell::RefCell, sync::Arc};

use crate::adapters::artifact_store::LocalArtifactStore;
use crate::adapters::mock_model::{MockModelFixture, MockModelProvider};
use crate::adapters::project_event_store::Project;
use crate::adapters::project_snapshot::LocalProjectSnapshot;
use crate::adapters::tools::ToolRegistry;
use crate::app::agent_loop::{
    AgentLoop, AgentLoopError, ChatDurabilityCheckpoint, CliRuntimeStages, CurrentContextState,
    ViewImageFollowUpFuture,
};
use crate::app::cancellation::AgentCancellation;
use crate::app::model_request_budget::ModelRequestBudget;
use crate::app::request_audit::RequestAuditService;
use crate::app::session::{AppendAfterDurableError, SessionProjection};
use crate::domain::context::{ProviderSession, ProviderSessionId, ProviderSessionRole};
use crate::domain::events::{EventEnvelope, EventId, EventSequence, OccurredAt, StreamId};
use crate::domain::model::{CapabilityMappingVersion, ContentItem, ModelCapabilities, ModelEvent};
use crate::domain::request_audit::{
    RequestAdapterVersion, RequestAuditBinding, RequestAuditRunContext,
};
use crate::domain::session::SessionEvent;
use crate::domain::tools::{ToolCall, ToolCallId, ToolName};
use crate::domain::world_state::ProjectSnapshotContext;
use crate::ports::EventSink;
use crate::ports::model::ModelResponseCompletion;
use crate::ports::tool::ToolExecutor;

#[derive(Debug)]
struct AppendFailure;

#[derive(Default)]
struct FailingDeliverySink {
    events: Vec<EventEnvelope<serde_json::Value>>,
}

impl EventSink<serde_json::Value> for FailingDeliverySink {
    type Error = AppendFailure;
    fn append_event(
        &mut self,
        event: &EventEnvelope<serde_json::Value>,
    ) -> Result<(), Self::Error> {
        if event.event_type.as_str() == "session.model_delivery_committed" {
            return Err(AppendFailure);
        }
        self.events.push(event.clone());
        Ok(())
    }
}

#[tokio::test]
async fn failed_accepted_delivery_checkpoint_stops_before_tool_execution() {
    let root = crate::support::TemporaryDirectory::new("delivery-commit-failure");
    let project = Project::initialize(root.path()).unwrap();
    let tools = ToolRegistry::for_project(root.path());
    let provider = MockModelProvider::new(MockModelFixture {
        capabilities: ModelCapabilities {
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
        },
        events: vec![ModelEvent::ToolCall {
            call: ToolCall::new(
                ToolCallId::new("must-not-run"),
                ToolName::new("write_file"),
                serde_json::json!({"path":"must-not-exist.txt","content":"unexpected","expected_hash":null}),
            ),
        }],
        completion: ModelResponseCompletion::without_response_id(),
    });
    let mut agent = AgentLoop::new(&provider, &tools)
        .with_research_phase(crate::domain::research_control::ResearchPhase::Experiment);
    let audit_context = RequestAuditRunContext::new(
        RequestAuditBinding::new(
            project.project_id().clone(),
            StreamId::new("session:delivery-commit-failure"),
        )
        .unwrap(),
        RequestAdapterVersion::new("fixture").unwrap(),
    );
    let stream_id = audit_context.binding().session_stream_id().clone();
    let started = SessionEvent::Started {
        primary_provider_session: ProviderSession::new(
            ProviderSessionId::new("primary:0"),
            ProviderSessionRole::Primary,
            0,
        )
        .reference()
        .clone(),
    }
    .into_envelope(
        EventId::new("start"),
        stream_id.clone(),
        EventSequence::new(1),
        OccurredAt::new("2026-10-28T00:00:00Z"),
    )
    .unwrap();
    let projection = RefCell::new(SessionProjection::replay([started]).unwrap());
    let sink = RefCell::new(FailingDeliverySink::default());
    let blobs = LocalArtifactStore::for_project(&project);
    let service = RequestAuditService::new(&blobs, audit_context.binding().clone());
    let accepted_audit = RefCell::new(None::<EventId>);
    let snapshot = LocalProjectSnapshot::new(&project)
        .inspect(&tools.specs(), &[], "fixture")
        .unwrap();
    let current = CurrentContextState::new(
        None,
        ProjectSnapshotContext::new(snapshot.snapshot).unwrap(),
        None,
    );
    let mut history_saved = false;
    let mut commit_attempted = false;
    let mut durability = |checkpoint: ChatDurabilityCheckpoint<'_>| {
        match checkpoint {
            ChatDurabilityCheckpoint::History(items) => {
                let mut projection = projection.borrow_mut();
                let mut sink = sink.borrow_mut();
                for item in items {
                    let sequence = projection.next_event_sequence().unwrap();
                    let event = SessionEvent::HistoryItemRecorded { item: item.clone() }
                        .into_envelope(
                            EventId::new(format!("history-{}", sequence.get())),
                            stream_id.clone(),
                            sequence,
                            OccurredAt::new("2026-10-28T00:00:00Z"),
                        )
                        .unwrap();
                    projection.append(&mut *sink, event).unwrap();
                }
                if items
                    .iter()
                    .any(|item| matches!(item, ContentItem::ToolCall { .. }))
                {
                    history_saved = true;
                }
            }
            ChatDurabilityCheckpoint::ModelDeliveryCommitted { .. } => {
                assert!(history_saved);
                commit_attempted = true;
                let mut projection = projection.borrow_mut();
                let id = accepted_audit.borrow().clone().unwrap();
                let delivery = projection.state().new_model_delivery(&id, None).unwrap();
                let sequence = projection.next_event_sequence().unwrap();
                let event = SessionEvent::ModelDeliveryCommitted { delivery }
                    .into_envelope(
                        EventId::new("delivery"),
                        stream_id.clone(),
                        sequence,
                        OccurredAt::new("2026-10-28T00:00:00Z"),
                    )
                    .unwrap();
                assert!(matches!(
                    projection.append(&mut *sink.borrow_mut(), event),
                    Err(AppendAfterDurableError::Sink(AppendFailure))
                ));
                assert!(
                    projection
                        .state()
                        .model_delivery_chain(&id)
                        .unwrap()
                        .is_none()
                );
                return Err(AgentLoopError::HistoryCheckpointCommit {
                    message: "injected delivery event append failure".into(),
                });
            }
            ChatDurabilityCheckpoint::ExecutionReceipt(_) => {
                panic!("tool executed after delivery persistence failure")
            }
            _ => {}
        }
        Ok(())
    };
    let mut starts = 0;
    let mut tool_start = |_: &crate::domain::tools::ToolExecutionPlan| starts += 1;
    let mut audit_count = 0;
    let mut audit = |attempt| {
        audit_count += 1;
        let id = EventId::new("audit-1");
        service
            .record(
                attempt,
                id.clone(),
                OccurredAt::new("2026-10-28T00:00:00Z"),
                &mut projection.borrow_mut(),
                &mut *sink.borrow_mut(),
            )
            .map_err(|error| error.into_agent_loop_error())?;
        accepted_audit.replace(Some(id));
        Ok(())
    };
    let mut follow_up = |_| -> ViewImageFollowUpFuture<'_> { Box::pin(async { unreachable!() }) };
    let mut text_delta = |_: &str| Ok(());
    let mut refresh = || Ok(current.clone());
    let outcome = agent
        .run_turn_audited_with_context_refresh_and_vision_stages(
            "Write one ordinary note",
            Vec::new(),
            AgentCancellation::new(),
            &audit_context,
            &mut audit,
            CliRuntimeStages::new(
                &mut follow_up,
                &mut text_delta,
                &mut tool_start,
                &mut durability,
                &mut refresh,
                Arc::new(ModelRequestBudget::new(2)),
            ),
            None,
        )
        .await;
    assert!(
        matches!(outcome, Err(AgentLoopError::HistoryCheckpointCommit { ref message })
        if message == "injected delivery event append failure")
    );
    assert!(history_saved && commit_attempted);
    assert_eq!(audit_count, 1);
    assert_eq!(starts, 0);
    assert!(
        sink.borrow()
            .events
            .iter()
            .any(|event| event.event_type.as_str() == "session.request_audit_recorded")
    );
    assert!(
        !sink
            .borrow()
            .events
            .iter()
            .any(|event| event.event_type.as_str() == "session.model_delivery_committed")
    );
    assert!(!root.path().join("must-not-exist.txt").exists());
}
