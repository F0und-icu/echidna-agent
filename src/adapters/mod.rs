//! Adapter implementations for declared Ports.

pub mod artifact_store;
pub(crate) mod file_diff;
pub mod fragment_cache;
pub mod image;
pub mod mock_model;
pub mod openai;
pub mod project_event_store;
pub(crate) mod project_memory;
pub(crate) mod project_snapshot;
pub(crate) mod tool_result_cache;
pub mod tools;
