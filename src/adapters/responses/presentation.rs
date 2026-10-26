//! Selected, redacted observations for the explicitly installed CLI presenter.
//! This vocabulary has no persistence or model-delivery authority.

use std::sync::Arc;

#[cfg(debug_assertions)]
use serde_json::Value;

use super::redaction::WireRedaction;
use crate::domain::model::ModelEvent;
use crate::ports::model::ModelProviderError;

pub(crate) trait ResponsesObserver: Send + Sync {
    fn role(&self) -> &'static str;
    fn begin(&self) -> Box<dyn RequestObserver>;
    fn notify(&self, notice: ResponseNotice);
    #[cfg(debug_assertions)]
    fn full_debug(&self) -> bool;
}

pub(crate) trait RequestObserver: Send {
    fn observe(&mut self, event: ResponsePresentation);
}

#[derive(Debug)]
pub(crate) enum ResponseNotice {
    RetryDiscarded { retry: usize },
    CaptureSaved { path: String },
    CaptureFailed { kind: std::io::ErrorKind },
}

#[derive(Debug)]
pub(crate) enum ResponsePresentation {
    #[cfg(debug_assertions)]
    Request(Value),
    #[cfg(debug_assertions)]
    Status(u16),
    #[cfg(debug_assertions)]
    ErrorBody(Value),
    #[cfg(debug_assertions)]
    Omitted(&'static str),
    ModelEvent {
        event: ModelEvent,
        reasoning_item_matches_deltas: bool,
    },
    ReasoningSummary {
        text: String,
        starts_part: bool,
    },
    Completed,
    Rejected {
        protocol_error: bool,
    },
    Interrupted,
}

pub(super) struct RequestOutput {
    observer: Box<dyn RequestObserver>,
    redaction: WireRedaction,
    completed: bool,
    pending_reasoning_deltas: String,
    #[cfg(debug_assertions)]
    full_debug: bool,
}

impl RequestOutput {
    pub(super) fn new(
        observer: &Arc<dyn ResponsesObserver>,
        bearer: &str,
        account: Option<&str>,
    ) -> Self {
        Self {
            observer: observer.begin(),
            redaction: WireRedaction::new(&[bearer, account.unwrap_or("")]),
            completed: false,
            pending_reasoning_deltas: String::new(),
            #[cfg(debug_assertions)]
            full_debug: observer.full_debug(),
        }
    }

    #[cfg(debug_assertions)]
    pub(super) fn request(&mut self, request: &impl serde::Serialize) {
        if !self.full_debug {
            return;
        }
        match serde_json::to_value(request) {
            Ok(mut value) => {
                self.redaction.sanitize(&mut value);
                // Only content consumed by the CLI crosses the Adapter boundary.
                let fields = ["instructions", "input", "text", "previous_response_id"];
                value
                    .as_object_mut()
                    .expect("encoded request is an object")
                    .retain(|key, _| fields.contains(&key.as_str()));
                self.observer.observe(ResponsePresentation::Request(value));
            }
            Err(_) => self.response_omitted("request content view unavailable"),
        }
    }

    #[cfg(debug_assertions)]
    pub(super) fn status(&mut self, status: u16) {
        if !self.full_debug {
            return;
        }
        self.observer.observe(ResponsePresentation::Status(status));
    }

    #[cfg(debug_assertions)]
    pub(super) fn response(&mut self, data: &str) {
        if !self.full_debug {
            return;
        }
        match serde_json::from_str::<Value>(data) {
            Ok(value) => {
                if value.get("type").and_then(Value::as_str) == Some("error") {
                    return;
                }
                if let Some(error) = value.get("error").or_else(|| {
                    value
                        .get("response")
                        .and_then(|response| response.get("error"))
                }) && !error.is_null()
                {
                    let mut error = error.clone();
                    self.redaction.sanitize(&mut error);
                    self.observer
                        .observe(ResponsePresentation::ErrorBody(error));
                }
            }
            Err(_) => self.response_omitted("non-JSON response cannot be structurally redacted"),
        }
    }

    #[cfg(debug_assertions)]
    pub(super) fn http_error_body(&mut self, data: &str) {
        if !self.full_debug {
            return;
        }
        if data.len() > super::MAX_HTTP_ERROR_BODY_BYTES {
            self.response_omitted("HTTP error body omitted: exceeds 65536 bytes");
        } else {
            match serde_json::from_str(data) {
                Ok(mut value) => {
                    self.redaction.sanitize(&mut value);
                    self.observer
                        .observe(ResponsePresentation::ErrorBody(value));
                }
                Err(_) => {
                    self.response_omitted("non-JSON response cannot be structurally redacted")
                }
            }
        }
    }

    #[cfg(debug_assertions)]
    pub(super) fn response_omitted(&mut self, reason: &'static str) {
        if !self.full_debug {
            return;
        }
        self.observer.observe(ResponsePresentation::Omitted(reason));
    }

    pub(super) fn model_event(&mut self, event: &ModelEvent) {
        // Compare original observations before redaction can collapse distinct text.
        let reasoning_item_matches_deltas = match event {
            ModelEvent::ReasoningDelta { text } => {
                self.pending_reasoning_deltas.push_str(text);
                false
            }
            ModelEvent::ReasoningItem { text } => {
                let same = self.pending_reasoning_deltas == *text;
                self.pending_reasoning_deltas.clear();
                same
            }
            _ => false,
        };
        let mut event = event.clone();
        match &mut event {
            ModelEvent::ReasoningDelta { text }
            | ModelEvent::ReasoningItem { text }
            | ModelEvent::TextDelta { text } => *text = self.redaction.text(text),
            ModelEvent::ToolCall { call } => {
                call.call_id = crate::domain::tools::ToolCallId::new(
                    self.redaction.text(call.call_id.as_str()),
                );
                call.name =
                    crate::domain::tools::ToolName::new(self.redaction.text(call.name.as_str()));
                self.redaction.sanitize(&mut call.arguments);
            }
            _ => {}
        }
        self.observer.observe(ResponsePresentation::ModelEvent {
            event,
            reasoning_item_matches_deltas,
        });
    }

    pub(super) fn reasoning_summary(&mut self, text: &str, starts_part: bool) {
        self.observer
            .observe(ResponsePresentation::ReasoningSummary {
                text: self.redaction.text(text),
                starts_part,
            });
    }

    pub(super) fn completed(&mut self) {
        self.completed = true;
        self.observer.observe(ResponsePresentation::Completed);
    }

    pub(super) fn rejected(&mut self, error: &ModelProviderError) {
        self.completed = true;
        self.observer.observe(ResponsePresentation::Rejected {
            protocol_error: matches!(
                error,
                ModelProviderError::ProtocolFailure { .. }
                    | ModelProviderError::InvalidToolArguments { .. }
            ),
        });
    }
}

impl Drop for RequestOutput {
    fn drop(&mut self) {
        if !self.completed {
            self.observer.observe(ResponsePresentation::Interrupted);
        }
    }
}
