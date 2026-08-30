//! Shared Responses streaming-event wire decoding.

use serde::Deserialize;
use serde_json::Value;

use crate::domain::model::{ModelEvent, ModelUsage};
use crate::domain::tools::{ToolCall, ToolCallId, ToolName};
use crate::ports::model::{ModelProviderError, ModelResponseCompletion, ProviderResponseId};

const PREVIOUS_RESPONSE_NOT_FOUND_CODE: &str = "previous_response_not_found";

#[derive(Debug)]
pub(super) struct DecodedResponsesEvent {
    pub events: Vec<ModelEvent>,
    pub completion: Option<ModelResponseCompletion>,
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
    continuation_required: bool,
    continuation_requested: bool,
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
            Ok(decoded(vec![ModelEvent::TextDelta { text }]))
        }
        "response.reasoning_text.delta" => {
            let text = event
                .delta
                .ok_or_else(|| protocol_error("reasoning text delta event omitted `delta`"))?;
            Ok(decoded(vec![ModelEvent::ReasoningDelta { text }]))
        }
        "response.output_item.done" => {
            let item = event
                .item
                .ok_or_else(|| protocol_error("output item event omitted `item`"))?;
            decode_output_item(item)
        }
        "response.completed" => {
            decode_completed(event.response, usage_required, continuation_required)
        }
        "response.failed" => Err(decode_provider_failure(
            event.response,
            continuation_requested,
        )),
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
        | "response.reasoning_summary_text.done"
        | "response.reasoning_text.done" => Ok(decoded(Vec::new())),
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

            Ok(decoded(vec![ModelEvent::ToolCall {
                call: ToolCall::new(
                    ToolCallId::new(function_call.call_id),
                    ToolName::new(function_call.name),
                    arguments,
                ),
            }]))
        }
        "message" => Ok(decoded(Vec::new())),
        "reasoning" => decode_reasoning_item(item),
        unknown => Err(protocol_error(format!(
            "unsupported completed output item `{unknown}`"
        ))),
    }
}

fn decode_reasoning_item(item: Value) -> Result<DecodedResponsesEvent, ModelProviderError> {
    let reasoning: ReasoningItem = serde_json::from_value(item)
        .map_err(|error| protocol_error(format!("invalid completed reasoning item: {error}")))?;
    let has_encrypted_content = reasoning
        .encrypted_content
        .as_ref()
        .is_some_and(has_non_empty_wire_content);

    let mut text = String::new();
    for content in reasoning.content {
        let kind = content
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| protocol_error("reasoning content omitted string `type`"))?;
        if kind != "reasoning_text" {
            return Err(protocol_error(format!(
                "unsupported completed reasoning content `{kind}`"
            )));
        }
        let part = content
            .get("text")
            .and_then(Value::as_str)
            .ok_or_else(|| protocol_error("reasoning text content omitted string `text`"))?;
        text.push_str(part);
    }

    if text.is_empty() {
        if has_encrypted_content {
            Err(protocol_error(
                "completed reasoning item contains only unsupported encrypted content",
            ))
        } else {
            Ok(decoded(Vec::new()))
        }
    } else {
        // Some conforming services return an opaque encrypted sidecar next to
        // complete reasoning_text. The supported plaintext is sufficient for
        // generic output-item replay, so the wire-only sidecar stays inside
        // this Adapter and never widens the provider-neutral Core.
        Ok(decoded(vec![ModelEvent::ReasoningItem { text }]))
    }
}

fn has_non_empty_wire_content(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::String(value) => !value.is_empty(),
        Value::Array(values) => !values.is_empty(),
        Value::Object(values) => !values.is_empty(),
        Value::Bool(_) | Value::Number(_) => true,
    }
}

fn decode_completed(
    response: Option<Value>,
    usage_required: bool,
    continuation_required: bool,
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
    let completion = if response.id.trim().is_empty() {
        if continuation_required {
            return Err(protocol_error(
                "completed response omitted a non-empty response identity required for incremental input",
            ));
        }
        ModelResponseCompletion::without_response_id()
    } else {
        ModelResponseCompletion::new(ProviderResponseId::new(response.id))
    };

    Ok(DecodedResponsesEvent {
        events,
        completion: Some(completion),
    })
}

pub(super) fn continuation_rejected_http_body(body: &str) -> bool {
    serde_json::from_str::<HttpErrorEnvelope>(body)
        .ok()
        .and_then(|envelope| envelope.error)
        .and_then(|error| error.code)
        .as_deref()
        == Some(PREVIOUS_RESPONSE_NOT_FOUND_CODE)
}

fn decode_provider_failure(
    response: Option<Value>,
    continuation_requested: bool,
) -> ModelProviderError {
    let error = response
        .as_ref()
        .and_then(|value| value.get("error"))
        .cloned()
        .and_then(|value| serde_json::from_value::<ProviderErrorBody>(value).ok());

    let code = error.as_ref().and_then(|error| error.code.clone());
    if continuation_requested && code.as_deref() == Some(PREVIOUS_RESPONSE_NOT_FOUND_CODE) {
        return ModelProviderError::ContinuationRejected {
            code: PREVIOUS_RESPONSE_NOT_FOUND_CODE.to_owned(),
        };
    }

    ModelProviderError::ProviderFailure {
        code,
        message: error
            .and_then(|error| error.message)
            .filter(|message| !message.trim().is_empty())
            .unwrap_or_else(|| "Responses provider failed without an error message".to_string()),
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
        message: format!("Responses provider response was incomplete: {reason}"),
    }
}

fn decoded(events: Vec<ModelEvent>) -> DecodedResponsesEvent {
    DecodedResponsesEvent {
        events,
        completion: None,
    }
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
struct ReasoningItem {
    #[serde(default)]
    content: Vec<Value>,
    #[serde(default)]
    encrypted_content: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct CompletedResponse {
    #[serde(rename = "id")]
    #[serde(default)]
    id: String,
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

#[derive(Debug, Deserialize)]
struct HttpErrorEnvelope {
    #[serde(default)]
    error: Option<ProviderErrorBody>,
}
