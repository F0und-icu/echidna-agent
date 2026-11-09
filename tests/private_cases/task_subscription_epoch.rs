//! Real subscription wire and task-selection epoch over synthetic Session facts.
use std::cell::RefCell;
use std::convert::Infallible;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;

use crate::adapters::agent_definition::{load_definition_binding, write_builtin_definition};
use crate::adapters::artifact_store::LocalArtifactStore;
use crate::adapters::project_event_store::Project;
use crate::adapters::project_snapshot::LocalProjectSnapshot;
use crate::adapters::responses::{ResponsesHttpAdapter, ResponsesHttpConfig};
use crate::adapters::tools::ToolRegistry;
use crate::app::agent_loop::{
    AgentLoop, AgentLoopError, ChatDurabilityCheckpoint, CliRuntimeStages, CurrentContextState,
    TaskContextPrepared, TaskContextStage, TaskRefreshDecision, TaskSelectionMaterial,
    TaskSelectionSampleOutput, ViewImageFollowUpFuture,
};
use crate::app::cancellation::AgentCancellation;
use crate::app::model_request_budget::ModelRequestBudget;
use crate::app::request_audit::RequestAuditService;
use crate::app::session::SessionProjection;
use crate::domain::agent_definition::BuiltinAgentTemplate;
use crate::domain::context::{
    ProviderSession, ProviderSessionId, ProviderSessionRef, ProviderSessionRole,
};
use crate::domain::events::{EventEnvelope, EventId, EventSequence, OccurredAt, StreamId};
use crate::domain::request_audit::{
    RequestAuditAttempt, RequestAuditBinding, RequestAuditRunContext,
};
use crate::domain::research_control::PromptComposition;
use crate::domain::session::{
    CompactionContextCapture, SessionEvent, SessionSourceIdentity, TaskContextCapture,
    TaskContextSelected, TaskInputTime, TaskSelectionSample, TimeZoneObservation,
};
use crate::domain::task_context::TaskSelectionDecision;
use crate::domain::world_state::ProjectSnapshotContext;
use crate::ports::EventSink;
use crate::ports::artifact::ArtifactStore;
use crate::ports::model::{
    ModelEventStream, ModelProvider, ModelProviderError, ModelRequest, ModelStreamEvent,
    PreparedModelRequest,
};
use crate::ports::tool::ToolExecutor;
use crate::rsi_subscription_continuation::continuation_regression::{
    FixtureCredential, capabilities, encrypted, read_request,
};
use crate::support::TemporaryDirectory;

#[derive(Default)]
struct Sink(Vec<EventEnvelope<Value>>);
impl EventSink<Value> for Sink {
    type Error = Infallible;
    fn append_event(&mut self, event: &EventEnvelope<Value>) -> Result<(), Self::Error> {
        self.0.push(event.clone());
        Ok(())
    }
}

struct Facts {
    project: Project,
    projection: SessionProjection,
    sink: Sink,
    binding: RequestAuditBinding,
    primary_audit: Option<EventId>,
    captured: Option<(
        crate::domain::session::SessionState,
        TaskContextCapture,
        Vec<EventEnvelope<Value>>,
    )>,
}

impl Facts {
    fn id(&self) -> EventId {
        EventId::new(format!("epoch-event-{}", self.sink.0.len() + 1))
    }
    fn append(&mut self, payload: SessionEvent) -> Result<EventId, AgentLoopError> {
        let id = self.id();
        let event = payload.into_envelope(
            id.clone(),
            self.binding.session_stream_id().clone(),
            EventSequence::new(self.sink.0.len() as u64 + 1),
            OccurredAt::new("2026-11-05T00:00:00Z"),
        )?;
        self.projection
            .append(&mut self.sink, event)
            .map_err(|error| AgentLoopError::HistoryCheckpointCommit {
                message: format!("{error:?}"),
            })?;
        Ok(id)
    }
    fn audit(&mut self, attempt: RequestAuditAttempt) -> Result<EventId, AgentLoopError> {
        let store = LocalArtifactStore::for_project(&self.project);
        let service = RequestAuditService::new(&store, self.binding.clone());
        let id = self.id();
        let manifest = service
            .record(
                attempt,
                id.clone(),
                OccurredAt::new("2026-11-05T00:00:00Z"),
                &mut self.projection,
                &mut self.sink,
            )
            .map_err(|error| AgentLoopError::HistoryCheckpointCommit {
                message: format!("{error:?}"),
            })?;
        assert_eq!(manifest.audit_event_id(), &id);
        Ok(id)
    }
    fn complete(&mut self) -> Result<(), AgentLoopError> {
        let id = self.primary_audit.as_ref().unwrap();
        let audit = self
            .projection
            .state()
            .request_audits()
            .iter()
            .find(|audit| audit.audit_event_id() == id)
            .unwrap();
        let store = LocalArtifactStore::for_project(&self.project);
        let service = RequestAuditService::new(&store, self.binding.clone());
        let verified = service.reconstruct_full_logical(audit).unwrap().unwrap();
        let chain = self.projection.state().model_delivery_chain(id)?.unwrap();
        let mut delivered = std::collections::BTreeMap::new();
        for accepted in chain {
            let manifest = self
                .projection
                .state()
                .request_audits()
                .iter()
                .find(|audit| audit.audit_event_id() == accepted.audit_event_id())
                .unwrap();
            delivered.insert(
                accepted.audit_event_id().clone(),
                service.reconstruct(manifest).unwrap(),
            );
        }
        let boundary = self
            .projection
            .state()
            .completed_operator_turn(&verified, &delivered)?
            .unwrap();
        self.append(SessionEvent::OperatorTurnBoundaryRecorded { boundary })?;
        Ok(())
    }
}

struct FixtureStage(Rc<RefCell<Facts>>, Arc<dyn ModelProvider>);
impl TaskContextStage for FixtureStage {
    fn selection_provider(
        &self,
    ) -> (
        Arc<dyn ModelProvider>,
        Option<crate::domain::profile::AgentModelProfile>,
    ) {
        (Arc::clone(&self.1), None)
    }
    fn record_completed_usage(
        &mut self,
        capture: &EventId,
        audit: &EventId,
        scope: &ProviderSessionRef,
        usage: crate::domain::model::ModelUsage,
    ) -> Result<(), AgentLoopError> {
        self.0
            .borrow_mut()
            .append(SessionEvent::TaskSelectionUsageRecorded {
                capture_event_id: capture.clone(),
                request_audit_event_id: audit.clone(),
                provider_session: scope.clone(),
                usage,
            })?;
        Ok(())
    }
    fn control_directory(&self) -> &'static str {
        ".echidna"
    }
    fn current_protected_context(
        &mut self,
    ) -> Result<crate::domain::session::ProtectedOperatorContext, AgentLoopError> {
        Ok(self
            .0
            .borrow()
            .projection
            .state()
            .current_protected_operator_context()?)
    }
    fn refresh_decision(
        &mut self,
        prepared: &TaskContextPrepared,
    ) -> Result<TaskRefreshDecision, AgentLoopError> {
        let facts = self.0.borrow();
        let (state, _, _) = facts.captured.as_ref().unwrap();
        let (id, decision) = state.deterministic_task_refresh(
            &prepared.input_ref,
            &prepared.candidates,
            true,
            crate::domain::session::TASK_REQUEST_PROOF_VERSION,
        )?;
        let previous = state.latest_task_context_selection().unwrap().1.clone();
        Ok(TaskRefreshDecision {
            prior_event_id: id,
            prior: previous,
            decision,
        })
    }
    fn prepare(
        &mut self,
        common: CompactionContextCapture,
        compile: crate::domain::session::TaskCompileCapture,
        input: &EventId,
        expansion: Option<TaskSelectionDecision>,
        source_refresh: bool,
    ) -> Result<TaskContextPrepared, AgentLoopError> {
        let mut facts = self.0.borrow_mut();
        let mut candidates = facts
            .projection
            .state()
            .task_selection_candidates_with_capture(
                facts.project.project_id().as_str(),
                input,
                &common,
                &[],
                0,
                &[],
                expansion.as_ref(),
                source_refresh,
                crate::domain::session::TASK_CANDIDATE_POLICY_VERSION,
                &crate::app::context_read::history_navigation_excerpt,
            )?;
        facts
            .projection
            .state()
            .append_current_visual_candidates(input, &mut candidates)?;
        let capture = TaskContextCapture {
            schema_version: 2,
            candidate_policy_version: crate::domain::session::TASK_CANDIDATE_POLICY_VERSION,
            common,
            candidates: candidates.clone(),
            expansion: expansion.clone(),
            source_refresh,
            compile,
            research_candidates: Vec::new(),
            research_omitted: 0,
            memory: Vec::new(),
        };
        let store = LocalArtifactStore::for_project(&facts.project);
        let blob = store
            .publish_bytes(&serde_json::to_vec(&capture).unwrap())
            .unwrap()
            .blob_id;
        let capture_id = facts.append(SessionEvent::TaskContextCaptured {
            blob_id: blob.clone(),
        })?;
        let (source, state) = SessionSourceIdentity::from_events(
            facts.project.project_id().clone(),
            &facts.sink.0,
            &[],
        )?;
        let events = facts.sink.0.clone();
        facts.captured = Some((state, capture, events));
        Ok(TaskContextPrepared {
            source,
            capture_event_id: capture_id,
            capture_blob: blob,
            input_ref: input.clone(),
            candidates,
            samples_used: 0,
            expansion,
        })
    }
    fn record_attempt(&mut self, attempt: RequestAuditAttempt) -> Result<EventId, AgentLoopError> {
        self.0.borrow_mut().audit(attempt)
    }
    fn record_sample(
        &mut self,
        prepared: &TaskContextPrepared,
        scope: &ProviderSessionRef,
        output: &TaskSelectionSampleOutput,
    ) -> Result<EventId, AgentLoopError> {
        self.0
            .borrow_mut()
            .append(SessionEvent::TaskSelectionSampleRecorded {
                sample: TaskSelectionSample {
                    schema_version: 1,
                    source: prepared.source.clone(),
                    capture_event_id: prepared.capture_event_id.clone(),
                    capture_blob: prepared.capture_blob.clone(),
                    input_ref: prepared.input_ref.clone(),
                    audit_event_id: output.audit_event_id.clone(),
                    provider_session: scope.clone(),
                    decision: output.decision.clone(),
                },
            })
    }
    fn selection(
        &mut self,
        prepared: &TaskContextPrepared,
        sample_event_id: Option<EventId>,
        decision: TaskSelectionDecision,
    ) -> Result<TaskSelectionMaterial, AgentLoopError> {
        let facts = self.0.borrow();
        let state = facts.projection.state();
        let (selection, _) = state.prepare_task_selection(
            prepared.source.clone(),
            prepared.capture_event_id.clone(),
            prepared.capture_blob.clone(),
            prepared.input_ref.clone(),
            sample_event_id,
            &prepared.candidates,
            &[],
            &[],
            decision,
        )?;
        let history = state.task_resume_history(
            &selection.selected_history,
            &facts.sink.0,
            selection.request_proof_version,
        )?;
        Ok(TaskSelectionMaterial {
            constraints: state.constraint_contents(&selection)?,
            excerpts: state.excerpt_contents(&selection)?,
            progress: state.selected_task_progress(
                &selection,
                &crate::app::context_read::history_navigation_excerpt,
            )?,
            selection,
            history,
            research_units: Vec::new(),
            memory: Vec::new(),
        })
    }
    fn commit(&mut self, selection: TaskContextSelected) -> Result<(), AgentLoopError> {
        let mut facts = self.0.borrow_mut();
        let (state, capture, events) = facts.captured.as_ref().unwrap();
        let history = state.compaction_source_history()?;
        assert_eq!(
            crate::domain::canonical_json::canonical_array_hash(&history).unwrap(),
            capture.compile.history_hash,
            "fixture captured History hash"
        );
        let active = state.task_input_provenance(&selection.input_ref)?;
        let active_items = state.task_input_items(&selection.input_ref)?;
        assert_eq!(
            history.get(capture.compile.turn_history_start),
            active_items.first(),
            "fixture active input position"
        );
        assert_eq!(
            capture
                .compile
                .history_provenance
                .get(&capture.compile.turn_history_start),
            active.get(&0),
            "fixture active input provenance"
        );
        let protected = state.current_protected_operator_context()?;
        assert_eq!(
            protected.retained_provenance, capture.compile.history_provenance,
            "fixture retained input provenance"
        );
        let expected_protected = protected.all_without_active(&active, &history)?;
        assert_eq!(
            expected_protected,
            (
                capture.compile.protected_items.clone(),
                capture.compile.protected_provenance.clone()
            ),
            "fixture protected input capture"
        );
        let expected_prompt =
            PromptComposition::from_binding(state.agent_definition_binding().unwrap().clone())?
                .with_evolution_session(
                    "epoch",
                    format!(
                        ".echidna/{}",
                        crate::domain::agent_evolution::pending_relative("epoch-fixture", "epoch")
                    ),
                );
        assert_eq!(
            crate::domain::session::restore_fragments(&capture.compile.prompt)?,
            expected_prompt.fragments()?,
            "fixture prompt capture"
        );
        let exposure = expected_prompt.tool_exposure_for_catalog(&capture.compile.tool_catalog)?;
        let resolved = crate::domain::tools::ResolvedToolPlan::resolve(
            capture.compile.tool_catalog.clone(),
            &exposure,
            &crate::domain::tools::DeferredActivationSet::default(),
        )?;
        assert_eq!(
            resolved.advertised_specs(),
            capture.compile.tool_specs,
            "fixture advertised tools"
        );
        assert_eq!(
            resolved.resolved_specs(),
            capture.compile.resolved_tool_specs,
            "fixture resolved tools"
        );
        assert_eq!(
            resolved.fingerprint().as_str(),
            capture.compile.resolved_tool_plan_hash,
            "fixture tool fingerprint"
        );
        crate::app::agent_loop::compile_captured_task_context(
            state,
            capture,
            &selection,
            events,
            &[],
        )
        .map_err(|error| AgentLoopError::HistoryCheckpointCommit {
            message: format!("fixture source reconstruction failed: {error}"),
        })?;
        facts.append(SessionEvent::TaskContextSelected { selection })?;
        Ok(())
    }
}

struct CancelOnRefresh {
    inner: FixtureStage,
    cancellation: AgentCancellation,
    fail: bool,
    refreshes: usize,
    adoptions: usize,
}

impl TaskContextStage for CancelOnRefresh {
    fn selection_provider(
        &self,
    ) -> (
        Arc<dyn ModelProvider>,
        Option<crate::domain::profile::AgentModelProfile>,
    ) {
        self.inner.selection_provider()
    }
    fn record_completed_usage(
        &mut self,
        capture: &EventId,
        audit: &EventId,
        scope: &ProviderSessionRef,
        usage: crate::domain::model::ModelUsage,
    ) -> Result<(), AgentLoopError> {
        self.inner
            .record_completed_usage(capture, audit, scope, usage)
    }
    fn control_directory(&self) -> &'static str {
        self.inner.control_directory()
    }
    fn current_protected_context(
        &mut self,
    ) -> Result<crate::domain::session::ProtectedOperatorContext, AgentLoopError> {
        self.inner.current_protected_context()
    }
    fn prepare(
        &mut self,
        common: CompactionContextCapture,
        compile: crate::domain::session::TaskCompileCapture,
        input: &EventId,
        expansion: Option<TaskSelectionDecision>,
        source_refresh: bool,
    ) -> Result<TaskContextPrepared, AgentLoopError> {
        self.inner
            .prepare(common, compile, input, expansion, source_refresh)
    }
    fn record_attempt(&mut self, attempt: RequestAuditAttempt) -> Result<EventId, AgentLoopError> {
        self.inner.record_attempt(attempt)
    }
    fn record_sample(
        &mut self,
        prepared: &TaskContextPrepared,
        scope: &ProviderSessionRef,
        sample: &TaskSelectionSampleOutput,
    ) -> Result<EventId, AgentLoopError> {
        self.inner.record_sample(prepared, scope, sample)
    }
    fn selection(
        &mut self,
        prepared: &TaskContextPrepared,
        sample: Option<EventId>,
        decision: TaskSelectionDecision,
    ) -> Result<TaskSelectionMaterial, AgentLoopError> {
        self.inner.selection(prepared, sample, decision)
    }
    fn refresh_decision(
        &mut self,
        prepared: &TaskContextPrepared,
    ) -> Result<TaskRefreshDecision, AgentLoopError> {
        let decision = self.inner.refresh_decision(prepared)?;
        self.refreshes += 1;
        self.cancellation.cancel();
        if self.fail {
            return Err(AgentLoopError::HistoryCheckpointCommit {
                message: "injected refresh persistence failure".to_owned(),
            });
        }
        Ok(decision)
    }
    fn commit(&mut self, selection: TaskContextSelected) -> Result<(), AgentLoopError> {
        self.adoptions += 1;
        self.inner.commit(selection)
    }
}

struct ObserveSettledBatch {
    calls: usize,
}
impl crate::app::agent_loop::ToolBatchStage<ResponsesHttpAdapter, ToolRegistry>
    for ObserveSettledBatch
{
    fn needs_current_context(&self) -> bool {
        true
    }
    fn apply<'call, 'provider: 'call, 'tools: 'call>(
        &'call mut self,
        _: &'call mut AgentLoop<'provider, 'tools, ResponsesHttpAdapter, ToolRegistry>,
        _: &'call crate::app::agent_loop::ToolBatchContext<'_>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), AgentLoopError>> + 'call>>
    {
        self.calls += 1;
        Box::pin(async { Ok(()) })
    }
}

struct OpaqueProbe {
    adapter: ResponsesHttpAdapter,
    seen: Arc<Mutex<Vec<crate::domain::model::OpaqueContinuationReference>>>,
}

struct FixtureContextReader;
impl crate::ports::context_read::ContextReadPort for FixtureContextReader {
    fn read(
        &self,
        _: crate::ports::context_read::ContextReadRequest,
    ) -> Result<
        crate::ports::context_read::ContextReadResponse,
        crate::ports::context_read::ContextReadError,
    > {
        unreachable!("the epoch fixture issues no Tool calls")
    }
}
struct PreparedProbe<'a> {
    inner: Box<dyn PreparedModelRequest + 'a>,
    seen: Arc<Mutex<Vec<crate::domain::model::OpaqueContinuationReference>>>,
}
impl ModelProvider for OpaqueProbe {
    fn capabilities(&self) -> crate::domain::model::ModelCapabilities {
        self.adapter.capabilities()
    }
    fn stream(&self, request: ModelRequest) -> Result<ModelEventStream, ModelProviderError> {
        self.prepare(request)?.stream()
    }
    fn prepare(
        &self,
        request: ModelRequest,
    ) -> Result<Box<dyn PreparedModelRequest + '_>, ModelProviderError> {
        Ok(Box::new(PreparedProbe {
            inner: self.adapter.prepare(request)?,
            seen: self.seen.clone(),
        }))
    }
}
impl PreparedModelRequest for PreparedProbe<'_> {
    fn stream(&self) -> Result<ModelEventStream, ModelProviderError> {
        let seen = self.seen.clone();
        Ok(Box::pin(self.inner.stream()?.inspect(move |event| {
            if let Ok(ModelStreamEvent::Event(
                crate::domain::model::ModelEvent::OpaqueContinuation { reference },
            )) = event
            {
                seen.lock().unwrap().push(reference.clone());
            }
        })))
    }
    fn retrying(&self, retry: usize) {
        self.inner.retrying(retry);
    }
}

fn response(id: &str, mut items: Vec<Value>, text: &str) -> String {
    items.push(json!({"type":"message", "id":format!("message-{id}"), "role":"assistant", "status":"completed",
        "content":[{"type":"output_text", "text":text}]}));
    let mut frames = items.iter().enumerate().map(|(index, item)|
        json!({"type":"response.output_item.done", "output_index":index, "item":item})).collect::<Vec<_>>();
    frames.push(json!({"type":"response.completed", "response":{"id":id, "status":"completed", "output":items}}));
    frames
        .into_iter()
        .map(|frame| format!("data: {frame}\n\n"))
        .collect()
}

pub(crate) fn selection_for_wire(wire: &Value) -> String {
    fn find(value: &Value) -> Option<&str> {
        match value {
            Value::String(text) => text.strip_prefix("TASK_CONTEXT_CANDIDATES_V1\n"),
            Value::Array(items) => items.iter().find_map(find),
            Value::Object(fields) => fields.values().find_map(find),
            _ => None,
        }
    }
    assert!(
        wire.get("tools")
            .is_none_or(|tools| tools.as_array().unwrap().is_empty())
    );
    let candidates: Value = serde_json::from_str(find(&wire["input"]).unwrap()).unwrap();
    json!({"relation":"new", "focus":{"kind":"new_from_input","input_ref":candidates["input"]["ref"]},
        "related_task_refs":[],"required_context_refs":[],"unresolved_references":[]}).to_string()
}

enum EpochResponse {
    Selection { id: &'static str, opaque: bool },
    Response(String),
}

async fn epoch_server(
    steps: Vec<EpochResponse>,
) -> (
    String,
    tokio::sync::mpsc::Receiver<Value>,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let (sender, receiver) = tokio::sync::mpsc::channel(steps.len());
    let task = tokio::spawn(async move {
        for step in steps {
            let (mut socket, _) = listener.accept().await.unwrap();
            let wire = read_request(&mut socket).await;
            let body = match step {
                EpochResponse::Selection { id, opaque } => response(
                    id,
                    if opaque {
                        vec![encrypted("selector")]
                    } else {
                        Vec::new()
                    },
                    &selection_for_wire(&wire),
                ),
                EpochResponse::Response(body) => body,
            };
            sender.send(wire).await.unwrap();
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            socket.write_all(header.as_bytes()).await.unwrap();
            socket.write_all(body.as_bytes()).await.unwrap();
        }
    });
    (url, receiver, task)
}

#[tokio::test]
async fn subscription_opaque_state_isolated_across_real_task_selection_epochs() {
    let root = TemporaryDirectory::new("subscription-task-epoch");
    let project = Project::initialize(root.path()).unwrap();
    let definition = root.path().join("definition");
    write_builtin_definition(
        BuiltinAgentTemplate::TechnicalWriting,
        &definition,
        Some("epoch-fixture"),
    )
    .unwrap();
    let catalog = ToolRegistry::for_project(project.root());
    let binding = load_definition_binding(&definition, None, None, &catalog.catalog()).unwrap();
    let session = StreamId::new("session:epoch");
    let provider_session = ProviderSession::new(
        ProviderSessionId::new("primary:0"),
        ProviderSessionRole::Primary,
        0,
    );
    let first = SessionEvent::StartedWithAgent {
        primary_provider_session: provider_session.reference().clone(),
        binding: binding.clone(),
    }
    .into_envelope(
        EventId::new("epoch-event-1"),
        session.clone(),
        EventSequence::new(1),
        OccurredAt::new("2026-11-05T00:00:00Z"),
    )
    .unwrap();
    let projection = SessionProjection::replay([first.clone()]).unwrap();
    let audit_binding =
        RequestAuditBinding::new(project.project_id().clone(), session.clone()).unwrap();
    let facts = Rc::new(RefCell::new(Facts {
        project: project.clone(),
        projection,
        sink: Sink(vec![first]),
        binding: audit_binding.clone(),
        primary_audit: None,
        captured: None,
    }));
    let (url, mut requests, server_task) = epoch_server(vec![
        EpochResponse::Selection {
            id: "initial-selector",
            opaque: false,
        },
        EpochResponse::Response(response(
            "parent-response",
            vec![encrypted("parent")],
            "A completed",
        )),
        EpochResponse::Selection {
            id: "selector-response",
            opaque: true,
        },
        EpochResponse::Response(response("new-response", Vec::new(), "B completed")),
    ])
    .await;
    let mut mapping = capabilities();
    mapping.context_window = 272_000;
    let observed = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(OpaqueProbe {
        adapter: ResponsesHttpAdapter::new(
            ResponsesHttpConfig::new(
                url,
                "fixture-subscription",
                "codex_subscription:rsi-isolation",
                mapping,
            )
            .with_codex_subscription_profile(),
            Arc::new(FixtureCredential),
        ),
        seen: observed.clone(),
    });
    let tools = ToolRegistry::for_agent(
        project.root(),
        session,
        crate::cli_agent::memory_scope(&binding),
        binding.definition().research().is_some(),
    )
    .with_project(&project)
    .with_context_reader(Arc::new(FixtureContextReader));
    binding
        .definition()
        .validate_for_catalog(&tools.catalog())
        .unwrap();
    let pending = format!(
        ".echidna/{}",
        crate::domain::agent_evolution::pending_relative(
            binding.definition().id().as_str(),
            "epoch"
        )
    );
    let prompt = PromptComposition::from_binding(binding)
        .unwrap()
        .with_evolution_session("epoch", pending);
    let mut agent = AgentLoop::new(provider.as_ref(), &tools)
        .with_prompt_composition(prompt)
        .with_output_store(LocalArtifactStore::for_project(&project));
    let audit = RequestAuditRunContext::new(
        audit_binding,
        ResponsesHttpAdapter::request_adapter_version(),
    );
    for input in ["EPOCH-A-ONLY task", "EPOCH-B-ONLY unrelated task"] {
        let mut stage = FixtureStage(facts.clone(), provider.clone());
        let primary = facts.clone();
        let mut record = move |attempt| {
            let id = primary.borrow_mut().audit(attempt)?;
            primary.borrow_mut().primary_audit = Some(id);
            Ok(())
        };
        let durable = facts.clone();
        let mut checkpoint =
            move |checkpoint: ChatDurabilityCheckpoint<'_>| -> Result<(), AgentLoopError> {
                let mut facts = durable.borrow_mut();
                match checkpoint {
                    ChatDurabilityCheckpoint::History(items) => {
                        for item in items {
                            facts
                                .append(SessionEvent::HistoryItemRecorded { item: item.clone() })?;
                        }
                    }
                    ChatDurabilityCheckpoint::OperatorInputAccepted { items, provenance } => {
                        let boundary = facts.projection.state().accepted_operator_input(
                            items,
                            provenance,
                            TaskInputTime {
                                unix_seconds: 1_793_894_400,
                                nanoseconds: 0,
                                timezone: TimeZoneObservation::Unknown,
                            },
                        )?;
                        *provenance = boundary.input_provenance()?;
                        facts.append(SessionEvent::OperatorTurnBoundaryRecorded { boundary })?;
                    }
                    ChatDurabilityCheckpoint::ModelDeliveryCommitted {
                        step_id,
                        server_output_items,
                        ..
                    } => {
                        let store = LocalArtifactStore::for_project(&facts.project);
                        let output = store
                            .publish_bytes(&serde_json::to_vec(server_output_items).unwrap())
                            .unwrap()
                            .blob_id;
                        let delivery = facts.projection.state().new_model_delivery_with_output(
                            facts.primary_audit.as_ref().unwrap(),
                            None,
                            output,
                            step_id,
                        )?;
                        facts.append(SessionEvent::ModelDeliveryCommitted { delivery })?;
                    }
                    ChatDurabilityCheckpoint::OperatorTurnCompleted => facts.complete()?,
                    _ => panic!("fixture produces no Tool, usage, or recovery checkpoint"),
                }
                Ok(())
            };
        let mut follow_up =
            |_| -> ViewImageFollowUpFuture<'_> { Box::pin(async { unreachable!("no image") }) };
        let mut text = |_: &str| Ok(());
        let mut tool_start = |_: &crate::domain::tools::ToolExecutionPlan| {};
        let mut refresh = || {
            let snapshot = LocalProjectSnapshot::new(&project)
                .inspect(&tools.specs(), &[], "epoch-fixture")
                .unwrap();
            Ok(CurrentContextState::new(
                None,
                ProjectSnapshotContext::new(snapshot.snapshot).unwrap(),
                None,
            ))
        };
        agent
            .run_turn_audited_with_context_refresh_and_vision_stages(
                input,
                Vec::new(),
                AgentCancellation::new(),
                &audit,
                &mut record,
                CliRuntimeStages::new(
                    &mut follow_up,
                    &mut text,
                    &mut tool_start,
                    &mut checkpoint,
                    &mut refresh,
                    Arc::new(ModelRequestBudget::new(4)),
                )
                .with_task_context(&mut stage),
                None,
            )
            .await
            .unwrap_or_else(|error| {
                panic!(
                    "task epoch input {input:?}: {error:?}; events={:?}",
                    facts
                        .borrow()
                        .sink
                        .0
                        .iter()
                        .map(|event| event.event_type.as_str())
                        .collect::<Vec<_>>()
                )
            });
        let protected = facts
            .borrow()
            .projection
            .state()
            .current_protected_operator_context()
            .unwrap();
        agent.set_protected_context(protected);
    }
    let mut wires = Vec::new();
    while let Some(wire) = requests.recv().await {
        wires.push(wire);
    }
    server_task.await.unwrap();
    assert_eq!(wires.len(), 4);
    assert!(wires[0].to_string().contains("TASK_CONTEXT_CANDIDATES_V1"));
    assert!(wires[1].to_string().contains("EPOCH-A-ONLY"));
    assert!(wires[2].to_string().contains("TASK_CONTEXT_CANDIDATES_V1"));
    for wire in &wires[2..] {
        assert!(wire.get("previous_response_id").is_none());
        assert!(!wire.to_string().contains("ciphertext-parent"));
    }
    assert!(!wires[3].to_string().contains("ciphertext-selector"));
    assert!(!wires[3].to_string().contains("EPOCH-A-ONLY"));
    assert!(wires[3].to_string().contains("EPOCH-B-ONLY"));
    let observed = observed.lock().unwrap();
    assert_eq!(
        observed.len(),
        2,
        "the actual Adapter decoded both opaque responses"
    );
    assert_eq!(
        observed[0].scope(),
        &crate::domain::model::OpaqueContinuationScope::for_provider_session(
            provider_session.reference()
        )
    );
    assert_ne!(
        observed[0].scope(),
        observed[1].scope(),
        "parent and selector occupy distinct scopes"
    );
    let facts = facts.borrow();
    let selections = facts
        .sink
        .0
        .iter()
        .filter(|event| event.event_type.as_str() == "session.task_context_selected")
        .collect::<Vec<_>>();
    assert_eq!(selections.len(), 2);
    assert_ne!(
        selections[0].payload["provider_session"],
        selections[1].payload["provider_session"]
    );
}

#[tokio::test]
async fn settled_batch_refresh_cancellation_preserves_real_outcome_and_fatal_errors() {
    use crate::app::agent_loop::AgentTurnOutcome;

    for fail in [false, true] {
        let root = TemporaryDirectory::new("task-refresh-cancellation");
        let project = Project::initialize(root.path()).unwrap();
        let definition = root.path().join("definition");
        write_builtin_definition(
            BuiltinAgentTemplate::TechnicalWriting,
            &definition,
            Some("epoch-fixture"),
        )
        .unwrap();
        let manifest_path = definition.join("agent.toml");
        let manifest = std::fs::read_to_string(&manifest_path).unwrap();
        std::fs::write(
            &manifest_path,
            format!("{manifest}\nrun_command = \"direct\"\n"),
        )
        .unwrap();
        let catalog = ToolRegistry::for_project(project.root());
        let binding = load_definition_binding(&definition, None, None, &catalog.catalog()).unwrap();
        let session = StreamId::new("session:epoch");
        let provider_session = ProviderSession::new(
            ProviderSessionId::new("primary:0"),
            ProviderSessionRole::Primary,
            0,
        );
        let first = SessionEvent::StartedWithAgent {
            primary_provider_session: provider_session.reference().clone(),
            binding: binding.clone(),
        }
        .into_envelope(
            EventId::new("epoch-event-1"),
            session.clone(),
            EventSequence::new(1),
            OccurredAt::new("2026-11-05T00:00:00Z"),
        )
        .unwrap();
        let audit_binding =
            RequestAuditBinding::new(project.project_id().clone(), session.clone()).unwrap();
        let facts = Rc::new(RefCell::new(Facts {
            project: project.clone(),
            projection: SessionProjection::replay([first.clone()]).unwrap(),
            sink: Sink(vec![first]),
            binding: audit_binding.clone(),
            primary_audit: None,
            captured: None,
        }));
        let body = response(
            "settled-batch",
            vec![
                json!({"type":"function_call", "id":"item-once", "call_id":"effect-once",
            "name":"run_command", "arguments":json!({"argv":["/bin/sh","-c","printf 'once\\n' >> effect-count.txt"]}).to_string()}),
            ],
            "Working.",
        );
        let (url, mut requests, server_task) = epoch_server(vec![
            EpochResponse::Selection {
                id: "refresh-initial-selector",
                opaque: false,
            },
            EpochResponse::Response(body),
        ])
        .await;
        let mut mapping = capabilities();
        mapping.context_window = 272_000;
        let provider = Arc::new(ResponsesHttpAdapter::new(
            ResponsesHttpConfig::new(
                url,
                "fixture-subscription",
                "codex_subscription:rsi-isolation",
                mapping,
            )
            .with_codex_subscription_profile(),
            Arc::new(FixtureCredential),
        ));
        let tools = ToolRegistry::for_agent(
            project.root(),
            session,
            crate::cli_agent::memory_scope(&binding),
            false,
        )
        .with_project(&project)
        .with_context_reader(Arc::new(FixtureContextReader));
        let pending = format!(
            ".echidna/{}",
            crate::domain::agent_evolution::pending_relative(
                binding.definition().id().as_str(),
                "epoch"
            )
        );
        let prompt = PromptComposition::from_binding(binding)
            .unwrap()
            .with_evolution_session("epoch", pending);
        let mut agent = AgentLoop::new(provider.as_ref(), &tools)
            .with_prompt_composition(prompt)
            .with_output_store(LocalArtifactStore::for_project(&project));
        let audit = RequestAuditRunContext::new(
            audit_binding,
            ResponsesHttpAdapter::request_adapter_version(),
        );
        let cancellation = AgentCancellation::new();
        let mut task = CancelOnRefresh {
            inner: FixtureStage(facts.clone(), provider.clone()),
            cancellation: cancellation.clone(),
            fail,
            refreshes: 0,
            adoptions: 0,
        };
        let mut batch = ObserveSettledBatch { calls: 0 };
        let primary = facts.clone();
        let mut record = move |attempt| {
            let id = primary.borrow_mut().audit(attempt)?;
            primary.borrow_mut().primary_audit = Some(id);
            Ok(())
        };
        let durable = facts.clone();
        let mut checkpoint =
            move |checkpoint: ChatDurabilityCheckpoint<'_>| -> Result<(), AgentLoopError> {
                let mut facts = durable.borrow_mut();
                match checkpoint {
                    ChatDurabilityCheckpoint::History(items) => {
                        for item in items {
                            facts
                                .append(SessionEvent::HistoryItemRecorded { item: item.clone() })?;
                        }
                    }
                    ChatDurabilityCheckpoint::OperatorInputAccepted { items, provenance } => {
                        let boundary = facts.projection.state().accepted_operator_input(
                            items,
                            provenance,
                            TaskInputTime {
                                unix_seconds: 1_793_894_400,
                                nanoseconds: 0,
                                timezone: TimeZoneObservation::Unknown,
                            },
                        )?;
                        *provenance = boundary.input_provenance()?;
                        facts.append(SessionEvent::OperatorTurnBoundaryRecorded { boundary })?;
                    }
                    ChatDurabilityCheckpoint::ModelDeliveryCommitted {
                        step_id,
                        server_output_items,
                        ..
                    } => {
                        let store = LocalArtifactStore::for_project(&facts.project);
                        let blob = store
                            .publish_bytes(&serde_json::to_vec(server_output_items).unwrap())
                            .unwrap()
                            .blob_id;
                        let delivery = facts.projection.state().new_model_delivery_with_output(
                            facts.primary_audit.as_ref().unwrap(),
                            None,
                            blob,
                            step_id,
                        )?;
                        facts.append(SessionEvent::ModelDeliveryCommitted { delivery })?;
                    }
                    ChatDurabilityCheckpoint::ExecutionReceipt(outcome) => {
                        facts.append(SessionEvent::ExecutionOutcomeRecorded {
                            receipt: outcome.receipt.clone(),
                            result: outcome.result.clone(),
                        })?;
                    }
                    _ => panic!("unexpected checkpoint in settled-batch cancellation fixture"),
                }
                Ok(())
            };
        let mut follow_up =
            |_| -> ViewImageFollowUpFuture<'_> { Box::pin(async { unreachable!("no image") }) };
        let mut text = |_: &str| Ok(());
        let mut tool_start = |_: &crate::domain::tools::ToolExecutionPlan| {};
        let mut refresh = || {
            let snapshot = LocalProjectSnapshot::new(&project)
                .inspect(&tools.specs(), &[], "epoch-fixture")
                .unwrap();
            Ok(CurrentContextState::new(
                None,
                ProjectSnapshotContext::new(snapshot.snapshot).unwrap(),
                None,
            ))
        };
        let result = agent
            .run_turn_audited_with_context_refresh_and_vision_stages(
                "Execute the synthetic effect once.",
                Vec::new(),
                cancellation.clone(),
                &audit,
                &mut record,
                CliRuntimeStages::new(
                    &mut follow_up,
                    &mut text,
                    &mut tool_start,
                    &mut checkpoint,
                    &mut refresh,
                    Arc::new(ModelRequestBudget::new(4)),
                )
                .with_task_context(&mut task),
                Some(&mut batch),
            )
            .await;
        if fail {
            assert!(
                matches!(result, Err(AgentLoopError::HistoryCheckpointCommit { message }) if message == "injected refresh persistence failure")
            );
        } else {
            let AgentTurnOutcome::Cancelled {
                model_requests,
                executions,
            } = result.unwrap()
            else {
                panic!("refresh cancellation must settle the Turn")
            };
            assert_eq!(model_requests, 2);
            assert_eq!(executions.len(), 1);
            assert_eq!(executions[0].result.call_id.as_str(), "effect-once");
            assert_eq!(
                executions[0].result.status,
                crate::domain::tools::ToolResultStatus::Succeeded
            );
        }
        assert!(cancellation.is_cancelled());
        assert_eq!(task.refreshes, 1);
        assert_eq!(
            task.adoptions, 1,
            "the cancelled refresh must not adopt a new selection"
        );
        assert_eq!(batch.calls, 1);
        assert_eq!(
            std::fs::read(root.path().join("effect-count.txt")).unwrap(),
            b"once\n"
        );
        {
            let facts = facts.borrow();
            assert_eq!(
                facts
                    .sink
                    .0
                    .iter()
                    .filter(|event| event.event_type.as_str() == "tool.execution_outcome_recorded")
                    .count(),
                1
            );
            assert_eq!(
                facts
                    .sink
                    .0
                    .iter()
                    .filter(|event| event.event_type.as_str() == "session.task_context_selected")
                    .count(),
                1
            );
            assert_eq!(
                facts
                    .sink
                    .0
                    .iter()
                    .filter(|event| event.event_type.as_str() == "session.request_audit_recorded")
                    .count(),
                2
            );
            assert_eq!(facts.projection.state().history().items().iter().filter(|item| matches!(item, crate::domain::model::ContentItem::ToolResult { result } if result.call_id.as_str() == "effect-once")).count(), 1);
        }
        let selector = requests.recv().await.unwrap();
        assert!(selector.to_string().contains("TASK_CONTEXT_CANDIDATES_V1"));
        let main = requests.recv().await.unwrap();
        assert!(!main.to_string().contains("TASK_CONTEXT_CANDIDATES_V1"));
        assert!(
            requests.recv().await.is_none(),
            "refresh must not sample another model request"
        );
        server_task.await.unwrap();
    }
}
