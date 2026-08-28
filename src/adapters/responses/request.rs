//! Shared Responses request wire encoding.

use base64::Engine;
use serde::Serialize;
use serde_json::Value;

use crate::domain::model::{ContentItem, MODEL_INPUT_IR_SCHEMA_VERSION, MessageRole};
use crate::domain::tools::{ToolCall, ToolResult, ToolResultStatus, ToolSpec};
use crate::ports::model::{ModelProviderError, ModelRequest, ModelRequestDelivery};

#[derive(Debug, Serialize)]
pub(super) struct ResponsesRequest {
    model: String,
    instructions: String,
    input: Vec<ResponsesInputItem>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<ResponsesFunctionTool>,
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
    },
    FunctionCallOutput {
        call_id: String,
        output: String,
    },
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
) -> Result<ResponsesRequest, ModelProviderError> {
    let (input, delivery) = request.into_parts();
    if input.schema_version.get() != MODEL_INPUT_IR_SCHEMA_VERSION {
        return Err(request_encoding_error(format!(
            "unsupported ModelInputIR schema version {}",
            input.schema_version.get()
        )));
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
            ContentItem::ToolCall { call } => items.push(encode_tool_call(call)?),
            ContentItem::ToolResult { result } => items.push(encode_tool_result(result)?),
            ContentItem::Unsupported { content_kind } => {
                return Err(request_encoding_error(format!(
                    "content item `{content_kind}` cannot be encoded"
                )));
            }
        }
    }

    let tools = input
        .tool_specs
        .into_iter()
        .map(ResponsesFunctionTool::from)
        .collect::<Vec<_>>();
    let exposes_tools = !tools.is_empty();

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
    })
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

fn encode_tool_call(call: ToolCall) -> Result<ResponsesInputItem, ModelProviderError> {
    let arguments = serde_json::to_string(&call.arguments).map_err(|error| {
        request_encoding_error(format!("failed to encode Tool Call arguments: {error}"))
    })?;
    Ok(ResponsesInputItem::FunctionCall {
        call_id: call.call_id.to_string(),
        name: call.name.to_string(),
        arguments,
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
