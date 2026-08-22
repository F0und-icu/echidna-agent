//! argv-only background Job launch and its normalized start contract.

use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::process::Command;
use tokio::sync::{Mutex as AsyncMutex, oneshot, watch};

use crate::domain::tools::{JobStatus, ResourceKey, ToolExecutionOutcome, ToolExecutionPlan};
use crate::ports::tool::ToolExecutorError;

use super::super::active::{ActiveProcessId, ActiveProcessLeases};
use super::super::command::{
    CommandResource, RunCommandArgs, command_resource_keys, validate_command,
};
use super::super::filesystem::{display_path, resolve_path};
use super::super::output::CapturedOutput;
use super::super::{decode_arguments, invalid_arguments, invalid_plan};
use super::{BackgroundJobs, JobEntry, JobSupervisor};

const DEFAULT_JOB_TIMEOUT_MS: u64 = 300_000;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StartJobArgs {
    argv: Vec<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    resources: Vec<CommandResource>,
    #[serde(default = "default_job_timeout_ms")]
    timeout_ms: u64,
}

impl StartJobArgs {
    fn command(&self) -> RunCommandArgs {
        RunCommandArgs {
            argv: self.argv.clone(),
            cwd: self.cwd.clone(),
            resources: self.resources.clone(),
        }
    }
}

pub(in crate::adapters::tools) fn normalize_start(
    project_root: &Path,
    value: Value,
) -> Result<(Value, Vec<ResourceKey>), ToolExecutorError> {
    let arguments: StartJobArgs = decode_arguments("start_job", value)?;
    validate_start(&arguments)?;
    let command = arguments.command();
    Ok((
        serde_json::to_value(arguments).expect("StartJobArgs serialization cannot fail"),
        command_resource_keys(project_root, &command),
    ))
}

pub(in crate::adapters::tools) async fn execute_start(
    project_root: &Path,
    jobs: &BackgroundJobs,
    active_leases: &ActiveProcessLeases,
    plan: ToolExecutionPlan,
) -> ToolExecutionOutcome {
    let arguments: StartJobArgs = match decode_arguments("start_job", plan.call.arguments.clone()) {
        Ok(arguments) => arguments,
        Err(error) => return invalid_plan(plan, error),
    };
    if let Err(error) = validate_start(&arguments) {
        return invalid_plan(plan, error);
    }
    let command_arguments = arguments.command();
    let cwd = resolve_path(
        project_root,
        command_arguments.cwd.as_deref().unwrap_or("."),
    );
    let job_id = jobs.next_job_id();
    let process_id = ActiveProcessId::Job(job_id.clone());
    let mut lease_keys = plan.resolved_resource_keys.clone();
    lease_keys.push(ResourceKey::job(job_id.as_str()));
    let active_lease = match active_leases.acquire(process_id, lease_keys) {
        Ok(active_lease) => active_lease,
        Err(conflict) => return resource_busy(plan, conflict),
    };
    let mut command = Command::new(&command_arguments.argv[0]);
    command
        .args(&command_arguments.argv[1..])
        .current_dir(&cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return ToolExecutionOutcome::failed(
                plan,
                "job_start_failed",
                json!({"error": error.to_string(), "cwd": cwd.display().to_string()}),
            );
        }
    };
    let (Some(stdin), Some(stdout), Some(stderr)) =
        (child.stdin.take(), child.stdout.take(), child.stderr.take())
    else {
        let _ = child.start_kill();
        let _ = child.wait().await;
        return ToolExecutionOutcome::failed(
            plan,
            "job_stdio_unavailable",
            json!({"error": "background Job requires piped stdin, stdout and stderr"}),
        );
    };

    let stdout_buffer = Arc::new(Mutex::new(CapturedOutput::new(
        plan.output_policy.max_bytes / 2,
    )));
    let stderr_buffer = Arc::new(Mutex::new(CapturedOutput::new(
        plan.output_policy.max_bytes / 2,
    )));
    let stdin = Arc::new(AsyncMutex::new(stdin));
    let (stop, stop_receiver) = oneshot::channel();
    let (terminal, _) = watch::channel(());
    let entry = JobEntry {
        status: JobStatus::Running,
        stdout: stdout_buffer.clone(),
        stderr: stderr_buffer.clone(),
        stdin: Some(stdin),
        stop: Some(stop),
        terminal,
        completion: None,
    };
    jobs.insert(job_id.clone(), entry);

    tokio::spawn(
        JobSupervisor {
            child,
            job_id: job_id.clone(),
            plan: plan.clone(),
            stdout,
            stderr,
            stdout_buffer,
            stderr_buffer,
            timeout_ms: arguments.timeout_ms,
            stop_receiver,
            jobs: jobs.jobs.clone(),
            active_lease,
        }
        .run(),
    );

    ToolExecutionOutcome::succeeded(
        plan,
        json!({
            "job_id": job_id,
            "status": JobStatus::Running,
            "cwd": display_path(project_root, &cwd),
            "timeout_ms": arguments.timeout_ms,
        }),
    )
}

fn resource_busy(plan: ToolExecutionPlan, conflict: ActiveProcessId) -> ToolExecutionOutcome {
    match conflict {
        ActiveProcessId::Job(job_id) => ToolExecutionOutcome::failed(
            plan,
            "job_resource_busy",
            json!({
                "error": "Job resources conflict with an active background Job",
                "conflicting_job_id": job_id,
            }),
        ),
        ActiveProcessId::Pty(pty_session_id) => ToolExecutionOutcome::failed(
            plan,
            "pty_resource_busy",
            json!({
                "error": "Job resources conflict with an active PTY session",
                "conflicting_pty_session_id": pty_session_id,
            }),
        ),
    }
}

const fn default_job_timeout_ms() -> u64 {
    DEFAULT_JOB_TIMEOUT_MS
}

fn validate_start(arguments: &StartJobArgs) -> Result<(), ToolExecutorError> {
    if arguments.timeout_ms == 0 {
        return Err(invalid_arguments(
            "start_job",
            "`timeout_ms` must be greater than zero",
        ));
    }
    validate_command(&arguments.command(), "start_job")
}
