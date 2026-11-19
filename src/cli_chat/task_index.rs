//! Disposable source-bound task navigation. Postings, cards and anchors
//! live on disk; process-local counts and cursors support incremental access.

use ring::rand::SecureRandom;
use std::cell::Cell;
use std::collections::{BTreeMap, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;

use super::task_postings::PackedPostings;
use super::{ChatError, DurableChatSession, context_read::index::ColdIndex};
use crate::domain::session::{
    SessionError, SessionState, TaskCardSource, TaskNavigationAccess, TaskNavigationChange,
    TaskNavigationCursor,
};
use crate::domain::task_context::TaskCandidate;

const ENTRY_BYTES: u64 = 8;
const ANCHOR_BYTES: u64 = 16;
const POSTING_BATCH_ROWS: usize = 65_536;

#[derive(Clone, Copy, Default)]
pub(super) struct TaskIndexStats {
    pub(super) source_rows: usize,
    pub(super) source_prefix_bytes: usize,
    pub(super) source_verification_bytes: usize,
    pub(super) source_index_read_bytes: usize,
    pub(super) source_index_text_write_bytes: usize,
    pub(super) source_index_hash_write_bytes: usize,
    pub(super) source_index_record_write_bytes: usize,
    pub(super) posting_rows_read: usize,
    pub(super) posting_bytes_read: usize,
    pub(super) posting_rows_written: usize,
    pub(super) posting_bytes_written: usize,
    pub(super) cache_rows_written: usize,
    pub(super) cache_bytes_written: usize,
    pub(super) card_rows_read: usize,
    pub(super) card_bytes_read: usize,
    pub(super) task_ref_rows_read: usize,
    pub(super) task_ref_bytes_read: usize,
    pub(super) anchor_rows_read: usize,
    pub(super) anchor_bytes_read: usize,
    pub(super) semantics_rows_read: usize,
    pub(super) semantics_bytes_read: usize,
    pub(super) cards_written: usize,
    pub(super) anchors_written: usize,
    pub(super) navigation_stage_read_bytes: usize,
    pub(super) navigation_stage_write_bytes: usize,
    pub(super) metadata_rebuilt: bool,
    pub(super) posting_keys: usize,
    pub(super) posting_batch_peak_rows: usize,
    pub(super) posting_batches: usize,
    pub(super) posting_file_opens: usize,
    pub(super) posting_file_bytes: u64,
    pub(super) posting_pending_bytes: usize,
}

pub(super) struct TaskIndex {
    directory: PathBuf,
    source_identity: blake3::Hash,
    source_watermark: Option<blake3::Hash>,
    offset: u64,
    count: usize,
    last_reference: Option<String>,
    cursor: TaskNavigationCursor,
    initialized: bool,
    card_count: usize,
    anchor_count: usize,
    last_card: Option<usize>,
    last_anchor: Option<usize>,
    recent: VecDeque<usize>,
    stats: Cell<TaskIndexStats>,
    postings: PackedPostings,
}

impl Drop for TaskIndex {
    fn drop(&mut self) {
        if let Err(error) = self.postings.close() {
            eprintln!("Task posting maintenance failed: {error}");
        }
        let _ = fs::remove_dir_all(&self.directory);
    }
}

fn fail(message: impl ToString) -> ChatError {
    ChatError::CompactionSummarySource {
        message: message.to_string(),
    }
}
fn domain_error(error: impl ToString) -> SessionError {
    SessionError::TaskContext {
        message: format!("task index: {}", error.to_string()),
    }
}

impl TaskIndex {
    fn new(identity: blake3::Hash) -> Result<Self, ChatError> {
        let mut key = [0; 32];
        ring::rand::SystemRandom::new()
            .fill(&mut key)
            .map_err(|_| fail("cannot create task index identity"))?;
        let directory = std::env::temp_dir().join(format!(
            "echidna-task-index-{}",
            blake3::hash(&key).to_hex()
        ));
        fs::create_dir(&directory)?;
        for name in ["card-data", "task-ref", "semantics"] {
            fs::create_dir(directory.join(name))?;
        }
        let postings = PackedPostings::new(&directory.join("postings"))?;
        Ok(Self {
            directory,
            source_identity: identity,
            source_watermark: None,
            offset: 0,
            count: 0,
            last_reference: None,
            cursor: TaskNavigationCursor::default(),
            initialized: false,
            card_count: 0,
            anchor_count: 0,
            last_card: None,
            last_anchor: None,
            recent: VecDeque::new(),
            stats: Cell::new(TaskIndexStats::default()),
            postings,
        })
    }

    fn measure(&self, update: impl FnOnce(&mut TaskIndexStats)) {
        let mut stats = self.stats.get();
        update(&mut stats);
        self.stats.set(stats);
    }

    pub(super) fn stats(&self) -> Result<TaskIndexStats, ChatError> {
        let mut stats = self.stats.get();
        stats.posting_keys = self.postings.len();
        stats.posting_file_bytes = self.postings.published_bytes()?;
        stats.posting_pending_bytes = self.postings.pending_bytes(stats.posting_file_bytes);
        Ok(stats)
    }

    pub(super) fn update(
        &mut self,
        source: &ColdIndex,
        session: &DurableChatSession,
    ) -> Result<(), ChatError> {
        let (watermark, read, text, hashes, records) = source.preparation_work();
        if self.source_watermark.is_none() {
            self.measure(|stats| stats.posting_file_opens = 2);
        }
        if self.source_watermark != Some(watermark) {
            self.measure(|stats| {
                stats.source_index_read_bytes = read;
                stats.source_index_text_write_bytes = text;
                stats.source_index_hash_write_bytes = hashes;
                stats.source_index_record_write_bytes = records;
            });
            self.source_watermark = Some(watermark);
        }
        let mut postings = BTreeMap::<[u8; 32], Vec<usize>>::new();
        let mut batch_rows = 0;
        self.offset = source.visit_records(self.offset, |record| {
            if record.session != session.session_id.as_str() {
                return Ok(());
            }
            if record.ordinal != self.count {
                return Err(fail("task index History order changed"));
            }
            let (text, _, verified_bytes) =
                source.verified_prefix(&session.project, &record, 1024)?;
            self.measure(|stats| {
                stats.source_rows += 1;
                stats.source_prefix_bytes += text.len();
                stats.source_verification_bytes += verified_bytes;
            });
            let semantics = serde_json::to_vec(&(record.origin, record.class, record.authority))
                .map_err(fail)?;
            let mut file = OpenOptions::new().write(true).create_new(true).open(
                self.directory
                    .join("semantics")
                    .join(record.ordinal.to_string()),
            )?;
            file.write_all(&semantics)?;
            self.measure(|stats| {
                stats.cache_rows_written += 1;
                stats.cache_bytes_written += semantics.len();
            });
            for term in crate::domain::retrieval::task_posting_keys(&text) {
                postings
                    .entry(*blake3::hash(term.as_bytes()).as_bytes())
                    .or_default()
                    .push(record.ordinal);
                batch_rows += 1;
                if batch_rows == POSTING_BATCH_ROWS {
                    self.flush_postings(&mut postings)?;
                    batch_rows = 0;
                }
            }
            self.count += 1;
            self.last_reference = Some(record.reference);
            Ok(())
        })?;
        self.flush_postings(&mut postings)
    }

    fn flush_postings(
        &mut self,
        postings: &mut BTreeMap<[u8; 32], Vec<usize>>,
    ) -> Result<(), ChatError> {
        let rows = postings.values().map(Vec::len).sum();
        self.measure(|stats| {
            stats.posting_batch_peak_rows = stats.posting_batch_peak_rows.max(rows);
            stats.posting_batches += usize::from(rows != 0);
        });
        for (digest, ordinals) in std::mem::take(postings) {
            self.append_postings("history", blake3::Hash::from_bytes(digest), &ordinals)?;
        }
        self.postings.flush().map_err(ChatError::from)
    }

    fn append_posting(
        &mut self,
        directory: &str,
        digest: blake3::Hash,
        ordinal: usize,
    ) -> Result<(), ChatError> {
        self.append_postings(directory, digest, &[ordinal])
    }

    fn append_postings(
        &mut self,
        directory: &str,
        digest: blake3::Hash,
        ordinals: &[usize],
    ) -> Result<(), ChatError> {
        self.postings.append(directory, digest, ordinals)?;
        self.measure(|stats| {
            stats.posting_rows_written += ordinals.len();
            stats.posting_bytes_written += ordinals.len() * ENTRY_BYTES as usize;
        });
        Ok(())
    }

    fn ordinal(
        &self,
        directory: &str,
        digest: blake3::Hash,
        position: u64,
    ) -> Result<usize, ChatError> {
        let ordinal = self.postings.ordinal(directory, digest, position)?;
        self.measure(|stats| {
            stats.posting_rows_read += 1;
            stats.posting_bytes_read += ENTRY_BYTES as usize;
        });
        Ok(ordinal)
    }

    fn posting_at(
        &self,
        directory: &str,
        digest: blake3::Hash,
        before: usize,
        limit: usize,
    ) -> Result<(Vec<usize>, bool), ChatError> {
        self.postings.published_bytes()?;
        let mut low = 0;
        let mut high = self.postings.count(directory, digest);
        while low < high {
            let middle = low + (high - low) / 2;
            if self.ordinal(directory, digest, middle)? < before {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        let start = low.saturating_sub(limit as u64);
        let mut values = Vec::new();
        for position in start..low {
            values.push(self.ordinal(directory, digest, position)?);
        }
        Ok((values, start > 0))
    }

    pub(super) fn posting(
        &self,
        card: bool,
        term: &str,
        before: usize,
        limit: usize,
    ) -> Result<(Vec<usize>, bool), ChatError> {
        self.posting_at(
            if card { "cards" } else { "history" },
            blake3::hash(term.as_bytes()),
            before,
            limit,
        )
    }

    pub(super) fn matches(&self, count: usize, last: Option<&str>) -> bool {
        self.count == count && self.last_reference.as_deref() == last
    }

    fn card_ordinal(&self, id: &str) -> Result<Option<usize>, ChatError> {
        let digest = blake3::hash(id.as_bytes());
        let (values, omitted) = self.posting_at("card-ref", digest, usize::MAX, 1)?;
        if omitted {
            return Err(fail("task reference index changed"));
        }
        Ok(values.first().copied())
    }

    fn append_card(
        &mut self,
        ordinal: usize,
        source: TaskCardSource,
        card: &TaskCandidate,
    ) -> Result<(), ChatError> {
        if self.last_card.is_some_and(|last| ordinal <= last) {
            return Err(fail("task card order changed"));
        }
        let header = source_header(source);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(self.directory.join("card-data").join(ordinal.to_string()))?;
        file.write_all(&header)?;
        let mut reference = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(self.directory.join("task-ref").join(ordinal.to_string()))?;
        reference.write_all(&header)?;
        self.measure(|stats| {
            stats.cache_rows_written += 2;
            stats.cache_bytes_written += header.len() * 2;
        });
        self.append_posting("card-ref", blake3::hash(card.reference.as_bytes()), ordinal)?;
        self.append_posting("card-data", blake3::hash(b"card-order"), ordinal)?;
        for term in crate::domain::retrieval::task_posting_keys(&card.goal) {
            self.append_posting("cards", blake3::hash(term.as_bytes()), ordinal)?;
        }
        self.card_count += 1;
        self.measure(|stats| stats.cards_written += 1);
        self.last_card = Some(ordinal);
        Ok(())
    }

    fn card(&self, state: &SessionState, ordinal: usize) -> Result<TaskCandidate, ChatError> {
        let mut file = File::open(self.directory.join("card-data").join(ordinal.to_string()))?;
        let mut header = [0; 17];
        file.read_exact(&mut header)?;
        self.measure(|stats| {
            stats.card_rows_read += 1;
            stats.card_bytes_read += header.len();
        });
        let source = parse_source_header(header)?;
        let id = state.task_id_from_source(source)?;
        Ok(state.task_card_from_source(&id, source)?)
    }

    fn task_ref(&self, state: &SessionState, ordinal: usize) -> Result<String, ChatError> {
        let mut file = File::open(self.directory.join("task-ref").join(ordinal.to_string()))?;
        let mut header = [0; 17];
        file.read_exact(&mut header)?;
        self.measure(|stats| {
            stats.task_ref_rows_read += 1;
            stats.task_ref_bytes_read += header.len();
        });
        Ok(state.task_id_from_source(parse_source_header(header)?)?)
    }

    fn append_anchor(&mut self, index: usize, task_id: &str) -> Result<(), ChatError> {
        if self.last_anchor.is_some_and(|last| index <= last) {
            return Err(fail("task anchor order changed"));
        }
        let card = self
            .card_ordinal(task_id)?
            .ok_or_else(|| fail("task anchor card is absent"))?;
        let mut bytes = [0; 16];
        bytes[..8].copy_from_slice(&(index as u64).to_le_bytes());
        bytes[8..].copy_from_slice(&(card as u64).to_le_bytes());
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.directory.join("anchors"))?;
        file.write_all(&bytes)?;
        self.measure(|stats| {
            stats.cache_rows_written += 1;
            stats.cache_bytes_written += ANCHOR_BYTES as usize;
        });
        self.append_posting("anchor-post", blake3::hash(task_id.as_bytes()), index)?;
        self.anchor_count += 1;
        self.measure(|stats| stats.anchors_written += 1);
        self.last_anchor = Some(index);
        Ok(())
    }

    fn anchor(&self, position: usize) -> Result<(usize, usize), ChatError> {
        let mut file = File::open(self.directory.join("anchors"))?;
        if file.metadata()?.len() != self.anchor_count as u64 * ANCHOR_BYTES {
            return Err(fail("task anchor file changed"));
        }
        file.seek(SeekFrom::Start(position as u64 * ANCHOR_BYTES))?;
        let mut bytes = [0; 16];
        file.read_exact(&mut bytes)?;
        self.measure(|stats| {
            stats.anchor_rows_read += 1;
            stats.anchor_bytes_read += ANCHOR_BYTES as usize;
        });
        let index = usize::try_from(u64::from_le_bytes(
            bytes[..8].try_into().expect("fixed width"),
        ))
        .map_err(fail)?;
        let card = usize::try_from(u64::from_le_bytes(
            bytes[8..].try_into().expect("fixed width"),
        ))
        .map_err(fail)?;
        Ok((index, card))
    }

    fn anchor_upper_bound(&self, index: usize) -> Result<usize, ChatError> {
        let mut low = 0;
        let mut high = self.anchor_count;
        while low < high {
            let middle = low + (high - low) / 2;
            if self.anchor(middle)?.0 <= index {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        Ok(low)
    }

    fn anchor_task(&self, state: &SessionState, index: usize) -> Result<Option<String>, ChatError> {
        let position = self.anchor_upper_bound(index)?;
        if position == 0 {
            return Ok(None);
        }
        let (anchor, card) = self.anchor(position - 1)?;
        if anchor != index {
            return Ok(None);
        }
        self.task_ref(state, card).map(Some)
    }

    fn remember(&mut self, id: &str) -> Result<(), ChatError> {
        let ordinal = self
            .card_ordinal(id)?
            .ok_or_else(|| fail("recent task card is absent"))?;
        if self.recent.len() == 64 {
            self.recent.pop_front();
        }
        self.recent.push_back(ordinal);
        Ok(())
    }

    fn stage_state(&self, stage: &std::path::Path, index: usize) -> Result<(u64, bool), ChatError> {
        let mut file = File::open(stage.join("groups").join(index.to_string()))?;
        if file.metadata()?.len() != 16 {
            return Err(fail("navigation group length changed"));
        }
        let mut bytes = [0; 16];
        file.read_exact(&mut bytes)?;
        self.measure(|stats| stats.navigation_stage_read_bytes += bytes.len());
        let count = u64::from_le_bytes(bytes[..8].try_into().expect("fixed width"));
        let boundary = u64::from_le_bytes(bytes[8..16].try_into().expect("fixed width"));
        match boundary {
            0 => Ok((count, false)),
            1 => Ok((count, true)),
            _ => Err(fail("navigation boundary state changed")),
        }
    }

    fn write_stage_state(
        &self,
        stage: &std::path::Path,
        index: usize,
        count: u64,
        boundary: bool,
    ) -> Result<(), ChatError> {
        let mut bytes = [0; 16];
        bytes[..8].copy_from_slice(&count.to_le_bytes());
        bytes[8..].copy_from_slice(&u64::from(boundary).to_le_bytes());
        let mut file = File::create(stage.join("groups").join(index.to_string()))?;
        file.write_all(&bytes)?;
        self.measure(|stats| stats.navigation_stage_write_bytes += 16);
        Ok(())
    }

    fn rebuild(
        &mut self,
        state: &SessionState,
        input: &crate::domain::events::EventId,
    ) -> Result<(), ChatError> {
        self.measure(|stats| stats.metadata_rebuilt = true);
        for name in ["card-data", "task-ref"] {
            fs::remove_dir_all(self.directory.join(name))?;
            fs::create_dir(self.directory.join(name))?;
        }
        for name in ["cards", "card-data", "card-ref", "anchor-post"] {
            self.postings.clear(name);
        }
        let anchor_path = self.directory.join("anchors");
        if anchor_path.exists() {
            fs::remove_file(anchor_path)?;
        }
        self.card_count = 0;
        self.anchor_count = 0;
        self.last_card = None;
        self.last_anchor = None;
        self.recent.clear();
        let stage = self.directory.join("navigation-stage");
        fs::create_dir(&stage)?;
        fs::create_dir(stage.join("positions"))?;
        fs::create_dir(stage.join("selections"))?;
        fs::create_dir(stage.join("groups"))?;
        for (ordinal, (id, _)) in state.history_items_with_event_ids().enumerate() {
            let bytes = (ordinal as u64).to_le_bytes();
            let mut file = OpenOptions::new().write(true).create_new(true).open(
                stage
                    .join("positions")
                    .join(blake3::hash(id.as_str().as_bytes()).to_hex().as_str()),
            )?;
            file.write_all(&bytes)?;
            self.measure(|stats| stats.navigation_stage_write_bytes += 8);
            self.write_stage_state(&stage, ordinal, 0, false)?;
        }
        let selection_count = Cell::new(0usize);
        let boundary_count = Cell::new(0usize);
        let cursor = state.task_navigation_rebuild_sources(
            input,
            |id| {
                let digest = blake3::hash(id.as_str().as_bytes());
                let mut file = File::open(stage.join("positions").join(digest.to_hex().as_str()))
                    .map_err(domain_error)?;
                let mut bytes = [0; 8];
                file.read_exact(&mut bytes).map_err(domain_error)?;
                self.measure(|stats| stats.navigation_stage_read_bytes += 8);
                let ordinal = usize::try_from(u64::from_le_bytes(bytes)).map_err(domain_error)?;
                if file.metadata().map_err(domain_error)?.len() != 8
                    || state
                        .history_items_with_event_ids()
                        .nth(ordinal)
                        .map(|(value, _)| value)
                        != Some(id)
                {
                    return Err(domain_error("navigation position changed"));
                }
                Ok(ordinal)
            },
            |index, selection| {
                let mut file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(stage.join("selections").join(index.to_string()))
                    .map_err(domain_error)?;
                let position = file.metadata().map_err(domain_error)?.len();
                let mut bytes = [0; 16];
                bytes[..8].copy_from_slice(&(selection as u64).to_le_bytes());
                bytes[8..].copy_from_slice(&position.to_le_bytes());
                file.write_all(&bytes).map_err(domain_error)?;
                self.measure(|stats| stats.navigation_stage_write_bytes += 16);
                let (count, boundary) = self.stage_state(&stage, index).map_err(domain_error)?;
                self.write_stage_state(&stage, index, count + 1, boundary)
                    .map_err(domain_error)?;
                selection_count.set(selection_count.get() + 1);
                Ok(())
            },
            |index| {
                let (count, boundary) = self.stage_state(&stage, index).map_err(domain_error)?;
                if !boundary {
                    self.write_stage_state(&stage, index, count, true)
                        .map_err(domain_error)?;
                    boundary_count.set(boundary_count.get() + 1);
                }
                Ok(())
            },
        )?;
        let mut seen_selections = 0u64;
        let mut seen_boundaries = 0usize;
        for (index, _) in state.history_items_with_event_ids().enumerate() {
            let (count, boundary) = self.stage_state(&stage, index)?;
            seen_selections += count;
            seen_boundaries += usize::from(boundary);
            let path = stage.join("selections").join(index.to_string());
            if count != 0 {
                let mut file = File::open(path)?;
                let length = file.metadata()?.len();
                if length != count * 16 {
                    return Err(fail("navigation selection length changed"));
                }
                let mut slot = 0;
                let mut anchor = None;
                for position in (0..length).step_by(16) {
                    let mut bytes = [0; 16];
                    file.read_exact(&mut bytes)?;
                    self.measure(|stats| stats.navigation_stage_read_bytes += 16);
                    if u64::from_le_bytes(bytes[8..].try_into().expect("fixed width")) != position {
                        return Err(fail("navigation selection changed"));
                    }
                    let selection = usize::try_from(u64::from_le_bytes(
                        bytes[..8].try_into().expect("fixed width"),
                    ))
                    .map_err(fail)?;
                    anchor = Some(state.task_navigation_rebuild_selection(
                        index,
                        selection,
                        &mut slot,
                        |change| {
                            let inserted = match change {
                                TaskNavigationChange::Card {
                                    ordinal,
                                    source,
                                    card,
                                } => {
                                    if self
                                        .card_ordinal(&card.reference)
                                        .map_err(domain_error)?
                                        .is_none()
                                    {
                                        self.append_card(ordinal, source, &card)
                                            .map_err(domain_error)?;
                                        true
                                    } else {
                                        false
                                    }
                                }
                                TaskNavigationChange::Recent(id) => {
                                    self.remember(&id).map_err(domain_error)?;
                                    false
                                }
                                TaskNavigationChange::Anchor { .. } => {
                                    unreachable!("selection emits cards and recency")
                                }
                            };
                            Ok(inserted)
                        },
                    )?);
                }
                self.append_anchor(index, anchor.as_deref().expect("nonempty selection group"))?;
            } else if boundary
                && let Some((ordinal, source, card)) =
                    state.task_navigation_rebuild_legacy(index)?
            {
                self.append_card(ordinal, source, &card)?;
                self.remember(&card.reference)?;
                self.append_anchor(index, &card.reference)?;
            }
        }
        if seen_selections != selection_count.get() as u64
            || seen_boundaries != boundary_count.get()
        {
            return Err(fail("navigation group totals changed"));
        }
        fs::remove_dir_all(stage)?;
        self.cursor = cursor;
        self.initialized = true;
        Ok(())
    }

    pub(super) fn update_navigation(
        &mut self,
        state: &SessionState,
        input: &crate::domain::events::EventId,
    ) -> Result<(), ChatError> {
        if !self.initialized {
            return self.rebuild(state, input);
        }
        let changes = state.task_navigation_changes(
            self.cursor,
            input,
            &|index| self.anchor_task(state, index).map_err(domain_error),
            &|id| {
                self.card_ordinal(id)
                    .map(|value| value.is_some())
                    .map_err(domain_error)
            },
        )?;
        let Some((cursor, changes)) = changes else {
            return self.rebuild(state, input);
        };
        for change in changes {
            match change {
                TaskNavigationChange::Card {
                    ordinal,
                    source,
                    card,
                } => self.append_card(ordinal, source, &card)?,
                TaskNavigationChange::Anchor { index, task_id } => {
                    self.append_anchor(index, &task_id)?
                }
                TaskNavigationChange::Recent(id) => self.remember(&id)?,
            }
        }
        self.cursor = cursor;
        Ok(())
    }

    pub(super) fn view<'a>(&'a self, state: &'a SessionState) -> TaskIndexView<'a> {
        TaskIndexView { index: self, state }
    }
}

fn source_header(source: TaskCardSource) -> [u8; 17] {
    let mut bytes = [0; 17];
    match source {
        TaskCardSource::Legacy(index) => bytes[1..9].copy_from_slice(&(index as u64).to_le_bytes()),
        TaskCardSource::Revision {
            selection,
            revision,
        } => {
            bytes[0] = 1;
            bytes[1..9].copy_from_slice(&(selection as u64).to_le_bytes());
            bytes[9..17].copy_from_slice(&(revision as u64).to_le_bytes());
        }
    }
    bytes
}

fn parse_source_header(bytes: [u8; 17]) -> Result<TaskCardSource, ChatError> {
    let first = usize::try_from(u64::from_le_bytes(
        bytes[1..9].try_into().expect("fixed width"),
    ))
    .map_err(fail)?;
    let second = usize::try_from(u64::from_le_bytes(
        bytes[9..17].try_into().expect("fixed width"),
    ))
    .map_err(fail)?;
    match bytes[0] {
        0 if second == 0 => Ok(TaskCardSource::Legacy(first)),
        1 => Ok(TaskCardSource::Revision {
            selection: first,
            revision: second,
        }),
        _ => Err(fail("task card source encoding changed")),
    }
}

pub(super) struct TaskIndexView<'a> {
    index: &'a TaskIndex,
    state: &'a SessionState,
}

impl TaskNavigationAccess for TaskIndexView<'_> {
    fn task_count(&self) -> usize {
        self.index.card_count
    }
    fn recent_cards(&self) -> Result<Vec<usize>, SessionError> {
        Ok(self.index.recent.iter().rev().copied().collect())
    }
    fn all_cards(&self) -> Result<Vec<usize>, SessionError> {
        let (cards, omitted) = self
            .index
            .posting_at("card-data", blake3::hash(b"card-order"), usize::MAX, 512)
            .map_err(domain_error)?;
        if omitted || cards.len() != self.index.card_count {
            return Err(domain_error("task card order changed"));
        }
        Ok(cards)
    }
    fn task_ordinal(&self, id: &str) -> Result<Option<usize>, SessionError> {
        let value = self.index.card_ordinal(id).map_err(domain_error)?;
        if let Some(ordinal) = value
            && self
                .index
                .task_ref(self.state, ordinal)
                .map_err(domain_error)?
                != id
        {
            return Err(domain_error("task reference changed"));
        }
        Ok(value)
    }
    fn card(&self, ordinal: usize) -> Result<TaskCandidate, SessionError> {
        self.index.card(self.state, ordinal).map_err(domain_error)
    }
    fn task_at(&self, index: usize) -> Result<Option<String>, SessionError> {
        let upper = self.index.anchor_upper_bound(index).map_err(domain_error)?;
        if upper == 0 {
            return Ok(None);
        }
        let (_, card) = self.index.anchor(upper - 1).map_err(domain_error)?;
        Ok(Some(
            self.index
                .task_ref(self.state, card)
                .map_err(domain_error)?,
        ))
    }
    fn semantics(
        &self,
        index: usize,
    ) -> Result<Option<crate::domain::session::HistorySourceSemantics>, SessionError> {
        let path = self
            .index
            .directory
            .join("semantics")
            .join(index.to_string());
        let mut file = File::open(path).map_err(domain_error)?;
        let length = file.metadata().map_err(domain_error)?.len();
        if !(1..=4096).contains(&length) {
            return Err(domain_error("source semantics size changed"));
        }
        let mut bytes = vec![0; length as usize];
        file.read_exact(&mut bytes).map_err(domain_error)?;
        self.index.measure(|stats| {
            stats.semantics_rows_read += 1;
            stats.semantics_bytes_read += bytes.len();
        });
        let (origin, class, authority) = serde_json::from_slice(&bytes).map_err(domain_error)?;
        Ok(Some(crate::domain::session::HistorySourceSemantics {
            origin,
            class,
            authority,
        }))
    }
    fn related_history(
        &self,
        ids: &[String],
        before: usize,
        limit: usize,
    ) -> Result<Vec<usize>, SessionError> {
        let mut values = Vec::new();
        for id in ids {
            if values.len() == limit {
                break;
            }
            let (starts, _) = self
                .index
                .posting_at("anchor-post", blake3::hash(id.as_bytes()), before, limit)
                .map_err(domain_error)?;
            for start in starts.into_iter().rev() {
                let upper = self.index.anchor_upper_bound(start).map_err(domain_error)?;
                let end = if upper < self.index.anchor_count {
                    self.index
                        .anchor(upper)
                        .map_err(domain_error)?
                        .0
                        .min(before)
                } else {
                    before
                };
                for ordinal in (start..end).rev().take(limit - values.len()) {
                    values.push(ordinal);
                }
                if values.len() == limit {
                    break;
                }
            }
        }
        Ok(values)
    }
}

pub(super) fn update<'a>(
    session: &'a DurableChatSession,
    source: &ColdIndex,
) -> Result<std::sync::MutexGuard<'a, Option<TaskIndex>>, ChatError> {
    let mut cache = session
        .task_index
        .lock()
        .map_err(|_| fail("task index lock poisoned"))?;
    let mut index = match cache.take() {
        Some(index) if index.source_identity == source.identity() => index,
        _ => TaskIndex::new(source.identity())?,
    };
    index.stats.set(TaskIndexStats::default());
    index.update(source, session)?;
    *cache = Some(index);
    Ok(cache)
}
