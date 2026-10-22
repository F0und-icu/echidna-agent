//! Fixed configured HTTP/stdio MCP tools. The Adapter owns wire DTOs,
//! catalog bounds and header rules; the Tool bridge owns ToolSpec mapping.

mod headers;
mod protocol;
#[cfg(unix)]
mod stdio;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
#[cfg(unix)]
use std::sync::{Mutex, Weak};
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;

use crate::adapters::credentials::CredentialSource;

use self::headers::HeaderBinding;
pub(crate) use self::protocol::CompletedCall;

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
    Http(protocol::McpClient),
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

pub(crate) struct McpRuntime {
    servers: BTreeMap<String, McpServerConfig>,
    credentials: Arc<dyn CredentialSource>,
    #[cfg(unix)]
    stdio_clients: Mutex<Vec<Weak<stdio::StdioClient>>>,
}

impl McpRuntime {
    pub(crate) fn new(
        servers: BTreeMap<String, McpServerConfig>,
        credentials: Arc<dyn CredentialSource>,
    ) -> Self {
        Self {
            servers,
            credentials,
            #[cfg(unix)]
            stdio_clients: Mutex::new(Vec::new()),
        }
    }

    pub(crate) async fn discover(&self) -> Result<Vec<McpCatalogTool>, McpError> {
        let mut tools = Vec::new();
        for (server_id, config) in &self.servers {
            let (client, endpoint) = if let Some(url) = &config.url {
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
                let client = McpClient::Http(protocol::McpClient::new(
                    url.clone(),
                    config.protocol_version,
                    config.timeout_ms,
                    bearer,
                )?);
                (client, url.clone())
            } else {
                #[cfg(unix)]
                {
                    let client = Arc::new(stdio::StdioClient::start(config)?);
                    self.stdio_clients
                        .lock()
                        .expect("MCP lifecycle mutex poisoned")
                        .push(Arc::downgrade(&client));
                    let endpoint = client.identity().to_owned();
                    (McpClient::Stdio(client), endpoint)
                }
                #[cfg(not(unix))]
                return Err(McpError::Catalog {
                    message: "stdio MCP requires Unix".to_owned(),
                });
            };
            let client = Arc::new(client);
            let discovery = async {
                client.initialize().await?;
                discover_server(server_id, config, endpoint, Arc::clone(&client)).await
            };
            let discovered = if config.command.is_some() {
                tokio::time::timeout(Duration::from_millis(config.timeout_ms), discovery)
                    .await
                    .map_err(|_| McpError::Catalog {
                        message: format!("MCP server `{server_id}` startup timed out"),
                    })??
            } else {
                discovery.await?
            };
            tools.extend(discovered);
        }
        Ok(tools)
    }
}

impl Drop for McpRuntime {
    fn drop(&mut self) {
        #[cfg(unix)]
        for client in self
            .stdio_clients
            .get_mut()
            .expect("MCP lifecycle mutex poisoned")
            .iter()
            .filter_map(Weak::upgrade)
        {
            client.shutdown();
        }
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
    endpoint: String,
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
                endpoint: endpoint.clone(),
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
