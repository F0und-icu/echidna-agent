use super::*;

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::sync::Mutex;

use echidna::app::agent_loop::AgentLoopError;
use echidna::app::request_audit::{RequestAuditService, RequestAuditServiceError};
use echidna::app::session::SessionProjection;
use echidna::domain::artifacts::BlobId;
use echidna::domain::context::{ProviderSessionId, ProviderSessionRef, ProviderSessionRole};
use echidna::domain::events::{EventEnvelope, EventId, EventSequence, OccurredAt};
use echidna::domain::request_audit::{
    RequestAdapterVersion, RequestAuditBinding, RequestAuditDeliveryReason, RequestAuditRunContext,
};
use echidna::domain::session::SessionEvent;
use echidna::ports::EventSink;
use echidna::ports::artifact::{ArtifactStore, StoredBlob};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Eq, PartialEq)]
enum AuditBlobFailure {
    Missing,
    Publish,
}

#[derive(Default)]
struct AuditBlobStore {
    blobs: Mutex<BTreeMap<BlobId, Vec<u8>>>,
}

impl AuditBlobStore {
    fn blob_count(&self) -> usize {
        self.blobs.lock().unwrap().len()
    }
}

impl ArtifactStore for AuditBlobStore {
    type Error = AuditBlobFailure;

    fn publish_file(&self, _source: &std::path::Path) -> Result<StoredBlob, Self::Error> {
        unreachable!("RequestAudit publishes rendered bytes")
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
            .ok_or(AuditBlobFailure::Missing)
    }
}

struct FailingAuditBlobStore;

impl ArtifactStore for FailingAuditBlobStore {
    type Error = AuditBlobFailure;

    fn publish_file(&self, _source: &std::path::Path) -> Result<StoredBlob, Self::Error> {
        unreachable!("RequestAudit publishes rendered bytes")
    }

    fn publish_bytes(&self, _bytes: &[u8]) -> Result<StoredBlob, Self::Error> {
        Err(AuditBlobFailure::Publish)
    }

    fn read_blob(&self, _blob_id: &BlobId) -> Result<Vec<u8>, Self::Error> {
        Err(AuditBlobFailure::Missing)
    }
}

#[derive(Default)]
struct AuditSink {
    events: Vec<EventEnvelope<Value>>,
}

impl EventSink<Value> for AuditSink {
    type Error = Infallible;

    fn append_event(&mut self, event: &EventEnvelope<Value>) -> Result<(), Self::Error> {
        self.events.push(event.clone());
        Ok(())
    }
}

fn audit_projection(stream_id: StreamId) -> SessionProjection {
    SessionProjection::replay([SessionEvent::Started {
        primary_provider_session: ProviderSessionRef {
            id: ProviderSessionId::new("primary-audit-loop"),
            role: ProviderSessionRole::Primary,
            generation: 1,
        },
    }
    .into_envelope(
        EventId::new("audit-loop-started"),
        stream_id,
        EventSequence::new(1),
        OccurredAt::new("2026-08-23T00:00:00Z"),
    )
    .unwrap()])
    .unwrap()
}

#[tokio::test]
async fn audited_agent_loop_records_each_full_incremental_and_provider_rejected_fallback_attempt() {
    let root = TemporaryDirectory::new("m3-request-audit-agent-loop");
    let tools = ToolRegistry::for_project(root.path());
    let provider = ScriptedProvider::new(
        incremental_capabilities(),
        [
            completed_response("first", "response-first"),
            rejected_response(),
            completed_response("fallback", "response-fallback"),
        ],
    );
    let mut agent = AgentLoop::new(&provider, &tools);
    let audit_context = RequestAuditRunContext::new(
        RequestAuditBinding::new(
            ProjectId::new("project-audit-loop"),
            StreamId::new("session:audit-loop"),
        )
        .unwrap(),
        RequestAdapterVersion::new("fixture-adapter-v1").unwrap(),
    );
    let binding = audit_context.binding().clone();
    let blobs = AuditBlobStore::default();
    let service = RequestAuditService::new(&blobs, binding.clone());
    let mut projection = audit_projection(binding.session_stream_id().clone());
    let mut sink = AuditSink::default();
    let mut manifests = Vec::new();
    let mut audit_index = 0_u32;
    {
        let mut record = |attempt| {
            audit_index += 1;
            service
                .record(
                    attempt,
                    EventId::new(format!("audit-loop-request-{audit_index}")),
                    OccurredAt::new(format!("2026-08-23T00:00:0{audit_index}Z")),
                    &mut projection,
                    &mut sink,
                )
                .map(|manifest| manifests.push(manifest))
                .map_err(RequestAuditServiceError::into_agent_loop_error)
        };
        agent
            .run_turn_audited(
                "first user input",
                AgentCancellation::new(),
                &audit_context,
                &mut record,
            )
            .await
            .unwrap();
        agent
            .run_turn_audited(
                "second user input",
                AgentCancellation::new(),
                &audit_context,
                &mut record,
            )
            .await
            .unwrap();
    }

    let requests = provider.requests();
    assert_eq!(requests.len(), 3);
    assert!(!requests[0].is_incremental());
    assert!(requests[1].is_incremental());
    assert!(!requests[2].is_incremental());
    assert_eq!(manifests.len(), requests.len());
    assert_eq!(
        manifests[0].delivery().reason(),
        RequestAuditDeliveryReason::Full(BaselineInvalidationReason::NoBaseline)
    );
    assert_eq!(
        manifests[1].delivery().reason(),
        RequestAuditDeliveryReason::AppendOnlyCandidate
    );
    assert_eq!(
        manifests[2].delivery().reason(),
        RequestAuditDeliveryReason::Full(BaselineInvalidationReason::ProviderRejected)
    );
    for (manifest, request) in manifests.iter().zip(&requests) {
        assert_eq!(manifest.schema_version(), 3);
        assert_eq!(
            manifest.binding().session_stream_id().as_str(),
            "session:audit-loop"
        );
        let reconstructed = service.reconstruct(manifest).unwrap();
        assert_eq!(&reconstructed, request.input());
        manifest
            .context_delivery_map()
            .expect("RequestAudit v3 retains the actual delivered map")
            .validate_against(&reconstructed)
            .unwrap();
    }
    assert_eq!(projection.state().request_audits(), manifests);
    assert_eq!(sink.events.len(), requests.len());
}

#[tokio::test]
async fn audited_agent_loop_commits_through_the_real_request_audit_service_before_streaming() {
    let root = TemporaryDirectory::new("m3-request-audit-real-service");
    let tools = ToolRegistry::for_project(root.path());
    let provider = ScriptedProvider::new(
        incremental_capabilities(),
        [completed_response("recorded", "response-recorded")],
    );
    let mut agent = AgentLoop::new(&provider, &tools);
    let binding = RequestAuditBinding::new(
        ProjectId::new("project-audit-real-service"),
        StreamId::new("session:audit-real-service"),
    )
    .unwrap();
    let audit_context = RequestAuditRunContext::new(
        binding.clone(),
        RequestAdapterVersion::new("fixture-adapter-v1").unwrap(),
    );
    let blobs = AuditBlobStore::default();
    let service = RequestAuditService::new(&blobs, binding.clone());
    let mut projection = audit_projection(binding.session_stream_id().clone());
    let mut sink = AuditSink::default();
    {
        let mut record = |attempt| {
            service
                .record(
                    attempt,
                    EventId::new("audit-real-service-request"),
                    OccurredAt::new("2026-08-23T00:00:01Z"),
                    &mut projection,
                    &mut sink,
                )
                .map(|_| ())
                .map_err(RequestAuditServiceError::into_agent_loop_error)
        };
        agent
            .run_turn_audited(
                "user input",
                AgentCancellation::new(),
                &audit_context,
                &mut record,
            )
            .await
            .unwrap();
    }

    assert_eq!(provider.requests().len(), 1);
    assert_eq!(projection.state().request_audits().len(), 1);
    let manifest = &projection.state().request_audits()[0];
    assert_eq!(manifest.schema_version(), 3);
    assert_eq!(manifest.prompt_renderer().unwrap().id(), "generic");
    assert_eq!(manifest.prompt_renderer().unwrap().version(), 1);
    assert_eq!(
        manifest.resolved_tool_plan_fingerprint(),
        agent
            .last_sampling_step_plan()
            .map(|step| step.resolved_tool_plan().fingerprint())
    );
    let reconstructed = service.reconstruct(manifest).unwrap();
    manifest
        .context_delivery_map()
        .expect("AgentLoop RequestAudit must persist delivered provenance")
        .validate_against(&reconstructed)
        .unwrap();
    assert_eq!(&reconstructed, provider.requests()[0].input());
    assert_eq!(sink.events.len(), 1);
    assert!(blobs.blob_count() > 0);
}

#[tokio::test]
async fn request_audit_v3_reconstruction_rejects_delivery_map_tampering() {
    let root = TemporaryDirectory::new("m4-request-audit-v3-tamper");
    let tools = ToolRegistry::for_project(root.path());
    let provider = ScriptedProvider::new(
        incremental_capabilities(),
        [completed_response("recorded", "response-recorded")],
    );
    let mut agent = AgentLoop::new(&provider, &tools);
    let binding = RequestAuditBinding::new(
        ProjectId::new("project-audit-v3-tamper"),
        StreamId::new("session:audit-v3-tamper"),
    )
    .unwrap();
    let audit_context = RequestAuditRunContext::new(
        binding.clone(),
        RequestAdapterVersion::new("fixture-adapter-v1").unwrap(),
    );
    let blobs = AuditBlobStore::default();
    let service = RequestAuditService::new(&blobs, binding.clone());
    let mut projection = audit_projection(binding.session_stream_id().clone());
    let mut sink = AuditSink::default();
    {
        let mut record = |attempt| {
            service
                .record(
                    attempt,
                    EventId::new("audit-v3-tamper-request"),
                    OccurredAt::new("2026-08-23T00:00:01Z"),
                    &mut projection,
                    &mut sink,
                )
                .map(|_| ())
                .map_err(RequestAuditServiceError::into_agent_loop_error)
        };
        agent
            .run_turn_audited(
                "user input",
                AgentCancellation::new(),
                &audit_context,
                &mut record,
            )
            .await
            .unwrap();
    }

    let encoded = serde_json::to_value(&projection.state().request_audits()[0]).unwrap();
    assert_eq!(encoded["schema_version"], serde_json::json!(3));

    let mut wrong_position = encoded.clone();
    wrong_position["context_delivery_map"]["content_items"][0]["final_position"] =
        serde_json::json!(1);
    let wrong_position: echidna::domain::request_audit::RequestAuditManifest =
        serde_json::from_value(wrong_position).unwrap();
    assert!(matches!(
        wrong_position.validate(),
        Err(echidna::domain::request_audit::RequestAuditError::ContextDeliveryMap { .. })
    ));
    assert!(matches!(
        service.reconstruct(&wrong_position),
        Err(
            echidna::domain::request_audit::RequestAuditReconstructionError::Audit(
                echidna::domain::request_audit::RequestAuditError::ContextDeliveryMap { .. }
            )
        )
    ));

    let mut wrong_hash = encoded.clone();
    wrong_hash["context_delivery_map"]["content_items"][0]["content_hash"] =
        serde_json::json!("a".repeat(64));
    let wrong_hash: echidna::domain::request_audit::RequestAuditManifest =
        serde_json::from_value(wrong_hash).unwrap();
    wrong_hash
        .validate()
        .expect("content bytes are required to detect a forged map hash");
    assert!(matches!(
        service.reconstruct(&wrong_hash),
        Err(
            echidna::domain::request_audit::RequestAuditReconstructionError::Audit(
                echidna::domain::request_audit::RequestAuditError::ContextDeliveryMap { .. }
            )
        )
    ));

    let mut wrong_provenance = encoded;
    wrong_provenance["context_delivery_map"]["content_items"][0]["provenance"]["authority_level"] =
        serde_json::json!("fact_reference");
    let wrong_provenance: echidna::domain::request_audit::RequestAuditManifest =
        serde_json::from_value(wrong_provenance).unwrap();
    assert!(matches!(
        wrong_provenance.validate(),
        Err(echidna::domain::request_audit::RequestAuditError::ContextDeliveryMap { .. })
    ));
    assert!(matches!(
        service.reconstruct(&wrong_provenance),
        Err(
            echidna::domain::request_audit::RequestAuditReconstructionError::Audit(
                echidna::domain::request_audit::RequestAuditError::ContextDeliveryMap { .. }
            )
        )
    ));
}

#[tokio::test]
async fn full_only_state_diff_audit_map_matches_the_actual_candidate() {
    let root = TemporaryDirectory::new("m4-request-audit-state-diff");
    let tools = ToolRegistry::for_project(root.path());
    let provider = ScriptedProvider::new(
        capabilities(false),
        [
            completed_response("first", "response-first"),
            completed_response("second", "response-second"),
        ],
    );
    let mut agent = AgentLoop::new(&provider, &tools).with_world_state(world_state("a"));
    let binding = RequestAuditBinding::new(
        ProjectId::new("project-audit-state-diff"),
        StreamId::new("session:audit-state-diff"),
    )
    .unwrap();
    let audit_context = RequestAuditRunContext::new(
        binding.clone(),
        RequestAdapterVersion::new("fixture-adapter-v1").unwrap(),
    );
    let blobs = AuditBlobStore::default();
    let service = RequestAuditService::new(&blobs, binding.clone());
    let mut projection = audit_projection(binding.session_stream_id().clone());
    let mut sink = AuditSink::default();
    {
        let mut audit_index = 0_u32;
        let mut record = |attempt| {
            audit_index += 1;
            service
                .record(
                    attempt,
                    EventId::new(format!("audit-state-diff-{audit_index}")),
                    OccurredAt::new(format!("2026-08-23T00:00:0{audit_index}Z")),
                    &mut projection,
                    &mut sink,
                )
                .map(|_| ())
                .map_err(RequestAuditServiceError::into_agent_loop_error)
        };

        agent
            .run_turn_audited(
                "first user input",
                AgentCancellation::new(),
                &audit_context,
                &mut record,
            )
            .await
            .unwrap();
        agent.set_world_state(world_state("b"));
        agent
            .run_turn_audited(
                "second user input",
                AgentCancellation::new(),
                &audit_context,
                &mut record,
            )
            .await
            .unwrap();
    }

    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert!(!requests[1].is_incremental());
    assert!(
        requests[1]
            .input()
            .content_items
            .iter()
            .any(|item| matches!(
                item,
                ContentItem::Text { text, .. } if text.contains("research_world_state_diff")
            ))
    );
    let manifest = &projection.state().request_audits()[1];
    assert_eq!(
        manifest.delivery().reason(),
        RequestAuditDeliveryReason::Full(BaselineInvalidationReason::IncrementalInputUnsupported)
    );
    let reconstructed = service.reconstruct(manifest).unwrap();
    assert_eq!(&reconstructed, requests[1].input());
    manifest
        .context_delivery_map()
        .unwrap()
        .validate_against(&reconstructed)
        .unwrap();
}

#[tokio::test]
async fn full_reasoning_tool_continuation_audit_preserves_order_and_provenance() {
    let root = TemporaryDirectory::new("m4-request-audit-reasoning-continuation");
    let tools = ToolRegistry::for_project(root.path());
    let provider = ScriptedProvider::new(
        capabilities(false),
        [
            vec![
                Ok(ModelStreamEvent::Event(ModelEvent::ReasoningItem {
                    text: "inspect the available research tools".to_owned(),
                })),
                Ok(ModelStreamEvent::Event(ModelEvent::ToolCall {
                    call: ToolCall::new(
                        ToolCallId::new("reasoning-tool-search"),
                        ToolName::new("tool_search"),
                        serde_json::json!({"query": "file inspection"}),
                    ),
                })),
                Ok(ModelStreamEvent::Completed(ModelResponseCompletion::new(
                    ProviderResponseId::new("response-reasoning-tool"),
                ))),
            ],
            completed_response("tool result considered", "response-after-tool"),
        ],
    );
    let mut agent = AgentLoop::new(&provider, &tools);
    let binding = RequestAuditBinding::new(
        ProjectId::new("project-audit-reasoning"),
        StreamId::new("session:audit-reasoning"),
    )
    .unwrap();
    let audit_context = RequestAuditRunContext::new(
        binding.clone(),
        RequestAdapterVersion::new("fixture-adapter-v1").unwrap(),
    );
    let blobs = AuditBlobStore::default();
    let service = RequestAuditService::new(&blobs, binding.clone());
    let mut projection = audit_projection(binding.session_stream_id().clone());
    let mut sink = AuditSink::default();
    {
        let mut audit_index = 0_u32;
        let mut record = |attempt| {
            audit_index += 1;
            service
                .record(
                    attempt,
                    EventId::new(format!("audit-reasoning-{audit_index}")),
                    OccurredAt::new(format!("2026-08-23T00:00:0{audit_index}Z")),
                    &mut projection,
                    &mut sink,
                )
                .map(|_| ())
                .map_err(RequestAuditServiceError::into_agent_loop_error)
        };

        agent
            .run_turn_audited(
                "inspect the project",
                AgentCancellation::new(),
                &audit_context,
                &mut record,
            )
            .await
            .unwrap();
    }

    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert!(!requests[1].is_incremental());
    let items = &requests[1].input().content_items;
    let reasoning_position = items
        .iter()
        .position(|item| matches!(item, ContentItem::Reasoning { .. }))
        .unwrap();
    assert!(
        matches!(items.get(reasoning_position + 1), Some(ContentItem::ToolCall { call }) if call.call_id.as_str() == "reasoning-tool-search")
    );
    assert!(
        matches!(items.get(reasoning_position + 2), Some(ContentItem::ToolResult { result }) if result.call_id.as_str() == "reasoning-tool-search")
    );

    let manifest = &projection.state().request_audits()[1];
    let reconstructed = service.reconstruct(manifest).unwrap();
    assert_eq!(&reconstructed, requests[1].input());
    let map = manifest.context_delivery_map().unwrap();
    map.validate_against(&reconstructed).unwrap();
    assert_eq!(
        map.content_items()[reasoning_position]
            .provenance()
            .authority_level(),
        echidna::domain::context::AuthorityLevel::ModelObservation
    );
    assert_eq!(
        map.content_items()[reasoning_position + 1]
            .provenance()
            .authority_level(),
        echidna::domain::context::AuthorityLevel::ModelObservation
    );
    assert_eq!(
        map.content_items()[reasoning_position + 2]
            .provenance()
            .authority_level(),
        echidna::domain::context::AuthorityLevel::DerivedNonEvidence
    );
}

#[tokio::test]
async fn request_audit_commit_failure_stops_before_the_provider_stream() {
    let root = TemporaryDirectory::new("m3-request-audit-commit-failure");
    let tools = ToolRegistry::for_project(root.path());
    let provider = ScriptedProvider::new(
        incremental_capabilities(),
        [completed_response("must not stream", "response-unused")],
    );
    let mut agent = AgentLoop::new(&provider, &tools);
    let binding = RequestAuditBinding::new(
        ProjectId::new("project-audit-commit-failure"),
        StreamId::new("session:audit-commit-failure"),
    )
    .unwrap();
    let audit_context = RequestAuditRunContext::new(
        binding.clone(),
        RequestAdapterVersion::new("fixture-adapter-v1").unwrap(),
    );
    let blobs = FailingAuditBlobStore;
    let service = RequestAuditService::new(&blobs, binding.clone());
    let mut projection = audit_projection(binding.session_stream_id().clone());
    let mut sink = AuditSink::default();
    let error = {
        let mut record = |attempt| {
            service
                .record(
                    attempt,
                    EventId::new("audit-commit-failure-request"),
                    OccurredAt::new("2026-08-23T00:00:01Z"),
                    &mut projection,
                    &mut sink,
                )
                .map(|_| ())
                .map_err(RequestAuditServiceError::into_agent_loop_error)
        };
        agent
            .run_turn_audited(
                "user input",
                AgentCancellation::new(),
                &audit_context,
                &mut record,
            )
            .await
            .unwrap_err()
    };

    assert!(matches!(
        error,
        AgentLoopError::RequestAuditCommit { ref message }
            if message.contains("publish RequestAudit Blob")
    ));
    assert!(provider.requests().is_empty());
    assert!(projection.state().request_audits().is_empty());
    assert!(sink.events.is_empty());
}
