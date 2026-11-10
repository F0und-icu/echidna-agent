//! JSONL Event Store implementation split by byte-scanning and recovery work.

mod recovery;
mod scan;

use std::fs::{File, OpenOptions};
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::marker::PhantomData;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::domain::events::{
    EventEnvelope, EventError, EventId, EventSchema, EventSequence, EventType,
    EventUpcasterRegistry, OccurredAt, StreamId,
};
use crate::ports::{EventSink, EventSource};

use super::ProjectEventStoreError;
use super::project::{Project, ProjectEventStream, ProjectWriter, create_directory};
use crate::adapters::file_revision::FileRevision;
use recovery::{RecoveryArtifact, persist_recovery_artifact, replace_stream_atomically};
use scan::{SessionRecordObserver, StreamScan, next_sequence, scan_stream};

pub(crate) use scan::scan_stream_bytes;

/// Process-local proof of the bytes already reduced by one Session consumer.
/// Unchanged files reuse verified bytes; external changes revalidate the prefix.
/// It is never a persisted authority or checkpoint.
#[derive(Clone)]
pub(crate) struct SessionStreamCursor {
    path: PathBuf,
    stream_id: StreamId,
    bytes: usize,
    hash: blake3::Hasher,
    last_sequence: Option<EventSequence>,
    revision: FileRevision,
}

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct SessionStreamCheckpoint {
    pub(crate) bytes: usize,
    pub(crate) hash: [u8; 32],
    pub(crate) last_sequence: Option<EventSequence>,
}

impl SessionStreamCursor {
    pub(crate) fn indexed_bytes(&self) -> usize {
        self.bytes
    }

    pub(crate) fn indexed_events(&self) -> u64 {
        self.last_sequence.map_or(0, |sequence| sequence.get())
    }

    pub(crate) fn checkpoint(&self) -> SessionStreamCheckpoint {
        SessionStreamCheckpoint {
            bytes: self.bytes,
            hash: *self.hash.finalize().as_bytes(),
            last_sequence: self.last_sequence,
        }
    }
    pub(crate) fn matches_watermark(&self, bytes: usize, hash: blake3::Hash) -> bool {
        self.bytes == bytes && self.hash.finalize() == hash
    }

    pub(crate) fn same_prefix(&self, other: &Self) -> bool {
        self.path == other.path
            && self.stream_id == other.stream_id
            && self.matches_watermark(other.bytes, other.hash.finalize())
    }
}

pub(crate) fn verify_session_checkpoint(
    project: &Project,
    session_id: &StreamId,
    checkpoint: &SessionStreamCheckpoint,
) -> Result<usize, ProjectEventStoreError> {
    let path = project.session_stream_path_existing(session_id)?;
    let file = File::open(&path).map_err(|source| ProjectEventStoreError::Io {
        operation: "read Session Event prefix for source verification",
        path: path.clone(),
        source,
    })?;
    verify_prefix_bytes(
        &mut BufReader::new(file),
        &path,
        checkpoint.bytes,
        blake3::Hash::from(checkpoint.hash),
    )?;
    Ok(checkpoint.bytes)
}

fn verify_session_prefix(
    reader: &mut BufReader<File>,
    path: &std::path::Path,
    stream_id: &StreamId,
    cursor: &SessionStreamCursor,
) -> Result<blake3::Hasher, ProjectEventStoreError> {
    if cursor.path != path || &cursor.stream_id != stream_id {
        return Err(ProjectEventStoreError::ValidatedPrefixChanged {
            path: path.to_path_buf(),
        });
    }
    let length = reader
        .get_ref()
        .metadata()
        .map_err(|source| ProjectEventStoreError::Io {
            operation: "inspect Session prefix length",
            path: path.to_path_buf(),
            source,
        })?
        .len();
    if length < cursor.bytes as u64 {
        return Err(ProjectEventStoreError::ValidatedPrefixChanged {
            path: path.to_path_buf(),
        });
    }
    // The live cursor tracks harness-owned append-only history. Only the new
    // suffix needs scanning; explicit historical audit still reads original bytes.
    reader
        .seek(SeekFrom::Start(cursor.bytes as u64))
        .map_err(|source| ProjectEventStoreError::Io {
            operation: "seek Session suffix",
            path: path.to_path_buf(),
            source,
        })?;
    Ok(cursor.hash.clone())
}

fn file_revision(
    file: &File,
    path: &std::path::Path,
) -> Result<FileRevision, ProjectEventStoreError> {
    file.metadata()
        .and_then(|metadata| FileRevision::of(&metadata))
        .map_err(|source| ProjectEventStoreError::Io {
            operation: "inspect Session file revision",
            path: path.to_path_buf(),
            source,
        })
}

fn verify_prefix_bytes(
    reader: &mut BufReader<File>,
    path: &std::path::Path,
    bytes: usize,
    expected: blake3::Hash,
) -> Result<blake3::Hasher, ProjectEventStoreError> {
    let mut remaining = bytes;
    let mut hash = blake3::Hasher::new();
    let mut chunk = [0; 65_536];
    while remaining > 0 {
        let capacity = remaining.min(chunk.len());
        let count =
            reader
                .read(&mut chunk[..capacity])
                .map_err(|source| ProjectEventStoreError::Io {
                    operation: "verify Session Event prefix",
                    path: path.to_path_buf(),
                    source,
                })?;
        if count == 0 {
            return Err(ProjectEventStoreError::ValidatedPrefixChanged {
                path: path.to_path_buf(),
            });
        }
        hash.update(&chunk[..count]);
        remaining -= count;
    }
    if hash.finalize() != expected {
        return Err(ProjectEventStoreError::ValidatedPrefixChanged {
            path: path.to_path_buf(),
        });
    }
    Ok(hash)
}

/// Reads the initialized Research stream without taking the writer lock or
/// opening a file for append. The shared scanner remains the only source of
/// JSONL validation and Event upcasting.
pub(crate) fn read_existing_research_events(
    project: &Project,
) -> Result<Vec<EventEnvelope<Value>>, ProjectEventStoreError> {
    let stream = ProjectEventStream::Research;
    let path = project.research_stream_path_existing()?;
    let stream_id = stream.stream_id();
    let registry = EventUpcasterRegistry::current();
    scan_stream(&path, &stream_id, &registry)?.into_complete(&path)
}

/// Reads an already-created Session stream without acquiring a Writer or
/// creating the stream. Cache validation treats every read failure as a miss.
pub(crate) fn read_existing_session_events(
    project: &Project,
    session_id: &StreamId,
) -> Result<Vec<EventEnvelope<Value>>, ProjectEventStoreError> {
    let stream = ProjectEventStream::Session(session_id.clone());
    let path = project.session_stream_path_existing(session_id)?;
    let stream_id = stream.stream_id();
    let registry = EventUpcasterRegistry::current();
    scan_stream(&path, &stream_id, &registry)?.into_complete(&path)
}

/// Visit each exact validated Session record and its original byte range for a
/// disposable source index. No derived index is an authority for the stream.
pub(crate) fn visit_existing_session_records(
    project: &Project,
    session_id: &StreamId,
    observer: &mut SessionRecordObserver<'_>,
) -> Result<(usize, blake3::Hash), ProjectEventStoreError> {
    let path = project.session_stream_path_existing(session_id)?;
    let stream_id = ProjectEventStream::Session(session_id.clone()).stream_id();
    let registry = EventUpcasterRegistry::current();
    let file = File::open(&path).map_err(|source| ProjectEventStoreError::Io {
        operation: "read Session Event stream for source index",
        path: path.clone(),
        source,
    })?;
    let mut hasher = blake3::Hasher::new();
    let scan = scan::scan_stream_reader_observed(
        &mut BufReader::new(file),
        &path,
        &stream_id,
        &registry,
        0,
        None,
        &mut hasher,
        false,
        Some(observer),
    )?;
    match scan {
        StreamScan::Complete {
            validated_bytes, ..
        } => Ok((validated_bytes, hasher.finalize())),
        StreamScan::Trailing { offset, .. } => {
            Err(ProjectEventStoreError::UnrecoveredTail { path, offset })
        }
    }
}

pub(crate) fn visit_session_records_after(
    project: &Project,
    session_id: &StreamId,
    cursor: &SessionStreamCursor,
    observer: &mut SessionRecordObserver<'_>,
) -> Result<(usize, blake3::Hash), ProjectEventStoreError> {
    let path = project.session_stream_path_existing(session_id)?;
    let stream_id = ProjectEventStream::Session(session_id.clone()).stream_id();
    if cursor.path != path || cursor.stream_id != stream_id {
        return Err(ProjectEventStoreError::ValidatedPrefixChanged { path });
    }
    visit_session_suffix(
        project,
        session_id,
        &SessionStreamCheckpoint {
            bytes: cursor.bytes,
            hash: *cursor.hash.finalize().as_bytes(),
            last_sequence: cursor.last_sequence,
        },
        Some(cursor),
        observer,
    )
}

pub(crate) fn visit_session_records_after_checkpoint(
    project: &Project,
    session_id: &StreamId,
    checkpoint: &SessionStreamCheckpoint,
    observer: &mut SessionRecordObserver<'_>,
) -> Result<(usize, blake3::Hash), ProjectEventStoreError> {
    visit_session_suffix(project, session_id, checkpoint, None, observer)
}

fn visit_session_suffix(
    project: &Project,
    session_id: &StreamId,
    checkpoint: &SessionStreamCheckpoint,
    cursor: Option<&SessionStreamCursor>,
    observer: &mut SessionRecordObserver<'_>,
) -> Result<(usize, blake3::Hash), ProjectEventStoreError> {
    let path = project.session_stream_path_existing(session_id)?;
    let stream_id = ProjectEventStream::Session(session_id.clone()).stream_id();
    let file = File::open(&path).map_err(|source| ProjectEventStoreError::Io {
        operation: "read Session Event suffix for source index",
        path: path.clone(),
        source,
    })?;
    let mut reader = BufReader::new(file);
    let mut hash = match cursor {
        Some(cursor) => verify_session_prefix(&mut reader, &path, &stream_id, cursor)?,
        None => verify_prefix_bytes(
            &mut reader,
            &path,
            checkpoint.bytes,
            blake3::Hash::from(checkpoint.hash),
        )?,
    };
    let registry = EventUpcasterRegistry::current();
    match scan::scan_stream_reader_observed(
        &mut reader,
        &path,
        &stream_id,
        &registry,
        checkpoint.bytes,
        checkpoint.last_sequence,
        &mut hash,
        false,
        Some(observer),
    )? {
        StreamScan::Complete {
            validated_bytes, ..
        } => Ok((validated_bytes, hash.finalize())),
        StreamScan::Trailing { offset, .. } => {
            Err(ProjectEventStoreError::UnrecoveredTail { path, offset })
        }
    }
}

/// Load and verify one bounded original byte segment of a previously indexed
/// Session stream. A same-size mutation is detected from bytes, not metadata.
pub(crate) fn read_indexed_session_segment(
    project: &Project,
    session_id: &StreamId,
    offset: u64,
    length: usize,
    expected: blake3::Hash,
) -> Result<Vec<u8>, ProjectEventStoreError> {
    const MAX_SEGMENT_BYTES: usize = 4 * 1024 * 1024;
    let path = project.session_stream_path_existing(session_id)?;
    if length > MAX_SEGMENT_BYTES {
        return Err(ProjectEventStoreError::Io {
            operation: "read indexed Session segment",
            path,
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "indexed segment exceeds 4 MiB",
            ),
        });
    }
    let mut file = File::open(&path).map_err(|source| ProjectEventStoreError::Io {
        operation: "open indexed Session segment",
        path: path.clone(),
        source,
    })?;
    file.seek(SeekFrom::Start(offset))
        .map_err(|source| ProjectEventStoreError::Io {
            operation: "seek indexed Session segment",
            path: path.clone(),
            source,
        })?;
    let mut bytes = vec![0; length];
    if let Err(source) = file.read_exact(&mut bytes) {
        if source.kind() == std::io::ErrorKind::UnexpectedEof {
            return Err(ProjectEventStoreError::ValidatedPrefixChanged { path });
        }
        return Err(ProjectEventStoreError::Io {
            operation: "read indexed Session segment",
            path,
            source,
        });
    }
    if blake3::hash(&bytes) != expected {
        return Err(ProjectEventStoreError::ValidatedPrefixChanged { path });
    }
    Ok(bytes)
}

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
    pub(crate) fn open_session_projection(
        &mut self,
        session_id: StreamId,
        cursor: Option<&SessionStreamCursor>,
    ) -> Result<(JsonlEventStore<'_>, Vec<EventEnvelope<Value>>), ProjectEventStoreError> {
        let stream = ProjectEventStream::Session(session_id.clone());
        let path = if cursor.is_some() {
            self.project.session_stream_path_existing(&session_id)?
        } else {
            self.project.stream_path(&stream)?
        };
        if let Some(parent) = path.parent() {
            create_directory(parent)?;
        }
        if !path.exists() {
            if cursor.is_some() {
                return Err(ProjectEventStoreError::ValidatedPrefixChanged { path });
            }
            create_empty_stream(&path)?;
        }
        let stream_id = stream.stream_id();
        let registry = EventUpcasterRegistry::current();
        let mut reader =
            BufReader::new(
                File::open(&path).map_err(|source| ProjectEventStoreError::Io {
                    operation: "read Session Event suffix",
                    path: path.clone(),
                    source,
                })?,
            );
        let revision = file_revision(reader.get_ref(), &path)?;
        let offset = cursor.map_or(0, |cursor| cursor.bytes);
        let mut hash = match cursor {
            Some(cursor) => verify_session_prefix(&mut reader, &path, &stream_id, cursor)?,
            None => blake3::Hasher::new(),
        };
        let scan = scan::scan_stream_reader(
            &mut reader,
            &path,
            &stream_id,
            &registry,
            offset,
            cursor.and_then(|cursor| cursor.last_sequence),
            &mut hash,
        )?;
        let (events, last_sequence, validated_bytes) = match scan {
            StreamScan::Complete {
                events,
                last_sequence,
                validated_bytes,
            } => (events, last_sequence, validated_bytes),
            StreamScan::Trailing { offset, .. } => {
                return Err(ProjectEventStoreError::RecoveryMetadataRequired { path, offset });
            }
        };
        let mut store = JsonlEventStore::open(
            path.clone(),
            stream_id.clone(),
            last_sequence,
            registry,
            PhantomData,
        )?;
        if file_revision(reader.get_ref(), &path)? != revision
            || file_revision(&store.file, &path)? != revision
        {
            return Err(ProjectEventStoreError::ValidatedPrefixChanged { path });
        }
        store.projection_cursor = Some(SessionStreamCursor {
            revision,
            path,
            stream_id,
            bytes: validated_bytes,
            hash,
            last_sequence,
        });
        Ok((store, events))
    }

    /// Opens the project Research JSONL Writer.
    pub fn open_research(
        &mut self,
        recovery: Option<RecoveryMetadata>,
    ) -> Result<JsonlEventStore<'_>, ProjectEventStoreError> {
        let stream = ProjectEventStream::Research;
        let path = self.project.research_stream_path_existing()?;
        self.open_existing_stream(stream, path, recovery)
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

        self.open_existing_stream(stream, path, recovery)
    }

    fn open_existing_stream(
        &mut self,
        stream: ProjectEventStream,
        path: PathBuf,
        recovery: Option<RecoveryMetadata>,
    ) -> Result<JsonlEventStore<'_>, ProjectEventStoreError> {
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
    projection_cursor: Option<SessionStreamCursor>,
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
            projection_cursor: None,
            _writer: writer,
        })
    }

    /// Returns the exact next sequence assigned by this stream Writer.
    pub fn next_sequence(&self) -> Result<EventSequence, ProjectEventStoreError> {
        next_sequence(self.last_sequence, &self.stream_id)
    }

    pub(crate) fn projection_cursor(&self) -> SessionStreamCursor {
        self.projection_cursor
            .as_ref()
            .expect("Session projection store was opened with a verified cursor")
            .clone()
    }

    /// The caller's prepared prefix belongs to this live projection. Opening
    /// this Writer already synchronized and verified the complete cached state.
    pub(crate) fn read_prepared_suffix(
        &self,
        prepared: &SessionStreamCursor,
    ) -> Result<Vec<EventEnvelope<Value>>, ProjectEventStoreError> {
        let current = self.projection_cursor();
        if prepared.path != current.path
            || prepared.stream_id != current.stream_id
            || prepared.bytes > current.bytes
        {
            return Err(ProjectEventStoreError::ValidatedPrefixChanged {
                path: self.path.clone(),
            });
        }
        let file = File::open(&self.path).map_err(|source| ProjectEventStoreError::Io {
            operation: "read prepared Session suffix",
            path: self.path.clone(),
            source,
        })?;
        let mut reader = BufReader::new(file);
        verify_session_prefix(&mut reader, &self.path, &self.stream_id, &current)?;
        reader
            .seek(SeekFrom::Start(prepared.bytes as u64))
            .map_err(|source| ProjectEventStoreError::Io {
                operation: "seek prepared Session suffix",
                path: self.path.clone(),
                source,
            })?;
        let mut hash = prepared.hash.clone();
        let scan = scan::scan_stream_reader(
            &mut reader,
            &self.path,
            &self.stream_id,
            &self.registry,
            prepared.bytes,
            prepared.last_sequence,
            &mut hash,
        )?;
        let events = scan.into_complete(&self.path)?;
        if hash.finalize() != current.hash.finalize() {
            return Err(ProjectEventStoreError::ValidatedPrefixChanged {
                path: self.path.clone(),
            });
        }
        Ok(events)
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

        if let Some(cursor) = &self.projection_cursor {
            let current = File::open(&self.path).map_err(|source| ProjectEventStoreError::Io {
                operation: "inspect Session path before append",
                path: self.path.clone(),
                source,
            })?;
            if file_revision(&self.file, &self.path)? != cursor.revision
                || file_revision(&current, &self.path)? != cursor.revision
            {
                return Err(ProjectEventStoreError::ValidatedPrefixChanged {
                    path: self.path.clone(),
                });
            }
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
        if let Some(cursor) = &mut self.projection_cursor {
            cursor.hash.update(&encoded);
            cursor.bytes += encoded.len();
            cursor.last_sequence = Some(event.sequence);
            cursor.revision = file_revision(&self.file, &self.path)?;
        }
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
