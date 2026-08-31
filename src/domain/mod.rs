//! Provider-neutral domain contracts.
//!
//! This module contains only data, protocol semantics, and pure state
//! transformations. It must not perform filesystem, network, process, or
//! clock I/O.

pub mod artifacts;
pub(crate) mod canonical_json;
pub mod context;
pub mod domain_profile;
pub mod events;
pub(crate) mod guidance;
pub mod history;
pub mod image;
pub mod memory;
pub mod model;
pub(crate) mod observation;
pub mod profile;
pub mod project;
pub mod request_audit;
pub mod research;
pub mod research_control;
pub(crate) mod retrieval;
pub mod sampling;
pub mod session;
pub mod snapshot;
pub mod tools;
pub mod trace_eval;
pub mod vision;
pub mod world_state;

pub use artifacts::ArtifactId;
