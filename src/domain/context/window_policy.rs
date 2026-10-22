//! Capacity-derived defaults shared by immutable Tool and Context composition.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ContextWindowPolicy {
    window_tokens: u32,
    content_multiplier: usize,
}

impl ContextWindowPolicy {
    pub(crate) fn for_window(window_tokens: u32) -> Self {
        Self {
            window_tokens,
            content_multiplier: (window_tokens / 131_072).clamp(1, 8) as usize,
        }
    }

    pub(crate) fn output_reserve_tokens(self) -> u32 {
        (self.window_tokens / 8).min(65_536)
    }

    pub(crate) fn tool_loop_reserve_tokens(self) -> u32 {
        (self.window_tokens / 8).min(131_072)
    }

    pub(crate) fn tool_output_bytes(self) -> usize {
        128 * 1024 * self.content_multiplier
    }

    pub(crate) fn core_memory_bytes(self) -> usize {
        64 * 1024 * self.content_multiplier
    }

    pub(crate) fn summary_bytes(self) -> usize {
        8 * 1024 * self.content_multiplier
    }

    pub(crate) fn state_section_bytes(self) -> usize {
        384 * self.content_multiplier
    }

    pub(crate) fn compaction_reference_items(self) -> usize {
        32 * self.content_multiplier
    }

    pub(crate) fn readback_default_bytes(self) -> usize {
        4 * 1024 * self.content_multiplier
    }

    pub(crate) fn readback_max_bytes(self) -> usize {
        16 * 1024 * self.content_multiplier
    }
}
