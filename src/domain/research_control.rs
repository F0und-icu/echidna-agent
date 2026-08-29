//! Stable research identity, fixed Phase control and Prompt composition.
//!
//! Each child module owns one independent reason for change: fixed phase
//! resources, durable phase-selection validation, or prompt composition.

mod phase;
mod prompt;
mod selection;

pub use phase::{BaseContract, ResearchPhase};
pub use prompt::{PromptComposition, ResearchControlError};
pub use selection::{
    ResearchPhaseReasonCode, ResearchPhaseSelection, ResearchPhaseSelectionSource,
};
