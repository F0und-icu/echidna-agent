//! Provider-neutral model request, capability, and stream-event contracts.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::artifacts::ArtifactId;
use super::tools::{ToolCall, ToolResult, ToolSpec};

/// The initial version of the semantic model input representation.
pub const MODEL_INPUT_IR_SCHEMA_VERSION: u16 = 2;

/// A version for the provider-neutral input representation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ModelInputIrSchemaVersion(u16);

impl ModelInputIrSchemaVersion {
    /// Constructs an explicitly versioned input representation.
    pub const fn new(value: u16) -> Self {
        Self(value)
    }

    /// Returns the serialized schema version.
    pub const fn get(self) -> u16 {
        self.0
    }
}

/// A version for an explicit Model capability mapping fixture.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct CapabilityMappingVersion(u16);

impl CapabilityMappingVersion {
    /// Constructs an explicit capability mapping version.
    pub const fn new(value: u16) -> Self {
        Self(value)
    }

    /// Returns the serialized mapping version.
    pub const fn get(self) -> u16 {
        self.0
    }
}

/// A semantic input item understood by the provider-neutral model contract.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContentItem {
    Text {
        role: MessageRole,
        text: String,
    },
    ImageArtifact {
        role: MessageRole,
        artifact_id: ArtifactId,
    },
    ToolCall {
        call: ToolCall,
    },
    ToolResult {
        result: ToolResult,
    },
    /// Preserves an unsupported item so it can fail explicitly at the Port.
    Unsupported {
        content_kind: String,
    },
}

impl ContentItem {
    pub fn user_text(text: impl Into<String>) -> Self {
        Self::Text {
            role: MessageRole::User,
            text: text.into(),
        }
    }

    pub fn assistant_text(text: impl Into<String>) -> Self {
        Self::Text {
            role: MessageRole::Assistant,
            text: text.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    User,
    Assistant,
}

/// Explicit request requirements that later Slices can derive from tools and
/// output contracts without guessing capability from a model name.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ModelRequestRequirements {
    pub tool_call: bool,
    pub structured_output: bool,
    pub incremental_input: bool,
    pub usage_reporting: bool,
}

/// Provider-neutral semantic input for one model request.
#[allow(clippy::upper_case_acronyms)]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ModelInputIR {
    pub schema_version: ModelInputIrSchemaVersion,
    pub instructions: String,
    pub content_items: Vec<ContentItem>,
    pub tool_specs: Vec<ToolSpec>,
    pub requirements: ModelRequestRequirements,
}

impl ModelInputIR {
    /// Constructs the current input representation with explicit semantics.
    pub fn current(
        instructions: impl Into<String>,
        content_items: Vec<ContentItem>,
        requirements: ModelRequestRequirements,
    ) -> Self {
        Self {
            schema_version: ModelInputIrSchemaVersion::new(MODEL_INPUT_IR_SCHEMA_VERSION),
            instructions: instructions.into(),
            content_items,
            tool_specs: Vec::new(),
            requirements,
        }
    }

    pub fn with_tool_specs(mut self, tool_specs: Vec<ToolSpec>) -> Self {
        if !tool_specs.is_empty() {
            self.requirements.tool_call = true;
        }
        self.tool_specs = tool_specs;
        self
    }
}

/// The explicitly declared features for one configured model capability map.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ModelCapabilities {
    pub text_input: bool,
    pub image_input: bool,
    pub streaming: bool,
    pub tool_call: bool,
    pub structured_output: bool,
    pub incremental_input: bool,
    pub usage_reporting: bool,
    pub context_window: u32,
    pub capability_mapping_version: CapabilityMappingVersion,
}

impl ModelCapabilities {
    /// Validates every semantic requirement without inferring from model names.
    pub fn validate_input(&self, input: &ModelInputIR) -> Result<(), ModelCapabilityError> {
        if !self.streaming {
            return Err(ModelCapabilityError::StreamingUnsupported);
        }
        if !input.instructions.is_empty() && !self.text_input {
            return Err(ModelCapabilityError::TextInputUnsupported);
        }

        for item in &input.content_items {
            match item {
                ContentItem::Text { .. } if !self.text_input => {
                    return Err(ModelCapabilityError::TextInputUnsupported);
                }
                ContentItem::ImageArtifact { artifact_id, .. } if !self.image_input => {
                    return Err(ModelCapabilityError::ImageInputUnsupported {
                        artifact_id: artifact_id.clone(),
                    });
                }
                ContentItem::Unsupported { content_kind } => {
                    return Err(ModelCapabilityError::UnsupportedContentItem {
                        content_kind: content_kind.clone(),
                    });
                }
                ContentItem::ToolCall { .. } | ContentItem::ToolResult { .. }
                    if !self.tool_call =>
                {
                    return Err(ModelCapabilityError::ToolCallUnsupported);
                }
                ContentItem::Text { .. }
                | ContentItem::ImageArtifact { .. }
                | ContentItem::ToolCall { .. }
                | ContentItem::ToolResult { .. } => {}
            }
        }

        if (input.requirements.tool_call || !input.tool_specs.is_empty()) && !self.tool_call {
            return Err(ModelCapabilityError::ToolCallUnsupported);
        }
        if input.requirements.structured_output && !self.structured_output {
            return Err(ModelCapabilityError::StructuredOutputUnsupported);
        }
        if input.requirements.incremental_input && !self.incremental_input {
            return Err(ModelCapabilityError::IncrementalInputUnsupported);
        }
        if input.requirements.usage_reporting && !self.usage_reporting {
            return Err(ModelCapabilityError::UsageReportingUnsupported);
        }

        Ok(())
    }
}

/// A typed capability error; unsupported input must never be dropped.
#[allow(clippy::enum_variant_names)]
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ModelCapabilityError {
    #[error("text model input is not supported")]
    TextInputUnsupported,
    #[error("image artifact `{artifact_id}` is not supported")]
    ImageInputUnsupported { artifact_id: ArtifactId },
    #[error("streaming model output is not supported")]
    StreamingUnsupported,
    #[error("model tool calls are not supported")]
    ToolCallUnsupported,
    #[error("structured model output is not supported")]
    StructuredOutputUnsupported,
    #[error("incremental model input is not supported")]
    IncrementalInputUnsupported,
    #[error("model usage reporting is not supported")]
    UsageReportingUnsupported,
    #[error("content item `{content_kind}` is not supported")]
    UnsupportedContentItem { content_kind: String },
}

/// Provider-neutral usage measured for one completed model response.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ModelUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// A provider-neutral event emitted by a model response stream.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModelEvent {
    TextDelta { text: String },
    ToolCall { call: ToolCall },
    Usage { usage: ModelUsage },
    Completed,
}
