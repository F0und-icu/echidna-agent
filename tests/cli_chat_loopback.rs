#[path = "cli_chat_cases/agent_definitions.rs"]
mod agent_definitions;
#[path = "cli_chat_cases/agent_evolution.rs"]
mod agent_evolution;
#[path = "cli_chat_cases/argument_regeneration.rs"]
mod argument_regeneration;
#[path = "cli_chat_cases/automatic_compaction.rs"]
mod automatic_compaction;
#[path = "cli_chat_cases/command_diagnostics.rs"]
mod command_diagnostics;
#[path = "cli_chat_cases/core_memory_continuation.rs"]
mod core_memory_continuation;
#[path = "cli_chat_cases/ctf_child_checkpoint.rs"]
mod ctf_child_checkpoint;
#[path = "cli_chat_cases/ctf_context.rs"]
mod ctf_context;
#[path = "cli_chat_cases/ctf_output.rs"]
mod ctf_output;
#[path = "cli_chat_cases/ctf_progress.rs"]
mod ctf_progress;
#[path = "cli_chat_cases/ctf_recovery.rs"]
mod ctf_recovery;
#[path = "cli_chat_cases/ctf_resume.rs"]
mod ctf_resume;
#[path = "cli_chat_cases/current_fact_guidance.rs"]
mod current_fact_guidance;
#[path = "cli_chat_cases/delivery_measurement.rs"]
mod delivery_measurement;
#[path = "cli_chat_cases/environment_guidance.rs"]
mod environment_guidance;
#[path = "cli_chat_cases/f4_output.rs"]
mod f4_output;
#[path = "cli_chat_cases/f5_output.rs"]
mod f5_output;
#[path = "cli_chat_cases/f6a_geometry.rs"]
mod f6a_geometry;
#[path = "cli_chat_cases/mcp_http.rs"]
mod mcp_http;
#[path = "cli_chat_cases/operator_steering.rs"]
mod operator_steering;
#[path = "cli_chat_cases/output_budget.rs"]
mod output_budget;
#[path = "support/provider_config.rs"]
mod provider_config;
#[path = "cli_chat_cases/provider_usage.rs"]
mod provider_usage;
mod support;
#[path = "cli_chat_cases/tool_activation_continuation.rs"]
mod tool_activation_continuation;
#[path = "cli_chat_cases/tool_argument_integrity.rs"]
mod tool_argument_integrity;
#[path = "cli_chat_cases/transport.rs"]
mod transport;
#[path = "cli_chat_cases/window_budgets.rs"]
mod window_budgets;

#[test]
fn final_text_integrity_reaches_cli_and_history_without_committing_conflicts() {
    for suffix in ["prefix complete", "conflict"] {
        let root = TemporaryDirectory::new("final-text-integrity");
        initialize_project(root.path());
        let mut events = vec![
            json!({"type":"response.output_text.delta","output_index":0,"content_index":0,"delta":"prefix"}),
            json!({"type":"response.output_text.done","output_index":0,"content_index":0,"text":suffix}),
            json!({"type":"response.completed","response":{"id":"final-text-response"}}),
        ];
        if suffix == "conflict" {
            events.insert(
                1,
                json!({"type":"response.output_item.done","output_index":1,
                "item":{"type":"function_call","call_id":"must-not-execute","name":"run_command",
                    "arguments":"{\"argv\":[\"/usr/bin/printf\",\"must-not-execute\"]}"}}),
            );
        }
        let body = events
            .iter()
            .map(|event| format!("data: {event}\n\n"))
            .collect();
        let server = spawn_responses_server(vec![(200, body)]);
        let config = write_config(root.path(), &server.base_url);
        let output = run_chat(root.path(), &config, "reply once\n/exit\n", &[]);
        server
            .requests
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
        assert!(server.requests.try_recv().is_err());
        server.join();
        let project = Project::discover(root.path()).unwrap();
        let state = reduced_session(&project, &session_id(root.path()));
        let successful = suffix == "prefix complete";
        assert_eq!(
            output.status.success(),
            successful,
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            state
                .history()
                .items()
                .iter()
                .filter(|item| matches!(
                    item,
                    echidna::domain::model::ContentItem::Text {
                        role: echidna::domain::model::MessageRole::Assistant,
                        ..
                    }
                ))
                .count(),
            usize::from(successful)
        );
        if successful {
            assert!(String::from_utf8_lossy(&output.stdout).contains("prefix complete"));
            assert_eq!(
                state.history().items().last().unwrap(),
                &echidna::domain::model::ContentItem::assistant_text(suffix)
            );
        }
        assert_eq!(state.request_audits().len(), 1);
        assert_eq!(state.execution_receipt_count(), 0);
    }
}

use std::collections::BTreeSet;
use std::fs;
use std::io::{BufRead, BufReader, Cursor, Read, Write};
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use echidna::adapters::project_event_store::{Project, ProjectEventStream};
use echidna::domain::artifacts::ArtifactId;
use echidna::domain::events::{EventEnvelope, EventId, EventType, OccurredAt, StreamId};
use echidna::domain::request_audit::REQUEST_AUDIT_MANIFEST_SCHEMA_VERSION;
use echidna::domain::research::{
    EvidenceLink, EvidenceRef, EvidenceRelation, ExecutionReceiptId, ExperimentCapsuleDraft,
    ExperimentDraft, ExperimentId, ExperimentRecord, ExperimentResult, FindingDraft, FindingId,
    FindingRecord, FindingStatus, HypothesisDraft, HypothesisId, HypothesisRecord,
    HypothesisStatus, NextActionDraft, NextActionId, NextActionRecord, NextActionStatus,
    ResearchEvent, ResearchRecordRef, ResearchReducer, TargetKind, TargetSnapshotDraft,
    TargetSnapshotId, TargetSnapshotRecord,
};
use echidna::domain::session::{
    CompactionValidationContext, EvidenceSpine, NarrativeSummary, ResearchStreamIdentity,
    SessionEvent, SessionReducer, SessionState, SessionSummaryDraft,
};
use echidna::ports::{EventSink, EventSource};
use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use support::TemporaryDirectory;

const INLINE_API_KEY_FIXTURE: &str = "loopback-inline-api-key";
const M4_LONG_FLOW_MANIFEST_SHA256: &str =
    "0196cd31b0d366dd4f1755708ed1874db5d6faf0e006482bf502722242bc7730";
const F2A_WAIT: Duration = Duration::from_secs(15);

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

struct F2aPendingVisionServer {
    base_url: String,
    requests: Receiver<String>,
    release: mpsc::Sender<()>,
    thread: JoinHandle<()>,
}

struct F2aFirstResponseThenPendingServer {
    base_url: String,
    requests: Receiver<String>,
    release: mpsc::Sender<()>,
    thread: JoinHandle<()>,
}

impl F2aPendingVisionServer {
    fn shutdown(self) -> Vec<String> {
        let _ = self.release.send(());
        let deadline = Instant::now() + F2A_WAIT;
        while !self.thread.is_finished() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(
            self.thread.is_finished(),
            "F2a pending Vision server did not stop within {} seconds",
            F2A_WAIT.as_secs()
        );
        self.thread
            .join()
            .expect("F2a pending Vision server should not panic");
        self.requests.try_iter().collect()
    }
}

impl F2aFirstResponseThenPendingServer {
    fn shutdown(self) -> Vec<String> {
        let _ = self.release.send(());
        let deadline = Instant::now() + F2A_WAIT;
        while !self.thread.is_finished() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(
            self.thread.is_finished(),
            "F2a first-response pending server did not stop within {} seconds",
            F2A_WAIT.as_secs()
        );
        self.thread
            .join()
            .expect("F2a first-response pending server should not panic");
        self.requests.try_iter().collect()
    }
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
    spawn_responses_server_with_lengths(
        responses
            .into_iter()
            .map(|(status, body)| {
                let length = body.len();
                (status, body, length)
            })
            .collect(),
    )
}

fn spawn_responses_server_with_lengths(responses: Vec<(u16, String, usize)>) -> LoopbackServer {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (sender, receiver) = mpsc::channel();
    let thread = thread::spawn(move || {
        for (status, body, length) in responses {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let request = read_http_request(&mut socket);
            sender.send(request).unwrap();
            let reason = if status == 200 { "OK" } else { "Error" };
            let headers = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                length
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

/// Holds one or more loopback requests open until the test explicitly
/// releases them. The listener and release wait are both bounded so a failed
/// child cannot leave a Gate thread blocked forever.
fn spawn_f2a_pending_vision_server() -> F2aPendingVisionServer {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let (request_sender, requests) = mpsc::channel();
    let (release, release_receiver) = mpsc::channel();
    let thread = thread::spawn(move || {
        let deadline = Instant::now() + F2A_WAIT;
        let mut sockets = Vec::new();
        loop {
            if release_receiver.try_recv().is_ok() || Instant::now() >= deadline {
                return;
            }
            match listener.accept() {
                Ok((mut socket, _)) => {
                    socket.set_nonblocking(false).unwrap();
                    socket
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    request_sender.send(read_http_request(&mut socket)).unwrap();
                    sockets.push(socket);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("F2a pending Vision server accept failed: {error}"),
            }
        }
    });
    F2aPendingVisionServer {
        base_url: format!("http://{address}/v1"),
        requests,
        release,
        thread,
    }
}

/// Delivers one completed Vision response and then deliberately leaves the
/// second request pending until the fixture releases it. This isolates
/// cancellation after a durable first observation without inventing a second
/// Provider result.
fn spawn_f2a_first_response_then_pending_server(
    first_body: String,
) -> F2aFirstResponseThenPendingServer {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let (request_sender, requests) = mpsc::channel();
    let (release, release_receiver) = mpsc::channel();
    let thread = thread::spawn(move || {
        let deadline = Instant::now() + F2A_WAIT;
        let mut received = 0;
        loop {
            if release_receiver.try_recv().is_ok() || Instant::now() >= deadline {
                return;
            }
            match listener.accept() {
                Ok((mut socket, _)) => {
                    socket.set_nonblocking(false).unwrap();
                    socket
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    request_sender.send(read_http_request(&mut socket)).unwrap();
                    if received == 0 {
                        let headers = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            first_body.len()
                        );
                        socket.write_all(headers.as_bytes()).unwrap();
                        socket.write_all(first_body.as_bytes()).unwrap();
                        socket.flush().unwrap();
                    } else {
                        let _ = release_receiver.recv_timeout(F2A_WAIT);
                        return;
                    }
                    received += 1;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("F2a first-response pending server accept failed: {error}"),
            }
        }
    });
    F2aFirstResponseThenPendingServer {
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

fn write_context_window_config(
    directory: &std::path::Path,
    base_url: &str,
    context_window: u32,
) -> std::path::PathBuf {
    let config = directory.join(format!("provider-context-{context_window}.toml"));
    fs::write(
        &config,
        provider_config::fixture()
            .replace("http://127.0.0.1:1/v1", base_url)
            .replacen(
                "context_window = 32768",
                &format!("context_window = {context_window}"),
                1,
            ),
    )
    .unwrap();
    config
}

fn write_dual_config(
    directory: &std::path::Path,
    primary_base_url: &str,
    vision_base_url: &str,
) -> std::path::PathBuf {
    let config = directory.join("provider-dual-config.toml");
    let fixture = provider_config::fixture()
        .replacen("http://127.0.0.1:1/v1", primary_base_url, 1)
        .replacen("http://127.0.0.1:1/v1", vision_base_url, 1);
    fs::write(&config, fixture).unwrap();
    config
}

fn write_direct_config(directory: &std::path::Path, primary_base_url: &str) -> std::path::PathBuf {
    let config = directory.join("provider-direct-config.toml");
    let fixture = provider_config::fixture()
        .replacen("http://127.0.0.1:1/v1", primary_base_url, 1)
        .replacen(
            "vision_mode = \"secondary_tool\"",
            "vision_mode = \"primary_direct\"",
            1,
        )
        .replacen("image_input = false", "image_input = true", 1);
    fs::write(&config, fixture).unwrap();
    config
}

fn write_disabled_config(directory: &std::path::Path) -> std::path::PathBuf {
    let config = directory.join("provider-disabled-config.toml");
    fs::write(
        &config,
        provider_config::fixture().replacen(
            "vision_mode = \"secondary_tool\"",
            "vision_mode = \"disabled\"",
            1,
        ),
    )
    .unwrap();
    config
}

fn write_no_image_capability_config(directory: &std::path::Path) -> std::path::PathBuf {
    let config = directory.join("provider-no-image-config.toml");
    fs::write(
        &config,
        provider_config::fixture()
            .replacen(
                "vision_mode = \"secondary_tool\"",
                "vision_mode = \"auto\"",
                1,
            )
            .replacen("\nvision = \"vision\"", "", 1)
            .replacen("image_input = true", "image_input = false", 1),
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
    let output = Command::new(env!("CARGO_BIN_EXE_echidna"))
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
    let mut command = Command::new(env!("CARGO_BIN_EXE_echidna"));
    command
        .current_dir(project)
        .args(["chat", "--config", config.to_str().unwrap()])
        .args(extra_args)
        .env("CYBER_AGENT_PRIMARY_API_KEY", "loopback-primary-secret")
        .env("CYBER_AGENT_VISION_API_KEY", "loopback-vision-secret")
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

fn spawn_f2a_chat(
    project: &std::path::Path,
    config: &std::path::Path,
    input: &str,
    extra_args: &[&str],
) -> Child {
    let mut child = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .current_dir(project)
        .args(["chat", "--config", config.to_str().unwrap()])
        .args(extra_args)
        .env("CYBER_AGENT_PRIMARY_API_KEY", "loopback-primary-secret")
        .env("CYBER_AGENT_VISION_API_KEY", "loopback-vision-secret")
        .env("OPENAI_API_KEY", "must-not-be-read")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child
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
    let mut child = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .current_dir(project)
        .args(["chat", "--config", config.to_str().unwrap()])
        .env("CYBER_AGENT_PRIMARY_API_KEY", "loopback-primary-secret")
        .env("CYBER_AGENT_VISION_API_KEY", "loopback-vision-secret")
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
    let mut entries = session_ids(project);
    assert_eq!(entries.len(), 1);
    entries.pop().unwrap()
}

fn session_ids(project: &std::path::Path) -> Vec<String> {
    let sessions = project.join(".echidna/sessions");
    let mut entries = fs::read_dir(sessions)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect::<Vec<_>>();
    entries.sort();
    entries
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
            .join(".echidna/sessions")
            .join(session)
            .join("events.jsonl"),
    )
    .unwrap()
    .lines()
    .map(|line| serde_json::from_str(line).unwrap())
    .collect()
}

fn text_response(text: &str, response_id: &str) -> String {
    let text = serde_json::to_string(text).unwrap();
    format!(
        "data: {{\"type\":\"response.output_text.delta\",\"delta\":{text}}}\n\n\
         data: {{\"type\":\"response.completed\",\"response\":{{\"id\":\"{response_id}\"}}}}\n\n"
    )
}

fn vision_response(summary: &str, response_id: &str) -> String {
    text_response(
        &serde_json::to_string(&json!({
            "schema_version": 2,
            "summary": summary,
            "extracted_text": "fixture screenshot",
            "regions": [],
            "task_observations": ["fixture boundary marker"],
            "uncertainty": ["synthetic input only"]
        }))
        .unwrap(),
        response_id,
    )
}

fn png_bytes(width: u32, height: u32) -> Vec<u8> {
    let image = RgbaImage::from_pixel(width, height, Rgba([11, 29, 47, 255]));
    let mut bytes = Vec::new();
    DynamicImage::ImageRgba8(image)
        .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
        .unwrap();
    bytes
}

fn request_body(request: &str) -> Value {
    serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap()
}

fn f2a_request_body(receiver: &Receiver<String>, label: &str) -> Value {
    let request = receiver.recv_timeout(F2A_WAIT).unwrap_or_else(|error| {
        panic!("timed out waiting {F2A_WAIT:?} for F2a {label} request: {error}")
    });
    request_body(&request)
}

fn f2a_join_server(server: LoopbackServer, label: &str) {
    let LoopbackServer { thread, .. } = server;
    let deadline = Instant::now() + F2A_WAIT;
    while !thread.is_finished() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        thread.is_finished(),
        "F2a {label} loopback server did not stop within {} seconds",
        F2A_WAIT.as_secs()
    );
    thread
        .join()
        .unwrap_or_else(|_| panic!("F2a {label} loopback server panicked"));
}

fn f2a_wait_child(child: Child, label: &str) -> std::process::Output {
    wait_child_before_deadline(child, label, F2A_WAIT)
}

fn wait_child_before_deadline(
    mut child: Child,
    label: &str,
    timeout: Duration,
) -> std::process::Output {
    let deadline = Instant::now() + timeout;
    // Drain both pipes while waiting: a long Tool loop must not deadlock the
    // child on the test harness's pipe capacity before its deadline.
    let stdout = child.stdout.take().map(|mut stdout| {
        thread::spawn(move || {
            let mut bytes = Vec::new();
            stdout.read_to_end(&mut bytes).unwrap();
            bytes
        })
    });
    let stderr = child.stderr.take().map(|mut stderr| {
        thread::spawn(move || {
            let mut bytes = Vec::new();
            stderr.read_to_end(&mut bytes).unwrap();
            bytes
        })
    });
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                child.wait().unwrap();
                return std::process::Output {
                    status,
                    stdout: stdout
                        .map(|reader| reader.join().unwrap())
                        .unwrap_or_default(),
                    stderr: stderr
                        .map(|reader| reader.join().unwrap())
                        .unwrap_or_default(),
                };
            }
            Ok(None) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(20));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                // Preserve the already-drained fixture output only for the
                // existing deadline panic, so a future timeout identifies the
                // last observed child progress without changing its deadline
                // or cleanup sequence.
                let stdout = stdout
                    .map(|reader| reader.join().unwrap())
                    .unwrap_or_default();
                let stderr = stderr
                    .map(|reader| reader.join().unwrap())
                    .unwrap_or_default();
                panic!(
                    "F2a {label} child did not exit within {} seconds\nstdout:\n{}\nstderr:\n{}",
                    timeout.as_secs(),
                    String::from_utf8_lossy(&stdout),
                    String::from_utf8_lossy(&stderr),
                );
            }
            Err(error) => panic!("failed to poll F2a {label} child: {error}"),
        }
    }
}

fn json_contains_type(value: &Value, expected: &str) -> bool {
    match value {
        Value::Array(values) => values
            .iter()
            .any(|value| json_contains_type(value, expected)),
        Value::Object(values) => {
            values.get("type").and_then(Value::as_str) == Some(expected)
                || values
                    .values()
                    .any(|value| json_contains_type(value, expected))
        }
        _ => false,
    }
}

fn json_count_type(value: &Value, expected: &str) -> usize {
    match value {
        Value::Array(values) => values
            .iter()
            .map(|value| json_count_type(value, expected))
            .sum(),
        Value::Object(values) => {
            usize::from(values.get("type").and_then(Value::as_str) == Some(expected))
                + values
                    .values()
                    .map(|value| json_count_type(value, expected))
                    .sum::<usize>()
        }
        _ => 0,
    }
}

fn json_contains_text(value: &Value, expected: &str) -> bool {
    match value {
        Value::String(value) => value.contains(expected),
        Value::Array(values) => values
            .iter()
            .any(|value| json_contains_text(value, expected)),
        Value::Object(values) => values
            .values()
            .any(|value| json_contains_text(value, expected)),
        _ => false,
    }
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

fn append_research_snapshot(project: &Project, label: &str) -> TargetSnapshotId {
    append_research_snapshot_with_artifacts(project, label, Vec::new())
}

fn append_research_snapshot_with_artifacts(
    project: &Project,
    label: &str,
    target_artifact_ids: Vec<ArtifactId>,
) -> TargetSnapshotId {
    let snapshot_id = TargetSnapshotId::new(format!("snapshot-{label}"));
    let record = TargetSnapshotRecord::from_draft(TargetSnapshotDraft {
        id: snapshot_id.clone(),
        target_id: "fixture.target".to_owned(),
        kind: TargetKind::Source,
        locator: format!("src/{label}"),
        version: "1.0.0".to_owned(),
        build_id: format!("build-{label}"),
        target_artifact_ids,
        environment_fingerprint: "fixture-environment".to_owned(),
        device_fingerprint: "fixture-device".to_owned(),
        captured_at: "2026-08-31T00:00:00Z".to_owned(),
        notes: String::new(),
    })
    .unwrap();
    let event = ResearchEvent::TargetSnapshotRecorded(record);
    let mut writer = project.open_writer().unwrap();
    let mut store = writer.open_research(None).unwrap();
    let envelope = EventEnvelope::current(
        EventId::new(format!("research-{label}")),
        ProjectEventStream::Research.stream_id(),
        store.next_sequence().unwrap(),
        OccurredAt::new("2026-08-31T00:00:00Z"),
        EventType::new(event.event_type().as_str()),
        event.encode_payload().unwrap(),
    );
    EventSink::append_event(&mut store, &envelope).unwrap();
    snapshot_id
}

fn append_research_event(project: &Project, label: &str, event: ResearchEvent) {
    let mut writer = project.open_writer().unwrap();
    let mut store = writer.open_research(None).unwrap();
    let envelope = EventEnvelope::current(
        EventId::new(format!("research-{label}")),
        ProjectEventStream::Research.stream_id(),
        store.next_sequence().unwrap(),
        OccurredAt::new(format!("2026-08-31T00:01:{label}Z")),
        EventType::new(event.event_type().as_str()),
        event.encode_payload().unwrap(),
    );
    EventSink::append_event(&mut store, &envelope).unwrap();
}

fn append_v2_compaction(project: &Project, session: &str, target: TargetSnapshotId) {
    let research_events = {
        let mut writer = project.open_writer().unwrap();
        let store = writer.open_research(None).unwrap();
        EventSource::read_events(&store, &ProjectEventStream::Research.stream_id()).unwrap()
    };
    let research_stream =
        ResearchStreamIdentity::from_events(project.project_id().clone(), &research_events)
            .unwrap();
    let research_state = ResearchReducer.reduce(research_events).unwrap();
    let receipt_ids = BTreeSet::new();
    let validation = CompactionValidationContext::new(
        project.project_id(),
        &research_stream,
        &research_state,
        &receipt_ids,
    );
    let draft = SessionSummaryDraft::new(
        EvidenceSpine::new(
            research_stream.clone(),
            vec![target],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap(),
        NarrativeSummary::new(
            "resume fixture",
            "reference-only summary",
            Vec::new(),
            Vec::new(),
        )
        .unwrap(),
    )
    .unwrap();
    let state = {
        let session_id = StreamId::new(session);
        let stream_id = ProjectEventStream::Session(session_id.clone()).stream_id();
        let mut writer = project.open_writer().unwrap();
        let store = writer.open_session(session_id, None).unwrap();
        SessionReducer
            .reduce(EventSource::read_events(&store, &stream_id).unwrap())
            .unwrap()
    };
    let compaction = state.next_compaction(draft, 0, &validation).unwrap();

    let session_id = StreamId::new(session);
    let stream_id = ProjectEventStream::Session(session_id.clone()).stream_id();
    let mut writer = project.open_writer().unwrap();
    let mut store = writer.open_session(session_id, None).unwrap();
    let sequence = store.next_sequence().unwrap();
    let envelope = SessionEvent::CompactionCreated { compaction }
        .into_envelope(
            EventId::new("cli-compaction"),
            stream_id,
            sequence,
            OccurredAt::new("2026-08-31T00:00:01Z"),
        )
        .unwrap();
    EventSink::append_event(&mut store, &envelope).unwrap();
}

fn m4_long_flow_tool_response(
    call_id: &str,
    tool_name: &str,
    arguments: Value,
    response_id: &str,
) -> String {
    let item = json!({
        "type": "function_call",
        "call_id": call_id,
        "name": tool_name,
        "arguments": serde_json::to_string(&arguments).unwrap(),
    });
    format!(
        "data: {{\"type\":\"response.output_item.done\",\"item\":{}}}\n\n\
         data: {{\"type\":\"response.completed\",\"response\":{{\"id\":\"{response_id}\"}}}}\n\n",
        serde_json::to_string(&item).unwrap(),
    )
}

fn tool_calls_response(calls: &[(&str, &str, Value)], response_id: &str) -> String {
    let mut response = String::new();
    for (call_id, tool_name, arguments) in calls {
        let item = json!({
            "type": "function_call",
            "call_id": call_id,
            "name": tool_name,
            "arguments": serde_json::to_string(arguments).unwrap(),
        });
        response.push_str(&format!(
            "data: {{\"type\":\"response.output_item.done\",\"item\":{}}}\n\n",
            serde_json::to_string(&item).unwrap(),
        ));
    }
    response.push_str(&format!(
        "data: {{\"type\":\"response.completed\",\"response\":{{\"id\":\"{response_id}\"}}}}\n\n"
    ));
    response
}

fn reduced_session(project: &Project, session: &str) -> SessionState {
    let session_id = StreamId::new(session);
    let stream_id = ProjectEventStream::Session(session_id.clone()).stream_id();
    let mut writer = project.open_writer().unwrap();
    let store = writer.open_session(session_id, None).unwrap();
    SessionReducer
        .reduce(EventSource::read_events(&store, &stream_id).unwrap())
        .unwrap()
}

fn assert_audits_reconstruct(project_root: &std::path::Path, state: &SessionState) {
    for manifest in state.request_audits() {
        assert_eq!(
            manifest.schema_version(),
            REQUEST_AUDIT_MANIFEST_SCHEMA_VERSION
        );
        let renderer = manifest.prompt_renderer().unwrap();
        assert_eq!(renderer.id(), "generic");
        assert_eq!(renderer.version(), 1);
        assert!(manifest.resolved_tool_plan_fingerprint().is_some());
        assert!(manifest.context_delivery_map().is_some());
        manifest
            .reconstruct(|blob| {
                fs::read(
                    project_root
                        .join(".echidna/artifacts/blobs/sha256")
                        .join(blob.as_str()),
                )
            })
            .unwrap();
    }
}

#[test]
fn f2a_secondary_cli_image_is_observed_before_primary_tool_loop_and_remains_auditable() {
    let root = TemporaryDirectory::new("f2a-cli-secondary-image");
    initialize_project(root.path());
    let project = Project::discover(root.path()).unwrap();
    append_research_snapshot(&project, "older");
    let current_target = append_research_snapshot(&project, "current");
    fs::write(root.path().join("screen.png"), png_bytes(16, 12)).unwrap();
    fs::write(
        root.path().join("target.txt"),
        "synthetic target boundary\n",
    )
    .unwrap();

    let vision = spawn_responses_server(vec![(
        200,
        vision_response("secondary fixture observation", "vision-secondary"),
    )]);
    let primary = spawn_responses_server(vec![
        (
            200,
            m4_long_flow_tool_response(
                "f2a-read-secondary",
                "read_file",
                json!({"path": "target.txt"}),
                "primary-secondary-tool",
            ),
        ),
        (
            200,
            text_response("F2A_SECONDARY_COMPLETE", "primary-secondary-final"),
        ),
    ]);
    let config = write_dual_config(root.path(), &primary.base_url, &vision.base_url);

    fs::write(
        &config,
        fs::read_to_string(&config)
            .unwrap()
            .replacen(
                "[capability_mappings.text-v1]",
                "[capability_mappings.text-v1]\nmax_output_tokens = 4096",
                1,
            )
            .replacen(
                "[capability_mappings.vision-v1]",
                "[capability_mappings.vision-v1]\nmax_output_tokens = 1024",
                1,
            ),
    )
    .unwrap();

    let output = run_chat(
        root.path(),
        &config,
        "/image screen.png\nInspect the synthetic image, then call read_file for target.txt before answering.\n/exit\n",
        &[],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Queued image"));
    assert!(stdout.contains(current_target.as_str()));
    assert!(stdout.contains("F2A_SECONDARY_COMPLETE"));

    let vision_request = f2a_request_body(&vision.requests, "secondary Vision");
    f2a_join_server(vision, "secondary Vision");
    assert_eq!(vision_request["model"], "fixture-vision-model");
    assert_eq!(vision_request["max_output_tokens"], 1024);
    assert!(json_contains_type(&vision_request["input"], "input_image"));
    assert!(json_contains_text(
        &vision_request["input"],
        "Inspect the synthetic image"
    ));

    let first_primary = f2a_request_body(&primary.requests, "secondary Primary first");
    let second_primary = f2a_request_body(&primary.requests, "secondary Primary second");
    f2a_join_server(primary, "secondary Primary");
    assert_eq!(first_primary["model"], "fixture-text-model");
    assert_eq!(first_primary["max_output_tokens"], 4096);
    assert_eq!(second_primary["max_output_tokens"], 4096);
    assert!(!json_contains_type(&first_primary["input"], "input_image"));
    assert!(json_contains_text(
        &first_primary["input"],
        "secondary fixture observation"
    ));
    assert_eq!(
        second_primary["previous_response_id"],
        "primary-secondary-tool"
    );
    assert!(json_contains_type(
        &second_primary["input"],
        "function_call_output"
    ));

    let session = session_id(root.path());
    let state = reduced_session(&project, &session);
    assert_eq!(state.vision_routes().len(), 1);
    assert_eq!(state.vision_observations().len(), 1);
    assert_ne!(
        state.primary_provider_session().unwrap().id,
        state.vision_observations()[0].provider_session.id
    );
    assert_eq!(state.request_audits().len(), 3);
    assert_audits_reconstruct(root.path(), &state);
    for (manifest, expected) in state.request_audits().iter().zip([1024, 4096, 4096]) {
        let input = manifest
            .reconstruct(|blob| {
                fs::read(
                    root.path()
                        .join(".echidna/artifacts/blobs/sha256")
                        .join(blob.as_str()),
                )
            })
            .unwrap();
        assert_eq!(input.requirements.max_output_tokens, Some(expected));
    }
    let events = session_events(root.path(), &session);
    assert_eq!(
        events
            .iter()
            .filter(
                |event| event["event_type"] == "tool.execution_outcome_recorded"
                    && event["payload"]["result"]["call_id"] == "f2a-read-secondary"
            )
            .count(),
        1,
        "the model result and its Receipt are durable in one fact"
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event["event_type"] == "tool.execution_receipt_recorded")
            .count(),
        1,
        "operator view_image and model read_file must each keep a real Receipt"
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event["event_type"] == "session.history_item_recorded")
            .count(),
        5
    );
    let research = fs::read_to_string(root.path().join(".echidna/research.jsonl")).unwrap();
    assert!(research.contains(current_target.as_str()));
    assert!(!research.contains("finding.recorded"));
}

#[test]
fn f2a_primary_direct_cli_image_reaches_only_primary_and_keeps_real_tool_receipt() {
    let root = TemporaryDirectory::new("f2a-cli-primary-direct");
    initialize_project(root.path());
    let project = Project::discover(root.path()).unwrap();
    append_research_snapshot(&project, "direct");
    fs::write(root.path().join("screen.png"), png_bytes(9, 7)).unwrap();
    fs::write(root.path().join("target.txt"), "direct fixture target\n").unwrap();

    let primary = spawn_responses_server(vec![
        (
            200,
            m4_long_flow_tool_response(
                "f2a-read-direct",
                "read_file",
                json!({"path": "target.txt"}),
                "primary-direct-tool",
            ),
        ),
        (
            200,
            text_response("F2A_DIRECT_COMPLETE", "primary-direct-final"),
        ),
    ]);
    let config = write_direct_config(root.path(), &primary.base_url);
    let output = run_chat(
        root.path(),
        &config,
        "/image screen.png\nInspect the direct image, then call read_file for target.txt before answering.\n/exit\n",
        &[],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("F2A_DIRECT_COMPLETE")
    );

    let first_primary = f2a_request_body(&primary.requests, "direct Primary first");
    let second_primary = f2a_request_body(&primary.requests, "direct Primary second");
    f2a_join_server(primary, "direct Primary");
    assert!(json_contains_type(&first_primary["input"], "input_image"));
    assert_eq!(
        second_primary["previous_response_id"],
        "primary-direct-tool"
    );
    assert!(!json_contains_type(&second_primary["input"], "input_image"));

    let session = session_id(root.path());
    let state = reduced_session(&project, &session);
    assert_eq!(state.vision_routes().len(), 1);
    assert_eq!(
        state.vision_routes()[0].route,
        echidna::domain::profile::VisionRoute::PrimaryDirect
    );
    assert_eq!(state.vision_observations().len(), 0);
    assert_eq!(state.request_audits().len(), 2);
    assert_audits_reconstruct(root.path(), &state);
    let events = session_events(root.path(), &session);
    assert_eq!(
        events
            .iter()
            .filter(
                |event| event["event_type"] == "tool.execution_outcome_recorded"
                    && event["payload"]["result"]["call_id"] == "f2a-read-direct"
            )
            .count(),
        1,
        "the model result and its Receipt are durable in one fact"
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event["event_type"] == "tool.execution_receipt_recorded")
            .count(),
        1
    );
}

#[test]
fn ec_k4_two_primary_direct_images_keep_order_without_duplicate_history_and_survive_compaction_resume()
 {
    let root = TemporaryDirectory::new("ec-k4-two-direct-images");
    initialize_project(root.path());
    let project = Project::discover(root.path()).unwrap();
    append_research_snapshot(&project, "two-direct");
    fs::write(root.path().join("first.png"), png_bytes(9, 7)).unwrap();
    fs::write(root.path().join("second.png"), png_bytes(13, 5)).unwrap();

    let initial = spawn_responses_server(vec![(
        200,
        text_response("TWO_DIRECT_COMPLETE", "two-direct-initial"),
    )]);
    let config = write_direct_config(root.path(), &initial.base_url);
    let initial_task = format!(
        "Compare both attached images in order. {}",
        "stable historical context ".repeat(1024)
    );
    let output = run_chat(
        root.path(),
        &config,
        &format!("/image first.png\n/image second.png\n{initial_task}\n/exit\n"),
        &[],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let request = f2a_request_body(&initial.requests, "two direct Primary");
    f2a_join_server(initial, "two direct Primary");
    assert_eq!(json_count_type(&request["input"], "input_image"), 2);

    let session = session_id(root.path());
    let state = reduced_session(&project, &session);
    assert_eq!(state.vision_routes().len(), 2);
    assert!(state.vision_observations().is_empty());
    assert_eq!(state.request_audits().len(), 1);
    let route_sources = state
        .vision_routes()
        .iter()
        .map(|route| route.source_artifact_ids.as_slice())
        .collect::<Vec<_>>();
    assert_eq!(route_sources.len(), 2);
    assert_eq!(route_sources[0].len(), 1);
    assert_eq!(route_sources[1].len(), 1);
    assert_ne!(route_sources[0], route_sources[1]);
    let direct_history = state
        .history()
        .items()
        .iter()
        .filter_map(|item| match item {
            echidna::domain::model::ContentItem::ImageArtifact { artifact_id, .. } => {
                Some(artifact_id)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(direct_history.len(), 2);
    assert_ne!(direct_history[0], direct_history[1]);
    let events = session_events(root.path(), &session);
    assert_eq!(
        events
            .iter()
            .filter(|event| event["event_type"] == "tool.execution_receipt_recorded")
            .count(),
        2
    );

    let resumed_server = spawn_responses_server(vec![(
        200,
        text_response("TWO_DIRECT_RESUMED", "two-direct-resumed"),
    )]);
    let resumed_config = write_direct_config(root.path(), &resumed_server.base_url);
    let resumed = run_chat(
        root.path(),
        &resumed_config,
        "/compact\nContinue from the compacted two-image history.\n/exit\n",
        &["--session", &session],
    );
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    assert!(String::from_utf8_lossy(&resumed.stdout).contains("Compaction revision 1 applied"));
    let resumed_request = f2a_request_body(&resumed_server.requests, "two direct resumed Primary");
    f2a_join_server(resumed_server, "two direct resumed Primary");
    assert!(json_contains_text(
        &resumed_request["input"],
        "Continue from the compacted two-image history."
    ));
    let resumed_state = reduced_session(&project, &session);
    assert!(resumed_state.latest_compaction().is_some());
    assert_eq!(resumed_state.vision_routes().len(), 2);
    assert!(resumed_state.vision_observations().is_empty());
    assert_eq!(
        resumed_state
            .history()
            .items()
            .iter()
            .filter(|item| matches!(
                item,
                echidna::domain::model::ContentItem::ImageArtifact { .. }
            ))
            .count(),
        2,
        "compaction and Resume retain the actual durable History projection; they do not replay image intake"
    );
    assert_eq!(
        session_events(root.path(), &session)
            .iter()
            .filter(|event| event["event_type"] == "tool.execution_receipt_recorded")
            .count(),
        2,
        "Resume must not create fresh image-intake executions"
    );
}

#[test]
fn ec_k4_two_secondary_images_make_ordered_distinct_observations_before_one_primary_request() {
    let root = TemporaryDirectory::new("ec-k4-two-secondary-images");
    initialize_project(root.path());
    let project = Project::discover(root.path()).unwrap();
    append_research_snapshot(&project, "two-secondary");
    fs::write(root.path().join("first.png"), png_bytes(16, 12)).unwrap();
    fs::write(root.path().join("second.png"), png_bytes(7, 15)).unwrap();

    let vision = spawn_responses_server(vec![
        (
            200,
            vision_response("first queued observation", "vision-first"),
        ),
        (
            200,
            vision_response("second queued observation", "vision-second"),
        ),
    ]);
    let primary = spawn_responses_server(vec![(
        200,
        text_response("TWO_SECONDARY_COMPLETE", "two-secondary-primary"),
    )]);
    let config = write_dual_config(root.path(), &primary.base_url, &vision.base_url);
    let output = run_chat(
        root.path(),
        &config,
        "/image first.png\n/image second.png\nCompare the two queued secondary images.\n/exit\n",
        &[],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let first_vision = f2a_request_body(&vision.requests, "first queued Vision");
    let second_vision = f2a_request_body(&vision.requests, "second queued Vision");
    f2a_join_server(vision, "two queued Vision");
    assert_eq!(json_count_type(&first_vision["input"], "input_image"), 1);
    assert_eq!(json_count_type(&second_vision["input"], "input_image"), 1);
    assert_ne!(first_vision["input"], second_vision["input"]);
    let primary_request = f2a_request_body(&primary.requests, "two queued Primary");
    f2a_join_server(primary, "two queued Primary");
    assert_eq!(json_count_type(&primary_request["input"], "input_image"), 0);
    assert!(json_contains_text(
        &primary_request["input"],
        "first queued observation"
    ));
    assert!(json_contains_text(
        &primary_request["input"],
        "second queued observation"
    ));

    let session = session_id(root.path());
    let state = reduced_session(&project, &session);
    assert_eq!(state.vision_routes().len(), 2);
    assert_eq!(state.vision_observations().len(), 2);
    assert_eq!(state.request_audits().len(), 3);
    let first_geometry = state.vision_observations()[0]
        .image_geometry
        .as_ref()
        .unwrap();
    let second_geometry = state.vision_observations()[1]
        .image_geometry
        .as_ref()
        .unwrap();
    assert_eq!(
        (first_geometry.source_width, first_geometry.source_height),
        (16, 12)
    );
    assert_eq!(
        (second_geometry.source_width, second_geometry.source_height),
        (7, 15)
    );
    assert_ne!(
        state.vision_observations()[0].source_artifact_id,
        state.vision_observations()[1].source_artifact_id
    );
    assert_eq!(
        state
            .history()
            .items()
            .iter()
            .filter(|item| matches!(
                item,
                echidna::domain::model::ContentItem::ImageArtifact { .. }
            ))
            .count(),
        0,
        "secondary input contributes only durable observations to Primary History"
    );
    assert_audits_reconstruct(root.path(), &state);
}

#[test]
fn f2a_model_view_image_follow_up_runs_secondary_before_the_next_primary_sample() {
    let root = TemporaryDirectory::new("f2a-cli-model-view-image-follow-up");
    initialize_project(root.path());
    let project = Project::discover(root.path()).unwrap();
    append_research_snapshot(&project, "follow-up");
    fs::write(root.path().join("screen.png"), png_bytes(10, 8)).unwrap();

    let vision = spawn_responses_server(vec![(
        200,
        vision_response("model follow-up observation", "vision-follow-up"),
    )]);
    let primary = spawn_responses_server(vec![
        (
            200,
            m4_long_flow_tool_response(
                "f2a-view-follow-up",
                "view_image",
                json!({
                    "event_id": "f2a-follow-up-artifact-event",
                    "occurred_at": "2026-08-31T01:00:00Z",
                    "artifact_id": "artifact-f2a-follow-up",
                    "source_path": "screen.png",
                    "target_snapshot_id": "snapshot-follow-up",
                    "toolchain_fingerprint": "f2a-loopback"
                }),
                "primary-follow-up-tool",
            ),
        ),
        (
            200,
            text_response("F2A_FOLLOW_UP_COMPLETE", "primary-follow-up-final"),
        ),
    ]);
    let config = write_dual_config(root.path(), &primary.base_url, &vision.base_url);
    let output = run_chat(
        root.path(),
        &config,
        "Call view_image for screen.png, then use the observation and answer.\n/exit\n",
        &["--domain", "mobile"],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("F2A_FOLLOW_UP_COMPLETE")
    );

    let first_primary = f2a_request_body(&primary.requests, "follow-up Primary first");
    let second_primary = f2a_request_body(&primary.requests, "follow-up Primary second");
    f2a_join_server(primary, "follow-up Primary");
    let vision_request = f2a_request_body(&vision.requests, "follow-up Vision");
    f2a_join_server(vision, "follow-up Vision");
    assert!(!json_contains_type(&first_primary["input"], "input_image"));
    assert!(json_contains_type(&vision_request["input"], "input_image"));
    assert!(json_contains_text(
        &second_primary["input"],
        "model follow-up observation"
    ));
    assert!(json_contains_type(
        &second_primary["input"],
        "function_call_output"
    ));

    let session = session_id(root.path());
    let state = reduced_session(&project, &session);
    assert_eq!(state.vision_routes().len(), 1);
    assert_eq!(state.vision_observations().len(), 1);
    assert_eq!(state.request_audits().len(), 3);
    assert_audits_reconstruct(root.path(), &state);
    assert_eq!(
        session_events(root.path(), &session)
            .iter()
            .filter(|event| event["event_type"] == "tool.execution_outcome_recorded")
            .count(),
        1
    );
}

#[test]
fn f2a_image_commands_reject_invalid_inputs_before_any_provider_request_or_route() {
    let missing = TemporaryDirectory::new("f2a-cli-image-missing-target");
    initialize_project(missing.path());
    fs::write(missing.path().join("screen.png"), png_bytes(4, 4)).unwrap();
    let missing_config = write_config(missing.path(), "http://127.0.0.1:1/v1");
    let output = run_chat(
        missing.path(),
        &missing_config,
        "/image screen.png\n/exit\n",
        &[],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("requires at least one recorded TargetSnapshot")
    );
    let missing_events = session_events(missing.path(), &session_id(missing.path()));
    assert!(missing_events.iter().all(|event| {
        !matches!(
            event["event_type"].as_str(),
            Some("tool.execution_receipt_recorded")
                | Some("session.vision_route_decided")
                | Some("session.request_audit_recorded")
        )
    }));

    let disabled = TemporaryDirectory::new("f2a-cli-image-disabled");
    initialize_project(disabled.path());
    let disabled_project = Project::discover(disabled.path()).unwrap();
    append_research_snapshot(&disabled_project, "disabled");
    fs::write(disabled.path().join("screen.png"), png_bytes(4, 4)).unwrap();
    let disabled_config = write_disabled_config(disabled.path());
    let output = run_chat(
        disabled.path(),
        &disabled_config,
        "/image screen.png\n/exit\n",
        &[],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("VISION_DISABLED")
    );
    assert_eq!(
        session_events(disabled.path(), &session_id(disabled.path()))
            .iter()
            .filter(|event| event["event_type"] == "tool.execution_receipt_recorded")
            .count(),
        0
    );

    let unavailable = TemporaryDirectory::new("f2a-cli-image-unavailable");
    initialize_project(unavailable.path());
    let unavailable_project = Project::discover(unavailable.path()).unwrap();
    append_research_snapshot(&unavailable_project, "unavailable");
    fs::write(unavailable.path().join("screen.png"), png_bytes(4, 4)).unwrap();
    let unavailable_config = write_no_image_capability_config(unavailable.path());
    let output = run_chat(
        unavailable.path(),
        &unavailable_config,
        "/image screen.png\n/exit\n",
        &[],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("NO_IMAGE_CAPABLE_PROVIDER")
    );

    let corrupt = TemporaryDirectory::new("f2a-cli-image-corrupt");
    initialize_project(corrupt.path());
    let corrupt_project = Project::discover(corrupt.path()).unwrap();
    append_research_snapshot(&corrupt_project, "corrupt");
    fs::write(corrupt.path().join("corrupt.png"), b"not a PNG").unwrap();
    let corrupt_config = write_config(corrupt.path(), "http://127.0.0.1:1/v1");
    let output = run_chat(
        corrupt.path(),
        &corrupt_config,
        "/image corrupt.png\n/exit\n",
        &[],
    );
    assert!(output.status.success());
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("view_image_invalid")
    );
    assert_eq!(
        session_events(corrupt.path(), &session_id(corrupt.path()))
            .iter()
            .filter(|event| event["event_type"] == "tool.execution_receipt_recorded")
            .count(),
        1,
        "a started failed view_image execution keeps its real Receipt"
    );

    let oversized = TemporaryDirectory::new("f2a-cli-image-oversized");
    initialize_project(oversized.path());
    let oversized_project = Project::discover(oversized.path()).unwrap();
    append_research_snapshot(&oversized_project, "oversized");
    fs::File::create(oversized.path().join("oversized.png"))
        .unwrap()
        .set_len(32 * 1024 * 1024 + 1)
        .unwrap();
    let oversized_config = write_config(oversized.path(), "http://127.0.0.1:1/v1");
    let output = run_chat(
        oversized.path(),
        &oversized_config,
        "/image oversized.png\n/exit\n",
        &[],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("33554432"),
        "unexpected oversized-image diagnostic: {stdout}"
    );
    assert_eq!(
        session_events(oversized.path(), &session_id(oversized.path()))
            .iter()
            .filter(|event| event["event_type"] == "tool.execution_receipt_recorded")
            .count(),
        1
    );
}

#[test]
fn ec_k4_queued_but_unsubmitted_images_are_not_reconstructed_after_restart() {
    let root = TemporaryDirectory::new("f2a-cli-image-pending-resume");
    initialize_project(root.path());
    let project = Project::discover(root.path()).unwrap();
    append_research_snapshot(&project, "pending");
    fs::write(root.path().join("first.png"), png_bytes(5, 5)).unwrap();
    fs::write(root.path().join("second.png"), png_bytes(6, 6)).unwrap();
    let inert_config = write_config(root.path(), "http://127.0.0.1:1/v1");
    let first = run_chat(
        root.path(),
        &inert_config,
        "/image first.png\n/image second.png\n/compact\n/exit\n",
        &[],
    );
    assert!(first.status.success());
    let first_stdout = String::from_utf8(first.stdout).unwrap();
    assert_eq!(first_stdout.matches("Queued image").count(), 2);
    assert!(first_stdout.contains("Compaction is unavailable while 2 image(s)"));
    let session = session_id(root.path());
    assert_eq!(
        session_events(root.path(), &session)
            .iter()
            .filter(|event| event["event_type"] == "tool.execution_receipt_recorded")
            .count(),
        2
    );

    let primary = spawn_responses_server(vec![(
        200,
        text_response("F2A_RESUME_WITHOUT_IMAGE", "resume-without-image"),
    )]);
    let resumed_config = write_dual_config(root.path(), &primary.base_url, "http://127.0.0.1:1/v1");
    let resumed = run_chat(
        root.path(),
        &resumed_config,
        "Continue without a reconstructed image.\n/exit\n",
        &["--session", &session],
    );
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    let request = f2a_request_body(&primary.requests, "resume Primary");
    f2a_join_server(primary, "resume Primary");
    assert!(!json_contains_type(&request["input"], "input_image"));
    let state = reduced_session(&project, &session);
    assert!(state.vision_routes().is_empty());
    assert!(state.vision_observations().is_empty());
}

#[test]
fn ec_k4_second_secondary_image_failure_keeps_durable_prefix_without_primary_history() {
    let root = TemporaryDirectory::new("ec-k4-second-secondary-failure");
    initialize_project(root.path());
    let project = Project::discover(root.path()).unwrap();
    append_research_snapshot(&project, "second-secondary-failure");
    fs::write(root.path().join("first.png"), png_bytes(8, 6)).unwrap();
    fs::write(root.path().join("second.png"), png_bytes(6, 8)).unwrap();
    let vision = spawn_responses_server(vec![
        (
            200,
            vision_response("durable first observation", "failure-first"),
        ),
        (503, "second Vision failure".to_owned()),
    ]);
    let primary = spawn_f2a_pending_vision_server();
    let config = write_dual_config(root.path(), &primary.base_url, &vision.base_url);
    let output = run_chat(
        root.path(),
        &config,
        "/image first.png\n/image second.png\nSubmit both images even if the second Vision request fails.\n/exit\n",
        &[],
    );
    assert!(
        !output.status.success(),
        "a recoverable stopped Turn must require explicit Resume"
    );
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("Initial images/task were not submitted to Primary")
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("unresolved stopped Turn"));
    let _ = f2a_request_body(&vision.requests, "first secondary failure Vision");
    let _ = f2a_request_body(&vision.requests, "second secondary failure Vision");
    f2a_join_server(vision, "second secondary failure Vision");
    assert!(
        primary.shutdown().is_empty(),
        "Primary must not start after a later initial-image failure"
    );

    let state = reduced_session(&project, &session_id(root.path()));
    assert_eq!(state.vision_routes().len(), 2);
    assert_eq!(state.vision_observations().len(), 1);
    assert_eq!(state.request_audits().len(), 2);
    assert!(state.history().items().is_empty());
    let events = session_events(root.path(), &session_id(root.path()));
    assert_eq!(
        events
            .iter()
            .filter(|event| event["event_type"] == "vision.analysis_failed")
            .count(),
        1
    );
}

#[test]
fn ec_k4_shared_budget_stops_between_secondary_images_without_primary_submission() {
    let root = TemporaryDirectory::new("ec-k4-second-secondary-budget");
    initialize_project(root.path());
    let project = Project::discover(root.path()).unwrap();
    append_research_snapshot(&project, "second-secondary-budget");
    fs::write(root.path().join("first.png"), png_bytes(8, 6)).unwrap();
    fs::write(root.path().join("second.png"), png_bytes(6, 8)).unwrap();
    let vision = spawn_responses_server(vec![(
        200,
        vision_response("budget first observation", "budget-first"),
    )]);
    let primary = spawn_f2a_pending_vision_server();
    let config = write_dual_config(root.path(), &primary.base_url, &vision.base_url);
    fs::write(
        &config,
        format!(
            "{}\n[runtime]\nmodel_request_limit_per_turn = 1\n",
            fs::read_to_string(&config).unwrap()
        ),
    )
    .unwrap();
    let output = run_chat(
        root.path(),
        &config,
        "/image first.png\n/image second.png\nSubmit both images under the shared request budget.\n/exit\n",
        &[],
    );
    assert!(
        !output.status.success(),
        "a budget-stopped Turn must require explicit Resume"
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("per-Turn model-request limit of 1"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("unresolved stopped Turn"));
    let _ = f2a_request_body(&vision.requests, "budget first Vision");
    f2a_join_server(vision, "budget first Vision");
    assert!(
        primary.shutdown().is_empty(),
        "the second initial image must not dispatch Vision or Primary after budget exhaustion"
    );

    let state = reduced_session(&project, &session_id(root.path()));
    assert_eq!(state.vision_routes().len(), 2);
    assert_eq!(state.vision_observations().len(), 1);
    assert_eq!(state.request_audits().len(), 1);
    assert!(state.history().items().is_empty());
    assert!(
        !session_events(root.path(), &session_id(root.path()))
            .iter()
            .any(|event| event["event_type"] == "vision.analysis_failed")
    );
}

#[test]
fn ec_k4_second_secondary_image_cancellation_keeps_prefix_without_primary_or_resume_replay() {
    let root = TemporaryDirectory::new("ec-k4-second-secondary-cancel");
    initialize_project(root.path());
    let project = Project::discover(root.path()).unwrap();
    append_research_snapshot(&project, "second-secondary-cancel");
    fs::write(root.path().join("first.png"), png_bytes(8, 6)).unwrap();
    fs::write(root.path().join("second.png"), png_bytes(6, 8)).unwrap();
    let vision = spawn_f2a_first_response_then_pending_server(vision_response(
        "cancel first observation",
        "cancel-first",
    ));
    let primary = spawn_f2a_pending_vision_server();
    let config = write_dual_config(root.path(), &primary.base_url, &vision.base_url);
    let child = spawn_f2a_chat(
        root.path(),
        &config,
        "/image first.png\n/image second.png\nCancel after the second initial Vision request begins.\n",
        &[],
    );
    let first_request = f2a_request_body(&vision.requests, "first cancellation Vision");
    let second_request = f2a_request_body(&vision.requests, "second cancellation Vision");
    assert_eq!(json_count_type(&first_request["input"], "input_image"), 1);
    assert_eq!(json_count_type(&second_request["input"], "input_image"), 1);
    let signal_status = Command::new("/bin/kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .unwrap();
    let _ = vision.shutdown();
    let output = f2a_wait_child(child, "second initial Vision cancellation");
    assert!(signal_status.success());
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("[vision request cancelled; no Primary request was started]")
    );
    assert!(
        primary.shutdown().is_empty(),
        "Primary must not start while a later initial image is cancelled"
    );

    let session = session_id(root.path());
    let state = reduced_session(&project, &session);
    assert_eq!(state.vision_routes().len(), 2);
    assert_eq!(state.vision_observations().len(), 1);
    assert_eq!(state.request_audits().len(), 2);
    assert!(state.history().items().is_empty());
    let events_before_resume = session_events(root.path(), &session);
    assert_eq!(
        events_before_resume
            .iter()
            .filter(|event| event["event_type"] == "vision.analysis_failed")
            .count(),
        1
    );

    let inert = write_config(root.path(), "http://127.0.0.1:1/v1");
    let resumed = run_chat(root.path(), &inert, "/exit\n", &["--session", &session]);
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    let events_after_resume = session_events(root.path(), &session);
    for event_type in [
        "vision.route_decided",
        "vision.observation_recorded",
        "vision.analysis_failed",
        "session.request_audit_recorded",
    ] {
        assert_eq!(
            events_after_resume
                .iter()
                .filter(|event| event["event_type"] == event_type)
                .count(),
            events_before_resume
                .iter()
                .filter(|event| event["event_type"] == event_type)
                .count(),
            "Resume must not replay queued image processing for {event_type}"
        );
    }
}

#[test]
fn f2a_secondary_pending_vision_sigint_releases_writer_and_resume_does_not_replay_image() {
    let root = TemporaryDirectory::new("f2a-cli-secondary-vision-cancel");
    initialize_project(root.path());
    let project = Project::discover(root.path()).unwrap();
    append_research_snapshot(&project, "cancel");
    fs::write(root.path().join("screen.png"), png_bytes(8, 8)).unwrap();

    let server = spawn_f2a_pending_vision_server();
    let config = write_dual_config(root.path(), "http://127.0.0.1:2/v1", &server.base_url);
    let child = spawn_f2a_chat(
        root.path(),
        &config,
        "/image screen.png\nInspect this image while the Vision response is pending.\n",
        &[],
    );

    let request = match server.requests.recv_timeout(F2A_WAIT) {
        Ok(request) => request,
        Err(error) => {
            let _ = Command::new("/bin/kill")
                .args(["-TERM", &child.id().to_string()])
                .status();
            let _ = server.shutdown();
            let output = f2a_wait_child(child, "secondary Vision timeout cleanup");
            panic!(
                "timed out waiting {F2A_WAIT:?} for the pending Vision request: {error}\\nstdout:\\n{}\\nstderr:\\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    };
    let request_body = request_body(&request);
    assert_eq!(request_body["model"], "fixture-vision-model");
    assert!(json_contains_type(&request_body["input"], "input_image"));

    // The scoped Vision request is deliberately held open. CLI persistence
    // must not retain the Project Writer lock across that remote await.
    let writer_available = project.open_writer().is_ok();
    let signal_status = Command::new("/bin/kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .unwrap();
    let extra_requests = server.shutdown();
    let output = f2a_wait_child(child, "secondary Vision cancellation");
    assert!(
        signal_status.success(),
        "SIGINT failed for pending Vision child: {signal_status}\\nstdout:\\n{}\\nstderr:\\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        writer_available,
        "Project Writer was unavailable while the pending Vision request was remote-awaiting"
    );
    assert!(
        extra_requests.is_empty(),
        "Primary must not be sampled while the initial Vision request is pending: {extra_requests:?}"
    );

    let session = session_id(root.path());
    let events_before_resume = session_events(root.path(), &session);
    assert_eq!(
        events_before_resume
            .iter()
            .filter(|event| event["event_type"] == "vision.route_decided")
            .count(),
        1
    );
    assert_eq!(
        events_before_resume
            .iter()
            .filter(|event| event["event_type"] == "session.request_audit_recorded")
            .count(),
        1
    );
    assert_eq!(
        events_before_resume
            .iter()
            .filter(|event| event["event_type"] == "vision.observation_recorded")
            .count(),
        0
    );
    let failure = events_before_resume
        .iter()
        .find(|event| event["event_type"] == "vision.analysis_failed")
        .expect("cancelled Vision must leave one terminal failure fact");
    assert_eq!(failure["payload"]["error_code"], "cancelled");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("[vision request cancelled; no Primary request was started]")
    );

    let inert_config = write_config(root.path(), "http://127.0.0.1:1/v1");
    let resumed = {
        let child = spawn_f2a_chat(
            root.path(),
            &inert_config,
            "/exit\n",
            &["--session", &session],
        );
        // The resume must address the existing Session explicitly; use the
        // same bounded process helper as the pending child.
        f2a_wait_child(child, "secondary Vision resume")
    };
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    let events_after_resume = session_events(root.path(), &session);
    assert_eq!(
        events_after_resume
            .iter()
            .filter(|event| event["event_type"] == "vision.route_decided")
            .count(),
        1
    );
    assert_eq!(
        events_after_resume
            .iter()
            .filter(|event| event["event_type"] == "session.request_audit_recorded")
            .count(),
        1
    );
    assert_eq!(
        events_after_resume
            .iter()
            .filter(|event| event["event_type"] == "vision.analysis_failed")
            .count(),
        1
    );
    assert_eq!(
        events_after_resume
            .iter()
            .filter(|event| event["event_type"] == "vision.observation_recorded")
            .count(),
        0
    );
    assert_eq!(
        events_after_resume
            .iter()
            .filter(|event| event["event_type"] == "session.resumed")
            .count(),
        1,
        "Resume may append its one lifecycle fact but must not replay pending image state"
    );
}

#[test]
fn synthetic_long_flow_preserves_authoritative_evidence_across_compaction_and_resume() {
    let fixture_root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/m4_long_flow");
    let manifest_bytes = fs::read(fixture_root.join("manifest.json")).unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(&manifest_bytes)),
        M4_LONG_FLOW_MANIFEST_SHA256,
        "the test anchors the complete fixture manifest to prevent a coordinated source/hash edit"
    );
    let manifest: Value = serde_json::from_slice(&manifest_bytes).unwrap();
    let source = fixture_root.join("target/fixture_router.rs");
    let source_bytes = fs::read(&source).unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(&source_bytes)),
        manifest["source_sha256"].as_str().unwrap(),
        "the manifest binds the harmless source toy by SHA-256"
    );

    let root = TemporaryDirectory::new("cli-chat-m4-s4-long-flow");
    let target_directory = root.path().join("target");
    fs::create_dir_all(&target_directory).unwrap();
    fs::copy(&source, target_directory.join("fixture_router.rs")).unwrap();
    initialize_project(root.path());

    let target_snapshot = json!({
        "event_id": "research-m4-s4-snapshot",
        "occurred_at": "2026-08-31T10:00:00Z",
        "operation": {
            "kind": "record_target_snapshot",
            "record": {
                "id": "snapshot-m4-s4",
                "target_id": "fixture.m4-s4.router",
                "kind": "source",
                "locator": "target/fixture_router.rs",
                "version": "1.0.0",
                "build_id": "m4-s4-fixture",
                "target_artifact_ids": ["artifact-m4-s4"],
                "environment_fingerprint": "offline-fixture-environment",
                "device_fingerprint": "offline-fixture-device",
                "captured_at": "2026-08-31T10:00:00Z",
                "notes": "harmless local source toy"
            }
        }
    });
    let artifact = json!({
        "event_id": "research-m4-s4-artifact",
        "occurred_at": "2026-08-31T10:01:00Z",
        "artifact_id": "artifact-m4-s4",
        "source_path": "target/fixture_router.rs",
        "media_type": "text/x-rust",
        "target_snapshot_id": "snapshot-m4-s4",
        "toolchain_fingerprint": "m4-s4-offline-fixture-v1"
    });
    let hypothesis = json!({
        "event_id": "research-m4-s4-hypothesis",
        "occurred_at": "2026-08-31T10:02:00Z",
        "operation": {
            "kind": "record_hypothesis",
            "record": {
                "id": "hypothesis-m4-s4",
                "title": "Redirect token may reach the internal dispatcher",
                "target_id": "fixture.m4-s4.router",
                "attack_surface": "caller-supplied redirect token",
                "vulnerability_class": "unvalidated internal redirect",
                "entry_point": "route_redirect",
                "sensitive_sink": "internal_dispatch",
                "rationale": "the source toy forwards an unvalidated token to an internal route selector",
                "status": "active",
                "priority": 10,
                "evidence_links": [{
                    "relation": "supports",
                    "evidence": {"kind": "artifact", "id": "artifact-m4-s4"}
                }]
            }
        }
    });
    let experiment = json!({
        "event_id": "research-m4-s4-experiment",
        "occurred_at": "2026-08-31T10:03:00Z",
        "operation": {
            "kind": "record_experiment",
            "experiment": {
                "id": "experiment-m4-s4",
                "hypothesis_id": "hypothesis-m4-s4",
                "target_snapshot_id": "snapshot-m4-s4",
                "capsule_id": "capsule-m4-s4",
                "goal": "read the exact candidate source after recording the hypothesis",
                "result": "inconclusive",
                "artifact_refs": ["artifact-m4-s4"]
            },
            "capsule": {
                "id": "capsule-m4-s4",
                "experiment_id": "experiment-m4-s4",
                "target_snapshot_id": "snapshot-m4-s4",
                "action": "Read harmless fixture redirect dispatch",
                "arguments": {"source_path": "target/fixture_router.rs"},
                "tool_version": "1",
                "execution_ids": ["execution:call-m4-s4-read"],
                "input_artifact_ids": ["artifact-m4-s4"],
                "environment_delta": "offline fixture only",
                "account_prerequisites": [],
                "toolchain_fingerprint": "m4-s4-offline-fixture-v1",
                "execution_receipt_ids": [],
                "output_artifact_ids": []
            }
        }
    });
    let server = spawn_responses_server(vec![
        (
            200,
            m4_long_flow_tool_response(
                "call-m4-s4-snapshot",
                "research_update",
                target_snapshot,
                "response-m4-s4-snapshot",
            ),
        ),
        (
            200,
            m4_long_flow_tool_response(
                "call-m4-s4-artifact",
                "artifact_store",
                artifact,
                "response-m4-s4-artifact",
            ),
        ),
        (
            200,
            m4_long_flow_tool_response(
                "call-m4-s4-hypothesis",
                "research_update",
                hypothesis,
                "response-m4-s4-hypothesis",
            ),
        ),
        (
            200,
            m4_long_flow_tool_response(
                "call-m4-s4-read",
                "read_file",
                json!({"path": "target/fixture_router.rs"}),
                "response-m4-s4-read",
            ),
        ),
        (
            200,
            m4_long_flow_tool_response(
                "call-m4-s4-experiment",
                "research_update",
                experiment,
                "response-m4-s4-experiment",
            ),
        ),
        (
            200,
            text_response(
                "Candidate only: the offline source toy remains unverified.",
                "response-m4-s4-candidate",
            ),
        ),
    ]);
    // This is an evidence/continuation fixture, not a pressure threshold.
    // Leave schema headroom; m5_context_pressure independently proves trimming.
    let config = write_context_window_config(root.path(), &server.base_url, 65_536);
    let run = run_chat(
        root.path(),
        &config,
        "inspect the harmless redirect toy and retain only a candidate\n/exit\n",
        &["--domain", "source", "--phase", "experiment"],
    );
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let run_output = String::from_utf8(run.stdout).unwrap();
    assert!(run_output.contains("Candidate only"));
    assert!(!run_output.to_ascii_lowercase().contains("confirmed"));
    assert!(!run_output.to_ascii_lowercase().contains("reproduced"));
    let requests = (0..6)
        .map(|_| server.requests.recv().unwrap())
        .map(|request| {
            serde_json::from_str::<Value>(request.split("\r\n\r\n").nth(1).unwrap()).unwrap()
        })
        .collect::<Vec<_>>();
    server.join();
    assert!(
        requests[0]["instructions"]
            .as_str()
            .unwrap()
            .contains("Source domain profile v1")
    );
    assert!(
        requests[0]["instructions"]
            .as_str()
            .unwrap()
            .contains("execute the smallest decisive experiment")
    );

    let session = session_id(root.path());
    let project = Project::discover(root.path()).unwrap();
    let research_events = {
        let mut writer = project.open_writer().unwrap();
        let store = writer.open_research(None).unwrap();
        EventSource::read_events(&store, &ProjectEventStream::Research.stream_id()).unwrap()
    };
    let research_state = ResearchReducer.reduce(research_events.clone()).unwrap();
    let research = serde_json::to_value(&research_state).unwrap();
    assert_eq!(
        research["target_snapshots"]["snapshot-m4-s4"]["target_artifact_ids"],
        json!(["artifact-m4-s4"])
    );
    assert_eq!(
        research["hypotheses"]["hypothesis-m4-s4"]["evidence_links"][0]["evidence"],
        json!({"kind": "artifact", "id": "artifact-m4-s4"})
    );
    assert_eq!(
        research["hypotheses"]["hypothesis-m4-s4"]["evidence_links"][0]["relation"],
        "supports"
    );
    assert_eq!(
        research["artifacts"]["records"]["artifact-m4-s4"]["scope"],
        json!({"kind": "target", "target_snapshot_id": "snapshot-m4-s4"})
    );
    assert_eq!(
        research["experiments"]["experiment-m4-s4"]["hypothesis_id"],
        "hypothesis-m4-s4"
    );
    assert_eq!(
        research["experiments"]["experiment-m4-s4"]["target_snapshot_id"],
        "snapshot-m4-s4"
    );
    assert_eq!(
        research["experiments"]["experiment-m4-s4"]["artifact_refs"],
        json!(["artifact-m4-s4"])
    );
    assert_eq!(
        research["capsules"]["capsule-m4-s4"]["execution_ids"],
        json!(["execution:call-m4-s4-read"])
    );
    assert_eq!(
        research["capsules"]["capsule-m4-s4"]["input_artifact_ids"],
        json!(["artifact-m4-s4"])
    );
    assert_eq!(
        research["capsules"]["capsule-m4-s4"]["execution_receipt_ids"],
        json!([])
    );
    assert_eq!(research["findings"], json!({}));

    let before_compaction = reduced_session(&project, &session);
    assert_eq!(
        before_compaction.history().ensure_all_tool_calls_paired(),
        Ok(())
    );
    assert_eq!(before_compaction.execution_receipt_count(), 5);
    assert_eq!(before_compaction.request_audits().len(), 6);
    assert!(before_compaction.vision_observations().is_empty());
    let raw_session_events = session_events(root.path(), &session);
    let receipt_event_ids = raw_session_events
        .iter()
        .filter(|event| event["event_type"] == "tool.execution_outcome_recorded")
        .map(|event| event["event_id"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(receipt_event_ids.len(), 5);
    let artifact_receipt_event_id = raw_session_events
        .iter()
        .find(|event| {
            event["event_type"] == "tool.execution_outcome_recorded"
                && event["payload"]["receipt"]["execution_id"] == "execution:call-m4-s4-artifact"
        })
        .and_then(|event| event["event_id"].as_str())
        .unwrap()
        .to_owned();
    let read_receipt_event_id = raw_session_events
        .iter()
        .find(|event| {
            event["event_type"] == "tool.execution_outcome_recorded"
                && event["payload"]["receipt"]["execution_id"] == "execution:call-m4-s4-read"
        })
        .and_then(|event| event["event_id"].as_str())
        .unwrap()
        .to_owned();
    let artifact_receipt = raw_session_events
        .iter()
        .find(|event| event["event_id"] == artifact_receipt_event_id)
        .unwrap();
    assert_eq!(
        artifact_receipt["payload"]["receipt"]["output_artifact_ids"],
        json!(["artifact-m4-s4"])
    );
    assert_audits_reconstruct(root.path(), &before_compaction);

    let research_stream =
        ResearchStreamIdentity::from_events(project.project_id().clone(), &research_events)
            .unwrap();
    let receipt_refs = receipt_event_ids
        .iter()
        .cloned()
        .map(ExecutionReceiptId::new)
        .collect::<BTreeSet<_>>();
    let validation = CompactionValidationContext::new(
        project.project_id(),
        &research_stream,
        &research_state,
        &receipt_refs,
    );
    let spine = EvidenceSpine::new(
        research_stream.clone(),
        vec![TargetSnapshotId::new("snapshot-m4-s4")],
        vec![HypothesisId::new("hypothesis-m4-s4")],
        vec![ExperimentId::new("experiment-m4-s4")],
        Vec::new(),
        Vec::new(),
        vec![ArtifactId::new("artifact-m4-s4")],
        vec![
            ExecutionReceiptId::new(artifact_receipt_event_id.clone()),
            ExecutionReceiptId::new(read_receipt_event_id.clone()),
        ],
    )
    .unwrap();
    let draft = SessionSummaryDraft::new(
        spine,
        NarrativeSummary::new(
            "retain the offline candidate boundary",
            "the toy source path is a candidate and is not a Finding",
            vec!["external reachability and impact remain unproven".to_owned()],
            vec!["preserve the exact artifact and receipt references".to_owned()],
        )
        .unwrap(),
    )
    .unwrap();
    let compaction = before_compaction
        .next_compaction(draft, 1, &validation)
        .unwrap();
    let session_id = StreamId::new(&session);
    let stream_id = ProjectEventStream::Session(session_id.clone()).stream_id();
    let mut writer = project.open_writer().unwrap();
    let mut store = writer.open_session(session_id, None).unwrap();
    let event = SessionEvent::CompactionCreated { compaction }
        .into_envelope(
            EventId::new("compaction-m4-s4"),
            stream_id,
            store.next_sequence().unwrap(),
            OccurredAt::new("2026-08-31T10:04:00Z"),
        )
        .unwrap();
    EventSink::append_event(&mut store, &event).unwrap();
    drop(store);
    drop(writer);

    let resumed_server = spawn_responses_server(vec![(
        200,
        text_response(
            "Evidence curation keeps the source observation as a candidate.",
            "response-m4-s4-resume",
        ),
    )]);
    let resumed_config = write_config(root.path(), &resumed_server.base_url);
    let resumed = run_chat(
        root.path(),
        &resumed_config,
        "curate the existing evidence without upgrading the candidate\n/exit\n",
        &[
            "--session",
            &session,
            "--domain",
            "source",
            "--phase",
            "evidence-curation",
        ],
    );
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    let resumed_request = resumed_server.requests.recv().unwrap();
    resumed_server.join();
    let resumed_request: Value =
        serde_json::from_str(resumed_request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert!(resumed_request["previous_response_id"].is_null());
    assert!(
        resumed_request["instructions"]
            .as_str()
            .unwrap()
            .contains("curate an auditable evidence record without changing claim truth")
    );
    let resume_wire = serde_json::to_string(&resumed_request).unwrap();
    for reference in [
        "snapshot-m4-s4",
        "hypothesis-m4-s4",
        "experiment-m4-s4",
        "artifact-m4-s4",
        artifact_receipt_event_id.as_str(),
        read_receipt_event_id.as_str(),
    ] {
        assert!(
            resume_wire.contains(reference),
            "resume request lacks {reference}"
        );
    }
    assert!(!resume_wire.contains("response-m4-s4-candidate"));

    let final_state = reduced_session(&project, &session);
    assert_eq!(final_state.history().ensure_all_tool_calls_paired(), Ok(()));
    assert_eq!(final_state.execution_receipt_count(), 5);
    assert_eq!(final_state.request_audits().len(), 7);
    assert!(final_state.vision_observations().is_empty());
    assert_audits_reconstruct(root.path(), &final_state);
    let full_requests = final_state
        .request_audits()
        .iter()
        .filter(|manifest| {
            manifest.delivery().mode()
                == echidna::domain::request_audit::RequestAuditDeliveryMode::Full
        })
        .count();
    assert_eq!(
        full_requests,
        2,
        "delivery decisions: {:?}",
        final_state
            .request_audits()
            .iter()
            .map(|audit| audit.delivery())
            .collect::<Vec<_>>()
    );
    assert_eq!(final_state.request_audits().len() - full_requests, 5);
    assert_eq!(
        final_state
            .request_audits()
            .last()
            .unwrap()
            .delivery()
            .mode(),
        echidna::domain::request_audit::RequestAuditDeliveryMode::Full
    );
    let latest_compaction = final_state.latest_compaction().unwrap();
    let summary = serde_json::to_value(latest_compaction.summary()).unwrap();
    // This fixture deliberately appends the legacy EvidenceSpine-only draft.
    // Reading it must not rewrite it into the new task-reference schema.
    assert_eq!(summary["schema_version"], 2);
    assert_eq!(
        summary["evidence_spine"]["target_snapshot_refs"],
        json!(["snapshot-m4-s4"])
    );
    assert_eq!(
        summary["evidence_spine"]["active_hypothesis_refs"],
        json!(["hypothesis-m4-s4"])
    );
    assert_eq!(
        summary["evidence_spine"]["experiment_refs"],
        json!(["experiment-m4-s4"])
    );
    assert_eq!(
        summary["evidence_spine"]["artifact_refs"],
        json!(["artifact-m4-s4"])
    );
    assert_eq!(
        summary["evidence_spine"]["execution_receipt_refs"],
        json!([artifact_receipt_event_id, read_receipt_event_id])
    );

    let final_events = session_events(root.path(), &session);
    let started_primary = final_events
        .iter()
        .find(|event| event["event_type"] == "session.started_with_agent")
        .and_then(|event| event["payload"]["primary_provider_session"]["id"].as_str())
        .unwrap();
    let resumed_primary = final_events
        .iter()
        .find(|event| event["event_type"] == "session.resumed")
        .and_then(|event| event["payload"]["primary_provider_session"]["id"].as_str())
        .unwrap();
    assert_ne!(started_primary, resumed_primary);

    let metrics = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .current_dir(root.path())
        .args(["metrics", "--session", &session])
        .output()
        .unwrap();
    assert!(
        metrics.status.success(),
        "{}",
        String::from_utf8_lossy(&metrics.stderr)
    );
    let metrics: Value = serde_json::from_slice(&metrics.stdout).unwrap();
    assert_eq!(metrics["status"], "ok");
    assert_eq!(metrics["requests"], 7);
    assert_eq!(metrics["full_requests"], 2);
    assert_eq!(metrics["incremental_requests"], 5);
    assert_eq!(metrics["fallbacks"], 0);
    assert_eq!(metrics["provider_usage"], "unavailable");
    assert_eq!(metrics["compactions"], 1);
    assert!(metrics["estimated_tokens"].as_u64().unwrap() > 0);
}

#[test]
fn append_only_research_evolution_rebuilds_compaction_before_resume() {
    let root = TemporaryDirectory::new("cli-chat-stale-compaction");
    initialize_project(root.path());
    let config = write_config(root.path(), "http://127.0.0.1:1/v1");
    let created = run_chat(root.path(), &config, "/exit\n", &[]);
    assert!(created.status.success());
    let session = session_id(root.path());
    let project = Project::discover(root.path()).unwrap();

    let bound_target = append_research_snapshot(&project, "bound");
    append_v2_compaction(&project, &session, bound_target);
    let current = append_research_snapshot(&project, "newer");
    let before = session_events(root.path(), &session);

    let resumed = run_chat(root.path(), &config, "/exit\n", &["--session", &session]);
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    let after = session_events(root.path(), &session);
    assert_eq!(&after[..before.len()], before.as_slice());
    assert_eq!(
        after[before.len()]["event_type"],
        "session.compaction_created"
    );
    assert_eq!(after[before.len() + 1]["event_type"], "session.resumed");
    assert_eq!(
        after[before.len()]["payload"]["summary"]["evidence_spine"]["target_snapshot_refs"],
        json!([current.as_str()])
    );
    // No stale source is reused and repeating Resume creates no new Summary.
    let again = run_chat(root.path(), &config, "/exit\n", &["--session", &session]);
    assert!(
        again.status.success(),
        "{}",
        String::from_utf8_lossy(&again.stderr)
    );
    let final_events = session_events(root.path(), &session);
    assert_eq!(
        final_events
            .iter()
            .filter(|event| event["event_type"] == "session.compaction_created")
            .count(),
        2
    );
}

#[test]
fn changed_historical_research_prefix_is_rejected_before_resume() {
    let root = TemporaryDirectory::new("cli-chat-tampered-compaction");
    initialize_project(root.path());
    let config = write_config(root.path(), "http://127.0.0.1:1/v1");
    assert!(
        run_chat(root.path(), &config, "/exit\n", &[])
            .status
            .success()
    );
    let session = session_id(root.path());
    let project = Project::discover(root.path()).unwrap();
    let bound = append_research_snapshot(&project, "bound");
    append_v2_compaction(&project, &session, bound);
    append_research_snapshot(&project, "newer");
    let research_path = root.path().join(".echidna/research.jsonl");
    let contents = fs::read_to_string(&research_path).unwrap();
    let mut lines = contents
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    lines[0]["occurred_at"] = json!("tampered-history-timestamp");
    fs::write(
        &research_path,
        lines
            .iter()
            .map(|line| format!("{}\n", serde_json::to_string(line).unwrap()))
            .collect::<String>(),
    )
    .unwrap();
    let before = session_events(root.path(), &session);
    let resumed = run_chat(root.path(), &config, "/exit\n", &["--session", &session]);
    assert!(!resumed.status.success());
    assert!(
        String::from_utf8_lossy(&resumed.stderr).contains("historical Research prefix"),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    let after = session_events(root.path(), &session);
    assert_eq!(after, before);
    assert!(
        after
            .iter()
            .all(|event| event["event_type"] != "session.resumed")
    );
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
        &project.path().join(".echidna"),
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
    assert!(first_output.contains("Echidna> source answer"));
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
            .any(|event| event["event_type"] == "session.started_with_agent")
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
    assert!(resumed_output.contains("Echidna> mobile answer"));
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
            .join(".echidna/artifacts/blobs/sha256")
            .is_dir()
    );
}

#[test]
fn m5_paste_preserves_multiline_input_and_domain_resume_never_drifts_to_source() {
    let project = TemporaryDirectory::new("m5-cli-paste-domain-resume");
    initialize_project(project.path());
    let first_server = spawn_responses_server(vec![(
        200,
        text_response("mobile multiline answer", "m5-paste-first"),
    )]);
    let config = write_config(project.path(), &first_server.base_url);
    let pasted = "map exported component\n/phase verification\nretain exact newline";
    let first = run_chat(
        project.path(),
        &config,
        &format!("/paste\n{pasted}\n/end\n/exit\n"),
        &["--domain", "mobile"],
    );
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(String::from_utf8_lossy(&first.stdout).contains("Paste mode"));
    let first_request = request_body(&first_server.requests.recv().unwrap());
    first_server.join();
    assert!(json_contains_text(&first_request["input"], pasted));
    assert!(
        first_request["instructions"]
            .as_str()
            .unwrap()
            .contains("Mobile domain profile v1")
    );

    let session = session_id(project.path());
    let domain_events = session_events(project.path(), &session)
        .into_iter()
        .filter(|event| event["event_type"] == "session.domain_profile_selected")
        .collect::<Vec<_>>();
    assert_eq!(domain_events.len(), 1);
    assert_eq!(
        domain_events[0]["payload"],
        json!({
            "schema_version": 1,
            "selection": {
                "profile": "mobile",
                "profile_generation": 1,
                "source": "operator",
                "reason_code": "OPERATOR_SELECTED"
            }
        })
    );
    let phase_events = session_events(project.path(), &session)
        .into_iter()
        .filter(|event| event["event_type"] == "session.research_phase_selected")
        .collect::<Vec<_>>();
    assert_eq!(phase_events.len(), 1);
    assert_eq!(phase_events[0]["payload"]["selection"]["phase"], "mapping");

    let second_server = spawn_responses_server(vec![(
        200,
        text_response("restored mobile answer", "m5-paste-resumed"),
    )]);
    let resumed_config = write_config(project.path(), &second_server.base_url);
    let resumed = run_chat(
        project.path(),
        &resumed_config,
        "continue\n/exit\n",
        &["--resume", "latest"],
    );
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    let resumed_request = request_body(&second_server.requests.recv().unwrap());
    second_server.join();
    assert!(
        resumed_request["instructions"]
            .as_str()
            .unwrap()
            .contains("Mobile domain profile v1")
    );
    assert_eq!(
        session_events(project.path(), &session)
            .iter()
            .filter(|event| event["event_type"] == "session.domain_profile_selected")
            .count(),
        1,
        "Resume without an override restores the durable Domain without duplicating it"
    );
}

#[test]
fn m5_production_compaction_applies_one_durable_reference_summary_then_resumes_exactly() {
    let root = TemporaryDirectory::new("m5-production-compaction");
    fs::write(root.path().join("candidate.txt"), "bounded candidate\n").unwrap();
    initialize_project(root.path());
    let project = Project::discover(root.path()).unwrap();
    let old_target_snapshot = append_research_snapshot(&project, "m5-compact-old");
    let target_snapshot = append_research_snapshot_with_artifacts(
        &project,
        "m5-compact",
        vec![
            ArtifactId::new("artifact-m5-compact-old"),
            ArtifactId::new("artifact-m5-compact"),
        ],
    );

    let first_server = spawn_responses_server(vec![
        (
            200,
            m4_long_flow_tool_response(
                "m5-compact-old-artifact",
                "artifact_store",
                json!({
                    "event_id": "artifact-event-m5-compact-old",
                    "occurred_at": "2026-08-31T00:00:00Z",
                    "artifact_id": "artifact-m5-compact-old",
                    "source_path": "candidate.txt",
                    "media_type": "text/plain",
                    "target_snapshot_id": old_target_snapshot.as_str(),
                    "toolchain_fingerprint": "m5-compact-fixture-v1"
                }),
                "m5-compact-old-artifact-response",
            ),
        ),
        (
            200,
            m4_long_flow_tool_response(
                "m5-compact-artifact",
                "artifact_store",
                json!({
                    "event_id": "artifact-event-m5-compact",
                    "occurred_at": "2026-08-31T00:00:01Z",
                    "artifact_id": "artifact-m5-compact",
                    "source_path": "candidate.txt",
                    "media_type": "text/plain",
                    "target_snapshot_id": target_snapshot.as_str(),
                    "toolchain_fingerprint": "m5-compact-fixture-v1"
                }),
                "m5-compact-artifact-response",
            ),
        ),
        (
            200,
            m4_long_flow_tool_response(
                "m5-compact-read",
                "read_file",
                json!({"path": "candidate.txt"}),
                "m5-compact-read-response",
            ),
        ),
        (
            200,
            text_response(
                "The bounded candidate is mapped but remains unverified.",
                "m5-compact-first-final",
            ),
        ),
    ]);
    // This fixture verifies evidence/compaction boundaries, not context
    // pressure. Keep room for the complete bound instructions and scoped
    // Artifact schemas; the separate pressure fixture tests tight windows.
    let first_config = write_context_window_config(root.path(), &first_server.base_url, 65_536);
    let first = run_chat(
        root.path(),
        &first_config,
        "map the bounded candidate without promoting a finding\n/exit\n",
        &["--domain", "mobile", "--phase", "experiment"],
    );
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    for _ in 0..4 {
        first_server.requests.recv().unwrap();
    }
    first_server.join();

    let session = session_id(root.path());
    let first_events = session_events(root.path(), &session);
    let old_artifact_receipt = first_events
        .iter()
        .find(|event| {
            event["event_type"] == "tool.execution_outcome_recorded"
                && event["payload"]["receipt"]["execution_id"]
                    == "execution:m5-compact-old-artifact"
        })
        .and_then(|event| event["event_id"].as_str())
        .map(ExecutionReceiptId::new)
        .unwrap();
    let artifact_receipt = first_events
        .iter()
        .find(|event| {
            event["event_type"] == "tool.execution_outcome_recorded"
                && event["payload"]["receipt"]["execution_id"] == "execution:m5-compact-artifact"
        })
        .and_then(|event| event["event_id"].as_str())
        .map(ExecutionReceiptId::new)
        .unwrap();
    let read_receipt = first_events
        .iter()
        .find(|event| {
            event["event_type"] == "tool.execution_outcome_recorded"
                && event["payload"]["receipt"]["execution_id"] == "execution:m5-compact-read"
        })
        .and_then(|event| event["event_id"].as_str())
        .map(ExecutionReceiptId::new)
        .unwrap();

    let hypothesis = HypothesisRecord::from_draft(HypothesisDraft {
        id: HypothesisId::new("hypothesis-m5-compact"),
        title: "Candidate parser boundary".to_owned(),
        target_id: "fixture.target".to_owned(),
        attack_surface: "bounded fixture".to_owned(),
        vulnerability_class: "input validation".to_owned(),
        entry_point: "candidate input".to_owned(),
        sensitive_sink: "fixture parser".to_owned(),
        rationale: "requires a controlled verification step".to_owned(),
        status: HypothesisStatus::Testing,
        priority: 10,
        evidence_links: vec![EvidenceLink {
            relation: EvidenceRelation::Supports,
            evidence: EvidenceRef::Artifact(ArtifactId::new("artifact-m5-compact")),
        }],
        last_action: None,
        next_action: Some(NextActionId::new("next-action-m5-compact")),
    })
    .unwrap();
    append_research_event(
        &project,
        "m5-compact-hypothesis",
        ResearchEvent::HypothesisRecorded {
            record: hypothesis,
            keep_independent: false,
        },
    );

    let (old_experiment, old_capsule) = ExperimentRecord::from_drafts(
        ExperimentDraft {
            id: ExperimentId::new("experiment-m5-compact-old"),
            hypothesis_id: HypothesisId::new("hypothesis-m5-compact"),
            target_snapshot_id: old_target_snapshot.clone(),
            capsule_id: echidna::domain::research::CapsuleId::new("capsule-m5-compact-old"),
            goal: "read the older bounded candidate".to_owned(),
            result: ExperimentResult::Inconclusive,
            artifact_refs: vec![ArtifactId::new("artifact-m5-compact-old")],
            notes: "older Snapshot only".to_owned(),
        },
        ExperimentCapsuleDraft {
            id: echidna::domain::research::CapsuleId::new("capsule-m5-compact-old"),
            experiment_id: ExperimentId::new("experiment-m5-compact-old"),
            target_snapshot_id: old_target_snapshot.clone(),
            action: "read older candidate fixture".to_owned(),
            arguments: json!({"path": "candidate.txt", "snapshot": "old"}),
            tool_version: "1".to_owned(),
            execution_ids: vec![echidna::domain::tools::ExecutionId::new(
                "execution:m5-compact-old-artifact",
            )],
            input_artifact_ids: vec![ArtifactId::new("artifact-m5-compact-old")],
            environment_delta: "offline fixture only".to_owned(),
            account_prerequisites: Vec::new(),
            toolchain_fingerprint: "m5-compact-fixture-v1".to_owned(),
            execution_receipt_ids: vec![old_artifact_receipt.clone()],
            output_artifact_ids: Vec::new(),
        },
    )
    .unwrap();
    append_research_event(
        &project,
        "m5-compact-old-experiment",
        ResearchEvent::ExperimentRecorded {
            experiment: old_experiment,
            capsule: Box::new(old_capsule),
            rerun_reason: None,
        },
    );

    let (experiment, capsule) = ExperimentRecord::from_drafts(
        ExperimentDraft {
            id: ExperimentId::new("experiment-m5-compact"),
            hypothesis_id: HypothesisId::new("hypothesis-m5-compact"),
            target_snapshot_id: target_snapshot.clone(),
            capsule_id: echidna::domain::research::CapsuleId::new("capsule-m5-compact"),
            goal: "read the bounded candidate".to_owned(),
            result: ExperimentResult::Inconclusive,
            artifact_refs: vec![ArtifactId::new("artifact-m5-compact")],
            notes: "no impact claim".to_owned(),
        },
        ExperimentCapsuleDraft {
            id: echidna::domain::research::CapsuleId::new("capsule-m5-compact"),
            experiment_id: ExperimentId::new("experiment-m5-compact"),
            target_snapshot_id: target_snapshot.clone(),
            action: "read candidate fixture".to_owned(),
            arguments: json!({"path": "candidate.txt"}),
            tool_version: "1".to_owned(),
            execution_ids: vec![echidna::domain::tools::ExecutionId::new(
                "execution:m5-compact-read",
            )],
            input_artifact_ids: vec![ArtifactId::new("artifact-m5-compact")],
            environment_delta: "offline fixture only".to_owned(),
            account_prerequisites: Vec::new(),
            toolchain_fingerprint: "m5-compact-fixture-v1".to_owned(),
            execution_receipt_ids: vec![read_receipt.clone()],
            output_artifact_ids: Vec::new(),
        },
    )
    .unwrap();
    append_research_event(
        &project,
        "m5-compact-experiment",
        ResearchEvent::ExperimentRecorded {
            experiment,
            capsule: Box::new(capsule),
            rerun_reason: None,
        },
    );

    let finding = FindingRecord::from_draft(FindingDraft {
        id: FindingId::new("finding-m5-compact"),
        title: "Unverified bounded candidate".to_owned(),
        status: FindingStatus::Candidate,
        target_id: "fixture.target".to_owned(),
        target_version: "1.0.0".to_owned(),
        vulnerability_class: "input validation".to_owned(),
        entry_point: "candidate input".to_owned(),
        sensitive_sink: "fixture parser".to_owned(),
        target_snapshot_ids: vec![target_snapshot.clone()],
        prerequisites: vec!["offline fixture".to_owned()],
        impact: "unverified".to_owned(),
        evidence_links: vec![EvidenceLink {
            relation: EvidenceRelation::Supports,
            evidence: EvidenceRef::Experiment(ExperimentId::new("experiment-m5-compact")),
        }],
    })
    .unwrap();
    append_research_event(
        &project,
        "m5-compact-finding",
        ResearchEvent::FindingRecorded {
            record: finding,
            keep_independent: false,
        },
    );
    let old_finding = FindingRecord::from_draft(FindingDraft {
        id: FindingId::new("finding-m5-compact-old"),
        title: "Older Snapshot candidate".to_owned(),
        status: FindingStatus::Candidate,
        target_id: "fixture.target".to_owned(),
        target_version: "1.0.0".to_owned(),
        vulnerability_class: "input validation".to_owned(),
        entry_point: "older candidate input".to_owned(),
        sensitive_sink: "older fixture parser".to_owned(),
        target_snapshot_ids: vec![old_target_snapshot],
        prerequisites: vec!["offline fixture".to_owned()],
        impact: "unverified older Snapshot".to_owned(),
        evidence_links: vec![EvidenceLink {
            relation: EvidenceRelation::Supports,
            evidence: EvidenceRef::Experiment(ExperimentId::new("experiment-m5-compact-old")),
        }],
    })
    .unwrap();
    append_research_event(
        &project,
        "m5-compact-old-finding",
        ResearchEvent::FindingRecorded {
            record: old_finding,
            keep_independent: false,
        },
    );
    append_research_event(
        &project,
        "m5-compact-old-next-action",
        ResearchEvent::NextActionRecorded(
            NextActionRecord::from_draft(NextActionDraft {
                id: NextActionId::new("next-action-m5-compact-old"),
                research_id: ResearchRecordRef::Finding(FindingId::new("finding-m5-compact-old")),
                priority: 8,
                action: "do not retain this older Snapshot action".to_owned(),
                status: NextActionStatus::Pending,
            })
            .unwrap(),
        ),
    );
    append_research_event(
        &project,
        "m5-compact-next-action",
        ResearchEvent::NextActionRecorded(
            NextActionRecord::from_draft(NextActionDraft {
                id: NextActionId::new("next-action-m5-compact"),
                research_id: ResearchRecordRef::Finding(FindingId::new("finding-m5-compact")),
                priority: 9,
                action: "verify reachability without changing claim truth".to_owned(),
                status: NextActionStatus::Pending,
            })
            .unwrap(),
        ),
    );

    let compacted_server = spawn_responses_server(vec![(
        200,
        text_response(
            "Continue from the exact compacted evidence boundary.",
            "m5-compact-second-final",
        ),
    )]);
    let compacted_config =
        write_context_window_config(root.path(), &compacted_server.base_url, 65_536);
    let compacted = run_chat(
        root.path(),
        &compacted_config,
        "/context\n/compact\ncontinue after explicit compaction\n/exit\n",
        &["--resume", "latest"],
    );
    assert!(
        compacted.status.success(),
        "{}",
        String::from_utf8_lossy(&compacted.stderr)
    );
    let compacted_output = String::from_utf8(compacted.stdout).unwrap();
    assert!(
        compacted_output.contains("Context pressure is unavailable before the first model request")
    );
    assert!(compacted_output.contains("Compaction revision 1 applied"));
    assert!(compacted_output.contains("next Primary request will be CompactionReset Full"));
    let compacted_request = request_body(&compacted_server.requests.recv().unwrap());
    compacted_server.join();
    assert!(compacted_request["previous_response_id"].is_null());
    let compacted_wire = serde_json::to_string(&compacted_request).unwrap();
    for expected in [
        "derived_session_summary_v3",
        "snapshot-m5-compact",
        "hypothesis-m5-compact",
        "experiment-m5-compact",
        "finding-m5-compact",
        "next-action-m5-compact",
        "artifact-m5-compact",
        read_receipt.as_str(),
        "continue after explicit compaction",
    ] {
        assert!(
            compacted_wire.contains(expected),
            "request lacks {expected}"
        );
    }

    let compacted_state = reduced_session(&project, &session);
    // F3 selects the retained suffix by actual budget/reduction rather than
    // always keeping twelve items. Original call/result facts remain durable
    // even if they are omitted from this request; no retained pair is split.
    let original_history = serde_json::to_string(compacted_state.history().items()).unwrap();
    assert!(original_history.contains("m5-compact-artifact"));
    assert!(original_history.contains("m5-compact-read"));
    let retained = compacted_state
        .latest_compaction()
        .unwrap()
        .retained_history();
    assert_eq!(
        echidna::domain::history::normalize_content_items(retained).unwrap(),
        retained
    );
    let summary =
        serde_json::to_value(compacted_state.latest_compaction().unwrap().summary()).unwrap();
    assert_eq!(summary["schema_version"], 3);
    assert_eq!(summary["revision"], 1);
    assert_eq!(
        summary["evidence_spine"]["target_snapshot_refs"],
        json!(["snapshot-m5-compact"])
    );
    assert_eq!(
        summary["evidence_spine"]["active_hypothesis_refs"],
        json!(["hypothesis-m5-compact"])
    );
    assert_eq!(
        summary["evidence_spine"]["experiment_refs"],
        json!(["experiment-m5-compact"])
    );
    assert_eq!(
        summary["evidence_spine"]["finding_refs"],
        json!(["finding-m5-compact"])
    );
    assert_eq!(
        summary["evidence_spine"]["next_action_refs"],
        json!(["next-action-m5-compact"])
    );
    assert_eq!(
        summary["evidence_spine"]["artifact_refs"],
        json!(["artifact-m5-compact"])
    );
    let receipt_refs = summary["evidence_spine"]["execution_receipt_refs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect::<BTreeSet<_>>();
    assert_eq!(receipt_refs, BTreeSet::from([read_receipt.as_str()]));
    assert!(!receipt_refs.contains(artifact_receipt.as_str()));
    assert!(!receipt_refs.contains(old_artifact_receipt.as_str()));
    let summary_wire = serde_json::to_string(&summary).unwrap();
    for old_reference in [
        "snapshot-m5-compact-old",
        "experiment-m5-compact-old",
        "finding-m5-compact-old",
        "next-action-m5-compact-old",
        "artifact-m5-compact-old",
    ] {
        assert!(
            !summary_wire.contains(old_reference),
            "Summary retained older Snapshot reference {old_reference}"
        );
    }
    assert_eq!(
        compacted_state
            .request_audits()
            .last()
            .unwrap()
            .delivery()
            .reason(),
        echidna::domain::request_audit::RequestAuditDeliveryReason::Full(
            echidna::domain::context::BaselineInvalidationReason::CompactionReset
        )
    );
    assert_eq!(
        session_events(root.path(), &session)
            .iter()
            .filter(|event| event["event_type"] == "session.compaction_created")
            .count(),
        1
    );

    let resumed_server = spawn_responses_server(vec![(
        200,
        text_response(
            "Resume retained the compacted boundary.",
            "m5-compact-resume",
        ),
    )]);
    let resumed_config = write_context_window_config(root.path(), &resumed_server.base_url, 65_536);
    let resumed = run_chat(
        root.path(),
        &resumed_config,
        "resume the compacted boundary\n/exit\n",
        &["--resume", "latest"],
    );
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    let resumed_request = request_body(&resumed_server.requests.recv().unwrap());
    resumed_server.join();
    assert!(resumed_request["previous_response_id"].is_null());
    assert!(
        resumed_request["instructions"]
            .as_str()
            .unwrap()
            .contains("Mobile domain profile v1")
    );
    assert!(
        resumed_request["instructions"]
            .as_str()
            .unwrap()
            .contains("execute the smallest decisive experiment")
    );
    let resumed_wire = serde_json::to_string(&resumed_request).unwrap();
    assert!(resumed_wire.contains("derived_session_summary_v3"));
    assert!(resumed_wire.contains("continue after explicit compaction"));
    assert!(resumed_wire.contains("resume the compacted boundary"));
}

#[test]
fn m5_compaction_preconditions_never_append_a_partial_fact() {
    let missing_research = TemporaryDirectory::new("m5-compact-missing-research");
    initialize_project(missing_research.path());
    let server = spawn_responses_server(vec![(
        200,
        text_response("history exists", "m5-compact-missing-research"),
    )]);
    let config = write_config(missing_research.path(), &server.base_url);
    let first = run_chat(
        missing_research.path(),
        &config,
        "create resumable history\n/exit\n",
        &[],
    );
    assert!(first.status.success());
    server.requests.recv().unwrap();
    server.join();
    let session = session_id(missing_research.path());
    let project = Project::discover(missing_research.path()).unwrap();
    append_research_event(
        &project,
        "m5-no-target-hypothesis",
        ResearchEvent::HypothesisRecorded {
            record: HypothesisRecord::from_draft(HypothesisDraft {
                id: HypothesisId::new("hypothesis-without-target-snapshot"),
                title: "Unbound hypothesis fixture".to_owned(),
                target_id: "fixture.unbound".to_owned(),
                attack_surface: "fixture".to_owned(),
                vulnerability_class: "fixture".to_owned(),
                entry_point: "fixture-entry".to_owned(),
                sensitive_sink: "fixture-sink".to_owned(),
                rationale: "exercise the missing current target boundary".to_owned(),
                status: HypothesisStatus::Active,
                priority: 1,
                evidence_links: Vec::new(),
                last_action: None,
                next_action: None,
            })
            .unwrap(),
            keep_independent: false,
        },
    );
    let offline_config = write_config(missing_research.path(), "http://127.0.0.1:1/v1");
    let rejected = run_chat(
        missing_research.path(),
        &offline_config,
        "/compact\n",
        &["--resume", "latest"],
    );
    assert!(!rejected.status.success());
    assert!(
        String::from_utf8_lossy(&rejected.stderr).contains("requires a current TargetSnapshot")
    );
    assert!(
        session_events(missing_research.path(), &session)
            .iter()
            .all(|event| event["event_type"] != "session.compaction_created")
    );

    let empty_history = TemporaryDirectory::new("m5-compact-empty-history");
    initialize_project(empty_history.path());
    let offline_config = write_config(empty_history.path(), "http://127.0.0.1:1/v1");
    let started = run_chat(empty_history.path(), &offline_config, "/exit\n", &[]);
    assert!(started.status.success());
    let session = session_id(empty_history.path());
    let project = Project::discover(empty_history.path()).unwrap();
    append_research_snapshot(&project, "m5-empty-history");
    let rejected = run_chat(
        empty_history.path(),
        &offline_config,
        "/compact\n",
        &["--resume", "latest"],
    );
    assert!(!rejected.status.success());
    assert!(
        String::from_utf8_lossy(&rejected.stderr)
            .contains("requires non-empty resumable History with a user goal")
    );
    assert!(
        session_events(empty_history.path(), &session)
            .iter()
            .all(|event| event["event_type"] != "session.compaction_created")
    );

    let incomplete_pair = TemporaryDirectory::new("m5-compact-incomplete-tool-pair");
    initialize_project(incomplete_pair.path());
    let project = Project::discover(incomplete_pair.path()).unwrap();
    append_research_snapshot(&project, "m5-incomplete-pair");
    let session_id = StreamId::new("m5-incomplete-pair");
    let stream_id = ProjectEventStream::Session(session_id.clone()).stream_id();
    let mut writer = project.open_writer().unwrap();
    let mut store = writer.open_session(session_id.clone(), None).unwrap();
    for (label, event) in [
        (
            "started",
            SessionEvent::Started {
                primary_provider_session: echidna::domain::context::ProviderSessionRef {
                    id: echidna::domain::context::ProviderSessionId::new(
                        "primary-before-incomplete-resume",
                    ),
                    role: echidna::domain::context::ProviderSessionRole::Primary,
                    generation: 1,
                },
            },
        ),
        (
            "user",
            SessionEvent::HistoryItemRecorded {
                item: echidna::domain::model::ContentItem::user_text(
                    "retain the interrupted call boundary",
                ),
            },
        ),
        (
            "call",
            SessionEvent::HistoryItemRecorded {
                item: echidna::domain::model::ContentItem::ToolCall {
                    call: echidna::domain::tools::ToolCall {
                        call_id: echidna::domain::tools::ToolCallId::new("m5-incomplete-call"),
                        name: echidna::domain::tools::ToolName::new("read_file"),
                        arguments: json!({"path": "candidate.txt"}),
                    },
                },
            },
        ),
    ] {
        let event = event
            .into_envelope(
                EventId::new(format!("m5-incomplete-{label}")),
                stream_id.clone(),
                store.next_sequence().unwrap(),
                OccurredAt::new("2026-08-31T00:02:00Z"),
            )
            .unwrap();
        EventSink::append_event(&mut store, &event).unwrap();
    }
    drop(store);
    drop(writer);
    let offline_config = write_config(incomplete_pair.path(), "http://127.0.0.1:1/v1");
    let rejected = run_chat(
        incomplete_pair.path(),
        &offline_config,
        "/compact\n",
        &["--session", session_id.as_str()],
    );
    assert!(!rejected.status.success());
    // Opening this interrupted legacy Session establishes a real recovery
    // boundary. Its old unknown Call is allowed, but these few items do not
    // produce a useful compaction. Raw History remains unchanged.
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("smaller usable next-Turn Context"));
    let state = reduced_session(&project, session_id.as_str());
    assert_eq!(state.history().items().len(), 2);
    assert_eq!(
        echidna::domain::history::normalize_content_items(state.history().items())
            .unwrap()
            .len(),
        3
    );
    let mut writer = project.open_writer().unwrap();
    let mut store = writer.open_session(session_id.clone(), None).unwrap();
    let receipt = serde_json::from_value(json!({
        "execution_id":"execution:m5-incomplete-call",
        "plan_fingerprint":"known-execution-with-missing-result", "status":"succeeded", "exit_reason":null,
        "output_artifact_ids":[], "environment_change":null
    })).unwrap();
    let event = SessionEvent::ExecutionReceiptRecorded { receipt }
        .into_envelope(
            EventId::new("m5-incomplete-bare-receipt"),
            stream_id,
            store.next_sequence().unwrap(),
            OccurredAt::new("2026-08-31T00:03:00Z"),
        )
        .unwrap();
    EventSink::append_event(&mut store, &event).unwrap();
    drop(store);
    drop(writer);
    let rejected = run_chat(
        incomplete_pair.path(),
        &offline_config,
        "/compact\n",
        &["--session", session_id.as_str()],
    );
    assert!(!rejected.status.success());
    let stderr = String::from_utf8_lossy(&rejected.stderr);
    assert!(
        stderr.contains("m5-incomplete-call")
            && stderr.contains("durable ExecutionReceipt but no paired durable ToolResult"),
        "{stderr}"
    );
    assert!(
        session_events(incomplete_pair.path(), session_id.as_str())
            .iter()
            .all(|event| event["event_type"] != "session.compaction_created")
    );
}

#[test]
fn f3_compaction_rejects_negative_reduction_without_appending() {
    let root = TemporaryDirectory::new("f3-compaction-no-reduction");
    initialize_project(root.path());
    let project = Project::discover(root.path()).unwrap();
    append_research_snapshot(&project, "small");
    let server = spawn_responses_server(vec![(200, text_response("ok", "short"))]);
    let config = write_config(root.path(), &server.base_url);
    assert!(
        run_chat(root.path(), &config, "hi\n/exit\n", &[])
            .status
            .success()
    );
    server.join();
    let session = session_id(root.path());
    let output = run_chat(root.path(), &config, "/compact\n", &["--session", &session]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("smaller usable next-Turn Context"));
    assert!(
        session_events(root.path(), &session)
            .iter()
            .all(|event| event["event_type"] != "session.compaction_created")
    );
}

#[test]
fn f3_large_compaction_bounds_reference_subset_and_drops_oversized_recent_history() {
    let root = TemporaryDirectory::new("f3-compaction-bounded");
    initialize_project(root.path());
    let project = Project::discover(root.path()).unwrap();
    append_research_snapshot(&project, "large");
    let server = spawn_responses_server(vec![(
        200,
        text_response(&"analysis ".repeat(3500), "large"),
    )]);
    let config = write_config(root.path(), &server.base_url);
    let initial = run_chat(
        root.path(),
        &config,
        "Investigate parser boundaries\n/exit\n",
        &[],
    );
    assert!(
        initial.status.success(),
        "{}",
        String::from_utf8_lossy(&initial.stderr)
    );
    server.join();
    let session = session_id(root.path());
    for index in 0..100 {
        let record = HypothesisRecord::from_draft(HypothesisDraft {
            id: HypothesisId::new(format!("hypothesis-f3-{index:03}-{}", "x".repeat(120))),
            title: format!("parser candidate {index}"),
            target_id: "fixture.target".to_owned(),
            attack_surface: format!("input {index}"),
            vulnerability_class: "validation".to_owned(),
            entry_point: format!("entry {index}"),
            sensitive_sink: "parser".to_owned(),
            rationale: "requires controlled verification".to_owned(),
            status: HypothesisStatus::Testing,
            priority: 1,
            evidence_links: vec![],
            last_action: None,
            next_action: None,
        })
        .unwrap();
        append_research_event(
            &project,
            &format!("f3-hyp-{index}"),
            ResearchEvent::HypothesisRecorded {
                record,
                keep_independent: true,
            },
        );
    }
    let output = run_chat(
        root.path(),
        &config,
        "/compact\n/exit\n",
        &["--session", &session],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let events = session_events(root.path(), &session);
    let compaction = &events
        .iter()
        .find(|event| event["event_type"] == "session.compaction_created")
        .unwrap()["payload"];
    assert!(serde_json::to_vec(&compaction["summary"]).unwrap().len() <= 8192);
    let references = compaction["summary"]["evidence_spine"]["active_hypothesis_refs"]
        .as_array()
        .unwrap();
    assert!(!references.is_empty() && references.len() <= 32);
    assert!(
        compaction["retained_history"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        compaction["summary"]["narrative"]["current_goal"]
            .as_str()
            .unwrap()
            .contains("parser boundaries")
    );
    assert!(
        !compaction["summary"]["narrative"]["unresolved_questions"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn m5_context_pressure_uses_only_planner_manifest_and_never_auto_compacts() {
    let normal = TemporaryDirectory::new("m5-context-pressure-normal");
    initialize_project(normal.path());
    let normal_server = spawn_responses_server(vec![(
        200,
        text_response("normal pressure", "m5-pressure-normal"),
    )]);
    let normal_config = write_config(normal.path(), &normal_server.base_url);
    let normal_output = run_chat(
        normal.path(),
        &normal_config,
        "inspect normal pressure\n/context\n/exit\n",
        &[],
    );
    assert!(normal_output.status.success());
    normal_server.requests.recv().unwrap();
    normal_server.join();
    let normal_stdout = String::from_utf8(normal_output.stdout).unwrap();
    assert!(
        normal_stdout.contains("[context pressure: normal]"),
        "{normal_stdout}"
    );
    assert!(normal_stdout.contains("No Compaction is suggested."));

    let warning = TemporaryDirectory::new("m5-context-pressure-warning");
    initialize_project(warning.path());
    let warning_server = spawn_responses_server(vec![(
        200,
        text_response("warning pressure", "m5-pressure-warning"),
    )]);
    // Keep this fixture inside the warning band with headroom for canonical
    // ToolSpec guidance; the separate critical lane still proves truncation.
    let warning_config =
        write_context_window_config(warning.path(), &warning_server.base_url, 24_000);
    let warning_output = run_chat(
        warning.path(),
        &warning_config,
        "inspect warning pressure\n/context\n/exit\n",
        &[],
    );
    assert!(
        warning_output.status.success(),
        "{}",
        String::from_utf8_lossy(&warning_output.stderr)
    );
    warning_server.requests.recv().unwrap();
    warning_server.join();
    let warning_stdout = String::from_utf8(warning_output.stdout).unwrap();
    assert!(
        warning_stdout.contains("[context pressure: warning]"),
        "{warning_stdout}"
    );

    let critical = TemporaryDirectory::new("m5-context-pressure-critical");
    initialize_project(critical.path());
    let discovered = Project::discover(critical.path()).unwrap();
    append_research_snapshot(&discovered, "m5-pressure-critical");
    append_research_event(
        &discovered,
        "m5-pressure-critical-hypothesis",
        ResearchEvent::HypothesisRecorded {
            record: HypothesisRecord::from_draft(HypothesisDraft {
                id: HypothesisId::new("hypothesis-m5-pressure-critical"),
                title: "critical pressure fixture".to_owned(),
                target_id: "fixture.target".to_owned(),
                attack_surface: "fixture".to_owned(),
                vulnerability_class: "fixture".to_owned(),
                entry_point: "fixture-entry".to_owned(),
                sensitive_sink: "fixture-sink".to_owned(),
                rationale: "x".repeat(80_000),
                status: HypothesisStatus::Active,
                priority: 1,
                evidence_links: Vec::new(),
                last_action: None,
                next_action: None,
            })
            .unwrap(),
            keep_independent: false,
        },
    );
    let critical_server = spawn_responses_server(vec![(
        200,
        text_response("critical pressure", "m5-pressure-critical"),
    )]);
    // Leave required fragments room while keeping the bounded Research
    // projection above the input budget; critical must not be a stopped Turn.
    let critical_config =
        write_context_window_config(critical.path(), &critical_server.base_url, 22_000);
    let critical_output = run_chat(
        critical.path(),
        &critical_config,
        "inspect critical pressure\n/context\n/exit\n",
        &[],
    );
    assert!(
        critical_output.status.success(),
        "{}",
        String::from_utf8_lossy(&critical_output.stderr)
    );
    critical_server.requests.recv().unwrap();
    critical_server.join();
    let critical_stdout = String::from_utf8(critical_output.stdout).unwrap();
    assert!(
        critical_stdout.contains("[context pressure: critical]"),
        "{critical_stdout}"
    );

    for (root, stdout) in [
        (&warning, warning_stdout.as_str()),
        (&critical, critical_stdout.as_str()),
    ] {
        assert!(stdout.contains("Consider /compact before a long next Turn."));
        assert!(stdout.contains("Provider usage is not inferred."));
        let session = session_id(root.path());
        assert!(
            session_events(root.path(), &session)
                .iter()
                .all(|event| event["event_type"] != "session.compaction_created")
        );
    }
}

#[test]
fn m5_cli_reports_real_tool_start_and_persisted_terminal_receipt() {
    let project = TemporaryDirectory::new("m5-cli-tool-visibility");
    initialize_project(project.path());
    fs::write(project.path().join("target.txt"), "bounded fixture\n").unwrap();
    let server = spawn_responses_server(vec![
        (
            200,
            m4_long_flow_tool_response(
                "m5-read",
                "read_file",
                json!({"path": "target.txt"}),
                "m5-tool",
            ),
        ),
        (200, text_response("tool complete", "m5-tool-final")),
    ]);
    let config = write_config(project.path(), &server.base_url);
    let output = run_chat(
        project.path(),
        &config,
        "read the bounded fixture\n/exit\n",
        &[],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    server.requests.recv().unwrap();
    server.requests.recv().unwrap();
    server.join();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("┌ 工具 #1 · read_file · 执行中"));
    assert!(stderr.contains("└ 工具 #1 · read_file · 完成"));
    assert!(stderr.contains("deadline 配置"), "{stderr}");
    assert!(
        stderr.contains("总耗时") && stderr.contains("含保存"),
        "{stderr}"
    );
    assert!(stderr.contains("path: target.txt"));
    assert!(stderr.contains("content: bounded fixture\n"));
    assert!(!stderr.contains("execution:m5-read"));
    assert!(!stderr.to_ascii_lowercase().contains("reasoning"));
    assert_eq!(
        session_events(project.path(), &session_id(project.path()))
            .iter()
            .filter(|event| event["event_type"] == "tool.execution_outcome_recorded")
            .count(),
        1
    );
}

#[test]
fn ec_r3_cli_shows_the_typed_research_rejection_reason_after_receipt_persistence() {
    let project = TemporaryDirectory::new("ec-r3-research-rejection-visibility");
    initialize_project(project.path());
    let rejected = json!({
        "event_id": "ec-r3-rejected-hypothesis",
        "occurred_at": "2026-09-29T12:00:00Z",
        "operation": {
            "kind": "record_hypothesis",
            "record": {
                "id": "hypothesis-ec-r3-rejected",
                "title": "Missing experiment evidence",
                "target_id": "fixture.ec-r3",
                "attack_surface": "parser",
                "vulnerability_class": "memory safety",
                "entry_point": "parse_input",
                "sensitive_sink": "copy_bytes",
                "rationale": "controlled CLI rejection fixture",
                "status": "active",
                "priority": 10,
                "evidence_links": [{
                    "relation": "supports",
                    "evidence": {"kind": "experiment", "id": "experiment-missing"}
                }]
            }
        }
    });
    let server = spawn_responses_server(vec![
        (
            200,
            m4_long_flow_tool_response(
                "ec-r3-rejected",
                "research_update",
                rejected,
                "ec-r3-rejected-step",
            ),
        ),
        (
            200,
            text_response("rejection observed", "ec-r3-rejected-final"),
        ),
    ]);
    let config = write_config(project.path(), &server.base_url);
    let output = run_chat(
        project.path(),
        &config,
        "record the controlled invalid research reference\n/exit\n",
        &["--domain", "source", "--phase", "experiment"],
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(output.status.success(), "{stderr}");
    assert!(
        stderr.contains("· research_update · 失败 · 原因 research_transition_rejected"),
        "{stderr}"
    );
    assert!(stderr.contains("deadline 配置"), "{stderr}");
    assert!(
        stderr.contains("总耗时") && stderr.contains("含保存"),
        "{stderr}"
    );
    server.requests.recv().unwrap();
    server.requests.recv().unwrap();
    server.join();
}

#[test]
fn m5_cli_labels_deduplicated_tool_calls_without_a_second_start_or_receipt() {
    let project = TemporaryDirectory::new("m5-cli-tool-dedup-visibility");
    initialize_project(project.path());
    let arguments = json!({"argv": ["/usr/bin/true"]});
    let server = spawn_responses_server(vec![
        (
            200,
            tool_calls_response(
                &[
                    ("m5-job-leader", "start_job", arguments.clone()),
                    ("m5-job-follower", "start_job", arguments),
                ],
                "m5-dedup-tool",
            ),
        ),
        (
            200,
            text_response("deduplicated tool complete", "m5-dedup-final"),
        ),
    ]);
    let config = write_config(project.path(), &server.base_url);
    let output = run_chat(
        project.path(),
        &config,
        "/phase experiment\nlaunch the bounded duplicate fixture\n/exit\n",
        &[],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    server.requests.recv().unwrap();
    server.requests.recv().unwrap();
    server.join();

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(stderr.matches("· 执行中").count(), 1, "{stderr}");
    assert_eq!(stderr.matches("· 完成").count(), 2, "{stderr}");
    assert_eq!(
        stderr.matches("复用已有收据（未再次执行）").count(),
        1,
        "{stderr}"
    );
    assert!(!stderr.contains("execution:m5-job-leader"));

    let events = session_events(project.path(), &session_id(project.path()));
    assert_eq!(
        events
            .iter()
            .filter(|event| event["event_type"] == "tool.execution_outcome_recorded")
            .count(),
        1,
        "the follower reuses the leader Receipt fact"
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| {
                event["event_type"] == "session.history_item_recorded"
                    && event["payload"]["item"]["kind"] == "tool_call"
            })
            .count(),
        2
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| {
                event["event_type"] == "session.history_item_recorded"
                    && event["payload"]["item"]["kind"] == "tool_result"
            })
            .count(),
        2
    );
}

#[test]
fn f3_graceful_interrupt_never_fabricates_a_receipt_for_unobserved_external_effects() {
    let project = TemporaryDirectory::new("m5-cli-tool-terminal-visibility");
    initialize_project(project.path());
    let marker = project.path().join("blocking-tool-started");
    let blocking_tool = project.path().join("blocking-tool.sh");
    fs::write(
        &blocking_tool,
        "#!/bin/sh\n: > \"$1\"\nexec /bin/sleep 30\n",
    )
    .unwrap();
    let mut permissions = fs::metadata(&blocking_tool).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&blocking_tool, permissions).unwrap();

    let server = spawn_responses_server(vec![
        (
            200,
            m4_long_flow_tool_response(
                "m5-failed",
                "run_command",
                json!({"argv": ["/usr/bin/false"]}),
                "m5-failed-tool",
            ),
        ),
        (200, text_response("failure observed", "m5-failed-final")),
        (
            200,
            m4_long_flow_tool_response(
                "m5-cancelled",
                "run_command",
                json!({
                    "argv": [
                        blocking_tool.to_str().unwrap(),
                        marker.to_str().unwrap()
                    ]
                }),
                "m5-cancelled-tool",
            ),
        ),
    ]);
    let config = write_config(project.path(), &server.base_url);
    let mut child = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .current_dir(project.path())
        .args(["chat", "--config", config.to_str().unwrap()])
        .env("CYBER_AGENT_PRIMARY_API_KEY", "loopback-primary-secret")
        .env("CYBER_AGENT_VISION_API_KEY", "loopback-vision-secret")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"exercise failure\nexercise cancellation\n/exit\n")
        .unwrap();

    let deadline = Instant::now() + F2A_WAIT;
    while !marker.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        marker.exists(),
        "the cancellable Tool never crossed its start boundary"
    );
    let signal_status = Command::new("/bin/kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(signal_status.success());

    let output = f2a_wait_child(child, "M5 Tool terminal visibility");
    f2a_join_server(server, "M5 Tool terminal visibility");
    assert!(
        !output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(stderr.matches("· 执行中").count(), 2, "{stderr}");
    assert!(
        stderr.contains("· run_command · 失败 · 原因 command_failed"),
        "{stderr}"
    );
    assert!(stderr.contains("deadline 配置"), "{stderr}");
    assert!(
        stderr.contains("总耗时") && stderr.contains("含保存"),
        "{stderr}"
    );
    assert!(!stderr.contains("· 取消"));
    assert!(stderr.contains("external effects are unknown"));
    assert!(stderr.contains("状态未知（未观察到批次终态）"));

    let events = session_events(project.path(), &session_id(project.path()));
    let receipt_statuses = events
        .iter()
        .filter(|event| event["event_type"] == "tool.execution_outcome_recorded")
        .map(|event| event["payload"]["receipt"]["status"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(receipt_statuses, vec!["failed"]);
    assert!(
        marker.exists(),
        "the effect before interruption must not be denied"
    );
    let state = reduced_session(
        &Project::discover(project.path()).unwrap(),
        &session_id(project.path()),
    );
    assert!(state.history().ensure_all_tool_calls_paired().is_err());
    let normalized =
        echidna::domain::history::normalize_content_items(state.history().items()).unwrap();
    let unresolved = normalized
        .iter()
        .find_map(|item| match item {
            echidna::domain::model::ContentItem::ToolResult { result }
                if result.call_id.as_str() == "m5-cancelled" =>
            {
                Some(result)
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(unresolved.output["execution_status"], "unknown");
}

#[test]
fn m5_legacy_domain_backfill_and_explicit_resume_override_are_durable() {
    let project = TemporaryDirectory::new("m5-cli-legacy-domain-override");
    initialize_project(project.path());
    let inert = write_config(project.path(), "http://127.0.0.1:1/v1");
    assert!(
        run_chat(project.path(), &inert, "/exit\n", &[])
            .status
            .success()
    );
    let session = session_id(project.path());
    assert!(
        session_events(project.path(), &session)
            .iter()
            .all(|event| event["event_type"] != "session.domain_profile_selected")
    );

    let source_server = spawn_responses_server(vec![(
        200,
        text_response("legacy source answer", "m5-legacy-domain"),
    )]);
    let source_config = write_config(project.path(), &source_server.base_url);
    let legacy = run_chat(
        project.path(),
        &source_config,
        "resume legacy domain\n/exit\n",
        &["--session", &session],
    );
    assert!(
        legacy.status.success(),
        "{}",
        String::from_utf8_lossy(&legacy.stderr)
    );
    let source_request = request_body(&source_server.requests.recv().unwrap());
    source_server.join();
    assert!(
        source_request["instructions"]
            .as_str()
            .unwrap()
            .contains("Source domain profile v1")
    );

    let web_server = spawn_responses_server(vec![(
        200,
        text_response("explicit web answer", "m5-domain-override"),
    )]);
    let web_config = write_config(project.path(), &web_server.base_url);
    let overridden = run_chat(
        project.path(),
        &web_config,
        "apply explicit domain\n/exit\n",
        &["--resume", &session, "--domain", "web"],
    );
    assert!(
        overridden.status.success(),
        "{}",
        String::from_utf8_lossy(&overridden.stderr)
    );
    let web_request = request_body(&web_server.requests.recv().unwrap());
    web_server.join();
    assert!(
        web_request["instructions"]
            .as_str()
            .unwrap()
            .contains("Web domain profile v1")
    );

    let domains = session_events(project.path(), &session)
        .into_iter()
        .filter(|event| event["event_type"] == "session.domain_profile_selected")
        .collect::<Vec<_>>();
    assert_eq!(domains.len(), 2);
    assert_eq!(
        domains[0]["payload"]["selection"],
        json!({
            "profile": "source",
            "profile_generation": 1,
            "source": "default",
            "reason_code": "LEGACY_DEFAULT_SOURCE"
        })
    );
    assert_eq!(
        domains[1]["payload"]["selection"],
        json!({
            "profile": "web",
            "profile_generation": 1,
            "source": "operator",
            "reason_code": "OPERATOR_SELECTED"
        })
    );
}

#[test]
fn m5_session_end_respects_the_active_chat_runtime_lease() {
    let project = TemporaryDirectory::new("m5-session-end-runtime-lease");
    initialize_project(project.path());
    let config = write_config(project.path(), "http://127.0.0.1:1/v1");
    let waiting = spawn_waiting_chat(project.path(), &config);
    let session = session_id(project.path());

    let rejected = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .current_dir(project.path())
        .args(["session", "end", &session])
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("Chat runtime is already active"));
    assert!(
        session_events(project.path(), &session)
            .iter()
            .all(|event| event["event_type"] != "session.ended")
    );

    waiting.exit();
    let ended = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .current_dir(project.path())
        .args(["session", "end", &session])
        .output()
        .unwrap();
    assert!(ended.status.success());
    assert_eq!(
        session_events(project.path(), &session)
            .iter()
            .filter(|event| event["event_type"] == "session.ended")
            .count(),
        1
    );
}

#[test]
fn m5_resume_latest_uses_most_recent_durable_activity_not_directory_order() {
    let project = TemporaryDirectory::new("m5-resume-latest-activity");
    initialize_project(project.path());
    let inert = write_config(project.path(), "http://127.0.0.1:1/v1");
    assert!(
        run_chat(project.path(), &inert, "/exit\n", &[])
            .status
            .success()
    );
    let first = session_ids(project.path())[0].clone();
    assert!(
        run_chat(project.path(), &inert, "/exit\n", &[])
            .status
            .success()
    );
    let sessions = session_ids(project.path());
    let second = sessions.iter().find(|id| *id != &first).unwrap().clone();

    assert!(
        run_chat(project.path(), &inert, "/exit\n", &["--resume", &first])
            .status
            .success()
    );
    let server = spawn_responses_server(vec![(
        200,
        text_response("latest activity restored", "m5-latest-activity"),
    )]);
    let config = write_config(project.path(), &server.base_url);
    let resumed = run_chat(
        project.path(),
        &config,
        "continue latest\n/exit\n",
        &["--resume", "latest"],
    );
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    server.requests.recv().unwrap();
    server.join();
    assert!(
        session_events(project.path(), &first)
            .iter()
            .any(|event| event["event_type"] == "session.history_item_recorded")
    );
    assert!(
        session_events(project.path(), &second)
            .iter()
            .all(|event| event["event_type"] != "session.history_item_recorded")
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
                "phase_generation": 3,
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
                "phase_generation": 3,
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
                "phase_generation": 2,
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
            .filter(|event| event["event_type"] == "tool.execution_outcome_recorded")
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
    assert!(output.contains("Echidna> first answer"));
    assert!(output.contains("Echidna> full fallback"));
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
fn transport_retry_marks_discarded_primary_output_and_persists_only_the_recovered_sample() {
    let project = TemporaryDirectory::new("cli-chat-transport-retry");
    initialize_project(project.path());
    let interrupted = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"discarded transport text\"}\n\n";
    let server = spawn_responses_server(vec![
        (200, interrupted.to_owned()),
        (
            200,
            text_response("recovered transport answer", "response-recovered"),
        ),
    ]);
    let config = write_config(project.path(), &server.base_url);

    let output = run_chat(
        project.path(),
        &config,
        "recover this request\n/exit\n",
        &[],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = String::from_utf8(output.stdout).unwrap();
    assert!(output.contains("partial primary output discarded; retry 1/2"));
    assert!(output.contains("recovered transport answer"));

    let first = server.requests.recv().unwrap();
    let second = server.requests.recv().unwrap();
    server.join();
    assert_eq!(
        first.split("\r\n\r\n").nth(1),
        second.split("\r\n\r\n").nth(1),
        "retry reuses the exact rendered request body"
    );

    let session = session_id(project.path());
    let events = session_events(project.path(), &session);
    assert_eq!(
        events
            .iter()
            .filter(|event| event["event_type"] == "session.request_audit_recorded")
            .count(),
        2
    );
    let history = events
        .iter()
        .filter(|event| event["event_type"] == "session.history_item_recorded")
        .map(|event| &event["payload"]["item"])
        .collect::<Vec<_>>();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0]["text"], "recover this request");
    assert_eq!(history[1]["text"], "recovered transport answer");
    assert!(
        history
            .iter()
            .all(|item| item["text"] != "discarded transport text")
    );
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
        fs::read_dir(project.path().join(".echidna/sessions"))
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
    let mut child = Command::new(env!("CARGO_BIN_EXE_echidna"))
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

mod f3_context {
    use super::*;

    fn append_finding(
        project: &Project,
        event_label: &str,
        id: &str,
        title: &str,
        target_snapshot_ids: Vec<TargetSnapshotId>,
    ) {
        let record = FindingRecord::from_draft(FindingDraft {
            id: FindingId::new(id),
            title: title.to_owned(),
            status: FindingStatus::Candidate,
            target_id: "fixture.target".to_owned(),
            target_version: "1.0.0".to_owned(),
            vulnerability_class: format!("fixture-class-{id}"),
            entry_point: format!("fixture-entry-{id}"),
            sensitive_sink: format!("fixture-sink-{id}"),
            target_snapshot_ids,
            prerequisites: Vec::new(),
            impact: "candidate only".to_owned(),
            evidence_links: Vec::new(),
        })
        .unwrap();
        append_research_event(
            project,
            event_label,
            ResearchEvent::FindingRecorded {
                record,
                keep_independent: false,
            },
        );
    }

    fn append_active_hypothesis(project: &Project, id: &str) {
        let record = HypothesisRecord::from_draft(HypothesisDraft {
            id: HypothesisId::new(id),
            title: "F3 current hypothesis".to_owned(),
            target_id: "fixture.target".to_owned(),
            attack_surface: "F3 fixture surface".to_owned(),
            vulnerability_class: "F3 fixture class".to_owned(),
            entry_point: "F3 fixture entry".to_owned(),
            sensitive_sink: "F3 fixture sink".to_owned(),
            rationale: "F3 refresh regression fixture".to_owned(),
            status: HypothesisStatus::Active,
            priority: 1,
            evidence_links: Vec::new(),
            last_action: None,
            next_action: None,
        })
        .unwrap();
        append_research_event(
            project,
            "f3-current-hypothesis",
            ResearchEvent::HypothesisRecorded {
                record,
                keep_independent: false,
            },
        );
    }

    fn write_f3_context_config(root: &std::path::Path, base_url: &str) -> std::path::PathBuf {
        let prompts = root.join("prompts");
        fs::create_dir_all(&prompts).unwrap();
        fs::write(prompts.join("personal.md"), "F3 personal prompt v1\n").unwrap();
        fs::write(prompts.join("project.md"), "F3 project prompt fixed\n").unwrap();
        fs::create_dir_all(root.join(".echidna/memory")).unwrap();
        fs::write(root.join(".echidna/memory/core.md"), "F3 core memory v1\n").unwrap();

        let config = write_config(root, base_url);
        let mut content = fs::read_to_string(&config).unwrap();
        content.push_str(
            "\n[prompts]\npersonal_file = \"prompts/personal.md\"\nproject_file = \"prompts/project.md\"\n",
        );
        fs::write(&config, content).unwrap();
        config
    }

    pub(super) fn write_sse_response(socket: &mut std::net::TcpStream, body: &str) {
        let headers = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        socket.write_all(headers.as_bytes()).unwrap();
        socket.write_all(body.as_bytes()).unwrap();
        socket.flush().unwrap();
    }

    pub(super) fn accept_before_deadline(
        listener: &TcpListener,
        label: &str,
    ) -> std::net::TcpStream {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            match listener.accept() {
                Ok((socket, _)) => {
                    // macOS may inherit O_NONBLOCK from the listener. The
                    // bounded HTTP reader expects a blocking accepted stream.
                    socket.set_nonblocking(false).unwrap();
                    return socket;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        Instant::now() < deadline,
                        "{label} loopback server did not receive the expected request"
                    );
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("{label} loopback server failed to accept: {error}"),
            }
        }
    }

    #[test]
    fn current_facts_refresh_but_instructions_stay_pinned_and_targets_stay_exact() {
        let root = TemporaryDirectory::new("f3-context-refresh");
        initialize_project(root.path());
        let project = Project::discover(root.path()).unwrap();
        let old_snapshot = append_research_snapshot(&project, "f3-old");
        let current_snapshot = append_research_snapshot(&project, "f3-current");
        append_finding(
            &project,
            "f3-old-finding",
            "f3-old-finding",
            "F3 stale finding must not reach current context",
            vec![old_snapshot],
        );
        append_finding(
            &project,
            "f3-current-finding",
            "f3-current-finding",
            "F3 exact current finding reaches context",
            vec![current_snapshot],
        );
        append_finding(
            &project,
            "f3-unknown-finding",
            "f3-unknown-finding",
            "F3 same-version finding without an exact snapshot is not proof",
            Vec::new(),
        );
        append_active_hypothesis(&project, "f3-current-hypothesis");
        fs::write(root.path().join("target.txt"), "F3 tool read fixture\n").unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let (request_sender, requests) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let server = thread::spawn(move || {
            let mut first = accept_before_deadline(&listener, "first F3 context");
            first
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            request_sender.send(read_http_request(&mut first)).unwrap();
            let _ = released.recv();
            write_sse_response(
                &mut first,
                &m4_long_flow_tool_response(
                    "f3-context-read",
                    "read_file",
                    json!({"path": "target.txt"}),
                    "f3-context-first",
                ),
            );

            let mut second = accept_before_deadline(&listener, "later F3 context");
            second
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            request_sender.send(read_http_request(&mut second)).unwrap();
            write_sse_response(
                &mut second,
                &text_response("F3 context refresh complete", "f3-context-final"),
            );
        });
        let config = write_f3_context_config(root.path(), &format!("http://{address}/v1"));
        let child = spawn_f2a_chat(
            root.path(),
            &config,
            "refresh current context after a tool result\n/exit\n",
            &[],
        );

        let first = request_body(
            &requests
                .recv_timeout(Duration::from_secs(15))
                .expect("first request should arrive"),
        );
        let first_wire = serde_json::to_string(&first).unwrap();
        let first_instructions = first["instructions"].as_str().unwrap();
        assert!(first_instructions.contains("F3 personal prompt v1"));
        assert!(first_instructions.contains("F3 project prompt fixed"));
        assert!(first_wire.contains("F3 core memory v1"));
        assert!(first_wire.contains("snapshot-f3-current"));
        assert!(first_wire.contains("fixture-environment"));
        // The bounded WorldState renderer may cut record prose, but a selected
        // record keeps its typed ID. Neither stale nor merely same-version
        // Finding is eligible to become current-context proof.
        assert!(first_wire.contains("f3-current-finding"));
        assert!(!first_wire.contains("f3-old-finding"));
        assert!(!first_wire.contains("f3-unknown-finding"));
        assert!(!first_wire.contains("F3 later next action"));

        append_research_event(
            &project,
            "f3-later-next-action",
            ResearchEvent::NextActionRecorded(
                NextActionRecord::from_draft(NextActionDraft {
                    id: NextActionId::new("f3-later-next-action"),
                    research_id: ResearchRecordRef::Hypothesis(HypothesisId::new(
                        "f3-current-hypothesis",
                    )),
                    priority: 5,
                    action: "F3 later next action".to_owned(),
                    status: NextActionStatus::Pending,
                })
                .unwrap(),
            ),
        );
        fs::write(
            root.path().join("prompts/personal.md"),
            "F3 personal prompt v2\n",
        )
        .unwrap();
        fs::write(
            root.path().join(".echidna/memory/core.md"),
            "F3 core memory v2\n",
        )
        .unwrap();
        release.send(()).unwrap();

        let second = request_body(
            &requests
                .recv_timeout(Duration::from_secs(15))
                .expect("later request should arrive"),
        );
        server.join().unwrap();
        let output = f2a_wait_child(child, "F3 context refresh");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let second_wire = serde_json::to_string(&second).unwrap();
        let second_instructions = second["instructions"].as_str().unwrap();
        assert!(second_instructions.contains("F3 personal prompt v1"));
        assert!(!second_instructions.contains("F3 personal prompt v2"));
        assert!(second_instructions.contains("F3 project prompt fixed"));
        assert!(second_wire.contains("F3 core memory v2"));
        assert!(!second_wire.contains("F3 core memory v1"));
        assert!(second_wire.contains("F3 later next action"));
    }

    #[test]
    fn configured_missing_prompt_fails_before_a_provider_request() {
        let root = TemporaryDirectory::new("f3-context-missing-prompt");
        initialize_project(root.path());
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let config = write_config(root.path(), &format!("http://{address}/v1"));
        let mut configured = fs::read_to_string(&config).unwrap();
        configured.push_str("\n[prompts]\npersonal_file = \"missing-personal.md\"\n");
        fs::write(&config, configured).unwrap();

        let output = run_chat(
            root.path(),
            &config,
            "this must not reach the provider\n/exit\n",
            &[],
        );
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("missing-personal.md"),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let deadline = Instant::now() + Duration::from_millis(200);
        loop {
            match listener.accept() {
                Ok(_) => panic!("configured missing Prompt must fail before a Provider request"),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        break;
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("failed to inspect Provider listener: {error}"),
            }
        }
    }
}
#[path = "cli_chat_cases/agent_definition_admission.rs"]
mod agent_definition_admission;
#[path = "cli_chat_cases/agent_definition_compaction.rs"]
mod agent_definition_compaction;

#[path = "cli_chat_cases/memory_visibility.rs"]
mod memory_visibility;
