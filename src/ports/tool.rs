//! Provider-neutral local Tool execution boundary.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use futures_util::FutureExt;
use thiserror::Error;

use crate::domain::tools::{
    DeferredActivationSet, ExecutionId, ResolvedToolPlan, ToolCall, ToolExecutionOutcome,
    ToolExecutionPlan, ToolExposurePlan, ToolSpec,
};

pub type ToolExecutionFuture<'executor> = Pin<
    Box<dyn Future<Output = Result<ToolExecutionOutcome, ToolExecutionError>> + Send + 'executor>,
>;

/// Scheduler-owned cancellation broadcast; each active command can await it.
#[derive(Clone)]
pub struct ToolCancellation {
    requested: Arc<dyn Fn() -> bool + Send + Sync>,
    notification: futures_util::future::Shared<futures_util::future::BoxFuture<'static, ()>>,
}

impl ToolCancellation {
    pub(crate) fn new(
        requested: impl Fn() -> bool + Send + Sync + 'static,
        notification: impl Future<Output = ()> + Send + 'static,
    ) -> Self {
        Self {
            requested: Arc::new(requested),
            notification: notification.boxed().shared(),
        }
    }

    pub(crate) fn never() -> Self {
        Self::new(|| false, std::future::pending())
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        (self.requested)()
    }

    pub(crate) async fn wait(&self) {
        self.notification.clone().await;
    }

    pub(crate) async fn interrupt(
        self,
        execution: ToolExecutionFuture<'_>,
    ) -> Result<ToolExecutionOutcome, ToolExecutionError> {
        futures_util::select_biased! {
            () = self.wait().fuse() => Err(ToolExecutionError::Interrupted),
            outcome = execution.fuse() => outcome,
        }
    }
}

pub trait ToolExecutor: Send + Sync {
    /// Executors may settle cancellation cooperatively. The default preserves
    /// unknown effects when an execution cannot return a terminal observation.
    fn execute_interruptible(
        &self,
        plan: ToolExecutionPlan,
        observer: Option<ToolObservationSender>,
        cancellation: ToolCancellation,
    ) -> ToolExecutionFuture<'_> {
        let execution = match observer {
            Some(observer) => self.execute_observed(plan, observer),
            None => self.execute(plan),
        };
        Box::pin(cancellation.interrupt(execution))
    }
    /// A current observation from an executor that owns live Job/PTY state.
    /// None means this executor does not provide that observation.
    fn live_processes(&self) -> Option<Vec<crate::domain::tools::LiveProcessObservation>> {
        None
    }
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
    #[error("Tool execution interrupted without a terminal observation")]
    Interrupted,
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
