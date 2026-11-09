//! Actual input sources in an isolated PTY through the shared private harness.
use std::io::{Read, Write};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::{app, terminal_console};

macro_rules! console_eprintln {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        writeln!(crate::terminal_console::stderr(), $($arg)*).unwrap();
    }};
}

// Exercise the private input owner in the PTY probe without widening CLI visibility.
#[allow(dead_code, clippy::duplicate_mod)]
#[path = "../../src/cli_chat/input.rs"]
mod input;

#[test]
fn subprocess_terminal_input_reader_probe() {
    let Ok(mode) = std::env::var("ECHIDNA_TERMINAL_INPUT_PROBE") else {
        return;
    };
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let mut lines = input::ChatInput::new().unwrap();
            lines.processing(false).unwrap();
            if mode == "full-handoff" {
                writeln!(terminal_console::stdout(), "SYNC_STALL_READY").unwrap();
                thread::sleep(Duration::from_secs(2));
                drop(lines);
            } else {
                assert_eq!(mode, "visible-draft");
                let cancellation = app::cancellation::AgentCancellation::new();
                let operation_cancellation = cancellation.clone();
                let operation = async {
                    writeln!(terminal_console::stdout(), "SYNC_STALL_READY").unwrap();
                    thread::sleep(Duration::from_secs(2));
                    writeln!(terminal_console::stdout(), "SYNC_STALL_DONE").unwrap();
                    while !operation_cancellation.is_cancelled() {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                    writeln!(terminal_console::stdout(), "OPERATION_SETTLED").unwrap();
                };
                let mut closed = false;
                let mut pending = None;
                input::interruptible(
                    operation,
                    &cancellation,
                    &mut lines,
                    true,
                    &mut closed,
                    &mut pending,
                )
                .await
                .unwrap();
                assert!(cancellation.is_cancelled());
                assert!(!closed);
                assert_eq!(
                    pending.as_ref().map(|input| input.text.as_str()),
                    Some("ab中文!🙂")
                );
                assert!(
                    tokio::time::timeout(Duration::from_millis(50), lines.next_line())
                        .await
                        .is_err(),
                    "the submitted correction was delivered again"
                );
                drop(lines);
            }
            println!("PROBE_COMPLETED");
        });
}

struct Probe {
    child: Box<dyn portable_pty::Child + Send + Sync>,
    writer: Box<dyn Write + Send>,
    output: Receiver<Vec<u8>>,
    captured: Vec<u8>,
    parser: vt100::Parser,
    offset: usize,
    master: Box<dyn MasterPty + Send>,
}

impl Probe {
    fn new(mode: &str, columns: u16) -> Self {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 24,
                cols: columns,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut command = CommandBuilder::new(std::env::current_exe().unwrap());
        command.args([
            "--exact",
            "terminal_input::subprocess_terminal_input_reader_probe",
            "--nocapture",
        ]);
        command.env("ECHIDNA_TERMINAL_INPUT_PROBE", mode);
        command.env("TERM", "xterm-256color");
        let child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);
        let writer = pair.master.take_writer().unwrap();
        let mut reader = pair.master.try_clone_reader().unwrap();
        let (sender, output) = mpsc::channel();
        thread::spawn(move || {
            let mut bytes = [0; 4096];
            while let Ok(count) = reader.read(&mut bytes) {
                if count == 0 || sender.send(bytes[..count].to_vec()).is_err() {
                    break;
                }
            }
        });
        let mut probe = Self {
            child,
            writer,
            output,
            captured: Vec::new(),
            parser: vt100::Parser::new(24, columns, 0),
            offset: 0,
            master: pair.master,
        };
        probe.wait_for("SYNC_STALL_READY", Duration::from_secs(5));
        probe
    }

    fn send(&mut self, text: &str) {
        self.writer.write_all(text.as_bytes()).unwrap();
        self.writer.flush().unwrap();
    }

    fn collect(&mut self) {
        if let Ok(bytes) = self.output.recv_timeout(Duration::from_millis(10)) {
            self.captured.extend(bytes);
        }
        while let Ok(bytes) = self.output.try_recv() {
            self.captured.extend(bytes);
        }
        self.parser.process(&self.captured[self.offset..]);
        self.offset = self.captured.len();
    }

    fn wait_for(&mut self, text: &str, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while !String::from_utf8_lossy(&self.captured).contains(text) {
            self.collect();
            assert!(
                Instant::now() < deadline,
                "probe did not show {text:?}: {}",
                String::from_utf8_lossy(&self.captured)
            );
        }
    }

    fn finish(&mut self) {
        self.wait_for("PROBE_COMPLETED", Duration::from_secs(5));
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(
                    status.success(),
                    "{}",
                    String::from_utf8_lossy(&self.captured)
                );
                break;
            }
            assert!(Instant::now() < deadline, "terminal reader did not exit");
            thread::sleep(Duration::from_millis(10));
        }
        self.collect();
        let termios = self.master.get_termios().unwrap();
        assert_ne!(termios.local_flags.bits() & libc::ICANON, 0);
        assert_ne!(termios.local_flags.bits() & libc::ECHO, 0);
        assert!(String::from_utf8_lossy(&self.captured).contains("\x1b[?2004l"));
    }
}

impl Drop for Probe {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn terminal_input_draft_stays_visible_during_synchronous_work() {
    for columns in [80, 24] {
        let mut probe = Probe::new("visible-draft", columns);
        probe.send("ab中文🙂zz\x7f\x7f\x1b[D!\x1b[C");
        let deadline = Instant::now() + Duration::from_millis(500);
        loop {
            probe.collect();
            let screen = probe.parser.screen();
            let (row, column) = screen.cursor_position();
            if screen.rows(0, columns).nth(row as usize).as_deref() == Some("working> ab中文!🙂")
                && column == 18
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "draft was hidden by synchronous work at {columns} columns: {}",
                screen.contents()
            );
        }
        assert!(!String::from_utf8_lossy(&probe.captured).contains("SYNC_STALL_DONE"));
        probe.send("\n");
        probe.finish();
        assert!(String::from_utf8_lossy(&probe.captured).contains("OPERATION_SETTLED"));
    }
}

#[test]
fn terminal_input_full_handoff_does_not_trap_shutdown() {
    let mut probe = Probe::new("full-handoff", 80);
    probe.send("one\ntwo\nthree\n");
    probe.wait_for("working> two", Duration::from_millis(500));
    probe.finish();
}
