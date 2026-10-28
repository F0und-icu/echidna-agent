use super::*;

#[tokio::test]
async fn chunked_continuation_error_is_classified_only_when_complete_and_bounded() {
    for (body_bytes, expect_rejection) in [
        (70 * 1024, true),
        (1024 * 1024, true),
        (1024 * 1024 + 1, false),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let empty = json!({"error":{"code":"previous_response_not_found", "message":"fixture"}})
            .to_string();
        let body = json!({"error":{"code":"previous_response_not_found", "message":format!("fixture{}", "x".repeat(body_bytes - empty.len()))}}).to_string();
        assert_eq!(body.len(), body_bytes);
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            read_http_request(&mut socket).await;
            socket.write_all(b"HTTP/1.1 400 Bad Request\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n").await.unwrap();
            for chunk in body.as_bytes().chunks(8192) {
                let head = format!("{:x}\r\n", chunk.len());
                if socket.write_all(head.as_bytes()).await.is_err()
                    || socket.write_all(chunk).await.is_err()
                    || socket.write_all(b"\r\n").await.is_err()
                {
                    return;
                }
            }
            let _ = socket.write_all(b"0\r\n\r\n").await;
        });
        let provider = adapter(endpoint, FakeCredentialSource::value(Some(FIXTURE_SECRET)));
        let result = ModelStreamingService::new(&provider)
            .consume(
                ModelRequest::incremental(
                    fixture_input(Default::default()),
                    ProviderResponseId::new("prior"),
                ),
                ModelStreamCancellation::new(),
            )
            .await
            .unwrap_err();
        if expect_rejection {
            assert!(
                matches!(
                    result,
                    ModelStreamingError::Provider(ModelProviderError::ContinuationRejected { .. })
                ),
                "{result:?}"
            );
        } else {
            assert!(
                matches!(
                    result,
                    ModelStreamingError::Provider(ModelProviderError::HttpFailure {
                        status: 400,
                        ..
                    })
                ),
                "{result:?}"
            );
        }
        server.await.unwrap();
    }
}
