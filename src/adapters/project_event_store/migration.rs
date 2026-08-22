use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::domain::events::{EventEnvelope, EventSchema, EventUpcasterRegistry};

use super::ProjectEventStoreError;
use super::durable_file::commit_new_durable_file;
use super::jsonl::scan_stream_bytes;
use super::project::{ProjectEventStream, ProjectWriter, create_directory, read_bytes};

/// Metadata written beside every explicit export/migration result.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExportMigrationManifest {
    /// BLAKE3 digest of the exact retained source JSONL bytes.
    pub source_hash: String,
    /// Schema of the newly written destination Event stream.
    pub target_schema: EventSchema,
    /// Caller-supplied migration tool version.
    pub tool_version: String,
}

impl ProjectWriter {
    /// Exports one source stream into a new current-schema JSONL stream.
    ///
    /// This is intentionally a skeleton, not an in-place upgrade command:
    /// source bytes are retained, destination files must be new, and the
    /// caller records both the requested target schema and tool version.
    pub fn export_migrate_stream(
        &mut self,
        stream: &ProjectEventStream,
        destination: &Path,
        target_schema: EventSchema,
        tool_version: impl Into<String>,
    ) -> Result<ExportMigrationManifest, ProjectEventStoreError> {
        let tool_version = tool_version.into();
        if tool_version.trim().is_empty() {
            return Err(ProjectEventStoreError::EmptyMigrationToolVersion);
        }
        let registry = EventUpcasterRegistry::current();
        if target_schema != *registry.current_schema() {
            return Err(ProjectEventStoreError::UnsupportedMigrationTarget {
                requested: target_schema,
                registered_current: registry.current_schema().clone(),
            });
        }

        let source = self.project.stream_path(stream)?;
        let destination = absolute_path(destination)?;
        if source == destination {
            return Err(ProjectEventStoreError::ExportWouldOverwriteSource { path: source });
        }
        let manifest_path = migration_manifest_path(&destination)?;
        match (destination.exists(), manifest_path.exists()) {
            (false, false) => {}
            (true, true) => {
                return if migration_pair_is_complete(
                    &destination,
                    &manifest_path,
                    stream,
                    &registry,
                )? {
                    Err(ProjectEventStoreError::ExportTargetExists { path: destination })
                } else {
                    Err(ProjectEventStoreError::IncompleteMigration {
                        destination,
                        manifest_path,
                    })
                };
            }
            _ => {
                return Err(ProjectEventStoreError::IncompleteMigration {
                    destination,
                    manifest_path,
                });
            }
        }

        let source_bytes = read_bytes(&source, "read source Event stream for export")?;
        let events = scan_stream_bytes(&source_bytes, &source, &stream.stream_id(), &registry)?
            .into_complete(&source)?;
        let manifest = ExportMigrationManifest {
            source_hash: blake3::hash(&source_bytes).to_hex().to_string(),
            target_schema,
            tool_version,
        };
        let event_bytes = encode_export_events(&events)?;
        let manifest_bytes = serde_json::to_vec_pretty(&manifest)
            .map_err(ProjectEventStoreError::MigrationManifestEncode)?;

        let parent = destination
            .parent()
            .ok_or_else(|| ProjectEventStoreError::NoFileName {
                path: destination.clone(),
            })?;
        create_directory(parent)?;
        commit_new_durable_file(&destination, &event_bytes)?;
        commit_new_durable_file(&manifest_path, &manifest_bytes)?;

        Ok(manifest)
    }
}

fn migration_pair_is_complete(
    destination: &Path,
    manifest_path: &Path,
    stream: &ProjectEventStream,
    registry: &EventUpcasterRegistry<Value>,
) -> Result<bool, ProjectEventStoreError> {
    let destination_bytes = read_bytes(destination, "read existing migration destination")?;
    if scan_stream_bytes(
        &destination_bytes,
        destination,
        &stream.stream_id(),
        registry,
    )
    .and_then(|scan| scan.into_complete(destination))
    .is_err()
    {
        return Ok(false);
    }

    let manifest_bytes = read_bytes(manifest_path, "read existing migration manifest")?;
    let Ok(manifest) = serde_json::from_slice::<ExportMigrationManifest>(&manifest_bytes) else {
        return Ok(false);
    };

    Ok(manifest.target_schema == *registry.current_schema()
        && is_blake3_digest(&manifest.source_hash)
        && !manifest.tool_version.trim().is_empty())
}

fn migration_manifest_path(destination: &Path) -> Result<PathBuf, ProjectEventStoreError> {
    let filename = destination
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(|| ProjectEventStoreError::NoFileName {
            path: destination.to_path_buf(),
        })?;
    Ok(destination.with_file_name(format!("{filename}.migration.json")))
}

fn absolute_path(path: &Path) -> Result<PathBuf, ProjectEventStoreError> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        std::env::current_dir()
            .map(|current| current.join(path))
            .map_err(|source| ProjectEventStoreError::Io {
                operation: "resolve export destination",
                path: path.to_path_buf(),
                source,
            })
    }
}

fn encode_export_events(
    events: &[EventEnvelope<Value>],
) -> Result<Vec<u8>, ProjectEventStoreError> {
    let mut bytes = Vec::new();
    for event in events {
        serde_json::to_writer(&mut bytes, event).map_err(ProjectEventStoreError::EventEncode)?;
        bytes.push(b'\n');
    }
    Ok(bytes)
}

fn is_blake3_digest(value: &str) -> bool {
    value.len() == blake3::OUT_LEN * 2 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
