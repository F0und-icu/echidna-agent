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
            "elapsed {:.1}s (including persistence) / configured deadline {:.1}s",
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
        self.sink.send(&plan.call.call_id, plan.call.name.as_str(), "started", None, format!(
            "\n┌ Tool #{} · {} · running (result not yet persisted) · configured deadline {:.1}s\n{}\n",
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
            ToolResultStatus::Succeeded => "succeeded",
            ToolResultStatus::Failed => "failed",
            ToolResultStatus::Cancelled => return,
        };
        let command = active
            .commands
            .get(checkpoint.index as usize)
            .map(String::as_str)
            .unwrap_or("");
        self.sink.send(&checkpoint.call_id, &active.name, "checkpointed", Some(checkpoint.status), format!(
            "\n├ Tool #{} · subcommand {}/{} · persisted ({status}; not a terminal batch result) · {}\nCommand:\n{command}\n{}\n{}",
            active.number, checkpoint.index as usize + 1, checkpoint.child_count,
            active.timing(), ContentView::preview(&checkpoint.result),
            capture_hints(&checkpoint.output_captures)
        ));
    }

    pub(super) fn finish_turn(&mut self) {
        for (call_id, active) in std::mem::take(&mut self.active) {
            self.sink.send(&call_id, &active.name, "unknown", None, format!(
                "\n└ Tool #{} · {} · outcome unknown (no terminal batch outcome observed) · {}\nPersisted child outcomes are recorded in the Session; no automatic replay.\n",
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
                "completed (partial failure)"
            }
            ToolResultStatus::Succeeded => "completed",
            ToolResultStatus::Failed => "failed",
            ToolResultStatus::Cancelled => "cancelled",
        };
        let reason = outcome
            .receipt
            .exit_reason
            .as_deref()
            .filter(|reason| !reason.trim().is_empty())
            .map(|reason| format!(" · reason {}", bounded_reason(reason)))
            .unwrap_or_default();
        let reuse = if reused {
            " · reused receipt (not executed again)"
        } else if outcome.receipt.cache_source.is_some() {
            " · cached result"
        } else {
            ""
        };
        self.sink.send(
            &outcome.result.call_id,
            outcome.result.tool_name.as_str(),
            "recorded",
            Some(outcome.receipt.status),
            format!(
                "└ Tool #{number} · {} · {status}{reason}{reuse} · {timing}\n{}\n{}",
                escaped_text(outcome.result.tool_name.as_str()),
                ContentView::preview(&outcome.result.output),
                capture_hints(&outcome.receipt.output_captures)
            ),
        );
    }
}

fn bounded_reason(reason: &str) -> String {
    let escaped = escaped_text(reason);
    let mut characters = escaped.chars();
    let mut bounded = characters.by_ref().take(120).collect::<String>();
    if characters.next().is_some() {
        bounded.push('…');
    }
    bounded
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
            .map(|index| format!(" subcommand {}", usize::from(index) + 1))
            .unwrap_or_default();
        let completeness = if capture.complete {
            "capture complete"
        } else {
            "capture incomplete; unretained bytes cannot be read back"
        };
        let _ = writeln!(
            view,
            "Raw output{child} {stream}: retained {}/observed {} bytes · {completeness}\n  read_output blob_id={} offset=0",
            capture.retained_bytes,
            capture.observed_bytes,
            capture.blob_id.as_str()
        );
    }
    if captures.len() > 4 {
        let _ = writeln!(
            view,
            "[{} more capture references; see the Session record]",
            captures.len() - 4
        );
    }
    view
}
