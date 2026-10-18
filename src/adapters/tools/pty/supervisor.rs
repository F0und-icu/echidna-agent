//! Dedicated blocking PTY child supervisor and terminal publication.

use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

use portable_pty::Child;

use crate::domain::tools::{
    PtySessionCompletion, PtySessionId, PtySessionStatus, ToolExecutionPlan,
};

use super::super::active::ActiveProcessLeaseGuard;
use super::super::output::{CapturedOutput, OUTPUT_DRAIN_GRACE, lock_captured_output};
use super::{PtySessionTable, lock_table};

pub(super) struct PtySupervisor {
    pub(super) child: Box<dyn Child + Send + Sync>,
    pub(super) session_id: PtySessionId,
    pub(super) plan: ToolExecutionPlan,
    pub(super) output: Arc<Mutex<CapturedOutput>>,
    pub(super) reader_done: Receiver<()>,
    pub(super) sessions: Arc<Mutex<PtySessionTable>>,
    pub(super) active_lease: ActiveProcessLeaseGuard,
}

impl PtySupervisor {
    pub(super) fn run(self) {
        let Self {
            mut child,
            session_id,
            plan,
            output,
            reader_done,
            sessions,
            active_lease,
        } = self;
        // `portable-pty::Child::wait` blocks. This supervisor is run on its
        // own standard thread, never on a Tokio runtime worker.
        let terminal = terminal_from_wait(child.wait());
        {
            let mut table = lock_table(&sessions);
            if let Some(session) = table.entries.get_mut(&session_id) {
                // Drop writer/master after the direct child is reaped so the
                // reader gets EOF or its current-platform terminal EIO.
                session.close_io();
            }
        }
        match reader_done.recv_timeout(OUTPUT_DRAIN_GRACE) {
            Ok(()) => lock_captured_output(&output).seal(),
            Err(_) => lock_captured_output(&output).mark_incomplete("pty_reader_grace_timeout"),
        }
        let completion = PtySessionCompletion {
            pty_session_id: session_id.clone(),
            status: terminal.status,
            exit_code: terminal.exit_code,
            start_execution_id: plan.execution_id,
            start_plan_fingerprint: plan.plan_fingerprint,
            exit_reason: terminal.exit_reason,
        };
        let mut table = lock_table(&sessions);
        if let Some(session) = table.entries.get_mut(&session_id) {
            session.publish_completion(completion);
        }
        drop(table);
        // The direct child has been reaped and the terminal state published;
        // only now can an overlapping Job/PTy resource be launched.
        drop(active_lease);
    }
}

struct TerminalState {
    status: PtySessionStatus,
    exit_code: Option<i32>,
    exit_reason: String,
}

fn terminal_from_wait(result: std::io::Result<portable_pty::ExitStatus>) -> TerminalState {
    match result {
        Ok(status) if status.success() => TerminalState {
            status: PtySessionStatus::Completed,
            exit_code: i32::try_from(status.exit_code()).ok(),
            exit_reason: "pty_process_completed".to_owned(),
        },
        Ok(status) if status.signal().is_some() => TerminalState {
            status: PtySessionStatus::Cancelled,
            exit_code: i32::try_from(status.exit_code()).ok(),
            exit_reason: format!(
                "pty_process_signaled:{}",
                status.signal().unwrap_or("unknown")
            ),
        },
        Ok(status) => TerminalState {
            status: PtySessionStatus::Failed,
            exit_code: i32::try_from(status.exit_code()).ok(),
            exit_reason: "pty_process_exited_nonzero".to_owned(),
        },
        Err(error) => TerminalState {
            status: PtySessionStatus::Failed,
            exit_code: None,
            exit_reason: format!("pty_process_wait_failed:{error}"),
        },
    }
}
