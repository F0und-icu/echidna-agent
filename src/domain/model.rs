//! Provider-neutral model request, capability, and stream-event contracts.

use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::artifacts::ArtifactId;
use super::context::{ProviderSessionRef, ProviderSessionRole};
use super::tools::{ToolCall, ToolResult, ToolSpec};

/// The current version of the semantic model input representation.
pub const MODEL_INPUT_IR_SCHEMA_VERSION: u16 = 4;
pub(crate) const MODEL_INPUT_IR_V2_SCHEMA_VERSION: u16 = 2;
pub(crate) const MODEL_INPUT_IR_V3_SCHEMA_VERSION: u16 = 3;
pub(crate) const MAX_TOOL_ARGUMENT_CORRECTIONS_PER_TURN: u8 = 2;
pub(crate) const MAX_TOOL_ARGUMENT_CORRECTIONS_PER_CHAIN: u8 = 2;

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

/// The delivery session which owns an opaque provider continuation reference.
/// It is a local binding, never a credential or Provider wire identity.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct OpaqueContinuationScope(String);

impl OpaqueContinuationScope {
    /// Derives the stable, local continuation scope for one exact Provider
    /// session. This is a Core binding only; it is neither a wire ID nor a
    /// credential.
    pub(crate) fn for_provider_session(session: &ProviderSessionRef) -> Self {
        let role = match session.role {
            ProviderSessionRole::Primary => "primary",
            ProviderSessionRole::Vision => "vision",
        };
        Self(format!(
            "provider-session:{}:{role}:{}",
            session.id, session.generation
        ))
    }
}

/// An auditable reference to Adapter-local opaque continuation data.
///
/// The encrypted replay value never crosses this Domain boundary. The content
/// identity and encoded length bind Context/Audit while the scope prevents
/// reuse across delivery sessions.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OpaqueContinuationReference {
    scope: OpaqueContinuationScope,
    content_identity: String,
    encoded_byte_length: u32,
}

impl OpaqueContinuationReference {
    pub(crate) fn new(
        scope: OpaqueContinuationScope,
        content_identity: String,
        encoded_byte_length: u32,
    ) -> Self {
        Self {
            scope,
            content_identity,
            encoded_byte_length,
        }
    }

    pub(crate) fn scope(&self) -> &OpaqueContinuationScope {
        &self.scope
    }

    pub(crate) const fn encoded_byte_length(&self) -> u32 {
        self.encoded_byte_length
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
    /// An Adapter-local opaque reasoning item retained for the current
    /// provider delivery only. Its reference is never Conversation History.
    OpaqueContinuation {
        reference: OpaqueContinuationReference,
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

/// A bounded, provider-neutral reasoning setting explicitly selected in a
/// capability mapping. Provider metadata may advertise validated custom values
/// alongside the known variants.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReasoningEffort {
    Low,
    Medium,
    High,
    XHigh,
    Max,
    Custom(String),
}

impl ReasoningEffort {
    pub(crate) fn as_str(&self) -> &str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
            Self::Max => "max",
            Self::Custom(value) => value,
        }
    }

    pub(crate) fn is_valid(&self) -> bool {
        match self {
            Self::Custom(value) => {
                valid_reasoning_effort(value)
                    && !matches!(value.as_str(), "low" | "medium" | "high" | "xhigh" | "max")
            }
            _ => true,
        }
    }
}

impl FromStr for ReasoningEffort {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "low" => Ok(Self::Low),
            "medium" => Ok(Self::Medium),
            "high" => Ok(Self::High),
            "xhigh" => Ok(Self::XHigh),
            "max" => Ok(Self::Max),
            _ if valid_reasoning_effort(value) => Ok(Self::Custom(value.to_owned())),
            _ => Err(()),
        }
    }
}

impl Serialize for ReasoningEffort {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ReasoningEffort {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value
            .parse()
            .map_err(|_| serde::de::Error::custom("invalid reasoning effort"))
    }
}

fn valid_reasoning_effort(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

/// Explicit request requirements that later Slices can derive from tools and
/// output contracts without guessing capability from a model name.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ModelRequestRequirements {
    pub tool_call: bool,
    pub structured_output: bool,
    pub incremental_input: bool,
    pub usage_reporting: bool,
    /// Explicit capability-mapping setting resolved before Context/audit capture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<ReasoningEffort>,
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
                .filter(|item| {
                    !matches!(
                        item,
                        ContentItem::Reasoning { .. } | ContentItem::OpaqueContinuation { .. }
                    )
                })
                .eq(logical_input.content_items.iter())
    }

    /// Current request contract for a retained reasoning delivery. This does
    /// not prove a prior prefix or a state Diff; those remain Context-owned.
    pub(crate) fn has_same_reasoning_contract(&self, logical_input: &Self) -> bool {
        self.schema_version == logical_input.schema_version
            && self.instructions == logical_input.instructions
            && self.tool_specs == logical_input.tool_specs
            && self.requirements == logical_input.requirements
            && self.content_items.iter().any(|item| {
                matches!(
                    item,
                    ContentItem::Reasoning { .. } | ContentItem::OpaqueContinuation { .. }
                )
            })
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
    /// Explicitly selected reasoning setting. None preserves the provider's
    /// existing behavior and does not infer support from a model name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<ReasoningEffort>,
    pub capability_mapping_version: CapabilityMappingVersion,
}

impl ModelCapabilities {
    /// Resolve the selected mapping setting before Context/audit capture. A
    /// request may repeat that exact setting, but may not override or invent it.
    pub(crate) fn resolve_reasoning_effort(
        &self,
        requested: Option<ReasoningEffort>,
    ) -> Result<Option<ReasoningEffort>, ModelCapabilityError> {
        if self
            .reasoning_effort
            .as_ref()
            .is_some_and(|effort| !effort.is_valid())
        {
            return Err(ModelCapabilityError::ReasoningEffortUnsupported);
        }
        if let Some(requested) = requested.as_ref()
            && (!requested.is_valid() || self.reasoning_effort.as_ref() != Some(requested))
        {
            return Err(ModelCapabilityError::ReasoningEffortUnsupported);
        }
        Ok(requested.or_else(|| self.reasoning_effort.clone()))
    }

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
        if self.resolve_reasoning_effort(input.requirements.reasoning_effort.clone())?
            != input.requirements.reasoning_effort
        {
            return Err(ModelCapabilityError::ReasoningEffortRequired);
        }
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
                | ContentItem::Reasoning { .. }
                | ContentItem::OpaqueContinuation { .. } => {}
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
    #[error("reasoning effort is not supported by the explicit capability mapping")]
    ReasoningEffortUnsupported,
    #[error(
        "request omitted the configured reasoning effort; resolve it before Context and audit capture"
    )]
    ReasoningEffortRequired,
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
    /// Provider-reported subset of input tokens served from its cache.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_input_tokens: Option<u64>,
    /// Provider-reported subset of output tokens used for reasoning.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_output_tokens: Option<u64>,
}

impl ModelUsage {
    pub(crate) fn validate(self) -> Result<(), ModelUsageError> {
        if let Some(cached_input_tokens) = self.cached_input_tokens
            && cached_input_tokens > self.input_tokens
        {
            return Err(ModelUsageError::CachedInputExceedsTotal {
                cached_input_tokens,
                input_tokens: self.input_tokens,
            });
        }
        if let Some(reasoning_output_tokens) = self.reasoning_output_tokens
            && reasoning_output_tokens > self.output_tokens
        {
            return Err(ModelUsageError::ReasoningOutputExceedsTotal {
                reasoning_output_tokens,
                output_tokens: self.output_tokens,
            });
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum ModelUsageError {
    #[error("cached input subset {cached_input_tokens} exceeds input total {input_tokens}")]
    CachedInputExceedsTotal {
        cached_input_tokens: u64,
        input_tokens: u64,
    },
    #[error(
        "reasoning output subset {reasoning_output_tokens} exceeds output total {output_tokens}"
    )]
    ReasoningOutputExceedsTotal {
        reasoning_output_tokens: u64,
        output_tokens: u64,
    },
}

/// A provider-neutral event emitted by a model response stream.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModelEvent {
    TextDelta {
        text: String,
    },
    ReasoningDelta {
        text: String,
    },
    ReasoningItem {
        text: String,
    },
    OpaqueContinuation {
        reference: OpaqueContinuationReference,
    },
    ToolCall {
        call: ToolCall,
    },
    Usage {
        usage: ModelUsage,
    },
}
