//! Argv-only single and batched command execution.

use super::ToolRegistry;
use crate::domain::tools::{ToolDeterminism, ToolExposure, ToolRetryPolicy, ToolSpec};

use std::collections::VecDeque;
use std::fs;
use std::io::Read;
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
use crate::ports::tool::ToolObservationSender;
use crate::ports::tool::{ToolCancellation, ToolExecutionError, ToolExecutorError};

use super::filesystem::{display_path, resolve_path};
use super::output::{BoundedText, CapturedOutput, OUTPUT_DRAIN_GRACE, bounded_text};
use super::resources::ResourceKeyResolver;
use super::{decode_arguments, invalid_arguments, invalid_plan};
use crate::domain::agent_evolution::{ScriptFile, validate_input_files};

fn verified_inputs_schema() -> Value {
    json!({"type":"array","maxItems":8,"items":{"type":"object","properties":{"path":{"type":"string","minLength":1},"blake3":{"type":"string","pattern":"^[0-9a-f]{64}$"}},"required":["path","blake3"],"additionalProperties":false}})
}

const MAX_COMMANDS: usize = 32;
const MAX_ATTESTED_INPUT_BYTES: usize = 1024 * 1024;

/// Reads the local regular-file inputs used by command attestation and RSI copies.
pub(crate) fn read_attested_input(path: &Path) -> Result<(Vec<u8>, fs::Permissions), String> {
    // Reject static FIFOs/devices/links before open; confirm the actual opened
    // descriptor as well. This local-input contract is not OS race isolation.
    let entry = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !entry.is_file() {
        return Err("input must be a regular file, not a link or special file".to_owned());
    }
    let file = fs::File::open(path).map_err(|error| error.to_string())?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("opened input is not a regular file".to_owned());
    }
    if metadata.len() > MAX_ATTESTED_INPUT_BYTES as u64 {
        return Err("input exceeds 1MiB".to_owned());
    }
    let mut bytes = Vec::new();
    file.take((MAX_ATTESTED_INPUT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > MAX_ATTESTED_INPUT_BYTES {
        return Err("input exceeds 1MiB".to_owned());
    }
    Ok((bytes, metadata.permissions()))
}

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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) verified_inputs: Vec<ScriptFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) code_inputs: Vec<String>,
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

fn normalize_one(
    project_root: &Path,
    value: Value,
    input_verification_enabled: bool,
    automatic_verification_enabled: bool,
) -> Result<(Value, Vec<ResourceKey>), ToolExecutorError> {
    if !automatic_verification_enabled && value.get("code_inputs").is_some() {
        return Err(invalid_arguments(
            "run_command",
            "code_inputs requires RSI improvement opt-in",
        ));
    }
    if !input_verification_enabled && value.get("verified_inputs").is_some() {
        return Err(invalid_arguments(
            "run_command",
            "verified_inputs is available only for an RSI-enabled Agent binding",
        ));
    }
    let arguments: RunCommandArgs = decode_arguments("run_command", value)?;
    validate_command(&arguments, "run_command")?;
    let keys = command_resource_keys(project_root, &arguments);
    Ok((
        serde_json::to_value(arguments).expect("RunCommandArgs serialization cannot fail"),
        keys,
    ))
}

fn normalize_batch(
    project_root: &Path,
    value: Value,
    input_verification_enabled: bool,
    automatic_verification_enabled: bool,
) -> Result<(Value, Vec<ResourceKey>), ToolExecutorError> {
    if !automatic_verification_enabled
        && value
            .get("commands")
            .and_then(Value::as_array)
            .is_some_and(|commands| {
                commands
                    .iter()
                    .any(|command| command.get("code_inputs").is_some())
            })
    {
        return Err(invalid_arguments(
            "run_commands",
            "code_inputs requires RSI improvement opt-in",
        ));
    }
    if !input_verification_enabled
        && value
            .get("commands")
            .and_then(Value::as_array)
            .is_some_and(|commands| {
                commands
                    .iter()
                    .any(|command| command.get("verified_inputs").is_some())
            })
    {
        return Err(invalid_arguments(
            "run_commands",
            "verified_inputs is available only for an RSI-enabled Agent binding",
        ));
    }
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

async fn execute_one(
    project_root: &Path,
    plan: ToolExecutionPlan,
    cancellation: &ToolCancellation,
) -> Result<ToolExecutionOutcome, ToolExecutionError> {
    if cancellation.is_cancelled() {
        return Ok(ToolExecutionOutcome::cancelled(
            plan,
            "agent_cancelled_before_process_start",
        ));
    }
    let arguments: RunCommandArgs =
        match decode_arguments("run_command", plan.call.arguments.clone()) {
            Ok(arguments) => arguments,
            Err(error) => return Ok(invalid_plan(plan, error)),
        };
    if let Err(error) = validate_command(&arguments, "run_command") {
        return Ok(invalid_plan(plan, error));
    }
    let output = run(
        project_root,
        &arguments,
        plan.output_policy.max_bytes,
        MAX_CAPTURE_STREAM_BYTES,
        cancellation,
    )
    .await?;
    let mut outcome = if output.cancelled {
        ToolExecutionOutcome::cancelled_with_output(
            plan,
            "agent_cancelled",
            output.result.unwrap_or_else(|value| value),
        )
    } else {
        match output.result {
            Ok(value) => ToolExecutionOutcome::succeeded(plan, value),
            Err(value) => ToolExecutionOutcome::failed(plan, "command_failed", value),
        }
    };
    outcome.pending_captures = output.captures;
    Ok(outcome)
}

async fn execute_batch_observed(
    project_root: &Path,
    plan: ToolExecutionPlan,
    observer: Option<ToolObservationSender>,
    cancellation: &ToolCancellation,
) -> Result<ToolExecutionOutcome, ToolExecutionError> {
    let arguments: RunCommandsArgs =
        match decode_arguments("run_commands", plan.call.arguments.clone()) {
            Ok(arguments) => arguments,
            Err(error) => return Ok(invalid_plan(plan, error)),
        };
    let resource_sets = match command_resource_sets(&arguments, project_root) {
        Ok(keys) => keys,
        Err(error) => return Ok(invalid_plan(plan, error)),
    };
    let max_each = per_item_limit(plan.output_policy.max_bytes, arguments.commands.len());
    let raw_each = batch_capture_stream_limit(arguments.commands.len());
    let observation = BatchObservation {
        plan: &plan,
        count: arguments.commands.len(),
        observer: observer.as_ref(),
        cancellation,
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
    }?;
    let partial_failure = results.iter().any(|(_, output)| output.result.is_err());
    let mut captures = Vec::new();
    let mut published = Vec::new();
    let commands = results
        .into_iter()
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
    let output = json!({"commands": commands, "partial_failure": partial_failure});
    let mut outcome = if cancellation.is_cancelled() {
        ToolExecutionOutcome::cancelled_with_output(plan, "agent_cancelled", output)
    } else {
        ToolExecutionOutcome::succeeded(plan, output)
    };
    outcome.pending_captures = captures;
    if observer.is_some() {
        outcome.result.output["output_captures"] =
            serde_json::to_value(&published).expect("capture serialization");
        outcome.receipt.output_captures = published;
    }
    Ok(outcome)
}

struct BatchObservation<'a> {
    plan: &'a ToolExecutionPlan,
    count: usize,
    observer: Option<&'a ToolObservationSender>,
    cancellation: &'a ToolCancellation,
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
    validate_input_files(&arguments.verified_inputs)
        .map_err(|message| invalid_arguments(tool_name, message))?;
    let automatic = arguments
        .code_inputs
        .iter()
        .map(|path| ScriptFile {
            path: path.clone(),
            blake3: "0".repeat(64),
        })
        .collect::<Vec<_>>();
    validate_input_files(&automatic).map_err(|message| invalid_arguments(tool_name, message))?;
    if arguments
        .verified_inputs
        .iter()
        .map(|file| &file.path)
        .chain(arguments.code_inputs.iter())
        .collect::<std::collections::BTreeSet<_>>()
        .len()
        > 8
    {
        return Err(invalid_arguments(
            tool_name,
            "at most 8 combined code inputs are supported",
        ));
    }
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
) -> Result<Vec<(usize, CapturedCommand)>, ToolExecutionError> {
    let mut results = Vec::with_capacity(commands.len());
    for (index, command) in commands.iter().enumerate() {
        if observation.cancellation.is_cancelled() {
            break;
        }
        results.push((
            index,
            observation
                .completed(
                    index,
                    run(
                        project_root,
                        command,
                        max_bytes,
                        raw_bytes,
                        observation.cancellation,
                    )
                    .await?,
                )
                .await,
        ));
    }
    Ok(results)
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
) -> Result<Vec<(usize, CapturedCommand)>, ToolExecutionError> {
    let mut pending = (0..commands.len()).collect::<VecDeque<_>>();
    let mut results = (0..commands.len()).map(|_| None).collect::<Vec<_>>();

    while !pending.is_empty() && !observation.cancellation.is_cancelled() {
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
            Ok(observation
                .completed(
                    *index,
                    run(
                        project_root,
                        &commands[*index],
                        max_bytes,
                        raw_bytes,
                        observation.cancellation,
                    )
                    .await?,
                )
                .await)
        }))
        .await;
        for (index, result) in wave.into_iter().zip(completed) {
            results[index] = Some(result?);
        }
    }

    Ok(results
        .into_iter()
        .enumerate()
        .filter_map(|(index, result)| result.map(|output| (index, output)))
        .collect())
}

async fn run(
    project_root: &Path,
    arguments: &RunCommandArgs,
    max_bytes: usize,
    raw_bytes: usize,
    cancellation: &ToolCancellation,
) -> Result<CapturedCommand, ToolExecutionError> {
    let mut arguments = arguments.clone();
    for path in arguments.code_inputs.clone() {
        let bytes = match read_attested_input(&project_root.join(&path)) {
            Ok((bytes, _)) => bytes,
            Err(error) => {
                return Ok(verification_failure(
                    project_root,
                    &arguments,
                    format!("read code input `{path}`: {error}"),
                ));
            }
        };
        let digest = blake3::hash(&bytes).to_hex().to_string();
        if let Some(expected) = arguments
            .verified_inputs
            .iter()
            .find(|file| file.path == path)
        {
            if expected.blake3 != digest {
                return Ok(verification_failure(
                    project_root,
                    &arguments,
                    format!("code input `{path}` does not match explicit expected digest"),
                ));
            }
        } else {
            arguments.verified_inputs.push(ScriptFile {
                path,
                blake3: digest,
            });
        }
    }
    let cwd = resolve_path(project_root, arguments.cwd.as_deref().unwrap_or("."));
    if let Err(error) = verify_inputs(project_root, &arguments.verified_inputs) {
        return Ok(CapturedCommand {
            result: Err(
                json!({"cwd":display_path(project_root, &cwd),"exit_code":null,"stdout":"","stderr":"","truncated":false,"error":error}),
            ),
            captures: Vec::new(),
            published: Vec::new(),
            cancelled: false,
        });
    }
    let argv = arguments
        .argv
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let output = run_probe_at(cwd.clone(), &argv, max_bytes, raw_bytes, cancellation).await?;
    let mut value = output_value(project_root, &output.cwd, &output);
    if let Some(error) = &output.error {
        value["error"] = Value::from(error.clone());
    }
    let verification = verify_inputs(project_root, &arguments.verified_inputs);
    if let Err(error) = &verification {
        value["error"] = Value::from(error.clone());
    }
    if verification.is_ok() && !arguments.verified_inputs.is_empty() {
        value["verified_inputs"] = serde_json::to_value(&arguments.verified_inputs)
            .expect("validated input identities serialize");
    }
    let result = if output.exit_code == Some(0) && output.error.is_none() && verification.is_ok() {
        Ok(value)
    } else {
        Err(value)
    };
    Ok(CapturedCommand {
        result,
        captures: output.captures,
        published: Vec::new(),
        cancelled: output.cancelled,
    })
}

fn verification_failure(
    project_root: &Path,
    arguments: &RunCommandArgs,
    error: String,
) -> CapturedCommand {
    CapturedCommand {
        result: Err(
            json!({"cwd":display_path(project_root, &resolve_path(project_root, arguments.cwd.as_deref().unwrap_or("."))),"exit_code":null,"stdout":"","stderr":"","truncated":false,"error":error}),
        ),
        captures: Vec::new(),
        published: Vec::new(),
        cancelled: false,
    }
}

fn verify_inputs(project_root: &Path, inputs: &[ScriptFile]) -> Result<(), String> {
    for input in inputs {
        let path = project_root.join(&input.path);
        let (bytes, _) = read_attested_input(&path)
            .map_err(|error| format!("read verified input `{}`: {error}", input.path))?;
        if blake3::hash(&bytes).to_hex().as_str() != input.blake3 {
            return Err(format!(
                "verified input `{}` does not match its declared bytes",
                input.path
            ));
        }
    }
    Ok(())
}

struct CapturedCommand {
    cancelled: bool,
    result: Result<Value, Value>,
    captures: Vec<PendingOutputCapture>,
    published: Vec<OutputCapture>,
}

pub(super) struct FixedProbeOutput {
    cancelled: bool,
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
    run_probe_at(
        cwd,
        argv,
        max_bytes,
        max_bytes / 2,
        &ToolCancellation::never(),
    )
    .await
    .expect("a fixed probe has no cooperative cancellation signal")
}

async fn run_probe_at(
    cwd: std::path::PathBuf,
    argv: &[&str],
    max_bytes: usize,
    raw_bytes: usize,
    cancellation: &ToolCancellation,
) -> Result<FixedProbeOutput, ToolExecutionError> {
    let Some(program) = argv.first() else {
        return Ok(FixedProbeOutput {
            cancelled: false,
            cwd,
            exit_code: None,
            stdout: bounded_text(&[], max_bytes / 2),
            stderr: bounded_text(&[], max_bytes / 2),
            error: Some("fixed probe has no program".to_owned()),
            captures: Vec::new(),
        });
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
            return Ok(FixedProbeOutput {
                cancelled: false,
                cwd,
                exit_code: None,
                stdout: bounded_text(&[], max_bytes / 2),
                stderr: bounded_text(&[], max_bytes / 2),
                error: Some(message),
                captures: Vec::new(),
            });
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
    let mut cancelled = false;
    let (status, stdout_incomplete, stderr_incomplete) = {
        let stdout_reader = read_bounded_output(stdout, &mut stdout_capture);
        let stderr_reader = read_bounded_output(stderr, &mut stderr_capture);
        tokio::pin!(stdout_reader);
        tokio::pin!(stderr_reader);
        let mut stdout_done = false;
        let mut stderr_done = false;
        let status = loop {
            tokio::select! {
                biased;
                status = child.wait() => break status,
                () = cancellation.wait(), if !cancelled => {
                    child.start_kill().map_err(|error| ToolExecutionError::OutcomeUnknown {
                        safe_reason: format!("foreground command cancellation failed: {error}"),
                    })?;
                    cancelled = true;
                }
                () = &mut stdout_reader, if !stdout_done => stdout_done = true,
                () = &mut stderr_reader, if !stderr_done => stderr_done = true,
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
    if cancelled && let Err(error) = &status {
        return Err(ToolExecutionError::OutcomeUnknown {
            safe_reason: format!("foreground command wait after cancellation failed: {error}"),
        });
    }
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
    Ok(FixedProbeOutput {
        cancelled,
        cwd,
        exit_code: status.ok().and_then(|status| status.code()),
        stdout: stdout_capture.preview(max_each),
        stderr: stderr_capture.preview(max_each),
        error,
        captures,
    })
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

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum Tool {
    RunCommand,
    RunCommands,
}

impl Tool {
    pub(super) const ALL: [Self; 2] = [Self::RunCommand, Self::RunCommands];

    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::RunCommand => "run_command",
            Self::RunCommands => "run_commands",
        }
    }

    pub(super) fn spec(self, registry: &ToolRegistry) -> ToolSpec {
        let mut spec = match self {
            Self::RunCommand => super::spec(
                self.name(),
                "3",
                super::description(
                    "one bounded local command",
                    "a batch or interactive terminal is needed.",
                    "argv, not a shell string. Omitted/null cwd uses the project root; '.' is project-relative. For a path discrepancy, compare pwd -P and the requested path before assuming a missing directory: symlink aliases such as /tmp and /private/tmp may refer to one location, but must be observed. Check the actual program path/implementation/version in the intended host or container before choosing platform-specific flags; ENOENT alone cannot distinguish program, cwd, interpreter or loader absence.",
                    "bounded lossy retained-prefix head/tail preview; output_captures contains durable raw Blob refs for read_output after publication.",
                    false,
                ),
                ToolExposure::Direct,
                json!({
                    "type": "object",
                    "properties": {
                        "argv": {"type": "array", "items": {"type": "string"}, "minItems": 1},
                        "cwd": {"type": "string"},
                        "resources": {"type": "array", "items": resource_schema()}
                    },
                    "required": ["argv"],
                    "additionalProperties": false
                }),
                self.output_schema(),
                ToolParallelism::ResourceLocked,
                Some("resource:{resources}"),
                ToolEffect::Interactive,
                ToolRetryPolicy::NeverAutoRetry,
                ToolDeterminism::Nondeterministic,
                120_000,
            ),
            Self::RunCommands => super::spec(
                self.name(),
                "4",
                super::description(
                    "independent bounded commands",
                    "one command or a long/interactive process is needed; use run_command, start_job or start_pty instead.",
                    "short independent checks in parallel, or explicit serial order for dependent steps. cwd has the same project-relative semantics as run_command; a child failure is not proof of broken parallel execution. Serial ordering does not stop later commands after an earlier command fails; only submit later steps whose prerequisites are already established.",
                    "ordered results and failures.",
                    false,
                ),
                ToolExposure::Direct,
                json!({
                    "type": "object",
                    "properties": {
                        "mode": {"type": "string", "enum": ["serial", "parallel"]},
                        "commands": {
                            "type": "array",
                            "minItems": 1,
                            "maxItems": 32,
                            "items": {
                                "type": "object",
                                "properties": {
                                    "argv": {"type": "array", "items": {"type": "string"}, "minItems": 1},
                                    "cwd": {"type": "string"},
                                    "resources": {"type": "array", "items": resource_schema()}
                                },
                                "required": ["argv"],
                                "additionalProperties": false
                            }
                        }
                    },
                    "required": ["mode", "commands"],
                    "additionalProperties": false
                }),
                self.output_schema(),
                ToolParallelism::ResourceLocked,
                Some("resource:{commands[*].resources}"),
                ToolEffect::Interactive,
                ToolRetryPolicy::NeverAutoRetry,
                ToolDeterminism::Nondeterministic,
                120_000,
            ),
        };
        if registry.input_verification_enabled {
            let (input_properties, output_properties) = if self == Self::RunCommand {
                (
                    &mut spec.input_schema["properties"],
                    &mut spec.output_schema["properties"],
                )
            } else {
                (
                    &mut spec.input_schema["properties"]["commands"]["items"]["properties"],
                    &mut spec.output_schema["properties"]["commands"]["items"]["properties"]["result"]
                        ["properties"],
                )
            };
            input_properties["verified_inputs"] = verified_inputs_schema();
            if registry.automatic_verification_enabled {
                input_properties["code_inputs"] = json!({"type":"array","maxItems":8,"items":{"type":"string","minLength":1},"uniqueItems":true});
            }
            output_properties["verified_inputs"] = verified_inputs_schema();
            spec.version = if self == Self::RunCommand { "4" } else { "5" }.to_owned();
        }
        spec
    }

    pub(super) fn normalize(
        self,
        registry: &ToolRegistry,
        arguments: Value,
    ) -> Result<(Value, Vec<ResourceKey>), ToolExecutorError> {
        let project_root = &registry.project_root;
        let input_verification_enabled = registry.input_verification_enabled;
        match self {
            Self::RunCommand => normalize_one(
                project_root,
                arguments,
                input_verification_enabled,
                registry.automatic_verification_enabled,
            ),
            Self::RunCommands => normalize_batch(
                project_root,
                arguments,
                input_verification_enabled,
                registry.automatic_verification_enabled,
            ),
        }
    }

    pub(super) async fn execute(
        self,
        registry: &ToolRegistry,
        plan: ToolExecutionPlan,
        observer: Option<crate::ports::tool::ToolObservationSender>,
        cancellation: ToolCancellation,
    ) -> Result<ToolExecutionOutcome, ToolExecutionError> {
        match self {
            Self::RunCommand => execute_one(&registry.project_root, plan, &cancellation).await,
            Self::RunCommands => {
                execute_batch_observed(&registry.project_root, plan, observer, &cancellation).await
            }
        }
    }

    fn output_schema(self) -> Value {
        let mut schema = match self {
            Self::RunCommand => command_output_schema(),
            Self::RunCommands => json!({
                "type": "object", "properties": {
                    "commands": {"type": "array", "minItems": 1, "maxItems": 32, "items": {"type": "object", "properties": {
                        "index": {"type": "integer", "minimum": 0}, "status": {"type": "string", "enum": ["succeeded", "failed"]}, "result": command_output_schema()
                    }, "required": ["index", "status", "result"], "additionalProperties": false}},
                    "partial_failure": json!({"type": "boolean"})
                }, "required": ["commands", "partial_failure"], "additionalProperties": false
            }),
        };
        schema["properties"]["output_captures"] = crate::domain::tools::capture_schema();
        schema
    }
}

fn command_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "cwd": {"type": "string", "minLength": 1}, "exit_code": {"type": ["integer", "null"]},
            "stdout": {"type": "string"}, "stderr": {"type": "string"}, "truncated": {"type": "boolean"}
        },
        "required": ["cwd", "exit_code", "stdout", "stderr", "truncated"],
        "additionalProperties": false
    })
}
