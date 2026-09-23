//! Provider-neutral model request, capability, and stream-event contracts.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::artifacts::ArtifactId;
use super::tools::{ToolCall, ToolResult, ToolSpec};

/// The current version of the semantic model input representation.
pub const MODEL_INPUT_IR_SCHEMA_VERSION: u16 = 3;
pub(crate) const MODEL_INPUT_IR_V2_SCHEMA_VERSION: u16 = 2;
pub(crate) const MAX_TOOL_ARGUMENT_CORRECTIONS_PER_TURN: u8 = 2;

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
    /// Completed model reasoning retained only for provider-visible
    /// continuation. This is never ordinary Conversation History.
    Reasoning {
        text: String,
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

/// Bounded, payload-free diagnostics for a rejected Tool argument document.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, Error)]
#[serde(deny_unknown_fields)]
#[error("category={category:?}, line={line}, column={column}, arguments_bytes={arguments_bytes}")]
pub struct ToolArgumentDiagnostic {
    pub category: ToolArgumentErrorCategory,
    pub line: usize,
    pub column: usize,
    pub arguments_bytes: usize,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ToolArgumentErrorCategory {
    Syntax,
    Data,
    Eof,
}

/// Explicit request requirements that later Slices can derive from tools and
/// output contracts without guessing capability from a model name.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ModelRequestRequirements {
    pub tool_call: bool,
    pub structured_output: bool,
    pub incremental_input: bool,
    pub usage_reporting: bool,
    /// Total generated tokens, including reasoning; not a billing limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
}

impl ModelRequestRequirements {
    pub(crate) fn normalized_for_tool_specs(mut self, has_tool_specs: bool) -> Self {
        if has_tool_specs {
            self.tool_call = true;
        }
        self
    }
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
        self.requirements = self
            .requirements
            .normalized_for_tool_specs(!tool_specs.is_empty());
        self.tool_specs = tool_specs;
        self
    }

    /// Returns whether this input is the exact logical request plus one or
    /// more completed reasoning items retained for Provider continuation.
    /// All non-reasoning semantics must remain byte-for-byte identical.
    pub(crate) fn is_reasoning_replay_of(&self, logical_input: &Self) -> bool {
        self.has_same_reasoning_contract(logical_input)
            && self
                .content_items
                .iter()
                .filter(|item| !matches!(item, ContentItem::Reasoning { .. }))
                .eq(logical_input.content_items.iter())
    }

    /// Current request contract for a retained reasoning delivery. This does
    /// not prove a prior prefix or a state Diff; those remain Context-owned.
    pub(crate) fn has_same_reasoning_contract(&self, logical_input: &Self) -> bool {
        self.schema_version == logical_input.schema_version
            && self.instructions == logical_input.instructions
            && self.tool_specs == logical_input.tool_specs
            && self.requirements == logical_input.requirements
            && self
                .content_items
                .iter()
                .any(|item| matches!(item, ContentItem::Reasoning { .. }))
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
    /// Explicitly declared supported per-response ceiling. None preserves
    /// legacy requests without output controls; no model-name inference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    pub capability_mapping_version: CapabilityMappingVersion,
}

impl ModelCapabilities {
    /// Resolve the declared ceiling before Context/audit capture. A smaller
    /// explicit request is allowed; adapters must never inject a missing cap.
    pub(crate) fn resolve_output_token_limit(
        &self,
        requested: Option<u32>,
    ) -> Result<Option<u32>, ModelCapabilityError> {
        if self
            .max_output_tokens
            .is_some_and(|limit| limit == 0 || limit >= self.context_window)
        {
            return Err(ModelCapabilityError::InvalidOutputTokenLimit);
        }
        if let Some(requested) = requested {
            if requested == 0 {
                return Err(ModelCapabilityError::InvalidOutputTokenLimit);
            }
            if self
                .max_output_tokens
                .is_none_or(|ceiling| requested > ceiling)
            {
                return Err(ModelCapabilityError::OutputTokenLimitUnsupported);
            }
        }
        Ok(requested.or(self.max_output_tokens))
    }

    /// Returns the canonical, versioned capability-map identity used by
    /// profile routing and durable request facts. The complete mapping,
    /// including its explicit version, remains the sole hash input.
    pub fn semantic_hash(&self) -> String {
        crate::domain::canonical_json::canonical_blake3_hash(self)
            .expect("ModelCapabilities is always JSON serializable")
    }

    /// Validates every semantic requirement without inferring from model names.
    pub fn validate_input(&self, input: &ModelInputIR) -> Result<(), ModelCapabilityError> {
        if self.resolve_output_token_limit(input.requirements.max_output_tokens)?
            != input.requirements.max_output_tokens
        {
            return Err(ModelCapabilityError::OutputTokenLimitRequired);
        }
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
                ContentItem::Reasoning { .. } if !self.text_input => {
                    return Err(ModelCapabilityError::TextInputUnsupported);
                }
                ContentItem::Text { .. }
                | ContentItem::ImageArtifact { .. }
                | ContentItem::ToolCall { .. }
                | ContentItem::ToolResult { .. }
                | ContentItem::Reasoning { .. } => {}
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
    #[error("output token limit must be positive and smaller than the declared context window")]
    InvalidOutputTokenLimit,
    #[error("output token limit is not supported by the explicit capability mapping")]
    OutputTokenLimitUnsupported,
    #[error(
        "request omitted the configured output token limit; resolve it before Context and audit capture"
    )]
    OutputTokenLimitRequired,
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
    ReasoningDelta { text: String },
    ReasoningItem { text: String },
    ToolCall { call: ToolCall },
    Usage { usage: ModelUsage },
}
