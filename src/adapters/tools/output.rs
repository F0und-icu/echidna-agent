//! Built-in Tool output policies and captured byte-offset views.

use std::sync::{Arc, Mutex, MutexGuard};

use serde_json::{Value, json};

pub(super) struct BoundedText {
    pub text: String,
    pub original_bytes: usize,
    pub truncated: bool,
}

pub(super) fn bounded_text(bytes: &[u8], max_bytes: usize) -> BoundedText {
    let original_bytes = bytes.len();
    let mut text = String::from_utf8_lossy(bytes).into_owned();
    let truncated = text.len() > max_bytes;
    if truncated {
        let mut end = max_bytes.min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    BoundedText {
        text,
        original_bytes,
        truncated,
    }
}

/// Bounded process output with one explicit caller-offset interpretation.
/// Jobs and PTY sessions share this byte-first owner so UTF-8 and arbitrary
/// binary streams cannot diverge in their incremental poll behavior.
pub(super) struct CapturedOutput {
    bytes: Vec<u8>,
    limit: usize,
    truncated: bool,
    read_error: Option<String>,
    sealed: bool,
}

impl CapturedOutput {
    pub(super) fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
            truncated: false,
            read_error: None,
            sealed: false,
        }
    }

    pub(super) fn append(&mut self, bytes: &[u8]) {
        if self.sealed {
            return;
        }
        let available = self.limit.saturating_sub(self.bytes.len());
        let captured = available.min(bytes.len());
        self.bytes.extend_from_slice(&bytes[..captured]);
        self.truncated |= captured < bytes.len();
    }

    pub(super) fn set_read_error(&mut self, error: impl Into<String>) {
        if !self.sealed {
            self.read_error = Some(error.into());
        }
    }

    /// Freezes the terminal byte view. A detached reader may still own an OS
    /// handle, but it can no longer mutate output after terminal publication.
    pub(super) fn seal(&mut self) {
        self.sealed = true;
    }

    /// Makes a bounded capture failure visible to callers. The underlying
    /// reader has already been stopped or detached before this is recorded.
    pub(super) fn mark_incomplete(&mut self, reason: impl Into<String>) {
        if !self.sealed {
            self.truncated = true;
            self.read_error = Some(reason.into());
            self.sealed = true;
        }
    }

    pub(super) fn slice_after_offset(&self, offset: u64) -> Result<Value, OffsetError> {
        let offset = usize::try_from(offset).map_err(|_| OffsetError {
            requested: offset,
            available: self.bytes.len(),
            reason: "offset_out_of_range",
        })?;
        let utf8_prefix = self.valid_utf8_prefix();
        let available = utf8_prefix.unwrap_or(self.bytes.len());
        if offset > available {
            return Err(OffsetError {
                requested: offset as u64,
                available,
                reason: "offset_out_of_range",
            });
        }
        if let Some(valid_end) = utf8_prefix
            && offset < valid_end
            && let Ok(text) = std::str::from_utf8(&self.bytes[..valid_end])
            && !text.is_char_boundary(offset)
        {
            return Err(OffsetError {
                requested: offset as u64,
                available,
                reason: "offset_not_utf8_boundary",
            });
        }
        Ok(json!({
            "offset": offset,
            "next_offset": available,
            "data": String::from_utf8_lossy(&self.bytes[offset..available]),
            "truncated": self.truncated,
            "read_error": self.read_error,
        }))
    }

    fn valid_utf8_prefix(&self) -> Option<usize> {
        match std::str::from_utf8(&self.bytes) {
            Ok(_) => Some(self.bytes.len()),
            Err(error) if error.error_len().is_none() => Some(error.valid_up_to()),
            Err(_) => None,
        }
    }
}

pub(super) struct OffsetError {
    pub(super) requested: u64,
    pub(super) available: usize,
    pub(super) reason: &'static str,
}

pub(super) fn lock_captured_output(
    output: &Arc<Mutex<CapturedOutput>>,
) -> MutexGuard<'_, CapturedOutput> {
    output.lock().unwrap_or_else(|error| error.into_inner())
}
