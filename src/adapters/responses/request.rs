//! Shared Responses request wire encoding.

use base64::Engine;
use serde::Serialize;
use serde_json::Value;

use crate::domain::model::{
    ContentItem, MODEL_INPUT_IR_SCHEMA_VERSION, MessageRole, OpaqueContinuationReference,
    OpaqueContinuationScope,
};
use crate::domain::tools::{ToolCall, ToolResult, ToolResultStatus, ToolSpec};
use crate::ports::model::{ModelProviderError, ModelRequest, ModelRequestDelivery};

use super::ResponsesWireProfile;

pub(super) const ECHIDNA_TOOL_NAMESPACE: &str = "echidna";
const ECHIDNA_TOOL_NAMESPACE_DESCRIPTION: &str = "Echidna local agent tools.";

#[derive(Debug, Serialize)]
pub(super) struct ResponsesRequest {
    model: String,
    instructions: String,
    input: Vec<ResponsesInputItem>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<ResponsesTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parallel_tool_calls: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    previous_response_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<ResponsesTextConfiguration>,
    store: bool,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    include: Option<Vec<&'static str>>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ResponsesInputItem {
    Message {
        role: &'static str,
        content: Vec<ResponsesMessageContent>,
    },
    FunctionCall {
        call_id: String,
        name: String,
        arguments: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        namespace: Option<&'static str>,
    },
    FunctionCallOutput {
        call_id: String,
        output: String,
    },
    Reasoning {
        content: Vec<ResponsesReasoningContent>,
    },
    #[serde(rename = "reasoning")]
    ReasoningOpaque {
        id: String,
        encrypted_content: String,
        summary: Vec<Value>,
    },
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ResponsesReasoningContent {
    ReasoningText { text: String },
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ResponsesMessageContent {
    InputText { text: String },
    OutputText { text: String },
    InputImage { image_url: String },
}

#[derive(Debug, Serialize)]
struct ResponsesTextConfiguration {
    format: ResponsesTextFormat,
}

#[derive(Debug, Serialize)]
struct ResponsesTextFormat {
    #[serde(rename = "type")]
    kind: &'static str,
}

#[derive(Debug, Serialize)]
struct ResponsesFunctionTool {
    #[serde(rename = "type")]
    kind: &'static str,
    name: String,
    description: String,
    parameters: Value,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum ResponsesTool {
    Function(ResponsesFunctionTool),
    Namespace(ResponsesFunctionNamespace),
}

#[derive(Debug, Serialize)]
struct ResponsesFunctionNamespace {
    #[serde(rename = "type")]
    kind: &'static str,
    name: &'static str,
    description: &'static str,
    tools: Vec<ResponsesFunctionTool>,
}

impl From<ToolSpec> for ResponsesFunctionTool {
    fn from(spec: ToolSpec) -> Self {
        Self {
            kind: "function",
            name: spec.name.to_string(),
            description: spec.description,
            parameters: spec.input_schema,
        }
    }
}

#[derive(Serialize)]
struct FunctionCallOutputBody<'output> {
    status: ToolResultStatus,
    output: &'output Value,
}

pub(super) fn encode_request(
    request: ModelRequest,
    model: String,
    image_resolver: Option<&super::ImageArtifactResolver>,
    profile: ResponsesWireProfile,
    opaque_resolver: &dyn Fn(
        &OpaqueContinuationReference,
        &OpaqueContinuationScope,
    ) -> Result<SubscriptionOpaqueReasoning, ModelProviderError>,
) -> Result<ResponsesRequest, ModelProviderError> {
    let (input, delivery, opaque_scope) = request.into_parts();
    if input.schema_version.get() != MODEL_INPUT_IR_SCHEMA_VERSION {
        return Err(request_encoding_error(format!(
            "unsupported ModelInputIR schema version {}",
            input.schema_version.get()
        )));
    }
    if profile == ResponsesWireProfile::ChatGptSubscription
        && (!matches!(delivery, ModelRequestDelivery::Full)
            || input.requirements.incremental_input
            || input.requirements.max_output_tokens.is_some())
    {
        return Err(request_encoding_error(
            "ChatGPT subscription Responses requires Full input without incremental or output-token controls",
        ));
    }
    let mut items = Vec::with_capacity(input.content_items.len());
    for item in input.content_items {
        match item {
            ContentItem::Text { role, text } => append_text_message(&mut items, role, text),
            ContentItem::ImageArtifact { role, artifact_id } => {
                let resolver = image_resolver.ok_or_else(|| {
                    request_encoding_error(format!(
                        "image artifact `{artifact_id}` requires a configured Artifact resolver"
                    ))
                })?;
                let (media_type, bytes) = resolver(&artifact_id)?;
                append_image_message(&mut items, role, media_type, bytes)?;
            }
            ContentItem::ToolCall { call } => items.push(encode_tool_call(call, profile)?),
            ContentItem::ToolResult { result } => items.push(encode_tool_result(result)?),
            ContentItem::Reasoning { text } => {
                if text.is_empty() {
                    return Err(request_encoding_error(
                        "completed reasoning input requires non-empty text",
                    ));
                }
                items.push(ResponsesInputItem::Reasoning {
                    content: vec![ResponsesReasoningContent::ReasoningText { text }],
                });
            }
            ContentItem::OpaqueContinuation { reference } => {
                if profile != ResponsesWireProfile::ChatGptSubscription {
                    return Err(request_encoding_error(
                        "opaque provider continuation requires the explicit ChatGPT subscription Responses profile",
                    ));
                }
                let scope = opaque_scope.as_ref().ok_or_else(|| {
                    request_encoding_error(
                        "opaque provider continuation requires a delivery session scope",
                    )
                })?;
                let opaque = opaque_resolver(&reference, scope)?;
                items.push(opaque.into_input_item());
            }
            ContentItem::Unsupported { content_kind } => {
                return Err(request_encoding_error(format!(
                    "content item `{content_kind}` cannot be encoded"
                )));
            }
        }
    }

    let function_tools = input
        .tool_specs
        .into_iter()
        .map(ResponsesFunctionTool::from)
        .collect::<Vec<_>>();
    let exposes_tools = !function_tools.is_empty();
    let tools = match profile {
        ResponsesWireProfile::Standard => function_tools
            .into_iter()
            .map(ResponsesTool::Function)
            .collect(),
        ResponsesWireProfile::ChatGptSubscription if exposes_tools => {
            vec![ResponsesTool::Namespace(ResponsesFunctionNamespace {
                kind: "namespace",
                name: ECHIDNA_TOOL_NAMESPACE,
                description: ECHIDNA_TOOL_NAMESPACE_DESCRIPTION,
                tools: function_tools,
            })]
        }
        ResponsesWireProfile::ChatGptSubscription => Vec::new(),
    };

    let previous_response_id = match delivery {
        ModelRequestDelivery::Full => None,
        ModelRequestDelivery::Incremental { continuation } => {
            Some(continuation.as_str().to_owned())
        }
    };

    Ok(ResponsesRequest {
        model,
        instructions: input.instructions,
        input: items,
        tools,
        tool_choice: exposes_tools.then_some("auto"),
        parallel_tool_calls: exposes_tools.then_some(false),
        previous_response_id,
        text: input
            .requirements
            .structured_output
            .then_some(ResponsesTextConfiguration {
                format: ResponsesTextFormat {
                    kind: "json_object",
                },
            }),
        store: false,
        stream: true,
        max_output_tokens: (profile == ResponsesWireProfile::Standard)
            .then_some(input.requirements.max_output_tokens)
            .flatten(),
        include: (profile == ResponsesWireProfile::ChatGptSubscription)
            .then_some(vec!["reasoning.encrypted_content"]),
    })
}

/// The complete stateless replay form of a subscription reasoning item.
/// It remains private to the Responses Adapter; Core receives only its
/// fingerprinted reference.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SubscriptionOpaqueReasoning {
    id: String,
    encrypted_content: String,
    summary: Vec<Value>,
}

impl SubscriptionOpaqueReasoning {
    pub(super) fn from_completed_item(item: &Value) -> Result<Self, ModelProviderError> {
        let id = item
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| {
                request_encoding_error(
                    "subscription encrypted reasoning item requires a non-empty string `id`",
                )
            })?
            .to_owned();
        let encrypted_content = item
            .get("encrypted_content")
            .and_then(Value::as_str)
            .filter(|content| !content.is_empty())
            .ok_or_else(|| {
                request_encoding_error(
                    "subscription encrypted reasoning item requires non-empty string `encrypted_content`",
                )
            })?
            .to_owned();
        let summary = item
            .get("summary")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                request_encoding_error(
                    "subscription encrypted reasoning item requires array `summary`",
                )
            })?
            .clone();
        let value = Self {
            id,
            encrypted_content,
            summary,
        };
        if value.encoded_byte_length()? > super::MAX_OPAQUE_CONTINUATION_ITEM_BYTES {
            return Err(request_encoding_error(
                "subscription encrypted reasoning item exceeds the bounded continuation size",
            ));
        }
        Ok(value)
    }

    pub(super) fn reference(
        &self,
        scope: OpaqueContinuationScope,
    ) -> Result<OpaqueContinuationReference, ModelProviderError> {
        let encoded = self.encoded_bytes()?;
        let encoded_byte_length = u32::try_from(encoded.len()).map_err(|_| {
            request_encoding_error("subscription encrypted reasoning item size overflow")
        })?;
        Ok(OpaqueContinuationReference::new(
            scope,
            blake3::hash(&encoded).to_hex().to_string(),
            encoded_byte_length,
        ))
    }

    pub(super) fn validates_reference(
        &self,
        reference: &OpaqueContinuationReference,
    ) -> Result<bool, ModelProviderError> {
        let expected = self.reference(reference.scope().clone())?;
        Ok(&expected == reference)
    }

    fn into_input_item(self) -> ResponsesInputItem {
        ResponsesInputItem::ReasoningOpaque {
            id: self.id,
            encrypted_content: self.encrypted_content,
            summary: self.summary,
        }
    }

    pub(super) fn encoded_byte_length(&self) -> Result<usize, ModelProviderError> {
        Ok(self.encoded_bytes()?.len())
    }

    fn encoded_bytes(&self) -> Result<Vec<u8>, ModelProviderError> {
        serde_json::to_vec(&ResponsesInputItem::ReasoningOpaque {
            id: self.id.clone(),
            encrypted_content: self.encrypted_content.clone(),
            summary: self.summary.clone(),
        })
        .map_err(|error| {
            request_encoding_error(format!(
                "failed to encode subscription encrypted reasoning: {error}"
            ))
        })
    }
}

fn append_image_message(
    items: &mut Vec<ResponsesInputItem>,
    role: MessageRole,
    media_type: String,
    bytes: Vec<u8>,
) -> Result<(), ModelProviderError> {
    if role != MessageRole::User {
        return Err(request_encoding_error(
            "Responses image Artifact input is supported only for the user role",
        ));
    }
    if !matches!(media_type.as_str(), "image/png" | "image/jpeg") {
        return Err(request_encoding_error(format!(
            "image Artifact media type `{media_type}` is not PNG or JPEG"
        )));
    }
    let image_url = format!(
        "data:{media_type};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    );
    let content = ResponsesMessageContent::InputImage { image_url };
    if let Some(ResponsesInputItem::Message {
        role: previous_role,
        content: previous_content,
    }) = items.last_mut()
        && *previous_role == "user"
    {
        previous_content.push(content);
        return Ok(());
    }
    items.push(ResponsesInputItem::Message {
        role: "user",
        content: vec![content],
    });
    Ok(())
}

fn append_text_message(items: &mut Vec<ResponsesInputItem>, role: MessageRole, text: String) {
    let (wire_role, content) = match role {
        MessageRole::User => ("user", ResponsesMessageContent::InputText { text }),
        MessageRole::Assistant => ("assistant", ResponsesMessageContent::OutputText { text }),
    };

    if let Some(ResponsesInputItem::Message {
        role: previous_role,
        content: previous_content,
    }) = items.last_mut()
        && *previous_role == wire_role
    {
        previous_content.push(content);
        return;
    }

    items.push(ResponsesInputItem::Message {
        role: wire_role,
        content: vec![content],
    });
}

fn encode_tool_call(
    call: ToolCall,
    profile: ResponsesWireProfile,
) -> Result<ResponsesInputItem, ModelProviderError> {
    let arguments = serde_json::to_string(&call.arguments).map_err(|error| {
        request_encoding_error(format!("failed to encode Tool Call arguments: {error}"))
    })?;
    Ok(ResponsesInputItem::FunctionCall {
        call_id: call.call_id.to_string(),
        name: call.name.to_string(),
        arguments,
        namespace: (profile == ResponsesWireProfile::ChatGptSubscription)
            .then_some(ECHIDNA_TOOL_NAMESPACE),
    })
}

fn encode_tool_result(result: ToolResult) -> Result<ResponsesInputItem, ModelProviderError> {
    let output = serde_json::to_string(&FunctionCallOutputBody {
        status: result.status,
        output: &result.output,
    })
    .map_err(|error| {
        request_encoding_error(format!("failed to encode Tool Result output: {error}"))
    })?;
    Ok(ResponsesInputItem::FunctionCallOutput {
        call_id: result.call_id.to_string(),
        output,
    })
}

fn request_encoding_error(message: impl Into<String>) -> ModelProviderError {
    ModelProviderError::RequestEncodingFailure {
        message: message.into(),
    }
}
