//! Scoped read selection and bounded presentation over validated source facts.

use std::io::{self, Write};

use crate::ports::context_read::{
    ContextReadEntry, ContextReadError, ContextReadRequest, ContextReadResponse, ContextReadStatus,
};

pub(crate) struct SourceRangeWriter {
    pub(crate) hash: blake3::Hasher,
    pub(crate) offset: usize,
    pub(crate) end: usize,
    pub(crate) position: usize,
    pub(crate) bytes: Vec<u8>,
    pub(crate) stop_at_end: bool,
    pub(crate) stopped: bool,
}

impl Write for SourceRangeWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let end = self.position.saturating_add(bytes.len());
        let consumed = if self.stop_at_end && end > self.end {
            self.end.saturating_sub(self.position)
        } else {
            bytes.len()
        };
        let bytes = &bytes[..consumed];
        self.hash.update(bytes);
        let end = self.position.saturating_add(bytes.len());
        if self.offset < end && self.end > self.position {
            let start = self.offset.saturating_sub(self.position);
            let stop = self.end.saturating_sub(self.position).min(bytes.len());
            self.bytes.extend_from_slice(&bytes[start..stop]);
        }
        self.position = end;
        if self.stop_at_end && consumed == 0 {
            self.stopped = true;
            return Err(io::Error::other("bounded source prefix complete"));
        }
        Ok(consumed)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Preserve the exact JSON prefix used for source-range offsets without
/// materializing an entire large Tool item.
pub(crate) fn history_navigation_excerpt(
    item: &crate::domain::model::ContentItem,
) -> Result<(String, bool), crate::domain::session::SessionError> {
    let mut writer = SourceRangeWriter {
        hash: blake3::Hasher::new(),
        offset: 0,
        end: 1024,
        position: 0,
        bytes: Vec::new(),
        stop_at_end: true,
        stopped: false,
    };
    let result = serde_json::to_writer(&mut writer, item);
    if result.is_err() && !writer.stopped {
        return Err(crate::domain::session::SessionError::TaskContext {
            message: "History source could not be encoded".to_owned(),
        });
    }
    while std::str::from_utf8(&writer.bytes).is_err() {
        writer.bytes.pop();
    }
    let text = String::from_utf8(writer.bytes).expect("validated UTF-8 source prefix");
    Ok((text, writer.stopped))
}

pub(crate) struct ContextSource {
    pub(crate) reference: String,
    pub(crate) scope: &'static str,
    pub(crate) revision: String,
    pub(crate) role: Option<crate::domain::model::MessageRole>,
    pub(crate) semantics: crate::domain::session::HistorySourceSemantics,
    pub(crate) observed_at: Option<String>,
    pub(crate) text: String,
    pub(crate) searchable: bool,
    pub(crate) available_offset: usize,
    pub(crate) total_bytes: usize,
}

pub(crate) struct ContextSources {
    pub(crate) history: Vec<ContextSource>,
    pub(crate) memory: Option<ContextSource>,
    pub(crate) progress: Option<ContextSource>,
    pub(crate) more: bool,
    pub(crate) ambiguous_time: bool,
}

pub(crate) fn read(
    request: ContextReadRequest,
    sources: ContextSources,
) -> Result<ContextReadResponse, ContextReadError> {
    match request {
        ContextReadRequest::Read {
            reference,
            offset,
            max_bytes,
        } => {
            if let Some(source) = sources
                .history
                .iter()
                .find(|source| source.reference == reference)
            {
                return Ok(response(vec![entry(source, offset, max_bytes)?], false));
            }
            if let Some((path, expected)) = reference
                .strip_prefix("memory:")
                .and_then(|value| value.rsplit_once(':'))
                && let Some(memory) = &sources.memory
                && memory.reference.starts_with(&format!("memory:{path}:"))
            {
                if expected != memory.revision {
                    return Ok(ContextReadResponse {
                        status: ContextReadStatus::Stale,
                        entries: Vec::new(),
                        truncated: false,
                        next_cursor: None,
                    });
                }
                return Ok(response(vec![entry(memory, offset, max_bytes)?], false));
            }
            if let Some((task, expected)) = reference
                .strip_prefix("progress:")
                .and_then(|value| value.rsplit_once(':'))
                && let Some(progress) = &sources.progress
                && progress.reference.starts_with(&format!("progress:{task}:"))
            {
                if expected != progress.revision {
                    return Ok(ContextReadResponse {
                        status: ContextReadStatus::Stale,
                        entries: Vec::new(),
                        truncated: false,
                        next_cursor: None,
                    });
                }
                return Ok(response(vec![entry(progress, offset, max_bytes)?], false));
            }
            Ok(response(Vec::new(), false))
        }
        ContextReadRequest::Progress { max_bytes, .. } => {
            let Some(progress) = &sources.progress else {
                return Ok(response(Vec::new(), false));
            };
            Ok(response(vec![entry(progress, 0, max_bytes)?], false))
        }
        ContextReadRequest::Search {
            query,
            after_unix,
            before_unix,
            max_bytes,
            ..
        } => {
            let _ = query;
            let mut found = Vec::new();
            let mut remaining = max_bytes;
            let mut ambiguous = sources.ambiguous_time;
            let mut truncated = sources.more;
            for source in sources.history.iter().chain(sources.memory.iter()) {
                if !source.searchable {
                    continue;
                }
                if after_unix.is_some() || before_unix.is_some() {
                    let observed = source.observed_at.as_deref().and_then(unix_seconds);
                    let Some(observed) = observed else {
                        ambiguous = true;
                        continue;
                    };
                    if after_unix.is_some_and(|after| observed < after)
                        || before_unix.is_some_and(|before| observed > before)
                    {
                        continue;
                    }
                }
                if remaining == 0 || found.len() == 10 {
                    truncated = true;
                    break;
                }
                let value = entry(source, source.available_offset, remaining)?;
                remaining -= value.text.len();
                found.push(value);
            }
            let mut result = response(found, truncated);
            if ambiguous {
                result.status = ContextReadStatus::Ambiguous;
            }
            Ok(result)
        }
    }
}

pub(crate) fn unix_seconds(value: &str) -> Option<u64> {
    if let Some(value) = value.strip_prefix("unix:") {
        return value.split('.').next()?.parse().ok();
    }
    let bytes = value.as_bytes();
    if bytes.len() < 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !matches!(bytes[10], b'T' | b't')
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return None;
    }
    let digits = |start: usize, length: usize| -> Option<i64> {
        bytes
            .get(start..start + length)?
            .iter()
            .try_fold(0_i64, |value, byte| {
                if byte.is_ascii_digit() {
                    Some(value * 10 + i64::from(*byte - b'0'))
                } else {
                    None
                }
            })
    };
    let year = digits(0, 4)?;
    let month = digits(5, 2)?;
    let day = digits(8, 2)?;
    let hour = digits(11, 2)?;
    let minute = digits(14, 2)?;
    let second = digits(17, 2)?;
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let month_days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    };
    if day < 1 || day > month_days || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let mut zone = 19;
    if bytes.get(zone) == Some(&b'.') {
        zone += 1;
        let start = zone;
        while bytes.get(zone).is_some_and(u8::is_ascii_digit) {
            zone += 1;
        }
        if zone == start {
            return None;
        }
    }
    let offset = match bytes.get(zone..) {
        Some([b'Z' | b'z']) => 0,
        Some([sign @ (b'+' | b'-'), h1, h2, b':', m1, m2])
            if [h1, h2, m1, m2].iter().all(|digit| digit.is_ascii_digit()) =>
        {
            let hours = i64::from((*h1 - b'0') * 10 + *h2 - b'0');
            let minutes = i64::from((*m1 - b'0') * 10 + *m2 - b'0');
            if hours > 23 || minutes > 59 {
                return None;
            }
            let offset = hours * 3600 + minutes * 60;
            if *sign == b'+' { offset } else { -offset }
        }
        _ => return None,
    };
    // Gregorian civil date to days since 1970-01-01, independent of host TZ.
    let adjusted_year = year - i64::from(month <= 2);
    let era = adjusted_year.div_euclid(400);
    let year_of_era = adjusted_year - era * 400;
    let adjusted_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * adjusted_month + 2) / 5 + day - 1;
    let days = era * 146_097 + year_of_era * 365 + year_of_era / 4 - year_of_era / 100
        + day_of_year
        - 719_468;
    u64::try_from(days * 86_400 + hour * 3600 + minute * 60 + second - offset).ok()
}

fn entry(
    source: &ContextSource,
    offset: usize,
    maximum: usize,
) -> Result<ContextReadEntry, ContextReadError> {
    let text = &source.text;
    let Some(start) = offset.checked_sub(source.available_offset) else {
        return Err(ContextReadError {
            message: "context read offset precedes the loaded source range".to_owned(),
        });
    };
    if offset > source.total_bytes || start > text.len() || !text.is_char_boundary(start) {
        return Err(ContextReadError {
            message: "context read offset is outside the source or not a UTF-8 boundary".to_owned(),
        });
    }
    let mut end = text.len().min(start.saturating_add(maximum));
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    if end == start && offset < source.total_bytes {
        return Err(ContextReadError {
            message: "context read range is too small for the next UTF-8 character".to_owned(),
        });
    }
    let absolute_end = source.available_offset + end;
    Ok(ContextReadEntry {
        reference: source.reference.clone(),
        scope: source.scope.to_owned(),
        source_revision: source.revision.clone(),
        source_role: source.role,
        source_origin: source.semantics.origin,
        semantic_class: source.semantics.class,
        source_authority: source.semantics.authority,
        observed_at: source.observed_at.clone(),
        valid_time: None,
        offset,
        next_offset: (absolute_end < source.total_bytes).then_some(absolute_end),
        total_bytes: source.total_bytes,
        text: text[start..end].to_owned(),
        truncated: absolute_end < source.total_bytes,
    })
}

fn response(entries: Vec<ContextReadEntry>, truncated: bool) -> ContextReadResponse {
    ContextReadResponse {
        status: if entries.is_empty() {
            ContextReadStatus::NotFound
        } else {
            ContextReadStatus::Found
        },
        entries,
        truncated,
        next_cursor: None,
    }
}
