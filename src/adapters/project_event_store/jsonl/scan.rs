use std::fs::File;
use std::io::{BufRead, BufReader, Cursor, Read, Seek, SeekFrom};
use std::path::Path;

use serde_json::Value;

use crate::domain::events::{
    EventEnvelope, EventError, EventSequence, EventUpcasterRegistry, StreamId,
};

use super::super::ProjectEventStoreError;

pub(super) type SessionRecordObserver<'a> = dyn FnMut(
        &EventEnvelope<Value>,
        usize,
        usize,
        blake3::Hash,
        &[u8],
    ) -> Result<(), ProjectEventStoreError>
    + 'a;

pub(crate) enum StreamScan {
    Complete {
        events: Vec<EventEnvelope<Value>>,
        last_sequence: Option<EventSequence>,
        validated_bytes: usize,
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

pub(crate) fn scan_stream(
    path: &Path,
    expected_stream_id: &StreamId,
    registry: &EventUpcasterRegistry<Value>,
) -> Result<StreamScan, ProjectEventStoreError> {
    let file = File::open(path).map_err(|source| read_error(path, source))?;
    scan_stream_reader(
        &mut BufReader::new(file),
        path,
        expected_stream_id,
        registry,
        0,
        None,
        &mut blake3::Hasher::new(),
    )
}

pub(crate) fn scan_stream_bytes(
    bytes: &[u8],
    path: &Path,
    expected_stream_id: &StreamId,
    registry: &EventUpcasterRegistry<Value>,
) -> Result<StreamScan, ProjectEventStoreError> {
    scan_stream_reader(
        &mut Cursor::new(bytes),
        path,
        expected_stream_id,
        registry,
        0,
        None,
        &mut blake3::Hasher::new(),
    )
}

/// One record scanner serves full files, byte fixtures and verified suffixes.
/// The reader starts after the exact prefix represented by offset/hash/sequence.
pub(super) fn scan_stream_reader<Reader: BufRead + Seek>(
    reader: &mut Reader,
    path: &Path,
    expected_stream_id: &StreamId,
    registry: &EventUpcasterRegistry<Value>,
    record_start: usize,
    last_sequence: Option<EventSequence>,
    validated_hash: &mut blake3::Hasher,
) -> Result<StreamScan, ProjectEventStoreError> {
    scan_stream_reader_observed(
        reader,
        path,
        expected_stream_id,
        registry,
        record_start,
        last_sequence,
        validated_hash,
        true,
        None,
    )
}

/// Stream validated records to a derived index without accumulating another
/// full Event vector. The caller decides how to bind the index to its source.
#[allow(clippy::too_many_arguments)]
pub(super) fn scan_stream_reader_observed<Reader: BufRead + Seek>(
    reader: &mut Reader,
    path: &Path,
    expected_stream_id: &StreamId,
    registry: &EventUpcasterRegistry<Value>,
    mut record_start: usize,
    mut last_sequence: Option<EventSequence>,
    validated_hash: &mut blake3::Hasher,
    retain_events: bool,
    mut observer: Option<&mut SessionRecordObserver<'_>>,
) -> Result<StreamScan, ProjectEventStoreError> {
    let mut events = Vec::new();
    let mut record = Vec::new();
    loop {
        record.clear();
        let count = reader
            .read_until(b'\n', &mut record)
            .map_err(|source| read_error(path, source))?;
        if count == 0 {
            return Ok(StreamScan::Complete {
                events,
                last_sequence,
                validated_bytes: record_start,
            });
        }
        let terminated = record.last() == Some(&b'\n');
        let body = if terminated {
            &record[..record.len() - 1]
        } else {
            &record
        };
        match decode_record(body, expected_stream_id, last_sequence, registry) {
            Ok(event) if terminated => {
                last_sequence = Some(event.sequence);
                if let Some(observer) = observer.as_mut() {
                    observer(
                        &event,
                        record_start,
                        record.len(),
                        blake3::hash(&record),
                        &record,
                    )?;
                }
                if retain_events {
                    events.push(event);
                }
                validated_hash.update(&record);
                record_start += record.len();
            }
            result => {
                let problem = match result {
                    Ok(_) => None,
                    Err(RecordProblem::Lexical(reason)) => Some((true, reason)),
                    Err(RecordProblem::Semantic(reason)) => Some((false, reason)),
                };
                let final_record = !terminated
                    || reader
                        .fill_buf()
                        .map_err(|source| read_error(path, source))?
                        .is_empty();
                if final_record && problem.as_ref().is_none_or(|(lexical, _)| *lexical) {
                    // Recovery needs the original prefix, only on a real tail.
                    // A reread must match the bytes just validated, including
                    // when the underlying file changed outside the writer lease.
                    reader
                        .seek(SeekFrom::Start(0))
                        .map_err(|source| read_error(path, source))?;
                    let mut valid_prefix = Vec::new();
                    reader
                        .take(record_start as u64)
                        .read_to_end(&mut valid_prefix)
                        .map_err(|source| read_error(path, source))?;
                    if valid_prefix.len() != record_start
                        || blake3::hash(&valid_prefix) != validated_hash.finalize()
                    {
                        return Err(ProjectEventStoreError::ValidatedPrefixChanged {
                            path: path.to_path_buf(),
                        });
                    }
                    return Ok(StreamScan::Trailing {
                        last_sequence,
                        offset: record_start as u64,
                        valid_prefix,
                        raw_tail: record,
                    });
                }
                let (lexical, reason) = problem.expect("a complete record failed decoding");
                return Err(if lexical {
                    ProjectEventStoreError::NonTailCorruption {
                        path: path.to_path_buf(),
                        offset: record_start as u64,
                        reason,
                    }
                } else {
                    ProjectEventStoreError::InvalidEventRecord {
                        path: path.to_path_buf(),
                        offset: record_start as u64,
                        reason,
                    }
                });
            }
        }
    }
}

fn read_error(path: &Path, source: std::io::Error) -> ProjectEventStoreError {
    ProjectEventStoreError::Io {
        operation: "read Event stream",
        path: path.to_path_buf(),
        source,
    }
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
