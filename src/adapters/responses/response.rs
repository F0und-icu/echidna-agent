//! Shared Responses streaming-event wire decoding.

mod text;

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
    #[serde(default)]
    output_index: Option<usize>,
    #[serde(default)]
    content_index: Option<usize>,
    #[serde(default)]
    item_id: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    part: Option<Value>,
}

#[derive(Default)]
pub(super) struct ResponseDecoder {
    text: text::TextOutput,
    completed_items: Vec<(Option<usize>, Vec<ModelEvent>)>,
}

impl ResponseDecoder {
    pub(super) fn decode_stream_event(
        &mut self,
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
                Ok(decoded(self.text.text(
                    event.output_index,
                    event.content_index,
                    event.item_id.as_deref(),
                    &text,
                    false,
                )?))
            }
            "response.output_text.done" => {
                let text = event
                    .text
                    .ok_or_else(|| protocol_error("final text event omitted `text`"))?;
                Ok(decoded(self.text.text(
                    event.output_index,
                    event.content_index,
                    event.item_id.as_deref(),
                    &text,
                    true,
                )?))
            }
            "response.content_part.done" => {
                let part = event
                    .part
                    .ok_or_else(|| protocol_error("content part event omitted `part`"))?;
                if part.get("type").and_then(Value::as_str) == Some("reasoning_text") {
                    // Like reasoning_text.done, this is a lifecycle marker.
                    // The completed reasoning output-item owner validates and
                    // emits its plaintext once; it is never Assistant text.
                    return Ok(decoded(Vec::new()));
                }
                Ok(decoded(self.text.part(
                    event.output_index,
                    event.content_index,
                    event.item_id.as_deref(),
                    &part,
                )?))
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
                if item.get("type").and_then(Value::as_str) == Some("message") {
                    Ok(decoded(self.text.message(event.output_index, &item)?))
                } else {
                    let decoded = decode_output_item(item)?;
                    if !decoded.events.is_empty() {
                        self.text.barrier(event.output_index);
                    }
                    self.completed_items
                        .push((event.output_index, decoded.events.clone()));
                    Ok(decoded)
                }
            }
            "response.completed" => {
                let mut decoded = decode_completed(
                    event.response.as_ref(),
                    usage_required,
                    continuation_required,
                )?;
                if let Some(output) = event.response.as_ref().and_then(|r| r.get("output")) {
                    let output = output
                        .as_array()
                        .ok_or_else(|| protocol_error("completed output is not an array"))?;
                    let mut text_events = Vec::new();
                    let mut completed_items = self.completed_items.iter();
                    for (index, item) in output.iter().enumerate() {
                        if item.get("type").and_then(Value::as_str) == Some("message") {
                            text_events.extend(self.text.message(Some(index), item)?);
                        } else {
                            let decoded = decode_output_item(item.clone())?;
                            if !completed_items.next().is_some_and(|(prior_index, prior)| {
                                prior_index.is_none_or(|prior| prior == index)
                                    && prior == &decoded.events
                            }) {
                                return Err(protocol_error(
                                    "completed output disagrees with completed non-text items",
                                ));
                            }
                        }
                    }
                    if completed_items.next().is_some() {
                        return Err(protocol_error(
                            "completed output omitted completed non-text items",
                        ));
                    }
                    self.text.validate_snapshot(output)?;
                    text_events.append(&mut decoded.events);
                    decoded.events = text_events;
                }
                Ok(decoded)
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
}

fn decode_output_item(item: Value) -> Result<DecodedResponsesEvent, ModelProviderError> {
    if item.get("status").is_some_and(|s| s != "completed") {
        return Err(protocol_error(
            "completed output item has non-completed status",
        ));
    }
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
                // Invalid inner JSON cannot be structurally redacted. Report
                // coordinates and size only, never a payload snippet or serde's
                // token-bearing error text. Strict decoding remains the owner.
                protocol_error(format!(
                    "function call arguments are not valid JSON: category={:?}, line={}, column={}, arguments_bytes={}; rejected before dispatch; malformed payload omitted",
                    error.classify(), error.line(), error.column(), function_call.arguments.len()
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
    response: Option<&Value>,
    usage_required: bool,
    continuation_required: bool,
) -> Result<DecodedResponsesEvent, ModelProviderError> {
    let response = CompletedResponse::deserialize(
        response.ok_or_else(|| protocol_error("completed event omitted `response`"))?,
    )
    .map_err(|error| protocol_error(format!("invalid completed response: {error}")))?;

    let mut events = Vec::with_capacity(2);
    if response
        .status
        .as_deref()
        .is_some_and(|status| status != "completed")
    {
        return Err(protocol_error(
            "completed response has non-completed status",
        ));
    }
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
    #[serde(default)]
    status: Option<String>,
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
