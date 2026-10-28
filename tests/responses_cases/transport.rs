use super::*;

#[tokio::test]
async fn request_send_diagnostics_classify_incomplete_headers_and_invalid_http_without_disclosure()
{
    for profile in ["standard", "siwc", "codex"] {
        for (response, http_cause) in [
            (
                "HTTP/1.1 200 OK\r\nX-Private: HTTP_HEADER_CANARY\r\n",
                "incomplete_message",
            ),
            (
                "HTTP/1.1 200 OK\r\ninvalid header name: HTTP_HEADER_CANARY\r\n\r\nHTTP_BODY_CANARY",
                "parse",
            ),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base_url = format!(
                "http://{}/TRANSPORT_URL_CANARY",
                listener.local_addr().unwrap()
            );
            let (sender, request) = oneshot::channel();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let captured = read_http_request(&mut socket).await;
                sender.send(captured).unwrap();
                tokio::time::sleep(Duration::from_millis(20)).await;
                socket.write_all(response.as_bytes()).await.unwrap();
                socket.shutdown().await.unwrap();
            });
            let ordinary = FakeCredentialSource::value(Some(FIXTURE_SECRET));
            let provider = match profile {
                "standard" => adapter(&base_url, ordinary),
                "siwc" => subscription_adapter(&base_url, ordinary),
                "codex" => codex_subscription_adapter(
                    &base_url,
                    AccountBoundCredentialSource::new(
                        "codex_subscription:fixture",
                        "SEND_BEARER_CANARY",
                        "SEND_ACCOUNT_CANARY",
                    ),
                ),
                _ => unreachable!(),
            };
            let input = ModelInputIR::current(
                "Fixture instruction.",
                vec![ContentItem::user_text("请求正文 REQUEST_BODY_CANARY 🐾")],
                ModelRequestRequirements::default(),
            );
            let error = ModelStreamingService::new(&provider)
                .consume(full(input), ModelStreamCancellation::new())
                .await
                .unwrap_err();
            let message =
                match error {
                    ModelStreamingError::Provider(ModelProviderError::ConnectionFailure {
                        timeout: false,
                        io_kind,
                        message,
                    }) if http_cause == "incomplete_message" => {
                        assert_eq!(io_kind, "None");
                        message
                    }
                    ModelStreamingError::Provider(ModelProviderError::TransportFailure {
                        message,
                    }) if http_cause == "parse" => message,
                    error => panic!("unexpected send failure category: {error}"),
                };
            // The server received the whole body before the header failure.
            // send_or_headers deliberately does not claim the request was unsent.
            let captured = request.await.unwrap();
            server.await.unwrap();
            assert!(captured.request_line.contains("TRANSPORT_URL_CANARY"));
            assert!(captured.body.contains("请求正文 REQUEST_BODY_CANARY 🐾"));
            assert!(captured.body.len() > captured.body.chars().count());
            for expected in [
                "phase=send_or_headers",
                "reqwest_kind=request",
                "io_kind=None",
                &format!("http_cause={http_cause}"),
            ] {
                assert!(message.contains(expected), "{profile}: {message}");
            }
            let field = |name: &str| -> u128 {
                message
                    .split_once(&format!("{name}="))
                    .unwrap()
                    .1
                    .split([';', ')'])
                    .next()
                    .unwrap()
                    .parse()
                    .unwrap()
            };
            assert_eq!(field("request_body_bytes"), captured.body.len() as u128);
            assert!(field("elapsed_ms") >= 20, "{message}");
            assert!(message.len() < 350, "{message}");
            for excluded in [
                FIXTURE_SECRET,
                "SEND_BEARER_CANARY",
                "SEND_ACCOUNT_CANARY",
                "TRANSPORT_URL_CANARY",
                "HTTP_HEADER_CANARY",
                "HTTP_BODY_CANARY",
                "REQUEST_BODY_CANARY",
                &base_url,
            ] {
                assert!(!message.contains(excluded), "{message}");
            }
        }
    }
}

#[tokio::test]
async fn header_closure_retries_identical_continuation_without_reexecuting_completed_tools() {
    for profile in ["standard", "siwc", "codex"] {
        let directory = support::TemporaryDirectory::new("header-closure-retry");
        fs::write(directory.path().join("target.txt"), "read exactly once").unwrap();
        let mut call = json!({"type":"function_call", "call_id":"completed-read",
            "name":"read_file", "arguments":"{\"path\":\"target.txt\"}"});
        if profile == "siwc" {
            call["namespace"] = json!("echidna");
        }
        let reasoning = if profile == "standard" {
            json!({"type":"reasoning", "status":"completed",
                "content":[{"type":"reasoning_text","text":"Read the retained fixture."}]})
        } else {
            json!({"type":"reasoning", "id":"header-retry-reasoning", "status":"completed",
                "content":[], "summary":[], "encrypted_content":"retained-ciphertext"})
        };
        let first = [
            json!({"type":"response.output_item.done","output_index":0,"item":reasoning}),
            json!({"type":"response.output_item.done","output_index":1,"item":call}),
            json!({"type":"response.completed","response":{"id":"completed-read-response",
                "status":"completed","output":[reasoning,call]}}),
        ]
        .iter()
        .map(|event| format!("data: {event}\n\n"))
        .collect::<String>();
        let last = concat!(
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"recovered final answer\"}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"recovered-response\",\"status\":\"completed\"}}\n\n"
        );
        let sse = |body: &str| {
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
        };
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
        let responses = [
            sse(&first),
            "HTTP/1.1 200 OK\r\nX-Test: truncated\r\n".to_owned(),
            "HTTP/1.1 200 OK\r\nX-Test: truncated\r\n".to_owned(),
            sse(last),
        ];
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            for response in responses {
                let (mut socket, _) = listener.accept().await.unwrap();
                requests.push(read_http_request(&mut socket).await);
                socket.write_all(response.as_bytes()).await.unwrap();
                socket.shutdown().await.unwrap();
            }
            requests
        });
        let credential = FakeCredentialSource::value(Some(FIXTURE_SECRET));
        let account = AccountBoundCredentialSource::new(
            "codex_subscription:fixture",
            "header-retry-bearer",
            "header-retry-account",
        );
        let provider = match profile {
            "standard" => ResponsesHttpAdapter::new(
                ResponsesHttpConfig::new(
                    base_url,
                    "fixture-model",
                    FIXTURE_CREDENTIAL_REFERENCE,
                    subscription_capabilities(),
                ),
                credential.clone(),
            ),
            "siwc" => subscription_adapter(base_url, credential.clone()),
            "codex" => codex_subscription_adapter(base_url, account.clone()),
            _ => unreachable!(),
        };
        let tools = ToolRegistry::for_project(directory.path());
        let mut agent = AgentLoop::new(&provider, &tools);
        let context = RequestAuditRunContext::new(
            RequestAuditBinding::new(
                ProjectId::new("header-retry"),
                StreamId::new("session:header-retry"),
            )
            .unwrap(),
            RequestAdapterVersion::new("fixture-header-retry-v1").unwrap(),
        );
        let mut audits = Vec::new();
        let outcome = timeout(
            Duration::from_secs(10),
            agent.run_turn_audited(
                "read target.txt",
                AgentCancellation::new(),
                &context,
                &mut |audit| {
                    audits.push(audit);
                    Ok(())
                },
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(
            matches!(outcome, AgentTurnOutcome::Completed {
            assistant_text, model_requests: 4, executions,
        } if assistant_text == "recovered final answer" && executions.len() == 1),
            "{profile}"
        );
        assert_eq!(audits.len(), 4);
        assert_eq!(agent.history().ensure_all_tool_calls_paired(), Ok(()));
        assert_eq!(
            agent
                .history()
                .items()
                .iter()
                .filter(|item| matches!(item, ContentItem::ToolResult { .. }))
                .count(),
            1
        );
        let requests = server.await.unwrap();
        assert!(
            requests[1..]
                .windows(2)
                .all(|pair| pair[0].body == pair[1].body)
        );
        let continuation: Value = serde_json::from_str(&requests[1].body).unwrap();
        let input = continuation["input"].as_array().unwrap();
        assert_eq!(input[1]["type"], "reasoning");
        assert_eq!(input[2]["type"], "function_call");
        assert_eq!(input[3]["type"], "function_call_output");
        if profile == "codex" {
            assert_eq!(
                account.resolutions(),
                2,
                "one preparation per logical request"
            );
        } else {
            assert_eq!(credential.references().len(), 2);
        }
    }
}

#[cfg(unix)]
#[tokio::test]
async fn peer_connection_reset_before_headers_is_a_typed_connection_failure() {
    use std::os::fd::AsRawFd;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        read_http_request(&mut socket).await;
        let linger = libc::linger {
            l_onoff: 1,
            l_linger: 0,
        };
        // Closing with zero linger deliberately emits RST rather than the
        // ordinary FIN covered by the incomplete-header fixture above.
        // SAFETY: the live socket descriptor and correctly sized linger value
        // remain valid for the duration of this synchronous setsockopt call.
        assert_eq!(
            unsafe {
                libc::setsockopt(
                    socket.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_LINGER,
                    (&linger as *const libc::linger).cast(),
                    std::mem::size_of_val(&linger) as libc::socklen_t,
                )
            },
            0
        );
    });
    let provider = adapter(endpoint, FakeCredentialSource::value(Some(FIXTURE_SECRET)));
    let error = ModelStreamingService::new(&provider)
        .consume(
            full(fixture_input(Default::default())),
            ModelStreamCancellation::new(),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(&error, ModelStreamingError::Provider(
        ModelProviderError::ConnectionFailure { timeout: false, io_kind, message }
    ) if io_kind == "Some(ConnectionReset)" && message.contains("phase=send_or_headers")),
        "{error}"
    );
    assert!(!error.to_string().contains(FIXTURE_SECRET));
    server.await.unwrap();
}

#[tokio::test]
async fn incomplete_body_retries_then_preserves_atomic_model_sample_boundaries() {
    for profile in ["standard", "siwc", "codex"] {
        let mut call = json!({"type":"function_call", "call_id":"uncommitted-call",
            "name":"write_file", "arguments":"{\"path\":\"must-not-exist.txt\",\"content\":\"uncommitted\",\"expected_hash\":null}"});
        if profile == "siwc" {
            call["namespace"] = json!("echidna");
        }
        let mut events =
            vec![json!({"type":"response.output_text.delta", "delta":"uncommitted text"})];
        if profile != "standard" {
            events.push(json!({"type":"response.output_item.done", "output_index":0,
                "item":{"type":"reasoning", "id":"uncommitted-opaque", "status":"completed",
                    "content":[], "summary":[], "encrypted_content":"uncommitted-ciphertext"}}));
        }
        events.push(json!({"type":"response.output_item.done", "output_index":1, "item":call}));
        let body = events
            .iter()
            .map(|event| format!("data: {event}\n\n"))
            .collect::<String>();
        let (base_url, mut requests, server) =
            spawn_sse_sequence_server(vec![body.clone(), body.clone(), body]).await;
        let ordinary = FakeCredentialSource::value(Some(FIXTURE_SECRET));
        let account = AccountBoundCredentialSource::new(
            "codex_subscription:fixture",
            "fixture-bearer",
            "fixture-account",
        );
        let provider = match profile {
            "standard" => adapter(base_url, ordinary.clone()),
            "siwc" => subscription_adapter(base_url, ordinary.clone()),
            "codex" => codex_subscription_adapter(base_url, account.clone()),
            _ => unreachable!(),
        };
        let directory = support::TemporaryDirectory::new("truncated-response");
        let tools = ToolRegistry::for_project(directory.path());
        let mut agent = AgentLoop::new(&provider, &tools);
        let error = agent
            .run_turn("read once", AgentCancellation::new())
            .await
            .unwrap_err();
        assert!(
            matches!(
                &error,
                echidna::app::agent_loop::AgentLoopError::Model(ModelStreamingError::Provider(
                    ModelProviderError::ResponseInterrupted {
                        status: 200,
                        timeout: false,
                        io_kind,
                        ..
                    }
                )) if io_kind == "None"
            ),
            "{profile}: {error}"
        );
        assert_eq!(
            agent.history().items(),
            &[ContentItem::user_text("read once")]
        );
        assert!(!directory.path().join("must-not-exist.txt").exists());
        if profile == "codex" {
            assert_eq!(account.resolutions(), 1);
        } else {
            assert_eq!(ordinary.references().len(), 1);
        }
        let mut captured = Vec::new();
        for _ in 0..3 {
            captured.push(requests.recv().await.unwrap());
        }
        assert_eq!(captured[0].body, captured[1].body);
        assert_eq!(captured[1].body, captured[2].body);
        server.await.unwrap();
    }
}

#[tokio::test]
async fn invalid_sse_utf8_remains_a_protocol_failure() {
    let body = b"data: \xff\n\n";
    let (base_url, request, server) =
        spawn_sse_server_chunked_with_declared_length(200, body, 17, body.len()).await;
    let provider = adapter(base_url, FakeCredentialSource::value(Some(FIXTURE_SECRET)));
    let error = ModelStreamingService::new(&provider)
        .consume(
            full(fixture_input(ModelRequestRequirements::default())),
            ModelStreamCancellation::new(),
        )
        .await
        .unwrap_err();
    assert!(matches!(error,
        ModelStreamingError::Provider(ModelProviderError::ProtocolFailure { message })
            if message.contains("invalid SSE stream: UTF8 error:")));
    request.await.unwrap();
    server.await.unwrap();
}
