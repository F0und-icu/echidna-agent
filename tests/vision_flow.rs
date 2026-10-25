//! Offline end-to-end contracts for the one scoped Vision path.
//!
//! These tests deliberately keep the Provider, Blob Store and Event Sink
//! local. They verify the ordering and identity boundaries that must hold
//! before either configured Provider is allowed to make a real request.

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::convert::Infallible;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use echidna::adapters::tools::ToolRegistry;
use echidna::app::agent_loop::{AgentLoop, AgentLoopError, ViewImageFollowUpFuture};
use echidna::app::cancellation::AgentCancellation;
use echidna::app::request_audit::{RequestAuditService, RequestAuditServiceError};
use echidna::app::session::{AppendAfterDurableError, SessionProjection};
use echidna::app::vision::{
    VisionAnalysisInput, VisionDirectRequest, VisionEventStamp, VisionReplayError,
    VisionReplayService, VisionSecondaryRequest, VisionSessionError, VisionSessionService,
};
use echidna::domain::artifacts::{ArtifactId, BlobId};
use echidna::domain::context::{
    AuthorityLevel, ContextOrigin, ModelIdentity, ModelProfile, ProviderSession, ProviderSessionId,
    ProviderSessionRef, ProviderSessionRole, SemanticClass,
};
use echidna::domain::domain_profile::DomainProfile;
use echidna::domain::events::{EventEnvelope, EventId, EventSequence, OccurredAt, StreamId};
use echidna::domain::model::{
    CapabilityMappingVersion, ContentItem, ModelCapabilities, ModelEvent, ModelUsage,
};
use echidna::domain::profile::{VisionMode, VisionRoute, VisionRouteReason};
use echidna::domain::project::ProjectId;
use echidna::domain::request_audit::{
    REQUEST_AUDIT_MANIFEST_SCHEMA_VERSION, RequestAdapterVersion, RequestAuditBinding,
    RequestAuditError, RequestAuditManifest, RequestAuditReconstructionError,
    RequestAuditRunContext,
};
use echidna::domain::session::{SessionError, SessionEvent, SessionReducer};
use echidna::domain::tools::{
    ExecutionId, ToolCall, ToolExecutionOutcome, ToolExecutionPlan, ToolSpec,
};
use echidna::domain::vision::{
    VISION_FACT_SCHEMA_VERSION, ViewImageSelection, VisionImageGeometry, VisionObservationId,
    VisionRouteRecord,
};
use echidna::ports::EventSink;
use echidna::ports::artifact::{ArtifactStore, StoredBlob};
use echidna::ports::model::{
    ModelEventStream, ModelProvider, ModelProviderError, ModelRequest, ModelResponseCompletion,
    ModelStreamEvent,
};
use echidna::ports::tool::{ToolExecutionFuture, ToolExecutor, ToolExecutorError};
use futures_util::stream;
use sha2::{Digest, Sha256};

#[path = "vision_flow_cases/geometry.rs"]
mod geometry;
mod support;

fn capabilities(image_input: bool, incremental_input: bool) -> ModelCapabilities {
    ModelCapabilities {
        text_input: true,
        image_input,
        streaming: true,
        tool_call: true,
        structured_output: true,
        incremental_input,
        usage_reporting: false,
        max_output_tokens: None,
        reasoning_effort: None,
        context_window: 128_000,
        capability_mapping_version: CapabilityMappingVersion::new(1),
    }
}

fn capability_hash(capabilities: &ModelCapabilities) -> String {
    capabilities.semantic_hash()
}

fn occurred_at(sequence: u64) -> OccurredAt {
    OccurredAt::new(format!("2026-08-26T00:00:{sequence:02}Z"))
}

#[derive(Default)]
struct EventLog {
    events: Vec<EventEnvelope<serde_json::Value>>,
    route_committed: Option<Arc<AtomicBool>>,
}

impl EventLog {
    fn with_route_flag(route_committed: Arc<AtomicBool>) -> Self {
        Self {
            events: Vec::new(),
            route_committed: Some(route_committed),
        }
    }
}

impl EventSink<serde_json::Value> for EventLog {
    type Error = Infallible;

    fn append_event(
        &mut self,
        event: &EventEnvelope<serde_json::Value>,
    ) -> Result<(), Self::Error> {
        self.events.push(event.clone());
        if event.event_type.as_str() == "vision.route_decided"
            && let Some(route_committed) = &self.route_committed
        {
            route_committed.store(true, Ordering::SeqCst);
        }
        Ok(())
    }
}

#[derive(Default)]
struct MemoryBlobs {
    values: Mutex<BTreeMap<BlobId, Vec<u8>>>,
    writes: AtomicUsize,
}

#[derive(Debug)]
struct MemoryBlobError;

impl MemoryBlobs {
    fn writes(&self) -> usize {
        self.writes.load(Ordering::Relaxed)
    }

    fn remove(&self, blob_id: &BlobId) -> Option<Vec<u8>> {
        self.values.lock().unwrap().remove(blob_id)
    }

    fn insert_unchecked(&self, blob_id: BlobId, bytes: Vec<u8>) {
        self.values.lock().unwrap().insert(blob_id, bytes);
    }
}

impl ArtifactStore for MemoryBlobs {
    type Error = MemoryBlobError;

    fn publish_file(&self, _source: &std::path::Path) -> Result<StoredBlob, Self::Error> {
        unreachable!("the Vision flow publishes immutable request and response bytes")
    }

    fn publish_bytes(&self, bytes: &[u8]) -> Result<StoredBlob, Self::Error> {
        let blob_id = BlobId::new(format!("{:x}", Sha256::digest(bytes))).unwrap();
        let mut values = self.values.lock().unwrap();
        let already_present = values.contains_key(&blob_id);
        values
            .entry(blob_id.clone())
            .or_insert_with(|| bytes.to_vec());
        self.writes.fetch_add(1, Ordering::Relaxed);
        Ok(StoredBlob {
            blob_id,
            size_bytes: u64::try_from(bytes.len()).unwrap(),
            already_present,
        })
    }

    fn read_blob(&self, blob_id: &BlobId) -> Result<Vec<u8>, Self::Error> {
        self.values
            .lock()
            .unwrap()
            .get(blob_id)
            .cloned()
            .ok_or(MemoryBlobError)
    }
}

struct ScriptedProvider {
    capabilities: ModelCapabilities,
    responses: Mutex<VecDeque<Vec<Result<ModelStreamEvent, ModelProviderError>>>>,
    requests: Mutex<Vec<ModelRequest>>,
    required_before_stream: Option<Arc<AtomicBool>>,
}

impl ScriptedProvider {
    fn new(
        capabilities: ModelCapabilities,
        responses: impl IntoIterator<Item = Vec<Result<ModelStreamEvent, ModelProviderError>>>,
    ) -> Self {
        Self {
            capabilities,
            responses: Mutex::new(responses.into_iter().collect()),
            requests: Mutex::new(Vec::new()),
            required_before_stream: None,
        }
    }

    fn requiring_before_stream(mut self, condition: Arc<AtomicBool>) -> Self {
        self.required_before_stream = Some(condition);
        self
    }

    fn requests(&self) -> Vec<ModelRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl ModelProvider for ScriptedProvider {
    fn capabilities(&self) -> ModelCapabilities {
        self.capabilities.clone()
    }

    fn stream(&self, request: ModelRequest) -> Result<ModelEventStream, ModelProviderError> {
        request.validate_capabilities(&self.capabilities)?;
        if let Some(condition) = &self.required_before_stream {
            assert!(
                condition.load(Ordering::SeqCst),
                "the route fact must be durable before the Vision request"
            );
        }
        self.requests.lock().unwrap().push(request);
        let response = self.responses.lock().unwrap().pop_front().ok_or_else(|| {
            ModelProviderError::ProviderFailure {
                code: Some("fixture_exhausted".to_owned()),
                message: "no fixture response remains".to_owned(),
            }
        })?;
        Ok(Box::pin(stream::iter(response)))
    }
}

fn completed(
    text: impl Into<String>,
    response_id: &str,
) -> Vec<Result<ModelStreamEvent, ModelProviderError>> {
    vec![
        Ok(ModelStreamEvent::Event(ModelEvent::TextDelta {
            text: text.into(),
        })),
        Ok(ModelStreamEvent::Completed(ModelResponseCompletion::new(
            echidna::ports::model::ProviderResponseId::new(response_id),
        ))),
    ]
}

fn failed_provider() -> Vec<Result<ModelStreamEvent, ModelProviderError>> {
    vec![Err(ModelProviderError::ProviderFailure {
        code: Some("fixture_failure".to_owned()),
        message: "Vision fixture rejected the request".to_owned(),
    })]
}

fn binding() -> RequestAuditBinding {
    RequestAuditBinding::new(
        ProjectId::new("vision-flow-project"),
        StreamId::new("session:vision-flow"),
    )
    .unwrap()
}

fn audit_context(binding: RequestAuditBinding) -> RequestAuditRunContext {
    RequestAuditRunContext::new(
        binding,
        RequestAdapterVersion::new("fixture-adapter-v1").unwrap(),
    )
}

fn projection(binding: &RequestAuditBinding) -> SessionProjection {
    SessionProjection::replay([started_event(binding)]).unwrap()
}

fn started_event(binding: &RequestAuditBinding) -> EventEnvelope<serde_json::Value> {
    SessionEvent::Started {
        primary_provider_session: ProviderSessionRef {
            id: ProviderSessionId::new("primary:vision-flow"),
            role: ProviderSessionRole::Primary,
            generation: 1,
        },
    }
    .into_envelope(
        EventId::new("session-started"),
        binding.session_stream_id().clone(),
        EventSequence::new(1),
        occurred_at(1),
    )
    .unwrap()
}

fn route(
    event_id: &str,
    source_artifact_id: &str,
    primary: &ModelCapabilities,
    vision: &ModelCapabilities,
    mode: VisionMode,
    route: VisionRoute,
    reason: VisionRouteReason,
) -> VisionRouteRecord {
    VisionRouteRecord {
        schema_version: VISION_FACT_SCHEMA_VERSION,
        route_event_id: EventId::new(event_id),
        mode,
        route,
        reason: Some(reason),
        source_artifact_ids: vec![ArtifactId::new(source_artifact_id)],
        primary_profile: ModelProfile::new("primary", 1),
        vision_profile: Some(ModelProfile::new("vision", 1)),
        primary_capability_hash: capability_hash(primary),
        vision_capability_hash: Some(capability_hash(vision)),
    }
}

fn vision_input(vision_capabilities: &ModelCapabilities) -> VisionAnalysisInput {
    VisionAnalysisInput {
        observation_id: VisionObservationId::new("observation:vision-flow"),
        route_event_id: EventId::new("vision-route"),
        source_artifact_id: ArtifactId::new("artifact:original"),
        delivery_artifact_id: ArtifactId::new("artifact:budgeted"),
        derived_artifact_ids: vec![ArtifactId::new("artifact:budgeted")],
        image_geometry: VisionImageGeometry {
            delivery_artifact_id: ArtifactId::new("artifact:budgeted"),
            source_width: 1200,
            source_height: 800,
            delivery_width: 600,
            delivery_height: 400,
        },
        task: "Inspect this screenshot for a potentially exposed debug control.".to_owned(),
        model: ModelIdentity::new("fixture-vision", "vision-v1"),
        provider_profile: ModelProfile::new("vision", 1),
        capability_hash: capability_hash(vision_capabilities),
        provider_session: ProviderSessionRef {
            id: ProviderSessionId::new("vision:vision-flow"),
            role: ProviderSessionRole::Vision,
            generation: 1,
        },
    }
}

#[tokio::test]
async fn secondary_path_commits_route_audit_observation_then_primary_without_sending_image_twice() {
    let primary_capabilities = capabilities(false, false);
    let vision_capabilities = capabilities(true, false);
    let route_committed = Arc::new(AtomicBool::new(false));
    let usage = ModelUsage {
        input_tokens: 100,
        output_tokens: 50,
        cached_input_tokens: Some(75),
        reasoning_output_tokens: Some(20),
    };
    let mut vision_response = completed(
        r#"{"schema_version":2,"summary":"A debug settings control is visible.","extracted_text":"Debug","regions":[],"task_observations":["debug control"],"uncertainty":["requires runtime validation"]}"#,
        "vision-response",
    );
    vision_response.insert(1, Ok(ModelStreamEvent::Event(ModelEvent::Usage { usage })));
    let vision_provider = ScriptedProvider::new(vision_capabilities.clone(), [vision_response])
        .requiring_before_stream(route_committed.clone());
    let primary_provider = ScriptedProvider::new(
        primary_capabilities.clone(),
        [completed(
            "Primary received the unverified observation.",
            "primary-response",
        )],
    );
    let binding = binding();
    let context = audit_context(binding.clone());
    let mut projection = projection(&binding);
    let mut sink = EventLog::with_route_flag(route_committed.clone());
    sink.events.push(started_event(&binding));
    let blobs = MemoryBlobs::default();
    let audit_service = RequestAuditService::new(&blobs, binding.clone());
    let vision_session = VisionSessionService::new(&blobs, binding.clone());
    let output = vision_session
        .analyze_image(
            &vision_provider,
            VisionSecondaryRequest {
                route: route(
                    "vision-route",
                    "artifact:original",
                    &primary_capabilities,
                    &vision_capabilities,
                    VisionMode::SecondaryTool,
                    VisionRoute::SecondaryTool,
                    VisionRouteReason::ForcedSecondaryTool,
                ),
                route_occurred_at: occurred_at(2),
                analysis: vision_input(&vision_capabilities),
            },
            &context,
            AgentCancellation::new(),
            &mut projection,
            &mut sink,
            || VisionEventStamp::new(EventId::new("vision-audit"), occurred_at(3)),
            || VisionEventStamp::new(EventId::new("vision-observation"), occurred_at(4)),
        )
        .await
        .unwrap();

    let root = support::TemporaryDirectory::new("vision-flow-secondary");
    let tools = ToolRegistry::for_project(root.path());
    let mut agent = AgentLoop::with_primary_context(
        &primary_provider,
        &tools,
        ModelIdentity::new("fixture-primary", "primary-v1"),
        ModelProfile::new("primary", 1),
        ProviderSession::new(
            ProviderSessionId::new("primary:vision-flow"),
            ProviderSessionRole::Primary,
            1,
        ),
    )
    .unwrap();
    {
        let mut record_primary = |attempt| {
            audit_service
                .record(
                    attempt,
                    EventId::new("primary-audit"),
                    occurred_at(5),
                    &mut projection,
                    &mut sink,
                )
                .map(|_| ())
                .map_err(RequestAuditServiceError::into_agent_loop_error)
        };
        agent
            .run_turn_audited_with_initial_image(
                "Assess the screenshot observation.",
                output.primary_context,
                AgentCancellation::new(),
                &context,
                &mut record_primary,
            )
            .await
            .unwrap();
    }

    let vision_requests = vision_provider.requests();
    assert_eq!(vision_requests.len(), 1);
    assert!(!vision_requests[0].is_incremental());
    assert!(vision_requests[0].input().tool_specs.is_empty());
    assert!(
        vision_requests[0]
            .input()
            .instructions
            .contains("Every regions item must contain exactly label")
    );
    assert!(
        vision_requests[0]
            .input()
            .instructions
            .contains("do not add description or any other field")
    );
    assert!(matches!(
        vision_requests[0].input().content_items.as_slice(),
        [ContentItem::Text { .. }, ContentItem::ImageArtifact { artifact_id, .. }, ContentItem::Text { .. }]
            if artifact_id.as_str() == "artifact:budgeted"
    ));

    let primary_requests = primary_provider.requests();
    assert_eq!(primary_requests.len(), 1);
    assert!(!primary_requests[0].is_incremental());
    assert_eq!(
        primary_requests[0]
            .input()
            .content_items
            .iter()
            .filter(|item| matches!(item, ContentItem::ImageArtifact { .. }))
            .count(),
        0
    );
    assert!(primary_requests[0]
        .input()
        .content_items
        .iter()
        .any(|item| matches!(item, ContentItem::Text { text, .. } if text.contains("unverified_model_observation"))));
    assert_eq!(projection.state().vision_routes().len(), 1);
    assert_eq!(projection.state().vision_observations().len(), 1);
    assert!(projection.state().vision_failures().is_empty());
    assert_eq!(projection.state().request_audits().len(), 2);
    let vision_audit = projection
        .state()
        .request_audits()
        .iter()
        .find(|audit| audit.audit_event_id().as_str() == "vision-audit")
        .unwrap();
    assert_eq!(
        vision_audit.schema_version(),
        REQUEST_AUDIT_MANIFEST_SCHEMA_VERSION
    );
    assert!(vision_audit.resolved_tool_plan_fingerprint().is_some());
    let vision_map = vision_audit.context_delivery_map().unwrap();
    vision_map
        .validate_against(vision_requests[0].input())
        .unwrap();
    assert_eq!(vision_map.instruction_spans().len(), 1);
    assert_eq!(vision_map.content_items().len(), 3);
    assert_eq!(
        vision_map.content_items()[2].provenance(),
        vision_map.content_items()[1].provenance()
    );
    assert_eq!(
        vision_map.content_items()[0].provenance().origin(),
        ContextOrigin::OperatorInput
    );
    assert_eq!(
        vision_map.content_items()[1].provenance().origin(),
        ContextOrigin::ArtifactProjection
    );
    assert_eq!(
        vision_map.content_items()[1].provenance().authority_level(),
        AuthorityLevel::FactReference
    );
    assert_eq!(
        vision_map.content_items()[1]
            .provenance()
            .references()
            .artifact_refs(),
        [
            ArtifactId::new("artifact:original"),
            ArtifactId::new("artifact:budgeted")
        ]
    );
    assert_eq!(
        vision_map.content_items()[1]
            .provenance()
            .references()
            .source_event_refs(),
        [EventId::new("vision-route")]
    );

    let mut forged_transform = serde_json::to_value(vision_audit).unwrap();
    forged_transform["context_delivery_map"]["content_items"][1]["transform"] = serde_json::json!({
        "kind": "artifact_reference",
        "artifact_id": "artifact:budgeted"
    });
    let forged_transform: RequestAuditManifest = serde_json::from_value(forged_transform).unwrap();
    forged_transform
        .validate()
        .expect("exact delivered bytes are required to validate an ArtifactReference transform");
    assert!(matches!(
        audit_service.reconstruct(&forged_transform),
        Err(RequestAuditReconstructionError::Audit(
            RequestAuditError::ContextDeliveryMap { .. }
        ))
    ));
    let primary_audit = projection
        .state()
        .request_audits()
        .iter()
        .find(|audit| audit.audit_event_id().as_str() == "primary-audit")
        .unwrap();
    let primary_map = primary_audit.context_delivery_map().unwrap();
    primary_map
        .validate_against(primary_requests[0].input())
        .unwrap();
    let observation_position = primary_requests[0]
        .input()
        .content_items
        .iter()
        .position(|item| {
            matches!(item, ContentItem::Text { text, .. } if text.contains("unverified_model_observation"))
        })
        .unwrap();
    let observation_provenance = primary_map.content_items()[observation_position].provenance();
    assert_eq!(observation_provenance.origin(), ContextOrigin::ModelOutput);
    assert_eq!(
        observation_provenance.semantic_class(),
        SemanticClass::ModelObservation
    );
    assert_eq!(
        observation_provenance.authority_level(),
        AuthorityLevel::ModelObservation
    );
    assert!(
        observation_provenance
            .references()
            .artifact_refs()
            .iter()
            .any(|artifact| artifact.as_str() == "artifact:original")
    );
    assert_eq!(
        observation_provenance.references().source_event_refs(),
        [
            EventId::new("vision-route"),
            EventId::new("vision-audit"),
            EventId::new("vision-observation"),
        ]
    );
    assert!(blobs.writes() > 0);

    let event_count = sink.events.len();
    let blob_writes = blobs.writes();
    let replayed = SessionProjection::replay(sink.events.clone()).unwrap();
    let replayed_observation = VisionReplayService::new(&blobs)
        .replay_secondary(
            replayed.state(),
            &EventId::new("vision-route"),
            &ArtifactId::new("artifact:original"),
        )
        .unwrap();
    assert_eq!(
        replayed_observation,
        projection.state().vision_observations()[0]
    );
    assert_eq!(vision_provider.requests().len(), 1);
    assert_eq!(primary_provider.requests().len(), 1);
    assert_eq!(sink.events.len(), event_count);
    assert_eq!(blobs.writes(), blob_writes);

    let usage_event = SessionEvent::ModelUsageRecorded {
        request_audit_event_id: output.observation.request_audit_event_id.clone(),
        provider_session: output.observation.provider_session.clone(),
        usage,
    };
    let mut before_observation = SessionReducer.reduce(sink.events[..3].to_vec()).unwrap();
    let rejected = usage_event
        .clone()
        .into_envelope(
            EventId::new("premature-vision-usage"),
            binding.session_stream_id().clone(),
            EventSequence::new(4),
            occurred_at(4),
        )
        .unwrap();
    assert!(matches!(
        SessionReducer.apply(&mut before_observation, rejected),
        Err(SessionError::InvalidModelUsageReference { .. })
    ));
    let accepted = usage_event
        .into_envelope(
            EventId::new("vision-usage"),
            binding.session_stream_id().clone(),
            EventSequence::new(sink.events.last().unwrap().sequence.get() + 1),
            occurred_at(6),
        )
        .unwrap();
    projection.append(&mut sink, accepted).unwrap();
    assert_eq!(
        SessionReducer.reduce(sink.events.clone()).unwrap(),
        *projection.state()
    );
}

#[tokio::test]
async fn direct_path_audits_one_primary_image_and_never_calls_a_secondary_provider() {
    let primary_capabilities = capabilities(true, true);
    let unused_vision = ScriptedProvider::new(
        capabilities(true, false),
        [completed("must not be called", "unused-vision")],
    );
    let primary_provider = ScriptedProvider::new(
        primary_capabilities.clone(),
        [
            completed("first primary response", "primary-one"),
            completed("second primary response", "primary-two"),
        ],
    );
    let binding = binding();
    let context = audit_context(binding.clone());
    let mut projection = projection(&binding);
    let mut sink = EventLog::default();
    let blobs = MemoryBlobs::default();
    let audit_service = RequestAuditService::new(&blobs, binding.clone());
    let initial_image = VisionSessionService::new(&blobs, binding.clone())
        .prepare_direct(
            VisionDirectRequest {
                route: route(
                    "vision-route",
                    "artifact:original",
                    &primary_capabilities,
                    &capabilities(true, false),
                    VisionMode::PrimaryDirect,
                    VisionRoute::PrimaryDirect,
                    VisionRouteReason::ForcedPrimaryDirect,
                ),
                route_occurred_at: occurred_at(2),
                source_artifact_id: ArtifactId::new("artifact:original"),
                delivery_artifact_id: ArtifactId::new("artifact:budgeted"),
            },
            &mut projection,
            &mut sink,
        )
        .unwrap();

    let root = support::TemporaryDirectory::new("vision-flow-direct");
    let tools = ToolRegistry::for_project(root.path());
    let mut agent = AgentLoop::with_primary_context(
        &primary_provider,
        &tools,
        ModelIdentity::new("fixture-primary", "primary-v1"),
        ModelProfile::new("primary", 1),
        ProviderSession::new(
            ProviderSessionId::new("primary:vision-flow"),
            ProviderSessionRole::Primary,
            1,
        ),
    )
    .unwrap();
    let mut audit_number = 0;
    let mut record_primary = |attempt| {
        audit_number += 1;
        audit_service
            .record(
                attempt,
                EventId::new(format!("primary-audit-{audit_number}")),
                occurred_at(2 + audit_number),
                &mut projection,
                &mut sink,
            )
            .map(|_| ())
            .map_err(RequestAuditServiceError::into_agent_loop_error)
    };
    agent
        .run_turn_audited_with_initial_image(
            "Inspect the initial screenshot.",
            initial_image,
            AgentCancellation::new(),
            &context,
            &mut record_primary,
        )
        .await
        .unwrap();
    agent
        .run_turn_audited(
            "Continue with the same evidence.",
            AgentCancellation::new(),
            &context,
            &mut record_primary,
        )
        .await
        .unwrap();

    let requests = primary_provider.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0]
            .input()
            .content_items
            .iter()
            .filter(|item| matches!(item, ContentItem::ImageArtifact { artifact_id, .. } if artifact_id.as_str() == "artifact:budgeted"))
            .count(),
        1
    );
    assert!(requests[1].is_incremental());
    assert!(unused_vision.requests().is_empty());
    assert_eq!(projection.state().vision_routes().len(), 1);
    assert!(projection.state().vision_observations().is_empty());
    assert_eq!(projection.state().request_audits().len(), 2);
    let primary_audit = projection
        .state()
        .request_audits()
        .iter()
        .find(|audit| audit.audit_event_id().as_str() == "primary-audit-1")
        .unwrap();
    let image_position = requests[0]
        .input()
        .content_items
        .iter()
        .position(|item| {
            matches!(
                item,
                ContentItem::ImageArtifact { artifact_id, .. }
                    if artifact_id.as_str() == "artifact:budgeted"
            )
        })
        .unwrap();
    let image_provenance = primary_audit
        .context_delivery_map()
        .unwrap()
        .content_items()[image_position]
        .provenance();
    assert_eq!(
        image_provenance.references().artifact_refs(),
        [
            ArtifactId::new("artifact:original"),
            ArtifactId::new("artifact:budgeted"),
        ]
    );
    assert_eq!(
        image_provenance.references().source_event_refs(),
        [EventId::new("vision-route")]
    );
}

#[tokio::test]
async fn failed_secondary_attempt_has_one_audit_and_one_terminal_failure_without_primary_fallback()
{
    let primary_capabilities = capabilities(false, false);
    let vision_capabilities = capabilities(true, false);
    let vision_provider = ScriptedProvider::new(vision_capabilities.clone(), [failed_provider()]);
    let primary_provider = ScriptedProvider::new(
        primary_capabilities.clone(),
        [completed("must not be called", "unused-primary")],
    );
    let binding = binding();
    let context = audit_context(binding.clone());
    let mut projection = projection(&binding);
    let mut sink = EventLog::default();
    let blobs = MemoryBlobs::default();
    let error = VisionSessionService::new(&blobs, binding.clone())
        .analyze_image(
            &vision_provider,
            VisionSecondaryRequest {
                route: route(
                    "vision-route",
                    "artifact:original",
                    &primary_capabilities,
                    &vision_capabilities,
                    VisionMode::SecondaryTool,
                    VisionRoute::SecondaryTool,
                    VisionRouteReason::ForcedSecondaryTool,
                ),
                route_occurred_at: occurred_at(2),
                analysis: vision_input(&vision_capabilities),
            },
            &context,
            AgentCancellation::new(),
            &mut projection,
            &mut sink,
            || VisionEventStamp::new(EventId::new("vision-audit"), occurred_at(3)),
            || VisionEventStamp::new(EventId::new("vision-failed"), occurred_at(4)),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, VisionSessionError::AnalysisFailed { .. }));

    assert_eq!(vision_provider.requests().len(), 1);
    assert!(primary_provider.requests().is_empty());
    assert_eq!(projection.state().request_audits().len(), 1);
    assert_eq!(projection.state().vision_failures().len(), 1);
    assert!(projection.state().vision_observations().is_empty());
}

#[tokio::test]
async fn secondary_retry_keeps_one_route_and_binds_the_observation_to_the_final_audit() {
    let primary_capabilities = capabilities(false, false);
    let vision_capabilities = capabilities(true, false);
    let vision_provider = ScriptedProvider::new(
        vision_capabilities.clone(),
        [
            vec![Err(ModelProviderError::ConnectionFailure {
                timeout: false,
                io_kind: "Some(ConnectionReset)".to_owned(),
                message: "fixture connection reset".to_owned(),
            })],
            completed(
                r#"{"schema_version":2,"summary":"Recovered Vision response.","extracted_text":null,"regions":[],"task_observations":[],"uncertainty":["transport was interrupted"]}"#,
                "vision-retry-success",
            ),
        ],
    );
    let binding = binding();
    let context = audit_context(binding.clone());
    let mut projection = projection(&binding);
    let mut sink = EventLog::default();
    let blobs = MemoryBlobs::default();
    let mut audit_sequence = 0_u64;

    let output = VisionSessionService::new(&blobs, binding)
        .analyze_image(
            &vision_provider,
            VisionSecondaryRequest {
                route: route(
                    "vision-route-retry",
                    "artifact:original",
                    &primary_capabilities,
                    &vision_capabilities,
                    VisionMode::SecondaryTool,
                    VisionRoute::SecondaryTool,
                    VisionRouteReason::ForcedSecondaryTool,
                ),
                route_occurred_at: occurred_at(2),
                analysis: VisionAnalysisInput {
                    route_event_id: EventId::new("vision-route-retry"),
                    observation_id: VisionObservationId::new("observation:vision-retry"),
                    ..vision_input(&vision_capabilities)
                },
            },
            &context,
            AgentCancellation::new(),
            &mut projection,
            &mut sink,
            || {
                audit_sequence += 1;
                VisionEventStamp::new(
                    EventId::new(format!("vision-audit-retry-{audit_sequence}")),
                    occurred_at(2 + audit_sequence),
                )
            },
            || VisionEventStamp::new(EventId::new("vision-observation-retry"), occurred_at(5)),
        )
        .await
        .unwrap();

    assert_eq!(vision_provider.requests().len(), 2);
    assert_eq!(projection.state().vision_routes().len(), 1);
    assert_eq!(projection.state().request_audits().len(), 2);
    assert_eq!(projection.state().vision_observations().len(), 1);
    assert!(projection.state().vision_failures().is_empty());
    assert_eq!(
        output.observation.request_audit_event_id,
        EventId::new("vision-audit-retry-2"),
        "the final observation cannot identify the abandoned sample"
    );
    assert_eq!(
        sink.events
            .iter()
            .map(|event| event.event_type.as_str())
            .collect::<Vec<_>>(),
        vec![
            "vision.route_decided",
            "session.request_audit_recorded",
            "session.request_audit_recorded",
            "vision.observation_recorded",
        ]
    );
}

#[tokio::test]
async fn targeted_reanalysis_appends_new_observations_and_replay_validates_raw_blobs() {
    let primary_capabilities = capabilities(false, false);
    let vision_capabilities = capabilities(true, false);
    let provider = ScriptedProvider::new(
        vision_capabilities.clone(),
        [
            completed(
                r#"{"schema_version":2,"summary":"Initial screenshot overview.","extracted_text":null,"regions":[],"task_observations":["debug label"],"uncertainty":["control effect unknown"]}"#,
                "vision-targeted-1",
            ),
            completed(
                r#"{"schema_version":2,"summary":"Targeted control inspection.","extracted_text":"Debug","regions":[],"task_observations":["debug toggle"],"uncertainty":["requires runtime confirmation"]}"#,
                "vision-targeted-2",
            ),
        ],
    );
    let binding = binding();
    let context = audit_context(binding.clone());
    let mut projection = projection(&binding);
    let mut sink = EventLog::default();
    sink.events.push(started_event(&binding));
    let blobs = MemoryBlobs::default();
    let service = VisionSessionService::new(&blobs, binding.clone());

    let first = service
        .analyze_image(
            &provider,
            VisionSecondaryRequest {
                route: route(
                    "vision-route-targeted-1",
                    "artifact:original",
                    &primary_capabilities,
                    &vision_capabilities,
                    VisionMode::SecondaryTool,
                    VisionRoute::SecondaryTool,
                    VisionRouteReason::ForcedSecondaryTool,
                ),
                route_occurred_at: occurred_at(2),
                analysis: VisionAnalysisInput {
                    route_event_id: EventId::new("vision-route-targeted-1"),
                    observation_id: VisionObservationId::new("observation:targeted-1"),
                    provider_session: ProviderSessionRef {
                        id: ProviderSessionId::new("vision:targeted-1"),
                        role: ProviderSessionRole::Vision,
                        generation: 1,
                    },
                    task: "Inspect the complete screenshot.".to_owned(),
                    ..vision_input(&vision_capabilities)
                },
            },
            &context,
            AgentCancellation::new(),
            &mut projection,
            &mut sink,
            || VisionEventStamp::new(EventId::new("vision-audit-targeted-1"), occurred_at(3)),
            || {
                VisionEventStamp::new(
                    EventId::new("vision-observation-targeted-1"),
                    occurred_at(4),
                )
            },
        )
        .await
        .unwrap();
    let second = service
        .analyze_image(
            &provider,
            VisionSecondaryRequest {
                route: route(
                    "vision-route-targeted-2",
                    "artifact:original",
                    &primary_capabilities,
                    &vision_capabilities,
                    VisionMode::SecondaryTool,
                    VisionRoute::SecondaryTool,
                    VisionRouteReason::ForcedSecondaryTool,
                ),
                route_occurred_at: occurred_at(5),
                analysis: VisionAnalysisInput {
                    route_event_id: EventId::new("vision-route-targeted-2"),
                    observation_id: VisionObservationId::new("observation:targeted-2"),
                    provider_session: ProviderSessionRef {
                        id: ProviderSessionId::new("vision:targeted-2"),
                        role: ProviderSessionRole::Vision,
                        generation: 2,
                    },
                    task: "Inspect only the debug control.".to_owned(),
                    ..vision_input(&vision_capabilities)
                },
            },
            &context,
            AgentCancellation::new(),
            &mut projection,
            &mut sink,
            || VisionEventStamp::new(EventId::new("vision-audit-targeted-2"), occurred_at(6)),
            || {
                VisionEventStamp::new(
                    EventId::new("vision-observation-targeted-2"),
                    occurred_at(7),
                )
            },
        )
        .await
        .unwrap();

    assert_eq!(provider.requests().len(), 2);
    assert_eq!(projection.state().vision_observations().len(), 2);
    assert_eq!(
        projection.state().vision_observations()[0],
        first.observation
    );
    assert_eq!(
        projection.state().vision_observations()[1],
        second.observation
    );
    assert_ne!(first.observation.id, second.observation.id);

    let event_count = sink.events.len();
    let reused_session_error = service
        .analyze_image(
            &provider,
            VisionSecondaryRequest {
                route: route(
                    "vision-route-targeted-3",
                    "artifact:original",
                    &primary_capabilities,
                    &vision_capabilities,
                    VisionMode::SecondaryTool,
                    VisionRoute::SecondaryTool,
                    VisionRouteReason::ForcedSecondaryTool,
                ),
                route_occurred_at: occurred_at(8),
                analysis: VisionAnalysisInput {
                    route_event_id: EventId::new("vision-route-targeted-3"),
                    observation_id: VisionObservationId::new("observation:targeted-3"),
                    provider_session: ProviderSessionRef {
                        id: ProviderSessionId::new("vision:targeted-2"),
                        role: ProviderSessionRole::Vision,
                        generation: 3,
                    },
                    task: "Inspect another region.".to_owned(),
                    ..vision_input(&vision_capabilities)
                },
            },
            &context,
            AgentCancellation::new(),
            &mut projection,
            &mut sink,
            || VisionEventStamp::new(EventId::new("vision-audit-targeted-3"), occurred_at(9)),
            || {
                VisionEventStamp::new(
                    EventId::new("vision-observation-targeted-3"),
                    occurred_at(10),
                )
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(
        reused_session_error,
        VisionSessionError::VisionSessionUnavailable { .. }
    ));
    assert_eq!(provider.requests().len(), 2);
    assert_eq!(sink.events.len(), event_count);

    let replayed = SessionProjection::replay(sink.events.clone()).unwrap();
    let second_terminal_index = sink
        .events
        .iter()
        .rposition(|event| event.event_type.as_str() == "vision.observation_recorded")
        .unwrap();
    let mut unknown_schema = sink.events.clone();
    unknown_schema[second_terminal_index].payload["schema_version"] = serde_json::json!(4);
    assert!(matches!(
        SessionProjection::replay(unknown_schema),
        Err(SessionError::MalformedPayload { .. })
    ));
    let mut swapped_audit = sink.events.clone();
    swapped_audit[second_terminal_index].payload["request_audit_event_id"] =
        serde_json::json!("vision-audit-targeted-1");
    assert!(matches!(
        SessionProjection::replay(swapped_audit),
        Err(SessionError::VisionRequestAuditInputMismatch { .. })
    ));
    let mut wrong_source = sink.events.clone();
    wrong_source[second_terminal_index].payload["source_artifact_id"] =
        serde_json::json!("artifact:not-routed");
    assert!(matches!(
        SessionProjection::replay(wrong_source),
        Err(SessionError::VisionSourceArtifactNotRouted { .. })
    ));
    let mut reused_session = sink.events.clone();
    reused_session[second_terminal_index].payload["provider_session"]["id"] =
        serde_json::json!("vision:targeted-1");
    assert!(matches!(
        SessionProjection::replay(reused_session),
        Err(SessionError::VisionSessionIdentityReused { .. })
    ));
    let mut duplicate_projection = SessionProjection::replay(sink.events.clone()).unwrap();
    assert!(matches!(
        duplicate_projection.append(
            &mut EventLog::default(),
            SessionEvent::VisionObservationRecorded {
                observation: second.observation.clone(),
            }
            .into_envelope(
                EventId::new("vision-observation-targeted-duplicate"),
                binding.session_stream_id().clone(),
                EventSequence::new(8),
                occurred_at(11),
            )
            .unwrap(),
        ),
        Err(AppendAfterDurableError::Session(
            SessionError::VisionRequestAlreadyTerminated { .. }
        ))
    ));
    let replay = VisionReplayService::new(&blobs);
    assert_eq!(
        replay
            .replay_secondary(
                replayed.state(),
                &EventId::new("vision-route-targeted-2"),
                &ArtifactId::new("artifact:original"),
            )
            .unwrap(),
        second.observation
    );
    let writes_before_failure_checks = blobs.writes();
    let raw_blob_id = second.observation.raw_observation_blob_id.clone();
    let raw = blobs.remove(&raw_blob_id).unwrap();
    assert!(matches!(
        replay.replay_secondary(
            replayed.state(),
            &EventId::new("vision-route-targeted-2"),
            &ArtifactId::new("artifact:original"),
        ),
        Err(VisionReplayError::ObservationBlobRead { .. })
    ));
    blobs.insert_unchecked(raw_blob_id.clone(), b"{}".to_vec());
    assert!(matches!(
        replay.replay_secondary(
            replayed.state(),
            &EventId::new("vision-route-targeted-2"),
            &ArtifactId::new("artifact:original"),
        ),
        Err(VisionReplayError::ObservationBlobInvalid { .. })
    ));
    blobs.insert_unchecked(raw_blob_id, raw);
    assert_eq!(blobs.writes(), writes_before_failure_checks);
}

struct FixedViewImageExecutor {
    spec: ToolSpec,
    output: serde_json::Value,
}

impl ToolExecutor for FixedViewImageExecutor {
    fn specs(&self) -> Vec<ToolSpec> {
        vec![self.spec.clone()]
    }

    fn plan(
        &self,
        execution_id: ExecutionId,
        call: ToolCall,
    ) -> Result<ToolExecutionPlan, ToolExecutorError> {
        ToolExecutionPlan::new(execution_id, call, &self.spec, Vec::new()).map_err(|error| {
            ToolExecutorError::PlanFailure {
                tool_name: self.spec.name.to_string(),
                message: error.to_string(),
            }
        })
    }

    fn execute(&self, plan: ToolExecutionPlan) -> ToolExecutionFuture<'_> {
        let output = self.output.clone();
        Box::pin(async move { Ok(ToolExecutionOutcome::succeeded(plan, output)) })
    }
}

fn view_image_executor() -> FixedViewImageExecutor {
    let registry = ToolRegistry::for_project(env!("CARGO_MANIFEST_DIR"));
    let spec = registry
        .specs()
        .into_iter()
        .find(|spec| spec.name.as_str() == "view_image")
        .expect("the fixed Tool catalog must expose view_image");
    FixedViewImageExecutor {
        spec,
        output: serde_json::json!({
            "schema_version": 1,
            "image_artifact_ref": "artifact:budgeted",
            "original": {"artifact": {"id": "artifact:original"}},
            "derived": {
                "artifact": {
                    "id": "artifact:budgeted",
                    "transform": "image_budget_v1",
                    "parent_artifact_ids": ["artifact:original"]
                }
            },
            "delivery": {
                "artifact_id": "artifact:budgeted",
                "media_type": "image/png",
                "transform": "image_budget_v1"
            }
        }),
    }
}

fn view_image_tool_call() -> ToolCall {
    ToolCall::new(
        echidna::domain::tools::ToolCallId::new("call-view-image"),
        echidna::domain::tools::ToolName::new("view_image"),
        serde_json::json!({}),
    )
}

fn completed_tool_call(
    call: ToolCall,
    response_id: &str,
) -> Vec<Result<ModelStreamEvent, ModelProviderError>> {
    vec![
        Ok(ModelStreamEvent::Event(ModelEvent::ToolCall { call })),
        Ok(ModelStreamEvent::Completed(ModelResponseCompletion::new(
            echidna::ports::model::ProviderResponseId::new(response_id),
        ))),
    ]
}

#[tokio::test]
async fn view_image_tool_result_requires_fixed_follow_up_before_the_next_primary_sample() {
    let primary_capabilities = capabilities(true, false);
    let provider = ScriptedProvider::new(
        primary_capabilities.clone(),
        [
            completed_tool_call(view_image_tool_call(), "primary-tool-call"),
            completed(
                "Primary received exactly one image Artifact.",
                "primary-image",
            ),
        ],
    );
    let binding = binding();
    let context = audit_context(binding.clone());
    let projection = Rc::new(RefCell::new(projection(&binding)));
    let sink = Rc::new(RefCell::new(EventLog::default()));
    let blobs = MemoryBlobs::default();
    let audit_service = RequestAuditService::new(&blobs, binding.clone());
    let tools = view_image_executor();
    let mut agent = AgentLoop::with_primary_context(
        &provider,
        &tools,
        ModelIdentity::new("fixture-primary", "primary-v1"),
        ModelProfile::new("primary", 1),
        ProviderSession::new(
            ProviderSessionId::new("primary:view-image-follow-up"),
            ProviderSessionRole::Primary,
            1,
        ),
    )
    .unwrap()
    .with_domain_profile(DomainProfile::Mobile);
    let mut audit_number = 0_u64;
    let audit_projection = projection.clone();
    let audit_sink = sink.clone();
    let mut record_primary = |attempt| {
        audit_number += 1;
        let mut projection = audit_projection.borrow_mut();
        let mut sink = audit_sink.borrow_mut();
        audit_service
            .record(
                attempt,
                EventId::new(format!("primary-audit-{audit_number}")),
                occurred_at(audit_number),
                &mut projection,
                &mut *sink,
            )
            .map(|_| ())
            .map_err(RequestAuditServiceError::into_agent_loop_error)
    };
    let follow_up_calls = Arc::new(AtomicUsize::new(0));
    let follow_up_calls_for_stage = follow_up_calls.clone();
    let follow_up_projection = projection.clone();
    let follow_up_sink = sink.clone();
    let follow_up_binding = binding.clone();
    let follow_up_primary_capabilities = primary_capabilities.clone();
    let blobs_ref = &blobs;
    let mut follow_up = move |selection: ViewImageSelection| -> ViewImageFollowUpFuture<'_> {
        follow_up_calls_for_stage.fetch_add(1, Ordering::SeqCst);
        let mut projection = follow_up_projection.borrow_mut();
        let mut sink = follow_up_sink.borrow_mut();
        let context = VisionSessionService::new(blobs_ref, follow_up_binding.clone())
            .prepare_direct(
                VisionDirectRequest {
                    route: route(
                        "vision-route:view-image",
                        selection.source_artifact_id.as_str(),
                        &follow_up_primary_capabilities,
                        &capabilities(true, false),
                        VisionMode::PrimaryDirect,
                        VisionRoute::PrimaryDirect,
                        VisionRouteReason::ForcedPrimaryDirect,
                    ),
                    route_occurred_at: occurred_at(2),
                    source_artifact_id: selection.source_artifact_id,
                    delivery_artifact_id: selection.delivery_artifact_id,
                },
                &mut projection,
                &mut *sink,
            )
            .map_err(|error| AgentLoopError::InitialImageContext {
                message: error.to_string(),
            });
        Box::pin(async move { context })
    };

    agent
        .run_turn_audited_with_view_image_follow_up(
            "Inspect the screenshot.",
            AgentCancellation::new(),
            &context,
            &mut record_primary,
            &mut follow_up,
        )
        .await
        .unwrap();

    let requests = provider.requests();
    assert_eq!(follow_up_calls.load(Ordering::SeqCst), 1);
    assert_eq!(requests.len(), 2);
    assert_eq!(projection.borrow().state().vision_routes().len(), 1);
    let event_types = sink
        .borrow()
        .events
        .iter()
        .map(|event| event.event_type.as_str().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        event_types,
        vec![
            "session.request_audit_recorded",
            "vision.route_decided",
            "session.request_audit_recorded",
        ]
    );
    assert_eq!(
        requests[1]
            .input()
            .content_items
            .iter()
            .filter(|item| matches!(item, ContentItem::ImageArtifact { artifact_id, .. } if artifact_id.as_str() == "artifact:budgeted"))
            .count(),
        1
    );
}

#[tokio::test]
async fn view_image_tool_result_without_follow_up_fails_before_a_second_primary_sample() {
    let provider = ScriptedProvider::new(
        capabilities(true, false),
        [completed_tool_call(
            view_image_tool_call(),
            "primary-tool-call",
        )],
    );
    let binding = binding();
    let context = audit_context(binding.clone());
    let mut projection = projection(&binding);
    let mut sink = EventLog::default();
    let blobs = MemoryBlobs::default();
    let audit_service = RequestAuditService::new(&blobs, binding.clone());
    let tools = view_image_executor();
    let mut agent = AgentLoop::with_primary_context(
        &provider,
        &tools,
        ModelIdentity::new("fixture-primary", "primary-v1"),
        ModelProfile::new("primary", 1),
        ProviderSession::new(
            ProviderSessionId::new("primary:view-image-follow-up"),
            ProviderSessionRole::Primary,
            1,
        ),
    )
    .unwrap()
    .with_domain_profile(DomainProfile::Mobile);
    let mut record_primary = |attempt| {
        audit_service
            .record(
                attempt,
                EventId::new("primary-audit"),
                occurred_at(1),
                &mut projection,
                &mut sink,
            )
            .map(|_| ())
            .map_err(RequestAuditServiceError::into_agent_loop_error)
    };

    let error = agent
        .run_turn_audited(
            "Inspect the screenshot.",
            AgentCancellation::new(),
            &context,
            &mut record_primary,
        )
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        AgentLoopError::ViewImageFollowUpRequired { call_id } if call_id.as_str() == "call-view-image"
    ));
    assert_eq!(provider.requests().len(), 1);
}
