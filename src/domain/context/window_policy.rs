//! One explicit capacity policy for planning and tool composition.

use serde::{Deserialize, Serialize};

use super::{ContextBudget, ContextPlannerError};
use crate::domain::model::ModelCapabilities;

/// Percentages are integer basis points. Content shares apply to the usable
/// parent input after both reservations; byte limits use the planner's 4-byte
/// token estimate and the planner still checks the final encoded request.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct ContextBudgetPolicy {
    pub(crate) output_reserve_bps: u16,
    pub(crate) tool_loop_reserve_bps: u16,
    pub(crate) pressure_bps: u16,
    pub(crate) target_bps: u16,
    pub(crate) summary_bps: u16,
    pub(crate) diagnostic_bps: u16,
    pub(crate) diagnostic_task_bps: u16,
    pub(crate) diagnostic_handoff_bps: u16,
    pub(crate) tool_output_bps: u16,
    pub(crate) core_memory_bps: u16,
    pub(crate) state_section_bps: u16,
    pub(crate) continuation_field_bps: u16,
    pub(crate) continuation_goal_part_bps: u16,
    pub(crate) readback_default_bps: u16,
    pub(crate) readback_max_bps: u16,
    pub(crate) compaction_reference_items: usize,
    pub(crate) max_content_bytes: Option<usize>,
    pub(crate) max_core_file_bytes: usize,
}

impl Default for ContextBudgetPolicy {
    fn default() -> Self {
        Self {
            output_reserve_bps: 1250,
            tool_loop_reserve_bps: 1250,
            pressure_bps: 8000,
            target_bps: 5000,
            summary_bps: 200,
            diagnostic_bps: 200,
            diagnostic_task_bps: 313,
            diagnostic_handoff_bps: 313,
            tool_output_bps: 3200,
            core_memory_bps: 1600,
            state_section_bps: 10,
            continuation_field_bps: 40,
            continuation_goal_part_bps: 16,
            readback_default_bps: 100,
            readback_max_bps: 400,
            compaction_reference_items: 64,
            max_content_bytes: None,
            max_core_file_bytes: 512 * 1024,
        }
    }
}

impl ContextBudgetPolicy {
    pub(crate) fn validate(self) -> Result<(), &'static str> {
        let shares = [
            self.output_reserve_bps,
            self.tool_loop_reserve_bps,
            self.pressure_bps,
            self.target_bps,
            self.summary_bps,
            self.diagnostic_bps,
            self.diagnostic_task_bps,
            self.diagnostic_handoff_bps,
            self.tool_output_bps,
            self.core_memory_bps,
            self.state_section_bps,
            self.continuation_field_bps,
            self.continuation_goal_part_bps,
            self.readback_default_bps,
            self.readback_max_bps,
        ];
        if shares.iter().any(|share| !(1..=10_000).contains(share)) {
            return Err("basis-point shares must be 1..=10000");
        }
        if u32::from(self.output_reserve_bps) + u32::from(self.tool_loop_reserve_bps) >= 10_000 {
            return Err(
                "output and tool-loop reservations must total less than 10000 basis points",
            );
        }
        if self.target_bps > self.pressure_bps {
            return Err("target_bps must not exceed pressure_bps");
        }
        if self.readback_default_bps > self.readback_max_bps {
            return Err("readback_default_bps must not exceed readback_max_bps");
        }
        if u32::from(self.diagnostic_task_bps) + u32::from(self.diagnostic_handoff_bps) >= 10_000 {
            return Err("diagnostic task and handoff shares must leave space for observations");
        }
        if self.compaction_reference_items == 0
            || self.max_content_bytes == Some(0)
            || self.max_core_file_bytes == 0
        {
            return Err("reference count and configured byte caps must be positive");
        }
        Ok(())
    }

    pub(crate) fn for_capabilities(
        self,
        capabilities: &ModelCapabilities,
    ) -> Result<ContextWindowPolicy, ContextPlannerError> {
        let output_limit = capabilities.resolve_output_token_limit(None)?.unwrap_or(0);
        let budget = ContextBudget::with_policy(
            capabilities.context_window,
            share(capabilities.context_window, self.output_reserve_bps).max(output_limit),
            share(capabilities.context_window, self.tool_loop_reserve_bps),
            self.pressure_bps,
            self.target_bps,
        )?;
        if output_limit > 0 && budget.input_token_limit() == 0 {
            return Err(ContextPlannerError::InvalidBudget {
                context_window_tokens: budget.context_window_tokens(),
                reserved_output_tokens: budget.reserved_output_tokens(),
                reserved_tool_loop_tokens: budget.reserved_tool_loop_tokens(),
            });
        }
        Ok(self.for_budget(budget))
    }

    pub(crate) fn for_budget(self, budget: ContextBudget) -> ContextWindowPolicy {
        ContextWindowPolicy {
            budget,
            policy: self,
        }
    }
}

fn share(tokens: u32, bps: u16) -> u32 {
    (u64::from(tokens) * u64::from(bps) / 10_000) as u32
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ContextWindowPolicy {
    budget: ContextBudget,
    policy: ContextBudgetPolicy,
}

impl ContextWindowPolicy {
    pub(crate) fn for_window(window_tokens: u32) -> Self {
        let policy = ContextBudgetPolicy::default();
        let budget = ContextBudget::with_policy(
            window_tokens,
            share(window_tokens, policy.output_reserve_bps),
            share(window_tokens, policy.tool_loop_reserve_bps),
            policy.pressure_bps,
            policy.target_bps,
        )
        .expect("default percentage reservations fit every window");
        Self { budget, policy }
    }

    pub(crate) fn budget(self) -> ContextBudget {
        self.budget
    }

    fn content_bytes(self, bps: u16) -> usize {
        // Round a fractional byte up so a valid nonzero share remains usable;
        // the final encoded request is still checked against the hard budget.
        let numerator = u64::from(self.budget.input_token_limit())
            * super::planner::ESTIMATED_BYTES_PER_TOKEN as u64
            * u64::from(bps);
        let bytes = numerator.div_ceil(10_000);
        let bytes = usize::try_from(bytes).unwrap_or(usize::MAX);
        self.policy
            .max_content_bytes
            .map_or(bytes, |limit| bytes.min(limit))
    }

    pub(crate) fn tool_output_bytes(self) -> usize {
        self.content_bytes(self.policy.tool_output_bps)
    }
    pub(crate) fn core_memory_bytes(self) -> usize {
        self.content_bytes(self.policy.core_memory_bps)
            .min(self.policy.max_core_file_bytes)
    }
    pub(crate) fn core_file_bytes(self) -> usize {
        self.policy.max_core_file_bytes
    }
    pub(crate) fn summary_bytes(self) -> usize {
        self.content_bytes(self.policy.summary_bps)
    }
    pub(crate) fn diagnostic_bytes(self) -> usize {
        self.content_bytes(self.policy.diagnostic_bps)
    }
    pub(crate) fn diagnostic_task_bytes(self) -> usize {
        self.diagnostic_input_bytes(self.policy.diagnostic_task_bps)
    }
    pub(crate) fn diagnostic_handoff_bytes(self) -> usize {
        self.diagnostic_input_bytes(self.policy.diagnostic_handoff_bps)
    }
    fn diagnostic_input_bytes(self, bps: u16) -> usize {
        ((self.diagnostic_bytes() as u128 * u128::from(bps)).div_ceil(10_000)) as usize
    }
    pub(crate) fn state_section_bytes(self) -> usize {
        self.content_bytes(self.policy.state_section_bps)
    }
    pub(crate) fn continuation_field_bytes(self) -> usize {
        self.content_bytes(self.policy.continuation_field_bps)
    }
    pub(crate) fn continuation_goal_part_bytes(self) -> usize {
        self.content_bytes(self.policy.continuation_goal_part_bps)
    }
    pub(crate) fn compaction_reference_items(self) -> usize {
        self.policy.compaction_reference_items
    }
    pub(crate) fn readback_default_bytes(self) -> usize {
        self.content_bytes(self.policy.readback_default_bps)
    }
    pub(crate) fn readback_max_bytes(self) -> usize {
        self.content_bytes(self.policy.readback_max_bps)
    }
}
