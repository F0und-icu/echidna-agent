//! Offline standard-model discovery and the real CLI selection path. Including
//! the library keeps the credential-injection boundary private in production.

include!("../src/lib.rs");

#[path = "support/mod.rs"]
mod support;

mod catalog_regression {
    use std::fs;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::path::Path;
    use std::process::{Command, Output};
    use std::sync::{Arc, Mutex, mpsc};
    use std::thread;
    use std::time::Duration;

    use serde_json::{Value, json};

    use crate::adapters::credentials::{CredentialSource, CredentialSourceError};
    use crate::provider_composition::{ConfiguredModelCatalog, ProviderComposition};
    use crate::support::TemporaryDirectory;

    const KEY: &str = "fixture-catalog-key-never-real";
    const REFERENCE: &str = "CYBER_AGENT_CATALOG_FIXTURE_API_KEY";

    struct Server {
        base: String,
        requests: mpsc::Receiver<String>,
        worker: thread::JoinHandle<()>,
    }

    impl Server {
        fn new(responses: Vec<String>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let base = format!("http://{}/custom/api/", listener.local_addr().unwrap());
            let (sender, requests) = mpsc::channel();
            let worker = thread::spawn(move || {
                for response in responses {
                    let deadline = std::time::Instant::now() + Duration::from_secs(10);
                    let mut stream = loop {
                        match listener.accept() {
                            Ok((stream, _)) => break stream,
                            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                                assert!(
                                    std::time::Instant::now() < deadline,
                                    "catalog request missing"
                                );
                                thread::sleep(Duration::from_millis(2));
                            }
                            Err(error) => panic!("{error}"),
                        }
                    };
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    let mut request = Vec::new();
                    let mut byte = [0];
                    while !request.ends_with(b"\r\n\r\n") {
                        assert!(request.len() < 16_384);
                        stream.read_exact(&mut byte).unwrap();
                        request.push(byte[0]);
                    }
                    sender.send(String::from_utf8(request).unwrap()).unwrap();
                    // A bounded-body rejection can close before this synthetic
                    // oversized response is completely written.
                    let _ = stream.write_all(response.as_bytes());
                }
            });
            Self {
                base,
                requests,
                worker,
            }
        }

        fn finish(self) -> Vec<String> {
            self.worker.join().unwrap();
            self.requests.try_iter().collect()
        }
    }

    fn http(status: u16, body: &str) -> String {
        format!(
            "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    fn data(models: Value) -> String {
        http(200, &json!({"object":"list", "data":models}).to_string())
    }

    fn config(base: &str, credential: &str) -> String {
        format!(
            r#"# Catalog fixture operator notes.
schema_version = 1
active_agent_profile = "test"
[runtime]
model_request_limit_per_turn = 0
auto_compact = true
[providers.custom]
binding = "http_sse"
wire_api = "responses"
base_url = "{base}"
model = "original-model" # keep the model comment
{credential}
capability_mapping = "shared"
generation = 1
connect_timeout_ms = 1000
read_timeout_ms = 1000
[providers.untouched]
binding = "http_sse"
wire_api = "responses"
base_url = "https://example.invalid/unselected"
model = "other-model"
credential_ref = "CYBER_AGENT_NEVER_RESOLVED_API_KEY"
capability_mapping = "shared"
generation = 7
[providers.inactive_subscription]
binding = "http_sse"
wire_api = "responses"
base_url = "https://chatgpt.com/backend-api/codex"
model = "old-codex-model"
codex_subscription = {{ account = "fixture", auth_store = "/tmp/echidna-catalog-auth-never-read" }}
capability_mapping = "subscription-only"
generation = 2
[capability_mappings.shared]
version = 4
text_input = true
image_input = false
streaming = true
tool_call = true
structured_output = false
incremental_input = false
usage_reporting = true
context_window = 32768
max_output_tokens = 8192
reasoning_effort = "high"
[capability_mappings.subscription-only]
version = 1
text_input = true
image_input = false
streaming = true
tool_call = true
structured_output = false
incremental_input = false
usage_reporting = true
context_window = 32768
[agent_profiles.test]
primary = "custom"
vision_mode = "disabled"
generation = 1
"#
        )
    }

    fn write_config(directory: &Path, base: &str, credential: &str) -> std::path::PathBuf {
        let path = directory.join("provider.toml");
        fs::write(&path, config(base, credential)).unwrap();
        path
    }

    fn cli(directory: &Path, config: &Path, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_echidna"))
            .arg("--config")
            .arg(config)
            .args(["auth"])
            .args(args)
            .current_dir(directory)
            .env(REFERENCE, KEY)
            .output()
            .unwrap()
    }

    fn assert_get(request: &str) {
        assert!(
            request.starts_with("GET /custom/api/models HTTP/1.1\r\n"),
            "{request}"
        );
        assert!(
            request
                .to_ascii_lowercase()
                .contains(&format!("authorization: bearer {KEY}"))
        );
        assert!(!request.to_ascii_lowercase().contains("chatgpt-account-id"));
        assert!(!request.contains("client_version="));
    }

    struct ExactCredentials(Mutex<Vec<String>>);
    impl CredentialSource for ExactCredentials {
        fn read(&self, reference: &str) -> Result<Option<String>, CredentialSourceError> {
            self.0.lock().unwrap().push(reference.to_owned());
            assert_eq!(reference, REFERENCE, "never resolve another Provider");
            Ok(Some(KEY.to_owned()))
        }
    }

    #[tokio::test]
    async fn configured_catalog_reuses_only_selected_inline_or_reference_credentials() {
        for credential in [
            format!("credential_ref = \"{REFERENCE}\""),
            format!("api_key = \"{KEY}\""),
        ] {
            let directory = TemporaryDirectory::new("api-catalog-injection");
            let server = Server::new(vec![data(json!([{"id":"future/org:arbitrary-8"}]))]);
            let path = write_config(directory.path(), &server.base, &credential);
            let credentials = Arc::new(ExactCredentials(Mutex::new(Vec::new())));
            let catalog =
                ProviderComposition::model_catalog(&path, "custom", credentials.clone()).unwrap();
            assert!(
                credentials.0.lock().unwrap().is_empty(),
                "construction is offline"
            );
            let ConfiguredModelCatalog::ApiKey(adapter) = catalog else {
                panic!("API-key route expected")
            };
            let models = adapter.models().await.unwrap();
            assert_eq!(models[0].slug, "future/org:arbitrary-8");
            assert_eq!(models[0].display_name, models[0].slug);
            assert!(!models[0].reasoning_levels_advertised);
            let reads = credentials.0.lock().unwrap();
            if credential.starts_with("credential_ref") {
                assert_eq!(&*reads, &[REFERENCE]);
            } else {
                assert!(reads.is_empty());
            }
            assert_get(&server.finish()[0]);
        }
    }

    #[test]
    fn actual_cli_models_lists_optional_metadata_without_inventing_capabilities() {
        let directory = TemporaryDirectory::new("api-catalog-cli-list");
        let server = Server::new(vec![data(json!([
            {"id":"id-only"},
            {"id":"arbitrary-vNext", "name":"Server name", "context_window":1048576,
             "input_modalities":["text","image","future"], "max_output_tokens":393216,
             "effort":{"supported_levels":["low","high","max","future-2"], "default_level":"high"},
             "api_capabilities":{"anthropic_messages":{"system_prompt_update":"in-history"}}}
        ]))]);
        let path = write_config(
            directory.path(),
            &server.base,
            &format!("api_key = \"{KEY}\""),
        );
        let before = fs::read(&path).unwrap();
        let output = cli(directory.path(), &path, &["models", "--provider", "custom"]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let models: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            models[0],
            json!({"slug":"id-only", "display_name":"id-only"})
        );
        assert_eq!(models[1]["context_window"], 1048576);
        assert_eq!(models[1]["display_name"], "Server name");
        assert_eq!(models[1]["default_reasoning_level"], "high");
        assert_eq!(
            models[1]["supported_reasoning_levels"][3]["effort"],
            "future-2"
        );
        assert!(models[1].get("max_output_tokens").is_none());
        assert!(
            models[1]
                .get("supports_reasoning_summary_parameter")
                .is_none()
        );
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_get(&server.finish()[0]);
    }

    #[test]
    fn actual_cli_selection_imports_only_advertised_metadata_and_preserves_other_state() {
        let directory = TemporaryDirectory::new("api-catalog-cli-select");
        let server = Server::new(vec![data(json!([{
            "id":"server-next-model", "name":"Next", "context_window":1048576,
            "input_modalities":["text","image"], "max_output_tokens":393216,
            "effort":{"supported_levels":["high","max","future-2"], "default_level":"high"},
            "supports_reasoning_summary_parameter":false,
            "api_capabilities":{"anthropic_messages":{"tool_update":"addition-only"}}
        }]))]);
        let path = write_config(
            directory.path(),
            &server.base,
            &format!("credential_ref = \"{REFERENCE}\""),
        );
        let before: toml::Value = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let output = cli(
            directory.path(),
            &path,
            &[
                "select-model",
                "--provider",
                "custom",
                "--model",
                "server-next-model",
                "--reasoning-effort",
                "future-2",
                "--apply-capabilities",
            ],
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# Catalog fixture operator notes."));
        assert!(text.contains("# keep the model comment"));
        let saved: toml::Value = toml::from_str(&text).unwrap();
        assert_eq!(saved["runtime"], before["runtime"]);
        for id in ["untouched", "inactive_subscription"] {
            assert_eq!(saved["providers"][id], before["providers"][id]);
        }
        assert_eq!(
            saved["capability_mappings"]["shared"],
            before["capability_mappings"]["shared"]
        );
        assert_eq!(
            saved["providers"]["custom"]["generation"].as_integer(),
            Some(2)
        );
        let selected = &saved["capability_mappings"]["custom-selection-2"];
        assert_eq!(selected["version"].as_integer(), Some(5));
        assert_eq!(selected["context_window"].as_integer(), Some(1048576));
        assert_eq!(selected["image_input"].as_bool(), Some(true));
        assert_eq!(selected["reasoning_effort"].as_str(), Some("future-2"));
        assert_eq!(selected["max_output_tokens"].as_integer(), Some(8192));
        assert_eq!(selected["incremental_input"].as_bool(), Some(false));
        assert!(
            saved["providers"]["custom"]
                .get("codex_subscription")
                .is_none()
        );
        assert!(!String::from_utf8_lossy(&output.stdout).contains(KEY));
        assert_get(&server.finish()[0]);
    }

    #[test]
    fn id_only_selection_accepts_explicit_effort_then_service_default_without_capability_changes() {
        let directory = TemporaryDirectory::new("api-catalog-manual-effort");
        let server = Server::new(vec![
            data(json!([{"id":"id-only"}])),
            data(json!([{"id":"id-only"}])),
        ]);
        let path = write_config(
            directory.path(),
            &server.base,
            &format!("api_key = \"{KEY}\""),
        );
        let output = cli(
            directory.path(),
            &path,
            &[
                "select-model",
                "--provider",
                "custom",
                "--model",
                "id-only",
                "--reasoning-effort",
                "next-effort",
            ],
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stderr).contains("not advertised"));
        let first: toml::Value = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let first_mapping = &first["capability_mappings"]["custom-selection-2"];
        assert_eq!(first_mapping["context_window"].as_integer(), Some(32768));
        assert_eq!(first_mapping["image_input"].as_bool(), Some(false));
        assert_eq!(
            first_mapping["reasoning_effort"].as_str(),
            Some("next-effort")
        );
        let output = cli(
            directory.path(),
            &path,
            &["select-model", "--provider", "custom", "--model", "id-only"],
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let second: toml::Value = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert!(
            second["capability_mappings"]["custom-selection-3"]
                .get("reasoning_effort")
                .is_none()
        );
        assert_eq!(
            second["capability_mappings"]["custom-selection-2"],
            *first_mapping
        );
        assert_eq!(
            second["capability_mappings"]["shared"]["reasoning_effort"].as_str(),
            Some("high")
        );
        for request in server.finish() {
            assert_get(&request);
        }
    }

    #[test]
    fn unsupported_selections_fail_without_publishing_configuration() {
        let catalog = data(json!([
            {"id":"id-only"},
            {"id":"declared", "effort":{"supported_levels":["high"]}}
        ]));
        for suffix in [
            vec!["--model", "missing"],
            vec!["--model", "id-only", "--apply-capabilities"],
            vec!["--model", "id-only", "--reasoning-effort", "bad effort"],
            vec!["--model", "declared", "--reasoning-effort", "max"],
            vec![],
        ] {
            let directory = TemporaryDirectory::new("api-catalog-invalid-selection");
            let server = Server::new(vec![catalog.clone()]);
            let path = write_config(
                directory.path(),
                &server.base,
                &format!("api_key = \"{KEY}\""),
            );
            let before = fs::read(&path).unwrap();
            let mut args = vec!["select-model", "--provider", "custom"];
            args.extend(suffix);
            let output = cli(directory.path(), &path, &args);
            assert!(!output.status.success(), "{args:?}");
            assert_eq!(fs::read(&path).unwrap(), before);
            assert!(!String::from_utf8_lossy(&output.stderr).contains(KEY));
            assert_get(&server.finish()[0]);
        }
    }

    #[test]
    fn malformed_duplicate_empty_and_oversized_catalogs_are_explicit_and_do_not_write() {
        let mut bodies = vec![
            "{}".to_owned(),
            "{invalid".to_owned(),
            "{\"data\":null}".to_owned(),
            json!({"data":[]}).to_string(),
            json!({"data":[{"id":"duplicate"},{"id":"duplicate"}]}).to_string(),
            json!({"data":vec![json!({"id":"same"});4097]}).to_string(),
            format!(
                "{{\"padding\":\"{}\",\"data\":[{{\"id\":\"one\"}}]}}",
                "x".repeat(1024 * 1024)
            ),
        ];
        for model in [
            json!({}),
            json!({"id":""}),
            json!({"id":"has space"}),
            json!({"id":"line\nbreak"}),
            json!({"id":"x".repeat(257)}),
            json!({"id":"one","name":""}),
            json!({"id":"one","context_window":0}),
            json!({"id":"one","context_window":-1}),
            json!({"id":"one","context_window":1.5}),
            json!({"id":"one","context_window":"1048576"}),
            json!({"id":"one","context_window":4294967296_u64}),
            json!({"id":"one","input_modalities":["text","text"]}),
            json!({"id":"one","input_modalities":[]}),
            json!({"id":"one","input_modalities":["bad\nvalue"]}),
            json!({"id":"one","effort":{}}),
            json!({"id":"one","effort":{"supported_levels":["high","high"]}}),
            json!({"id":"one","effort":{"supported_levels":["bad effort"]}}),
            json!({"id":"one","effort":{"supported_levels":["high"],"default_level":"max"}}),
        ] {
            bodies.push(json!({"data":[model]}).to_string());
        }
        for body in bodies {
            let directory = TemporaryDirectory::new("api-catalog-malformed");
            let server = Server::new(vec![http(200, &body)]);
            let path = write_config(
                directory.path(),
                &server.base,
                &format!("api_key = \"{KEY}\""),
            );
            let before = fs::read(&path).unwrap();
            let output = cli(
                directory.path(),
                &path,
                &["select-model", "--provider", "custom", "--model", "one"],
            );
            assert!(
                !output.status.success(),
                "unexpected success for {} bytes",
                body.len()
            );
            assert!(String::from_utf8_lossy(&output.stderr).contains("model catalog"));
            assert_eq!(fs::read(&path).unwrap(), before);
            assert_get(&server.finish()[0]);
        }
    }

    #[test]
    fn catalog_http_errors_are_status_only_and_redirects_never_forward_credentials() {
        let redirect_target = TcpListener::bind("127.0.0.1:0").unwrap();
        redirect_target.set_nonblocking(true).unwrap();
        for status in [401, 403, 429, 500, 302] {
            let directory = TemporaryDirectory::new("api-catalog-http-error");
            let body =
                format!("{{\"error\":{{\"code\":\"{KEY}\",\"message\":\"private remote body\"}}}}");
            let response = http(status, &body).replacen(
                "Content-Type:",
                &format!(
                    "Location: http://{}/capture\r\nContent-Type:",
                    redirect_target.local_addr().unwrap()
                ),
                1,
            );
            let server = Server::new(vec![response]);
            let path = write_config(
                directory.path(),
                &server.base,
                &format!("api_key = \"{KEY}\""),
            );
            let before = fs::read(&path).unwrap();
            let output = cli(
                directory.path(),
                &path,
                &["select-model", "--provider", "custom", "--model", "one"],
            );
            assert!(!output.status.success());
            let error = String::from_utf8_lossy(&output.stderr);
            assert!(error.contains(&format!("HTTP {status}")), "{error}");
            assert!(!error.contains(KEY));
            assert!(!error.contains("private remote body"));
            assert!(!error.contains("/capture"));
            assert_eq!(fs::read(&path).unwrap(), before);
            assert_get(&server.finish()[0]);
        }
        assert!(
            matches!(redirect_target.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
        );
    }

    #[test]
    fn api_key_account_operations_remain_unsupported_and_missing_reference_never_falls_back() {
        let directory = TemporaryDirectory::new("api-catalog-account-isolation");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let path = write_config(
            directory.path(),
            &format!("http://{}/v1", listener.local_addr().unwrap()),
            &format!("credential_ref = \"{REFERENCE}\""),
        );
        let before = fs::read(&path).unwrap();
        for operation in ["status", "refresh", "logout", "login"] {
            let output = cli(
                directory.path(),
                &path,
                &[operation, "--provider", "custom"],
            );
            assert!(!output.status.success());
            assert!(
                String::from_utf8_lossy(&output.stderr)
                    .contains("does not select a subscription credential")
            );
        }
        let output = Command::new(env!("CARGO_BIN_EXE_echidna"))
            .arg("--config")
            .arg(&path)
            .args(["auth", "models", "--provider", "custom"])
            .current_dir(directory.path())
            .env_remove(REFERENCE)
            .env("OPENAI_API_KEY", "must-not-fall-back")
            .env("DEEPSEEK_API_KEY", "must-not-fall-back")
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr)
                .contains("credential is unavailable or invalid")
        );
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(
            matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
        );
    }

    #[tokio::test]
    async fn body_limit_is_enforced_without_content_length() {
        let directory = TemporaryDirectory::new("api-catalog-chunk-limit");
        let chunk = "x".repeat(1024 * 1024 + 1);
        let response = format!(
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{chunk}\r\n0\r\n\r\n",
            chunk.len()
        );
        let server = Server::new(vec![response]);
        let path = write_config(
            directory.path(),
            &server.base,
            &format!("api_key = \"{KEY}\""),
        );
        let ConfiguredModelCatalog::ApiKey(adapter) = ProviderComposition::model_catalog(
            &path,
            "custom",
            Arc::new(ExactCredentials(Mutex::new(Vec::new()))),
        )
        .unwrap() else {
            panic!("API-key expected")
        };
        assert!(
            adapter
                .models()
                .await
                .unwrap_err()
                .to_string()
                .contains("exceeds")
        );
        assert_get(&server.finish()[0]);
    }

    #[tokio::test]
    async fn interrupted_catalog_body_and_connection_failure_remain_safe_visible_errors() {
        let directory = TemporaryDirectory::new("api-catalog-interrupted");
        let server = Server::new(vec![
            "HTTP/1.1 200 OK\r\nContent-Length: 1024\r\nConnection: close\r\n\r\n{\"data\":"
                .to_owned(),
        ]);
        let path = write_config(
            directory.path(),
            &server.base,
            &format!("api_key = \"{KEY}\""),
        );
        let ConfiguredModelCatalog::ApiKey(adapter) = ProviderComposition::model_catalog(
            &path,
            "custom",
            Arc::new(ExactCredentials(Mutex::new(Vec::new()))),
        )
        .unwrap() else {
            panic!("API-key expected")
        };
        let error = adapter.models().await.unwrap_err().to_string();
        assert!(error.contains("transport failed (body;"), "{error}");
        assert!(!error.contains(KEY));
        assert_get(&server.finish()[0]);
        // Closing a bound loopback port gives a deterministic pre-response
        // failure without contacting an external service or reading real keys.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}/v1", listener.local_addr().unwrap());
        drop(listener);
        let path = write_config(directory.path(), &base, &format!("api_key = \"{KEY}\""));
        let ConfiguredModelCatalog::ApiKey(adapter) = ProviderComposition::model_catalog(
            &path,
            "custom",
            Arc::new(ExactCredentials(Mutex::new(Vec::new()))),
        )
        .unwrap() else {
            panic!("API-key expected")
        };
        let error = adapter.models().await.unwrap_err().to_string();
        assert!(error.contains("transport failed (request;"), "{error}");
        assert!(!error.contains(&base));
        assert!(!error.contains(KEY));
    }

    #[tokio::test]
    async fn invalid_or_failed_credentials_do_not_send_or_expose_source_details() {
        struct SuppliedCredential(Result<Option<String>, CredentialSourceError>);
        impl CredentialSource for SuppliedCredential {
            fn read(&self, reference: &str) -> Result<Option<String>, CredentialSourceError> {
                assert_eq!(reference, REFERENCE);
                self.0.clone()
            }
        }
        let directory = TemporaryDirectory::new("api-catalog-credential-failure");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}/v1", listener.local_addr().unwrap());
        let path = write_config(
            directory.path(),
            &base,
            &format!("credential_ref = \"{REFERENCE}\""),
        );
        for supplied in [
            Ok(None),
            Ok(Some(" \t ".to_owned())),
            Ok(Some("invalid\nheader".to_owned())),
            Err(CredentialSourceError::ReadFailed {
                message: format!("private source details {KEY}"),
            }),
        ] {
            let ConfiguredModelCatalog::ApiKey(adapter) = ProviderComposition::model_catalog(
                &path,
                "custom",
                Arc::new(SuppliedCredential(supplied)),
            )
            .unwrap() else {
                panic!("API-key expected")
            };
            let error = adapter.models().await.unwrap_err().to_string();
            assert_eq!(error, "model catalog credential is unavailable or invalid");
            assert!(!error.contains(KEY));
        }
        assert!(
            matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
        );
    }
}
