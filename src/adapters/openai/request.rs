//! Private OpenAI Responses full-request wire encoding.

use serde::Serialize;
use serde_json::Value;

use crate::domain::model::{ContentItem, MODEL_INPUT_IR_SCHEMA_VERSION, MessageRole, ModelInputIR};
use crate::domain::tools::{ToolCall, ToolResult, ToolResultStatus, ToolSpec};
use crate::ports::model::ModelProviderError;

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

pub(super) fn encode_full_request(
    input: ModelInputIR,
    model: String,
) -> Result<ResponsesRequest, ModelProviderError> {
    if input.schema_version.get() != MODEL_INPUT_IR_SCHEMA_VERSION {
        return Err(request_encoding_error(format!(
            "unsupported ModelInputIR schema version {}",
            input.schema_version.get()
        )));
    }
    if input.requirements.incremental_input {
        return Err(request_encoding_error(
            "incremental model input is not implemented by the full-request Adapter",
        ));
    }
    if input.requirements.structured_output {
        return Err(request_encoding_error(
            "structured output requires an output contract from a later Slice",
        ));
    }

    let mut items = Vec::with_capacity(input.content_items.len());
    for item in input.content_items {
        match item {
            ContentItem::Text { role, text } => append_text_message(&mut items, role, text),
            ContentItem::ImageArtifact { artifact_id, .. } => {
                return Err(request_encoding_error(format!(
                    "image artifact `{artifact_id}` requires the deferred Artifact resolver"
                )));
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

    Ok(ResponsesRequest {
        model,
        instructions: input.instructions,
        input: items,
        tools,
        tool_choice: exposes_tools.then_some("auto"),
        parallel_tool_calls: exposes_tools.then_some(false),
        store: false,
        stream: true,
    })
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
