use std::collections::{BTreeMap, VecDeque};
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use cyber_agent::adapters::tools::ToolRegistry;
use cyber_agent::app::agent_loop::{AgentLoop, AgentTurnOutcome};
use cyber_agent::app::cancellation::AgentCancellation;
use cyber_agent::app::request_audit::RequestAuditService;
use cyber_agent::app::session::SessionProjection;
use cyber_agent::app::trace_eval::{TraceEvalService, TraceEvalServiceError};
use cyber_agent::domain::artifacts::BlobId;
use cyber_agent::domain::context::{ProviderSessionId, ProviderSessionRef, ProviderSessionRole};
use cyber_agent::domain::events::{EventEnvelope, EventId, EventSequence, OccurredAt, StreamId};
use cyber_agent::domain::model::{
    CapabilityMappingVersion, ContentItem, MessageRole, ModelCapabilities, ModelEvent,
};
use cyber_agent::domain::project::ProjectId;
use cyber_agent::domain::request_audit::{
    RequestAdapterVersion, RequestAuditAttempt, RequestAuditBinding, RequestAuditRunContext,
};
use cyber_agent::domain::research::{ResearchReducer, ResearchState};
use cyber_agent::domain::session::{SessionEvent, SessionState};
use cyber_agent::domain::tools::{
    ToolCall, ToolCallId, ToolExecutionOutcome, ToolName, ToolResult, ToolResultStatus,
};
use cyber_agent::domain::trace_eval::{
    BatchOpportunity, EvalEvidence, EvalProvenance, ToolAccuracyCaseExpectation,
    ToolAccuracyDeferredMetricKind, TraceEventRange, TraceExpectedInvariant, TraceFixtureManifest,
    TraceProviderEventReference,
};
use cyber_agent::ports::artifact::{ArtifactStore, StoredBlob};
use cyber_agent::ports::model::{
    ModelEventStream, ModelProvider, ModelProviderError, ModelRequest, ModelResponseCompletion,
    ModelStreamEvent,
};
use cyber_agent::ports::tool::{ToolExecutionFuture, ToolExecutor, ToolExecutorError};
use futures_util::stream;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const SESSION_ID: &str = "session:tool-accuracy";

#[derive(Clone, Debug, Eq, PartialEq)]
enum BlobFailure {
    Missing,
}

#[derive(Default)]
struct MemoryBlobStore {
    blobs: Mutex<BTreeMap<BlobId, Vec<u8>>>,
}

impl ArtifactStore for MemoryBlobStore {
    type Error = BlobFailure;

    fn publish_file(&self, _source: &Path) -> Result<StoredBlob, Self::Error> {
        unreachable!("controlled Trace RequestAudit uses in-memory Bytes")
    }

    fn publish_bytes(&self, bytes: &[u8]) -> Result<StoredBlob, Self::Error> {
        let blob_id = BlobId::new(format!("{:x}", Sha256::digest(bytes))).unwrap();
        let mut blobs = self.blobs.lock().unwrap();
        let already_present = blobs.contains_key(&blob_id);
        blobs
            .entry(blob_id.clone())
            .or_insert_with(|| bytes.to_vec());
        Ok(StoredBlob {
            blob_id,
            size_bytes: u64::try_from(bytes.len()).unwrap(),
            already_present,
        })
    }

    fn read_blob(&self, blob_id: &BlobId) -> Result<Vec<u8>, Self::Error> {
        self.blobs
            .lock()
            .unwrap()
            .get(blob_id)
            .cloned()
            .ok_or(BlobFailure::Missing)
    }
}

#[derive(Default)]
struct RecordingSink {
    events: Vec<EventEnvelope<Value>>,
}

impl cyber_agent::ports::EventSink<Value> for RecordingSink {
    type Error = &'static str;

    fn append_event(&mut self, event: &EventEnvelope<Value>) -> Result<(), Self::Error> {
        self.events.push(event.clone());
        Ok(())
    }
}

struct ScriptedProvider {
    responses: Mutex<VecDeque<Vec<ModelEvent>>>,
    requests: Mutex<Vec<ModelRequest>>,
}

impl ScriptedProvider {
    fn new(responses: impl IntoIterator<Item = Vec<ModelEvent>>) -> Self {
        Self {
            responses: Mutex::new(responses.into_iter().collect()),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<ModelRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl ModelProvider for ScriptedProvider {
    fn capabilities(&self) -> ModelCapabilities {
        capabilities()
    }

    fn stream(&self, request: ModelRequest) -> Result<ModelEventStream, ModelProviderError> {
        request.validate_capabilities(&self.capabilities())?;
        self.requests.lock().unwrap().push(request);
        let events = self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .expect("the fixture provides one response per RequestAudit sample");
        Ok(Box::pin(stream::iter(
            events
                .into_iter()
                .map(|event| Ok(ModelStreamEvent::Event(event)))
                .chain(std::iter::once(Ok(ModelStreamEvent::Completed(
                    ModelResponseCompletion::without_response_id(),
                )))),
        )))
    }
}

/// A controlled executor whose first otherwise-valid read is rejected during
/// planning. The next identical model call reaches the real file Tool, so the
/// derived report must not mistake identical arguments for a correction.
struct RejectFirstReadExecutor {
    registry: ToolRegistry,
    reject_first_read: AtomicBool,
}

impl RejectFirstReadExecutor {
    fn new(project_root: impl Into<std::path::PathBuf>) -> Self {
        Self {
            registry: ToolRegistry::for_project(project_root),
            reject_first_read: AtomicBool::new(true),
        }
    }
}

impl ToolExecutor for RejectFirstReadExecutor {
    fn catalog(&self) -> Vec<cyber_agent::domain::tools::ToolSpec> {
        self.registry.catalog()
    }

    fn plan(
        &self,
        execution_id: cyber_agent::domain::tools::ExecutionId,
        call: ToolCall,
    ) -> Result<cyber_agent::domain::tools::ToolExecutionPlan, ToolExecutorError> {
        self.registry.plan(execution_id, call)
    }

    fn plan_model_call(
        &self,
        resolved: &cyber_agent::domain::tools::ResolvedToolPlan,
        execution_id: cyber_agent::domain::tools::ExecutionId,
        call: ToolCall,
    ) -> Result<cyber_agent::domain::tools::ToolExecutionPlan, ToolExecutorError> {
        if call.name.as_str() == "read_file" && self.reject_first_read.swap(false, Ordering::SeqCst)
        {
            return Err(ToolExecutorError::InvalidArguments {
                tool_name: call.name.to_string(),
                message: "controlled first-read validation rejection".to_owned(),
            });
        }
        self.registry.plan_model_call(resolved, execution_id, call)
    }

    fn execute(
        &self,
        plan: cyber_agent::domain::tools::ToolExecutionPlan,
    ) -> ToolExecutionFuture<'_> {
        self.registry.execute(plan)
    }
}

struct ToolAccuracyFixture {
    blobs: MemoryBlobStore,
    source_events: Vec<EventEnvelope<Value>>,
    session: SessionState,
    research: ResearchState,
    manifest: TraceFixtureManifest,
    batch_call_ids: [ToolCallId; 2],
    failed_execution: ToolResult,
}

#[test]
fn every_model_visible_tool_has_canonical_guidance_and_typed_output_objects() {
    let tools = ToolRegistry::for_project(".");
    let specs = tools.catalog();
    assert_eq!(specs.len(), 28);
    assert!(specs.iter().any(|spec| spec.name.as_str() == "read_output"));
    for spec in specs {
        for heading in ["Use when:", "Do not use when:", "Prefer:", "Produces:"] {
            assert!(
                spec.description.contains(heading),
                "{} lacks {heading}",
                spec.name
            );
        }
        assert_ne!(
            spec.output_schema,
            json!({"type": "object"}),
            "{} has generic output",
            spec.name
        );
        assert!(
            spec.output_schema["oneOf"].is_array()
                || spec.output_schema["additionalProperties"] == json!(false),
            "{} output is open",
            spec.name
        );
        assert_typed_output_objects(spec.name.as_str(), &spec.output_schema, "$output");
    }
}

fn assert_typed_output_objects(tool_name: &str, schema: &Value, path: &str) {
    let Value::Object(object) = schema else {
        if let Value::Array(items) = schema {
            for (index, item) in items.iter().enumerate() {
                assert_typed_output_objects(tool_name, item, &format!("{path}[{index}]"));
            }
        }
        return;
    };
    if object.get("type") == Some(&json!("object")) {
        let typed_properties = object
            .get("properties")
            .and_then(Value::as_object)
            .is_some_and(|properties| !properties.is_empty());
        let typed_map = object
            .get("additionalProperties")
            .is_some_and(Value::is_object);
        let schema_payload = tool_name == "tool_search"
            && (path.ends_with(".input_schema") || path.ends_with(".output_schema"));
        assert!(
            typed_properties || typed_map || schema_payload,
            "{tool_name} has an unconstrained output object at {path}"
        );
    }
    for (key, value) in object {
        assert_typed_output_objects(tool_name, value, &format!("{path}.{key}"));
    }
}

#[tokio::test]
async fn accuracy_observes_agent_loop_audits_without_relaxing_replay() {
    let fixture = controlled_accuracy_fixture().await;
    let service = TraceEvalService::new(&fixture.blobs);

    assert!(matches!(
        service.prepare_mechanical_replay(
            &fixture.manifest,
            &fixture.source_events,
            &fixture.session,
            &fixture.research,
        ),
        Err(TraceEvalServiceError::ToolReceiptForCallMissing { call_id }) if call_id == "call-invalid"
    ));

    let report = service
        .tool_accuracy_report(
            &fixture.manifest,
            &fixture.source_events,
            &fixture.session,
            &fixture.research,
            ToolAccuracyCaseExpectation::new(
                "controlled-trace-first-action",
                [ToolName::new("read_file")],
            )
            .unwrap()
            .with_batch_opportunity(
                BatchOpportunity::new(fixture.batch_call_ids.clone(), ToolName::new("read_files"))
                    .unwrap(),
            ),
        )
        .unwrap();

    assert_eq!(report.schema_version, 1);
    assert_eq!(report.case_id(), "controlled-trace-first-action");
    assert_eq!(
        report.evaluation().provenance(),
        EvalProvenance::ToolAccuracy
    );
    assert_eq!(report.evaluation().evidence(), EvalEvidence::NonEvidence);
    assert_eq!(report.metrics().cases, 1);
    assert_eq!(report.metrics().first_action_observations, 1);
    assert_eq!(report.metrics().acceptable_first_action_matches, 1);
    assert_eq!(report.metrics().invalid_argument_count, 1);
    assert_eq!(report.metrics().correction_attempt_count, 1);
    assert_eq!(report.metrics().correction_success_count, 1);
    assert_eq!(report.metrics().redundant_reads, 1);
    assert_eq!(report.metrics().missed_batch_opportunities, 1);
    assert_eq!(report.metrics().tool_search_calls, 1);
    assert_eq!(report.metrics().tool_search_hits, 1);
    assert_eq!(report.metrics().deferred_activations, None);
    for metric in [
        ToolAccuracyDeferredMetricKind::DeferredActivation,
        ToolAccuracyDeferredMetricKind::UnmatchedReplayTermination,
        ToolAccuracyDeferredMetricKind::ObservationPromotion,
        ToolAccuracyDeferredMetricKind::ReplayPromotion,
        ToolAccuracyDeferredMetricKind::VisionPromotion,
    ] {
        assert!(
            report
                .metrics()
                .deferred_metrics
                .iter()
                .any(|deferred| deferred.metric == metric),
            "{metric:?} must remain explicitly deferred without a durable signal"
        );
    }
    assert!(report.metrics().schema_token_cost > 0);
    assert_eq!(
        fixture.failed_execution.output["failure"]["code"],
        "read_failed"
    );
    assert!(
        fixture.failed_execution.output["failure"]["message"]
            .as_str()
            .is_some_and(|message| message != "read_failed")
    );
}

#[tokio::test]
async fn deferred_activation_without_an_adjacent_request_audit_is_not_scored_as_zero() {
    let fixture = controlled_accuracy_fixture().await;
    let search_reference = fixture.manifest.provider_event_refs()[3].clone();
    let search_receipt = fixture
        .manifest
        .tool_receipt_refs()
        .iter()
        .find(|event_id| event_id.as_str() == "receipt-call-search")
        .cloned()
        .unwrap();
    let manifest = TraceFixtureManifest::new(
        "tool-accuracy-deferred-activation",
        fixture.manifest.source_session_id().clone(),
        fixture.manifest.event_range(),
        fixture.manifest.source_stream_hash(),
        Vec::new(),
        Vec::new(),
        vec![search_reference],
        vec![search_receipt],
        fixture.manifest.expected_invariants().to_vec(),
    )
    .unwrap();
    let report = TraceEvalService::new(&fixture.blobs)
        .tool_accuracy_report(
            &manifest,
            &fixture.source_events,
            &fixture.session,
            &fixture.research,
            ToolAccuracyCaseExpectation::new("deferred-activation", [ToolName::new("tool_search")])
                .unwrap(),
        )
        .unwrap();

    assert_eq!(report.metrics().deferred_activations, None);
    assert!(report.metrics().deferred_metrics.iter().any(|metric| {
        metric.metric == ToolAccuracyDeferredMetricKind::DeferredActivation
            && metric.reason == "no adjacent RequestAudit proves the Deferred ToolSpec transition"
    }));
}

#[tokio::test]
async fn only_changed_arguments_in_the_immediately_following_sample_count_as_a_correction() {
    let root = std::env::temp_dir().join(format!(
        "cyber-agent-m4-tool-accuracy-unchanged-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("same.txt"), "fixture\n").unwrap();
    std::fs::write(root.join("later.txt"), "later\n").unwrap();
    let first = tool_call(
        "call-invalid-unchanged",
        "read_file",
        json!({"path": "same.txt"}),
    );
    let unchanged = tool_call("call-unchanged", "read_file", json!({"path": "same.txt"}));
    let changed_later = tool_call(
        "call-changed-later",
        "read_file",
        json!({"path": "later.txt"}),
    );
    let samples = vec![
        vec![ContentItem::ToolCall {
            call: first.clone(),
        }],
        vec![ContentItem::ToolCall {
            call: unchanged.clone(),
        }],
        vec![ContentItem::ToolCall {
            call: changed_later.clone(),
        }],
        vec![ContentItem::Text {
            role: MessageRole::Assistant,
            text: "complete".to_owned(),
        }],
    ];
    let provider = ScriptedProvider::new(samples.iter().map(|items| model_events(items)));
    let tools = RejectFirstReadExecutor::new(&root);
    let mut agent = AgentLoop::new(&provider, &tools);
    let context = audit_context();
    let mut attempts = Vec::new();
    let mut outcomes = Vec::new();
    let mut text_delta = |_text: &str| Ok(());
    let mut record_execution = |outcome: &ToolExecutionOutcome| {
        outcomes.push(outcome.clone());
        Ok(())
    };
    agent
        .run_turn_audited_with_stages(
            "inspect unchanged arguments",
            AgentCancellation::new(),
            &context,
            &mut |attempt| {
                attempts.push(attempt);
                Ok(())
            },
            &mut text_delta,
            &mut record_execution,
        )
        .await
        .unwrap();
    let results = agent
        .history()
        .items()
        .iter()
        .filter_map(|item| match item {
            ContentItem::ToolResult { result } => Some((result.call_id.clone(), result.clone())),
            ContentItem::Text { .. }
            | ContentItem::ImageArtifact { .. }
            | ContentItem::ToolCall { .. }
            | ContentItem::Unsupported { .. } => None,
            ContentItem::Reasoning { .. } => {
                panic!("provider reasoning must not enter Conversation History")
            }
        })
        .collect();
    let fixture = record_agent_trace(
        context,
        attempts,
        outcomes,
        results,
        samples,
        [first.call_id, changed_later.call_id],
    );
    let service = TraceEvalService::new(&fixture.blobs);
    let report = service
        .tool_accuracy_report(
            &fixture.manifest,
            &fixture.source_events,
            &fixture.session,
            &fixture.research,
            ToolAccuracyCaseExpectation::new("unchanged-arguments", [ToolName::new("read_file")])
                .unwrap(),
        )
        .unwrap();
    assert_eq!(report.metrics().invalid_argument_count, 1);
    assert_eq!(report.metrics().correction_attempt_count, 0);
    assert_eq!(report.metrics().correction_success_count, 0);
    let _ = std::fs::remove_dir_all(root);
}

fn execution_failure_and_cancellation_use_stable_codes() {
    // Kept in this module so the model-visible failure assertions remain
    // alongside the Trace report; both constructors are production outcomes.
    let tools = ToolRegistry::for_project(".");
    let resolved = tools
        .resolve_tool_plan(
            &Default::default(),
            &cyber_agent::domain::tools::DeferredActivationSet::default(),
        )
        .unwrap();
    let plan = tools
        .plan_model_call(
            &resolved,
            cyber_agent::domain::tools::ExecutionId::new("execution-failure-code"),
            tool_call("failure-code", "read_file", json!({"path": "missing"})),
        )
        .unwrap();
    let failed =
        ToolExecutionOutcome::failed(plan.clone(), "read_failed", json!({"error": "missing"}));
    let cancelled = ToolExecutionOutcome::cancelled(plan, "agent_cancelled");
    assert_eq!(failed.result.output["failure"]["code"], "read_failed");
    assert_eq!(failed.result.output["failure"]["message"], "missing");
    assert_eq!(
        cancelled.result.output["failure"]["code"],
        "tool_execution_cancelled"
    );
    assert_eq!(
        cancelled.result.output["failure"]["message"],
        "agent_cancelled"
    );
}

#[test]
fn execution_failure_and_cancellation_use_stable_codes_test() {
    execution_failure_and_cancellation_use_stable_codes();
}

async fn controlled_accuracy_fixture() -> ToolAccuracyFixture {
    let root = std::env::temp_dir().join(format!(
        "cyber-agent-m4-tool-accuracy-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("same.txt"), "before\n").unwrap();
    std::fs::write(root.join("first.txt"), "first\n").unwrap();
    std::fs::write(root.join("second.txt"), "second\n").unwrap();

    let invalid = tool_call("call-invalid", "read_file", json!({}));
    let corrected = tool_call("call-corrected", "read_file", json!({"path": "same.txt"}));
    let repeated = tool_call("call-repeated", "read_file", json!({"path": "same.txt"}));
    let search = tool_call("call-search", "tool_search", json!({"query": "write file"}));
    let before_hash = blake3::hash(b"before\n").to_hex().to_string();
    let write = tool_call(
        "call-write",
        "write_file",
        json!({"path": "same.txt", "content": "after\n", "expected_hash": before_hash}),
    );
    let changed_read = tool_call(
        "call-changed-read",
        "read_file",
        json!({"path": "same.txt"}),
    );
    let batch_first = tool_call(
        "call-batch-first",
        "read_file",
        json!({"path": "first.txt"}),
    );
    let batch_second = tool_call(
        "call-batch-second",
        "read_file",
        json!({"path": "second.txt"}),
    );
    let missing = tool_call("call-missing", "read_file", json!({"path": "missing.txt"}));
    let model_samples = vec![
        vec![ContentItem::ToolCall {
            call: invalid.clone(),
        }],
        vec![ContentItem::ToolCall {
            call: corrected.clone(),
        }],
        vec![ContentItem::ToolCall {
            call: repeated.clone(),
        }],
        vec![ContentItem::ToolCall {
            call: search.clone(),
        }],
        vec![ContentItem::ToolCall {
            call: write.clone(),
        }],
        vec![ContentItem::ToolCall {
            call: changed_read.clone(),
        }],
        vec![
            ContentItem::ToolCall {
                call: batch_first.clone(),
            },
            ContentItem::ToolCall {
                call: batch_second.clone(),
            },
        ],
        vec![ContentItem::ToolCall {
            call: missing.clone(),
        }],
        vec![ContentItem::Text {
            role: MessageRole::Assistant,
            text: "controlled trace complete".to_owned(),
        }],
    ];
    let provider = ScriptedProvider::new(model_samples.iter().map(|items| model_events(items)));
    let tools = ToolRegistry::for_project(&root);
    let mut agent = AgentLoop::new(&provider, &tools);
    let context = audit_context();
    let mut attempts = Vec::<RequestAuditAttempt>::new();
    let mut outcomes = Vec::<ToolExecutionOutcome>::new();
    let mut text_delta = |_text: &str| Ok(());
    let mut record_execution = |outcome: &ToolExecutionOutcome| {
        outcomes.push(outcome.clone());
        Ok(())
    };

    let outcome = agent
        .run_turn_audited_with_stages(
            "inspect controlled fixture",
            AgentCancellation::new(),
            &context,
            &mut |attempt| {
                attempts.push(attempt);
                Ok(())
            },
            &mut text_delta,
            &mut record_execution,
        )
        .await
        .unwrap();
    assert!(matches!(outcome, AgentTurnOutcome::Completed { .. }));
    assert_eq!(attempts.len(), model_samples.len());
    let requests = provider.requests();
    assert!(!contains_tool(&requests[3], "write_file"));
    assert!(contains_tool(&requests[4], "write_file"));
    assert_eq!(agent.history().ensure_all_tool_calls_paired(), Ok(()));

    let failed_execution = agent
        .history()
        .items()
        .iter()
        .find_map(|item| match item {
            ContentItem::ToolResult { result } if result.call_id == missing.call_id => {
                Some(result.clone())
            }
            ContentItem::Text { .. }
            | ContentItem::ImageArtifact { .. }
            | ContentItem::ToolCall { .. }
            | ContentItem::ToolResult { .. }
            | ContentItem::Unsupported { .. } => None,
            ContentItem::Reasoning { .. } => {
                panic!("provider reasoning must not enter Conversation History")
            }
        })
        .unwrap();
    let history_results = agent
        .history()
        .items()
        .iter()
        .filter_map(|item| match item {
            ContentItem::ToolResult { result } => Some((result.call_id.clone(), result.clone())),
            ContentItem::Text { .. }
            | ContentItem::ImageArtifact { .. }
            | ContentItem::ToolCall { .. }
            | ContentItem::Unsupported { .. } => None,
            ContentItem::Reasoning { .. } => {
                panic!("provider reasoning must not enter Conversation History")
            }
        })
        .collect();

    let fixture = record_agent_trace(
        context,
        attempts,
        outcomes,
        history_results,
        model_samples,
        [batch_first.call_id.clone(), batch_second.call_id.clone()],
    );
    let _ = std::fs::remove_dir_all(root);
    ToolAccuracyFixture {
        failed_execution,
        ..fixture
    }
}

fn record_agent_trace(
    context: RequestAuditRunContext,
    attempts: Vec<RequestAuditAttempt>,
    outcomes: Vec<ToolExecutionOutcome>,
    results: BTreeMap<ToolCallId, ToolResult>,
    model_samples: Vec<Vec<ContentItem>>,
    batch_call_ids: [ToolCallId; 2],
) -> ToolAccuracyFixture {
    let blobs = MemoryBlobStore::default();
    let started = envelope(
        SessionEvent::Started {
            primary_provider_session: ProviderSessionRef {
                id: ProviderSessionId::new("primary-tool-accuracy"),
                role: ProviderSessionRole::Primary,
                generation: 1,
            },
        },
        "started",
        1,
    );
    let mut projection = SessionProjection::replay([started.clone()]).unwrap();
    let mut sink = RecordingSink::default();
    let mut sequence = 2;
    let receipts = outcomes
        .into_iter()
        .map(|outcome| (outcome.result.call_id.clone(), outcome.receipt))
        .collect::<BTreeMap<_, _>>();

    let mut provider_refs = Vec::new();
    let mut receipt_event_ids = Vec::new();
    for (sample_index, (attempt, items)) in attempts.into_iter().zip(model_samples).enumerate() {
        let audit_event_id = EventId::new(format!("audit-{sample_index}"));
        RequestAuditService::new(&blobs, context.binding().clone())
            .record(
                attempt,
                audit_event_id.clone(),
                OccurredAt::new("2026-08-29T00:00:01Z"),
                &mut projection,
                &mut sink,
            )
            .unwrap();
        sequence += 1;
        let mut history_event_ids = Vec::new();
        for (item_index, item) in items.into_iter().enumerate() {
            let history_event_id = format!("model-{sample_index}-{item_index}");
            append(
                &mut projection,
                &mut sink,
                SessionEvent::HistoryItemRecorded { item: item.clone() },
                &history_event_id,
                sequence,
            );
            history_event_ids.push(EventId::new(history_event_id));
            sequence += 1;
            let ContentItem::ToolCall { call } = item else {
                continue;
            };
            if let Some(receipt) = receipts.get(&call.call_id) {
                let receipt_event_id = format!("receipt-{}", call.call_id);
                append(
                    &mut projection,
                    &mut sink,
                    SessionEvent::ExecutionReceiptRecorded {
                        receipt: receipt.clone(),
                    },
                    &receipt_event_id,
                    sequence,
                );
                receipt_event_ids.push(EventId::new(receipt_event_id));
                sequence += 1;
            }
            let result = results
                .get(&call.call_id)
                .cloned()
                .expect("AgentLoop keeps every model Tool Call paired");
            append(
                &mut projection,
                &mut sink,
                SessionEvent::HistoryItemRecorded {
                    item: ContentItem::ToolResult { result },
                },
                &format!("result-{}", call.call_id),
                sequence,
            );
            sequence += 1;
        }
        provider_refs
            .push(TraceProviderEventReference::new(audit_event_id, history_event_ids).unwrap());
    }

    let mut source_events = vec![started];
    source_events.extend(sink.events);
    let manifest = TraceFixtureManifest::new(
        "tool-accuracy-controlled",
        StreamId::new(SESSION_ID),
        TraceEventRange::new(EventSequence::new(1), EventSequence::new(sequence - 1)).unwrap(),
        TraceFixtureManifest::source_stream_hash_for(&source_events).unwrap(),
        Vec::new(),
        Vec::new(),
        provider_refs,
        receipt_event_ids,
        vec![
            TraceExpectedInvariant::FullRequestSemanticReplay,
            TraceExpectedInvariant::ToolPairing,
            TraceExpectedInvariant::ZeroExternalActions,
            TraceExpectedInvariant::NonEvidence,
        ],
    )
    .unwrap();
    ToolAccuracyFixture {
        blobs,
        source_events,
        session: projection.state().clone(),
        research: ResearchReducer
            .reduce(Vec::<EventEnvelope<Value>>::new())
            .unwrap(),
        manifest,
        batch_call_ids,
        failed_execution: ToolResult {
            call_id: ToolCallId::new("unused"),
            tool_name: ToolName::new("unused"),
            status: ToolResultStatus::Failed,
            output: Value::Null,
        },
    }
}

fn model_events(items: &[ContentItem]) -> Vec<ModelEvent> {
    items
        .iter()
        .map(|item| match item {
            ContentItem::ToolCall { call } => ModelEvent::ToolCall { call: call.clone() },
            ContentItem::Text {
                role: MessageRole::Assistant,
                text,
            } => ModelEvent::TextDelta { text: text.clone() },
            ContentItem::Text { .. }
            | ContentItem::ImageArtifact { .. }
            | ContentItem::ToolResult { .. }
            | ContentItem::Unsupported { .. }
            | ContentItem::Reasoning { .. } => panic!("fixture sample must be model output"),
        })
        .collect()
}

fn contains_tool(request: &ModelRequest, name: &str) -> bool {
    request
        .input()
        .tool_specs
        .iter()
        .any(|spec| spec.name.as_str() == name)
}

fn tool_call(id: &str, name: &str, arguments: Value) -> ToolCall {
    ToolCall::new(ToolCallId::new(id), ToolName::new(name), arguments)
}

fn audit_context() -> RequestAuditRunContext {
    RequestAuditRunContext::new(
        RequestAuditBinding::new(
            ProjectId::new("tool-accuracy-project"),
            StreamId::new(SESSION_ID),
        )
        .unwrap(),
        RequestAdapterVersion::new("tool-accuracy-adapter-v1").unwrap(),
    )
}

fn capabilities() -> ModelCapabilities {
    ModelCapabilities {
        text_input: true,
        image_input: false,
        streaming: true,
        tool_call: true,
        structured_output: true,
        incremental_input: false,
        usage_reporting: true,
        context_window: 32_768,
        capability_mapping_version: CapabilityMappingVersion::new(1),
    }
}

fn envelope(event: SessionEvent, id: &str, sequence: u64) -> EventEnvelope<Value> {
    event
        .into_envelope(
            EventId::new(id),
            StreamId::new(SESSION_ID),
            EventSequence::new(sequence),
            OccurredAt::new("2026-08-29T00:00:00Z"),
        )
        .unwrap()
}

fn append(
    projection: &mut SessionProjection,
    sink: &mut RecordingSink,
    event: SessionEvent,
    id: &str,
    sequence: u64,
) {
    projection
        .append(sink, envelope(event, id, sequence))
        .unwrap();
}
