//! Local Markdown Memory filesystem boundary.
//!
//! This adapter owns path confinement, symlink rejection, BLAKE3 content
//! hashes and durable Markdown replacement. Event ordering remains with the
//! fixed Memory tools so that no filesystem helper becomes a transaction API.

use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::adapters::file_diff::replacement_diff as audit_replacement_diff;
use crate::adapters::project_event_store::{Project, replace_durable_file, sync_directory};
use crate::domain::memory::{MemoryPath, MemoryScope};

const MEMORY_DIRECTORY: &str = "memory";
const MAX_CORE_MEMORY_BYTES: usize = 512 * 1024;
const MAX_RETRIEVAL_MEMORY_DOCUMENTS: usize = 64;
const MAX_RETRIEVAL_MEMORY_DOCUMENT_BYTES: u64 = 64 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MemoryDocument {
    pub(crate) content: Option<String>,
    pub(crate) hash: Option<String>,
}

/// A bounded, read-only Memory fact selected for local retrieval Shadow.
/// The Markdown remains authoritative; this value is never persisted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RetrievalMemoryDocument {
    pub(crate) path: MemoryPath,
    pub(crate) document: MemoryDocument,
}

pub(crate) struct LocalProjectMemory<'project> {
    project: &'project Project,
    scope: MemoryScope,
    core_byte_limit: usize,
}

impl<'project> LocalProjectMemory<'project> {
    /// Enumerate the task-read vocabulary without retaining the directory or
    /// imposing the separate Research Shadow candidate limit.
    pub(crate) fn visit_context_paths(
        &self,
        mut visit: impl FnMut(MemoryPath) -> io::Result<()>,
    ) -> Result<(), LocalProjectMemoryError> {
        self.ensure_control_directory_existing()?;
        if !self.validate_memory_root_existing()? {
            return Ok(());
        }
        for name in ["decisions.md", "failed-paths.md"] {
            let path = MemoryPath::parse(name).expect("fixed Memory path");
            visit(path).map_err(|source| LocalProjectMemoryError::Read {
                path: self.memory_root().join(name),
                source,
            })?;
        }
        let notes = self.memory_root().join("notes");
        if validate_existing_directory(&notes)? {
            for entry in
                fs::read_dir(&notes).map_err(|source| LocalProjectMemoryError::Inspect {
                    path: notes.clone(),
                    source,
                })?
            {
                let entry = entry.map_err(|source| LocalProjectMemoryError::Inspect {
                    path: notes.clone(),
                    source,
                })?;
                let entry_path = entry.path();
                let metadata = fs::symlink_metadata(&entry_path).map_err(|source| {
                    LocalProjectMemoryError::Inspect {
                        path: entry_path.clone(),
                        source,
                    }
                })?;
                if metadata.file_type().is_symlink() {
                    return Err(LocalProjectMemoryError::Symlink { path: entry_path });
                }
                if !metadata.is_file() {
                    return Err(LocalProjectMemoryError::NotRegular { path: entry_path });
                }
                let name = entry.file_name().into_string().map_err(|_| {
                    LocalProjectMemoryError::InvalidRetrievalPath {
                        path: entry_path.display().to_string(),
                    }
                })?;
                let path = MemoryPath::parse(format!("notes/{name}")).map_err(|_| {
                    LocalProjectMemoryError::InvalidRetrievalPath {
                        path: entry_path.display().to_string(),
                    }
                })?;
                visit(path).map_err(|source| LocalProjectMemoryError::Read {
                    path: entry_path,
                    source,
                })?;
            }
        }
        visit(MemoryPath::parse("core.md").expect("fixed Memory path")).map_err(|source| {
            LocalProjectMemoryError::Read {
                path: self.memory_root().join("core.md"),
                source,
            }
        })?;
        Ok(())
    }

    /// Stream and validate the current exact Memory revision with bounded
    /// working memory. The callback receives original, unescaped UTF-8 bytes.
    pub(crate) fn visit_context_content(
        &self,
        path: &MemoryPath,
        mut visit: impl FnMut(&[u8]) -> io::Result<()>,
    ) -> Result<Option<(String, usize)>, LocalProjectMemoryError> {
        self.ensure_control_directory_existing()?;
        if !self.validate_memory_root_existing()? {
            return Ok(None);
        }
        self.ensure_note_parent_existing(path)?;
        let document_path = self.document_path(path);
        let metadata = match fs::symlink_metadata(&document_path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(LocalProjectMemoryError::Symlink {
                    path: document_path,
                });
            }
            Ok(metadata) if !metadata.is_file() => {
                return Err(LocalProjectMemoryError::NotRegular {
                    path: document_path,
                });
            }
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(LocalProjectMemoryError::Inspect {
                    path: document_path,
                    source,
                });
            }
        };
        self.validate_size(path, metadata.len())?;
        let mut file =
            File::open(&document_path).map_err(|source| LocalProjectMemoryError::Read {
                path: document_path.clone(),
                source,
            })?;
        let mut hasher = blake3::Hasher::new();
        let mut total = 0usize;
        let mut pending = Vec::with_capacity(4);
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let count = file
                .read(&mut buffer)
                .map_err(|source| LocalProjectMemoryError::Read {
                    path: document_path.clone(),
                    source,
                })?;
            if count == 0 {
                break;
            }
            total = total
                .checked_add(count)
                .ok_or_else(|| LocalProjectMemoryError::Read {
                    path: document_path.clone(),
                    source: io::Error::other("Memory document size overflow"),
                })?;
            if total as u64 > metadata.len() {
                return Err(LocalProjectMemoryError::Read {
                    path: document_path,
                    source: io::Error::other("Memory document changed while reading"),
                });
            }
            hasher.update(&buffer[..count]);
            visit(&buffer[..count]).map_err(|source| LocalProjectMemoryError::Read {
                path: document_path.clone(),
                source,
            })?;
            pending.extend_from_slice(&buffer[..count]);
            match std::str::from_utf8(&pending) {
                Ok(_) => pending.clear(),
                Err(error) if error.error_len().is_none() => {
                    let valid = error.valid_up_to();
                    pending.drain(..valid);
                }
                Err(error) => {
                    return Err(LocalProjectMemoryError::Read {
                        path: document_path,
                        source: io::Error::new(io::ErrorKind::InvalidData, error),
                    });
                }
            }
        }
        if total as u64 != metadata.len() || !pending.is_empty() {
            return Err(LocalProjectMemoryError::Read {
                path: document_path,
                source: io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Memory document changed or ends in incomplete UTF-8",
                ),
            });
        }
        Ok(Some((hasher.finalize().to_hex().to_string(), total)))
    }
    pub(crate) fn new(project: &'project Project) -> Self {
        Self::with_scope(project, MemoryScope::Project)
    }

    pub(crate) fn with_scope(project: &'project Project, scope: MemoryScope) -> Self {
        Self {
            project,
            scope,
            core_byte_limit: MAX_CORE_MEMORY_BYTES,
        }
    }

    pub(crate) fn with_core_byte_limit(mut self, limit: usize) -> Self {
        self.core_byte_limit = limit;
        self
    }

    pub(crate) fn read(
        &self,
        path: &MemoryPath,
    ) -> Result<MemoryDocument, LocalProjectMemoryError> {
        self.ensure_control_directory_existing()?;
        if !self.validate_memory_root_existing()? {
            return Ok(MemoryDocument {
                content: None,
                hash: None,
            });
        }
        let document_path = self.document_path(path);
        self.ensure_note_parent_existing(path)?;
        match fs::symlink_metadata(&document_path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                Err(LocalProjectMemoryError::Symlink {
                    path: document_path,
                })
            }
            Ok(metadata) if !metadata.file_type().is_file() => {
                Err(LocalProjectMemoryError::NotRegular {
                    path: document_path,
                })
            }
            Ok(metadata) => {
                self.validate_size(path, metadata.len())?;
                let bytes =
                    fs::read(&document_path).map_err(|source| LocalProjectMemoryError::Read {
                        path: document_path.clone(),
                        source,
                    })?;
                let content = String::from_utf8(bytes).map_err(|source| {
                    LocalProjectMemoryError::NotUtf8 {
                        path: document_path.clone(),
                        source,
                    }
                })?;
                Ok(MemoryDocument {
                    hash: Some(content_hash(content.as_bytes())),
                    content: Some(content),
                })
            }
            Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(MemoryDocument {
                content: None,
                hash: None,
            }),
            Err(source) => Err(LocalProjectMemoryError::Inspect {
                path: document_path,
                source,
            }),
        }
    }

    /// Reads the fixed project Memory vocabulary and valid note files without
    /// creating directories, files, revisions, cache entries, or Events.
    pub(crate) fn read_retrieval_documents(
        &self,
    ) -> Result<Vec<RetrievalMemoryDocument>, LocalProjectMemoryError> {
        self.ensure_control_directory_existing()?;
        if !self.validate_memory_root_existing()? {
            return Ok(Vec::new());
        }
        let mut paths = ["decisions.md", "failed-paths.md"]
            .into_iter()
            .map(MemoryPath::parse)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| LocalProjectMemoryError::InvalidRetrievalPath {
                path: error.to_string(),
            })?;
        paths.extend(self.retrieval_note_paths()?);
        paths.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        paths
            .into_iter()
            .map(|path| {
                self.validate_retrieval_document_size(&path)?;
                let document = self.read(&path)?;
                Ok(RetrievalMemoryDocument { path, document })
            })
            .filter_map(
                |document: Result<RetrievalMemoryDocument, LocalProjectMemoryError>| match document
                {
                    Ok(document) if document.document.content.is_some() => Some(Ok(document)),
                    Ok(_) => None,
                    Err(error) => Some(Err(error)),
                },
            )
            // Missing optional paths do not consume quota; present empty files do.
            // Stay lazy so no later document body is read once the quota is full.
            .take(MAX_RETRIEVAL_MEMORY_DOCUMENTS)
            .collect()
    }

    /// Performs one atomic Markdown replacement after all path components have
    /// been verified. The caller holds the project Writer lock and has already
    /// published any revision Blobs needed by the subsequent Event.
    pub(crate) fn replace(
        &self,
        path: &MemoryPath,
        content: &str,
    ) -> Result<(), LocalProjectMemoryError> {
        self.validate_content(path, content)?;
        self.ensure_control_directory_existing()?;
        let memory_root = self.memory_root();
        self.ensure_memory_root()?;
        if path.as_str().starts_with("notes/") {
            ensure_directory(&memory_root.join("notes"))?;
        }
        let document_path = self.document_path(path);
        match fs::symlink_metadata(&document_path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(LocalProjectMemoryError::Symlink {
                    path: document_path,
                });
            }
            Ok(metadata) if !metadata.file_type().is_file() => {
                return Err(LocalProjectMemoryError::NotRegular {
                    path: document_path,
                });
            }
            Ok(_) => {}
            Err(source) if source.kind() == io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(LocalProjectMemoryError::Inspect {
                    path: document_path,
                    source,
                });
            }
        }
        replace_durable_file(&document_path, content.as_bytes()).map_err(|error| {
            LocalProjectMemoryError::DurableReplace {
                path: document_path,
                error: Box::new(error),
            }
        })
    }

    pub(crate) fn memory_root(&self) -> PathBuf {
        self.project
            .control_directory()
            .join(self.scope.storage_directory())
    }

    fn base_memory_root(&self) -> PathBuf {
        self.project.control_directory().join(MEMORY_DIRECTORY)
    }

    fn ensure_memory_root(&self) -> Result<(), LocalProjectMemoryError> {
        let base = self.base_memory_root();
        ensure_directory(&base)?;
        if self.scope.namespace_path().is_some() {
            ensure_directory(&base.join("agents"))?;
        }
        ensure_directory(&self.memory_root())
    }

    fn validate_memory_root_existing(&self) -> Result<bool, LocalProjectMemoryError> {
        let base = self.base_memory_root();
        if !validate_existing_directory(&base)? {
            return Ok(false);
        }
        if self.scope.namespace_path().is_some() {
            let agents = base.join("agents");
            if !validate_existing_directory(&agents)? {
                return Ok(false);
            }
            return validate_existing_directory(&self.memory_root());
        }
        Ok(true)
    }

    fn document_path(&self, path: &MemoryPath) -> PathBuf {
        self.memory_root().join(path.as_str())
    }

    fn retrieval_note_paths(&self) -> Result<Vec<MemoryPath>, LocalProjectMemoryError> {
        let notes = self.memory_root().join("notes");
        let entries = match fs::read_dir(&notes) {
            Ok(entries) => entries,
            Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => {
                return Err(LocalProjectMemoryError::Inspect {
                    path: notes,
                    source,
                });
            }
        };
        let mut paths = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|source| LocalProjectMemoryError::Inspect {
                path: notes.clone(),
                source,
            })?;
            let path = entry.path();
            let metadata =
                fs::symlink_metadata(&path).map_err(|source| LocalProjectMemoryError::Inspect {
                    path: path.clone(),
                    source,
                })?;
            if metadata.file_type().is_symlink() {
                return Err(LocalProjectMemoryError::Symlink { path });
            }
            if !metadata.is_file() {
                return Err(LocalProjectMemoryError::NotRegular { path });
            }
            let name = entry.file_name().into_string().map_err(|_| {
                LocalProjectMemoryError::InvalidRetrievalPath {
                    path: path.display().to_string(),
                }
            })?;
            let memory_path = MemoryPath::parse(format!("notes/{name}")).map_err(|_| {
                LocalProjectMemoryError::InvalidRetrievalPath {
                    path: path.display().to_string(),
                }
            })?;
            paths.push(memory_path);
        }
        paths.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        Ok(paths)
    }

    fn validate_retrieval_document_size(
        &self,
        path: &MemoryPath,
    ) -> Result<(), LocalProjectMemoryError> {
        let document_path = self.document_path(path);
        match fs::symlink_metadata(&document_path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                Err(LocalProjectMemoryError::Symlink {
                    path: document_path,
                })
            }
            Ok(metadata) if !metadata.is_file() => Err(LocalProjectMemoryError::NotRegular {
                path: document_path,
            }),
            Ok(metadata) if metadata.len() > MAX_RETRIEVAL_MEMORY_DOCUMENT_BYTES => {
                Err(LocalProjectMemoryError::RetrievalDocumentTooLarge {
                    path: document_path,
                    size_bytes: metadata.len(),
                    max_bytes: MAX_RETRIEVAL_MEMORY_DOCUMENT_BYTES,
                })
            }
            Ok(_) | Err(_) => Ok(()),
        }
    }

    pub(crate) fn validate_content(
        &self,
        path: &MemoryPath,
        content: &str,
    ) -> Result<(), LocalProjectMemoryError> {
        self.validate_size(path, content.len() as u64)
    }

    fn validate_size(
        &self,
        path: &MemoryPath,
        size_bytes: u64,
    ) -> Result<(), LocalProjectMemoryError> {
        if path.as_str() == "core.md" && size_bytes > self.core_byte_limit as u64 {
            Err(LocalProjectMemoryError::CoreTooLarge {
                path: self.document_path(path),
                size_bytes,
                max_bytes: self.core_byte_limit,
            })
        } else {
            Ok(())
        }
    }

    fn ensure_control_directory_existing(&self) -> Result<(), LocalProjectMemoryError> {
        let path = self.project.control_directory();
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                Err(LocalProjectMemoryError::Symlink { path })
            }
            Ok(metadata) if !metadata.file_type().is_dir() => {
                Err(LocalProjectMemoryError::NotDirectory { path })
            }
            Ok(_) => Ok(()),
            Err(source) => Err(LocalProjectMemoryError::Inspect { path, source }),
        }
    }

    fn ensure_note_parent_existing(
        &self,
        path: &MemoryPath,
    ) -> Result<(), LocalProjectMemoryError> {
        if !path.as_str().starts_with("notes/") {
            return Ok(());
        }
        let notes = self.memory_root().join("notes");
        match fs::symlink_metadata(&notes) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                Err(LocalProjectMemoryError::Symlink { path: notes })
            }
            Ok(metadata) if !metadata.file_type().is_dir() => {
                Err(LocalProjectMemoryError::NotDirectory { path: notes })
            }
            Ok(_) => Ok(()),
            Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(LocalProjectMemoryError::Inspect {
                path: notes,
                source,
            }),
        }
    }
}

pub(crate) fn content_hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

/// Produces a small deterministic replacement Diff for an auditable revision.
/// It intentionally favours complete, self-contained evidence over a complex
/// patch language that would need a separate parser in this early slice.
pub(crate) fn replacement_diff(old: Option<&str>, new: Option<&str>) -> String {
    audit_replacement_diff(old, new, "previous", "current")
}

fn ensure_directory(path: &Path) -> Result<(), LocalProjectMemoryError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(LocalProjectMemoryError::Symlink {
                path: path.to_path_buf(),
            })
        }
        Ok(metadata) if metadata.file_type().is_dir() => Ok(()),
        Ok(_) => Err(LocalProjectMemoryError::NotDirectory {
            path: path.to_path_buf(),
        }),
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            fs::create_dir(path).map_err(|source| LocalProjectMemoryError::CreateDirectory {
                path: path.to_path_buf(),
                source,
            })?;
            let parent = path
                .parent()
                .ok_or_else(|| LocalProjectMemoryError::CreateDirectory {
                    path: path.to_path_buf(),
                    source: io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "Memory directory has no parent",
                    ),
                })?;
            sync_directory(parent).map_err(|error| LocalProjectMemoryError::PersistDirectory {
                path: path.to_path_buf(),
                error: Box::new(error),
            })?;
            ensure_directory(path)
        }
        Err(source) => Err(LocalProjectMemoryError::Inspect {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn validate_existing_directory(path: &Path) -> Result<bool, LocalProjectMemoryError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(LocalProjectMemoryError::Symlink {
                path: path.to_path_buf(),
            })
        }
        Ok(metadata) if metadata.file_type().is_dir() => Ok(true),
        Ok(_) => Err(LocalProjectMemoryError::NotDirectory {
            path: path.to_path_buf(),
        }),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(LocalProjectMemoryError::Inspect {
            path: path.to_path_buf(),
            source,
        }),
    }
}

#[derive(Debug, Error)]
pub(crate) enum LocalProjectMemoryError {
    #[error("Memory path `{path}` must not be a symlink")]
    Symlink { path: PathBuf },
    #[error("Memory path `{path}` must be a directory")]
    NotDirectory { path: PathBuf },
    #[error("Memory document `{path}` must be a regular file")]
    NotRegular { path: PathBuf },
    #[error("inspect Memory path `{path}`: {source}")]
    Inspect {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("read Memory document `{path}`: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(
        "core Memory document `{path}` is {size_bytes} bytes, above the {max_bytes}-byte limit"
    )]
    CoreTooLarge {
        path: PathBuf,
        size_bytes: u64,
        max_bytes: usize,
    },
    #[error("Memory document `{path}` is not UTF-8 Markdown: {source}")]
    NotUtf8 {
        path: PathBuf,
        #[source]
        source: std::string::FromUtf8Error,
    },
    #[error(
        "retrieval Memory document `{path}` is {size_bytes} bytes, above the {max_bytes}-byte limit"
    )]
    RetrievalDocumentTooLarge {
        path: PathBuf,
        size_bytes: u64,
        max_bytes: u64,
    },
    #[error("retrieval Memory path `{path}` is outside the supported project Memory vocabulary")]
    InvalidRetrievalPath { path: String },
    #[error("create Memory directory `{path}`: {source}")]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("persist Memory directory `{path}`: {error}")]
    PersistDirectory {
        path: PathBuf,
        #[source]
        error: Box<crate::adapters::project_event_store::ProjectEventStoreError>,
    },
    #[error("durably replace Memory document `{path}`: {error}")]
    DurableReplace {
        path: PathBuf,
        #[source]
        error: Box<crate::adapters::project_event_store::ProjectEventStoreError>,
    },
}
