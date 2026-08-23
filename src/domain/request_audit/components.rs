//! Typed ordered component references and their structural validation.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::domain::artifacts::BlobId;
use crate::domain::canonical_json::canonical_blake3_hash;

use super::RequestAuditError;

/// A BLAKE3 semantic content identity. It is deliberately distinct from the
/// SHA-256 [`BlobId`], which identifies exact persisted bytes.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct SemanticHash(String);

impl SemanticHash {
    pub(super) fn from_canonical(value: &impl Serialize) -> Result<Self, RequestAuditError> {
        canonical_blake3_hash(value).map(Self).map_err(|error| {
            RequestAuditError::CanonicalEncoding {
                message: error.to_string(),
            }
        })
    }

    pub(super) fn validate(&self, field: &'static str) -> Result<(), RequestAuditError> {
        if is_lowercase_hex_hash(&self.0) {
            Ok(())
        } else {
            Err(RequestAuditError::InvalidSemanticHash { field })
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The typed place occupied by an exact rendered request component.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", content = "index", rename_all = "snake_case")]
pub enum RequestAuditComponentIdentity {
    Instructions,
    ContentItem(u32),
    ToolSpec(u32),
    RequestRequirements,
}

impl RequestAuditComponentIdentity {
    pub(super) const fn instructions() -> Self {
        Self::Instructions
    }

    pub(super) fn content_item(index: usize) -> Result<Self, RequestAuditError> {
        u32::try_from(index)
            .map(Self::ContentItem)
            .map_err(|_| RequestAuditError::ComponentIndexOverflow)
    }

    pub(super) fn tool_spec(index: usize) -> Result<Self, RequestAuditError> {
        u32::try_from(index)
            .map(Self::ToolSpec)
            .map_err(|_| RequestAuditError::ComponentIndexOverflow)
    }

    pub(super) const fn requirements() -> Self {
        Self::RequestRequirements
    }
}

/// The source boundary is deliberately rendered semantic content, not a
/// fabricated Session Event reference. Session History does not retain Event
/// IDs for its individual items.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestAuditComponentSource {
    RenderedInstructions,
    RenderedContentItem,
    RenderedToolSpec,
    RenderedRequestRequirements,
}

/// One exact Blob reference in Provider send order.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RequestAuditComponentReference {
    pub(super) position: u32,
    pub(super) identity: RequestAuditComponentIdentity,
    pub(super) source: RequestAuditComponentSource,
    pub(super) blob_id: BlobId,
    pub(super) size_bytes: u64,
    pub(super) semantic_hash: SemanticHash,
}

impl RequestAuditComponentReference {
    pub const fn position(&self) -> u32 {
        self.position
    }

    pub fn identity(&self) -> &RequestAuditComponentIdentity {
        &self.identity
    }

    pub const fn source(&self) -> RequestAuditComponentSource {
        self.source
    }

    pub fn blob_id(&self) -> &BlobId {
        &self.blob_id
    }

    pub const fn size_bytes(&self) -> u64 {
        self.size_bytes
    }

    pub fn semantic_hash(&self) -> &SemanticHash {
        &self.semantic_hash
    }
}

pub(super) fn validate_component_layout(
    components: &[RequestAuditComponentReference],
) -> Result<(), RequestAuditError> {
    if components.len() < 2 {
        return Err(RequestAuditError::ComponentLayoutIncomplete);
    }
    let mut identities = BTreeSet::new();
    let mut stage = ComponentStage::Instructions;
    let mut expected_content_index = 0_u32;
    let mut expected_tool_index = 0_u32;
    for (index, component) in components.iter().enumerate() {
        let expected_position =
            u32::try_from(index).map_err(|_| RequestAuditError::ComponentPositionOverflow)?;
        if component.position != expected_position {
            return Err(RequestAuditError::ComponentPositionMismatch {
                expected: expected_position,
                received: component.position,
            });
        }
        if !identities.insert(component.identity.clone()) {
            return Err(RequestAuditError::DuplicateComponentIdentity {
                identity: component.identity.clone(),
            });
        }
        BlobId::new(component.blob_id.as_str().to_owned()).map_err(|_| {
            RequestAuditError::InvalidBlobReference {
                position: component.position,
            }
        })?;
        component
            .semantic_hash
            .validate("component.semantic_hash")?;
        match (&stage, &component.identity, component.source) {
            (
                ComponentStage::Instructions,
                RequestAuditComponentIdentity::Instructions,
                RequestAuditComponentSource::RenderedInstructions,
            ) => stage = ComponentStage::ContentItems,
            (
                ComponentStage::ContentItems,
                RequestAuditComponentIdentity::ContentItem(item_index),
                RequestAuditComponentSource::RenderedContentItem,
            ) if *item_index == expected_content_index => {
                expected_content_index = expected_content_index
                    .checked_add(1)
                    .ok_or(RequestAuditError::ComponentIndexOverflow)?;
            }
            (
                ComponentStage::ContentItems,
                RequestAuditComponentIdentity::ToolSpec(tool_index),
                RequestAuditComponentSource::RenderedToolSpec,
            ) if *tool_index == expected_tool_index => {
                stage = ComponentStage::ToolSpecs;
                expected_tool_index = expected_tool_index
                    .checked_add(1)
                    .ok_or(RequestAuditError::ComponentIndexOverflow)?;
            }
            (
                ComponentStage::ContentItems | ComponentStage::ToolSpecs,
                RequestAuditComponentIdentity::RequestRequirements,
                RequestAuditComponentSource::RenderedRequestRequirements,
            ) if index + 1 == components.len() => stage = ComponentStage::Complete,
            (
                ComponentStage::ToolSpecs,
                RequestAuditComponentIdentity::ToolSpec(tool_index),
                RequestAuditComponentSource::RenderedToolSpec,
            ) if *tool_index == expected_tool_index => {
                expected_tool_index = expected_tool_index
                    .checked_add(1)
                    .ok_or(RequestAuditError::ComponentIndexOverflow)?;
            }
            _ => {
                return Err(RequestAuditError::ComponentOrderMismatch {
                    position: component.position,
                    expected: stage.expected(),
                });
            }
        }
    }
    if stage == ComponentStage::Complete {
        Ok(())
    } else {
        Err(RequestAuditError::ComponentLayoutIncomplete)
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ComponentStage {
    Instructions,
    ContentItems,
    ToolSpecs,
    Complete,
}

impl ComponentStage {
    const fn expected(self) -> &'static str {
        match self {
            Self::Instructions => "instructions",
            Self::ContentItems => "content item, tool spec, or final requirements",
            Self::ToolSpecs => "tool spec or final requirements",
            Self::Complete => "no additional component",
        }
    }
}

fn is_lowercase_hex_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}
