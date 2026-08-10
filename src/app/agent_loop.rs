//! Bounded single-Agent Tool Call -> Tool Result -> full-resample loop.

use thiserror::Error;

use crate::domain::history::{ConversationHistory, HistoryError};
use crate::domain::model::{ModelEvent, ModelInputIR, ModelRequestRequirements};
use crate::domain::tools::{ExecutionId, ToolCall, ToolExecutionOutcome, ToolExposure, ToolResult};
use crate::ports::model::ModelProvider;
use crate::ports::tool::ToolExecutor;

use super::cancellation::{AgentCancellation, run_until_cancelled};
use super::model_streaming::{ModelStreamOutcome, ModelStreamingError, ModelStreamingService};

const MAX_MODEL_REQUESTS_PER_TURN: usize = 32;

pub struct AgentLoop<'provider, 'tools, Provider: ?Sized, Tools: ?Sized> {
    provider: &'provider Provider,
    tools: &'tools Tools,
    instructions: String,
    history: ConversationHistory,
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
        Self {
            provider,
            tools,
            instructions: instructions.into(),
            history: ConversationHistory::default(),
        }
    }

    pub fn history(&self) -> &ConversationHistory {
        &self.history
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
        let mut executions = Vec::new();

        for request_index in 0..MAX_MODEL_REQUESTS_PER_TURN {
            self.history.ensure_all_tool_calls_paired()?;
            let input = ModelInputIR::current(
                self.instructions.clone(),
                self.history.items().to_vec(),
                ModelRequestRequirements::default(),
            )
            .with_tool_specs(tool_specs.clone());
            let stream = ModelStreamingService::new(self.provider)
                .consume(input, cancellation.clone())
                .await?;
            let events = match stream {
                ModelStreamOutcome::Completed { events } => events,
                ModelStreamOutcome::Cancelled { .. } => {
                    return Ok(AgentTurnOutcome::Cancelled {
                        model_requests: request_index + 1,
                        executions,
                    });
                }
            };
            let sample = ModelSample::from_events(events);
            let tool_call = sample.single_tool_call()?;
            sample.commit(&mut self.history)?;

            let Some(call) = tool_call else {
                let assistant_text = sample.assistant_text();
                if assistant_text.is_empty() {
                    return Err(AgentLoopError::EmptyModelResponse);
                }
                return Ok(AgentTurnOutcome::Completed {
                    assistant_text,
                    model_requests: request_index + 1,
                    executions,
                });
            };

            let execution_id = ExecutionId::new(format!("execution:{}", call.call_id));
            let plan = match self.tools.plan(execution_id, call.clone()) {
                Ok(plan) => plan,
                Err(error) => {
                    self.history
                        .append_tool_result(ToolResult::failed(&call, error.to_string()))?;
                    continue;
                }
            };
            let outcome =
                run_until_cancelled(self.tools.execute(plan.clone()), &cancellation).await;
            match outcome {
                Some(outcome) => {
                    self.history.append_tool_result(outcome.result.clone())?;
                    executions.push(outcome);
                }
                None => {
                    let outcome = ToolExecutionOutcome::cancelled(plan, "agent_cancelled");
                    self.history.append_tool_result(outcome.result.clone())?;
                    executions.push(outcome);
                    return Ok(AgentTurnOutcome::Cancelled {
                        model_requests: request_index + 1,
                        executions,
                    });
                }
            }
        }

        Err(AgentLoopError::ModelRequestLimit {
            limit: MAX_MODEL_REQUESTS_PER_TURN,
        })
    }
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
    #[error("one model response requested {count} Tools; M1a supports exactly one at a time")]
    MultipleToolCalls { count: usize },
    #[error("the model completed without text or a Tool Call")]
    EmptyModelResponse,
    #[error("the Agent reached the per-Turn model-request limit of {limit}")]
    ModelRequestLimit { limit: usize },
}

enum SampleItem {
    AssistantText(String),
    ToolCall(ToolCall),
}

struct ModelSample {
    items: Vec<SampleItem>,
}

impl ModelSample {
    fn from_events(events: Vec<ModelEvent>) -> Self {
        let mut items = Vec::new();
        let mut text = String::new();
        for event in events {
            match event {
                ModelEvent::TextDelta { text: delta } => text.push_str(&delta),
                ModelEvent::ToolCall { call } => {
                    if !text.is_empty() {
                        items.push(SampleItem::AssistantText(std::mem::take(&mut text)));
                    }
                    items.push(SampleItem::ToolCall(call));
                }
                ModelEvent::Usage { .. } | ModelEvent::Completed => {}
            }
        }
        if !text.is_empty() {
            items.push(SampleItem::AssistantText(text));
        }
        Self { items }
    }

    fn single_tool_call(&self) -> Result<Option<ToolCall>, AgentLoopError> {
        let calls = self
            .items
            .iter()
            .filter_map(|item| match item {
                SampleItem::ToolCall(call) => Some(call),
                SampleItem::AssistantText(_) => None,
            })
            .collect::<Vec<_>>();
        match calls.as_slice() {
            [] => Ok(None),
            [call] => Ok(Some((*call).clone())),
            _ => Err(AgentLoopError::MultipleToolCalls { count: calls.len() }),
        }
    }

    fn commit(&self, history: &mut ConversationHistory) -> Result<(), HistoryError> {
        let mut staged = history.clone();
        for item in &self.items {
            match item {
                SampleItem::AssistantText(text) => staged.append_assistant_text(text.clone()),
                SampleItem::ToolCall(call) => staged.append_tool_call(call.clone())?,
            }
        }
        *history = staged;
        Ok(())
    }

    fn assistant_text(&self) -> String {
        self.items
            .iter()
            .filter_map(|item| match item {
                SampleItem::AssistantText(text) => Some(text.as_str()),
                SampleItem::ToolCall(_) => None,
            })
            .collect()
    }
}
