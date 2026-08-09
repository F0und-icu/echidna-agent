//! Provider-neutral model request, capability, and stream-event contracts.

use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::artifacts::ArtifactId;

/// The initial version of the semantic model input representation.
pub const MODEL_INPUT_IR_SCHEMA_VERSION: u16 = 1;

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
        text: String,
    },
    ImageArtifact {
        artifact_id: ArtifactId,
    },
    /// Preserves an unsupported item so it can fail explicitly at the Port.
    Unsupported {
        content_kind: String,
    },
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
            requirements,
        }
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
                ContentItem::ImageArtifact { artifact_id } if !self.image_input => {
                    return Err(ModelCapabilityError::ImageInputUnsupported {
                        artifact_id: artifact_id.clone(),
                    });
                }
                ContentItem::Unsupported { content_kind } => {
                    return Err(ModelCapabilityError::UnsupportedContentItem {
                        content_kind: content_kind.clone(),
                    });
                }
                ContentItem::Text { .. } | ContentItem::ImageArtifact { .. } => {}
            }
        }

        if input.requirements.tool_call && !self.tool_call {
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

/// A provider-neutral identifier for a model-requested Tool Call.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ModelToolCallId(String);

impl ModelToolCallId {
    /// Constructs an identifier supplied by a model event stream.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the tool-call identifier as supplied by the model stream.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ModelToolCallId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
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
    TextDelta {
        text: String,
    },
    ToolCall {
        call_id: ModelToolCallId,
        name: String,
        arguments: serde_json::Value,
    },
    Usage {
        usage: ModelUsage,
    },
    Completed,
}

#[cfg(test)]
mod tests {
    use super::{
        CapabilityMappingVersion, ContentItem, ModelCapabilities, ModelCapabilityError,
        ModelInputIR, ModelRequestRequirements, ModelToolCallId,
    };
    use crate::domain::artifacts::ArtifactId;
    use serde_json::json;

    fn capabilities() -> ModelCapabilities {
        ModelCapabilities {
            text_input: true,
            image_input: true,
            streaming: true,
            tool_call: true,
            structured_output: true,
            incremental_input: true,
            usage_reporting: true,
            context_window: 128_000,
            capability_mapping_version: CapabilityMappingVersion::new(1),
        }
    }

    fn text_input(requirements: ModelRequestRequirements) -> ModelInputIR {
        ModelInputIR::current(
            "test instructions",
            vec![ContentItem::Text {
                text: "test input".into(),
            }],
            requirements,
        )
    }

    #[test]
    fn capabilities_deserialize_from_a_complete_versioned_fixture() {
        let fixture: ModelCapabilities = serde_json::from_value(json!({
            "text_input": true,
            "image_input": false,
            "streaming": true,
            "tool_call": false,
            "structured_output": true,
            "incremental_input": false,
            "usage_reporting": true,
            "context_window": 65_536,
            "capability_mapping_version": 17,
        }))
        .expect("complete capability fixture should deserialize");

        assert_eq!(
            fixture,
            ModelCapabilities {
                text_input: true,
                image_input: false,
                streaming: true,
                tool_call: false,
                structured_output: true,
                incremental_input: false,
                usage_reporting: true,
                context_window: 65_536,
                capability_mapping_version: CapabilityMappingVersion::new(17),
            }
        );
    }

    #[test]
    fn model_input_ir_json_has_a_stable_current_version_and_content_shape() {
        let input = ModelInputIR::current(
            "test instructions",
            vec![
                ContentItem::Text {
                    text: "test input".into(),
                },
                ContentItem::ImageArtifact {
                    artifact_id: ArtifactId::new("artifact-1"),
                },
            ],
            ModelRequestRequirements {
                tool_call: true,
                structured_output: true,
                incremental_input: false,
                usage_reporting: true,
            },
        );

        assert_eq!(
            serde_json::to_value(input).expect("ModelInputIR should serialize"),
            json!({
                "schema_version": 1,
                "instructions": "test instructions",
                "content_items": [
                    {"kind": "text", "text": "test input"},
                    {"kind": "image_artifact", "artifact_id": "artifact-1"},
                ],
                "requirements": {
                    "tool_call": true,
                    "structured_output": true,
                    "incremental_input": false,
                    "usage_reporting": true,
                },
            })
        );
    }

    #[test]
    fn shared_artifact_and_tool_call_identifiers_preserve_display_values() {
        let artifact_id = ArtifactId::new("artifact-1");
        let call_id = ModelToolCallId::new("tool-call-1");

        assert_eq!(artifact_id.as_str(), "artifact-1");
        assert_eq!(artifact_id.to_string(), "artifact-1");
        assert_eq!(call_id.as_str(), "tool-call-1");
        assert_eq!(call_id.to_string(), "tool-call-1");
        assert_eq!(
            ModelCapabilityError::ImageInputUnsupported { artifact_id }.to_string(),
            "image artifact `artifact-1` is not supported"
        );
    }

    #[test]
    fn capability_validation_uses_explicit_input_and_fixture_fields() {
        let mut fixture = capabilities();
        fixture.text_input = false;

        assert_eq!(
            fixture.validate_input(&text_input(ModelRequestRequirements::default())),
            Err(ModelCapabilityError::TextInputUnsupported)
        );
    }

    #[test]
    fn capability_validation_rejects_image_and_unknown_content_explicitly() {
        let mut fixture = capabilities();
        fixture.image_input = false;
        let image = ModelInputIR::current(
            "test instructions",
            vec![ContentItem::ImageArtifact {
                artifact_id: ArtifactId::new("artifact-1"),
            }],
            ModelRequestRequirements::default(),
        );

        assert_eq!(
            fixture.validate_input(&image),
            Err(ModelCapabilityError::ImageInputUnsupported {
                artifact_id: ArtifactId::new("artifact-1"),
            })
        );

        let unknown = ModelInputIR::current(
            "test instructions",
            vec![ContentItem::Unsupported {
                content_kind: "audio_artifact".into(),
            }],
            ModelRequestRequirements::default(),
        );
        assert_eq!(
            capabilities().validate_input(&unknown),
            Err(ModelCapabilityError::UnsupportedContentItem {
                content_kind: "audio_artifact".into(),
            })
        );
    }

    #[test]
    fn capability_validation_rejects_each_requested_feature() {
        let cases = [
            (
                "streaming",
                ModelRequestRequirements::default(),
                ModelCapabilityError::StreamingUnsupported,
            ),
            (
                "tool_call",
                ModelRequestRequirements {
                    tool_call: true,
                    ..ModelRequestRequirements::default()
                },
                ModelCapabilityError::ToolCallUnsupported,
            ),
            (
                "structured_output",
                ModelRequestRequirements {
                    structured_output: true,
                    ..ModelRequestRequirements::default()
                },
                ModelCapabilityError::StructuredOutputUnsupported,
            ),
            (
                "incremental_input",
                ModelRequestRequirements {
                    incremental_input: true,
                    ..ModelRequestRequirements::default()
                },
                ModelCapabilityError::IncrementalInputUnsupported,
            ),
            (
                "usage_reporting",
                ModelRequestRequirements {
                    usage_reporting: true,
                    ..ModelRequestRequirements::default()
                },
                ModelCapabilityError::UsageReportingUnsupported,
            ),
        ];

        for (missing, requirements, expected_error) in cases {
            let mut fixture = capabilities();
            match missing {
                "streaming" => fixture.streaming = false,
                "tool_call" => fixture.tool_call = false,
                "structured_output" => fixture.structured_output = false,
                "incremental_input" => fixture.incremental_input = false,
                "usage_reporting" => fixture.usage_reporting = false,
                _ => unreachable!("test cases are exhaustive"),
            }

            assert_eq!(
                fixture.validate_input(&text_input(requirements)),
                Err(expected_error)
            );
        }
    }
}
