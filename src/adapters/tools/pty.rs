//! Current-platform interactive PTY sessions backed by `portable-pty`.

mod control;
mod launch;
mod supervisor;

use std::collections::BTreeMap;
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use portable_pty::{ChildKiller, MasterPty};
use tokio::sync::watch;

use crate::domain::tools::{PtySessionCompletion, PtySessionId, PtySessionStatus};

pub(super) use control::{
    execute_input, execute_poll, execute_resize, execute_stop, normalize_input, normalize_poll,
    normalize_resize, normalize_stop,
};
pub(super) use launch::{execute_start, normalize_start};
pub(in crate::adapters::tools::pty) use supervisor::PtySupervisor;

use super::output::CapturedOutput;

type PtyWriter = Box<dyn Write + Send>;
type PtyMaster = Box<dyn MasterPty + Send>;
type PtyKiller = Box<dyn ChildKiller + Send + Sync>;

/// The live PTY table is deliberately process-local. Session persistence and
/// restart recovery require the later Event/Project Slice rather than an
/// unsafe claim that an OS terminal survives a new Cyber Agent process.
pub(super) struct PtySessions {
    pub(in crate::adapters::tools::pty) sessions: Arc<Mutex<PtySessionTable>>,
    next_session_sequence: AtomicU64,
}

impl PtySessions {
    pub(super) fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(PtySessionTable::default())),
            next_session_sequence: AtomicU64::new(0),
        }
    }

    pub(in crate::adapters::tools::pty) fn next_session_id(&self) -> PtySessionId {
        let sequence = self.next_session_sequence.fetch_add(1, Ordering::Relaxed) + 1;
        PtySessionId::new(format!("pty:{sequence}"))
    }

    pub(in crate::adapters::tools::pty) fn insert(
        &self,
        session_id: PtySessionId,
        entry: PtySessionEntry,
    ) {
        lock_table(&self.sessions).entries.insert(session_id, entry);
    }
}

#[derive(Default)]
pub(in crate::adapters::tools::pty) struct PtySessionTable {
    pub(in crate::adapters::tools::pty) entries: BTreeMap<PtySessionId, PtySessionEntry>,
}

pub(in crate::adapters::tools::pty) struct PtySessionEntry {
    pub(in crate::adapters::tools::pty) status: PtySessionStatus,
    pub(in crate::adapters::tools::pty) output: Arc<Mutex<CapturedOutput>>,
    pub(in crate::adapters::tools::pty) writer: Option<Arc<Mutex<PtyWriter>>>,
    pub(in crate::adapters::tools::pty) master: Option<Arc<Mutex<PtyMaster>>>,
    pub(in crate::adapters::tools::pty) killer: Option<Arc<Mutex<PtyKiller>>>,
    pub(in crate::adapters::tools::pty) terminal: watch::Sender<()>,
    pub(in crate::adapters::tools::pty) completion: Option<PtySessionCompletion>,
}

impl PtySessionEntry {
    pub(in crate::adapters::tools::pty) fn request_stop(
        &mut self,
    ) -> Option<Arc<Mutex<PtyKiller>>> {
        if self.status != PtySessionStatus::Running {
            return None;
        }
        self.status = PtySessionStatus::Stopping;
        self.killer.clone()
    }

    pub(in crate::adapters::tools::pty) fn terminal_receiver(&self) -> watch::Receiver<()> {
        self.terminal.subscribe()
    }

    pub(in crate::adapters::tools::pty) fn close_io(&mut self) {
        self.writer = None;
        self.master = None;
        self.killer = None;
    }

    pub(in crate::adapters::tools::pty) fn publish_completion(
        &mut self,
        completion: PtySessionCompletion,
    ) {
        self.status = completion.status;
        self.close_io();
        self.completion = Some(completion);
        self.terminal.send_replace(());
    }
}

pub(in crate::adapters::tools::pty) fn lock_table(
    table: &Arc<Mutex<PtySessionTable>>,
) -> MutexGuard<'_, PtySessionTable> {
    table.lock().unwrap_or_else(|error| error.into_inner())
}

pub(in crate::adapters::tools::pty) fn lock_writer(
    writer: &Arc<Mutex<PtyWriter>>,
) -> MutexGuard<'_, PtyWriter> {
    writer.lock().unwrap_or_else(|error| error.into_inner())
}

pub(in crate::adapters::tools::pty) fn lock_master(
    master: &Arc<Mutex<PtyMaster>>,
) -> MutexGuard<'_, PtyMaster> {
    master.lock().unwrap_or_else(|error| error.into_inner())
}

pub(in crate::adapters::tools::pty) fn lock_killer(
    killer: &Arc<Mutex<PtyKiller>>,
) -> MutexGuard<'_, PtyKiller> {
    killer.lock().unwrap_or_else(|error| error.into_inner())
}
