//! Bounded single-Agent Tool Call -> Tool Result -> full-resample loop.

use std::collections::BTreeMap;

use thiserror::Error;

use crate::domain::context::{
    BaselineAssessment, BaselineInvalidationReason, ContextError, InternalRequestMetadata,
    ModelIdentity, ModelProfile, ProviderSession, ProviderSessionId, ProviderSessionRole,
    StepSnapshot, TurnContext, TurnId,
};
use crate::domain::history::{ConversationHistory, HistoryError};
use crate::domain::model::{ModelInputIR, ModelRequestRequirements};
use crate::domain::tools::{
    ExecutionId, IdempotencyKey, ToolCall, ToolExecutionOutcome, ToolExposure, ToolResult,
};
use crate::ports::model::{
    ModelProvider, ModelProviderError, ModelRequest, ModelResponseCompletion, ProviderResponseId,
};
use crate::ports::tool::ToolExecutor;

use super::cancellation::AgentCancellation;
use super::model_sample::ModelSample;
use super::model_streaming::{ModelStreamOutcome, ModelStreamingError, ModelStreamingService};
use super::tool_scheduler::execute_batch;

const MAX_MODEL_REQUESTS_PER_TURN: usize = 32;

pub struct AgentLoop<'provider, 'tools, Provider: ?Sized, Tools: ?Sized> {
    provider: &'provider Provider,
    tools: &'tools Tools,
    instructions: String,
    history: ConversationHistory,
    model: ModelIdentity,
    profile: ModelProfile,
    provider_session: ProviderSession,
    continuation: Option<CommittedContinuation>,
    completed_idempotency: BTreeMap<IdempotencyKey, ToolExecutionOutcome>,
    next_turn_sequence: u64,
    last_baseline_assessment: Option<BaselineAssessment>,
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
        Ok(Self {
            provider,
            tools,
            instructions: instructions.into(),
            history: ConversationHistory::default(),
            model,
            profile,
            provider_session,
            continuation: None,
            completed_idempotency: BTreeMap::new(),
            next_turn_sequence: 0,
            last_baseline_assessment: None,
        })
    }

    pub fn history(&self) -> &ConversationHistory {
        &self.history
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
        if cancellation.is_cancelled() {
            return Ok(AgentTurnOutcome::Cancelled {
                model_requests: 0,
                executions: Vec::new(),
            });
        }
        self.history.append_user_text(user_text);
        let tool_specs = self
            .tools
            .specs()
            .into_iter()
            .filter(|spec| spec.exposure == ToolExposure::Direct)
            .collect::<Vec<_>>();
        let capabilities = self.provider.capabilities();
        self.next_turn_sequence += 1;
        let turn = TurnContext::new(
            TurnId::new(format!("turn:{}", self.next_turn_sequence)),
            self.model.clone(),
            self.profile.clone(),
            self.provider_session.reference().clone(),
            capabilities.context_window,
            &tool_specs,
        )?;
        let mut executions = Vec::new();
        let mut provider_requests = 0;

        for request_index in 0..MAX_MODEL_REQUESTS_PER_TURN {
            self.history.ensure_all_tool_calls_paired()?;
            let input = ModelInputIR::current(
                self.instructions.clone(),
                self.history.items().to_vec(),
                ModelRequestRequirements::default(),
            )
            .with_tool_specs(tool_specs.clone());
            let snapshot = StepSnapshot::capture(
                &turn,
                request_index,
                &input,
                &capabilities,
                InternalRequestMetadata::default(),
            )?;
            let assessment = self
                .provider_session
                .assess_delivery(&snapshot, capabilities.incremental_input);
            self.last_baseline_assessment = Some(assessment.clone());
            let request = self.request_for_assessment(&input, &assessment)?;
            reserve_provider_request(&mut provider_requests)?;
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
                    self.last_baseline_assessment = Some(BaselineAssessment::FullRequest {
                        reason: BaselineInvalidationReason::ProviderRejected,
                    });
                    reserve_provider_request(&mut provider_requests)?;
                    ModelStreamingService::new(self.provider)
                        .consume(ModelRequest::full(input.clone()), cancellation.clone())
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
                .record_completed_sample(&snapshot, server_output_items)?;
            self.commit_continuation(&completion);

            if tool_calls.is_empty() {
                return Ok(AgentTurnOutcome::Completed {
                    assistant_text,
                    model_requests: provider_requests,
                    executions,
                });
            }

            let scheduled = self.execute_tool_calls(tool_calls, &cancellation).await?;
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
        cancellation: &AgentCancellation,
    ) -> Result<ScheduledCalls, AgentLoopError> {
        let mut prepared = Vec::with_capacity(calls.len());
        let mut plans = Vec::new();
        for call in calls {
            let execution_id = ExecutionId::new(format!("execution:{}", call.call_id));
            match self.tools.plan(execution_id, call.clone()) {
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

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum AgentLoopConfigurationError {
    #[error("the AgentLoop requires a primary ProviderSession")]
    PrimarySessionRequired,
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
    Context(#[from] ContextError),
    #[error("the model completed without text or a Tool Call")]
    EmptyModelResponse,
    #[error("an incremental-capable Provider completed without a continuation identity")]
    ContinuationIdentityMissing,
    #[error("the Agent reached the per-Turn model-request limit of {limit}")]
    ModelRequestLimit { limit: usize },
    #[error("the Tool scheduler returned fewer outcomes than staged plans")]
    SchedulerResultMissing,
    #[error("the Tool scheduler returned more outcomes than staged plans")]
    SchedulerResultCountMismatch,
}
