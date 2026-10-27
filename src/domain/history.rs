//! Pure in-memory conversation History with strict Tool Call/Result pairing.

use std::collections::BTreeMap;

use thiserror::Error;

use super::model::{ContentItem, MessageRole};
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
    pub(crate) fn contains_tool_call_id(&self, call_id: &ToolCallId) -> bool {
        self.tool_pairs.contains_key(call_id)
    }
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
            ContentItem::Reasoning { .. } | ContentItem::OpaqueContinuation { .. } => {
                Err(HistoryError::ProviderContinuationItem)
            }
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

    /// Returns unpaired Calls in their original model order. The History
    /// normalizer and Chat recovery both consume this sole pairing view.
    pub(crate) fn unpaired_tool_calls_in_call_order(&self) -> Vec<(ToolCallId, ToolName)> {
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
                | ContentItem::OpaqueContinuation { .. }
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
/// Call without a Result receives one deterministic terminal Result before
/// the next User message, or at the end if there is no later User message.
/// Assistant narration can interleave a parallel Call group. A real Result
/// across a User boundary is rejected, not reordered. The synthetic result is
/// an explicit interruption-uncertainty marker: it proves neither that execution
/// did not start nor that an effect can be replayed.
///
/// Duplicate Call IDs, duplicate Results, and Result/Call tool-name
/// mismatches remain malformed History and are rejected rather than repaired.
pub fn normalize_content_items(items: &[ContentItem]) -> Result<Vec<ContentItem>, HistoryError> {
    normalize_with_subcommands(items, &[])
}

pub(crate) fn normalize_with_subcommands(
    items: &[ContentItem],
    children: &[super::tools::SubcommandCheckpoint],
) -> Result<Vec<ContentItem>, HistoryError> {
    normalize_with_recorded_results(items, children, &[])
}

/// Real results may be durable beside their Receipts before the ordered
/// History checkpoint. Only those recorded values replace uncertainty here.
pub(crate) fn normalize_with_recorded_results(
    items: &[ContentItem],
    children: &[super::tools::SubcommandCheckpoint],
    recorded_results: &[&ToolResult],
) -> Result<Vec<ContentItem>, HistoryError> {
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
            ContentItem::Reasoning { .. } | ContentItem::OpaqueContinuation { .. } => {
                return Err(HistoryError::ProviderContinuationItem);
            }
        }
    }

    // Validate the whole input first so a late real Result cannot be replaced
    // by a synthetic marker. Replay each User-delimited group through the same
    // pairing owner; scanning only that group avoids rescanning every prefix.
    let mut normalized = Vec::with_capacity(history.items.len());
    let mut group = ConversationHistory::default();
    for item in &history.items {
        if matches!(
            item,
            ContentItem::Text {
                role: MessageRole::User,
                ..
            } | ContentItem::ImageArtifact {
                role: MessageRole::User,
                ..
            }
        ) {
            normalized.extend(finish_recovery_group(
                std::mem::take(&mut group),
                &history,
                children,
                recorded_results,
            )?);
        }
        group.append_content_item(item.clone())?;
    }
    normalized.extend(finish_recovery_group(
        group,
        &history,
        children,
        recorded_results,
    )?);
    Ok(normalized)
}

fn finish_recovery_group(
    mut group: ConversationHistory,
    complete_input: &ConversationHistory,
    children: &[super::tools::SubcommandCheckpoint],
    recorded_results: &[&ToolResult],
) -> Result<Vec<ContentItem>, HistoryError> {
    for (call_id, tool_name) in group.unpaired_tool_calls_in_call_order() {
        if complete_input.tool_pairs[&call_id].completed {
            return Err(HistoryError::ToolResultAfterUserMessage { call_id });
        }
        if let Some(result) = recorded_results
            .iter()
            .find(|result| result.call_id == call_id)
        {
            group.append_tool_result((*result).clone())?;
            continue;
        }
        let mut output = serde_json::json!({
            "error": "aborted",
            "execution_status": "unknown",
            "guidance": "do_not_assume_no_effect_or_auto_rerun"
        });
        let mut known = children
            .iter()
            .filter(|child| child.call_id == call_id)
            .collect::<Vec<_>>();
        if !known.is_empty() {
            known.sort_by_key(|child| child.index);
            output["completed_children"] = serde_json::json!(known);
            output["unobserved_children"] = serde_json::json!("unknown");
        }
        group.append_tool_result(ToolResult {
            call_id,
            tool_name,
            status: super::tools::ToolResultStatus::Cancelled,
            output,
        })?;
    }

    Ok(group.items)
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

/// Minimum suffix that preserves the latest settled Tool batch, including
/// parallel calls and narration interleaved before all results arrived.
/// Selection is separate from legacy Compaction replay, which may retain zero.
pub(crate) fn latest_complete_tool_batch_items(
    items: &[ContentItem],
) -> Result<usize, HistoryError> {
    ConversationHistory::from_complete_content_items(items)?;
    let mut pending = std::collections::BTreeSet::new();
    let mut latest_start = None;
    for (index, item) in items.iter().enumerate() {
        match item {
            ContentItem::ToolCall { call } => {
                if pending.is_empty() {
                    latest_start = Some(index);
                }
                pending.insert(&call.call_id);
            }
            ContentItem::ToolResult { result } => {
                pending.remove(&result.call_id);
            }
            _ => {}
        }
    }
    Ok(latest_start.map_or(0, |start| items.len() - start))
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
            | ContentItem::OpaqueContinuation { .. }
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
    #[error(
        "Tool Result `{call_id}` crosses a later User message; recovery refuses to reorder execution facts"
    )]
    ToolResultAfterUserMessage { call_id: ToolCallId },
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
