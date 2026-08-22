//! Ongoing ordinary Job polling, stdin and stop operations.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;
use tokio::sync::watch;

use crate::domain::tools::{
    JobId, JobStatus, ResourceKey, ToolExecutionOutcome, ToolExecutionPlan,
};
use crate::ports::tool::ToolExecutorError;

use super::super::{decode_arguments, invalid_plan};
use super::{BackgroundJobs, OffsetError, lock_output, lock_table};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct JobIdArgs {
    job_id: JobId,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PollJobArgs {
    job_id: JobId,
    #[serde(default)]
    stdout_offset: u64,
    #[serde(default)]
    stderr_offset: u64,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct InputJobArgs {
    job_id: JobId,
    input: String,
    #[serde(default)]
    close: bool,
}

pub(in crate::adapters::tools) fn normalize_poll(
    value: Value,
) -> Result<(Value, Vec<ResourceKey>), ToolExecutorError> {
    let arguments: PollJobArgs = decode_arguments("poll_job", value)?;
    validate_job_id("poll_job", &arguments.job_id)?;
    let job_id = arguments.job_id.clone();
    Ok((
        serde_json::to_value(arguments).expect("PollJobArgs serialization cannot fail"),
        vec![ResourceKey::job(job_id.as_str())],
    ))
}

pub(in crate::adapters::tools) fn normalize_input(
    value: Value,
) -> Result<(Value, Vec<ResourceKey>), ToolExecutorError> {
    let arguments: InputJobArgs = decode_arguments("input_job", value)?;
    validate_job_id("input_job", &arguments.job_id)?;
    let job_id = arguments.job_id.clone();
    Ok((
        serde_json::to_value(arguments).expect("InputJobArgs serialization cannot fail"),
        vec![ResourceKey::job(job_id.as_str())],
    ))
}

pub(in crate::adapters::tools) fn normalize_stop(
    value: Value,
) -> Result<(Value, Vec<ResourceKey>), ToolExecutorError> {
    let arguments: JobIdArgs = decode_arguments("stop_job", value)?;
    validate_job_id("stop_job", &arguments.job_id)?;
    let job_id = arguments.job_id.clone();
    Ok((
        serde_json::to_value(arguments).expect("JobIdArgs serialization cannot fail"),
        vec![ResourceKey::job(job_id.as_str())],
    ))
}

pub(in crate::adapters::tools) async fn execute_poll(
    jobs: &BackgroundJobs,
    plan: ToolExecutionPlan,
) -> ToolExecutionOutcome {
    let arguments: PollJobArgs = match decode_arguments("poll_job", plan.call.arguments.clone()) {
        Ok(arguments) => arguments,
        Err(error) => return invalid_plan(plan, error),
    };
    let snapshot = {
        let table = lock_table(&jobs.jobs);
        let Some(job) = table.entries.get(&arguments.job_id) else {
            return unknown_job(plan, &arguments.job_id);
        };
        (
            job.status,
            job.stdout.clone(),
            job.stderr.clone(),
            job.completion.clone(),
        )
    };
    let stdout = lock_output(&snapshot.1).slice_after_offset(arguments.stdout_offset);
    let stderr = lock_output(&snapshot.2).slice_after_offset(arguments.stderr_offset);
    let (stdout, stderr) = match (stdout, stderr) {
        (Ok(stdout), Ok(stderr)) => (stdout, stderr),
        (stdout, stderr) => {
            return ToolExecutionOutcome::failed(
                plan,
                "job_offset_out_of_range",
                json!({
                    "job_id": arguments.job_id,
                    "stdout": offset_error_value(stdout.err()),
                    "stderr": offset_error_value(stderr.err()),
                }),
            );
        }
    };
    ToolExecutionOutcome::succeeded(
        plan,
        json!({
            "job_id": arguments.job_id,
            "status": snapshot.0,
            "stdout": stdout,
            "stderr": stderr,
            "completion": snapshot.3,
        }),
    )
}

pub(in crate::adapters::tools) async fn execute_input(
    jobs: &BackgroundJobs,
    plan: ToolExecutionPlan,
) -> ToolExecutionOutcome {
    let arguments: InputJobArgs = match decode_arguments("input_job", plan.call.arguments.clone()) {
        Ok(arguments) => arguments,
        Err(error) => return invalid_plan(plan, error),
    };
    let stdin = {
        let mut table = lock_table(&jobs.jobs);
        let Some(job) = table.entries.get_mut(&arguments.job_id) else {
            return unknown_job(plan, &arguments.job_id);
        };
        if job.status != JobStatus::Running {
            return not_running(plan, &arguments.job_id, job.status);
        }
        if arguments.close {
            job.stdin.take()
        } else {
            job.stdin.clone()
        }
    };
    let Some(stdin) = stdin else {
        return ToolExecutionOutcome::failed(
            plan,
            "job_input_closed",
            json!({"job_id": arguments.job_id, "error": "Job stdin is already closed"}),
        );
    };
    let mut stdin = stdin.lock().await;
    if let Err(error) = stdin.write_all(arguments.input.as_bytes()).await {
        return ToolExecutionOutcome::failed(
            plan,
            "job_input_failed",
            json!({"job_id": arguments.job_id, "error": error.to_string()}),
        );
    }
    if let Err(error) = stdin.flush().await {
        return ToolExecutionOutcome::failed(
            plan,
            "job_input_failed",
            json!({"job_id": arguments.job_id, "error": error.to_string()}),
        );
    }
    drop(stdin);
    ToolExecutionOutcome::succeeded(
        plan,
        json!({
            "job_id": arguments.job_id,
            "bytes_written": arguments.input.len(),
            "input_closed": arguments.close,
        }),
    )
}

pub(in crate::adapters::tools) async fn execute_stop(
    jobs: &BackgroundJobs,
    plan: ToolExecutionPlan,
) -> ToolExecutionOutcome {
    let arguments: JobIdArgs = match decode_arguments("stop_job", plan.call.arguments.clone()) {
        Ok(arguments) => arguments,
        Err(error) => return invalid_plan(plan, error),
    };
    let (terminal, stop) = {
        let mut table = lock_table(&jobs.jobs);
        let Some(job) = table.entries.get_mut(&arguments.job_id) else {
            return unknown_job(plan, &arguments.job_id);
        };
        if let Some(completion) = job.completion.clone() {
            return terminal_outcome(plan, &arguments.job_id, job.status, completion, true);
        }
        let terminal = job.terminal_receiver();
        let stop = job.request_stop();
        (terminal, stop)
    };

    // A failed send means the supervisor no longer has the stop receiver. It
    // may already be finishing a natural completion, so terminal publication
    // remains the sole completion authority instead of reporting a false
    // stop-unavailable error.
    if let Some(stop) = stop {
        let _ = stop.send(());
    }
    let Some((status, completion)) = wait_for_terminal(jobs, &arguments.job_id, terminal).await
    else {
        return ToolExecutionOutcome::failed(
            plan,
            "job_terminal_unavailable",
            json!({
                "job_id": arguments.job_id,
                "error": "Job terminal state was not published",
            }),
        );
    };
    terminal_outcome(plan, &arguments.job_id, status, completion, false)
}

/// Wait for the supervisor's one terminal publication without polling or
/// spinning. The table check covers a completion that won the race before the
/// receiver observed the watch notification.
async fn wait_for_terminal(
    jobs: &BackgroundJobs,
    job_id: &JobId,
    mut terminal: watch::Receiver<()>,
) -> Option<(JobStatus, crate::domain::tools::JobCompletion)> {
    loop {
        {
            let table = lock_table(&jobs.jobs);
            let job = table.entries.get(job_id)?;
            if let Some(completion) = job.completion.clone() {
                return Some((job.status, completion));
            }
        }

        // The JobEntry retains the Sender for its entire in-process lifetime,
        // so a closed receiver is only a defensive indication that terminal
        // publication cannot happen. In the normal path this awaits the
        // supervisor rather than repeatedly checking the mutex.
        if terminal.changed().await.is_err() {
            return None;
        }
    }
}

fn terminal_outcome(
    plan: ToolExecutionPlan,
    job_id: &JobId,
    status: JobStatus,
    completion: crate::domain::tools::JobCompletion,
    already_terminal: bool,
) -> ToolExecutionOutcome {
    ToolExecutionOutcome::succeeded(
        plan,
        json!({
            "job_id": job_id,
            "status": status,
            "completion": completion,
            "already_terminal": already_terminal,
        }),
    )
}

fn unknown_job(plan: ToolExecutionPlan, job_id: &JobId) -> ToolExecutionOutcome {
    ToolExecutionOutcome::failed(
        plan,
        "job_not_found",
        json!({"job_id": job_id, "error": "Job does not exist in this process"}),
    )
}

fn not_running(plan: ToolExecutionPlan, job_id: &JobId, status: JobStatus) -> ToolExecutionOutcome {
    ToolExecutionOutcome::failed(
        plan,
        "job_not_running",
        json!({"job_id": job_id, "status": status, "error": "Job is not accepting input"}),
    )
}

fn validate_job_id(tool_name: &str, job_id: &JobId) -> Result<(), ToolExecutorError> {
    if job_id.as_str().is_empty() {
        return Err(super::super::invalid_arguments(
            tool_name,
            "`job_id` must not be empty",
        ));
    }
    Ok(())
}

fn offset_error_value(error: Option<OffsetError>) -> Value {
    match error {
        Some(error) => json!({
            "error": error.reason,
            "reason": error.reason,
            "requested_offset": error.requested,
            "available_offset": error.available,
        }),
        None => Value::Null,
    }
}
