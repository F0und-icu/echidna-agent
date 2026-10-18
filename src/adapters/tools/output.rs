//! Built-in Tool output policies and captured byte-offset views.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use crate::domain::tools::{OutputStream, PendingOutputCapture};
use serde_json::{Value, json};

pub(super) const OUTPUT_DRAIN_GRACE: Duration = Duration::from_millis(100);

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
    observed_bytes: u64,
    eof: bool,
}

impl CapturedOutput {
    pub(super) fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
            truncated: false,
            read_error: None,
            sealed: false,
            observed_bytes: 0,
            eof: false,
        }
    }

    pub(super) fn append(&mut self, bytes: &[u8]) {
        if self.sealed {
            return;
        }
        self.observed_bytes = self.observed_bytes.saturating_add(bytes.len() as u64);
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
        self.eof = self.read_error.is_none();
    }

    pub(super) fn capture(
        &self,
        stream: OutputStream,
        child_index: Option<u8>,
    ) -> PendingOutputCapture {
        PendingOutputCapture {
            stream,
            child_index,
            bytes: self.bytes.clone(),
            observed_bytes: self.observed_bytes,
            eof: self.eof,
            read_error: self.read_error.clone(),
        }
    }

    /// The tail is from the retained prefix, never from beyond the raw ceiling.
    pub(super) fn preview(&self, limit: usize) -> BoundedText {
        let mut view = bounded_text(&self.bytes, limit);
        if view.truncated && limit >= 64 {
            let marker = "\n[...retained prefix...]\n";
            let side = ((limit - marker.len()) / 2).min(self.bytes.len());
            let tail = String::from_utf8_lossy(&self.bytes[self.bytes.len() - side..]);
            let mut start = tail.len().saturating_sub(side);
            while !tail.is_char_boundary(start) {
                start += 1;
            }
            view.text = format!(
                "{}{}{}",
                bounded_text(&self.bytes[..side], side).text,
                marker,
                &tail[start..]
            );
        }
        view.original_bytes = usize::try_from(self.observed_bytes).unwrap_or(usize::MAX);
        view.truncated |= self.truncated || self.observed_bytes > limit as u64;
        view
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

    pub(super) fn slice_after_offset(
        &self,
        offset: u64,
        max_bytes: usize,
    ) -> Result<Value, OffsetError> {
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
        let mut end = available.min(offset.saturating_add(max_bytes));
        if let Some(valid_end) = utf8_prefix {
            let text =
                std::str::from_utf8(&self.bytes[..valid_end]).expect("validated UTF8 prefix");
            while end > offset && !text.is_char_boundary(end) {
                end -= 1;
            }
        }
        let data = bounded_text(&self.bytes[offset..end], max_bytes);
        Ok(json!({
            "offset": offset,
            "next_offset": end,
            "data": data.text,
            "truncated": self.truncated || end < available || data.truncated,
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
