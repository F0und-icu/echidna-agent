//! Model-free Unix process integration tests for native configured stdio MCP.

#[cfg(unix)]
mod mcp_stdio_regression {
    use std::collections::BTreeMap;
    use std::fs;
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::sync::Arc;
    use std::time::Duration;

    use serde_json::{Value, json};

    use crate::adapters::credentials::{CredentialSource, CredentialSourceError};
    use crate::adapters::mcp::{CompletedCall, McpError, McpRuntime, McpServerConfig};
    use crate::adapters::tools::ToolRegistry;
    use crate::domain::tools::{
        ExecutionId, ToolCall, ToolCallId, ToolEffect, ToolExposure, ToolResultStatus,
    };
    use crate::ports::tool::{ToolExecutionError, ToolExecutor};
    use crate::provider_composition::ProviderComposition;
    use crate::support::TemporaryDirectory;

    struct NoCredentials;
    impl CredentialSource for NoCredentials {
        fn read(&self, _: &str) -> Result<Option<String>, CredentialSourceError> {
            panic!("stdio fixtures must not read provider/MCP credentials")
        }
    }

    fn fixture_path() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mcp_stdio/server.py")
    }

    fn server(directory: &TemporaryDirectory, mode: &str, timeout: u64) -> String {
        format!(
            "command = \"python3\"\nargs = [{}, {}, {}]\nprotocol_version = \"2025-11-25\"\ntimeout_ms = {timeout}\nstartup_timeout_ms = {}\n",
            json!(fixture_path().to_str().unwrap()),
            json!(mode),
            json!(directory.path().to_str().unwrap()),
            if mode == "startup_timeout" {
                timeout
            } else {
                3000
            }
        )
    }

    fn config_file(directory: &TemporaryDirectory, server: &str) -> PathBuf {
        let path = directory.path().join("provider.toml");
        fs::write(
            &path,
            format!(
                "{}\n[mcp_servers.local]\n{server}",
                crate::provider_config::fixture()
            ),
        )
        .unwrap();
        path
    }

    fn composition(
        directory: &TemporaryDirectory,
        mode: &str,
        timeout: u64,
    ) -> ProviderComposition {
        ProviderComposition::load(
            config_file(directory, &server(directory, mode, timeout)),
            Arc::new(NoCredentials),
        )
        .unwrap()
    }

    async fn discover_chat_tools(
        composition: &ProviderComposition,
        project_root: &Path,
    ) -> Result<ToolRegistry, McpError> {
        let discovered = composition.discover_chat_mcp().await?;
        ToolRegistry::for_project_with_mcp(project_root, &discovered)
    }

    fn runtime(directory: &TemporaryDirectory, mode: &str, timeout: u64) -> McpRuntime {
        let mut config: McpServerConfig =
            toml::from_str(&server(directory, mode, timeout)).unwrap();
        config.resolve_relative_to(directory.path());
        McpRuntime::new(
            BTreeMap::from([("local".to_owned(), config)]),
            Arc::new(NoCredentials),
        )
    }

    fn launch(directory: &TemporaryDirectory) -> Value {
        serde_json::from_slice(&fs::read(directory.path().join("launch.json")).unwrap()).unwrap()
    }

    // Failure diagnostics only: never print fixture environment, arguments or
    // RPC contents. Read process status only for IDs actually recorded by it.
    fn startup_diagnostics(directory: &TemporaryDirectory) -> Value {
        let (launch_status, launch) = match fs::read(directory.path().join("launch.json")) {
            Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
                Ok(launch) => ("present".to_owned(), Some(launch)),
                Err(_) => ("invalid JSON".to_owned(), None),
            },
            Err(error) => (format!("read {:?}", error.kind()), None),
        };
        let methods = match fs::read_to_string(directory.path().join("requests.jsonl")) {
            Ok(text) => text
                .lines()
                .map(|line| match serde_json::from_str::<Value>(line) {
                    Ok(value) => value
                        .get("method")
                        .and_then(Value::as_str)
                        .unwrap_or("<missing method>")
                        .to_owned(),
                    Err(_) => "<invalid JSON>".to_owned(),
                })
                .collect::<Vec<_>>(),
            Err(error) => vec![format!("<read {:?}>", error.kind())],
        };
        let mut processes = Vec::new();
        if let Some(launch) = &launch {
            for field in ["pid", "descendant"] {
                if let Some(pid) = launch
                    .get(field)
                    .and_then(Value::as_u64)
                    .filter(|pid| *pid > 0 && *pid <= i32::MAX as u64)
                {
                    let observed = match Command::new("ps")
                        .args([
                            "-o",
                            "pid=,ppid=,pgid=,stat=,wchan=",
                            "-p",
                            &pid.to_string(),
                        ])
                        .output()
                    {
                        Ok(output) => {
                            json!({"status": output.status.code(), "process": String::from_utf8_lossy(&output.stdout)})
                        }
                        Err(error) => json!({"read_error_kind":format!("{:?}", error.kind())}),
                    };
                    processes.push(json!({"role":field,"recorded_pid":pid,"observed":observed}));
                }
            }
        }
        json!({"launch_record":launch_status,"descendant_ready":launch.as_ref().and_then(|value| value.get("descendant_ready")).and_then(Value::as_bool),"request_methods":methods,"recorded_processes":processes})
    }

    fn requests(directory: &TemporaryDirectory) -> Vec<Value> {
        fs::read_to_string(directory.path().join("requests.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    async fn await_call(directory: &TemporaryDirectory) {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if fs::read_to_string(directory.path().join("requests.jsonl"))
                    .is_ok_and(|text| text.contains("tools/call"))
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("fixture observes dispatch within watchdog");
    }

    fn assert_reaped_and_group_stopped(directory: &TemporaryDirectory) {
        let launch = launch(directory);
        let pid = launch["pid"].as_i64().unwrap() as libc::pid_t;
        // Only after the production owner has shut down: ECHILD proves it reaped its direct child.
        let result = unsafe { libc::waitpid(pid, std::ptr::null_mut(), libc::WNOHANG) };
        assert_eq!(result, -1, "child must already be reaped by its owner");
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ECHILD)
        );
        let descendant = launch["descendant"].as_i64().unwrap().to_string();
        let until = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            let output = Command::new("ps")
                .args(["-o", "stat=", "-p", &descendant])
                .output()
                .unwrap();
            let status = String::from_utf8(output.stdout).unwrap();
            if status.trim().is_empty() || status.trim().starts_with('Z') {
                break;
            }
            if std::time::Instant::now() >= until {
                let detail = Command::new("ps")
                    .args(["-o", "pid=,ppid=,pgid=,stat=,wchan=", "-p", &descendant])
                    .output()
                    .unwrap();
                panic!(
                    "same-group descendant not stopped within 2s: fixture={} leader={pid} descendant={descendant} state={status:?} ps_status={} process={}",
                    directory.path().display(),
                    detail.status,
                    String::from_utf8_lossy(&detail.stdout),
                );
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[tokio::test]
    async fn stdio_catalog_preserves_schema_arguments_results_and_initialization_order() {
        let directory = TemporaryDirectory::new("stdio-paginate");
        let composition = composition(&directory, "paginate", 2000);
        assert!(
            !directory.path().join("launch.json").exists(),
            "composition is offline"
        );
        let tools = discover_chat_tools(&composition, directory.path())
            .await
            .unwrap();
        let imported: Vec<_> = tools
            .catalog()
            .into_iter()
            .filter(|spec| spec.name.as_str().starts_with("mcp__"))
            .collect();
        assert_eq!(imported.len(), 2);
        let spec = imported
            .iter()
            .find(|spec| spec.description.contains("remote.echo"))
            .unwrap();
        assert_eq!(spec.exposure, ToolExposure::Deferred);
        assert_eq!(spec.effect, ToolEffect::ExternalMutation);
        assert_eq!(
            spec.input_schema["properties"]["execution"]["$ref"],
            "#/$defs/Payload"
        );
        let arguments = json!({"execution":{"token":"remote-data"}});
        let outcome = tools
            .execute(
                tools
                    .plan(
                        ExecutionId::new("stdio-e"),
                        ToolCall::new(
                            ToolCallId::new("stdio-c"),
                            spec.name.clone(),
                            arguments.clone(),
                        ),
                    )
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(outcome.result.status, ToolResultStatus::Succeeded);
        assert_eq!(
            outcome.result.output["mcp_result"]["structuredContent"],
            arguments
        );
        assert_eq!(
            outcome.result.output["mcp_result"]["extension"]["preserved"],
            true
        );
        let seen = requests(&directory);
        assert_eq!(
            seen.iter()
                .map(|r| r["method"].as_str().unwrap())
                .collect::<Vec<_>>(),
            [
                "initialize",
                "notifications/initialized",
                "tools/list",
                "tools/list",
                "tools/call"
            ]
        );
        assert!(seen[1].get("id").is_none());
        assert_eq!(seen[3]["params"]["cursor"], "next");
        drop(composition);
        assert_reaped_and_group_stopped(&directory);
    }

    #[tokio::test]
    async fn stdio_fragment_ping_notifications_and_stderr_flood_remain_usable() {
        for mode in ["fragment", "ping", "ping_absent", "stderr"] {
            let directory = TemporaryDirectory::new(&format!("stdio-{mode}"));
            let runtime = runtime(&directory, mode, 3000);
            let tools = runtime.discover().await.unwrap();
            assert!(matches!(
                tools[0].call(json!({})).await.unwrap(),
                CompletedCall::Result(_)
            ));
            if matches!(mode, "ping" | "ping_absent") {
                assert!(
                    requests(&directory)
                        .iter()
                        .any(|r| r["id"] == "server-ping" && r["result"].is_object())
                );
            }
            drop(runtime);
            assert_reaped_and_group_stopped(&directory);
        }
    }

    #[tokio::test]
    async fn stdio_dispatched_failures_are_unknown_then_closed_without_replay() {
        for mode in [
            "eof",
            "wrong_id",
            "malformed",
            "no_newline",
            "frame_limit",
            "material_limit",
            "timeout",
            "unknown_request",
            "unknown_notification",
            "server_reply_limit",
            "bad_params_message_scalar",
            "bad_params_message_null",
            "bad_params_message_array",
            "bad_params_progress_scalar",
            "bad_params_progress_null",
            "bad_params_progress_array",
            "bad_params_ping_scalar",
            "bad_params_ping_null",
            "bad_params_ping_array",
        ] {
            let directory = TemporaryDirectory::new(&format!("stdio-unknown-{mode}"));
            let composition = composition(&directory, mode, 350);
            let tools = discover_chat_tools(&composition, directory.path())
                .await
                .unwrap_or_else(|error| {
                    panic!(
                        "{mode} startup failed: {error}; {}",
                        startup_diagnostics(&directory)
                    )
                });
            let name = tools
                .catalog()
                .into_iter()
                .find(|spec| spec.name.as_str().starts_with("mcp__"))
                .unwrap()
                .name;
            let plan = |id| {
                tools
                    .plan(
                        ExecutionId::new(format!("e-{id}")),
                        ToolCall::new(ToolCallId::new(format!("c-{id}")), name.clone(), json!({})),
                    )
                    .unwrap()
            };
            let error = tools.execute(plan(1)).await.expect_err(mode);
            assert!(
                matches!(error, ToolExecutionError::OutcomeUnknown { .. }),
                "{mode}: {error:?}"
            );
            assert_reaped_and_group_stopped(&directory);
            let second = tools.execute(plan(2)).await.unwrap();
            assert_eq!(second.result.status, ToolResultStatus::Failed);
            assert!(second.result.output.to_string().contains("no request sent"));
            assert_eq!(
                requests(&directory)
                    .iter()
                    .filter(|r| r["method"] == "tools/call")
                    .count(),
                1,
                "{mode} must not replay"
            );
        }
    }

    #[tokio::test]
    async fn stdio_matching_error_and_tool_error_are_known_completed_outcomes() {
        for mode in ["rpc_error", "tool_error"] {
            let directory = TemporaryDirectory::new(&format!("stdio-known-{mode}"));
            let composition = composition(&directory, mode, 2000);
            let tools = discover_chat_tools(&composition, directory.path())
                .await
                .unwrap();
            let name = tools
                .catalog()
                .into_iter()
                .find(|spec| spec.name.as_str().starts_with("mcp__"))
                .unwrap()
                .name;
            for id in 0..2 {
                let outcome = tools
                    .execute(
                        tools
                            .plan(
                                ExecutionId::new(format!("known-{id}")),
                                ToolCall::new(
                                    ToolCallId::new(format!("known-{id}")),
                                    name.clone(),
                                    json!({}),
                                ),
                            )
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                assert_eq!(outcome.result.status, ToolResultStatus::Failed);
                assert!(
                    outcome
                        .result
                        .output
                        .to_string()
                        .contains(if mode == "rpc_error" {
                            "retained"
                        } else {
                            "extension"
                        })
                );
            }
            drop(composition);
            assert_reaped_and_group_stopped(&directory);
        }
    }

    #[tokio::test]
    async fn stdio_cancellation_after_write_poisons_session_even_with_catalog_retained() {
        let directory = TemporaryDirectory::new("stdio-cancel");
        let runtime = runtime(&directory, "cancel", 2000);
        let tools = runtime.discover().await.unwrap();
        let mut call = Box::pin(tools[0].call(json!({})));
        tokio::select! {
            _ = &mut call => panic!("cancel fixture unexpectedly returned"),
            () = await_call(&directory) => {}
        }
        drop(call);
        assert_reaped_and_group_stopped(&directory);
        assert!(matches!(
            tools[0].call(json!({})).await,
            Err(McpError::PreExecution { .. })
        ));
        assert_eq!(
            requests(&directory)
                .iter()
                .filter(|r| r["method"] == "tools/call")
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn stdio_oversized_outgoing_call_is_preexecution_and_does_not_poison_session() {
        let directory = TemporaryDirectory::new("stdio-outgoing-limit");
        let runtime = runtime(&directory, "normal", 2000);
        let tools = runtime.discover().await.unwrap();
        let error = tools[0]
            .call(json!({"text":"x".repeat(2 * 1024 * 1024)}))
            .await;
        assert!(matches!(error, Err(McpError::PreExecution { .. })));
        assert!(matches!(
            tools[0].call(json!({})).await,
            Ok(CompletedCall::Result(_))
        ));
        assert_eq!(
            requests(&directory)
                .iter()
                .filter(|r| r["method"] == "tools/call")
                .count(),
            1
        );
        drop(runtime);
        assert_reaped_and_group_stopped(&directory);
    }

    #[tokio::test]
    async fn stdio_composition_drop_closes_retained_registry() {
        let directory = TemporaryDirectory::new("stdio-retained-registry");
        let composition = composition(&directory, "normal", 2000);
        let tools = discover_chat_tools(&composition, directory.path())
            .await
            .unwrap();
        drop(composition);
        assert_reaped_and_group_stopped(&directory);
        let name = tools
            .catalog()
            .into_iter()
            .find(|spec| spec.name.as_str().starts_with("mcp__"))
            .unwrap()
            .name;
        let outcome = tools
            .execute(
                tools
                    .plan(
                        ExecutionId::new("closed"),
                        ToolCall::new(ToolCallId::new("closed"), name, json!({})),
                    )
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(outcome.result.status, ToolResultStatus::Failed);
        assert!(
            outcome
                .result
                .output
                .to_string()
                .contains("no request sent")
        );
        assert!(
            !requests(&directory)
                .iter()
                .any(|r| r["method"] == "tools/call")
        );
    }

    #[test]
    fn stdio_cleanup_is_synchronous_after_tokio_runtime_has_ended() {
        let directory = TemporaryDirectory::new("stdio-ended-runtime");
        let runtime = runtime(&directory, "normal", 2000);
        let executor = tokio::runtime::Runtime::new().unwrap();
        let tools = executor.block_on(runtime.discover()).unwrap();
        drop(executor);
        drop(runtime);
        assert_reaped_and_group_stopped(&directory);
        drop(tools);
    }

    #[tokio::test]
    async fn stdio_exited_unreaped_leader_keeps_descendant_owned_until_shutdown() {
        let directory = TemporaryDirectory::new("stdio-exited-leader");
        let runtime = runtime(&directory, "leader_exit", 2000);
        let tools = runtime.discover().await.unwrap();
        tools[0].call(json!({})).await.unwrap();
        let pid = launch(&directory)["pid"].as_i64().unwrap().to_string();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let output = Command::new("ps")
                    .args(["-o", "stat=", "-p", &pid])
                    .output()
                    .unwrap();
                if String::from_utf8(output.stdout)
                    .unwrap()
                    .trim()
                    .starts_with('Z')
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("leader exits but stays unreaped before shutdown");
        drop(runtime);
        assert_reaped_and_group_stopped(&directory);
    }

    #[tokio::test]
    async fn stdio_startup_and_catalog_failures_cleanup_without_partial_catalog() {
        for mode in [
            "bad_initialize",
            "bad_catalog",
            "catalog_cycle",
            "catalog_error",
            "catalog_many",
            "startup_timeout",
        ] {
            let directory = TemporaryDirectory::new(&format!("stdio-startup-{mode}"));
            // The peer must launch before this fixture can verify descendant
            // cleanup. Use the existing 3s startup allowance under test load;
            // startup_timeout still stalls initialize for 60s and must be killed.
            let runtime = runtime(&directory, mode, 3000);
            assert!(runtime.discover().await.is_err(), "{mode}");
            assert_reaped_and_group_stopped(&directory);
        }
    }

    #[tokio::test]
    async fn stdio_launch_identity_changes_with_explicit_environment_without_exposure() {
        let directory = TemporaryDirectory::new("stdio-identity");
        let mut identities = Vec::new();
        for secret in ["PRIVATE_IDENTITY_A", "PRIVATE_IDENTITY_B"] {
            let mut config: McpServerConfig = toml::from_str(&format!(
                "{}\nenv = {{ MCP_EXPLICIT = {}}}\n",
                server(&directory, "normal", 2000),
                json!(secret)
            ))
            .unwrap();
            config.resolve_relative_to(directory.path());
            assert!(!format!("{config:?}").contains(secret));
            let runtime = McpRuntime::new(
                BTreeMap::from([("local".to_owned(), config)]),
                Arc::new(NoCredentials),
            );
            let tools = runtime.discover().await.unwrap();
            identities.push(tools[0].endpoint.clone());
            assert!(!tools[0].endpoint.contains(secret));
            drop(runtime);
            assert_reaped_and_group_stopped(&directory);
        }
        assert_ne!(identities[0], identities[1]);
    }

    #[test]
    fn stdio_config_rejects_incompatible_transports_and_invalid_boundary_values() {
        let directory = TemporaryDirectory::new("stdio-invalid-config");
        let valid = server(&directory, "normal", 2000);
        for (label, source) in [
            ("neither", "protocol_version = \"2025-11-25\"".to_owned()),
            ("both", format!("{valid}\nurl = \"http://localhost:1/mcp\"")),
            ("modern", valid.replace("2025-11-25", "2026-07-28")),
            (
                "credential",
                format!("{valid}\ncredential_ref = \"CYBER_AGENT_MCP_SECRET_API_KEY\""),
            ),
            ("empty command", valid.replace("\"python3\"", "\"\"")),
            ("empty cwd", format!("{valid}\ncwd = \"\"")),
            (
                "env key",
                format!("{valid}\nenv = {{ 'BAD=KEY' = 'value' }}"),
            ),
            (
                "nul arg",
                valid.replace("args = [", "args = [\"\\u0000\", "),
            ),
            (
                "http args",
                "url = \"http://localhost:1/mcp\"\nprotocol_version = \"2025-11-25\"\nargs = []"
                    .to_owned(),
            ),
        ] {
            assert!(
                ProviderComposition::load(
                    config_file(&directory, &source),
                    Arc::new(NoCredentials)
                )
                .is_err(),
                "{label}"
            );
        }
    }

    #[tokio::test]
    async fn stdio_relative_command_and_explicit_cwd_resolve_from_config_directory() {
        use std::os::unix::fs::PermissionsExt;
        let directory = TemporaryDirectory::new("stdio-explicit-cwd");
        fs::create_dir(directory.path().join("work")).unwrap();
        let command = directory.path().join("peer");
        fs::copy(fixture_path(), &command).unwrap();
        fs::set_permissions(&command, fs::Permissions::from_mode(0o700)).unwrap();
        let source = format!(
            "command = \"./peer\"\ncwd = \"work\"\nargs = [\"normal\", {}]\nprotocol_version = \"2025-11-25\"\n",
            json!(directory.path().to_str().unwrap())
        );
        let composition =
            ProviderComposition::load(config_file(&directory, &source), Arc::new(NoCredentials))
                .unwrap();
        let _tools = discover_chat_tools(&composition, directory.path())
            .await
            .unwrap();
        assert_eq!(
            Path::new(launch(&directory)["cwd"].as_str().unwrap())
                .canonicalize()
                .unwrap(),
            directory.path().join("work").canonicalize().unwrap()
        );
        drop(composition);
        assert_reaped_and_group_stopped(&directory);
    }

    #[tokio::test]
    async fn stdio_and_existing_http_share_catalog_without_changing_http_identity() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/mcp", listener.local_addr().unwrap());
        let peer = tokio::spawn(async move {
            for expected in ["tools/list", "tools/call"] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut received = Vec::new();
                let mut buffer = [0_u8; 1024];
                let request = loop {
                    let count = socket.read(&mut buffer).await.unwrap();
                    assert_ne!(count, 0);
                    received.extend_from_slice(&buffer[..count]);
                    if let Some(end) = received.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&received[..end]);
                        let length: usize = head
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(str::trim)
                                    .map(str::to_owned)
                            })
                            .unwrap()
                            .parse()
                            .unwrap();
                        if received.len() >= end + 4 + length {
                            break serde_json::from_slice::<Value>(
                                &received[end + 4..end + 4 + length],
                            )
                            .unwrap();
                        }
                    }
                };
                assert_eq!(request["method"], expected);
                let result = if expected == "tools/list" {
                    json!({"tools":[{"name":"remote.http","inputSchema":{"type":"object","properties":{}}}]})
                } else {
                    json!({"content":[],"structuredContent":{"http":true}})
                };
                let body = json!({"jsonrpc":"2.0","id":request["id"],"result":result}).to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let directory = TemporaryDirectory::new("stdio-mixed-http");
        let mut stdio: McpServerConfig =
            toml::from_str(&server(&directory, "normal", 2000)).unwrap();
        stdio.resolve_relative_to(directory.path());
        let http: McpServerConfig = toml::from_str(&format!(
            "url = {endpoint:?}\nprotocol_version = \"2026-07-28\"\n"
        ))
        .unwrap();
        let runtime = McpRuntime::new(
            BTreeMap::from([("local".to_owned(), stdio), ("remote".to_owned(), http)]),
            Arc::new(NoCredentials),
        );
        let tools = runtime.discover().await.unwrap();
        assert_eq!(tools.len(), 2);
        assert!(tools[0].endpoint.starts_with("stdio:"));
        assert_eq!(tools[1].endpoint, endpoint);
        assert!(matches!(
            tools[0].call(json!({})).await,
            Ok(CompletedCall::Result(_))
        ));
        assert!(matches!(
            tools[1].call(json!({})).await,
            Ok(CompletedCall::Result(_))
        ));
        tokio::time::timeout(Duration::from_secs(3), peer)
            .await
            .unwrap()
            .unwrap();
        drop(runtime);
        assert_reaped_and_group_stopped(&directory);
    }

    fn cli(directory: &TemporaryDirectory, config: &Path, operation: &str) -> std::process::Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_echidna"));
        command
            .current_dir(directory.path())
            .args([operation, "--config", config.to_str().unwrap()])
            .env("CYBER_AGENT_PRIMARY_API_KEY", "AMBIENT_PROVIDER_PRIVATE")
            .env("OPENAI_API_KEY", "AMBIENT_VENDOR_PRIVATE")
            .env("MCP_UNCONFIGURED_PRIVATE", "ambient-excluded")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().unwrap();
        child.stdin.take().unwrap().write_all(b"/exit\n").unwrap();
        let until = std::time::Instant::now() + Duration::from_secs(8);
        while child.try_wait().unwrap().is_none() {
            if std::time::Instant::now() >= until {
                child.kill().unwrap();
                panic!("CLI watchdog elapsed");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        child.wait_with_output().unwrap()
    }

    #[test]
    fn stdio_cli_doctor_is_offline_chat_exit_uses_config_paths_and_filtered_environment() {
        let directory = TemporaryDirectory::new("stdio-cli");
        fs::copy(fixture_path(), directory.path().join("server.py")).unwrap();
        let launcher = directory.path().join("run-mcp");
        fs::write(
            &launcher,
            "#!/bin/sh\nexec python3 server.py startup_stderr . \"$@\"\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&launcher, fs::Permissions::from_mode(0o700)).unwrap();
        let source = "command = \"./run-mcp\"\nargs = [\"$LITERAL_ARGUMENT\"]\nprotocol_version = \"2025-11-25\"\nenv = { MCP_LITERAL = \"$NO_EXPANSION\", MCP_EXPLICIT = \"explicit-value\" }\n";
        let config = config_file(&directory, source);
        let doctor = cli(&directory, &config, "doctor");
        assert!(
            doctor.status.success(),
            "{}",
            String::from_utf8_lossy(&doctor.stderr)
        );
        assert!(!directory.path().join("launch.json").exists());
        let initialized = Command::new(env!("CARGO_BIN_EXE_echidna"))
            .current_dir(directory.path())
            .arg("init")
            .output()
            .unwrap();
        assert!(initialized.status.success());
        let chat = cli(&directory, &config, "chat");
        assert!(
            chat.status.success(),
            "{}",
            String::from_utf8_lossy(&chat.stderr)
        );
        let observed = launch(&directory);
        assert_eq!(
            Path::new(observed["cwd"].as_str().unwrap())
                .canonicalize()
                .unwrap(),
            directory.path().canonicalize().unwrap()
        );
        assert_eq!(observed["literal"], "$NO_EXPANSION");
        assert_eq!(observed["explicit"], "explicit-value");
        assert_eq!(observed["args"], json!(["$LITERAL_ARGUMENT"]));
        let keys = observed["env_keys"].as_array().unwrap();
        for absent in [
            "CYBER_AGENT_PRIMARY_API_KEY",
            "OPENAI_API_KEY",
            "MCP_UNCONFIGURED_PRIVATE",
        ] {
            assert!(!keys.contains(&json!(absent)));
        }
        let terminal = format!(
            "{}{}",
            String::from_utf8_lossy(&chat.stdout),
            String::from_utf8_lossy(&chat.stderr)
        );
        for secret in [
            "AMBIENT_PROVIDER_PRIVATE",
            "AMBIENT_VENDOR_PRIVATE",
            "MCP_STDERR_PRIVATE_CANARY",
        ] {
            assert!(!terminal.contains(secret));
        }
        // This process is the CLI's child, so only its descendant termination can be observed here.
        let pid = observed["pid"].as_i64().unwrap().to_string();
        let status = Command::new("ps")
            .args(["-o", "stat=", "-p", &pid])
            .output()
            .unwrap();
        assert!(String::from_utf8(status.stdout).unwrap().trim().is_empty());
    }

    #[test]
    fn stdio_config_decode_errors_never_echo_secret_source_values() {
        let directory = TemporaryDirectory::new("stdio-private-config");
        let secret = "PRIVATE_DECODE_CANARY";
        let valid = server(&directory, "normal", 2000);
        for source in [
            format!("{valid}\nenv = [{secret:?}]"),
            format!("{valid}\nargs = {{{secret:?} = 'value'}}"),
            format!("{valid}\nunknown_private = {secret:?}"),
            valid.replace("2025-11-25", secret),
        ] {
            let config = config_file(&directory, &source);
            let output = cli(&directory, &config, "doctor");
            assert!(!output.status.success());
            let terminal = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(!terminal.contains(secret), "{terminal}");
            assert!(terminal.contains("<redacted>"), "{terminal}");
            assert!(terminal.contains("bytes"), "{terminal}");
        }
    }
    #[tokio::test]
    async fn stdio_startup_observer_identifies_failed_stage_and_preserves_cause() {
        use crate::adapters::mcp::observation::{McpObservation, McpStartupStage};
        for (mode, expected) in [
            ("normal", None),
            ("bad_initialize", Some(McpStartupStage::Initializing)),
            ("bad_catalog", Some(McpStartupStage::Listing)),
            ("startup_timeout", Some(McpStartupStage::Initializing)),
        ] {
            let directory = TemporaryDirectory::new(&format!("stdio-stages-{mode}"));
            let events = Arc::new(std::sync::Mutex::new(Vec::new()));
            let recorded = Arc::clone(&events);
            let runtime =
                runtime(&directory, mode, 350).with_observer(Some(Arc::new(move |event| {
                    recorded.lock().unwrap().push(event)
                })));
            let result = runtime.discover().await;
            if let Some(expected) = expected {
                let error = result.err().expect("startup must fail visibly");
                assert!(
                    matches!(&error, McpError::Startup { server_id, stage, source } if server_id == "local" && *stage == expected && !source.to_string().is_empty()),
                    "{error}"
                );
                let events = events.lock().unwrap();
                assert!(
                    matches!(events.first(), Some(McpObservation::Startup { server_id, stage: McpStartupStage::Starting }) if server_id == "local")
                );
                assert!(
                    matches!(events.last(), Some(McpObservation::StartupFailed { server_id, stage }) if server_id == "local" && *stage == expected),
                    "{events:?}"
                );
                assert!(!events.iter().any(|event| matches!(
                    event,
                    McpObservation::Startup {
                        stage: McpStartupStage::Ready,
                        ..
                    }
                )));
            } else {
                assert!(!result.unwrap().is_empty());
                assert!(matches!(
                    events.lock().unwrap().last(),
                    Some(McpObservation::Startup {
                        stage: McpStartupStage::Ready,
                        ..
                    })
                ));
            }
            drop(runtime);
            assert_reaped_and_group_stopped(&directory);
        }
    }

    #[tokio::test]
    async fn stdio_separate_startup_deadline_does_not_relax_short_tool_call() {
        let directory = TemporaryDirectory::new("stdio-separate-deadlines");
        let runtime = runtime(&directory, "slow_startup_short_call", 40);
        let tools = runtime
            .discover()
            .await
            .expect("150ms initialize uses explicit startup deadline, not the 40ms call deadline");
        assert!(
            matches!(tools[0].call(json!({})).await, Err(McpError::Protocol { message }) if message == "stdio request timed out after dispatch")
        );
        assert_eq!(
            requests(&directory)
                .iter()
                .filter(|request| request["method"] == "tools/call")
                .count(),
            1
        );
        drop(runtime);
        assert_reaped_and_group_stopped(&directory);
    }

    #[test]
    fn startup_timeout_validation_uses_the_config_boundary_for_both_transports() {
        for transport in ["url = \"http://127.0.0.1:1/mcp\"", "command = \"python3\""] {
            for timeout in [0, 86_400_001] {
                let directory = TemporaryDirectory::new("mcp-invalid-startup-deadline");
                let path = config_file(
                    &directory,
                    &format!(
                        "{transport}\nprotocol_version = \"2025-11-25\"\nstartup_timeout_ms = {timeout}\n"
                    ),
                );
                let error = ProviderComposition::load(path, Arc::new(NoCredentials))
                    .err()
                    .expect("invalid deadline rejected without credentials or startup");
                assert!(
                    error.to_string().contains("mcp_servers.startup_timeout_ms"),
                    "{error}"
                );
                assert!(!directory.path().join("launch.json").exists());
            }
        }
    }
}
