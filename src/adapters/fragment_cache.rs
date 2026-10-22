//! Project-isolated persistence for rebuildable rendered Context Fragments.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::adapters::project_event_store::{
    Project, ProjectEventStoreError, replace_durable_file, sync_directory,
};
use crate::domain::canonical_json::canonical_blake3_hash;
use crate::domain::context::{
    ContextFragment, ContextFragmentCacheKey, ContextFragmentContent, ContextFragmentId,
    ContextFragmentPriority, ContextFragmentSource, FragmentCachePolicy,
};
use crate::domain::project::ProjectId;
use crate::ports::context::{ContextFragmentCache, ContextFragmentCacheError};

const CACHE_SCHEMA_VERSION: u16 = 2;
const CACHE_DIRECTORY: &str = "cache";
const CONTEXT_FRAGMENTS_DIRECTORY: &str = "context-fragments";

/// The filesystem Adapter for derived Context Fragment reuse. Project facts
/// and rendered Fragment sources remain authoritative outside this directory.
#[derive(Clone, Debug)]
pub struct LocalFragmentCache {
    project: Project,
}

impl LocalFragmentCache {
    pub fn for_project(project: &Project) -> Self {
        Self {
            project: project.clone(),
        }
    }

    fn cache_directory_existing(&self) -> Result<Option<PathBuf>, LocalFragmentCacheError> {
        let control = self.project.control_directory();
        if directory_if_exists(&control)?.is_none() {
            return Ok(None);
        }
        let cache = control.join(CACHE_DIRECTORY);
        if directory_if_exists(&cache)?.is_none() {
            return Ok(None);
        }
        let fragments = cache.join(CONTEXT_FRAGMENTS_DIRECTORY);
        directory_if_exists(&fragments)
    }

    fn ensure_cache_directory(&self) -> Result<PathBuf, LocalFragmentCacheError> {
        let control = self.project.control_directory();
        if directory_if_exists(&control)?.is_none() {
            return Err(LocalFragmentCacheError::ControlDirectoryMissing { path: control });
        }
        let cache = control.join(CACHE_DIRECTORY);
        ensure_cache_directory(&cache)?;
        let fragments = cache.join(CONTEXT_FRAGMENTS_DIRECTORY);
        ensure_cache_directory(&fragments)?;
        Ok(fragments)
    }
}

impl LocalFragmentCache {
    fn lookup_local(
        &self,
        key: &ContextFragmentCacheKey,
    ) -> Result<Option<ContextFragment>, LocalFragmentCacheError> {
        if key.cache_policy() == FragmentCachePolicy::DoNotCache {
            return Ok(None);
        }
        let Some(directory) = self.cache_directory_existing()? else {
            return Ok(None);
        };
        let path = directory.join(format!("{}.json", key.as_str()));
        let Some(bytes) = regular_file_bytes(&path)? else {
            return Ok(None);
        };
        let Some(entry) = decode_cache_entry(&bytes) else {
            return Ok(None);
        };
        if entry.cache_schema_version != CACHE_SCHEMA_VERSION
            || entry.project_id != *self.project.project_id()
            || entry.cache_key != key.as_str()
            || entry.payload_checksum().ok().as_deref() != Some(entry.payload_checksum.as_str())
        {
            return Ok(None);
        }
        let Some(fragment) = entry.fragment.reconstruct() else {
            return Ok(None);
        };
        if !key.matches_fragment(&fragment) {
            return Ok(None);
        }
        Ok(Some(fragment))
    }

    fn store_local(
        &self,
        key: &ContextFragmentCacheKey,
        fragment: &ContextFragment,
    ) -> Result<(), LocalFragmentCacheError> {
        if !key.matches_fragment(fragment) {
            return Err(LocalFragmentCacheError::FragmentIdentityMismatch {
                cache_key: key.as_str().to_owned(),
                fragment_id: fragment.id().clone(),
            });
        }
        if key.cache_policy() == FragmentCachePolicy::DoNotCache {
            return Ok(());
        }
        let entry = FragmentCacheEntry::from_fragment(
            self.project.project_id().clone(),
            key.as_str().to_owned(),
            fragment,
        )?;
        let bytes = serde_json::to_vec(&entry)
            .map_err(|source| LocalFragmentCacheError::Encode { source })?;

        // Derived cache directory creation and durable replacement share the
        // one Project writer authority with every other local persisted path.
        let _writer =
            self.project
                .open_writer()
                .map_err(|source| LocalFragmentCacheError::Writer {
                    source: Box::new(source),
                })?;
        let directory = self.ensure_cache_directory()?;
        let path = directory.join(format!("{}.json", key.as_str()));
        replace_durable_file(&path, &bytes).map_err(|source| {
            LocalFragmentCacheError::PersistEntry {
                path,
                source: Box::new(source),
            }
        })
    }
}

impl ContextFragmentCache for LocalFragmentCache {
    fn lookup(
        &self,
        key: &ContextFragmentCacheKey,
    ) -> Result<Option<ContextFragment>, ContextFragmentCacheError> {
        self.lookup_local(key)
            .map_err(|error| ContextFragmentCacheError::lookup(&error))
    }

    fn store(
        &self,
        key: &ContextFragmentCacheKey,
        fragment: &ContextFragment,
    ) -> Result<(), ContextFragmentCacheError> {
        self.store_local(key, fragment)
            .map_err(|error| ContextFragmentCacheError::store(&error))
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct FragmentCacheEntry {
    cache_schema_version: u16,
    project_id: ProjectId,
    cache_key: String,
    fragment: PersistedFragment,
    payload_checksum: String,
}

impl FragmentCacheEntry {
    fn from_fragment(
        project_id: ProjectId,
        cache_key: String,
        fragment: &ContextFragment,
    ) -> Result<Self, LocalFragmentCacheError> {
        let fragment = PersistedFragment {
            id: fragment.id().as_str().to_owned(),
            source: fragment.source(),
            priority: fragment.priority(),
            cache_policy: fragment.cache_policy(),
            content_hash: fragment.content_hash().as_str().to_owned(),
            token_estimate: fragment.token_estimate(),
            content: fragment.content().clone(),
            artifact_reference: fragment.artifact_reference().cloned(),
        };
        let mut entry = Self {
            cache_schema_version: CACHE_SCHEMA_VERSION,
            project_id,
            cache_key,
            fragment,
            payload_checksum: String::new(),
        };
        entry.payload_checksum = entry.payload_checksum()?;
        Ok(entry)
    }

    fn payload_checksum(&self) -> Result<String, LocalFragmentCacheError> {
        canonical_blake3_hash(&FragmentCachePayload {
            cache_schema_version: self.cache_schema_version,
            project_id: &self.project_id,
            cache_key: &self.cache_key,
            fragment: &self.fragment,
        })
        .map_err(|source| LocalFragmentCacheError::Encode { source })
    }
}

#[derive(Serialize)]
struct FragmentCachePayload<'entry> {
    cache_schema_version: u16,
    project_id: &'entry ProjectId,
    cache_key: &'entry str,
    fragment: &'entry PersistedFragment,
}

/// This raw DTO is intentionally reconstructed through the validated Domain
/// constructor rather than deserializing `ContextFragment` directly.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PersistedFragment {
    id: String,
    source: ContextFragmentSource,
    priority: ContextFragmentPriority,
    cache_policy: FragmentCachePolicy,
    content_hash: String,
    token_estimate: u32,
    content: ContextFragmentContent,
    artifact_reference: Option<crate::domain::artifacts::ArtifactId>,
}

impl PersistedFragment {
    fn reconstruct(self) -> Option<ContextFragment> {
        let fragment = ContextFragment::with_artifact_reference(
            ContextFragmentId::new(self.id).ok()?,
            self.source,
            self.priority,
            self.cache_policy,
            self.content,
            self.artifact_reference,
        )
        .ok()?;
        (fragment.content_hash().as_str() == self.content_hash
            && fragment.token_estimate() == self.token_estimate)
            .then_some(fragment)
    }
}

fn directory_if_exists(path: &Path) -> Result<Option<PathBuf>, LocalFragmentCacheError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(LocalFragmentCacheError::CacheSymlink {
                path: path.to_path_buf(),
            })
        }
        Ok(metadata) if metadata.file_type().is_dir() => Ok(Some(path.to_path_buf())),
        Ok(_) => Err(LocalFragmentCacheError::CacheNotDirectory {
            path: path.to_path_buf(),
        }),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(LocalFragmentCacheError::Inspect {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn ensure_cache_directory(path: &Path) -> Result<(), LocalFragmentCacheError> {
    match directory_if_exists(path)? {
        Some(_) => Ok(()),
        None => {
            fs::create_dir(path).map_err(|source| LocalFragmentCacheError::CreateDirectory {
                path: path.to_path_buf(),
                source,
            })?;
            let parent = path
                .parent()
                .ok_or_else(|| LocalFragmentCacheError::CreateDirectory {
                    path: path.to_path_buf(),
                    source: io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "fragment cache directory has no parent",
                    ),
                })?;
            sync_directory(parent).map_err(|source| LocalFragmentCacheError::PersistDirectory {
                path: path.to_path_buf(),
                source: Box::new(source),
            })?;
            ensure_cache_directory(path)
        }
    }
}

fn regular_file_bytes(path: &Path) -> Result<Option<Vec<u8>>, LocalFragmentCacheError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(LocalFragmentCacheError::CacheSymlink {
                path: path.to_path_buf(),
            })
        }
        Ok(metadata) if metadata.file_type().is_file() => {
            fs::read(path)
                .map(Some)
                .map_err(|source| LocalFragmentCacheError::Read {
                    path: path.to_path_buf(),
                    source,
                })
        }
        Ok(_) => Err(LocalFragmentCacheError::CacheNotRegular {
            path: path.to_path_buf(),
        }),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(LocalFragmentCacheError::Inspect {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// A cache file must round-trip exactly through its private DTO. This keeps
/// Serde's permissive nested enum decoding from accepting an entry with an
/// unknown nested field as if it were current cache data.
fn decode_cache_entry(bytes: &[u8]) -> Option<FragmentCacheEntry> {
    let value = serde_json::from_slice::<serde_json::Value>(bytes).ok()?;
    let entry = serde_json::from_value::<FragmentCacheEntry>(value.clone()).ok()?;
    (serde_json::to_value(&entry).ok()? == value).then_some(entry)
}

#[derive(Debug, Error)]
enum LocalFragmentCacheError {
    #[error("fragment cache path `{path}` must not be a symlink")]
    CacheSymlink { path: PathBuf },
    #[error("fragment cache path `{path}` must be a directory")]
    CacheNotDirectory { path: PathBuf },
    #[error("fragment cache entry `{path}` must be a regular file")]
    CacheNotRegular { path: PathBuf },
    #[error("project control directory `{path}` is missing")]
    ControlDirectoryMissing { path: PathBuf },
    #[error("inspect fragment cache path `{path}`: {source}")]
    Inspect {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("read fragment cache entry `{path}`: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("create fragment cache directory `{path}`: {source}")]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("persist fragment cache directory `{path}`: {source}")]
    PersistDirectory {
        path: PathBuf,
        #[source]
        source: Box<ProjectEventStoreError>,
    },
    #[error("acquire fragment cache project writer: {source}")]
    Writer {
        #[source]
        source: Box<ProjectEventStoreError>,
    },
    #[error("persist fragment cache entry `{path}`: {source}")]
    PersistEntry {
        path: PathBuf,
        #[source]
        source: Box<ProjectEventStoreError>,
    },
    #[error("encode fragment cache entry: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("fragment `{fragment_id}` does not match cache key `{cache_key}`")]
    FragmentIdentityMismatch {
        cache_key: String,
        fragment_id: ContextFragmentId,
    },
}
