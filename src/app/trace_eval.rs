//! Narrow composition for M4-S2 Trace Replay, Context Recompile and Shadow Eval.
//!
//! The service has no Event, Research, Memory or Project writer. Every result
//! is derived-only and carries the closed `non_evidence` boundary.

mod error;
mod results;
mod service;

pub use error::{TraceEvalServiceError, TraceReplayRunError};
pub use results::{
    ContextRecompileInput, ContextRecompileResult, MechanicalReplayResult, ShadowModelEvalResult,
};
pub use service::TraceEvalService;
