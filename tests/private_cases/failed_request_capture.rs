//! Explicit failed-attempt capture through the real Adapter, in every build.

mod captures {
    use std::fs;
    use std::path::Path;
    use std::sync::Arc;
    use std::time::Duration;

    use futures_util::StreamExt;
    use serde_json::{Value, json};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    use crate::adapters::credentials::{
        AccountBoundCredential, CredentialResolution, CredentialSource, CredentialSourceError,
    };
    use crate::adapters::responses::{ResponsesHttpAdapter, ResponsesHttpConfig};
    use crate::adapters::tools::ToolRegistry;
    use crate::domain::model::{
        CapabilityMappingVersion, ContentItem, ModelCapabilities, ModelInputIR,
    };
    use crate::ports::model::{ModelProvider, ModelProviderError, ModelRequest, ModelStreamEvent};
    use crate::ports::tool::ToolExecutor;
    use crate::support::TemporaryDirectory;

    const BEARER: &str = "fixture-bearer-\"\\secret";
    const ACCOUNT: &str = "fixture-account-\"\\secret";
    const COMPLETE: &str =
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"capture-complete\"}}\n\n";

    struct Credentials(&'static str);

    impl CredentialSource for Credentials {
        fn read(&self, _: &str) -> Result<Option<String>, CredentialSourceError> {
            Ok(Some(BEARER.to_owned()))
        }

        fn resolve(&self, reference: &str) -> CredentialResolution {
            match self.0 {
                "codex" => CredentialResolution::PendingAccountBound(Box::pin(async {
                    Ok(AccountBoundCredential::new(
                        BEARER.to_owned(),
                        ACCOUNT.to_owned(),
                    ))
                })),
                "missing" => CredentialResolution::Ready(Ok(None)),
                "pending-missing" => CredentialResolution::Pending(Box::pin(async { Ok(None) })),
                "pending" => CredentialResolution::Pending(Box::pin(std::future::pending())),
                _ => CredentialResolution::Ready(self.read(reference)),
            }
        }
    }

    fn adapter(
        base: &str,
        profile: &'static str,
        directory: Option<&Path>,
    ) -> ResponsesHttpAdapter {
        let config = ResponsesHttpConfig::new(
            base,
            "fixture",
            "fixture",
            ModelCapabilities {
                text_input: true,
                image_input: false,
                streaming: true,
                tool_call: true,
                structured_output: false,
                incremental_input: false,
                usage_reporting: false,
                max_output_tokens: None,
                reasoning_effort: None,
                context_window: 128_000,
                capability_mapping_version: CapabilityMappingVersion::new(1),
            },
        )
        .with_timeouts(1_000, 1_000);
        let config = match profile {
            "siwc" => config.with_chatgpt_subscription_profile(),
            "codex" => config.with_codex_subscription_profile(),
            _ => config,
        };
        ResponsesHttpAdapter::new(config, Arc::new(Credentials(profile)))
            .with_failure_capture(directory)
    }

    fn request(text: &str) -> ModelRequest {
        let tools = ToolRegistry::for_project(env!("CARGO_MANIFEST_DIR"))
            .specs()
            .into_iter()
            .filter(|tool| tool.name.as_str() == "read_file")
            .collect();
        ModelRequest::full(
            ModelInputIR::current(
                "Complete instruction fixture 🐾.",
                vec![ContentItem::user_text(text)],
                Default::default(),
            )
            .with_tool_specs(tools),
        )
    }

    fn response(status: &str, body: &str, extra_bytes: usize) -> Vec<u8> {
        format!("HTTP/1.1 {status}\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()+extra_bytes).into_bytes()
    }

    async fn server(responses: Vec<Vec<u8>>) -> (String, tokio::task::JoinHandle<Vec<Vec<u8>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let mut requests = Vec::new();
            for response in responses {
                let (mut socket, _) = listener.accept().await.unwrap();
                requests.push(read_request(&mut socket).await);
                socket.write_all(&response).await.unwrap();
                socket.shutdown().await.unwrap();
            }
            requests
        });
        (base, task)
    }

    async fn read_request(socket: &mut TcpStream) -> Vec<u8> {
        tokio::time::timeout(Duration::from_secs(5), async {
            let mut request = Vec::new();
            let mut buffer = [0; 4096];
            let end = loop {
                if let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                    break end + 4;
                }
                let n = socket.read(&mut buffer).await.unwrap();
                assert_ne!(n, 0);
                request.extend_from_slice(&buffer[..n]);
            };
            let headers = std::str::from_utf8(&request[..end]).unwrap();
            let length = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length: "))
                .unwrap()
                .parse::<usize>()
                .unwrap();
            while request.len() < end + length {
                let n = socket.read(&mut buffer).await.unwrap();
                assert_ne!(n, 0);
                request.extend_from_slice(&buffer[..n]);
            }
            request
        })
        .await
        .unwrap()
    }

    fn files(directory: &Path) -> Vec<(std::path::PathBuf, Value)> {
        let mut entries: Vec<_> = fs::read_dir(directory)
            .unwrap()
            .map(|entry| {
                let path = entry.unwrap().path();
                let value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
                (path, value)
            })
            .collect();
        entries.sort_by(|left, right| left.0.cmp(&right.0));
        entries
    }

    async fn failure(provider: &ResponsesHttpAdapter, text: &str) -> ModelProviderError {
        let mut stream = provider.stream(request(text)).unwrap();
        loop {
            match stream.next().await {
                Some(Err(error)) => {
                    assert!(stream.next().await.is_none());
                    return error;
                }
                Some(Ok(_)) => {}
                None => panic!("fixture must fail"),
            }
        }
    }

    #[tokio::test]
    async fn captures_send_http_body_sse_and_provider_errors_without_changing_categories() {
        for (kind, wire) in [
            (
                "send",
                b"HTTP/1.1 200 OK\r\nX-Test: response-secret\r\n".to_vec(),
            ),
            ("http", response("401 Unauthorized", "response-secret", 0)),
            ("body", response("200 OK", "", 1)),
            (
                "sse",
                response("200 OK", "data: {\"type\":\"unknown\"}\n\n", 0),
            ),
            (
                "provider",
                response(
                    "200 OK",
                    "data: {\"type\":\"error\",\"code\":\"server_error\",\"message\":\"bounded fixture error\"}\n\n",
                    0,
                ),
            ),
        ] {
            let root = TemporaryDirectory::new("capture-failure-kinds");
            let directory = root.path().join("new/private/captures");
            let (base, server) = server(vec![wire]).await;
            let provider = adapter(&base, "standard", Some(&directory));
            let error = failure(&provider, "Retain full input 🐾").await;
            assert!(
                matches!(
                    (&error, kind),
                    (ModelProviderError::ConnectionFailure { .. }, "send")
                        | (ModelProviderError::HttpFailure { status: 401 }, "http")
                        | (
                            ModelProviderError::ResponseInterrupted { status: 200, .. },
                            "body"
                        )
                        | (ModelProviderError::ProtocolFailure { .. }, "sse")
                        | (ModelProviderError::ProviderFailure { .. }, "provider")
                ),
                "{kind}: {error}"
            );
            let requests = server.await.unwrap();
            let body = &requests[0][requests[0]
                .windows(4)
                .position(|x| x == b"\r\n\r\n")
                .unwrap()
                + 4..];
            let captured = files(&directory);
            assert_eq!(captured.len(), 1);
            let snapshot = &captured[0].1;
            assert_eq!(
                snapshot["request"]["original_encoded_body_bytes"],
                body.len()
            );
            assert_eq!(
                snapshot["request"]["body"],
                serde_json::from_slice::<Value>(body).unwrap()
            );
            assert!(
                !snapshot["request"]["body"]["tools"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
            assert_eq!(snapshot["failure"], error.to_string());
            assert_eq!(
                snapshot["request"]["headers"],
                json!({"accept":"text/event-stream","content-type":"application/json"})
            );
            assert!(
                snapshot["notice"]
                    .as_str()
                    .unwrap()
                    .contains("not raw network bytes")
            );
            assert!(!snapshot.to_string().contains("response-secret"));
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(
                    fs::metadata(&captured[0].0).unwrap().permissions().mode() & 0o777,
                    0o600
                );
                for parent in [
                    &directory,
                    &directory.parent().unwrap().to_path_buf(),
                    &root.path().join("new"),
                ] {
                    assert_eq!(
                        fs::metadata(parent).unwrap().permissions().mode() & 0o777,
                        0o700
                    );
                }
            }
        }
    }

    #[tokio::test]
    async fn actual_subscription_secrets_nested_json_keys_opaque_and_images_are_removed() {
        for profile in ["standard", "siwc", "codex"] {
            let root = TemporaryDirectory::new("capture-privacy");
            let directory = root.path().join("captures");
            let wire = response("401 Unauthorized", "response-only-secret", 0);
            let (base, server) = server(vec![wire]).await;
            let expected_endpoint = format!("{base}/retained");
            let base = base.replacen("http://", "http://url-user:url-password@", 1)
                + "/retained?query=URL_QUERY_SECRET#URL_FRAGMENT_SECRET";
            let provider = adapter(&base, profile, Some(&directory));
            let account = if profile == "codex" { ACCOUNT } else { BEARER };
            let content=json!({
                BEARER:"key-one",account:"key-two","[redacted]":"ordinary-collision",
                "visible":"retain me","auth":{"Authorization":"AUTH_VALUE","COOKIE":"COOKIE_VALUE","api_key":"API_VALUE","access_token":"ACCESS_VALUE","refresh_token":"REFRESH_VALUE"},
                "ordinary_json":"{ \"spacing\" : [ 1, 2 ] }",
                "nested":json!({"encrypted_content":"OPAQUE_VALUE","image":"before data:image/png;base64,IMAGE_VALUE after","text":format!("{BEARER} {account}")}).to_string(),
            }).to_string();
            let _ = failure(&provider, &content).await;
            let requests = server.await.unwrap();
            let original = String::from_utf8(requests[0].clone()).unwrap();
            assert!(original.contains("IMAGE_VALUE"));
            assert!(original.contains("authorization: Bearer"));
            if profile == "codex" {
                assert!(original.contains("chatgpt-account-id:"));
            }
            let captured = files(&directory);
            let snapshot = &captured[0].1;
            assert_eq!(snapshot["request"]["endpoint"], expected_endpoint);
            let encoded = snapshot.to_string();
            for secret in [
                BEARER,
                account,
                "AUTH_VALUE",
                "COOKIE_VALUE",
                "API_VALUE",
                "ACCESS_VALUE",
                "REFRESH_VALUE",
                "OPAQUE_VALUE",
                "IMAGE_VALUE",
                "url-user",
                "url-password",
                "URL_QUERY_SECRET",
                "URL_FRAGMENT_SECRET",
                "response-only-secret",
            ] {
                assert!(
                    !encoded.contains(secret),
                    "{profile}: leaked {secret}: {encoded}"
                );
                let escaped = serde_json::to_string(secret).unwrap();
                assert!(
                    !encoded.contains(&escaped[1..escaped.len() - 1]),
                    "{profile}: encoded secret"
                );
            }
            assert!(encoded.contains("retain me"));
            assert!(encoded.contains("key-one") || profile != "codex");
            assert!(encoded.contains("key-two"));
            assert!(encoded.contains("ordinary-collision"));
            assert!(encoded.contains("redacted-key collision"));
            let retained: Value = serde_json::from_str(
                snapshot["request"]["body"]["input"][0]["content"][0]["text"]
                    .as_str()
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(retained["ordinary_json"], "{ \"spacing\" : [ 1, 2 ] }");
        }
    }

    #[tokio::test]
    async fn success_default_off_and_unobserved_or_cancelled_attempts_do_not_create_directories() {
        let root = TemporaryDirectory::new("capture-no-files");
        let directory = root.path().join("captures");
        let (base, server) = server(vec![
            response("200 OK", COMPLETE, 0),
            response("401 Unauthorized", "", 0),
            response(
                "200 OK",
                "data: {\"type\":\"response.output_text.delta\",\"delta\":\"partial\"}\n\n",
                0,
            ),
        ])
        .await;
        let provider = adapter(&base, "standard", Some(&directory));
        let mut success = provider.stream(request("success")).unwrap();
        assert!(matches!(
            success.next().await,
            Some(Ok(ModelStreamEvent::Completed(_)))
        ));
        drop(success);
        let _ = failure(&adapter(&base, "standard", None), "default off").await;
        drop(provider.stream(request("never polled")).unwrap());
        let mut partial = provider.stream(request("drop after output")).unwrap();
        assert!(matches!(
            partial.next().await,
            Some(Ok(ModelStreamEvent::Event(_)))
        ));
        drop(partial);
        assert_eq!(server.await.unwrap().len(), 3);
        assert!(!directory.exists());
        let mut capabilities = provider.capabilities();
        capabilities.image_input = true;
        let image_provider = ResponsesHttpAdapter::new(
            ResponsesHttpConfig::new(&base, "fixture", "fixture", capabilities),
            Arc::new(Credentials("standard")),
        )
        .with_failure_capture(Some(&directory));
        let image = ModelRequest::full(ModelInputIR::current(
            "fixture",
            vec![ContentItem::ImageArtifact {
                role: crate::domain::model::MessageRole::User,
                artifact_id: crate::domain::artifacts::ArtifactId::new("unresolved-image"),
            }],
            Default::default(),
        ));
        assert!(matches!(
            image_provider.stream(image),
            Err(ModelProviderError::RequestEncodingFailure { .. })
        ));
        assert!(!directory.exists());
        for profile in ["missing", "pending-missing", "pending"] {
            let provider = adapter(&base, profile, Some(&directory));
            match provider.stream(request("credential failure")) {
                Err(ModelProviderError::CredentialUnavailable { .. }) => {}
                Ok(mut stream) => {
                    let result =
                        tokio::time::timeout(Duration::from_millis(10), stream.next()).await;
                    if profile == "pending" {
                        assert!(result.is_err());
                    } else {
                        assert!(matches!(
                            result.unwrap(),
                            Some(Err(ModelProviderError::CredentialUnavailable { .. }))
                        ));
                    }
                }
                _ => panic!("wrong credential failure"),
            }
            assert!(!directory.exists());
        }
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let provider = adapter(
            &format!("http://{}", listener.local_addr().unwrap()),
            "standard",
            Some(&directory),
        );
        let mut stream = provider.stream(request("cancel during send")).unwrap();
        let poll = async {
            assert!(
                tokio::time::timeout(Duration::from_millis(30), stream.next())
                    .await
                    .is_err()
            );
        };
        let accept = async {
            let (mut socket, _) = listener.accept().await.unwrap();
            let _ = read_request(&mut socket).await;
            socket
        };
        let (_, socket) = tokio::join!(poll, accept);
        drop(stream);
        assert!(!directory.exists());
        drop(socket);
    }

    #[tokio::test]
    async fn each_failed_physical_attempt_has_a_unique_file_and_preserves_existing_files() {
        let root = TemporaryDirectory::new("capture-physical-attempts");
        let directory = root.path().join("captures");
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("sentinel"), b"unchanged").unwrap();
        let (base, server) = server(vec![
            response("200 OK", "", 1),
            response("200 OK", "", 1),
            response("200 OK", COMPLETE, 0),
        ])
        .await;
        let provider = adapter(&base, "standard", Some(&directory));
        let prepared = provider.prepare(request("same frozen packet")).unwrap();
        for _ in 0..2 {
            let mut stream = prepared.stream().unwrap();
            assert!(matches!(
                stream.next().await,
                Some(Err(ModelProviderError::ResponseInterrupted { .. }))
            ));
        }
        let mut stream = prepared.stream().unwrap();
        assert!(matches!(
            stream.next().await,
            Some(Ok(ModelStreamEvent::Completed(_)))
        ));
        let packets = server.await.unwrap();
        assert!(packets.windows(2).all(|pair| pair[0] == pair[1]));
        assert_eq!(fs::read(directory.join("sentinel")).unwrap(), b"unchanged");
        fs::remove_file(directory.join("sentinel")).unwrap();
        let captured = files(&directory);
        assert_eq!(captured.len(), 2);
        assert_ne!(captured[0].1["capture_id"], captured[1].1["capture_id"]);
        assert_eq!(captured[0].1["request"], captured[1].1["request"]);
    }

    #[tokio::test]
    async fn connection_and_capture_io_failures_preserve_the_original_provider_error() {
        let root = TemporaryDirectory::new("capture-io-failure");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let directory = root.path().join("connection");
        let error = failure(&adapter(&base, "standard", Some(&directory)), "connection").await;
        assert!(matches!(
            error,
            ModelProviderError::ConnectionFailure { .. }
        ));
        assert_eq!(files(&directory).len(), 1);
        let invalid = root.path().join("ordinary-file");
        fs::write(&invalid, b"untouched").unwrap();
        let (base, server) = server(vec![response("401 Unauthorized", "", 0)]).await;
        let error = failure(&adapter(&base, "standard", Some(&invalid)), "failure").await;
        assert_eq!(error, ModelProviderError::HttpFailure { status: 401 });
        server.await.unwrap();
        assert_eq!(fs::read(&invalid).unwrap(), b"untouched");
    }
}
