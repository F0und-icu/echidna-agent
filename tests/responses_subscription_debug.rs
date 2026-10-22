// This test is compiled as a private copy of the library so it can exercise
// the crate-private CLI diagnostic owner through ResponsesHttpAdapter.
include!("../src/lib.rs");

#[cfg(debug_assertions)]
mod debug_regression {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::process::Command;
    use std::sync::Arc;
    use std::thread;

    use futures_util::StreamExt;

    use crate::adapters::credentials::{CredentialSource, CredentialSourceError};
    use crate::adapters::responses::{ResponsesHttpAdapter, ResponsesHttpConfig};
    use crate::domain::model::{
        CapabilityMappingVersion, ContentItem, ModelCapabilities, ModelInputIR,
        ModelRequestRequirements,
    };
    use crate::ports::model::{ModelProvider, ModelProviderError, ModelRequest};

    const CHILD_ENV: &str = "ECHIDNA_SUBSCRIPTION_DEBUG_CAPTURE_CHILD";
    const BEARER_SECRET: &str = "subscription-debug-bearer-secret";
    const HTTP_SECRET: &str = "subscription-debug-http-secret";
    const SSE_SECRET: &str = "subscription-debug-sse-secret";
    const STANDARD_SECRET: &str = "standard-debug-error-marker";

    struct FixtureCredentialSource;

    impl CredentialSource for FixtureCredentialSource {
        fn read(&self, reference: &str) -> Result<Option<String>, CredentialSourceError> {
            assert_eq!(reference, "chatgpt_subscription:fixture");
            Ok(Some(BEARER_SECRET.to_owned()))
        }
    }

    #[test]
    fn subscription_failures_redact_actual_debug_stderr() {
        if std::env::var_os(CHILD_ENV).is_some() {
            run_child();
            return;
        }

        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "debug_regression::subscription_failures_redact_actual_debug_stderr",
                "--nocapture",
            ])
            .env(CHILD_ENV, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "debug child failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );

        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains("[debug 发送 primary"));
        assert!(stderr.contains("HTTP 错误"));
        assert!(stderr.contains("Provider 返回停止状态：响应未提交"));
        assert!(
            stderr.contains(STANDARD_SECRET),
            "standard-profile debug output unexpectedly stopped showing its error body:\n{stderr}"
        );
        for secret in [BEARER_SECRET, HTTP_SECRET, SSE_SECRET] {
            assert!(
                !stderr.contains(secret),
                "subscription debug stderr disclosed fixture secret `{secret}`:\n{stderr}"
            );
        }
    }

    fn run_child() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let (base_url, server) = spawn_failure_server();
            let provider = ResponsesHttpAdapter::new(
                ResponsesHttpConfig::new(
                    base_url.clone(),
                    "fixture-subscription-model",
                    "chatgpt_subscription:fixture",
                    subscription_capabilities(),
                )
                .with_chatgpt_subscription_profile(),
                Arc::new(FixtureCredentialSource),
            )
            .with_cli_output(Some("primary"), true);

            assert_subscription_failure(&provider).await;
            assert_subscription_failure(&provider).await;
            let standard_provider = ResponsesHttpAdapter::new(
                ResponsesHttpConfig::new(
                    base_url,
                    "fixture-standard-model",
                    "chatgpt_subscription:fixture",
                    subscription_capabilities(),
                ),
                Arc::new(FixtureCredentialSource),
            )
            .with_cli_output(Some("primary"), true);
            assert_standard_http_failure(&standard_provider).await;
            server.join().unwrap();
        });
    }

    async fn assert_subscription_failure(provider: &ResponsesHttpAdapter) {
        let input = ModelInputIR::current(
            "Fixture instruction.",
            vec![ContentItem::user_text("Fixture input.")],
            ModelRequestRequirements::default(),
        );
        let mut stream = provider.stream(ModelRequest::full(input)).unwrap();
        assert!(matches!(
            stream.next().await,
            Some(Err(ModelProviderError::ProviderFailure { .. }))
        ));
    }

    async fn assert_standard_http_failure(provider: &ResponsesHttpAdapter) {
        let input = ModelInputIR::current(
            "Fixture instruction.",
            vec![ContentItem::user_text("Fixture input.")],
            ModelRequestRequirements::default(),
        );
        let mut stream = provider.stream(ModelRequest::full(input)).unwrap();
        assert!(matches!(
            stream.next().await,
            Some(Err(ModelProviderError::HttpFailure { status: 401 }))
        ));
    }

    fn subscription_capabilities() -> ModelCapabilities {
        ModelCapabilities {
            text_input: true,
            image_input: true,
            streaming: true,
            tool_call: true,
            structured_output: true,
            incremental_input: false,
            usage_reporting: false,
            max_output_tokens: None,
            reasoning_effort: None,
            context_window: 128_000,
            capability_mapping_version: CapabilityMappingVersion::new(1),
        }
    }

    fn spawn_failure_server() -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let http_body =
            format!("{{\"error\":{{\"code\":\"unrecognized\",\"message\":\"{HTTP_SECRET}\"}}}}");
        let sse_body = format!(
            "data: {{\"type\":\"response.failed\",\"response\":{{\"error\":{{\"code\":\"unrecognized\",\"message\":\"{SSE_SECRET}\"}}}}}}\n\n"
        );
        let standard_body = format!(
            "{{\"error\":{{\"code\":\"standard_fixture\",\"message\":\"{STANDARD_SECRET}\"}}}}"
        );
        let server = thread::spawn(move || {
            respond_failure(
                &listener,
                "401 Unauthorized",
                "application/json",
                &http_body,
            );
            respond_failure(&listener, "200 OK", "text/event-stream", &sse_body);
            respond_failure(
                &listener,
                "401 Unauthorized",
                "application/json",
                &standard_body,
            );
        });
        (base_url, server)
    }

    fn respond_failure(listener: &TcpListener, status: &str, content_type: &str, body: &str) {
        let (mut stream, _) = listener.accept().unwrap();
        let request = read_headers(&mut stream);
        assert!(request.starts_with("POST /responses HTTP/1.1"));
        assert!(request.contains("authorization: Bearer subscription-debug-bearer-secret"));
        write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        stream.flush().unwrap();
    }

    fn read_headers(stream: &mut TcpStream) -> String {
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let mut bytes = Vec::new();
        let mut chunk = [0_u8; 1024];
        while !bytes.windows(4).any(|window| window == b"\r\n\r\n") {
            let count = stream.read(&mut chunk).unwrap();
            assert_ne!(count, 0);
            bytes.extend_from_slice(&chunk[..count]);
        }
        String::from_utf8(bytes).unwrap()
    }
}

mod release_regression {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::process::Command;
    use std::sync::{Arc, mpsc};
    use std::thread;

    use futures_util::StreamExt;
    use serde_json::json;

    use crate::adapters::credentials::{
        AccountBoundCredential, CredentialResolution, CredentialSource, CredentialSourceError,
    };
    use crate::adapters::responses::{ResponsesHttpAdapter, ResponsesHttpConfig};
    use crate::domain::model::{
        CapabilityMappingVersion, ContentItem, ModelCapabilities, ModelEvent, ModelInputIR,
        ModelRequestRequirements,
    };
    use crate::ports::model::{ModelProvider, ModelRequest, ModelStreamEvent};

    const CHILD_ENV: &str = "ECHIDNA_CODEX_SUMMARY_RELEASE_CAPTURE_CHILD";
    const BEARER_SECRET: &str = "codex-summary-bearer-secret";

    struct FixtureCredentialSource;

    impl CredentialSource for FixtureCredentialSource {
        fn read(&self, _: &str) -> Result<Option<String>, CredentialSourceError> {
            panic!("Codex uses the account-bound credential resolution path")
        }

        fn resolve(&self, reference: &str) -> CredentialResolution {
            assert_eq!(reference, "codex_subscription:fixture");
            CredentialResolution::PendingAccountBound(Box::pin(async {
                Ok(AccountBoundCredential::new(
                    BEARER_SECRET.to_owned(),
                    "workspace-summary-fixture".to_owned(),
                ))
            }))
        }
    }

    #[test]
    fn codex_summary_is_redacted_escaped_and_separated_in_release_stderr() {
        if let Ok(case) = std::env::var(CHILD_ENV) {
            run_child(&case);
            return;
        }

        for (case, summary, marker) in [
            ("completed", "first", "[推理摘要已完成]"),
            ("done", "done-only", "[推理摘要已完成]"),
            (
                "failed",
                "terminal-summary",
                "[推理摘要未提交：Provider 返回停止状态]",
            ),
            (
                "incomplete",
                "terminal-summary",
                "[推理摘要未提交：Provider 返回停止状态]",
            ),
            ("eof", "terminal-summary", "[推理摘要中断：响应未完成]"),
            ("drop", "terminal-summary", "[推理摘要中断：响应未完成]"),
        ] {
            let output = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "release_regression::codex_summary_is_redacted_escaped_and_separated_in_release_stderr",
                    "--nocapture",
                ])
                .env(CHILD_ENV, case)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{case} child failed:\n{}",
                String::from_utf8_lossy(&output.stderr)
            );

            let stdout = String::from_utf8(output.stdout).unwrap();
            let stderr = String::from_utf8(output.stderr).unwrap();
            assert!(stderr.contains("[推理摘要 primary #"), "{case}: {stderr}");
            assert!(stderr.contains(summary), "{case}: {stderr}");
            assert!(stderr.contains(marker), "{case}: {stderr}");
            assert!(!stderr.contains(BEARER_SECRET), "{case}: {stderr}");
            assert!(!stdout.contains("推理摘要"), "{case}: {stdout}");
            assert!(!stdout.contains(summary), "{case}: {stdout}");
            if case == "completed" {
                assert!(stderr.contains("first\nsecond"), "{stderr}");
                assert!(!stderr.contains("firstsecond"), "{stderr}");
                assert!(stderr.contains("\\u{1b}[31m [redacted]"), "{stderr}");
                assert_eq!(stderr.matches("first").count(), 1, "{stderr}");
                assert_eq!(stderr.matches("[redacted]").count(), 1, "{stderr}");
            }
        }
    }

    fn run_child(case: &str) {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let (base_url, server) = spawn_summary_server(case);
            let provider = ResponsesHttpAdapter::new(
                ResponsesHttpConfig::new(
                    base_url,
                    "fixture-codex-summary-model",
                    "codex_subscription:fixture",
                    capabilities(),
                )
                .with_codex_subscription_profile(),
                Arc::new(FixtureCredentialSource),
            )
            .with_cli_output(
                Some("primary"),
                #[cfg(debug_assertions)]
                false,
            );
            let input = ModelInputIR::current(
                "Fixture instruction.",
                vec![ContentItem::user_text("Fixture input.")],
                ModelRequestRequirements::default(),
            );
            let mut stream = provider.stream(ModelRequest::full(input)).unwrap();
            match case {
                "completed" | "done" => {
                    assert!(matches!(
                        stream.next().await,
                        Some(Ok(ModelStreamEvent::Completed(_)))
                    ));
                    assert!(stream.next().await.is_none());
                }
                "failed" | "incomplete" => assert!(matches!(stream.next().await, Some(Err(_)))),
                "eof" => assert!(matches!(stream.next().await, Some(Err(_)))),
                "drop" => {
                    assert!(matches!(
                        stream.next().await,
                        Some(Ok(ModelStreamEvent::Event(ModelEvent::TextDelta { text })))
                            if text == "drop trigger"
                    ));
                    drop(stream);
                }
                _ => unreachable!("release fixture cases are exhaustive"),
            }
            server.join().unwrap();
        });
    }

    #[test]
    fn codex_summary_support_false_omits_only_summary_from_the_reasoning_object() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let (base_url, request_receiver, server) = spawn_no_summary_wire_server();
            let provider = ResponsesHttpAdapter::new(
                ResponsesHttpConfig::new(
                    base_url,
                    "fixture-codex-no-summary-model",
                    "codex_subscription:fixture",
                    capabilities(),
                )
                .with_codex_subscription_profile()
                .with_codex_reasoning_summary_support(Some(false)),
                Arc::new(FixtureCredentialSource),
            );
            let input = ModelInputIR::current(
                "Fixture instruction.",
                vec![ContentItem::user_text("Fixture input.")],
                ModelRequestRequirements::default(),
            );
            let mut stream = provider.stream(ModelRequest::full(input)).unwrap();
            assert!(matches!(
                stream.next().await,
                Some(Ok(ModelStreamEvent::Completed(_)))
            ));
            assert!(stream.next().await.is_none());

            let request = request_receiver.recv().unwrap();
            server.join().unwrap();
            let (_, body) = request.split_once("\r\n\r\n").unwrap();
            let wire: serde_json::Value = serde_json::from_str(body).unwrap();
            assert_eq!(wire["reasoning"], json!({}));
            assert_eq!(wire["include"], json!(["reasoning.encrypted_content"]));
        });
    }

    #[test]
    fn codex_body_failure_diagnostics_are_typed_and_private() {
        if let Ok(case) = std::env::var(CHILD_ENV) {
            run_body_failure_child(&case);
            return;
        }
        for (case, expected) in [
            ("timeout", "timeout=true, io_kind=None"),
            ("chunk", "timeout=false, io_kind=Some(InvalidInput)"),
        ] {
            let output = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "release_regression::codex_body_failure_diagnostics_are_typed_and_private",
                    "--nocapture",
                ])
                .env(CHILD_ENV, case)
                .output()
                .unwrap();
            let stderr = String::from_utf8(output.stderr).unwrap();
            assert!(output.status.success(), "{case}: {stderr}");
            for field in [
                "response body read failed",
                "HTTP 200",
                "elapsed_ms=",
                expected,
            ] {
                assert!(stderr.contains(field), "{case}: {stderr}");
            }
            for secret in [
                BEARER_SECRET,
                "transport-url-canary",
                "transport-body-canary",
                "transport-frame-canary",
                "workspace-summary-fixture",
            ] {
                assert!(!stderr.contains(secret), "{case}: {stderr}");
            }
        }
    }

    fn run_body_failure_child(case: &str) {
        use std::time::Duration;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!(
            "http://{}/transport-url-canary?secret=transport-url-canary",
            listener.local_addr().unwrap()
        );
        let (release, wait) = mpsc::channel();
        let chunked = case == "chunk";
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let _request = read_request(&mut socket);
            let event = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"transport-body-canary\"}\n\n";
            if chunked {
                write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{event}\r\nZ-transport-frame-canary\r\n", event.len()).unwrap();
            } else {
                write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: 1000\r\n\r\n{event}").unwrap();
            }
            socket.flush().unwrap();
            let _ = wait.recv_timeout(Duration::from_secs(15));
        });
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let provider = ResponsesHttpAdapter::new(
                ResponsesHttpConfig::new(
                    base_url,
                    "fixture-model",
                    "codex_subscription:fixture",
                    capabilities(),
                )
                .with_codex_subscription_profile()
                .with_timeouts(2_000, 2_000),
                Arc::new(FixtureCredentialSource),
            )
            .with_cli_output(
                Some("primary"),
                #[cfg(debug_assertions)]
                true,
            );
            let input = ModelInputIR::current(
                "Fixture instruction.",
                vec![ContentItem::user_text("Fixture input.")],
                ModelRequestRequirements::default(),
            );
            let mut stream = provider.stream(ModelRequest::full(input)).unwrap();
            assert!(matches!(
                stream.next().await,
                Some(Ok(ModelStreamEvent::Event(ModelEvent::TextDelta { .. })))
            ));
            let error = stream.next().await.unwrap().unwrap_err();
            assert!(matches!(
                error,
                crate::ports::model::ModelProviderError::TransportFailure { .. }
            ));
            eprintln!("{error}");
            assert!(stream.next().await.is_none());
        });
        release.send(()).unwrap();
        server.join().unwrap();
    }

    fn capabilities() -> ModelCapabilities {
        ModelCapabilities {
            text_input: true,
            image_input: true,
            streaming: true,
            tool_call: true,
            structured_output: true,
            incremental_input: false,
            usage_reporting: false,
            max_output_tokens: None,
            reasoning_effort: None,
            context_window: 128_000,
            capability_mapping_version: CapabilityMappingVersion::new(1),
        }
    }

    fn spawn_summary_server(case: &str) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let item = json!({
            "type": "reasoning",
            "id": "release-summary-item",
            "status": "completed",
            "content": [],
            "summary": [
                {"type": "summary_text", "text": "first"},
                {"type": "summary_text", "text": format!("second \u{001b}[31m {BEARER_SECRET}")},
            ],
        });
        let terminal_summary = json!({
            "type": "response.reasoning_summary_text.delta",
            "output_index": 0,
            "item_id": "release-terminal-summary-item",
            "summary_index": 0,
            "delta": "terminal-summary",
        });
        let body = match case {
            "completed" => [
            json!({
                "type": "response.reasoning_summary_text.delta",
                "output_index": 0,
                "item_id": "release-summary-item",
                "summary_index": 0,
                "delta": "first",
            }),
            json!({
                "type": "response.reasoning_summary_text.delta",
                "output_index": 0,
                "item_id": "release-summary-item",
                "summary_index": 1,
                "delta": "",
            }),
            json!({
                "type": "response.reasoning_summary_text.delta",
                "output_index": 0,
                "item_id": "release-summary-item",
                "summary_index": 1,
                "delta": format!("second \u{001b}[31m {BEARER_SECRET}"),
            }),
            json!({
                "type": "response.reasoning_summary_text.done",
                "output_index": 0,
                "item_id": "release-summary-item",
                "summary_index": 0,
                "text": "first",
            }),
            json!({
                "type": "response.reasoning_summary_text.done",
                "output_index": 0,
                "item_id": "release-summary-item",
                "summary_index": 1,
                "text": format!("second \u{001b}[31m {BEARER_SECRET}"),
            }),
            json!({"type": "response.output_item.done", "output_index": 0, "item": item.clone()}),
            json!({
                "type": "response.completed",
                "response": {
                    "id": "release-summary-response",
                    "status": "completed",
                    "output": [item],
                },
            }),
        ]
        .into_iter()
        .map(|event| format!("data: {event}\n\n"))
        .collect::<String>(),
            "done" => [
                json!({
                    "type": "response.reasoning_summary_text.done",
                    "output_index": 0,
                    "item_id": "release-done-summary-item",
                    "summary_index": 0,
                    "text": "done-only",
                }),
                json!({
                    "type": "response.completed",
                    "response": {"id": "release-done-response", "status": "completed"},
                }),
            ]
            .into_iter()
            .map(|event| format!("data: {event}\n\n"))
            .collect::<String>(),
            "failed" => [
                terminal_summary.clone(),
                json!({"type": "response.failed", "response": {"error": {"code": "fixture", "message": "fixture"}}}),
            ]
            .into_iter()
            .map(|event| format!("data: {event}\n\n"))
            .collect::<String>(),
            "incomplete" => [
                terminal_summary.clone(),
                json!({"type": "response.incomplete", "response": {"incomplete_details": {"reason": "fixture"}}}),
            ]
            .into_iter()
            .map(|event| format!("data: {event}\n\n"))
            .collect::<String>(),
            "eof" => [terminal_summary.clone()]
                .into_iter()
                .map(|event| format!("data: {event}\n\n"))
                .collect::<String>(),
            "drop" => [
                terminal_summary,
                json!({"type": "response.output_text.delta", "delta": "drop trigger"}),
            ]
            .into_iter()
            .map(|event| format!("data: {event}\n\n"))
            .collect::<String>(),
            _ => unreachable!("release fixture cases are exhaustive"),
        };
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_headers(&mut stream);
            assert!(request.starts_with("POST /responses HTTP/1.1"));
            assert!(request.contains("authorization: Bearer codex-summary-bearer-secret"));
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
            stream.flush().unwrap();
        });
        (base_url, server)
    }

    fn spawn_no_summary_wire_server() -> (String, mpsc::Receiver<String>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let (request_sender, request_receiver) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);
            assert!(request.starts_with("POST /responses HTTP/1.1"));
            request_sender.send(request).unwrap();
            let body = "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"codex-no-summary\",\"status\":\"completed\"}}\n\n";
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
            stream.flush().unwrap();
        });
        (base_url, request_receiver, server)
    }

    fn read_headers(stream: &mut TcpStream) -> String {
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let mut bytes = Vec::new();
        let mut chunk = [0_u8; 1024];
        while !bytes.windows(4).any(|window| window == b"\r\n\r\n") {
            let count = stream.read(&mut chunk).unwrap();
            assert_ne!(count, 0);
            bytes.extend_from_slice(&chunk[..count]);
        }
        String::from_utf8(bytes).unwrap()
    }

    fn read_request(stream: &mut TcpStream) -> String {
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let mut bytes = Vec::new();
        let mut chunk = [0_u8; 1024];
        let header_end = loop {
            if let Some(position) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                break position + 4;
            }
            let count = stream.read(&mut chunk).unwrap();
            assert_ne!(count, 0);
            bytes.extend_from_slice(&chunk[..count]);
        };
        let headers = std::str::from_utf8(&bytes[..header_end]).unwrap();
        let content_length = headers
            .lines()
            .find_map(|line| line.strip_prefix("content-length: "))
            .unwrap()
            .parse::<usize>()
            .unwrap();
        while bytes.len() < header_end + content_length {
            let count = stream.read(&mut chunk).unwrap();
            assert_ne!(count, 0);
            bytes.extend_from_slice(&chunk[..count]);
        }
        String::from_utf8(bytes).unwrap()
    }
}
