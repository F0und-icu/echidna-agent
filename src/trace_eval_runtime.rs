//! Concrete Replay Adapter plus AgentLoop composition root for M4-S2.

use crate::adapters::replay::{ReplayAdapterError, ReplayModelProvider, ReplayToolExecutor};
use crate::app::agent_loop::{AgentLoop, AgentTurnOutcome};
use crate::app::cancellation::AgentCancellation;
use crate::app::trace_eval::{MechanicalReplayResult, ShadowModelEvalResult, TraceReplayRunError};
use crate::domain::model::ModelCapabilities;
use crate::domain::research_control::PromptComposition;
use crate::domain::trace_eval::{EvalResult, EvalSource, TraceReplayPlan};
use crate::ports::model::ModelProvider;

/// The concrete fixed-adapter runtime created from one App-compiled Trace.
/// Keeping this wiring at the crate composition root preserves the internal
/// `app -> ports -> domain` and `adapters -> ports/domain` directions.
pub struct PreparedTraceReplay {
    source: EvalSource,
    model: ReplayModelProvider,
    tools: ReplayToolExecutor,
    replay_prompt: PromptComposition,
}

impl PreparedTraceReplay {
    pub fn new(
        plan: TraceReplayPlan,
        capabilities: ModelCapabilities,
    ) -> Result<Self, ReplayAdapterError> {
        let (source, model_steps, tool_specs, tool_steps) = plan.into_parts();
        let first_input = model_steps
            .first()
            .ok_or(ReplayAdapterError::ModelStepsRequired)?
            .expected_input();
        let replay_prompt = PromptComposition::legacy_replay(first_input.instructions.clone())
            .map_err(|error| ReplayAdapterError::HistoricalPrompt {
                message: error.to_string(),
            })?;
        Ok(Self {
            model: ReplayModelProvider::new(capabilities, model_steps)?,
            tools: ReplayToolExecutor::new(source.clone(), tool_specs, tool_steps)?,
            source,
            replay_prompt,
        })
    }

    pub fn model_provider(&self) -> &ReplayModelProvider {
        &self.model
    }

    pub fn tool_executor(&self) -> &ReplayToolExecutor {
        &self.tools
    }

    pub async fn mechanical_replay(
        &self,
        agent: &mut AgentLoop<'_, '_, ReplayModelProvider, ReplayToolExecutor>,
        user_inputs: impl IntoIterator<Item = String>,
    ) -> Result<MechanicalReplayResult, TraceReplayRunError> {
        // Historical RequestAudit instruction bytes are the Replay contract.
        // They are queued once for this fixed evaluation agent and never
        // change the ordinary runtime's Base/Domain/Phase composition.
        agent.queue_prompt_composition(self.replay_prompt.clone());
        let mut model_requests: usize = 0;
        for user_input in user_inputs {
            match agent
                .run_turn(user_input, AgentCancellation::new())
                .await
                .map_err(TraceReplayRunError::Agent)?
            {
                AgentTurnOutcome::Completed {
                    model_requests: requests,
                    ..
                }
                | AgentTurnOutcome::Cancelled {
                    model_requests: requests,
                    ..
                } => model_requests = model_requests.saturating_add(requests),
            }
        }
        if self.model.remaining_steps() != 0 || self.tools.remaining_steps() != 0 {
            return Err(TraceReplayRunError::TraceNotFullyConsumed {
                remaining_model_steps: self.model.remaining_steps(),
                remaining_tool_steps: self.tools.remaining_steps(),
            });
        }
        Ok(MechanicalReplayResult::new(
            EvalResult::mechanical_replay(self.source.clone()),
            model_requests,
            self.tools.replay_uses(),
        ))
    }

    /// Runs one Shadow turn through a caller-selected Provider and the fixed
    /// Replay Tool adapter. Its type prevents a real Tool executor entering.
    pub async fn shadow_model_eval<Provider>(
        &self,
        agent: &mut AgentLoop<'_, '_, Provider, ReplayToolExecutor>,
        user_input: impl Into<String>,
        cancellation: AgentCancellation,
    ) -> Result<ShadowModelEvalResult, TraceReplayRunError>
    where
        Provider: ModelProvider + ?Sized,
    {
        let outcome = agent
            .run_turn(user_input, cancellation)
            .await
            .map_err(TraceReplayRunError::Agent)?;
        let model_requests = match outcome {
            AgentTurnOutcome::Completed { model_requests, .. }
            | AgentTurnOutcome::Cancelled { model_requests, .. } => model_requests,
        };
        if self.tools.remaining_steps() != 0 {
            return Err(TraceReplayRunError::ReplayToolsNotFullyConsumed {
                remaining_tool_steps: self.tools.remaining_steps(),
            });
        }
        Ok(ShadowModelEvalResult::new(
            EvalResult::shadow_model_eval(self.source.clone()),
            model_requests,
            self.tools.replay_uses(),
        ))
    }
}
