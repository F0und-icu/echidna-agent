//! JSONL Event Store implementation split by byte-scanning and recovery work.

mod recovery;
mod scan;

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::marker::PhantomData;
use std::path::PathBuf;

use serde::Serialize;
use serde_json::Value;

use crate::domain::events::{
    EventEnvelope, EventError, EventId, EventSchema, EventSequence, EventType,
    EventUpcasterRegistry, OccurredAt, StreamId,
};
use crate::ports::{EventSink, EventSource};

use super::ProjectEventStoreError;
use super::project::{ProjectEventStream, ProjectWriter, create_directory};
use recovery::{RecoveryArtifact, persist_recovery_artifact, replace_stream_atomically};
use scan::{StreamScan, next_sequence, scan_stream};

pub(crate) use scan::scan_stream_bytes;

/// Explicit caller-provided facts for a possible `StreamRecovered` Event.
///
/// Recovering a tail is only permitted when these identity and time values are
/// supplied. Domain obtains neither a clock nor an ID while handling Events.
#[derive(Clone, Debug)]
pub struct RecoveryMetadata {
    /// Event identifier used only if a corrupt final record is recovered.
    pub event_id: EventId,
    /// Externally supplied occurrence time used only when recovery occurs.
    pub occurred_at: OccurredAt,
}

impl RecoveryMetadata {
    /// Creates recovery Event metadata from explicit values.
    pub fn new(event_id: EventId, occurred_at: OccurredAt) -> Self {
        Self {
            event_id,
            occurred_at,
        }
    }
}

impl ProjectWriter {
    /// Opens the project Research JSONL Writer.
    pub fn open_research(
        &mut self,
        recovery: Option<RecoveryMetadata>,
    ) -> Result<JsonlEventStore<'_>, ProjectEventStoreError> {
        self.open_stream(ProjectEventStream::Research, recovery)
    }

    /// Opens one project Session JSONL Writer.
    pub fn open_session(
        &mut self,
        session_id: StreamId,
        recovery: Option<RecoveryMetadata>,
    ) -> Result<JsonlEventStore<'_>, ProjectEventStoreError> {
        self.open_stream(ProjectEventStream::Session(session_id), recovery)
    }

    fn open_stream(
        &mut self,
        stream: ProjectEventStream,
        recovery: Option<RecoveryMetadata>,
    ) -> Result<JsonlEventStore<'_>, ProjectEventStoreError> {
        let path = self.project.stream_path(&stream)?;
        if let Some(parent) = path.parent() {
            create_directory(parent)?;
        }
        if !path.exists() {
            create_empty_stream(&path)?;
        }

        let stream_id = stream.stream_id();
        let registry = EventUpcasterRegistry::current();
        let scan = scan_stream(&path, &stream_id, &registry)?;
        let last_sequence = match scan {
            StreamScan::Complete { last_sequence, .. } => last_sequence,
            StreamScan::Trailing {
                last_sequence,
                offset,
                valid_prefix,
                raw_tail,
            } => {
                let metadata =
                    recovery.ok_or_else(|| ProjectEventStoreError::RecoveryMetadataRequired {
                        path: path.clone(),
                        offset,
                    })?;
                let next_sequence = next_sequence(last_sequence, &stream_id)?;
                let artifact = RecoveryArtifact::for_tail(&self.project, &raw_tail);
                let recovered = EventEnvelope::current(
                    metadata.event_id,
                    stream_id.clone(),
                    next_sequence,
                    metadata.occurred_at,
                    EventType::new("StreamRecovered"),
                    serde_json::to_value(RecoveryPayload {
                        recovery_artifact: artifact.reference().to_owned(),
                        truncated_from_offset: offset,
                        resumed_after_sequence: last_sequence.map(EventSequence::get),
                    })
                    .map_err(ProjectEventStoreError::RecoveryPayloadEncode)?,
                );
                let mut encoded =
                    serde_json::to_vec(&recovered).map_err(ProjectEventStoreError::EventEncode)?;
                encoded.push(b'\n');

                persist_recovery_artifact(&artifact, &raw_tail)?;
                replace_stream_atomically(&path, &valid_prefix, &encoded)?;
                return JsonlEventStore::open(
                    path,
                    stream_id,
                    Some(recovered.sequence),
                    registry,
                    PhantomData,
                );
            }
        };

        JsonlEventStore::open(path, stream_id, last_sequence, registry, PhantomData)
    }
}

/// One serialized Event Store borrowed from the project-level OS writer lock.
pub struct JsonlEventStore<'writer> {
    path: PathBuf,
    stream_id: StreamId,
    file: File,
    last_sequence: Option<EventSequence>,
    registry: EventUpcasterRegistry<Value>,
    _writer: PhantomData<&'writer mut ProjectWriter>,
}

impl<'writer> JsonlEventStore<'writer> {
    fn open(
        path: PathBuf,
        stream_id: StreamId,
        last_sequence: Option<EventSequence>,
        registry: EventUpcasterRegistry<Value>,
        writer: PhantomData<&'writer mut ProjectWriter>,
    ) -> Result<Self, ProjectEventStoreError> {
        let file = OpenOptions::new()
            .append(true)
            .read(true)
            .open(&path)
            .map_err(|source| ProjectEventStoreError::Io {
                operation: "open Event stream for append",
                path: path.clone(),
                source,
            })?;
        Ok(Self {
            path,
            stream_id,
            file,
            last_sequence,
            registry,
            _writer: writer,
        })
    }

    /// Returns the exact next sequence assigned by this stream Writer.
    pub fn next_sequence(&self) -> Result<EventSequence, ProjectEventStoreError> {
        next_sequence(self.last_sequence, &self.stream_id)
    }
}

impl EventSource<Value> for JsonlEventStore<'_> {
    type Error = ProjectEventStoreError;

    fn read_events(&self, stream_id: &StreamId) -> Result<Vec<EventEnvelope<Value>>, Self::Error> {
        ensure_stream_matches(&self.stream_id, stream_id)?;
        scan_stream(&self.path, &self.stream_id, &self.registry)?.into_complete(&self.path)
    }
}

impl EventSink<Value> for JsonlEventStore<'_> {
    type Error = ProjectEventStoreError;

    fn append_event(&mut self, event: &EventEnvelope<Value>) -> Result<(), Self::Error> {
        ensure_stream_matches(&self.stream_id, &event.stream_id)?;
        let schema = EventSchema::from_envelope(event);
        if schema != *self.registry.current_schema() {
            return Err(EventError::UnsupportedSchema {
                schema_family: schema.family,
                schema_version: schema.version,
            }
            .into());
        }

        let expected = next_sequence(self.last_sequence, &self.stream_id)?;
        if event.sequence != expected {
            return Err(EventError::UnexpectedSequence {
                stream_id: self.stream_id.clone(),
                expected,
                received: event.sequence,
            }
            .into());
        }

        let mut encoded = serde_json::to_vec(event).map_err(ProjectEventStoreError::EventEncode)?;
        encoded.push(b'\n');
        self.file
            .write_all(&encoded)
            .map_err(|source| ProjectEventStoreError::Io {
                operation: "append Event record",
                path: self.path.clone(),
                source,
            })?;
        self.file
            .sync_data()
            .map_err(|source| ProjectEventStoreError::Io {
                operation: "sync appended Event record",
                path: self.path.clone(),
                source,
            })?;
        self.last_sequence = Some(event.sequence);
        Ok(())
    }
}

#[derive(Serialize)]
struct RecoveryPayload {
    recovery_artifact: String,
    truncated_from_offset: u64,
    resumed_after_sequence: Option<u64>,
}

fn create_empty_stream(path: &std::path::Path) -> Result<(), ProjectEventStoreError> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .and_then(|file| file.sync_all())
        .map_err(|source| ProjectEventStoreError::Io {
            operation: "create Session Event stream",
            path: path.to_path_buf(),
            source,
        })
}

fn ensure_stream_matches(
    expected: &StreamId,
    received: &StreamId,
) -> Result<(), ProjectEventStoreError> {
    if expected == received {
        Ok(())
    } else {
        Err(ProjectEventStoreError::StreamIdMismatch {
            expected: expected.clone(),
            received: received.clone(),
        })
    }
}
