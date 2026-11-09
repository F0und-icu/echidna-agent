//! Cancellation at an actual pending selector, using a terminal and loopback Provider.
use std::collections::BTreeMap;

use super::task_context::{
    ScriptedResponse, continue_selection_response, definition, new_selection_response,
    selector_candidates, spawn_scripted_responses_server,
};
use super::*;
use portable_pty::{CommandBuilder, PtySize, native_pty_system};

#[test]
fn incremental_actual_tool_result_substitution_fails_source_replay() {
    use echidna::domain::model::{ContentItem, ModelInputIR};
    use echidna::domain::request_audit::RequestAuditManifest;
    use sha2::{Digest, Sha256};

    let root = TemporaryDirectory::new("actual-delivery-substitution");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let server = spawn_scripted_responses_server(vec![
        ScriptedResponse::InitialSelection,
        ScriptedResponse::Response(
            200,
            m4_long_flow_tool_response(
                "actual-tool",
                "run_command",
                json!({"argv":["/bin/echo","actual output"]}),
                "actual-first",
            ),
        ),
        ScriptedResponse::Response(
            200,
            text_response("Actual delivery complete.", "actual-final"),
        ),
    ]);
    let config = write_context_window_config(root.path(), &server.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "Run one synthetic command.\n/exit\n",
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for _ in 0..3 {
        server.requests.recv_timeout(F2A_WAIT).unwrap();
    }
    server.join();
    let session = session_id(root.path());
    let events = session_events(root.path(), &session);
    let position = events
        .iter()
        .rposition(|event| event["event_type"] == "session.request_audit_recorded")
        .unwrap();
    let prefix = events[..=position].to_vec();
    let event_path = root
        .path()
        .join(".echidna/sessions")
        .join(&session)
        .join("events.jsonl");
    let write_events = |events: &[Value]| {
        fs::write(
            &event_path,
            events
                .iter()
                .map(|event| format!("{}\n", serde_json::to_string(event).unwrap()))
                .collect::<String>(),
        )
        .unwrap()
    };
    write_events(&prefix);
    let valid = run_chat(root.path(), &config, "/exit\n", &["--resume", "latest"]);
    assert!(
        valid.status.success(),
        "{}",
        String::from_utf8_lossy(&valid.stderr)
    );
    // Give the actual ToolResult a distinct, semantically equivalent Blob so
    // missing/corrupt coverage cannot be satisfied by reading logical bytes.
    let blobs = root.path().join(".echidna/artifacts/blobs/sha256");
    let mut isolated = prefix.clone();
    let actual_components = isolated[position]["payload"]["components"]
        .as_array_mut()
        .unwrap();
    let actual_component = actual_components
        .iter_mut()
        .find(|component| {
            if component["identity"]["kind"] != "content_item" {
                return false;
            }
            let bytes = fs::read(blobs.join(component["blob_id"].as_str().unwrap())).unwrap();
            matches!(
                serde_json::from_slice::<ContentItem>(&bytes),
                Ok(ContentItem::ToolResult { .. })
            )
        })
        .unwrap();
    let mut isolated_bytes =
        fs::read(blobs.join(actual_component["blob_id"].as_str().unwrap())).unwrap();
    isolated_bytes.push(b' ');
    let isolated_blob = format!("{:x}", Sha256::digest(&isolated_bytes));
    fs::write(blobs.join(&isolated_blob), &isolated_bytes).unwrap();
    actual_component["blob_id"] = json!(isolated_blob);
    actual_component["size_bytes"] = json!(isolated_bytes.len());
    write_events(&isolated);
    let isolated_valid = run_chat(root.path(), &config, "/exit\n", &["--resume", "latest"]);
    assert!(
        isolated_valid.status.success(),
        "{}",
        String::from_utf8_lossy(&isolated_valid.stderr)
    );
    fs::remove_file(blobs.join(&isolated_blob)).unwrap();
    let missing = run_chat(root.path(), &config, "/exit\n", &["--resume", "latest"]);
    assert!(
        !missing.status.success(),
        "missing actual-only Blob was accepted"
    );
    fs::write(blobs.join(&isolated_blob), b"corrupt").unwrap();
    let corrupt = run_chat(root.path(), &config, "/exit\n", &["--resume", "latest"]);
    assert!(
        !corrupt.status.success(),
        "corrupt actual-only Blob was accepted"
    );
    fs::write(blobs.join(&isolated_blob), &isolated_bytes).unwrap();
    let mut forged = prefix.clone();
    let payload = &mut forged[position]["payload"];
    assert_eq!(payload["delivery"]["mode"], "incremental");
    let original_logical = payload["full_logical_input"].clone();
    let original_logical_components = payload["full_logical_components"].clone();
    let original_logical_map = payload["full_context_delivery_map"].clone();
    let components = payload["components"].as_array().unwrap();
    let read =
        |component: &Value| fs::read(blobs.join(component["blob_id"].as_str().unwrap())).unwrap();
    let instructions = String::from_utf8(read(&components[0])).unwrap();
    let mut items = Vec::new();
    let mut tools = Vec::new();
    let mut requirements = Value::Null;
    for component in &components[1..] {
        match component["identity"]["kind"].as_str().unwrap() {
            "content_item" => {
                items.push(serde_json::from_slice::<Value>(&read(component)).unwrap())
            }
            "tool_spec" => tools.push(serde_json::from_slice::<Value>(&read(component)).unwrap()),
            "request_requirements" => {
                requirements = serde_json::from_slice(&read(component)).unwrap()
            }
            other => panic!("unexpected component {other}"),
        }
    }
    let mut delivered: ModelInputIR = serde_json::from_value(json!({"schema_version":payload["delivered_input"]["schema_version"], "instructions":instructions, "content_items":items, "tool_specs":tools, "requirements":requirements})).unwrap();
    let index = delivered
        .content_items
        .iter()
        .position(|item| matches!(item, ContentItem::ToolResult { .. }))
        .unwrap();
    let ContentItem::ToolResult { result } = &mut delivered.content_items[index] else {
        unreachable!()
    };
    result.output = json!({"substituted":"actual delivery only"});
    let replacement = &delivered.content_items[index];
    let bytes = delivery_lineage::canonical_bytes(replacement);
    let blob = format!("{:x}", Sha256::digest(&bytes));
    fs::write(blobs.join(&blob), &bytes).unwrap();
    let hash = delivery_lineage::canonical_hash(replacement);
    let component = payload["components"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|component| {
            component["identity"]["kind"] == "content_item"
                && component["identity"]["index"] == index
        })
        .unwrap();
    component["blob_id"] = json!(blob);
    component["size_bytes"] = json!(bytes.len());
    component["semantic_hash"] = json!(hash);
    payload["context_delivery_map"]["content_items"][index]["content_hash"] = json!(hash);
    payload["delivered_input"]["semantic_hash"] =
        json!(delivery_lineage::canonical_hash(&delivered));
    assert_eq!(payload["full_logical_input"], original_logical);
    assert_eq!(
        payload["full_logical_components"],
        original_logical_components
    );
    assert_eq!(payload["full_context_delivery_map"], original_logical_map);
    let manifest: RequestAuditManifest = serde_json::from_value(payload.clone()).unwrap();
    manifest.validate().unwrap();
    let decoded = forged
        .iter()
        .cloned()
        .map(|event| serde_json::from_value::<EventEnvelope<Value>>(event).unwrap())
        .collect::<Vec<_>>();
    SessionReducer
        .reduce(decoded)
        .expect("actual-only substitution passes structural replay");
    write_events(&forged);
    let before = fs::read(&event_path).unwrap();
    let rejected = run_chat(root.path(), &config, "/exit\n", &["--resume", "latest"]);
    assert!(
        !rejected.status.success(),
        "actual-only substitution was accepted"
    );
    assert_eq!(fs::read(&event_path).unwrap(), before);
}

#[test]
fn cancelling_second_semantic_selection_preserves_first_sample_without_adoption() {
    let root = TemporaryDirectory::new("selector-expansion-cancel");
    initialize_project(root.path());
    let agent = definition(root.path(), None);
    let initial = spawn_scripted_responses_server(vec![
        ScriptedResponse::InitialSelection,
        ScriptedResponse::Response(200, text_response("FIRST_OBSERVATION", "first")),
    ]);
    let config = write_context_window_config(root.path(), &initial.base_url, 272_000);
    let output = run_chat(
        root.path(),
        &config,
        "First task.\n/exit\n",
        &["--agent", agent.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    initial.requests.recv_timeout(F2A_WAIT).unwrap();
    initial.requests.recv_timeout(F2A_WAIT).unwrap();
    initial.join();
    let session = session_id(root.path());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (sent, requests) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut first, _) = listener.accept().unwrap();
        let wire = request_body(&read_http_request(&mut first));
        let candidates = selector_candidates(&wire);
        sent.send(wire).unwrap();
        let decision = json!({
            "relation":"new", "focus":{"kind":"new_from_input","input_ref":candidates["input"]["ref"]},
            "related_task_refs":[], "required_context_refs":[], "unresolved_references":["FIRST_OBSERVATION"]
        });
        let body = text_response(&decision.to_string(), "first-semantic-selection");
        write!(first, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        let (mut second, _) = listener.accept().unwrap();
        let wire = request_body(&read_http_request(&mut second));
        selector_candidates(&wire);
        sent.send(wire).unwrap();
        released.recv_timeout(F2A_WAIT).unwrap();
    });
    let config = write_context_window_config(root.path(), &format!("http://{address}/v1"), 272_000);
    let child = spawn_f2a_chat(
        root.path(),
        &config,
        "Second task needs FIRST_OBSERVATION.\n/exit\n",
        &["--resume", "latest"],
    );
    requests.recv_timeout(F2A_WAIT).unwrap();
    requests.recv_timeout(F2A_WAIT).unwrap();
    assert!(
        Command::new("/bin/kill")
            .args(["-INT", &child.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    let output = f2a_wait_child(child, "cancel expanded selector");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("[request cancelled]"));
    release.send(()).unwrap();
    server.join().unwrap();
    let events = session_events(root.path(), &session);
    assert_eq!(
        events
            .iter()
            .filter(|e| e["event_type"] == "session.task_selection_sample_recorded")
            .count(),
        2
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| e["event_type"] == "session.task_context_selected")
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| e["event_type"] == "session.model_delivery_committed")
            .count(),
        1
    );
    assert_eq!(
        reduced_session(&Project::discover(root.path()).unwrap(), &session)
            .execution_receipt_count(),
        0
    );
}

#[test]
fn pending_selector_accepts_terminal_correction_and_ctrl_c() {
    for correction in [true, false] {
        let root = TemporaryDirectory::new("pending-selector-terminal");
        initialize_project(root.path());
        let agent = definition(root.path(), None);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (sent, requests) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let server = thread::spawn(move || {
            let mut pending = None;
            let mut cancelled_input = String::new();
            let mut task = String::new();
            for index in 0..6 {
                let (mut socket, _) = listener.accept().unwrap();
                socket.set_read_timeout(Some(F2A_WAIT)).unwrap();
                let raw = read_http_request(&mut socket);
                let wire = request_body(&raw);
                sent.send(wire.clone()).unwrap();
                if index == 3 {
                    released.recv_timeout(F2A_WAIT).unwrap();
                }
                let body = match index {
                    0 => new_selection_response(&wire, "first-selection"),
                    1 => text_response("FIRST_DONE", "first-main"),
                    2 => {
                        cancelled_input = selector_candidates(&wire)["input"]["ref"]
                            .as_str()
                            .unwrap()
                            .to_owned();
                        pending = Some(socket);
                        continue;
                    }
                    3 => {
                        task = selector_candidates(&wire)["tasks"][0]["ref"]
                            .as_str()
                            .unwrap()
                            .to_owned();
                        continue_selection_response(&wire, "corrected-selection", None)
                    }
                    4 => tool_calls_response(
                        &[
                            (
                                "cancel-progress",
                                "context_read",
                                json!({"operation":"progress","task_ref":task,"max_bytes":16384}),
                            ),
                            (
                                "cancel-raw",
                                "context_read",
                                json!({"operation":"read","reference":cancelled_input,"max_bytes":1024}),
                            ),
                        ],
                        "corrected-read",
                    ),
                    5 => text_response("CORRECTED_DONE", "corrected-main"),
                    _ => unreachable!(),
                };
                write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
            drop(pending);
        });
        let config =
            write_context_window_config(root.path(), &format!("http://{address}/v1"), 272_000);
        let pair = native_pty_system().openpty(PtySize::default()).unwrap();
        let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_echidna"));
        command.cwd(root.path());
        command.args([
            "--config",
            config.to_str().unwrap(),
            "chat",
            "--agent",
            agent.to_str().unwrap(),
        ]);
        command.env("CYBER_AGENT_PRIMARY_API_KEY", "loopback-primary-secret");
        command.env("TERM", "xterm-256color");
        let mut child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);
        let mut writer = pair.master.take_writer().unwrap();
        let mut reader = pair.master.try_clone_reader().unwrap();
        let (output_sender, output) = mpsc::channel();
        thread::spawn(move || {
            let mut bytes = [0; 4096];
            while let Ok(count) = reader.read(&mut bytes) {
                if count == 0 || output_sender.send(bytes[..count].to_vec()).is_err() {
                    break;
                }
            }
        });
        let mut captured = Vec::new();
        let mut observed = 0;
        let mut wait = |needle: &str| {
            let deadline = Instant::now() + F2A_WAIT;
            loop {
                if let Some(offset) = String::from_utf8_lossy(&captured[observed..]).find(needle) {
                    observed += offset + needle.len();
                    break;
                }
                let bytes = output
                    .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                    .unwrap_or_else(|error| {
                        panic!(
                            "waiting for {needle}: {error}; {}",
                            String::from_utf8_lossy(&captured)
                        )
                    });
                captured.extend(bytes);
            }
        };
        wait("you> ");
        writer.write_all(b"First task.\n").unwrap();
        writer.flush().unwrap();
        let initial = requests.recv_timeout(F2A_WAIT).unwrap();
        assert_eq!(
            selector_candidates(&initial)["input"]["text"],
            "First task."
        );
        requests.recv_timeout(F2A_WAIT).unwrap();
        wait("FIRST_DONE");
        wait("you> ");
        let session = session_id(root.path());
        let events = session_events(root.path(), &session);
        let initial_samples = events
            .iter()
            .filter(|e| e["event_type"] == "session.task_selection_sample_recorded")
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(initial_samples.len(), 1);
        assert_eq!(
            events
                .iter()
                .filter(|e| e["event_type"] == "session.model_delivery_committed")
                .count(),
            1
        );
        writer.write_all(b"Old second task.\n").unwrap();
        writer.flush().unwrap();
        let old = requests.recv_timeout(F2A_WAIT).unwrap();
        assert!(
            selector_candidates(&old)["input"]
                .to_string()
                .contains("Old second task")
        );
        if correction {
            writer.write_all(b"Corrected task.\n").unwrap();
        } else {
            writer.write_all(b"\x03").unwrap();
        }
        writer.flush().unwrap();
        wait("[request cancelled]");
        let events = session_events(root.path(), &session);
        assert_eq!(
            events
                .iter()
                .filter(|event| {
                    event["event_type"] == "session.operator_turn_boundary_recorded"
                        && event["payload"]["kind"] == "initial_selection_cancelled"
                })
                .count(),
            1
        );
        assert!(events.iter().any(|event| {
            event["event_type"] == "session.history_item_recorded"
                && event["payload"]["item"]["kind"] == "text"
                && event["payload"]["item"]["role"] == "user"
                && event["payload"]["item"]["text"] == "Old second task."
        }));
        assert_eq!(
            events
                .iter()
                .filter(|e| e["event_type"] == "session.task_context_selected")
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|e| e["event_type"] == "session.task_selection_sample_recorded")
                .cloned()
                .collect::<Vec<_>>(),
            initial_samples,
            "cancelling the second selector preserves only the original accepted sample"
        );
        assert_eq!(
            events
                .iter()
                .filter(|e| e["event_type"] == "session.model_delivery_committed")
                .count(),
            1
        );
        if !correction {
            writer.write_all(b"Corrected task.\n").unwrap();
            writer.flush().unwrap();
        }
        let next = requests.recv_timeout(F2A_WAIT).unwrap();
        assert!(
            selector_candidates(&next)["input"]
                .to_string()
                .contains("Corrected task")
        );
        assert!(
            !selector_candidates(&next)
                .to_string()
                .contains("Old second task."),
            "a cancelled initial input must not become a task or context candidate"
        );
        release.send(()).unwrap();
        let main = requests.recv_timeout(F2A_WAIT).unwrap();
        assert!(main.to_string().contains("Corrected task"));
        assert!(!main["input"].to_string().contains("Old second task."));
        requests.recv_timeout(F2A_WAIT).unwrap();
        wait("CORRECTED_DONE");
        writer.write_all(b"/exit\n").unwrap();
        writer.flush().unwrap();
        let deadline = Instant::now() + F2A_WAIT;
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            assert!(Instant::now() < deadline, "terminal did not exit");
            thread::sleep(Duration::from_millis(10));
        }
        server.join().unwrap();
        let project = Project::discover(root.path()).unwrap();
        let state = reduced_session(&project, &session);
        let results = state
            .history()
            .items()
            .iter()
            .filter_map(|item| match item {
                echidna::domain::model::ContentItem::ToolResult { result } => {
                    Some((result.call_id.as_str(), result))
                }
                _ => None,
            })
            .collect::<BTreeMap<_, _>>();
        let progress = &results["cancel-progress"].output;
        assert_eq!(progress["status"], "found");
        assert!(!progress.to_string().contains("Old second task."));
        let raw = &results["cancel-raw"].output;
        assert_eq!(raw["status"], "found");
        assert_eq!(raw["entries"][0]["text"], "Old second task.");
        let resumed = run_chat(root.path(), &config, "/exit\n", &["--resume", "latest"]);
        assert!(
            resumed.status.success(),
            "{}",
            String::from_utf8_lossy(&resumed.stderr)
        );
    }
}
