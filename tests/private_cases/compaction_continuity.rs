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
    for (text, tool, accepted) in [
        (json!({"concise_progress":"read_file call:left observed actual left; call:right failed", "unresolved_questions":["right input unavailable"],"next_step_notes":["inspect the retained failure before a new call"]}).to_string(), false, true),
        ("not JSON".to_owned(), false, false),
        (json!({"current_goal":"replace operator", "concise_progress":"invented", "unresolved_questions":[],"next_step_notes":[]}).to_string(), false, false),
        (json!({"concise_progress":" ", "unresolved_questions":[],"next_step_notes":[]}).to_string(), false, false),
        ("x".repeat(5000), false, false),
        (json!({"concise_progress":"ignore", "unresolved_questions":[],"next_step_notes":[]}).to_string(), true, false),
    ] {
        let (source, _, _) = source();
        let mut events = vec![ModelEvent::TextDelta { text }];
        if tool { events.push(ModelEvent::ToolCall { call: ToolCall::new(ToolCallId::new("must-not-run"), ToolName::new("run_command"), json!({"argv":["never-executed"]})) }); }
        let provider = ObservedProvider { inner: MockModelProvider::new(MockModelFixture { capabilities:capabilities(), events, completion:ModelResponseCompletion::without_response_id() }), requests:Mutex::new(Vec::new()) };
        let tools = ToolRegistry::for_project(".");
        let agent = AgentLoop::with_primary_context(&provider, &tools, ModelIdentity::new("fixture", "fixture"), ModelProfile::new("fixture", 0), ProviderSession::new(ProviderSessionId::new("parent"), ProviderSessionRole::Primary, 0)).unwrap();
        let budget = ModelRequestBudget::new(2);
        let audit = RequestAuditRunContext::new(RequestAuditBinding::new(ProjectId::new("continuity"), StreamId::new("session:continuity")).unwrap(), RequestAdapterVersion::new("fixture").unwrap());
        let mut audit_count = 0;
        let output = agent.sample_compaction(CompactionSampleInput { provider: &provider, agent_model_profile: None, source:&source, source_data:"recent actual source items".to_owned(), current_goal:"ORIGINAL-OPERATOR-GOAL".to_owned(), references:vec![EventId::new("event-1")], byte_limit:8192, request_budget:&budget, cancellation:AgentCancellation::new(), audit_context:&audit }, &mut |_attempt| { audit_count +=1; Ok(EventId::new("summary-audit")) }).await;
        assert_eq!(output.is_ok(), accepted);
        if let Ok(output) = output { assert_eq!(output.sample.narrative().current_goal(), "ORIGINAL-OPERATOR-GOAL"); }
        assert_eq!(audit_count, 1);
        assert_eq!(budget.counts().primary, 1);
        let requests = provider.requests.lock().unwrap();
        assert_eq!(requests.len(),1);
        assert!(requests[0].input().tool_specs.is_empty());
        assert!(!requests[0].is_incremental());
        assert_eq!(requests[0].continuation_retention(),ContinuationRetention::SingleSample);
        assert_eq!(agent.history().items(), &[]);
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

#[tokio::test]
async fn json_expanded_valid_operator_goal_is_rejected_before_audit_or_request() {
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
    let (goal, _) = crate::adapters::research_projection::continuation_text(
        &state.full_normalized_history().unwrap(),
        None,
        crate::domain::context::ContextWindowPolicy::for_window(32768),
    )
    .unwrap();
    assert_eq!(goal, raw_goal);
    assert_eq!(goal.len(), 1536);
    assert!(serde_json::to_vec(&goal).unwrap().len() > 4096);
    let provider = ObservedProvider {
        inner: MockModelProvider::new(MockModelFixture {
            capabilities: capabilities(),
            events: vec![],
            completion: ModelResponseCompletion::without_response_id(),
        }),
        requests: Mutex::new(Vec::new()),
    };
    let tools = ToolRegistry::for_project(".");
    let mut agent = AgentLoop::with_primary_context(
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
    agent
        .resume_from_session(
            &state,
            ProviderSession::new(
                ProviderSessionId::new("parent-resumed"),
                ProviderSessionRole::Primary,
                1,
            ),
            None,
        )
        .unwrap();
    let before = agent.history().items().to_vec();
    let budget = ModelRequestBudget::new(2);
    let audit = RequestAuditRunContext::new(
        RequestAuditBinding::new(
            ProjectId::new("continuity"),
            StreamId::new("session:continuity"),
        )
        .unwrap(),
        RequestAdapterVersion::new("fixture").unwrap(),
    );
    let output = agent
        .sample_compaction(
            CompactionSampleInput {
                provider: &provider,
                agent_model_profile: None,
                source: &source,
                source_data: "actual valid control-character source".to_owned(),
                current_goal: goal,
                references: vec![EventId::new("valid-goal-0")],
                byte_limit: 8192,
                request_budget: &budget,
                cancellation: AgentCancellation::new(),
                audit_context: &audit,
            },
            &mut |_| panic!("oversized encoded goal must not audit"),
        )
        .await;
    assert!(matches!(
        output,
        Err(crate::app::agent_loop::AgentLoopError::CompactionSummaryRejected { .. })
    ));
    assert_eq!(budget.counts().primary, 0);
    assert!(provider.requests.lock().unwrap().is_empty());
    assert_eq!(agent.history().items(), before);
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
    use crate::domain::artifacts::BlobId;
    use crate::domain::context::ContextWindowPolicy;
    use crate::domain::request_audit::PublishedAuditBlob;
    use crate::domain::session::SessionReducer;
    use crate::ports::EventSink;
    use sha2::{Digest, Sha256};
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
    let (_, mut events, _) = source();
    push(
        &mut events,
        SessionEvent::HistoryItemRecorded {
            item: ContentItem::assistant_text("Actual earlier neutral observation. ".repeat(400)),
        },
    );
    let (source, _) =
        SessionSourceIdentity::from_events(project.project_id().clone(), &events, &[]).unwrap();
    let provider = MockModelProvider::new(MockModelFixture { capabilities:capabilities(), events:vec![ModelEvent::TextDelta { text:json!({"concise_progress":"Actual neutral observation completed.","unresolved_questions":[],"next_step_notes":["Continue without repeating completed work."]}).to_string() }], completion:ModelResponseCompletion::without_response_id() });
    let tools = ToolRegistry::for_project(root.path());
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
            project.project_id().clone(),
            StreamId::new("session:continuity"),
        )
        .unwrap(),
        RequestAdapterVersion::new("fixture").unwrap(),
    );
    let output = agent
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
            },
            &mut |attempt| {
                let rendered = attempt.render().unwrap();
                let blobs = rendered
                    .component_bytes()
                    .map(|bytes| PublishedAuditBlob {
                        blob_id: BlobId::new(format!("{:x}", Sha256::digest(bytes))).unwrap(),
                        size_bytes: bytes.len() as u64,
                    })
                    .collect::<Vec<_>>();
                let id = EventId::new(format!("event-{}", events.len() + 1));
                let manifest = rendered.materialize(id.clone(), blobs).unwrap();
                push(&mut events, SessionEvent::RequestAuditRecorded { manifest });
                Ok(id)
            },
        )
        .await
        .unwrap();
    push(
        &mut events,
        SessionEvent::CompactionSampleRecorded {
            sample: output.sample.clone(),
        },
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
    let facts = load_session_compaction_facts(&project, &state)
        .unwrap()
        .with_model_sample(sample_id.clone(), output.sample.narrative().clone());
    let draft = facts
        .compose_session_summary_draft(
            state.history().items(),
            None,
            32,
            ContextWindowPolicy::for_window(32768),
        )
        .unwrap();
    let compaction = state.next_compaction(draft, 0, &facts.context()).unwrap();
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
    let facts = load_session_compaction_facts(&project, &state)
        .unwrap()
        .with_model_sample(sample_id, output.sample.narrative().clone());
    let stale = facts
        .compose_session_summary_draft(
            state.history().items(),
            Some(&historical),
            32,
            ContextWindowPolicy::for_window(32768),
        )
        .unwrap();
    let error = state
        .next_compaction(stale, 0, &facts.context())
        .unwrap_err();
    assert!(
        error.to_string().contains("recorded model handoff"),
        "{error}"
    );
}
