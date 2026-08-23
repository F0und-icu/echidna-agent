//! The one deterministic Context budget estimator until a measured tokenizer
//! is selected.

use crate::domain::artifacts::ArtifactId;
use crate::domain::model::{ContentItem, MessageRole};

use super::error::ContextPlannerError;
use super::fragment::ContextFragmentContent;

const INSTRUCTION_SEPARATOR_BYTES: usize = 2;

pub(super) fn estimate_encoded_tokens(encoded_bytes: usize) -> u32 {
    let tokens = encoded_bytes.saturating_add(3) / 4;
    u32::try_from(tokens).unwrap_or(u32::MAX)
}

/// Estimates one validated Fragment conservatively for its rendered channel.
/// Every instruction reserves a `\n\n` separator, including the first, so
/// summing Fragment estimates cannot undercount the final joined instructions.
pub(super) fn estimate_fragment_tokens(
    content: &ContextFragmentContent,
) -> Result<u32, ContextPlannerError> {
    match content {
        ContextFragmentContent::Instructions { text } => Ok(estimate_encoded_tokens(
            text.len().saturating_add(INSTRUCTION_SEPARATOR_BYTES),
        )),
        ContextFragmentContent::ContentItems { items } => estimate_content_items_tokens(items),
    }
}

/// Estimates the final rendered Context channels without accidentally omitting
/// the JSON vector that encloses one or more Content Items.
pub(super) fn estimate_rendered_channels_tokens(
    instructions: &str,
    content_items: &[ContentItem],
) -> Result<u32, ContextPlannerError> {
    let instruction_tokens = estimate_encoded_tokens(instructions.len());
    let content_tokens = if content_items.is_empty() {
        0
    } else {
        estimate_content_items_tokens(content_items)?
    };
    Ok(instruction_tokens.saturating_add(content_tokens))
}

pub(super) fn estimate_artifact_reference_tokens(
    artifact_id: &ArtifactId,
) -> Result<u32, ContextPlannerError> {
    estimate_content_items_tokens(&[artifact_reference_item(artifact_id)])
}

pub(super) fn artifact_reference_item(artifact_id: &ArtifactId) -> ContentItem {
    ContentItem::Text {
        role: MessageRole::User,
        text: format!("Artifact reference: {artifact_id}"),
    }
}

/// Produces the largest UTF-8-safe truncation that fits the same rendered
/// channel estimate used for whole Fragments. The ellipsis participates in
/// both byte and JSON encoding accounting.
pub(super) fn truncate_content_to_token_limit(
    content: &ContextFragmentContent,
    token_limit: u32,
) -> Result<Option<ContextFragmentContent>, ContextPlannerError> {
    match content {
        ContextFragmentContent::Instructions { text } => {
            truncate_text_to_token_limit(text, token_limit, |text| {
                ContextFragmentContent::Instructions {
                    text: text.to_owned(),
                }
            })
        }
        ContextFragmentContent::ContentItems { items } => match items.as_slice() {
            [ContentItem::Text { role, text }] => {
                truncate_text_to_token_limit(text, token_limit, |text| {
                    ContextFragmentContent::ContentItems {
                        items: vec![ContentItem::Text {
                            role: *role,
                            text: text.to_owned(),
                        }],
                    }
                })
            }
            _ => Ok(None),
        },
    }
}

fn estimate_content_items_tokens(items: &[ContentItem]) -> Result<u32, ContextPlannerError> {
    serde_json::to_vec(items)
        .map(|encoded| estimate_encoded_tokens(encoded.len()))
        .map_err(|error| ContextPlannerError::CanonicalEncoding {
            message: error.to_string(),
        })
}

fn truncate_text_to_token_limit(
    text: &str,
    token_limit: u32,
    content: impl Fn(&str) -> ContextFragmentContent,
) -> Result<Option<ContextFragmentContent>, ContextPlannerError> {
    let complete = content(text);
    if estimate_fragment_tokens(&complete)? <= token_limit {
        return Ok(Some(complete));
    }

    // Each longer UTF-8 prefix can only increase the encoded representation,
    // so search the character boundaries instead of repeatedly serializing
    // every prefix of a potentially large Tool or Memory fragment.
    let boundaries = text
        .char_indices()
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let mut low = 0;
    let mut high = boundaries.len();
    let mut best = None;
    while low < high {
        let middle = low + (high - low) / 2;
        let truncated = format!("{}…", &text[..boundaries[middle]]);
        let candidate = content(&truncated);
        if estimate_fragment_tokens(&candidate)? <= token_limit {
            best = Some(candidate);
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    Ok(best)
}
