//! Readable projection of existing execution/durability observations.

use std::collections::BTreeMap;
use std::io::{self, Write};

use crate::domain::tools::{ToolCallId, ToolExecutionOutcome, ToolExecutionPlan, ToolResultStatus};
use crate::terminal_content::{ContentView, escaped_text};

#[derive(Default)]
pub(super) struct ToolOutput {
    next: u64,
    active: BTreeMap<ToolCallId, u64>,
}

impl ToolOutput {
    pub(super) fn started(&mut self, plan: &ToolExecutionPlan) {
        self.next += 1;
        self.active.insert(plan.call.call_id.clone(), self.next);
        let _ = writeln!(
            io::stderr().lock(),
            "\n┌ 工具 #{} · {} · 执行中\n{}",
            self.next,
            escaped_text(plan.call.name.as_str()),
            ContentView::preview(&plan.call.arguments)
        );
    }

    // Called only after the existing durable Receipt owner has succeeded.
    pub(super) fn recorded(&mut self, outcome: &ToolExecutionOutcome, reused: bool) {
        let number = self
            .active
            .remove(&outcome.result.call_id)
            .unwrap_or_else(|| {
                self.next += 1;
                self.next
            });
        let status = match outcome.receipt.status {
            ToolResultStatus::Succeeded
                if outcome
                    .result
                    .output
                    .get("partial_failure")
                    .and_then(serde_json::Value::as_bool)
                    == Some(true) =>
            {
                "完成（部分失败）"
            }
            ToolResultStatus::Succeeded => "完成",
            ToolResultStatus::Failed => "失败",
            ToolResultStatus::Cancelled => "取消",
        };
        let reuse = if reused {
            " · 复用已有收据（未再次执行）"
        } else if outcome.receipt.cache_source.is_some() {
            " · 缓存结果"
        } else {
            ""
        };
        let _ = writeln!(
            io::stderr().lock(),
            "└ 工具 #{number} · {} · {status}{reuse}\n{}",
            escaped_text(outcome.result.tool_name.as_str()),
            ContentView::preview(&outcome.result.output)
        );
    }
}
