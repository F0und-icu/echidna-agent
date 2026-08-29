use serde::{Deserialize, Serialize};

use super::{ResearchControlError, ResearchPhase};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchPhaseSelectionSource {
    Default,
    Operator,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ResearchPhaseReasonCode {
    NewSessionDefaultMapping,
    LegacyDefaultMapping,
    OperatorSelected,
}

/// The exact durable value stored by the Session Event. Source and reason are
/// validated together so persisted facts cannot forge automatic selection.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchPhaseSelection {
    phase: ResearchPhase,
    phase_generation: u64,
    source: ResearchPhaseSelectionSource,
    reason_code: ResearchPhaseReasonCode,
}

impl ResearchPhaseSelection {
    pub const fn new_session_default() -> Self {
        Self::default_mapping(ResearchPhaseReasonCode::NewSessionDefaultMapping)
    }

    pub const fn legacy_default() -> Self {
        Self::default_mapping(ResearchPhaseReasonCode::LegacyDefaultMapping)
    }

    const fn default_mapping(reason_code: ResearchPhaseReasonCode) -> Self {
        Self {
            phase: ResearchPhase::Mapping,
            phase_generation: ResearchPhase::Mapping.generation(),
            source: ResearchPhaseSelectionSource::Default,
            reason_code,
        }
    }

    pub const fn operator(phase: ResearchPhase) -> Self {
        Self {
            phase,
            phase_generation: phase.generation(),
            source: ResearchPhaseSelectionSource::Operator,
            reason_code: ResearchPhaseReasonCode::OperatorSelected,
        }
    }

    pub const fn phase(self) -> ResearchPhase {
        self.phase
    }

    pub const fn phase_generation(self) -> u64 {
        self.phase_generation
    }

    pub const fn source(self) -> ResearchPhaseSelectionSource {
        self.source
    }

    pub const fn reason_code(self) -> ResearchPhaseReasonCode {
        self.reason_code
    }

    pub fn validate(self) -> Result<(), ResearchControlError> {
        if self.phase_generation != self.phase.generation() {
            return Err(ResearchControlError::PhaseGenerationMismatch {
                phase: self.phase,
                expected: self.phase.generation(),
                received: self.phase_generation,
            });
        }
        let valid = matches!(
            (self.source, self.reason_code, self.phase),
            (
                ResearchPhaseSelectionSource::Default,
                ResearchPhaseReasonCode::NewSessionDefaultMapping
                    | ResearchPhaseReasonCode::LegacyDefaultMapping,
                ResearchPhase::Mapping
            ) | (
                ResearchPhaseSelectionSource::Operator,
                ResearchPhaseReasonCode::OperatorSelected,
                _
            )
        );
        if !valid {
            return Err(ResearchControlError::InvalidPhaseSelection);
        }
        Ok(())
    }
}
