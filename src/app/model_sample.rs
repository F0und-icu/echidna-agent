//! Atomic decoding and local commit of one completed model sample.

use crate::domain::history::{ConversationHistory, HistoryError};
use crate::domain::model::{ContentItem, ModelEvent};
use crate::domain::tools::ToolCall;

enum SampleItem {
    AssistantText(String),
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

    pub(super) fn single_tool_call(&self) -> Result<Option<ToolCall>, usize> {
        let calls = self
            .items
            .iter()
            .filter_map(|item| match item {
                SampleItem::ToolCall(call) => Some(call),
                SampleItem::AssistantText(_) => None,
            })
            .collect::<Vec<_>>();
        match calls.as_slice() {
            [] => Ok(None),
            [call] => Ok(Some((*call).clone())),
            _ => Err(calls.len()),
        }
    }

    pub(super) fn commit(&self, history: &mut ConversationHistory) -> Result<(), HistoryError> {
        let mut staged = history.clone();
        for item in &self.items {
            match item {
                SampleItem::AssistantText(text) => staged.append_assistant_text(text.clone()),
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
                SampleItem::ToolCall(_) => None,
            })
            .collect()
    }

    pub(super) fn content_items(&self) -> Vec<ContentItem> {
        self.items
            .iter()
            .map(|item| match item {
                SampleItem::AssistantText(text) => ContentItem::assistant_text(text.clone()),
                SampleItem::ToolCall(call) => ContentItem::ToolCall { call: call.clone() },
            })
            .collect()
    }
}
