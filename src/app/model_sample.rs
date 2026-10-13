//! Atomic decoding and local commit of one completed model sample.

use crate::domain::history::{ConversationHistory, HistoryError};
use crate::domain::model::{ContentItem, ModelEvent, OpaqueContinuationReference};
use crate::domain::tools::ToolCall;

enum SampleItem {
    AssistantText(String),
    Reasoning(String),
    OpaqueContinuation(OpaqueContinuationReference),
    ToolCall(ToolCall),
}

pub(super) struct ModelSample {
    items: Vec<SampleItem>,
}

impl ModelSample {
    pub(super) fn from_events(events: Vec<ModelEvent>) -> Self {
        let mut items = Vec::new();
        let mut text = String::new();
        for event in events {
            match event {
                ModelEvent::TextDelta { text: delta } => text.push_str(&delta),
                ModelEvent::ReasoningDelta { .. } => {}
                ModelEvent::ReasoningItem { text: reasoning } => {
                    if !text.is_empty() {
                        items.push(SampleItem::AssistantText(std::mem::take(&mut text)));
                    }
                    items.push(SampleItem::Reasoning(reasoning));
                }
                ModelEvent::OpaqueContinuation { reference } => {
                    if !text.is_empty() {
                        items.push(SampleItem::AssistantText(std::mem::take(&mut text)));
                    }
                    items.push(SampleItem::OpaqueContinuation(reference));
                }
                ModelEvent::ToolCall { call } => {
                    if !text.is_empty() {
                        items.push(SampleItem::AssistantText(std::mem::take(&mut text)));
                    }
                    items.push(SampleItem::ToolCall(call));
                }
                ModelEvent::Usage { .. } => {}
            }
        }
        if !text.is_empty() {
            items.push(SampleItem::AssistantText(text));
        }
        Self { items }
    }

    pub(super) fn tool_calls(&self) -> Vec<ToolCall> {
        self.items
            .iter()
            .filter_map(|item| match item {
                SampleItem::ToolCall(call) => Some(call.clone()),
                SampleItem::AssistantText(_)
                | SampleItem::Reasoning(_)
                | SampleItem::OpaqueContinuation(_) => None,
            })
            .collect()
    }

    pub(super) fn commit(&self, history: &mut ConversationHistory) -> Result<(), HistoryError> {
        let mut staged = history.clone();
        for item in &self.items {
            match item {
                SampleItem::AssistantText(text) => staged.append_assistant_text(text.clone()),
                SampleItem::Reasoning(_) | SampleItem::OpaqueContinuation(_) => {}
                SampleItem::ToolCall(call) => staged.append_tool_call(call.clone())?,
            }
        }
        *history = staged;
        Ok(())
    }

    pub(super) fn assistant_text(&self) -> String {
        self.items
            .iter()
            .filter_map(|item| match item {
                SampleItem::AssistantText(text) => Some(text.as_str()),
                SampleItem::Reasoning(_)
                | SampleItem::OpaqueContinuation(_)
                | SampleItem::ToolCall(_) => None,
            })
            .collect()
    }

    pub(super) fn content_items(&self) -> Vec<ContentItem> {
        self.items
            .iter()
            .map(|item| match item {
                SampleItem::AssistantText(text) => ContentItem::assistant_text(text.clone()),
                SampleItem::Reasoning(text) => ContentItem::Reasoning { text: text.clone() },
                SampleItem::OpaqueContinuation(reference) => ContentItem::OpaqueContinuation {
                    reference: reference.clone(),
                },
                SampleItem::ToolCall(call) => ContentItem::ToolCall { call: call.clone() },
            })
            .collect()
    }
}
