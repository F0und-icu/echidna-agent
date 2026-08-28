//! Typed, derived-only outputs for each M4-S2 evaluation mode.

use crate::domain::context::{
    ContextBudget, ContextFragment, ContextPlan, ContextPlanner, FullLogicalManifest,
};
use crate::domain::events::EventId;
use crate::domain::model::{ModelCapabilities, ModelInputIR, ModelRequestRequirements};
use crate::domain::tools::ToolSpec;
use crate::domain::trace_eval::{EvalResult, TraceReplayToolUse};

/// Caller-supplied current inputs for one derived Context comparison. The
/// historical request can be reconstructed exactly, but its original fragment
/// candidates were not a persisted fact and are never fabricated here.
pub struct ContextRecompileInput<'a> {
    pub request_audit_event_id: &'a EventId,
    pub planner: &'a ContextPlanner,
    pub fragments: Vec<ContextFragment>,
    pub budget: ContextBudget,
    pub capabilities: &'a ModelCapabilities,
    pub tool_specs: Vec<ToolSpec>,
    pub requirements: ModelRequestRequirements,
    pub prior_manifest: Option<&'a FullLogicalManifest>,
}

pub struct ContextRecompileResult {
    pub(super) evaluation: EvalResult,
    pub(super) historical_delivered_input: ModelInputIR,
    pub(super) candidate: ContextPlan,
    pub(super) candidate_matches_historical_delivery: bool,
}

impl ContextRecompileResult {
    pub(crate) fn new(
        evaluation: EvalResult,
        historical_delivered_input: ModelInputIR,
        candidate: ContextPlan,
        candidate_matches_historical_delivery: bool,
    ) -> Self {
        Self {
            evaluation,
            historical_delivered_input,
            candidate,
            candidate_matches_historical_delivery,
        }
    }

    pub const fn evaluation(&self) -> &EvalResult {
        &self.evaluation
    }

    pub fn historical_delivered_input(&self) -> &ModelInputIR {
        &self.historical_delivered_input
    }

    pub fn candidate(&self) -> &ContextPlan {
        &self.candidate
    }

    pub const fn candidate_matches_historical_delivery(&self) -> bool {
        self.candidate_matches_historical_delivery
    }
}

pub struct MechanicalReplayResult {
    pub(super) evaluation: EvalResult,
    pub(super) model_requests: usize,
    pub(super) replay_tool_uses: Vec<TraceReplayToolUse>,
}

impl MechanicalReplayResult {
    pub(crate) fn new(
        evaluation: EvalResult,
        model_requests: usize,
        replay_tool_uses: Vec<TraceReplayToolUse>,
    ) -> Self {
        Self {
            evaluation,
            model_requests,
            replay_tool_uses,
        }
    }

    pub const fn evaluation(&self) -> &EvalResult {
        &self.evaluation
    }

    pub const fn model_requests(&self) -> usize {
        self.model_requests
    }

    pub fn replay_tool_uses(&self) -> &[TraceReplayToolUse] {
        &self.replay_tool_uses
    }
}

pub struct ShadowModelEvalResult {
    pub(super) evaluation: EvalResult,
    pub(super) model_requests: usize,
    pub(super) replay_tool_uses: Vec<TraceReplayToolUse>,
}

impl ShadowModelEvalResult {
    pub(crate) fn new(
        evaluation: EvalResult,
        model_requests: usize,
        replay_tool_uses: Vec<TraceReplayToolUse>,
    ) -> Self {
        Self {
            evaluation,
            model_requests,
            replay_tool_uses,
        }
    }

    pub const fn evaluation(&self) -> &EvalResult {
        &self.evaluation
    }

    pub const fn model_requests(&self) -> usize {
        self.model_requests
    }

    pub fn replay_tool_uses(&self) -> &[TraceReplayToolUse] {
        &self.replay_tool_uses
    }
}
