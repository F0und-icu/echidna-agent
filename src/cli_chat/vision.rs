//! Fixed production CLI composition for queued local images.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::adapters::project_event_store::ProjectEventStream;
use crate::adapters::research_projection::load_research_state;
use crate::adapters::tools::{ToolRegistry, ViewImageToolInput};
use crate::app::agent_loop::{AgentLoopError, InitialImagePrimaryContext};
use crate::app::cancellation::AgentCancellation;
use crate::app::session::SessionProjection;
use crate::app::vision::{
    VisionAnalysisInput, VisionDirectRequest, VisionEventStamp, VisionSecondaryRequest,
    VisionSessionService,
};
use crate::domain::context::{ProviderSessionId, ProviderSessionRef, ProviderSessionRole};
use crate::domain::events::StreamId;
use crate::domain::profile::{VisionRoute, VisionRouteDecision};
use crate::domain::request_audit::{RequestAuditBinding, RequestAuditRunContext};
use crate::domain::tools::{ExecutionId, ToolCall, ToolCallId, ToolName, ToolResultStatus};
use crate::domain::vision::{
    VISION_FACT_SCHEMA_VERSION, ViewImageSelection, VisionObservationId, VisionRouteRecord,
};
use crate::ports::EventSource;
use crate::ports::tool::ToolExecutor;
use crate::provider_composition::ProviderComposition;

use super::{ChatError, DurableChatSession, event_id, occurred_at};

const CLI_IMAGE_TOOLCHAIN_FINGERPRINT: &str = "cyber-agent:view-image:v1";
static NEXT_CLI_IMAGE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug)]
pub(super) struct PendingImage {
    selection: ViewImageSelection,
    target_snapshot_id: Option<crate::domain::research::TargetSnapshotId>,
}

impl PendingImage {
    pub(super) fn from_selection(selection: ViewImageSelection) -> Self {
        Self {
            selection,
            target_snapshot_id: None,
        }
    }

    pub(super) fn delivery_artifact_id(&self) -> &crate::domain::artifacts::ArtifactId {
        &self.selection.delivery_artifact_id
    }

    pub(super) fn target_snapshot_id(&self) -> Option<&crate::domain::research::TargetSnapshotId> {
        self.target_snapshot_id.as_ref()
    }
}

pub(super) async fn ingest_image(
    session: &DurableChatSession,
    tools: &ToolRegistry,
    composition: &ProviderComposition,
    path: &str,
) -> Result<PendingImage, ChatError> {
    let path = path.trim();
    if path.is_empty() {
        return Err(ChatError::ImagePathRequired);
    }
    require_image_route(composition)?;

    let research = load_research_state(&session.project).map_err(ChatError::ResearchProjection)?;
    let target_snapshot_id = research
        .latest_target_snapshot_id()
        .cloned()
        .ok_or(ChatError::ImageTargetSnapshotRequired)?;
    let operation_id = image_operation_id(&session.session_id);
    let occurred_at = occurred_at();
    let input = ViewImageToolInput::new(
        format!("{operation_id}:artifact"),
        occurred_at.as_str(),
        format!("artifact:{operation_id}"),
        Path::new(path).display().to_string(),
        target_snapshot_id.as_str(),
        CLI_IMAGE_TOOLCHAIN_FINGERPRINT,
    );
    let call = ToolCall::new(
        ToolCallId::new(format!("call:{operation_id}")),
        ToolName::new("view_image"),
        input.into_value(),
    );
    let plan = tools
        .plan(ExecutionId::new(format!("execution:{operation_id}")), call)
        .map_err(|error| ChatError::ImageIntake {
            message: error.to_string(),
        })?;
    let outcome = tools.execute(plan).await?;
    session.record_execution(&outcome)?;
    if outcome.result.status != ToolResultStatus::Succeeded {
        return Err(ChatError::ImageIntake {
            message: outcome.result.output.to_string(),
        });
    }
    let selection = ViewImageSelection::from_tool_result(&outcome.result)
        .map_err(|error| ChatError::ImageIntake {
            message: error.to_string(),
        })?
        .ok_or_else(|| ChatError::ImageIntake {
            message: "view_image completed without a typed image selection".to_owned(),
        })?;
    Ok(PendingImage {
        selection,
        target_snapshot_id: Some(target_snapshot_id),
    })
}

pub(super) async fn prepare_primary_contexts(
    session: &DurableChatSession,
    composition: &ProviderComposition,
    pending: Vec<PendingImage>,
    task: &str,
    cancellation: AgentCancellation,
    request_budget: std::sync::Arc<crate::app::model_request_budget::ModelRequestBudget>,
) -> Result<Vec<InitialImagePrimaryContext>, ChatError> {
    let mut contexts = Vec::with_capacity(pending.len());
    for image in pending {
        contexts.push(
            prepare_primary_context(
                session,
                composition,
                image,
                task,
                cancellation.clone(),
                std::sync::Arc::clone(&request_budget),
            )
            .await?,
        );
    }
    Ok(contexts)
}

pub(super) async fn prepare_primary_context(
    session: &DurableChatSession,
    composition: &ProviderComposition,
    pending: PendingImage,
    task: &str,
    cancellation: AgentCancellation,
    request_budget: std::sync::Arc<crate::app::model_request_budget::ModelRequestBudget>,
) -> Result<InitialImagePrimaryContext, ChatError> {
    let decision = require_image_route(composition)?;
    let profile = composition.agent_profile();
    let mut writer = session.project.open_writer()?;
    let mut store = writer.open_session(session.session_id.clone(), None)?;
    let stream_id = ProjectEventStream::Session(session.session_id.clone()).stream_id();
    let mut projection = SessionProjection::replay(EventSource::read_events(&store, &stream_id)?)?;
    let sequence = store.next_sequence()?;
    let route_event_id = event_id(
        session.project.project_id(),
        &session.session_id,
        sequence.get(),
        "vision-route",
    );
    let binding = RequestAuditBinding::new(session.project.project_id().clone(), stream_id)?;
    let route = VisionRouteRecord {
        schema_version: VISION_FACT_SCHEMA_VERSION,
        route_event_id: route_event_id.clone(),
        mode: profile.vision_mode(),
        route: decision.route,
        reason: decision.reason,
        source_artifact_ids: vec![pending.selection.source_artifact_id.clone()],
        primary_profile: profile.primary().profile().clone(),
        vision_profile: profile.vision().map(|vision| vision.profile().clone()),
        primary_capability_hash: profile.primary().capabilities().semantic_hash(),
        vision_capability_hash: profile
            .vision()
            .map(|vision| vision.capabilities().semantic_hash()),
    };
    let service = VisionSessionService::new(&session.artifacts, binding.clone())
        .with_request_budget(request_budget);

    match decision.route {
        VisionRoute::PrimaryDirect => service
            .prepare_direct(
                VisionDirectRequest {
                    route,
                    route_occurred_at: occurred_at(),
                    source_artifact_id: pending.selection.source_artifact_id,
                    delivery_artifact_id: pending.selection.delivery_artifact_id,
                },
                &mut projection,
                &mut store,
            )
            .map_err(vision_error),
        VisionRoute::SecondaryTool => {
            let image_geometry = crate::adapters::image::resolve_vision_geometry(
                &session.project,
                &pending.selection,
            )
            .map_err(|error| ChatError::ImageIntake {
                message: error.to_string(),
            })?;
            let vision_profile = profile
                .vision()
                .ok_or_else(|| ChatError::VisionComposition {
                    message: "secondary_tool resolved without a Vision profile".to_owned(),
                })?;
            let vision_provider =
                composition
                    .vision_provider()
                    .ok_or_else(|| ChatError::VisionComposition {
                        message: "secondary_tool resolved without a Vision Provider".to_owned(),
                    })?;
            let adapter_version =
                composition
                    .vision_request_adapter_version()
                    .ok_or_else(|| ChatError::VisionComposition {
                        message: "secondary_tool resolved without a Vision Adapter identity"
                            .to_owned(),
                    })?;
            let vision_generation = sequence.get();
            let prepared = service
                .prepare_secondary(
                    VisionSecondaryRequest {
                        route,
                        route_occurred_at: occurred_at(),
                        analysis: VisionAnalysisInput {
                            observation_id: VisionObservationId::new(format!(
                                "observation:{}:{}",
                                session.session_id.as_str(),
                                sequence.get()
                            )),
                            route_event_id,
                            source_artifact_id: pending.selection.source_artifact_id,
                            delivery_artifact_id: pending.selection.delivery_artifact_id,
                            derived_artifact_ids: pending.selection.derived_artifact_ids,
                            image_geometry,
                            task: task.to_owned(),
                            model: vision_profile.model().clone(),
                            provider_profile: vision_profile.profile().clone(),
                            capability_hash: vision_profile.capabilities().semantic_hash(),
                            provider_session: ProviderSessionRef {
                                id: ProviderSessionId::new(format!(
                                    "vision:{}:{}",
                                    session.session_id.as_str(),
                                    vision_generation
                                )),
                                role: ProviderSessionRole::Vision,
                                generation: vision_generation,
                            },
                        },
                    },
                    &mut projection,
                    &mut store,
                )
                .map_err(vision_error)?;
            drop(store);
            drop(writer);

            let terminal = service
                .analyze_prepared(
                    vision_provider,
                    prepared,
                    &RequestAuditRunContext::new(binding, adapter_version.clone()),
                    cancellation,
                    |attempt| session.record_request_audit_manifest(attempt, None),
                )
                .await
                .map_err(vision_error)?;

            let mut writer = session.project.open_writer()?;
            let mut store = writer.open_session(session.session_id.clone(), None)?;
            let stream_id = ProjectEventStream::Session(session.session_id.clone()).stream_id();
            let mut projection =
                SessionProjection::replay(EventSource::read_events(&store, &stream_id)?)?;
            let sequence = store.next_sequence()?;
            let terminal_stamp = VisionEventStamp::new(
                event_id(
                    session.project.project_id(),
                    &session.session_id,
                    sequence.get(),
                    "vision-observation",
                ),
                occurred_at(),
            );
            let outcome = service
                .commit_secondary_terminal(terminal, terminal_stamp, &mut projection, &mut store)
                .map_err(vision_error)?;
            Ok(outcome.primary_context)
        }
        VisionRoute::NoImage | VisionRoute::Rejected => Err(ChatError::VisionComposition {
            message: "an image Turn requires one accepted Direct or Secondary route".to_owned(),
        }),
    }
}

fn require_image_route(
    composition: &ProviderComposition,
) -> Result<VisionRouteDecision, ChatError> {
    let decision = composition.agent_profile().resolve_vision_route(true);
    match decision.route {
        VisionRoute::PrimaryDirect | VisionRoute::SecondaryTool => Ok(decision),
        VisionRoute::NoImage | VisionRoute::Rejected => Err(ChatError::VisionUnavailable {
            reason: decision
                .reason
                .map_or("NO_IMAGE_ROUTE", |reason| reason.as_str()),
        }),
    }
}

fn vision_error(error: crate::app::vision::VisionSessionError) -> ChatError {
    let error = match error.into_recoverable_stop() {
        Ok(error) => return ChatError::Agent(error),
        Err(error) => error,
    };
    match error {
        crate::app::vision::VisionSessionError::Analysis(
            crate::app::vision::VisionAnalysisError::Cancelled,
        ) => ChatError::VisionCancelled,
        crate::app::vision::VisionSessionError::AnalysisFailed { failure }
            if failure.error_code == "cancelled" =>
        {
            ChatError::VisionCancelled
        }
        error => ChatError::VisionComposition {
            message: error.to_string(),
        },
    }
}

fn image_operation_id(session_id: &StreamId) -> String {
    let sequence = NEXT_CLI_IMAGE.fetch_add(1, Ordering::Relaxed);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!(
        "cli-image:{}:{}-{}-{sequence}",
        session_id.as_str(),
        timestamp.as_secs(),
        timestamp.subsec_nanos()
    )
}

pub(super) fn follow_up_error(error: ChatError) -> AgentLoopError {
    match error {
        ChatError::Agent(error) => error,
        ChatError::VisionCancelled => AgentLoopError::ViewImageFollowUpCancelled,
        error => AgentLoopError::InitialImageContext {
            message: error.to_string(),
        },
    }
}
