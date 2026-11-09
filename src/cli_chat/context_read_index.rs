//! Disposable, source-verified paging over validated Session history.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::Arc;

use ring::rand::SecureRandom;
use serde::{Deserialize, Serialize};

use super::{ChatError, ContextReadRuntime, ContextSource, ContextSources};
use crate::domain::model::{ContentItem, MessageRole};
use crate::domain::session::{HistorySourceSemantics, SessionState};
use crate::ports::context_read::{ContextReadRequest, ContextReadResponse};

const PAGE_BYTES: usize = 4 * 1024 * 1024;
const SEGMENT_BYTES: usize = 1024 * 1024;
const PAGE_RECORDS: usize = 256;
const INDEX_BUFFER_BYTES: usize = 8192;
const PREFIX_BYTES: usize = 1024;

#[derive(Serialize, Deserialize)]
pub(in crate::cli_chat) struct Record {
    pub(in crate::cli_chat) session: String,
    pub(in crate::cli_chat) ordinal: usize,
    pub(in crate::cli_chat) reference: String,
    event_id: String,
    searchable: bool,
    peer: bool,
    raw_offset: u64,
    raw_len: usize,
    hashes_offset: u64,
    text_hashes_offset: u64,
    text_offset: u64,
    text_len: usize,
    text_stored_len: usize,
    revision: String,
    role: Option<MessageRole>,
    pub(in crate::cli_chat) origin: crate::domain::context::ContextOrigin,
    pub(in crate::cli_chat) class: crate::domain::context::SemanticClass,
    pub(in crate::cli_chat) authority: crate::domain::context::AuthorityLevel,
    observed: String,
}

#[derive(Serialize, Deserialize)]
struct RecordFrame {
    record: Record,
    tag: [u8; 32],
}

#[derive(Serialize, Deserialize)]
struct PeerCheckpoint {
    session: String,
    source: crate::adapters::project_event_store::SessionStreamCheckpoint,
    event_count: usize,
    history_count: usize,
    included: bool,
}

#[derive(Serialize, Deserialize)]
struct PeerFrame {
    checkpoint: PeerCheckpoint,
    tag: [u8; 32],
}

#[derive(Serialize, Deserialize)]
struct OrdinalRecord {
    session: String,
    event: String,
    ordinal: usize,
}

#[derive(Serialize, Deserialize)]
struct OrdinalFrame {
    record: OrdinalRecord,
    tag: [u8; 32],
}

#[derive(Serialize, Deserialize)]
enum CallOwner {
    Unique { session: String, record_end: usize },
    Ambiguous,
}

#[derive(Serialize, Deserialize)]
struct CallOwnerRecord {
    call_id: String,
    owner: CallOwner,
}

#[derive(Serialize, Deserialize)]
struct CallOwnerFrame {
    record: CallOwnerRecord,
    tag: [u8; 32],
}

enum IndexSuffix<'a> {
    Current(&'a crate::adapters::project_event_store::SessionStreamCursor),
    Peer(&'a crate::adapters::project_event_store::SessionStreamCheckpoint),
}

#[derive(Clone, Serialize, Deserialize)]
struct AuxiliaryRecord {
    reference: String,
    scope: String,
    revision: String,
    observed_at: Option<String>,
    text: String,
    offset: usize,
    total_bytes: usize,
    memory_text_offset: u64,
    memory_hashes_offset: u64,
    metadata_len: usize,
    metadata_hash: [u8; 32],
}

#[derive(Serialize, Deserialize)]
struct AuxiliaryFrame {
    record: AuxiliaryRecord,
    tag: [u8; 32],
}

struct AuxiliaryFile(PathBuf);

impl Drop for AuxiliaryFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

struct Scratch(PathBuf);

struct PeerDirectory(PathBuf);

#[derive(Clone, Copy, Default)]
struct IndexWork {
    source_scan_bytes: usize,
    text_write_bytes: usize,
    hash_write_bytes: usize,
    record_write_bytes: usize,
}

impl IndexWork {
    fn add(&mut self, other: Self) {
        self.source_scan_bytes += other.source_scan_bytes;
        self.text_write_bytes += other.text_write_bytes;
        self.hash_write_bytes += other.hash_write_bytes;
        self.record_write_bytes += other.record_write_bytes;
    }
}

impl Drop for PeerDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(in crate::cli_chat) struct ColdIndex {
    scratch: Arc<Scratch>,
    peers: Arc<PeerDirectory>,
    peer_count: usize,
    peer_manifest: [u8; 32],
    nonce: [u8; 32],
    index_len: u64,
    watermark: blake3::Hash,
    current_cursor: crate::adapters::project_event_store::SessionStreamCursor,
    current_history_count: usize,
    agent_id: String,
    indexed_events: usize,
    indexed_bytes: usize,
    initial_indexed_events: usize,
    initial_indexed_bytes: usize,
    preparation_source_bytes: usize,
    preparation_text_bytes: usize,
    preparation_hash_bytes: usize,
    preparation_record_bytes: usize,
    owners: Arc<std::sync::Mutex<super::super::task_posting_manifest::PostingManifest>>,
}

pub(super) struct ColdSearch {
    index: Arc<ColdIndex>,
    cursor_key: [u8; 32],
    query: String,
    after: Option<u64>,
    before: Option<u64>,
    token: String,
    scope: String,
    definition_digest: String,
    generation: u64,
    index_offset: u64,
    record: Option<Record>,
    raw_part: usize,
    text_position: usize,
    tail: Vec<u8>,
    auxiliary: AuxiliaryFile,
    auxiliary_text: AuxiliaryFile,
    auxiliary_hashes: AuxiliaryFile,
    record_text: AuxiliaryFile,
    record_hashes: AuxiliaryFile,
    record_materialized: bool,
    auxiliary_offset: u64,
    auxiliary_len: u64,
    auxiliary_record: Option<AuxiliaryRecord>,
    auxiliary_position: usize,
    auxiliary_tail: Vec<u8>,
    auxiliary_metadata: bool,
    ambiguous_time: bool,
    indexed_events: usize,
    indexed_bytes: usize,
    initial_indexed_events: usize,
    initial_indexed_bytes: usize,
    auxiliary_preparation_memory_read_bytes: usize,
    auxiliary_preparation_write_bytes: usize,
    source_index_work: IndexWork,
}

struct HashWriter<'a> {
    file: &'a mut File,
    hashes: &'a mut File,
    hash: blake3::Hasher,
    len: usize,
    part_len: usize,
    key: &'a [u8; 32],
}

struct MeteredFile {
    file: File,
    read_bytes: usize,
}

impl Read for MeteredFile {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let amount = self.file.read(bytes)?;
        self.read_bytes += amount;
        Ok(amount)
    }
}

impl Seek for MeteredFile {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.file.seek(position)
    }
}

impl Write for HashWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.file.write_all(bytes)?;
        let mut remaining = bytes;
        while !remaining.is_empty() {
            let amount = remaining.len().min(SEGMENT_BYTES - self.part_len);
            self.hash.update(&remaining[..amount]);
            self.part_len += amount;
            remaining = &remaining[amount..];
            if self.part_len == SEGMENT_BYTES {
                write_hash(self.hashes, self.key, self.hash.finalize())?;
                self.hash = blake3::Hasher::new();
                self.part_len = 0;
            }
        }
        self.len += bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

fn fail(message: impl ToString) -> ChatError {
    ChatError::CompactionSummarySource {
        message: message.to_string(),
    }
}

fn tag(key: &[u8; 32], offset: u64, bytes: &[u8]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new_keyed(key);
    hasher.update(&offset.to_le_bytes());
    hasher.update(bytes);
    *hasher.finalize().as_bytes()
}

fn record_text_key(cursor_key: &[u8; 32], record: &Record) -> Result<[u8; 32], ChatError> {
    let identity = serde_json::to_vec(&(
        &record.session,
        &record.event_id,
        record.raw_offset,
        record.raw_len,
        &record.revision,
    ))
    .map_err(fail)?;
    Ok(*blake3::keyed_hash(cursor_key, &identity).as_bytes())
}

fn write_hash(file: &mut File, key: &[u8; 32], hash: blake3::Hash) -> io::Result<()> {
    let offset = file.stream_position()?;
    file.write_all(hash.as_bytes())?;
    file.write_all(&tag(key, offset, hash.as_bytes()))
}

fn read_hash(file: &mut File, key: &[u8; 32], offset: u64) -> Result<blake3::Hash, ChatError> {
    file.seek(SeekFrom::Start(offset))?;
    let mut digest = [0u8; 32];
    let mut actual_tag = [0u8; 32];
    file.read_exact(&mut digest)?;
    file.read_exact(&mut actual_tag)?;
    if actual_tag != tag(key, offset, &digest) {
        return Err(fail("derived context index hash changed"));
    }
    Ok(blake3::Hash::from(digest))
}

fn peer_path(peers: &PeerDirectory, session: &str) -> PathBuf {
    peers
        .0
        .join(blake3::hash(session.as_bytes()).to_hex().to_string())
}

fn owner_path(scratch: &Scratch, id: &str) -> PathBuf {
    scratch
        .0
        .join("call-owners")
        .join(blake3::hash(id.as_bytes()).to_hex().to_string())
}

fn read_call_owner(
    scratch: &Scratch,
    key: &[u8; 32],
    manifest: &super::super::task_posting_manifest::PostingManifest,
    id: &str,
) -> Result<Option<CallOwner>, ChatError> {
    let path = owner_path(scratch, id);
    let expected = manifest.get("artifact-owner", blake3::hash(id.as_bytes()))?;
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound && expected.is_none() => {
            return Ok(None);
        }
        Err(error) => return Err(error.into()),
    };
    if expected != Some(bytes.len() as u64) {
        return Err(fail("derived Artifact owner size changed"));
    }
    let frame: CallOwnerFrame = serde_json::from_slice(&bytes).map_err(fail)?;
    if frame.record.call_id != id
        || frame.tag != tag(key, 0, &serde_json::to_vec(&frame.record).map_err(fail)?)
    {
        return Err(fail("derived Artifact owner changed"));
    }
    Ok(Some(frame.record.owner))
}

fn write_call_owner(
    scratch: &Scratch,
    key: &[u8; 32],
    manifest: &mut super::super::task_posting_manifest::PostingManifest,
    id: &str,
    session: &str,
    record_end: usize,
) -> Result<(), ChatError> {
    let owner = match read_call_owner(scratch, key, manifest, id)? {
        None => CallOwner::Unique {
            session: session.to_owned(),
            record_end,
        },
        Some(CallOwner::Unique { session: old, .. }) if old == session => return Ok(()),
        Some(_) => CallOwner::Ambiguous,
    };
    let record = CallOwnerRecord {
        call_id: id.to_owned(),
        owner,
    };
    let frame = CallOwnerFrame {
        tag: tag(key, 0, &serde_json::to_vec(&record).map_err(fail)?),
        record,
    };
    let bytes = serde_json::to_vec(&frame).map_err(fail)?;
    let path = owner_path(scratch, id);
    let temporary = path.with_extension("new");
    fs::write(&temporary, &bytes)?;
    fs::rename(temporary, path)?;
    manifest.set(
        "artifact-owner",
        blake3::hash(id.as_bytes()),
        bytes.len() as u64,
    )?;
    Ok(())
}

fn ordinal_path(scratch: &Scratch, session: &str, event: &str) -> PathBuf {
    let key = serde_json::to_vec(&(session, event)).expect("source identity serializes");
    scratch
        .0
        .join("ordinals")
        .join(blake3::hash(&key).to_hex().to_string())
}

fn read_ordinal(
    scratch: &Scratch,
    key: &[u8; 32],
    session: &str,
    event: &str,
) -> Result<Option<usize>, ChatError> {
    let bytes = match fs::read(ordinal_path(scratch, session, event)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(fail("derived source ordinal is missing"));
        }
        Err(error) => return Err(error.into()),
    };
    let frame: OrdinalFrame = serde_json::from_slice(&bytes).map_err(fail)?;
    if frame.record.session != session
        || frame.record.event != event
        || frame.tag != tag(key, 0, &serde_json::to_vec(&frame.record).map_err(fail)?)
    {
        return Err(fail("derived source ordinal changed"));
    }
    Ok(Some(frame.record.ordinal))
}

fn write_ordinal(
    scratch: &Scratch,
    key: &[u8; 32],
    session: &str,
    event: &str,
    ordinal: usize,
) -> Result<(), ChatError> {
    let record = OrdinalRecord {
        session: session.to_owned(),
        event: event.to_owned(),
        ordinal,
    };
    let frame = OrdinalFrame {
        tag: tag(key, 0, &serde_json::to_vec(&record).map_err(fail)?),
        record,
    };
    let path = ordinal_path(scratch, session, event);
    let temporary = path.with_extension("new");
    fs::write(&temporary, serde_json::to_vec(&frame).map_err(fail)?)?;
    fs::rename(temporary, path)?;
    Ok(())
}

fn read_peer(
    peers: &PeerDirectory,
    key: &[u8; 32],
    session: &str,
) -> Result<Option<PeerCheckpoint>, ChatError> {
    let bytes = match fs::read(peer_path(peers, session)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let frame: PeerFrame = serde_json::from_slice(&bytes).map_err(fail)?;
    if frame.checkpoint.session != session
        || frame.tag
            != tag(
                key,
                0,
                &serde_json::to_vec(&frame.checkpoint).map_err(fail)?,
            )
    {
        return Err(fail("derived Session checkpoint changed"));
    }
    Ok(Some(frame.checkpoint))
}

fn write_peer(
    peers: &PeerDirectory,
    key: &[u8; 32],
    checkpoint: PeerCheckpoint,
) -> Result<(), ChatError> {
    let encoded = serde_json::to_vec(&checkpoint).map_err(fail)?;
    let path = peer_path(peers, &checkpoint.session);
    let temporary = path.with_extension("new");
    let frame = PeerFrame {
        tag: tag(key, 0, &encoded),
        checkpoint,
    };
    fs::write(&temporary, serde_json::to_vec(&frame).map_err(fail)?)?;
    fs::rename(temporary, path)?;
    Ok(())
}

fn fork_peers(index: &ColdIndex) -> Result<Arc<PeerDirectory>, ChatError> {
    let mut random = [0u8; 32];
    ring::rand::SystemRandom::new()
        .fill(&mut random)
        .map_err(|_| fail("cannot create peer checkpoint view"))?;
    let peers = Arc::new(PeerDirectory(
        index
            .scratch
            .0
            .join(format!("peers-{}", blake3::hash(&random).to_hex())),
    ));
    fs::create_dir(&peers.0)?;
    for entry in fs::read_dir(&index.peers.0)? {
        let entry = entry?;
        fs::hard_link(entry.path(), peers.0.join(entry.file_name()))?;
    }
    Ok(peers)
}

fn peer_manifest(peers: &PeerDirectory, key: &[u8; 32]) -> Result<(usize, [u8; 32]), ChatError> {
    let mut count = 0usize;
    let mut digest = [0u8; 32];
    for entry in fs::read_dir(&peers.0)? {
        let entry = entry?;
        let frame: PeerFrame = serde_json::from_slice(&fs::read(entry.path())?).map_err(fail)?;
        let checkpoint = read_peer(peers, key, &frame.checkpoint.session)?
            .ok_or_else(|| fail("Session checkpoint disappeared"))?;
        if entry.path() != peer_path(peers, &checkpoint.session) {
            return Err(fail("unexpected Session checkpoint entry"));
        }
        let item = tag(key, 0, &serde_json::to_vec(&checkpoint).map_err(fail)?);
        for (result, byte) in digest.iter_mut().zip(item) {
            *result ^= byte;
        }
        count += 1;
    }
    Ok((count, digest))
}

fn match_offset(
    project: &str,
    text: &str,
    query: &str,
    maximum: usize,
    retrieval_work: &mut crate::domain::retrieval::TaskRetrievalWork,
) -> Result<Option<usize>, ChatError> {
    let excerpt = crate::domain::retrieval::TaskExcerptLayout::new(text, query, maximum);
    let overlap = crate::domain::retrieval::task_query_source_span_bound(query);
    let width = crate::domain::retrieval::MAX_FIELD_BYTES + overlap;
    let mut start = 0;
    while start < text.len() {
        let mut end = text.len().min(start + width);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        let part = &text[start..end];
        if let Some(offset) = crate::domain::retrieval::task_ranked_match_offset(
            project,
            part,
            query,
            &excerpt,
            start,
            retrieval_work,
        )
        .map_err(fail)?
        {
            return Ok(Some(excerpt.range(start + offset).start));
        }
        if end == text.len() {
            break;
        }
        start = end - overlap;
        while !text.is_char_boundary(start) {
            start -= 1;
        }
    }
    Ok(None)
}

fn verified_auxiliary(
    runtime: &ContextReadRuntime,
    index: &ColdIndex,
    record: AuxiliaryRecord,
    maximum: usize,
) -> Result<(ContextSource, usize), ChatError> {
    let mut verified_bytes = 0usize;
    let (text, total_bytes, scope, semantics, observed_at) = if let Some((path, revision)) = record
        .reference
        .strip_prefix("memory:")
        .and_then(|value| value.rsplit_once(':'))
    {
        let (historical, current) =
            runtime.memory_source(path, revision, &record.reference, record.offset, maximum)?;
        verified_bytes += historical.as_ref().map_or(0, |source| source.total_bytes);
        verified_bytes += current.as_ref().map_or(0, |source| source.total_bytes);
        let source = historical
            .or(current.filter(|source| source.revision == revision))
            .ok_or_else(|| fail("Memory source revision is unavailable"))?;
        let local = record
            .offset
            .checked_sub(source.available_offset)
            .ok_or_else(|| fail("Memory source range changed"))?;
        if local > source.text.len() || !source.text.is_char_boundary(local) {
            return Err(fail("Memory source range changed"));
        }
        let mut end = source.text.len().min(local.saturating_add(maximum));
        while !source.text.is_char_boundary(end) {
            end -= 1;
        }
        (
            source.text[local..end].to_owned(),
            source.total_bytes,
            "memory",
            super::MEMORY_SEMANTICS,
            source.observed_at,
        )
    } else if record.scope == "artifact_metadata" {
        let (id, blob_revision, metadata_revision) =
            super::artifact_reference(&record.reference)
                .ok_or_else(|| fail("invalid Artifact reference"))?;
        runtime.session.with_projection(|projection, _, _| {
            let project = runtime.session.project_read(projection.state())?;
            let artifact = project
                .research_state()
                .artifact(&crate::domain::artifacts::ArtifactId::new(id))
                .filter(|artifact| artifact.blob_id.as_str() == blob_revision)
                .ok_or_else(|| fail("Artifact source revision is unavailable"))?;
            let (owner, verified) = index.artifact_owner(
                &runtime.session.project,
                &runtime.session.session_id,
                artifact,
                true,
            )?;
            verified_bytes += verified;
            if owner != Some(true) {
                return Err(fail("Artifact source scope changed"));
            }
            let value = serde_json::to_string(artifact).map_err(fail)?;
            if blake3::hash(value.as_bytes()).as_bytes() != &record.metadata_hash {
                return Err(fail("Artifact metadata changed after indexing"));
            }
            if metadata_revision != Some(record.revision.as_str()) {
                return Err(fail(
                    "Artifact metadata revision differs from its reference",
                ));
            }
            if record.offset > value.len() || !value.is_char_boundary(record.offset) {
                return Err(fail("Artifact source range changed"));
            }
            let mut end = value.len().min(record.offset.saturating_add(maximum));
            while !value.is_char_boundary(end) {
                end -= 1;
            }
            let observed = project
                .events
                .iter()
                .find(|event| event.event_id == artifact.created_by_event_id)
                .map(|event| event.occurred_at.as_str().to_owned());
            Ok((
                value[record.offset..end].to_owned(),
                value.len(),
                "artifact_metadata",
                super::ARTIFACT_SEMANTICS,
                observed,
            ))
        })?
    } else {
        let (response, verified) =
            runtime.read_artifact_scoped(&record.reference, record.offset, maximum, index)?;
        verified_bytes += verified;
        if matches!(
            response.status,
            crate::ports::context_read::ContextReadStatus::Ambiguous
        ) {
            return Err(fail("Artifact source scope changed"));
        }
        let entry = response
            .entries
            .into_iter()
            .next()
            .ok_or_else(|| fail("Artifact source revision is unavailable"))?;
        if entry.source_revision != record.revision || entry.scope != record.scope {
            return Err(fail("Artifact source revision changed"));
        }
        verified_bytes += entry.total_bytes;
        (
            entry.text,
            entry.total_bytes,
            "artifact",
            super::ARTIFACT_SEMANTICS,
            entry.observed_at,
        )
    };
    if total_bytes != record.total_bytes || !record.text.starts_with(&text) {
        return Err(fail("auxiliary context source changed after indexing"));
    }
    Ok((
        ContextSource {
            reference: record.reference,
            scope,
            revision: record.revision,
            role: None,
            semantics,
            observed_at,
            text,
            searchable: true,
            available_offset: record.offset,
            total_bytes,
        },
        verified_bytes,
    ))
}

impl ColdIndex {
    pub(super) fn artifact_owner(
        &self,
        project: &crate::adapters::project_event_store::Project,
        current_session: &crate::domain::events::StreamId,
        record: &crate::domain::artifacts::ArtifactRecord,
        verify: bool,
    ) -> Result<(Option<bool>, usize), ChatError> {
        use crate::domain::artifacts::ArtifactScope;
        let owner = match &record.scope {
            ArtifactScope::Session { session_id } => Some((session_id.as_str().to_owned(), None)),
            ArtifactScope::Target { .. } => {
                let owners = self
                    .owners
                    .lock()
                    .map_err(|_| io::Error::other("Artifact owner index lock poisoned"))?;
                match read_call_owner(
                    &self.scratch,
                    &self.nonce,
                    &owners,
                    record.produced_by_tool_call_id.as_str(),
                )? {
                    Some(CallOwner::Unique {
                        session,
                        record_end,
                    }) => Some((session, Some(record_end))),
                    Some(CallOwner::Ambiguous) | None => None,
                }
            }
        };
        let Some((owner, record_end)) = owner else {
            return Ok((None, 0));
        };
        let (included, checkpoint) = if owner == current_session.as_str() {
            (true, self.current_cursor.checkpoint())
        } else {
            let Some(peer) = read_peer(&self.peers, &self.nonce, &owner)? else {
                return Ok((None, 0));
            };
            (peer.included, peer.source)
        };
        if !included {
            return Ok((Some(false), 0));
        }
        if record_end.is_some_and(|end| end > checkpoint.bytes) {
            return Ok((None, 0));
        }
        let verified = if verify {
            crate::adapters::project_event_store::verify_session_checkpoint(
                project,
                &crate::domain::events::StreamId::new(owner),
                &checkpoint,
            )?
        } else {
            0
        };
        Ok((Some(true), verified))
    }

    pub(super) fn refresh_peers(runtime: &ContextReadRuntime) -> Result<Arc<Self>, ChatError> {
        Self::refresh_peers_with_work(runtime).map(|(index, _)| index)
    }

    fn refresh_peers_with_work(
        runtime: &ContextReadRuntime,
    ) -> Result<(Arc<Self>, IndexWork), ChatError> {
        let mut cached = runtime
            .session
            .context_source_index
            .lock()
            .map_err(|_| io::Error::other("context source index lock poisoned"))?;
        let previous = cached
            .as_ref()
            .ok_or_else(|| fail("context source index is unavailable"))?;
        let mut owners = previous
            .owners
            .lock()
            .map_err(|_| io::Error::other("Artifact owner index lock poisoned"))?;
        if peer_manifest(&previous.peers, &previous.nonce)?
            != (previous.peer_count, previous.peer_manifest)
        {
            return Err(fail("derived Session checkpoint set changed"));
        }
        let mut records = OpenOptions::new()
            .read(true)
            .write(true)
            .open(previous.scratch.0.join("records"))?;
        let mut text = OpenOptions::new()
            .read(true)
            .write(true)
            .open(previous.scratch.0.join("text"))?;
        let mut hashes = OpenOptions::new()
            .read(true)
            .write(true)
            .open(previous.scratch.0.join("hashes"))?;
        records.set_len(previous.index_len)?;
        records.seek(SeekFrom::Start(previous.index_len))?;
        text.seek(SeekFrom::End(0))?;
        hashes.seek(SeekFrom::End(0))?;
        let old_text = text.stream_position()?;
        let old_hashes = hashes.stream_position()?;
        let mut work = IndexWork::default();
        let mut staged: Option<Arc<PeerDirectory>> = None;
        let mut rebuild = false;
        let mut indexed_events = 0usize;
        let mut indexed_bytes = 0usize;
        let mut watermark = blake3::Hasher::new();
        watermark.update(previous.watermark.as_bytes());
        runtime
            .session
            .project
            .visit_existing_session_ids::<ChatError>(|id| {
                if id == runtime.session.session_id {
                    return Ok(());
                }
                let old = read_peer(&previous.peers, &previous.nonce, id.as_str())?;
                let path = runtime.session.project.session_stream_path_existing(&id)?;
                let length = fs::metadata(&path)?.len();
                if let Some(old) = &old {
                    if length < old.source.bytes as u64 {
                        return Err(fail("Session source was truncated"));
                    }
                    if length == old.source.bytes as u64 {
                        let (verified, hash) = crate::adapters::project_event_store::visit_session_records_after_checkpoint(
                            &runtime.session.project, &id, &old.source, &mut |_, _, _, _, _| {
                                Err(crate::adapters::project_event_store::ProjectEventStoreError::ValidatedPrefixChanged { path: path.clone() })
                            },
                        )?;
                        if verified != old.source.bytes || hash.as_bytes() != &old.source.hash {
                            return Err(fail("Session source changed during scope verification"));
                        }
                        work.source_scan_bytes += verified;
                        return Ok(());
                    }
                }
                let snapshot = runtime.peer_snapshot(&id)?;
                if old
                    .as_ref()
                    .is_some_and(|old| old.included && snapshot.projection.is_none())
                {
                    rebuild = true;
                    return Ok(());
                }
                let peers = match &staged {
                    Some(peers) => Arc::clone(peers),
                    None => {
                        let peers = fork_peers(previous)?;
                        staged = Some(Arc::clone(&peers));
                        peers
                    }
                };
                let (checkpoint, semantics_changed) = Self::index_peer(
                    runtime,
                    &id,
                    snapshot,
                    old.as_ref(),
                    &mut records,
                    &mut text,
                    &mut hashes,
                    &previous.scratch,
                    &previous.nonce,
                    &mut owners,
                )?;
                work.source_scan_bytes += checkpoint.source.bytes;
                if semantics_changed {
                    rebuild = true;
                    return Ok(());
                }
                indexed_events += checkpoint.event_count - old.as_ref().map_or(0, |value| value.event_count);
                indexed_bytes += checkpoint.source.bytes - old.as_ref().map_or(0, |value| value.source.bytes);
                watermark.update(id.as_str().as_bytes());
                watermark.update(&(checkpoint.source.bytes as u64).to_le_bytes());
                watermark.update(&checkpoint.source.hash);
                write_peer(&peers, &previous.nonce, checkpoint)?;
                Ok(())
            })?;
        for entry in fs::read_dir(&previous.peers.0)? {
            let frame: PeerFrame =
                serde_json::from_slice(&fs::read(entry?.path())?).map_err(fail)?;
            let checkpoint =
                read_peer(&previous.peers, &previous.nonce, &frame.checkpoint.session)?
                    .ok_or_else(|| fail("Session checkpoint disappeared"))?;
            runtime.session.project.session_stream_path_existing(
                &crate::domain::events::StreamId::new(&checkpoint.session),
            )?;
        }
        work.text_write_bytes += (text.stream_position()? - old_text) as usize;
        work.hash_write_bytes += (hashes.stream_position()? - old_hashes) as usize;
        work.record_write_bytes += (records.stream_position()? - previous.index_len) as usize;
        if rebuild {
            let rebuilt = Self::build(runtime)?;
            work.add(rebuilt.index_work());
            let updated = Arc::new(rebuilt);
            drop(owners);
            *cached = Some(updated);
            drop(cached);
            let (index, repeated) = Self::refresh_peers_with_work(runtime)?;
            work.add(repeated);
            return Ok((index, work));
        }
        let updated = if let Some(peers) = staged {
            records.flush()?;
            text.flush()?;
            hashes.flush()?;
            let (peer_count, peer_manifest) = peer_manifest(&peers, &previous.nonce)?;
            Arc::new(Self {
                scratch: Arc::clone(&previous.scratch),
                peers,
                peer_count,
                peer_manifest,
                nonce: previous.nonce,
                index_len: records.stream_position()?,
                watermark: watermark.finalize(),
                current_cursor: previous.current_cursor.clone(),
                current_history_count: previous.current_history_count,
                agent_id: previous.agent_id.clone(),
                indexed_events,
                indexed_bytes,
                initial_indexed_events: previous.initial_indexed_events,
                initial_indexed_bytes: previous.initial_indexed_bytes,
                preparation_source_bytes: previous.preparation_source_bytes,
                preparation_text_bytes: previous.preparation_text_bytes,
                preparation_hash_bytes: previous.preparation_hash_bytes,
                preparation_record_bytes: previous.preparation_record_bytes,
                owners: Arc::clone(&previous.owners),
            })
        } else {
            Arc::clone(previous)
        };
        drop(owners);
        *cached = Some(Arc::clone(&updated));
        Ok((updated, work))
    }

    #[allow(clippy::too_many_arguments)]
    fn index_peer(
        runtime: &ContextReadRuntime,
        id: &crate::domain::events::StreamId,
        snapshot: super::PeerSnapshot,
        previous: Option<&PeerCheckpoint>,
        records: &mut File,
        text: &mut File,
        hashes: &mut File,
        scratch: &Scratch,
        key: &[u8; 32],
        owners: &mut super::super::task_posting_manifest::PostingManifest,
    ) -> Result<(PeerCheckpoint, bool), ChatError> {
        let previous_count = previous.map_or(0, |value| value.event_count);
        if previous_count > snapshot.events.len() {
            return Err(fail("Session source was truncated"));
        }
        let included = snapshot.projection.is_some();
        let use_suffix = previous.is_some_and(|value| value.included == included);
        let suffix_events = if use_suffix {
            &snapshot.events[previous_count..]
        } else {
            &snapshot.events[..]
        };
        let mut semantics_changed = false;
        let watermark = if let Some(projection) = &snapshot.projection {
            let resume = if use_suffix {
                previous.map(|value| IndexSuffix::Peer(&value.source))
            } else {
                None
            };
            Self::index_session(
                runtime,
                id,
                projection.state(),
                Some(suffix_events),
                resume,
                if use_suffix {
                    previous.map_or(0, |value| value.history_count)
                } else {
                    0
                },
                records,
                text,
                hashes,
                scratch,
                key,
                owners,
                &mut semantics_changed,
            )?
        } else {
            let path = runtime.session.project.session_stream_path_existing(id)?;
            let mut verified = suffix_events.iter();
            let mut observer = |event: &crate::domain::events::EventEnvelope<serde_json::Value>,
                                raw_offset,
                                raw_len,
                                _,
                                _: &[u8]| {
                if verified.next() != Some(event) {
                    return Err(crate::adapters::project_event_store::ProjectEventStoreError::ValidatedPrefixChanged { path: path.clone() });
                }
                if event.event_type.as_str()
                    == crate::domain::session::HISTORY_ITEM_RECORDED_EVENT_TYPE
                {
                    let decoded = crate::domain::session::SessionEvent::decode(
                        &event.event_type,
                        event.payload.clone(),
                    )
                    .map_err(|error| {
                        crate::adapters::project_event_store::ProjectEventStoreError::Io {
                            operation: "decode Artifact owner source",
                            path: path.clone(),
                            source: io::Error::other(error),
                        }
                    })?;
                    if let crate::domain::session::SessionEvent::HistoryItemRecorded {
                        item: ContentItem::ToolCall { call },
                    } = decoded
                    {
                        write_call_owner(
                            scratch,
                            key,
                            owners,
                            call.call_id.as_str(),
                            id.as_str(),
                            raw_offset + raw_len,
                        )
                        .map_err(|error| {
                            crate::adapters::project_event_store::ProjectEventStoreError::Io {
                                operation: "index Artifact owner",
                                path: path.clone(),
                                source: io::Error::other(error.to_string()),
                            }
                        })?;
                    }
                }
                Ok(())
            };
            let result = if use_suffix {
                crate::adapters::project_event_store::visit_session_records_after_checkpoint(
                    &runtime.session.project,
                    id,
                    &previous.expect("suffix has checkpoint").source,
                    &mut observer,
                )?
            } else {
                crate::adapters::project_event_store::visit_existing_session_records(
                    &runtime.session.project,
                    id,
                    &mut observer,
                )?
            };
            if verified.next().is_some() {
                return Err(fail("Session changed during scope verification"));
            }
            result
        };
        Ok((
            PeerCheckpoint {
                session: id.as_str().to_owned(),
                source: crate::adapters::project_event_store::SessionStreamCheckpoint {
                    bytes: watermark.0,
                    hash: *watermark.1.as_bytes(),
                    last_sequence: snapshot.events.last().map(|event| event.sequence),
                },
                event_count: snapshot.events.len(),
                history_count: snapshot.history_count,
                included,
            },
            semantics_changed,
        ))
    }

    pub(in crate::cli_chat) fn identity(&self) -> blake3::Hash {
        blake3::hash(&self.nonce)
    }

    fn index_work(&self) -> IndexWork {
        IndexWork {
            source_scan_bytes: self.preparation_source_bytes,
            text_write_bytes: self.preparation_text_bytes,
            hash_write_bytes: self.preparation_hash_bytes,
            record_write_bytes: self.preparation_record_bytes,
        }
    }

    pub(in crate::cli_chat) fn preparation_work(
        &self,
    ) -> (blake3::Hash, usize, usize, usize, usize) {
        (
            blake3::Hash::from_bytes(self.current_cursor.checkpoint().hash),
            self.preparation_source_bytes,
            self.preparation_text_bytes,
            self.preparation_hash_bytes,
            self.preparation_record_bytes,
        )
    }

    /// Visits authenticated record metadata at the frozen index watermark.
    /// This is an initial index-building operation, not a per-query scan.
    pub(in crate::cli_chat) fn visit_records(
        &self,
        offset: u64,
        mut visit: impl FnMut(Record) -> Result<(), ChatError>,
    ) -> Result<u64, ChatError> {
        let mut file = BufReader::new(File::open(self.scratch.0.join("records"))?);
        if offset > self.index_len {
            return Err(fail("context index position is outside its watermark"));
        }
        file.seek(SeekFrom::Start(offset))?;
        while file.stream_position()? < self.index_len {
            let offset = file.stream_position()?;
            let mut line = String::new();
            if file.read_line(&mut line)? == 0 {
                return Err(fail("derived context index was truncated"));
            }
            let frame: RecordFrame = serde_json::from_str(&line).map_err(fail)?;
            if frame.tag
                != tag(
                    &self.nonce,
                    offset,
                    &serde_json::to_vec(&frame.record).map_err(fail)?,
                )
                || file.stream_position()? > self.index_len
            {
                return Err(fail("derived context index record changed"));
            }
            visit(frame.record)?;
        }
        Ok(self.index_len)
    }

    /// Return a bounded canonical prefix only after checking the real record
    /// bytes and the corresponding authenticated derived-cache segment.
    pub(in crate::cli_chat) fn verified_prefix(
        &self,
        project: &crate::adapters::project_event_store::Project,
        record: &Record,
        maximum: usize,
    ) -> Result<(String, bool, usize), ChatError> {
        let (source_bytes, hash_bytes) = self.verify_record(project, record)?;
        let mut read_bytes = source_bytes + hash_bytes;
        let mut hashes = File::open(self.scratch.0.join("hashes"))?;
        let mut file = File::open(self.scratch.0.join("text"))?;
        let mut result = Vec::new();
        let mut position = 0;
        while position < record.text_stored_len && result.len() < maximum {
            let length = (record.text_stored_len - position).min(SEGMENT_BYTES);
            file.seek(SeekFrom::Start(record.text_offset + position as u64))?;
            let mut segment = vec![0; length];
            file.read_exact(&mut segment)?;
            read_bytes += segment.len();
            let expected = read_hash(
                &mut hashes,
                &self.nonce,
                record.text_hashes_offset + ((position / SEGMENT_BYTES) * 64) as u64,
            )?;
            read_bytes += 64;
            if blake3::hash(&segment) != expected {
                return Err(fail("derived context source cache changed"));
            }
            let amount = segment.len().min(maximum - result.len());
            result.extend_from_slice(&segment[..amount]);
            position += length;
        }
        let text = super::range_text(result)?;
        let truncated = text.len() < record.text_len;
        Ok((text, truncated, read_bytes))
    }

    fn verify_record(
        &self,
        project: &crate::adapters::project_event_store::Project,
        record: &Record,
    ) -> Result<(usize, usize), ChatError> {
        let mut hashes = File::open(self.scratch.0.join("hashes"))?;
        let mut position = 0;
        let mut hash_bytes = 0;
        while position < record.raw_len {
            let length = (record.raw_len - position).min(SEGMENT_BYTES);
            let expected = read_hash(
                &mut hashes,
                &self.nonce,
                record.hashes_offset + ((position / SEGMENT_BYTES) * 64) as u64,
            )?;
            hash_bytes += 64;
            let actual = crate::adapters::project_event_store::read_indexed_session_segment(
                project,
                &crate::domain::events::StreamId::new(&record.session),
                record.raw_offset + position as u64,
                length,
                expected,
            )?;
            position += actual.len();
        }
        Ok((position, hash_bytes))
    }

    pub(super) fn acquire_runtime(runtime: &ContextReadRuntime) -> Result<Arc<Self>, ChatError> {
        let mut cached = runtime
            .session
            .context_source_index
            .lock()
            .map_err(|_| io::Error::other("context source index lock poisoned"))?;
        let Some(previous) = cached.as_ref() else {
            let index = Arc::new(Self::build(runtime)?);
            *cached = Some(Arc::clone(&index));
            return Ok(index);
        };
        if previous.agent_id != runtime.current_binding()?.definition().id().as_str() {
            let index = Arc::new(Self::build(runtime)?);
            *cached = Some(Arc::clone(&index));
            return Ok(index);
        }
        let mut discarded = (0usize, 0usize, 0usize, 0usize);
        let updated = runtime.session.with_projection(|projection, store, _| {
            let cursor = store.projection_cursor();
            if previous.current_cursor.same_prefix(&cursor) {
                return Ok(Some(Arc::clone(previous)));
            }
            let mut records = OpenOptions::new()
                .read(true)
                .write(true)
                .open(previous.scratch.0.join("records"))?;
            let mut text = OpenOptions::new()
                .read(true)
                .write(true)
                .open(previous.scratch.0.join("text"))?;
            let mut hashes = OpenOptions::new()
                .read(true)
                .write(true)
                .open(previous.scratch.0.join("hashes"))?;
            records.set_len(previous.index_len)?;
            records.seek(SeekFrom::Start(previous.index_len))?;
            text.seek(SeekFrom::End(0))?;
            hashes.seek(SeekFrom::End(0))?;
            let old_text = text.stream_position()?;
            let old_hashes = hashes.stream_position()?;
            let mut owners = previous
                .owners
                .lock()
                .map_err(|_| io::Error::other("Artifact owner index lock poisoned"))?;
            let mut semantics_changed = false;
            let (length, hash) = Self::index_session(
                runtime,
                &runtime.session.session_id,
                projection.state(),
                None,
                Some(IndexSuffix::Current(&previous.current_cursor)),
                previous.current_history_count,
                &mut records,
                &mut text,
                &mut hashes,
                &previous.scratch,
                &previous.nonce,
                &mut owners,
                &mut semantics_changed,
            )?;
            if !cursor.matches_watermark(length, hash) {
                return Err(fail("Session changed during incremental context indexing"));
            }
            if semantics_changed {
                discarded = (
                    length,
                    (text.stream_position()? - old_text) as usize,
                    (hashes.stream_position()? - old_hashes) as usize,
                    (records.stream_position()? - previous.index_len) as usize,
                );
                return Ok(None);
            }
            records.flush()?;
            text.flush()?;
            hashes.flush()?;
            let mut watermark = blake3::Hasher::new();
            watermark.update(previous.watermark.as_bytes());
            watermark.update(&(length as u64).to_le_bytes());
            watermark.update(hash.as_bytes());
            let indexed_events =
                usize::try_from(cursor.indexed_events() - previous.current_cursor.indexed_events())
                    .map_err(fail)?;
            Ok(Some(Arc::new(Self {
                scratch: Arc::clone(&previous.scratch),
                peers: Arc::clone(&previous.peers),
                peer_count: previous.peer_count,
                peer_manifest: previous.peer_manifest,
                nonce: previous.nonce,
                index_len: records.stream_position()?,
                watermark: watermark.finalize(),
                current_cursor: cursor,
                current_history_count: projection.state().history().items().len(),
                agent_id: previous.agent_id.clone(),
                indexed_events,
                indexed_bytes: length - previous.current_cursor.indexed_bytes(),
                initial_indexed_events: previous.initial_indexed_events,
                initial_indexed_bytes: previous.initial_indexed_bytes,
                preparation_source_bytes: length,
                preparation_text_bytes: (text.stream_position()? - old_text) as usize,
                preparation_hash_bytes: (hashes.stream_position()? - old_hashes) as usize,
                preparation_record_bytes: (records.stream_position()? - previous.index_len)
                    as usize,
                owners: Arc::clone(&previous.owners),
            })))
        })?;
        let updated = match updated {
            Some(index) => index,
            None => {
                let mut index = Self::build(runtime)?;
                index.preparation_source_bytes += discarded.0;
                index.preparation_text_bytes += discarded.1;
                index.preparation_hash_bytes += discarded.2;
                index.preparation_record_bytes += discarded.3;
                Arc::new(index)
            }
        };
        *cached = Some(Arc::clone(&updated));
        Ok(updated)
    }

    pub(in crate::cli_chat) fn acquire(
        session: &super::super::DurableChatSession,
    ) -> Result<Arc<Self>, ChatError> {
        let runtime = ContextReadRuntime::new(session, 0, false)?;
        Self::acquire_runtime(&runtime)
    }

    fn build(runtime: &ContextReadRuntime) -> Result<Self, ChatError> {
        let mut nonce = [0u8; 32];
        ring::rand::SystemRandom::new()
            .fill(&mut nonce)
            .map_err(|_| fail("cannot create context cursor identity"))?;
        let path =
            std::env::temp_dir().join(format!("echidna-context-{}", blake3::hash(&nonce).to_hex()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            fs::DirBuilder::new().mode(0o700).create(&path)?;
        }
        #[cfg(not(unix))]
        fs::create_dir(&path)?;
        let scratch = Arc::new(Scratch(path));
        let peers = Arc::new(PeerDirectory(scratch.0.join("peers")));
        fs::create_dir(&peers.0)?;
        fs::create_dir(scratch.0.join("ordinals"))?;
        fs::create_dir(scratch.0.join("call-owners"))?;
        let mut owners = super::super::task_posting_manifest::PostingManifest::new(
            scratch.0.join("owner-proofs"),
            nonce,
        )?;
        let mut records = File::create(scratch.0.join("records"))?;
        let mut text = File::create(scratch.0.join("text"))?;
        let mut hashes = File::create(scratch.0.join("hashes"))?;
        let mut watermarks = blake3::Hasher::new();
        let mut indexed_events = 0usize;
        let mut indexed_bytes = 0usize;
        let ((length, hash), current_cursor, current_history_count) =
            runtime.session.with_projection(|projection, store, _| {
                let mut semantics_changed = false;
                let watermark = Self::index_session(
                    runtime,
                    &runtime.session.session_id,
                    projection.state(),
                    None,
                    None,
                    0,
                    &mut records,
                    &mut text,
                    &mut hashes,
                    &scratch,
                    &nonce,
                    &mut owners,
                    &mut semantics_changed,
                )?;
                let cursor = store.projection_cursor();
                if !cursor.matches_watermark(watermark.0, watermark.1) {
                    return Err(fail("Session changed during context indexing"));
                }
                Ok((
                    watermark,
                    cursor,
                    projection.state().history().items().len(),
                ))
            })?;
        watermarks.update(runtime.session.session_id.as_str().as_bytes());
        watermarks.update(&(length as u64).to_le_bytes());
        watermarks.update(hash.as_bytes());
        indexed_events += usize::try_from(current_cursor.indexed_events()).map_err(fail)?;
        indexed_bytes += length;
        records.flush()?;
        text.flush()?;
        hashes.flush()?;
        let index_len = records.stream_position()?;
        let (peer_count, peer_manifest) = peer_manifest(&peers, &nonce)?;
        Ok(Self {
            scratch,
            peers,
            peer_count,
            peer_manifest,
            nonce,
            index_len,
            watermark: watermarks.finalize(),
            current_cursor,
            current_history_count,
            agent_id: runtime
                .current_binding()?
                .definition()
                .id()
                .as_str()
                .to_owned(),
            indexed_events,
            indexed_bytes,
            initial_indexed_events: indexed_events,
            initial_indexed_bytes: indexed_bytes,
            preparation_source_bytes: length,
            preparation_text_bytes: text.stream_position()? as usize,
            preparation_hash_bytes: hashes.stream_position()? as usize,
            preparation_record_bytes: index_len as usize,
            owners: Arc::new(std::sync::Mutex::new(owners)),
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn index_session(
        runtime: &ContextReadRuntime,
        id: &crate::domain::events::StreamId,
        state: &SessionState,
        events: Option<&[crate::domain::events::EventEnvelope<serde_json::Value>]>,
        cursor: Option<IndexSuffix<'_>>,
        start_history: usize,
        records: &mut File,
        text: &mut File,
        hashes: &mut File,
        scratch: &Scratch,
        key: &[u8; 32],
        owners: &mut super::super::task_posting_manifest::PostingManifest,
        semantics_changed: &mut bool,
    ) -> Result<(usize, blake3::Hash), ChatError> {
        let mut history = state.history_items_with_event_ids().skip(start_history);
        let mut ordinal = start_history;
        let mut verified_events = events.map(|events| events.iter());
        let path = runtime.session.project.session_stream_path_existing(id)?;
        let mut pending = Vec::new().into_iter();
        let mut next = None;
        let mut write_error = None;
        let is_suffix = cursor.is_some();
        let mut observer = |event: &crate::domain::events::EventEnvelope<serde_json::Value>,
                            raw_offset,
                            raw_len,
                            _,
                            raw: &[u8]| {
            if verified_events
                .as_mut()
                .is_some_and(|events| events.next() != Some(event))
            {
                return Err(crate::adapters::project_event_store::ProjectEventStoreError::ValidatedPrefixChanged { path: path.clone() });
            }
            if is_suffix {
                let decoded = crate::domain::session::SessionEvent::decode(
                    &event.event_type,
                    event.payload.clone(),
                )
                .map_err(|error| {
                    crate::adapters::project_event_store::ProjectEventStoreError::Io {
                        operation: "decode Session source index suffix",
                        path: path.clone(),
                        source: io::Error::other(error),
                    }
                })?;
                match decoded {
                    crate::domain::session::SessionEvent::OperatorTurnBoundaryRecorded {
                        boundary:
                            crate::domain::session::OperatorTurnBoundary::Accepted { inputs, .. },
                    } => {
                        for input in inputs {
                            let old = read_ordinal(
                                scratch,
                                key,
                                id.as_str(),
                                input.event_id().as_str(),
                            )
                            .map_err(|error| {
                                crate::adapters::project_event_store::ProjectEventStoreError::Io {
                                    operation: "read Session source ordinal",
                                    path: path.clone(),
                                    source: io::Error::other(error.to_string()),
                                }
                            })?;
                            if old.is_some_and(|ordinal| ordinal < start_history) {
                                *semantics_changed = true;
                            }
                        }
                    }
                    crate::domain::session::SessionEvent::VisionRouteDecided { .. }
                    | crate::domain::session::SessionEvent::VisionObservationRecorded { .. }
                    | crate::domain::session::SessionEvent::Resumed { .. }
                    | crate::domain::session::SessionEvent::AgentDefinitionApplied { .. }
                    | crate::domain::session::SessionEvent::AgentDefinitionAppliedAtCheckpoint {
                        ..
                    } => *semantics_changed = true,
                    _ => {}
                }
            }
            if next.is_none() {
                next = history.next();
            }
            let Some((history_id, item)) = next else {
                return Ok(());
            };
            if history_id != &event.event_id {
                return Ok(());
            }
            let history_ordinal = ordinal;
            ordinal += 1;
            let result: Result<(), ChatError> = (|| {
                if pending.len() == 0 {
                    let mut batch = vec![(history_id, item)];
                    batch.extend(history.by_ref().take(PAGE_RECORDS - 1));
                    let semantics = state.history_source_semantics_many(&batch)?;
                    pending = batch
                        .into_iter()
                        .zip(semantics)
                        .collect::<Vec<_>>()
                        .into_iter();
                    // The current item was included in the batch.
                }
                let ((expected, expected_item), semantics) = pending
                    .next()
                    .ok_or_else(|| fail("missing indexed source semantics"))?;
                if expected != history_id {
                    return Err(fail("History source ordering changed"));
                }
                let searchable = !matches!(item, ContentItem::ToolCall { call } if call.name.as_str() == "context_read")
                    && !matches!(item, ContentItem::ToolResult { result } if result.tool_name.as_str() == "context_read");
                let hashes_offset = hashes.stream_position()?;
                for segment in raw.chunks(SEGMENT_BYTES) {
                    write_hash(hashes, key, blake3::hash(segment))?;
                }
                let text_hashes_offset = hashes.stream_position()?;
                let text_offset = text.stream_position()?;
                let mut revision_writer = super::SourceRangeWriter {
                    hash: blake3::Hasher::new(),
                    offset: 0,
                    end: PREFIX_BYTES,
                    position: 0,
                    bytes: Vec::new(),
                    stop_at_end: false,
                    stopped: false,
                };
                serde_json::to_writer(&mut revision_writer, expected_item).map_err(fail)?;
                let revision = revision_writer.hash.finalize().to_hex().to_string();
                let (prefix, text_len) = match expected_item {
                    ContentItem::Text { text, .. } => {
                        (&text.as_bytes()[..text.len().min(PREFIX_BYTES)], text.len())
                    }
                    _ => (revision_writer.bytes.as_slice(), revision_writer.position),
                };
                let mut writer = HashWriter {
                    file: &mut *text,
                    hashes: &mut *hashes,
                    hash: blake3::Hasher::new(),
                    len: 0,
                    part_len: 0,
                    key,
                };
                writer.write_all(prefix)?;
                let text_stored_len = writer.len;
                if writer.part_len != 0 {
                    write_hash(writer.hashes, key, writer.hash.finalize())?;
                }
                let role = match expected_item {
                    ContentItem::Text { role, .. } | ContentItem::ImageArtifact { role, .. } => {
                        Some(*role)
                    }
                    _ => None,
                };
                let peer = id != &runtime.session.session_id;
                let reference = if peer {
                    format!("session-revision:{}:{history_id}:{revision}", id.as_str())
                } else {
                    history_id.to_string()
                };
                let entry = Record {
                    session: id.as_str().to_owned(),
                    ordinal: history_ordinal,
                    reference,
                    event_id: history_id.to_string(),
                    searchable,
                    peer,
                    raw_offset: raw_offset as u64,
                    raw_len,
                    hashes_offset,
                    text_hashes_offset,
                    text_offset,
                    text_len,
                    text_stored_len,
                    revision,
                    role,
                    origin: semantics.origin,
                    class: semantics.class,
                    authority: semantics.authority,
                    observed: event.occurred_at.as_str().to_owned(),
                };
                let record_bytes = serde_json::to_vec(&entry).map_err(fail)?;
                let frame = RecordFrame {
                    tag: tag(key, records.stream_position()?, &record_bytes),
                    record: entry,
                };
                serde_json::to_writer(&mut *records, &frame).map_err(fail)?;
                records.write_all(b"\n")?;
                write_ordinal(
                    scratch,
                    key,
                    id.as_str(),
                    history_id.as_str(),
                    history_ordinal,
                )?;
                if let ContentItem::ToolCall { call } = item {
                    write_call_owner(
                        scratch,
                        key,
                        owners,
                        call.call_id.as_str(),
                        id.as_str(),
                        raw_offset + raw_len,
                    )?;
                }
                Ok(())
            })();
            if let Err(error) = result {
                write_error = Some(error);
            }
            next = pending
                .as_slice()
                .first()
                .map(|((id, item), _)| (*id, *item));
            if next.is_none() {
                next = history.next();
            }
            Ok(())
        };
        let watermark = match cursor {
            Some(IndexSuffix::Current(cursor)) => {
                crate::adapters::project_event_store::visit_session_records_after(
                    &runtime.session.project,
                    id,
                    cursor,
                    &mut observer,
                )?
            }
            Some(IndexSuffix::Peer(checkpoint)) => {
                crate::adapters::project_event_store::visit_session_records_after_checkpoint(
                    &runtime.session.project,
                    id,
                    checkpoint,
                    &mut observer,
                )?
            }
            None => crate::adapters::project_event_store::visit_existing_session_records(
                &runtime.session.project,
                id,
                &mut observer,
            )?,
        };
        if verified_events
            .as_mut()
            .is_some_and(|events| events.next().is_some())
        {
            return Err(fail(
                "validated Session source changed during index preparation",
            ));
        }
        if let Some(error) = write_error {
            return Err(error);
        }
        Ok(watermark)
    }
}

impl ColdSearch {
    pub(super) fn build(
        runtime: &ContextReadRuntime,
        query: &str,
        after: Option<u64>,
        before: Option<u64>,
    ) -> Result<Self, ChatError> {
        let previous_index = runtime
            .session
            .context_source_index
            .lock()
            .map_err(|_| io::Error::other("context source index lock poisoned"))?
            .clone();
        let first = ColdIndex::acquire_runtime(runtime)?;
        let mut source_index_work = IndexWork::default();
        let mut indexed_events = 0;
        let mut indexed_bytes = 0;
        if previous_index
            .as_ref()
            .is_none_or(|old| !Arc::ptr_eq(old, &first))
        {
            indexed_events += first.indexed_events;
            indexed_bytes += first.indexed_bytes;
            source_index_work.add(first.index_work());
        }
        let (index, refresh_work) = ColdIndex::refresh_peers_with_work(runtime)?;
        source_index_work.add(refresh_work);
        if !Arc::ptr_eq(&first, &index) {
            indexed_events += index.indexed_events;
            indexed_bytes += index.indexed_bytes;
        }
        let mut watermarks = blake3::Hasher::new();
        watermarks.update(index.watermark.as_bytes());
        let mut cursor_key = [0u8; 32];
        ring::rand::SystemRandom::new()
            .fill(&mut cursor_key)
            .map_err(|_| fail("cannot create context search identity"))?;
        let auxiliary = AuxiliaryFile(
            index
                .scratch
                .0
                .join(format!("aux-{}", blake3::hash(&cursor_key).to_hex())),
        );
        let mut auxiliary_file = File::create(&auxiliary.0)?;
        let auxiliary_text = AuxiliaryFile(auxiliary.0.with_extension("text"));
        let auxiliary_hashes = AuxiliaryFile(auxiliary.0.with_extension("hashes"));
        let record_text = AuxiliaryFile(auxiliary.0.with_extension("record-text"));
        let record_hashes = AuxiliaryFile(auxiliary.0.with_extension("record-hashes"));
        let mut memory_text = File::create(&auxiliary_text.0)?;
        let mut memory_hashes = File::create(&auxiliary_hashes.0)?;
        let mut auxiliary_preparation_memory_read_bytes = 0;
        runtime.visit_memory_sources(|store, path| {
            let source = store
                .visit_context_content(path, |_| Ok(()))
                .map_err(io::Error::other)?;
            let Some((revision, total_bytes)) = source else {
                return Ok(());
            };
            auxiliary_preparation_memory_read_bytes += total_bytes;
            let reference = format!("memory:{}:{revision}", path.as_str());
            watermarks.update(reference.as_bytes());
            watermarks.update(revision.as_bytes());
            let record = AuxiliaryRecord {
                reference,
                scope: "memory".to_owned(),
                revision,
                observed_at: None,
                text: String::new(),
                offset: 0,
                total_bytes,
                memory_text_offset: 0,
                memory_hashes_offset: 0,
                metadata_len: 0,
                metadata_hash: [0; 32],
            };
            let encoded = serde_json::to_vec(&record).map_err(io::Error::other)?;
            let frame = AuxiliaryFrame {
                tag: tag(&cursor_key, auxiliary_file.stream_position()?, &encoded),
                record,
            };
            serde_json::to_writer(&mut auxiliary_file, &frame).map_err(io::Error::other)?;
            auxiliary_file.write_all(b"\n")?;
            Ok(())
        })?;
        let ambiguous_time = runtime.visit_auxiliary_sources(&index, after, before, |source| {
            watermarks.update(source.reference.as_bytes());
            watermarks.update(source.revision.as_bytes());
            let memory_text_offset = memory_text.stream_position()?;
            let memory_hashes_offset = memory_hashes.stream_position()?;
            let mut writer = HashWriter {
                file: &mut memory_text,
                hashes: &mut memory_hashes,
                hash: blake3::Hasher::new(),
                len: 0,
                part_len: 0,
                key: &cursor_key,
            };
            writer.write_all(source.text.as_bytes())?;
            if writer.part_len != 0 {
                write_hash(writer.hashes, &cursor_key, writer.hash.finalize())?;
            }
            let record = AuxiliaryRecord {
                reference: source.reference,
                scope: source.scope.to_owned(),
                revision: source.revision,
                observed_at: source.observed_at,
                metadata_len: source.text.len(),
                metadata_hash: *blake3::hash(source.text.as_bytes()).as_bytes(),
                text: String::new(),
                offset: source.available_offset,
                total_bytes: source.total_bytes,
                memory_text_offset,
                memory_hashes_offset,
            };
            let encoded = serde_json::to_vec(&record).map_err(fail)?;
            let frame = AuxiliaryFrame {
                tag: tag(&cursor_key, auxiliary_file.stream_position()?, &encoded),
                record,
            };
            serde_json::to_writer(&mut auxiliary_file, &frame).map_err(fail)?;
            auxiliary_file.write_all(b"\n")?;
            Ok(())
        })?;
        auxiliary_file.flush()?;
        memory_text.flush()?;
        memory_hashes.flush()?;
        let auxiliary_len = auxiliary_file.stream_position()?;
        let auxiliary_preparation_write_bytes = usize::try_from(auxiliary_len).map_err(fail)?
            + usize::try_from(memory_text.stream_position()?).map_err(fail)?
            + usize::try_from(memory_hashes.stream_position()?).map_err(fail)?;
        let binding = runtime.current_binding()?;
        let definition_digest = binding.definition().digest().as_str().to_owned();
        let scope = format!(
            "context-read-v1:{}:{}:{}",
            runtime.session.project.project_id().as_str(),
            definition_digest,
            watermarks.finalize().to_hex()
        );
        let initial_indexed_events = index.initial_indexed_events;
        let initial_indexed_bytes = index.initial_indexed_bytes;
        Ok(Self {
            index,
            cursor_key,
            query: query.to_owned(),
            after,
            before,
            token: String::new(),
            scope,
            definition_digest,
            generation: 0,
            index_offset: 0,
            record: None,
            raw_part: 0,
            text_position: 0,
            tail: Vec::new(),
            auxiliary,
            auxiliary_text,
            auxiliary_hashes,
            record_text,
            record_hashes,
            record_materialized: false,
            auxiliary_offset: 0,
            auxiliary_len,
            auxiliary_record: None,
            auxiliary_position: 0,
            auxiliary_tail: Vec::new(),
            auxiliary_metadata: false,
            ambiguous_time,
            indexed_events,
            indexed_bytes,
            initial_indexed_events,
            initial_indexed_bytes,
            auxiliary_preparation_memory_read_bytes,
            auxiliary_preparation_write_bytes,
            source_index_work,
        })
    }

    pub(super) fn belongs_to_definition(&self, digest: &str) -> bool {
        self.definition_digest == digest
    }

    pub(super) fn matches(
        &self,
        runtime: &ContextReadRuntime,
        query: &str,
        after: Option<u64>,
        before: Option<u64>,
        cursor: &str,
    ) -> Result<bool, ChatError> {
        Ok(self.query == query
            && self.after == after
            && self.before == before
            && self.token == cursor
            && self
                .belongs_to_definition(runtime.current_binding()?.definition().digest().as_str()))
    }

    pub(super) fn page(
        &mut self,
        runtime: &ContextReadRuntime,
        maximum: usize,
    ) -> Result<ContextReadResponse, ChatError> {
        let mut index = BufReader::with_capacity(
            INDEX_BUFFER_BYTES,
            MeteredFile {
                file: File::open(self.index.scratch.0.join("records"))?,
                read_bytes: 0,
            },
        );
        index.seek(SeekFrom::Start(self.index_offset))?;
        let mut auxiliary = BufReader::with_capacity(
            INDEX_BUFFER_BYTES,
            MeteredFile {
                file: File::open(&self.auxiliary.0)?,
                read_bytes: 0,
            },
        );
        auxiliary.seek(SeekFrom::Start(self.auxiliary_offset))?;
        let mut memory_text = File::open(&self.auxiliary_text.0)?;
        let mut memory_hashes = File::open(&self.auxiliary_hashes.0)?;
        let mut hashes = File::open(self.index.scratch.0.join("hashes"))?;
        let mut sources = Vec::new();
        let mut bytes = 0usize;
        let mut source_verification_bytes = 0usize;
        let mut source_materialization_read_bytes = 0usize;
        let mut source_materialization_write_bytes = 0usize;
        let mut retrieval_work = crate::domain::retrieval::TaskRetrievalWork::default();
        let mut records = 0usize;
        let mut complete = false;
        let mut remaining = maximum;
        while (self.auxiliary_record.is_some() || self.auxiliary_offset < self.auxiliary_len)
            && sources.len() < 10
            && remaining > 0
            && records < PAGE_RECORDS
        {
            let used = bytes + index.get_ref().read_bytes + auxiliary.get_ref().read_bytes;
            if PAGE_BYTES - used <= INDEX_BUFFER_BYTES {
                break;
            }
            if self.auxiliary_record.is_none() {
                let mut line = String::new();
                let capacity = PAGE_BYTES - used - INDEX_BUFFER_BYTES;
                let amount = (&mut auxiliary)
                    .take(capacity as u64)
                    .read_line(&mut line)?;
                if amount == 0 {
                    return Err(fail("derived auxiliary index was truncated"));
                }
                if !line.ends_with('\n') {
                    if amount == capacity
                        && self.auxiliary_offset + (amount as u64) < self.auxiliary_len
                        && used > 0
                    {
                        break;
                    }
                    return Err(fail(
                        "derived auxiliary index was truncated or exceeds one page",
                    ));
                }
                let frame: AuxiliaryFrame = serde_json::from_str(&line).map_err(fail)?;
                if frame.tag
                    != tag(
                        &self.cursor_key,
                        self.auxiliary_offset,
                        &serde_json::to_vec(&frame.record).map_err(fail)?,
                    )
                {
                    return Err(fail("derived auxiliary index record changed"));
                }
                self.auxiliary_offset = auxiliary.stream_position()?;
                if self.auxiliary_offset > self.auxiliary_len {
                    return Err(fail("derived auxiliary index length changed"));
                }
                self.auxiliary_metadata = frame.record.scope == "artifact_metadata";
                self.auxiliary_record = Some(frame.record);
                self.auxiliary_position = 0;
                self.auxiliary_tail.clear();
            }
            let record = self
                .auxiliary_record
                .as_ref()
                .expect("auxiliary record loaded");
            let previous_auxiliary_position = self.auxiliary_position;
            let previous_auxiliary_tail = self.auxiliary_tail.clone();
            let (value, start) = {
                let source_len = if self.auxiliary_metadata {
                    record.metadata_len
                } else {
                    record.total_bytes
                };
                let length = (source_len - self.auxiliary_position).min(SEGMENT_BYTES);
                if length == 0 {
                    if record.scope == "memory"
                        && match_offset(
                            runtime.session.project.project_id().as_str(),
                            &record.reference,
                            &self.query,
                            remaining,
                            &mut retrieval_work,
                        )?
                        .is_some()
                    {
                        let (source, verified) =
                            verified_auxiliary(runtime, &self.index, record.clone(), 0)?;
                        source_verification_bytes += verified;
                        if self.after.is_some() || self.before.is_some() {
                            self.ambiguous_time = true;
                        } else {
                            sources.push(source);
                        }
                    }
                    if record.scope == "memory" || self.auxiliary_metadata {
                        self.auxiliary_record = None;
                        records += 1;
                    } else {
                        self.auxiliary_metadata = true;
                        self.auxiliary_position = 0;
                        self.auxiliary_tail.clear();
                    }
                    continue;
                }
                let used = bytes + index.get_ref().read_bytes + auxiliary.get_ref().read_bytes;
                let hash_bytes = if self.auxiliary_metadata { 64 } else { 0 };
                if used + length + hash_bytes > PAGE_BYTES {
                    break;
                }
                let chunk = if record.scope == "memory" {
                    let (path, revision) = record
                        .reference
                        .strip_prefix("memory:")
                        .and_then(|value| value.rsplit_once(':'))
                        .ok_or_else(|| fail("invalid Memory reference"))?;
                    let (historical, current) = runtime.memory_source_bytes(
                        path,
                        revision,
                        &record.reference,
                        self.auxiliary_position,
                        length,
                    )?;
                    source_verification_bytes +=
                        historical.as_ref().map_or(0, |source| source.total_bytes)
                            + current.as_ref().map_or(0, |source| source.total_bytes);
                    let source = historical
                        .or(current.filter(|source| source.revision == revision))
                        .ok_or_else(|| fail("Memory source revision is unavailable"))?;
                    if source.total_bytes != record.total_bytes || source.bytes.len() != length {
                        return Err(fail("Memory source range changed"));
                    }
                    bytes += source.bytes.len();
                    source.bytes
                } else if self.auxiliary_metadata {
                    memory_text.seek(SeekFrom::Start(
                        record.memory_text_offset + self.auxiliary_position as u64,
                    ))?;
                    let mut chunk = vec![0; length];
                    memory_text.read_exact(&mut chunk)?;
                    let expected = read_hash(
                        &mut memory_hashes,
                        &self.cursor_key,
                        record.memory_hashes_offset
                            + ((self.auxiliary_position / SEGMENT_BYTES) * 64) as u64,
                    )?;
                    if blake3::hash(&chunk) != expected {
                        return Err(fail("derived auxiliary source cache changed"));
                    }
                    bytes += chunk.len() + 64;
                    chunk
                } else {
                    let blob =
                        crate::domain::artifacts::BlobId::new(&record.revision).map_err(fail)?;
                    let (chunk, total, _) = runtime
                        .session
                        .artifacts
                        .read_artifact_range(&blob, self.auxiliary_position as u64, length)
                        .map_err(fail)?;
                    if total != record.total_bytes as u64 || chunk.len() != length {
                        return Err(fail("Artifact size differs from indexed source"));
                    }
                    source_verification_bytes += usize::try_from(total).map_err(fail)?;
                    bytes += chunk.len();
                    chunk
                };
                let start = self.auxiliary_position - self.auxiliary_tail.len();
                let mut combined = std::mem::take(&mut self.auxiliary_tail);
                combined.extend_from_slice(&chunk);
                self.auxiliary_position += chunk.len();
                let keep = crate::domain::retrieval::task_query_source_span_bound(&self.query)
                    .saturating_add(512)
                    .min(combined.len());
                self.auxiliary_tail
                    .extend_from_slice(&combined[combined.len() - keep..]);
                let valid_start = combined
                    .iter()
                    .position(|byte| byte & 0xc0 != 0x80)
                    .unwrap_or(combined.len());
                (
                    super::range_text(combined[valid_start..].to_vec())?,
                    start + valid_start,
                )
            };
            let mut found = match_offset(
                runtime.session.project.project_id().as_str(),
                &value,
                &self.query,
                remaining,
                &mut retrieval_work,
            )?;
            if found.is_none()
                && self.auxiliary_position <= SEGMENT_BYTES
                && match_offset(
                    runtime.session.project.project_id().as_str(),
                    &record.reference,
                    &self.query,
                    remaining,
                    &mut retrieval_work,
                )?
                .is_some()
            {
                found = Some(0);
            }
            if let Some(local) = found {
                let excerpt = local;
                let mut end = value.len().min(excerpt.saturating_add(remaining));
                while !value.is_char_boundary(end) {
                    end -= 1;
                }
                if end == excerpt {
                    if sources.is_empty() {
                        return Err(fail(
                            "context read range is too small for the next UTF-8 character",
                        ));
                    }
                    self.auxiliary_position = previous_auxiliary_position;
                    self.auxiliary_tail = previous_auxiliary_tail;
                    break;
                }
                let mut selected = record.clone();
                selected.offset = start + excerpt;
                selected.text = value[excerpt..end].to_owned();
                if self.auxiliary_metadata {
                    selected.scope = "artifact_metadata".to_owned();
                    selected.total_bytes = record.metadata_len;
                    let original = selected
                        .reference
                        .strip_prefix("artifact:")
                        .ok_or_else(|| fail("invalid Artifact reference"))?;
                    let revision = blake3::Hash::from_bytes(record.metadata_hash)
                        .to_hex()
                        .to_string();
                    selected.reference = format!("artifact-metadata:{original}:{revision}");
                    selected.revision = revision;
                }
                let selected_size = selected.text.len();
                // The cursor's scan remains bound to its original snapshot,
                // but ownership must reflect current Session facts at the hit.
                let verification_index = if selected.scope == "memory" {
                    Arc::clone(&self.index)
                } else {
                    let previous = runtime
                        .session
                        .context_source_index
                        .lock()
                        .map_err(|_| io::Error::other("context source index lock poisoned"))?
                        .clone();
                    let current = ColdIndex::acquire_runtime(runtime)?;
                    if previous
                        .as_ref()
                        .is_none_or(|old| !Arc::ptr_eq(old, &current))
                    {
                        self.source_index_work.add(current.index_work());
                        self.indexed_events += current.indexed_events;
                        self.indexed_bytes += current.indexed_bytes;
                    }
                    let (refreshed, work) = ColdIndex::refresh_peers_with_work(runtime)?;
                    self.source_index_work.add(work);
                    if !Arc::ptr_eq(&current, &refreshed) {
                        self.indexed_events += refreshed.indexed_events;
                        self.indexed_bytes += refreshed.indexed_bytes;
                    }
                    refreshed
                };
                let (source, verified) =
                    verified_auxiliary(runtime, &verification_index, selected, selected_size)?;
                source_verification_bytes += verified;
                let unknown_time = source
                    .observed_at
                    .as_deref()
                    .and_then(crate::app::context_read::unix_seconds)
                    .is_none();
                if (self.after.is_some() || self.before.is_some()) && unknown_time {
                    self.ambiguous_time = true;
                } else {
                    remaining -= source.text.len();
                    sources.push(source);
                }
                self.auxiliary_record = None;
                records += 1;
            } else if self.auxiliary_position
                >= if self.auxiliary_metadata {
                    record.metadata_len
                } else {
                    record.total_bytes
                }
            {
                if record.scope == "memory" || self.auxiliary_metadata {
                    self.auxiliary_record = None;
                    records += 1;
                } else {
                    self.auxiliary_metadata = true;
                    self.auxiliary_position = 0;
                    self.auxiliary_tail.clear();
                }
            }
        }
        while self.auxiliary_record.is_none()
            && self.auxiliary_offset == self.auxiliary_len
            && remaining > 0
            && bytes + index.get_ref().read_bytes + auxiliary.get_ref().read_bytes < PAGE_BYTES
            && records < PAGE_RECORDS
            && sources.len() < 10
        {
            if self.record.is_none() {
                if self.index_offset == self.index.index_len {
                    complete = true;
                    break;
                }
                let mut line = String::new();
                let available = PAGE_BYTES
                    - bytes
                    - index.get_ref().read_bytes
                    - auxiliary.get_ref().read_bytes;
                if available <= INDEX_BUFFER_BYTES {
                    break;
                }
                let amount = (&mut index)
                    .take((available - INDEX_BUFFER_BYTES) as u64)
                    .read_line(&mut line)?;
                if amount == 0 {
                    return Err(fail("derived context index was truncated"));
                }
                if !line.ends_with('\n') {
                    return Err(fail("context index record exceeds page budget"));
                }
                let frame: RecordFrame = serde_json::from_str(&line).map_err(fail)?;
                if frame.tag
                    != tag(
                        &self.index.nonce,
                        self.index_offset,
                        &serde_json::to_vec(&frame.record).map_err(fail)?,
                    )
                {
                    return Err(fail("derived context index record changed"));
                }
                self.index_offset = index.stream_position()?;
                if self.index_offset > self.index.index_len {
                    return Err(fail("derived context index length changed"));
                }
                self.record = Some(frame.record);
                self.record_materialized = false;
                self.raw_part = 0;
                self.text_position = 0;
                self.tail.clear();
            }
            let record = self.record.as_ref().expect("record loaded above");
            if !record.searchable {
                self.record = None;
                records += 1;
                continue;
            }
            if self.raw_part < record.raw_len {
                let length = (record.raw_len - self.raw_part).min(SEGMENT_BYTES);
                if bytes + index.get_ref().read_bytes + auxiliary.get_ref().read_bytes + length + 64
                    > PAGE_BYTES
                {
                    break;
                }
                let expected = read_hash(
                    &mut hashes,
                    &self.index.nonce,
                    record.hashes_offset + ((self.raw_part / SEGMENT_BYTES) * 64) as u64,
                )?;
                crate::adapters::project_event_store::read_indexed_session_segment(
                    &runtime.session.project,
                    &crate::domain::events::StreamId::new(&record.session),
                    record.raw_offset + self.raw_part as u64,
                    length,
                    expected,
                )?;
                bytes += length + 64;
                self.raw_part += length;
                continue;
            }
            let observed = crate::app::context_read::unix_seconds(&record.observed);
            if observed.is_some_and(|value| {
                self.after.is_some_and(|after| value < after)
                    || self.before.is_some_and(|before| value > before)
            }) {
                self.record = None;
                records += 1;
                continue;
            }
            if record.text_len == 0 {
                self.record = None;
                records += 1;
                continue;
            }
            let text_key = record_text_key(&self.cursor_key, record)?;
            if !self.record_materialized {
                let path = runtime.session.project.session_stream_path_existing(
                    &crate::domain::events::StreamId::new(&record.session),
                )?;
                let mut source = BufReader::new(MeteredFile {
                    file: File::open(path)?,
                    read_bytes: 0,
                });
                source.seek(SeekFrom::Start(record.raw_offset))?;
                let event: crate::domain::events::EventEnvelope<serde_json::Value> =
                    serde_json::from_reader((&mut source).take(record.raw_len as u64))
                        .map_err(fail)?;
                source_materialization_read_bytes += source.get_ref().read_bytes;
                if event.stream_id.as_str() != format!("session:{}", record.session)
                    || event.event_id.as_str() != record.event_id
                {
                    return Err(fail("Session source identity changed"));
                }
                let event = crate::domain::events::EventUpcasterRegistry::current()
                    .upcast(event)
                    .map_err(fail)?;
                let item = match crate::domain::session::SessionEvent::decode(
                    &event.event_type,
                    event.payload,
                )? {
                    crate::domain::session::SessionEvent::HistoryItemRecorded { item } => item,
                    _ => return Err(fail("Session source is not a History record")),
                };
                let mut revision = super::SourceRangeWriter {
                    hash: blake3::Hasher::new(),
                    offset: 0,
                    end: 0,
                    position: 0,
                    bytes: Vec::new(),
                    stop_at_end: false,
                    stopped: false,
                };
                serde_json::to_writer(&mut revision, &item).map_err(fail)?;
                if revision.hash.finalize().to_hex().as_str() != record.revision {
                    return Err(fail("Session source revision changed"));
                }
                let mut text = File::create(&self.record_text.0)?;
                let mut text_hashes = File::create(&self.record_hashes.0)?;
                let mut writer = HashWriter {
                    file: &mut text,
                    hashes: &mut text_hashes,
                    hash: blake3::Hasher::new(),
                    len: 0,
                    part_len: 0,
                    key: &text_key,
                };
                match &item {
                    ContentItem::Text { text, .. } => writer.write_all(text.as_bytes())?,
                    _ => serde_json::to_writer(&mut writer, &item).map_err(fail)?,
                }
                if writer.len != record.text_len {
                    return Err(fail("Session source length changed"));
                }
                if writer.part_len != 0 {
                    write_hash(writer.hashes, &text_key, writer.hash.finalize())?;
                }
                source_materialization_write_bytes +=
                    text.stream_position()? as usize + text_hashes.stream_position()? as usize;
                let (source, hashes) =
                    self.index.verify_record(&runtime.session.project, record)?;
                source_verification_bytes += source + hashes;
                self.record_materialized = true;
            }
            let length = (record.text_len - self.text_position).min(SEGMENT_BYTES);
            if bytes + index.get_ref().read_bytes + auxiliary.get_ref().read_bytes + length + 64
                > PAGE_BYTES
            {
                break;
            }
            let mut text = File::open(&self.record_text.0)?;
            let mut text_hashes = File::open(&self.record_hashes.0)?;
            text.seek(SeekFrom::Start(self.text_position as u64))?;
            let mut chunk = vec![0; length];
            text.read_exact(&mut chunk)?;
            let expected = read_hash(
                &mut text_hashes,
                &text_key,
                ((self.text_position / SEGMENT_BYTES) * 64) as u64,
            )?;
            if blake3::hash(&chunk) != expected {
                return Err(fail("derived context source cache changed"));
            }
            bytes += length + 64;
            let start = self.text_position - self.tail.len();
            let previous_tail = self.tail.clone();
            let mut combined = std::mem::take(&mut self.tail);
            combined.extend_from_slice(&chunk);
            let valid_start = combined
                .iter()
                .position(|byte| byte & 0xc0 != 0x80)
                .unwrap_or(combined.len());
            let value = super::range_text(combined[valid_start..].to_vec())?;
            if let Some(local) = match_offset(
                runtime.session.project.project_id().as_str(),
                &value,
                &self.query,
                remaining,
                &mut retrieval_work,
            )? {
                // Earlier raw segments may have changed while this search was
                // suspended between pages. Recheck the whole source before a
                // hit; this is source-integrity IO, separate from page scan.
                let (source_bytes, hash_bytes) =
                    self.index.verify_record(&runtime.session.project, record)?;
                source_verification_bytes += source_bytes + hash_bytes;
                let local_start = local;
                let excerpt_start = start + valid_start + local_start;
                let mut end = value.len().min(local_start.saturating_add(remaining));
                while !value.is_char_boundary(end) {
                    end -= 1;
                }
                if end == local_start {
                    if sources.is_empty() {
                        return Err(fail(
                            "context read range is too small for the next UTF-8 character",
                        ));
                    }
                    self.tail = previous_tail;
                    break;
                }
                remaining -= end - local_start;
                sources.push(ContextSource {
                    reference: record.reference.clone(),
                    scope: if record.peer {
                        "agent_session"
                    } else {
                        "session"
                    },
                    revision: record.revision.clone(),
                    role: record.role,
                    semantics: HistorySourceSemantics {
                        origin: record.origin,
                        class: record.class,
                        authority: record.authority,
                    },
                    observed_at: Some(record.observed.clone()),
                    text: value[local_start..end].to_owned(),
                    searchable: true,
                    available_offset: excerpt_start,
                    total_bytes: record.text_len,
                });
                self.record = None;
                records += 1;
            } else {
                self.text_position += length;
                let keep = crate::domain::retrieval::task_query_source_span_bound(&self.query)
                    .saturating_add(512)
                    .min(combined.len());
                self.tail
                    .extend_from_slice(&combined[combined.len() - keep..]);
                if self.text_position >= record.text_len {
                    self.record = None;
                    records += 1;
                }
            }
        }
        let mut response = crate::app::context_read::read(
            ContextReadRequest::Search {
                query: self.query.clone(),
                after_unix: self.after,
                before_unix: self.before,
                max_bytes: maximum,
                cursor: None,
            },
            ContextSources {
                history: sources,
                memory: None,
                progress: None,
                more: !complete,
                ambiguous_time: self.ambiguous_time,
            },
        )
        .map_err(fail)?;
        if !complete {
            self.generation += 1;
            let cursor = serde_json::to_vec(&(
                self.scope.as_str(),
                self.query.as_str(),
                self.after,
                self.before,
                self.generation,
                self.auxiliary_offset,
                self.auxiliary_position,
                self.auxiliary_metadata,
                self.index_offset,
                self.raw_part,
                self.text_position,
            ))
            .map_err(fail)?;
            self.token = blake3::keyed_hash(&self.cursor_key, &cursor)
                .to_hex()
                .to_string();
            response.next_cursor = Some(self.token.clone());
            response.truncated = true;
        }
        super::super::timing::context_read_work(
            runtime.diagnostics,
            &super::super::timing::ContextReadWork {
                scan_bytes: bytes + index.get_ref().read_bytes + auxiliary.get_ref().read_bytes,
                records,
                rank_window_checks: retrieval_work.rank_window_checks,
                retrieval_source_window_bytes: retrieval_work.source_window_bytes,
                retrieval_index_source_bytes: retrieval_work.index_source_bytes,
                retrieval_needle_scan_bytes: retrieval_work.needle_scan_bytes,
                retrieval_candidate_anchors: retrieval_work.candidate_anchors,
                retrieval_local_gram_probes: retrieval_work.local_gram_probes,
                retrieval_window_gram_probes: retrieval_work.window_gram_probes,
                retrieval_rank_window_bytes: retrieval_work.rank_window_bytes,
                source_verification_bytes,
                source_materialization_read_bytes,
                source_materialization_write_bytes,
                source_index_scan_bytes: self.source_index_work.source_scan_bytes,
                source_index_text_write_bytes: self.source_index_work.text_write_bytes,
                source_index_hash_write_bytes: self.source_index_work.hash_write_bytes,
                source_index_record_write_bytes: self.source_index_work.record_write_bytes,
                auxiliary_preparation_memory_read_bytes: self
                    .auxiliary_preparation_memory_read_bytes,
                auxiliary_preparation_write_bytes: self.auxiliary_preparation_write_bytes,
                indexed_events: self.indexed_events,
                indexed_bytes: self.indexed_bytes,
                initial_indexed_events: self.initial_indexed_events,
                initial_indexed_bytes: self.initial_indexed_bytes,
            },
        );
        self.indexed_events = 0;
        self.indexed_bytes = 0;
        self.initial_indexed_events = 0;
        self.initial_indexed_bytes = 0;
        self.auxiliary_preparation_memory_read_bytes = 0;
        self.auxiliary_preparation_write_bytes = 0;
        self.source_index_work = IndexWork::default();
        Ok(response)
    }
}
