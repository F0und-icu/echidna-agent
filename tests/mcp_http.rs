//! Loopback coverage for configured Streamable HTTP MCP discovery and calls.
//!
//! This test compiles the library as its crate so the production-only
//! composition and Adapter boundary stay crate-private.

include!("../src/lib.rs");

#[path = "support/mod.rs"]
mod support;

mod mcp_http_regression {
    use std::collections::{BTreeMap, HashMap, VecDeque};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use base64::Engine;
    use futures_util::stream;
    use serde_json::{Value, json};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::mpsc;
    use tokio::task::JoinHandle;

    use crate::adapters::credentials::{CredentialSource, CredentialSourceError};
    use crate::app::agent_loop::{AgentLoop, AgentLoopError};
    use crate::app::cancellation::AgentCancellation;
    use crate::domain::model::{
        CapabilityMappingVersion, ContentItem, ModelCapabilities, ModelEvent,
    };
    use crate::domain::tools::{
        ExecutionId, ToolCall, ToolCallId, ToolEffect, ToolExposure, ToolName, ToolResultStatus,
    };
    use crate::ports::model::{
        ModelEventStream, ModelProvider, ModelProviderError, ModelRequest, ModelResponseCompletion,
        ModelStreamEvent,
    };
    use crate::ports::tool::{ToolExecutionError, ToolExecutor};
    use crate::provider_composition::{ProviderComposition, ProviderCompositionError};
    use crate::support::TemporaryDirectory;

    #[derive(Default)]
    struct RecordingCredentials {
        values: BTreeMap<String, Option<String>>,
        reads: Mutex<Vec<String>>,
    }

    impl RecordingCredentials {
        fn with_values(
            values: impl IntoIterator<Item = (&'static str, &'static str)>,
        ) -> Arc<Self> {
            Arc::new(Self {
                values: values
                    .into_iter()
                    .map(|(name, value)| (name.to_owned(), Some(value.to_owned())))
                    .collect(),
                reads: Mutex::new(Vec::new()),
            })
        }

        fn reads(&self) -> Vec<String> {
            self.reads.lock().expect("credential reads mutex").clone()
        }
    }

    impl CredentialSource for RecordingCredentials {
        fn read(&self, reference: &str) -> Result<Option<String>, CredentialSourceError> {
            self.reads
                .lock()
                .expect("credential reads mutex")
                .push(reference.to_owned());
            Ok(self.values.get(reference).cloned().flatten())
        }
    }

    struct CapturedRequest {
        method: String,
        headers: HashMap<String, String>,
        body: Value,
    }

    struct Response {
        status: u16,
        headers: Vec<(&'static str, String)>,
        content_type: &'static str,
        body: String,
        declared_length: Option<usize>,
        delay: Duration,
        tolerate_closed_peer: bool,
    }

    impl Response {
        fn json(body: Value) -> Self {
            Self {
                status: 200,
                headers: Vec::new(),
                content_type: "application/json",
                body: body.to_string(),
                declared_length: None,
                delay: Duration::ZERO,
                tolerate_closed_peer: false,
            }
        }

        fn accepted() -> Self {
            Self {
                status: 202,
                headers: Vec::new(),
                content_type: "application/json",
                body: String::new(),
                declared_length: None,
                delay: Duration::ZERO,
                tolerate_closed_peer: false,
            }
        }

        fn sse(message: Value) -> Self {
            Self {
                status: 200,
                headers: Vec::new(),
                content_type: "text/event-stream",
                body: format!("data: {message}\n\n"),
                declared_length: None,
                delay: Duration::ZERO,
                tolerate_closed_peer: false,
            }
        }

        fn sse_messages(messages: impl IntoIterator<Item = Value>) -> Self {
            Self {
                status: 200,
                headers: Vec::new(),
                content_type: "text/event-stream",
                body: messages
                    .into_iter()
                    .map(|message| format!("data: {message}\n\n"))
                    .collect(),
                declared_length: None,
                delay: Duration::ZERO,
                tolerate_closed_peer: false,
            }
        }

        fn raw(content_type: &'static str, body: impl Into<String>) -> Self {
            Self {
                status: 200,
                headers: Vec::new(),
                content_type,
                body: body.into(),
                declared_length: None,
                delay: Duration::ZERO,
                tolerate_closed_peer: false,
            }
        }

        fn with_header(mut self, name: &'static str, value: impl Into<String>) -> Self {
            self.headers.push((name, value.into()));
            self
        }

        fn with_status(mut self, status: u16) -> Self {
            self.status = status;
            self
        }

        fn with_declared_length(mut self, length: usize) -> Self {
            self.declared_length = Some(length);
            self
        }

        fn delayed(mut self, delay: Duration) -> Self {
            self.delay = delay;
            self.tolerate_closed_peer = true;
            self
        }

        fn tolerate_closed_peer(mut self) -> Self {
            self.tolerate_closed_peer = true;
            self
        }
    }

    struct LoopbackServer {
        url: String,
        requests: mpsc::Receiver<CapturedRequest>,
        task: JoinHandle<()>,
    }

    async fn spawn_server(responses: Vec<Response>) -> LoopbackServer {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind MCP loopback listener");
        let address = listener.local_addr().expect("MCP listener address");
        let (sender, requests) = mpsc::channel(responses.len());
        let task = tokio::spawn(async move {
            for response in responses {
                let (mut socket, _) = listener.accept().await.expect("MCP accepts connection");
                sender
                    .send(read_request(&mut socket).await)
                    .await
                    .expect("MCP test receives request");
                let tolerate_closed_peer = response.tolerate_closed_peer;
                if let Err(error) = write_response(&mut socket, &response).await
                    && !tolerate_closed_peer
                {
                    panic!("write MCP response: {error}");
                }
            }
        });
        LoopbackServer {
            url: format!("http://{address}/mcp"),
            requests,
            task,
        }
    }

    async fn read_request(socket: &mut TcpStream) -> CapturedRequest {
        let mut received = Vec::new();
        let mut buffer = [0_u8; 1024];
        let (header_end, content_length) = loop {
            let count = socket.read(&mut buffer).await.expect("read MCP request");
            assert!(count > 0, "MCP client closed before a complete request");
            received.extend_from_slice(&buffer[..count]);
            assert!(
                received.len() <= 1024 * 1024,
                "MCP fixture request is bounded"
            );
            if let Some(index) = received.windows(4).position(|window| window == b"\r\n\r\n") {
                let header_end = index + 4;
                let header = std::str::from_utf8(&received[..header_end])
                    .expect("MCP request headers are UTF-8");
                let content_length = header
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().expect("valid content length"))
                    })
                    .unwrap_or_default();
                if received.len() >= header_end + content_length {
                    break (header_end, content_length);
                }
            }
        };
        let header = std::str::from_utf8(&received[..header_end]).expect("MCP headers are UTF-8");
        let headers = header
            .lines()
            .skip(1)
            .filter_map(|line| {
                let (name, value) = line.split_once(':')?;
                Some((name.to_ascii_lowercase(), value.trim().to_owned()))
            })
            .collect();
        let method = header
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().next())
            .expect("MCP request line has a method")
            .to_owned();
        CapturedRequest {
            method,
            headers,
            body: serde_json::from_slice(&received[header_end..header_end + content_length])
                .expect("MCP request body is JSON"),
        }
    }

    async fn write_response(socket: &mut TcpStream, response: &Response) -> std::io::Result<()> {
        let reason = match response.status {
            200 => "OK",
            202 => "Accepted",
            404 => "Not Found",
            _ => "Fixture",
        };
        let headers = response
            .headers
            .iter()
            .map(|(name, value)| format!("{name}: {value}\r\n"))
            .collect::<String>();
        let head = format!(
            "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n{}\r\n",
            response.status,
            reason,
            response.content_type,
            response.declared_length.unwrap_or(response.body.len()),
            headers
        );
        if !response.delay.is_zero() {
            tokio::time::sleep(response.delay).await;
        }
        socket.write_all(head.as_bytes()).await?;
        socket.write_all(response.body.as_bytes()).await?;
        socket.shutdown().await
    }

    fn provider_config(mcp_servers: &str) -> String {
        format!(
            r#"schema_version = 1
active_agent_profile = "default"

[providers.primary]
binding = "http_sse"
wire_api = "responses"
base_url = "http://127.0.0.1:1/v1"
model = "fixture-model"
credential_ref = "CYBER_AGENT_PRIMARY_API_KEY"
capability_mapping = "text-v1"
generation = 1

[capability_mappings.text-v1]
version = 1
text_input = true
image_input = false
streaming = true
tool_call = true
structured_output = true
incremental_input = false
usage_reporting = true
context_window = 32768

[agent_profiles.default]
primary = "primary"
vision_mode = "disabled"
generation = 1

{mcp_servers}
"#
        )
    }

    fn load_composition(
        directory: &TemporaryDirectory,
        mcp_servers: &str,
        credentials: Arc<RecordingCredentials>,
    ) -> ProviderComposition {
        let config = directory.path().join("provider.toml");
        std::fs::write(&config, provider_config(mcp_servers)).expect("write MCP fixture config");
        ProviderComposition::load(config, credentials).expect("MCP fixture config is valid")
    }

    fn tool_definition(name: &str) -> Value {
        json!({
            "name": name,
            "description": "fixture tool",
            "inputSchema": {
                "$defs": {
                    "Execution": {
                        "type": "object",
                        "properties": {"token": {"type": "string"}},
                        "required": ["token"]
                    }
                },
                "type": "object",
                "properties": {
                    "execution": {"$ref": "#/$defs/Execution"},
                    "trace": {"type": "string", "x-mcp-header": "Trace"}
                },
                "required": ["execution"]
            }
        })
    }

    fn model_capabilities() -> ModelCapabilities {
        ModelCapabilities {
            text_input: true,
            image_input: false,
            streaming: true,
            tool_call: true,
            structured_output: true,
            incremental_input: false,
            usage_reporting: true,
            context_window: 32_768,
            max_output_tokens: None,
            reasoning_effort: None,
            capability_mapping_version: CapabilityMappingVersion::new(1),
        }
    }

    struct ScriptedProvider {
        responses: Mutex<VecDeque<Vec<ModelEvent>>>,
        requests: Mutex<Vec<ModelRequest>>,
        request_count: AtomicUsize,
    }

    impl ScriptedProvider {
        fn new(responses: impl IntoIterator<Item = Vec<ModelEvent>>) -> Self {
            Self {
                responses: Mutex::new(responses.into_iter().collect()),
                requests: Mutex::new(Vec::new()),
                request_count: AtomicUsize::new(0),
            }
        }

        fn requests(&self) -> Vec<ModelRequest> {
            self.requests
                .lock()
                .expect("MCP model requests mutex")
                .clone()
        }
    }

    impl ModelProvider for ScriptedProvider {
        fn capabilities(&self) -> ModelCapabilities {
            model_capabilities()
        }

        fn stream(&self, request: ModelRequest) -> Result<ModelEventStream, ModelProviderError> {
            request.validate_capabilities(&self.capabilities())?;
            self.request_count.fetch_add(1, Ordering::SeqCst);
            self.requests
                .lock()
                .expect("MCP model requests mutex")
                .push(request);
            let events = self
                .responses
                .lock()
                .expect("MCP model responses mutex")
                .pop_front()
                .ok_or_else(|| ModelProviderError::ProviderFailure {
                    code: Some("fixture_exhausted".to_owned()),
                    message: "the MCP fixture must not request a third model sample".to_owned(),
                })?;
            Ok(Box::pin(stream::iter(
                events
                    .into_iter()
                    .map(|event| Ok(ModelStreamEvent::Event(event)))
                    .chain(std::iter::once(Ok(ModelStreamEvent::Completed(
                        ModelResponseCompletion::without_response_id(),
                    )))),
            )))
        }
    }

    fn contains_tool(request: &ModelRequest, name: &ToolName) -> bool {
        request
            .input()
            .tool_specs
            .iter()
            .any(|spec| &spec.name == name)
    }

    #[tokio::test]
    async fn modern_catalog_keeps_schema_root_arguments_and_bearers_isolated() {
        let alpha_definition = tool_definition("remote.alpha");
        let invalid_definition = json!({
            "name": "remote.invalid-header",
            "inputSchema": {
                "type": "object",
                "properties": {"bad": {"type": "number", "x-mcp-header": "Bad"}}
            }
        });
        let duplicate_headers = json!({
            "name": "remote.duplicate-headers",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "first": {"type": "string", "x-mcp-header": "Trace"},
                    "second": {"type": "string", "x-mcp-header": "trace"}
                }
            }
        });
        let ref_path_header = json!({
            "name": "remote.ref-path-header",
            "inputSchema": {
                "$defs": {
                    "Value": {
                        "type": "object",
                        "properties": {"trace": {"type": "string", "x-mcp-header": "Trace"}}
                    }
                },
                "type": "object",
                "properties": {"value": {"$ref": "#/$defs/Value"}}
            }
        });
        let zeta_definition = json!({
            "name": "remote.zeta",
            "inputSchema": {"type": "object", "properties": {}}
        });
        let mut server = spawn_server(vec![
            Response::json(json!({"jsonrpc":"2.0","id":"1","result":{"tools":[invalid_definition, duplicate_headers, ref_path_header, alpha_definition.clone()]}})),
            Response::json(json!({"jsonrpc":"2.0","id":"1","result":{"tools":[zeta_definition]}})),
            Response::json(json!({"jsonrpc":"2.0","id":"2","result":{"content":[{"type":"text","text":"done"}],"structuredContent":{"ok":true}}})),
        ])
        .await;
        let directory = TemporaryDirectory::new("mcp-modern");
        let credentials = RecordingCredentials::with_values([
            ("CYBER_AGENT_MCP_ALPHA_API_KEY", "alpha-secret"),
            ("CYBER_AGENT_MCP_ZETA_API_KEY", "zeta-secret"),
        ]);
        let servers = format!(
            r#"[mcp_servers.zeta]
url = "{}"
protocol_version = "2026-07-28"
credential_ref = "CYBER_AGENT_MCP_ZETA_API_KEY"

[mcp_servers.alpha]
url = "{}"
protocol_version = "2026-07-28"
credential_ref = "CYBER_AGENT_MCP_ALPHA_API_KEY"
"#,
            server.url, server.url
        );
        let composition = load_composition(&directory, &servers, Arc::clone(&credentials));
        assert!(
            credentials.reads().is_empty(),
            "load and doctor remain offline"
        );

        let tools = composition
            .discover_chat_tools(directory.path())
            .await
            .expect("modern catalog is discovered at chat startup");
        assert_eq!(
            credentials.reads(),
            [
                "CYBER_AGENT_MCP_ALPHA_API_KEY",
                "CYBER_AGENT_MCP_ZETA_API_KEY"
            ]
        );
        let name = ToolName::new(format!(
            "mcp__alpha__{}",
            &blake3::hash(b"remote.alpha").to_hex()[..20]
        ));
        let spec = tools
            .catalog()
            .into_iter()
            .find(|spec| spec.name == name)
            .expect("alpha MCP tool is registered");
        assert!(
            !tools.catalog().iter().any(|candidate| {
                candidate.description.contains("remote.invalid-header")
                    || candidate.description.contains("remote.duplicate-headers")
                    || candidate.description.contains("remote.ref-path-header")
            }),
            "each invalid header annotation excludes only its remote tool"
        );
        assert_eq!(spec.exposure, ToolExposure::Deferred);
        assert_eq!(spec.effect, ToolEffect::ExternalMutation);
        assert_eq!(spec.input_schema, alpha_definition["inputSchema"]);
        assert_eq!(
            spec.input_schema["properties"]["execution"]["$ref"],
            "#/$defs/Execution"
        );

        let arguments = json!({
            "execution": {"token": "remote-data"},
            "trace": "=?base64?already?="
        });
        let plan = tools
            .plan(
                ExecutionId::new("mcp-modern-execution"),
                ToolCall::new(ToolCallId::new("mcp-modern-call"), name, arguments.clone()),
            )
            .expect("remote object arguments are planned without local envelope parsing");
        assert_eq!(plan.call.arguments, arguments);
        assert_eq!(plan.rerun_reason, None);
        let outcome = tools
            .execute(plan)
            .await
            .expect("complete remote MCP call has a durable outcome");
        assert_eq!(outcome.result.status, ToolResultStatus::Succeeded);
        assert_eq!(
            outcome.result.output["mcp_result"]["structuredContent"]["ok"],
            true
        );

        let requests = [
            server.requests.recv().await.expect("alpha list request"),
            server.requests.recv().await.expect("zeta list request"),
            server.requests.recv().await.expect("alpha call request"),
        ];
        server.task.await.expect("MCP loopback completes");
        assert_eq!(requests[0].headers["authorization"], "Bearer alpha-secret");
        assert_eq!(requests[1].headers["authorization"], "Bearer zeta-secret");
        assert_eq!(requests[2].headers["authorization"], "Bearer alpha-secret");
        assert_eq!(requests[0].body["method"], "tools/list");
        assert_eq!(requests[1].body["method"], "tools/list");
        assert_eq!(requests[2].body["method"], "tools/call");
        assert_eq!(requests[2].body["params"]["name"], "remote.alpha");
        assert_eq!(requests[2].body["params"]["arguments"], arguments);
        assert_eq!(
            requests[2].headers["mcp-param-trace"],
            format!(
                "=?base64?{}?=",
                base64::engine::general_purpose::STANDARD.encode(b"=?base64?already?=")
            )
        );
        assert_eq!(requests[2].headers["mcp-method"], "tools/call");
        assert_eq!(requests[2].headers["mcp-name"], "remote.alpha");
        assert_eq!(
            requests[2].body["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"],
            "2026-07-28"
        );
    }

    #[tokio::test]
    async fn non_static_header_annotations_are_excluded_without_reading_schema_data() {
        let annotated_property = || json!({"type": "string", "x-mcp-header": "Trace"});
        let invalid_definition = |suffix: &str, input_schema: Value| json!({"name": format!("remote.{suffix}-header"), "inputSchema": input_schema});
        let invalid_definitions = vec![
            invalid_definition(
                "all-of",
                json!({"type": "object", "allOf": [{"properties": {"trace": annotated_property()}}]}),
            ),
            invalid_definition(
                "any-of",
                json!({"type": "object", "anyOf": [{"properties": {"trace": annotated_property()}}]}),
            ),
            invalid_definition(
                "one-of",
                json!({"type": "object", "oneOf": [{"properties": {"trace": annotated_property()}}]}),
            ),
            invalid_definition(
                "not",
                json!({"type": "object", "not": {"properties": {"trace": annotated_property()}}}),
            ),
            invalid_definition(
                "if",
                json!({"type": "object", "if": {"properties": {"trace": annotated_property()}}}),
            ),
            invalid_definition(
                "then",
                json!({"type": "object", "then": {"properties": {"trace": annotated_property()}}}),
            ),
            invalid_definition(
                "else",
                json!({"type": "object", "else": {"properties": {"trace": annotated_property()}}}),
            ),
            invalid_definition(
                "items",
                json!({"type": "object", "properties": {"values": {"type": "array", "items": annotated_property()}}}),
            ),
            invalid_definition(
                "prefix-items",
                json!({"type": "object", "properties": {"values": {"type": "array", "prefixItems": [annotated_property()]}}}),
            ),
            invalid_definition(
                "contains",
                json!({"type": "object", "properties": {"values": {"type": "array", "contains": annotated_property()}}}),
            ),
            invalid_definition(
                "unevaluated-items",
                json!({"type": "object", "properties": {"values": {"type": "array", "unevaluatedItems": annotated_property()}}}),
            ),
            invalid_definition(
                "additional-properties",
                json!({"type": "object", "additionalProperties": annotated_property()}),
            ),
            invalid_definition(
                "unevaluated-properties",
                json!({"type": "object", "unevaluatedProperties": annotated_property()}),
            ),
            invalid_definition(
                "property-names",
                json!({"type": "object", "propertyNames": annotated_property()}),
            ),
            invalid_definition(
                "pattern-properties",
                json!({"type": "object", "patternProperties": {"^trace$": annotated_property()}}),
            ),
            invalid_definition(
                "dependent-schemas",
                json!({"type": "object", "dependentSchemas": {"flag": {"properties": {"trace": annotated_property()}}}}),
            ),
            invalid_definition(
                "content-schema",
                json!({"type": "object", "contentSchema": {"properties": {"trace": annotated_property()}}}),
            ),
            invalid_definition(
                "definitions",
                json!({"type": "object", "definitions": {"Value": {"properties": {"trace": annotated_property()}}}}),
            ),
            invalid_definition(
                "defs",
                json!({"type": "object", "$defs": {"Value": {"properties": {"trace": annotated_property()}}}}),
            ),
        ];
        let invalid_names = invalid_definitions
            .iter()
            .map(|definition| {
                definition["name"]
                    .as_str()
                    .expect("fixture invalid tool names are strings")
                    .to_owned()
            })
            .collect::<Vec<_>>();
        let valid_neighbor = json!({
            "name": "remote.valid-neighbor",
            "inputSchema": {
                "$defs": {"Constraint": {"type": "object"}},
                "$ref": "#/$defs/Constraint",
                "type": "object",
                "allOf": [{"type": "object", "properties": {"limit": {"type": "integer"}}}],
                "properties": {
                    "trace": {"type": "string", "x-mcp-header": "Trace"},
                    "values": {"type": "array", "items": {"type": "string"}},
                    "always": true,
                    "never": false
                }
            }
        });
        let data_key = json!({
            "name": "remote.schema-data-key",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "literal": {
                        "const": {"x-mcp-header": "const-data"},
                        "default": {"x-mcp-header": "default-data"},
                        "examples": [{"x-mcp-header": "example-data"}]
                    }
                }
            }
        });
        let mut definitions = invalid_definitions;
        definitions.push(valid_neighbor);
        definitions.push(data_key);
        let mut server = spawn_server(vec![
            Response::json(json!({
                "jsonrpc":"2.0", "id":"1",
                "result":{"tools": definitions}
            })),
            Response::json(json!({"jsonrpc":"2.0","id":"2","result":{"content":[]}})),
            Response::json(json!({"jsonrpc":"2.0","id":"3","result":{"content":[]}})),
        ])
        .await;
        let directory = TemporaryDirectory::new("mcp-non-static-header");
        let servers = format!(
            r#"[mcp_servers.remote]
url = "{}"
protocol_version = "2026-07-28"
"#,
            server.url
        );
        let tools = load_composition(&directory, &servers, RecordingCredentials::with_values([]))
            .discover_chat_tools(directory.path())
            .await
            .expect("invalid annotations exclude only their tools");
        for invalid_name in &invalid_names {
            assert!(
                !tools
                    .catalog()
                    .iter()
                    .any(|spec| spec.description.contains(invalid_name)),
                "annotation under a non-static schema keyword must exclude {invalid_name}"
            );
        }
        let neighbor = tools
            .catalog()
            .into_iter()
            .find(|spec| spec.description.contains("remote.valid-neighbor"))
            .expect("statically reachable neighbor remains available")
            .name;
        let data_key = tools
            .catalog()
            .into_iter()
            .find(|spec| spec.description.contains("remote.schema-data-key"))
            .expect("schema data named x-mcp-header is not an annotation")
            .name;

        let neighbor_arguments = json!({"trace": "neighbor"});
        tools
            .execute(
                tools
                    .plan(
                        ExecutionId::new("non-static-neighbor"),
                        ToolCall::new(
                            ToolCallId::new("non-static-neighbor"),
                            neighbor,
                            neighbor_arguments.clone(),
                        ),
                    )
                    .expect("valid neighbor plans"),
            )
            .await
            .expect("valid neighbor completes");
        let data_arguments = json!({"literal": {"x-mcp-header": "const-data"}});
        tools
            .execute(
                tools
                    .plan(
                        ExecutionId::new("schema-data-key"),
                        ToolCall::new(
                            ToolCallId::new("schema-data-key"),
                            data_key,
                            data_arguments.clone(),
                        ),
                    )
                    .expect("schema-data key plans"),
            )
            .await
            .expect("schema-data key completes");

        let list = server.requests.recv().await.expect("list request");
        let neighbor_call = server.requests.recv().await.expect("neighbor call");
        let data_call = server.requests.recv().await.expect("data-key call");
        server.task.await.expect("MCP loopback completes");
        assert_eq!(list.body["method"], "tools/list");
        assert_eq!(
            neighbor_call.body["params"]["arguments"],
            neighbor_arguments
        );
        assert_eq!(neighbor_call.headers["mcp-param-trace"], "neighbor");
        assert_eq!(data_call.body["params"]["arguments"], data_arguments);
        assert!(
            !data_call.headers.contains_key("mcp-param-const-data")
                && !data_call.headers.contains_key("mcp-param-default-data")
                && !data_call.headers.contains_key("mcp-param-example-data"),
            "const/default/examples values never become local header annotations"
        );
    }

    #[tokio::test]
    async fn legacy_initializes_freezes_a_session_and_preserves_complete_result() {
        let definition = json!({
            "name": "legacy.tool",
            "inputSchema": {"type": "object", "properties": {"value": {"type": "string"}}}
        });
        let mut server = spawn_server(vec![
            Response::json(json!({
                "jsonrpc":"2.0", "id":"1",
                "result":{"protocolVersion":"2025-11-25","capabilities":{"tools":{}}}
            }))
            .with_header("Mcp-Session-Id", "fixture-session"),
            Response::accepted(),
            Response::json(json!({"jsonrpc":"2.0","id":"2","result":{"tools":[definition]}})),
            Response::json(json!({"jsonrpc":"2.0","id":"3","result":{"content":[]}})),
        ])
        .await;
        let directory = TemporaryDirectory::new("mcp-legacy");
        let servers = format!(
            r#"[mcp_servers.legacy]
url = "{}"
protocol_version = "2025-11-25"
timeout_ms = 1000
"#,
            server.url
        );
        let credentials = RecordingCredentials::with_values([]);
        let composition = load_composition(&directory, &servers, Arc::clone(&credentials));
        let tools = composition
            .discover_chat_tools(directory.path())
            .await
            .expect("legacy initialization and list succeed");
        let name = tools
            .catalog()
            .into_iter()
            .find(|spec| spec.description.contains("legacy.tool"))
            .expect("legacy tool is frozen into the catalog")
            .name;
        let outcome = tools
            .execute(
                tools
                    .plan(
                        ExecutionId::new("legacy-execution"),
                        ToolCall::new(
                            ToolCallId::new("legacy-call"),
                            name,
                            json!({"value":"present"}),
                        ),
                    )
                    .expect("legacy call plans"),
            )
            .await
            .expect("legacy complete result is known");
        assert_eq!(outcome.result.status, ToolResultStatus::Succeeded);

        let requests = [
            server.requests.recv().await.expect("initialize"),
            server
                .requests
                .recv()
                .await
                .expect("initialized notification"),
            server.requests.recv().await.expect("tools list"),
            server.requests.recv().await.expect("tools call"),
        ];
        server.task.await.expect("legacy loopback completes");
        assert_eq!(requests[0].body["method"], "initialize");
        assert_eq!(requests[1].body["method"], "notifications/initialized");
        assert_eq!(requests[2].headers["mcp-session-id"], "fixture-session");
        assert_eq!(requests[3].headers["mcp-session-id"], "fixture-session");
        assert_eq!(requests[3].body["params"]["arguments"]["value"], "present");
    }

    #[tokio::test]
    async fn incomplete_remote_result_is_a_transient_unknown_without_a_receipt() {
        let definition = json!({
            "name": "remote.incomplete",
            "inputSchema": {"type": "object", "properties": {}}
        });
        let mut server = spawn_server(vec![
            Response::json(json!({"jsonrpc":"2.0","id":"1","result":{"tools":[definition]}})),
            Response::json(json!({"jsonrpc":"2.0","id":"2","result":{"resultType":"input_required","content":[]}})),
        ])
        .await;
        let directory = TemporaryDirectory::new("mcp-incomplete");
        let servers = format!(
            r#"[mcp_servers.remote]
url = "{}"
protocol_version = "2026-07-28"
"#,
            server.url
        );
        let credentials = RecordingCredentials::with_values([]);
        let composition = load_composition(&directory, &servers, credentials);
        let tools = composition
            .discover_chat_tools(directory.path())
            .await
            .expect("catalog is known before the call begins");
        let name = tools
            .catalog()
            .into_iter()
            .find(|spec| spec.description.contains("remote.incomplete"))
            .expect("incomplete fixture tool exists")
            .name;
        let error = tools
            .execute(
                tools
                    .plan(
                        ExecutionId::new("unknown-execution"),
                        ToolCall::new(ToolCallId::new("unknown-call"), name, json!({})),
                    )
                    .expect("object arguments plan"),
            )
            .await
            .expect_err("input_required does not establish a remote completion");
        assert!(matches!(
            error,
            ToolExecutionError::OutcomeUnknown { ref safe_reason }
                if safe_reason.contains("incomplete")
        ));
        let _ = server.requests.recv().await.expect("list request");
        let _ = server.requests.recv().await.expect("call request");
        server.task.await.expect("incomplete loopback completes");
    }

    #[tokio::test]
    async fn mcp_error_results_are_known_failures_and_sse_complete_results_succeed() {
        let definition = json!({
            "name": "remote.result",
            "inputSchema": {"type": "object", "properties": {}}
        });
        let mut server = spawn_server(vec![
            Response::json(json!({"jsonrpc":"2.0","id":"1","result":{"tools":[definition]}})),
            Response::json(json!({"jsonrpc":"2.0","id":"2","result":{"isError":true,"content":[{"type":"text","text":"known tool error"}]}})),
            Response::json(json!({"jsonrpc":"2.0","id":"3","error":{"code":-32001,"message":"known JSON-RPC error"}})),
            Response::sse(json!({"jsonrpc":"2.0","id":"4","result":{"content":[{"type":"text","text":"SSE complete"}]}})),
        ])
        .await;
        let directory = TemporaryDirectory::new("mcp-known-results");
        let servers = format!(
            r#"[mcp_servers.remote]
url = "{}"
protocol_version = "2026-07-28"
"#,
            server.url
        );
        let composition =
            load_composition(&directory, &servers, RecordingCredentials::with_values([]));
        let tools = composition
            .discover_chat_tools(directory.path())
            .await
            .expect("catalog succeeds");
        let name = tools
            .catalog()
            .into_iter()
            .find(|spec| spec.description.contains("remote.result"))
            .expect("result fixture tool exists")
            .name;
        for (call_id, expected_reason) in [
            ("remote-error", "mcp_tool_error"),
            ("rpc-error", "mcp_jsonrpc_error"),
        ] {
            let outcome = tools
                .execute(
                    tools
                        .plan(
                            ExecutionId::new(format!("execution-{call_id}")),
                            ToolCall::new(ToolCallId::new(call_id), name.clone(), json!({})),
                        )
                        .expect("object arguments plan"),
                )
                .await
                .expect("known remote error has a real receipt and paired result");
            assert_eq!(outcome.result.status, ToolResultStatus::Failed);
            assert_eq!(
                outcome.receipt.exit_reason.as_deref(),
                Some(expected_reason)
            );
            assert!(outcome.result.output["mcp_result"].is_object());
        }
        let sse = tools
            .execute(
                tools
                    .plan(
                        ExecutionId::new("execution-sse"),
                        ToolCall::new(ToolCallId::new("sse"), name, json!({})),
                    )
                    .expect("SSE object arguments plan"),
            )
            .await
            .expect("SSE final JSON-RPC result completes the call");
        assert_eq!(sse.result.status, ToolResultStatus::Succeeded);
        assert_eq!(
            sse.result.output["mcp_result"]["content"][0]["text"],
            "SSE complete"
        );
        for _ in 0..4 {
            let _ = server.requests.recv().await.expect("loopback request");
        }
        server.task.await.expect("known-result loopback completes");
    }

    #[tokio::test]
    async fn wrong_jsonrpc_id_after_dispatch_is_unknown() {
        let definition = json!({
            "name": "remote.bad-id",
            "inputSchema": {"type": "object", "properties": {}}
        });
        let mut server = spawn_server(vec![
            Response::json(json!({"jsonrpc":"2.0","id":"1","result":{"tools":[definition]}})),
            Response::json(json!({"jsonrpc":"2.0","id":"not-the-call","result":{"content":[]}})),
        ])
        .await;
        let directory = TemporaryDirectory::new("mcp-bad-id");
        let servers = format!(
            r#"[mcp_servers.remote]
url = "{}"
protocol_version = "2026-07-28"
"#,
            server.url
        );
        let composition =
            load_composition(&directory, &servers, RecordingCredentials::with_values([]));
        let tools = composition
            .discover_chat_tools(directory.path())
            .await
            .expect("catalog succeeds");
        let name = tools
            .catalog()
            .into_iter()
            .find(|spec| spec.description.contains("remote.bad-id"))
            .expect("bad-id fixture tool exists")
            .name;
        assert!(matches!(
            tools
                .execute(
                    tools
                        .plan(
                            ExecutionId::new("bad-id-execution"),
                            ToolCall::new(ToolCallId::new("bad-id-call"), name, json!({})),
                        )
                        .expect("object arguments plan"),
                )
                .await,
            Err(ToolExecutionError::OutcomeUnknown { .. })
        ));
        let _ = server.requests.recv().await.expect("list request");
        let _ = server.requests.recv().await.expect("call request");
        server.task.await.expect("bad-id loopback completes");
    }

    #[tokio::test]
    async fn annotated_integer_headers_accept_integral_number_spelling_without_rewriting_arguments()
    {
        let definition = json!({
            "name": "remote.integer-header",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "attempt": {"type": "integer", "x-mcp-header": "Attempt"}
                }
            }
        });
        let mut server = spawn_server(vec![
            Response::json(json!({"jsonrpc":"2.0","id":"1","result":{"tools":[definition]}})),
            Response::json(json!({"jsonrpc":"2.0","id":"2","result":{"content":[]}})),
        ])
        .await;
        let directory = TemporaryDirectory::new("mcp-integer-header");
        let servers = format!(
            r#"[mcp_servers.remote]
url = "{}"
protocol_version = "2026-07-28"
"#,
            server.url
        );
        let composition =
            load_composition(&directory, &servers, RecordingCredentials::with_values([]));
        let tools = composition
            .discover_chat_tools(directory.path())
            .await
            .expect("catalog succeeds");
        let name = tools
            .catalog()
            .into_iter()
            .find(|spec| spec.description.contains("remote.integer-header"))
            .expect("integer-header fixture tool exists")
            .name;
        let integral = json!({"attempt": 1e2});
        let outcome = tools
            .execute(
                tools
                    .plan(
                        ExecutionId::new("integer-header-execution"),
                        ToolCall::new(
                            ToolCallId::new("integer-header-call"),
                            name.clone(),
                            integral.clone(),
                        ),
                    )
                    .expect("integral number spelling is valid raw remote input"),
            )
            .await
            .expect("remote completion is known");
        assert_eq!(outcome.result.status, ToolResultStatus::Succeeded);
        let fractional = tools
            .execute(
                tools
                    .plan(
                        ExecutionId::new("fractional-header-execution"),
                        ToolCall::new(
                            ToolCallId::new("fractional-header-call"),
                            name,
                            json!({"attempt": 1.5}),
                        ),
                    )
                    .expect("remote schema remains server-owned"),
            )
            .await
            .expect("pre-execution header failure remains a known failure");
        assert_eq!(fractional.result.status, ToolResultStatus::Failed);
        assert_eq!(
            fractional.receipt.exit_reason.as_deref(),
            Some("mcp_pre_execution")
        );

        let _ = server.requests.recv().await.expect("list request");
        let call = server.requests.recv().await.expect("integral call request");
        assert_eq!(call.headers["mcp-param-attempt"], "100");
        assert_eq!(call.body["params"]["arguments"], integral);
        assert!(
            server.requests.try_recv().is_err(),
            "fractional value was not dispatched"
        );
        server
            .task
            .await
            .expect("integer-header loopback completes");
    }

    #[tokio::test]
    async fn discovery_paginates_in_order_and_rejects_bad_cursors_and_catalog_limits() {
        let first = json!({
            "name": "remote.first",
            "inputSchema": {"type": "object", "properties": {}}
        });
        let second = json!({
            "name": "remote.second",
            "inputSchema": {"type": "object", "properties": {}}
        });
        let mut paged = spawn_server(vec![
            Response::json(json!({
                "jsonrpc":"2.0", "id":"1",
                "result":{"tools":[first], "nextCursor":"second-page"}
            })),
            Response::json(json!({
                "jsonrpc":"2.0", "id":"2", "result":{"tools":[second]}
            })),
        ])
        .await;
        let directory = TemporaryDirectory::new("mcp-pagination");
        let servers = format!(
            r#"[mcp_servers.remote]
url = "{}"
protocol_version = "2026-07-28"
"#,
            paged.url
        );
        let tools = load_composition(&directory, &servers, RecordingCredentials::with_values([]))
            .discover_chat_tools(directory.path())
            .await
            .expect("both pages form one frozen catalog");
        let names = tools
            .catalog()
            .into_iter()
            .filter(|spec| spec.description.contains("remote."))
            .map(|spec| spec.description)
            .collect::<Vec<_>>();
        assert_eq!(names.len(), 2);
        let first_request = paged.requests.recv().await.expect("first list page");
        let second_request = paged.requests.recv().await.expect("second list page");
        assert!(
            first_request.body["params"]
                .as_object()
                .is_some_and(|params| params.is_empty())
        );
        assert_eq!(second_request.body["params"]["cursor"], "second-page");
        paged.task.await.expect("pagination loopback completes");

        let cursor_cases = vec![
            (
                "invalid-cursor",
                vec![Response::json(json!({
                    "jsonrpc":"2.0", "id":"1", "result":{"tools":[], "nextCursor": 7}
                }))],
                "invalid nextCursor",
            ),
            (
                "repeated-cursor",
                vec![
                    Response::json(json!({
                        "jsonrpc":"2.0", "id":"1", "result":{"tools":[], "nextCursor":"again"}
                    })),
                    Response::json(json!({
                        "jsonrpc":"2.0", "id":"2", "result":{"tools":[], "nextCursor":"again"}
                    })),
                ],
                "repeated a tools/list cursor",
            ),
        ];
        for (case, responses, expected) in cursor_cases {
            let server = spawn_server(responses).await;
            let directory_name = format!("mcp-{case}");
            let directory = TemporaryDirectory::new(&directory_name);
            let servers = format!(
                r#"[mcp_servers.remote]
url = "{}"
protocol_version = "2026-07-28"
"#,
                server.url
            );
            let error =
                match load_composition(&directory, &servers, RecordingCredentials::with_values([]))
                    .discover_chat_tools(directory.path())
                    .await
                {
                    Ok(_) => panic!("invalid discovery cursor rejects the whole catalog"),
                    Err(error) => error,
                };
            assert!(error.to_string().contains(expected), "{case}: {error}");
            server.task.await.expect("cursor loopback completes");
        }

        let page_responses = (0..64)
            .map(|page| {
                Response::json(json!({
                    "jsonrpc":"2.0",
                    "id": (page + 1).to_string(),
                    "result":{"tools":[], "nextCursor": format!("page-{page}")}
                }))
            })
            .collect();
        let page_limited = spawn_server(page_responses).await;
        let directory = TemporaryDirectory::new("mcp-page-limit");
        let servers = format!(
            r#"[mcp_servers.remote]
url = "{}"
protocol_version = "2026-07-28"
"#,
            page_limited.url
        );
        let error =
            match load_composition(&directory, &servers, RecordingCredentials::with_values([]))
                .discover_chat_tools(directory.path())
                .await
            {
                Ok(_) => panic!("a sixty-fifth page is forbidden"),
                Err(error) => error,
            };
        assert!(error.to_string().contains("exceeded 64 tools/list pages"));
        page_limited
            .task
            .await
            .expect("page-limit loopback completes");

        let too_many_tools = (0..1025)
            .map(|index| {
                json!({
                    "name": format!("remote.limit-{index}"),
                    "inputSchema": {"type": "object", "properties": {}}
                })
            })
            .collect::<Vec<_>>();
        let tool_limited = spawn_server(vec![Response::json(json!({
            "jsonrpc":"2.0", "id":"1", "result":{"tools":too_many_tools}
        }))])
        .await;
        let directory = TemporaryDirectory::new("mcp-tool-limit");
        let servers = format!(
            r#"[mcp_servers.remote]
url = "{}"
protocol_version = "2026-07-28"
"#,
            tool_limited.url
        );
        let error =
            match load_composition(&directory, &servers, RecordingCredentials::with_values([]))
                .discover_chat_tools(directory.path())
                .await
            {
                Ok(_) => panic!("more than 1024 tools reject the whole catalog"),
                Err(error) => error,
            };
        assert!(error.to_string().contains("exceeded 1024 tools"));
        tool_limited
            .task
            .await
            .expect("tool-limit loopback completes");
    }

    #[tokio::test]
    async fn unicode_mcp_name_is_base64_encoded_while_the_remote_name_stays_exact() {
        let remote_name = "远程.工具";
        let definition = json!({
            "name": remote_name,
            "inputSchema": {"type": "object", "properties": {}}
        });
        let mut server = spawn_server(vec![
            Response::json(json!({"jsonrpc":"2.0","id":"1","result":{"tools":[definition]}})),
            Response::json(json!({"jsonrpc":"2.0","id":"2","result":{"content":[]}})),
        ])
        .await;
        let directory = TemporaryDirectory::new("mcp-unicode-name");
        let servers = format!(
            r#"[mcp_servers.remote]
url = "{}"
protocol_version = "2026-07-28"
"#,
            server.url
        );
        let tools = load_composition(&directory, &servers, RecordingCredentials::with_values([]))
            .discover_chat_tools(directory.path())
            .await
            .expect("unicode-named remote tool is catalogued");
        let name = tools
            .catalog()
            .into_iter()
            .find(|spec| spec.description.contains(remote_name))
            .expect("unicode tool is registered")
            .name;
        tools
            .execute(
                tools
                    .plan(
                        ExecutionId::new("unicode-name-execution"),
                        ToolCall::new(ToolCallId::new("unicode-name-call"), name, json!({})),
                    )
                    .expect("unicode tool plans"),
            )
            .await
            .expect("complete unicode-named call succeeds");
        let _ = server.requests.recv().await.expect("list request");
        let call = server.requests.recv().await.expect("unicode call request");
        assert_eq!(call.body["params"]["name"], remote_name);
        assert_eq!(
            call.headers["mcp-name"],
            format!(
                "=?base64?{}?=",
                base64::engine::general_purpose::STANDARD.encode(remote_name.as_bytes())
            )
        );
        server.task.await.expect("unicode-name loopback completes");
    }

    #[tokio::test]
    async fn legacy_sse_server_requests_receive_bounded_post_acknowledgements() {
        let definition = json!({
            "name": "legacy.server-request",
            "inputSchema": {"type": "object", "properties": {}}
        });
        for (case, method, expected_response) in [
            ("ping", "ping", json!({"result": {}})),
            (
                "unknown",
                "vendor/unknown",
                json!({"error": {"code": -32601, "message": "Method not found"}}),
            ),
        ] {
            let mut server = spawn_server(vec![
                Response::json(json!({
                    "jsonrpc":"2.0", "id":"1",
                    "result":{"protocolVersion":"2025-11-25","capabilities":{"tools":{}}}
                }))
                .with_header("Mcp-Session-Id", "server-request-session"),
                Response::accepted(),
                Response::json(json!({"jsonrpc":"2.0","id":"2","result":{"tools":[definition]}})),
                Response::sse_messages([
                    json!({"jsonrpc":"2.0", "id":"server-request", "method":method, "params":{}}),
                    json!({"jsonrpc":"2.0", "id":"3", "result":{"content":[]}}),
                ]),
                Response::accepted(),
            ])
            .await;
            let directory_name = format!("mcp-legacy-server-{case}");
            let directory = TemporaryDirectory::new(&directory_name);
            let servers = format!(
                r#"[mcp_servers.legacy]
url = "{}"
protocol_version = "2025-11-25"
"#,
                server.url
            );
            let tools =
                load_composition(&directory, &servers, RecordingCredentials::with_values([]))
                    .discover_chat_tools(directory.path())
                    .await
                    .expect("legacy catalog succeeds");
            let name = tools
                .catalog()
                .into_iter()
                .find(|spec| spec.description.contains("legacy.server-request"))
                .expect("legacy server-request tool exists")
                .name;
            tools
                .execute(
                    tools
                        .plan(
                            ExecutionId::new(format!("legacy-server-{case}-execution")),
                            ToolCall::new(
                                ToolCallId::new(format!("legacy-server-{case}-call")),
                                name,
                                json!({}),
                            ),
                        )
                        .expect("legacy server-request call plans"),
                )
                .await
                .expect("the final result remains a known completion");
            let requests = [
                server.requests.recv().await.expect("initialize"),
                server.requests.recv().await.expect("initialized"),
                server.requests.recv().await.expect("list"),
                server.requests.recv().await.expect("call"),
                server
                    .requests
                    .recv()
                    .await
                    .expect("server-request response"),
            ];
            server
                .task
                .await
                .expect("server-request loopback completes");
            assert_eq!(requests[4].method, "POST");
            assert_eq!(requests[4].body["id"], "server-request");
            assert_eq!(requests[4].body["jsonrpc"], "2.0");
            assert_eq!(
                requests[4].headers["mcp-session-id"],
                "server-request-session"
            );
            for (key, value) in expected_response
                .as_object()
                .expect("expected response object")
            {
                assert_eq!(requests[4].body[key], *value, "{case} response body");
            }
        }
    }

    #[tokio::test]
    async fn legacy_session_404_is_unknown_without_reinitialization_or_replay() {
        let definition = json!({
            "name": "legacy.expired",
            "inputSchema": {"type": "object", "properties": {}}
        });
        let mut server = spawn_server(vec![
            Response::json(json!({
                "jsonrpc":"2.0", "id":"1",
                "result":{"protocolVersion":"2025-11-25","capabilities":{"tools":{}}}
            }))
            .with_header("Mcp-Session-Id", "expired-session"),
            Response::accepted(),
            Response::json(json!({"jsonrpc":"2.0","id":"2","result":{"tools":[definition]}})),
            Response::raw("application/json", "{}")
                .with_status(404)
                .with_header("Mcp-Session-Id", "expired-session"),
        ])
        .await;
        let directory = TemporaryDirectory::new("mcp-legacy-session-404");
        let servers = format!(
            r#"[mcp_servers.legacy]
url = "{}"
protocol_version = "2025-11-25"
"#,
            server.url
        );
        let tools = load_composition(&directory, &servers, RecordingCredentials::with_values([]))
            .discover_chat_tools(directory.path())
            .await
            .expect("legacy catalog succeeds before dispatch");
        let name = tools
            .catalog()
            .into_iter()
            .find(|spec| spec.description.contains("legacy.expired"))
            .expect("expired fixture tool exists")
            .name;
        assert!(matches!(
            tools
                .execute(
                    tools
                        .plan(
                            ExecutionId::new("legacy-expired-execution"),
                            ToolCall::new(ToolCallId::new("legacy-expired-call"), name, json!({}),),
                        )
                        .expect("expired call plans"),
                )
                .await,
            Err(ToolExecutionError::OutcomeUnknown { .. })
        ));
        let requests = [
            server.requests.recv().await.expect("initialize"),
            server.requests.recv().await.expect("initialized"),
            server.requests.recv().await.expect("list"),
            server.requests.recv().await.expect("one dispatched call"),
        ];
        server.task.await.expect("session-404 loopback completes");
        assert_eq!(
            requests
                .iter()
                .map(|request| request.body["method"].as_str().expect("method"))
                .collect::<Vec<_>>(),
            [
                "initialize",
                "notifications/initialized",
                "tools/list",
                "tools/call"
            ]
        );
        assert!(
            server.requests.recv().await.is_none(),
            "no reinitialize or replay request"
        );
    }

    #[tokio::test]
    async fn post_dispatch_malformed_eof_overflow_and_timeout_responses_are_unknown() {
        let definition = json!({
            "name": "remote.unknown-boundary",
            "inputSchema": {"type": "object", "properties": {}}
        });
        let cases = vec![
            (
                "malformed-result",
                Response::json(json!({"jsonrpc":"2.0","id":"2","result":[]})),
                1_000,
            ),
            (
                "malformed-error",
                Response::json(json!({"jsonrpc":"2.0","id":"2","error":[]})),
                1_000,
            ),
            (
                "malformed-json",
                Response::raw("application/json", "{"),
                1_000,
            ),
            (
                "truncated-json",
                Response::raw(
                    "application/json",
                    json!({"jsonrpc":"2.0","id":"2","result":{"content":[]}}).to_string(),
                )
                .with_declared_length(1_024),
                1_000,
            ),
            (
                "sse-eof",
                Response::sse_messages([json!({
                    "jsonrpc":"2.0", "method":"notifications/progress", "params":{}
                })]),
                1_000,
            ),
            (
                "oversized-json",
                Response::raw("application/json", " ".repeat(2 * 1024 * 1024 + 1))
                    .tolerate_closed_peer(),
                1_000,
            ),
            (
                "timeout",
                Response::json(json!({"jsonrpc":"2.0","id":"2","result":{"content":[]}}))
                    .delayed(Duration::from_millis(150)),
                40,
            ),
        ];
        for (case, call_response, timeout_ms) in cases {
            let mut server = spawn_server(vec![
                Response::json(json!({
                    "jsonrpc":"2.0", "id":"1", "result":{"tools":[definition.clone()]}
                })),
                call_response,
            ])
            .await;
            let directory_name = format!("mcp-post-dispatch-{case}");
            let directory = TemporaryDirectory::new(&directory_name);
            let servers = format!(
                r#"[mcp_servers.remote]
url = "{}"
protocol_version = "2026-07-28"
timeout_ms = {timeout_ms}
"#,
                server.url
            );
            let tools =
                load_composition(&directory, &servers, RecordingCredentials::with_values([]))
                    .discover_chat_tools(directory.path())
                    .await
                    .expect("the boundary fixture catalogs before dispatch");
            let name = tools
                .catalog()
                .into_iter()
                .find(|spec| spec.description.contains("remote.unknown-boundary"))
                .expect("unknown-boundary fixture tool exists")
                .name;
            assert!(
                matches!(
                    tools
                        .execute(
                            tools
                                .plan(
                                    ExecutionId::new(format!("{case}-execution")),
                                    ToolCall::new(
                                        ToolCallId::new(format!("{case}-call")),
                                        name,
                                        json!({}),
                                    ),
                                )
                                .expect("remote object arguments plan"),
                        )
                        .await,
                    Err(ToolExecutionError::OutcomeUnknown { .. })
                ),
                "{case} cannot establish a durable remote completion"
            );
            let _ = server.requests.recv().await.expect("list request");
            let _ = server.requests.recv().await.expect("call request");
            server.task.await.expect("post-dispatch loopback completes");
        }
    }

    #[tokio::test]
    async fn configured_deferred_mcp_unknown_stops_agent_loop_after_search_without_a_remote_pair() {
        let definition = json!({
            "name": "remote.agent-unknown",
            "inputSchema": {"type": "object", "properties": {}}
        });
        let mut server = spawn_server(vec![
            Response::json(json!({"jsonrpc":"2.0","id":"1","result":{"tools":[definition]}})),
            Response::json(json!({
                "jsonrpc":"2.0", "id":"2",
                "result":{"resultType":"input_required", "content":[]}
            })),
        ])
        .await;
        let directory = TemporaryDirectory::new("mcp-agent-loop-unknown");
        let servers = format!(
            r#"[mcp_servers.remote]
url = "{}"
protocol_version = "2026-07-28"
"#,
            server.url
        );
        let tools = load_composition(&directory, &servers, RecordingCredentials::with_values([]))
            .discover_chat_tools(directory.path())
            .await
            .expect("the fixed registry is discovered before AgentLoop start");
        let remote_name = tools
            .catalog()
            .into_iter()
            .find(|spec| spec.description.contains("remote.agent-unknown"))
            .expect("configured remote tool is deferred")
            .name;
        let provider = ScriptedProvider::new([
            vec![ModelEvent::ToolCall {
                call: ToolCall::new(
                    ToolCallId::new("search-remote"),
                    ToolName::new("tool_search"),
                    json!({"query": remote_name.as_str()}),
                ),
            }],
            vec![ModelEvent::ToolCall {
                call: ToolCall::new(
                    ToolCallId::new("unknown-remote"),
                    remote_name.clone(),
                    json!({}),
                ),
            }],
        ]);
        let mut agent = AgentLoop::new(&provider, &tools);
        assert!(matches!(
            agent
                .run_turn(
                    "find and call the configured remote tool",
                    AgentCancellation::new()
                )
                .await,
            Err(AgentLoopError::ToolExecutionOutcomeUnknown { .. })
        ));
        assert_eq!(
            provider.request_count.load(Ordering::SeqCst),
            2,
            "no third sample"
        );
        let requests = provider.requests();
        assert!(!contains_tool(&requests[0], &remote_name));
        assert!(contains_tool(&requests[1], &remote_name));
        let results = agent
            .history()
            .items()
            .iter()
            .filter_map(|item| match item {
                ContentItem::ToolResult { result } => Some(result),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(results.len(), 1, "only tool_search has a durable result");
        assert_eq!(results[0].call_id.as_str(), "search-remote");
        assert!(agent.history().ensure_all_tool_calls_paired().is_err());
        let _ = server.requests.recv().await.expect("catalog request");
        let _ = server
            .requests
            .recv()
            .await
            .expect("incomplete remote call");
        server.task.await.expect("AgentLoop loopback completes");
    }

    #[test]
    fn malformed_mcp_config_is_rejected_without_credential_access() {
        let directory = TemporaryDirectory::new("mcp-invalid-config");
        let credentials =
            RecordingCredentials::with_values([("CYBER_AGENT_MCP_ALPHA_API_KEY", "alpha-secret")]);
        let invalid = r#"[mcp_servers.alpha]
url = "https://example.invalid/mcp?forbidden=true"
protocol_version = "2026-07-28"
credential_ref = "CYBER_AGENT_MCP_ALPHA_API_KEY"
"#;
        let config = directory.path().join("provider.toml");
        std::fs::write(&config, provider_config(invalid)).expect("write invalid MCP config");
        assert!(matches!(
            ProviderComposition::load(config, credentials.clone()),
            Err(ProviderCompositionError::InvalidMcpServer { .. })
        ));
        assert!(credentials.reads().is_empty());
    }
}
