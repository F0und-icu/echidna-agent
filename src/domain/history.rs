//! Pure in-memory conversation History with strict Tool Call/Result pairing.

use std::collections::BTreeMap;

use thiserror::Error;

use super::model::ContentItem;
use super::tools::{ToolCall, ToolCallId, ToolName, ToolResult};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ConversationHistory {
    items: Vec<ContentItem>,
    tool_pairs: BTreeMap<ToolCallId, ToolPairState>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ToolPairState {
    tool_name: ToolName,
    completed: bool,
}

impl ConversationHistory {
    pub fn append_user_text(&mut self, text: impl Into<String>) {
        self.items.push(ContentItem::user_text(text));
    }

    pub fn append_assistant_text(&mut self, text: impl Into<String>) {
        let text = text.into();
        if !text.is_empty() {
            self.items.push(ContentItem::assistant_text(text));
        }
    }

    pub fn append_tool_call(&mut self, call: ToolCall) -> Result<(), HistoryError> {
        if self.tool_pairs.contains_key(&call.call_id) {
            return Err(HistoryError::DuplicateToolCall {
                call_id: call.call_id,
            });
        }
        self.tool_pairs.insert(
            call.call_id.clone(),
            ToolPairState {
                tool_name: call.name.clone(),
                completed: false,
            },
        );
        self.items.push(ContentItem::ToolCall { call });
        Ok(())
    }

    pub fn append_tool_result(&mut self, result: ToolResult) -> Result<(), HistoryError> {
        let state = self.tool_pairs.get_mut(&result.call_id).ok_or_else(|| {
            HistoryError::MissingToolCall {
                call_id: result.call_id.clone(),
            }
        })?;
        if state.tool_name != result.tool_name {
            return Err(HistoryError::ToolNameMismatch {
                call_id: result.call_id,
                expected: state.tool_name.clone(),
                received: result.tool_name,
            });
        }
        if state.completed {
            return Err(HistoryError::DuplicateToolResult {
                call_id: result.call_id,
            });
        }
        state.completed = true;
        self.items.push(ContentItem::ToolResult { result });
        Ok(())
    }

    pub fn ensure_all_tool_calls_paired(&self) -> Result<(), HistoryError> {
        if let Some((call_id, _)) = self.tool_pairs.iter().find(|(_, state)| !state.completed) {
            return Err(HistoryError::MissingToolResult {
                call_id: call_id.clone(),
            });
        }
        Ok(())
    }

    pub fn items(&self) -> &[ContentItem] {
        &self.items
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum HistoryError {
    #[error("Tool Call `{call_id}` already exists in History")]
    DuplicateToolCall { call_id: ToolCallId },
    #[error("Tool Result `{call_id}` has no matching Tool Call")]
    MissingToolCall { call_id: ToolCallId },
    #[error("Tool Call `{call_id}` has no Tool Result")]
    MissingToolResult { call_id: ToolCallId },
    #[error("Tool Result `{call_id}` already exists in History")]
    DuplicateToolResult { call_id: ToolCallId },
    #[error(
        "Tool Result `{call_id}` names `{received}` but the matching Tool Call names `{expected}`"
    )]
    ToolNameMismatch {
        call_id: ToolCallId,
        expected: ToolName,
        received: ToolName,
    },
}
