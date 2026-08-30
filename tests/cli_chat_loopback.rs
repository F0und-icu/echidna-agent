#[path = "support/provider_config.rs"]
mod provider_config;
mod support;

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde_json::Value;
use support::TemporaryDirectory;

const INLINE_API_KEY_FIXTURE: &str = "loopback-inline-api-key";

struct LoopbackServer {
    base_url: String,
    requests: Receiver<String>,
    thread: JoinHandle<()>,
}

struct PendingLoopbackServer {
    base_url: String,
    requests: Receiver<String>,
    release: mpsc::Sender<()>,
    thread: JoinHandle<()>,
}

impl PendingLoopbackServer {
    fn join(self) {
        self.release.send(()).unwrap();
        self.thread.join().unwrap();
    }
}

impl LoopbackServer {
    fn join(self) {
        self.thread.join().unwrap();
    }
}

fn spawn_responses_server(responses: Vec<(u16, String)>) -> LoopbackServer {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (sender, receiver) = mpsc::channel();
    let thread = thread::spawn(move || {
        for (status, body) in responses {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let request = read_http_request(&mut socket);
            sender.send(request).unwrap();
            let reason = if status == 200 { "OK" } else { "Error" };
            let headers = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            socket.write_all(headers.as_bytes()).unwrap();
            socket.write_all(body.as_bytes()).unwrap();
            socket.flush().unwrap();
        }
    });
    LoopbackServer {
        base_url: format!("http://{address}/v1"),
        requests: receiver,
        thread,
    }
}

fn spawn_pending_responses_server() -> PendingLoopbackServer {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (request_sender, requests) = mpsc::channel();
    let (release, release_receiver) = mpsc::channel();
    let thread = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        request_sender.send(read_http_request(&mut socket)).unwrap();
        release_receiver.recv().unwrap();
    });
    PendingLoopbackServer {
        base_url: format!("http://{address}/v1"),
        requests,
        release,
        thread,
    }
}

fn read_http_request(socket: &mut std::net::TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    let expected_len = loop {
        let read = socket.read(&mut buffer).unwrap();
        assert_ne!(read, 0, "loopback client closed before a complete request");
        bytes.extend_from_slice(&buffer[..read]);
        if let Some(header_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            let headers = std::str::from_utf8(&bytes[..header_end]).unwrap();
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then_some(value.trim())
                })
                .unwrap()
                .parse::<usize>()
                .unwrap();
            break header_end + 4 + content_length;
        }
    };
    while bytes.len() < expected_len {
        let read = socket.read(&mut buffer).unwrap();
        assert_ne!(read, 0, "loopback client closed before its request body");
        bytes.extend_from_slice(&buffer[..read]);
    }
    String::from_utf8(bytes).unwrap()
}

fn write_config(directory: &std::path::Path, base_url: &str) -> std::path::PathBuf {
    let config = directory.join("provider-config.toml");
    fs::write(
        &config,
        provider_config::fixture().replace("http://127.0.0.1:1/v1", base_url),
    )
    .unwrap();
    config
}

fn write_inline_config(directory: &std::path::Path, base_url: &str) -> std::path::PathBuf {
    let config = directory.join("provider-inline-config.toml");
    fs::write(
        &config,
        provider_config::fixture()
            .replace("http://127.0.0.1:1/v1", base_url)
            .replacen(
                "credential_ref = \"CYBER_AGENT_PRIMARY_API_KEY\"",
                &format!("api_key = \"{INLINE_API_KEY_FIXTURE}\""),
                1,
            ),
    )
    .unwrap();
    config
}

fn initialize_project(directory: &std::path::Path) {
    let output = Command::new(env!("CARGO_BIN_EXE_cyber-agent"))
        .current_dir(directory)
        .arg("init")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_chat(
    project: &std::path::Path,
    config: &std::path::Path,
    input: &str,
    extra_args: &[&str],
) -> std::process::Output {
    run_chat_with_pid(project, config, input, extra_args).1
}

fn run_chat_with_pid(
    project: &std::path::Path,
    config: &std::path::Path,
    input: &str,
    extra_args: &[&str],
) -> (u32, std::process::Output) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cyber-agent"));
    command
        .current_dir(project)
        .args(["chat", "--config", config.to_str().unwrap()])
        .args(extra_args)
        .env("CYBER_AGENT_PRIMARY_API_KEY", "loopback-primary-secret")
        .env("OPENAI_API_KEY", "must-not-be-read")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    let process_id = child.id();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    (process_id, child.wait_with_output().unwrap())
}

struct WaitingChat {
    child: Child,
    _output: BufReader<ChildStdout>,
}

impl WaitingChat {
    fn exit(mut self) {
        self.child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(b"/exit\n")
            .unwrap();
        let status = self.child.wait().unwrap();
        assert!(status.success());
    }
}

fn spawn_waiting_chat(project: &std::path::Path, config: &std::path::Path) -> WaitingChat {
    let mut child = Command::new(env!("CARGO_BIN_EXE_cyber-agent"))
        .current_dir(project)
        .args(["chat", "--config", config.to_str().unwrap()])
        .env("CYBER_AGENT_PRIMARY_API_KEY", "loopback-primary-secret")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut started = String::new();
    output.read_line(&mut started).unwrap();
    assert!(started.contains("Started session"), "{started}");
    WaitingChat {
        child,
        _output: output,
    }
}

fn session_id(project: &std::path::Path) -> String {
    let sessions = project.join(".cyber-agent/sessions");
    let mut entries = fs::read_dir(sessions)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect::<Vec<_>>();
    entries.sort();
    assert_eq!(entries.len(), 1);
    entries.pop().unwrap()
}

fn assert_session_id_binds_process(session: &str, process_id: u32) {
    let components = session.split('-').collect::<Vec<_>>();
    assert_eq!(components.len(), 5, "unexpected session format: {session}");
    assert_eq!(components[0], "chat");
    components[1].parse::<u64>().unwrap();
    components[2].parse::<u32>().unwrap();
    assert_eq!(components[3], process_id.to_string());
    components[4].parse::<u64>().unwrap();
}

fn session_events(project: &std::path::Path, session: &str) -> Vec<Value> {
    fs::read_to_string(
        project
            .join(".cyber-agent/sessions")
            .join(session)
            .join("events.jsonl"),
    )
    .unwrap()
    .lines()
    .map(|line| serde_json::from_str(line).unwrap())
    .collect()
}

fn text_response(text: &str, response_id: &str) -> String {
    format!(
        "data: {{\"type\":\"response.output_text.delta\",\"delta\":\"{text}\"}}\n\n\
         data: {{\"type\":\"response.completed\",\"response\":{{\"id\":\"{response_id}\"}}}}\n\n"
    )
}

fn assert_tree_does_not_contain(root: &std::path::Path, needle: &[u8]) {
    for entry in fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            assert_tree_does_not_contain(&path, needle);
        } else {
            let bytes = fs::read(&path).unwrap();
            assert!(
                !bytes.windows(needle.len()).any(|window| window == needle),
                "fixture credential leaked into {}",
                path.display()
            );
        }
    }
}

#[test]
fn inline_api_key_drives_loopback_chat_without_entering_output_or_project_facts() {
    let project = TemporaryDirectory::new("cli-chat-inline-api-key");
    initialize_project(project.path());
    let server = spawn_responses_server(vec![(
        200,
        text_response("inline credential answer", "response-inline"),
    )]);
    let config = write_inline_config(project.path(), &server.base_url);

    let output = run_chat(
        project.path(),
        &config,
        "inline credential turn\n/exit\n",
        &[],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains(INLINE_API_KEY_FIXTURE));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(INLINE_API_KEY_FIXTURE));
    let request = server.requests.recv().unwrap();
    server.join();
    let authorization = request
        .lines()
        .find(|line| line.to_ascii_lowercase().starts_with("authorization:"))
        .unwrap();
    assert_eq!(
        authorization.split_once(':').unwrap().1.trim(),
        format!("Bearer {INLINE_API_KEY_FIXTURE}")
    );
    assert_tree_does_not_contain(
        &project.path().join(".cyber-agent"),
        INLINE_API_KEY_FIXTURE.as_bytes(),
    );
}

#[test]
fn new_session_ids_include_the_creating_process_and_do_not_repeat_across_processes() {
    let first = TemporaryDirectory::new("cli-chat-session-id-first");
    let second = TemporaryDirectory::new("cli-chat-session-id-second");
    initialize_project(first.path());
    initialize_project(second.path());
    let first_config = write_config(first.path(), "http://127.0.0.1:1/v1");
    let second_config = write_config(second.path(), "http://127.0.0.1:1/v1");

    let (first_process_id, first_output) =
        run_chat_with_pid(first.path(), &first_config, "/exit\n", &[]);
    let (second_process_id, second_output) =
        run_chat_with_pid(second.path(), &second_config, "/exit\n", &[]);
    assert!(first_output.status.success());
    assert!(second_output.status.success());
    assert_ne!(first_process_id, second_process_id);

    let first_session = session_id(first.path());
    let second_session = session_id(second.path());
    assert_session_id_binds_process(&first_session, first_process_id);
    assert_session_id_binds_process(&second_session, second_process_id);
    assert_ne!(first_session, second_session);
}

#[test]
fn chat_loopback_streams_text_and_persists_started_history_audit_and_explicit_resume() {
    let project = TemporaryDirectory::new("cli-chat-loopback");
    initialize_project(project.path());
    let first_server =
        spawn_responses_server(vec![(200, text_response("source answer", "response-1"))]);
    let config = write_config(project.path(), &first_server.base_url);

    let first = run_chat(project.path(), &config, "inspect source\n/exit\n", &[]);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let first_output = String::from_utf8(first.stdout).unwrap();
    assert!(first_output.contains("Started session"));
    assert!(first_output.contains("assistant> source answer"));
    assert!(first_output.contains("remains active for explicit resume"));
    let first_request = first_server.requests.recv().unwrap();
    first_server.join();
    let first_request: Value =
        serde_json::from_str(first_request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(first_request["model"], "fixture-text-model");
    assert!(
        first_request["instructions"]
            .as_str()
            .unwrap()
            .contains("Source domain profile v1")
    );

    let session = session_id(project.path());
    let first_events = session_events(project.path(), &session);
    assert!(
        first_events
            .iter()
            .any(|event| event["event_type"] == "session.started")
    );
    assert_eq!(
        first_events
            .iter()
            .filter(|event| event["event_type"] == "session.history_item_recorded")
            .count(),
        2
    );
    assert_eq!(
        first_events
            .iter()
            .filter(|event| event["event_type"] == "session.request_audit_recorded")
            .count(),
        1
    );

    let second_server =
        spawn_responses_server(vec![(200, text_response("mobile answer", "response-2"))]);
    let resumed_config = write_config(project.path(), &second_server.base_url);
    let resumed = run_chat(
        project.path(),
        &resumed_config,
        "inspect mobile\n/exit\n",
        &["--session", &session, "--domain", "mobile"],
    );
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    let resumed_output = String::from_utf8(resumed.stdout).unwrap();
    assert!(resumed_output.contains("Resumed session"));
    assert!(resumed_output.contains("assistant> mobile answer"));
    let resumed_request = second_server.requests.recv().unwrap();
    second_server.join();
    let resumed_request: Value =
        serde_json::from_str(resumed_request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert!(
        resumed_request["instructions"]
            .as_str()
            .unwrap()
            .contains("Mobile domain profile v1")
    );
    assert!(resumed_request["previous_response_id"].is_null());

    let resumed_events = session_events(project.path(), &session);
    assert!(
        resumed_events
            .iter()
            .any(|event| event["event_type"] == "session.resumed")
    );
    assert_eq!(
        resumed_events
            .iter()
            .filter(|event| event["event_type"] == "session.history_item_recorded")
            .count(),
        4
    );
    assert_eq!(
        resumed_events
            .iter()
            .filter(|event| event["event_type"] == "session.request_audit_recorded")
            .count(),
        2
    );
    assert!(
        project
            .path()
            .join(".cyber-agent/artifacts/blobs/sha256")
            .is_dir()
    );
}

#[test]
fn new_chat_writes_one_default_phase_before_its_first_provider_request() {
    let project = TemporaryDirectory::new("cli-chat-default-phase");
    initialize_project(project.path());
    let server = spawn_responses_server(vec![
        (200, text_response("first answer", "phase-first")),
        (200, text_response("second answer", "phase-second")),
    ]);
    let config = write_config(project.path(), &server.base_url);

    let output = run_chat(project.path(), &config, "first\nsecond\n/exit\n", &[]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let first_request = server.requests.recv().unwrap();
    let second_request = server.requests.recv().unwrap();
    server.join();
    assert!(first_request.starts_with("POST /v1/responses HTTP/1.1"));
    assert!(second_request.starts_with("POST /v1/responses HTTP/1.1"));

    let events = session_events(project.path(), &session_id(project.path()));
    let phases = events
        .iter()
        .filter(|event| event["event_type"] == "session.research_phase_selected")
        .collect::<Vec<_>>();
    assert_eq!(phases.len(), 1);
    assert_eq!(
        phases[0]["payload"],
        serde_json::json!({
            "schema_version": 1,
            "selection": {
                "phase": "mapping",
                "phase_generation": 1,
                "source": "default",
                "reason_code": "NEW_SESSION_DEFAULT_MAPPING"
            }
        })
    );
}

#[test]
fn legacy_resume_keeps_replay_read_only_then_writes_legacy_mapping_on_real_turn() {
    let project = TemporaryDirectory::new("cli-chat-legacy-phase");
    initialize_project(project.path());
    let inert_config = write_config(project.path(), "http://127.0.0.1:1/v1");
    let created = run_chat(project.path(), &inert_config, "/exit\n", &[]);
    assert!(created.status.success());
    let session = session_id(project.path());
    assert!(
        session_events(project.path(), &session)
            .iter()
            .all(|event| event["event_type"] != "session.research_phase_selected")
    );

    let server = spawn_responses_server(vec![(
        200,
        text_response("legacy mapping answer", "legacy-phase"),
    )]);
    let config = write_config(project.path(), &server.base_url);
    let resumed = run_chat(
        project.path(),
        &config,
        "resume legacy\n/exit\n",
        &["--session", &session],
    );
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    let request = server.requests.recv().unwrap();
    server.join();
    assert!(request.starts_with("POST /v1/responses HTTP/1.1"));

    let phases = session_events(project.path(), &session)
        .into_iter()
        .filter(|event| event["event_type"] == "session.research_phase_selected")
        .collect::<Vec<_>>();
    assert_eq!(phases.len(), 1);
    assert_eq!(
        phases[0]["payload"],
        serde_json::json!({
            "schema_version": 1,
            "selection": {
                "phase": "mapping",
                "phase_generation": 1,
                "source": "default",
                "reason_code": "LEGACY_DEFAULT_MAPPING"
            }
        })
    );
}

#[test]
fn explicit_phase_command_records_operator_source_and_reason_before_the_turn() {
    let project = TemporaryDirectory::new("cli-chat-operator-phase");
    initialize_project(project.path());
    let server = spawn_responses_server(vec![(
        200,
        text_response("verification answer", "operator-phase"),
    )]);
    let config = write_config(project.path(), &server.base_url);

    let output = run_chat(
        project.path(),
        &config,
        "/phase verification\nverify boundary\n/exit\n",
        &[],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    server.requests.recv().unwrap();
    server.join();

    let events = session_events(project.path(), &session_id(project.path()));
    let phase = events
        .iter()
        .find(|event| event["event_type"] == "session.research_phase_selected")
        .unwrap();
    assert_eq!(
        phase["payload"],
        serde_json::json!({
            "schema_version": 1,
            "selection": {
                "phase": "verification",
                "phase_generation": 1,
                "source": "operator",
                "reason_code": "OPERATOR_SELECTED"
            }
        })
    );
}

#[test]
fn durable_receipt_survives_a_later_provider_failure_without_a_stale_projection_duplicate() {
    let project = TemporaryDirectory::new("cli-chat-receipt-after-provider-failure");
    initialize_project(project.path());
    let tool_call = concat!(
        "data: {\"type\":\"response.output_item.done\",\"item\":{",
        "\"type\":\"function_call\",\"call_id\":\"research-get\",",
        "\"name\":\"research_get\",\"arguments\":\"{}\"}}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"response-tool\"}}\n\n"
    );
    let failed = concat!(
        "data: {\"type\":\"response.failed\",\"response\":{\"error\":{",
        "\"code\":\"rate_limit_exceeded\",\"message\":\"loopback failure\"}}}\n\n"
    );
    let server =
        spawn_responses_server(vec![(200, tool_call.to_owned()), (200, failed.to_owned())]);
    let config = write_config(project.path(), &server.base_url);

    let output = run_chat(project.path(), &config, "read research\n", &[]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("loopback failure"));
    let first_request = server.requests.recv().unwrap();
    let second_request = server.requests.recv().unwrap();
    server.join();
    assert!(first_request.starts_with("POST /v1/responses HTTP/1.1"));
    assert!(second_request.starts_with("POST /v1/responses HTTP/1.1"));

    let session = session_id(project.path());
    let events = session_events(project.path(), &session);
    assert_eq!(
        events
            .iter()
            .filter(|event| event["event_type"] == "tool.execution_receipt_recorded")
            .count(),
        1,
        "the cache-owned source Receipt must be replayed and deduplicated"
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event["event_type"] == "tool.result_cache_hit")
            .count(),
        0
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event["event_type"] == "session.request_audit_recorded")
            .count(),
        2,
        "both actual Provider attempts must have durable RequestAudit facts"
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event["event_type"] == "session.history_item_recorded")
            .count(),
        3,
        "the user input, Tool Call and Tool Result committed before the later Provider failure remain durable"
    );
    let history = events
        .iter()
        .filter(|event| event["event_type"] == "session.history_item_recorded")
        .map(|event| &event["payload"]["item"])
        .collect::<Vec<_>>();
    assert_eq!(history[0]["kind"], "text");
    assert_eq!(history[0]["role"], "user");
    assert_eq!(history[0]["text"], "read research");
    assert_eq!(history[1]["kind"], "tool_call");
    assert_eq!(history[1]["call"]["call_id"], "research-get");
    assert_eq!(history[2]["kind"], "tool_result");
    assert_eq!(history[2]["result"]["call_id"], "research-get");
}

#[test]
fn rejected_incremental_text_is_not_presented_before_the_full_fallback() {
    let project = TemporaryDirectory::new("cli-chat-incremental-fallback");
    initialize_project(project.path());
    let rejected_incremental = concat!(
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"discarded incremental text\"}\n\n",
        "data: {\"type\":\"response.failed\",\"response\":{\"error\":{",
        "\"code\":\"previous_response_not_found\",\"message\":\"loopback rejection\"}}}\n\n"
    );
    let server = spawn_responses_server(vec![
        (200, text_response("first answer", "response-first")),
        (200, rejected_incremental.to_owned()),
        (200, text_response("full fallback", "response-fallback")),
    ]);
    let config = write_config(project.path(), &server.base_url);

    let output = run_chat(project.path(), &config, "first\nsecond\n/exit\n", &[]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = String::from_utf8(output.stdout).unwrap();
    assert!(output.contains("assistant> first answer"));
    assert!(output.contains("assistant> full fallback"));
    assert!(
        !output.contains("discarded incremental text"),
        "a rejected incremental attempt must not leak partial text before its Full fallback"
    );

    let requests = (0..3)
        .map(|_| server.requests.recv().unwrap())
        .map(|request| {
            serde_json::from_str::<Value>(request.split("\r\n\r\n").nth(1).unwrap()).unwrap()
        })
        .collect::<Vec<_>>();
    server.join();
    assert!(requests[0]["previous_response_id"].is_null());
    assert_eq!(requests[1]["previous_response_id"], "response-first");
    assert!(requests[2]["previous_response_id"].is_null());
}

#[test]
fn chat_runtime_lease_rejects_a_second_process_for_the_same_project() {
    let project = TemporaryDirectory::new("cli-chat-runtime-lease-same-project");
    initialize_project(project.path());
    let config = write_config(project.path(), "http://127.0.0.1:1/v1");
    let first = spawn_waiting_chat(project.path(), &config);

    let second = run_chat(project.path(), &config, "/exit\n", &[]);
    assert!(!second.status.success());
    assert!(String::from_utf8_lossy(&second.stderr).contains("Chat runtime is already active"));
    assert_eq!(
        fs::read_dir(project.path().join(".cyber-agent/sessions"))
            .unwrap()
            .count(),
        1,
        "the rejected process must not start a second Session"
    );
    first.exit();
}

#[test]
fn chat_runtime_leases_are_isolated_between_projects() {
    let left = TemporaryDirectory::new("cli-chat-runtime-lease-left");
    let right = TemporaryDirectory::new("cli-chat-runtime-lease-right");
    initialize_project(left.path());
    initialize_project(right.path());
    let left_config = write_config(left.path(), "http://127.0.0.1:1/v1");
    let right_config = write_config(right.path(), "http://127.0.0.1:1/v1");

    let left_chat = spawn_waiting_chat(left.path(), &left_config);
    let right_chat = spawn_waiting_chat(right.path(), &right_config);
    left_chat.exit();
    right_chat.exit();
}

#[test]
fn ctrl_c_cancels_an_active_request_and_keeps_the_session_resumable() {
    let project = TemporaryDirectory::new("cli-chat-ctrl-c");
    initialize_project(project.path());
    let server = spawn_pending_responses_server();
    let config = write_config(project.path(), &server.base_url);
    let mut child = Command::new(env!("CARGO_BIN_EXE_cyber-agent"))
        .current_dir(project.path())
        .args(["chat", "--config", config.to_str().unwrap()])
        .env("CYBER_AGENT_PRIMARY_API_KEY", "loopback-primary-secret")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"wait for cancellation\n/exit\n")
        .unwrap();
    let request = server
        .requests
        // The complete Gate runs several process-heavy integration tests in
        // parallel; allow scheduling delay without weakening the assertion.
        .recv_timeout(Duration::from_secs(15))
        .unwrap();
    assert!(request.starts_with("POST /v1/responses HTTP/1.1"));
    let signal_status = Command::new("/bin/kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(signal_status.success());

    let output = child.wait_with_output().unwrap();
    server.join();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = String::from_utf8(output.stdout).unwrap();
    assert!(output.contains("[request cancelled]"));
    assert!(output.contains("remains active for explicit resume"));
    let session = session_id(project.path());
    let events = session_events(project.path(), &session);
    assert_eq!(
        events
            .iter()
            .filter(|event| event["event_type"] == "session.history_item_recorded")
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event["event_type"] == "session.request_audit_recorded")
            .count(),
        1
    );
}
