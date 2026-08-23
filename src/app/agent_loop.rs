//! Bounded single-Agent Tool Call -> Tool Result -> full-resample loop.

mod compaction;
mod context_state;
mod request_audit;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use thiserror::Error;

use crate::domain::context::{
    BaselineAssessment, BaselineInvalidationReason, ContextError, ContextPlan, ContextPlanner,
    FullLogicalManifest, InternalRequestMetadata, ModelIdentity, ModelProfile, ProviderSession,
    ProviderSessionId, ProviderSessionRole, StepSnapshot, TurnContext, TurnId,
};
use crate::domain::events::StreamId;
use crate::domain::history::{ConversationHistory, HistoryError};
use crate::domain::mode::{ModeError, PromptComposition};
use crate::domain::model::ModelInputIR;
use crate::domain::profile::{AgentModelProfile, AgentModelProfileError};
use crate::domain::request_audit::{
    RequestAuditAttempt, RequestAuditError, RequestAuditRunContext,
};
use crate::domain::session::{SessionError, SessionState, SessionSummary, SessionSummaryDraft};
use crate::domain::tools::{
    ExecutionId, IdempotencyKey, ToolCall, ToolExecutionOutcome, ToolExposurePlan, ToolResult,
};
use crate::domain::world_state::{ProjectSnapshotContext, ResearchWorldState, WorldStateError};
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
    next_turn_sequence: u64,
    last_baseline_assessment: Option<BaselineAssessment>,
    active_agent_model_profile: Option<AgentModelProfile>,
    pending_agent_model_profile: Option<AgentModelProfile>,
    active_tool_exposure_plan: ToolExposurePlan,
    committed_delivery_input: Option<ModelInputIR>,
    committed_history_len: usize,
    committed_world_state: Option<ResearchWorldState>,
    committed_project_snapshot: Option<ProjectSnapshotContext>,
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
    pub fn new(
        provider: &'provider Provider,
        tools: &'tools Tools,
        instructions: impl Into<String>,
    ) -> Self {
        Self::with_primary_context(
            provider,
            tools,
            instructions,
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
        instructions: impl Into<String>,
        model: ModelIdentity,
        profile: ModelProfile,
        provider_session: ProviderSession,
    ) -> Result<Self, AgentLoopConfigurationError> {
        if provider_session.reference().role != ProviderSessionRole::Primary {
            return Err(AgentLoopConfigurationError::PrimarySessionRequired);
        }
        let prompt_composition = PromptComposition::new(instructions.into())?;
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
            next_turn_sequence: 0,
            last_baseline_assessment: None,
            active_agent_model_profile: None,
            pending_agent_model_profile: None,
            active_tool_exposure_plan: ToolExposurePlan::default(),
            committed_delivery_input: None,
            committed_history_len: 0,
            committed_world_state: None,
            committed_project_snapshot: None,
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
        let normalized_items = session.normalized_history_items()?;
        let history = ConversationHistory::from_complete_content_items(&normalized_items)?;
        let applied_compaction = session
            .latest_compaction()
            .map(|compaction| AppliedCompaction::from_compaction(stream_id.clone(), compaction))
            .transpose()?;
        provider_session.reset(BaselineInvalidationReason::SessionResumed);
        self.history = history;
        self.provider_session = provider_session;
        self.bound_session_stream = Some(stream_id);
        self.session_summary = session
            .latest_compaction()
            .map(|compaction| compaction.summary().clone());
        self.pending_session_summary = None;
        self.applied_compaction = applied_compaction;
        self.clear_committed_primary_delivery_state();
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

    /// Queues a prompt/Mode change for the next Turn boundary.
    pub fn queue_prompt_composition(&mut self, composition: PromptComposition) {
        self.pending_prompt_composition = Some(composition);
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
        self.run_turn_inner(user_text, cancellation, None).await
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
        )
        .await
    }

    async fn run_turn_inner(
        &mut self,
        user_text: impl Into<String>,
        cancellation: AgentCancellation,
        mut audit: Option<RequestAuditStage<'_>>,
    ) -> Result<AgentTurnOutcome, AgentLoopError> {
        self.apply_pending_configuration()?;
        if cancellation.is_cancelled() {
            return Ok(AgentTurnOutcome::Cancelled {
                model_requests: 0,
                executions: Vec::new(),
            });
        }
        let turn_history_start = self.history.items().len();
        self.history.append_user_text(user_text);
        let capabilities = self.provider.capabilities();
        let context_budget = self.context_planner.default_budget(&capabilities)?;
        self.next_turn_sequence += 1;
        let turn_id = TurnId::new(format!("turn:{}", self.next_turn_sequence));
        let mut executions = Vec::new();
        let mut provider_requests = 0;

        for request_index in 0..MAX_MODEL_REQUESTS_PER_TURN {
            self.history.ensure_all_tool_calls_paired()?;
            // A successful tool_search changes the actual schema visible to
            // the next sample, so direct specs are intentionally refreshed.
            let tool_specs = self.current_tool_specs();
            let turn = TurnContext::new(
                turn_id.clone(),
                self.model.clone(),
                self.profile.clone(),
                self.provider_session.reference().clone(),
                capabilities.context_window,
                &tool_specs,
            )?;
            let (plan, cache_summary) =
                context_state::compile_context(context_state::ContextCompilationInput {
                    prompt_composition: &self.prompt_composition,
                    world_state: self.world_state.as_ref(),
                    project_snapshot: self.project_snapshot.as_ref(),
                    session_summary: self.session_summary.as_ref(),
                    history: self.history.items(),
                    turn_history_start,
                    planner: &self.context_planner,
                    cache: self.fragment_cache.as_deref(),
                    budget: context_budget,
                    capabilities: &capabilities,
                    tool_specs: &tool_specs,
                    prior_manifest: self.committed_context_manifest.as_ref(),
                })?;
            let full_input = plan.model_input();
            let full_snapshot = StepSnapshot::capture(
                &turn,
                request_index,
                &full_input,
                &capabilities,
                InternalRequestMetadata::default(),
            )?;
            let full_assessment = self
                .provider_session
                .assess_delivery(&full_snapshot, capabilities.incremental_input);
            let (mut delivery_input, mut snapshot, assessment) = if matches!(
                full_assessment,
                BaselineAssessment::FullRequest {
                    reason: BaselineInvalidationReason::InputNotAppendOnly
                }
            ) {
                match context_state::state_diff_delivery_input(
                    context_state::StateDiffDeliveryInput {
                        full_input: &full_input,
                        current_plan: &plan,
                        planner: &self.context_planner,
                        budget: context_budget,
                        committed_input: self.committed_delivery_input.as_ref(),
                        committed_history_len: self.committed_history_len,
                        committed_manifest: self.committed_context_manifest.as_ref(),
                        committed_world_state: self.committed_world_state.as_ref(),
                        world_state: self.world_state.as_ref(),
                        committed_project_snapshot: self.committed_project_snapshot.as_ref(),
                        project_snapshot: self.project_snapshot.as_ref(),
                        history: self.history.items(),
                        turn_history_start,
                    },
                )? {
                    Some(candidate_input) => {
                        let candidate_snapshot = StepSnapshot::capture(
                            &turn,
                            request_index,
                            &candidate_input,
                            &capabilities,
                            InternalRequestMetadata::default(),
                        )?;
                        let candidate_assessment = self
                            .provider_session
                            .assess_delivery(&candidate_snapshot, capabilities.incremental_input);
                        match candidate_assessment {
                            BaselineAssessment::AppendOnlyCandidate { .. } => {
                                (candidate_input, candidate_snapshot, candidate_assessment)
                            }
                            _ => (full_input.clone(), full_snapshot, full_assessment),
                        }
                    }
                    None => (full_input.clone(), full_snapshot, full_assessment),
                }
            } else {
                (full_input.clone(), full_snapshot, full_assessment)
            };
            self.last_baseline_assessment = Some(assessment.clone());
            let request = self.request_for_assessment(&delivery_input, &assessment)?;
            reserve_provider_request(&mut provider_requests)?;
            self.last_context_plan = Some(plan.clone());
            self.record_request_audit_attempt(
                &mut audit,
                &full_input,
                &request,
                &capabilities,
                &assessment,
            )?;
            let stream = match ModelStreamingService::new(self.provider)
                .consume(request.clone(), cancellation.clone())
                .await
            {
                Ok(outcome) => Ok(outcome),
                Err(ModelStreamingError::Provider(ModelProviderError::ContinuationRejected {
                    ..
                })) if request.is_incremental() => {
                    self.provider_session
                        .reset(BaselineInvalidationReason::ProviderRejected);
                    let fallback_assessment = BaselineAssessment::FullRequest {
                        reason: BaselineInvalidationReason::ProviderRejected,
                    };
                    self.last_baseline_assessment = Some(fallback_assessment.clone());
                    delivery_input = full_input.clone();
                    snapshot = StepSnapshot::capture(
                        &turn,
                        request_index,
                        &delivery_input,
                        &capabilities,
                        InternalRequestMetadata::default(),
                    )?;
                    reserve_provider_request(&mut provider_requests)?;
                    let fallback_request = ModelRequest::full(delivery_input.clone());
                    self.record_request_audit_attempt(
                        &mut audit,
                        &full_input,
                        &fallback_request,
                        &capabilities,
                        &fallback_assessment,
                    )?;
                    ModelStreamingService::new(self.provider)
                        .consume(fallback_request, cancellation.clone())
                        .await
                }
                Err(error) => Err(error),
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
            delivery_input.content_items.extend(server_output_items);
            self.committed_delivery_input = Some(delivery_input);
            self.committed_history_len = self.history.items().len();
            self.committed_world_state = self.world_state.clone();
            self.committed_project_snapshot = self.project_snapshot.clone();
            self.committed_context_manifest = Some(plan.full_logical_manifest().clone());
            self.last_successful_context = Some(ContextInspection {
                plan,
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

            let exposed_tool_names = tool_specs.iter().map(|spec| spec.name.clone()).collect();
            let scheduled = self
                .execute_tool_calls(tool_calls, &exposed_tool_names, &cancellation)
                .await?;
            executions.extend(scheduled.executions);
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

    fn current_tool_specs(&self) -> Vec<crate::domain::tools::ToolSpec> {
        self.tools.specs()
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
                .mode()
                .map(|mode| mode.tool_exposure().clone())
                .unwrap_or_else(ToolExposurePlan::default);
            if self.active_tool_exposure_plan != exposure {
                self.tools.configure_exposure(exposure.clone())?;
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
        self.committed_history_len = self.history.items().len();
        self.committed_world_state = None;
        self.committed_project_snapshot = None;
        self.last_baseline_assessment = None;
    }

    fn request_for_assessment(
        &self,
        full_input: &ModelInputIR,
        assessment: &BaselineAssessment,
    ) -> Result<ModelRequest, AgentLoopError> {
        Ok(match assessment {
            BaselineAssessment::FullRequest { .. } => ModelRequest::full(full_input.clone()),
            BaselineAssessment::AppendOnlyCandidate { delta_items } => {
                let continuation = self
                    .continuation
                    .as_ref()
                    .filter(|stored| stored.session == *self.provider_session.reference())
                    .map(|stored| stored.response_id.clone())
                    .ok_or(AgentLoopError::ContinuationIdentityMissing)?;
                let mut delta = full_input.clone();
                delta.content_items = delta_items.clone();
                ModelRequest::incremental(delta, continuation)
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
        exposed_tool_names: &std::collections::BTreeSet<crate::domain::tools::ToolName>,
        cancellation: &AgentCancellation,
    ) -> Result<ScheduledCalls, AgentLoopError> {
        let mut prepared = Vec::with_capacity(calls.len());
        let mut plans = Vec::new();
        for call in calls {
            let execution_id = ExecutionId::new(format!("execution:{}", call.call_id));
            if !exposed_tool_names.contains(&call.name) {
                prepared.push(PreparedToolCall::PlanningFailure(ToolResult::failed(
                    &call,
                    ToolExecutorError::ToolNotExposed {
                        tool_name: call.name.to_string(),
                    }
                    .to_string(),
                )));
                continue;
            }
            match self.tools.plan_exposed(execution_id, call.clone()) {
                Ok(plan) => {
                    plans.push(plan);
                    prepared.push(PreparedToolCall::Planned);
                }
                Err(error) => prepared.push(PreparedToolCall::PlanningFailure(ToolResult::failed(
                    &call,
                    error.to_string(),
                ))),
            }
        }

        let scheduled = execute_batch(
            self.tools,
            plans,
            cancellation,
            &mut self.completed_idempotency,
        )
        .await;
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
    PromptComposition(#[from] ModeError),
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
    Mode(#[from] ModeError),
    #[error(transparent)]
    WorldState(#[from] WorldStateError),
    #[error(transparent)]
    Configuration(#[from] AgentLoopConfigurationError),
    #[error("AgentLoop History does not contain the current Turn input")]
    ContextStateMismatch,
    #[error("the model completed without text or a Tool Call")]
    EmptyModelResponse,
    #[error("an incremental-capable Provider completed without a continuation identity")]
    ContinuationIdentityMissing,
    #[error("the selected RequestAudit delivery does not match the actual Provider request")]
    RequestAuditDeliveryMismatch,
    #[error("RequestAudit commit failed: {message}")]
    RequestAuditCommit { message: String },
    #[error("the Agent reached the per-Turn model-request limit of {limit}")]
    ModelRequestLimit { limit: usize },
    #[error("the Tool scheduler returned fewer outcomes than staged plans")]
    SchedulerResultMissing,
    #[error("the Tool scheduler returned more outcomes than staged plans")]
    SchedulerResultCountMismatch,
}
