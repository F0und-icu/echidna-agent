//! PTY launch, argv validation and current-platform terminal setup.

use std::io::Read;
use std::path::Path;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;

use portable_pty::{Child, CommandBuilder, PtySize, native_pty_system};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::watch;

use crate::domain::tools::{
    PtySessionStatus, ResourceKey, ToolExecutionOutcome, ToolExecutionPlan,
};
use crate::ports::tool::ToolExecutorError;

use super::super::active::{ActiveProcessId, ActiveProcessLeases};
use super::super::command::{
    CommandResource, RunCommandArgs, command_resource_keys, validate_command,
};
use super::super::filesystem::{display_path, resolve_path};
use super::super::output::{CapturedOutput, lock_captured_output};
use super::super::{decode_arguments, invalid_arguments, invalid_plan};
use super::{PtyKiller, PtyMaster, PtySessionEntry, PtySessions, PtySupervisor, PtyWriter};

const DEFAULT_ROWS: u16 = 24;
const DEFAULT_COLS: u16 = 80;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StartPtyArgs {
    argv: Vec<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    resources: Vec<CommandResource>,
    #[serde(default = "default_rows")]
    rows: u16,
    #[serde(default = "default_cols")]
    cols: u16,
}

impl StartPtyArgs {
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
    let arguments: StartPtyArgs = decode_arguments("start_pty", value)?;
    validate_start(&arguments)?;
    let command = arguments.command();
    Ok((
        serde_json::to_value(arguments).expect("StartPtyArgs serialization cannot fail"),
        command_resource_keys(project_root, &command),
    ))
}

pub(in crate::adapters::tools) async fn execute_start(
    project_root: &Path,
    sessions: &PtySessions,
    active_leases: &ActiveProcessLeases,
    plan: ToolExecutionPlan,
) -> ToolExecutionOutcome {
    let arguments: StartPtyArgs = match decode_arguments("start_pty", plan.call.arguments.clone()) {
        Ok(arguments) => arguments,
        Err(error) => return invalid_plan(plan, error),
    };
    if let Err(error) = validate_start(&arguments) {
        return invalid_plan(plan, error);
    }
    let cwd = resolve_path(project_root, arguments.cwd.as_deref().unwrap_or("."));
    let session_id = sessions.next_session_id();
    let process_id = ActiveProcessId::Pty(session_id.clone());
    let mut lease_keys = plan.resolved_resource_keys.clone();
    lease_keys.push(ResourceKey::pty(session_id.as_str()));
    let active_lease = match active_leases.acquire(process_id, lease_keys) {
        Ok(active_lease) => active_lease,
        Err(conflict) => return resource_busy(plan, conflict),
    };
    let spawn_arguments = arguments.clone();
    let spawn_cwd = cwd.clone();
    let spawned = match tokio::task::spawn_blocking(move || {
        spawn_pty(&spawn_arguments, &spawn_cwd).map(|spawned| (spawned, active_lease))
    })
    .await
    {
        Ok(Ok(spawned)) => spawned,
        Ok(Err(error)) => {
            return ToolExecutionOutcome::failed(
                plan,
                "pty_start_failed",
                json!({"error": error, "cwd": cwd.display().to_string()}),
            );
        }
        Err(error) => {
            return ToolExecutionOutcome::failed(
                plan,
                "pty_start_failed",
                json!({"error": error.to_string(), "cwd": cwd.display().to_string()}),
            );
        }
    };
    let (mut spawned, active_lease) = spawned;
    let output = Arc::new(Mutex::new(CapturedOutput::new(
        crate::domain::tools::MAX_CAPTURE_STREAM_BYTES,
    )));
    let (reader_done_sender, reader_done_receiver) = mpsc::channel();
    spawn_reader(
        spawned
            .reader
            .take()
            .expect("successful PTY setup owns one reader"),
        output.clone(),
        reader_done_sender,
    );
    let (terminal, _) = watch::channel(());
    let entry = PtySessionEntry {
        status: PtySessionStatus::Running,
        output: output.clone(),
        writer: Some(Arc::new(Mutex::new(
            spawned
                .writer
                .take()
                .expect("successful PTY setup owns one writer"),
        ))),
        master: Some(Arc::new(Mutex::new(
            spawned
                .master
                .take()
                .expect("successful PTY setup owns one master"),
        ))),
        killer: Some(Arc::new(Mutex::new(
            spawned
                .killer
                .take()
                .expect("successful PTY setup owns one killer"),
        ))),
        terminal,
        completion: None,
    };
    sessions.insert(session_id.clone(), entry);
    let supervisor = PtySupervisor {
        child: spawned
            .child
            .take()
            .expect("successful PTY setup owns one child"),
        session_id: session_id.clone(),
        plan: plan.clone(),
        output,
        reader_done: reader_done_receiver,
        sessions: sessions.sessions.clone(),
        active_lease,
    };
    thread::spawn(move || supervisor.run());
    ToolExecutionOutcome::succeeded(
        plan,
        json!({
            "pty_session_id": session_id,
            "status": PtySessionStatus::Running,
            "cwd": display_path(project_root, &cwd),
            "rows": arguments.rows,
            "cols": arguments.cols,
        }),
    )
}

struct SpawnedPty {
    child: Option<Box<dyn Child + Send + Sync>>,
    killer: Option<PtyKiller>,
    master: Option<PtyMaster>,
    writer: Option<PtyWriter>,
    reader: Option<Box<dyn Read + Send>>,
}

impl Drop for SpawnedPty {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn spawn_pty(arguments: &StartPtyArgs, cwd: &Path) -> Result<SpawnedPty, String> {
    let system = native_pty_system();
    let pair = system
        .openpty(PtySize {
            rows: arguments.rows,
            cols: arguments.cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|error| error.to_string())?;
    let mut command = CommandBuilder::new(&arguments.argv[0]);
    command.args(&arguments.argv[1..]);
    command.cwd(cwd);
    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|error| error.to_string())?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|error| error.to_string())?;
    let child = pair
        .slave
        .spawn_command(command)
        .map_err(|error| error.to_string())?;
    drop(pair.slave);
    let killer = child.clone_killer();
    Ok(SpawnedPty {
        child: Some(child),
        killer: Some(killer),
        master: Some(pair.master),
        writer: Some(writer),
        reader: Some(reader),
    })
}

fn spawn_reader(
    mut reader: Box<dyn Read + Send>,
    output: Arc<Mutex<CapturedOutput>>,
    done: mpsc::Sender<()>,
) {
    thread::spawn(move || {
        let mut buffer = [0_u8; 8 * 1024];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => lock_captured_output(&output).append(&buffer[..read]),
                Err(error) if is_terminal_pty_read_error(&error) => break,
                Err(error) => {
                    lock_captured_output(&output).set_read_error(error.to_string());
                    break;
                }
            }
        }
        let _ = done.send(());
    });
}

fn is_terminal_pty_read_error(error: &std::io::Error) -> bool {
    // The Unix PTY master reports EIO after the final slave closes. This is
    // terminal EOF, not an output failure, on the current macOS/Unix path.
    #[cfg(unix)]
    {
        error.raw_os_error() == Some(5)
    }
    #[cfg(not(unix))]
    {
        let _ = error;
        false
    }
}

fn resource_busy(plan: ToolExecutionPlan, conflict: ActiveProcessId) -> ToolExecutionOutcome {
    match conflict {
        ActiveProcessId::Job(job_id) => ToolExecutionOutcome::failed(
            plan,
            "job_resource_busy",
            json!({
                "error": "PTY resources conflict with an active background Job",
                "conflicting_job_id": job_id,
            }),
        ),
        ActiveProcessId::Pty(pty_session_id) => ToolExecutionOutcome::failed(
            plan,
            "pty_resource_busy",
            json!({
                "error": "PTY resources conflict with an active PTY session",
                "conflicting_pty_session_id": pty_session_id,
            }),
        ),
    }
}

const fn default_rows() -> u16 {
    DEFAULT_ROWS
}

const fn default_cols() -> u16 {
    DEFAULT_COLS
}

fn validate_start(arguments: &StartPtyArgs) -> Result<(), ToolExecutorError> {
    if arguments.rows == 0 || arguments.cols == 0 {
        return Err(invalid_arguments(
            "start_pty",
            "`rows` and `cols` must be greater than zero",
        ));
    }
    validate_command(&arguments.command(), "start_pty")
}
