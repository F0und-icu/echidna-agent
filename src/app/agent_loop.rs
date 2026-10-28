//! Bounded single-Agent Tool Call -> Tool Result -> full-resample loop.

mod argument_correction;
mod compaction;
mod compaction_sample;
pub(crate) use compaction_sample::{
    CompactionGraphSample, CompactionSampleInput, compile_captured_compaction_source,
    minimum_compaction_handoff_bytes,
};
mod context_state;
mod definition;
mod execution;
mod improvement;
pub(crate) use improvement::{
    ImprovementBoundary, ImprovementSampleInput, ImprovementSampleOutput,
};
mod output_capture;
mod recovery;
mod request_audit;

pub(crate) use compaction::{CompactionFit, ToolBatchContext};
pub(crate) use context_state::CurrentContextState;

pub(crate) trait ToolBatchStage<Provider: ModelProvider + ?Sized, Tools: ToolExecutor + ?Sized> {
    fn needs_current_context(&self) -> bool;

    fn apply<'call, 'provider: 'call, 'tools: 'call>(
        &'call mut self,
        agent: &'call mut AgentLoop<'provider, 'tools, Provider, Tools>,
        active: &'call ToolBatchContext<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<(), AgentLoopError>> + 'call>>;
}

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
use crate::domain::model::{ContentItem, MessageRole, ModelInputIR, OpaqueContinuationScope};
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
    ToolExecutionOutcome, ToolExecutionPlan, ToolExposurePlan, ToolFailure,
    ToolFailureRetryability, ToolResult,
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
use super::model_retry::{ModelRetryError, prepare_and_consume_with_retry};
use super::model_sample::ModelSample;
use super::model_streaming::{ModelStreamOutcome, ModelStreamingError, ModelStreamingEventStage};
use super::tool_scheduler::{ScheduledToolResult, execute_batch};
use compaction::AppliedCompaction;
use request_audit::RequestAuditStage;

use crate::app::model_request_budget::{DEFAULT_MODEL_REQUEST_LIMIT, ModelRequestBudget};
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
        let provenance = crate::domain::vision::VisionRouteRecord::primary_direct_provenance(
            source_artifact_id,
            delivery_artifact_id.clone(),
            route_event_id,
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
        let provenance = observation.primary_context_provenance(observation_event_id)?;
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

/// The one fixed pre-sampling Context refresh. Its result is typed current
/// data, not a hook payload; the AgentLoop immediately captures a fresh
/// immutable SamplingStepPlan after installing it.
pub(crate) type ContextRefreshStage<'a> =
    dyn FnMut() -> Result<CurrentContextState, AgentLoopError> + 'a;

/// The existing fixed stage that makes a completed Tool outcome durable before
/// the next Provider sample. It remains available to the generic audited
/// runtime path; CLI Chat uses [`ChatDurabilityStage`] so its paired History
/// fact and real Receipt are checkpointed by one Session owner.
pub type ToolExecutionReceiptStage<'a> =
    dyn FnMut(&ToolExecutionOutcome) -> Result<(), AgentLoopError> + 'a;

/// A bounded CLI-only durability checkpoint. This is one typed stage rather
/// than a general transaction or Hook abstraction: user/model History is
/// checkpointed before its dependent action; a real completed Receipt is
/// checkpointed at scheduler completion; and ordered paired Tool Results are
/// checkpointed before a later model sample.
pub(crate) enum ChatDurabilityCheckpoint<'a> {
    Usage {
        provider_session: &'a crate::domain::context::ProviderSessionRef,
        usage: crate::domain::model::ModelUsage,
    },
    ModelSampleRejected {
        diagnostic: &'a crate::domain::model::ToolArgumentDiagnostic,
        corrections_used: u8,
    },
    ArgumentCorrectionReset,
    History(&'a [ContentItem]),
    OperatorInputAccepted {
        items: &'a [ContentItem],
        provenance: &'a mut BTreeMap<usize, ContextProvenance>,
    },
    OperatorTurnCompleted,
    ModelDeliveryCommitted {
        step_id: &'a str,
        previous_step_id: Option<&'a str>,
        provider_session: &'a crate::domain::context::ProviderSessionRef,
    },
    ExecutionReceipt(&'a ToolExecutionOutcome),
    Subcommand(&'a crate::domain::tools::SubcommandCheckpoint),
}

/// The fixed Chat persistence handoff. The CLI Session owner supplies this
/// stage; AgentLoop retains History pairing, Tool execution and scheduling.
pub(crate) type ChatDurabilityStage<'a> = dyn for<'checkpoint> FnMut(ChatDurabilityCheckpoint<'checkpoint>) -> Result<(), AgentLoopError>
    + 'a;

/// The fixed CLI-only notification emitted at the exact scheduler boundary
/// before a Tool future is started. It is presentation only and cannot alter
/// the immutable execution plan or dispatch path.
pub(crate) type ToolExecutionStartStage<'a> = dyn FnMut(&ToolExecutionPlan) + 'a;

/// The one fixed stage for user-visible assistant text while a response is
/// streaming. Reasoning is intentionally not part of this stage and remains
/// outside Conversation History and Assistant output. The protocol Adapter may
/// present returned reasoning separately on labeled CLI stderr (ADR-0018).
pub type AssistantTextDeltaStage<'a> = dyn FnMut(&str) -> Result<(), AgentLoopError> + 'a;

/// The fixed callback bundle used only by production CLI Vision composition.
/// Grouping these existing stages keeps the entry point bounded without
/// introducing a registry or another lifecycle owner.
pub(crate) struct CliRuntimeStages<'a> {
    request_budget: Arc<ModelRequestBudget>,
    follow_up: &'a mut ViewImageFollowUp<'a>,
    text_delta: &'a mut AssistantTextDeltaStage<'a>,
    tool_start: &'a mut ToolExecutionStartStage<'a>,
    durability: &'a mut ChatDurabilityStage<'a>,
    context_refresh: &'a mut ContextRefreshStage<'a>,
}

impl<'a> CliRuntimeStages<'a> {
    pub(crate) fn new(
        follow_up: &'a mut ViewImageFollowUp<'a>,
        text_delta: &'a mut AssistantTextDeltaStage<'a>,
        tool_start: &'a mut ToolExecutionStartStage<'a>,
        durability: &'a mut ChatDurabilityStage<'a>,
        context_refresh: &'a mut ContextRefreshStage<'a>,
        request_budget: Arc<ModelRequestBudget>,
    ) -> Self {
        Self {
            request_budget,
            follow_up,
            text_delta,
            tool_start,
            durability,
            context_refresh,
        }
    }
}

struct StreamingStages<'a> {
    request_budget: Option<Arc<ModelRequestBudget>>,
    text_delta: Option<&'a mut AssistantTextDeltaStage<'a>>,
    tool_start: Option<&'a mut ToolExecutionStartStage<'a>>,
    record_execution: Option<&'a mut ToolExecutionReceiptStage<'a>>,
    durability: Option<&'a mut ChatDurabilityStage<'a>>,
    context_refresh: Option<&'a mut ContextRefreshStage<'a>>,
}

impl StreamingStages<'_> {
    fn none() -> Self {
        Self {
            request_budget: None,
            text_delta: None,
            tool_start: None,
            record_execution: None,
            durability: None,
            context_refresh: None,
        }
    }
}

/// One accepted Provider delivery; its required provenance travels atomically.
struct CommittedPrimaryDelivery {
    step_id: String,
    continuation: Option<CommittedContinuation>,
    context_manifest: FullLogicalManifest,
    input: ModelInputIR,
    map: ContextDeliveryMap,
    unactivated_tool_plan: ResolvedToolPlan,
    history_len: usize,
    world_state: Option<ResearchWorldState>,
    project_snapshot: Option<ProjectSnapshotContext>,
    project_memory_core: Option<crate::domain::context::ContextFragment>,
}

pub struct AgentLoop<'provider, 'tools, Provider: ?Sized, Tools: ?Sized> {
    provider: &'provider Provider,
    tools: &'tools Tools,
    output_publisher: Option<Box<output_capture::OutputPublisher<'tools>>>,
    prompt_composition: PromptComposition,
    pending_prompt_composition: Option<PromptComposition>,
    world_state: Option<ResearchWorldState>,
    project_snapshot: Option<ProjectSnapshotContext>,
    project_memory_core: Option<crate::domain::context::ContextFragment>,
    session_summary: Option<SessionSummary>,
    pending_session_summary: Option<SessionSummaryDraft>,
    history: ConversationHistory,
    model: ModelIdentity,
    profile: ModelProfile,
    provider_session: ProviderSession,
    completed_idempotency: BTreeMap<IdempotencyKey, ToolExecutionOutcome>,
    context_planner: ContextPlanner,
    fragment_cache: Option<Arc<dyn ContextFragmentCache>>,
    last_context_plan: Option<ContextPlan>,
    last_successful_context: Option<ContextInspection>,
    last_sampling_step_plan: Option<SamplingStepPlan>,
    next_turn_sequence: u64,
    consecutive_argument_corrections: u8,
    last_baseline_assessment: Option<BaselineAssessment>,
    active_agent_model_profile: Option<AgentModelProfile>,
    pending_agent_model_profile: Option<AgentModelProfile>,
    active_tool_exposure_plan: ToolExposurePlan,
    committed_delivery: Option<CommittedPrimaryDelivery>,
    history_provenance: BTreeMap<usize, ContextProvenance>,
    protected_input: crate::domain::session::ProtectedOperatorContext,
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
            output_publisher: None,
            pending_prompt_composition: None,
            world_state: None,
            project_snapshot: None,
            project_memory_core: None,
            session_summary: None,
            pending_session_summary: None,
            history: ConversationHistory::default(),
            model,
            profile,
            provider_session,
            completed_idempotency: BTreeMap::new(),
            context_planner: ContextPlanner::current(),
            fragment_cache: None,
            last_context_plan: None,
            last_successful_context: None,
            last_sampling_step_plan: None,
            next_turn_sequence: 0,
            consecutive_argument_corrections: 0,
            last_baseline_assessment: None,
            active_agent_model_profile: None,
            pending_agent_model_profile: None,
            active_tool_exposure_plan,
            committed_delivery: None,
            history_provenance: BTreeMap::new(),
            protected_input: Default::default(),
            bound_session_stream: None,
            applied_compaction: None,
        })
    }

    pub fn history(&self) -> &ConversationHistory {
        &self.history
    }

    pub(crate) fn with_context_budget_policy(
        mut self,
        policy: crate::domain::context::ContextBudgetPolicy,
    ) -> Self {
        self.context_planner = ContextPlanner::current().with_budget_policy(policy);
        self
    }

    pub(crate) fn set_protected_context(
        &mut self,
        protected: crate::domain::session::ProtectedOperatorContext,
    ) {
        self.history_provenance = protected.retained_provenance.clone();
        self.protected_input = protected;
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
        let normalized_items = session.compaction_source_history()?;
        let history = ConversationHistory::from_complete_content_items(&normalized_items)?;
        let applied_compaction = session
            .latest_compaction()
            .map(|compaction| AppliedCompaction::from_compaction(stream_id.clone(), compaction))
            .transpose()?;
        provider_session.reset(BaselineInvalidationReason::SessionResumed);
        self.history = history;
        self.protected_input = session.current_protected_operator_context()?;
        self.history_provenance = self.protected_input.retained_provenance.clone();
        self.provider_session = provider_session;
        self.consecutive_argument_corrections = 0;
        self.bound_session_stream = Some(stream_id);
        self.session_summary = session
            .latest_compaction()
            .map(|compaction| compaction.summary().clone());
        self.pending_session_summary = None;
        self.applied_compaction = applied_compaction;
        let mut composition = self
            .pending_prompt_composition
            .take()
            .unwrap_or_else(|| self.prompt_composition.clone());
        if composition.research_enabled() {
            composition = composition.with_research_phase(session.effective_research_phase());
        }
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

    /// Installs one typed current Context snapshot before a sampling Step or
    /// Compaction preflight. Pending operator configuration is applied first;
    /// this stage never changes Domain/Profile/Phase from refreshed facts.
    pub(crate) fn refresh_current_context(
        &mut self,
        current: CurrentContextState,
    ) -> Result<(), AgentLoopError> {
        self.apply_pending_configuration()?;
        self.world_state = current.world_state().cloned();
        self.project_snapshot = Some(current.project_snapshot().clone());
        self.project_memory_core = current.project_memory_core().cloned();
        Ok(())
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
            Vec::new(),
            None,
            StreamingStages::none(),
            None,
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
            Vec::new(),
            None,
            StreamingStages::none(),
            None,
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
            vec![initial_image],
            None,
            StreamingStages::none(),
            None,
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
            Vec::new(),
            Some(follow_up),
            StreamingStages::none(),
            None,
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
            Vec::new(),
            None,
            StreamingStages {
                request_budget: None,
                text_delta: Some(text_delta),
                tool_start: None,
                record_execution: Some(record_execution),
                durability: None,
                context_refresh: None,
            },
            None,
        )
        .await
    }

    /// Production Chat's existing Vision/durability path plus the one fixed
    /// current-facts refresh before every sampling Step. The refresh runs
    /// after the Turn-boundary configuration has been applied and before a
    /// new immutable SamplingStepPlan exists.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn run_turn_audited_with_context_refresh_and_vision_stages(
        &mut self,
        user_text: impl Into<String>,
        initial_images: Vec<InitialImagePrimaryContext>,
        cancellation: AgentCancellation,
        audit_context: &RequestAuditRunContext,
        record_attempt: &mut dyn FnMut(RequestAuditAttempt) -> Result<(), AgentLoopError>,
        stages: CliRuntimeStages<'_>,
        tool_batch: Option<&mut dyn ToolBatchStage<Provider, Tools>>,
    ) -> Result<AgentTurnOutcome, AgentLoopError> {
        let primary_capability_hash = self.provider.capabilities().semantic_hash();
        let initial_images = initial_images
            .into_iter()
            .map(|context| context.into_content_item(&self.profile, &primary_capability_hash))
            .collect::<Result<Vec<_>, _>>()?;
        self.run_turn_inner(
            user_text,
            cancellation,
            Some((audit_context, record_attempt)),
            initial_images,
            Some(stages.follow_up),
            StreamingStages {
                request_budget: Some(stages.request_budget),
                text_delta: Some(stages.text_delta),
                tool_start: Some(stages.tool_start),
                record_execution: None,
                durability: Some(stages.durability),
                context_refresh: Some(stages.context_refresh),
            },
            tool_batch,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_turn_inner(
        &mut self,
        user_text: impl Into<String>,
        cancellation: AgentCancellation,
        mut audit: Option<RequestAuditStage<'_>>,
        initial_images: Vec<(ContentItem, ContextProvenance)>,
        mut view_image_follow_up: Option<&mut ViewImageFollowUp<'_>>,
        stages: StreamingStages<'_>,
        mut tool_batch: Option<&mut dyn ToolBatchStage<Provider, Tools>>,
    ) -> Result<AgentTurnOutcome, AgentLoopError> {
        let StreamingStages {
            request_budget,
            mut text_delta,
            mut tool_start,
            mut record_execution,
            mut durability,
            mut context_refresh,
        } = stages;
        let request_budget = request_budget
            .unwrap_or_else(|| Arc::new(ModelRequestBudget::new(DEFAULT_MODEL_REQUEST_LIMIT)));
        self.apply_pending_configuration()?;
        if cancellation.is_cancelled() {
            return Ok(AgentTurnOutcome::Cancelled {
                model_requests: 0,
                executions: Vec::new(),
            });
        }
        let user_text = user_text.into();
        let user_provenance = ContextProvenance::operator_input(&user_text)?;
        let mut turn_history_start = self.history.items().len();
        self.history.append_user_text(user_text);
        self.history_provenance
            .insert(turn_history_start, user_provenance);
        for (initial_image, provenance) in initial_images {
            let history_index = self.history.items().len();
            self.history.append_content_item(initial_image)?;
            self.history_provenance.insert(history_index, provenance);
        }
        if let Some(stage) = durability.as_deref_mut() {
            stage(ChatDurabilityCheckpoint::History(
                &self.history.items()[turn_history_start..],
            ))?;
        }
        let original_input = self.history.items()[turn_history_start..].to_vec();
        let mut input_provenance: BTreeMap<_, _> = self
            .history_provenance
            .range(turn_history_start..)
            .map(|(index, provenance)| (index - turn_history_start, provenance.clone()))
            .collect();
        if let Some(stage) = durability.as_deref_mut() {
            stage(ChatDurabilityCheckpoint::OperatorInputAccepted {
                items: &original_input,
                provenance: &mut input_provenance,
            })?;
            self.history_provenance.extend(
                input_provenance
                    .iter()
                    .map(|(index, provenance)| (turn_history_start + index, provenance.clone())),
            );
        }
        let mut detached_input = false;
        let capabilities = self.provider.capabilities();
        let context_budget = self.context_planner.default_budget(&capabilities)?;
        self.next_turn_sequence += 1;
        let turn_id = TurnId::new(format!("turn:{}", self.next_turn_sequence));
        let mut deferred_activations = DeferredActivationSet::default();
        let unactivated_tool_plan = self
            .tools
            .resolve_tool_plan(&self.active_tool_exposure_plan, &deferred_activations)?;
        let mut executions = Vec::new();
        let mut corrections_used = self.consecutive_argument_corrections;
        let mut correction_feedback = Vec::new();
        let mut turn_has_committed_sample = false;
        let mut next_request_index = 0;
        loop {
            let request_index = next_request_index;
            if let Some(limit) = request_budget.limit()
                && request_index >= limit
            {
                return Err(AgentLoopError::ModelRequestLimit { limit });
            }
            next_request_index += 1;
            if cancellation.is_cancelled() {
                return Ok(AgentTurnOutcome::Cancelled {
                    model_requests: request_budget.counts().primary,
                    executions,
                });
            }
            if let Some(refresh) = context_refresh.as_deref_mut() {
                self.refresh_current_context(refresh()?)?;
            }
            self.history.ensure_all_tool_calls_paired()?;
            let resolved_tool_plan = if request_index == 0 {
                unactivated_tool_plan.clone()
            } else {
                self.tools
                    .resolve_tool_plan(&self.active_tool_exposure_plan, &deferred_activations)?
            };
            let expired_plan = (!turn_has_committed_sample)
                .then_some(())
                .and_then(|_| {
                    self.committed_delivery
                        .as_ref()
                        .map(|state| &state.unactivated_tool_plan)
                })
                .filter(|prior| *prior == &resolved_tool_plan);
            let tool_specs = resolved_tool_plan.advertised_specs();
            let turn = TurnContext::new(
                turn_id.clone(),
                self.model.clone(),
                self.profile.clone(),
                self.provider_session.reference().clone(),
                capabilities.context_window,
                tool_specs,
            )?;
            let protected = self.protected_without_active(&input_provenance);
            let (plan, cache_summary) =
                context_state::compile_context(context_state::ContextCompilationInput {
                    prompt_composition: &self.prompt_composition,
                    world_state: self.world_state.as_ref(),
                    project_snapshot: self.project_snapshot.as_ref(),
                    project_memory_core: self.project_memory_core.as_ref(),
                    session_summary: self.session_summary.as_ref(),
                    history: self.history.items(),
                    history_provenance: &self.history_provenance,
                    protected_input: Some((&protected.0, &protected.1)),
                    turn_history_start,
                    detached_input: detached_input
                        .then_some((original_input.as_slice(), &input_provenance)),
                    planner: &self.context_planner,
                    cache: self.fragment_cache.as_deref(),
                    budget: context_budget,
                    capabilities: &capabilities,
                    tool_specs,
                    prior_manifest: self
                        .committed_delivery
                        .as_ref()
                        .map(|state| &state.context_manifest),
                    correction_feedback: &correction_feedback,
                    capacity_breakdown: None,
                })?;
            let sampling_step = SamplingStepPlan::new(
                format!("{}:step:{request_index}", turn_id),
                TurnConfigurationSnapshot::new(
                    turn.clone(),
                    self.prompt_composition
                        .research_enabled()
                        .then(|| self.prompt_composition.domain_profile()),
                    self.prompt_composition
                        .research_enabled()
                        .then(|| self.prompt_composition.research_phase()),
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
            let retains_reasoning = self
                .committed_delivery
                .as_ref()
                .map(|state| &state.input)
                .is_some_and(|input| {
                    input.content_items.iter().any(|item| {
                        matches!(
                            item,
                            ContentItem::Reasoning { .. } | ContentItem::OpaqueContinuation { .. }
                        )
                    })
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
                            correction_feedback: &correction_feedback,
                            step_id: sampling_step.step_id(),
                            full_input: &full_input,
                            full_delivery_map: &full_delivery_map,
                            current_plan: plan,
                            planner: &self.context_planner,
                            budget: context_budget,
                            committed_delivery: self.committed_delivery.as_ref(),
                            world_state: self.world_state.as_ref(),
                            project_snapshot: self.project_snapshot.as_ref(),
                            project_memory_core: self.project_memory_core.as_ref(),
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
                                    if reason.permits_full_input_relationship(
                                        &full_input,
                                        &candidate_input,
                                    ) && (reason
                                        != BaselineInvalidationReason::ToolsChanged
                                        || candidate_input.is_reasoning_replay_of(&full_input)
                                        || ((!deferred_activations.names().is_empty()
                                            || expired_plan.is_some())
                                            && self
                                                .committed_delivery
                                                .as_ref()
                                                .map(|state| &state.input)
                                                .is_some_and(|previous| {
                                                    self.provider_session
                                                        .tool_change_preserves_delivery(
                                                            &candidate_snapshot,
                                                            previous,
                                                            &candidate_input,
                                                            expired_plan,
                                                        )
                                                }))) =>
                                {
                                    (
                                        candidate_input,
                                        candidate_map,
                                        candidate_snapshot,
                                        candidate_assessment,
                                    )
                                }
                                BaselineAssessment::FullRequest { reason } if retains_reasoning => {
                                    return Err(AgentLoopError::ProviderContinuationUnavailable {
                                        reason: ContinuationRejection::BaselineRejected { reason },
                                    });
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
            self.last_context_plan = Some(plan.clone());
            self.last_sampling_step_plan = Some(sampling_step.clone());
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
                let mut discard_events =
                    |_event: &crate::domain::model::ModelEvent| Ok::<(), ModelStreamingError>(());
                match self
                    .consume_prepared_request_with_retry(
                        request.clone(),
                        cancellation.clone(),
                        &request_budget,
                        &mut audit,
                        &sampling_step,
                        &full_input,
                        &request_delivery_map,
                        &assessment,
                        &mut discard_events,
                    )
                    .await
                {
                    Ok(outcome) => {
                        if let ModelStreamOutcome::Completed { events, .. } = &outcome {
                            for event in events {
                                if let crate::domain::model::ModelEvent::TextDelta { text } = event
                                    && let Some(stage) = text_delta.as_deref_mut()
                                {
                                    stage(text)?;
                                }
                            }
                        }
                        Ok(outcome)
                    }
                    Err(AgentLoopError::Model(ModelStreamingError::Provider(
                        ModelProviderError::ContinuationRejected { .. },
                    ))) => {
                        self.provider_session
                            .reset(BaselineInvalidationReason::ProviderRejected);
                        let fallback_assessment = BaselineAssessment::FullRequest {
                            reason: BaselineInvalidationReason::ProviderRejected,
                        };
                        self.last_baseline_assessment = Some(fallback_assessment.clone());
                        if !delivery_input.content_items.iter().any(|item| {
                            matches!(
                                item,
                                ContentItem::Reasoning { .. }
                                    | ContentItem::OpaqueContinuation { .. }
                            )
                        }) {
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
                        let fallback_request = ModelRequest::full(delivery_input.clone())
                            .with_opaque_continuation_scope(
                                OpaqueContinuationScope::for_provider_session(
                                    self.provider_session.reference(),
                                ),
                            );
                        delivery_map.validate_against(&delivery_input)?;
                        self.consume_prepared_request_with_retry(
                            fallback_request,
                            cancellation.clone(),
                            &request_budget,
                            &mut audit,
                            &sampling_step,
                            &full_input,
                            &delivery_map,
                            &fallback_assessment,
                            &mut forward_text_delta,
                        )
                        .await
                    }
                    Err(error) => Err(error),
                }
            } else {
                self.consume_prepared_request_with_retry(
                    request.clone(),
                    cancellation.clone(),
                    &request_budget,
                    &mut audit,
                    &sampling_step,
                    &full_input,
                    &request_delivery_map,
                    &assessment,
                    &mut forward_text_delta,
                )
                .await
            };
            let stream = match stream {
                Err(AgentLoopError::Model(ModelStreamingError::Provider(
                    ModelProviderError::InvalidToolArguments { diagnostic },
                ))) => {
                    // No model sample has committed and no sibling ToolCall
                    // has dispatched. Persist rejection before any correction.
                    if let Some(stage) = durability.as_deref_mut() {
                        stage(ChatDurabilityCheckpoint::ModelSampleRejected {
                            diagnostic: &diagnostic,
                            corrections_used,
                        })?;
                    }
                    if corrections_used
                        >= crate::domain::model::MAX_TOOL_ARGUMENT_CORRECTIONS_PER_CHAIN
                    {
                        return Err(ModelStreamingError::Provider(
                            ModelProviderError::InvalidToolArguments { diagnostic },
                        )
                        .into());
                    }
                    corrections_used += 1;
                    self.consecutive_argument_corrections = corrections_used;
                    correction_feedback.push(argument_correction::feedback(
                        sampling_step.step_id(),
                        &diagnostic,
                    )?);
                    continue;
                }
                other => other?,
            };
            let (events, completion) = match stream {
                ModelStreamOutcome::Completed { events, completion } => (events, completion),
                ModelStreamOutcome::Cancelled { .. } => {
                    return Ok(AgentTurnOutcome::Cancelled {
                        model_requests: request_budget.counts().primary,
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
            let sample_history_start = self.history.items().len();
            sample.commit(&mut self.history)?;
            if let Some(stage) = durability.as_deref_mut() {
                stage(ChatDurabilityCheckpoint::History(
                    &self.history.items()[sample_history_start..],
                ))?;
                if corrections_used > 0 {
                    stage(ChatDurabilityCheckpoint::ArgumentCorrectionReset)?;
                }
            }
            corrections_used = 0;
            self.consecutive_argument_corrections = 0;
            self.provider_session
                .record_completed_sample(&snapshot, server_output_items.clone())?;
            turn_has_committed_sample = true;
            delivery_input
                .content_items
                .extend(server_output_items.clone());
            delivery_map
                .append_model_output_items(sampling_step.step_id(), &server_output_items)?;
            delivery_map.validate_against(&delivery_input)?;
            let previous_step_id = self
                .committed_delivery
                .as_ref()
                .map(|delivery| delivery.step_id.clone());
            self.committed_delivery = Some(CommittedPrimaryDelivery {
                step_id: sampling_step.step_id().to_owned(),
                continuation: completion.response_id().cloned().map(|response_id| {
                    CommittedContinuation {
                        session: self.provider_session.reference().clone(),
                        response_id,
                    }
                }),
                input: delivery_input,
                map: delivery_map,
                unactivated_tool_plan: unactivated_tool_plan.clone(),
                history_len: self.history.items().len(),
                world_state: self.world_state.clone(),
                project_snapshot: self.project_snapshot.clone(),
                project_memory_core: self.project_memory_core.clone(),
                context_manifest: plan.full_logical_manifest().clone(),
            });
            self.last_successful_context = Some(ContextInspection {
                plan: plan.clone(),
                baseline_assessment: self
                    .last_baseline_assessment
                    .clone()
                    .expect("the actual request always has a Baseline assessment"),
                cache_summary,
            });
            if let Some(usage) = sample.usage()
                && let Some(stage) = durability.as_deref_mut()
            {
                stage(ChatDurabilityCheckpoint::Usage {
                    provider_session: self.provider_session.reference(),
                    usage,
                })?;
            }
            if let Some(stage) = durability.as_deref_mut() {
                stage(ChatDurabilityCheckpoint::ModelDeliveryCommitted {
                    step_id: sampling_step.step_id(),
                    previous_step_id: previous_step_id.as_deref(),
                    provider_session: self.provider_session.reference(),
                })?;
            }

            if tool_calls.is_empty() {
                self.history.ensure_all_tool_calls_paired()?;
                if let Some(stage) = durability.as_deref_mut() {
                    stage(ChatDurabilityCheckpoint::OperatorTurnCompleted)?;
                }
                return Ok(AgentTurnOutcome::Completed {
                    assistant_text,
                    model_requests: request_budget.counts().primary,
                    executions,
                });
            }

            let scheduled = self
                .execute_tool_calls(
                    tool_calls,
                    sampling_step.resolved_tool_plan(),
                    &cancellation,
                    &mut tool_start,
                    &mut record_execution,
                    &mut durability,
                )
                .await?;
            for execution in &scheduled.executions {
                deferred_activations
                    .apply_search_result(&execution.result, sampling_step.resolved_tool_plan())?;
            }
            let image_history_start = self.history.items().len();
            let view_image_follow_up = self
                .append_view_image_contexts(&scheduled.executions, &mut view_image_follow_up)
                .await;
            // A later scoped image can stop after earlier observations were
            // admitted. Checkpoint those known contexts before propagating it.
            if let Some(stage) = durability.as_deref_mut()
                && self.history.items().len() > image_history_start
            {
                stage(ChatDurabilityCheckpoint::History(
                    &self.history.items()[image_history_start..],
                ))?;
            }
            executions.extend(scheduled.executions);
            match view_image_follow_up {
                Ok(()) => {}
                Err(AgentLoopError::ViewImageFollowUpCancelled) if cancellation.is_cancelled() => {
                    return Ok(AgentTurnOutcome::Cancelled {
                        model_requests: request_budget.counts().primary,
                        executions,
                    });
                }
                Err(error) => return Err(error),
            }
            if scheduled.cancelled {
                return Ok(AgentTurnOutcome::Cancelled {
                    model_requests: request_budget.counts().primary,
                    executions,
                });
            }
            if !cancellation.is_cancelled()
                && let Some(stage) = tool_batch.as_deref_mut()
            {
                if (stage.needs_current_context() || self.delivery_needs_compaction()?)
                    && let Some(refresh) = context_refresh.as_deref_mut()
                {
                    self.refresh_current_context(refresh()?)?;
                }
                let before = self.applied_compaction.clone();
                let previous_len = self.history.items().len();
                let tool_plan = self
                    .tools
                    .resolve_tool_plan(&self.active_tool_exposure_plan, &deferred_activations)?;
                let active = ToolBatchContext {
                    turn_id: &turn_id,
                    history_end: self.history.items().len(),
                    history_start: turn_history_start,
                    original_input: &original_input,
                    input_provenance: &input_provenance,
                    detached: detached_input,
                    tool_specs: tool_plan.advertised_specs(),
                };
                stage.apply(self, &active).await?;
                if self.applied_compaction != before {
                    correction_feedback.clear();
                    let dropped = previous_len
                        .checked_sub(self.history.items().len())
                        .ok_or(AgentLoopError::ContextStateMismatch)?;
                    if detached_input || dropped > turn_history_start {
                        detached_input = true;
                        turn_history_start = 0;
                    } else {
                        turn_history_start -= dropped;
                        self.history_provenance.extend(input_provenance.iter().map(
                            |(index, provenance)| (turn_history_start + index, provenance.clone()),
                        ));
                    }
                }
            }
        }
    }

    // Keep the existing immutable audit inputs explicit at this fixed stage.
    #[allow(clippy::too_many_arguments)]
    async fn consume_prepared_request_with_retry(
        &self,
        request: ModelRequest,
        cancellation: AgentCancellation,
        request_budget: &ModelRequestBudget,
        audit: &mut Option<RequestAuditStage<'_>>,
        sampling_step: &SamplingStepPlan,
        full_input: &ModelInputIR,
        delivery_map: &ContextDeliveryMap,
        assessment: &BaselineAssessment,
        event_stage: &mut ModelStreamingEventStage<'_>,
    ) -> Result<ModelStreamOutcome, AgentLoopError> {
        let provider = self.provider;
        prepare_and_consume_with_retry(
            provider,
            cancellation,
            || {
                provider
                    .prepare(request.clone())
                    .map_err(ModelStreamingError::Provider)
            },
            || {
                request_budget
                    .admit(ProviderSessionRole::Primary)
                    .map_err(|limit| AgentLoopError::ModelRequestLimit { limit })?;
                self.record_request_audit_attempt(
                    audit,
                    sampling_step,
                    full_input,
                    &request,
                    delivery_map,
                    assessment,
                )
            },
            event_stage,
        )
        .await
        .map_err(|error| match error {
            ModelRetryError::Attempt(error) => error,
            ModelRetryError::Streaming(error) => AgentLoopError::Model(error),
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
            let exposure = composition.tool_exposure_for_catalog(&self.tools.catalog())?;
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
        self.committed_delivery = None;
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
                ModelRequest::full(full_input.clone()).with_opaque_continuation_scope(
                    OpaqueContinuationScope::for_provider_session(
                        self.provider_session.reference(),
                    ),
                ),
                full_delivery_map.clone(),
            ),
            BaselineAssessment::AppendOnlyCandidate { delta_items } => {
                let continuation = self
                    .committed_delivery
                    .as_ref()
                    .and_then(|state| state.continuation.as_ref())
                    .filter(|stored| stored.session == *self.provider_session.reference())
                    .map(|stored| stored.response_id.clone())
                    .ok_or(AgentLoopError::ContinuationIdentityMissing)?;
                let mut delta = full_input.clone();
                delta.content_items = delta_items.clone();
                let delivery_map = full_delivery_map.for_incremental_delta(full_input, &delta)?;
                (
                    ModelRequest::incremental(delta, continuation).with_opaque_continuation_scope(
                        OpaqueContinuationScope::for_provider_session(
                            self.provider_session.reference(),
                        ),
                    ),
                    delivery_map,
                )
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

    async fn execute_tool_calls(
        &mut self,
        calls: Vec<ToolCall>,
        resolved_tool_plan: &ResolvedToolPlan,
        cancellation: &AgentCancellation,
        start_stage: &mut Option<&mut ToolExecutionStartStage<'_>>,
        receipt_stage: &mut Option<&mut ToolExecutionReceiptStage<'_>>,
        durability_stage: &mut Option<&mut ChatDurabilityStage<'_>>,
    ) -> Result<execution::ScheduledCalls, AgentLoopError> {
        execution::execute(
            execution::ExecutionContext {
                tools: self.tools,
                output_publisher: self.output_publisher.as_deref(),
                history: &mut self.history,
                completed_idempotency: &mut self.completed_idempotency,
            },
            calls,
            resolved_tool_plan,
            cancellation,
            start_stage,
            receipt_stage,
            durability_stage,
        )
        .await
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CommittedContinuation {
    session: crate::domain::context::ProviderSessionRef,
    response_id: ProviderResponseId,
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

/// Why a completed provider delivery cannot accompany the current logical
/// Context. These describe rejected local candidates, never Provider usage.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ContinuationRejection {
    #[error("MISSING_BASELINE: committed delivery provenance or manifest is unavailable")]
    MissingBaseline,
    #[error("HISTORY_REPLACED: current History cannot be appended without loss")]
    HistoryReplaced,
    #[error("RESEARCH_PROJECTION_CHANGED: full Research projections are not both retained")]
    ResearchProjectionChanged,
    #[error("PROJECT_PROJECTION_CHANGED: full Project projections are not both retained")]
    ProjectProjectionChanged,
    #[error("CORE_MEMORY_CHANGED: Core Memory changed without a proven complete replacement")]
    CoreMemoryChanged,
    #[error(
        "BASELINE_REJECTED: {reason:?} is incompatible with this exact reasoning/state-diff replay"
    )]
    BaselineRejected { reason: BaselineInvalidationReason },
    #[error(
        "DELIVERY_BUDGET_EXCEEDED: actual delivery candidate estimate={rendered_input_tokens} input tokens, planner limit={input_token_limit}; Provider usage=unavailable"
    )]
    DeliveryBudgetExceeded {
        rendered_input_tokens: u32,
        input_token_limit: u32,
    },
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum AgentLoopError {
    #[error("RSI scope was cancelled; no retry or adoption occurred")]
    ImprovementCancelled,
    #[error("RSI scope returned an unsupported response: {message}")]
    ImprovementResponse { message: String },
    #[error("Tool output persistence failed: {message}; execution is not retried")]
    ToolOutputPublication { message: String },
    #[error(transparent)]
    Model(#[from] ModelStreamingError),
    #[error("model rejection checkpoint failed: {message}")]
    ModelRejectionCommit { message: String },
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
        "completed Provider reasoning continuation cannot be retained: {reason}; candidate was not sent; no automatic retry or reasoning discard"
    )]
    ProviderContinuationUnavailable { reason: ContinuationRejection },
    #[error("the selected RequestAudit delivery does not match the actual Provider request")]
    RequestAuditDeliveryMismatch,
    #[error("RequestAudit commit failed: {message}")]
    RequestAuditCommit { message: String },
    #[error("History checkpoint commit failed: {message}")]
    HistoryCheckpointCommit { message: String },
    #[error("Provider usage checkpoint commit failed: {message}")]
    ModelUsageCommit { message: String },
    #[error("current Context refresh failed: {message}")]
    ContextRefresh { message: String },
    #[error("durable tool-batch compaction failed: {message}")]
    ToolBatchCompaction { message: String },
    #[error("Compaction summary rejected: {reason}")]
    CompactionSummaryRejected { reason: String },
    #[error("Compaction summary cancelled; parent delivery retained")]
    CompactionCancelled,
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
    #[error("scoped Vision Provider failed after its durable failure checkpoint: {message}")]
    VisionProviderStopped { message: String },
    #[error("the Agent reached the per-Turn model-request limit of {limit}")]
    ModelRequestLimit { limit: usize },
    #[error(
        "Tool execution was interrupted before an outcome was observed; external effects are unknown, no Receipt or result was invented and no Tool was retried"
    )]
    ToolExecutionInterrupted,
    #[error(
        "Tool execution outcome is unknown after dispatch: {safe_reason}; no Receipt or result was invented and no Tool was retried"
    )]
    ToolExecutionOutcomeUnknown { safe_reason: String },
    #[error("the Tool scheduler returned fewer outcomes than staged plans")]
    SchedulerResultMissing,
    #[error("the Tool scheduler returned more outcomes than staged plans")]
    SchedulerResultCountMismatch,
    #[error("UNMATCHED_TOOL_CALL: Tool `{tool_name}` is not present in the Replay trace")]
    UnmatchedToolCall { tool_name: String },
}
