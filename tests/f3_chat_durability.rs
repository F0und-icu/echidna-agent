#[path = "support/provider_config.rs"]
mod provider_config;
mod support;

use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde_json::{Value, json};
use support::TemporaryDirectory;

const WAIT: Duration = Duration::from_secs(10);

struct ToolThenPendingServer {
    base_url: String,
    requests: Receiver<String>,
    release_tool_response: Sender<()>,
    shutdown: Sender<()>,
    thread: JoinHandle<()>,
}

impl ToolThenPendingServer {
    fn release_tool_response(&self) {
        self.release_tool_response
            .send(())
            .expect("loopback server should await the Tool response release");
    }

    fn shutdown(self) {
        let _ = self.shutdown.send(());
        self.thread
            .join()
            .expect("loopback durability server should not panic");
    }
}

fn spawn_tool_then_pending_server(first_response: String) -> ToolThenPendingServer {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (request_sender, requests) = mpsc::channel();
    let (release_tool_response, release_receiver) = mpsc::channel();
    let (shutdown, shutdown_receiver) = mpsc::channel();
    let thread = thread::spawn(move || {
        let (mut first, _) = listener.accept().unwrap();
        request_sender.send(read_http_request(&mut first)).unwrap();
        release_receiver
            .recv()
            .expect("test should release the scripted Tool response");
        write_sse_response(&mut first, &first_response);

        listener.set_nonblocking(true).unwrap();
        let mut pending_sockets = Vec::new();
        loop {
            if shutdown_receiver.try_recv().is_ok() {
                return;
            }
            match listener.accept() {
                Ok((mut socket, _)) => {
                    socket
                        .set_nonblocking(false)
                        .expect("accepted loopback socket should restore blocking reads");
                    socket
                        .set_read_timeout(Some(WAIT))
                        .expect("loopback socket should accept a read timeout");
                    request_sender.send(read_http_request(&mut socket)).unwrap();
                    pending_sockets.push(socket);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("loopback accept failed: {error}"),
            }
        }
    });
    ToolThenPendingServer {
        base_url: format!("http://{address}/v1"),
        requests,
        release_tool_response,
        shutdown,
        thread,
    }
}

struct TextServer {
    base_url: String,
    requests: Receiver<String>,
    thread: JoinHandle<()>,
}

impl TextServer {
    fn join(self) {
        self.thread
            .join()
            .expect("loopback text server should not panic");
    }
}

fn spawn_text_server(text: &str) -> TextServer {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (request_sender, requests) = mpsc::channel();
    let response = text_response(text);
    let thread = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        request_sender.send(read_http_request(&mut socket)).unwrap();
        write_sse_response(&mut socket, &response);
    });
    TextServer {
        base_url: format!("http://{address}/v1"),
        requests,
        thread,
    }
}

fn read_http_request(socket: &mut std::net::TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    let expected_length = loop {
        let read = socket.read(&mut buffer).unwrap();
        assert_ne!(read, 0, "client closed before its complete request");
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
    while bytes.len() < expected_length {
        let read = socket.read(&mut buffer).unwrap();
        assert_ne!(read, 0, "client closed before its request body");
        bytes.extend_from_slice(&buffer[..read]);
    }
    String::from_utf8(bytes).unwrap()
}

fn write_sse_response(socket: &mut std::net::TcpStream, body: &str) {
    let headers = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    socket.write_all(headers.as_bytes()).unwrap();
    socket.write_all(body.as_bytes()).unwrap();
    socket.flush().unwrap();
}

fn tool_call_response() -> String {
    tool_calls_response(&[(
        "f3-durable-read",
        "read_file",
        json!({"path": "target.txt"}),
    )])
}

fn tool_calls_response(calls: &[(&str, &str, Value)]) -> String {
    let mut response = String::new();
    for (call_id, name, arguments) in calls {
        let item = json!({
            "type": "function_call",
                "call_id": call_id,
                "name": name,
                "arguments": serde_json::to_string(arguments).unwrap(),
        });
        response.push_str(&format!(
            "data: {{\"type\":\"response.output_item.done\",\"item\":{}}}\n\n",
            serde_json::to_string(&item).unwrap(),
        ));
    }
    response.push_str(
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"f3-tool-step\"}}\n\n",
    );
    response
}

fn text_response(text: &str) -> String {
    let text = serde_json::to_string(text).unwrap();
    format!(
        "data: {{\"type\":\"response.output_text.delta\",\"delta\":{text}}}\n\n\
         data: {{\"type\":\"response.completed\",\"response\":{{\"id\":\"f3-resumed\"}}}}\n\n"
    )
}

fn initialize_project(directory: &std::path::Path) {
    let output = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .current_dir(directory)
        .arg("init")
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output);
}

fn write_config(directory: &std::path::Path, base_url: &str) -> std::path::PathBuf {
    let path = directory.join("provider-config.toml");
    fs::write(
        &path,
        provider_config::fixture().replace("http://127.0.0.1:1/v1", base_url),
    )
    .unwrap();
    path
}

fn spawn_chat(project: &std::path::Path, config: &std::path::Path, input: &str) -> Child {
    let mut child = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .current_dir(project)
        .args(["chat", "--config", config.to_str().unwrap()])
        .env("CYBER_AGENT_PRIMARY_API_KEY", "f3-loopback-primary")
        .env("CYBER_AGENT_VISION_API_KEY", "f3-loopback-vision")
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

fn run_chat(
    project: &std::path::Path,
    config: &std::path::Path,
    session: &str,
    input: &str,
) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .current_dir(project)
        .args([
            "chat",
            "--config",
            config.to_str().unwrap(),
            "--session",
            session,
        ])
        .env("CYBER_AGENT_PRIMARY_API_KEY", "f3-loopback-primary")
        .env("CYBER_AGENT_VISION_API_KEY", "f3-loopback-vision")
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
    child.wait_with_output().unwrap()
}

fn only_session_id(project: &std::path::Path) -> String {
    let mut sessions = fs::read_dir(project.join(".echidna/sessions"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(sessions.len(), 1);
    sessions.pop().unwrap()
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

fn event_count(events: &[Value], event_type: &str) -> usize {
    events
        .iter()
        .filter(|event| event["event_type"] == event_type)
        .count()
}

fn wait_for_receipt(project: &std::path::Path, session: &str, expected: usize) {
    let deadline = std::time::Instant::now() + WAIT;
    loop {
        let count = session_events(project, session)
            .iter()
            .filter(|event| event["event_type"] == "tool.execution_receipt_recorded")
            .count();
        if count == expected {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for {expected} durable Receipt(s); observed {count}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn killed_chat_retains_completed_tool_history_and_does_not_replay_the_tool_on_resume() {
    let root = TemporaryDirectory::new("f3-killed-after-tool");
    initialize_project(root.path());
    fs::write(root.path().join("target.txt"), "durable tool fixture\n").unwrap();
    let server = spawn_tool_then_pending_server(tool_call_response());
    let config = write_config(root.path(), &server.base_url);
    let mut child = spawn_chat(root.path(), &config, "read target.txt, then continue\n");

    server.requests.recv_timeout(WAIT).unwrap();
    server.release_tool_response();
    let second_request = server.requests.recv_timeout(WAIT).unwrap();
    assert!(
        second_request.contains("function_call_output"),
        "the second request is reached only after the local result is available"
    );
    child.kill().unwrap();
    child.wait().unwrap();
    server.shutdown();

    let session = only_session_id(root.path());
    let events = session_events(root.path(), &session);
    assert_eq!(event_count(&events, "tool.execution_receipt_recorded"), 1);
    assert_eq!(event_count(&events, "session.history_item_recorded"), 3);
    let serialized_history = serde_json::to_string(&events).unwrap();
    assert!(serialized_history.contains("f3-durable-read"));
    assert!(serialized_history.contains("durable tool fixture"));

    let resumed = spawn_text_server("F3_RESUMED_WITHOUT_REPLAY");
    let resume_config = write_config(root.path(), &resumed.base_url);
    let output = run_chat(
        root.path(),
        &resume_config,
        &session,
        "continue from durable result\n/exit\n",
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("F3_RESUMED_WITHOUT_REPLAY")
    );
    let resumed_request = resumed.requests.recv_timeout(WAIT).unwrap();
    resumed.join();
    assert!(resumed_request.contains("function_call_output"));
    assert_eq!(
        event_count(
            &session_events(root.path(), &session),
            "tool.execution_receipt_recorded"
        ),
        1,
        "Resume may use the durable Tool result as History but never re-executes it"
    );
}

#[test]
fn fast_completed_tool_receipt_survives_while_a_parallel_slow_tool_is_interrupted() {
    let root = TemporaryDirectory::new("f3-fast-receipt-before-slow-tool");
    initialize_project(root.path());
    fs::write(root.path().join("fast.txt"), "fast durable result\n").unwrap();
    let slow_path = root.path().join("slow.pipe");
    let fifo = Command::new("/usr/bin/mkfifo")
        .arg(&slow_path)
        .status()
        .unwrap();
    assert!(
        fifo.success(),
        "mkfifo must create the controlled slow read"
    );
    let server = spawn_tool_then_pending_server(tool_calls_response(&[
        ("f3-fast", "read_file", json!({"path": "fast.txt"})),
        ("f3-slow", "read_file", json!({"path": "slow.pipe"})),
    ]));
    let config = write_config(root.path(), &server.base_url);
    let mut child = spawn_chat(root.path(), &config, "read the fast and slow fixtures\n");

    server.requests.recv_timeout(WAIT).unwrap();
    let session = only_session_id(root.path());
    server.release_tool_response();
    wait_for_receipt(root.path(), &session, 1);
    child.kill().unwrap();
    child.wait().unwrap();
    server.shutdown();

    let events = session_events(root.path(), &session);
    assert_eq!(event_count(&events, "tool.execution_receipt_recorded"), 1);
    assert_eq!(event_count(&events, "session.history_item_recorded"), 3);
    let serialized = serde_json::to_string(&events).unwrap();
    assert!(serialized.contains("execution:f3-fast"));
    assert!(!serialized.contains("execution:f3-slow"));

    let inert = write_config(root.path(), "http://127.0.0.1:1/v1");
    let resume = run_chat(root.path(), &inert, &session, "/exit\n");
    assert!(!resume.status.success());
    assert!(
        String::from_utf8_lossy(&resume.stderr)
            .contains("has a durable ExecutionReceipt but no paired durable ToolResult")
    );
}

#[test]
fn model_history_checkpoint_failure_stops_before_tool_execution() {
    let root = TemporaryDirectory::new("f3-history-checkpoint-failure");
    initialize_project(root.path());
    fs::write(root.path().join("target.txt"), "must not be read\n").unwrap();
    let server = spawn_tool_then_pending_server(tool_call_response());
    let config = write_config(root.path(), &server.base_url);
    let child = spawn_chat(root.path(), &config, "call read_file\n");

    server.requests.recv_timeout(WAIT).unwrap();
    let session = only_session_id(root.path());
    let events_path = root
        .path()
        .join(".echidna/sessions")
        .join(&session)
        .join("events.jsonl");
    fs::set_permissions(&events_path, fs::Permissions::from_mode(0o444)).unwrap();
    server.release_tool_response();
    let output = child.wait_with_output().unwrap();
    server.shutdown();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("History checkpoint commit failed"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let events = session_events(root.path(), &session);
    assert_eq!(event_count(&events, "tool.execution_receipt_recorded"), 0);
    assert_eq!(event_count(&events, "session.history_item_recorded"), 1);
    assert_eq!(
        fs::read_to_string(root.path().join("target.txt")).unwrap(),
        "must not be read\n"
    );
}
