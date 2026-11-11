//! One Turn-local Resource Key scheduler for the single AgentLoop.

use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::pin::Pin;

use futures_util::stream::{FuturesUnordered, StreamExt};

use crate::domain::tools::{
    IdempotencyKey, ToolExecutionOutcome, ToolExecutionPlan, ToolFailure, ToolFailureRetryability,
    ToolResult, ToolResultStatus, ToolRetryPolicy, scheduling_conflicts,
};
use crate::ports::tool::{
    ToolCancellation, ToolExecutionError, ToolExecutor, ToolObservationSender,
};

use super::agent_loop::{AgentLoopError, ToolExecutionStartStage};
use super::cancellation::AgentCancellation;

type ActiveFuture<'tools> = Pin<
    Box<
        dyn Future<Output = (usize, Result<ToolExecutionOutcome, ToolExecutionError>)>
            + Send
            + 'tools,
    >,
>;

pub(super) type CompletionStage<'a> =
    dyn FnMut(ExecutionCheckpoint<'_>) -> Result<(), AgentLoopError> + 'a;

pub(super) enum ExecutionCheckpoint<'a> {
    Parent(&'a mut ToolExecutionOutcome),
    Child(&'a mut SubcommandObservation),
}

pub(super) struct SubcommandObservation {
    pub(super) checkpoint: crate::domain::tools::SubcommandCheckpoint,
    pub(super) captures: Vec<crate::domain::tools::PendingOutputCapture>,
    acknowledge: tokio::sync::oneshot::Sender<Vec<crate::domain::tools::OutputCapture>>,
}

fn observation_sender(
    sender: tokio::sync::mpsc::Sender<SubcommandObservation>,
) -> ToolObservationSender {
    ToolObservationSender(std::sync::Arc::new(move |checkpoint, captures| {
        let sender = sender.clone();
        Box::pin(async move {
            let (acknowledge, received) = tokio::sync::oneshot::channel();
            if sender
                .send(SubcommandObservation {
                    checkpoint,
                    captures,
                    acknowledge,
                })
                .await
                .is_ok()
                && let Ok(captures) = received.await
            {
                return captures;
            }
            // Only the owning scheduler cancels a producer. Channel closure
            // cannot manufacture a Tool result or launch dependent work.
            std::future::pending().await
        })
    }))
}

pub(super) struct ScheduledToolBatch {
    pub(super) outcomes: Vec<ScheduledToolResult>,
    pub(super) cancelled: bool,
    /// A post-dispatch Tool error has no durable outcome. The AgentLoop owns
    /// the fatal return after it has recorded known completed results.
    pub(super) unknown_stop: Option<String>,
}

pub(super) enum ScheduledToolResult {
    Executed(Box<ToolExecutionOutcome>),
    NotStarted(ToolResult),
    Interrupted,
}

/// Executes one completed model sample's valid plans. Resource conflict rules
/// live here rather than in Tool adapters, so a batch Tool has no second lock
/// implementation. Results retain model-call order even when execution ends
/// in a different order. `ReplaySafe` does not cause a retry in this Slice:
/// every non-deduplicated plan is started at most once.
pub(super) async fn execute_batch<Tools: ToolExecutor + ?Sized>(
    tools: &Tools,
    plans: Vec<ToolExecutionPlan>,
    cancellation: &AgentCancellation,
    completed_receipts: &mut BTreeMap<IdempotencyKey, ToolExecutionOutcome>,
    start_stage: &mut Option<&mut ToolExecutionStartStage<'_>>,
    completion_stage: &mut Option<&mut CompletionStage<'_>>,
    observe_children: bool,
) -> Result<ScheduledToolBatch, AgentLoopError> {
    let mut outcomes = (0..plans.len()).map(|_| None).collect::<Vec<_>>();
    let mut followers = vec![Vec::new(); plans.len()];
    let mut leaders = BTreeMap::<IdempotencyKey, usize>::new();
    let mut pending = VecDeque::new();

    for (index, plan) in plans.iter().enumerate() {
        if let Some(mut outcome) = completed_outcome(plan, completed_receipts) {
            checkpoint_completion(completion_stage, &mut outcome)?;
            outcomes[index] = Some(ScheduledToolResult::Executed(Box::new(outcome)));
            continue;
        }
        if let Some(leader) = dedupe_leader(plan, index, &mut leaders) {
            followers[leader].push(index);
        } else {
            pending.push_back(index);
        }
    }

    let mut active = BTreeMap::<usize, ToolExecutionPlan>::new();
    let mut running = FuturesUnordered::<ActiveFuture<'_>>::new();
    let mut cancelled = cancellation.is_cancelled();
    let mut unknown_stop = None;
    let (sender, mut observations) = tokio::sync::mpsc::channel(1);
    let sender = observation_sender(sender);
    let mut children = Vec::new();
    let (cancel_sender, mut cancel_receiver) = tokio::sync::watch::channel(cancelled);
    let requested = cancel_receiver.clone();
    let tool_cancellation = ToolCancellation::new(move || *requested.borrow(), async move {
        // The scheduler retains the sender until every active executor settles.
        cancel_receiver
            .wait_for(|cancelled| *cancelled)
            .await
            .expect("scheduler retains the cancellation sender while executors are active");
    });

    while unknown_stop.is_none() && (!pending.is_empty() || !running.is_empty()) {
        if !cancelled && cancellation.is_cancelled() {
            cancelled = true;
            cancel_sender.send_replace(true);
        }
        while !cancelled
            && let Some(position) = pending
                .iter()
                .position(|index| can_start(&plans[*index], active.values()))
        {
            let Some(index) = pending.remove(position) else {
                break;
            };
            let plan = &plans[index];
            if let Some(stage) = start_stage.as_deref_mut() {
                stage(plan);
            }
            active.insert(index, plan.clone());
            let execution = tools.execute_interruptible(
                plan.clone(),
                observe_children.then(|| sender.clone()),
                tool_cancellation.clone(),
            );
            running.push(Box::pin(async move { (index, execution.await) }));
        }

        if running.is_empty() {
            break;
        }

        let completion = tokio::select! {
            biased;
            Some(observation) = observations.recv() => {
                checkpoint_child(observation, completion_stage, &mut children, &active)?;
                // Check cancellation before polling acknowledged producers;
                // otherwise that poll could launch the next serial child.
                continue;
            }
            _ = cancellation.wait_for_cancellation(), if !cancelled => { continue; }
            completed = running.next() => completed,
        };
        match completion {
            Some((index, result)) => {
                // Preserve accepted observations even if the executor could
                // not settle its parent execution after cancellation.
                while let Ok(observation) = observations.try_recv() {
                    checkpoint_child(observation, completion_stage, &mut children, &active)?;
                }
                let mut outcome = match result {
                    Ok(outcome) => outcome,
                    Err(ToolExecutionError::Interrupted) => {
                        active.remove(&index);
                        outcomes[index] = Some(ScheduledToolResult::Interrupted);
                        continue;
                    }
                    Err(ToolExecutionError::OutcomeUnknown { safe_reason }) => {
                        unknown_stop = Some(safe_reason);
                        break;
                    }
                };
                active.remove(&index);
                crate::domain::tools::reconcile_subcommands(&children, &mut outcome)?;
                checkpoint_completion(completion_stage, &mut outcome)?;
                if outcome.receipt.status != ToolResultStatus::Cancelled
                    && let Some(key) = completion_key(&plans[index])
                {
                    completed_receipts.insert(key, outcome.clone());
                }
                for follower in &followers[index] {
                    let mut follower_outcome =
                        ToolExecutionOutcome::deduplicated_for(&plans[*follower], &outcome);
                    checkpoint_completion(completion_stage, &mut follower_outcome)?;
                    outcomes[*follower] =
                        Some(ScheduledToolResult::Executed(Box::new(follower_outcome)));
                }
                outcomes[index] = Some(ScheduledToolResult::Executed(Box::new(outcome)));
            }
            None => break,
        }
    }

    if cancelled || unknown_stop.is_some() {
        observations.close();
        while let Ok(observation) = observations.try_recv() {
            checkpoint_child(observation, completion_stage, &mut children, &active)?;
        }
        // Dropping a future does not establish its external effect. Preserve
        // unobserved active executions as unknown, never as synthetic Receipt
        // facts. Pending plans are known not to have crossed the start point.
        drop(running);
        for index in active.keys().copied().collect::<Vec<_>>() {
            outcomes[index] = Some(ScheduledToolResult::Interrupted);
        }
        for index in pending {
            outcomes[index] = Some(ScheduledToolResult::NotStarted(not_started_result(
                &plans[index],
            )));
        }
        for (leader, follower_indexes) in followers.iter().enumerate() {
            for follower in follower_indexes {
                if outcomes[*follower].is_some() {
                    continue;
                }
                outcomes[*follower] = Some(
                    match outcomes[leader].as_ref().expect(
                        "every dedupe leader is completed, active, or pending at cancellation",
                    ) {
                        ScheduledToolResult::Executed(source) => {
                            let mut outcome =
                                ToolExecutionOutcome::deduplicated_for(&plans[*follower], source);
                            checkpoint_completion(completion_stage, &mut outcome)?;
                            ScheduledToolResult::Executed(Box::new(outcome))
                        }
                        ScheduledToolResult::NotStarted(_) | ScheduledToolResult::Interrupted => {
                            ScheduledToolResult::NotStarted(not_started_result(&plans[*follower]))
                        }
                    },
                );
            }
        }
    }

    let outcomes = outcomes
        .into_iter()
        .map(|outcome| outcome.expect("the scheduler completes or cancels every staged Tool plan"))
        .collect();
    Ok(ScheduledToolBatch {
        outcomes,
        cancelled,
        unknown_stop,
    })
}

/// The scheduler owns exactly when a per-call completed outcome crosses the
/// fixed durability stage. Actual futures checkpoint on completion; cached,
/// deduplicated outcomes checkpoint when they are created. Unobserved active
/// cancellation and pending plans never fabricate a Receipt.
fn checkpoint_completion(
    completion_stage: &mut Option<&mut CompletionStage<'_>>,
    outcome: &mut ToolExecutionOutcome,
) -> Result<(), AgentLoopError> {
    if let Some(stage) = completion_stage.as_deref_mut() {
        stage(ExecutionCheckpoint::Parent(outcome))?;
    }
    Ok(())
}

fn checkpoint_child(
    mut observation: SubcommandObservation,
    stage: &mut Option<&mut CompletionStage<'_>>,
    children: &mut Vec<crate::domain::tools::SubcommandCheckpoint>,
    active: &BTreeMap<usize, ToolExecutionPlan>,
) -> Result<(), AgentLoopError> {
    let child = &observation.checkpoint;
    if !active.values().any(|plan| {
        plan.execution_id == child.execution_id
            && plan.plan_fingerprint == child.plan_fingerprint
            && child.agrees_with_call(&plan.call)
    }) {
        return Err(crate::domain::tools::ToolContractError::InvalidSubcommandCheckpoint.into());
    }
    child.validate_after(children)?;
    let Some(stage) = stage.as_deref_mut() else {
        return Err(AgentLoopError::ToolOutputPublication {
            message: "child checkpoint has no durable consumer".to_owned(),
        });
    };
    stage(ExecutionCheckpoint::Child(&mut observation))?;
    children.push(observation.checkpoint.clone());
    let _ = observation
        .acknowledge
        .send(observation.checkpoint.output_captures);
    Ok(())
}

fn not_started_result(plan: &ToolExecutionPlan) -> ToolResult {
    ToolResult::from_pre_execution_failure(
        &plan.call,
        ToolFailure::pre_execution(
            "tool_not_started",
            ToolFailureRetryability::Terminal,
            "Tool execution was cancelled before the scheduler start boundary",
        ),
        serde_json::json!({"error": "agent_cancelled_before_start"}),
    )
}

fn completed_outcome(
    plan: &ToolExecutionPlan,
    completed_receipts: &BTreeMap<IdempotencyKey, ToolExecutionOutcome>,
) -> Option<ToolExecutionOutcome> {
    let key = dedupe_key(plan)?;
    completed_receipts
        .get(&key)
        .map(|source| ToolExecutionOutcome::deduplicated_for(plan, source))
}

fn dedupe_key(plan: &ToolExecutionPlan) -> Option<IdempotencyKey> {
    if plan.rerun_reason.is_some() {
        return None;
    }
    completion_key(plan)
}

fn completion_key(plan: &ToolExecutionPlan) -> Option<IdempotencyKey> {
    (plan.retry_policy == ToolRetryPolicy::DedupeByKey)
        .then(|| plan.idempotency_key.clone())
        .flatten()
}

fn dedupe_leader(
    plan: &ToolExecutionPlan,
    index: usize,
    leaders: &mut BTreeMap<IdempotencyKey, usize>,
) -> Option<usize> {
    let key = dedupe_key(plan)?;
    let existing = leaders.get(&key).copied();
    leaders.entry(key).or_insert(index);
    existing
}

fn can_start<'active>(
    candidate: &ToolExecutionPlan,
    mut active: impl Iterator<Item = &'active ToolExecutionPlan>,
) -> bool {
    active.all(|current| {
        !scheduling_conflicts(
            candidate.parallelism,
            candidate.effect,
            &candidate.resolved_resource_keys,
            current.parallelism,
            current.effect,
            &current.resolved_resource_keys,
        )
    })
}
