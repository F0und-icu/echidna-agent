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

    /// Appends one persisted History item through the sole Tool pairing
    /// owner. This preserves every non-Tool item verbatim and rejects invalid
    /// Tool pair transitions.
    pub(crate) fn append_content_item(&mut self, item: ContentItem) -> Result<(), HistoryError> {
        match item {
            ContentItem::ToolCall { call } => self.append_tool_call(call),
            ContentItem::ToolResult { result } => self.append_tool_result(result),
            ContentItem::Reasoning { .. } => Err(HistoryError::ProviderContinuationItem),
            ContentItem::Text { .. }
            | ContentItem::ImageArtifact { .. }
            | ContentItem::Unsupported { .. } => {
                self.append_non_tool_item(item);
                Ok(())
            }
        }
    }

    /// Replays complete persisted History through the same pairing rules used
    /// by the AgentLoop and Context Planner.
    pub(crate) fn from_complete_content_items(items: &[ContentItem]) -> Result<Self, HistoryError> {
        let mut history = Self::default();
        for item in items {
            history.append_content_item(item.clone())?;
        }
        history.ensure_all_tool_calls_paired()?;
        Ok(history)
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

    fn append_non_tool_item(&mut self, item: ContentItem) {
        self.items.push(item);
    }

    fn unpaired_tool_calls_in_call_order(&self) -> Vec<(ToolCallId, ToolName)> {
        self.items
            .iter()
            .filter_map(|item| match item {
                ContentItem::ToolCall { call } => {
                    self.tool_pairs.get(&call.call_id).and_then(|state| {
                        (!state.completed).then(|| (call.call_id.clone(), state.tool_name.clone()))
                    })
                }
                ContentItem::ToolResult { .. }
                | ContentItem::Reasoning { .. }
                | ContentItem::Text { .. }
                | ContentItem::ImageArtifact { .. }
                | ContentItem::Unsupported { .. } => None,
            })
            .collect()
    }
}

/// Produces a complete, provider-safe History while retaining the one Tool
/// Call/Result pairing owner. Complete pairs and non-Tool items keep their
/// input order. Orphan Results are historical noise and are omitted; every
/// Call without a Result receives one deterministic terminal Result at the
/// end of the normalized History.
///
/// Duplicate Call IDs, duplicate Results, and Result/Call tool-name
/// mismatches remain malformed History and are rejected rather than repaired.
pub fn normalize_content_items(items: &[ContentItem]) -> Result<Vec<ContentItem>, HistoryError> {
    let mut history = ConversationHistory::default();
    for item in items {
        match item {
            ContentItem::ToolCall { call } => history.append_tool_call(call.clone())?,
            ContentItem::ToolResult { result } => {
                match history.append_tool_result(result.clone()) {
                    Ok(()) => {}
                    Err(HistoryError::MissingToolCall { .. }) => {
                        // A Result that was never preceded by its Call cannot be
                        // made meaningful by a later Call, so it is omitted.
                    }
                    Err(error) => return Err(error),
                }
            }
            ContentItem::Text { .. }
            | ContentItem::ImageArtifact { .. }
            | ContentItem::Unsupported { .. } => history.append_content_item(item.clone())?,
            ContentItem::Reasoning { .. } => {
                return Err(HistoryError::ProviderContinuationItem);
            }
        }
    }

    for (call_id, tool_name) in history.unpaired_tool_calls_in_call_order() {
        history.append_tool_result(ToolResult {
            call_id,
            tool_name,
            status: super::tools::ToolResultStatus::Cancelled,
            output: serde_json::json!({"error": "aborted"}),
        })?;
    }

    Ok(history.items)
}

/// Selects a recent normalized History suffix without splitting any Tool
/// Call/Result pair. Pair discovery and complete-history validation remain in
/// this sole History owner.
pub(crate) fn retain_recent_complete_history(
    items: &[ContentItem],
    requested_recent_items: usize,
) -> Result<(usize, Vec<ContentItem>), HistoryError> {
    let normalized = normalize_content_items(items)?;
    let mut retained_from = normalized.len().saturating_sub(requested_recent_items);
    loop {
        let required_call_ids = normalized[retained_from..]
            .iter()
            .filter_map(|item| match item {
                ContentItem::ToolResult { result } => Some(result.call_id.clone()),
                _ => None,
            })
            .collect::<std::collections::BTreeSet<_>>();
        let expanded_from = normalized[..retained_from]
            .iter()
            .enumerate()
            .filter_map(|(index, item)| match item {
                ContentItem::ToolCall { call } if required_call_ids.contains(&call.call_id) => {
                    Some(index)
                }
                _ => None,
            })
            .min()
            .unwrap_or(retained_from);
        if expanded_from == retained_from {
            break;
        }
        retained_from = expanded_from;
    }
    let retained = normalized[retained_from..].to_vec();
    ConversationHistory::from_complete_content_items(&retained)?;
    Ok((retained_from, retained))
}

/// Validates externally assembled Context items by replaying Tool pairing
/// through the same History owner used by the AgentLoop.
pub(crate) fn validate_content_items(items: &[ContentItem]) -> Result<(), HistoryError> {
    let mut history = ConversationHistory::default();
    for item in items {
        match item {
            ContentItem::ToolCall { call } => history.append_tool_call(call.clone())?,
            ContentItem::ToolResult { result } => history.append_tool_result(result.clone())?,
            ContentItem::Text { .. }
            | ContentItem::ImageArtifact { .. }
            | ContentItem::Reasoning { .. }
            | ContentItem::Unsupported { .. } => {}
        }
    }
    history.ensure_all_tool_calls_paired()
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum HistoryError {
    #[error("provider continuation reasoning is not Conversation History")]
    ProviderContinuationItem,
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
