//! Read-only Mechanical Replay for durable Vision observations.

use thiserror::Error;

use super::analysis::decode_model_reply;
use crate::domain::artifacts::ArtifactId;
use crate::domain::events::EventId;
use crate::domain::model::{ContentItem, MessageRole};
use crate::domain::profile::VisionRoute;
use crate::domain::session::SessionState;
use crate::domain::vision::VisionObservation;
use crate::ports::artifact::ArtifactStore;

pub struct VisionReplayService<'blobs, Blobs: ?Sized> {
    blobs: &'blobs Blobs,
}

impl<'blobs, Blobs> VisionReplayService<'blobs, Blobs>
where
    Blobs: ArtifactStore + ?Sized,
    Blobs::Error: std::fmt::Debug,
{
    pub fn new(blobs: &'blobs Blobs) -> Self {
        Self { blobs }
    }

    /// Reads only existing Session and Blob facts. It performs no Provider
    /// request, Event append, Blob write or route recomputation.
    pub fn replay_secondary(
        &self,
        session: &SessionState,
        route_event_id: &EventId,
        source_artifact_id: &ArtifactId,
    ) -> Result<VisionObservation, VisionReplayError> {
        let route = session
            .vision_routes()
            .iter()
            .find(|route| &route.route_event_id == route_event_id)
            .ok_or(VisionReplayError::RouteMissing)?;
        if route.route != VisionRoute::SecondaryTool {
            return Err(VisionReplayError::NotSecondaryRoute);
        }
        if !route.source_artifact_ids.contains(source_artifact_id) {
            return Err(VisionReplayError::SourceArtifactNotRouted);
        }
        let observation = session
            .vision_observations()
            .iter()
            .find(|observation| {
                &observation.route_event_id == route_event_id
                    && &observation.source_artifact_id == source_artifact_id
            })
            .cloned();
        let Some(observation) = observation else {
            if session.vision_failures().iter().any(|failure| {
                &failure.route_event_id == route_event_id
                    && &failure.source_artifact_id == source_artifact_id
            }) {
                return Err(VisionReplayError::RecordedFailure);
            }
            return Err(VisionReplayError::ObservationMissing);
        };
        if let Some(geometry) = &observation.image_geometry {
            let audit = session
                .request_audits()
                .iter()
                .find(|audit| audit.audit_event_id() == &observation.request_audit_event_id)
                .ok_or(VisionReplayError::GeometryAuditMismatch)?;
            let input = audit
                .reconstruct(|id| self.blobs.read_blob(id))
                .map_err(|error| VisionReplayError::GeometryAuditRead {
                    message: format!("{error:?}"),
                })?;
            let metadata = ContentItem::Text {
                role: MessageRole::User,
                text: geometry.context_text(&observation.source_artifact_id),
            };
            // The fixed scoped request is task, image, runtime geometry. A
            // user task that happens to contain geometry JSON is not proof.
            if !matches!(input.content_items.as_slice(),
                [ContentItem::Text { role: MessageRole::User, .. },
                 ContentItem::ImageArtifact { role: MessageRole::User, artifact_id }, frame]
                if artifact_id == &geometry.delivery_artifact_id && frame == &metadata)
            {
                return Err(VisionReplayError::GeometryAuditMismatch);
            }
        }
        let bytes = self
            .blobs
            .read_blob(&observation.raw_observation_blob_id)
            .map_err(|error| VisionReplayError::ObservationBlobRead {
                message: format!("{error:?}"),
            })?;
        let raw = std::str::from_utf8(&bytes).map_err(|error| {
            VisionReplayError::ObservationBlobInvalid {
                message: error.to_string(),
            }
        })?;
        let reply =
            decode_model_reply(raw).map_err(|error| VisionReplayError::ObservationBlobInvalid {
                message: error.to_string(),
            })?;
        if !reply.matches_observation(&observation) {
            return Err(VisionReplayError::ObservationBlobMismatch);
        }
        Ok(observation)
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum VisionReplayError {
    #[error("Vision image geometry does not match its audited image request")]
    GeometryAuditMismatch,
    #[error("Vision geometry RequestAudit cannot be reconstructed: {message}")]
    GeometryAuditRead { message: String },
    #[error("the requested Vision route is not present in the replayed Session")]
    RouteMissing,
    #[error("the replayed route is not a secondary_tool route")]
    NotSecondaryRoute,
    #[error("the source Artifact is not part of the replayed Vision route")]
    SourceArtifactNotRouted,
    #[error("the replayed Session has no VisionObservation for the selected route and Artifact")]
    ObservationMissing,
    #[error("the replayed Vision route has a recorded failed analysis")]
    RecordedFailure,
    #[error("the Vision observation Blob cannot be read: {message}")]
    ObservationBlobRead { message: String },
    #[error("the Vision observation Blob is invalid: {message}")]
    ObservationBlobInvalid { message: String },
    #[error("the Vision observation Blob content does not match the durable observation fact")]
    ObservationBlobMismatch,
}
