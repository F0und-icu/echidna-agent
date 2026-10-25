//! Terminal steering uses the same cancellation boundary as Ctrl+C.

use std::future::Future;
use std::io;

use crossterm::event::{Event, EventStream, KeyCode, KeyEventKind, KeyModifiers};
use futures_util::StreamExt;
use tokio::io::{AsyncBufReadExt, BufReader, Lines, Stdin};

use crate::app::cancellation::AgentCancellation;
use crate::terminal_console::{Edit, InputMode, Terminal};

pub(super) enum ChatInput {
    Lines(Lines<BufReader<Stdin>>),
    Terminal {
        terminal: Terminal,
        events: EventStream,
    },
}

impl ChatInput {
    pub(super) fn new() -> io::Result<Self> {
        Ok(match Terminal::start()? {
            Some(terminal) => Self::Terminal {
                terminal,
                events: EventStream::new(),
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

    pub(super) fn processing(&self, follow_up: bool) -> io::Result<()> {
        self.prompt(InputMode::Processing)?;
        if follow_up && matches!(self, Self::Terminal { .. }) {
            use std::io::Write;
            writeln!(
                crate::terminal_console::stderr(),
                "[Follow-up accepted; starting processing]"
            )?;
        }
        Ok(())
    }

    pub(super) async fn next_line(&mut self) -> io::Result<Option<String>> {
        let Self::Terminal { terminal, events } = self else {
            let Self::Lines(lines) = self else {
                unreachable!()
            };
            return lines.next_line().await;
        };
        loop {
            terminal.drain_previews()?;
            let event = tokio::select! {
                event = events.next() => match event {
                    Some(Ok(event)) => event,
                    result => {
                        terminal.edit(Edit::Clear)?;
                        return match result {
                            Some(Err(error)) => Err(error),
                            None => Err(io::Error::new(io::ErrorKind::UnexpectedEof, "terminal event stream ended")),
                            Some(Ok(_)) => unreachable!(),
                        };
                    }
                },
                _ = terminal.wake.notified() => continue,
            };
            let edit = match event {
                Event::Key(key) if key.kind != KeyEventKind::Release => match key.code {
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        terminal.edit(Edit::Clear)?;
                        return Err(io::Error::new(io::ErrorKind::Interrupted, "Ctrl-C"));
                    }
                    KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        if terminal.empty()? {
                            return Ok(None);
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
                    _ => continue,
                },
                Event::Paste(text) => {
                    // crossterm uses lossy UTF-8 for bracketed paste. Reject
                    // its ambiguous replacement marker rather than send guessed bytes.
                    if text.contains('\u{fffd}') {
                        terminal.edit(Edit::Clear)?;
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "paste contains an ambiguous UTF-8 replacement character",
                        ));
                    }
                    if text
                        .chars()
                        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
                    {
                        terminal.edit(Edit::Clear)?;
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "paste contains terminal control characters",
                        ));
                    }
                    Edit::Insert(text)
                }
                Event::Resize(columns, _) => Edit::Resize(columns),
                _ => continue,
            };
            if let Some(line) = terminal.edit(edit)? {
                return Ok(Some(line));
            }
        }
    }

    pub(super) fn end_paste(&self) -> io::Result<()> {
        if let Self::Terminal { terminal, .. } = self {
            terminal.mode(InputMode::Task)?;
        }
        Ok(())
    }
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
