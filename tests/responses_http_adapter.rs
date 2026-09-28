use std::collections::BTreeMap;
use std::fs;
use std::io::Cursor;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use echidna::adapters::credentials::{CredentialSource, CredentialSourceError};
use echidna::adapters::project_event_store::Project;
use echidna::adapters::responses::{ResponsesHttpAdapter, ResponsesHttpConfig};
use echidna::adapters::tools::ToolRegistry;
use echidna::app::agent_loop::{AgentLoop, AgentTurnOutcome};
use echidna::app::cancellation::AgentCancellation;
use echidna::app::model_streaming::{
    ModelStreamCancellation, ModelStreamOutcome, ModelStreamingError, ModelStreamingService,
};
use echidna::domain::artifacts::ArtifactId;
use echidna::domain::events::StreamId;
use echidna::domain::model::{
    CapabilityMappingVersion, ContentItem, MessageRole, ModelCapabilities, ModelEvent,
    ModelInputIR, ModelRequestRequirements, ModelUsage,
};
use echidna::domain::project::ProjectId;
use echidna::domain::request_audit::{
    RequestAdapterVersion, RequestAuditBinding, RequestAuditRunContext,
};
use echidna::domain::tools::{
    ExecutionId, ToolCall, ToolCallId, ToolExecutionOutcome, ToolName, ToolResult, ToolResultStatus,
};
use echidna::ports::model::{
    ModelProvider, ModelProviderError, ModelRequest, ModelResponseCompletion, ProviderResponseId,
};
use echidna::ports::tool::ToolExecutor;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::timeout;

#[path = "support/live_provider.rs"]
mod live_provider;
mod support;

const FIXTURE_SECRET: &str = "fixture-secret";

#[path = "responses_cases/text_finalization.rs"]
mod text_finalization;
const FIXTURE_CREDENTIAL_REFERENCE: &str = "CYBER_AGENT_FIXTURE_API_KEY";

#[derive(Clone)]
enum FakeCredentialResult {
    Value(Option<String>),
    Failure(String),
    GenericOnly(String),
}

struct FakeCredentialSource {
    result: FakeCredentialResult,
    references: Mutex<Vec<String>>,
}

impl FakeCredentialSource {
    fn value(value: Option<&str>) -> Arc<Self> {
        Arc::new(Self {
            result: FakeCredentialResult::Value(value.map(str::to_owned)),
            references: Mutex::new(Vec::new()),
        })
    }

    fn failure(message: &str) -> Arc<Self> {
        Arc::new(Self {
            result: FakeCredentialResult::Failure(message.to_owned()),
            references: Mutex::new(Vec::new()),
        })
    }

    fn generic_only(value: &str) -> Arc<Self> {
        Arc::new(Self {
            result: FakeCredentialResult::GenericOnly(value.to_owned()),
            references: Mutex::new(Vec::new()),
        })
    }

    fn references(&self) -> Vec<String> {
        self.references
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

impl CredentialSource for FakeCredentialSource {
    fn read(&self, reference: &str) -> Result<Option<String>, CredentialSourceError> {
        self.references
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(reference.to_owned());

        match &self.result {
            FakeCredentialResult::Value(value) => Ok(value.clone()),
            FakeCredentialResult::Failure(message) => Err(CredentialSourceError::ReadFailed {
                message: message.clone(),
            }),
            FakeCredentialResult::GenericOnly(value) if reference == "OPENAI_API_KEY" => {
                Ok(Some(value.clone()))
            }
            FakeCredentialResult::GenericOnly(_) => Ok(None),
        }
    }
}

fn capabilities() -> ModelCapabilities {
    ModelCapabilities {
        text_input: true,
        image_input: true,
        streaming: true,
        tool_call: true,
        structured_output: true,
        incremental_input: true,
        usage_reporting: true,
        max_output_tokens: None,
        context_window: 128_000,
        capability_mapping_version: CapabilityMappingVersion::new(1),
    }
}

fn fixture_input(requirements: ModelRequestRequirements) -> ModelInputIR {
    ModelInputIR::current(
        "Keep evidence and hypotheses separate.",
        vec![
            ContentItem::user_text("Inspect the target manifest."),
            ContentItem::user_text("Report only reproducible impact."),
        ],
        requirements,
    )
}

fn full(input: ModelInputIR) -> ModelRequest {
    ModelRequest::full(input)
}

fn adapter(
    base_url: impl Into<String>,
    credential_source: Arc<dyn CredentialSource>,
) -> ResponsesHttpAdapter {
    ResponsesHttpAdapter::new(
        ResponsesHttpConfig::new(
            base_url,
            "fixture-model",
            FIXTURE_CREDENTIAL_REFERENCE,
            capabilities(),
        ),
        credential_source,
    )
}

async fn execute_tool(
    tools: &ToolRegistry,
    execution_id: &str,
    name: &str,
    arguments: Value,
) -> ToolExecutionOutcome {
    let call = ToolCall::new(
        ToolCallId::new(format!("call-{execution_id}")),
        ToolName::new(name),
        arguments,
    );
    let plan = tools.plan(ExecutionId::new(execution_id), call).unwrap();
    tools.execute(plan).await
}

async fn publish_image_fixture(project: &Project) -> (ArtifactId, Vec<u8>) {
    let tools = ToolRegistry::for_project(project.root());
    let target = execute_tool(
        &tools,
        "responses-image-target",
        "research_update",
        json!({
            "event_id": "event-responses-image-target",
            "occurred_at": "2026-08-26T00:00:00Z",
            "operation": {"kind": "record_target_snapshot", "record": {
                "id": "snapshot-responses-image", "target_id": "fixture", "kind": "source",
                "locator": "fixture/responses-image", "version": "1", "build_id": "1",
                "target_artifact_ids": ["artifact-responses-image-root"],
                "environment_fingerprint": "env", "device_fingerprint": "device",
                "captured_at": "2026-08-26T00:00:00Z"
            }}
        }),
    )
    .await;
    assert_eq!(target.result.status, ToolResultStatus::Succeeded);

    let mut png = Vec::new();
    image::DynamicImage::new_rgba8(2, 2)
        .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let path = project.root().join("responses-image.png");
    fs::write(&path, &png).unwrap();
    let image = execute_tool(
        &tools,
        "responses-image-view",
        "view_image",
        json!({
            "event_id": "event-responses-image-view",
            "occurred_at": "2026-08-26T00:00:01Z",
            "artifact_id": "artifact-responses-image",
            "source_path": path.display().to_string(),
            "target_snapshot_id": "snapshot-responses-image",
            "toolchain_fingerprint": "responses-image-fixture-v1"
        }),
    )
    .await;
    assert_eq!(image.result.status, ToolResultStatus::Succeeded);
    (
        ArtifactId::new(image.result.output["image_artifact_ref"].as_str().unwrap()),
        png,
    )
}

fn assert_stream_error(
    result: Result<echidna::ports::model::ModelEventStream, ModelProviderError>,
    expected: ModelProviderError,
) {
    match result {
        Err(error) => assert_eq!(error, expected),
        Ok(_) => panic!("the Adapter unexpectedly accepted the request"),
    }
}

#[test]
fn credential_source_uses_only_the_dedicated_reference() {
    let source = FakeCredentialSource::value(Some(FIXTURE_SECRET));
    let provider = adapter("http://127.0.0.1:1/v1", source.clone());

    match provider.stream(full(fixture_input(ModelRequestRequirements::default()))) {
        Ok(_) => {}
        Err(error) => panic!("fixture credential should be accepted: {error}"),
    }
    assert_eq!(source.references(), vec![FIXTURE_CREDENTIAL_REFERENCE]);
}

#[test]
fn credential_source_reports_missing_and_source_failure_without_network() {
    let missing = adapter("http://127.0.0.1:1/v1", FakeCredentialSource::value(None));
    assert_stream_error(
        missing.stream(full(fixture_input(ModelRequestRequirements::default()))),
        ModelProviderError::CredentialUnavailable {
            credential_ref: FIXTURE_CREDENTIAL_REFERENCE.to_owned(),
        },
    );

    let failed = adapter(
        "http://127.0.0.1:1/v1",
        FakeCredentialSource::failure("fixture source unavailable"),
    );
    assert_stream_error(
        failed.stream(full(fixture_input(ModelRequestRequirements::default()))),
        ModelProviderError::CredentialSourceFailure {
            credential_ref: FIXTURE_CREDENTIAL_REFERENCE.to_owned(),
            message: "credential source failed: fixture source unavailable".to_owned(),
        },
    );
}

#[test]
fn generic_provider_key_is_not_a_credential_fallback() {
    let source = FakeCredentialSource::generic_only("must-not-be-used");
    let provider = adapter("http://127.0.0.1:1/v1", source.clone());

    assert_stream_error(
        provider.stream(full(fixture_input(ModelRequestRequirements::default()))),
        ModelProviderError::CredentialUnavailable {
            credential_ref: FIXTURE_CREDENTIAL_REFERENCE.to_owned(),
        },
    );
    assert_eq!(source.references(), vec![FIXTURE_CREDENTIAL_REFERENCE]);
}

#[test]
fn invalid_authorization_header_is_rejected_before_network() {
    let provider = adapter(
        "http://127.0.0.1:1/v1",
        FakeCredentialSource::value(Some("invalid\nsecret")),
    );

    assert_stream_error(
        provider.stream(full(fixture_input(ModelRequestRequirements::default()))),
        ModelProviderError::CredentialInvalid {
            credential_ref: FIXTURE_CREDENTIAL_REFERENCE.to_owned(),
        },
    );
}

#[test]
fn deferred_request_features_fail_explicitly() {
    let input = ModelInputIR::current(
        "Inspect the image.",
        vec![ContentItem::ImageArtifact {
            role: MessageRole::User,
            artifact_id: ArtifactId::new("artifact-fixture"),
        }],
        ModelRequestRequirements::default(),
    );
    let provider = adapter(
        "http://127.0.0.1:1/v1",
        FakeCredentialSource::value(Some(FIXTURE_SECRET)),
    );
    match provider.stream(full(input)) {
        Err(ModelProviderError::RequestEncodingFailure { message }) => {
            assert!(message.contains("configured Artifact resolver"));
        }
        Err(error) => panic!("unexpected error: {error}"),
        Ok(_) => panic!("unresolved image input must not be silently accepted"),
    }
}

#[tokio::test]
async fn resolved_image_uses_real_responses_data_url_and_fixed_json_object_output() {
    let body = concat!(
        "data: {\"type\":\"response.completed\",\"response\":{",
        "\"id\":\"response_image_fixture\"}}\n\n"
    );
    let (base_url, request_receiver, server) = spawn_sse_server(200, body).await;
    let directory = support::TemporaryDirectory::new("responses-project-image-resolver");
    let project = Project::initialize(directory.path()).unwrap();
    let (artifact_id, expected_png) = publish_image_fixture(&project).await;
    let provider = adapter(base_url, FakeCredentialSource::value(Some(FIXTURE_SECRET)))
        .with_project_image_resolver(project);
    let input = ModelInputIR::current(
        "Inspect the screenshot.",
        vec![ContentItem::ImageArtifact {
            role: MessageRole::User,
            artifact_id,
        }],
        ModelRequestRequirements {
            structured_output: true,
            ..ModelRequestRequirements::default()
        },
    );
    let outcome = ModelStreamingService::new(&provider)
        .consume(full(input), ModelStreamCancellation::new())
        .await
        .unwrap();
    assert!(matches!(outcome, ModelStreamOutcome::Completed { .. }));
    let request = request_receiver.await.unwrap();
    server.await.unwrap();
    let body: Value = serde_json::from_str(&request.body).unwrap();
    assert_eq!(body["text"]["format"]["type"], "json_object");
    assert_eq!(body["input"][0]["content"][0]["type"], "input_image");
    let image_url = body["input"][0]["content"][0]["image_url"]
        .as_str()
        .unwrap();
    let encoded = image_url.strip_prefix("data:image/png;base64,").unwrap();
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap(),
        expected_png
    );
}

#[tokio::test]
async fn project_image_resolver_never_crosses_project_artifact_facts() {
    let first_directory = support::TemporaryDirectory::new("responses-image-project-a");
    let first_project = Project::initialize(first_directory.path()).unwrap();
    let (artifact_id, _) = publish_image_fixture(&first_project).await;
    let second_directory = support::TemporaryDirectory::new("responses-image-project-b");
    let second_project = Project::initialize(second_directory.path()).unwrap();
    let provider = adapter(
        "http://127.0.0.1:1/v1",
        FakeCredentialSource::value(Some(FIXTURE_SECRET)),
    )
    .with_project_image_resolver(second_project);
    let input = ModelInputIR::current(
        "Inspect the screenshot.",
        vec![ContentItem::ImageArtifact {
            role: MessageRole::User,
            artifact_id,
        }],
        ModelRequestRequirements::default(),
    );
    match provider.stream(full(input)) {
        Err(ModelProviderError::RequestEncodingFailure { message }) => {
            assert!(message.contains("not present in the selected project"));
        }
        Err(error) => panic!("unexpected error: {error}"),
        Ok(_) => panic!("a different project's Artifact must not be resolved"),
    }
}

#[derive(Debug)]
struct CapturedRequest {
    request_line: String,
    headers: BTreeMap<String, String>,
    body: String,
}

async fn spawn_sse_server(
    status: u16,
    body: &str,
) -> (String, oneshot::Receiver<CapturedRequest>, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let response_body = body.as_bytes().to_vec();
    let (request_sender, request_receiver) = oneshot::channel();

    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let request = read_http_request(&mut socket).await;
        let _ = request_sender.send(request);
        let reason = if status == 200 { "OK" } else { "Unauthorized" };
        let headers = format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            response_body.len()
        );
        if socket.write_all(headers.as_bytes()).await.is_err() {
            return;
        }
        for chunk in response_body.chunks(17) {
            if socket.write_all(chunk).await.is_err() {
                return;
            }
            tokio::task::yield_now().await;
        }
        let _ = socket.shutdown().await;
    });

    (format!("http://{address}/v1"), request_receiver, task)
}

async fn spawn_pending_server() -> (
    String,
    oneshot::Receiver<CapturedRequest>,
    oneshot::Sender<()>,
    JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (request_sender, request_receiver) = oneshot::channel();
    let (release_sender, release_receiver) = oneshot::channel();

    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let request = read_http_request(&mut socket).await;
        let _ = request_sender.send(request);
        let _ = release_receiver.await;
        let _ = socket.shutdown().await;
    });

    (
        format!("http://{address}/v1"),
        request_receiver,
        release_sender,
        task,
    )
}

async fn spawn_sse_sequence_server(
    bodies: Vec<String>,
) -> (String, mpsc::Receiver<CapturedRequest>, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (request_sender, request_receiver) = mpsc::channel(bodies.len());

    let task = tokio::spawn(async move {
        for body in bodies {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_http_request(&mut socket).await;
            request_sender.send(request).await.unwrap();
            let response_body = body.into_bytes();
            let headers = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                response_body.len()
            );
            socket.write_all(headers.as_bytes()).await.unwrap();
            for chunk in response_body.chunks(17) {
                socket.write_all(chunk).await.unwrap();
                tokio::task::yield_now().await;
            }
            socket.shutdown().await.unwrap();
        }
    });

    (format!("http://{address}/v1"), request_receiver, task)
}

async fn read_http_request(socket: &mut TcpStream) -> CapturedRequest {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 1024];
    let (header_end, content_length) = loop {
        let read = socket.read(&mut buffer).await.unwrap();
        assert!(read > 0, "client closed before sending a complete request");
        bytes.extend_from_slice(&buffer[..read]);
        assert!(bytes.len() <= 1024 * 1024, "fixture request is too large");

        if let Some(header_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            let header_end = header_end + 4;
            let header_text = std::str::from_utf8(&bytes[..header_end]).unwrap();
            let content_length = header_text
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            if bytes.len() >= header_end + content_length {
                break (header_end, content_length);
            }
        }
    };

    let header_text = std::str::from_utf8(&bytes[..header_end]).unwrap();
    let mut lines = header_text.lines();
    let request_line = lines.next().unwrap().to_owned();
    let headers = lines
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.to_ascii_lowercase(), value.trim().to_owned()))
        })
        .collect();
    let body = String::from_utf8(bytes[header_end..header_end + content_length].to_vec()).unwrap();

    CapturedRequest {
        request_line,
        headers,
        body,
    }
}

#[tokio::test]
async fn full_request_and_chunked_sse_decode_to_ordered_model_events() {
    let (base_url, request_receiver, server) =
        spawn_sse_server(200, include_str!("fixtures/responses/success.sse")).await;
    let provider = adapter(base_url, FakeCredentialSource::value(Some(FIXTURE_SECRET)));
    let service = ModelStreamingService::new(&provider);

    let outcome = service
        .consume(
            full(fixture_input(ModelRequestRequirements {
                usage_reporting: true,
                max_output_tokens: None,
                ..ModelRequestRequirements::default()
            })),
            ModelStreamCancellation::new(),
        )
        .await
        .unwrap();

    assert_eq!(
        outcome,
        ModelStreamOutcome::Completed {
            events: vec![
                ModelEvent::TextDelta {
                    text: "possible finding".to_owned(),
                },
                ModelEvent::ToolCall {
                    call: ToolCall::new(
                        ToolCallId::new("call_fixture"),
                        ToolName::new("inspect_manifest"),
                        json!({"path": "AndroidManifest.xml"}),
                    ),
                },
                ModelEvent::Usage {
                    usage: ModelUsage {
                        input_tokens: 21,
                        output_tokens: 34,
                    },
                },
            ],
            completion: ModelResponseCompletion::new(ProviderResponseId::new("resp_fixture")),
        }
    );

    let request = request_receiver.await.unwrap();
    server.await.unwrap();
    assert_eq!(request.request_line, "POST /v1/responses HTTP/1.1");
    assert!(matches!(
        request.headers.get("authorization").map(String::as_str),
        Some("Bearer fixture-secret")
    ));
    assert_eq!(
        request.headers.get("accept").map(String::as_str),
        Some("text/event-stream")
    );
    let actual: Value = serde_json::from_str(&request.body).unwrap();
    let expected: Value =
        serde_json::from_str(include_str!("fixtures/responses/full_request.json")).unwrap();
    assert_eq!(actual, expected);
}

#[tokio::test]
async fn reasoning_extension_tool_usage_and_full_only_completion_share_one_decoder() {
    let (base_url, request_receiver, server) = spawn_sse_server(
        200,
        include_str!("fixtures/responses/reasoning_success.sse"),
    )
    .await;
    let mut full_only_capabilities = capabilities();
    full_only_capabilities.incremental_input = false;
    let provider = ResponsesHttpAdapter::new(
        ResponsesHttpConfig::new(
            base_url,
            "fixture-reasoning-model",
            FIXTURE_CREDENTIAL_REFERENCE,
            full_only_capabilities,
        ),
        FakeCredentialSource::value(Some(FIXTURE_SECRET)),
    );

    let outcome = ModelStreamingService::new(&provider)
        .consume(
            full(fixture_input(ModelRequestRequirements {
                usage_reporting: true,
                max_output_tokens: None,
                ..ModelRequestRequirements::default()
            })),
            ModelStreamCancellation::new(),
        )
        .await
        .unwrap();

    assert_eq!(
        outcome,
        ModelStreamOutcome::Completed {
            events: vec![
                ModelEvent::ReasoningDelta {
                    text: "check the manifest first".to_owned(),
                },
                ModelEvent::ReasoningItem {
                    text: "check the manifest first".to_owned(),
                },
                ModelEvent::ToolCall {
                    call: ToolCall::new(
                        ToolCallId::new("call_reasoning_fixture"),
                        ToolName::new("inspect_manifest"),
                        json!({"path": "AndroidManifest.xml"}),
                    ),
                },
                ModelEvent::TextDelta {
                    text: "reproducible result".to_owned(),
                },
                ModelEvent::Usage {
                    usage: ModelUsage {
                        input_tokens: 13,
                        output_tokens: 21,
                    },
                },
            ],
            completion: ModelResponseCompletion::without_response_id(),
        }
    );

    let request = request_receiver.await.unwrap();
    server.await.unwrap();
    assert_eq!(request.request_line, "POST /v1/responses HTTP/1.1");
    let actual: Value = serde_json::from_str(&request.body).unwrap();
    assert_eq!(actual["model"], json!("fixture-reasoning-model"));
    assert!(actual.get("previous_response_id").is_none());
}

#[tokio::test]
async fn full_only_tool_loop_replays_reasoning_call_and_result_in_wire_order() {
    let reasoning = json!({
        "type":"reasoning",
        "status":null,
        "content":[{"type":"reasoning_text",
            "text":"Search the bounded fixture first."}],
        "encrypted_content":null
    });
    let call = json!({
        "type":"function_call",
        "call_id":"call-full-reasoning",
        "name":"search_files",
        "arguments":"{\"pattern\":\"cyber-agent\",\"paths\":[\"Cargo.toml\"]}"
    });
    let first = [
        json!({"type":"response.output_item.done","output_index":0,
            "item":reasoning}),
        json!({"type":"response.output_item.done","output_index":1,
            "item":call}),
        json!({"type":"response.completed","response":{
            "id":"response-nullable-reasoning","status":"completed",
            "output":[reasoning,call]
        }}),
    ]
    .into_iter()
    .map(|event| format!("data: {event}\n\n"))
    .collect::<String>();
    let second = concat!(
        "data: {\"type\":\"response.output_text.delta\",",
        "\"delta\":\"bounded search complete\"}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{}}\n\n",
    );
    let (base_url, mut requests, server) =
        spawn_sse_sequence_server(vec![first.to_owned(), second.to_owned()]).await;
    let mut full_only_capabilities = capabilities();
    full_only_capabilities.incremental_input = false;
    full_only_capabilities.usage_reporting = false;
    let provider = ResponsesHttpAdapter::new(
        ResponsesHttpConfig::new(
            base_url,
            "fixture-full-only-model",
            FIXTURE_CREDENTIAL_REFERENCE,
            full_only_capabilities,
        ),
        FakeCredentialSource::value(Some(FIXTURE_SECRET)),
    );
    let tools = ToolRegistry::for_project(env!("CARGO_MANIFEST_DIR"));
    let mut agent = AgentLoop::new(&provider, &tools);

    let audit_context = RequestAuditRunContext::new(
        RequestAuditBinding::new(
            ProjectId::new("project-full-only-reasoning"),
            StreamId::new("session:full-only-reasoning"),
        )
        .unwrap(),
        RequestAdapterVersion::new("fixture-responses-v1").unwrap(),
    );
    let mut audits = Vec::new();
    let outcome = agent
        .run_turn_audited(
            "inspect the package manifest",
            AgentCancellation::new(),
            &audit_context,
            &mut |attempt| {
                audits.push(attempt);
                Ok(())
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        AgentTurnOutcome::Completed {
            assistant_text,
            model_requests: 2,
            executions,
        } if assistant_text == "bounded search complete" && executions.len() == 1
    ));
    assert!(
        agent
            .history()
            .items()
            .iter()
            .all(|item| !matches!(item, ContentItem::Reasoning { .. }))
    );
    assert_eq!(audits.len(), 2);

    let _first_request = requests.recv().await.unwrap();
    let second_request = requests.recv().await.unwrap();
    server.await.unwrap();
    let second_body: Value = serde_json::from_str(&second_request.body).unwrap();
    assert!(second_body.get("previous_response_id").is_none());
    let input = second_body["input"].as_array().unwrap();
    assert_eq!(input.len(), 4);
    assert_eq!(input[0]["type"], json!("message"));
    assert_eq!(input[1]["type"], json!("reasoning"));
    assert_eq!(input[2]["type"], json!("function_call"));
    assert_eq!(input[3]["type"], json!("function_call_output"));
}

#[tokio::test]
async fn full_only_tool_loop_treats_absent_null_and_empty_reasoning_as_no_plaintext() {
    for (case, status, content) in [
        ("missing", None, None),
        ("null", Some(Value::Null), Some(Value::Null)),
        ("empty", Some(json!("completed")), Some(json!([]))),
    ] {
        let mut reasoning = json!({"type":"reasoning","id":format!("reasoning-{case}")});
        if let Some(status) = status {
            reasoning["status"] = status;
        }
        if let Some(content) = content {
            reasoning["content"] = content;
        }
        let call = json!({
            "type":"function_call",
            "call_id":format!("call-empty-reasoning-{case}"),
            "name":"search_files",
            "arguments":"{\"pattern\":\"echidna\",\"paths\":[\"Cargo.toml\"]}"
        });
        let first = [
            json!({"type":"response.output_item.done","output_index":0,
                "item":reasoning}),
            json!({"type":"response.output_item.done","output_index":1,
                "item":call}),
            json!({"type":"response.completed","response":{
                "id":format!("response-empty-reasoning-{case}"),"status":"completed",
                "output":[reasoning,call]
            }}),
        ]
        .into_iter()
        .map(|event| format!("data: {event}\n\n"))
        .collect::<String>();
        let second = format!(
            "data: {}\n\ndata: {}\n\n",
            json!({"type":"response.output_text.delta","delta":format!("complete-{case}")}),
            json!({"type":"response.completed","response":{
                "id":format!("response-empty-reasoning-final-{case}"),
                "status":"completed"
            }})
        );
        let (base_url, mut requests, server) = spawn_sse_sequence_server(vec![first, second]).await;
        let mut full_only_capabilities = capabilities();
        full_only_capabilities.incremental_input = false;
        full_only_capabilities.usage_reporting = false;
        let provider = ResponsesHttpAdapter::new(
            ResponsesHttpConfig::new(
                base_url,
                "fixture-empty-reasoning-model",
                FIXTURE_CREDENTIAL_REFERENCE,
                full_only_capabilities,
            ),
            FakeCredentialSource::value(Some(FIXTURE_SECRET)),
        );
        let tools = ToolRegistry::for_project(env!("CARGO_MANIFEST_DIR"));
        let mut agent = AgentLoop::new(&provider, &tools);

        let outcome = agent
            .run_turn("inspect the package manifest", AgentCancellation::new())
            .await
            .unwrap_or_else(|error| panic!("{case}: {error}"));
        assert!(matches!(
            outcome,
            AgentTurnOutcome::Completed {
                assistant_text,
                model_requests: 2,
                executions,
            } if assistant_text == format!("complete-{case}") && executions.len() == 1
        ));
        assert!(
            agent
                .history()
                .items()
                .iter()
                .all(|item| !matches!(item, ContentItem::Reasoning { .. })),
            "{case}: empty reasoning must not enter History"
        );
        assert_eq!(agent.history().ensure_all_tool_calls_paired(), Ok(()));

        requests.recv().await.unwrap();
        let second_request = requests.recv().await.unwrap();
        server.await.unwrap();
        let second_body: Value = serde_json::from_str(&second_request.body).unwrap();
        assert!(second_body.get("previous_response_id").is_none());
        let input = second_body["input"].as_array().unwrap();
        assert_eq!(input.len(), 3, "{case}: no reasoning item may be replayed");
        assert_eq!(input[0]["type"], json!("message"));
        assert_eq!(input[1]["type"], json!("function_call"));
        assert_eq!(input[2]["type"], json!("function_call_output"));
    }
}

#[tokio::test]
async fn nullable_reasoning_never_commits_or_executes_without_a_valid_completion() {
    let reasoning = json!({
        "type":"reasoning",
        "status":null,
        "content":[{"type":"reasoning_text","text":"Do not commit this sample."}]
    });
    let call = json!({
        "type":"function_call",
        "call_id":"call-must-not-run",
        "name":"write_file",
        "arguments":"{\"path\":\"must-not-exist.txt\",\"content\":\"executed\\n\",\"expected_hash\":null}"
    });
    let prefix = [
        json!({"type":"response.output_item.done","output_index":0,
            "item":reasoning}),
        json!({"type":"response.output_item.done","output_index":1,
            "item":call}),
    ]
    .into_iter()
    .map(|event| format!("data: {event}\n\n"))
    .collect::<String>();
    let mismatched_reasoning = json!({
        "type":"reasoning",
        "status":null,
        "content":[{"type":"reasoning_text","text":"Changed in the snapshot."}]
    });
    let terminals = [
        ("eof", String::new(), "ended before `response.completed`"),
        (
            "failed",
            format!(
                "data: {}\n\n",
                json!({"type":"response.failed","response":{
                    "error":{"code":"synthetic_failure","message":"synthetic failure"}
                }})
            ),
            "synthetic failure",
        ),
        (
            "incomplete",
            format!(
                "data: {}\n\n",
                json!({"type":"response.incomplete","response":{
                    "incomplete_details":{"reason":"synthetic_limit"}
                }})
            ),
            "was incomplete: synthetic_limit",
        ),
        (
            "snapshot-mismatch",
            format!(
                "data: {}\n\n",
                json!({"type":"response.completed","response":{
                    "id":"response-mismatch","status":"completed",
                    "output":[mismatched_reasoning,call]
                }})
            ),
            "completed output disagrees with completed non-text items",
        ),
    ];

    for (case, terminal, expected_error) in terminals {
        let body = format!("{prefix}{terminal}");
        let (base_url, request_receiver, server) = spawn_sse_server(200, &body).await;
        let credentials = FakeCredentialSource::value(Some(FIXTURE_SECRET));
        let mut full_only_capabilities = capabilities();
        full_only_capabilities.incremental_input = false;
        full_only_capabilities.usage_reporting = false;
        let provider = ResponsesHttpAdapter::new(
            ResponsesHttpConfig::new(
                base_url,
                "fixture-nullable-reasoning-model",
                FIXTURE_CREDENTIAL_REFERENCE,
                full_only_capabilities,
            ),
            credentials.clone(),
        );
        let directory =
            support::TemporaryDirectory::new(&format!("nullable-reasoning-atomicity-{case}"));
        let tools = ToolRegistry::for_project(directory.path());
        let mut agent = AgentLoop::new(&provider, &tools);

        let error = timeout(
            Duration::from_secs(3),
            agent.run_turn("reject the incomplete sample", AgentCancellation::new()),
        )
        .await
        .unwrap_or_else(|_| panic!("{case}: AgentLoop timed out instead of stopping"))
        .unwrap_err();
        assert!(
            error.to_string().contains(expected_error),
            "{case}: {error}"
        );
        assert_eq!(
            credentials.references(),
            vec![FIXTURE_CREDENTIAL_REFERENCE.to_owned()],
            "{case}: the failed sample must not trigger an automatic retry"
        );
        assert_eq!(
            agent.history().items(),
            &[ContentItem::user_text("reject the incomplete sample")],
            "{case}: model output must not commit to History"
        );
        assert!(
            !directory.path().join("must-not-exist.txt").exists(),
            "{case}: the uncommitted ToolCall must not execute"
        );
        request_receiver.await.unwrap();
        server.await.unwrap();
    }
}

#[tokio::test]
async fn tool_schema_call_and_result_encode_in_full_history_order() {
    let body = concat!(
        "data: {\"type\":\"response.completed\",\"response\":{",
        "\"id\":\"response_tool_fixture\"}}\n\n"
    );
    let (base_url, request_receiver, server) = spawn_sse_server(200, body).await;
    let provider = adapter(base_url, FakeCredentialSource::value(Some(FIXTURE_SECRET)));
    let service = ModelStreamingService::new(&provider);
    let read_file = ToolRegistry::for_project(env!("CARGO_MANIFEST_DIR"))
        .specs()
        .into_iter()
        .find(|spec| spec.name.as_str() == "read_file")
        .unwrap();
    let call = ToolCall::new(
        ToolCallId::new("call-read"),
        ToolName::new("read_file"),
        json!({"path": "Cargo.toml"}),
    );
    let result = ToolResult {
        call_id: call.call_id.clone(),
        tool_name: call.name.clone(),
        status: ToolResultStatus::Succeeded,
        output: json!({"content": "fixture"}),
    };
    let input = ModelInputIR::current(
        "Inspect local files.",
        vec![
            ContentItem::user_text("Inspect Cargo.toml."),
            ContentItem::assistant_text("I will read it."),
            ContentItem::Reasoning {
                text: "The file read is the next required step.".to_owned(),
            },
            ContentItem::ToolCall { call },
            ContentItem::ToolResult { result },
        ],
        ModelRequestRequirements::default(),
    )
    .with_tool_specs(vec![read_file]);

    assert!(matches!(
        service
            .consume(full(input), ModelStreamCancellation::new())
            .await
            .unwrap(),
        ModelStreamOutcome::Completed { .. }
    ));
    let request = request_receiver.await.unwrap();
    server.await.unwrap();
    let actual: Value = serde_json::from_str(&request.body).unwrap();
    let expected: Value =
        serde_json::from_str(include_str!("fixtures/responses/tool_request.json")).unwrap();
    assert_eq!(actual, expected);
}

#[tokio::test]
async fn shared_input_definitions_reach_wire_from_the_resolved_tool_plan_unchanged() {
    let (base_url, request_receiver, server) = spawn_sse_server(
        200,
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"shared-schema-fixture\"}}\n\n",
    )
    .await;
    let provider = adapter(base_url, FakeCredentialSource::value(Some(FIXTURE_SECRET)));
    let tools = ToolRegistry::for_project(env!("CARGO_MANIFEST_DIR"));
    let resolved = tools
        .resolve_tool_plan(&Default::default(), &Default::default())
        .unwrap();
    let specs = resolved.advertised_specs();
    let research = resolved
        .advertised_spec(&ToolName::new("research_update"))
        .unwrap();
    assert_eq!(research.input_schema["$defs"]["text"]["minLength"], 1);
    let input = fixture_input(ModelRequestRequirements::default()).with_tool_specs(specs.to_vec());
    ModelStreamingService::new(&provider)
        .consume(full(input), ModelStreamCancellation::new())
        .await
        .unwrap();
    let request = request_receiver.await.unwrap();
    server.await.unwrap();
    let actual: Value = serde_json::from_str(&request.body).unwrap();
    let wire = actual["tools"].as_array().unwrap();
    assert_eq!(wire.len(), specs.len());
    for (entry, spec) in wire.iter().zip(specs) {
        assert_eq!(entry["name"], spec.name.as_str());
        assert_eq!(entry["parameters"], spec.input_schema);
        assert!(entry.get("output_schema").is_none());
    }
}

#[tokio::test]
async fn http_rejection_is_a_typed_provider_error() {
    let (base_url, request_receiver, server) = spawn_sse_server(401, "unauthorized").await;
    let provider = adapter(base_url, FakeCredentialSource::value(Some(FIXTURE_SECRET)));
    let service = ModelStreamingService::new(&provider);

    assert_eq!(
        service
            .consume(
                full(fixture_input(ModelRequestRequirements::default())),
                ModelStreamCancellation::new(),
            )
            .await,
        Err(ModelStreamingError::Provider(
            ModelProviderError::HttpFailure { status: 401 }
        ))
    );
    let _ = request_receiver.await.unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn provider_failure_preserves_code_and_message() {
    let body = concat!(
        "data: {\"type\":\"response.failed\",\"response\":{\"error\":{",
        "\"code\":\"rate_limit_exceeded\",\"message\":\"fixture limit\"}}}\n\n"
    );
    let (base_url, request_receiver, server) = spawn_sse_server(200, body).await;
    let provider = adapter(base_url, FakeCredentialSource::value(Some(FIXTURE_SECRET)));
    let service = ModelStreamingService::new(&provider);

    assert_eq!(
        service
            .consume(
                full(fixture_input(ModelRequestRequirements::default())),
                ModelStreamCancellation::new(),
            )
            .await,
        Err(ModelStreamingError::Provider(
            ModelProviderError::ProviderFailure {
                code: Some("rate_limit_exceeded".to_owned()),
                message: "fixture limit".to_owned(),
            }
        ))
    );
    let _ = request_receiver.await.unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn incremental_request_encodes_only_delta_and_the_opaque_continuation() {
    let body = concat!(
        "data: {\"type\":\"response.completed\",\"response\":{",
        "\"id\":\"resp-after-delta\"}}\n\n"
    );
    let (base_url, request_receiver, server) = spawn_sse_server(200, body).await;
    let provider = adapter(base_url, FakeCredentialSource::value(Some(FIXTURE_SECRET)));
    let service = ModelStreamingService::new(&provider);
    let delta = ModelInputIR::current(
        "Keep evidence and hypotheses separate.",
        vec![ContentItem::user_text("Only this new input is sent.")],
        ModelRequestRequirements::default(),
    );

    let outcome = service
        .consume(
            ModelRequest::incremental(delta, ProviderResponseId::new("resp-before-delta")),
            ModelStreamCancellation::new(),
        )
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        ModelStreamOutcome::Completed { completion, .. }
            if completion.response_id().is_some_and(|id| id.as_str() == "resp-after-delta")
    ));

    let request = request_receiver.await.unwrap();
    server.await.unwrap();
    let actual: Value = serde_json::from_str(&request.body).unwrap();
    assert_eq!(actual["previous_response_id"], json!("resp-before-delta"));
    assert_eq!(actual["store"], json!(false));
    assert_eq!(
        actual["input"],
        json!([{
            "type": "message",
            "role": "user",
            "content": [{"type": "input_text", "text": "Only this new input is sent."}],
        }])
    );
}

#[tokio::test]
async fn only_exact_incremental_previous_response_rejections_are_typed_for_fallback() {
    let cases = [
        (
            "previous_response_not_found",
            true,
            ModelRequest::incremental(
                fixture_input(ModelRequestRequirements::default()),
                ProviderResponseId::new("resp-old"),
            ),
        ),
        (
            "previous_response_not_found",
            false,
            full(fixture_input(ModelRequestRequirements::default())),
        ),
        (
            "rate_limit_exceeded",
            false,
            ModelRequest::incremental(
                fixture_input(ModelRequestRequirements::default()),
                ProviderResponseId::new("resp-old"),
            ),
        ),
    ];

    for (code, is_continuation_rejection, request) in cases {
        let body = format!(
            "data: {{\"type\":\"response.failed\",\"response\":{{\"error\":{{\"code\":\"{code}\",\"message\":\"fixture\"}}}}}}\n\n"
        );
        let (base_url, request_receiver, server) = spawn_sse_server(200, &body).await;
        let provider = adapter(base_url, FakeCredentialSource::value(Some(FIXTURE_SECRET)));
        let service = ModelStreamingService::new(&provider);
        let result = service
            .consume(request, ModelStreamCancellation::new())
            .await;

        if is_continuation_rejection {
            assert_eq!(
                result,
                Err(ModelStreamingError::Provider(
                    ModelProviderError::ContinuationRejected {
                        code: "previous_response_not_found".to_owned(),
                    }
                ))
            );
        } else {
            assert!(matches!(
                result,
                Err(ModelStreamingError::Provider(
                    ModelProviderError::ProviderFailure { .. }
                ))
            ));
        }
        let _ = request_receiver.await.unwrap();
        server.await.unwrap();
    }
}

#[tokio::test]
async fn http_previous_response_rejection_is_typed_only_for_incremental_delivery() {
    for incremental in [true, false] {
        let body = r#"{"error":{"code":"previous_response_not_found","message":"fixture"}}"#;
        let (base_url, request_receiver, server) = spawn_sse_server(400, body).await;
        let provider = adapter(base_url, FakeCredentialSource::value(Some(FIXTURE_SECRET)));
        let request = if incremental {
            ModelRequest::incremental(
                fixture_input(ModelRequestRequirements::default()),
                ProviderResponseId::new("resp-old"),
            )
        } else {
            full(fixture_input(ModelRequestRequirements::default()))
        };
        let result = ModelStreamingService::new(&provider)
            .consume(request, ModelStreamCancellation::new())
            .await;

        if incremental {
            assert!(matches!(
                result,
                Err(ModelStreamingError::Provider(
                    ModelProviderError::ContinuationRejected { .. }
                ))
            ));
        } else {
            assert_eq!(
                result,
                Err(ModelStreamingError::Provider(
                    ModelProviderError::HttpFailure { status: 400 }
                ))
            );
        }
        let _ = request_receiver.await.unwrap();
        server.await.unwrap();
    }
}

#[tokio::test]
async fn missing_completed_response_identity_is_a_protocol_failure_for_incremental_capability() {
    let body = "data: {\"type\":\"response.completed\",\"response\":{}}\n\n";
    let (base_url, request_receiver, server) = spawn_sse_server(200, body).await;
    let provider = adapter(base_url, FakeCredentialSource::value(Some(FIXTURE_SECRET)));
    let result = ModelStreamingService::new(&provider)
        .consume(
            full(fixture_input(ModelRequestRequirements::default())),
            ModelStreamCancellation::new(),
        )
        .await;
    assert!(matches!(
        result,
        Err(ModelStreamingError::Provider(ModelProviderError::ProtocolFailure { message }))
            if message.contains("response identity")
    ));
    let _ = request_receiver.await.unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn loopback_rejection_fallback_sends_full_history_and_never_commits_partial_output() {
    let first = concat!(
        "data: {\"type\":\"response.output_item.done\",\"item\":{",
        "\"type\":\"function_call\",\"call_id\":\"call-search\",",
        "\"name\":\"search_files\",\"arguments\":\"{\\\"pattern\\\":\\\"Cargo\\\",\\\"paths\\\":[\\\"Cargo.toml\\\"]}\"}}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp-before-fallback\"}}\n\n",
    );
    let rejection = concat!(
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"discard partial\"}\n\n",
        "data: {\"type\":\"response.failed\",\"response\":{\"error\":{",
        "\"code\":\"previous_response_not_found\",\"message\":\"fixture\"}}}\n\n",
    );
    let fallback = concat!(
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"fallback answer\"}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp-after-fallback\"}}\n\n",
    );
    let (base_url, mut requests, server) = spawn_sse_sequence_server(vec![
        first.to_owned(),
        rejection.to_owned(),
        fallback.to_owned(),
    ])
    .await;
    let provider = adapter(base_url, FakeCredentialSource::value(Some(FIXTURE_SECRET)));
    let tools = ToolRegistry::for_project(env!("CARGO_MANIFEST_DIR"));
    let mut agent = AgentLoop::new(&provider, &tools);

    let outcome = agent
        .run_turn("inspect Cargo", AgentCancellation::new())
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        AgentTurnOutcome::Completed {
            assistant_text,
            model_requests: 3,
            executions,
        } if assistant_text == "fallback answer" && executions.len() == 1
    ));
    assert!(
        agent
            .history()
            .items()
            .iter()
            .all(|item| item != &ContentItem::assistant_text("discard partial"))
    );

    let first_request = requests.recv().await.unwrap();
    let delta_request = requests.recv().await.unwrap();
    let fallback_request = requests.recv().await.unwrap();
    server.await.unwrap();
    let first_body: Value = serde_json::from_str(&first_request.body).unwrap();
    let delta_body: Value = serde_json::from_str(&delta_request.body).unwrap();
    let fallback_body: Value = serde_json::from_str(&fallback_request.body).unwrap();
    assert_eq!(first_body["store"], json!(false));
    assert!(first_body.get("previous_response_id").is_none());
    assert_eq!(
        delta_body["previous_response_id"],
        json!("resp-before-fallback")
    );
    assert_eq!(delta_body["store"], json!(false));
    assert_eq!(delta_body["input"].as_array().unwrap().len(), 1);
    assert_eq!(
        delta_body["input"][0]["type"],
        json!("function_call_output")
    );
    assert!(fallback_body.get("previous_response_id").is_none());
    assert_eq!(fallback_body["store"], json!(false));
    assert_eq!(fallback_body["input"].as_array().unwrap().len(), 3);
    assert_eq!(fallback_body["input"][1]["type"], json!("function_call"));
    assert_eq!(
        fallback_body["input"][2]["type"],
        json!("function_call_output")
    );
}

#[tokio::test]
async fn long_escaped_arguments_decode_from_chunked_sse_without_delta_reassembly() {
    let arguments = json!({"content":"汉字😀 \"quote\" \\escape\n\t".repeat(600)});
    let item = json!({"type":"function_call","call_id":"long-call",
        "name":"write_file","arguments":arguments.to_string()});
    let events = [
        json!({"type":"response.function_call_arguments.delta","delta":"incomplete fragment"}),
        json!({"type":"response.function_call_arguments.done","arguments":"unused lifecycle field"}),
        json!({"type":"response.output_item.done","output_index":0,"item":item}),
        json!({"type":"response.completed","response":{"id":"long-response","output":[item]}}),
    ];
    let body = events
        .iter()
        .map(|e| format!("data: {e}\n\n"))
        .collect::<String>();
    let (url, request, server) = spawn_sse_server(200, &body).await;
    let provider = adapter(url, FakeCredentialSource::value(Some(FIXTURE_SECRET)));
    let result = ModelStreamingService::new(&provider)
        .consume(
            full(fixture_input(ModelRequestRequirements::default())),
            ModelStreamCancellation::new(),
        )
        .await
        .unwrap();
    let ModelStreamOutcome::Completed { events, .. } = result else {
        panic!("expected completion")
    };
    assert_eq!(
        events,
        vec![ModelEvent::ToolCall {
            call: ToolCall::new(
                ToolCallId::new("long-call"),
                ToolName::new("write_file"),
                arguments,
            )
        }]
    );
    request.await.unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn malformed_inner_arguments_have_safe_metadata_in_items_and_terminal_snapshots() {
    for final_snapshot in [false, true] {
        let arguments = "{\n\"content\":\"private-canary\\q\"}";
        let item = json!({"type":"function_call","call_id":"private-call",
            "name":"private-name","arguments":arguments});
        let event = if final_snapshot {
            json!({"type":"response.completed","response":{"id":"failure",
                "output":[item]}})
        } else {
            json!({"type":"response.output_item.done","item":item})
        };
        let (url, request, server) = spawn_sse_server(200, &format!("data: {event}\n\n")).await;
        let provider = adapter(url, FakeCredentialSource::value(Some(FIXTURE_SECRET)));
        let error = ModelStreamingService::new(&provider)
            .consume(
                full(fixture_input(ModelRequestRequirements::default())),
                ModelStreamCancellation::new(),
            )
            .await
            .unwrap_err();
        let ModelStreamingError::Provider(ModelProviderError::InvalidToolArguments { diagnostic }) =
            error
        else {
            panic!("expected strict protocol rejection")
        };
        let message = diagnostic.to_string();
        assert!(message.contains("category=Syntax, line=2"), "{message}");
        assert!(message.contains(&format!("arguments_bytes={}", arguments.len())));
        assert!(!message.contains("private"));
        request.await.unwrap();
        server.await.unwrap();
    }
}

#[tokio::test]
async fn malformed_unknown_and_uncompleted_streams_are_protocol_errors() {
    let cases = [
        ("data: {not-json}\n\n", "invalid Responses event JSON"),
        (
            "data: {\"type\":\"response.future\"}\n\n",
            "unsupported Responses event `response.future`",
        ),
        (
            "data: {\"type\":\"response.reasoning_text.delta\"}\n\n",
            "reasoning text delta event omitted `delta`",
        ),
        (
            "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"reasoning\",\"content\":[],\"encrypted_content\":\"opaque\"}}\n\n",
            "only unsupported encrypted content",
        ),
        (
            "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"reasoning\",\"content\":[{\"type\":\"future_reasoning\",\"text\":\"opaque\"}]}}\n\n",
            "unsupported completed reasoning content `future_reasoning`",
        ),
        (
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"partial\"}\n\n",
            "ended before `response.completed`",
        ),
    ];

    for (body, expected_message) in cases {
        let (base_url, request_receiver, server) = spawn_sse_server(200, body).await;
        let provider = adapter(base_url, FakeCredentialSource::value(Some(FIXTURE_SECRET)));
        let service = ModelStreamingService::new(&provider);
        let result = service
            .consume(
                full(fixture_input(ModelRequestRequirements::default())),
                ModelStreamCancellation::new(),
            )
            .await;

        match result {
            Err(ModelStreamingError::Provider(ModelProviderError::ProtocolFailure { message })) => {
                assert!(message.contains(expected_message))
            }
            other => panic!("expected protocol failure, got {other:?}"),
        }
        let _ = request_receiver.await.unwrap();
        server.await.unwrap();
    }
}

#[tokio::test]
async fn cancelling_a_pending_http_request_drops_the_adapter_stream() {
    let (base_url, request_receiver, release_server, server) = spawn_pending_server().await;
    let provider = adapter(base_url, FakeCredentialSource::value(Some(FIXTURE_SECRET)));
    let service = ModelStreamingService::new(&provider);
    let cancellation = ModelStreamCancellation::new();
    let consume = service.consume(
        full(fixture_input(ModelRequestRequirements::default())),
        cancellation.clone(),
    );
    tokio::pin!(consume);

    tokio::select! {
        result = &mut consume => panic!("request completed before cancellation: {result:?}"),
        request = request_receiver => {
            let _ = request.unwrap();
        }
    }
    cancellation.cancel();

    assert_eq!(
        timeout(Duration::from_secs(1), &mut consume)
            .await
            .expect("cancelled request should finish promptly")
            .unwrap(),
        ModelStreamOutcome::Cancelled { events: Vec::new() }
    );
    let _ = release_server.send(());
    server.await.unwrap();
}

#[tokio::test]
#[ignore = "requires explicit CYBER_AGENT_LIVE_TESTS=1 and a configured dedicated test key"]
async fn synthetic_primary_live_smoke_uses_provider_composition() {
    if std::env::var("CYBER_AGENT_LIVE_TESTS").as_deref() != Ok("1") {
        return;
    }
    let composition = live_provider::load();
    let service = ModelStreamingService::new(composition.primary_provider());
    let outcome = service
        .consume(
            full(ModelInputIR::current(
                "Answer the fixed synthetic test without external data.",
                vec![ContentItem::user_text(
                    "Reply with a short acknowledgement.",
                )],
                ModelRequestRequirements {
                    usage_reporting: true,
                    max_output_tokens: composition
                        .primary_provider()
                        .capabilities()
                        .max_output_tokens,
                    ..ModelRequestRequirements::default()
                },
            )),
            ModelStreamCancellation::new(),
        )
        .await
        .expect("authorized synthetic live request should complete");

    match outcome {
        ModelStreamOutcome::Completed { events, completion } => {
            assert!(
                events
                    .iter()
                    .any(|event| matches!(event, ModelEvent::TextDelta { .. }))
            );
            assert!(
                events
                    .iter()
                    .any(|event| matches!(event, ModelEvent::Usage { .. }))
            );
            if composition
                .primary_provider()
                .capabilities()
                .incremental_input
            {
                assert!(completion.response_id().is_some());
            }
        }
        ModelStreamOutcome::Cancelled { .. } => panic!("live smoke was unexpectedly cancelled"),
    }
}
