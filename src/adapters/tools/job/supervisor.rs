//! One Job process supervisor: completion, stop/timeout and child reaping.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::process::{Child, ChildStderr, ChildStdout};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::domain::tools::{JobCompletion, JobId, JobStatus, ToolExecutionPlan};

use super::super::active::ActiveProcessLeaseGuard;
use super::super::output::{CapturedOutput, lock_captured_output};
use super::{JobTable, lock_table};

const OUTPUT_DRAIN_GRACE: Duration = Duration::from_millis(100);

pub(super) struct JobSupervisor {
    pub(super) child: Child,
    pub(super) job_id: JobId,
    pub(super) plan: ToolExecutionPlan,
    pub(super) stdout: ChildStdout,
    pub(super) stderr: ChildStderr,
    pub(super) stdout_buffer: Arc<Mutex<CapturedOutput>>,
    pub(super) stderr_buffer: Arc<Mutex<CapturedOutput>>,
    pub(super) timeout_ms: u64,
    pub(super) stop_receiver: oneshot::Receiver<()>,
    pub(super) jobs: Arc<Mutex<JobTable>>,
    pub(super) active_lease: ActiveProcessLeaseGuard,
}

impl JobSupervisor {
    pub(super) async fn run(self) {
        let Self {
            mut child,
            job_id,
            plan,
            stdout,
            stderr,
            stdout_buffer,
            stderr_buffer,
            timeout_ms,
            mut stop_receiver,
            jobs,
            active_lease,
        } = self;
        let mut stdout_task = tokio::spawn(drain_output(stdout, stdout_buffer.clone()));
        let mut stderr_task = tokio::spawn(drain_output(stderr, stderr_buffer.clone()));
        let terminal = tokio::select! {
            status = child.wait() => terminal_from_wait(status),
            request = &mut stop_receiver => terminal_after_stop(&mut child, request.is_ok()).await,
            () = tokio::time::sleep(Duration::from_millis(timeout_ms)) => terminal_after_timeout(&mut child).await,
        };
        finish_reader(
            &mut stdout_task,
            &stdout_buffer,
            "stdout_reader_grace_timeout",
        )
        .await;
        finish_reader(
            &mut stderr_task,
            &stderr_buffer,
            "stderr_reader_grace_timeout",
        )
        .await;
        let completion = JobCompletion {
            job_id: job_id.clone(),
            status: terminal.status,
            exit_code: terminal.exit_code,
            start_execution_id: plan.execution_id,
            start_plan_fingerprint: plan.plan_fingerprint,
            exit_reason: terminal.exit_reason,
        };
        let mut table = lock_table(&jobs);
        if let Some(job) = table.entries.get_mut(&job_id) {
            job.publish_completion(completion);
        }
        drop(table);
        // Keep the RAII lease alive through the stable terminal publication.
        drop(active_lease);
    }
}

struct TerminalState {
    status: JobStatus,
    exit_code: Option<i32>,
    exit_reason: String,
}

fn terminal_from_wait(result: std::io::Result<std::process::ExitStatus>) -> TerminalState {
    match result {
        Ok(status) if status.success() => TerminalState {
            status: JobStatus::Completed,
            exit_code: status.code(),
            exit_reason: "process_completed".to_owned(),
        },
        Ok(status) => TerminalState {
            status: JobStatus::Failed,
            exit_code: status.code(),
            exit_reason: "process_exited_nonzero".to_owned(),
        },
        Err(error) => TerminalState {
            status: JobStatus::Failed,
            exit_code: None,
            exit_reason: format!("process_wait_failed:{error}"),
        },
    }
}

async fn terminal_after_stop(child: &mut Child, receiver_open: bool) -> TerminalState {
    let reason = if receiver_open {
        "job_stopped"
    } else {
        "job_stop_receiver_closed"
    };
    terminal_after_termination(child, JobStatus::Cancelled, reason).await
}

async fn terminal_after_timeout(child: &mut Child) -> TerminalState {
    terminal_after_termination(child, JobStatus::TimedOut, "job_timeout").await
}

async fn terminal_after_termination(
    child: &mut Child,
    status: JobStatus,
    exit_reason: &'static str,
) -> TerminalState {
    let kill_error = child.start_kill().err().map(|error| error.to_string());
    let (exit_code, wait_error) = match child.wait().await {
        Ok(status) => (status.code(), None),
        Err(error) => (None, Some(error.to_string())),
    };
    TerminalState {
        status,
        exit_code,
        exit_reason: match (kill_error, wait_error) {
            (None, None) => exit_reason.to_owned(),
            (Some(kill), None) => format!("{exit_reason}:{kill}"),
            (None, Some(wait)) => format!("{exit_reason}:{wait}"),
            (Some(kill), Some(wait)) => format!("{exit_reason}:{kill}; {wait}"),
        },
    }
}

async fn finish_reader(
    task: &mut JoinHandle<()>,
    output: &Arc<Mutex<CapturedOutput>>,
    timeout_reason: &'static str,
) {
    match tokio::time::timeout(OUTPUT_DRAIN_GRACE, &mut *task).await {
        Ok(Ok(())) => lock_captured_output(output).seal(),
        Ok(Err(error)) => {
            lock_captured_output(output).mark_incomplete(format!("reader_task_failed:{error}"))
        }
        Err(_) => {
            task.abort();
            let _ = task.await;
            lock_captured_output(output).mark_incomplete(timeout_reason);
        }
    }
}

async fn drain_output<Reader>(mut reader: Reader, output: Arc<Mutex<CapturedOutput>>)
where
    Reader: tokio::io::AsyncRead + Unpin,
{
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        match reader.read(&mut buffer).await {
            Ok(0) => return,
            Ok(read) => lock_captured_output(&output).append(&buffer[..read]),
            Err(error) => {
                lock_captured_output(&output).set_read_error(error.to_string());
                return;
            }
        }
    }
}
