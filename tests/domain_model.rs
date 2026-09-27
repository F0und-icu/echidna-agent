use echidna::domain::artifacts::ArtifactId;
use echidna::domain::model::{
    CapabilityMappingVersion, ContentItem, MessageRole, ModelCapabilities, ModelCapabilityError,
    ModelInputIR, ModelRequestRequirements,
};
use echidna::domain::tools::ToolCallId;
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
        max_output_tokens: None,
        context_window: 128_000,
        capability_mapping_version: CapabilityMappingVersion::new(1),
    }
}

fn text_input(requirements: ModelRequestRequirements) -> ModelInputIR {
    ModelInputIR::current(
        "test instructions",
        vec![ContentItem::user_text("test input")],
        requirements,
    )
}

#[test]
fn declared_output_limits_preserve_legacy_shapes_and_reject_missing_or_unsupported_limits() {
    let legacy = capabilities();
    assert!(
        serde_json::to_value(&legacy)
            .unwrap()
            .get("max_output_tokens")
            .is_none()
    );
    assert!(
        serde_json::to_value(ModelRequestRequirements::default())
            .unwrap()
            .get("max_output_tokens")
            .is_none()
    );
    let mut caps = legacy.clone();
    caps.max_output_tokens = Some(8192);
    assert_ne!(caps.semantic_hash(), legacy.semantic_hash());
    assert_eq!(
        caps.validate_input(&text_input(Default::default())),
        Err(ModelCapabilityError::OutputTokenLimitRequired)
    );
    for value in [1, 4096, 8192] {
        let input = text_input(ModelRequestRequirements {
            max_output_tokens: Some(value),
            ..Default::default()
        });
        assert_eq!(caps.validate_input(&input), Ok(()));
        assert_eq!(
            legacy.validate_input(&input),
            Err(ModelCapabilityError::OutputTokenLimitUnsupported)
        );
    }
    for value in [0, 8193] {
        assert!(
            caps.validate_input(&text_input(ModelRequestRequirements {
                max_output_tokens: Some(value),
                ..Default::default()
            }))
            .is_err()
        );
    }
    for value in [0, caps.context_window, u32::MAX] {
        caps.max_output_tokens = Some(value);
        assert_eq!(
            caps.validate_input(&text_input(Default::default())),
            Err(ModelCapabilityError::InvalidOutputTokenLimit)
        );
    }
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
            max_output_tokens: None,
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
            ContentItem::user_text("test input"),
            ContentItem::ImageArtifact {
                role: MessageRole::User,
                artifact_id: ArtifactId::new("artifact-1"),
            },
            ContentItem::Reasoning {
                text: "completed provider reasoning".to_owned(),
            },
        ],
        ModelRequestRequirements {
            tool_call: true,
            structured_output: true,
            incremental_input: false,
            usage_reporting: true,
            max_output_tokens: None,
        },
    );

    assert_eq!(
        serde_json::to_value(input).expect("ModelInputIR should serialize"),
        json!({
            "schema_version": 3,
            "instructions": "test instructions",
            "content_items": [
                {"kind": "text", "role": "user", "text": "test input"},
                {"kind": "image_artifact", "role": "user", "artifact_id": "artifact-1"},
                {"kind": "reasoning", "text": "completed provider reasoning"},
            ],
            "tool_specs": [],
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
    let call_id = ToolCallId::new("tool-call-1");

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
            role: MessageRole::User,
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
                max_output_tokens: None,
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
