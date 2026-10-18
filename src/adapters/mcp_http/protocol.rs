use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderName, HeaderValue};
use serde_json::{Value, json};

use super::headers::{HeaderBinding, encode_header_value, request_headers};
use super::{McpError, McpProtocolVersion};

const MAX_HTTP_BODY_BYTES: usize = 2 * 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const MCP_PROTOCOL_VERSION: HeaderName = HeaderName::from_static("mcp-protocol-version");
const MCP_METHOD: HeaderName = HeaderName::from_static("mcp-method");
const MCP_NAME: HeaderName = HeaderName::from_static("mcp-name");
const MCP_SESSION_ID: HeaderName = HeaderName::from_static("mcp-session-id");

#[derive(Clone)]
pub(super) struct McpClient {
    endpoint: String,
    protocol: McpProtocolVersion,
    client: reqwest::Client,
    bearer: Option<String>,
    session_id: Arc<Mutex<Option<String>>>,
    next_id: Arc<AtomicU64>,
}

pub(crate) enum CompletedCall {
    Result(Value),
    Error(Value),
}

impl McpClient {
    pub(super) fn new(
        endpoint: String,
        protocol: McpProtocolVersion,
        timeout_ms: u64,
        bearer: Option<String>,
    ) -> Result<Self, McpError> {
        let client = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(Duration::from_millis(timeout_ms))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| McpError::Transport {
                message: error.to_string(),
            })?;
        Ok(Self {
            endpoint,
            protocol,
            client,
            bearer,
            session_id: Arc::new(Mutex::new(None)),
            next_id: Arc::new(AtomicU64::new(1)),
        })
    }

    pub(super) async fn initialize(&self) -> Result<(), McpError> {
        if self.protocol != McpProtocolVersion::Legacy20251125 {
            return Ok(());
        }
        let result = self
            .request(
                "initialize",
                json!({
                    "protocolVersion": self.protocol.as_str(),
                    "capabilities": {},
                    "clientInfo": {"name": "echidna", "version": env!("CARGO_PKG_VERSION")}
                }),
                None,
            )
            .await?;
        let result = into_result(result)?;
        if result.get("protocolVersion").and_then(Value::as_str) != Some(self.protocol.as_str())
            || !result
                .get("capabilities")
                .and_then(Value::as_object)
                .and_then(|capabilities| capabilities.get("tools"))
                .is_some_and(Value::is_object)
        {
            return Err(McpError::Catalog {
                message:
                    "legacy initialize did not confirm the configured protocol and tools capability"
                        .to_owned(),
            });
        }
        self.notification("notifications/initialized", json!({}))
            .await
    }

    pub(super) async fn list_tools(&self, cursor: Option<&str>) -> Result<Value, McpError> {
        let mut params = serde_json::Map::new();
        if let Some(cursor) = cursor {
            params.insert("cursor".to_owned(), Value::String(cursor.to_owned()));
        }
        match self
            .request("tools/list", Value::Object(params), None)
            .await?
        {
            CompletedCall::Result(result) => Ok(result),
            CompletedCall::Error(error) => Err(McpError::Catalog {
                message: format!("tools/list returned JSON-RPC error: {}", safe_error(&error)),
            }),
        }
    }

    pub(super) async fn call(
        &self,
        remote_name: &str,
        arguments: Value,
        headers: &[HeaderBinding],
    ) -> Result<CompletedCall, McpError> {
        let parameter_headers = request_headers(headers, &arguments)?;
        let mut params = serde_json::Map::new();
        params.insert("name".to_owned(), Value::String(remote_name.to_owned()));
        params.insert("arguments".to_owned(), arguments);
        if self.protocol == McpProtocolVersion::Modern20260728 {
            params.insert(
                "_meta".to_owned(),
                json!({
                    "io.modelcontextprotocol/protocolVersion": self.protocol.as_str(),
                    "io.modelcontextprotocol/clientInfo": {"name": "echidna", "version": env!("CARGO_PKG_VERSION")},
                    "io.modelcontextprotocol/clientCapabilities": {}
                }),
            );
        }
        self.request(
            "tools/call",
            Value::Object(params),
            Some((remote_name, parameter_headers)),
        )
        .await
    }

    async fn notification(&self, method: &str, params: Value) -> Result<(), McpError> {
        let response = self
            .post(
                json!({"jsonrpc": "2.0", "method": method, "params": params}),
                method,
                None,
            )
            .await?;
        if response.status() != reqwest::StatusCode::ACCEPTED {
            return Err(McpError::Catalog {
                message: format!("{method} was not accepted"),
            });
        }
        Ok(())
    }

    async fn request(
        &self,
        method: &str,
        params: Value,
        call_headers: Option<(&str, Vec<(String, String)>)>,
    ) -> Result<CompletedCall, McpError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed).to_string();
        let body = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let response = self.post(body, method, call_headers).await?;
        self.decode_response(response, &id).await
    }

    async fn post(
        &self,
        body: Value,
        method: &str,
        call_headers: Option<(&str, Vec<(String, String)>)>,
    ) -> Result<reqwest::Response, McpError> {
        let mut request = self
            .client
            .post(&self.endpoint)
            .header(ACCEPT, "application/json, text/event-stream")
            .header(CONTENT_TYPE, "application/json")
            .json(&body);
        if let Some(bearer) = &self.bearer {
            request = request.header(AUTHORIZATION, format!("Bearer {bearer}"));
        }
        if self.protocol == McpProtocolVersion::Modern20260728 {
            request = request
                .header(MCP_PROTOCOL_VERSION, self.protocol.as_str())
                .header(MCP_METHOD, method);
        } else if method != "initialize" {
            request = request.header(MCP_PROTOCOL_VERSION, self.protocol.as_str());
        }
        if let Some(session_id) = self
            .session_id
            .lock()
            .expect("MCP session mutex poisoned")
            .clone()
        {
            request = request.header(MCP_SESSION_ID, session_id);
        }
        if let Some((name, headers)) = call_headers
            && self.protocol == McpProtocolVersion::Modern20260728
        {
            request = request.header(MCP_NAME, encode_header_value(name));
            for (header, value) in headers {
                let name = HeaderName::from_bytes(header.as_bytes()).map_err(|_| {
                    McpError::PreExecution {
                        message: "invalid generated MCP parameter header name".to_owned(),
                    }
                })?;
                let value = HeaderValue::from_str(&value).map_err(|_| McpError::PreExecution {
                    message: "invalid generated MCP parameter header value".to_owned(),
                })?;
                request = request.header(name, value);
            }
        }
        request.send().await.map_err(|error| McpError::Transport {
            message: error.to_string(),
        })
    }

    async fn decode_response(
        &self,
        response: reqwest::Response,
        id: &str,
    ) -> Result<CompletedCall, McpError> {
        let session = response
            .headers()
            .get(MCP_SESSION_ID)
            .and_then(|value| value.to_str().ok())
            .map(ToOwned::to_owned);
        if let Some(session) = session {
            if session.is_empty()
                || session.len() > 256
                || !session.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
            {
                return Err(McpError::Catalog {
                    message: "MCP server returned an invalid session identifier".to_owned(),
                });
            }
            *self.session_id.lock().expect("MCP session mutex poisoned") = Some(session);
        }
        if response.status() == reqwest::StatusCode::NOT_FOUND
            && self
                .session_id
                .lock()
                .expect("MCP session mutex poisoned")
                .is_some()
        {
            return Err(McpError::SessionExpired);
        }
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        if content_type.starts_with("application/json") {
            let value = bounded_json(response).await?;
            return decode_message(value, id);
        }
        if content_type.starts_with("text/event-stream") {
            return self.decode_sse(response, id).await;
        }
        Err(McpError::Protocol {
            message: "MCP response used an unsupported content type".to_owned(),
        })
    }

    async fn decode_sse(
        &self,
        response: reqwest::Response,
        id: &str,
    ) -> Result<CompletedCall, McpError> {
        let size = Arc::new(AtomicU64::new(0));
        let counted = response.bytes_stream().map({
            let size = Arc::clone(&size);
            move |chunk| {
                let chunk = chunk.map_err(|error| std::io::Error::other(error.to_string()))?;
                let total =
                    size.fetch_add(chunk.len() as u64, Ordering::Relaxed) + chunk.len() as u64;
                if total > MAX_HTTP_BODY_BYTES as u64 {
                    return Err(std::io::Error::other("MCP response exceeds 2 MiB"));
                }
                Ok(chunk)
            }
        });
        let mut events = counted.eventsource();
        while let Some(event) = events.next().await {
            let event = event.map_err(|error| McpError::Protocol {
                message: error.to_string(),
            })?;
            if event.data.is_empty() {
                continue;
            }
            let value: Value =
                serde_json::from_str(&event.data).map_err(|_| McpError::Protocol {
                    message: "MCP SSE event was not JSON-RPC JSON".to_owned(),
                })?;
            if is_server_request(&value) {
                self.respond_to_server_request(&value).await?;
                continue;
            }
            if is_notification(&value) {
                let method = value
                    .get("method")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if method == "notifications/message" || method == "notifications/progress" {
                    eprintln!("[MCP transport notification: {method}]");
                    continue;
                }
                return Err(McpError::Protocol {
                    message: "MCP SSE sent an unsupported notification".to_owned(),
                });
            }
            return decode_message(value, id);
        }
        Err(McpError::Protocol {
            message: "MCP SSE ended before a final JSON-RPC response".to_owned(),
        })
    }

    async fn respond_to_server_request(&self, request: &Value) -> Result<(), McpError> {
        let id = request
            .get("id")
            .cloned()
            .ok_or_else(|| McpError::Protocol {
                message: "MCP server request had no id".to_owned(),
            })?;
        let method = request
            .get("method")
            .and_then(Value::as_str)
            .ok_or_else(|| McpError::Protocol {
                message: "MCP server request had no method".to_owned(),
            })?;
        let body = if method == "ping" {
            json!({"jsonrpc": "2.0", "id": id, "result": {}})
        } else {
            eprintln!("[MCP transport unsupported server request: {method}]");
            json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "Method not found"}})
        };
        let response = self.post(body, "response", None).await?;
        if response.status() != reqwest::StatusCode::ACCEPTED {
            return Err(McpError::Protocol {
                message: "MCP server-request response was not accepted".to_owned(),
            });
        }
        Ok(())
    }
}

fn decode_message(value: Value, expected_id: &str) -> Result<CompletedCall, McpError> {
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

fn into_result(completed: CompletedCall) -> Result<Value, McpError> {
    match completed {
        CompletedCall::Result(result) => Ok(result),
        CompletedCall::Error(error) => Err(McpError::Catalog {
            message: format!("initialize returned JSON-RPC error: {}", safe_error(&error)),
        }),
    }
}

fn is_server_request(value: &Value) -> bool {
    value.get("method").is_some() && value.get("id").is_some()
}

fn is_notification(value: &Value) -> bool {
    value.get("method").is_some() && value.get("id").is_none()
}

async fn bounded_json(response: reqwest::Response) -> Result<Value, McpError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_HTTP_BODY_BYTES as u64)
    {
        return Err(McpError::Protocol {
            message: "MCP response exceeds 2 MiB".to_owned(),
        });
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| McpError::Transport {
            message: error.to_string(),
        })?;
        if bytes.len().saturating_add(chunk.len()) > MAX_HTTP_BODY_BYTES {
            return Err(McpError::Protocol {
                message: "MCP response exceeds 2 MiB".to_owned(),
            });
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| McpError::Protocol {
        message: "MCP response was not valid JSON".to_owned(),
    })
}

fn safe_error(error: &Value) -> String {
    error
        .get("message")
        .and_then(Value::as_str)
        .map(|message| message.chars().take(160).collect())
        .unwrap_or_else(|| "invalid JSON-RPC error".to_owned())
}
