use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cyber_agent::adapters::openai::{
    OPENAI_API_KEY_REFERENCE, OpenAiCredentialSource, OpenAiCredentialSourceError,
    OpenAiResponsesAdapter, OpenAiResponsesConfig, ProcessEnvironmentCredentialSource,
};
use cyber_agent::adapters::tools::ToolRegistry;
use cyber_agent::app::model_streaming::{
    ModelStreamCancellation, ModelStreamOutcome, ModelStreamingError, ModelStreamingService,
};
use cyber_agent::domain::artifacts::ArtifactId;
use cyber_agent::domain::model::{
    CapabilityMappingVersion, ContentItem, MessageRole, ModelCapabilities, ModelEvent,
    ModelInputIR, ModelRequestRequirements, ModelUsage,
};
use cyber_agent::domain::tools::{ToolCall, ToolCallId, ToolName, ToolResult, ToolResultStatus};
use cyber_agent::ports::model::{ModelProvider, ModelProviderError};
use cyber_agent::ports::tool::ToolExecutor;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::time::timeout;

const FIXTURE_SECRET: &str = "fixture-secret";

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

impl OpenAiCredentialSource for FakeCredentialSource {
    fn read(&self, reference: &str) -> Result<Option<String>, OpenAiCredentialSourceError> {
        self.references
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(reference.to_owned());

        match &self.result {
            FakeCredentialResult::Value(value) => Ok(value.clone()),
            FakeCredentialResult::Failure(message) => {
                Err(OpenAiCredentialSourceError::ReadFailed {
                    message: message.clone(),
                })
            }
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

fn adapter(
    base_url: impl Into<String>,
    credential_source: Arc<dyn OpenAiCredentialSource>,
) -> OpenAiResponsesAdapter {
    OpenAiResponsesAdapter::new(
        OpenAiResponsesConfig::new(base_url, "fixture-model", capabilities()),
        credential_source,
    )
}

fn assert_stream_error(
    result: Result<cyber_agent::ports::model::ModelEventStream, ModelProviderError>,
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

    match provider.stream(fixture_input(ModelRequestRequirements::default())) {
        Ok(_) => {}
        Err(error) => panic!("fixture credential should be accepted: {error}"),
    }
    assert_eq!(source.references(), vec![OPENAI_API_KEY_REFERENCE]);
}

#[test]
fn credential_source_reports_missing_and_source_failure_without_network() {
    let missing = adapter("http://127.0.0.1:1/v1", FakeCredentialSource::value(None));
    assert_stream_error(
        missing.stream(fixture_input(ModelRequestRequirements::default())),
        ModelProviderError::CredentialUnavailable {
            credential_ref: OPENAI_API_KEY_REFERENCE.to_owned(),
        },
    );

    let failed = adapter(
        "http://127.0.0.1:1/v1",
        FakeCredentialSource::failure("fixture source unavailable"),
    );
    assert_stream_error(
        failed.stream(fixture_input(ModelRequestRequirements::default())),
        ModelProviderError::CredentialSourceFailure {
            credential_ref: OPENAI_API_KEY_REFERENCE.to_owned(),
            message: "credential source failed: fixture source unavailable".to_owned(),
        },
    );
}

#[test]
fn generic_openai_key_is_not_a_credential_fallback() {
    let source = FakeCredentialSource::generic_only("must-not-be-used");
    let provider = adapter("http://127.0.0.1:1/v1", source.clone());

    assert_stream_error(
        provider.stream(fixture_input(ModelRequestRequirements::default())),
        ModelProviderError::CredentialUnavailable {
            credential_ref: OPENAI_API_KEY_REFERENCE.to_owned(),
        },
    );
    assert_eq!(source.references(), vec![OPENAI_API_KEY_REFERENCE]);
}

#[test]
fn invalid_authorization_header_is_rejected_before_network() {
    let provider = adapter(
        "http://127.0.0.1:1/v1",
        FakeCredentialSource::value(Some("invalid\nsecret")),
    );

    assert_stream_error(
        provider.stream(fixture_input(ModelRequestRequirements::default())),
        ModelProviderError::CredentialInvalid {
            credential_ref: OPENAI_API_KEY_REFERENCE.to_owned(),
        },
    );
}

#[test]
fn deferred_request_features_fail_explicitly() {
    let cases = [
        (
            fixture_input(ModelRequestRequirements {
                structured_output: true,
                ..ModelRequestRequirements::default()
            }),
            "output contract",
        ),
        (
            fixture_input(ModelRequestRequirements {
                incremental_input: true,
                ..ModelRequestRequirements::default()
            }),
            "incremental model input",
        ),
        (
            ModelInputIR::current(
                "Inspect the image.",
                vec![ContentItem::ImageArtifact {
                    role: MessageRole::User,
                    artifact_id: ArtifactId::new("artifact-fixture"),
                }],
                ModelRequestRequirements::default(),
            ),
            "deferred Artifact resolver",
        ),
    ];

    for (input, message_fragment) in cases {
        let provider = adapter(
            "http://127.0.0.1:1/v1",
            FakeCredentialSource::value(Some(FIXTURE_SECRET)),
        );
        match provider.stream(input) {
            Err(ModelProviderError::RequestEncodingFailure { message }) => {
                assert!(message.contains(message_fragment));
            }
            Err(error) => panic!("unexpected error: {error}"),
            Ok(_) => panic!("deferred request input must not be silently accepted"),
        }
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
        socket.write_all(headers.as_bytes()).await.unwrap();
        for chunk in response_body.chunks(17) {
            socket.write_all(chunk).await.unwrap();
            tokio::task::yield_now().await;
        }
        socket.shutdown().await.unwrap();
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
        spawn_sse_server(200, include_str!("fixtures/openai/success.sse")).await;
    let provider = adapter(base_url, FakeCredentialSource::value(Some(FIXTURE_SECRET)));
    let service = ModelStreamingService::new(&provider);

    let outcome = service
        .consume(
            fixture_input(ModelRequestRequirements {
                usage_reporting: true,
                ..ModelRequestRequirements::default()
            }),
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
                ModelEvent::Completed,
            ],
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
        serde_json::from_str(include_str!("fixtures/openai/full_request.json")).unwrap();
    assert_eq!(actual, expected);
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
            ContentItem::ToolCall { call },
            ContentItem::ToolResult { result },
        ],
        ModelRequestRequirements::default(),
    )
    .with_tool_specs(vec![read_file]);

    assert!(matches!(
        service
            .consume(input, ModelStreamCancellation::new())
            .await
            .unwrap(),
        ModelStreamOutcome::Completed { .. }
    ));
    let request = request_receiver.await.unwrap();
    server.await.unwrap();
    let actual: Value = serde_json::from_str(&request.body).unwrap();
    let expected: Value =
        serde_json::from_str(include_str!("fixtures/openai/tool_request.json")).unwrap();
    assert_eq!(actual, expected);
}

#[tokio::test]
async fn http_rejection_is_a_typed_provider_error() {
    let (base_url, request_receiver, server) = spawn_sse_server(401, "unauthorized").await;
    let provider = adapter(base_url, FakeCredentialSource::value(Some(FIXTURE_SECRET)));
    let service = ModelStreamingService::new(&provider);

    assert_eq!(
        service
            .consume(
                fixture_input(ModelRequestRequirements::default()),
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
                fixture_input(ModelRequestRequirements::default()),
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
async fn malformed_unknown_and_uncompleted_streams_are_protocol_errors() {
    let cases = [
        ("data: {not-json}\n\n", "invalid Responses event JSON"),
        (
            "data: {\"type\":\"response.future\"}\n\n",
            "unsupported Responses event `response.future`",
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
                fixture_input(ModelRequestRequirements::default()),
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
        fixture_input(ModelRequestRequirements::default()),
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
#[ignore = "requires explicit CYBER_AGENT_LIVE_TESTS=1 and a dedicated OpenAI test key"]
async fn synthetic_openai_live_smoke() {
    if std::env::var("CYBER_AGENT_LIVE_TESTS").as_deref() != Ok("1") {
        return;
    }
    let model = std::env::var("CYBER_AGENT_OPENAI_LIVE_MODEL")
        .expect("CYBER_AGENT_OPENAI_LIVE_MODEL must select the synthetic live-test model");
    let provider = OpenAiResponsesAdapter::new(
        OpenAiResponsesConfig::new(
            "https://api.openai.com/v1",
            model,
            ModelCapabilities {
                text_input: true,
                image_input: false,
                streaming: true,
                tool_call: false,
                structured_output: false,
                incremental_input: false,
                usage_reporting: true,
                context_window: 1,
                capability_mapping_version: CapabilityMappingVersion::new(1),
            },
        ),
        Arc::new(ProcessEnvironmentCredentialSource),
    );
    let service = ModelStreamingService::new(&provider);
    let outcome = service
        .consume(
            ModelInputIR::current(
                "Answer the fixed synthetic test without external data.",
                vec![ContentItem::user_text(
                    "Reply with a short acknowledgement.",
                )],
                ModelRequestRequirements {
                    usage_reporting: true,
                    ..ModelRequestRequirements::default()
                },
            ),
            ModelStreamCancellation::new(),
        )
        .await
        .expect("authorized synthetic live request should complete");

    match outcome {
        ModelStreamOutcome::Completed { events } => {
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
            assert_eq!(events.last(), Some(&ModelEvent::Completed));
        }
        ModelStreamOutcome::Cancelled { .. } => panic!("live smoke was unexpectedly cancelled"),
    }
}
