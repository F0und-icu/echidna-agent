//! Durable route, audit and terminal composition for scoped Vision work.

use thiserror::Error;

use super::analysis::{VisionAnalysisError, VisionAnalysisInput, VisionAnalysisService};
use crate::app::agent_loop::{AgentLoopError, InitialImagePrimaryContext};
use crate::app::cancellation::AgentCancellation;
use crate::app::request_audit::{RequestAuditService, RequestAuditServiceError};
use crate::app::session::SessionProjection;
use crate::domain::artifacts::ArtifactId;
use crate::domain::events::{EventId, OccurredAt};
use crate::domain::profile::VisionRoute;
use crate::domain::request_audit::{RequestAuditBinding, RequestAuditRunContext};
use crate::domain::session::SessionEvent;
use crate::domain::vision::{VisionAnalysisFailure, VisionObservation, VisionRouteRecord};
use crate::ports::EventSink;
use crate::ports::artifact::ArtifactStore;
use crate::ports::model::ModelProvider;

#[derive(Clone, Debug)]
pub struct VisionEventStamp {
    pub event_id: EventId,
    pub occurred_at: OccurredAt,
}

impl VisionEventStamp {
    pub fn new(event_id: EventId, occurred_at: OccurredAt) -> Self {
        Self {
            event_id,
            occurred_at,
        }
    }
}

#[derive(Clone, Debug)]
pub struct VisionDirectRequest {
    pub route: VisionRouteRecord,
    pub route_occurred_at: OccurredAt,
    pub source_artifact_id: ArtifactId,
    pub delivery_artifact_id: ArtifactId,
}

#[derive(Clone, Debug)]
pub struct VisionSecondaryRequest {
    pub route: VisionRouteRecord,
    pub route_occurred_at: OccurredAt,
    pub analysis: VisionAnalysisInput,
    pub audit: VisionEventStamp,
    pub terminal: VisionEventStamp,
}

pub struct VisionSessionService<'blobs, Blobs: ?Sized> {
    blobs: &'blobs Blobs,
    binding: RequestAuditBinding,
}

impl<'blobs, Blobs> VisionSessionService<'blobs, Blobs>
where
    Blobs: ArtifactStore + ?Sized,
    Blobs::Error: std::fmt::Debug,
{
    pub fn new(blobs: &'blobs Blobs, binding: RequestAuditBinding) -> Self {
        Self { blobs, binding }
    }

    /// Commits one Direct route and returns the only token that can insert its
    /// selected image into the audited Primary loop.
    pub fn prepare_direct<S>(
        &self,
        request: VisionDirectRequest,
        projection: &mut SessionProjection,
        sink: &mut S,
    ) -> Result<InitialImagePrimaryContext, VisionSessionError>
    where
        S: EventSink<serde_json::Value>,
        S::Error: std::fmt::Debug,
    {
        self.require_session(projection)?;
        request.route.validate().map_err(route_error)?;
        if request.route.route != VisionRoute::PrimaryDirect {
            return Err(VisionSessionError::Route {
                message: "Direct image preparation requires a primary_direct route".to_owned(),
            });
        }
        if !request
            .route
            .source_artifact_ids
            .contains(&request.source_artifact_id)
        {
            return Err(VisionSessionError::Route {
                message: "the Direct source Artifact is not part of the selected route".to_owned(),
            });
        }
        if request.delivery_artifact_id.as_str().trim().is_empty() {
            return Err(VisionSessionError::RequiredInput {
                field: "delivery_artifact_id",
            });
        }
        let primary_profile = request.route.primary_profile.clone();
        let primary_capability_hash = request.route.primary_capability_hash.clone();
        let route_event_id = request.route.route_event_id.clone();
        let source_artifact_id = request.source_artifact_id.clone();
        self.record_route(request.route, request.route_occurred_at, projection, sink)?;
        Ok(InitialImagePrimaryContext::direct(
            source_artifact_id,
            request.delivery_artifact_id,
            route_event_id,
            primary_profile,
            primary_capability_hash,
        )?)
    }

    /// Commits a new Secondary route, then exactly one RequestAudit and one
    /// Observation/Failure terminal. Each targeted re-analysis uses a new
    /// request value, so old observations are never overwritten.
    pub async fn analyze_image<VisionProvider, S>(
        &self,
        provider: &VisionProvider,
        request: VisionSecondaryRequest,
        audit_context: &RequestAuditRunContext,
        cancellation: AgentCancellation,
        projection: &mut SessionProjection,
        sink: &mut S,
    ) -> Result<VisionSessionOutcome, VisionSessionError>
    where
        VisionProvider: ModelProvider + ?Sized,
        S: EventSink<serde_json::Value>,
        S::Error: std::fmt::Debug,
    {
        self.require_session(projection)?;
        validate_secondary_request(&request)?;
        self.require_scoped_vision_session(projection, &request.analysis.provider_session)?;
        let primary_profile = request.route.primary_profile.clone();
        let primary_capability_hash = request.route.primary_capability_hash.clone();
        self.record_route(request.route, request.route_occurred_at, projection, sink)?;

        let audit_service = RequestAuditService::new(self.blobs, self.binding.clone());
        let mut audit_id = Some(request.audit.event_id);
        let analysis = VisionAnalysisService::new(provider, self.blobs)
            .analyze_image(request.analysis, audit_context, cancellation, |attempt| {
                let event_id = audit_id.take().ok_or_else(|| {
                    "Vision analysis attempted more than one RequestAudit commit".to_owned()
                })?;
                audit_service
                    .record(
                        attempt,
                        event_id,
                        request.audit.occurred_at.clone(),
                        projection,
                        sink,
                    )
                    .map_err(request_audit_message)
            })
            .await;

        match analysis {
            Ok(output) => {
                let observation = output.observation;
                let observation_event_id = request.terminal.event_id.clone();
                self.append_terminal(
                    SessionEvent::VisionObservationRecorded {
                        observation: observation.clone(),
                    },
                    request.terminal,
                    projection,
                    sink,
                )?;
                let primary_context = InitialImagePrimaryContext::secondary(
                    &observation,
                    observation_event_id,
                    primary_profile,
                    primary_capability_hash,
                )?;
                Ok(VisionSessionOutcome {
                    observation,
                    primary_context,
                })
            }
            Err(VisionAnalysisError::AttemptFailed { failure }) => {
                let failure = *failure;
                self.append_terminal(
                    SessionEvent::VisionAnalysisFailed {
                        failure: failure.clone(),
                    },
                    request.terminal,
                    projection,
                    sink,
                )?;
                Err(VisionSessionError::AnalysisFailed {
                    failure: Box::new(failure),
                })
            }
            Err(error) => Err(VisionSessionError::Analysis(error)),
        }
    }

    fn record_route<S>(
        &self,
        route: VisionRouteRecord,
        occurred_at: OccurredAt,
        projection: &mut SessionProjection,
        sink: &mut S,
    ) -> Result<(), VisionSessionError>
    where
        S: EventSink<serde_json::Value>,
        S::Error: std::fmt::Debug,
    {
        let sequence = projection.next_event_sequence().map_err(session_error)?;
        let event = SessionEvent::VisionRouteDecided {
            route: route.clone(),
        }
        .into_envelope(
            route.route_event_id,
            self.binding.session_stream_id().clone(),
            sequence,
            occurred_at,
        )
        .map_err(session_error)?;
        projection
            .append(sink, event)
            .map_err(|error| VisionSessionError::Session {
                message: format!("{error:?}"),
            })?;
        Ok(())
    }

    fn append_terminal<S>(
        &self,
        event: SessionEvent,
        stamp: VisionEventStamp,
        projection: &mut SessionProjection,
        sink: &mut S,
    ) -> Result<(), VisionSessionError>
    where
        S: EventSink<serde_json::Value>,
        S::Error: std::fmt::Debug,
    {
        let sequence = projection.next_event_sequence().map_err(session_error)?;
        let envelope = event
            .into_envelope(
                stamp.event_id,
                self.binding.session_stream_id().clone(),
                sequence,
                stamp.occurred_at,
            )
            .map_err(session_error)?;
        projection
            .append(sink, envelope)
            .map_err(|error| VisionSessionError::Session {
                message: format!("{error:?}"),
            })?;
        Ok(())
    }

    fn require_session(&self, projection: &SessionProjection) -> Result<(), VisionSessionError> {
        if projection.state().stream_id() != Some(self.binding.session_stream_id()) {
            return Err(VisionSessionError::Session {
                message: "Vision Session binding does not match the projection".to_owned(),
            });
        }
        Ok(())
    }

    fn require_scoped_vision_session(
        &self,
        projection: &SessionProjection,
        provider_session: &crate::domain::context::ProviderSessionRef,
    ) -> Result<(), VisionSessionError> {
        if projection
            .state()
            .primary_provider_session()
            .is_some_and(|primary| primary.id == provider_session.id)
        {
            return Err(VisionSessionError::VisionSessionUnavailable {
                message: "Vision ProviderSession collides with the active Primary".to_owned(),
            });
        }
        let already_used = projection
            .state()
            .vision_observations()
            .iter()
            .any(|observation| observation.provider_session.id == provider_session.id)
            || projection
                .state()
                .vision_failures()
                .iter()
                .any(|failure| failure.provider_session.id == provider_session.id);
        if already_used {
            return Err(VisionSessionError::VisionSessionUnavailable {
                message: "Vision ProviderSession was already used by another scoped request"
                    .to_owned(),
            });
        }
        Ok(())
    }
}

fn validate_secondary_request(request: &VisionSecondaryRequest) -> Result<(), VisionSessionError> {
    request.route.validate().map_err(route_error)?;
    if request.route.route != VisionRoute::SecondaryTool {
        return Err(VisionSessionError::Route {
            message: "Vision analysis requires a secondary_tool route".to_owned(),
        });
    }
    if request.route.route_event_id != request.analysis.route_event_id {
        return Err(VisionSessionError::Route {
            message: "Vision analysis route identity does not match the durable route".to_owned(),
        });
    }
    if !request
        .route
        .source_artifact_ids
        .contains(&request.analysis.source_artifact_id)
    {
        return Err(VisionSessionError::Route {
            message: "the Vision source Artifact is not part of the selected route".to_owned(),
        });
    }
    if request.route.vision_profile.as_ref() != Some(&request.analysis.provider_profile)
        || request.route.vision_capability_hash.as_deref()
            != Some(request.analysis.capability_hash.as_str())
    {
        return Err(VisionSessionError::Route {
            message: "Vision analysis provider identity does not match the selected route"
                .to_owned(),
        });
    }
    let route_id = &request.route.route_event_id;
    if route_id == &request.audit.event_id
        || route_id == &request.terminal.event_id
        || request.audit.event_id == request.terminal.event_id
    {
        return Err(VisionSessionError::EventIdentityCollision);
    }
    Ok(())
}

fn route_error(error: impl ToString) -> VisionSessionError {
    VisionSessionError::Route {
        message: error.to_string(),
    }
}

fn session_error(error: impl ToString) -> VisionSessionError {
    VisionSessionError::Session {
        message: error.to_string(),
    }
}

fn request_audit_message<BlobError, SinkError>(
    error: RequestAuditServiceError<BlobError, SinkError>,
) -> String
where
    BlobError: std::fmt::Debug,
    SinkError: std::fmt::Debug,
{
    error.into_agent_loop_error().to_string()
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VisionSessionOutcome {
    pub observation: VisionObservation,
    pub primary_context: InitialImagePrimaryContext,
}

#[derive(Debug, Error)]
pub enum VisionSessionError {
    #[error("Vision input `{field}` must not be empty")]
    RequiredInput { field: &'static str },
    #[error("Vision route is invalid: {message}")]
    Route { message: String },
    #[error("Vision route, audit and terminal Event IDs must be distinct")]
    EventIdentityCollision,
    #[error("Vision scoped ProviderSession is unavailable: {message}")]
    VisionSessionUnavailable { message: String },
    #[error("Vision Session composition failed: {message}")]
    Session { message: String },
    #[error(transparent)]
    Analysis(#[from] VisionAnalysisError),
    #[error(
        "Vision analysis failed after its durable RequestAudit: {}",
        failure.message
    )]
    AnalysisFailed { failure: Box<VisionAnalysisFailure> },
    #[error(transparent)]
    AgentLoop(#[from] AgentLoopError),
}
