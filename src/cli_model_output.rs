//! Terminal rendering of selected Provider observations.

use crate::adapters::responses::presentation::{
    RequestObserver, ResponseNotice, ResponsePresentation, ResponsesObserver,
};
use crate::terminal_console as console;
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};

use crate::domain::model::ModelEvent;

use crate::terminal_content::ContentView;
use crate::terminal_content::escaped_text;

static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);

pub(crate) fn discarded_partial_response(role: &str, retry: usize) {
    let _ = writeln!(
        console::stdout(),
        "\n[response interrupted; partial {role} output discarded; retry {retry}/2]"
    );
}

pub(crate) struct RequestOutput {
    role: &'static str,
    request_id: u64,
    saw_reasoning: bool,
    reasoning_line_open: bool,
    saw_reasoning_summary: bool,
    reasoning_summary_line_open: bool,
    completed: bool,
    rejected: bool,

    vision_text_open: bool,

    full: bool,
}

impl RequestOutput {
    pub(crate) fn new(role: &'static str, full: bool) -> Self {
        Self {
            role,
            request_id: NEXT_REQUEST.fetch_add(1, Ordering::Relaxed),
            saw_reasoning: false,
            reasoning_line_open: false,
            saw_reasoning_summary: false,
            reasoning_summary_line_open: false,
            completed: false,
            rejected: false,

            vision_text_open: false,

            full,
        }
    }

    fn request(&self, value: &Value) {
        if !self.full {
            return;
        }
        let mode = if value.get("previous_response_id").is_some() {
            "incremental (this delivery only)"
        } else {
            "Full"
        };
        let mut stderr = console::stderr();
        let _ = writeln!(
            stderr,
            "\n[debug sending {} #{} · {mode}]",
            self.role, self.request_id
        );
        let _ = writeln!(
            stderr,
            "[content view: tool definitions and transport fields omitted; not a verbatim wire capture]"
        );
        for (key, label) in [
            ("instructions", "Instructions"),
            ("input", "Input"),
            ("text", "Response format"),
        ] {
            if let Some(content) = value.get(key) {
                let _ = writeln!(stderr, "── {label} ──\n{}", ContentView::full(content));
            }
        }
        let _ = writeln!(
            stderr,
            "[debug send finished; model content {}]\n",
            if self.role == "primary" {
                "appears in assistant output"
            } else {
                "appears in Vision output below"
            }
        );
    }

    pub(crate) fn model_event(&mut self, event: &ModelEvent, reasoning_item_matches_deltas: bool) {
        self.end_reasoning_summary_line();
        match event {
            ModelEvent::ReasoningDelta { text } => {
                self.reasoning_text(text);
            }
            ModelEvent::ReasoningItem { text } => {
                if !reasoning_item_matches_deltas {
                    self.end_reasoning_line();
                    self.reasoning_text(text);
                }
                self.end_reasoning_line();
            }
            ModelEvent::TextDelta { text } => {
                self.end_reasoning_line();

                if self.full && self.role == "vision" {
                    let mut stderr = console::stderr();
                    if !self.vision_text_open {
                        let _ = writeln!(stderr, "[debug Vision output · pending]");
                        self.vision_text_open = true;
                    }
                    let _ = write!(stderr, "{}", escaped_text(text));
                    let _ = stderr.flush();
                }
            }
            ModelEvent::ToolCall { call } => {
                self.end_reasoning_line();

                if self.full {
                    self.emit(
                        "model tool call (not executed)",
                        json!({"name":call.name, "arguments":call.arguments}),
                    );
                }
            }
            _ => {}
        }
    }

    pub(crate) fn reasoning_summary(&mut self, text: &str, starts_part: bool) {
        if text.is_empty() {
            return;
        }
        self.end_reasoning_line();
        if starts_part && self.saw_reasoning_summary {
            self.end_reasoning_summary_line();
        }
        let mut stderr = console::stderr();
        if !self.saw_reasoning_summary {
            let _ = writeln!(
                stderr,
                "[reasoning summary {} #{} · pending]",
                self.role, self.request_id
            );
            self.saw_reasoning_summary = true;
        }
        let rendered = escaped_text(text);
        let _ = stderr.write_all(rendered.as_bytes());
        self.reasoning_summary_line_open = !text.ends_with('\n');
        let _ = stderr.flush();
    }

    pub(crate) fn completed(&mut self) {
        self.completed = true;
        self.end_reasoning_line();
        self.end_reasoning_summary_line();
        if self.saw_reasoning_summary {
            let _ = writeln!(console::stderr(), "[reasoning summary completed]");
        }
        if self.full_debug() {
            self.emit("response completed", Value::Null);
        } else if self.saw_reasoning {
            self.end_reasoning_line();
            let _ = writeln!(console::stderr(), "[reasoning completed]");
        }
    }

    fn rejected(&mut self, protocol_error: bool, response: &str) {
        self.rejected = true;
        self.end_reasoning_line();
        self.end_reasoning_summary_line();
        if self.saw_reasoning_summary {
            let _ = writeln!(
                console::stderr(),
                "[reasoning summary not committed: {}]",
                if protocol_error {
                    "protocol decoding failed"
                } else {
                    "Provider returned a stop status"
                }
            );
        }

        if self.full {
            self.emit(
                if protocol_error {
                    "protocol decoding failed: response not committed (does not imply network interruption)"
                } else {
                    "Provider returned a stop status: response not committed"
                },
                json!(response),
            );
            return;
        }
        let _ = writeln!(
            console::stderr(),
            "[rejected response] {}",
            escaped_text(response)
        );
        if self.saw_reasoning {
            let _ = writeln!(
                console::stderr(),
                "[reasoning not committed: {}]",
                if protocol_error {
                    "protocol decoding failed"
                } else {
                    "Provider returned a stop status"
                }
            );
        }
    }

    fn reasoning_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let mut stderr = console::stderr();
        if !self.saw_reasoning {
            let _ = writeln!(
                stderr,
                "[reasoning {} #{} · pending]",
                self.role, self.request_id
            );
            self.saw_reasoning = true;
        }
        // Preserve readable whitespace without interpreting terminal controls.
        let rendered = escaped_text(text);
        let _ = stderr.write_all(rendered.as_bytes());
        self.reasoning_line_open = !text.ends_with('\n');
        let _ = stderr.flush();
    }

    fn end_reasoning_line(&mut self) {
        if self.reasoning_line_open {
            let _ = writeln!(console::stderr());
            self.reasoning_line_open = false;
        }
    }

    fn end_reasoning_summary_line(&mut self) {
        if self.reasoning_summary_line_open {
            let _ = writeln!(console::stderr());
            self.reasoning_summary_line_open = false;
        }
    }

    fn full_debug(&self) -> bool {
        self.full
    }

    fn emit(&self, kind: &str, payload: Value) {
        let _ = writeln!(
            console::stderr(),
            "\n[debug {} #{} · {kind}]\n{}",
            self.role,
            self.request_id,
            if payload.is_null() {
                String::new()
            } else {
                ContentView::full(&payload)
            }
        );
    }
}

impl Drop for RequestOutput {
    fn drop(&mut self) {
        if self.completed || self.rejected {
            return;
        }
        self.end_reasoning_summary_line();
        if self.saw_reasoning_summary {
            let _ = writeln!(
                console::stderr(),
                "[reasoning summary interrupted: response not completed]"
            );
        }
        if self.full_debug() {
            self.end_reasoning_line();

            self.emit("response interrupted: completion not observed", Value::Null);
        } else if self.saw_reasoning {
            self.end_reasoning_line();
            let _ = writeln!(
                console::stderr(),
                "[reasoning interrupted: response not completed]"
            );
        }
    }
}

pub(crate) fn observer(role: &'static str, full: bool) -> Arc<dyn ResponsesObserver> {
    Arc::new(CliModelOutput { role, full })
}

struct CliModelOutput {
    role: &'static str,

    full: bool,
}

impl ResponsesObserver for CliModelOutput {
    fn role(&self) -> &'static str {
        self.role
    }
    fn begin(&self) -> Box<dyn RequestObserver> {
        Box::new(RequestOutput::new(self.role, self.full))
    }

    fn full_debug(&self) -> bool {
        self.full
    }
    fn notify(&self, notice: ResponseNotice) {
        match notice {
            ResponseNotice::RetryDiscarded { retry } => {
                discarded_partial_response(self.role, retry)
            }
            ResponseNotice::CaptureSaved { path } => {
                let _ = writeln!(
                    console::stderr(),
                    "[debug-failures] saved request diagnostic snapshot: {}",
                    escaped_text(&path)
                );
            }
            ResponseNotice::CaptureFailed { kind } => {
                let _ = writeln!(
                    console::stderr(),
                    "[debug-failures] capture failed ({kind:?}); original Provider error preserved"
                );
            }
        }
    }
}

impl RequestObserver for RequestOutput {
    fn observe(&mut self, event: ResponsePresentation) {
        match event {
            ResponsePresentation::Request(value) => self.request(&value),

            ResponsePresentation::Status(status) => {
                if self.full && status >= 400 {
                    self.emit("HTTP error", json!(status));
                }
            }

            ResponsePresentation::ErrorBody(value) => {
                if self.full {
                    self.emit("Provider error", value);
                }
            }

            ResponsePresentation::Omitted(reason) => {
                if self.full {
                    self.emit("response omitted", json!(reason));
                }
            }
            ResponsePresentation::ModelEvent {
                event,
                reasoning_item_matches_deltas,
            } => self.model_event(&event, reasoning_item_matches_deltas),
            ResponsePresentation::ReasoningSummary { text, starts_part } => {
                self.reasoning_summary(&text, starts_part)
            }
            ResponsePresentation::Completed => self.completed(),
            ResponsePresentation::Rejected {
                protocol_error,
                response,
            } => self.rejected(protocol_error, &response),
            ResponsePresentation::Interrupted => {}
        }
    }
}
