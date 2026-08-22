use std::path::Path;

use serde_json::Value;

use crate::domain::events::{
    EventEnvelope, EventError, EventSequence, EventUpcasterRegistry, StreamId,
};

use super::super::ProjectEventStoreError;
use super::super::project::read_bytes;

pub(crate) enum StreamScan {
    Complete {
        events: Vec<EventEnvelope<Value>>,
        last_sequence: Option<EventSequence>,
    },
    Trailing {
        last_sequence: Option<EventSequence>,
        offset: u64,
        valid_prefix: Vec<u8>,
        raw_tail: Vec<u8>,
    },
}

impl StreamScan {
    pub(crate) fn into_complete(
        self,
        path: &Path,
    ) -> Result<Vec<EventEnvelope<Value>>, ProjectEventStoreError> {
        match self {
            Self::Complete { events, .. } => Ok(events),
            Self::Trailing { offset, .. } => Err(ProjectEventStoreError::UnrecoveredTail {
                path: path.to_path_buf(),
                offset,
            }),
        }
    }
}

/// The sole byte-level JSONL scanner. Only a lexical failure at the final
/// record boundary is recoverable; every semantic failure is read-only.
pub(crate) fn scan_stream(
    path: &Path,
    expected_stream_id: &StreamId,
    registry: &EventUpcasterRegistry<Value>,
) -> Result<StreamScan, ProjectEventStoreError> {
    let bytes = read_bytes(path, "read Event stream")?;
    scan_stream_bytes(&bytes, path, expected_stream_id, registry)
}

pub(crate) fn scan_stream_bytes(
    bytes: &[u8],
    path: &Path,
    expected_stream_id: &StreamId,
    registry: &EventUpcasterRegistry<Value>,
) -> Result<StreamScan, ProjectEventStoreError> {
    let mut events = Vec::new();
    let mut last_sequence = None;
    let mut record_start = 0_usize;

    for (index, byte) in bytes.iter().enumerate() {
        if *byte != b'\n' {
            continue;
        }

        let record = &bytes[record_start..index];
        let final_record = index + 1 == bytes.len();
        match decode_record(record, expected_stream_id, last_sequence, registry) {
            Ok(event) => {
                last_sequence = Some(event.sequence);
                events.push(event);
            }
            Err(RecordProblem::Lexical(_reason)) if final_record => {
                return Ok(StreamScan::Trailing {
                    last_sequence,
                    offset: record_start as u64,
                    valid_prefix: bytes[..record_start].to_vec(),
                    raw_tail: bytes[record_start..].to_vec(),
                });
            }
            Err(RecordProblem::Lexical(reason)) => {
                return Err(ProjectEventStoreError::NonTailCorruption {
                    path: path.to_path_buf(),
                    offset: record_start as u64,
                    reason,
                });
            }
            Err(RecordProblem::Semantic(reason)) => {
                return Err(ProjectEventStoreError::InvalidEventRecord {
                    path: path.to_path_buf(),
                    offset: record_start as u64,
                    reason,
                });
            }
        }
        record_start = index + 1;
    }

    if record_start < bytes.len() {
        let record = &bytes[record_start..];
        return match decode_record(record, expected_stream_id, last_sequence, registry) {
            Ok(_) | Err(RecordProblem::Lexical(_)) => Ok(StreamScan::Trailing {
                last_sequence,
                offset: record_start as u64,
                valid_prefix: bytes[..record_start].to_vec(),
                raw_tail: record.to_vec(),
            }),
            Err(problem) => Err(ProjectEventStoreError::InvalidEventRecord {
                path: path.to_path_buf(),
                offset: record_start as u64,
                reason: problem.into_reason(),
            }),
        };
    }

    Ok(StreamScan::Complete {
        events,
        last_sequence,
    })
}

fn decode_record(
    record: &[u8],
    expected_stream_id: &StreamId,
    last_sequence: Option<EventSequence>,
    registry: &EventUpcasterRegistry<Value>,
) -> Result<EventEnvelope<Value>, RecordProblem> {
    if record.is_empty() {
        return Err(RecordProblem::Lexical("empty JSONL record".to_owned()));
    }
    let event: EventEnvelope<Value> = serde_json::from_slice(record)
        .map_err(|error| RecordProblem::Lexical(format!("invalid JSON: {error}")))?;
    if event.stream_id != *expected_stream_id {
        return Err(RecordProblem::Semantic(format!(
            "stream ID `{}` does not match expected `{expected_stream_id}`",
            event.stream_id
        )));
    }
    let expected_sequence = next_sequence(last_sequence, expected_stream_id)
        .map_err(|error| RecordProblem::Semantic(error.to_string()))?;
    if event.sequence != expected_sequence {
        return Err(RecordProblem::Semantic(format!(
            "sequence `{}` does not equal expected next sequence `{expected_sequence}`",
            event.sequence
        )));
    }
    registry
        .upcast(event)
        .map_err(|error| RecordProblem::Semantic(error.to_string()))
}

enum RecordProblem {
    Lexical(String),
    Semantic(String),
}

impl RecordProblem {
    fn into_reason(self) -> String {
        match self {
            Self::Lexical(reason) | Self::Semantic(reason) => reason,
        }
    }
}

pub(crate) fn next_sequence(
    last_sequence: Option<EventSequence>,
    stream_id: &StreamId,
) -> Result<EventSequence, ProjectEventStoreError> {
    let last = last_sequence.unwrap_or(EventSequence::new(0));
    let next = last
        .get()
        .checked_add(1)
        .ok_or_else(|| EventError::SequenceOverflow {
            stream_id: stream_id.clone(),
            last,
        })?;
    Ok(EventSequence::new(next))
}
