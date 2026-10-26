//! Real terminal + loopback transport, no live Provider or model-quality claim.
use super::*;
use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};

struct TerminalChat {
    child: Box<dyn portable_pty::Child + Send + Sync>,
    writer: Box<dyn Write + Send>,
    output: Receiver<Vec<u8>>,
    captured: Vec<u8>,
    screen_parser: vt100::Parser,
    screen_offset: usize,
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
        Self::start(root, config, no_color, term, None)
    }

    fn start(
        root: &std::path::Path,
        config: &std::path::Path,
        no_color: Option<&str>,
        term: &str,
        agent: Option<&std::path::Path>,
    ) -> Self {
        let pair = native_pty_system().openpty(PtySize::default()).unwrap();
        let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_echidna"));
        command.cwd(root);
        command.args(["--config", config.to_str().unwrap(), "chat"]);
        if let Some(agent) = agent {
            command.args(["--agent", agent.to_str().unwrap()]);
        }
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
            screen_parser: vt100::Parser::new(24, 80, 0),
            screen_offset: 0,
            _master: pair.master,
        };
        chat.wait_for("you> ");
        chat
    }

    fn send(&mut self, text: &str) {
        self.writer.write_all(text.as_bytes()).unwrap();
        self.writer.flush().unwrap();
    }

    fn screen(&mut self) -> vt100::Screen {
        while let Ok(bytes) = self.output.recv_timeout(Duration::from_millis(100)) {
            self.captured.extend(bytes);
        }
        self.screen_parser
            .process(&self.captured[self.screen_offset..]);
        self.screen_offset = self.captured.len();
        self.screen_parser.screen().clone()
    }

    fn resize(&mut self, columns: u16) {
        self.screen();
        self.screen_parser.screen_mut().set_size(24, columns);
        self._master
            .resize(PtySize {
                rows: 24,
                cols: columns,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
    }

    fn assert_draft(&mut self, text: &str, column: u16) {
        let screen = self.screen();
        let (row, cursor) = screen.cursor_position();
        assert_eq!(
            screen.rows(0, 80).nth(row as usize).unwrap_or_default(),
            text,
            "{}",
            screen.contents()
        );
        assert_eq!(cursor, column, "{}", screen.contents());
    }

    fn wait_for_draft(&mut self, text: &str, column: u16) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let screen = self.screen();
            let (row, cursor) = screen.cursor_position();
            if screen.rows(0, 80).nth(row as usize).unwrap_or_default() == text && cursor == column
            {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "terminal did not restore draft {text:?} at column {column}: {}",
                screen.contents()
            );
        }
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
                self.assert_restored();
                return;
            }
            assert!(Instant::now() < deadline, "terminal CLI did not exit");
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn assert_restored(&mut self) {
        while let Ok(bytes) = self.output.recv_timeout(Duration::from_millis(100)) {
            self.captured.extend(bytes);
        }
        let termios = self._master.get_termios().unwrap();
        assert_ne!(termios.local_flags.bits() & libc::ICANON, 0);
        assert_ne!(termios.local_flags.bits() & libc::ECHO, 0);
        if String::from_utf8_lossy(&self.captured).contains("\x1b[?2004h") {
            assert!(String::from_utf8_lossy(&self.captured).contains("\x1b[?2004l"));
        }
    }
}

#[test]
fn accepted_task_shows_working_before_context_preparation_failure() {
    let root = TemporaryDirectory::new("working-before-context-error");
    initialize_project(root.path());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let config = write_config(
        root.path(),
        &format!("http://{}/v1", listener.local_addr().unwrap()),
    );
    let mut chat = TerminalChat::new(root.path(), &config);
    // Change an explicit source only after startup, so the next Turn's real
    // context refresh fails before any Provider request can be admitted.
    fs::create_dir_all(root.path().join(".echidna/memory/core.md")).unwrap();
    chat.send("Read the current task sources.\n");
    chat.wait_for("chat failed:");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = chat.child.try_wait().unwrap() {
            assert!(!status.success());
            break;
        }
        assert!(
            Instant::now() < deadline,
            "preparation failure did not settle"
        );
        thread::sleep(Duration::from_millis(10));
    }
    chat.assert_restored();
    let output = String::from_utf8_lossy(&chat.captured);
    let working = output
        .find("working> ")
        .expect("accepted task status missing");
    assert!(working < output.find("chat failed:").unwrap(), "{output}");
    let project = Project::discover(root.path()).unwrap();
    let state = reduced_session(&project, &session_id(root.path()));
    assert!(state.history().items().is_empty());
    assert!(state.request_audits().is_empty());
    assert!(
        matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
    );
}

#[test]
fn rsi2_terminal_stop_cancels_diagnosis_without_recovery_or_probe_requests() {
    let root = TemporaryDirectory::new("rsi2-terminal-stop");
    initialize_project(root.path());
    let definition = super::agent_evolution::improvement_definition(root.path());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
    let (observed, requests) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    let server = thread::spawn(move || {
        let mut first = f3_context::accept_before_deadline(&listener, "RSI2 primary");
        let request = read_http_request(&mut first);
        observed.send(request).unwrap();
        let body = text_response("TASK_COMPLETED", "ordinary-completed");
        write!(first,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
        drop(first);
        let mut diagnostic = f3_context::accept_before_deadline(&listener, "RSI2 diagnosis");
        observed.send(read_http_request(&mut diagnostic)).unwrap();
        wait.recv_timeout(Duration::from_secs(20)).unwrap();
        drop(diagnostic);
        assert!(
            matches!(listener.accept(),Err(error) if error.kind()==std::io::ErrorKind::WouldBlock)
        );
    });
    let config = write_config(root.path(), &base_url);
    let mut chat = TerminalChat::start(
        root.path(),
        &config,
        None,
        "xterm-256color",
        Some(&definition),
    );
    chat.send("[rsi-feedback] Keep future replies concise.\n");
    requests.recv_timeout(Duration::from_secs(10)).unwrap();
    let diagnostic = requests.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(diagnostic.contains("RSI2 diagnosis"));
    chat.send("/stp\x1b[Do\x1b[C");
    chat.assert_draft("working> /stop", 14);
    assert!(!String::from_utf8_lossy(&chat.captured).contains("[RSI2 inconclusive]"));
    assert!(requests.try_recv().is_err());
    chat.send("\n");
    chat.wait_for("[RSI2 inconclusive]");
    assert!(!String::from_utf8_lossy(&chat.captured).contains("[turn stopped]"));
    chat.exit();
    chat.assert_restored();
    release.send(()).unwrap();
    server.join().unwrap();
    let events = session_events(root.path(), &session_id(root.path()));
    assert_eq!(
        events
            .iter()
            .filter(|event| event["event_type"] == "session.request_audit_recorded")
            .count(),
        2
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event["event_type"] == "session.agent_definition_applied")
            .count(),
        0
    );
    assert!(
        !events
            .iter()
            .any(|event| event["event_type"] == "session.resumed")
    );
}

impl Drop for TerminalChat {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn terminal_processing_prompt_whitespace_and_follow_up_handoff_are_truthful() {
    let root = TemporaryDirectory::new("terminal-processing");
    let harness = TemporaryDirectory::new("terminal-processing-config");
    initialize_project(root.path());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let config = write_config(
        harness.path(),
        &format!("http://{}/v1", listener.local_addr().unwrap()),
    );
    let (requests_tx, requests) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let server = thread::spawn(move || {
        let mut old = f3_context::accept_before_deadline(&listener, "old processing request");
        requests_tx.send(read_http_request(&mut old)).unwrap();
        let mut next = f3_context::accept_before_deadline(&listener, "follow-up request");
        requests_tx.send(read_http_request(&mut next)).unwrap();
        drop(old);
        released.recv_timeout(Duration::from_secs(10)).unwrap();
        let body = text_response("FOLLOW_UP_DONE", "follow-up");
        write!(next, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    });
    let mut chat = TerminalChat::new(root.path(), &config);
    // Ordinary whitespace clears the editable row without echo or a new row,
    // both before a task and while the original request remains held open.
    for working in [false, true] {
        if working {
            chat.send("old task\n");
            assert!(
                requests
                    .recv_timeout(Duration::from_secs(10))
                    .unwrap()
                    .contains("old task")
            );
            chat.assert_draft("working> ", 9);
        }
        let before = chat.screen();
        let offset = chat.captured.len();
        chat.send("\n   \n\u{a0}\n");
        let after = chat.screen();
        assert_eq!(after.cursor_position(), before.cursor_position());
        assert_eq!(after.contents(), before.contents());
        assert!(
            !chat.captured[offset..]
                .windows(2)
                .any(|bytes| bytes == b"\r\n")
        );
        assert!(requests.try_recv().is_err());
        assert!(!String::from_utf8_lossy(&chat.captured).contains("[interrupt requested]"));
    }
    chat.resize(6);
    chat.assert_draft("*", 1);
    chat.resize(80);
    chat.assert_draft("working> ", 9);
    chat.send("new tsk\x1b[D\x1b[Da\x1b[F");
    chat.assert_draft("working> new task", 17);
    assert!(requests.try_recv().is_err());
    chat.send("\n");
    let next = requests.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(next.contains("new task"));
    chat.assert_draft("working> ", 9);
    let captured = String::from_utf8_lossy(&chat.captured);
    let marker = "[Follow-up accepted; starting processing]";
    assert_eq!(captured.matches(marker).count(), 1);
    assert!(captured.find("[request cancelled]").unwrap() < captured.find(marker).unwrap());
    assert!(!next.contains(marker));
    assert!(!next.contains("working>"));
    release.send(()).unwrap();
    chat.wait_for("FOLLOW_UP_DONE");
    chat.wait_for_draft("you> ", 5);
    chat.exit();
    server.join().unwrap();
    let project = Project::discover(root.path()).unwrap();
    let state = reduced_session(&project, &session_id(root.path()));
    assert_eq!(state.request_audits().len(), 2);
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
    assert_eq!(users, ["old task", "new task"]);
    assert!(
        !serde_json::to_string(state.history().items())
            .unwrap()
            .contains(marker)
    );
    let captured = String::from_utf8_lossy(&chat.captured);
    assert_eq!(
        captured
            .matches("[request budget] admitted=1 primary=1 vision=0")
            .count(),
        2
    );
}

#[test]
fn terminal_processing_queued_context_command_does_not_claim_a_follow_up() {
    let root = TemporaryDirectory::new("terminal-processing-command");
    let harness = TemporaryDirectory::new("terminal-processing-command-config");
    initialize_project(root.path());
    let server = spawn_f2a_pending_vision_server();
    let config = write_config(harness.path(), &server.base_url);
    let mut chat = TerminalChat::new(root.path(), &config);
    chat.send("held task\n");
    server
        .requests
        .recv_timeout(Duration::from_secs(10))
        .unwrap();
    chat.assert_draft("working> ", 9);
    chat.send("/context\n");
    chat.wait_for("[request cancelled]");
    chat.wait_for_draft("you> ", 5);
    assert!(!String::from_utf8_lossy(&chat.captured).contains("[Follow-up accepted"));
    assert!(server.requests.try_recv().is_err());
    chat.exit();
    assert!(server.shutdown().is_empty());
}

#[test]
fn terminal_processing_paste_mode_preserves_blank_lines_and_exact_text() {
    let root = TemporaryDirectory::new("terminal-processing-paste");
    let harness = TemporaryDirectory::new("terminal-processing-paste-config");
    initialize_project(root.path());
    let server = spawn_responses_server(vec![(
        200,
        text_response("PASTE_BLANKS_DONE", "paste-blanks"),
    )]);
    let config = write_config(harness.path(), &server.base_url);
    let mut chat = TerminalChat::new(root.path(), &config);
    chat.send("/paste\n\n  \n\u{a0}\nbody\n\n/end\n");
    chat.wait_for("PASTE_BLANKS_DONE");
    chat.wait_for_draft("you> ", 5);
    chat.exit();
    let expected = "\n  \n\u{a0}\nbody\n";
    let body = f2a_request_body(&server.requests, "paste blank lines");
    assert!(
        body["input"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["role"] == "user")
            .flat_map(|item| item["content"].as_array().unwrap())
            .any(|content| content["text"] == expected)
    );
    let project = Project::discover(root.path()).unwrap();
    let state = reduced_session(&project, &session_id(root.path()));
    assert!(
        state
            .history()
            .items()
            .iter()
            .any(|item| *item == echidna::domain::model::ContentItem::user_text(expected))
    );
    f2a_join_server(server, "paste blank lines");
}

#[test]
fn terminal_processing_follow_up_is_not_announced_after_persistence_failure() {
    let root = TemporaryDirectory::new("terminal-processing-persistence");
    let harness = TemporaryDirectory::new("terminal-processing-persistence-config");
    initialize_project(root.path());
    let server = spawn_f2a_pending_vision_server();
    let config = write_config(harness.path(), &server.base_url);
    let mut chat = TerminalChat::new(root.path(), &config);
    chat.send("held task\n");
    server
        .requests
        .recv_timeout(Duration::from_secs(10))
        .unwrap();
    chat.assert_draft("working> ", 9);
    let events = root
        .path()
        .join(".echidna/sessions")
        .join(session_id(root.path()))
        .join("events.jsonl");
    let original = fs::read(&events).unwrap();
    fs::write(&events, b"invalid fixture Session JSONL\n").unwrap();
    chat.send("follow-up must not start\n");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = chat.child.try_wait().unwrap() {
            assert!(!status.success());
            break;
        }
        assert!(
            Instant::now() < deadline,
            "persistence failure did not stop the CLI"
        );
        thread::sleep(Duration::from_millis(10));
    }
    chat.assert_restored();
    let captured = String::from_utf8_lossy(&chat.captured);
    assert!(captured.contains("[request cancelled]"));
    assert!(!captured.contains("[Follow-up accepted"));
    assert!(server.requests.try_recv().is_err());
    assert_eq!(
        fs::read(&events).unwrap(),
        b"invalid fixture Session JSONL\n"
    );
    // Restore only this test's injected bytes so its prior durable facts can
    // be checked; production did not recover or append over the corrupt stream.
    fs::write(&events, original).unwrap();
    let project = Project::discover(root.path()).unwrap();
    let state = reduced_session(&project, &session_id(root.path()));
    assert_eq!(state.request_audits().len(), 1);
    assert!(
        !serde_json::to_string(state.history().items())
            .unwrap()
            .contains("follow-up must not start")
    );
    assert!(server.shutdown().is_empty());
}

#[test]
fn terminal_processing_pipe_preserves_streams_without_status_or_follow_up_notice() {
    let root = TemporaryDirectory::new("terminal-processing-pipe");
    let harness = TemporaryDirectory::new("terminal-processing-pipe-config");
    initialize_project(root.path());
    let server = spawn_responses_server(vec![(200, text_response("PIPE_DONE", "pipe-done"))]);
    let config = write_config(harness.path(), &server.base_url);
    let output = run_chat(root.path(), &config, "piped task\n/exit\n", &[]);
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("PIPE_DONE"));
    for stream in [&output.stdout, &output.stderr] {
        let text = String::from_utf8_lossy(stream);
        assert!(!text.contains("working>"));
        assert!(!text.contains("[Follow-up accepted"));
        assert!(!text.contains("\x1b[?2004"));
    }
    let body = f2a_request_body(&server.requests, "piped task");
    assert!(body.to_string().contains("piped task"));
    f2a_join_server(server, "piped task");
}

#[test]
fn terminal_editor_generic_agent_preserves_binding_and_exact_edited_task() {
    let root = TemporaryDirectory::new("terminal-editor-generic-agent");
    let harness = TemporaryDirectory::new("terminal-editor-generic-config");
    initialize_project(root.path());
    let definition = root.path().join("writer");
    let exported = Command::new(env!("CARGO_BIN_EXE_echidna"))
        .current_dir(root.path())
        .args([
            "agent",
            "init",
            definition.to_str().unwrap(),
            "--from",
            "technical-writing",
        ])
        .output()
        .unwrap();
    assert!(exported.status.success());
    let server = spawn_responses_server(vec![(
        200,
        text_response("GENERIC_EDITED", "generic-edited"),
    )]);
    let config = write_config(harness.path(), &server.base_url);
    let mut chat = TerminalChat::start(
        root.path(),
        &config,
        None,
        "xterm-256color",
        Some(&definition),
    );
    chat.wait_for("with Agent `technical-writing`");
    chat.send("ordr\x1b[De\x1b[C");
    chat.assert_draft("you> order", 10);
    assert!(server.requests.try_recv().is_err());
    chat.send("\n");
    chat.wait_for("GENERIC_EDITED");
    chat.exit();
    chat.assert_restored();
    let request = f2a_request_body(&server.requests, "generic edited task");
    let instructions = request["instructions"].as_str().unwrap();
    assert!(instructions.contains("technical writer"));
    assert!(!instructions.contains("Pragmatic Vulnerability Researcher"));
    assert!(!instructions.contains("RSI"));
    let project = Project::discover(root.path()).unwrap();
    let state = reduced_session(&project, &session_id(root.path()));
    let binding = state.agent_definition_binding().unwrap();
    assert_eq!(binding.definition().id().as_str(), "technical-writing");
    assert!(binding.definition().research().is_none());
    assert_eq!(
        state.history().items(),
        &[
            echidna::domain::model::ContentItem::user_text("order"),
            echidna::domain::model::ContentItem::assistant_text("GENERIC_EDITED"),
        ]
    );
    assert_eq!(state.request_audits().len(), 1);
    assert_eq!(state.execution_receipt_count(), 0);
    f2a_join_server(server, "generic edited task");
}

#[test]
fn terminal_editor_left_right_insert_submits_only_the_edited_text() {
    let root = TemporaryDirectory::new("terminal-editor-cursor");
    let harness = TemporaryDirectory::new("terminal-editor-config");
    initialize_project(root.path());
    let server = spawn_responses_server(vec![(200, text_response("EDITED_REPLY", "edited"))]);
    let config = write_config(harness.path(), &server.base_url);
    let mut chat = TerminalChat::new(root.path(), &config);
    chat.send("abcd\x1b[D\x1b[DX\x1b[C\x1b[3~\n");
    chat.wait_for("EDITED_REPLY");
    chat.exit();
    let body = f2a_request_body(&server.requests, "edited terminal request");
    assert!(
        body["input"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["role"] == "user")
            .flat_map(|item| item["content"].as_array().unwrap())
            .any(|content| content["text"] == "abXc"),
        "{body}"
    );
    f2a_join_server(server, "edited terminal request");
}

#[test]
fn terminal_editor_unicode_erase_and_long_viewport_leave_no_visible_ghosts() {
    let root = TemporaryDirectory::new("terminal-editor-display");
    let harness = TemporaryDirectory::new("terminal-editor-config");
    initialize_project(root.path());
    let server = spawn_responses_server(vec![(200, text_response("UNICODE_REPLY", "unicode"))]);
    let config = write_config(harness.path(), &server.base_url);
    let mut chat = TerminalChat::new(root.path(), &config);
    chat.send("中文e\u{301}界\x1b[D\x7fX");
    chat.assert_draft("you> 中文X界", 10);
    chat.send("\x1b[C\x7f\x7f\x7f\x7f");
    chat.assert_draft("you> ", 5);
    chat.send("🇦x🇧\x1b[H\x1b[C\x1b[3~");
    chat.assert_draft("you> 🇦🇧", 7);
    chat.send("\x1b[D\x1b[C\x7f");
    chat.assert_draft("you> ", 5);
    chat.send(&"长".repeat(100));
    let screen = chat.screen();
    assert!(screen.cursor_position().1 < 80);
    chat.resize(24);
    // vt100 truncates/resizes rows; it does not emulate every native terminal's
    // scrollback reflow. This proves PTY resize delivery and viewport redraw.
    let screen = chat.screen();
    assert!(screen.cursor_position().1 < 24);
    chat.resize(80);
    chat.send(&"\x7f".repeat(100));
    chat.assert_draft("you> ", 5);
    chat.send("final 中文\n");
    chat.wait_for("UNICODE_REPLY");
    chat.exit();
    let project = Project::discover(root.path()).unwrap();
    let state = reduced_session(&project, &session_id(root.path()));
    assert!(
        state
            .history()
            .items()
            .iter()
            .any(|item| *item == echidna::domain::model::ContentItem::user_text("final 中文"))
    );
    f2a_join_server(server, "unicode input");
}

#[test]
fn terminal_editor_typing_during_sampling_preserves_draft_until_enter() {
    let root = TemporaryDirectory::new("terminal-editor-running");
    let harness = TemporaryDirectory::new("terminal-editor-config");
    initialize_project(root.path());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let config = write_config(
        harness.path(),
        &format!("http://{}/v1", listener.local_addr().unwrap()),
    );
    let (started_tx, started) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let (requests_tx, requests) = mpsc::channel();
    let server = thread::spawn(move || {
        let mut socket = f3_context::accept_before_deadline(&listener, "editable stream");
        requests_tx.send(read_http_request(&mut socket)).unwrap();
        write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\ndata: {{\"type\":\"response.output_text.delta\",\"output_index\":0,\"content_index\":0,\"delta\":\"STREAM_PREFIX\"}}\n\n").unwrap();
        socket.flush().unwrap();
        started_tx.send(()).unwrap();
        released.recv_timeout(Duration::from_secs(10)).unwrap();
        // Completion is released only after typing/arrows/deletion were observed.
        write!(socket, "data: {{\"type\":\"response.output_text.done\",\"output_index\":0,\"content_index\":0,\"text\":\"STREAM_PREFIX_DONE\"}}\n\ndata: {{\"type\":\"response.completed\",\"response\":{{\"id\":\"stream-edited\"}}}}\n\n").unwrap();
        drop(socket);
        let mut socket = f3_context::accept_before_deadline(&listener, "submitted draft");
        requests_tx.send(read_http_request(&mut socket)).unwrap();
        let body = text_response("CORRECTION_DONE", "next-edited");
        write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    });
    let mut chat = TerminalChat::new(root.path(), &config);
    chat.send("first task\n");
    started.recv_timeout(Duration::from_secs(10)).unwrap();
    chat.wait_for("STREAM_PREFIX");
    chat.send("draft 中文zz\x7f\x7f\x1b[DX");
    chat.assert_draft("working> draft 中X文", 18);
    assert!(requests.try_recv().is_ok());
    assert!(requests.try_recv().is_err());
    release.send(()).unwrap();
    chat.wait_for("_DONE");
    chat.wait_for_draft("you> draft 中X文", 14);
    chat.send("\n");
    chat.wait_for("CORRECTION_DONE");
    chat.exit();
    let next = requests.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(next.contains("draft 中X文"));
    assert!(!String::from_utf8_lossy(&chat.captured).contains("[request cancelled]"));
    server.join().unwrap();
}

#[test]
fn terminal_editor_paste_whitespace_is_visible_and_preserves_exact_text() {
    let root = TemporaryDirectory::new("terminal-editor-paste");
    let harness = TemporaryDirectory::new("terminal-editor-config");
    initialize_project(root.path());
    let server = spawn_responses_server(vec![(200, text_response("PASTE_DONE", "paste"))]);
    let config = write_config(harness.path(), &server.base_url);
    let mut chat = TerminalChat::new(root.path(), &config);
    chat.send("\x1b[200~line一\nline二\tend\x1b[201~");
    chat.assert_draft("you> line一\\nline二\\tend", 24);
    assert!(server.requests.try_recv().is_err());
    chat.send("\n");
    chat.wait_for("PASTE_DONE");
    chat.exit();
    let project = Project::discover(root.path()).unwrap();
    let state = reduced_session(&project, &session_id(root.path()));
    assert!(
        state
            .history()
            .items()
            .iter()
            .any(|item| *item
                == echidna::domain::model::ContentItem::user_text("line一\nline二\tend"))
    );
    f2a_join_server(server, "bracketed paste");
}

#[test]
fn terminal_editor_stream_equal_width_and_cjk_wrap_preserve_draft() {
    for prefix in ["x".repeat(71), format!("{}中", "x".repeat(70))] {
        assert_streamed_output_matches_direct(&prefix, "TAIL");
    }
}

#[test]
fn terminal_editor_stream_tab_at_right_margin_preserves_draft_and_output() {
    assert_streamed_output_matches_direct(&format!("{}\t", "x".repeat(69)), "TAIL");
}

#[test]
fn terminal_editor_stream_split_crlf_preserves_draft_and_output() {
    assert_streamed_output_matches_direct("MARKER\r", "\nTAIL");
}

#[test]
fn terminal_editor_stream_newline_after_full_width_preserves_draft_and_output() {
    for newline in ["\r\n", "\n"] {
        assert_streamed_output_matches_direct(&"x".repeat(71), &format!("{newline}TAIL"));
    }
}

fn assert_streamed_output_matches_direct(prefix: &str, suffix: &str) {
    let root = TemporaryDirectory::new("terminal-editor-wrap");
    let harness = TemporaryDirectory::new("terminal-editor-config");
    initialize_project(root.path());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let config = write_config(
        harness.path(),
        &format!("http://{}/v1", listener.local_addr().unwrap()),
    );
    let (release, released) = mpsc::channel();
    let initial = prefix.to_owned();
    let final_text = format!("{prefix}{suffix}");
    let server = thread::spawn(move || {
        let mut socket = f3_context::accept_before_deadline(&listener, "wrapped stream");
        read_http_request(&mut socket);
        let delta = json!({"type":"response.output_text.delta","output_index":0,"content_index":0,"delta":initial});
        write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\ndata: {delta}\n\n").unwrap();
        socket.flush().unwrap();
        released.recv_timeout(Duration::from_secs(10)).unwrap();
        let done = json!({"type":"response.output_text.done","output_index":0,"content_index":0,"text":final_text});
        write!(socket, "data: {done}\n\ndata: {{\"type\":\"response.completed\",\"response\":{{\"id\":\"wrapped\"}}}}\n\n").unwrap();
    });
    let mut chat = TerminalChat::new(root.path(), &config);
    chat.send("wrap stream\n");
    chat.wait_for(prefix);
    chat.send("kept 中文");
    chat.assert_draft("working> kept 中文", 18);
    release.send(()).unwrap();
    chat.wait_for("TAIL");
    chat.wait_for_draft("you> kept 中文", 14);
    let screen = chat.screen();
    let rows = screen.rows(0, 80).collect::<Vec<_>>();
    let mut direct = vt100::Parser::new(24, 80, 0);
    // The original canonical terminal translates each LF to CRLF. The raw
    // editor's presentation must match that same physical screen contract.
    direct.process(
        format!("Echidna> {prefix}{suffix}\r\n")
            .replace('\n', "\r\n")
            .as_bytes(),
    );
    let expected = direct
        .screen()
        .rows(0, 80)
        .take_while(|row| !row.is_empty())
        .collect::<Vec<_>>();
    let answer = rows
        .iter()
        .position(|row| row.starts_with("Echidna> "))
        .unwrap_or_else(|| panic!("assistant line disappeared: {rows:?}"));
    assert_eq!(
        &rows[answer..answer + expected.len()],
        expected.as_slice(),
        "prefix={prefix:?} suffix={suffix:?}"
    );
    chat.send(&"\x7f".repeat(7));
    chat.exit();
    server.join().unwrap();
    let project = Project::discover(root.path()).unwrap();
    let state = reduced_session(&project, &session_id(root.path()));
    assert_eq!(
        state.history().items().last(),
        Some(&echidna::domain::model::ContentItem::assistant_text(
            format!("{prefix}{suffix}")
        ))
    );
}

#[test]
fn terminal_editor_ctrl_c_and_invalid_paste_restore_terminal_without_submitting() {
    for input in ["unfinished\x03", "unfinished\x1b[200~\u{fffd}\x1b[201~"] {
        let root = TemporaryDirectory::new("terminal-editor-exit");
        let harness = TemporaryDirectory::new("terminal-editor-config");
        initialize_project(root.path());
        let config = write_config(harness.path(), "http://127.0.0.1:1/v1");
        let mut chat = TerminalChat::new(root.path(), &config);
        chat.send(input);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = chat.child.try_wait().unwrap() {
                assert_eq!(status.success(), input.contains('\x03'));
                break;
            }
            assert!(Instant::now() < deadline, "editor did not restore/exit");
            thread::sleep(Duration::from_millis(10));
        }
        chat.assert_restored();
        let project = Project::discover(root.path()).unwrap();
        let state = reduced_session(&project, &session_id(root.path()));
        assert!(state.history().items().is_empty());
        assert!(state.request_audits().is_empty());
    }
}

#[test]
fn terminal_editor_large_paste_remains_responsive_in_a_narrow_viewport() {
    let root = TemporaryDirectory::new("terminal-editor-large-paste");
    let harness = TemporaryDirectory::new("terminal-editor-config");
    initialize_project(root.path());
    let config = write_config(harness.path(), "http://127.0.0.1:1/v1");
    let mut chat = TerminalChat::new(root.path(), &config);
    chat.resize(24);
    let started = Instant::now();
    chat.send(&format!(
        "\x1b[200~{}PASTE_END\x1b[201~",
        "a".repeat(100_000)
    ));
    chat.wait_for("PASTE_END");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "large paste stalled the editor: {:?}",
        started.elapsed()
    );
    let screen = chat.screen();
    assert!(screen.cursor_position().1 < 24);
    chat.send("\x03");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = chat.child.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(Instant::now() < deadline, "large draft prevented Ctrl-C");
        thread::sleep(Duration::from_millis(10));
    }
    chat.assert_restored();
    let project = Project::discover(root.path()).unwrap();
    let state = reduced_session(&project, &session_id(root.path()));
    assert!(state.history().items().is_empty());
    assert!(state.request_audits().is_empty());
}

#[test]
fn terminal_editor_tool_preview_preserves_unsubmitted_draft_and_real_receipt() {
    let root = TemporaryDirectory::new("terminal-editor-tools");
    let harness = TemporaryDirectory::new("terminal-editor-config");
    initialize_project(root.path());
    let server = spawn_responses_server(vec![
        (
            200,
            m4_long_flow_tool_response(
                "editable-tool",
                "run_commands",
                json!({"mode":"serial","commands":[
                    {"argv":["sh","-c","printf PREVIEW_ONE"]},
                    {"argv":["sh","-c","while [ ! -e release ]; do sleep 0.02; done; printf PREVIEW_TWO"]}
                ]}),
                "editable-tool-response",
            ),
        ),
        (200, text_response("TOOL_FINISHED", "tool-finished")),
        (200, text_response("DRAFT_FINISHED", "draft-finished")),
    ]);
    let config = write_config(harness.path(), &server.base_url);
    let mut chat = TerminalChat::new(root.path(), &config);
    chat.send("run two bounded commands\n");
    let _ = f2a_request_body(&server.requests, "tool request");
    chat.wait_for("子命令 1/2");
    chat.send("next draftzz\x7f\x7f");
    chat.assert_draft("working> next draft", 19);
    assert!(server.requests.try_recv().is_err());
    fs::write(root.path().join("release"), b"release").unwrap();
    chat.wait_for("TOOL_FINISHED");
    chat.wait_for_draft("you> next draft", 15);
    chat.send("\n");
    chat.wait_for("DRAFT_FINISHED");
    chat.exit();
    let project = Project::discover(root.path()).unwrap();
    let state = reduced_session(&project, &session_id(root.path()));
    assert_eq!(state.execution_receipt_count(), 1);
    assert_eq!(
        state
            .history()
            .items()
            .iter()
            .filter(|item| **item == echidna::domain::model::ContentItem::user_text("next draft"))
            .count(),
        1
    );
    f2a_join_server(server, "editable tool previews");
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
        let has_sgr = captured
            .split("\x1b[")
            .skip(1)
            .any(|sequence| sequence.chars().find(|c| ('@'..='~').contains(c)) == Some('m'));
        assert_eq!(has_sgr, colored, "{captured}");
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
    let deadline = Instant::now() + Duration::from_secs(5);
    while chat.child.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
    chat.assert_restored();
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
    chat.assert_restored();
    assert!(!String::from_utf8_lossy(&chat.captured).contains("[Follow-up accepted"));
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
        let mut chat = if interruption == "invalid_utf8" {
            TerminalChat::with_theme_environment(root.path(), &config, None, "dumb")
        } else {
            TerminalChat::new(root.path(), &config)
        };
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
            chat.send("only check connection and stpzz\x7f\x7f\x1b[Do\x1b[C");
            chat.assert_draft("working> only check connection and stop", 39);
            assert!(server.requests.try_recv().is_err());
            assert!(!String::from_utf8_lossy(&chat.captured).contains("[request cancelled]"));
            chat.send("\n");
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
        if interruption == "invalid_utf8" {
            let captured = String::from_utf8_lossy(&chat.captured);
            assert!(!captured.contains("working>"));
            assert!(!captured.contains("[Follow-up accepted"));
        }
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
