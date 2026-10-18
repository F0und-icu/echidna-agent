//! Argv-only single and batched command execution.

use std::collections::VecDeque;
use std::path::Path;
use std::process::Stdio;

use futures_util::future::join_all;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

use crate::domain::tools::{
    MAX_CAPTURE_STREAM_BYTES, OutputStream, PendingOutputCapture, ResourceKey, ToolEffect,
    ToolExecutionOutcome, ToolExecutionPlan, ToolParallelism, batch_capture_stream_limit,
    scheduling_conflicts,
};
use crate::domain::tools::{OutputCapture, SubcommandCheckpoint};
use crate::ports::tool::ToolExecutorError;
use crate::ports::tool::ToolObservationSender;

use super::filesystem::{display_path, resolve_path};
use super::output::{BoundedText, CapturedOutput, OUTPUT_DRAIN_GRACE, bounded_text};
use super::resources::ResourceKeyResolver;
use super::{decode_arguments, invalid_arguments, invalid_plan};

const MAX_COMMANDS: usize = 32;

pub(super) fn resource_schema() -> Value {
    json!({
        "oneOf": [
            {
                "type": "object",
                "properties": {
                    "kind": {"const": "device"},
                    "serial": {"type": "string", "minLength": 1}
                },
                "required": ["kind", "serial"],
                "additionalProperties": false
            },
            {
                "type": "object",
                "properties": {
                    "kind": {"const": "pty"},
                    "session": {"type": "string", "minLength": 1}
                },
                "required": ["kind", "session"],
                "additionalProperties": false
            },
            {
                "type": "object",
                "properties": {
                    "kind": {"const": "output"},
                    "path": {"type": "string", "minLength": 1}
                },
                "required": ["kind", "path"],
                "additionalProperties": false
            }
        ]
    })
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
pub(super) enum CommandResource {
    Device { serial: String },
    Pty { session: String },
    Output { path: String },
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RunCommandArgs {
    pub(super) argv: Vec<String>,
    #[serde(default)]
    pub(super) cwd: Option<String>,
    #[serde(default)]
    pub(super) resources: Vec<CommandResource>,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum CommandBatchMode {
    Serial,
    Parallel,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RunCommandsArgs {
    mode: CommandBatchMode,
    commands: Vec<RunCommandArgs>,
}

pub(super) fn normalize_one(
    project_root: &Path,
    value: Value,
) -> Result<(Value, Vec<ResourceKey>), ToolExecutorError> {
    let arguments: RunCommandArgs = decode_arguments("run_command", value)?;
    validate_command(&arguments, "run_command")?;
    let keys = command_resource_keys(project_root, &arguments);
    Ok((
        serde_json::to_value(arguments).expect("RunCommandArgs serialization cannot fail"),
        keys,
    ))
}

pub(super) fn normalize_batch(
    project_root: &Path,
    value: Value,
) -> Result<(Value, Vec<ResourceKey>), ToolExecutorError> {
    let arguments: RunCommandsArgs = decode_arguments("run_commands", value)?;
    let keys = command_resource_sets(&arguments, project_root)?
        .into_iter()
        .flatten()
        .collect();
    Ok((
        serde_json::to_value(arguments).expect("RunCommandsArgs serialization cannot fail"),
        keys,
    ))
}

pub(super) async fn execute_one(
    project_root: &Path,
    plan: ToolExecutionPlan,
) -> ToolExecutionOutcome {
    let arguments: RunCommandArgs =
        match decode_arguments("run_command", plan.call.arguments.clone()) {
            Ok(arguments) => arguments,
            Err(error) => return invalid_plan(plan, error),
        };
    if let Err(error) = validate_command(&arguments, "run_command") {
        return invalid_plan(plan, error);
    }
    let output = run(
        project_root,
        &arguments,
        plan.output_policy.max_bytes,
        MAX_CAPTURE_STREAM_BYTES,
    )
    .await;
    let mut outcome = match output.result {
        Ok(value) => ToolExecutionOutcome::succeeded(plan, value),
        Err(value) => ToolExecutionOutcome::failed(plan, "command_failed", value),
    };
    outcome.pending_captures = output.captures;
    outcome
}

pub(super) async fn execute_batch(
    project_root: &Path,
    plan: ToolExecutionPlan,
) -> ToolExecutionOutcome {
    execute_batch_observed(project_root, plan, None).await
}

pub(super) async fn execute_batch_observed(
    project_root: &Path,
    plan: ToolExecutionPlan,
    observer: Option<ToolObservationSender>,
) -> ToolExecutionOutcome {
    let arguments: RunCommandsArgs =
        match decode_arguments("run_commands", plan.call.arguments.clone()) {
            Ok(arguments) => arguments,
            Err(error) => return invalid_plan(plan, error),
        };
    let resource_sets = match command_resource_sets(&arguments, project_root) {
        Ok(keys) => keys,
        Err(error) => return invalid_plan(plan, error),
    };
    let max_each = per_item_limit(plan.output_policy.max_bytes, arguments.commands.len());
    let raw_each = batch_capture_stream_limit(arguments.commands.len());
    let observation = BatchObservation {
        plan: &plan,
        count: arguments.commands.len(),
        observer: observer.as_ref(),
    };
    let results = match arguments.mode {
        CommandBatchMode::Serial => {
            run_serial(
                project_root,
                &arguments.commands,
                max_each,
                raw_each,
                &observation,
            )
            .await
        }
        CommandBatchMode::Parallel => {
            run_parallel(
                project_root,
                &arguments.commands,
                &resource_sets,
                max_each,
                raw_each,
                &observation,
            )
            .await
        }
    };
    let partial_failure = results.iter().any(|output| output.result.is_err());
    let mut captures = Vec::new();
    let mut published = Vec::new();
    let commands = results
        .into_iter()
        .enumerate()
        .map(|(index, mut output)| {
            for capture in &mut output.captures {
                capture.child_index = Some(index as u8);
            }
            captures.extend(output.captures);
            published.extend(output.published);
            match output.result {
                Ok(value) => json!({"index": index, "status": "succeeded", "result": value}),
                Err(value) => json!({"index": index, "status": "failed", "result": value}),
            }
        })
        .collect::<Vec<_>>();
    let mut outcome = ToolExecutionOutcome::succeeded(
        plan,
        json!({"commands": commands, "partial_failure": partial_failure}),
    );
    outcome.pending_captures = captures;
    if observer.is_some() {
        outcome.result.output["output_captures"] =
            serde_json::to_value(&published).expect("capture serialization");
        outcome.receipt.output_captures = published;
    }
    outcome
}

struct BatchObservation<'a> {
    plan: &'a ToolExecutionPlan,
    count: usize,
    observer: Option<&'a ToolObservationSender>,
}

impl BatchObservation<'_> {
    async fn completed(&self, index: usize, mut output: CapturedCommand) -> CapturedCommand {
        if let Some(observer) = self.observer {
            for capture in &mut output.captures {
                capture.child_index = Some(index as u8);
            }
            let checkpoint =
                SubcommandCheckpoint::observed(self.plan, self.count, index, output.result.clone());
            output.published = observer
                .checkpoint(checkpoint, std::mem::take(&mut output.captures))
                .await;
        }
        output
    }
}

pub(super) fn validate_command(
    arguments: &RunCommandArgs,
    tool_name: &str,
) -> Result<(), ToolExecutorError> {
    if arguments.argv.is_empty() || arguments.argv[0].is_empty() {
        return Err(invalid_arguments(
            tool_name,
            "`argv` must contain a non-empty program",
        ));
    }
    for resource in &arguments.resources {
        match resource {
            CommandResource::Device { serial } if serial.is_empty() => {
                return Err(invalid_arguments(
                    tool_name,
                    "device `serial` must not be empty",
                ));
            }
            CommandResource::Pty { session } if session.is_empty() => {
                return Err(invalid_arguments(
                    tool_name,
                    "PTY `session` must not be empty",
                ));
            }
            CommandResource::Output { path } if path.is_empty() => {
                return Err(invalid_arguments(
                    tool_name,
                    "output `path` must not be empty",
                ));
            }
            CommandResource::Device { .. }
            | CommandResource::Pty { .. }
            | CommandResource::Output { .. } => {}
        }
    }
    Ok(())
}

fn command_resource_sets(
    arguments: &RunCommandsArgs,
    project_root: &Path,
) -> Result<Vec<Vec<ResourceKey>>, ToolExecutorError> {
    if arguments.commands.is_empty() || arguments.commands.len() > MAX_COMMANDS {
        return Err(invalid_arguments(
            "run_commands",
            "`commands` must contain between 1 and 32 entries",
        ));
    }
    arguments
        .commands
        .iter()
        .map(|command| {
            validate_command(command, "run_commands")?;
            Ok(command_resource_keys(project_root, command))
        })
        .collect()
}

pub(super) fn command_resource_keys(
    project_root: &Path,
    arguments: &RunCommandArgs,
) -> Vec<ResourceKey> {
    if arguments.resources.is_empty() {
        return vec![ResourceKey::GlobalSerial];
    }
    let resolver = ResourceKeyResolver::new(project_root);
    arguments
        .resources
        .iter()
        .map(|resource| match resource {
            CommandResource::Device { serial } => resolver.device(serial),
            CommandResource::Pty { session } => resolver.pty(session),
            CommandResource::Output { path } => resolver.output(path),
        })
        .collect()
}

async fn run_serial(
    project_root: &Path,
    commands: &[RunCommandArgs],
    max_bytes: usize,
    raw_bytes: usize,
    observation: &BatchObservation<'_>,
) -> Vec<CapturedCommand> {
    let mut results = Vec::with_capacity(commands.len());
    for (index, command) in commands.iter().enumerate() {
        results.push(
            observation
                .completed(
                    index,
                    run(project_root, command, max_bytes, raw_bytes).await,
                )
                .await,
        );
    }
    results
}

/// The command batch has no second Resource Key definition: it forms waves
/// with the Domain conflict predicate used by the AgentLoop scheduler.
async fn run_parallel(
    project_root: &Path,
    commands: &[RunCommandArgs],
    resource_sets: &[Vec<ResourceKey>],
    max_bytes: usize,
    raw_bytes: usize,
    observation: &BatchObservation<'_>,
) -> Vec<CapturedCommand> {
    let mut pending = (0..commands.len()).collect::<VecDeque<_>>();
    let mut results = (0..commands.len()).map(|_| None).collect::<Vec<_>>();

    while !pending.is_empty() {
        let mut wave: Vec<usize> = Vec::new();
        while let Some(position) = pending.iter().position(|candidate| {
            wave.iter().all(|active| {
                !scheduling_conflicts(
                    ToolParallelism::ResourceLocked,
                    ToolEffect::Interactive,
                    resource_sets[*candidate].as_slice(),
                    ToolParallelism::ResourceLocked,
                    ToolEffect::Interactive,
                    resource_sets[*active].as_slice(),
                )
            })
        }) {
            if let Some(index) = pending.remove(position) {
                wave.push(index);
            }
        }
        let completed = join_all(wave.iter().map(|index| async move {
            observation
                .completed(
                    *index,
                    run(project_root, &commands[*index], max_bytes, raw_bytes).await,
                )
                .await
        }))
        .await;
        for (index, result) in wave.into_iter().zip(completed) {
            results[index] = Some(result);
        }
    }

    results
        .into_iter()
        .map(|result| result.expect("each admitted command is assigned to a completed wave"))
        .collect()
}

async fn run(
    project_root: &Path,
    arguments: &RunCommandArgs,
    max_bytes: usize,
    raw_bytes: usize,
) -> CapturedCommand {
    let cwd = resolve_path(project_root, arguments.cwd.as_deref().unwrap_or("."));
    let argv = arguments
        .argv
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let output = run_probe_at(cwd.clone(), &argv, max_bytes, raw_bytes).await;
    let mut value = output_value(project_root, &output.cwd, &output);
    if let Some(error) = &output.error {
        value["error"] = Value::from(error.clone());
    }
    let result = if output.exit_code == Some(0) && output.error.is_none() {
        Ok(value)
    } else {
        Err(value)
    };
    CapturedCommand {
        result,
        captures: output.captures,
        published: Vec::new(),
    }
}

struct CapturedCommand {
    result: Result<Value, Value>,
    captures: Vec<PendingOutputCapture>,
    published: Vec<OutputCapture>,
}

pub(super) struct FixedProbeOutput {
    pub(super) cwd: std::path::PathBuf,
    pub(super) exit_code: Option<i32>,
    pub(super) stdout: BoundedText,
    pub(super) stderr: BoundedText,
    pub(super) error: Option<String>,
    captures: Vec<PendingOutputCapture>,
}

enum ProcessFailureStage {
    Spawn,
    Wait,
    ReadStdout,
    ReadStderr,
}

impl ProcessFailureStage {
    // Diagnostic prose inside the existing error string, not ToolFailure's
    // execution stage or a new persisted schema. ENOENT has no guessed cause.
    fn describe(&self, program: &str, cwd: &Path, error: impl std::fmt::Display) -> String {
        let stage = match self {
            Self::Spawn => "spawn",
            Self::Wait => "wait",
            Self::ReadStdout => "read_stdout",
            Self::ReadStderr => "read_stderr",
        };
        format!("program={program:?}; effective_cwd={cwd:?}; process_stage={stage}; error={error}")
    }
}

/// Executes one adapter-owned fixed probe.  This is deliberately narrower
/// than `run_command`: callers provide a compile-time command list. Process
/// lifetime, kill-on-drop and output bounds remain owned by this module.
pub(super) async fn run_fixed_probe(
    project_root: &Path,
    argv: &[&str],
    max_bytes: usize,
) -> FixedProbeOutput {
    let cwd = resolve_path(project_root, ".");
    run_probe_at(cwd, argv, max_bytes, max_bytes / 2).await
}

async fn run_probe_at(
    cwd: std::path::PathBuf,
    argv: &[&str],
    max_bytes: usize,
    raw_bytes: usize,
) -> FixedProbeOutput {
    let Some(program) = argv.first() else {
        return FixedProbeOutput {
            cwd,
            exit_code: None,
            stdout: bounded_text(&[], max_bytes / 2),
            stderr: bounded_text(&[], max_bytes / 2),
            error: Some("fixed probe has no program".to_owned()),
            captures: Vec::new(),
        };
    };
    let mut command = Command::new(program);
    command
        .args(&argv[1..])
        .current_dir(&cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            let message = ProcessFailureStage::Spawn.describe(program, &cwd, &error);
            return FixedProbeOutput {
                cwd,
                exit_code: None,
                stdout: bounded_text(&[], max_bytes / 2),
                stderr: bounded_text(&[], max_bytes / 2),
                error: Some(message),
                captures: Vec::new(),
            };
        }
    };
    let stdout = child
        .stdout
        .take()
        .expect("fixed probe stdout is configured as piped");
    let stderr = child
        .stderr
        .take()
        .expect("fixed probe stderr is configured as piped");
    let max_each = max_bytes / 2;
    let mut stdout_capture = CapturedOutput::new(raw_bytes);
    let mut stderr_capture = CapturedOutput::new(raw_bytes);
    let (status, stdout_incomplete, stderr_incomplete) = {
        let stdout_reader = read_bounded_output(stdout, &mut stdout_capture);
        let stderr_reader = read_bounded_output(stderr, &mut stderr_capture);
        tokio::pin!(stdout_reader);
        tokio::pin!(stderr_reader);
        let mut stdout_done = false;
        let mut stderr_done = false;
        let status = loop {
            tokio::select! {
                () = &mut stdout_reader, if !stdout_done => stdout_done = true,
                () = &mut stderr_reader, if !stderr_done => stderr_done = true,
                status = child.wait() => break status,
            }
        };
        let grace = tokio::time::sleep(OUTPUT_DRAIN_GRACE);
        tokio::pin!(grace);
        while !stdout_done || !stderr_done {
            tokio::select! {
                () = &mut stdout_reader, if !stdout_done => stdout_done = true,
                () = &mut stderr_reader, if !stderr_done => stderr_done = true,
                () = &mut grace => break,
            }
        }
        (status, !stdout_done, !stderr_done)
    };
    // The reader futures and their local pipe handles dropped with the inner
    // scope above, so no reader can mutate a capture after it is incomplete.
    if stdout_incomplete {
        stdout_capture.mark_incomplete("stdout_reader_grace_timeout");
    }
    if stderr_incomplete {
        stderr_capture.mark_incomplete("stderr_reader_grace_timeout");
    }
    let error = status
        .as_ref()
        .err()
        .map(|error| ProcessFailureStage::Wait.describe(program, &cwd, error));
    let captures = vec![
        stdout_capture.capture(OutputStream::Stdout, None),
        stderr_capture.capture(OutputStream::Stderr, None),
    ];
    let error = error.or_else(|| {
        captures
            .iter()
            .zip([
                ProcessFailureStage::ReadStdout,
                ProcessFailureStage::ReadStderr,
            ])
            .find_map(|(capture, stage)| {
                capture
                    .read_error
                    .as_ref()
                    .map(|error| stage.describe(program, &cwd, error))
            })
    });
    FixedProbeOutput {
        cwd,
        exit_code: status.ok().and_then(|status| status.code()),
        stdout: stdout_capture.preview(max_each),
        stderr: stderr_capture.preview(max_each),
        error,
        captures,
    }
}

/// Drains a child pipe so the process cannot block while retaining at most
/// the caller's byte budget in memory. The final UTF-8 view remains owned by
/// the existing bounded-text normalizer.
async fn read_bounded_output<Reader>(mut reader: Reader, captured: &mut CapturedOutput)
where
    Reader: AsyncRead + Unpin,
{
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        let read = match reader.read(&mut buffer).await {
            Ok(read) => read,
            Err(error) => {
                captured.set_read_error(error.to_string());
                break;
            }
        };
        if read == 0 {
            break;
        }
        captured.append(&buffer[..read]);
    }
    captured.seal();
}

fn output_value(project_root: &Path, cwd: &Path, output: &FixedProbeOutput) -> Value {
    json!({
        "cwd": display_path(project_root, cwd),
        "exit_code": output.exit_code,
        "stdout": output.stdout.text,
        "stderr": output.stderr.text,
        "truncated": output.stdout.truncated || output.stderr.truncated,
    })
}

fn per_item_limit(total: usize, count: usize) -> usize {
    total / count.max(1)
}
