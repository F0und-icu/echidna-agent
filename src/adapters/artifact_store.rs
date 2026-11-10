//! Local SHA-256 content-addressed Blob publication.
//!
//! This Adapter owns filesystem bytes only. Artifact provenance is appended as
//! a Research Event by the fixed Tool after this Adapter returns successfully.

mod error;

use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::adapters::project_event_store::{
    Project, ProjectEventStoreError, commit_durable_temporary_file, create_durable_temporary_file,
    remove_durable_temporary_file_best_effort, sync_directory,
};
use crate::domain::artifacts::BlobId;
use crate::ports::artifact::{ArtifactStore, StoredBlob};

use crate::adapters::file_revision::FileRevision;
use error::LocalArtifactStoreError;

const BLOB_PATH_SEGMENTS: [&str; 3] = ["artifacts", "blobs", "sha256"];
const HASH_BUFFER_BYTES: usize = 64 * 1024;

/// A project-isolated local Artifact Blob Store.
pub(crate) struct LocalArtifactStore {
    project: Project,
    verified_ranges:
        std::sync::Mutex<std::collections::BTreeMap<BlobId, (FileRevision, Option<String>)>>,
}

impl LocalArtifactStore {
    pub(crate) fn for_project(project: &Project) -> Self {
        Self {
            project: project.clone(),
            verified_ranges: Default::default(),
        }
    }
}

impl ArtifactStore for LocalArtifactStore {
    type Error = LocalArtifactStoreError;

    fn publish_file(&self, source: &Path) -> Result<StoredBlob, Self::Error> {
        let blob_directory = self.ensure_blob_directory()?;
        let (blob_id, size_bytes, temporary_path) = stage_source_once(source, &blob_directory)?;
        self.publish_staged(blob_directory, blob_id, size_bytes, temporary_path)
    }

    fn publish_bytes(&self, bytes: &[u8]) -> Result<StoredBlob, Self::Error> {
        let blob_directory = self.ensure_blob_directory()?;
        let blob_id =
            BlobId::new(format!("{:x}", Sha256::digest(bytes))).expect("SHA-256 is valid");
        let size_bytes =
            u64::try_from(bytes.len()).map_err(|_| LocalArtifactStoreError::SourceRead {
                path: blob_directory.clone(),
                source: io::Error::other("Artifact size exceeds u64"),
            })?;
        // Already-owned bytes need no staged copy when the destination passes
        // the same full integrity check as publication. Never cache this check.
        if matches!(
            inspect_existing_blob(&blob_directory.join(blob_id.as_str()), &blob_id, size_bytes)?,
            ExistingBlob::Verified
        ) {
            return Ok(StoredBlob {
                blob_id,
                size_bytes,
                already_present: true,
            });
        }
        let temporary_path = stage_bytes_once(bytes, &blob_directory)?;
        self.publish_staged(blob_directory, blob_id, size_bytes, temporary_path)
    }

    fn read_blob(&self, blob_id: &BlobId) -> Result<Vec<u8>, Self::Error> {
        self.read_verified_range(blob_id, 0, usize::MAX, Some(u64::MAX))
            .map(|(bytes, _, _)| bytes)
    }
}

impl LocalArtifactStore {
    /// Verifies immutable bytes while retaining only a bounded readback range.
    pub(crate) fn read_output_range(
        &self,
        blob_id: &BlobId,
        offset: u64,
        limit: usize,
    ) -> Result<(Vec<u8>, u64), LocalArtifactStoreError> {
        self.read_verified_range(
            blob_id,
            offset,
            limit,
            Some(crate::domain::tools::MAX_CAPTURE_STREAM_BYTES as u64),
        )
        .map(|(bytes, size, _)| (bytes, size))
    }

    /// Validate a Blob revision once, then read only the requested range.
    pub(crate) fn read_artifact_range(
        &self,
        blob_id: &BlobId,
        offset: u64,
        limit: usize,
    ) -> Result<(Vec<u8>, u64, Option<String>), LocalArtifactStoreError> {
        self.read_verified_range(blob_id, offset, limit, None)
    }

    fn read_verified_range(
        &self,
        blob_id: &BlobId,
        offset: u64,
        limit: usize,
        maximum_size: Option<u64>,
    ) -> Result<(Vec<u8>, u64, Option<String>), LocalArtifactStoreError> {
        self.inspect_blob_directory()?;
        let path = self.blob_directory().join(blob_id.as_str());
        let mut file = open_regular_existing_blob(&path)?;
        let metadata =
            file.metadata()
                .map_err(|source| LocalArtifactStoreError::ExistingBlobInspect {
                    path: path.clone(),
                    source,
                })?;
        let revision = FileRevision::of(&metadata).map_err(|source| {
            LocalArtifactStoreError::ExistingBlobInspect {
                path: path.clone(),
                source,
            }
        })?;
        let size = metadata.len();
        if maximum_size.is_some_and(|maximum| size > maximum) || offset > size {
            return Err(LocalArtifactStoreError::OutputRange { offset, size });
        }
        let cached = self
            .verified_ranges
            .lock()
            .expect("Blob revision lock poisoned")
            .get(blob_id)
            .filter(|(previous, hash)| {
                previous == &revision && (maximum_size.is_some() || hash.is_some())
            })
            .cloned();
        if let Some((_, hash)) = cached {
            file.seek(SeekFrom::Start(offset)).map_err(|source| {
                LocalArtifactStoreError::ExistingBlobInspect {
                    path: path.clone(),
                    source,
                }
            })?;
            let mut bytes = vec![0; (size - offset).min(limit as u64) as usize];
            file.read_exact(&mut bytes).map_err(|source| {
                LocalArtifactStoreError::ExistingBlobInspect {
                    path: path.clone(),
                    source,
                }
            })?;
            let after = file
                .metadata()
                .and_then(|metadata| FileRevision::of(&metadata))
                .map_err(|source| LocalArtifactStoreError::ExistingBlobInspect {
                    path: path.clone(),
                    source,
                })?;
            if after != revision {
                return Err(LocalArtifactStoreError::ExistingBlobConflict {
                    path,
                    blob_id: blob_id.clone(),
                });
            }
            return Ok((bytes, size, hash));
        }
        let mut position = 0u64;
        let end = offset.saturating_add(limit as u64);
        let mut bytes = Vec::new();
        let mut content_hash = maximum_size.is_none().then(blake3::Hasher::new);
        // Also bound reads if a concurrent writer grows the opened file after
        // metadata inspection. The extra byte makes growth an integrity error.
        let mut file = file.take(size.saturating_add(1));
        let (actual_blob_id, actual_size) = hash_open_existing_blob(&path, &mut file, |chunk| {
            if let Some(hash) = &mut content_hash {
                hash.update(chunk);
            }
            let chunk_end = position.saturating_add(chunk.len() as u64);
            if offset < chunk_end && end > position {
                let start = offset.saturating_sub(position) as usize;
                let stop = end.saturating_sub(position).min(chunk.len() as u64) as usize;
                bytes.extend_from_slice(&chunk[start..stop]);
            }
            position = chunk_end;
        })?;
        if actual_blob_id != *blob_id || actual_size != size {
            return Err(LocalArtifactStoreError::ExistingBlobConflict {
                path,
                blob_id: blob_id.clone(),
            });
        }
        let after = file
            .get_ref()
            .metadata()
            .and_then(|metadata| FileRevision::of(&metadata))
            .map_err(|source| LocalArtifactStoreError::ExistingBlobInspect {
                path: path.clone(),
                source,
            })?;
        if after != revision {
            return Err(LocalArtifactStoreError::ExistingBlobConflict {
                path,
                blob_id: blob_id.clone(),
            });
        }
        let content_hash = content_hash.map(|hash| hash.finalize().to_hex().to_string());
        self.verified_ranges
            .lock()
            .expect("Blob revision lock poisoned")
            .insert(blob_id.clone(), (revision, content_hash.clone()));
        Ok((bytes, size, content_hash))
    }

    fn publish_staged(
        &self,
        blob_directory: PathBuf,
        blob_id: BlobId,
        size_bytes: u64,
        temporary_path: PathBuf,
    ) -> Result<StoredBlob, LocalArtifactStoreError> {
        let destination = blob_directory.join(blob_id.as_str());

        let result = (|| match inspect_existing_blob(&destination, &blob_id, size_bytes)? {
            ExistingBlob::Verified => Ok(StoredBlob {
                blob_id,
                size_bytes,
                already_present: true,
            }),
            ExistingBlob::Absent => {
                match commit_durable_temporary_file(&temporary_path, &destination) {
                    Ok(()) => Ok(StoredBlob {
                        blob_id,
                        size_bytes,
                        already_present: false,
                    }),
                    Err(ProjectEventStoreError::Io { source, .. })
                        if source.kind() == io::ErrorKind::AlreadyExists =>
                    {
                        inspect_existing_blob(&destination, &blob_id, size_bytes)?;
                        Ok(StoredBlob {
                            blob_id,
                            size_bytes,
                            already_present: true,
                        })
                    }
                    Err(error) => Err(LocalArtifactStoreError::Publish {
                        path: destination,
                        error: Box::new(error),
                    }),
                }
            }
        })();
        let published = matches!(
            result.as_ref(),
            Ok(StoredBlob {
                already_present: false,
                ..
            })
        );
        if !published {
            remove_durable_temporary_file_best_effort(&temporary_path);
        }
        result
    }

    fn blob_directory(&self) -> PathBuf {
        BLOB_PATH_SEGMENTS
            .into_iter()
            .fold(self.project.control_directory(), |path, segment| {
                path.join(segment)
            })
    }

    fn inspect_blob_directory(&self) -> Result<(), LocalArtifactStoreError> {
        let mut current = self.project.root().to_path_buf();
        for segment in
            std::iter::once(self.project.control_directory_name()).chain(BLOB_PATH_SEGMENTS)
        {
            current.push(segment);
            match fs::symlink_metadata(&current) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(LocalArtifactStoreError::BlobDirectorySymlink { path: current });
                }
                Ok(metadata) if !metadata.file_type().is_dir() => {
                    return Err(LocalArtifactStoreError::BlobDirectoryNotDirectory {
                        path: current,
                    });
                }
                Ok(_) => {}
                Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(()),
                Err(source) => {
                    return Err(LocalArtifactStoreError::InspectBlobDirectory {
                        path: current,
                        source,
                    });
                }
            }
        }
        Ok(())
    }
}

enum ExistingBlob {
    Absent,
    Verified,
}

impl LocalArtifactStore {
    fn ensure_blob_directory(&self) -> Result<PathBuf, LocalArtifactStoreError> {
        let mut current = self.project.root().to_path_buf();
        for segment in
            std::iter::once(self.project.control_directory_name()).chain(BLOB_PATH_SEGMENTS)
        {
            current.push(segment);
            ensure_directory_component(&current)?;
        }
        Ok(current)
    }
}

fn ensure_directory_component(path: &Path) -> Result<(), LocalArtifactStoreError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(LocalArtifactStoreError::BlobDirectorySymlink {
                path: path.to_path_buf(),
            })
        }
        Ok(metadata) if metadata.file_type().is_dir() => Ok(()),
        Ok(_) => Err(LocalArtifactStoreError::BlobDirectoryNotDirectory {
            path: path.to_path_buf(),
        }),
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            fs::create_dir(path).map_err(|source| {
                LocalArtifactStoreError::CreateBlobDirectory {
                    path: path.to_path_buf(),
                    source,
                }
            })?;
            let parent =
                path.parent()
                    .ok_or_else(|| LocalArtifactStoreError::CreateBlobDirectory {
                        path: path.to_path_buf(),
                        source: io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "Artifact Blob directory has no parent",
                        ),
                    })?;
            sync_directory(parent).map_err(|error| {
                LocalArtifactStoreError::PersistBlobDirectory {
                    path: path.to_path_buf(),
                    error: Box::new(error),
                }
            })?;
            ensure_directory_component(path)
        }
        Err(source) => Err(LocalArtifactStoreError::InspectBlobDirectory {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn stage_source_once(
    source: &Path,
    blob_directory: &Path,
) -> Result<(BlobId, u64, PathBuf), LocalArtifactStoreError> {
    let mut source_file = open_regular_source(source)?;
    let (mut temporary_file, temporary_path) = create_durable_temporary_file(blob_directory)
        .map_err(|error| LocalArtifactStoreError::Stage {
            path: blob_directory.to_path_buf(),
            error: Box::new(error),
        })?;
    let mut hasher = Sha256::new();
    let mut size_bytes = 0_u64;
    let mut buffer = [0_u8; HASH_BUFFER_BYTES];
    let stage_result = (|| -> Result<(), LocalArtifactStoreError> {
        loop {
            let read = source_file.read(&mut buffer).map_err(|error| {
                LocalArtifactStoreError::SourceRead {
                    path: source.to_path_buf(),
                    source: error,
                }
            })?;
            if read == 0 {
                return temporary_file.sync_all().map_err(|source| {
                    LocalArtifactStoreError::StagedBlobSync {
                        path: temporary_path.clone(),
                        source,
                    }
                });
            }
            temporary_file
                .write_all(&buffer[..read])
                .map_err(|source| LocalArtifactStoreError::StagedBlobWrite {
                    path: temporary_path.clone(),
                    source,
                })?;
            hasher.update(&buffer[..read]);
            size_bytes = size_bytes.checked_add(read as u64).ok_or_else(|| {
                LocalArtifactStoreError::SourceRead {
                    path: source.to_path_buf(),
                    source: io::Error::other("Artifact size exceeds u64"),
                }
            })?;
        }
    })();
    drop(temporary_file);
    if let Err(error) = stage_result {
        remove_durable_temporary_file_best_effort(&temporary_path);
        return Err(error);
    }
    let blob_id = BlobId::new(format!("{:x}", hasher.finalize())).expect("SHA-256 is valid");
    Ok((blob_id, size_bytes, temporary_path))
}

fn stage_bytes_once(
    bytes: &[u8],
    blob_directory: &Path,
) -> Result<PathBuf, LocalArtifactStoreError> {
    let (mut temporary_file, temporary_path) = create_durable_temporary_file(blob_directory)
        .map_err(|error| LocalArtifactStoreError::Stage {
            path: blob_directory.to_path_buf(),
            error: Box::new(error),
        })?;
    let stage_result = temporary_file
        .write_all(bytes)
        .and_then(|()| temporary_file.sync_all())
        .map_err(|source| LocalArtifactStoreError::StagedBlobWrite {
            path: temporary_path.clone(),
            source,
        });
    drop(temporary_file);
    if let Err(error) = stage_result {
        remove_durable_temporary_file_best_effort(&temporary_path);
        return Err(error);
    }
    Ok(temporary_path)
}

fn inspect_existing_blob(
    path: &Path,
    expected_blob_id: &BlobId,
    expected_size: u64,
) -> Result<ExistingBlob, LocalArtifactStoreError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(LocalArtifactStoreError::ExistingBlobSymlink {
                    path: path.to_path_buf(),
                });
            }
            if !metadata.file_type().is_file() {
                return Err(LocalArtifactStoreError::ExistingBlobNotRegular {
                    path: path.to_path_buf(),
                });
            }
            let (actual_blob_id, actual_size) = hash_existing_blob(path)?;
            if actual_blob_id != *expected_blob_id || actual_size != expected_size {
                return Err(LocalArtifactStoreError::ExistingBlobConflict {
                    path: path.to_path_buf(),
                    blob_id: expected_blob_id.clone(),
                });
            }
            Ok(ExistingBlob::Verified)
        }
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(ExistingBlob::Absent),
        Err(source) => Err(LocalArtifactStoreError::ExistingBlobInspect {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn hash_existing_blob(path: &Path) -> Result<(BlobId, u64), LocalArtifactStoreError> {
    let mut file = open_regular_existing_blob(path)?;
    hash_open_existing_blob(path, &mut file, |_| {})
}

fn hash_open_existing_blob(
    path: &Path,
    file: &mut impl Read,
    mut collect: impl FnMut(&[u8]),
) -> Result<(BlobId, u64), LocalArtifactStoreError> {
    let mut hasher = Sha256::new();
    let mut size_bytes = 0_u64;
    let mut buffer = [0_u8; HASH_BUFFER_BYTES];
    loop {
        let read =
            file.read(&mut buffer)
                .map_err(|source| LocalArtifactStoreError::ExistingBlobRead {
                    path: path.to_path_buf(),
                    source,
                })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        collect(&buffer[..read]);
        size_bytes = size_bytes.checked_add(read as u64).ok_or_else(|| {
            LocalArtifactStoreError::ExistingBlobRead {
                path: path.to_path_buf(),
                source: io::Error::other("Artifact size exceeds u64"),
            }
        })?;
    }
    let blob_id = BlobId::new(format!("{:x}", hasher.finalize())).expect("SHA-256 is valid");
    Ok((blob_id, size_bytes))
}

fn open_regular_source(path: &Path) -> Result<File, LocalArtifactStoreError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(LocalArtifactStoreError::SourceSymlink {
                path: path.to_path_buf(),
            })
        }
        Ok(metadata) if !metadata.file_type().is_file() => {
            Err(LocalArtifactStoreError::SourceNotRegular {
                path: path.to_path_buf(),
            })
        }
        Ok(_) => File::open(path).map_err(|source| LocalArtifactStoreError::SourceRead {
            path: path.to_path_buf(),
            source,
        }),
        Err(source) => Err(LocalArtifactStoreError::SourceInspect {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn open_regular_existing_blob(path: &Path) -> Result<File, LocalArtifactStoreError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(LocalArtifactStoreError::ExistingBlobSymlink {
                path: path.to_path_buf(),
            })
        }
        Ok(metadata) if !metadata.file_type().is_file() => {
            Err(LocalArtifactStoreError::ExistingBlobNotRegular {
                path: path.to_path_buf(),
            })
        }
        Ok(_) => File::open(path).map_err(|source| LocalArtifactStoreError::ExistingBlobRead {
            path: path.to_path_buf(),
            source,
        }),
        Err(source) => Err(LocalArtifactStoreError::ExistingBlobInspect {
            path: path.to_path_buf(),
            source,
        }),
    }
}
