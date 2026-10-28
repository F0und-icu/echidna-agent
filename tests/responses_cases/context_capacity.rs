use super::*;

#[tokio::test]
async fn exact_capacity_codes_are_typed_across_error_and_failed_events() {
    for profile in ["standard", "siwc", "codex"] {
        for event in [
            json!({"type":"error","code":"context_length_exceeded","message":"fixture"}),
            json!({"type":"error","error":{"code":"context_length_exceeded","message":"fixture"}}),
            json!({"type":"response.failed","response":{"error":{"code":"context_length_exceeded","message":"fixture"}}}),
        ] {
            assert_eq!(
                error_event_fixture(profile, event).await,
                ModelProviderError::ContextCapacityExceeded {
                    code: "context_length_exceeded".to_owned()
                }
            );
        }
        for event in [
            json!({"type":"error","code":"CONTEXT_LENGTH_EXCEEDED","message":"fixture"}),
            json!({"type":"error","code":"invalid_request","message":"context_length_exceeded"}),
            json!({"type":"error","code":"context_length_exceeded","message":7}),
            json!({"type":"error","code":"context_length_exceeded","message":"x","param":7}),
            json!({"type":"response.failed","response":{"error":{"code":"context_length_exceeded","message":"x","param":7}}}),
            json!({"type":"response.failed","response":{"error":{"code":"context_length_exceeded","message":null}}}),
            json!({"type":"response.failed","response":{"code":"context_length_exceeded","message":"fixture"}}),
            json!({"type":"response.failed","response":{"error":null,"code":"context_length_exceeded","message":"fixture"}}),
            json!({"type":"response.failed","response":{"error":"context_length_exceeded"}}),
        ] {
            assert!(!matches!(
                error_event_fixture(profile, event).await,
                ModelProviderError::ContextCapacityExceeded { .. }
            ));
        }
    }
}

#[tokio::test]
async fn http_capacity_requires_a_valid_exact_code_and_keeps_other_failures_distinct() {
    for (body, capacity) in [
        (
            r#"{"error":{"code":"context_length_exceeded","message":"fixture"}}"#,
            true,
        ),
        (
            r#"{"error":{"code":"invalid_request","message":"context_length_exceeded"}}"#,
            false,
        ),
        (
            r#"{"error":{"code":"context_length_exceeded","message":7}}"#,
            false,
        ),
        (
            r#"{"error":{"code":"context_length_exceeded","message":"x","param":7}}"#,
            false,
        ),
        (
            r#"{"error":{"code":"context_length_exceeded","message":null}}"#,
            false,
        ),
        (
            r#"{"code":"context_length_exceeded","message":"fixture"}"#,
            false,
        ),
        (
            r#"{"error":null,"code":"context_length_exceeded","message":"fixture"}"#,
            false,
        ),
        (r#"{"error":"context_length_exceeded"}"#, false),
        (r#"{"error":{"code":[]}}"#, false),
        ("not json", false),
    ] {
        let (base_url, request, server) = spawn_sse_server(400, body).await;
        let provider = adapter(base_url, FakeCredentialSource::value(Some(FIXTURE_SECRET)));
        let error = ModelStreamingService::new(&provider)
            .consume(
                full(fixture_input(ModelRequestRequirements::default())),
                ModelStreamCancellation::new(),
            )
            .await
            .unwrap_err();
        assert_eq!(
            matches!(
                error,
                ModelStreamingError::Provider(ModelProviderError::ContextCapacityExceeded { .. })
            ),
            capacity
        );
        if !capacity {
            assert!(matches!(
                error,
                ModelStreamingError::Provider(ModelProviderError::HttpFailure { status: 400, .. })
            ));
        }
        request.await.unwrap();
        server.await.unwrap();
    }
}

#[tokio::test]
async fn unrelated_http_status_does_not_become_a_capacity_stop() {
    let (base_url, request, server) = spawn_sse_server(
        500,
        r#"{"error":{"code":"context_length_exceeded","message":"fixture"}}"#,
    )
    .await;
    let provider = adapter(base_url, FakeCredentialSource::value(Some(FIXTURE_SECRET)));
    let error = ModelStreamingService::new(&provider)
        .consume(
            full(fixture_input(ModelRequestRequirements::default())),
            ModelStreamCancellation::new(),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ModelStreamingError::Provider(ModelProviderError::HttpFailure { status: 500, .. })
    ));
    request.await.unwrap();
    server.await.unwrap();
}
