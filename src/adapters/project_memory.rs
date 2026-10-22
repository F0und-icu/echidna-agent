//! Local Markdown Memory filesystem boundary.
//!
//! This adapter owns path confinement, symlink rejection, BLAKE3 content
//! hashes and durable Markdown replacement. Event ordering remains with the
//! fixed Memory tools so that no filesystem helper becomes a transaction API.

use std::fs;
use std::io;
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
}

impl<'project> LocalProjectMemory<'project> {
    pub(crate) fn new(project: &'project Project) -> Self {
        Self::with_scope(project, MemoryScope::Project)
    }

    pub(crate) fn with_scope(project: &'project Project, scope: MemoryScope) -> Self {
        Self { project, scope }
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

    fn memory_root(&self) -> PathBuf {
        let memory_root = self.base_memory_root();
        match self.scope.namespace_path() {
            Some(agent_id) => memory_root.join("agents").join(agent_id),
            None => memory_root,
        }
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
        if path.as_str() == "core.md" && size_bytes > MAX_CORE_MEMORY_BYTES as u64 {
            Err(LocalProjectMemoryError::CoreTooLarge {
                path: self.document_path(path),
                size_bytes,
                max_bytes: MAX_CORE_MEMORY_BYTES,
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
