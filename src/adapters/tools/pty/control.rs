//! Poll, input, resize and stop controls for a live PTY session.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::watch;

use portable_pty::PtySize;

use crate::domain::tools::{
    PtySessionCompletion, PtySessionId, PtySessionStatus, ResourceKey, ToolExecutionOutcome,
    ToolExecutionPlan,
};
use crate::ports::tool::ToolExecutorError;

use super::super::output::{OffsetError, lock_captured_output};
use super::super::{decode_arguments, invalid_arguments, invalid_plan};
use super::{PtySessions, lock_killer, lock_master, lock_table, lock_writer};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PtySessionIdArgs {
    pty_session_id: PtySessionId,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PollPtyArgs {
    pty_session_id: PtySessionId,
    #[serde(default)]
    offset: u64,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct InputPtyArgs {
    pty_session_id: PtySessionId,
    input: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ResizePtyArgs {
    pty_session_id: PtySessionId,
    rows: u16,
    cols: u16,
}

pub(in crate::adapters::tools) fn normalize_poll(
    value: Value,
) -> Result<(Value, Vec<ResourceKey>), ToolExecutorError> {
    let arguments: PollPtyArgs = decode_arguments("poll_pty", value)?;
    validate_session_id("poll_pty", &arguments.pty_session_id)?;
    let session_id = arguments.pty_session_id.clone();
    Ok((
        serde_json::to_value(arguments).expect("PollPtyArgs serialization cannot fail"),
        vec![ResourceKey::pty(session_id.as_str())],
    ))
}

pub(in crate::adapters::tools) fn normalize_input(
    value: Value,
) -> Result<(Value, Vec<ResourceKey>), ToolExecutorError> {
    let arguments: InputPtyArgs = decode_arguments("input_pty", value)?;
    validate_session_id("input_pty", &arguments.pty_session_id)?;
    let session_id = arguments.pty_session_id.clone();
    Ok((
        serde_json::to_value(arguments).expect("InputPtyArgs serialization cannot fail"),
        vec![ResourceKey::pty(session_id.as_str())],
    ))
}

pub(in crate::adapters::tools) fn normalize_resize(
    value: Value,
) -> Result<(Value, Vec<ResourceKey>), ToolExecutorError> {
    let arguments: ResizePtyArgs = decode_arguments("resize_pty", value)?;
    validate_session_id("resize_pty", &arguments.pty_session_id)?;
    validate_size(&arguments)?;
    let session_id = arguments.pty_session_id.clone();
    Ok((
        serde_json::to_value(arguments).expect("ResizePtyArgs serialization cannot fail"),
        vec![ResourceKey::pty(session_id.as_str())],
    ))
}

pub(in crate::adapters::tools) fn normalize_stop(
    value: Value,
) -> Result<(Value, Vec<ResourceKey>), ToolExecutorError> {
    let arguments: PtySessionIdArgs = decode_arguments("stop_pty", value)?;
    validate_session_id("stop_pty", &arguments.pty_session_id)?;
    let session_id = arguments.pty_session_id.clone();
    Ok((
        serde_json::to_value(arguments).expect("PtySessionIdArgs serialization cannot fail"),
        vec![ResourceKey::pty(session_id.as_str())],
    ))
}

pub(in crate::adapters::tools) async fn execute_poll(
    sessions: &PtySessions,
    plan: ToolExecutionPlan,
) -> ToolExecutionOutcome {
    let arguments: PollPtyArgs = match decode_arguments("poll_pty", plan.call.arguments.clone()) {
        Ok(arguments) => arguments,
        Err(error) => return invalid_plan(plan, error),
    };
    let snapshot = {
        let table = lock_table(&sessions.sessions);
        let Some(session) = table.entries.get(&arguments.pty_session_id) else {
            return unknown_session(plan, &arguments.pty_session_id);
        };
        (
            session.status,
            session.output.clone(),
            session.completion.clone(),
        )
    };
    match lock_captured_output(&snapshot.1).slice_after_offset(arguments.offset) {
        Ok(output) => ToolExecutionOutcome::succeeded(
            plan,
            json!({
                "pty_session_id": arguments.pty_session_id,
                "status": snapshot.0,
                "output": output,
                "completion": snapshot.2,
            }),
        ),
        Err(error) => ToolExecutionOutcome::failed(
            plan,
            "pty_offset_out_of_range",
            json!({
                "pty_session_id": arguments.pty_session_id,
                "output": offset_error_value(error),
            }),
        ),
    }
}

pub(in crate::adapters::tools) async fn execute_input(
    sessions: &PtySessions,
    plan: ToolExecutionPlan,
) -> ToolExecutionOutcome {
    let arguments: InputPtyArgs = match decode_arguments("input_pty", plan.call.arguments.clone()) {
        Ok(arguments) => arguments,
        Err(error) => return invalid_plan(plan, error),
    };
    let writer = {
        let table = lock_table(&sessions.sessions);
        let Some(session) = table.entries.get(&arguments.pty_session_id) else {
            return unknown_session(plan, &arguments.pty_session_id);
        };
        if session.status != PtySessionStatus::Running {
            return not_running(plan, &arguments.pty_session_id, session.status);
        }
        session.writer.clone()
    };
    let Some(writer) = writer else {
        return ToolExecutionOutcome::failed(
            plan,
            "pty_input_closed",
            json!({"pty_session_id": arguments.pty_session_id, "error": "PTY input is closed"}),
        );
    };
    let input = arguments.input.clone();
    let write_result = tokio::task::spawn_blocking(move || {
        let mut writer = lock_writer(&writer);
        std::io::Write::write_all(&mut *writer, input.as_bytes())?;
        std::io::Write::flush(&mut *writer)
    })
    .await;
    match write_result {
        Ok(Ok(())) => ToolExecutionOutcome::succeeded(
            plan,
            json!({
                "pty_session_id": arguments.pty_session_id,
                "bytes_written": arguments.input.len(),
            }),
        ),
        Ok(Err(error)) => ToolExecutionOutcome::failed(
            plan,
            "pty_input_failed",
            json!({"pty_session_id": arguments.pty_session_id, "error": error.to_string()}),
        ),
        Err(error) => ToolExecutionOutcome::failed(
            plan,
            "pty_input_failed",
            json!({"pty_session_id": arguments.pty_session_id, "error": error.to_string()}),
        ),
    }
}

pub(in crate::adapters::tools) async fn execute_resize(
    sessions: &PtySessions,
    plan: ToolExecutionPlan,
) -> ToolExecutionOutcome {
    let arguments: ResizePtyArgs = match decode_arguments("resize_pty", plan.call.arguments.clone())
    {
        Ok(arguments) => arguments,
        Err(error) => return invalid_plan(plan, error),
    };
    if let Err(error) = validate_size(&arguments) {
        return invalid_plan(plan, error);
    }
    let master = {
        let table = lock_table(&sessions.sessions);
        let Some(session) = table.entries.get(&arguments.pty_session_id) else {
            return unknown_session(plan, &arguments.pty_session_id);
        };
        if session.status != PtySessionStatus::Running {
            return not_running(plan, &arguments.pty_session_id, session.status);
        }
        session.master.clone()
    };
    let Some(master) = master else {
        return ToolExecutionOutcome::failed(
            plan,
            "pty_resize_unavailable",
            json!({"pty_session_id": arguments.pty_session_id, "error": "PTY terminal is closed"}),
        );
    };
    let size = PtySize {
        rows: arguments.rows,
        cols: arguments.cols,
        pixel_width: 0,
        pixel_height: 0,
    };
    let resize_result =
        tokio::task::spawn_blocking(move || lock_master(&master).resize(size)).await;
    match resize_result {
        Ok(Ok(())) => ToolExecutionOutcome::succeeded(
            plan,
            json!({
                "pty_session_id": arguments.pty_session_id,
                "rows": arguments.rows,
                "cols": arguments.cols,
            }),
        ),
        Ok(Err(error)) => ToolExecutionOutcome::failed(
            plan,
            "pty_resize_failed",
            json!({"pty_session_id": arguments.pty_session_id, "error": error.to_string()}),
        ),
        Err(error) => ToolExecutionOutcome::failed(
            plan,
            "pty_resize_failed",
            json!({"pty_session_id": arguments.pty_session_id, "error": error.to_string()}),
        ),
    }
}

pub(in crate::adapters::tools) async fn execute_stop(
    sessions: &PtySessions,
    plan: ToolExecutionPlan,
) -> ToolExecutionOutcome {
    let arguments: PtySessionIdArgs =
        match decode_arguments("stop_pty", plan.call.arguments.clone()) {
            Ok(arguments) => arguments,
            Err(error) => return invalid_plan(plan, error),
        };
    let (terminal, killer) = {
        let mut table = lock_table(&sessions.sessions);
        let Some(session) = table.entries.get_mut(&arguments.pty_session_id) else {
            return unknown_session(plan, &arguments.pty_session_id);
        };
        if let Some(completion) = session.completion.clone() {
            return terminal_outcome(
                plan,
                &arguments.pty_session_id,
                session.status,
                completion,
                true,
            );
        }
        (session.terminal_receiver(), session.request_stop())
    };
    // `kill` is a synchronous portable-pty call. It is deliberately run off
    // the Tokio runtime; a natural completion may win this race, and the
    // supervisor remains the sole terminal publication authority.
    if let Some(killer) = killer {
        let _ = tokio::task::spawn_blocking(move || lock_killer(&killer).kill()).await;
    }
    let Some((status, completion)) =
        wait_for_terminal(sessions, &arguments.pty_session_id, terminal).await
    else {
        return ToolExecutionOutcome::failed(
            plan,
            "pty_terminal_unavailable",
            json!({
                "pty_session_id": arguments.pty_session_id,
                "error": "PTY terminal state was not published",
            }),
        );
    };
    terminal_outcome(plan, &arguments.pty_session_id, status, completion, false)
}

async fn wait_for_terminal(
    sessions: &PtySessions,
    session_id: &PtySessionId,
    mut terminal: watch::Receiver<()>,
) -> Option<(PtySessionStatus, PtySessionCompletion)> {
    loop {
        {
            let table = lock_table(&sessions.sessions);
            let session = table.entries.get(session_id)?;
            if let Some(completion) = session.completion.clone() {
                return Some((session.status, completion));
            }
        }
        if terminal.changed().await.is_err() {
            return None;
        }
    }
}

fn terminal_outcome(
    plan: ToolExecutionPlan,
    session_id: &PtySessionId,
    status: PtySessionStatus,
    completion: PtySessionCompletion,
    already_terminal: bool,
) -> ToolExecutionOutcome {
    ToolExecutionOutcome::succeeded(
        plan,
        json!({
            "pty_session_id": session_id,
            "status": status,
            "completion": completion,
            "already_terminal": already_terminal,
        }),
    )
}

fn unknown_session(plan: ToolExecutionPlan, session_id: &PtySessionId) -> ToolExecutionOutcome {
    ToolExecutionOutcome::failed(
        plan,
        "pty_session_not_found",
        json!({"pty_session_id": session_id, "error": "PTY handle is unknown in this runtime; historical or other-runtime handles may be expired. No process was reattached."}),
    )
}

fn not_running(
    plan: ToolExecutionPlan,
    session_id: &PtySessionId,
    status: PtySessionStatus,
) -> ToolExecutionOutcome {
    ToolExecutionOutcome::failed(
        plan,
        "pty_session_not_running",
        json!({"pty_session_id": session_id, "status": status, "error": "PTY session is not accepting controls"}),
    )
}

fn validate_session_id(
    tool_name: &str,
    session_id: &PtySessionId,
) -> Result<(), ToolExecutorError> {
    if session_id.as_str().is_empty() {
        return Err(invalid_arguments(
            tool_name,
            "`pty_session_id` must not be empty",
        ));
    }
    Ok(())
}

fn validate_size(arguments: &ResizePtyArgs) -> Result<(), ToolExecutorError> {
    if arguments.rows == 0 || arguments.cols == 0 {
        return Err(invalid_arguments(
            "resize_pty",
            "`rows` and `cols` must be greater than zero",
        ));
    }
    Ok(())
}

fn offset_error_value(error: OffsetError) -> Value {
    json!({
        "error": error.reason,
        "reason": error.reason,
        "requested_offset": error.requested,
        "available_offset": error.available,
    })
}
