use super::*;

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::sync::Mutex;

use cyber_agent::app::agent_loop::AgentLoopError;
use cyber_agent::app::request_audit::{RequestAuditService, RequestAuditServiceError};
use cyber_agent::app::session::SessionProjection;
use cyber_agent::domain::artifacts::BlobId;
use cyber_agent::domain::context::{ProviderSessionId, ProviderSessionRef, ProviderSessionRole};
use cyber_agent::domain::events::{EventEnvelope, EventId, EventSequence, OccurredAt};
use cyber_agent::domain::request_audit::{
    RequestAdapterVersion, RequestAuditBinding, RequestAuditDeliveryReason, RequestAuditRunContext,
};
use cyber_agent::domain::session::SessionEvent;
use cyber_agent::ports::EventSink;
use cyber_agent::ports::artifact::{ArtifactStore, StoredBlob};
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
    let mut agent = AgentLoop::new(&provider, &tools, "fixture instructions");
    let audit_context = RequestAuditRunContext::new(
        RequestAuditBinding::new(
            ProjectId::new("project-audit-loop"),
            StreamId::new("session:audit-loop"),
        )
        .unwrap(),
        RequestAdapterVersion::new("fixture-adapter-v1").unwrap(),
    );
    let mut audits = Vec::new();
    {
        let mut record = |attempt| {
            audits.push(attempt);
            Ok(())
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
    assert_eq!(audits.len(), requests.len());
    assert_eq!(
        audits[0].delivery().reason(),
        RequestAuditDeliveryReason::Full(BaselineInvalidationReason::NoBaseline)
    );
    assert_eq!(
        audits[1].delivery().reason(),
        RequestAuditDeliveryReason::AppendOnlyCandidate
    );
    assert_eq!(
        audits[2].delivery().reason(),
        RequestAuditDeliveryReason::Full(BaselineInvalidationReason::ProviderRejected)
    );
    assert!(
        audits.iter().all(|attempt| {
            attempt.binding().session_stream_id().as_str() == "session:audit-loop"
        })
    );
}

#[tokio::test]
async fn audited_agent_loop_commits_through_the_real_request_audit_service_before_streaming() {
    let root = TemporaryDirectory::new("m3-request-audit-real-service");
    let tools = ToolRegistry::for_project(root.path());
    let provider = ScriptedProvider::new(
        incremental_capabilities(),
        [completed_response("recorded", "response-recorded")],
    );
    let mut agent = AgentLoop::new(&provider, &tools, "fixture instructions");
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
    assert_eq!(sink.events.len(), 1);
    assert!(blobs.blob_count() > 0);
}

#[tokio::test]
async fn request_audit_commit_failure_stops_before_the_provider_stream() {
    let root = TemporaryDirectory::new("m3-request-audit-commit-failure");
    let tools = ToolRegistry::for_project(root.path());
    let provider = ScriptedProvider::new(
        incremental_capabilities(),
        [completed_response("must not stream", "response-unused")],
    );
    let mut agent = AgentLoop::new(&provider, &tools, "fixture instructions");
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
