//! Adapter implementations for declared Ports.

pub(crate) mod agent_definition;
pub(crate) mod agent_evolution;
pub mod artifact_store;
pub mod credentials;
pub(crate) mod file_diff;
pub mod fragment_cache;
pub(crate) mod http_error_body;
pub mod image;
pub(crate) mod mcp;
pub mod mock_model;
pub mod project_event_store;
pub(crate) mod project_memory;
pub(crate) mod project_snapshot;
pub mod replay;
pub(crate) mod research_projection;
pub mod responses;
pub(crate) mod session_compaction;
pub(crate) mod subscription_auth;
pub mod tools;
