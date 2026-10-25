//! Shared Responses streaming-event wire decoding.

mod error_event;
mod text;

use serde::Deserialize;
use serde_json::Value;

use crate::domain::model::{
    ModelEvent, ModelUsage, OpaqueContinuationReference, OpaqueContinuationScope,
    ToolArgumentDiagnostic, ToolArgumentErrorCategory,
};
use crate::domain::tools::{ToolCall, ToolCallId, ToolName};
use crate::ports::model::{ModelProviderError, ModelResponseCompletion, ProviderResponseId};

use super::ResponsesWireProfile;
use super::request::{ECHIDNA_TOOL_NAMESPACE, SubscriptionOpaqueReasoning};

const PREVIOUS_RESPONSE_NOT_FOUND_CODE: &str = "previous_response_not_found";
const SUBSCRIPTION_QUOTA_CODE: &str = "insufficient_quota";

#[derive(Debug)]
pub(super) struct DecodedResponsesEvent {
    pub events: Vec<ModelEvent>,
    pub reasoning_summaries: Vec<ReasoningSummary>,
    pub completion: Option<ModelResponseCompletion>,
    pub opaque_continuations: Vec<DecodedOpaqueContinuation>,
}

#[derive(Debug)]
pub(super) struct ReasoningSummary {
    pub text: String,
    pub starts_part: bool,
}

#[derive(Debug)]
pub(super) struct DecodedOpaqueContinuation {
    pub reference: OpaqueContinuationReference,
    pub item: SubscriptionOpaqueReasoning,
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
    summary_index: Option<Value>,
    #[serde(default)]
    item_id: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    part: Option<Value>,
}

pub(super) struct ResponseDecoder {
    text: text::TextOutput,
    reasoning_summary: text::TextOutput,
    last_presented_reasoning_summary_part: Option<(usize, usize)>,
    completed_items: Vec<(Option<usize>, Vec<ModelEvent>)>,
    profile: ResponsesWireProfile,
    opaque_scope: Option<OpaqueContinuationScope>,
    error_redaction: error_event::ErrorRedaction,
}

impl ResponseDecoder {
    pub(super) fn new(
        profile: ResponsesWireProfile,
        opaque_scope: Option<OpaqueContinuationScope>,
        bearer: &str,
        account_id: Option<&[u8]>,
    ) -> Self {
        Self {
            text: text::TextOutput::default(),
            reasoning_summary: text::TextOutput::default(),
            last_presented_reasoning_summary_part: None,
            completed_items: Vec::new(),
            profile,
            opaque_scope,
            error_redaction: error_event::ErrorRedaction::new(bearer, account_id),
        }
    }

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
            "response.reasoning_summary_text.delta"
                if self.profile == ResponsesWireProfile::CodexSubscription =>
            {
                let output_index = summary_output_index(&event)?;
                let summary_index = summary_index(&event)?;
                let item_id = summary_item_id(&event)?;
                let text = event.delta.as_deref().ok_or_else(|| {
                    protocol_error("reasoning summary text delta event omitted `delta`")
                })?;
                Ok(decoded_summary(self.reasoning_summary_suffix(
                    output_index,
                    summary_index,
                    item_id,
                    text,
                    false,
                )?))
            }
            "response.reasoning_summary_text.done"
                if self.profile == ResponsesWireProfile::CodexSubscription =>
            {
                let output_index = summary_output_index(&event)?;
                let summary_index = summary_index(&event)?;
                let item_id = summary_item_id(&event)?;
                let text = event.text.as_deref().ok_or_else(|| {
                    protocol_error("final reasoning summary text event omitted `text`")
                })?;
                Ok(decoded_summary(self.reasoning_summary_suffix(
                    output_index,
                    summary_index,
                    item_id,
                    text,
                    true,
                )?))
            }
            "response.output_item.done" => {
                let item = event
                    .item
                    .ok_or_else(|| protocol_error("output item event omitted `item`"))?;
                let reasoning_summaries =
                    self.reasoning_summaries_from_item(event.output_index, &item)?;
                if item.get("type").and_then(Value::as_str) == Some("message") {
                    let mut decoded = decoded(self.text.message(event.output_index, &item)?);
                    decoded.reasoning_summaries = reasoning_summaries;
                    Ok(decoded)
                } else {
                    let mut decoded = decode_output_item(
                        item,
                        self.profile,
                        self.opaque_scope.as_ref(),
                        CompletedItemSource::OutputItemDone,
                    )?;
                    if !decoded.events.is_empty() {
                        self.text.barrier(event.output_index);
                    }
                    self.completed_items
                        .push((event.output_index, decoded.events.clone()));
                    decoded.reasoning_summaries = reasoning_summaries;
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
                    let mut reasoning_summaries = Vec::new();
                    let mut completed_item_index = 0;
                    for (index, item) in output.iter().enumerate() {
                        if item.get("type").and_then(Value::as_str) == Some("message") {
                            text_events.extend(self.text.message(Some(index), item)?);
                        } else {
                            reasoning_summaries
                                .extend(self.reasoning_summaries_from_item(Some(index), item)?);
                            let decoded = decode_output_item(
                                item.clone(),
                                self.profile,
                                self.opaque_scope.as_ref(),
                                CompletedItemSource::CompletedSnapshot,
                            )?;
                            if !self.completed_items.get(completed_item_index).is_some_and(
                                |(prior_index, prior)| {
                                    prior_index.is_none_or(|prior| prior == index)
                                        && prior == &decoded.events
                                },
                            ) {
                                return Err(protocol_error(
                                    "completed output disagrees with completed non-text items",
                                ));
                            }
                            completed_item_index += 1;
                        }
                    }
                    if completed_item_index < self.completed_items.len() {
                        return Err(protocol_error(
                            "completed output omitted completed non-text items",
                        ));
                    }
                    self.text.validate_snapshot(output)?;
                    self.reasoning_summary
                        .validate_reasoning_summary_snapshot(output)?;
                    text_events.append(&mut decoded.events);
                    decoded.events = text_events;
                    decoded.reasoning_summaries = reasoning_summaries;
                }
                Ok(decoded)
            }
            "error" => Err(error_event::decode(
                data,
                self.profile,
                continuation_requested,
                &self.error_redaction,
            )),
            "response.failed" => Err(decode_provider_failure(
                self.profile,
                event.response,
                continuation_requested,
            )),
            "response.incomplete" => Err(decode_incomplete(self.profile, event.response)),
            "response.metadata" | "keepalive"
                if self.profile == ResponsesWireProfile::CodexSubscription =>
            {
                Ok(decoded(Vec::new()))
            }
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

    fn reasoning_summaries_from_item(
        &mut self,
        output_index: Option<usize>,
        item: &Value,
    ) -> Result<Vec<ReasoningSummary>, ModelProviderError> {
        if self.profile != ResponsesWireProfile::CodexSubscription
            || item.get("type").and_then(Value::as_str) != Some("reasoning")
        {
            return Ok(Vec::new());
        }
        let Some(summary) = item.get("summary") else {
            return Ok(Vec::new());
        };
        let summary = summary
            .as_array()
            .ok_or_else(|| protocol_error("completed reasoning item summary is not an array"))?;
        if summary.is_empty() {
            return Ok(Vec::new());
        }
        let output_index = output_index
            .ok_or_else(|| protocol_error("completed reasoning summary omitted `output_index`"))?;
        let item_id = item
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| protocol_error("completed reasoning summary omitted non-empty `id`"))?;
        let mut suffixes = Vec::new();
        for (summary_index, part) in summary.iter().enumerate() {
            if part.get("type").and_then(Value::as_str) != Some("summary_text") {
                return Err(protocol_error(
                    "completed reasoning summary has unsupported content",
                ));
            }
            let text = part.get("text").and_then(Value::as_str).ok_or_else(|| {
                protocol_error("completed reasoning summary omitted string `text`")
            })?;
            if let Some(suffix) =
                self.reasoning_summary_suffix(output_index, summary_index, item_id, text, true)?
            {
                suffixes.push(suffix);
            }
        }
        Ok(suffixes)
    }

    fn reasoning_summary_suffix(
        &mut self,
        output_index: usize,
        summary_index: usize,
        item_id: &str,
        text: &str,
        final_text: bool,
    ) -> Result<Option<ReasoningSummary>, ModelProviderError> {
        let position = (output_index, summary_index);
        let starts_part = self.last_presented_reasoning_summary_part != Some(position);
        let suffix = self.reasoning_summary.text_suffix(
            Some(output_index),
            Some(summary_index),
            Some(item_id),
            text,
            final_text,
        )?;
        if suffix.is_some() {
            self.last_presented_reasoning_summary_part = Some(position);
        }
        Ok(suffix.map(|text| ReasoningSummary { text, starts_part }))
    }
}

fn summary_output_index(event: &ResponsesStreamEvent) -> Result<usize, ModelProviderError> {
    event
        .output_index
        .ok_or_else(|| protocol_error("reasoning summary text event omitted `output_index`"))
}

fn summary_index(event: &ResponsesStreamEvent) -> Result<usize, ModelProviderError> {
    event
        .summary_index
        .as_ref()
        .and_then(Value::as_u64)
        .and_then(|index| usize::try_from(index).ok())
        .ok_or_else(|| protocol_error("reasoning summary text event omitted valid `summary_index`"))
}

fn summary_item_id(event: &ResponsesStreamEvent) -> Result<&str, ModelProviderError> {
    event
        .item_id
        .as_deref()
        .filter(|item_id| !item_id.is_empty())
        .ok_or_else(|| protocol_error("reasoning summary text event omitted non-empty `item_id`"))
}

fn decode_output_item(
    item: Value,
    profile: ResponsesWireProfile,
    opaque_scope: Option<&OpaqueContinuationScope>,
    source: CompletedItemSource,
) -> Result<DecodedResponsesEvent, ModelProviderError> {
    if let Some((item_family, status_class)) = invalid_completed_item_status(&item) {
        return Err(protocol_error(format!(
            "completed output item has non-completed status (source={} item_family={} status_class={})",
            source.label(),
            item_family.label(),
            status_class.label(),
        )));
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
            match profile {
                ResponsesWireProfile::ChatGptSubscription
                    if function_call.namespace.as_deref() != Some(ECHIDNA_TOOL_NAMESPACE) =>
                {
                    return Err(protocol_error(
                        "subscription function call omitted the advertised `echidna` namespace",
                    ));
                }
                ResponsesWireProfile::CodexSubscription if function_call.namespace.is_some() => {
                    return Err(protocol_error(
                        "Codex subscription function call included an unsupported namespace",
                    ));
                }
                _ => {}
            }
            if function_call.call_id.is_empty() || function_call.name.is_empty() {
                return Err(protocol_error(
                    "completed function call requires non-empty `call_id` and `name`",
                ));
            }
            let arguments = serde_json::from_str(&function_call.arguments).map_err(|error| {
                // Invalid inner JSON cannot be structurally redacted. Report
                // coordinates and size only, never a payload snippet or serde's
                // token-bearing error text. Strict decoding remains the owner.
                let category = match error.classify() {
                    serde_json::error::Category::Syntax => ToolArgumentErrorCategory::Syntax,
                    serde_json::error::Category::Data => ToolArgumentErrorCategory::Data,
                    serde_json::error::Category::Eof => ToolArgumentErrorCategory::Eof,
                    serde_json::error::Category::Io => {
                        return protocol_error("unexpected argument reader failure");
                    }
                };
                ModelProviderError::InvalidToolArguments {
                    diagnostic: ToolArgumentDiagnostic {
                        category,
                        line: error.line(),
                        column: error.column(),
                        arguments_bytes: function_call.arguments.len(),
                    },
                }
            })?;

            Ok(decoded(vec![ModelEvent::ToolCall {
                call: ToolCall::new(
                    ToolCallId::new(function_call.call_id),
                    ToolName::new(function_call.name),
                    arguments,
                ),
            }]))
        }
        "reasoning" => decode_reasoning_item(item, profile, opaque_scope),
        unknown => Err(protocol_error(format!(
            "unsupported completed output item `{unknown}`"
        ))),
    }
}

#[derive(Clone, Copy)]
enum CompletedItemSource {
    OutputItemDone,
    CompletedSnapshot,
}

impl CompletedItemSource {
    fn label(self) -> &'static str {
        match self {
            Self::OutputItemDone => "output_item_done",
            Self::CompletedSnapshot => "completed_snapshot",
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum CompletedItemFamily {
    Reasoning,
    FunctionCall,
    Other,
}

impl CompletedItemFamily {
    fn label(self) -> &'static str {
        match self {
            Self::Reasoning => "reasoning",
            Self::FunctionCall => "function_call",
            Self::Other => "other",
        }
    }
}

#[derive(Clone, Copy)]
enum CompletedItemStatusClass {
    KnownNonCompleted,
    NullOutsideReasoning,
    UnknownString,
    NonString,
}

impl CompletedItemStatusClass {
    fn label(self) -> &'static str {
        match self {
            Self::KnownNonCompleted => "known_non_completed",
            Self::NullOutsideReasoning => "null_outside_reasoning",
            Self::UnknownString => "unknown_string",
            Self::NonString => "non_string",
        }
    }
}

fn invalid_completed_item_status(
    item: &Value,
) -> Option<(CompletedItemFamily, CompletedItemStatusClass)> {
    let item_family = match item.get("type").and_then(Value::as_str) {
        Some("reasoning") => CompletedItemFamily::Reasoning,
        Some("function_call") => CompletedItemFamily::FunctionCall,
        _ => CompletedItemFamily::Other,
    };
    let status_class = match item.get("status") {
        None => return None,
        Some(Value::String(status)) if status == "completed" => return None,
        Some(Value::Null) if item_family == CompletedItemFamily::Reasoning => return None,
        Some(Value::String(status))
            if matches!(
                status.as_str(),
                "in_progress" | "incomplete" | "failed" | "cancelled"
            ) =>
        {
            CompletedItemStatusClass::KnownNonCompleted
        }
        Some(Value::String(_)) => CompletedItemStatusClass::UnknownString,
        Some(Value::Null) => CompletedItemStatusClass::NullOutsideReasoning,
        Some(_) => CompletedItemStatusClass::NonString,
    };
    Some((item_family, status_class))
}

fn decode_reasoning_item(
    item: Value,
    profile: ResponsesWireProfile,
    opaque_scope: Option<&OpaqueContinuationScope>,
) -> Result<DecodedResponsesEvent, ModelProviderError> {
    let reasoning: ReasoningItem = serde_json::from_value(item.clone())
        .map_err(|error| protocol_error(format!("invalid completed reasoning item: {error}")))?;
    let has_encrypted_content = reasoning
        .encrypted_content
        .as_ref()
        .is_some_and(has_non_empty_wire_content);

    let mut text = String::new();
    for content in reasoning.content.unwrap_or_default() {
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

    if profile.is_subscription() && has_encrypted_content {
        let opaque = SubscriptionOpaqueReasoning::from_completed_item(&item).map_err(|error| {
            protocol_error(format!(
                "invalid subscription encrypted reasoning item: {error}"
            ))
        })?;
        let scope = opaque_scope
            .cloned()
            .ok_or_else(|| protocol_error("subscription reasoning item omitted delivery scope"))?;
        let reference = opaque.reference(scope).map_err(|error| {
            protocol_error(format!(
                "invalid subscription encrypted reasoning item: {error}"
            ))
        })?;
        return Ok(DecodedResponsesEvent {
            events: vec![ModelEvent::OpaqueContinuation {
                reference: reference.clone(),
            }],
            reasoning_summaries: Vec::new(),
            completion: None,
            opaque_continuations: vec![DecodedOpaqueContinuation {
                reference,
                item: opaque,
            }],
        });
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
        reasoning_summaries: Vec::new(),
        completion: Some(completion),
        opaque_continuations: Vec::new(),
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
    profile: ResponsesWireProfile,
    response: Option<Value>,
    continuation_requested: bool,
) -> ModelProviderError {
    if profile.is_subscription() {
        return ModelProviderError::ProviderFailure {
            code: subscription_error_code(response.as_ref()),
            message: format!(
                "{} Responses stream reported response.failed",
                profile.subscription_label()
            ),
        };
    }
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

fn decode_incomplete(profile: ResponsesWireProfile, response: Option<Value>) -> ModelProviderError {
    if profile.is_subscription() {
        return ModelProviderError::ProviderFailure {
            code: Some("response_incomplete".to_owned()),
            message: format!(
                "{} Responses stream reported response.incomplete",
                profile.subscription_label()
            ),
        };
    }
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
        reasoning_summaries: Vec::new(),
        completion: None,
        opaque_continuations: Vec::new(),
    }
}

fn decoded_summary(summary: Option<ReasoningSummary>) -> DecodedResponsesEvent {
    let mut decoded = decoded(Vec::new());
    if let Some(summary) = summary {
        decoded.reasoning_summaries.push(summary);
    }
    decoded
}

pub(super) enum BoundedHttpErrorBody {
    Complete(String),
    Unreadable,
    Oversized,
}

/// Subscription admission failures expose only a fixed local classification,
/// the HTTP status, and the one code the runtime already handles explicitly.
pub(super) fn subscription_http_failure(
    profile: ResponsesWireProfile,
    status: u16,
    body: &BoundedHttpErrorBody,
) -> ModelProviderError {
    let (code, suffix) = match body {
        BoundedHttpErrorBody::Unreadable => (None, "error body unavailable"),
        BoundedHttpErrorBody::Oversized => (None, "error body exceeds 65536 bytes"),
        BoundedHttpErrorBody::Complete(body) => {
            match serde_json::from_str::<SubscriptionHttpErrorEnvelope>(body) {
                Err(_) => (None, "malformed error body"),
                Ok(envelope) => match envelope.error.and_then(|error| error.code) {
                    None => (None, "missing error code"),
                    Some(code) if code == SUBSCRIPTION_QUOTA_CODE => (Some(code), ""),
                    Some(_) => (None, "unrecognized error code"),
                },
            }
        }
    };
    ModelProviderError::ProviderFailure {
        code,
        message: if suffix.is_empty() {
            format!(
                "{} Responses request failed with HTTP {status}",
                profile.subscription_label()
            )
        } else {
            format!(
                "{} Responses request failed with HTTP {status}: {suffix}",
                profile.subscription_label()
            )
        },
    }
}

fn subscription_error_code(response: Option<&Value>) -> Option<String> {
    response
        .and_then(|response| response.get("error"))
        .and_then(|error| error.get("code"))
        .and_then(Value::as_str)
        .filter(|code| *code == SUBSCRIPTION_QUOTA_CODE)
        .map(str::to_owned)
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
    #[serde(default)]
    namespace: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ReasoningItem {
    #[serde(default)]
    content: Option<Vec<Value>>,
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

#[derive(Debug, Deserialize)]
struct SubscriptionHttpErrorEnvelope {
    #[serde(default)]
    error: Option<SubscriptionHttpError>,
}

#[derive(Debug, Deserialize)]
struct SubscriptionHttpError {
    #[serde(default)]
    code: Option<String>,
}
