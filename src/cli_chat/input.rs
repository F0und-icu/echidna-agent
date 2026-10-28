//! Terminal steering uses the same cancellation boundary as Ctrl+C.

use std::future::Future;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use tokio::io::{AsyncBufReadExt, BufReader, Lines, Stdin};
use tokio::sync::mpsc;

use crate::app::cancellation::AgentCancellation;
use crate::terminal_console::{Edit, InputMode, Terminal, TerminalEditor};

pub(super) enum ChatInput {
    Lines(Lines<BufReader<Stdin>>),
    Terminal {
        // Join the only input reader before the terminal restoration guard.
        reader: TerminalReader,
        terminal: Terminal,
    },
}

impl ChatInput {
    pub(super) fn new() -> io::Result<Self> {
        Ok(match Terminal::start()? {
            Some((terminal, editor)) => Self::Terminal {
                reader: TerminalReader::start(editor)?,
                terminal,
            },
            None => Self::Lines(BufReader::new(tokio::io::stdin()).lines()),
        })
    }

    pub(super) fn prompt(&self, mode: InputMode) -> io::Result<()> {
        match self {
            Self::Terminal { terminal, .. } => terminal.mode(mode),
            Self::Lines(_) if mode == InputMode::Processing => Ok(()),
            Self::Lines(_) => {
                use std::io::Write;
                write!(io::stdout(), "{}", mode.prompt())?;
                io::stdout().flush()
            }
        }
    }

    pub(super) fn processing(&self) -> io::Result<()> {
        self.prompt(InputMode::Processing)
    }

    pub(super) fn is_interactive(&self) -> bool {
        matches!(self, Self::Terminal { .. })
    }

    pub(super) async fn next_line(&mut self) -> io::Result<Option<String>> {
        let Self::Terminal { terminal, reader } = self else {
            let Self::Lines(lines) = self else {
                unreachable!()
            };
            return lines.next_line().await;
        };
        loop {
            let line = reader.received.recv().await.ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "terminal input reader stopped",
                )
            })??;
            if line.as_ref().is_some_and(|line| line.trim().is_empty())
                && terminal.input_mode()? != InputMode::Paste
            {
                continue;
            }
            return Ok(line);
        }
    }

    pub(super) fn end_paste(&self) -> io::Result<()> {
        if let Self::Terminal { terminal, .. } = self {
            terminal.mode(InputMode::Task)?;
        }
        Ok(())
    }
}

pub(super) fn report_follow_up_accepted() -> io::Result<()> {
    use std::io::Write;
    writeln!(
        crate::terminal_console::stderr(),
        "[Follow-up accepted; starting processing]"
    )
}

pub(super) struct TerminalReader {
    received: mpsc::Receiver<io::Result<Option<String>>>,
    stopping: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl TerminalReader {
    fn start(editor: TerminalEditor) -> io::Result<Self> {
        let (sender, received) = mpsc::channel(1);
        let stopping = Arc::new(AtomicBool::new(false));
        let worker_stopping = Arc::clone(&stopping);
        let worker = thread::Builder::new()
            .name("echidna-terminal-input".to_owned())
            .spawn(move || {
                if let Err(error) = Self::read(editor, &sender, &worker_stopping) {
                    // The receiver is closed only when the Chat itself exits.
                    let _ = sender.blocking_send(Err(error));
                }
            })?;
        Ok(Self {
            received,
            stopping,
            worker: Some(worker),
        })
    }

    fn read(
        editor: TerminalEditor,
        sender: &mpsc::Sender<io::Result<Option<String>>>,
        stopping: &AtomicBool,
    ) -> io::Result<()> {
        while !stopping.load(Ordering::Acquire) {
            editor.drain_previews()?;
            if !event::poll(Duration::from_millis(20))? {
                continue;
            }
            let message = match Self::apply(&editor, event::read()?)? {
                ReaderEvent::Edited => continue,
                ReaderEvent::Line(line) => Ok(Some(line)),
                ReaderEvent::Rejected(error) => Err(error),
                ReaderEvent::Eof => {
                    let _ = sender.blocking_send(Ok(None));
                    return Ok(());
                }
            };
            if sender.blocking_send(message).is_err() {
                return Ok(());
            }
        }
        Ok(())
    }

    fn apply(terminal: &TerminalEditor, event: Event) -> io::Result<ReaderEvent> {
        let edit = match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => match key.code {
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    terminal.edit(Edit::Clear)?;
                    return Ok(ReaderEvent::Rejected(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "Ctrl-C",
                    )));
                }
                KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    if terminal.empty()? {
                        return Ok(ReaderEvent::Eof);
                    }
                    Edit::Delete
                }
                KeyCode::Char(text)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                        && !text.is_control() =>
                {
                    Edit::Insert(text.to_string())
                }
                KeyCode::Enter => Edit::Submit,
                KeyCode::Char('j' | 'm') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    Edit::Submit
                }
                KeyCode::Left => Edit::Left,
                KeyCode::Right => Edit::Right,
                KeyCode::Home => Edit::Home,
                KeyCode::End => Edit::End,
                KeyCode::Backspace => Edit::Backspace,
                KeyCode::Delete => Edit::Delete,
                _ => return Ok(ReaderEvent::Edited),
            },
            Event::Paste(text) => {
                // crossterm uses lossy UTF-8 for bracketed paste. Reject
                // its ambiguous replacement marker rather than send guessed bytes.
                if text.contains('\u{fffd}') {
                    terminal.edit(Edit::Clear)?;
                    return Ok(ReaderEvent::Rejected(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "paste contains an ambiguous UTF-8 replacement character",
                    )));
                }
                if text
                    .chars()
                    .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
                {
                    terminal.edit(Edit::Clear)?;
                    return Ok(ReaderEvent::Rejected(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "paste contains terminal control characters",
                    )));
                }
                Edit::Insert(text)
            }
            Event::Resize(columns, _) => Edit::Resize(columns),
            _ => return Ok(ReaderEvent::Edited),
        };
        Ok(match terminal.edit(edit)? {
            Some(line) => ReaderEvent::Line(line),
            None => ReaderEvent::Edited,
        })
    }
}

impl Drop for TerminalReader {
    fn drop(&mut self) {
        self.received.close();
        self.stopping.store(true, Ordering::Release);
        self.worker
            .take()
            .expect("terminal input reader joins once")
            .join()
            .expect("terminal input reader panicked");
    }
}

enum ReaderEvent {
    Edited,
    Line(String),
    Rejected(io::Error),
    Eof,
}

/// Pipes keep their sequential input contract. Only actual terminal lines
/// interrupt a running Turn; a queued line is processed by the normal CLI
/// boundary after cancellation settles, never inside the sampling plan.
pub(super) async fn interruptible<F>(
    operation: F,
    cancellation: &AgentCancellation,
    lines: &mut ChatInput,
    terminal: bool,
    closed: &mut bool,
    pending: &mut Option<String>,
) -> io::Result<F::Output>
where
    F: Future,
{
    tokio::pin!(operation);
    let mut read_terminal = terminal && !*closed;
    loop {
        tokio::select! {
            // A ready terminal correction takes precedence over a ready next
            // model/tool poll. No already-started effect is claimed undone.
            biased;
            signal = tokio::signal::ctrl_c() => {
                signal?;
                cancellation.cancel();
                return Ok(operation.await);
            }
            line = lines.next_line(), if read_terminal => {
                match line {
                    Ok(Some(line)) if line.trim().is_empty() => continue,
                    Ok(Some(line)) => {
                        if line != "/stop" {
                            *pending = Some(line);
                        }
                        console_eprintln!("\n[interrupt requested] Waiting for the current operation to settle; no rollback or background-process stop is implied.");
                        cancellation.cancel();
                        return Ok(operation.await);
                    }
                    Ok(None) => {
                        *closed = true;
                        read_terminal = false;
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                        cancellation.cancel();
                        return Ok(operation.await);
                    }
                    Err(error) => {
                        console_eprintln!("[input rejected] {error}; cancelling the current Turn. Re-enter your instruction after the stop.");
                        cancellation.cancel();
                        // Preserve an App/durability error from cancellation;
                        // a malformed terminal line must not mask its outcome.
                        return Ok(operation.await);
                    }
                }
            }
            result = &mut operation => return Ok(result),
        }
    }
}

pub(super) fn reject_pending(pending: &mut Option<String>) {
    if pending.take().is_some() {
        console_eprintln!(
            "[queued input not submitted or saved] Resolve the stop, then re-enter your instruction explicitly. No automatic retry."
        );
    }
}
