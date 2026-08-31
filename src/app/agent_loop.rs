//! Bounded single-Agent Tool Call -> Tool Result -> full-resample loop.

mod compaction;
mod context_state;
mod request_audit;

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use thiserror::Error;

use crate::domain::context::{
    AuthorityLevel, BaselineAssessment, BaselineInvalidationReason, ContextDeliveryMap,
    ContextError, ContextOrigin, ContextPlan, ContextPlanner, ContextProvenance, ContextReferences,
    ContextSlot, FreshnessIdentity, FullLogicalManifest, InternalRequestMetadata, ModelIdentity,
    ModelProfile, ProviderSession, ProviderSessionId, ProviderSessionRole, RetentionClass,
    SemanticClass, StepSnapshot, TurnContext, TurnId,
};
use crate::domain::domain_profile::DomainProfile;
use crate::domain::events::{EventId, StreamId};
use crate::domain::history::{ConversationHistory, HistoryError};
use crate::domain::model::{ContentItem, MessageRole, ModelInputIR};
use crate::domain::profile::{AgentModelProfile, AgentModelProfileError};
use crate::domain::request_audit::{
    RequestAuditAttempt, RequestAuditError, RequestAuditRunContext,
};
use crate::domain::research_control::{PromptComposition, ResearchControlError, ResearchPhase};
use crate::domain::sampling::{SamplingStepPlan, TurnConfigurationSnapshot};
use crate::domain::session::{
    CompactionValidationContext, SessionError, SessionState, SessionSummary, SessionSummaryDraft,
};
use crate::domain::tools::{
    DeferredActivationSet, ExecutionId, IdempotencyKey, ResolvedToolPlan, ToolCall,
    ToolExecutionOutcome, ToolExposurePlan, ToolFailure, ToolFailureRetryability, ToolResult,
};
use crate::domain::world_state::{ProjectSnapshotContext, ResearchWorldState, WorldStateError};
use crate::domain::{
    ArtifactId,
    vision::{ViewImageSelection, VisionObservation},
};
use crate::ports::context::ContextFragmentCache;
use crate::ports::model::{
    ModelProvider, ModelProviderError, ModelRequest, ModelResponseCompletion, ProviderResponseId,
};
use crate::ports::tool::{ToolExecutor, ToolExecutorError};

use super::cancellation::AgentCancellation;
use super::context_compilation::{ContextCacheSummary, ContextCompilationError};
use super::model_sample::ModelSample;
use super::model_streaming::{ModelStreamOutcome, ModelStreamingError, ModelStreamingService};
use super::tool_scheduler::execute_batch;
use compaction::AppliedCompaction;
use request_audit::RequestAuditStage;

const MAX_MODEL_REQUESTS_PER_TURN: usize = 32;
static NEXT_PRIMARY_RUNTIME_SESSION: AtomicU64 = AtomicU64::new(1);

/// Opaque proof that the Vision Session service durably prepared exactly one
/// Direct image or one Secondary observation for the selected Primary.
/// Callers cannot construct this token without the fixed App composition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InitialImagePrimaryContext {
    content_item: ContentItem,
    provenance: ContextProvenance,
    primary_profile: ModelProfile,
    primary_capability_hash: String,
}

impl InitialImagePrimaryContext {
    pub(crate) fn direct(
        source_artifact_id: ArtifactId,
        delivery_artifact_id: ArtifactId,
        route_event_id: EventId,
        primary_profile: ModelProfile,
        primary_capability_hash: String,
    ) -> Result<Self, AgentLoopError> {
        let mut artifact_refs = vec![source_artifact_id];
        if !artifact_refs.contains(&delivery_artifact_id) {
            artifact_refs.push(delivery_artifact_id.clone());
        }
        let provenance = ContextProvenance::new(
            ContextSlot::TurnUpdates,
            ContextOrigin::ArtifactProjection,
            SemanticClass::AuthoritativeReference,
            AuthorityLevel::FactReference,
            RetentionClass::Recent,
            FreshnessIdentity::new(delivery_artifact_id.as_str())?
                .with_producer_version("vision-route:1"),
            ContextReferences::new(Vec::new(), artifact_refs, Vec::new(), vec![route_event_id])?,
        )?;
        Ok(Self {
            content_item: ContentItem::ImageArtifact {
                role: MessageRole::User,
                artifact_id: delivery_artifact_id,
            },
            provenance,
            primary_profile,
            primary_capability_hash,
        })
    }

    pub(crate) fn secondary(
        observation: &VisionObservation,
        observation_event_id: EventId,
        primary_profile: ModelProfile,
        primary_capability_hash: String,
    ) -> Result<Self, AgentLoopError> {
        let text = observation.primary_context_text().map_err(|error| {
            AgentLoopError::InitialImageContext {
                message: error.to_string(),
            }
        })?;
        let mut artifact_refs = vec![observation.source_artifact_id.clone()];
        artifact_refs.extend(observation.derived_artifact_ids.iter().cloned());
        let provenance = ContextProvenance::new(
            ContextSlot::TurnUpdates,
            ContextOrigin::ModelOutput,
            SemanticClass::ModelObservation,
            AuthorityLevel::ModelObservation,
            RetentionClass::Recent,
            FreshnessIdentity::new(&observation.task_hash)?
                .with_producer_version("vision-observation:1"),
            ContextReferences::new(
                Vec::new(),
                artifact_refs,
                Vec::new(),
                vec![
                    observation.route_event_id.clone(),
                    observation.request_audit_event_id.clone(),
                    observation_event_id,
                ],
            )?,
        )?;
        Ok(Self {
            content_item: ContentItem::Text {
                role: MessageRole::User,
                text,
            },
            provenance,
            primary_profile,
            primary_capability_hash,
        })
    }

    fn into_content_item(
        self,
        primary_profile: &ModelProfile,
        primary_capability_hash: &str,
    ) -> Result<(ContentItem, ContextProvenance), AgentLoopError> {
        if &self.primary_profile != primary_profile
            || self.primary_capability_hash != primary_capability_hash
        {
            return Err(AgentLoopError::InitialImagePrimaryIdentityMismatch);
        }
        Ok((self.content_item, self.provenance))
    }
}

/// The one fixed post-`view_image` stage. This is deliberately a typed
/// boundary rather than a general Hook registry: the stage receives only a
/// validated local-image selection and must return exactly one mutually
/// exclusive Primary context (direct Artifact or secondary observation).
pub type ViewImageFollowUp<'a> = dyn FnMut(ViewImageSelection) -> ViewImageFollowUpFuture<'a> + 'a;

/// The asynchronous result shape for the fixed [`ViewImageFollowUp`] stage.
pub type ViewImageFollowUpFuture<'a> =
    Pin<Box<dyn Future<Output = Result<InitialImagePrimaryContext, AgentLoopError>> + 'a>>;

/// The one fixed stage that makes a completed Tool outcome durable before the
/// next Provider sample. It is deliberately scoped to Receipt persistence;
/// it does not own Tool execution, Cache policy, History pairing or a general
/// Hook mechanism.
pub type ToolExecutionReceiptStage<'a> =
    dyn FnMut(&ToolExecutionOutcome) -> Result<(), AgentLoopError> + 'a;

/// The one fixed stage for user-visible assistant text while a response is
/// streaming. Reasoning is intentionally not part of this stage and remains
/// outside Conversation History and CLI output.
pub type AssistantTextDeltaStage<'a> = dyn FnMut(&str) -> Result<(), AgentLoopError> + 'a;

/// The fixed callback bundle used only by production CLI Vision composition.
/// Grouping these existing stages keeps the entry point bounded without
/// introducing a registry or another lifecycle owner.
pub(crate) struct CliVisionStages<'a> {
    follow_up: &'a mut ViewImageFollowUp<'a>,
    text_delta: &'a mut AssistantTextDeltaStage<'a>,
    record_execution: &'a mut ToolExecutionReceiptStage<'a>,
}

impl<'a> CliVisionStages<'a> {
    pub(crate) fn new(
        follow_up: &'a mut ViewImageFollowUp<'a>,
        text_delta: &'a mut AssistantTextDeltaStage<'a>,
        record_execution: &'a mut ToolExecutionReceiptStage<'a>,
    ) -> Self {
        Self {
            follow_up,
            text_delta,
            record_execution,
        }
    }
}

struct StreamingStages<'a> {
    text_delta: Option<&'a mut AssistantTextDeltaStage<'a>>,
    record_execution: Option<&'a mut ToolExecutionReceiptStage<'a>>,
}

impl StreamingStages<'_> {
    fn none() -> Self {
        Self {
            text_delta: None,
            record_execution: None,
        }
    }
}

pub struct AgentLoop<'provider, 'tools, Provider: ?Sized, Tools: ?Sized> {
    provider: &'provider Provider,
    tools: &'tools Tools,
    prompt_composition: PromptComposition,
    pending_prompt_composition: Option<PromptComposition>,
    world_state: Option<ResearchWorldState>,
    project_snapshot: Option<ProjectSnapshotContext>,
    session_summary: Option<SessionSummary>,
    pending_session_summary: Option<SessionSummaryDraft>,
    history: ConversationHistory,
    model: ModelIdentity,
    profile: ModelProfile,
    provider_session: ProviderSession,
    continuation: Option<CommittedContinuation>,
    completed_idempotency: BTreeMap<IdempotencyKey, ToolExecutionOutcome>,
    context_planner: ContextPlanner,
    fragment_cache: Option<Arc<dyn ContextFragmentCache>>,
    committed_context_manifest: Option<FullLogicalManifest>,
    last_context_plan: Option<ContextPlan>,
    last_successful_context: Option<ContextInspection>,
    last_sampling_step_plan: Option<SamplingStepPlan>,
    next_turn_sequence: u64,
    last_baseline_assessment: Option<BaselineAssessment>,
    active_agent_model_profile: Option<AgentModelProfile>,
    pending_agent_model_profile: Option<AgentModelProfile>,
    active_tool_exposure_plan: ToolExposurePlan,
    committed_delivery_input: Option<ModelInputIR>,
    committed_delivery_map: Option<ContextDeliveryMap>,
    committed_history_len: usize,
    committed_world_state: Option<ResearchWorldState>,
    committed_project_snapshot: Option<ProjectSnapshotContext>,
    history_provenance: BTreeMap<usize, ContextProvenance>,
    bound_session_stream: Option<StreamId>,
    applied_compaction: Option<AppliedCompaction>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContextInspection {
    plan: ContextPlan,
    baseline_assessment: BaselineAssessment,
    cache_summary: ContextCacheSummary,
}

impl ContextInspection {
    pub fn plan(&self) -> &ContextPlan {
        &self.plan
    }

    pub fn baseline_assessment(&self) -> &BaselineAssessment {
        &self.baseline_assessment
    }

    pub const fn cache_summary(&self) -> ContextCacheSummary {
        self.cache_summary
    }
}

impl<'provider, 'tools, Provider, Tools> AgentLoop<'provider, 'tools, Provider, Tools>
where
    Provider: ModelProvider + ?Sized,
    Tools: ToolExecutor + ?Sized,
{
    pub fn new(provider: &'provider Provider, tools: &'tools Tools) -> Self {
        Self::with_primary_context(
            provider,
            tools,
            ModelIdentity::new("primary", "default"),
            ModelProfile::new("default", 0),
            ProviderSession::new(
                ProviderSessionId::new("primary:0"),
                ProviderSessionRole::Primary,
                0,
            ),
        )
        .expect("the built-in AgentLoop context must be primary")
    }

    /// Builds the one primary AgentLoop from an explicitly selected model,
    /// profile and session. The legacy [`Self::new`] constructor retains the
    /// existing M1a fixture context until CLI profile composition exists.
    pub fn with_primary_context(
        provider: &'provider Provider,
        tools: &'tools Tools,
        model: ModelIdentity,
        profile: ModelProfile,
        provider_session: ProviderSession,
    ) -> Result<Self, AgentLoopConfigurationError> {
        if provider_session.reference().role != ProviderSessionRole::Primary {
            return Err(AgentLoopConfigurationError::PrimarySessionRequired);
        }
        let prompt_composition = PromptComposition::current();
        let active_tool_exposure_plan = prompt_composition
            .tool_exposure()
            .restricted_to_catalog(&tools.catalog());
        Ok(Self {
            provider,
            tools,
            prompt_composition,
            pending_prompt_composition: None,
            world_state: None,
            project_snapshot: None,
            session_summary: None,
            pending_session_summary: None,
            history: ConversationHistory::default(),
            model,
            profile,
            provider_session,
            continuation: None,
            completed_idempotency: BTreeMap::new(),
            context_planner: ContextPlanner::current(),
            fragment_cache: None,
            committed_context_manifest: None,
            last_context_plan: None,
            last_successful_context: None,
            last_sampling_step_plan: None,
            next_turn_sequence: 0,
            last_baseline_assessment: None,
            active_agent_model_profile: None,
            pending_agent_model_profile: None,
            active_tool_exposure_plan,
            committed_delivery_input: None,
            committed_delivery_map: None,
            committed_history_len: 0,
            committed_world_state: None,
            committed_project_snapshot: None,
            history_provenance: BTreeMap::new(),
            bound_session_stream: None,
            applied_compaction: None,
        })
    }

    pub fn history(&self) -> &ConversationHistory {
        &self.history
    }

    /// Replaces volatile runtime History from a strictly replayed Session
    /// State, normalizes any interrupted Tool Calls, then resumes with a new
    /// Primary ProviderSession. The next Provider request is therefore Full.
    pub fn resume_from_session(
        &mut self,
        session: &SessionState,
        mut provider_session: ProviderSession,
        validation: Option<&CompactionValidationContext<'_>>,
    ) -> Result<(), AgentLoopConfigurationError> {
        if session.lifecycle() != crate::domain::session::SessionLifecycle::Active
            || !session.has_resumed()
        {
            return Err(AgentLoopConfigurationError::ActiveResumedSessionRequired);
        }
        let expected = session
            .primary_provider_session()
            .ok_or(AgentLoopConfigurationError::ActiveResumedSessionRequired)?;
        if expected != provider_session.reference() {
            return Err(
                AgentLoopConfigurationError::ReplayedSessionProviderMismatch {
                    expected: expected.clone(),
                    received: provider_session.reference().clone(),
                },
            );
        }
        let stream_id = session
            .stream_id()
            .cloned()
            .ok_or(AgentLoopConfigurationError::CompactionSessionStreamMissing)?;
        if let Some(compaction) = session.latest_compaction()
            && compaction.summary().requires_reference_validation()
        {
            let validation = validation
                .ok_or(AgentLoopConfigurationError::CompactionValidationContextRequired)?;
            compaction.validate_references(validation)?;
        }
        let normalized_items = session.normalized_history_items()?;
        let history = ConversationHistory::from_complete_content_items(&normalized_items)?;
        let applied_compaction = session
            .latest_compaction()
            .map(|compaction| AppliedCompaction::from_compaction(stream_id.clone(), compaction))
            .transpose()?;
        provider_session.reset(BaselineInvalidationReason::SessionResumed);
        self.history = history;
        self.history_provenance.clear();
        self.provider_session = provider_session;
        self.bound_session_stream = Some(stream_id);
        self.session_summary = session
            .latest_compaction()
            .map(|compaction| compaction.summary().clone());
        self.pending_session_summary = None;
        self.applied_compaction = applied_compaction;
        let composition = self
            .pending_prompt_composition
            .take()
            .unwrap_or_else(|| self.prompt_composition.clone())
            .with_research_phase(session.effective_research_phase());
        self.pending_prompt_composition = Some(composition);
        self.clear_committed_primary_delivery_state();
        self.last_sampling_step_plan = None;
        Ok(())
    }

    pub fn with_fragment_cache(mut self, cache: Arc<dyn ContextFragmentCache>) -> Self {
        self.fragment_cache = Some(cache);
        self
    }

    /// Sets a complete fixed prompt composition before the first Turn.
    pub fn with_prompt_composition(mut self, composition: PromptComposition) -> Self {
        self.pending_prompt_composition = Some(composition);
        self
    }

    /// Selects one fixed vulnerability-research domain before the first Turn.
    pub fn with_domain_profile(mut self, profile: DomainProfile) -> Self {
        self.queue_domain_profile(profile);
        self
    }

    /// Selects one fixed research objective before the first Turn.
    pub fn with_research_phase(mut self, phase: ResearchPhase) -> Self {
        self.queue_research_phase(phase);
        self
    }

    /// Queues a complete Prompt change for the next Turn boundary.
    pub fn queue_prompt_composition(&mut self, composition: PromptComposition) {
        self.pending_prompt_composition = Some(composition);
    }

    /// Queues a fixed vulnerability-research DomainProfile for the next Turn
    /// boundary while preserving Personal/Project and Phase composition.
    pub fn queue_domain_profile(&mut self, profile: DomainProfile) {
        let composition = self
            .pending_prompt_composition
            .clone()
            .unwrap_or_else(|| self.prompt_composition.clone());
        self.pending_prompt_composition = Some(composition.with_domain_profile(profile));
    }

    /// Queues an already-authorized ResearchPhase selection for the next Turn
    /// boundary. Durable Session append remains the caller's fact boundary.
    pub fn queue_research_phase(&mut self, phase: ResearchPhase) {
        let composition = self
            .pending_prompt_composition
            .clone()
            .unwrap_or_else(|| self.prompt_composition.clone());
        self.pending_prompt_composition = Some(composition.with_research_phase(phase));
    }

    /// Supplies the current derived research view for the next full Context
    /// compilation. The Research ledger remains its sole fact owner.
    pub fn with_world_state(mut self, world_state: ResearchWorldState) -> Self {
        self.world_state = Some(world_state);
        self
    }

    /// Replaces the derived research projection before the next sample. It
    /// never mutates the authoritative Research ledger.
    pub fn set_world_state(&mut self, world_state: ResearchWorldState) {
        self.world_state = Some(world_state);
    }

    /// Supplies the current derived Project Snapshot for the fixed Context
    /// slot. Request-audit/Diff persistence remains outside this Slice.
    pub fn with_project_snapshot(mut self, project_snapshot: ProjectSnapshotContext) -> Self {
        self.project_snapshot = Some(project_snapshot);
        self
    }

    /// Replaces the derived project snapshot before the next sample. The
    /// cache and snapshot fact owners remain outside this loop.
    pub fn set_project_snapshot(&mut self, project_snapshot: ProjectSnapshotContext) {
        self.project_snapshot = Some(project_snapshot);
    }

    /// Replaces the local candidate for the next Compaction. It does not
    /// change active model Context or reset the current Provider Baseline.
    pub fn update_session_summary_candidate(&mut self, summary: SessionSummaryDraft) {
        self.pending_session_summary = Some(summary);
    }

    /// Queues a named Agent profile for the next Turn. Validation happens now
    /// and is rechecked before any user History mutation or Provider call.
    pub fn queue_agent_model_profile(
        &mut self,
        profile: AgentModelProfile,
    ) -> Result<(), AgentLoopConfigurationError> {
        profile.validate()?;
        self.pending_agent_model_profile = Some(profile);
        Ok(())
    }

    pub fn agent_model_profile(&self) -> Option<&AgentModelProfile> {
        self.active_agent_model_profile.as_ref()
    }

    /// The most recently compiled Context, including a request which later
    /// failed before Provider completion.
    pub fn last_context_plan(&self) -> Option<&ContextPlan> {
        self.last_context_plan.as_ref()
    }

    /// Context state is promoted here only after the Provider response and
    /// local History/Baseline commit both succeed.
    pub fn last_successful_context(&self) -> Option<&ContextInspection> {
        self.last_successful_context.as_ref()
    }

    pub fn last_sampling_step_plan(&self) -> Option<&SamplingStepPlan> {
        self.last_sampling_step_plan.as_ref()
    }

    /// Returns the local delivery decision for the most recent actual model
    /// request.
    pub fn last_baseline_assessment(&self) -> Option<&BaselineAssessment> {
        self.last_baseline_assessment.as_ref()
    }

    pub async fn run_turn(
        &mut self,
        user_text: impl Into<String>,
        cancellation: AgentCancellation,
    ) -> Result<AgentTurnOutcome, AgentLoopError> {
        self.run_turn_inner(
            user_text,
            cancellation,
            None,
            None,
            None,
            StreamingStages::none(),
        )
        .await
    }

    /// Runs one Turn through an explicitly supplied request-audit stage. The
    /// ordinary [`Self::run_turn`] path deliberately remains unaudited so
    /// callers opt into Blob/Event persistence rather than getting a silent
    /// second fact path.
    pub async fn run_turn_audited(
        &mut self,
        user_text: impl Into<String>,
        cancellation: AgentCancellation,
        audit_context: &RequestAuditRunContext,
        record_attempt: &mut dyn FnMut(RequestAuditAttempt) -> Result<(), AgentLoopError>,
    ) -> Result<AgentTurnOutcome, AgentLoopError> {
        self.run_turn_inner(
            user_text,
            cancellation,
            Some((audit_context, record_attempt)),
            None,
            None,
            StreamingStages::none(),
        )
        .await
    }

    /// Runs the existing audited Primary path with one initial image context.
    /// Direct inserts the Artifact exactly once; secondary injects only the
    /// durable observation and Artifact references after scoped Vision has
    /// completed. It does not create another AgentLoop or Provider session.
    pub async fn run_turn_audited_with_initial_image(
        &mut self,
        user_text: impl Into<String>,
        initial_image: InitialImagePrimaryContext,
        cancellation: AgentCancellation,
        audit_context: &RequestAuditRunContext,
        record_attempt: &mut dyn FnMut(RequestAuditAttempt) -> Result<(), AgentLoopError>,
    ) -> Result<AgentTurnOutcome, AgentLoopError> {
        let primary_capability_hash = self.provider.capabilities().semantic_hash();
        let initial_image =
            initial_image.into_content_item(&self.profile, &primary_capability_hash)?;
        self.run_turn_inner(
            user_text,
            cancellation,
            Some((audit_context, record_attempt)),
            Some(initial_image),
            None,
            StreamingStages::none(),
        )
        .await
    }

    /// Runs the audited Primary path and handles each successful local
    /// `view_image` Tool Result before the next Primary sample. The caller's
    /// one typed stage owns route persistence and, for a secondary route, the
    /// scoped Vision request; this loop never infers an image from any other
    /// Tool Result and never sends both representations to Primary.
    pub async fn run_turn_audited_with_view_image_follow_up(
        &mut self,
        user_text: impl Into<String>,
        cancellation: AgentCancellation,
        audit_context: &RequestAuditRunContext,
        record_attempt: &mut dyn FnMut(RequestAuditAttempt) -> Result<(), AgentLoopError>,
        follow_up: &mut ViewImageFollowUp<'_>,
    ) -> Result<AgentTurnOutcome, AgentLoopError> {
        self.run_turn_inner(
            user_text,
            cancellation,
            Some((audit_context, record_attempt)),
            None,
            Some(follow_up),
            StreamingStages::none(),
        )
        .await
    }

    /// Runs the ordinary audited Primary path with fixed text-delta and
    /// Receipt stages. This is the CLI Chat path: text appears in streaming
    /// order, while every Tool outcome reaches the Session owner before a
    /// later Provider sample can fail.
    pub async fn run_turn_audited_with_stages(
        &mut self,
        user_text: impl Into<String>,
        cancellation: AgentCancellation,
        audit_context: &RequestAuditRunContext,
        record_attempt: &mut dyn FnMut(RequestAuditAttempt) -> Result<(), AgentLoopError>,
        text_delta: &mut AssistantTextDeltaStage<'_>,
        record_execution: &mut ToolExecutionReceiptStage<'_>,
    ) -> Result<AgentTurnOutcome, AgentLoopError> {
        self.run_turn_inner(
            user_text,
            cancellation,
            Some((audit_context, record_attempt)),
            None,
            None,
            StreamingStages {
                text_delta: Some(text_delta),
                record_execution: Some(record_execution),
            },
        )
        .await
    }

    /// Runs the production CLI path with the same streaming/Receipt stages
    /// plus the one fixed Vision composition. The optional initial image and
    /// every successful model-originated `view_image` result are mutually
    /// exclusive Direct-or-Secondary Primary contexts prepared by the
    /// existing Vision Session owner.
    pub(crate) async fn run_turn_audited_with_vision_stages(
        &mut self,
        user_text: impl Into<String>,
        initial_image: Option<InitialImagePrimaryContext>,
        cancellation: AgentCancellation,
        audit_context: &RequestAuditRunContext,
        record_attempt: &mut dyn FnMut(RequestAuditAttempt) -> Result<(), AgentLoopError>,
        stages: CliVisionStages<'_>,
    ) -> Result<AgentTurnOutcome, AgentLoopError> {
        let primary_capability_hash = self.provider.capabilities().semantic_hash();
        let initial_image = initial_image
            .map(|context| context.into_content_item(&self.profile, &primary_capability_hash))
            .transpose()?;
        self.run_turn_inner(
            user_text,
            cancellation,
            Some((audit_context, record_attempt)),
            initial_image,
            Some(stages.follow_up),
            StreamingStages {
                text_delta: Some(stages.text_delta),
                record_execution: Some(stages.record_execution),
            },
        )
        .await
    }

    async fn run_turn_inner(
        &mut self,
        user_text: impl Into<String>,
        cancellation: AgentCancellation,
        mut audit: Option<RequestAuditStage<'_>>,
        initial_image: Option<(ContentItem, ContextProvenance)>,
        mut view_image_follow_up: Option<&mut ViewImageFollowUp<'_>>,
        stages: StreamingStages<'_>,
    ) -> Result<AgentTurnOutcome, AgentLoopError> {
        let StreamingStages {
            mut text_delta,
            mut record_execution,
        } = stages;
        self.apply_pending_configuration()?;
        if cancellation.is_cancelled() {
            return Ok(AgentTurnOutcome::Cancelled {
                model_requests: 0,
                executions: Vec::new(),
            });
        }
        let user_text = user_text.into();
        let user_provenance = ContextProvenance::operator_input(&user_text)?;
        let turn_history_start = self.history.items().len();
        self.history.append_user_text(user_text);
        self.history_provenance
            .insert(turn_history_start, user_provenance);
        if let Some((initial_image, provenance)) = initial_image {
            let history_index = self.history.items().len();
            self.history.append_content_item(initial_image)?;
            self.history_provenance.insert(history_index, provenance);
        }
        let capabilities = self.provider.capabilities();
        let context_budget = self.context_planner.default_budget(&capabilities)?;
        self.next_turn_sequence += 1;
        let turn_id = TurnId::new(format!("turn:{}", self.next_turn_sequence));
        let mut deferred_activations = DeferredActivationSet::default();
        let mut executions = Vec::new();
        let mut provider_requests = 0;

        for request_index in 0..MAX_MODEL_REQUESTS_PER_TURN {
            self.history.ensure_all_tool_calls_paired()?;
            let resolved_tool_plan = self
                .tools
                .resolve_tool_plan(&self.active_tool_exposure_plan, &deferred_activations)?;
            let tool_specs = resolved_tool_plan.advertised_specs();
            let turn = TurnContext::new(
                turn_id.clone(),
                self.model.clone(),
                self.profile.clone(),
                self.provider_session.reference().clone(),
                capabilities.context_window,
                tool_specs,
            )?;
            let (plan, cache_summary) =
                context_state::compile_context(context_state::ContextCompilationInput {
                    prompt_composition: &self.prompt_composition,
                    world_state: self.world_state.as_ref(),
                    project_snapshot: self.project_snapshot.as_ref(),
                    session_summary: self.session_summary.as_ref(),
                    history: self.history.items(),
                    history_provenance: &self.history_provenance,
                    turn_history_start,
                    planner: &self.context_planner,
                    cache: self.fragment_cache.as_deref(),
                    budget: context_budget,
                    capabilities: &capabilities,
                    tool_specs,
                    prior_manifest: self.committed_context_manifest.as_ref(),
                })?;
            let sampling_step = SamplingStepPlan::new(
                format!("{}:step:{request_index}", turn_id),
                TurnConfigurationSnapshot::new(
                    turn.clone(),
                    self.prompt_composition.domain_profile(),
                    self.prompt_composition.research_phase(),
                    self.active_agent_model_profile.clone(),
                ),
                plan,
                resolved_tool_plan,
                capabilities.clone(),
            )?;
            let plan = sampling_step.context_plan();
            let full_input = plan.model_input();
            let full_delivery_map = plan.delivery_map().clone();
            let full_snapshot = StepSnapshot::capture_with_renderer(
                &turn,
                request_index,
                &full_input,
                &capabilities,
                sampling_step.prompt_renderer(),
                InternalRequestMetadata::default(),
            )?;
            let full_assessment = self
                .provider_session
                .assess_delivery(&full_snapshot, capabilities.incremental_input);
            let retains_reasoning = self.committed_delivery_input.as_ref().is_some_and(|input| {
                input
                    .content_items
                    .iter()
                    .any(|item| matches!(item, ContentItem::Reasoning { .. }))
            });
            let (mut delivery_input, mut delivery_map, mut snapshot, assessment) =
                if retains_reasoning
                    || matches!(
                        full_assessment,
                        BaselineAssessment::FullRequest {
                            reason: BaselineInvalidationReason::InputNotAppendOnly
                        }
                    )
                {
                    match context_state::state_diff_delivery_input(
                        context_state::StateDiffDeliveryInput {
                            step_id: sampling_step.step_id(),
                            full_input: &full_input,
                            full_delivery_map: &full_delivery_map,
                            current_plan: plan,
                            planner: &self.context_planner,
                            budget: context_budget,
                            committed_input: self.committed_delivery_input.as_ref(),
                            committed_delivery_map: self.committed_delivery_map.as_ref(),
                            committed_history_len: self.committed_history_len,
                            committed_manifest: self.committed_context_manifest.as_ref(),
                            committed_world_state: self.committed_world_state.as_ref(),
                            world_state: self.world_state.as_ref(),
                            committed_project_snapshot: self.committed_project_snapshot.as_ref(),
                            project_snapshot: self.project_snapshot.as_ref(),
                            history: self.history.items(),
                            history_provenance: &self.history_provenance,
                            turn_history_start,
                        },
                    )? {
                        Some((candidate_input, candidate_map)) => {
                            let candidate_snapshot = StepSnapshot::capture_with_renderer(
                                &turn,
                                request_index,
                                &candidate_input,
                                &capabilities,
                                sampling_step.prompt_renderer(),
                                InternalRequestMetadata::default(),
                            )?;
                            let candidate_assessment = self.provider_session.assess_delivery(
                                &candidate_snapshot,
                                capabilities.incremental_input,
                            );
                            match candidate_assessment {
                                BaselineAssessment::AppendOnlyCandidate { .. } => (
                                    candidate_input,
                                    candidate_map,
                                    candidate_snapshot,
                                    candidate_assessment,
                                ),
                                BaselineAssessment::FullRequest { reason }
                                    if reason.permits_state_diff_full_delivery()
                                        || (reason.permits_distinct_full_delivery()
                                            && candidate_input
                                                .is_reasoning_replay_of(&full_input)) =>
                                {
                                    (
                                        candidate_input,
                                        candidate_map,
                                        candidate_snapshot,
                                        candidate_assessment,
                                    )
                                }
                                _ if retains_reasoning => {
                                    return Err(AgentLoopError::ProviderContinuationUnavailable);
                                }
                                _ => (
                                    full_input.clone(),
                                    full_delivery_map.clone(),
                                    full_snapshot,
                                    full_assessment,
                                ),
                            }
                        }
                        None => (
                            full_input.clone(),
                            full_delivery_map.clone(),
                            full_snapshot,
                            full_assessment,
                        ),
                    }
                } else {
                    (
                        full_input.clone(),
                        full_delivery_map.clone(),
                        full_snapshot,
                        full_assessment,
                    )
                };
            self.last_baseline_assessment = Some(assessment.clone());
            let (request, request_delivery_map) =
                self.request_for_assessment(&delivery_input, &delivery_map, &assessment)?;
            reserve_provider_request(&mut provider_requests)?;
            self.last_context_plan = Some(plan.clone());
            self.last_sampling_step_plan = Some(sampling_step.clone());
            self.record_request_audit_attempt(
                &mut audit,
                &sampling_step,
                &full_input,
                &request,
                &request_delivery_map,
                &assessment,
            )?;
            let mut forward_text_delta = |event: &crate::domain::model::ModelEvent| {
                if let (crate::domain::model::ModelEvent::TextDelta { text }, Some(stage)) =
                    (event, text_delta.as_deref_mut())
                {
                    stage(text).map_err(|error| ModelStreamingError::EventStage {
                        message: error.to_string(),
                    })?;
                }
                Ok(())
            };
            let stream = if request.is_incremental() {
                let mut incremental_text_deltas = Vec::new();
                let mut buffer_incremental_text = |event: &crate::domain::model::ModelEvent| {
                    if let crate::domain::model::ModelEvent::TextDelta { text } = event {
                        incremental_text_deltas.push(text.clone());
                    }
                    Ok(())
                };
                match ModelStreamingService::new(self.provider)
                    .consume_with_event_stage(
                        request.clone(),
                        cancellation.clone(),
                        &mut buffer_incremental_text,
                    )
                    .await
                {
                    Ok(outcome) => {
                        for text in incremental_text_deltas {
                            if let Some(stage) = text_delta.as_deref_mut() {
                                stage(&text)?;
                            }
                        }
                        Ok(outcome)
                    }
                    Err(ModelStreamingError::Provider(
                        ModelProviderError::ContinuationRejected { .. },
                    )) => {
                        self.provider_session
                            .reset(BaselineInvalidationReason::ProviderRejected);
                        let fallback_assessment = BaselineAssessment::FullRequest {
                            reason: BaselineInvalidationReason::ProviderRejected,
                        };
                        self.last_baseline_assessment = Some(fallback_assessment.clone());
                        if !delivery_input
                            .content_items
                            .iter()
                            .any(|item| matches!(item, ContentItem::Reasoning { .. }))
                        {
                            delivery_input = full_input.clone();
                            delivery_map = full_delivery_map.clone();
                            snapshot = StepSnapshot::capture_with_renderer(
                                &turn,
                                request_index,
                                &delivery_input,
                                &capabilities,
                                sampling_step.prompt_renderer(),
                                InternalRequestMetadata::default(),
                            )?;
                        }
                        reserve_provider_request(&mut provider_requests)?;
                        let fallback_request = ModelRequest::full(delivery_input.clone());
                        delivery_map.validate_against(&delivery_input)?;
                        self.record_request_audit_attempt(
                            &mut audit,
                            &sampling_step,
                            &full_input,
                            &fallback_request,
                            &delivery_map,
                            &fallback_assessment,
                        )?;
                        ModelStreamingService::new(self.provider)
                            .consume_with_event_stage(
                                fallback_request,
                                cancellation.clone(),
                                &mut forward_text_delta,
                            )
                            .await
                    }
                    Err(error) => Err(error),
                }
            } else {
                ModelStreamingService::new(self.provider)
                    .consume_with_event_stage(
                        request.clone(),
                        cancellation.clone(),
                        &mut forward_text_delta,
                    )
                    .await
            }?;
            let (events, completion) = match stream {
                ModelStreamOutcome::Completed { events, completion } => (events, completion),
                ModelStreamOutcome::Cancelled { .. } => {
                    return Ok(AgentTurnOutcome::Cancelled {
                        model_requests: provider_requests,
                        executions,
                    });
                }
            };
            let sample = ModelSample::from_events(events);
            let tool_calls = sample.tool_calls();
            let assistant_text = sample.assistant_text();
            if tool_calls.is_empty() && assistant_text.is_empty() {
                return Err(AgentLoopError::EmptyModelResponse);
            }
            self.require_continuation_identity(&completion, capabilities.incremental_input)?;
            let server_output_items = sample.content_items();
            sample.commit(&mut self.history)?;
            self.provider_session
                .record_completed_sample(&snapshot, server_output_items.clone())?;
            self.commit_continuation(&completion);
            delivery_input
                .content_items
                .extend(server_output_items.clone());
            delivery_map
                .append_model_output_items(sampling_step.step_id(), &server_output_items)?;
            delivery_map.validate_against(&delivery_input)?;
            self.committed_delivery_input = Some(delivery_input);
            self.committed_delivery_map = Some(delivery_map);
            self.committed_history_len = self.history.items().len();
            self.committed_world_state = self.world_state.clone();
            self.committed_project_snapshot = self.project_snapshot.clone();
            self.committed_context_manifest = Some(plan.full_logical_manifest().clone());
            self.last_successful_context = Some(ContextInspection {
                plan: plan.clone(),
                baseline_assessment: self
                    .last_baseline_assessment
                    .clone()
                    .expect("the actual request always has a Baseline assessment"),
                cache_summary,
            });

            if tool_calls.is_empty() {
                return Ok(AgentTurnOutcome::Completed {
                    assistant_text,
                    model_requests: provider_requests,
                    executions,
                });
            }

            let scheduled = self
                .execute_tool_calls(
                    tool_calls,
                    sampling_step.resolved_tool_plan(),
                    &cancellation,
                    &mut record_execution,
                )
                .await?;
            for execution in &scheduled.executions {
                deferred_activations
                    .apply_search_result(&execution.result, sampling_step.resolved_tool_plan())?;
            }
            let view_image_follow_up = self
                .append_view_image_contexts(&scheduled.executions, &mut view_image_follow_up)
                .await;
            executions.extend(scheduled.executions);
            match view_image_follow_up {
                Ok(()) => {}
                Err(AgentLoopError::ViewImageFollowUpCancelled) if cancellation.is_cancelled() => {
                    return Ok(AgentTurnOutcome::Cancelled {
                        model_requests: provider_requests,
                        executions,
                    });
                }
                Err(error) => return Err(error),
            }
            if scheduled.cancelled {
                return Ok(AgentTurnOutcome::Cancelled {
                    model_requests: provider_requests,
                    executions,
                });
            }
        }

        Err(AgentLoopError::ModelRequestLimit {
            limit: MAX_MODEL_REQUESTS_PER_TURN,
        })
    }

    async fn append_view_image_contexts(
        &mut self,
        executions: &[ToolExecutionOutcome],
        follow_up: &mut Option<&mut ViewImageFollowUp<'_>>,
    ) -> Result<(), AgentLoopError> {
        for execution in executions {
            let Some(selection) =
                ViewImageSelection::from_tool_result(&execution.result).map_err(|error| {
                    AgentLoopError::ViewImageResult {
                        message: error.to_string(),
                    }
                })?
            else {
                continue;
            };
            let stage = follow_up.as_deref_mut().ok_or_else(|| {
                AgentLoopError::ViewImageFollowUpRequired {
                    call_id: execution.result.call_id.clone(),
                }
            })?;
            let context = stage(selection).await?;
            let primary_capability_hash = self.provider.capabilities().semantic_hash();
            let (content, provenance) =
                context.into_content_item(&self.profile, &primary_capability_hash)?;
            let history_index = self.history.items().len();
            self.history.append_content_item(content)?;
            self.history_provenance.insert(history_index, provenance);
        }
        Ok(())
    }

    fn apply_pending_configuration(&mut self) -> Result<(), AgentLoopConfigurationError> {
        let pending_composition = self.pending_prompt_composition.as_ref();
        let pending_profile = self.pending_agent_model_profile.as_ref();
        if let Some(profile) = pending_profile {
            profile.validate()?;
            if profile.primary().model() != &self.model {
                return Err(AgentLoopConfigurationError::PrimaryModelDoesNotMatchProvider);
            }
            if profile.primary().capabilities() != &self.provider.capabilities() {
                return Err(AgentLoopConfigurationError::PrimaryCapabilitiesDoNotMatchProvider);
            }
        }
        if let Some(composition) = pending_composition {
            let exposure = composition
                .tool_exposure()
                .restricted_to_catalog(&self.tools.catalog());
            if self.active_tool_exposure_plan != exposure {
                self.tools
                    .resolve_tool_plan(&exposure, &DeferredActivationSet::default())?;
                self.active_tool_exposure_plan = exposure;
                self.provider_session
                    .reset(BaselineInvalidationReason::ToolsChanged);
            }
        }
        if let Some(composition) = self.pending_prompt_composition.take() {
            self.prompt_composition = composition;
        }
        if let Some(profile) = self.pending_agent_model_profile.take() {
            let reset_primary = self.primary_semantics_changed(&profile);
            self.model = profile.primary().model().clone();
            self.profile = profile.primary().profile().clone();
            if reset_primary {
                let generation = NEXT_PRIMARY_RUNTIME_SESSION.fetch_add(1, Ordering::Relaxed);
                let reset_id = format!(
                    "{}:runtime-reset:{generation}",
                    self.provider_session.reference().id
                );
                self.provider_session = ProviderSession::new(
                    ProviderSessionId::new(reset_id),
                    ProviderSessionRole::Primary,
                    generation,
                );
                self.clear_committed_primary_delivery_state();
            }
            self.active_agent_model_profile = Some(profile);
        }
        Ok(())
    }

    fn primary_semantics_changed(&self, next: &AgentModelProfile) -> bool {
        let next_primary = next.primary();
        self.profile != *next_primary.profile()
            || self.model != *next_primary.model()
            || self.provider.capabilities() != *next_primary.capabilities()
    }

    fn clear_committed_primary_delivery_state(&mut self) {
        self.continuation = None;
        self.committed_context_manifest = None;
        self.committed_delivery_input = None;
        self.committed_delivery_map = None;
        self.committed_history_len = self.history.items().len();
        self.committed_world_state = None;
        self.committed_project_snapshot = None;
        self.last_baseline_assessment = None;
    }

    fn request_for_assessment(
        &self,
        full_input: &ModelInputIR,
        full_delivery_map: &ContextDeliveryMap,
        assessment: &BaselineAssessment,
    ) -> Result<(ModelRequest, ContextDeliveryMap), AgentLoopError> {
        full_delivery_map.validate_against(full_input)?;
        Ok(match assessment {
            BaselineAssessment::FullRequest { .. } => (
                ModelRequest::full(full_input.clone()),
                full_delivery_map.clone(),
            ),
            BaselineAssessment::AppendOnlyCandidate { delta_items } => {
                let continuation = self
                    .continuation
                    .as_ref()
                    .filter(|stored| stored.session == *self.provider_session.reference())
                    .map(|stored| stored.response_id.clone())
                    .ok_or(AgentLoopError::ContinuationIdentityMissing)?;
                let mut delta = full_input.clone();
                delta.content_items = delta_items.clone();
                let delivery_map = full_delivery_map.for_incremental_delta(full_input, &delta)?;
                (ModelRequest::incremental(delta, continuation), delivery_map)
            }
        })
    }

    fn require_continuation_identity(
        &self,
        completion: &ModelResponseCompletion,
        incremental_input_supported: bool,
    ) -> Result<(), AgentLoopError> {
        if incremental_input_supported && completion.response_id().is_none() {
            return Err(AgentLoopError::ContinuationIdentityMissing);
        }
        Ok(())
    }

    fn commit_continuation(&mut self, completion: &ModelResponseCompletion) {
        self.continuation =
            completion
                .response_id()
                .cloned()
                .map(|response_id| CommittedContinuation {
                    session: self.provider_session.reference().clone(),
                    response_id,
                });
    }

    async fn execute_tool_calls(
        &mut self,
        calls: Vec<ToolCall>,
        resolved_tool_plan: &ResolvedToolPlan,
        cancellation: &AgentCancellation,
        receipt_stage: &mut Option<&mut ToolExecutionReceiptStage<'_>>,
    ) -> Result<ScheduledCalls, AgentLoopError> {
        let mut prepared = Vec::with_capacity(calls.len());
        let mut plans = Vec::new();
        for call in calls {
            let execution_id = ExecutionId::new(format!("execution:{}", call.call_id));
            match self
                .tools
                .plan_model_call(resolved_tool_plan, execution_id, call.clone())
            {
                Ok(plan) => {
                    if resolved_tool_plan.validates_execution_plan(&plan) {
                        plans.push(plan);
                        prepared.push(PreparedToolCall::Planned);
                    } else {
                        prepared.push(PreparedToolCall::PlanningFailure(planning_failure_result(
                            &call,
                            ToolExecutorError::ResolvedPlanMismatch,
                        )));
                    }
                }
                Err(ToolExecutorError::UnmatchedToolCall { tool_name }) => {
                    return Err(AgentLoopError::UnmatchedToolCall { tool_name });
                }
                Err(error) => prepared.push(PreparedToolCall::PlanningFailure(
                    planning_failure_result(&call, error),
                )),
            }
        }

        let planned_outcome_count = plans.len();
        let scheduled = execute_batch(
            self.tools,
            plans,
            cancellation,
            &mut self.completed_idempotency,
        )
        .await;
        if scheduled.outcomes.len() != planned_outcome_count {
            return Err(AgentLoopError::SchedulerResultCountMismatch);
        }
        let mut outcomes = scheduled.outcomes.into_iter();
        let mut executions = Vec::new();
        for entry in prepared {
            match entry {
                PreparedToolCall::Planned => {
                    let outcome = outcomes
                        .next()
                        .ok_or(AgentLoopError::SchedulerResultMissing)?;
                    self.history.append_tool_result(outcome.result.clone())?;
                    executions.push(outcome);
                }
                PreparedToolCall::PlanningFailure(result) => {
                    self.history.append_tool_result(result)?;
                }
            }
        }
        if outcomes.next().is_some() {
            return Err(AgentLoopError::SchedulerResultCountMismatch);
        }
        self.history.ensure_all_tool_calls_paired()?;
        if let Some(stage) = receipt_stage.as_deref_mut() {
            for outcome in &executions {
                stage(outcome)?;
            }
        }
        Ok(ScheduledCalls {
            executions,
            cancelled: scheduled.cancelled,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CommittedContinuation {
    session: crate::domain::context::ProviderSessionRef,
    response_id: ProviderResponseId,
}

enum PreparedToolCall {
    Planned,
    PlanningFailure(ToolResult),
}

struct ScheduledCalls {
    executions: Vec<ToolExecutionOutcome>,
    cancelled: bool,
}

fn planning_failure_result(call: &ToolCall, error: ToolExecutorError) -> ToolResult {
    let (code, retryability, message) = match error {
        ToolExecutorError::InvalidArguments { message, .. } => (
            "invalid_arguments",
            ToolFailureRetryability::CorrectArguments,
            message,
        ),
        ToolExecutorError::UnknownTool { tool_name } => (
            "unknown_tool",
            ToolFailureRetryability::Terminal,
            format!("Tool `{tool_name}` is not registered"),
        ),
        ToolExecutorError::ToolNotExposed { tool_name } => (
            "tool_not_exposed",
            ToolFailureRetryability::Terminal,
            format!("Tool `{tool_name}` is not advertised by the captured Tool plan"),
        ),
        ToolExecutorError::PlanFailure { message, .. } => (
            "tool_plan_failure",
            ToolFailureRetryability::Terminal,
            message,
        ),
        ToolExecutorError::ResolvedPlanFailure { message } => (
            "resolved_tool_plan_failure",
            ToolFailureRetryability::Terminal,
            message,
        ),
        ToolExecutorError::ResolvedPlanMismatch => (
            "resolved_tool_plan_mismatch",
            ToolFailureRetryability::Terminal,
            "the Tool execution plan does not match the immutable advertised plan".to_owned(),
        ),
        ToolExecutorError::UnmatchedToolCall { tool_name } => (
            "unmatched_tool_call",
            ToolFailureRetryability::Terminal,
            format!("Tool `{tool_name}` is not present in the fixed Replay trace"),
        ),
    };
    ToolResult::from_pre_execution_failure(
        call,
        ToolFailure::pre_execution(code, retryability, message.clone()),
        serde_json::json!({"error": message}),
    )
}

fn reserve_provider_request(provider_requests: &mut usize) -> Result<(), AgentLoopError> {
    if *provider_requests == MAX_MODEL_REQUESTS_PER_TURN {
        return Err(AgentLoopError::ModelRequestLimit {
            limit: MAX_MODEL_REQUESTS_PER_TURN,
        });
    }
    *provider_requests += 1;
    Ok(())
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum AgentLoopConfigurationError {
    #[error("the AgentLoop requires a primary ProviderSession")]
    PrimarySessionRequired,
    #[error(transparent)]
    History(#[from] HistoryError),
    #[error(transparent)]
    Session(#[from] SessionError),
    #[error(transparent)]
    PromptComposition(#[from] ResearchControlError),
    #[error(transparent)]
    AgentModelProfile(#[from] AgentModelProfileError),
    #[error(transparent)]
    Tool(#[from] crate::ports::tool::ToolExecutorError),
    #[error("the selected primary profile capability mapping does not match the active Provider")]
    PrimaryCapabilitiesDoNotMatchProvider,
    #[error("the selected primary model does not match the Provider bound to this AgentLoop")]
    PrimaryModelDoesNotMatchProvider,
    #[error("resume requires an Active SessionState with a durable session.resumed fact")]
    ActiveResumedSessionRequired,
    #[error(
        "replayed Session primary identity {expected:?} does not match supplied runtime identity {received:?}"
    )]
    ReplayedSessionProviderMismatch {
        expected: crate::domain::context::ProviderSessionRef,
        received: crate::domain::context::ProviderSessionRef,
    },
    #[error("apply_compaction requires a durable session.compaction_created fact")]
    CompactionRequired,
    #[error("Session Summary v2 requires current Project, Research stream and Receipt facts")]
    CompactionValidationContextRequired,
    #[error("the durable Compaction Summary does not match the active local candidate")]
    CompactionSummaryCandidateMismatch,
    #[error("Compaction requires an Active replayed SessionState")]
    ActiveSessionCompactionRequired,
    #[error("Compaction requires a replayed Session stream identity")]
    CompactionSessionStreamMissing,
    #[error(
        "Compaction Session stream `{received}` does not match the AgentLoop-bound Session stream `{expected}`"
    )]
    CompactionSessionStreamMismatch {
        expected: StreamId,
        received: StreamId,
    },
    #[error(
        "Compaction Session primary identity {expected:?} does not match the active AgentLoop identity {received:?}"
    )]
    CompactionProviderMismatch {
        expected: crate::domain::context::ProviderSessionRef,
        received: crate::domain::context::ProviderSessionRef,
    },
    #[error(
        "Compaction revision {received_revision} is stale for Session stream `{stream_id}`; revision {applied_revision} is already applied"
    )]
    StaleCompaction {
        stream_id: StreamId,
        applied_revision: u64,
        received_revision: u64,
    },
    #[error(
        "Compaction revision {revision} conflicts with the already applied durable fact for Session stream `{stream_id}`"
    )]
    ConflictingCompaction { stream_id: StreamId, revision: u64 },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentTurnOutcome {
    Completed {
        assistant_text: String,
        model_requests: usize,
        executions: Vec<ToolExecutionOutcome>,
    },
    Cancelled {
        model_requests: usize,
        executions: Vec<ToolExecutionOutcome>,
    },
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum AgentLoopError {
    #[error(transparent)]
    Model(#[from] ModelStreamingError),
    #[error(transparent)]
    History(#[from] HistoryError),
    #[error(transparent)]
    Session(#[from] SessionError),
    #[error(transparent)]
    RequestAudit(#[from] RequestAuditError),
    #[error(transparent)]
    Context(#[from] ContextError),
    #[error(transparent)]
    ContextCompilation(#[from] ContextCompilationError),
    #[error(transparent)]
    ContextPlanner(#[from] crate::domain::context::ContextPlannerError),
    #[error(transparent)]
    ResearchControl(#[from] ResearchControlError),
    #[error(transparent)]
    WorldState(#[from] WorldStateError),
    #[error(transparent)]
    Configuration(#[from] AgentLoopConfigurationError),
    #[error(transparent)]
    Tool(#[from] ToolExecutorError),
    #[error(transparent)]
    ToolContract(#[from] crate::domain::tools::ToolContractError),
    #[error(transparent)]
    SamplingPlan(#[from] crate::domain::sampling::SamplingPlanError),
    #[error("AgentLoop History does not contain the current Turn input")]
    ContextStateMismatch,
    #[error("the model completed without text or a Tool Call")]
    EmptyModelResponse,
    #[error("an incremental-capable Provider completed without a continuation identity")]
    ContinuationIdentityMissing,
    #[error(
        "completed Provider reasoning continuation cannot be retained by the current Context plan"
    )]
    ProviderContinuationUnavailable,
    #[error("the selected RequestAudit delivery does not match the actual Provider request")]
    RequestAuditDeliveryMismatch,
    #[error("RequestAudit commit failed: {message}")]
    RequestAuditCommit { message: String },
    #[error("Execution Receipt commit failed: {message}")]
    ExecutionReceiptCommit { message: String },
    #[error("assistant text presentation failed: {message}")]
    TextDeltaOutput { message: String },
    #[error("initial image context is invalid: {message}")]
    InitialImageContext { message: String },
    #[error("initial image route identity does not match the active Primary provider")]
    InitialImagePrimaryIdentityMismatch,
    #[error("successful view_image Tool Result is malformed: {message}")]
    ViewImageResult { message: String },
    #[error(
        "successful view_image Tool Call `{call_id}` requires the fixed image follow-up stage before the next model sample"
    )]
    ViewImageFollowUpRequired {
        call_id: crate::domain::tools::ToolCallId,
    },
    #[error("the fixed view_image follow-up was cancelled before the next model sample")]
    ViewImageFollowUpCancelled,
    #[error("the Agent reached the per-Turn model-request limit of {limit}")]
    ModelRequestLimit { limit: usize },
    #[error("the Tool scheduler returned fewer outcomes than staged plans")]
    SchedulerResultMissing,
    #[error("the Tool scheduler returned more outcomes than staged plans")]
    SchedulerResultCountMismatch,
    #[error("UNMATCHED_TOOL_CALL: Tool `{tool_name}` is not present in the Replay trace")]
    UnmatchedToolCall { tool_name: String },
}
