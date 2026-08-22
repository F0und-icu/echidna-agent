//! Local SHA-256 content-addressed Blob publication.
//!
//! This Adapter owns filesystem bytes only. Artifact provenance is appended as
//! a Research Event by the fixed Tool after this Adapter returns successfully.

mod error;

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::adapters::project_event_store::{
    Project, ProjectEventStoreError, commit_durable_temporary_file, create_durable_temporary_file,
    remove_durable_temporary_file_best_effort, sync_directory,
};
use crate::domain::artifacts::BlobId;
use crate::ports::artifact::{ArtifactStore, StoredBlob};

use error::LocalArtifactStoreError;

const BLOB_PATH_SEGMENTS: [&str; 4] = [".cyber-agent", "artifacts", "blobs", "sha256"];
const HASH_BUFFER_BYTES: usize = 64 * 1024;

/// A project-isolated local Artifact Blob Store.
pub(crate) struct LocalArtifactStore {
    project_root: PathBuf,
}

impl LocalArtifactStore {
    pub(crate) fn for_project(project: &Project) -> Self {
        Self {
            project_root: project.root().to_path_buf(),
        }
    }
}

impl ArtifactStore for LocalArtifactStore {
    type Error = LocalArtifactStoreError;

    fn publish_file(&self, source: &Path) -> Result<StoredBlob, Self::Error> {
        let blob_directory = self.ensure_blob_directory()?;
        let (blob_id, size_bytes, temporary_path) = stage_source_once(source, &blob_directory)?;
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
}

enum ExistingBlob {
    Absent,
    Verified,
}

impl LocalArtifactStore {
    fn ensure_blob_directory(&self) -> Result<PathBuf, LocalArtifactStoreError> {
        let mut current = self.project_root.clone();
        for segment in BLOB_PATH_SEGMENTS {
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
