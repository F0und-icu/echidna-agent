mod support;

use std::fs;
use std::time::Duration;

use cyber_agent::adapters::project_event_store::Project;
use cyber_agent::adapters::tools::ToolRegistry;
use cyber_agent::domain::tools::{
    ExecutionId, ToolCachePolicy, ToolCall, ToolCallId, ToolDeterminism, ToolEffect, ToolExposure,
    ToolName, ToolParallelism, ToolResultStatus, ToolRetryPolicy,
};
use cyber_agent::ports::tool::{ToolExecutor, ToolExecutorError};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

use support::TemporaryDirectory;

fn initialized_project(label: &str) -> (TemporaryDirectory, Project) {
    let directory = TemporaryDirectory::new(label);
    let project = Project::initialize(directory.path()).unwrap();
    (directory, project)
}

async fn execute(
    tools: &ToolRegistry,
    call_id: &str,
    arguments: Value,
) -> cyber_agent::domain::tools::ToolExecutionOutcome {
    execute_named(tools, call_id, "http_request", arguments).await
}

async fn execute_named(
    tools: &ToolRegistry,
    call_id: &str,
    name: &str,
    arguments: Value,
) -> cyber_agent::domain::tools::ToolExecutionOutcome {
    let plan = tools
        .plan(
            ExecutionId::new(format!("execution-{call_id}")),
            ToolCall::new(ToolCallId::new(call_id), ToolName::new(name), arguments),
        )
        .unwrap();
    tools.execute(plan).await
}

async fn record_target_snapshot(tools: &ToolRegistry) {
    let plan = tools
        .plan(
            ExecutionId::new("execution-snapshot"),
            ToolCall::new(
                ToolCallId::new("call-snapshot"),
                ToolName::new("research_update"),
                json!({
                    "event_id": "event-snapshot",
                    "occurred_at": "2026-08-30T00:00:00Z",
                    "operation": {"kind": "record_target_snapshot", "record": {
                        "id": "snapshot-a", "target_id": "fixture.target", "kind": "web",
                        "locator": "http://fixture.invalid", "version": "1", "build_id": "1",
                        "target_artifact_ids": ["artifact-root"], "environment_fingerprint": "env",
                        "device_fingerprint": "device", "captured_at": "2026-08-30T00:00:00Z"
                    }}
                }),
            ),
        )
        .unwrap();
    assert_eq!(
        tools.execute(plan).await.result.status,
        ToolResultStatus::Succeeded
    );
}

fn request_args(url: String, event_id: &str, artifact_id: &str) -> Value {
    json!({
        "method": "get",
        "url": url,
        "timeout_ms": 500,
        "capture_bytes": 1024,
        "preview_bytes": 128,
        "artifact": {
            "event_id": event_id,
            "occurred_at": "2026-08-30T00:01:00Z",
            "artifact_id": artifact_id,
            "target_snapshot_id": "snapshot-a",
            "toolchain_fingerprint": "loopback-fixture-v1"
        }
    })
}

#[test]
fn http_request_contract_is_external_nondeterministic_noncacheable_and_deferred_by_default() {
    let tools = ToolRegistry::for_project(".");
    let spec = tools
        .catalog()
        .into_iter()
        .find(|spec| spec.name.as_str() == "http_request")
        .unwrap();
    assert_eq!(spec.exposure, ToolExposure::Deferred);
    assert_eq!(spec.parallelism, ToolParallelism::ResourceLocked);
    assert_eq!(spec.effect, ToolEffect::ExternalMutation);
    assert_eq!(spec.retry_policy, ToolRetryPolicy::NeverAutoRetry);
    assert_eq!(spec.determinism, ToolDeterminism::Nondeterministic);
    assert_eq!(spec.cache_policy, ToolCachePolicy::None);
    assert_eq!(
        spec.resource_key_template.as_deref(),
        Some("file:.cyber-agent/research.jsonl")
    );
}

async fn spawn_loopback(
    responses: Vec<Vec<u8>>,
    delay_before_response: Option<Duration>,
) -> (String, JoinHandle<Vec<Vec<u8>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for response in responses {
            let (mut stream, _) = listener.accept().await.unwrap();
            requests.push(read_request(&mut stream).await);
            if let Some(delay) = delay_before_response {
                tokio::time::sleep(delay).await;
            }
            let _ = stream.write_all(&response).await;
            let _ = stream.shutdown().await;
        }
        requests
    });
    (format!("http://{address}"), task)
}

async fn read_request(stream: &mut TcpStream) -> Vec<u8> {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 1024];
    loop {
        let read = stream.read(&mut buffer).await.unwrap();
        if read == 0 {
            return request;
        }
        request.extend_from_slice(&buffer[..read]);
        let Some(header_end) = header_end(&request) else {
            continue;
        };
        let content_length = String::from_utf8_lossy(&request[..header_end])
            .lines()
            .find_map(|line| line.strip_prefix("Content-Length: "))
            .and_then(|value| value.trim().parse::<usize>().ok())
            .unwrap_or(0);
        if request.len() >= header_end + 4 + content_length {
            return request;
        }
    }
}

fn header_end(request: &[u8]) -> Option<usize> {
    request.windows(4).position(|window| window == b"\r\n\r\n")
}

fn response(status: &str, headers: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
    let mut bytes = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\n", body.len()).into_bytes();
    for (name, value) in headers {
        bytes.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
    }
    bytes.extend_from_slice(b"Connection: close\r\n\r\n");
    bytes.extend_from_slice(body);
    bytes
}

#[test]
fn malformed_or_oversized_pure_bodies_are_rejected_before_execution() {
    let (_directory, project) = initialized_project("http-request-pre-execution");
    let tools = ToolRegistry::for_project(project.root());
    let mut malformed = request_args(
        "http://127.0.0.1:1/never-contacted".to_owned(),
        "event-invalid-base64",
        "artifact-invalid-base64",
    );
    malformed["body"] = json!({"kind": "base64", "data": "%%%"});
    let error = tools
        .plan(
            ExecutionId::new("execution-invalid-base64"),
            ToolCall::new(
                ToolCallId::new("call-invalid-base64"),
                ToolName::new("http_request"),
                malformed,
            ),
        )
        .unwrap_err();
    assert!(matches!(error, ToolExecutorError::InvalidArguments { .. }));

    let mut oversized = request_args(
        "http://127.0.0.1:1/never-contacted".to_owned(),
        "event-oversized-body",
        "artifact-oversized-body",
    );
    oversized["body"] = json!({"kind": "text", "value": "x".repeat(1_048_577)});
    let error = tools
        .plan(
            ExecutionId::new("execution-oversized-body"),
            ToolCall::new(
                ToolCallId::new("call-oversized-body"),
                ToolName::new("http_request"),
                oversized,
            ),
        )
        .unwrap_err();
    assert!(matches!(error, ToolExecutorError::InvalidArguments { .. }));
}

#[tokio::test]
async fn loopback_success_uses_structured_json_body_and_never_echoes_credentials() {
    let (_directory, project) = initialized_project("http-request-success");
    let tools = ToolRegistry::for_project(project.root());
    record_target_snapshot(&tools).await;
    let (base_url, server) = spawn_loopback(
        vec![response(
            "200 OK",
            &[("Content-Type", "text/plain; charset=utf-8")],
            b"hello",
        )],
        None,
    )
    .await;
    let mut arguments = request_args(
        format!("{base_url}/echo"),
        "event-success",
        "artifact-success",
    );
    arguments["method"] = json!("post");
    arguments["headers"] = json!({
        "Authorization": "Bearer never-echo-this",
        "Cookie": "session=also-never-echo-this"
    });
    arguments["query"] = json!([{"name": "term", "value": "one two"}]);
    arguments["body"] = json!({"kind": "json", "value": {"check": true}});

    let outcome = execute(&tools, "call-success", arguments).await;
    assert_eq!(outcome.result.status, ToolResultStatus::Succeeded);
    assert_eq!(outcome.result.output["status"], 200);
    assert_eq!(
        outcome.result.output["metadata"]["media_type"],
        "text/plain"
    );
    assert_eq!(outcome.result.output["body_preview"]["data"], "hello");
    assert_eq!(
        outcome.result.output["body_artifact"]["capture_complete"],
        true
    );
    assert_eq!(
        outcome.receipt.output_artifact_ids[0].as_str(),
        "artifact-success"
    );
    let rendered = serde_json::to_string(&outcome.result.output).unwrap();
    assert!(!rendered.contains("never-echo-this"));
    assert!(!rendered.contains("also-never-echo-this"));
    assert!(
        !serde_json::to_string(&outcome.receipt)
            .unwrap()
            .contains("never-echo-this")
    );

    let requests = server.await.unwrap();
    let request = String::from_utf8_lossy(&requests[0]);
    assert!(request.starts_with("POST /echo?term=one+two HTTP/1.1"));
    assert!(
        request.contains("authorization: Bearer never-echo-this")
            || request.contains("Authorization: Bearer never-echo-this")
    );
    assert!(
        request.contains("content-type: application/json")
            || request.contains("Content-Type: application/json")
    );
    assert!(request.ends_with("{\"check\":true}"));

    let blob_id = outcome.result.output["body_artifact"]["blob_id"]
        .as_str()
        .unwrap();
    assert_eq!(
        fs::read(
            project
                .root()
                .join(".cyber-agent/artifacts/blobs/sha256")
                .join(blob_id)
        )
        .unwrap(),
        b"hello"
    );
    let research = fs::read_to_string(project.root().join(".cyber-agent/research.jsonl")).unwrap();
    assert!(!research.contains("never-echo-this"));
    assert!(!research.contains("also-never-echo-this"));
}

#[tokio::test]
async fn artifact_reference_body_reads_the_reducer_resolved_blob_not_a_model_path() {
    let (_directory, project) = initialized_project("http-request-artifact-body");
    let tools = ToolRegistry::for_project(project.root());
    record_target_snapshot(&tools).await;
    let body_bytes = b"Artifact request body bytes";
    let source = project.root().join("request-body.bin");
    fs::write(&source, body_bytes).unwrap();
    let stored = execute_named(
        &tools,
        "call-input-artifact",
        "artifact_store",
        json!({
            "event_id": "event-input-artifact",
            "occurred_at": "2026-08-30T00:01:00Z",
            "artifact_id": "artifact-input-body",
            "source_path": source.display().to_string(),
            "media_type": "application/octet-stream",
            "target_snapshot_id": "snapshot-a",
            "toolchain_fingerprint": "loopback-fixture-v1"
        }),
    )
    .await;
    assert_eq!(stored.result.status, ToolResultStatus::Succeeded);

    let (base_url, server) = spawn_loopback(
        vec![response(
            "200 OK",
            &[("Content-Type", "text/plain")],
            b"accepted",
        )],
        None,
    )
    .await;
    let mut arguments = request_args(
        format!("{base_url}/artifact-body"),
        "event-artifact-body-response",
        "artifact-artifact-body-response",
    );
    arguments["method"] = json!("post");
    arguments["body"] = json!({"kind": "artifact_ref", "artifact_id": "artifact-input-body"});
    let outcome = execute(&tools, "call-artifact-body", arguments).await;

    assert_eq!(outcome.result.status, ToolResultStatus::Succeeded);
    let requests = server.await.unwrap();
    assert!(requests[0].ends_with(body_bytes));
}

#[tokio::test]
async fn oversized_artifact_reference_is_rejected_from_authoritative_size_before_network() {
    let (_directory, project) = initialized_project("http-request-artifact-body-limit");
    let tools = ToolRegistry::for_project(project.root());
    record_target_snapshot(&tools).await;
    let source = project.root().join("oversized-request-body.bin");
    fs::write(&source, vec![b'x'; 1_048_577]).unwrap();
    let stored = execute_named(
        &tools,
        "call-oversized-input-artifact",
        "artifact_store",
        json!({
            "event_id": "event-oversized-input-artifact",
            "occurred_at": "2026-08-30T00:01:00Z",
            "artifact_id": "artifact-oversized-input-body",
            "source_path": source.display().to_string(),
            "media_type": "application/octet-stream",
            "target_snapshot_id": "snapshot-a",
            "toolchain_fingerprint": "loopback-fixture-v1"
        }),
    )
    .await;
    assert_eq!(stored.result.status, ToolResultStatus::Succeeded);

    let mut arguments = request_args(
        "http://127.0.0.1:1/must-not-be-contacted".to_owned(),
        "event-oversized-artifact-body-response",
        "artifact-oversized-artifact-body-response",
    );
    arguments["method"] = json!("post");
    arguments["body"] =
        json!({"kind": "artifact_ref", "artifact_id": "artifact-oversized-input-body"});
    let outcome = execute(&tools, "call-oversized-artifact-body", arguments).await;

    assert_eq!(outcome.result.status, ToolResultStatus::Failed);
    assert_eq!(
        outcome.result.output["failure"]["code"],
        "artifact_blob_too_large"
    );
    assert_eq!(outcome.result.output["failure"]["stage"], "execution");
    assert!(outcome.receipt.output_artifact_ids.is_empty());
}

#[tokio::test]
async fn invalid_response_artifact_provenance_is_rejected_before_network() {
    let (_directory, project) = initialized_project("http-request-preflight");
    let tools = ToolRegistry::for_project(project.root());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        tokio::time::timeout(Duration::from_millis(150), listener.accept())
            .await
            .is_ok()
    });
    let mut arguments = request_args(
        format!("http://{address}/must-not-be-requested"),
        "event-invalid-provenance",
        "artifact-invalid-provenance",
    );
    arguments["artifact"]["target_snapshot_id"] = json!("missing-snapshot");

    let outcome = execute(&tools, "call-invalid-provenance", arguments).await;

    assert_eq!(outcome.result.status, ToolResultStatus::Failed);
    assert_eq!(
        outcome.result.output["failure"]["code"],
        "artifact_provenance_rejected"
    );
    assert_eq!(outcome.result.output["failure"]["stage"], "execution");
    assert!(!server.await.unwrap());
}

#[tokio::test]
async fn loopback_4xx_5xx_and_redirect_remain_distinct_completed_http_results() {
    let (_directory, project) = initialized_project("http-request-statuses");
    let tools = ToolRegistry::for_project(project.root());
    record_target_snapshot(&tools).await;
    let (base_url, server) = spawn_loopback(
        vec![
            response(
                "404 Not Found",
                &[("Content-Type", "text/plain")],
                b"missing",
            ),
            response(
                "503 Service Unavailable",
                &[("Content-Type", "text/plain")],
                b"retry later",
            ),
            response("302 Found", &[("Location", "/final")], b""),
            response("200 OK", &[("Content-Type", "text/plain")], b"redirected"),
        ],
        None,
    )
    .await;

    let not_found = execute(
        &tools,
        "call-404",
        request_args(format!("{base_url}/not-found"), "event-404", "artifact-404"),
    )
    .await;
    let unavailable = execute(
        &tools,
        "call-503",
        request_args(
            format!("{base_url}/unavailable"),
            "event-503",
            "artifact-503",
        ),
    )
    .await;
    let redirected = execute(
        &tools,
        "call-redirect",
        request_args(
            format!("{base_url}/redirect"),
            "event-redirect",
            "artifact-redirect",
        ),
    )
    .await;

    assert_eq!(not_found.result.status, ToolResultStatus::Succeeded);
    assert_eq!(not_found.result.output["status"], 404);
    assert_eq!(unavailable.result.status, ToolResultStatus::Succeeded);
    assert_eq!(unavailable.result.output["status"], 503);
    assert_eq!(redirected.result.status, ToolResultStatus::Succeeded);
    assert_eq!(redirected.result.output["status"], 200);
    assert_eq!(
        redirected.result.output["final_url"],
        format!("{base_url}/final")
    );
    assert_eq!(
        redirected.result.output["body_preview"]["data"],
        "redirected"
    );
    assert_eq!(server.await.unwrap().len(), 4);
}

#[tokio::test]
async fn redirect_target_is_rejected_before_a_request_with_url_credentials_is_sent() {
    let (_directory, project) = initialized_project("http-request-unsafe-redirect");
    let tools = ToolRegistry::for_project(project.root());
    record_target_snapshot(&tools).await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let _ = read_request(&mut stream).await;
        let location = format!("http://user:secret@{address}/must-not-be-requested");
        let response = response("302 Found", &[("Location", &location)], b"");
        stream.write_all(&response).await.unwrap();
        stream.shutdown().await.unwrap();
        tokio::time::timeout(Duration::from_millis(150), listener.accept())
            .await
            .is_ok()
    });

    let outcome = execute(
        &tools,
        "call-unsafe-redirect",
        request_args(
            format!("http://{address}/redirect"),
            "event-unsafe-redirect",
            "artifact-unsafe-redirect",
        ),
    )
    .await;

    assert_eq!(outcome.result.status, ToolResultStatus::Failed);
    assert_eq!(outcome.result.output["failure"]["stage"], "execution");
    assert!(!server.await.unwrap());
}

#[tokio::test]
async fn loopback_timeout_is_an_execution_failure_with_a_receipt() {
    let (_directory, project) = initialized_project("http-request-timeout");
    let tools = ToolRegistry::for_project(project.root());
    record_target_snapshot(&tools).await;
    let (base_url, server) = spawn_loopback(
        vec![response("200 OK", &[], b"too late")],
        Some(Duration::from_millis(150)),
    )
    .await;
    let mut arguments = request_args(
        format!("{base_url}/slow"),
        "event-timeout",
        "artifact-timeout",
    );
    arguments["timeout_ms"] = json!(25);
    let outcome = execute(&tools, "call-timeout", arguments).await;
    assert_eq!(outcome.result.status, ToolResultStatus::Failed);
    assert_eq!(
        outcome.result.output["failure"]["code"],
        "http_request_timeout"
    );
    assert_eq!(
        outcome.receipt.exit_reason.as_deref(),
        Some("http_request_timeout")
    );
    assert!(outcome.receipt.output_artifact_ids.is_empty());
    server.await.unwrap();
}

#[tokio::test]
async fn binary_and_oversize_loopback_bodies_publish_bounded_auditable_artifacts() {
    let (_directory, project) = initialized_project("http-request-binary-oversize");
    let tools = ToolRegistry::for_project(project.root());
    record_target_snapshot(&tools).await;
    let binary = [0_u8, 255, 1, 2];
    let oversize = b"abcdefghij";
    let (base_url, server) = spawn_loopback(
        vec![
            response(
                "200 OK",
                &[("Content-Type", "application/octet-stream")],
                &binary,
            ),
            response("200 OK", &[("Content-Type", "text/plain")], oversize),
        ],
        None,
    )
    .await;
    let binary_outcome = execute(
        &tools,
        "call-binary",
        request_args(
            format!("{base_url}/binary"),
            "event-binary",
            "artifact-binary",
        ),
    )
    .await;
    let mut oversize_arguments = request_args(
        format!("{base_url}/oversize"),
        "event-oversize",
        "artifact-oversize",
    );
    oversize_arguments["capture_bytes"] = json!(4);
    oversize_arguments["preview_bytes"] = json!(4);
    let oversize_outcome = execute(&tools, "call-oversize", oversize_arguments).await;

    assert_eq!(
        binary_outcome.result.output["body_preview"]["encoding"],
        "base64"
    );
    assert_eq!(
        binary_outcome.result.output["body_artifact"]["blob_id"],
        format!("{:x}", Sha256::digest(binary))
    );
    assert_eq!(
        oversize_outcome.result.output["metadata"]["body_truncated"],
        true
    );
    assert_eq!(
        oversize_outcome.result.output["body_artifact"]["capture_complete"],
        false
    );
    let blob_id = oversize_outcome.result.output["body_artifact"]["blob_id"]
        .as_str()
        .unwrap();
    assert_eq!(
        fs::read(
            project
                .root()
                .join(".cyber-agent/artifacts/blobs/sha256")
                .join(blob_id)
        )
        .unwrap(),
        b"abcd"
    );
    let research = fs::read_to_string(project.root().join(".cyber-agent/research.jsonl")).unwrap();
    assert!(research.contains("cyber-agent-captured-prefix=4"));
    server.await.unwrap();
}
