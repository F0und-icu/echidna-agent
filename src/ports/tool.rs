//! Provider-neutral local Tool execution boundary.

use std::future::Future;
use std::pin::Pin;

use thiserror::Error;

use crate::domain::tools::{
    ExecutionId, ToolCall, ToolExecutionOutcome, ToolExecutionPlan, ToolSpec,
};

pub type ToolExecutionFuture<'executor> =
    Pin<Box<dyn Future<Output = ToolExecutionOutcome> + Send + 'executor>>;

pub trait ToolExecutor: Send + Sync {
    fn specs(&self) -> Vec<ToolSpec>;

    fn plan(
        &self,
        execution_id: ExecutionId,
        call: ToolCall,
    ) -> Result<ToolExecutionPlan, ToolExecutorError>;

    fn execute(&self, plan: ToolExecutionPlan) -> ToolExecutionFuture<'_>;
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ToolExecutorError {
    #[error("Tool `{tool_name}` is not registered")]
    UnknownTool { tool_name: String },
    #[error("Tool `{tool_name}` arguments are invalid: {message}")]
    InvalidArguments { tool_name: String, message: String },
    #[error("Tool `{tool_name}` execution plan failed: {message}")]
    PlanFailure { tool_name: String, message: String },
}
