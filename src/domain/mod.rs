//! Provider-neutral domain contracts.
//!
//! This module contains only data, protocol semantics, and pure state
//! transformations. It must not perform filesystem, network, process, or
//! clock I/O.

pub mod artifacts;
pub mod context;
pub mod events;
pub mod history;
pub mod model;
pub mod project;
pub mod tools;

pub use artifacts::ArtifactId;
