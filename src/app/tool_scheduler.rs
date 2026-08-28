//! One Turn-local Resource Key scheduler for the single AgentLoop.

use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::pin::Pin;

use futures_util::future::{Either, select};
use futures_util::stream::{FuturesUnordered, StreamExt};

use crate::domain::tools::{
    IdempotencyKey, ToolExecutionOutcome, ToolExecutionPlan, ToolResultStatus, ToolRetryPolicy,
    scheduling_conflicts,
};
use crate::ports::tool::ToolExecutor;

use super::cancellation::AgentCancellation;

type ActiveFuture<'tools> =
    Pin<Box<dyn Future<Output = (usize, ToolExecutionOutcome)> + Send + 'tools>>;

pub(super) struct ScheduledToolBatch {
    pub(super) outcomes: Vec<ToolExecutionOutcome>,
    pub(super) cancelled: bool,
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
) -> ScheduledToolBatch {
    let mut outcomes = vec![None; plans.len()];
    let mut followers = vec![Vec::new(); plans.len()];
    let mut leaders = BTreeMap::<IdempotencyKey, usize>::new();
    let mut pending = VecDeque::new();

    for (index, plan) in plans.iter().enumerate() {
        if let Some(outcome) = completed_outcome(plan, completed_receipts) {
            outcomes[index] = Some(outcome);
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

    while !cancelled && (!pending.is_empty() || !running.is_empty()) {
        while let Some(position) = pending
            .iter()
            .position(|index| can_start(&plans[*index], active.values()))
        {
            let Some(index) = pending.remove(position) else {
                break;
            };
            let plan = &plans[index];
            active.insert(index, plan.clone());
            let execution = tools.execute(plan.clone());
            running.push(Box::pin(async move { (index, execution.await) }));
        }

        if running.is_empty() {
            break;
        }

        let next_completion = running.next();
        let wait_for_cancellation = cancellation.wait_for_cancellation();
        futures_util::pin_mut!(next_completion, wait_for_cancellation);
        match select(next_completion, wait_for_cancellation).await {
            Either::Left((Some((index, outcome)), _)) => {
                active.remove(&index);
                if outcome.receipt.status != ToolResultStatus::Cancelled
                    && let Some(key) = completion_key(&plans[index])
                {
                    completed_receipts.insert(key, outcome.clone());
                }
                for follower in &followers[index] {
                    let follower_outcome =
                        ToolExecutionOutcome::deduplicated_for(&plans[*follower], &outcome);
                    outcomes[*follower] = Some(follower_outcome);
                }
                outcomes[index] = Some(outcome);
            }
            Either::Left((None, _)) => break,
            Either::Right(((), _)) => cancelled = true,
        }
    }

    if cancelled {
        // Dropping `running` cancels adapter futures (including child
        // processes configured with kill-on-drop). Every staged Tool Call is
        // still paired before the AgentLoop returns its cancellation outcome.
        for (index, outcome) in outcomes.iter_mut().enumerate() {
            if outcome.is_none() {
                let cancelled =
                    ToolExecutionOutcome::cancelled(plans[index].clone(), "agent_cancelled");
                *outcome = Some(cancelled);
            }
        }
    }

    let outcomes = outcomes
        .into_iter()
        .map(|outcome| outcome.expect("the scheduler completes or cancels every staged Tool plan"))
        .collect();
    ScheduledToolBatch {
        outcomes,
        cancelled,
    }
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
