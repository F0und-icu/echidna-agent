//! Real terminal + loopback transport, no live Provider or model-quality claim.
use super::*;
use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};

struct TerminalChat {
    child: Box<dyn portable_pty::Child + Send + Sync>,
    writer: Box<dyn Write + Send>,
    output: Receiver<Vec<u8>>,
    captured: Vec<u8>,
    _master: Box<dyn MasterPty + Send>,
}

impl TerminalChat {
    fn new(root: &std::path::Path, config: &std::path::Path) -> Self {
        Self::with_theme_environment(root, config, None, "xterm-256color")
    }

    fn with_theme_environment(
        root: &std::path::Path,
        config: &std::path::Path,
        no_color: Option<&str>,
        term: &str,
    ) -> Self {
        let pair = native_pty_system().openpty(PtySize::default()).unwrap();
        let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_echidna"));
        command.cwd(root);
        command.args(["--config", config.to_str().unwrap(), "chat"]);
        command.env("CYBER_AGENT_PRIMARY_API_KEY", "loopback-primary-secret");
        command.env("CYBER_AGENT_VISION_API_KEY", "loopback-vision-secret");
        command.env("TERM", term);
        command.env_remove("NO_COLOR");
        if let Some(value) = no_color {
            command.env("NO_COLOR", value);
        }
        let child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);
        let writer = pair.master.take_writer().unwrap();
        let mut reader = pair.master.try_clone_reader().unwrap();
        let (tx, output) = mpsc::channel();
        thread::spawn(move || {
            let mut chunk = [0; 4096];
            while let Ok(count) = reader.read(&mut chunk) {
                if count == 0 || tx.send(chunk[..count].to_vec()).is_err() {
                    break;
                }
            }
        });
        let mut chat = Self {
            child,
            writer,
            output,
            captured: Vec::new(),
            _master: pair.master,
        };
        chat.wait_for("you> ");
        chat
    }

    fn send(&mut self, text: &str) {
        self.writer.write_all(text.as_bytes()).unwrap();
        self.writer.flush().unwrap();
    }

    fn wait_for(&mut self, needle: &str) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while !String::from_utf8_lossy(&self.captured).contains(needle) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let bytes = self.output.recv_timeout(remaining).unwrap_or_else(|error| {
                panic!(
                    "terminal did not show {needle}: {error}; {}",
                    String::from_utf8_lossy(&self.captured)
                )
            });
            self.captured.extend(bytes);
        }
    }

    fn exit(&mut self) {
        self.send("/exit\n");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success());
                return;
            }
            assert!(Instant::now() < deadline, "terminal CLI did not exit");
            thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for TerminalChat {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn echidna_terminal_theme_respects_no_color_and_dumb_without_changing_facts() {
    for (no_color, term, colored) in [
        (None, "xterm-256color", true),
        (Some(""), "xterm-256color", false),
        (Some("1"), "xterm-256color", false),
        (None, "dumb", false),
    ] {
        let root = TemporaryDirectory::new("echidna-terminal-theme");
        let harness = TemporaryDirectory::new("echidna-terminal-config");
        initialize_project(root.path());
        let server =
            spawn_responses_server(vec![(200, text_response("THEME_REPLY", "theme-response"))]);
        let config = write_config(harness.path(), &server.base_url);
        let mut chat = TerminalChat::with_theme_environment(root.path(), &config, no_color, term);
        chat.send("reply once\n");
        chat.wait_for("THEME_REPLY");
        chat.exit();
        let captured = String::from_utf8_lossy(&chat.captured);
        assert!(captured.contains("Local Agent"));
        assert!(captured.contains("Question. Observe. Verify."));
        assert!(captured.contains("Echidna> "));
        assert_eq!(captured.contains('\u{1b}'), colored, "{captured}");
        let request = f2a_request_body(&server.requests, "theme request").to_string();
        let history =
            serde_json::to_string(&session_events(root.path(), &session_id(root.path()))).unwrap();
        for content in [&request, &history] {
            assert!(!content.contains("Echidna>"));
            assert!(!content.contains("Question. Observe. Verify."));
        }
        f2a_join_server(server, "theme request");
    }
}

#[test]
fn terminal_eof_during_sampling_finishes_the_response_without_cancelling() {
    let root = TemporaryDirectory::new("terminal-eof");
    let harness = TemporaryDirectory::new("terminal-eof-provider");
    initialize_project(root.path());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let config = write_config(
        harness.path(),
        &format!("http://{}/v1", listener.local_addr().unwrap()),
    );
    let (seen_tx, seen) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let server = thread::spawn(move || {
        let mut socket = f3_context::accept_before_deadline(&listener, "terminal EOF");
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        read_http_request(&mut socket);
        seen_tx.send(()).unwrap();
        released.recv_timeout(Duration::from_secs(10)).unwrap();
        let body = text_response("EOF_RESPONSE_COMPLETED", "eof-response");
        write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    });
    let mut chat = TerminalChat::new(root.path(), &config);
    chat.send("finish this reply\n");
    seen.recv_timeout(Duration::from_secs(10)).unwrap();
    chat.send("\x04");
    release.send(()).unwrap();
    chat.wait_for("EOF_RESPONSE_COMPLETED");
    chat.wait_for("EOF received;");
    server.join().unwrap();
    let project = Project::discover(root.path()).unwrap();
    let state = reduced_session(&project, &session_id(root.path()));
    assert_eq!(
        state.history().items().last().unwrap(),
        &echidna::domain::model::ContentItem::assistant_text("EOF_RESPONSE_COMPLETED")
    );
    assert!(!String::from_utf8_lossy(&chat.captured).contains("[request cancelled]"));
}

#[test]
fn terminal_correction_during_unknown_tool_execution_is_not_automatically_submitted() {
    let root = TemporaryDirectory::new("tool-steering");
    let harness = TemporaryDirectory::new("tool-steering-provider");
    initialize_project(root.path());
    let server = spawn_responses_server(vec![(
        200,
        m4_long_flow_tool_response(
            "started-command",
            "run_command",
            json!({"argv":["sh", "-c", "printf x >> started; exec sleep 5"]}),
            "started-response",
        ),
    )]);
    let config = write_config(harness.path(), &server.base_url);
    let mut chat = TerminalChat::new(root.path(), &config);
    chat.send("run the bounded command\n");
    server
        .requests
        .recv_timeout(Duration::from_secs(10))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !root.path().join("started").exists() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
    chat.send("only report status, do not run again\n");
    chat.wait_for("[queued input not submitted or saved]");
    chat.wait_for("external effects are unknown");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = chat.child.try_wait().unwrap() {
            assert!(!status.success());
            break;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(fs::read(root.path().join("started")).unwrap(), b"x");
    let project = Project::discover(root.path()).unwrap();
    let state = reduced_session(&project, &session_id(root.path()));
    assert_eq!(state.request_audits().len(), 1);
    assert_eq!(state.execution_receipt_count(), 0);
    assert!(
        !serde_json::to_string(state.history().items())
            .unwrap()
            .contains("only report status")
    );
    server.join();
}

#[test]
fn terminal_stop_and_correction_cancel_inflight_sampling_before_new_turn() {
    for interruption in ["correction", "stop", "invalid_utf8"] {
        let root = TemporaryDirectory::new("terminal-steering");
        let harness = TemporaryDirectory::new("terminal-provider");
        initialize_project(root.path());
        // Hold old request open. A second accepted request can only come from
        // cancellation plus a new Turn, not completion of the old response.
        let server = spawn_f2a_pending_vision_server();
        let config = write_config(harness.path(), &server.base_url);
        let mut chat = TerminalChat::new(root.path(), &config);
        chat.send("old broad task\n");
        let old = server
            .requests
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
        assert!(old.contains("old broad task"));
        chat.send("\n"); // blank input must not cancel
        if interruption != "correction" {
            if interruption == "stop" {
                chat.send("/stop\n");
            } else {
                chat.writer.write_all(b"\xff\n").unwrap();
                chat.writer.flush().unwrap();
                chat.wait_for("[input rejected]");
            }
            chat.wait_for("[request cancelled]");
            assert!(server.requests.try_recv().is_err());
            chat.send("only check connection and stop\n");
        } else {
            chat.send("only check connection and stop\n");
        }
        let new = server
            .requests
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
        assert!(new.contains("only check connection and stop"));
        assert!(new.contains("Latest user scope wins"));
        chat.send("/stop\n");
        // Wait for the second cancellation, not an old occurrence.
        chat.captured.clear();
        chat.wait_for("[request cancelled]");
        chat.exit();
        let project = Project::discover(root.path()).unwrap();
        let state = reduced_session(&project, &session_id(root.path()));
        let users = state
            .history()
            .items()
            .iter()
            .filter_map(|item| match item {
                echidna::domain::model::ContentItem::Text {
                    role: echidna::domain::model::MessageRole::User,
                    text,
                } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(users, ["old broad task", "only check connection and stop"]);
        assert_eq!(state.execution_receipt_count(), 0);
        assert!(server.shutdown().is_empty(), "no automatic extra requests");
    }
}

#[test]
fn terminal_correction_during_initial_vision_does_not_submit_old_primary_task() {
    let root = TemporaryDirectory::new("vision-steering");
    let harness = TemporaryDirectory::new("vision-steering-provider");
    initialize_project(root.path());
    append_research_snapshot(&Project::discover(root.path()).unwrap(), "vision-steering");
    let vision = spawn_f2a_pending_vision_server();
    let primary = spawn_responses_server(vec![(200, text_response("NEW_TASK_DONE", "corrected"))]);
    let config = write_dual_config(harness.path(), &primary.base_url, &vision.base_url);
    let image = root.path().join("image.png");
    image::RgbImage::new(2, 2).save(&image).unwrap();
    let mut chat = TerminalChat::new(root.path(), &config);
    chat.send(&format!("/image {}\n", image.display()));
    chat.send("old image task\n");
    vision
        .requests
        .recv_timeout(Duration::from_secs(10))
        .unwrap();
    chat.send("cancel the image task; answer this text only\n");
    let request = primary
        .requests
        .recv_timeout(Duration::from_secs(10))
        .unwrap();
    assert!(request.contains("answer this text only"));
    assert!(!request.contains("old image task"));
    assert!(!request.contains("input_image"));
    chat.wait_for("NEW_TASK_DONE");
    chat.exit();
    assert!(vision.shutdown().is_empty());
    primary.join();
}
