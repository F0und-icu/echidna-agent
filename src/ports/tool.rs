//! Provider-neutral local Tool execution boundary.

use std::future::Future;
use std::pin::Pin;

use thiserror::Error;

use crate::domain::tools::{
    ExecutionId, ToolCall, ToolExecutionOutcome, ToolExecutionPlan, ToolExposurePlan, ToolSpec,
};

pub type ToolExecutionFuture<'executor> =
    Pin<Box<dyn Future<Output = ToolExecutionOutcome> + Send + 'executor>>;

pub trait ToolExecutor: Send + Sync {
    /// Returns the schemas this executor actually permits a model to call.
    fn specs(&self) -> Vec<ToolSpec>;

    fn plan(
        &self,
        execution_id: ExecutionId,
        call: ToolCall,
    ) -> Result<ToolExecutionPlan, ToolExecutorError>;

    /// Plans a model-originated call. Executors with local typed/internal
    /// callers may keep those callers on [`Self::plan`], while this entry
    /// point must enforce the same exposure authority used by [`Self::specs`].
    fn plan_exposed(
        &self,
        execution_id: ExecutionId,
        call: ToolCall,
    ) -> Result<ToolExecutionPlan, ToolExecutorError> {
        self.plan(execution_id, call)
    }

    /// Atomically selects the fixed Mode exposure plan for the next model
    /// turn. Executors without a dynamic catalog retain their fixed specs.
    fn configure_exposure(&self, _plan: ToolExposurePlan) -> Result<(), ToolExecutorError> {
        Ok(())
    }

    fn execute(&self, plan: ToolExecutionPlan) -> ToolExecutionFuture<'_>;
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ToolExecutorError {
    #[error("Tool `{tool_name}` is not registered")]
    UnknownTool { tool_name: String },
    #[error("Tool `{tool_name}` is not exposed for the current Mode")]
    ToolNotExposed { tool_name: String },
    #[error("Tool `{tool_name}` arguments are invalid: {message}")]
    InvalidArguments { tool_name: String, message: String },
    #[error("Tool `{tool_name}` execution plan failed: {message}")]
    PlanFailure { tool_name: String, message: String },
}
