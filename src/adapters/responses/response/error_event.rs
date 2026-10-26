//! Terminal error-event validation and its bounded diagnostic projection.

use serde_json::{Number, Value};

use super::super::redaction::WireRedaction;
use crate::diagnostic_text::escaped_text;
use crate::ports::model::ModelProviderError;

use super::{PREVIOUS_RESPONSE_NOT_FOUND_CODE, ResponsesWireProfile, protocol_error};

pub(super) struct ErrorRedaction {
    redaction: WireRedaction,
}

impl ErrorRedaction {
    pub(super) fn new(bearer: &str, account_id: Option<&[u8]>) -> Self {
        let account = account_id.map(|account_id| {
            std::str::from_utf8(account_id)
                .expect("account header was constructed from a UTF-8 credential")
        });
        Self {
            redaction: WireRedaction::new(&[bearer, account.unwrap_or("")]),
        }
    }

    fn project(&self, text: &str, limit: usize) -> String {
        // Replace whole request values before selecting a bounded prefix, so a
        // value straddling that prefix cannot leak its credential fragment.
        let redacted = self.redaction.text(text);
        let mut characters = redacted.chars();
        let mut bounded: String = characters.by_ref().take(limit).collect();
        if characters.next().is_some() {
            bounded.push_str("... [truncated]");
        }
        escaped_text(&bounded)
    }
}

struct ErrorFields<'a> {
    code: Option<&'a str>,
    message: Option<&'a str>,
    param: Option<&'a str>,
    sequence_number: Option<&'a Number>,
    origin: &'static str,
}

pub(super) fn decode(
    data: &str,
    profile: ResponsesWireProfile,
    continuation_requested: bool,
    redaction: &ErrorRedaction,
) -> ModelProviderError {
    let value = match serde_json::from_str::<Value>(data) {
        Ok(value) => value,
        Err(_) => return protocol_error("invalid Responses error event JSON"),
    };
    let fields = match fields(&value) {
        Ok(fields) => fields,
        Err(error) => return error,
    };
    if !profile.is_subscription()
        && continuation_requested
        && fields.code == Some(PREVIOUS_RESPONSE_NOT_FOUND_CODE)
    {
        return ModelProviderError::ContinuationRejected {
            code: PREVIOUS_RESPONSE_NOT_FOUND_CODE.to_owned(),
        };
    }
    let message = match fields.message {
        None => "<missing>".to_owned(),
        Some(message) if message.trim().is_empty() => "<empty>".to_owned(),
        Some(message) => redaction.project(message, 1024),
    };
    let param = fields
        .param
        .map(|param| redaction.project(param, 256))
        .unwrap_or_else(|| "<absent>".to_owned());
    let sequence_number = fields
        .sequence_number
        .map(ToString::to_string)
        .unwrap_or_else(|| "<absent>".to_owned());
    ModelProviderError::ProviderFailure {
        code: fields.code.map(|code| redaction.project(code, 128)),
        message: format!(
            "Responses SSE error (origin={}; param={param}; sequence_number={sequence_number}): message={message}",
            fields.origin,
        ),
    }
}

fn fields(value: &Value) -> Result<ErrorFields<'_>, ModelProviderError> {
    let (details, origin) = match value.get("error") {
        None => (value, "top-level"),
        Some(details) if details.is_object() => (details, "nested error"),
        Some(_) => {
            return Err(protocol_error(
                "Responses error event `error` must be an object",
            ));
        }
    };
    let message = match details.get("message") {
        None => None,
        Some(Value::String(message)) => Some(message.as_str()),
        Some(_) => {
            return Err(protocol_error(
                "Responses error event `message` must be a string",
            ));
        }
    };
    let sequence_number = match value.get("sequence_number") {
        None => None,
        Some(Value::Number(sequence_number)) => Some(sequence_number),
        Some(_) => {
            return Err(protocol_error(
                "Responses error event `sequence_number` must be a number",
            ));
        }
    };
    Ok(ErrorFields {
        code: nullable_string(details, "code")?,
        message,
        param: nullable_string(details, "param")?,
        sequence_number,
        origin,
    })
}

fn nullable_string<'a>(
    value: &'a Value,
    field: &str,
) -> Result<Option<&'a str>, ModelProviderError> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.as_str())),
        Some(_) => Err(protocol_error(format!(
            "Responses error event `{field}` must be a string or null"
        ))),
    }
}
