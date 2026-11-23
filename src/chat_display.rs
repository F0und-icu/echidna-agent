//! Process-facing Chat presentation. Only the JSON Lines writer owns framed stdout.
use std::fmt;
use std::io::{self, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{SyncSender, sync_channel};

use serde::Serialize;

use crate::CliError;
use crate::domain::context::ProviderSessionRole;
use crate::domain::model::ModelUsage;
use crate::domain::tools::ToolResultStatus;

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum Event<'a> {
    Session {
        session_id: &'a str,
        disposition: &'a str,
    },
    Input {
        mode: &'a str,
    },
    AssistantDelta {
        text: &'a str,
    },
    Turn {
        state: &'a str,
    },
    Message {
        text: &'a str,
    },
    Diagnostic {
        text: &'a str,
    },
    ModelLog {
        role: &'a str,
        request_id: Option<u64>,
        text: &'a str,
    },
    Sample {
        role: &'a str,
        request_id: u64,
        state: &'a str,
    },
    Mcp {
        text: &'a str,
    },
    Tool {
        call_id: &'a str,
        name: &'a str,
        stage: &'a str,
        status: Option<ToolResultStatus>,
        text: &'a str,
    },
    PreviewsSkipped {
        count: u64,
    },
    Usage {
        role: ProviderSessionRole,
        audit_id: &'a str,
        usage: ModelUsage,
    },
    Timing {
        phase: &'a str,
        stage_ms: u128,
        turn_ms: u128,
    },
    Error {
        text: &'a str,
    },
}

impl Event<'_> {
    fn line(&self) -> Vec<u8> {
        #[derive(Serialize)]
        struct Record<'a> {
            v: u8,
            #[serde(flatten)]
            event: &'a Event<'a>,
        }
        let mut line = serde_json::to_vec(&Record { v: 1, event: self })
            .expect("display records have JSON-compatible fields");
        line.push(b'\n');
        line
    }
}

#[derive(Clone)]
pub(crate) struct JsonLines {
    frames: SyncSender<Vec<u8>>,
    dropped: Arc<AtomicU64>,
}

tokio::task_local! { static STREAM: JsonLines; }

impl JsonLines {
    pub(crate) fn current() -> Option<Self> {
        STREAM.try_with(Clone::clone).ok()
    }

    pub(crate) fn send(&self, event: Event<'_>) -> io::Result<()> {
        self.frames
            .send(event.line())
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "Chat display closed"))
    }

    pub(crate) fn preview(&self, event: Event<'_>) {
        if self.frames.try_send(event.line()).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(crate) fn run(chat: impl FnOnce() -> Result<(), CliError>) -> Result<(), CliError> {
        let (frames, received) = sync_channel::<Vec<u8>>(8);
        let dropped = Arc::new(AtomicU64::new(0));
        let stream = Self {
            frames,
            dropped: dropped.clone(),
        };
        let worker = std::thread::Builder::new()
            .name("chat-jsonl".into())
            .spawn(move || -> io::Result<()> {
                let mut out = io::stdout();
                let report_drops = |out: &mut io::Stdout| -> io::Result<()> {
                    let count = dropped.swap(0, Ordering::Relaxed);
                    if count > 0 {
                        out.write_all(&Event::PreviewsSkipped { count }.line())?;
                    }
                    Ok(())
                };
                for line in received {
                    report_drops(&mut out)?;
                    out.write_all(&line)?;
                    out.flush()?;
                }
                report_drops(&mut out)?;
                out.flush()
            })
            .map_err(CliError::ChatDisplay)?;
        // The CLI drives its root future on this thread. Cross-thread observers
        // capture the sender when composed, including callbacks during cleanup.
        let result = STREAM.sync_scope(stream, || {
            let result = chat();
            if let Err(error) = &result {
                STREAM.with(|s| {
                    let _ = s.send(Event::Error {
                        text: &error.to_string(),
                    });
                });
            }
            result
        });
        let written = worker.join().expect("Chat display writer panicked");
        result.and(written.map_err(CliError::ChatDisplay))
    }
}

// A formatting call is one record, including embedded newlines. Request and
// MCP observers retain their own sender across callback/thread boundaries.
#[derive(Clone, Copy)]
pub(crate) enum Log {
    Message,
    Diagnostic,
    Model {
        role: &'static str,
        request_id: Option<u64>,
    },
    Mcp,
}

pub(crate) struct LogWriter {
    pub(crate) stream: Option<JsonLines>,
    pub(crate) log: Log,
    pub(crate) stderr: bool,
}

impl Write for LogWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if let Some(stream) = &self.stream {
            let text = std::str::from_utf8(bytes).map_err(io::Error::other)?;
            stream.send(match self.log {
                Log::Message => Event::Message { text },
                Log::Diagnostic => Event::Diagnostic { text },
                Log::Model { role, request_id } => Event::ModelLog {
                    role,
                    request_id,
                    text,
                },
                Log::Mcp => Event::Mcp { text },
            })?;
        } else if self.stderr {
            crate::terminal_console::stderr().write_all(bytes)?;
        } else {
            crate::terminal_console::stdout().write_all(bytes)?;
        }
        Ok(bytes.len())
    }

    fn write_fmt(&mut self, args: fmt::Arguments<'_>) -> io::Result<()> {
        self.write_all(args.to_string().as_bytes())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.stream.is_some() {
            Ok(())
        } else if self.stderr {
            crate::terminal_console::stderr().flush()
        } else {
            crate::terminal_console::stdout().flush()
        }
    }
}

pub(crate) fn print(args: fmt::Arguments<'_>, stderr: bool) {
    let stream = JsonLines::current();
    let machine = stream.is_some();
    let result = LogWriter {
        stream,
        log: if stderr {
            Log::Diagnostic
        } else {
            Log::Message
        },
        stderr,
    }
    .write_fmt(args);
    // The JSON writer reports its actual I/O error when the Chat exits.
    if !machine {
        result.expect("CLI output failed");
    }
}

pub(crate) fn model_log(
    stream: &Option<JsonLines>,
    role: &'static str,
    request_id: Option<u64>,
) -> LogWriter {
    LogWriter {
        stream: stream.clone(),
        log: Log::Model { role, request_id },
        stderr: true,
    }
}
