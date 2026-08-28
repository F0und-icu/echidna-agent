//! Fixed, local-only adapters for Trace Replay and Shadow Eval.
//!
//! Model and Tool adapters remain separate because they implement different
//! Ports and own independent ordering and failure boundaries.

mod model;
mod tool;

pub use crate::domain::trace_eval::{
    TraceReplayModelStep as ReplayModelStep, TraceReplayToolStep as ReplayToolStep,
    TraceReplayToolUse as ReplayToolUse,
};
pub use model::ReplayModelProvider;
pub use tool::ReplayToolExecutor;

use thiserror::Error;

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ReplayAdapterError {
    #[error("ReplayModelProvider requires at least one semantic sample")]
    ModelStepsRequired,
    #[error("Replay Tool spec `{tool_name}` is not Direct")]
    NonDirectToolSpec { tool_name: String },
    #[error("Replay Tool specs contain a duplicate Tool name")]
    DuplicateToolSpec,
    #[error("Replay Tool step has no recorded direct spec for `{tool_name}`")]
    ToolStepSpecMissing { tool_name: String },
}
