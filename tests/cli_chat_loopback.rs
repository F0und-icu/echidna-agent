#[path = "support/provider_config.rs"]
mod provider_config;
mod support;

use std::collections::BTreeSet;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use cyber_agent::adapters::project_event_store::{Project, ProjectEventStream};
use cyber_agent::domain::artifacts::ArtifactId;
use cyber_agent::domain::events::{EventEnvelope, EventId, EventType, OccurredAt, StreamId};
use cyber_agent::domain::request_audit::REQUEST_AUDIT_MANIFEST_SCHEMA_VERSION;
use cyber_agent::domain::research::{
    ExecutionReceiptId, ExperimentId, HypothesisId, ResearchEvent, ResearchReducer, TargetKind,
    TargetSnapshotDraft, TargetSnapshotId, TargetSnapshotRecord,
};
use cyber_agent::domain::session::{
    CompactionValidationContext, EvidenceSpine, NarrativeSummary, ResearchStreamIdentity,
    SessionEvent, SessionReducer, SessionState, SessionSummaryDraft,
};
use cyber_agent::ports::{EventSink, EventSource};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use support::TemporaryDirectory;

const INLINE_API_KEY_FIXTURE: &str = "loopback-inline-api-key";
const M4_LONG_FLOW_MANIFEST_SHA256: &str =
    "0196cd31b0d366dd4f1755708ed1874db5d6faf0e006482bf502722242bc7730";

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

fn append_research_snapshot(project: &Project, label: &str) -> TargetSnapshotId {
    let snapshot_id = TargetSnapshotId::new(format!("snapshot-{label}"));
    let record = TargetSnapshotRecord::from_draft(TargetSnapshotDraft {
        id: snapshot_id.clone(),
        target_id: "fixture.target".to_owned(),
        kind: TargetKind::Source,
        locator: format!("src/{label}"),
        version: "1.0.0".to_owned(),
        build_id: format!("build-{label}"),
        target_artifact_ids: Vec::new(),
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
                        .join(".cyber-agent/artifacts/blobs/sha256")
                        .join(blob.as_str()),
                )
            })
            .unwrap();
    }
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
    let config = write_config(root.path(), &server.base_url);
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
        research["artifacts"]["records"]["artifact-m4-s4"]["target_snapshot_id"],
        "snapshot-m4-s4"
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
        .filter(|event| event["event_type"] == "tool.execution_receipt_recorded")
        .map(|event| event["event_id"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(receipt_event_ids.len(), 5);
    let artifact_receipt_event_id = raw_session_events
        .iter()
        .find(|event| {
            event["event_type"] == "tool.execution_receipt_recorded"
                && event["payload"]["receipt"]["execution_id"] == "execution:call-m4-s4-artifact"
        })
        .and_then(|event| event["event_id"].as_str())
        .unwrap()
        .to_owned();
    let read_receipt_event_id = raw_session_events
        .iter()
        .find(|event| {
            event["event_type"] == "tool.execution_receipt_recorded"
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
                == cyber_agent::domain::request_audit::RequestAuditDeliveryMode::Full
        })
        .count();
    assert_eq!(full_requests, 2);
    assert_eq!(final_state.request_audits().len() - full_requests, 5);
    assert_eq!(
        final_state
            .request_audits()
            .last()
            .unwrap()
            .delivery()
            .mode(),
        cyber_agent::domain::request_audit::RequestAuditDeliveryMode::Full
    );
    let latest_compaction = final_state.latest_compaction().unwrap();
    let summary = serde_json::to_value(latest_compaction.summary()).unwrap();
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
        .find(|event| event["event_type"] == "session.started")
        .and_then(|event| event["payload"]["primary_provider_session"]["id"].as_str())
        .unwrap();
    let resumed_primary = final_events
        .iter()
        .find(|event| event["event_type"] == "session.resumed")
        .and_then(|event| event["payload"]["primary_provider_session"]["id"].as_str())
        .unwrap();
    assert_ne!(started_primary, resumed_primary);

    let metrics = Command::new(env!("CARGO_BIN_EXE_cyber-agent"))
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
fn stale_v2_compaction_is_rejected_before_a_durable_resume_event() {
    let root = TemporaryDirectory::new("cli-chat-stale-compaction");
    initialize_project(root.path());
    let config = write_config(root.path(), "http://127.0.0.1:1/v1");
    let created = run_chat(root.path(), &config, "/exit\n", &[]);
    assert!(created.status.success());
    let session = session_id(root.path());
    let project = Project::discover(root.path()).unwrap();

    let bound_target = append_research_snapshot(&project, "bound");
    append_v2_compaction(&project, &session, bound_target);
    append_research_snapshot(&project, "newer");
    let before = session_events(root.path(), &session);

    let resumed = run_chat(root.path(), &config, "/exit\n", &["--session", &session]);
    assert!(!resumed.status.success());
    assert!(
        String::from_utf8_lossy(&resumed.stderr).contains("Research revision is stale"),
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
