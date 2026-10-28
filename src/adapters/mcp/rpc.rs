use super::McpError;
use serde_json::Value;
pub(super) const MAX_MESSAGE_BYTES: usize = 2 * 1024 * 1024;

pub(crate) enum CompletedCall {
    Result(Value),
    Error(Value),
}

pub(super) fn decode_message(value: Value, expected_id: &str) -> Result<CompletedCall, McpError> {
    if value.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
        || value.get("id").and_then(Value::as_str) != Some(expected_id)
    {
        return Err(McpError::Protocol {
            message: "MCP response did not match its JSON-RPC request".to_owned(),
        });
    }
    match (value.get("result"), value.get("error")) {
        (Some(result), None) if result.is_object() => Ok(CompletedCall::Result(result.clone())),
        (None, Some(error)) if error.is_object() => Ok(CompletedCall::Error(error.clone())),
        _ => Err(McpError::Protocol {
            message: "MCP response did not contain one valid JSON-RPC result or error".to_owned(),
        }),
    }
}

pub(super) fn into_result(completed: CompletedCall) -> Result<Value, McpError> {
    match completed {
        CompletedCall::Result(result) => Ok(result),
        CompletedCall::Error(error) => Err(McpError::Catalog {
            message: format!("initialize returned JSON-RPC error: {error}"),
        }),
    }
}

pub(super) fn is_server_request(value: &Value) -> bool {
    value.get("method").is_some() && value.get("id").is_some()
}

pub(super) fn is_notification(value: &Value) -> bool {
    value.get("method").is_some() && value.get("id").is_none()
}
