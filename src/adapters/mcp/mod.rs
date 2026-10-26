//! Fixed configured HTTP/stdio MCP tools. The Adapter owns wire DTOs,
//! catalog bounds and header rules; the Tool bridge owns ToolSpec mapping.

mod headers;
mod http;
mod lifecycle;
pub(crate) mod observation;
mod rpc;
pub(crate) use lifecycle::McpRuntime;
#[cfg(unix)]
mod stdio;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;

use self::headers::HeaderBinding;
pub(crate) use self::rpc::CompletedCall;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
pub(crate) enum McpProtocolVersion {
    #[serde(rename = "2025-11-25")]
    Legacy20251125,
    #[serde(rename = "2026-07-28")]
    Modern20260728,
}

impl McpProtocolVersion {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Legacy20251125 => "2025-11-25",
            Self::Modern20260728 => "2026-07-28",
        }
    }
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct McpServerConfig {
    pub(crate) url: Option<String>,
    pub(crate) command: Option<PathBuf>,
    pub(crate) args: Option<Vec<String>>,
    pub(crate) env: Option<BTreeMap<String, String>>,
    pub(crate) cwd: Option<PathBuf>,
    pub(crate) protocol_version: McpProtocolVersion,
    pub(crate) credential_ref: Option<String>,
    #[serde(default = "default_timeout_ms")]
    pub(crate) timeout_ms: u64,
    pub(crate) startup_timeout_ms: Option<u64>,
}

impl std::fmt::Debug for McpServerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpServerConfig")
            .field("stdio", &self.command.is_some())
            .field("protocol_version", &self.protocol_version)
            .field("timeout_ms", &self.timeout_ms)
            .finish_non_exhaustive()
    }
}

impl McpServerConfig {
    fn startup_timeout_ms(&self) -> u64 {
        self.startup_timeout_ms.unwrap_or(self.timeout_ms)
    }

    pub(crate) fn resolve_relative_to(&mut self, parent: &Path) {
        if let Some(command) = &mut self.command {
            if !command.as_os_str().is_empty()
                && command.is_relative()
                && (command.as_os_str().as_encoded_bytes().contains(&b'/')
                    || !matches!(
                        command.components().next(),
                        Some(std::path::Component::Normal(_))
                    ))
            {
                *command = parent.join(&*command);
            }
            let cwd = self.cwd.get_or_insert_with(|| parent.to_path_buf());
            if !cwd.as_os_str().is_empty() && cwd.is_relative() {
                *cwd = parent.join(&*cwd);
            }
        }
    }
}

enum McpClient {
    Http(http::McpClient),
    #[cfg(unix)]
    Stdio(Arc<stdio::StdioClient>),
}

impl McpClient {
    async fn initialize(&self) -> Result<(), McpError> {
        match self {
            Self::Http(client) => client.initialize().await,
            #[cfg(unix)]
            Self::Stdio(client) => client.initialize().await,
        }
    }
    async fn list_tools(&self, cursor: Option<&str>) -> Result<Value, McpError> {
        match self {
            Self::Http(client) => client.list_tools(cursor).await,
            #[cfg(unix)]
            Self::Stdio(client) => client.list_tools(cursor).await,
        }
    }
    async fn call(
        &self,
        name: &str,
        arguments: Value,
        headers: &[HeaderBinding],
    ) -> Result<CompletedCall, McpError> {
        match self {
            Self::Http(client) => client.call(name, arguments, headers).await,
            #[cfg(unix)]
            Self::Stdio(client) => client.call(name, arguments).await,
        }
    }
}

const fn default_timeout_ms() -> u64 {
    60_000
}

#[derive(Clone)]
pub(crate) struct McpCatalogTool {
    pub(crate) server_id: String,
    pub(crate) endpoint: String,
    pub(crate) protocol_version: McpProtocolVersion,
    pub(crate) remote_name: String,
    pub(crate) definition: Value,
    pub(crate) input_schema: Value,
    pub(crate) timeout_ms: u64,
    pub(crate) header_bindings: Vec<HeaderBinding>,
    client: Arc<McpClient>,
}

impl McpCatalogTool {
    pub(crate) async fn call(&self, arguments: Value) -> Result<CompletedCall, McpError> {
        self.client
            .call(&self.remote_name, arguments, &self.header_bindings)
            .await
    }
}

#[derive(Debug, Error)]
pub(crate) enum McpError {
    #[error("MCP server `{server_id}` startup failed during {stage}: {source}")]
    Startup {
        server_id: String,
        stage: observation::McpStartupStage,
        source: Box<McpError>,
    },
    #[error("MCP server `{server_id}` credential failed: {message}")]
    Credential { server_id: String, message: String },
    #[error("MCP catalog failed: {message}")]
    Catalog { message: String },
    #[error("MCP annotated tool `{tool_name}` is invalid: {message}")]
    InvalidAnnotatedTool { tool_name: String, message: String },
    #[error("MCP call arguments are invalid: {message}")]
    PreExecution { message: String },
    #[error("MCP session expired before a final result was observed")]
    SessionExpired,
    #[error("MCP transport failed: {message}")]
    Transport { message: String },
    #[error("MCP protocol failed: {message}")]
    Protocol { message: String },
}
