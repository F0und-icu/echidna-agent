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
pub mod memory;
pub mod model;
pub mod project;
pub mod research;
pub mod snapshot;
pub mod tools;

pub use artifacts::ArtifactId;
