use super::*;

#[tokio::test]
async fn truncated_body_is_transport_failure_without_committing_or_executing_the_sample() {
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
        let (base_url, request, server) =
            spawn_sse_server_chunked_with_declared_length(200, &body, 17, body.len() + 1).await;
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
            matches!(error,
            echidna::app::agent_loop::AgentLoopError::Model(
                ModelStreamingError::Provider(ModelProviderError::TransportFailure { ref message }))
                if message.contains("response body read failed before `response.completed`")),
            "{profile}: {error}"
        );
        let diagnostic = error.to_string();
        for expected in [
            "HTTP 200",
            "elapsed_ms=",
            "timeout=false",
            "io_kind=Some(UnexpectedEof)",
        ] {
            assert!(diagnostic.contains(expected), "{profile}: {diagnostic}");
        }
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
        request.await.unwrap();
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
