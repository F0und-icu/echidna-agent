//! Local rebuildable project-inspection cache lifecycle.
//!
//! `fingerprint` owns project metadata traversal and content verification.
//! This module owns only cache validation, durable cache persistence and the
//! fixed adapter result returned to `inspect_project`.

mod fingerprint;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;
use thiserror::Error;

use crate::adapters::project_event_store::{Project, replace_durable_file, sync_directory};
use crate::domain::snapshot::{
    FileManifest, FileManifestEntry, ProjectFingerprint, ProjectSnapshot,
    SNAPSHOT_CACHE_SCHEMA_VERSION, ToolCapabilityCache, tool_capabilities,
};
use crate::domain::tools::ToolSpec;

use fingerprint::{apply_second_level, first_level};

const CACHE_DIRECTORY: &str = "cache";
const SNAPSHOT_FILE: &str = "project-snapshot.json";
const MANIFEST_FILE: &str = "file-manifest.json";
const TOOL_CAPABILITIES_FILE: &str = "tool-capabilities.json";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CacheDisposition {
    Hit,
    IncrementalRefresh { reason: String },
    FullRebuild { reason: String },
}

#[derive(Clone, Debug)]
pub(crate) struct InspectionResult {
    pub(crate) disposition: CacheDisposition,
    pub(crate) snapshot: ProjectSnapshot,
    pub(crate) manifest: FileManifest,
    pub(crate) tool_capabilities: ToolCapabilityCache,
    pub(crate) hashed_paths: Vec<String>,
}

/// The result of validating the existing Project Snapshot cache. Unlike
/// [`InspectionResult`], this value is produced without rebuilding or
/// persisting any cache file.
#[derive(Clone, Debug)]
pub(crate) struct ReadOnlyInspectionResult {
    pub(crate) disposition: CacheDisposition,
    pub(crate) source_fingerprint: ProjectFingerprint,
    pub(crate) snapshot: Option<ProjectSnapshot>,
}

pub(crate) struct LocalProjectSnapshot<'project> {
    project: &'project Project,
}

impl<'project> LocalProjectSnapshot<'project> {
    pub(crate) fn new(project: &'project Project) -> Self {
        Self { project }
    }

    pub(crate) fn inspect(
        &self,
        specs: &[ToolSpec],
        hash_paths: &[String],
        created_at: &str,
    ) -> Result<InspectionResult, LocalProjectSnapshotError> {
        ensure_control_directory(self.project)?;
        let first_level = first_level(self.project)?;
        let fingerprint = first_level.fingerprint;
        let expected_tool_capabilities = tool_capabilities(specs);
        if fingerprint.cacheable
            && let Some(hit) =
                self.load_complete_cache(&fingerprint, &expected_tool_capabilities)?
        {
            if hash_paths.is_empty() {
                return Ok(InspectionResult {
                    disposition: CacheDisposition::Hit,
                    snapshot: hit.snapshot,
                    manifest: hit.manifest,
                    tool_capabilities: hit.tool_capabilities,
                    hashed_paths: Vec::new(),
                });
            }
            // A cheap fingerprint is not content identity. Verify only the
            // requested paths, without another full tree walk or a second
            // hashing owner. An unchanged verification needs no cache write.
            let (entries, hashed_paths) = apply_second_level(
                self.project.root(),
                hit.manifest.entries.clone(),
                Some(&hit.manifest),
                hash_paths,
            )?;
            if entries == hit.manifest.entries {
                return Ok(InspectionResult {
                    disposition: CacheDisposition::Hit,
                    snapshot: hit.snapshot,
                    manifest: hit.manifest,
                    tool_capabilities: hit.tool_capabilities,
                    hashed_paths,
                });
            }
            return self.persist_inspection(
                CacheDisposition::IncrementalRefresh {
                    reason: "explicit_hash_paths".to_owned(),
                },
                fingerprint,
                entries,
                expected_tool_capabilities,
                hashed_paths,
                created_at,
            );
        }
        let reason = if fingerprint.cacheable {
            self.cache_miss_reason(&fingerprint)?
        } else {
            "git_parse_failed".to_owned()
        };
        let prior_manifest = self.load_verified_manifest_for_reuse()?;
        let entries = match first_level.entries {
            Some(entries) => entries,
            None => fingerprint::collect_entries(self.project)?,
        };
        let (entries, hashed_paths) = apply_second_level(
            self.project.root(),
            entries,
            prior_manifest.as_ref(),
            hash_paths,
        )?;
        self.persist_inspection(
            CacheDisposition::FullRebuild {
                reason: reason.clone(),
            },
            fingerprint,
            entries,
            expected_tool_capabilities,
            hashed_paths,
            created_at,
        )
    }

    /// Validates only the current cache and its cheap source fingerprint.
    /// This is the inspection CLI's read-only path: it never calls
    /// `persist_inspection`, creates a directory, or replaces a cache file.
    pub(crate) fn inspect_read_only(
        &self,
        specs: &[ToolSpec],
    ) -> Result<ReadOnlyInspectionResult, LocalProjectSnapshotError> {
        ensure_control_directory(self.project)?;
        let fingerprint = first_level(self.project)?.fingerprint;
        let expected_tool_capabilities = tool_capabilities(specs);
        if fingerprint.cacheable
            && let Some(cache) =
                self.load_complete_cache(&fingerprint, &expected_tool_capabilities)?
        {
            return Ok(ReadOnlyInspectionResult {
                // `rebuild_reason` describes how this cache was produced. It
                // must not change the current observation: a complete cache
                // matching the current source fingerprint is a hit.
                disposition: CacheDisposition::Hit,
                source_fingerprint: fingerprint,
                snapshot: Some(cache.snapshot),
            });
        }
        let reason = if fingerprint.cacheable {
            self.cache_miss_reason(&fingerprint)?
        } else {
            "git_parse_failed".to_owned()
        };
        Ok(ReadOnlyInspectionResult {
            disposition: CacheDisposition::FullRebuild { reason },
            source_fingerprint: fingerprint,
            snapshot: None,
        })
    }

    fn persist_inspection(
        &self,
        disposition: CacheDisposition,
        fingerprint: ProjectFingerprint,
        entries: Vec<FileManifestEntry>,
        expected_tool_capabilities: Vec<crate::domain::snapshot::ToolCapability>,
        hashed_paths: Vec<String>,
        created_at: &str,
    ) -> Result<InspectionResult, LocalProjectSnapshotError> {
        let reason = match &disposition {
            CacheDisposition::IncrementalRefresh { reason }
            | CacheDisposition::FullRebuild { reason } => reason.clone(),
            CacheDisposition::Hit => unreachable!("a cache hit does not rebuild cache files"),
        };
        let manifest = FileManifest {
            cache_schema_version: SNAPSHOT_CACHE_SCHEMA_VERSION,
            project_id: self.project.project_id().clone(),
            source_fingerprint: fingerprint.clone(),
            created_at: created_at.to_owned(),
            rebuild_reason: reason.clone(),
            entries,
        };
        let tool_capabilities = ToolCapabilityCache {
            cache_schema_version: SNAPSHOT_CACHE_SCHEMA_VERSION,
            project_id: self.project.project_id().clone(),
            source_fingerprint: fingerprint.clone(),
            created_at: created_at.to_owned(),
            rebuild_reason: reason.clone(),
            tools: expected_tool_capabilities,
        };
        let manifest_bytes = encode_cache(&manifest)?;
        let tool_capability_bytes = encode_cache(&tool_capabilities)?;
        let snapshot = ProjectSnapshot {
            cache_schema_version: SNAPSHOT_CACHE_SCHEMA_VERSION,
            project_id: self.project.project_id().clone(),
            source_fingerprint: fingerprint,
            created_at: created_at.to_owned(),
            rebuild_reason: reason,
            file_manifest_hash: byte_hash(&manifest_bytes),
            tool_capability_hash: byte_hash(&tool_capability_bytes),
            file_count: manifest.entries.len(),
        };
        self.persist_cache(&manifest_bytes, &tool_capability_bytes, &snapshot)?;
        Ok(InspectionResult {
            disposition,
            snapshot,
            manifest,
            tool_capabilities,
            hashed_paths,
        })
    }

    fn load_complete_cache(
        &self,
        fingerprint: &ProjectFingerprint,
        expected_tool_capabilities: &[crate::domain::snapshot::ToolCapability],
    ) -> Result<Option<CompleteCache>, LocalProjectSnapshotError> {
        let Some(cache_directory) = self.cache_directory_existing()? else {
            return Ok(None);
        };
        let manifest_bytes = match read_cache_file(&cache_directory.join(MANIFEST_FILE))? {
            Some(bytes) => bytes,
            None => return Ok(None),
        };
        let tool_capability_bytes =
            match read_cache_file(&cache_directory.join(TOOL_CAPABILITIES_FILE))? {
                Some(bytes) => bytes,
                None => return Ok(None),
            };
        let snapshot_bytes = match read_cache_file(&cache_directory.join(SNAPSHOT_FILE))? {
            Some(bytes) => bytes,
            None => return Ok(None),
        };
        let manifest: FileManifest = match decode_cache(&manifest_bytes) {
            Ok(cache) => cache,
            Err(_) => return Ok(None),
        };
        let tool_capabilities: ToolCapabilityCache = match decode_cache(&tool_capability_bytes) {
            Ok(cache) => cache,
            Err(_) => return Ok(None),
        };
        let snapshot: ProjectSnapshot = match decode_cache(&snapshot_bytes) {
            Ok(cache) => cache,
            Err(_) => return Ok(None),
        };
        let expected_project_id = self.project.project_id();
        let valid = [
            manifest.cache_schema_version == SNAPSHOT_CACHE_SCHEMA_VERSION,
            tool_capabilities.cache_schema_version == SNAPSHOT_CACHE_SCHEMA_VERSION,
            snapshot.cache_schema_version == SNAPSHOT_CACHE_SCHEMA_VERSION,
            &manifest.project_id == expected_project_id,
            &tool_capabilities.project_id == expected_project_id,
            &snapshot.project_id == expected_project_id,
            manifest.source_fingerprint == *fingerprint,
            tool_capabilities.source_fingerprint == *fingerprint,
            snapshot.source_fingerprint == *fingerprint,
            snapshot.file_manifest_hash == byte_hash(&manifest_bytes),
            snapshot.tool_capability_hash == byte_hash(&tool_capability_bytes),
            snapshot.file_count == manifest.entries.len(),
            manifest.created_at == tool_capabilities.created_at,
            manifest.created_at == snapshot.created_at,
            manifest.rebuild_reason == tool_capabilities.rebuild_reason,
            manifest.rebuild_reason == snapshot.rebuild_reason,
            tool_capabilities.tools == expected_tool_capabilities,
        ]
        .into_iter()
        .all(|value| value);
        Ok(valid.then_some(CompleteCache {
            snapshot,
            manifest,
            tool_capabilities,
        }))
    }

    fn cache_miss_reason(
        &self,
        fingerprint: &ProjectFingerprint,
    ) -> Result<String, LocalProjectSnapshotError> {
        let Some(cache_directory) = self.cache_directory_existing()? else {
            return Ok("cache_absent".to_owned());
        };
        let snapshot_path = cache_directory.join(SNAPSHOT_FILE);
        let Some(snapshot_bytes) = read_cache_file(&snapshot_path)? else {
            return Ok("snapshot_missing".to_owned());
        };
        let Ok(snapshot) = decode_cache::<ProjectSnapshot>(&snapshot_bytes) else {
            return Ok("snapshot_corrupt".to_owned());
        };
        if snapshot.cache_schema_version != SNAPSHOT_CACHE_SCHEMA_VERSION {
            return Ok("snapshot_schema_incompatible".to_owned());
        }
        if snapshot.project_id != *self.project.project_id() {
            return Ok("project_identity_changed".to_owned());
        }
        if snapshot.source_fingerprint != *fingerprint {
            return Ok("source_fingerprint_changed".to_owned());
        }
        Ok("cache_incomplete_or_corrupt".to_owned())
    }

    fn load_verified_manifest_for_reuse(
        &self,
    ) -> Result<Option<FileManifest>, LocalProjectSnapshotError> {
        let Some(cache_directory) = self.cache_directory_existing()? else {
            return Ok(None);
        };
        let Some(manifest_bytes) = read_cache_file(&cache_directory.join(MANIFEST_FILE))? else {
            return Ok(None);
        };
        let Some(snapshot_bytes) = read_cache_file(&cache_directory.join(SNAPSHOT_FILE))? else {
            return Ok(None);
        };
        let Ok(manifest) = decode_cache::<FileManifest>(&manifest_bytes) else {
            return Ok(None);
        };
        let Ok(snapshot) = decode_cache::<ProjectSnapshot>(&snapshot_bytes) else {
            return Ok(None);
        };
        if manifest.cache_schema_version == SNAPSHOT_CACHE_SCHEMA_VERSION
            && manifest.project_id == *self.project.project_id()
            && snapshot.cache_schema_version == SNAPSHOT_CACHE_SCHEMA_VERSION
            && snapshot.project_id == *self.project.project_id()
            && manifest.source_fingerprint == snapshot.source_fingerprint
            && snapshot.file_manifest_hash == byte_hash(&manifest_bytes)
            && snapshot.file_count == manifest.entries.len()
        {
            Ok(Some(manifest))
        } else {
            Ok(None)
        }
    }

    fn persist_cache(
        &self,
        manifest: &[u8],
        tool_capabilities: &[u8],
        snapshot: &ProjectSnapshot,
    ) -> Result<(), LocalProjectSnapshotError> {
        let cache_directory = self.ensure_cache_directory()?;
        replace_cache_file(&cache_directory.join(MANIFEST_FILE), manifest)?;
        replace_cache_file(
            &cache_directory.join(TOOL_CAPABILITIES_FILE),
            tool_capabilities,
        )?;
        let snapshot = encode_cache(snapshot)?;
        replace_cache_file(&cache_directory.join(SNAPSHOT_FILE), &snapshot)
    }

    fn cache_directory_existing(&self) -> Result<Option<PathBuf>, LocalProjectSnapshotError> {
        let path = self.project.control_directory().join(CACHE_DIRECTORY);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                Err(LocalProjectSnapshotError::CacheSymlink { path })
            }
            Ok(metadata) if metadata.file_type().is_dir() => Ok(Some(path)),
            Ok(_) => Err(LocalProjectSnapshotError::CacheNotDirectory { path }),
            Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(LocalProjectSnapshotError::Inspect { path, source }),
        }
    }

    fn ensure_cache_directory(&self) -> Result<PathBuf, LocalProjectSnapshotError> {
        let control = self.project.control_directory();
        ensure_directory(&control)?;
        let cache = control.join("cache");
        ensure_directory(&cache)?;
        Ok(cache)
    }
}

struct CompleteCache {
    snapshot: ProjectSnapshot,
    manifest: FileManifest,
    tool_capabilities: ToolCapabilityCache,
}

fn ensure_directory(path: &Path) -> Result<(), LocalProjectSnapshotError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(LocalProjectSnapshotError::CacheSymlink {
                path: path.to_path_buf(),
            })
        }
        Ok(metadata) if metadata.file_type().is_dir() => Ok(()),
        Ok(_) => Err(LocalProjectSnapshotError::CacheNotDirectory {
            path: path.to_path_buf(),
        }),
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            fs::create_dir(path).map_err(|source| LocalProjectSnapshotError::CreateDirectory {
                path: path.to_path_buf(),
                source,
            })?;
            let parent =
                path.parent()
                    .ok_or_else(|| LocalProjectSnapshotError::CreateDirectory {
                        path: path.to_path_buf(),
                        source: io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "Cache directory has no parent",
                        ),
                    })?;
            sync_directory(parent).map_err(|error| {
                LocalProjectSnapshotError::PersistDirectory {
                    path: path.to_path_buf(),
                    error: Box::new(error),
                }
            })?;
            ensure_directory(path)
        }
        Err(source) => Err(LocalProjectSnapshotError::Inspect {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn ensure_control_directory(project: &Project) -> Result<(), LocalProjectSnapshotError> {
    let path = project.control_directory();
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(LocalProjectSnapshotError::ControlDirectorySymlink { path })
        }
        Ok(metadata) if !metadata.file_type().is_dir() => {
            Err(LocalProjectSnapshotError::ControlDirectoryNotDirectory { path })
        }
        Ok(_) => Ok(()),
        Err(source) => Err(LocalProjectSnapshotError::Inspect { path, source }),
    }
}

fn read_cache_file(path: &Path) -> Result<Option<Vec<u8>>, LocalProjectSnapshotError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(LocalProjectSnapshotError::CacheSymlink {
                path: path.to_path_buf(),
            })
        }
        Ok(metadata) if !metadata.file_type().is_file() => {
            Err(LocalProjectSnapshotError::CacheNotRegular {
                path: path.to_path_buf(),
            })
        }
        Ok(_) => fs::read(path)
            .map(Some)
            .map_err(|source| LocalProjectSnapshotError::Read {
                path: path.to_path_buf(),
                source,
            }),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(LocalProjectSnapshotError::Inspect {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn replace_cache_file(path: &Path, bytes: &[u8]) -> Result<(), LocalProjectSnapshotError> {
    replace_durable_file(path, bytes).map_err(|error| LocalProjectSnapshotError::Persist {
        path: path.to_path_buf(),
        error: Box::new(error),
    })
}

fn encode_cache<T: Serialize>(value: &T) -> Result<Vec<u8>, LocalProjectSnapshotError> {
    serde_json::to_vec_pretty(value).map_err(|error| LocalProjectSnapshotError::Encode { error })
}

fn decode_cache<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, serde_json::Error> {
    serde_json::from_slice(bytes)
}

fn byte_hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

#[derive(Debug, Error)]
pub(crate) enum LocalProjectSnapshotError {
    #[error("inspect project path `{path}`: {source}")]
    Inspect {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("traverse project directory `{path}`: {source}")]
    Traverse {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("read project file `{path}`: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("cache path `{path}` must not be a symlink")]
    CacheSymlink { path: PathBuf },
    #[error("cache path `{path}` must be a directory")]
    CacheNotDirectory { path: PathBuf },
    #[error("cache file `{path}` must be a regular file")]
    CacheNotRegular { path: PathBuf },
    #[error("project control directory `{path}` must not be a symlink")]
    ControlDirectorySymlink { path: PathBuf },
    #[error("project control directory `{path}` must be a directory")]
    ControlDirectoryNotDirectory { path: PathBuf },
    #[error("required project control file `{path}` is missing")]
    ControlFileMissing { path: PathBuf },
    #[error("required project control file `{path}` must not be a symlink")]
    ControlFileSymlink { path: PathBuf },
    #[error("required project control file `{path}` must be a regular file")]
    ControlFileNotRegular { path: PathBuf },
    #[error("create cache directory `{path}`: {source}")]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("persist cache directory `{path}`: {error}")]
    PersistDirectory {
        path: PathBuf,
        #[source]
        error: Box<crate::adapters::project_event_store::ProjectEventStoreError>,
    },
    #[error("persist cache file `{path}`: {error}")]
    Persist {
        path: PathBuf,
        #[source]
        error: Box<crate::adapters::project_event_store::ProjectEventStoreError>,
    },
    #[error("encode project cache: {error}")]
    Encode { error: serde_json::Error },
    #[error("requested hash path `{path}` escapes the project")]
    InvalidHashPath { path: String },
    #[error("requested hash path `{path}` is not a regular non-symlink file")]
    HashNotRegular { path: PathBuf },
}
