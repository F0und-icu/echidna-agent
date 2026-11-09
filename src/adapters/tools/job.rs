//! In-process ordinary background process Jobs.
//!
//! This adapter deliberately owns only live process state. Job persistence,
//! Event writing, Artifacts and PTY sessions are separate later slices.

use super::{ToolRegistry, command};
use crate::domain::tools::{
    ResourceKey, ToolDeterminism, ToolEffect, ToolExecutionOutcome, ToolExecutionPlan,
    ToolExposure, ToolParallelism, ToolRetryPolicy, ToolSpec,
};
use crate::ports::tool::ToolExecutorError;
use serde_json::{Value, json};

mod control;
mod launch;
mod supervisor;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use tokio::process::ChildStdin;
use tokio::sync::{Mutex as AsyncMutex, oneshot, watch};

use crate::domain::tools::{JobCompletion, JobId, JobStatus};

use super::output::CapturedOutput;
use super::process_identity::ProcessHandleSequence;

use control::{
    execute_input, execute_poll, execute_stop, normalize_input, normalize_poll, normalize_stop,
};
use launch::{execute_start, normalize_start};
pub(in crate::adapters::tools::job) use supervisor::JobSupervisor;

/// The live Job table is intentionally per ToolRegistry/Agent process. A
/// later Event/Project slice owns recovery semantics rather than silently
/// pretending an OS process can survive a restart.
pub(super) struct BackgroundJobs {
    pub(in crate::adapters::tools::job) jobs: Arc<Mutex<JobTable>>,
    handles: ProcessHandleSequence,
}

impl BackgroundJobs {
    pub(super) fn new() -> Self {
        Self {
            jobs: Arc::new(Mutex::new(JobTable::default())),
            handles: ProcessHandleSequence::new(),
        }
    }

    pub(in crate::adapters::tools::job) fn next_job_id(&self) -> JobId {
        JobId::new(self.handles.next("job"))
    }

    pub(super) fn has_active_processes(&self) -> bool {
        lock_table(&self.jobs)
            .entries
            .values()
            .any(|entry| matches!(entry.status, JobStatus::Running | JobStatus::Stopping))
    }

    pub(super) fn live_processes(&self) -> Vec<(JobId, JobStatus)> {
        lock_table(&self.jobs)
            .entries
            .iter()
            .filter(|(_, entry)| matches!(entry.status, JobStatus::Running | JobStatus::Stopping))
            .map(|(id, entry)| (id.clone(), entry.status))
            .collect()
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

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum Tool {
    Start,
    Poll,
    Input,
    Stop,
}

impl Tool {
    pub(super) const ALL: [Self; 4] = [Self::Start, Self::Poll, Self::Input, Self::Stop];

    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::Start => "start_job",
            Self::Poll => "poll_job",
            Self::Input => "input_job",
            Self::Stop => "stop_job",
        }
    }

    pub(super) fn spec(self) -> ToolSpec {
        match self {
            Self::Start => super::spec(
                self.name(),
                "2",
                super::description(
                    "long non-interactive work",
                    "the command is short or terminal-interactive.",
                    "long non-PTY work with an explicit lifetime (timeout_ms defaults to 300000). Declare actual resources: omitted/empty resources hold a global execution lease until termination. Retain the Job ID; discover poll_job/stop_job through tool_search for status or early stop. Discovery remains available while the Job runs; stopping a Turn is not stopping a Job.",
                    "Job ID and status.",
                    false,
                ),
                ToolExposure::Deferred,
                json!({
                    "type": "object",
                    "properties": {
                        "argv": {"type": "array", "items": {"type": "string"}, "minItems": 1},
                        "cwd": {"type": "string"},
                        "resources": {"type": "array", "items": command::resource_schema()},
                        "timeout_ms": {"type": "integer", "minimum": 1}
                    },
                    "required": ["argv"],
                    "additionalProperties": false
                }),
                self.output_schema(),
                ToolParallelism::ResourceLocked,
                Some("resource:{resources}"),
                ToolEffect::Interactive,
                ToolRetryPolicy::DedupeByKey,
                ToolDeterminism::Nondeterministic,
                10_000,
            ),
            Self::Poll => super::spec(
                self.name(),
                "2",
                super::description(
                    "new Job output or completion",
                    "there is no Job ID or offset is stale.",
                    "increasing offsets.",
                    "status, content and truncation.",
                    false,
                ),
                ToolExposure::Deferred,
                json!({
                    "type": "object",
                    "properties": {
                        "job_id": {"type": "string", "minLength": 1},
                        "stdout_offset": {"type": "integer", "minimum": 0},
                        "stderr_offset": {"type": "integer", "minimum": 0}
                    },
                    "required": ["job_id"],
                    "additionalProperties": false
                }),
                self.output_schema(),
                ToolParallelism::ResourceLocked,
                Some("job:{job_id}"),
                ToolEffect::ReadOnly,
                ToolRetryPolicy::ReplaySafe,
                ToolDeterminism::EnvironmentDependent,
                10_000,
            ),
            Self::Input => super::spec(
                self.name(),
                "1",
                super::description(
                    "stdin for a running Job",
                    "the process is a PTY or stdin is closed.",
                    "one bounded write.",
                    "Job ID and bytes.",
                    false,
                ),
                ToolExposure::Deferred,
                json!({
                    "type": "object",
                    "properties": {
                        "job_id": {"type": "string", "minLength": 1},
                        "input": {"type": "string"},
                        "close": {"type": "boolean"}
                    },
                    "required": ["job_id", "input"],
                    "additionalProperties": false
                }),
                self.output_schema(),
                ToolParallelism::ResourceLocked,
                Some("job:{job_id}"),
                ToolEffect::Interactive,
                ToolRetryPolicy::NeverAutoRetry,
                ToolDeterminism::Nondeterministic,
                10_000,
            ),
            Self::Stop => super::spec(
                self.name(),
                "2",
                super::description(
                    "stop a running Job",
                    "start or polling is needed.",
                    "natural completion.",
                    "terminal status.",
                    false,
                ),
                ToolExposure::Deferred,
                json!({
                    "type": "object",
                    "properties": {
                        "job_id": {"type": "string", "minLength": 1}
                    },
                    "required": ["job_id"],
                    "additionalProperties": false
                }),
                self.output_schema(),
                ToolParallelism::ResourceLocked,
                Some("job:{job_id}"),
                ToolEffect::Interactive,
                ToolRetryPolicy::NeverAutoRetry,
                ToolDeterminism::Nondeterministic,
                10_000,
            ),
        }
    }

    pub(super) fn normalize(
        self,
        registry: &ToolRegistry,
        arguments: Value,
    ) -> Result<(Value, Vec<ResourceKey>), ToolExecutorError> {
        let project_root = &registry.project_root;
        match self {
            Self::Start => normalize_start(project_root, arguments),
            Self::Poll => normalize_poll(arguments),
            Self::Input => normalize_input(arguments),
            Self::Stop => normalize_stop(arguments),
        }
    }

    pub(super) async fn execute(
        self,
        registry: &ToolRegistry,
        plan: ToolExecutionPlan,
    ) -> ToolExecutionOutcome {
        match self {
            Self::Start => {
                execute_start(
                    &registry.project_root,
                    &registry.jobs,
                    &registry.active_leases,
                    plan,
                )
                .await
            }
            Self::Poll => execute_poll(&registry.jobs, plan).await,
            Self::Input => execute_input(&registry.jobs, plan).await,
            Self::Stop => execute_stop(&registry.jobs, plan).await,
        }
    }

    fn output_schema(self) -> Value {
        let mut schema = match self {
            Self::Start => json!({
                "type": "object", "properties": {"job_id": json!({"type": "string", "minLength": 1}), "status": json!({"type": "string", "enum": ["running"]}), "pid": {"type": "integer", "minimum": 1}},
                "required": ["job_id", "status", "pid"], "additionalProperties": false
            }),
            Self::Poll => json!({
                "type": "object", "properties": {"job_id": json!({"type": "string", "minLength": 1}), "status": json!({"type": "string", "enum": ["running", "stopping", "completed", "failed", "cancelled", "timed_out"]}), "stdout": super::output::stream_schema(), "stderr": super::output::stream_schema(), "completion": {"oneOf": [job_completion_schema(), {"type": "null"}]}},
                "required": ["job_id", "status", "stdout", "stderr", "completion"], "additionalProperties": false
            }),
            Self::Input => json!({
                "type": "object", "properties": {"job_id": json!({"type": "string", "minLength": 1}), "bytes_written": json!({"type": "integer", "minimum": 0}), "input_closed": json!({"type": "boolean"})},
                "required": ["job_id", "bytes_written", "input_closed"], "additionalProperties": false
            }),
            Self::Stop => json!({
                "type": "object", "properties": {"job_id": json!({"type": "string", "minLength": 1}), "status": json!({"type": "string", "enum": ["completed", "failed", "cancelled", "timed_out"]}), "completion": job_completion_schema(), "already_terminal": json!({"type": "boolean"})},
                "required": ["job_id", "status", "completion", "already_terminal"], "additionalProperties": false
            }),
        };
        if matches!(self, Self::Poll | Self::Stop) {
            schema["properties"]["output_captures"] = crate::domain::tools::capture_schema();
        }
        schema
    }
}

fn job_completion_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "job_id": {"type": "string", "minLength": 1},
            "status": {"type": "string", "enum": ["completed", "failed", "cancelled", "timed_out"]},
            "exit_code": {"type": ["integer", "null"]},
            "start_execution_id": {"type": "string", "minLength": 1},
            "start_plan_fingerprint": {"type": "string", "minLength": 1},
            "exit_reason": {"type": "string", "minLength": 1}
        },
        "required": ["job_id", "status", "exit_code", "start_execution_id", "start_plan_fingerprint", "exit_reason"],
        "additionalProperties": false
    })
}
