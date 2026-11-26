//! Terminal rendering of selected Provider observations.

use crate::adapters::responses::presentation::{
    RequestObserver, ResponseNotice, ResponsePresentation, ResponsesObserver,
};
use crate::chat_display::{Event, JsonLines, LogWriter, ModelResponse, model_log};
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};

use crate::domain::model::ModelEvent;
use crate::ports::model::ModelRequestPresentation;

use crate::terminal_content::ContentView;
use crate::terminal_content::escaped_text;

static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);

pub(crate) struct RequestOutput {
    stream: Option<JsonLines>,
    role: &'static str,
    request_id: u64,
    saw_reasoning: bool,
    reasoning_line_open: bool,
    saw_reasoning_summary: bool,
    reasoning_summary_line_open: bool,
    completed: bool,
    rejected: bool,

    scoped_text_open: bool,

    full: bool,
    selection: bool,
}

impl RequestOutput {
    fn new(role: &'static str, full: bool, selection: bool, stream: Option<JsonLines>) -> Self {
        Self {
            stream,
            role,
            request_id: NEXT_REQUEST.fetch_add(1, Ordering::Relaxed),
            saw_reasoning: false,
            reasoning_line_open: false,
            saw_reasoning_summary: false,
            reasoning_summary_line_open: false,
            completed: false,
            rejected: false,

            scoped_text_open: false,

            full,
            selection,
        }
    }

    fn log(&self) -> LogWriter {
        model_log(&self.stream, self.role, Some(self.request_id))
    }

    fn sample(&self, state: &'static str) {
        if let Some(stream) = &self.stream {
            let _ = stream.send(Event::Sample {
                role: self.role,
                request_id: self.request_id,
                state,
            });
        }
    }

    fn request(&self, value: &Value, omitted: &[&'static str]) {
        if !self.full {
            return;
        }
        let mode = if value.get("previous_response_id").is_some() {
            "incremental (this delivery only)"
        } else {
            "Full"
        };
        let mut visible_input = None;
        if let Some(stream) = &self.stream {
            let mut input = value["input"].clone();
            let mut omitted = omitted.to_vec();
            for item in input.as_array_mut().expect("encoded input is an array") {
                if let Some(parts) = item.get_mut("content").and_then(Value::as_array_mut) {
                    for part in parts {
                        if let Some(part) = part.as_object_mut()
                            && part.get("type").and_then(Value::as_str) == Some("input_image")
                            && part.remove("image_url").is_some()
                        {
                            part.insert("omitted".to_owned(), json!("image_payload"));
                            if !omitted.contains(&"image_payload") {
                                omitted.push("image_payload");
                            }
                        }
                    }
                }
            }
            let _ = stream.send(Event::ModelRequest {
                role: self.role,
                request_id: self.request_id,
                phase: "prepared_for_send",
                model: value["model"]
                    .as_str()
                    .expect("encoded request has a model"),
                delivery: if value.get("previous_response_id").is_some() {
                    "incremental"
                } else {
                    "full"
                },
                content: json!({
                    "instructions": value["instructions"],
                    "input": &input,
                    "text": value.get("text"),
                    "previous_response_id": value.get("previous_response_id"),
                }),
                omitted: &omitted,
                truncated: false,
            });
            visible_input = Some(input);
        }
        let mut stderr = self.log();
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
            let content = if key == "input" {
                visible_input.as_ref().or(value.get(key))
            } else {
                value.get(key)
            };
            if let Some(content) = content {
                let _ = writeln!(stderr, "── {label} ──\n{}", ContentView::full(content));
            }
        }
        let _ = writeln!(
            stderr,
            "[debug send finished; model content {}]\n",
            match self.role {
                "primary" => "appears in assistant output".to_owned(),
                "vision" => "appears in Vision output below".to_owned(),
                role => format!("appears in {role} output below"),
            }
        );
    }

    pub(crate) fn model_event(&mut self, event: &ModelEvent, reasoning_item_matches_deltas: bool) {
        let response = match event {
            ModelEvent::TextDelta { text } => Some(ModelResponse::TextDelta { text }),
            ModelEvent::ReasoningDelta { text } => Some(ModelResponse::ReasoningDelta { text }),
            ModelEvent::ReasoningItem { text } => Some(ModelResponse::ReasoningItem {
                text,
                matches_deltas: reasoning_item_matches_deltas,
            }),
            ModelEvent::ToolCall { call } => Some(ModelResponse::ToolCall { call }),
            _ => None,
        };
        if let Some(response) = response {
            self.response(response);
        }
        if self.selection && !self.full {
            return;
        }
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

                if self.full && self.role != "primary" {
                    let mut stderr = self.log();
                    if !self.scoped_text_open {
                        let label = if self.role == "vision" {
                            "Vision"
                        } else {
                            self.role
                        };
                        let _ = writeln!(stderr, "[debug {label} output · pending]");
                        self.scoped_text_open = true;
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
        self.response(ModelResponse::ReasoningSummary { text, starts_part });
        if self.selection && !self.full {
            return;
        }
        if text.is_empty() {
            return;
        }
        self.end_reasoning_line();
        if starts_part && self.saw_reasoning_summary {
            self.end_reasoning_summary_line();
        }
        let mut stderr = self.log();
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
        self.sample("completed");
        self.end_reasoning_line();
        self.end_reasoning_summary_line();
        if self.saw_reasoning_summary {
            let _ = writeln!(self.log(), "[reasoning summary completed]");
        }
        if self.full_debug() {
            self.emit("response completed", Value::Null);
        } else if self.saw_reasoning {
            self.end_reasoning_line();
            let _ = writeln!(self.log(), "[reasoning completed]");
        }
    }

    fn rejected(&mut self, protocol_error: bool, response: &str) {
        self.rejected = true;
        self.response(ModelResponse::ResponseError {
            category: if protocol_error {
                "protocol_rejected"
            } else {
                "provider_rejected"
            },
            text: response,
        });
        self.sample("rejected");
        self.end_reasoning_line();
        self.end_reasoning_summary_line();
        if self.saw_reasoning_summary {
            let _ = writeln!(
                self.log(),
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
        if !self.selection {
            let _ = writeln!(self.log(), "[rejected response] {}", escaped_text(response));
        }
        if self.saw_reasoning {
            let _ = writeln!(
                self.log(),
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
        let mut stderr = self.log();
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
            let _ = writeln!(self.log());
            self.reasoning_line_open = false;
        }
    }

    fn end_reasoning_summary_line(&mut self) {
        if self.reasoning_summary_line_open {
            let _ = writeln!(self.log());
            self.reasoning_summary_line_open = false;
        }
    }

    fn full_debug(&self) -> bool {
        self.full
    }

    fn response(&self, response: ModelResponse<'_>) {
        if self.full
            && let Some(stream) = &self.stream
        {
            let _ = stream.send(Event::ModelResponse {
                role: self.role,
                request_id: self.request_id,
                response,
            });
        }
    }

    fn emit(&self, kind: &str, payload: Value) {
        let _ = writeln!(
            self.log(),
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
        self.sample("interrupted");
        self.end_reasoning_summary_line();
        if self.saw_reasoning_summary {
            let _ = writeln!(
                self.log(),
                "[reasoning summary interrupted: response not completed]"
            );
        }
        if self.full_debug() {
            self.end_reasoning_line();

            self.emit("response interrupted: completion not observed", Value::Null);
        } else if self.saw_reasoning {
            self.end_reasoning_line();
            let _ = writeln!(
                self.log(),
                "[reasoning interrupted: response not completed]"
            );
        }
    }
}

pub(crate) fn observer(role: &'static str, full: bool) -> Arc<dyn ResponsesObserver> {
    Arc::new(CliModelOutput {
        role,
        full,
        stream: JsonLines::current(),
    })
}

struct CliModelOutput {
    stream: Option<JsonLines>,
    role: &'static str,

    full: bool,
}

impl ResponsesObserver for CliModelOutput {
    fn role(&self) -> &'static str {
        self.role
    }
    fn begin(&self, presentation: ModelRequestPresentation) -> Box<dyn RequestObserver> {
        let selection = presentation == ModelRequestPresentation::TaskSelection;
        let role = if selection {
            "task selection"
        } else {
            self.role
        };
        let output = RequestOutput::new(role, self.full, selection, self.stream.clone());
        output.sample("started");
        Box::new(output)
    }

    fn full_debug(&self) -> bool {
        self.full
    }
    fn notify(&self, notice: ResponseNotice) {
        match notice {
            ResponseNotice::RetryDiscarded {
                retry,
                presentation,
            } => {
                let role = if presentation == ModelRequestPresentation::TaskSelection {
                    "task selection"
                } else {
                    self.role
                };
                let mut log = model_log(&self.stream, role, None);
                log.stderr = false;
                let _ = writeln!(
                    log,
                    "\n[response interrupted; partial {role} output discarded; retry {retry}/2]"
                );
            }
            ResponseNotice::CaptureSaved { path } => {
                let _ = writeln!(
                    model_log(&self.stream, self.role, None),
                    "[debug-failures] saved request diagnostic snapshot: {}",
                    escaped_text(&path)
                );
            }
            ResponseNotice::CaptureFailed { kind } => {
                let _ = writeln!(
                    model_log(&self.stream, self.role, None),
                    "[debug-failures] capture failed ({kind:?}); original Provider error preserved"
                );
            }
        }
    }
}

impl RequestObserver for RequestOutput {
    fn observe(&mut self, event: ResponsePresentation) {
        match event {
            ResponsePresentation::Request { value, omitted } => self.request(&value, &omitted),

            ResponsePresentation::Status(status) => {
                self.response(ModelResponse::ResponseStatus { status });
                if self.full && status >= 400 {
                    self.emit("HTTP error", json!(status));
                }
            }

            ResponsePresentation::ErrorBody(value) => {
                if self.full {
                    let text = match &value {
                        Value::String(text) => text.clone(),
                        _ => value.to_string(),
                    };
                    self.response(ModelResponse::ResponseError {
                        category: "http_error",
                        text: &text,
                    });
                    self.emit("Provider error", value);
                }
            }

            ResponsePresentation::Omitted(reason) => {
                if self.full {
                    self.response(ModelResponse::ResponseError {
                        category: "omitted",
                        text: reason,
                    });
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
