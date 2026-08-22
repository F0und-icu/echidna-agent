//! Filesystem-backed project discovery and JSONL Event Store Adapter.
//!
//! The public facade is deliberately small. `project` owns layout and the OS
//! writer lock, `durable_file` owns private no-overwrite publication, `jsonl`
//! owns every byte-level stream/recovery rule, and `migration` owns the
//! explicit non-destructive export skeleton.

mod durable_file;
mod error;
mod jsonl;
mod migration;
mod project;

pub(crate) use durable_file::{
    commit_durable_temporary_file, create_durable_temporary_file,
    remove_durable_temporary_file_best_effort, sync_directory,
};
pub use error::ProjectEventStoreError;
pub(crate) use jsonl::read_existing_research_events;
pub use jsonl::{JsonlEventStore, RecoveryMetadata};
pub use migration::ExportMigrationManifest;
pub use project::{Project, ProjectEventStream, ProjectWriter};
