//! Best-effort bounded tool presentation; never an execution acknowledgement.

use std::fs::File;
use std::io::{self, Write};
use std::os::fd::AsFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::time::Duration;

pub(super) struct PresentationSink {
    sender: Option<SyncSender<String>>,
    dropped: Arc<AtomicU64>,
    finished: Receiver<()>,
}

impl Default for PresentationSink {
    fn default() -> Self {
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
                            "[工具展示拥塞：跳过 {count} 条预览；执行事实以 Session 记录为准]"
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
        Self {
            sender,
            dropped,
            finished: completion,
        }
    }
}

impl PresentationSink {
    pub(super) fn send(&self, frame: String) {
        if let Some(sender) = &self.sender
            && sender.try_send(frame).is_err()
        {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

impl Drop for PresentationSink {
    fn drop(&mut self) {
        self.sender.take();
        // Normal output drains; an unread terminal never creates an unbounded
        // join. Exactly one worker exists per Chat, not one per Turn or Tool.
        let _ = self.finished.recv_timeout(Duration::from_millis(100));
    }
}
