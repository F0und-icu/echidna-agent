//! Sequential configured-server discovery and owned transport lifecycle.
use super::observation::{McpObservation, McpObserver, McpStartupStage};
#[cfg(unix)]
use super::stdio;
use super::{McpCatalogTool, McpClient, McpError, McpProtocolVersion, McpServerConfig, headers};
use crate::adapters::credentials::CredentialSource;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;
#[cfg(unix)]
use std::sync::{Mutex, Weak};
use std::time::Duration;
const MAX_LIST_PAGES: usize = 64;
const MAX_LIST_TOOLS: usize = 1024;
const MAX_LIST_MATERIAL_BYTES: usize = 8 * 1024 * 1024;

pub(crate) struct McpRuntime {
    servers: BTreeMap<String, McpServerConfig>,
    credentials: Arc<dyn CredentialSource>,
    observer: Option<McpObserver>,
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
            observer: None,
            #[cfg(unix)]
            stdio_clients: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn with_observer(mut self, observer: Option<McpObserver>) -> Self {
        self.observer = observer;
        self
    }

    fn observe(&self, server_id: &str, stage: McpStartupStage) {
        if let Some(observer) = &self.observer {
            observer(McpObservation::Startup {
                server_id: server_id.to_owned(),
                stage,
            });
        }
    }

    pub(crate) async fn discover(&self) -> Result<Vec<McpCatalogTool>, McpError> {
        let mut tools = Vec::new();
        for (server_id, config) in &self.servers {
            let mut stage = McpStartupStage::Starting;
            self.observe(server_id, stage);
            let startup = async {
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
                    (
                        McpClient::Http(super::http::McpClient::new(
                            server_id.clone(),
                            url.clone(),
                            config.protocol_version,
                            config.timeout_ms,
                            bearer,
                            config.startup_timeout_ms(),
                            self.observer.clone(),
                        )?),
                        url.clone(),
                    )
                } else {
                    #[cfg(unix)]
                    {
                        let client = Arc::new(super::stdio::StdioClient::start(
                            server_id,
                            config,
                            self.observer.clone(),
                        )?);
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
                stage = McpStartupStage::Initializing;
                self.observe(server_id, stage);
                client.initialize().await?;
                stage = McpStartupStage::Listing;
                self.observe(server_id, stage);
                discover_server(server_id, config, endpoint, client, self.observer.as_ref()).await
            };
            let discovered =
                tokio::time::timeout(Duration::from_millis(config.startup_timeout_ms()), startup)
                    .await
                    .map_err(|_| McpError::Catalog {
                        message: "startup deadline exceeded".to_owned(),
                    })
                    .and_then(|result| result);
            match discovered {
                Ok(discovered) => {
                    self.observe(server_id, McpStartupStage::Ready);
                    tools.extend(discovered);
                }
                Err(source) => {
                    if let Some(observer) = &self.observer {
                        observer(McpObservation::StartupFailed {
                            server_id: server_id.clone(),
                            stage,
                        });
                    }
                    return Err(McpError::Startup {
                        server_id: server_id.clone(),
                        stage,
                        source: Box::new(source),
                    });
                }
            }
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

async fn discover_server(
    server_id: &str,
    config: &McpServerConfig,
    endpoint: String,
    client: Arc<McpClient>,
    observer: Option<&McpObserver>,
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
                        if let Some(observer) = observer {
                            observer(McpObservation::ToolExcluded {
                                server_id: server_id.to_owned(),
                                name: remote_name.to_owned(),
                                message,
                            });
                        }
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
