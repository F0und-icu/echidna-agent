//! Readable projection of existing execution/durability observations.

use std::collections::BTreeMap;
use std::fmt::Write;
use std::time::Instant;

use crate::domain::tools::{
    OutputCapture, OutputStream, SubcommandCheckpoint, ToolCallId, ToolExecutionOutcome,
    ToolExecutionPlan, ToolResultStatus,
};
use crate::terminal_content::{ContentView, escaped_text};

mod presentation_sink;
use presentation_sink::PresentationSink;

#[derive(Default)]
pub(super) struct ToolOutput {
    next: u64,
    active: BTreeMap<ToolCallId, ActiveTool>,
    sink: PresentationSink,
}

struct ActiveTool {
    number: u64,
    name: String,
    started: Instant,
    deadline_ms: u64,
    commands: Vec<String>,
}

impl ActiveTool {
    fn timing(&self) -> String {
        format!(
            "耗时 {:.1}s / deadline {:.1}s",
            self.started.elapsed().as_secs_f64(),
            self.deadline_ms as f64 / 1000.0
        )
    }
}

impl ToolOutput {
    pub(super) fn started(&mut self, plan: &ToolExecutionPlan) {
        self.next += 1;
        self.active.insert(
            plan.call.call_id.clone(),
            ActiveTool {
                number: self.next,
                name: escaped_text(plan.call.name.as_str()),
                started: Instant::now(),
                deadline_ms: plan.timeout_ms,
                commands: if plan.call.name.as_str() == "run_commands" {
                    plan.call
                        .arguments
                        .get("commands")
                        .and_then(serde_json::Value::as_array)
                        .into_iter()
                        .flatten()
                        .map(|command| ContentView::preview(&command["argv"]))
                        .collect()
                } else {
                    Vec::new()
                },
            },
        );
        self.sink.send(format!(
            "\n┌ 工具 #{} · {} · 执行中（结果尚未保存） · deadline {:.1}s\n{}\n",
            self.next,
            escaped_text(plan.call.name.as_str()),
            plan.timeout_ms as f64 / 1000.0,
            ContentView::preview(&plan.call.arguments)
        ));
    }

    // Invoked only after the real Session child Event has been appended.
    pub(super) fn checkpointed(&mut self, checkpoint: &SubcommandCheckpoint) {
        let Some(active) = self.active.get(&checkpoint.call_id) else {
            return;
        };
        let status = match checkpoint.status {
            ToolResultStatus::Succeeded => "成功",
            ToolResultStatus::Failed => "失败",
            ToolResultStatus::Cancelled => return,
        };
        let command = active
            .commands
            .get(checkpoint.index as usize)
            .map(String::as_str)
            .unwrap_or("");
        self.sink.send(format!(
            "\n├ 工具 #{} · 子命令 {}/{} · 已保存（{status}；非批次终态） · {}\n命令：\n{command}\n{}\n{}",
            active.number, checkpoint.index as usize + 1, checkpoint.child_count,
            active.timing(), ContentView::preview(&checkpoint.result),
            capture_hints(&checkpoint.output_captures)
        ));
    }

    pub(super) fn finish_turn(&mut self) {
        for (_, active) in std::mem::take(&mut self.active) {
            self.sink.send(format!(
                "\n└ 工具 #{} · {} · 状态未知（未观察到批次终态） · {}\n已保存子项以 Session 为准；不自动重跑。\n",
                active.number, active.name, active.timing()
            ));
        }
    }

    // Called only after the existing durable Receipt owner has succeeded.
    pub(super) fn recorded(&mut self, outcome: &ToolExecutionOutcome, reused: bool) {
        let active = self.active.remove(&outcome.result.call_id);
        let timing = active.as_ref().map(ActiveTool::timing).unwrap_or_default();
        let number = active.map(|active| active.number).unwrap_or_else(|| {
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
        self.sink.send(format!(
            "└ 工具 #{number} · {} · {status}{reuse} · {timing}\n{}\n{}",
            escaped_text(outcome.result.tool_name.as_str()),
            ContentView::preview(&outcome.result.output),
            capture_hints(&outcome.receipt.output_captures)
        ));
    }
}

// Only typed durable observations supply readback hints. No Blob reads here.
fn capture_hints(captures: &[OutputCapture]) -> String {
    let mut view = String::new();
    for capture in captures.iter().take(4) {
        let stream = match capture.stream {
            OutputStream::Stdout => "stdout",
            OutputStream::Stderr => "stderr",
            OutputStream::Pty => "pty",
        };
        let child = capture
            .child_index
            .map(|index| format!(" 子命令 {}", usize::from(index) + 1))
            .unwrap_or_default();
        let completeness = if capture.complete {
            "已完整捕获"
        } else {
            "捕获不完整；未保留字节不可回读"
        };
        let _ = writeln!(
            view,
            "原始输出{child} {stream}：保留 {}/已观察 {} bytes · {completeness}\n  read_output blob_id={} offset=0",
            capture.retained_bytes,
            capture.observed_bytes,
            capture.blob_id.as_str()
        );
    }
    if captures.len() > 4 {
        let _ = writeln!(
            view,
            "[另有 {} 个捕获引用，见 Session 记录]",
            captures.len() - 4
        );
    }
    view
}
