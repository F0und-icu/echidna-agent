//! Session-bound context reads using existing source and projection owners.

use std::io::{self, Write};

use crate::adapters::project_memory::LocalProjectMemory;
use crate::app::context_read::{ContextSource, ContextSources, SourceRangeWriter};
use crate::domain::memory::MemoryPath;
use crate::domain::model::ContentItem;
use crate::ports::context_read::{
    ContextReadError, ContextReadPort, ContextReadRequest, ContextReadResponse,
};

use super::{ChatError, DurableChatSession};

#[path = "context_read_index.rs"]
pub(super) mod index;

const MEMORY_SEMANTICS: crate::domain::session::HistorySourceSemantics =
    crate::domain::session::HistorySourceSemantics {
        origin: crate::domain::context::ContextOrigin::ProjectMemory,
        class: crate::domain::context::SemanticClass::DerivedProjection,
        authority: crate::domain::context::AuthorityLevel::DerivedNonEvidence,
    };
const ARTIFACT_SEMANTICS: crate::domain::session::HistorySourceSemantics =
    crate::domain::session::HistorySourceSemantics {
        origin: crate::domain::context::ContextOrigin::ArtifactProjection,
        class: crate::domain::context::SemanticClass::AuthoritativeReference,
        authority: crate::domain::context::AuthorityLevel::FactReference,
    };
const PROGRESS_SEMANTICS: crate::domain::session::HistorySourceSemantics =
    crate::domain::session::HistorySourceSemantics {
        origin: crate::domain::context::ContextOrigin::SessionProjection,
        class: crate::domain::context::SemanticClass::DerivedProjection,
        authority: crate::domain::context::AuthorityLevel::DerivedNonEvidence,
    };

// Each unfinished search retains three auxiliary files and its index snapshot.
const MAX_ACTIVE_COLD_SEARCHES: usize = 4;

fn range_text(mut bytes: Vec<u8>) -> Result<String, ChatError> {
    loop {
        match String::from_utf8(bytes) {
            Ok(text) => return Ok(text),
            Err(error) if error.utf8_error().error_len().is_none() => {
                let valid = error.utf8_error().valid_up_to();
                bytes = error.into_bytes();
                bytes.truncate(valid);
            }
            Err(_) => {
                return Err(ChatError::CompactionSummarySource {
                    message: "context source range is not UTF-8 aligned".to_owned(),
                });
            }
        }
    }
}

fn history_source(
    reference: String,
    scope: &'static str,
    item: &ContentItem,
    semantics: crate::domain::session::HistorySourceSemantics,
    observed_at: Option<String>,
    offset: usize,
    maximum: usize,
) -> Result<ContextSource, ChatError> {
    let searchable = match item {
        ContentItem::ToolCall { call } => call.name.as_str() != "context_read",
        ContentItem::ToolResult { result } => result.tool_name.as_str() != "context_read",
        _ => true,
    };
    let mut writer = SourceRangeWriter {
        hash: blake3::Hasher::new(),
        offset,
        end: offset.saturating_add(maximum),
        position: 0,
        bytes: Vec::new(),
        stop_at_end: false,
        stopped: false,
    };
    serde_json::to_writer(&mut writer as &mut dyn Write, item).map_err(|error| {
        ChatError::CompactionSummarySource {
            message: format!("context source encoding failed: {error}"),
        }
    })?;
    let revision = writer.hash.finalize().to_hex().to_string();
    let (bytes, total_bytes) = if let ContentItem::Text { text, .. } = item {
        if offset > text.len() || !text.is_char_boundary(offset) {
            return Err(ChatError::CompactionSummarySource {
                message: "context source offset is invalid".to_owned(),
            });
        }
        let end = text.len().min(offset.saturating_add(maximum));
        (text.as_bytes()[offset..end].to_vec(), text.len())
    } else {
        if offset > writer.position {
            return Err(ChatError::CompactionSummarySource {
                message: "context source offset is invalid".to_owned(),
            });
        }
        (writer.bytes, writer.position)
    };
    let role = match item {
        ContentItem::Text { role, .. } | ContentItem::ImageArtifact { role, .. } => Some(*role),
        _ => None,
    };
    Ok(ContextSource {
        reference,
        scope,
        revision,
        role,
        semantics,
        observed_at,
        text: range_text(bytes)?,
        searchable,
        available_offset: offset,
        total_bytes,
    })
}

fn artifact_text_media(record: &crate::domain::artifacts::ArtifactRecord) -> bool {
    let media = record
        .media_type
        .split_once(';')
        .map_or(record.media_type.as_str(), |(media, _)| media)
        .trim();
    let Some((kind, subtype)) = media.split_once('/') else {
        return false;
    };
    if kind.eq_ignore_ascii_case("text") {
        return !subtype.is_empty();
    }
    if !kind.eq_ignore_ascii_case("application") {
        return false;
    }
    let subtype = subtype.to_ascii_lowercase();
    matches!(subtype.as_str(), "json" | "xml" | "x-ndjson")
        || subtype
            .strip_suffix("+json")
            .is_some_and(|name| !name.is_empty())
        || subtype
            .strip_suffix("+xml")
            .is_some_and(|name| !name.is_empty())
}

fn artifact_reference(reference: &str) -> Option<(&str, &str, Option<&str>)> {
    if let Some(value) = reference.strip_prefix("artifact-metadata:") {
        let (value, metadata_revision) = value.rsplit_once(':')?;
        let (id, blob) = value.rsplit_once(':')?;
        Some((id, blob, Some(metadata_revision)))
    } else {
        let (id, blob) = reference.strip_prefix("artifact:")?.rsplit_once(':')?;
        Some((id, blob, None))
    }
}

pub(super) struct ContextReadRuntime {
    session: DurableChatSession,
    core_limit: usize,
    diagnostics: bool,
    index: std::sync::Mutex<Vec<index::ColdSearch>>,
}

struct PeerSnapshot {
    events: Vec<crate::domain::events::EventEnvelope<serde_json::Value>>,
    projection: Option<crate::app::session::SessionProjection>,
    history_count: usize,
}

struct MemorySource {
    reference: String,
    revision: String,
    observed_at: Option<String>,
    bytes: Vec<u8>,
    total_bytes: usize,
    searchable: bool,
}

impl MemorySource {
    fn into_context(self, offset: usize) -> Result<ContextSource, ChatError> {
        Ok(ContextSource {
            reference: self.reference,
            scope: "memory",
            revision: self.revision,
            role: None,
            semantics: MEMORY_SEMANTICS,
            observed_at: self.observed_at,
            text: range_text(self.bytes)?,
            searchable: self.searchable,
            available_offset: offset,
            total_bytes: self.total_bytes,
        })
    }
}

impl ContextReadRuntime {
    fn current_binding(
        &self,
    ) -> Result<crate::domain::agent_definition::AgentDefinitionBinding, ChatError> {
        Ok(self
            .session
            .current_binding
            .read()
            .map_err(|_| io::Error::other("Agent binding lock poisoned"))?
            .clone())
    }
    pub(super) fn new(
        session: &DurableChatSession,
        core_limit: usize,
        diagnostics: bool,
    ) -> Result<Self, ChatError> {
        let mut reader_session = DurableChatSession::new(
            session.project.clone(),
            session.session_id.clone(),
            session.binding.clone(),
        )?;
        reader_session.projection = std::sync::Arc::clone(&session.projection);
        reader_session.current_binding = std::sync::Arc::clone(&session.current_binding);
        reader_session.context_source_index = std::sync::Arc::clone(&session.context_source_index);
        Ok(Self {
            session: reader_session,
            core_limit,
            diagnostics,
            index: std::sync::Mutex::new(Vec::new()),
        })
    }

    fn peer_snapshot(
        &self,
        id: &crate::domain::events::StreamId,
    ) -> Result<PeerSnapshot, ChatError> {
        let events = crate::adapters::project_event_store::read_existing_session_events(
            &self.session.project,
            id,
        )?;
        let state = crate::domain::session::SessionReducer.reduce(events.iter().cloned())?;
        let current_binding = self.current_binding()?;
        let history_count = state.history().items().len();
        let same_agent = match state.agent_definition_binding() {
            Some(binding) => binding.definition().id() == current_binding.definition().id(),
            // Pre-definition Sessions resume as the builtin research identity.
            None => current_binding.definition().id().as_str() == "research",
        };
        if !same_agent {
            return Ok(PeerSnapshot {
                events,
                projection: None,
                history_count,
            });
        }
        let project_events = crate::adapters::project_event_store::read_existing_research_events(
            &self.session.project,
        )?;
        let verified = crate::app::session::SessionProjection::replay_project_verified(
            self.session.project.project_id().clone(),
            &events,
            &project_events,
            || {
                crate::adapters::research_projection::load_reference_catalog(&self.session.project)
                    .map_err(
                        |message| crate::domain::session::SessionError::TaskContext { message },
                    )
            },
            &self.session.artifacts,
        )?;
        Ok(PeerSnapshot {
            events,
            projection: Some(verified),
            history_count,
        })
    }

    fn peer_history(
        &self,
        session_id: &str,
        event_id: &str,
        reference: &str,
        offset: usize,
        maximum: usize,
    ) -> Result<Option<ContextSource>, ChatError> {
        let id = self
            .session
            .project
            .existing_session_ids()?
            .into_iter()
            .find(|id| id.as_str() == session_id);
        let Some(id) = id else {
            return Ok(None);
        };
        let snapshot = self.peer_snapshot(&id)?;
        let Some(verified) = snapshot.projection else {
            return Ok(None);
        };
        let Some((id, item)) = verified
            .state()
            .history_items_with_event_ids()
            .find(|(id, _)| id.as_str() == event_id)
        else {
            return Ok(None);
        };
        let observed = snapshot
            .events
            .iter()
            .find(|event| &event.event_id == id)
            .map(|event| event.occurred_at.as_str().to_owned());
        let semantics = verified.state().history_source_semantics(id, item)?;
        history_source(
            reference.to_owned(),
            "agent_session",
            item,
            semantics,
            observed,
            offset,
            maximum,
        )
        .map(Some)
    }

    fn read_artifact(
        &self,
        reference: &str,
        offset: usize,
        maximum: usize,
    ) -> Result<ContextReadResponse, ChatError> {
        index::ColdIndex::acquire_runtime(self)?;
        let index = index::ColdIndex::refresh_peers(self)?;
        self.read_artifact_scoped(reference, offset, maximum, &index)
            .map(|(response, _)| response)
    }

    fn read_artifact_scoped(
        &self,
        reference: &str,
        offset: usize,
        maximum: usize,
        index: &index::ColdIndex,
    ) -> Result<(ContextReadResponse, usize), ChatError> {
        use crate::ports::context_read::ContextReadStatus;
        let empty = |status| {
            (
                ContextReadResponse {
                    status,
                    entries: Vec::new(),
                    truncated: false,
                    next_cursor: None,
                },
                0,
            )
        };
        let Some((id, blob_revision, metadata_revision)) = artifact_reference(reference) else {
            return Ok(empty(ContextReadStatus::NotFound));
        };
        let read = self
            .session
            .with_projection(|projection, _, _| self.session.project_read(projection.state()))?;
        let state = read.research_state();
        let Some(record) = state.artifact(&crate::domain::artifacts::ArtifactId::new(id)) else {
            return Ok(empty(ContextReadStatus::NotFound));
        };
        let observed_at = read
            .events
            .iter()
            .find(|event| event.event_id == record.created_by_event_id)
            .map(|event| event.occurred_at.as_str().to_owned());
        let (owner, verified_bytes) = index.artifact_owner(
            &self.session.project,
            &self.session.session_id,
            record,
            true,
        )?;
        match owner {
            Some(true) => {}
            Some(false) => return Ok(empty(ContextReadStatus::NotFound)),
            None => return Ok(empty(ContextReadStatus::Ambiguous)),
        }
        if blob_revision != record.blob_id.as_str() {
            return Ok(empty(ContextReadStatus::Stale));
        }
        let text_media = metadata_revision.is_none() && artifact_text_media(record);
        let (bytes, total, _) = self
            .session
            .artifacts
            .read_artifact_range(
                &record.blob_id,
                if text_media { offset as u64 } else { 0 },
                if text_media { maximum } else { 0 },
            )
            .map_err(|error| ChatError::CompactionSummarySource {
                message: format!("Artifact Blob read failed: {error}"),
            })?;
        if total != record.size_bytes {
            return Err(ChatError::CompactionSummarySource {
                message: "Artifact size differs from its Blob".to_owned(),
            });
        }
        let source = if text_media {
            ContextSource {
                reference: reference.to_owned(),
                scope: "artifact",
                revision: blob_revision.to_owned(),
                role: None,
                semantics: ARTIFACT_SEMANTICS,
                observed_at: observed_at.clone(),
                text: range_text(bytes)?,
                searchable: false,
                available_offset: offset,
                total_bytes: usize::try_from(total).map_err(|_| {
                    ChatError::CompactionSummarySource {
                        message: "Artifact exceeds platform size".to_owned(),
                    }
                })?,
            }
        } else {
            let text = serde_json::to_string(record).map_err(|error| {
                ChatError::CompactionSummarySource {
                    message: error.to_string(),
                }
            })?;
            if metadata_revision
                .is_some_and(|revision| revision != blake3::hash(text.as_bytes()).to_hex().as_str())
            {
                return Ok(empty(ContextReadStatus::Stale));
            }
            let total_bytes = text.len();
            ContextSource {
                reference: reference.to_owned(),
                scope: "artifact_metadata",
                revision: metadata_revision.unwrap_or(blob_revision).to_owned(),
                role: None,
                semantics: ARTIFACT_SEMANTICS,
                observed_at,
                text,
                searchable: false,
                available_offset: 0,
                total_bytes,
            }
        };
        crate::app::context_read::read(
            ContextReadRequest::Read {
                reference: reference.to_owned(),
                offset,
                max_bytes: maximum,
            },
            ContextSources {
                history: vec![source],
                memory: None,
                progress: None,
                more: false,
                ambiguous_time: false,
            },
        )
        .map(|response| (response, verified_bytes))
        .map_err(|error| ChatError::CompactionSummarySource {
            message: error.to_string(),
        })
    }

    fn memory_source(
        &self,
        path: &str,
        revision: &str,
        reference: &str,
        offset: usize,
        maximum: usize,
    ) -> Result<(Option<ContextSource>, Option<ContextSource>), ChatError> {
        let (historical, current) =
            self.memory_source_bytes(path, revision, reference, offset, maximum)?;
        Ok((
            historical
                .map(|source| source.into_context(offset))
                .transpose()?,
            current
                .map(|source| source.into_context(offset))
                .transpose()?,
        ))
    }

    fn memory_source_bytes(
        &self,
        path: &str,
        revision: &str,
        reference: &str,
        offset: usize,
        maximum: usize,
    ) -> Result<(Option<MemorySource>, Option<MemorySource>), ChatError> {
        let binding = self.current_binding()?;
        let Some(scope) = crate::cli_agent::memory_scope(&binding) else {
            return Ok((None, None));
        };
        let path = match MemoryPath::parse(path) {
            Ok(path) => path,
            Err(_) => return Ok((None, None)),
        };
        let store = LocalProjectMemory::with_scope(&self.session.project, scope.clone())
            .with_core_byte_limit(self.core_limit);
        let mut bytes = Vec::new();
        let mut position = 0usize;
        let document = store
            .visit_context_content(&path, |chunk| {
                let end = position.saturating_add(chunk.len());
                let requested_end = offset.saturating_add(maximum);
                if offset < end && requested_end > position {
                    let start = offset.saturating_sub(position);
                    let stop = requested_end.saturating_sub(position).min(chunk.len());
                    bytes.extend_from_slice(&chunk[start..stop]);
                }
                position = end;
                Ok(())
            })
            .map_err(super::context::ChatContextRefreshError::from)?;
        let current = match document {
            Some((hash, total_bytes)) => {
                let bytes = if hash == revision { bytes } else { Vec::new() };
                Some(MemorySource {
                    reference: format!("memory:{}:{hash}", path.as_str()),
                    revision: hash,
                    observed_at: None,
                    bytes,
                    searchable: true,
                    total_bytes,
                })
            }
            None => None,
        };
        if current
            .as_ref()
            .is_some_and(|value| value.revision == revision)
        {
            return Ok((None, current));
        }
        let read = self
            .session
            .with_projection(|projection, _, _| self.session.project_read(projection.state()))?;
        let mut memory_state = crate::domain::memory::MemoryState::default();
        let mut seen = std::collections::BTreeSet::new();
        for event in read.events.iter() {
            if !matches!(
                event.event_type.as_str(),
                "memory.changed" | "memory.external_change_detected"
            ) || !seen.insert(&event.event_id)
            {
                continue;
            }
            let crate::domain::research::ResearchEvent::Memory(memory) =
                crate::domain::research::ResearchEvent::decode(
                    event.event_type.as_str(),
                    event.payload.clone(),
                )
                .map_err(|error| ChatError::CompactionSummarySource {
                    message: format!("Memory source decode failed: {error}"),
                })?
            else {
                unreachable!("Memory Event types decode as Memory")
            };
            crate::domain::memory::MemoryReducer
                .apply(&mut memory_state, memory)
                .map_err(|error| ChatError::CompactionSummarySource {
                    message: format!("Memory revision chain failed: {error}"),
                })?;
        }
        let mut found = None;
        for event in read.events.iter().rev() {
            if !matches!(
                event.event_type.as_str(),
                "memory.changed" | "memory.external_change_detected"
            ) {
                continue;
            }
            let crate::domain::research::ResearchEvent::Memory(memory) =
                crate::domain::research::ResearchEvent::decode(
                    event.event_type.as_str(),
                    event.payload.clone(),
                )
                .map_err(|error| ChatError::CompactionSummarySource {
                    message: error.to_string(),
                })?
            else {
                continue;
            };
            let value = memory.revision();
            if value.scope != scope || value.path != path {
                continue;
            }
            if value.new_hash.as_deref() == Some(revision) {
                found = value
                    .new_blob_id
                    .clone()
                    .map(|blob| (blob, Some(event.occurred_at.as_str().to_owned())));
                break;
            }
            if found.is_none() && value.old_hash.as_deref() == Some(revision) {
                found = value.old_blob_id.clone().map(|blob| (blob, None));
            }
        }
        if let Some((blob, observed_at)) = found {
            let (bytes, total, content_hash) = self
                .session
                .artifacts
                .read_artifact_range(&blob, offset as u64, maximum)
                .map_err(|error| ChatError::CompactionSummarySource {
                    message: format!("Memory revision Blob read failed: {error}"),
                })?;
            if content_hash.as_deref() != Some(revision) {
                return Err(ChatError::CompactionSummarySource {
                    message: "Memory revision hash differs from its Blob".to_owned(),
                });
            }
            let total_bytes =
                usize::try_from(total).map_err(|_| ChatError::CompactionSummarySource {
                    message: "Memory revision exceeds platform size".to_owned(),
                })?;
            return Ok((
                Some(MemorySource {
                    reference: reference.to_owned(),
                    revision: revision.to_owned(),
                    observed_at,
                    bytes,
                    searchable: false,
                    total_bytes,
                }),
                current,
            ));
        }
        Ok((None, current))
    }

    fn visit_memory_sources(
        &self,
        mut visit: impl FnMut(&LocalProjectMemory<'_>, &MemoryPath) -> io::Result<()>,
    ) -> Result<(), ChatError> {
        let binding = self.current_binding()?;
        let Some(scope) = crate::cli_agent::memory_scope(&binding) else {
            return Ok(());
        };
        let store = LocalProjectMemory::with_scope(&self.session.project, scope)
            .with_core_byte_limit(self.core_limit);
        store
            .visit_context_paths(|path| visit(&store, &path))
            .map_err(super::context::ChatContextRefreshError::from)?;
        Ok(())
    }

    fn visit_auxiliary_sources(
        &self,
        index: &index::ColdIndex,
        after: Option<u64>,
        before: Option<u64>,
        mut visit: impl FnMut(ContextSource) -> Result<(), ChatError>,
    ) -> Result<bool, ChatError> {
        let mut ambiguous = false;
        self.session.with_projection(|projection, _, _| {
            let project = self.session.project_read(projection.state())?;
            for record in project.research_state().retrieval_artifact_records() {
                let observed_at = project
                    .events
                    .iter()
                    .find(|event| event.event_id == record.created_by_event_id)
                    .map(|event| event.occurred_at.as_str().to_owned());
                let time = observed_at
                    .as_deref()
                    .and_then(crate::app::context_read::unix_seconds);
                if time.is_some_and(|time| {
                    after.is_some_and(|lower| time < lower)
                        || before.is_some_and(|upper| time > upper)
                }) {
                    continue;
                }
                match index
                    .artifact_owner(
                        &self.session.project,
                        &self.session.session_id,
                        record,
                        false,
                    )?
                    .0
                {
                    Some(true) => {}
                    Some(false) => continue,
                    None => {
                        ambiguous = true;
                        continue;
                    }
                }
                let metadata = serde_json::to_string(record).map_err(|error| {
                    ChatError::CompactionSummarySource {
                        message: error.to_string(),
                    }
                })?;
                let (scope, total_bytes) = if artifact_text_media(record) {
                    (
                        "artifact",
                        usize::try_from(record.size_bytes).map_err(|_| {
                            ChatError::CompactionSummarySource {
                                message: "Artifact exceeds platform size".to_owned(),
                            }
                        })?,
                    )
                } else {
                    ("artifact_metadata", metadata.len())
                };
                visit(ContextSource {
                    reference: format!("artifact:{}:{}", record.id, record.blob_id),
                    scope,
                    revision: record.blob_id.as_str().to_owned(),
                    role: None,
                    semantics: ARTIFACT_SEMANTICS,
                    observed_at,
                    text: metadata,
                    searchable: true,
                    available_offset: 0,
                    total_bytes,
                })?;
            }
            Ok(())
        })?;
        Ok(ambiguous)
    }

    fn progress_source(&self, task_ref: &str) -> Result<Option<ContextSource>, ChatError> {
        self.session.with_projection(|projection, _, _| {
            projection
                .state()
                .task_progress_source(
                    task_ref,
                    &crate::app::context_read::history_navigation_excerpt,
                )?
                .map(|(_, text)| {
                    let revision = crate::domain::canonical_json::canonical_blake3_hash(&text)
                        .map_err(|error| ChatError::CompactionSummarySource {
                            message: error.to_string(),
                        })?;
                    let total_bytes = text.len();
                    Ok(ContextSource {
                        reference: format!("progress:{task_ref}:{revision}"),
                        scope: "task_progress",
                        revision,
                        role: None,
                        semantics: PROGRESS_SEMANTICS,
                        observed_at: None,
                        text,
                        searchable: false,
                        available_offset: 0,
                        total_bytes,
                    })
                })
                .transpose()
        })
    }

    fn sources(&self, request: &ContextReadRequest) -> Result<ContextSources, ChatError> {
        if let ContextReadRequest::Read {
            reference,
            offset,
            max_bytes,
        } = request
        {
            if let Some((session_id, event_id)) = reference
                .strip_prefix("session:")
                .and_then(|value| value.split_once(':'))
            {
                let history = self
                    .peer_history(session_id, event_id, reference, *offset, *max_bytes)?
                    .into_iter()
                    .collect();
                return Ok(ContextSources {
                    history,
                    memory: None,
                    progress: None,
                    more: false,
                    ambiguous_time: false,
                });
            }
            if let Some((path, revision)) = reference
                .strip_prefix("memory:")
                .and_then(|value| value.rsplit_once(':'))
            {
                let (historical, current) =
                    self.memory_source(path, revision, reference, *offset, *max_bytes)?;
                return Ok(ContextSources {
                    history: historical.into_iter().collect(),
                    memory: current,
                    progress: None,
                    more: false,
                    ambiguous_time: false,
                });
            }
            if let Some((task, _)) = reference
                .strip_prefix("progress:")
                .and_then(|value| value.rsplit_once(':'))
            {
                return Ok(ContextSources {
                    history: Vec::new(),
                    memory: None,
                    progress: self.progress_source(task)?,
                    more: false,
                    ambiguous_time: false,
                });
            }
            let history = self.session.with_projection(|projection, _, _| {
                let events = crate::adapters::project_event_store::read_existing_session_events(
                    &self.session.project,
                    &self.session.session_id,
                )?;
                let Some((id, item)) = projection
                    .state()
                    .history_items_with_event_ids()
                    .find(|(id, _)| id.as_str() == reference)
                else {
                    return Ok(Vec::new());
                };
                let observed = events
                    .iter()
                    .find(|event| &event.event_id == id)
                    .map(|event| event.occurred_at.as_str().to_owned());
                let semantics = projection.state().history_source_semantics(id, item)?;
                Ok(vec![history_source(
                    reference.clone(),
                    "session",
                    item,
                    semantics,
                    observed,
                    *offset,
                    *max_bytes,
                )?])
            })?;
            return Ok(ContextSources {
                history,
                memory: None,
                progress: None,
                more: false,
                ambiguous_time: false,
            });
        }
        let ContextReadRequest::Progress { task_ref, .. } = request else {
            unreachable!("all context read operations are covered")
        };
        Ok(ContextSources {
            history: Vec::new(),
            memory: None,
            progress: self.progress_source(task_ref)?,
            more: false,
            ambiguous_time: false,
        })
    }
}

impl ContextReadPort for ContextReadRuntime {
    fn read(&self, request: ContextReadRequest) -> Result<ContextReadResponse, ContextReadError> {
        if let ContextReadRequest::Read {
            reference,
            offset,
            max_bytes,
        } = &request
            && let Some((session_id, event_and_revision)) = reference
                .strip_prefix("session-revision:")
                .and_then(|value| value.split_once(':'))
            && let Some((event_id, expected_revision)) = event_and_revision.rsplit_once(':')
        {
            let history = self
                .peer_history(session_id, event_id, reference, *offset, *max_bytes)
                .map_err(|error| ContextReadError {
                    message: error.to_string(),
                })?;
            if history
                .as_ref()
                .is_some_and(|source| source.revision != expected_revision)
            {
                return Ok(ContextReadResponse {
                    status: crate::ports::context_read::ContextReadStatus::Stale,
                    entries: Vec::new(),
                    truncated: false,
                    next_cursor: None,
                });
            }
            return crate::app::context_read::read(
                request,
                ContextSources {
                    history: history.into_iter().collect(),
                    memory: None,
                    progress: None,
                    more: false,
                    ambiguous_time: false,
                },
            );
        }
        if let ContextReadRequest::Search {
            query,
            after_unix,
            before_unix,
            max_bytes,
            cursor,
        } = &request
        {
            let result = (|| -> Result<_, ChatError> {
                let mut index = self
                    .index
                    .lock()
                    .map_err(|_| io::Error::other("context index lock poisoned"))?;
                let binding = self.current_binding()?;
                let digest = binding.definition().digest().as_str();
                index.retain(|search| search.belongs_to_definition(digest));
                if let Some(cursor) = cursor {
                    let mut found = None;
                    for (position, search) in index.iter().enumerate() {
                        if search.matches(self, query, *after_unix, *before_unix, cursor)? {
                            found = Some(position);
                            break;
                        }
                    }
                    let position = found.ok_or_else(|| ChatError::CompactionSummarySource {
                        message: "context search cursor is stale or belongs to another query"
                            .to_owned(),
                    })?;
                    let result = index[position].page(self, *max_bytes);
                    if match &result {
                        Ok(response) => response.next_cursor.is_none(),
                        Err(_) => true,
                    } {
                        index.remove(position);
                    }
                    result
                } else {
                    if index.len() == MAX_ACTIVE_COLD_SEARCHES {
                        return Err(ChatError::CompactionSummarySource {
                            message: "too many unfinished context searches; complete an existing cursor before starting another".to_owned(),
                        });
                    }
                    let mut search =
                        index::ColdSearch::build(self, query, *after_unix, *before_unix)?;
                    let response = search.page(self, *max_bytes)?;
                    if response.next_cursor.is_some() {
                        index.push(search);
                    }
                    Ok(response)
                }
            })();
            return result.map_err(|error| ContextReadError {
                message: error.to_string(),
            });
        }
        if let ContextReadRequest::Read {
            reference,
            offset,
            max_bytes,
        } = &request
            && (reference.starts_with("artifact:") || reference.starts_with("artifact-metadata:"))
        {
            return self
                .read_artifact(reference, *offset, *max_bytes)
                .map_err(|error| ContextReadError {
                    message: error.to_string(),
                });
        }
        let sources = self.sources(&request).map_err(|error| ContextReadError {
            message: error.to_string(),
        })?;
        crate::app::context_read::read(request, sources)
    }
}
