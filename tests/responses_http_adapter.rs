use std::collections::BTreeMap;
use std::fs;
use std::io::Cursor;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use echidna::adapters::credentials::{
    AccountBoundCredential, CredentialResolution, CredentialSource, CredentialSourceError,
};
use echidna::adapters::project_event_store::Project;
use echidna::adapters::responses::{ResponsesHttpAdapter, ResponsesHttpConfig};
use echidna::adapters::tools::ToolRegistry;
use echidna::app::agent_loop::{AgentLoop, AgentTurnOutcome};
use echidna::app::cancellation::AgentCancellation;
use echidna::app::model_streaming::{
    ModelStreamCancellation, ModelStreamOutcome, ModelStreamingError, ModelStreamingService,
};
use echidna::domain::artifacts::ArtifactId;
use echidna::domain::context::{
    ProviderSession, ProviderSessionId, ProviderSessionRef, ProviderSessionRole,
};
use echidna::domain::events::{EventId, EventSequence, OccurredAt, StreamId};
use echidna::domain::model::{
    CapabilityMappingVersion, ContentItem, MessageRole, ModelCapabilities, ModelCapabilityError,
    ModelEvent, ModelInputIR, ModelRequestRequirements, ModelUsage, ReasoningEffort,
};
use echidna::domain::project::ProjectId;
use echidna::domain::request_audit::{
    RequestAdapterVersion, RequestAuditBinding, RequestAuditRunContext,
};
use echidna::domain::session::{SessionEvent, SessionReducer};
use echidna::domain::tools::{
    ExecutionId, ToolCall, ToolCallId, ToolExecutionOutcome, ToolName, ToolResult, ToolResultStatus,
};
use echidna::ports::model::{
    ModelProvider, ModelProviderError, ModelRequest, ModelResponseCompletion, ModelStreamEvent,
    ProviderResponseId,
};
use echidna::ports::tool::ToolExecutor;
use futures_util::StreamExt;
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
type CredentialReadResult = Result<Option<String>, CredentialSourceError>;

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

struct PendingCredentialSource {
    polls: Arc<AtomicUsize>,
    receiver: Mutex<Option<oneshot::Receiver<CredentialReadResult>>>,
}

struct AccountBoundCredentialSource {
    reference: String,
    bearer: String,
    account_id: String,
    resolutions: AtomicUsize,
}

impl AccountBoundCredentialSource {
    fn new(reference: &str, bearer: &str, account_id: &str) -> Arc<Self> {
        Arc::new(Self {
            reference: reference.to_owned(),
            bearer: bearer.to_owned(),
            account_id: account_id.to_owned(),
            resolutions: AtomicUsize::new(0),
        })
    }

    fn resolutions(&self) -> usize {
        self.resolutions.load(Ordering::SeqCst)
    }
}

impl CredentialSource for AccountBoundCredentialSource {
    fn read(&self, _: &str) -> Result<Option<String>, CredentialSourceError> {
        panic!("an account-bound credential source must not use the synchronous reader")
    }

    fn resolve(&self, reference: &str) -> CredentialResolution {
        assert_eq!(reference, self.reference);
        self.resolutions.fetch_add(1, Ordering::SeqCst);
        let bearer = self.bearer.clone();
        let account_id = self.account_id.clone();
        CredentialResolution::PendingAccountBound(Box::pin(async move {
            Ok(AccountBoundCredential::new(bearer, account_id))
        }))
    }
}

impl PendingCredentialSource {
    fn new() -> (Arc<Self>, oneshot::Sender<CredentialReadResult>) {
        let (sender, receiver) = oneshot::channel();
        (
            Arc::new(Self {
                polls: Arc::new(AtomicUsize::new(0)),
                receiver: Mutex::new(Some(receiver)),
            }),
            sender,
        )
    }

    fn polls(&self) -> usize {
        self.polls.load(Ordering::SeqCst)
    }
}

impl CredentialSource for PendingCredentialSource {
    fn read(&self, _: &str) -> Result<Option<String>, CredentialSourceError> {
        panic!("a pending credential source must not use the synchronous reader")
    }

    fn resolve(&self, _: &str) -> CredentialResolution {
        let receiver = self
            .receiver
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
            .expect("each fixture adapter has one model request");
        let polls = Arc::clone(&self.polls);
        CredentialResolution::Pending(Box::pin(async move {
            polls.fetch_add(1, Ordering::SeqCst);
            receiver
                .await
                .expect("fixture credential sender remains alive")
        }))
    }
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
        reasoning_effort: None,
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

fn subscription_capabilities() -> ModelCapabilities {
    let mut capabilities = capabilities();
    capabilities.incremental_input = false;
    capabilities.usage_reporting = false;
    capabilities
}

fn subscription_adapter(
    base_url: impl Into<String>,
    credential_source: Arc<dyn CredentialSource>,
) -> ResponsesHttpAdapter {
    ResponsesHttpAdapter::new(
        ResponsesHttpConfig::new(
            base_url,
            "fixture-subscription-model",
            "chatgpt_subscription:fixture",
            subscription_capabilities(),
        )
        .with_chatgpt_subscription_profile(),
        credential_source,
    )
}

fn codex_subscription_adapter(
    base_url: impl Into<String>,
    credential_source: Arc<dyn CredentialSource>,
) -> ResponsesHttpAdapter {
    ResponsesHttpAdapter::new(
        ResponsesHttpConfig::new(
            base_url,
            "fixture-codex-model",
            "codex_subscription:fixture",
            subscription_capabilities(),
        )
        .with_codex_subscription_profile(),
        credential_source,
    )
}

fn subscription_full(input: ModelInputIR, scope: &str) -> ModelRequest {
    ModelRequest::full(input).with_opaque_continuation_scope(
        serde_json::from_value(json!(scope)).expect("fixture scope serializes"),
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
    spawn_sse_server_chunked(status, body, 17).await
}

async fn spawn_sse_server_chunked(
    status: u16,
    body: &str,
    chunk_bytes: usize,
) -> (String, oneshot::Receiver<CapturedRequest>, JoinHandle<()>) {
    spawn_sse_server_chunked_with_declared_length(status, body, chunk_bytes, body.len()).await
}

async fn spawn_sse_server_chunked_with_declared_length(
    status: u16,
    body: &str,
    chunk_bytes: usize,
    declared_body_bytes: usize,
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
            declared_body_bytes
        );
        if socket.write_all(headers.as_bytes()).await.is_err() {
            return;
        }
        for chunk in response_body.chunks(chunk_bytes) {
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
    assert!(!request.headers.contains_key("chatgpt-account-id"));
    let actual: Value = serde_json::from_str(&request.body).unwrap();
    let expected: Value =
        serde_json::from_str(include_str!("fixtures/responses/full_request.json")).unwrap();
    assert_eq!(actual, expected);
    assert!(actual.get("reasoning").is_none());
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
async fn subscription_profile_encodes_full_stateless_request_and_namespace_tools() {
    let (base_url, request_receiver, server) = spawn_sse_server(
        200,
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"subscription-request\",\"status\":\"completed\"}}\n\n",
    )
    .await;
    let provider =
        subscription_adapter(base_url, FakeCredentialSource::value(Some(FIXTURE_SECRET)));
    let read_file = ToolRegistry::for_project(env!("CARGO_MANIFEST_DIR"))
        .specs()
        .into_iter()
        .find(|spec| spec.name.as_str() == "read_file")
        .expect("fixture registry supplies read_file");
    let input =
        fixture_input(ModelRequestRequirements::default()).with_tool_specs(vec![read_file.clone()]);

    assert!(matches!(
        ModelStreamingService::new(&provider)
            .consume(
                subscription_full(input, "subscription-request-scope"),
                ModelStreamCancellation::new(),
            )
            .await,
        Ok(ModelStreamOutcome::Completed { .. })
    ));

    let request = request_receiver.await.unwrap();
    server.await.unwrap();
    assert!(!request.headers.contains_key("chatgpt-account-id"));
    let wire: Value = serde_json::from_str(&request.body).unwrap();
    assert_eq!(wire["model"], json!("fixture-subscription-model"));
    assert_eq!(wire["store"], json!(false));
    assert_eq!(wire["stream"], json!(true));
    assert_eq!(wire["include"], json!(["reasoning.encrypted_content"]));
    assert!(wire.get("previous_response_id").is_none());
    assert!(wire.get("max_output_tokens").is_none());
    assert!(wire.get("reasoning").is_none());
    assert_eq!(
        wire["input"],
        json!([{
            "type": "message",
            "role": "user",
            "content": [
                {"type": "input_text", "text": "Inspect the target manifest."},
                {"type": "input_text", "text": "Report only reproducible impact."},
            ],
        }])
    );
    let tools = wire["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 1);
    let namespace = &tools[0];
    assert_eq!(namespace["type"], json!("namespace"));
    assert_eq!(namespace["name"], json!("echidna"));
    assert_eq!(
        namespace["description"],
        json!("Echidna local agent tools.")
    );
    assert_eq!(namespace["tools"][0]["type"], json!("function"));
    assert_eq!(
        namespace["tools"][0]["name"],
        json!(read_file.name.as_str())
    );
    assert_eq!(namespace["tools"][0]["parameters"], read_file.input_schema);
}

#[tokio::test]
async fn codex_subscription_profile_sends_verified_account_header_and_plain_functions() {
    let (base_url, request_receiver, server) = spawn_sse_server(
        200,
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"codex-request\",\"status\":\"completed\"}}\n\n",
    )
    .await;
    let credentials = AccountBoundCredentialSource::new(
        "codex_subscription:fixture",
        "codex-bearer-fixture",
        "workspace-fixture",
    );
    let provider = codex_subscription_adapter(base_url, credentials.clone());
    let read_file = ToolRegistry::for_project(env!("CARGO_MANIFEST_DIR"))
        .specs()
        .into_iter()
        .find(|spec| spec.name.as_str() == "read_file")
        .expect("fixture registry supplies read_file");
    let input =
        fixture_input(ModelRequestRequirements::default()).with_tool_specs(vec![read_file.clone()]);

    assert!(matches!(
        ModelStreamingService::new(&provider)
            .consume(full(input), ModelStreamCancellation::new(),)
            .await,
        Ok(ModelStreamOutcome::Completed { .. })
    ));

    let request = request_receiver.await.unwrap();
    server.await.unwrap();
    assert_eq!(request.request_line, "POST /v1/responses HTTP/1.1");
    assert_eq!(
        request.headers.get("authorization").map(String::as_str),
        Some("Bearer codex-bearer-fixture")
    );
    assert_eq!(
        request
            .headers
            .get("chatgpt-account-id")
            .map(String::as_str),
        Some("workspace-fixture")
    );
    assert_eq!(credentials.resolutions(), 1);

    let wire: Value = serde_json::from_str(&request.body).unwrap();
    assert_eq!(wire["model"], json!("fixture-codex-model"));
    assert_eq!(
        wire["instructions"],
        json!("Keep evidence and hypotheses separate.")
    );
    assert_eq!(wire["store"], json!(false));
    assert_eq!(wire["stream"], json!(true));
    assert_eq!(wire["include"], json!(["reasoning.encrypted_content"]));
    assert!(wire.get("previous_response_id").is_none());
    assert!(wire.get("max_output_tokens").is_none());
    assert_eq!(wire["reasoning"], json!({"summary": "auto"}));
    assert_eq!(wire["tools"][0]["type"], json!("function"));
    assert_eq!(wire["tools"][0]["name"], json!(read_file.name.as_str()));
    assert_eq!(wire["tools"][0]["parameters"], read_file.input_schema);
    assert!(wire["tools"][0].get("namespace").is_none());
}

#[tokio::test]
async fn explicit_reasoning_effort_uses_the_same_nested_wire_field_for_all_profiles() {
    for profile in ["standard", "siwc", "codex"] {
        let (base_url, request_receiver, server) = spawn_sse_server(
            200,
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"reasoning-effort\",\"status\":\"completed\"}}\n\n",
        )
        .await;
        let mut capabilities = capabilities();
        if profile != "standard" {
            capabilities.incremental_input = false;
            capabilities.usage_reporting = false;
        }
        capabilities.reasoning_effort = Some(ReasoningEffort::XHigh);
        let provider = match profile {
            "standard" => ResponsesHttpAdapter::new(
                ResponsesHttpConfig::new(
                    base_url,
                    "fixture-reasoning-standard",
                    FIXTURE_CREDENTIAL_REFERENCE,
                    capabilities,
                ),
                FakeCredentialSource::value(Some(FIXTURE_SECRET)),
            ),
            "siwc" => ResponsesHttpAdapter::new(
                ResponsesHttpConfig::new(
                    base_url,
                    "fixture-reasoning-siwc",
                    "chatgpt_subscription:fixture",
                    capabilities,
                )
                .with_chatgpt_subscription_profile(),
                FakeCredentialSource::value(Some(FIXTURE_SECRET)),
            ),
            "codex" => ResponsesHttpAdapter::new(
                ResponsesHttpConfig::new(
                    base_url,
                    "fixture-reasoning-codex",
                    "codex_subscription:fixture",
                    capabilities,
                )
                .with_codex_subscription_profile(),
                AccountBoundCredentialSource::new(
                    "codex_subscription:fixture",
                    "codex-bearer-fixture",
                    "workspace-fixture",
                ),
            ),
            _ => unreachable!(),
        };
        let input = fixture_input(ModelRequestRequirements {
            reasoning_effort: Some(ReasoningEffort::XHigh),
            ..ModelRequestRequirements::default()
        });
        let request = if profile == "siwc" {
            subscription_full(input, "reasoning-effort-siwc")
        } else {
            full(input)
        };

        assert!(matches!(
            ModelStreamingService::new(&provider)
                .consume(request, ModelStreamCancellation::new())
                .await,
            Ok(ModelStreamOutcome::Completed { .. })
        ));
        let request = request_receiver.await.unwrap();
        server.await.unwrap();
        let wire: Value = serde_json::from_str(&request.body).unwrap();
        let expected = if profile == "codex" {
            json!({"effort": "xhigh", "summary": "auto"})
        } else {
            json!({"effort": "xhigh"})
        };
        assert_eq!(wire["reasoning"], expected, "{profile}");
    }
}

#[tokio::test]
async fn codex_summary_events_require_a_valid_summary_index_while_other_profiles_ignore_it() {
    let body = [
        json!({
            "type": "response.reasoning_summary_text.delta",
            "output_index": 0,
            "item_id": "summary-index-fixture",
            "summary_index": "invalid",
            "delta": "must stay ignored outside Codex",
        }),
        json!({
            "type": "response.completed",
            "response": {"id": "summary-index-response", "status": "completed"},
        }),
    ]
    .into_iter()
    .map(|event| format!("data: {event}\n\n"))
    .collect::<String>();

    for profile in ["standard", "siwc", "codex"] {
        let (base_url, request, server) = spawn_sse_server(200, &body).await;
        let provider = match profile {
            "standard" => adapter(base_url, FakeCredentialSource::value(Some(FIXTURE_SECRET))),
            "siwc" => {
                subscription_adapter(base_url, FakeCredentialSource::value(Some(FIXTURE_SECRET)))
            }
            "codex" => codex_subscription_adapter(
                base_url,
                AccountBoundCredentialSource::new(
                    "codex_subscription:fixture",
                    "codex-summary-index-bearer",
                    "workspace-summary-index",
                ),
            ),
            _ => unreachable!("the fixed profiles are exhaustive"),
        };
        let model_request = match profile {
            "siwc" => subscription_full(
                fixture_input(ModelRequestRequirements::default()),
                "siwc-summary-index-scope",
            ),
            _ => full(fixture_input(ModelRequestRequirements::default())),
        };
        let result = ModelStreamingService::new(&provider)
            .consume(model_request, ModelStreamCancellation::new())
            .await;

        if profile == "codex" {
            assert!(matches!(
                result,
                Err(ModelStreamingError::Provider(ModelProviderError::ProtocolFailure { message }))
                    if message.contains("valid `summary_index`")
            ));
        } else {
            assert_eq!(
                result,
                Ok(ModelStreamOutcome::Completed {
                    events: Vec::new(),
                    completion: ModelResponseCompletion::new(ProviderResponseId::new(
                        "summary-index-response"
                    )),
                }),
                "{profile} must retain its ignored-summary behavior"
            );
        }
        request.await.unwrap();
        server.await.unwrap();
    }
}

#[tokio::test]
async fn codex_summary_events_require_explicit_output_and_item_identity() {
    let cases = [
        (
            "missing-output-index",
            json!({
                "type": "response.reasoning_summary_text.delta",
                "item_id": "summary-identity-item",
                "summary_index": 0,
                "delta": "missing output index",
            }),
            "omitted `output_index`",
        ),
        (
            "missing-item-id",
            json!({
                "type": "response.reasoning_summary_text.done",
                "output_index": 0,
                "summary_index": 0,
                "text": "missing item id",
            }),
            "omitted non-empty `item_id`",
        ),
    ];

    for (case, event, expected) in cases {
        let body = format!("data: {event}\n\n");
        let (base_url, request, server) = spawn_sse_server(200, &body).await;
        let provider = codex_subscription_adapter(
            base_url,
            AccountBoundCredentialSource::new(
                "codex_subscription:fixture",
                &format!("codex-summary-{case}-bearer"),
                &format!("workspace-summary-{case}"),
            ),
        );

        assert!(matches!(
            ModelStreamingService::new(&provider)
                .consume(
                    full(fixture_input(ModelRequestRequirements::default())),
                    ModelStreamCancellation::new(),
                )
                .await,
            Err(ModelStreamingError::Provider(ModelProviderError::ProtocolFailure { message }))
                if message.contains(expected)
        ));
        request.await.unwrap();
        server.await.unwrap();
    }
}

#[tokio::test]
async fn codex_completed_summary_items_reject_invalid_parts() {
    let cases = [
        (
            "wrong-type",
            json!({"type": "output_text", "text": "not a summary"}),
            "unsupported content",
        ),
        (
            "missing-text",
            json!({"type": "summary_text"}),
            "omitted string `text`",
        ),
        (
            "non-string-text",
            json!({"type": "summary_text", "text": 7}),
            "omitted string `text`",
        ),
    ];

    for (case, part, expected) in cases {
        let item = json!({
            "type": "reasoning",
            "id": format!("summary-invalid-part-{case}"),
            "status": "completed",
            "content": [],
            "summary": [part],
        });
        let body = format!(
            "data: {}\n\n",
            json!({"type": "response.output_item.done", "output_index": 0, "item": item})
        );
        let (base_url, request, server) = spawn_sse_server(200, &body).await;
        let provider = codex_subscription_adapter(
            base_url,
            AccountBoundCredentialSource::new(
                "codex_subscription:fixture",
                &format!("codex-summary-invalid-part-{case}-bearer"),
                &format!("workspace-summary-invalid-part-{case}"),
            ),
        );

        assert!(matches!(
            ModelStreamingService::new(&provider)
                .consume(
                    full(fixture_input(ModelRequestRequirements::default())),
                    ModelStreamCancellation::new(),
                )
                .await,
            Err(ModelStreamingError::Provider(ModelProviderError::ProtocolFailure { message }))
                if message.contains(expected)
        ));
        request.await.unwrap();
        server.await.unwrap();
    }
}

#[tokio::test]
async fn codex_summary_events_reject_conflicting_identity_and_order() {
    let cases = [
        (
            "identity",
            vec![
                json!({
                    "type": "response.reasoning_summary_text.delta",
                    "output_index": 0,
                    "item_id": "summary-first-item",
                    "summary_index": 0,
                    "delta": "first",
                }),
                json!({
                    "type": "response.reasoning_summary_text.done",
                    "output_index": 0,
                    "item_id": "summary-second-item",
                    "summary_index": 0,
                    "text": "first",
                }),
            ],
            "text item identity changed",
        ),
        (
            "order",
            vec![
                json!({
                    "type": "response.reasoning_summary_text.delta",
                    "output_index": 0,
                    "item_id": "summary-order-item",
                    "summary_index": 1,
                    "delta": "later",
                }),
                json!({
                    "type": "response.reasoning_summary_text.delta",
                    "output_index": 0,
                    "item_id": "summary-order-item",
                    "summary_index": 0,
                    "delta": "earlier",
                }),
            ],
            "final text would reorder prior output",
        ),
    ];

    for (case, events, expected) in cases {
        let body = events
            .into_iter()
            .map(|event| format!("data: {event}\n\n"))
            .collect::<String>();
        let (base_url, request, server) = spawn_sse_server(200, &body).await;
        let provider = codex_subscription_adapter(
            base_url,
            AccountBoundCredentialSource::new(
                "codex_subscription:fixture",
                &format!("codex-summary-{case}-bearer"),
                &format!("workspace-summary-{case}"),
            ),
        );

        assert!(matches!(
            ModelStreamingService::new(&provider)
                .consume(
                    full(fixture_input(ModelRequestRequirements::default())),
                    ModelStreamCancellation::new(),
                )
                .await,
            Err(ModelStreamingError::Provider(ModelProviderError::ProtocolFailure { message }))
                if message.contains(expected)
        ));
        request.await.unwrap();
        server.await.unwrap();
    }
}

#[tokio::test]
async fn codex_summary_finalization_rejects_a_conflicting_completed_item() {
    let first = json!({
        "type": "reasoning",
        "id": "summary-conflict-item",
        "status": "completed",
        "content": [],
        "summary": [{"type": "summary_text", "text": "first"}],
    });
    let second = json!({
        "type": "reasoning",
        "id": "summary-conflict-item",
        "status": "completed",
        "content": [],
        "summary": [{"type": "summary_text", "text": "second"}],
    });
    let body = [
        json!({"type": "response.output_item.done", "output_index": 0, "item": first}),
        json!({
            "type": "response.completed",
            "response": {
                "id": "summary-conflict-response",
                "status": "completed",
                "output": [second],
            },
        }),
    ]
    .into_iter()
    .map(|event| format!("data: {event}\n\n"))
    .collect::<String>();
    let (base_url, request, server) = spawn_sse_server(200, &body).await;
    let provider = codex_subscription_adapter(
        base_url,
        AccountBoundCredentialSource::new(
            "codex_subscription:fixture",
            "codex-summary-conflict-bearer",
            "workspace-summary-conflict",
        ),
    );
    let result = ModelStreamingService::new(&provider)
        .consume(
            full(fixture_input(ModelRequestRequirements::default())),
            ModelStreamCancellation::new(),
        )
        .await;

    assert!(matches!(
        result,
        Err(ModelStreamingError::Provider(ModelProviderError::ProtocolFailure { message }))
            if message.contains("final text changed after finalization")
    ));
    request.await.unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn codex_final_snapshot_summary_preserves_prior_completed_item_proof() {
    let prior = json!({
        "type": "reasoning",
        "id": "summary-final-only-item",
        "status": "completed",
        "content": [],
        "summary": [],
    });
    let completed = json!({
        "type": "reasoning",
        "id": "summary-final-only-item",
        "status": "completed",
        "content": [],
        "summary": [{"type": "summary_text", "text": "final-only summary"}],
    });
    let body = [
        json!({"type": "response.output_item.done", "output_index": 0, "item": prior}),
        json!({
            "type": "response.completed",
            "response": {
                "id": "summary-final-only-response",
                "status": "completed",
                "output": [completed],
            },
        }),
    ]
    .into_iter()
    .map(|event| format!("data: {event}\n\n"))
    .collect::<String>();
    let (base_url, request, server) = spawn_sse_server(200, &body).await;
    let provider = codex_subscription_adapter(
        base_url,
        AccountBoundCredentialSource::new(
            "codex_subscription:fixture",
            "codex-summary-final-only-bearer",
            "workspace-summary-final-only",
        ),
    );

    assert_eq!(
        ModelStreamingService::new(&provider)
            .consume(
                full(fixture_input(ModelRequestRequirements::default())),
                ModelStreamCancellation::new(),
            )
            .await,
        Ok(ModelStreamOutcome::Completed {
            events: Vec::new(),
            completion: ModelResponseCompletion::new(ProviderResponseId::new(
                "summary-final-only-response"
            )),
        })
    );
    request.await.unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn codex_completed_reasoning_item_summary_succeeds_without_a_final_snapshot() {
    let item = json!({
        "type": "reasoning",
        "id": "summary-item-only-item",
        "status": "completed",
        "content": [],
        "summary": [{"type": "summary_text", "text": "item-only summary"}],
    });
    let body = [
        json!({"type": "response.output_item.done", "output_index": 0, "item": item}),
        json!({
            "type": "response.completed",
            "response": {"id": "summary-item-only-response", "status": "completed"},
        }),
    ]
    .into_iter()
    .map(|event| format!("data: {event}\n\n"))
    .collect::<String>();
    let (base_url, request, server) = spawn_sse_server(200, &body).await;
    let provider = codex_subscription_adapter(
        base_url,
        AccountBoundCredentialSource::new(
            "codex_subscription:fixture",
            "codex-summary-item-only-bearer",
            "workspace-summary-item-only",
        ),
    );

    assert_eq!(
        ModelStreamingService::new(&provider)
            .consume(
                full(fixture_input(ModelRequestRequirements::default())),
                ModelStreamCancellation::new(),
            )
            .await,
        Ok(ModelStreamOutcome::Completed {
            events: Vec::new(),
            completion: ModelResponseCompletion::new(ProviderResponseId::new(
                "summary-item-only-response"
            )),
        })
    );
    request.await.unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn codex_final_snapshot_summary_does_not_synthesize_a_completed_reasoning_item() {
    let body = format!(
        "data: {}\n\n",
        json!({
            "type": "response.completed",
            "response": {
                "id": "summary-no-prior-response",
                "status": "completed",
                "output": [{
                    "type": "reasoning",
                    "id": "summary-no-prior-item",
                    "status": "completed",
                    "content": [],
                    "summary": [{"type": "summary_text", "text": "unproven"}],
                }],
            },
        })
    );
    let (base_url, request, server) = spawn_sse_server(200, &body).await;
    let provider = codex_subscription_adapter(
        base_url,
        AccountBoundCredentialSource::new(
            "codex_subscription:fixture",
            "codex-summary-no-prior-bearer",
            "workspace-summary-no-prior",
        ),
    );
    let result = ModelStreamingService::new(&provider)
        .consume(
            full(fixture_input(ModelRequestRequirements::default())),
            ModelStreamCancellation::new(),
        )
        .await;

    assert!(matches!(
        result,
        Err(ModelStreamingError::Provider(ModelProviderError::ProtocolFailure { message }))
            if message.contains("completed output disagrees with completed non-text items")
    ));
    request.await.unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn codex_metadata_annotation_is_ignored_until_response_completed() {
    let metadata_canary = "codex-metadata-must-not-become-output";
    let body = [
        json!({
            "type": "response.output_text.delta",
            "delta": "Codex answer",
        }),
        json!({
            "type": "response.metadata",
            "metadata": {"canary": metadata_canary},
        }),
        json!({
            "type": "response.completed",
            "response": {"id": "codex-metadata-response", "status": "completed"},
        }),
    ]
    .into_iter()
    .map(|event| format!("data: {event}\n\n"))
    .collect::<String>();
    let (base_url, request, server) = spawn_sse_server(200, &body).await;
    let provider = codex_subscription_adapter(
        base_url,
        AccountBoundCredentialSource::new(
            "codex_subscription:fixture",
            "codex-metadata-bearer",
            "workspace-metadata",
        ),
    );

    let outcome = ModelStreamingService::new(&provider)
        .consume(
            full(fixture_input(ModelRequestRequirements::default())),
            ModelStreamCancellation::new(),
        )
        .await
        .expect("Codex metadata must not terminate a completed response");

    assert_eq!(
        outcome,
        ModelStreamOutcome::Completed {
            events: vec![ModelEvent::TextDelta {
                text: "Codex answer".to_owned(),
            }],
            completion: ModelResponseCompletion::new(ProviderResponseId::new(
                "codex-metadata-response",
            )),
        }
    );
    assert!(!format!("{outcome:?}").contains(metadata_canary));
    request.await.unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn codex_metadata_annotation_does_not_complete_or_mask_later_stream_errors() {
    let metadata_canary = "codex-metadata-terminal-canary";
    let summary_canary = "codex-summary-terminal-canary";
    let terminals = [
        (
            "eof",
            String::new(),
            "response stream ended before `response.completed`",
        ),
        (
            "malformed",
            "data: {not-json}\n\n".to_owned(),
            "invalid Responses event JSON",
        ),
        (
            "unknown",
            "data: {\"type\":\"response.future\"}\n\n".to_owned(),
            "unsupported Responses event `response.future`",
        ),
        (
            "failed",
            format!(
                "data: {}\n\n",
                json!({"type": "response.failed", "response": {"error": {
                    "code": "metadata-terminal-failure", "message": "metadata terminal failure"
                }}})
            ),
            "Codex subscription Responses stream reported response.failed",
        ),
        (
            "incomplete",
            format!(
                "data: {}\n\n",
                json!({"type": "response.incomplete", "response": {
                    "incomplete_details": {"reason": "metadata terminal limit"}
                }})
            ),
            "Codex subscription Responses stream reported response.incomplete",
        ),
    ];

    for (case, terminal, expected_error) in terminals {
        let body = format!(
            "data: {}\n\ndata: {}\n\n{terminal}",
            json!({
                "type": "response.reasoning_summary_text.delta",
                "output_index": 0,
                "item_id": "codex-summary-terminal-item",
                "summary_index": 0,
                "delta": summary_canary,
            }),
            json!({"type": "response.metadata", "metadata": {"canary": metadata_canary}})
        );
        let (base_url, request, server) = spawn_sse_server(200, &body).await;
        let provider = codex_subscription_adapter(
            base_url,
            AccountBoundCredentialSource::new(
                "codex_subscription:fixture",
                "codex-metadata-terminal-bearer",
                "workspace-metadata-terminal",
            ),
        );

        let error = ModelStreamingService::new(&provider)
            .consume(
                full(fixture_input(ModelRequestRequirements::default())),
                ModelStreamCancellation::new(),
            )
            .await
            .expect_err("{case}: metadata must not fabricate a completed response");
        let visible_error = error.to_string();
        assert!(
            visible_error.contains(expected_error),
            "{case}: {visible_error}"
        );
        assert!(
            !visible_error.contains(metadata_canary),
            "{case}: metadata must not enter the protocol error"
        );
        assert!(
            !visible_error.contains(summary_canary),
            "{case}: summary presentation must not enter the protocol error"
        );
        request.await.unwrap();
        server.await.unwrap();
    }
}

#[tokio::test]
async fn standard_and_siwc_profiles_reject_codex_metadata_annotations() {
    for profile in ["standard", "siwc"] {
        let body = format!(
            "data: {}\n\n",
            json!({"type": "response.metadata", "metadata": {"canary": profile}})
        );
        let (base_url, request, server) = spawn_sse_server(200, &body).await;
        let provider = match profile {
            "standard" => adapter(base_url, FakeCredentialSource::value(Some(FIXTURE_SECRET))),
            "siwc" => {
                subscription_adapter(base_url, FakeCredentialSource::value(Some(FIXTURE_SECRET)))
            }
            _ => unreachable!("the fixed profiles are exhaustive"),
        };
        let model_request = match profile {
            "standard" => full(fixture_input(ModelRequestRequirements::default())),
            "siwc" => subscription_full(
                fixture_input(ModelRequestRequirements::default()),
                "siwc-metadata-rejection-scope",
            ),
            _ => unreachable!("the fixed profiles are exhaustive"),
        };

        let result = ModelStreamingService::new(&provider)
            .consume(model_request, ModelStreamCancellation::new())
            .await;
        assert!(
            matches!(
                result,
                Err(ModelStreamingError::Provider(ModelProviderError::ProtocolFailure { message }))
                    if message.contains("unsupported Responses event `response.metadata`")
            ),
            "{profile} must retain the explicit metadata rejection"
        );
        request.await.unwrap();
        server.await.unwrap();
    }
}

#[test]
fn account_bound_credentials_are_rejected_by_standard_and_siwc_profiles_before_transport() {
    for (profile, provider) in [
        (
            "standard",
            adapter(
                "http://127.0.0.1:1/v1",
                AccountBoundCredentialSource::new(
                    FIXTURE_CREDENTIAL_REFERENCE,
                    "bound-standard",
                    "workspace-standard",
                ),
            ),
        ),
        (
            "siwc",
            subscription_adapter(
                "http://127.0.0.1:1/v1",
                AccountBoundCredentialSource::new(
                    "chatgpt_subscription:fixture",
                    "bound-siwc",
                    "workspace-siwc",
                ),
            ),
        ),
    ] {
        assert!(
            matches!(
                provider.stream(full(fixture_input(ModelRequestRequirements::default()))),
                Err(ModelProviderError::RequestEncodingFailure { message })
                    if message.contains("account-bound credentials require the explicit Codex subscription")
            ),
            "{profile} must reject a Codex account-bound credential before transport"
        );
    }
}

#[tokio::test]
async fn codex_account_credentials_reject_empty_or_oversized_header_values_before_transport() {
    for account_id in [
        String::new(),
        "x".repeat(257),
        "workspace-fixture\r\nforged-header: value".to_owned(),
    ] {
        let credentials = AccountBoundCredentialSource::new(
            "codex_subscription:fixture",
            "bound-codex",
            &account_id,
        );
        let provider = codex_subscription_adapter("http://127.0.0.1:1/v1", credentials.clone());
        let mut stream = provider
            .stream(full(fixture_input(ModelRequestRequirements::default())))
            .expect("the credential resolves only when the stream is polled");
        assert!(matches!(
            stream.next().await,
            Some(Err(ModelProviderError::CredentialInvalid { credential_ref }))
                if credential_ref == "codex_subscription:fixture"
        ));
        assert_eq!(
            credentials.resolutions(),
            1,
            "the account-bound credential must be validated only while the Codex stream is polled"
        );
    }
}

#[test]
fn codex_subscription_rejects_unbound_ready_and_pending_credentials_before_transport() {
    let ready = FakeCredentialSource::value(Some(FIXTURE_SECRET));
    let ready_provider = codex_subscription_adapter("http://127.0.0.1:1/v1", ready.clone());
    assert!(matches!(
        ready_provider.stream(full(fixture_input(ModelRequestRequirements::default()))),
        Err(ModelProviderError::RequestEncodingFailure { message })
            if message.contains("Codex subscription Responses requires an account-bound credential")
    ));
    assert_eq!(ready.references(), vec!["codex_subscription:fixture"]);

    let (pending, _release) = PendingCredentialSource::new();
    let pending_provider = codex_subscription_adapter("http://127.0.0.1:1/v1", pending.clone());
    assert!(matches!(
        pending_provider.stream(full(fixture_input(ModelRequestRequirements::default()))),
        Err(ModelProviderError::RequestEncodingFailure { message })
            if message.contains("Codex subscription Responses requires an account-bound credential")
    ));
    assert_eq!(
        pending.polls(),
        0,
        "Codex must reject an unbound pending credential without polling it"
    );
}

#[tokio::test]
async fn subscription_tool_loop_replays_opaque_reasoning_without_disclosure() {
    for profile in ["siwc", "codex"] {
        let metadata_canary = format!("subscription-metadata-canary-{profile}");
        let summary_text = format!("subscription-summary-canary-{profile}");
        let summary = json!([{"type": "summary_text", "text": summary_text}]);
        let reasoning = json!({
            "type": "reasoning",
            "id": "rsn-sub",
            "status": "completed",
            "content": [],
            "encrypted_content": "ciphertext-canary",
            "summary": summary.clone(),
        });
        let mut call = json!({
            "type": "function_call",
            "call_id": "call-subscription",
            "name": "search_files",
            "arguments": "{\"pattern\":\"echidna\",\"paths\":[\"Cargo.toml\"]}",
        });
        if profile == "siwc" {
            call["namespace"] = json!("echidna");
        }
        let mut first_events = vec![
            json!({"type": "response.output_item.done", "output_index": 0, "item": reasoning}),
            json!({"type": "response.output_item.done", "output_index": 1, "item": call}),
        ];
        if profile == "codex" {
            first_events.push(json!({
                "type": "response.metadata",
                "metadata": {"canary": metadata_canary},
            }));
        }
        first_events.push(json!({"type": "response.completed", "response": {
            "id": "response-subscription", "status": "completed", "output": [reasoning, call],
        }}));
        let first = first_events
            .into_iter()
            .map(|event| format!("data: {event}\n\n"))
            .collect::<String>();
        let second = concat!(
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"subscription search complete\"}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"response-subscription-final\",\"status\":\"completed\"}}\n\n",
        );
        let (base_url, mut requests, server) =
            spawn_sse_sequence_server(vec![first, second.to_owned()]).await;
        let provider = match profile {
            "siwc" => {
                subscription_adapter(base_url, FakeCredentialSource::value(Some(FIXTURE_SECRET)))
            }
            "codex" => codex_subscription_adapter(
                base_url,
                AccountBoundCredentialSource::new(
                    "codex_subscription:fixture",
                    "codex-opaque-bearer",
                    "workspace-opaque",
                ),
            ),
            _ => unreachable!("the fixed profiles are exhaustive"),
        };
        let tools = ToolRegistry::for_project(env!("CARGO_MANIFEST_DIR"));
        let mut agent = AgentLoop::new(&provider, &tools);
        let audit_context = RequestAuditRunContext::new(
            RequestAuditBinding::new(
                ProjectId::new(format!("project-{profile}-opaque")),
                StreamId::new(format!("session:{profile}-opaque")),
            )
            .unwrap(),
            ResponsesHttpAdapter::request_adapter_version(),
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
            } if assistant_text == "subscription search complete" && executions.len() == 1
        ));
        assert!(agent.history().items().iter().all(|item| {
            !matches!(
                item,
                ContentItem::Reasoning { .. } | ContentItem::OpaqueContinuation { .. }
            )
        }));
        assert!(!format!("{:?}", agent.history()).contains("ciphertext-canary"));
        assert!(!format!("{audits:?}").contains("ciphertext-canary"));
        assert!(!format!("{:?}", agent.history()).contains(&summary_text));
        assert!(!format!("{audits:?}").contains(&summary_text));
        assert!(!format!("{:?}", agent.history()).contains(&metadata_canary));
        assert!(!format!("{audits:?}").contains(&metadata_canary));

        let first_request = requests.recv().await.unwrap();
        let second_request = requests.recv().await.unwrap();
        server.await.unwrap();
        let first_wire: Value = serde_json::from_str(&first_request.body).unwrap();
        let second_wire: Value = serde_json::from_str(&second_request.body).unwrap();
        match profile {
            "siwc" => {
                assert!(!first_request.headers.contains_key("chatgpt-account-id"));
                assert!(!second_request.headers.contains_key("chatgpt-account-id"));
                assert_eq!(first_wire["tools"][0]["type"], json!("namespace"));
                assert_eq!(first_wire["tools"][0]["name"], json!("echidna"));
            }
            "codex" => {
                assert_eq!(
                    first_request
                        .headers
                        .get("chatgpt-account-id")
                        .map(String::as_str),
                    Some("workspace-opaque")
                );
                assert_eq!(
                    second_request
                        .headers
                        .get("chatgpt-account-id")
                        .map(String::as_str),
                    Some("workspace-opaque")
                );
                let search_files = first_wire["tools"]
                    .as_array()
                    .expect("Codex request advertises a tool array")
                    .iter()
                    .find(|tool| tool["name"] == json!("search_files"))
                    .expect("Codex request advertises search_files");
                assert_eq!(search_files["type"], json!("function"));
                assert!(search_files.get("namespace").is_none());
            }
            _ => unreachable!("the fixed profiles are exhaustive"),
        }
        assert!(second_wire.get("previous_response_id").is_none());
        let input = second_wire["input"].as_array().unwrap();
        assert_eq!(input.len(), 4);
        assert_eq!(input[0]["type"], json!("message"));
        assert_eq!(input[1]["type"], json!("reasoning"));
        assert_eq!(input[1]["id"], json!("rsn-sub"));
        assert_eq!(input[1]["encrypted_content"], json!("ciphertext-canary"));
        assert_eq!(input[1]["summary"], summary);
        assert_eq!(input[2]["type"], json!("function_call"));
        assert_eq!(input[2]["name"], json!("search_files"));
        match profile {
            "siwc" => assert_eq!(input[2]["namespace"], json!("echidna")),
            "codex" => assert!(input[2].get("namespace").is_none()),
            _ => unreachable!("the fixed profiles are exhaustive"),
        }
        assert_eq!(input[3]["type"], json!("function_call_output"));
        assert_eq!(input[3]["call_id"], input[2]["call_id"]);
    }
}

#[tokio::test]
async fn subscription_decoder_enforces_siwc_and_codex_namespaces_but_preserves_standard_calls() {
    for (case, namespace) in [("missing", None), ("foreign", Some("other"))] {
        let mut call = json!({
            "type": "function_call",
            "call_id": format!("call-{case}"),
            "name": "search_files",
            "arguments": "{\"pattern\":\"Cargo.toml\",\"paths\":[\".\"]}",
        });
        if let Some(namespace) = namespace {
            call["namespace"] = json!(namespace);
        }
        let body = [
            json!({"type": "response.output_item.done", "output_index": 0, "item": call}),
            json!({"type": "response.completed", "response": {
                "id": format!("response-{case}"), "status": "completed", "output": [call],
            }}),
        ]
        .into_iter()
        .map(|event| format!("data: {event}\n\n"))
        .collect::<String>();
        let (base_url, request, server) = spawn_sse_server(200, &body).await;
        let provider =
            subscription_adapter(base_url, FakeCredentialSource::value(Some(FIXTURE_SECRET)));
        let result = ModelStreamingService::new(&provider)
            .consume(
                subscription_full(
                    fixture_input(ModelRequestRequirements::default()),
                    "subscription-namespace-scope",
                ),
                ModelStreamCancellation::new(),
            )
            .await;
        assert!(matches!(
            result,
            Err(ModelStreamingError::Provider(ModelProviderError::ProtocolFailure { message }))
                if message.contains("advertised `echidna` namespace")
        ));
        request.await.unwrap();
        server.await.unwrap();
    }

    let codex_call = json!({
        "type": "function_call",
        "call_id": "call-codex-foreign-namespace",
        "namespace": "foreign",
        "name": "search_files",
        "arguments": "{\"pattern\":\"Cargo.toml\",\"paths\":[\".\"]}",
    });
    let codex_body = [
        json!({"type": "response.output_item.done", "output_index": 0, "item": codex_call}),
        json!({"type": "response.completed", "response": {
            "id": "response-codex-foreign-namespace", "status": "completed", "output": [codex_call],
        }}),
    ]
    .into_iter()
    .map(|event| format!("data: {event}\n\n"))
    .collect::<String>();
    let (base_url, request, server) = spawn_sse_server(200, &codex_body).await;
    let provider = codex_subscription_adapter(
        base_url,
        AccountBoundCredentialSource::new(
            "codex_subscription:fixture",
            "codex-namespace-bearer",
            "workspace-namespace",
        ),
    );
    let result = ModelStreamingService::new(&provider)
        .consume(
            subscription_full(
                fixture_input(ModelRequestRequirements::default()),
                "codex-namespace-scope",
            ),
            ModelStreamCancellation::new(),
        )
        .await;
    assert!(matches!(
        result,
        Err(ModelStreamingError::Provider(ModelProviderError::ProtocolFailure { message }))
            if message.contains("Codex subscription function call included an unsupported namespace")
    ));
    request.await.unwrap();
    server.await.unwrap();

    let call = json!({
        "type": "function_call",
        "call_id": "call-standard-namespaced",
        "namespace": "foreign",
        "name": "search_files",
        "arguments": "{\"pattern\":\"Cargo.toml\",\"paths\":[\".\"]}",
    });
    let body = [
        json!({"type": "response.output_item.done", "output_index": 0, "item": call}),
        json!({"type": "response.completed", "response": {
            "id": "response-standard-namespaced", "status": "completed", "output": [call],
        }}),
    ]
    .into_iter()
    .map(|event| format!("data: {event}\n\n"))
    .collect::<String>();
    let (base_url, request, server) = spawn_sse_server(200, &body).await;
    let provider = adapter(base_url, FakeCredentialSource::value(Some(FIXTURE_SECRET)));
    assert!(matches!(
        ModelStreamingService::new(&provider)
            .consume(
                full(fixture_input(ModelRequestRequirements::default())),
                ModelStreamCancellation::new(),
            )
            .await,
        Ok(ModelStreamOutcome::Completed { events, .. })
            if matches!(events.as_slice(), [ModelEvent::ToolCall { call }]
                if call.name.as_str() == "search_files")
    ));
    request.await.unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn subscription_opaque_references_require_the_owning_adapter_and_scope() {
    let opaque = json!({
        "type": "reasoning",
        "id": "rsn-closure",
        "status": "completed",
        "content": [],
        "encrypted_content": "closure-ciphertext",
        "summary": [],
    });
    let completed = [
        json!({"type": "response.output_item.done", "output_index": 0, "item": opaque}),
        json!({"type": "response.completed", "response": {
            "id": "response-closure", "status": "completed", "output": [opaque],
        }}),
    ]
    .into_iter()
    .map(|event| format!("data: {event}\n\n"))
    .collect::<String>();
    let (base_url, request, server) = spawn_sse_server(200, &completed).await;
    let source = FakeCredentialSource::value(Some(FIXTURE_SECRET));
    let provider = subscription_adapter(base_url, source.clone());
    let outcome = ModelStreamingService::new(&provider)
        .consume(
            subscription_full(
                fixture_input(ModelRequestRequirements::default()),
                "subscription-closure-scope",
            ),
            ModelStreamCancellation::new(),
        )
        .await
        .unwrap();
    request.await.unwrap();
    server.await.unwrap();
    let reference = match outcome {
        ModelStreamOutcome::Completed { events, .. } => events
            .into_iter()
            .find_map(|event| match event {
                ModelEvent::OpaqueContinuation { reference } => Some(reference),
                _ => None,
            })
            .expect("completed subscription response exposes an opaque reference"),
        ModelStreamOutcome::Cancelled { .. } => panic!("fixture response must complete"),
    };
    assert_eq!(source.references(), vec!["chatgpt_subscription:fixture"]);

    let retained = ModelInputIR::current(
        "Keep opaque continuation bounded.",
        vec![ContentItem::OpaqueContinuation {
            reference: reference.clone(),
        }],
        ModelRequestRequirements::default(),
    );
    let fresh_source = FakeCredentialSource::value(Some(FIXTURE_SECRET));
    let fresh_adapter = subscription_adapter("http://127.0.0.1:1/v1", fresh_source.clone());
    assert!(matches!(
        fresh_adapter.stream(subscription_full(
            retained.clone(),
            "subscription-closure-scope",
        )),
        Err(ModelProviderError::RequestEncodingFailure { message })
            if message.contains("unavailable in this Adapter")
    ));
    assert!(fresh_source.references().is_empty());

    assert!(matches!(
        provider.stream(subscription_full(retained, "different-delivery-scope")),
        Err(ModelProviderError::RequestEncodingFailure { message })
            if message.contains("belongs to another delivery session")
    ));
    assert_eq!(source.references(), vec!["chatgpt_subscription:fixture"]);
}

#[tokio::test]
async fn incomplete_subscription_stream_does_not_commit_opaque_references() {
    let body = concat!(
        "data: {\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{",
        "\"type\":\"reasoning\",\"id\":\"rsn-incomplete\",\"status\":\"completed\",",
        "\"content\":[],\"encrypted_content\":\"incomplete-ciphertext\",\"summary\":[]}}\n\n",
    );
    let (base_url, request, server) = spawn_sse_server(200, body).await;
    let source = FakeCredentialSource::value(Some(FIXTURE_SECRET));
    let provider = subscription_adapter(base_url, source.clone());
    let mut stream = provider
        .stream(subscription_full(
            fixture_input(ModelRequestRequirements::default()),
            "subscription-incomplete-scope",
        ))
        .unwrap();
    let reference = match stream.next().await.unwrap().unwrap() {
        ModelStreamEvent::Event(ModelEvent::OpaqueContinuation { reference }) => reference,
        other => panic!("expected opaque continuation event, got {other:?}"),
    };
    assert!(matches!(
        stream.next().await,
        Some(Err(ModelProviderError::ProtocolFailure { message }))
            if message.contains("ended before `response.completed`")
    ));
    request.await.unwrap();
    server.await.unwrap();

    let retained = ModelInputIR::current(
        "Keep opaque continuation bounded.",
        vec![ContentItem::OpaqueContinuation { reference }],
        ModelRequestRequirements::default(),
    );
    assert!(matches!(
        provider.stream(subscription_full(retained, "subscription-incomplete-scope")),
        Err(ModelProviderError::RequestEncodingFailure { message })
            if message.contains("unavailable in this Adapter")
    ));
    assert_eq!(source.references(), vec!["chatgpt_subscription:fixture"]);
}

#[tokio::test]
async fn subscription_opaque_store_rejects_cumulative_payloads_without_commit_or_retry() {
    let encrypted_content = "x".repeat(3_400_000);
    let one_reasoning = json!({
        "type": "reasoning",
        "id": "rsn-cumulative-0",
        "status": "completed",
        "content": [],
        "encrypted_content": encrypted_content,
        "summary": [],
    });
    assert!(
        serde_json::to_vec(&one_reasoning).unwrap().len() < 4 * 1024 * 1024,
        "each staged reasoning item remains below the four-MiB item limit"
    );
    let body = (0..5)
        .map(|index| {
            let mut reasoning = one_reasoning.clone();
            reasoning["id"] = json!(format!("rsn-cumulative-{index}"));
            format!(
                "data: {}\n\n",
                json!({
                    "type": "response.output_item.done",
                    "output_index": index,
                    "item": reasoning,
                })
            )
        })
        .collect::<String>();
    let (base_url, request, server) = spawn_sse_server_chunked(200, &body, 64 * 1024).await;
    let credentials = FakeCredentialSource::value(Some(FIXTURE_SECRET));
    let provider = subscription_adapter(base_url, credentials.clone());
    let scope = "subscription-cumulative-scope";
    let mut stream = provider
        .stream(subscription_full(
            fixture_input(ModelRequestRequirements::default()),
            scope,
        ))
        .unwrap();

    let first_reference = match stream.next().await.unwrap().unwrap() {
        ModelStreamEvent::Event(ModelEvent::OpaqueContinuation { reference }) => reference,
        other => panic!("expected the first staged opaque continuation, got {other:?}"),
    };
    for _ in 1..4 {
        assert!(matches!(
            stream.next().await.unwrap().unwrap(),
            ModelStreamEvent::Event(ModelEvent::OpaqueContinuation { .. })
        ));
    }
    assert!(matches!(
        stream.next().await,
        Some(Err(ModelProviderError::ProtocolFailure { message }))
            if message.contains("bounded byte limit")
    ));
    request.await.unwrap();
    server.await.unwrap();

    let retained = ModelInputIR::current(
        "A failed sample cannot retain opaque delivery state.",
        vec![ContentItem::OpaqueContinuation {
            reference: first_reference,
        }],
        ModelRequestRequirements::default(),
    );
    assert!(matches!(
        provider.stream(subscription_full(retained, scope)),
        Err(ModelProviderError::RequestEncodingFailure { message })
            if message.contains("unavailable in this Adapter")
    ));
    assert_eq!(
        credentials.references(),
        vec!["chatgpt_subscription:fixture"],
        "the rejected sample cannot commit a reference or trigger a retry"
    );
}

#[tokio::test]
async fn subscription_failed_or_incomplete_opaque_tool_samples_are_atomic_and_not_retried() {
    for profile in ["siwc", "codex"] {
        for case in ["failed", "incomplete", "unknown"] {
            let expected = match (profile, case) {
                ("siwc", "failed") => {
                    "ChatGPT subscription Responses stream reported response.failed"
                }
                ("siwc", "incomplete") => {
                    "ChatGPT subscription Responses stream reported response.incomplete"
                }
                ("codex", "failed") => {
                    "Codex subscription Responses stream reported response.failed"
                }
                ("codex", "incomplete") => {
                    "Codex subscription Responses stream reported response.incomplete"
                }
                (_, "unknown") => "unsupported Responses event `response.future`",
                _ => unreachable!("the fixed profiles and cases are exhaustive"),
            };
            let metadata_canary = format!("subscription-atomic-metadata-canary-{profile}-{case}");
            let summary_text = format!("subscription-atomic-summary-canary-{profile}-{case}");
            let summary = json!([{"type": "summary_text", "text": summary_text}]);
            let code_secret = format!("subscription-terminal-code-secret-{profile}-{case}");
            let detail_secret = format!("subscription-terminal-detail-secret-{profile}-{case}");
            let large_secret = format!("subscription-terminal-large-secret-{profile}-{case}");
            let terminal = match case {
                "failed" => json!({"type": "response.failed", "response": {
                    "error": {
                        "code": code_secret,
                        "message": detail_secret,
                        "irrelevant": format!("{large_secret}{}", "x".repeat(64 * 1024)),
                    },
                }}),
                "incomplete" => json!({"type": "response.incomplete", "response": {
                    "incomplete_details": {
                        "reason": detail_secret,
                        "irrelevant": format!("{large_secret}{}", "x".repeat(64 * 1024)),
                    },
                }}),
                "unknown" => json!({"type": "response.future"}),
                _ => unreachable!("fixture case is fixed"),
            };
            let opaque = json!({
                "type": "reasoning",
                "id": format!("rsn-atomic-{profile}-{case}"),
                "status": "completed",
                "content": [],
                "encrypted_content": format!("atomic-ciphertext-{profile}-{case}"),
                "summary": summary,
            });
            let mut call = json!({
                "type": "function_call",
                "call_id": format!("call-atomic-{profile}-{case}"),
                "name": "write_file",
                "arguments": "{\"path\":\"must-not-exist.txt\",\"content\":\"must not execute\\n\",\"expected_hash\":null}",
            });
            if profile == "siwc" {
                call["namespace"] = json!("echidna");
            }
            let mut events = vec![
                json!({"type": "response.output_item.done", "output_index": 0, "item": opaque}),
                json!({"type": "response.output_item.done", "output_index": 1, "item": call}),
            ];
            if profile == "codex" {
                events.push(json!({
                    "type": "response.metadata",
                    "metadata": {"canary": metadata_canary},
                }));
            }
            events.push(terminal);
            let body = events
                .into_iter()
                .map(|event| format!("data: {event}\n\n"))
                .collect::<String>();
            let (base_url, request, server) = spawn_sse_server_chunked(200, &body, 64 * 1024).await;
            let siwc_credentials = FakeCredentialSource::value(Some(FIXTURE_SECRET));
            let codex_credentials = AccountBoundCredentialSource::new(
                "codex_subscription:fixture",
                "codex-atomic-bearer",
                "workspace-atomic",
            );
            let provider = match profile {
                "siwc" => subscription_adapter(base_url, siwc_credentials.clone()),
                "codex" => codex_subscription_adapter(base_url, codex_credentials.clone()),
                _ => unreachable!("the fixed profiles are exhaustive"),
            };
            let directory =
                support::TemporaryDirectory::new(&format!("subscription-atomic-{profile}-{case}"));
            let tools = ToolRegistry::for_project(directory.path());
            let mut agent = AgentLoop::new(&provider, &tools);

            let error = agent
                .run_turn(
                    "reject the staged subscription sample",
                    AgentCancellation::new(),
                )
                .await
                .unwrap_err();
            let visible_error = error.to_string();
            assert!(
                visible_error.contains(expected),
                "{profile}/{case}: {visible_error}"
            );
            for secret in [&code_secret, &detail_secret, &large_secret] {
                assert!(
                    !visible_error.contains(secret),
                    "{profile}/{case}: terminal provider values must not enter the error: {visible_error}"
                );
            }
            assert_eq!(
                agent.history().items(),
                &[ContentItem::user_text(
                    "reject the staged subscription sample"
                )],
                "{profile}/{case}: opaque reasoning and ToolCall must not commit to History"
            );
            assert!(
                !format!("{:?}", agent.history()).contains(&metadata_canary),
                "{profile}/{case}: metadata must not enter History"
            );
            assert!(
                !format!("{:?}", agent.history()).contains(&summary_text),
                "{profile}/{case}: summary presentation must not enter History"
            );
            assert!(
                !directory.path().join("must-not-exist.txt").exists(),
                "{profile}/{case}: the staged ToolCall must not execute"
            );
            match profile {
                "siwc" => assert_eq!(
                    siwc_credentials.references(),
                    vec!["chatgpt_subscription:fixture"],
                    "{profile}/{case}: terminal subscription failure must not retry"
                ),
                "codex" => assert_eq!(
                    codex_credentials.resolutions(),
                    1,
                    "{profile}/{case}: terminal subscription failure must not retry"
                ),
                _ => unreachable!("the fixed profiles are exhaustive"),
            }
            request.await.unwrap();
            server.await.unwrap();
        }
    }
}

#[tokio::test]
async fn subscription_http_quota_error_is_structured_safe_and_not_retried() {
    let body = r#"{"error":{"code":"insufficient_quota","message":"account-routing-secret"}}"#;
    let (base_url, request, server) = spawn_sse_server(429, body).await;
    let credentials = FakeCredentialSource::value(Some(FIXTURE_SECRET));
    let provider = subscription_adapter(base_url, credentials.clone());
    let result = ModelStreamingService::new(&provider)
        .consume(
            subscription_full(
                fixture_input(ModelRequestRequirements::default()),
                "subscription-quota-scope",
            ),
            ModelStreamCancellation::new(),
        )
        .await;
    assert_eq!(
        result,
        Err(ModelStreamingError::Provider(
            ModelProviderError::ProviderFailure {
                code: Some("insufficient_quota".to_owned()),
                message: "ChatGPT subscription Responses request failed with HTTP 429".to_owned(),
            }
        ))
    );
    assert!(!format!("{result:?}").contains("account-routing-secret"));
    assert_eq!(
        credentials.references(),
        vec!["chatgpt_subscription:fixture"]
    );
    request.await.unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn subscription_failed_event_keeps_only_the_allowlisted_quota_code() {
    let secret = "subscription-failed-event-secret";
    let body = format!(
        "data: {}\n\n",
        json!({"type": "response.failed", "response": {
            "error": {"code": "insufficient_quota", "message": secret},
        }})
    );
    let (base_url, request, server) = spawn_sse_server(200, &body).await;
    let credentials = FakeCredentialSource::value(Some(FIXTURE_SECRET));
    let provider = subscription_adapter(base_url, credentials.clone());
    let result = ModelStreamingService::new(&provider)
        .consume(
            subscription_full(
                fixture_input(ModelRequestRequirements::default()),
                "subscription-failed-quota-scope",
            ),
            ModelStreamCancellation::new(),
        )
        .await;
    assert_eq!(
        result,
        Err(ModelStreamingError::Provider(
            ModelProviderError::ProviderFailure {
                code: Some("insufficient_quota".to_owned()),
                message: "ChatGPT subscription Responses stream reported response.failed"
                    .to_owned(),
            }
        ))
    );
    assert!(!format!("{result:?}").contains(secret));
    assert_eq!(
        credentials.references(),
        vec!["chatgpt_subscription:fixture"]
    );
    request.await.unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn subscription_http_error_omits_unrecognized_remote_code() {
    let code_secret = "subscription-http-code-secret";
    let message_secret = "subscription-http-message-secret";
    let body = format!(r#"{{"error":{{"code":"{code_secret}","message":"{message_secret}"}}}}"#);
    let (base_url, request, server) = spawn_sse_server(403, &body).await;
    let credentials = FakeCredentialSource::value(Some(FIXTURE_SECRET));
    let provider = subscription_adapter(base_url, credentials.clone());
    let result = ModelStreamingService::new(&provider)
        .consume(
            subscription_full(
                fixture_input(ModelRequestRequirements::default()),
                "subscription-http-unrecognized-scope",
            ),
            ModelStreamCancellation::new(),
        )
        .await;
    assert_eq!(
        result,
        Err(ModelStreamingError::Provider(
            ModelProviderError::ProviderFailure {
                code: None,
                message: "ChatGPT subscription Responses request failed with HTTP 403: unrecognized error code".to_owned(),
            }
        ))
    );
    let visible = format!("{result:?}");
    assert!(!visible.contains(code_secret));
    assert!(!visible.contains(message_secret));
    assert_eq!(
        credentials.references(),
        vec!["chatgpt_subscription:fixture"]
    );
    request.await.unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn subscription_http_error_classifies_malformed_and_missing_codes() {
    for (case, body, suffix, secret) in [
        (
            "malformed",
            r#"{"error":"subscription-malformed-secret""#,
            "malformed error body",
            "subscription-malformed-secret",
        ),
        (
            "missing",
            r#"{"error":{"message":"subscription-missing-secret"}}"#,
            "missing error code",
            "subscription-missing-secret",
        ),
    ] {
        let (base_url, request, server) = spawn_sse_server(400, body).await;
        let credentials = FakeCredentialSource::value(Some(FIXTURE_SECRET));
        let provider = subscription_adapter(base_url, credentials.clone());
        let result = ModelStreamingService::new(&provider)
            .consume(
                subscription_full(
                    fixture_input(ModelRequestRequirements::default()),
                    &format!("subscription-http-{case}-scope"),
                ),
                ModelStreamCancellation::new(),
            )
            .await;
        assert_eq!(
            result,
            Err(ModelStreamingError::Provider(
                ModelProviderError::ProviderFailure {
                    code: None,
                    message: format!(
                        "ChatGPT subscription Responses request failed with HTTP 400: {suffix}"
                    ),
                }
            ))
        );
        assert!(!format!("{result:?}").contains(secret));
        assert_eq!(
            credentials.references(),
            vec!["chatgpt_subscription:fixture"],
            "{case}: the failure must not retry"
        );
        request.await.unwrap();
        server.await.unwrap();
    }
}

#[tokio::test]
async fn subscription_http_error_rejects_oversized_body_without_disclosure() {
    let secret = "subscription-http-oversized-secret";
    let body = format!(
        r#"{{"error":{{"code":"{secret}","message":"{}"}}}}"#,
        "x".repeat(64 * 1024)
    );
    let (base_url, request, server) = spawn_sse_server_chunked(502, &body, 64 * 1024).await;
    let credentials = FakeCredentialSource::value(Some(FIXTURE_SECRET));
    let provider = subscription_adapter(base_url, credentials.clone());
    let result = ModelStreamingService::new(&provider)
        .consume(
            subscription_full(
                fixture_input(ModelRequestRequirements::default()),
                "subscription-http-oversized-scope",
            ),
            ModelStreamCancellation::new(),
        )
        .await;
    assert_eq!(
        result,
        Err(ModelStreamingError::Provider(
            ModelProviderError::ProviderFailure {
                code: None,
                message: "ChatGPT subscription Responses request failed with HTTP 502: error body exceeds 65536 bytes".to_owned(),
            }
        ))
    );
    assert!(!format!("{result:?}").contains(secret));
    assert_eq!(
        credentials.references(),
        vec!["chatgpt_subscription:fixture"]
    );
    request.await.unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn subscription_http_error_classifies_truncated_body_as_unreadable() {
    let secret = "subscription-http-truncated-secret";
    let body = format!(r#"{{"error":{{"code":"{secret}"}}}}"#);
    let (base_url, request, server) =
        spawn_sse_server_chunked_with_declared_length(503, &body, 64 * 1024, body.len() + 1).await;
    let credentials = FakeCredentialSource::value(Some(FIXTURE_SECRET));
    let provider = subscription_adapter(base_url, credentials.clone());
    let result = ModelStreamingService::new(&provider)
        .consume(
            subscription_full(
                fixture_input(ModelRequestRequirements::default()),
                "subscription-http-truncated-scope",
            ),
            ModelStreamCancellation::new(),
        )
        .await;
    assert_eq!(
        result,
        Err(ModelStreamingError::Provider(
            ModelProviderError::ProviderFailure {
                code: None,
                message: "ChatGPT subscription Responses request failed with HTTP 503: error body unavailable".to_owned(),
            }
        ))
    );
    assert!(!format!("{result:?}").contains(secret));
    assert_eq!(
        credentials.references(),
        vec!["chatgpt_subscription:fixture"]
    );
    request.await.unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn explicit_resume_discards_opaque_delivery_state_before_the_next_subscription_request() {
    let opaque = json!({
        "type": "reasoning",
        "id": "rsn-before-resume",
        "status": "completed",
        "content": [],
        "encrypted_content": "resume-ciphertext",
        "summary": [],
    });
    let first = [
        json!({"type": "response.output_item.done", "output_index": 0, "item": opaque}),
        json!({"type": "response.output_text.delta", "delta": "opaque state committed"}),
        json!({"type": "response.completed", "response": {
            "id": "response-before-resume", "status": "completed",
        }}),
    ]
    .into_iter()
    .map(|event| format!("data: {event}\n\n"))
    .collect::<String>();
    let second = concat!(
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"fresh after resume\"}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"response-after-resume\",\"status\":\"completed\"}}\n\n",
    );
    let (base_url, mut requests, server) =
        spawn_sse_sequence_server(vec![first, second.to_owned()]).await;
    let provider =
        subscription_adapter(base_url, FakeCredentialSource::value(Some(FIXTURE_SECRET)));
    let tools = ToolRegistry::for_project(env!("CARGO_MANIFEST_DIR"));
    let mut agent = AgentLoop::new(&provider, &tools);
    agent
        .run_turn("create opaque delivery state", AgentCancellation::new())
        .await
        .unwrap();

    let stream = StreamId::new("session:subscription-resume");
    let resumed_primary = ProviderSessionRef {
        id: ProviderSessionId::new("primary-subscription-resumed"),
        role: ProviderSessionRole::Primary,
        generation: 2,
    };
    let session = SessionReducer
        .reduce([
            SessionEvent::Started {
                primary_provider_session: ProviderSessionRef {
                    id: ProviderSessionId::new("primary-subscription-initial"),
                    role: ProviderSessionRole::Primary,
                    generation: 1,
                },
            }
            .into_envelope(
                EventId::new("subscription-resume-started"),
                stream.clone(),
                EventSequence::new(1),
                OccurredAt::new("2026-10-12T00:00:00Z"),
            )
            .unwrap(),
            SessionEvent::Resumed {
                primary_provider_session: resumed_primary.clone(),
            }
            .into_envelope(
                EventId::new("subscription-resume-resumed"),
                stream,
                EventSequence::new(2),
                OccurredAt::new("2026-10-12T00:00:01Z"),
            )
            .unwrap(),
        ])
        .unwrap();
    agent
        .resume_from_session(
            &session,
            ProviderSession::new(
                resumed_primary.id,
                ProviderSessionRole::Primary,
                resumed_primary.generation,
            ),
            None,
        )
        .unwrap();
    assert!(agent.history().items().is_empty());

    let outcome = agent
        .run_turn("continue after explicit Resume", AgentCancellation::new())
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        AgentTurnOutcome::Completed { assistant_text, .. } if assistant_text == "fresh after resume"
    ));
    let first_request = requests.recv().await.unwrap();
    let resumed_request = requests.recv().await.unwrap();
    server.await.unwrap();
    assert!(first_request.body.contains("create opaque delivery state"));
    let resumed_wire: Value = serde_json::from_str(&resumed_request.body).unwrap();
    assert!(resumed_wire.get("previous_response_id").is_none());
    assert!(
        resumed_wire["input"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| { item.get("type").and_then(Value::as_str) != Some("reasoning") })
    );
    assert!(!resumed_request.body.contains("resume-ciphertext"));
}

#[tokio::test]
async fn pending_credentials_wait_until_first_poll_and_never_fallback() {
    let (base_url, mut request_receiver, server) = spawn_sse_server(
        200,
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"pending-credential\",\"status\":\"completed\"}}\n\n",
    )
    .await;
    let (source, release) = PendingCredentialSource::new();
    let provider = adapter(base_url, source.clone());
    let mut stream = provider
        .stream(full(fixture_input(ModelRequestRequirements::default())))
        .unwrap();
    assert_eq!(source.polls(), 0);
    let poll = tokio::spawn(async move { stream.next().await });
    timeout(Duration::from_secs(1), async {
        while source.polls() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("first stream poll must begin credential resolution");
    assert_eq!(source.polls(), 1);
    assert!(
        timeout(Duration::from_millis(50), &mut request_receiver)
            .await
            .is_err(),
        "the request must wait for the pending credential"
    );
    release.send(Ok(Some(FIXTURE_SECRET.to_owned()))).unwrap();
    let request = request_receiver.await.unwrap();
    assert_eq!(request.request_line, "POST /v1/responses HTTP/1.1");
    assert!(matches!(
        poll.await.unwrap(),
        Some(Ok(ModelStreamEvent::Completed(_)))
    ));
    server.await.unwrap();

    let (failed_source, failed_release) = PendingCredentialSource::new();
    let failed_provider = adapter("http://127.0.0.1:1/v1", failed_source.clone());
    let mut failed_stream = failed_provider
        .stream(full(fixture_input(ModelRequestRequirements::default())))
        .unwrap();
    let failed_poll = tokio::spawn(async move { failed_stream.next().await });
    timeout(Duration::from_secs(1), async {
        while failed_source.polls() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("failed fixture must begin credential resolution once");
    failed_release
        .send(Err(CredentialSourceError::ReadFailed {
            message: "pending fixture failed".to_owned(),
        }))
        .unwrap();
    assert_eq!(
        failed_poll.await.unwrap(),
        Some(Err(ModelProviderError::CredentialSourceFailure {
            credential_ref: FIXTURE_CREDENTIAL_REFERENCE.to_owned(),
            message: "credential source failed: pending fixture failed".to_owned(),
        }))
    );
    assert_eq!(failed_source.polls(), 1);

    let (subscription_source, _subscription_release) = PendingCredentialSource::new();
    let subscription_provider =
        subscription_adapter("http://127.0.0.1:1/v1", subscription_source.clone());
    assert_stream_error(
        subscription_provider.stream(ModelRequest::incremental(
            fixture_input(ModelRequestRequirements::default()),
            ProviderResponseId::new("unsupported-incremental"),
        )),
        ModelProviderError::Capability(ModelCapabilityError::IncrementalInputUnsupported),
    );
    assert_eq!(subscription_source.polls(), 0);
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
