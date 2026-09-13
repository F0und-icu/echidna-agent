//! Byte-capture identity, bounds and Receipt/Result agreement; no I/O.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{ExecutionReceipt, ToolContractError, ToolResult};
use crate::domain::artifacts::BlobId;

pub(crate) const MAX_CAPTURE_STREAM_BYTES: usize = 1024 * 1024;
const MAX_CAPTURE_EXECUTION_BYTES: usize = 8 * 1024 * 1024;

pub(crate) fn batch_capture_stream_limit(child_count: usize) -> usize {
    MAX_CAPTURE_STREAM_BYTES.min(MAX_CAPTURE_EXECUTION_BYTES / (2 * child_count.max(1)))
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputStream {
    Stdout,
    Stderr,
    Pty,
}

/// Immutable retained prefix, not an ArtifactRecord or verified Finding.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OutputCapture {
    pub stream: OutputStream,
    pub child_index: Option<u8>,
    pub blob_id: BlobId,
    pub observed_bytes: u64,
    pub retained_bytes: u64,
    pub eof: bool,
    pub complete: bool,
    pub read_error: Option<String>,
}

/// Transient execution bytes. Never serialized into a Receipt or History.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PendingOutputCapture {
    pub stream: OutputStream,
    pub child_index: Option<u8>,
    pub bytes: Vec<u8>,
    pub observed_bytes: u64,
    pub eof: bool,
    pub read_error: Option<String>,
}

impl PendingOutputCapture {
    pub(crate) fn published(&self, blob_id: BlobId) -> OutputCapture {
        OutputCapture {
            stream: self.stream,
            child_index: self.child_index,
            blob_id,
            observed_bytes: self.observed_bytes,
            retained_bytes: self.bytes.len() as u64,
            eof: self.eof,
            complete: self.eof
                && self.read_error.is_none()
                && self.observed_bytes == self.bytes.len() as u64,
            read_error: self.read_error.clone(),
        }
    }
}

pub(crate) fn validate_captures(captures: &[OutputCapture]) -> Result<(), ToolContractError> {
    let mut total = 0u64;
    if captures.len() > 64 {
        return Err(invalid_capture());
    }
    for (index, capture) in captures.iter().enumerate() {
        total = total
            .checked_add(capture.retained_bytes)
            .ok_or_else(invalid_capture)?;
        if capture.retained_bytes > MAX_CAPTURE_STREAM_BYTES as u64
            || capture.retained_bytes > capture.observed_bytes
            || capture.child_index.is_some_and(|index| index >= 32)
            || capture.complete
                != (capture.eof
                    && capture.read_error.is_none()
                    && capture.retained_bytes == capture.observed_bytes)
            || captures[..index].iter().any(|other| {
                other.stream == capture.stream && other.child_index == capture.child_index
            })
        {
            return Err(invalid_capture());
        }
    }
    if total > MAX_CAPTURE_EXECUTION_BYTES as u64 {
        return Err(invalid_capture());
    }
    Ok(())
}

impl ExecutionReceipt {
    pub(crate) fn validate_output_captures(
        &self,
        result: &ToolResult,
    ) -> Result<(), ToolContractError> {
        validate_captures(&self.output_captures)?;
        let captures: Vec<OutputCapture> = match result.output.get("output_captures") {
            Some(value) => serde_json::from_value(value.clone()).map_err(|_| invalid_capture())?,
            None => Vec::new(),
        };
        if captures != self.output_captures {
            return Err(invalid_capture());
        }
        Ok(())
    }
}

pub(crate) fn capture_schema() -> Value {
    serde_json::json!({"type":"array", "maxItems":64, "items": {
        "type":"object", "properties": {
            "stream":{"type":"string", "enum":["stdout","stderr","pty"]},
            "child_index":{"type":["integer","null"],"minimum":0,"maximum":31},
            "blob_id":{"type":"string","pattern":"^[0-9a-f]{64}$"},
            "observed_bytes":{"type":"integer","minimum":0},
            "retained_bytes":{"type":"integer","minimum":0,"maximum":MAX_CAPTURE_STREAM_BYTES},
            "eof":{"type":"boolean"},"complete":{"type":"boolean"},
            "read_error":{"type":["string","null"]}
        },"required":["stream","child_index","blob_id","observed_bytes","retained_bytes","eof","complete","read_error"],
        "additionalProperties":false
    }})
}

fn invalid_capture() -> ToolContractError {
    ToolContractError::InvalidOutputCapture
}
