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

#[cfg(unix)]
pub(crate) use durable_file::commit_new_durable_file_with;
pub(crate) use durable_file::{
    commit_durable_temporary_file, commit_new_durable_file, create_durable_temporary_file,
    remove_durable_temporary_file_best_effort, replace_durable_file, sync_directory,
};
pub use error::ProjectEventStoreError;
pub(crate) use jsonl::read_research_projection;
pub use jsonl::{JsonlEventStore, RecoveryMetadata};
pub(crate) use jsonl::{SessionStreamCheckpoint, SessionStreamCursor};
pub(crate) use jsonl::{
    read_existing_research_events, read_existing_session_events, read_indexed_session_segment,
    verify_session_checkpoint, visit_existing_session_records, visit_session_records_after,
    visit_session_records_after_checkpoint,
};
pub use migration::ExportMigrationManifest;
pub use project::{Project, ProjectEventStream, ProjectWriter};
