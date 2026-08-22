use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

use crate::domain::events::StreamId;
use crate::domain::project::ProjectId;
use serde::{Deserialize, Serialize};

use super::durable_file::{commit_new_durable_file, sync_directory};
use super::error::ProjectEventStoreError;

pub(crate) const CONTROL_DIRECTORY: &str = ".cyber-agent";
pub(crate) const PROJECT_FILE: &str = "project.toml";
pub(crate) const RESEARCH_FILE: &str = "research.jsonl";
pub(crate) const SESSIONS_DIRECTORY: &str = "sessions";
pub(crate) const RECOVERY_DIRECTORY: &str = "artifacts/recovery";
const WRITER_LOCK_FILE: &str = "writer.lock";
const RESEARCH_STREAM_ID: &str = "research";

/// One initialized, canonicalized Cyber Agent project root.
#[derive(Clone, Debug)]
pub struct Project {
    root: PathBuf,
    project_id: ProjectId,
}

impl Project {
    /// Initializes `root` with the minimum isolated project facts.
    ///
    /// The identifier deterministically derives from the canonical root. The
    /// project configuration is the final durable no-overwrite commit
    /// sentinel: no existing file is opened with truncation semantics.
    pub fn initialize(root: &Path) -> Result<Self, ProjectEventStoreError> {
        let root = canonical_directory(root)?;
        let control_directory = root.join(CONTROL_DIRECTORY);
        let project_file = control_directory.join(PROJECT_FILE);
        if control_directory.exists() {
            return if project_file.exists() {
                Err(ProjectEventStoreError::AlreadyInitialized { project_file })
            } else {
                Err(ProjectEventStoreError::IncompleteInitialization { control_directory })
            };
        }

        let project = Self {
            project_id: project_id_for(&root),
            root,
        };
        let encoded = toml::to_string_pretty(&ProjectToml {
            project_id: project.project_id.clone(),
            canonical_root: project.root.clone(),
        })
        .map_err(ProjectEventStoreError::ProjectTomlEncode)?;

        create_directory(&control_directory)?;
        create_directory(&control_directory.join(SESSIONS_DIRECTORY))?;
        create_directory(&control_directory.join(RECOVERY_DIRECTORY))?;
        create_empty_new(&control_directory.join(RESEARCH_FILE))?;
        commit_new_durable_file(&project_file, encoded.as_bytes())?;
        sync_directory(&project.root)?;

        Ok(project)
    }

    /// Searches `start` and its ancestors for one initialized project.
    pub fn discover(start: &Path) -> Result<Self, ProjectEventStoreError> {
        let mut candidate = canonical_existing_path(start)?;
        if candidate.is_file() {
            candidate = candidate.parent().map(Path::to_path_buf).ok_or_else(|| {
                ProjectEventStoreError::ProjectNotFound {
                    start: start.to_path_buf(),
                }
            })?;
        }

        loop {
            let control_directory = candidate.join(CONTROL_DIRECTORY);
            let project_file = control_directory.join(PROJECT_FILE);
            if control_directory.exists() && !project_file.is_file() {
                return Err(ProjectEventStoreError::IncompleteInitialization { control_directory });
            }
            if project_file.is_file() {
                let encoded = fs::read_to_string(&project_file).map_err(|source| {
                    ProjectEventStoreError::Io {
                        operation: "read project configuration",
                        path: project_file.clone(),
                        source,
                    }
                })?;
                let config: ProjectToml = toml::from_str(&encoded).map_err(|source| {
                    ProjectEventStoreError::ProjectTomlDecode {
                        path: project_file.clone(),
                        source,
                    }
                })?;
                let configured_root = canonical_directory(&config.canonical_root)?;
                if configured_root != candidate {
                    return Err(ProjectEventStoreError::ProjectRootMismatch {
                        project_file,
                        configured_root,
                        discovered_root: candidate,
                    });
                }
                let expected_project_id = project_id_for(&configured_root);
                if config.project_id != expected_project_id {
                    return Err(ProjectEventStoreError::ProjectIdentityMismatch {
                        project_file,
                        configured: config.project_id,
                        expected: expected_project_id,
                    });
                }

                return Ok(Self {
                    root: configured_root,
                    project_id: expected_project_id,
                });
            }

            let Some(parent) = candidate.parent() else {
                break;
            };
            if parent == candidate {
                break;
            }
            candidate = parent.to_path_buf();
        }

        Err(ProjectEventStoreError::ProjectNotFound {
            start: start.to_path_buf(),
        })
    }

    /// Returns the canonical project root used for project identity.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the identifier stored in the isolated project configuration.
    pub fn project_id(&self) -> &ProjectId {
        &self.project_id
    }

    /// Opens the one project-level OS advisory Event Writer lock.
    pub fn open_writer(&self) -> Result<ProjectWriter, ProjectEventStoreError> {
        let lock_path = self.control_directory().join(WRITER_LOCK_FILE);
        let lock_file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|source| ProjectEventStoreError::Io {
                operation: "open project writer lock",
                path: lock_path.clone(),
                source,
            })?;
        match lock_file.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => {
                return Err(ProjectEventStoreError::WriterLocked {
                    project_root: self.root.clone(),
                    source: io::Error::new(io::ErrorKind::WouldBlock, "project writer is locked"),
                });
            }
            Err(std::fs::TryLockError::Error(source)) => {
                return Err(ProjectEventStoreError::Io {
                    operation: "acquire project writer lock",
                    path: lock_path,
                    source,
                });
            }
        }

        Ok(ProjectWriter {
            project: self.clone(),
            _lock_file: lock_file,
        })
    }

    pub(crate) fn control_directory(&self) -> PathBuf {
        self.root.join(CONTROL_DIRECTORY)
    }

    /// Resolves the initialized Research stream without creating a file or
    /// directory. Readers and M2-S2 updates both require this immutable fact
    /// stream to have been created by project initialization.
    pub(crate) fn research_stream_path_existing(&self) -> Result<PathBuf, ProjectEventStoreError> {
        let path = self.control_directory().join(RESEARCH_FILE);
        match fs::metadata(&path) {
            Ok(metadata) if metadata.is_file() => Ok(path),
            Ok(_) => Err(ProjectEventStoreError::InvalidResearchStream { path }),
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                Err(ProjectEventStoreError::MissingResearchStream { path })
            }
            Err(source) => Err(ProjectEventStoreError::Io {
                operation: "inspect initialized Research Event stream",
                path,
                source,
            }),
        }
    }

    pub(crate) fn stream_path(
        &self,
        stream: &ProjectEventStream,
    ) -> Result<PathBuf, ProjectEventStoreError> {
        match stream {
            ProjectEventStream::Research => Ok(self.control_directory().join(RESEARCH_FILE)),
            ProjectEventStream::Session(session_id) => Ok(self
                .control_directory()
                .join(SESSIONS_DIRECTORY)
                .join(session_path_segment(session_id)?)
                .join("events.jsonl")),
        }
    }

    pub(crate) fn session_stream_path_existing(
        &self,
        session_id: &StreamId,
    ) -> Result<PathBuf, ProjectEventStoreError> {
        let path = self.stream_path(&ProjectEventStream::Session(session_id.clone()))?;
        match fs::metadata(&path) {
            Ok(metadata) if metadata.is_file() => Ok(path),
            Ok(_) => Err(ProjectEventStoreError::Io {
                operation: "inspect Session Event stream",
                path,
                source: io::Error::new(io::ErrorKind::InvalidData, "not a regular file"),
            }),
            Err(source) => Err(ProjectEventStoreError::Io {
                operation: "inspect Session Event stream",
                path,
                source,
            }),
        }
    }
}

/// The minimum serialized project configuration, kept at the filesystem edge.
#[derive(Debug, Deserialize, Serialize)]
struct ProjectToml {
    project_id: ProjectId,
    canonical_root: PathBuf,
}

/// The persisted Event stream families used by the local project runtime.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProjectEventStream {
    /// The project-wide versioned Research JSONL stream.
    Research,
    /// One isolated Session audit JSONL stream.
    Session(StreamId),
}

impl ProjectEventStream {
    /// Returns the stable envelope stream identifier.
    pub fn stream_id(&self) -> StreamId {
        match self {
            Self::Research => StreamId::new(RESEARCH_STREAM_ID),
            Self::Session(session_id) => StreamId::new(format!("session:{session_id}")),
        }
    }
}

/// Holder of the one project-level OS advisory writer lock.
pub struct ProjectWriter {
    pub(crate) project: Project,
    _lock_file: File,
}

pub(crate) fn canonical_existing_path(path: &Path) -> Result<PathBuf, ProjectEventStoreError> {
    fs::canonicalize(path).map_err(|source| ProjectEventStoreError::Io {
        operation: "canonicalize project path",
        path: path.to_path_buf(),
        source,
    })
}

pub(crate) fn create_directory(path: &Path) -> Result<(), ProjectEventStoreError> {
    fs::create_dir_all(path).map_err(|source| ProjectEventStoreError::Io {
        operation: "create project directory",
        path: path.to_path_buf(),
        source,
    })
}

pub(crate) fn read_bytes(
    path: &Path,
    operation: &'static str,
) -> Result<Vec<u8>, ProjectEventStoreError> {
    fs::read(path).map_err(|source| ProjectEventStoreError::Io {
        operation,
        path: path.to_path_buf(),
        source,
    })
}

fn canonical_directory(path: &Path) -> Result<PathBuf, ProjectEventStoreError> {
    let canonical = canonical_existing_path(path)?;
    if canonical.is_dir() {
        Ok(canonical)
    } else {
        Err(ProjectEventStoreError::Io {
            operation: "resolve project root directory",
            path: path.to_path_buf(),
            source: io::Error::new(
                io::ErrorKind::NotADirectory,
                "project root is not a directory",
            ),
        })
    }
}

fn project_id_for(root: &Path) -> ProjectId {
    let digest = blake3::hash(root.as_os_str().as_encoded_bytes());
    ProjectId::new(format!("project-{}", digest.to_hex()))
}

fn session_path_segment(session_id: &StreamId) -> Result<&str, ProjectEventStoreError> {
    let value = session_id.as_str();
    if !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        Ok(value)
    } else {
        Err(ProjectEventStoreError::InvalidSessionId {
            session_id: session_id.clone(),
        })
    }
}

fn create_empty_new(path: &Path) -> Result<(), ProjectEventStoreError> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .and_then(|file| file.sync_all())
        .map_err(|source| ProjectEventStoreError::Io {
            operation: "create empty Research Event stream",
            path: path.to_path_buf(),
            source,
        })
}
