//! Fixed Tool-side semantic samples for Trace Replay.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;

use crate::domain::canonical_json::canonical_blake3_hash;
use crate::domain::tools::{
    DeferredActivationSet, ExecutionId, ResolvedToolPlan, ToolCall, ToolExecutionOutcome,
    ToolExecutionPlan, ToolExposure, ToolExposurePlan, ToolName, ToolResultStatus, ToolSpec,
};
use crate::domain::trace_eval::{EvalSource, TraceReplayToolStep, TraceReplayToolUse};
use crate::ports::tool::{ToolExecutionFuture, ToolExecutor, ToolExecutorError};

use super::ReplayAdapterError;

struct ReplayToolState {
    remaining_steps: VecDeque<TraceReplayToolStep>,
    planned_steps: BTreeMap<ExecutionId, TraceReplayToolStep>,
    uses: Vec<TraceReplayToolUse>,
}

/// A fixed `ToolExecutor` that only replays the next declared Tool call. It
/// cannot hold a real Tool adapter or a fallback path.
pub struct ReplayToolExecutor {
    source: EvalSource,
    specs: BTreeMap<ToolName, ToolSpec>,
    fixed_plan: ResolvedToolPlan,
    state: Mutex<ReplayToolState>,
}

impl ReplayToolExecutor {
    pub fn new(
        source: EvalSource,
        specs: impl IntoIterator<Item = ToolSpec>,
        steps: impl IntoIterator<Item = TraceReplayToolStep>,
    ) -> Result<Self, ReplayAdapterError> {
        let mut replay_specs = BTreeMap::new();
        for spec in specs {
            if spec.exposure != ToolExposure::Direct {
                return Err(ReplayAdapterError::NonDirectToolSpec {
                    tool_name: spec.name.to_string(),
                });
            }
            if replay_specs.insert(spec.name.clone(), spec).is_some() {
                return Err(ReplayAdapterError::DuplicateToolSpec);
            }
        }
        let steps = steps.into_iter().collect::<VecDeque<_>>();
        for step in &steps {
            if !replay_specs.contains_key(&step.expected_call().name) {
                return Err(ReplayAdapterError::ToolStepSpecMissing {
                    tool_name: step.expected_call().name.to_string(),
                });
            }
        }
        let fixed_plan = ResolvedToolPlan::resolve(
            replay_specs.values().cloned(),
            &ToolExposurePlan::default(),
            &DeferredActivationSet::default(),
        )
        .map_err(|_| ReplayAdapterError::DuplicateToolSpec)?;
        Ok(Self {
            source,
            specs: replay_specs,
            fixed_plan,
            state: Mutex::new(ReplayToolState {
                remaining_steps: steps,
                planned_steps: BTreeMap::new(),
                uses: Vec::new(),
            }),
        })
    }

    pub fn replay_uses(&self) -> Vec<TraceReplayToolUse> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .uses
            .clone()
    }

    pub fn remaining_steps(&self) -> usize {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.remaining_steps.len() + state.planned_steps.len()
    }

    fn plan_replay(
        &self,
        execution_id: ExecutionId,
        call: ToolCall,
        resolved: Option<&ResolvedToolPlan>,
    ) -> Result<ToolExecutionPlan, ToolExecutorError> {
        let spec = resolved
            .unwrap_or(&self.fixed_plan)
            .advertised_spec(&call.name)
            .ok_or_else(|| ToolExecutorError::UnmatchedToolCall {
                tool_name: call.name.to_string(),
            })?;
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let expected =
            state
                .remaining_steps
                .front()
                .ok_or_else(|| ToolExecutorError::UnmatchedToolCall {
                    tool_name: call.name.to_string(),
                })?;
        if expected.expected_call().name != call.name
            || call_semantic_hash(expected.expected_call()) != call_semantic_hash(&call)
        {
            return Err(ToolExecutorError::UnmatchedToolCall {
                tool_name: call.name.to_string(),
            });
        }
        let mut plan = ToolExecutionPlan::new(execution_id.clone(), call, spec, Vec::new())
            .map_err(|error| ToolExecutorError::PlanFailure {
                tool_name: spec.name.to_string(),
                message: error.to_string(),
            })?;
        if let Some(resolved) = resolved {
            plan.bind_resolved_tool_plan(resolved).map_err(|error| {
                ToolExecutorError::PlanFailure {
                    tool_name: spec.name.to_string(),
                    message: error.to_string(),
                }
            })?;
        }
        let step = state
            .remaining_steps
            .pop_front()
            .expect("the checked Replay Tool step is present");
        state.planned_steps.insert(execution_id, step);
        Ok(plan)
    }
}

impl ToolExecutor for ReplayToolExecutor {
    fn catalog(&self) -> Vec<ToolSpec> {
        self.specs.values().cloned().collect()
    }

    fn resolve_tool_plan(
        &self,
        _exposure: &ToolExposurePlan,
        _activations: &DeferredActivationSet,
    ) -> Result<ResolvedToolPlan, ToolExecutorError> {
        Ok(self.fixed_plan.clone())
    }

    fn plan(
        &self,
        execution_id: ExecutionId,
        call: ToolCall,
    ) -> Result<ToolExecutionPlan, ToolExecutorError> {
        self.plan_replay(execution_id, call, None)
    }

    fn plan_model_call(
        &self,
        resolved: &ResolvedToolPlan,
        execution_id: ExecutionId,
        call: ToolCall,
    ) -> Result<ToolExecutionPlan, ToolExecutorError> {
        if resolved != &self.fixed_plan {
            return Err(ToolExecutorError::ResolvedPlanMismatch);
        }
        self.plan_replay(execution_id, call, Some(resolved))
    }

    fn execute(&self, plan: ToolExecutionPlan) -> ToolExecutionFuture<'_> {
        Box::pin(async move {
            let step = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .planned_steps
                .remove(&plan.execution_id);
            let Some(step) = step else {
                return ToolExecutionOutcome::failed(
                    plan,
                    "replay_step_missing",
                    serde_json::json!({"error": "Replay Tool plan has no staged historical result"}),
                );
            };
            let (source_receipt_event_id, source_receipt, _expected_call, source_result) =
                step.into_parts();
            let source_receipt_for_use = source_receipt.clone();
            let mut outcome = match source_result.status {
                ToolResultStatus::Succeeded => {
                    ToolExecutionOutcome::succeeded(plan, source_result.output)
                }
                ToolResultStatus::Failed => ToolExecutionOutcome::failed_preserving_output(
                    plan,
                    source_receipt
                        .exit_reason
                        .unwrap_or_else(|| "replayed_tool_failure".to_owned()),
                    source_result.output,
                ),
                ToolResultStatus::Cancelled => ToolExecutionOutcome::cancelled_with_output(
                    plan,
                    source_receipt
                        .exit_reason
                        .unwrap_or_else(|| "replayed_tool_cancelled".to_owned()),
                    source_result.output,
                ),
            };
            outcome.receipt.output_captures = source_receipt_for_use.output_captures.clone();
            self.state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .uses
                .push(TraceReplayToolUse::new(
                    self.source.clone(),
                    source_receipt_event_id,
                    source_receipt_for_use,
                    outcome.receipt.clone(),
                ));
            outcome
        })
    }
}

fn call_semantic_hash(call: &ToolCall) -> String {
    canonical_blake3_hash(&(&call.name, &call.arguments))
        .expect("ToolCall is always JSON serializable")
}
