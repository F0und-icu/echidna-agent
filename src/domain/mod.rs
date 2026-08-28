//! Provider-neutral domain contracts.
//!
//! This module contains only data, protocol semantics, and pure state
//! transformations. It must not perform filesystem, network, process, or
//! clock I/O.

pub mod artifacts;
pub(crate) mod canonical_json;
pub mod context;
pub mod events;
pub mod history;
pub mod image;
pub mod memory;
pub mod mode;
pub mod model;
pub mod profile;
pub mod project;
pub mod request_audit;
pub mod research;
pub mod research_profile;
pub mod session;
pub mod snapshot;
pub mod tools;
pub mod trace_eval;
pub mod vision;
pub mod world_state;

pub use artifacts::ArtifactId;
