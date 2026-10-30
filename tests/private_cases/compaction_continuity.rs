//! Narrow production-owner regressions for sparse-prose long-task handoffs.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use serde_json::{Value, json};

use crate::adapters::agent_definition::{bind_definition, builtin_definition};
use crate::adapters::mock_model::{MockModelFixture, MockModelProvider};
use crate::adapters::tools::ToolRegistry;
use crate::app::agent_loop::{AgentLoop, CompactionSampleInput};
use crate::app::cancellation::AgentCancellation;
use crate::app::model_request_budget::ModelRequestBudget;
use crate::domain::agent_definition::BuiltinAgentTemplate;
use crate::domain::context::{
    ModelIdentity, ModelProfile, ProviderSession, ProviderSessionId, ProviderSessionRole,
};
use crate::domain::events::{EventEnvelope, EventId, EventSequence, OccurredAt, StreamId};
use crate::domain::history::{latest_complete_tool_batch_items, retain_recent_complete_history};
use crate::domain::model::{CapabilityMappingVersion, ContentItem, ModelCapabilities, ModelEvent};
use crate::domain::project::ProjectId;
use crate::domain::request_audit::{
    RequestAdapterVersion, RequestAuditBinding, RequestAuditRunContext,
};
use crate::domain::research::ExecutionReceiptId;
use crate::domain::session::{SessionEvent, SessionSourceIdentity, TaskCompactionValidationFacts};
use crate::domain::tools::{ToolCall, ToolCallId, ToolName, ToolResult, ToolResultStatus};
use crate::ports::model::{
    ContinuationRetention, ModelEventStream, ModelProvider, ModelProviderError, ModelRequest,
    ModelResponseCompletion,
};

fn source() -> (
    SessionSourceIdentity,
    Vec<EventEnvelope<Value>>,
    crate::domain::agent_definition::AgentDefinitionBinding,
) {
    let tools = ToolRegistry::for_project(".");
    use crate::ports::tool::ToolExecutor;
    let binding = bind_definition(
        builtin_definition(BuiltinAgentTemplate::TechnicalWriting, &tools.catalog()).unwrap(),
        None,
        None,
    )
    .unwrap();
    let mut events = Vec::new();
    for (index, event) in [
        SessionEvent::StartedWithAgent {
            primary_provider_session: ProviderSession::new(
                ProviderSessionId::new("parent"),
                ProviderSessionRole::Primary,
                0,
            )
            .reference()
            .clone(),
            binding: binding.clone(),
        },
        SessionEvent::HistoryItemRecorded {
            item: ContentItem::user_text("ORIGINAL-OPERATOR-GOAL"),
        },
    ]
    .into_iter()
    .enumerate()
    {
        events.push(
            event
                .into_envelope(
                    EventId::new(format!("event-{index}")),
                    StreamId::new("session:continuity"),
                    EventSequence::new(index as u64 + 1),
                    OccurredAt::new("2026-10-26T00:00:00Z"),
                )
                .unwrap(),
        );
    }
    let (source, _) =
        SessionSourceIdentity::from_events(ProjectId::new("continuity"), &events, &[]).unwrap();
    (source, events, binding)
}

fn call(id: &str) -> ContentItem {
    ContentItem::ToolCall {
        call: ToolCall::new(
            ToolCallId::new(id),
            ToolName::new("read_file"),
            json!({"path":format!("{id}.txt")}),
        ),
    }
}
fn result(id: &str, status: ToolResultStatus) -> ContentItem {
    ContentItem::ToolResult {
        result: ToolResult {
            call_id: ToolCallId::new(id),
            tool_name: ToolName::new("read_file"),
            status,
            output: json!({"actual":id}),
        },
    }
}

#[test]
fn latest_batch_floor_keeps_parallel_calls_interleaved_prose_and_failed_results() {
    let history = vec![
        ContentItem::user_text("task"),
        call("old"),
        result("old", ToolResultStatus::Succeeded),
        call("left"),
        ContentItem::assistant_text("Checking another branch"),
        call("right"),
        result("right", ToolResultStatus::Failed),
        result("left", ToolResultStatus::Succeeded),
    ];
    let minimum = latest_complete_tool_batch_items(&history).unwrap();
    assert_eq!(minimum, 5);
    assert_eq!(
        retain_recent_complete_history(&history, minimum).unwrap().1,
        history[3..]
    );
    assert!(
        retain_recent_complete_history(&history, 0)
            .unwrap()
            .1
            .is_empty(),
        "historical zero-retention events remain readable"
    );
}

#[test]
fn task_reference_selection_uses_durable_order_instead_of_nonmonotonic_ids() {
    let (source, _, binding) = source();
    let facts = TaskCompactionValidationFacts::new(
        source,
        &binding,
        BTreeSet::new(),
        vec![
            ExecutionReceiptId::new("z-first"),
            ExecutionReceiptId::new("a-second"),
            ExecutionReceiptId::new("m-latest"),
        ],
        vec![
            EventId::new("z-first"),
            EventId::new("a-second"),
            EventId::new("m-latest"),
        ],
        BTreeMap::new(),
    )
    .unwrap();
    let selected = serde_json::to_value(facts.references(2)).unwrap();
    assert_eq!(
        selected["execution_receipt_refs"],
        json!(["m-latest", "a-second"])
    );
    assert_eq!(
        selected["history_event_refs"],
        json!(["m-latest", "a-second"])
    );
}

#[test]
fn modern_summary_projects_narrative_without_machine_reference_dump() {
    use crate::domain::session::{
        CompactionValidationContext, NarrativeSummary, SessionSummary, SessionSummaryDraft,
    };
    let (source, _, binding) = source();
    let receipts = (0..128)
        .map(|i| ExecutionReceiptId::new(format!("receipt-{i:04}-{}", "r".repeat(80))))
        .collect::<Vec<_>>();
    let history = (0..128)
        .map(|i| EventId::new(format!("history-{i:04}-{}", "h".repeat(80))))
        .collect::<Vec<_>>();
    let facts = TaskCompactionValidationFacts::new(
        source,
        &binding,
        BTreeSet::new(),
        receipts,
        history,
        BTreeMap::new(),
    )
    .unwrap();
    let narrative = NarrativeSummary::new(
        "goal",
        "complete observed progress",
        vec!["unresolved".into()],
        vec!["next action".into()],
    )
    .unwrap();
    let draft = SessionSummaryDraft::for_task(facts.references(128), None, narrative.clone())
        .unwrap()
        .with_byte_limit(65536)
        .unwrap();
    let project = ProjectId::new("continuity");
    let summary = SessionSummary::from_draft(
        draft,
        1,
        &CompactionValidationContext::for_task(&project, &facts),
    )
    .unwrap();
    assert_eq!(summary.historical_byte_limit(), Some(65536));
    let durable = serde_json::to_string(&summary).unwrap();
    let fingerprint = summary.fingerprint().to_owned();
    let provenance = serde_json::to_value(summary.context_provenance().unwrap().unwrap()).unwrap();
    let projection = summary.model_projection().unwrap();
    assert_eq!(
        projection["narrative"],
        serde_json::to_value(narrative).unwrap()
    );
    assert!(projection["evidence_spine"].is_null());
    assert!(projection.get("task_references").is_none());
    let item = summary.context_item().unwrap();
    let ContentItem::Text { text, .. } = item else {
        panic!("summary must be text")
    };
    let original_bytes = "derived_session_summary_v4:".len() + durable.len();
    assert!(text.len() < durable.len() / 2);
    assert_eq!(serde_json::to_string(&summary).unwrap(), durable);
    assert_eq!(summary.fingerprint(), fingerprint);
    let references = serde_json::to_value(facts.references(128)).unwrap();
    let receipt_set = |value: &Value| {
        value
            .as_array()
            .unwrap()
            .iter()
            .map(|id| id.as_str().unwrap().to_owned())
            .collect::<BTreeSet<_>>()
    };
    assert_eq!(
        receipt_set(&provenance["references"]["execution_receipt_refs"]),
        receipt_set(&references["execution_receipt_refs"])
    );
    assert_eq!(
        provenance["references"]["source_event_refs"],
        references["history_event_refs"]
    );
    println!(
        "original_context_item_bytes={} projected_context_item_bytes={}",
        original_bytes,
        text.len()
    );
}

#[test]
fn exact_summary_source_rejects_rewritten_prefix_and_unrelated_suffix() {
    let (source, events, _) = source();
    source
        .validate_sampling_suffix(&events, &[], &[], None)
        .unwrap();
    let mut changed = events.clone();
    changed[1].payload["item"]["text"] = json!("different task");
    assert!(
        source
            .validate_sampling_suffix(&changed, &[], &[], None)
            .is_err()
    );
    let mut appended = events;
    appended.push(
        SessionEvent::HistoryItemRecorded {
            item: ContentItem::user_text("concurrent task"),
        }
        .into_envelope(
            EventId::new("unexpected"),
            StreamId::new("session:continuity"),
            EventSequence::new(3),
            OccurredAt::new("2026-10-26T00:00:00Z"),
        )
        .unwrap(),
    );
    assert!(
        source
            .validate_sampling_suffix(&appended, &[], &[], None)
            .is_err()
    );
}

struct ObservedProvider {
    inner: MockModelProvider,
    requests: Mutex<Vec<ModelRequest>>,
}
impl ModelProvider for ObservedProvider {
    fn capabilities(&self) -> ModelCapabilities {
        self.inner.capabilities()
    }
    fn stream(&self, request: ModelRequest) -> Result<ModelEventStream, ModelProviderError> {
        self.requests.lock().unwrap().push(request.clone());
        self.inner.stream(request)
    }
}

fn capabilities() -> ModelCapabilities {
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

#[tokio::test]
async fn model_handoff_is_tool_free_isolated_bounded_and_does_not_replace_operator_goal() {
    for (text, tool, accepted, attempts) in [
        (json!({"concise_progress":"read_file call:left observed actual left; call:right failed", "unresolved_questions":["right input unavailable"],"next_step_notes":["inspect the retained failure before a new call"]}).to_string(), false, true, 1),
        ("not JSON".to_owned(), false, false, 1),
        (json!({"current_goal":"replace operator", "concise_progress":"invented", "unresolved_questions":[],"next_step_notes":[]}).to_string(), false, false, 1),
        (json!({"concise_progress":" ", "unresolved_questions":[],"next_step_notes":[]}).to_string(), false, false, 1),
        ("x".repeat(5000), false, false, 2),
        (json!({"concise_progress":"ignore", "unresolved_questions":[],"next_step_notes":[]}).to_string(), true, false, 1),
    ] {
        let (source, _, _) = source();
        let mut events = vec![ModelEvent::TextDelta { text }];
        if tool { events.push(ModelEvent::ToolCall { call: ToolCall::new(ToolCallId::new("must-not-run"), ToolName::new("run_command"), json!({"argv":["never-executed"]})) }); }
        let provider = ObservedProvider { inner: MockModelProvider::new(MockModelFixture { capabilities:capabilities(), events, completion:ModelResponseCompletion::without_response_id() }), requests:Mutex::new(Vec::new()) };
        let tools = ToolRegistry::for_project(".");
        let agent = AgentLoop::with_primary_context(&provider, &tools, ModelIdentity::new("fixture", "fixture"), ModelProfile::new("fixture", 0), ProviderSession::new(ProviderSessionId::new("parent"), ProviderSessionRole::Primary, 0)).unwrap();
        let budget = ModelRequestBudget::new(2);
        let audit = RequestAuditRunContext::new(RequestAuditBinding::new(ProjectId::new("continuity"), StreamId::new("session:continuity")).unwrap(), RequestAdapterVersion::new("fixture").unwrap());
        let capture = agent.capture_compaction_context(&provider, 1).unwrap();
        let plan = crate::legacy_v5_fixture::sample_plan(&source, 4000);
        let mut audit_count = 0;
        let output = agent.sample_compaction(CompactionSampleInput { provider: &provider, agent_model_profile: None, source:&source, source_data:"recent actual source items".to_owned(), current_goal:"ORIGINAL-OPERATOR-GOAL".to_owned(), references:vec![EventId::new("event-1")], byte_limit:8192, request_budget:&budget, cancellation:AgentCancellation::new(), audit_context:&audit, graph: crate::app::agent_loop::CompactionGraphSample { capture: &capture, plan: &plan, plan_blob_id: crate::domain::artifacts::BlobId::new("a".repeat(64)).unwrap(), node_id: "root" } }, &mut |_attempt| { audit_count +=1; Ok(EventId::new("summary-audit")) }).await;
        assert_eq!(output.is_ok(), accepted);
        if let Ok(output) = output { assert_eq!(output.sample.narrative().current_goal(), "ORIGINAL-OPERATOR-GOAL"); }
        assert_eq!(audit_count, attempts);
        assert_eq!(budget.counts().primary, attempts);
        let requests = provider.requests.lock().unwrap();
        assert_eq!(requests.len(),attempts);
        assert!(requests[0].input().tool_specs.is_empty());
        assert!(!requests[0].is_incremental());
        assert_eq!(requests[0].continuation_retention(),ContinuationRetention::SingleSample);
        assert_eq!(agent.history().items(), &[]);
    }
}

#[tokio::test]
async fn handoff_minimum_output_is_checked_before_admission_and_audit() {
    let handoff = json!({"concise_progress":"a", "unresolved_questions":[], "next_step_notes":[]})
        .to_string();
    let minimum = handoff.len();
    for (available, accepted) in [(minimum - 1, false), (minimum, true)] {
        let (source, _, _) = source();
        let provider = ObservedProvider {
            inner: MockModelProvider::new(MockModelFixture {
                capabilities: capabilities(),
                events: vec![ModelEvent::TextDelta {
                    text: handoff.clone(),
                }],
                completion: ModelResponseCompletion::without_response_id(),
            }),
            requests: Mutex::new(Vec::new()),
        };
        let tools = ToolRegistry::for_project(".");
        let agent = AgentLoop::with_primary_context(
            &provider,
            &tools,
            ModelIdentity::new("fixture", "fixture"),
            ModelProfile::new("fixture", 0),
            ProviderSession::new(
                ProviderSessionId::new("parent"),
                ProviderSessionRole::Primary,
                0,
            ),
        )
        .unwrap();
        let budget = ModelRequestBudget::new(2);
        let audit = RequestAuditRunContext::new(
            RequestAuditBinding::new(
                ProjectId::new("continuity"),
                StreamId::new("session:continuity"),
            )
            .unwrap(),
            RequestAdapterVersion::new("fixture").unwrap(),
        );
        let capture = agent.capture_compaction_context(&provider, 1).unwrap();
        let plan = crate::legacy_v5_fixture::sample_plan(&source, available);
        let mut audit_count = 0;
        let result = agent
            .sample_compaction(
                CompactionSampleInput {
                    provider: &provider,
                    agent_model_profile: None,
                    source: &source,
                    source_data: "source".to_owned(),
                    current_goal: "goal".to_owned(),
                    references: vec![EventId::new("event-1")],
                    byte_limit: 8192,
                    request_budget: &budget,
                    cancellation: AgentCancellation::new(),
                    audit_context: &audit,
                    graph: crate::app::agent_loop::CompactionGraphSample {
                        capture: &capture,
                        plan: &plan,
                        plan_blob_id: crate::domain::artifacts::BlobId::new("a".repeat(64))
                            .unwrap(),
                        node_id: "root",
                    },
                },
                &mut |_attempt| {
                    audit_count += 1;
                    Ok(EventId::new("summary-audit"))
                },
            )
            .await;
        assert_eq!(result.is_ok(), accepted);
        assert_eq!(audit_count, usize::from(accepted));
        assert_eq!(budget.counts().primary, usize::from(accepted));
        assert_eq!(
            provider.requests.lock().unwrap().len(),
            usize::from(accepted)
        );
    }
}

#[tokio::test]
async fn oversized_handoff_regenerates_once_with_shared_attempts_and_parent_isolation() {
    use crate::ports::model::ModelStreamEvent;
    use std::collections::VecDeque;

    struct Attempts {
        events: Mutex<VecDeque<Vec<Result<ModelStreamEvent, ModelProviderError>>>>,
        requests: Mutex<Vec<ModelRequest>>,
    }
    impl ModelProvider for Attempts {
        fn capabilities(&self) -> ModelCapabilities {
            let mut value = capabilities();
            value.context_window = 272_000;
            value
        }
        fn stream(&self, request: ModelRequest) -> Result<ModelEventStream, ModelProviderError> {
            self.requests.lock().unwrap().push(request);
            let events = self
                .events
                .lock()
                .unwrap()
                .pop_front()
                .expect("bounded attempt");
            Ok(Box::pin(futures_util::stream::iter(events)))
        }
    }

    let handoff = json!({"concise_progress":"界".repeat(24), "unresolved_questions":[], "next_step_notes":[]}).to_string();
    let maximum = handoff.len();
    let oversized = "界".repeat(maximum / 3 + 1);
    for (script, limit, expected_attempts, accepted) in [
        (vec!["size", "ok"], 3, 2, true),
        (vec!["transport", "size", "ok"], 3, 3, true),
        (vec!["size", "transport", "ok"], 3, 3, true),
        (vec!["size", "size", "ok"], 3, 2, false),
        (vec!["transport", "transport", "size"], 3, 3, false),
        (vec!["size", "ok"], 1, 1, false),
    ] {
        let events = script
            .into_iter()
            .map(|kind| match kind {
                "size" => vec![
                    Ok(ModelStreamEvent::Event(ModelEvent::TextDelta {
                        text: oversized.clone(),
                    })),
                    Ok(ModelStreamEvent::Completed(
                        ModelResponseCompletion::without_response_id(),
                    )),
                ],
                "transport" => vec![
                    Ok(ModelStreamEvent::Event(ModelEvent::TextDelta {
                        text: "abandoned".to_owned(),
                    })),
                    Err(ModelProviderError::ResponseInterrupted {
                        status: 200,
                        elapsed_ms: 1,
                        timeout: false,
                        io_kind: "UnexpectedEof".to_owned(),
                    }),
                ],
                "ok" => vec![
                    Ok(ModelStreamEvent::Event(ModelEvent::TextDelta {
                        text: handoff.clone(),
                    })),
                    Ok(ModelStreamEvent::Completed(
                        ModelResponseCompletion::without_response_id(),
                    )),
                ],
                _ => unreachable!(),
            })
            .collect();
        let provider = Attempts {
            events: Mutex::new(events),
            requests: Mutex::new(Vec::new()),
        };
        let parent = MockModelProvider::new(MockModelFixture {
            capabilities: provider.capabilities(),
            events: vec![
                ModelEvent::ReasoningItem {
                    text: "PARENT-REASONING".to_owned(),
                },
                ModelEvent::TextDelta {
                    text: "PARENT-COMPLETED".to_owned(),
                },
            ],
            completion: ModelResponseCompletion::without_response_id(),
        });
        let tools = ToolRegistry::for_project(".");
        let mut agent = AgentLoop::with_primary_context(
            &parent,
            &tools,
            ModelIdentity::new("fixture", "fixture"),
            ModelProfile::new("fixture", 0),
            ProviderSession::new(
                ProviderSessionId::new("parent"),
                ProviderSessionRole::Primary,
                0,
            ),
        )
        .unwrap();
        agent
            .run_turn("ORIGINAL-OPERATOR-GOAL", AgentCancellation::new())
            .await
            .unwrap();
        let parent_history = agent.history().items().to_vec();
        let parent_delivery = agent.delivery_input_estimate().unwrap();
        let (source, _, _) = source();
        let capture = agent.capture_compaction_context(&provider, 1).unwrap();
        let plan = crate::legacy_v5_fixture::sample_plan(&source, maximum);
        let budget = ModelRequestBudget::new(limit);
        let audit = RequestAuditRunContext::new(
            RequestAuditBinding::new(
                ProjectId::new("continuity"),
                StreamId::new("session:continuity"),
            )
            .unwrap(),
            RequestAdapterVersion::new("fixture").unwrap(),
        );
        let mut audits = Vec::new();
        let output = agent
            .sample_compaction(
                CompactionSampleInput {
                    provider: &provider,
                    agent_model_profile: None,
                    source: &source,
                    source_data: "same frozen source".to_owned(),
                    current_goal: "ORIGINAL-OPERATOR-GOAL".to_owned(),
                    references: vec![EventId::new("event-1")],
                    byte_limit: 8192,
                    request_budget: &budget,
                    cancellation: AgentCancellation::new(),
                    audit_context: &audit,
                    graph: crate::app::agent_loop::CompactionGraphSample {
                        capture: &capture,
                        plan: &plan,
                        plan_blob_id: crate::domain::artifacts::BlobId::new("a".repeat(64))
                            .unwrap(),
                        node_id: "root",
                    },
                },
                &mut |attempt| {
                    audits.push(attempt);
                    Ok(EventId::new(format!("size-audit-{}", audits.len())))
                },
            )
            .await;
        assert_eq!(
            output.is_ok(),
            accepted,
            "{expected_attempts} attempts, quota {limit}: {}",
            output
                .as_ref()
                .err()
                .map(ToString::to_string)
                .unwrap_or_default()
        );
        assert_eq!(audits.len(), expected_attempts);
        assert_eq!(budget.counts().primary, expected_attempts);
        let requests = provider.requests.lock().unwrap();
        assert_eq!(requests.len(), expected_attempts);
        assert!(requests.iter().all(|request| request == &requests[0]));
        assert!(requests[0].input().tool_specs.is_empty());
        assert_eq!(
            requests[0].continuation_retention(),
            ContinuationRetention::SingleSample
        );
        if let Ok(output) = output {
            assert_eq!(
                output.sample.audit_event_id(),
                &EventId::new(format!("size-audit-{expected_attempts}"))
            );
            assert_eq!(output.sample.graph_node().unwrap().3, handoff);
        } else if limit > 1 {
            let error = output.err().unwrap().to_string();
            assert!(
                error.contains(&format!("actual={}", oversized.len())),
                "{error}"
            );
            assert!(error.contains(&format!("maximum={maximum}")), "{error}");
        }
        assert_eq!(agent.history().items(), parent_history);
        assert_eq!(agent.delivery_input_estimate().unwrap(), parent_delivery);
    }
}

#[test]
fn canonical_many_prefixes_match_full_hashes_without_reserializing_archived_bodies() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Counted<'a> {
        body: Value,
        count: &'a AtomicUsize,
    }
    impl serde::Serialize for Counted<'_> {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            self.count.fetch_add(1, Ordering::Relaxed);
            self.body.serialize(serializer)
        }
    }
    let count = AtomicUsize::new(0);
    let items = (0..256).map(|index| Counted { body: json!({"z":index,"a":{"unicode":"中\n\"", "body":"neutral archived observation ".repeat(160)}}), count: &count }).collect::<Vec<_>>();
    let lengths = (0..=32).map(|index| index * 8).collect::<BTreeSet<_>>();
    let started = std::time::Instant::now();
    let hashes =
        crate::domain::canonical_json::canonical_array_prefix_hashes(&items, &lengths).unwrap();
    eprintln!(
        "32-prefix canonical probe: {} events serialized once in {:?}",
        items.len(),
        started.elapsed()
    );
    assert_eq!(count.load(Ordering::Relaxed), items.len());
    for length in lengths {
        assert_eq!(
            hashes[&length],
            crate::domain::canonical_json::canonical_blake3_hash(&&items[..length]).unwrap()
        );
    }
}

#[tokio::test]
async fn cancelled_or_exhausted_summary_never_audits_or_dispatches() {
    for cancelled in [false, true] {
        let (source, _, _) = source();
        let provider = ObservedProvider {
            inner: MockModelProvider::new(MockModelFixture {
                capabilities: capabilities(),
                events: vec![],
                completion: ModelResponseCompletion::without_response_id(),
            }),
            requests: Mutex::new(Vec::new()),
        };
        let tools = ToolRegistry::for_project(".");
        let agent = AgentLoop::with_primary_context(
            &provider,
            &tools,
            ModelIdentity::new("fixture", "fixture"),
            ModelProfile::new("fixture", 0),
            ProviderSession::new(
                ProviderSessionId::new("parent"),
                ProviderSessionRole::Primary,
                0,
            ),
        )
        .unwrap();
        let budget = ModelRequestBudget::new(1);
        let cancellation = AgentCancellation::new();
        if cancelled {
            cancellation.cancel();
        } else {
            budget.admit(ProviderSessionRole::Primary).unwrap();
        }
        let audit = RequestAuditRunContext::new(
            RequestAuditBinding::new(
                ProjectId::new("continuity"),
                StreamId::new("session:continuity"),
            )
            .unwrap(),
            RequestAdapterVersion::new("fixture").unwrap(),
        );
        let capture = agent.capture_compaction_context(&provider, 1).unwrap();
        let plan = crate::legacy_v5_fixture::sample_plan(&source, 4000);
        let result = agent
            .sample_compaction(
                CompactionSampleInput {
                    provider: &provider,
                    agent_model_profile: None,
                    source: &source,
                    source_data: "neutral source".to_owned(),
                    current_goal: "ORIGINAL-OPERATOR-GOAL".to_owned(),
                    references: vec![EventId::new("event-1")],
                    byte_limit: 8192,
                    request_budget: &budget,
                    cancellation,
                    audit_context: &audit,
                    graph: crate::app::agent_loop::CompactionGraphSample {
                        capture: &capture,
                        plan: &plan,
                        plan_blob_id: crate::domain::artifacts::BlobId::new("a".repeat(64))
                            .unwrap(),
                        node_id: "root",
                    },
                },
                &mut |_| panic!("no summary audit allowed"),
            )
            .await;
        if cancelled {
            assert!(matches!(
                result,
                Err(crate::app::agent_loop::AgentLoopError::CompactionCancelled)
            ));
        } else {
            assert!(matches!(
                result,
                Err(crate::app::agent_loop::AgentLoopError::ModelRequestLimit { .. })
            ));
        }
        assert!(provider.requests.lock().unwrap().is_empty());
        assert!(agent.history().items().is_empty());
    }
}

#[test]
fn historical_sample_rejects_json_expanded_valid_operator_goal() {
    let (_, mut events, _) = source();
    let raw_goal = format!("x{}", "\u{1}".repeat(1535));
    for (index, event) in [
        SessionEvent::HistoryItemRecorded {
            item: ContentItem::user_text(raw_goal.clone()),
        },
        SessionEvent::HistoryItemRecorded {
            item: ContentItem::assistant_text("PARENT-DURABLE-PROGRESS"),
        },
        SessionEvent::Resumed {
            primary_provider_session: ProviderSession::new(
                ProviderSessionId::new("parent-resumed"),
                ProviderSessionRole::Primary,
                1,
            )
            .reference()
            .clone(),
        },
    ]
    .into_iter()
    .enumerate()
    {
        let envelope = event
            .into_envelope(
                EventId::new(format!("valid-goal-{index}")),
                StreamId::new("session:continuity"),
                EventSequence::new(index as u64 + 2),
                OccurredAt::new("2026-10-26T00:00:00Z"),
            )
            .unwrap();
        if index == 0 {
            events[1] = envelope;
        } else {
            events.push(envelope);
        }
    }
    let (source, state) =
        SessionSourceIdentity::from_events(ProjectId::new("continuity"), &events, &[]).unwrap();
    let policy = crate::domain::context::ContextWindowPolicy::for_window(272000);
    let (goal, _) = crate::adapters::research_projection::continuation_text(
        &state.full_normalized_history().unwrap(),
        None,
        policy,
    )
    .unwrap();
    assert_eq!(goal, raw_goal);
    assert_eq!(goal.len(), 1536);
    assert!(goal.len() <= policy.continuation_field_bytes());
    assert!(serde_json::to_vec(&goal).unwrap().len() > 4096);
    let narrative =
        crate::domain::session::NarrativeSummary::new(goal, "progress", vec![], vec![]).unwrap();
    let sample =
        crate::legacy_v5_fixture::historical_sample(&source, EventId::new("audit"), narrative);
    let error = sample.validate().unwrap_err();
    assert!(
        error
            .to_string()
            .contains("invalid bounded Compaction model sample")
    );
    let valid = crate::legacy_v5_fixture::historical_sample(
        &source,
        EventId::new("audit"),
        crate::domain::session::NarrativeSummary::new("goal", "progress", vec![], vec![]).unwrap(),
    );
    valid.validate().unwrap();
}

#[tokio::test]
async fn audit_append_failure_stops_summary_before_dispatch_without_replacing_parent() {
    let (source, _, _) = source();
    let provider = ObservedProvider {
        inner: MockModelProvider::new(MockModelFixture {
            capabilities: capabilities(),
            events: vec![],
            completion: ModelResponseCompletion::without_response_id(),
        }),
        requests: Mutex::new(Vec::new()),
    };
    let tools = ToolRegistry::for_project(".");
    let agent = AgentLoop::with_primary_context(
        &provider,
        &tools,
        ModelIdentity::new("fixture", "fixture"),
        ModelProfile::new("fixture", 0),
        ProviderSession::new(
            ProviderSessionId::new("parent"),
            ProviderSessionRole::Primary,
            0,
        ),
    )
    .unwrap();
    let budget = ModelRequestBudget::new(2);
    let audit = RequestAuditRunContext::new(
        RequestAuditBinding::new(
            ProjectId::new("continuity"),
            StreamId::new("session:continuity"),
        )
        .unwrap(),
        RequestAdapterVersion::new("fixture").unwrap(),
    );
    let capture = agent.capture_compaction_context(&provider, 1).unwrap();
    let plan = crate::legacy_v5_fixture::sample_plan(&source, 4000);
    let result = agent
        .sample_compaction(
            CompactionSampleInput {
                provider: &provider,
                agent_model_profile: None,
                source: &source,
                source_data: "neutral source".to_owned(),
                current_goal: "ORIGINAL-OPERATOR-GOAL".to_owned(),
                references: vec![EventId::new("event-1")],
                byte_limit: 8192,
                request_budget: &budget,
                cancellation: AgentCancellation::new(),
                audit_context: &audit,
                graph: crate::app::agent_loop::CompactionGraphSample {
                    capture: &capture,
                    plan: &plan,
                    plan_blob_id: crate::domain::artifacts::BlobId::new("a".repeat(64)).unwrap(),
                    node_id: "root",
                },
            },
            &mut |_| {
                Err(crate::app::agent_loop::AgentLoopError::RequestAuditCommit {
                    message: "injected durable append failure".to_owned(),
                })
            },
        )
        .await;
    assert!(matches!(
        result,
        Err(crate::app::agent_loop::AgentLoopError::RequestAuditCommit { .. })
    ));
    assert_eq!(budget.counts().primary, 1);
    assert!(provider.requests.lock().unwrap().is_empty());
    assert!(agent.history().items().is_empty());
}

#[tokio::test]
async fn historical_v5_is_valid_after_later_history_but_cannot_authorize_a_fresh_summary() {
    use crate::adapters::project_event_store::Project;
    use crate::adapters::session_compaction::{
        load_resume_compaction_facts, load_session_compaction_facts,
    };
    use crate::domain::context::ContextWindowPolicy;
    use crate::domain::session::SessionReducer;
    use crate::ports::EventSink;
    use std::io::Write;
    use std::process::{Command, Stdio};
    fn research_snapshot(project: &Project, label: &str) {
        use crate::adapters::project_event_store::ProjectEventStream;
        use crate::domain::research::{
            ResearchEvent, TargetKind, TargetSnapshotDraft, TargetSnapshotId, TargetSnapshotRecord,
        };
        let snapshot = TargetSnapshotRecord::from_draft(TargetSnapshotDraft {
            id: TargetSnapshotId::new(format!("snapshot-{label}")),
            target_id: "fixture".into(),
            kind: TargetKind::Source,
            locator: format!("src/{label}"),
            version: "1".into(),
            build_id: format!("build-{label}"),
            target_artifact_ids: vec![],
            environment_fingerprint: "environment".into(),
            device_fingerprint: "device".into(),
            captured_at: "2026-10-26T00:00:00Z".into(),
            notes: String::new(),
        })
        .unwrap();
        let mut writer = project.open_writer().unwrap();
        let mut store = writer.open_research(None).unwrap();
        let event = ResearchEvent::TargetSnapshotRecorded(snapshot);
        let envelope = EventEnvelope::current(
            EventId::new(format!("research-{label}")),
            ProjectEventStream::Research.stream_id(),
            store.next_sequence().unwrap(),
            OccurredAt::new("2026-10-26T00:00:00Z"),
            crate::domain::events::EventType::new(event.event_type().as_str()),
            event.encode_payload().unwrap(),
        );
        store.append_event(&envelope).unwrap();
    }
    fn push(events: &mut Vec<EventEnvelope<Value>>, event: SessionEvent) {
        let index = events.len() + 1;
        events.push(
            event
                .into_envelope(
                    EventId::new(format!("event-{index}")),
                    StreamId::new("session:continuity"),
                    EventSequence::new(index as u64),
                    OccurredAt::new("2026-10-26T00:00:00Z"),
                )
                .unwrap(),
        );
    }
    let root = crate::support::TemporaryDirectory::new("compaction-sample-expiry");
    let project = Project::initialize(root.path()).unwrap();
    research_snapshot(&project, "before");
    let (_, mut events, _) = source();
    let tools = ToolRegistry::for_project(root.path());
    let binding = bind_definition(
        builtin_definition(
            BuiltinAgentTemplate::Research,
            &crate::ports::tool::ToolExecutor::catalog(&tools),
        )
        .unwrap(),
        None,
        None,
    )
    .unwrap();
    events[0] = SessionEvent::StartedWithAgent {
        primary_provider_session: ProviderSession::new(
            ProviderSessionId::new("parent"),
            ProviderSessionRole::Primary,
            0,
        )
        .reference()
        .clone(),
        binding,
    }
    .into_envelope(
        EventId::new("event-0"),
        StreamId::new("session:continuity"),
        EventSequence::new(1),
        OccurredAt::new("2026-10-26T00:00:00Z"),
    )
    .unwrap();
    push(
        &mut events,
        SessionEvent::HistoryItemRecorded {
            item: ContentItem::assistant_text("Actual earlier neutral observation. ".repeat(400)),
        },
    );
    let research =
        crate::adapters::project_event_store::read_existing_research_events(&project).unwrap();
    let (source, _) =
        SessionSourceIdentity::from_events(project.project_id().clone(), &events, &research)
            .unwrap();
    let audit = RequestAuditRunContext::new(
        RequestAuditBinding::new(
            project.project_id().clone(),
            StreamId::new("session:continuity"),
        )
        .unwrap(),
        RequestAdapterVersion::new("fixture").unwrap(),
    );
    let audit_id = EventId::new(format!("event-{}", events.len() + 1));
    let manifest = crate::legacy_v5_fixture::historical_audit(
        &source,
        audit_id.clone(),
        &audit,
        capabilities(),
    );
    push(&mut events, SessionEvent::RequestAuditRecorded { manifest });
    let narrative = crate::domain::session::NarrativeSummary::new(
        "ORIGINAL-OPERATOR-GOAL",
        "Actual neutral observation completed.",
        vec![],
        vec!["Continue without repeating completed work.".into()],
    )
    .unwrap();
    let sample = crate::legacy_v5_fixture::historical_sample(&source, audit_id, narrative.clone());
    push(
        &mut events,
        SessionEvent::CompactionSampleRecorded { sample },
    );
    let sample_id = events.last().unwrap().event_id.clone();
    let session_id = StreamId::new("continuity");
    {
        let mut writer = project.open_writer().unwrap();
        let mut store = writer.open_session(session_id.clone(), None).unwrap();
        for event in &events {
            store.append_event(event).unwrap();
        }
    }
    let state = SessionReducer.reduce(events.clone()).unwrap();
    let facts = load_session_compaction_facts(&project, &state).unwrap();
    let draft = facts
        .compose_session_summary_draft(
            state.history().items(),
            None,
            32,
            ContextWindowPolicy::for_window(32768),
        )
        .unwrap();
    let compaction = crate::legacy_v5_fixture::legacy_v5_candidate(
        &state,
        draft,
        &facts.context(),
        sample_id.clone(),
        narrative.clone(),
    );
    compaction.validate_references(&facts.context()).unwrap();
    let historical = compaction.summary().clone();
    let previous_len = events.len();
    push(&mut events, SessionEvent::CompactionCreated { compaction });
    push(
        &mut events,
        SessionEvent::HistoryItemRecorded {
            item: ContentItem::user_text("A new operator instruction after the summary"),
        },
    );
    push(
        &mut events,
        SessionEvent::HistoryItemRecorded {
            item: ContentItem::assistant_text("New actual work after the old summary"),
        },
    );
    {
        let mut writer = project.open_writer().unwrap();
        let mut store = writer.open_session(session_id, None).unwrap();
        for event in &events[previous_len..] {
            store.append_event(event).unwrap();
        }
    }
    let read = crate::adapters::research_projection::ProjectReadContext::load(
        &project,
        &mut crate::adapters::research_projection::SessionReceiptCatalogCache::default(),
        None,
    )
    .unwrap();
    historical
        .validate_references(
            &load_resume_compaction_facts(&project, &historical, &read, None)
                .unwrap()
                .context(),
        )
        .unwrap();
    let state = SessionReducer.reduce(events).unwrap();
    let facts = load_session_compaction_facts(&project, &state).unwrap();
    let stale = facts
        .compose_session_summary_draft(
            state.history().items(),
            Some(&historical),
            32,
            ContextWindowPolicy::for_window(32768),
        )
        .unwrap();
    let stale = crate::legacy_v5_fixture::legacy_v5_candidate(
        &state,
        stale,
        &facts.context(),
        sample_id,
        narrative,
    );
    let error = stale.validate_references(&facts.context()).unwrap_err();
    assert!(
        error.to_string().contains("recorded model handoff"),
        "{error}"
    );
    let config = root.path().join("provider.toml");
    std::fs::write(&config, crate::provider_config::fixture()).unwrap();
    let chat = |input: &str| {
        let mut child = Command::new(env!("CARGO_BIN_EXE_echidna"))
            .current_dir(root.path())
            .args([
                "chat",
                "--config",
                config.to_str().unwrap(),
                "--resume",
                "continuity",
            ])
            .env("CYBER_AGENT_PRIMARY_API_KEY", "fake")
            .env("CYBER_AGENT_VISION_API_KEY", "fake")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    };
    let stopped = chat(&format!("{}\n/exit\n", "PROTECTED-OVERSIZED-".repeat(6000)));
    assert!(
        !stopped.status.success(),
        "{}",
        String::from_utf8_lossy(&stopped.stdout)
    );
    assert!(
        String::from_utf8_lossy(&stopped.stderr).contains("required Context fragments"),
        "{}",
        String::from_utf8_lossy(&stopped.stderr)
    );
    let before = crate::adapters::project_event_store::read_existing_session_events(
        &project,
        &StreamId::new("continuity"),
    )
    .unwrap();
    let before = SessionReducer.reduce(before).unwrap();
    assert_eq!(
        before
            .latest_compaction()
            .unwrap()
            .summary()
            .schema_version(),
        5
    );
    research_snapshot(&project, "after");
    let resumed = chat("/context\n/compact\nFORBIDDEN-NEW-TASK\n/exit\n");
    let stdout = String::from_utf8_lossy(&resumed.stdout);
    assert!(!resumed.status.success());
    assert!(
        stdout.contains("[current refreshed candidate]"),
        "{stdout}; stderr={}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    assert!(
        stdout.contains("Compaction could not find a smaller admissible Context"),
        "{stdout}"
    );
    assert!(
        stdout.contains("Task input not accepted or saved"),
        "{stdout}"
    );
    let after = crate::adapters::project_event_store::read_existing_session_events(
        &project,
        &StreamId::new("continuity"),
    )
    .unwrap();
    let after = SessionReducer.reduce(after).unwrap();
    assert_eq!(after.history(), before.history());
    assert_eq!(after.request_audits(), before.request_audits());
    assert_eq!(
        after.execution_receipt_count(),
        before.execution_receipt_count()
    );
    assert_eq!(after.latest_compaction().unwrap().summary().revision(), 2);
}
