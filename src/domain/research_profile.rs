//! Fixed vulnerability-research Prompt Profiles.
//!
//! These four compile-time Profiles are deliberately not a configurable
//! registry or filesystem loader. A selected Profile becomes the existing
//! ModePack consumed by PromptComposition and the one AgentLoop.

use crate::domain::mode::{ModePack, PromptComposition};
use crate::domain::research::TargetKind;
use crate::domain::tools::{ToolExposure, ToolExposurePlan, ToolName};

/// The fixed research domains available to the runtime in M4-S1.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResearchProfile {
    Source,
    Web,
    Mobile,
    Binary,
}

impl ResearchProfile {
    /// The complete fixed M4-S1 runtime set. This is not a dynamic registry.
    pub const ALL: [Self; 4] = [Self::Source, Self::Web, Self::Mobile, Self::Binary];

    /// Each Profile generation is its explicit, operator-visible prompt
    /// version. Changing a prompt requires incrementing its generation.
    pub const fn generation(self) -> u64 {
        1
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

    /// Firmware remains a distinct target kind until a separately authorized
    /// fixed Profile exists for it.
    pub fn supports_target_kind(self, target_kind: TargetKind) -> bool {
        self.target_kind() == target_kind
    }

    const fn prompt(self) -> &'static str {
        match self {
            Self::Source => include_str!("research_profile/prompts/source-v1.md"),
            Self::Web => include_str!("research_profile/prompts/web-v1.md"),
            Self::Mobile => include_str!("research_profile/prompts/mobile-v1.md"),
            Self::Binary => include_str!("research_profile/prompts/binary-v1.md"),
        }
    }

    /// Fixed exposure overrides only name built-in Tool contracts. Any Tool
    /// not named here retains the registry's existing exposure.
    fn tool_exposure(self) -> ToolExposurePlan {
        match self {
            Self::Source => {
                ToolExposurePlan::new([(ToolName::new("view_image"), ToolExposure::Deferred)])
            }
            Self::Web | Self::Mobile | Self::Binary => ToolExposurePlan::default(),
        }
    }

    /// Converts this fixed runtime Profile into the existing Mode Pack
    /// contract. The static Profile data is validated here once at use time.
    pub fn mode_pack(self) -> ModePack {
        ModePack::new(
            self.id(),
            self.generation(),
            self.prompt(),
            self.tool_exposure(),
        )
        .expect("fixed Research Profile data must form a valid ModePack")
    }

    /// Adds this Profile's Mode Pack to an existing fixed prompt composition.
    pub fn apply_to(self, composition: PromptComposition) -> PromptComposition {
        composition.with_mode(self.mode_pack())
    }
}
