//! Terminal rendering of configured MCP lifecycle and protocol observations.
use crate::adapters::mcp::observation::{McpObservation, McpObserver};
use crate::chat_display::{JsonLines, Log, LogWriter};
use crate::diagnostic_text::escaped_text;
use std::io::Write;
use std::sync::Arc;

pub(crate) fn observer() -> McpObserver {
    let stream = JsonLines::current();
    Arc::new(move |event| {
        let message = match event {
            McpObservation::Startup { server_id, stage } => {
                format!("[MCP {}: {stage}]", escaped_text(&server_id))
            }
            McpObservation::StartupFailed { server_id, stage } => format!(
                "[MCP {}: startup failed during {stage}]",
                escaped_text(&server_id)
            ),
            McpObservation::ToolExcluded {
                server_id,
                name,
                message,
            } => format!(
                "[MCP {} tool excluded: {}: {}]",
                escaped_text(&server_id),
                escaped_text(&name),
                escaped_text(&message)
            ),
            McpObservation::Notification { server_id, method } => format!(
                "[MCP {} transport notification: {}]",
                escaped_text(&server_id),
                escaped_text(&method)
            ),
            McpObservation::UnsupportedRequest { server_id, method } => format!(
                "[MCP {} unsupported server request: {}]",
                escaped_text(&server_id),
                escaped_text(&method)
            ),
            McpObservation::Cleanup {
                server_id,
                group_error,
                kill_error,
                wait_error,
                stderr_bytes,
                stderr_error,
            } => format!(
                "[MCP {} stdio cleanup: group_io={group_error:?} child_io={kill_error:?} wait_io={wait_error:?} stderr_bytes={stderr_bytes} stderr_io={stderr_error:?}]",
                escaped_text(&server_id)
            ),
        };
        let _ = writeln!(
            LogWriter {
                stream: stream.clone(),
                log: Log::Mcp,
                stderr: true
            },
            "{message}"
        );
    })
}
