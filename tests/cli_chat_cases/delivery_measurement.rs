//! Offline measurements at the formal CLI/wire/audit boundaries, not telemetry.

use super::*;
use echidna::domain::model::{ContentItem, MessageRole};
use echidna::domain::request_audit::RequestAuditDeliveryMode;
use echidna::domain::tools::ToolResultStatus;
use std::fs::FileTimes;
use std::path::Path;

const SOURCE_BEFORE: &str = "alpha=10\nbeta=20\n";
const SOURCE_AFTER: &str = "alpha=11\nbeta=20\n";
const MEMORY_BEFORE: &str = "currency=USD\n";
const MEMORY_AFTER: &str = "currency=EUR\n";

fn replace_same_metadata(path: &Path, content: &str) {
    let before = fs::metadata(path).unwrap();
    assert_eq!(before.len(), content.len() as u64);
    fs::write(path, content).unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(path)
        .unwrap()
        .set_times(FileTimes::new().set_modified(before.modified().unwrap()))
        .unwrap();
    let after = fs::metadata(path).unwrap();
    assert_eq!(after.len(), before.len());
    assert_eq!(after.modified().unwrap(), before.modified().unwrap());
    assert_eq!(fs::read_to_string(path).unwrap(), content);
}

fn snapshot_fingerprint(root: &Path) -> Value {
    let snapshot: Value = serde_json::from_slice(
        &fs::read(root.join(".echidna/cache/project-snapshot.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(snapshot["source_fingerprint"]["kind"], "filesystem");
    snapshot["source_fingerprint"].clone()
}

fn delivery_server() -> LoopbackServer {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
    let (sender, requests) = mpsc::channel();
    let thread = thread::spawn(move || {
        for index in 0..4 {
            let mut socket = f3_context::accept_before_deadline(&listener, "delivery measurement");
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            socket
                .set_write_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            sender.send(read_http_request(&mut socket)).unwrap();
            let phase = index / 2;
            let response = if index % 2 == 0 {
                m4_long_flow_tool_response(
                    &format!("read-{phase}"),
                    "read_file",
                    json!({"path":"source.txt"}),
                    &format!("read-response-{phase}"),
                )
            } else {
                text_response(&format!("DONE_{phase}"), &format!("done-response-{phase}"))
            };
            f3_context::write_sse_response(&mut socket, &response);
        }
    });
    LoopbackServer {
        base_url,
        requests,
        thread,
    }
}

fn four_cases(repetitions: usize, emit: bool) {
    for incremental in [false, true] {
        for repetition in 0..repetitions {
            let cases = [
                "unchanged",
                "source_edit",
                "memory_edit",
                "external_log_growth",
            ];
            for offset in 0..cases.len() {
                let case = cases[(offset + repetition) % cases.len()];
                let root = TemporaryDirectory::new("delivery-project");
                let harness = TemporaryDirectory::new("delivery-harness");
                initialize_project(root.path());
                fs::write(root.path().join("source.txt"), SOURCE_BEFORE).unwrap();
                let memory = root.path().join(".echidna/memory/core.md");
                fs::create_dir_all(memory.parent().unwrap()).unwrap();
                fs::write(&memory, MEMORY_BEFORE).unwrap();
                let log = harness.path().join("external.log");
                fs::write(&log, "outside project\n").unwrap();
                assert!(!harness.path().starts_with(root.path()));
                let server = delivery_server();
                let config =
                    write_context_window_config(harness.path(), &server.base_url, 1_048_576);
                if !incremental {
                    let configured = fs::read_to_string(&config)
                        .unwrap()
                        .replace("incremental_input = true", "incremental_input = false");
                    fs::write(&config, configured).unwrap();
                }
                let project = Project::discover(root.path()).unwrap();
                let mut session = String::new();
                let mut old_events = Vec::new();
                let mut old_fingerprint = Value::Null;
                for phase in 0..2 {
                    if phase == 1 {
                        match case {
                            "source_edit" => {
                                replace_same_metadata(&root.path().join("source.txt"), SOURCE_AFTER)
                            }
                            "memory_edit" => replace_same_metadata(&memory, MEMORY_AFTER),
                            "external_log_growth" => fs::OpenOptions::new()
                                .append(true)
                                .open(&log)
                                .unwrap()
                                .write_all(&vec![b'l'; 1024 * 1024])
                                .unwrap(),
                            "unchanged" => {}
                            _ => unreachable!(),
                        }
                    }
                    let extra = if phase == 0 {
                        vec![]
                    } else {
                        vec!["--resume", session.as_str()]
                    };
                    let start = Instant::now();
                    let output = f2a_wait_child(
                        spawn_f2a_chat(
                            root.path(),
                            &config,
                            "read source.txt once\n/exit\n",
                            &extra,
                        ),
                        "delivery measurement",
                    );
                    let cli_elapsed_ns = start.elapsed().as_nanos();
                    assert!(
                        output.status.success(),
                        "{}",
                        String::from_utf8_lossy(&output.stderr)
                    );
                    assert!(
                        String::from_utf8_lossy(&output.stdout).contains(&format!("DONE_{phase}"))
                    );
                    if phase == 0 {
                        session = session_id(root.path());
                    }
                    assert_eq!(session_id(root.path()), session);
                    let event_path = root
                        .path()
                        .join(".echidna/sessions")
                        .join(&session)
                        .join("events.jsonl");
                    let events = fs::read(&event_path).unwrap();
                    assert!(events.starts_with(&old_events));
                    let state = reduced_session(&project, &session);
                    assert_eq!(state.request_audits().len(), (phase + 1) * 2);
                    assert_eq!(state.execution_receipt_count(), (phase + 1) as u64);
                    assert_eq!(state.history().ensure_all_tool_calls_paired(), Ok(()));
                    assert_audits_reconstruct(root.path(), &state);
                    let expected_source = if phase == 1 && case == "source_edit" {
                        SOURCE_AFTER
                    } else {
                        SOURCE_BEFORE
                    };
                    let expected_memory = if phase == 1 && case == "memory_edit" {
                        MEMORY_AFTER
                    } else {
                        MEMORY_BEFORE
                    };
                    let current_call = format!("read-{phase}");
                    let results: Vec<_> = state
                        .history()
                        .items()
                        .iter()
                        .filter_map(|item| {
                            if let ContentItem::ToolResult { result } = item {
                                Some(result)
                            } else {
                                None
                            }
                        })
                        .collect();
                    assert_eq!(results.len(), phase + 1);
                    let result = results.last().unwrap();
                    assert_eq!(result.call_id.as_str(), current_call);
                    assert_eq!(result.status, ToolResultStatus::Succeeded);
                    assert_eq!(result.output["content"], expected_source);
                    assert_eq!(result.output["truncated"], false);
                    assert!(state.history().items().iter().any(|item| matches!(item,
                        ContentItem::Text { role: MessageRole::Assistant, text } if text == &format!("DONE_{phase}"))));
                    let mut samples = Vec::new();
                    for step in 0..2 {
                        let raw = server.requests.recv_timeout(F2A_WAIT).unwrap();
                        let body = raw.split_once("\r\n\r\n").unwrap().1;
                        let wire: Value = serde_json::from_str(body).unwrap();
                        let audit = &state.request_audits()[phase * 2 + step];
                        let is_delta = incremental && step == 1;
                        assert_eq!(
                            audit.delivery().mode(),
                            if is_delta {
                                RequestAuditDeliveryMode::Incremental
                            } else {
                                RequestAuditDeliveryMode::Full
                            }
                        );
                        if is_delta {
                            assert_eq!(
                                wire["previous_response_id"],
                                format!("read-response-{phase}")
                            );
                        } else {
                            assert!(wire.get("previous_response_id").is_none());
                        }
                        let reconstructed = audit
                            .reconstruct(|blob| {
                                fs::read(
                                    root.path()
                                        .join(".echidna/artifacts/blobs/sha256")
                                        .join(blob.as_str()),
                                )
                            })
                            .unwrap();
                        assert_eq!(wire["instructions"], reconstructed.instructions);
                        assert_eq!(
                            wire["tools"].as_array().unwrap().len(),
                            reconstructed.tool_specs.len()
                        );
                        if step == 0 {
                            assert!(wire.to_string().contains(expected_memory.trim()));
                            if phase == 1 && case == "memory_edit" {
                                assert!(!wire.to_string().contains(MEMORY_BEFORE.trim()));
                            }
                        } else {
                            let item = wire["input"]
                                .as_array()
                                .unwrap()
                                .iter()
                                .find(|item| {
                                    item["type"] == "function_call_output"
                                        && item["call_id"] == current_call
                                })
                                .unwrap();
                            let delivered: Value =
                                serde_json::from_str(item["output"].as_str().unwrap()).unwrap();
                            assert_eq!(delivered["output"]["content"], expected_source);
                            assert_eq!(delivered["output"]["truncated"], false);
                        }
                        samples.push(json!({
                            "step": step, "http_body_bytes": body.len(),
                            "input_json_bytes": serde_json::to_vec(&wire["input"]).unwrap().len(),
                            "tools_json_bytes": serde_json::to_vec(&wire["tools"]).unwrap().len(),
                            "instructions_utf8_bytes": wire["instructions"].as_str().unwrap().len(),
                            "audit_manifest_json_bytes": serde_json::to_vec(audit).unwrap().len(),
                            "audit_component_bytes": audit.components().iter().map(|c| c.size_bytes()).sum::<u64>(),
                            "delivery": audit.delivery()
                        }));
                    }
                    // Use the existing public CLI projection; do not widen its private
                    // estimator API or duplicate the estimator in measurement code.
                    let metrics_output = f2a_wait_child(
                        Command::new(env!("CARGO_BIN_EXE_echidna"))
                            .current_dir(root.path())
                            .args(["metrics", "--session", &session])
                            .stdin(Stdio::null())
                            .stdout(Stdio::piped())
                            .stderr(Stdio::piped())
                            .spawn()
                            .unwrap(),
                        "delivery metrics",
                    );
                    assert!(metrics_output.status.success());
                    let metrics: Value = serde_json::from_slice(&metrics_output.stdout).unwrap();
                    assert_eq!(metrics["status"], "ok");
                    assert_eq!(metrics["requests"], (phase + 1) * 2);
                    assert_eq!(metrics["provider_usage"], "unavailable");
                    let fingerprint = snapshot_fingerprint(root.path());
                    if phase == 1 && matches!(case, "unchanged" | "external_log_growth") {
                        assert_eq!(fingerprint, old_fingerprint);
                    }
                    let manifest: Value = serde_json::from_slice(
                        &fs::read(root.path().join(".echidna/cache/file-manifest.json")).unwrap(),
                    )
                    .unwrap();
                    assert!(
                        !manifest["entries"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|entry| entry["relative_path"] == "external.log"
                                || entry["relative_path"] == "provider-context-1048576.toml")
                    );
                    if emit {
                        println!(
                            "CHAT_MEASUREMENT {}",
                            json!({
                                "schema":1, "case":case, "incremental":incremental, "repetition":repetition,
                                "phase":phase, "cli_elapsed_ns":cli_elapsed_ns, "session_bytes":events.len(),
                            "requests":samples, "receipts":state.execution_receipt_count(),
                            "cumulative_audit_component_token_estimate": metrics["estimated_tokens"],
                                "snapshot_fingerprint_changed": if phase == 0 { Value::Null } else { json!(fingerprint != old_fingerprint) },
                                "unavailable":["hash_calls", "hash_bytes", "hash_time", "directory_scans", "ttft", "provider_usage"]
                            })
                        );
                    }
                    old_events = events;
                    old_fingerprint = fingerprint;
                }
                f2a_join_server(server, "delivery measurement");
            }
        }
    }
}

#[test]
fn chat_delivery_preserves_current_content_across_same_metadata_resume() {
    four_cases(1, false);
}

#[test]
#[ignore = "offline release measurement; opt in with --exact --ignored --nocapture --test-threads=1"]
fn chat_delivery_four_case_measurement() {
    four_cases(3, true);
}
