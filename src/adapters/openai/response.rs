//! Private OpenAI Responses streaming-event wire decoding.

use serde::Deserialize;
use serde_json::Value;

use crate::domain::model::{ModelEvent, ModelUsage};
use crate::domain::tools::{ToolCall, ToolCallId, ToolName};
use crate::ports::model::ModelProviderError;

#[derive(Debug)]
pub(super) struct DecodedResponsesEvent {
    pub events: Vec<ModelEvent>,
    pub completed: bool,
}

#[derive(Debug, Deserialize)]
struct ResponsesStreamEvent {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    delta: Option<String>,
    #[serde(default)]
    item: Option<Value>,
    #[serde(default)]
    response: Option<Value>,
}

pub(super) fn decode_stream_event(
    data: &str,
    usage_required: bool,
) -> Result<DecodedResponsesEvent, ModelProviderError> {
    let event: ResponsesStreamEvent = serde_json::from_str(data).map_err(|error| {
        protocol_error(format!(
            "invalid Responses event JSON at line {}, column {}",
            error.line(),
            error.column()
        ))
    })?;

    match event.kind.as_str() {
        "response.output_text.delta" => {
            let text = event
                .delta
                .ok_or_else(|| protocol_error("text delta event omitted `delta`"))?;
            Ok(decoded(vec![ModelEvent::TextDelta { text }], false))
        }
        "response.output_item.done" => {
            let item = event
                .item
                .ok_or_else(|| protocol_error("output item event omitted `item`"))?;
            decode_output_item(item)
        }
        "response.completed" => decode_completed(event.response, usage_required),
        "response.failed" => Err(decode_provider_failure(event.response)),
        "response.incomplete" => Err(decode_incomplete(event.response)),
        "response.created"
        | "response.in_progress"
        | "response.output_item.added"
        | "response.content_part.added"
        | "response.content_part.done"
        | "response.output_text.done"
        | "response.function_call_arguments.delta"
        | "response.function_call_arguments.done"
        | "response.reasoning_summary_part.added"
        | "response.reasoning_summary_part.done"
        | "response.reasoning_summary_text.delta"
        | "response.reasoning_summary_text.done" => Ok(decoded(Vec::new(), false)),
        unknown => Err(protocol_error(format!(
            "unsupported Responses event `{unknown}`"
        ))),
    }
}

fn decode_output_item(item: Value) -> Result<DecodedResponsesEvent, ModelProviderError> {
    let item_kind = item
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| protocol_error("output item omitted string `type`"))?;

    match item_kind {
        "function_call" => {
            let function_call: FunctionCallItem =
                serde_json::from_value(item).map_err(|error| {
                    protocol_error(format!("invalid completed function call: {error}"))
                })?;
            if function_call.call_id.is_empty() || function_call.name.is_empty() {
                return Err(protocol_error(
                    "completed function call requires non-empty `call_id` and `name`",
                ));
            }
            let arguments = serde_json::from_str(&function_call.arguments).map_err(|error| {
                protocol_error(format!(
                    "function call arguments are not valid JSON: {error}"
                ))
            })?;

            Ok(decoded(
                vec![ModelEvent::ToolCall {
                    call: ToolCall::new(
                        ToolCallId::new(function_call.call_id),
                        ToolName::new(function_call.name),
                        arguments,
                    ),
                }],
                false,
            ))
        }
        "message" | "reasoning" => Ok(decoded(Vec::new(), false)),
        unknown => Err(protocol_error(format!(
            "unsupported completed output item `{unknown}`"
        ))),
    }
}

fn decode_completed(
    response: Option<Value>,
    usage_required: bool,
) -> Result<DecodedResponsesEvent, ModelProviderError> {
    let response: CompletedResponse = serde_json::from_value(
        response.ok_or_else(|| protocol_error("completed event omitted `response`"))?,
    )
    .map_err(|error| protocol_error(format!("invalid completed response: {error}")))?;

    let mut events = Vec::with_capacity(2);
    match response.usage {
        Some(usage) => events.push(ModelEvent::Usage {
            usage: ModelUsage {
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
            },
        }),
        None if usage_required => {
            return Err(protocol_error(
                "completed response omitted required token Usage",
            ));
        }
        None => {}
    }
    events.push(ModelEvent::Completed);

    Ok(decoded(events, true))
}

fn decode_provider_failure(response: Option<Value>) -> ModelProviderError {
    let error = response
        .as_ref()
        .and_then(|value| value.get("error"))
        .cloned()
        .and_then(|value| serde_json::from_value::<ProviderErrorBody>(value).ok());

    ModelProviderError::ProviderFailure {
        code: error.as_ref().and_then(|error| error.code.clone()),
        message: error
            .and_then(|error| error.message)
            .filter(|message| !message.trim().is_empty())
            .unwrap_or_else(|| "OpenAI response failed without an error message".to_string()),
    }
}

fn decode_incomplete(response: Option<Value>) -> ModelProviderError {
    let reason = response
        .as_ref()
        .and_then(|value| value.get("incomplete_details"))
        .and_then(|value| value.get("reason"))
        .and_then(Value::as_str)
        .unwrap_or("unknown");

    ModelProviderError::ProviderFailure {
        code: Some("response_incomplete".to_string()),
        message: format!("OpenAI response was incomplete: {reason}"),
    }
}

fn decoded(events: Vec<ModelEvent>, completed: bool) -> DecodedResponsesEvent {
    DecodedResponsesEvent { events, completed }
}

fn protocol_error(message: impl Into<String>) -> ModelProviderError {
    ModelProviderError::ProtocolFailure {
        message: message.into(),
    }
}

#[derive(Debug, Deserialize)]
struct FunctionCallItem {
    call_id: String,
    name: String,
    arguments: String,
}

#[derive(Debug, Deserialize)]
struct CompletedResponse {
    #[serde(rename = "id")]
    _id: String,
    #[serde(default)]
    usage: Option<ResponseUsage>,
}

#[derive(Debug, Deserialize)]
struct ResponseUsage {
    input_tokens: u64,
    output_tokens: u64,
}

#[derive(Debug, Deserialize)]
struct ProviderErrorBody {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    message: Option<String>,
}
