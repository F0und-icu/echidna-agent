//! Bounded byte capture and explicit-offset views for ordinary Jobs.

use serde_json::{Value, json};

pub(super) struct JobOutput {
    bytes: Vec<u8>,
    limit: usize,
    truncated: bool,
    read_error: Option<String>,
}

impl JobOutput {
    pub(super) fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
            truncated: false,
            read_error: None,
        }
    }

    pub(super) fn append(&mut self, bytes: &[u8]) {
        let available = self.limit.saturating_sub(self.bytes.len());
        let captured = available.min(bytes.len());
        self.bytes.extend_from_slice(&bytes[..captured]);
        self.truncated |= captured < bytes.len();
    }

    pub(super) fn set_read_error(&mut self, error: impl Into<String>) {
        self.read_error = Some(error.into());
    }

    /// Makes a bounded capture failure visible to callers. The underlying
    /// reader has already been aborted and awaited before this is recorded.
    pub(super) fn mark_incomplete(&mut self, reason: impl Into<String>) {
        self.truncated = true;
        self.set_read_error(reason);
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
            // Job output is bytes first. Lossy conversion is explicit at this
            // display boundary so arbitrary command output remains observable.
            "data": String::from_utf8_lossy(&self.bytes[offset..available]),
            "truncated": self.truncated,
            "read_error": self.read_error,
        }))
    }

    /// Return a byte offset that is safe for a caller to resume at. A reader
    /// can stop between bytes of a valid UTF-8 code point, so an incomplete
    /// trailing sequence is withheld until its continuation arrives (or the
    /// stream is marked truncated).
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
