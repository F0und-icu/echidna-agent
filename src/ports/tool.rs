//! Provider-neutral local Tool execution boundary.

use std::future::Future;
use std::pin::Pin;

use thiserror::Error;

use crate::domain::tools::{
    DeferredActivationSet, ExecutionId, ResolvedToolPlan, ToolCall, ToolExecutionOutcome,
    ToolExecutionPlan, ToolExposurePlan, ToolSpec,
};

pub type ToolExecutionFuture<'executor> = Pin<
    Box<dyn Future<Output = Result<ToolExecutionOutcome, ToolExecutionError>> + Send + 'executor>,
>;

pub trait ToolExecutor: Send + Sync {
    /// Optional fixed child-checkpoint path; non-command executors retain the
    /// ordinary execution contract. Only the owning App acknowledges durability.
    fn execute_observed(
        &self,
        plan: ToolExecutionPlan,
        _observer: ToolObservationSender,
    ) -> ToolExecutionFuture<'_> {
        self.execute(plan)
    }
    /// Returns the immutable canonical catalog. Exposure resolution is pure
    /// and never stored by the executor.
    fn catalog(&self) -> Vec<ToolSpec> {
        self.specs()
    }

    fn resolve_tool_plan(
        &self,
        exposure: &ToolExposurePlan,
        activations: &DeferredActivationSet,
    ) -> Result<ResolvedToolPlan, ToolExecutorError> {
        ResolvedToolPlan::resolve(self.catalog(), exposure, activations).map_err(
            |error| match error {
                crate::domain::tools::ToolContractError::UnknownToolExposure { tool_name } => {
                    ToolExecutorError::UnknownTool { tool_name }
                }
                error => ToolExecutorError::ResolvedPlanFailure {
                    message: error.to_string(),
                },
            },
        )
    }

    /// Compatibility inspection for the default immutable plan. Callers
    /// selecting Domain/Phase exposure must resolve an explicit plan.
    fn specs(&self) -> Vec<ToolSpec> {
        self.resolve_tool_plan(
            &ToolExposurePlan::default(),
            &DeferredActivationSet::default(),
        )
        .expect("an executor's canonical catalog must resolve its default Tool plan")
        .advertised_specs()
        .to_vec()
    }

    fn plan(
        &self,
        execution_id: ExecutionId,
        call: ToolCall,
    ) -> Result<ToolExecutionPlan, ToolExecutorError>;

    /// Plans a model-originated call exclusively through the immutable plan
    /// which advertised it. Implementations must not fall back to a raw or
    /// newer catalog.
    fn plan_model_call(
        &self,
        resolved: &ResolvedToolPlan,
        execution_id: ExecutionId,
        call: ToolCall,
    ) -> Result<ToolExecutionPlan, ToolExecutorError> {
        if resolved.advertised_spec(&call.name).is_none() {
            return Err(ToolExecutorError::ToolNotExposed {
                tool_name: call.name.to_string(),
            });
        }
        let tool_name = call.name.to_string();
        let mut plan = self.plan(execution_id, call)?;
        plan.bind_resolved_tool_plan(resolved)
            .map_err(|error| ToolExecutorError::PlanFailure {
                tool_name,
                message: error.to_string(),
            })?;
        Ok(plan)
    }

    fn execute(&self, plan: ToolExecutionPlan) -> ToolExecutionFuture<'_>;
}

/// A terminal execution boundary that cannot be represented as a durable
/// Receipt/ToolResult pair. It is deliberately transient: the owning
/// scheduler stops the Turn before History can be extended.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ToolExecutionError {
    #[error("Tool outcome is unknown after dispatch: {safe_reason}")]
    OutcomeUnknown { safe_reason: String },
}

/// Opaque bounded handoff shared by the command Adapter and App scheduler.
#[derive(Clone)]
pub struct ToolObservationSender(pub(crate) std::sync::Arc<SubcommandHandoff>);

type SubcommandHandoff = dyn Fn(
        crate::domain::tools::SubcommandCheckpoint,
        Vec<crate::domain::tools::PendingOutputCapture>,
    ) -> Pin<Box<dyn Future<Output = Vec<crate::domain::tools::OutputCapture>> + Send>>
    + Send
    + Sync;

impl ToolObservationSender {
    pub(crate) async fn checkpoint(
        &self,
        checkpoint: crate::domain::tools::SubcommandCheckpoint,
        captures: Vec<crate::domain::tools::PendingOutputCapture>,
    ) -> Vec<crate::domain::tools::OutputCapture> {
        (self.0)(checkpoint, captures).await
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ToolExecutorError {
    /// A fixed Replay Tool has no matching historical step. The AgentLoop
    /// treats this boundary as terminal so it cannot issue another Provider
    /// request or fall back to a real Tool.
    #[error("UNMATCHED_TOOL_CALL: Tool `{tool_name}` is not present in the Replay trace")]
    UnmatchedToolCall { tool_name: String },
    #[error("Tool `{tool_name}` is not registered")]
    UnknownTool { tool_name: String },
    #[error("Tool `{tool_name}` is not exposed for the current Mode")]
    ToolNotExposed { tool_name: String },
    #[error("Tool `{tool_name}` arguments are invalid: {message}")]
    InvalidArguments { tool_name: String, message: String },
    #[error("Tool `{tool_name}` execution plan failed: {message}")]
    PlanFailure { tool_name: String, message: String },
    #[error("resolved Tool plan failed: {message}")]
    ResolvedPlanFailure { message: String },
    #[error("Tool execution plan does not match the Sampling Step Tool plan")]
    ResolvedPlanMismatch,
}
