//! Selected observations for the explicitly installed CLI presenter.
//! This vocabulary has no persistence or model-delivery authority.

use std::sync::Arc;

use serde_json::Value;

use crate::domain::model::ModelEvent;
use crate::ports::model::ModelProviderError;

pub(crate) trait ResponsesObserver: Send + Sync {
    fn role(&self) -> &'static str;
    fn begin(&self) -> Box<dyn RequestObserver>;
    fn notify(&self, notice: ResponseNotice);

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
    Request(Value),

    Status(u16),

    ErrorBody(Value),

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
        response: String,
    },
    Interrupted,
}

pub(super) struct RequestOutput {
    observer: Box<dyn RequestObserver>,
    completed: bool,
    pending_reasoning_deltas: String,

    full_debug: bool,
}

impl RequestOutput {
    pub(super) fn new(observer: &Arc<dyn ResponsesObserver>) -> Self {
        Self {
            observer: observer.begin(),
            completed: false,
            pending_reasoning_deltas: String::new(),

            full_debug: observer.full_debug(),
        }
    }

    pub(super) fn request(&mut self, request_body: &[u8]) {
        if !self.full_debug {
            return;
        }
        match serde_json::from_slice::<Value>(request_body) {
            Ok(mut value) => {
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

    pub(super) fn status(&mut self, status: u16) {
        if !self.full_debug {
            return;
        }
        self.observer.observe(ResponsePresentation::Status(status));
    }

    pub(super) fn http_error_body(&mut self, data: &str) {
        if !self.full_debug {
            return;
        }
        let value = serde_json::from_str(data).unwrap_or_else(|_| Value::String(data.to_owned()));
        self.observer
            .observe(ResponsePresentation::ErrorBody(value));
    }

    pub(super) fn response_omitted(&mut self, reason: &'static str) {
        if !self.full_debug {
            return;
        }
        self.observer.observe(ResponsePresentation::Omitted(reason));
    }

    pub(super) fn model_event(&mut self, event: &ModelEvent) {
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
        self.observer.observe(ResponsePresentation::ModelEvent {
            event: event.clone(),
            reasoning_item_matches_deltas,
        });
    }

    pub(super) fn reasoning_summary(&mut self, text: &str, starts_part: bool) {
        self.observer
            .observe(ResponsePresentation::ReasoningSummary {
                text: text.to_owned(),
                starts_part,
            });
    }

    pub(super) fn completed(&mut self) {
        self.completed = true;
        self.observer.observe(ResponsePresentation::Completed);
    }

    pub(super) fn rejected(&mut self, error: &ModelProviderError, data: &str) {
        self.completed = true;
        self.observer.observe(ResponsePresentation::Rejected {
            protocol_error: matches!(
                error,
                ModelProviderError::ProtocolFailure { .. }
                    | ModelProviderError::InvalidToolArguments { .. }
            ),
            response: data.to_owned(),
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
