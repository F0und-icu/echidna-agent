//! One fixed Tool planning, scheduling, publication and pairing owner.

use super::*;
use crate::app::tool_scheduler;

pub(super) struct ExecutionContext<'a, Tools: ?Sized> {
    pub(super) tools: &'a Tools,
    pub(super) output_publisher: Option<&'a output_capture::OutputPublisher<'a>>,
    pub(super) history: &'a mut ConversationHistory,
    pub(super) completed_idempotency: &'a mut BTreeMap<IdempotencyKey, ToolExecutionOutcome>,
}

pub(super) async fn execute<Tools>(
    context: ExecutionContext<'_, Tools>,
    calls: Vec<ToolCall>,
    resolved_tool_plan: &ResolvedToolPlan,
    cancellation: &AgentCancellation,
    start_stage: &mut Option<&mut ToolExecutionStartStage<'_>>,
    receipt_stage: &mut Option<&mut ToolExecutionReceiptStage<'_>>,
    durability_stage: &mut Option<&mut ChatDurabilityStage<'_>>,
) -> Result<ScheduledCalls, AgentLoopError>
where
    Tools: ToolExecutor + ?Sized,
{
    let mut prepared = Vec::with_capacity(calls.len());
    let mut plans = Vec::new();
    for call in calls {
        let execution_id = ExecutionId::for_model_call(&call.call_id);
        match context
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
    let scheduled = {
        let observe_children = durability_stage.is_some();
        let mut checkpoint_receipt = |checkpoint: tool_scheduler::ExecutionCheckpoint<'_>| {
            let outcome = match checkpoint {
                tool_scheduler::ExecutionCheckpoint::Parent(outcome) => outcome,
                tool_scheduler::ExecutionCheckpoint::Child(observation) => {
                    let publish = context.output_publisher.ok_or_else(|| {
                        AgentLoopError::ToolOutputPublication {
                            message: "child checkpoint requires an ArtifactStore".to_owned(),
                        }
                    })?;
                    observation.checkpoint.output_captures = publish(&observation.captures)?;
                    observation.checkpoint.validate()?;
                    let stage = durability_stage.as_deref_mut().ok_or_else(|| {
                        AgentLoopError::ToolOutputPublication {
                            message: "child checkpoint requires Session durability".to_owned(),
                        }
                    })?;
                    stage(ChatDurabilityCheckpoint::Subcommand(
                        &observation.checkpoint,
                    ))?;
                    return Ok(());
                }
            };
            output_capture::publish_outcome(context.output_publisher, outcome)?;
            outcome.receipt.validate_output_captures(&outcome.result)?;
            if let Some(stage) = durability_stage.as_deref_mut() {
                stage(ChatDurabilityCheckpoint::ExecutionReceipt(outcome))?;
            }
            Ok(())
        };
        let mut scheduler_receipt_stage: Option<&mut tool_scheduler::CompletionStage<'_>> =
            Some(&mut checkpoint_receipt);
        execute_batch(
            context.tools,
            plans,
            cancellation,
            &mut *context.completed_idempotency,
            start_stage,
            &mut scheduler_receipt_stage,
            observe_children,
        )
        .await?
    };
    let tool_scheduler::ScheduledToolBatch {
        outcomes: scheduled_outcomes,
        cancelled: scheduled_cancelled,
        unknown_stop,
    } = scheduled;
    if scheduled_outcomes.len() != planned_outcome_count {
        return Err(AgentLoopError::SchedulerResultCountMismatch);
    }
    let mut outcomes = scheduled_outcomes.into_iter();
    let mut executions = Vec::new();
    let result_history_start = context.history.items().len();
    let mut interrupted = false;
    for entry in prepared {
        match entry {
            PreparedToolCall::Planned => {
                let outcome = outcomes
                    .next()
                    .ok_or(AgentLoopError::SchedulerResultMissing)?;
                match outcome {
                    ScheduledToolResult::Executed(outcome) => {
                        let outcome = *outcome;
                        context.history.append_tool_result(outcome.result.clone())?;
                        executions.push(outcome);
                    }
                    ScheduledToolResult::NotStarted(result) => {
                        context.history.append_tool_result(result)?;
                    }
                    ScheduledToolResult::Interrupted => interrupted = true,
                }
            }
            PreparedToolCall::PlanningFailure(result) => {
                context.history.append_tool_result(result)?;
            }
        }
    }
    if outcomes.next().is_some() {
        return Err(AgentLoopError::SchedulerResultCountMismatch);
    }
    if let Some(stage) = durability_stage.as_deref_mut() {
        stage(ChatDurabilityCheckpoint::History(
            &context.history.items()[result_history_start..],
        ))?;
    }
    if durability_stage.is_none()
        && let Some(stage) = receipt_stage.as_deref_mut()
    {
        for outcome in &executions {
            stage(outcome)?;
        }
    }
    if let Some(safe_reason) = unknown_stop {
        return Err(AgentLoopError::ToolExecutionOutcomeUnknown { safe_reason });
    }
    if interrupted {
        return Err(AgentLoopError::ToolExecutionInterrupted);
    }
    context.history.ensure_all_tool_calls_paired()?;
    Ok(ScheduledCalls {
        executions,
        cancelled: scheduled_cancelled,
    })
}

enum PreparedToolCall {
    Planned,
    PlanningFailure(ToolResult),
}

pub(super) struct ScheduledCalls {
    pub(super) executions: Vec<ToolExecutionOutcome>,
    pub(super) cancelled: bool,
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
