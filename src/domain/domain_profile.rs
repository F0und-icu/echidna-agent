//! Fixed vulnerability-research target domains.
//!
//! The four compile-time values are one closed axis inside the single Agent.
//! They are not personalities, workflow stages or a dynamic registry.

use serde::{Deserialize, Serialize};

use crate::domain::research::TargetKind;
use crate::domain::tools::{ToolExposure, ToolExposurePlan, ToolName};

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DomainProfile {
    Source,
    Web,
    Mobile,
    Binary,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DomainProfileSelectionSource {
    Default,
    Operator,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DomainProfileReasonCode {
    NewSessionDefaultSource,
    LegacyDefaultSource,
    DefinitionDefault,
    OperatorSelected,
}

/// The durable Domain selection restored by Session Resume. Source and reason
/// are validated together so a default cannot be persisted as an operator
/// choice, or vice versa.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DomainProfileSelection {
    profile: DomainProfile,
    profile_generation: u64,
    source: DomainProfileSelectionSource,
    reason_code: DomainProfileReasonCode,
}

impl DomainProfileSelection {
    pub const fn definition_default(profile: DomainProfile) -> Self {
        if matches!(profile, DomainProfile::Source) {
            return Self::new_session_default();
        }
        Self {
            profile,
            profile_generation: profile.generation(),
            source: DomainProfileSelectionSource::Default,
            reason_code: DomainProfileReasonCode::DefinitionDefault,
        }
    }

    pub const fn new_session_default() -> Self {
        Self::default_source(DomainProfileReasonCode::NewSessionDefaultSource)
    }

    pub const fn legacy_default() -> Self {
        Self::default_source(DomainProfileReasonCode::LegacyDefaultSource)
    }

    const fn default_source(reason_code: DomainProfileReasonCode) -> Self {
        Self {
            profile: DomainProfile::Source,
            profile_generation: DomainProfile::Source.generation(),
            source: DomainProfileSelectionSource::Default,
            reason_code,
        }
    }

    pub const fn operator(profile: DomainProfile) -> Self {
        Self {
            profile,
            profile_generation: profile.generation(),
            source: DomainProfileSelectionSource::Operator,
            reason_code: DomainProfileReasonCode::OperatorSelected,
        }
    }

    pub const fn profile(self) -> DomainProfile {
        self.profile
    }

    pub const fn profile_generation(self) -> u64 {
        self.profile_generation
    }

    pub const fn source(self) -> DomainProfileSelectionSource {
        self.source
    }

    pub const fn reason_code(self) -> DomainProfileReasonCode {
        self.reason_code
    }

    pub fn validate(self) -> Result<(), DomainProfileSelectionError> {
        let legacy_prompt = self.profile == DomainProfile::Binary && self.profile_generation == 1;
        if self.profile_generation != self.profile.generation() && !legacy_prompt {
            return Err(DomainProfileSelectionError::GenerationMismatch {
                profile: self.profile,
                expected: self.profile.generation(),
                received: self.profile_generation,
            });
        }
        let valid = matches!(
            (self.source, self.reason_code, self.profile),
            (
                DomainProfileSelectionSource::Default,
                DomainProfileReasonCode::NewSessionDefaultSource
                    | DomainProfileReasonCode::LegacyDefaultSource,
                DomainProfile::Source
            ) | (
                DomainProfileSelectionSource::Default,
                DomainProfileReasonCode::DefinitionDefault,
                _
            ) | (
                DomainProfileSelectionSource::Operator,
                DomainProfileReasonCode::OperatorSelected,
                _
            )
        );
        if valid {
            Ok(())
        } else {
            Err(DomainProfileSelectionError::InvalidSelection)
        }
    }
}

#[derive(Clone, Debug, thiserror::Error, Eq, PartialEq)]
pub enum DomainProfileSelectionError {
    #[error(
        "DomainProfile `{profile:?}` generation mismatch: expected {expected}, received {received}"
    )]
    GenerationMismatch {
        profile: DomainProfile,
        expected: u64,
        received: u64,
    },
    #[error("DomainProfile selection source and reason are inconsistent")]
    InvalidSelection,
}

impl DomainProfile {
    pub const ALL: [Self; 4] = [Self::Source, Self::Web, Self::Mobile, Self::Binary];

    pub const fn generation(self) -> u64 {
        match self {
            Self::Binary => 2,
            _ => 1,
        }
    }

    pub const fn id(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Web => "web",
            Self::Mobile => "mobile",
            Self::Binary => "binary",
        }
    }

    pub const fn target_kind(self) -> TargetKind {
        match self {
            Self::Source => TargetKind::Source,
            Self::Web => TargetKind::Web,
            Self::Mobile => TargetKind::Mobile,
            Self::Binary => TargetKind::Binary,
        }
    }

    /// Firmware remains unsupported until a separately justified fixed
    /// DomainProfile exists.
    pub fn supports_target_kind(self, target_kind: TargetKind) -> bool {
        self.target_kind() == target_kind
    }

    pub const fn prompt(self) -> &'static str {
        match self {
            Self::Source => include_str!("domain_profile/prompts/source-v1.md"),
            Self::Web => include_str!("domain_profile/prompts/web-v1.md"),
            Self::Mobile => include_str!("domain_profile/prompts/mobile-v1.md"),
            Self::Binary => include_str!("domain_profile/prompts/binary-v2.md"),
        }
    }

    /// Domain overrides stay bounded to domain-specific availability. Phase
    /// posture is composed separately by the Tool exposure owner.
    pub fn tool_exposure(self) -> ToolExposurePlan {
        match self {
            Self::Source => {
                ToolExposurePlan::new([(ToolName::new("view_image"), ToolExposure::Deferred)])
            }
            Self::Web | Self::Mobile => {
                ToolExposurePlan::new([(ToolName::new("http_request"), ToolExposure::Direct)])
            }
            Self::Binary => ToolExposurePlan::default(),
        }
    }
}
