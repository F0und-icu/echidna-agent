//! Adapter implementations for declared Ports.

pub mod artifact_store;
pub mod credentials;
pub(crate) mod file_diff;
pub mod fragment_cache;
pub mod image;
pub mod mock_model;
pub mod project_event_store;
pub(crate) mod project_memory;
pub(crate) mod project_snapshot;
pub mod replay;
pub(crate) mod research_projection;
pub mod responses;
pub mod tools;
