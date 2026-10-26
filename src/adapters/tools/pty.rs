//! Current-platform interactive PTY sessions backed by `portable-pty`.

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
use std::io::Write;
use std::sync::{Arc, Mutex, MutexGuard};

use portable_pty::{ChildKiller, MasterPty};
use tokio::sync::watch;

use crate::domain::tools::{PtySessionCompletion, PtySessionId, PtySessionStatus};

use control::{
    execute_input, execute_poll, execute_resize, execute_stop, normalize_input, normalize_poll,
    normalize_resize, normalize_stop,
};
use launch::{execute_start, normalize_start};
pub(in crate::adapters::tools::pty) use supervisor::PtySupervisor;

use super::output::CapturedOutput;
use super::process_identity::ProcessHandleSequence;

type PtyWriter = Box<dyn Write + Send>;
type PtyMaster = Box<dyn MasterPty + Send>;
type PtyKiller = Box<dyn ChildKiller + Send + Sync>;

/// The live PTY table is deliberately process-local. Session persistence and
/// restart recovery require the later Event/Project Slice rather than an
/// unsafe claim that an OS terminal survives a new Cyber Agent process.
pub(super) struct PtySessions {
    pub(in crate::adapters::tools::pty) sessions: Arc<Mutex<PtySessionTable>>,
    handles: ProcessHandleSequence,
}

impl PtySessions {
    pub(super) fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(PtySessionTable::default())),
            handles: ProcessHandleSequence::new(),
        }
    }

    pub(in crate::adapters::tools::pty) fn next_session_id(&self) -> PtySessionId {
        PtySessionId::new(self.handles.next("pty"))
    }

    pub(in crate::adapters::tools::pty) fn insert(
        &self,
        session_id: PtySessionId,
        entry: PtySessionEntry,
    ) {
        lock_table(&self.sessions).entries.insert(session_id, entry);
    }

    pub(super) fn has_active_processes(&self) -> bool {
        lock_table(&self.sessions).entries.values().any(|entry| {
            matches!(
                entry.status,
                PtySessionStatus::Running | PtySessionStatus::Stopping
            )
        })
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

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum Tool {
    Start,
    Poll,
    Input,
    Resize,
    Stop,
}

impl Tool {
    pub(super) const ALL: [Self; 5] = [
        Self::Start,
        Self::Poll,
        Self::Input,
        Self::Resize,
        Self::Stop,
    ];

    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::Start => "start_pty",
            Self::Poll => "poll_pty",
            Self::Input => "input_pty",
            Self::Resize => "resize_pty",
            Self::Stop => "stop_pty",
        }
    }

    pub(super) fn spec(self) -> ToolSpec {
        match self {
            Self::Start => super::spec(
                self.name(),
                "2",
                super::description(
                    "a terminal-required child",
                    "non-terminal work is sufficient.",
                    "run_command or start_job for non-terminal work. Declare actual resources; omitted/empty resources retain a global execution lease. Retain the PTY ID; tool_search can discover poll_pty/stop_pty while it runs. Stopping a Turn is not stopping a PTY.",
                    "PTY ID and status.",
                    false,
                ),
                ToolExposure::Deferred,
                json!({
                    "type": "object",
                    "properties": {
                        "argv": {"type": "array", "items": {"type": "string"}, "minItems": 1},
                        "cwd": {"type": "string"},
                        "resources": {"type": "array", "items": command::resource_schema()},
                        "rows": {"type": "integer", "minimum": 1, "maximum": 65535},
                        "cols": {"type": "integer", "minimum": 1, "maximum": 65535}
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
                    "new PTY output or completion",
                    "there is no PTY ID or offset is stale.",
                    "increasing offsets.",
                    "status, content and truncation.",
                    false,
                ),
                ToolExposure::Deferred,
                json!({
                    "type": "object",
                    "properties": {
                        "pty_session_id": {"type": "string", "minLength": 1},
                        "offset": {"type": "integer", "minimum": 0}
                    },
                    "required": ["pty_session_id"],
                    "additionalProperties": false
                }),
                self.output_schema(),
                ToolParallelism::ResourceLocked,
                Some("pty:{pty_session_id}"),
                ToolEffect::ReadOnly,
                ToolRetryPolicy::ReplaySafe,
                ToolDeterminism::EnvironmentDependent,
                10_000,
            ),
            Self::Input => super::spec(
                self.name(),
                "1",
                super::description(
                    "input for a running PTY",
                    "the process is closed or not a PTY.",
                    "one bounded input.",
                    "PTY ID and bytes.",
                    false,
                ),
                ToolExposure::Deferred,
                json!({
                    "type": "object",
                    "properties": {
                        "pty_session_id": {"type": "string", "minLength": 1},
                        "input": {"type": "string"}
                    },
                    "required": ["pty_session_id", "input"],
                    "additionalProperties": false
                }),
                self.output_schema(),
                ToolParallelism::ResourceLocked,
                Some("pty:{pty_session_id}"),
                ToolEffect::Interactive,
                ToolRetryPolicy::NeverAutoRetry,
                ToolDeterminism::Nondeterministic,
                10_000,
            ),
            Self::Resize => super::spec(
                self.name(),
                "1",
                super::description(
                    "resize a running PTY",
                    "the PTY is closed or input is needed.",
                    "positive dimensions.",
                    "PTY ID and dimensions.",
                    false,
                ),
                ToolExposure::Deferred,
                json!({
                    "type": "object",
                    "properties": {
                        "pty_session_id": {"type": "string", "minLength": 1},
                        "rows": {"type": "integer", "minimum": 1, "maximum": 65535},
                        "cols": {"type": "integer", "minimum": 1, "maximum": 65535}
                    },
                    "required": ["pty_session_id", "rows", "cols"],
                    "additionalProperties": false
                }),
                self.output_schema(),
                ToolParallelism::ResourceLocked,
                Some("pty:{pty_session_id}"),
                ToolEffect::Interactive,
                ToolRetryPolicy::NeverAutoRetry,
                ToolDeterminism::Nondeterministic,
                10_000,
            ),
            Self::Stop => super::spec(
                self.name(),
                "2",
                super::description(
                    "stop a running PTY",
                    "polling, resizing or input is needed.",
                    "natural completion.",
                    "terminal status.",
                    false,
                ),
                ToolExposure::Deferred,
                json!({
                    "type": "object",
                    "properties": {
                        "pty_session_id": {"type": "string", "minLength": 1}
                    },
                    "required": ["pty_session_id"],
                    "additionalProperties": false
                }),
                self.output_schema(),
                ToolParallelism::ResourceLocked,
                Some("pty:{pty_session_id}"),
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
            Self::Resize => normalize_resize(arguments),
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
                    &registry.ptys,
                    &registry.active_leases,
                    plan,
                )
                .await
            }
            Self::Poll => execute_poll(&registry.ptys, plan).await,
            Self::Input => execute_input(&registry.ptys, plan).await,
            Self::Resize => execute_resize(&registry.ptys, plan).await,
            Self::Stop => execute_stop(&registry.ptys, plan).await,
        }
    }

    fn output_schema(self) -> Value {
        let mut schema = match self {
            Self::Start => json!({
                "type": "object", "properties": {"pty_session_id": json!({"type": "string", "minLength": 1}), "status": json!({"type": "string", "enum": ["running"]})},
                "required": ["pty_session_id", "status"], "additionalProperties": false
            }),
            Self::Poll => json!({
                "type": "object", "properties": {"pty_session_id": json!({"type": "string", "minLength": 1}), "status": json!({"type": "string", "enum": ["running", "stopping", "completed", "failed", "cancelled"]}), "output": super::output::stream_schema(), "completion": {"oneOf": [pty_completion_schema(), {"type": "null"}]}},
                "required": ["pty_session_id", "status", "output", "completion"], "additionalProperties": false
            }),
            Self::Input => json!({
                "type": "object", "properties": {"pty_session_id": json!({"type": "string", "minLength": 1}), "bytes_written": json!({"type": "integer", "minimum": 0})},
                "required": ["pty_session_id", "bytes_written"], "additionalProperties": false
            }),
            Self::Resize => json!({
                "type": "object", "properties": {"pty_session_id": json!({"type": "string", "minLength": 1}), "rows": {"type": "integer", "minimum": 1, "maximum": 65535}, "cols": {"type": "integer", "minimum": 1, "maximum": 65535}},
                "required": ["pty_session_id", "rows", "cols"], "additionalProperties": false
            }),
            Self::Stop => json!({
                "type": "object", "properties": {"pty_session_id": json!({"type": "string", "minLength": 1}), "status": json!({"type": "string", "enum": ["completed", "failed", "cancelled"]}), "completion": pty_completion_schema(), "already_terminal": json!({"type": "boolean"})},
                "required": ["pty_session_id", "status", "completion", "already_terminal"], "additionalProperties": false
            }),
        };
        if matches!(self, Self::Poll | Self::Stop) {
            schema["properties"]["output_captures"] = crate::domain::tools::capture_schema();
        }
        schema
    }
}

fn pty_completion_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "pty_session_id": {"type": "string", "minLength": 1},
            "status": {"type": "string", "enum": ["completed", "failed", "cancelled"]},
            "exit_code": {"type": ["integer", "null"]},
            "start_execution_id": {"type": "string", "minLength": 1},
            "start_plan_fingerprint": {"type": "string", "minLength": 1},
            "exit_reason": {"type": "string", "minLength": 1}
        },
        "required": ["pty_session_id", "status", "exit_code", "start_execution_id", "start_plan_fingerprint", "exit_reason"],
        "additionalProperties": false
    })
}
