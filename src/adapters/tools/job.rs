//! In-process ordinary background process Jobs.
//!
//! This adapter deliberately owns only live process state. Job persistence,
//! Event writing, Artifacts and PTY sessions are separate later slices.

mod control;
mod launch;
mod supervisor;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use tokio::process::ChildStdin;
use tokio::sync::{Mutex as AsyncMutex, oneshot, watch};

use crate::domain::tools::{JobCompletion, JobId, JobStatus};

use super::output::CapturedOutput;

pub(super) use control::{
    execute_input, execute_poll, execute_stop, normalize_input, normalize_poll, normalize_stop,
};
pub(super) use launch::{execute_start, normalize_start};
pub(in crate::adapters::tools::job) use supervisor::JobSupervisor;

/// The live Job table is intentionally per ToolRegistry/Agent process. A
/// later Event/Project slice owns recovery semantics rather than silently
/// pretending an OS process can survive a restart.
pub(super) struct BackgroundJobs {
    pub(in crate::adapters::tools::job) jobs: Arc<Mutex<JobTable>>,
    next_job_sequence: AtomicU64,
}

impl BackgroundJobs {
    pub(super) fn new() -> Self {
        Self {
            jobs: Arc::new(Mutex::new(JobTable::default())),
            next_job_sequence: AtomicU64::new(0),
        }
    }

    pub(in crate::adapters::tools::job) fn next_job_id(&self) -> JobId {
        let sequence = self.next_job_sequence.fetch_add(1, Ordering::Relaxed) + 1;
        JobId::new(format!("job:{sequence}"))
    }
}

#[derive(Default)]
pub(in crate::adapters::tools::job) struct JobTable {
    pub(in crate::adapters::tools::job) entries: BTreeMap<JobId, JobEntry>,
}

pub(in crate::adapters::tools::job) struct JobEntry {
    pub(in crate::adapters::tools::job) status: JobStatus,
    pub(in crate::adapters::tools::job) stdout: Arc<Mutex<CapturedOutput>>,
    pub(in crate::adapters::tools::job) stderr: Arc<Mutex<CapturedOutput>>,
    pub(in crate::adapters::tools::job) stdin: Option<Arc<AsyncMutex<ChildStdin>>>,
    pub(in crate::adapters::tools::job) stop: Option<oneshot::Sender<()>>,
    pub(in crate::adapters::tools::job) terminal: watch::Sender<()>,
    pub(in crate::adapters::tools::job) completion: Option<JobCompletion>,
}

impl JobEntry {
    pub(in crate::adapters::tools::job) fn request_stop(&mut self) -> Option<oneshot::Sender<()>> {
        if self.status != JobStatus::Running {
            return None;
        }
        self.status = JobStatus::Stopping;
        self.stop.take()
    }

    pub(in crate::adapters::tools::job) fn terminal_receiver(&self) -> watch::Receiver<()> {
        self.terminal.subscribe()
    }

    pub(in crate::adapters::tools::job) fn publish_completion(
        &mut self,
        completion: JobCompletion,
    ) {
        self.status = completion.status;
        self.stdin = None;
        self.stop = None;
        self.completion = Some(completion);
        self.terminal.send_replace(());
    }
}

impl BackgroundJobs {
    pub(in crate::adapters::tools::job) fn insert(&self, job_id: JobId, entry: JobEntry) {
        let mut table = lock_table(&self.jobs);
        table.entries.insert(job_id, entry);
    }
}

pub(in crate::adapters::tools::job) fn lock_table(
    table: &Arc<Mutex<JobTable>>,
) -> MutexGuard<'_, JobTable> {
    table.lock().unwrap_or_else(|error| error.into_inner())
}
