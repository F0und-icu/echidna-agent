//! Terminal error-event validation and its bounded diagnostic projection.

use serde_json::{Number, Value};

use crate::ports::model::ModelProviderError;

use super::{PREVIOUS_RESPONSE_NOT_FOUND_CODE, ResponsesWireProfile, protocol_error};

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
) -> ModelProviderError {
    let value = match serde_json::from_str::<Value>(data) {
        Ok(value) => value,
        Err(_) => return protocol_error("invalid Responses error event JSON"),
    };
    let fields = match fields(&value) {
        Ok(fields) => fields,
        Err(error) => return error,
    };
    if let Some(error) = super::capacity_failure(fields.code) {
        return error;
    }
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
        Some(message) => message.to_owned(),
    };
    let param = fields
        .param
        .map(str::to_owned)
        .unwrap_or_else(|| "<absent>".to_owned());
    let sequence_number = fields
        .sequence_number
        .map(ToString::to_string)
        .unwrap_or_else(|| "<absent>".to_owned());
    ModelProviderError::ProviderFailure {
        code: fields.code.map(str::to_owned),
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

/// Capacity classification shares the terminal error field validator. Other
/// wire paths retain their original failure category when validation fails.
pub(super) fn validated_code(value: &Value) -> Result<Option<&str>, ModelProviderError> {
    fields(value).map(|fields| fields.code)
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
