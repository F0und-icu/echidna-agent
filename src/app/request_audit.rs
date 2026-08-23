//! Blob publication, durable Session commit, and read-only request recovery.

use crate::app::agent_loop::AgentLoopError;
use crate::app::session::{AppendAfterDurableError, SessionProjection};
use crate::domain::events::{EventId, OccurredAt};
use crate::domain::model::ModelInputIR;
use crate::domain::request_audit::{
    PublishedAuditBlob, RequestAuditAttempt, RequestAuditBinding, RequestAuditError,
    RequestAuditManifest, RequestAuditReconstructionError,
};
use crate::domain::session::{SessionError, SessionEvent};
use crate::ports::EventSink;
use crate::ports::artifact::ArtifactStore;

/// The named App service for the only M3-S3c write path. It publishes exact
/// rendered bytes first, then makes the Manifest visible only through the
/// existing append-before-apply Session composition.
pub struct RequestAuditService<'store, Store: ?Sized> {
    blobs: &'store Store,
    binding: RequestAuditBinding,
}

impl<'store, Store> RequestAuditService<'store, Store>
where
    Store: ArtifactStore + ?Sized,
{
    pub fn new(blobs: &'store Store, binding: RequestAuditBinding) -> Self {
        Self { blobs, binding }
    }

    /// Publishes every component before appending the Session fact. A Blob
    /// failure leaves the Projection unchanged; an Event append failure may
    /// leave intentionally unreachable orphan Blobs but never an applied fact.
    pub fn record<S>(
        &self,
        attempt: RequestAuditAttempt,
        event_id: EventId,
        occurred_at: OccurredAt,
        projection: &mut SessionProjection,
        sink: &mut S,
    ) -> Result<RequestAuditManifest, RequestAuditServiceError<Store::Error, S::Error>>
    where
        S: EventSink<serde_json::Value>,
    {
        self.require_binding(attempt.binding())
            .map_err(RequestAuditServiceError::Audit)?;
        self.require_projection(projection, &event_id)?;
        let rendered = attempt.render().map_err(RequestAuditServiceError::Audit)?;
        let blobs = rendered
            .component_bytes()
            .map(|bytes| {
                self.blobs
                    .publish_bytes(bytes)
                    .map(|stored| PublishedAuditBlob {
                        blob_id: stored.blob_id,
                        size_bytes: stored.size_bytes,
                    })
                    .map_err(RequestAuditServiceError::Blob)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let manifest = rendered
            .materialize(event_id.clone(), blobs)
            .map_err(RequestAuditServiceError::Audit)?;
        let stream_id = manifest.binding().session_stream_id().clone();
        let sequence = projection
            .next_event_sequence()
            .map_err(RequestAuditServiceError::Session)?;
        let event = SessionEvent::RequestAuditRecorded {
            manifest: manifest.clone(),
        }
        .into_envelope(event_id, stream_id, sequence, occurred_at)
        .map_err(RequestAuditServiceError::Session)?;
        projection
            .append(sink, event)
            .map_err(RequestAuditServiceError::Append)?;
        Ok(manifest)
    }

    /// Reconstructs the actual delivered semantic input through Blob reads
    /// only. It has no Provider, Tool, credential, Event append or state
    /// mutation path.
    pub fn reconstruct(
        &self,
        manifest: &RequestAuditManifest,
    ) -> Result<ModelInputIR, RequestAuditReconstructionError<Store::Error>> {
        self.require_binding(manifest.binding())
            .map_err(RequestAuditReconstructionError::Audit)?;
        manifest.reconstruct(|blob_id| self.blobs.read_blob(blob_id))
    }

    fn require_binding(&self, binding: &RequestAuditBinding) -> Result<(), RequestAuditError> {
        if binding != &self.binding {
            return Err(RequestAuditError::ServiceBindingMismatch {
                expected: self.binding.clone(),
                received: binding.clone(),
            });
        }
        Ok(())
    }

    fn require_projection<BlobError, SinkError>(
        &self,
        projection: &SessionProjection,
        event_id: &EventId,
    ) -> Result<(), RequestAuditServiceError<BlobError, SinkError>> {
        let Some(stream_id) = projection.state().stream_id() else {
            return Err(RequestAuditServiceError::Session(
                SessionError::RequestAuditSessionStreamMissing,
            ));
        };
        if stream_id != self.binding.session_stream_id() {
            return Err(RequestAuditServiceError::Session(
                SessionError::RequestAuditSessionStreamMismatch {
                    expected: stream_id.clone(),
                    received: self.binding.session_stream_id().clone(),
                },
            ));
        }
        if projection.state().contains_event_id(event_id) {
            return Err(RequestAuditServiceError::Audit(
                RequestAuditError::EventIdAlreadyApplied {
                    event_id: event_id.as_str().to_owned(),
                },
            ));
        }
        Ok(())
    }
}

/// Distinguishes rendering/shape failure, immutable Blob failure, Session
/// validation and durable Event append without flattening caller errors.
#[derive(Debug)]
pub enum RequestAuditServiceError<BlobError, SinkError> {
    Audit(RequestAuditError),
    Blob(BlobError),
    Session(SessionError),
    Append(AppendAfterDurableError<SinkError>),
}

impl<BlobError, SinkError> RequestAuditServiceError<BlobError, SinkError>
where
    BlobError: std::fmt::Debug,
    SinkError: std::fmt::Debug,
{
    /// Converts the one audited AgentLoop callback failure without widening the
    /// runtime into a general persistence or Hook error abstraction.
    pub fn into_agent_loop_error(self) -> AgentLoopError {
        let message = match self {
            Self::Audit(error) => error.to_string(),
            Self::Blob(error) => format!("publish RequestAudit Blob: {error:?}"),
            Self::Session(error) => error.to_string(),
            Self::Append(AppendAfterDurableError::Session(error)) => error.to_string(),
            Self::Append(AppendAfterDurableError::Sink(error)) => {
                format!("append RequestAudit Session Event: {error:?}")
            }
        };
        AgentLoopError::RequestAuditCommit { message }
    }
}
