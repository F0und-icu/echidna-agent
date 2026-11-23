//! Best-effort bounded tool presentation; never an execution acknowledgement.

use crate::chat_display::{Event, JsonLines};
use crate::domain::tools::{ToolCallId, ToolResultStatus};
use std::fs::File;
use std::io::{self, Write};
use std::os::fd::AsFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::time::Duration;

pub(super) enum PresentationSink {
    JsonLines(JsonLines),
    Terminal(crate::terminal_console::PreviewSender),
    Worker {
        sender: Option<SyncSender<String>>,
        dropped: Arc<AtomicU64>,
        finished: Receiver<()>,
    },
}

impl Default for PresentationSink {
    fn default() -> Self {
        if let Some(stream) = JsonLines::current() {
            return Self::JsonLines(stream);
        }
        if let Some(terminal) = crate::terminal_console::preview_sender() {
            return Self::Terminal(terminal);
        }
        let (sender, frames) = mpsc::sync_channel::<String>(8);
        let (finished, completion) = mpsc::channel();
        let dropped = Arc::new(AtomicU64::new(0));
        let missed = Arc::clone(&dropped);
        // A duplicated descriptor avoids holding Rust's global stderr lock
        // while a slow terminal blocks this worker. No flags are changed.
        let writer = io::stderr().as_fd().try_clone_to_owned().map(File::from);
        let sender =
            writer.ok().and_then(|mut writer| {
                std::thread::Builder::new()
                    .name("tool-presentation".to_owned())
                    .spawn(move || {
                        for frame in frames {
                            let count = missed.swap(0, Ordering::Relaxed);
                            if count > 0 && writeln!(writer,
                            "[Tool presentation congested: skipped {count} previews; execution facts remain in the Session record]"
                        ).is_err() { break; }
                            if writer.write_all(frame.as_bytes()).is_err() {
                                break;
                            }
                        }
                        let _ = finished.send(());
                    })
                    .ok()
                    .map(|_| sender)
            });
        Self::Worker {
            sender,
            dropped,
            finished: completion,
        }
    }
}

impl PresentationSink {
    pub(super) fn send(
        &self,
        call_id: &ToolCallId,
        name: &str,
        stage: &'static str,
        status: Option<ToolResultStatus>,
        frame: String,
    ) {
        match self {
            Self::JsonLines(stream) => stream.preview(Event::Tool {
                call_id: call_id.as_str(),
                name,
                stage,
                status,
                text: &frame,
            }),
            Self::Terminal(terminal) => terminal.send(frame),
            Self::Worker {
                sender, dropped, ..
            } => {
                if let Some(sender) = sender
                    && sender.try_send(frame).is_err()
                {
                    dropped.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    }
}

impl Drop for PresentationSink {
    fn drop(&mut self) {
        let Self::Worker {
            sender, finished, ..
        } = self
        else {
            return;
        };
        sender.take();
        // Normal output drains; an unread terminal never creates an unbounded
        // join. Exactly one worker exists per Chat, not one per Turn or Tool.
        let _ = finished.recv_timeout(Duration::from_millis(100));
    }
}
