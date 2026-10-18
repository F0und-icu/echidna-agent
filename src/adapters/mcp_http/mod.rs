//! Fixed configured Streamable HTTP MCP tools. The Adapter owns wire DTOs,
//! catalog bounds and header rules; the Tool bridge owns ToolSpec mapping.

mod headers;
mod protocol;

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;

use crate::adapters::credentials::CredentialSource;

use self::headers::HeaderBinding;
pub(crate) use self::protocol::CompletedCall;
use self::protocol::McpClient;

const MAX_LIST_PAGES: usize = 64;
const MAX_LIST_TOOLS: usize = 1024;
const MAX_LIST_MATERIAL_BYTES: usize = 8 * 1024 * 1024;

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

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct McpServerConfig {
    pub(crate) url: String,
    pub(crate) protocol_version: McpProtocolVersion,
    pub(crate) credential_ref: Option<String>,
    #[serde(default = "default_timeout_ms")]
    pub(crate) timeout_ms: u64,
}

const fn default_timeout_ms() -> u64 {
    60_000
}

pub(crate) struct McpRuntime {
    servers: BTreeMap<String, McpServerConfig>,
    credentials: Arc<dyn CredentialSource>,
}

impl McpRuntime {
    pub(crate) fn new(
        servers: BTreeMap<String, McpServerConfig>,
        credentials: Arc<dyn CredentialSource>,
    ) -> Self {
        Self {
            servers,
            credentials,
        }
    }

    pub(crate) async fn discover(&self) -> Result<Vec<McpCatalogTool>, McpError> {
        let mut tools = Vec::new();
        for (server_id, config) in &self.servers {
            let bearer = config
                .credential_ref
                .as_deref()
                .map(|reference| {
                    self.credentials
                        .read(reference)
                        .map_err(|error| McpError::Credential {
                            server_id: server_id.clone(),
                            message: error.to_string(),
                        })?
                        .filter(|value| !value.trim().is_empty())
                        .ok_or_else(|| McpError::Credential {
                            server_id: server_id.clone(),
                            message: "configured MCP credential is unavailable".to_owned(),
                        })
                })
                .transpose()?;
            let client = Arc::new(McpClient::new(
                config.url.clone(),
                config.protocol_version,
                config.timeout_ms,
                bearer,
            )?);
            client.initialize().await?;
            tools.extend(discover_server(server_id, config, client).await?);
        }
        Ok(tools)
    }
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

async fn discover_server(
    server_id: &str,
    config: &McpServerConfig,
    client: Arc<McpClient>,
) -> Result<Vec<McpCatalogTool>, McpError> {
    let mut cursor = None;
    let mut seen_cursors = std::collections::BTreeSet::new();
    let mut names = std::collections::BTreeSet::new();
    let mut material = 0_usize;
    let mut returned_tools = 0_usize;
    let mut tools = Vec::new();
    for _ in 0..MAX_LIST_PAGES {
        let result = client.list_tools(cursor.as_deref()).await?;
        material = material.saturating_add(
            serde_json::to_vec(&result)
                .map_err(|_| McpError::Catalog {
                    message: "tools/list result could not be measured".to_owned(),
                })?
                .len(),
        );
        if material > MAX_LIST_MATERIAL_BYTES {
            return Err(McpError::Catalog {
                message: format!("MCP server `{server_id}` exceeded 8 MiB tools/list material"),
            });
        }
        let page = result.as_object().ok_or_else(|| McpError::Catalog {
            message: format!("MCP server `{server_id}` tools/list result was not an object"),
        })?;
        let definitions =
            page.get("tools")
                .and_then(Value::as_array)
                .ok_or_else(|| McpError::Catalog {
                    message: format!(
                        "MCP server `{server_id}` tools/list result had no tools array"
                    ),
                })?;
        for definition in definitions {
            returned_tools += 1;
            if returned_tools > MAX_LIST_TOOLS {
                return Err(McpError::Catalog {
                    message: format!("MCP server `{server_id}` exceeded 1024 tools"),
                });
            }
            let remote_name = definition
                .get("name")
                .and_then(Value::as_str)
                .filter(|name| !name.is_empty())
                .ok_or_else(|| McpError::Catalog {
                    message: format!(
                        "MCP server `{server_id}` returned a tool without a non-empty name"
                    ),
                })?;
            if !names.insert(remote_name.to_owned()) {
                return Err(McpError::Catalog {
                    message: format!(
                        "MCP server `{server_id}` returned duplicate tool `{remote_name}`"
                    ),
                });
            }
            let input_schema = definition
                .get("inputSchema")
                .filter(|schema| schema.is_object())
                .cloned()
                .ok_or_else(|| McpError::Catalog {
                    message: format!("MCP tool `{remote_name}` has no object inputSchema"),
                })?;
            let header_bindings = if config.protocol_version == McpProtocolVersion::Modern20260728 {
                match headers::bindings(&input_schema, remote_name) {
                    Ok(bindings) => bindings,
                    Err(McpError::InvalidAnnotatedTool { message, .. }) => {
                        eprintln!("[MCP tool excluded: {remote_name}: {message}]");
                        continue;
                    }
                    Err(error) => return Err(error),
                }
            } else {
                Vec::new()
            };
            tools.push(McpCatalogTool {
                server_id: server_id.to_owned(),
                endpoint: config.url.clone(),
                protocol_version: config.protocol_version,
                remote_name: remote_name.to_owned(),
                definition: definition.clone(),
                input_schema,
                timeout_ms: config.timeout_ms,
                header_bindings,
                client: Arc::clone(&client),
            });
        }
        let next = match page.get("nextCursor") {
            None | Some(Value::Null) => None,
            Some(Value::String(cursor)) if !cursor.is_empty() => Some(cursor.clone()),
            _ => {
                return Err(McpError::Catalog {
                    message: format!("MCP server `{server_id}` returned an invalid nextCursor"),
                });
            }
        };
        let Some(next) = next else {
            return Ok(tools);
        };
        if !seen_cursors.insert(next.clone()) {
            return Err(McpError::Catalog {
                message: format!("MCP server `{server_id}` repeated a tools/list cursor"),
            });
        }
        cursor = Some(next);
    }
    Err(McpError::Catalog {
        message: format!("MCP server `{server_id}` exceeded 64 tools/list pages"),
    })
}

#[derive(Debug, Error)]
pub(crate) enum McpError {
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
